# API 参考手册（免读源码版）

> 为**不想读源代码的人和 AI** 准备的速查 API 参考。
> 覆盖：`Color` / `ColorF64`、`Transform2D`、`Camera2D`、`Render2D`，以及相关小类型。
> 完整的**概念**讲解（坐标系、Layer、KeyState 边沿、物理/逻辑像素、页池…）见 [ENGINE_GUIDE.md](ENGINE_GUIDE.md)。
>
> 约定：所有坐标单位为**世界像素**；`Y+` 指向屏幕**下方**；`layer` 数值小先画。
>
> 还有：本项目使用的 `wgpu` 版本为 `30.0.0`，与常用的 `0.20.0` 的 API 有诸多不同，建议使用 `rjw_render::wgpu` 的重导出

---

## 目录

- [1. Color / ColorF64（颜色）](#1-color--colorf64颜色)
- [2. Transform2D（变换）](#2-transform2d变换)
- [3. Camera2D（相机）](#3-camera2d相机)
- [4. SpriteRect（精灵矩形）](#4-spriterect精灵矩形)
- [5. Render2D（2D 批渲染器）](#5-render2d2d-批渲染器)
- [6. ClearConfig（清屏配置）](#6-clearconfig清屏配置)
- [7. 其他常用小类型速查](#7-其他常用小类型速查)

---

## 1. Color / ColorF64（颜色）

crate：`rjw_color`

### `Color`（f32 存储）

| 函数 | 签名 / 用法 | 说明 |
|---|---|---|
| `Color::rgba` | `Color::rgba(r: f32, g: f32, b: f32, a: f32)` | 0..=1 浮点颜色 |
| `Color::rgb` | `Color::rgb(r, g, b)` | alpha=1.0 |
| `Color::rgba_u8` | `Color::rgba_u8(r: u8, g: u8, b: u8, a: u8)` | 0..=255 |
| `Color::rgb_u8` | `Color::rgb_u8(r, g, b)` | alpha=255 |
| 常量 | `Color::RED` `Color::GREEN` `Color::BLUE` `Color::WHITE` `Color::BLACK` 等 | 见 `consts` 模块 |

```rust
use rjw_color::Color;

// 半透明红色
let red = Color::rgba(1.0, 0.0, 0.0, 0.5);
// 0~255 用法
let green = Color::rgba_u8(60, 200, 80, 255);
// 写进绘制命令需转 [f32; 4]（渲染器内部自动处理，你直接传 Color 即可）
let arr: [f32; 4] = Color::WHITE.into();
```

### `ColorF64`（f64 存储，用于 wgpu 清屏）

| 函数 | 用法 | 说明 |
|---|---|---|
| `ColorF64::rgba` | `ColorF64::rgba(f64, f64, f64, f64)` | 高精度 |
| `.into()` | `let c: wgpu::Color = ColorF64::rgba(...).into();` | 直接转换给 `ClearConfig.color` |

```rust
// 清屏颜色：
let clear = wgpu::Color {
    r: 0.13, g: 0.24, b: 0.12, a: 1.0,
};
// 或
use rjw_color::ColorF64;
let clear = ColorF64::rgba(0.13, 0.24, 0.12, 1.0).into();
```

> ⚠️ `Color` ↔ `wgpu::Color` **不直接互转**。绘制用 `Color`，清屏用 `ColorF64().into()` 或手写 `wgpu::Color`。

---

## 2. Transform2D（变换）

crate：`rjw_transform`

```rust
pub struct Transform2D {
    pub pos:      glam::Vec2,  // 平移（世界）
    pub scale:    glam::Vec2,  // 缩放（x/y 可独立）
    pub rotation: f32,         // 旋转（弧度，绕 pos 中心）
}
```

#### 构建器（返回新值，链式）

| 函数 | 用法 | 说明 |
|---|---|---|
| `IDENTITY` | `Transform2D::IDENTITY` | 单位变换 |
| `with_pos` | `.with_pos(Vec2::new(x, y))` | 设置位置 |
| `with_scale` | `.with_scale(Vec2::new(sx, sy))` | 设置缩放 |
| `with_rot` | `.with_rot(0.5)` | 设置旋转（弧度） |
| `with_move_by` | `.with_move_by(delta)` | 平移 delta |
| `with_walk_by` | `.with_walk_by(local)` | **按当前旋转方向**位移（先旋转再平移） |
| `with_scale_by` | `.with_scale_by(factor)` | 缩放乘 |
| `with_rotate_by` | `.with_rotate_by(rad)` | 旋转加 |

#### 空间运算

| 函数 | 说明 |
|---|---|
| `transform_point(local)` | 局部点 → 父/世界点 |
| `inverse_transform_point(world)` | 世界点 → 局部点（命中检测用） |
| `transform_vec(local_vec)` | 局部方向 → 世界方向（只旋转+缩放，不平移） |
| `inverse_transform_vec(world_vec)` | 反过来 |
| `with_transform(&parent)` | 组合父级：`result = parent * self` |
| `transform_components(pos, scale, rot)` | 与 raw 组件组合 |
| `with_inverse_transform(&parent)` / `inverse_transform_components` | 放入**父级逆空间**（UI 面板局部坐标） |
| `inverse()` | 逆变换对象 |

```rust
use rjw_transform::Transform2D;
use glam::Vec2;

// 一个中心在 (100, 50)、放大 2 倍、旋转 90° 的对象
let tf = Transform2D::IDENTITY
    .with_pos(Vec2::new(100.0, 50.0))
    .with_scale(Vec2::splat(2.0))
    .with_rot(std::f32::consts::FRAC_PI_2);

// 局部点 → 世界
let world = tf.transform_point(Vec2::new(0.0, 0.0));

// 世界鼠标点 → 面板局部坐标（命中检测）
let panel = Transform2D::IDENTITY.with_pos(Vec2::new(200.0, 100.0));
let local = panel.inverse_transform_point(mouse_world);
```

> 💡 **旋转中心 = pos**：让精灵绕自身中心转，矩形要写成
> `SpriteRect::from_texture(Vec2::splat(-w/2), Vec2::splat(w))`，再把 `pos` 放在中心。

---

## 3. Camera2D（相机）

crate：`rjw_transform`

```rust
pub struct Camera2D {
    pub position:     Vec2,  // 相机中心（世界）—— 即窗口中心对应的世界点
    pub rotation:     f32,   // 旋转（弧度）
    pub zoom:         Vec2,  // 缩放
    pub viewport_pos: Vec2,  // 视口左上角（窗口像素）
    pub viewport_size: Vec2, // 视口尺寸（像素）
}
```

#### 构造 / 视口

| 函数 | 用法 | 说明 |
|---|---|---|
| `Camera2D::new` | `Camera2D::new(Vec2::new(w, h))` | 以窗口尺寸建相机；**之后必须 `set_vp`** |
| `set_vp` | `cam.set_vp(Vec2::new(w, h), Vec2::ZERO)` | 设置视口大小 + 位置（高 DPI 用 `render.size()` 的物理像素） |

#### 移动（重点：`walk_xy`）

| 函数 | 用法 | 说明 |
|---|---|---|
| `move_by` | `cam.move_by(Vec2::new(dx, dy))` | 绝对平移（不随旋转） |
| `walk_xy` | `cam.walk_xy(Vec2::new(lx, ly))` | **沿相机自身方向**移动：把 `(lx, ly)` 按 `rotation` 旋转后加到 position |
| `walk_xplus` | `cam.walk_xplus(v)` | 沿相机横向 + 方向走 v |
| `walk_yplus` | `cam.walk_yplus(v)` | 沿相机纵向 + 方向走 v |

> ⚠️ `walk_xy` 的 `(lx, ly)` 是**相机局部坐标**（lx 横向、ly 纵向即深度/上下），会被 `rotation` 旋转——不是直接的世界位移。相机没旋转时 `walk_xy` ≈ `move_by`。

```rust
use rjw_transform::Camera2D;
use glam::Vec2;

let mut cam = Camera2D::new(Vec2::new(1280.0, 720.0));
cam.set_vp(Vec2::new(1280.0, 720.0), Vec2::ZERO);

// 每帧键盘移动相机（沿相机局部方向）
let speed = 400.0;
cam.walk_xy(Vec2::new(
    (right as i32 - left as i32) as f32 * speed,
    (down as i32 - up as i32) as f32 * speed,
));

// 滚轮缩放
cam.zoom *= Vec2::splat(1.1_f32.powf(wheel_y as f32));
```

#### 矩阵 / 坐标转换

| 函数 | 说明 |
|---|---|
| `vp_matrix()` | 列主序 VP（P×V），**直接**喂 `render2d.set_mvp(...)` 无需转置 |
| `view_matrix()` / `projection_matrix()` | 拆开拿视图/投影 |
| `screen_to_world(screen_px)` | 窗口像素 → 世界坐标（内部处理 Y 翻转） |
| `world_to_screen(world)` | 世界 → 窗口像素 |
| `world_transform()` | 把相机看作 `Transform2D`（UI 反父级计算用） |

```rust
// 鼠标指向的世界点
let mouse_px = ctx.mouse.get_mouse_position(); // (f64, f64)
let world = cam.screen_to_world(Vec2::new(mouse_px.0 as f32, mouse_px.1 as f32));
```

---

## 4. SpriteRect（精灵矩形）

crate：`rjw_2d_render`（`data` 模块）

```rust
pub struct SpriteRect {
    pub mesh_tl: Vec2, // 世界坐标左上角
    pub mesh_wh: Vec2, // 世界尺寸
    pub uv_tl:   Vec2, // 归一化 UV 左上 (0..1)
    pub uv_wh:   Vec2, // 归一化 UV 尺寸 (0..1)
}
```

| 函数 | 用法 | 说明 |
|---|---|---|
| `from_texture` | `SpriteRect::from_texture(tl, wh)` | 整张贴图 |
| `from_texture_px` | `SpriteRect::from_texture_px(tl, wh, uv_tl_px, uv_wh_px, inv_tex_wh)` | 按**像素**取纹理子区（sprite sheet）；`inv_tex_wh = Vec2::new(1/w, 1/h)` |
| `new` | `SpriteRect::new(tl, wh, uv_tl, uv_wh)` | 全手动（UV 归一化） |

```rust
use rjw_2d_render::SpriteRect;
use glam::Vec2;

// 整张贴在 (0,0)，尺寸 32×32
let a = SpriteRect::from_texture(Vec2::ZERO, Vec2::splat(32.0));

// 从 128×128 图集中取 (0,0) 起的 32×32 子图
let b = SpriteRect::from_texture_px(
    Vec2::ZERO, Vec2::splat(32.0),
    Vec2::ZERO, Vec2::splat(32.0),
    Vec2::new(1.0 / 128.0, 1.0 / 128.0),
);
```

---

## 5. Render2D（2D 批渲染器）

crate：`rjw_2d_render`

> 生命周期注意：`Render2D::new(&RenderContext)` 持有 surface 的 `'static` 引用，
> 要求 `RenderContext` 比 `Render2D` 活得更久（框架中天然满足）。

#### 创建 / 全局

| 函数 | 用法 | 说明 |
|---|---|---|
| `new` | `Render2D::new(&render_ctx)` | 基于 `RenderContext` 创建 |
| `set_mvp` | `r2d.set_mvp(cam.vp_matrix())` | 设置 VP（每帧渲染前调用） |
| `create_texture` | `r2d.create_texture("label", &rgba8, w, h)` | 建纹理（RGBA8，`len==w*h*4` 否则 panic），返回 `ArcTextureWrapped` |

#### Sprite（实例化，推荐）

| 函数 | 用法 | 说明 |
|---|---|---|
| `add_sprite2d_default` | `r2d.add_sprite2d_default(rect, color, transform, layer, &tex)` | 贴纹理精灵 |
| `add_sprite2d_default_solid` | `r2d.add_sprite2d_default_solid(rect, color, transform, layer)` | 纯色精灵（内部用 1×1 白纹理） |

```rust
// 一个绕中心旋转的 96×96 贴图精灵
let tf = Transform2D::IDENTITY
    .with_pos(Vec2::new(0.0, 0.0))
    .with_rot(t * 0.8);
r2d.add_sprite2d_default(
    SpriteRect::from_texture(Vec2::splat(-48.0), Vec2::splat(96.0)),
    Color::WHITE,
    tf,
    0.0,        // layer
    &my_texture,
);
```

#### Mesh / 多边形（非实例化）

| 函数 | 用法 | 说明 |
|---|---|---|
| `add_mesh` | `r2d.add_mesh(&verts, &tri_indices, color, layer)` | 显式顶点+三角形索引（世界坐标） |
| `add_polygon_fan` | `r2d.add_polygon_fan(&verts, color, layer)` | 顶点数组自动三角形扇（画圆/任意凸多边形） |
| `add_polygon_strip` | `r2d.add_polygon_strip(&verts, color, layer)` | 三角形条带（兼容行为同 fan） |
| `add_mesh_fn` | `r2d.add_mesh_fn(color, layer, \|sink\| { let i=sink.push_vertex(p); sink.push_tri(a,b,c); })` | 流程式安全建网格 |
| `add_mesh_fn_prealloc(max_verts, max_tris, color, layer, \|v_slice, t_slice\| {...})` | 已知顶点/三角形数时直接写预分配切片，返回 `(实际顶点数, 三角形数)` | 高性能路径，与 `add_mesh_fn` 等价但零帧内堆分配（除了 realloc）；虽然参数带有 max 但建议不要浪费>:( |

```rust
// 画一个圆（中心 c, 半径 r）：fan 顶点 = 中心 + 圆周采样
let mut verts = Vec::with_capacity(24);
verts.push(c);
for i in 0..=22 {
    let a = i as f32 / 22.0 * std::f32::consts::TAU;
    verts.push(c + Vec2::new(a.cos(), a.sin()) * r);
}
r2d.add_polygon_fan(&verts, Color::CYAN, 1.0);
```

`add_mesh_fn_prealloc` 示例（已知 4 顶点 / 2 三角形，直接写预分配切片）：

```rust
use rjw_2d_render::VertexP3U2C4; // 闭包接收的切片元素类型

// 画一个菱形（世界坐标直通 VP，颜色统一取 color）
r2d.add_mesh_fn_prealloc(4, 2, Color::CYAN, 1.0, |v_slice, t_slice| {
    let pts = [
        glam::Vec2::new(0.0, -20.0),
        glam::Vec2::new(20.0, 0.0),
        glam::Vec2::new(0.0, 20.0),
        glam::Vec2::new(-20.0, 0.0),
    ];
    for (dst, src) in v_slice.iter_mut().zip(pts.iter()) {
        dst.pos = [src.x, src.y, 0.0]; // 只写 pos；颜色由内部统一填
    }
    t_slice[0] = rjw_2d_render::TriIndicies::new(0, 1, 2);
    t_slice[1] = rjw_2d_render::TriIndicies::new(0, 2, 3);
    (4, 2) // (实际顶点数, 实际三角形数)
});
```

> 💡 注意：闭包里「实际用了多少就返回多少」`(used_verts, used_tris)`——max 只是容量上限，别浪费 >:(

`add_mesh_fn` 示例（流程式 push，无需预知数量）：

```rust
// 用 push_vertex（返回局部索引）构建，再 push_tri 组三角形
r2d.add_mesh_fn(Color::PURPLE, 1.0, |sink| {
    let a = sink.push_vertex(glam::Vec2::new(-10.0, -10.0));
    let b = sink.push_vertex(glam::Vec2::new(10.0, -10.0));
    let c = sink.push_vertex(glam::Vec2::new(0.0, 12.0));
    sink.push_tri(a, b, c);
});
```

#### 提交

| 函数 | 用法 | 说明 |
|---|---|---|
| `render` | `r2d.render(&ClearConfig { ... })` | **全流程**：begin_frame → 创建 pass（按 clear）→ 绘制 → 提交呈现 |
| `flush` | `r2d.flush(&mut pass)` | 只录制绘制到**用户自己建**的 pass（自管 encoder/present） |
| `begin_frame` | `r2d.begin_frame() -> Option<(SurfaceTexture, TextureView)>` | 手动获取表面 |

```rust
// 最常用：录完命令后
r2d.render(&ClearConfig {
    color: Some(wgpu::Color { r: 0.1, g: 0.2, b: 0.15, a: 1.0 }),
    depth: None,
    stencil: None,
});
```

#### 访问器

| 函数 | 说明 |
|---|---|
| `white_texture()` | 1×1 白色默认纹理引用 |
| `device()` / `queue()` | 暴露底层 wgpu 给高级用法 |

#### 分页机制（无需手动处理）

- 实例缓冲是**页池**：单帧精灵数量可超 `MAX_INSTANCES_PER_DRAW`(4096)
- `prepare()` 自动分页、每页只写一次、`draw()` 逐页绑定/绘制
- **不要**自己裁减数量去凑 4096；也不用担心覆盖问题（每页独立）

---

## 6. ClearConfig（清屏配置）

```rust
pub struct ClearConfig {
    pub color:   Option<wgpu::Color>, // Some=清屏 / None=保留旧内容
    pub depth:   Option<f32>,         // Some=清深度（自动建深度纹理）
    pub stencil: Option<u32>,         // Some=清模板
}
```

```rust
// 清深度的 2D 场景
r2d.render(&ClearConfig {
    color: Some(wgpu::Color::BLACK),
    depth: Some(1.0),
    stencil: None,
});
```

---

## 7. 其他常用小类型速查

| 类型 / 函数 | 位置 | 用途 |
|---|---|---|
| `KeyState::pressed()/released()` | `rjw_keystate` | 按住/松开 |
| `KeyState::down_edge()/up_edge()` | `rjw_keystate` | 按下/松开**那一帧**（瞬时操作用它） |
| `KeyState::true_edge()/down_true_edge()` | `rjw_keystate` | 系统级真实边沿（按住期间不重复触发） |
| `KeyCode::KeyW/...` | `rjw_main` 重导出 winit | 键盘常量 |
| `MouseButton::Left/...` | winit | 鼠标按钮 |
| `ctx.timer.dt().get_f32()` | `rjw_time` | 帧间隔秒（建议 `min(0.05)`） |
| `ctx.timer.get_fps()` | `rjw_time` | FPS |
| `ArcTextureWrapped.uid` | `rjw_render` | 纹理唯一 ID（合批内部用） |

---

*还想看更多？源码在 `crates/rjw_*/src/`，目录与本文一一对应。*