// Bearbeitungszustand (nicht im Projekt gespeichert), Rückgängig-Verlauf und alle Bearbeitungs-
// aktionen, die Editor, Tastatur und Seitenleisten gemeinsam benutzen.

use std::time::{Duration, Instant};

use crate::geometry::{
    apply, create_mesh, eval_mesh, output_to_shape, rotate_quad, scale_quad, set_corners, shape_to_output, square_to_quad,
    translate_quad,
};
use crate::model::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Surface,
    Mask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskScope {
    Surface,
    Layer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestPattern {
    None,
    Grid,
    White,
}

/// Raster, das zur Kalibrierung über Flächen gelegt wird
#[derive(Clone, Debug, PartialEq)]
pub struct GridSettings {
    pub enabled: bool,
    pub cols: u32,
    pub rows: u32,
    /// Linienstärke in Ausgabepixeln
    pub thickness: f32,
    pub color: [u8; 3],
    pub diagonals: bool,
    pub only_selected: bool,
}

impl Default for GridSettings {
    fn default() -> Self {
        Self { enabled: false, cols: 8, rows: 8, thickness: 3.0, color: [255, 255, 255], diagonals: false, only_selected: false }
    }
}

#[derive(Clone, Debug)]
pub struct ViewState {
    /// Bearbeitungsmodus: Umrisse, Griffe und Mauszeiger auf der Ausgabe
    pub edit_mode: bool,
    pub test_pattern: TestPattern,
    pub tool: Tool,
    /// Wohin neue Masken gehören: in die gewählte Fläche oder als Masken-Ebene
    pub mask_scope: MaskScope,
    pub selected_surface: Option<String>,
    pub selected_corner: Option<usize>,
    pub selected_mask: Option<String>,
    /// Maskenpunkt (Masken-Werkzeug) bzw. Mesh-Punkt (Flächen-Werkzeug)
    pub selected_point: Option<usize>,
    /// Punkte rasten an Ecken/Punkten anderer Flächen und Masken ein
    pub snap: bool,
    pub grid: GridSettings,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            edit_mode: true,
            test_pattern: TestPattern::None,
            tool: Tool::Surface,
            mask_scope: MaskScope::Surface,
            selected_surface: None,
            selected_corner: None,
            selected_mask: None,
            selected_point: None,
            snap: true,
            grid: GridSettings::default(),
        }
    }
}

/// Laufzeitzustand der Show
#[derive(Clone, Debug)]
pub struct ShowState {
    pub blackout: bool,
    /// Gesamthelligkeit 0…1
    pub master: f64,
}

impl Default for ShowState {
    fn default() -> Self {
        Self { blackout: false, master: 1.0 }
    }
}

// --- Rückgängig

const HISTORY_MAX: usize = 200;

/// Verlauf aus Projekt-Schnappschüssen. Zusammengehörige Änderungen (eine Mausbewegung,
/// wiederholte Pfeiltasten, ein Regler) tragen dieselbe Gesten-ID und ergeben einen Schritt.
#[derive(Default)]
pub struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
    gesture: Option<String>,
}

impl History {
    /// Vor einer Änderung aufrufen
    pub fn checkpoint(&mut self, before: &Project, gesture: Option<&str>) {
        if let (Some(g), Some(cur)) = (gesture, &self.gesture) {
            if g == cur {
                return;
            }
        }
        self.undo.push(before.clone());
        if self.undo.len() > HISTORY_MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.gesture = gesture.map(str::to_string);
    }

    pub fn undo(&mut self, current: &mut Project) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(current, prev));
        self.gesture = None;
        true
    }

    pub fn redo(&mut self, current: &mut Project) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(current, next));
        self.gesture = None;
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.gesture = None;
    }
}

/// Gesten-IDs für wiederholte Tasten/Mausrad/Regler: bündelt Wiederholungen zu einem Schritt
#[derive(Default)]
pub struct RepeatGesture {
    id: String,
    key: String,
    at: Option<Instant>,
}

impl RepeatGesture {
    pub fn get(&mut self, key: &str, window_ms: u64) -> String {
        let now = Instant::now();
        let fresh = self.key != key || self.at.is_none_or(|t| now.duration_since(t) > Duration::from_millis(window_ms));
        if fresh {
            self.id = new_id();
            self.key = key.to_string();
        }
        self.at = Some(now);
        self.id.clone()
    }
}

/// Projekt + Bearbeitungszustand + Verlauf
pub struct Doc {
    pub project: Project,
    pub view: ViewState,
    pub show: ShowState,
    pub history: History,
    pub repeat: RepeatGesture,
    /// Ungespeicherte Änderungen (gegenüber der Projektdatei)
    pub dirty: bool,
    /// Zähler, der bei jeder Projektänderung steigt (Autosave, Caches)
    pub revision: u64,
    pub path: Option<std::path::PathBuf>,
}

impl Doc {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            view: ViewState::default(),
            show: ShowState::default(),
            history: History::default(),
            repeat: RepeatGesture::default(),
            dirty: false,
            revision: 0,
            path: None,
        }
    }

    /// Projekt ändern (mit Rückgängig-Schritt). gesture: gleiche ID = gleicher Schritt.
    pub fn edit<R>(&mut self, gesture: Option<&str>, f: impl FnOnce(&mut Project) -> R) -> R {
        let before = self.project.clone();
        let r = f(&mut self.project);
        if self.project != before {
            self.history.checkpoint(&before, gesture);
            self.dirty = true;
            self.revision += 1;
        }
        r
    }

    pub fn repeat_gesture(&mut self, key: &str, window_ms: u64) -> String {
        self.repeat.get(key, window_ms)
    }

    pub fn undo(&mut self) {
        if self.history.undo(&mut self.project) {
            self.after_history();
        }
    }

    pub fn redo(&mut self) {
        if self.history.redo(&mut self.project) {
            self.after_history();
        }
    }

    fn after_history(&mut self) {
        self.dirty = true;
        self.revision += 1;
        self.fix_selection();
    }

    /// Auswahl bereinigen, wenn Elemente verschwunden sind
    pub fn fix_selection(&mut self) {
        let p = &self.project;
        if self.view.selected_surface.as_deref().is_some_and(|id| p.surface(Some(id)).is_none()) {
            self.view.selected_surface = None;
            self.view.selected_corner = None;
        }
        if self.view.selected_mask.as_deref().is_some_and(|id| p.find_mask(id).is_none()) {
            self.view.selected_mask = None;
            self.view.selected_point = None;
        }
    }

    pub fn replace_project(&mut self, project: Project, path: Option<std::path::PathBuf>) {
        self.project = project;
        self.view.selected_surface = None;
        self.view.selected_corner = None;
        self.view.selected_mask = None;
        self.view.selected_point = None;
        self.history.clear();
        self.dirty = false;
        self.revision += 1;
        self.path = path;
    }

    pub fn selected_surface(&self) -> Option<&Surface> {
        self.project.surface(self.view.selected_surface.as_deref())
    }

    pub fn selected_mask(&self) -> Option<MaskRef<'_>> {
        self.project.find_mask(self.view.selected_mask.as_deref()?)
    }

    // --- Auswahl

    pub fn select_surface(&mut self, id: Option<String>, corner: Option<usize>) {
        let keep_mask = self.selected_mask().is_some_and(|m| m.surface.map(|s| &s.id) == id.as_ref());
        self.view.selected_surface = id;
        self.view.selected_corner = corner;
        if !keep_mask {
            self.view.selected_mask = None;
            self.view.selected_point = None;
        }
        // Im Flächen-Werkzeug steht selected_point für einen Mesh-Punkt
        if self.view.tool == Tool::Surface {
            self.view.selected_point = None;
        }
    }

    pub fn select_mesh_point(&mut self, surface: String, index: usize) {
        self.view.selected_surface = Some(surface);
        self.view.selected_corner = None;
        self.view.selected_point = Some(index);
        self.view.selected_mask = None;
    }

    /// Maske (Flächenmaske oder Masken-Ebene) auswählen
    pub fn select_mask(&mut self, mask_id: &str, point: Option<usize>) {
        let Some(found) = self.project.find_mask(mask_id) else { return };
        let surface = found.surface.map(|s| s.id.clone());
        self.view.tool = Tool::Mask;
        self.view.selected_mask = Some(mask_id.to_string());
        self.view.selected_point = point;
        self.view.selected_surface = surface;
        self.view.selected_corner = None;
    }

    // --- Flächen

    pub fn add_surface(&mut self) {
        let n = self.project.surface_count();
        // Neue Fläche leicht versetzt, damit sie nicht exakt über der letzten liegt
        let off = (n % 5) as f64 * 0.04;
        let corners = translate_quad(&default_corners(), off, off);
        let s = create_surface(new_id(), format!("Fläche {}", n + 1), corners);
        let id = s.id.clone();
        self.edit(None, |p| p.layers.push(Layer::Surface(s)));
        self.view.tool = Tool::Surface;
        self.select_surface(Some(id), None);
    }

    pub fn duplicate_surface(&mut self, id: &str) {
        let Some(idx) = self.project.index_of(id) else { return };
        let Some(src) = self.project.surface(Some(id)).cloned() else { return };
        let mut copy = src.clone();
        copy.id = new_id();
        copy.name = format!("{} Kopie", src.name).chars().take(100).collect();
        copy.locked = false;
        let moved = clamp_quad(translate_quad(&src.corners, 0.03, 0.03));
        set_corners(&mut copy, moved);
        for m in &mut copy.masks {
            m.id = new_id();
        }
        let new_id = copy.id.clone();
        self.edit(None, |p| p.layers.insert(idx + 1, Layer::Surface(copy)));
        self.select_surface(Some(new_id), None);
    }

    pub fn remove_layer(&mut self, id: &str) {
        self.edit(None, |p| p.layers.retain(|l| l.id() != id));
        self.fix_selection();
    }

    pub fn move_layer(&mut self, id: &str, to: usize) {
        self.edit(None, |p| {
            if let Some(from) = p.index_of(id) {
                let l = p.layers.remove(from);
                p.layers.insert(to.min(p.layers.len()), l);
            }
        });
    }

    fn with_selected_surface(&mut self, gesture: Option<&str>, f: impl FnOnce(&mut Surface)) {
        let Some(id) = self.view.selected_surface.clone() else { return };
        if self.project.surface(Some(&id)).is_none_or(|s| s.locked) {
            return;
        }
        self.edit(gesture, |p| {
            if let Some(s) = p.surface_mut(&id) {
                f(s);
            }
        });
    }

    pub fn scale_selected(&mut self, factor: f64, gesture: Option<&str>) {
        self.with_selected_surface(gesture, |s| {
            let q = clamp_quad(scale_quad(&s.corners, factor));
            set_corners(s, q);
        });
    }

    pub fn rotate_selected(&mut self, radians: f64, aspect: f64, gesture: Option<&str>) {
        self.with_selected_surface(gesture, |s| {
            let q = clamp_quad(rotate_quad(&s.corners, radians, aspect));
            set_corners(s, q);
        });
    }

    pub fn reset_corners(&mut self) {
        self.with_selected_surface(None, |s| set_corners(s, default_corners()));
    }

    /// Ausgewählte Ecke/Fläche/Punkt um (dx, dy) Ausgabepixel verschieben
    pub fn nudge(&mut self, dx: f64, dy: f64, out_w: f64, out_h: f64, gesture: Option<&str>) {
        let ox = dx / out_w;
        let oy = dy / out_h;
        if self.view.tool == Tool::Mask {
            let Some(i) = self.view.selected_point else { return };
            let Some(sel) = self.selected_mask() else { return };
            if sel.layer.is_some_and(|l| l.locked) {
                return;
            }
            let pts = mask_points_out(sel.mask, sel.surface);
            let Some(p) = pts.get(i) else { return };
            let Some(np) = out_to_mask(Point::new(p.x + ox, p.y + oy), sel.surface) else { return };
            let id = sel.mask.id.clone();
            self.edit(gesture, |pr| {
                if let Some(m) = pr.mask_mut(&id) {
                    if let Some(q) = m.points.get_mut(i) {
                        *q = np;
                    }
                }
            });
            return;
        }
        let corner = self.view.selected_corner;
        let point = self.view.selected_point;
        self.with_selected_surface(gesture, |s| {
            if s.shape == Shape::Mesh {
                if let (Some(m), Some(i)) = (&mut s.mesh, point) {
                    if let Some(c) = m.points.get_mut(i) {
                        *c = clamp_point(Point::new(c.x + ox, c.y + oy));
                        s.corners = crate::geometry::mesh_corners(m);
                        return;
                    }
                }
            }
            match corner {
                Some(i) => {
                    let mut q = s.corners;
                    q[i] = clamp_point(Point::new(q[i].x + ox, q[i].y + oy));
                    set_corners(s, q);
                }
                None => {
                    let q = clamp_quad(translate_quad(&s.corners, ox, oy));
                    set_corners(s, q);
                }
            }
        });
    }

    /// Form wechseln (Ecken sinnvoll umrechnen, Mesh bildet die bisherige Abbildung nach)
    pub fn set_shape(&mut self, id: &str, shape: Shape) {
        let Some(s) = self.project.surface(Some(id)) else { return };
        if s.shape == shape || s.locked {
            return;
        }
        let mid = |a: Point, b: Point| Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
        let mut corners = s.corners;
        if shape == Shape::Triangle && s.shape != Shape::Triangle {
            corners = [mid(s.corners[0], s.corners[1]), s.corners[2], s.corners[3], s.corners[3]];
        } else if s.shape == Shape::Triangle && shape != Shape::Triangle {
            let [a, b, c, _] = s.corners;
            let half = Point::new((b.x - c.x) / 2.0, (b.y - c.y) / 2.0);
            corners = [Point::new(a.x - half.x, a.y - half.y), Point::new(a.x + half.x, a.y + half.y), b, c];
        }
        let mut tmp = s.clone();
        tmp.corners = corners;
        tmp.shape = if shape == Shape::Mesh { Shape::Quad } else { shape };
        tmp.mesh = None;
        let mesh = (shape == Shape::Mesh).then(|| {
            let src = if s.shape == Shape::Triangle { tmp.clone() } else { tmp.clone() };
            create_mesh(|u, v| shape_to_output(&src, Point::new(u, v)).unwrap_or(Point::new(u, v)), 4, 4)
        });
        self.edit(None, |p| {
            if let Some(s) = p.surface_mut(id) {
                s.corners = corners;
                s.shape = shape;
                s.mesh = mesh;
            }
        });
        self.view.selected_point = None;
    }

    pub fn resize_mesh(&mut self, id: &str, cols: usize, rows: usize) {
        let (cols, rows) = (cols.clamp(1, MESH_MAX), rows.clamp(1, MESH_MAX));
        self.edit(None, |p| {
            if let Some(s) = p.surface_mut(id) {
                if let Some(m) = &s.mesh {
                    let old = m.clone();
                    let mesh = create_mesh(|u, v| eval_mesh(&old, u, v), cols, rows);
                    s.corners = crate::geometry::mesh_corners(&mesh);
                    s.mesh = Some(mesh);
                }
            }
        });
        self.view.selected_point = None;
    }

    /// Wölbung entfernen, Ecken behalten
    pub fn flatten_mesh(&mut self, id: &str) {
        self.edit(None, |p| {
            if let Some(s) = p.surface_mut(id) {
                if let (Some(m), Some(h)) = (&s.mesh, square_to_quad(&s.corners)) {
                    s.mesh = Some(create_mesh(|u, v| apply(&h, Point::new(u, v)), m.cols, m.rows));
                }
            }
        });
    }

    // --- Masken

    /// Neue Maske anlegen. scope Surface = in den Stapel der gewählten Fläche, Layer = Masken-Ebene
    /// ganz oben (wirkt auf alles darunter). points liegen im Ausgaberaum.
    pub fn new_mask(&mut self, scope: MaskScope, points: Vec<Point>) -> Option<String> {
        let surface = if scope == MaskScope::Surface { self.selected_surface().cloned() } else { None };
        if scope == MaskScope::Surface && surface.is_none() {
            return None;
        }
        let id = new_id();
        let n_points = points.len();
        if let Some(s) = &surface {
            let pts: Vec<Point> = points.iter().filter_map(|p| out_to_mask(*p, Some(s))).collect();
            // Erste Maske einer Fläche: „hinzufügen“ (nur Inneres zeigen); weitere: „ausschneiden“
            let mode = if s.masks.is_empty() { MaskMode::Add } else { MaskMode::Subtract };
            let mask = create_mask(id.clone(), format!("Maske {}", s.masks.len() + 1), pts, mode);
            let sid = s.id.clone();
            self.edit(None, |p| {
                if let Some(s) = p.surface_mut(&sid) {
                    s.masks.push(mask);
                }
            });
        } else {
            let n = self.project.mask_layer_count() + 1;
            let layer = create_mask_layer(id.clone(), format!("Masken-Ebene {n}"), points.into_iter().map(clamp_point).collect());
            self.edit(None, |p| p.layers.push(Layer::Mask(layer)));
        }
        self.view.tool = Tool::Mask;
        self.view.selected_mask = Some(id.clone());
        self.view.selected_point = n_points.checked_sub(1);
        self.view.selected_surface = surface.map(|s| s.id);
        Some(id)
    }

    /// Form einer Fläche aus allen Ebenen darunter ausschneiden (Masken-Ebene direkt darunter)
    pub fn cutout_below(&mut self, id: &str) {
        let Some(s) = self.project.surface(Some(id)) else { return };
        let Some(idx) = self.project.index_of(id) else { return };
        let pts = crate::geometry::shape_outline(s);
        let layer = create_mask_layer(new_id(), format!("Ausschnitt „{}“", s.name).chars().take(100).collect(), pts);
        self.edit(None, |p| p.layers.insert(idx, Layer::Mask(layer)));
    }

    pub fn set_mask_points(&mut self, mask_id: &str, points: Vec<Point>, gesture: Option<&str>) {
        self.edit(gesture, |p| {
            if let Some(m) = p.mask_mut(mask_id) {
                m.points = points;
            }
        });
    }

    pub fn delete_selected_point(&mut self) {
        let Some(i) = self.view.selected_point else { return };
        let Some(sel) = self.selected_mask() else { return };
        if sel.layer.is_some_and(|l| l.locked) || i >= sel.mask.points.len() {
            return;
        }
        let id = sel.mask.id.clone();
        let mut points = sel.mask.points.clone();
        points.remove(i);
        let n = points.len();
        self.set_mask_points(&id, points, None);
        self.view.selected_point = if n > 0 { Some(i.saturating_sub(1)) } else { None };
    }

    pub fn move_mask(&mut self, mask_id: &str, to: usize) {
        self.edit(None, |p| {
            for l in &mut p.layers {
                if let Layer::Surface(s) = l {
                    if let Some(from) = s.masks.iter().position(|m| m.id == mask_id) {
                        let m = s.masks.remove(from);
                        s.masks.insert(to.min(s.masks.len()), m);
                    }
                }
            }
        });
    }

    pub fn remove_mask(&mut self, mask_id: &str) {
        self.edit(None, |p| p.remove_mask(mask_id));
        if self.view.selected_mask.as_deref() == Some(mask_id) {
            self.view.selected_mask = None;
            self.view.selected_point = None;
        }
    }

    pub fn update_mask(&mut self, mask_id: &str, gesture: Option<&str>, f: impl FnOnce(&mut Mask)) {
        self.edit(gesture, |p| {
            if let Some(m) = p.mask_mut(mask_id) {
                f(m);
            }
        });
    }

    pub fn set_mask_layer_locked(&mut self, mask_id: &str, locked: bool) {
        self.edit(None, |p| {
            for l in &mut p.layers {
                if let Layer::Mask(m) = l {
                    if m.mask.id == mask_id {
                        m.locked = locked;
                    }
                }
            }
        });
    }

    pub fn update_surface(&mut self, id: &str, gesture: Option<&str>, f: impl FnOnce(&mut Surface)) {
        self.edit(gesture, |p| {
            if let Some(s) = p.surface_mut(id) {
                f(s);
            }
        });
    }
}

/// Maskenpunkte im Ausgaberaum (Flächenmasken folgen dem Corner Pin; Masken-Ebenen liegen schon dort)
pub fn mask_points_out(mask: &Mask, surface: Option<&Surface>) -> Vec<Point> {
    match surface {
        None => mask.points.clone(),
        Some(s) => mask.points.iter().map(|p| shape_to_output(s, *p).unwrap_or(*p)).collect(),
    }
}

/// Ausgaberaum → Maskenraum
pub fn out_to_mask(p: Point, surface: Option<&Surface>) -> Option<Point> {
    match surface {
        None => Some(clamp_point(p)),
        Some(s) => output_to_shape(s, p).map(clamp_point),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_groups_gestures() {
        let mut d = Doc::new(create_project("T"));
        d.add_surface();
        let id = d.view.selected_surface.clone().unwrap();
        d.nudge(10.0, 0.0, 1000.0, 1000.0, Some("g1"));
        d.nudge(10.0, 0.0, 1000.0, 1000.0, Some("g1"));
        d.nudge(10.0, 0.0, 1000.0, 1000.0, Some("g2"));
        let x = d.project.surface(Some(&id)).unwrap().corners[0].x;
        d.undo();
        assert!((d.project.surface(Some(&id)).unwrap().corners[0].x - (x - 0.01)).abs() < 1e-9);
        d.undo();
        assert!((d.project.surface(Some(&id)).unwrap().corners[0].x - (x - 0.03)).abs() < 1e-9);
        d.undo();
        assert!(d.project.surface(Some(&id)).is_none(), "Fläche wieder weg");
        assert!(d.view.selected_surface.is_none());
        d.redo();
        assert!(d.project.surface(Some(&id)).is_some());
    }

    #[test]
    fn masks_follow_surface() {
        let mut d = Doc::new(create_project("T"));
        let sid = d.project.surfaces().next().unwrap().id.clone();
        d.select_surface(Some(sid.clone()), None);
        let pts = vec![Point::new(0.35, 0.3), Point::new(0.65, 0.3), Point::new(0.5, 0.7)];
        let mid = d.new_mask(MaskScope::Surface, pts.clone()).unwrap();
        let sel = d.selected_mask().unwrap();
        assert_eq!(sel.mask.mode, MaskMode::Add);
        let out = mask_points_out(sel.mask, sel.surface);
        assert!((out[0].x - 0.35).abs() < 1e-9 && (out[2].y - 0.7).abs() < 1e-9);
        // Fläche verschieben → Maske wandert mit
        d.view.tool = Tool::Surface;
        d.select_surface(Some(sid.clone()), None);
        d.nudge(100.0, 0.0, 1000.0, 1000.0, None);
        let found = d.project.find_mask(&mid).unwrap();
        let out = mask_points_out(found.mask, found.surface);
        assert!((out[0].x - 0.45).abs() < 1e-9);
        // zweite Maske schneidet aus
        let m2 = d.new_mask(MaskScope::Surface, pts).unwrap();
        assert_eq!(d.project.find_mask(&m2).unwrap().mask.mode, MaskMode::Subtract);
    }

    #[test]
    fn shape_switch_keeps_mapping() {
        let mut d = Doc::new(create_project("T"));
        let sid = d.project.surfaces().next().unwrap().id.clone();
        let before = d.project.surface(Some(&sid)).unwrap().corners;
        d.set_shape(&sid, Shape::Mesh);
        let s = d.project.surface(Some(&sid)).unwrap();
        assert_eq!(s.mesh.as_ref().unwrap().points.len(), 25);
        for i in 0..4 {
            assert!((s.corners[i].x - before[i].x).abs() < 1e-9);
        }
        d.set_shape(&sid, Shape::Triangle);
        assert!(d.project.surface(Some(&sid)).unwrap().mesh.is_none());
        d.set_shape(&sid, Shape::Quad);
        assert_eq!(d.project.surface(Some(&sid)).unwrap().shape, Shape::Quad);
    }

    #[test]
    fn mask_layer_and_cutout() {
        let mut d = Doc::new(create_project("T"));
        let sid = d.project.surfaces().next().unwrap().id.clone();
        d.new_mask(MaskScope::Layer, vec![Point::new(0.0, 0.0)]);
        assert_eq!(d.project.mask_layer_count(), 1);
        assert_eq!(d.view.selected_point, Some(0));
        d.cutout_below(&sid);
        assert!(matches!(d.project.layers[0], Layer::Mask(_)));
        d.delete_selected_point();
        assert!(d.view.selected_point.is_none());
    }

    #[test]
    fn no_history_without_change() {
        let mut d = Doc::new(create_project("T"));
        d.edit(None, |_| {});
        assert!(!d.history.can_undo());
        assert!(!d.dirty);
    }
}
