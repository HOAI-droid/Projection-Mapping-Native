// libprojectM 4 (LGPL) wird zur Laufzeit geladen – die App läuft auch ohne; Milkdrop wird dann
// im UI mit einem Installationshinweis angezeigt (brew install projectm).

use std::ffi::{CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};

use libloading::Library;

pub type Handle = *mut c_void;

pub struct Api {
    _lib: Library,
    pub path: PathBuf,
    create: unsafe extern "C" fn() -> Handle,
    destroy: unsafe extern "C" fn(Handle),
    load_preset_file: unsafe extern "C" fn(Handle, *const c_char, bool),
    set_window_size: unsafe extern "C" fn(Handle, usize, usize),
    set_soft_cut_duration: Option<unsafe extern "C" fn(Handle, f64)>,
    set_preset_duration: Option<unsafe extern "C" fn(Handle, f64)>,
    set_hard_cut_enabled: Option<unsafe extern "C" fn(Handle, bool)>,
    set_preset_locked: Option<unsafe extern "C" fn(Handle, bool)>,
    set_mesh_size: Option<unsafe extern "C" fn(Handle, usize, usize)>,
    set_fps: Option<unsafe extern "C" fn(Handle, i32)>,
    set_texture_search_paths: Option<unsafe extern "C" fn(Handle, *const *const c_char, usize)>,
    pcm_add_float: unsafe extern "C" fn(Handle, *const f32, u32, c_int),
    render_frame_fbo: Option<unsafe extern "C" fn(Handle, u32)>,
}

/// Mögliche Orte der Bibliothek (Homebrew auf Apple Silicon/Intel, im App-Bundle, Linux)
pub fn candidates() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Ok(p) = std::env::var("PM_PROJECTM_LIB") {
        v.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join("../Frameworks/libprojectM-4.dylib"));
            v.push(dir.join("libprojectM-4.dylib"));
        }
    }
    for p in [
        "/opt/homebrew/lib/libprojectM-4.dylib",
        "/opt/homebrew/opt/projectm/lib/libprojectM-4.dylib",
        "/usr/local/lib/libprojectM-4.dylib",
        "/usr/local/opt/projectm/lib/libprojectM-4.dylib",
        "/usr/local/lib/libprojectM-4.so",
        "/usr/local/lib/libprojectM-4.so.4",
        "/usr/lib/x86_64-linux-gnu/libprojectM-4.so.4",
        "/usr/lib/aarch64-linux-gnu/libprojectM-4.so.4",
        "/usr/lib/libprojectM-4.so.4",
    ] {
        v.push(PathBuf::from(p));
    }
    v
}

impl Api {
    /// projectM ab 4.2 kann in ein eigenes Framebuffer-Objekt zeichnen – das braucht die App
    /// (ein unsichtbarer Kontext hat kein Fenster, in das 4.1 zeichnen würde)
    pub fn supports_fbo(&self) -> bool {
        self.render_frame_fbo.is_some()
    }

    pub fn load() -> Result<Self, String> {
        let mut last = String::from("libprojectM-4 nicht gefunden");
        for p in candidates() {
            if !p.exists() {
                continue;
            }
            match unsafe { Self::load_from(&p) } {
                Ok(api) => return Ok(api),
                Err(e) => last = format!("{}: {e}", p.display()),
            }
        }
        Err(last)
    }

    unsafe fn load_from(path: &Path) -> Result<Self, String> {
        unsafe {
            let lib = Library::new(path).map_err(|e| e.to_string())?;
            macro_rules! req {
                ($name:literal) => {
                    *lib.get(concat!($name, "\0").as_bytes()).map_err(|e| format!("{}: {e}", $name))?
                };
            }
            macro_rules! opt {
                ($name:literal) => {
                    lib.get(concat!($name, "\0").as_bytes()).ok().map(|s| *s)
                };
            }
            Ok(Self {
                create: req!("projectm_create"),
                destroy: req!("projectm_destroy"),
                load_preset_file: req!("projectm_load_preset_file"),
                set_window_size: req!("projectm_set_window_size"),
                set_soft_cut_duration: opt!("projectm_set_soft_cut_duration"),
                set_preset_duration: opt!("projectm_set_preset_duration"),
                set_hard_cut_enabled: opt!("projectm_set_hard_cut_enabled"),
                set_preset_locked: opt!("projectm_set_preset_locked"),
                set_mesh_size: opt!("projectm_set_mesh_size"),
                set_fps: opt!("projectm_set_fps"),
                set_texture_search_paths: opt!("projectm_set_texture_search_paths"),
                pcm_add_float: req!("projectm_pcm_add_float"),
                render_frame_fbo: opt!("projectm_opengl_render_frame_fbo"),
                path: path.to_path_buf(),
                _lib: lib,
            })
        }
    }
}

/// Eine projectM-Instanz (braucht einen aktuellen OpenGL-Kontext im selben Thread)
pub struct Instance<'a> {
    api: &'a Api,
    handle: Handle,
}

impl<'a> Instance<'a> {
    pub fn new(api: &'a Api, w: usize, h: usize, texture_paths: &[PathBuf]) -> Option<Self> {
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return None;
        }
        let inst = Self { api, handle };
        unsafe {
            (api.set_window_size)(handle, w, h);
            // Wechsel bestimmt die App (Szenen-Uhr), nicht projectM
            if let Some(f) = api.set_preset_duration {
                f(handle, 1.0e7);
            }
            if let Some(f) = api.set_hard_cut_enabled {
                f(handle, false);
            }
            if let Some(f) = api.set_preset_locked {
                f(handle, true);
            }
            if let Some(f) = api.set_mesh_size {
                f(handle, 64, 48);
            }
            if let Some(f) = api.set_fps {
                f(handle, 60);
            }
            if let Some(f) = api.set_texture_search_paths {
                let c: Vec<CString> = texture_paths.iter().filter_map(|p| CString::new(p.to_string_lossy().as_bytes()).ok()).collect();
                let ptrs: Vec<*const c_char> = c.iter().map(|s| s.as_ptr()).collect();
                if !ptrs.is_empty() {
                    f(handle, ptrs.as_ptr(), ptrs.len());
                }
            }
        }
        Some(inst)
    }

    pub fn set_size(&self, w: usize, h: usize) {
        unsafe { (self.api.set_window_size)(self.handle, w, h) }
    }

    pub fn load_preset(&self, path: &Path, smooth: bool, blend_seconds: f64) {
        let Ok(c) = CString::new(path.to_string_lossy().as_bytes()) else { return };
        unsafe {
            if let Some(f) = self.api.set_soft_cut_duration {
                f(self.handle, blend_seconds.max(0.0));
            }
            (self.api.load_preset_file)(self.handle, c.as_ptr(), smooth && blend_seconds > 0.0);
            // Sperre nach dem Laden erneuern (manche Versionen heben sie beim Wechsel auf)
            if let Some(f) = self.api.set_preset_locked {
                f(self.handle, true);
            }
        }
    }

    pub fn add_pcm(&self, mono: &[f32]) {
        if mono.is_empty() {
            return;
        }
        // 1 = PROJECTM_MONO
        unsafe { (self.api.pcm_add_float)(self.handle, mono.as_ptr(), mono.len() as u32, 1) }
    }

    /// In das Framebuffer-Objekt fbo zeichnen
    pub fn render(&self, fbo: u32) {
        if let Some(f) = self.api.render_frame_fbo {
            unsafe { f(self.handle, fbo) }
        }
    }
}

impl Drop for Instance<'_> {
    fn drop(&mut self) {
        unsafe { (self.api.destroy)(self.handle) }
    }
}
