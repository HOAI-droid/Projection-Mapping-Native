// Geometrie: Homographie (Corner Pin), Dreieck, Verzerrungsgitter (Mesh) und Helfer für Flächen und Masken.
//
// Matrizen sind 3×3, zeilenweise in einem Array der Länge 9:
// [a b c; d e f; g h i]  →  x' = (a·u + b·v + c) / (g·u + h·v + i), y' analog.
//
// Jede Fläche bildet ihren Inhaltsraum (u,v ∈ 0…1) auf den Ausgaberaum ab:
//   Viereck/Kreis: Homographie aus den 4 Ecken (Kreis = Scheibe im Inhaltsraum)
//   Dreieck:       affin, Spitze (0.5,0) → Ecke 0, (1,1) → Ecke 1, (0,1) → Ecke 2
//   Mesh:          bikubische Catmull-Rom-Interpolation über das Kontrollgitter

use crate::model::{Mesh, Point, Quad, Shape, Surface};

pub type Mat3 = [f64; 9];

const EPS: f64 = 1e-12;

/// Projektive Abbildung vom Einheitsquadrat auf ein Viereck (Heckbert 1989).
/// (0,0)→q[0] oben links, (1,0)→q[1], (1,1)→q[2], (0,1)→q[3]. None bei entartetem Viereck.
pub fn square_to_quad(q: &Quad) -> Option<Mat3> {
    let [p0, p1, p2, p3] = *q;
    let dx1 = p1.x - p2.x;
    let dx2 = p3.x - p2.x;
    let dx3 = p0.x - p1.x + p2.x - p3.x;
    let dy1 = p1.y - p2.y;
    let dy2 = p3.y - p2.y;
    let dy3 = p0.y - p1.y + p2.y - p3.y;
    let (mut g, mut h) = (0.0, 0.0);
    if dx3.abs() > EPS || dy3.abs() > EPS {
        let det = dx1 * dy2 - dx2 * dy1;
        if det.abs() < EPS {
            return None;
        }
        g = (dx3 * dy2 - dx2 * dy3) / det;
        h = (dx1 * dy3 - dx3 * dy1) / det;
    }
    let m = [
        p1.x - p0.x + g * p1.x,
        p3.x - p0.x + h * p3.x,
        p0.x,
        p1.y - p0.y + g * p1.y,
        p3.y - p0.y + h * p3.y,
        p0.y,
        g,
        h,
        1.0,
    ];
    if determinant(&m).abs() < EPS { None } else { Some(m) }
}

pub fn determinant(m: &Mat3) -> f64 {
    let [a, b, c, d, e, f, g, h, i] = *m;
    a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
}

pub fn invert(m: &Mat3) -> Option<Mat3> {
    let [a, b, c, d, e, f, g, h, i] = *m;
    let ca = e * i - f * h;
    let cb = -(d * i - f * g);
    let cc = d * h - e * g;
    let det = a * ca + b * cb + c * cc;
    if det.abs() < EPS {
        return None;
    }
    let s = 1.0 / det;
    Some([
        ca * s,
        -(b * i - c * h) * s,
        (b * f - c * e) * s,
        cb * s,
        (a * i - c * g) * s,
        -(a * f - c * d) * s,
        cc * s,
        -(a * h - b * g) * s,
        (a * e - b * d) * s,
    ])
}

pub fn multiply(m: &Mat3, n: &Mat3) -> Mat3 {
    let mut r = [0.0; 9];
    for row in 0..3 {
        for col in 0..3 {
            r[row * 3 + col] = (0..3).map(|k| m[row * 3 + k] * n[k * 3 + col]).sum();
        }
    }
    r
}

pub fn apply(m: &Mat3, p: Point) -> Point {
    let w = m[6] * p.x + m[7] * p.y + m[8];
    Point::new((m[0] * p.x + m[1] * p.y + m[2]) / w, (m[3] * p.x + m[4] * p.y + m[5]) / w)
}

pub fn quad_to_quad(src: &Quad, dst: &Quad) -> Option<Mat3> {
    let a = square_to_quad(src)?;
    let b = square_to_quad(dst)?;
    Some(multiply(&b, &invert(&a)?))
}

/// Inverse Homographie Ausgaberaum → Einheitsquadrat, so skaliert, dass w im Inneren positiv ist
/// (der Shader verwirft w ≤ 0)
pub fn surface_inverse(corners: &Quad) -> Option<Mat3> {
    let inv = invert(&square_to_quad(corners)?)?;
    let c = centroid(corners);
    let w = inv[6] * c.x + inv[7] * c.y + inv[8];
    Some(if w < 0.0 { inv.map(|v| -v) } else { inv })
}

pub fn is_convex_quad(q: &Quad) -> bool {
    let mut sign = 0.0;
    for i in 0..4 {
        let a = q[i];
        let b = q[(i + 1) % 4];
        let c = q[(i + 2) % 4];
        let cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
        if cross.abs() < EPS {
            return false;
        }
        let s = cross.signum();
        if sign == 0.0 {
            sign = s;
        } else if s != sign {
            return false;
        }
    }
    true
}

// --- Flächen-Helfer (Drehen/Skalieren in Ausgabepixeln, damit 16:9 nicht verzerrt)

pub fn centroid(pts: &[Point]) -> Point {
    let n = pts.len().max(1) as f64;
    let (x, y) = pts.iter().fold((0.0, 0.0), |(x, y), p| (x + p.x, y + p.y));
    Point::new(x / n, y / n)
}

pub fn translate_quad(q: &Quad, dx: f64, dy: f64) -> Quad {
    q.map(|p| Point::new(p.x + dx, p.y + dy))
}

pub fn scale_quad(q: &Quad, factor: f64) -> Quad {
    let c = centroid(q);
    q.map(|p| Point::new(c.x + (p.x - c.x) * factor, c.y + (p.y - c.y) * factor))
}

pub fn rotate_quad(q: &Quad, radians: f64, aspect: f64) -> Quad {
    let c = centroid(q);
    let (sin, cos) = radians.sin_cos();
    q.map(|p| {
        let dx = (p.x - c.x) * aspect;
        let dy = p.y - c.y;
        Point::new(c.x + (dx * cos - dy * sin) / aspect, c.y + dx * sin + dy * cos)
    })
}

/// Punkt-in-Polygon (Even-odd-Regel)
pub fn point_in_polygon(p: Point, poly: &[Point]) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let a = poly[i];
        let b = poly[j];
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Abstand Punkt–Strecke und Parameter t (0…1) des nächsten Punkts
pub fn dist_to_segment(p: Point, a: Point, b: Point) -> (f64, f64) {
    let vx = b.x - a.x;
    let vy = b.y - a.y;
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 0.0 { (((p.x - a.x) * vx + (p.y - a.y) * vy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let x = a.x + t * vx;
    let y = a.y + t * vy;
    ((p.x - x).hypot(p.y - y), t)
}

/// Mittlere Kantenlänge einer Fläche in Pixeln
pub fn quad_pixel_size(q: &Quad, width: f64, height: f64) -> (f64, f64) {
    let len = |a: Point, b: Point| ((b.x - a.x) * width).hypot((b.y - a.y) * height);
    ((len(q[0], q[1]) + len(q[3], q[2])) / 2.0, (len(q[0], q[3]) + len(q[1], q[2])) / 2.0)
}

// --- Dreieck

/// Affine Matrix Inhalt → Ausgabe für ein Dreieck aus den Ecken 0–2
pub fn triangle_matrix(c: &Quad) -> Mat3 {
    let (a, b, d) = (c[0], c[1], c[2]); // Spitze, unten rechts, unten links
    let col0 = Point::new(b.x - d.x, b.y - d.y);
    let col1 = Point::new(0.5 * col0.x - (a.x - d.x), 0.5 * col0.y - (a.y - d.y));
    let off = Point::new(d.x - col1.x, d.y - col1.y);
    [col0.x, col1.x, off.x, col0.y, col1.y, off.y, 0.0, 0.0, 1.0]
}

// --- Mesh

fn grid_point(m: &Mesh, ix: isize, iy: isize) -> Point {
    let (cols, rows) = (m.cols as isize, m.rows as isize);
    // Außerhalb des Gitters linear fortsetzen (für die Randzellen der Interpolation)
    if ix < 0 {
        return lerp_out(grid_point(m, 0, iy), grid_point(m, 1, iy));
    }
    if ix > cols {
        return lerp_out(grid_point(m, cols, iy), grid_point(m, cols - 1, iy));
    }
    if iy < 0 {
        return lerp_out(grid_point(m, ix, 0), grid_point(m, ix, 1));
    }
    if iy > rows {
        return lerp_out(grid_point(m, ix, rows), grid_point(m, ix, rows - 1));
    }
    m.points[(iy * (cols + 1) + ix) as usize]
}

fn lerp_out(edge: Point, inner: Point) -> Point {
    Point::new(2.0 * edge.x - inner.x, 2.0 * edge.y - inner.y)
}

fn catmull(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * (2.0 * p1 + (-p0 + p2) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

/// Punkt des Meshs bei (u,v) – weich interpoliert und exakt durch alle Kontrollpunkte
pub fn eval_mesh(m: &Mesh, u: f64, v: f64) -> Point {
    let fx = u.clamp(0.0, 1.0) * m.cols as f64;
    let fy = v.clamp(0.0, 1.0) * m.rows as f64;
    let ix = (fx.floor() as isize).min(m.cols as isize - 1);
    let iy = (fy.floor() as isize).min(m.rows as isize - 1);
    let s = fx - ix as f64;
    let t = fy - iy as f64;
    let row = |y: isize| {
        let a = grid_point(m, ix - 1, y);
        let b = grid_point(m, ix, y);
        let c = grid_point(m, ix + 1, y);
        let d = grid_point(m, ix + 2, y);
        Point::new(catmull(a.x, b.x, c.x, d.x, s), catmull(a.y, b.y, c.y, d.y, s))
    };
    let (r0, r1, r2, r3) = (row(iy - 1), row(iy), row(iy + 1), row(iy + 2));
    Point::new(catmull(r0.x, r1.x, r2.x, r3.x, t), catmull(r0.y, r1.y, r2.y, r3.y, t))
}

/// Neues Gitter, das eine bestehende Abbildung nachbildet
pub fn create_mesh(map: impl Fn(f64, f64) -> Point, cols: usize, rows: usize) -> Mesh {
    let mut points = Vec::with_capacity((cols + 1) * (rows + 1));
    for y in 0..=rows {
        for x in 0..=cols {
            points.push(map(x as f64 / cols as f64, y as f64 / rows as f64));
        }
    }
    Mesh { cols, rows, points }
}

/// Indizes der vier Gitter-Ecken (oben links, oben rechts, unten rechts, unten links)
pub fn mesh_corner_indices(cols: usize, rows: usize) -> [usize; 4] {
    let w = cols + 1;
    [0, cols, rows * w + cols, rows * w]
}

pub fn mesh_corners(m: &Mesh) -> Quad {
    mesh_corner_indices(m.cols, m.rows).map(|i| m.points[i])
}

/// Feines Dreiecksnetz: pro Zelle sub×sub Teilquadrate; je Eckpunkt Position (Ausgaberaum) und uv
pub fn tessellate_mesh(m: &Mesh, sub: usize) -> Vec<[f32; 4]> {
    let nx = m.cols * sub;
    let ny = m.rows * sub;
    let mut grid = Vec::with_capacity((nx + 1) * (ny + 1));
    for y in 0..=ny {
        for x in 0..=nx {
            grid.push(eval_mesh(m, x as f64 / nx as f64, y as f64 / ny as f64));
        }
    }
    let vert = |x: usize, y: usize| {
        let p = grid[y * (nx + 1) + x];
        [p.x as f32, p.y as f32, (x as f64 / nx as f64) as f32, (y as f64 / ny as f64) as f32]
    };
    let mut out = Vec::with_capacity(nx * ny * 6);
    for y in 0..ny {
        for x in 0..nx {
            out.extend_from_slice(&[vert(x, y), vert(x + 1, y), vert(x, y + 1), vert(x, y + 1), vert(x + 1, y), vert(x + 1, y + 1)]);
        }
    }
    out
}

// --- Alle Formen

/// Inhaltskoordinate → Ausgaberaum
pub fn shape_to_output(s: &Surface, uv: Point) -> Option<Point> {
    match s.shape {
        Shape::Triangle => Some(apply(&triangle_matrix(&s.corners), uv)),
        Shape::Mesh => s.mesh.as_ref().map(|m| eval_mesh(m, uv.x, uv.y)),
        _ => square_to_quad(&s.corners).map(|h| apply(&h, uv)),
    }
}

/// Ausgaberaum → Inhaltskoordinate (beim Mesh numerisch per Newton-Verfahren)
pub fn output_to_shape(s: &Surface, p: Point) -> Option<Point> {
    if s.shape == Shape::Triangle {
        return invert(&triangle_matrix(&s.corners)).map(|inv| apply(&inv, p));
    }
    let hinv = square_to_quad(&s.corners).and_then(|h| invert(&h));
    let mut uv = hinv.map(|h| apply(&h, p)).unwrap_or(Point::new(0.5, 0.5));
    let m = match (&s.mesh, s.shape) {
        (Some(m), Shape::Mesh) => m,
        _ => return hinv.map(|_| uv),
    };
    let e = 1e-4;
    for _ in 0..20 {
        let f = eval_mesh(m, uv.x, uv.y);
        let rx = f.x - p.x;
        let ry = f.y - p.y;
        if rx.hypot(ry) < 1e-7 {
            break;
        }
        let fu = eval_mesh(m, uv.x + e, uv.y);
        let fv = eval_mesh(m, uv.x, uv.y + e);
        let a = (fu.x - f.x) / e;
        let b = (fv.x - f.x) / e;
        let c = (fu.y - f.y) / e;
        let d = (fv.y - f.y) / e;
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            break;
        }
        uv = Point::new(uv.x - (d * rx - b * ry) / det, uv.y - (-c * rx + a * ry) / det);
    }
    Some(uv)
}

/// Umriss der Fläche im Ausgaberaum (für Treffer, Snap und Darstellung)
pub fn shape_outline(s: &Surface) -> Vec<Point> {
    match s.shape {
        Shape::Triangle => s.corners[..3].to_vec(),
        Shape::Circle => match square_to_quad(&s.corners) {
            Some(h) => (0..48)
                .map(|i| {
                    let a = i as f64 / 48.0 * std::f64::consts::TAU;
                    apply(&h, Point::new(0.5 + 0.5 * a.cos(), 0.5 + 0.5 * a.sin()))
                })
                .collect(),
            None => s.corners.to_vec(),
        },
        Shape::Mesh => match &s.mesh {
            Some(m) => {
                let n = 6;
                let nx = m.cols * n;
                let ny = m.rows * n;
                let mut pts = Vec::new();
                for i in 0..nx {
                    pts.push(eval_mesh(m, i as f64 / nx as f64, 0.0));
                }
                for i in 0..ny {
                    pts.push(eval_mesh(m, 1.0, i as f64 / ny as f64));
                }
                for i in (1..=nx).rev() {
                    pts.push(eval_mesh(m, i as f64 / nx as f64, 1.0));
                }
                for i in (1..=ny).rev() {
                    pts.push(eval_mesh(m, 0.0, i as f64 / ny as f64));
                }
                pts
            }
            None => s.corners.to_vec(),
        },
        Shape::Quad => s.corners.to_vec(),
    }
}

/// Neue Ecken setzen; ein vorhandenes Mesh wird perspektivisch mitgeführt
pub fn set_corners(s: &mut Surface, corners: Quad) {
    if s.shape == Shape::Mesh {
        if let Some(m) = &mut s.mesh {
            if let Some(h) = quad_to_quad(&s.corners, &corners) {
                for p in &mut m.points {
                    *p = crate::model::clamp_point(apply(&h, *p));
                }
                s.corners = mesh_corners(m);
                return;
            }
        }
    }
    s.corners = corners;
}

/// Ramer-Douglas-Peucker: vereinfacht eine Freihand-Linie auf die wesentlichen Punkte
pub fn simplify_path(pts: &[Point], epsilon_px: f64, w: f64, h: f64) -> Vec<Point> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let px: Vec<Point> = pts.iter().map(|p| Point::new(p.x * w, p.y * h)).collect();
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    *keep.last_mut().unwrap() = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let mut max_d = 0.0;
        let mut idx = None;
        for i in a + 1..b {
            let d = dist_to_segment(px[i], px[a], px[b]).0;
            if d > max_d {
                max_d = d;
                idx = Some(i);
            }
        }
        if let Some(i) = idx {
            if max_d > epsilon_px {
                keep[i] = true;
                stack.push((a, i));
                stack.push((i, b));
            }
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{create_surface, default_corners};

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6
    }

    #[test]
    fn homography_maps_corners() {
        let q: Quad = [Point::new(0.1, 0.2), Point::new(0.8, 0.1), Point::new(0.9, 0.9), Point::new(0.2, 0.7)];
        let h = square_to_quad(&q).unwrap();
        assert!(close(apply(&h, Point::new(0.0, 0.0)), q[0]));
        assert!(close(apply(&h, Point::new(1.0, 0.0)), q[1]));
        assert!(close(apply(&h, Point::new(1.0, 1.0)), q[2]));
        assert!(close(apply(&h, Point::new(0.0, 1.0)), q[3]));
        let inv = surface_inverse(&q).unwrap();
        let p = apply(&h, Point::new(0.3, 0.6));
        assert!(close(apply(&inv, p), Point::new(0.3, 0.6)));
    }

    #[test]
    fn degenerate_quad() {
        let q: Quad = [Point::new(0.0, 0.0), Point::new(0.5, 0.5), Point::new(1.0, 1.0), Point::new(0.2, 0.2)];
        assert!(square_to_quad(&q).is_none());
        assert!(!is_convex_quad(&q));
        assert!(is_convex_quad(&default_corners()));
    }

    #[test]
    fn triangle_maps_corners() {
        let c: Quad = [Point::new(0.5, 0.1), Point::new(0.9, 0.9), Point::new(0.1, 0.9), Point::new(0.1, 0.9)];
        let m = triangle_matrix(&c);
        assert!(close(apply(&m, Point::new(0.5, 0.0)), c[0]));
        assert!(close(apply(&m, Point::new(1.0, 1.0)), c[1]));
        assert!(close(apply(&m, Point::new(0.0, 1.0)), c[2]));
    }

    #[test]
    fn mesh_interpolates_and_inverts() {
        let mut s = create_surface("a".into(), "A".into(), default_corners());
        let h = square_to_quad(&s.corners).unwrap();
        let mut m = create_mesh(|u, v| apply(&h, Point::new(u, v)), 4, 3);
        // Kontrollpunkte werden exakt getroffen
        for (i, p) in m.points.iter().enumerate() {
            let (x, y) = (i % 5, i / 5);
            assert!(close(eval_mesh(&m, x as f64 / 4.0, y as f64 / 3.0), *p));
        }
        m.points[6].x += 0.05;
        s.shape = Shape::Mesh;
        s.mesh = Some(m);
        let target = Point::new(0.37, 0.61);
        let out = shape_to_output(&s, target).unwrap();
        let back = output_to_shape(&s, out).unwrap();
        assert!((back.x - target.x).abs() < 1e-5 && (back.y - target.y).abs() < 1e-5);
        let tri = tessellate_mesh(s.mesh.as_ref().unwrap(), 2);
        assert_eq!(tri.len(), 4 * 3 * 4 * 6);
    }

    #[test]
    fn set_corners_moves_mesh() {
        let mut s = create_surface("a".into(), "A".into(), default_corners());
        let h = square_to_quad(&s.corners).unwrap();
        s.mesh = Some(create_mesh(|u, v| apply(&h, Point::new(u, v)), 2, 2));
        s.shape = Shape::Mesh;
        let moved = translate_quad(&s.corners, 0.1, 0.0);
        set_corners(&mut s, moved);
        assert!(close(s.corners[0], moved[0]));
        assert!(close(s.mesh.as_ref().unwrap().points[4], Point::new(0.6, 0.5)));
    }

    #[test]
    fn polygon_and_simplify() {
        let sq = [Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)];
        assert!(point_in_polygon(Point::new(0.5, 0.5), &sq));
        assert!(!point_in_polygon(Point::new(1.5, 0.5), &sq));
        let line: Vec<Point> = (0..=10).map(|i| Point::new(i as f64 / 10.0, 0.0)).collect();
        assert_eq!(simplify_path(&line, 1.0, 100.0, 100.0).len(), 2);
    }

    #[test]
    fn rotate_keeps_centroid() {
        let q = default_corners();
        let r = rotate_quad(&q, 0.5, 16.0 / 9.0);
        assert!(close(centroid(&r), centroid(&q)));
    }
}
