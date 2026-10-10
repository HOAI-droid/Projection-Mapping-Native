// Medienbibliothek: Bilder und Videos importieren (auch per Drag & Drop), Vorschau, löschen.

use egui::{Color32, RichText, Ui};

use super::Shared;
use crate::media::{ffmpeg, MediaKind};
use crate::model::{Content, Layer};

/// Vorschaubild als egui-Textur (wird beim ersten Aufruf angelegt)
pub fn thumb(ctx: &egui::Context, sh: &mut Shared, id: &str) -> Option<egui::TextureHandle> {
    if let Some(t) = sh.ui.thumbs.get(id) {
        return Some(t.clone());
    }
    let (w, h, data) = sh.eng.media.thumbs.get(id)?;
    let img = egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], data);
    let t = ctx.load_texture(format!("thumb-{id}"), img, egui::TextureOptions::LINEAR);
    sh.ui.thumbs.insert(id.to_string(), t.clone());
    Some(t)
}

pub fn media_panel(ui: &mut Ui, sh: &mut Shared) {
    ui.heading("Medien");
    if ffmpeg().is_none() {
        ui.colored_label(Color32::from_rgb(0xfa, 0xcc, 0x15), "⚠ ffmpeg fehlt – für Videos, Kamera und den Musik-Player einmalig im Terminal:");
        ui.code("brew install ffmpeg");
    }
    ui.horizontal(|ui| {
        if ui.button("+ Bilder/Videos importieren …").clicked() {
            if let Some(files) = rfd::FileDialog::new()
                .add_filter("Bilder und Videos", &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "mp4", "mov", "m4v", "webm", "mkv", "avi", "mpg", "mpeg"])
                .pick_files()
            {
                super::import_paths(sh, files);
            }
        }
    });
    ui.label(RichText::new("Oder Dateien einfach ins Fenster ziehen. Die Dateien werden in den App-Ordner kopiert.").weak().small());

    // laufende/fehlgeschlagene Importe
    let mut clear = false;
    for j in &sh.eng.media.jobs {
        match &j.error {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&j.name);
                });
            }
            Some(e) => {
                ui.colored_label(Color32::from_rgb(0xf8, 0x71, 0x71), format!("{}: {e}", j.name));
                clear = true;
            }
        }
    }
    if clear && ui.small_button("Fehler ausblenden").clicked() {
        sh.eng.media.clear_errors();
    }

    // Welche Medien sind in Benutzung?
    let used: std::collections::HashSet<String> = sh
        .eng
        .doc
        .project
        .layers
        .iter()
        .filter_map(|l| match l {
            Layer::Surface(s) => match &s.content {
                Content::Image { media_id } | Content::Video { media_id, .. } => Some(media_id.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();

    let selected = sh.eng.doc.selected_surface().map(|s| s.id.clone());
    for kind in [MediaKind::Image, MediaKind::Video] {
        let items: Vec<_> = sh.eng.media.items.iter().filter(|m| m.kind == kind).cloned().collect();
        ui.add_space(6.0);
        ui.label(RichText::new(if kind == MediaKind::Image { format!("Bilder ({})", items.len()) } else { format!("Videos ({})", items.len()) }).strong());
        if items.is_empty() {
            continue;
        }
        let mut remove = None;
        egui::Grid::new(("media", kind as u8)).num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            for m in items {
                match thumb(ui.ctx(), sh, &m.id) {
                    Some(t) => {
                        let size = t.size_vec2();
                        let scale = 72.0 / size.x.max(size.y).max(1.0);
                        ui.image((t.id(), size * scale));
                    }
                    None => {
                        ui.spinner();
                    }
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(&m.name).strong());
                    let mut info = format!("{} × {}", m.width, m.height);
                    if kind == MediaKind::Video {
                        info.push_str(&format!(" · {:.1} s · {:.0} fps", m.duration, m.fps));
                    }
                    ui.label(RichText::new(info).weak().small());
                    ui.horizontal(|ui| {
                        if let Some(sid) = &selected {
                            if ui.small_button("Auf gewählte Fläche").clicked() {
                                let id = m.id.clone();
                                sh.eng.doc.update_surface(sid, None, |s| {
                                    s.content = if kind == MediaKind::Image {
                                        Content::Image { media_id: id }
                                    } else {
                                        Content::Video { media_id: id, r#loop: true, speed: 1.0, playing: true }
                                    }
                                });
                            }
                        }
                        let in_use = used.contains(&m.id);
                        if ui
                            .add_enabled(!in_use, egui::Button::new("Löschen").small())
                            .on_disabled_hover_text("Wird von einer Fläche benutzt")
                            .clicked()
                        {
                            remove = Some(m.id.clone());
                        }
                    });
                });
                ui.end_row();
            }
        });
        if let Some(id) = remove {
            sh.eng.media.remove(&id);
            sh.ui.thumbs.remove(&id);
        }
    }
}
