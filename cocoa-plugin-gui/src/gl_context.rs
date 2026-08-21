//! NSOpenGL setup: pixel format, context parameters, and the glow loader.
//!
//! This is the only module that knows the rendering backend is OpenGL
//! (macos-editor-plan.md §2): the Cocoa equivalent of the Wayland runtime's
//! `egl_context.rs`. If Apple ever removes GL, a `CAMetalLayer` + egui-wgpu
//! backend replaces this module (and the `NSOpenGLView` superclass in
//! `window_main_thread/view.rs`) behind the same `Editor` API, without
//! touching plugins.
//!
//! OpenGL on macOS is deprecated but ships and runs (GL 4.1 core at most; we
//! ask for a 3.2 core profile, the baseline `egui_glow` is happy with).

// Using deprecated GL is this module's entire, deliberate job (plan §2);
// the deprecation warnings carry no information here.
#![allow(deprecated)]

use std::sync::Arc;

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSOpenGLContext, NSOpenGLContextParameter, NSOpenGLPFAAlphaSize, NSOpenGLPFAColorSize,
    NSOpenGLPFADoubleBuffer, NSOpenGLPFAOpenGLProfile, NSOpenGLPixelFormat,
    NSOpenGLProfileVersion3_2Core,
};
use plugin_gui_core::EditorError;

/// Create the pixel format the editor view renders with: double-buffered
/// RGBA8, no depth/stencil (egui needs neither), 3.2 core profile.
pub(crate) fn pixel_format(_mtm: MainThreadMarker) -> Result<Retained<NSOpenGLPixelFormat>, EditorError> {
    // Zero-terminated attribute list, exactly as NSOpenGLPixelFormat's
    // initializer specifies.
    let attrs: [u32; 8] = [
        NSOpenGLPFAOpenGLProfile,
        NSOpenGLProfileVersion3_2Core,
        NSOpenGLPFADoubleBuffer,
        NSOpenGLPFAColorSize,
        24,
        NSOpenGLPFAAlphaSize,
        8,
        0,
    ];
    // SAFETY: `attrs` is a valid, zero-terminated attribute array that
    // outlives the call (the initializer copies what it needs).
    unsafe {
        NSOpenGLPixelFormat::initWithAttributes(
            NSOpenGLPixelFormat::alloc(),
            std::ptr::NonNull::new_unchecked(attrs.as_ptr() as *mut u32),
        )
    }
    .ok_or_else(|| EditorError::Cocoa("no matching NSOpenGLPixelFormat".to_string()))
}

/// Set the context's swap interval to 0 so `flushBuffer` never blocks on
/// vsync: frame pacing is done explicitly by the CADisplayLink tick (the
/// same reason the Wayland runtime sets eglSwapInterval(0) and paces via
/// frame callbacks) — a blocking swap would stall the main thread, which
/// on this platform is also the host UI's thread.
pub(crate) fn set_swap_interval_zero(ctx: &NSOpenGLContext) {
    let zero: i32 = 0;
    // SAFETY: `values` points at one GLint, which is exactly what the
    // SwapInterval parameter reads.
    unsafe {
        ctx.setValues_forParameter(
            std::ptr::NonNull::from(&zero),
            NSOpenGLContextParameter::SwapInterval,
        );
    }
}

/// Load the OpenGL framework and build a `glow::Context` from it. The
/// returned `Library` must stay alive as long as the glow context: glow
/// holds raw function pointers into it.
///
/// Must be called with a current GL context (the loader itself is
/// context-independent on macOS — symbols come from the framework, not the
/// context — but callers are about to use the result, so the invariant is
/// theirs anyway).
pub(crate) fn load_glow() -> Result<(Arc<glow::Context>, libloading::Library), EditorError> {
    // The framework path resolves through the dyld shared cache; the file
    // does not exist on disk on modern macOS but dlopen still finds it.
    const OPENGL_FRAMEWORK: &str = "/System/Library/Frameworks/OpenGL.framework/OpenGL";
    // SAFETY: loading a system framework runs no arbitrary init code
    // beyond the framework's own, which is the point of the call.
    let lib = unsafe { libloading::Library::new(OPENGL_FRAMEWORK) }
        .map_err(|e| EditorError::GlLoad(format!("OpenGL.framework: {e}")))?;

    // SAFETY: the loader only resolves symbols from the framework we just
    // opened; a missing symbol becomes a null pointer, which glow treats
    // as "function unavailable".
    let gl = unsafe {
        glow::Context::from_loader_function(|name| {
            lib.get::<*const std::ffi::c_void>(name.as_bytes())
                .map(|sym| *sym)
                .unwrap_or(std::ptr::null())
        })
    };
    Ok((Arc::new(gl), lib))
}
