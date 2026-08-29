//! 标签控件（属性化 builder；未设置的属性回落全局 [`Theme::label`]）。

use std::sync::Arc;

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;
use crate::draw::{Size, TextAlign, TextVAlign};
use rjw_text::Align;
use crate::ui::Ui;
use super::{Response, Widget};

// ─── Label ──────────────────────────────────────────────────────

/// 标签控件（属性化 builder；未设置的属性回落全局 [`Theme::label`]）。
///
/// ```no_run
/// # let mut ui: rjw_ui::Ui = todo!();
/// ui.add(rjw_ui::Label::new("红色 20px").color(rjw_color::Color::RED).font_size(20.0));
/// ```
pub struct Label<'a> {
    text: &'a str,
    color: Option<Color>,
    font_size: Option<Size<f32>>,
    font_family: Option<&'a str>,
    /// 自动换行宽度（[`Size<f32>`]：逻辑（默认）或物理；`Some(w)` 且 `w > 0` 时换行）。
    wrap: Option<Size<f32>>,
    /// 省略模式：文本超出可用/分配宽度时以 "…" 截断（单行，内容自洽）。
    ellipsis: bool,
}

impl<'a> Label<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text, color: None, font_size: None, font_family: None, wrap: None, ellipsis: false }
    }

    /// 文本颜色（默认 `Theme::label.color`）。
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }

    /// 字号（[`Size<f32>`]：`Logical`（默认，× scale）/ `Physical` 原样；
    /// 默认 `Theme::label.font_size`）。
    pub fn font_size(mut self, s: impl Into<Size<f32>>) -> Self {
        self.font_size = Some(s.into());
        self
    }

    /// 字体族（默认 `Theme::label.font_family`）。
    pub fn font_family(mut self, f: &'a str) -> Self {
        self.font_family = Some(f);
        self
    }

    /// 按宽度自动换行（[`Size<f32>`]：逻辑（默认）或物理；`<= 0` = 不换行，同默认）。
    pub fn wrap(mut self, max_w: impl Into<Size<f32>>) -> Self {
        self.wrap = Some(max_w.into());
        self
    }

    /// **省略模式**：文本超出可用宽度（容器固定宽 / 沙箱 `avail_w` / 分配矩形）时
    /// 以 "…" 截断为单行（内容自洽，配合 Resizable 窗口缩窄）。
    pub fn ellipsis(mut self) -> Self {
        self.ellipsis = true;
        self
    }

    /// **解析文本样式**（属性覆盖 / 主题回落）→ `(颜色, 字号, 对齐, 字体族)`。
    /// 先把主题值拷出（Copy / owned），避免主题借用与绘制时 `&mut ui` 调用冲突。
    fn resolve_style(&self, ui: &Ui) -> (Color, f32, Align, Option<Arc<str>>) {
        let color = self.color.unwrap_or(ui.theme.label.color);
        let size = self
            .font_size
            .map(|s| s.to_physical(ui.scale()))
            .unwrap_or(ui.theme.label.font_size);
        let align = ui.theme.label.align;
        let family = match self.font_family {
            Some(f) => Some(Arc::from(f)),
            None => ui.theme.label.font_family.clone(),
        };
        (color, size, align, family)
    }

    /// **省略模式绘制**：文本超出分配宽度 → "…"截断（单行、内容自洽：宽 = `rect.w`，noclip）。
    fn draw_ellipsis(
        &self,
        ui: &mut Ui,
        rect: Rect,
        color: Color,
        size: f32,
        align: Align,
        family: Option<Arc<str>>,
    ) {
        let natural = ui.text_size(self.text, size, family.as_deref());
        let text: std::borrow::Cow<'_, str> = if natural.x > rect.w {
            crate::edit::ellipsize(self.text, rect.w, |s| {
                ui.text_size(s, size, family.as_deref()).x
            })
        } else {
            std::borrow::Cow::Borrowed(self.text)
        };
        ui.push_text_rect_noclip(
            rect,
            &text,
            size,
            color,
            family,
            TextAlign::from(align),
            TextVAlign::Center,
            None,
        );
    }

    /// **换行标签绘制**：直接传预排版缓冲（`wrap` 宽度参与缓存键），保证渲染与测量一致。
    /// 默认自动换行（`LimitedInParent`）：`size()` 已按可用宽测量换行高度，渲染**必须**
    /// 用同一宽度的换行缓冲（`rect.w`），否则"逻辑换行、渲染仍溢出"。
    fn draw_wrapped(
        &self,
        ui: &mut Ui,
        rect: Rect,
        color: Color,
        size: f32,
        align: Align,
        family: Option<Arc<str>>,
    ) {
        let natural = ui.text_size(self.text, size, family.as_deref());
        let wrap_w = self
            .wrap
            .map(|w| w.to_physical(ui.scale()))
            .filter(|&w| w > 0.0)
            .unwrap_or(if natural.x > rect.w { rect.w } else { 0.0 });
        let buf =
            (wrap_w > 0.0).then(|| ui.wrap_buffer(self.text, size, family.as_deref(), wrap_w));
        ui.push_text_rect_noclip(
            rect,
            self.text,
            size,
            color,
            family,
            TextAlign::from(align),
            TextVAlign::Center,
            buf,
        );
    }
}

impl Widget for Label<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        let size = self
            .font_size
            .map(|s| s.to_physical(ui.scale()))
            .unwrap_or(ui.theme.label.font_size);
        let family = match self.font_family {
            Some(f) => Some(Arc::from(f)),
            None => ui.theme.label.font_family.clone(),
        };
        let wrap = self.wrap.map(|w| w.to_physical(ui.scale()));
        // 显式换行宽：直接按它测量。
        let natural = match wrap {
            Some(w) if w > 0.0 => ui.text_size_wrap(self.text, size, family.as_deref(), w),
            _ => ui.text_size(self.text, size, family.as_deref()),
        };
        if self.ellipsis {
            // 省略：宽度 ≤ 可用宽（单行；高度 = 自然行高）。
            if let Some(avail) = ui.avail_w() {
                if avail < natural.x {
                    return Vec2::new(avail, natural.y);
                }
            }
            natural
        } else if wrap.is_none() || wrap.is_some_and(|w| w <= 0.0) {
            // 默认（无显式换行宽）：**LimitedInParent**——在父级可用宽内自动换行
            // （Resizable 窗口缩窄后 Label 不溢出；无可用宽 = 自然尺寸）。
            if let Some(avail) = ui.avail_w() {
                if avail < natural.x {
                    return ui.text_size_wrap(self.text, size, family.as_deref(), avail);
                }
            }
            natural
        } else {
            natural
        }
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        // 解析样式（主题回落）后，按省略 / 换行两种模式分别绘制（见 `draw_ellipsis` /
        // `draw_wrapped`）。
        let (color, size, align, family) = self.resolve_style(ui);
        if self.ellipsis {
            self.draw_ellipsis(ui, rect, color, size, align, family);
        } else {
            self.draw_wrapped(ui, rect, color, size, align, family);
        }
        Response::default()
    }
}

