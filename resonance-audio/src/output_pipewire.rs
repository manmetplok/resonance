//! Native PipeWire playback stream. Bypasses cpal's ALSA-via-
//! pipewire-alsa-plugin path, whose ALSA shim keeps a >=2-period ring
//! on top of the graph cycle (~2.7-5.3 ms extra at 48 kHz / q128) with
//! no latency introspection — the single biggest avoidable chunk of the
//! live-monitoring round trip (doc #260 finding #11). A native
//! `pw_stream` renders straight into the graph's cycle buffer, so
//! output-side buffering is ~1 quantum, and the stream handle exposes
//! real latency/time info for follow-up work (finding #13).
//!
//! Lifecycle mirrors [`crate::input_pipewire`]: [`build`] spawns a
//! `ThreadLoop` (PipeWire's RT-thread wrapper), creates a `Core` +
//! `Stream` under its lock, registers a process callback that runs the
//! engine mixer directly into the dequeued graph buffer, and returns a
//! [`PipeWireOutputHandle`] whose Drop tears everything down in order.
//!
//! Device changes need no rebuild: the stream targets the default sink
//! via autoconnect, and the session manager moves it live when the
//! default changes (unlike the cpal path, which was pinned to the
//! device it opened at startup).
//!
//! Sample-rate / channel negotiation is asynchronous — the builder
//! waits up to 500 ms for the first `param_changed` and returns the
//! negotiated values, falling back to the requested ones on timeout
//! (the pinned 48 kHz graph makes a mismatch practically impossible).

use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use pipewire as pw;
use pipewire::spa;

use pw::context::ContextRc;
use pw::core::CoreRc;
use pw::properties::properties;
use pw::stream::{StreamFlags, StreamListener, StreamRc};
use pw::thread_loop::ThreadLoopRc;
use spa::param::audio::AudioInfoRaw;
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::format_utils;
use spa::param::ParamType;
use spa::pod::serialize::PodSerializer;
use spa::pod::{Object, Pod, Value};

use crate::engine::SharedState;
use crate::mixer::MixFn;

/// Handle returned by [`build`]. Owns the PipeWire thread loop, the
/// core, the stream, and the registered listener. Drop order matters
/// (lock → drop stream/listener/core → unlock → drop thread loop), so
/// every field is wrapped in [`ManuallyDrop`] and the custom `Drop`
/// impl below sequences them.
pub(crate) struct PipeWireOutputHandle {
    listener: ManuallyDrop<StreamListener<UserData>>,
    stream: ManuallyDrop<StreamRc>,
    _core: ManuallyDrop<CoreRc>,
    _context: ManuallyDrop<ContextRc>,
    thread_loop: ManuallyDrop<ThreadLoopRc>,
}

impl PipeWireOutputHandle {
    /// Run `f` against the live stream with the thread loop locked —
    /// the safe way for follow-up work (latency params, `pw_stream`
    /// time info; doc #260 finding #13) to poke the stream from a
    /// non-PipeWire thread.
    #[allow(dead_code)]
    pub(crate) fn with_stream<R>(&self, f: impl FnOnce(&StreamRc) -> R) -> R {
        let _lock = self.thread_loop.lock();
        f(&self.stream)
    }
}

/// Closure-captured state shared with the process / param-changed
/// callbacks running on the PipeWire RT thread.
struct UserData {
    /// Engine shared state: the process callback publishes the graph's
    /// reported playback latency into it every cycle (doc #260 finding
    /// #13).
    shared: Arc<SharedState>,
    /// The engine mixer, rendering directly into the graph buffer.
    mix: MixFn,
    /// Hard cap on frames per callback — the mixer's pre-allocated
    /// scratch size. The graph never asks for more than the max
    /// quantum this was sized from; a larger request is clamped (the
    /// mixer clamps + logs internally too).
    max_frames: usize,
    /// Negotiated channels — written by the param_changed callback,
    /// read by `process` every block (`AtomicU16` so the two RT
    /// callbacks race safely).
    channels: Arc<AtomicU16>,
    /// Negotiated sample rate. Same shape as `channels`.
    rate: Arc<AtomicU32>,
    /// One-shot signal so the builder can wait for the first
    /// `param_changed` to land before returning.
    notify: Arc<(Mutex<bool>, Condvar)>,
}

/// Build a PipeWire playback stream on the default sink (or
/// `sink_name`) with `channels` interleaved f32 channels at
/// `sample_rate` (the engine's rate), voting `quantum` frames of node
/// latency so the graph runs the engine's cycle size. `mix` is called
/// once per graph cycle to fill the dequeued buffer. Returns the live
/// handle plus the negotiated `(rate, channels)` once the graph has
/// attached the stream.
pub(crate) fn build(
    sink_name: Option<&str>,
    shared: Arc<SharedState>,
    sample_rate: u32,
    channels: u16,
    quantum: u32,
    max_frames: usize,
    mix: MixFn,
) -> Result<(PipeWireOutputHandle, u32, u16), String> {
    pw::init();

    // SAFETY: `ThreadLoopRc::new` is marked unsafe because the
    // resulting loop must outlive any objects created against it; we
    // satisfy that by storing the loop in the same
    // `PipeWireOutputHandle` as the stream and ordering Drop so the
    // loop is destroyed last.
    let thread_loop = unsafe {
        ThreadLoopRc::new(Some("resonance-output"), None)
            .map_err(|e| format!("PipeWire ThreadLoop::new: {e}"))?
    };
    thread_loop.start();

    // The build sequence creates objects (Context / Core / Stream)
    // that share the loop; the threaded loop's lock must be held
    // around any pipewire call that touches them.
    let lock = thread_loop.lock();

    let context = ContextRc::new(&thread_loop, None)
        .map_err(|e| format!("PipeWire Context::new: {e}"))?;
    let core = context
        .connect_rc(None)
        .map_err(|e| format!("PipeWire Core::connect: {e}"))?;

    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Production",
        *pw::keys::NODE_NAME => "resonance-output",
        *pw::keys::APP_NAME => "resonance-app",
        // Vote the engine quantum (unlike the input stream's historical
        // 1024 vote) so the graph runs our cycle size without relying
        // on an external force-quantum.
        *pw::keys::NODE_LATENCY => format!("{}/{}", quantum, sample_rate).as_str(),
    };
    if let Some(name) = sink_name {
        props.insert(*pw::keys::TARGET_OBJECT, name);
    }

    let stream = StreamRc::new(core.clone(), "resonance-output", props)
        .map_err(|e| format!("PipeWire Stream::new: {e}"))?;

    let channels_atomic = Arc::new(AtomicU16::new(channels));
    let rate_atomic = Arc::new(AtomicU32::new(sample_rate));
    let notify = Arc::new((Mutex::new(false), Condvar::new()));

    let user_data = UserData {
        shared,
        mix,
        max_frames,
        channels: Arc::clone(&channels_atomic),
        rate: Arc::clone(&rate_atomic),
        notify: Arc::clone(&notify),
    };

    let listener = stream
        .add_local_listener_with_user_data(user_data)
        .param_changed(on_param_changed)
        .state_changed(on_state_changed)
        .process(on_process)
        .register()
        .map_err(|e| format!("PipeWire Stream::register: {e}"))?;

    // Ask the graph for interleaved f32 stereo at the engine's rate,
    // with an explicit FL/FR position so the session manager can remap
    // onto whatever layout the sink runs.
    let mut audio_info = AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(sample_rate);
    audio_info.set_channels(channels as u32);
    let mut position = [0u32; 64];
    if channels == 2 {
        position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
        position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
        audio_info.set_position(position);
    }
    let pod_obj = Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let pod_bytes: Vec<u8> = PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(pod_obj),
    )
    .map_err(|e| format!("PipeWire pod serialize: {e}"))?
    .0
    .into_inner();
    let pod = Pod::from_bytes(&pod_bytes)
        .ok_or_else(|| "PipeWire Pod::from_bytes: invalid pod bytes".to_string())?;
    let mut params = [pod];

    stream
        .connect(
            spa::utils::Direction::Output,
            None,
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS | StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(|e| format!("PipeWire Stream::connect: {e}"))?;

    // Release the loop lock so the RT thread can attach the stream
    // and fire the first `param_changed`.
    drop(lock);

    // Wait briefly for the first `param_changed` so the caller can log
    // real negotiated values; on timeout return the requested ones.
    let (lock_pair, cvar) = (&notify.0, &notify.1);
    let mut got = lock_pair.lock().expect("notify mutex poisoned");
    let timeout = Duration::from_millis(500);
    while !*got {
        let (g, wait) = cvar
            .wait_timeout(got, timeout)
            .expect("notify cvar poisoned");
        got = g;
        if wait.timed_out() {
            break;
        }
    }
    drop(got);

    let negotiated_rate = rate_atomic.load(Ordering::Acquire);
    let negotiated_channels = channels_atomic.load(Ordering::Acquire);

    Ok((
        PipeWireOutputHandle {
            listener: ManuallyDrop::new(listener),
            stream: ManuallyDrop::new(stream),
            _core: ManuallyDrop::new(core),
            _context: ManuallyDrop::new(context),
            thread_loop: ManuallyDrop::new(thread_loop),
        },
        negotiated_rate,
        negotiated_channels,
    ))
}

/// Listener callback fired whenever the stream's params change; parse
/// the negotiated format into the shared atomics and wake the builder.
fn on_param_changed(
    _stream: &pw::stream::Stream,
    user_data: &mut UserData,
    id: u32,
    param: Option<&Pod>,
) {
    let Some(param) = param else {
        return;
    };
    if id != ParamType::Format.as_raw() {
        return;
    }
    let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
        return;
    };
    if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
        return;
    }
    let mut info = AudioInfoRaw::new();
    if info.parse(param).is_err() {
        return;
    }
    user_data
        .channels
        .store(info.channels() as u16, Ordering::Release);
    user_data.rate.store(info.rate(), Ordering::Release);

    let (lock_pair, cvar) = (&user_data.notify.0, &user_data.notify.1);
    if let Ok(mut g) = lock_pair.lock() {
        *g = true;
        cvar.notify_all();
    }
}

/// Surface stream death to the app — the graph killing the stream
/// (sink gone, daemon restart) is otherwise silent: the engine thread
/// stays alive, so the app's engine-death check never fires while no
/// audio plays. Publishes `SharedState::output_stream_lost`, which the
/// app's tick handler polls into a persistent banner. Runs on the
/// PipeWire loop thread, not the RT process path, so a plain atomic
/// store is fine here.
///
/// Recovery: when the session manager revives the stream (e.g.
/// PipeWire reconnects it to a new sink after a device swap), the
/// stream transitions back through `Connecting`/`Paused` to
/// `Streaming`, and that healthy transition clears the flag — so the
/// app's banner clears itself when audio is actually flowing again. A
/// stream the graph never revives stays in `Error` and the flag (and
/// banner) stay set until the app restarts.
fn on_state_changed(
    _stream: &pw::stream::Stream,
    user_data: &mut UserData,
    old: pw::stream::StreamState,
    new: pw::stream::StreamState,
) {
    match &new {
        pw::stream::StreamState::Error(e) => {
            user_data
                .shared
                .output_stream_lost
                .store(true, Ordering::Release);
            eprintln!("audio: PipeWire output stream error (was {old:?}): {e}");
        }
        pw::stream::StreamState::Streaming => {
            if user_data
                .shared
                .output_stream_lost
                .swap(false, Ordering::AcqRel)
            {
                eprintln!("audio: PipeWire output stream recovered (was {old:?})");
            }
        }
        _ => {}
    }
}

/// Process callback fired by libpipewire's RT scheduler once per graph
/// cycle: dequeue the cycle buffer, run the engine mixer straight into
/// it, and publish the written chunk. No intermediate ring — this is
/// where the ALSA-shim periods used to live.
fn on_process(stream: &pw::stream::Stream, user_data: &mut UserData) {
    // Publish the playback-side latency (stream -> device, as reported
    // by the graph: includes downstream filters, device buffering and
    // configured offsets) for the engine's round-trip queries
    // (doc #260 finding #13). RT-safe struct fill.
    unsafe {
        let mut time = std::mem::zeroed::<pw::sys::pw_time>();
        if pw::sys::pw_stream_get_time_n(
            stream.as_raw_ptr(),
            &mut time,
            std::mem::size_of::<pw::sys::pw_time>(),
        ) == 0
        {
            let engine_rate = user_data.rate.load(Ordering::Relaxed);
            let samples = crate::platform::pw_delay_to_engine_samples(
                time.delay,
                time.rate.num,
                time.rate.denom,
                engine_rate,
            );
            user_data
                .shared
                .playback_latency_samples
                .store(samples, Ordering::Relaxed);
        }
    }
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    // Frames the graph wants this cycle (0 = driver didn't say; fall
    // back to buffer capacity, which MAP_BUFFERS sizes to the quantum).
    let requested = buffer.requested() as usize;
    let datas = buffer.datas_mut();
    if datas.is_empty() {
        return;
    }
    let channels = user_data.channels.load(Ordering::Relaxed).max(1) as usize;
    let stride = channels * std::mem::size_of::<f32>();

    let data = &mut datas[0];
    let max_bytes = data.as_raw().maxsize as usize;
    let Some(bytes) = data.data() else {
        return;
    };
    let capacity = bytes.len().min(max_bytes) / stride;
    let frames = if requested > 0 { requested.min(capacity) } else { capacity }
        .min(user_data.max_frames);
    if frames == 0 {
        return;
    }
    let byte_len = frames * stride;
    // PipeWire SHM buffers are normally 4-byte aligned; a misaligned
    // buffer would make the cast panic on the RT thread and tear the
    // stream down. Skip the cycle instead (the graph plays silence).
    let Ok(samples) = bytemuck::try_cast_slice_mut::<u8, f32>(&mut bytes[..byte_len]) else {
        *data.chunk_mut().offset_mut() = 0;
        *data.chunk_mut().stride_mut() = stride as i32;
        *data.chunk_mut().size_mut() = 0;
        return;
    };

    (user_data.mix)(samples, channels);

    *data.chunk_mut().offset_mut() = 0;
    *data.chunk_mut().stride_mut() = stride as i32;
    *data.chunk_mut().size_mut() = byte_len as u32;
}

impl Drop for PipeWireOutputHandle {
    fn drop(&mut self) {
        // Ordering: lock the loop so no callback can fire while we
        // tear down stream/listener/core; then unlock; then drop the
        // thread_loop, whose own Drop calls pw_thread_loop_stop()
        // (signals + joins the RT thread) before destroying the loop.
        let lock = self.thread_loop.lock();
        // SAFETY: ManuallyDrop fields are dropped in reverse order
        // (listener first because it borrows the stream).
        unsafe {
            ManuallyDrop::drop(&mut self.listener);
            ManuallyDrop::drop(&mut self.stream);
            ManuallyDrop::drop(&mut self._core);
            ManuallyDrop::drop(&mut self._context);
        }
        drop(lock);
        // SAFETY: thread_loop is the last to go.
        unsafe {
            ManuallyDrop::drop(&mut self.thread_loop);
        }
    }
}
