//! 文本编辑器控件（**单行 / 多行**，属性化 builder；未设置的属性回落全局
//! [`Theme::input`](crate::style::Theme::input)）。
//!
//! # 为什么要有它
//!
//! `Ui::text_input` / `Ui::text_area` 一族只有"自动尺寸"和"显式 `Rect`"两种形态，
//! 想调一个属性（宽 / 高 / 字号 / 有没有缩放柄 / 要不要自动换行）就得换一个方法名——
//! 组合一多就变成方法爆炸。本控件把它们收成一个 **责任链 builder**：
//!
//! ```no_run
//! # let mut ui: rjw_ui::Ui = todo!();
//! # let mut name = String::new();
//! // 单行、固定宽、无缩放柄：
//! ui.add(rjw_ui::TextEditor::new("name", &mut name).width(rjw_ui::Size::Logical(200.0)));
//! ```
//!
//! ```no_run
//! # let mut ui: rjw_ui::Ui = todo!();
//! # let mut note = String::new();
//! // 多行、不自动换行、右下角可拖拽改宽高、字号 16：
//! ui.add(
//!     rjw_ui::TextEditor::new("note", &mut note)
//!         .multiline()
//!         .no_wrap()
//!         .resize(rjw_ui::Resize::Both)
//!         .font_size(16.0),
//! );
//! ```
//!
//! # 与旧 API 的关系
//!
//! 本 builder 是**唯一**的文本编辑入口：`Ui::text_input*` / `UiAdd::text_input*` 一族
//! 现在都只做"定矩形 + 调本控件"这一件事（见 [`crate::ui::Ui::text_input_at`]）。
//! 旧方法名全部保留（源码兼容），语义不变。
//!
//! # 交互
//!
//! 文本编辑器**自己**处理命中 / 焦点 / 选择拖拽 / 光标（`Response` 的 `hovered` /
//! `pressed` / `clicked` 因此恒为 `false`，只有 `rect` 有意义）——需要这些状态的
//! 调用方请用 [`Ui::text_input_at`](crate::Ui::text_input_at) 上层的
//! `WidgetState`（caret / 选择 / 滚动）查询。

use std::sync::Arc;

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use super::{Response, Widget};
use crate::draw::{CornerRadius, Size};
use crate::style::{Brush, InputStyle};
use crate::ui::{Resize, Ui};

// ─── TextEditor ─────────────────────────────────────────────────

/// **文本编辑器**（单行 / 多行；属性化 builder）。
///
/// 责任链配置（全部可选，未设置 = 回落 [`Theme::input`](crate::style::Theme::input)）：
///
/// | 属性 | 方法 | 说明 |
/// |---|---|---|
/// | 矩形 | [`at`](Self::at) | 显式 `Rect`（绝对定位，不占容器光标） |
/// | 尺寸 | [`width`](Self::width) / [`height`](Self::height) | 自动申请时的宽 / 高（默认：单行 `min_w`×`height`；多行 `max(min_w,200)`×`90`，**逻辑像素**） |
/// | 下限 / 上限 | [`min_size`](Self::min_size) / [`max_size`](Self::max_size) | 申请尺寸的 clamp（**物理像素**） |
/// | 多行 | [`multiline`](Self::multiline) | `Enter` 换行 / `↑↓` 跨行 / 垂直滚动 |
/// | 换行 | [`no_wrap`](Self::no_wrap) | 多行**不自动换行**（横向滚动跟随光标） |
/// | 缩放柄 | [`resize`](Self::resize) | `Resize::None`（默认）/ `Horizontal` / `Both` |
/// | 字号 / 字体 | [`font_size`](Self::font_size) / [`font_family`](Self::font_family) | |
/// | 颜色 | [`text_color`](Self::text_color) / [`caret_color`](Self::caret_color) / [`selection_color`](Self::selection_color) / [`preedit_color`](Self::preedit_color) | |
/// | 面板 | [`background`](Self::background) / [`border`](Self::border) / [`border_w`](Self::border_w) / [`radius`](Self::radius) / [`padding_x`](Self::padding_x) | |
///
/// 放置：容器内占光标 `ui.add(editor)`（自动尺寸）/ 绝对定位 `ui.add_at(pos, editor)`
/// 或 `.at(rect)`。
pub struct TextEditor<'a> {
    id: &'a str,
    value: &'a mut String,
    /// 显式矩形（`Some` = 绝对定位、**不占**容器光标；`None` = 自动申请）。
    rect: Option<Rect>,
    /// 多行（`Enter` 换行 / `↑↓` / 竖直滚动）。
    multiline: bool,
    /// 多行是否自动换行（单行恒不换行；`.no_wrap()` 置 `false`）。
    wrap: bool,
    /// 右下角缩放柄（`None` = 无）。
    resize: Resize,
    /// 自动申请时的宽 / 高（`None` = 主题默认公式）。
    width: Option<Size<f32>>,
    height: Option<Size<f32>>,
    /// 申请尺寸下限 / 上限（**物理像素**；同
    /// [`Ui::resizable_text_area_at`](crate::Ui::resizable_text_area_at) 的 `min`）。
    min_size: Option<Vec2>,
    max_size: Option<Vec2>,
    // ── 样式覆盖（None = 回落主题） ──
    font_size: Option<Size<f32>>,
    font_family: Option<Arc<str>>,
    fg: Option<Color>,
    bg: Option<Brush>,
    border: Option<Color>,
    border_focus: Option<Color>,
    caret: Option<Color>,
    selection: Option<Color>,
    preedit: Option<Color>,
    radius: Option<Size<CornerRadius>>,
    padding_x: Option<Size<f32>>,
    border_w: Option<Size<f32>>,
}

impl<'a> TextEditor<'a> {
    /// 新建文本编辑器（`id`：状态键 / 焦点 id；`value`：内容，就地读写）。
    pub fn new(id: &'a str, value: &'a mut String) -> Self {
        Self {
            id,
            value,
            rect: None,
            multiline: false,
            wrap: true,
            resize: Resize::None,
            width: None,
            height: None,
            min_size: None,
            max_size: None,
            font_size: None,
            font_family: None,
            fg: None,
            bg: None,
            border: None,
            border_focus: None,
            caret: None,
            selection: None,
            preedit: None,
            radius: None,
            padding_x: None,
            border_w: None,
        }
    }

    /// 显式矩形（绝对定位；**不占**容器光标，同
    /// [`Ui::text_input_at`](crate::Ui::text_input_at)）。
    pub fn at(mut self, rect: Rect) -> Self {
        self.rect = Some(rect);
        self
    }

    /// **多行**（`Enter` 换行 / `↑↓` 跨行 / `Home`·`End` / 垂直滚动）。
    pub fn multiline(mut self) -> Self {
        self.multiline = true;
        self
    }

    /// 多行**不自动换行**（行宽不限，超出内容区横向滚动跟随光标；显式 `\n` 分行）。
    pub fn no_wrap(mut self) -> Self {
        self.wrap = false;
        self
    }

    /// 右下角**缩放柄**（[`Resize::Horizontal`] 只调宽 / [`Resize::Both`] 宽高同调）。
    /// 尺寸跨帧持久于 [`UiState::sizes`](crate::UiState::sizes)，也受尺寸责任链
    /// （[`Ui::size_handler`](crate::Ui::size_handler)）约束。
    ///
    /// **柄的形状 / 颜色 / 尺寸来自主题** [`InputStyle::grip`](crate::style::InputStyle::grip)
    /// （默认 [`GripShape::Diagonal`](crate::style::GripShape::Diagonal) **三条斜线**；
    /// `Bars` = 三条横线，`Hidden` = "不画图案但仍能拖"）。
    pub fn resize(mut self, r: Resize) -> Self {
        self.resize = r;
        self
    }

    /// 自动申请时的**宽**（[`Size<f32>`]：逻辑（默认）/ 物理）。
    pub fn width(mut self, w: impl Into<Size<f32>>) -> Self {
        self.width = Some(w.into());
        self
    }

    /// 自动申请时的**高**（[`Size<f32>`]：逻辑（默认）/ 物理）。
    pub fn height(mut self, h: impl Into<Size<f32>>) -> Self {
        self.height = Some(h.into());
        self
    }

    /// 申请尺寸**下限**（物理像素；`resize` 时同时是拖拽下限）。
    ///
    /// **不调 = 主题下限**（`Vec2::new(InputStyle::min_w, InputStyle::height)` =
    /// 最小宽 + **一行文字的标准高**，与 [`Theme::row_h`](crate::Theme::row_h) 同一套
    /// 标准）——只有开了 `.resize(..)` 才生效。没有这条默认值就能把输入框拖到 0
    /// 并被 [`UiState::sizes`](crate::UiState::sizes) 持久下来。
    pub fn min_size(mut self, min: Vec2) -> Self {
        self.min_size = Some(min);
        self
    }

    /// 申请尺寸**上限**（物理像素）。
    pub fn max_size(mut self, max: Vec2) -> Self {
        self.max_size = Some(max);
        self
    }

    /// 字号（默认 `Theme::input.font_size`）。
    pub fn font_size(mut self, s: impl Into<Size<f32>>) -> Self {
        self.font_size = Some(s.into());
        self
    }

    /// 字体族（默认 `Theme::input.font_family`）。
    pub fn font_family(mut self, f: &str) -> Self {
        self.font_family = Some(Arc::from(f));
        self
    }

    /// 文本颜色（默认 `Theme::input.fg`）。
    pub fn text_color(mut self, c: Color) -> Self {
        self.fg = Some(c);
        self
    }

    /// 背景刷（默认 `Theme::input.bg`）。
    pub fn background(mut self, b: Brush) -> Self {
        self.bg = Some(b);
        self
    }

    /// 边框色（默认 `Theme::input.border`）。
    pub fn border(mut self, c: Color) -> Self {
        self.border = Some(c);
        self
    }

    /// 聚焦边框色（默认 `Theme::input.border_focus`）。
    pub fn border_focus(mut self, c: Color) -> Self {
        self.border_focus = Some(c);
        self
    }

    /// 光标颜色（默认 `Theme::input.caret`）。
    pub fn caret_color(mut self, c: Color) -> Self {
        self.caret = Some(c);
        self
    }

    /// 选择高亮色（默认 `Theme::input.sel_bg`）。
    pub fn selection_color(mut self, c: Color) -> Self {
        self.selection = Some(c);
        self
    }

    /// IME 组合候选串颜色（默认 `Theme::input.preedit`）。
    pub fn preedit_color(mut self, c: Color) -> Self {
        self.preedit = Some(c);
        self
    }

    /// 圆角（[`Size<CornerRadius>`]：逻辑（默认）/ 物理；默认 `Theme::input.radius`）。
    pub fn radius(mut self, r: impl Into<Size<CornerRadius>>) -> Self {
        self.radius = Some(r.into());
        self
    }

    /// 内容水平内边距（默认 `Theme::input.padding_x`）。
    pub fn padding_x(mut self, p: impl Into<Size<f32>>) -> Self {
        self.padding_x = Some(p.into());
        self
    }

    /// 边框宽（默认 `Theme::input.border_w`）。
    pub fn border_w(mut self, w: impl Into<Size<f32>>) -> Self {
        self.border_w = Some(w.into());
        self
    }

    /// **解析样式**（属性覆盖 / 主题回落 → 物理像素）：先把主题值拷出再逐项覆盖，
    /// 得到本控件这一帧真正用于绘制的 [`InputStyle`]。
    fn resolve_style(&self, ui: &Ui) -> InputStyle {
        let scale = ui.scale();
        let mut style = ui.theme().input.clone();
        if let Some(v) = self.font_size {
            style.font_size = v.to_physical(scale);
        }
        if let Some(f) = &self.font_family {
            style.font_family = Some(f.clone());
        }
        if let Some(v) = self.fg {
            style.fg = v;
        }
        if let Some(v) = self.bg {
            style.bg = v;
        }
        if let Some(v) = self.border {
            style.border = v;
        }
        if let Some(v) = self.border_focus {
            style.border_focus = v;
        }
        if let Some(v) = self.caret {
            style.caret = v;
        }
        if let Some(v) = self.selection {
            style.sel_bg = v;
        }
        if let Some(v) = self.preedit {
            style.preedit = v;
        }
        if let Some(v) = self.radius {
            style.radius = v.to_physical(scale);
        }
        if let Some(v) = self.padding_x {
            style.padding_x = v.to_physical(scale);
        }
        if let Some(v) = self.border_w {
            style.border_w = v.to_physical(scale);
        }
        style
    }

    /// **自动申请矩形**：默认尺寸 = 单行 `(min_w, height)` / 多行 `(max(min_w,200), 90)`
    /// （与 [`UiAdd::text_input`](crate::ui::UiAdd::text_input) /
    /// [`UiAdd::text_area`](crate::ui::UiAdd::text_area) 一致），再套 `.width` / `.height`
    /// 覆盖与 `.min_size` / `.max_size` 约束。
    ///
    /// ⚠ 开了 `.resize(..)` 时**必须先问尺寸责任链**（[`Ui::resolved_size`]）：上一帧
    /// 拖出来的尺寸存在 [`UiState::sizes`](crate::UiState::sizes)，不并进申请尺寸就会出现
    /// "画的是拖大的框、申请的却是默认尺寸"——窗口不跟着长、**下面的控件不动**、而
    /// 框自己溢出父级（用户报的"下面的控件不会跟着下去"）。
    fn allocate_rect(&self, ui: &mut Ui, style: &InputStyle) -> Rect {
        let scale = ui.scale();
        let (dw, dh) = if self.multiline {
            // ⚠ 必须 `× scale`：`style.*` 已经是**物理像素**（主题在下传前被 DPI 预乘），
            // 而这两个默认值是**逻辑像素**——混用会让不同 DPI 下默认尺寸不一致
            // （150% 下曾是 90 物理像素 = 60 逻辑像素，比文档写的 90 逻辑像素小 1/3）。
            (
                style.min_w.max(MULTILINE_DEF_W * scale),
                MULTILINE_DEF_H * scale,
            )
        } else {
            (style.min_w, style.height)
        };
        let default = Vec2::new(
            self.width.map_or(dw, |w| w.to_physical(scale)),
            self.height.map_or(dh, |h| h.to_physical(scale)),
        );
        // 责任链 / 用户拖拽持久值：只有可缩放控件才有那条跨帧记录。
        let persisted = (self.resize != Resize::None).then(|| ui.resolved_size(self.id, default));
        let size = resolve_editor_size(
            default,
            self.min_size.or_else(|| self.resize_min(style)),
            self.max_size,
            persisted,
        );
        ui.allocate(size)
    }

    /// **本控件的尺寸下限**（物理像素）：显式 `.min_size(..)` 优先，否则给了
    /// `.resize(..)` 就用主题下限（[`default_min`]），不可缩放时无下限。
    fn resize_min(&self, style: &InputStyle) -> Option<Vec2> {
        (self.resize != Resize::None).then(|| default_min(style))
    }
}

impl Widget for TextEditor<'_> {
    /// **多行 ⇒ [`SizeClass::Multiline`]**：水平行（`row`）里多行编辑器可以**撑高整行**
    /// （行高 = 编辑器高），单行输入框仍被钉到标准行高（文字中心线对齐）。
    fn size_class(&self) -> super::SizeClass {
        if self.multiline {
            super::SizeClass::Multiline
        } else {
            super::SizeClass::SingleLine
        }
    }

    /// 定矩形（`.at` / 自动申请）→ 逐控件样式覆盖 → 交给文本编辑核心。
    fn ui(self, ui: &mut Ui) -> Response {
        let style = self.resolve_style(ui);
        // 先定矩形：`.at(rect)` 绝对定位；否则按默认尺寸 + 覆盖 / 约束申请（占光标）。
        let rect = match self.rect {
            Some(r) => r,
            None => self.allocate_rect(ui, &style),
        };
        // 样式覆盖经**帧内主题**下传：核心读 `ui.theme.input`，因此这里临时替换、
        // 调用后还原——调用方看到的主题不变（帧内后续控件不受影响）。
        let saved = std::mem::replace(&mut ui.theme.input, style);
        if self.resize != Resize::None {
            // 缩放柄路径：尺寸责任链 + 拖拽（核心只负责绘制文本）。
            // 下限与 `allocate_rect` **同源**（都不调 `.min_size` ⇒ 主题下限：
            // 最小宽 + 一行文字高）——两处不一致就会"申请尺寸有下限、拖拽却能拖到 0"。
            let min = self.min_size.unwrap_or_else(|| default_min(&ui.theme().input));
            if self.multiline {
                ui.resizable_text_area_at(self.id, rect, self.value, min, self.resize);
            } else {
                ui.resizable_text_input_at(self.id, rect, self.value, min.x, self.resize);
            }
        } else if self.multiline {
            ui.text_area_impl(self.id, rect, self.value, self.wrap);
        } else {
            ui.text_input_core(self.id, rect, self.value);
        }
        ui.theme.input = saved;
        Response { rect, ..Default::default() }
    }
}

// ─── 尺寸解算（纯函数，可单测） ─────────────────────────────────

/// 多行编辑器**默认宽**（**逻辑像素**；`allocate_rect` 里 × scale ⇒ 物理像素）。
const MULTILINE_DEF_W: f32 = 200.0;

/// 多行编辑器**默认高**（**逻辑像素**；同 [`MULTILINE_DEF_W`]）。
const MULTILINE_DEF_H: f32 = 90.0;

/// **开了 `.resize(..)` 且没调 `.min_size(..)` 时的默认下限**（物理像素）：
/// 最小宽 = [`InputStyle::min_w`]，最小高 = [`InputStyle::height`]（**一行文字的标准高**，
/// 与 `Theme::row_h` 同一套标准）。
fn default_min(style: &InputStyle) -> Vec2 {
    Vec2::new(style.min_w, style.height)
}

/// **尺寸解算**（纯函数）：默认（已含 `.width/.height` 覆盖）→ 责任链 / 用户拖拽持久值
/// → 压 `max` → 抬 `min`。
///
/// 顺序理由：
/// - 持久值必须**在** `min/max` 之间自由取值（拖大拖小都生效）；
/// - `min/max` 是**声明式约束**，不能被持久值绕过；
/// - `max` 先于 `min` ⇒ `min > max` 时 **min 胜**，与
///   [`apply_constraints`](crate::widgets::apply_constraints) 的口径一致。
fn resolve_editor_size(
    default: Vec2,
    min: Option<Vec2>,
    max: Option<Vec2>,
    persisted: Option<Vec2>,
) -> Vec2 {
    let mut size = persisted.unwrap_or(default);
    if let Some(m) = max {
        size = size.min(m);
    }
    if let Some(m) = min {
        size = size.max(m);
    }
    size
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> InputStyle {
        // 150% DPI 下的典型值（主题在 `Ui` 内已被 DPI 预乘）。
        InputStyle {
            min_w: 210.0,
            height: 39.0,
            ..InputStyle::default()
        }
    }

    #[test]
    fn default_min_is_min_width_and_one_line_height() {
        assert_eq!(default_min(&style()), Vec2::new(210.0, 39.0));
    }

    #[test]
    fn persisted_size_wins_over_default() {
        let got = resolve_editor_size(
            Vec2::new(300.0, 135.0),
            None,
            None,
            Some(Vec2::new(600.0, 400.0)),
        );
        assert_eq!(got, Vec2::new(600.0, 400.0), "拖大的尺寸必须原样进申请尺寸");
    }

    #[test]
    fn min_and_max_clamp_the_persisted_size() {
        let min = Vec2::new(210.0, 39.0);
        let max = Vec2::new(800.0, 600.0);
        // 拖到比下限还小（例如历史遗留的 0×0）⇒ 抬到下限。
        let small = resolve_editor_size(
            Vec2::new(300.0, 135.0),
            Some(min),
            Some(max),
            Some(Vec2::ZERO),
        );
        assert_eq!(small, min, "持久值不能被允许突破下限（拖到 0 的旧值也要被抬回来）");
        // 超过上限 ⇒ 压到上限。
        let big = resolve_editor_size(
            Vec2::new(300.0, 135.0),
            Some(min),
            Some(max),
            Some(Vec2::new(4000.0, 4000.0)),
        );
        assert_eq!(big, max);
    }

    #[test]
    fn min_wins_when_min_exceeds_max() {
        // 与 `apply_constraints` 同一口径：先压 max 再抬 min ⇒ min 胜。
        let got = resolve_editor_size(
            Vec2::new(100.0, 100.0),
            Some(Vec2::new(400.0, 400.0)),
            Some(Vec2::new(200.0, 200.0)),
            None,
        );
        assert_eq!(got, Vec2::new(400.0, 400.0));
    }

    #[test]
    fn no_persisted_no_constraints_keeps_default() {
        assert_eq!(
            resolve_editor_size(Vec2::new(300.0, 135.0), None, None, None),
            Vec2::new(300.0, 135.0)
        );
    }
}
