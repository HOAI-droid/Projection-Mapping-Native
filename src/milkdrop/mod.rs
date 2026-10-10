// Milkdrop-Visualisierungen (die originalen Winamp-Presets) über projectM.
//
// projectM zeichnet mit OpenGL in einem eigenen Thread mit eigenem, unsichtbarem Kontext. Pro Fläche
// läuft eine Instanz; ihr Bild wird ausgelesen und als Textur in den Metal-Renderer übernommen – so
// lässt es sich auf jede Form legen, mit Masken und Verzerrungsgitter. Die Musik kommt als rohe
// Samples aus der Audio-Engine.

pub mod gl;
pub mod projectm;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::render::Renderer;

/// Mehr Milkdrop-Flächen teilen sich Instanzen (Rechenzeit)
const MAX_INSTANCES: usize = 8;

enum Cmd {
    Ensure { key: String, w: usize, h: usize },
    Preset { key: String, path: PathBuf, blend: f64, smooth: bool },
    Remove { key: String },
    Pcm(Vec<f32>),
    Render,
}

pub struct Frame {
    pub key: String,
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Off,
    Starting,
    Ready(String),
    Error(String),
}

enum Event {
    Status(Status),
    Frame(Frame),
}

struct Worker {
    tx: Sender<Cmd>,
    rx: Receiver<Event>,
}

struct InstState {
    w: usize,
    h: usize,
    preset: String,
}

pub struct MilkdropPool {
    worker: Option<Worker>,
    pub status: Status,
    instances: HashMap<String, InstState>,
    pub preset_dir: PathBuf,
    /// Presets relativ zum Ordner (sortiert)
    pub presets: Vec<String>,
    scan: Option<Receiver<Vec<String>>>,
    pub download: Option<Receiver<Result<(), String>>>,
    pub download_error: Option<String>,
    used: HashSet<String>,
}

/// Standardordner für Presets im App-Datenordner
pub fn default_preset_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("presets")
}

impl MilkdropPool {
    pub fn new(preset_dir: PathBuf) -> Self {
        let mut p = Self {
            worker: None,
            status: Status::Off,
            instances: HashMap::new(),
            preset_dir,
            presets: vec![],
            scan: None,
            download: None,
            download_error: None,
            used: HashSet::new(),
        };
        p.rescan();
        p
    }

    /// Preset-Ordner im Hintergrund durchsuchen
    pub fn rescan(&mut self) {
        let (tx, rx) = channel();
        let dir = self.preset_dir.clone();
        std::thread::spawn(move || {
            let _ = tx.send(scan_presets(&dir));
        });
        self.scan = Some(rx);
    }

    pub fn set_preset_dir(&mut self, dir: PathBuf) {
        self.preset_dir = dir;
        self.rescan();
    }

    /// Preset-Sammlung „Cream of the Crop“ (~10 000 Presets) herunterladen und entpacken
    pub fn download_presets(&mut self) {
        let (tx, rx) = channel();
        let dir = self.preset_dir.clone();
        std::thread::spawn(move || {
            let _ = tx.send(download_presets(&dir));
        });
        self.download = Some(rx);
        self.download_error = None;
    }

    pub fn downloading(&self) -> bool {
        self.download.is_some()
    }

    pub fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    fn start_worker(&mut self) {
        let (tx, rx_cmd) = channel();
        let (tx_ev, rx) = channel();
        let tex_paths = vec![self.preset_dir.clone(), self.preset_dir.join("textures")];
        std::thread::Builder::new()
            .name("milkdrop".into())
            .spawn(move || worker(rx_cmd, tx_ev, tex_paths))
            .ok();
        self.worker = Some(Worker { tx, rx });
        self.status = Status::Starting;
    }

    pub fn begin_frame(&mut self) {
        self.used.clear();
        if let Some(rx) = &self.scan {
            if let Ok(list) = rx.try_recv() {
                self.presets = list;
                self.scan = None;
            }
        }
        if let Some(rx) = &self.download {
            if let Ok(r) = rx.try_recv() {
                self.download = None;
                match r {
                    Ok(()) => self.rescan(),
                    Err(e) => self.download_error = Some(e),
                }
            }
        }
    }

    /// Textur-Schlüssel für eine Milkdrop-Fläche (und ob schon ein Bild da ist)
    pub fn surface(&mut self, surface_key: &str, preset: &str, blend: f64, size: (f64, f64), renderer: &Renderer) -> (String, bool) {
        if self.worker.is_none() {
            self.start_worker();
        }
        // Zu viele Instanzen: weitere Flächen mit gleichem Preset teilen sich eine
        let mut key = surface_key.to_string();
        if !self.instances.contains_key(&key) && self.instances.len() >= MAX_INSTANCES {
            key = format!("preset:{preset}");
        }
        self.used.insert(key.clone());
        let tex_key = format!("md:{key}");
        let Some(worker) = &self.worker else { return (tex_key, false) };

        // Auflösung an die Fläche anpassen (spart Rechenzeit bei kleinen Flächen)
        let scale = (1280.0 / size.0.max(size.1).max(1.0)).min(1.0);
        let w = ((size.0 * scale).round() as usize).max(64);
        let h = ((size.1 * scale).round() as usize).max(64);
        let inst = self.instances.entry(key.clone()).or_insert_with(|| {
            let _ = worker.tx.send(Cmd::Ensure { key: key.clone(), w, h });
            InstState { w, h, preset: String::new() }
        });
        if (w as f64 - inst.w as f64).abs() / inst.w as f64 > 0.15 || (h as f64 - inst.h as f64).abs() / inst.h as f64 > 0.15 {
            inst.w = w;
            inst.h = h;
            let _ = worker.tx.send(Cmd::Ensure { key: key.clone(), w, h });
        }
        if inst.preset != preset && !preset.is_empty() {
            let smooth = !inst.preset.is_empty();
            inst.preset = preset.to_string();
            let _ = worker.tx.send(Cmd::Preset { key: key.clone(), path: self.preset_dir.join(preset), blend, smooth });
        }
        (tex_key.clone(), renderer.has_texture(&tex_key))
    }

    /// Nach dem Durchgang über alle Flächen: Musik weitergeben, Bild anfordern, fertige Bilder hochladen
    pub fn end_frame(&mut self, pcm: Vec<f32>, renderer: &mut Renderer) {
        let unused: Vec<String> = self.instances.keys().filter(|k| !self.used.contains(*k)).cloned().collect();
        for k in unused {
            self.instances.remove(&k);
            renderer.remove_texture(&format!("md:{k}"));
            if let Some(w) = &self.worker {
                let _ = w.tx.send(Cmd::Remove { key: k });
            }
        }
        let Some(worker) = &self.worker else { return };
        if !self.instances.is_empty() {
            let _ = worker.tx.send(Cmd::Pcm(pcm));
            let _ = worker.tx.send(Cmd::Render);
        }
        let mut latest: HashMap<String, Frame> = HashMap::new();
        while let Ok(ev) = worker.rx.try_recv() {
            match ev {
                Event::Status(s) => self.status = s,
                Event::Frame(f) => {
                    latest.insert(f.key.clone(), f);
                }
            }
        }
        for (key, f) in latest {
            if self.instances.contains_key(&key) {
                renderer.upload_rgba(&format!("md:{key}"), f.w, f.h, &f.data, false);
            }
        }
    }
}

/// Welches Preset gerade dran ist (beim automatischen Weiterschalten deterministisch aus der Uhr)
pub fn current_preset<'a>(preset: &'a str, cycle: f64, random: bool, names: &'a [String], now_ms: f64, seed: f64) -> &'a str {
    if cycle <= 0.0 || names.is_empty() {
        return preset;
    }
    let k = (now_ms / (cycle * 1000.0)).floor() as i64;
    let base = names.iter().position(|n| n == preset).unwrap_or(0);
    if !random {
        return &names[(base + k.max(0) as usize) % names.len()];
    }
    if names.len() < 2 {
        return &names[0];
    }
    // Zufällig, aber deterministisch aus Schritt und Fläche
    let mut h = ((k as u64) ^ ((seed * 1e6) as u64)).wrapping_mul(2654435761) & 0xffff_ffff;
    h = ((h ^ (h >> 15)).wrapping_mul(2246822519)) & 0xffff_ffff;
    &names[(base + 1 + (h as usize % (names.len() - 1))) % names.len()]
}

/// Anzeigename eines Presets (Dateiname ohne Endung)
pub fn display_name(rel: &str) -> &str {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    file.strip_suffix(".milk").or_else(|| file.strip_suffix(".prjm")).unwrap_or(file)
}

pub fn scan_presets(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>, depth: usize) {
        if depth > 8 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out, depth + 1);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("milk") || x.eq_ignore_ascii_case("prjm")) {
                if let Ok(rel) = p.strip_prefix(base) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let mut out = vec![];
    walk(dir, dir, &mut out, 0);
    out.sort_by_key(|a| a.to_lowercase());
    out
}

fn download_presets(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let zip = dir.join("cream-of-the-crop.zip");
    let url = "https://github.com/projectM-visualizer/presets-cream-of-the-crop/archive/refs/heads/master.zip";
    let ok = std::process::Command::new("curl")
        .args(["-L", "--fail", "--silent", "--show-error", "-o"])
        .arg(&zip)
        .arg(url)
        .status()
        .map_err(|e| format!("curl: {e}"))?
        .success();
    if !ok {
        return Err("Download fehlgeschlagen (Internetverbindung?)".into());
    }
    let ok = std::process::Command::new("unzip").args(["-q", "-o"]).arg(&zip).arg("-d").arg(dir).status().map_err(|e| format!("unzip: {e}"))?.success();
    let _ = std::fs::remove_file(&zip);
    if !ok {
        return Err("Entpacken fehlgeschlagen".into());
    }
    Ok(())
}

// --- Milkdrop-Thread

struct GlInstance<'a> {
    pm: projectm::Instance<'a>,
    fbo: glow::NativeFramebuffer,
    tex: glow::NativeTexture,
    pbo: [glow::NativeBuffer; 2],
    /// Bild im PBO wartet aufs Auslesen
    pending: [bool; 2],
    cur: usize,
    w: usize,
    h: usize,
}

fn worker(rx: Receiver<Cmd>, tx: Sender<Event>, tex_paths: Vec<PathBuf>) {
    use glow::HasContext;
    let api = match projectm::Api::load() {
        Ok(a) => a,
        Err(e) => {
            let _ = tx.send(Event::Status(Status::Error(format!("projectM nicht gefunden ({e}). Die fertige App bringt projectM mit; beim Selbstbauen: siehe README."))));
            return;
        }
    };
    if !api.supports_fbo() {
        let _ = tx.send(Event::Status(Status::Error(format!(
            "projectM in {} ist zu alt (ab Version 4.2 nötig). Die fertige App bringt eine passende Version mit.",
            api.path.display()
        ))));
        return;
    }
    let ctx = match gl::create() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(Event::Status(Status::Error(format!("OpenGL-Kontext für Milkdrop nicht verfügbar: {e}"))));
            return;
        }
    };
    let glc = unsafe { glow::Context::from_loader_function(|s| ctx.proc_address(s)) };
    let version = unsafe { glc.get_parameter_string(glow::VERSION) };
    let _ = tx.send(Event::Status(Status::Ready(format!("projectM: {} · OpenGL {version}", api.path.display()))));

    let mut instances: HashMap<String, GlInstance> = HashMap::new();
    let mut presets: HashMap<String, (PathBuf, f64, bool)> = HashMap::new();
    let mut failed_create = false;

    let make_targets = |w: usize, h: usize| unsafe {
        let tex = glc.create_texture().unwrap();
        glc.bind_texture(glow::TEXTURE_2D, Some(tex));
        glc.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w as i32, h as i32, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
        glc.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        glc.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        let fbo = glc.create_framebuffer().unwrap();
        glc.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        glc.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
        let pbo = [glc.create_buffer().unwrap(), glc.create_buffer().unwrap()];
        for b in pbo {
            glc.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(b));
            glc.buffer_data_size(glow::PIXEL_PACK_BUFFER, (w * h * 4) as i32, glow::STREAM_READ);
        }
        glc.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
        (fbo, tex, pbo)
    };
    let free_targets = |i: &GlInstance| unsafe {
        glc.delete_framebuffer(i.fbo);
        glc.delete_texture(i.tex);
        for b in i.pbo {
            glc.delete_buffer(b);
        }
    };

    while let Ok(first) = rx.recv() {
        // Alles abholen, was inzwischen anliegt; höchstens einmal zeichnen
        let mut cmds = vec![first];
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        let mut render = false;
        let mut pcm: Vec<f32> = vec![];
        for cmd in cmds {
            match cmd {
                Cmd::Ensure { key, w, h } => {
                    if let Some(inst) = instances.get_mut(&key) {
                        if inst.w != w || inst.h != h {
                            free_targets(inst);
                            let (fbo, tex, pbo) = make_targets(w, h);
                            inst.fbo = fbo;
                            inst.tex = tex;
                            inst.pbo = pbo;
                            inst.pending = [false; 2];
                            inst.w = w;
                            inst.h = h;
                            inst.pm.set_size(w, h);
                        }
                        continue;
                    }
                    let Some(pm) = projectm::Instance::new(&api, w, h, &tex_paths) else {
                        if !failed_create {
                            failed_create = true;
                            let _ = tx.send(Event::Status(Status::Error("projectM konnte nicht gestartet werden (OpenGL zu alt?)".into())));
                        }
                        continue;
                    };
                    let (fbo, tex, pbo) = make_targets(w, h);
                    if let Some((path, blend, _)) = presets.get(&key) {
                        pm.load_preset(path, false, *blend);
                    }
                    instances.insert(key, GlInstance { pm, fbo, tex, pbo, pending: [false; 2], cur: 0, w, h });
                }
                Cmd::Preset { key, path, blend, smooth } => {
                    if let Some(inst) = instances.get(&key) {
                        inst.pm.load_preset(&path, smooth, blend);
                    }
                    presets.insert(key, (path, blend, smooth));
                }
                Cmd::Remove { key } => {
                    if let Some(inst) = instances.remove(&key) {
                        free_targets(&inst);
                    }
                    presets.remove(&key);
                }
                Cmd::Pcm(mut p) => pcm.append(&mut p),
                Cmd::Render => render = true,
            }
        }
        if !render {
            continue;
        }
        for (key, inst) in instances.iter_mut() {
            inst.pm.add_pcm(&pcm);
            unsafe {
                glc.bind_framebuffer(glow::FRAMEBUFFER, Some(inst.fbo));
                glc.viewport(0, 0, inst.w as i32, inst.h as i32);
                glc.clear_color(0.0, 0.0, 0.0, 1.0);
                glc.clear(glow::COLOR_BUFFER_BIT);
                inst.pm.render(inst.fbo.0.get());
                // Asynchron auslesen: dieses Bild in PBO cur, das vorige aus dem anderen PBO holen
                glc.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(inst.fbo));
                glc.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(inst.pbo[inst.cur]));
                glc.read_pixels(0, 0, inst.w as i32, inst.h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::BufferOffset(0));
                inst.pending[inst.cur] = true;
                let other = 1 - inst.cur;
                if inst.pending[other] {
                    glc.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(inst.pbo[other]));
                    let size = inst.w * inst.h * 4;
                    let ptr = glc.map_buffer_range(glow::PIXEL_PACK_BUFFER, 0, size as i32, glow::MAP_READ_BIT);
                    if !ptr.is_null() {
                        let src = std::slice::from_raw_parts(ptr, size);
                        // OpenGL liefert die Zeilen von unten nach oben
                        let row = inst.w * 4;
                        let mut data = vec![0u8; size];
                        for y in 0..inst.h {
                            let s = (inst.h - 1 - y) * row;
                            data[y * row..(y + 1) * row].copy_from_slice(&src[s..s + row]);
                        }
                        glc.unmap_buffer(glow::PIXEL_PACK_BUFFER);
                        // Milkdrop ist deckend
                        for px in data.chunks_exact_mut(4) {
                            px[3] = 255;
                        }
                        let _ = tx.send(Event::Frame(Frame { key: key.clone(), w: inst.w as u32, h: inst.h as u32, data }));
                    }
                    inst.pending[other] = false;
                }
                glc.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
                inst.cur = other;
            }
        }
        unsafe {
            glc.bind_framebuffer(glow::FRAMEBUFFER, None);
            glc.flush();
        }
    }
    for inst in instances.values() {
        free_targets(inst);
    }
    instances.clear();
    drop(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_is_deterministic() {
        let names: Vec<String> = (0..10).map(|i| format!("p{i}.milk")).collect();
        assert_eq!(current_preset("p3.milk", 0.0, false, &names, 99999.0, 0.5), "p3.milk");
        assert_eq!(current_preset("p3.milk", 10.0, false, &names, 25_000.0, 0.5), "p5.milk");
        let a = current_preset("p3.milk", 10.0, true, &names, 25_000.0, 0.5);
        let b = current_preset("p3.milk", 10.0, true, &names, 25_000.0, 0.5);
        assert_eq!(a, b);
        assert_ne!(a, "p3.milk");
    }

    #[test]
    fn names() {
        assert_eq!(display_name("cream/Geiss - Cosmic Dust.milk"), "Geiss - Cosmic Dust");
    }

    #[test]
    fn scans_folder() {
        let dir = std::env::temp_dir().join(format!("pm-presets-{}", crate::model::new_id()));
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("a/b/X.milk"), "").unwrap();
        std::fs::write(dir.join("a/y.MILK"), "").unwrap();
        std::fs::write(dir.join("a/readme.txt"), "").unwrap();
        assert_eq!(scan_presets(&dir), vec!["a/b/X.milk", "a/y.MILK"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
