// Kopfleiste, Werkzeugleiste, Ebenen- und Maskenliste, Dialoge.

use std::time::Duration;

use egui::{Color32, RichText, Ui};

use super::editor::{mode_color, ACCENT};
use super::{RightTab, Shared};
use crate::model::{Content, Layer, MaskMode};
use crate::state::{MaskScope, TestPattern, Tool};

pub fn save(sh: &mut Shared) {
    match sh.eng.doc.path.clone() {
        Some(p) => match sh.eng.save_to(p) {
            Ok(()) => sh.ui.notify("Gespeichert"),
            Err(e) => sh.ui.notify(format!("Speichern fehlgeschlagen: {e}")),
        },
        None => save_as(sh),
    }
}

pub fn save_as(sh: &mut Shared) {
    let name = sanitize_file_name(&sh.eng.doc.project.name);
    let mut dialog = rfd::FileDialog::new().add_filter("Projection-Mapping-Projekt", &["json"]).set_file_name(format!("{name}.json"));
    if let Some(dir) = sh.eng.doc.path.as_ref().and_then(|p| p.parent()) {
        dialog = dialog.set_directory(dir);
    }
    if let Some(mut path) = dialog.save_file() {
        if path.extension().is_none() {
            path.set_extension("json");
        }
        match sh.eng.save_to(path) {
            Ok(()) => sh.ui.notify("Gespeichert"),
            Err(e) => sh.ui.notify(format!("Speichern fehlgeschlagen: {e}")),
        }
    }
}

pub fn open(sh: &mut Shared) {
    if let Some(path) = rfd::FileDialog::new().add_filter("Projection-Mapping-Projekt", &["json"]).pick_file() {
        match sh.eng.open(path) {
            Ok(()) => sh.ui.notify("Projekt geladen"),
            Err(e) => sh.ui.notify(format!("Öffnen fehlgeschlagen: {e}")),
        }
    }
}

fn sanitize_file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
    if s.trim().is_empty() { "Projekt".into() } else { s.trim().to_string() }
}

pub fn top_bar(ui: &mut Ui, sh: &mut Shared) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("◆ Projection Mapping").strong().color(ACCENT));
        let mut name = sh.eng.doc.project.name.clone();
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(170.0).hint_text("Projektname"));
        if r.changed() {
            let g = sh.eng.doc.repeat_gesture("project-name", 1500);
            sh.eng.doc.edit(Some(&g), |p| p.name = name.chars().take(100).collect());
        }
        if sh.eng.doc.dirty {
            ui.label(RichText::new("●").color(Color32::from_rgb(0xfa, 0xcc, 0x15))).on_hover_text("Ungespeicherte Änderungen (der Stand wird trotzdem automatisch gesichert)");
        }
        ui.separator();
        if ui.button("Neu").clicked() {
            sh.ui.confirm_new = true;
        }
        if ui.button("Öffnen …").on_hover_text("⌘O").clicked() {
            open(sh);
        }
        if ui.button("Speichern").on_hover_text("⌘S").clicked() {
            save(sh);
        }
        if ui.button("Speichern unter …").on_hover_text("⇧⌘S").clicked() {
            save_as(sh);
        }
        ui.separator();
        if ui.add_enabled(sh.eng.doc.history.can_undo(), egui::Button::new("↶")).on_hover_text("Rückgängig (⌘Z)").clicked() {
            sh.eng.doc.undo();
        }
        if ui.add_enabled(sh.eng.doc.history.can_redo(), egui::Button::new("↷")).on_hover_text("Wiederholen (⇧⌘Z)").clicked() {
            sh.eng.doc.redo();
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("?").on_hover_text("Tastenkürzel und Hilfe").clicked() {
                sh.ui.show_help = !sh.ui.show_help;
            }
            // Blackout
            let bo = sh.eng.doc.show.blackout;
            let label = RichText::new(if bo { "■ BLACKOUT" } else { "Blackout" }).strong();
            let btn = egui::Button::new(if bo { label.color(Color32::WHITE) } else { label }).fill(if bo { Color32::from_rgb(0xb9, 0x1c, 0x1c) } else { ui.visuals().widgets.inactive.bg_fill });
            if ui.add(btn).on_hover_text("Alles schwarz (Taste B)").clicked() {
                sh.eng.doc.show.blackout = !bo;
            }
            let mut m = sh.eng.doc.show.master as f32;
            if ui.add(egui::Slider::new(&mut m, 0.0..=1.0).show_value(false)).on_hover_text("Gesamthelligkeit").changed() {
                sh.eng.doc.show.master = m as f64;
            }
            ui.label(format!("Hell {:.0} %", sh.eng.doc.show.master * 100.0));
            ui.separator();
            // Ausgabe
            if sh.ui.output_open {
                if ui.button(if sh.ui.output_fullscreen { "Vollbild beenden" } else { "Vollbild (F)" }).clicked() {
                    let fs = !sh.ui.output_fullscreen;
                    ui.ctx().send_viewport_cmd_to(super::output_viewport(), egui::ViewportCommand::Fullscreen(fs));
                    sh.ui.output_fullscreen = fs;
                }
                if ui.button("Ausgabe schließen").clicked() {
                    sh.ui.output_open = false;
                    sh.eng.settings.output_open = false;
                }
            } else if ui.button(RichText::new("▶ Ausgabe öffnen").strong()).on_hover_text("Fenster für den Beamer öffnen").clicked() {
                sh.ui.output_open = true;
                sh.eng.settings.output_open = true;
            }
            ui.label(RichText::new(format!("{} × {} · {:.0} fps", sh.eng.ref_size.0, sh.eng.ref_size.1, sh.eng.fps)).weak().small());
        });
    });
}

pub fn toolbar(ui: &mut Ui, sh: &mut Shared) {
    let v = &mut sh.eng.doc.view;
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut v.tool, Tool::Surface, "⬚ Fläche  V").on_hover_text("Ecken ziehen, Flächen verschieben");
        ui.selectable_value(&mut v.tool, Tool::Mask, "✎ Maske  M").on_hover_text("Masken zeichnen und bearbeiten");
        if v.tool == Tool::Mask {
            ui.separator();
            ui.selectable_value(&mut v.mask_scope, MaskScope::Surface, "Neu: auf Fläche").on_hover_text("Neue Maske gehört zur gewählten Fläche und wandert mit deren Ecken mit");
            if ui.selectable_label(v.mask_scope == MaskScope::Layer, "Neu: als Ebene ✂").on_hover_text("Neue Masken-Ebene schneidet aus allen Ebenen darunter aus").clicked() {
                v.mask_scope = MaskScope::Layer;
                v.selected_mask = None;
                v.selected_point = None;
            }
        }
        ui.separator();
        let snap_label = if v.snap { "◎ Snap an  S" } else { "◎ Snap aus  S" };
        ui.toggle_value(&mut v.snap, snap_label)
            .on_hover_text("Punkte rasten an Ecken, Maskenpunkten und Kanten ein (⌥ beim Ziehen: ohne Snap)");
        ui.toggle_value(&mut v.grid.enabled, "▦ Raster  G").on_hover_text("Raster auf die Flächen legen, um die Verzerrung zu prüfen");
        ui.separator();
        ui.label("Testbild");
        ui.selectable_value(&mut v.test_pattern, TestPattern::None, "Aus");
        ui.selectable_value(&mut v.test_pattern, TestPattern::Grid, "Gitter");
        ui.selectable_value(&mut v.test_pattern, TestPattern::White, "Weiß");
        ui.separator();
        let label = if v.edit_mode { "✎ Bearbeiten  E" } else { "▶ Show  E" };
        ui.toggle_value(&mut v.edit_mode, label).on_hover_text("Bearbeiten: Umrisse und Griffe auf der Ausgabe · Show: nur das Bild");
    });
    if v.grid.enabled {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Raster").weak());
            ui.add(egui::DragValue::new(&mut v.grid.cols).range(1..=64).prefix("Spalten "));
            ui.add(egui::DragValue::new(&mut v.grid.rows).range(1..=64).prefix("Zeilen "));
            ui.add(egui::Slider::new(&mut v.grid.thickness, 1.0..=30.0).step_by(1.0).text("Linienstärke px"));
            ui.color_edit_button_srgb(&mut v.grid.color);
            ui.checkbox(&mut v.grid.diagonals, "Diagonalen");
            ui.checkbox(&mut v.grid.only_selected, "nur gewählte Fläche");
        });
    }
}

pub fn status_bar(ui: &mut Ui, sh: &mut Shared) {
    ui.horizontal(|ui| {
        if let Some((msg, at)) = &sh.ui.message {
            if at.elapsed() < Duration::from_secs(5) {
                ui.label(RichText::new(msg).color(ACCENT));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let path = sh.eng.doc.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "nicht gespeichert".into());
            ui.label(RichText::new(path).weak().small());
        });
    });
}

fn content_label(c: &Content) -> &'static str {
    c.kind().short()
}

pub fn layers_panel(ui: &mut Ui, sh: &mut Shared) {
    ui.horizontal(|ui| {
        ui.heading("Ebenen");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("+ Maske ✂").on_hover_text("Neue Masken-Ebene: schneidet aus allen Ebenen darunter aus").clicked() {
                sh.eng.doc.new_mask(MaskScope::Layer, vec![]);
            }
            if ui.button("+ Fläche").on_hover_text("Neue Fläche (Viereck mit Corner Pin)").clicked() {
                sh.eng.doc.add_surface();
                sh.ui.right_tab = RightTab::Surface;
            }
        });
    });

    let layers = sh.eng.doc.project.layers.clone();
    let view = sh.eng.doc.view.clone();
    let mut number = 0;
    let numbers: Vec<usize> = layers
        .iter()
        .map(|l| {
            if matches!(l, Layer::Surface(_)) {
                number += 1;
                number
            } else {
                0
            }
        })
        .collect();
    let is_selected = |l: &Layer| match l {
        Layer::Surface(s) => view.selected_surface.as_ref() == Some(&s.id),
        Layer::Mask(m) => view.selected_mask.as_ref() == Some(&m.mask.id),
    };

    // Oben in der Liste = vorne im Bild
    for (idx, l) in layers.iter().enumerate().rev() {
        let active = is_selected(l);
        let frame = egui::Frame::new()
            .fill(if active { Color32::from_rgb(0x0b, 0x3b, 0x4a) } else { Color32::TRANSPARENT })
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(4, 2));
        let resp = frame
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    match l {
                        Layer::Surface(s) => {
                            let eye = if s.visible { "👁" } else { "–" };
                            if ui.small_button(eye).on_hover_text(if s.visible { "Ausblenden" } else { "Einblenden" }).clicked() {
                                let id = s.id.clone();
                                sh.eng.doc.update_surface(&id, None, |s| s.visible = !s.visible);
                            }
                            ui.label(RichText::new(format!("{}", numbers[idx])).weak());
                        }
                        Layer::Mask(m) => {
                            let eye = if m.mask.enabled { "👁" } else { "–" };
                            if ui.small_button(eye).on_hover_text(if m.mask.enabled { "Ausschalten" } else { "Einschalten" }).clicked() {
                                let id = m.mask.id.clone();
                                sh.eng.doc.update_mask(&id, None, |m| m.enabled = !m.enabled);
                            }
                            ui.label(RichText::new("✂").color(mode_color(m.mask.mode)));
                        }
                    }
                    // Name (Doppelklick = umbenennen)
                    let renaming = sh.ui.rename.as_ref().is_some_and(|(id, _)| id == l.id());
                    if renaming {
                        let (_, text) = sh.ui.rename.as_mut().unwrap();
                        let r = ui.add(egui::TextEdit::singleline(text).desired_width(120.0));
                        r.request_focus();
                        if r.lost_focus() {
                            let (id, text) = sh.ui.rename.take().unwrap();
                            let name: String = text.trim().chars().take(100).collect();
                            if !name.is_empty() {
                                sh.eng.doc.edit(None, |p| {
                                    for l in &mut p.layers {
                                        match l {
                                            Layer::Surface(s) if s.id == id => s.name = name.clone(),
                                            Layer::Mask(m) if m.mask.id == id => m.mask.name = name.clone(),
                                            _ => {}
                                        }
                                    }
                                });
                            }
                        }
                    } else {
                        let dim = match l {
                            Layer::Surface(s) => !s.visible,
                            Layer::Mask(m) => !m.mask.enabled,
                        };
                        let text = RichText::new(l.name());
                        let r = ui.add(egui::Label::new(if dim { text.weak() } else { text }).sense(egui::Sense::click()).truncate());
                        if r.double_clicked() {
                            sh.ui.rename = Some((l.id().to_string(), l.name().to_string()));
                        } else if r.clicked() {
                            select_layer(sh, l);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let locked = l.locked();
                        if ui.small_button(if locked { "🔒" } else { "🔓" }).on_hover_text(if locked { "Entsperren" } else { "Sperren (gegen versehentliches Verschieben)" }).clicked() {
                            match l {
                                Layer::Surface(s) => {
                                    let id = s.id.clone();
                                    sh.eng.doc.update_surface(&id, None, |s| s.locked = !locked);
                                }
                                Layer::Mask(m) => sh.eng.doc.set_mask_layer_locked(&m.mask.id, !locked),
                            }
                        }
                        let meta = match l {
                            Layer::Surface(s) => {
                                let mut t = content_label(&s.content).to_string();
                                if !s.masks.is_empty() {
                                    t.push_str(&format!(" · {} M.", s.masks.len()));
                                }
                                t
                            }
                            Layer::Mask(m) => if m.mask.mode == MaskMode::Subtract { "schneidet aus" } else { "nur innen" }.into(),
                        };
                        ui.label(RichText::new(meta).weak().small());
                    });
                });
            })
            .response;
        if resp.interact(egui::Sense::click()).clicked() {
            select_layer(sh, l);
        }
    }

    // Aktionen für die gewählte Ebene
    let sel = layers.iter().position(&is_selected);
    if let Some(i) = sel {
        let l = &layers[i];
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(i + 1 < layers.len(), egui::Button::new("↑")).on_hover_text("Eine Ebene nach vorne").clicked() {
                sh.eng.doc.move_layer(l.id(), i + 1);
            }
            if ui.add_enabled(i > 0, egui::Button::new("↓")).on_hover_text("Eine Ebene nach hinten").clicked() {
                sh.eng.doc.move_layer(l.id(), i - 1);
            }
            if let Layer::Surface(s) = l {
                if ui.button("Duplizieren").clicked() {
                    sh.eng.doc.duplicate_surface(&s.id);
                }
            }
            if ui.button(RichText::new("Löschen").color(Color32::from_rgb(0xf8, 0x71, 0x71))).clicked() {
                sh.ui.confirm_delete = Some(l.id().to_string());
            }
        });
    }
    ui.label(
        RichText::new("Oben in der Liste = vorne. Eine Masken-Ebene ✂ schneidet aus allem darunter aus – so lassen sich Flächen aus Flächen ausschneiden. Doppelklick = umbenennen.")
            .weak()
            .small(),
    );
}

fn select_layer(sh: &mut Shared, l: &Layer) {
    match l {
        Layer::Surface(s) => {
            sh.eng.doc.view.tool = if sh.eng.doc.view.tool == Tool::Mask && sh.eng.doc.selected_mask().is_some_and(|m| m.layer.is_some()) {
                Tool::Surface
            } else {
                sh.eng.doc.view.tool
            };
            sh.eng.doc.select_surface(Some(s.id.clone()), None);
            sh.ui.right_tab = RightTab::Surface;
        }
        Layer::Mask(m) => {
            sh.eng.doc.select_mask(&m.mask.id, None);
            sh.ui.right_tab = RightTab::Surface;
        }
    }
}

/// Masken-Stapel der gewählten Fläche (wie in MadMapper: jede Maske fügt hinzu, schneidet aus oder
/// bildet die Schnittmenge – von oben nach unten verrechnet)
pub fn masks_panel(ui: &mut Ui, sh: &mut Shared) {
    let Some(surface) = sh.eng.doc.selected_surface().cloned() else {
        ui.heading("Masken der Fläche");
        ui.label(RichText::new("Fläche wählen, um ihr Masken zu geben. Für Masken über mehrere Flächen: „+ Maske ✂“ bei den Ebenen.").weak().small());
        return;
    };
    ui.horizontal(|ui| {
        ui.heading(format!("Masken · {}", surface.name));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("+ Maske zeichnen").on_hover_text("Dann auf der Bühne oder der Projektion zeichnen").clicked() {
                let v = &mut sh.eng.doc.view;
                v.tool = Tool::Mask;
                v.mask_scope = MaskScope::Surface;
                v.selected_mask = None;
                v.selected_point = None;
            }
        });
    });
    if surface.masks.is_empty() {
        ui.label(
            RichText::new("„+ Maske zeichnen“, dann den Umriss mit gedrückter Maustaste nachfahren oder Punkt für Punkt klicken. Die erste Maske bestimmt, was sichtbar ist; weitere Masken schneiden heraus oder fügen hinzu.")
                .weak()
                .small(),
        );
        return;
    }
    let n = surface.masks.len();
    for (i, m) in surface.masks.iter().enumerate() {
        let active = sh.eng.doc.view.selected_mask.as_ref() == Some(&m.id);
        let frame = egui::Frame::new()
            .fill(if active { Color32::from_rgb(0x0b, 0x3b, 0x4a) } else { Color32::TRANSPARENT })
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(4, 2));
        let resp = frame
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut en = m.enabled;
                    if ui.checkbox(&mut en, "").on_hover_text("Aktiv").changed() {
                        sh.eng.doc.update_mask(&m.id, None, |m| m.enabled = en);
                    }
                    if ui.add(egui::Label::new(&m.name).sense(egui::Sense::click()).truncate()).clicked() {
                        sh.eng.doc.select_mask(&m.id, None);
                    }
                    ui.label(RichText::new(m.mode.label().split(' ').next().unwrap_or("")).color(mode_color(m.mode)));
                    ui.label(RichText::new(format!("{} P.", m.points.len())).weak().small());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(i + 1 < n, egui::Button::new("↓").small()).on_hover_text("Nach unten").clicked() {
                            sh.eng.doc.move_mask(&m.id, i + 1);
                        }
                        if ui.add_enabled(i > 0, egui::Button::new("↑").small()).on_hover_text("Nach oben (früher verrechnen)").clicked() {
                            sh.eng.doc.move_mask(&m.id, i - 1);
                        }
                    });
                });
            })
            .response;
        if resp.interact(egui::Sense::click()).clicked() {
            sh.eng.doc.select_mask(&m.id, None);
        }
    }
    if let Some(sel) = sh.eng.doc.selected_mask().filter(|m| m.surface.is_some()).map(|m| m.mask.clone()) {
        ui.add_space(4.0);
        super::props::mask_props(ui, sh, &sel, false);
    }
}

/// Bestätigungen und Hilfe
pub fn dialogs(ctx: &egui::Context, sh: &mut Shared) {
    if let Some(id) = sh.ui.confirm_delete.clone() {
        let name = sh.eng.doc.project.layers.iter().find(|l| l.id() == id).map(|l| l.name().to_string()).unwrap_or_default();
        egui::Window::new("Löschen?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label(format!("„{name}“ löschen? (Rückgängig mit ⌘Z)"));
            ui.horizontal(|ui| {
                if ui.button("Löschen").clicked() {
                    sh.eng.doc.remove_layer(&id);
                    sh.ui.confirm_delete = None;
                }
                if ui.button("Abbrechen").clicked() {
                    sh.ui.confirm_delete = None;
                }
            });
        });
    }
    if sh.ui.confirm_new {
        egui::Window::new("Neues Projekt?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            if sh.eng.doc.dirty {
                ui.label("Ungespeicherte Änderungen gehen verloren.");
            }
            ui.horizontal(|ui| {
                if ui.button("Neues Projekt").clicked() {
                    sh.eng.new_project();
                    sh.ui.confirm_new = false;
                }
                if ui.button("Abbrechen").clicked() {
                    sh.ui.confirm_new = false;
                }
            });
        });
    }
    if sh.ui.show_help {
        let mut open = true;
        egui::Window::new("Hilfe").open(&mut open).default_width(460.0).show(ctx, |ui| {
            egui::Grid::new("keys").striped(true).show(ui, |ui| {
                for (k, d) in [
                    ("E", "Bearbeiten ↔ Show (Umrisse auf der Ausgabe ein/aus)"),
                    ("F", "Ausgabe öffnen / Vollbild ein/aus"),
                    ("V / M", "Werkzeug Fläche / Maske"),
                    ("S", "Snap ein/aus (⌥ beim Ziehen: ohne Snap)"),
                    ("G / ⇧G", "Raster auf den Flächen / Testgitter über das ganze Bild"),
                    ("B", "Blackout"),
                    ("T", "Tap-Tempo (Beat von Hand)"),
                    ("Pfeile", "Ecke/Fläche/Punkt um 1 px verschieben (mit ⇧ 10 px)"),
                    ("Tab", "nächste Ecke bzw. nächster Maskenpunkt"),
                    ("Entf", "Maskenpunkt löschen"),
                    ("Enter / Esc", "Maske fertig / Auswahl aufheben"),
                    ("⌘Z / ⇧⌘Z", "Rückgängig / Wiederholen"),
                    ("⌘S / ⌘O", "Speichern / Öffnen"),
                    ("Pinch / ⌥+Scrollen", "Gewählte Fläche skalieren / drehen"),
                ] {
                    ui.label(RichText::new(k).strong());
                    ui.label(d);
                    ui.end_row();
                }
            });
            ui.separator();
            ui.label("Ausgabefenster auf den Beamer ziehen und F drücken. Beim nächsten Start erscheint es dort automatisch wieder.");
            ui.label("Dateien (Bilder, Videos, Musik) lassen sich einfach ins Fenster ziehen.");
            ui.label(format!("Daten: {}", sh.eng.data_dir.display()));
        });
        if !open {
            sh.ui.show_help = false;
        }
    }
}
