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
        let egl = unsafe {
            Egl::load_required().map_err(|e| EditorError::EglInit(e.to_string()))?
        };

        // Raw wl_display pointer for EGL — from wayland-backend.
        let display_ptr = conn.backend().display_ptr() as *mut c_void;

        let display = unsafe {
            egl.get_platform_display(
                EGL_PLATFORM_WAYLAND_KHR,
                display_ptr,
                &[egl::ATTRIB_NONE],
            )
            .map_err(|e| EditorError::EglInit(format!("get_platform_display: {e}")))?
        };

        egl.initialize(display)
            .map_err(|e| EditorError::EglInit(format!("initialize: {e}")))?;

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

        // Wrap the wl_surface in a wl_egl_window at the physical buffer size.
        let wl_egl_surface = WlEglSurface::new(wl_surface.id(), phys_w, phys_h)
            .map_err(|e| EditorError::EglSurface(e.to_string()))?;

        let surface = unsafe {
            egl.create_window_surface(
                display,
                config,
                wl_egl_surface.ptr() as egl::NativeWindowType,
                None,
            )
            .map_err(|e| EditorError::EglSurface(e.to_string()))?
        };

        Ok(Self {
            egl,
            display,
            _config: config,
            context,
            surface,
            wl_egl_surface,
            current_size: (phys_w, phys_h),
        })
    }

    pub fn make_current(&self) -> Result<(), EditorError> {
        self.egl
            .make_current(self.display, Some(self.surface), Some(self.surface), Some(self.context))
            .map_err(|e| EditorError::EglContext(format!("make_current: {e}")))
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
        let _ = self.egl.terminate(self.display);
    }
}
