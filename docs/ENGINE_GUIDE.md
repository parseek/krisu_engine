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
     → f.submit(&mut cam, Clear::color(..))    ← 一个画面 = 一次 submit = 一个 pass
     → f.ui(theme, |ui| ..) / f.text(|t| ..)   ← 可选：overlay 层
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
| UI 固定最顶层 | 走 `f.ui(theme, \|ui\| ..)`（运行时已把 UI 层设为 `SortMode::None` + 默认 base layer 1e7） |
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

**推荐：走运行时（`f.ui` 已替你管 `UiState` / `Theme` / DPI / UI 层 Render2D）**

```rust
fn update(&mut self, ctx: &mut Ctx) {
    let Some(mut f) = ctx.frame() else { return };
    f.draw().sprite(...);                       // 世界层
    f.ui(Theme::dark(), |ui| {
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
    });   // 关闭即提交（UI 层恒在世界层之后）
    f.submit(&mut self.cam, Clear::color(Color::rgb(0.05, 0.05, 0.08)));
}
```

**低层/逃逸舱口**：自己持有 `UiState` 并手动开帧（需要自定义 `base_layer` / `scale_factor` / 复用别的 `Text` 时）：

```rust
let mut ui = Ui::begin(window, &mut text, &mut ui_state)
    .capture(ctx.mouse(), ctx.keys())          // 输入快照（&MouseInput, &KeyboardInput）
    .theme(Theme::dark())
    .base_layer(1.0e7)                          // 默认 1e7
    .scale_factor(ctx.scale_factor().unwrap_or(1.0))  // 控件坐标/字号按逻辑像素
    .build();
// ... 录制 ...
ui.finish(r2d);      // 免全量排序提交（视口/渲染器此时才需要；UI 无需相机）
```

### 18.3 ⚠️ 易混淆点

- **坐标一律屏幕逻辑像素**（左上角原点、Y+ 向下）：调用 `.scale_factor(ctx.scale_factor().unwrap_or(1.0))` 后，所有控件坐标 / 字号按逻辑像素使用，内部自动换算物理像素绘制与命中；不设置则 scale = 1.0（与物理像素一致）。与引擎世界坐标（中心原点）不同；内部经相机屏幕固定变换绘制，旋转/缩放相机下依然 1:1。
- **顶层 pack 直接可用**：`build()` 内建**根容器**（PackSide::Top，可用宽 = 视口物理宽）——`finish` 前任何位置调 `label` / `button` / `slider` 等 pack 控件，顶层即自顶向下流式堆叠；绝对定位仍用 `*_at(pos, ...)`（含 `label_at` / `panel_at` / `pack_at` / `grid_at`）。容器内用无位置形式（`p.button(...)` 占光标）。容器内嵌套容器用 `*_at(offset)`（相对当前容器内容原点），**不占光标**——v1 不支持容器内"光标嵌套"。
- **交互控件必须有稳定 ID 字符串**（按钮 / 滑块 / 勾选 / 单选 / 输入框）；ID 变化 = 状态丢失。`UiState::reset()` 清空全部状态。
- **ID 命名空间**（[`IdRelative`] / [`IdAbsolute`] / [`Ui::id_for`]）：窗口 / 滚动容器 / grid / 可拖拽面板 / 下拉框是**命名空间边界**——进入自动压栈、退出自动弹栈（`with_id` 闭包作用域保证配对），其内控件的**绝对 ID** 自动带容器前缀（如 `"chishi/btn"`）。控件公开 API 仍传**相对名字**（`&str`），内部自动解析；状态键 / 焦点 / 单选组值 / 窗口 id 一律用**绝对 ID**。`ui.id_for(id_relative)` 从名字生成绝对 ID（顶层零拷贝）。类型层面杜绝双重前缀与相对/绝对混用（详见 `crate::id`）。
- **闭包内不可借用已被 `ui` 借用的字段**（如 `self.ui_state`）；需要重置等操作时用局部标记，`ui.finish()` 后统一处理（见示例）。
- **单选**的选中状态完全存于 `UiState.radio_groups`（`group → 控件绝对 ID`），应用只读 `checked()`；初始选中用 `state.radio_groups.insert("组名", IdAbsolute::from("id"))`（顶层无前缀 = 原样；窗口内单选值自动带窗口前缀，与应用无关）。
- **文本输入**：普通字符走 `ctx.keys().chars()`（含 Shift 组合，控制字符已过滤）；**中文输入法（IME）已支持**——`rjw_main` 建窗时自动 `set_ime_allowed(true)`，`rjw_keyboard` 收集上屏文本（`ime_commits()`）与组合候选（`ime_preedit()`，输入框以灰色绘制在光标后），Enter 确认上屏。
- **可拖拽面板 / 窗口**：`drag_panel_at(id, pos, |p| ...)` 按住面板拖动；`ui.window(id).pos(..).show(|w| ...)` 是**可重叠窗口**——点击即**置顶**（焦点 z-order，`UiState.window_z`），位置持久于 `UiState.panel_pos`；拖动期间**抑制内部子控件交互**。拖拽位置按**物理像素粒度**跟随（1px 跟手，不受 DPI 逻辑量化影响）。
- **窗口重叠点击裁决**：重叠区域点击**只让最上层窗口**获得拖拽与置顶（`Ui::finish` 内部 `resolve_win_press`）——不会同时拖动两个窗口。
- **QuadVertices 渲染（不使用 Sprite）**：全部图元（背景 / 控件背景 / 文字）转为**四边形顶点**（`Render2D::quads`，每四顶点一组 **TL,TR,BL,BR**，固定索引 `[0,1,3, 3,2,0]`），按 **(窗口, 元素序, 图形/文字组, 纹理)** 分组提交。**UI 自行管理绘制顺序**——**UI 的 Render2D 必须 `set_sort_mode(SortMode::None)`**（完全按提交顺序绘制）：`finish` 按 `(win 升序, 元素序（控件录制序）, 元素内 图形→文字, 纹理 uid)` 提交（**免全量排序**：win + depth 分桶、桶内保持录制序，语义与 `sort_by_key((win, depth, elem, group, seq))` 完全等价）——窗口间层级由提交顺序保证（`layer = base + z*1.0` 仅作兜底），**窗口内"每个控件 背景→文字 依次绘制"（后录控件完整覆盖先录控件）**，不随纹理 uid / HashMap 顺序抖动。⚠ 不可按 `(win, 图形组, 文字组)` 提交：那会把所有背景排到所有文字之前，后录控件的背景会被先录控件的文字盖住（重叠层级错误）。⚠ **不要用 `set_sort_mode(SortMode::LayerAndStates)`（`SortMode::LayerAndStates`）**：它会在同一 layer 内按 `(rstates, texture_uid)` 重排，字形图集页先于程序化纹理页（圆角/渐变）注册 → 重排后圆角/渐变图形排在文字之后绘制、**盖住文字**（曾因此踩坑：圆角按钮文字消失、渐变状态栏盖住标签）。`set_sort_mode(SortMode::LayerOnly)`（`LayerOnly`，稳定排序）同层保持提交顺序，可接受。
- **白纹理合批（单窗口一次 DrawCall）**：WHITE 基础纹理（1×1，`clamp_margin`）预置进**字形图集页**（[`rjw_text::Text::white_region`]；`DynamicAtlas::white` 为与 key 无关的内置槽位，参与 compact 重排）——实心填充（Solid / 边框 / 光标 / 调试叠加）与字形**同页同纹理**：按控件级顺序提交（背景+文字相邻）且同纹理，Render2D 合批为**单个 draw call**，省去图形↔文字的纹理状态切换。字形本体保持 `InsertOpts::no_clamp()`（避免 clamp margin 挤压）。
- **窗口 transform**：四边形顶点为**相对窗口原点的局部像素**，提交时经 `screen_fixed_tf(窗口原点)` 变换到世界——**移动窗口只改变换、顶点不变**（也支持将窗口嵌入游戏场景，给任意世界变换）。`quads` / `mesh(..).transform(..)` 均支持 `Transform2D`（`IDENTITY` = 顶点即世界坐标）。
- **窗口顶点缓存**：窗口内容不变时，四边形顶点**跨帧缓存**（`UiState.window_quads` 按**内容签名**命中）——静态窗口每帧零字形收集/重建；hover 变色、文字编辑等任何内容变化都会使签名变化而自动重建。**移动窗口不影响缓存**（顶点是局部的，transform 每帧用当前原点）。
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
  （点击穿透拦截计数——`> 0` 说明鼠标下有叠放、背后控件被正确抑制）。

注意：`occluded_hits` / `last_press_window` 是跨帧持久数据，须在 `Ui::begin` **之前**读取
（begin 会借用 `ui_state`），上一帧 finish 写入、本帧显示（见 `examples/eg260818UI`
右上角诊断面板——把窗口 A/B/背包叠在同一处点击即可看到解析结果）。

**点击穿透（窗口遮挡）**：重叠区域**只有鼠标下最上层窗口**的控件响应——`hit_abs`
（所有控件共用）与窗口/面板自身的拖拽命中都过 `window_occluded(z, mouse, window_rects)`
闸门（`z=0` 的非窗口内容被任意窗口遮挡）；窗口矩形跨帧缓存于 `UiState.window_rects`
（`ui.window(id)` 录制时更新，`finish` 末尾只保留本帧录制过的窗口，销毁/置顶换 z 的旧条目
自动清理）。已知边界：窗口**首次出现的那一帧**矩形尚不可知（跨帧缓存盲区），下一帧起
严格生效——置顶方向从第一帧就正确（`win_press_top` 只保留最上层按下窗口）。

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
  `alpha = 1`，沿法线外扩 1 **物理像素**得到外环 `alpha = 0`，带状三角形插出羽化边缘。
  `sprite.wgsl` / `InstanceData` 零改动，也不需要第二条管线。
- **单位弧表 + 步长抽样**（而非每半径一张纹理 / 一张表）：32 点的单位四分之一圆弧表
  服务所有半径；半径 → 目标弧距 2px → `stride`（2 的幂）→ `segs = 32 / stride`。
  `N_FINE` 是 2 的幂 ⇒ `segs × stride` 恒等于 `N_FINE`。
- **羽化环**用更粗的段数（`FEATHER_SEGS_MAX = 4`，整除 `segs`）⇒ 环上每点精确落在
  硬体轮廓的某点上；这是顶点数的主要旋钮（~136 顶点/圆角矩形）。
- **索引内联、不缓存、不共用**：索引与顶点同源（同一次抽样循环），分开算一旦不一致
  会产生错乱三角形且**不会 panic**。也**不用静态网格**：静态网格把几何冻结在 GPU
  缓冲，与立即模式每帧重录冲突；羽化带是 1 物理像素、不可被实例缩放。
- **圆角边框 = 环带**（`tess::push_rounded_ring`）：外轮廓与内轮廓之间的一圈带子，
  内半径按 `max(0, r_outer - width)`（与 CSS `border-radius` 同规则）。比"外圈实心 +
  内圈实心"少一次抗锯齿边缘混合。
- 半径**不取整**（镶嵌器接受任意小数半径），高 DPI 下不再有 `radius × scale` 的
  取整误差。
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

### 18.9 布局增强（换行 / min-max / flex）

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

### 18.10 文本输入增强（单行 / 多行 / IME / 剪贴板）

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
- **行距**：行高 = 字号 × `TEXT_AREA_LINE_SPACING`（1.2）——排版缓冲（`line_mult`）、
  光标 y、高亮、内容测量（`measure_buffer`）全部一致。

**控件自持排版缓冲**：输入框 / TextArea 的 `Arc<Buffer>` 缓存在
`WidgetState::text_buf`（key 含文本/字号/字体/换行宽/行距/版本）——文本频繁变化的
输入框**不写** `UiState::text_buffers` 全局缓存（不污染静态标签缓存）；`DrawKind::Text`
带 `buf` 字段（不进窗口内容签名——排版由 text/size/family 决定）。

**IME 组合候选浮动提示框**：preedit 非空时在输入框**下方**画浮动小框（底色 + 边框 +
灰色候选，自动宽度），不再占行内；系统候选框 `set_ime_cursor_area` 跟随光标
（含水平 / 垂直滚动偏移，物理像素）。

**维护约定**：文本编辑的纯逻辑（行/列换算、选择、滚动、byte/char）在 `edit.rs`
（无 GPU，可单测）；新增编辑控件时复用 `edit::*` 与 `clipboard_get/set`，并在按下
响应中置位 `press_claimed`。

### 18.11 维护约定（对 AI）

- 布局 / 命中 / 状态机是**纯逻辑**（`layout.rs` / `hit.rs` / `state.rs` / `focus.rs`），改动后跑 `cargo test -p rjw_ui`（无 GPU 依赖）。
- 新增控件 = 在 `ui.rs` 加 `Ui::xxx_at` 实现 + 在 `ui::UiAdd` trait 里加便捷方法默认实现（Panel / Pack / Grid 等全部容器自动获得，无需改宏）。
- 新增**交互**控件时必须调用 `register_focus(&id_for, rect, FocusKind::X)`（键盘导航 / 焦点描边；`id_for = ui.id_for(id)` 为**绝对 ID**）；需要 Enter/Space 激活的控件用 `key_click(&id_for, kind)` 合成点击。持久状态一律经 `state_mut().widget(&id_for)` 读写（绝对 ID）。
- 绘制命令坐标语义：**相对当前容器 origin 的局部坐标**，容器弹出时统一平移；命中测试用 `abs_base + 局部`。新增容器时务必保持该约定。
- 网格 cell 缓存（`UiState::grid_cells`）保证跨帧布局稳定；无缓存首帧渐进扩展，次帧起稳定。
