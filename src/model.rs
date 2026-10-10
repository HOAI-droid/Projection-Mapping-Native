// Datenmodell – kompatibel zum Projektformat 4 der Web-Version (JSON).
// Projekte lassen sich zwischen Web- und nativer Version austauschen; Szenen und Show-Einstellungen
// werden unverändert mitgeführt, auch wenn die native Version sie (noch) nicht benutzt.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::effects::{default_params, effects};

/// Punkt im normierten Raum: (0,0) oben links, (1,1) unten rechts
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Vier Ecken: oben links, oben rechts, unten rechts, unten links
pub type Quad = [Point; 4];

/// Eckpunkte dürfen etwas außerhalb des Bildes liegen (Fläche ragt aus dem Bild)
pub const POINT_MIN: f64 = -1.0;
pub const POINT_MAX: f64 = 2.0;
pub const SPEED_MIN: f64 = 0.25;
pub const SPEED_MAX: f64 = 4.0;
pub const FEATHER_MAX: f64 = 200.0;
pub const MESH_MAX: usize = 16;

pub fn clamp_point(p: Point) -> Point {
    Point::new(p.x.clamp(POINT_MIN, POINT_MAX), p.y.clamp(POINT_MIN, POINT_MAX))
}

pub fn clamp_quad(q: Quad) -> Quad {
    q.map(clamp_point)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Num(f64),
    Str(String),
}

impl ParamValue {
    pub fn num(&self) -> f64 {
        match self {
            ParamValue::Num(n) => *n,
            ParamValue::Str(s) => s.parse().unwrap_or(0.0),
        }
    }
}

pub type EffectParams = BTreeMap<String, ParamValue>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AudioSource {
    Level,
    #[default]
    Bass,
    Mid,
    High,
    Beat,
}

impl AudioSource {
    pub const ALL: [AudioSource; 5] = [Self::Level, Self::Bass, Self::Mid, Self::High, Self::Beat];
    pub fn label(self) -> &'static str {
        match self {
            Self::Level => "Pegel",
            Self::Bass => "Bass",
            Self::Mid => "Mitten",
            Self::High => "Höhen",
            Self::Beat => "Beat",
        }
    }
}

/// Kopplung eines Reglers an die Musik
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Modulation {
    pub source: AudioSource,
    /// −1 … 1: Anteil des Reglerbereichs, um den der Wert verschoben wird
    pub amount: f64,
    /// 0 … 1: Glättung (0 = sofort, 1 = sehr träge)
    pub smoothing: f64,
    /// 0 … 1: Werte unterhalb der Schwelle zählen nicht
    pub threshold: f64,
}

impl Default for Modulation {
    fn default() -> Self {
        Self { source: AudioSource::Bass, amount: 0.5, smoothing: 0.3, threshold: 0.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum Content {
    Testgrid,
    Color {
        color: String,
    },
    Image {
        media_id: String,
    },
    Video {
        media_id: String,
        #[serde(default = "yes")]
        r#loop: bool,
        #[serde(default = "one")]
        speed: f64,
        #[serde(default = "yes")]
        playing: bool,
    },
    Effect {
        effect: String,
        #[serde(default)]
        params: EffectParams,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        r#mod: Option<BTreeMap<String, Modulation>>,
    },
    Camera {
        #[serde(default)]
        device_id: String,
        #[serde(default)]
        mirror: bool,
    },
    Milkdrop {
        preset: String,
        #[serde(default = "two")]
        blend: f64,
        #[serde(default)]
        cycle: f64,
        #[serde(default)]
        random: bool,
    },
    /// leer – auch für unbekannte Inhaltstypen
    #[serde(other)]
    None,
}

fn yes() -> bool {
    true
}
fn one() -> f64 {
    1.0
}
fn two() -> f64 {
    2.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentType {
    None,
    Testgrid,
    Color,
    Image,
    Video,
    Effect,
    Milkdrop,
    Camera,
}

impl ContentType {
    pub const ALL: [ContentType; 8] = [
        Self::None,
        Self::Testgrid,
        Self::Color,
        Self::Image,
        Self::Video,
        Self::Effect,
        Self::Milkdrop,
        Self::Camera,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Leer",
            Self::Testgrid => "Testgitter",
            Self::Color => "Farbe",
            Self::Image => "Bild",
            Self::Video => "Video",
            Self::Effect => "Effekt ✨",
            Self::Milkdrop => "Milkdrop (Winamp) 🌈",
            Self::Camera => "Live-Kamera 📷",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            Self::None => "leer",
            Self::Testgrid => "Gitter",
            Self::Color => "Farbe",
            Self::Image => "Bild",
            Self::Video => "Video",
            Self::Effect => "Effekt",
            Self::Milkdrop => "Milkdrop",
            Self::Camera => "Kamera",
        }
    }
}

impl Content {
    pub fn kind(&self) -> ContentType {
        match self {
            Content::None => ContentType::None,
            Content::Testgrid => ContentType::Testgrid,
            Content::Color { .. } => ContentType::Color,
            Content::Image { .. } => ContentType::Image,
            Content::Video { .. } => ContentType::Video,
            Content::Effect { .. } => ContentType::Effect,
            Content::Camera { .. } => ContentType::Camera,
            Content::Milkdrop { .. } => ContentType::Milkdrop,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "lowercase")]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Multiply,
    Screen,
}

impl BlendMode {
    pub const ALL: [BlendMode; 4] = [Self::Normal, Self::Add, Self::Screen, Self::Multiply];
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Add => "Additiv",
            Self::Screen => "Bildschirm",
            Self::Multiply => "Multiplizieren",
        }
    }
}

/// strecken / füllen (beschneiden) / einpassen (Ränder bleiben frei)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FitMode {
    #[default]
    Stretch,
    Fill,
    Fit,
}

impl FitMode {
    pub const ALL: [FitMode; 3] = [Self::Stretch, Self::Fill, Self::Fit];
    pub fn label(self) -> &'static str {
        match self {
            Self::Stretch => "Strecken",
            Self::Fill => "Füllen",
            Self::Fit => "Einpassen",
        }
    }
}

/// add = Inneres hinzufügen · subtract = Inneres ausschneiden · intersect = nur die Überschneidung
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MaskMode {
    #[default]
    Add,
    Subtract,
    Intersect,
}

impl MaskMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Add => "＋ Hinzufügen",
            Self::Subtract => "− Ausschneiden",
            Self::Intersect => "∩ Schnittmenge",
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Self::Add => "Macht das Innere sichtbar. Steht sie ganz oben, ist nur ihr Inneres zu sehen.",
            Self::Subtract => "Schneidet das Innere aus dem bisher Sichtbaren heraus (z. B. ein Fenster).",
            Self::Intersect => "Lässt nur sichtbar, was auch innerhalb dieser Maske liegt.",
        }
    }
}

/// Freiform-Maske. Bei Flächenmasken liegen die Punkte im Inhaltsraum der Fläche (0…1, bewegt sich
/// also mit dem Corner Pin), bei Masken-Ebenen im Ausgaberaum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mask {
    pub id: String,
    #[serde(default = "mask_name")]
    pub name: String,
    #[serde(default)]
    pub points: Vec<Point>,
    #[serde(default)]
    pub mode: MaskMode,
    /// weiche Kante in Ausgabepixeln
    #[serde(default)]
    pub feather: f64,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn mask_name() -> String {
    "Maske".into()
}

/// Masken-Ebene im Ebenenstapel: wirkt auf alle Ebenen DARUNTER
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskLayer {
    #[serde(flatten)]
    pub mask: Mask,
    #[serde(default)]
    pub locked: bool,
}

/// Form einer Fläche. Dreieck nutzt die Ecken 0–2 (0 = Spitze).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    #[default]
    Quad,
    Circle,
    Triangle,
    Mesh,
}

impl Shape {
    pub const ALL: [Shape; 4] = [Self::Quad, Self::Circle, Self::Triangle, Self::Mesh];
    pub fn label(self) -> &'static str {
        match self {
            Self::Quad => "▭ Viereck",
            Self::Circle => "◯ Kreis",
            Self::Triangle => "△ Dreieck",
            Self::Mesh => "▦ Gitter",
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Self::Quad => "Vier Ecken, perspektivisch (Corner Pin)",
            Self::Circle => "Kreis/Ellipse innerhalb der vier Ecken",
            Self::Triangle => "Drei Ecken",
            Self::Mesh => "Verzerrungsgitter für gewölbte Objekte (Säulen, Kugeln, Fahrzeuge …)",
        }
    }
    pub fn handle_count(self) -> usize {
        if self == Self::Triangle { 3 } else { 4 }
    }
}

/// Verzerrungsgitter: (cols+1)×(rows+1) Kontrollpunkte im Ausgaberaum, zeilenweise von oben links
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub cols: usize,
    pub rows: usize,
    pub points: Vec<Point>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Surface {
    pub id: String,
    #[serde(default = "surface_name")]
    pub name: String,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub shape: Shape,
    pub corners: Quad,
    #[serde(default)]
    pub mesh: Option<Mesh>,
    #[serde(default = "no_content")]
    pub content: Content,
    #[serde(default = "one")]
    pub opacity: f64,
    #[serde(default)]
    pub blend: BlendMode,
    #[serde(default)]
    pub fit: FitMode,
    #[serde(default)]
    pub masks: Vec<Mask>,
    #[serde(default)]
    pub opacity_mod: Option<Modulation>,
}

fn surface_name() -> String {
    "Fläche".into()
}
fn no_content() -> Content {
    Content::None
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Layer {
    Surface(Surface),
    Mask(MaskLayer),
}

impl Layer {
    pub fn id(&self) -> &str {
        match self {
            Layer::Surface(s) => &s.id,
            Layer::Mask(m) => &m.mask.id,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Layer::Surface(s) => &s.name,
            Layer::Mask(m) => &m.mask.name,
        }
    }
    pub fn locked(&self) -> bool {
        match self {
            Layer::Surface(s) => s.locked,
            Layer::Mask(m) => m.locked,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub format: u32,
    pub name: String,
    /// Ebenenstapel (Flächen und Masken-Ebenen): Index 0 liegt ganz unten
    pub layers: Vec<Layer>,
    /// Szenen der Web-Version – werden unverändert gespeichert
    #[serde(default = "empty_array")]
    pub scenes: serde_json::Value,
    #[serde(default = "empty_object")]
    pub show: serde_json::Value,
}

fn empty_array() -> serde_json::Value {
    serde_json::Value::Array(vec![])
}
fn empty_object() -> serde_json::Value {
    serde_json::json!({ "fadeDuration": 1.5, "fadeKind": "cross", "loop": true, "advance": "time", "beatsPerScene": 16 })
}

// --- Anlegen

pub fn new_id() -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    seed ^= COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, std::sync::atomic::Ordering::Relaxed);
    (0..10)
        .map(|_| {
            // xorshift
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            ALPHABET[(seed % ALPHABET.len() as u64) as usize] as char
        })
        .collect()
}

pub fn default_corners() -> Quad {
    [Point::new(0.3, 0.25), Point::new(0.7, 0.25), Point::new(0.7, 0.75), Point::new(0.3, 0.75)]
}

pub fn create_surface(id: String, name: String, corners: Quad) -> Surface {
    Surface {
        id,
        name,
        visible: true,
        locked: false,
        shape: Shape::Quad,
        corners,
        mesh: None,
        content: Content::Testgrid,
        opacity: 1.0,
        blend: BlendMode::Normal,
        fit: FitMode::Stretch,
        masks: vec![],
        opacity_mod: None,
    }
}

pub fn create_mask(id: String, name: String, points: Vec<Point>, mode: MaskMode) -> Mask {
    Mask { id, name, points, mode, feather: 0.0, enabled: true }
}

pub fn create_mask_layer(id: String, name: String, points: Vec<Point>) -> MaskLayer {
    MaskLayer { mask: create_mask(id, name, points, MaskMode::Subtract), locked: false }
}

pub fn create_project(name: &str) -> Project {
    Project {
        format: 4,
        name: name.into(),
        layers: vec![Layer::Surface(create_surface(new_id(), "Fläche 1".into(), default_corners()))],
        scenes: empty_array(),
        show: empty_object(),
    }
}

// --- Zugriff

impl Project {
    pub fn surfaces(&self) -> impl DoubleEndedIterator<Item = &Surface> {
        self.layers.iter().filter_map(|l| if let Layer::Surface(s) = l { Some(s) } else { None })
    }

    pub fn surface(&self, id: Option<&str>) -> Option<&Surface> {
        let id = id?;
        self.surfaces().find(|s| s.id == id)
    }

    pub fn surface_mut(&mut self, id: &str) -> Option<&mut Surface> {
        self.layers.iter_mut().find_map(|l| match l {
            Layer::Surface(s) if s.id == id => Some(s),
            _ => None,
        })
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.layers.iter().position(|l| l.id() == id)
    }

    /// Findet eine Maske: Masken-Ebene oder Flächenmaske (mit ID der Fläche)
    pub fn find_mask(&self, mask_id: &str) -> Option<MaskRef<'_>> {
        for l in &self.layers {
            match l {
                Layer::Mask(m) if m.mask.id == mask_id => return Some(MaskRef { mask: &m.mask, surface: None, layer: Some(m) }),
                Layer::Surface(s) => {
                    if let Some(m) = s.masks.iter().find(|m| m.id == mask_id) {
                        return Some(MaskRef { mask: m, surface: Some(s), layer: None });
                    }
                }
                _ => {}
            }
        }
        None
    }

    pub fn mask_mut(&mut self, mask_id: &str) -> Option<&mut Mask> {
        for l in &mut self.layers {
            match l {
                Layer::Mask(m) if m.mask.id == mask_id => return Some(&mut m.mask),
                Layer::Surface(s) => {
                    if let Some(m) = s.masks.iter_mut().find(|m| m.id == mask_id) {
                        return Some(m);
                    }
                }
                _ => {}
            }
        }
        None
    }

    pub fn remove_mask(&mut self, mask_id: &str) {
        self.layers.retain(|l| !matches!(l, Layer::Mask(m) if m.mask.id == mask_id));
        for l in &mut self.layers {
            if let Layer::Surface(s) = l {
                s.masks.retain(|m| m.id != mask_id);
            }
        }
    }

    pub fn mask_layer_count(&self) -> usize {
        self.layers.iter().filter(|l| matches!(l, Layer::Mask(_))).count()
    }

    pub fn surface_count(&self) -> usize {
        self.surfaces().count()
    }
}

#[derive(Clone, Copy)]
pub struct MaskRef<'a> {
    pub mask: &'a Mask,
    pub surface: Option<&'a Surface>,
    pub layer: Option<&'a MaskLayer>,
}

// --- Laden (mit Migration älterer Formate und Reparatur fehlerhafter Werte)

/// Liest eine Projektdatei (auch Format 1–3 der Web-Version) und repariert fehlende/ungültige Felder.
pub fn parse_project(text: &str) -> anyhow::Result<Project> {
    let mut v: serde_json::Value = serde_json::from_str(text)?;
    let obj = v.as_object_mut().ok_or_else(|| anyhow::anyhow!("Keine Projektdatei"))?;
    // Format 1/2: surfaces + globale masks → Ebenenstapel
    if !obj.contains_key("layers") {
        let mut layers = Vec::new();
        if let Some(serde_json::Value::Array(surfaces)) = obj.remove("surfaces") {
            for mut s in surfaces {
                if let Some(o) = s.as_object_mut() {
                    o.insert("kind".into(), "surface".into());
                }
                layers.push(s);
            }
        } else {
            anyhow::bail!("Keine Projektdatei (weder „layers“ noch „surfaces“)");
        }
        if let Some(serde_json::Value::Array(masks)) = obj.remove("masks") {
            for mut m in masks {
                if let Some(o) = m.as_object_mut() {
                    o.insert("kind".into(), "mask".into());
                }
                layers.push(m);
            }
        }
        obj.insert("layers".into(), serde_json::Value::Array(layers));
    }
    obj.insert("format".into(), 4.into());
    if !obj.get("name").is_some_and(|n| n.is_string()) {
        obj.insert("name".into(), "Unbenannt".into());
    }
    // Ebenen einzeln lesen: eine kaputte Ebene soll nicht das ganze Projekt verwerfen
    let raw_layers = obj.remove("layers").and_then(|l| if let serde_json::Value::Array(a) = l { Some(a) } else { None });
    obj.insert("layers".into(), serde_json::Value::Array(vec![]));
    let mut project: Project = serde_json::from_value(v)?;
    for mut l in raw_layers.unwrap_or_default() {
        migrate_masks(&mut l);
        match serde_json::from_value::<Layer>(l) {
            Ok(layer) => project.layers.push(layer),
            Err(e) => log::warn!("Ebene übersprungen: {e}"),
        }
    }
    sanitize(&mut project);
    Ok(project)
}

/// Ältere Masken kennen statt mode nur invert
fn migrate_masks(l: &mut serde_json::Value) {
    fn fix(m: &mut serde_json::Value, layer: bool) {
        if let Some(o) = m.as_object_mut() {
            if !o.contains_key("mode") {
                let inv = o.get("invert").and_then(|v| v.as_bool()).unwrap_or(false);
                o.insert("mode".into(), if inv || layer { "subtract" } else { "add" }.into());
            }
        }
    }
    let is_mask = l.get("kind").and_then(|k| k.as_str()) == Some("mask");
    if is_mask {
        fix(l, true);
    } else if let Some(serde_json::Value::Array(ms)) = l.get_mut("masks") {
        for m in ms {
            fix(m, false);
        }
    }
    // Inhalt nicht lesbar → leer statt die ganze Fläche zu verwerfen
    if let Some(c) = l.get("content").cloned() {
        if serde_json::from_value::<Content>(c).is_err() {
            l["content"] = serde_json::json!({ "type": "none" });
        }
    }
}

fn finite_or(v: f64, d: f64) -> f64 {
    if v.is_finite() { v } else { d }
}

/// Werte in gültige Bereiche bringen (auch nach dem Laden fremder Dateien)
pub fn sanitize(p: &mut Project) {
    let mut seen = std::collections::HashSet::new();
    p.layers.retain(|l| seen.insert(l.id().to_string()));
    for l in &mut p.layers {
        match l {
            Layer::Surface(s) => {
                s.corners = clamp_quad(s.corners.map(|c| Point::new(finite_or(c.x, 0.5), finite_or(c.y, 0.5))));
                s.opacity = finite_or(s.opacity, 1.0).clamp(0.0, 1.0);
                if s.shape == Shape::Mesh {
                    let ok = s.mesh.as_ref().is_some_and(|m| {
                        (1..=MESH_MAX).contains(&m.cols) && (1..=MESH_MAX).contains(&m.rows) && m.points.len() == (m.cols + 1) * (m.rows + 1)
                    });
                    if ok {
                        let m = s.mesh.as_mut().unwrap();
                        for q in &mut m.points {
                            *q = clamp_point(*q);
                        }
                        s.corners = crate::geometry::mesh_corners(m);
                    } else {
                        s.shape = Shape::Quad;
                        s.mesh = None;
                    }
                } else {
                    s.mesh = None;
                }
                for m in &mut s.masks {
                    sanitize_mask(m);
                }
                sanitize_content(&mut s.content);
            }
            Layer::Mask(m) => {
                sanitize_mask(&mut m.mask);
                if m.mask.mode == MaskMode::Add {
                    m.mask.mode = MaskMode::Intersect;
                }
            }
        }
    }
}

fn sanitize_mask(m: &mut Mask) {
    m.feather = finite_or(m.feather, 0.0).clamp(0.0, FEATHER_MAX);
    for q in &mut m.points {
        *q = clamp_point(*q);
    }
}

pub fn sanitize_content(c: &mut Content) {
    match c {
        Content::Video { speed, .. } => *speed = finite_or(*speed, 1.0).clamp(SPEED_MIN, SPEED_MAX),
        Content::Effect { effect, params, .. } => {
            let Some(def) = effects().get(effect) else {
                *c = Content::None;
                return;
            };
            let mut full = default_params(def);
            for p in &def.params {
                if let Some(v) = params.get(p.id()) {
                    let v = match (p, v) {
                        (crate::effects::EffectParam::Float { min, max, .. }, ParamValue::Num(n)) => ParamValue::Num(n.clamp(*min, *max)),
                        (crate::effects::EffectParam::Select { options, .. }, ParamValue::Num(n)) => {
                            ParamValue::Num(n.round().clamp(0.0, options.len().saturating_sub(1) as f64))
                        }
                        (crate::effects::EffectParam::Color { .. }, ParamValue::Str(s)) if s.len() == 7 && s.starts_with('#') => {
                            ParamValue::Str(s.to_lowercase())
                        }
                        _ => continue,
                    };
                    full.insert(p.id().to_string(), v);
                }
            }
            *params = full;
        }
        Content::Milkdrop { blend, cycle, .. } => {
            *blend = finite_or(*blend, 2.0).clamp(0.0, 20.0);
            *cycle = finite_or(*cycle, 0.0).clamp(0.0, 600.0);
        }
        _ => {}
    }
}

pub fn to_json(p: &Project) -> String {
    serde_json::to_string_pretty(p).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let p = create_project("Test");
        let text = to_json(&p);
        let q = parse_project(&text).unwrap();
        assert_eq!(p, q);
        assert!(text.contains("\"kind\": \"surface\""));
        assert!(text.contains("\"type\": \"testgrid\""));
    }

    #[test]
    fn reads_web_format() {
        let text = r##"{
          "format": 4, "name": "Web",
          "layers": [
            {"kind":"surface","id":"a1","name":"Wand","visible":true,"locked":false,"shape":"quad",
             "corners":[{"x":0.1,"y":0.1},{"x":0.9,"y":0.1},{"x":0.9,"y":0.9},{"x":0.1,"y":0.9}],
             "mesh":null,"content":{"type":"effect","effect":"strobe","params":{"speed":99},"mod":{"speed":{"source":"bass","amount":0.5,"smoothing":0.3,"threshold":0}}},
             "opacity":0.5,"blend":"add","fit":"fill","masks":[{"id":"m1","name":"M","points":[{"x":0,"y":0}],"mode":"subtract","feather":3,"enabled":true}],"opacityMod":null},
            {"kind":"surface","id":"v1","name":"Video","corners":[{"x":0,"y":0},{"x":1,"y":0},{"x":1,"y":1},{"x":0,"y":1}],
             "content":{"type":"video","mediaId":"xyz","loop":false,"speed":2,"playing":true}},
            {"kind":"surface","id":"u1","name":"Unbekannt","corners":[{"x":0,"y":0},{"x":1,"y":0},{"x":1,"y":1},{"x":0,"y":1}],
             "content":{"type":"hologram"}},
            {"kind":"mask","id":"l1","name":"Ebene","points":[],"mode":"subtract","feather":0,"enabled":true,"locked":true}
          ],
          "scenes": [{"id":"s1","name":"A","looks":{},"duration":10}],
          "show": {"fadeDuration": 2}
        }"##;
        let p = parse_project(text).unwrap();
        assert_eq!(p.layers.len(), 4);
        let Layer::Surface(s) = &p.layers[0] else { panic!() };
        assert_eq!(s.blend, BlendMode::Add);
        assert_eq!(s.masks[0].mode, MaskMode::Subtract);
        let Content::Effect { params, r#mod, .. } = &s.content else { panic!() };
        assert!(params.len() > 1, "fehlende Parameter ergänzt");
        assert!(r#mod.is_some());
        let Layer::Surface(v) = &p.layers[1] else { panic!() };
        assert_eq!(v.content, Content::Video { media_id: "xyz".into(), r#loop: false, speed: 2.0, playing: true });
        let Layer::Surface(u) = &p.layers[2] else { panic!() };
        assert_eq!(u.content, Content::None);
        let Layer::Mask(m) = &p.layers[3] else { panic!() };
        assert!(m.locked);
        // Szenen bleiben erhalten
        assert!(to_json(&p).contains("\"looks\""));
    }

    #[test]
    fn migrates_format_1() {
        let text = r#"{"name":"Alt","surfaces":[{"id":"s","corners":[{"x":0,"y":0},{"x":1,"y":0},{"x":1,"y":1},{"x":0,"y":1}],
          "masks":[{"id":"k","points":[],"invert":true}]}],
          "masks":[{"id":"g","points":[]}]}"#;
        let p = parse_project(text).unwrap();
        assert_eq!(p.layers.len(), 2);
        let Layer::Surface(s) = &p.layers[0] else { panic!() };
        assert_eq!(s.masks[0].mode, MaskMode::Subtract);
        assert!(matches!(&p.layers[1], Layer::Mask(m) if m.mask.mode == MaskMode::Subtract));
    }

    #[test]
    fn ids_unique() {
        let a = new_id();
        let b = new_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 10);
    }
}
