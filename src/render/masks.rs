// Masken werden auf der CPU in ein Alpha-Bild gerastert (weiche Kante = Weichzeichner).
// Neu gerastert wird nur, wenn sich Punkte, Einstellungen oder Größe ändern.
//
// Stapel-Regeln (von oben nach unten wie in der Liste):
//   Beginnt der Stapel mit „hinzufügen“, ist zunächst nichts sichtbar, sonst alles.
//   add = Inneres sichtbar machen · subtract = Inneres ausschneiden ·
//   intersect = nur sichtbar lassen, was auch innerhalb dieser Maske liegt.
// So lassen sich Masken auf Masken setzen (z. B. Fenster aus einer Fassade schneiden und darin
// wieder ein Kreuz sichtbar machen).

use tiny_skia::{FillRule, Mask as SkMask, PathBuilder, Transform};

use crate::model::{Mask, MaskMode, Point};

/// Schlüssel, der sich ändert, sobald das Ergebnis anders aussehen würde
pub fn cache_key(masks: &[&Mask], w: u32, h: u32, feather_scale: f64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (w, h, (feather_scale * 100.0).round() as i64).hash(&mut hasher);
    for m in masks {
        (m.mode as u8, m.feather.to_bits()).hash(&mut hasher);
        for p in &m.points {
            (p.x.to_bits(), p.y.to_bits()).hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Rastert einen Maskenstapel. to_px rechnet einen Maskenpunkt in Bildpixel um;
/// feather_scale wandelt die weiche Kante (Ausgabepixel) in Bildpixel.
pub fn rasterize(masks: &[&Mask], w: u32, h: u32, feather_scale: f64, to_px: impl Fn(Point) -> (f32, f32)) -> Vec<u8> {
    let n = (w * h) as usize;
    // Startzustand: leer, wenn die erste Maske etwas hinzufügt; sonst alles sichtbar
    let start = if masks.first().is_some_and(|m| m.mode == MaskMode::Add) { 0.0 } else { 1.0 };
    let mut acc = vec![start; n];
    for m in masks {
        let Some(shape) = fill_polygon(&m.points, w, h, &to_px) else {
            // Leere/entartete Maske: Schnittmenge mit nichts = nichts
            if m.mode == MaskMode::Intersect {
                acc.iter_mut().for_each(|a| *a = 0.0);
            }
            continue;
        };
        let mut a: Vec<f32> = shape.data().iter().map(|&v| v as f32 / 255.0).collect();
        let sigma = (m.feather * feather_scale / 2.0) as f32;
        if sigma > 0.25 {
            gaussian_blur(&mut a, w as usize, h as usize, sigma);
        }
        for (d, s) in acc.iter_mut().zip(&a) {
            *d = match m.mode {
                MaskMode::Add => s + *d * (1.0 - s),
                MaskMode::Subtract => *d * (1.0 - s),
                MaskMode::Intersect => *d * s,
            };
        }
    }
    acc.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()
}

fn fill_polygon(points: &[Point], w: u32, h: u32, to_px: &impl Fn(Point) -> (f32, f32)) -> Option<SkMask> {
    if points.len() < 3 {
        return None;
    }
    let mut pb = PathBuilder::new();
    for (i, p) in points.iter().enumerate() {
        let (x, y) = to_px(*p);
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.close();
    let path = pb.finish()?;
    let mut mask = SkMask::new(w, h)?;
    mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
    Some(mask)
}

/// Annähernd gaußscher Weichzeichner: drei Boxfilter-Durchgänge je Richtung
fn gaussian_blur(data: &mut [f32], w: usize, h: usize, sigma: f32) {
    // Boxbreite für 3 Durchgänge (Kovesi)
    let wi = ((12.0 * sigma * sigma / 3.0 + 1.0).sqrt()).floor() as usize;
    let r = (wi.max(1) / 2).max(1);
    let mut tmp = vec![0.0f32; data.len()];
    for _ in 0..3 {
        box_h(data, &mut tmp, w, h, r);
        box_v(&tmp, data, w, h, r);
    }
}

fn box_h(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f32;
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let out = &mut dst[y * w..(y + 1) * w];
        let at = |i: isize| row[i.clamp(0, w as isize - 1) as usize];
        let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
        for x in 0..w {
            out[x] = sum * norm;
            sum += at(x as isize + r as isize + 1) - at(x as isize - r as isize);
        }
    }
}

fn box_v(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f32;
    for x in 0..w {
        let at = |i: isize| src[i.clamp(0, h as isize - 1) as usize * w + x];
        let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
        for y in 0..h {
            dst[y * w + x] = sum * norm;
            sum += at(y as isize + r as isize + 1) - at(y as isize - r as isize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::create_mask;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64, mode: MaskMode) -> Mask {
        create_mask("m".into(), "M".into(), vec![Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)], mode)
    }

    fn px(d: &[u8], w: u32, x: u32, y: u32) -> u8 {
        d[(y * w + x) as usize]
    }

    #[test]
    fn add_then_subtract() {
        let outer = square(0.1, 0.1, 0.9, 0.9, MaskMode::Add);
        let hole = square(0.4, 0.4, 0.6, 0.6, MaskMode::Subtract);
        let d = rasterize(&[&outer, &hole], 100, 100, 1.0, |p| (p.x as f32 * 100.0, p.y as f32 * 100.0));
        assert_eq!(px(&d, 100, 2, 2), 0, "außerhalb unsichtbar");
        assert_eq!(px(&d, 100, 20, 20), 255, "innen sichtbar");
        assert_eq!(px(&d, 100, 50, 50), 0, "Loch ausgeschnitten");
    }

    #[test]
    fn subtract_first_starts_visible() {
        let hole = square(0.4, 0.4, 0.6, 0.6, MaskMode::Subtract);
        let d = rasterize(&[&hole], 100, 100, 1.0, |p| (p.x as f32 * 100.0, p.y as f32 * 100.0));
        assert_eq!(px(&d, 100, 2, 2), 255);
        assert_eq!(px(&d, 100, 50, 50), 0);
    }

    #[test]
    fn feather_softens_edge() {
        let mut m = square(0.25, 0.25, 0.75, 0.75, MaskMode::Add);
        m.feather = 10.0;
        let d = rasterize(&[&m], 100, 100, 1.0, |p| (p.x as f32 * 100.0, p.y as f32 * 100.0));
        let edge = px(&d, 100, 25, 50);
        assert!(edge > 60 && edge < 200, "Kante weich: {edge}");
        assert!(px(&d, 100, 50, 50) > 250);
    }

    #[test]
    fn key_changes_with_points() {
        let a = square(0.1, 0.1, 0.9, 0.9, MaskMode::Add);
        let mut b = a.clone();
        b.points[0].x = 0.2;
        assert_ne!(cache_key(&[&a], 10, 10, 1.0), cache_key(&[&b], 10, 10, 1.0));
    }
}
