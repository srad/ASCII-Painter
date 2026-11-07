//! Deterministic interaction checks: no real preferences, dialogs or media processes.
use crate::{
    app::{self, State},
    controls::{self, Number},
    theme, ui,
    viewer::ViewMode,
};
use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, col};

fn frame(rt: &mut Runtime<State>) {
    rt.frame(1.0 / 60.0);
}
fn click(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("missing {key}"));
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frame(rt);
}
fn key(rt: &mut Runtime<State>, key: Key) {
    rt.key(KeyEvent { key, mods: Mods::default() });
    frame(rt);
}
fn numeric_view(s: &State, _: &mut Cx<State>) -> El {
    col()
        .w_full()
        .h_full()
        .padding(20.0)
        .gap(10.0)
        .fill(theme::PANEL)
        .child(controls::number(s, &theme::kit(), "contrast", "Contrast", Number::Contrast, true))
        .child(rune_kit::Button::new("Blur").build(&theme::kit()).key("blur").on_press(|_: &mut State| {}))
}
fn numeric_runtime() -> Runtime<State> {
    let mut rt = Runtime::new(State::default(), numeric_view, (360.0, 180.0), Env::new(true, 1.0));
    rt.poll = Some(app::poll);
    frame(&mut rt);
    rt
}
fn type_value(rt: &mut Runtime<State>, text: &str) {
    click(rt, "contrast-value");
    key(rt, Key::Command('a'));
    key(rt, Key::Text(text.into()));
}

#[test]
fn numeric_drafts_allow_empty_and_reject_nonfinite_values_until_committed() {
    let mut s = State::default();
    let original = s.prefs.settings.tone.contrast;
    for text in ["", "NaN", "inf", "-1", "4"] {
        let d = s.numbers.get_mut(&Number::Contrast).unwrap();
        d.edit.text = text.into();
        d.dirty = true;
        assert!(!controls::commit_number(&mut s, Number::Contrast));
        assert_eq!(s.prefs.settings.tone.contrast, original);
    }
    let d = s.numbers.get_mut(&Number::Contrast).unwrap();
    d.edit.text = "2.25".into();
    d.dirty = true;
    assert!(controls::commit_number(&mut s, Number::Contrast));
    assert_eq!(s.prefs.settings.tone.contrast, 2.25);
}
#[test]
fn numeric_enter_blur_and_escape_have_distinct_commit_behavior() {
    let mut rt = numeric_runtime();
    // Click the field's box as a pointer user would.
    type_value(&mut rt, "2.25");
    assert_eq!(rt.state.prefs.settings.tone.contrast, 1.0);
    key(&mut rt, Key::Enter);
    assert_eq!(rt.state.prefs.settings.tone.contrast, 2.25);
    type_value(&mut rt, "1.75");
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.prefs.settings.tone.contrast, 2.25);
    type_value(&mut rt, "1.50");
    click(&mut rt, "blur");
    frame(&mut rt);
    assert_eq!(rt.state.prefs.settings.tone.contrast, 1.5);
}
#[test]
fn slider_keyboard_and_cancelled_drag_preserve_the_committed_value() {
    let mut rt = numeric_runtime();
    click(&mut rt, "contrast");
    let base = rt.state.prefs.settings.tone.contrast;
    key(&mut rt, Key::Right);
    assert!((rt.state.prefs.settings.tone.contrast - base - 0.01).abs() < 1e-5);
    let original = rt.state.prefs.settings.tone.contrast;
    let (x, y) = rt.find("contrast").unwrap();
    rt.pointer_down(x - 70.0, y);
    rt.pointer_move(x + 70.0, y);
    frame(&mut rt);
    assert_ne!(rt.state.prefs.settings.tone.contrast, original);
    key(&mut rt, Key::Escape);
    assert!((rt.state.prefs.settings.tone.contrast - original).abs() < 1e-5);
    rt.pointer_up(x + 70.0, y);
    frame(&mut rt);
    assert_eq!(rt.state.prefs.settings.tone.contrast, original);
}
#[test]
fn rgb_hsv_roundtrips_and_numeric_limits_share_one_policy() {
    for rgb in [
        ascii_painter_core::Rgb::new(255, 0, 0),
        ascii_painter_core::Rgb::new(12, 94, 231),
        ascii_painter_core::Rgb::BLACK,
        ascii_painter_core::Rgb::WHITE,
    ] {
        assert_eq!(controls::hsv_to_rgb(controls::rgb_to_hsv(rgb)), rgb);
    }
    let s = State::default();
    let spec = Number::Columns.spec(&s);
    assert_eq!(spec.at(0.0), 8.0);
    assert_eq!(spec.at(1.0), 2000.0);
    assert!((spec.fraction(160.0) - ((160.0f64 / 8.0).ln() / (2000.0f64 / 8.0).ln()) as f32).abs() < 1e-5);
}
#[test]
fn comparison_camera_handles_cumulative_rune_drag_deltas() {
    let mut s = State::default();
    crate::scenes::image(&mut s);
    s.viewer.mode = ViewMode::Ascii;
    let mut rt = Runtime::new(s, ui::view, (1400.0, 900.0), Env::new(true, 1.0));
    frame(&mut rt);
    let (x, y) = rt.find("viewer").unwrap();
    rt.pointer_down(x, y);
    frame(&mut rt);
    rt.pointer_move(x + 10.0, y + 20.0);
    frame(&mut rt);
    rt.pointer_move(x + 30.0, y + 40.0);
    frame(&mut rt);
    assert_eq!(rt.state.viewer.pan, [30.0, 40.0]);
    rt.pointer_up(x + 30.0, y + 40.0);
    frame(&mut rt);
    click(&mut rt, "fit");
    assert_eq!(rt.state.viewer.pan, [0.0, 0.0]);
    assert_eq!(rt.state.viewer.zoom, 1.0);
}
#[test]
fn minimum_layout_keeps_core_controls_inside_the_window_at_supported_dpi() {
    for scale in [1.0, 1.5, 2.0] {
        for size in [(900.0, 600.0), (1400.0, 900.0)] {
            let mut rt = Runtime::new(State::default(), ui::view, size, Env::new(true, scale));
            for _ in 0..3 {
                frame(&mut rt);
            }
            for key in ["titlebar", "toolbar", "open", "sidebar", "viewer", "export-bar", "export", "output-path"] {
                let rect = rt.presented(key).unwrap_or_else(|| panic!("missing {key}")).0;
                assert!(
                    rect[0] >= -0.1
                        && rect[1] >= -0.1
                        && rect[0] + rect[2] <= size.0 + 0.1
                        && rect[1] + rect[3] <= size.1 + 0.1,
                    "{key} {rect:?} outside {size:?} at {scale}"
                );
            }
        }
    }
}
#[test]
fn modal_blocks_background_controls_and_file_drops() {
    let mut s = State::default();
    crate::scenes::image(&mut s);
    controls::open_color(&mut s, controls::ColorTarget::Foreground, ascii_painter_core::Rgb::WHITE);
    let mut rt = Runtime::new(s, ui::view, (1400.0, 900.0), Env::new(true, 1.0));
    frame(&mut rt);
    click(&mut rt, "view-Original");
    assert_eq!(rt.state.viewer.mode, ViewMode::Split);
    rt.drop_files(vec!["not-a-real-file.png".into()], Some((20.0, 60.0)));
    frame(&mut rt);
    assert_eq!(rt.state.source.as_ref().unwrap().path.to_string_lossy(), "demo/sunset.png");
    assert!(rt.state.color.is_some());
}
#[test]
fn closing_rejects_late_native_dialog_results() {
    let mut s = State::default();
    let epoch = s.begin_dialog().unwrap();
    app::close(&mut s, &mut rune::Tx::default());
    assert!(!s.finish_dialog(epoch));
    assert!(s.closing);
}
#[test]
fn unchanged_state_does_not_request_reconversion_or_preference_writes() {
    let mut s = State::default();
    let mut tx = rune::Tx::default();
    assert!(!app::poll(&mut s, 0.0, &mut tx));
    assert!(!app::poll(&mut s, 100.0, &mut tx));
    assert!(s.preference_path.is_none());
    assert!(s.preset_dir.is_none());
}

#[test]
fn save_dialog_defaults_and_answers_normalize_format_without_rewriting_native_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = State::default();
    crate::scenes::image(&mut s);
    s.set_output_path(dir.path().join("art.txt"));
    let mut rt = Runtime::new(s, ui::view, (1400.0, 900.0), Env::new(true, 1.0));
    rt.poll = Some(app::poll);
    frame(&mut rt);
    click(&mut rt, "browse-output");
    let (_, options) = rt.file_dialog().expect("save dialog");
    assert_eq!(options.file_name.as_deref(), Some("art.png"));
    assert_eq!(options.directory.as_deref(), Some(dir.path()));
    let answer = dir.path().join(" café 作品.old");
    rt.resolve_file_dialog(Some(answer.clone())).unwrap();
    frame(&mut rt);
    assert_eq!(rt.state.output_path, answer.with_extension("png"));
    assert!(!rt.state.native_dialog);
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        click(&mut rt, "browse-output");
        let answer = dir.path().join(std::ffi::OsString::from_vec(b"art-\xff.old".to_vec()));
        rt.resolve_file_dialog(Some(answer.clone())).unwrap();
        frame(&mut rt);
        assert_eq!(rt.state.output_path, answer.with_extension("png"));
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "dialogs do not write files");
}

#[test]
fn native_resize_actions_are_typed_and_available_during_the_press() {
    use rune::{TitlebarAction, WindowResize::*};
    fn resize_view(edge: &rune::WindowResize, _: &mut Cx<rune::WindowResize>) -> El {
        rune::el().key("grip").size(100.0, 100.0).window_resize(*edge)
    }
    for edge in [North, South, East, West, NorthEast, NorthWest, SouthEast, SouthWest] {
        let mut rt = Runtime::new(edge, resize_view, (100.0, 100.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt.pointer_move(20.0, 20.0);
        rt.pointer_down(20.0, 20.0);
        assert_eq!(rt.take_titlebar_action(), Some(TitlebarAction::Resize(edge)));
        assert_eq!(rt.take_titlebar_action(), None, "only the initiating event consumes it");
    }
}

#[test]
fn colour_cancel_discards_uncommitted_drafts_before_opening_another_colour() {
    let mut s = State::default();
    controls::open_color(&mut s, controls::ColorTarget::Foreground, ascii_painter_core::Rgb::WHITE);
    let mut rt = Runtime::new(s, ui::view, (900.0, 600.0), Env::new(true, 1.0));
    rt.poll = Some(app::poll);
    for _ in 0..4 {
        frame(&mut rt);
    }
    for name in ["colour-dialog", "color-apply", "color-cancel"] {
        let r = rt.presented(name).unwrap().0;
        assert!(r[1] >= 0.0 && r[1] + r[3] <= 600.1, "{name}: {r:?}");
    }
    click(&mut rt, "color-r-value");
    key(&mut rt, Key::Command('a'));
    key(&mut rt, Key::Text("99".into()));
    click(&mut rt, "color-cancel");
    assert!(rt.state.color.is_none());
    rt.update(|s, _| controls::open_color(s, controls::ColorTarget::Background, ascii_painter_core::Rgb::new(1, 2, 3)));
    frame(&mut rt);
    click(&mut rt, "color-apply");
    assert_eq!(rt.state.prefs.settings.color.bg, ascii_painter_core::Rgb::new(1, 2, 3));
}

#[derive(Clone, Copy, Debug)]
enum PolishScene {
    Empty,
    Image,
    Video,
    Exporting,
    Cancelling,
}

fn polish_state(scene: PolishScene) -> State {
    let mut s = State::default();
    match scene {
        PolishScene::Empty => {}
        PolishScene::Image => crate::scenes::image(&mut s),
        PolishScene::Video => crate::scenes::video(&mut s),
        PolishScene::Exporting | PolishScene::Cancelling => {
            crate::scenes::video(&mut s);
            s.export = Some(app::ExportState::fixture(ascii_painter_video::Progress::default()));
            s.export.as_mut().unwrap().cancelling = matches!(scene, PolishScene::Cancelling);
        }
    }
    s
}
fn settle(rt: &mut Runtime<State>, frames: usize) {
    for _ in 0..frames {
        frame(rt);
    }
}
fn polish_runtime(state: State, size: (f32, f32), scale: f32) -> Runtime<State> {
    let mut rt = Runtime::new(state, ui::view, size, Env::new(true, scale));
    rt.poll = Some(app::poll);
    rt.set_system_scroll_bars(rune::ScrollBars::Always);
    settle(&mut rt, 3);
    rt
}
fn element(rt: &Runtime<State>, key: &str) -> rune::ElementInfo {
    rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap_or_else(|| panic!("missing {key}"))
}
fn sidebar_thumb(rt: &Runtime<State>) -> rune::ElementInfo {
    rt.elements()
        .into_iter()
        .find(|e| e.path.ends_with("/settings-scroll-scroll-area/scroll-bar/thumb-slot/thumb"))
        .expect("the sidebar thumb, not a slider thumb")
}
fn sidebar_offset(rt: &Runtime<State>) -> f32 {
    let prefix = format!("{}/", element(rt, "settings-scroll").path);
    rt.elements().into_iter().filter(|e| e.path.starts_with(&prefix)).map(|e| e.scrolled[1]).fold(0.0, f32::max)
}
fn assert_color(actual: rune::Color, expected: rune::Color) {
    for (a, e) in
        [actual.r, actual.g, actual.b, actual.a].into_iter().zip([expected.r, expected.g, expected.b, expected.a])
    {
        assert!((a - e).abs() < 0.0001, "actual {actual:?}, expected {expected:?}");
    }
}
fn pointer_click(rt: &mut Runtime<State>, point: [f32; 2]) {
    rt.pointer_move(point[0], point[1]);
    rt.pointer_down(point[0], point[1]);
    rt.pointer_up(point[0], point[1]);
    frame(rt);
}
fn tab_focus(rt: &mut Runtime<State>, name: &str) {
    for _ in 0..128 {
        key(rt, Key::Tab);
        if element(rt, name).focus_visible {
            return;
        }
    }
    panic!("could not tab to {name}");
}

#[test]
fn ui_polish_action_sizes_and_labels_fit_every_scene_size_and_dpi() {
    for scene in
        [PolishScene::Empty, PolishScene::Image, PolishScene::Video, PolishScene::Exporting, PolishScene::Cancelling]
    {
        for size in [(900.0, 600.0), (1400.0, 900.0)] {
            for scale in [1.0, 1.5, 2.0] {
                let rt = polish_runtime(polish_state(scene), size, scale);
                for (key, expected) in [("open", [144.0, 40.0]), ("export", [180.0, 40.0])] {
                    let action = element(&rt, key);
                    assert_eq!([action.rect[2], action.rect[3]], expected, "{scene:?}, {size:?}, {scale}: {key}");
                    let label_path = format!("{}/label", action.path);
                    let label = rt.elements().into_iter().find(|e| e.path == label_path).unwrap();
                    assert!(label.rect[0] >= action.rect[0] && label.rect[1] >= action.rect[1]);
                    assert!(label.rect[0] + label.rect[2] <= action.rect[0] + action.rect[2] + 0.1);
                    assert!(label.rect[1] + label.rect[3] <= action.rect[1] + action.rect[3] + 0.1);
                }
                assert_eq!(element(&rt, "browse-output").rect[3], 28.0);
                assert_eq!(element(&rt, "toolbar").rect[3], 64.0);
                for key in ["titlebar", "toolbar", "open", "sidebar", "viewer", "export-bar", "export", "output-path"] {
                    let rect = rt.presented(key).unwrap().0;
                    assert!(
                        rect[0] >= -0.1
                            && rect[1] >= -0.1
                            && rect[0] + rect[2] <= size.0 + 0.1
                            && rect[1] + rect[3] <= size.1 + 0.1,
                        "{scene:?}: {key} {rect:?} outside {size:?} at {scale}"
                    );
                    assert!(rect[2] > 0.0 && rect[3] > 0.0);
                }
            }
        }
    }
}

#[test]
fn ui_polish_new_action_margins_are_clickable_without_starting_an_export() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.png");
    let output = dir.path().join("out.png");
    std::fs::write(&input, b"source identity").unwrap();
    std::fs::write(&output, b"old output").unwrap();
    let mut state = polish_state(PolishScene::Image);
    state.source.as_mut().unwrap().path = input;
    state.set_output_path(output.clone());
    let mut rt = polish_runtime(state, (900.0, 600.0), 1.0);
    let open = element(&rt, "open").rect;
    pointer_click(&mut rt, [open[0] + 140.0, open[1] + 20.0]);
    assert!(rt.file_dialog().is_some(), "the added right margin must open the dialog");
    rt.resolve_file_dialog(None).unwrap();
    frame(&mut rt);
    tab_focus(&mut rt, "open");
    key(&mut rt, Key::Text(" ".into()));
    assert!(rt.file_dialog().is_some(), "keyboard activation is unchanged");
    rt.resolve_file_dialog(None).unwrap();
    frame(&mut rt);
    tab_focus(&mut rt, "export");
    let focused = element(&rt, "export").rect;
    // Rune Kit's 2-point outline with a 2.5-point offset fits the footer padding.
    assert!(focused[0] >= 4.5 && focused[1] >= 4.5);
    assert!(focused[0] + focused[2] + 4.5 <= 900.0 && focused[1] + focused[3] + 4.5 <= 600.0);
    let export = element(&rt, "export").rect;
    pointer_click(&mut rt, [export[0] + export[2] - 176.0, export[1] + 20.0]);
    assert!(rt.state.overwrite.is_some(), "the added left margin must request confirmation");
    assert!(rt.state.export.is_none());
    assert_eq!(std::fs::read(output).unwrap(), b"old output");
}

#[test]
fn ui_polish_disabled_export_and_cancelling_actions_remain_inactive() {
    for scene in [PolishScene::Empty, PolishScene::Cancelling] {
        let mut rt = polish_runtime(polish_state(scene), (900.0, 600.0), 1.0);
        let export = element(&rt, "export").rect;
        pointer_click(&mut rt, [export[0] + export[2] - 176.0, export[1] + 20.0]);
        assert!(rt.state.overwrite.is_none() && rt.file_dialog().is_none());
        assert_eq!(rt.state.export.is_some(), matches!(scene, PolishScene::Cancelling));
    }
}

#[test]
fn ui_polish_sidebar_colors_dragging_and_rebuilds_preserve_scroll_behavior() {
    let mut rt = polish_runtime(State::default(), (900.0, 600.0), 1.0);
    settle(&mut rt, 40);
    let rest = theme::rgb(ascii_painter_core::Rgb { r: 0x82, g: 0x60, b: 0xae });
    let active = theme::rgb(ascii_painter_core::Rgb { r: 0x9c, g: 0x7a, b: 0xce });
    let thumb = sidebar_thumb(&rt);
    assert_color(thumb.fill, rest);
    assert_eq!(thumb.rect[2], 6.0);
    assert!(thumb.opacity > 0.99);
    let viewport = element(&rt, "settings-scroll").rect;
    let point = [viewport[0] + 5.0, viewport[1] + viewport[3] / 2.0];
    rt.pointer_move(point[0], point[1]);
    rt.wheel(point[0], point[1], -120.0);
    settle(&mut rt, 40);
    let offset = sidebar_offset(&rt);
    assert!(offset > 0.0);
    rt.update(|_, _| {});
    settle(&mut rt, 3);
    assert!((sidebar_offset(&rt) - offset).abs() < 0.01);
    let thumb = sidebar_thumb(&rt).rect;
    let (x, y) = (thumb[0] + thumb[2] / 2.0, thumb[1] + thumb[3] / 2.0);
    rt.pointer_move(x, y);
    settle(&mut rt, 40);
    assert_color(sidebar_thumb(&rt).fill, active);
    assert!((sidebar_thumb(&rt).rect[2] - 10.0).abs() < 0.01);
    rt.pointer_down(x, y);
    rt.pointer_move(x - 30.0, y + 20.0);
    frame(&mut rt);
    rt.pointer_move(x - 30.0, y + 40.0);
    settle(&mut rt, 40);
    assert_color(sidebar_thumb(&rt).fill, active);
    assert!(sidebar_offset(&rt) > offset);
    rt.pointer_up(x - 30.0, y + 40.0);
    rt.pointer_move(point[0], point[1]);
    settle(&mut rt, 40);
    assert_color(sidebar_thumb(&rt).fill, rest);
    assert!((sidebar_thumb(&rt).rect[2] - 6.0).abs() < 0.01);
    let luminance = |c: rune::Color| c.r * 0.2126 + c.g * 0.7152 + c.b * 0.0722;
    for bg in [theme::PANEL, theme::RAISED] {
        assert!((luminance(rest) + 0.05) / (luminance(bg) + 0.05) >= 3.0);
    }
}

#[test]
fn ui_polish_sidebar_retains_when_scrolling_reveal_and_fade() {
    let mut rt = polish_runtime(State::default(), (900.0, 600.0), 1.0);
    rt.set_system_scroll_bars(rune::ScrollBars::WhenScrolling);
    settle(&mut rt, 240);
    assert!(sidebar_thumb(&rt).opacity < 0.01);
    let viewport = element(&rt, "settings-scroll").rect;
    let (x, y) = (viewport[0] + 5.0, viewport[1] + viewport[3] / 2.0);
    rt.pointer_move(x, y);
    rt.wheel(x, y, -120.0);
    settle(&mut rt, 15);
    assert!(sidebar_thumb(&rt).opacity > 0.9);
    settle(&mut rt, 240);
    assert!(sidebar_thumb(&rt).opacity < 0.01);
}
