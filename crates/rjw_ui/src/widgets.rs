//! 控件 trait 与**属性化 builder**（非宏、可调试的控件扩展方式）+ 内置组合控件。
//!
//! # 为什么
//!
//! 旧控件 API 由 `widget_api!` 宏统一生成（`p.label` / `p.button` …）——加一个新控件要
//! 改宏（报错指向宏展开、难以调试），且控件属性（颜色 / 字号 / 字体 / 内边距）只能
//! 跟随全局 [`Theme`](crate::style::Theme)，无法逐控件覆盖。该宏现已移除，
//! 容器便捷方法统一由 [`crate::ui::UiAdd`] trait 提供（默认方法，无需宏展开）。
//!
//! 本模块提供：
//! - [`Widget`] trait：**新控件 = 一个实现该 trait 的 builder 结构体**——普通 Rust，
//!   无宏展开，报错定位精确，可单测；
//! - 属性化 builder（**按控件拆分到独立子文件**）：[`label`]`::Label` /
//!   [`button`]`::Button` / [`checkbox`]`::Checkbox` / [`divider`]`::Divider` /
//!   [`slider`]`::Slider`——`Option` 覆盖字段 + 链式 setter，未设置的属性回落到全局
//!   [`Theme`](crate::style::Theme)；
//! - 内置**组合控件**：`numberinput`（[`NumberInput`]）/ `fontmodal`（[`FontModal`]），
//!   由基础原语组合而成，同时是"跨 crate 自定义控件"的真实范例；
//! - 统一响应 [`Response`]（hover / pressed / clicked / released / toggled）；
//! - 放置方式：[`Ui::add`](crate::ui::Ui::add)（容器内占光标）/
//!   [`Ui::add_at`](crate::ui::Ui::add_at)（绝对定位）；容器包装（`Panel` / `Pack` /
//!   `Grid` / `Window` / `Scroll` / `FlexCtx`）经 [`crate::ui::UiAdd`] 提供同样的
//!   `add` / `add_at` 与全部便捷方法。
//!
//! `lib.rs` 在 crate 根重导出全部控件（`rjw_ui::Button` 等），下游通常无需直接引用
//! 本模块路径。

use glam::Vec2;
use rjw_transform::Rect;

use crate::state::ButtonState;
use crate::ui::Ui;

mod button;
mod checkbox;
mod divider;
mod fontmodal;
mod label;
mod numberinput;
mod slider;

pub use button::Button;
pub use checkbox::Checkbox;
pub use divider::Divider;
pub use fontmodal::FontModal;
pub use label::Label;
pub use numberinput::NumberInput;
pub use slider::Slider;

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
#[derive(Default)]
pub enum Expansion {
    /// 内容按自身尺寸（clamp min/max），**不撑大父级**——内容可能溢出父级，
    /// 由控件用 noclip 绘制 / 省略自洽（如装饰性分隔线）。
    DisableAutoExpansion,
    /// **限制在父级可用空间内**：取 min(内容, 沙箱可用宽，`Ui::avail_w`)，超出
    /// 部分由控件自处理（Label 自动换行 / "…"省略、Button 省略、TextArea 滚动）；
    /// 无可用空间时退化为 [`Expansion::UnlimitedExpansion`]。
    LimitedInParent,
    /// 内容自然尺寸（clamp min/max），**撑大父级**（默认，DOM 语义）。
    #[default]
    UnlimitedExpansion,
}


/// **控件 trait**：新控件 = 实现此 trait 的 builder 结构体（普通 Rust，无宏）。
///
/// - [`Widget::size`]：期望尺寸（**逻辑像素**；内容测量可调用 `ui.text_size` /
///   `ui.text_size_wrap`，或读取样式常量）；
/// - [`Widget::ui`]：在分配好的矩形内渲染 + 交互，返回 [`Response`]。
///
/// 放置：[`Ui::add`](crate::ui::Ui::add)（容器内占光标）/
/// [`Ui::add_at`](crate::ui::Ui::add_at)（绝对定位）；容器包装经
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
