//! Reusable numeric and RGB controls missing from Rune Kit.
use crate::{app::State, theme};
use ascii_painter_core::{
    Rgb,
    settings::{MAX_COLUMNS, MIN_COLUMNS},
};
use rune::{El, Key, Phase, ScrollSpec, TextEdit, col, el, row};
use rune_kit::{Button, ButtonVariant, Dialog, Kit, ScrollBar, TextInput};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Number {
    Columns,
    Aspect,
    Brightness,
    Contrast,
    Gamma,
    Saturation,
    Sharpen,
    EdgeBoost,
    Denoise,
    Black,
    White,
    Shadows,
    Highlights,
    Tolerance,
    Hysteresis,
    Smoothing,
    Font,
    Seek,
    Start,
    End,
    Red,
    Green,
    Blue,
    Hue,
    Sat,
    Value,
}
impl Number {
    const ALL: [Self; 26] = [
        Self::Columns,
        Self::Aspect,
        Self::Brightness,
        Self::Contrast,
        Self::Gamma,
        Self::Saturation,
        Self::Sharpen,
        Self::EdgeBoost,
        Self::Denoise,
        Self::Black,
        Self::White,
        Self::Shadows,
        Self::Highlights,
        Self::Tolerance,
        Self::Hysteresis,
        Self::Smoothing,
        Self::Font,
        Self::Seek,
        Self::Start,
        Self::End,
        Self::Red,
        Self::Green,
        Self::Blue,
        Self::Hue,
        Self::Sat,
        Self::Value,
    ];
    pub fn read(self, s: &State) -> f64 {
        let p = &s.prefs.settings;
        f64::from(match self {
            Self::Columns => p.grid.columns as f32,
            Self::Aspect => p.grid.cell_aspect_override.unwrap_or(0.5),
            Self::Brightness => p.tone.brightness,
            Self::Contrast => p.tone.contrast,
            Self::Gamma => p.tone.gamma,
            Self::Saturation => p.tone.saturation,
            Self::Sharpen => p.tone.sharpen,
            Self::EdgeBoost => p.tone.edge_boost,
            Self::Denoise => p.tone.denoise,
            Self::Black => p.tone.black_point,
            Self::White => p.tone.white_point,
            Self::Shadows => p.tone.shadows,
            Self::Highlights => p.tone.highlights,
            Self::Tolerance => p.color.key_tolerance,
            Self::Hysteresis => p.temporal.hysteresis,
            Self::Smoothing => p.temporal.color_smoothing,
            Self::Font => p.render.font_px,
            Self::Seek => return s.video().map_or(0.0, |v| v.time),
            Self::Start => return s.video().map_or(0.0, |v| v.start),
            Self::End => return s.video().map_or(0.0, |v| v.end),
            Self::Red => s.color.as_ref().map_or(0.0, |c| f32::from(c.current.r)),
            Self::Green => s.color.as_ref().map_or(0.0, |c| f32::from(c.current.g)),
            Self::Blue => s.color.as_ref().map_or(0.0, |c| f32::from(c.current.b)),
            Self::Hue => s.color.as_ref().map_or(0.0, |c| c.hsv[0]),
            Self::Sat => s.color.as_ref().map_or(0.0, |c| c.hsv[1] * 100.0),
            Self::Value => s.color.as_ref().map_or(0.0, |c| c.hsv[2] * 100.0),
        })
    }
    pub fn spec(self, s: &State) -> NumericSpec {
        let (min, max, step, decimals, log) = match self {
            Self::Columns => (MIN_COLUMNS as f64, MAX_COLUMNS as f64, 1.0, 0, true),
            Self::Aspect => (0.3, 1.0, 0.01, 2, false),
            Self::Brightness | Self::Shadows | Self::Highlights => (-1.0, 1.0, 0.01, 2, false),
            Self::Contrast | Self::Saturation | Self::Sharpen => (0.0, 3.0, 0.01, 2, false),
            Self::Gamma => (0.2, 5.0, 0.01, 2, true),
            Self::EdgeBoost => (0.0, 2.0, 0.01, 2, false),
            Self::Black => (0.0, 0.99, 0.01, 2, false),
            Self::White => (0.01, 1.0, 0.01, 2, false),
            Self::Smoothing => (0.0, 0.95, 0.01, 2, false),
            Self::Font => (4.0, 64.0, 0.5, 1, false),
            Self::Seek | Self::Start | Self::End => (
                0.0,
                s.video().and_then(|v| v.info.duration).unwrap_or_else(|| self.read(s).max(3600.0)).max(0.1),
                0.1,
                1,
                false,
            ),
            Self::Red | Self::Green | Self::Blue => (0.0, 255.0, 1.0, 0, false),
            Self::Hue => (0.0, 360.0, 1.0, 0, false),
            Self::Sat | Self::Value => (0.0, 100.0, 1.0, 0, false),
            _ => (0.0, 1.0, 0.01, 2, false),
        };
        NumericSpec { min, max, step, decimals, logarithmic: log }
    }
    pub fn set(self, s: &mut State, value: f64) {
        if !value.is_finite() {
            return;
        }
        let spec = self.spec(s);
        let value = value.clamp(spec.min, spec.max);
        let p = &mut s.prefs.settings;
        match self {
            Self::Columns => p.grid.columns = value.round() as u32,
            Self::Aspect => p.grid.cell_aspect_override = Some(value as f32),
            Self::Brightness => p.tone.brightness = value as f32,
            Self::Contrast => p.tone.contrast = value as f32,
            Self::Gamma => p.tone.gamma = value as f32,
            Self::Saturation => p.tone.saturation = value as f32,
            Self::Sharpen => p.tone.sharpen = value as f32,
            Self::EdgeBoost => p.tone.edge_boost = value as f32,
            Self::Denoise => p.tone.denoise = value as f32,
            Self::Black => p.tone.black_point = (value as f32).min(p.tone.white_point - 0.01),
            Self::White => p.tone.white_point = (value as f32).max(p.tone.black_point + 0.01),
            Self::Shadows => p.tone.shadows = value as f32,
            Self::Highlights => p.tone.highlights = value as f32,
            Self::Tolerance => p.color.key_tolerance = value as f32,
            Self::Hysteresis => p.temporal.hysteresis = value as f32,
            Self::Smoothing => p.temporal.color_smoothing = value as f32,
            Self::Font => p.render.font_px = value as f32,
            Self::Seek => {
                if let Some(v) = s.video_mut() {
                    v.time = value;
                }
                s.dirty();
            }
            Self::Start => {
                if let Some(v) = s.video_mut() {
                    v.start = if v.end > 0.0 { value.min((v.end - 0.001).max(0.0)) } else { value };
                }
            }
            Self::End => {
                if let Some(v) = s.video_mut() {
                    v.end = value.max(v.start + 0.001);
                }
            }
            Self::Red | Self::Green | Self::Blue => {
                if let Some(c) = &mut s.color {
                    match self {
                        Self::Red => c.current.r = value.round() as u8,
                        Self::Green => c.current.g = value.round() as u8,
                        _ => c.current.b = value.round() as u8,
                    }
                    c.hsv = rgb_to_hsv(c.current);
                    c.sync_hex();
                }
            }
            Self::Hue | Self::Sat | Self::Value => {
                if let Some(c) = &mut s.color {
                    let i = match self {
                        Self::Hue => 0,
                        Self::Sat => 1,
                        _ => 2,
                    };
                    c.hsv[i] = value as f32 / if i == 0 { 1.0 } else { 100.0 };
                    c.current = hsv_to_rgb(c.hsv);
                    c.sync_hex();
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct NumericSpec {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub decimals: usize,
    pub logarithmic: bool,
}
impl NumericSpec {
    pub fn fraction(self, value: f64) -> f32 {
        let v = value.clamp(self.min, self.max);
        (if self.logarithmic {
            (v / self.min).ln() / (self.max / self.min).ln()
        } else {
            (v - self.min) / (self.max - self.min)
        }) as f32
    }
    pub fn at(self, fraction: f32) -> f64 {
        let f = f64::from(fraction.clamp(0.0, 1.0));
        let v = if self.logarithmic {
            self.min * (self.max / self.min).powf(f)
        } else {
            self.min + f * (self.max - self.min)
        };
        ((v / self.step).round() * self.step).clamp(self.min, self.max)
    }
    pub fn parse(self, text: &str) -> Option<f64> {
        text.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= self.min && *v <= self.max)
    }
    fn format(self, value: f64) -> String {
        format!("{:.*}", self.decimals, value)
    }
}
pub struct NumericDraft {
    pub edit: TextEdit,
    pub invalid: bool,
    was_focused: bool,
    pub dirty: bool,
}

pub fn sync_numbers(s: &mut State) {
    for n in Number::ALL {
        let value = n.read(s);
        let spec = n.spec(s);
        let draft = s.numbers.entry(n).or_insert_with(|| NumericDraft {
            edit: TextEdit::new(spec.format(value)),
            invalid: false,
            was_focused: false,
            dirty: false,
        });
        if !draft.edit.focused && !draft.dirty {
            draft.edit = TextEdit::new(spec.format(value));
            draft.invalid = false;
        }
    }
}
pub fn commit_number(s: &mut State, number: Number) -> bool {
    let spec = number.spec(s);
    let Some(draft) = s.numbers.get(&number) else { return false };
    if !draft.dirty {
        return false;
    }
    let value = spec.parse(&draft.edit.text);
    if let Some(value) = value {
        number.set(s, value);
    }
    let formatted = spec.format(number.read(s));
    if let Some(draft) = s.numbers.get_mut(&number) {
        draft.edit.replace_text(formatted);
        draft.invalid = false;
        draft.dirty = false;
    }
    value.is_some()
}
pub fn commit_all(s: &mut State) {
    for n in Number::ALL {
        commit_number(s, n);
    }
}
pub fn poll_edits(s: &mut State) -> bool {
    let mut blur = Vec::new();
    for (&number, draft) in &mut s.numbers {
        if draft.was_focused && !draft.edit.focused {
            blur.push(number);
        }
        draft.was_focused = draft.edit.focused;
    }
    let mut changed = false;
    for n in blur {
        changed |= commit_number(s, n);
    }
    changed
}

pub fn number(s: &State, kit: &Kit, key: &'static str, label: &'static str, n: Number, enabled: bool) -> El {
    let value = n.read(s);
    let spec = n.spec(s);
    let f = spec.fraction(value);
    let draft = &s.numbers[&n];
    let field = TextInput::new(&draft.edit)
        .label(label)
        .width(Some(72.0))
        .height(26.0)
        .invalid(draft.invalid)
        .disabled(!enabled)
        .on_edit(move |s: &mut State| &mut s.numbers.get_mut(&n).unwrap().edit)
        .on_change(move |s: &mut State, tx| {
            let spec = n.spec(s);
            let d = s.numbers.get_mut(&n).unwrap();
            d.dirty = true;
            d.invalid = spec.parse(&d.edit.text).is_none();
            tx.wake_after(Duration::from_millis(16));
        })
        .on_submit(move |s: &mut State, tx| {
            commit_number(s, n);
            tx.blur();
        })
        .on_key(move |s: &mut State, e, tx| {
            if e.key != Key::Escape {
                return false;
            }
            let text = n.spec(s).format(n.read(s));
            let d = s.numbers.get_mut(&n).unwrap();
            d.edit.replace_text(text);
            d.dirty = false;
            d.invalid = false;
            tx.blur();
            true
        })
        .build(kit)
        .key(format!("{key}-value"));
    let track = el()
        .key(key)
        .role("slider")
        .label(label)
        .focusable(enabled)
        .disabled(!enabled)
        .h(20.0)
        .w_full()
        .range_value(value, spec.min, spec.max)
        .range_step(spec.step)
        .on_drag(move |s: &mut State, d, _| {
            if !enabled {
                return;
            }
            if d.phase == Phase::Down {
                d.data = n.read(s) as f32;
            }
            if d.cancelled() {
                n.set(s, f64::from(d.data));
            } else {
                n.set(s, n.spec(s).at(d.x / d.w.max(1.0)));
            }
        })
        .on_step(move |s: &mut State, steps, _| {
            if enabled {
                n.set(s, n.read(s) + n.spec(s).step * f64::from(steps));
            }
        })
        .on_step_limit(move |s: &mut State, end, _| {
            if enabled {
                let spec = n.spec(s);
                n.set(s, if end { spec.max } else { spec.min });
            }
        })
        .child(el().pin(Some(0.0), Some(8.0), Some(0.0), None).h(4.0).radius(2.0).fill(theme::BORDER))
        .child(el().absolute(0.0, 8.0).w_frac(f).h(4.0).radius(2.0).fill(theme::ACCENT))
        .child(
            el().absolute(0.0, 0.0)
                .w_frac(f)
                .h(20.0)
                .child(el().pin(None, Some(4.0), Some(-6.0), None).size(12.0, 12.0).radius(6.0).fill(theme::TEXT)),
        );
    col()
        .w_full()
        .gap(3.0)
        .dim(if enabled { 1.0 } else { 0.45 })
        .child(row().w_full().items_center().child(theme::label(label).grow(1.0)).child(field))
        .child(track)
}

#[derive(Clone, Copy)]
pub enum ColorTarget {
    Foreground,
    Background,
    ChromaKey,
    Palette(usize),
}
pub struct ColorEditor {
    pub target: ColorTarget,
    pub current: Rgb,
    pub original: Rgb,
    pub hsv: [f32; 3],
    pub hex: TextEdit,
    pub invalid: bool,
}
impl ColorEditor {
    fn sync_hex(&mut self) {
        self.hex.replace_text(format!("#{:02X}{:02X}{:02X}", self.current.r, self.current.g, self.current.b));
        self.invalid = false;
    }
}
pub fn open_color(s: &mut State, target: ColorTarget, color: Rgb) {
    if s.blocked() {
        return;
    }
    let mut editor = ColorEditor {
        target,
        current: color,
        original: color,
        hsv: rgb_to_hsv(color),
        hex: TextEdit::default(),
        invalid: false,
    };
    editor.sync_hex();
    s.color = Some(editor);
    s.open_select = None;
    for n in [Number::Red, Number::Green, Number::Blue, Number::Hue, Number::Sat, Number::Value] {
        s.numbers
            .insert(n, NumericDraft { edit: TextEdit::default(), invalid: false, was_focused: false, dirty: false });
    }
    sync_numbers(s);
}
fn parse_hex(text: &str) -> Option<Rgb> {
    let text = text.trim().trim_start_matches('#');
    if text.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(text, 16).ok()?;
    Some(Rgb::new((value >> 16) as u8, (value >> 8) as u8, value as u8))
}
fn commit_hex(s: &mut State) {
    if let Some(c) = &mut s.color {
        if let Some(color) = parse_hex(&c.hex.text) {
            c.current = color;
            c.hsv = rgb_to_hsv(color);
            c.invalid = false;
        } else {
            c.invalid = true;
        }
    }
}
pub fn color_dialog(s: &State, kit: &Kit, overlays: &mut Vec<El>) {
    let Some(color) = &s.color else { return };
    let mut body = col().padding(16.0).gap(8.0).w_full().child(
        row()
            .gap(8.0)
            .child(el().size(60.0, 42.0).radius(6.0).fill(theme::rgb(color.original)))
            .child(el().size(60.0, 42.0).radius(6.0).fill(theme::rgb(color.current)))
            .child(
                TextInput::new(&color.hex)
                    .label("HEX colour")
                    .width(Some(150.0))
                    .invalid(color.invalid)
                    .on_edit(|s: &mut State| &mut s.color.as_mut().unwrap().hex)
                    .on_change(|s: &mut State, _| commit_hex(s))
                    .on_submit(|s: &mut State, tx| {
                        commit_hex(s);
                        tx.blur();
                    })
                    .build(kit)
                    .key("color-hex"),
            ),
    );
    for (key, label, n) in [
        ("color-r", "Red", Number::Red),
        ("color-g", "Green", Number::Green),
        ("color-b", "Blue", Number::Blue),
        ("color-h", "Hue", Number::Hue),
        ("color-s", "Saturation %", Number::Sat),
        ("color-v", "Value %", Number::Value),
    ] {
        body = body.child(number(s, kit, key, label, n, true));
    }
    body = col()
        .w_full()
        .child(
            body.key("color-scroll")
                .min_h(0.0)
                .max_h((s.viewport_size[1] - 190.0).max(140.0))
                .scroll(ScrollSpec { stick_bottom: false })
                .scroll_bar(kit),
        )
        .child(
            row()
                .gap(8.0)
                .padding(16.0)
                .child(Button::new("Cancel").on_press(|s: &mut State| s.color = None).build(kit).key("color-cancel"))
                .child(
                    Button::new("Apply colour")
                        .variant(ButtonVariant::Submit)
                        .disabled(color.invalid)
                        .on_press(|s: &mut State| {
                            commit_all(s);
                            commit_hex(s);
                            if s.color.as_ref().is_some_and(|c| c.invalid) {
                                return;
                            }
                            if let Some(c) = s.color.take() {
                                let p = &mut s.prefs.settings.color;
                                match c.target {
                                    ColorTarget::Foreground => p.fg = c.current,
                                    ColorTarget::Background => p.bg = c.current,
                                    ColorTarget::ChromaKey => p.chroma_key = Some(c.current),
                                    ColorTarget::Palette(i) => {
                                        if let Some(p) = p.custom_palette.get_mut(i) {
                                            *p = c.current;
                                        }
                                    }
                                }
                            }
                        })
                        .build(kit)
                        .key("color-apply"),
                ),
        );
    Dialog::new("colour", "Edit colour")
        .open(true)
        .fit()
        .width(440.0)
        .click_outside(false)
        .on_close(|s: &mut State| s.color = None)
        .child(body)
        .build(kit, overlays);
}

pub fn rgb_to_hsv(rgb: Rgb) -> [f32; 3] {
    let [r, g, b] = [rgb.r, rgb.g, rgb.b].map(|v| f32::from(v) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h * 60.0, if max == 0.0 { 0.0 } else { d / max }, max]
}
pub fn hsv_to_rgb([h, s, v]: [f32; 3]) -> Rgb {
    let c = v * s;
    let h = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let rgb = match h as u32 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    let [r, g, b] = rgb.map(|a| ((a + v - c).clamp(0.0, 1.0) * 255.0).round() as u8);
    Rgb::new(r, g, b)
}
