// Projection Mapping – native macOS-App (Rust, wgpu/Metal, egui).
//
// Steuerfenster auf dem Mac, Ausgabefenster auf dem Beamer. Flächen (Viereck, Kreis, Dreieck,
// Verzerrungsgitter), Masken auf Masken, 41 Effekte, Bilder, Videos, Live-Kamera,
// Audio-Reaktivität und Milkdrop über projectM.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod effects;
mod engine;
mod geometry;
mod media;
mod milkdrop;
mod model;
mod render;
mod snap;
mod state;
mod ui;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,projection_mapping=info")).init();

    // Bildschirm während der Show nicht abschalten lassen (macOS)
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/bin/caffeinate").args(["-dims", "-w", &std::process::id().to_string()]).spawn();
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Projection Mapping")
            .with_app_id("projection-mapping")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native("Projection Mapping", options, Box::new(|cc| Ok(Box::new(ui::App::new(cc)))))
}
