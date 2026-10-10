// Audio-Quelle wählen (Eingang oder eigener Player), Pegel und Spektrum ansehen, Beat/Tempo.

use egui::{Color32, RichText, Ui, vec2};

use super::Shared;
use super::editor::ACCENT;
use crate::audio::SourceKind;

fn meter(ui: &mut Ui, label: &str, v: f32, color: Color32) {
    ui.horizontal(|ui| {
        ui.add_sized([52.0, 14.0], egui::Label::new(RichText::new(label).small()));
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(220.0), 10.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 3.0, Color32::from_gray(40));
        let mut r = rect;
        r.set_width(rect.width() * v.clamp(0.0, 1.0));
        ui.painter().rect_filled(r, 3.0, color);
    });
}

fn time(t: f64) -> String {
    let t = t.max(0.0) as u64;
    format!("{}:{:02}", t / 60, t % 60)
}

pub fn audio_panel(ui: &mut Ui, sh: &mut Shared) {
    ui.heading("Audio");
    ui.label(RichText::new("Effekte mit ♪, Milkdrop und gekoppelte Regler reagieren auf die gewählte Quelle.").weak().small());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.selectable_label(sh.eng.audio.source == SourceKind::None, "Aus").clicked() {
            sh.eng.audio.stop();
            sh.eng.settings.audio_input = None;
        }
        if ui.selectable_label(sh.eng.audio.source == SourceKind::Input, "🎤 Eingang").on_hover_text("Mikrofon, Audio-Interface oder BlackHole/Loopback").clicked() {
            let name = sh.eng.settings.audio_input.clone().unwrap_or_default();
            sh.eng.audio.use_input(&name);
            if sh.eng.audio.error.is_none() {
                sh.eng.settings.audio_input = Some(name);
            }
        }
        if ui.selectable_label(sh.eng.audio.source == SourceKind::Player, "▶ Player").on_hover_text("Eigene Musikdateien abspielen").clicked() {
            if sh.eng.audio.player.tracks.is_empty() {
                pick_tracks(sh);
            } else {
                let i = sh.eng.audio.player.index;
                sh.eng.audio.play_track(i);
            }
        }
    });
    if let Some(e) = &sh.eng.audio.error {
        ui.colored_label(Color32::from_rgb(0xf8, 0x71, 0x71), e);
    }

    // Eingang
    if sh.eng.audio.source == SourceKind::Input {
        ui.horizontal(|ui| {
            let cur = sh.eng.audio.input_name.clone();
            let label = if cur.is_empty() { "Standard-Eingang".to_string() } else { cur.clone() };
            let mut chosen = None;
            egui::ComboBox::from_id_salt("audio-in").selected_text(label).width(230.0).show_ui(ui, |ui| {
                if ui.selectable_label(cur.is_empty(), "Standard-Eingang").clicked() {
                    chosen = Some(String::new());
                }
                for n in sh.eng.audio.inputs.clone() {
                    if ui.selectable_label(n == cur, &n).clicked() {
                        chosen = Some(n);
                    }
                }
            });
            if ui.button("⟳").on_hover_text("Geräte neu suchen").clicked() {
                sh.eng.audio.refresh_inputs();
            }
            if let Some(n) = chosen {
                sh.eng.audio.use_input(&n);
                sh.eng.settings.audio_input = Some(n);
            }
        });
        ui.label(
            RichText::new("Ton vom Mac selbst (Spotify, Apple Music, YouTube …): das kostenlose „BlackHole 2ch“ installieren (brew install blackhole-2ch), in der Audio-MIDI-Konfiguration ein Multi-Ausgabegerät aus Lautsprechern + BlackHole anlegen und hier „BlackHole 2ch“ wählen.")
                .weak()
                .small(),
        );
    }

    // Player
    if sh.eng.audio.source == SourceKind::Player || !sh.eng.audio.player.tracks.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new("Player").strong());
        let playing = sh.eng.audio.player.playing() && sh.eng.audio.source == SourceKind::Player;
        ui.horizontal(|ui| {
            if ui.button("⏮").clicked() {
                sh.eng.audio.prev();
            }
            if ui.button(if playing { "❚❚" } else { "▶" }).clicked() {
                sh.eng.audio.toggle_play();
            }
            if ui.button("⏭").clicked() {
                sh.eng.audio.next();
            }
            if ui.button("+ Musik …").clicked() {
                pick_tracks(sh);
            }
        });
        let p = &sh.eng.audio.player;
        if sh.eng.audio.source == SourceKind::Player && p.duration > 0.0 {
            let mut pos = p.position() as f32;
            let dur = p.duration as f32;
            ui.horizontal(|ui| {
                ui.label(RichText::new(time(pos as f64)).monospace().small());
                let r = ui.add(egui::Slider::new(&mut pos, 0.0..=dur).show_value(false));
                ui.label(RichText::new(time(dur as f64)).monospace().small());
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    sh.eng.audio.seek(pos as f64);
                }
            });
        }
        let mut play = None;
        let mut remove = None;
        let index = sh.eng.audio.player.index;
        for i in 0..sh.eng.audio.player.tracks.len() {
            ui.horizontal(|ui| {
                let name = sh.eng.audio.player.track_name(i);
                let current = i == index && sh.eng.audio.source == SourceKind::Player;
                if ui.selectable_label(current, name).clicked() {
                    play = Some(i);
                }
                if ui.small_button("✕").on_hover_text("Aus der Liste entfernen").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = play {
            sh.eng.audio.play_track(i);
        }
        if let Some(i) = remove {
            sh.eng.audio.remove_track(i);
        }
    }

    // Analyse
    ui.add_space(8.0);
    ui.label(RichText::new("Analyse").strong());
    let f = sh.eng.audio.frame.clone();
    meter(ui, "Pegel", f.level, Color32::from_gray(200));
    meter(ui, "Bass", f.bass, Color32::from_rgb(0xf8, 0x71, 0x71));
    meter(ui, "Mitten", f.mid, Color32::from_rgb(0xfa, 0xcc, 0x15));
    meter(ui, "Höhen", f.high, Color32::from_rgb(0x60, 0xa5, 0xfa));
    meter(ui, "Beat", f.beat, ACCENT);
    // Spektrum
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(300.0), 60.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 3.0, Color32::from_gray(20));
    let n = f.spectrum.len();
    let bw = rect.width() / n as f32;
    for i in 0..n {
        let h = f.spectrum[i] * rect.height();
        let x = rect.min.x + i as f32 * bw;
        ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(x + 0.5, rect.max.y - h), egui::pos2(x + bw - 0.5, rect.max.y)), 0.0, egui::Color32::from_rgb(0x4a, 0xde, 0x80));
        let py = rect.max.y - f.peaks[i] * rect.height();
        ui.painter().line_segment([egui::pos2(x, py), egui::pos2(x + bw, py)], egui::Stroke::new(1.0, Color32::WHITE));
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(if f.bpm > 0.0 { format!("{:.0} BPM", f.bpm) } else { "– BPM".into() }).strong());
        ui.label(RichText::new(format!("Beat #{}", f.beat_count)).weak());
        if ui.button("Tap").on_hover_text("Im Takt klicken (oder Taste T): Beat von Hand").clicked() {
            sh.eng.audio.tap();
        }
    });
    let an = &mut sh.eng.audio.analyzer;
    ui.add(egui::Slider::new(&mut an.gain, 0.2..=4.0).text("Verstärkung"));
    ui.add(egui::Slider::new(&mut an.sensitivity, 1.1..=2.0).text("Beat-Schwelle")).on_hover_text("Klein = empfindlich, groß = nur deutliche Schläge");
}

fn pick_tracks(sh: &mut Shared) {
    if let Some(files) = rfd::FileDialog::new().add_filter("Musik", &["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "aiff", "aif"]).pick_files() {
        sh.eng.audio.add_tracks(files);
    }
}
