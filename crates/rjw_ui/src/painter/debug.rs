//! 屏幕空间调试图元（进 `debug_queue`：不进窗口缓存、恒在 UI 内容之后提交）。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use super::Painter;
use crate::draw::{DebugShape, DrawKind, UiDraw};

impl Painter {
    /// 录制一条屏幕空间调试图元（坐标 = **绝对逻辑屏幕像素**）。
    pub fn debug(&mut self, shape: DebugShape, color: Color) {
        let seq = self.q.next_seq();
        let (depth, win) = (self.q.depth, self.q.cur_win);
        self.q.debug_queue.push(UiDraw {
            depth,
            seq,
            win,
            elem: 0,
            // 调试形状自带几何（`DebugShape`），`rect` 字段未用。
            rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            clip: None,
            full_w: false,
            kind: DrawKind::Debug { color, shape },
        });
    }

    /// 屏幕空间线段。
    pub fn debug_line(
        &mut self,
        a: impl Into<Vec2>,
        b: impl Into<Vec2>,
        width: f32,
        color: Color,
    ) {
        self.debug(DebugShape::Line { a: a.into(), b: b.into(), width }, color);
    }

    /// 屏幕空间矩形边框。
    pub fn debug_rect_outline(&mut self, rect: Rect, width: f32, color: Color) {
        self.debug(DebugShape::RectOutline { rect, width }, color);
    }

    /// 屏幕空间圆环（`segments` 段折线近似）。
    pub fn debug_circle_outline(
        &mut self,
        center: impl Into<Vec2>,
        radius: f32,
        segments: usize,
        width: f32,
        color: Color,
    ) {
        self.debug(
            DebugShape::CircleOutline { center: center.into(), radius, segments, width },
            color,
        );
    }

    /// 屏幕空间十字标记。
    pub fn debug_cross(
        &mut self,
        center: impl Into<Vec2>,
        half: f32,
        width: f32,
        color: Color,
    ) {
        self.debug(DebugShape::Cross { center: center.into(), half, width }, color);
    }

    /// 屏幕空间网格（`rect` 内按 `spacing` 画竖线 + 横线）。
    pub fn debug_grid(&mut self, rect: Rect, spacing: f32, width: f32, color: Color) {
        self.debug(DebugShape::Grid { rect, spacing, width }, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 调试图元**不进内容队列**（否则会被窗口顶点缓存 / 内容排序吞掉），
    /// 且 `clip` 恒 `None`（诊断图元不该被内容裁剪）。
    #[test]
    fn debug_goes_to_the_debug_queue_only() {
        let mut p = Painter::new(1.0);
        p.clipped(Some(Rect::new(0.0, 0.0, 5.0, 5.0)), |p| {
            p.debug_line(Vec2::ZERO, Vec2::new(1.0, 1.0), 1.0, Color::WHITE);
        });
        assert!(p.commands().is_empty(), "内容队列里不应有调试命令");
        assert_eq!(p.debug_commands().len(), 1);
        let d = &p.debug_commands()[0];
        assert_eq!(d.elem, 0);
        assert_eq!(d.clip, None);
        assert!(matches!(d.kind, DrawKind::Debug { .. }));
    }
}
