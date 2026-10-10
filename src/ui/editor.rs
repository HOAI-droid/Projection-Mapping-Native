// Interaktive Bearbeitungsebene über dem gerenderten Bild – in der Steuerung (über der Vorschau) und
// direkt auf der Beamer-Ausgabe.
//
// Werkzeug „Fläche“:  Ecke ziehen · Fläche innen ziehen = verschieben · Trackpad-Pinch = skalieren ·
//                     ⌥ + Scrollen = drehen · beim Verzerrungsgitter die Gitterpunkte ziehen
// Werkzeug „Maske“:   ziehen = Freihand-Umriss · Klick = Punkt setzen · Punkt ziehen ·
//                     Klick auf Kante = Punkt einfügen · Entf = löschen
// Snap (Taste S): Punkte rasten ein; ⌥ beim Ziehen = ohne Snap.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Shape as EShape, Stroke, Ui, vec2};

use crate::engine::Engine;
use crate::geometry::{centroid, dist_to_segment, eval_mesh, point_in_polygon, set_corners, shape_outline, simplify_path, translate_quad};
use crate::model::{clamp_point, clamp_quad, new_id, Layer, Mask, MaskMode, Point, Quad, Shape, Surface};
use crate::render::testgrid::CORNER_COLORS;
use crate::snap::{snap_delta, snap_point, snap_targets, SnapTargets};
use crate::state::{mask_points_out, out_to_mask, MaskScope, Tool};

const HIT_CORNER: f32 = 16.0;
const HIT_POINT: f32 = 12.0;
const HIT_EDGE: f32 = 8.0;

pub const ACCENT: Color32 = Color32::from_rgb(0x22, 0xd3, 0xee);

pub fn mode_color(mode: MaskMode) -> Color32 {
    match mode {
        MaskMode::Add => Color32::from_rgb(0x4a, 0xde, 0x80),
        MaskMode::Subtract => Color32::from_rgb(0xf8, 0x71, 0x71),
        MaskMode::Intersect => Color32::from_rgb(0xfa, 0xcc, 0x15),
    }
}

pub fn corner_color(i: usize) -> Color32 {
    let [r, g, b] = CORNER_COLORS[i % 4];
    Color32::from_rgb(r, g, b)
}

enum Drag {
    Corner { surface: String, index: usize, origin: Point, start: Point, gesture: String, snap: SnapTargets },
    Move { surface: String, origin: Quad, start: Point, gesture: String, moved: bool, snap: SnapTargets },
    MeshPoint { surface: String, index: usize, origin: Point, start: Point, gesture: String, snap: SnapTargets },
    MaskPoint { mask: String, surface: Option<Surface>, index: usize, points: Vec<Point>, origin_out: Point, start: Point, gesture: String, snap: SnapTargets },
    /// Neue Maske: Freihand-Linie oder (ohne Bewegung) erster Punkt
    Draw { path: Vec<Point>, moved: bool },
}

#[derive(Default)]
pub struct EditorState {
    drag: Option<Drag>,
    snap_mark: Option<Point>,
}

struct Frame {
    rect: Rect,
    w: f64,
    h: f64,
}

impl Frame {
    fn norm(&self, p: Pos2) -> Point {
        Point::new((p.x - self.rect.min.x) as f64 / self.w, (p.y - self.rect.min.y) as f64 / self.h)
    }
    fn screen(&self, p: Point) -> Pos2 {
        Pos2::new(self.rect.min.x + (p.x * self.w) as f32, self.rect.min.y + (p.y * self.h) as f32)
    }
    fn px_dist(&self, a: Point, b: Point) -> f32 {
        (((a.x - b.x) * self.w).hypot((a.y - b.y) * self.h)) as f32
    }
}

struct MaskEntry {
    mask: Mask,
    surface: Option<Surface>,
    locked: bool,
    is_layer: bool,
}

/// Im Masken-Werkzeug bearbeitbar: alle Masken-Ebenen + die Masken der gewählten Fläche
fn mask_list(eng: &Engine) -> Vec<MaskEntry> {
    let p = &eng.doc.project;
    let mut v: Vec<MaskEntry> = p
        .layers
        .iter()
        .filter_map(|l| if let Layer::Mask(m) = l { Some(MaskEntry { mask: m.mask.clone(), surface: None, locked: m.locked, is_layer: true }) } else { None })
        .collect();
    if let Some(s) = eng.doc.selected_surface() {
        for m in &s.masks {
            v.push(MaskEntry { mask: m.clone(), surface: Some(s.clone()), locked: false, is_layer: false });
        }
    }
    v
}

/// Zeigt die Bearbeitungsebene in rect und verarbeitet Maus/Trackpad
pub fn show(ui: &mut Ui, rect: Rect, eng: &mut Engine, st: &mut EditorState, on_output: bool) {
    let f = Frame { rect, w: rect.width().max(1.0) as f64, h: rect.height().max(1.0) as f64 };
    let id = ui.id().with(("editor", on_output));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let (pressed, down, released, pos, alt, zoom, scroll) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            i.pointer.interact_pos(),
            i.modifiers.alt,
            i.zoom_delta(),
            i.smooth_scroll_delta,
        )
    });

    if pressed && st.drag.is_none() {
        if let Some(p) = pos.filter(|p| rect.contains(*p) && resp.hovered()) {
            on_press(eng, st, &f, f.norm(p), alt);
        }
    }
    if st.drag.is_some() {
        if let Some(p) = pos {
            if down {
                on_move(eng, st, &f, f.norm(p), alt);
            }
        }
        if released || !down {
            on_end(eng, st, &f);
        }
    }

    // Pinch = skalieren, ⌥ + Scrollen = drehen (gewählte Fläche)
    if resp.hovered() && eng.doc.view.tool == Tool::Surface && eng.doc.view.selected_surface.is_some() {
        if (zoom - 1.0).abs() > 1e-4 {
            let g = eng.doc.repeat_gesture("wheel-scale", 400);
            eng.doc.scale_selected(zoom as f64, Some(&g));
        }
        if alt && scroll.y.abs() > 0.0 {
            let g = eng.doc.repeat_gesture("wheel-rotate", 400);
            let aspect = eng.ref_size.0 as f64 / eng.ref_size.1.max(1) as f64;
            eng.doc.rotate_selected(-scroll.y as f64 * 0.002, aspect, Some(&g));
        }
    }

    draw(ui, &f, eng, st, on_output);
}

fn snapped(eng: &Engine, f: &Frame, p: Point, exclude_mask: Option<&str>, alt: bool) -> Point {
    if eng.doc.view.snap && !alt { snap_point(p, &snap_targets(&eng.doc.project, None, exclude_mask), f.w, f.h).0 } else { p }
}

fn on_press(eng: &mut Engine, st: &mut EditorState, f: &Frame, p: Point, alt: bool) {
    let view = eng.doc.view.clone();
    let project = eng.doc.project.clone();
    let visible: Vec<&Surface> = project.surfaces().filter(|s| s.visible).collect();

    if view.tool == Tool::Surface {
        // 0. Gitterpunkte des gewählten Meshs
        if let Some(s) = visible.iter().find(|s| Some(&s.id) == view.selected_surface.as_ref() && s.shape == Shape::Mesh && !s.locked) {
            if let Some(m) = &s.mesh {
                if let Some(i) = m.points.iter().position(|q| f.px_dist(*q, p) <= HIT_POINT) {
                    eng.doc.select_mesh_point(s.id.clone(), i);
                    st.drag = Some(Drag::MeshPoint {
                        surface: s.id.clone(),
                        index: i,
                        origin: m.points[i],
                        start: p,
                        gesture: new_id(),
                        snap: snap_targets(&project, Some(&s.id), None),
                    });
                    return;
                }
            }
        }
        // 1. Ecken (gewählte Fläche zuerst, dann oberste zuerst); Mesh-Ecken nur über Gitterpunkte
        let mut order: Vec<&&Surface> = visible.iter().rev().collect();
        order.sort_by_key(|s| Some(&s.id) != view.selected_surface.as_ref());
        for s in order {
            if s.locked || s.shape == Shape::Mesh {
                continue;
            }
            for i in 0..s.shape.handle_count() {
                if f.px_dist(s.corners[i], p) <= HIT_CORNER {
                    eng.doc.select_surface(Some(s.id.clone()), Some(i));
                    st.drag = Some(Drag::Corner {
                        surface: s.id.clone(),
                        index: i,
                        origin: s.corners[i],
                        start: p,
                        gesture: new_id(),
                        snap: snap_targets(&project, Some(&s.id), None),
                    });
                    return;
                }
            }
        }
        // 2. Innenfläche → verschieben (oberste zuerst)
        for s in visible.iter().rev() {
            if point_in_polygon(p, &shape_outline(s)) {
                eng.doc.select_surface(Some(s.id.clone()), None);
                if !s.locked {
                    st.drag = Some(Drag::Move {
                        surface: s.id.clone(),
                        origin: s.corners,
                        start: p,
                        gesture: new_id(),
                        moved: false,
                        snap: snap_targets(&project, Some(&s.id), None),
                    });
                }
                return;
            }
        }
        // 3. Leerer Bereich → Auswahl aufheben
        eng.doc.select_surface(None, None);
        return;
    }

    // --- Masken-Werkzeug
    let masks = mask_list(eng);
    let sel = view.selected_mask.as_ref().and_then(|id| masks.iter().find(|m| &m.mask.id == id));
    if let Some(sel) = sel.filter(|m| !m.locked) {
        let pts = mask_points_out(&sel.mask, sel.surface.as_ref());
        // 1. Punkt der gewählten Maske
        if let Some(i) = pts.iter().position(|q| f.px_dist(*q, p) <= HIT_POINT) {
            eng.doc.view.selected_point = Some(i);
            st.drag = Some(Drag::MaskPoint {
                mask: sel.mask.id.clone(),
                surface: sel.surface.clone(),
                index: i,
                points: sel.mask.points.clone(),
                origin_out: pts[i],
                start: p,
                gesture: new_id(),
                snap: snap_targets(&project, None, Some(&sel.mask.id)),
            });
            return;
        }
        // 2. Kante der gewählten Maske → Punkt einfügen
        if pts.len() >= 2 {
            let n = pts.len();
            let edges = if n >= 3 { n } else { n - 1 };
            for k in 0..edges {
                let a = pts[k];
                let b = pts[(k + 1) % n];
                let (d, _) = dist_to_segment(Point::new(p.x * f.w, p.y * f.h), Point::new(a.x * f.w, a.y * f.h), Point::new(b.x * f.w, b.y * f.h));
                if d <= HIT_EDGE as f64 {
                    let Some(mp) = out_to_mask(p, sel.surface.as_ref()) else { return };
                    let mut points = sel.mask.points.clone();
                    points.insert(k + 1, mp);
                    let g = new_id();
                    eng.doc.set_mask_points(&sel.mask.id, points.clone(), Some(&g));
                    eng.doc.view.selected_point = Some(k + 1);
                    st.drag = Some(Drag::MaskPoint {
                        mask: sel.mask.id.clone(),
                        surface: sel.surface.clone(),
                        index: k + 1,
                        points,
                        origin_out: p,
                        start: p,
                        gesture: g,
                        snap: snap_targets(&project, None, Some(&sel.mask.id)),
                    });
                    return;
                }
            }
        }
    }
    // 3. Punkt einer anderen sichtbaren Maske → diese auswählen und ziehen
    for m in &masks {
        if Some(&m.mask.id) == view.selected_mask.as_ref() {
            continue;
        }
        let pts = mask_points_out(&m.mask, m.surface.as_ref());
        if let Some(i) = pts.iter().position(|q| f.px_dist(*q, p) <= HIT_POINT) {
            eng.doc.select_mask(&m.mask.id, Some(i));
            if !m.locked {
                st.drag = Some(Drag::MaskPoint {
                    mask: m.mask.id.clone(),
                    surface: m.surface.clone(),
                    index: i,
                    points: m.mask.points.clone(),
                    origin_out: pts[i],
                    start: p,
                    gesture: new_id(),
                    snap: snap_targets(&project, None, Some(&m.mask.id)),
                });
            }
            return;
        }
    }
    // 4. Gewählte Maske: Klick ins Leere hängt einen Punkt an
    if let Some(sel) = sel.filter(|m| !m.locked) {
        let g = new_id();
        let target = snapped(eng, f, p, Some(&sel.mask.id), alt);
        let Some(mp) = out_to_mask(target, sel.surface.as_ref()) else { return };
        let mut points = sel.mask.points.clone();
        points.push(mp);
        let idx = points.len() - 1;
        eng.doc.set_mask_points(&sel.mask.id, points.clone(), Some(&g));
        eng.doc.view.selected_point = Some(idx);
        st.drag = Some(Drag::MaskPoint {
            mask: sel.mask.id.clone(),
            surface: sel.surface.clone(),
            index: idx,
            points,
            origin_out: target,
            start: p,
            gesture: g,
            snap: snap_targets(&project, None, Some(&sel.mask.id)),
        });
        return;
    }
    if sel.is_some() {
        return; // gesperrte Ebene
    }
    // 5. Keine Maske gewählt: neue Maske beginnen – auch innerhalb einer bestehenden (Masken auf Masken)
    let first = snapped(eng, f, p, None, alt);
    st.drag = Some(Drag::Draw { path: vec![first], moved: false });
}

fn on_move(eng: &mut Engine, st: &mut EditorState, f: &Frame, p: Point, alt: bool) {
    let use_snap = eng.doc.view.snap && !alt;
    let Some(d) = st.drag.as_mut() else { return };
    if let Drag::Draw { path, moved } = d {
        let last = *path.last().unwrap();
        if f.px_dist(last, p) < 4.0 {
            return;
        }
        *moved = *moved || f.px_dist(path[0], p) > 6.0;
        path.push(p);
        return;
    }
    let mut mark = None;
    match d {
        Drag::Corner { surface, index, origin, start, gesture, snap } => {
            let mut pt = Point::new(origin.x + p.x - start.x, origin.y + p.y - start.y);
            if use_snap {
                (pt, mark) = snap_point(pt, snap, f.w, f.h);
            }
            let (sid, i, g) = (surface.clone(), *index, gesture.clone());
            eng.doc.update_surface(&sid, Some(&g), |s| {
                let mut q = s.corners;
                q[i] = clamp_point(pt);
                set_corners(s, q);
            });
        }
        Drag::MeshPoint { surface, index, origin, start, gesture, snap } => {
            let mut pt = Point::new(origin.x + p.x - start.x, origin.y + p.y - start.y);
            if use_snap {
                (pt, mark) = snap_point(pt, snap, f.w, f.h);
            }
            let (sid, i, g) = (surface.clone(), *index, gesture.clone());
            eng.doc.update_surface(&sid, Some(&g), |s| {
                if let Some(m) = &mut s.mesh {
                    if let Some(q) = m.points.get_mut(i) {
                        *q = clamp_point(pt);
                    }
                    s.corners = crate::geometry::mesh_corners(m);
                }
            });
        }
        Drag::Move { surface, origin, start, gesture, moved, snap } => {
            let (dx, dy) = (p.x - start.x, p.y - start.y);
            // Kleine Zitterbewegungen beim Anklicken nicht als Verschieben werten
            if !*moved && ((dx * f.w).hypot(dy * f.h)) < 3.0 {
                return;
            }
            *moved = true;
            let mut q = translate_quad(origin, dx, dy);
            if use_snap {
                let (sx, sy, m) = snap_delta(&q, snap, f.w, f.h);
                q = translate_quad(&q, sx, sy);
                mark = m;
            }
            let (sid, g) = (surface.clone(), gesture.clone());
            eng.doc.update_surface(&sid, Some(&g), |s| set_corners(s, clamp_quad(q)));
        }
        Drag::MaskPoint { mask, surface, index, points, origin_out, start, gesture, snap } => {
            let mut pt = Point::new(origin_out.x + p.x - start.x, origin_out.y + p.y - start.y);
            if use_snap {
                (pt, mark) = snap_point(pt, snap, f.w, f.h);
            }
            let Some(mp) = out_to_mask(pt, surface.as_ref()) else { return };
            let mut pts = points.clone();
            if let Some(q) = pts.get_mut(*index) {
                *q = mp;
            }
            let (mid, g) = (mask.clone(), gesture.clone());
            eng.doc.set_mask_points(&mid, pts, Some(&g));
        }
        Drag::Draw { .. } => {}
    }
    st.snap_mark = mark;
}

fn on_end(eng: &mut Engine, st: &mut EditorState, f: &Frame) {
    st.snap_mark = None;
    let Some(Drag::Draw { path, moved }) = st.drag.take() else { return };
    let v = &eng.doc.view;
    let scope = if v.mask_scope == MaskScope::Layer || v.selected_surface.is_none() { MaskScope::Layer } else { MaskScope::Surface };
    // Freihand: Linie vereinfachen; sonst ein einzelner Startpunkt (weiter per Klick)
    let simplified = if moved { simplify_path(&path, 2.5, f.w, f.h) } else { vec![] };
    if simplified.len() >= 3 {
        eng.doc.new_mask(scope, simplified);
        eng.doc.view.selected_point = None;
    } else {
        eng.doc.new_mask(scope, vec![path[0]]);
    }
}

// --- Darstellung

fn label(painter: &egui::Painter, pos: Pos2, text: String, size: f32, color: Color32) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(size), color);
    let r = Rect::from_center_size(pos, galley.size());
    painter.rect_filled(r.expand2(vec2(5.0, 2.0)), 4.0, Color32::from_black_alpha(170));
    painter.galley(r.min, galley, color);
}

fn polygon(painter: &egui::Painter, f: &Frame, pts: &[Point], closed: bool, stroke: Stroke, fill: Color32) {
    if pts.len() < 2 {
        return;
    }
    let screen: Vec<Pos2> = pts.iter().map(|p| f.screen(*p)).collect();
    if closed && pts.len() >= 3 {
        if fill != Color32::TRANSPARENT {
            // Füllung als einzelne Dreiecke (auch für nicht konvexe Masken ausreichend als Andeutung)
            painter.add(EShape::convex_polygon(screen.clone(), fill, Stroke::NONE));
        }
        painter.add(EShape::closed_line(screen, stroke));
    } else {
        painter.add(EShape::line(screen, stroke));
    }
}

fn draw(ui: &Ui, f: &Frame, eng: &Engine, st: &EditorState, on_output: bool) {
    let painter = ui.painter_at(f.rect);
    let view = &eng.doc.view;
    let project = &eng.doc.project;
    let font = if on_output { 22.0 } else { 14.0 };
    let selected = eng.doc.selected_surface();

    // Flächen-Umrisse
    let mut n = 0;
    for l in &project.layers {
        let Layer::Surface(s) = l else { continue };
        n += 1;
        if !s.visible {
            continue;
        }
        let active = Some(&s.id) == view.selected_surface.as_ref();
        let dim = view.tool == Tool::Mask;
        let color = if active {
            ACCENT
        } else if s.locked {
            Color32::from_gray(120)
        } else {
            Color32::from_white_alpha(170)
        };
        let color = if dim { color.gamma_multiply(0.45) } else { color };
        polygon(&painter, f, &shape_outline(s), true, Stroke::new(if active { 2.0 } else { 1.2 }, color), Color32::TRANSPARENT);
        // Gitterlinien des Meshs zeigen die Wölbung
        if active && view.tool == Tool::Surface {
            if let (Shape::Mesh, Some(m)) = (s.shape, &s.mesh) {
                let stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.55));
                let k = 24;
                for c in 0..=m.cols {
                    let pts: Vec<Point> = (0..=k).map(|i| eval_mesh(m, c as f64 / m.cols as f64, i as f64 / k as f64)).collect();
                    polygon(&painter, f, &pts, false, stroke, Color32::TRANSPARENT);
                }
                for r in 0..=m.rows {
                    let pts: Vec<Point> = (0..=k).map(|i| eval_mesh(m, i as f64 / k as f64, r as f64 / m.rows as f64)).collect();
                    polygon(&painter, f, &pts, false, stroke, Color32::TRANSPARENT);
                }
            }
        }
        let c = centroid(if s.shape == Shape::Triangle { &s.corners[..3] } else { &s.corners });
        label(&painter, f.screen(c), format!("{n} · {}{}", s.name, if s.locked { " 🔒" } else { "" }), font, if dim { Color32::from_gray(150) } else { Color32::WHITE });
    }

    // Masken-Ebenen sind im Flächen-Werkzeug als Umriss zu sehen
    if view.tool == Tool::Surface {
        for l in &project.layers {
            if let Layer::Mask(m) = l {
                if m.mask.points.len() >= 2 {
                    let mut c = mode_color(m.mask.mode);
                    if !m.mask.enabled {
                        c = c.gamma_multiply(0.35);
                    }
                    polygon(&painter, f, &m.mask.points, true, Stroke::new(1.5, c), Color32::TRANSPARENT);
                }
            }
        }
    }

    // Eckgriffe
    if view.tool == Tool::Surface {
        for s in project.surfaces().filter(|s| s.visible && !s.locked) {
            let active = Some(&s.id) == view.selected_surface.as_ref();
            if s.shape == Shape::Mesh {
                let (true, Some(m)) = (active, &s.mesh) else { continue };
                for (i, q) in m.points.iter().enumerate() {
                    let sel = view.selected_point == Some(i);
                    let r = if sel { 6.0 } else { 4.0 };
                    let rect = Rect::from_center_size(f.screen(*q), vec2(r * 2.0, r * 2.0));
                    painter.rect_filled(rect, 1.0, if sel { Color32::WHITE } else { ACCENT });
                    painter.rect_stroke(rect, 1.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
                }
                continue;
            }
            for i in 0..s.shape.handle_count() {
                let c = f.screen(s.corners[i]);
                if active && view.selected_corner == Some(i) {
                    painter.circle_stroke(c, 18.0, Stroke::new(2.0, Color32::WHITE));
                }
                painter.circle(c, if active { 10.0 } else { 6.0 }, corner_color(i), Stroke::new(1.0, Color32::BLACK));
                if active {
                    painter.text(c, Align2::CENTER_CENTER, format!("{}", i + 1), FontId::proportional(12.0), Color32::BLACK);
                }
            }
        }
        // Fadenkreuz am gewählten Eck- bzw. Gitterpunkt
        if let Some(s) = selected {
            let cross = match (s.shape, &s.mesh, view.selected_point, view.selected_corner) {
                (Shape::Mesh, Some(m), Some(i), _) => m.points.get(i).map(|q| (*q, ACCENT)),
                (_, _, _, Some(i)) => Some((s.corners[i], corner_color(i))),
                _ => None,
            };
            if let Some((q, color)) = cross {
                let c = f.screen(q);
                let stroke = Stroke::new(1.0, color.gamma_multiply(0.8));
                painter.line_segment([Pos2::new(f.rect.min.x, c.y), Pos2::new(f.rect.max.x, c.y)], stroke);
                painter.line_segment([Pos2::new(c.x, f.rect.min.y), Pos2::new(c.x, f.rect.max.y)], stroke);
            }
        }
    }

    // Masken (Farbe zeigt den Modus: grün hinzufügen, rot ausschneiden, gelb Schnittmenge)
    if view.tool == Tool::Mask {
        for m in mask_list(eng) {
            let pts = mask_points_out(&m.mask, m.surface.as_ref());
            if pts.is_empty() {
                continue;
            }
            let is_sel = Some(&m.mask.id) == view.selected_mask.as_ref();
            let mut c = mode_color(m.mask.mode);
            if !m.mask.enabled {
                c = c.gamma_multiply(0.4);
            }
            let width = if is_sel { 2.5 } else { 1.5 };
            let dashed = m.is_layer;
            if dashed && pts.len() >= 2 {
                let mut screen: Vec<Pos2> = pts.iter().map(|p| f.screen(*p)).collect();
                if pts.len() >= 3 {
                    screen.push(screen[0]);
                }
                painter.extend(EShape::dashed_line(&screen, Stroke::new(width, c), 8.0, 5.0));
            } else {
                polygon(&painter, f, &pts, pts.len() >= 3, Stroke::new(width, c), Color32::TRANSPARENT);
            }
            if pts.len() == 1 {
                painter.circle_filled(f.screen(pts[0]), 3.0, c);
            }
            if is_sel {
                for (i, q) in pts.iter().enumerate() {
                    let sel = view.selected_point == Some(i);
                    let r = if sel { 7.0 } else { 5.0 };
                    let rect = Rect::from_center_size(f.screen(*q), vec2(r * 2.0, r * 2.0));
                    painter.rect_filled(rect, 1.0, if sel { Color32::WHITE } else { c });
                    painter.rect_stroke(rect, 1.0, Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
                }
            }
        }
    }

    // Freihand-Linie während des Zeichnens
    if let Some(Drag::Draw { path, .. }) = &st.drag {
        if path.len() > 1 {
            polygon(&painter, f, path, false, Stroke::new(2.0, Color32::WHITE), Color32::TRANSPARENT);
        }
    }

    // Snap-Anzeige
    if let Some(m) = st.snap_mark {
        let c = f.screen(m);
        let stroke = Stroke::new(1.5, Color32::from_rgb(0xff, 0x4f, 0xd8));
        painter.circle_stroke(c, 11.0, stroke);
        painter.line_segment([c - vec2(16.0, 0.0), c + vec2(16.0, 0.0)], stroke);
        painter.line_segment([c - vec2(0.0, 16.0), c + vec2(0.0, 16.0)], stroke);
    }

    // Hinweis
    let hint = if view.tool == Tool::Mask {
        let sel = eng.doc.selected_mask();
        Some(match sel {
            Some(m) if m.layer.is_some_and(|l| l.locked) => "Ebene gesperrt".to_string(),
            Some(_) => "Klick: Punkt anhängen · Kante: Punkt einfügen · Entf: Punkt löschen · Enter: fertig".to_string(),
            None if view.mask_scope == MaskScope::Layer || selected.is_none() => {
                "Ziehen: Umriss frei zeichnen · Klick: Punkt für Punkt → neue Masken-Ebene (schneidet alles darunter aus)".to_string()
            }
            None => format!("Ziehen: Umriss frei zeichnen · Klick: Punkt für Punkt → neue Maske auf „{}“", selected.map(|s| s.name.as_str()).unwrap_or("")),
        })
    } else {
        None
    };
    if let Some(h) = hint {
        let pos = f.rect.min + vec2(12.0, 10.0);
        let galley = painter.layout(h, FontId::proportional(font * 0.85), Color32::WHITE, f.rect.width() - 24.0);
        painter.rect_filled(Rect::from_min_size(pos, galley.size()).expand(5.0), 4.0, Color32::from_black_alpha(170));
        painter.galley(pos, galley, Color32::WHITE);
    }
}

/// Vollflächiges Kalibrier-Testbild (Gitter) über dem Bild
pub fn draw_test_grid(painter: &egui::Painter, rect: Rect, px_w: u32, px_h: u32) {
    painter.rect_filled(rect, 0.0, Color32::BLACK);
    let white = Stroke::new(1.0, Color32::WHITE);
    let (cols, rows) = (16, 9);
    for i in 0..=cols {
        let x = (rect.min.x + rect.width() * i as f32 / cols as f32).clamp(rect.min.x + 0.5, rect.max.x - 0.5);
        painter.line_segment([Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)], white);
    }
    for j in 0..=rows {
        let y = (rect.min.y + rect.height() * j as f32 / rows as f32).clamp(rect.min.y + 0.5, rect.max.y - 0.5);
        painter.line_segment([Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)], white);
    }
    // Mittelkreuz + Kreis (zeigt Verzerrung durch Keystone/Zoom des Beamers)
    let cyan = Stroke::new(2.0, ACCENT);
    let c = rect.center();
    painter.line_segment([Pos2::new(c.x, rect.min.y), Pos2::new(c.x, rect.max.y)], cyan);
    painter.line_segment([Pos2::new(rect.min.x, c.y), Pos2::new(rect.max.x, c.y)], cyan);
    painter.circle_stroke(c, rect.height() * 0.4, cyan);
    painter.text(
        rect.min + vec2(10.0, 10.0),
        Align2::LEFT_TOP,
        format!("{px_w} × {px_h} px"),
        FontId::proportional((18.0 * rect.height() / 720.0).max(11.0)),
        Color32::WHITE,
    );
}
