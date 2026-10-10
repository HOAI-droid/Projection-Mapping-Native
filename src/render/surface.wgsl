// Flächen-Shader: exakte Homographie pro Pixel.
//
// Für jeden Ausgabepixel wird mit der inversen Abbildung zurück in die Inhaltskoordinate (u,v)
// gerechnet. Damit stimmt die Perspektive in jedem Pixel – ohne Knick entlang der Diagonalen wie bei
// zwei affinen Dreiecken. Verzerrungsgitter (Mesh) kommen als feines Dreiecksnetz mit uv je Eckpunkt.
// Gearbeitet wird mit vormultipliziertem Alpha auf schwarzem Hintergrund.

struct U {
  inv0: vec4<f32>,       // Zeilen der inversen Matrix (Ausgaberaum → Inhalt)
  inv1: vec4<f32>,
  inv2: vec4<f32>,
  color: vec4<f32>,      // Farbe (vormultipliziert)
  fit: vec4<f32>,        // Einpassung xy, Deckkraft, Quelle (0 Textur · 1 Farbe · 3 nichts)
  grid: vec4<f32>,       // Spalten, Zeilen, Linienstärke (px), Diagonalen
  grid_color: vec4<f32>, // rgb, Raster an (1/0)
  flags: vec4<f32>,      // Mesh, Form (0 Viereck · 1 Kreis · 2 Dreieck), Maske, –
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var mask_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

struct VIn {
  @location(0) pos: vec2<f32>,
  @location(1) uv: vec2<f32>,
};

struct VOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) pos: vec2<f32>,
  @location(1) uv: vec2<f32>,
};

@vertex
fn vs(v: VIn) -> VOut {
  var o: VOut;
  o.clip = vec4<f32>(v.pos.x * 2.0 - 1.0, 1.0 - v.pos.y * 2.0, 0.0, 1.0);
  o.pos = v.pos;
  o.uv = v.uv;
  return o;
}

// Rasterlinie mit konstanter Breite in Bildschirmpixeln – auch bei starker Verzerrung sichtbar
fn grid_line(uv: vec2<f32>, g: vec2<f32>, fg: vec2<f32>, f1: f32, f2: f32) -> f32 {
  let d = abs(fract(g + 0.5) - 0.5) / fg;
  var line = min(d.x, d.y);
  if (u.grid.w > 0.5) {
    let d1 = abs(uv.x - uv.y) / f1;
    let d2 = abs(uv.x + uv.y - 1.0) / f2;
    line = min(line, min(d1, d2));
  }
  return 1.0 - clamp(line - u.grid.z * 0.5 + 0.5, 0.0, 1.0);
}

@fragment
fn fs(i: VOut) -> @location(0) vec4<f32> {
  var uv = i.uv;
  var w = 1.0;
  if (u.flags.x < 0.5) {
    let p = vec3<f32>(i.pos, 1.0);
    let q = vec3<f32>(dot(u.inv0.xyz, p), dot(u.inv1.xyz, p), dot(u.inv2.xyz, p));
    w = q.z;
    uv = q.xy / q.z;
  }

  // Alle Ableitungen vorab (müssen im einheitlichen Kontrollfluss stehen)
  let fw = max(fwidth(uv), vec2<f32>(1e-6));
  let dc = length(uv - 0.5);
  let fdc = max(fwidth(dc), 1e-6);
  let e = uv.y - abs(uv.x - 0.5) * 2.0;
  let fe = max(fwidth(e), 1e-6);
  let g = uv * u.grid.xy;
  let fg = max(fwidth(g), vec2<f32>(1e-6));
  let f1 = max(fwidth(uv.x - uv.y), 1e-6);
  let f2 = max(fwidth(uv.x + uv.y), 1e-6);
  let t = (uv - 0.5) * u.fit.xy + 0.5;
  let tdx = dpdx(t);
  let tdy = dpdy(t);

  if (w <= 0.0) {
    discard; // Punkt liegt „hinter“ der Ebene
  }

  // 1-px-Kantenglättung je nach Form
  var a: f32;
  let shape = u.flags.y;
  if (shape > 0.5 && shape < 1.5) {
    a = clamp((0.5 - dc) / fdc + 0.5, 0.0, 1.0);
  } else if (shape > 1.5) {
    let ea = clamp(e / fe + 0.5, 0.0, 1.0);
    a = ea * clamp((1.0 - uv.y) / fw.y + 0.5, 0.0, 1.0);
  } else {
    let edge = clamp(uv / fw + 0.5, vec2<f32>(0.0), vec2<f32>(1.0)) * clamp((1.0 - uv) / fw + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    a = edge.x * edge.y;
  }
  if (u.flags.z > 0.5) {
    a = a * textureSampleLevel(mask_tex, samp, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).r;
  }
  if (a <= 0.0) {
    discard;
  }

  var c = vec4<f32>(0.0);
  let source = u.fit.w;
  if (source < 0.5) {
    // Beim Einpassen bleibt der Rand außerhalb des Bildes leer
    let inside = step(vec2<f32>(0.0), t) * step(t, vec2<f32>(1.0));
    c = textureSampleGrad(tex, samp, clamp(t, vec2<f32>(0.0), vec2<f32>(1.0)), tdx, tdy) * inside.x * inside.y;
  } else if (source < 1.5) {
    c = u.color;
  }
  c = c * u.fit.z;
  if (u.grid_color.w > 0.5) {
    c = mix(c, vec4<f32>(u.grid_color.rgb, 1.0), grid_line(uv, g, fg, f1, f2));
  }
  return c * a;
}

