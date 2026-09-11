//! 按钮控件（属性化 builder；未设置的属性回落全局 [`Theme::button`]）。

use std::sync::Arc;

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;
use crate::draw::Size;
use crate::style::{ButtonStyle, Theme};
use crate::ui::Ui;
use super::{Response, Widget};

// ─── Button ─────────────────────────────────────────────────────

/// 按钮控件（属性化 builder；未设置的属性回落全局 [`Theme::button`]）。
pub struct Button<'a> {
    id: &'a str,
    label: &'a str,
    /// 文本色（默认 `ButtonStyle::fg`）。
    color: Option<Color>,
    bg: Option<Color>,
    bg_hover: Option<Color>,
    bg_pressed: Option<Color>,
    border: Option<Color>,
    border_w: Option<Size<f32>>,
    /// 圆角半径（[`Size<f32>`]：逻辑（默认）或物理；0 = 直角）。
    radius: Option<Size<f32>>,
    /// 内边距（x = 水平，y = 垂直；[`Size<Vec2>`]：逻辑（默认）或物理）。
    padding: Option<Size<Vec2>>,
    font_size: Option<Size<f32>>,
    font_family: Option<&'a str>,
}

impl<'a> Button<'a> {
    pub fn new(id: &'a str, label: &'a str) -> Self {
        Self {
            id,
            label,
            color: None,
            bg: None,
            bg_hover: None,
            bg_pressed: None,
            border: None,
            border_w: None,
            radius: None,
            padding: None,
            font_size: None,
            font_family: None,
        }
    }

    /// 文本颜色（默认 `ButtonStyle::fg`）。
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }
    /// 常态背景（默认 `ButtonStyle::bg`）。
    pub fn bg(mut self, c: Color) -> Self {
        self.bg = Some(c);
        self
    }
    /// 悬停背景（默认 `ButtonStyle::bg_hover`）。
    pub fn bg_hover(mut self, c: Color) -> Self {
        self.bg_hover = Some(c);
        self
    }
    /// 按下背景（默认 `ButtonStyle::bg_pressed`）。
    pub fn bg_pressed(mut self, c: Color) -> Self {
        self.bg_pressed = Some(c);
        self
    }
    /// 边框颜色（默认 `ButtonStyle::border`）。
    pub fn border(mut self, c: Color) -> Self {
        self.border = Some(c);
        self
    }
    /// 边框宽度（[`Size<f32>`]：逻辑（默认）或物理；默认 `ButtonStyle::border_w`）。
    pub fn border_w(mut self, w: impl Into<Size<f32>>) -> Self {
        self.border_w = Some(w.into());
        self
    }
    /// 圆角半径（[`Size<f32>`]：逻辑（默认）或物理；默认 `ButtonStyle::radius`）。
    pub fn radius(mut self, r: impl Into<Size<f32>>) -> Self {
        self.radius = Some(r.into());
        self
    }
    /// 内边距（[`Size<Vec2>`]：逻辑（默认）或物理；默认 `ButtonStyle::padding`）。
    pub fn padding(mut self, p: impl Into<Size<Vec2>>) -> Self {
        self.padding = Some(p.into());
        self
    }
    /// 字号（[`Size<f32>`]：逻辑（默认）或物理；默认 `ButtonStyle::font_size`）。
    pub fn font_size(mut self, s: impl Into<Size<f32>>) -> Self {
        self.font_size = Some(s.into());
        self
    }
    /// 字体族（默认 `ButtonStyle::font_family`）。
    pub fn font_family(mut self, f: &'a str) -> Self {
        self.font_family = Some(f);
        self
    }

    /// 主题样式 + 本控件覆盖 → 最终样式（`scale`：API 边界 Size 换算用）。
    fn resolve(&self, theme: &Theme, scale: f32) -> ButtonStyle {
        let base = &theme.button;
        ButtonStyle {
            bg: self.bg.unwrap_or(base.bg),
            bg_hover: self.bg_hover.unwrap_or(base.bg_hover),
            bg_pressed: self.bg_pressed.unwrap_or(base.bg_pressed),
            fg: self.color.unwrap_or(base.fg),
            border: self.border.unwrap_or(base.border),
            border_w: self.border_w.map(|w| w.to_physical(scale)).unwrap_or(base.border_w),
            radius: self.radius.map(|r| r.to_physical(scale)).unwrap_or(base.radius),
            padding: self.padding.map(|p| p.to_physical(scale)).unwrap_or(base.padding),
            font_size: self.font_size.map(|s| s.to_physical(scale)).unwrap_or(base.font_size),
            font_family: self
                .font_family
                .map(Arc::from)
                .or_else(|| base.font_family.clone()),
        }
    }
}

impl Widget for Button<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        let size = self
            .font_size
            .map(|s| s.to_physical(ui.scale()))
            .unwrap_or(ui.theme.button.font_size);
        let family = match self.font_family {
            Some(f) => Some(Arc::from(f)),
            None => ui.theme.button.font_family.clone(),
        };
        let tsize = ui.text_size(self.label, size, family.as_deref());
        let pad = self
            .padding
            .map(|p| p.to_physical(ui.scale()))
            .unwrap_or(ui.theme.button.padding);
        Vec2::new(tsize.x + pad.x * 2.0, tsize.y + pad.y * 2.0)
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        let style = self.resolve(&ui.theme, ui.scale());
        let s = ui.button_at_styled(self.id, rect, self.label, &style);
        Response {
            hovered: s.hovered,
            pressed: s.pressed,
            clicked: s.clicked,
            released: s.released,
            toggled: false,
        }
    }
}
