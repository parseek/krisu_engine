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
7. [输入：键盘 / 鼠标](#7-输入键盘--鼠标)
8. [Transform2D 变换](#8-transform2d-变换)
9. [颜色：Color 与 ColorF64](#9-颜色color-与-colorf64)
10. [时间：DeltaTimer](#10-时间deltatimer)
11. [窗口与事件循环](#11-窗口与事件循环)
12. [视口 / 缩放 / 高 DPI](#12-视口--缩放--高-dpi)
13. [性能与内存约定](#13-性能与内存约定)
14. [对 AI 的维护约定](#14-对-ai-的维护约定)
15. [快速速查表](#15-快速速查表)

---

## 1. 引擎是什么 / 模块地图

`krusie` 是一个 **Rust + wgpu (30.0.0)** 的 2D 游戏/渲染引擎（视觉验证为主的工作区，含可运行 examples）。

```
crates/
├─ rjw_main        # 入口：run_app(App) + 事件循环 + 窗口 + MainContext(键盘/鼠标/计时)
├─ rjw_render      # 底层渲染上下文：RenderContext / 纹理 TextureWrapped / wgpu 重导出
├─ rjw_2d_render   # ★ 2D 批渲染器：Render2D / SpriteRect / Mesh / 分页实例缓冲
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
     → 录制绘制命令(add_sprite2d_* / add_mesh / add_polygon_fan)
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
            .with_inner_size(LogicalSize::new(1280.0, 720.0)) // 逻辑大小，高DPI下会进行转换，请注意
    }

    fn on_init(&mut self, ctx: &mut MainContext) {
        let window = ctx.primary_window().expect("window");
        self.render = Some(RenderContext::new(window, &RenderConfig::default()));
        let render = self.render.as_ref().unwrap();
        self.r2d = Some(Render2D::new(render));
        // camera 视口 = 表面物理尺寸（高 DPI 安全）
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
        r2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(Vec2::splat(-50.0), Vec2::splat(100.0)),
            Color::GREEN,
            Transform2D::default(),
            0.0, // layer
        );
        r2d.render(&ClearConfig { color: Some(wgpu::Color { r: 0.1, g: 0.1, b: 0.2, a: 1.0 }), depth: None, stencil: None });
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    rjw_main::run_app(App {
        render: None, r2d: None,
        cam: Camera2D::default(),
    })
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
- `vp_matrix() = projection_matrix() * view_matrix()`，**列主序**，可直接喂给 `render2d.set_mvp(...)`（无需转置——Render2D 直接透传）。
- `screen_to_world(screen_px)` / `world_to_screen(world)`：屏幕像素 ↔ 世界坐标互转。**内部会做 Y 翻转**（屏幕像素 Y↓，世界 Y↓ 恰好一致，转换中无需自己再翻）。
- `world_transform()`：把相机当作 `Transform2D`（用于 UI 反父级运算）。

### 3.3 窗口中心 ↔ 世界

**游戏画面在窗口中心** = 相机锁定玩家：

```rust
// 每帧让相机平滑跟随玩家：玩家=世界点，相机重心=窗口中心 ⇒ 画面居中
self.cam.position += (player.pos - self.cam.position) * (1.0 - (-20.0 * dt).exp());
```

---

## 4. 绘制模型：Render2D 的批处理管线

```
【每帧】  add_sprite2d_* / add_mesh / add_polygon_*（命令录制）
              │
              ▼
        Render2D::render(&ClearConfig)
              │
   ① prepare()：sort_layer_then_states() 排序
   ② 实例数据按 MAX_INSTANCES_PER_DRAW(4096) 分页
   ③ draw()：逐页绑定缓冲 → draw_indexed（多次绘制，合批）
   ④ 提交并呈现
```

### 4.1 两种绘制路径

| 路径 | 方法 | 说明 |
|---|---|---|
| **Sprite（实例化）** | `add_sprite2d_default(rect,color,transform,layer,texture)` | 同纹理相邻批次可合批，性能最优 |
| **Sprite（纯色）** | `add_sprite2d_default_solid(...)` | 内部用 1×1 白色纹理 |
| **Mesh（非实例化）** | `add_mesh(verts, tris, color, layer)` | 世界坐标顶点直通 VP；适合圆/多边形/图形 |
| **Mesh（便捷）** | `add_polygon_fan` / `add_polygon_strip` / `add_mesh_fn*` | 画圆、线、任意网格 |

### 4.2 `SpriteRect`（位置/大小/UV）

```rust
SpriteRect {
    mesh_tl: Vec2,   // 世界坐标左上角（本地 space 的 rect 左下 → 最终位置）
    mesh_wh: Vec2,   // 世界尺寸
    uv_tl:   Vec2,   // 归一化 UV 左上
    uv_wh:   Vec2,   // 归一化 UV 尺寸
}
```

- `from_texture(tl, wh)`：整张纹理铺满。
- `from_texture_px(tl, wh, uv_tl_px, uv_wh_px, inv_tex_wh)`：按像素取纹理子区域（sprite sheet）。

### 4.3 `ClearConfig`

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
Sort 是稳定的 (layer, states) 排序——**同 layer 按录制顺序**；不同纹理决定是否合批。
UI 应给一个**很大的固定值**（如 `1e7`），避免被 y-sort 世界坐标覆盖。

### 5.2 y-sort：RPG 纵深感的标准做法

想让「屏幕下方的世界物体盖住上方的物体」：

```rust
const LAYER_Y_SORT_BASE: f32 = 10.0;   // 所有“立绘”从这里起步
fn y_layer(foot_y: f32) -> f32 { LAYER_Y_SORT_BASE + foot_y }

// 绘制时传 foot_y（脚底世界 Y）：
add_sprite2d_default(rect, color, tf, y_layer(entity.foot_y), &tex);
```

引擎对 (layer, states) 排序后，Y 大（靠下）的物体自动盖住 Y 小（靠上）的物体——**无需手动穿插绘制调用**。同 Y 的细节遮挡用小数偏移：本体 `+0.0`、血条 `+0.1/+0.2`、攻击弧 `+0.3`。

> ⚠️ 若图层里有 UI：UI 必须用远超 `y_layer` 上限的常量层（本 RPG 用 `1e7`）。

---

## 6. 纹理与合批

- 创建：`render2d.create_texture(label, &rgba8_data, w, h)`（RGBA8，`len == w*h*4` 否则 panic）。返回 `ArcTextureWrapped`。
- 合批：**同一 `ArcTextureWrapped` 的相邻（同 states）Sprite 自动合批**；换纹理即断批。
- 纹理池：`Render2D.textures` 持有 `Arc`，防止释放。

**关于 1×1 白色纹理**：纯色 Sprite 使用 `white_texture`（uid 独立），`add_sprite2d_default_solid` 就是克隆它来录制的。

---

## 7. 输入：键盘 / 鼠标

### 7.1 键盘——`KeyState`（重点：边沿）

`ctx.keyboard.get(KeyCode::KeyW)` 返回 `KeyState`：

| 方法 | 含义 |
|---|---|
| `.pressed()` | 当前是否按住 |
| `.released()` | 是否没按 |
| `.down_edge()` | **本轮“按下”边沿**（按下的那一刻触发一次，操作系统可能会在你按下时触发多次，不想要可以使用 `down_true_edge()`） |
| `.up_edge()` | **本轮“松开”边沿**（同上） |
| `.true_edge()` / `.down_true_edge()` | 真实边沿（按住的时候不会反复触发） |
| `.sudden_up()` | 突然松开（未在上一帧处于按下状态） |

> ⚠️ **易混淆**：`pressed()` 是「按住」，每帧都 true；`down_edge()` 只在「按下动作发生的那一帧」true——**攻击/跳跃等瞬时操作必须用 `down_edge()`**，否则每帧触发多次。

键盘常量：`KeyCode::KeyW/KeyA/KeyS/KeyD`、`KeyCode::ArrowUp/...`、`KeyCode::Space`、`KeyCode::Escape`、`KeyCode::KeyR` 等（winit `KeyCode`，经 `rjw_main` 重导出）。

### 7.2 鼠标

```rust
ctx.mouse.get_mouse_position()                        // (x,y) 窗口像素
ctx.mouse.get_mouse_delta()                           // 本帧移动增量
ctx.mouse.get_mouse_button_state(MouseButton::Left)   // KeyState（同理用 down_edge 表示“点击”）
ctx.mouse.get_wheel_delta()                           // (x, y) 滚轮
ctx.mouse.get_pixel_wheel()                           // 触控板像素级滚轮
ctx.mouse.in_window()                                 // 是否在窗口内
```

世界坐标系下用 `cam.screen_to_world(Vec2::new(mouse.0, mouse.1))` 得到鼠标指向的世界点。

---

## 8. Transform2D 变换

```rust
pub struct Transform2D {
    pub pos:  Vec2,   // 位置（世界）
    pub scale: Vec2,  // 缩放
    pub rotation: f32, // 旋转（弧度）
}
```

- 构建器：`with_pos(..)` / `with_scale(..)` / `with_rot(..)` / `with_move_by` / `with_walk_by` / `with_scale_by` / `with_rotate_by`。
- **旋转中心**：`rotation` 绕 **变换原点**（即 `pos`）旋转——所以精灵要「绕中心转」，矩形应写成 `SpriteRect { mesh_tl: Vec2::splat(-w/2), mesh_wh: Vec2::splat(w) }`，Transform 的 pos 落在中心。
- `transform_point` / `inverse_transform_point`：局部 ↔ 父空间点变换。
- `with_transform(&parent)`：组合父子层级；UI 命中检测可用 `inverse_transform_point`。

---

## 9. 颜色：Color 与 ColorF64

| 类型 | 存储 | 常用构造 | 用途 |
|---|---|---|---|
| `Color` | `f32` ×4 | `rgba(r,g,b,a)`、`rgba_u8(..)`、`rgb(..)`、`rgb_u8(..)`、常量 `RED/GREEN/...` | 绘制命令 |
| `ColorF64` | `f64` ×4 | `rgba(f64,..)` | 可直接 `.into()` `wgpu::Color`（清屏） |

> `Color` 与 `wgpu::Color` 不直接互转；清屏用 `ColorF64(...).into()` 或手写 `wgpu::Color`（不建议）。

---

## 10. 时间：DeltaTimer

```rust
ctx.timer.dt()              // 帧间隔秒（Duration）
ctx.timer.dt().get_f32()    // 帧间隔秒（Duration）的 f32 转换
ctx.timer.get_fps()         // 当前 FPS
```

每帧用 `dt` 做位移：`pos += vel * dt`；注意 `dt` 在窗口拖动时可能很大，建议 `dt.min(0.05)` 防跳变。

---

## 11. 窗口与事件循环

- 入口：`run_app(App::new())`，`App` trait 必须实现 `primary_window_attrib`（可选）、`on_init`、`about_to_wait`；`on_resized` 可选。
- `MainContext`：`keyboard` / `mouse` / `timer` / `primary_window()` / `request_exit()`。
- `on_init` 中创建 `RenderContext` + `Render2D`（注意：`Render2D` 持有 surface 的 `'static` 引用，`RenderContext` 必须比 Render2D 活得久——本框架中 RenderContext 存活到事件循环结束，天然满足）。
- `on_resized`：`render.resize(w,h)` + 更新相机视口 `cam.set_vp`。
- 窗口属性：`.with_title(...)` / `.with_inner_size(LogicalSize::new(...))`——**LogicalSize 是逻辑像素**，高 DPI 屏会自动映射为更大的物理像素。

---

## 12. 视口 / 缩放 / 高 DPI

| 概念 | 说明 |
|---|---|
| **LogicalSize** | 窗口逻辑尺寸（对人/布局友好） |
| **物理像素** | 实际屏幕像素 = 逻辑 × DPI scale |
| **`render.size()`** | **物理像素**，必须用它建相机视口和 `set_vp`，否则高 DPI 下画面偏移 |
| 相机 `viewport_size` | 物理像素；窗口 resize 时经 `on_resized` 同步 |

> ⚠️ **最容易翻车**：拿 `LogicalSize` 直接当相机视口 → 高 DPI 笔记本上画面偏左上、尺寸不对。一律用 `render.size()`。

**缩放**（滚轮放大缩小）：
```rust
let wheel = ctx.mouse.get_wheel_delta();  // (x, y)
cam.zoom *= Vec2::splat(1.1_f64.powf(wheel.1) as f32);
```

---

## 13. 性能与内存约定

- `Render2D` 内部 `buf_*` 全部常驻复用（`clear()` 只清长度不释放），避免每帧堆分配。
- **实例缓冲是“页池”**：单帧总实例数可远超 4096，`prepare()` 自动按 `MAX_INSTANCES_PER_DRAW` 分页、`draw()` 逐页绑定/绘制——**不要自己砍精灵数量去“凑”4096**。（注：实际容量已经加到了 8192，正考虑更好的扩容方式，对于静态瓦片地图，可以考虑一下静态方式，但目前尚未给出，敬请期待）
- 页池按需一次性增长、永久复用，不在渲染循环中反复分配。
- Mesh 顶点走 u16 索引，单帧顶点数 ≤ 65535；顶点/索引缓冲按需 2× 扩容。
- 每帧录制命令数不设硬上限，但推荐：可见性剔除（相机外的瓦片不要画）——见 `examples/eg260731RPG` 的 `draw_tiles`（按相机 AABB 裁剪）。

---

## 14. 对 AI 的维护约定

给接手改代码的 AI 助手的清单：

1. **坐标系**：Y+ 向下。写“向上移动”用 `y -=`。
2. **逻辑像素 ≠ 物理像素**：相机一律用 `render.size()`（物理）。别拿 `LogicalSize` 直接做相机视口。
3. **瞬时操作用 `down_edge()`**；持续操作用 `pressed()`。
4. **透明覆盖问题（大坑）**：`Queue::write_buffer` 在 `submit` 前会**全部先执行**。往同一个实例缓冲反复写入多批会导致最后一批覆盖前面的——**所以引擎用“页池”：每页只写一次、绑定对应页**。改 `draw()` 时不要退回“单缓冲逐批写”。
5. **Layer 数值小先画**；RPG 里实体/地形用 y-sort 动态 layer，UI 用 ≥1e7 固定层。
6. **纹理合批**：相邻同 `ArcTextureWrapped` 才合批；夹别的纹理就断批。
7. 纹理数据长度必须 `w*h*4`；`create_texture` 会对不上 panic。
8. 改公共 crate（`rjw_2d_render` 等）后，务必 `cargo check --workspace` 防回归。
9. 新增瓦片/实体时，确认 `Tile::is_blocked` / 绘制 `match` 分支 / 纹理创建三点一致 +1。
10. 敌人 AI、波次等逻辑都在 `update()`；绘制分离在 `draw_*`，两者不要互相穿插状态变更。

---

## 15. 快速速查表

| 要做的事 | 代码 |
|---|---|
| 键盘 W 按住 | `ctx.keyboard.get(KeyCode::KeyW).pressed()` |
| 空格“按下那一下” | `ctx.keyboard.get(KeyCode::KeySpace).down_edge()` |
| 空格“只是按下那一下，按着的时候不会连续触发” | `ctx.keyboard.get(KeyCode::KeySpace).down_true_edge()` |
| 鼠标左键点击 | `ctx.mouse.get_mouse_button_state(MouseButton::Left).down_edge()` |
| 鼠标世界坐标 | `cam.screen_to_world(Vec2::new(mx, my))` |
| 圆心在 `p`、半径 `r` 的圆 | `add_polygon_fan(&[p, p+r*(cos,sin)...], color, layer)`（见 RPG `draw_circle`） |
| 绕中心旋转的精灵 | `SpriteRect{mesh_tl: Vec2::splat(-w/2),...}` + `Transform2D::with_pos(c).with_rot(a)` |
| 让画面跟随玩家（居中） | `cam.position += (player.pos - cam.position) * (1-exp(-k*dt))` |
| UI 固定最顶层 | `layer = 1e7`（超过任何 y_layer 上限，理论上） |
| 退出 | `Esc` → `ctx.request_exit()` |
| 程序化纹理 | `create_texture(label, &rgba_vec, w, h)` |

---

*遇到报错请优先怀疑：坐标系方向、物理/逻辑像素、`down_edge` vs `pressed`、页池覆盖（勿改回单缓冲）、layer 数值大小。这五类占引擎使用失误的 95%。*