#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod compositor;
mod controls;
mod export;
mod jobs;
mod preferences;
mod scenes;
mod settings_ui;
#[cfg(test)]
mod tests;
mod theme;
mod ui;
mod viewer;
mod worker;

use std::sync::{Arc, Mutex};

fn run() -> Result<(), String> {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    let initial_file = if args.first().is_some_and(|arg| arg == "--") {
        args.remove(0);
        (!args.is_empty()).then(|| std::path::PathBuf::from(args.remove(0)))
    } else if args.first().is_some_and(|arg| !arg.to_string_lossy().starts_with('-')) {
        Some(std::path::PathBuf::from(args.remove(0)))
    } else {
        None
    };
    let argv = args
        .into_iter()
        .map(|a| a.into_string().map_err(|_| "Rune tooling arguments must be valid Unicode".to_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    let headless = rune::tooling::Args::parse_from(argv.clone()).headless();
    let mut state = if headless { app::State::default() } else { app::State::native(initial_file.clone()) };
    if headless && initial_file.is_some() {
        state.initial_file = initial_file;
        state.services_enabled = true;
        state.locate_ffmpeg();
    }
    let window = state.prefs.window;
    let fatal = Arc::new(Mutex::new(None));
    state.fatal = fatal.clone();
    let spec = rune::AppSpec {
        scenes: scenes::SCENES,
        min_size: Some((900.0, 600.0)),
        setup: Some(app::setup),
        poll: Some(app::poll),
        on_key: Some(ui::on_key),
        fonts: &[
            include_bytes!("../../../vendor/rune/assets/Inter-Regular.ttf"),
            include_bytes!("../../../vendor/rune/assets/Inter-Medium.ttf"),
            include_bytes!("../../../vendor/rune/assets/Inter-SemiBold.ttf"),
            ascii_painter_core::glyphs::FONT_BYTES,
        ],
        families: ("Inter", "DejaVu Sans Mono"),
        ..rune::AppSpec::new("ASCII Painter", window.size, app::State::default, ui::view)
    };
    rune::set_app_id("ascii-painter");
    let app = rune::App::new(spec)
        .args(argv)
        .titlebar(rune::Titlebar::new(36.0))
        .keep_running()
        .on_close(app::close)
        .window_options(rune::WindowOptions {
            min_size: Some((900.0, 600.0)),
            position: window.position,
            maximized: window.maximized,
            fullscreen: window.fullscreen,
            ..Default::default()
        })
        .compositor(compositor::ViewerCompositor::default());
    let handle = app.handle();
    let updates = handle.clone();
    let app = app
        .init(move || {
            state.app_handle = (!headless).then_some(handle);
            state
        })
        .on_window(move |window| {
            let window = window.clone();
            updates.update(move |s, _| s.window = Some(window));
        });
    app.try_run()?;
    let error = fatal.lock().unwrap().take();
    if let Some(error) = error {
        return Err(error);
    }
    Ok(())
}

fn has_headless_flag() -> bool {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--") {
        args.remove(0);
        if !args.is_empty() {
            args.remove(0);
        }
    } else if args.first().is_some_and(|arg| !arg.to_string_lossy().starts_with('-')) {
        args.remove(0);
    }
    let argv: Vec<String> = args.into_iter().map(|a| a.to_string_lossy().into_owned()).collect();
    rune::tooling::Args::parse_from(argv).headless()
}

fn main() {
    let isolated = std::env::var_os("ASCII_PAINTER_CONFIG_DIR").map(std::path::PathBuf::from);
    if !has_headless_flag() || isolated.is_some() {
        if let Some(log_path) = ascii_painter_core::logging::init(isolated.as_deref()) {
            log::info!(
                "Starting ASCII Painter v{} ({}/{}, pid={}, log={})",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                std::process::id(),
                log_path.display()
            );
        }
    }
    if let Err(error) = run() {
        log::error!("ASCII Painter could not start: {error}");
        log::logger().flush();
        eprintln!("ASCII Painter could not start: {error}");
        #[cfg(target_os = "windows")]
        rfd::MessageDialog::new()
            .set_title("ASCII Painter could not start")
            .set_description(&error)
            .set_level(rfd::MessageLevel::Error)
            .show();
        std::process::exit(1);
    }
    log::info!("ASCII Painter exited cleanly");
    log::logger().flush();
}
