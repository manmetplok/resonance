//! Native PipeWire input stream. Bypasses cpal's ALSA-via-
//! pipewire-alsa-plugin path, which silently downmixes any capture
//! request to two channels regardless of what we ask for — fatal for
//! pro-audio sources where each input is its own AUX port.
//!
//! Lifecycle: the public [`build`] function spawns a `ThreadLoop`
//! (PipeWire's RT-thread wrapper), creates a `Core` + `Stream` under
//! its lock, registers a process callback that pushes interleaved f32
//! into the same recording / monitor ringbufs the cpal backend used,
//! and returns a [`PipeWireInputHandle`] whose Drop cleanly tears the
//! whole thing down.
//!
//! Sample-rate / channel negotiation is asynchronous in PipeWire: the
//! values aren't known until the graph attaches the stream and fires
//! `param_changed`. The builder waits up to 500 ms for the first
//! event and returns the negotiated values; if no event arrives in
//! time it returns the *requested* values and the (yet-to-be-wired)
//! `AudioCommand::InputRateNegotiated` will eventually correct them.

use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use parking_lot::Mutex as PlMutex;
use pipewire as pw;
use pipewire::spa;
use ringbuf::traits::Producer;

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

/// Handle returned by [`build`]. Owns the PipeWire thread loop, the
/// core, the stream, and the registered listener. Drop order matters
/// (lock → drop stream/listener/core → unlock → drop thread loop), so
/// every field is wrapped in [`ManuallyDrop`] and the custom
/// `Drop` impl below sequences them.
pub(crate) struct PipeWireInputHandle {
    listener: ManuallyDrop<StreamListener<UserData>>,
    stream: ManuallyDrop<StreamRc>,
    _core: ManuallyDrop<CoreRc>,
    _context: ManuallyDrop<ContextRc>,
    thread_loop: ManuallyDrop<ThreadLoopRc>,
}

/// Closure-captured state shared with the process / param-changed
/// callbacks running on the PipeWire RT thread.
struct UserData {
    shared: Arc<SharedState>,
    rec_producer: Option<ringbuf::HeapProd<f32>>,
    mon_producer: Arc<PlMutex<ringbuf::HeapProd<f32>>>,
    /// Negotiated channels — written by the param_changed callback,
    /// read by `process` to know how to split incoming samples.
    /// Stored in an `AtomicU16` so the param_changed and process
    /// callbacks can race safely (param_changed updates rarely; the
    /// process callback reads every block).
    channels: Arc<AtomicU16>,
    /// Negotiated sample rate. Same shape as `channels`.
    rate: Arc<AtomicU32>,
    /// One-shot signal so the builder can wait for the first
    /// `param_changed` to land before returning.
    notify: Arc<(Mutex<bool>, Condvar)>,
    /// Number of samples we've already counted as "first callback" —
    /// used to log only the very first delivery for diagnostics.
    first_callback_logged: bool,
}

/// Build a PipeWire capture stream targeting `source_name` with at
/// least `desired_channels` channels at `sample_rate` (the engine's
/// rate). Returns the live handle plus the negotiated `(rate,
/// channels)` once the graph has attached the stream.
///
/// `rec_producer` is taken via `&mut Option<...>` so a failure during
/// PipeWire init (no daemon, missing libpipewire, etc.) leaves the
/// producer intact for the caller to retry with the cpal backend.
/// The producer is `.take()`n only after the connect path has
/// committed.
pub(crate) fn build(
    source_name: Option<&str>,
    shared: Arc<SharedState>,
    rec_producer: &mut Option<ringbuf::HeapProd<f32>>,
    mon_producer: Arc<PlMutex<ringbuf::HeapProd<f32>>>,
    sample_rate: u32,
    desired_channels: u16,
) -> Result<(PipeWireInputHandle, u32, u16), String> {
    pw::init();

    // SAFETY: `ThreadLoopRc::new` is marked unsafe because the
    // resulting loop must outlive any objects created against it; we
    // satisfy that by storing the loop in the same `PipeWireInputHandle`
    // as the stream and ordering Drop so the loop is destroyed last.
    let thread_loop = unsafe {
        ThreadLoopRc::new(Some("resonance-input"), None)
            .map_err(|e| format!("PipeWire ThreadLoop::new: {e}"))?
    };
    // start() spawns the internal RT thread; ThreadLoopRc's Drop
    // calls pw_thread_loop_stop / destroy to join + free it.
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
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Production",
        *pw::keys::NODE_NAME => "resonance-input",
        *pw::keys::APP_NAME => "resonance-app",
        *pw::keys::NODE_LATENCY => format!("1024/{}", sample_rate).as_str(),
    };
    if let Some(name) = source_name {
        props.insert(*pw::keys::TARGET_OBJECT, name);
    }

    let stream =
        StreamRc::new(core.clone(), "resonance-input", props)
            .map_err(|e| format!("PipeWire Stream::new: {e}"))?;

    let channels_atomic = Arc::new(AtomicU16::new(desired_channels));
    let rate_atomic = Arc::new(AtomicU32::new(sample_rate));
    let notify = Arc::new((Mutex::new(false), Condvar::new()));

    // Past every fallible setup step — safe to take ownership of the
    // recording producer now. If we'd taken it earlier and bailed out
    // of `Context::new` / `connect_rc`, the dispatcher's cpal
    // fallback would have nothing to feed.
    let user_data = UserData {
        shared,
        rec_producer: rec_producer.take(),
        mon_producer,
        channels: Arc::clone(&channels_atomic),
        rate: Arc::clone(&rate_atomic),
        notify: Arc::clone(&notify),
        first_callback_logged: false,
    };

    let listener = stream
        .add_local_listener_with_user_data(user_data)
        .param_changed(on_param_changed)
        .process(on_process)
        .register()
        .map_err(|e| format!("PipeWire Stream::register: {e}"))?;

    // Ask the graph for f32 audio at the engine's rate with at least
    // `desired_channels` channels. Omitting `position` leaves the
    // server free to lay out the ports however the source wants —
    // FL/FR for stereo sources, AUX0..AUXN for pro-audio, etc.
    let mut audio_info = AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(sample_rate);
    audio_info.set_channels(desired_channels as u32);
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
            spa::utils::Direction::Input,
            None,
            StreamFlags::AUTOCONNECT
                | StreamFlags::MAP_BUFFERS
                | StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(|e| format!("PipeWire Stream::connect: {e}"))?;

    // Release the loop lock so the RT thread can attach the stream
    // and fire the first `param_changed`.
    drop(lock);

    // Wait briefly for the first `param_changed` so the caller has
    // real rate/channel values to feed `RecordingState`. If the wait
    // times out we return the requested values; a late param_changed
    // can still post `AudioCommand::InputRateNegotiated` to fix it
    // once that command is wired up.
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
    eprintln!(
        "[input] pipewire negotiated: rate={} channels={}",
        negotiated_rate, negotiated_channels
    );

    Ok((
        PipeWireInputHandle {
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

/// Listener callback fired whenever the stream's params change. We
/// only care about the format param: parse the negotiated audio info,
/// store rate + channels in the shared atomics, and notify the
/// builder if it's still waiting for the first event.
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

/// Process callback fired by libpipewire's RT scheduler. Pulls a
/// buffer of interleaved f32, gates on `recording` / `monitoring`
/// atomics, and pushes into the corresponding ringbuf — the same
/// contract `recording::drain_ring_to_buffers` consumes today.
fn on_process(stream: &pw::stream::Stream, user_data: &mut UserData) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let datas = buffer.datas_mut();
    if datas.is_empty() {
        return;
    }

    // Pro-audio / DSP profile delivers one chunk per channel; consumer
    // profiles deliver a single interleaved chunk. We pick which path
    // based on `datas.len()` so the same backend handles both.
    let channels = user_data.channels.load(Ordering::Relaxed) as usize;
    let chunk_count = datas.len();

    if !user_data.first_callback_logged {
        user_data.first_callback_logged = true;
        eprintln!(
            "[input] pipewire first callback: chunks={} channels(negotiated)={}",
            chunk_count, channels
        );
    }

    if chunk_count == 1 {
        // Interleaved path: cast the byte slice to &[f32] and push.
        let chunk = &mut datas[0];
        let size = chunk.chunk().size() as usize;
        let Some(bytes) = chunk.data() else {
            return;
        };
        let len = (size / std::mem::size_of::<f32>()).min(bytes.len() / std::mem::size_of::<f32>());
        if len == 0 {
            return;
        }
        let samples: &[f32] = bytemuck::cast_slice(&bytes[..len * std::mem::size_of::<f32>()]);
        push_to_ringbufs(user_data, samples);
    } else {
        // Planar path (DSP / pro-audio): each chunk is a separate
        // mono channel. Walk every chunk once to collect (size,
        // bytes) pairs into a Vec, then re-interleave into a small
        // stack burst buffer so the consumer contract (interleaved
        // f32) is preserved. Single-pass collection avoids the
        // disjoint-borrow problem of indexing `datas` in a loop.
        let used_channels = chunk_count.min(channels.max(1));
        // Stride large enough for a typical quantum (≤ 1024) without
        // chunking the inner re-interleave loop; bounded statically
        // so no allocation happens on the RT thread.
        const MAX_FRAMES: usize = 4096;
        const MAX_CHANNELS: usize = 16;
        let mut planes: [Option<&[f32]>; MAX_CHANNELS] = [None; MAX_CHANNELS];
        let mut min_frames = MAX_FRAMES;
        let used = used_channels.min(MAX_CHANNELS);
        for (ch, data) in datas.iter_mut().take(used).enumerate() {
            let chunk_size = data.chunk().size() as usize;
            let Some(bytes) = data.data() else {
                return;
            };
            let frames_here = chunk_size.min(bytes.len()) / std::mem::size_of::<f32>();
            if frames_here == 0 {
                return;
            }
            min_frames = min_frames.min(frames_here);
            let take_bytes = frames_here * std::mem::size_of::<f32>();
            planes[ch] = Some(bytemuck::cast_slice(&bytes[..take_bytes]));
        }
        let frames = min_frames;
        if frames == 0 || used == 0 {
            return;
        }

        // Re-interleave in 256-frame strides so the temp buffer stays
        // small. Bounded by `used * STRIDE`; with used ≤ 16 and stride
        // 256 that's a fixed 16 KiB stack scratch.
        const STRIDE: usize = 256;
        let mut scratch = [0.0f32; STRIDE * MAX_CHANNELS];
        let mut i = 0;
        while i < frames {
            let burst = (frames - i).min(STRIDE);
            for f in 0..burst {
                for c in 0..used {
                    let plane = match planes[c] {
                        Some(p) => p,
                        None => return,
                    };
                    scratch[f * used + c] = plane[i + f];
                }
            }
            push_to_ringbufs(user_data, &scratch[..burst * used]);
            i += burst;
        }
    }
}

#[inline]
fn push_to_ringbufs(user_data: &mut UserData, samples: &[f32]) {
    if user_data.shared.recording.load(Ordering::Relaxed) {
        if let Some(prod) = user_data.rec_producer.as_mut() {
            let pushed = prod.push_slice(samples);
            if pushed < samples.len() {
                user_data
                    .shared
                    .recording_overflow
                    .store(true, Ordering::Relaxed);
            }
        }
    }
    if user_data.shared.monitoring.load(Ordering::Relaxed) {
        if let Some(mut prod) = user_data.mon_producer.try_lock() {
            let _ = prod.push_slice(samples);
        }
    }
}

impl Drop for PipeWireInputHandle {
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
