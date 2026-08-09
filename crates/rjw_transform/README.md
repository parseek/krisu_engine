# rjw_transform

中文：
`rjw_transform` 提供 2D 变换与正交相机：`Transform2D`（位置 / 缩放 / 旋转，构建器链 + 父子组合 / 逆组合）与 `Camera2D`（正交投影、VP 矩阵、屏幕 / 世界坐标互转）。

English：
`rjw_transform` provides 2D transforms and an orthographic camera: `Transform2D` (position / scale / rotation with a builder chain, compose / inverse) and `Camera2D` (orthographic projection, VP matrix, screen-world conversion).

---

## 功能特性 / Features

中文：
- `Transform2D`：`with_pos` / `with_scale` / `with_rot` 构建器；`with_transform` / `inverse` 父子组合；`transform_point` / `transform_vec`。
- `Camera2D`：正交投影（原点居中、X+ 右、Y+ 下），`vp_matrix()` / `view_matrix()` / `projection_matrix()`。
- 坐标转换：`world_to_screen()` / `screen_to_world()`；`walk_xy` / `walk_xplus` / `walk_yplus` 按相机旋转方向移动。
- 重导出 `glam` 及常用类型（`Vec2` / `Vec3` / `Vec4` / `Mat4`）。

English：
- `Transform2D`: builder methods `with_pos` / `with_scale` / `with_rot`; parent-child `with_transform` / `inverse`; `transform_point` / `transform_vec`.
- `Camera2D`: orthographic projection (origin-centered, X+ right, Y+ down), `vp_matrix()` / `view_matrix()` / `projection_matrix()`.
- Coordinate conversion: `world_to_screen()` / `screen_to_world()`; `walk_xy` / `walk_xplus` / `walk_yplus` move along the camera rotation.
- Re-exports `glam` and common types (`Vec2` / `Vec3` / `Vec4` / `Mat4`).

---

## 示例代码 / Example

```rust
use rjw_transform::{Camera2D, Transform2D};
use rjw_transform::Vec2;

let tf = Transform2D::IDENTITY
    .with_pos(Vec2::new(100.0, 0.0))
    .with_rot(0.5)
    .with_scale(Vec2::splat(2.0));

let mut cam = Camera2D::new(Vec2::new(1280.0, 720.0));
cam.set_vp(Vec2::new(1280.0, 720.0), Vec2::ZERO);
let vp = cam.vp_matrix();
let world = cam.screen_to_world(Vec2::new(640.0, 360.0));
```

---

## 许可 / License

MIT © 2026 KrisuRJW
