//! 分割线控件（占光标）：宽 = 容器可用宽 / 当前最宽子项，行高 = 线厚 + 上下留白。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;
use crate::draw::Size;
use crate::ui::Ui;
use super::{Response, Widget};

// ─── Divider ────────────────────────────────────────────────────

/// 分割线控件（占光标）：宽 = 容器可用宽（`avail_w`）/ 当前最宽子项，行高 =
/// 线厚 + 上下留白。属性可选，未设置回落 [`Theme::divider`](crate::style::Theme::divider)。
pub struct Divider {
    color: Option<Color>,
    thickness: Option<Size<f32>>,
    margin: Option<Size<f32>>,
}

impl Divider {
    pub fn new() -> Self {
        Self { color: None, thickness: None, margin: None }
    }
    /// 线颜色（默认 `Theme::divider.color`）。
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
    /// 线厚度（[`Size<f32>`]：逻辑（默认）或物理；默认 `Theme::divider.thickness`）。
    pub fn thickness(mut self, t: impl Into<Size<f32>>) -> Self {
        self.thickness = Some(t.into());
        self
    }
    /// 上下留白（[`Size<f32>`]：逻辑（默认）或物理；默认 `Theme::divider.margin`）。
    pub fn margin(mut self, m: impl Into<Size<f32>>) -> Self {
        self.margin = Some(m.into());
        self
    }
}

impl Default for Divider {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Divider {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        let st = ui.theme.divider.clone();
        let t = self.thickness.map(|x| x.to_physical(ui.scale())).unwrap_or(st.thickness);
        let m = self.margin.map(|x| x.to_physical(ui.scale())).unwrap_or(st.margin);
        // 宽 = 容器可用宽（固定宽窗口 / 沙箱）；无可用宽 = 默认 120。
        let w = ui.avail_w().unwrap_or(120.0);
        Vec2::new(w, t + m * 2.0)
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        let st = ui.theme.divider.clone();
        let t = self.thickness.map(|x| x.to_physical(ui.scale())).unwrap_or(st.thickness);
        let c = self.color.unwrap_or(st.color);
        let y = rect.y + (rect.h - t) * 0.5; // 垂直居中（行高被 clamp 时仍居中）
        ui.push_solid_rect(Rect::new(rect.x, y, rect.w, t), c);
        Response::default()
    }
}
