//! Source-aspect-aligned comparison and camera geometry, independent of texture resolution.
use crate::{app::State, theme};
use ascii_painter_core::Size2;
use image::RgbaImage;
use rune::{El, Phase, col, el, text};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    Original,
    Ascii,
    SideBySide,
    #[default]
    Split,
}
impl ViewMode {
    pub const ALL: [Self; 4] = [Self::Original, Self::Ascii, Self::SideBySide, Self::Split];
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Ascii => "ASCII",
            Self::SideBySide => "Side by side",
            Self::Split => "Split",
        }
    }
}

/// Immutable premultiplied-sRGB mip pyramid, prepared on a worker, uploaded only once.
pub struct ViewerImage {
    pub id: u64,
    pub levels: Vec<RgbaImage>,
}
impl ViewerImage {
    pub fn new(mut image: RgbaImage) -> Arc<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        for p in image.pixels_mut() {
            let a = u16::from(p[3]);
            for c in &mut p.0[..3] {
                *c = ((u16::from(*c) * a + 127) / 255) as u8;
            }
        }
        let mut levels = vec![image];
        loop {
            let last = levels.last().unwrap();
            if last.width() == 1 && last.height() == 1 {
                break;
            }
            levels.push(image::imageops::resize(
                last,
                (last.width() / 2).max(1),
                (last.height() / 2).max(1),
                image::imageops::FilterType::Triangle,
            ));
        }
        Arc::new(Self { id: NEXT.fetch_add(1, Ordering::Relaxed), levels })
    }
}

pub struct ViewerState {
    pub mode: ViewMode,
    pub zoom: f32,
    pub pan: [f32; 2],
    pub split: f32,
    pub size: [f32; 2],
    start_pan: [f32; 2],
    start_split: f32,
    divider: bool,
}
impl Default for ViewerState {
    fn default() -> Self {
        Self {
            mode: ViewMode::default(),
            zoom: 1.0,
            pan: [0.0; 2],
            split: 0.5,
            size: [800.0, 600.0],
            start_pan: [0.0; 2],
            start_split: 0.5,
            divider: false,
        }
    }
}
impl ViewerState {
    pub fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = [0.0; 2];
    }
    pub fn zoom_at(&mut self, pointer: [f32; 2], factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let next = (self.zoom * factor).clamp(0.1, 50.0);
        let ratio = next / self.zoom;
        let centre_x = if self.mode == ViewMode::SideBySide {
            self.size[0] * if pointer[0] < self.size[0] * 0.5 { 0.25 } else { 0.75 }
        } else {
            self.size[0] * 0.5
        };
        for (i, &coordinate) in pointer.iter().enumerate() {
            let anchor = coordinate - if i == 0 { centre_x } else { self.size[i] * 0.5 };
            self.pan[i] = anchor - (anchor - self.pan[i]) * ratio;
        }
        self.zoom = next;
    }
    pub fn drag(&mut self, d: &rune::Drag) {
        if d.phase == Phase::Down {
            self.start_pan = self.pan;
            self.start_split = self.split;
            self.divider = self.mode == ViewMode::Split && (d.x - self.size[0] * self.split).abs() <= 8.0;
        }
        if d.cancelled() {
            self.pan = self.start_pan;
            self.split = self.start_split;
            return;
        }
        if self.divider {
            self.split = (d.x / self.size[0].max(1.0)).clamp(0.0, 1.0);
        } else {
            self.pan = [self.start_pan[0] + d.dx, self.start_pan[1] + d.dy];
        }
    }
}

/// `[x, y, width, height]` in the area's local points.
pub fn image_rect(area: [f32; 4], source: Size2, zoom: f32, pan: [f32; 2]) -> [f32; 4] {
    let scale = (area[2] / source.w.max(1) as f32).min(area[3] / source.h.max(1) as f32) * zoom;
    let (w, h) = (source.w as f32 * scale, source.h as f32 * scale);
    [area[0] + (area[2] - w) * 0.5 + pan[0], area[1] + (area[3] - h) * 0.5 + pan[1], w, h]
}

fn image_slot(mark: u32, rect: [f32; 4]) -> El {
    el().key(if mark == 1 { "original-image" } else { "ascii-image" })
        .absolute(rect[0], rect[1])
        .size(rect[2], rect[3])
        .effect(mark)
}
fn caption(label: &str, x: f32) -> El {
    text(label)
        .font(theme::small())
        .color(theme::TEXT)
        .padding(7.0)
        .fill(theme::PANEL.alpha(0.9))
        .radius(5.0)
        .absolute(x + 14.0, 14.0)
}

pub fn view(s: &State) -> El {
    let v = &s.viewer;
    let [w, h] = v.size;
    let mut canvas = el()
        .key("viewer")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .h_full()
        .fill(theme::CANVAS)
        .clip_rounded(0.0)
        .on_layout(|s: &mut State, r| s.viewer.size = [r[2], r[3]])
        .on_double_press(|s: &mut State| s.viewer.reset_view())
        .on_drag(|s: &mut State, d, _| s.viewer.drag(d))
        .on_wheel(|s: &mut State, e, _| {
            s.viewer.zoom_at([e.x, e.y], (e.dy / 300.0).exp());
            true
        })
        .on_magnify(|s: &mut State, e, _| {
            s.viewer.zoom_at([e.x, e.y], 1.0 + e.delta);
            true
        });
    let Some(source) = &s.source else {
        return canvas.child(
            col()
                .absolute(0.0, 0.0)
                .size(w, h)
                .center()
                .gap(14.0)
                .child(text("Aa").font(theme::hero()).color(theme::ACCENT))
                .child(text("Every image has another language").font(theme::heading()).color(theme::TEXT))
                .child(theme::muted("Open an image or video, or drop one here."))
                .child(theme::muted("Shape light with characters. Export it your way.")),
        );
    };
    let source_size = source.display_size();
    let full = image_rect([0.0, 0.0, w, h], source_size, v.zoom, v.pan);
    let original = s.preview.original.is_some();
    let ascii = s.preview.ascii.is_some();
    match v.mode {
        ViewMode::Original => {
            if original {
                canvas = canvas.child(image_slot(1, full));
            }
        }
        ViewMode::Ascii => {
            if ascii {
                canvas = canvas.child(image_slot(2, full));
            }
        }
        ViewMode::SideBySide => {
            for (mark, left, available) in [(1, 0.0, original), (2, w * 0.5, ascii)] {
                let rect = image_rect([0.0, 0.0, w * 0.5, h], source_size, v.zoom, v.pan);
                let mut half = el().absolute(left, 0.0).size(w * 0.5, h).clip_rounded(0.0);
                if available {
                    half = half.child(image_slot(mark, rect));
                }
                canvas = canvas.child(half).child(caption(if mark == 1 { "ORIGINAL" } else { "ASCII" }, left));
            }
        }
        ViewMode::Split => {
            if ascii {
                canvas = canvas.child(image_slot(2, full));
            }
            if original {
                canvas = canvas
                    .child(el().absolute(0.0, 0.0).size(w * v.split, h).clip_rounded(0.0).child(image_slot(1, full)));
            }
            let x = w * v.split;
            canvas = canvas
                .child(el().absolute(x - 0.5, 0.0).size(1.0, h).fill(theme::TEXT.alpha(0.7)))
                .child(el().absolute(x - 6.0, h * 0.5 - 15.0).size(12.0, 30.0).radius(6.0).fill(theme::ACCENT))
                .child(caption("ORIGINAL", 0.0))
                .child(caption("ASCII", (w - 90.0).max(0.0)));
        }
    }
    if s.preview.waiting {
        canvas =
            canvas.child(theme::muted("Updating preview…").padding(8.0).fill(theme::PANEL).absolute(16.0, h - 40.0));
    }
    if let Some(error) = &s.preview.error {
        canvas = canvas.child(
            text(error)
                .font(theme::small())
                .color(theme::DANGER)
                .padding(12.0)
                .fill(theme::PANEL)
                .absolute(16.0, h - 70.0)
                .max_w((w - 32.0).max(100.0)),
        );
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fit_uses_source_aspect_not_reduced_texture_dimensions() {
        let r = image_rect([0.0, 0.0, 1000.0, 500.0], Size2 { w: 400, h: 300 }, 1.0, [0.0; 2]);
        assert!((r[2] / r[3] - 4.0 / 3.0).abs() < 1e-6);
        assert!((r[0] - 166.66667).abs() < 0.001);
        assert_eq!(r[3], 500.0);
    }
    #[test]
    fn zoom_keeps_the_point_under_the_pointer_in_every_mode() {
        for mode in ViewMode::ALL {
            let mut v = ViewerState { mode, size: [800.0, 600.0], pan: [20.0, -10.0], ..Default::default() };
            let pointer = [140.0, 180.0];
            let center = if mode == ViewMode::SideBySide { [200.0, 300.0] } else { [400.0, 300.0] };
            let before = [pointer[0] - center[0] - v.pan[0], pointer[1] - center[1] - v.pan[1]];
            v.zoom_at(pointer, 2.0);
            for i in 0..2 {
                assert!(((pointer[i] - center[i] - v.pan[i]) / v.zoom - before[i]).abs() < 0.001);
            }
            v.zoom_at(pointer, 1e9);
            assert_eq!(v.zoom, 50.0);
            v.zoom_at(pointer, 1e-9);
            assert_eq!(v.zoom, 0.1);
        }
    }
    #[test]
    fn mip_levels_are_premultiplied_and_terminate_at_one_pixel() {
        let image = ViewerImage::new(RgbaImage::from_fn(3, 5, |_, _| image::Rgba([255, 128, 64, 128])));
        assert_eq!(image.levels[0].get_pixel(0, 0).0, [128, 64, 32, 128]);
        assert_eq!(image.levels.last().unwrap().dimensions(), (1, 1));
    }
}
