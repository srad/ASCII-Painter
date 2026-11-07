//! The complete conversion settings surface. Relevance remains owned by the core crate.
use crate::{
    app::State,
    controls::{self, ColorTarget, Number, number},
    theme,
};
use ascii_painter_core::{
    Rgb,
    settings::{CharsetPreset, ColorMode, Control, Detail, Dither, MappingMode, PaletteKind, Polarity, RampOrder},
};
use rune::{El, col, el, row};
use rune_kit::{Button, ButtonVariant, Checkbox, Kit, Select, SelectAppearance, TextInput};

pub struct Choice<T: 'static> {
    pub key: &'static str,
    pub label: &'static str,
    pub current: T,
    pub options: &'static [T],
    pub name: fn(T) -> &'static str,
    pub set: fn(&mut State, T),
    pub enabled: bool,
}

pub fn choice<T: Copy + PartialEq + 'static>(s: &State, k: &Kit, overlays: &mut Vec<El>, choice: Choice<T>) -> El {
    let Choice { key, label, current, options, name, set, enabled } = choice;
    let labels: Vec<_> = options.iter().map(|v| name(*v)).collect();
    let select = Select::new(key, &labels)
        .label(label)
        .current(Some(name(current)))
        .width(200.0)
        .appearance(SelectAppearance::Field)
        .disabled(!enabled)
        .open(s.open_select.as_deref() == Some(key))
        .on_toggle(move |s: &mut State| {
            s.open_select = if s.open_select.as_deref() == Some(key) { None } else { Some(key.into()) };
        })
        .on_close(|s: &mut State| s.open_select = None)
        .on_choose(move |s: &mut State, value, _| {
            if let Some(&option) = options.iter().find(|v| name(**v) == value) {
                set(s, option);
            }
            s.open_select = None;
        })
        .build(k, overlays);
    col().w_full().gap(6.0).child(theme::muted(label)).child(select.w_full())
}

fn check(
    k: &Kit,
    key: &'static str,
    label: &'static str,
    checked: bool,
    enabled: bool,
    f: impl Fn(&mut State) + 'static,
) -> El {
    Checkbox::new(checked).label(label).disabled(!enabled).on_toggle(f).build(k).key(key)
}

fn section(s: &State, k: &Kit, title: &'static str, body: El) -> El {
    let closed = s.collapsed.contains(title);
    let header = Button::new(format!("{}  {title}", if closed { "+" } else { "−" }))
        .variant(ButtonVariant::GhostMuted)
        .on_press(move |s: &mut State| {
            if !s.collapsed.remove(title) {
                s.collapsed.insert(title);
            }
        })
        .build(k)
        .w_full();
    col()
        .key(title)
        .w_full()
        .gap(8.0)
        .padding(12.0)
        .radius(10.0)
        .fill(theme::RAISED)
        .border(1.0, theme::BORDER)
        .child(header)
        .children((!closed).then_some(body))
}

fn color_button(_k: &Kit, key: &str, label: &str, rgb: Rgb, target: ColorTarget) -> El {
    row()
        .gap(10.0)
        .items_center()
        .child(
            el().key(key)
                .size(32.0, 24.0)
                .radius(5.0)
                .fill(theme::rgb(rgb))
                .border(1.0, theme::BORDER)
                .role("button")
                .label(format!("Edit {label} colour"))
                .focusable(true)
                .on_press(move |s: &mut State| controls::open_color(s, target, rgb)),
        )
        .child(theme::label(label))
}

pub fn panel(s: &State, k: &Kit, overlays: &mut Vec<El>) -> El {
    let p = &s.prefs.settings;
    let grid = col()
        .w_full()
        .gap(10.0)
        .child(number(s, k, "columns", "Columns", Number::Columns, true))
        .child(check(k, "terminal-aspect", "Terminal cell aspect", p.grid.cell_aspect_override.is_some(), true, |s| {
            let slot = &mut s.prefs.settings.grid.cell_aspect_override;
            *slot = if slot.is_some() { None } else { Some(0.5) };
        }))
        .children(
            p.grid.cell_aspect_override.map(|_| number(s, k, "aspect", "Cell width ÷ height", Number::Aspect, true)),
        );

    let mut mapping = col()
        .w_full()
        .gap(10.0)
        .child(choice(
            s,
            k,
            overlays,
            Choice {
                key: "mapping",
                label: "Mapping mode",
                current: p.mapping,
                options: &MappingMode::ALL,
                name: MappingMode::label,
                set: |s, v| s.prefs.settings.mapping = v,
                enabled: true,
            },
        ))
        .child(choice(
            s,
            k,
            overlays,
            Choice {
                key: "charset",
                label: "Characters",
                current: p.charset.preset,
                options: &CharsetPreset::ALL,
                name: CharsetPreset::label,
                set: |s, v| s.prefs.settings.charset.preset = v,
                enabled: p.relevant(Control::Charset),
            },
        ));
    if p.charset.preset == CharsetPreset::Custom && p.relevant(Control::Charset) {
        mapping = mapping
            .child(
                TextInput::new(&s.charset)
                    .label("Custom character ramp")
                    .placeholder("light → dense")
                    .on_edit(|s: &mut State| &mut s.charset)
                    .on_change(|s: &mut State, _| s.prefs.settings.charset.custom = s.charset.text.clone())
                    .build(k)
                    .key("custom-charset")
                    .w_full(),
            )
            .child(
                row()
                    .gap(6.0)
                    .child(
                        Button::new("Sort by ink")
                            .variant(if p.charset.order == RampOrder::ByDensity {
                                ButtonVariant::Outline
                            } else {
                                ButtonVariant::Ghost
                            })
                            .on_press(|s: &mut State| s.prefs.settings.charset.order = RampOrder::ByDensity)
                            .build(k),
                    )
                    .child(
                        Button::new("As written")
                            .variant(if p.charset.order == RampOrder::AsWritten {
                                ButtonVariant::Outline
                            } else {
                                ButtonVariant::Ghost
                            })
                            .on_press(|s: &mut State| s.prefs.settings.charset.order = RampOrder::AsWritten)
                            .build(k),
                    ),
            );
    }
    if !s.preview.rejected.is_empty() {
        mapping = mapping.child(theme::muted(format!("Ignored: {:?}", s.preview.rejected)));
    }
    mapping = mapping.child(choice(
        s,
        k,
        overlays,
        Choice {
            key: "dither",
            label: "Dithering",
            current: p.dither,
            options: &Dither::ALL,
            name: Dither::label,
            set: |s, v| s.prefs.settings.dither = v,
            enabled: p.relevant(Control::Dither),
        },
    ));

    let mut tone = col().w_full().gap(8.0).child(
        row()
            .gap(6.0)
            .child(
                Button::new("Full image")
                    .variant(if p.tone.detail == Detail::Full { ButtonVariant::Outline } else { ButtonVariant::Ghost })
                    .on_press(|s: &mut State| s.prefs.settings.tone.detail = Detail::Full)
                    .build(k),
            )
            .child(
                Button::new("Edges only")
                    .variant(if p.tone.detail == Detail::EdgesOnly {
                        ButtonVariant::Outline
                    } else {
                        ButtonVariant::Ghost
                    })
                    .on_press(|s: &mut State| s.prefs.settings.tone.detail = Detail::EdgesOnly)
                    .build(k),
            ),
    );
    for (key, label, n) in [
        ("brightness", "Brightness", Number::Brightness),
        ("contrast", "Contrast", Number::Contrast),
        ("gamma", "Gamma", Number::Gamma),
        ("saturation", "Saturation", Number::Saturation),
        ("sharpen", "Sharpen", Number::Sharpen),
        ("edge-boost", "Edge boost", Number::EdgeBoost),
        ("denoise", "Denoise", Number::Denoise),
    ] {
        tone = tone.child(number(s, k, key, label, n, true));
    }
    tone = tone.child(check(k, "auto-levels", "Auto levels", p.tone.auto_levels, true, |s| {
        s.prefs.settings.tone.auto_levels = !s.prefs.settings.tone.auto_levels
    }));
    for (key, label, n, enabled) in [
        ("black", "Black point", Number::Black, !p.tone.auto_levels),
        ("white", "White point", Number::White, !p.tone.auto_levels),
        ("shadows", "Shadows", Number::Shadows, true),
        ("highlights", "Highlights", Number::Highlights, true),
    ] {
        tone = tone.child(number(s, k, key, label, n, enabled));
    }
    tone = tone
        .child(check(k, "invert", "Invert", p.tone.polarity == Polarity::Inverted, true, |s| {
            let p = &mut s.prefs.settings.tone.polarity;
            *p = if *p == Polarity::Normal { Polarity::Inverted } else { Polarity::Normal };
        }))
        .child(
            Button::new("Reset tone")
                .on_press(|s: &mut State| s.prefs.settings.tone = Default::default())
                .build(k)
                .key("reset-tone"),
        );

    let mut color = col().w_full().gap(10.0).child(choice(
        s,
        k,
        overlays,
        Choice {
            key: "color-mode",
            label: "Colour mode",
            current: p.color.mode,
            options: &ColorMode::ALL,
            name: ColorMode::label,
            set: |s, v| s.prefs.settings.color.mode = v,
            enabled: true,
        },
    ));
    if p.relevant(Control::Foreground) {
        color = color.child(color_button(k, "foreground", "Text", p.color.fg, ColorTarget::Foreground));
    }
    if p.relevant(Control::Background) {
        color = color.child(color_button(k, "background", "Background", p.color.bg, ColorTarget::Background));
    }
    if p.relevant(Control::Foreground) {
        color = color.child(
            Button::new("Swap text / background")
                .on_press(|s: &mut State| {
                    let c = &mut s.prefs.settings.color;
                    std::mem::swap(&mut c.fg, &mut c.bg);
                })
                .build(k),
        );
    }
    color = color.child(choice(
        s,
        k,
        overlays,
        Choice {
            key: "palette",
            label: "Palette",
            current: p.color.palette,
            options: &PaletteKind::ALL,
            name: PaletteKind::label,
            set: |s, v| s.prefs.settings.color.palette = v,
            enabled: p.relevant(Control::Palette),
        },
    ));
    if p.color.palette == PaletteKind::Custom && p.relevant(Control::Palette) {
        let mut palette = col().w_full().gap(6.0);
        let mut swatches = row().w_full().gap(6.0);
        for (i, &rgb) in p.color.custom_palette.iter().enumerate() {
            if i > 0 && i % 7 == 0 {
                palette = palette.child(swatches);
                swatches = row().w_full().gap(6.0);
            }
            swatches = swatches.child(
                el().key(format!("palette-{i}"))
                    .size(27.0, 25.0)
                    .radius(5.0)
                    .fill(theme::rgb(rgb))
                    .role("button")
                    .label(format!("Palette colour {}", i + 1))
                    .focusable(true)
                    .help("Edit colour; right-click to remove")
                    .on_press(move |s: &mut State| controls::open_color(s, ColorTarget::Palette(i), rgb))
                    .on_secondary_press(move |s: &mut State, _, _| {
                        if s.prefs.settings.color.custom_palette.len() > 1 {
                            s.prefs.settings.color.custom_palette.remove(i);
                        }
                    }),
            );
        }
        color = color
            .child(
                palette.child(swatches).child(
                    Button::new("+")
                        .on_press(|s: &mut State| s.prefs.settings.color.custom_palette.push(Rgb::new(128, 128, 128)))
                        .build(k),
                ),
            )
            .child(theme::muted("Right-click a colour to remove it."));
    }
    color = color.child(check(k, "chroma-key", "Chroma key", p.color.chroma_key.is_some(), true, |s| {
        let key = &mut s.prefs.settings.color.chroma_key;
        *key = if key.is_some() { None } else { Some(Rgb::new(0, 255, 0)) };
    }));
    if let Some(key) = p.color.chroma_key {
        color = color.child(color_button(k, "chroma-color", "Key colour", key, ColorTarget::ChromaKey)).child(number(
            s,
            k,
            "key-tolerance",
            "Key tolerance",
            Number::Tolerance,
            true,
        ));
    }
    let render =
        col().w_full().gap(10.0).child(number(s, k, "font-size", "Font pixels (PNG / MP4)", Number::Font, true)).child(
            check(
                k,
                "transparent",
                "Transparent background",
                p.render.transparent_background,
                p.relevant(Control::TransparentBackground),
                |s| s.prefs.settings.render.transparent_background = !s.prefs.settings.render.transparent_background,
            ),
        );

    let mut panel = col()
        .w_full()
        .gap(12.0)
        .child(section(s, k, "GRID", grid))
        .child(section(s, k, "MAPPING", mapping))
        .child(section(s, k, "TONE", tone))
        .child(section(s, k, "COLOUR", color));
    if s.is_video() {
        let temporal = col()
            .w_full()
            .gap(8.0)
            .child(number(s, k, "hysteresis", "Glyph hysteresis", Number::Hysteresis, p.relevant(Control::Hysteresis)))
            .child(number(
                s,
                k,
                "smoothing",
                "Colour smoothing",
                Number::Smoothing,
                p.relevant(Control::ColorSmoothing),
            ))
            .child(theme::muted("Temporal settings apply to exports. Preview shows a single frame."));
        panel = panel.child(section(s, k, "VIDEO STABILITY", temporal));
    }
    panel.child(section(s, k, "RENDER", render))
}
