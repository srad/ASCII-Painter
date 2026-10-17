#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod settings_ui;
mod theme;
mod viewer;
mod worker;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        // No OS title bar: the app draws its own (see `title_bar` in app.rs). The window stays opaque
        // and square; on Windows 11 the compositor still rounds and outlines the corners of the
        // frameless window, anti-aliased.
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("ASCII Painter")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([900.0, 600.0])
            .with_drag_and_drop(true)
            .with_decorations(false),
        ..Default::default()
    };
    let initial_file = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    eframe::run_native(
        "ASCII Painter",
        options,
        Box::new(|cc| Ok(Box::new(app::AsciiPainterApp::new(cc, initial_file)))),
    )
}
