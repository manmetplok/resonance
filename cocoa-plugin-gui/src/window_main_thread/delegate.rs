//! The editor window's `NSWindowDelegate`: close-button routing and
//! resize/backing-change feedback.
//!
//! Held strongly by [`super::EditorMain`] because `NSWindow.delegate` is a
//! weak reference.

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{define_class, msg_send, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSWindow, NSWindowDelegate};
use objc2_foundation::{NSNotification, NSObject};

use super::view::EditorView;

pub(super) struct DelegateIvars {
    view: Retained<EditorView>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements;
    // `WindowDelegate` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RESCocoaPluginWindowDelegate"]
    #[ivars = DelegateIvars]
    pub(super) struct WindowDelegate;

    unsafe impl NSObjectProtocol for WindowDelegate {}

    unsafe impl NSWindowDelegate for WindowDelegate {
        /// The close button feeds the same `on_close`-exactly-once path a
        /// Wayland `xdg_toplevel.close` does; the view decides whether the
        /// close can proceed now or must wait out an in-flight frame.
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> bool {
            self.ivars().view.handle_close_request()
        }

        /// Every applied size — user drag, zoom, or our own
        /// `setContentSize` — flows to the `SharedSize` mirror from here.
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            self.ivars().view.publish_size();
        }

        #[unsafe(method(windowDidChangeBackingProperties:))]
        fn window_did_change_backing_properties(&self, _notification: &NSNotification) {
            self.ivars().view.mark_repaint();
        }
    }
);

impl WindowDelegate {
    pub(super) fn new(mtm: MainThreadMarker, view: Retained<EditorView>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars { view });
        // SAFETY: NSObject's plain `init` on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}
