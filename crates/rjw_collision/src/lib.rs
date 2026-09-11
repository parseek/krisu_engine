//! 轻量 2D 碰撞原语（当前：AABB）。
//!
//! 建立在 `rjw_transform` 的 [`Rect`] 与 [`Transform2D`] 之上：
//! - [`Aabb`]：轴对齐碰撞体（**局部矩形 + 世界变换**），取代旧的 `Collider` 枚举；
//! - [`Aabb::overlaps`]：两碰撞体的世界 AABB 相交判定，取代旧 4 参 `collides`；
//! - [`Aabb::slide`]：带碰撞的移动解析（分离轴，滑动），取代旧 5 参 `move_and_collide`，
//!   供玩家/实体与静态世界（tilemap 等）交互。

use glam::Vec2;
use rjw_transform::{Rect, Transform2D};

/// 轴对齐碰撞体：局部矩形 + 世界变换。
///
/// - `rect`：**局部**矩形（左上角 + 宽高，约定见 [`Rect`]）；
/// - `transform`：把局部矩形放到世界空间的变换；[`Aabb::world`] 返回变换后的世界 AABB
///   （旋转/缩放按四角包围盒保守处理）。
///
/// 平移语义下实体的**世界左上角** = `rect.min() + transform.pos`。推荐把 `rect` 取成
/// `(0, 0)` 锚定的局部盒，此时世界左上角就是 `transform.pos`，[`Aabb::slide`] 的返回值、
/// `transform.pos` 与 `world().min()` 三者逐位相等。
#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub rect: Rect,
    pub transform: Transform2D,
}

// 手写而非 `#[derive(Default)]`：`Rect` 没有实现 `Default`（其空值语义是 [`Rect::ZERO`]），
// 而本次改动不允许修改 `rjw_transform`。对外表现与 derive 等价：
// `Aabb: Default`，且 `default() == Aabb { rect: Rect::ZERO, transform: Transform2D::IDENTITY }`。
impl Default for Aabb {
    #[inline]
    fn default() -> Self {
        Self { rect: Rect::ZERO, transform: Transform2D::IDENTITY }
    }
}

impl Aabb {
    /// 只给局部矩形构造，世界变换为单位变换（[`Transform2D::IDENTITY`]）。
    #[inline]
    pub fn new(rect: Rect) -> Self {
        Self { rect, transform: Transform2D::IDENTITY }
    }

    /// 局部矩形 + 世界变换构造。
    #[inline]
    pub fn at(rect: Rect, transform: Transform2D) -> Self {
        Self { rect, transform }
    }

    /// 世界空间 AABB（保守：变换后取四角包围盒）。
    #[inline]
    pub fn world(&self) -> Rect {
        self.rect.transform(&self.transform)
    }

    /// 两碰撞体是否相交（各自世界 AABB；底层 `Rect::intersects` 为半开区间，
    /// 边沿接触不算相交，接触判定用 `Rect::touches`）。
    #[inline]
    pub fn overlaps(&self, other: &Aabb) -> bool {
        self.world().intersects(&other.world())
    }

    /// 扫掠移动 + 沿障碍滑动（原 `move_and_collide` 的语义，**必须逐位保持**）：
    /// 先按 X 移动并对每个障碍做轴分离回退，再按 Y；返回最终位置。
    ///
    /// 采用**扫掠**判定（而非终点重叠）：大位移也不会穿过障碍物（防止隧道效应）。
    /// 语义：
    /// - `delta` 为本帧位移（即原 `vel * dt`）；
    /// - 实体世界左上角 = `self.rect.min() + self.transform.pos`，实体尺寸 = `rect.w` / `rect.h`；
    /// - `obstacles` 是世界 AABB 列表，**只读**，不被改动；
    /// - 就地更新 `self.transform.pos`，并返回移动后的世界左上角（不穿透任何障碍物）。
    #[inline]
    pub fn slide(&mut self, delta: impl Into<glam::Vec2>, obstacles: &[Rect]) -> Vec2 {
        let delta: Vec2 = delta.into();
        let size = Vec2::new(self.rect.w, self.rect.h);
        // `rect` 是局部矩形：其左上角相对 `transform.pos` 有一个固定偏移（平移语义）。
        let offset = self.rect.min();
        let mut out = self.transform.pos + offset;

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
                if (delta.y > 0.0 && out.y + size.y <= o.y && target_y + size.y > o.y)
                    || (delta.y < 0.0 && out.y >= o.y + o.h && target_y < o.y + o.h)
                {
                    // 撞上沿 → 贴 `o.y - size.y`；撞下沿 → 贴 `o.y + o.h`。
                    target_y = if delta.y > 0.0 { o.y - size.y } else { o.y + o.h };
                }
            }
            out.y = target_y;
        }

        // 写回：世界左上角 `out` ↔ `transform.pos`（`rect.min()` 是局部固定偏移）。
        // `rect.min() == (0, 0)` 时退化为 `self.transform.pos = out`，与原实现逐位一致。
        self.transform.pos = out - offset;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_at_world_and_default() {
        // `new`：变换为单位变换，世界 AABB == 局部矩形。
        let a = Aabb::new(Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(a.transform.pos, Vec2::ZERO);
        assert_eq!(a.transform.scale, Vec2::ONE);
        assert_eq!(a.transform.rotation, 0.0);
        assert_eq!(a.world(), Rect::new(0.0, 0.0, 10.0, 10.0));

        // `at`：世界 AABB = 局部矩形按变换摆放（纯平移）。
        let b = Aabb::at(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Transform2D::IDENTITY.with_pos(Vec2::new(8.0, 8.0)),
        );
        assert_eq!(b.world(), Rect::new(8.0, 8.0, 10.0, 10.0));

        // 局部矩形带偏移时，世界左上角 = rect.min() + transform.pos。
        let c = Aabb::at(
            Rect::new(3.0, 4.0, 10.0, 10.0),
            Transform2D::IDENTITY.with_pos(Vec2::new(8.0, 8.0)),
        );
        assert_eq!(c.world().min(), Vec2::new(11.0, 12.0));

        // `Default`：空矩形 + 单位变换（与 `new(Rect::ZERO)` 同义）。
        let d = Aabb::default();
        assert_eq!(d.rect, Rect::ZERO);
        assert_eq!(d.transform.pos, Vec2::ZERO);
        assert_eq!(d.transform.scale, Vec2::ONE);
        assert_eq!(d.transform.rotation, 0.0);
    }

    #[test]
    fn overlaps_detects_intersection_and_rejects_far_apart() {
        // 原 `aabb_collides_and_not` 的等价覆盖（相交 / 不相交 / 对称）。
        let a = Aabb::new(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = Aabb::at(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Transform2D::IDENTITY.with_pos(Vec2::new(8.0, 8.0)),
        );
        assert!(a.overlaps(&b));
        assert!(b.overlaps(&a));

        let c = Aabb::new(Rect::new(20.0, 20.0, 4.0, 4.0));
        assert!(!a.overlaps(&c));
        assert!(!c.overlaps(&a));
    }

    #[test]
    fn slide_stops_on_x_and_slides_on_y() {
        // 障碍物在右侧 (20..30, 0..10)，实体 5x5 从 (0,0) 出发。
        let wall = Rect::new(20.0, 0.0, 10.0, 10.0);

        // 纯右移 → 停在墙左侧（贴边 0 间隙）
        let mut body = Aabb::new(Rect::new(0.0, 0.0, 5.0, 5.0));
        let out = body.slide(Vec2::new(100.0, 0.0), &[wall]);
        assert_eq!(out.x, 15.0, "应停在墙左沿 (20 - 5)");
        assert_eq!(out.y, 0.0);
        assert_eq!(body.transform.pos, out, "slide 修改 transform.pos 并返回同一位置");
        assert_eq!(body.world().min(), out, "返回值即世界左上角");
        assert!(!body.world().intersects(&wall), "不得穿透");
        assert!(body.world().touches(&wall), "贴边允许 0 间隙");

        // 斜向移动 → X 先停，Y 继续（滑动）
        let mut body = Aabb::new(Rect::new(0.0, 0.0, 5.0, 5.0));
        let out = body.slide(Vec2::new(100.0, 100.0), &[wall]);
        assert_eq!(out.x, 15.0, "X 轴应被墙阻挡");
        assert_eq!(out.y, 100.0, "Y 轴不受影响，继续滑动");
        assert_eq!(body.transform.pos, out);
        assert_eq!(body.world().min(), out);
    }

    #[test]
    fn slide_stops_on_y_and_slides_on_x() {
        // Y 轴方向的等价覆盖：地板在下方（足够宽，X 修正后仍与实体在 X 上重叠）。
        let floor = Rect::new(0.0, 20.0, 1000.0, 10.0);

        // 纯下移 → 停在地板上沿
        let mut body = Aabb::new(Rect::new(0.0, 0.0, 5.0, 5.0));
        let out = body.slide(Vec2::new(0.0, 100.0), &[floor]);
        assert_eq!(out.x, 0.0);
        assert_eq!(out.y, 15.0, "应停在地板上沿 (20 - 5)");
        assert_eq!(body.transform.pos, out);
        assert!(!body.world().intersects(&floor), "不得穿透");
        assert!(body.world().touches(&floor), "贴边允许 0 间隙");

        // 斜向移动 → Y 被挡，X 继续（滑动）
        let mut body = Aabb::new(Rect::new(0.0, 0.0, 5.0, 5.0));
        let out = body.slide(Vec2::new(100.0, 100.0), &[floor]);
        assert_eq!(out.x, 100.0, "X 轴不受影响，继续滑动");
        assert_eq!(out.y, 15.0, "Y 轴应被地板阻挡");
        assert_eq!(body.transform.pos, out);
        assert_eq!(body.world().min(), out);
    }

    #[test]
    fn slide_handles_negative_directions_and_keeps_obstacles_untouched() {
        // 向左：贴左墙的右沿 (-30..-20)。
        let wall_l = Rect::new(-30.0, 0.0, 10.0, 10.0);
        let mut body = Aabb::new(Rect::new(0.0, 0.0, 5.0, 5.0));
        let out = body.slide(Vec2::new(-100.0, 0.0), &[wall_l]);
        assert_eq!(out.x, -20.0, "向左应停在墙右沿 (-30 + 10)");
        assert_eq!(out.y, 0.0);
        assert_eq!(body.transform.pos, out);

        // 向上：贴天花下沿 (-30..-20)。
        let ceil = Rect::new(0.0, -30.0, 10.0, 10.0);
        let mut body = Aabb::new(Rect::new(0.0, 0.0, 5.0, 5.0));
        let out = body.slide(Vec2::new(0.0, -100.0), &[ceil]);
        assert_eq!(out.x, 0.0);
        assert_eq!(out.y, -20.0, "向上应停在天花下沿 (-30 + 10)");
        assert_eq!(body.transform.pos, out);

        // 输入障碍数组不被改动（`&[Rect]` 只读）。
        let obstacles = [wall_l, ceil];
        assert_eq!(obstacles, [wall_l, ceil]);

        // **先 X 后 Y**：先撞左墙 → x = -20；Y 扫掠使用修正后的 X，
        // 此时实体已不再与天花在 X 上重叠 → Y 自由通过（顺序敏感，正是原实现语义）。
        let mut body = Aabb::new(Rect::new(0.0, 0.0, 5.0, 5.0));
        let out = body.slide(Vec2::new(-100.0, -100.0), &obstacles);
        assert_eq!(out, Vec2::new(-20.0, -100.0), "X 先停；修正后的 X 已离开天花范围");
        assert_eq!(body.transform.pos, out);

        // 大位移扫掠：一帧跨越整面墙也不会穿过去（防隧道）。
        let wall = Rect::new(20.0, 0.0, 10.0, 10.0);
        let mut body = Aabb::new(Rect::new(0.0, 0.0, 5.0, 5.0));
        let out = body.slide(Vec2::new(1.0e6, 0.0), &[wall]);
        assert_eq!(out.x, 15.0, "扫掠判定：大位移同样夹在墙左沿");
        assert!(!body.world().intersects(&wall));
    }
}
