use crate::{
    app::State,
    controls::{self, Number, number},
    export::OutputFormat,
    settings_ui, theme,
    viewer::{self, ViewMode},
};
use ascii_painter_video::EncodeQuality;
use rune::{Cx, El, FileDialogOptions, Phase, ScrollSpec, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, Dialog, Kit, Progress, ScrollBar, Select, SelectAppearance, TextInput,
};

fn button(label: &str) -> Button {
    Button::new(label).control_size(ControlSize::Large)
}

pub fn open_dialog(s: &mut State, tx: &mut rune::Tx) {
    let Some(epoch) = s.begin_dialog() else { return };
    let options = FileDialogOptions { title: "Open image or video".into(), ..Default::default() }
        .filter(
            "Images and videos",
            &[
                "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "tga", "qoi", "ico", "pnm", "hdr", "mp4",
                "mov", "mkv", "webm", "avi", "m4v", "wmv", "flv", "mpg", "mpeg", "ts", "apng", "avif", "heic",
            ],
        )
        .filter("All files", &["*"]);
    tx.open_file(options, move |s: &mut State, result, _| {
        if !s.finish_dialog(epoch) {
            return;
        }
        match result {
            Ok(Some(path)) => s.open(path),
            Ok(None) => {}
            Err(error) => s.error(error.to_string()),
        }
    });
}
fn save_dialog(s: &mut State, tx: &mut rune::Tx) {
    if !s.output_path.as_os_str().is_empty() {
        match crate::export::normalize_output_path(&s.output_path, s.format) {
            Ok(path) => s.set_output_path(path),
            Err(error) => {
                s.error(error);
                return;
            }
        }
    }
    let Some(epoch) = s.begin_dialog() else { return };
    let options = FileDialogOptions {
        title: "Export ASCII art".into(),
        directory: s.output_path.parent().map(|p| p.to_path_buf()),
        file_name: s.output_path.file_name().map(|p| p.to_string_lossy().into_owned()),
        ..Default::default()
    }
    .filter(s.format.label(), &[s.format.extension()]);
    tx.save_file(options, move |s: &mut State, result, _| {
        if !s.finish_dialog(epoch) {
            return;
        }
        match result {
            Ok(Some(path)) => match crate::export::normalize_output_path(&path, s.format) {
                Ok(path) => s.set_output_path(path),
                Err(error) => s.error(error),
            },
            Ok(None) => {}
            Err(error) => s.error(error.to_string()),
        }
    });
}
fn ffmpeg_dialog(s: &mut State, tx: &mut rune::Tx) {
    let Some(epoch) = s.begin_dialog() else { return };
    tx.open_folder(
        FileDialogOptions {
            title: "Choose FFmpeg folder".into(),
            directory: s.prefs.ffmpeg_dir.clone(),
            ..Default::default()
        },
        move |s: &mut State, result, _| {
            if !s.finish_dialog(epoch) {
                return;
            }
            match result {
                Ok(Some(path)) => {
                    s.prefs.ffmpeg_dir = Some(path);
                    s.locate_ffmpeg();
                    s.dirty();
                }
                Ok(None) => {}
                Err(error) => s.error(error.to_string()),
            }
        },
    );
}

fn toolbar(s: &State, k: &Kit) -> El {
    let path =
        s.source.as_ref().map_or_else(|| "or drop a file anywhere".into(), |source| source.path.display().to_string());
    row()
        .key("toolbar")
        .w_full()
        .h(64.0)
        .px(16.0)
        .gap(12.0)
        .items_center()
        .fill(theme::RAISED)
        .child(
            button("Open…")
                .variant(ButtonVariant::Submit)
                .on_press_tx(open_dialog)
                .build(k)
                .key("open")
                .w(theme::OPEN_ACTION_WIDTH)
                .h(theme::PRIMARY_ACTION_HEIGHT),
        )
        .child(theme::muted(path.clone()).grow(1.0).min_w(0.0).truncate_middle().help(path))
        .child(row().gap(3.0).children(ViewMode::ALL.into_iter().map(|mode| {
            Button::new(mode.label())
                .variant(if s.viewer.mode == mode { ButtonVariant::Outline } else { ButtonVariant::GhostMuted })
                .on_press(move |s: &mut State| s.viewer.mode = mode)
                .build(k)
                .key(format!("view-{mode:?}"))
        })))
        .child(theme::muted(format!("{:.0}%", s.viewer.zoom * 100.0)).w(42.0))
        .child(
            Button::new("Fit")
                .help("Reset zoom and pan; double-click the canvas")
                .on_press(|s: &mut State| s.viewer.reset_view())
                .build(k)
                .key("fit"),
        )
}

fn presets(s: &State, k: &Kit, overlays: &mut Vec<El>) -> El {
    let names: Vec<_> = s.presets.iter().map(|p| p.name.as_str()).collect();
    let modified =
        s.presets.iter().find(|p| p.name == s.prefs.preset_name).is_none_or(|p| p.settings != s.prefs.settings);
    let select = Select::new("preset", &names)
        .label("Preset")
        .current(Some(&s.prefs.preset_name))
        .width(s.prefs.sidebar_width - 120.0)
        .appearance(SelectAppearance::Field)
        .open(s.open_select.as_deref() == Some("preset"))
        .on_toggle(|s: &mut State| {
            s.open_select = if s.open_select.as_deref() == Some("preset") { None } else { Some("preset".into()) }
        })
        .on_close(|s: &mut State| s.open_select = None)
        .on_choose(|s: &mut State, name, _| {
            s.choose_preset(name);
            s.open_select = None;
        })
        .build(k, overlays);
    let builtin = ascii_painter_core::presets::builtin().iter().any(|p| p.name == s.prefs.preset_name);
    let can_save = s.preset_dir.is_some()
        && !s.new_preset.text.trim().is_empty()
        && !ascii_painter_core::presets::builtin()
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(s.new_preset.text.trim()));
    let mut body = col()
        .w_full()
        .gap(9.0)
        .child(
            row()
                .w_full()
                .items_center()
                .child(theme::label("Preset").grow(1.0))
                .children(modified.then(|| theme::muted("MODIFIED"))),
        )
        .child(
            row()
                .w_full()
                .gap(4.0)
                .child(
                    Button::new("‹")
                        .help("Previous preset")
                        .on_press(|s: &mut State| s.cycle_preset(-1))
                        .build(k)
                        .key("preset-previous"),
                )
                .child(select.grow(1.0).min_w(0.0))
                .child(
                    Button::new("›")
                        .help("Next preset")
                        .on_press(|s: &mut State| s.cycle_preset(1))
                        .build(k)
                        .key("preset-next"),
                ),
        )
        .child(
            TextInput::new(&s.new_preset)
                .label("New preset name")
                .placeholder("Save your own style…")
                .on_edit(|s: &mut State| &mut s.new_preset)
                .on_submit(|s: &mut State, _| s.save_preset())
                .build(k)
                .key("preset-name")
                .w_full(),
        )
        .child(
            row()
                .gap(6.0)
                .child(
                    Button::new("Save preset")
                        .disabled(!can_save)
                        .on_press(State::save_preset)
                        .build(k)
                        .key("preset-save"),
                )
                .child(
                    Button::new("Delete")
                        .disabled(builtin || s.preset_dir.is_none())
                        .on_press(State::delete_preset)
                        .build(k)
                        .key("preset-delete"),
                ),
        );
    for error in &s.preset_errors {
        body = body.child(theme::muted(error));
    }
    theme::card("YOUR STYLE", body)
}

fn sidebar(s: &State, k: &Kit, overlays: &mut Vec<El>) -> El {
    let ffmpeg = col()
        .gap(8.0)
        .w_full()
        .child(
            theme::muted(match &s.ffmpeg {
                Ok(paths) => format!("FFmpeg: {}", paths.ffmpeg.display()),
                Err(error) => error.clone(),
            })
            .max_lines(3),
        )
        .child(
            row()
                .gap(6.0)
                .child(Button::new("Choose folder…").on_press_tx(ffmpeg_dialog).build(k).key("ffmpeg-folder"))
                .child(
                    Button::new("Use bundled / PATH")
                        .on_press(|s: &mut State| {
                            s.prefs.ffmpeg_dir = None;
                            s.locate_ffmpeg();
                            s.dirty();
                        })
                        .build(k)
                        .key("ffmpeg-default"),
                ),
        );
    let content = col()
        .w_full()
        .gap(14.0)
        .padding(14.0)
        .child(presets(s, k, overlays))
        .child(settings_ui::panel(s, k, overlays))
        .child(theme::card("VIDEO SUPPORT", ffmpeg));
    col().key("sidebar").w(s.prefs.sidebar_width).min_h(0.0).h_full().fill(theme::PANEL).child(
        col()
            .key("settings-scroll")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec { stick_bottom: false })
            .child(content)
            .scroll_bar_styled(theme::settings_scrollbar_style(k)),
    )
}

fn video_controls(s: &State, k: &Kit) -> El {
    col().w_full().gap(6.0).child(number(s, k, "seek", "Preview time (seconds)", Number::Seek, true)).child(
        row()
            .w_full()
            .gap(14.0)
            .child(number(s, k, "trim-start", "Trim start (s)", Number::Start, true).grow(1.0))
            .child(
                Button::new("Start here")
                    .on_press(|s: &mut State| {
                        if let Some(v) = s.video() {
                            Number::Start.set(s, v.time);
                        }
                    })
                    .build(k)
                    .key("start-here"),
            )
            .child(number(s, k, "trim-end", "Trim end (s)", Number::End, true).grow(1.0))
            .child(
                Button::new("End here")
                    .on_press(|s: &mut State| {
                        if let Some(v) = s.video() {
                            Number::End.set(s, v.time);
                        }
                    })
                    .build(k)
                    .key("end-here"),
            ),
    )
}

fn footer(s: &State, k: &Kit, overlays: &mut Vec<El>) -> El {
    let mut formats = settings_ui::choice(
        s,
        k,
        overlays,
        settings_ui::Choice {
            key: "format",
            label: "Output format",
            current: s.format,
            options: OutputFormat::options(s.is_video()),
            name: OutputFormat::label,
            set: |s, v| s.change_format(v),
            enabled: s.export.is_none(),
        },
    )
    .w(190.0);
    if s.is_video() && s.format == OutputFormat::Mp4 {
        formats = row().gap(16.0).child(formats).child(
            settings_ui::choice(
                s,
                k,
                overlays,
                settings_ui::Choice {
                    key: "quality",
                    label: "MP4 quality",
                    current: s.prefs.video_quality,
                    options: &EncodeQuality::ALL,
                    name: EncodeQuality::label,
                    set: |s, v| s.prefs.video_quality = v,
                    enabled: s.export.is_none(),
                },
            )
            .w(190.0),
        );
    }
    let mut info = Vec::new();
    if let Some(grid) = s.preview.grid {
        info.push(format!("{} × {} characters", grid.cols, grid.rows));
    }
    if let Some(size) = s.preview.output_size {
        info.push(format!("{} × {} px", size.w, size.h));
    }
    if s.preview.reduced {
        info.push("Reduced preview · full-resolution export".into());
    }
    if let Some(video) = s.video() {
        if video.info.hdr {
            info.push("HDR input: colours may need tone adjustment".into());
        }
        if s.format == OutputFormat::Ansi
            && let Some(grid) = s.preview.grid
        {
            let seconds = (video.end - video.start).max(0.0);
            let mb = grid.cols as f64 * grid.rows as f64 * 22.0 * video.info.fps.as_f64() * seconds / 1_000_000.0;
            info.push(format!("ANSI video ≈ {mb:.1} MB"));
        }
    }
    let blocker = s.export_blocker();
    let action = if let Some(export) = &s.export {
        button(if export.cancelling { "Cancelling…" } else { "Cancel export" })
            .disabled(export.cancelling)
            .on_press(State::cancel_export)
    } else {
        button("Export")
            .variant(ButtonVariant::Submit)
            .disabled(blocker.is_some())
            .on_press(State::request_export)
            .help(blocker.unwrap_or_else(|| "Export at full resolution".into()))
    };
    let mut footer =
        col().key("export-bar").w_full().padding(14.0).gap(8.0).fill(theme::RAISED).border(1.0, theme::BORDER);
    if s.is_video() {
        footer = footer.child(video_controls(s, k));
    }
    footer = footer
        .child(
            row()
                .w_full()
                .gap(16.0)
                .items_center()
                .child(formats)
                .child(theme::muted(info.join("  ·  ")).grow(1.0).min_w(0.0).max_lines(2)),
        )
        .child(
            row()
                .w_full()
                .gap(8.0)
                .items_center()
                .child(
                    TextInput::new(&s.output)
                        .label("Output file")
                        .placeholder("Choose an output file…")
                        .disabled(s.export.is_some())
                        .on_edit(|s: &mut State| &mut s.output)
                        .on_change(|s: &mut State, _| s.output_edited())
                        .build(k)
                        .key("output-path")
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(
                    button("Browse…")
                        .disabled(s.export.is_some())
                        .on_press_tx(save_dialog)
                        .build(k)
                        .key("browse-output"),
                )
                .child(action.build(k).key("export").w(theme::EXPORT_ACTION_WIDTH).h(theme::PRIMARY_ACTION_HEIGHT)),
        );
    if let Some(export) = &s.export {
        let progress = export.progress.as_ref();
        let detail = progress.map_or_else(
            || "Preparing…".into(),
            |p| {
                format!(
                    "{}{} frames  ·  {:.1} fps{}",
                    p.done,
                    p.total.map_or_else(String::new, |t| format!(" / {t}")),
                    p.rate,
                    p.eta.map_or_else(String::new, |e| format!("  ·  {}s remaining", e.as_secs()))
                )
            },
        );
        footer = footer
            .child(Progress::new(progress.and_then(|p| p.fraction())).build(k).w_full())
            .child(theme::muted(detail));
    } else if let Some(status) = &s.status {
        footer = footer.child(theme::muted(status).truncate().w_full());
    }
    footer
}

fn overwrite(s: &State, k: &Kit, overlays: &mut Vec<El>) {
    let Some(intent) = &s.overwrite else { return };
    let content = col()
        .w_full()
        .padding(16.0)
        .gap(14.0)
        .child(theme::label(format!("{} already exists.", intent.output.display())).wrap())
        .child(theme::muted("The existing file stays intact until this export succeeds."))
        .child(
            row()
                .gap(8.0)
                .child(button("Cancel").on_press(|s: &mut State| s.overwrite = None).build(k).key("overwrite-cancel"))
                .child(
                    button("Replace file")
                        .variant(ButtonVariant::Submit)
                        .on_press(State::confirm_export)
                        .build(k)
                        .key("overwrite-confirm")
                        .default_button(),
                ),
        );
    Dialog::new("overwrite", "Replace file?")
        .open(true)
        .fit()
        .click_outside(false)
        .on_close(|s: &mut State| s.overwrite = None)
        .child(content)
        .build(k, overlays);
}

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    cx.caption_color(theme::TEXT);
    let k = theme::kit();
    let mut overlays = Vec::new();
    let inset = cx.chrome_insets();
    let title = row()
        .key("titlebar")
        .h(36.0)
        .w_full()
        .pl(16.0 + inset.leading)
        .pr(12.0 + inset.trailing)
        .gap(10.0)
        .items_center()
        .window_drag()
        .fill(theme::PANEL)
        .child(text("Aa").font(theme::body().mono()).color(theme::ACCENT))
        .child(theme::label("ASCII Painter"))
        .child(theme::muted("/  CHARACTER STUDIO"));
    let divider = el()
        .key("sidebar-resize")
        .w(6.0)
        .h_full()
        .fill(theme::BORDER)
        .hover(theme::ACCENT.alpha(0.4))
        .cursor(rune::Cursor::ResizeColumn)
        .on_drag(|s: &mut State, d, _| {
            if d.phase == Phase::Down {
                s.sidebar_start = s.prefs.sidebar_width;
            }
            s.prefs.sidebar_width =
                if d.cancelled() { s.sidebar_start } else { (s.sidebar_start + d.dx).clamp(280.0, 560.0) };
        });
    let middle =
        row().w_full().grow(1.0).min_h(0.0).child(sidebar(s, &k, &mut overlays)).child(divider).child(viewer::view(s));
    let mut body = col()
        .key("app")
        .role("window")
        .label("ASCII Painter")
        .on_layout(|s: &mut State, r| s.viewport_size = [r[2], r[3]])
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(theme::PANEL)
        .on_drop_files(|s: &mut State, paths, _| {
            if !s.blocked()
                && let Some(path) = paths.first()
            {
                s.open(path.clone());
            }
        })
        .child(title)
        .child(toolbar(s, &k));
    if let Some(warning) = &s.warning {
        body = body.child(
            row()
                .w_full()
                .padding(8.0)
                .fill(theme::BORDER)
                .gap(8.0)
                .child(theme::muted(warning).grow(1.0).min_w(0.0).max_lines(3))
                .child(Button::new("Dismiss").on_press(|s: &mut State| s.warning = None).build(&k)),
        );
    }
    body = body.child(middle).child(footer(s, &k, &mut overlays));
    if s.native_dialog || s.closing {
        body = body.inert();
    }
    overwrite(s, &k, &mut overlays);
    controls::color_dialog(s, &k, &mut overlays);
    if let Some(toast) = &s.toast {
        let alpha = ((toast.until - s.now) / 0.4).clamp(0.0, 1.0) as f32;
        overlays.push(
            row()
                .key("export-toast")
                .pin(None, None, Some(20.0), Some(20.0))
                .max_w(560.0)
                .gap(12.0)
                .items_center()
                .padding(14.0)
                .radius(10.0)
                .fill(theme::RAISED)
                .border(1.0, if toast.error { theme::DANGER } else { theme::SUCCESS })
                .opacity(alpha)
                .child(text(if toast.error { "!" } else { "✓" }).font(theme::body()).color(if toast.error {
                    theme::DANGER
                } else {
                    theme::SUCCESS
                }))
                .child(theme::label(&toast.text).max_w(440.0).max_lines(3).help(&toast.text))
                .child(Button::new("×").on_press(|s: &mut State| s.toast = None).build(&k)),
        );
    }
    let root = rune_kit::root(body, overlays);
    resize_grips(root, s)
}

#[allow(unused_mut)]
fn resize_grips(mut root: El, s: &State) -> El {
    #[cfg(target_os = "linux")]
    {
        use rune::WindowResize::*;
        let native = s.window.as_ref().map(|w| w.geometry());
        if native.is_some_and(|g| g.maximized || g.fullscreen) {
            return root;
        }
        for (key, edge, l, t, r, b, w, h) in [
            ("resize-n", North, Some(12.0), Some(0.0), Some(12.0), None, None, Some(6.0)),
            ("resize-s", South, Some(12.0), None, Some(12.0), Some(0.0), None, Some(6.0)),
            ("resize-w", West, Some(0.0), Some(12.0), None, Some(12.0), Some(6.0), None),
            ("resize-e", East, None, Some(12.0), Some(0.0), Some(12.0), Some(6.0), None),
            ("resize-nw", NorthWest, Some(0.0), Some(0.0), None, None, Some(12.0), Some(12.0)),
            ("resize-ne", NorthEast, None, Some(0.0), Some(0.0), None, Some(12.0), Some(12.0)),
            ("resize-sw", SouthWest, Some(0.0), None, None, Some(0.0), Some(12.0), Some(12.0)),
            ("resize-se", SouthEast, None, None, Some(0.0), Some(0.0), Some(12.0), Some(12.0)),
        ] {
            let mut grip = el().key(key).pin(l, t, r, b).window_resize(edge);
            if let Some(w) = w {
                grip = grip.w(w);
            }
            if let Some(h) = h {
                grip = grip.h(h);
            }
            root = root.child(grip);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = s;
    root
}

pub fn on_key(s: &mut State, event: &rune::KeyEvent, tx: &mut rune::Tx) -> bool {
    if s.blocked() {
        return false;
    }
    if let Some(window) = &s.window {
        let fullscreen = window.geometry().fullscreen;
        if event.key == rune::Key::Function(11) || (fullscreen && event.key == rune::Key::Escape) {
            let _ = window.set_fullscreen(!fullscreen && event.key != rune::Key::Escape);
            return true;
        }
    }
    if event.is_command('o') {
        open_dialog(s, tx);
        return true;
    }
    false
}
