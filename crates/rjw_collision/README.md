# rjw_collision

中文：
`rjw_collision` 提供 2D 轴对齐碰撞：一个 `Aabb`（局部矩形 + 世界变换）与**扫掠滑动**（先 X 后 Y、逐轴回退、贴边 0 间隙）。

English：
`rjw_collision` provides 2D axis-aligned collision: a single `Aabb` (local rect + world transform) and **swept sliding** (X then Y, per-axis rollback, zero-gap resting).

> v0.3 起取代旧 API：`Collider`（单变体枚举）/ `collides(a, ta, b, tb)`（4 参）/ `move_and_collide(pos, size, vel, dt, obstacles)`（5 参）。

---

## API

```rust
pub struct Aabb { pub rect: Rect, pub transform: Transform2D }

impl Aabb {
    pub fn new(rect: Rect) -> Self;                        // transform = IDENTITY
    pub fn at(rect: Rect, transform: Transform2D) -> Self;
    pub fn world(&self) -> Rect;                           // 世界 AABB（rect.transform(&transform)）
    pub fn overlaps(&self, other: &Aabb) -> bool;          // 世界 AABB 相交
    /// 扫掠移动 + 沿障碍滑动；写回 transform.pos 并返回新位置（世界左上角）。
    pub fn slide(&mut self, delta: impl Into<Vec2>, obstacles: &[Rect]) -> Vec2;
}
```

`slide` 的「位置」语义 = **实体世界左上角**（`rect.min() + transform.pos`）：`Aabb::at(Rect::new(pos, size), IDENTITY)`
时与原 `move_and_collide(pos, size, ..)` 逐位一致。

## 示例 / Example

```rust
use rjw_collision::Aabb;
use rjw_transform::{Rect, Transform2D, Vec2};

let mut body = Aabb::at(
    Rect::new(player_pos.x, player_pos.y, size.x, size.y),
    Transform2D::IDENTITY,
);
player_pos = body.slide(vel * dt, &solid_rects);   // 撞墙停住、沿墙滑动

if body.overlaps(&enemy_body) { /* 命中 */ }
```

与 `rjw_tilemap` 配合：`map.solid_rects()` 给出世界空间 solid AABB 切片，直接作为 `obstacles`。

---

## 许可 / License

MIT © 2026 KrisuRJW
