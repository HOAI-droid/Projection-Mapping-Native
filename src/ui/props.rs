// Eigenschaften der gewählten Fläche: Form, Inhalt (Effekt, Bild, Video, Kamera, Milkdrop),
// Darstellung, Transformation – und die Einstellungen einer gewählten Maske.

use egui::{Color32, RichText, Ui};

use super::editor::mode_color;
use super::{RightTab, Shared};
use crate::effects::{default_params, effects, EffectParam};
use crate::geometry::is_convex_quad;
use crate::media::MediaKind;
use crate::milkdrop::{display_name, Status};
use crate::model::*;

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title).strong());
}

/// Knopf, der gedrückt gehalten wiederholt auslöst
fn repeat_button(ui: &mut Ui, label: &str, hint: &str) -> bool {
    let r = ui.button(label).on_hover_text(hint);
    if r.clicked() {
        return true;
    }
    if r.is_pointer_button_down_on() {
        let held = ui.input(|i| i.pointer.press_start_time().map(|t| i.time - t).unwrap_or(0.0));
        if held > 0.35 {
            return true;
        }
    }
    false
}

pub fn properties(ui: &mut Ui, sh: &mut Shared) {
    // Masken-Ebene gewählt → deren Einstellungen
    if let Some(sel) = sh.eng.doc.selected_mask() {
        if sel.layer.is_some() {
            let m = sel.mask.clone();
            ui.heading(format!("Masken-Ebene · {}", m.name));
            mask_props(ui, sh, &m, true);
            return;
        }
    }
    let Some(s) = sh.eng.doc.selected_surface().cloned() else {
        ui.heading("Eigenschaften");
        ui.label(RichText::new("Fläche oder Masken-Ebene in der Liste oder auf der Bühne anklicken.").weak());
        return;
    };
    ui.heading(format!("Eigenschaften · {}", s.name));
    if let Some(e) = sh.eng.content_errors.get(&s.id) {
        ui.colored_label(Color32::from_rgb(0xf8, 0x71, 0x71), format!("⚠ {e}"));
    }

    shape_section(ui, sh, &s);

    section(ui, "Inhalt");
    let kind = s.content.kind();
    egui::ComboBox::from_id_salt("content-type").selected_text(kind.label()).width(220.0).height(400.0).show_ui(ui, |ui| {
        for t in ContentType::ALL {
            if ui.selectable_label(t == kind, t.label()).clicked() && t != kind {
                choose_type(sh, &s, t);
            }
        }
    });

    match &s.content {
        Content::Effect { .. } => effect_panel(ui, sh, &s),
        Content::Camera { .. } => camera_section(ui, sh, &s),
        Content::Milkdrop { .. } => milkdrop_panel(ui, sh, &s),
        Content::Color { color } => {
            let mut rgb = hex_rgb(color);
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                let g = sh.eng.doc.repeat_gesture(&format!("color-{}", s.id), 800);
                let c = rgb_hex(rgb);
                sh.eng.doc.update_surface(&s.id, Some(&g), |s| s.content = Content::Color { color: c });
            }
        }
        Content::Image { .. } | Content::Video { .. } => media_picker(ui, sh, &s),
        _ => {}
    }

    if let Content::Video { r#loop, speed, playing, .. } = s.content.clone() {
        ui.horizontal(|ui| {
            if ui.button(if playing { "❚❚ Pause" } else { "▶ Abspielen" }).clicked() {
                sh.eng.doc.update_surface(&s.id, None, |s| {
                    if let Content::Video { playing, .. } = &mut s.content {
                        *playing = !*playing;
                    }
                });
            }
            let mut l = r#loop;
            if ui.checkbox(&mut l, "Schleife").changed() {
                sh.eng.doc.update_surface(&s.id, None, |s| {
                    if let Content::Video { r#loop, .. } = &mut s.content {
                        *r#loop = l;
                    }
                });
            }
            egui::ComboBox::from_id_salt("speed").selected_text(format!("{speed}×")).width(70.0).show_ui(ui, |ui| {
                for v in [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0] {
                    if ui.selectable_label(v == speed, format!("{v}×")).clicked() {
                        sh.eng.doc.update_surface(&s.id, None, |s| {
                            if let Content::Video { speed, .. } = &mut s.content {
                                *speed = v;
                            }
                        });
                    }
                }
            });
        });
    }

    if matches!(kind, ContentType::Image | ContentType::Video | ContentType::Camera) {
        section(ui, "Einpassung");
        ui.horizontal(|ui| {
            for f in FitMode::ALL {
                if ui.selectable_label(s.fit == f, f.label()).clicked() {
                    sh.eng.doc.update_surface(&s.id, None, |s| s.fit = f);
                }
            }
        });
    }

    section(ui, "Deckkraft");
    ui.horizontal(|ui| {
        let mut o = s.opacity as f32;
        if ui.add(egui::Slider::new(&mut o, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0} %", v * 100.0))).changed() {
            let g = sh.eng.doc.repeat_gesture(&format!("opacity-{}", s.id), 800);
            sh.eng.doc.update_surface(&s.id, Some(&g), |s| s.opacity = o as f64);
        }
        if mod_toggle(ui, s.opacity_mod.is_some()) {
            let m = if s.opacity_mod.is_some() { None } else { Some(Modulation::default()) };
            sh.eng.doc.update_surface(&s.id, None, |s| s.opacity_mod = m);
        }
    });
    if let Some(m) = s.opacity_mod {
        if let Some(nm) = modulation_editor(ui, &m, &format!("{}-opacity", s.id)) {
            let g = sh.eng.doc.repeat_gesture(&format!("mod-{}-opacity", s.id), 800);
            sh.eng.doc.update_surface(&s.id, Some(&g), |s| s.opacity_mod = Some(nm));
        }
    }

    section(ui, "Mischmodus");
    ui.horizontal_wrapped(|ui| {
        for b in BlendMode::ALL {
            if ui.selectable_label(s.blend == b, b.label()).clicked() {
                sh.eng.doc.update_surface(&s.id, None, |s| s.blend = b);
            }
        }
    });

    section(ui, if s.locked { "Transformieren (gesperrt)" } else { "Transformieren" });
    let aspect = sh.eng.ref_size.0 as f64 / sh.eng.ref_size.1.max(1) as f64;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!s.locked, |ui| {
            if repeat_button(ui, "−", "Kleiner") {
                let g = sh.eng.doc.repeat_gesture("scale-btn", 600);
                sh.eng.doc.scale_selected(1.0 / 1.006, Some(&g));
            }
            if repeat_button(ui, "+", "Größer") {
                let g = sh.eng.doc.repeat_gesture("scale-btn", 600);
                sh.eng.doc.scale_selected(1.006, Some(&g));
            }
            if repeat_button(ui, "↺", "Links drehen") {
                let g = sh.eng.doc.repeat_gesture("rot-btn", 600);
                sh.eng.doc.rotate_selected(-std::f64::consts::PI / 540.0, aspect, Some(&g));
            }
            if repeat_button(ui, "↻", "Rechts drehen") {
                let g = sh.eng.doc.repeat_gesture("rot-btn", 600);
                sh.eng.doc.rotate_selected(std::f64::consts::PI / 540.0, aspect, Some(&g));
            }
            if ui.button("Zurücksetzen").on_hover_text("Ecken zurücksetzen").clicked() {
                sh.eng.doc.reset_corners();
            }
        });
    });
    match sh.eng.doc.view.selected_corner {
        Some(i) => {
            let c = s.corners[i];
            ui.label(RichText::new(format!("Ecke {}: {:.0}, {:.0} px", i + 1, c.x * sh.eng.ref_size.0 as f64, c.y * sh.eng.ref_size.1 as f64)).monospace().weak());
        }
        None => {
            ui.label(RichText::new("Ecke wählen (Klick oder Tab) – dann Pfeiltasten: 1 px, mit ⇧ 10 px.").weak().small());
        }
    }
    if s.shape == Shape::Quad && !is_convex_quad(&s.corners) {
        ui.colored_label(Color32::from_rgb(0xfa, 0xcc, 0x15), "⚠ Das Viereck ist verdreht oder nicht konvex – das Bild kann umklappen.");
    }

    section(ui, "Ausschneiden");
    if ui.button("✂ Form aus Ebenen darunter ausschneiden").on_hover_text("Legt direkt unter dieser Fläche eine Masken-Ebene mit ihrer Form an").clicked() {
        sh.eng.doc.cutout_below(&s.id);
    }
    ui.label(RichText::new("Praktisch, wenn diese Fläche halbtransparent ist oder nur Licht zeigt: Darunterliegende Flächen scheinen dann nicht durch.").weak().small());
}

fn hex_rgb(hex: &str) -> [u8; 3] {
    let c = crate::effects::hex_to_rgb(hex);
    [(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8]
}

fn rgb_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn choose_type(sh: &mut Shared, s: &Surface, t: ContentType) {
    let content = match t {
        ContentType::None => Content::None,
        ContentType::Testgrid => Content::Testgrid,
        ContentType::Color => Content::Color { color: "#ffffff".into() },
        ContentType::Image | ContentType::Video => {
            let kind = if t == ContentType::Image { MediaKind::Image } else { MediaKind::Video };
            match sh.eng.media.items.iter().find(|m| m.kind == kind) {
                Some(m) if kind == MediaKind::Image => Content::Image { media_id: m.id.clone() },
                Some(m) => Content::Video { media_id: m.id.clone(), r#loop: true, speed: 1.0, playing: true },
                None => {
                    sh.ui.notify(if kind == MediaKind::Image { "Noch keine Bilder – im Tab „Medien“ importieren." } else { "Noch keine Videos – im Tab „Medien“ importieren." });
                    sh.ui.right_tab = RightTab::Media;
                    return;
                }
            }
        }
        ContentType::Effect => {
            let def = effects().get("edgechase").unwrap_or(&effects().list[0]);
            Content::Effect { effect: def.id.clone(), params: default_params(def), r#mod: None }
        }
        ContentType::Camera => {
            if sh.eng.camera_list.is_empty() {
                sh.eng.refresh_cameras();
            }
            Content::Camera { device_id: String::new(), mirror: false }
        }
        ContentType::Milkdrop => {
            let names = &sh.eng.milkdrop.presets;
            let preset = if names.is_empty() { String::new() } else { names[rand_index(names.len())].clone() };
            Content::Milkdrop { preset, blend: 2.0, cycle: 0.0, random: true }
        }
    };
    sh.eng.doc.update_surface(&s.id, None, |s| s.content = content);
}

fn rand_index(n: usize) -> usize {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as usize).unwrap_or(0);
    t % n.max(1)
}

fn shape_section(ui: &mut Ui, sh: &mut Shared, s: &Surface) {
    section(ui, "Form");
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(!s.locked, |ui| {
            for shape in Shape::ALL {
                if ui.selectable_label(s.shape == shape, shape.label()).on_hover_text(shape.help()).clicked() {
                    sh.eng.doc.set_shape(&s.id, shape);
                }
            }
        });
    });
    if let (Shape::Mesh, Some(m)) = (s.shape, &s.mesh) {
        ui.horizontal(|ui| {
            let (mut c, mut r) = (m.cols, m.rows);
            let a = ui.add(egui::DragValue::new(&mut c).range(1..=MESH_MAX).prefix("Spalten "));
            let b = ui.add(egui::DragValue::new(&mut r).range(1..=MESH_MAX).prefix("Zeilen "));
            if (a.changed() || b.changed()) && (c, r) != (m.cols, m.rows) {
                sh.eng.doc.resize_mesh(&s.id, c, r);
            }
            if ui.button("Glätten").on_hover_text("Wölbung entfernen, Ecken behalten").clicked() {
                sh.eng.doc.flatten_mesh(&s.id);
            }
        });
        ui.label(RichText::new("Gitterpunkte ziehen, um die Fläche der Wölbung anzupassen. Ecken des Gitters ziehen das ganze Gitter perspektivisch mit (über die Pfeiltasten oder „Zurücksetzen“).").weak().small());
    }
}

/// Kleiner ♪-Knopf neben einem Regler: Musik-Kopplung an/aus. true = umschalten
fn mod_toggle(ui: &mut Ui, on: bool) -> bool {
    let text = RichText::new("♪").color(if on { Color32::BLACK } else { ui.visuals().text_color() });
    let btn = egui::Button::new(text).small().fill(if on { super::editor::ACCENT } else { ui.visuals().widgets.inactive.bg_fill });
    ui.add(btn).on_hover_text(if on { "Musik-Kopplung entfernen" } else { "An die Musik koppeln" }).clicked()
}

/// Einstellungen einer Kopplung; Some = geändert
fn modulation_editor(ui: &mut Ui, m: &Modulation, id: &str) -> Option<Modulation> {
    let mut n = *m;
    let mut changed = false;
    egui::Frame::new().fill(Color32::from_gray(28)).corner_radius(4.0).inner_margin(6).show(ui, |ui| {
        ui.push_id(id, |ui| {
            ui.horizontal_wrapped(|ui| {
                for s in AudioSource::ALL {
                    if ui.selectable_label(n.source == s, s.label()).clicked() {
                        n.source = s;
                        changed = true;
                    }
                }
            });
            let mut a = n.amount as f32;
            changed |= ui.add(egui::Slider::new(&mut a, -1.0..=1.0).text("Stärke").custom_formatter(|v, _| format!("{:.0} %", v * 100.0))).changed();
            n.amount = a as f64;
            let mut sm = n.smoothing as f32;
            changed |= ui.add(egui::Slider::new(&mut sm, 0.0..=0.95).text("Glättung")).changed();
            n.smoothing = sm as f64;
            let mut th = n.threshold as f32;
            changed |= ui.add(egui::Slider::new(&mut th, 0.0..=0.9).text("Schwelle")).changed();
            n.threshold = th as f64;
        });
    });
    changed.then_some(n)
}

fn effect_panel(ui: &mut Ui, sh: &mut Shared, s: &Surface) {
    let Content::Effect { effect, params, r#mod } = &s.content else { return };
    let lib = effects();
    let def = lib.get(effect);
    let cat = sh.ui.effect_category.clone().or_else(|| def.map(|d| d.category.clone())).unwrap_or_else(|| lib.categories[0].clone());
    ui.horizontal_wrapped(|ui| {
        for c in &lib.categories {
            if ui.selectable_label(*c == cat, c).clicked() {
                sh.ui.effect_category = Some(c.clone());
            }
        }
    });
    egui::Grid::new("effects").num_columns(2).spacing([4.0, 4.0]).show(ui, |ui| {
        let mut col = 0;
        for e in lib.list.iter().filter(|e| e.category == cat) {
            let text = format!("{} {}{}", e.icon, e.name, if e.audio { " ♪" } else { "" });
            let btn = egui::Button::new(text).selected(e.id == *effect).min_size(egui::vec2(150.0, 0.0));
            if ui.add(btn).on_hover_text(&e.description).clicked() && e.id != *effect {
                let content = Content::Effect { effect: e.id.clone(), params: default_params(e), r#mod: None };
                sh.eng.doc.update_surface(&s.id, None, |s| s.content = content);
            }
            col += 1;
            if col % 2 == 0 {
                ui.end_row();
            }
        }
    });
    let Some(def) = def else { return };
    ui.add_space(4.0);
    let mut desc = def.description.clone();
    if def.audio {
        desc.push_str(" Reagiert auf die Musik (Quelle im Tab „Audio“ wählen).");
    }
    ui.label(RichText::new(desc).weak().small());
    if let Some(w) = &def.warning {
        ui.colored_label(Color32::from_rgb(0xfa, 0xcc, 0x15), format!("⚠ {w}"));
    }
    let set_param = |sh: &mut Shared, pid: &str, v: ParamValue| {
        let g = sh.eng.doc.repeat_gesture(&format!("fx-{}-{pid}", s.id), 800);
        let pid = pid.to_string();
        sh.eng.doc.update_surface(&s.id, Some(&g), |s| {
            if let Content::Effect { params, .. } = &mut s.content {
                params.insert(pid, v);
            }
        });
    };
    for p in &def.params {
        let cur = params.get(p.id()).cloned().unwrap_or_else(|| p.default_value());
        match p {
            EffectParam::Color { id, label, .. } => {
                ui.horizontal(|ui| {
                    ui.label(label);
                    let mut rgb = hex_rgb(match &cur {
                        ParamValue::Str(s) => s,
                        _ => "#ffffff",
                    });
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        set_param(sh, id, ParamValue::Str(rgb_hex(rgb)));
                    }
                });
            }
            EffectParam::Select { id, label, options, .. } => {
                ui.label(label);
                ui.horizontal_wrapped(|ui| {
                    for (i, o) in options.iter().enumerate() {
                        if ui.selectable_label(cur.num().round() as usize == i, o).clicked() {
                            set_param(sh, id, ParamValue::Num(i as f64));
                        }
                    }
                });
            }
            EffectParam::Float { id, label, min, max, step, default, unit } => {
                let m = r#mod.as_ref().and_then(|m| m.get(id)).copied();
                ui.horizontal(|ui| {
                    let mut v = cur.num();
                    let mut slider = egui::Slider::new(&mut v, *min..=*max).text(format!("{label}{}", unit.as_ref().map(|u| format!(" ({u})")).unwrap_or_default()));
                    if *step > 0.0 {
                        slider = slider.step_by(*step);
                    }
                    let r = ui.add(slider);
                    if r.changed() {
                        set_param(sh, id, ParamValue::Num(v));
                    }
                    if r.double_clicked() {
                        set_param(sh, id, ParamValue::Num(*default));
                    }
                    if mod_toggle(ui, m.is_some()) {
                        let nm = if m.is_some() { None } else { Some(Modulation::default()) };
                        let pid = id.clone();
                        sh.eng.doc.update_surface(&s.id, None, |s| {
                            if let Content::Effect { r#mod, .. } = &mut s.content {
                                let map = r#mod.get_or_insert_with(Default::default);
                                match nm {
                                    Some(x) => {
                                        map.insert(pid, x);
                                    }
                                    None => {
                                        map.remove(&pid);
                                    }
                                }
                                if map.is_empty() {
                                    *r#mod = None;
                                }
                            }
                        });
                    }
                });
                if let Some(m) = m {
                    if let Some(nm) = modulation_editor(ui, &m, &format!("{}-{id}", s.id)) {
                        let g = sh.eng.doc.repeat_gesture(&format!("mod-{}-{id}", s.id), 800);
                        let pid = id.clone();
                        sh.eng.doc.update_surface(&s.id, Some(&g), |s| {
                            if let Content::Effect { r#mod: Some(map), .. } = &mut s.content {
                                map.insert(pid, nm);
                            }
                        });
                    }
                }
            }
        }
    }
    if ui.button("Standardwerte").clicked() {
        let content = Content::Effect { effect: def.id.clone(), params: default_params(def), r#mod: None };
        sh.eng.doc.update_surface(&s.id, None, |s| s.content = content);
    }
}

fn media_picker(ui: &mut Ui, sh: &mut Shared, s: &Surface) {
    let (kind, current) = match &s.content {
        Content::Image { media_id } => (MediaKind::Image, media_id.clone()),
        Content::Video { media_id, .. } => (MediaKind::Video, media_id.clone()),
        _ => return,
    };
    let items: Vec<_> = sh.eng.media.items.iter().filter(|m| m.kind == kind).cloned().collect();
    if items.is_empty() {
        ui.label(RichText::new("Noch nichts importiert – im Tab „Medien“ hinzufügen oder Dateien ins Fenster ziehen.").weak());
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for m in items {
            let selected = m.id == current;
            let tex = super::media_panel::thumb(ui.ctx(), sh, &m.id);
            let resp = match tex {
                Some(t) => {
                    let size = t.size_vec2();
                    let scale = 64.0 / size.x.max(size.y).max(1.0);
                    ui.add(egui::Button::image(egui::Image::new((t.id(), size * scale))).selected(selected))
                }
                None => ui.add(egui::Button::new(&m.name).selected(selected)),
            };
            if resp.on_hover_text(&m.name).clicked() && !selected {
                let id = m.id.clone();
                sh.eng.doc.update_surface(&s.id, None, |s| {
                    s.content = match &s.content {
                        Content::Video { r#loop, speed, .. } => Content::Video { media_id: id, r#loop: *r#loop, speed: *speed, playing: true },
                        _ => Content::Image { media_id: id },
                    }
                });
            }
        }
    });
}

fn camera_section(ui: &mut Ui, sh: &mut Shared, s: &Surface) {
    let Content::Camera { device_id, mirror } = s.content.clone() else { return };
    if sh.eng.camera_list.is_empty() && !sh.eng.camera_scanning() {
        sh.eng.refresh_cameras();
    }
    ui.horizontal(|ui| {
        let label = if device_id.is_empty() { "Standardkamera".to_string() } else { device_id.clone() };
        egui::ComboBox::from_id_salt("camera").selected_text(label).width(220.0).show_ui(ui, |ui| {
            let mut choose = |ui: &mut Ui, id: String, label: &str| {
                if ui.selectable_label(id == device_id, label).clicked() {
                    sh.eng.doc.update_surface(&s.id, None, |s| s.content = Content::Camera { device_id: id.clone(), mirror });
                }
            };
            choose(ui, String::new(), "Standardkamera");
            for c in sh.eng.camera_list.clone() {
                choose(ui, c.clone(), &c);
            }
        });
        if ui.button("⟳").on_hover_text("Kameras neu suchen").clicked() {
            sh.eng.refresh_cameras();
        }
    });
    let mut m = mirror;
    if ui.checkbox(&mut m, "Spiegeln (wie ein Selfie)").changed() {
        let d = device_id.clone();
        sh.eng.doc.update_surface(&s.id, None, |s| s.content = Content::Camera { device_id: d, mirror: m });
    }
    ui.label(
        RichText::new("MacBook-Kamera, iPhone (Continuity-Kamera) oder USB-/HDMI-Grabber. Beim ersten Mal fragt macOS nach der Erlaubnis für die Kamera.")
            .weak()
            .small(),
    );
}

fn milkdrop_panel(ui: &mut Ui, sh: &mut Shared, s: &Surface) {
    let Content::Milkdrop { preset, blend, cycle, random } = s.content.clone() else { return };
    let md = &mut sh.eng.milkdrop;
    match &md.status {
        Status::Error(e) => {
            ui.colored_label(Color32::from_rgb(0xf8, 0x71, 0x71), format!("⚠ {e}"));
            ui.label("Einmalig im Terminal installieren, dann die App neu starten:");
            ui.code("brew install projectm");
        }
        Status::Starting => {
            ui.label(RichText::new("projectM startet …").weak());
        }
        _ => {}
    }

    // Preset-Ordner
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} Presets", md.presets.len())).strong());
        if md.scanning() {
            ui.spinner();
        }
        if ui.button("Ordner …").on_hover_text(md.preset_dir.display().to_string()).clicked() {
            if let Some(dir) = rfd::FileDialog::new().set_directory(&md.preset_dir).pick_folder() {
                md.set_preset_dir(dir);
            }
        }
    });
    if md.presets.is_empty() && !md.scanning() {
        ui.label(RichText::new("Noch keine Presets. Die Sammlung „Cream of the Crop“ (~10 000 Milkdrop-Presets, frei) lässt sich direkt laden:").weak());
    }
    if md.downloading() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Presets werden heruntergeladen …");
        });
    } else if (md.presets.is_empty() || ui.input(|i| i.modifiers.alt)) && ui.button("⬇ Presets herunterladen").clicked() {
        md.download_presets();
    }
    if let Some(e) = &md.download_error {
        ui.colored_label(Color32::from_rgb(0xf8, 0x71, 0x71), e);
    }

    let names = md.presets.clone();
    let set = |sh: &mut Shared, preset: String, blend: f64, cycle: f64, random: bool, g: Option<&str>| {
        sh.eng.doc.update_surface(&s.id, g, |s| s.content = Content::Milkdrop { preset, blend, cycle, random });
    };
    if !names.is_empty() {
        ui.horizontal(|ui| {
            let idx = names.iter().position(|n| *n == preset).unwrap_or(0);
            if ui.button("◀").on_hover_text("Vorheriges").clicked() {
                set(sh, names[(idx + names.len() - 1) % names.len()].clone(), blend, cycle, random, None);
            }
            if ui.button("▶").on_hover_text("Nächstes").clicked() {
                set(sh, names[(idx + 1) % names.len()].clone(), blend, cycle, random, None);
            }
            if ui.button("🎲").on_hover_text("Zufällig").clicked() {
                set(sh, names[rand_index(names.len())].clone(), blend, cycle, random, None);
            }
            ui.label(RichText::new(display_name(&preset)).strong());
        });
        ui.add(egui::TextEdit::singleline(&mut sh.ui.preset_search).hint_text("Preset suchen …").desired_width(f32::INFINITY));
        let q = sh.ui.preset_search.to_lowercase();
        let filtered: Vec<&String> = names.iter().filter(|n| q.is_empty() || n.to_lowercase().contains(&q)).collect();
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        egui::ScrollArea::vertical().id_salt("presets").max_height(220.0).show_rows(ui, row_h, filtered.len(), |ui, range| {
            for n in &filtered[range] {
                if ui.selectable_label(**n == preset, display_name(n)).on_hover_text(*n).clicked() {
                    set(sh, (*n).clone(), blend, cycle, random, None);
                }
            }
        });
    }
    let mut b = blend as f32;
    if ui.add(egui::Slider::new(&mut b, 0.0..=10.0).text("Überblendung s")).changed() {
        let g = sh.eng.doc.repeat_gesture(&format!("md-blend-{}", s.id), 800);
        set(sh, preset.clone(), b as f64, cycle, random, Some(&g));
    }
    let mut c = cycle as f32;
    if ui
        .add(egui::Slider::new(&mut c, 0.0..=120.0).text("Weiterschalten nach s").custom_formatter(|v, _| if v <= 0.0 { "aus".into() } else { format!("{v:.0}") }))
        .changed()
    {
        let g = sh.eng.doc.repeat_gesture(&format!("md-cycle-{}", s.id), 800);
        set(sh, preset.clone(), blend, c as f64, random, Some(&g));
    }
    let mut r = random;
    if ui.checkbox(&mut r, "zufällig weiterschalten").changed() {
        set(sh, preset.clone(), blend, cycle, r, None);
    }
    ui.label(RichText::new("Die originalen Winamp-Visualisierungen über projectM. Musik kommt aus der Audio-Quelle (Tab „Audio“).").weak().small());
}

/// Einstellungen einer Maske – auch für Masken-Ebenen
pub fn mask_props(ui: &mut Ui, sh: &mut Shared, m: &Mask, layer: bool) {
    ui.horizontal(|ui| {
        ui.label("Name");
        let mut name = m.name.clone();
        if ui.text_edit_singleline(&mut name).changed() {
            let g = sh.eng.doc.repeat_gesture(&format!("mask-name-{}", m.id), 1500);
            sh.eng.doc.update_mask(&m.id, Some(&g), |mm| mm.name = name.chars().take(100).collect());
        }
    });
    ui.label("Wirkung");
    let modes: &[MaskMode] = if layer { &[MaskMode::Subtract, MaskMode::Intersect] } else { &[MaskMode::Add, MaskMode::Subtract, MaskMode::Intersect] };
    ui.horizontal(|ui| {
        for &mode in modes {
            let label = if layer {
                if mode == MaskMode::Subtract { "✂ Ausschneiden" } else { "◼ Nur Inneres" }
            } else {
                mode.label()
            };
            let text = RichText::new(label).color(if m.mode == mode { mode_color(mode) } else { ui.visuals().text_color() });
            if ui.selectable_label(m.mode == mode, text).on_hover_text(mode.help()).clicked() {
                sh.eng.doc.update_mask(&m.id, None, |mm| mm.mode = mode);
            }
        }
    });
    let help = if layer {
        match m.mode {
            MaskMode::Subtract => "Alles, was in der Ebenenliste darunter liegt, wird innerhalb der Maske ausgeschnitten.",
            _ => "Von allem darunter bleibt nur das Innere der Maske übrig.",
        }
    } else {
        m.mode.help()
    };
    ui.label(RichText::new(help).weak().small());
    let mut f = m.feather as f32;
    if ui.add(egui::Slider::new(&mut f, 0.0..=100.0).step_by(1.0).text("Weiche Kante px")).changed() {
        let g = sh.eng.doc.repeat_gesture(&format!("feather-{}", m.id), 800);
        sh.eng.doc.update_mask(&m.id, Some(&g), |mm| mm.feather = f as f64);
    }
    if ui.button(RichText::new("Maske löschen").color(Color32::from_rgb(0xf8, 0x71, 0x71))).clicked() {
        sh.eng.doc.remove_mask(&m.id);
    }
    ui.label(RichText::new("Punkt ziehen · Klick auf Kante: Punkt einfügen · Klick ins Leere: Punkt anhängen · Entf: Punkt löschen · Enter: fertig").weak().small());
}
