//! 分割线控件（占光标）：**水平**线宽 = 容器可用宽 / 当前最宽子项，行高 = 线厚 + 上下留白；
//! **竖直**线宽 = 线厚 + 左右留白，高 = **一行**（`Theme::row_h`）——横向排列（`row`）里当
//! 竖分割线用（菜单栏 / 工具栏）。
//!
//! 为什么竖线要显式一个方向、而不是让调用方自己 `allocate` + `painter().solid(..)`：
//! 竖线的**高度**必须跟"行"的强制行高一致（`row` 把子项高度覆盖成 `row_h`），
//! **宽度**又必须等于"线厚 + 两侧留白"才能让相邻控件间距正确；这两条一写错就是
//! "线贴在文字上 / 线短一截"，所以按控件封装 + 纯函数 `divider_line` 解算落笔矩形（可单测）。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;
use crate::draw::Size;
use crate::ui::Ui;
use super::{Response, Widget};

// ─── Divider ────────────────────────────────────────────────────

/// **分割线方向**（默认水平，保持既有行为）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DividerAxis {
    /// 水平线（默认）：占一行，宽 = 容器可用宽（`avail_w`）/ 当前最宽子项，兜底 120。
    #[default]
    Horizontal,
    /// 竖直线：宽 = 线厚 + 2×留白，高 = **一行**（`Theme::row_h`）——`row` 里当竖分割线。
    Vertical,
}

/// 分割线控件（占光标）：宽 = 容器可用宽（`avail_w`）/ 当前最宽子项，行高 =
/// 线厚 + 上下留白。属性可选，未设置回落 [`Theme::divider`](crate::style::Theme::divider)。
pub struct Divider {
    axis: DividerAxis,
    color: Option<Color>,
    thickness: Option<Size<f32>>,
    margin: Option<Size<f32>>,
}

impl Divider {
    pub fn new() -> Self {
        Self { axis: DividerAxis::Horizontal, color: None, thickness: None, margin: None }
    }
    /// **方向**（默认 [`DividerAxis::Horizontal`]）。
    pub fn axis(mut self, axis: DividerAxis) -> Self {
        self.axis = axis;
        self
    }
    /// **水平线**（默认行为的显式写法）。
    pub fn horizontal(self) -> Self {
        self.axis(DividerAxis::Horizontal)
    }
    /// **竖直线**（横向排列里当竖分割线）：宽 = 线厚 + 2×留白，高 = 一行。
    pub fn vertical(self) -> Self {
        self.axis(DividerAxis::Vertical)
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
    /// 线两侧留白（[`Size<f32>`]：逻辑（默认）或物理；默认 `Theme::divider.margin`）。
    ///
    /// 水平线：留白在**上下**（行高 = 线厚 + 2×留白）；竖直线：留白在**左右**
    /// （占位宽 = 线厚 + 2×留白）并同时上下收缩线长。
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

/// **分割线的落笔矩形**（纯函数，可单测）：从"占位矩形"里取出真正画线的那一条。
///
/// - 水平：线厚居中在占位矩形**垂直方向**（行高 = 线厚 + 2×留白 ⇒ 上下各留一截）；
/// - 竖直：线厚居中在**水平方向**，长度 = 占位高 − 2×留白（上下各留一截）。
fn divider_line(rect: Rect, axis: DividerAxis, thickness: f32, margin: f32) -> Rect {
    match axis {
        DividerAxis::Horizontal => {
            Rect::new(rect.x, rect.y + (rect.h - thickness) * 0.5, rect.w, thickness)
        }
        DividerAxis::Vertical => {
            let h = (rect.h - margin * 2.0).max(0.0);
            Rect::new(rect.x + (rect.w - thickness) * 0.5, rect.y + (rect.h - h) * 0.5, thickness, h)
        }
    }
}

impl Widget for Divider {
    fn ui(self, ui: &mut Ui) -> Response {
        let st = ui.theme.divider.clone();
        let t = self.thickness.map(|x| x.to_physical(ui.scale())).unwrap_or(st.thickness);
        let m = self.margin.map(|x| x.to_physical(ui.scale())).unwrap_or(st.margin);
        let c = self.color.unwrap_or(st.color);
        let rect = match self.axis {
            DividerAxis::Horizontal => {
                // 宽 = 容器可用宽（固定宽窗口 / 沙箱）；无可用宽 = 默认 120。
                let w = ui.avail_w().unwrap_or(120.0);
                // ⚠ 占光标、**撑大父级**（与旧 `size()` + 默认 `Expansion::UnlimitedExpansion`
                // 一致）：分隔线"宽 = 可用宽"本身就是父级宽度的一部分；改成
                // `DisableAutoExpansion` 会让固定宽窗口的尺寸整块变掉（实测：所有窗口尺寸 +50%）。
                ui.allocate(Vec2::new(w, t + m * 2.0))
            }
            DividerAxis::Vertical => {
                // 高 = **一行**：`row` 里子项本来就被强制成 `row_h`（同值 ⇒ 单独放在
                // 垂直堆叠里也自洽）；宽 = 线厚 + 左右留白 ⇒ 相邻控件的间距自动正确。
                let h = ui.theme().row_h;
                ui.allocate(Vec2::new(t + m * 2.0, h))
            }
        };
        // 被裁剪层完全剔除 ⇒ 直接 return（不镶嵌、不入段：scissor 只省片元）。
        if ui.culled(rect) {
            return Response { rect, culled: true, ..Default::default() };
        }
        ui.painter().solid(divider_line(rect, self.axis, t, m), c);
        Response { rect, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **水平线**：线厚在占位矩形里垂直居中，宽 = 占位宽（旧的唯一行为，别改坏）。
    #[test]
    fn divider_line_horizontal_centers_thickness() {
        let r = divider_line(Rect::new(10.0, 20.0, 200.0, 7.0), DividerAxis::Horizontal, 1.0, 3.0);
        assert_eq!(r, Rect::new(10.0, 23.0, 200.0, 1.0));
    }

    /// **竖直线**：线厚在水平方向居中、长度 = 占位高 − 2×留白（上下各留一截）。
    #[test]
    fn divider_line_vertical_centers_thickness_and_trims_margins() {
        let r = divider_line(Rect::new(10.0, 20.0, 7.0, 39.0), DividerAxis::Vertical, 1.0, 3.0);
        assert_eq!(r, Rect::new(13.0, 23.0, 1.0, 33.0));
        // 占位比留白还矮 ⇒ 长度夹到 0（不产生负高，否则光栅化拿到反向矩形）。
        let r = divider_line(Rect::new(0.0, 0.0, 7.0, 4.0), DividerAxis::Vertical, 1.0, 3.0);
        assert_eq!((r.y, r.h), (2.0, 0.0));
    }

    /// 省定义：`vertical()` / `horizontal()` 只改方向，不动样式字段。
    #[test]
    fn divider_axis_builders_only_change_axis() {
        let d = Divider::new().vertical().color(Color::WHITE).thickness(2.0).margin(4.0);
        assert_eq!(d.axis, DividerAxis::Vertical);
        assert_eq!(d.horizontal().axis, DividerAxis::Horizontal);
        assert_eq!(Divider::new().axis, DividerAxis::Horizontal, "默认仍是水平");
    }
}
