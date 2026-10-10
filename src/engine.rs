// Engine: hält Projekt, Renderer und alle Inhaltsquellen zusammen.
//
// Pro Bild (tick): Musik analysieren, neue Video-/Kamera-/Milkdrop-Bilder übernehmen, Effekte einen
// Schritt weiterrechnen, Masken bei Änderung neu rastern und daraus die Zeichenliste bauen. Die
// Steuerung (Live-Vorschau) und die Beamer-Ausgabe zeichnen beide dieselbe Liste – jede in ihrer Größe.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::audio::AudioEngine;
use crate::effects::{EffectParam, EffectUniforms, effects, write_params, hex_to_rgb};
use crate::geometry::{quad_pixel_size, tessellate_mesh};
use crate::media::camera::CameraStream;
use crate::media::video::{Playback, VideoPlayer};
use crate::media::{ImageLoader, MediaKind, MediaLibrary};
use crate::milkdrop::{MilkdropPool, current_preset, default_preset_dir};
use crate::model::*;
use crate::render::{masks, DrawItem, GridDraw, Renderer, Source, SurfaceDraw, ViewTarget};
use crate::state::Doc;

/// Auflösung der Flächenmasken (im Inhaltsraum der Fläche)
const SURFACE_MASK_SIZE: u32 = 1024;

/// Einstellungen, die zwischen zwei Starts erhalten bleiben
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub output_open: bool,
    pub output_fullscreen: bool,
    pub output_pos: Option<[f32; 2]>,
    pub output_size: Option<[f32; 2]>,
    pub preset_dir: Option<PathBuf>,
    pub audio_input: Option<String>,
    pub project_path: Option<PathBuf>,
    pub dirty: bool,
    pub first_run_done: bool,
}

pub struct Engine {
    pub doc: Doc,
    pub renderer: Renderer,
    pub media: MediaLibrary,
    pub audio: AudioEngine,
    pub milkdrop: MilkdropPool,
    pub settings: Settings,
    pub data_dir: PathBuf,
    images: ImageLoader,
    videos: HashMap<String, VideoPlayer>,
    cameras: HashMap<String, CameraStream>,
    pub camera_list: Vec<String>,
    camera_scan: Option<std::sync::mpsc::Receiver<Vec<String>>>,
    mask_keys: HashMap<String, u64>,
    mesh_cache: HashMap<String, (Mesh, Arc<Vec<[f32; 4]>>)>,
    mod_state: HashMap<String, f32>,
    pub draw_list: Vec<DrawItem>,
    /// Pixelgröße der echten Ausgabe (Seitenverhältnisse, weiche Kanten, Rasterlinien)
    pub ref_size: (u32, u32),
    /// Fehler je Fläche (Video/Kamera), für die Anzeige
    pub content_errors: HashMap<String, String>,
    pub stage: Option<ViewTarget>,
    pub output: Option<ViewTarget>,
    start: Instant,
    last_tick: Option<Instant>,
    frames: u32,
    fps_since: Instant,
    pub fps: f32,
    saved_revision: u64,
    last_autosave: Instant,
}

impl Engine {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, data_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&data_dir);
        let settings: Settings = std::fs::read_to_string(data_dir.join("settings.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        // Letzten Stand wiederherstellen
        let project = std::fs::read_to_string(data_dir.join("autosave.json"))
            .ok()
            .and_then(|t| parse_project(&t).ok())
            .unwrap_or_else(|| create_project("Neues Projekt"));
        let mut doc = Doc::new(project);
        doc.path = settings.project_path.clone().filter(|p| p.exists());
        doc.dirty = settings.dirty;
        let preset_dir = settings.preset_dir.clone().unwrap_or_else(|| default_preset_dir(&data_dir));
        let mut audio = AudioEngine::new();
        audio.refresh_inputs();
        Self {
            doc,
            renderer: Renderer::new(device, queue),
            media: MediaLibrary::new(data_dir.join("media")),
            audio,
            milkdrop: MilkdropPool::new(preset_dir),
            settings,
            data_dir,
            images: ImageLoader::default(),
            videos: HashMap::new(),
            cameras: HashMap::new(),
            camera_list: vec![],
            camera_scan: None,
            mask_keys: HashMap::new(),
            mesh_cache: HashMap::new(),
            mod_state: HashMap::new(),
            draw_list: vec![],
            ref_size: (1920, 1080),
            content_errors: HashMap::new(),
            stage: None,
            output: None,
            start: Instant::now(),
            last_tick: None,
            frames: 0,
            fps_since: Instant::now(),
            fps: 0.0,
            saved_revision: 0,
            last_autosave: Instant::now(),
        }
    }

    pub fn time(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    pub fn refresh_cameras(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::media::camera::list_cameras());
        });
        self.camera_scan = Some(rx);
    }

    pub fn camera_scanning(&self) -> bool {
        self.camera_scan.is_some()
    }

    /// Einmal pro Bild (von welchem Fenster auch immer zuerst); doppelte Aufrufe im selben Bild werden ignoriert
    pub fn tick(&mut self) {
        let now = Instant::now();
        if self.last_tick.is_some_and(|t| now.duration_since(t) < Duration::from_millis(7)) {
            return;
        }
        self.last_tick = Some(now);
        self.frames += 1;
        if self.fps_since.elapsed() >= Duration::from_secs(1) {
            self.fps = self.frames as f32 / self.fps_since.elapsed().as_secs_f32();
            self.frames = 0;
            self.fps_since = Instant::now();
        }

        self.media.poll();
        if let Some(rx) = &self.camera_scan {
            if let Ok(list) = rx.try_recv() {
                self.camera_list = list;
                self.camera_scan = None;
            }
        }
        self.audio.update();
        let audio_bytes = self.audio.frame.texture_bytes();
        self.renderer.upload_r8("audio", 128, 3, &audio_bytes);
        self.milkdrop.begin_frame();
        for (id, w, h, data) in self.images.poll() {
            self.renderer.upload_rgba(&format!("img:{id}"), w, h, &data, true);
        }

        self.build_draw_list();

        let pcm = self.audio.take_pcm();
        self.milkdrop.end_frame(pcm, &mut self.renderer);
        self.autosave(false);
    }

    fn build_draw_list(&mut self) {
        let (ref_w, ref_h) = (self.ref_size.0 as f64, self.ref_size.1 as f64);
        let time = self.time();
        let now_ms = time * 1000.0;
        let project = self.doc.project.clone();
        let view = self.doc.view.clone();
        let grid = &view.grid;
        let grid_color = [grid.color[0] as f32 / 255.0, grid.color[1] as f32 / 255.0, grid.color[2] as f32 / 255.0];

        let mut items = Vec::new();
        let mut used_images = HashSet::new();
        let mut used_videos: HashMap<String, Playback> = HashMap::new();
        let mut used_cameras = HashSet::new();
        let mut used_masks = HashSet::new();
        let mut used_effects = HashSet::new();
        let mut used_meshes = HashSet::new();
        self.content_errors.clear();

        for layer in &project.layers {
            let s = match layer {
                Layer::Mask(m) => {
                    if !m.mask.enabled || m.mask.points.len() < 3 {
                        continue;
                    }
                    // Region der Masken-Ebene als Alpha (Modus wirkt erst beim Verrechnen)
                    let mut region = m.mask.clone();
                    region.mode = MaskMode::Add;
                    let key = format!("mlayer:{}", m.mask.id);
                    let (w, h) = (self.ref_size.0.clamp(16, 4096), self.ref_size.1.clamp(16, 4096));
                    self.ensure_mask(&key, &[&region], w, h, 1.0, |p| ((p.x * w as f64) as f32, (p.y * h as f64) as f32));
                    used_masks.insert(key.clone());
                    items.push(DrawItem::Mask { texture: key, subtract: m.mask.mode == MaskMode::Subtract });
                    continue;
                }
                Layer::Surface(s) => s,
            };

            let (pw, ph) = quad_pixel_size(&s.corners, ref_w, ref_h);
            let aspect = pw / ph.max(1.0);
            let seed = seed_of(&s.id);

            // Dreiecksnetz des Verzerrungsgitters (nur bei Änderung neu)
            let mesh = match (&s.mesh, s.shape) {
                (Some(m), Shape::Mesh) => {
                    used_meshes.insert(s.id.clone());
                    let fresh = !matches!(self.mesh_cache.get(&s.id), Some((old, _)) if old == m);
                    if fresh {
                        self.mesh_cache.insert(s.id.clone(), (m.clone(), Arc::new(tessellate_mesh(m, 8))));
                    }
                    Some(self.mesh_cache[&s.id].1.clone())
                }
                _ => None,
            };

            // Flächenmasken im Inhaltsraum
            let active: Vec<&Mask> = s.masks.iter().filter(|m| m.enabled && m.points.len() >= 3).collect();
            let mask = if active.is_empty() {
                None
            } else {
                let key = format!("msurf:{}", s.id);
                let size = SURFACE_MASK_SIZE;
                let feather_scale = size as f64 / ((pw + ph) / 2.0).max(1.0);
                self.ensure_mask(&key, &active, size, size, feather_scale, |p| ((p.x * size as f64) as f32, (p.y * size as f64) as f32));
                used_masks.insert(key.clone());
                Some(key)
            };

            let show_grid = grid.enabled && s.visible && (!grid.only_selected || view.selected_surface.as_deref() == Some(&s.id));
            let grid_draw = show_grid.then(|| GridDraw {
                cols: grid.cols as f32,
                rows: grid.rows as f32,
                thickness: grid.thickness,
                color: grid_color,
                diagonals: grid.diagonals,
            });
            let base = SurfaceDraw {
                key: s.id.clone(),
                shape: s.shape,
                corners: s.corners,
                mesh,
                source: Source::None,
                opacity: 1.0,
                blend: s.blend,
                fit_scale: [1.0, 1.0],
                mask,
                grid: grid_draw.clone(),
            };

            // Deckkraft (mit Musik-Kopplung)
            let mut opacity = s.opacity as f32;
            if let Some(m) = &s.opacity_mod {
                let f = self.mod_value(&format!("{}:opacity", s.id), m);
                let a = m.amount as f32;
                opacity *= if a >= 0.0 { 1.0 - a + a * f } else { 1.0 + a * f };
            }
            let opacity = opacity.clamp(0.0, 1.0);

            let mut source = None;
            if s.visible && opacity > 0.001 {
                source = match &s.content {
                    Content::None => None,
                    Content::Testgrid => Some((Source::Texture("testgrid".into()), [1.0, 1.0])),
                    Content::Color { color } => Some((Source::Color(hex_to_rgb(color)), [1.0, 1.0])),
                    Content::Effect { effect, params, r#mod } => {
                        let key = format!("fx:{}", s.id);
                        used_effects.insert(key.clone());
                        match effects().get(effect) {
                            Some(def) => {
                                let mut u = EffectUniforms {
                                    audio: [self.audio.frame.level, self.audio.frame.bass, self.audio.frame.mid, self.audio.frame.high],
                                    a: [time as f32, aspect as f32, seed, self.audio.frame.beat],
                                    b: [self.audio.frame.beat_count as f32, self.audio.frame.bpm, 0.0, 0.0],
                                    params: [[0.0; 4]; crate::effects::MAX_PARAMS],
                                };
                                let modulated = self.modulate(&s.id, def, params, r#mod.as_ref());
                                write_params(def, &modulated, &mut u.params);
                                // Puffergröße nach der Fläche (lange Seite begrenzt), Rückkopplung kleiner
                                let long = if def.feedback { 768.0 } else { pw.max(ph).clamp(64.0, 1600.0) };
                                let (w, h) = if aspect >= 1.0 { (long, long / aspect) } else { (long * aspect, long) };
                                if self.renderer.render_effect(&key, def, &u, w.round() as u32, h.round() as u32) {
                                    Some((Source::Texture(key), [1.0, 1.0]))
                                } else {
                                    None
                                }
                            }
                            None => None,
                        }
                    }
                    Content::Image { media_id } => {
                        let key = format!("img:{media_id}");
                        used_images.insert(key.clone());
                        match self.media.get(media_id) {
                            Some(item) if item.kind == MediaKind::Image => {
                                if !self.renderer.has_texture(&key) {
                                    self.images.request(media_id, self.media.path(item));
                                }
                                self.renderer.texture(&key).map(|t| {
                                    let ca = t.width as f64 / t.height.max(1) as f64;
                                    (Source::Texture(key.clone()), fit_scale(s.fit, ca, aspect))
                                })
                            }
                            _ => {
                                self.content_errors.insert(s.id.clone(), "Bild fehlt in der Medienbibliothek".into());
                                None
                            }
                        }
                    }
                    Content::Video { media_id, r#loop, speed, playing } => {
                        used_videos.insert(media_id.clone(), Playback { playing: *playing, looping: *r#loop, speed: *speed });
                        let key = format!("vid:{media_id}");
                        if !self.videos.contains_key(media_id) {
                            if let Some(item) = self.media.get(media_id).filter(|m| m.kind == MediaKind::Video) {
                                let p = VideoPlayer::new(self.media.path(item), item.width, item.height, item.fps, Playback {
                                    playing: *playing,
                                    looping: *r#loop,
                                    speed: *speed,
                                });
                                self.videos.insert(media_id.clone(), p);
                            } else {
                                self.content_errors.insert(s.id.clone(), "Video fehlt in der Medienbibliothek".into());
                            }
                        }
                        if let Some(p) = self.videos.get_mut(media_id) {
                            if let Some(frame) = p.take_frame() {
                                self.renderer.upload_rgba(&key, p.width, p.height, &frame, false);
                            }
                            if let Some(e) = p.error() {
                                self.content_errors.insert(s.id.clone(), e);
                            }
                        }
                        self.renderer.texture(&key).map(|t| {
                            let ca = t.width as f64 / t.height.max(1) as f64;
                            (Source::Texture(key.clone()), fit_scale(s.fit, ca, aspect))
                        })
                    }
                    Content::Camera { device_id, mirror } => {
                        used_cameras.insert(device_id.clone());
                        let key = format!("cam:{device_id}");
                        let cam = self.cameras.entry(device_id.clone()).or_insert_with(|| CameraStream::open(device_id));
                        if let Some((w, h, frame)) = cam.take_frame() {
                            self.renderer.upload_rgba(&key, w, h, &frame, false);
                        }
                        if let Some(e) = cam.error() {
                            self.content_errors.insert(s.id.clone(), e);
                        }
                        self.renderer.texture(&key).map(|t| {
                            let ca = t.width as f64 / t.height.max(1) as f64;
                            let f = fit_scale(s.fit, ca, aspect);
                            // Spiegeln = Textur horizontal umdrehen
                            (Source::Texture(key.clone()), [if *mirror { -f[0] } else { f[0] }, f[1]])
                        })
                    }
                    Content::Milkdrop { preset, blend, cycle, random } => {
                        let names = &self.milkdrop.presets;
                        let want = if names.is_empty() {
                            preset.clone()
                        } else if names.iter().any(|n| n == preset) || *cycle > 0.0 {
                            current_preset(preset, *cycle, *random, names, now_ms, seed as f64).to_string()
                        } else {
                            // Preset aus einem anderen Ordner (z. B. Web-Version): erstes vorhandenes nehmen
                            names[(seed as f64 * names.len() as f64) as usize % names.len()].clone()
                        };
                        // Auflösung: bis 720 Zeilen
                        let hh = 720.0f64.min(ph.max(240.0));
                        let (key, ready) = self.milkdrop.surface(&s.id, &want, *blend, (hh * aspect, hh), &self.renderer);
                        ready.then(|| (Source::Texture(key), [1.0, 1.0]))
                    }
                };
            }

            match source {
                Some((src, fit)) => items.push(DrawItem::Surface(SurfaceDraw { source: src, opacity, fit_scale: fit, ..base })),
                None if grid_draw.is_some() => items.push(DrawItem::Surface(SurfaceDraw { source: Source::None, opacity: 1.0, ..base })),
                None => {}
            }
        }

        // Nicht mehr gebrauchte Quellen freigeben
        self.videos.retain(|id, _| used_videos.contains_key(id));
        for (id, pb) in &used_videos {
            if let Some(p) = self.videos.get(id) {
                p.set_playback(*pb);
            }
        }
        self.cameras.retain(|id, _| used_cameras.contains(id));
        let used_video_keys: HashSet<String> = used_videos.keys().map(|k| format!("vid:{k}")).collect();
        let used_camera_keys: HashSet<String> = used_cameras.iter().map(|k| format!("cam:{k}")).collect();
        self.renderer.retain_textures("img:", &used_images);
        self.renderer.retain_textures("vid:", &used_video_keys);
        self.renderer.retain_textures("cam:", &used_camera_keys);
        self.renderer.retain_textures("mlayer:", &used_masks);
        self.renderer.retain_textures("msurf:", &used_masks);
        self.mask_keys.retain(|k, _| used_masks.contains(k));
        self.renderer.retain_effects(&used_effects);
        self.renderer.retain_meshes(&used_meshes);
        self.mesh_cache.retain(|k, _| used_meshes.contains(k));
        self.draw_list = items;
    }

    fn ensure_mask(&mut self, key: &str, masks: &[&Mask], w: u32, h: u32, feather_scale: f64, to_px: impl Fn(Point) -> (f32, f32)) {
        let ck = masks::cache_key(masks, w, h, feather_scale);
        if self.mask_keys.get(key) == Some(&ck) && self.renderer.has_texture(key) {
            return;
        }
        let data = masks::rasterize(masks, w, h, feather_scale, to_px);
        self.renderer.upload_r8(key, w, h, &data);
        self.mask_keys.insert(key.to_string(), ck);
    }

    /// Audio-Wert 0…1 mit Schwelle und Glättung
    fn mod_value(&mut self, key: &str, m: &Modulation) -> f32 {
        let raw = self.audio.frame.get(m.source);
        let th = m.threshold as f32;
        let v = if th > 0.0 { ((raw - th) / (1.0 - th)).max(0.0) } else { raw };
        let prev = *self.mod_state.get(key).unwrap_or(&v);
        let sm = m.smoothing as f32;
        let s = prev * sm + v * (1.0 - sm);
        self.mod_state.insert(key.to_string(), s);
        s
    }

    /// Effekt-Regler mit der Musik verschieben
    fn modulate(&mut self, surface_id: &str, def: &crate::effects::EffectDef, params: &EffectParams, mods: Option<&std::collections::BTreeMap<String, Modulation>>) -> EffectParams {
        let Some(mods) = mods else { return params.clone() };
        let mut out = params.clone();
        for p in &def.params {
            let EffectParam::Float { id, min, max, default, .. } = p else { continue };
            let Some(m) = mods.get(id) else { continue };
            let f = self.mod_value(&format!("{surface_id}:{id}"), m) as f64;
            let base = params.get(id).map(|v| v.num()).unwrap_or(*default);
            out.insert(id.clone(), ParamValue::Num((base + m.amount * (max - min) * f).clamp(*min, *max)));
        }
        out
    }

    // --- Ansichten

    fn ensure_target(renderer: &Renderer, slot: &mut Option<ViewTarget>, w: u32, h: u32) -> bool {
        let (w, h) = (w.max(1), h.max(1));
        if slot.as_ref().is_some_and(|t| t.width == w && t.height == h) {
            return false;
        }
        let old_id = slot.as_ref().and_then(|t| t.egui_id);
        let mut t = renderer.create_view_target(w, h);
        t.egui_id = old_id;
        *slot = Some(t);
        true
    }

    /// Zeichnet die Live-Vorschau; true = Zielbild neu angelegt (egui-Textur aktualisieren)
    pub fn render_stage(&mut self, w: u32, h: u32) -> bool {
        let fresh = Self::ensure_target(&self.renderer, &mut self.stage, w, h);
        self.render_into(true);
        fresh
    }

    pub fn render_output(&mut self, w: u32, h: u32) -> bool {
        let fresh = Self::ensure_target(&self.renderer, &mut self.output, w, h);
        self.render_into(false);
        fresh
    }

    fn render_into(&mut self, stage: bool) {
        let master = if self.doc.show.blackout { 0.0 } else { self.doc.show.master as f32 };
        let white = self.doc.view.test_pattern == crate::state::TestPattern::White;
        let ref_w = self.ref_size.0 as f32;
        let target = if stage { self.stage.as_mut() } else { self.output.as_mut() };
        if let Some(t) = target {
            self.renderer.render_view(t, &self.draw_list, ref_w, master, white);
        }
    }

    // --- Speichern

    pub fn save_settings(&mut self) {
        self.settings.project_path = self.doc.path.clone();
        self.settings.dirty = self.doc.dirty;
        self.settings.preset_dir = Some(self.milkdrop.preset_dir.clone());
        if let Ok(t) = serde_json::to_string_pretty(&self.settings) {
            let _ = std::fs::write(self.data_dir.join("settings.json"), t);
        }
    }

    /// Aktuellen Stand sichern (höchstens alle 2 s, force = sofort)
    pub fn autosave(&mut self, force: bool) {
        if self.doc.revision == self.saved_revision {
            return;
        }
        if !force && self.last_autosave.elapsed() < Duration::from_secs(2) {
            return;
        }
        let text = to_json(&self.doc.project);
        let tmp = self.data_dir.join("autosave.json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, self.data_dir.join("autosave.json"));
        }
        self.saved_revision = self.doc.revision;
        self.last_autosave = Instant::now();
        self.save_settings();
    }

    pub fn save_to(&mut self, path: PathBuf) -> anyhow::Result<()> {
        std::fs::write(&path, to_json(&self.doc.project))?;
        self.doc.path = Some(path);
        self.doc.dirty = false;
        self.save_settings();
        Ok(())
    }

    pub fn open(&mut self, path: PathBuf) -> anyhow::Result<()> {
        let text = std::fs::read_to_string(&path)?;
        let project = parse_project(&text)?;
        self.doc.replace_project(project, Some(path));
        self.autosave(true);
        Ok(())
    }

    pub fn new_project(&mut self) {
        self.doc.replace_project(create_project("Neues Projekt"), None);
        self.autosave(true);
    }
}

/// Skalierung der Texturkoordinaten für die Einpassung: t = (uv − 0.5) · scale + 0.5.
/// Werte > 1 lassen Ränder frei, < 1 schneiden ab.
pub fn fit_scale(fit: FitMode, content_aspect: f64, surface_aspect: f64) -> [f32; 2] {
    if fit == FitMode::Stretch || !(content_aspect > 0.0) || !(surface_aspect > 0.0) {
        return [1.0, 1.0];
    }
    let wider = content_aspect > surface_aspect;
    let r = if fit == FitMode::Fit {
        if wider { [1.0, content_aspect / surface_aspect] } else { [surface_aspect / content_aspect, 1.0] }
    } else if wider {
        [surface_aspect / content_aspect, 1.0]
    } else {
        [1.0, content_aspect / surface_aspect]
    };
    [r[0] as f32, r[1] as f32]
}

/// Stabiler Zufallswert 0…1 je Fläche, damit gleiche Effekte nicht synchron laufen
pub fn seed_of(id: &str) -> f32 {
    let mut h: u32 = 2166136261;
    for c in id.encode_utf16() {
        h = (h ^ c as u32).wrapping_mul(16777619);
    }
    (h % 10000) as f32 / 10000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_modes() {
        assert_eq!(fit_scale(FitMode::Stretch, 2.0, 1.0), [1.0, 1.0]);
        // 2:1-Bild auf quadratischer Fläche: einpassen → oben/unten frei, füllen → links/rechts abgeschnitten
        assert_eq!(fit_scale(FitMode::Fit, 2.0, 1.0), [1.0, 2.0]);
        assert_eq!(fit_scale(FitMode::Fill, 2.0, 1.0), [0.5, 1.0]);
    }

    #[test]
    fn seeds_differ() {
        assert_ne!(seed_of("a"), seed_of("b"));
        assert!((0.0..1.0).contains(&seed_of("xyz")));
    }
}
