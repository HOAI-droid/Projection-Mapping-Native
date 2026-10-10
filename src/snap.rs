// Einrasten (Snap): Punkte fangen sich an Ecken anderer Flächen, an Maskenpunkten, an Kanten und am
// Bildrand. Gerechnet wird in Bildschirmpixeln, damit sich der Fangradius auf Bühne und Beamer gleich
// anfühlt.

use crate::geometry::{dist_to_segment, shape_outline};
use crate::model::{Layer, Point, Project, Shape};
use crate::state::mask_points_out;

pub const SNAP_RADIUS_PX: f64 = 12.0;

#[derive(Default, Clone)]
pub struct SnapTargets {
    pub points: Vec<Point>,
    pub segments: Vec<(Point, Point)>,
}

const FRAME: [Point; 4] = [Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)];

/// Fangziele sammeln. exclude_*: Fläche bzw. Maske, die gerade bewegt wird
pub fn snap_targets(project: &Project, exclude_surface: Option<&str>, exclude_mask: Option<&str>) -> SnapTargets {
    let mut t = SnapTargets { points: FRAME.to_vec(), segments: (0..4).map(|i| (FRAME[i], FRAME[(i + 1) % 4])).collect() };
    let poly = |t: &mut SnapTargets, pts: &[Point]| {
        t.points.extend_from_slice(pts);
        if pts.len() >= 2 {
            for i in 0..pts.len() {
                t.segments.push((pts[i], pts[(i + 1) % pts.len()]));
            }
        }
    };
    for l in &project.layers {
        match l {
            Layer::Mask(m) => {
                if Some(m.mask.id.as_str()) != exclude_mask && m.mask.enabled {
                    poly(&mut t, &m.mask.points);
                }
            }
            Layer::Surface(s) => {
                if !s.visible || Some(s.id.as_str()) == exclude_surface {
                    continue;
                }
                // Fangpunkte: die Griffe; Kanten: der tatsächliche Umriss (Kreis, Mesh …)
                match (s.shape, &s.mesh) {
                    (Shape::Triangle, _) => t.points.extend_from_slice(&s.corners[..3]),
                    (Shape::Mesh, Some(m)) => t.points.extend_from_slice(&m.points),
                    _ => t.points.extend_from_slice(&s.corners),
                }
                let outline = shape_outline(s);
                for i in 0..outline.len() {
                    t.segments.push((outline[i], outline[(i + 1) % outline.len()]));
                }
                for m in &s.masks {
                    if Some(m.id.as_str()) != exclude_mask && m.enabled {
                        poly(&mut t, &mask_points_out(m, Some(s)));
                    }
                }
            }
        }
    }
    t
}

/// Rastet p ein (zuerst an Punkten, dann an Kanten). Liefert den Fangpunkt für die Anzeige.
pub fn snap_point(p: Point, t: &SnapTargets, w: f64, h: f64) -> (Point, Option<Point>) {
    let px = |q: Point| Point::new(q.x * w, q.y * h);
    let pp = px(p);
    let mut best = None;
    let mut best_d = SNAP_RADIUS_PX;
    for q in &t.points {
        let d = (q.x * w - pp.x).hypot(q.y * h - pp.y);
        if d < best_d {
            best_d = d;
            best = Some(*q);
        }
    }
    if let Some(b) = best {
        return (b, Some(b));
    }
    best_d = SNAP_RADIUS_PX * 0.7;
    for (a, b) in &t.segments {
        let (d, k) = dist_to_segment(pp, px(*a), px(*b));
        if d < best_d {
            best_d = d;
            best = Some(Point::new(a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k));
        }
    }
    match best {
        Some(b) => (b, Some(b)),
        None => (p, None),
    }
}

/// Beim Verschieben ganzer Flächen: Korrektur, damit die nächstgelegene Ecke einrastet
pub fn snap_delta(corners: &[Point], t: &SnapTargets, w: f64, h: f64) -> (f64, f64, Option<Point>) {
    let mut best = (0.0, 0.0, None);
    let mut best_d = SNAP_RADIUS_PX;
    for c in corners {
        for q in &t.points {
            let d = ((q.x - c.x) * w).hypot((q.y - c.y) * h);
            if d < best_d {
                best_d = d;
                best = (q.x - c.x, q.y - c.y, Some(*q));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::create_project;

    #[test]
    fn snaps_to_frame_and_corners() {
        let p = create_project("T");
        let t = snap_targets(&p, None, None);
        let (q, mark) = snap_point(Point::new(0.004, 0.003), &t, 1000.0, 1000.0);
        assert_eq!(q, Point::new(0.0, 0.0));
        assert!(mark.is_some());
        // Ecke der Fläche (0.3, 0.25)
        let (q, _) = snap_point(Point::new(0.305, 0.252), &t, 1000.0, 1000.0);
        assert_eq!(q, Point::new(0.3, 0.25));
        // weit weg: unverändert
        let (q, mark) = snap_point(Point::new(0.5, 0.5), &t, 1000.0, 1000.0);
        assert_eq!(q, Point::new(0.5, 0.5));
        assert!(mark.is_none());
        // Kante: auf die Linie gezogen
        let (q, _) = snap_point(Point::new(0.5, 0.254), &t, 1000.0, 1000.0);
        assert!((q.y - 0.25).abs() < 1e-9);
    }

    #[test]
    fn excludes_moving_surface() {
        let p = create_project("T");
        let id = p.surfaces().next().unwrap().id.clone();
        let t = snap_targets(&p, Some(&id), None);
        assert_eq!(t.points.len(), 4);
        let (dx, dy, _) = snap_delta(&[Point::new(0.995, 0.5)], &t, 1000.0, 1000.0);
        assert_eq!((dx, dy), (0.0, 0.0));
        let (dx, _, _) = snap_delta(&[Point::new(0.995, 0.998)], &t, 1000.0, 1000.0);
        assert!((dx - 0.005).abs() < 1e-9);
    }
}
