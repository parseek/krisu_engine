//! 绘制原语：屏幕固定变换 + 实心矩形 / 边框（记录式，实际提交在 `Ui::finish`）。
//!
//! 屏幕固定变换数学（与 `rjw_transform::Camera2D` 单元测试
//! `screen_fixed_transform_maps_local_to_screen_1to1` 一致）：
//! `{ pos: cam.screen_to_world(anchor_px), rotation: +cam.rotation, scale: 1/zoom }`
//! —— 局部像素点经该变换到世界、再 `world_to_screen`，恒等于 `anchor_px + local`（1:1，
//! 不随相机旋转/缩放而变形）。注意旋转符号为 **`+cam.rotation`**（用 `-` 会双重旋转）。

use glam::Vec2;
use rjw_2d_render::SpriteRect;
use rjw_color::Color;
use rjw_transform::{Camera2D, Rect, Transform2D};

/// 屏幕固定变换：把屏幕像素锚点映射为世界中的 `Transform2D`。
#[inline]
pub fn screen_fixed_tf(cam: &Camera2D, anchor_px: Vec2) -> Transform2D {
    Transform2D::IDENTITY
        .with_pos(cam.screen_to_world(anchor_px))
        .with_rot(cam.rotation)
        .with_scale(Vec2::new(1.0 / cam.zoom.x, 1.0 / cam.zoom.y))
}

/// 屏幕矩形 → 精灵矩形（mesh 局部坐标从 (0,0) 起，尺寸 = 矩形宽高）。
#[inline]
pub fn rect_sprite(rect: &Rect) -> SpriteRect {
    SpriteRect::from_texture(Vec2::ZERO, Vec2::new(rect.w, rect.h))
}

/// **屏幕像素取整**（pixel snapping）：左上角与右下角分别四舍五入到整数像素，
/// 保证取整后矩形不缩水（面积 ≥ 原矩形）。在提交绘制前对**物理像素**矩形调用，
/// 避免高 DPI / 非整数布局下出现半像素采样导致的边缘模糊与闪烁。
#[inline]
pub fn snap_rect(r: &Rect) -> Rect {
    let x0 = r.x.round();
    let y0 = r.y.round();
    let x1 = (r.x + r.w).round();
    let y1 = (r.y + r.h).round();
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// 屏幕像素点取整（锚点 / 文本位置用）。
#[inline]
pub fn snap_point(p: Vec2) -> Vec2 {
    Vec2::new(p.x.round(), p.y.round())
}

/// **文本块内容对齐偏移**（屏幕像素，UI"浮点整数"不变量的一部分）。
///
/// 水平：左 `0`、中 `-round(content_w/2)`、右 `-content_w`；
/// 垂直：`-first_line_top - round(content_h/2)`（行盒垂直居中）。
///
/// 前置条件（调用方保证）：`content`（排版内容**物理**尺寸，`measure_buffer` 已取整）
/// 与 `first_line_top`（行盒顶相对视觉原点，`rjw_text` 收集期已取整）均为**整数像素**。
/// 因此本函数内部全部加/减法操作数都是整数 —— 与整数锚点相加
/// （`block_tl = anchor + off`）时两侧均为整数，结果恒为整数：
/// 小数（如 0.5px 的居中奇数宽 / 行盒偏移）只在 `round` 边界被消化，
/// 不会流入加法链 → 无误差累加、无亚像素摆放。
#[inline]
pub fn text_block_offset(align: TextAlign, content: Vec2, first_line_top: f32) -> Vec2 {
    let off_x = match align {
        TextAlign::Left => 0.0,
        TextAlign::Center => -(content.x * 0.5).round(),
        TextAlign::Right => -content.x,
    };
    let off_y = -first_line_top - (content.y * 0.5).round();
    Vec2::new(off_x, off_y)
}

/// 两矩形求交（裁剪用；无交集返回 `None`）。纯函数（可单测）。
#[inline]
pub fn intersect_rect(a: &Rect, b: &Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    if x1 > x0 && y1 > y0 {
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    } else {
        None
    }
}

/// 物理矩形应用裁剪（`clip_abs` = 绝对物理；`None` 返回原矩形，`Some` 求交，空 = 全裁）。
#[inline]
pub(crate) fn clipped(pr: Rect, clip_abs: Option<Rect>) -> Option<Rect> {
    match clip_abs {
        Some(c) => intersect_rect(&pr, &c),
        None => (pr.w > 0.0 && pr.h > 0.0).then_some(pr),
    }
}

/// 边框四边（画在矩形内边缘；宽度 <= 0 或 宽度 >= 半尺寸时退化）。
pub fn border_rects(rect: &Rect, width: f32) -> [Rect; 4] {
    let w = width.max(0.0);
    let hw = rect.w * 0.5;
    let hh = rect.h * 0.5;
    let w = w.min(hw).min(hh);
    [
        Rect::new(rect.x, rect.y, rect.w, w),                    // 上
        Rect::new(rect.x, rect.y + rect.h - w, rect.w, w),       // 下
        Rect::new(rect.x, rect.y, w, rect.h),                    // 左
        Rect::new(rect.x + rect.w - w, rect.y, w, rect.h),       // 右
    ]
}

/// 调试形状（逻辑像素）→ 物理像素线段列表：每条 = `([起点, 终点], 线宽)`。
///
/// 纯几何（可单测）：`scale` 为 DPI 物理/逻辑换算；线段随后由 `QuadCollector`
/// 经 `thick_line_quad` 生成带厚度四边形。Grid 每方向最多 512 条（防病态输入）。
pub(crate) fn debug_shape_segments(shape: &DebugShape, scale: f32) -> Vec<([Vec2; 2], f32)> {
    use std::f32::consts::TAU;
    let mut segs: Vec<([Vec2; 2], f32)> = Vec::new();
    match shape {
        DebugShape::Line { a, b, width } => {
            segs.push(([*a * scale, *b * scale], width * scale));
        }
        DebugShape::RectOutline { rect, width } => {
            let tl = Vec2::new(rect.x, rect.y);
            let tr = Vec2::new(rect.x + rect.w, rect.y);
            let br = Vec2::new(rect.x + rect.w, rect.y + rect.h);
            let bl = Vec2::new(rect.x, rect.y + rect.h);
            for (a, b) in [(tl, tr), (tr, br), (br, bl), (bl, tl)] {
                segs.push(([a * scale, b * scale], width * scale));
            }
        }
        DebugShape::CircleOutline {
            center,
            radius,
            segments,
            width,
        } => {
            let seg = (*segments).max(3);
            let c = *center * scale;
            let r = *radius * scale;
            for i in 0..seg {
                let a0 = i as f32 / seg as f32 * TAU;
                let a1 = (i + 1) as f32 / seg as f32 * TAU;
                let p0 = c + Vec2::new(a0.cos(), a0.sin()) * r;
                let p1 = c + Vec2::new(a1.cos(), a1.sin()) * r;
                segs.push(([p0, p1], width * scale));
            }
        }
        DebugShape::Cross { center, half, width } => {
            let c = *center * scale;
            let h = *half * scale;
            segs.push(([c - Vec2::new(h, 0.0), c + Vec2::new(h, 0.0)], width * scale));
            segs.push(([c - Vec2::new(0.0, h), c + Vec2::new(0.0, h)], width * scale));
        }
        DebugShape::Grid { rect, spacing, width } => {
            let w = width * scale;
            let sp = (*spacing).max(f32::EPSILON) * scale;
            let x0 = rect.x * scale;
            let y0 = rect.y * scale;
            let x1 = (rect.x + rect.w) * scale;
            let y1 = (rect.y + rect.h) * scale;
            let mut x = x0;
            let mut n = 0;
            while x <= x1 && n < 512 {
                segs.push(([Vec2::new(x, y0), Vec2::new(x, y1)], w));
                x += sp;
                n += 1;
            }
            let mut y = y0;
            n = 0;
            while y <= y1 && n < 512 {
                segs.push(([Vec2::new(x0, y), Vec2::new(x1, y)], w));
                y += sp;
                n += 1;
            }
        }
    }
    segs
}

/// 屏幕空间调试图元（`rjw_ui` 的 DebugDraw；坐标 = **逻辑屏幕像素**，左上角原点、
/// Y+ 向下，与 UI 控件坐标一致）。经 [`Ui::debug_line`] 等录制，`finish` 时以
/// **白色纹理四边形**在 UI 内容**之后**提交（覆盖在一切 UI 之上）。
///
/// 世界坐标（游戏场景内）的调试图元见 [`rjw_2d_render::debug_draw`]。
#[derive(Clone, Copy, Debug)]
pub enum DebugShape {
    /// 线段（`a` → `b`；`width` 为逻辑像素）。
    Line { a: Vec2, b: Vec2, width: f32 },
    /// 矩形边框。
    RectOutline { rect: Rect, width: f32 },
    /// 圆环（`segments` 段折线近似）。
    CircleOutline { center: Vec2, radius: f32, segments: usize, width: f32 },
    /// 十字标记（点 / 采样位置）。
    Cross { center: Vec2, half: f32, width: f32 },
    /// 网格线（`rect` 范围内按 `spacing` 竖线 + 横线；每方向最多 512 条）。
    Grid { rect: Rect, spacing: f32, width: f32 },
}

/// 渐变方向（`Gradient` 绘制命令）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientAxis {
    /// 沿 y（0 = 顶部 → 底部）。
    Vertical,
    /// 沿 x（0 = 左侧 → 右侧）。
    Horizontal,
}

/// 绘制命令种类（记录式；`Ui::finish` 逐条提交到 `Render2D`）。
#[derive(Clone, Debug)]
pub enum DrawKind {
    /// 实心矩形。
    Solid(Color),
    /// **圆角矩形**（背景填充；`radius` 逻辑像素，9-patch 绘制，颜色顶点色 tint）。
    RoundedRect { color: Color, radius: f32 },
    /// **线性渐变矩形**（`stops` 沿 `axis`；程序化纹理进动态 Atlas）。
    Gradient { axis: GradientAxis, stops: Vec<(f32, Color)> },
    /// 矩形边框（画在 rect 内缘）。
    Border { color: Color, width: f32 },
    /// 文本（绘制时经 `rjw_text` 责任链渲染）。
    Text {
        text: String,
        size: f32,
        color: Color,
        align: TextAlign,
        family: Option<String>,
        /// 文本局部裁剪（相对内容起点；`None` = 不裁剪）。
        clip: Option<Rect>,
    },
    /// 文本输入框光标（竖条）。
    Caret { color: Color, width: f32 },
    /// 屏幕空间调试图元（DebugDraw；坐标 = 逻辑屏幕像素，覆盖在 UI 内容之上）。
    Debug { color: Color, shape: DebugShape },
}

/// 文本水平对齐（垂直恒居中）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

impl DrawKind {
    /// 类别分组（同一 layer 内"**背景/图形 → 文字**"排序用）：
    /// - `0`：背景 / 图形（Solid / Border / Caret）——先画；
    /// - `1`：文字（Text）——后画（覆盖在图形之上）。
    /// - `2`：调试图元（Debug）——内容排序时不会出现（走独立调试队列，恒最后提交）。
    ///
    /// 窗口内元素**不处理互相重叠**：仅保证类别顺序，同类间保持录制顺序。
    #[inline]
    pub fn group(&self) -> u8 {
        match self {
            DrawKind::Text { .. } => 1,
            DrawKind::Debug { .. } => 2,
            _ => 0,
        }
    }
}

/// 一条绘制命令（坐标 = 相对当前容器 origin 的局部坐标，容器弹出时统一平移）。
#[derive(Clone, Debug)]
pub struct UiDraw {
    pub depth: u32,
    pub seq: u32,
    /// 所属窗口的 z 序（[`crate::Ui::window_at`]；非窗口内容 = 0）。
    /// 窗口间按 z 升序绘制（焦点窗口 z 最大 → 最后画 → 最上层）。
    pub win: u32,
    /// **元素序**：所属控件（元素）开始录制时的序号。
    ///
    /// 排序键 `(win, depth, elem, group, seq)`：**元素间按录制顺序**（后录元素
    /// 覆盖先录元素，重叠层级正确），**元素内**再按"背景/图形 → 文字"（`group`）。
    /// 容器背景 / 边框等"容器装饰"用 `elem = 0`（恒画在本容器元素之下）。
    pub elem: u32,
    pub rect: Rect,
    /// **裁剪区**（**绝对逻辑屏幕坐标**；滚动容器等设置，`None` = 不裁剪）。
    /// 与 `rect` 一样随容器弹出平移（`translate`）。收集期与内容求交，越界部分剔除。
    pub clip: Option<Rect>,
    pub kind: DrawKind,
}

impl UiDraw {
    #[inline]
    pub fn translate(&mut self, by: Vec2) {
        self.rect.x += by.x;
        self.rect.y += by.y;
        if let Some(c) = &mut self.clip {
            c.x += by.x;
            c.y += by.y;
        }
    }
}

/// 一条文本命令的便捷构造。
#[allow(clippy::too_many_arguments)]
pub fn text_cmd(
    depth: u32,
    seq: u32,
    win: u32,
    elem: u32,
    rect: Rect,
    text: String,
    size: f32,
    color: Color,
    align: TextAlign,
    family: Option<String>,
    clip: Option<Rect>,
    clip_outer: Option<Rect>,
) -> UiDraw {
    UiDraw {
        depth,
        seq,
        win,
        elem,
        rect,
        clip: clip_outer,
        kind: DrawKind::Text {
            text,
            size,
            color,
            align,
            family,
            clip,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_rect_math() {
        // 相交：取交集
        let i = intersect_rect(&Rect::new(0.0, 0.0, 100.0, 50.0), &Rect::new(50.0, 25.0, 100.0, 50.0)).unwrap();
        assert_eq!(i, Rect::new(50.0, 25.0, 50.0, 25.0));
        // 完全包含：取较小
        let i = intersect_rect(&Rect::new(0.0, 0.0, 100.0, 50.0), &Rect::new(10.0, 10.0, 20.0, 20.0)).unwrap();
        assert_eq!(i, Rect::new(10.0, 10.0, 20.0, 20.0));
        // 不相交：None（滚动裁剪 → 全裁）
        assert!(intersect_rect(&Rect::new(0.0, 0.0, 10.0, 10.0), &Rect::new(20.0, 20.0, 10.0, 10.0)).is_none());
        // 仅边接触（半开区间）：None
        assert!(intersect_rect(&Rect::new(0.0, 0.0, 10.0, 10.0), &Rect::new(10.0, 0.0, 10.0, 10.0)).is_none());
        // clipped helper 语义：None 裁剪返回原矩形
        assert_eq!(clipped(Rect::new(0.0, 0.0, 5.0, 5.0), None), Some(Rect::new(0.0, 0.0, 5.0, 5.0)));
        assert_eq!(clipped(Rect::new(0.0, 0.0, 5.0, 5.0), Some(Rect::new(2.0, 2.0, 10.0, 10.0))), Some(Rect::new(2.0, 2.0, 3.0, 3.0)));
        assert_eq!(clipped(Rect::new(0.0, 0.0, 5.0, 5.0), Some(Rect::new(9.0, 9.0, 10.0, 10.0))), None);
    }

    #[test]
    fn snap_rect_rounds_to_integer_pixels() {
        let r = Rect::new(10.4, 20.6, 30.2, 15.7);
        let s = snap_rect(&r);
        assert_eq!(s.x, 10.0);
        assert_eq!(s.y, 21.0);
        // 左上角 + 右下角分别四舍五入：x1 = round(40.6) = 41, y1 = round(36.3) = 36
        assert_eq!(s.w, 31.0);
        assert_eq!(s.h, 15.0);
        // 全部整数像素
        for v in [s.x, s.y, s.w, s.h] {
            assert_eq!(v.fract(), 0.0, "取整后应为整数像素，实际 {v}");
        }
        // 尺寸偏差 ≤ 1px（round 允许 ≤0.5px 偏移换取像素对齐）
        assert!((s.w - r.w).abs() <= 1.0 && (s.h - r.h).abs() <= 1.0);
    }

    #[test]
    fn snap_rect_clamps_negative_shrink() {
        // 极端：宽高取整可能为负（如 0.4 → round 0.4=0, round(0.4+0.2)=1 → w=1）
        // 构造 w 不足 0.5 且四舍五入抵消的情形：clamp 到 ≥ 0
        let r = Rect::new(0.2, 0.2, 0.1, 0.1);
        let s = snap_rect(&r);
        assert!(s.w >= 0.0 && s.h >= 0.0, "宽高不得为负，实际 {s:?}");
        let r2 = Rect::new(0.5, 0.5, 0.4, 0.4);
        let s2 = snap_rect(&r2);
        assert!(s2.w >= 0.0 && s2.h >= 0.0, "宽高不得为负，实际 {s2:?}");
    }

    #[test]
    fn snap_point_rounds() {
        assert_eq!(snap_point(Vec2::new(1.2, -3.6)), Vec2::new(1.0, -4.0));
    }

    #[test]
    fn debug_shape_segments_scale_and_count() {
        // 线段：1 条，端点与线宽按 scale 缩放
        let segs = debug_shape_segments(
            &DebugShape::Line { a: Vec2::new(10.0, 20.0), b: Vec2::new(30.0, 40.0), width: 2.0 },
            1.5,
        );
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].0, [Vec2::new(15.0, 30.0), Vec2::new(45.0, 60.0)]);
        assert_eq!(segs[0].1, 3.0);
        // 矩形框：4 条边，角点落在缩放后的矩形角上
        let segs = debug_shape_segments(
            &DebugShape::RectOutline { rect: Rect::new(0.0, 0.0, 100.0, 50.0), width: 1.0 },
            2.0,
        );
        assert_eq!(segs.len(), 4);
        let corners: Vec<Vec2> = segs.iter().flat_map(|([a, b], _)| [*a, *b]).collect();
        for c in [Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0), Vec2::new(200.0, 100.0), Vec2::new(0.0, 100.0)] {
            assert!(corners.contains(&c), "角点 {c:?} 应出现在矩形框线段中");
        }
        // 圆环：segments 条线段，半径缩放
        let segs = debug_shape_segments(
            &DebugShape::CircleOutline { center: Vec2::ZERO, radius: 10.0, segments: 32, width: 1.0 },
            1.0,
        );
        assert_eq!(segs.len(), 32);
        for ([a, b], w) in &segs {
            assert!((a.length() - 10.0).abs() < 0.2 && (b.length() - 10.0).abs() < 0.2, "环上点半径≈10");
            assert_eq!(*w, 1.0);
        }
        // 十字：2 条线段（横 + 竖）
        let segs = debug_shape_segments(
            &DebugShape::Cross { center: Vec2::new(5.0, 5.0), half: 8.0, width: 2.0 },
            1.0,
        );
        assert_eq!(segs.len(), 2);
        // 网格：40×20 每 10px → 竖线 0,10,20,30,40 = 5 条；横线 0,10,20 = 3 条
        let segs = debug_shape_segments(
            &DebugShape::Grid { rect: Rect::new(0.0, 0.0, 40.0, 20.0), spacing: 10.0, width: 1.0 },
            1.0,
        );
        assert_eq!(segs.len(), 5 + 3, "40px 宽每 10px 一条（含两端）→ 5 条；20px 高 → 3 条");
        // segments 下限：segments=0 → 按 3 处理
        let segs = debug_shape_segments(
            &DebugShape::CircleOutline { center: Vec2::ZERO, radius: 5.0, segments: 0, width: 1.0 },
            1.0,
        );
        assert_eq!(segs.len(), 3);
    }

    #[test]
    fn debug_kind_group_is_last() {
        // 调试图元分组 2（恒排在图形 0 / 文字 1 之后）
        assert_eq!(
            DrawKind::Debug {
                color: Color::WHITE,
                shape: DebugShape::Line { a: Vec2::ZERO, b: Vec2::ONE, width: 1.0 },
            }
            .group(),
            2
        );
        assert_eq!(DrawKind::Solid(Color::WHITE).group(), 0);
    }

    #[test]
    fn text_block_offset_uses_integer_operands_only() {
        // 不变量：UI 文本定位的所有加/减法操作数必须为整数（防止误差累加 / 亚像素摆放）。
        // content（物理尺寸）与 first_line_top（行盒顶）均为整数输入。
        // 居中 + 奇数内容宽 21：off_x = -round(10.5) = -11（整数）；
        // 垂直：-(-7) - round(17/2) = 7 - 9 = -2（整数）。
        let off = text_block_offset(TextAlign::Center, Vec2::new(21.0, 17.0), -7.0);
        assert_eq!(off, Vec2::new(-11.0, -2.0), "居中奇数宽 + 行盒偏移应为整数");
        assert_eq!(off.x.fract(), 0.0, "off.x 必须为整数像素，实际 {}", off.x);
        assert_eq!(off.y.fract(), 0.0, "off.y 必须为整数像素，实际 {}", off.y);
        // 左对齐：水平偏移恒为 0
        let off_l = text_block_offset(TextAlign::Left, Vec2::new(21.0, 17.0), -7.0);
        assert_eq!(off_l.x, 0.0);
        assert_eq!(off_l.y.fract(), 0.0);
        // 右对齐：-content_w
        let off_r = text_block_offset(TextAlign::Right, Vec2::new(30.0, 17.0), -7.0);
        assert_eq!(off_r.x, -30.0);
        // 偶数宽/高：取整无偏差（round(20/2)=10、round(16/2)=8）
        let off_e = text_block_offset(TextAlign::Center, Vec2::new(20.0, 16.0), -6.0);
        assert_eq!(off_e, Vec2::new(-10.0, -2.0));
        // 空文本（content = 0）：偏移为 0
        assert_eq!(
            text_block_offset(TextAlign::Center, Vec2::ZERO, 0.0),
            Vec2::ZERO
        );
        // 锚点 + 偏移 = 整数 + 整数：任意组合结果恒为整数
        for anchor in [Vec2::new(100.0, 200.0), Vec2::new(0.5, -3.5).round()] {
            let block = anchor + text_block_offset(TextAlign::Center, Vec2::new(21.0, 17.0), -7.0);
            assert_eq!(block.x.fract(), 0.0, "block.x 必须为整数，实际 {}", block.x);
            assert_eq!(block.y.fract(), 0.0, "block.y 必须为整数，实际 {}", block.y);
        }
    }
}
