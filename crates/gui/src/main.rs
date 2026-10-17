#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod settings_ui;
mod viewer;
mod worker;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("ASCII Painter")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    let initial_file = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    eframe::run_native(
        "ASCII Painter",
        options,
        Box::new(|cc| Ok(Box::new(app::AsciiPainterApp::new(cc, initial_file)))),
    )
}
