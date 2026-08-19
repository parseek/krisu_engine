//! 控件 trait 与**属性化 builder**（非宏、可调试的控件扩展方式）。
//!
//! # 为什么
//!
//! 旧控件 API 由 `widget_api!` 宏统一生成（`p.label` / `p.button` …）——加一个新控件要
//! 改宏（报错指向宏展开、难以调试），且控件属性（颜色 / 字号 / 字体 / 内边距）只能
//! 跟随全局 [`Theme`]（`crate::style::Theme`），无法逐控件覆盖。
//!
//! 本模块提供：
//! - [`Widget`] trait：**新控件 = 一个实现该 trait 的 builder 结构体**——普通 Rust，
//!   无宏展开，报错定位精确，可单测；
//! - 属性化 builder：[`Label`] / [`Button`] / [`Checkbox`]——`Option` 覆盖字段 + 链式
//!   setter，未设置的属性回落到全局 [`Theme`]；
//! - 统一响应 [`Response`]（hover / pressed / clicked / released / toggled）；
//! - 放置方式：[`Ui::add`]（容器内占光标）/ [`Ui::add_at`]（绝对定位）；容器包装
//!   （`Panel` / `Pack` / `Grid` / `Window` / `Scroll` / `FlexCtx`）经 [`UiAdd`] 提供
//!   同样的 `add` / `add_at`。
//!
//! # 用法
//!
//! ```no_run
//! # let cam = todo!(); let mouse = todo!(); let keyboard = todo!();
//! # let text = todo!(); let r2d = todo!(); let state = todo!(); let window = todo!();
//! use rjw_color::Color;
//! use rjw_ui::{Button, Label, Theme, Ui, UiAdd};
//! let mut ui = Ui::begin(&window, &cam, &mouse, &keyboard, &mut text, &mut r2d, &mut state)
//!     .theme(Theme::dark())
//!     .build();
//! // 容器内：占光标
//! ui.pack_at(glam::Vec2::new(16.0, 16.0), rjw_ui::PackSide::Top, |p| {
//!     if p.add(Button::new("btn_ok", "确定").color(Color::WHITE)).clicked() {
//!         // …
//!     }
//!     p.add(Label::new("红色大字").color(Color::RED).font_size(20.0));
//! });
//! // 顶层：绝对定位（不占光标）
//! ui.add_at(glam::Vec2::new(400.0, 40.0), Label::new("HUD"));
//! ui.finish();
//! ```
//!
//! # 添加一个新控件
//!
//! 1. 定义 builder 结构体（属性 = `Option<T>` 覆盖字段）；
//! 2. `impl Widget`：`size` 测量内容（可调 `ui.text_size` 等），`ui` 在给定矩形内
//!    渲染 + 交互（可调 `Ui` 的 `pub(crate)` 原语，或复用现有 `*_at` 方法）；
//! 3. 需要复用现有控件时，优先给 `Ui` 增加 `xxx_at_styled(…, &Style)` 变体并让旧方法
//!    委托它（见 `button_at` / `checkbox_at` 的改造方式）。
//!
//! 详见 `docs/WIDGET_GUIDE.md`。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::draw::{TextAlign, TextVAlign};
use crate::state::ButtonState;
use crate::style::{ButtonStyle, CheckboxStyle, Theme};
use crate::ui::Ui;

// ─── 统一响应 ───────────────────────────────────────────────────

/// 控件统一交互响应（hover / pressed / clicked / released / toggled）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Response {
    /// 鼠标悬停在本体（含按下时）。
    pub hovered: bool,
    /// 处于按下状态（按下后未释放）。
    pub pressed: bool,
    /// 本帧完成一次点击（按下 + 释放均在本体内；键盘 Enter/Space 激活同理）。
    pub clicked: bool,
    /// 本帧释放（无论释放位置）。
    pub released: bool,
    /// 勾选 / 单选类控件：本帧是否切换（其余控件恒 `false`）。
    pub toggled: bool,
}

impl Response {
    #[inline]
    pub fn hovered(&self) -> bool {
        self.hovered
    }
    #[inline]
    pub fn pressed(&self) -> bool {
        self.pressed
    }
    #[inline]
    pub fn clicked(&self) -> bool {
        self.clicked
    }
    #[inline]
    pub fn released(&self) -> bool {
        self.released
    }
    #[inline]
    pub fn toggled(&self) -> bool {
        self.toggled
    }
}

impl From<ButtonState> for Response {
    fn from(s: ButtonState) -> Self {
        Self {
            hovered: s.hovered,
            pressed: s.pressed,
            clicked: s.clicked,
            released: s.released,
            toggled: false,
        }
    }
}

// ─── Widget trait ───────────────────────────────────────────────

/// **控件 trait**：新控件 = 实现此 trait 的 builder 结构体（普通 Rust，无宏）。
///
/// - [`Widget::size`]：期望尺寸（**逻辑像素**；内容测量可调用 `ui.text_size` /
///   `ui.text_size_wrap`，或读取样式常量）；
/// - [`Widget::ui`]：在分配好的矩形内渲染 + 交互，返回 [`Response`]。
///
/// 放置：[`Ui::add`]（容器内占光标）/ [`Ui::add_at`]（绝对定位）；容器包装经
/// [`UiAdd`] 提供同样的 `add` / `add_at`。
pub trait Widget {
    /// 期望尺寸（逻辑像素；内容测量可经 `&mut Ui` 排版/缓存）。
    fn size(&self, ui: &mut Ui) -> Vec2;

    /// 在 `rect`（相对当前容器内容原点，逻辑像素）内渲染 + 交互。
    fn ui(self, ui: &mut Ui, rect: Rect) -> Response;
}

/// **容器可放置控件**：`add`（占光标）/ `add_at`（绝对定位）。
///
/// 由全部容器包装（`Panel` / `Pack` / `Grid` / `Window` / `Scroll` / `FlexCtx`）
/// 实现——`p.add(…)` / `w.add(…)` 与 `ui.add(…)` 同语义。
pub trait UiAdd<'a> {
    /// 容器持有的 `Ui`（包装字段，仅本 crate 内实现）。
    fn ui_mut(&mut self) -> &mut Ui<'a>;

    /// 在容器内**占光标**放置控件（尺寸 = 控件测量值）。
    fn add(&mut self, w: impl Widget) -> Response {
        let size = w.size(self.ui_mut());
        let rect = self.ui_mut().child_rect(size.x, size.y);
        w.ui(self.ui_mut(), rect)
    }

    /// **绝对定位**放置控件（`pos` 相对当前容器内容原点；不占光标）。
    fn add_at(&mut self, pos: Vec2, w: impl Widget) -> Response {
        let size = w.size(self.ui_mut());
        w.ui(self.ui_mut(), Rect::new(pos.x, pos.y, size.x, size.y))
    }
}

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
    font_size: Option<f32>,
    font_family: Option<&'a str>,
    /// 自动换行宽度（逻辑像素；`Some(w)` 且 `w > 0` 时按宽度换行）。
    wrap: Option<f32>,
}

impl<'a> Label<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text, color: None, font_size: None, font_family: None, wrap: None }
    }

    /// 文本颜色（默认 `Theme::label.color`）。
    pub fn color(mut self, c: Color) -> Self {
        self.color = Some(c);
        self
    }

    /// 字号（默认 `Theme::label.font_size`）。
    pub fn font_size(mut self, s: f32) -> Self {
        self.font_size = Some(s);
        self
    }

    /// 字体族（默认 `Theme::label.font_family`）。
    pub fn font_family(mut self, f: &'a str) -> Self {
        self.font_family = Some(f);
        self
    }

    /// 按宽度自动换行（`max_w` 逻辑像素；`<= 0` = 不换行，同默认）。
    pub fn wrap(mut self, max_w: f32) -> Self {
        self.wrap = Some(max_w);
        self
    }
}

impl Widget for Label<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        let size = self.font_size.unwrap_or(ui.theme.label.font_size);
        let family = match self.font_family {
            Some(f) => Some(f.to_owned()),
            None => ui.theme.label.font_family.clone(),
        };
        match self.wrap {
            Some(w) if w > 0.0 => ui.text_size_wrap(self.text, size, family.as_deref(), w),
            _ => ui.text_size(self.text, size, family.as_deref()),
        }
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        // 先把主题值拷出（Copy / owned），避免主题借用与下方 &mut ui 调用冲突
        let color = self.color.unwrap_or(ui.theme.label.color);
        let size = self.font_size.unwrap_or(ui.theme.label.font_size);
        let align = ui.theme.label.align;
        let family = match self.font_family {
            Some(f) => Some(f.to_owned()),
            None => ui.theme.label.font_family.clone(),
        };
        // 换行标签：直接传预排版缓冲（wrap 宽度参与缓存键），保证渲染与测量一致。
        let buf = self
            .wrap
            .filter(|&w| w > 0.0)
            .map(|w| ui.wrap_buffer(self.text, size, family.as_deref(), w));
        ui.push_text_rect(
            rect,
            self.text,
            size,
            color,
            family,
            TextAlign::from(align),
            TextVAlign::Center,
            None,
            buf,
        );
        Response::default()
    }
}

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
    border_w: Option<f32>,
    /// 圆角半径（逻辑像素；0 = 直角）。
    radius: Option<f32>,
    /// 内边距（x = 水平，y = 垂直）。
    padding: Option<Vec2>,
    font_size: Option<f32>,
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
    /// 边框宽度（默认 `ButtonStyle::border_w`）。
    pub fn border_w(mut self, w: f32) -> Self {
        self.border_w = Some(w);
        self
    }
    /// 圆角半径（逻辑像素；默认 `ButtonStyle::radius`）。
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = Some(r);
        self
    }
    /// 内边距（x = 水平，y = 垂直；默认 `ButtonStyle::padding`）。
    pub fn padding(mut self, p: Vec2) -> Self {
        self.padding = Some(p);
        self
    }
    /// 字号（默认 `ButtonStyle::font_size`）。
    pub fn font_size(mut self, s: f32) -> Self {
        self.font_size = Some(s);
        self
    }
    /// 字体族（默认 `ButtonStyle::font_family`）。
    pub fn font_family(mut self, f: &'a str) -> Self {
        self.font_family = Some(f);
        self
    }

    /// 主题样式 + 本控件覆盖 → 最终样式。
    fn resolve(&self, theme: &Theme) -> ButtonStyle {
        let base = &theme.button;
        ButtonStyle {
            bg: self.bg.unwrap_or(base.bg),
            bg_hover: self.bg_hover.unwrap_or(base.bg_hover),
            bg_pressed: self.bg_pressed.unwrap_or(base.bg_pressed),
            fg: self.color.unwrap_or(base.fg),
            border: self.border.unwrap_or(base.border),
            border_w: self.border_w.unwrap_or(base.border_w),
            radius: self.radius.unwrap_or(base.radius),
            padding: self.padding.unwrap_or(base.padding),
            font_size: self.font_size.unwrap_or(base.font_size),
            font_family: self
                .font_family
                .map(|f| f.to_owned())
                .or_else(|| base.font_family.clone()),
        }
    }
}

impl Widget for Button<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        let size = self.font_size.unwrap_or(ui.theme.button.font_size);
        let family = match self.font_family {
            Some(f) => Some(f.to_owned()),
            None => ui.theme.button.font_family.clone(),
        };
        let tsize = ui.text_size(self.label, size, family.as_deref());
        let pad = self.padding.unwrap_or(ui.theme.button.padding);
        Vec2::new(tsize.x + pad.x * 2.0, tsize.y + pad.y * 2.0)
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        let style = self.resolve(&ui.theme);
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

// ─── Checkbox ───────────────────────────────────────────────────

/// 勾选框控件（勾选值由调用方维护；属性化 builder，未设置回落全局 [`Theme::checkbox`]）。
pub struct Checkbox<'a> {
    id: &'a str,
    label: &'a str,
    checked: bool,
    fg: Option<Color>,
    box_border: Option<Color>,
    checked_fill: Option<Color>,
    font_size: Option<f32>,
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
    /// 字号（默认 `CheckboxStyle::font_size`）。
    pub fn font_size(mut self, s: f32) -> Self {
        self.font_size = Some(s);
        self
    }
    /// 字体族（默认 `CheckboxStyle::font_family`）。
    pub fn font_family(mut self, f: &'a str) -> Self {
        self.font_family = Some(f);
        self
    }

    fn resolve(&self, theme: &Theme) -> CheckboxStyle {
        let base = &theme.checkbox;
        CheckboxStyle {
            box_size: base.box_size,
            box_border: self.box_border.unwrap_or(base.box_border),
            checked_fill: self.checked_fill.unwrap_or(base.checked_fill),
            fg: self.fg.unwrap_or(base.fg),
            font_size: self.font_size.unwrap_or(base.font_size),
            font_family: self
                .font_family
                .map(|f| f.to_owned())
                .or_else(|| base.font_family.clone()),
            gap: base.gap,
        }
    }
}

impl Widget for Checkbox<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        let box_size = ui.theme.checkbox.box_size;
        let gap = ui.theme.checkbox.gap;
        let size = self.font_size.unwrap_or(ui.theme.checkbox.font_size);
        let family = match self.font_family {
            Some(f) => Some(f.to_owned()),
            None => ui.theme.checkbox.font_family.clone(),
        };
        let tsize = ui.text_size(self.label, size, family.as_deref());
        Vec2::new(box_size + gap + tsize.x, box_size.max(tsize.y))
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        let style = self.resolve(&ui.theme);
        let s = ui.checkbox_at_styled(self.id, rect, self.label, self.checked, &style);
        Response {
            hovered: s.hovered,
            pressed: s.pressed,
            clicked: s.clicked,
            released: false,
            toggled: s.toggled,
        }
    }
}
