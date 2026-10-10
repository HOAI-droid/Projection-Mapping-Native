// Testbild für eine Fläche: Gitter, Diagonalen, Kreis und farbige, nummerierte Ecken.

use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

/// Farben der vier Ecken – identisch in Steuerung, Testgitter und Ausgabe
pub const CORNER_COLORS: [[u8; 3]; 4] = [[0xff, 0x5a, 0x5a], [0x4a, 0xde, 0x80], [0x60, 0xa5, 0xfa], [0xfa, 0xcc, 0x15]];

/// 5×7-Pixelschrift für die wenigen Zeichen des Testbilds
fn glyph(c: char) -> [u8; 7] {
    match c {
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        _ => [0; 7],
    }
}

/// Text mittig um (cx, cy) zeichnen; px = Größe eines Schriftpixels
fn text(pm: &mut Pixmap, s: &str, cx: f32, cy: f32, px: f32, color: Color) {
    let mut paint = Paint::default();
    paint.set_color(color);
    let n = s.chars().count() as f32;
    let w = n * 6.0 * px - px;
    let x0 = cx - w / 2.0;
    let y0 = cy - 3.5 * px;
    for (i, c) in s.chars().enumerate() {
        let g = glyph(c);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    let x = x0 + (i as f32 * 6.0 + col as f32) * px;
                    let y = y0 + row as f32 * px;
                    if let Some(r) = Rect::from_xywh(x, y, px + 0.5, px + 0.5) {
                        pm.fill_rect(r, &paint, Transform::identity(), None);
                    }
                }
            }
        }
    }
}

fn stroke_line(pm: &mut Pixmap, pts: &[(f32, f32)], width: f32, color: Color) {
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for p in &pts[1..] {
        pb.line_to(p.0, p.1);
    }
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        pm.stroke_path(&path, &paint, &Stroke { width, ..Default::default() }, Transform::identity(), None);
    }
}

/// Erzeugt das Testbild (quadratisch, RGBA)
pub fn create(size: u32) -> (u32, u32, Vec<u8>) {
    let mut pm = Pixmap::new(size, size).expect("Testbild");
    pm.fill(Color::BLACK);
    let s = size as f32;
    let step = s / 16.0;
    for i in 0..=16 {
        let major = i % 4 == 0;
        let c = if major { Color::WHITE } else { Color::from_rgba8(0x66, 0x66, 0x66, 255) };
        let w = if major { 3.0 } else { 1.5 };
        let p = (i as f32 * step).round();
        stroke_line(&mut pm, &[(p, 0.0), (p, s)], w, c);
        stroke_line(&mut pm, &[(0.0, p), (s, p)], w, c);
    }
    // Diagonalen und Kreis zeigen sofort, ob die Perspektive stimmt
    let grey = Color::from_rgba8(0x88, 0x88, 0x88, 255);
    stroke_line(&mut pm, &[(0.0, 0.0), (s, s)], 2.0, grey);
    stroke_line(&mut pm, &[(s, 0.0), (0.0, s)], 2.0, grey);
    if let Some(circle) = PathBuilder::from_circle(s / 2.0, s / 2.0, s * 0.375) {
        let mut paint = Paint::default();
        paint.set_color(Color::WHITE);
        paint.anti_alias = true;
        pm.stroke_path(&circle, &paint, &Stroke { width: 4.0, ..Default::default() }, Transform::identity(), None);
    }
    stroke_line(&mut pm, &[(6.0, 6.0), (s - 6.0, 6.0), (s - 6.0, s - 6.0), (6.0, s - 6.0), (6.0, 6.0)], 12.0, Color::WHITE);

    let r = s * 0.09;
    let pos = [(r * 1.3, r * 1.3), (s - r * 1.3, r * 1.3), (s - r * 1.3, s - r * 1.3), (r * 1.3, s - r * 1.3)];
    for (i, (x, y)) in pos.iter().enumerate() {
        let [cr, cg, cb] = CORNER_COLORS[i];
        if let Some(c) = PathBuilder::from_circle(*x, *y, r) {
            let mut paint = Paint::default();
            paint.set_color(Color::from_rgba8(cr, cg, cb, 255));
            paint.anti_alias = true;
            pm.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
        text(&mut pm, &format!("{}", i + 1), *x, *y, r * 0.16, Color::BLACK);
    }

    // „▲ OBEN“
    let tri_y = s * 0.2;
    let th = s * 0.035;
    let mut pb = PathBuilder::new();
    let tx = s / 2.0 - s * 0.11;
    pb.move_to(tx, tri_y - th);
    pb.line_to(tx + th, tri_y + th * 0.8);
    pb.line_to(tx - th, tri_y + th * 0.8);
    pb.close();
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(Color::WHITE);
        paint.anti_alias = true;
        pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }
    text(&mut pm, "OBEN", s / 2.0 + s * 0.035, tri_y, s * 0.009, Color::WHITE);

    (size, size, pm.take())
}

#[cfg(test)]
mod tests {
    #[test]
    fn creates_image() {
        let (w, h, d) = super::create(256);
        assert_eq!((w, h), (256, 256));
        assert_eq!(d.len(), 256 * 256 * 4);
        // Ecke 1 ist rot
        let i = ((256.0 * 0.09 * 1.3) as usize * 256 + (256.0 * 0.09 * 0.6) as usize) * 4;
        assert!(d[i] > 200 && d[i + 1] < 120);
    }
}
