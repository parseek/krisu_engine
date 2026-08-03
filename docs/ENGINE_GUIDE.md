# krisu_engine（工作区名 `krusie`）—— 使用与维护指南

> 一份给**人**和**AI**共同阅读的引擎手册。
> 所有例子以当前工作区（`c:\Repos\krusie`）实际可编译的 API 为准。
> 若你是 AI 助手：改代码前先读「⚠️ 易混淆概念」与「对 AI 的维护约定」两节，能避免绝大多数返工。

---

## 0. 目录

1. [引擎是什么 / 模块地图](#1-引擎是什么--模块地图)
2. [快速上手：最小程序](#2-快速上手最小程序)
3. [⚠️ 坐标系（最容易搞错）](#3-坐标系最容易搞错)
4. [绘制模型：Render2D 的批处理管线](#4-绘制模型render2d-的批处理管线)
5. [Layer 语义与 y-sort 惯用法](#5-layer-语义与-y-sort-惯用法)
6. [纹理与合批](#6-纹理与合批)
7. [渲染状态（RStates）与 Builder 责任链](#7-渲染状态rstates与-builder-责任链)
8. [输入：键盘 / 鼠标](#8-输入键盘--鼠标)
9. [Transform2D 变换](#9-transform2d-变换)
10. [颜色：Color 与 ColorF64](#10-颜色color-与-colorf64)
11. [时间：DeltaTimer](#11-时间deltatimer)
12. [窗口与事件循环](#12-窗口与事件循环)
13. [视口 / 缩放 / 高 DPI](#13-视口--缩放--高-dpi)
14. [性能与内存约定](#14-性能与内存约定)
15. [对 AI 的维护约定](#15-对-ai-的维护约定)
16. [快速速查表](#16-快速速查表)

---

## 1. 引擎是什么 / 模块地图

`krusie` 是一个 **Rust + wgpu (30.0.0)** 的 2D 游戏/渲染引擎（视觉验证为主的工作区，含可运行 examples）。

```
crates/
├─ rjw_main        # 入口：run_app(App) + 事件循环 + 窗口 + MainContext(键盘/鼠标/计时)
├─ rjw_render      # 底层渲染上下文：RenderContext / 纹理 TextureWrapped / wgpu 重导出
├─ rjw_2d_render   # ★ 2D 批渲染器：Render2D / SpriteRect / Mesh / RStates / 分页实例缓冲 / 统一管线
├─ rjw_transform   # Transform2D + Camera2D（正交相机、VP 矩阵、坐标转换）
├─ rjw_color       # Color(f32) / ColorF64(f64) + 常用常量（RED/GREEN/...）
├─ rjw_keyboard    # 键盘输入 → KeyState
├─ rjw_keystate    # KeyState 边沿状态机（pressed/edge/true_edge）
├─ rjw_mouse       # 鼠标位置/增量/滚轮/按钮状态
└─ rjw_time        # DeltaTimer（帧间隔 dt 与 FPS）

examples/
├─ eg260729        # 最小清屏示例（手动 RenderPass）
├─ eg260731        # Render2D 精灵/多边形/mesh 能力演示
└─ eg260731RPG     # ★ 综合 RPG：y-sort、波次系统、相机跟踪、程序化纹理
```

**最核心概念一条线**：

```
App (impl rjw_main::App)
 └─ on_init: RenderContext::new(window) → Render2D::new(render)
 └─ about_to_wait（每帧）:
     读输入(键盘/鼠标) → 更新逻辑 → 摆相机(Camera2D)
     → render2d.set_mvp(cam.vp_matrix())
     → 录制绘制命令(add_sprite2d* / add_mesh / add_polygon_*) + 可选链式 RStates
     → render2d.render(&ClearConfig)
```

---

## 2. 快速上手：最小程序

```rust
use rjw_main::*;
use rjw_render::{RenderConfig, RenderContext, wgpu};
use rjw_transform::{Camera2D, Transform2D};
use rjw_2d_render::{ClearConfig, Render2D, SpriteRect};
use rjw_color::Color;
use glam::Vec2;

struct App {
    render: Option<RenderContext>,
    r2d: Option<Render2D>,
    cam: Camera2D,
}

impl App for App {
    fn primary_window_attrib(&self) -> WindowAttributes {
        WindowAttributes::default()
            .with_title("my app")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
    }

    fn on_init(&mut self, ctx: &mut MainContext) {
        let window = ctx.primary_window().expect("window");
        self.render = Some(RenderContext::new(window, &RenderConfig::default()));
        let render = self.render.as_ref().unwrap();
        self.r2d = Some(Render2D::new(render));
        let (w, h) = render.size();
        let mut cam = Camera2D::new(Vec2::new(w as f32, h as f32));
        cam.set_vp(Vec2::new(w as f32, h as f32), Vec2::ZERO);
        self.cam = cam;
    }

    fn on_resized(&mut self, _ctx: &mut MainContext, width: u32, height: u32) {
        if let Some(r) = &mut self.render { r.resize(width, height); }
        self.cam.set_vp(Vec2::new(width as f32, height as f32), Vec2::ZERO);
    }

    fn about_to_wait(&mut self, ctx: &mut MainContext) {
        if ctx.keyboard.get(KeyCode::Escape).down_edge() { ctx.request_exit(); }
        let r2d = self.r2d.as_mut().unwrap();
        r2d.set_mvp(self.cam.vp_matrix());
        // 在屏幕中心画一个 100×100 绿色方块（世界坐标）
        // add_sprite2d_solid 返回 Sprite2DBuilder；不链式调用 = 使用默认渲染状态
        r2d.add_sprite2d_solid(
            SpriteRect::from_texture(Vec2::splat(-50.0), Vec2::splat(100.0)),
            Color::GREEN,
            Transform2D::default(),
            0.0,
        );
        r2d.render(&ClearConfig { color: Some(wgpu::Color { r: 0.1, g: 0.1, b: 0.2, a: 1.0 }), depth: None, stencil: None });
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    rjw_main::run_app(App { render: None, r2d: None, cam: Camera2D::default() })
}
```

---

## 3. 坐标系（最容易搞错）

### 3.1 世界 / 屏幕坐标系（`Camera2D` 规范）

```
        -y (上)
         |
  -x ---- O ---- +x (右)
         |
        +y (下)     ← 注意：Y 正方向是【向下】！
```

| 概念 | 说明 |
|---|---|
| 原点 `(0,0)` | 位于 **视口中心**（不是左上角！） |
| `X+` | 向右 |
| `Y+` | **向下**（与数学惯例相反，与屏幕像素一致） |
| 世界单位 | 像素（一张 32×32 纹理贴到 `SpriteRect::from_texture` 的 32×32 世界区域，1:1） |
| 视口中心 | 等于 `Camera2D.position`（相机所在的世界点） |

> ⚠️ **最易混淆**：写 `pos = Vec2::new(x, y)` 时，`y` 增加 = 往下走。
> 键盘「W 上移」应写 `pos.y -= 速度`；「S 下移」应写 `pos.y += 速度`。
> 参考 `examples/eg260731RPG` 中 W→`dir.y -= 1` 的写法。

### 3.2 `Camera2D` 字段与行为

```rust
pub struct Camera2D {
    pub position: Vec2,   // 相机中心（世界坐标）—— 即窗口中心对应的世界点
    pub rotation: f32,    // 旋转（弧度）
    pub zoom: Vec2,       // 缩放（x/y 可非均匀）
    pub viewport_pos: Vec2,  // 视口左上角（窗口像素）
    pub viewport_size: Vec2, // 视口尺寸（像素）
}
```

- `Camera2D::new(window_size_px)`：建一个覆盖整窗的相机；**之后必须 `set_vp(size, pos)`** 才能用正确投影。
- `vp_matrix() = projection_matrix() * view_matrix()`，**列主序**，可直接喂给 `render2d.set_mvp(...)`（无需转置）。
- `screen_to_world(screen_px)` / `world_to_screen(world)`：屏幕像素 ↔ 世界坐标互转。
- `world_transform()`：把相机当作 `Transform2D`（用于 UI 反父级运算）。

### 3.3 窗口中心 ↔ 世界

**游戏画面在窗口中心** = 相机锁定玩家：

```rust
self.cam.position += (player.pos - self.cam.position) * (1.0 - (-20.0 * dt).exp());
```

---

## 4. 绘制模型：Render2D 的批处理管线

```
【每帧】  add_sprite2d* / add_mesh / add_polygon_*（命令录制，返回 Builder）
              │ (可选链式 .blend(...).depth_test(...) 设置 RStates)
              │ (Builder Drop → push 到 DrawCommandQueue)
              ▼
        Render2D::render(&ClearConfig)
              │
   ① prepare()：sort_layer_then_states() 排序
   ② RStates resolve（None → default_rstates）
   ③ 实例数据按 MAX_INSTANCES_PER_DRAW(8192) 分页
   ④ draw()：按 DrawOp.rstates 从管线缓存取/创建管线 → 逐页绑定 → draw_indexed
   ⑤ 提交并呈现
```

### 4.1 统一管线架构

Sprite 与 Mesh **共用同一 `vs_main` 入口 + slot0(顶点)/slot1(实例) 布局**。

- **Sprite**：slot1 绑定实例页缓冲 → `draw_indexed(quad_indices, 0, N_instances)`
- **Mesh**：slot1 绑定"身份实例缓冲"（mesh_tl=0, mesh_wh=1, model=I）→ `draw_indexed(mesh_indices, 0, 1)` —— 等效于非实例化直通 VP

渲染状态（Blend / DepthStencil / Cull / Polygon / FrontFace / Conservative）全部**按 RStates 从管线缓存中自动获取或创建**，无需手动管理管线。

### 4.2 两种绘制类别

| 类别 | 方法 | Builder | 说明 |
|---|---|---|---|
| **Sprite（贴纹理）** | `add_sprite2d(rect,color,transform,layer,&tex)` | `Sprite2DBuilder` | 同纹理+同 RStates 合批 |
| **Sprite（纯色）** | `add_sprite2d_solid(rect,color,transform,layer)` | `Sprite2DBuilder` | 内部用 1×1 白纹理 |
| **Mesh** | `add_mesh(verts, tris, color, layer)` | `MeshBuilder` | 世界坐标顶点直通 VP |
| **Mesh（便捷）** | `add_polygon_fan` / `add_polygon_strip` / `add_mesh_fn*` | `MeshBuilder` | 画圆、线、任意网格 |

### 4.3 `SpriteRect`（位置/大小/UV）

```rust
SpriteRect {
    mesh_tl: Vec2,   // 世界坐标左上角
    mesh_wh: Vec2,   // 世界尺寸
    uv_tl:   Vec2,   // 归一化 UV 左上
    uv_wh:   Vec2,   // 归一化 UV 尺寸
}
```

- `from_texture(tl, wh)`：整张纹理铺满。
- `from_texture_px(tl, wh, uv_tl_px, uv_wh_px, inv_tex_wh)`：按像素取纹理子区域。

### 4.4 `ClearConfig`

```rust
ClearConfig { color: Option<wgpu::Color>, depth: Option<f32>, stencil: Option<u32> }
```
`color: Some(...)` 清屏 / `None` 保留旧内容。深度/模板需要时自动建纹理。

---

## 5. Layer 语义与 y-sort 惯用法

### 5.1 Layer 是「数值小的先画」

```
layer = 0      最早画（最底层）
layer = 10     后画
layer = 100    最后画（最顶层）
```
Sort 是稳定的 (layer, states) 排序——states 包含 RStates(u64) + texture_uid。
UI 应给一个**很大的固定值**（如 `1e7`），避免被 y-sort 世界坐标覆盖。

### 5.2 y-sort：RPG 纵深感的标准做法

想让「屏幕下方的世界物体盖住上方的物体」：

```rust
const LAYER_Y_SORT_BASE: f32 = 10.0;
fn y_layer(foot_y: f32) -> f32 { LAYER_Y_SORT_BASE + foot_y }

// 绘制时传 foot_y（脚底世界 Y）：
render2d.add_sprite2d(rect, color, tf, y_layer(entity.foot_y), &tex);
```

引擎对 (layer, states) 排序后，Y 大（靠下）的物体自动盖住 Y 小（靠上）的物体。同 Y 的细节遮挡用小数偏移。

---

## 6. 纹理与合批

- 创建：`render2d.create_texture(label, &rgba8_data, w, h)`（RGBA8，`len == w*h*4` 否则 panic）。
- 合批：**同一纹理 + 同一 RStates 的连续 Sprite 自动合批**。
- 纹理池：`Render2D.textures` 持有 `Arc`，防止释放。
- 1×1 白色纹理：纯色 Sprite 使用 `white_texture`。

---

## 7. 渲染状态（RStates）与 Builder 责任链

`RStates` 是一个 u64 bitfield，涵盖 6 个渲染控制域：

| 域 | 字段 | 示例 |
|---|---|---|
| Blend | `BlendMode` (Alpha/Additive/Multiply/Premultiplied) | `.blend(Additive)` |
| Sampler | mag/min/mip filter + addr_u/v/w | `.samp_addr_u(Repeat).samp_mag(Nearest)` |
| Cull+Raster | cull + polygon + front_face + conservative | `.cull(Back).polygon(Line)` |
| Depth | test + write + compare | `.depth_test(true).depth_write(true)` |
| Stencil | test + write + compare | `.stencil_test(true).stencil_compare(Always)` |

### 7.1 三级控制

| 级别 | 使用方式 | 作用范围 |
|---|---|---|
| **全局默认** | `render2d.default_blend(Additive).default_depth_test(true).set_mvp(...)` | 所有不链式调用的 add_* |
| **单条绘制** | `render2d.add_sprite2d(...).blend(Multiply)` | 该条命令 |
| **批量设置** | `.blend_state(BlendDesc{...}).samp_state(SamplerDesc{...}).depth_state(DepthState{...})` | 同上 |

### 7.2 Builder 使用

```rust
// 不链式 = 使用 default_rstates（默认 Alpha Blend + Linear + No Cull）
render2d.add_sprite2d(rect, Color::WHITE, tf, 0.0, &tex);

// 链式覆盖
render2d.add_sprite2d(rect, Color::WHITE, tf, 0.0, &tex)
    .blend(BlendMode::Additive)
    .samp_addr_u(AddressMode::Repeat)
    .samp_mag(FilterMode::Nearest);

// Mesh 还可以 set_texture（覆盖默认白色纹理）
render2d.add_polygon_fan(&verts, Color::CYAN, 96.0)
    .set_texture(&tex)
    .blend(BlendMode::Multiply);

// 批量设置
use rjw_2d_render::{BlendDesc, BlendMode, DepthState, CompareFunc};
render2d.add_sprite2d(rect, Color::WHITE, tf, 0.0, &tex)
    .depth_state(DepthState { test: true, write: true, compare: CompareFunc::Less });

// 全局默认（责任链风格，返回 &mut Render2D）
render2d
    .default_blend(BlendMode::Additive)
    .default_depth_test(true)
    .default_depth_write(true)
    .default_depth_compare(CompareFunc::Less);
```

### 7.3 MeshBuilder 特有的 `.set_texture()`

```rust
// Mesh 默认白色纹理；set_texture 覆盖
render2d.add_mesh(&verts, &tris, Color::WHITE, 96.0)
    .set_texture(&my_tex)
    .blend(BlendMode::Alpha);
```

### 7.4 重要类型

| 类型 | 说明 |
|---|---|
| `RStates` | u64 bitfield，含全部 6 个控制域 |
| `BlendMode` | Alpha / Additive / Multiply / Premultiplied |
| `FilterMode` | Linear / Nearest |
| `AddressMode` | ClampToEdge / Repeat / MirrorRepeat |
| `CullMode` | None / Front / Back |
| `PolygonMode` | Fill / Line / Point |
| `FrontFaceWinding` | Ccw / Cw |
| `CompareFunc` | Never / Less / Equal / LessEq / Greater / NotEq / GreaterEq / Always |
| `BlendDesc` / `SamplerDesc` / `RasterState` / `DepthState` / `StencilState` | 批量设置描述符 |

---

## 8. 输入：键盘 / 鼠标

### 8.1 键盘——`KeyState`（重点：边沿）

`ctx.keyboard.get(KeyCode::KeyW)` 返回 `KeyState`：

| 方法 | 含义 |
|---|---|
| `.pressed()` | 当前是否按住 |
| `.released()` | 是否没按 |
| `.down_edge()` | **本轮"按下"边沿**（按下的那一刻触发一次，可能重复） |
| `.up_edge()` | **本轮"松开"边沿**（同上） |
| `.true_edge()` / `.down_true_edge()` | 真实边沿（按住时不会反复触发） |
| `.sudden_up()` | 突然松开（未在上一帧处于按下状态） |

> ⚠️ **攻击/跳跃等瞬时操作必须用 `down_edge()`**，否则每帧触发多次。

### 8.2 鼠标

```rust
ctx.mouse.get_mouse_position()                        // (x,y) 窗口像素
ctx.mouse.get_mouse_delta()                           // 本帧移动增量
ctx.mouse.get_mouse_button_state(MouseButton::Left)   // KeyState
ctx.mouse.get_wheel_delta()                           // (x, y) 滚轮
ctx.mouse.get_pixel_wheel()                           // 触控板像素级滚轮
ctx.mouse.in_window()                                 // 是否在窗口内
```

---

## 9. Transform2D 变换

```rust
pub struct Transform2D {
    pub pos:  Vec2,   // 位置（世界）
    pub scale: Vec2,  // 缩放
    pub rotation: f32, // 旋转（弧度）
}
```

- 构建器：`with_pos(..)` / `with_scale(..)` / `with_rot(..)` / `with_move_by` / `with_walk_by` / `with_scale_by` / `with_rotate_by`。
- **旋转中心**：旋转绕 `pos` —— 精灵要「绕中心转」，矩形写 `mesh_tl: Vec2::splat(-w/2)`。
- `transform_point` / `inverse_transform_point`：局部 ↔ 父空间点变换。

---

## 10. 颜色：Color 与 ColorF64

| 类型 | 存储 | 常用构造 | 用途 |
|---|---|---|---|
| `Color` | `f32` ×4 | `rgba(r,g,b,a)`、`rgba_u8(..)`、常量 `RED/GREEN/...` | 绘制命令 |
| `ColorF64` | `f64` ×4 | `rgba(f64,..)` | 可 `.into()` `wgpu::Color`（清屏） |

---

## 11. 时间：DeltaTimer

```rust
ctx.timer.dt()              // 帧间隔秒（Duration）
ctx.timer.dt().get_f32()    // 帧间隔秒（Duration）的 f32 转换
ctx.timer.get_fps()         // 当前 FPS
```

每帧用 `dt` 做位移：`pos += vel * dt`；建议 `dt.min(0.05)` 防跳变。

---

## 12. 窗口与事件循环

- 入口：`run_app(App::new())`，`App` trait 必须实现 `on_init`、`about_to_wait`；`on_resized` 可选。
- `MainContext`：`keyboard` / `mouse` / `timer` / `primary_window()` / `request_exit()`。
- `on_init` 中创建 `RenderContext` + `Render2D`（`RenderContext` 必须比 `Render2D` 活得久）。
- `on_resized`：`render.resize(w,h)` + 更新相机视口。

---

## 13. 视口 / 缩放 / 高 DPI

| 概念 | 说明 |
|---|---|
| **LogicalSize** | 窗口逻辑尺寸 |
| **物理像素** | 实际屏幕像素 = 逻辑 × DPI scale |
| **`render.size()`** | **物理像素**，必须用它建相机视口，否则高 DPI 下画面偏移 |
| 相机 `viewport_size` | 物理像素 |

**缩放**（滚轮）：
```rust
cam.zoom *= Vec2::splat(1.1_f64.powf(wheel.1) as f32);
```

---

## 14. 性能与内存约定

- `Render2D` 内部 `buf_*` 全部常驻复用（`clear()` 只清长度不释放）。
- **实例缓冲是"页池"**：单帧总实例数可远超 8192，自动分页；**不要自己裁减数量去凑**。
- **统一管线缓存**：`DrawPage` 按 `RStates::raw()` keys 缓存 `RenderPipeline`，首次遇新状态时创建、后续帧直接命中。HashMap 常驻，Query 事件循环保持不变。
- **builder 不产生堆分配**：`Sprite2DBuilder` / `MeshBuilder` 均为栈上 struct，Drop 时直接转移到 `DrawCommandQueue` 的 Vec。
- 页池按需一次性增长、永久复用。
- Mesh 顶点走 u16 索引，单帧顶点数 ≤ 65535。

---

## 15. 对 AI 的维护约定

给接手改代码的 AI 助手的清单：

1. **坐标系**：Y+ 向下。写"向上移动"用 `y -=`。
2. **逻辑像素 ≠ 物理像素**：相机一律用 `render.size()`（物理）。
3. **瞬时操作用 `down_edge()`**；持续操作用 `pressed()`。
4. **透明覆盖问题（大坑）**：`Queue::write_buffer` 在 `submit` 前会**全部先执行**——所以引擎用"页池"：每页只写一次、绑定对应页。
5. **Layer 数值小先画**；RPG 里实体/地形用 y-sort 动态 layer，UI 用 ≥1e7 固定层。
6. **RStates resolve**：`prepare()` 中 `States.rstates: None` → `default_rstates`；`Some(r)` → 直接用 `r.raw()`。
7. **Builder 是责任链，Drop 自动 push**：`add_*` 返回 builder，不链式调用也自动 push（`rstates: None`）。不要手动 push。
8. **统一管线**：不再有 `sprite_pipeline` / `mesh_pipeline` 分支。所有绘制走 `get_or_create_pipeline(rst_raw)`，Mesh 绑定 identity instance buffer。
9. **管线缓存 key = RStates::raw()**：u64 哈希，同一个 raw 值只创建一次管线。
10. 改 `rstates.rs` 的 bitfield 布局时务必更新 `to_blend()` / `to_depth_stencil()` / `to_cull()` 等解包方法。
11. 纹理数据长度必须 `w*h*4`。
12. 改公共 crate 后，务必 `cargo check --workspace`。

---

## 16. 快速速查表

| 要做的事 | 代码 |
|---|---|
| 键盘 W 按住 | `ctx.keyboard.get(KeyCode::KeyW).pressed()` |
| 空格"按下那一下" | `ctx.keyboard.get(KeyCode::KeySpace).down_edge()` |
| 鼠标左键点击 | `ctx.mouse.get_mouse_button_state(MouseButton::Left).down_edge()` |
| 鼠标世界坐标 | `cam.screen_to_world(Vec2::new(mx, my))` |
| 绕中心旋转的精灵 | `SpriteRect{mesh_tl: Vec2::splat(-w/2),...}` + `pos` 在中心 |
| 让画面跟随玩家 | `cam.position += (player.pos - cam.position) * (1-exp(-k*dt))` |
| UI 固定最顶层 | `layer = 1e7` |
| 退出 | `Esc` → `ctx.request_exit()` |
| 加性混合 Sprite | `.blend(BlendMode::Additive)` |
| 全局启用深度测试 | `render2d.default_depth_test(true).default_depth_write(true)` |
| Mesh 贴纹理 | `.set_texture(&tex)` |
| 设置采样器重复 | `.samp_addr_u(AddressMode::Repeat).samp_addr_v(AddressMode::Repeat)` |
| 延迟丢弃 builder（显式绑定变量） | `let _b = render2d.add_sprite2d(...).blend(...);` —— `_b` 在作用域结束时 Drop |

---

*遇到报错请优先怀疑：坐标系方向、物理/逻辑像素、`down_edge` vs `pressed`、页池覆盖（勿改回单缓冲）、layer 数值大小。这五类占引擎使用失误的 95%。*