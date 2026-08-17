//! 轻量 2D 碰撞原语（当前：AABB）。
//!
//! 建立在 `rjw_transform` 的 [`Rect`] 与 [`Transform2D`] 之上：
//! - [`Collider::Aabb`]：轴对齐碰撞盒；
//! - [`collides`]：两碰撞器相交判定；
//! - [`move_and_collide`]：带碰撞的移动解析（分离轴，滑动），供玩家/实体与静态世界（tilemap 等）交互。

use glam::Vec2;
use rjw_transform::{Rect, Transform2D};

/// 碰撞器形状（当前仅 AABB；预留 Circle 等扩展位）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Collider {
    Aabb(Rect),
}

impl Collider {
    /// 世界空间 AABB（保守：变换后取包围盒）。
    #[inline]
    pub fn world_aabb(&self, t: &Transform2D) -> Rect {
        match self {
            Collider::Aabb(r) => r.transform(t),
        }
    }
}

/// 两碰撞器是否相交（世界空间判定；调用方自行提供各自变换）。
#[inline]
pub fn collides(a: &Collider, ta: &Transform2D, b: &Collider, tb: &Transform2D) -> bool {
    a.world_aabb(ta).intersects(&b.world_aabb(tb))
}

/// 带碰撞的移动解析：把 `pos` 按 `vel * dt` 移动，与 `obstacles`（世界 AABB 列表）碰撞时
/// **按轴分离滑动**（先 X 后 Y），返回最终位置。
///
/// 采用**扫掠**判定（而非终点重叠）：大位移也不会穿过障碍物（防止隧道效应）。
/// 语义：
/// - `pos` 是实体 AABB（局部 rect，已含实体尺寸）的**当前左上角世界坐标**；
/// - `vel` 为速度（像素/秒），`dt` 为帧时长；
/// - 返回值是移动后不穿透任何障碍物的左上角坐标。
#[inline]
pub fn move_and_collide(
    pos: Vec2,
    size: Vec2,
    vel: Vec2,
    dt: f32,
    obstacles: &[Rect],
) -> Vec2 {
    let delta = vel * dt;
    let mut out = pos;

    // X 轴扫掠
    if delta.x != 0.0 {
        let mut target_x = out.x + delta.x;
        for o in obstacles {
            // 仅当 Y 区间与障碍物重叠时才可能被 X 向阻挡
            if !(out.y < o.y + o.h && o.y < out.y + size.y) {
                continue;
            }
            if delta.x > 0.0 {
                // 向右：起点在障碍左沿左侧、且目标越过左沿 → 夹在左沿
                if out.x + size.x <= o.x && target_x + size.x > o.x {
                    target_x = o.x - size.x;
                }
            } else if delta.x < 0.0 {
                // 向左：起点在障碍右沿右侧、且目标越过右沿 → 夹在右沿
                if out.x >= o.x + o.w && target_x < o.x + o.w {
                    target_x = o.x + o.w;
                }
            }
        }
        out.x = target_x;
    }

    // Y 轴扫掠（使用修正后的 X）
    if delta.y != 0.0 {
        let mut target_y = out.y + delta.y;
        for o in obstacles {
            if !(out.x < o.x + o.w && o.x < out.x + size.x) {
                continue;
            }
            if delta.y > 0.0 {
                if out.y + size.y <= o.y && target_y + size.y > o.y {
                    target_y = o.y - size.y;
                }
            } else if delta.y < 0.0 {
                if out.y >= o.y + o.h && target_y < o.y + o.h {
                    target_y = o.y + o.h;
                }
            }
        }
        out.y = target_y;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_collides_and_not() {
        let a = Collider::Aabb(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = Collider::Aabb(Rect::new(8.0, 8.0, 10.0, 10.0));
        assert!(collides(&a, &Transform2D::IDENTITY, &b, &Transform2D::IDENTITY));
        let c = Collider::Aabb(Rect::new(20.0, 20.0, 4.0, 4.0));
        assert!(!collides(&a, &Transform2D::IDENTITY, &c, &Transform2D::IDENTITY));
    }

    #[test]
    fn move_and_collide_slides_along_axis() {
        // 障碍物在右侧 (20..30, 0..10)，实体从 (0,0) 大小 5x5 向右下移动
        let wall = Rect::new(20.0, 0.0, 10.0, 10.0);
        // 纯右移 → 停在墙左侧
        let out = move_and_collide(Vec2::ZERO, Vec2::splat(5.0), Vec2::new(100.0, 0.0), 1.0, &[wall]);
        assert_eq!(out.x, 15.0, "应停在墙左沿 (20 - 5)");
        assert_eq!(out.y, 0.0);
        // 斜向移动 → X 先停，Y 继续（滑动）
        let out = move_and_collide(Vec2::ZERO, Vec2::splat(5.0), Vec2::new(100.0, 100.0), 1.0, &[wall]);
        assert_eq!(out.x, 15.0, "X 轴应被墙阻挡");
        assert_eq!(out.y, 100.0, "Y 轴不受影响，继续滑动");
    }
}
