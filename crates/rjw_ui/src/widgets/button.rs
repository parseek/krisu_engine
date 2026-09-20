//! 按钮控件（属性化 builder；未设置的属性回落全局 [`Theme::button`]）。

use std::sync::Arc;

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;
use crate::draw::Size;
use crate::draw::CornerRadius;
use crate::draw::{TextAlign, TextVAlign};
use crate::focus::FocusKind;
use crate::state::ButtonState;
use crate::style::{Brush, ButtonStyle, Theme};
use crate::ui::Ui;
use super::{Response, Sense, Widget};

// ─── Button ─────────────────────────────────────────────────────

/// 按钮控件（属性化 builder；未设置的属性回落全局 [`Theme::button`]）。
pub struct Button<'a> {
    id: &'a str,
    label: &'a str,
    /// 文本色（默认 `ButtonStyle::fg`）。
    color: Option<Color>,
    bg: Option<Brush>,
    bg_hover: Option<Brush>,
    bg_pressed: Option<Brush>,
    border: Option<Color>,
    border_w: Option<Size<f32>>,
    /// 圆角半径（[`Size<f32>`]：逻辑（默认）或物理；0 = 直角）。
    radius: Option<Size<CornerRadius>>,
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
    /// 常态背景刷（默认 `ButtonStyle::bg`；接受 [`Color`] 或 [`Brush`]）。
    pub fn bg(mut self, c: impl Into<Brush>) -> Self {
        self.bg = Some(c.into());
        self
    }
    /// 悬停背景刷（默认 `ButtonStyle::bg_hover`）。
    pub fn bg_hover(mut self, c: impl Into<Brush>) -> Self {
        self.bg_hover = Some(c.into());
        self
    }
    /// 按下背景刷（默认 `ButtonStyle::bg_pressed`）。
    pub fn bg_pressed(mut self, c: impl Into<Brush>) -> Self {
        self.bg_pressed = Some(c.into());
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
    /// 圆角半径（[`Size<CornerRadius>`]：逻辑（默认）或物理；默认 `ButtonStyle::radius`）。
    ///
    /// 接受 `f32`（四角相同）或 [`CornerRadius`]（**只圆某些角**，例如只圆上面两个角）。
    pub fn radius(mut self, r: impl Into<Size<CornerRadius>>) -> Self {
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
    fn ui(self, ui: &mut Ui) -> Response {
        let style = self.resolve(&ui.theme, ui.scale());
        // ① 先量（文本测量必须在申请之前——申请会推进容器光标）
        let size = {
            let tsize = ui.text_size(self.label, style.font_size, style.font_family.as_deref());
            Vec2::new(
                tsize.x + style.padding.x * 2.0,
                tsize.y + style.padding.y * 2.0,
            )
        };
        // ② 申请（占光标；`add_at` 的绝对定位由 `place_once` 覆盖）
        let rect = ui.allocate(size);
        // 被裁剪层完全剔除 ⇒ 直接 return（不镶嵌、不入段：scissor 只省片元）。
        if ui.culled(rect) {
            return Response { rect, culled: true, ..Default::default() };
        }
        // ③ 交互 + 绘制交给显式 rect 入口（键盘激活 / 省略号 / 三态配色都在那里）
        let s = ui.button_at_styled(self.id, rect, self.label, &style);
        Response { rect, ..s.into() }
    }
}

// ─── `Ui` 的显式 rect 入口（**实现体就近放控件自己的文件**）────────

// 搬运说明（`ui.rs` → `widgets/`，路线图 P2a）：公开路径**不变**（仍是
// `Ui::button_at` / `Ui::button_at_styled`），但实现体住在控件自己的文件里——`ui.rs`
// 只保留"引擎"逻辑（容器 / 布局 / 命中 / 结算）。`impl Ui` 可以写在同 crate 的任何
// 模块（`Ui` 类型对全 crate 可见）⇒ 不需要在 `ui.rs` 里留一行转发。
//
// `widgets` 是 `ui` 的**兄弟模块**，所以这里只能用 `Ui` 的**公开**方法
// （`note_placed` / `interact` / `push_panel_like` / `ellipsized` / `painter()` /
// `theme()`），不靠 `pub(crate)` 后门——新增依赖时先想"这是不是控件作者也该有的公开面"。
impl Ui<'_> {
    /// 按钮（显式 rect；样式取全局 `Theme::button`）。
    pub fn button_at(&mut self, id: &str, rect: Rect, label: &str) -> ButtonState {
        let style = self.theme().button.clone();
        self.button_at_styled(id, rect, label, &style)
    }

    /// 按钮（显式 rect + **样式可覆盖**——widget 层 [`Button`] 经此合并主题与逐控件属性；
    /// [`Self::button_at`] 委托本方法）。
    ///
    /// 非公开：样式必须来自 [`Theme`] 或 widget builder，避免"同一种控件两条入口"。
    pub(crate) fn button_at_styled(
        &mut self,
        id: &str,
        rect: Rect,
        label: &str,
        style: &ButtonStyle,
    ) -> ButtonState {
        let abs = self.id_for(id);
        self.note_placed(rect);
        // 命中 / 焦点链 / 键盘激活 / 跨帧状态机：一句话（`Sense::CLICK` —— 按钮没有拖拽语义，
        // 所以不 `claim_press`）。
        let resp = self.interact(&abs, rect, Sense::CLICK.focus(FocusKind::Button));
        let bg = style.pick_bg(resp.pressed, resp.hovered);
        let elem = self.elem_hint();
        // 背景 + 边框（radius > 0 走圆角双层矩形）。
        self.push_panel_like(rect, bg, style.border, style.border_w, style.radius, elem);
        // 按钮文本自动省略（Resizable 窗口缩窄 / max 约束下不溢出）：
        // 文本超出可用区（rect 宽 - 水平内边距）→ "…"截断（内容自洽，noclip）。
        let label_owned = self.ellipsized(
            label,
            style.font_size,
            style.font_family.as_deref(),
            (rect.w - style.padding.x * 2.0).max(0.0),
        );
        let draw_label: String = label_owned.unwrap_or_else(|| label.to_owned());
        // 文本与背景同 elem（旧写法）：`push_text_rect` 会取更大的 elem ⇒ 文字在上。
        self.painter().text(
            rect,
            &draw_label,
            style.font_size,
            style.fg,
            style.font_family.clone(),
            TextAlign::Center,
            TextVAlign::Center,
            None,
            None,
        );
        ButtonState {
            hovered: resp.hovered,
            pressed: resp.pressed,
            clicked: resp.clicked,
            released: resp.released,
        }
    }
}
