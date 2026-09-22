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
├─ rjw_krusie      # ★ 统一入口（聚合，无实现）：一行 `use rjw_krusie::prelude::*;` + 按需命名空间
├─ rjw_main        # 平台底座：winit / 输入 / 计时重导出 + 默认窗口标题（事件循环在 rjw_krusie::runtime）
├─ rjw_render      # 底层渲染上下文：RenderContext / 纹理 TextureWrapped / 静态网格 MeshData / 泛型注册表 TypedRegistry / wgpu 重导出
├─ rjw_2d_render   # ★ 2D 批渲染器：Render2D / SpriteRect / Mesh / StaticMesh / RStates / 分页实例缓冲 / 统一管线
├─ rjw_atlas       # ★ 运行时图集：DynamicAtlas（Guillotine 空闲矩形 + 寿命 + clamp_margin + 去碎片重排）+ StaticAtlas（TOML）
├─ rjw_text        # ★ 文本渲染：cosmic-text 排版 + swash 光栅化 + DynamicAtlas 字形缓存 + 责任链
├─ rjw_ui          # ★ UI：hybrid 模式（立即外观 + ID 持久状态）+ DOM 风格自动尺寸 + Tkinter 布局（pack/grid/place）
├─ rjw_transform   # Transform2D + Camera2D（正交相机、VP 矩阵、坐标转换）
├─ rjw_color       # Color(f32) / ColorF64(f64) + 常用常量（RED/GREEN/...）
├─ rjw_keyboard    # 键盘输入 → KeyState（含 `chars()` 字符输入 / IME）
├─ rjw_keystate    # KeyState 边沿状态机（pressed/edge/true_edge）
├─ rjw_mouse       # 鼠标位置/增量/滚轮/按钮状态
├─ rjw_time        # DeltaTimer（帧间隔 dt 与 FPS）
├─ rjw_collision   # 碰撞（矩形相交等）
└─ rjw_tilemap     # 瓦片地图（chunk + 组 + 相机剔除）

examples/
├─ egHello            # ★ 最小完整骨架（App::config/init/update + Ctx/Frame 守卫 + submit）
├─ egMultiView        # ★ 一帧多画面（多个 region + 多次 submit + `viewport_borders` 描边调试）
├─ eg260729           # 最小清屏示例（Clear 枚举）
├─ eg260731           # Render2D 精灵/多边形/mesh 能力演示
├─ eg260806CustomDraw  # custom / CustomDraw 逃逸舱口（自建管线三角形）
├─ eg260731RPG        # ★ 综合 RPG：y-sort、波次系统、相机跟踪、程序化纹理、静态地形（石头/花经 StaticMesh 合批）
├─ eg260810TextChain  # ★ 文本责任链（TextStyle → Label → draw）演示
├─ egTilemap          # ★ 瓦片地图（chunk + 相机剔除 + 屏幕固定 HUD）
├─ egDebugDraw        # ★ 调试图元（DebugPainter：线 / 框 / 圆 / 十字 / 网格）
└─ eg260818UI         # ★ rjw_ui 示例：pack 菜单 + grid 背包 + place 状态栏 + 输入框/滑块/单选
```

**最核心概念一条线**：

```
App (impl rjw_krusie::App)                    ← 游戏侧唯一 trait
 └─ config(): AppConfig                       ← 窗口 / 渲染 / 清屏 / 后台策略
 └─ init(gfx: &Gfx): 建资源（纹理 / 网格 / 字体 / 图集）
 └─ update(ctx: &mut Ctx)（每帧必调）:
      let Some(mut f) = ctx.frame() else { return };   ← 无帧则不执行渲染代码（逻辑照常）
     读输入(键盘/鼠标) → 更新逻辑 → 摆相机(Camera2D)
     → f.draw().sprite(..)/mesh(..)/polygon(..)/region(..)/static_mesh(..)/custom(..) + 链式 RStates
     → f.ui(theme) 开一段录 UI（可多段 / 任意位置） / f.text(|t| ..)（世界文本）  ← 可选：overlay 层
     （f 析构自动 present；未 submit 则按 AppConfig::clear 收尾）
```

---

## 2. 快速上手：最小程序

```rust
// 整套库：一行起步（运行时 + 绘制 + 相机 + 颜色 + 文本 + UI 全部到位）
use rjw_krusie::prelude::*;

#[derive(Default)]
struct Game {
    cam: Camera2D,
    t: f32,
}

impl App for Game {
    /// 窗口 / 渲染 / 清屏 / 后台策略（唯一描述窗口的地方）
    fn config(&self) -> AppConfig {
        AppConfig::new("my app").size(1280.0, 720.0)
    }

    /// 只调用一次：用 `Gfx` 建长期资源（纹理 / 网格 / 文本子系统…）
    fn init(&mut self, _gfx: &Gfx) {}

    /// 每帧调用（**包括取不到表面的帧**）
    fn update(&mut self, ctx: &mut Ctx) {
        // ① 逻辑半程：无帧也执行（后台模拟 / 计时 / 输入状态持续）
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }
        self.t += ctx.dt();

        // ② 渲染半程：守卫在应用里 —— 无帧时这段代码一行都不执行
        let Some(mut f) = ctx.frame() else { return };

        // 相机自己存视口：先写回画面矩形，再画（屏幕↔世界换算也据此）
        self.cam.set_region(f.region());

        // 在屏幕中心画一个 100×100 绿色方块（世界坐标）
        f.draw()
            .solid(SpriteRect::new((-50.0, -50.0), (100.0, 100.0)))
            .tint(Color::GREEN)
            .layer(0.0);

        // 一个画面 = 一次 submit(相机, clear)；present 可省略（Frame 析构自动呈现）
        f.submit(&mut self.cam, Clear::color(Color::rgb(0.1, 0.1, 0.2)));
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    run(Game::default())
}
```

替换旧骨架时最容易踩的三点：

| 旧写法 | 新写法 |
|---|---|
| `on_init` 里自建 `RenderContext` / `Render2D` 并存进 `Option` 字段 | 不需要：运行时各持一个世界层与一个 UI 层渲染器 |
| `on_resized` 里 `render.resize` + `cam.set_vp(..)` | 不需要：surface 自动重配，`f.region()` 自动跟随；`submit` 写回 `cam.region` |
| `about_to_wait` 里 `begin_frame()` / `ClearConfig` / `present()` | 不需要：`f.submit(..)` + `Frame` 析构（自动清屏与 present） |

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
| 世界单位 | 像素（一张 32×32 纹理贴到 `SpriteRect::new` 的 32×32 世界区域，1:1） |
| 视口中心 | 等于 `Camera2D.position`（相机所在的世界点） |

> ⚠️ **最易混淆**：写 `pos = Vec2::new(x, y)` 时，`y` 增加 = 往下走。
> 键盘「W 上移」应写 `pos.y -= 速度`；「S 下移」应写 `pos.y += 速度`。
> 参考 `examples/eg260731RPG` 中 W→`dir.y -= 1` 的写法。

### 3.2 `Camera2D` 字段与行为

```rust
pub struct Camera2D {
    pub region: Rect,            // 画面矩形（屏幕像素，左上原点）
    pub transform: Transform2D,  // 相机在世界中的位姿：pos / rotation / scale
}
// transform.scale = 世界单位/像素（越大视野越广）；`zoom`（越大越放大）是派生视图：zoom = 1/scale
```

- `Camera2D::new(region)` / `Camera2D::full((w, h))`：位姿默认 `IDENTITY`（无旋转、zoom 1、世界原点居画面中心）。
- 位姿与坐标方法经 `Deref/DerefMut` 直接来自 `Transform2D`：`cam.move_by(..)` / `cam.transform_point(..)`；
  字段赋值写 `cam.transform.pos = ..`。
- `vp_matrix() = projection_matrix() * view_matrix()`，**列主序**，由 `f.submit(&mut cam, clear)` 自动取用（不需要手动喂）。
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
【每帧】  sprite / solid / region / quads / mesh / polygon / static_mesh / custom（命令录制，返回 Builder）
              │ (可选链式 .tint(..).states(RStates) 设置状态；Drop → push 到 DrawCommandQueue)
              ▼
        Frame::submit(&mut cam, Clear)          // = 写回 cam.region → 取 VP → 开 pass → 提交 → 清帧
              │   （独立使用 Render2D 时：frame.pass(clear) + r2d.submit(&mut pass, &cam)）
   ① prepare()：按 SortMode 排序（LayerAndStates 默认）
   ② RStates resolve（None → 全局默认状态 `Render2D::states()`）
   ③ 实例数据按 MAX_INSTANCES_PER_DRAW(8192) 分页
   ④ draw()：按 DrawOp.rstates 取/建管线（key 含「本 pass 是否绑深度附件」）→ 逐页 draw_indexed
   ⑤ 帧级 VP 槽环：每个画面一个槽，动态偏移绑定（多画面互不串味）
   ⑥ **present**（`Frame` 析构统一收尾：`submit` 只落 pass，**呈现只做一次**；
      `AppConfig::clear` 仅在应用**从未取帧/从未 submit** 时兜底）

> ⚠ **「提交」与「呈现」是两件事**：`f.submit(..)` 只把队列落成一个 pass，
> 真正的 `encoder.submit + surface.present` 由 `Frame` 析构（或显式 `f.present()`）完成。
> 若某处让 `Frame` 以为"已经 present 过"而跳过收尾，画面就**永远是空白**
> （`RUST_LOG=warn` 下会看到 `RenderFrame 未 present 就被丢弃`）。
> 冒烟开关 `--frames N` 收尾会打印 `[OK] N iterations / N frames presented`；
> **0 呈现直接退出码 2**，用来挡住"能跑但没画面"的回归。
```

### 4.1 统一管线架构

Sprite、StaticMesh 与动态 Mesh **共用同一 `vs_main` 入口 + slot0(顶点)/slot1(实例) 布局**。

- **Sprite**：顶点用注册的四边形网格（`quad_mesh_id`），slot1 绑实例页缓冲 → `draw_indexed(quad_indices, 0, N_instances)`
- **StaticMesh**：顶点用 `MESHES` 注册表中用户网格，slot1 绑实例页缓冲 → `draw_indexed(mesh_indices, 0, N_instances)` —— 同 mesh_id 的实例自动合批
- **动态 Mesh（mesh / polygon / quads）**：顶点每帧上传 `draw_page.mesh_vb/mesh_ib`，slot1 绑 identity 实例（`mesh_tl=0, mesh_wh=1, mesh_pos = pos 直通`）→ `draw_indexed(段索引范围, 0, 1)`

渲染状态（Blend / DepthStencil / Cull / Polygon / FrontFace / Conservative / **Sampler**）全部**按 RStates 从管线缓存中自动获取或创建**，无需手动管理管线。
**采样器由 RStates 位域（bits 8..24）驱动**：`.samp_mag(Nearest)` / `.samp_addr_u(Repeat)` 会真正创建对应 GPU 采样器（`Render2D` 内部缓存）；bind group 由 `Render2D` 按 `(tex_uid, samp_key)` 缓存。

### 4.2 绘制类别

| 类别 | 方法 | Builder | 说明 |
|---|---|---|---|
| **Sprite（贴纹理）** | `sprite(rect, &tex).color(..).transform(..).layer(..)` | `SpriteBuilder` | 同纹理+同 RStates 合批 |
| **Sprite（纯色）** | `solid(rect).color(..).transform(..).layer(..)` | `SpriteBuilder` | 内部用 1×1 白纹理 |
| **Mesh（数据）** | `mesh(&verts, &tris).color(..).transform(..)` | `MeshBuilder` | 世界坐标顶点直通 VP，每帧上传 |
| **Mesh / 多边形（流式）** | `mesh_with(\|s\| ..)` / `polygon(&verts)` / `polygon_with(\|p\| ..)` / `polygon_uv(&verts, &uvs)` | `MeshBuilder` | 画圆、线、任意网格（流式版零临时 `Vec`） |
| **四边形段** | `quads(&verts, &tex).transform(tf).color(tint)` / `quads_with(\|q\| .., &tex)` | `MeshBuilder` | 顶点 TL,TR,BL,BR（UI 整段提交用） |
| **StaticMesh** | `static_mesh(id, &tex).color(..).transform(tf).layer(..)` | `StaticMeshBuilder` | 注册表网格 + 实例化合批（GPU 顶点常驻） |
| **逃逸舱口** | `custom(cd).layer(..)` | `CustomBuilder` | 注入原生 wgpu 绘制 |

### 4.3 `SpriteRect`（位置/大小/UV）

```rust
SpriteRect {
    mesh_tl: Vec2,   // 世界坐标左上角
    mesh_wh: Vec2,   // 世界尺寸
    uv_tl:   Vec2,   // 归一化 UV 左上
    uv_wh:   Vec2,   // 归一化 UV 尺寸
}
```

- `SpriteRect::new(tl, wh)`：**整张纹理**铺满（最常用；参数可为 `Vec2` 或 `(x, y)`）。
- `SpriteRect::centered(center, wh)`：以中心点 + 尺寸（旋转 / 居中绘制更自然）。
- `SpriteRect::with_uv(tl, wh, uv_tl, uv_wh)`：手动归一化 UV。
- `SpriteRect::with_uv_px(tl, wh, uv_tl_px, uv_wh_px, tex_wh)` / `with_uv_tex(.., &tex)`：
  像素 UV 子区（内部归一化，**不用自己算 `1/尺寸`**）。
- 链式调整：`.at(pos)` / `.move_by(delta)` / `.size(wh)` / `.uv(..)` / `.uv_px(..)` /
  `.shrink(每边量)`（世界）/ `.shrink_uv(每边量)`（归一化 UV，传 `0..1` 比例）。
- 裁剪类特效（每边收窄 / 越界外扩）统一走上面的 `with_uv_px` + `Edges`（`shrink` / `shrink_uv`，负值即外扩，不 clamp）；
  图集精灵直接用 `AtlasSprite`（`r2d.region(..)`）。

### 4.4 `Clear`（清屏意图）

```rust
Clear::Keep                          // 全部保留（叠加画面 / 多 pass）
Clear::color(Color::rgb(..))         // 只清颜色（接受 Color 或 ColorF64）
Clear::color_depth(Color::BLACK, 1.0)// 颜色 + 深度
Clear::depth(1.0)                    // 只清深度（保留颜色；重叠画面的独立 pass）
Clear::stencil(0)                    // 只清模板
```
`Clear` 是**枚举**（v0.3 起取代三 `Option` 的 `ClearConfig`）：不需要 wgpu 类型、不需要自己判断
「是否需要深度附件」——`PassBuilder` 从已排队的 recorder + clear 自动推导。

#### 4.4.1 多画面：`f.set_region(..)` + 多次 `submit`

```rust
let left  = Rect::new(0.0, 0.0, 960.0, 1080.0);
let right = Rect::new(960.0, 0.0, 960.0, 1080.0);
let mini  = Rect::new(1370.0, 12.0, 538.0, 367.0);   // 画中画，压在 right 上

f.set_region(left);  f.submit(&mut cam_l, Clear::color(SKY));
f.set_region(right); f.submit(&mut cam_r, Clear::color(SKY));
f.set_region(mini);  f.submit(&mut cam_m, Clear::depth(1.0));  // 只清深度，颜色保留
```

- **一个画面 = 一次 `submit(相机, clear)` = 一个 pass**；`submit` 会把 `region` 写回 `cam.region`
  （屏幕↔世界换算据此），并占用一个**帧级 VP 槽**（多画面互不串味）；
- **`submit` 是"提交即清帧"**：世界层与 UI 层的命令队列都会被清空 ⇒ 每个画面**先录制
（`f.draw()` / `f.draw_ui()` / `f.text_ui()` / `f.ui(..)`）再 `submit`**；交错写入
  （录左屏 → 提交左屏 → 录右屏 → 提交右屏）才是每个画面各自见自己的内容。
  把全部内容录完再连续 `submit` 两次，后一次的队列已空（只在第一个画面里出现）。
- 每个画面的 UI 层有自己的坐标系：UI 层坐标 = **该画面矩形左上角为原点的物理像素**
  （`f.draw_ui()` 的 (0,0) 就是本画面左上角），所以分屏各画各的 HUD 不用做任何偏移换算；
- `f.region()` 在 `set_region` 之后立刻返回新矩形；UI 层（`draw_ui` / `text_ui` / `ui`）的坐标空间
  是**整屏左上原点物理像素**，与当前 `region` 无关，所以各画面的 UI/描边不会被画面矩形裁掉；
- ⚠ **颜色清屏只对第一个画面生效**：wgpu 的 load-op 清屏作用于**整张附件**，
  所以第 2..N 个画面的 `Clear::color(..)` 会被运行时**降级**为「`Clear::Keep` + 在画面矩形内铺一块该颜色的底」
  ⇒ 语义仍是"把这个画面清成该颜色"，但**不会抹掉已经画好的别的画面**（这正是"左分屏全黑"的根因）。
  需要 `Clear::without_color()` 这种"去掉颜色只留深度/模板"的写法时运行时自动完成，用户无需手写。

**调试辅助：给每个画面描边**

```rust
AppConfig::new("游戏").size(1280.0, 720.0)
    .viewport_borders(ViewportBorders::On)   // 帧末在最上层描边 + 标注 `#序号 宽×高`
```

- 颜色按画面序号循环（红 / 绿 / 蓝 / 黄）；只描边不填充；
- 实现是**帧末一次独立的全屏 overlay pass**（`Clear::Keep`），在 present 之前提交
  ——不能在各画面自己的 pass 里画（会连同别的画面的边框一起被该画面的裁剪矩形切掉）；
- 配合 `RUST_LOG=rjw_krusie=debug` 可打印每个画面的 `region` 与清屏意图。
  详见 `docs/DEBUGGING.md` §3。

### 4.5 外部自定义绘制（`custom` / `CustomDraw`）

引擎的**逃逸舱口**：在 `Render2D` 自带的统一管线之外，注入任意原生 wgpu 绘制调用，如自定义 shader、线框调试、后处理或特殊顶点格式。

#### 核心 API

| 项 | 说明 |
|---|---|
| `CustomDraw` trait | `fn draw(&self, pass: &mut wgpu::RenderPass<'_>)`；`Send + Sync` 约束 |
| blanket impl | 闭包 `Fn(&mut wgpu::RenderPass) + Send + Sync` 自动实现该 trait |
| `custom(cd)` | 返回 `CustomBuilder`；`.layer(..)` + 链式 RStates 参与排序 |
| `CustomBuilder` | 与 `SpriteBuilder` / `MeshBuilder` 相同的责任链（`.texture()` 对 custom 无效） |

#### 用法

**👉 完整可运行示例见 [`examples/eg260806CustomDraw/`](../examples/eg260806CustomDraw/src/main.rs)**（`cargo run -p eg260806CustomDraw`）：演示结构体形式（`Tri` 自建管线）+ 闭包形式 + 与引擎 Sprite 混排 3 层。

```rust
// ① 闭包形式（最常用）
r2d.custom(|pass| {
    // pass 已由引擎打开——不要调用 begin_render_pass / end
    // 可以 set_pipeline、set_vertex_buffer、draw 等
})
.layer(1.0);

// ② 结构体形式（可复用、持有共享资源）
#[derive(Clone)]
struct Wireframe { mdl: wgpu::RenderPipeline }
impl rjw_2d_render::CustomDraw for Wireframe {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.mdl);
        // ...
    }
}
r2d.custom(Wireframe { mdl }).layer(96.0);
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

> ⚠️ **pass 生命周期**：`custom` 的闭包在 `render()` / `record()` 内部的 `draw()` 阶段调用。此时 RenderPass 已 `begin`，请在闭包内**使用**，而不要再 `begin_render_pass`。

#### 排序与执行时机

- 排序键 = `(layer, states)`，与 Sprite/Mesh 一致。`CustomBuilder` 链式的 `.blend(...)` 等 RStates 决定其在同一 layer 内相对 Sprite/Mesh 的先后。
- 每帧 `render()` 结束时 `buf_custom_draws.clear()`——**不要在闭包中缓存跨帧状态**；需要持久资源请 `Arc` 捕获并在 `CustomDraw` 结构体中保存。
- `record()` 同样执行 custom draws（用于用户自建 pass 的内部子 pass）。

#### 与引擎管线的边界

| 场景 | 推荐 |
|---|---|
| 普通 Sprite/Mesh（引擎已覆盖） | 直接用 `sprite*` / `mesh*`（可合批、状态缓存） |
| 特殊混合/着色、调试线框、后处理 | `custom` 注入原生 wgpu |
| 完全独立于 `Render2D` 的渲染 | 用 `acquire_frame()` / `record()` + 自建 pass，或直接在事件循环自建 encoder |

### 4.6 静态网格 StaticMesh（GPU 顶点常驻 + 实例化合批）

当一批元素**位置/纹理/层级固定、不参与实体 y-sort** 时（地图装饰如石头、花、栅栏…），应使用 `register_mesh` + `static_mesh*` 静态化：

- **`MeshData`**（`rjw_render`）在 GPU 上持有顶点/索引缓冲，`register_mesh` 注册进全局 `MESHES`，返回 `mesh_id`。
- **`static_mesh(id, &tex).color(c).transform(tf).layer(l)`** 每帧只提交一个轻量实例（变换 + 颜色）；同 `mesh_id` + 同 RStates + 同纹理的实例自动合批为极少数 `draw_indexed`。
- **共享网格模式**：为"圆形"等常见图形只建一个**单位网格**（半径为 1），实例变换用 `Translate(pos) * Scale(r)`——整张地图几百个圆共享同一顶点缓冲，DrawCall 从"每圆一次动态提交"降到每层 1 次。
- **⚠️ 哪些元素不能静态化**：会**插入实体绘制顺序**的元素（如 RPG 中 `y_layer(foot_y)` 的树）必须保持动态路径（Sprite / `mesh*`），否则遮挡关系错误。固定 `LAYER_TERRAIN` 之类层级的元素才安全。
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

// 绘制时把 foot_y（脚底世界 Y）换算成 layer：
f.draw()
    .sprite(rect, &tex)
    .tint(color)
    .transform(tf)
    .layer(y_layer(entity.foot_y));
```

引擎对 (layer, states) 排序后，Y 大（靠下）的物体自动盖住 Y 小（靠上）的物体。同 Y 的细节遮挡用小数偏移。

---

## 6. 纹理与合批

- 创建：`gpu.texture(label, Rgba8::new(&rgba8_data, (w, h)))`（RGBA8，`len == w*h*4` 否则 panic）。
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
use rjw_atlas::{AtlasConfig, DynamicAtlas};

// 只要一个能力对象：`Gfx`（`Deref<Target = Gpu>`）——不再手动穿 device/queue/layout
let mut atlas = DynamicAtlas::new(
    gfx,
    AtlasConfig {
        max_pages: 2,      // 最多 2 页
        padding: 0,        // 精灵间像素间距
        lifetime: 200,     // 200 帧未被 `region()` 访问即视为不再需要
        page_size: 2048,   // 单页像素尺寸
    },
);
```

> `DynamicAtlas::new(gfx, cfg)`：`gfx` 可为 `&Gfx`（`Deref` 到 `&Gpu`）或 `&Gpu`。
> 需要接管**已有页纹理**时用 `DynamicAtlas::from_raw(...)`（`rjw_text` 的字形图集
> 即此路径；`rjw_ui::ProcTextures` 已在 v0.4 删除，见 §18.6）。

#### 插入精灵

```rust
// ★ 最常用：key + RGBA8 像素 → Option<AtlasRegion>
let grass = atlas.insert("grass", Rgba8::new(&grass_rgba, (32, 32)));

// 需要控制原点 / 边界扩展时用 InsertOpts
let custom = atlas.insert_with(
    "custom",
    Rgba8::new(&rgba, (32, 32)),
    InsertOpts::new().origin((5, 5)).no_clamp(),
);

// 动态再生（被逐出后按 SpriteSource 重新生成像素，实现自动复活）
let dyn_spr = atlas.insert_dynamic("tile", (32, 32), Box::new(my_sprite_source));
```

`insert` 返回 `None` 表示所有页均已满（达到 `max_pages` 限制）。

| 方法 | 说明 |
|---|---|
| `insert(key, Rgba8)` | 最常用（默认 clamp_margin、非常驻、原点 `(0,0)`） |
| `insert_with(key, Rgba8, InsertOpts)` | `InsertOpts::{origin, no_clamp, permanent}` |
| `insert_dynamic(key, size, SpriteSource)` | 动态再生精灵 |
| `white()` | 1×1 白色像素（纯色填充与字形图集同页合批） |
| `handle(&key)` / `sprite(&handle)` | 稳定句柄（RAII 保活）→ 绘制用 `AtlasSprite` |
| `region(&key)` / `region_or_revive(&key)` | 取区域（后者在寿命到期时复活） |
| `generation()` / `stats()` / `compact()` | 世代号（重排检测）/ 统计 / 去碎片重排 |
| `tick()` | **由运行时每渲染帧驱动**（寿命递减 / 动态再生的时机；应用不自己调） |

> ⚠ `key` / `Rgba8.size` / `InsertOpts::origin` 的旧 5~7 参写法（`insert(name, rgba, w, h, origin_px, clamp_margin)`）
> 已在 v0.3 收敛。

#### 插入白色像素

```rust
let white = atlas.white();
```

插入 1×1 纯白像素，用于纯色填充的 Sprite 合批到同一图集页内，避免纯色绘制使用独立纹理打断合批。

#### 获取 / 刷新寿命

```rust
if let Some(region) = atlas.get("grass") {
    // region.lifetime 被重置为 config.lifetime
}
```

`get()` 命中后刷新该条目的寿命，若 `tick()` 倒计时归零则标记可以移出（逻辑踢出，纹理页不回收）。

#### 帧尾 / 去碎片

```rust
atlas.tick(); // 寿命衰减 → 移除到期条目
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

- **寿命系统**：每个非永久精灵有 `config.lifetime` 帧的倒计时，每次 `region()` / `region_or_revive()` 命中时重置
- **墓碑**：`tick()` 将到期且保存了源数据的精灵移入 `tombstones`（携带 RGBA + 宽高 + 原点）
- **复活**：`region_or_revive(name)` 若在 entries 中找不到 → 从 tombstones 取出 RGBA → 重新 `insert_inner()` 写入图集 → 返回引用
- **常驻精灵**：用 `insert_with(.., InsertOpts::new().permanent())` 或 `insert_with(.., InsertOpts::new().permanent())` 插入的精灵 `source: None`，永久存在

```rust
// 普通精灵（可复活）
atlas.insert("grass", &grass_rgba, 32, 32);

// 若干帧后被踢出…
atlas.tick();

// 下次使用时自动复活
let region = atlas.region_or_revive("grass"); // ← 自动重新插入图集
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

- `tex` 必须在加载前已通过 `TEXTURES` 注册（例如 `gpu.texture(..)` 或 `TextureWrapped` 手动注册）。
- 未找到纹理时返回 `StaticAtlasError::TexNotFound`。

### 7.3 一行绘制：`Render2D::region(AtlasSprite)`

图集的核心价值是**一行绘制**。v0.3 起引擎**自带**图集直达入口（`r2d.region(AtlasSprite)`），
不再需要项目级「`Tex::draw()` 封装」（旧的「`AtlasRegion` → 手搓 `SpriteRect` → `sprite`」四步已收敛）：

```rust
use rjw_atlas::{AtlasConfig, DynamicAtlas, RegionRef, Rgba8};

struct Tex {
    atlas: DynamicAtlas<&'static str>,  // 持有图集（保证页纹理存活）
    grass: RegionRef,                   // 稳定句柄（RAII 保活；图集重排后仍可用）
    player: RegionRef,
}

impl Tex {
    fn create(gfx: &Gfx) -> Self {
        let mut atlas = DynamicAtlas::new(gfx, AtlasConfig::default());
        // 1) 插入像素（`insert` 返回 `Option<AtlasRegion>`）
        atlas.insert("grass", Rgba8::new(&make_grass(), (32, 32)));
        atlas.insert("player", Rgba8::new(&make_player(), (32, 32)));
        // 2) 取稳定句柄（绘制只认句柄，不认名字）
        let grass = atlas.handle(&"grass").unwrap();
        let player = atlas.handle(&"player").unwrap();
        Self { atlas, grass, player }
    }
}
```

绘制（每帧）：

```rust
// 引擎一行直达：`sprite(&handle)` 给出「页纹理 + 区域」，`region(..)` 直接提交
if let Some(spr) = tex.atlas.sprite(&tex.grass) {
    f.draw()
        .region(spr)
        .at(world_tl)          // 初始 mesh 左上角在原点、尺寸 = 图集区域尺寸
        .tint(Color::WHITE)
        .transform(tf)
        .layer(y_layer(foot_y))
        .blend(BlendMode::Additive);
}
```

**关键设计要点**：

1. `Tex` 持有 `DynamicAtlas`，确保图集页纹理不会被释放；`RegionRef` 句柄是 RAII 保活的。
2. `atlas.sprite(&handle) -> Option<AtlasSprite>`（`{ texture, region }`）取代了「`TEXTURES.get(page_uid)` + 手算像素→归一化 UV + `SpriteRect::with_uv_tex` + `sprite`」四步。
3. `region(..)` 返回 `SpriteBuilder`，后续链式状态/RStates 与世界层完全一致。
4. 需要**子区域裁剪**（九宫格 / 特效）时，用 `SpriteRect::with_uv_px(.., tex_wh)` + `r2d.sprite(..)`；`AtlasRegion` 自身带 `tl_px`/`wh_px`，换算即 `with_uv_tex`。

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
| **全局默认** | `r2d.states_mut(RStates::new().blend(Additive).depth_test(true))` | 所有未链式设置状态的命令 |
| **单条绘制** | `f.draw().sprite(..).blend(Multiply)` | 该条命令 |
| **批量设置** | `.states(RStates::new().blend(Additive).sampler(SamplerDesc{..}))` | 同上 |

### 8.2 Builder 使用

```rust
// 不链式 = 继承全局默认状态（默认 Alpha Blend + Linear + No Cull）
f.draw().sprite(rect, &tex);

// 链式覆盖（对象糖）
f.draw().sprite(rect, &tex)
    .samp(FilterMode::Nearest, AddressMode::Repeat)
    .blend(BlendMode::Additive);

// Mesh 可以 .texture(..) 覆盖默认白纹理
f.draw().polygon(&verts)
    .texture(&tex)
    .blend(BlendMode::Multiply)
    .tint(Color::CYAN)
    .layer(96.0);

// 深度状态（对象糖）
use rjw_2d_render::{BlendMode, DepthState, CompareFunc, RStates};
f.draw().sprite(rect, &tex)
    .depth(DepthState { test: true, write: true, compare: CompareFunc::Less });

// 全局默认状态（唯一入口）
r2d.states_mut(
    RStates::new()
        .blend(BlendMode::Additive)
        .depth_test(true)
        .depth_write(true)
        .depth_compare(CompareFunc::Less),
);
```

### 8.3 MeshBuilder 的 `.texture()`

```rust
// Mesh 默认白色纹理；.texture(..) 覆盖
f.draw().mesh(&verts, &tris)
    .texture(&my_tex)
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
| `BlendDesc` / `SamplerDesc` / `RasterState` / `DepthState` / `StencilState` | 状态对象（统一走 `.states(RStates)`，或对象糖 `.blend(..)` / `.sampler(..)` / `.raster(..)` / `.depth(..)` / `.stencil(..)`） |
| `Draw2D<'a, K>` | **唯一 Builder 本体**（别名 `SpriteBuilder` / `MeshBuilder` / `StaticMeshBuilder` / `CustomBuilder`）；kind 标记（`Sprite` / `Mesh` / `StaticMesh` / `Custom`）与 `DrawKind` 为内部机制（`#[doc(hidden)]`），用户经 `r2d.sprite(..)` 等入口获得 |
| `SortMode` / `SortPolicy` / `SortKey` | 排序策略 / 自定义排序 trait / 公开排序键（`rjw_2d_render::sort`） |
| `Cull` / `Culler` | 剔除模式（`Off` / `Viewport` / `Rect` / `Fn`）/ 剔除器（`rjw_2d_render::cull`） |

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

let mut font = Text::new(r2d.device(), r2d.queue(), r2d.texture_layout());

// 左上角单行
font.draw_label(r2d, "Hello", Color::WHITE, 14.0, 18.0, Vec2::new(10.0, 10.0), "SimHei", Align::Left, 0.0);

// 屏幕居中（相机中心）
let _ = font.draw_label_ex(r2d, "GAME OVER\n按 R 重开", Color::RED, 22.0, 28.0, cam.position, "SimHei", Align::Center, LAYER_UI, Vec2::new(0.5, 0.5));
```

---

## 9. 输入：键盘 / 鼠标

> 入口都在 `Ctx` 上：`ctx.key(..)` / `ctx.keys()` / `ctx.button(..)` / `ctx.mouse()`，
> 持帧时同名的 `f.key(..)` / `f.mouse()` 也可用（`Frame` 转发给 `Ctx`）。
> **`next_frame()`（边沿推进）由运行时统一调用**，应用不需要手动 `end_frame()`。

### 9.1 键盘——`KeyState`（重点：边沿）

`ctx.key(KeyCode::KeyW)` 返回 `KeyState`：

| 方法 | 含义 |
|---|---|
| `.pressed()` | 当前是否按住 |
| `.released()` | 是否没按 |
| `.down_edge()` | **本轮"按下"边沿**（按下的那一刻触发一次，可能重复） |
| `.up_edge()` | **本轮"松开"边沿**（同上） |
| `.true_edge()` / `.down_true_edge()` | 真实边沿（按住时不会反复触发） |
| `.sudden_up()` | 突然松开（未在上一帧处于按下状态） |

> ⚠️ **攻击/跳跃等瞬时操作必须用 `down_edge()`**，否则每帧触发多次。

文本 / IME（`rjw_keyboard`，经 `ctx.keys()` 访问）：

```rust
ctx.keys().chars()                    // 本帧上屏字符（含 Shift 组合；控制字符已过滤）
ctx.keys().ime_commits()              // 输入法上屏文本
ctx.keys().ime_preedit()              // 组合中的候选串（UI 输入框以灰色画在光标后）
ctx.keys().ime_preedit_caret()        // 候选串里的光标位置
ctx.keys().keys()                     // 本帧所有按下的键
```

### 9.2 鼠标

```rust
ctx.mouse().pos_px()                      // (x, y) 物理像素（窗口左上原点）
ctx.mouse().motion()                      // 本帧移动增量
ctx.mouse().button(MouseButton::Left)     // KeyState（边沿语义同键盘）
ctx.mouse().buttons()                     // 全部按钮状态
ctx.mouse().wheel()                       // 滚轮 `ScrollDelta`（`to_pixel()` / `to_line()`）
ctx.mouse().in_window()                   // 是否在窗口内
```

> 旧名（`get_mouse_position` / `get_mouse_delta` / `get_mouse_button_state` /
> `get_wheel_delta` / `get_pixel_wheel` / `get_key` / `get_chars`）在 v0.3 **已删除**，
> 换名映射见 `docs/API_DESIGN.md` §8.3。

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
| **`f.region()` / `f.set_region(..)`** | 当前画面矩形（物理像素、左上原点）；不调用即 **整屏**（默认行为，单画面游戏无需关心） |
| **`f.scale()`** | DPI scale（本机 1.5）；把 UI/描边常量按物理像素写时用 `px(v) = v * f.scale()` |
| 相机 `viewport_size` | 物理像素 |

> **多画面**（左右分屏 / 画中画）：用 `f.set_region(rect)` + 多次 `submit`，见 §4.4.1；
> 每个相机的 `region` 由 `submit` 写回，屏幕↔世界换算据此。

**缩放**（滚轮）：
```rust
cam.zoom *= Vec2::splat(1.1_f64.powf(wheel.1) as f32);
```

---

## 15. 性能与内存约定

- `Render2D` 内部 `buf_*` 全部常驻复用（`clear()` 只清长度不释放）。
- **实例缓冲是"页池"**：单帧总实例数可远超 8192，自动分页；**不要自己裁减数量去凑**。
- **统一管线缓存**：`DrawPage` 按 `RStates::raw()` keys 缓存 `RenderPipeline`，首次遇新状态时创建、后续帧直接命中。HashMap 常驻，Query 事件循环保持不变。
- **builder 不产生堆分配**：`SpriteBuilder` / `MeshBuilder` / `StaticMeshBuilder` 均为栈上 struct，Drop 时直接转移到 `DrawCommandQueue` 的 Vec。
- 页池按需一次性增长、永久复用。
- Mesh 顶点走 u16 索引，单帧顶点数 ≤ 65535。
- **静态网格合批**：固定层、不参与 y-sort 的地图元素用 `register_mesh` + `static_mesh*` 静态化；同 mesh_id 的实例自动合并为极少数 DrawCall。常见图形（如圆）用**一个单位网格 + 实例缩放**共享顶点，避免每实例一份缓冲。

---

## 16. 对 AI 的维护约定

给接手改代码的 AI 助手的清单：

1. **坐标系**：Y+ 向下。写"向上移动"用 `y -=`。
2. **逻辑像素 ≠ 物理像素**：相机一律用 `render.size()`（物理）。
3. **瞬时操作用 `down_edge()`**；持续操作用 `pressed()`。
4. **透明覆盖问题（大坑）**：`Queue::write_buffer` 在 `submit` 前会**全部先执行**——所以引擎用"页池"：每页只写一次、绑定对应页。
5. **Layer 数值小先画**；RPG 里实体/地形用 y-sort 动态 layer，UI 用 ≥1e7 固定层。
6. **RStates resolve**：`prepare()` 中 `States.rstates: None` → **全局默认状态**（`Render2D::states()`）；`Some(r)` → 直接用 `r.raw()`。
7. **Builder 是责任链，Drop 自动 push**：`add_*` 返回 builder，不链式调用也自动 push（`rstates: None`）。不要手动 push。
8. **统一管线**：不再有 `sprite_pipeline` / `mesh_pipeline` 分支。所有绘制走 `get_or_create_pipeline(rst_raw)`，Sprite/StaticMesh 绑实例页、动态 Mesh 绑 identity instance buffer。
9. **管线缓存 key = RStates::raw()**：u64 哈希，同一个 raw 值只创建一次管线。
10. **采样器由 RStates 位域驱动**：`.samp_*` 链式方法真实创建 GPU 采样器；`TextureWrapped` **不再持有** sampler / bind group（bind group 由 `Render2D` 缓存）。
11. **静态网格**：`MeshData` 注册进 `MESHES` 后经 `static_mesh*` 实例化；固定层、不参与 y-sort 的元素才静态化，**会插入实体排序的（如 y_layer 树）保持动态**；地图重开时按版本号重建静态地形缓存。
12. 改 `rstates.rs` 的 bitfield 布局时务必更新 `to_blend()` / `to_depth_stencil()` / `to_cull()` / `to_sampler_desc()` 等解包方法。
13. 纹理数据长度必须 `w*h*4`。
14. 改公共 crate 后，务必 `cargo check --workspace`。
15. **建议**：进行**破坏性更改**、**特性添加**等操作时，请务必更新 [`API_REFERENCE.md`](API_REFERENCE.md) 和 [`ENGINE_GUIDE.md`](ENGINE_GUIDE.md)。
16. **统一入口 `rjw_krusie::prelude`**（happy path 专用清单：应用骨架 / 绘制 / 相机 / 文本 / UI / 图集 / 瓦片）。
    新增类型**默认不进** prelude，确属 happy path 才加入，并同步 [`crates/rjw_krusie/src/lib.rs`](../crates/rjw_krusie/src/lib.rs) 与本手册；
    冲突名（winit `Window` / `Size`，引擎 `LogicalSize…`）一律走命名空间，**不要 `as` 改名**。
    各 example 与最小示例统一写 `use rjw_krusie::prelude::*;`。
17. **`Vec2` 入参一律写成 `impl Into<Vec2>`**（接受 `Vec2` 或 `(x, y)`；两轴同值写 `(x, x)`——glam 无 `From<f32>`）。
    覆盖：`Transform2D::with_pos/with_scale/…`、`Camera2D::new/set_vp/move_by/walk_xy`、`Rect::from_points/contains_point`、
    `SpriteRect` 全族、`MeshSink`/`PolygonSink`/`QuadSink`、`Draw2D::pos/scale`、`Render2D::debug(..)` + `DebugPainter::*`、
    `Text::label*`、`Tile::new(...).uv(..)`、`Ui::debug_*`、`Layout::set_next_min/max`。
18. **「每边量」参数一律写成 `impl Into<Edges>`**（`f32` = 四边同值 / `(x, y)` = 左右、上下 / `Edges` 逐边）：
    `SpriteRect::shrink`（世界）/ `SpriteRect::shrink_uv`（归一化 UV）。
    结构体字段仍是具体类型；需要友好构造时另加 `impl Into<…>` 的构造 / 链式方法。

---

## 17. 快速速查表

| 要做的事 | 代码 |
|---|---|
| 整套库导入（推荐） | `use rjw_krusie::prelude::*;` |
| 低层 / 冲突名 | `rjw_krusie::ui::Window` · `rjw_krusie::gpu::RenderContext` · `rjw_krusie::render2d::Draw2D` |
| 无帧时跳过渲染 | `let Some(mut f) = ctx.frame() else { return };` |
| 键盘 W 按住 | `ctx.key(KeyCode::KeyW).pressed()` |
| 空格"按下那一下" | `ctx.key(KeyCode::KeySpace).down_edge()` |
| 鼠标左键点击 | `ctx.button(MouseButton::Left).down_edge()` |
| 鼠标世界坐标 | `cam.screen_to_world(f.mouse().pos_px())`（先 `cam.set_region(f.region())`） |
| 左右分屏 / 画中画 | `f.set_region(rect); f.submit(&mut cam, Clear::color(c));`（一个画面一次；§4.4.1） |
| 调试：给每个画面描边 | `AppConfig::new(..).viewport_borders(ViewportBorders::On)` |
| 绕中心旋转的精灵 | `r2d.solid(SpriteRect::centered(center, (w, h)))` |
| 让画面跟随玩家 | `cam.transform.pos += (player.pos - cam.transform.pos) * (1-exp(-k*dt))` |
| UI 固定最顶层 | 走 `f.ui(theme)` 开一段（运行时已把 UI 层设为 `SortMode::None` + 默认 base layer 1e7） |
| 退出 | `Esc` → `ctx.exit()`（持帧时 `f.exit()`） |
| 加性混合 Sprite | `.blend(BlendMode::Additive)` |
| 全局启用深度测试 | `render2d.states_mut(RStates::new().depth_test(true).depth_write(true))` |
| Mesh 贴纹理 | `.texture(&tex)` |
| 带 UV 的多边形 | `r2d.polygon_with(\|p\| { p.vertex_uv(a, uv0); p.vertex_uv(b, uv1); p.vertex_uv(c, uv2); })` |
| 设置采样器重复 | `.samp_state(SamplerDesc::linear(AddressMode::Repeat))` |
| 反转混合 Sprite | `.blend(BlendMode::Inverse)` |
| 关闭混合 | `.blend(BlendMode::Disabled)` |
| 注入原生 wgpu 绘制 | `r2d.custom(\|pass\| { ... }).layer(l)`（闭包在引擎已开的 pass 内执行） |
| 延迟丢弃 builder（显式绑定变量） | `let _b = f.draw().sprite(...).blend(...);` —— builder 借用了 `f`，`_b` 必须比后续对 `f` 的使用先结束（Drop 即提交） |
| Atlas 一行绘制 | `if let Some(spr) = atlas.sprite(&handle) { r2d.region(spr).layer(l); }` |
| Atlas 插入精灵 | `atlas.insert(key, Rgba8::new(&rgba, (w, h))).unwrap()` |
| Atlas 常驻精灵 | `atlas.insert_with(key, Rgba8::new(&rgba, (w, h)), InsertOpts::new().permanent())` |
| Atlas 插入白色像素 | `atlas.white()` |
| Atlas 自动复活查找 | `atlas.region_or_revive(&key)` |
| Atlas 编号句柄 | `let h = atlas.handle(&key).unwrap();`（RAII 保活；重排后 `sprite(&h)` 取最新 UV） |
| Atlas 寿命推进 | `atlas.tick()`（引擎已对字形图集每渲染帧驱动；用户自建图集自行调用） |
| Atlas 从 TOML 批量导入 | `atlas.load_toml(toml_str, \|k\| data.get(k).cloned())`（feature `toml`） |
| Atlas 导出 TOML | `atlas.export_toml()`（feature `toml`） |
| 注册静态网格 | `let id = gfx.mesh(MeshSpec::pod("m", &verts, &idx));`（`Gfx` 仅在 `init` 可用） |
| 静态网格实例（Transform2D） | `r2d.static_mesh(id, &tex).tint(c).transform(tf).layer(l)` |
| 静态网格实例（Mat4） | `r2d.static_mesh(id, &tex).matrix(mat).layer(l)` |
| 纯色圆共享单位网格 | 单位圆网格 + `with_pos(pos).with_scale(Vec2::splat(r))` 实例化 |
| 2D 碰撞滑动 | `Aabb::at(rect, tf).slide(vel * dt, &map.solid_rects())` |

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

**推荐：走运行时（`f.ui(theme)` 开一段，运行时替你管 `UiState` / `Theme` / DPI / UI 层 Render2D）**

```rust
fn update(&mut self, ctx: &mut Ctx) {
    let Some(mut f) = ctx.frame() else { return };
    f.draw().sprite(...);                       // 世界层
    // ── UI 段 1（「ui anywhere」：一帧可开任意多段、位置随意）──
    let mut ui = f.ui(Theme::dark());
    ui.label_at(vec2(16.0, 12.0), "FPS: 60");       // place：绝对定位 + 自然尺寸
    ui.pack_at(vec2(16.0, 90.0), PackSide::Top, |p| {  // pack：垂直堆叠
        if p.button("btn_start", "开始游戏").clicked() { /* ... */ }
        self.volume = p.slider("vol", 0.0..=1.0, self.volume);
        if p.checkbox("fs", "全屏", self.fs).toggled() { self.fs = !self.fs; }
        if p.radio("diff_hard", "diff", "困难").checked() { /* 单选组互斥 */ }
        p.text_input("name", &mut self.player_name);   // 点击聚焦、打字（IME）、Enter/Esc 失焦
    });
    ui.window("win_a").pos(vec2(560.0, 240.0)).show(|w| {   // 可重叠窗口：点击置顶 + 可拖拽
        w.label("窗口 A");
        w.button("win_a_btn", "A 按钮");
    });
    ui.drag_panel_at("inv_panel", vec2(300.0, 90.0), |pp| {  // 可拖拽面板（位置持久）
        pp.label("背包");
        pp.grid_at(vec2(0.0, 28.0), 3, "inv", |g| {          // 3 列网格（cell 跨帧缓存）
            g.button("slot_0", "物品 0");
        });
    });
    ui.finish();                                 // 段收尾（也可省略：作用域结束即收尾）

    // ── 段与段之间可以做任何事（世界层 / 世界文本 / 逻辑）──
    f.text(|t| { t.label("世界里的文字").size(16.0).at((0.0, 0.0)).draw(10.0); });

    let mut hud = f.ui(Theme::dark());           // ── UI 段 2 ──
    hud.label_at(vec2(16.0, 690.0), "FPS HUD");
    hud.finish();

    f.submit(&mut self.cam, Clear::color(Color::rgb(0.05, 0.05, 0.08)));
```
}
```

**低层/逃逸舱口**：自己持有 `UiState` 并手动开帧（需要自定义 `base_layer` / `scale_factor` / 复用别的 `Text` 时）：

```rust
// ⚠ 帧级账由调用方负责：每帧**一次** `ui_state.begin_frame()`（段起始的 `build()`
// 会在未开场时兜底开场），录制收尾调 `ui.end_frame(r2d)` 做帧收尾（焦点/光标/统计）。
ui_state.begin_frame();
let mut ui = Ui::begin(window, &mut text, &mut ui_state)
    .capture(ctx.mouse(), ctx.keys())          // 输入快照（&MouseInput, &KeyboardInput）
    .theme(Theme::dark())
    .base_layer(1.0e7)                          // 默认 1e7
    .scale_factor(ctx.scale_factor().unwrap_or(1.0))  // 控件坐标/字号按逻辑像素
    .build();
// ... 录制 ...
ui.end_frame(r2d);   // 帧收尾 + 免全量排序提交（视口/渲染器此时才需要；UI 无需相机）
```

> **段 vs 帧**：`Ui::finish(r2d)` = **段收尾**（分桶 → 顶点 → 提交，一帧可多次）；
> `Ui::end_frame(r2d)` = **帧收尾**（输入结算 / 焦点导航 + 描边 / 光标定夺 / 统计写回 /
> 帧级暂存复位，**每帧一次**）。运行时路径（`f.ui(theme)`）自动处理这两件事。

### 18.3 ⚠️ 易混淆点

- **坐标一律屏幕逻辑像素**（左上角原点、Y+ 向下）：调用 `.scale_factor(ctx.scale_factor().unwrap_or(1.0))` 后，所有控件坐标 / 字号按逻辑像素使用，内部自动换算物理像素绘制与命中；不设置则 scale = 1.0（与物理像素一致）。与引擎世界坐标（中心原点）不同；内部经相机屏幕固定变换绘制，旋转/缩放相机下依然 1:1。
- **UI 段存续期间 `f` 被借用**：`f.ui(theme)` 返回的 `UiSession` 活着时不能 `f.draw()` / `f.text()` / `f.submit()`（编译期拦住）；段之间可以随便交错（世界层、世界文本、逻辑都行）。需要重置 `UiState` 等操作时用局部标记，`ui.finish()` 之后统一处理（见示例）。
- **一帧多段：段序 = 绘制序**：每段各自 `finish` 提交到同一个 UI 层 `Render2D`，所以**后一段整体压在前一段之上**（同一窗口内也如此）。因此**一个窗口（或一处顶层放置）尽量在同一段内录完**——跨段会让该窗口的顶点缓存每帧重算两次。
- **顶层 pack 直接可用**：`build()` 内建**根容器**（PackSide::Top，可用宽 = 视口物理宽）——`finish` 前任何位置调 `label` / `button` / `slider` 等 pack 控件，顶层即自顶向下流式堆叠；绝对定位仍用 `*_at(pos, ...)`（含 `label_at` / `panel_at` / `pack_at` / `grid_at`）。容器内用无位置形式（`p.button(...)` 占光标）。容器内嵌套容器用 `*_at(offset)`（相对当前容器内容原点），**不占光标**——v1 不支持容器内"光标嵌套"。
- **交互控件必须有稳定 ID 字符串**（按钮 / 滑块 / 勾选 / 单选 / 输入框）；ID 变化 = 状态丢失。`UiState::reset()` 清空全部状态。
- **ID 命名空间**（[`IdRelative`] / [`IdAbsolute`] / [`Ui::id_for`]）：窗口 / 滚动容器 / grid / 可拖拽面板 / 下拉框是**命名空间边界**——进入自动压栈、退出自动弹栈（`with_id` 闭包作用域保证配对），其内控件的**绝对 ID** 自动带容器前缀（如 `"chishi/btn"`）。控件公开 API 仍传**相对名字**（`&str`），内部自动解析；状态键 / 焦点 / 单选组值 / 窗口 id 一律用**绝对 ID**。`ui.id_for(id_relative)` 从名字生成绝对 ID（顶层零拷贝）。类型层面杜绝双重前缀与相对/绝对混用（详见 `crate::id`）。
- **闭包内不可借用已被 `ui` 借用的字段**（如 `self.ui_state`）；需要重置等操作时用局部标记，段收尾（`ui.finish()`）后统一处理——见上文"UI 段存续期间 `f` 被借用"。
- **单选**的选中状态完全存于 `UiState.radio_groups`（`group → 控件绝对 ID`），应用只读 `checked()`；初始选中用 `state.radio_groups.insert("组名", IdAbsolute::from("id"))`（顶层无前缀 = 原样；窗口内单选值自动带窗口前缀，与应用无关）。
- **文本输入**：普通字符走 `ctx.keys().chars()`（含 Shift 组合，控制字符已过滤）；**中文输入法（IME）已支持**——`rjw_main` 建窗时自动 `set_ime_allowed(true)`，`rjw_keyboard` 收集上屏文本（`ime_commits()`）与组合候选（`ime_preedit()`，输入框以灰色绘制在光标后），Enter 确认上屏。
- **可拖拽面板 / 窗口**：`drag_panel_at(id, pos, |p| ...)` 按住面板拖动；`ui.window(id).pos(..).show(|w| ...)` 是**可重叠窗口**——点击即**置顶**（焦点 z-order，`UiState.window_z`），位置持久于 `UiState.panel_pos`；拖动期间**抑制内部子控件交互**。拖拽位置按**物理像素粒度**跟随（1px 跟手，不受 DPI 逻辑量化影响）。
- **窗口重叠点击裁决**：重叠区域点击**只让最上层窗口**获得拖拽与置顶（`Ui::finish` 内部 `resolve_win_press`）——不会同时拖动两个窗口。
- **QuadVertices 渲染（不使用 Sprite）**：全部图元（背景 / 控件背景 / 文字）转为**四边形顶点**（`Render2D::quads`，每四顶点一组 **TL,TR,BL,BR**，固定索引 `[0,1,3, 3,2,0]`），按 **(窗口, 元素序, 图形/文字组, 纹理)** 分组提交。**UI 自行管理绘制顺序**——**UI 的 Render2D 必须 `set_sort_mode(SortMode::None)`**（完全按提交顺序绘制）：`finish` 按 `(win 升序, 元素序（控件录制序）, 元素内 图形→文字, 纹理 uid)` 提交（**免全量排序**：win + depth 分桶、桶内保持录制序，语义与 `sort_by_key((win, depth, elem, group, seq))` 完全等价）——窗口间层级由提交顺序保证（`layer = base + z*1.0` 仅作兜底），**窗口内"每个控件 背景→文字 依次绘制"（后录控件完整覆盖先录控件）**，不随纹理 uid / HashMap 顺序抖动。⚠ 不可按 `(win, 图形组, 文字组)` 提交：那会把所有背景排到所有文字之前，后录控件的背景会被先录控件的文字盖住（重叠层级错误）。⚠ **不要用 `set_sort_mode(SortMode::LayerAndStates)`（`SortMode::LayerAndStates`）**：它会在同一 layer 内按 `(rstates, texture_uid)` 重排，字形图集页先于程序化纹理页（圆角/渐变）注册 → 重排后圆角/渐变图形排在文字之后绘制、**盖住文字**（曾因此踩坑：圆角按钮文字消失、渐变状态栏盖住标签）。`set_sort_mode(SortMode::LayerOnly)`（`LayerOnly`，稳定排序）同层保持提交顺序，可接受。
- **白纹理合批（单窗口一次 DrawCall）**：WHITE 基础纹理（1×1，`clamp_margin`）预置进**字形图集页**（[`rjw_text::Text::white_region`]；`DynamicAtlas::white` 为与 key 无关的内置槽位，参与 compact 重排）——实心填充（Solid / 边框 / 光标 / 调试叠加）与字形**同页同纹理**：按控件级顺序提交（背景+文字相邻）且同纹理，Render2D 合批为**单个 draw call**，省去图形↔文字的纹理状态切换。字形本体保持 `InsertOpts::no_clamp()`（避免 clamp margin 挤压）。
- **窗口 transform**：四边形顶点为**相对窗口原点的局部像素**，提交时经 `screen_fixed_tf(窗口原点)` 变换到世界——**移动窗口只改变换、顶点不变**（也支持将窗口嵌入游戏场景，给任意世界变换）。`quads` / `mesh(..).transform(..)` 均支持 `Transform2D`（`IDENTITY` = 顶点即世界坐标）。
- **窗口顶点缓存**：窗口内容不变时，四边形顶点**跨帧缓存**（`UiState.window_quads` 按**内容签名**命中）——静态窗口每帧零字形收集/重建；hover 变色、文字编辑等任何内容变化都会使签名变化而自动重建。**移动窗口不影响缓存**（顶点是局部的，transform 每帧用当前原点）。
- **win=0（非窗口）放置子槽缓存**：顶层 `pack_at` / `flex_at` / `scroll_at` / `add_at` 等各自一个**子槽**（`UiState.z0_quads`），逐槽全量签名 —— 一处变化只重建那一槽。
  ⚠ **两个坑（都踩过，实测把 UI 帧时间翻倍）**：
  1. **键必须带段号**：组号是"**段内**第几个顶层放置"（`Ui::z0_ranges` 随 `Ui` 视图每段重建），各段都从 0 起且兜底组恒为 0 ⇒ 只用组号会让不同段的同号槽**互相覆盖**，每帧交替 miss；
  2. **陈旧清理必须按帧、且只做一次**：清理语句若写在**段收尾**，一次调用只见到本段的槽，会把同帧其它段刚写好的缓存删掉。
  正确做法：键 = `(段号, 组号)`，清理在**下一帧开场**按"上一帧的完整槽集合"（`UiFrameState::z0_seen`）做一次。验证：`collect` 602µs → 2µs、`cache_miss` 7 → 0（`--frames 240` + `RJ_CACHE_TRACE=1`）。
- **帧时间三分解**：`UiStats` 的 `ui_frame_us ≈ prologue_us`（各段开场：懒开场 / 冻结输入 / 装载帧级事实 / 建根容器）`+ 应用录制（含主题构造）+ finish_us`（分桶 → 顶点 → 提交）。排查"UI 变慢了"先看这三项，能立刻分清**引擎 bug** 还是**应用内容增长**（示例 `[perf]` 行直接打印）。
- **深度测试**：`RStates::default()` 深度测试**默认关闭**，QuadVertices 纯 2D 覆盖无需深度；世界层需要深度时用 `render2d.states_mut(RStates::new().depth_test(true))`（UI 独立 Render2D 不受影响）。
- **独立 UI 渲染（推荐）**：UI 录制到**单独 Render2D**（`Render2D::set_sort_mode(SortMode::None)` 关闭排序，UI 自行管理绘制顺序），与世界合并提交：`r2d.encode(clear, &view, None)`（世界）→ `r2d_ui.encode(...)`（UI，color: None 不覆盖）→ `queue.submit([cb_world, cb_ui])` → `queue.present(st)`（一次 present）。
- **输入屏蔽**：文本输入框聚焦时应用快捷键不应触发——检查 `ui.state().text_focus()`（`Some` 表示**文本控件**持焦点；如示例中 `R` 重置 / `Esc` 退出前）；输入框内 `Esc` 取消焦点（不再传给应用层）。
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
  （点击穿透拦截计数——`> 0` 说明鼠标下有叠放、背后控件被正确抑制）；
- `UiState::widget_occluded_hits()`：上帧**命中但被同一窗口内更上层控件遮挡而未响应**的
  次数（控件级遮挡拦截计数——`> 0` 说明鼠标下有**控件重叠**，下层控件被正确抑制）。

**容器尺寸必须包住子控件（否则交互必然出问题）**：`grid_at` / `pack_at` / `add_at` 这类
**绝对放置**不进容器的自然尺寸（它们不占光标），若容器尺寸只按"流内子项"结算，内容就会
**长到容器外**——画得出来，却不在窗口矩形 / 窗口 z-order 判定里：拖不动、被别的窗口
"穿透"、诊断面板也指错窗口（示例"背包按钮超出窗口"就是这么来的）。两道防线：

1. **尺寸**：`Frame::content_bounds`（[`Frame::note_content`]）——每处子项矩形（`Child::Expand`）、
   每个绝对容器整体（`container` / `flex_at`）、每个 `add_at` 控件都记进当前容器帧，
   `settle_size` 取"自然尺寸 ∪ 内容包围盒的右下角"。**固定轴例外**（`fixed_w` / `fixed_h`）：
   那一轴由调用方定死，内容按它排布（要裁就配 `Placement::Clip` / ScrollView）。
2. **交互**：窗口遮挡矩形 = 窗口盒子 **∪ 本帧子控件的命中区**（`Ui::win_hit_bounds`）。
   即便内容真的溢出（固定尺寸容器 / `Child::Fit` / 有意为之的装饰），"看得见就能点、
   被更上层压住就不响应"依然成立。

> 为什么不是"每个控件一个子窗口"？那要给每个控件独立的 z 记录、id 命名空间、裁剪层与
> 缓存生命周期——而引擎已经有**控件命中区注册表**（`hit::HitRegion` + `Ui::hit_abs`）：
> 把窗口的遮挡矩形按它并集，一次跨帧表的成本就得到同样的保证。控件级遮挡（同一窗口内
> 重叠控件只有最上层响应）与窗口级 z-order 正交，两者叠加即可。

注意：`occluded_hits` / `last_press_window` 是跨帧持久数据，须在 `Ui::begin` **之前**读取
（begin 会借用 `ui_state`），上一帧 finish 写入、本帧显示（见 `examples/eg260818UI`
右上角诊断面板——把窗口 A/B/背包叠在同一处点击即可看到解析结果）。

**点击穿透（窗口遮挡）**：重叠区域**只有鼠标下最上层窗口**的控件响应——`hit_abs`
（所有控件共用）与窗口/面板自身的拖拽命中都过 `window_occluded(z, mouse, window_rects)`
闸门（`z=0` 的非窗口内容被任意窗口遮挡）。要素（**踩过坑，别改回去**）：

- **表按窗口绝对 ID 键**（`UiState::window_rects`），查询时把 ID 解成**当前 z**
  （`Ui::window_rects_iter`）：z 会在帧末被"点击置顶"抬到 `max+1`，而矩形是那一刻录下的；
  按 z 存 + 按旧 z 比较，会让**刚被抬高的窗口"消失一帧"**——它画在上面却挡不住本在它下面
  （但 z 大于它旧 z）的窗口的控件；
- **表跨帧存活**：矩形 = 窗口盒子 ∪ 本帧子控件命中区（`Ui::win_hit_bounds`），录制时按 ID
  写入；陈旧清理在**帧末**、数据源是 `UiFrameState::window_ids_seen`（帧级清单）。
  ⚠ **不要用本视图的 `win_ids` / `win_origins` 去清**：它们在同帧更早的 `finish()` 末尾
  已被 `save_frame_state()` swap 走（空表）⇒ 会把整张遮挡表**每帧清光**，遮挡判定退化成
  "只看本帧已录制的窗口"，于是**本帧录在后面的窗口挡不住前面窗口的控件**
  （"被遮挡的控件仍被触发"，用 `--sim-cover` 可复现，见 `docs/DEBUGGING.md` §8.2）；
- **帧末复核按下归属**（`Ui::resolve_widget_press`）：命中那一刻本帧几何可能还没录完
  （盖住我的窗口本帧才移过来），而**帧末**所有窗口都录完了——于是帧末再看一次"我是不是
  被更高 z 的窗口盖住"，是则撤销这次认领（清 `pressed` / `clicked` / `dragging`），
  诊断计数 `UiState::press_cancelled_by_window()`。
  ⚠ **比较基准必须是控件所在窗口的"当前 z"，不能是认领时的旧 z**（故 `press_widget` 记的是
  **窗口 ID**、复核时现解 z）：`resolve_win_press` 在帧末会把**被点的窗口**抬到 `max+1`，
  拿旧 z 比就是"窗口比自己新 z 小、而自己的矩形当然覆盖鼠标" ⇒ **窗口把自己判成被别人盖住**，
  于是**每一次**窗口内控件的按下都被撤销——滑块 / 滚动条 / 文本选择全拖不动（真实回归；
  `--sim-cover` 的**段 0 正对照**守着它：按住约 14 帧，误撤时只剩 1 帧）。
  它保证的是**状态不延续**（不会一直拖到释放）；同帧内按下 + 抬起（极快点击）不受保护——
  所以控件作者**别在 `down_edge` 上直接执行一次性动作**，用 `hit::update_interact` +
  `clicked()`（释放帧才成立，天然安全）。
- 已知边界：窗口**首次出现的那一帧**矩形尚不可知（跨帧缓存盲区），下一帧起严格生效——
  置顶方向从第一帧就正确（`win_press_top` 只保留最上层按下窗口）。

**点击穿透（控件遮挡）**：**同一窗口 / 面板内**的重叠控件同理——`ui.hit_abs(控件绝对 id, rect)`
登记的命中区域按**录制次序**（后录制 = 画在上面）比较，鼠标下若有更上层的**别的控件**覆盖，
本控件不响应（点击 / 悬停 / 拖拽全都不响应），诊断计数 `UiState::widget_occluded_hits`。
这条规则修的是"重叠控件被**一起触发**"（示例 `--sim-overlap` 可脚本化复现：去掉本机制
`below=1 above=1`，加上则 `below=0 above=1`）。要素：

- **身份 = 控件自己的绝对 id**（`IdAbsolute` 的哈希）：同一控件的多个区域（滑块轨道 + 手柄、
  输入框 + 手柄）**互不遮挡**——否则轨道会被自己的手柄挡掉；
- **层级 = 登记时的录制位置**（`Ui::seq`，同帧单调递增）：与本帧绘制先后一致；
- **跨帧判定**：判定输入是**上一帧**登记的整表（`UiState::hit_regions` →
  `begin_frame` 翻页成 `prev_hit_regions`）。同一帧里"后录制"的控件此刻还没录制，
  无法参与比较；与窗口级 `window_rects` 同一思路，首帧盲区同样存在；
- **本体 vs 控件**：窗口 / 面板 / 浮层的整块区域用 `ui.hit_body_abs(rect)`——本体**包含**
  子控件，若也走控件级遮挡，鼠标停在子控件上时本体就会被判成"被挡住"（浮层误判
  "点在面板外"而收起）。本体层级由窗口遮挡负责，两者正交。
- **裁剪层**：登记项带 `clip`，鼠标在裁剪层（`Scroll` 可视区 / Clip 沙箱）之外时不算覆盖。

### 18.6 渲染增强（圆角 / 渐变）

**样式如何设置**：`Theme` 子样式的 `radius` 字段（面板 / 窗口 / 按钮 / 输入框，逻辑像素，
默认 0 = 直角）；绘制原语 `ui.rounded_rect_at(pos, size, radius, color)` 与
`ui.gradient_rect_at(pos, size, axis, stops)`（绝对定位，`elem = 0` 装饰层）。
子样式与主题都是**责任链**（`with_*` setter 返回 `Self`，只改链上字段）：

```rust
// 全局主题：with_radius 级联 panel/button/input（圆角生成器已修复，高 DPI 无缺口）
let theme = Theme::dark().with_radius(8.0);
// 逐容器样式覆盖（容器责任链）：只改这个窗口的圆角/背景，其余回落全局主题
ui.window("win")
    .pos(Vec2::new(560.0, 240.0))
    .width(220.0)
    .style(PanelStyle::default().with_radius(8.0).with_bg(Color::rgba_u8(40, 44, 62, 255)))
    .show(|w| { w.label("窗口"); });
// 等价旧写法（仍可用）：let mut theme = Theme::dark(); theme.panel.radius = 8.0; …

ui.gradient_rect_at(Vec2::new(0.0, 0.0), Vec2::new(1280.0, 56.0),
    GradientAxis::Horizontal,
    vec![(0.0, Color::rgba_u8(38, 52, 90, 255)), (1.0, Color::rgba_u8(26, 34, 60, 255))]);
```

**v0.4：圆角不再走纹理。** `rjw_ui::proc`（`ProcTextures` / `rounded_rect_rgba` /
`rounded_9patch` / `ROUNDED_TEX_SIZE`）**已删除**。圆角矩形由 CPU **镶嵌成三角形**
（`rjw_ui::tess`）：

- **不改着色器**：抗锯齿不靠 SDF，靠光栅化器对顶点 alpha 的线性插值——硬体轮廓
  `alpha = 1`，同心的外圈轮廓 `alpha = 0`，两者配成带状三角形插出羽化边缘。
  `sprite.wgsl` / `InstanceData` 零改动，也不需要第二条管线。
- **梯度以几何边缘为中心**：硬体**内缩** `f/2`、外环**外扩** `f/2`（`f` = 羽化宽，
  `Theme::feather` 逻辑像素 × DPI，默认 1.0）⇒ 渲染出的视觉尺寸恒等于给定矩形。
  若只向外扩，每个矩形都会胖 `f` 像素，相邻矩形重叠处会多混一次 alpha 而露缝。
- **一圈轮廓 + 整圈带状化**（直边的关键）：一圈轮廓 = 4 角 × `(segs + 1)` 个点，
  **角的末点与下一角的首点在全局点号上相邻**，两者之间就是那条直边 ⇒
  「两圈轮廓之间沿整圈推进成带」会自动覆盖四条直边。早期实现按角分别成带，
  直边完全没有几何——表现就是"边框只剩 4 个圆角孤岛、四条边整个消失"、
  以及"羽化只在角上有效"。也正因如此，参与带状化的两圈必须用**同一套**
  `stride`/`segs`（逐点对应），顶点数由半径统一决定（~74 顶点/圆角矩形）。
- **索引内联、不缓存、不共用**：索引与顶点同源（同一次抽样循环），分开算一旦不一致
  会产生错乱三角形且**不会 panic**。也**不用静态网格**：静态网格把几何冻结在 GPU
  缓冲，与立即模式每帧重录冲突；羽化带是亚像素级的软边、不可被实例缩放。
- **圆角边框 = 环带**（`tess::push_rounded_ring`）：外轮廓与内轮廓之间的一圈带子
  （**含四条直边**），内半径按 `max(0, r_outer - width)`（与 CSS `border-radius`
  同规则）。边框的**内外两条边界都做羽化**（最多四圈同心轮廓：外羽化 / 外轮廓 /
  内轮廓 / 内羽化）。比"外圈实心 + 内圈实心"少一次边缘混合。
  边框宽 ≥ 半边尺寸时内轮廓会塌缩 ⇒ 直接退化成一块实心圆角矩形（语义一致，
  且避免尺寸为 0 的矩形让角心次序颠倒、带状绕序整体翻反）。
- 半径**不取整**（镶嵌器接受任意小数半径），高 DPI 下不再有 `radius × scale` 的
  取整误差。半径小到放不下两个同心轮廓时（`radius - f/2 < 0.5`）自动退回**不羽化**。
- **`Theme::feather`**（`with_feather(px)`）是唯一的质量旋钮：0 = 硬边，
  1.0 = 标准 1px 抗锯齿，调大 = 软边。`eg260818UI` 的「主题调节」窗口可实时拖。
- GPU 侧的唯一新入口是 `Render2D::mesh_indexed(vertices, indices, texture)`
  （`ColorMode::Instance`：`tint` 是整段实例色，顶点自带色不被改写）；
  桥接后端仅在窗口 FX tint 非白时才调 `.tint()`，其余批次保留可跨段合批的普通 `Mesh`。

**仍进动态 Atlas 的**：WHITE（`1×1`，兼作渐变/圆角的 UV 源）；线性渐变**不再需要纹理**
（四角顶点色由光栅化器双线性插值）。要点：

- **提交分组是 `(win, 图形/文字组, 纹理 uid)`**（`GROUP_GRAPHIC=0` / `GROUP_TEXT=1`）：
  圆角 / 渐变 / 白纹理属于图形组，恒先于字形文字组——非白纹理 uid 不会因与白纹理
  比较而排序错位（背景不会盖住文字）。窗口几何缓存（`UiState.window_quads`）同步
  携带分组。
- `rjw_atlas` 的 Guillotine 打包 / `clamp_margin` / 页纹理注册机制不变；
  `rjw_text::Text::user_texture` 仍是对外可用的自定义纹理入口（UI 自己不再用它做圆角）。

### 18.7 主题扩展（密度 / 行距 / 阴影）

`Theme` 除了"颜色 + 圆角 + 羽化 + 边框宽"，还有两组**布局 / 深度**令牌。都是纯令牌，
**不改着色器、不加绘制通道**。

**布局密度**（"同一套界面在小屏排得下、在大屏更舒展"）：

```rust
use rjw_ui::{Density, Theme};

let compact = Theme::dark().density(Density::Compact);   // 小屏 / 工具面板 / 高信息密度
let loose   = Theme::dark().density(Density::Spacious);  // 触屏 / 演示 / 大屏
// 单维微调（都是"在现值上叠乘"的倍率，可任意组合）：
let t = Theme::dark()
    .with_font_scale(1.10)      // 只放大字号
    .with_spacing_scale(0.90)   // 只收紧间距
    .with_line_spacing(1.45);   // 只放松多行行距
```

| 档位 | 间距 × | 字号 × | 行距 × |
|---|---|---|---|
| `Density::Compact` | 0.84 | 0.92 | 1.10 |
| `Density::Cozy`（默认） | 1.0 | 1.0 | 1.2 = `DEFAULT_LINE_SPACING` |
| `Density::Spacious` | 1.18 | 1.08 | 1.30 |

- **间距类** = `gap` / `row_h` / `panel.padding` / `button.padding` / `slider.{track_h,
  handle_w,height,min_w}` / `input.{padding_x,height,min_w}` / `checkbox.{box_size,gap}` /
  `divider.margin` / `combo.{menu_pad_v,item_pad_x,item_min_w}`；
- **字号类** = `label` / `button` / `checkbox` / `input` / `combo` 的 `font_size`；
- **密度不碰**：圆角（造型选择）、边框宽 / 羽化（亚像素观感）、颜色、阴影；
- `Density` 是枚举而不是裸 bool（约定 R2），`Density::default()` = `Cozy` ⇒
  **默认观感与扩展前逐像素一致**（既有 sim 坐标 / 截图基线不受影响）。

**行距**（`Theme::line_spacing`，行高 = 字号 × 该值）：作用于**可能换行的文本**——
多行 `TextArea`、`wrap(..)` 标签、换行预览。`wrap <= 0` 的**单行**文本行高恒等于字号
（盒子高度不变，文本框内字形垂直居中位置才不会漂）。下界 0.5（行盒重叠会让光标定位
失去意义）。`Theme::scaled(DPI)` **不缩放**行距——它是倍率不是尺寸。

> ⚠ **缓存正确性**（改这里必须两边都改）：行距是**运行时可变**的主题令牌，而
> `DrawKind::Text` 的 `buf`（排版结果）按设计**不进命令哈希**。所以：
> ① 排版缓冲缓存键含行距位（`UiState.text_buffers`，`(mult_bits, TEXT_LINE_HEIGHT_VERSION)`）；
> ② 窗口 / `win=0` 子槽的**几何签名**以 `theme.line_spacing.to_bits()` 为前缀
>   （`Ui::hash_cmds`）。漏掉 ②，改了行距后窗口会继续命中旧顶点缓存（内容含换行文本时，
>  窗口 / 标签高度就停在旧值不更新）。

**字重**（`Theme::font_weight`，默认 `Weight::NORMAL` = 400）：

```rust
use rjw_ui::{Theme, Weight};

let t = Theme::dark().with_font_weight(Weight::BOLD);   // 700
let t = Theme::dark().with_font_weight(Weight(550));    // 任意数值（可变字体 / 精细档）
```

- **一个令牌管全 UI**：所有排版都经 `Ui::cache_buffer_wrap` / `Ui::ensure_text_buf`
  这两个出口建缓冲 ⇒ 在那里统一 `.weight(self.theme.font_weight)`，**不新增参数**、
  不改任何控件签名（标签 / 按钮 / 输入框 / 下拉 / 换行文本自动一起变）。
- **它是排版输入，不是"画粗一点"**：字重改**字形**也改**步进宽度** ⇒ 文本自然宽 /
  换行位置 / 控件尺寸都会变。字体没有该字面时 cosmic-text 按最接近的字面回落。
- **不是尺寸量**：`Theme::scaled(DPI)` 与 `Density` 都不碰它（同 `line_spacing` 一类）。
- **缓存三处**（漏一处就"改了字重画面不动"）：① `UiState::text_buffers` 键
  （`(…, (mult_bits, weight.0, TEXT_LINE_HEIGHT_VERSION))`）；② 输入框自持缓冲
  `WidgetState::text_buf` 的键字符串；③ 窗口 / `win=0` 子槽几何签名前缀
  （`Ui::hash_cmds` 里 `write_u16(font_weight.0)`）——③ 与行距同理：固定矩形里的居中
  文本（按钮 / 输入框）矩形不变，而字形变了，只靠命令哈希永远不失效。
- 验证：`--sim-weight`（第 30 帧 400 → 700，前后量同一串文本的实测宽：`112 → 116`
  ⇒ 字重真的进了排版输入；`[FAIL] 字重没进排版输入` 就是漏了上面某一处——A/B 实测：
  去掉 `.weight(..)` 后输出立刻变成 `112.0 → 112.0 [FAIL]`）；
  单测 `style::tests::font_weight_is_a_font_choice_not_a_size_token` +
  `widgets::fontmodal::tests::weight_choices_are_the_seven_ordered_steps_and_labeled_uniquely`。
- 应用侧入口：`builtin::FontModal`（字体弹窗）现在同时管字体族 + 字重，见
  `docs/API_REFERENCE.md`「字重令牌」。

**阴影**（`PanelStyle::shadow` / `ShadowStyle { blur, offset, color }`，色令牌
`Palette::shadow`）：

```rust
use rjw_ui::{Palette, ShadowStyle, Theme};
use glam::Vec2;

let theme = Theme::themed(&Palette::dark());          // 预设已带投影（深色 alpha 120 / 浅色 48）
let flat  = Theme::dark().without_shadow();           // 平面：blur = 0 = 不画（约定 R3，不用 Option）
let lifted = Theme::dark().with_shadow(ShadowStyle {  // 主题级：整对象替换
    blur: 24.0,
    offset: Vec2::new(0.0, 8.0),
    ..ShadowStyle::default()
});
// 逐容器（只让某一个窗口不一样）：
let one = PanelStyle::default().with_shadow_color(Color::rgba_u8(0, 0, 0, 90));
```

- 画在**本体之下、更低 z 的窗口之上**——"投影落在下面的窗口上"；
- `tess::push_rounded_shadow` 用 `SHADOW_STEPS = 4` 圈**同心**轮廓：第 `t` 圈是
  `rect.grow(blur·t)` 再**整体平移 `offset·t`**，alpha 按 `a·(1−t)²` 衰减（无平台段）；
- **offset 必须逐环分配**，不能只平移整体轮廓：只平移内轮廓会在窗口正下方留下
  **等 alpha 的实心条带**（"阴影下方突出"——已修）。回归测试断言
  "边缘下方 1px 的 alpha < 面板 alpha" 且随距离**单调下降**；
- 半径 0 的直角矩形同样支持；`blur <= 0` / 颜色透明 / 退化矩形 ⇒ **零几何**；
- 整块阴影完全在裁剪层之外时整个跳过（不进顶点）；
- **颜色是任意色**（不只是黑）：顶点 RGB 与调用方给的颜色**逐位相同**，只有 alpha 按
  `a·(1−t)²` 衰减（单测 `tess::tests::shadow_keeps_the_callers_rgb_and_alpha`）。
  所以"投影颜色"既能调**深浅**（alpha：120 → 40 = 轻轻浮起）也能调**色相**
  （如带一点蓝的投影）。⚠ `blur > 0` 而 alpha 调到 0 仍会镶嵌几何（只是看不见）——
  要真正省下来请把 `blur` 归零（`without_shadow()`）。

`eg260818UI` 的「主题调节（实时）」窗口把上表全部做成了按钮 + 滑杆（密度三档一键
铺开、字号 / 间距 / 行距 / 圆角 / 羽化 / 边框宽 / 三组颜色实时可拖）。

### 18.8 滚动容器（scroll_at）

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

### 18.9 键盘导航（焦点遍历）

交互控件（按钮 / 勾选 / 单选 / 滑块 / 输入框 / 下拉框）录制时调用 `Ui::register_focus`
登记进**本帧焦点链**（`focus.rs` 的 `FocusEntry`：**绝对 ID** / 窗口 z / 类型 / **绝对逻辑矩形** /
裁剪）。`finish` 末尾 `handle_focus_keys`：

1. 链按 `(win, 注册序)` 稳定排序（非窗口 0 在前，窗口按 z 从下到上）；
2. **Tab / Shift+Tab / ↑ / ↓** → `focus_step`（纯函数，可单测）取下一个焦点，
   更新 `UiState.focused`（跨帧持久）；焦点控件本帧未录制 → 自动清除焦点；
3. **Esc** → 优先收起展开的下拉框（`UiState.combo_open`），否则取消焦点；
4. **焦点描边**：对焦点控件画一圈 `DrawKind::Border`（`Theme::focus` 样式），
   `elem = self.seq + 1`（全局最大 → 画在窗口内容之上），`clip` 沿用控件自身。

**激活与调值**（控件录制时即时处理，无跨帧延迟）：

- **Enter / Space**：`Ui::key_click(&id_for, kind)`（`id_for = ui.id_for(id)` 绝对 ID；焦点匹配 + 非 IME 组合中）→ 按钮 /
  勾选 / 单选 / 下拉框合成一次 clicked（`key_click` 排除 TextInput / Slider）；
- **← / →**：焦点为滑块时步进调值（`span × 5%`）；焦点为输入框时移动光标（原有）；
- **↑ / ↓**：下拉框展开且焦点在按钮上时循环切换选项（选中即收起）；
- 文本输入框内 Tab 照常遍历焦点（`chars()` 已过滤 `\t` 控制字符，不会插入制表符）；
  Enter / Esc 失焦行为保持不变（应用快捷键需检查 `ui.state().text_focus()`）。

> ⚠️ 焦点是**跨帧持久状态**（`UiState.focused`），但焦点链**每帧重建**（immediate-mode：
> 控件动态增删自动反映）；焦点描边改变窗口内容签名 → 窗口顶点缓存自动失效重建。

### 18.10 布局增强（换行 / min-max / flex）

**自动换行标签**（宽度内按词/字换行，多行垂直居中）：

```rust
ui.label_wrap_at(Vec2::new(16.0, 600.0), 240.0, "宽度 240 内自动换行的说明文字……");
p.label_wrap(180.0, "pack 内 180 宽换行");   // 容器内占光标
```

- 底层：`rjw_text::create_buffer_wrap(text, attrs, size, lh, align, wrap_width, policy)`
  （`wrap_width` 物理像素，参与排版缓存键）——cosmic-text 按词/字换行，`buffer.set_size(Some(w), None)`
  高度留空（设单行高会只保留首行）；
- 测量 `Text::measure_buffer` 返回（宽 = min(自然宽, wrap)，高 = 行数 × 行高，整数）；
  UI 侧 `UiState.text_buffers` 缓存键增加**换行宽度**（不同宽度各自缓存）。

**min/max 尺寸约束**（作用于**紧接着的下一个子项**，一次性）：

```rust
p.min_size(160.0, 0.0);   // 下一子项最小宽 160（0 = 该轴不约束）
p.max_size(120.0, 0.0);   // 下一子项最大宽 120
p.button("btn", "按钮");   // 实际宽度被 clamp 到 [160, 120] 交集 → 120..160 语义按序应用
```

- `Frame` 内部 `next_min` / `next_max` 字段（`set_next_min` / `set_next_max`，多次调用
  取各轴 max / 非零较小值），`child_rect` 分配时 clamp，**用后自动清零**；
- 约束只影响该子项的**布局尺寸**（矩形），内容（如按钮文字）在矩形内照常居中。

**flex 权重容器**（固定总高按权重等分，同帧精确分配）：

```rust
ui.flex_at(Vec2::new(880.0, 450.0), 150.0, &[1, 2, 1], |f, i| {
    f.button(&format!("row_{i}"), &format!("行 {i}"));  // 高度 = 150 按权重分配（扣 gap）
});
```

- 分配：`可用高 = total_h - gap×(n-1)`；子项高 = `可用高 × wᵢ / Σw`；`Frame::force_next_h`
  强制高度（一次性），`set_fixed_h` 固定容器结算高度；
- 子项内容超高时**溢出可见**（需要滚动时在子项内嵌 `scroll_at`）；
- `pos` 相对当前容器原点，不占父容器光标。

### 18.11 文本输入增强（单行 / 多行 / IME / 剪贴板）

**滚动跟随光标（超长文本）**：单行输入框在绘制时把文本左移 `WidgetState::text_scroll`
（`edit::scroll_follow_caret(caret_x, content_w, text_w, margin=8)`：光标移出右侧 → 左移，
clamp 到 `max(0, text_w - content_w)`）；光标 / 选择 / IME 候选定位都叠加该偏移。
⚠ **裁剪窗口必须固定在内容区**：文本命令 `rect.x -= scroll` 的同时，`clip.x = scroll`
补偿（clip 相对移动后的 rect）——否则裁剪窗口随文本一起左移，永远只显示文本开头且
偏离文本框（垂直滚动同理：`clip.y = scroll` 补偿 `rect.y -= scroll`）。

**文本选择 + 剪贴板**：

- 按下时 `sel_anchor = caret`，按住拖动 → 光标跟随鼠标（选择范围 = `[min,max)`，
  纯逻辑 `edit::sel_range` / `selected_text`）；选择高亮在**文本之下**绘制
  （同一 elem 图形组先于文字组），颜色 `InputStyle::sel_bg`；
- **拖选越界跟随 + edge-scroll**：拖出输入框后光标仍跟随鼠标（越界 clamp 到文本两端）；
  鼠标停在框外右缘/下缘时**自动滚动**（光标位置 = 鼠标 + 上一帧滚动偏移）——"看不见的
  地方也选得到"；**纯单击（无位移）释放时清理 anchor**；
- **无 Shift 的方向键移动 = 单选**（清除选择）；**Shift + 方向键/Home/End = 扩展选择**；
  **Ctrl+A = 全选**；
- `Ctrl+C/V/X`：复制选择 / 粘贴（替换选择）/ 剪切（`arboard` 系统剪贴板；
  失败静默）；⚠ **Ctrl 按下时过滤 `chars()`**（winit 会把 'c'/'v'/'x' 当字符给出，
  否则 Ctrl+C 留下 'c'、Ctrl+V 粘贴后多 'v'）；⚠ **单行输入框粘贴过滤换行**
  （HTML input 语义——多行粘贴拼接成一行，否则 '\n' 进入单行文本错乱）；
- **选择高亮受内容区裁剪**（不溢出输入框 / 滚动容器），多行高亮随垂直滚动上移；
- **高亮只覆盖选中文字本身**：宽度经 `edit::selection_highlight_w(ink_w, space_w)`
  ——`ink_w > 0` 用 `ink_w`，只有空行 / 零宽选区才用一格宽兜底（`w == 0` 的高亮块会被
  丢弃 ⇒ 空行在选区里会看不见）。⚠ 曾经**无条件** `+ space_w` 做"行尾提示"：每次选到
  行尾 / Ctrl+A 都多一格，多行逐行相乘 → 观感就是"选择高亮里多出空格"（见
  `docs/DEBUGGING.md` §8.5）；选区跨行的行尾语义由"下一行同样有高亮"表达即可；
- **窗口/面板拖拽协同**：输入框按下置位 `Ui::press_claimed` → `window_impl` /
  `panel_impl` 不建立拖拽基准（选择拖拽优先，窗口从空白/标题区拖动），并**清除
  旧拖拽基准**（否则 `update_drag` 已置 dragging，残留 press_mouse 被当作基准 → 窗口"瞬移"）。

**多行 TextArea**（`text_area_at` / 容器内 `p.text_area`）：Enter 插入 `\n`（`chars()`
已过滤控制字符，换行必须显式处理）、↑/↓ 跨**视觉行**（保持列）、Home/End 视觉行首尾；
渲染用 `create_buffer_wrap`（内容区宽度自动换行），垂直滚动（`WidgetState::scroll_y`：
滚轮 + 光标行跟随），clip 相对文本块（上缘 = scroll_y）。

- **光标 / 点击 / 选择按"视觉行"（自动换行后）定位，与显示完全一致**
  （[`rjw_text::Text::visual_lines`]：每个 `LayoutRun` 一个视觉行，含全文字节范围；
  `edit::char_to_byte` / `byte_to_char` 换算）；跨视觉行选择高亮逐行绘制；
- **行距**：行高 = 字号 × `Theme::line_spacing`（默认 `DEFAULT_LINE_SPACING` = 1.2）——
  排版缓冲（`line_mult`）、光标 y、高亮、内容测量（`measure_buffer`）全部一致。

**控件自持排版缓冲**：输入框 / TextArea 的 `Arc<Buffer>` 缓存在
`WidgetState::text_buf`（key 含文本/字号/字体/换行宽/行距/版本）——文本频繁变化的
输入框**不写** `UiState::text_buffers` 全局缓存（不污染静态标签缓存）；`DrawKind::Text`
带 `buf` 字段（不进窗口内容签名——排版由 text/size/family 决定，**行距另经签名前缀并入**，
见 §18.7）。

**IME 组合候选浮动提示框**：preedit 非空时在输入框**下方**画浮动小框（底色 + 边框 +
灰色候选，自动宽度），不再占行内；系统候选框 `set_ime_cursor_area` 跟随光标
（含水平 / 垂直滚动偏移，物理像素）。

**维护约定**：文本编辑的纯逻辑（行/列换算、选择、滚动、byte/char）在 `edit.rs`
（无 GPU，可单测）；新增编辑控件时复用 `edit::*` 与 `clipboard_get/set`，并在按下
响应中置位 `press_claimed`。

**运行时导入字体 / 图片**（应用侧"文件 → 资源"，引擎只提供通路）：

```rust
// 字体：加载进**运行时**文本子系统（UI 排版与字形图集都用它），拿回新增族名
let fams = text.load_font_data(std::fs::read(path)?);   // → Vec<String>（空 = 该族已在库）
if let Some(name) = fams.first() { theme = theme.with_font_family(name); }

// 图片：帧内注册纹理（世界层与 UI 层共用同一个 Arc<TextureRegistry>）
let tex = f.draw().gpu().texture("imported", Rgba8::new(&rgba, (w, h)));  //  → uid
let bg = ImageBg::new(tex.uid, Vec2::new(w as f32, h as f32));
window.style(style.with_bg_image(bg));
```

- `Text::load_font_data` **返回新增族名**（与 `fontdb` 的槽位顺序无关的集合求差）：
  `label.font_family(name)` 只认族名，应用必须知道"这个文件叫什么族"才能用；
  `FontSystem::new()` 启动时已索引系统字体 ⇒ 导入**系统字体自己的文件**会返回空
  （"该族已在库里"），这不是失败。
- 图片通路与启动期（`Gfx::texture`）**共用一张纹理表** ⇒ 拿到的 `uid` 在 UI 后端
  一样解析得到；`Gpu` 已进 `rjw_kruskie::prelude`（运行时上传要它）。
- ⚠ 应用侧的两个坑（示例 `examples/eg260818UI/src/filedialog.rs` 有完整实现）：
  ① **系统文件选择器是阻塞调用**，只能在**帧外**弹（UI 里点按钮只记待办）；
  ② `Gpu` 只能从 `Frame::draw()` 拿（`Ctx` 没有）⇒ 图片注册在**帧内**做。
  脚本化验证（不弹对话框）：`--sim-import <路径>`，见 `docs/DEBUGGING.md`。

### 18.12 窗口外框（标题栏 / 关闭 / 收起）与缩放柄令牌

**位置**：`.pos(..)` **不调** = **引擎自动分配**（Win32 `CW_USEDEFAULT` 语义）：按窗口
**首次出现顺序**级联（每级右下 `AUTO_POS_STEP` = 28 **逻辑**像素，越出视口可用范围**回绕**），
分配结果记进 `UiState::auto_pos` ⇒ **跨帧稳定**（不会每帧顺级联往下漂）。优先级：
`Ui::pos_handler` 脚本（优先级降序）> 用户拖拽（`UiState::panel_pos`）> **自动位置** >
`.pos()` 传入值兜底 —— 自动位置只是"初值"，用户拖过就停在用户放置处；
`UiState::reset()`（示例里的 "R 重开"）连 `auto_pos` 一起清空 ⇒ 重新从第一格级联。

窗口的**外框**是三个独立选项，责任链上按需开启（`ui.window(id)` 的 builder）：

```rust
let mut open = true;        // 应用持有：× 点击直接置 false
let mut folded = false;     // 应用持有：收起状态
ui.window("win_a")
    .pos(vec2(560.0, 240.0))
    .width(220.0)
    .title("窗口 A")              // 标题栏（不调 = 完全没有栏）
    .close_button(&mut open)      // 右上角 ×（**贴窗口外框右缘**，点击 ⇒ *open = false）
    .collapsible(true, Some(&mut folded))   // 右上角 ⌃；状态**应用持有**（点击 ⇒ *folded 取反）
    .show(|w| {
        w.label("内容（收起时整块跳过）");
    });
// 也可以把收起状态**交给引擎托管**（应用不必多一个字段）：
ui.window("win_b").title("窗口 B").collapsible(true, None).show(|w| { w.label("…"); });
// 代码里收起 / 读取（引擎托管的那些；键 = 窗口**绝对 ID**）：
ui.state_mut().set_collapsed("win_b", true);
let folded = ui.state().is_collapsed("win_b");
// ⌃ 收起后同一窗口只剩标题栏；× 关掉后**整窗短路**，重开由应用决定：
if !open && ui.button("reopen_a", "显示窗口 A").clicked() { open = true; }
```

| 现象 | 机制 |
|---|---|
| **不调三个选项 = 逐像素等于旧行为** | `WindowChrome::bar_on()` 为假 ⇒ 不录标题栏、不加通条、不动任何几何（既有截图 / sim 基线不受影响） |
| 标题栏 = 内容**第一行**（占位） | 第一遍只在窗口顶边**占位**（`y = 0`，高 = 一行）让内容从条下开始 —— 窗口高度因此**自然**包含它 |
| **"占位"与"绘制"分开：绘制在结算之后** | 标题文本 / 按钮由 `ornament_at`——**一个长宽已确定的容器**（`rect = (0, 0, size.x, bar_h)`）——在**窗口尺寸结算之后、按下裁决之前**录 ⇒ 那里拿到的是**最终外框宽**，所以按钮贴外缘、标题可按最终宽度居中/省略（旧实现第一遍就录，自动宽窗口不知道最终宽度，只能回退"跟随标题"）。`ornament_at` **不 note_content** ⇒ 装饰绝不参与尺寸结算。⚠ 它必须在**窗口 ID 命名空间内**（`with_id` 作用域里）跑：否则按钮 id 丢掉窗口前缀，帧末"被更高窗口遮挡"的复核会把它们当成 win=0 内容而**撤销按下**（实测：⌃ 能命中但点不动、收起状态永不翻转） |
| **自动宽窗口的宽度由内容决定** | 占位宽 = 0（展开态）⇒ 标题**不再撑宽窗口**；只有**没有内容**时（收起态且未给 `.width()`）才用"标题 + 按钮簇 + 内边距"兜底（否则塌成 `2×pad`、标题被省略成空）。固定宽窗口（`.width(..)`）不变 |
| **标题行贴窗口顶边** | 录标题行**之前**把内容光标抬到 `y = 0`（x 保持内容左缘）⇒ 标题 / ▲ / ✕ **上移一个 `pad_total`**，下一个内容行自然落在「条下沿 + `gap`」——即**下面的内容与窗口高度各少一个 `pad_total`**（用户实测："可以往上抬"） |
| 通条底色 + 分隔线 | 整窗宽矩形，高 = `title_bar_h(row_h)` = **一行**（`title_bar_h` 是纯函数、可单测），底边那条就是面板边框色的 1px 分隔线；圆角取面板**上面两角**，与面板边框**连续**（不会"标题栏把上边框啃掉"）。⚠ 条只是**背景装饰、不裁剪内容**：标题 / ▲ / ✕（边长 `row_h - 2`）允许**比条高再高一点**（用户明确要求） |
| 标题栏空白处仍可拖窗 | 只有 `×` / `⌃` 上的按下会 `claim_press()`（与滑块 / 滚动条同一机制）——点按钮不会顺带把窗口拖走 |
| **caption 按钮贴窗口外框右缘（Windows 风格）** | 按钮**不在"行"里排**，而是用 `Ui::add_at` **绝对定位在窗口外框坐标系**（窗口帧局部 `(0,0)` = 外框左上角）⇒ 簇右缘 = **外框右缘 − `TITLE_BUTTON_INSET`（0）**、`y = 0`、高 `row_h`。落点由纯函数 `ui.rs::title_bar_layout`（可单测）解算（见下）；`✕` 恒在**最右**（顺序 `[⌃][✕]`）。⚠ 旧实现把按钮当行内子项、用 `spacer = 内容宽 − 标题宽` 推到**内容**右缘 ⇒ 永远差 `pad + 4`（实测 win_a @150% DPI：✕ 右缘 340 / 外框 358 ⇒ 偏左 18px），且 spacer 随文本测量漂 |
| 贴外缘 + 圆角要一起处理 | 最右按钮的**右上角取面板右上圆角**（`TitleIconButton::corners(..)`）⇒ 贴外缘时不会方角戳出圆角轮廓（Windows 11 的 caption 高亮同样跟窗口圆角走）；没开 `.style(radius)` 时面板圆角 0 = 纯直角贴角 |
| 自动宽窗口 | 没有 `.width()` 就没有"外框右缘"可贴 ⇒ 簇**跟随标题**（`cluster_x = pad + title_w + gap`），外框宽由内容推导（与旧版自动宽窗口一致） |
| `×` 的关闭语义 | `*open = false` 时**整窗短路**：不录制、不写原点 / 尺寸、**不占遮挡矩形**（不会留下"看不见却挡点击"的窗口）；下一帧起彻底消失，**重开是应用的责任** |
| `collapsible(show, collapsed)` | 收起状态**两种所有权**：`Some(&mut bool)` = 应用持有（`show = false` 时按钮不画，但 `*c` **照旧生效**——菜单 / 代码可收起展开而不必放按钮）；`None` = **引擎托管**（存 `UiState::collapsed`，键 = 窗口**绝对 ID**；点 ⌃ 由引擎翻转；应用用 `UiState::{is_collapsed, set_collapsed, toggle_collapsed}`，`reset()` 一并清空）。状态在录制**开头**读取 ⇒ 两种都是"点击当帧不变、**下一帧**生效" |
| 按钮是**几何**不是字形 | `Icon::Close` / `ChevronUp` / `ChevronDown`（`TitleIconButton`，只依赖公开 API）⇒ 换字体不会变豆腐块 |

> ⚠ **别再用"内容右缘 + spacer"排 caption 按钮**：那条路上有三个坑，实测各差 `pad`、4px，
> 还会互相抵消成"看起来差不多"：
> 1. `Ui::avail_w()` 返回 `fixed_w − 2×pad`（`Frame::fixed_avail_w`），补 `2×pad` 才是**内容宽**；
> 2. 内容右缘比**外框右缘**又少 `pad`（`Frame::natural_size`：`fixed_w + 2×pad` 才是外框），
>    旧实现还额外留了 4px 余量 ⇒ ✕ 离窗口右缘 `pad + 4`；
> 3. spacer 要用**实测标题宽**才算得准，字体 / 字号 / 字重一变就漂。
> 现在：按钮走 `Ui::add_at` 绝对定位（`title_bar_layout` 纯函数解算 + 单测钉住"簇右缘 =
> 外框右缘 − inset"），标题留在行里、按 `title_max` 省略号截断。
> 排查开关：`RJ_CHROME_TRACE=1` 打印**解算结果** ——
> `chrome[collapsed=false] bar_w=358.0 title_w=62.0 title_max=252.0 collapse=[275.0,0.0 37.0x39.0] close=[321.0,0.0 37.0x39.0] inset=0.0`
> （断言口径就在这条里：`close.x + close.w == bar_w − inset`）。

**可用宽下传：嵌套容器不会把内容排到固定宽窗口外面**：

`.width(150.0)` 这类**窄的固定宽窗口**里，`row(标签 + 输入框)` 曾经整排突到面板外
（用户实测："指定 width 里，width 较小的时候控件会突出去，直到你去拖拽缩放"）。三条机器保证：

| 规则 | 位置 | 说明 |
|---|---|---|
| `Ui::avail_w()` **由内向外**扫 frame 栈 | `ui.rs::avail_w` / `layout.rs::stack_avail_w` | 只看 `frames.last()` 时，`row`（自身不设固定宽）里的控件拿不到外层窗口的可用宽 ⇒ `Label` 不换行、按自然宽排 |
| 水平行的**剩余宽**参与约束 | `Frame::remaining_w` | 一行里**后面的**控件可用宽 = `内容盒右缘 − 光标`（左推）/ 光标 − 内容盒左缘（右推）。只按"单子项 ≤ max_w"不够：标签 126 + 间距 9 + 输入框 210 每个都没超 276，**整行**却超了 |
| 嵌套容器**继承**父级内容最大宽 | `Ui::container` → `Frame::set_max_w` | 子 frame 自己没固定宽时继承（扣掉自己的内边距）⇒ 子项被 clamp、`LimitedInParent` 控件拿到 `avail_w` 后自动换行。**只改上限，不改写结算宽**（容器仍按内容结算，不会被撑成整宽） |

> ⚠ **两个刻意的取舍**：
> 1. 余量 < `ADAPT_MIN_W`（24 物理像素）时**不再压窄**：宁可让内容溢出可见，也不把控件
>    压成一条线 / 0 宽（"控件凭空消失"比"内容溢出"更难排查）。
> 2. 控件自己的**下限**也要被压住：`TextEditor` 的 `.resize(..)` 路径会用主题下限
>    （`input.min_w`）把框**撑回**去（申请 141、画 210 —— 实测就是这样突出去的）。
>    现在它按**实际申请到的宽**（`rect.w`）压下限（`TextEditor::clamp_to_avail`）。
> 验收：`--sim-row-overflow`（`docs/DEBUGGING.md` §2），行宽 ≤ 可用宽 + 两个方向的失败
> 都验过（改回 `content_max_w → None` ⇒ 行宽 271 > 225）。

**内容被裁切时，窗口外的部分不可命中**（"幽灵控件"）：

窗口是 `Placement::Clip`（或高度被用户拖过 ⇒ 固定尺寸视口）时，**绘制**命令会被改写裁剪层，
但子控件的**命中区**照旧登记 ⇒ 溢出到面板外的控件"看不见却点得到"（用户实测的
"Vertical 缩放幽灵控件"）。修法：`window_impl` 在被裁切时记 `cur_win_hit_limit`
（绝对矩形，用**上一帧结算尺寸**——尺寸要录完才知道，而鼠标事件本来就针对"屏幕上已有的
几何"），`hit_impl` 在**登记命中区之前**要求鼠标点落在限制内：越界的命中既不响应、也**不会
把窗口的遮挡矩形撑到窗口外面**（同一条守卫顺带修掉"遮挡矩形随溢出内容变大"）。
未裁切的窗口（`Placement::Expand`）**不变**：溢出内容看得见就还能点（见 §"看得见就能点"）。

**缩放柄令牌**（固定宽窗口右下角那个"拖拽按钮"）：

`PanelStyle::grip: GripStyle { shape: GripShape, color, size, step, count }`，
`GripShape::{Squares（默认，历史观感）, Bars（`Icon::Grip` 三条横线）, Diagonal（三条 45° 斜线）, Hidden}`；
逐窗口入口 `PanelStyle::{with_grip, with_grip_color, with_grip_shape, without_grip}`。
`Hidden` 只是**不画图案**，**拖动缩放照旧**——命中区独立存在（`grip.extent()`，
下限 14px）。只有**允许拖拽缩放**的窗口才画柄（见下节 `.resize(..)`）。

> ⚠ `Bars` 画的是**三条实心横杠**（`push_solid_rect`，宽 `size*count`、高 `size`、
> 间距 `step`），**不用 `Icon::Grip` 图标**：图标每条杠只有 `size` 高（默认 4 逻辑像素），
> 再叠上 `Theme::feather` 的羽化带（默认每侧 0.5）就把三条糊成一坨（实测："三横看起来
> 是斜的一坨"）。实心矩形没有羽化，任意尺寸都读得出三条。
>
> `Diagonal` 是**三条 45° 斜线**（从左下到右上；`Icon::GripDiagonal`）——经典"缩放角"观感。
> 几何契约：三条线的**首端点在一条水平线上等距**、**末端点在一条竖直线上等距**，即第 `i` 条
> 从 `(s, 0.92)` 到 `(0.92, s)`（`s = 0.20 / 0.40 / 0.60`）⇒ 三条都是 45°、互相平行、
> 垂直间距相等（单测 `draw::corner_radius_tests::grip_diagonal_matches_the_spec` 钉住）。
> 它只能走图标（一条斜线不是轴对齐矩形），所以方框取 `size * count * 1.5`（比横线版大 50%）：
> 斜线在 `size*count` 的小方框里间距不到 1px，羽化会把它们糊成一片。`GripStyle::extent()`
> （命中区下限的来源）同步按 1.5× 算。

### 拖拽缩放（窗口）：显式 `bool` + 轴向

```rust
ui.window("w").width(200.0).resize(true, Resize::Both).show(|w| ..);      // 宽高同调
ui.window("popup").width(300.0).resize(false, Resize::None).show(|w| ..); // 固定宽但不可拖
```

| 写法 | 效果 |
|---|---|
| **不调 `.resize(..)`** | **旧行为**：有 `.width(..)` 就能横向拖（右下角柄），没有就不出柄 |
| `.resize(false, ..)` | **不画柄、不响应拖拽**；`.width(..)` 仍是布局固定宽（菜单 / 下拉浮层用） |
| `.resize(true, Resize::Horizontal)` | 只调宽（`↔` 光标） |
| `.resize(true, Resize::Vertical)` | 只调高（`↕` 光标）：宽度仍由内容决定，拖出的高度跨帧持久 |
| `.resize(true, Resize::Both)` | **宽高同调**（`↖↘` 光标）：高度跨帧持久于 `UiState::window_heights`，被拖过之后由用户接管（内容不再撑高；**并且内容自动裁剪**，见下） |

> ⚠ **没有 `.width()` 也能拖宽**（用户实测："resize Horizontal 不在设置 width 的情况下
> 无法缩放"）：`window_impl` 里持久宽（`UiState::window_widths`）必须**无条件**读
> （`width.or(persisted)`），只在 `width.map(..)` 里读会让拖出来的宽度**没人用**
> （写进去又被丢掉）。语义与高度一致：**拖过就由用户接管**（该轴不再参与内容撑开）。
> 轴 → "谁接管" 的表格抽成纯函数 `resize_fixes_width` / `resize_fixes_height` 并单测
> （`resize_axes_pick_the_axes_that_the_user_takes_over`）。
>
> ⚠ **缩放柄的拖拽基准属于"某一次按住"**：`resize_handle_apply` 在松手时清空
> `press_mouse` / `press_panel`，并在**拖拽本次刚激活**时补一次基准（按下边沿可能丢：
> 焦点切换 / 注入式输入 / 上一帧命中被遮挡）。旧实现只在 `down_edge && hit` 捕获基准且
> 从不清空 ⇒ 下一次拖拽沿用**上一次**的基准，尺寸每帧按位移**连乘**下去
> （用户实测："窗口收缩高度应当为恒定值"：往上收缩时高度一档一档掉到下限）。
> 诊断开关 `RJ_GRIP_TRACE=1` 打印每次 `hit/edge/drag/基准/结果`。
>
> ⚠ **窗口柄让位给内容与标题栏按钮**：柄的**命中**在内容之前算（登记顺序决定控件级
> 遮挡的胜负：后登记者在上 ⇒ 内容里的控件赢），但**应用**推迟到内容 + 标题栏之后，
> 且只在 `!press_claimed` 时生效 —— 否则"点面板内最后一个控件的缩放柄"会连带把窗口也缩了
> （用户实测："在文本编辑器控件上缩放柄无法使用"）。同一条也让收起态里 ⌃/✕ 的**行中心**
> （正压在窗口右下角柄上）照常可点（`--sim-chrome` 阶段 5）。

> ⚠ **高度被用户固定 ⇒ 内容强制裁剪**（`window_content_clipped(strict, fixed_h)`）：
> 窗口一旦变成"固定尺寸视口"，内容撑不高它了——不裁剪就会**画到窗口外面**
> （用户实测：TTT 窗口缩小后标签溢出到面板外）。触发条件是**高度**被拖过，
> **固定宽不触发**（固定宽窗口的高度仍由内容决定，垂直方向没有溢出可言）；
> `.placement(Placement::Clip)` 照旧是显式开关。裁剪之后窗口里的 `scroll_at` 才谈得上滚动
> （可视区之外的内容被裁掉、滚动条能带回来）。单测
> `ui::tests::window_content_clips_when_height_is_user_fixed`。

- `allow` 是**显式 bool**（用户指定的签名）：枚举 `Resize` 只表达"哪条轴"，
  "允不允许"用布尔更直白；判定抽成纯函数 `resolve_window_resize` 并单测
  （"没设置 = 旧行为"最容易在重构里丢，而它在 GUI 里几乎看不出来：柄画了但不响应、
  或没画却响应）。
- 拖拽基于通用 `resize_handle`（`current` / `min` **两轴都给**），按下即 `claim_press`
  ⇒ 拖柄不会顺带把窗口拖走。
- **验证**：`--sim-resize` 三段 —— ① 斜向拖 `img_box_fill` 的柄 +60/+40 ⇒ 结算尺寸
  `326×130 → 386×170`：两条轴都动，且**变化量正好等于注入位移**（若脚本每帧重算目标点，
  会退化成"追着拖"（+180/+223），所以这条断言同时也守着脚本自身）；② **没有 `.width()`**
  的 `grip_win` 横向拖 +50 ⇒ `378×82 → 454×82`（只宽变、高不变 ⇒ `Resize::Horizontal`）；
  ③ **收缩高度**：往上拖 40 ⇒ `170 → 130`，且**松手当帧与 40 帧后相同**
  （"窗口收缩高度应当为恒定值"：不回弹、不抖动）。
  ⚠ 脚本的拖拽目标点必须**冻结**（帧号记一次）：柄会随窗口尺寸移动，每帧重算 = 鼠标
  追着柄跑 ⇒ 位移逐帧累加（阶段 1 / 阶段 3 各踩过一次）。

**验证**（都不需要人眼看屏幕）：

```bash
cargo run -p eg260818UI -- --sim-chrome --frames 100   # 真的去点 ⌃ / ×（坐标从 debug_dump 的 win_a 外框矩形推导）
# sim-chrome: scale=1.5 ⌃=Vec2(353.5, 714.75) ×=Vec2(399.5, 714.75)  ← 点的是**贴右缘**那格
# sim-chrome: win_a open=true  collapsed=false size=(358,306)   ← 初始
# sim-chrome: win_a open=true  collapsed=true  size=(358,53)    ← 点 ⌃：只剩一行标题栏
# sim-chrome: win_a open=false collapsed=true  size=(0,0)       ← 点 ×：整窗短路
# sim-chrome: win_a open=true  collapsed=true  size=(358,53)    ← 应用重开
# sim-chrome: win_a open=true  collapsed=false size=(358,306)   ← 应用展开
```

（上面是 150% DPI 的实测值。）`size` 是 `.show(..)` 的返回值（窗口结算尺寸），`pos` 不变、
五段 `size` 对称即证明**按钮上的按下没有变成窗口拖拽**，也证明按钮右移**没有改变任何窗口
尺寸**。⚠ 点击点必须**从 dump 的外框矩形推导**（`right = origin.x + size.x`）：写死
"内容右缘 + pad"那种算法在按钮右移后就点不准了 —— 脚本本身也是这条不变量的回归。
引擎侧的不变量由 `ui::tests::title_bar_buttons_hug_the_window_outer_right_edge` /
`title_bar_layout_*` 与 `window_chrome_bar_and_collapse_flags` 守着
（"空外框不画栏" / "`collapsible(false, ..)` 不画栏但状态生效" / "`None` 引擎托管"）。

> ⚠ **收起态里 resize 柄会盖到 caption 按钮上**（既有缺陷，非本轮引入）：窗口只有一行高时，
> 右下角缩放柄的命中区（`GripStyle::extent()`，本机 150% DPI 实测 `35×35`、起点 `y = 723`）
> 往左上延伸会覆盖 `×` / `⌃` 所在的一行（✕ 矩形 `(381,705,37,39)`，y 到 744），而柄在
> `window_impl` 里**先于**按钮判定 ⇒ 点行中心会被柄抢走。`RJ_HIT_TRACE=1` 的实证：
> `hit[frame 41] win_a::resize OK rect=(383,723,35,35) mouse=(399.5,724.5)`，同时
> `RJ_CHROME_TRACE` 里 `bar_w` 358 → 357（`window_widths` 被拖了 1px）。`--sim-chrome`
> 因此点按钮的**上半部**（`row_h * 0.25`，y < 723）绕开柄，把"按钮路径"与"柄路径"分开验证；
> 修柄 / 按钮重叠另案。

### 18.13 菜单栏（menu_bar）= **一行 + 全宽背景**

菜单栏的**本质是 `row`**（`PackSide::Left`）多了一条**覆盖整栏宽度的背景**（可以是一整块
面板 / 整个屏幕）；里面既能放**菜单触发器**，也能放**任何控件**——按钮 / 标签 / 文本输入 /
分割线（含**竖向分割线**）。`MenuBar` `Deref` 到 `Pack` ⇒ `UiAdd` 的方法直接可用
（与 `MenuCtx` `Deref` 到 `Window` 同一套模式）。菜单点开后是**下拉面板**，内容同样是
**闭包上下文**（`item` / `item_checked` / `caption` / `separator`，并 `Deref` 到 `Window`）：

```rust
use rjw_ui::{Ui, UiAdd, Divider, Size};   // `UiAdd` 让 `bar.label` / `bar.button` / `bar.add` 可用

ui.menu_bar("menubar", vec2(620.0, 12.0), |bar| {   // 位置 = 栏左上角（顶层 = 屏幕坐标）
    bar.width(Size::Physical(Vec2::new(screen_w, 0.0)));   // 背景铺满整屏（不调 = 自然宽）
    bar.menu("文件", |m| {
        m.caption("文件名过滤");                        // 纯文本行（不可点）
        m.text_input("menu_filter", &mut self.filter);  // 菜单里也能放**文本输入**
        m.separator();                                  // 菜单内的水平分割线
        if m.item("导入图片…") { /* 点完自动收起 */ }
    });
    bar.menu("视图", |m| {
        m.item_checked("窗口 A 显示", &mut self.show_a); // 带勾选：点击翻转 `&mut bool`
        m.row(|r| {                                     // **横向排版**（`Deref` 到 `Window`）
            if r.button("d0", "紧凑").clicked() { }
        });
    });
    bar.separator_v();                                  // ← **竖向分割线**（菜单组 | 其它）
    bar.add(Divider::new().vertical());                 // ← 等价写法（`separator_v()` 就是它）
    bar.label("状态：就绪");                             // ← 栏里放**任何控件**都行
});
```

| 行为 | 机制 |
|---|---|
| **栏 = 一行** | `Ui::menu_bar` 内部 `container(pos, Frame::new_stack(PackSide::Left, gap, pad))` + `force_h_all(row_h)` ⇒ 触发器 / 竖分割线 / 塞进来的控件**同一个行高**；`gap` / `pad` 取 `Theme::menubar`（默认 2 / 4）⇒ 第一个触发器从 `pos + pad` 起 |
| **全宽背景** | 子项录**完之后**再 `push_panel_like(栏矩形, …, elem = 0)`：按 `(win, depth, elem, seq)` 压在**同深度已有的底层绘制之上**、所有控件之下（子项在容器里 depth + 1）。默认取 `Theme::menubar`（`bg` / `border` / `border_w` / `radius`），链式可覆盖：`bg` / `border` / `border_w` / `radius` / `width`。⚠ 它**不**盖在普通窗口（`win > 0`）之上 |
| **底边线只画底边** | 单独 `push_solid_rect` 栏内下沿那条（**不是**四边环）：通栏条画环会在屏幕边缘多出两条竖线（`Theme::menubar.border_w = 0` 即不画） |
| **栏宽** | `bar.width(..)` 只决定**背景铺多宽**（不 clamp 子项）；右推某块用 `bar.min_size(剩余宽, 0.0); bar.label("")`（与标题栏同一招）。栏**不占父容器光标**（浮在顶层） |
| 展开状态 | `UiState::menu_open`（**触发器绝对 ID**；与 `combo_open` 分开存——同一时刻只该有一个菜单开着，而下拉框属于某个控件） |
| 点另一个触发器 | 切换（旧的关、新的开）；再点自己 = 收起（走 `action`，**优先于**收起规则） |
| 点菜单项 | **执行 + 自动收起**（`item` / `item_checked` 内部把 `close` 标志交给栏） |
| **点栏外** | 收起（栏外 = 既不在**栏矩形**内、也不在下拉面板矩形内、也不在触发器上）——纯函数 `widgets::menubar::menu_bar_should_close`，逐组合单测 |
| 点栏内空白 / 竖分割线 / 栏里的别的控件 | **不收起**（"栏 = 一行容器"带来的语义；旧实现只认"落在某个触发器上"，会误关） |
| **Esc** | 收起。应用自己的 Esc 语义先看 `UiState::menu_open()`（菜单开着那一帧别抢） |
| 下拉面板 | 一个 [`Level::Normal`] + **`WindowClamp::Locked`** + **`.resize(false, Resize::None)`** 的浮层窗口（点它不置顶、**拖不动、也没有缩放柄**），且 z 落在**浮层区间**（`WIN_TOPMOST` 基址 + 嵌套层数，见 §18.15「分层 z」） |
| 触发器交互 | `allocate_sense(.., Sense::DRAG)`：占光标 + 命中 + 按下认领一次做完（按下不会被外层当成拖拽基准） |
| 触发器几何 | 宽 = **文字实测宽 + 2 × `Theme::menubar.trigger_pad_x`**，高 = `Theme::row_h`；栏高 = `row_h + 2 × Theme::menubar.padding`；子项间距 = `Theme::menubar.gap`（默认 2 —— 菜单条要**紧**，不是工具栏的 `Theme::gap`） |

**样式令牌**（`Theme::menubar: MenubarStyle`，**独立于 `Theme::button`**）：

| 字段 | 默认 | 语义 |
|---|---|---|
| `bg` | `Palette::surface_raised` | 栏背景（铺满 `bar.width(..)` 的整条宽度） |
| `border` / `border_w` | `Palette::border` / 1.0 | **底边线**（只画底边，不是四边环；`0` = 不画）。线画在栏内下沿 |
| `radius` | `CornerRadius::default()`（0） | 栏圆角。**不参与 `with_radius` 级联**（通栏条圆角化会露出底下的内容） |
| `padding` | 4.0 | 栏内边距（**四边同值**）：横向 = 首 / 末子项与栏边的距离；纵向 = 栏比内容高出来的那截 |
| `gap` | 2.0 | 触发器之间的间距 |
| `font_size` / `font_family` | 13.0 / `None` | 触发器文本（`with_font_size` / `with_font_family` 会级联到它） |
| `fg` | `Palette::text` | 触发器文字色 |
| `trigger_bg` | **全透明** | 触发器**常态**底色 —— **这一条就是"菜单条 ≠ 一排按钮"的关键**：全透明 ⇒ 引擎连背景命令都不推，只有悬停 / 展开时才画圆角高亮 |
| `trigger_hover` / `trigger_pressed` | `Palette::surface_hover` / `surface_active` | 悬停 / 按下（含"当前展开"）底色 |
| `trigger_radius` | 4.0（`with_radius` 级联为 `min(r, 6)`） | 触发器高亮圆角 |
| `trigger_pad_x` | 10.0 | 触发器左右内边距（决定触发器宽） |
| `separator` / `separator_w` / `separator_margin` | `Palette::border` / 1.0 / 6.0 | 竖分割线颜色 / 线宽 / 两侧留白（`separator_v()` 用它，**不是** `Theme::divider`：菜单条里的竖线更短更淡） |

```rust
// 换主题：整组替换（或只改想要的字段）
let theme = Theme::dark().with_menubar(MenubarStyle {
    border_w: 0.0,                    // 不要底边线
    trigger_hover: Color::TRANSPARENT, // 连悬停高亮也不要（纯文字）
    ..Theme::dark().menubar
});
```
```toml
# 手写主题文件也能单独调它（⚠ 颜色写 0–1 归一化浮点：`{ r = 0.13, ... }`）
[theme.menubar]
padding = 6.0
trigger_pad_x = 12.0
trigger_hover = { r = 0.24, g = 0.25, b = 0.28, a = 1.0 }
```

> ⚠ **手写 TOML 的颜色是 0–1 归一化浮点**（`to_toml` 导出的形式）。写 `{ r = 32, g = 34 }`
> 这种**0–255 整数**会被当成"分量 > 1"⇒ 渲染时被夹到**全白**（不是报错）——这是手写主题的
> 经典坑，`docs/DEBUGGING.md` 有症状 / 排查。


> 面板的**录制 / 样式 / 宽度 / 关闭规则**全在 [`crate::widgets::menu`]（`menu::popup_show`）——
> 与 [`Dropdown`](crate::Dropdown) **同一套实现**，本模块只负责"横向一行 + 栏的判定"。
> 下面那些排版 / 锁位约定因此对**两者同时成立**（改动只在 `menu.rs` 一处）。

> ⚠ **下拉面板现在是"嵌套窗口"**（录在栏容器里 ⇒ dump 的 `origin` 相对**直接容器**）：
> 用 `debug_dump()` 算屏幕坐标时必须叠加**栏原点**（`p.origin + bar`）。不叠加就会点到栏外
> ⇒ 菜单当场收起（实测症状：`--sim-menu` 报"菜单没开 / 菜单项没执行"）。这是 dump 的既有
> 语义（嵌套窗口 origin 相对直接容器），`--sim-dropdown` 里子菜单那一段也一样要叠加。

> ⚠ **下拉面板必须锁位置**（`WindowClamp::Locked`）：它是个窗口，默认可拖——拖走之后
> 面板与触发器脱节，而**命中判定按窗口走**，视觉却跑别处（"控件严重错位"）。
> `--sim-menu` 阶段 2 专测这条：在面板空白处按住拖 600+px，面板原点必须不变
> （A/B 实测：去掉 `Locked` 立刻变成 `[FAIL] 面板被拖走了`，原点被拖到 x=1682）。
>
> ⚠ **下拉面板的排版**（三条一起才对齐，缺一条就"看着有点怪"）：
>
> 1. **内边距 = `popup_padding(theme)` = `ComboStyle::item_pad_x`**（`PanelStyle::padding` 是
>    **标量**，四边同值）：菜单项 / `caption` / `separator` / `row` 全部从**同一个内容原点**
>    起排 ⇒ 天然同列。勾选**框**画在菜单项**内容里**（不占内边距 —— 用户要的"小边距"）。
>    （别用"给下一子项缩进"的花招：垂直栈里子项 `x` **恒等于内容原点**，缩进宽度无效。）
> 2. **面板宽 = 上一帧的结算宽**（`UiState::window_sizes`，首帧自然宽、次帧起精确）：
>    `fill = 固定宽已知` ⇒ 子项请求"极宽"由窗口 clamp 到内容宽 ⇒ **高亮/分割线铺满面板**；
>    首帧必须请求**自然宽**，否则 1e6 的请求会把自然尺寸撑成一百万、面板宽度再也收不回来。
>    内容宽 = `max(上一帧面板宽, 最小面板宽) − 2 × (内边距 + 边框)`（第 2 帧即收敛；
>    `RJ_MENU_TRACE=1` 可看到 `prev=None → Some(226) → Some(240)` 这条收敛轨迹）。
> 3. **菜单内的分割线自己画**，不用 [`Divider`](crate::Divider) 的水平模式：`Divider` 的宽 =
>    `avail_w()`，而**自动宽**窗口里那是 `None` ⇒ 退回固定 120 ⇒ 线又短又不在该在的位置
>    （用户实测："Menu 分割线错位"）。自己画时请求"极宽"由窗口 clamp ⇒ 恒等于内容宽。
>    ⚠ 与第 2 条相反：**栏里**（`row` 里）的竖分割线该用 `Divider::vertical()` —— 那里宽度是
>    自然量（线厚 + 2×留白），高度由行强制，没有"可用宽"问题。
>
> `caption` 另外做了层级区分：字号 ×0.85 + `Palette::text_muted`（一眼看出是分组标题、
> 不是可点的项）；勾选标记是**方框**（`[☐]/[☑]`，画在菜单项**内容里**、方框列恒留位 ⇒
> 勾选与否文字都对齐），勾选时填 `CheckboxStyle::checked_fill` + 矢量勾号。
>
> 面板 z 用了哨兵，所以**菜单栏录在哪里都盖得住别人**（不必强求录在各窗口之后）。
> 排查通道：`RJ_MENU_TRACE=1` 打印**栏矩形 + 收起判定事实**（
> `menu[bar menubar] content=(252,39) bar=(135,18 252x39) on_trigger=… on_bar=… down_outside=…
> item_clicked=… esc=… popup=… close=…`）与每个下拉内容的行矩形（`item/separator/caption`
> 必须同 `x` 同 `w`，实测 `x=8 w=224`，DPI 1.5）。

**验证**：`--sim-menu` 三段——① 点「视图」触发器 → 菜单打开 → 点第一个菜单项 → 勾选翻转 +
菜单自动收起；② 在面板空白处按住拖 600+px → 面板原点不变（`Locked`）；③ **点栏内空白** →
菜单**仍开着**（`on_bar` 语义）。引擎侧不变量由
`state::tests::menu_open_is_readable_and_cleared_by_reset`（读得到 + `reset` 清空）、
`widgets::menubar::tests::close_rules_cover_every_combination`（收起规则逐组合）与
`style::tests::menubar_style_themes_scales_and_cascades`（主题派生 / DPI / 全局级联）守着。
实测（DPI 1.5，demo 栏铺满整屏）：

```
menu[bar menubar] content=(222,39) bar=(135,18 1920x51) on_trigger=… on_bar=… close=…
sim-menu: 面板原点=Some(Vec2(214.0, 65.0)) · 期望=Some(Vec2(214.0, 65.0)) … [OK] 菜单面板不会被拖动
sim-menu: 点栏内空白后 menu_open=true [OK] 点栏内空白不收起菜单
```

（栏宽 1920 = 整屏；栏高 51 = `row_h 39 + 2 × padding 6`；触发器几何 = 文字宽 + `2 × trigger_pad_x 15`。）

### 18.14 分段按钮组（`Segmented`）与"边框归零"的兜底

**分段按钮组**：一组**互斥**选项拼成一个整体（相邻段共享边、只有整组外侧角是圆的、
选中段高亮）。返回值写进 `&mut usize`（与 `Slider` / `NumberInput` 同风格）：

```rust
let mut idx = 1;                       // 当前选中段（跨帧由应用持有）
ui.row(|r| {
    r.add(Segmented::new("density", &["紧凑", "标准", "宽松"], &mut idx));
});
// 点第 3 段后 idx == 2
```

为什么单独一个控件：**段需要知道自己在组里的位置**（首/中/尾决定哪两个角圆、哪条边画
分隔线）——让三个 `Button` 相邻去猜布局不可能稳，所以由它一次画完整组
（宽度按各段文字实测、再按 `rect.w` 归一）。角点规则有单测
（`only_outer_corners_are_rounded`：首段左上/左下圆、中段全直角、尾段右上/右下圆）。

> ⚠ **分隔线独立于 `border_w`**：边框宽 > 0 时用 `ButtonStyle::border`，边框被关掉
> （`border_w = 0`，平面风格）时退化成 `Palette::surface_dim`——否则各段同一底色连成
> 一条长条，看不出这是三个选项。组的**外侧轮廓**仍走 `border_w`（关掉就真的没有外框）。

**边框归零（`border_w = 0`）时的可见性兜底**——凡是"只靠一圈描边存在"的东西都必须有
第二视觉来源，否则主题一关边框它就**整个消失**：

| 控件 | 兜底 |
|---|---|
| `Checkbox`（未勾选） | 本来只画一圈 `Border`；`border_w <= 0` 时改画**实心底**（`surface_sunken`，悬停 `surface_hover`）——否则未勾选的框消失，标签看起来"没有控件"（实测："控件严重错位"：两个勾选框一个有方块一个没有） |
| `Segmented` | 段间分隔线换成 `surface_dim`（见上） |
| `Button` / `Input` / `Panel` | 本来就有 `bg` 填充 ⇒ 不受影响 |

**验证**：`--sim-tuner` 阶段 3（点预设行第 3 段 ⇒ `preset == 2`，
`[OK] 分段按钮组可点`）+ `Segmented` 的角点单测。

### 18.15 按钮下拉菜单（`Dropdown`）——**统一的下拉面板**

图一（`难度 ▾` → `简单/普通/困难`）与图二（菜单栏下拉里的文本输入 / 分割线 / 菜单项）
**简并成同一个控件**：`Dropdown` 是普通 [`Widget`](crate::Widget)，用 `UiAdd::add` 加进
任何容器；菜单体是一个 [`MenuCtx`](crate::MenuCtx)（`Deref` 到 `Window`）⇒
**菜单内又可以 `UiAdd::add`**（文本输入 / 分割线 / 菜单项 / 横向排版 / 再嵌一个下拉当子菜单）。

```rust
use rjw_ui::{Dropdown, PopupSide, Ui, UiAdd};

// ① 选项列表模式：菜单项由引擎排（选中行打勾 + 整行高亮），点击写回 `&mut u32` 并收起。
ui.add(Dropdown::options("diff", "普通", &mut idx, &["简单", "普通", "困难"]));

// ② 富内容模式：菜单内容自己写（`m` 是 MenuCtx ⇒ 全部 UiAdd 方法 + 菜单语义）。
ui.add(Dropdown::new("file", "文件名过滤").width(160.0).menu(|m| {
    m.text_input("filter", &mut filter);        // ← 菜单里的文本输入
    // **责任链菜单项**（`Item`）：普通 / 勾选 / **子菜单** 三种形态 + 点击行为 flag。
    m.item(Item::new("保持打开").click_behavior(MenuClick::Keep));   // 点完**不收起**
    m.item(Item::new("窗口 A 显示").checked(&mut show_a));           // 勾选项（自持）
    m.submenu("编码", |s| {                                          // = Submenu
        if s.item("UTF-8") { }                                       // Hover 在行**右侧**展开
    });
    m.separator();                              // ← 分割线（自绘，满内容宽）
    if m.item("导入图片…") { /* 点完自动收起（旧写法 = Item::new(..)，`From<&str>`） */ }
}));
```

| API | 语义 |
|---|---|
| `Dropdown::options(id, label, &mut u32, &[&str])` | 选项列表模式（图一）：`label` 一般是"当前选项"文字 |
| `Dropdown::new(id, label).menu(\|m\| ..)` | 富内容模式（图二）：菜单体是闭包，参数 [`MenuCtx`](crate::MenuCtx) |
| `.side(PopupSide::{Below,Right})` | 面板方位：`Below`（默认，正下方 2px）/ `Right`（子菜单） |
| `.width(..)` / `.font_size(..)` | 触发器固定宽（默认按文字自动）/ 字号 |
| `m.item("文本")` | **旧写法（糖）**：= `Item::new("文本")`，返回"本帧是否被点击"；点击即收起 |
| `m.item(Item::new(..)…)` | **责任链菜单项**（[`Item`](crate::Item)）：`.click_behavior(MenuClick)` / `.checked(&mut bool)` / `.submenu(\|s\| ..)` |
| `MenuClick::{Close,Keep}` | **点击行为 flag**：点完「收起整个 popup」（默认）/「**保留** popup」（"点了还要继续操作"的项）。⚠ `Submenu` 恒为"不收起" |
| `m.submenu(label, \|s\| ..)` | **子菜单（Submenu）**：普通项样式 + 右侧 ▸，**Hover** 在行**右侧**展开（= `Item::new(label).submenu(..)`） |
| `m.item_checked(label, &mut bool)` | 旧写法（糖）：= `Item::new(label).checked(..)` |
| `m.caption(text)` / `m.separator()` | 分组标题（小字号 + `text_muted`）/ 自绘分割线 |
| `Ui::combo_at` / `UiAdd::combo` | **糖**（旧的"下拉框"入口，签名 / 行为不变）：= 选项列表模式 + 固定布局宽。`FontModal` 的字重下拉仍走它 |

**`Submenu` 的五条语义**（`Item::submenu` / `m.submenu`）：

| 行为 | 机制 |
|---|---|
| **Hover 即开** | 行的 `hit`（`hit_abs`）⇒ 开；判据是纯函数 `menu::submenu_open_now(hit, was, inside)` |
| **鼠标进子面板保持** | `inside` = 鼠标在本行**子面板或其任意后代窗口**内（`state.window_rects` 绝对矩形 + `id_in_window_tree` **按 `/` 边界**的祖先前缀判定）⇒ 移到同菜单其他行 / 移出菜单自动收起 |
| **开在行右侧** | `PopupSide::Right` ⇒ `popup_origin(行矩形, Right) = (行右缘 + 2, 行顶)`；面板最小宽用 `ComboStyle::item_min_w`（**不用行宽**，否则与父面板一样宽） |
| **点击不收起** | 子菜单行恒 `close_on_click = false`（`MenuClick` 对它无效），也不切换（Hover 已决定，避免抖动） |
| **链式收起** | 子面板里点普通项 ⇒ 子面板 `popup_show` 报 `item_clicked` ⇒ `*self.close = true` **冒泡到父** ⇒ 整条链（子 + 父）一起收 |

**状态**：子菜单是否展开是**行自持**的 `WidgetState::submenu_open`（`pub(crate)`，`reset()` 随
`widgets` 清空）——⚠ **绝不借** `UiState::combo_open`（那是**父下拉**的槽位：早期"菜单里嵌一个
`Dropdown`"就被它覆盖，症状是"点一下整条 popup 消失"）。层级不限（子菜单里还能再 `submenu`）。

**状态与语义**：展开状态跨帧持久于 [`UiState::combo_open`](crate::UiState::combo_open)
（**控件（触发器）的绝对 ID**，与 `menu_open` 记触发器一致；面板窗口 id = `<控件 id>::popup`）。
单槽 ⇒ 同一时刻只有一个下拉开着。点触发器切换；点菜单项 → 执行 + 收起；点面板外 / `Esc` → 收起；
键盘 Tab 可到、Enter/Space 展开、**↑/↓ 循环切换选项**（选中即收起）。

**点外收起的四条**（一条都不许丢，见 `menu::popup_show`）：

| 事件 | 行为 |
|---|---|
| 点了菜单项（`MenuCtx::item*` 上报） | 收起 |
| `Esc` | 收起 |
| 左键按下在**面板外** | 收起 |
| 左键按下在**面板内 / 触发器上 / 任意浮层（`z >= WIN_TOPMOST`）上** | **不**收起 |

最后那条是给**子菜单**留的：菜单里再开子菜单时，点子菜单不该被外层菜单当成"点面板外"
而把外层一起关掉（实现 = `Ui::window_under_mouse()` 的 z 是否落在**浮层区间**
`z >= WIN_TOPMOST`）。菜单栏那边还多一条"栏"的判定：按下落在**另一个触发器**上 =
切换菜单（不是收起）——所以 `MenuBar::finish` 用**本栏**的 `on_trigger` 收口。

**分层 z（浮层 = 基址 + 嵌套层数）**：所有浮层的 z 都从 `WIN_TOPMOST`（基址）起、按
**当前嵌套层数**递增（`Ui::push_overlay_z` / `pop_overlay`，见 `ui::overlay_z`）。

| 为什么不能是同一个哨兵值 | 说明 |
|---|---|
| 画面上 | 同一 z ⇒ 两层命令落进同一个 `(win, elem)` 分组排序，而窗口的**阴影 / 背景 / 边框**是 `elem = 0`、控件是 `elem ≥ 1` ⇒ **子层的阴影会被父层的控件盖住**（用户实测："下级 popup 阴影被绘制在了上级控件后面"）。分层 z ⇒ 子层（含阴影）整段排在父层之后 |
| 状态上 | `win_origins` / `win_ids` 按 z 键 ⇒ 同 z 只会留下最后一个（诊断通道漏掉嵌套浮层）；分层 z 后每个浮层有自己的提交原点 |
| 判定上 | "落在任意浮层上"是**区间判定**（`is_overlay_z`），不是 `== WIN_TOPMOST` |

普通窗口的 z 从 1 起按 `max+1` 递增并**排除浮层区间** ⇒ 浮层恒在最上（两者相差 ~2³²）。

**公开几何助手**（示例 / 脚本算坐标用，避免在脚本里抄魔数）：
`menu::item_h(font_size)`（行高）、`menu::popup_padding(theme)`（内边距）、
`menu::popup_origin(trigger, side)`（面板原点）、`menu::popup_gap(scale)`（行距）。
它们与引擎**同源**：`--sim-dropdown` 就是用它们算点击点、并用 `popup_origin` 反查"面板该在哪"。

**验证**：`--sim-dropdown`（**7 段，全 `[OK]`**）——
① 点触发器开下拉且**面板原点 = `popup_origin(触发器, Below)`**；
② 点选项 ⇒ 选中索引变 + 自动收起 + 面板消失；
③ 富内容下拉同样开得起来（同一个控件、同一套浮层）；
④ `text_focus()` 落在 `<下拉 id>::popup/...` ⇒ **菜单里的文本输入真可聚焦**；
⑤ **只悬停**子菜单行 ⇒ 子面板原点 = `popup_origin(行, Right)`、**父 popup 未消失**、
   且**子面板 z > 父面板 z**（分层 z 的硬断言：阴影不再被父层控件盖住）；
⑥ 点子菜单里的项 ⇒ 应用状态变 + **整条链**（子 + 父）收起；
⑦ 点 `MenuClick::Keep` 项 ⇒ 执行 + **popup 保留**。
引擎侧不变量由纯函数单测守着（`menu.rs`：行高公式、面板方位、宽度收敛、关闭真值表、行底色
优先级、勾选判定、**子菜单展开真值表**、**窗口子树前缀边界**、`Item` 责任链默认档；`ui/tests.rs`：
`title_bar_h` 贴顶、**`overlay_z` 分层与区间**；`dropdown.rs`；`state.rs`：`combo_open()` 可读 +
`reset` 清空）。

### 18.16 主题序列化（TOML：导出 / 导入 / 启动参数）

`Theme`（含全部子样式）可**序列化**为 TOML 文本，于是"调好的主题"能落盘、能进版本库、
能在启动时指定：

| API | 语义 |
|---|---|
| `Theme::to_toml() -> Result<String, String>` | **全量导出**：`format_version` 头 + `[theme]` 字段树（所有样式字段） |
| `Theme::from_toml(s) -> Result<Theme, String>` | 从 TOML **加载**（起点 = `Theme::default()`；缺字段回落默认） |
| `Theme::apply_toml(&mut self, s)` | **在当前主题上合并覆盖**（文件里出现的字段才改）——手写小文件最常用的语义 |
| `THEME_FORMAT_VERSION` | 格式版本（导出写进文件；加载时**比本引擎新** ⇒ 报错拒绝） |

```toml
format_version = 1              # 认不出 ⇒ 加载报错；缺失 ⇒ 整份文档当"裸主题表"

[theme]
row_h = 26.0
gap = 6.0
font_weight = 700               # u16 代理（Weight 在 rjw_text，不能在此 derive）
line_spacing = 1.2

[theme.panel]
padding = 6.0
radius = { tl = 6.0, tr = 6.0, br = 6.0, bl = 6.0 }   # 也可以写 `radius = 6.0`（四角同值）
bg = { kind = "vertical", colors = [ { r = 0.98, g = 0.98, b = 0.98, a = 1.0 }, { r = 0.94, g = 0.94, b = 0.94, a = 1.0 } ] }
shadow = { blur = 8.0, offset = { x = 0.0, y = 2.0 }, color = { r = 0.0, g = 0.0, b = 0.0, a = 0.47 } }
```

> **刷子（`Brush`）有显式表示 `{ kind = "solid"|"vertical"|"horizontal", colors = [..] }`**
> （`solid` 1 个颜色、`vertical`/`horizontal` 2 个）——**不要**用 serde 默认的"外部标签枚举"
> 表示（`{ Vertical = [色, 色] }`）：TOML 会把它写成**数组表** `[[…bg.Vertical]]`，读回来时
> `toml` 的枚举反序列化报 `wanted exactly 1 element, more than 1 element in 'button.bg'`
> （用户实测）。加载侧**兼容**旧写法（形状翻译，见 `theme_toml::translate_legacy_brushes`），
> 形状不对时给**带字段路径**的错误。

**三条语义**（`crate::theme_toml` 模块文档里有完整说明）：

1. **`to_toml` 是全量**：导出的文件导入回来逐字段一致（单测的强断言 = "再导出一次文本
   完全相同"）。唯一例外：`PanelStyle::bg_image` **不序列化**（纹理 `uid` 跨进程不可移植，
   `serde(skip)` ⇒ 加载后回落 `None`，要贴图由应用自己灌）。
2. **`apply_toml` 是合并**：先把当前主题序列化成 TOML 值，再把文件里的值**递归**合并上去
   ⇒ `gap = 12` 两行的手写文件就只改间距；`from_toml` = 在 `Default` 上合并。
3. **宽容**：缺字段回落默认（每个样式结构体 `serde(default)`）、多余键忽略（向前兼容）；
   只有**版本比本引擎新** / TOML 语法 / 字段类型不合法才 `Err(String)`（消息带原文，直接
   显示给用户）。

> **特性开关**：`rjw_ui` 的 `serde`（= `toml`，默认开）——`Color` 的 serde 由
> `rjw_color/serde` 带进；`Weight` / `Align` 用**代理模块**（数值 / 小写名字）；
> `CornerRadius` 反序列化**同时接受标量与表**（`radius = 3.0` 或四角表）；
> `Brush` 用显式 `{kind, colors}`（见上，含旧写法兼容）。
>
> **示例侧**（`eg260818UI`，`rfd` 文件选择器）：顶栏多了「导出主题…」「导入主题…」——
> 导出 = `to_toml` 落盘（另存为对话框），导入 = `load_theme_onto`（在**当前**主题上
> `apply_toml`）后当作**基底**生效（旋钮暂不生效，窗口里有「恢复调节」）；
> **`--theme <路径>`** 就是同一条通路在**启动时**跑一次：

```
cargo run -p eg260818UI -- --theme C:\my-theme.toml
theme: 启动载入 C:\my-theme.toml
```

**验证**（`rfd` 对话框无法无头跑，但"文件"这条通路是真实的）：

```
cargo run -p eg260818UI -- --sim-theme C:\rust-targets\sim-theme.toml --frames 70
sim-theme: ① 7403 字节 · row_h=26 gap=6 字重=400 · 再导出逐字相同=true [OK] 主题导出 → 文件 → 导入：字段级往返一致
sim-theme: ② 导入前 row_h=26 → 引擎侧 row_h=27（期望 27）[OK] 导入的主题真的进了引擎（全量文件改一行 ⇒ 只那一项变）
```

引擎侧不变量由单测守着（`theme_toml.rs`：round-trip 文本逐字相同、merge 只在文件写到的
字段上生效、版本过高 / 语法错误 / 缺 `[theme]` 的可读报错、`bg_image` 不入文件）。

### 18.17 维护约定（对 AI）

- 布局 / 命中 / 状态机是**纯逻辑**（`layout.rs` / `hit.rs` / `state.rs` / `focus.rs`），改动后跑 `cargo test -p rjw_ui`（无 GPU 依赖）。
- 新增控件 = 在 `ui.rs` 加 `Ui::xxx_at` 实现 + 在 `ui::UiAdd` trait 里加便捷方法默认实现（Panel / Pack / Grid 等全部容器自动获得，无需改宏）。
- **新增控件优先做成 `Widget`**（`impl Widget for Xxx`）：那样 `UiAdd::add` / `add_at` 天然可用
  （`Dropdown` / `Segmented` / `NumberInput` 都是这条路）。**别为同一种控件开两个入口**
  （`Ui::xxx_at` 只作为"显式 rect / 容器内占光标"的内部或糖入口，见 `Ui::combo_at`）。
- **下拉 / 菜单类浮层一律走 `widgets::menu::popup_show`**（唯一实现）：哨兵 z、锁定位置、
  `.resize(false, Resize::None)`、面板样式、宽度收敛、点外 / Esc / 点项的收起判定都在那里。
  复制一份浮层录制代码 = 迟早出现"两套菜单观感不一致"（本轮就是来消除这个的）。
- 浮层的**几何**要能被脚本算出来：新增/改动行高或内边距时，同步改 `menu::item_h` /
  `menu::popup_padding`（公开助手），别让脚本自己抄一遍公式。
- **同一帧可以有多个 z 相同的 `WIN_TOPMOST` 浮层**（下拉里再开子菜单）：`Ui` 帧内的
  `win_ids` / `win_origins` **按 z 键**，相同 z 只会留下最后一个 ⇒ 诊断**必须按窗口 ID**
  （[`Ui::debug_dump`](crate::Ui::debug_dump) 走 `frame_state.window_ids_seen` +
  `UiState::window_origins`）。渲染不受影响：采集时减、提交时加用的是同一个 z 键值。
  跨帧状态槽位同理：**子菜单状态挂行自己（`WidgetState`），不要挤 `UiState::combo_open`**。
- **浮层 z 一律走 `Ui::push_overlay_z` / `pop_overlay`**（`ui::overlay_z` = 基址 + 嵌套层数）：
  写死同一个哨兵会让子浮层的**阴影**被父浮层控件盖住（见 §18.15「分层 z」）。
  "是否在浮层上"用区间判定 `ui::is_overlay_z`，不要写 `== WIN_TOPMOST`。
- **主题加字段**：样式结构体都已 `derive(Serialize, Deserialize)` + `serde(default)` ⇒
  加字段（带 `Default`）**不需要**动格式版本；但**改字段名 / 删字段** = 破坏文件兼容 ⇒
  视为格式变更并抬 `THEME_FORMAT_VERSION`。新字段若类型没有 serde（外部 crate 的类型）
  就加**代理模块**（`weight_serde` / `align_serde`）或 `serde(skip)`（不可移植的值）。
- 新增**交互**控件时必须调用 `register_focus(&id_for, rect, FocusKind::X)`（键盘导航 / 焦点描边；`id_for = ui.id_for(id)` 为**绝对 ID**）；需要 Enter/Space 激活的控件用 `key_click(&id_for, kind)` 合成点击。持久状态一律经 `state_mut().widget(&id_for)` 读写（绝对 ID）。
- 绘制命令坐标语义：**相对当前容器 origin 的局部坐标**，容器弹出时统一平移；命中测试用 `abs_base + 局部`。新增容器时务必保持该约定。
- **半透明与元素序**（两条都会静默毁掉画面）：
  - `tess::push_rounded_rect` / `push_rounded_ring` / `push_convex` 的硬体 alpha **就是调用方给的颜色 alpha**（羽化环从它降到 0）——**不要写死 `alpha = 1`**：曾导致所有半透明圆角矩形 / 图标渲染成不透明（取色器 `#6EA8FF0A` 色块实测像素 = 纯色）。
  - 组合控件里"画在自家背景之上"的装饰（手柄 / 箭头 / 分隔线）必须传 `ui.elem_hint()` 作为 `elem`（`push_panel_like` 的 `elem` 参数、`push_draw` 的第 3 个参数）：**元素序小的先画**，写死 `0`/`1` 会被本控件自己的背景或文本框整块盖住（`NumberInput` 的拖拽手柄、`ColorPicker` 的展开箭头都曾因此消失）。用绘制器（`ui.painter()`）时这条是**默认正确**的——原语各自取当时的 `elem_hint()`，要"压住自家已有内容"就**再取一次** painter（见 §18.18）。
  - 相邻矩形拼成的"多段渐变"（如色相条）要用**纯四边形**（`DrawKind::Rect`）而不是多个带羽化的圆角矩形：两侧羽化的 alpha 斜坡会在共享边都降到 0，透出一条缝。
  - 四角渐变在一整块几何里只做**逐三角形线性**插值：颜色场含交叉项（如 SV 平面 `V·lerp(白, 色相, S)`）时会出现折痕 ⇒ 切成网格（`colorpicker::hsv::sv_plane_cells`）。
- **可拖拽容器**（窗口 / 面板）另有一条硬约定：`abs_base` 必须等于本帧实际平移量（`display_pos`），且**交互（命中 / 拖拽基准 / clamp）先于内容录制求解**——命中矩形取**上一帧结算尺寸**（`UiState::window_sizes` / `panel_sizes`，鼠标事件正是针对屏幕上那个矩形产生的）。若像早期实现那样"`abs_base` 用上一帧位置、几何用本帧位置"，拖拽期间一切走 `abs_base` 的绝对空间量（文本 `box_clip`、IME 光标、滑块基准）都会落后一帧（快速拖动时文字被裁 / 点击偏移）。位置求解复用 `ui::resolve_drag`（纯函数，可单测）。
- 网格 cell 缓存（`UiState::grid_cells`）保证跨帧布局稳定；无缓存首帧渐进扩展，次帧起稳定。
- **缓存了 UV / 图集区域的跨帧缓存，键里必须并入 `DynamicAtlas::revision()`**（`rjw_ui` 的窗口顶点缓存、`rjw_ui` 的 win=0 子槽缓存即如此）：`generation()` 只覆盖"重排搬动"，漏掉"逐出 + 空闲槽位被复用"——此时旧 UV 采样到别的字形像素（"陈旧文字 / 背景消失"），而内容签名不变 ⇒ 缓存永不失效。校验区域用 `region_peek()`（**不刷新寿命**，别用 `region()`——那会保活被校验的条目）。

### 18.18 绘制器（`Painter` / `DrawQueue`）：录制状态从 `Ui` 里抽出来

**动机**：绘制命令的"写出"本来散在 `Ui` 的几十个 `push_*` 方法里，每个方法各自读
`self.queue` / `self.seq` / `self.depth` / `self.cur_win` / `self.clip` 五个字段，于是
"画一条命令"这件事**无法脱离 `Ui` 存在**——而 `Ui` 需要字形图集（GPU）才能构造，所以本仓
历史上所有绘制 bug（圆角丢失、背景图在直角面板上消失、拖拽柄被自家背景盖住）都只能靠
示例截图发现，**写不出单测**。

现在录制状态自成一个组件：

```
painter.rs          // 门面：Painter / DrawQueue / Ui::painter / Ui::painter_clipped
painter/queue.rs    // DrawQueue：命令队列 + 播放头（seq / depth / cur_win / clip）
painter/prim.rs     // 原语：solid / border / panel(_elem) / panel_img(_elem) / shadow / rounded_at / icon_at / image_at / gradient_at
painter/text.rs     // text / text_noclip（环境裁剪进 UiDraw.clip，软裁剪进 DrawKind::Text.clip）
painter/debug.rs    // debug_line / debug_rect_outline / … （进 debug_queue）
```

- `Painter` **按值拥有** `DrawQueue`；`Ui` 只有字段 `painter: Painter`（`Ui::painter()` 借出）。
- `painter.rs` **不 `use crate::ui::Ui`**，`DrawQueue` 可 `Default` 独立构造 ⇒ `Painter::new(1.0)`
  就能录命令并 `commands()` 读回（`painter/*` 的单测即如此：`rjw_ui` 第一次能在无字体图集 /
  无 GPU 的情况下断言"画出来的是哪几条命令"）。
- `Ui::push_*` / `*_at` 全部保留为**薄包装**（公开 API 不破坏），实现改成一行
  `self.painter.<原语>(..)`；`PanelCmdCtx` / `push_panel_img_cmds`（面板三层命令的纯函数）从
  `ui.rs` 移到了 `draw.rs`——它只依赖 `UiDraw` / `DrawKind`，绘制器与单测共用。

**两条硬约定（都是踩过的坑）**：
1. **`elem` 默认逐条取 `elem_hint()`**（= 当前 `seq + 1`），与原 `push_*` 逐位一致：同一元素内
   按 `seq` 排序、**图形先于文字**（`group`）。所以"装饰压住自家已有内容"靠的是**重新取一次
   `ui.painter()`**（那时 `elem_hint()` 已经更大），**不是**把 painter 的 elem 冻结成一个值——
   冻结会让后画的图形（拖拽柄）排到自家文字之前，被整块盖住（`NumberInput` 手柄 /
   `ColorPicker` 箭头的历史 bug）。
2. **绘制与 `ui.*` 不能交错**：`ui.painter()` 借 `&mut self`，painter 存活期内 `ui.text_size` /
   `ui.hit_abs` / `ui.*_at` 都借不到 `ui`（egui 那种"随便交错"靠 `Painter` 持 `Context` 克隆 +
   内部可变性；本引擎不用 `Arc<Mutex<_>>` 换它，保持零堆分配 / 无锁）。顺序是「先量 → 画 → 再量」。

**裁剪就在 painter 上**：`Clip` 沙箱 / ScrollView 可视区 / 严格窗口内容裁剪是 `view_at` /
`scroll_at` 写进 `DrawQueue::clip` 的**当前强制层**——沙箱内取到的 painter 的 `clip()` 就是它，
录出的每条命令自带该裁剪（`UiDraw.clip`）⇒ **沙箱里的控件不需要 `painter_clipped`**。
`clipped(..)` / `painter_clipped(..)` 只用于"要一层与当前**不同**的裁剪"：临时更窄（本控件自己
再裁一刀，且不改全局层——改全局层会连带子控件 / 兄弟控件），或传 `None` 主动不裁。

**等价性怎么验**：迁移前后同一场景 `[perf] cmds=` / `wins=` / `cache_hit=` 逐项相同（实测
`cmds=380 wins=7 cache_hit=13 cache_miss=0`，与迁移前的 HEAD 构建逐位一致），外加全部
`--sim-*` 判定行不变、`cargo test -p rjw_ui` 全绿。

### 18.20 环境裁剪：batch scissor（不再切割几何）

**动机**：环境裁剪层（严格窗口内容 / 固定高窗口 / ScrollView 可视区 / Clip 沙箱 /
文本框盒）原本在 `collect_cmds` 里**逐命令与几何求交**（`draw::clipped`）。代价与副作用：

- **圆角被切平**：`RoundedRect` / 圆角环带被切成直角（历史 bug："窗口拖到视口边缘时整个
  边框瞬间变方"就是这条）；
- **投影要特例**：部分可见时"内轮廓"会错位成暗带 ⇒ 只能整块跳过（`fully_visible`）；
- 每命令一次矩形求交 + 渐变重采样（`resample_gradient_local`）。

现在：**几何保持原形，越界像素交给 GPU scissor**（`glScissor` 语义）。

```
UiDraw.clip（绝对屏幕坐标）
  → QuadCollector 分组键（win, elem, group, tex, clip_key=量化 i32 四元组）
  → CachedQuad / SegRun.clip
  → batch_scissor(clip, anchor, tf)   // view.rs：减窗口原点再经批次变换（保守 AABB）
  → UiBatch.clip                       // rjw_ui → 后端的公开字段
  → 桥接：Draw2D::scissor(rect)        // rjw_krusie/runtime/layers/ui_backend.rs
  → Render2D 逐 DrawOp set_scissor_rect（最终 = 命令级 ∩ 画面级 ∩ 目标矩形）
```

- **切段规则**：`(win, tex, clip)` 任一不同即切段（一次 `draw_indexed` 只能一个 scissor）
  ——`gpu_batch::batch_contract_tests::clip_change_splits_the_run` 钉住；
- **空 scissor ⇒ 丢 op**（不是退化成"不裁剪"）：`prepare` 跳组、`scissor_px` 返回 `None`；
- **签名**：`cmd_sig_hash` 现在**哈希 `d.clip`**（量化）——旧实现不哈希它，裁剪变了而命令
  内容相同时窗口顶点缓存会命中旧分组（"裁剪不生效"的陈旧几何）；
- **窗口 FX**：`batch_scissor` 把裁剪区经批次变换映射（纯平移 ⇒ 逐位相等；缩放 ⇒ 跟着缩；
  旋转 ⇒ 保守 AABB，宁可多画一点绝不误裁）；
- **文本软裁剪仍是 CPU**（`DrawKind::Text::clip`）：它是排版/省略语义，不是像素裁剪。

**可观测证据**：

| 通道 | 看什么 |
|---|---|
| `[perf] ... clip_batches=N` | 本帧带 scissor 的批次数（远大于窗口数 ⇒ 裁剪区太碎，每个不同 scissor 都多一次 draw） |
| `--ui-dump` 的 `clip=` | 每窗**最后一批**的 scissor（`None` = 不裁） |
| `--sim-clip` | 严格窗口的 scissor == 它的内容区（`origin` + `size`）且 `clip_batches > 0` |
| 渲染器单测 | `scissor_px`（取整/钳制/全外 ⇒ 跳过）、`rect_intersect`（命令级 ∩ 画面级） |

**裁剪层必须与缓存同空间**（用户实测 bug："被裁窗口的 scissor 和内容不会一起移动"）：
`QuadCollector.cur_clip` / `CachedQuad` / `SegRun.clip` 里存的是**窗口局部**裁剪层
（`d.clip` 已随容器平移成绝对坐标后再减 `anchor_px`）。存绝对值会怎样：① 分组键被
"窗口位置"污染；② 更糟的是 §18.21 的签名去绝对化之后，**拖动窗口会命中缓存** ⇒ 提交时
拿到的裁剪层是**上一帧的绝对矩形** ⇒ scissor 留在旧位置（实测偏一个位移量，`--sim-clip`
的拖动段当场抓住）。改成局部后：`batch_scissor(clip_local, tf)` = 局部裁剪 × 批次变换，
纯平移下就是"裁剪层 + 窗口原点"，窗口怎么动 scissor 就怎么跟。

⚠ **代价**：裁剪区越多，draw call 越多（每个不同 scissor 至少要一段）。演示场景实测
`clip_batches=11`、`segs=39`（1 个严格窗口 + 若干文本框盒 + 滚动可视区），
`cmds/wins/cache_*` 与改动前逐位一致——即"裁剪改道"没有带来额外几何重建。

### 18.21 提交期优化（①–⑤′；⑥ GPU 持久缓冲不做）

> **决策记录**：⑥（GPU 持久缓冲）不做。⑥ 之外的**下一刀已落地**（阶段 9）：**缓存提交
> 计划而非几何碎片**——把"合并 / 切段"从每帧搬到缓存填充时，命中路径变成
> `remove`（move 一个 `Vec`）→ 借用提交 → `insert`，**零顶点拷贝**。`finish` 实测
> **0.68ms → 0.36ms（−47%）**、`clone_us` 恒为 0、`asm` 170µs → 23µs，代价是
> **draw call 40 → 44（+10%）**。理由、数据与取舍见
> [`UI_ARCHITECTURE.md`](UI_ARCHITECTURE.md) §6.2 与 §18.24。

| # | 做法 | 效果（实测） |
|---|---|---|
| ① | **签名去绝对化**：`cmd_sig_hash` 哈希 `rect − anchor` / `clip − anchor`（与"缓存里存的是窗口局部顶点"同口径） | 拖动窗口从"**每帧 1 次整窗 MISS**"（`RJ_CACHE_TRACE`：f22…f61 每帧一条 `MISS win`）变成**稳态 0 MISS**（只剩 f1 冷启动 7 条）。`--sim-drag` 实测 |
| ② | **scratch 复用**：提交期的临时容器（阶段 9 起是 `scratch_units`（提交单元列表）与 `scratch_ordered`（单元内待切段条目））住 `UiState`，每帧 `clear()` 复用 | 去掉每帧 `Vec` / `BTreeSet` 的反复分配（演示 `segs=40`/帧） |
| ③ | **单组段直接 move 几何**：`run.quads == 1` 时 `mem::take` 该条几何，省一次 `append` | 省一次全量拷贝（多条段仍走 `append`）。⚠ 曾一度撤销：当时 `--sim-dropdown` 子菜单面板整块丢失，**误判为它的锅**——真因是同期"裁剪层绝对/局部空间不一致"（见 §18.22 坑 1）。空间修正后重新启用，全部 `--sim-*` 通过 |
| ④ | **签名瘦身**：环境裁剪以量化 `i32` 四元组入签名（4 次整型写入）；文本只哈希**自身软裁剪**（环境层已上移到 batch scissor） | `sig_us` 未见增长（57–61µs，与改动前同档） |
| ⑤ | **`[perf]` 计数**：`UiStats` 增 `clip_batches / seg_count / vert_count / tri_count`，示例 `[perf]` 打印 `clip_batches= segs= verts= tris=`；阶段 9 又加了 **`draw_ops=`**（UI 层 `Render2D::draw_op_count()`，**真实 draw 数**） | `segs` = 批次候选数；**`draw_ops` 才是真的发了几次 draw**（实测两者相等：每个 `mesh_indexed` 带自己的矩阵下标 ⇒ `Render2D` 的动态段合批对 UI 批次不生效）；`verts/tris` = 这一帧提交了多少 |
| ⑤' | **`submit` 拆两笔账**：`UiStats::submit_asm_us`（UI 自己装配：组装/排序/切段/段内拼接）+ `submit_flush_us`（`flush_seg` → `UiBackend::submit`，真实后端在这里把顶点拷进渲染器暂存） | **不拆这两笔就无法判断那 0.3ms 该算谁**。实测 `submit=283µs(asm=166 flush=117)`——上一轮把整块当成"一次拷贝"就是这么来的 |

> 现场看 `[perf]` 稳态（**改前 / 改后各 3 轮交错跑、取中位数**，480 帧/轮；机器状态
> 会漂，跨天比较无效，必须同机交错 A/B）：
>
> | 指标 | 改前 | 改后（§18.23 的 place 维度） |
> |---|---|---|
> | `finish` | 0.50ms | 0.53ms（**+30µs**，见 §18.23 的成本说明） |
> | `clone` | 139.9µs | 150.2µs |
> | `submit` | 256.9µs（asm 145.9 / flush 112.7） | 265.0µs（asm 153.1 / flush 111.9） |
> | `verts / tris / segs / cache_hit` | 23936 / 31887 / 40 / 14.0 | **完全相同** |
>
> 早先单次测得的 `clone=168.9 submit=289.3 finish=570µs` 是**另一天同一机器**的值
> （整机慢 ~30%），不可与上表直接比。
>
> **成本模型、拷贝链（C1–C4）与下一步的结构性方案见
> [`UI_ARCHITECTURE.md`](UI_ARCHITECTURE.md)。**

### 18.22 剔除（culling）：分配 → 被裁掉 → 直接 return

**scissor 不是剔除**。scissor 只省**片元**：完全看不见的控件照样要镶嵌顶点、进段、发
draw。所以两层都要有：

| 层 | 粒度 | 省什么 | 在哪 |
|---|---|---|---|
| **剔除** | 整条命令（或整个控件） | CPU 镶嵌 + 顶点/索引 + 段（draw call） | 分配处（主）+ `collect_cmds`（兜底） |
| **裁剪** | 像素 | 片元 | GPU `set_scissor_rect`（`UiBatch.clip`） |

**主剔除（分配处）**：`allocate_sense*` 返回的 [`Response::culled`](crate::widgets::Response)
（以及 `Ui::culled(rect)`，给只用 `allocate` 的控件）在**强制裁剪层**里判"这个矩形一个
像素都看不见"，控件据此**直接 `return`**：

```rust
let (rect, resp) = ui.allocate_sense(self.id, size, Sense::CLICK);
if resp.culled { return resp; }        // 分配 → 被裁掉 → 直接 return
let p = ui.painter(); p.panel(..); p.text(..);
resp
```

判据是**纯几何**（`rect` 与裁剪层无交集），与鼠标/交互无关；`Ui::culled` 把
`rect + abs_base`（绝对）与裁剪层（绝对）比——**空间必须一致**（见下）。

**兜底剔除（`collect_cmds`）**：命令层再判一次"整条在裁剪层之外 ⇒ 不镶嵌、不入段"，
兜住两种分配处看不见的情况：① 自绘装饰溢出到裁剪层外（本体可见、装饰不可见）；
② 没检查 `culled` 的控件（含第三方）。投影按**外沿**判（否则贴边窗口的投影被误剔）。

**判据因命令类型而异**（三条，别再退化成"一律 `snap_rect(&d.rect)`"）：

| 命令 | 用于判"全外"的矩形 | 为什么 |
|---|---|---|
| `DrawKind::Shadow` | 本体矩形**外沿**（外扩 blur + offset） | 投影天然溢出本体，否则贴边窗口的投影被误剔 |
| `DrawKind::Text` | [`gpu_batch::text_visible_rect(rect, soft_clip)`](crate::gpu_batch) = **软裁剪层的绝对位置** | 文本的 `rect` 是**排版锚点矩形**（宽/高 = 容器盒），内容随滚动平移 ⇒ 拿它判会把"滚过头但可见"的整条文本剔掉（见坑 3） |
| 其余 | `snap_rect(&d.rect)` | 该 rect 就是墨迹 / 染色范围 |

三个**踩过的坑**（都被 sim 当场抓住，写进注释）：

1. **空间**：`d.rect` 在 `collect_cmds` 里已是**绝对**坐标，而缓存键用的 `cur_clip` 是
   **窗口局部**的。拿局部裁剪层去比绝对矩形 = 把内容**整块误剔** —— 实测"严格窗口内容
   全没了 / 子菜单面板不见了"（用户报的"控件消失"）。正确做法：剔除用 **绝对**
   `clip_abs`，进缓存键用 **局部**。
2. **分配处剔除只覆盖"当前已知的强制层"**：窗口结算后才知道的严格裁剪
   （`Placement::Clip` / 固定高）在录制期**还没有** `Ui::clip`，分配处剔不掉它——
   那条路径靠 `collect_cmds` 的兜底剔除（`finish` 里 retro-clip 已把 `d.clip` 补上）。
3. **排版矩形 ≠ 墨迹矩形**（用户报的"单行输入框滚过头文字消失"，`DEBUGGING.md` §8.4）：
   文本命令的 `rect` 宽/高取的是**输入框盒**（`content_w` / `rect.h`），而内容会随
   `text_scroll` / `scroll_y` 平移 ⇒ 滚过 `padding + content_w`（纵向 `+ rect.h`）后
   `rect` **整个滑出盒子** ⇒ 误剔整条文本。修法：文本改用软裁剪层（锚在盒子上）算可见
   窗口，并让命令矩形**名副其实**（单行宽取全文宽、多行高取全文高；观感不变，因为
   `draw_text_quads` 只用 rect 的 x/y）。诊断口径：`[perf] culled_text=` +
   `--sim-text-cull`（实测修前"未滚动 `culled_text=0` → 滚到 1000px `culled_text=1`、
   `verts` 掉 44"，修后两者都不变）。

**实测收益**（演示稳态，`[perf]` 前后对照）：`verts 30186 → 23736（−21%）`、
`tris 41235 → 31621（−23%）`、`segs 39 → 40`（段数不变，因为省掉的是"本来也要合进
同段"的顶点）。全部 14 个 `--sim-*` 判定不变。

### 18.23 win=0 的绘制序：**顶层放置序 `place`**（修两个闪烁）

**症状**（用户报告）：
1. **可拖动玩家名面板闪烁**——把它拖到顶部「FPS / 点击次数」标签上时，标签**从面板里
   透出来**，且随 FPS 文本逐帧变宽而抖动；
2. **ScrollBar 闪烁**——列表滚动时滑块忽隐忽现，只在条目**间隙**里露一条。

**根因（一个）**：绘制序只有 `(win, elem, group, tex, clip)`，而 `elem = 0` 的语义是
"画在**本容器**元素之下"——**只有"每个容器一个排序空间"才成立**。窗口恰好天然满足
（一扇窗 = 一个 `win` 桶），但**所有非窗口内容共享 `win = 0`**：面板的投影/底色
（`panel_impl` 传 `elem = 0`）与 `scroll_at` 的滚动条（当时也传 `0`）于是落到
**整个 win=0 空间的最底**，被**任何**别的 win=0 内容穿透。全仓只有这两处 win=0 装饰用
`elem = 0`，与用户报的两个控件**一一对应**。

**判据（引擎侧实测，`RJ_ORDER_TRACE=<帧号>` 打印实际提交顺序）**：

```
# 修前（frame 100，面板已被拖到 (30,18) 压在标签上）
i=0  elem=0  verts=733  v0=(30,30)   ← 面板投影
i=1  elem=0  verts=144  v0=(30,30)   ← 面板底色      ⇒ 先画 = 被下面两条盖住
i=3  elem=1  verts=20   v0=(25,23)   ← FPS 标签（后画 = 在上）
i=4  elem=2  verts=24   v0=(24,54)   ← 点击次数
# 修后（frame 150）
i=0  place=0 elem=1 verts=28  v0=(25,23)  ← 标签先画
i=1  place=0 elem=2 verts=24  v0=(24,54)
i=14 place=2 elem=0 verts=877 v0=(30,30)  ← **面板（投影+底色）后画 ⇒ 盖住标签**
i=16 place=2 elem=15 v0=(44,35)           ← 面板自己的 label（仍在自身底色之上）
```

`scroll_at` 同理：修前滚动条 `elem=0` 排在列表项（`elem ≥ 1`）**之前**，修后排在
**之后**（`--sim-zorder` 的帧 150 trace：`place=1 elem=82 verts=82`，轨道 + 滑块
合成一条，且在全部条目之后）。

**修法**：

| 改动 | 说明 |
|---|---|
| **排序键 = `(win, place, elem, group, tex, clip)`** | `place` = "到这条命令为止已经开始了几个顶层放置"（`z0_place_for_seq`，纯函数）。win=0 从此有了自己的排序空间；窗口内 `place` 恒 0（`win` 已隔离） |
| `z0_ranges: Vec<u32>` 取代 `(组号, 起, 止)` | 记的是每个放置**第一条命令的 `seq`**（`begin_top_placement` 传 `queue.seq + 1`）。删掉 `end_top_placement` 与 `cur_z0_group`（放置之间不可能嵌套 ⇒ 结束位置无意义）。缓存槽键 = `(段号, place)` |
| **不再存"陈旧 place"** | `place` 逐帧重算：放置增删 ⇒ 键变 ⇒ 直接 miss 重建。若把它当缓存字段，就会"命中一个序已过期的条目"⇒ 绘制序错一帧（又是一种闪烁） |
| `scrollbar` 的 `elem`：`0` → `elem_hint()` | 滚动条不是容器装饰，它要**盖在自家内容之上**（与它注册的控件级遮挡语义一致：`scrollbar` 文档早就写着"画在内容之上"，代码却传了 `0`） |

**两个踩过的坑**（都由 trace 当场定位）：

1. **放置边界 off-by-one**：`begin_top_placement` 最初记 `queue.seq`（容器入口时的
   "已用序号"）。而 `scroll_at` 的滚动条紧挨着后面的 `flex_at` 容器——滚动条第二条命令的
   `seq` 恰好等于新放置的起点 ⇒ 被 `≤` 命中进**新**放置，同一个滚动条被拆成
   `place=1` / `place=2`（滑块又被列表项盖住）。改成记 `seq + 1`（本放置第一条命令的序号）。
2. **序号播放头落后**：`scrollbar` 是少数**直接写队列**的入口，旧写法
   `let seq = next_seq(); push(seq); push(seq + 1)` 只取了**一次**号 ⇒ 下一条命令拿到
   **重复序号**，而 `place` 是按 `seq` 归一化的 ⇒ 重复序号跨放置边界时又把同一容器拆开。
   改为每条命令各取一次号，并在 `begin_top_placement` 加 `debug_assert`（播放头必须
   ≥ 已入队命令的 `seq`）把这一类问题变成显式失败。

**成本（同机交错 A/B 3 轮取中位数，480 帧/轮）**：`finish 0.50ms → 0.53ms`（**+30µs，
+6%**），`clone +10µs`；`verts / tris / segs / cache_hit` **逐位不变**。多出来的开销来自
win=0 的缓存槽按 `place` 切得更细（每条命中条目的 per-entry 开销），属于"正确性成本"。

**验证**：新增 `--sim-zorder`（把面板拖到标签上 + 用滚动条翻页后再点同一屏幕点 ⇒
选中项号必须变大，两条**硬断言**，防止现场是空跑的）；`RJ_ORDER_TRACE=<帧号>|all|slot`
三个档位（`slot` 打每个放置槽的 `seq` 区间，就是定位上面两个坑的工具）；7 条纯函数单测
（放置序语义 + 提交排序键 + **反例**：把 `place` 从键里去掉，断言必须失败）；全部
16 个 `--sim-*` 通过。

### 18.24 提交期优化落地：**缓存"提交计划"而不是"几何碎片"**

**一句话**：把"合并 / 切段"从**每帧**搬到**缓存填充时**——缓存的东西从"碎片几何"变成
"已经按提交序合并好的批次计划"（`BatchPlan`，粒度 = 一个**提交单元** `(win, place)`）。

**流程**（`Ui::finish`）：

```text
collect_units  每个单元：
                 命中 ⇒ 把计划从缓存 remove 出来（move 一个 Vec，零顶点拷贝）
                 未命中 ⇒ collect_cmds 镶嵌 + merge_plans 合并（这一帧只做一次）
units.sort     (win, place)：单元级排序（~15 个；单元内顺序已烘在计划里）
flush_seg(&plan) × N   计划**借用**提交（UiBatch 的顶点/索引改成 &[..]）
回填          命中与未命中都把"本帧计划 + 本帧签名"move 回缓存
```

**实测（同机交错 A/B 3 轮 × 480 帧，取中位数）**：

| 指标 | 改前 | 改后 |
|---|---|---|
| `finish` | 0.68ms | **0.36ms（−47%）** |
| `clone_us` | 193.6µs | **0.0µs**（恒为 0，回归哨兵） |
| `submit_asm_us` | 169.6µs | **22.8µs** |
| `segs` / `draw_ops`（真实 draw） | 40 / 40 | **44 / 44（+10%）** |
| `verts / tris` | 23936 / 31887 | **逐位不变** |

**为什么 draw call 会涨**（一度写错，实测纠正）：`Render2D` 的动态段合批条件含
`dyn_seg_mat`，而每个 `mesh_indexed(..)` 都带**自己的矩阵下标** ⇒ **UI 批次之间永远
不合并**（`draw_op_count() == segs` 坐实）。所以"批次不再跨单元合并"= 真的多 4 次 draw。
取舍：**+10% draw call 换掉每帧两次全量顶点 memcpy**（在 `frame ≈ 1.8ms / ui ≈ 0.5ms`
的账上划算）。将来 UI 批次多到 draw 成为瓶颈时，可在**提交期**只把"相邻且同
`(tex, clip, 变换)`"的少数几对计划合并（会重新引入一次拷贝，但只针对那几对）。

**公共 API 变化**：`UiBatch` 的 `vertices` / `indices` 改成**借用切片**（`UiBatch<'a>`），
`UiBackend::submit(&mut self, UiBatch<'_>)`；`RecordingBackend` 改存**拥有**顶点的
`RecordedBatch`。`docs/API_REFERENCE.md` §5.6 已同步。

**不变量**：命中与未命中**都提交本帧的每个单元**——而且这次是**结构性**成立的（数据是
`remove` 出来的所有权值，`z`→`id` 只解一次），不是靠"克隆一份兜底"；历史那类"整窗不提交
一帧"的竞态在结构上不存在。

**验证**：16 个 `--sim-*` 全通过（44 条 `[OK]`）；`cargo test --workspace` 全绿；
`verts/tris` 与改前逐位一致（几何没变，只是搬运次数变了）。

### 18.19 Widget 协议：**尺寸在 `ui()` 里就地申请**（v0.3）

**动机**：旧协议要写两个方法（`fn size(&self, ui)` + `fn ui(self, ui, rect)`），于是
"尺寸怎么算"与"怎么画"被拆到两处、必须手工保持一致；而控件内部那一整套交互样板
（`id_for` → `hit_abs` → `mouse_left` → `register_focus` → `claim_press` →
`update_interact` → `update_drag`）在 `NumberInput` / `Dropdown` / 示例探针里抄了七八遍，
借用顺序还容易写错。现在：

```rust
pub trait Widget {
    fn ui(self, ui: &mut Ui) -> Response;      // 唯一的必写方法
}
```

`ui()` 里三步：**先量 → 申请 → 画**：

| 步骤 | API | 注意 |
|---|---|---|
| 量 | `ui.text_size(_wrap)` / `ui.avail_w()` / 主题常量 | **必须在申请之前**（申请会推进容器光标，`avail_w()` 随后变成"下一项"的约束） |
| 申请 | `allocate` / `allocate_mode(size, Expansion)` / `allocate_at` / `allocate_sense(_mode/_at)` | 尺寸是**物理像素 `Vec2`**（刻意不收 `Into<Size<Vec2>>`：`text_size` 与主题常量已是物理像素，再换算一次会 ×DPI） |
| 画 | `ui.painter()`（`panel` / `solid` / `text` / `rounded_at` / …） | 一个绘制块一个 painter（见 §18.18） |
| 交互 | `Sense`（`hover/click/drag/focus`）+ `Ui::interact`（`allocate_sense*` 已含） | `drag` = 按下即 `claim_press()`；`Response.pressed` = **持续按住**、`clicked` = 本帧点击 |

- **膨胀模式**从 trait 钩子变成**申请方式**（`Expansion::{UnlimitedExpansion, LimitedInParent,
  DisableAutoExpansion}`）；**min/max** 用 `apply_constraints(desired, c)` 自己应用
  （`SizeConstraints` 仍是公开纯函数）；
- `Ui::add` / `add_at` / `UiAdd::add` **签名不变**（调用点零改动）：`add = w.ui(self)`，
  `add_at` = 打一个**一次性放置覆盖**（控件的第一次申请消费它）；
- `Ui::widget_size` 删除；`Response` 增 `rect`（最终矩形）——`UiAdd::label` 靠它返回尺寸。

**踩到的坑（都写进注释/文档了）**：

1. `Vec2 → Size::Logical` 的隐式换算：首次实现让 `allocate` 收 `impl Into<Size<Vec2>>`，
   于是 `scale = 1.5` 下**每个控件又乘了一次 DPI**（所有窗口尺寸 +50%、标题栏按钮点空、
   下拉面板下移 20px）。`--sim-chrome` / `--sim-dropdown` 当场抓住；现在签名只收 `Vec2`
   并注明"物理像素"。**这条坑现在有纪律兜着**：`From<f32>` / `From<Vec2>` ⇒ `Logical` 只
   对**调用点**生效，API **实现体内部必须显式**用 `Logical` / `Physical`（正典见 `Size` 的
   「单位纪律」rustdoc / `docs/UI_ARCHITECTURE.md` §5.0）——"再乘一次 DPI"正是该纪律第 2 条
   （构造必须指名单位）要防的事。
2. `Divider` 曾被我改成 `DisableAutoExpansion`（"装饰件不撑大父级"听起来对，但旧代码是
   默认 Expand）：固定宽窗口的高度少掉分隔线那一块 ⇒ 窗口尺寸整块变掉。**行为一致性优先于
   命名直觉**。
3. `Response.pressed` 必须是**持续按住**（旧 `ButtonState::pressed` / `ws.pressed` 的语义）：
   读"本帧边沿"会让按住期间的三态配色退回悬停色。
4. 交错交互与绘制的控件（`Segmented` 每段命中、`NumberInput` 手柄拖拽）要**先收完交互再画**
   （painter 借 `&mut Ui`，不能边画边命中）；`Segmented` 据此重排成"逐段申请+收交互 → 一个
   painter 画完"。
