// Effekt-Bibliothek: generative Inhalte als GLSL-Fragment-Shader (wie die „Materials“ in MadMapper).
//
// Die Definitionen stammen 1:1 aus der Web-Version (assets/effects.json). Ein Effekt ist eine Funktion
//
//   vec4 effect(vec2 uv)   // uv: 0…1 auf der Fläche, (0,0) oben links → Farbe + Alpha
//
// Zur Verfügung stehen: uTime, uAspect, uSeed, uAudio (Pegel, Bass, Mitten, Höhen), uBeat, uBeatCount,
// uBpm, die Hilfsfunktionen aus der Bibliothek (spectrum(), peakAt(), waveAt(), perimeter() …) und je
// Parameter ein Wert p_<id> (float; Farben als vec3; Auswahl als float-Index).
// Rückkopplungs-Effekte lesen mit prev(uv) ihr voriges Bild.
//
// Jeder Effekt wird in eine eigene Textur gerechnet (Größe = Fläche in Ausgabepixeln) und dann wie ein
// Bild auf die Fläche gelegt. wgpu übersetzt das GLSL über naga nach Metal.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use serde::Deserialize;

use crate::model::ParamValue;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EffectParam {
    Float {
        id: String,
        label: String,
        min: f64,
        max: f64,
        step: f64,
        default: f64,
        #[serde(default)]
        unit: Option<String>,
    },
    Color {
        id: String,
        label: String,
        default: String,
    },
    Select {
        id: String,
        label: String,
        options: Vec<String>,
        default: f64,
    },
}

impl EffectParam {
    pub fn id(&self) -> &str {
        match self {
            EffectParam::Float { id, .. } | EffectParam::Color { id, .. } | EffectParam::Select { id, .. } => id,
        }
    }

    pub fn default_value(&self) -> ParamValue {
        match self {
            EffectParam::Float { default, .. } | EffectParam::Select { default, .. } => ParamValue::Num(*default),
            EffectParam::Color { default, .. } => ParamValue::Str(default.clone()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct EffectDef {
    pub id: String,
    pub name: String,
    pub category: String,
    pub icon: String,
    pub description: String,
    #[serde(default)]
    pub warning: Option<String>,
    #[serde(default)]
    pub audio: bool,
    #[serde(default)]
    pub feedback: bool,
    pub params: Vec<EffectParam>,
    pub glsl: String,
}

#[derive(Deserialize)]
struct Library {
    lib: String,
    categories: Vec<String>,
    effects: Vec<EffectDef>,
}

pub struct Effects {
    pub lib: String,
    pub categories: Vec<String>,
    pub list: Vec<EffectDef>,
    map: HashMap<String, usize>,
}

impl Effects {
    pub fn get(&self, id: &str) -> Option<&EffectDef> {
        self.map.get(id).map(|&i| &self.list[i])
    }
}

/// Alle Effekte (einmalig aus der eingebetteten JSON-Datei gelesen)
pub fn effects() -> &'static Effects {
    static LIB: OnceLock<Effects> = OnceLock::new();
    LIB.get_or_init(|| {
        let lib: Library = serde_json::from_str(include_str!("../assets/effects.json")).expect("effects.json fehlerhaft");
        let map = lib.effects.iter().enumerate().map(|(i, e)| (e.id.clone(), i)).collect();
        Effects { lib: lib.lib, categories: lib.categories, list: lib.effects, map }
    })
}

pub fn default_params(def: &EffectDef) -> BTreeMap<String, ParamValue> {
    def.params.iter().map(|p| (p.id().to_string(), p.default_value())).collect()
}

/// Höchstzahl Parameter je Effekt (je ein vec4 im Uniform-Block)
pub const MAX_PARAMS: usize = 32;

/// Uniform-Block der Effekt-Shader (std140: alles in vec4-Schritten)
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EffectUniforms {
    pub audio: [f32; 4],
    /// uTime, uAspect, uSeed, uBeat
    pub a: [f32; 4],
    /// uBeatCount, uBpm, –, –
    pub b: [f32; 4],
    pub params: [[f32; 4]; MAX_PARAMS],
}

/// Vertex-Shader für Effekt-Durchgänge: ein bildschirmfüllendes Dreieck, uv (0,0) oben links
pub const EFFECT_VERT_WGSL: &str = r#"
struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
  let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
  var o: Out;
  o.pos = vec4<f32>(p.x * 2.0 - 1.0, 1.0 - p.y * 2.0, 0.0, 1.0);
  o.uv = p;
  return o;
}
"#;

/// Vollständiger GLSL-Fragment-Shader eines Effekts
pub fn fragment_glsl(def: &EffectDef) -> String {
    let lib = &effects().lib;
    let mut defines = String::new();
    for (i, p) in def.params.iter().enumerate().take(MAX_PARAMS) {
        let swz = if matches!(p, EffectParam::Color { .. }) { "xyz" } else { "x" };
        defines.push_str(&format!("#define p_{} (P[{i}].{swz})\n", p.id()));
    }
    format!(
        r#"#version 450
layout(location = 0) in vec2 vUv;
layout(location = 0) out vec4 outColor;
layout(set = 0, binding = 0) uniform Globals {{
  vec4 uAudio;
  vec4 uA;
  vec4 uB;
  vec4 P[{MAX_PARAMS}];
}};
layout(set = 0, binding = 1) uniform texture2D tAudio;
layout(set = 0, binding = 2) uniform texture2D tPrev;
layout(set = 0, binding = 3) uniform sampler sLinear;
#define uTime (uA.x)
#define uAspect (uA.y)
#define uSeed (uA.z)
#define uBeat (uA.w)
#define uBeatCount (uB.x)
#define uBpm (uB.y)
#define uAudioTex sampler2D(tAudio, sLinear)
vec4 prev(vec2 uv) {{ return texture(sampler2D(tPrev, sLinear), uv); }}
{defines}
{lib}
{glsl}
void main() {{
  vec4 e = clamp(effect(vUv), 0.0, 1.0);
  outColor = vec4(e.rgb * e.a, e.a);
}}
"#,
        glsl = def.glsl
    )
}

/// Übersetzt einen Effekt mit naga (für Tests und um Fehler vor wgpu abzufangen)
pub fn parse_effect(def: &EffectDef) -> Result<naga::Module, String> {
    let src = fragment_glsl(def);
    let mut frontend = naga::front::glsl::Frontend::default();
    let module = frontend
        .parse(&naga::front::glsl::Options::from(naga::ShaderStage::Fragment), &src)
        .map_err(|e| e.emit_to_string(&src))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|e| e.emit_to_string(&src))?;
    Ok(module)
}

pub fn hex_to_rgb(hex: &str) -> [f32; 3] {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    [((n >> 16) & 255) as f32 / 255.0, ((n >> 8) & 255) as f32 / 255.0, (n & 255) as f32 / 255.0]
}

/// Parameterwerte in den Uniform-Block schreiben
pub fn write_params(def: &EffectDef, values: &BTreeMap<String, ParamValue>, out: &mut [[f32; 4]; MAX_PARAMS]) {
    for (i, p) in def.params.iter().enumerate().take(MAX_PARAMS) {
        let v = values.get(p.id()).cloned().unwrap_or_else(|| p.default_value());
        out[i] = match (p, v) {
            (EffectParam::Color { .. }, ParamValue::Str(s)) => {
                let c = hex_to_rgb(&s);
                [c[0], c[1], c[2], 1.0]
            }
            (EffectParam::Color { default, .. }, _) => {
                let c = hex_to_rgb(default);
                [c[0], c[1], c[2], 1.0]
            }
            (_, ParamValue::Num(n)) => [n as f32, 0.0, 0.0, 0.0],
            (_, ParamValue::Str(s)) => [s.parse().unwrap_or(0.0), 0.0, 0.0, 0.0],
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_effects_compile() {
        let mut failed = Vec::new();
        for def in &effects().list {
            if let Err(e) = parse_effect(def) {
                failed.push(format!("--- {}\n{e}", def.id));
            }
        }
        assert!(failed.is_empty(), "{} Effekte fehlerhaft:\n{}", failed.len(), failed.join("\n"));
        assert_eq!(effects().list.len(), 41);
    }

    #[test]
    fn params_fit() {
        for def in &effects().list {
            assert!(def.params.len() <= MAX_PARAMS, "{}", def.id);
        }
    }
}

#[cfg(test)]
mod metal_tests {
    use super::*;

    fn to_msl(module: &naga::Module) -> Result<String, String> {
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(module)
            .map_err(|e| format!("{e:?}"))?;
        let options = naga::back::msl::Options { lang_version: (2, 4), ..Default::default() };
        naga::back::msl::write_string(module, &info, &options, &naga::back::msl::PipelineOptions::default())
            .map(|(s, _)| s)
            .map_err(|e| format!("{e:?}"))
    }

    /// Alle Shader lassen sich nach Metal (MSL) übersetzen – so wie wgpu es auf dem Mac tut
    #[test]
    fn all_shaders_translate_to_metal() {
        for def in &effects().list {
            let m = parse_effect(def).unwrap();
            to_msl(&m).unwrap_or_else(|e| panic!("{}: {e}", def.id));
        }
        for src in [include_str!("render/surface.wgsl"), include_str!("render/full.wgsl"), EFFECT_VERT_WGSL] {
            let m = naga::front::wgsl::parse_str(src).unwrap();
            to_msl(&m).unwrap();
        }
    }
}
