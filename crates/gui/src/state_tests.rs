use super::*;

fn wait_until(mut done: impl FnMut() -> bool) {
    let start = std::time::Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn loaded(name: &str) -> Loaded {
    Loaded {
        source: Source { path: name.into(), kind: SourceKind::Image(Arc::new(image::DynamicImage::new_rgb8(8, 6))) },
        original: None,
    }
}

fn loaded_video(name: &str) -> Loaded {
    Loaded {
        source: Source {
            path: name.into(),
            kind: SourceKind::Video(VideoSource {
                info: MediaInfo {
                    display: Size2 { w: 8, h: 6 },
                    fps: ascii_painter_video::Fps { num: 5, den: 1 },
                    duration: Some(1.0),
                    frames: Some(5),
                    has_audio: false,
                    hdr: false,
                },
                time: 0.0,
                start: 0.0,
                end: 1.0,
            }),
        },
        original: None,
    }
}

#[test]
fn completion_between_pending_poll_and_caller_check_is_not_a_worker_failure() {
    let mut s = State { load_generation: 1, ..Default::default() };
    let (release, held) = mpsc::channel();
    let mut job = Job::spawn("load-race", s.wake.clone(), move |_| {
        held.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(loaded("accepted.png"))
    });
    job.on_pending(move |job| {
        release.send(()).unwrap();
        wait_until(|| job.is_finished());
    });
    s.loading.push((1, job));
    poll(&mut s, 1.0, &mut Tx::default());
    assert!(s.toast.is_none(), "queued success must not become a worker failure");
    assert_eq!(s.loading.len(), 1, "a pending observation is retried next frame");
    poll(&mut s, 1.1, &mut Tx::default());
    assert_eq!(s.source.as_ref().unwrap().path, PathBuf::from("accepted.png"));
    assert!(s.loading.is_empty());
}

#[test]
fn export_completion_between_pending_poll_and_caller_check_is_retried() {
    let mut s = State::default();
    let (release, held) = mpsc::channel();
    let mut job = Job::spawn("export-race", s.wake.clone(), move |_| {
        held.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(PathBuf::from("saved.png"))
    });
    job.on_pending(move |job| {
        release.send(()).unwrap();
        wait_until(|| job.is_finished());
    });
    let (_, progress_rx) = mpsc::channel();
    s.export = Some(ExportState { job: Some(job), progress_rx, progress: None, cancelling: false });
    poll(&mut s, 1.0, &mut Tx::default());
    assert!(s.toast.is_none());
    assert!(s.export.is_some());
    poll(&mut s, 1.1, &mut Tx::default());
    assert_eq!(s.status.as_deref(), Some("Saved saved.png"));
    assert!(s.export.is_none());
}

#[test]
fn loading_preserves_every_compatible_format_and_falls_back_only_when_needed() {
    for video in [false, true] {
        for format in [OutputFormat::Png, OutputFormat::Mp4, OutputFormat::Text, OutputFormat::Ansi] {
            let mut s = State { format, load_generation: 1, ..Default::default() };
            let job = Job::spawn("format-load", s.wake.clone(), move |_| {
                Ok(if video { loaded_video("new.mp4") } else { loaded("new.png") })
            });
            s.loading.push((1, job));
            wait_until(|| {
                poll(&mut s, 1.0, &mut Tx::default());
                s.loading.is_empty()
            });
            let expected = if OutputFormat::options(video).contains(&format) {
                format
            } else if video {
                OutputFormat::Mp4
            } else {
                OutputFormat::Png
            };
            assert_eq!(s.format, expected, "video={video}, selected={format:?}");
            assert_eq!(s.output_path.extension().unwrap(), expected.extension());
        }
    }
}

#[test]
fn unknown_and_deleted_presets_start_at_the_directional_boundary() {
    let mut s = State::default();
    let first = s.presets.first().unwrap().name.clone();
    let last = s.presets.last().unwrap().name.clone();
    for unknown in ["Custom", "deleted user preset"] {
        s.prefs.preset_name = unknown.into();
        s.cycle_preset(1);
        assert_eq!(s.prefs.preset_name, first);
        s.prefs.preset_name = unknown.into();
        s.cycle_preset(-1);
        assert_eq!(s.prefs.preset_name, last);
    }
}

#[test]
fn typed_output_is_normalized_before_confirmation_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    let output = dir.path().join("art.png");
    std::fs::write(&input, b"source").unwrap();
    std::fs::write(&output, b"old").unwrap();
    let mut s = State { source: Some(loaded("unused").source), ..Default::default() };
    s.source.as_mut().unwrap().path = input;
    s.output = TextEdit::new(format!("  {}  ", dir.path().join("art.txt").display()));
    s.output_edited();
    s.request_export();
    assert_eq!(s.output_path, output);
    assert_eq!(s.overwrite.as_ref().unwrap().output, output);
    assert!(s.export.is_none());
    s.confirm_export();
    wait_until(|| {
        poll(&mut s, 1.0, &mut Tx::default());
        s.export.is_none()
    });
    assert_eq!(&std::fs::read(&output).unwrap()[..8], b"\x89PNG\r\n\x1a\n");
    assert!(!dir.path().join("art.txt").exists());
}

#[test]
fn a_queued_result_does_not_join_a_thread_still_inside_its_waker() {
    let mut s = State { load_generation: 1, ..Default::default() };
    let (entered, observed) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let held = std::sync::Mutex::new(held);
    let wake = Arc::new(move || {
        entered.send(()).unwrap();
        let _ = held.lock().unwrap().recv_timeout(Duration::from_secs(5));
    });
    let job = Job::spawn("held-waker", wake, |_| Ok(loaded("ready.png")));
    s.loading.push((1, job));
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    let start = std::time::Instant::now();
    poll(&mut s, 1.0, &mut Tx::default());
    let elapsed = start.elapsed();
    let pending = !s.loading.is_empty();
    let _ = release.send(()); // release even if a check below fails
    wait_until(|| {
        poll(&mut s, 1.1, &mut Tx::default());
        s.loading.is_empty()
    });
    assert!(pending);
    assert!(elapsed < Duration::from_millis(200), "UI joined an unfinished waker: {elapsed:?}");
    assert_eq!(s.source.as_ref().unwrap().path, PathBuf::from("ready.png"));
}

#[test]
fn worker_panics_are_reported_once_as_disconnections() {
    let mut s = State { load_generation: 1, ..Default::default() };
    s.loading.push((1, Job::spawn("panicked-load", s.wake.clone(), |_| panic!("test panic"))));
    wait_until(|| {
        poll(&mut s, 1.0, &mut Tx::default());
        s.loading.is_empty()
    });
    assert_eq!(s.toast.as_ref().unwrap().text, "Media worker stopped unexpectedly");
    s.toast = None;
    poll(&mut s, 1.1, &mut Tx::default());
    assert!(s.toast.is_none());
    let (_, progress_rx) = mpsc::channel();
    s.export = Some(ExportState {
        job: Some(Job::spawn("panicked-export", s.wake.clone(), |_| panic!("test panic"))),
        progress_rx,
        progress: None,
        cancelling: false,
    });
    wait_until(|| {
        poll(&mut s, 1.2, &mut Tx::default());
        s.export.is_none()
    });
    assert_eq!(s.toast.as_ref().unwrap().text, "Export worker stopped unexpectedly");
}

#[test]
fn native_title_follows_accepted_unicode_sources_and_ignores_failed_or_stale_loads() {
    let mut s = State::default();
    poll(&mut s, 0.0, &mut Tx::default());
    assert_eq!(s.requested_title.as_deref(), Some("ASCII Painter"));
    for (index, video) in [false, true].into_iter().enumerate() {
        let name = if video { "电影.mp4" } else { "café.png" };
        s.load_generation = index as u64 + 1;
        s.loading.push((
            s.load_generation,
            Job::spawn("unicode-load", s.wake.clone(), move |_| {
                Ok(if video { loaded_video(name) } else { loaded(name) })
            }),
        ));
        wait_until(|| {
            poll(&mut s, 1.0, &mut Tx::default());
            s.loading.is_empty()
        });
        assert_eq!(s.requested_title.as_deref(), Some(format!("ASCII Painter — {name}").as_str()));
    }
    let title = s.requested_title.clone();
    let path = s.source.as_ref().unwrap().path.clone();
    let output = s.output_path.clone();
    s.format = OutputFormat::Ansi;
    s.loading.push((1, Job::spawn("stale-success", s.wake.clone(), |_| Ok(loaded("obsolete.png")))));
    s.loading.push((1, Job::spawn("stale-error", s.wake.clone(), |_| Err("obsolete error".into()))));
    s.loading.push((2, Job::spawn("current-error", s.wake.clone(), |_| Err("cannot load".into()))));
    wait_until(|| {
        poll(&mut s, 2.0, &mut Tx::default());
        s.loading.is_empty()
    });
    assert_eq!(s.format, OutputFormat::Ansi);
    assert_eq!(s.output_path, output);
    assert_eq!(s.source.as_ref().unwrap().path, path);
    assert_eq!(s.requested_title, title);
    assert_eq!(s.toast.as_ref().unwrap().text, "cannot load");
}

#[test]
fn preset_navigation_wraps_modified_listed_names_and_handles_empty_single_and_deleted_lists() {
    let mut s = State::default();
    let first = s.presets.first().unwrap().name.clone();
    let last = s.presets.last().unwrap().name.clone();
    s.choose_preset(&first);
    s.prefs.settings.tone.contrast = 1.5;
    s.cycle_preset(-1);
    assert_eq!(s.prefs.preset_name, last);
    s.cycle_preset(1);
    assert_eq!(s.prefs.preset_name, first);
    s.presets.truncate(1);
    for direction in [-1, 1] {
        s.prefs.preset_name = "unknown".into();
        s.cycle_preset(direction);
        assert_eq!(s.prefs.preset_name, first);
        s.cycle_preset(direction);
        assert_eq!(s.prefs.preset_name, first);
    }
    s.presets.clear();
    s.cycle_preset(-1);
    s.cycle_preset(1);
    assert_eq!(s.prefs.preset_name, first);
    let dir = tempfile::tempdir().unwrap();
    s.preset_dir = Some(dir.path().into());
    let user = Preset { name: "User preset".into(), settings: Settings::default() };
    presets::save_user(dir.path(), &user).unwrap();
    s.reload_presets();
    s.choose_preset(&user.name);
    s.delete_preset();
    assert!(!s.presets.iter().any(|p| p.name == user.name));
    s.cycle_preset(1);
    assert_eq!(s.prefs.preset_name, first);
}

#[test]
fn output_edits_formats_and_invalid_destinations_share_one_normalization_policy() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    std::fs::write(&input, b"source").unwrap();
    let mut s = State { source: Some(loaded("unused").source), ..Default::default() };
    s.source.as_mut().unwrap().path = input;
    for format in [OutputFormat::Png, OutputFormat::Text, OutputFormat::Ansi, OutputFormat::Mp4] {
        s.set_output_path(dir.path().join("café.旧"));
        s.change_format(format);
        assert_eq!(s.output_path, dir.path().join(format!("café.{}", format.extension())));
    }
    s.set_output_path(PathBuf::new());
    s.change_format(OutputFormat::Png);
    assert!(s.output_path.as_os_str().is_empty());
    s.set_output_path(dir.path().join("without-extension"));
    s.change_format(OutputFormat::Ansi);
    assert_eq!(s.output_path, dir.path().join("without-extension.ans"));
    for path in [
        "   ".to_owned(),
        dir.path().display().to_string(),
        format!("{}{sep}", dir.path().join("new").display(), sep = std::path::MAIN_SEPARATOR),
    ] {
        s.output = TextEdit::new(path);
        s.output_edited();
        s.request_export();
        assert!(s.export.is_none());
        assert!(s.overwrite.is_none());
        assert!(s.toast.as_ref().unwrap().error);
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn gui_rejects_linked_destinations_before_confirmation_or_staging() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    let target = dir.path().join("target.png");
    let link = dir.path().join("link.png");
    std::fs::write(&input, b"source").unwrap();
    std::fs::write(&target, b"target bytes").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    match std::os::windows::fs::symlink_file(&target, &link) {
        Ok(()) => {}
        Err(error) if error.raw_os_error() == Some(1314) => {
            eprintln!("Windows symbolic-link validation unavailable: privilege not held");
            return;
        }
        Err(error) => panic!("test link: {error}"),
    }
    let mut s = State { source: Some(loaded("unused").source), ..Default::default() };
    s.source.as_mut().unwrap().path = input.clone();
    s.set_output_path(link.clone());
    s.request_export();
    assert!(s.export.is_none() && s.overwrite.is_none());
    assert!(s.toast.as_ref().unwrap().error);
    assert_eq!(std::fs::read_link(&link).unwrap(), target);
    assert_eq!(std::fs::read(&target).unwrap(), b"target bytes");
    assert_eq!(std::fs::read(&input).unwrap(), b"source");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 3);
}

#[cfg(unix)]
#[test]
fn unedited_native_paths_keep_non_utf8_bytes_through_format_and_export() {
    use std::os::unix::ffi::OsStringExt;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    std::fs::write(&input, b"source").unwrap();
    let path = dir.path().join(std::ffi::OsString::from_vec(b"art-\xff.old".to_vec()));
    let mut s = State { source: Some(loaded("unused").source), ..Default::default() };
    s.source.as_mut().unwrap().path = input;
    s.set_output_path(path.clone()); // exactly what the native dialog callback does
    s.change_format(OutputFormat::Png);
    assert_eq!(s.output_path, path.with_extension("png"));
    s.request_export();
    wait_until(|| {
        poll(&mut s, 1.0, &mut Tx::default());
        s.export.is_none()
    });
    assert!(path.with_extension("png").exists());
}

#[test]
fn stale_preview_successes_and_failures_never_mutate_the_current_preview() {
    let mut s = State { generation: 3, ..Default::default() };
    s.preview.waiting = true;
    assert!(!s.accept_preview(PreviewResponse { generation: 2, result: Err("stale error".into()) }));
    assert!(s.preview.waiting);
    assert!(s.preview.error.is_none());
    let output = crate::worker::PreviewOutput {
        ascii: ViewerImage::new(image::RgbaImage::new(1, 1)),
        original: None,
        grid: GridSize { cols: 1, rows: 1 },
        output_size: Size2 { w: 1, h: 1 },
        reduced: false,
        rejected: Vec::new(),
    };
    assert!(!s.accept_preview(PreviewResponse { generation: 1, result: Ok(output) }));
    assert!(s.preview.ascii.is_none());
    assert!(s.accept_preview(PreviewResponse { generation: 3, result: Err("current error".into()) }));
    assert!(!s.preview.waiting);
    assert_eq!(s.preview.error.as_deref(), Some("current error"));
    s.closing = true;
    assert!(!s.accept_preview(PreviewResponse { generation: 3, result: Err("late callback".into()) }));
    assert_eq!(s.preview.error.as_deref(), Some("current error"));
}
#[test]
fn out_of_order_load_results_only_apply_the_latest_source() {
    let mut s = State { load_generation: 2, ..Default::default() };
    let old = Job::spawn("old-load", s.wake.clone(), |_| Err("obsolete failure".into()));
    let new = Job::spawn("new-load", s.wake.clone(), |_| Ok(loaded("new.png")));
    s.loading = vec![(1, old), (2, new)];
    wait_until(|| {
        poll(&mut s, 1.0, &mut Tx::default());
        s.loading.is_empty()
    });
    assert_eq!(s.source.as_ref().unwrap().path, PathBuf::from("new.png"));
    assert!(s.toast.is_none());
}
#[test]
fn overwrite_confirmation_uses_the_captured_settings_format_source_and_path() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    let output = dir.path().join("out.png");
    std::fs::write(&input, b"source identity").unwrap();
    std::fs::write(&output, b"old destination").unwrap();
    let mut s = State { source: Some(loaded("unused").source), ..Default::default() };
    s.source.as_mut().unwrap().path = input;
    s.prefs.settings.grid.columns = 16;
    let captured = s.prefs.settings.clone();
    s.set_output_path(output.clone());
    s.request_export();
    assert!(s.overwrite.is_some());
    assert!(s.export.is_none());
    assert_eq!(std::fs::read(&output).unwrap(), b"old destination");
    s.prefs.settings.grid.columns = 100;
    s.format = OutputFormat::Text;
    s.set_output_path(dir.path().join("later.txt"));
    s.confirm_export();
    assert!(s.export.is_some());
    wait_until(|| {
        poll(&mut s, 2.0, &mut Tx::default());
        s.export.is_none()
    });
    let actual = image::open(&output).unwrap();
    let c = Converter::new(captured).unwrap();
    let expected = c.output_size(c.grid_for(Size2 { w: 8, h: 6 }));
    assert_eq!((actual.width(), actual.height()), (expected.w, expected.h));
    assert!(!dir.path().join("later.txt").exists());
}
#[test]
fn shutdown_cancels_immediately_and_joins_cpu_work_off_the_ui_thread() {
    let mut s = State::default();
    let (release, held) = mpsc::channel();
    let job = Job::spawn("held-load", s.wake.clone(), move |_| {
        held.recv().unwrap();
        Err("cancelled".into())
    });
    let cancel = job.cancel.clone();
    s.loading.push((1, job));
    let start = std::time::Instant::now();
    close(&mut s, &mut Tx::default());
    let elapsed = start.elapsed();
    let was_still_working = !s.cleanup.as_ref().unwrap().is_finished();
    release.send(()).unwrap();
    wait_until(|| s.cleanup.as_ref().unwrap().is_finished());
    assert!(cancel.is_cancelled());
    assert!(was_still_working);
    assert!(elapsed < Duration::from_millis(200));
    poll(&mut s, 1.0, &mut Tx::default());
    assert!(s.cleanup.is_none());
}

#[test]
fn gui_video_export_publishes_a_completed_mp4_instead_of_cancelling_it() {
    let paths = match FfmpegPaths::locate(None) {
        Ok(paths) => paths,
        Err(error) => {
            assert!(std::env::var_os("ASCII_PAINTER_REQUIRE_MEDIA_TESTS").is_none(), "required media tools: {error}");
            eprintln!("skipping: {error}");
            return;
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.mp4");
    let output = dir.path().join("output.mp4");
    let status = std::process::Command::new(&paths.ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=d=0.6:s=80x60:r=5",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=0.6",
            "-c:v",
            "libx264",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(&input)
        .status()
        .unwrap();
    assert!(status.success());
    let info = ascii_painter_video::probe(&paths, &input).unwrap();
    std::fs::write(&output, b"old destination").unwrap();
    let mut s = State {
        ffmpeg: Ok(paths.clone()),
        source: Some(Source {
            path: input.clone(),
            kind: SourceKind::Video(VideoSource { info, time: 0.0, start: 0.0, end: 0.6 }),
        }),
        format: OutputFormat::Mp4,
        ..Default::default()
    };
    s.prefs.settings.grid.columns = 16;
    s.prefs.settings.render.font_px = 8.0;
    s.set_output_path(output.clone());
    s.request_export();
    assert!(s.overwrite.is_some());
    s.confirm_export();
    wait_until(|| {
        poll(&mut s, 1.0, &mut Tx::default());
        s.export.is_none()
    });
    assert!(s.status.as_ref().is_some_and(|s| s.starts_with("Saved ")), "{:?}", s.status);
    let exported = ascii_painter_video::probe(&paths, &output).unwrap();
    assert_eq!(exported.frames, Some(3));
    assert!(exported.has_audio);
    assert!(input.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2, "no staging output is left behind");
}

#[test]
fn repeated_opens_keep_one_active_loader_and_only_the_latest_pending_file() {
    let mut s = State::default();
    let (release, held) = mpsc::channel();
    let job = Job::spawn("held-load", s.wake.clone(), move |_| {
        held.recv().unwrap();
        Err("cancelled".into())
    });
    s.loading.push((0, job));
    s.open("first.png".into());
    s.open("latest.png".into());
    assert_eq!(s.loading.len(), 1);
    assert_eq!(s.pending_load.as_ref().unwrap().1, PathBuf::from("latest.png"));
    close(&mut s, &mut Tx::default());
    release.send(()).unwrap();
    wait_until(|| s.cleanup.as_ref().unwrap().is_finished());
    assert!(s.pending_load.is_none());
}
