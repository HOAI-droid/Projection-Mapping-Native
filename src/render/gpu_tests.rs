// GPU-Tests: zeichnen echte Szenen in eine Textur und prüfen Pixel.
// Laufen mit jedem wgpu-Adapter (auf dem Mac Metal, unter Linux z. B. lavapipe). Ohne Adapter werden
// sie übersprungen.

use std::collections::BTreeMap;

use crate::effects::effects;
use crate::engine::Engine;
use crate::geometry::{apply, create_mesh, square_to_quad};
use crate::model::*;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

fn engine() -> Option<Engine> {
    let (device, queue) = device()?;
    let dir = std::env::temp_dir().join(format!("pm-gpu-{}", new_id()));
    let mut e = Engine::new(device, queue, dir);
    e.doc.replace_project(Project { layers: vec![], ..create_project("Test") }, None);
    e.ref_size = (200, 100);
    Some(e)
}

fn frame(e: &mut Engine) -> Vec<u8> {
    // tick ignoriert zu schnelle Aufrufe – hier jedes Mal neu rechnen
    std::thread::sleep(std::time::Duration::from_millis(9));
    e.tick();
    e.render_output(200, 100);
    e.renderer.read_view(e.output.as_ref().unwrap())
}

fn px(d: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * 200 + x) * 4) as usize;
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

fn quad(x0: f64, y0: f64, x1: f64, y1: f64) -> Quad {
    [Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)]
}

fn color_surface(id: &str, q: Quad, color: &str) -> Surface {
    let mut s = create_surface(id.into(), id.into(), q);
    s.content = Content::Color { color: color.into() };
    s
}

macro_rules! need_gpu {
    () => {
        match engine() {
            Some(e) => e,
            None => {
                eprintln!("kein GPU-Adapter – Test übersprungen");
                return;
            }
        }
    };
}

#[test]
fn draws_color_quad() {
    let mut e = need_gpu!();
    e.doc.project.layers.push(Layer::Surface(color_surface("a", quad(0.25, 0.2, 0.75, 0.8), "#ff0000")));
    let d = frame(&mut e);
    assert_eq!(px(&d, 100, 50), [255, 0, 0, 255]);
    assert_eq!(px(&d, 10, 10)[0], 0, "außerhalb schwarz");
    assert_eq!(px(&d, 160, 50)[0], 0);
}

#[test]
fn perspective_quad_and_shapes() {
    let mut e = need_gpu!();
    // Trapez: oben schmal, unten breit
    let q = [Point::new(0.4, 0.1), Point::new(0.6, 0.1), Point::new(0.9, 0.9), Point::new(0.1, 0.9)];
    e.doc.project.layers.push(Layer::Surface(color_surface("t", q, "#00ff00")));
    let d = frame(&mut e);
    assert_eq!(px(&d, 100, 15)[1], 255, "oben Mitte innen");
    assert_eq!(px(&d, 40, 15)[1], 0, "oben links außen");
    assert_eq!(px(&d, 30, 85)[1], 255, "unten links innen");

    // Kreis: Ecken des Rechtecks bleiben frei
    e.doc.project.layers.clear();
    let mut c = color_surface("c", quad(0.25, 0.0, 0.75, 1.0), "#0000ff");
    c.shape = Shape::Circle;
    e.doc.project.layers.push(Layer::Surface(c));
    let d = frame(&mut e);
    assert_eq!(px(&d, 100, 50)[2], 255);
    assert_eq!(px(&d, 52, 3)[2], 0, "Ecke außerhalb des Kreises");

    // Dreieck: Spitze oben Mitte
    e.doc.project.layers.clear();
    let mut t = color_surface("d", [Point::new(0.5, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0), Point::new(0.0, 1.0)], "#ffffff");
    t.shape = Shape::Triangle;
    e.doc.project.layers.push(Layer::Surface(t));
    let d = frame(&mut e);
    assert_eq!(px(&d, 100, 90)[0], 255);
    assert_eq!(px(&d, 20, 20)[0], 0, "links oben außerhalb des Dreiecks");
}

#[test]
fn surface_masks_and_mask_layers() {
    let mut e = need_gpu!();
    let mut s = color_surface("a", quad(0.0, 0.0, 1.0, 1.0), "#ffffff");
    // Maske „hinzufügen“ im Inhaltsraum: linke Hälfte sichtbar; darin ein Loch
    s.masks.push(create_mask("m1".into(), "M1".into(), quad(0.0, 0.0, 0.5, 1.0).to_vec(), MaskMode::Add));
    s.masks.push(create_mask("m2".into(), "M2".into(), quad(0.1, 0.3, 0.3, 0.7).to_vec(), MaskMode::Subtract));
    e.doc.project.layers.push(Layer::Surface(s));
    let d = frame(&mut e);
    assert_eq!(px(&d, 70, 50)[0], 255, "linke Hälfte sichtbar");
    assert_eq!(px(&d, 150, 50)[0], 0, "rechte Hälfte ausgeblendet");
    assert_eq!(px(&d, 40, 50)[0], 0, "Loch ausgeschnitten");

    // Masken-Ebene schneidet aus allem darunter; Fläche darüber bleibt
    e.doc.project.layers.clear();
    e.doc.project.layers.push(Layer::Surface(color_surface("u", quad(0.0, 0.0, 1.0, 1.0), "#ff0000")));
    e.doc.project.layers.push(Layer::Mask(create_mask_layer("cut".into(), "Cut".into(), quad(0.4, 0.0, 0.6, 1.0).to_vec())));
    e.doc.project.layers.push(Layer::Surface(color_surface("o", quad(0.45, 0.4, 0.55, 0.6), "#00ff00")));
    let d = frame(&mut e);
    assert_eq!(px(&d, 20, 50), [255, 0, 0, 255]);
    assert_eq!(px(&d, 100, 20)[0], 0, "ausgeschnitten");
    assert_eq!(px(&d, 100, 50)[1], 255, "Fläche über der Masken-Ebene bleibt");
    // „Nur Inneres“
    if let Layer::Mask(m) = &mut e.doc.project.layers[1] {
        m.mask.mode = MaskMode::Intersect;
    }
    let d = frame(&mut e);
    assert_eq!(px(&d, 20, 50)[0], 0);
    assert_eq!(px(&d, 85, 20)[0], 255);
}

#[test]
fn mesh_warp_follows_points() {
    let mut e = need_gpu!();
    let mut s = color_surface("m", quad(0.2, 0.2, 0.8, 0.8), "#ffff00");
    let h = square_to_quad(&s.corners).unwrap();
    let mut mesh = create_mesh(|u, v| apply(&h, Point::new(u, v)), 2, 2);
    // Mitte der Oberkante nach oben ziehen → Wölbung
    mesh.points[1] = Point::new(0.5, 0.02);
    s.shape = Shape::Mesh;
    s.mesh = Some(mesh);
    e.doc.project.layers.push(Layer::Surface(s));
    let d = frame(&mut e);
    assert_eq!(px(&d, 100, 8)[0], 255, "nach oben gewölbt");
    assert_eq!(px(&d, 45, 8)[0], 0, "am Rand nicht");
    assert_eq!(px(&d, 100, 70)[1], 255);
}

#[test]
fn blend_master_blackout_and_white() {
    let mut e = need_gpu!();
    e.doc.project.layers.push(Layer::Surface(color_surface("a", quad(0.0, 0.0, 1.0, 1.0), "#800000")));
    let mut b = color_surface("b", quad(0.0, 0.0, 1.0, 1.0), "#008000");
    b.blend = BlendMode::Add;
    e.doc.project.layers.push(Layer::Surface(b));
    let d = frame(&mut e);
    let p = px(&d, 100, 50);
    assert!((p[0] as i32 - 128).abs() <= 1 && (p[1] as i32 - 128).abs() <= 1, "additiv: {p:?}");
    e.doc.show.master = 0.5;
    let p = px(&frame(&mut e), 100, 50);
    assert!((p[0] as i32 - 64).abs() <= 2, "Gesamthelligkeit: {p:?}");
    e.doc.show.blackout = true;
    assert_eq!(px(&frame(&mut e), 100, 50)[0], 0);
    e.doc.show.blackout = false;
    e.doc.view.test_pattern = crate::state::TestPattern::White;
    assert_eq!(px(&frame(&mut e), 3, 3), [255, 255, 255, 255]);
}

#[test]
fn opacity_and_grid() {
    let mut e = need_gpu!();
    let mut s = color_surface("a", quad(0.0, 0.0, 1.0, 1.0), "#ffffff");
    s.opacity = 0.5;
    e.doc.project.layers.push(Layer::Surface(s));
    let p = px(&frame(&mut e), 101, 51);
    assert!((p[0] as i32 - 128).abs() <= 2, "Deckkraft: {p:?}");
    // Raster: 2×2 → Linie in der Mitte
    e.doc.view.grid.enabled = true;
    e.doc.view.grid.cols = 2;
    e.doc.view.grid.rows = 2;
    e.doc.view.grid.color = [255, 0, 0];
    let d = frame(&mut e);
    let line = px(&d, 100, 25);
    assert!(line[0] > 200 && line[1] < 60, "Rasterlinie: {line:?}");
}

#[test]
fn testgrid_texture_on_surface() {
    let mut e = need_gpu!();
    let s = create_surface("g".into(), "G".into(), quad(0.0, 0.0, 1.0, 1.0));
    e.doc.project.layers.push(Layer::Surface(s));
    let d = frame(&mut e);
    // Ecke 1 ist rot (oben links)
    let p = px(&d, 25, 15);
    // klein gerechnet (Mipmaps) – aber deutlich rot
    assert!(p[0] > 60 && p[0] > 2 * p[1], "Ecke 1: {p:?}");
}

#[test]
fn all_effects_render() {
    let mut e = need_gpu!();
    let mut dark = vec![];
    for def in &effects().list {
        let mut s = create_surface("fx".into(), "FX".into(), quad(0.0, 0.0, 1.0, 1.0));
        s.content = Content::Effect { effect: def.id.clone(), params: crate::effects::default_params(def), r#mod: None };
        e.doc.project.layers = vec![Layer::Surface(s)];
        let scope = e.renderer.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut lit = 0;
        // Blitz-Effekte zeigen nur in manchen Bildern etwas
        for _ in 0..40 {
            let d = frame(&mut e);
            lit = lit.max(d.chunks_exact(4).filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 30).count());
            if lit > 0 {
                break;
            }
        }
        if let Some(err) = pollster::block_on(scope.pop()) {
            panic!("Effekt {}: {err}", def.id);
        }
        if lit == 0 {
            dark.push(def.id.clone());
        }
    }
    // Musik-Effekte dürfen ohne Musik dunkel sein, alle anderen müssen etwas zeigen
    let silent_ok: Vec<String> = effects().list.iter().filter(|d| d.audio).map(|d| d.id.clone()).collect();
    let unexpected: Vec<&String> = dark.iter().filter(|d| !silent_ok.contains(d)).collect();
    assert!(unexpected.is_empty(), "Effekte ohne Bild: {unexpected:?}");
}

#[test]
fn modulation_moves_param() {
    let mut e = need_gpu!();
    let mut s = color_surface("a", quad(0.0, 0.0, 1.0, 1.0), "#ffffff");
    s.opacity_mod = Some(Modulation { source: AudioSource::Bass, amount: 1.0, smoothing: 0.0, threshold: 0.0 });
    e.doc.project.layers.push(Layer::Surface(s));
    // Ohne Musik: Bass 0 → bei Stärke 100 % unsichtbar
    let d = frame(&mut e);
    assert_eq!(px(&d, 100, 50)[0], 0);
    let _ = BTreeMap::<String, Modulation>::new();
}
