// Unsichtbarer OpenGL-Kontext für projectM (ohne Fenster):
// macOS über CGL (OpenGL 4.1 Core), Linux über EGL (für Tests auch ohne Bildschirm).
// Der Kontext gehört dem Milkdrop-Thread; die App selbst zeichnet über Metal (wgpu).

use std::ffi::{c_void, CString};

use libloading::Library;

pub struct GlContext {
    lib: Library,
    #[cfg(not(target_os = "macos"))]
    egl_get_proc: unsafe extern "C" fn(*const std::ffi::c_char) -> *const c_void,
    destroy: Box<dyn FnMut()>,
}

impl GlContext {
    /// Adresse einer GL-Funktion (für glow)
    pub fn proc_address(&self, name: &str) -> *const c_void {
        let Ok(c) = CString::new(name) else { return std::ptr::null() };
        #[cfg(not(target_os = "macos"))]
        unsafe {
            let p = (self.egl_get_proc)(c.as_ptr());
            if !p.is_null() {
                return p;
            }
        }
        unsafe { self.lib.get::<*const c_void>(c.as_bytes_with_nul()).map(|s| *s).unwrap_or(std::ptr::null()) }
    }
}

impl Drop for GlContext {
    fn drop(&mut self) {
        (self.destroy)();
    }
}

#[cfg(target_os = "macos")]
pub fn create() -> Result<GlContext, String> {
    type Obj = *mut c_void;
    unsafe {
        let lib = Library::new("/System/Library/Frameworks/OpenGL.framework/OpenGL").map_err(|e| e.to_string())?;
        let choose: unsafe extern "C" fn(*const i32, *mut Obj, *mut i32) -> i32 = *lib.get(b"CGLChoosePixelFormat\0").map_err(|e| e.to_string())?;
        let create_ctx: unsafe extern "C" fn(Obj, Obj, *mut Obj) -> i32 = *lib.get(b"CGLCreateContext\0").map_err(|e| e.to_string())?;
        let set_current: unsafe extern "C" fn(Obj) -> i32 = *lib.get(b"CGLSetCurrentContext\0").map_err(|e| e.to_string())?;
        let destroy_pix: unsafe extern "C" fn(Obj) -> i32 = *lib.get(b"CGLDestroyPixelFormat\0").map_err(|e| e.to_string())?;
        let destroy_ctx: unsafe extern "C" fn(Obj) -> i32 = *lib.get(b"CGLDestroyContext\0").map_err(|e| e.to_string())?;
        // kCGLPFAOpenGLProfile = 99 → kCGLOGLPVersion_GL4_Core = 0x4100; ColorSize 8 = 24; AlphaSize 11 = 8;
        // Accelerated = 73; AllowOfflineRenderers = 96
        let attribs: [i32; 10] = [99, 0x4100, 8, 24, 11, 8, 73, 96, 0, 0];
        let mut pix: Obj = std::ptr::null_mut();
        let mut n = 0;
        let err = choose(attribs.as_ptr(), &mut pix, &mut n);
        if err != 0 || pix.is_null() {
            return Err(format!("CGLChoosePixelFormat: Fehler {err}"));
        }
        let mut ctx: Obj = std::ptr::null_mut();
        let err = create_ctx(pix, std::ptr::null_mut(), &mut ctx);
        destroy_pix(pix);
        if err != 0 || ctx.is_null() {
            return Err(format!("CGLCreateContext: Fehler {err}"));
        }
        let err = set_current(ctx);
        if err != 0 {
            destroy_ctx(ctx);
            return Err(format!("CGLSetCurrentContext: Fehler {err}"));
        }
        let ctx_addr = ctx as usize;
        Ok(GlContext {
            lib,
            destroy: Box::new(move || {
                set_current(std::ptr::null_mut());
                destroy_ctx(ctx_addr as Obj);
            }),
        })
    }
}

#[cfg(not(target_os = "macos"))]
pub fn create() -> Result<GlContext, String> {
    type Ptr = *mut c_void;
    const EGL_NONE: i32 = 0x3038;
    unsafe {
        let lib = Library::new("libEGL.so.1").map_err(|e| e.to_string())?;
        let get_proc: unsafe extern "C" fn(*const std::ffi::c_char) -> *const c_void = *lib.get(b"eglGetProcAddress\0").map_err(|e| e.to_string())?;
        let get_display: unsafe extern "C" fn(Ptr) -> Ptr = *lib.get(b"eglGetDisplay\0").map_err(|e| e.to_string())?;
        let get_platform_display: Option<unsafe extern "C" fn(u32, Ptr, *const isize) -> Ptr> = lib.get(b"eglGetPlatformDisplay\0").ok().map(|s| *s);
        let initialize: unsafe extern "C" fn(Ptr, *mut i32, *mut i32) -> u32 = *lib.get(b"eglInitialize\0").map_err(|e| e.to_string())?;
        let bind_api: unsafe extern "C" fn(u32) -> u32 = *lib.get(b"eglBindAPI\0").map_err(|e| e.to_string())?;
        let choose_config: unsafe extern "C" fn(Ptr, *const i32, *mut Ptr, i32, *mut i32) -> u32 = *lib.get(b"eglChooseConfig\0").map_err(|e| e.to_string())?;
        let create_context: unsafe extern "C" fn(Ptr, Ptr, Ptr, *const i32) -> Ptr = *lib.get(b"eglCreateContext\0").map_err(|e| e.to_string())?;
        let make_current: unsafe extern "C" fn(Ptr, Ptr, Ptr, Ptr) -> u32 = *lib.get(b"eglMakeCurrent\0").map_err(|e| e.to_string())?;
        let destroy_context: unsafe extern "C" fn(Ptr, Ptr) -> u32 = *lib.get(b"eglDestroyContext\0").map_err(|e| e.to_string())?;
        // Ohne Bildschirm: Mesa-„surfaceless“-Plattform, sonst Standardanzeige
        let mut dpy = match get_platform_display {
            Some(f) => f(0x31DD, std::ptr::null_mut(), std::ptr::null()),
            None => std::ptr::null_mut(),
        };
        let (mut major, mut minor) = (0, 0);
        if dpy.is_null() || initialize(dpy, &mut major, &mut minor) == 0 {
            dpy = get_display(std::ptr::null_mut());
            if dpy.is_null() || initialize(dpy, &mut major, &mut minor) == 0 {
                return Err("EGL nicht verfügbar".into());
            }
        }
        if bind_api(0x30A2) == 0 {
            return Err("EGL: OpenGL-API nicht verfügbar".into());
        }
        // EGL_RENDERABLE_TYPE = EGL_OPENGL_BIT
        let attribs = [0x3040, 0x8, EGL_NONE];
        let mut cfg: Ptr = std::ptr::null_mut();
        let mut n = 0;
        if choose_config(dpy, attribs.as_ptr(), &mut cfg, 1, &mut n) == 0 || n == 0 {
            return Err("EGL: keine passende Konfiguration".into());
        }
        // OpenGL 3.3 Core
        let ctx_attribs = [0x3098, 3, 0x30FB, 3, 0x30FD, 1, EGL_NONE];
        let ctx = create_context(dpy, cfg, std::ptr::null_mut(), ctx_attribs.as_ptr());
        if ctx.is_null() {
            return Err("EGL: Kontext konnte nicht erstellt werden".into());
        }
        if make_current(dpy, std::ptr::null_mut(), std::ptr::null_mut(), ctx) == 0 {
            return Err("EGL: Kontext konnte nicht aktiviert werden (surfaceless)".into());
        }
        let (d, c) = (dpy as usize, ctx as usize);
        Ok(GlContext {
            lib,
            egl_get_proc: get_proc,
            destroy: Box::new(move || {
                make_current(d as Ptr, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
                destroy_context(d as Ptr, c as Ptr);
            }),
        })
    }
}
