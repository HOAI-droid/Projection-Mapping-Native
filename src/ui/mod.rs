// Bedienoberfläche (egui): Steuerfenster auf dem Mac-Bildschirm und Ausgabefenster für den Beamer.
//
// Beide Fenster teilen sich eine Engine. Das Ausgabefenster ist ein eigenes Betriebssystem-Fenster
// (eigener Bildtakt) – auf den Beamer ziehen und mit F in den Vollbildmodus schalten; Position und
// Vollbild merkt sich die App für den nächsten Start.

mod audio_panel;
pub mod editor;
mod media_panel;
mod panels;
mod props;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use egui::{Color32, Key, Modifiers, Rect, Ui, ViewportBuilder, ViewportCommand, ViewportId, vec2};
use parking_lot::Mutex;

use crate::engine::Engine;
use crate::render::ViewTarget;
use crate::state::{TestPattern, Tool};
use editor::EditorState;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
    Surface,
    Audio,
    Media,
}

pub struct UiState {
    pub right_tab: RightTab,
    pub effect_category: Option<String>,
    /// Ebene, die gerade umbenannt wird (ID, Text)
    pub rename: Option<(String, String)>,
    pub preset_search: String,
    pub thumbs: HashMap<String, egui::TextureHandle>,
    pub stage_editor: EditorState,
    pub output_editor: EditorState,
    /// kurze Meldung unten (Text, seit)
    pub message: Option<(String, Instant)>,
    pub confirm_delete: Option<String>,
    pub output_open: bool,
    pub output_fullscreen: bool,
    pub show_help: bool,
    pub confirm_new: bool,
}

impl UiState {
    pub fn notify(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), Instant::now()));
    }
}

pub struct Shared {
    pub eng: Engine,
    pub ui: UiState,
}

pub const OUTPUT_ID: &str = "ausgabe";

pub fn output_viewport() -> ViewportId {
    ViewportId::from_hash_of(OUTPUT_ID)
}

pub struct App {
    shared: Arc<Mutex<Shared>>,
    rs: egui_wgpu::RenderState,
    exiting: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let rs = cc.wgpu_render_state.clone().expect("wgpu wird benötigt");
        let data_dir = data_dir();
        let eng = Engine::new(rs.device.clone(), rs.queue.clone(), data_dir);
        let output_open = eng.settings.output_open || !eng.settings.first_run_done;
        let output_fullscreen = eng.settings.output_fullscreen;
        setup_style(&cc.egui_ctx);
        let shared = Shared {
            eng,
            ui: UiState {
                right_tab: RightTab::Surface,
                effect_category: None,
                rename: None,
                preset_search: String::new(),
                thumbs: HashMap::new(),
                stage_editor: EditorState::default(),
                output_editor: EditorState::default(),
                message: None,
                confirm_delete: None,
                output_open,
                output_fullscreen,
                show_help: false,
                confirm_new: false,
            },
        };
        let mut app = Self { shared: Arc::new(Mutex::new(shared)), rs, exiting: false };
        {
            let mut sh = app.shared.lock();
            sh.eng.settings.first_run_done = true;
            sh.eng.settings.output_open = output_open;
            if let Some(name) = sh.eng.settings.audio_input.clone() {
                // zuletzt benutzten Audio-Eingang wieder öffnen
                sh.eng.audio.use_input(&name);
            }
        }
        app.exiting = false;
        app
    }
}

pub fn data_dir() -> std::path::PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("Projection Mapping")
}

/// Systemschriften als Ersatz für Symbole, die in den mitgelieferten Schriften fehlen (Pfeile, Formen …)
fn setup_fonts(ctx: &egui::Context) {
    let candidates = [
        "/System/Library/Fonts/Apple Symbols.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/Library/Fonts/Arial Unicode.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    let mut fonts = egui::FontDefinitions::default();
    for (i, path) in candidates.iter().enumerate() {
        let Ok(data) = std::fs::read(path) else { continue };
        let name = format!("fallback-{i}");
        fonts.font_data.insert(name.clone(), Arc::new(egui::FontData::from_owned(data)));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(name.clone());
        }
    }
    ctx.set_fonts(fonts);
}

fn setup_style(ctx: &egui::Context) {
    setup_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = vec2(6.0, 6.0);
        s.spacing.button_padding = vec2(8.0, 4.0);
        s.spacing.slider_width = 170.0;
        s.visuals.selection.bg_fill = Color32::from_rgb(0x0e, 0x74, 0x90);
        s.visuals.hyperlink_color = editor::ACCENT;
    });
}

/// egui-Textur für ein Zielbild anlegen oder auf das neue Bild umstellen
fn sync_texture(rs: &egui_wgpu::RenderState, target: &mut ViewTarget, fresh: bool) -> egui::TextureId {
    let mut r = rs.renderer.write();
    match target.egui_id {
        Some(id) => {
            if fresh {
                r.update_egui_texture_from_wgpu_texture(&rs.device, &target.srgb_view, wgpu::FilterMode::Linear, id);
            }
            id
        }
        None => {
            let id = r.register_native_texture(&rs.device, &target.srgb_view, wgpu::FilterMode::Linear);
            target.egui_id = Some(id);
            id
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let shared = self.shared.clone();
        let mut guard = shared.lock();
        let sh = &mut *guard;
        sh.eng.tick();

        // Dateien ins Fenster gezogen: Medien importieren (Musik → Player)
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            import_paths(sh, dropped);
        }

        handle_keys(&ctx, sh, false);

        egui::Panel::top("topbar").show(ui, |ui| panels::top_bar(ui, sh));
        egui::Panel::top("toolbar").show(ui, |ui| panels::toolbar(ui, sh));
        egui::Panel::bottom("statusbar").show(ui, |ui| panels::status_bar(ui, sh));
        egui::Panel::left("layers").default_size(270.0).size_range(200.0..=420.0).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("left").show(ui, |ui| {
                panels::layers_panel(ui, sh);
                ui.separator();
                panels::masks_panel(ui, sh);
            });
        });
        egui::Panel::right("props").default_size(340.0).size_range(280.0..=520.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut sh.ui.right_tab, RightTab::Surface, "Fläche");
                ui.selectable_value(&mut sh.ui.right_tab, RightTab::Audio, "Audio ♪");
                ui.selectable_value(&mut sh.ui.right_tab, RightTab::Media, "Medien");
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("right").show(ui, |ui| match sh.ui.right_tab {
                RightTab::Surface => props::properties(ui, sh),
                RightTab::Audio => audio_panel::audio_panel(ui, sh),
                RightTab::Media => media_panel::media_panel(ui, sh),
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::from_gray(18))).show(ui, |ui| self.stage(ui, sh));

        panels::dialogs(&ctx, sh);

        if sh.ui.output_open {
            self.show_output(&ctx, sh);
        }

        // Beenden: alles sichern
        if ctx.input(|i| i.viewport().close_requested()) && !self.exiting {
            self.exiting = true;
            sh.eng.settings.output_open = sh.ui.output_open;
            sh.eng.autosave(true);
            sh.eng.save_settings();
        }
        ctx.request_repaint();
    }
}

impl App {
    /// Live-Vorschau mit Bearbeitungsebene (Seitenverhältnis der Ausgabe)
    fn stage(&mut self, ui: &mut Ui, sh: &mut Shared) {
        let avail = ui.available_rect_before_wrap().shrink(12.0);
        let (rw, rh) = (sh.eng.ref_size.0.max(1) as f32, sh.eng.ref_size.1.max(1) as f32);
        let scale = (avail.width() / rw).min(avail.height() / rh);
        let size = vec2(rw * scale, rh * scale);
        let rect = Rect::from_center_size(avail.center(), size);
        let ppp = ui.ctx().pixels_per_point();
        let fresh = sh.eng.render_stage((size.x * ppp).round() as u32, (size.y * ppp).round() as u32);
        if let Some(t) = sh.eng.stage.as_mut() {
            let id = sync_texture(&self.rs, t, fresh);
            ui.painter().image(id, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, Color32::from_gray(60)), egui::StrokeKind::Outside);
        if sh.eng.doc.view.test_pattern == TestPattern::Grid {
            editor::draw_test_grid(ui.painter(), rect, sh.eng.ref_size.0, sh.eng.ref_size.1);
        }
        let Shared { eng, ui: st } = sh;
        editor::show(ui, rect, eng, &mut st.stage_editor, false);
        ui.advance_cursor_after_rect(rect);
    }

    /// Ausgabefenster (Beamer)
    fn show_output(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        let s = &sh.eng.settings;
        let mut builder = ViewportBuilder::default()
            .with_title("Projection Mapping – Ausgabe")
            .with_inner_size(s.output_size.unwrap_or([960.0, 540.0]))
            .with_min_inner_size([160.0, 90.0]);
        if let Some(p) = s.output_pos {
            builder = builder.with_position(p);
        }
        if sh.ui.output_fullscreen {
            builder = builder.with_fullscreen(true);
        }
        let shared = self.shared.clone();
        let rs = self.rs.clone();
        ctx.show_viewport_deferred(output_viewport(), builder, move |ui, _class| {
            let mut guard = shared.lock();
            output_ui(ui, &mut guard, &rs);
        });
    }
}

fn output_ui(ui: &mut Ui, sh: &mut Shared, rs: &egui_wgpu::RenderState) {
    let ctx = ui.ctx().clone();
    sh.eng.tick();
    let (inner, outer, fullscreen, close) = ctx.input(|i| {
        let v = i.viewport();
        (v.inner_rect, v.outer_rect, v.fullscreen.unwrap_or(false), v.close_requested())
    });
    if close {
        sh.ui.output_open = false;
        sh.eng.settings.output_open = false;
        sh.eng.save_settings();
        return;
    }
    // Fensterlage merken (für den nächsten Start)
    sh.ui.output_fullscreen = fullscreen;
    sh.eng.settings.output_fullscreen = fullscreen;
    if let Some(o) = outer {
        if !fullscreen {
            sh.eng.settings.output_pos = Some([o.min.x, o.min.y]);
        }
    }
    if let Some(r) = inner {
        if !fullscreen {
            sh.eng.settings.output_size = Some([r.width(), r.height()]);
        }
    }

    handle_keys(&ctx, sh, true);

    let rect = ui.max_rect();
    let ppp = ctx.pixels_per_point();
    let (pw, ph) = ((rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32);
    // Die Ausgabe bestimmt die Bezugsgröße (Seitenverhältnisse, Feather, Rasterlinien)
    sh.eng.ref_size = (pw, ph);
    let fresh = sh.eng.render_output(pw, ph);
    if let Some(t) = sh.eng.output.as_mut() {
        let id = sync_texture(rs, t, fresh);
        ui.painter().image(id, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
    if sh.eng.doc.view.test_pattern == TestPattern::Grid {
        editor::draw_test_grid(ui.painter(), rect, pw, ph);
    }
    let edit = sh.eng.doc.view.edit_mode;
    if edit {
        let Shared { eng, ui: st } = sh;
        editor::show(ui, rect, eng, &mut st.output_editor, true);
        // Hinweisleiste unten
        let text = if fullscreen {
            "E Show · V Fläche · M Maske · S Snap · G Raster · B Blackout · ⌘Z".to_string()
        } else {
            "Auf den Beamer ziehen, dann F = Vollbild  ·  E Show · V Fläche · M Maske · S Snap · G Raster · B Blackout".to_string()
        };
        let font = egui::FontId::proportional(15.0);
        let galley = ui.painter().layout_no_wrap(text, font, Color32::WHITE);
        let pos = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.max.y - galley.size().y - 14.0);
        ui.painter().rect_filled(Rect::from_min_size(pos, galley.size()).expand(6.0), 6.0, Color32::from_black_alpha(190));
        ui.painter().galley(pos, galley, Color32::WHITE);
    } else if ui.rect_contains_pointer(rect) {
        // Show-Modus: kein Mauszeiger auf der Projektion
        ctx.set_cursor_icon(egui::CursorIcon::None);
    }
    ctx.request_repaint();
}

/// Medien oder Musik importieren
pub fn import_paths(sh: &mut Shared, paths: Vec<std::path::PathBuf>) {
    let (audio, media): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| {
        p.extension().is_some_and(|e| ["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "aiff", "aif"].contains(&e.to_string_lossy().to_lowercase().as_str()))
    });
    if !audio.is_empty() {
        sh.eng.audio.add_tracks(audio);
        sh.ui.right_tab = RightTab::Audio;
    }
    let media: Vec<_> = media.into_iter().filter(|p| crate::media::kind_of(p).is_some()).collect();
    if !media.is_empty() {
        let n = media.len();
        sh.eng.media.import(media);
        sh.ui.right_tab = RightTab::Media;
        sh.ui.notify(format!("{n} Datei(en) werden importiert …"));
    }
}

fn key(ctx: &egui::Context, mods: Modifiers, k: Key) -> bool {
    ctx.input_mut(|i| i.consume_key(mods, k))
}

/// Tastenkürzel (Steuerung und Ausgabe)
pub fn handle_keys(ctx: &egui::Context, sh: &mut Shared, on_output: bool) {
    // In Textfeldern gehören alle Tasten dem Feld
    if ctx.text_edit_focused() {
        return;
    }
    let cmd = Modifiers::COMMAND;
    let shift = Modifiers::SHIFT;
    if key(ctx, cmd | shift, Key::Z) || key(ctx, cmd, Key::Y) {
        sh.eng.doc.redo();
    }
    if key(ctx, cmd, Key::Z) {
        sh.eng.doc.undo();
    }
    if key(ctx, cmd | shift, Key::S) {
        panels::save_as(sh);
    }
    if key(ctx, cmd, Key::S) {
        panels::save(sh);
    }
    if key(ctx, cmd, Key::O) {
        panels::open(sh);
    }

    let (w, h) = (sh.eng.ref_size.0 as f64, sh.eng.ref_size.1 as f64);
    // Pfeiltasten: 1 px, mit ⇧ 10 px (Schieberegler mit Fokus behalten ihre Pfeiltasten)
    let slider_focus = ctx.memory(|m| m.focused().is_some());
    if !slider_focus {
        for (k, dx, dy) in [(Key::ArrowLeft, -1.0, 0.0), (Key::ArrowRight, 1.0, 0.0), (Key::ArrowUp, 0.0, -1.0), (Key::ArrowDown, 0.0, 1.0)] {
            for (mods, step) in [(shift, 10.0), (Modifiers::NONE, 1.0)] {
                if key(ctx, mods, k) {
                    let g = sh.eng.doc.repeat_gesture("arrows", 600);
                    sh.eng.doc.nudge(dx * step, dy * step, w, h, Some(&g));
                }
            }
        }
    }

    let tab_back = key(ctx, shift, Key::Tab);
    let tab = key(ctx, Modifiers::NONE, Key::Tab);
    if tab || tab_back {
        let dir: isize = if tab_back { -1 } else { 1 };
        let doc = &mut sh.eng.doc;
        if doc.view.tool == Tool::Mask {
            if let Some(n) = doc.selected_mask().map(|m| m.mask.points.len()).filter(|n| *n > 0) {
                let i = doc.view.selected_point.map_or(0, |i| (i as isize + dir).rem_euclid(n as isize) as usize);
                doc.view.selected_point = Some(i);
            }
        } else if let Some(s) = doc.selected_surface().or_else(|| doc.project.surfaces().next()).cloned() {
            let n = s.shape.handle_count() as isize;
            let i = doc.view.selected_corner.map_or(0, |i| (i as isize + dir).rem_euclid(n) as usize);
            doc.select_surface(Some(s.id), Some(i));
        }
    }
    if key(ctx, Modifiers::NONE, Key::Delete) || key(ctx, Modifiers::NONE, Key::Backspace) {
        if sh.eng.doc.view.tool == Tool::Mask {
            sh.eng.doc.delete_selected_point();
        }
    }
    if key(ctx, Modifiers::NONE, Key::Enter) && sh.eng.doc.view.tool == Tool::Mask {
        // Maske fertig: nächster Klick beginnt eine neue
        sh.eng.doc.view.selected_mask = None;
        sh.eng.doc.view.selected_point = None;
    }
    if key(ctx, Modifiers::NONE, Key::Escape) {
        let v = &mut sh.eng.doc.view;
        if v.tool == Tool::Mask {
            if v.selected_point.is_some() {
                v.selected_point = None;
            } else {
                v.selected_mask = None;
            }
        } else if v.selected_corner.is_some() {
            v.selected_corner = None;
        } else {
            v.selected_surface = None;
        }
    }
    let plain = |k| key(ctx, Modifiers::NONE, k);
    if plain(Key::E) {
        sh.eng.doc.view.edit_mode = !sh.eng.doc.view.edit_mode;
    }
    if plain(Key::V) {
        sh.eng.doc.view.tool = Tool::Surface;
    }
    if plain(Key::M) {
        sh.eng.doc.view.tool = Tool::Mask;
    }
    if key(ctx, shift, Key::G) {
        let v = &mut sh.eng.doc.view;
        v.test_pattern = if v.test_pattern == TestPattern::Grid { TestPattern::None } else { TestPattern::Grid };
    }
    if plain(Key::G) {
        sh.eng.doc.view.grid.enabled = !sh.eng.doc.view.grid.enabled;
    }
    if plain(Key::S) {
        sh.eng.doc.view.snap = !sh.eng.doc.view.snap;
    }
    if plain(Key::B) {
        sh.eng.doc.show.blackout = !sh.eng.doc.show.blackout;
    }
    if plain(Key::T) {
        sh.eng.audio.tap();
    }
    if plain(Key::F) {
        if !sh.ui.output_open {
            sh.ui.output_open = true;
        } else {
            let fs = !sh.ui.output_fullscreen;
            if on_output {
                ctx.send_viewport_cmd(ViewportCommand::Fullscreen(fs));
            } else {
                ctx.send_viewport_cmd_to(output_viewport(), ViewportCommand::Fullscreen(fs));
            }
            sh.ui.output_fullscreen = fs;
        }
    }
}
