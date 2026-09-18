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

/// **滑块可用的数值类型**（滑块内部一律用 `f32` 做拖拽数学）。
///
/// 已为 `f32` / `f64` / `i8..i64` / `isize` / `u8..u64` / `usize` 实现。自定义类型
/// （定点数 / 角度包装等）也可实现：给出到 `f32` 的映射与回写规则即可。
pub trait SliderValue: Copy {
    /// 转 `f32`（滑条内部单位）。
    fn to_f32(self) -> f32;
    /// 从 `f32` 收回。**整数类型在此四舍五入**——这是"整数滑条一格一格跳"的来源。
    fn from_f32(v: f32) -> Self;
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
}

/// 整数类型统一实现：`to_f32` 直接转，`from_f32` **四舍五入**（滑条步进 = 1）。
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
                v.round() as $t
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
        let rect = ui.allocate(Vec2::new(ui.theme.slider.min_w.max(40.0), ui.theme.slider.height));
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
}
