//! 滑块 builder（链式：拖拽精度 / Shift·Ctrl 速度），交互与绘制委托 `Ui::slider_at_drag`。
//!
//! # 泛型数值类型
//!
//! [`Slider::new`] 对**范围与值**的类型 `T: SliderValue` 泛型——`f32` / `f64` /
//! 全部有符号与无符号整数都能直接用：
//!
//! ```no_run
//! # use rjw_ui::{Slider, Ui};
//! # let (mut ui, mut gain, mut hp, mut count): (Ui, f32, u32, f64) = (todo!(), 0.0, 0, 0.0);
//! ui.add(Slider::new("gain", 0.0..=1.0, &mut gain));      // f32
//! ui.add(Slider::new("hp", 0..=100, &mut hp));            // u32：自动取整
//! ui.add(Slider::new("mix", 0.0..=1.0, &mut count));      // f64
//! ```
//!
//! 内部仍用 `f32` 做拖拽数学（与渲染 / 命中同单位），进出各换算一次：
//! 整数类型在 [`SliderValue::from_f32`] 里**四舍五入**，于是范围是整数时滑条自然
//! 一格一格跳；`f32` 不量化、`f64` 走 `f32` 中转（滑条的分辨率本来就受物理像素限制，
//! 双精度只是接口便利）。

use glam::Vec2;
use crate::ui::Ui;
use super::{Response, Widget};

/// **滑块 / 数字输入框可用的数值类型**。
///
/// 滑块内部一律用 `f32` 做拖拽数学（与渲染 / 命中同单位）；**数字输入框**内部用 `f64`
/// （它的精度要求更高：`step = 0.01` 配 `1000` 这种组合在 `f32` 里已经丢位，见
/// [`crate::NumberInput`]）。
///
/// 已为 `f32` / `f64` / `i8..i64` / `isize` / `u8..u64` / `usize` 实现。自定义类型
/// （定点数 / 角度包装等）也可实现：**只需**给出 [`Self::to_f32`] 与 [`Self::from_f32`]，
/// 其余方法都有默认实现（默认走 `f32` 中转；想保住双精度 / 整数语义就覆盖
/// [`Self::to_f64`] / [`Self::from_f64`] / [`Self::parse_text`] / [`Self::fmt_text`]）。
pub trait SliderValue: Copy {
    /// 转 `f32`（滑条内部单位）。
    fn to_f32(self) -> f32;
    /// 从 `f32` 收回。**整数类型在此四舍五入**——这是"整数滑条一格一格跳"的来源。
    fn from_f32(v: f32) -> Self;
    /// 转 `f64`（**数字输入框**的拖拽 / 吸附数学用它；默认经 `f32` 中转）。
    fn to_f64(self) -> f64 {
        f64::from(self.to_f32())
    }
    /// 从 `f64` 收回（默认经 `f32` 中转；整数在[`Self::from_f32`] 里四舍五入）。
    fn from_f64(v: f64) -> Self {
        Self::from_f32(v as f32)
    }
    /// 是**整数类型**吗（决定显示无小数、默认步进 = 1、输入只收整数文本）。默认 `false`。
    fn is_integral() -> bool {
        false
    }
    /// 默认拖拽步进（浮点 `0.01` = 每物理像素 ±0.01；整数 `1`）。
    ///
    /// ⚠ 它同时决定**显示小数位**（见 `NumberInput` 的 `step_decimals`）：`0.01` ⇒ 2 位。
    /// 需要更细的（颜色通道 / 归一化参数）显式 `.step(0.001)` 等覆盖。
    fn default_step() -> Self {
        Self::from_f64(0.01)
    }
    /// 解析用户输入（默认按 `f32` 解析后走 [`Self::from_f32`]；`f64` / 整数各自覆盖，
    /// 免得大值 / 高精度在手打时被 `f32` 截断）。
    fn parse_text(s: &str) -> Option<Self> {
        s.trim().parse::<f32>().ok().map(Self::from_f32)
    }
    /// 按 `decimals` 位小数显示（整数类型覆盖为无小数）。
    fn fmt_text(self, decimals: usize) -> String {
        format!("{:.*}", decimals, self.to_f64())
    }
}

impl SliderValue for f32 {
    #[inline]
    fn to_f32(self) -> f32 {
        self
    }
    #[inline]
    fn from_f32(v: f32) -> Self {
        v
    }
}

impl SliderValue for f64 {
    #[inline]
    fn to_f32(self) -> f32 {
        self as f32
    }
    #[inline]
    fn from_f32(v: f32) -> Self {
        v as f64
    }
    /// ⚠ **覆盖**默认实现：`f64` 直接进出，不走 `f32` 中转（否则数字输入框的手打 /
    /// 吸附会在 `f32` 上丢位）。
    #[inline]
    fn to_f64(self) -> f64 {
        self
    }
    #[inline]
    fn from_f64(v: f64) -> Self {
        v
    }
    #[inline]
    fn parse_text(s: &str) -> Option<Self> {
        s.trim().parse::<f64>().ok()
    }
}

/// 整数类型统一实现：`to_f32` 直接转，`from_f32` / `from_f64` **四舍五入**（步进 = 1），
/// 显示无小数、默认步进 1、解析走自身（大整数不经过 `f32`）。
macro_rules! slider_int {
    ($($t:ty),* $(,)?) => {$(
        impl SliderValue for $t {
            #[inline]
            fn to_f32(self) -> f32 {
                self as f32
            }
            #[inline]
            fn from_f32(v: f32) -> Self {
                // `round` 后再转：半整数朝远离 0 的方向（与 `f32::round` 一致）。
                // `as` 在浮点→整数是**饱和转换**（越界夹到类型边界，NaN → 0）。
                v.round() as $t
            }
            /// ⚠ **覆盖**默认实现：`f64` 直接四舍五入进类型，不走 `f32` 中转
            /// （`f32` 只有 24 位有效位，`1e15` 这种整数会被毁掉）。
            #[inline]
            fn from_f64(v: f64) -> Self {
                v.round() as $t
            }
            #[inline]
            fn to_f64(self) -> f64 {
                self as f64
            }
            #[inline]
            fn is_integral() -> bool {
                true
            }
            #[inline]
            fn default_step() -> Self {
                1
            }
            #[inline]
            fn parse_text(s: &str) -> Option<Self> {
                s.trim().parse::<$t>().ok()
            }
            #[inline]
            fn fmt_text(self, _decimals: usize) -> String {
                format!("{self}")
            }
        }
    )*};
}

slider_int!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

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
pub struct Slider<'a, T: SliderValue = f32> {
    id: &'a str,
    range: std::ops::RangeInclusive<T>,
    value: &'a mut T,
    /// 拖拽精度（每像素数值倍率；默认 1 = 值随鼠标 1:1）。
    drag_sensitivity: f32,
    /// 按住 Shift 拖拽速度倍率（默认 10）。
    shift_speed: f32,
    /// 按住 Ctrl 拖拽速度倍率（默认 0.1）。
    ctrl_speed: f32,
}

impl<'a, T: SliderValue> Slider<'a, T> {
    /// 构造：`range` 与 `value` 的类型决定 `T`（`f32` / `f64` / 各整数类型）。
    pub fn new(id: &'a str, range: std::ops::RangeInclusive<T>, value: &'a mut T) -> Self {
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
    /// 拖拽速度倍率：按住 **Shift** = ×`shift_speed`（默认 10）、按住 **Ctrl** =
    /// ×`ctrl_speed`（默认 0.1）、否则 1.0。
    fn resolve_speed(&self, ui: &Ui) -> f32 {
        let shift = ui.key_down(winit::keyboard::KeyCode::ShiftLeft)
            || ui.key_down(winit::keyboard::KeyCode::ShiftRight);
        let ctrl = ui.key_down(winit::keyboard::KeyCode::ControlLeft)
            || ui.key_down(winit::keyboard::KeyCode::ControlRight);
        if shift {
            self.shift_speed
        } else if ctrl {
            self.ctrl_speed
        } else {
            1.0
        }
    }
}

impl<T: SliderValue> Widget for Slider<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        // ① 申请（尺寸 = 主题，不需要测量）
        let rect = ui.allocate(Vec2::new(ui.theme().slider.min_w.max(40.0), ui.theme().slider.height));
        // 被裁剪层完全剔除 ⇒ 直接 return（不镶嵌、不入段：scissor 只省片元）。
        if ui.culled(rect) {
            return Response { rect, culled: true, ..Default::default() };
        }
        // 拖拽灵敏度 = 每像素数值倍率 × 修饰键速度（Shift 快 / Ctrl 慢）。
        let speed = self.resolve_speed(ui);
        // 进出各换算一次：类型只在 API 边界出现，交互 / 绘制全程 `f32`。
        let lo = self.range.start().to_f32();
        let hi = self.range.end().to_f32();
        let cur = self.value.to_f32();
        let new = ui.slider_at_drag(
            self.id,
            rect,
            lo..=hi,
            cur,
            self.drag_sensitivity * speed,
        );
        // 只有真的变了才回写：避免整数类型每帧被"round 成同一个值"反复写（无副作用，
        // 但省掉一次比较；更重要的是语义清楚——滑条只写它改过的值）。
        if new != cur {
            *self.value = T::from_f32(new);
        }
        Response { rect, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_types_round_trip_without_quantizing() {
        assert_eq!(<f32 as SliderValue>::from_f32(0.37).to_f32(), 0.37);
        assert_eq!(<f64 as SliderValue>::from_f32(0.37).to_f32(), 0.37);
    }

    #[test]
    fn integer_types_round_to_whole_steps() {
        // 整数滑条"一格一格跳"的来源：`from_f32` 四舍五入。
        assert_eq!(<u32 as SliderValue>::from_f32(3.4), 3);
        assert_eq!(<u32 as SliderValue>::from_f32(3.6), 4);
        assert_eq!(<i32 as SliderValue>::from_f32(-2.5), -3, "半整数朝远离 0");
        assert_eq!(<i32 as SliderValue>::from_f32(-2.4), -2);
        assert_eq!(<u8 as SliderValue>::from_f32(255.0), 255);
        assert_eq!(<i64 as SliderValue>::from_f32(-1.0e9), -1_000_000_000);
    }

    #[test]
    fn integer_types_convert_in() {
        assert_eq!(<u64 as SliderValue>::to_f32(1_000_000), 1.0e6);
        assert_eq!(<i8 as SliderValue>::to_f32(-100), -100.0);
    }

    /// **整数类型覆盖了 `f64` 直通**（默认实现会经 `f32` 中转，`1e15` 会被毁掉）：
    /// 这条守着"数字输入框接大整数不丢位"。
    #[test]
    fn integer_types_round_trip_large_values_through_f64() {
        assert_eq!(<i64 as SliderValue>::from_f64(1.0e15), 1_000_000_000_000_000);
        assert_eq!(<i64 as SliderValue>::to_f64(1_000_000_000_000_000), 1.0e15);
        // 若走 f32 中转：`1e15 as f32` = 999999986991104 ⇒ 差近 1300 万。
        assert_ne!(<i64 as SliderValue>::from_f32(1.0e15), 1_000_000_000_000_000);
        // 越界是**饱和**（不是 UB / 回绕）。
        assert_eq!(<u8 as SliderValue>::from_f64(300.0), 255);
        assert_eq!(<i8 as SliderValue>::from_f64(-300.0), -128);
    }

    /// 整数：显示无小数、默认步进 1、解析只收整数文本；`f64`：解析保精度。
    #[test]
    fn text_helpers_follow_the_type() {
        assert!(<i32 as SliderValue>::is_integral());
        assert!(!<f32 as SliderValue>::is_integral());
        assert!(!<f64 as SliderValue>::is_integral());
        assert_eq!(<u32 as SliderValue>::default_step(), 1);
        assert_eq!(<f32 as SliderValue>::default_step(), 0.01);
        assert_eq!(<i32 as SliderValue>::parse_text(" -42 "), Some(-42));
        assert_eq!(<i32 as SliderValue>::parse_text("3.5"), None, "整数不收小数");
        assert_eq!(<u32 as SliderValue>::parse_text("-1"), None, "无符号不收负号");
        assert_eq!(<i32 as SliderValue>::fmt_text(-7, 3), "-7", "整数不带小数");
        // `f64` 走自己的解析（默认实现按 `f32` 解析会把低位截掉）。
        let long = "0.30000000000000004";
        assert_eq!(<f64 as SliderValue>::parse_text(long), long.parse::<f64>().ok());
        assert_ne!(
            <f64 as SliderValue>::parse_text(long),
            long.parse::<f32>().ok().map(f64::from),
            "别退回 f32 解析"
        );
    }

    /// **自定义类型只需给两个方法**（其余走默认实现）——保证这次给 trait 加默认方法
    /// 不会破坏外部实现。
    #[test]
    fn custom_types_need_only_the_two_conversions() {
        #[derive(Clone, Copy, PartialEq, Debug)]
        struct Half(i32);
        impl SliderValue for Half {
            fn to_f32(self) -> f32 {
                self.0 as f32 * 0.5
            }
            fn from_f32(v: f32) -> Self {
                Half((v * 2.0).round() as i32)
            }
        }
        assert_eq!(Half(3).to_f64(), 1.5, "默认 f64 走 f32 中转");
        assert_eq!(Half::from_f64(2.0), Half(4));
        assert!(!Half::is_integral());
        assert_eq!(Half::parse_text("1.5"), Some(Half(3)), "默认按 f32 解析");
        assert_eq!(Half(3).fmt_text(2), "1.50");
    }
}
