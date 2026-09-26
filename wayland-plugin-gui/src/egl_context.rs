//! EGL context creation + management for a Wayland `wl_surface`.
//!
//! Dynamically loads `libEGL.so` at runtime via [`khronos_egl::DynamicInstance`]
//! so nothing has to link against EGL at compile time. The context is created
//! against the plugin's `wl_display` using `EGL_PLATFORM_WAYLAND_KHR`, and the
//! drawable is a `wl_egl_window` wrapping the `wl_surface`.

use std::ffi::c_void;

use khronos_egl as egl;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Proxy};
use wayland_egl::WlEglSurface;

use crate::error::EditorError;

pub type Egl = egl::DynamicInstance<egl::EGL1_5>;

/// `EGL_PLATFORM_WAYLAND_KHR` — from the `EGL_EXT_platform_wayland` / `EGL_KHR_platform_wayland`
/// extensions. `khronos-egl 6` does not expose this constant directly, so we define it locally.
const EGL_PLATFORM_WAYLAND_KHR: egl::Enum = 0x31D8;

/// What [`EglContext::build`] makes: config, context, window surface, the
/// `wl_egl_window` behind it, and its physical size.
type Built = (egl::Config, egl::Context, egl::Surface, WlEglSurface, (i32, i32));

pub struct EglContext {
    egl: Egl,
    display: egl::Display,
    _config: egl::Config,
    context: egl::Context,
    surface: egl::Surface,
    wl_egl_surface: WlEglSurface,
    current_size: (i32, i32),
}

impl EglContext {
    /// Set up EGL for the given Wayland connection and attach it to `wl_surface`.
    ///
    /// `logical_size` is in compositor logical pixels. `scale` is the integer
    /// buffer scale (usually 1 or 2 on KDE/GNOME). The underlying `wl_egl_window`
    /// is created at physical size (logical_size * scale) and `set_buffer_scale`
    /// is called on the surface so the compositor knows how to interpret it.
    pub fn new(
        conn: &Connection,
        wl_surface: &WlSurface,
        logical_size: (u32, u32),
        scale: i32,
    ) -> Result<Self, EditorError> {
        // SAFETY: `Egl::load_required` calls `dlopen("libEGL.so.1")` and
        // exposes the resolved entry points. Unsafe because Rust can't
        // verify the resolved symbols actually match the EGL 1.5 ABI we
        // claim — if the host has a buggy or wrong libEGL the resulting
        // function pointers would be unsound. We accept this: a missing
        // or ABI-incompatible libEGL means the plugin GUI cannot run at
        // all, which is the same failure mode as link-time EGL.
        let egl = unsafe { Egl::load_required().map_err(|e| EditorError::EglInit(e.to_string()))? };

        // Raw wl_display pointer for EGL — from wayland-backend.
        let display_ptr = conn.backend().display_ptr() as *mut c_void;

        // SAFETY: `get_platform_display` takes a raw `*mut c_void` it
        // assumes is a live `wl_display`. We sourced `display_ptr` from
        // the `Connection`'s own backend, so it points at a `wl_display`
        // owned by `conn` and remains valid for at least as long as
        // `conn` outlives this `EglContext`. The Wayland platform attrib
        // list is terminated with `ATTRIB_NONE` as the spec requires.
        let display = unsafe {
            egl.get_platform_display(EGL_PLATFORM_WAYLAND_KHR, display_ptr, &[egl::ATTRIB_NONE])
                .map_err(|e| EditorError::EglInit(format!("get_platform_display: {e}")))?
        };

        egl.initialize(display)
            .map_err(|e| EditorError::EglInit(format!("initialize: {e}")))?;

        // From here on the display is initialized and owns driver state (a
        // render-node fd, the DRI screen, an event queue and proxies on our
        // `wl_display`), so a failure must terminate it again — see `Drop`.
        match Self::build(&egl, display, wl_surface, logical_size, scale) {
            Ok((config, context, surface, wl_egl_surface, current_size)) => Ok(Self {
                egl,
                display,
                _config: config,
                context,
                surface,
                wl_egl_surface,
                current_size,
            }),
            Err(err) => {
                let _ = egl.terminate(display);
                Err(err)
            }
        }
    }

    /// Everything after `eglInitialize`: config, context and window
    /// surface. Cleans up the context itself if the surface fails; the
    /// caller terminates the display on any error.
    fn build(
        egl: &Egl,
        display: egl::Display,
        wl_surface: &WlSurface,
        logical_size: (u32, u32),
        scale: i32,
    ) -> Result<Built, EditorError> {
        egl.bind_api(egl::OPENGL_API)
            .map_err(|e| EditorError::EglInit(format!("bind_api: {e}")))?;

        let config_attribs = [
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_BIT,
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::DEPTH_SIZE,
            0,
            egl::STENCIL_SIZE,
            8,
            egl::NONE,
        ];

        let config = egl
            .choose_first_config(display, &config_attribs)
            .map_err(|e| EditorError::EglInit(format!("choose_config: {e}")))?
            .ok_or(EditorError::EglNoConfig)?;

        // egui_glow uses `#version 140` shaders (no Core profile), so we ask
        // for a GL 3.0 compatibility context — high enough for egui's VAO
        // requirements, low enough that GLSL 140 is accepted without Core
        // profile strictness.
        let context_attribs = [
            egl::CONTEXT_MAJOR_VERSION,
            3,
            egl::CONTEXT_MINOR_VERSION,
            0,
            egl::NONE,
        ];

        let context = egl
            .create_context(display, config, None, &context_attribs)
            .map_err(|e| EditorError::EglContext(e.to_string()))?;

        // Tell the compositor to treat the buffer we'll attach as being at
        // `scale` physical pixels per logical pixel. Without this, KDE/GNOME
        // will not display a buffer whose physical size differs from the
        // logical surface size. The set_buffer_scale state is applied on the
        // next surface commit, which happens implicitly inside eglSwapBuffers
        // once we actually draw a frame — so we don't commit here.
        let phys_w = logical_size.0 as i32 * scale.max(1);
        let phys_h = logical_size.1 as i32 * scale.max(1);
        wl_surface.set_buffer_scale(scale.max(1));

        // The window surface stage; on failure the context goes too.
        let surfaces = (|| {
            // Wrap the wl_surface in a wl_egl_window at the physical buffer size.
            let wl_egl_surface = WlEglSurface::new(wl_surface.id(), phys_w, phys_h)
                .map_err(|e| EditorError::EglSurface(e.to_string()))?;

            // SAFETY: `create_window_surface` takes a `NativeWindowType` raw
            // pointer it treats as a native window handle. On Wayland the
            // platform-specific native window is a `wl_egl_window*`, which
            // `wl_egl_surface.ptr()` returns from a `WlEglSurface` whose
            // lifetime is tied to `self.wl_egl_surface` below. The EGL
            // surface borrows that handle for its lifetime, and `Drop` for
            // `EglContext` destroys the surface before `wl_egl_surface`
            // itself is dropped (field declaration order: `surface` before
            // `wl_egl_surface`, and drop runs in declaration order).
            let surface = unsafe {
                egl.create_window_surface(
                    display,
                    config,
                    wl_egl_surface.ptr() as egl::NativeWindowType,
                    None,
                )
                .map_err(|e| EditorError::EglSurface(e.to_string()))?
            };
            Ok((surface, wl_egl_surface))
        })();
        let (surface, wl_egl_surface) = match surfaces {
            Ok(s) => s,
            Err(err) => {
                let _ = egl.destroy_context(display, context);
                return Err(err);
            }
        };

        Ok((config, context, surface, wl_egl_surface, (phys_w, phys_h)))
    }

    pub fn make_current(&self) -> Result<(), EditorError> {
        self.egl
            .make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
            .map_err(|e| EditorError::EglContext(format!("make_current: {e}")))
    }

    /// Disable the driver's implicit swap throttle (`eglSwapInterval(0)`).
    ///
    /// With the default interval of 1, Mesa's Wayland backend blocks
    /// inside `eglSwapBuffers` waiting for its own internal frame
    /// callback — which stalls the entire editor event loop (input
    /// included) whenever the compositor withholds frames from an
    /// occluded surface. The event loop paces frames explicitly via
    /// `wl_surface.frame()` callbacks instead, so swaps must not block.
    /// Wayland compositors always present complete frames, so interval 0
    /// cannot tear here. Must be called while the context is current.
    pub fn set_swap_interval_zero(&self) -> Result<(), EditorError> {
        self.egl
            .swap_interval(self.display, 0)
            .map_err(|e| EditorError::EglContext(format!("swap_interval: {e}")))
    }

    pub fn swap_buffers(&self) -> Result<(), EditorError> {
        self.egl
            .swap_buffers(self.display, self.surface)
            .map_err(|e| EditorError::EglContext(format!("swap_buffers: {e}")))
    }

    /// Resize the underlying wl_egl_window. `width` and `height` are in
    /// physical pixels. Must be called before the next draw after a configure
    /// event changes the surface size or the scale changes.
    pub fn resize(&mut self, phys_width: i32, phys_height: i32) {
        if (phys_width, phys_height) != self.current_size {
            self.wl_egl_surface.resize(phys_width, phys_height, 0, 0);
            self.current_size = (phys_width, phys_height);
        }
    }

    /// Resolve a GL function pointer by name. Used by `glow::Context::from_loader_function`.
    pub fn get_proc_address(&self, name: &str) -> *const c_void {
        self.egl
            .get_proc_address(name)
            .map(|p| p as *const c_void)
            .unwrap_or(std::ptr::null())
    }
}

impl Drop for EglContext {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_surface(self.display, self.surface);
        let _ = self.egl.destroy_context(self.display, self.context);
        // Terminate the display (PLG-02). It is NOT shared: EGL keys a
        // platform display on the native display pointer, and that is this
        // editor's private `wl_display` — every editor thread opens its own
        // `Connection`, and the host's own GL (if any) runs on a different
        // one. Leaving it initialized leaked a DRM render-node fd and the
        // driver's screen per editor open, and left Mesa holding an
        // initialized display keyed by a pointer that is about to be
        // disconnected; a later editor whose `wl_display` landed at the same
        // address got that stale display back, proxies and event queue from
        // a dead connection included.
        //
        // Order matters: the driver's event queue and proxies live on our
        // `wl_display`, so this must run before the `Connection` drops.
        // `EditorThread::run_inner` guarantees that on every exit path:
        // `egl_ctx` is declared after (so dropped before) the connection,
        // the event loop and the state that hold it.
        let _ = self.egl.terminate(self.display);
        // Drop this thread's EGL bookkeeping too, so the editor thread
        // leaves nothing bound to the terminated display behind.
        let _ = self.egl.release_thread();
    }
}
