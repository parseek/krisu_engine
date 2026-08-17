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

    /// 由两个点构造（自动归一化为左上角 + 正宽高）。
    #[inline]
    pub fn from_points(a: Vec2, b: Vec2) -> Self {
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
    pub fn contains_point(&self, p: Vec2) -> bool {
        let r = self.normalized();
        p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h
    }

    /// 区间相交（容忍负宽高；边沿接触视为相交）。
    #[inline]
    pub fn intersects(&self, other: &Rect) -> bool {
        let a = self.normalized();
        let b = other.normalized();
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
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
        let t = Transform2D::IDENTITY.with_pos(Vec2::new(100.0, 0.0)).with_rot(0.785398); // 45°
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).transform(&t);
        // 旋转后包围盒应包含所有原角点的新位置
        for c in [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0), Vec2::new(10.0, 10.0)] {
            assert!(r.contains_point(t.transform_point(c)), "包围盒应包含变换后的角点 {c:?}");
        }
    }
}
