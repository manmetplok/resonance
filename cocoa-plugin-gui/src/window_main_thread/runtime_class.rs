//! Objective-C classes registered at runtime under a name unique to the
//! loaded image.
//!
//! Every plugin bundle links its own copy of this crate, and the
//! Objective-C class namespace is one flat table per process. A fixed
//! class name therefore worked for the first plugin whose editor opened
//! and aborted the host on the second one (`define_class!` panics on a
//! duplicate name, and the panic cannot unwind out of the main-thread
//! dispatch). Letting `define_class!` auto-name the class does not help
//! either: on a clash it silently reuses the other image's class, so one
//! plugin's windows would run a second plugin's copy of this code — fine
//! only while every bundle comes from one build and none is unloaded.
//!
//! A per-image name cannot be a compile-time constant, since cargo builds
//! this crate once for all plugins. So the classes are built with
//! [`ClassBuilder`] under `<base>_<address of a static in this image>`,
//! and [`RuntimeDefined`] supplies what `define_class!` otherwise would:
//! the class, its Rust state (boxed, behind one pointer ivar), and a
//! `dealloc` that frees it.

use std::ffi::{c_void, CString};
use std::ptr::NonNull;

use objc2::rc::Allocated;
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Ivar, MessageReceiver, Sel};
use objc2::{sel, ClassType, MainThreadMarker, MainThreadOnly, Message};

/// Its address differs in every image that links this crate, which is
/// what makes the registered class names unique per plugin binary.
static IMAGE_ANCHOR: u8 = 0;

/// The ivar holding the `Box<Ivars>` pointer.
const IVAR_NAME: &std::ffi::CStr = c"resonanceIvars";

/// A class registered by [`register`], with its state ivar resolved.
pub(super) struct RuntimeClass {
    pub(super) class: &'static AnyClass,
    ivar: &'static Ivar,
}

/// A class whose Objective-C half is registered at runtime by [`register`]
/// and whose Rust state lives in a `Box<Self::Ivars>` owned by the object.
///
/// # Safety
///
/// `runtime_class` must return a class registered by [`register::<Self>`],
/// and `Self` must be a `#[repr(C)]` wrapper around `Self::Super`.
pub(super) unsafe trait RuntimeDefined: ClassType + Message + MainThreadOnly {
    type Ivars: 'static;

    fn runtime_class() -> &'static RuntimeClass;
}

/// Build and register `T`'s class: a subclass of `T::Super` named
/// `<base>_<image address>`, carrying the state ivar, a `dealloc` that
/// frees the state, and whatever `add_methods` adds.
pub(super) fn register<T: RuntimeDefined>(
    base: &str,
    add_methods: impl FnOnce(&mut ClassBuilder),
) -> RuntimeClass
where
    T::Super: ClassType,
{
    let name = CString::new(format!("{base}_{:x}", &IMAGE_ANCHOR as *const u8 as usize))
        .expect("class name has no interior NUL");
    let mut builder = ClassBuilder::new(&name, <T::Super as ClassType>::class())
        .unwrap_or_else(|| panic!("Objective-C class {name:?} is already registered"));
    builder.add_ivar::<*mut c_void>(IVAR_NAME);
    let dealloc: unsafe extern "C-unwind" fn(NonNull<T>, Sel) = dealloc::<T>;
    // SAFETY: `dealloc` has the `- (void)dealloc` signature.
    unsafe { builder.add_method(sel!(dealloc), dealloc) };
    add_methods(&mut builder);
    let class = builder.register();
    let ivar = class
        .instance_variable(IVAR_NAME)
        .expect("the state ivar was just added");
    RuntimeClass { class, ivar }
}

/// Allocate a `T` with its state already in place, so it is readable
/// from any method the superclass initializer happens to call. If the
/// initializer then fails, it releases the object and `dealloc` frees
/// the state.
pub(super) fn alloc_with_ivars<T: RuntimeDefined>(
    mtm: MainThreadMarker,
    ivars: T::Ivars,
) -> Allocated<T> {
    let this = T::alloc(mtm);
    let ptr = Allocated::as_ptr(&this);
    if !ptr.is_null() {
        // SAFETY: a live, freshly allocated instance of `T`'s class, which
        // carries the state ivar (`RuntimeDefined` contract).
        unsafe {
            let obj = &*(ptr as *const AnyObject);
            *T::runtime_class().ivar.load_ptr::<*mut c_void>(obj) =
                Box::into_raw(Box::new(ivars)).cast();
        }
    }
    this
}

/// The state [`alloc_with_ivars`] stored in `this`.
pub(super) fn ivars<T: RuntimeDefined>(this: &T) -> &T::Ivars {
    // SAFETY: every instance comes from `alloc_with_ivars`, which sets the
    // pointer before the object is initialized; only `dealloc` clears it,
    // and nothing borrows the object after that.
    unsafe {
        let obj = &*(this as *const T as *const AnyObject);
        let state = *T::runtime_class().ivar.load::<*mut c_void>(obj);
        debug_assert!(!state.is_null(), "{} used after dealloc", T::NAME);
        &*(state as *const T::Ivars)
    }
}

/// `- (void)dealloc`: free the Rust state, then run the superclass's.
unsafe extern "C-unwind" fn dealloc<T: RuntimeDefined>(this: NonNull<T>, _cmd: Sel)
where
    T::Super: ClassType,
{
    // SAFETY: `this` is the instance being deallocated; see `ivars`.
    let state = unsafe {
        let obj = &*(this.as_ptr() as *const AnyObject);
        std::mem::replace(
            &mut *T::runtime_class().ivar.load_ptr::<*mut c_void>(obj),
            std::ptr::null_mut(),
        )
    };
    if !state.is_null() {
        // Plugin state drops here (the `EditorApp` among it). A panic must
        // not unwind into the Objective-C runtime — that aborts the host.
        let dropped = std::panic::catch_unwind(|| {
            // SAFETY: the pointer came from `Box::into_raw` in
            // `alloc_with_ivars` and was taken out of the ivar above.
            drop(unsafe { Box::from_raw(state as *mut T::Ivars) });
        });
        if dropped.is_err() {
            tracing::error!("cocoa-plugin-gui: dropping {} state panicked", T::NAME);
        }
    }
    // SAFETY: `dealloc` takes no arguments and returns nothing; called
    // once, as the last thing this object does.
    unsafe {
        MessageReceiver::send_super_message(
            this,
            <T::Super as ClassType>::class(),
            sel!(dealloc),
            (),
        )
    }
}
