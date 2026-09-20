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

- [0. 统一入口（rjw_krusie）](#0-统一入口rjw_krusie)
- [1. Color / ColorF64（颜色）](#1-color--colorf64颜色)
- [2. Transform2D（变换）](#2-transform2d变换)
- [3. Camera2D（相机）](#3-camera2d相机)
- [4. SpriteRect（精灵矩形）](#4-spriterect精灵矩形)
- [5. Render2D（2D 批渲染器）](#5-render2d2d-批渲染器)
  - [5.4.2 静态网格 StaticMesh](#542-静态网格-staticmesh)
- [6. RStates 渲染状态与 Builder 责任链](#6-rstates-渲染状态与-builder-责任链)
- [7. Clear（清屏意图）](#7-clear清屏意图)
- [8. DynamicAtlas（纹理图集）](#8-dynamicatlas纹理图集)
- [9. Text（文本渲染）](#9-text文本渲染)
- [10. 其他常用小类型速查](#10-其他常用小类型速查)
- [11. UI（rjw_ui）](#11-uirjw_ui)
  - [11.1 绘制器（Painter / DrawQueue）](#绘制器painter--drawqueuev03-新增)

---

## 0. 统一入口（`rjw_krusie`）

crate：`rjw_krusie`（**聚合 + 模块化运行时**）——整套库一行起步；低层与命名冲突走命名空间。
完整的**契约**（分层 / 13 条规则 / 责任表 / 简并表 / 旧→新映射 / 实现进度）见
[API_DESIGN.md](API_DESIGN.md)。

```rust
use rjw_krusie::prelude::*;

#[derive(Default)]
struct Game;

impl App for Game {
    fn config(&self) -> AppConfig { AppConfig::new("my game").size(1280.0, 720.0) }
    fn update(&mut self, ctx: &mut Ctx) {
        // 逻辑：无帧也执行（后台模拟 / 计时 / 输入状态持续）
        if ctx.key(KeyCode::Escape).down_edge() { ctx.exit(); }

        // 渲染：守卫在应用里 —— 取不到表面时渲染代码一行不执行
        let Some(mut f) = ctx.frame() else { return };
        f.draw().solid(SpriteRect::new((-50.0, -50.0), (100.0, 100.0)))
            .color(Color::GREEN);
        f.submit(&mut Camera2D::full(f.region().size()), Clear::color(Color::rgb(0.1, 0.1, 0.2)));
    }
}

fn main() -> Result<(), EventLoopError> { run(Game) }
```

```rust
// 按需补充（低层类型 / 自由函数 / 冲突名 / 逃生口）
use rjw_krusie::atlas::RegionRef;
use rjw_krusie::collision::Aabb;
use rjw_krusie::render2d::{CustomDraw, Draw2D, VertexP3U2C4};
use rjw_krusie::gpu::{RenderContext, TEXTURES, MESHES};
```

| 层 | 内容 |
|---|---|
| `rjw_krusie::prelude` | 运行时（`App` `run` `run_with` `AppConfig` `Background` `Ctx` `Frame` `Gfx` `WindowId` `Clear` `Vsync`）；绘制（`Render2D` `SpriteRect` `Edges` `Layer` `SortMode` `Cull` `RStates` + 状态枚举与描述符 `DepthState`/`StencilState`/`SamplerDesc`/`RasterState`）；资源（`ArcTextureWrapped` `Rgba8` `MeshSpec` `MeshId`）；数学/相机（`Camera2D` `Transform2D` `Rect` `Vec2` `Mat4` `glam` + 引擎 dpi 类型）；颜色；输入（`KeyCode` `MouseButton` `KeyState` `ScrollDelta`）；图集；文本（`Text` `TextStyle` `TextBuffer` `Align` …）；UI（`Ui` `UiState` `Theme` + 常用控件）；瓦片 |
| 命名空间 | `runtime`（`app` 别名）`main` `gpu` `render2d` `transform` `color` `atlas` `text` `ui` `tilemap` `collision`（同时保留原 crate 名，如 `rjw_krusie::rjw_ui::Ui`） |

**prelude 零冲突**：winit 类型（`WindowAttributes` / `LogicalSize` / …）与低层机制
（`RenderContext` / `RenderFrame` / `PassBuilder` / `Draw2D` / `SortKey` / `VertexP3U2C4` /
`TEXTURES` / `MESHES`）都在命名空间里，不进 prelude；引擎自有的 dpi 类型（`LogicalSize` 等）
在 prelude。**不做 `as` 改名**。

**裁剪**：`default-features = false` 可只保留运行时 + 2D 绘制（不装 `atlas` / `text` / `ui` / `tilemap` / `collision`）。

> 各 crate 仍可**单独使用**（`use rjw_2d_render::Render2D;` …）——`rjw_krusie` 只是聚合 + 运行时，不改变底层 crate 的 API。

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
| `.into()` | `let c: wgpu::Color = ColorF64::rgba(...).into();` | 直接转换给 `Clear::Color(..)`（运行时内部使用） |

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
| `with_pos` | `.with_pos((x, y))` | 设置位置 |
| `with_scale` | `.with_scale((sx, sy))` | 设置缩放 |
| `with_rot` | `.with_rot(0.5)` | 设置旋转（弧度） |

> 旧的 `with_move_by` / `with_walk_by` / `with_scale_by` / `with_rotate_by` 已删除（`with_pos(pos + d)` 已足够）。
> 原地移动（`&mut self`）用 `move_by(delta)` / `move_local(delta)`。

#### 空间运算

| 函数 | 说明 |
|---|---|
| `transform_point(local)` | 局部点 → 父/世界点 |
| `inverse_transform_point(world)` | 世界点 → 局部点（命中检测用） |
| `transform_vec(local_vec)` / `inverse_transform_vec(world_vec)` | 方向向量正 / 逆变换 |
| `compose(&parent)` | 组合父级：`parent * self` |
| `compose_inverse(&parent)` | 组合父级之逆 |
| `to_matrix()` | 列主序模型矩阵（`m * vec4(local, 0, 1) == transform_point(local)`；GPU / 剔除 / 相机统一出口） |
| `inverse()` | 逆变换**对象**（⚠ 非均匀缩放 + 旋转下与 `inverse_transform_point` 的点级精确逆不同） |

> 💡 **旋转中心 = pos**：让精灵绕自身中心转，矩形写成 `SpriteRect::new((-w / 2.0, -w / 2.0), (w, w))`。

---

## 3. Camera2D（相机）

crate：`rjw_transform`

```rust
pub struct Camera2D {
    pub region:    Rect,            // 画面矩形（屏幕像素，左上原点）
    pub transform: Transform2D,     // 相机在世界中的位姿：pos / rotation / scale
}
// Camera2D: Deref/DerefMut<Target = Transform2D> ⇒ `cam.move_by(..)` 等位姿方法直接用
// transform.scale = 世界单位/像素；`zoom()`（越大越放大）是派生视图 = 1 / scale
```

#### 构造 / 画面

| 函数 | 用法 | 说明 |
|---|---|---|
| `Camera2D::new` | `Camera2D::new(Rect::new(0.0, 0.0, w, h))` | 以画面矩形建相机（位姿 = `IDENTITY`） |
| `Camera2D::full` | `Camera2D::full((w, h))` | 全窗口画面（等价 `new(Rect::new(0, 0, w, h))`） |
| `set_region` / `region()` | `cam.set_region(f.region())` | 画面矩形读 / 写（`submit` 会写回相机的 `region`） |

> 高 DPI 下用物理像素（`f.region()` / `Gfx::size()` 已经是物理像素）。

#### 移动（经 `Deref` 来自 `Transform2D`）

| 函数 | 用法 | 说明 |
|---|---|---|
| `move_by` | `cam.move_by((dx, dy))` | 世界坐标平移（不随旋转） |
| `move_local` | `cam.move_local((lx, ly))` | 沿相机自身方向移动 |

#### 矩阵 / 坐标转换

| 函数 | 说明 |
|---|---|
| `vp_matrix()` | 列主序 VP（P×V）：由 `Frame::submit(&mut cam, clear)` / `Render2D::submit(&mut pass, &cam)` 自动取用（无 `set_mvp`） |
| `view_matrix()` / `projection_matrix()` | 世界 → 画面居中像素 / 画面居中像素 → NDC（含 Y 翻转） |
| `screen_to_world(screen_px)` | 窗口像素 → 世界 |
| `world_to_screen(world)` | 世界 → 窗口像素（与上者互为精确逆） |
| `world_to_region_local(world)` | 世界 → **画面居中像素**（不含 `region` 偏移；屏幕固定绘制用） |
| `view_half_size()` / `view_aabb()` | 可见半宽高（世界单位）/ 世界视口保守 AABB（含旋转，剔除不误杀） |
| `zoom()` / `set_zoom(zoom)` | 派生缩放视图（越大越放大）；写入 `transform.scale = 1/zoom` |

```rust
// 鼠标指向的世界点
let world = cam.screen_to_world(f.mouse().pos_px());
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
| `new` | `SpriteRect::new(tl, wh)` | ★**整张纹理**铺满（最常用） |
| `centered` | `SpriteRect::centered(center, wh)` | 以中心点 + 尺寸 |
| `with_uv` | `SpriteRect::with_uv(tl, wh, uv_tl, uv_wh)` | 归一化 UV 全指定 |
| `with_uv_px` | `SpriteRect::with_uv_px(tl, wh, uv_tl_px, uv_wh_px, tex_wh)` | 像素 UV 子区（**给纹理尺寸，无需算倒数**） |
| `with_uv_tex` | `SpriteRect::with_uv_tex(tl, wh, uv_tl_px, uv_wh_px, &tex)` | 像素 UV 子区（尺寸自动取自纹理） |
| `at` / `move_by` | `r.at(pos)` / `r.move_by(delta)` | 移动（保持尺寸与 UV） |
| `size` | `r.size(wh)` | 改世界尺寸 |
| `uv` / `uv_px` | `r.uv(tl, wh)` / `r.uv_px(tl_px, wh_px, tex_wh)` | 改 UV（归一化 / 像素） |
| `shrink` / `shrink_uv` | `r.shrink(4.0)` / `r.shrink_uv((0.02, 0.0))` | 世界矩形 / 归一化 UV **各边**收窄（`f32` 四边同值 / `(x, y)` 分轴 / `Edges` 逐边；负值即外扩） |

> 位置 / 尺寸 / UV 参数接受 `Vec2` 或 `(x, y)`（正方形写 `(x, x)`）。
> **v0.2.0 起**：`new` 即"整张纹理"；需要子区用 `with_uv*`（旧 `from_texture` / `from_texture_px` 已删除）。

```rust
use rjw_2d_render::SpriteRect;
use glam::Vec2;

let a = SpriteRect::new(Vec2::ZERO, (32.0, 32.0));                  // 整张纹理
let b = SpriteRect::with_uv_tex(Vec2::ZERO, (32.0, 32.0), (8, 8), (16, 16), &tex);
let c = SpriteRect::centered(pos, (48.0, 48.0));                     // 以 pos 为中心
let d = a.shrink(4.0).at((100.0, 50.0));                             // 链式：四周各收 4px 后移动
```

### 4.1 像素 UV 子区与裁剪量 `Edges`

crate：`rjw_2d_render`（`data` 模块）

> **v0.3.0 起**：`SpriteRectPx` 已删除（[`API_DESIGN.md`](API_DESIGN.md) §5「精灵矩形」简并）。
> "世界矩形 + 像素 UV"由 `SpriteRect::with_uv_px` / `with_uv_tex` 直接表达，内部归一化，
> **不需要**自己算 `1/尺寸`；图集精灵直接用 `AtlasSprite`。

```rust
use rjw_2d_render::{Edges, SpriteRect};

// 整张贴图（尺寸显式给出）
let base = SpriteRect::with_uv_px(Vec2::ZERO, (64.0, 64.0), Vec2::ZERO, (64.0, 64.0), tex_wh);
// 子区 (8,8)-(24,24)，纹理尺寸取自纹理
let sub = SpriteRect::with_uv_tex(Vec2::ZERO, (64.0, 64.0), (8, 8), (16, 16), &tex);
// 已有矩形改像素 UV
let other = SpriteRect::new(pos, (32.0, 32.0)).uv_px((8, 8), (16, 16), tex_wh);
```

**裁剪量 `Edges`**（每边各自的量，不是总量）：

| 构造 | 说明 |
|---|---|
| `Edges::all(v)` / `Edges::xy(x, y)` / `Edges::lrtb(l, r, t, b)` | 四边同值 / 左右上下 / 逐边 |
| `Edges::new().left(8.0)` | 链式只改某一边 |
| 直接传 `f32` / `(x, y)` / `Vec2` / `[f32; 2]` | 等价 `all(v)` / `xy(x, y)` |

| 调整 | 用法 | 说明 |
|---|---|---|
| `shrink` | `rect.shrink(4.0)` | **世界矩形**各边收窄（UV 不动；负值即外扩） |
| `shrink_uv` | `rect.shrink_uv(Edges::xy(0.125, 0.25))` | **归一化 UV** 各边收窄（参数为 `0..1` 比例；负值即外扩） |

> 收窄不 clamp：过窄 / 越界由调用方负责（引擎希望"所见即所写"，避免隐藏的边界修正）。

---

## 5. Render2D（2D 批渲染器）

crate：`rjw_2d_render`（**v0.2.0 起 API 重新设计**：统一 Builder 责任链 + 唯一状态入口 + 排序/剔除分离）

> 生命周期：`Render2D::new(&RenderContext)` 持有 surface 的 `'static` 引用，要求 `RenderContext` 比 `Render2D` 更久。
>
> **设计原则**：入口**少参数**（≤ 2）；"在哪层 / 什么色 / 什么变换 / 什么状态"全部走链式；
> 排序与剔除是**对命令索引数组的操作**，已独立到 `rjw_2d_render::sort` / `cull`（纯函数、可单测、可复用）。

### 5.1 创建 / 相机 / 全局

| 函数 | 用法 | 说明 |
|---|---|---|
| `new` | `Render2D::new(&render_ctx)` | 基于 `RenderContext` 创建 |
| `set_mvp` / `set_camera` | **已删除**：相机由调用方持有，`submit(&mut pass, &cam)` / `Frame::submit(&mut cam, clear)` 自动取 VP 与画面矩形 |
| `mvp()` | `r2d.mvp()` | 当前 VP |
| `texture_layout()` | `r2d.texture_layout()` | 纹理 bind group layout（`rjw_text` / `rjw_ui` 自建 bind group 用） |
| `device()` / `queue()` | `r2d.device()` / `r2d.queue()` | 暴露底层 wgpu（高级用法） |

### 5.2 排序（`rjw_2d_render::sort`）

| 函数 | 说明 |
|---|---|
| `sort(SortMode)` | `LayerAndStates`（默认，按 `(layer, states)` 排序合批）/ `LayerOnly`（仅按 layer 稳定排序，同层保录制序，UI 适用）/ `None`（完全按录制顺序） |
| `sort_custom(Box<dyn SortPolicy>)` | 注入**自定义排序策略**（`fn sort(&self, order: &mut [usize], keys: &[SortKey])`） |
| `sort_mode() -> SortMode` | 当前内置模式 |

- `SortKey { layer, rstates, texture_uid }`：与**命令下标**对齐的排序键（`keys[order[i]]` 才是第 `i` 条命令的键）。
- `SortMode::apply(order, keys)` / `SortPolicy::sort(..)`：纯函数，只重排索引数组，命令数据不动。

### 5.3 剔除（`rjw_2d_render::cull`）

| 函数 | 说明 |
|---|---|
| `cull(impl Into<Cull>)` | **单一入口**：`Cull::Off`（默认）/ `Cull::Viewport` / `Cull::Rect(rect)` / `Cull::Fn(Box<dyn Fn(&Rect) -> bool>)` / **`Cull::from(&cam)`**（以相机剔除，取代旧 `set_cull_camera`） |
| `cull_mode()` / `culler()` / `culler_mut()` | 当前模式 / 剔除器（下游可复用同一套可见性判定） |

- `Culler::new(Cull::...)`、`visible(&Rect) -> bool`、`retain(&mut Vec<usize>, aabb_of)`（就地过滤索引数组）。
- 纯几何：`sprite_world_aabb(&SpriteRect, &Mat4)`、`viewport_world_rect(&Mat4)`、`transform2d_model(&Transform2D)`。
- **行为**：只有 Sprite 命令参与剔除（动态 Mesh / StaticMesh / Custom 恒保留）。

### 5.4 全局默认渲染状态（**唯一状态入口**）

| 函数 | 说明 |
|---|---|
| `states_mut(RStates)` | 设置全局默认状态（未链式设置状态的命令继承它） |
| `states() -> RStates` | 当前全局默认状态（只读） |
| `reset()` | 重置画面矩形 / scissor / 全局默认状态 / 剔除（一体复位） |

```rust
use rjw_2d_render::{AddressMode, BlendMode, CompareFunc, RStates};
r2d.states_mut(
    RStates::new()
        .blend(BlendMode::Additive)
        .depth_test(true)
        .depth_write(true)
        .depth_compare(CompareFunc::Less)
        .samp_addr_all(AddressMode::Repeat),
);
```

> 旧版的 21 个 `default_*` 方法已删除——`RStates` 本身就是链式状态语言，只需一个入口。

### 5.5 录制入口（画什么）：数据版 / 流式版

| 函数 | 用法 | 说明 |
|---|---|---|
| `sprite(rect, &tex)` | `r2d.sprite(rect, &tex).tint(c).transform(tf).layer(l)` | 贴纹理精灵（实例化合批） |
| `solid(rect)` | `r2d.solid(rect).tint(c).layer(l)` | 纯色精灵（内部 1×1 白纹理） |
| `region(AtlasSprite)` | `r2d.region(spr).layer(l)` | ★ 图集直达（`atlas.sprite(&handle)` 的产物：区域 + 页纹理一次拿到） |
| `mesh(&verts, &tris)` | `r2d.mesh(&verts, &tris).tint(c).transform(tf)` | 显式顶点 + u16 索引（世界坐标） |
| `mesh_with(\|sink\| ..)` | `r2d.mesh_with(\|s\| { s.push_tri(a,b,c); }).tint(c)` | 流式构造（自定三角化 / 逐顶点 UV） |
| `polygon(&verts)` | `r2d.polygon(&verts).tint(c).layer(l)` | 多边形（**fan 三角化**：首顶点为中心） |
| `polygon_with(\|p\| ..)` | `r2d.polygon_with(\|p\| { p.vertex(a); p.vertex_uv(b, uv); .. })` | 流式多边形（自动 fan 三角化；带 UV 用它） |
| `quads(&verts, &tex)` | `r2d.quads(&verts, &tex).transform(tf).tint(tint)` | 四边形段（顶点 TL,TR,BL,BR；`.tint()` 为**整段实例色**） |
| `quads_with(\|q\| .., &tex)` | `r2d.quads_with(\|q\| { q.quad(tl,tr,bl,br); }, &tex)` | 流式四边形段 |
| `static_mesh(MeshId, &tex)` | `r2d.static_mesh(id, &tex).tint(c).transform(tf).layer(l)` | 静态网格实例（句柄来自 `Gfx::mesh`；`MESHES` 注册表 + 实例化合批） |
| `custom(cd)` | `r2d.custom(\|pass\| { .. }).layer(l)` | 注入原生 wgpu 绘制（`CustomDraw` / 闭包 blanket impl） |

> **入口命名规则**：`kind(数据…)` = 已有数据直接给；`kind_with(|sink| …)` = 流式构造（零临时 `Vec`）。
> 旧版 16 个 `add_*` 与 `mesh_with_cap` / `polygon_uv` 已删除；
> 需要预留容量请在闭包外自行 `Vec::with_capacity`（`mesh(&verts, &tris)` 直接给已构造好的数据），
> 带 UV 的多边形用 `polygon_with` 的 `vertex_uv(..)`。

### 5.6 责任链修饰（4 个 kind 共用同一份实现）

| 方法 | 默认 | 说明 |
|---|---|---|
| `.layer(impl Into<Layer>)` | `0.0` | 绘制层级（数值小先绘制；接受 `f32` / `i32` / `u32` / `f64`） |
| `.tint(Color)` | `WHITE` | Sprite/StaticMesh：实例色；mesh/polygon：**逐顶点色**；quads：**整段实例色** |
| `.transform(Transform2D)` | `IDENTITY` | 局部 → 世界 |
| `.at(p)` / `.rot(r)` / `.scale(s)` | — | `transform` 便捷糖 |
| `.matrix(Mat4)` | — | 直接给列主序模型矩阵（覆盖 transform） |
| `.texture(&tex)` | 入口参数 | 覆盖采样纹理（Mesh 系默认白纹理） |
| `.states(RStates)` | `None` = 继承全局 | 完整渲染状态（**唯一权威**，覆盖此前的糖） |
| `.blend(BlendMode)` / `.sampler(SamplerDesc)` / `.cull(CullMode)` / `.raster(RasterState)` / `.depth(impl Into<DepthState>)` / `.stencil(impl Into<StencilState>)` | — | `states` 便捷糖；**收对象/枚举**，不收裸 bool（`DepthState::{test_write, test_only, off}`） |

```rust
// 世界坐标旋转精灵 + 加性混合
r2d.sprite(rect, &tex)
    .tint(Color::WHITE)
    .transform(Transform2D::IDENTITY.with_rot(t))
    .layer(y_layer(foot_y))
    .blend(BlendMode::Additive);

// 流式画圆（零临时 Vec）
r2d.polygon_with(|p| {
    p.vertex(center);
    for i in 0..=22 {
        let a = i as f32 / 22.0 * std::f32::consts::TAU;
        p.vertex(center + Vec2::new(a.cos(), a.sin()) * r);
    }
})
.color(Color::CYAN)
.layer(96.0);
```

> `Draw2D<'a, K>` 是本工程**唯一的绘制 Builder**（`Sprite` / `Mesh` / `StaticMesh` / `Custom` 四个 kind）；
> 类型别名 `SpriteBuilder` / `MeshBuilder` / `StaticMeshBuilder` / `CustomBuilder` 供签名标注。

#### 5.6.1 Scissor（命令级裁剪，v0.3 新增）

| 函数 | 说明 |
|---|---|
| `Draw2D::scissor(rect: Rect)` | **命令级 scissor**（屏幕/目标像素、左上原点）。最终 = 命令级 **∩** 画面级 `Render2D::scissor` **∩** 目标矩形；**空矩形 ⇒ 该命令整条跳过**（0 draw，不是"不裁剪"）；不同 scissor 的命令**不合批**（一次 `draw_indexed` 只能一个 scissor） |
| `Draw2D::scissor_opt(Option<Rect>)` | 同上，`None` = 不设（继承画面级） |
| `Render2D::scissor(Option<Rect>)` | **画面级** scissor（整画面；浮点、按目标钳制；见 §5.1） |
| `Render2D::draw_op_count() -> usize` | 上一帧 `prepare()` 后的 draw op 数（= draw call 数），诊断"scissor 让 draw 变多" |

> `scissor` 走**两级**：画面级（`Render2D`，作用于整个 `pass`）与命令级（`Draw2D`，逐条命令）。
> 两者的矩形都在绘制时按像素取整并钳到目标；任一为空/全在目标外则该 op 不发 draw。
> UI 用它承载环境裁剪（[`UiBatch::clip`]），从而**不再切割几何**——圆角、环边、投影
> 保持原形。见 `docs/ENGINE_GUIDE.md` §18.20。

### 5.7 静态网格 StaticMesh

| 函数 | 说明 |
|---|---|
| `Gfx::mesh(MeshSpec { .. }) -> MeshId` | 建**静态网格**（`&Gpu` 工厂：`gfx.mesh(..)` 或 `gpu.mesh(..)`），返回可复用句柄 |
| `static_mesh(mesh_id, &tex).tint(..).transform(tf).layer(..)` | 提交一个实例（CPU 侧只有变换 + 颜色） |
| `static_mesh(id, &tex).matrix(mat)` | 直接给模型矩阵（跳过 `Transform2D` 推导） |

- **合批条件**：`(mesh_id, rstates, tex_uid)` 相同且绘制序列连续 → 合并为同一次 `draw_indexed`。
- **适用**：固定层级、不参与 y-sort 的地图元素（石头 / 花 / 栅栏）；**会插入实体排序的（如 y-sort 的树）必须保持动态**。
- `MeshSpec` 见 [`MeshSpec`]（`label` / `vertices` / `indices`）；低层自建用 `MeshData::from_pod(device, &verts, &indices, label)` / `from_buffers(vb, ib, index_count)`。

### 5.8 提交（**提交即清帧**）

| 函数 | 说明 |
|---|---|
| `submit(&mut PassBuilder, &Camera2D)` | ★ 一个画面 = 一个 pass = 一个 VP 槽：写回 `cam.region` → 取 `cam.vp_matrix()` → 开 pass → 提交队列 → 清帧 |
| `Render2D::render(&mut RenderContext, Clear)` | 单画面一行糖：`acquire_frame` → `submit`（用当前相机）→ `present` |
| `discard()` | 丢弃未提交的录制（无帧帧 / 主动放弃本帧） |
| `RenderFrame::pass(clear)` / `pass_to(target, clear)` | 低层：自己开 pass（多次 `record` 共用一次 Load/Store；离屏走 `pass_to`） |
| `PassBuilder::record(&mut R)` | 把某个 `PassRecorder`（如 `Render2D`）录进本 pass；**可多次** |
| `RenderFrame::present()` | 提交 encoder + present（`#[must_use]`：忘记会记 warning） |
| `RenderContext::acquire_frame()` | 取当前表面帧（`None` = 取帧失败，跳过本帧；`FrameSource` 是同一入口的抽象） |

> 应用层通常**不直接碰这些**：`rjw_krusie::runtime` 的 `Frame::{submit, present}` 已封装
> 「取帧 → 相机写回 → 开 pass → 提交 → present」；无帧时 `Ctx::frame()` 返回 `None`。

### 5.9 每帧内部流水线

```
apply_order()：take_order() → fill_sort_keys() → SortPolicy::sort() → Culler::retain() → set_order()
prepare()    ：RStates resolve（None → 全局默认）→ 生成实例 / 动态段 → 按 (layer, mesh_id, rstates, tex) 分组
               → 按 MAX_INSTANCES_PER_DRAW(8192) 分页 → 上传实例页 / 动态顶点缓冲
draw()       ：按 DrawOp.rstates 取/建管线 → 绑定纹理 bind group → 逐页 draw_indexed
```

- 实例缓冲是**页池**：单帧实例数可远超 8192，自动分页；**不要自己裁减数量去凑**。
- 命令 / 网格 / 排序键缓冲全部**常驻复用**（`clear()` 只清长度不释放）；Builder 为栈上 struct，无堆分配。

### 5.10 纹理与采样器

- 创建：`gpu.texture(label, Rgba8::new(&rgba, (w, h))) -> ArcTextureWrapped`（RGBA8，`len == w*h*4` 否则 panic）；
  注册表低层用 `gpu.textures().register(arc)`。
- `TextureWrapped`（`rjw_render`）**只持有纹理本身**；采样器完全由 `RStates` 位域（bits 8..24）驱动
  （`.sampler(FilterMode::Nearest, AddressMode::Repeat)` 或 `.states(..)`），`Render2D` 内部按需创建并缓存 `wgpu::Sampler`。
- bind group 按 `(tex_uid, samp_key)` 缓存，value 持有 `Arc<Texture>` 防悬挂；`prepare` 末尾自动剔除失效条目。
- 1×1 白纹理：`Gpu::white_texture()` / `Render2D` 内部的 `white_texture`（纯色绘制与 `solid` 使用）。
- **纹理注册表是每 `RenderContext` 私有的**（不再是全局 `static`）：`gpu.textures()`（`&TextureRegistry`）/
  `r2d.textures()`（绘制期）；需要长期持有用 `gpu.texture_registry()`（`&Arc<..>`）。
  操作：`register` / `register_named` / `get` / `remove` / `rename` / `contains_uid` / `contains_name`。
  > v0.3 起 `rjw_render::TEXTURES` / `MESHES` 两个全局 `static` **已删除**。旧全局单例下
  > `Render2D::new` 每次注册的 1×1 白纹理与四边形网格会**覆盖**先建渲染器的条目
  > （同一 uid 指向别人的纹理）；实例化后跨 `RenderContext` 彻底隔离。
  > uid 仍由进程级计数器保证全局单调不复用，但**uid 相等不再蕴含「同一张纹理」**。

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

### 6.2 Builder 链方法（`SpriteBuilder` / `MeshBuilder` / `StaticMeshBuilder` / `CustomBuilder` 通用）

| 分类 | 方法 |
|---|---|
| 通用修饰 | `.layer(impl Into<Layer>)` / `.tint(Color)` / `.transform(tf)` / `.at(p)` / `.rot(r)` / `.scale(s)` / `.matrix(mat)` |
| 纹理 | `.texture(&tex)`（Mesh 系默认白纹理；Sprite/StaticMesh 由入口参数给出） |
| 状态（全量） | `.states(RStates)`（唯一权威；后写覆盖前写） |
| 状态（对象糖） | `.blend(BlendMode)` / `.samp(FilterMode, AddressMode)` / `.cull(CullMode)` / `.depth(impl Into<DepthState>)` / `.stencil(impl Into<StencilState>)` |
| 状态（描述符） | `.blend_state(BlendDesc)` / `.samp_state(SamplerDesc)` / `.raster_state(RasterState)` |
| 提交 | `.done()`（显式提交；亦可依赖 Drop 自动 push） |

> 💡 需要 `RStates` 的完整表达能力（polygon / front_face / conservative_raster 等）时，用
> `.states(RStates::new().polygon(PolygonMode::Line))`——**状态只有一个语言：`RStates`**。

不链式调用 = 继承全局默认（`Render2D::states()`）；链式任一项 = 该命令显式状态。

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
| `StaticMeshBuilder<'a>` | `static_mesh` 返回（Drop 即提交，或 `.done()` 显式） |
| `HasUid` | trait：`fn uid(&self) -> u64`（`rjw_render`） |
| `TypedRegistry<T: HasUid>` | 泛型注册表：`register` / `register_named` / `get` / `get_ref` / `remove` / `remove_name_mapping` / `rename` / `contains_uid` / `contains_name` |
| `MeshRegistry` | 静态网格注册表（`TypedRegistry<MeshData>`）；**每 `RenderContext` 私有**，经 `gpu.meshes()` / `r2d.meshes()` / `gpu.mesh_registry()` 取得 |

### 6.4 使用示例

```rust
use rjw_2d_render::{BlendMode, FilterMode, AddressMode, DepthState, CompareFunc, RStates};
use rjw_krusie::prelude::*;

// 不链式 = 继承全局默认状态（`r2d.states()`）
f.draw().sprite(rect, &tex);

// 单条链式覆盖（对象糖）
f.draw().sprite(rect, &tex)
    .samp(FilterMode::Nearest, AddressMode::Repeat)
    .blend(BlendMode::Additive);

// Mesh + 纹理 + 渲染状态
f.draw().polygon(&verts)
    .texture(&tex)
    .blend(BlendMode::Multiply)
    .layer(96.0)
    .tint(Color::CYAN);

// 深度状态（对象糖；`DepthState` 有 test_write / test_only / off 等构造）
f.draw().sprite(rect, &tex)
    .depth(DepthState { test: true, write: true, compare: CompareFunc::Less });

// 全量状态（唯一状态语言）
f.draw().solid(rect).states(RStates::new().blend(BlendMode::Additive).depth_test(true));

// 全局默认状态（唯一入口）
r2d.states_mut(RStates::new().blend(BlendMode::Additive).depth_test(true).depth_write(true));
```

---

## 7. Clear（清屏意图）

crate：`rjw_render`（v0.3 起取代三 `Option` 的 `ClearConfig`）

```rust
pub enum Clear {
    Keep,                          // 全部保留（叠加画面 / 多 pass）
    Color(ColorF64),               // 只清颜色
    ColorDepth(ColorF64, f32),     // 颜色 + 深度
    Depth(f32),                    // 只清深度（保留颜色；重叠画面的独立 pass）
    Stencil(u32),                  // 只清模板
}
impl Clear {
    pub fn color(c: impl Into<ColorF64>) -> Self;   // 接受 Color 或 ColorF64
    pub fn color_depth(c: impl Into<ColorF64>, depth: f32) -> Self;
    pub fn depth(depth: f32) -> Self;
    pub fn stencil(value: u32) -> Self;
    pub fn uses_depth(&self) -> bool;
    pub fn uses_stencil(&self) -> bool;
}
```

```rust
// 一个画面 = 一次 submit(相机, clear)
f.submit(&mut cam, Clear::color(Color::rgb(0.1, 0.1, 0.2)));
// 重叠画面只清深度
f.submit(&mut cam2, Clear::depth(1.0));
// 独立用法（自带渲染上下文时）
r2d.render(&mut render_ctx, Clear::color(Color::BLACK));
```

> 深度 / 模板附件**是否需要绑定**由 `PassBuilder` 从「已排队的 recorder + clear」自动推导，
> 调用方不再手算 `need_depth_stencil`。

---

## 8. DynamicAtlas（纹理图集）

crate：`rjw_atlas`

```rust
pub struct AtlasConfig { pub max_pages: usize, pub padding: u32, pub lifetime: u32, pub page_size: u32 }
pub struct AtlasRegion { pub tl_px: (u32,u32), pub wh_px: (u32,u32), pub origin_px: (u32,u32), pub page_uid: u64 }
pub struct AtlasSprite { pub region: AtlasRegion, pub texture: ArcTextureWrapped }   // 可直接绘制
pub struct InsertOpts { /* origin_px / clamp_margin / permanent */ }
pub struct AtlasStats { /* pages / page_size / total_free / largest_free / fragmentation_percent / generation */ }
pub struct DynamicAtlas<K = String>
pub struct StaticAtlas<K = String>
```

> 💡 `DynamicAtlas` / `StaticAtlas` 均实现 `Index<&Q>` / `IndexMut<&Q>`（`K: Borrow<Q>`）：
> `atlas[&key]` / `atlas["name"]` 直接读写区域。

| 方法 | 说明 |
|---|---|
| `DynamicAtlas::new(gfx, config)` | ★ 创建空图集（`gfx: &Gpu`；页尺寸在 `config.page_size`）。页纹理注册进**该 `Gpu` 的**纹理注册表（不再是全局） |
| `insert(key, Rgba8)` | ★ 最常用：`Rgba8::new(&rgba, (w, h))`，默认 clamp_margin、非常驻、原点 (0,0) |
| `insert_with(key, Rgba8, InsertOpts)` | 指定原点 / `no_clamp()` / `permanent()` |
| `insert_dynamic(key, size, SpriteSource)` | 动态再生精灵（复活时调生成器） |
| `white()` | 1×1 白像素（与字形同页 → UI 实心填充可合批） |
| `region(key)` | 查找（**会刷新寿命**，不触发复活） |
| `region_or_revive(key)` | ★ 查找；若被逐出则自动复活 |
| `handle(key)` | 取 RAII 稳定句柄 `RegionRef`（保活 + 重排后仍有效） |
| `sprite(&handle)` | ★ 解析成 `AtlasSprite`（区域 + 页纹理）→ `r2d.region(spr)` 一次提交 |
| `tick()` | 寿命-1（引擎每渲染帧调用）；有源数据→墓碑，可复活 |
| `stats()` | `AtlasStats`（页数 / 空闲 / 碎片度 / 世代） |
| `compact()` | 去碎片：带源条目全量重排到最少页（重传纹理，`generation`+1） |
| `generation()` | 重排世代号（搬动条目时 +1；缓存区域者据此刷新） |
| `page_size()` / `page_count()` | 查询 |
| `load_toml` / `export_toml` | TOML 导入 / 导出（feature `toml`） |
| `StaticAtlas::from_toml(s, registry)` / `get(name)` | 静态图集（`K=String` 特化）；`tex` 字段按名在本上下文的纹理注册表里解析 |


## 9. Text（文本渲染）

crate：`rjw_text`

基于 `cosmic-text` 排版 + `swash` 字形光栅化 + `DynamicAtlas` 字形缓存。

```rust
pub struct Text { /* font_system, scale_context, glyph_cache: DynamicAtlas<AtlasKey>, locations, ... */ }
```

### 9.1 字形图集是**公开**的（供 UI 等消费者）

字形图集把「字形 + UI 自定义纹理 + WHITE 基础纹理」放在**同一页**，因此它们
纹理相同 → 后端可合批（省掉图形↔文字的纹理状态切换）。它现在是公开可用 / 可修改的：

| 入口 | 说明 |
|---|---|
| `Text::glyph_cache() -> &DynamicAtlas<AtlasKey>` | 只读：`page_count()` / `stats()` / `generation()` / `region(&key)` |
| `Text::glyph_cache_mut() -> &mut DynamicAtlas<AtlasKey>` | **可变**：插入自定义纹理、`compact()` 去碎片、低级查询 |
| `Text::user_texture(id: u64, px: Rgba8) -> Option<AtlasRegion>` | ★ **推荐路径**：固定用 `AtlasKey::Custom(id)` 命名空间 + `permanent()`（不被逐出、不撞字形键） |
| `Text::white_region() -> Option<AtlasRegion>` | 1×1 白纹理 region（UI 实心填充 / 边框 / 光标采样；每次调用刷新寿命） |
| `Text::tick()` | 寿命推进（引擎每渲染帧调用；用户不必手动） |

`AtlasKey` 的两个命名空间**必须分清**：

```rust
pub enum AtlasKey {
    Glyph(cosmic_text::CacheKey),   // rjw_text 内部字形命名空间——消费者不要写
    Custom(u64),                    // 消费者命名空间（定长去重键，如圆角半径）
}
```

**使用 `glyph_cache_mut()` 的四条约定**（违反会静默错位，不报错）：

1. **不要写 `AtlasKey::Glyph(..)`** —— 那是光栅化逻辑的命名空间，会被覆盖 / 误判命中。
2. **不要手改 WHITE 条目** —— 它是 UI 实心填充与字形合批的基础。
3. 插入**非 `permanent()`** 的条目会被 `tick()` 的寿命机制逐出；自己长期持有 region
   的调用方请用 `permanent()`，或改用 `handle()` 走 RAII 保活。
4. **图集重排后 `AtlasRegion` 失效** —— 用 `generation()` 变化判定并重新 `region()` 取。

> 缓存键与命名空间隔离有回归测试锁定：`rjw_text::tests::atlas_key_namespaces_are_distinct`。

### 9.2 方法表（⚠ 部分为 v0.2 旧 API）

> ⚠ 下表中 `draw_label*` / `TextLayout` / `TextRender` / `render_from` /
> `Text::new(device, queue, layout)` / `create_buffer*` / `measure` / `visual_lines`
> 都是 **v0.2 的旧名**，v0.3 已删除或改名为 `Label` 责任链
> （`Text::label(..).size(..).at(..).draw(layer)`）——现行写法见
> `examples/eg260810TextChain` 与本文档 §9.3。

| 方法 | 说明 |
|---|---|
| `Text::new(gfx: &Gpu)` | 创建字体管理器（自动加载系统字体） |
| `load_font_data(data: Vec<u8>) -> Vec<String>` | 加载额外的 ttf/otf/ttc 字体数据，**返回本次新增的字体族名**（去重；空 = 该族已在库里）。应用"导入字体"时必须拿到族名才能用（`label.font_family(name)` 只认族名）——见 `Text::font_families`（内部求差，与 `fontdb` 的槽位顺序无关） |
| `Text::label(text) -> Label` | ★ 责任链入口 → `.size/.line_height/.align/.font_family/.at/.center/.anchor/.draw(layer)` |
| `Text::label_from(&Arc<Buffer>) -> Label` | 从用户保存的共享 `Arc<Buffer>` 进入（跳过整形） |
| `Text::measure_buffer(buffer) -> Vec2` | 已排版 Buffer 的内容宽高（空文本返回 (0,0)） |
| `Text::lines(buffer) -> Vec<VisualLine>` | **视觉行**（自动换行后）：`(byte_start, byte_end, top, width)`——光标/点击/选择与显示对齐 |
| `Text::buffer(..)` / `Text::geometry(..)` | UI 稳定集成面（预排版 / 几何） |
| `white_region()` / `user_texture(..)` / `tick()` | 见 §9.1 |

> **性能**：`Text` 内置**排版缓存（LRU）**——按（文本/字号/行高/对齐/attrs）缓存 cosmic-text 排版，
> 相同输入经 **O(1) 签名**预过滤后返回共享 `Arc<Buffer>`（不深拷贝，跳过重复整形；
> 上限 `MAX_LAYOUT_CACHE` = 128，满时淘汰最久未用）。缓存启用规则：**Debug 恒缓存**；
> **Release 仅缓存 ≤ `LARGE_TEXT_CACHE_LIMIT` = 512 字节的小文本**（大文本多为动态/低频，
> 不入缓存、每帧直接整形）；静态大文本请保存 `Arc<Buffer>` 经 `label_from` 手动复用。
> 空格等**无图字形**只判定一次（`no_image`）；字形图集去碎片重排后自动同步区域。

### 9.3 用法示例（v0.3 现行 API）

```rust
use rjw_krusie::prelude::*;

// 世界层文本（`f.text` 绑定世界层渲染器）
f.text(|t| {
    t.label("HP 100").size(16.0).at(world_pos).draw(10.0);
});

// 屏幕固定文本（`f.text_ui` 绑定 UI 层，物理像素、左上原点）
f.text_ui(|t| {
    t.label("FPS 60").size(14.0).at((16.0, 16.0)).color(Color::YELLOW).draw(0.0);
});

// 往字形图集插自定义纹理（与字形同页 → 同纹理合批）
f.text(|t| {
    // 由任意 RGBA 像素（此处 32×32，自行填充）构造
    let px = vec![255u8; 32 * 32 * 4];
    let region = t
        .glyph_cache_mut()
        .insert_with(
            rjw_krusie::text::AtlasKey::Custom(0xABCD),
            rjw_krusie::gpu::Rgba8::new(&px, (32, 32)),
            rjw_krusie::atlas::InsertOpts::new().permanent(),
        );
    let _ = region;
});
```

> **注（v0.4）**：`rjw_ui::proc`（圆角 9-patch 程序化纹理：`rounded_rect_rgba` /
> `rounded_9patch` / `ROUNDED_TEX_SIZE`）**已删除**。圆角不再走纹理，改由 CPU 镶嵌成
> 三角形（见「渲染增强」一节），因此上面这个示例改为完全自备像素。

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
| `SpriteBuilder<'a>` | `rjw_2d_render` | `sprite` / `solid` 返回 |
| `MeshBuilder<'a>` | `rjw_2d_render` | `mesh*` / `polygon*` / `quads*` 返回 |
| `StaticMeshBuilder<'a>` | `rjw_2d_render` | `static_mesh` 返回，Drop 即提交 |
| `Draw2D<'a, K>` | `rjw_2d_render` | 唯一 Builder 本体（kind 标记与 `DrawKind` 为内部机制，`#[doc(hidden)]`） |
| `SortMode` / `SortPolicy` / `SortKey` | `rjw_2d_render` | 排序（对索引数组重排） |
| `Cull` / `Culler` | `rjw_2d_render` | 剔除（对索引数组过滤 + 可见性判定） |
| `MeshData` | `rjw_render` | 静态网格（GPU 顶点/索引 + uid） |
| `HasUid` | `rjw_render` | 全局唯一 id trait |
| `TypedRegistry<T>` | `rjw_render` | 泛型线程安全注册表（纹理/网格共用） |
| `CustomDraw` | `rjw_2d_render` | 外部绘制 trait（闭包 blanket impl） |
| `CustomBuilder<'a>` | `rjw_2d_render` | `custom` 返回，可链式 RStates |

---

## 11. UI（rjw_ui）

> crate：`rjw_ui`。坐标一律**屏幕像素**（左上角原点、Y+ 向下）；交互状态经 **ID** 持久化于 `UiState`（应用持有）。完整概念见 [ENGINE_GUIDE.md](ENGINE_GUIDE.md)「18. UI」。

### 入口与生命周期

> **一帧 = 开场 + N 段 + 收尾**（"ui anywhere"）：运行时入口 `Frame::ui(theme)` 返回一段
> （`UiSession`，`Deref` 到 `Ui`），**一帧可调用任意多次、位置随意**（世界绘制之前 /
> 之间 / 之后均可），`&mut Ui` 可透传给任意函数。帧级账（帧号 / 命中区翻页 / 输入快照 /
> 焦点导航 + 描边 / 光标 / 统计）**每帧只做一次**：开场在第一段懒执行（应用常先
> `debug_inject_mouse`，快照早了会丢点击边沿），收尾由运行时在 UI 队列被提交前补齐。

| 函数 | 签名 / 用法 | 说明 |
|---|---|---|
| `Frame::ui(Theme)` | `let mut ui = f.ui(Theme::dark());` … `ui.finish();` | **运行时入口**（推荐）：开一段 UI；段结束（`finish()`/作用域结束）自动提交到 UI 层渲染器。段存活期间 `f` 被借用（不能 `draw`/`text`/`submit`），段之间可任意交错。**帧收尾**（焦点描边 / 光标 / 统计）由运行时在 `submit` / `present` / 帧尾自动补齐（幂等），应用无需手动调 |
| `Ui::begin` | `Ui::begin(window, &mut text, &mut state) -> UiInit` | 低层段入口；`window` 用于 IME 候选框定位与光标图标。**输入与绘制解耦**：输入经 `UiInit::capture` 快照、相机/渲染器延迟到 `Ui::finish` 传入。⚠ 帧级账由调用方负责：每帧一次 `UiState::begin_frame()`（`build()` 会兜底） |
| `UiInit::capture(&MouseInput, &KeyboardInput)` | `.capture(ctx.mouse(), ctx.keys())` | 把键盘/鼠标设备状态**拷贝**为 Ui 自持快照（省略 = 空输入，headless 安全）。**本帧第一段冻结一次**，后续段复用（段间注入只影响下一帧） |
| `UiInit::theme(Theme)` / `Ui::theme()` / `Ui::theme_mut()` | `.theme(Theme::dark())` | 主题（默认浅色；`Theme::dark()` 深色；构建后经 `ui.theme()` / `theme_mut()` 读写） |
| `UiInit::base_layer(f64)` | `.base_layer(1e7)` | 基层层级（默认 `1e7`） |
| `UiInit::scale_factor(f64)` | `.scale_factor(ctx.scale() as f64)` | DPI：控件坐标/字号按逻辑像素，内部换算物理像素（默认 1.0） |
| `UiInit::debug_layout()` / `without_debug_layout()` | `.debug_layout()` | 调试 UI 布局：给每个控件/容器矩形画描边（颜色/宽度见 [样式小节](#样式theme可-clone-覆盖) 的 `DebugStyle`；默认关闭）。**无裸布尔**：开 = 调 `debug_layout()`，关 = `without_debug_layout()` |
| `Ui::debug_layout()` / `Ui::without_debug_layout()` | `ui.debug_layout()` | 同 `UiInit` 版本，段内运行时开关 |
| `UiInit::build()` | → `Ui` | 完成构建（本帧未开场则顺带开场：帧号 +1 / 命中区翻页 / 输入快照冻结 / 责任链种入） |
| `Ui::finish(&mut dyn UiBackend)` | `ui.finish(&mut backend)` | **段收尾**：按 `(win, depth, 图形/文字, 录制序)` 序**免全量排序**提交（win + depth 分桶、桶内保持录制序）；**产出 `UiBatch` 批次交给后端**；段统计累加进帧级暂存。一帧可多次 |
| `Ui::end_frame(&mut dyn UiBackend)` | `ui.end_frame(&mut backend)` | **帧收尾**（每帧一次）：输入结算（空白清焦点 / 清一次性边沿 / 窗口按下裁决）/ 焦点导航 + 描边 / 光标定夺 / 统计写回（`UiStats.frame` 每帧 +1、`ui_frame_us` = 开场→收尾）/ 帧级暂存关场 |
| `UiState::new()` | 应用持有 | 跨帧持久状态容器 |
| `UiState::begin_frame()` / `frame_open()` | 运行时内部 / 诊断 | 每帧开场一次（帧号 / 命中区翻页 / 帧级暂存清零）；`frame_open()` 判断本帧是否录过 UI |
| `UiState::reset()` / `remove(id)` | 示例"R 重开" | 清空全部 / 移除单个控件状态 |
| `UiState::text_focus() -> Option<TextFocus>` | `if ui.state().text_focus().is_none() { /* 快捷键 */ }` | **文本焦点**（只有输入框/多行框持焦点才为 `Some`）；取代旧 `capturing_text()` —— 按钮/滑块的 Tab 焦点不再吞应用快捷键 |
| `UiState::combo_open() -> Option<&str>` | `if ui.state().combo_open().is_none() && esc { /* 自己的 Esc */ }` | **当前展开的下拉菜单**（[`Dropdown`](crate::Dropdown) 的**控件绝对 ID**；面板窗口 id = 它 + `::popup`）。与 `menu_open()` 对称（两个槽分开存：栏是应用级 UI，下拉属于某个控件）；`reset()` 清空 |
| `Ui::debug_dump() -> UiDebugDump` | `eprintln!("{}", ui.debug_dump())` | 引擎侧状态快照（每窗口 `id/z/origin/submit/size/drag/press/stored`），单行可 grep；任一段都能调用，帧级暂存跨段共享 ⇒ 后一段能看到前一段录的窗口。⚠ 按**本帧录制过的窗口 ID** 列（不是按 z）：浮层 z = `WIN_TOPMOST` 基址 + **嵌套层数**（子菜单比父面板 +1，`ui::overlay_z`），按 id 列才不会漏掉嵌套浮层；`origin` 是**相对直接容器**的原点（顶层窗口 = 屏幕坐标，**嵌套浮层要叠加外层窗口原点**）。见 [DEBUGGING.md](DEBUGGING.md) §1 |

### 绘制器（`Painter` / `DrawQueue`，v0.3 新增）

**绘制器是独立于 `Ui` 的组件**（`rjw_ui::painter`）：它只拥有**录制状态**（命令队列 +
播放头）与 API 边界的 DPI 换算，**不引用 `Ui`、不需要字体图集 / GPU**。

```rust
pub struct Painter { /* 私有：DrawQueue + scale */ }
pub struct DrawQueue { /* 私有：内容队列 / 调试队列 / seq / depth / cur_win / clip */ }

impl Ui {
    fn painter(&mut self) -> &mut Painter;                        // 一个绘制块取一次
    fn painter_clipped<R>(&mut self, clip: Option<Rect>, f: impl FnOnce(&mut Painter) -> R) -> R;
    fn elem_hint(&self) -> u32;                                    // = painter().elem_hint()
}
impl Painter {
    fn new(scale: f32) -> Painter;                                 // 独立构造（单测 / 离屏）
    fn commands(&self) -> &[UiDraw];   fn debug_commands(&self) -> &[UiDraw];
    fn clip(&self) -> Option<Rect>;    fn scale(&self) -> f32;     fn elem_hint(&self) -> u32;
    fn clipped<R>(&mut self, clip: Option<Rect>, f: impl FnOnce(&mut Painter) -> R) -> R;
    // 原语（默认 elem = elem_hint()）
    fn solid(&mut self, rect, color);                     fn border(&mut self, rect, color, width);
    fn panel(&mut self, rect, bg, border, border_w, radius);
    fn panel_elem(&mut self, rect, bg, border, border_w, radius, elem);      // elem = 0 容器装饰
    fn panel_img(&mut self, rect, bg, img, border, border_w, radius);
    fn panel_img_elem(&mut self, rect, bg, img, border, border_w, radius, elem);
    fn shadow(&mut self, rect, &ShadowStyle, radius);      // elem = 0
    fn rounded_at(&mut self, pos, size, radius, color);    fn gradient_at(&mut self, pos, size, gradient);
    fn icon_at(&mut self, pos, size, icon, color);         fn image_at(&mut self, pos, size, bg);
    fn text(&mut self, rect, text, size, color, family, align, valign, soft_clip, buf);
    fn text_noclip(&mut self, rect, text, size, color, family, align, valign, buf);
    fn draw(&mut self, kind: DrawKind, rect, elem);        // 底层（显式 elem）
    // 调试图元（进 debug 队列）
    fn debug_line / debug_rect_outline / debug_circle_outline / debug_cross / debug_grid
}
```

| 约定 | 说明 |
|---|---|
| **一段绘制一个 painter** | `ui.painter()` 借 `&mut self`；painter 存活期内 `ui.text_size` / `ui.hit_abs` / `ui.*_at` 都借不到 `ui`。顺序 = 先量 → 画 → 再量 |
| **`elem` 默认逐条取** | 原语各自取"录制时的 `seq + 1`"，与旧 `Ui::push_*` 逐位一致。装饰要压在自家内容之上时**重新取一次 `ui.painter()`**（此时 `elem_hint` 已更大）；**不要**把一个 painter 的 elem 想成冻结值 |
| **容器装饰用 `elem = 0`** | **本容器**的背景 / 边框 / 阴影：`panel_elem(.., 0)` / `panel_img_elem(.., 0)` / `shadow(..)`。⚠ 语义是"画在本容器元素**之下**"——**只有窗口（`win > 0`）才天然成立**（一扇窗 = 一个排序空间）；**win=0 的容器**靠"顶层放置序 `place`"获得自己的排序空间（见 `ENGINE_GUIDE.md` §18.23），所以 `drag_panel_at` 的底色不会被别的 win=0 内容穿透。**滚动条 / 手柄这类"要盖在自家内容之上"的装饰不要传 0，用 `elem_hint()`** |
| **裁剪就在 painter 上** | Clip 沙箱 / ScrollView 可视区 / 严格窗口内容裁剪 = `Painter::clip()` 的**当前层**，命令自带它 ⇒ **沙箱里的控件什么都不用做**。只有"要一层与当前**不同**的裁剪"才用 `painter.clipped(Some(rect), \|p\| ..)` / `ui.painter_clipped(rect, ..)`（更窄，或 `None` 主动不裁），块外自动恢复 |
| **独立可用** | `Painter::new(1.0)` + `commands()` ⇒ 无字体图集也能断言"画出了哪几条命令"（`rjw_ui` 的 painter 单测就是这么写的） |

> 旧 `Ui::push_solid_rect` / `push_border_rect` / `push_panel_like(_img)` / `push_panel_shadow` /
> `push_text_rect(_noclip)` / `rounded_rect_at` / `gradient_rect_at` / `icon_at` / `image_at`
> **全部保留**，实现为绘制器的薄包装（既有调用点无需改动）。

### UI 绘制后端（`rjw_ui::backend`，v0.3 新增）

`rjw_ui` **只输出批次数据**，不直接调用任何渲染器。批次经 [`UiBackend`] 交给后端：

```rust
pub trait UiBackend {
    fn texture(&self, uid: u64) -> Option<Arc<TextureWrapped>>;
    fn submit(&mut self, batch: UiBatch);                       // 顺序 = 绘制顺序
}
```

> **只有两个方法**（v0.3）：UI 的绘制输出就是「纹理 + 顶点」。后端**不需要**参与纹理生成
> ——矩形渐变已改为四角顶点色（见「渲染增强」一节），不再有程序化渐变纹理请求。

```rust
pub type Tri = [u16; 3];

pub struct UiBatch<'a> {
    pub texture: Arc<TextureWrapped>,
    pub vertices: &'a [VertexP3U2C4],  // 已是最终屏幕物理像素坐标
    pub indices: &'a [Tri],            // 相对 vertices；UI 全程直出三角形
    pub transform: Transform2D,        // 实例级（窗口 FX 不重建顶点）
    pub tint: Color,                   // 实例级整段染色（顶点色已含控件自身 tint）
    pub layer: f64,
    pub clip: Option<Rect>,            // **批次 scissor**（屏幕物理像素；None = 不裁剪）
    pub source: UiBatchSource,         // 实例用户数据
}

pub struct UiBatchSource { pub window: u32, pub elements: u32, pub debug: bool }
```

> **⚠ 借用（v0.3 / 阶段 9）**：`vertices` / `indices` 是**借来的切片**——几何住在
> `rjw_ui` 的提交计划缓存里，提交期只借用一次，**不再"克隆一份再交给后端"**（那是
> `finish` 里最大的一笔 memcpy）。后端必须在 `submit` 调用内消费完（拷进自己的缓冲 /
> 命令队列），不要存起来跨帧用。`RecordingBackend` 因此把批次存成**拥有**顶点的
> `RecordedBatch`（`batches: Vec<RecordedBatch>`）。
>
> **`clip`（v0.3）**：UI 的环境裁剪（严格窗口内容 / ScrollView 可视区 / Clip 沙箱 /
> 文本框盒）以**批次 scissor** 交付，几何保持原形（圆角 / 环带 / 投影不再被切平）。
> 后端把它交给渲染器的**命令级 scissor**（`Draw2D::scissor`，见 §5.6）；`None` = 只受
> 画面级 `Render2D::scissor` 约束；**空矩形应跳过该批次**（不是"不裁剪"）。
>
> `RecordingBackend` 提供 `clipped_batches()` / `clips()` 供断言。

> `indices` 允许为空——后端此时应按「每 4 顶点一组、顺序 `TL,TR,BL,BR`」的旧四边形
> 约定补出索引（`Render2dUiBackend` 即如此回退），使外部后端仍可只产出顶点。
> `rjw_ui` 自己的产出**恒带索引**：圆角 / 羽化本身就是三角形，四边形只是它的退化情形。

**实现者**：`rjw_krusie::runtime::layers::ui_backend::Render2dUiBackend`（桥接到 `Render2D`）；
`rjw_ui::RecordingBackend`（**纯 CPU**，收集批次供测试断言 draw call 数）。

**实例粒度 / DrawCall 取舍**（一个 `UiBatch` = 一个实例 = 一次 draw call 候选）：

| 规则 | 原因 |
|---|---|
| 同一**提交单元**（窗口 / win=0 顶层放置）内所有控件 / 容器合成一批 | 「尽量减少 DrawCall」——实例内容范围 = 整个单元 |
| **按窗口切** | 批次的 `transform` / `tint` 是窗口级的；烘进顶点会让 FX 动画每帧重建整窗顶点，摧毁窗口顶点缓存 |
| **按 win=0 顶层放置切**（v0.3 / 阶段 8–9） | 每个放置要有自己的**排序空间**（否则 `elem = 0` 的装饰被邻居穿透）；同时使"合并结果"能作为**计划**整体跨帧缓存 |
| **按纹理切** | 一次 draw call 只能绑一个纹理（bind group） |
| 超 `MAX_UI_SEG_VERTS` 切 | u16 索引上限 |

> ⚠ **批次数 = draw 数**：每个 `mesh_indexed(..)` 带自己的矩阵下标，`Render2D` 的动态段
> 合批对 UI 批次**不生效**（实测 UI 层 `draw_op_count() == segs`）。所以"分成几个批次"
> 是真的多画几次——阶段 9 用 **+4 次 draw（40 → 44）** 换掉了每帧 ~300µs 的顶点拷贝，
> 详见 `UI_ARCHITECTURE.md` §6.2。

`source.elements` 记录本批次覆盖的控件数，是上述取舍的可观测指标。
切段规则由纯函数 `segment_runs` 裁决，契约由 `ui::batch_contract_tests` 断言
（单窗口单纹理 = 1 次 draw call；控件数不增加 draw call；窗口/纹理切换必切段）。

### 容器（布局）

**容器责任链 builder**（**唯一入口**）：选项链式设置、`.show(f)` 终结；
裸布尔全部换枚举（`Level` / `Placement` / `Resize` / `Child`，见下表），
旧的 6 个 `window_at*` / 3 个 `modal_at*` 变体**已删除**（v0.3）。

**枚举选项（取代裸布尔 / 开关方法）**

| 枚举 | 变体 | 取代 |
|---|---|---|
| `Level` | `Topmost`（默认，点击置顶） / `Normal`（点击不改 z 序） | `.topmost(bool)` |
| `Placement` | `Expand`（默认，内容撑高、不裁剪） / `Clip`（强制裁剪到窗口矩形） | `.strict()` |
| `Resize` | `None`（默认） / `Horizontal`（只调宽） / `Both`（宽高同调） | `show_handle: bool` |
| `Child` | `Expand` / `Fit`（子项尺寸策略，用于 `child_rect`） | 裸 `bool` 语义 |

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
| `ui.window(id)` | `.pos(..)`（**不调 = 引擎自动级联**，Win32 `CW_USEDEFAULT` 语义：按首次出现顺序右下偏移 28 逻辑像素，越界回绕；结果持久于 `UiState::auto_pos`，用户拖拽优先） `.width(w)` `.gap(Size)` `.level(Level)` `.placement(Placement)` `.style(PanelStyle)` `.clamp(WindowClamp)` **`.resize(allow, axes)`** `.title(&str)` `.close_button(&mut bool)` `.shrink(bool, &mut bool)` `.show(\|w\| ..)` | **可重叠窗口**（唯一入口）：点击置顶（焦点 z-order，`UiState.window_z`）+ 可拖拽（位置持久于 `UiState.panel_pos`）；`.width` = 固定宽（右下角可缩放）；`.gap` = **内容子项行距**（不调 = `Theme::gap`；下拉 / 菜单浮层用 `Physical(popup_gap(scale))` 拿"逻辑 1px"的紧行距）；`.placement(Clip)` = 强制裁剪；`.style` = 逐窗口样式覆盖（默认 `Theme::panel`）；`.clamp` = 位置约束（`Screen` 限位不跑出屏幕（默认）/ `Free` 自由 / `Locked` 锁定位置不可拖）。窗口内同一 layer 按"背景/图形→文字"绘制。**拖拽缩放** `.resize(allow: bool, axes: Resize)`：`allow = false` ⇒ 不画柄也不响应拖拽（`.width` 仍是布局固定宽）；`Resize::Both` ⇒ 宽高同调（高度持久于 `UiState::window_heights`，**并自动裁剪内容**）；**不调** = 旧行为（有 `.width` 才能横向拖）。**外框**（标题栏 / × / 收起）见下 |
| `ui.panel()` | `.pos(..)` `.drag(id)` `.style(..)` `.show(\|pp\| ..)` | 面板 = `panel_at` + `drag_panel_at` 统一入口 |
| `ui.modal(id)` | `.pos(..)` `.width(w)` `.show(\|m\| ..)` | 模态对话框（唯一入口） |

**菜单栏**（横向触发器 + 同一套下拉面板）：

| 入口 | 链 | 语义 |
|---|---|---|
| `ui.menu_bar(id, pos, \|bar\| ..)` | `bar.menu(label, \|m\| ..)` → `MenuCtx::{item, item_checked, submenu, caption, separator}` | 返回栏尺寸；`pos` = 栏左上角（顶层 = 屏幕坐标）。展开状态跨帧持久于 `UiState::menu_open`（触发器绝对 ID）；**同一时刻只有一个菜单开着**，点菜单项 / 点栏外 / Esc 都收起（点**另一个触发器** = 切换，不算点外）。`MenuCtx` **`Deref` 到 `Window`** ⇒ 菜单里同样能放 `label` / `button` / `divider` / `row`（横向排版）/ `text_input` / `add(..)`，以及 [`Item`](crate::Item) 责任链菜单项（含 **`Submenu`**：Hover 在行右侧展开）。**下拉面板与 [`Dropdown`](crate::Dropdown) 共用同一实现**（`crate::widgets::menu::popup_show`）：`Level::Normal` + **`WindowClamp::Locked`**（点它不置顶、**拖不动**）+ **不画缩放柄** + z 强制 `WIN_TOPMOST` 哨兵 + **行距 `popup_gap(scale)`** —— 所以菜单栏录在哪里都盖得住别人。面板排版由引擎保证：内边距 = `ComboStyle::item_pad_x`（菜单项 / `caption` / `separator` 天然同列）、面板宽取上一帧结算宽（子项高亮 / 分割线**铺满面板**）、勾选是**方框**且画在项内容里、`caption` 用 `text_muted` + 小字号做分组标题。细节见 `docs/ENGINE_GUIDE.md` §18.13/§18.15；几何可 `RJ_MENU_TRACE=1` 打印 |

**窗口外框（标题栏 / 关闭 / 收起）**：三个选项各自独立、**都不调就完全没有外框**
（逐像素等于旧行为）；任一开启都在窗口内容**第一行**录一条标题栏（底色
`Palette::surface_raised` + 面板同色边框 ⇒ 通条，底边那条就是分隔线）。

| 选项 | 签名 | 语义 |
|---|---|---|
| `.title(t)` | `title(text: &str) -> Self` | 标题文字（过长按省略号截断，不撑宽窗口）；标题栏空白处**仍可拖动窗口**。标题行**贴窗口顶边**（条高 = `title_bar_h(row_h)` = **一行**，纯函数）：下面的内容与窗口高度各少一个 `pad_total`（用户实测"可以往上抬"）；条只是**背景装饰、不裁剪内容** ⇒ 标题 / ▲ / ✕（边长 `row_h - 2`）允许**比条高** |
| `.close_button(open)` | `close_button(open: &mut bool) -> Self` | 标题栏右侧画 ×；点击把 `*open` 置 `false`。`*open == false` 时**整窗短路**——不录制、不写原点/尺寸、**不占遮挡矩形**（不会留下"看不见却挡点击"的窗口）；重新打开是**应用的责任**（把 `*open` 置回 `true`，如菜单勾选） |
| `.shrink(show, collapsed)` | `shrink(show: bool, collapsed: &mut bool) -> Self` | `collapsed = true` = 只留标题栏（跳过内容闭包）；点 ⌃ 取反。`show = false` 时**不画按钮**，但 `*collapsed` 照旧生效（由菜单/代码收起展开）——这是两个参数分开的用处 |

`*collapsed` 在录制**开头**读取（点击当帧不变、下一帧生效）；`×` / `⌃` 上的按下会
**认领**（`claim_press`）⇒ 点按钮不会顺带拖动窗口。

**caption 按钮的位置**（Windows 风格）：`[⌃][✕]` 顺序（`✕` 恒在**最右**）用
`Ui::add_at` **绝对定位在窗口外框坐标系**——固定尺寸窗口下簇右缘 = **外框右缘** −
`TITLE_BUTTON_INSET`（0）、`y = 0`、高 `row_h`；最右按钮的右上角**取面板右上圆角**，
贴外缘时不会戳出圆角。自动宽窗口（不调 `.width(..)`）没有"外框右缘"可贴 ⇒ 簇**跟随标题**。
落点由纯函数 `ui.rs::title_bar_layout` 解算（单测钉住"贴右缘"）。⚠ 旧版把它当行内子项、
用 `spacer = 内容宽 − 标题宽` 推到**内容**右缘 ⇒ 离窗口右缘永远差 `pad + 4`（实测 18px）。

选项载体 `WindowOptions` / `PanelOptions`（公开，可独立构造/复用）。容器闭包内经
`UiAdd::window(id)` / `UiAdd::panel()` 同样可用。

| 函数 | 签名 | 语义 |
|---|---|---|
| `label_at` | `ui.label_at(pos, text) -> Vec2` | place：绝对定位 + 内容自然尺寸 |
| `pack_at` | `ui.pack_at(pos, side, \|p\| ...) -> Vec2` | pack：按 `PackSide::Top/Left` 堆叠，宽/高 = 最大子项 |
| `panel_at` | `ui.panel_at(pos, \|pp\| ...) -> Vec2` | 背景 + 边框 + 内容垂直堆叠，尺寸自动包裹（等价 `ui.panel().pos(pos).show(..)`） |
| `drag_panel_at` | `ui.drag_panel_at(id, pos, \|pp\| ...) -> Vec2` | 同 panel_at，且按住面板任意处可**拖动**（位置持久于 `UiState.panel_pos`；拖动期间子控件不响应；等价 `ui.panel().pos(pos).drag(id).show(..)`） |
| `scroll_at` | `ui.scroll_at(pos, view_size, id, \|s\| ...) -> Vec2` | **滚动容器**：内容在可视区内垂直堆叠（pack Top），滚轮 / 滚动条（拖 thumb、点轨道翻页）滚动；可视区外**裁剪**；偏移持久于 `UiState.scrolls` |
| `grid_at` | `ui.grid_at(pos, cols, id, \|g\| ...) -> Vec2` | 均匀网格；`id` 缓存单元格尺寸（跨帧稳定） |
| `flex_at` | `ui.flex_at(pos, total_h, &[w1,w2,..], \|f, i\| ...) -> Vec2` | **flex 容器**：固定总高 `total_h` 按 `weights` **权重等分**子项高度（扣 gap；回调按索引布局，同帧精确）；内容超高溢出可见（需滚动时内嵌 `scroll_at`） |
| 容器内 `*_at(offset)` | `p.panel_at(offset, \|inner\| ...)` | 嵌套容器（相对当前容器内容原点，不占光标） |

### 控件协议（`Widget` / `Sense` / 申请 API，v0.3 起）

**`Widget` 只有一个方法**（尺寸在 `ui()` 里就地申请，参考 egui）：

```rust
pub trait Widget {
    fn ui(self, ui: &mut Ui) -> Response;     // 申请 → 画 → 收交互 → 返回响应
}

impl Ui {
    // 申请（尺寸 = 物理像素 Vec2；要逻辑单位先 to_physical(scale)）
    fn allocate(&mut self, size: Vec2) -> Rect;
    fn allocate_mode(&mut self, size: Vec2, mode: Expansion) -> Rect;
    fn allocate_at(&mut self, pos: impl Into<Position>, size: Vec2) -> Rect;
    fn allocate_sense(&mut self, id: &str, size: Vec2, sense: Sense) -> (Rect, Response);
    fn allocate_sense_mode(&mut self, id: &str, size: Vec2, mode: Expansion, sense: Sense) -> (Rect, Response);
    fn allocate_sense_at(&mut self, pos: impl Into<Position>, id: &str, size: Vec2, sense: Sense) -> (Rect, Response);
    // 交互（申请与交互分开时用；`allocate_sense*` 已含这一步）
    fn interact(&mut self, id: &IdAbsolute<'_>, rect: Rect, sense: Sense) -> Response;
}

pub struct Sense { pub hover: bool, pub click: bool, pub drag: bool, pub focus: Option<FocusKind> }
impl Sense {
    pub const NONE / HOVER / CLICK / DRAG: Self;
    pub fn focus(self, kind: FocusKind) -> Self;    // 进焦点链（Tab + 焦点描边）
}
```

- `Sense::drag` = **按下即 `claim_press()`**（外层窗口 / 面板不把这次按下当作拖动基准）
  + `update_drag`；拖拽基准与数值映射仍由控件维护（`WidgetState::{press_mouse,press_panel}`）。
- `interact` 一次做完：命中（含窗口 / 控件级遮挡 / 裁剪过滤）→ 焦点 → 按下认领 →
  `update_interact`（hover / pressed / clicked / released）→ `note_press_handled` → `update_drag`。
- `Response` 增 `rect: Rect`（本帧最终矩形；`UiAdd::label` 就靠它返回尺寸）与
  `culled: bool`（**被裁剪层完全剔除** ⇒ 控件应立刻 `return resp`，不镶嵌不入段）；
  `pressed` = **持续按住**（与 `ButtonState::pressed` 同义），`clicked` = 本帧完成点击。
- `Ui::culled(rect) -> bool`：只用 `allocate`（只要矩形）的控件自己判一次可见性，
  等价于 `Response::culled`。**scissor 只省片元**，剔除才省镶嵌/顶点/draw——
  见 `docs/ENGINE_GUIDE.md` §18.22。
- 膨胀语义是**申请方式**：`Expansion::{UnlimitedExpansion(默认), LimitedInParent(压到 avail_w), DisableAutoExpansion(不撑大父级)}`；
  min/max 用 `apply_constraints(desired, c)` 自己应用。
- `add_at(pos, w)`：给 `Ui` 打**一次性放置覆盖**，控件的第一次申请消费它（**只在第一次**；
  控件要摆多块用 `allocate_at`）。
- ⚠ **顺序**：先量（`text_size` / `avail_w`）→ 再申请 → 再画（`ui.painter()`，一个绘制块一个）。

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
| `NumberInput` | `p.add(NumberInput::new(id, &mut f32).range(min, max).step(s))` | **数字条**：右侧 `GRIP_W`（公开常量 **20px**）宽那条手柄**水平拖动**调值（向右 = 增；Shift ×10 / Ctrl ×0.1；拖到窗口边缘自动 warp），**文本框**点击 = 进入编辑（只收数字 / 负号 / 小数点）；显示精度跟 `step` 走（`0.25` → `2` 位小数、`≥1` → 整数）。常见组合：**滑杆后跟数字条**（拖滑杆粗调、数字条精确输入，两者绑同一个 `&mut f32`）——`eg260818UI` 的主题调节窗口整列都是这个形态，脚本化验证见 `--sim-tuner` |
| `Dropdown` | `p.add(Dropdown::options(id, label, &mut u32, &[&str]))` / `p.add(Dropdown::new(id, label).menu(\|m\| ..))` | **按钮下拉菜单**（下拉框与菜单栏下拉**简并后**的唯一入口；`Widget` ⇒ 任意容器 `add`）：`options` = **选项列表模式**（菜单项由引擎排：选中行打勾 + 整行高亮，点击写回 `&mut u32` 并收起，键盘 ↑/↓ 切换）；`menu(..)` = **富内容模式**，闭包参数是 `MenuCtx`（`Deref` 到 `Window`）⇒ **菜单内又可以 `UiAdd::add`**（文本输入 / 分割线 / 菜单项 / 横向排版 / **子菜单**）。`.side(PopupSide::Below\|Right)`（默认下方 2px）/ `.width(..)` / `.font_size(..)`。展开状态 = `UiState::combo_open()`（**控件绝对 ID**）；点触发器切换、点项执行+收起、点面板外 / Esc 收起、点**任意 `WIN_TOPMOST` 浮层**不收起（子菜单用）。面板 = 锁定位置 + 不画缩放柄 + `WIN_TOPMOST`。几何助手（公开，脚本算坐标用）：`item_h(font_size)` / `popup_padding(theme)` / `popup_origin(trigger, side)` / `popup_gap(scale)`（行距 = 逻辑 1px 的 floor 值，见 `MENU_GAP`）。见 `docs/ENGINE_GUIDE.md` §18.15，仿真 `--sim-dropdown`（7 段） |
| `Item`（菜单项责任链） | `m.item(Item::new("…").click_behavior(MenuClick::Keep))` / `.checked(&mut bool)` / `.submenu(\|s\| ..)` | **菜单项 builder**（`MenuCtx::item` 接受 `&str`（旧糖，`From<&str>`）或 `Item`；返回"本帧是否被点击"）。`.click_behavior(MenuClick::{Close,Keep})` = **点击行为 flag**（点完收起 / **保留 popup**）；`.checked(&mut bool)` = 勾选项（方框 + 点击自动翻转）；`.submenu(\|s\| ..)` = **子菜单（Submenu）**：普通项样式 + 右侧 ▸，**Hover 在行右侧展开**（`PopupSide::Right`），**点击不收起**，子面板里点项 ⇒ **整条链**收起；状态是**行自持**的 `WidgetState::submenu_open`（不碰 `combo_open`）。便捷糖：`m.item_checked(label, &mut bool)` / `m.submenu(label, \|s\| ..)` |
| `combo` / `combo_at` | `p.combo(id, current, &[String], Option<u32>) -> Option<u32>` | **旧入口（糖）**：= `Dropdown` 的选项列表模式 + 固定布局宽，签名 / 返回值 / 行为与旧版一致（`None` = 无选择 / 未展开；`Some(i)` = 本帧新选中）。新代码请用 `Dropdown`；`FontModal` 的字重下拉仍走它（老代码不必改） |
| `Segmented` | `p.add(Segmented::new(id, &["紧凑","标准","宽松"], &mut idx))` | **分段按钮组**（互斥选项**拼在一起**）：相邻段共享边、只有整组外侧角是圆角、选中段高亮；点击把新索引写进 `&mut usize`。**段间分隔线与 `ButtonStyle::border_w` 解耦**（边框关掉时退化成 `Palette::surface_dim`，否则三段连成一条）。`.font_size(..)` 可覆盖字号。见 `docs/ENGINE_GUIDE.md` §18.14 |

### 状态视图

| 类型 | 方法 | 说明 |
|---|---|---|
| `ButtonState` | `hovered()/pressed()/clicked()/released()` | 按钮状态（本帧点击 = 按下+释放均在本体） |
| `CheckboxState` | `checked()/toggled()/clicked()` | 勾选框 / 单选状态 |

### 样式（`Theme`，可 clone 覆盖）

`Theme { label, panel, button, slider, input, checkbox, divider, debug, focus, modal, combo, gap, row_h, feather, line_spacing, font_weight, palette }`，子样式见 `crates/rjw_ui/src/style.rs`：

**主题序列化（TOML）**（`rjw_ui` 的 `serde` / `toml` feature，**默认开**）：

| API | 语义 |
|---|---|
| `Theme::to_toml() -> Result<String, String>` | **全量导出**：`format_version` 头 + `[theme]` 字段树。⚠ `panel.bg_image` **不序列化**（纹理 uid 不可移植） |
| `Theme::from_toml(s) -> Result<Theme, String>` | 从 TOML 加载（起点 `Theme::default()`；缺字段回落默认、多余键忽略） |
| `Theme::apply_toml(&mut self, s) -> Result<(), String>` | **在当前主题上合并覆盖**（文件里出现的字段才改）——手写 `gap = 12` 这类小文件的语义 |
| `THEME_FORMAT_VERSION`（`rjw_ui::theme_toml`） | 格式版本；加载时**比本引擎新**⇒报错拒载。加字段不用改它，**改名 / 删字段要改** |

`Weight` 存 `u16`（`weight = 700`）、`Align` 存小写名、`CornerRadius` 反序列化**两种写法都收**
（`radius = 6.0` 或 `radius = { tl = .. }`）、`Brush`（背景刷）用显式
`{ kind = "solid|vertical|horizontal", colors = [..] }`（**不要**用 serde 默认的外部标签枚举
`{ Vertical = [..] }`——那在 TOML 里是数组表，读回来报 `wanted exactly 1 element`；加载侧兼容旧写法）。
示例侧入口：顶栏「导出主题…」「导入主题…」（`rfd`）
+ **`--theme <路径>`**（启动载入；与导入同一条通路）+ `--sim-theme <路径>`（脚本化验证）。
见 `docs/ENGINE_GUIDE.md` §18.16。
`LabelStyle`（font_size/color/align）、`PanelStyle`（bg/border/padding/**radius**/**shadow**/**grip**）、`ButtonStyle`（三态 bg + padding + **radius**）、
`SliderStyle`（track/fill/handle）、`InputStyle`（bg/border_focus/caret/**sel_bg**/preedit/padding_x/height/min_w + **radius**）、
`CheckboxStyle`（box_size/checked_fill/gap）、`DividerStyle`、`DebugStyle`（layout_outline / layout_outline_width）、
`FocusStyle`（color / width，键盘导航焦点描边）、`ModalStyle`（dim / size）、
`ComboStyle`（下拉浮层现代菜单：menu_bg/border/radius/pad_v + item_hover/selected/pad_x/min_w + fg/fg_mark）。
`Theme::default()` 浅色，`Theme::dark()` 深色。

**布局令牌**（"同一套界面在小屏排得下、在大屏更舒展"）：`Theme::density(Density)` 一趟按比例
缩放间距 + 字号 + 行距（`Compact` 0.84/0.92/1.10、`Cozy` **默认** = 1.0/1.0/1.2、
`Spacious` 1.18/1.08/1.30）；单维微调用 `with_font_scale` / `with_spacing_scale` /
`with_line_spacing`（都是**倍率**、在现值上叠乘）。`Theme::line_spacing`（行高 = 字号 × 该值，
默认 `DEFAULT_LINE_SPACING` = 1.2）只作用于**可能换行的文本**（多行 TextArea / `wrap(..)`
标签），`wrap <= 0` 的单行文本行高恒等于字号；`Theme::scaled(DPI)` 不缩放它（倍率不是尺寸）。
`eg260818UI` 的「主题调节」窗口可实时切档 + 拖三根倍率滑杆。

**字重令牌**：`Theme::font_weight: Weight`（`rjw_ui::Weight` = `fontdb` 的 `Weight(u16)`，
常量 `THIN` 100 … `NORMAL` 400 … `BLACK` 900；默认 `NORMAL` = 与扩展前逐像素一致），
入口 `Theme::with_font_weight(w)`。它是**全局**文本令牌：作用于 `Ui` 里所有排版
（标签 / 按钮 / 输入框 / 下拉 / 换行文本……——它们都经同一对出口建缓冲）。
字体没有该字面时由 cosmic-text 按最接近的字面回落。**不是尺寸量**：
`Theme::scaled(DPI)` / `Density` 都不碰它。⚠ 字重会改**字形与步进宽度**（布局随之变）
⇒ 排版缓冲缓存键与窗口 / win=0 子槽的几何签名都含字重，改字重时会自动重建
（见 `docs/ENGINE_GUIDE.md` §18.7）。

`builtin::FontModal`（字体弹窗）现在同时管**字体族 + 字重**：
`FontModal { input, weight: &mut Weight, apply: &mut dyn FnMut(&str, Weight) }`，
字重下拉项来自 `rjw_ui::FONT_WEIGHT_CHOICES`（**九档 100…900**：超细/特细/细/常规/中等/半粗/粗/特粗/黑），
显示名 `weight_label(w)`。
⚠ `weight` 是**草稿**：下拉**选中那一帧就写回**（下拉是"点一次就收起"的控件，攒到"确定"会被
下一帧重置 ⇒ 症状"选不中任何其他字重"）；应用要把它与"已应用值"分开（打开时拷入、确定提交、
取消丢弃）。对话框里的**预览**用**草稿字重**排版（临时代换 `Theme::font_weight` 后还原；
`RJ_FONT_TRACE=1` 打印预览用的字重 + 样本实测宽）。
`eg260818UI` 里字重由弹窗写草稿、「确定」时经 `apply` 落到应用侧 `TopBar::font_weight`，
下一帧主题按它重建；`--sim-weight`（字重是排版输入）/ `--sim-weight-modal`（下拉真能换档 +
确定才提交）脚本化守护这两条路径。

**投影令牌**：`PanelStyle::shadow: ShadowStyle { blur, offset, color }`（色令牌 `Palette::shadow`），
主题级入口 `Theme::with_shadow(ShadowStyle)` / `Theme::without_shadow()`，逐容器入口
`PanelStyle::{with_shadow, with_shadow_color, without_shadow}`。`blur = 0` = 不画（不是
`Option`）。**颜色是任意色**：顶点 RGB 原样带出、只有 alpha 按圈衰减 ⇒ alpha 管深浅
（深色预设 120 / 浅色 48）、RGB 管色相；`blur > 0` 而 alpha = 0 仍会镶嵌几何（看不见而已），
要省几何请归零 `blur`。见 §11「渲染增强」下方的说明。`eg260818UI` 的「主题调节」窗口里
「投影」滑杆后面那个色块就是它（可拖 alpha），`--sim-shadow` 脚本化守护这条通路。

**缩放柄令牌**：`PanelStyle::grip: GripStyle { shape: GripShape, color, size, step, count }`
—— 只对**允许拖拽缩放**的窗口（`.resize(true, ..)`，或没调 `.resize` 但设了 `.width(..)`）生效。
`GripShape::{Squares（默认，历史观感）, Bars（**三条实心横杠**）, Diagonal（**三条 45° 斜线**：首端点在同一水平线上等距、末端点在同一竖直线上等距）, Hidden}`；
逐窗口入口 `PanelStyle::{with_grip, with_grip_color, with_grip_shape, without_grip}`。
`Hidden` 只是**不画图案**，**拖动缩放照旧**（命中区独立存在，跟随 `size*step*count`，下限 14px）。
`Bars` 用实心矩形（小尺寸下图标会被 `Theme::feather` 糊成一坨）；`Diagonal` 只能走图标，
方框取 1.5× 免得三条斜线糊在一起。

**边框归零（`border_w = 0`）的可见性**：未勾选的 `Checkbox` 本来只画一圈描边，边框关掉后
会**整个消失**（标签看起来"没有控件"）⇒ 此时改画**实心底**（`surface_sunken` / 悬停
`surface_hover`）。`Segmented` 的段间分隔线同样与 `border_w` 解耦（退化成 `surface_dim`）。
凡"只靠描边存在"的新控件都要提供第二视觉来源，见 `docs/ENGINE_GUIDE.md` §18.14。

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

> **圆角半径**（`radius`，逻辑像素，默认 0 = 直角）：面板 / 窗口 / 按钮 / 输入框 /
> 勾选框（`CheckboxStyle.radius`）的**背景与边框**都由 CPU 把矩形**镶嵌成三角形**
> （`tess` 模块）——**不生成任何纹理、不改着色器**。
>
> - 硬体轮廓 `alpha = 1`，同心的**外圈**轮廓 `alpha = 0`，两者配成带状三角形，
>   由光栅化器插值出羽化边缘（这就是抗锯齿）。梯度**以几何边缘为中心**：
>   硬体内缩 `f/2`、外环外扩 `f/2` ⇒ 视觉尺寸恒等于给定矩形。
> - 羽化宽 = `Theme::feather`（**逻辑像素**，默认 1.0 ≈ 标准 1px 抗锯齿；
>   0 = 硬边；用 `with_feather(px)` 改，DPI 预乘为物理像素）。小控件自动收紧
>   （`min(f, min(w,h)/4)`），半径小到放不下两个同心轮廓时自动退回不羽化。
> - 「单位四分之一圆弧表 + 步长抽样」：一张 32 点细表服务**所有**半径——
>   半径 → 目标弧距 2px → `stride`（2 的幂）→ `segs = 32 / stride`。
>   `N_FINE` 是 2 的幂 ⇒ `segs × stride` 恒等于 `N_FINE`，四角弧首尾严丝合缝。
> - **带状化沿整圈推进**（角的末点与下一角的首点在全局点号上相邻）⇒ 四条**直边**
>   自动被覆盖；按角分别成带会漏掉它们（曾经的表现：边框只剩 4 个圆角孤岛、
>   四条边整个消失，羽化也只在角上有效）。
> - **背景颜色按顶点位置双线性 lerp**：硬体**每一个**顶点（含圆角弧上的）都按自己在
>   矩形中的位置取色（`bilinear_color`）。若弧上顶点直接带"本角颜色"，渐变的两端会被
>   钉在弧的跨度上——200px 宽的胶囊 + 半径 18，左右各 18px 变成纯端色、整条斜坡被压进
>   中间 164px（肉眼："两端发平、渐变被拉长"）。羽化环则**照抄**硬体同序号顶点的颜色、
>   只把 alpha 置 0 ⇒ 纯 alpha 斜坡，不夹带色偏。
> - 半径**不做取整**（镶嵌器接受任意小数半径，超出半高时 clamp 成胶囊）；
>   因此高 DPI 下不再有 `radius × scale` 的取整误差问题。
> - 圆角**边框**是一圈**环带**（外轮廓与内轮廓之间，**含四条直边**；内半径按
>   `max(0, r_outer - width)`，与 CSS `border-radius` 同规则）——边框的**内外两条
>   边界都做羽化**（最多四圈同心轮廓）。比"外圈实心 + 内圈实心"少一次边缘混合；
>   边框宽 ≥ 半边尺寸时退化成一块实心圆角矩形（语义一致）。
> - **边框颜色**是调色板令牌（`Palette::border` 面板 / 分割线；`Palette::border_strong`
>   按钮 / 输入框 / 勾选框），**边框宽度**用 `Theme::with_border_w(w)`（逻辑像素，
>   级联 `panel` / `button` / `input` / `checkbox`；0 = 不画）。
> - `Theme::with_radius(r)` 级联到全部有圆角的子样式：
>   `panel` / `button` / `input` / `checkbox`（取 `r/2`）/ `combo.menu_radius`（取 `min(r, 6)`）。
>
> 实时调参：`eg260818UI` 的「主题调节…」窗口可拖圆角 / 羽化 / 微渐变 / **背景 RGB** /
> **边框 RGB** / **边框宽** / 强调 RGB / **投影模糊宽**，并一键切 dark / light / legacy
> 预设与紧凑 / 标准 / 宽松三档密度。
>
> **投影**（`ShadowStyle`，v0.4 新增）：从面板矩形向外 `blur` 像素铺 `SHADOW_STEPS = 4` 段
> **同心**圆角带，第 `t` 圈外扩 `blur·t` 并偏移 `offset·t`，alpha = `a·(1−t)²`（本体边缘
> 最浓、最外圈为 0，天然抗锯齿）。**完全靠顶点色**——无纹理、无着色器改动，进窗口顶点
> 缓存（内容不变时零开销），与背景同段合批。`offset` 必须**逐环分摊**，只平移内轮廓会在
> 本体正下方留下一条等浓度暗带（看起来像"阴影下方突出"）；`blur <= 0` / 颜色全透明 /
> 退化矩形 ⇒ **零几何**。只挂 `PanelStyle`（窗口 / 面板 / 浮层），薄控件不加投影。

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
| `Esc` | 收起展开的下拉框；否则**取消焦点**（输入框内原有行为不变；应用快捷键需在 `text_focus()` 为 `None` 时处理） |

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
| **多行 TextArea** | `p.text_area(id, &mut String)` / `text_area_at(id, rect, ...)`：Enter 换行、↑/↓ 跨**视觉行**（保持列）、Home/End 行首尾、按内容区宽度自动换行（`create_buffer_wrap`）、**光标/点击/选择按视觉行定位与显示一致**（`Text::visual_lines`）、行距 = `Theme::line_spacing`、跨视觉行选择高亮逐行绘制 |

- 输入框按下时置位 `press_claimed`：窗口/面板**不建立拖拽基准**（选择拖拽优先；窗口从空白/标题区拖动），并清除旧拖拽基准（防"瞬移"）；
- 主题：`InputStyle::sel_bg`（选择高亮色，默认浅蓝 / dark 深蓝）。

### 渲染增强（圆角 / 渐变 / 矢量图标）

| 函数 | 签名 | 说明 |
|---|---|---|
| `rounded_rect_at` | `ui.rounded_rect_at(pos, size, radius, color)` | 圆角矩形背景原语（radius 逻辑像素；CPU 镶嵌成三角形 + 1px 羽化，无纹理） |
| `gradient_rect_at` | `ui.gradient_rect_at(pos, size, gradient)` | **矩形渐变**原语（绝对定位）。`gradient` 接受 `Gradient` 或 `Color`（`Into`） |
| `gradient_rect` | `ui.gradient_rect(size, gradient)` | 同上，但位置来自当前容器游标（随布局流） |
| `icon_at` | `ui.icon_at(pos, size, icon, color)` | **矢量图标**（绝对定位；`size` 为方框，非方形时按 `min(w,h)` 居中等比） |
| `icon` | `ui.icon(size, icon, color)` | 同上，但位置来自当前容器游标——`row` 内连续调用即得工具栏 |
| `Icon` | `Check` / `ChevronUp` / `ChevronDown` / `ChevronLeft` / `ChevronRight` / `Close` / `Grip` / `Warning` | **画出来的几何**（单位方框 `[0,1]²` 内的凸分片，`Icon::parts()` 公开；`Close` = 两条顺时针凸平行四边形拼的 ×，标题栏关闭按钮用） |
| `image_at` | `ui.image_at(pos, size, bg)` | **背景图**（绝对定位；`bg: ImageBg` 决定铺排 / 染色 / 圆角遮罩） |
| `image` | `ui.image(size, bg)` | 同上，但位置来自当前容器游标 |
| `ImageBg` | `new(tex, texel)` + `.fit(..)` / `.radius(..)` / `.tint(..)` | 纹理 uid + **纹素尺寸** + 铺排 + 染色 + 圆角遮罩；`PanelStyle::with_bg_image` 可直接当窗口/面板底图 |
| `ImageFit` | `Stretch` / `Fill` / `Center` / `Tile` | 拉伸 / 等比覆盖（居中裁剪）/ 原始尺寸居中 / 1:1 平铺 |
| `Gradient` | `pure(c)` / `vertical(top, bottom)` / `horizontal(left, right)` / `rotated(from, to, angle)` / `corners(tl, tr, bl, br)` | **四角颜色**（`pub tl/tr/bl/br`）；`From<Color>` 给纯色 |
| `lerp_color` | `lerp_color(a, b, k)` | 颜色线性插值（`Gradient` 构造器与四角采样的基础） |

**矢量图标不需要字体**：`▾` / `✓` / `≡` 这类字符的可用性、宽度、基线全由字体决定
（缺字形会走 fallback，甚至会因字体不同而宽高不一）。`Icon` 改为在单位方框内给出
**凸多边形分片**，由 `crate::tess::push_convex` 扇形三角化 + 按 `Theme::feather`
做边缘羽化（与圆角矩形同一套顶点 alpha 插值机制，`DrawKind::Icon` 走同一条批）。
自定义图标请按 `Icon::parts()` 的形状（单位方框、**凸**、屏幕顺时针）提供分片。

```rust
// 使用前
use rjw_krusie::prelude::*;
// 1) 纯色（等价实心）
ui.gradient_rect_at(pos, size, Color::from_hex("#3af"));               // Color: Into<Gradient>
// 2) 上下 / 左右双色
ui.gradient_rect_at(pos, size, Gradient::vertical(Color::RED, Color::BLUE));
ui.gradient_rect_at(pos, size, Gradient::horizontal(Color::RED, Color::BLUE));
// 3) 任意角度（0° = 下→上，90° = 左→右，逆时针为正）
ui.gradient_rect_at(pos, size, Gradient::rotated(Color::RED, Color::BLUE, 0.5));
// 4) 四角各异（双线性；1D 纹理表达不了）
ui.gradient_rect_at(pos, size, Gradient::corners(a, b, c, d));
// 5) 矢量图标
ui.icon_at(pos, Vec2::splat(16.0), Icon::Check, Color::WHITE);
ui.row(|r| r.icon(Vec2::splat(18.0), Icon::ChevronDown, Color::WHITE)); // 工具栏
// 6) 背景图（`tex` = TextureWrapped::uid，`texel` = 纹素尺寸）
let bg = ImageBg::new(tex, Vec2::new(64.0, 64.0));                     // 默认 Stretch
ui.image_at(pos, size, bg.fit(ImageFit::Fill).radius(8.0));            // 等比覆盖 + 圆角遮罩
ui.image_at(pos, size, bg.fit(ImageFit::Tile));                        // 1:1 平铺（直角）
ui.window("w").style(ui.theme().panel.clone().with_bg_image(bg));      // 当窗口底图
```

**背景图与圆角遮罩为什么能共存**：`Stretch` / `Fill` / `Center` 的 UV 是顶点位置的
**仿射**映射，扇形三角化下的重心插值精确再现它 ⇒ 圆角硬体 + 羽化带上的每个顶点都带
正确的 UV，边缘由羽化带的 alpha 斜坡裁掉（真正的圆角遮罩，不是把直角图片贴上去）。
整个特性**零着色器改动、零额外 draw call**（图片按纹理切段，与字形 / 白纹理各一段；
几何进窗口顶点缓存，跨帧复用）。`Tile` 用逐块四边形实现（1:1，边缘部分块按比例截断
UV），**不支持圆角遮罩**——平铺需要 UV 环绕（`u > 1`），那要求批次携带 `Repeat`
采样器（`UiBatch` 目前不带 `RStates`）；块数超 `MAX_IMAGE_TILES` 时退化为拉伸。

### 取色器（`ColorPicker`）

内联只占一行（色块 + `#RRGGBB` + 展开箭头），点开是**独立置顶窗口**面板：

| 项 | 签名 | 说明 |
|---|---|---|
| `ColorPicker` | `ColorPicker::new(id, &mut Color)` | 主构造（颜色直接写在 `&mut Color` 上，"变没变"由调用方前后比较） |
| `.alpha(on)` | | 面板里多一行 Alpha（默认只 RGB） |
| `.with_hex(&mut String)` | | 可选：把顶部文本框绑到调用方缓冲；**不传则用全局跨帧缓冲** |
| `.popup_width(w)` | | 面板宽（默认 = 内联宽的 1.9 倍与"通道行最小宽"取大） |
| `ColorFormat` | `U8` / `Hex` / `F` | 文本框呈现格式（全局偏好，所有取色器一致） |
| `ColorPickerState` | `UiState::color_picker` | **全局跨帧数据**：`mode` / `text`（替补缓冲）/ `open`（同时只有一个面板）/ HSV 缓存 |
| `format_color` | `format_color(c, mode, with_alpha)` | 按模式呈现（`255, 0, 0` / `#FF00AA` / `1.00, 0.00, 0.00`） |
| `parse_color` | `parse_color(text, mode)` | **自动识别格式**：先按当前模式，再试另两种；`None` = 不可识别（不改颜色） |

面板内容：模式行（u8/HEX/F）+ 文本框（不可识别时右侧出现 `Icon::Warning` 按钮，按下恢复
有效值）+ HSV 区（SV 平面 + 6 段色相条）+ 通道行（颜色滑块 + `NumberInput`）+ 可选 A 行。
**SV 平面 = 一个四角顶点色的圆角矩形**（`[白, 纯色相, 黑, 黑]` 的双线性插值恰好等于
HSV 公式）⇒ 无纹理、无着色器改动、无额外 draw call。实现按职责拆在
`widgets/colorpicker/{format,hsv,state,panel}.rs`（纯函数各自带单测）。

```rust
use rjw_krusie::prelude::*;
ui.add(ColorPicker::new("tint", &mut color).alpha(true));
// 只想要文本解析 / 呈现（不画控件）：
let c = parse_color("#FF00AA", ColorFormat::U8).unwrap();
assert_eq!(format_color(c, ColorFormat::F, false), "1.00, 0.00, 0.67");
```

**渐变不需要纹理**（v0.3 起）：顶点格式 `VertexP3U2C4` 自带 4 分量顶点色，
光栅化器本就做重心插值 ⇒ 一个 quad + 白纹理即可，**管线零改动**。这一决定替代了旧实现
（把渐变烘成 1×64 条纹纹理塞进动态图集）。旧实现的代价：每帧一次 `String` 建 key
（`{t:.3}` 还会静默撞键）、`permanent` 条目让图集**永久无法 `repack_all`**、
每次纹理切换多一次 draw call、且**单轴纹理表达不了四角各异的颜色**。

- **不支持多段 stops**（3+ 停靠点）：四角顶点色是双线性的，无法精确表达多段。
  多段渐变请用 `rjw_text::Gradient`（作用于**文字**，本就支持多段；
  见 `Gradient::glyph_h/glyph_v/line_h/line_v/frame_h/frame_v`）。
- **裁剪保锚**：矩形被裁剪时四角色按其在**原矩形**中的相对位置重采样，
  颜色的空间锚定不变（否则裁剪会让渐变整体平移）。⚠ 重采样前必须把命令的**绝对**
  矩形换算到与裁剪结果相同的**窗口局部**空间（`resample_gradient_local`）——
  混用会让 u/v 整体偏心窗口原点，渐变被平移甚至外推出界。
- **圆角 + 渐变天然共存**：圆角镶嵌直接吃**四角色**（`RoundedRectSpec.corners`），
  所以 `Brush` 的两端色与圆角是同一套顶点色路径，不需要专门着色器，也不需要纹理。
- `GradientAxis` 现在只属于**文字渐变**（`rjw_text::GradientAxis`），
  不再是矩形渐变的参数；`rjw_ui` 根不再导出它（`rjw_ui::text::GradientAxis` 仍可用）。
- 提交分组为 `(win, 图形/文字组, 纹理 uid)`：渐变与圆角都属于**图形组**（白纹理），
  先于文字；
  ⚠ **UI 的 Render2D 必须 `set_sort_mode(SortMode::None)`**（完全按提交顺序绘制）——
  `SortMode::LayerAndStates` 会按纹理 uid 重排而盖住文字（示例 `eg260818UI` 即如此配置）；
- 控件级集成：`Theme` 的 `PanelStyle::radius` / `ButtonStyle::radius` / `InputStyle::radius` /
  `CheckboxStyle.radius`，背景色则统一是 [`Brush`]（纯色 / 两端色渐变，见下节）。

#### 背景刷 `Brush`（主题里的背景渐变）

`PanelStyle::bg` / `ButtonStyle::{bg,bg_hover,bg_pressed}` / `InputStyle::bg` 的类型是
`Brush`（`Color: Into<Brush>` ⇒ 既有 `with_bg(Color::RED)` 调用点不用改）：

```rust
pub enum Brush { Solid(Color), Vertical(Color, Color), Horizontal(Color, Color) }
```

- `Brush::corners() -> [Color; 4]` 是所有绘制路径的统一输入，与圆角天然共存。
- 只有**两端色**：多段 stops 的能力在 `Gradient`（显式原语，支持 `rotated` / 四角各异）
  与 `rjw_text::Gradient`（文字）上；主题默认值要便宜、好维护。
- `Brush::as_solid()` 让"两端同色"退化回纯色路径；`PartialEq<Color>` 让
  `theme.panel.bg == Color::RED` 这类断言可直接写。
- 表面微渐变由 `Palette.bevel` + `bevel_raised` / `bevel_sunken` 从一个表面色派生：
  深色取 0.10（面板/按钮上亮下暗、输入框上暗下亮），浅色取 0.02，`legacy_dark` 取 0。

#### 配色令牌 `Palette`

`Theme::themed(&Palette)` 从一份调色板组装整套主题；`Theme::{light,dark,dark_legacy}`
是它的三个预设。字段按**层次**命名（`surface_dim` < `surface_sunken` 例外 < `surface` <
`surface_raised` < `surface_overlay` < `surface_hover` < `surface_active`），
一套明暗阶梯服务全部控件。换肤只需换一份 `Palette`：

```rust
let mut p = rjw_ui::Palette::dark();
p.accent = Color::rgba_u8(255, 120, 200, 255);
let theme = rjw_ui::Theme::themed(&p);
```

`Theme::palette()` 返回**组装来源**（手工改过字段后不代表实际颜色）。
`Palette::legacy_dark()` + `Theme::dark_legacy()` 精确复刻 v0.3 的硬编码深色配色。

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
| `UiState::widget_occluded_hits` | `state.widget_occluded_hits() -> u32` | 诊断：上帧**命中但被同窗口内更上层控件遮挡而未响应**的次数（控件级遮挡拦截计数） |
| `UiState::press_cancelled_by_window` | `state.press_cancelled_by_window() -> u32` | 诊断：上帧**认领按下后被帧末复核撤销**的次数——命中那一刻被判"没被遮挡"、而帧末完备的遮挡表表明它其实被更高 z 的窗口盖住了（见 `Ui::resolve_widget_press`；应为 0） |
| `UiStats::prologue_us` | `stats.prologue_us`（f64，µs） | 各段**开场**耗时（懒开场 / 冻结输入 / 装载帧级事实 / 建根容器）。与 `finish_us` 一起把 `ui_frame_us` 三分解：`prologue + 应用录制 + finish` |

> 世界坐标调试图元（游戏场景：碰撞盒 / 网格 / 速度矢量）见 `rjw_2d_render::debug_draw`
> （`draw_line` / `draw_rect_outline` / `draw_circle_outline` / `draw_circle_filled` /
> `draw_cross` / `draw_grid`）。示例：`examples/egDebugDraw`（rjw_ui 屏幕空间 + 世界空间 + debug_layout）、
> `examples/eg260818UI`（右上角窗口诊断面板）。
>
> 窗口遮挡（点击穿透）已修复：重叠区域**只有鼠标下最上层窗口**的控件响应——`window_occluded`
> 判定（`hit.rs`），窗口矩形跨帧缓存于 `UiState.window_rects`；`occluded_hits > 0` 即证明
> 背后控件被正确抑制。

### 快速上手

**运行时路径（推荐）**：一帧可开任意多段（`f.ui(theme)`），位置随意。

```rust
fn update(&mut self, ctx: &mut Ctx) {
    let Some(mut f) = ctx.frame() else { return };
    f.draw().sprite(...);                       // 世界层
    let mut ui = f.ui(Theme::dark());           // 段 1
    ui.pack_at(Vec2::new(16.0, 16.0), PackSide::Top, |p| {
        if p.button("start", "开始游戏").clicked() { /* ... */ }
        self.volume = p.slider("vol", 0.0..=1.0, self.volume);
        if p.checkbox("fs", "全屏", self.fs).toggled() { self.fs = !self.fs; }
        p.text_input("name", &mut self.name);
    });
    ui.finish();                                // 段收尾（可省略：作用域结束即收尾）
    f.text(|t| { /* 世界文本（段之间随便交错） */ });
    let mut hud = f.ui(Theme::dark());          // 段 2（同一帧）
    hud.label_at(Vec2::new(16.0, 690.0), "HUD");
    hud.finish();
    f.submit(&mut self.cam, Clear::color(Color::rgb(0.05, 0.05, 0.08)));
}
```

**低层路径**（自己持有 `UiState`，自定义 `base_layer` / 复用别的 `Text`）：

```rust
use rjw_ui::{IdAbsolute, PackSide, Theme, Ui, UiState};

let mut state = UiState::new();
state
    .radio_groups
    .insert("diff".into(), IdAbsolute::from("diff_normal")); // 默认选中

// 每帧（window/font 来自主循环；输入设备经 capture 快照；相机/渲染器延迟到收尾）：
state.begin_frame();                            // 帧级账由调用方负责（每帧一次）
let mut ui = Ui::begin(window, &mut font, &mut state)
    .capture(&ctx.mouse, &ctx.keyboard)
    .theme(Theme::dark()).build();

ui.pack_at(Vec2::new(16.0, 16.0), PackSide::Top, |p| {
    if p.button("start", "开始游戏").clicked() { /* ... */ }
    volume = p.slider("vol", 0.0..=1.0, volume);
    if p.checkbox("fs", "全屏", fs).toggled() { fs = !fs; }
    p.text_input("name", &mut name);
});
ui.end_frame(r2d);                              // 帧收尾（焦点导航/描边 + 光标 + 统计 + 提交）
```

> 约定：交互控件 ID 必须稳定；顶层 pack 控件（`label`/`button`/…）经**根容器**（`build()`
> 内建，可用宽 = 视口宽）直接流式堆叠，绝对定位用 `*_at`；控件坐标 = 屏幕**逻辑**像素
> （`.scale_factor` 设置 DPI，不设置则等于物理像素）；文本输入支持中文 IME
> （`ctx.keys().ime_commits()` / `ime_preedit()`，候选框跟随光标）；输入框聚焦时用
> `ui.state().text_focus()` 屏蔽应用快捷键；
> 控件文本排版缓冲自持于 `UiState.text_buffers`（`CachePolicy::User`，不推入 `rjw_text` LRU）；
> **独立 UI 渲染**：UI 录到 UI 层自己的 Render2D（`set_sort_mode(SortMode::None)` 关闭排序），与世界
> `encode` 合并提交（一次 present）；`Ui::finish` 按 `(win, depth, 图形/文字, 录制序)`
> 免排序（win + depth 分桶）**逐段**提交，`Ui::end_frame` 做帧收尾。

---

*还想看更多？源码在 `crates/rjw_*/src/`，目录与本文一一对应。*
