// Bildschirmfüllende Durchgänge: Masken-Ebenen, Gesamthelligkeit, Weißbild, Kopie

struct FOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) idx: u32) -> FOut {
  let p = vec2<f32>(f32((idx << 1u) & 2u), f32(idx & 2u));
  var o: FOut;
  o.clip = vec4<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
  o.uv = p;
  return o;
}

@group(0) @binding(0) var full_tex: texture_2d<f32>;
@group(0) @binding(1) var full_samp: sampler;

/// Masken-Ebene: Alpha = Maske (verrechnet wird über den Mischmodus der Pipeline)
@fragment
fn fs_mask(i: FOut) -> @location(0) vec4<f32> {
  let m = textureSampleLevel(full_tex, full_samp, i.uv, 0.0).r;
  return vec4<f32>(m);
}

/// Einfarbig (Gesamthelligkeit über die Blend-Konstante, Weißbild)
@fragment
fn fs_solid(i: FOut) -> @location(0) vec4<f32> {
  return vec4<f32>(1.0);
}
