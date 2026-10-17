//! Original / ASCII / side-by-side / split-wipe viewer with shared zoom and pan.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, TextureHandle, Ui, Vec2, pos2,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    Original,
    Ascii,
    SideBySide,
    #[default]
    Split,
}

impl ViewMode {
    pub const ALL: [ViewMode; 4] = [Self::Original, Self::Ascii, Self::SideBySide, Self::Split];

    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Ascii => "ASCII",
            Self::SideBySide => "Side by side",
            Self::Split => "Split",
        }
    }
}

pub struct ViewerState {
    pub mode: ViewMode,
    /// 1 = fit to the view.
    pub zoom: f32,
    pub pan: Vec2,
    /// Divider position in split mode, `0..=1` of the view width.
    pub split: f32,
    dragging_divider: bool,
}

impl Default for ViewerState {
    fn default() -> Self {
        Self { mode: ViewMode::Split, zoom: 1.0, pan: Vec2::ZERO, split: 0.5, dragging_divider: false }
    }
}

impl ViewerState {
    pub fn with_mode(mode: ViewMode) -> Self {
        Self { mode, ..Self::default() }
    }

    pub fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
    }
}

pub struct ViewerImages<'a> {
    pub original: Option<&'a TextureHandle>,
    pub ascii: Option<&'a TextureHandle>,
    /// Width ÷ height of the source; both images are shown in a rect of this aspect.
    pub aspect: f32,
}

const FULL_UV: Rect = Rect { min: Pos2 { x: 0.0, y: 0.0 }, max: Pos2 { x: 1.0, y: 1.0 } };
const DIVIDER_GRAB: f32 = 8.0;

/// The image rect inside `area` for the current zoom and pan.
fn image_rect(area: Rect, aspect: f32, state: &ViewerState) -> Rect {
    let fit = if area.width() / area.height() > aspect {
        Vec2::new(area.height() * aspect, area.height())
    } else {
        Vec2::new(area.width(), area.width() / aspect)
    };
    Rect::from_center_size(area.center() + state.pan, fit * state.zoom)
}

fn draw(painter: &egui::Painter, texture: Option<&TextureHandle>, rect: Rect) {
    if let Some(texture) = texture {
        painter.image(texture.id(), rect, FULL_UV, Color32::WHITE);
    }
}

fn caption(painter: &egui::Painter, pos: Pos2, anchor: Align2, text: &str) {
    let galley_rect = painter.text(pos, anchor, text, FontId::proportional(13.0), Color32::WHITE);
    painter.rect_filled(galley_rect.expand(4.0), 4.0, Color32::from_black_alpha(150));
    painter.text(pos, anchor, text, FontId::proportional(13.0), Color32::WHITE);
}

pub fn show(ui: &mut Ui, state: &mut ViewerState, images: ViewerImages) {
    let (area, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let painter = ui.painter_at(area);
    painter.rect_filled(area, 0.0, Color32::from_gray(24));
    let aspect = images.aspect.max(0.01);
    let pointer = response.hover_pos();

    // Zoom (wheel or pinch) around the pointer.
    if let Some(p) = pointer {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = (scroll / 300.0).exp() * pinch;
        if (factor - 1.0).abs() > 1e-4 {
            let old = state.zoom;
            state.zoom = (old * factor).clamp(0.1, 50.0);
            let anchor = p - area.center();
            state.pan = anchor - (anchor - state.pan) * (state.zoom / old);
        }
    }

    let divider_x = area.left() + state.split * area.width();
    let split_mode = state.mode == ViewMode::Split;
    let near_divider = |p: Pos2| split_mode && (p.x - divider_x).abs() <= DIVIDER_GRAB;
    if response.drag_started() {
        state.dragging_divider = response.interact_pointer_pos().is_some_and(near_divider);
    }
    if response.dragged() {
        if state.dragging_divider {
            if let Some(p) = response.interact_pointer_pos() {
                state.split = ((p.x - area.left()) / area.width()).clamp(0.0, 1.0);
            }
        } else {
            state.pan += response.drag_delta();
        }
    }
    if response.drag_stopped() {
        state.dragging_divider = false;
    }
    if response.double_clicked() {
        state.reset_view();
    }
    if state.dragging_divider || pointer.is_some_and(near_divider) {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    } else if response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }

    match state.mode {
        ViewMode::Original => draw(&painter, images.original, image_rect(area, aspect, state)),
        ViewMode::Ascii => draw(&painter, images.ascii, image_rect(area, aspect, state)),
        ViewMode::SideBySide => {
            let (left, right) = area.split_left_right_at_fraction(0.5);
            draw(&painter.with_clip_rect(left), images.original, image_rect(left, aspect, state));
            draw(&painter.with_clip_rect(right), images.ascii, image_rect(right, aspect, state));
            painter.vline(left.right(), area.y_range(), Stroke::new(2.0, Color32::from_gray(60)));
            caption(&painter, left.left_top() + Vec2::splat(10.0), Align2::LEFT_TOP, "Original");
            caption(&painter, right.left_top() + Vec2::splat(10.0), Align2::LEFT_TOP, "ASCII");
        }
        ViewMode::Split => {
            let rect = image_rect(area, aspect, state);
            draw(&painter, images.ascii, rect);
            let left = Rect::from_min_max(area.min, pos2(divider_x, area.max.y));
            draw(&painter.with_clip_rect(left), images.original, rect);
            painter.vline(divider_x, area.y_range(), Stroke::new(2.0, Color32::WHITE));
            painter.circle(
                pos2(divider_x, area.center().y),
                9.0,
                Color32::from_gray(40),
                Stroke::new(2.0, Color32::WHITE),
            );
            caption(&painter, area.left_top() + Vec2::splat(10.0), Align2::LEFT_TOP, "Original");
            caption(&painter, area.right_top() + Vec2::new(-10.0, 10.0), Align2::RIGHT_TOP, "ASCII");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_rect_fits_and_zooms_around_center() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(200.0, 100.0));
        let mut state = ViewerState::default();
        let fit = image_rect(area, 1.0, &state);
        assert_eq!(fit.size(), Vec2::new(100.0, 100.0));
        assert_eq!(fit.center(), area.center());
        state.zoom = 2.0;
        assert_eq!(image_rect(area, 1.0, &state).size(), Vec2::new(200.0, 200.0));
        let wide = image_rect(area, 4.0, &ViewerState::default());
        assert_eq!(wide.size(), Vec2::new(200.0, 50.0));
    }
}
