//! 控件 trait 与**属性化 builder**（非宏、可调试的控件扩展方式）。
//!
//! # 为什么
//!
//! 旧控件 API 由 `widget_api!` 宏统一生成（`p.label` / `p.button` …）——加一个新控件要
//! 改宏（报错指向宏展开、难以调试），且控件属性（颜色 / 字号 / 字体 / 内边距）只能
//! 跟随全局 [`Theme`]（`crate::style::Theme`），无法逐控件覆盖。该宏现已移除，
//! 容器便捷方法统一由 [`crate::ui::UiAdd`] trait 提供（默认方法，无需宏展开）。
//!
//! 本模块提供：
//! - [`Widget`] trait：**新控件 = 一个实现该 trait 的 builder 结构体**——普通 Rust，
//!   无宏展开，报错定位精确，可单测；
//! - 属性化 builder：[`Label`] / [`Button`] / [`Checkbox`]——`Option` 覆盖字段 + 链式
//!   setter，未设置的属性回落到全局 [`Theme`]；
//! - 统一响应 [`Response`]（hover / pressed / clicked / released / toggled）；
//! - 放置方式：[`Ui::add`]（容器内占光标）/ [`Ui::add_at`]（绝对定位）；容器包装
//!   （`Panel` / `Pack` / `Grid` / `Window` / `Scroll` / `FlexCtx`）经
//!   [`crate::ui::UiAdd`] 提供同样的 `add` / `add_at` 与全部便捷方法。
//!
//! # 用法
//!
//! ```no_run
//! # let viewport = todo!(); let mouse = todo!(); let keyboard = todo!();
//! # let text = todo!(); let r2d = todo!(); let state = todo!(); let window = todo!();
//! use rjw_color::Color;
//! use rjw_ui::{Button, Label, Theme, Ui, UiAdd};
//! let mut ui = Ui::begin(&window, &mut text, &mut state)
//!     .capture(&mouse, &keyboard)
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
//! ui.finish(&viewport, &mut r2d);
//! ```
//!
//! # 添加一个新控件
//!
//! 1. 定义 builder 结构体（属性 = `Option<T>` 覆盖字段）；
//! 2. `impl Widget`：`size` 测量内容（可调 `ui.text_size` 等），`ui` 在给定矩形内
//!    渲染 + 交互（用 `Ui` 的**公开原语**，或复用现有 `*_at` 方法）；
//! 3. 需要复用现有控件时，优先给 `Ui` 增加 `xxx_at_styled(…, &Style)` 变体并让旧方法
//!    委托它（见 `button_at` / `checkbox_at` 的改造方式）。
//!
//! # 跨 crate 自定义控件模板（只用公开 API）
//!
//! `Widget` trait、`Ui` 的测量/命中/焦点/绘制原语、`UiState`/`WidgetState` 交互状态
//! 全部公开——下面两个模板（滑块 + 数字输入，含**拖动调值**）只依赖公开接口，
//! 可原样放进你自己的 crate：
//!
//! ```no_run
//! use std::ops::RangeInclusive;
//! use glam::Vec2;
//! use rjw_transform::Rect;
//! use rjw_ui::draw::TextVAlign;
//! use rjw_ui::hit::update_drag;
//! use rjw_ui::{FocusKind, IdAbsolute, Response, TextAlign, Ui, Widget};
//!
//! /// 滑块（模板）：尺寸取主题，交互/绘制全部委托现有原语 [`Ui::slider_at`]——
//! /// 与内置控件同一条路径（"现有控件能使用接口就使用接口"）。
//! pub struct Slider<'a> {
//!     id: &'a str,
//!     range: RangeInclusive<f32>,
//!     value: f32,
//! }
//! impl<'a> Slider<'a> {
//!     pub fn new(id: &'a str, range: RangeInclusive<f32>, value: f32) -> Self {
//!         Self { id, range, value }
//!     }
//! }
//! impl Widget for Slider<'_> {
//!     fn size(&self, ui: &mut Ui) -> Vec2 {
//!         Vec2::new(ui.theme.slider.min_w.max(40.0), ui.theme.slider.height)
//!     }
//!     fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
//!         let _new = ui.slider_at(self.id, rect, self.range, self.value);
//!         Response::default()
//!     }
//! }
//!
//! /// 数字输入（模板）：文本框 + 右侧拖拽手柄（按住上下拖动调值）。
//! /// 演示公开交互原语：`hit_abs` / `mouse_left` / `register_focus` /
//! /// `update_drag` + `WidgetState` 拖拽基准 + `push_*` 绘制原语。
//! pub struct NumberInput<'a> {
//!     id: &'a str,
//!     value: f32,
//! }
//! impl<'a> NumberInput<'a> {
//!     pub fn new(id: &'a str, value: f32) -> Self {
//!         Self { id, value }
//!     }
//! }
//! impl Widget for NumberInput<'_> {
//!     fn size(&self, ui: &mut Ui) -> Vec2 {
//!         Vec2::new(ui.theme.input.min_w, ui.theme.input.height)
//!     }
//!     fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
//!         // 1) 交互基础（公开原语）；状态键 / 焦点用**绝对 ID**（`ui.id_for(..)` 解析，
//!         //    容器内自动带前缀——详见 [`crate::id`] 模块）。
//!         let id_for = ui.id_for(self.id);
//!         let mouse = ui.mouse_logical();
//!         let hit = ui.hit_abs(&rect);
//!         let btn = ui.mouse_left();
//!         ui.register_focus(&id_for, rect, FocusKind::TextInput);
//!         // 自身有拖拽语义 → 声明按下归属：阻止外层窗口把本次按下当作窗口拖拽基准
//!         // （窗口内拖手柄不再连窗口一起动）。
//!         if btn.down_edge() && hit {
//!             ui.claim_press();
//!         }
//!         // 2) 拖拽调值：**向上拖 = 增加**（y 减小 → 值增大）；拖动 1 逻辑像素 = 0.1。
//!         //    ⚠ 拖拽基准存在**独立的**状态 ID（`{id}::grip`）——与 text_input_at
//!         //    共用同一 WidgetState 会把 press_mouse 互相覆盖（拖拽失灵）。
//!         let mut value = self.value;
//!         let drag_id = IdAbsolute::owned(format!("{}::grip", id_for.as_str()));
//!         {
//!             let ws = ui.state_mut().widget(&drag_id);
//!             let dragging = update_drag(ws, hit, btn);
//!             if btn.down_edge() && hit {
//!                 ws.press_mouse = Some(mouse);
//!                 ws.press_panel = Some(Vec2::new(0.0, value));
//!             }
//!             if dragging {
//!                 let pm = ws.press_mouse.unwrap_or(mouse);
//!                 let base = ws.press_panel.unwrap_or(Vec2::ZERO).y;
//!                 value = (base - (mouse.y - pm.y) * 0.1).round();
//!             }
//!         }
//!         // 拖动手柄悬停/拖拽 → ↔ 光标（EwResize）；点击文本框输入 → 内置 I 型
//!         if hit {
//!             ui.set_cursor(rjw_ui::UiCursor::EwResize);
//!         }
//!         // 3) 文本框：`text_input_at` 直接写**持久** `&mut String`（每帧重格式化会
//!         //    把打字冲掉——"无法输入"）；真实控件还需**屏蔽非数字输入**
//!         //    （`retain` 只留数字/负号/小数点）并解析回数值。
//!         let mut buf = format!("{value:.1}");
//!         ui.text_input_at(self.id, rect, &mut buf);
//!         // 主题值先拷出（Copy），避免与下方 &mut ui 调用（push_*）的借用冲突
//!         let (border, fg, font_size) = {
//!             let st = &ui.theme.input;
//!             (st.border, st.fg, st.font_size)
//!         };
//!         let grip = Rect::new(rect.x + rect.w - 14.0, rect.y, 14.0, rect.h);
//!         ui.push_panel_like(grip, border, border, 1.0, 0.0, 1);
//!         ui.push_text_rect(
//!             grip,
//!             "≡",
//!             font_size,
//!             fg,
//!             None,
//!             TextAlign::Center,
//!             TextVAlign::Center,
//!             None,
//!             None,
//!         );
//!         Response::default()
//!     }
//! }
//!
//! // 放置：容器内占光标（add）/ 顶层绝对定位（add_at）
//! # let mut ui: rjw_ui::Ui = todo!();
//! ui.add(Slider::new("vol", 0.0..=1.0, 0.5));
//! ui.add(NumberInput::new("hp", 100.0));
//! ```
//!
//! 详见 `docs/WIDGET_GUIDE.md`。

// 以下需求已实现（见 rjw_ui / examples/eg260818UI）：
// - 多行文本：自动换行（Ui::text_area_at）与不自动换行 + 水平滚动（Ui::text_area_at_nw）模式
// - 不同字体：默认、SimHei、Sarasa Mono SC（demo 顶层 combo，Theme::with_font_family 级联）
// - 数字输入框：拖动手柄整体调值（EwResize 光标，向上拖 = 增加），点击文本框 I 型光标输入
//   （非数字输入被屏蔽；拖拽基准用独立状态 ID `{id}::grip`）
// - 窗体：悬停、拖动时保持普通 Arrow（光标由 set_cursor / 内置规则管理）

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::draw::{Size, TextAlign, TextVAlign};
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

/// **控件尺寸约束**（每轴可选；`None` = 该轴不约束）。见 [`Widget::constraints`]。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizeConstraints {
    pub min_w: Option<f32>,
    pub max_w: Option<f32>,
    pub min_h: Option<f32>,
    pub max_h: Option<f32>,
}

/// **控件膨胀模式**（内容尺寸相对父级空间的行为）。见 [`Widget::expansion`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expansion {
    /// 内容按自身尺寸（clamp min/max），**不撑大父级**——内容可能溢出父级，
    /// 由控件用 noclip 绘制 / 省略自洽（如装饰性分隔线）。
    DisableAutoExpansion,
    /// **限制在父级可用空间内**：取 min(内容, 沙箱可用宽，`Ui::avail_w`)，超出
    /// 部分由控件自处理（Label 自动换行 / "…"省略、Button 省略、TextArea 滚动）；
    /// 无可用空间时退化为 [`Expansion::UnlimitedExpansion`]。
    LimitedInParent,
    /// 内容自然尺寸（clamp min/max），**撑大父级**（默认，DOM 语义）。
    UnlimitedExpansion,
}

impl Default for Expansion {
    fn default() -> Self {
        Self::UnlimitedExpansion
    }
}

/// **控件 trait**：新控件 = 实现此 trait 的 builder 结构体（普通 Rust，无宏）。
///
/// - [`Widget::size`]：期望尺寸（**逻辑像素**；内容测量可调用 `ui.text_size` /
///   `ui.text_size_wrap`，或读取样式常量）；
/// - [`Widget::ui`]：在分配好的矩形内渲染 + 交互，返回 [`Response`]。
///
/// 放置：[`Ui::add`]（容器内占光标）/ [`Ui::add_at`]（绝对定位）；容器包装经
/// [`crate::ui::UiAdd`] 提供同样的 `add` / `add_at` 与全部便捷方法（`p.button` 等）。
pub trait Widget {
    /// 期望尺寸（逻辑像素；内容测量可经 `&mut Ui` 排版/缓存）。
    fn size(&self, ui: &mut Ui) -> Vec2;

    /// 在 `rect`（相对当前容器内容原点，逻辑像素）内渲染 + 交互。
    fn ui(self, ui: &mut Ui, rect: Rect) -> Response;

    /// **尺寸约束**（每轴 `Option<f32>`；默认全 `None`，不约束）。
    /// `Ui::add` 在布局前对 `size()` 结果按此 clamp。
    fn constraints(&self) -> SizeConstraints {
        SizeConstraints::default()
    }

    /// **膨胀模式**（默认 [`Expansion::UnlimitedExpansion`]）：
    /// 决定内容是否撑大父级 / 是否限制在父级可用空间内（见 [`Expansion`]）。
    fn expansion(&self) -> Expansion {
        Expansion::UnlimitedExpansion
    }

    /// **可选拖拽缩放范围** `(min, max)`（逻辑像素；默认 `None` = 不可缩放）。
    ///
    /// 声明后控件应在 `size()` 中优先读持久尺寸（`ui.state().sizes`，首次 = 内容
    /// 自然尺寸），并在 `ui()` 里调用 [`Ui::resize_handle`] 处理右下角缩放柄并
    /// 写回 [`UiState::sizes`]。内置 `window_at_w` / `window_at_strict_w` 已演示
    /// 该模式（宽度缩放）。
    fn resizable(&self) -> Option<(Vec2, Vec2)> {
        None
    }
}

/// 应用尺寸约束：`natural` 每轴 clamp 到 `min`/`max`（纯函数，可单测）。
/// 顺序 = 先压 max 再抬 min（**min 恒优先**：`min > max` 时结果为 min）。
#[inline]
pub fn apply_constraints(natural: Vec2, c: SizeConstraints) -> Vec2 {
    let clamp = |v: f32, lo: Option<f32>, hi: Option<f32>| {
        let v = match hi {
            Some(hi) => v.min(hi),
            None => v,
        };
        match lo {
            Some(lo) => v.max(lo),
            None => v,
        }
    };
    Vec2::new(
        clamp(natural.x, c.min_w, c.max_w),
        clamp(natural.y, c.min_h, c.max_h),
    )
}

// ─── 控件 ID（Label 派生 / 字符串 / 数字） ──────────────────────

/// 控件 ID（跨帧状态键，如勾选框勾选状态）的三种来源。
///
/// - [`WidgetId::Label`]：用 **label 文本本身**作 ID——同容器内标签唯一时最简；
/// - [`WidgetId::String`]：显式字符串 ID（原 `&str` 参数）；
/// - [`WidgetId::Int`]：数字 ID（如列表行索引 `i as u64`）。
///
/// 便捷转换（[`From`]）：`None` → `Label`；`Some("id")` / `"id"` → `String`；
/// `42u64` → `Int`。示例见 [`crate::ui::UiAdd::checkbox_mut`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetId<'a> {
    /// ID = label 文本本身（唯一标签无需显式 ID）。
    Label,
    /// 显式字符串 ID。
    String(&'a str),
    /// 数字 ID。
    Int(u64),
}

impl<'a> From<Option<&'a str>> for WidgetId<'a> {
    #[inline]
    fn from(id: Option<&'a str>) -> Self {
        match id {
            Some(s) => WidgetId::String(s),
            None => WidgetId::Label,
        }
    }
}
impl<'a> From<&'a str> for WidgetId<'a> {
    #[inline]
    fn from(s: &'a str) -> Self {
        WidgetId::String(s)
    }
}
impl From<u64> for WidgetId<'_> {
    #[inline]
    fn from(i: u64) -> Self {
        WidgetId::Int(i)
    }
}

impl WidgetId<'_> {
    /// 解析为实际 ID 字符串（`Label` 用 label 文本；`Int` 转十进制）。
    pub(crate) fn resolve(&self, label: &str) -> String {
        match self {
            WidgetId::Label => label.to_owned(),
            WidgetId::String(s) => (*s).to_owned(),
            WidgetId::Int(i) => i.to_string(),
        }
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
}

impl Widget for Label<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        let size = self
            .font_size
            .map(|s| s.to_physical(ui.scale()))
            .unwrap_or(ui.theme.label.font_size);
        let family = match self.font_family {
            Some(f) => Some(f.to_owned()),
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
        // 先把主题值拷出（Copy / owned），避免主题借用与下方 &mut ui 调用冲突
        let color = self.color.unwrap_or(ui.theme.label.color);
        let size = self
            .font_size
            .map(|s| s.to_physical(ui.scale()))
            .unwrap_or(ui.theme.label.font_size);
        let align = ui.theme.label.align;
        let family = match self.font_family {
            Some(f) => Some(f.to_owned()),
            None => ui.theme.label.font_family.clone(),
        };
        if self.ellipsis {
            // 省略模式：文本超出分配宽度 → "…"截断（内容自洽：宽 = rect.w，noclip）。
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
        } else {
            // 换行标签：直接传预排版缓冲（wrap 宽度参与缓存键），保证渲染与测量一致。
            // 默认自动换行（LimitedInParent）：`size()` 已按可用宽测量换行高度，渲染
            // **必须用同一宽度的换行缓冲**（rect.w），否则"逻辑换行、渲染仍溢出"。
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
                .map(|f| f.to_owned())
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
            Some(f) => Some(f.to_owned()),
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
            box_border: self.box_border.unwrap_or(base.box_border),
            border_w: base.border_w,
            checked_fill: self.checked_fill.unwrap_or(base.checked_fill),
            fg: self.fg.unwrap_or(base.fg),
            font_size: self.font_size.map(|s| s.to_physical(scale)).unwrap_or(base.font_size),
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
        let size = self
            .font_size
            .map(|s| s.to_physical(ui.scale()))
            .unwrap_or(ui.theme.checkbox.font_size);
        let family = match self.font_family {
            Some(f) => Some(f.to_owned()),
            None => ui.theme.checkbox.font_family.clone(),
        };
        let tsize = ui.text_size(self.label, size, family.as_deref());
        Vec2::new(box_size + gap + tsize.x, box_size.max(tsize.y))
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        let style = self.resolve(&ui.theme, ui.scale());
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

/// **滑块 builder**（链式：拖拽精度 / Shift·Ctrl 速度）。放置：`ui.add(Slider::new(..))`
/// 或 `p.add(..)`（占光标）。交互与绘制委托 [`Ui::slider_at_drag`]（增量拖拽，
/// 点击轨道即定位）。
///
/// ```no_run
/// # use rjw_ui::{Slider, Ui};
/// # let mut ui: rjw_ui::Ui = todo!();
/// # let mut vol: f32 = 0.5;
/// ui.add(
///     Slider::new("vol", 0.0..=1.0, &mut vol)
///         .drag_sensitivity(2.0)   // 每像素 2× 全值/宽（更快）
///         .shift_speed(10.0)       // 按住 Shift 拖拽 ×10
///         .ctrl_speed(0.1),        // 按住 Ctrl 拖拽 ×0.1
/// );
/// ```
pub struct Slider<'a> {
    id: &'a str,
    range: std::ops::RangeInclusive<f32>,
    value: &'a mut f32,
    /// 拖拽精度（每像素数值倍率；默认 1 = 值随鼠标 1:1）。
    drag_sensitivity: f32,
    /// 按住 Shift 拖拽速度倍率（默认 10）。
    shift_speed: f32,
    /// 按住 Ctrl 拖拽速度倍率（默认 0.1）。
    ctrl_speed: f32,
}

impl<'a> Slider<'a> {
    pub fn new(id: &'a str, range: std::ops::RangeInclusive<f32>, value: &'a mut f32) -> Self {
        Self { id, range, value, drag_sensitivity: 1.0, shift_speed: 10.0, ctrl_speed: 0.1 }
    }
    /// 拖拽**精度**：每像素数值倍率（默认 1 = 值随鼠标 1:1；`> 1` 更快、`< 1` 更慢）。
    pub fn drag_sensitivity(mut self, s: f32) -> Self {
        self.drag_sensitivity = s;
        self
    }
    /// 按住 **Shift** 拖拽速度倍率（默认 10）。
    pub fn shift_speed(mut self, s: f32) -> Self {
        self.shift_speed = s;
        self
    }
    /// 按住 **Ctrl** 拖拽速度倍率（默认 0.1）。
    pub fn ctrl_speed(mut self, s: f32) -> Self {
        self.ctrl_speed = s;
        self
    }
}

impl Widget for Slider<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        Vec2::new(ui.theme.slider.min_w.max(40.0), ui.theme.slider.height)
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        // 速度倍率：Shift = ×shift_speed（默认 10），Ctrl = ×ctrl_speed（默认 0.1）。
        let shift = ui.key_down(winit::keyboard::KeyCode::ShiftLeft)
            || ui.key_down(winit::keyboard::KeyCode::ShiftRight);
        let ctrl = ui.key_down(winit::keyboard::KeyCode::ControlLeft)
            || ui.key_down(winit::keyboard::KeyCode::ControlRight);
        let speed = if shift {
            self.shift_speed
        } else if ctrl {
            self.ctrl_speed
        } else {
            1.0
        };
        *self.value = ui.slider_at_drag(
            self.id,
            rect,
            self.range,
            *self.value,
            self.drag_sensitivity * speed,
        );
        Response::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_id_resolve() {
        // Label：以 label 文本为 ID
        assert_eq!(WidgetId::Label.resolve("窗口 A 选项"), "窗口 A 选项");
        // String：显式字符串 ID（忽略 label）
        assert_eq!(WidgetId::String("win_b_cb").resolve("任意"), "win_b_cb");
        // Int：数字 ID
        assert_eq!(WidgetId::Int(7).resolve("任意"), "7");
        // From 转换：None → Label、Some/&str → String、u64 → Int
        assert_eq!(WidgetId::from(None::<&str>).resolve("label-x"), "label-x");
        assert_eq!(WidgetId::from(Some("id-y")).resolve("label-x"), "id-y");
        assert_eq!(WidgetId::from("id-z").resolve("label-x"), "id-z");
        assert_eq!(WidgetId::from(42u64).resolve("label-x"), "42");
        // 区分度：同容器内不同 label 的 Label ID 互不相同
        assert_ne!(
            WidgetId::Label.resolve("窗口 A"),
            WidgetId::Label.resolve("窗口 B")
        );
    }

    #[test]
    fn apply_constraints_clamps_each_axis() {
        // 无约束：原样
        assert_eq!(apply_constraints(Vec2::new(10.0, 20.0), SizeConstraints::default()), Vec2::new(10.0, 20.0));
        // min 抬升
        let c = SizeConstraints { min_w: Some(30.0), min_h: Some(40.0), ..Default::default() };
        assert_eq!(apply_constraints(Vec2::new(10.0, 20.0), c), Vec2::new(30.0, 40.0));
        // max 压缩
        let c = SizeConstraints { max_w: Some(50.0), max_h: Some(60.0), ..Default::default() };
        assert_eq!(apply_constraints(Vec2::new(100.0, 200.0), c), Vec2::new(50.0, 60.0));
        // 单轴独立
        let c = SizeConstraints { max_w: Some(50.0), ..Default::default() };
        assert_eq!(apply_constraints(Vec2::new(100.0, 20.0), c), Vec2::new(50.0, 20.0));
        // min > max 时 min 优先（clamp 顺序）
        let c = SizeConstraints { min_w: Some(80.0), max_w: Some(50.0), ..Default::default() };
        assert_eq!(apply_constraints(Vec2::new(10.0, 0.0), c), Vec2::new(80.0, 0.0));
    }

    #[test]
    fn expansion_default_is_unlimited() {
        assert_eq!(Expansion::default(), Expansion::UnlimitedExpansion);
    }
}
