// Medienbibliothek: importierte Bilder und Videos liegen im App-Datenordner
// (~/Library/Application Support/Projection Mapping/media). Videos und Kameras laufen über ffmpeg
// (auf dem Mac mit Hardware-Dekodierung über VideoToolbox), Bilder über die image-Bibliothek
// (HEIC von iPhones wandelt macOS mit „sips“ um).

pub mod camera;
pub mod video;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};

use serde::{Deserialize, Serialize};

use crate::model::new_id;

/// Sucht ein Werkzeug (ffmpeg/ffprobe) auch dort, wo Homebrew es ablegt –
/// aus dem Finder gestartete Apps kennen den PATH des Terminals nicht.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    if let Ok(p) = std::env::var(format!("PM_{}", name.to_uppercase())) {
        return Some(PathBuf::from(p));
    }
    let mut candidates: Vec<PathBuf> = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/opt/local/bin"]
        .iter()
        .map(|d| Path::new(d).join(name))
        .collect();
    if let Ok(exe) = std::env::current_exe() {
        // mitgeliefert im App-Bundle (Contents/MacOS oder Contents/Resources)
        if let Some(dir) = exe.parent() {
            candidates.insert(0, dir.join(name));
            candidates.insert(1, dir.join("../Resources").join(name));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join(name)));
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub fn ffmpeg() -> Option<PathBuf> {
    static P: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    P.get_or_init(|| find_tool("ffmpeg")).clone()
}

pub fn ffprobe() -> Option<PathBuf> {
    static P: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    P.get_or_init(|| find_tool("ffprobe")).clone()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub id: String,
    /// Originaler Dateiname
    pub name: String,
    pub kind: MediaKind,
    /// Dateiname im Medienordner
    pub file: String,
    pub width: u32,
    pub height: u32,
    /// Sekunden (nur Video)
    #[serde(default)]
    pub duration: f64,
    /// Bilder pro Sekunde (nur Video)
    #[serde(default)]
    pub fps: f64,
}

pub struct ImportJob {
    pub name: String,
    pub error: Option<String>,
}

enum ImportResult {
    Done(MediaItem),
    Failed(String, String),
}

pub struct MediaLibrary {
    pub dir: PathBuf,
    pub items: Vec<MediaItem>,
    /// laufende Importe
    pub jobs: Vec<ImportJob>,
    tx: Sender<ImportResult>,
    rx: Receiver<ImportResult>,
    /// Vorschaubilder (RGBA, max. 160 px), werden vom UI in egui-Texturen übernommen
    pub thumbs: std::collections::HashMap<String, (u32, u32, Vec<u8>)>,
    thumb_tx: Sender<(String, Option<(u32, u32, Vec<u8>)>)>,
    thumb_rx: Receiver<(String, Option<(u32, u32, Vec<u8>)>)>,
    thumb_pending: std::collections::HashSet<String>,
    pub revision: u64,
}

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif"];
const VIDEO_EXT: &[&str] = &["mp4", "mov", "m4v", "webm", "mkv", "avi", "mpg", "mpeg", "hevc", "gif"];

pub fn kind_of(path: &Path) -> Option<MediaKind> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if ext == "gif" {
        // animierte GIFs als Video, sonst Bild
        return Some(MediaKind::Video);
    }
    if IMAGE_EXT.contains(&ext.as_str()) {
        Some(MediaKind::Image)
    } else if VIDEO_EXT.contains(&ext.as_str()) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

impl MediaLibrary {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let items = std::fs::read_to_string(dir.join("media.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Vec<MediaItem>>(&t).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|m| dir.join(&m.file).is_file())
            .collect();
        let (tx, rx) = channel();
        let (thumb_tx, thumb_rx) = channel();
        Self {
            dir,
            items,
            jobs: vec![],
            tx,
            rx,
            thumbs: Default::default(),
            thumb_tx,
            thumb_rx,
            thumb_pending: Default::default(),
            revision: 0,
        }
    }

    pub fn path(&self, item: &MediaItem) -> PathBuf {
        self.dir.join(&item.file)
    }

    pub fn get(&self, id: &str) -> Option<&MediaItem> {
        self.items.iter().find(|m| m.id == id)
    }

    fn save_index(&self) {
        if let Ok(t) = serde_json::to_string_pretty(&self.items) {
            let _ = std::fs::write(self.dir.join("media.json"), t);
        }
    }

    /// Dateien im Hintergrund importieren (kopieren, prüfen, Vorschau)
    pub fn import(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            let Some(kind) = kind_of(&path) else { continue };
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            self.jobs.push(ImportJob { name: name.clone(), error: None });
            let dir = self.dir.clone();
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let r = import_file(&dir, &path, kind).map_err(|e| format!("{e:#}"));
                let _ = tx.send(match r {
                    Ok(item) => ImportResult::Done(item),
                    Err(e) => ImportResult::Failed(name, e),
                });
            });
        }
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(i) = self.items.iter().position(|m| m.id == id) {
            let item = self.items.remove(i);
            let _ = std::fs::remove_file(self.dir.join(&item.file));
            self.thumbs.remove(id);
            self.save_index();
            self.revision += 1;
        }
    }

    /// Fertige Importe und Vorschaubilder übernehmen (einmal pro Bild aufrufen)
    pub fn poll(&mut self) {
        while let Ok(r) = self.rx.try_recv() {
            match r {
                ImportResult::Done(item) => {
                    if let Some(i) = self.jobs.iter().position(|j| j.name == item.name && j.error.is_none()) {
                        self.jobs.remove(i);
                    }
                    self.items.push(item);
                    self.save_index();
                    self.revision += 1;
                }
                ImportResult::Failed(name, err) => {
                    log::warn!("Import von {name} fehlgeschlagen: {err}");
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.name == name && j.error.is_none()) {
                        j.error = Some(err);
                    }
                }
            }
        }
        while let Ok((id, thumb)) = self.thumb_rx.try_recv() {
            self.thumb_pending.remove(&id);
            if let Some(t) = thumb {
                self.thumbs.insert(id, t);
            }
        }
        // Vorschaubilder für alle Medien nachladen
        for item in &self.items {
            if self.thumbs.contains_key(&item.id) || self.thumb_pending.contains(&item.id) {
                continue;
            }
            self.thumb_pending.insert(item.id.clone());
            let path = self.dir.join(&item.file);
            let kind = item.kind;
            let id = item.id.clone();
            let tx = self.thumb_tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send((id, make_thumb(&path, kind, 160).ok()));
            });
        }
    }

    pub fn clear_errors(&mut self) {
        self.jobs.retain(|j| j.error.is_none());
    }
}

fn import_file(dir: &Path, src: &Path, kind: MediaKind) -> anyhow::Result<MediaItem> {
    let id = new_id();
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut file = format!("{id}.{ext}");
    let dst = dir.join(&file);
    if kind == MediaKind::Image && (ext == "heic" || ext == "heif") {
        // HEIC (iPhone-Fotos) mit macOS-Bordmitteln nach PNG
        file = format!("{id}.png");
        let out = dir.join(&file);
        let status = Command::new("sips").args(["-s", "format", "png"]).arg(src).arg("--out").arg(&out).stdout(Stdio::null()).status();
        anyhow::ensure!(status.is_ok_and(|s| s.success()), "HEIC konnte nicht umgewandelt werden (sips)");
    } else {
        std::fs::copy(src, &dst)?;
    }
    let path = dir.join(&file);
    let result = (|| -> anyhow::Result<MediaItem> {
        match kind {
            MediaKind::Image => {
                let (w, h) = image::image_dimensions(&path)?;
                Ok(MediaItem { id: id.clone(), name: name.clone(), kind, file: file.clone(), width: w, height: h, duration: 0.0, fps: 0.0 })
            }
            MediaKind::Video => {
                let info = probe_video(&path)?;
                Ok(MediaItem {
                    id: id.clone(),
                    name: name.clone(),
                    kind,
                    file: file.clone(),
                    width: info.width,
                    height: info.height,
                    duration: info.duration,
                    fps: info.fps,
                })
            }
        }
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result
}

pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub fps: f64,
}

pub fn probe_video(path: &Path) -> anyhow::Result<VideoInfo> {
    let probe = ffprobe().ok_or_else(|| anyhow::anyhow!("ffprobe fehlt – im Terminal: brew install ffmpeg"))?;
    let out = Command::new(probe)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,avg_frame_rate,r_frame_rate:format=duration", "-of", "json"])
        .arg(path)
        .output()?;
    anyhow::ensure!(out.status.success(), "Keine lesbare Videodatei");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let s = &v["streams"][0];
    let width = s["width"].as_u64().unwrap_or(0) as u32;
    let height = s["height"].as_u64().unwrap_or(0) as u32;
    anyhow::ensure!(width > 0 && height > 0, "Kein Videobild gefunden");
    let rate = |k: &str| -> f64 {
        let t = s[k].as_str().unwrap_or("0/0");
        let mut it = t.split('/');
        let a: f64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(0.0);
        let b: f64 = it.next().and_then(|x| x.parse().ok()).unwrap_or(1.0);
        if b > 0.0 { a / b } else { 0.0 }
    };
    let mut fps = rate("avg_frame_rate");
    if !(1.0..=240.0).contains(&fps) {
        fps = rate("r_frame_rate");
    }
    if !(1.0..=240.0).contains(&fps) {
        fps = 30.0;
    }
    let duration = v["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    Ok(VideoInfo { width, height, duration, fps })
}

/// Bild laden: RGBA, vormultipliziert, lange Seite höchstens max_side
pub fn load_image(path: &Path, max_side: u32) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let img = image::open(path)?;
    let img = if img.width().max(img.height()) > max_side { img.resize(max_side, max_side, image::imageops::FilterType::Triangle) } else { img };
    let mut rgba = img.to_rgba8();
    for px in rgba.pixels_mut() {
        let a = px[3] as u16;
        if a < 255 {
            for c in 0..3 {
                px[c] = ((px[c] as u16 * a + 127) / 255) as u8;
            }
        }
    }
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

fn make_thumb(path: &Path, kind: MediaKind, size: u32) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    match kind {
        MediaKind::Image => {
            let img = image::open(path)?.thumbnail(size, size).to_rgba8();
            Ok((img.width(), img.height(), img.into_raw()))
        }
        MediaKind::Video => {
            let ff = ffmpeg().ok_or_else(|| anyhow::anyhow!("ffmpeg fehlt"))?;
            let out = Command::new(ff)
                .args(["-v", "error", "-ss", "0.5", "-i"])
                .arg(path)
                .args(["-frames:v", "1", "-vf", &format!("scale={size}:-2"), "-f", "image2pipe", "-vcodec", "png", "pipe:1"])
                .stdin(Stdio::null())
                .output()?;
            let img = image::load_from_memory(&out.stdout)?.to_rgba8();
            Ok((img.width(), img.height(), img.into_raw()))
        }
    }
}

/// Bilder im Hintergrund laden und als Textur bereitstellen
pub struct ImageLoader {
    tx: Sender<(String, anyhow::Result<(u32, u32, Vec<u8>)>)>,
    rx: Receiver<(String, anyhow::Result<(u32, u32, Vec<u8>)>)>,
    pending: std::collections::HashSet<String>,
    pub failed: std::collections::HashSet<String>,
}

impl Default for ImageLoader {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, pending: Default::default(), failed: Default::default() }
    }
}

impl ImageLoader {
    pub fn request(&mut self, id: &str, path: PathBuf) {
        if self.pending.contains(id) || self.failed.contains(id) {
            return;
        }
        self.pending.insert(id.to_string());
        let tx = self.tx.clone();
        let id = id.to_string();
        std::thread::spawn(move || {
            let _ = tx.send((id, load_image(&path, 4096)));
        });
    }

    /// Fertig geladene Bilder (ID, Breite, Höhe, Pixel)
    pub fn poll(&mut self) -> Vec<(String, u32, u32, Vec<u8>)> {
        let mut out = vec![];
        while let Ok((id, r)) = self.rx.try_recv() {
            self.pending.remove(&id);
            match r {
                Ok((w, h, d)) => out.push((id, w, h, d)),
                Err(e) => {
                    log::warn!("Bild {id}: {e}");
                    self.failed.insert(id);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(kind_of(Path::new("a.JPG")), Some(MediaKind::Image));
        assert_eq!(kind_of(Path::new("a.mov")), Some(MediaKind::Video));
        assert_eq!(kind_of(Path::new("a.txt")), None);
    }

    #[test]
    fn premultiplies() {
        let dir = std::env::temp_dir().join(format!("pm-test-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 100, 50, 128])).save(&p).unwrap();
        let (w, h, d) = load_image(&p, 4096).unwrap();
        assert_eq!((w, h), (4, 4));
        assert_eq!(d[0], 100);
        assert_eq!(d[3], 128);
        std::fs::remove_dir_all(dir).ok();
    }
}
