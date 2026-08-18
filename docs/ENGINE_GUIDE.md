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
7. [运行时图集：DynamicAtlas 与 StaticAtlas（rjw_atlas）](#7-运行时图集dynamicatlas-与-staticatlasrjw_atlas)
8. [渲染状态（RStates）与 Builder 责任链](#8-渲染状态rstates与-builder-责任链)
9. [输入：键盘 / 鼠标](#9-输入键盘--鼠标)
10. [Transform2D 变换](#10-transform2d-变换)
11. [颜色：Color 与 ColorF64](#11-颜色color-与-colorf64)
12. [时间：DeltaTimer](#12-时间deltatimer)
13. [窗口与事件循环](#13-窗口与事件循环)
14. [视口 / 缩放 / 高 DPI](#14-视口--缩放--高-dpi)
15. [性能与内存约定](#15-性能与内存约定)
16. [对 AI 的维护约定](#16-对-ai-的维护约定)
17. [快速速查表](#17-快速速查表)
18. [UI（rjw_ui）](#18-uirjw_ui)

---

## 1. 引擎是什么 / 模块地图

`krusie` 是一个 **Rust + wgpu (30.0.0)** 的 2D 游戏/渲染引擎（视觉验证为主的工作区，含可运行 examples）。

```
crates/
├─ rjw_main        # 入口：run_app(App) + 事件循环 + 窗口 + MainContext(键盘/鼠标/计时)
├─ rjw_render      # 底层渲染上下文：RenderContext / 纹理 TextureWrapped / 静态网格 MeshData / 泛型注册表 TypedRegistry / wgpu 重导出
├─ rjw_2d_render   # ★ 2D 批渲染器：Render2D / SpriteRect / Mesh / StaticMesh / RStates / 分页实例缓冲 / 统一管线
├─ rjw_atlas       # ★ 运行时图集：DynamicAtlas（Guillotine 空闲矩形 + 寿命 + clamp_margin + 去碎片重排）+ StaticAtlas（TOML）
├─ rjw_text        # ★ 文本渲染：cosmic-text 排版 + swash 光栅化 + DynamicAtlas 字形缓存 + 责任链
├─ rjw_ui          # ★ UI：hybrid 模式（立即外观 + ID 持久状态）+ DOM 风格自动尺寸 + Tkinter 布局（pack/grid/place）
├─ rjw_transform   # Transform2D + Camera2D（正交相机、VP 矩阵、坐标转换）
├─ rjw_color       # Color(f32) / ColorF64(f64) + 常用常量（RED/GREEN/...）
├─ rjw_keyboard    # 键盘输入 → KeyState（含 get_chars 字符输入）
├─ rjw_keystate    # KeyState 边沿状态机（pressed/edge/true_edge）
├─ rjw_mouse       # 鼠标位置/增量/滚轮/按钮状态
├─ rjw_time        # DeltaTimer（帧间隔 dt 与 FPS）
├─ rjw_collision   # 碰撞（矩形相交等）
└─ rjw_tilemap     # 瓦片地图（chunk + 组 + 相机剔除）

examples/
├─ eg260729           # 最小清屏示例（手动 RenderPass）
├─ eg260731           # Render2D 精灵/多边形/mesh 能力演示
├─ eg260731CustomDraw # add_custom / CustomDraw 逃逸舱口（自建管线三角形）
├─ eg260731RPG        # ★ 综合 RPG：y-sort、波次系统、相机跟踪、程序化纹理、静态地形（石头/花经 StaticMesh 合批）
├─ eg260810TextChain  # ★ 文本责任链（TextLayout → TextRender）演示
├─ egTilemap          # ★ 瓦片地图（chunk + 相机剔除 + 屏幕固定 HUD）
└─ eg260818UI         # ★ rjw_ui 示例：pack 菜单 + grid 背包 + place 状态栏 + 输入框/滑块/单选
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

Sprite、StaticMesh 与动态 Mesh **共用同一 `vs_main` 入口 + slot0(顶点)/slot1(实例) 布局**。

- **Sprite**：顶点用注册的四边形网格（`quad_mesh_id`），slot1 绑实例页缓冲 → `draw_indexed(quad_indices, 0, N_instances)`
- **StaticMesh**：顶点用 `MESHES` 注册表中用户网格，slot1 绑实例页缓冲 → `draw_indexed(mesh_indices, 0, N_instances)` —— 同 mesh_id 的实例自动合批
- **动态 Mesh（add_mesh / add_polygon_*）**：顶点每帧上传 `draw_page.mesh_vb/mesh_ib`，slot1 绑 identity 实例（`mesh_tl=0, mesh_wh=1, mesh_pos = pos 直通`）→ `draw_indexed(段索引范围, 0, 1)`

渲染状态（Blend / DepthStencil / Cull / Polygon / FrontFace / Conservative / **Sampler**）全部**按 RStates 从管线缓存中自动获取或创建**，无需手动管理管线。
**采样器由 RStates 位域（bits 8..24）驱动**：`.samp_mag(Nearest)` / `.samp_addr_u(Repeat)` 会真正创建对应 GPU 采样器（`Render2D` 内部缓存）；bind group 由 `Render2D` 按 `(tex_uid, samp_key)` 缓存。

### 4.2 绘制类别

| 类别 | 方法 | Builder | 说明 |
|---|---|---|---|
| **Sprite（贴纹理）** | `add_sprite2d(rect,color,transform,layer,&tex)` | `Sprite2DBuilder` | 同纹理+同 RStates 合批 |
| **Sprite（纯色）** | `add_sprite2d_solid(rect,color,transform,layer)` | `Sprite2DBuilder` | 内部用 1×1 白纹理 |
| **Mesh（动态）** | `add_mesh(verts, tris, color, layer)` | `MeshBuilder` | 世界坐标顶点直通 VP，每帧上传 |
| **Mesh（便捷）** | `add_polygon_fan` / `add_polygon_strip` / `add_mesh_fn*` | `MeshBuilder` | 画圆、线、任意网格 |
| **StaticMesh** | `add_static_mesh(mesh_id,color,transform,layer,&tex)` | `StaticMeshBuilder` | 注册表网格 + 实例化合批（GPU 顶点常驻） |

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

### 4.5 外部自定义绘制（`add_custom` / `CustomDraw`）

引擎的**逃逸舱口**：在 `Render2D` 自带的统一管线之外，注入任意原生 wgpu 绘制调用，如自定义 shader、线框调试、后处理或特殊顶点格式。

#### 核心 API

| 项 | 说明 |
|---|---|
| `CustomDraw` trait | `fn draw(&self, pass: &mut wgpu::RenderPass<'_>)`；`Send + Sync` 约束 |
| blanket impl | 闭包 `Fn(&mut wgpu::RenderPass) + Send + Sync` 自动实现该 trait |
| `add_custom(layer, cd)` | 返回 `CustomBuilder`，可链式设 RStates 参与排序 |
| `CustomBuilder` | 与 `Sprite2DBuilder`/`MeshBuilder` 相同的责任链（无 `.set_texture()`） |

#### 用法

**👉 完整可运行示例见 [`examples/eg260806CustomDraw/`](../examples/eg260806CustomDraw/src/main.rs)**（`cargo run -p eg260806CustomDraw`）：演示结构体形式（`Tri` 自建管线）+ 闭包形式 + 与引擎 Sprite 混排 3 层。

```rust
// ① 闭包形式（最常用）
r2d.add_custom(1.0, |pass| {
    // pass 已由引擎打开——不要调用 begin_render_pass / end
    // 可以 set_pipeline、set_vertex_buffer、draw 等
});

// ② 结构体形式（可复用、持有共享资源）
#[derive(Clone)]
struct Wireframe { mdl: wgpu::RenderPipeline }
impl rjw_2d_render::CustomDraw for Wireframe {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.mdl);
        // ...
    }
}
r2d.add_custom(96.0, Wireframe { mdl });
```

> 💡 **最小完整使用模式**（建管线 → 建顶点 → draw，详见 `eg260731CustomDraw`）：
>
> ```rust
> // ① 自建管线（layout 可无 bind group）
> let shader = device.create_shader_module(...);
> let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
>     bind_group_layouts: &[],           // 不需要 bind group 时传空
>     immediate_size: 0,
> });
> let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
>     layout: Some(&layout),
>     // ...目标格式 = render.format()（surface_format），其余按需
> });
>
> // ② 顶点缓冲（设备缓冲 + bytemuck 上传）
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

> ⚠️ **pass 生命周期**：`add_custom` 的闭包在 `render()` / `flush()` 内部的 `draw()` 阶段调用。此时 RenderPass 已 `begin`，请在闭包内**使用**，而不要再 `begin_render_pass`。

#### 排序与执行时机

- 排序键 = `(layer, states)`，与 Sprite/Mesh 一致。`CustomBuilder` 链式的 `.blend(...)` 等 RStates 决定其在同一 layer 内相对 Sprite/Mesh 的先后。
- 每帧 `render()` 结束时 `buf_custom_draws.clear()`——**不要在闭包中缓存跨帧状态**；需要持久资源请 `Arc` 捕获并在 `CustomDraw` 结构体中保存。
- `flush()` 同样执行 custom draws（用于用户自建 pass 的内部子 pass）。

#### 与引擎管线的边界

| 场景 | 推荐 |
|---|---|
| 普通 Sprite/Mesh（引擎已覆盖） | 直接用 `add_sprite2d*` / `add_mesh*`（可合批、状态缓存） |
| 特殊混合/着色、调试线框、后处理 | `add_custom` 注入原生 wgpu |
| 完全独立于 `Render2D` 的渲染 | 用 `begin_frame()` / `flush()` + 自建 pass，或直接在事件循环自建 encoder |

### 4.6 静态网格 StaticMesh（GPU 顶点常驻 + 实例化合批）

当一批元素**位置/纹理/层级固定、不参与实体 y-sort** 时（地图装饰如石头、花、栅栏…），应使用 `register_mesh` + `add_static_mesh*` 静态化：

- **`MeshData`**（`rjw_render`）在 GPU 上持有顶点/索引缓冲，`register_mesh` 注册进全局 `MESHES`，返回 `mesh_id`。
- **`add_static_mesh(mesh_id, color, transform, layer, &tex)`** 每帧只提交一个轻量实例（变换 + 颜色）；同 `mesh_id` + 同 RStates + 同纹理的实例自动合批为极少数 `draw_indexed`。
- **共享网格模式**：为"圆形"等常见图形只建一个**单位网格**（半径为 1），实例变换用 `Translate(pos) * Scale(r)`——整张地图几百个圆共享同一顶点缓冲，DrawCall 从"每圆一次动态提交"降到每层 1 次。
- **⚠️ 哪些元素不能静态化**：会**插入实体绘制顺序**的元素（如 RPG 中 `y_layer(foot_y)` 的树）必须保持动态路径（Sprite / `add_mesh*`），否则遮挡关系错误。固定 `LAYER_TERRAIN` 之类层级的元素才安全。
- **地图重开重建**：静态地形列表随地图生成一次、缓存在 App 层；地图重开（如 R）时按版本号重建（参考 `eg260731RPG` 的 `map_rev` + `StaticTerrain` 模式）。

示例见 [`API_REFERENCE.md`](API_REFERENCE.md#542-静态网格-staticmesh) §5.4.2。

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
- 合批：**同一纹理 + 同一 RStates 的连续绘制自动合批**（含 Sprite 与 StaticMesh）。
- 采样器**与纹理解耦**：`TextureWrapped` 只持有纹理本身（texture/view/uid），不再持有 sampler / bind group。
  - 采样器完全由 `RStates` 位域（bits 8..24）驱动——`.samp_mag(Nearest)` / `.samp_addr_u(Repeat)` 等链式方法**真实生效**。
  - `Render2D` 内部按需创建并缓存 `wgpu::Sampler`（默认线性 + ClampToEdge 走零开销快路径）。
  - bind group 由 `Render2D` 按 `(tex_uid, samp_key)` 缓存，value 持有 `Arc<Texture>` 防悬挂；`prepare` 末尾自动剔除 `TEXTURES.remove` 掉的失效条目。
- 全局注册表 `TEXTURES`（`TypedRegistry<TextureWrapped>`）：支持 `register`/`register_named`/`get`/`remove`/`remove_name_mapping`/`rename`/`contains_uid`/`contains_name`。
- 1×1 白色纹理：纯色 Sprite/StaticMesh 使用 `white_texture`。

---

## 7. 运行时图集：DynamicAtlas 与 StaticAtlas（`rjw_atlas`）

`rjw_atlas` 提供两种图集——**运行时动态图集**（Guillotine 空闲矩形打包 + 自动分页 + 去碎片重排）与**静态预排布图集**（TOML 反序列化）。图集将多张精灵纹理合入一或数张大纹理页中，使同一页内的绘制天然满足同一纹理的合批条件。

> 引擎内部通过全局纹理注册表 `rjw_render::TEXTURES`（`DashMap`）按纹理 uid 查找页纹理，完全解耦 `rjw_2d_render`。

### 7.1 DynamicAtlas —— 运行时在线打包

```
插入精灵 → Guillotine 空闲矩形分配器找空位（best-fit + 古莱丁切分，按行堆放）→ 放不下时先整页去碎片重排 → 仍放不下则自动建新页 → 写到 GPU 纹理
```

核心类型：

| 类型 | 说明 |
|---|---|
| `DynamicAtlas<K = String>` | 主结构体，泛型 `K` 为精灵键类型（String 特化提供 TOML 导入导出） |
| `AtlasConfig` | 配置：`max_pages`（最大页数）、`padding`（精灵间距）、`lifetime`（帧寿命） |
| `AtlasRegion` | 图集区域描述：`tl_px`（像素左上角）、`wh_px`（尺寸）、`origin_px`（原点偏移）、`page_uid`（所在页纹理 uid） |

#### 创建与配置

```rust
use rjw_atlas::{DynamicAtlas, AtlasConfig};

let mut atlas = DynamicAtlas::new(
    device,
    queue,
    layout,   // wgpu::BindGroupLayout（从 Render2D 获取：render2d.bind_group_layout()）
    AtlasConfig {
        max_pages: 2,      // 最多 2 张 2048×2048 页
        padding: 0,        // 精灵之间无间距
        lifetime: 200,     // 200 帧未 get() 即视为不再需要
        ..Default::default()
    },
    2048,                  // 单页像素尺寸
);
```

约束：`DynamicAtlas::new` 需要 `device: &wgpu::Device`、`queue: &wgpu::Queue`、`layout: &wgpu::BindGroupLayout`。必须从 `RenderContext` 获取前三者，从 `Render2D` 获取布局。

#### 插入精灵

```rust
// ★ 最常用：insert_ex(name, rgba_bytes, w, h) → Option<AtlasRegion>
let grass = atlas.insert_ex("grass", &grass_rgba, 32, 32).unwrap();

// 完整参数版：insert(name, rgba, w, h, origin_px, clamp_margin)
let custom = atlas.insert("custom", &rgba, 32, 32, (5, 5), false).unwrap();
```

> **简化说明**：`DynamicAtlas` 内部持有 `device`/`queue`/`layout`（`new` 时传入一次，后续方法无需再传）。

便捷方法一览：

| 方法 | 等价于 |
|---|---|
| `insert_ex(name, rgba, w, h)` | `insert(name, rgba, w, h, (0,0), true)` — 最常用 |
| `insert_ex_origin(name, rgba, w, h, (ox,oy))` | `insert(name, rgba, w, h, origin_px, true)` |
| `insert_no_clamp(name, rgba, w, h)` | `insert(name, rgba, w, h, (0,0), false)` |
| `insert_white()` | 无参，直接返回 1×1 白像素 |

参数说明（`insert` 完整签名）：

| 参数 | 含义 |
|---|---|
| `name` | 精灵名（可重复插入，已存在则更新寿命并返回旧 region） |
| `rgba` | RGBA8 字节切片，长度 `w * h * 4` |
| `w, h` | 精灵宽高（像素） |
| `origin_px` | 原点偏移（通常 `(0, 0)`） |
| `clamp_margin` | `true` 则自动在四周扩 1px 边界像素复制（防止 GPU 采样到相邻精灵） |

`insert` 返回 `None` 表示所有页均已满（达到 `max_pages` 限制）。

#### 插入白色像素

```rust
let white = atlas.insert_white();
```

插入 1×1 纯白像素，用于纯色填充的 Sprite 合批到同一图集页内，避免纯色绘制使用独立纹理打断合批。

#### 获取 / 刷新寿命

```rust
if let Some(region) = atlas.get("grass") {
    // region.lifetime 被重置为 config.lifetime
}
```

`get()` 命中后刷新该条目的寿命，若 `end_frame()` 倒计时归零则标记可以移出（逻辑踢出，纹理页不回收）。

#### 帧尾 / 去碎片

```rust
atlas.end_frame(); // 寿命衰减 → 移除到期条目
atlas.compact();   // 去碎片：全量重排（带源条目按面积降序排到最少页，重传纹理，generation+1）
```

- **分配器**：`Guillotine`（空闲矩形列表，best-fit + 古莱丁切分）。按行堆放时每行下方始终保留整宽空闲矩形，混合字形高度也不会碎片化到“页未满却开新页”。
- **去碎片**：`compact()` 优先把全部带源条目重排进最少页（有无法搬动的永久条目时退回按页重建空闲矩形），并把 `generation()` +1。
- **区域缓存**：持有 `AtlasRegion` 副本的调用方（如 `rjw_text`）需在 `generation()` 变化后重新拉取区域，避免旧 UV 指向已搬动的像素。

#### 获取页信息

```rust
atlas.page_count();       // 当前页数
atlas.page_size();        // 单页尺寸（= N，如 2048）
atlas.generation();       // 去碎片重排世代号（搬动条目时 +1）
```

### 7.1.1 精灵生命周期与自动复活

`DynamicAtlas` 支持精灵**踢出→复活**机制：

- **寿命系统**：每个非永久精灵有 `config.lifetime` 帧的倒计时，每次 `get()` / `get_or_revive()` 命中时重置
- **墓碑**：`end_frame()` 将到期且保存了源数据的精灵移入 `tombstones`（携带 RGBA + 宽高 + 原点）
- **复活**：`get_or_revive(name)` 若在 entries 中找不到 → 从 tombstones 取出 RGBA → 重新 `insert_inner()` 写入图集 → 返回引用
- **常驻精灵**：用 `insert_permanent()` 或 `insert_ex_permanent()` 插入的精灵 `source: None`，永久存在

```rust
// 普通精灵（可复活）
atlas.insert_ex("grass", &grass_rgba, 32, 32);

// 若干帧后被踢出…
atlas.end_frame();

// 下次使用时自动复活
let region = atlas.get_or_revive("grass"); // ← 自动重新插入图集
```

### 7.1.2 TOML 批量导入/导出

从 sprite sheet TOML 文件（`spr.toml`）批量导入到动态图集：

```rust
// 加载 TOML，裁剪指定子区并插入图集
let count = atlas.load_toml(
    &toml_str,
    |tex_name| {
        // 返回 (完整纹理RGBA, 宽度, 高度)
        data.get(tex_name).cloned()  // HashMap 查找
    },
).unwrap();

// 导出当前布局
let exported = atlas.export_toml().unwrap();
```

### 7.2 StaticAtlas —— 静态预排布（`spr.toml`）

适合打包好的精灵表（sprite sheet），一次性加载。需要 `serde` feature。

```rust
use rjw_atlas::StaticAtlas;

let toml_str = std::fs::read_to_string("spr.toml").unwrap();
let sa = StaticAtlas::from_toml(&toml_str).unwrap();
let region = sa.get("my_sprite").unwrap();
```

TOML 格式（`spr.toml`）：

```toml
[my_sprite]
tex = "my_sheet"     # 对应已注册纹理标签
lt = [0, 0]          # 左上角像素
wh = [32, 32]        # 宽高
or = [0, 0]          # 原点偏移
```

- `tex` 必须在加载前已通过 `TEXTURES` 注册（例如 `render2d.create_texture(...)` 或 `TextureWrapped` 手动注册）。
- 未找到纹理时返回 `StaticAtlasError::TexNotFound`。

### 7.3 简便绘制封装 —— `Tex::draw()` 模式

图集的核心价值是**一行绘制**——把 `AtlasRegion` 转成 `SpriteRect` 再调 `render2d.add_sprite2d()` 的全套操作封装为一个方法。这是 `eg260731RPG` 的实践，推荐所有项目复制使用。

```rust
// ═══ 封装结构体（项目级，不放引擎） ═══
use rjw_render::TEXTURES;
use rjw_2d_render::{Render2D, Sprite2DBuilder, SpriteRect};
use rjw_atlas::{DynamicAtlas, AtlasRegion};
use rjw_color::Color;
use rjw_transform::Transform2D;
use glam::Vec2;

struct Tex {
    atlas: DynamicAtlas,        // 持有图集（保证页纹理存活）
    grass: AtlasRegion,         // 各精灵区域...
    player: AtlasRegion,
    white: AtlasRegion,
}

impl Tex {
    fn create(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout) -> Self {
        let mut atlas = DynamicAtlas::new(device, queue, layout, AtlasConfig::default());
        let white  = atlas.insert_white();
        let grass  = atlas.insert_ex("grass", &make_grass(), 32, 32).unwrap();
        let player = atlas.insert_ex("player", &make_player(), 32, 32).unwrap();
        Self { atlas, grass, player, white }
    }

    /// 一行绘制图集精灵。
    /// `world_tl` = 世界左上角，`world_wh` = 世界尺寸。
    /// 返回 `Sprite2DBuilder`，可继续链式 `.blend(...).depth_test(...)`。
    fn draw<'a>(
        &self,
        r2d: &'a mut Render2D,
        region: &AtlasRegion,
        world_tl: Vec2,
        world_wh: Vec2,
        color: Color,
        transform: Transform2D,
        layer: impl Into<Layer> + 'a,
    ) -> Sprite2DBuilder<'a> {
        let ps = self.atlas.page_size() as f32;
        let inv = Vec2::new(1.0 / ps, 1.0 / ps);
        let spr = SpriteRect::from_texture_px(
            world_tl,
            world_wh,
            Vec2::new(region.tl_px.0 as f32, region.tl_px.1 as f32),
            Vec2::new(region.wh_px.0 as f32, region.wh_px.1 as f32),
            inv,
        );
        let tex_ref = TEXTURES.get(region.page_uid).expect("atlas page texture must be registered");
        r2d.add_sprite2d(spr, color, transform, layer, &tex_ref)
    }
}
```

**使用示例**：

```rust
// 绘制草地瓦片 —— 一行调用
tex.draw(r2d, &tex.grass, world_tl, Vec2::splat(32.0), Color::WHITE, Transform2D::default(), 10.0);

// 绘制纯色 UI 条（与草地同属图集页 = 合批）
tex.draw(r2d, &tex.white, bar_pos, bar_wh, Color::rgba(0.9, 0.2, 0.2, 1.0), Transform2D::default(), LAYER_UI);

// 带混合模式
tex.draw(r2d, &tex.slime, tl, wh, Color::WHITE, tf, y_layer(foot))
    .blend(BlendMode::Additive);
```

**关键设计要点**：

1. `Tex` 结构体**持有 `DynamicAtlas`**，确保图集页纹理不会被释放。
2. `AtlasRegion` 字段直接存为 `Tex` 的成员，方便 `tex.grass` / `tex.white` 语义化引用。
3. `draw()` 内部通过 `TEXTURES.get(uid)` 查找页纹理，返回值直接喂给 `add_sprite2d`——与 `create_texture` 路径的纹理使用完全统一。
4. 返回 `Sprite2DBuilder`，与 `Render2D::add_sprite2d` 接口一致，支持链式 RStates。

---

## 8. 渲染状态（RStates）与 Builder 责任链

`RStates` 是一个 u64 bitfield，涵盖 6 个渲染控制域：

| 域 | 字段 | 示例 |
|---|---|---|
| Blend | `BlendMode` (Alpha/Additive/Multiply/Premultiplied/Inverse/Subtract/Min/Max/Disabled) | `.blend(Additive)` |
| Sampler | mag/min/mip filter + addr_u/v/w | `.samp_addr_u(Repeat).samp_mag(Nearest)` |
| Cull+Raster | cull + polygon + front_face + conservative | `.cull(Back).polygon(Line)` |
| Depth | test + write + compare | `.depth_test(true).depth_write(true)` |
| Stencil | test + write + compare | `.stencil_test(true).stencil_compare(Always)` |

### 8.1 三级控制

| 级别 | 使用方式 | 作用范围 |
|---|---|---|
| **全局默认** | `render2d.default_blend(Additive).default_depth_test(true).set_mvp(...)` | 所有不链式调用的 add_* |
| **单条绘制** | `render2d.add_sprite2d(...).blend(Multiply)` | 该条命令 |
| **批量设置** | `.blend_state(BlendDesc{...}).samp_state(SamplerDesc{...}).depth_state(DepthState{...})` | 同上 |

### 8.2 Builder 使用

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

### 8.3 MeshBuilder 特有的 `.set_texture()`

```rust
// Mesh 默认白色纹理；set_texture 覆盖
render2d.add_mesh(&verts, &tris, Color::WHITE, 96.0)
    .set_texture(&my_tex)
    .blend(BlendMode::Alpha);
```

### 8.4 重要类型

| 类型 | 说明 |
|---|---|
| `RStates` | u64 bitfield，含全部 6 个控制域 |
| `BlendMode` | Alpha / Additive / Multiply / Premultiplied / Inverse / Subtract / Min / Max / Disabled |
| `FilterMode` | Linear / Nearest |
| `AddressMode` | ClampToEdge / Repeat / MirrorRepeat |
| `CullMode` | None / Front / Back |
| `PolygonMode` | Fill / Line / Point |
| `FrontFaceWinding` | Ccw / Cw |
| `CompareFunc` | Never / Less / Equal / LessEq / Greater / NotEq / GreaterEq / Always |
| `BlendDesc` / `SamplerDesc` / `RasterState` / `DepthState` / `StencilState` | 批量设置描述符 |

---

## 8.5 文本渲染（`rjw_text`）

`rjw_text` 基于 `cosmic-text` 排版 + `swash` 字形光栅化 + `DynamicAtlas` 字形缓存。

### 核心 API

| 方法 | 说明 |
|---|---|
| `Text::new(device, queue, layout)` | 创建字体系统（自动加载系统字体） |
| `Text::measure(text, attrs, size, lh, align) -> Vec2` | 排版 + 测量内容宽高（GUI 布局用） |
| `Text::measure_buffer(buffer) -> Vec2` | 已排版 Buffer 的内容宽高（行盒；空文本返回 (0,0)） |
| `Text::draw_label(r2d, text, color, size, lh, pos, family, align, layer) -> Vec2` | ★ 左上角起始渲染，返回内容宽高（feature = `rjw_2d_render`） |
| `Text::draw_label_ex(r2d, text, color, size, lh, pos, family, align, layer, origin) -> Vec2` | 扩展版：origin 归一化到 [0,1]，(0.5,0.5)=原点居中（feature = `rjw_2d_render`） |
| `Text::draw_label_with(text, size, lh, pos, family, align, origin, callback) -> Vec2` | 回调版：不绑定 Render2D，GUI 自定义字形绘制 |
| `Text::text(..) -> TextLayout` | 责任链入口（阶段一：排版配置；常量字符串内联） |
| `TextLayout::into_render() -> TextRender` | 转阶段二（用 `Text` 内部缓冲，单标签快速路径，跨帧复用容量） |
| `TextLayout::into_render_with(&mut TextBuffer) -> TextRender` | 转阶段二（用户持缓冲，多标签并存） |
| `TextLayout::precache() -> Self` | 预缓存：字形入图集（预热），返回自身可稍后渲染 |
| `TextLayout::into_render() -> TextRender` | 转阶段二：直接堆存储 |
| `TextRender::from_layout(layout)` / `TextRender::new(..)` | 转换 / 直接构造（调用 TextRender 的函数） |
| `TextRender::origin/origin_px/offset/color/map` | 渲染设置：原点 / 偏移 / 全局色 / 逐字形修改 |
| `TextRender::transform(Option<Transform2D>)` | 渲染级变换（作用整个文本块，绘制均应用） |
| `TextRender::draw_with(callback)` | 回调 `(measure, line, region, topleft)` 绘制（核心，无 feature 依赖） |
| `TextRender::draw_sprite2d(r2d, layer)` | 直接渲染到 Render2D（feature = `rjw_2d_render`） |
| `TextRender::draw_2d_gradient(r2d, layer, mode, axis, stops)` | 渐变渲染：Glyph/Line/Frame × 横/竖向（feature = `rjw_2d_render`） |
| `GlyphType` | 字形类型：`Normal`（单色）/ `Color`（Emoji）；`GlyphData::glyph_str()` 取对应字符 |
| `Text::build_style() -> TextStyle` | 构建可复用样式（简化重复字体/字号/行距；支持克隆继承） |

### 性能设计：排版缓存与字形跳过

- **排版缓存（LRU）**：`Text` 内部按（文本 / 字号 / 行高 / 对齐 / attrs）缓存 cosmic-text 排版结果；相同输入经 **O(1) 签名**预过滤命中后返回共享 `Arc<Buffer>`（不深拷贝），跳过每帧重复的 `Shaping::Advanced` 整形（Debug 下是最主要开销）。缓存上限 [`MAX_LAYOUT_CACHE`]（默认 128），满时按 LRU 淘汰最久未用条目。
- **无图字形跳过**：空格 / 零尺寸 / swash 渲染失败的字形记入 `no_image` 集合，只判定一次，避免每帧重复光栅化。
- **图集去碎片同步**：字形图集 `compact()` 重排后 `generation()` 变化，`Text` 自动从图集重新拉取字形区域（`sync_atlas_regions`），无需用户处理。

### 使用示例

```rust
use rjw_text::{Text, Align};

let mut font = Text::new(r2d.device(), r2d.queue(), r2d.tex_bind_group_layout());

// 左上角单行
font.draw_label(r2d, "Hello", Color::WHITE, 14.0, 18.0, Vec2::new(10.0, 10.0), "SimHei", Align::Left, 0.0);

// 屏幕居中（相机中心）
let _ = font.draw_label_ex(r2d, "GAME OVER\n按 R 重开", Color::RED, 22.0, 28.0, cam.position, "SimHei", Align::Center, LAYER_UI, Vec2::new(0.5, 0.5));
```

---

## 9. 输入：键盘 / 鼠标

### 9.1 键盘——`KeyState`（重点：边沿）

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

### 9.2 鼠标

```rust
ctx.mouse.get_mouse_position()                        // (x,y) 窗口像素
ctx.mouse.get_mouse_delta()                           // 本帧移动增量
ctx.mouse.get_mouse_button_state(MouseButton::Left)   // KeyState
ctx.mouse.get_wheel_delta()                           // (x, y) 滚轮
ctx.mouse.get_pixel_wheel()                           // 触控板像素级滚轮
ctx.mouse.in_window()                                 // 是否在窗口内
```

---

## 10. Transform2D 变换

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

## 11. 颜色：Color 与 ColorF64

| 类型 | 存储 | 常用构造 | 用途 |
|---|---|---|---|
| `Color` | `f32` ×4 | `rgba(r,g,b,a)`、`rgba_u8(..)`、常量 `RED/GREEN/...` | 绘制命令 |
| `ColorF64` | `f64` ×4 | `rgba(f64,..)` | 可 `.into()` `wgpu::Color`（清屏） |

---

## 12. 时间：DeltaTimer

```rust
ctx.timer.dt()              // 帧间隔秒（Duration）
ctx.timer.dt().get_f32()    // 帧间隔秒（Duration）的 f32 转换
ctx.timer.get_fps()         // 当前 FPS
```

每帧用 `dt` 做位移：`pos += vel * dt`；建议 `dt.min(0.05)` 防跳变。

---

## 13. 窗口与事件循环

- 入口：`run_app(App::new())`，`App` trait 必须实现 `on_init`、`about_to_wait`；`on_resized` 可选。
- `MainContext`：`keyboard` / `mouse` / `timer` / `primary_window()` / `request_exit()`。
- `on_init` 中创建 `RenderContext` + `Render2D`（`RenderContext` 必须比 `Render2D` 活得久）。
- `on_resized`：`render.resize(w,h)` + 更新相机视口。

---

## 14. 视口 / 缩放 / 高 DPI

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

## 15. 性能与内存约定

- `Render2D` 内部 `buf_*` 全部常驻复用（`clear()` 只清长度不释放）。
- **实例缓冲是"页池"**：单帧总实例数可远超 8192，自动分页；**不要自己裁减数量去凑**。
- **统一管线缓存**：`DrawPage` 按 `RStates::raw()` keys 缓存 `RenderPipeline`，首次遇新状态时创建、后续帧直接命中。HashMap 常驻，Query 事件循环保持不变。
- **builder 不产生堆分配**：`Sprite2DBuilder` / `MeshBuilder` / `StaticMeshBuilder` 均为栈上 struct，Drop 时直接转移到 `DrawCommandQueue` 的 Vec。
- 页池按需一次性增长、永久复用。
- Mesh 顶点走 u16 索引，单帧顶点数 ≤ 65535。
- **静态网格合批**：固定层、不参与 y-sort 的地图元素用 `register_mesh` + `add_static_mesh*` 静态化；同 mesh_id 的实例自动合并为极少数 DrawCall。常见图形（如圆）用**一个单位网格 + 实例缩放**共享顶点，避免每实例一份缓冲。

---

## 16. 对 AI 的维护约定

给接手改代码的 AI 助手的清单：

1. **坐标系**：Y+ 向下。写"向上移动"用 `y -=`。
2. **逻辑像素 ≠ 物理像素**：相机一律用 `render.size()`（物理）。
3. **瞬时操作用 `down_edge()`**；持续操作用 `pressed()`。
4. **透明覆盖问题（大坑）**：`Queue::write_buffer` 在 `submit` 前会**全部先执行**——所以引擎用"页池"：每页只写一次、绑定对应页。
5. **Layer 数值小先画**；RPG 里实体/地形用 y-sort 动态 layer，UI 用 ≥1e7 固定层。
6. **RStates resolve**：`prepare()` 中 `States.rstates: None` → `default_rstates`；`Some(r)` → 直接用 `r.raw()`。
7. **Builder 是责任链，Drop 自动 push**：`add_*` 返回 builder，不链式调用也自动 push（`rstates: None`）。不要手动 push。
8. **统一管线**：不再有 `sprite_pipeline` / `mesh_pipeline` 分支。所有绘制走 `get_or_create_pipeline(rst_raw)`，Sprite/StaticMesh 绑实例页、动态 Mesh 绑 identity instance buffer。
9. **管线缓存 key = RStates::raw()**：u64 哈希，同一个 raw 值只创建一次管线。
10. **采样器由 RStates 位域驱动**：`.samp_*` 链式方法真实创建 GPU 采样器；`TextureWrapped` **不再持有** sampler / bind group（bind group 由 `Render2D` 缓存）。
11. **静态网格**：`MeshData` 注册进 `MESHES` 后经 `add_static_mesh*` 实例化；固定层、不参与 y-sort 的元素才静态化，**会插入实体排序的（如 y_layer 树）保持动态**；地图重开时按版本号重建静态地形缓存。
12. 改 `rstates.rs` 的 bitfield 布局时务必更新 `to_blend()` / `to_depth_stencil()` / `to_cull()` / `to_sampler_desc()` 等解包方法。
13. 纹理数据长度必须 `w*h*4`。
14. 改公共 crate 后，务必 `cargo check --workspace`。
15. **建议**：进行**破坏性更改**、**特性添加**等操作时，请务必更新 [`API_REFERENCE.md`](API_REFERENCE.md) 和 [`ENGINE_GUIDE.md`](ENGINE_GUIDE.md)。

---

## 17. 快速速查表

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
| Mesh 多边形带 UV | `r2d.add_polygon_fan_uv(&verts, &uvs, color, layer)` / `add_polygon_strip_uv` |
| 设置采样器重复 | `.samp_addr_u(AddressMode::Repeat).samp_addr_v(AddressMode::Repeat)` |
| 反转混合 Sprite | `.blend(BlendMode::Inverse)` |
| 关闭混合 | `.blend(BlendMode::Disabled)` |
| 注入原生 wgpu 绘制 | `r2d.add_custom(1.0, \|pass\| { ... })` |
| 延迟丢弃 builder（显式绑定变量） | `let _b = render2d.add_sprite2d(...).blend(...);` —— `_b` 在作用域结束时 Drop |
| Atlas 一行绘制 | `tex.draw(r2d, &tex.grass, tl, wh, color, tf, layer)` |
| 纯色用 white 合批 | `tex.draw(r2d, &tex.white, tl, wh, color, tf, layer)` |
| Atlas 插入精灵 | `atlas.insert_ex(name, &rgba, w, h).unwrap()` |
| Atlas 常驻精灵 | `atlas.insert_ex_permanent(name, &rgba, w, h)` |
| Atlas 插入白色像素 | `atlas.insert_white()` |
| Atlas 自动复活查找 | `atlas.get_or_revive(name)` |
| Atlas 从 TOML 批量导入 | `atlas.load_toml(toml_str, \|k\| data.get(k).cloned())` |
| Atlas 导出 TOML | `atlas.export_toml()` |
| 注册静态网格 | `let id = render2d.register_mesh(Arc::new(MeshData::from_pod(device, &verts, &idx, "m")));` |
| 静态网格实例（Transform2D） | `render2d.add_static_mesh(id, color, tf, layer, &tex).done()` |
| 静态网格实例（Mat4） | `render2d.add_static_mesh_matrix(id, color, model, layer, &tex).done()` |
| 纯色圆共享单位网格 | 单位圆网格 + `with_pos(pos).with_scale(Vec2::splat(r))` 实例化 |

---

*遇到报错请优先怀疑：坐标系方向、物理/逻辑像素、`down_edge` vs `pressed`、页池覆盖（勿改回单缓冲）、layer 数值大小。这五类占引擎使用失误的 95%。*

---

## 18. UI（rjw_ui）

> 完整 API 见 `crates/rjw_ui` 的 crate 文档与 [API_REFERENCE.md](API_REFERENCE.md)「10. UI」章节；示例：`cargo run -p eg260818UI`。

### 18.1 是什么

`rjw_ui` 是引擎的 UI 模块，三句话概括：

1. **hybrid 模式**：外观逐帧录制（立即模式），交互状态（hover / 按下 / 焦点 / 输入内容 / 滑块拖拽 / 单选组 / grid 单元格缓存）经 **ID** 持久化在 `UiState`（应用持有，跨帧）。
2. **DOM 风格自动尺寸**：叶子控件由内容测量自然撑开（`Text::measure` + padding），容器（panel / pack / grid）闭包结束时按子控件结算自身尺寸——**默认无需手写宽高**；需要时传显式 `Rect`（`*_at` 变体）覆盖。
3. **Tkinter 风格几何管理器**：`place`（`*_at(pos)` 绝对定位）、`pack`（按 `PackSide` 堆叠）、`grid`（均匀网格）。

### 18.2 最小用法

```rust
// UiState 存在应用结构体里（跨帧持久）
let mut ui = Ui::begin(window, &cam, &ctx.mouse, &ctx.keyboard, &mut text, &mut r2d, &mut ui_state)
    .theme(Theme::dark())
    .base_layer(LAYER_UI)                               // 默认 1e7
    .scale_factor(ctx.scale_factor().unwrap_or(1.0))    // DPI：控件坐标/字号按逻辑像素
    .build();

ui.label_at(Vec2::new(16.0, 12.0), "FPS: 60");          // place：绝对定位 + 内容自然尺寸
ui.pack_at(Vec2::new(16.0, 90.0), PackSide::Top, |p| {  // pack：垂直堆叠
    if p.button("btn_start", "开始游戏").clicked() { /* ... */ }
    volume = p.slider("vol", 0.0..=1.0, volume);         // 返回新值
    if p.checkbox("fs", "全屏", fs).toggled() { fs = !fs; }
    if p.radio("diff_hard", "diff", "困难").checked() { /* 单选组互斥 */ }
    p.text_input("name", &mut player_name);              // 点击聚焦、打字（IME 已支持）、Enter/Esc 失焦
});
ui.window_at("win_a", Vec2::new(560.0, 240.0), |w| {    // 可重叠窗口：点击置顶 + 可拖拽
    w.label("窗口 A");
    w.button("win_a_btn", "A 按钮");
});
ui.drag_panel_at("inv_panel", Vec2::new(300.0, 90.0), |pp| {  // 可拖拽面板（位置持久）
    pp.label("背包");
    pp.grid_at(Vec2::new(0.0, 28.0), 3, "inv", |g| {    // 3 列网格（cell 跨帧缓存）
        g.button("slot_0", "物品 0");
    });
});
ui.finish();   // 排序（窗口 z → 深度 → 图形/文字 → 录制序）并提交绘制
```

### 18.3 ⚠️ 易混淆点

- **坐标一律屏幕逻辑像素**（左上角原点、Y+ 向下）：调用 `.scale_factor(ctx.scale_factor().unwrap_or(1.0))` 后，所有控件坐标 / 字号按逻辑像素使用，内部自动换算物理像素绘制与命中；不设置则 scale = 1.0（与物理像素一致）。与引擎世界坐标（中心原点）不同；内部经相机屏幕固定变换绘制，旋转/缩放相机下依然 1:1。
- **顶层用 `*_at(pos, ...)`**（含 `label_at` / `panel_at` / `pack_at` / `grid_at`）；容器内才可用无位置形式（`p.button(...)` 占光标）。容器内嵌套容器用 `*_at(offset)`（相对当前容器内容原点），**不占光标**——v1 不支持容器内"光标嵌套"。
- **交互控件必须有稳定 ID 字符串**（按钮 / 滑块 / 勾选 / 单选 / 输入框）；ID 变化 = 状态丢失。`UiState::reset()` 清空全部状态。
- **闭包内不可借用已被 `ui` 借用的字段**（如 `self.ui_state`）；需要重置等操作时用局部标记，`ui.finish()` 后统一处理（见示例）。
- **单选**的选中状态完全存于 `UiState.radio_groups`（`group → id`），应用只读 `checked()`；初始选中用 `state.radio_groups.insert("组名", "id")`。
- **文本输入**：普通字符走 `rjw_keyboard::get_chars()`（含 Shift 组合，控制字符已过滤）；**中文输入法（IME）已支持**——`rjw_main` 建窗时自动 `set_ime_allowed(true)`，`rjw_keyboard` 收集上屏文本（`get_ime_commits`）与组合候选（`get_ime_preedit`，输入框以灰色绘制在光标后），Enter 确认上屏。
- **可拖拽面板 / 窗口**：`drag_panel_at(id, pos, |p| ...)` 按住面板拖动；`window_at(id, pos, |w| ...)` 是**可重叠窗口**——点击即**置顶**（焦点 z-order，`UiState.window_z`），位置持久于 `UiState.panel_pos`；拖动期间**抑制内部子控件交互**。拖拽位置按**物理像素粒度**跟随（1px 跟手，不受 DPI 逻辑量化影响）。
- **窗口重叠点击裁决**：重叠区域点击**只让最上层窗口**获得拖拽与置顶（`Ui::finish` 内部 `resolve_win_press`）——不会同时拖动两个窗口。
- **QuadVertices 渲染（不使用 Sprite）**：全部图元（背景 / 控件背景 / 文字）转为**四边形顶点**（`Render2D::add_quads`，每四顶点一组 **TL,TR,BL,BR**，固定索引 `[0,1,3, 3,2,0]`），按 `(窗口, 图形/文字组, 纹理)` 分组提交。**UI 自行管理绘制顺序**——**UI 的 Render2D 必须 `set_sorting(false)`**（完全按提交顺序绘制）：`finish` 按 `(win 升序, 白纹理图形组 → 字形文字组, 纹理 uid)` 提交——窗口间层级由提交顺序保证（`layer = base + z*1.0` 仅作兜底），窗口内**“背景/图形 → 文字”严格稳定**，不随纹理 uid / HashMap 顺序抖动。⚠ **不要用 `set_sorting(true)`（`SortMode::LayerAndStates`）**：它会在同一 layer 内按 `(rstates, texture_uid)` 重排，字形图集页先于程序化纹理页（圆角/渐变）注册 → 重排后圆角/渐变图形排在文字之后绘制、**盖住文字**（曾因此踩坑：圆角按钮文字消失、渐变状态栏盖住标签）。`set_layer_sort(true)`（`LayerOnly`，稳定排序）同层保持提交顺序，可接受。
- **窗口 transform**：四边形顶点为**相对窗口原点的局部像素**，提交时经 `screen_fixed_tf(窗口原点)` 变换到世界——**移动窗口只改变换、顶点不变**（也支持将窗口嵌入游戏场景，给任意世界变换）。`add_quads` / `add_mesh_transform` 均支持 `Transform2D`（`IDENTITY` = 顶点即世界坐标）。
- **窗口顶点缓存**：窗口内容不变时，四边形顶点**跨帧缓存**（`UiState.window_quads` 按**内容签名**命中）——静态窗口每帧零字形收集/重建；hover 变色、文字编辑等任何内容变化都会使签名变化而自动重建。**移动窗口不影响缓存**（顶点是局部的，transform 每帧用当前原点）。
- **深度测试**：`RStates::default()` 深度测试**默认关闭**，QuadVertices 纯 2D 覆盖无需深度；世界层需要深度时用 `render2d.default_depth_test(true)`（UI 独立 Render2D 不受影响）。
- **独立 UI 渲染（推荐）**：UI 录制到**单独 Render2D**（`Render2D::set_sorting(false)` 关闭排序，UI 自行管理绘制顺序），与世界合并提交：`r2d.render_command_buffer(clear, &view, None)`（世界）→ `r2d_ui.render_command_buffer(...)`（UI，color: None 不覆盖）→ `queue.submit([cb_world, cb_ui])` → `queue.present(st)`（一次 present）。
- **输入屏蔽**：文本输入框聚焦时应用快捷键不应触发——检查 `UiState::capturing_text()`（如示例中 `R` 重置 / `Esc` 退出前）；输入框内 `Esc` 取消焦点（不再传给应用层）。
- **IME 定位**：输入框聚焦时自动调用 `Window::set_ime_cursor_area`（`Ui::begin` 需传 `&Window`），中文输入法的候选框跟随输入框光标。
- **文本性能**：控件排版缓冲（`Arc<Buffer>`）自持于 `UiState.text_buffers`（`CachePolicy::User`，**不推入 `rjw_text` 内部 LRU**）；静态标签每帧命中缓存跳过重复整形，容量上限 [`TEXT_BUFFER_CACHE_CAP`]=128（超出整体清空重建）。

### 18.5 调试（Debug UI / DebugDraw / 窗口诊断）

**样式如何设置**：调试视觉风格统一在 `Theme::debug`（`DebugStyle`，可 clone 覆盖）配置——
`theme.debug.layout_outline`（`debug_layout` 描边颜色，默认青色）与
`theme.debug.layout_outline_width`（描边宽度，物理像素，默认 1.0）；`ui.debug_*` 图元
的样式（颜色 / 线宽）**每次调用显式传参**（逻辑像素）。示例：

```rust
let mut theme = Theme::dark();
theme.debug.layout_outline = Color::MAGENTA;   // debug_layout 描边改洋红
theme.debug.layout_outline_width = 2.0;        // 2 物理像素宽
// Ui::begin(..).theme(theme).debug_layout(true).build()
```

**Debug UI（调试 rjw_ui 自身）**：

- `UiInit::debug_layout(true)` 或帧内 `ui.debug_layout(on)`：给**每一个录制命令的矩形**
  （控件 / 容器 / 文本块 / 光标）画描边——可视化布局矩形与命中区域；开启时跳过窗口
  顶点缓存（每帧重建），是纯调试视图。描边走独立 debug 叠加层，**恒覆盖在 UI 内容之上**。

**rjw_ui 的 DebugDraw（屏幕空间）**：`ui.debug_line` / `ui.debug_rect_outline` /
`ui.debug_circle_outline` / `ui.debug_cross` / `ui.debug_grid`——坐标 = **绝对逻辑屏幕像素**
（Y+ 向下，与 UI 控件一致），经独立 `debug_queue` 录制，`finish` 时按窗口分组、白纹理
四边形提交，**恒覆盖在 UI 内容之上**（不进窗口缓存）。世界坐标调试图元（游戏场景：
碰撞盒 / 网格 / 速度矢量）见 `rjw_2d_render::debug_draw`——两者分工：**画场景用世界，
画 UI 用屏幕**。示例 `examples/egDebugDraw` 同时演示三套。

**窗口诊断（重叠点击排查"哪个窗口赢了"）**：

- `ui.window_order()`：全部窗口 z 序（`(id, z)` 升序）；
- `ui.window_under_mouse()`：鼠标下**最上层**窗口（重叠点击时唯一可交互的窗口）；
- `UiState::last_press_window()`：上次按下由哪个窗口接收（`finish::resolve_win_press` 的"赢家"）；
- `UiState::occluded_hits()`：上帧**命中但被更高窗口遮挡而未响应**的控件次数
  （点击穿透拦截计数——`> 0` 说明鼠标下有叠放、背后控件被正确抑制）。

注意：`occluded_hits` / `last_press_window` 是跨帧持久数据，须在 `Ui::begin` **之前**读取
（begin 会借用 `ui_state`），上一帧 finish 写入、本帧显示（见 `examples/eg260818UI`
右上角诊断面板——把窗口 A/B/背包叠在同一处点击即可看到解析结果）。

**点击穿透（窗口遮挡）**：重叠区域**只有鼠标下最上层窗口**的控件响应——`hit_abs`
（所有控件共用）与窗口/面板自身的拖拽命中都过 `window_occluded(z, mouse, window_rects)`
闸门（`z=0` 的非窗口内容被任意窗口遮挡）；窗口矩形跨帧缓存于 `UiState.window_rects`
（`window_at` 录制时更新，`finish` 末尾只保留本帧录制过的窗口，销毁/置顶换 z 的旧条目
自动清理）。已知边界：窗口**首次出现的那一帧**矩形尚不可知（跨帧缓存盲区），下一帧起
严格生效——置顶方向从第一帧就正确（`win_press_top` 只保留最上层按下窗口）。

### 18.6 渲染增强（圆角 / 渐变）

**样式如何设置**：`Theme` 子样式的 `radius` 字段（面板 / 窗口 / 按钮 / 输入框，逻辑像素，
默认 0 = 直角）；绘制原语 `ui.rounded_rect_at(pos, size, radius, color)` 与
`ui.gradient_rect_at(pos, size, axis, stops)`（绝对定位，`elem = 0` 装饰层）。

```rust
let mut theme = Theme::dark();
theme.panel.radius = 8.0;    // 面板/窗口圆角
theme.button.radius = 6.0;   // 按钮圆角（三态背景同样生效）
theme.input.radius = 4.0;    // 输入框圆角
// Ui::begin(..).theme(theme).build()

ui.gradient_rect_at(Vec2::new(0.0, 0.0), Vec2::new(1280.0, 56.0),
    GradientAxis::Horizontal,
    vec![(0.0, Color::rgba_u8(38, 52, 90, 255)), (1.0, Color::rgba_u8(26, 34, 60, 255))]);
```

**程序化纹理进动态 Atlas**：圆角矩形（`32×32`）、线性渐变（主轴 `64` 级）、WHITE（`1×1`）
由 `rjw_ui::ProcTextures`（`UiState` 持有，惰性初始化）生成并 `insert_permanent` 塞进
`rjw_atlas::DynamicAtlas`——与字形图集同机制（Guillotine 打包、页纹理自动注册进
`TEXTURES`、`clamp_margin` 防采样透色）。要点：

- **圆角纹理只存白色 + alpha**：同半径一张纹理，绘制时用**顶点色 tint** 得到任意颜色
  （图集不随颜色膨胀）；key 仅含半径；
- **9-patch 绘制**（`proc::rounded_9patch`）：四角原样采样、四边/中心拉伸——任意
  矩形尺寸圆弧不畸变；`radius > 0` 的边框 ≈ 外圈 border 色圆角 + 内圈 bg 色圆角
  （`Ui::push_panel_like`）；
- 渐变矩形直接拉伸采样（主轴 64 级已平滑）；渐变纹理存**真实颜色**（key 含停靠点，
  改变停靠即换纹理）；
- **提交分组升级为 `(win, 图形/文字组, 纹理 uid)`**（`GROUP_GRAPHIC=0` / `GROUP_TEXT=1`）：
  圆角 / 渐变 / 白纹理属于图形组，恒先于字形文字组——非白纹理 uid 不会因与白纹理
  比较而排序错位（背景不会盖住文字）。窗口顶点缓存（`UiState.window_quads`）同步
  携带分组。

### 18.7 滚动容器（scroll_at）

```rust
ui.scroll_at(Vec2::new(880.0, 130.0), Vec2::new(240.0, 300.0), "scroll_demo", |s| {
    s.label("滚动列表");
    for i in 0..40 {
        s.button(&format!("log_{i}"), &format!("日志条目 {i}"));
    }
});
```

- 内容在可视区内 **pack Top 堆叠**（子项 `s.label` / `s.button` 等占光标），超出部分
  滚动查看：**滚轮**（鼠标在可视区内）+ 右侧**滚动条**（拖 thumb 按比例滚动、点轨道
  翻页）；滚动偏移持久于 `UiState.scrolls`（`id` 键，跨帧，含内容高供 clamp）；
- **裁剪**：`scroll_at` 把可视区（∩ 外层裁剪）设为 `Ui.clip`，录制命令时写入
  `UiDraw.clip`（**绝对逻辑屏幕坐标**，随容器平移），`collect_cmds` 收集期与内容矩形
  求交（[`draw::intersect_rect`]）——Solid / RoundedRect / Gradient / Border / Caret
  按交集绘制，Text 把外层裁剪转相对文本块后与命令自带裁剪合并（`draw_text_quads`）；
- 维护约定：新增控件时，录制命令必须带上 `self.clip`（`UiDraw.clip` 字段），
  否则滚动容器内无法裁剪；`UiDraw::translate` 会同步平移 `clip`。

### 18.8 键盘导航（焦点遍历）

交互控件（按钮 / 勾选 / 单选 / 滑块 / 输入框 / 下拉框）录制时调用 `Ui::register_focus`
登记进**本帧焦点链**（`focus.rs` 的 `FocusEntry`：id / 窗口 z / 类型 / **绝对逻辑矩形** /
裁剪）。`finish` 末尾 `handle_focus_keys`：

1. 链按 `(win, 注册序)` 稳定排序（非窗口 0 在前，窗口按 z 从下到上）；
2. **Tab / Shift+Tab / ↑ / ↓** → `focus_step`（纯函数，可单测）取下一个焦点，
   更新 `UiState.focused`（跨帧持久）；焦点控件本帧未录制 → 自动清除焦点；
3. **Esc** → 优先收起展开的下拉框（`UiState.combo_open`），否则取消焦点；
4. **焦点描边**：对焦点控件画一圈 `DrawKind::Border`（`Theme::focus` 样式），
   `elem = self.seq + 1`（全局最大 → 画在窗口内容之上），`clip` 沿用控件自身。

**激活与调值**（控件录制时即时处理，无跨帧延迟）：

- **Enter / Space**：`Ui::key_click(id, kind)`（焦点匹配 + 非 IME 组合中）→ 按钮 /
  勾选 / 单选 / 下拉框合成一次 clicked（`key_click` 排除 TextInput / Slider）；
- **← / →**：焦点为滑块时步进调值（`span × 5%`）；焦点为输入框时移动光标（原有）；
- **↑ / ↓**：下拉框展开且焦点在按钮上时循环切换选项（选中即收起）；
- 文本输入框内 Tab 照常遍历焦点（`get_chars` 已过滤 `\t` 控制字符，不会插入制表符）；
  Enter / Esc 失焦行为保持不变（应用快捷键需检查 `capturing_text()`）。

> ⚠️ 焦点是**跨帧持久状态**（`UiState.focused`），但焦点链**每帧重建**（immediate-mode：
> 控件动态增删自动反映）；焦点描边改变窗口内容签名 → 窗口顶点缓存自动失效重建。

### 18.9 维护约定（对 AI）

- 布局 / 命中 / 状态机是**纯逻辑**（`layout.rs` / `hit.rs` / `state.rs` / `focus.rs`），改动后跑 `cargo test -p rjw_ui`（无 GPU 依赖）。
- 新增控件 = 在 `ui.rs` 加 `Ui::xxx_at` 实现 + 在 `widget_api!` 宏里加便捷方法（Panel / Pack / Grid 自动获得）。
- 新增**交互**控件时必须调用 `register_focus(id, rect, FocusKind::X)`（键盘导航 / 焦点描边）；需要 Enter/Space 激活的控件用 `key_click(id, kind)` 合成点击。
- 绘制命令坐标语义：**相对当前容器 origin 的局部坐标**，容器弹出时统一平移；命中测试用 `abs_base + 局部`。新增容器时务必保持该约定。
- 网格 cell 缓存（`UiState::grid_cells`）保证跨帧布局稳定；无缓存首帧渐进扩展，次帧起稳定。
