//! DebugDraw：调试可视化图元（线段 / 矩形框 / 圆 / 十字 / 网格）。
//!
//! 用途：绘制碰撞体轮廓、视口范围、采样点、路径、速度矢量等调试信息——
//! 与 `rjw_ui`（Debug UI：`window_at` + `label` / `checkbox` / `slider` 组成即时模式
//! 调试面板，见 `examples/egDebugDraw`）搭配即构成完整的调试叠加层。
//!
//! # 用法（风格对象 + 画笔）
//!
//! ```ignore
//! // 一次 debug(style) 内的所有图元共享同一份样式（颜色 / 线宽 / 层级 / 分段数）
//! let mut p = r2d.debug(DebugStyle::new(Color::YELLOW).width(2.0).layer(10.0));
//! p.rect(bounds);
//! for o in &obstacles { p.rect(o.rect); }
//! r2d.debug(DebugStyle::new(Color::RED).width(2.0).layer(10.0))
//!     .line(ball_pos, ball_pos + ball_vel * 0.1);
//! ```
//!
//! - 坐标一律为**世界坐标**（经相机变换到屏幕）；屏幕固定的调试叠加可先用相机把
//!   屏幕点转世界，或直接走 `rjw_ui`。
//! - 实现：全部经 `Render2D` 动态 mesh 路径（[`Render2D::mesh_with`]）提交，
//!   相邻图元同状态自动合并为同一动态段（一次绘制），调试开销小。
//! - 默认管线无剔除（`RStates::default().cull == None`），三角形绕序无关紧要。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::{Layer, Render2D};

/// 线段 → 带厚度四边形（世界坐标）：`dir = normalize(b - a)`，法线 `n = (-dir.y, dir.x)`，
/// 四角 = `a ± n·w/2`、`b ± n·w/2`（按 [TL, TR, BL, BR] 顺序，与 `QUAD_TRI_INDICIES`
/// 的 [0,1,3, 3,2,0] 索引约定一致）。
///
/// 纯几何函数（可单测）：`width` 为世界单位；`a == b` 或 `width <= 0` 时返回 `None`
/// （退化线段，不产生图元）。下游（`rjw_ui` 的屏幕空间调试图元）也直接复用本函数。
#[inline]
pub fn thick_line_quad(a: impl Into<Vec2>, b: impl Into<Vec2>, width: f32) -> Option<[Vec2; 4]> {
    let (a, b) = (a.into(), b.into());
    let d = b - a;
    let len = d.length();
    if len <= f32::EPSILON || width <= 0.0 {
        return None;
    }
    let n = Vec2::new(-d.y / len, d.x / len) * (width * 0.5);
    Some([a - n, a + n, b - n, b + n])
}

/// 调试图元样式：颜色 / 线宽 / 层级 / 圆弧分段数。
///
/// 一份样式由 [`Render2D::debug`] 取用并生成一个 [`DebugPainter`]；画笔上的所有
/// 图元共享该样式（因此同一风格的连续图元只需写一次样式）。
///
/// - `width`：世界单位的线宽（[`DebugPainter::disc`] 不用它——实心圆无描边）；
/// - `layer`：绘制层级（数值小先绘制）；
/// - `segments`：圆 / 圆盘的分段数（≥ 3 生效）。
#[derive(Debug, Clone, Copy)]
pub struct DebugStyle {
    color: Color,
    width: f32,
    layer: Layer,
    segments: u32,
}

impl DebugStyle {
    /// 以颜色构造样式：`width = 1.0`、`layer = 0.0`、`segments = 24`。
    #[inline]
    pub fn new(color: Color) -> Self {
        Self {
            color,
            width: 1.0,
            layer: Layer::default(),
            segments: 24,
        }
    }

    /// 线宽（世界单位）。
    #[inline]
    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }

    /// 绘制层级（数值小先绘制）。
    #[inline]
    pub fn layer(mut self, layer: impl Into<Layer>) -> Self {
        self.layer = layer.into();
        self
    }

    /// 圆 / 圆盘的分段数（实际使用 `max(3)`）。
    #[inline]
    pub fn segments(mut self, n: u32) -> Self {
        self.segments = n;
        self
    }

    /// 颜色。
    #[inline]
    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
}

impl Default for DebugStyle {
    /// 白色 / `width = 1.0` / `layer = 0.0` / `segments = 24`。
    #[inline]
    fn default() -> Self {
        Self {
            color: Color::WHITE,
            width: 1.0,
            layer: Layer::default(),
            segments: 24,
        }
    }
}

impl From<Color> for DebugStyle {
    /// 便捷：`r2d.debug(Color::RED).line(a, b)`（其余取默认：width 1.0 / layer 0.0 / segments 24）。
    #[inline]
    fn from(color: Color) -> Self {
        Self { color, ..Self::default() }
    }
}

/// 调试画笔：由 [`Render2D::debug`] 借用渲染器 + 样式生成，逐个提交图元。
///
/// 画笔存活期间独占 `Render2D`（`&mut`），因此一个 [`Render2D::debug`] 调用对应
/// 一段连续录制；同一份样式的图元放在同一段里即可自动合批。
pub struct DebugPainter<'a> {
    r2d: &'a mut Render2D,
    style: DebugStyle,
}

impl<'a> DebugPainter<'a> {
    /// 构造画笔（由 [`Render2D::debug`] 调用）。
    #[inline]
    pub(crate) fn new(r2d: &'a mut Render2D, style: DebugStyle) -> Self {
        Self { r2d, style }
    }

    /// 一条线段（世界坐标；`a == b` 或 `width <= 0` 时无操作）。
    pub fn line(&mut self, a: impl Into<Vec2>, b: impl Into<Vec2>) {
        let Some([tl, tr, bl, br]) = thick_line_quad(a, b, self.style.width) else {
            return;
        };
        let (color, layer) = (self.style.color, self.style.layer);
        self.r2d
            .mesh_with(|sink| {
                let i0 = sink.push_vertex(tl);
                let i1 = sink.push_vertex(tr);
                let i2 = sink.push_vertex(bl);
                let i3 = sink.push_vertex(br);
                sink.push_tri(i0, i1, i2);
                sink.push_tri(i1, i3, i2);
            })
            .tint(color)
            .layer(layer);
    }

    /// 矩形边框（4 条线段合并为一次 mesh 提交；世界坐标）。
    pub fn rect(&mut self, r: Rect) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let width = self.style.width;
        let tl = Vec2::new(r.x, r.y);
        let tr = Vec2::new(r.x + r.w, r.y);
        let br = Vec2::new(r.x + r.w, r.y + r.h);
        let bl = Vec2::new(r.x, r.y + r.h);
        let quads = [
            thick_line_quad(tl, tr, width),
            thick_line_quad(tr, br, width),
            thick_line_quad(br, bl, width),
            thick_line_quad(bl, tl, width),
        ];
        let (color, layer) = (self.style.color, self.style.layer);
        self.r2d
            .mesh_with(|sink| {
                for q in quads.into_iter().flatten() {
                    let i0 = sink.push_vertex(q[0]);
                    let i1 = sink.push_vertex(q[1]);
                    let i2 = sink.push_vertex(q[2]);
                    let i3 = sink.push_vertex(q[3]);
                    sink.push_tri(i0, i1, i2);
                    sink.push_tri(i1, i3, i2);
                }
            })
            .tint(color)
            .layer(layer);
    }

    /// 圆环（`segments` 段折线近似，每段是带厚度四边形；世界坐标）。
    pub fn circle(&mut self, center: impl Into<Vec2>, radius: f32) {
        let center = center.into();
        let (width, segments) = (self.style.width, self.style.segments);
        if radius <= 0.0 || width <= 0.0 {
            return;
        }
        let seg = segments.max(3) as usize;
        let (color, layer) = (self.style.color, self.style.layer);
        self.r2d
            .mesh_with(|sink| {
                for i in 0..seg {
                    let a0 = i as f32 / seg as f32 * std::f32::consts::TAU;
                    let a1 = (i + 1) as f32 / seg as f32 * std::f32::consts::TAU;
                    let p0 = center + Vec2::new(a0.cos(), a0.sin()) * radius;
                    let p1 = center + Vec2::new(a1.cos(), a1.sin()) * radius;
                    if let Some([tl, tr, bl, br]) = thick_line_quad(p0, p1, width) {
                        let i0 = sink.push_vertex(tl);
                        let i1 = sink.push_vertex(tr);
                        let i2 = sink.push_vertex(bl);
                        let i3 = sink.push_vertex(br);
                        sink.push_tri(i0, i1, i2);
                        sink.push_tri(i1, i3, i2);
                    }
                }
            })
            .tint(color)
            .layer(layer);
    }

    /// 实心圆 / 圆盘（三角扇；世界坐标；不使用 `width`）。
    pub fn disc(&mut self, center: impl Into<Vec2>, radius: f32) {
        let center = center.into();
        let segments = self.style.segments;
        if radius <= 0.0 {
            return;
        }
        let seg = segments.max(3) as usize;
        let (color, layer) = (self.style.color, self.style.layer);
        self.r2d
            .mesh_with(|sink| {
                let c = sink.push_vertex(center);
                let mut prev: Option<u16> = None;
                for i in 0..=seg {
                    let a = i as f32 / seg as f32 * std::f32::consts::TAU;
                    let p = center + Vec2::new(a.cos(), a.sin()) * radius;
                    let v = sink.push_vertex(p);
                    match prev {
                        None => {}
                        Some(pv) => sink.push_tri(c, pv, v),
                    }
                    prev = Some(v);
                }
            })
            .tint(color)
            .layer(layer);
    }

    /// 十字标记（点 / 采样位置可视化；世界坐标；提交两条线段）。
    pub fn cross(&mut self, center: impl Into<Vec2>, half: f32) {
        let center = center.into();
        self.line(center - Vec2::new(half, 0.0), center + Vec2::new(half, 0.0));
        self.line(center - Vec2::new(0.0, half), center + Vec2::new(0.0, half));
    }

    /// 网格线（`rect` 范围内按 `spacing` 画竖线 + 横线；每方向最多 512 条，防病态输入）。
    pub fn grid(&mut self, rect: Rect, spacing: f32) {
        let width = self.style.width;
        if spacing <= 0.0 || width <= 0.0 {
            return;
        }
        let x0 = rect.x;
        let y0 = rect.y;
        let x1 = rect.x + rect.w;
        let y1 = rect.y + rect.h;
        let mut segs: Vec<[Vec2; 2]> = Vec::new();
        let mut x = x0;
        let mut n = 0;
        while x <= x1 && n < 512 {
            segs.push([Vec2::new(x, y0), Vec2::new(x, y1)]);
            x += spacing;
            n += 1;
        }
        let mut y = y0;
        n = 0;
        while y <= y1 && n < 512 {
            segs.push([Vec2::new(x0, y), Vec2::new(x1, y)]);
            y += spacing;
            n += 1;
        }
        if segs.is_empty() {
            return;
        }
        let (color, layer) = (self.style.color, self.style.layer);
        self.r2d
            .mesh_with(|sink| {
                for [a, b] in segs {
                    if let Some([tl, tr, bl, br]) = thick_line_quad(a, b, width) {
                        let i0 = sink.push_vertex(tl);
                        let i1 = sink.push_vertex(tr);
                        let i2 = sink.push_vertex(bl);
                        let i3 = sink.push_vertex(br);
                        sink.push_tri(i0, i1, i2);
                        sink.push_tri(i1, i3, i2);
                    }
                }
            })
            .tint(color)
            .layer(layer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thick_line_quad_horizontal_exact() {
        // 水平线段：法线 (0,1)，四角 = 端点 ± (0, 1)·w/2
        let q = thick_line_quad(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), 2.0).unwrap();
        assert_eq!(
            q,
            [
                Vec2::new(0.0, -1.0),
                Vec2::new(0.0, 1.0),
                Vec2::new(10.0, -1.0),
                Vec2::new(10.0, 1.0),
            ]
        );
    }

    #[test]
    fn thick_line_quad_slanted_geometry() {
        // 斜线段 (0,0)→(3,4)，宽度 2：
        // 宽度方向距离 = 2；长边方向与线段平行；四边形中心 = 线段中点。
        let q = thick_line_quad(Vec2::new(0.0, 0.0), Vec2::new(3.0, 4.0), 2.0).unwrap();
        let width_axis = q[1] - q[0];
        assert!(
            (width_axis.length() - 2.0).abs() < 1e-4,
            "宽度应等于 2，实际 {}",
            width_axis.length()
        );
        let seg = Vec2::new(3.0, 4.0).normalize();
        let long_edge = (q[2] - q[0]).normalize();
        assert!(long_edge.dot(seg).abs() > 0.999, "长边应与线段平行");
        let center = (q[0] + q[1] + q[2] + q[3]) / 4.0;
        assert!(
            (center - Vec2::new(1.5, 2.0)).length() < 1e-4,
            "四边形中心 = 线段中点，实际 {center:?}"
        );
        // 两个端点分别落在四边形两端（沿线段方向投影为 0 和段长 5）
        let proj0 = (q[0] - Vec2::new(0.0, 0.0)).dot(seg);
        let proj2 = (q[2] - Vec2::new(0.0, 0.0)).dot(seg);
        assert!(proj0.abs() < 1e-4 && (proj2 - 5.0).abs() < 1e-4, "端点投影 {proj0}/{proj2}");
    }

    #[test]
    fn thick_line_quad_degenerate() {
        assert!(thick_line_quad(Vec2::ZERO, Vec2::ZERO, 2.0).is_none(), "a==b 应退化");
        assert!(
            thick_line_quad(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), 0.0).is_none(),
            "width<=0 应退化"
        );
        assert!(
            thick_line_quad(Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), -1.0).is_none(),
            "负宽度应退化"
        );
    }

    #[test]
    fn style_defaults_and_builders() {
        let d = DebugStyle::default();
        assert_eq!(d.width, 1.0);
        assert_eq!(d.layer.as_f64(), 0.0);
        assert_eq!(d.segments, 24);
        assert_eq!(d.color, Color::WHITE);

        let s = DebugStyle::new(Color::RED)
            .width(2.0)
            .layer(5.0)
            .segments(48)
            .color(Color::GREEN);
        assert_eq!(s.width, 2.0);
        assert_eq!(s.layer.as_f64(), 5.0);
        assert_eq!(s.segments, 48);
        assert_eq!(s.color, Color::GREEN);
    }
}
