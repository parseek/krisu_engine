//! 勾选框控件（勾选值由调用方维护；属性化 builder，未设置回落全局 [`Theme::checkbox`]）。

use std::sync::Arc;

use glam::Vec2;
use rjw_color::Color;
use crate::draw::Size;
use crate::style::{CheckboxStyle, Theme};
use crate::ui::Ui;
use super::{Response, Widget};

// ─── Checkbox ───────────────────────────────────────────────────

/// 勾选框控件（勾选值由调用方维护；属性化 builder，未设置回落全局 [`Theme::checkbox`]）。
pub struct Checkbox<'a> {
    id: &'a str,
    label: &'a str,
    checked: bool,
    fg: Option<Color>,
    box_border: Option<Color>,
    checked_fill: Option<Color>,
    font_size: Option<Size<f32>>,
    font_family: Option<&'a str>,
}

impl<'a> Checkbox<'a> {
    pub fn new(id: &'a str, label: &'a str, checked: bool) -> Self {
        Self {
            id,
            label,
            checked,
            fg: None,
            box_border: None,
            checked_fill: None,
            font_size: None,
            font_family: None,
        }
    }

    /// 标签文本色（默认 `CheckboxStyle::fg`）。
    pub fn color(mut self, c: Color) -> Self {
        self.fg = Some(c);
        self
    }
    /// 方框边框色（默认 `CheckboxStyle::box_border`）。
    pub fn box_border(mut self, c: Color) -> Self {
        self.box_border = Some(c);
        self
    }
    /// 选中填充色（默认 `CheckboxStyle::checked_fill`）。
    pub fn checked_fill(mut self, c: Color) -> Self {
        self.checked_fill = Some(c);
        self
    }
    /// 字号（[`Size<f32>`]：逻辑（默认）或物理；默认 `CheckboxStyle::font_size`）。
    pub fn font_size(mut self, s: impl Into<Size<f32>>) -> Self {
        self.font_size = Some(s.into());
        self
    }
    /// 字体族（默认 `CheckboxStyle::font_family`）。
    pub fn font_family(mut self, f: &'a str) -> Self {
        self.font_family = Some(f);
        self
    }

    fn resolve(&self, theme: &Theme, scale: f32) -> CheckboxStyle {
        let base = &theme.checkbox;
        CheckboxStyle {
            box_size: base.box_size,
            radius: base.radius,
            box_border: self.box_border.unwrap_or(base.box_border),
            border_w: base.border_w,
            checked_fill: self.checked_fill.unwrap_or(base.checked_fill),
            fg: self.fg.unwrap_or(base.fg),
            font_size: self.font_size.map(|s| s.to_physical(scale)).unwrap_or(base.font_size),
            font_family: self
                .font_family
                .map(Arc::from)
                .or_else(|| base.font_family.clone()),
            gap: base.gap,
        }
    }
}

impl Widget for Checkbox<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let style = self.resolve(ui.theme(), ui.scale());
        // ① 先量：方框 + 间距 + 文本宽
        let tsize = ui.text_size(self.label, style.font_size, style.font_family.as_deref());
        let size = Vec2::new(
            style.box_size + style.gap + tsize.x,
            style.box_size.max(tsize.y),
        );
        // ② 申请（占光标）
        let rect = ui.allocate(size);
        // 被裁剪层完全剔除 ⇒ 直接 return（不镶嵌、不入段：scissor 只省片元）。
        if ui.culled(rect) {
            return Response { rect, culled: true, ..Default::default() };
        }
        // ③ 交互 + 绘制（显式 rect 入口负责键盘激活 / 勾选态配色）
        let s = ui.checkbox_at_styled(self.id, rect, self.label, self.checked, &style);
        Response {
            rect,
            culled: false,
            hovered: s.hovered,
            pressed: s.pressed,
            clicked: s.clicked,
            released: false,
            toggled: s.toggled,
        }
    }
}
