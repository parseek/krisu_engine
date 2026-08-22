# API 参考手册（免读源码版）

> 为**不想读源代码的人和 AI** 准备的速查 API 参考。
> 覆盖：`Color` / `ColorF64`、`Transform2D`、`Camera2D`、`Render2D`、`RStates` 及 builder 责任链、相关小类型。
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
  - [5.4.2 静态网格 StaticMesh](#542-静态网格-staticmesh)
- [6. RStates 渲染状态与 Builder 责任链](#6-rstates-渲染状态与-builder-责任链)
- [7. ClearConfig（清屏配置）](#7-clearconfig清屏配置)
- [8. DynamicAtlas（纹理图集）](#8-dynamicatlas纹理图集)
- [9. Text（文本渲染）](#9-text文本渲染)
- [10. 其他常用小类型速查](#10-其他常用小类型速查)
- [11. UI（rjw_ui）](#11-uirjw_ui)

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

let red = Color::rgba(1.0, 0.0, 0.0, 0.5);
let green = Color::rgba_u8(60, 200, 80, 255);
let arr: [f32; 4] = Color::WHITE.into();
```

### `ColorF64`（f64 存储，用于 wgpu 清屏）

| 函数 | 用法 | 说明 |
|---|---|---|
| `ColorF64::rgba` | `ColorF64::rgba(f64, f64, f64, f64)` | 高精度 |
| `.into()` | `let c: wgpu::Color = ColorF64::rgba(...).into();` | 直接转换给 `ClearConfig.color` |

---

## 2. Transform2D（变换）

crate：`rjw_transform`

```rust
pub struct Transform2D {
    pub pos:      glam::Vec2,
    pub scale:    glam::Vec2,
    pub rotation: f32,
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
| `with_walk_by` | `.with_walk_by(local)` | 按当前旋转方向位移 |
| `with_scale_by` | `.with_scale_by(factor)` | 缩放乘 |
| `with_rotate_by` | `.with_rotate_by(rad)` | 旋转加 |

#### 空间运算

| 函数 | 说明 |
|---|---|
| `transform_point(local)` | 局部点 → 父/世界点 |
| `inverse_transform_point(world)` | 世界点 → 局部点（命中检测用） |
| `transform_vec(local_vec)` | 局部方向 → 世界方向 |
| `inverse_transform_vec(world_vec)` | 反过来 |
| `with_transform(&parent)` | 组合父级：`result = parent * self` |
| `inverse()` | 逆变换对象 |

> 💡 **旋转中心 = pos**：让精灵绕自身中心转，矩形写成 `SpriteRect::from_texture(Vec2::splat(-w/2), Vec2::splat(w))`。

---

## 3. Camera2D（相机）

crate：`rjw_transform`

```rust
pub struct Camera2D {
    pub position:     Vec2,  // 相机中心（世界）
    pub rotation:     f32,
    pub zoom:         Vec2,
    pub viewport_pos: Vec2,  // 视口左上角（窗口像素）
    pub viewport_size: Vec2, // 视口尺寸（像素）
}
```

#### 构造 / 视口

| 函数 | 用法 | 说明 |
|---|---|---|
| `Camera2D::new` | `Camera2D::new(Vec2::new(w, h))` | 以窗口尺寸建相机；**之后必须 `set_vp`** |
| `set_vp` | `cam.set_vp(Vec2::new(w, h), Vec2::ZERO)` | 设置视口大小 + 位置（高 DPI 用 `render.size()` 的物理像素） |

#### 移动

| 函数 | 用法 | 说明 |
|---|---|---|
| `move_by` | `cam.move_by(Vec2::new(dx, dy))` | 绝对平移（不随旋转） |
| `walk_xy` | `cam.walk_xy(Vec2::new(lx, ly))` | 沿相机自身方向移动 |
| `walk_xplus` | `cam.walk_xplus(v)` | 沿相机横向 + 方向走 v |
| `walk_yplus` | `cam.walk_yplus(v)` | 沿相机纵向 + 方向走 v |

#### 矩阵 / 坐标转换

| 函数 | 说明 |
|---|---|
| `vp_matrix()` | 列主序 VP（P×V），直接喂 `render2d.set_mvp(...)` |
| `screen_to_world(screen_px)` | 窗口像素 → 世界坐标 |
| `world_to_screen(world)` | 世界 → 窗口像素 |
| `world_transform()` | 把相机看作 `Transform2D` |

```rust
// 鼠标指向的世界点
let mouse_px = ctx.mouse.get_mouse_position();
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
| `from_texture_px` | `SpriteRect::from_texture_px(tl, wh, uv_tl_px, uv_wh_px, inv_tex_wh)` | 按像素取纹理子区 |
| `new` | `SpriteRect::new(tl, wh, uv_tl, uv_wh)` | 全手动（UV 归一化） |

```rust
use rjw_2d_render::SpriteRect;
use glam::Vec2;

let a = SpriteRect::from_texture(Vec2::ZERO, Vec2::splat(32.0));
let b = SpriteRect::from_texture_px(
    Vec2::ZERO, Vec2::splat(32.0),
    Vec2::ZERO, Vec2::splat(32.0),
    Vec2::new(1.0 / 128.0, 1.0 / 128.0),
);
```

### 4.1 SpriteRectPx（像素 UV 精灵矩形）

crate：`rjw_2d_render`（`data` 模块）

与 `SpriteRect` 字段一一对应，但 `uv_tl` / `uv_wh` 以**像素**为单位（而非归一化坐标），
并额外持有纹理像素尺寸 `tex_wh`，便于实现裁剪类特效（shrink / expand / exceed 等）。
引擎主要使用 `ArcTextureWrapped`（内置 `width` / `height`），可直接用 `from_tex` / `from_tex_px` 构造。

```rust
pub struct SpriteRectPx {
    pub mesh_tl: Vec2, // 世界坐标左上角
    pub mesh_wh: Vec2, // 世界尺寸
    pub uv_tl:   Vec2, // 纹理子区左上角（像素）
    pub uv_wh:   Vec2, // 纹理子区尺寸（像素）
    pub tex_wh:  Vec2, // 纹理尺寸（像素）
}
```

| 函数 | 用法 | 说明 |
|---|---|---|
| `new` | `SpriteRectPx::new(tl, wh, uv_tl_px, uv_wh_px, tex_wh_px)` | 全手动（UV 像素） |
| `from_texture` | `SpriteRectPx::from_texture(tl, wh, tex_wh_px)` | 整张贴图 |
| `from_tex` | `SpriteRectPx::from_tex(tl, wh, &tex)` | 整张贴图（尺寸取 `tex.width`/`tex.height`） |
| `from_tex_px` | `SpriteRectPx::from_tex_px(tl, wh, uv_tl_px, uv_wh_px, &tex)` | 像素子区（尺寸取 `tex.width`/`tex.height`） |
| `to_sprite_rect` | `px.to_sprite_rect() -> SpriteRect` | 转为归一化 `SpriteRect`（`Into` 同样可用：`let s: SpriteRect = px.into();`） |
| `shrink_mesh_x/y/(x,y)` | `px.shrink_mesh_x(8.0)` | 世界坐标网格收缩（同 `SpriteRect`） |
| `shrink_uv_x/y/(x,y)` | `px.shrink_uv_x(8.0)` | UV 双侧各收窄 px（居中，clamp 到 0 不翻转） |
| `shrink_left/right/up/down` | `px.shrink_left(8.0)` | UV 单侧收窄 px（clamp 到 0） |
| `expand_left/right/up/down` | `px.expand_down(8.0)` | UV 单侧展开 px（**Clamp** 到纹理边界） |
| `expand` | `px.expand(8.0)` | UV 四周各展开 px（Clamp） |
| `exceed_left/right/up/down` | `px.exceed_left(8.0)` | UV 单侧展开 px（**不 Clamp**，允许越界采样） |

```rust
use rjw_2d_render::{SpriteRectPx, TextureWrapped};
use glam::Vec2;

// 整张贴图（尺寸自动取自纹理）
let base = SpriteRectPx::from_tex(Vec2::ZERO, Vec2::splat(64.0), &tex);
// 向下展开 16px（Clamp 到纹理下边界）
let r = base.expand_down(16.0).to_sprite_rect();
r2d.add_sprite2d(r, Color::WHITE, Transform2D::default(), 0.0, &tex);
// 或直接传 SpriteRectPx（add_sprite2d* 接受 impl Into<SpriteRect>）
r2d.add_sprite2d(base.shrink_uv_x(8.0), Color::WHITE, Transform2D::default(), 0.0, &tex);
// 向左越界展开 4px（不 Clamp）
r2d.add_sprite2d(base.exceed_left(4.0), Color::WHITE, Transform2D::default(), 0.0, &tex);
```

---

## 5. Render2D（2D 批渲染器）

crate：`rjw_2d_render`

> 生命周期：`Render2D::new(&RenderContext)` 持有 surface 的 `'static` 引用，要求 `RenderContext` 比 `Render2D` 活得更久。

### 5.1 创建 / 全局

| 函数 | 用法 | 说明 |
|---|---|---|
| `new` | `Render2D::new(&render_ctx)` | 基于 `RenderContext` 创建 |
| `set_mvp` | `r2d.set_mvp(cam.vp_matrix())` | 设置 VP（每帧渲染前调用） |
| `create_texture` | `r2d.create_texture("label", &rgba8, w, h)` | 建纹理（RGBA8，`len==w*h*4` 否则 panic），返回 `ArcTextureWrapped` |
| `register_mesh` | `r2d.register_mesh(Arc<MeshData>) -> u64` | 注册静态网格到全局 `MESHES` 注册表，返回可复用 `mesh_id` |
| `white_texture()` | `r2d.white_texture()` | 1×1 白色默认纹理引用 |
| `device()` / `queue()` | `r2d.device()` / `r2d.queue()` | 暴露底层 wgpu 给高级用法 |

### 5.2 全局默认渲染状态（责任链，返回 `&mut Self`）

| 函数 | 说明 |
|---|---|
| `reset_default_state()` | 重置为"出厂默认"（全零 bitfield） |
| `default_blend(mode)` | 设置默认 BlendMode |
| `default_samp_mag(f)` / `default_samp_min(f)` / `default_samp_mip(f)` | 设置默认采样器过滤 |
| `default_samp_addr_u(a)` / `default_samp_addr_v(a)` / `default_samp_addr_w(a)` | 设置默认寻址模式 |
| `default_cull(c)` / `default_polygon(p)` / `default_front_face(f)` | 默认剔除/光栅化 |
| `default_conservative_raster(b)` | 默认保守光栅化 |
| `default_depth_test(b)` / `default_depth_write(b)` / `default_depth_compare(f)` | 默认深度状态 |
| `default_stencil_test(b)` / `default_stencil_write(b)` / `default_stencil_compare(f)` | 默认模板状态 |
| `default_blend_state(d)` / `default_samp_state(d)` / `default_raster_state(s)` / `default_depth_state(s)` / `default_stencil_state(s)` | 批量设置 |

```rust
render2d
    .default_blend(BlendMode::Additive)
    .default_depth_test(true)
    .default_depth_write(true)
    .default_samp_addr_u(AddressMode::Repeat);
// 此后所有不链式的 add_* 命令都继承这些状态
```

### 5.3 Sprite 绘制（返回 `Sprite2DBuilder`）

| 函数 | 用法 | 说明 |
|---|---|---|
| `add_sprite2d` | `r2d.add_sprite2d(rect, color, transform, layer, &tex)` | 贴纹理精灵 |
| `add_sprite2d_solid` | `r2d.add_sprite2d_solid(rect, color, transform, layer)` | 纯色精灵（内部用 1×1 白纹理） |

```rust
// 绕中心旋转的精灵
let tf = Transform2D::IDENTITY
    .with_pos(Vec2::new(0.0, 0.0))
    .with_rot(t * 0.8);
r2d.add_sprite2d(
    SpriteRect::from_texture(Vec2::splat(-48.0), Vec2::splat(96.0)),
    Color::WHITE, tf, 0.0, &my_texture,
);
// 可链式设渲染状态：
r2d.add_sprite2d(rect, Color::WHITE, tf, 0.0, &tex)
    .blend(BlendMode::Additive)
    .samp_mag(FilterMode::Nearest);
```

### 5.4 Mesh / 多边形（返回 `MeshBuilder`）

| 函数 | 用法 | 说明 |
|---|---|---|
| `add_mesh` | `r2d.add_mesh(&verts, &tri_indices, color, layer)` | 显式顶点+三角形索引（世界坐标） |
| `add_polygon_fan` | `r2d.add_polygon_fan(&verts, color, layer)` | 顶点数组自动三角形扇 |
| `add_polygon_strip` | `r2d.add_polygon_strip(&verts, color, layer)` | 三角形条带 |
| `add_polygon_fan_uv` | `r2d.add_polygon_fan_uv(&verts, &uvs, color, layer)` | 三角形扇 + UV 坐标 |
| `add_polygon_strip_uv` | `r2d.add_polygon_strip_uv(&verts, &uvs, color, layer)` | 三角形条带 + UV 坐标 |
| `add_mesh_fn` | `r2d.add_mesh_fn(color, layer, \|sink\| { ... })` | 流程式安全建网格 |
| `add_mesh_fn_prealloc` | `r2d.add_mesh_fn_prealloc(max_verts, max_tris, color, layer, \|v_slice, t_slice\| { ... })` | 已知顶点/三角形数，直接写预分配切片 |

```rust
// 画一个圆（中心 c, 半径 r）
let mut verts = Vec::with_capacity(24);
verts.push(c);
for i in 0..=22 {
    let a = i as f32 / 22.0 * std::f32::consts::TAU;
    verts.push(c + Vec2::new(a.cos(), a.sin()) * r);
}
r2d.add_polygon_fan(&verts, Color::CYAN, 1.0);

// Mesh 可链式设纹理：
r2d.add_polygon_fan(&verts, Color::CYAN, 96.0)
    .set_texture(&tex)
    .blend(BlendMode::Multiply);
```

> 💡 `MeshBuilder` 独有 `.set_texture(&ArcTextureWrapped)`——mesh 默认白色纹理，此方法覆盖。

### 5.4.1 外部自定义绘制（`add_custom` / `CustomDraw`，返回 `CustomBuilder`）

| 函数 | 用法 | 说明 |
|---|---|---|
| `add_custom` | `r2d.add_custom(layer, \|pass\| { ... })` | 注入一段原生 wgpu 绘制调用，参与 (layer, states) 排序 |

**👉 完整可运行示例见 `examples/eg260731CustomDraw/`**（`cargo run -p eg260731CustomDraw`）：演示结构体形式 + 闭包形式 + 与引擎 Sprite 混排。

```rust
// 闭包直接传（blanket impl）—— pass 是 &mut wgpu::RenderPass
r2d.add_custom(1.0, |pass| {
    // 这里可以用任意原生 wgpu API：set_pipeline / draw / 自行绑定缓冲……
    // 注意：pass 管理器已在引擎内打开，不要 begin_render_pass
});

// 或实现 CustomDraw trait 的结构体
struct MyFx;
impl rjw_2d_render::CustomDraw for MyFx {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        // 自定义绘制……
    }
}
r2d.add_custom(1.0, MyFx);
```

> 💡 **最小完整模式**（自建管线 + 顶点缓冲，参考 `eg260731CustomDraw` 的 `Tri` 结构体）：
>
> ```rust
> // ① 建管线：模块 + 空 pipeline layout + 自定义 VertexBufferLayout
> let shader = device.create_shader_module(...);               // 自定义 WGSL
> let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
>     bind_group_layouts: &[],                 // 不需要 bind group 时传空
>     immediate_size: 0,
> });
> let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
>     layout: Some(&layout),                   // 其余字段按需（见 example）
>     ...
> });
>
> // ② 建顶点缓冲（bytemuck::cast_slice 传入 f32 数组）
> let vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
>     contents: bytemuck::cast_slice(&verts),
>     usage: wgpu::BufferUsages::VERTEX,
> });
>
> // ③ 在 CustomDraw::draw / 闭包中执行（pass 已由引擎打开）
> pass.set_pipeline(&pipeline);
> pass.set_vertex_buffer(0, vbo.slice(..));
> pass.draw(0..n_verts, 0..1);
> ```

要点：

- **签名**：`add_custom<CD: CustomDraw + 'static>(&mut self, layer, cd) -> CustomBuilder<'_>`；`CustomDraw: Send + Sync`，闭包 `Fn(&mut wgpu::RenderPass) + Send + Sync` 自动实现。
- **排序位置**：返回的 `CustomBuilder` 可链式设置 RStates（`.blend(...)` / `.depth_test(...)` 等，但注意 `CustomBuilder` **没有** `.set_texture()`），这些值参与 `(layer, states)` 排序，决定该闭包在 Sprite/Mesh 之间的**执行顺序**。
- **执行时机**：闭包在 `render()` 或 `flush()` 的 `draw()` 阶段被调用；`buf_custom_draws` 每帧结束后 `clear()`，**请勿**跨帧持有 `add_custom` 内部状态。
- **适用场景**：引擎封装之外的管线（自定义 shader、线框调试、后处理、自定义顶点格式等）。
- 若不链式调用 RStates，则 `CustomBuilder` 仍按 `default_rstates` 参与排序（resolve 后为整数值相加大致落在默认位置）。

### 5.4.2 静态网格 StaticMesh

| 函数 | 用法 | 说明 |
|---|---|---|
| `add_static_mesh` | `r2d.add_static_mesh(mesh_id, color, transform, layer, &tex)` | 静态网格实例（Transform2D 变换），返回 `StaticMeshBuilder` |
| `add_static_mesh_matrix` | `r2d.add_static_mesh_matrix(mesh_id, color, model_mat4, layer, &tex)` | 静态网格实例（直接列主序 Mat4，跳过 Transform2D 推导） |

`MeshData`（`rjw_render`）包装 GPU 顶点/索引缓冲：

| 函数 | 说明 |
|---|---|
| `MeshData::from_buffers(vb, ib, index_count)` | 直接包装已创建缓冲（自动分配 uid） |
| `MeshData::from_pod(device, &verts, &indices, label)` | 从 CPU 数据创建缓冲（`T: bytemuck::Pod`，索引 u16） |
| `mesh.uid` | 全局唯一 id（`HasUid` trait） |

```rust
use std::sync::Arc;
use rjw_2d_render::MeshData;

// ① 建单位圆网格（半径为 1，后续实例 transform = Translate(pos) * Scale(r) 复用）
let circle = Arc::new(MeshData::from_pod(render2d.device(), &verts, &idx, "circle"));
let circle_id = render2d.register_mesh(circle);

// ② 提交多个静态实例（同 mesh_id + 同 RStates + 同纹理 → 自动合批为极少数 DrawCall）
let white = render2d.white_texture().clone();
for inst in &instances {
    let tf = Transform2D::default().with_pos(inst.pos).with_scale(Vec2::splat(inst.r));
    render2d.add_static_mesh(circle_id, inst.color, tf, inst.layer, &white).done();
}

// 矩阵版（高级）
render2d.add_static_mesh_matrix(circle_id, Color::WHITE, model, 96.0, &tex).done();
```

要点：

- **`StaticMeshBuilder`**：与 `Sprite2DBuilder` / `MeshBuilder` 相同的 RStates 责任链（`.blend(...)` / `.samp_mag(...)` / `.depth_test(...)` 等），**没有** `.set_texture()`——纹理由 `add_static_mesh*` 参数直接传入；`.done()` 立即消费提交，或依赖 Drop 自动 push。
- **顶点坐标**：`MeshData` 顶点即世界坐标（静态网格走实例化直通 VP），变换由实例 `model` 提供；顶点自带 UV，配合采样器直通（同 `IDENTITY_INSTANCE` 语义）正确贴图。
- **合批条件**：`(mesh_id, rstates, tex_uid)` 相同且绘制序列连续 → 合并为同一次 `draw_indexed`。相同内容的网格请**复用同一个 `Arc<MeshData>`** 注册，否则 id 不同无法合批。
- **适用场景**：固定遮挡层、不参与 y-sort 的地图元素（石头、花、栅栏…）应静态化；**会插入实体绘制顺序的元素（如 RPG 中 `y_layer(foot_y)` 的树）保持动态**，切勿加入静态网格。

### 5.5 提交

| 函数 | 用法 | 说明 |
|---|---|---|
| `render` | `r2d.render(&ClearConfig { ... })` | **全流程**：begin_frame → 创建 pass → 绘制 → 提交呈现 |
| `flush` | `r2d.flush(&mut pass)` | 只录制绘制到用户自己建的 pass（仅传入 Pass） |
| `render_command_buffer` | `r2d.render_command_buffer(&ClearConfig, &target_view, depth)` | 仅编码为 `CommandBuffer`（不提交/不 present，适合离屏渲染/合并提交） |
| `begin_frame` | `r2d.begin_frame() -> Option<(SurfaceTexture, TextureView)>` | 手动获取表面 |

```rust
r2d.render(&ClearConfig {
    color: Some(wgpu::Color { r: 0.1, g: 0.2, b: 0.15, a: 1.0 }),
    depth: None,
    stencil: None,
});
```

### 5.6 分页机制（无需手动处理）

- 实例缓冲是**页池**：单帧精灵数量可超 `MAX_INSTANCES_PER_DRAW`(8192)
- `prepare()` 自动分页、每页只写一次、`draw()` 逐页绑定/绘制

### 5.7 纹理与采样器

- `TextureWrapped`（`rjw_render`）**只持有纹理本身**（`texture` / `view` / `width` / `height` / `uid`），**不再持有 sampler / bind group**——两者与纹理解耦。
- **采样器完全由 `RStates` 位域（bits 8..24）驱动**：`.samp_mag(Nearest)` / `.samp_addr_u(Repeat)` 等链式方法**真正生效**，`Render2D` 按需创建并缓存 `wgpu::Sampler`（默认线性 + ClampToEdge 有零开销快路径）。
- **bind group 由 `Render2D` 缓存**：key = `(tex_uid, samp_key)`，value 持有 `Arc<Texture>` 防悬挂；`prepare` 末尾自动剔除 `TEXTURES.remove` 掉的失效条目（资源正确释放）。
- 全局注册表 `TEXTURES` 支持 `register` / `register_named` / `get` / `remove` / `remove_name_mapping` / `rename` / `contains_uid` / `contains_name`（`TypedRegistry<TextureWrapped>`）。

---

## 6. RStates 渲染状态与 Builder 责任链

crate：`rjw_2d_render`（`rstates` 模块）

`RStates` 是 u64 bitfield，涵盖 6 个控制域：Blend / Sampler / Cull+Raster / Depth / Stencil / Reserved。

### 6.1 RStates 自身方法（用于构造，不可变链式）

| 分类 | 方法 | 说明 |
|---|---|---|
| Blend | `blend(BlendMode)` / `blend_state(BlendDesc)` | Alpha/Additive/Multiply/Premultiplied/Inverse/Subtract/Min/Max/Disabled |
| Sampler | `samp_mag(f)` / `samp_min(f)` / `samp_mip(f)` | Linear / Nearest |
| | `samp_addr_u(a)` / `samp_addr_v(a)` / `samp_addr_w(a)` | ClampToEdge / Repeat / MirrorRepeat |
| | `samp_state(SamplerDesc)` | 批量设置采样器 |
| Cull+Raster | `cull(CullMode)` / `polygon(PolygonMode)` / `front_face(FrontFaceWinding)` / `conservative_raster(bool)` | None/Front/Back; Fill/Line/Point; Ccw/Cw |
| | `raster_state(RasterState)` | 批量设置光栅化 |
| Depth | `depth_test(bool)` / `depth_write(bool)` / `depth_compare(CompareFunc)` | Less/LessEq/Greater/... |
| | `depth_state(DepthState)` | 批量设置深度 |
| Stencil | `stencil_test(bool)` / `stencil_write(bool)` / `stencil_compare(CompareFunc)` | Always/Never/... |
| | `stencil_state(StencilState)` | 批量设置模板 |

### 6.2 Builder 链方法（`Sprite2DBuilder` / `MeshBuilder` / `StaticMeshBuilder` 通用）

| 分类 | 方法 |
|---|---|
| Blend | `.blend(m)` / `.blend_state(d)` |
| Sampler | `.samp_mag(f)` / `.samp_min(f)` / `.samp_mip(f)` / `.samp_addr_u(a)` / `.samp_addr_v(a)` / `.samp_addr_w(a)` / `.samp_state(d)` |
| Cull+Raster | `.cull(c)` / `.polygon(p)` / `.front_face(f)` / `.conservative_raster(b)` / `.raster_state(s)` |
| Depth | `.depth_test(b)` / `.depth_write(b)` / `.depth_compare(f)` / `.depth_state(s)` |
| Stencil | `.stencil_test(b)` / `.stencil_write(b)` / `.stencil_compare(f)` / `.stencil_state(s)` |
| **MeshBuilder only** | `.set_texture(&tex)` |
| **StaticMeshBuilder only** | `.done()`（立即消费提交；亦可靠 Drop 自动 push） |

> 💡 `StaticMeshBuilder` 的纹理由 `add_static_mesh*` 参数传入，因此**没有** `.set_texture()`。
> 采样器相关方法（`.samp_*`）会真正创建对应 GPU 采样器（见 §5.7）。

不链式调用 = `rstates: None` → `draw()` 阶段 resolve 为 `Render2D.default_rstates`。

### 6.3 重要类型一览

| 类型 | 值 |
|---|---|
| `RStates` | u64 bitfield，`RStates::default() / new()` = 全零（默认） |
| `BlendMode` | `Alpha` / `Additive` / `Multiply` / `Premultiplied` / `Inverse` / `Subtract` / `Min` / `Max` / `Disabled` |
| `FilterMode` | `Linear` / `Nearest` |
| `AddressMode` | `ClampToEdge` / `Repeat` / `MirrorRepeat` |
| `CullMode` | `None` / `Front` / `Back` |
| `PolygonMode` | `Fill` / `Line` / `Point` |
| `FrontFaceWinding` | `Ccw` / `Cw` |
| `CompareFunc` | `Never` / `Less` / `Equal` / `LessEq` / `Greater` / `NotEq` / `GreaterEq` / `Always` |
| `BlendDesc` | `{ blend_mode: BlendMode }` |
| `SamplerDesc` | `{ mag, min, mip: FilterMode, addr_u, addr_v, addr_w: AddressMode }` |
| `RasterState` | `{ cull: CullMode, polygon: PolygonMode, front_face: FrontFaceWinding, conservative: bool }` |
| `DepthState` | `{ test: bool, write: bool, compare: CompareFunc }` |
| `StencilState` | `{ test: bool, write: bool, compare: CompareFunc }` |
| `MeshData` | 静态网格：`{ vertex_buffer, index_buffer, index_count, uid }`（`rjw_render`） |
| `StaticMeshBuilder<'a>` | `add_static_mesh*` 返回（立即可 `.done()` 提交） |
| `HasUid` | trait：`fn uid(&self) -> u64`（`rjw_render`） |
| `TypedRegistry<T: HasUid>` | 泛型注册表：`register` / `register_named` / `get` / `get_ref` / `remove` / `remove_name_mapping` / `rename` / `contains_uid` / `contains_name` |
| `MESHES` | 全局静态网格注册表（`TypedRegistry<MeshData>`，`rjw_render`） |

### 6.4 使用示例

```rust
use rjw_2d_render::{BlendMode, FilterMode, AddressMode, DepthState, CompareFunc};

// 不链式 = 默认
render2d.add_sprite2d(rect, Color::WHITE, tf, 0.0, &tex);

// 单条链式覆盖
render2d.add_sprite2d(rect, Color::WHITE, tf, 0.0, &tex)
    .blend(BlendMode::Additive)
    .samp_addr_u(AddressMode::Repeat)
    .samp_mag(FilterMode::Nearest);

// Mesh + set_texture + 渲染状态
render2d.add_polygon_fan(&verts, Color::CYAN, 96.0)
    .set_texture(&tex)
    .blend(BlendMode::Multiply);

// 批量设置
render2d.add_sprite2d(rect, Color::WHITE, tf, 0.0, &tex)
    .depth_state(DepthState { test: true, write: true, compare: CompareFunc::Less });

// 全局默认（责任链，返回 &mut Render2D）
render2d
    .default_blend(BlendMode::Additive)
    .default_depth_test(true)
    .default_depth_write(true);
```

---

## 7. ClearConfig（清屏配置）

```rust
pub struct ClearConfig {
    pub color:   Option<wgpu::Color>,
    pub depth:   Option<f32>,
    pub stencil: Option<u32>,
}
```

```rust
r2d.render(&ClearConfig {
    color: Some(wgpu::Color::BLACK),
    depth: Some(1.0),
    stencil: None,
});
```

---

## 8. DynamicAtlas（纹理图集）

crate：`rjw_atlas`

```rust
pub struct AtlasConfig { pub max_pages: usize, pub padding: u32, pub lifetime: u32 }
pub struct AtlasRegion { pub tl_px: (u32,u32), pub wh_px: (u32,u32), pub origin_px: (u32,u32), pub page_uid: u64 }
pub struct DynamicAtlas<K = String>  // K 为精灵键类型，String 特化提供 TOML 导入导出
pub struct StaticAtlas<K = String>   // 泛型与 DynamicAtlas 一致；from_toml/to_toml (serde feature only)
```

> 💡 `DynamicAtlas` / `StaticAtlas` 均实现 `Index<&Q>` / `IndexMut<&Q>`（`K: Borrow<Q>`）：
> `atlas[&key]` / `atlas["name"]` 直接读写区域，`get()` 语义见下表（DynamicAtlas 的 `get` 会刷新寿命）。

| 方法 | 说明 |
|---|---|
| `DynamicAtlas::new(device, queue, layout, config, page_size)` | 创建空图集（`page_size` 为单页像素尺寸，如 2048） |
| `insert(name, rgba, w, h, origin_px, clamp_margin)` | 插入/替换精灵（完整参数） |
| `insert_ex(name, rgba, w, h)` | ★ 最常用：origin=(0,0), clamp_margin=true，自动保存源数据 |
| `insert_ex_origin(name, rgba, w, h, origin_px)` | 指定原点，clamp_margin=true |
| `insert_ex_permanent(name, rgba, w, h)` | 常驻精灵（不会过期踢出） |
| `insert_dyn(name, w, h, origin_px, clamp_margin, regen)` | 动态再生精灵（每次复活调生成器） |
| `insert_no_clamp(name, rgba, w, h)` | origin=(0,0), clamp_margin=false |
| `insert_white()` | 插入 1×1 白像素 |
| `get(name)` | 查找（重置寿命，不触发复活） |
| `get_or_revive(name)` | ★ 查找；若被踢出则自动复活 |
| `load_toml(toml_str, rgba_provider)` | 从 TOML 批量导入（闭包提供源纹理 RGBA） |
| `export_toml()` | 导出当前 entries 为 TOML 文本 |
| `end_frame()` | 寿命-1，有源数据→墓碑；常驻直接删除 |
| `compact()` | 去碎片：带源条目全量重排到最少页（重传纹理，`generation`+1）；含永久条目时退回按页重建空闲矩形 |
| `generation()` | 去碎片重排世代号（搬动条目时 +1；缓存区域者据此刷新） |
| `page_size()` / `page_count()` / `texture_uid_of(name)` | 查询 |
| `parse_toml_entries(toml_str)` | 辅助：解析 TOML 返回原始条目表 |
| `StaticAtlas::from_toml(s)` | 从 TOML 反序列化（`K=String` 特化） |
| `StaticAtlas::get(name)` | 查找（接受 `&str` 等可借用键） |

## 9. Text（文本渲染）

crate：`rjw_text`

基于 `cosmic-text` 排版 + `swash` 字形光栅化 + `DynamicAtlas` 字形缓存。

```rust
pub struct Text { /* font_system: FontSystem, glyph_cache: DynamicAtlas<cosmic_text::CacheKey>, ... */ }
```

| 方法 | 说明 |
|---|---|
| `Text::new(device, queue, layout)` | 创建字体管理器（自动加载系统字体） |
| `load_font_data(data: Vec<u8>)` | 加载额外的 ttf/otf 字体数据 |
| `create_buffer(text, attrs, size, line_height, align) -> Arc<Buffer>` | 排版并返回**共享只读** `Arc<Buffer>`（相同输入命中缓存，O(1) 签名预过滤、不深拷贝） |
| `create_buffer_wrap(text, attrs, size, line_height, align, wrap_width, policy)` | 同 `create_buffer_policy`，但指定**排版宽度**（物理像素）：超出自动**换行**（多行）；宽度参与缓存键 |
| `measure(text, attrs, size, line_height, align) -> Vec2` | 排版 + 测量内容宽高（GUI 布局用） |
| `measure_buffer(buffer) -> Vec2` | 已排版 Buffer 的内容宽高（行盒；空文本返回 (0,0)） |
| `visual_lines(buffer) -> Vec<VisualLine>` | **视觉行**（自动换行后）列表：`(byte_start, byte_end, top, width)`——光标/点击/选择与显示对齐（TextArea） |
| `white_region() -> Option<AtlasRegion>` | **字形图集页内的 WHITE 基础纹理**（1×1 clamp_margin）——UI 实心填充与字形同页合批 |
| `draw_label(r2d, text, color, size, line_height, pos, family, align, layer) -> Vec2` | ★ 一行渲染：pos=左上角，返回内容宽高（feature = `rjw_2d_render`） |
| `draw_label_ex(r2d, text, color, size, line_height, pos, family, align, layer, origin) -> Vec2` | 扩展版：origin 归一化到 [0,1]，(0.5,0.5)=居中（feature = `rjw_2d_render`） |
| `draw_label_with(text, size, line_height, pos, family, align, origin, callback) -> Vec2` | 回调版标签渲染：不绑定 Render2D，GUI 自定义字形绘制 |
| `draw_text(buffer, callback)` | 遍历字形精灵，闭包 `(region, world_pos, world_size)` 自定义绘制 |
| `Text::text(..) -> TextLayout` | 责任链入口（阶段一：排版配置；常量字符串 `TextStorage` 内联） |
| `TextLayout::size/line_height/line_space/align/attrs/font_family` | 排版链设置 |
| `TextLayout::measure() -> Vec2` | 排版 + 测量内容宽高（不消费链） |
| `TextLayout::into_buffer() -> Arc<Buffer>` | 排版并交出共享 cosmic-text Buffer（消费链） |
| `Text::render_from(&mut self, buffer: &Buffer) -> TextRender` | ★ 从用户保存的 `Arc<Buffer>` 直接进入阶段二（责任链渲染，跳过整形；静态大文本手动缓存路径） |
| `TextLayout::into_render() -> TextRender` | 转阶段二（用 `Text` 内部缓冲，单标签快速路径，跨帧复用容量） |
| `TextLayout::into_render_with(&mut TextBuffer) -> TextRender` | 转阶段二（用户持缓冲，多标签并存） |
| `TextLayout::precache() -> Self` | 预缓存：字形入图集（预热），返回自身可稍后渲染 |
| `TextLayout::into_render() -> TextRender` | 转阶段二：**直接堆存储** |
| `TextRender::from_layout(layout)` | 从 `TextLayout` 转换（TextRender 的函数） |
| `TextRender::new(text, string, size, lh, align)` | 直接构造（跳过 builder） |
| `TextRender::origin/origin_px/offset/color/map` | 渲染设置：归一化/像素原点、偏移、全局色、逐字形修改 |
| `TextRender::transform(Option<Transform2D>)` | 渲染级变换：作用整个文本块，`draw_with` / `draw_sprite2d` / `draw_2d_gradient` 均应用 |
| `TextRender::draw_with(callback)` | 回调 `(measure, line, region, topleft)` 绘制（核心，无 feature 依赖） |
| `TextRender::draw_sprite2d(r2d, layer)` | 直接渲染到 Render2D（feature = `rjw_2d_render`） |
| `TextRender::draw_2d_gradient(r2d, layer, mode, axis, stops)` | 渐变渲染：Glyph/Line/Frame × 横/竖向，动态 mesh 逐顶点色（feature = `rjw_2d_render`） |
| `TextStorage` / `LineSpace` / `GradientMode` / `GradientAxis` | 文本内联存储 / 行距(像素·倍率) / 渐变区间 / 渐变方向 |
| `GlyphType` | 字形类型：`Normal`（单色可染色）/ `Color`（Emoji 等）；`GlyphData::glyph_str()` 取对应字符 &str |
| `Text::build_style() -> TextStyle` | 构建可复用样式（临时持有 `&mut Text`；可复用多次 `text(..)`） |
| `Style` / `TextStyle` | 解耦样式（family=`AttrsOwned` 无借用，克隆继承 `base.clone().size(..)`）/ 临时样式句柄 |
| `RenderDefaults` / `OwnedAttrs` | 渲染默认（color/origin/offset/transform）/ 无借用完整文本属性（cosmic-text `AttrsOwned`） |

> **性能**：`Text` 内置**排版缓存（LRU）**——按（文本/字号/行高/对齐/attrs）缓存 cosmic-text 排版，相同输入经 **O(1) 签名**预过滤后返回共享 `Arc<Buffer>`（不深拷贝，跳过重复整形；上限 [`MAX_LAYOUT_CACHE`]=128，满时淘汰最久未用）。缓存启用规则：**Debug 恒缓存**；**Release 仅缓存 ≤ [`LARGE_TEXT_CACHE_LIMIT`]=512 字节的小文本**（大文本多为动态/低频，不入缓存、每帧直接整形）；静态大文本请保存 `Arc<Buffer>` 经 `render_from` 手动复用。空格等**无图字形**只判定一次（`no_image`）；字形图集去碎片重排后自动同步区域。

```rust
use rjw_text::{Text, Align};

let mut font = Text::new(device, queue, layout);

// 左上角单行文本
font.draw_label(r2d, "Hello World", Color::WHITE, 14.0, 18.0, Vec2::new(10.0, 10.0), "SimHei", Align::Left, 0.0);

// 屏幕居中 Game Over
let size = font.draw_label_ex(r2d, "GAME OVER\n按 R 重开", Color::RED, 22.0, 28.0, cam.position, "SimHei", Align::Center, 1e7, Vec2::new(0.5, 0.5));
```

> 注：`draw_label` / `draw_label_ex` 依赖默认 feature `rjw_2d_render`；纯测量/回调 API
> （`measure` / `measure_buffer` / `draw_text` / `draw_label_with`）不依赖渲染器，
> 可通过 `default-features = false` 关闭该 feature。

## 10. 其他常用小类型速查

| 类型 / 函数 | 位置 | 用途 |
|---|---|---|
| `KeyState::pressed()/released()` | `rjw_keystate` | 按住/松开 |
| `KeyState::down_edge()/up_edge()` | `rjw_keystate` | 按下/松开**那一帧** |
| `KeyState::true_edge()/down_true_edge()` | `rjw_keystate` | 系统级真实边沿 |
| `KeyCode::KeyW/...` | `rjw_main` 重导出 winit | 键盘常量 |
| `MouseButton::Left/...` | winit | 鼠标按钮 |
| `ctx.timer.dt().get_f32()` | `rjw_time` | 帧间隔秒 |
| `ctx.timer.get_fps()` | `rjw_time` | FPS |
| `ArcTextureWrapped.uid` | `rjw_render` | 纹理唯一 ID |
| `Sprite2DBuilder<'a>` | `rjw_2d_render` | add_sprite2d* 返回 |
| `MeshBuilder<'a>` | `rjw_2d_render` | add_mesh / add_polygon_* 返回 |
| `StaticMeshBuilder<'a>` | `rjw_2d_render` | add_static_mesh* 返回，`.done()` 立即提交 |
| `MeshData` | `rjw_render` | 静态网格（GPU 顶点/索引 + uid） |
| `HasUid` | `rjw_render` | 全局唯一 id trait |
| `TypedRegistry<T>` | `rjw_render` | 泛型线程安全注册表（纹理/网格共用） |
| `CustomDraw` | `rjw_2d_render` | 外部绘制 trait（闭包 blanket impl） |
| `CustomBuilder<'a>` | `rjw_2d_render` | add_custom 返回，可链式 RStates |

---

## 11. UI（rjw_ui）

> crate：`rjw_ui`。坐标一律**屏幕像素**（左上角原点、Y+ 向下）；交互状态经 **ID** 持久化于 `UiState`（应用持有）。完整概念见 [ENGINE_GUIDE.md](ENGINE_GUIDE.md)「18. UI」。

### 入口与生命周期

| 函数 | 签名 / 用法 | 说明 |
|---|---|---|
| `Ui::begin` | `Ui::begin(window, &mut text, &mut state) -> UiInit` | 一帧一次；`window` 用于 IME 候选框定位与光标图标。**输入与绘制解耦**：输入经 `UiInit::capture` 快照、相机/渲染器延迟到 `Ui::finish` 传入 |
| `UiInit::capture(&MouseInput, &KeyboardInput)` | `.capture(&ctx.mouse, &ctx.keyboard)` | 把键盘/鼠标设备状态**拷贝**为 Ui 自持快照（省略 = 空输入，headless 安全） |
| `UiInit::theme(Theme)` | `.theme(Theme::dark())` | 主题（默认浅色；`Theme::dark()` 深色） |
| `UiInit::base_layer(f64)` | `.base_layer(1e7)` | 基层层级（默认 `1e7`） |
| `UiInit::scale_factor(f64)` | `.scale_factor(ctx.scale_factor().unwrap_or(1.0))` | DPI：控件坐标/字号按逻辑像素，内部换算物理像素（默认 1.0） |
| `UiInit::debug_layout(bool)` | `.debug_layout(true)` | 调试 UI 布局：给每个控件/容器矩形画描边（颜色/宽度见 [样式小节](#样式theme可-clone-覆盖) 的 `DebugStyle`；默认 false） |
| `Ui::debug_layout(bool)` | `ui.debug_layout(on)` | 同 `UiInit::debug_layout`，帧内运行时开关 |
| `UiInit::build()` | → `Ui` | 完成构建（内部 `state.begin_frame()`） |
| `Ui::finish(&Viewport, &mut Render2D)` | `ui.finish(&viewport, r2d)` | 按 `(win, depth, 图形/文字, 录制序)` 序**免全量排序**提交（win + depth 分桶、桶内保持录制序，语义与排序完全等价）并提交绘制（视口/渲染器在此延迟传入；UI 无需相机，`Viewport{pos,size}` 提供屏幕固定变换）；清空帧状态 |
| `UiState::new()` | 应用持有 | 跨帧持久状态容器 |
| `UiState::reset()` / `remove(id)` | 示例"R 重开" | 清空全部 / 移除单个控件状态 |
| `UiState::capturing_text()` | `if !ui_state.capturing_text() { /* 快捷键 */ }` | 输入框聚焦时屏蔽应用快捷键 |

### 容器（布局）

**容器责任链 builder**（推荐）：统一 `window_at*` / `panel_at` / `modal_at*` 的选项组合，
选项链式设置、`.show(f)` 终结（旧 `*_at` 入口保留，薄委托）：

**单位（[`Size`](crate::draw::Size) / [`Position`](crate::draw::Position)）**：坐标 / 尺寸参数
（`pos`、`width`、builder 的 `.pos/.width` 等）接受带单位包装——`Size::Logical` /
`Position::Logical`（默认，`From<f32/Vec2>`，× scale 换算并取整）或 `Size::Physical` /
`Position::Physical`（原样）。换算仅在 **API 边界**一次完成，Ui 内部布局 / 命中 / 绘制
一律**物理像素**（`Ui::scale()` 只供边界换算；`scale_factor` 在 `build()` 时把 Theme
**预乘**——全部样式尺寸 / 字号 × scale 取整，见 [`Theme::scaled`](crate::style::Theme::scaled)）。
`label_wrap_at` / `view_at` / `scroll_at` / `list_at` / `flex_at` / `grid_at` / `divider_at` /
`rounded_rect_at` / `gradient_rect_at` 同样带单位（`pos` → `Position`，`max_w` / `size` /
`view_size` / `total_h` / `w` / `radius` → `Size`）。

| 入口 | 链 | 语义 |
|---|---|---|
| `ui.window(id)` | `.pos(..)` `.width(w)` `.strict()` `.topmost(bool)` `.style(PanelStyle)` `.clamp(WindowClamp)` `.show(\|w\| ..)` | 窗口 = `window_at` + `window_at_w` + `window_at_strict` 统一入口；`.width` = 固定宽（右下角可缩放）；`.strict` = 强制裁剪；`.style` = 逐窗口样式覆盖（默认 `Theme::panel`）；`.clamp` = 位置约束（`Screen` 限位不跑出屏幕（默认；窗口比画面大时仍可拖动）/ `Free` 自由 / `Locked` 锁定位置不可拖） |
| `ui.panel()` | `.pos(..)` `.drag(id)` `.style(..)` `.show(\|pp\| ..)` | 面板 = `panel_at` + `drag_panel_at` 统一入口 |
| `ui.modal(id)` | `.pos(..)` `.width(w)` `.show(\|m\| ..)` | 模态对话框 = `modal_at` + `modal_at_w` 统一入口 |

选项载体 `WindowOptions` / `PanelOptions`（公开，可独立构造/复用）。容器闭包内经
`UiAdd::window(id)` / `UiAdd::panel()` 同样可用。

| 函数 | 签名 | 语义 |
|---|---|---|
| `label_at` | `ui.label_at(pos, text) -> Vec2` | place：绝对定位 + 内容自然尺寸 |
| `pack_at` | `ui.pack_at(pos, side, \|p\| ...) -> Vec2` | pack：按 `PackSide::Top/Left` 堆叠，宽/高 = 最大子项 |
| `panel_at` | `ui.panel_at(pos, \|pp\| ...) -> Vec2` | 背景 + 边框 + 内容垂直堆叠，尺寸自动包裹（等价 `ui.panel().pos(pos).show(..)`） |
| `drag_panel_at` | `ui.drag_panel_at(id, pos, \|pp\| ...) -> Vec2` | 同 panel_at，且按住面板任意处可**拖动**（位置持久于 `UiState.panel_pos`；拖动期间子控件不响应；等价 `ui.panel().pos(pos).drag(id).show(..)`） |
| `window_at` | `ui.window_at(id, pos, \|w\| ...) -> Vec2` | **可重叠窗口**：点击置顶（焦点 z-order，`UiState.window_z`）+ 可拖拽；窗口内同一 layer 按"背景/图形→文字"绘制，不做元素重叠处理（等价 `ui.window(id).pos(pos).show(..)`） |
| `window_at_w` | `ui.window_at_w(id, pos, width, \|w\| ...) -> Vec2` | 同 `window_at`，固定宽 + 右下角可鼠标缩放（等价 `ui.window(id).pos(pos).width(width).show(..)`） |
| `window_at_strict` | `ui.window_at_strict(id, pos, \|w\| ...) -> Vec2` | 同 `window_at`，内容强制裁剪到窗口矩形（等价 `ui.window(id).pos(pos).strict().show(..)`） |
| `window_at_strict_w` | `ui.window_at_strict_w(id, pos, width, \|w\| ...) -> Vec2` | 固定宽 + 严格裁剪（等价 `.width(w).strict()`） |
| `scroll_at` | `ui.scroll_at(pos, view_size, id, \|s\| ...) -> Vec2` | **滚动容器**：内容在可视区内垂直堆叠（pack Top），滚轮 / 滚动条（拖 thumb、点轨道翻页）滚动；可视区外**裁剪**；偏移持久于 `UiState.scrolls` |
| `grid_at` | `ui.grid_at(pos, cols, id, \|g\| ...) -> Vec2` | 均匀网格；`id` 缓存单元格尺寸（跨帧稳定） |
| `flex_at` | `ui.flex_at(pos, total_h, &[w1,w2,..], \|f, i\| ...) -> Vec2` | **flex 容器**：固定总高 `total_h` 按 `weights` **权重等分**子项高度（扣 gap；回调按索引布局，同帧精确）；内容超高溢出可见（需滚动时内嵌 `scroll_at`） |
| 容器内 `*_at(offset)` | `p.panel_at(offset, \|inner\| ...)` | 嵌套容器（相对当前容器内容原点，不占光标） |

### 控件（容器内：`p.xxx(...)` 占光标；`*_at` 变体显式 `Rect`）

| 控件 | 签名 | 返回值 / 行为 |
|---|---|---|
| `label` | `p.label(text) -> Vec2` | 文本，内容自然尺寸 |
| `label_wrap` | `p.label_wrap(max_w, text) -> Vec2` | **自动换行标签**：`max_w`（逻辑像素）内按词/字换行；宽 = min(自然宽, max_w)，高 = 行数 × 行高；`max_w <= 0` = 不换行 |
| `min_size` | `p.min_size(w, h)` | **下一子项最小尺寸约束**（`0` = 该轴不约束；一次性，作用于紧接着的下一个子项） |
| `max_size` | `p.max_size(w, h)` | **下一子项最大尺寸约束**（同上） |
| `button` | `p.button(id, label) -> ButtonState` | hover / pressed / clicked（按下+释放均在本体） |
| `slider` | `p.slider(id, range, value) -> f32` | 拖拽；返回更新后的值（越界 clamp） |
| `checkbox` | `p.checkbox(id, label, checked) -> CheckboxState` | `.toggled()` 本帧切换；checked 由用户维护 |
| `radio` | `p.radio(id, group, label) -> CheckboxState` | 组内互斥（`UiState.radio_groups`）；`.checked()` 读选中 |
| `text_input` | `p.text_input(id, &mut String)` | 单行输入框：点击聚焦/定位光标、打字/退格/删除/方向键、Enter/Esc 失焦、光标闪烁；**超长文本滚动跟随光标**（光标始终可见）、**拖选文本 + Ctrl+C/V/X 复制/粘贴/剪切**（选择优先于窗口拖拽）；**支持中文 IME**（组合候选浮动提示框 + 候选框定位到光标） |
| `text_area` | `p.text_area(id, &mut String)` / `p.text_area_at(id, rect, &mut String)` | **多行文本输入框**：Enter 换行、↑/↓ 跨行（保持列）、Home/End 行首尾、按宽度自动换行、超出高度垂直滚动（滚轮 + 光标跟随）、跨行选择 + Ctrl+C/V/X、IME 支持；光标按逻辑行（`\n`）定位（超宽长行换行后近似） |

### 状态视图

| 类型 | 方法 | 说明 |
|---|---|---|
| `ButtonState` | `hovered()/pressed()/clicked()/released()` | 按钮状态（本帧点击 = 按下+释放均在本体） |
| `CheckboxState` | `checked()/toggled()/clicked()` | 勾选框 / 单选状态 |

### 样式（`Theme`，可 clone 覆盖）

`Theme { label, panel, button, slider, input, checkbox, divider, debug, focus, modal, combo, gap, row_h }`，子样式见 `crates/rjw_ui/src/style.rs`：
`LabelStyle`（font_size/color/align）、`PanelStyle`（bg/border/padding/**radius**）、`ButtonStyle`（三态 bg + padding + **radius**）、
`SliderStyle`（track/fill/handle）、`InputStyle`（bg/border_focus/caret/**sel_bg**/preedit/padding_x/height/min_w + **radius**）、
`CheckboxStyle`（box_size/checked_fill/gap）、`DividerStyle`、`DebugStyle`（layout_outline / layout_outline_width）、
`FocusStyle`（color / width，键盘导航焦点描边）、`ModalStyle`（dim / size）、
`ComboStyle`（下拉浮层现代菜单：menu_bg/border/radius/pad_v + item_hover/selected/pad_x/min_w + fg/fg_mark）。
`Theme::default()` 浅色，`Theme::dark()` 深色。

**子样式责任链**：每个子样式都有 `with_*` builder setter（返回 `Self`，只改链上字段，
其余回落默认）——`PanelStyle::default().with_radius(8.0)` / `ButtonStyle::default().
with_bg(c).with_radius(6.0)` / `SliderStyle::default().with_track(c).with_fill(c)` 等，
与 `Theme::with_*` 同风格；`font_family` setter 接受 `impl AsRef<str>`（可直接传 `&str` /
`&String` / `String`），内部统一存 `Arc<str>`（跨样式 / 跨命令共享，克隆零字符串复制）。
样式可整体替换进主题（`Theme::with_panel(..)`），也用于容器 builder 的逐容器覆盖
（`ui.window(..).style(panel_style)`）。

**DPI 预乘**：`Ui::begin(..).scale_factor(s).build()` 在 `build()` 时对最终 Theme 调用
[`Theme::scaled`](crate::style::Theme::scaled)（全尺寸 / 字号字段 × s 取整，颜色 / 字体族
不变）——与 `with_*` 调用顺序无关。之后 Ui 内部以物理像素为单位；公开坐标 / 尺寸参数
用 [`Size`](crate::draw::Size) / [`Position`](crate::draw::Position)（默认 Logical，
× scale 换算；`Size::Physical` 原样）。widget builder 的数值覆盖（`Label::font_size` /
`Button::radius/padding/font_size` / `Divider::thickness/margin` 等）同样接收 `Size`。

> **圆角半径**（`radius`，逻辑像素，默认 0 = 直角）：面板 / 窗口 / 按钮 / 输入框的
> 背景与边框走 **9-patch 圆角矩形**（程序化纹理进字形图集，颜色顶点色 tint）——
> 任意尺寸圆弧不畸变；`radius > 0` 时边框 ≈ 外圈 border 色圆角 + 内圈 bg 色圆角。
> 纹理生成按像素半区选圆心，**非整数物理半径（高 DPI：radius × scale）无缺口**；
> 绘制侧物理半径**取整**（`(radius × scale).round()`），9-patch 9 块边界恒落在
> 整数像素——高 DPI 下圆角填充不偏右下、无 1px 缝隙。

#### 调试样式（`DebugStyle`）

| 字段 | 类型 / 默认 | 说明 |
|---|---|---|
| `layout_outline` | `Color` = 青色 | `debug_layout` 布局描边颜色 |
| `layout_outline_width` | `f32` = 1.0 | `debug_layout` 描边宽度（**物理像素**） |

```rust
let mut theme = Theme::dark();
theme.debug.layout_outline = Color::MAGENTA;      // 改描边颜色
theme.debug.layout_outline_width = 2.0;           // 改描边宽度（物理像素）
// .theme(theme) 传入 Ui
```

> DebugDraw 图元（`ui.debug_*`）的样式 = **每次调用显式传参**（`color` + `width`，逻辑像素）；
> 需要统一样式时自建常量保存后传入。

#### 焦点样式（`FocusStyle`，键盘导航）

| 字段 | 类型 / 默认 | 说明 |
|---|---|---|
| `color` | `Color` = 青色（dark 主题下亮青） | 当前焦点控件的描边颜色 |
| `width` | `f32` = 1.0 | 焦点描边宽度（**逻辑像素**，内部 × scale 取整） |

### 键盘导航（焦点遍历）

交互控件（按钮 / 勾选 / 单选 / 滑块 / 输入框 / 下拉框 / 列表项按钮）每帧注册进
**焦点链**（[`rjw_ui::focus`]，按 `(win, 录制序)` 排序），`UiState.focused` 即当前焦点：

| 按键 | 行为 |
|---|---|
| `Tab` | 焦点链**下一个**（环绕） |
| `Shift + Tab` | 焦点链**上一个**（环绕） |
| `↑ / ↓` | 焦点链上 / 下一个（同 Shift+Tab / Tab） |
| `Enter / Space` | **激活**焦点控件：按钮点击、勾选/单选切换、下拉框展开/收起（输入框与滑块除外） |
| `← / →` | 焦点为**滑块**时调值（步进 = 范围 5%）；焦点为输入框时移动光标（原有） |
| `Esc` | 收起展开的下拉框；否则**取消焦点**（输入框内原有行为不变；应用快捷键需在 `capturing_text()` 为 false 时处理） |

- 焦点控件画一圈**描边**（`Theme::focus` / `FocusStyle`），裁剪沿用控件自身（滚动容器内正确）；
- 焦点控件本帧未录制（窗口关闭等）自动清除焦点；`Tab` 从链首重新开始；
- **IME 组合中禁用方向键/Tab 焦点移动**（上下键是输入法候选选择）；点击其他窗口置顶时自动清除旧窗口的输入框焦点。
- 示例 `eg260818UI`：Tab 遍历主菜单 → 背包 → 窗口 A/B → 列表 → 下拉框，全程键盘可操作。

### 文本输入增强（单行 / 多行 / IME / 剪贴板）

**单行 `text_input` 与多行 `text_area` 共用能力**（`rjw_ui::edit` 纯逻辑 + 单测）：

| 能力 | 说明 |
|---|---|
| **超长滚动跟随光标** | 文本超出内容区时左移（`WidgetState::text_scroll`），光标右侧保留 8 逻辑像素；TextArea 垂直滚动（`scroll_y`）跟随光标行 + 滚轮 |
| **文本选择** | 按住**拖选**（`WidgetState::sel_anchor`；选择优先于窗口/面板拖拽——从输入框拖拽 = 选择文本）；选择后打字 / 退格 / 删除 / 粘贴**替换选择**；**拖出框外仍跟随 + edge-scroll**（停在框外边缘自动滚动，"看不见的地方也选得到"）；**纯单击（无位移）释放清理 anchor** |
| **复制 / 粘贴 / 剪切 / 全选** | `Ctrl+C` / `Ctrl+V` / `Ctrl+X` / `Ctrl+A`（`arboard` 系统剪贴板；TextArea 跨行选择）；**单行粘贴过滤换行**（多行拼接成一行） |
| **Shift 选择** | `Shift + ←/→/↑/↓/Home/End` 扩展 / 收缩选择；**无 Shift 方向键移动 = 单选**（清除选择） |
| **IME 组合候选浮动提示框** | 组合串（preedit）画在输入框**下方浮动小框**（底色 + 边框 + 灰色文本，自动宽度），不再占行内；系统候选框 `set_ime_cursor_area` 跟随光标（含水平/垂直滚动） |
| **多行 TextArea** | `p.text_area(id, &mut String)` / `text_area_at(id, rect, ...)`：Enter 换行、↑/↓ 跨**视觉行**（保持列）、Home/End 行首尾、按内容区宽度自动换行（`create_buffer_wrap`）、**光标/点击/选择按视觉行定位与显示一致**（`Text::visual_lines`）、行距 1.2、跨视觉行选择高亮逐行绘制 |

- 输入框按下时置位 `press_claimed`：窗口/面板**不建立拖拽基准**（选择拖拽优先；窗口从空白/标题区拖动），并清除旧拖拽基准（防"瞬移"）；
- 主题：`InputStyle::sel_bg`（选择高亮色，默认浅蓝 / dark 深蓝）。

### 渲染增强（圆角 / 渐变，程序化纹理进动态 Atlas）

| 函数 | 签名 | 说明 |
|---|---|---|
| `rounded_rect_at` | `ui.rounded_rect_at(pos, size, radius, color)` | 圆角矩形背景原语（radius 逻辑像素；9-patch 绘制，颜色顶点色 tint） |
| `gradient_rect_at` | `ui.gradient_rect_at(pos, size, axis, stops)` | 线性渐变矩形原语（`axis`：`GradientAxis::Vertical/Horizontal`；`stops: Vec<(f32, Color)>`） |
| `GradientAxis` | `Vertical` / `Horizontal` | 渐变方向（`Vertical` 沿 y：0 = 顶部） |

- 程序化纹理（圆角矩形 `32×32`、渐变主轴 `64` 级、WHITE `1×1`）**塞进动态 Atlas**
  （`rjw_ui::ProcTextures` → `UiState` 持有，惰性创建、`insert_permanent` 永久保留、
  `clamp_margin` 防采样透色），页纹理自动注册进 `rjw_render::TEXTURES`；
- 圆角纹理只存**白色 + alpha**（同半径一张，颜色由顶点色 tint，不随颜色膨胀图集）；
- 圆角**9-patch**：四角原样、四边/中心拉伸（任意矩形尺寸圆弧不畸变）；
  渐变矩形直接拉伸采样（主轴 64 级已平滑）；
- 提交分组升级为 `(win, 图形/文字组, 纹理 uid)`：圆角 / 渐变属于**图形组**，先于文字
  （不会因非白纹理 uid 排序错位盖住文字）；⚠ **UI 的 Render2D 必须 `set_sorting(false)`**
  （完全按提交顺序绘制）——`set_sorting(true)`（`LayerAndStates`）会按纹理 uid 重排，
  圆角/渐变会被排在文字之后绘制而盖住文字（示例 `eg260818UI` 即如此配置）；
- 控件级集成：`Theme` 的 `PanelStyle::radius` / `ButtonStyle::radius` / `InputStyle::radius`。

### 调试（Debug UI / DebugDraw / 窗口诊断）

| 函数 | 签名 | 说明 |
|---|---|---|
| `debug_layout` | `ui.debug_layout(true)` / `.debug_layout(true)`（构建期） | 每个控件/容器矩形画描边（布局 + 命中区域可视化；样式见 `DebugStyle`；开启时跳过窗口顶点缓存） |
| `debug_line` | `ui.debug_line(a, b, width, color)` | 屏幕空间线段（**绝对逻辑像素**，覆盖在 UI 内容之上） |
| `debug_rect_outline` | `ui.debug_rect_outline(&Rect, width, color)` | 屏幕空间矩形框 |
| `debug_circle_outline` | `ui.debug_circle_outline(center, r, segments, width, color)` | 屏幕空间圆环 |
| `debug_cross` | `ui.debug_cross(center, half, width, color)` | 屏幕空间十字标记 |
| `debug_grid` | `ui.debug_grid(&Rect, spacing, width, color)` | 屏幕空间网格（每方向 ≤ 512 条） |
| `window_order` | `ui.window_order() -> Vec<(String, u32)>` | 诊断：窗口 z 序（按 z 升序） |
| `window_under_mouse` | `ui.window_under_mouse() -> Option<(String, u32)>` | 诊断：鼠标下**最上层**窗口（重叠点击时唯一可交互的窗口） |
| `UiState::last_press_window` | `state.last_press_window() -> Option<(&str, u32)>` | 诊断：上次按下由哪个窗口接收（重叠点击"赢家"） |
| `UiState::occluded_hits` | `state.occluded_hits() -> u32` | 诊断：上帧**命中但被更高窗口遮挡而未响应**的控件次数（点击穿透拦截计数） |

> 世界坐标调试图元（游戏场景：碰撞盒 / 网格 / 速度矢量）见 `rjw_2d_render::debug_draw`
> （`draw_line` / `draw_rect_outline` / `draw_circle_outline` / `draw_circle_filled` /
> `draw_cross` / `draw_grid`）。示例：`examples/egDebugDraw`（rjw_ui 屏幕空间 + 世界空间 + debug_layout）、
> `examples/eg260818UI`（右上角窗口诊断面板）。
>
> 窗口遮挡（点击穿透）已修复：重叠区域**只有鼠标下最上层窗口**的控件响应——`window_occluded`
> 判定（`hit.rs`），窗口矩形跨帧缓存于 `UiState.window_rects`；`occluded_hits > 0` 即证明
> 背后控件被正确抑制。

### 快速上手

```rust
use rjw_ui::{IdAbsolute, PackSide, Theme, Ui, UiState};

let mut state = UiState::new();
state
    .radio_groups
    .insert("diff".into(), IdAbsolute::from("diff_normal")); // 默认选中

// 每帧（window/font 来自主循环；输入设备经 capture 快照；相机/渲染器延迟到 finish）：
let mut ui = Ui::begin(window, &mut font, &mut state)
    .capture(&ctx.mouse, &ctx.keyboard)
    .theme(Theme::dark()).build();

ui.pack_at(Vec2::new(16.0, 16.0), PackSide::Top, |p| {
    if p.button("start", "开始游戏").clicked() { /* ... */ }
    volume = p.slider("vol", 0.0..=1.0, volume);
    if p.checkbox("fs", "全屏", fs).toggled() { fs = !fs; }
    p.text_input("name", &mut name);
});
ui.finish(&viewport, r2d);
```

> 约定：交互控件 ID 必须稳定；顶层 pack 控件（`label`/`button`/…）经**根容器**（`build()`
> 内建，可用宽 = 视口宽）直接流式堆叠，绝对定位用 `*_at`；控件坐标 = 屏幕**逻辑**像素
> （`.scale_factor` 设置 DPI，不设置则等于物理像素）；文本输入支持中文 IME
> （`rjw_keyboard::get_ime_commits` / `get_ime_preedit`，候选框跟随光标）；输入框聚焦时用
> `UiState::capturing_text()` 屏蔽应用快捷键；
> 控件文本排版缓冲自持于 `UiState.text_buffers`（`CachePolicy::User`，不推入 `rjw_text` LRU）；
> **独立 UI 渲染**：UI 录到单独 Render2D（`Render2D::set_sorting(false)` 关闭排序），与世界
> `render_command_buffer` 合并提交（一次 present）；`finish` 按 `(win, depth, 图形/文字, 录制序)`
> 免排序（win + depth 分桶）提交。

---

*还想看更多？源码在 `crates/rjw_*/src/`，目录与本文一一对应。*
