//! The conversion settings controls.

use ascii_painter_core::Rgb;
use ascii_painter_core::settings::{
    CharsetPreset, ColorMode, Control, Detail, Dither, MAX_COLUMNS, MIN_COLUMNS, MappingMode, PaletteKind, Polarity,
    RampOrder, Settings,
};
use eframe::egui::{self, CollapsingHeader, ComboBox, Slider, Ui};

/// Facts about the loaded input that change which controls are shown.
pub struct PanelFacts<'a> {
    pub is_video: bool,
    pub rejected_chars: &'a [char],
}

fn combo<T: Copy + PartialEq>(ui: &mut Ui, label: &str, value: &mut T, options: &[T], name: fn(T) -> &'static str) {
    ComboBox::from_label(label).selected_text(name(*value)).show_ui(ui, |ui| {
        for &option in options {
            ui.selectable_value(value, option, name(option));
        }
    });
}

fn color_button(ui: &mut Ui, label: &str, color: &mut Rgb) {
    ui.horizontal(|ui| {
        let mut rgb = [color.r, color.g, color.b];
        if ui.color_edit_button_srgb(&mut rgb).changed() {
            *color = Rgb::new(rgb[0], rgb[1], rgb[2]);
        }
        ui.label(label);
    });
}

fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    CollapsingHeader::new(title).default_open(true).show(ui, add);
}

pub fn settings_panel(ui: &mut Ui, s: &mut Settings, facts: &PanelFacts) {
    section(ui, "Grid", |ui| {
        ui.add(Slider::new(&mut s.grid.columns, MIN_COLUMNS..=MAX_COLUMNS).logarithmic(true).text("columns"));
        let mut custom = s.grid.cell_aspect_override.is_some();
        ui.checkbox(&mut custom, "Terminal cell aspect")
            .on_hover_text("Shape the grid for display in a terminal instead of the PNG/MP4 font.");
        match (custom, &mut s.grid.cell_aspect_override) {
            (true, Some(aspect)) => {
                ui.add(Slider::new(aspect, 0.3..=1.0).text("cell width ÷ height"));
            }
            (true, slot @ None) => *slot = Some(0.5),
            (false, slot) => *slot = None,
        }
    });

    section(ui, "Mapping", |ui| {
        combo(ui, "mode", &mut s.mapping, &MappingMode::ALL, MappingMode::label);
        ui.add_enabled_ui(s.relevant(Control::Charset), |ui| {
            combo(ui, "characters", &mut s.charset.preset, &CharsetPreset::ALL, CharsetPreset::label);
            if s.charset.preset == CharsetPreset::Custom {
                ui.add(
                    egui::TextEdit::singleline(&mut s.charset.custom)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("light → dense"),
                );
                ui.horizontal(|ui| {
                    ui.radio_value(&mut s.charset.order, RampOrder::ByDensity, "sort by ink");
                    ui.radio_value(&mut s.charset.order, RampOrder::AsWritten, "as written");
                });
            }
            if !facts.rejected_chars.is_empty() {
                let list: String = facts.rejected_chars.iter().map(|c| format!("{c:?} ")).collect();
                ui.colored_label(ui.visuals().warn_fg_color, format!("ignored: {list}"));
            }
        });
        ui.add_enabled_ui(s.relevant(Control::Dither), |ui| {
            combo(ui, "dithering", &mut s.dither, &Dither::ALL, Dither::label);
        });
    });

    section(ui, "Tone", |ui| {
        ui.horizontal(|ui| {
            ui.radio_value(&mut s.tone.detail, Detail::Full, "full image");
            ui.radio_value(&mut s.tone.detail, Detail::EdgesOnly, "edges only");
        });
        ui.add(Slider::new(&mut s.tone.brightness, -1.0..=1.0).text("brightness"));
        ui.add(Slider::new(&mut s.tone.contrast, 0.0..=3.0).text("contrast"));
        ui.add(Slider::new(&mut s.tone.gamma, 0.2..=5.0).logarithmic(true).text("gamma"));
        ui.add(Slider::new(&mut s.tone.saturation, 0.0..=3.0).text("saturation"));
        ui.add(Slider::new(&mut s.tone.sharpen, 0.0..=3.0).text("sharpen"));
        ui.add(Slider::new(&mut s.tone.edge_boost, 0.0..=2.0).text("edge boost"));
        ui.add(Slider::new(&mut s.tone.denoise, 0.0..=1.0).text("denoise"))
            .on_hover_text("Edge-preserving noise reduction; calms grain and video flicker.");
        ui.checkbox(&mut s.tone.auto_levels, "auto levels")
            .on_hover_text("Stretch the image's own darkest and brightest tones to the full range.");
        ui.add_enabled_ui(!s.tone.auto_levels, |ui| {
            ui.add(Slider::new(&mut s.tone.black_point, 0.0..=0.99).text("black point"));
            ui.add(Slider::new(&mut s.tone.white_point, 0.01..=1.0).text("white point"));
        });
        ui.add(Slider::new(&mut s.tone.shadows, -1.0..=1.0).text("shadows"));
        ui.add(Slider::new(&mut s.tone.highlights, -1.0..=1.0).text("highlights"));
        let mut inverted = s.tone.polarity == Polarity::Inverted;
        if ui.checkbox(&mut inverted, "invert").changed() {
            s.tone.polarity = if inverted { Polarity::Inverted } else { Polarity::Normal };
        }
        if ui.button("Reset tone").clicked() {
            s.tone = Default::default();
        }
    });

    section(ui, "Colour", |ui| {
        combo(ui, "colour mode", &mut s.color.mode, &ColorMode::ALL, ColorMode::label);
        if s.relevant(Control::Foreground) {
            color_button(ui, "text", &mut s.color.fg);
        }
        if s.relevant(Control::Background) {
            color_button(ui, "background", &mut s.color.bg);
        }
        if s.relevant(Control::Foreground) && ui.button("⇄ swap").clicked() {
            std::mem::swap(&mut s.color.fg, &mut s.color.bg);
        }
        ui.add_enabled_ui(s.relevant(Control::Palette), |ui| {
            combo(ui, "palette", &mut s.color.palette, &PaletteKind::ALL, PaletteKind::label);
            if s.color.palette == PaletteKind::Custom {
                let mut remove = None;
                ui.horizontal_wrapped(|ui| {
                    for (i, color) in s.color.custom_palette.iter_mut().enumerate() {
                        let mut rgb = [color.r, color.g, color.b];
                        let response = ui.color_edit_button_srgb(&mut rgb);
                        if response.changed() {
                            *color = Rgb::new(rgb[0], rgb[1], rgb[2]);
                        }
                        if response.secondary_clicked() {
                            remove = Some(i);
                        }
                    }
                    if ui.button("+").clicked() {
                        s.color.custom_palette.push(Rgb::new(128, 128, 128));
                    }
                });
                if let Some(i) = remove.filter(|_| s.color.custom_palette.len() > 1) {
                    s.color.custom_palette.remove(i);
                }
                ui.weak("right-click a colour to remove it");
            }
        });
        let mut keyed = s.color.chroma_key.is_some();
        ui.checkbox(&mut keyed, "chroma key")
            .on_hover_text("Replace a backdrop colour (green screen) with the background.");
        match (keyed, &mut s.color.chroma_key) {
            (true, Some(key)) => {
                color_button(ui, "key colour", key);
                ui.add(Slider::new(&mut s.color.key_tolerance, 0.0..=1.0).text("key tolerance"));
            }
            (true, slot @ None) => *slot = Some(Rgb::new(0, 255, 0)),
            (false, slot) => *slot = None,
        }
    });

    if facts.is_video {
        section(ui, "Video stability", |ui| {
            ui.add_enabled_ui(s.relevant(Control::Hysteresis), |ui| {
                ui.add(Slider::new(&mut s.temporal.hysteresis, 0.0..=1.0).text("glyph hysteresis")).on_hover_text(
                    "Keep the previous frame's character unless the new one fits clearly better. Reduces flicker.",
                );
            });
            ui.add_enabled_ui(s.relevant(Control::ColorSmoothing), |ui| {
                ui.add(Slider::new(&mut s.temporal.color_smoothing, 0.0..=0.95).text("colour smoothing"));
            });
            ui.weak("Temporal settings apply to exports; the preview shows a single frame.");
        });
    }

    section(ui, "Render", |ui| {
        ui.add(Slider::new(&mut s.render.font_px, 4.0..=64.0).text("font px (PNG / MP4)"));
        ui.add_enabled_ui(s.relevant(Control::TransparentBackground), |ui| {
            ui.checkbox(&mut s.render.transparent_background, "transparent background")
                .on_hover_text("PNG / WebP keep alpha; MP4, JPEG and BMP are composited onto the background.");
        });
    });
}
