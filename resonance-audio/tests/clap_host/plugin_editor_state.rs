//! The engine's editor open / failed-to-open / closed reporting
//! (ba doc #283, ba todo #1347, audit finding X4 in doc #275).
//!
//! Before this, `handle_open_plugin_editor` reported a failure as a bare
//! `AudioEvent::Error("Failed to open plugin editor")` — no instance id,
//! no reason — and reported success not at all, so the app marked a slot
//! `editor_open = true` the moment it sent the command and nothing could
//! ever correct it. A plugin whose window failed to open left the slot
//! reading "Close Editor" over nothing.
//!
//! These tests pin the failure path in particular, since that is the one
//! that used to lie: every step of the CLAP GUI negotiation
//! (`is_api_supported` → `create` → `show`) must come back as its own
//! [`PluginEditorFailure`], a failed `show` must roll its `create` back,
//! and the plugin-initiated `clap_host_gui.closed()` (the user closing
//! the floating window from its own titlebar) must be picked up.
//!
//! They drive a hand-rolled fake CLAP plugin built straight from
//! `clap_sys` vtables through the `__instance_from_raw_for_test` hook —
//! no shared library and no compositor involved, so the whole matrix
//! runs headless. See `tests/clap_host/clap_latency_tracking.rs` for the same
//! harness applied to the latency callbacks.

use std::ffi::{c_char, c_void, CStr};
use std::ptr;

use clap_sys::ext::gui::{clap_host_gui, clap_plugin_gui, CLAP_EXT_GUI};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;

use resonance_audio::test_support::{
    plugin_editor_failure_events, ClapInstance, __instance_from_raw_for_test,
};
use resonance_audio::types::{AudioEvent, PluginEditorFailure};

// ---------------------------------------------------------------------------
// Fake plugin with a configurable GUI extension
// ---------------------------------------------------------------------------

/// How the fake plugin's GUI extension behaves. Each flag corresponds to
/// one refusal the host has to classify.
#[derive(Clone, Copy)]
struct GuiBehaviour {
    /// Serve `clap.gui` from `get_extension` at all.
    has_gui: bool,
    /// `is_api_supported` verdict for (wayland, floating).
    api_supported: bool,
    /// `create` verdict.
    create_ok: bool,
    /// Expose a `show` entry point.
    provide_show: bool,
    /// `show` verdict.
    show_ok: bool,
}

impl GuiBehaviour {
    /// Everything works.
    fn working() -> Self {
        Self {
            has_gui: true,
            api_supported: true,
            create_ok: true,
            provide_show: true,
            show_ok: true,
        }
    }
}

struct FakeState {
    host: *const clap_host,
    gui: GuiBehaviour,
    create_calls: u32,
    show_calls: u32,
    hide_calls: u32,
    destroy_calls: u32,
}

unsafe fn fake_state<'a>(plugin: *const clap_plugin) -> &'a mut FakeState {
    &mut *((*plugin).plugin_data as *mut FakeState)
}

unsafe extern "C" fn fake_init(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn fake_destroy(_plugin: *const clap_plugin) {}
unsafe extern "C" fn fake_activate(
    _plugin: *const clap_plugin,
    _sample_rate: f64,
    _min_frames: u32,
    _max_frames: u32,
) -> bool {
    true
}
unsafe extern "C" fn fake_deactivate(_plugin: *const clap_plugin) {}
unsafe extern "C" fn fake_start_processing(_plugin: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn fake_stop_processing(_plugin: *const clap_plugin) {}

// -- clap_plugin_gui entry points --

unsafe extern "C" fn gui_is_api_supported(
    plugin: *const clap_plugin,
    _api: *const c_char,
    _is_floating: bool,
) -> bool {
    fake_state(plugin).gui.api_supported
}

unsafe extern "C" fn gui_create(
    plugin: *const clap_plugin,
    _api: *const c_char,
    _is_floating: bool,
) -> bool {
    let state = fake_state(plugin);
    state.create_calls += 1;
    state.gui.create_ok
}

unsafe extern "C" fn gui_destroy(plugin: *const clap_plugin) {
    fake_state(plugin).destroy_calls += 1;
}

unsafe extern "C" fn gui_get_size(
    _plugin: *const clap_plugin,
    width: *mut u32,
    height: *mut u32,
) -> bool {
    *width = 640;
    *height = 480;
    true
}

unsafe extern "C" fn gui_set_size(
    _plugin: *const clap_plugin,
    _width: u32,
    _height: u32,
) -> bool {
    true
}

unsafe extern "C" fn gui_show(plugin: *const clap_plugin) -> bool {
    let state = fake_state(plugin);
    state.show_calls += 1;
    state.gui.show_ok
}

unsafe extern "C" fn gui_hide(plugin: *const clap_plugin) -> bool {
    fake_state(plugin).hide_calls += 1;
    true
}

const GUI_BASE: clap_plugin_gui = clap_plugin_gui {
    is_api_supported: Some(gui_is_api_supported),
    get_preferred_api: None,
    create: Some(gui_create),
    destroy: Some(gui_destroy),
    set_scale: None,
    get_size: Some(gui_get_size),
    can_resize: None,
    get_resize_hints: None,
    adjust_size: None,
    set_size: Some(gui_set_size),
    set_parent: None,
    set_transient: None,
    suggest_title: None,
    show: Some(gui_show),
    hide: Some(gui_hide),
};

static FAKE_GUI_EXT: clap_plugin_gui = clap_plugin_gui {
    show: Some(gui_show),
    ..GUI_BASE
};

/// Same plugin, but without a `show` entry point — a host that walks the
/// negotiation to the end has to treat that as a `ShowFailed` and roll
/// the `create` back rather than leave a created-but-invisible window.
static FAKE_GUI_EXT_NO_SHOW: clap_plugin_gui = clap_plugin_gui {
    show: None,
    ..GUI_BASE
};

unsafe extern "C" fn fake_get_extension(
    plugin: *const clap_plugin,
    id: *const c_char,
) -> *const c_void {
    let state = fake_state(plugin);
    if CStr::from_ptr(id).to_bytes() == CLAP_EXT_GUI.to_bytes() && state.gui.has_gui {
        return if state.gui.provide_show {
            &FAKE_GUI_EXT as *const clap_plugin_gui as *const c_void
        } else {
            &FAKE_GUI_EXT_NO_SHOW as *const clap_plugin_gui as *const c_void
        };
    }
    ptr::null()
}

/// Build a `ClapInstance` around a fresh fake plugin with the given GUI
/// behaviour, plus a raw pointer to its backing state so tests can count
/// the CLAP calls the host made. Plugin and state are intentionally
/// leaked — the instance's `Drop` still dereferences them.
fn make_instance(gui: GuiBehaviour) -> (ClapInstance, *mut FakeState) {
    let mut state_ptr: *mut FakeState = ptr::null_mut();
    let instance = __instance_from_raw_for_test(
        |host| {
            let state = Box::into_raw(Box::new(FakeState {
                host,
                gui,
                create_calls: 0,
                show_calls: 0,
                hide_calls: 0,
                destroy_calls: 0,
            }));
            state_ptr = state;
            let plugin = Box::new(clap_plugin {
                desc: ptr::null(),
                plugin_data: state as *mut c_void,
                init: Some(fake_init),
                destroy: Some(fake_destroy),
                activate: Some(fake_activate),
                deactivate: Some(fake_deactivate),
                start_processing: Some(fake_start_processing),
                stop_processing: Some(fake_stop_processing),
                reset: None,
                process: None,
                get_extension: Some(fake_get_extension),
                on_main_thread: None,
            });
            Box::into_raw(plugin) as *const clap_plugin
        },
        48_000,
    )
    .expect("fake plugin instance");
    (instance, state_ptr)
}

/// Fire `clap_host_gui.closed(was_destroyed)` exactly as a plugin whose
/// floating window the user just closed would.
fn plugin_reports_closed(state: &FakeState, was_destroyed: bool) {
    let host = state.host;
    let get_ext = unsafe { (*host).get_extension }.expect("host get_extension");
    let ext = unsafe { get_ext(host, CLAP_EXT_GUI.as_ptr()) } as *const clap_host_gui;
    assert!(
        !ext.is_null(),
        "host must serve clap_host_gui so a plugin can report a user-initiated close"
    );
    let closed = unsafe { (*ext).closed }.expect("clap_host_gui.closed");
    unsafe { closed(host, was_destroyed) };
}

// ---------------------------------------------------------------------------
// Failure path — the one that used to lie
// ---------------------------------------------------------------------------

#[test]
fn plugin_without_gui_reports_no_editor() {
    let (mut instance, state) = make_instance(GuiBehaviour {
        has_gui: false,
        ..GuiBehaviour::working()
    });
    let state = unsafe { &*state };

    assert!(!instance.has_gui());
    assert_eq!(instance.open_gui(), Err(PluginEditorFailure::NoEditor));
    assert!(!instance.gui_open());
    assert_eq!(state.create_calls, 0);
    // Structural: no point offering "open editor" again.
    assert!(!PluginEditorFailure::NoEditor.is_transient());
}

#[test]
fn unsupported_window_api_is_reported_as_such() {
    let (mut instance, state) = make_instance(GuiBehaviour {
        api_supported: false,
        ..GuiBehaviour::working()
    });
    let state = unsafe { &*state };

    assert_eq!(
        instance.open_gui(),
        Err(PluginEditorFailure::UnsupportedWindowApi)
    );
    assert!(!instance.gui_open());
    assert_eq!(state.create_calls, 0, "must not create after a refused API");
    assert!(!PluginEditorFailure::UnsupportedWindowApi.is_transient());
}

#[test]
fn failed_create_is_reported_as_create_failed() {
    // This is the shape of the ba-todo-#1352 failure: the plugin's
    // windowing stack is broken (third editor in one process), so
    // `create` dies. It must surface as a failure the app can explain,
    // not as a slot that claims to be open.
    let (mut instance, state) = make_instance(GuiBehaviour {
        create_ok: false,
        ..GuiBehaviour::working()
    });
    let state = unsafe { &*state };

    assert_eq!(instance.open_gui(), Err(PluginEditorFailure::CreateFailed));
    assert!(!instance.gui_open());
    assert_eq!(state.create_calls, 1);
    assert_eq!(state.show_calls, 0);
    assert_eq!(
        state.destroy_calls, 0,
        "nothing was created, so nothing to roll back"
    );
    // Transient: a later attempt can work, so the app keeps offering it.
    assert!(PluginEditorFailure::CreateFailed.is_transient());
}

#[test]
fn failed_show_rolls_the_create_back() {
    let (mut instance, state) = make_instance(GuiBehaviour {
        show_ok: false,
        ..GuiBehaviour::working()
    });
    let state = unsafe { &*state };

    assert_eq!(instance.open_gui(), Err(PluginEditorFailure::ShowFailed));
    assert!(!instance.gui_open(), "a failed open must not read as open");
    assert_eq!(state.create_calls, 1);
    assert_eq!(state.show_calls, 1);
    assert_eq!(
        state.destroy_calls, 1,
        "a created-but-unshown window must be destroyed"
    );
}

#[test]
fn missing_show_entry_point_rolls_the_create_back() {
    let (mut instance, state) = make_instance(GuiBehaviour {
        provide_show: false,
        ..GuiBehaviour::working()
    });
    let state = unsafe { &*state };

    assert_eq!(instance.open_gui(), Err(PluginEditorFailure::ShowFailed));
    assert!(!instance.gui_open());
    assert_eq!(state.create_calls, 1);
    assert_eq!(state.destroy_calls, 1);
}

#[test]
fn failure_events_carry_the_instance_id_and_a_reason() {
    let events = plugin_editor_failure_events(4242, PluginEditorFailure::CreateFailed);

    match &events[0] {
        AudioEvent::PluginEditorState {
            instance_id,
            open,
            failure,
        } => {
            assert_eq!(*instance_id, 4242, "the app must be able to correlate");
            assert!(!*open, "a failed open must report the editor as closed");
            assert_eq!(*failure, Some(PluginEditorFailure::CreateFailed));
        }
        other => panic!("expected PluginEditorState, got {other:?}"),
    }

    // The banner text is no longer the bare "Failed to open plugin
    // editor" — it names the instance and the reason.
    match &events[1] {
        AudioEvent::Error(msg) => {
            assert!(msg.contains("4242"), "error must name the instance: {msg}");
            assert!(
                msg.contains(PluginEditorFailure::CreateFailed.message()),
                "error must carry the reason: {msg}"
            );
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn every_failure_has_its_own_message() {
    let all = [
        PluginEditorFailure::UnknownInstance,
        PluginEditorFailure::NoEditor,
        PluginEditorFailure::UnsupportedWindowApi,
        PluginEditorFailure::CreateFailed,
        PluginEditorFailure::ShowFailed,
    ];
    for (i, a) in all.iter().enumerate() {
        assert!(!a.message().is_empty());
        for b in &all[i + 1..] {
            assert_ne!(
                a.message(),
                b.message(),
                "{a:?} and {b:?} would be indistinguishable to a user"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Success + close paths
// ---------------------------------------------------------------------------

#[test]
fn successful_open_and_host_close_are_both_transitions() {
    let (mut instance, state) = make_instance(GuiBehaviour::working());
    let state_ref = unsafe { &*state };

    assert_eq!(instance.open_gui(), Ok(()));
    assert!(instance.gui_open());
    assert_eq!(state_ref.create_calls, 1);
    assert_eq!(state_ref.show_calls, 1);

    // Re-opening an open editor is a no-op success, not a second window.
    assert_eq!(instance.open_gui(), Ok(()));
    assert_eq!(state_ref.create_calls, 1);

    assert!(instance.close_gui(), "closing an open editor is a transition");
    assert!(!instance.gui_open());
    assert_eq!(state_ref.hide_calls, 1);
    assert_eq!(state_ref.destroy_calls, 1);

    // Closing again changed nothing, so it must not be reported as a
    // transition (the app would otherwise see phantom events).
    assert!(!instance.close_gui());
    assert_eq!(state_ref.destroy_calls, 1);
}

#[test]
fn titlebar_close_is_picked_up_and_the_host_destroys_the_window() {
    let (mut instance, state) = make_instance(GuiBehaviour::working());
    let state_ref = unsafe { &*state };
    assert_eq!(instance.open_gui(), Ok(()));

    // Nothing pending yet.
    assert!(!instance.take_gui_closed());

    // User clicks the window's own close button; the plugin tells the
    // host and leaves the teardown to it.
    plugin_reports_closed(state_ref, false);

    assert!(instance.take_gui_closed(), "the close must be reported");
    assert!(!instance.gui_open());
    assert_eq!(
        state_ref.destroy_calls, 1,
        "was_destroyed=false means the host owns the destroy"
    );
    // Consumed exactly once.
    assert!(!instance.take_gui_closed());
}

#[test]
fn titlebar_close_with_was_destroyed_is_acknowledged_with_a_destroy() {
    let (mut instance, state) = make_instance(GuiBehaviour::working());
    let state_ref = unsafe { &*state };
    assert_eq!(instance.open_gui(), Ok(()));

    plugin_reports_closed(state_ref, true);

    assert!(instance.take_gui_closed());
    assert!(!instance.gui_open());
    assert_eq!(
        state_ref.destroy_calls, 1,
        "clap/ext/gui.h: \"If was_destroyed is true, then the host must call \
         clap_plugin_gui->destroy() to acknowledge the gui destruction.\" \
         was_destroyed describes the WINDOW; the plugin never frees its own \
         gui object, so destroy is the host's call either way"
    );
}

/// The property the two tests above are really protecting, stated once as
/// a sequence rather than as a call count.
///
/// A plugin-initiated close must leave the plugin's own create/destroy
/// pairing balanced, so that re-opening allocates on top of nothing. When
/// `take_gui_closed` skipped the destroy, `gui_open` was still cleared —
/// so the next `open_gui` sailed past its already-open guard and called
/// `create` a second time on a gui that had never been destroyed. That is
/// the shape of ba todo #1352 (a third editor dying in EGL init), which
/// is why this pins the sequence and not just the teardown.
#[test]
fn reopening_after_a_titlebar_close_creates_only_on_a_balanced_teardown() {
    for was_destroyed in [true, false] {
        let (mut instance, state) = make_instance(GuiBehaviour::working());
        let state_ref = unsafe { &*state };

        assert_eq!(instance.open_gui(), Ok(()));
        assert_eq!(state_ref.create_calls, 1);
        assert_eq!(state_ref.destroy_calls, 0);

        plugin_reports_closed(state_ref, was_destroyed);
        assert!(instance.take_gui_closed());
        assert_eq!(
            state_ref.destroy_calls, 1,
            "was_destroyed={was_destroyed}: the close must be acknowledged \
             with exactly one destroy"
        );

        assert_eq!(instance.open_gui(), Ok(()));
        assert_eq!(
            (state_ref.create_calls, state_ref.destroy_calls),
            (2, 1),
            "was_destroyed={was_destroyed}: re-opening must create again, and \
             every create must be preceded by the destroy of the one before \
             it — two creates against one destroy is an allocation on top of \
             a live gui"
        );
    }
}

#[test]
fn close_notification_from_a_previous_window_is_not_reported() {
    let (mut instance, state) = make_instance(GuiBehaviour::working());
    let state_ref = unsafe { &*state };

    // A `closed()` that arrives while no editor is open (e.g. fired as
    // part of the host's own teardown) must not produce an event, and
    // must not be attributed to the NEXT window either.
    assert_eq!(instance.open_gui(), Ok(()));
    assert!(instance.close_gui());
    plugin_reports_closed(state_ref, true);
    assert!(!instance.take_gui_closed());

    assert_eq!(instance.open_gui(), Ok(()));
    assert!(
        !instance.take_gui_closed(),
        "a stale notification must not close the freshly opened editor"
    );
    assert!(instance.gui_open());
}
