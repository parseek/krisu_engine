//! 滑块 builder（链式：拖拽精度 / Shift·Ctrl 速度），交互与绘制委托 `Ui::slider_at_drag`。

use glam::Vec2;
use rjw_transform::Rect;
use crate::ui::Ui;
use super::{Response, Widget};

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

impl Widget for Slider<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        Vec2::new(ui.theme.slider.min_w.max(40.0), ui.theme.slider.height)
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        // 拖拽灵敏度 = 每像素数值倍率 × 修饰键速度（Shift 快 / Ctrl 慢）。
        let speed = self.resolve_speed(ui);
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
