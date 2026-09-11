use crate::Transform2D;
use glam::Vec2;

/// 轴对齐矩形（AABB），供剔除 / 碰撞 / 布局使用。
///
/// 约定：`x/y` 为左上角，`w/h` 为宽高（可为负，相交判定按区间处理）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    #[inline]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// 由两个点构造（自动归一化为左上角 + 正宽高；接受 `Vec2` 或 `(x, y)`）。
    #[inline]
    pub fn from_points(a: impl Into<Vec2>, b: impl Into<Vec2>) -> Self {
        let (a, b) = (a.into(), b.into());
        let min = a.min(b);
        let max = a.max(b);
        Self { x: min.x, y: min.y, w: max.x - min.x, h: max.y - min.y }
    }

    /// 由一组点构造包围盒（空输入返回 `ZERO`）。
    #[inline]
    pub fn from_point_slice(points: &[Vec2]) -> Self {
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        for p in points {
            min = min.min(*p);
            max = max.max(*p);
        }
        if max.x < min.x || max.y < min.y {
            return Self::ZERO;
        }
        Self { x: min.x, y: min.y, w: max.x - min.x, h: max.y - min.y }
    }

    /// 归一化：保证 `w/h >= 0`（交换负方向边界）。
    #[inline]
    pub fn normalized(self) -> Self {
        let (x, w) = if self.w < 0.0 { (self.x + self.w, -self.w) } else { (self.x, self.w) };
        let (y, h) = if self.h < 0.0 { (self.y + self.h, -self.h) } else { (self.y, self.h) };
        Self { x, y, w, h }
    }

    #[inline]
    pub fn min(&self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }

    #[inline]
    pub fn max(&self) -> Vec2 {
        Vec2::new(self.x + self.w, self.y + self.h)
    }

    #[inline]
    pub fn center(&self) -> Vec2 {
        Vec2::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    #[inline]
    pub fn contains_point(&self, p: impl Into<Vec2>) -> bool {
        let p = p.into();
        let r = self.normalized();
        p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h
    }

    /// 面积正相交（容忍负宽高；**半开区间**：边沿接触不算相交，接触判定用 [`Self::touches`]）。
    #[inline]
    pub fn intersects(&self, other: &Rect) -> bool {
        let a = self.normalized();
        let b = other.normalized();
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    /// 是否接触或相交（闭区间；边沿贴边算接触）。
    #[inline]
    pub fn touches(&self, other: &Rect) -> bool {
        let a = self.normalized();
        let b = other.normalized();
        a.x <= b.x + b.w && b.x <= a.x + a.w && a.y <= b.y + b.h && b.y <= a.y + a.h
    }

    /// 完全包含（容忍负宽高；`other` 归一化后判定）。
    #[inline]
    pub fn contains(&self, other: &Rect) -> bool {
        let a = self.normalized();
        let b = other.normalized();
        a.x <= b.x && a.y <= b.y && a.x + a.w >= b.x + b.w && a.y + a.h >= b.y + b.h
    }

    /// 并集包围盒。
    #[inline]
    pub fn union(&self, other: &Rect) -> Rect {
        let a = self.normalized();
        let b = other.normalized();
        let min = a.min().min(b.min());
        let max = a.max().max(b.max());
        Rect::from_points(min, max)
    }

    /// 保守 AABB 变换：把矩形四角经 `t` 变换后取包围盒（旋转/缩放均保守，不误杀）。
    #[inline]
    pub fn transform(&self, t: &Transform2D) -> Rect {
        let a = self.min();
        let b = self.max();
        let pts = [
            t.transform_point(a),
            t.transform_point(Vec2::new(b.x, a.y)),
            t.transform_point(Vec2::new(a.x, b.y)),
            t.transform_point(b),
        ];
        Rect::from_point_slice(&pts)
    }

    /// **内缩**（减法语义）：四边各向内收 `by`（`x/y += by`、`w/h −= 2by`），
    /// 宽高 clamp 到 ≥ 0（内缩超过半宽/半高时退化为空矩形，不产生负尺寸）。
    /// 用于"外框内缩"式绘制（如勾选框中心填充 = 外框 shrink 边框宽 + 内边距）。
    #[inline]
    pub fn shrink(&self, by: f32) -> Rect {
        let by = by.max(0.0);
        Rect::new(
            self.x + by,
            self.y + by,
            (self.w - by * 2.0).max(0.0),
            (self.h - by * 2.0).max(0.0),
        )
    }

    // ── 画面 / 视口辅助（Rect 是被唯一承认的「屏幕矩形」类型，见 API_DESIGN §5） ──

    /// 左上角（= `min` 的别名，语义上更贴近「屏幕位置」）。
    #[inline]
    pub fn pos(&self) -> Vec2 {
        self.min()
    }

    /// 尺寸 `(w, h)`。
    #[inline]
    pub fn size(&self) -> Vec2 {
        Vec2::new(self.w, self.h)
    }

    /// 宽高任一为 0 视为空。
    #[inline]
    pub fn is_empty(&self) -> bool {
        let r = self.normalized();
        r.w <= 0.0 || r.h <= 0.0
    }

    /// 在自身内部**再挖一个矩形**（`inner` 相对自身左上角的像素偏移）。
    ///
    /// 用于分屏 / 画中画：`window_rect.inset(Rect::new(x, y, w, h))`。
    #[inline]
    pub fn inset(&self, inner: Rect) -> Rect {
        Rect::new(self.x + inner.x, self.y + inner.y, inner.w, inner.h)
    }

    /// 左半幅（宽度取半，`x` 不变）。
    #[inline]
    pub fn left_half(&self) -> Rect {
        Rect::new(self.x, self.y, self.w * 0.5, self.h)
    }

    /// 右半幅（宽度取半，`x` 右移半宽）。
    #[inline]
    pub fn right_half(&self) -> Rect {
        Rect::new(self.x + self.w * 0.5, self.y, self.w * 0.5, self.h)
    }

    /// 上半幅（高度取半，`y` 不变）。
    #[inline]
    pub fn top_half(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h * 0.5)
    }

    /// 下半幅（高度取半，`y` 下移半高）。
    #[inline]
    pub fn bottom_half(&self) -> Rect {
        Rect::new(self.x, self.y + self.h * 0.5, self.w, self.h * 0.5)
    }
}

impl From<(f32, f32, f32, f32)> for Rect {
    /// `(x, y, w, h)`。
    #[inline]
    fn from(value: (f32, f32, f32, f32)) -> Self {
        Self::new(value.0, value.1, value.2, value.3)
    }
}

impl From<[f32; 4]> for Rect {
    /// `[x, y, w, h]`。
    #[inline]
    fn from(value: [f32; 4]) -> Self {
        Self::new(value[0], value[1], value[2], value[3])
    }
}

impl From<Rect> for [f32; 4] {
    #[inline]
    fn from(r: Rect) -> Self {
        [r.x, r.y, r.w, r.h]
    }
}

impl From<Rect> for (f32, f32, f32, f32) {
    #[inline]
    fn from(r: Rect) -> Self {
        (r.x, r.y, r.w, r.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_fixes_negative_dimensions() {
        let r = Rect::new(10.0, 20.0, -5.0, -3.0).normalized();
        assert_eq!(r, Rect::new(5.0, 17.0, 5.0, 3.0));
    }

    #[test]
    fn intersects_half_open_edges() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        // 边沿接触（半开区间）→ 不算相交
        let b = Rect::new(10.0, 0.0, 5.0, 5.0);
        assert!(!a.intersects(&b), "右沿接触不应相交（半开区间）");
        // 真正重叠 → 相交
        let c = Rect::new(9.0, 0.0, 5.0, 5.0);
        assert!(a.intersects(&c), "部分重叠应相交");
        let d = Rect::new(10.1, 0.0, 5.0, 5.0);
        assert!(!a.intersects(&d), "完全分离不应相交");
    }

    #[test]
    fn from_points_normalizes() {
        let r = Rect::from_points(Vec2::new(5.0, 8.0), Vec2::new(1.0, 2.0));
        assert_eq!(r, Rect::new(1.0, 2.0, 4.0, 6.0));
    }

    #[test]
    fn transform_is_conservative_for_rotation() {
        let t = Transform2D::IDENTITY.with_pos(Vec2::new(100.0, 0.0)).with_rot(std::f32::consts::FRAC_PI_4); // 45°
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).transform(&t);
        // 旋转后包围盒应包含所有原角点的新位置
        for c in [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0), Vec2::new(10.0, 10.0)] {
            assert!(r.contains_point(t.transform_point(c)), "包围盒应包含变换后的角点 {c:?}");
        }
    }

    #[test]
    fn shrink_uses_subtraction_and_clamps_nonnegative() {
        // 减法内缩：x/y 增大、w/h 减小
        let r = Rect::new(10.0, 20.0, 100.0, 50.0).shrink(4.0);
        assert_eq!(r, Rect::new(14.0, 24.0, 92.0, 42.0));
        // 内缩超过半宽 → 宽 clamp 到 0（不产生负尺寸）
        let r2 = Rect::new(0.0, 0.0, 10.0, 10.0).shrink(6.0);
        assert_eq!(r2, Rect::new(6.0, 6.0, 0.0, 0.0));
        // 负数 by → 按 0 处理（无变化）
        assert_eq!(Rect::new(0.0, 0.0, 5.0, 5.0).shrink(-3.0), Rect::new(0.0, 0.0, 5.0, 5.0));
        // 非方矩形
        assert_eq!(Rect::new(1.0, 2.0, 20.0, 8.0).shrink(1.5), Rect::new(2.5, 3.5, 17.0, 5.0));
    }

    #[test]
    fn touches_includes_edges_intersects_excludes_them() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let edge = Rect::new(10.0, 0.0, 5.0, 5.0);
        assert!(!a.intersects(&edge), "半开区间：贴边不算相交");
        assert!(a.touches(&edge), "闭区间：贴边算接触");
    }

    #[test]
    fn tuple_and_array_conversions_roundtrip() {
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(Rect::from((1.0, 2.0, 3.0, 4.0)), r);
        assert_eq!(Rect::from([1.0, 2.0, 3.0, 4.0]), r);
        let t: (f32, f32, f32, f32) = r.into();
        assert_eq!(t, (1.0, 2.0, 3.0, 4.0));
        let a: [f32; 4] = r.into();
        assert_eq!(a, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn splits_and_inset_cover_window_layout() {
        let window = Rect::new(0.0, 0.0, 1280.0, 720.0);
        assert_eq!(window.left_half(), Rect::new(0.0, 0.0, 640.0, 720.0));
        assert_eq!(window.right_half(), Rect::new(640.0, 0.0, 640.0, 720.0));
        assert_eq!(window.top_half(), Rect::new(0.0, 0.0, 1280.0, 360.0));
        assert_eq!(window.bottom_half(), Rect::new(0.0, 360.0, 1280.0, 360.0));
        // 画中画：相对窗口的 (900, 12, 280, 240)
        assert_eq!(
            window.inset(Rect::new(900.0, 12.0, 280.0, 240.0)),
            Rect::new(900.0, 12.0, 280.0, 240.0)
        );
        assert_eq!(window.pos(), Vec2::ZERO);
        assert_eq!(window.size(), Vec2::new(1280.0, 720.0));
        assert!(!window.is_empty());
        assert!(Rect::new(0.0, 0.0, 0.0, 10.0).is_empty());
    }
}
