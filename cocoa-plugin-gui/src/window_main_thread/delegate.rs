//! The editor window's `NSWindowDelegate`: close-button routing and
//! resize/backing-change feedback.
//!
//! Held strongly by [`super::EditorMain`] because `NSWindow.delegate` is a
//! weak reference.

use std::sync::OnceLock;

use objc2::encode::{Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyProtocol, Bool, ClassBuilder, NSObjectProtocol, Sel};
use objc2::{msg_send, sel, ClassType, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{NSWindow, NSWindowDelegate};
use objc2_foundation::{NSNotification, NSObject};

use super::runtime_class::{self, RuntimeClass, RuntimeDefined};
use super::view::EditorView;

pub(super) struct DelegateIvars {
    view: Retained<EditorView>,
}

/// The window delegate. Like [`EditorView`], its Objective-C class is
/// registered at runtime under a per-image name (see [`runtime_class`]).
#[repr(C)]
pub(super) struct WindowDelegate {
    superclass: NSObject,
}

// SAFETY: `WindowDelegate` is a `#[repr(C)]` wrapper around its
// superclass, and an Objective-C object pointer like it.
unsafe impl RefEncode for WindowDelegate {
    const ENCODING_REF: Encoding = NSObject::ENCODING_REF;
}

// SAFETY: a reference-counted Objective-C object.
unsafe impl Message for WindowDelegate {}

// SAFETY: `class()` is a subclass of `Super` (registered by
// `runtime_class::register`); main-thread-only because it holds the
// view; `#[repr(C)]` wrapper around `Super`.
unsafe impl ClassType for WindowDelegate {
    type Super = NSObject;
    type ThreadKind = dyn MainThreadOnly;
    /// The base name; the registered one carries a per-image suffix.
    const NAME: &'static str = "RESCocoaPluginWindowDelegate";

    fn class() -> &'static AnyClass {
        Self::runtime_class().class
    }

    fn as_super(&self) -> &Self::Super {
        &self.superclass
    }

    const __INNER: () = ();
    type __SubclassingType = Self;
}

impl std::ops::Deref for WindowDelegate {
    type Target = NSObject;

    fn deref(&self) -> &NSObject {
        &self.superclass
    }
}

// SAFETY: registered by `runtime_class::register::<Self>`; `#[repr(C)]`
// wrapper around `Super`.
unsafe impl RuntimeDefined for WindowDelegate {
    type Ivars = DelegateIvars;

    fn runtime_class() -> &'static RuntimeClass {
        static CLASS: OnceLock<RuntimeClass> = OnceLock::new();
        CLASS.get_or_init(|| runtime_class::register::<Self>(Self::NAME, register_delegate_methods))
    }
}

// SAFETY: the class inherits NSObject's conformance.
unsafe impl NSObjectProtocol for WindowDelegate {}

// SAFETY: the class declares the protocol, and implements the methods
// below with their protocol signatures.
unsafe impl NSWindowDelegate for WindowDelegate {}

fn register_delegate_methods(builder: &mut ClassBuilder) {
    /// The close button feeds the same `on_close`-exactly-once path a
    /// Wayland `xdg_toplevel.close` does; the view decides whether the
    /// close can proceed now or must wait out an in-flight frame.
    extern "C-unwind" fn window_should_close(
        this: &WindowDelegate,
        _: Sel,
        _sender: &NSWindow,
    ) -> Bool {
        Bool::new(this.ivars().view.handle_close_request())
    }
    /// Every applied size — user drag, zoom, or our own
    /// `setContentSize` — flows to the `SharedSize` mirror from here.
    extern "C-unwind" fn window_did_resize(this: &WindowDelegate, _: Sel, _n: &NSNotification) {
        this.ivars().view.publish_size();
    }
    extern "C-unwind" fn window_did_change_backing_properties(
        this: &WindowDelegate,
        _: Sel,
        _n: &NSNotification,
    ) {
        this.ivars().view.mark_repaint();
    }

    if let Some(protocol) = AnyProtocol::get(c"NSWindowDelegate") {
        builder.add_protocol(protocol);
    }
    // SAFETY: each function's signature matches its selector's.
    unsafe {
        builder.add_method(
            sel!(windowShouldClose:),
            window_should_close as extern "C-unwind" fn(_, _, _) -> _,
        );
        builder.add_method(
            sel!(windowDidResize:),
            window_did_resize as extern "C-unwind" fn(_, _, _),
        );
        builder.add_method(
            sel!(windowDidChangeBackingProperties:),
            window_did_change_backing_properties as extern "C-unwind" fn(_, _, _),
        );
    }
}

impl WindowDelegate {
    pub(super) fn new(mtm: MainThreadMarker, view: Retained<EditorView>) -> Retained<Self> {
        let this = runtime_class::alloc_with_ivars::<Self>(mtm, DelegateIvars { view });
        // SAFETY: NSObject's plain `init` on a freshly allocated instance.
        unsafe { msg_send![this, init] }
    }

    fn ivars(&self) -> &DelegateIvars {
        runtime_class::ivars(self)
    }
}
