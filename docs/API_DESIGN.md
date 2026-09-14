# krusie API 设计契约（API_DESIGN）

> **地位**：本文件是 krusie 引擎公有 API 的**唯一契约**。命名、参数上限、责任划分、简并关系一律以此为准；
> 代码、示例、文档与之冲突时，以本文件为准并修代码。
>
> 目标（用户要求）：**简洁优雅的调用方式，避免过多参数；清晰的功能与责任，避免调用混乱。**
>
> 版本：0.3.0（破坏性重设计，不保留旧名 shim）。
> 计划与阶段划分见本次重设计的实施计划（P0–P4）。

---

## 1. 分层

依赖方向自上而下，禁止反向引用。

```
L0  rjw_krusie::runtime   应用骨架：Engine / Gfx / Ctx / Frame / WindowId / FrameSource + 可选层
L1  rjw_krusie::prelude   happy path：一行导入（零命名冲突）
L2  子系统门面            Render2D / Text / Ui / TileMap / DynamicAtlas（各一条主链）
L3  机制与逃生口          RStates / Draw2D / SortPolicy / registries / escape（不进 prelude）
```

| 层 | 谁用 | 稳定性 |
|---|---|---|
| L0 | 所有应用（`impl App` + `run`） | 稳定，破坏性变更需走本文件修订 |
| L1 | 所有应用（`use rjw_krusie::prelude::*;`） | 稳定；`prelude` 只增不减 |
| L2 | 需要独立使用某子系统、或写库的人 | 稳定 |
| L3 | 写自定义管线 / 后处理 / 扩展的人 | 可用但不受「≤2 参 / 无 bool」约束 |

**每层只暴露该层的概念**：L0 不暴露 wgpu/winit 类型；L1 不暴露 `Draw2D`/`SortKey`/registry 等机制类型；
L2 的构造器只收 `&Gpu`，不收 `device/queue/layout` 三件套。

---

## 2. 规则（13 条）

| # | 规则 | 说明与反例 |
|---|---|---|
| R1 | **动作入口 ≤2 参**（不含 `&mut self`） | 超出的必须收成一个参数对象。反例：旧 `Text::draw_label_ex`（12 参）、`DynamicAtlas::new`（5 参）、`TextureWrapped::from_rgba8`（6 参）。**数据类型的构造器**（`Quad::new(tl,tr,bl,br)`、`Rect::new(x,y,w,h)`、`Color::rgba(r,g,b,a)`）不受此限 |
| R2 | **禁止裸 bool 参数** | 用枚举（`Vsync::On/Off`、`Background::…`、`Resize::…`、`Placement::…`）或具名方法（`.no_cull()`、`.without_debug_layout()`）。反例：`begin_pass(clear, need_depth_stencil: bool)`、`.depth(bool)`、`topmost(bool)`、`clamp_margin: bool` |
| R3 | **禁止把 `Option` 当模式开关** | 用枚举变体或具名方法。反例：`ClearConfig{depth: Option<f32>}`（`None`=Load）、`set_sorter(Option<Box<dyn>>)`、`set_cull_camera(Option<&Camera2D>)`、`show_handle: bool` 这类「有无」语义 |
| R4 | **一个概念一条入口** | `_with(闭包)` 只作流式变体；消灭同名反向入口（旧 `RenderFrame::record` vs `Render2D::record`）。反例：8 种 `window_at*`、8 种图集 insert、10 条文本绘制路径 |
| R5 | **构造器不暴露低层句柄** | 一律收 `&Gpu`（device+queue+texture_layout 的能力对象）。反例：`Text::new(device, queue, layout)`、`DynamicAtlas::new(device, queue, layout, …)`、`ProcTextures::white(device, queue, layout)` |
| R6 | **名字即职责** | 录制 = `draw`/`record`；提交 = `submit`；呈现 = `present`。反例：`Render2D::record()` 其实「开 pass + 编码 + 清队列」；`DynamicAtlas::get(&mut self)` 名为查询实为改寿命；`get_wheel_line_delta()` 返回像素值 |
| R7 | **顺序由类型表达**，不靠注释 | `PassBuilder` 收集 recorder 后统一推导深度附件需求；相机姿态只在 `Frame::submit` 读取（不做姿态快照，避免「先改姿态还是先设相机」的隐式契约） |
| R8 | **单位进类型或名字** | `Metric<T>`（Logical/Physical）、`pos_px`、`region`（屏幕像素矩形）、`transform.scale`（世界单位/像素） |
| R9 | **prelude 零冲突** | 冲突类型改名（`ui::Window`→`UiWindow`、`ui::Label`→`UiLabel`），不做 `as`。winit 类型不进 prelude |
| R10 | **公开面 = 被消费过的面** | 零调用的 `pub` 降级为 `pub(crate)` 或 `#[doc(hidden)]`（清单见 §10） |
| R11 | **状态糖保留，但收对象/枚举** | 构建器糖 ≡ 对 `RStates` 的字段级修改；`.states(r)` 整体覆盖；后写覆盖前写。糖不得引入 `bool` |
| R12 | **无帧不执行渲染代码** | 守卫在应用侧：`let Some(mut f) = ctx.frame() else { return };`。无帧时 `Ctx::frame()` 返回 `None`，不得有任何渲染/提交调用；`App::update` 本身照常执行（允许后台模拟） |
| R13 | **简并优先** | 同一概念若已有两个类型或两套语言，必须坍缩为一个（见 §5）。新增 API 前先检查 §5 是否已有对应概念 |

---

## 3. 责任表

| 类型 | 唯一责任 | 不负责 |
|---|---|---|
| `Engine` | 组合根：窗口 + 事件循环 + 帧节拍 + 资源生命周期 | 不录制、不布局 |
| `Gfx` | 长期能力对象：`device`/`queue`/`layout` + 资源工厂（纹理/网格/文本/图集/UI state） | 无帧概念 |
| `Ctx` | **本帧宿主事实**：`dt`/`fps`/输入/窗口/DPI/退出 + `frame()` 取帧 | 不做渲染 |
| `Frame` | **本帧渲染面**：画面矩形 / 绘制 / 文本 / UI / 提交 / 呈现 | 不管窗口与循环 |
| `WindowId` | 窗口身份（`Ctx` 窗口作用域） | 不管渲染 |
| `FrameSource` | 取帧抽象（`Some`/`None`），测试可注入 `Never` | 不管提交 |
| `PassBuilder` | 开 pass：从已排队 recorder 推导深度附件需求 | 不管录制内容 |
| `RenderFrame` | 一帧 GPU 资源：surface 纹理 + encoder + 帧级深度 + present | 不管排序/合批 |
| `Render2D` | 录制 → 排序 → 剔除 → 合批 → 写 pass | 不取帧、不 present、不拥有 GPU、**不含相机** |
| `Camera2D` | **矩形区域 + 2D 变换** + VP 矩阵 + 坐标互转 | 不渲染、不被运行时持有 |
| `Text` | 字体系统 + 字形图集 + 文本链 | 不渲染（`Label::draw` 才提交） |
| `Ui` | 每帧 UI 录制（外观立即 + 状态经 ID 持久） | 不拥有 `UiState`/`Render2D` |
| `DynamicAtlas` | 运行时图集：打包 / 寿命 / 复活 / 句柄 | 不绘制 |
| `TileMap` | 瓦片集合 + chunk 预生成 + 自身的剔除 | 不拥有图集 |

**关键解耦**：「画什么」（世界坐标命令）与「用什么相机看」（`Frame::submit` 的相机）互不相干。

---

## 4. 帧语义

| 情况 | 行为 |
|---|---|
| 取帧成功、应用已 `submit` | 提交 → `present` |
| 取帧成功、应用未 `submit`（或未取帧） | `Frame::drop` / 引擎帧尾用 `AppConfig::clear` 清屏后 `present`（不会黑屏/卡帧） |
| 取帧失败（最小化/遮挡/超时/丢失） | `Ctx::frame()` 返回 `None`；应用 `else { return; }`；无任何渲染调用；`dt`/`fps` 照常推进（受 `DT_MAX` 钳制）；按 `AppConfig::background` 退避（默认 `Throttle(50ms)`），避免 `Poll` 空转 |
| 一帧多次 `submit` | 每次 = 一个画面 = 一个 pass + 一个 VP 槽；重叠画面用 `Clear::color_depth(..)` 清深度 |
| 多窗口（未来） | `Ctx` 属一个 `WindowId`；引擎按窗口各调一次 `update`；`frame_of(WindowId)` 为加法扩展 |

---

## 5. 简并表

「概念 → 现状多个表达 → 坍缩为」。这张表是 R13 的判据，也是重构时的主要抓手。

| 概念 | 坍缩前（多个表达） | 坍缩后（唯一） |
|---|---|---|
| 矩形区域 | `Rect` / `Viewport` / UI `Position`+`Size` / `SpriteRect.mesh_*` / `AtlasRegion.tl_px+wh_px` | `Rect` |
| 2D 变换 | `Transform2D` / `Camera2D` 的位姿字段 / `world_transform()` / `view_matrix()` / `*_components` / `*_by` | `Transform2D` |
| 相机 | `Camera2D` / `Viewport`（identity 相机）/ `ViewCull` trait | `Camera2D { region, transform }` |
| 缩放 | `Camera2D.zoom` 与 `Transform2D.scale` | `transform.scale`（`zoom()` 派生只读视图） |
| 精灵矩形 | `SpriteRect` / `SpriteRectPx`（自带 `tex_wh`）/ 手写 `with_uv_tex` | `SpriteRect`（像素 UV 归并入 `with_uv_px`）+ `AtlasSprite` |
| 纹理句柄 | `TextureWrapped` / `ArcTextureWrapped` / `TextureRegistry` / `TEXTURES` / 手查 `page_uid` | 公开句柄 `ArcTextureWrapped` + `Gfx::textures()`（**注册表每 `RenderContext` 私有**，`TEXTURES`/`MESHES` 全局 static 已删） |
| UI 单位 | `Position<T>` / `Size<T>` / `Metric<T>` / 裸 `Vec2` | `Metric<T>`（`Pos`/`Size` 为别名） |
| UI ID | `IdRelative` / `IdAbsolute` / `IdStack` / `WidgetId` | `Id`（内部栈与绝对键） |
| 文本样式 | `Style` / `TextStyle` / `TextLayout` 三份同款 setter | `TextStyle` |
| 文本测量 | `measure` / `measure_buffer` / `TextLayout::measure` / `TextRender::measure` | `Measure` |
| 字形载荷 | `(&AtlasRegion, Vec2, Vec2)` 与 `(MeasureInfo, LineMeasureInfo, AtlasRegion, Transform2D)` | `Glyph` |
| 调试图元 | `debug_draw::*` 7 自由函数 / `Ui::debug_*` / `draw::DebugShape` | `DebugPainter` |
| 帧与提交 | `RenderFrame` / `PassScope` / `Frame::record` / `Render2D::record` / `render` / `record_to` | `RenderFrame` + `PassBuilder`；应用只见 `Frame::{draw, submit, submit_ui, present}` |
| 渲染状态 | `RStates` + 构建器 12 糖 + `RStates::to_*` 转换（双份语言） | `RStates` + 糖（收对象）；转换转 `pub(crate)` |
| 键状态机 | keyboard / mouse 各写一遍转移逻辑 | `KeyStateMachine` |
| 滚动 | `ScrollDelta` + `to_pixel(Option<f64>)` + 15.0/300.0 两套线因子 | `ScrollDelta` + 单一 `LINE_FACTOR` |
| 碰撞体 | `Collider{Aabb}` / `Body{pos,size}` / `Rect` | `Aabb { rect, transform }` |
| 清屏 | `ClearConfig`(3 `Option`) / `Clear` / `AppConfig::clear` | `Clear` |
| 帧源 | 无（直接 `RenderContext::begin_frame`） | `FrameSource`（`RenderContext` 实现 + `Never`） |

---

## 6. prelude 清单（L1）

`use rjw_krusie::prelude::*;` 之后应能写完整应用，**不需要任何补充 `use`，不需要 `as`**。

| 分类 | 条目 |
|---|---|
| 运行时 | `App` `run` `run_with` `AppConfig` `Ctx` `Frame` `Gfx` `WindowId` `Clear` `Vsync` `Background` |
| 绘制 | `Render2D` `SpriteRect` `Edges` `Layer` `SortMode` `Cull` `CullMode` `RStates` `BlendMode` `FilterMode` `AddressMode` `CompareFunc` `PolygonMode` `FrontFaceWinding` `SamplerDesc` `RasterState` `DepthState` `StencilState` `Quad` |
| 资源 | `ArcTextureWrapped` `Rgba8` `MeshSpec` `MeshId` |
| 数学/相机 | `Camera2D` `Transform2D` `Rect` `Vec2` `Vec3` `Mat4` `Viewport`→**已删** `glam` `vec2` `vec3` |
| 颜色 | `Color` `ColorF64` |
| 输入 | `KeyCode` `MouseButton` `KeyState` `ScrollDelta` |
| 图集 | `DynamicAtlas` `AtlasConfig` `AtlasRegion` `RegionRef` `AtlasSprite` `AtlasStats` |
| 文本 | `Text` `TextStyle` `Label` `TextBuffer` `Align` `Gradient` |
| UI | `Ui` `UiState` `UiAdd` `Theme` `UiWindow` `UiLabel` `Button` `Checkbox` `Slider` `NumberInput` `PackSide` `Anchor` `WindowClamp` `WindowFx` `Level` `Placement` `Resize` |
| 瓦片 | `TileMap` `Tile` |

**不进 prelude**（走命名空间 `rjw_krusie::<模块>::…`，或 `rjw_krusie::escape`）：
`WindowAttributes`/`LogicalSize`/`PhysicalSize`/…（winit）、`RenderFrame`、`PassBuilder`、`PassRecorder`、`RenderContext`、
`RenderConfig`、`Draw2D`/`SpriteBuilder`/…、`SortKey`/`SortPolicy`、`VertexP3U2C4`/`MeshData`（`MESHES`/`TEXTURES` 全局表已删除，注册表经 `Gpu` 取）、
`Device`/`Queue`/`wgpu`、`DrawKind`、`Ctx::escape` 相关、`UiBackend`/`UiBatch`（UI 后端契约）。

命名空间：`main`（winit 适配）· `gpu` · `render2d` · `transform` · `color` · `atlas` · `text` · `ui` · `tilemap` · `collision` · `escape`。
保留原 crate 名别名（`rjw_krusie::rjw_2d_render` == `rjw_krusie::render2d`）。

**crate 内命名空间**（不与上面的 `rjw_krusie::*` 混淆）：
- `rjw_ui::backend`：UI 绘制后端契约（`UiBackend` / `UiBatch` / `UiBatchSource` / `RecordingBackend`）；
- `rjw_ui::text`：**UI 文本模块**（公开）——`TextAlign` / `TextVAlign` / `text_block_offset` /
  `text_cmd` + `edit` 的纯逻辑文本操作 + `rjw_text` 的形状层类型（`Align` / `TextStyle` /
  `VisualLine` / `GradientAxis` …）。自定义控件与自建后端的公开入口。
  条目均为**重导出**（定义仍在 `draw` / `edit` / `ui`）。

**字形图集公开**：`rjw_text::Text::{glyph_cache, glyph_cache_mut, user_texture, white_region}`
——UI 等消费者可把自定义纹理插进字形图集（同页 → 同纹理合批）。`AtlasKey` 两命名空间
（`Glyph` / `Custom`）**必须分清**：消费者只写 `Custom`（推荐走 `user_texture`）。
见 `docs/API_REFERENCE.md` §9.1。

---

## 7. 逃生口清单（L3）

明确命名、不进 prelude、可用但不受 R1/R2/R3 约束。

| 用途 | 入口 |
|---|---|
| 原始编码 / 后处理 | `Frame::escape()` → `Escape::{ encoder, device, queue, pass }`；`Frame::raw()` |
| 自定义绘制注入 | `Render2D::custom(impl CustomDraw)` |
| 自定义排序 | `Render2D::sort_custom(Box<dyn SortPolicy>)` + `SortKey` |
| 离屏 / 外部目标 | `RenderFrame::pass_to(RenderTarget, Clear)` |
| 自定义控件 | `Ui::{child_rect, push_*, hit_abs, register_focus, key_click, claim_press, set_cursor, id_for}` |
| 调试图元 | `Render2D::debug(DebugStyle)` → `DebugPainter` |
| 注册表 | `Gfx::{textures, meshes}`（`&TextureRegistry` / `&MeshRegistry`）与 `Gfx::{texture_registry, mesh_registry}`（`&Arc<..>`，供长生命周期持有者）；绘制期 `Render2D::{textures, meshes, gpu}` |
| 资源 / 低级句柄 | `Render2D::{device, queue, texture_layout, white_texture}`（自建缓冲 / bind group）；`Gpu::{device, queue, texture_layout}` |
| UI 绘制后端 | `rjw_ui::{UiBackend, UiBatch, UiBatchSource, RecordingBackend}`；真实后端 `rjw_krusie::runtime::layers::ui_backend::Render2dUiBackend` |
| 世界坐标调试图元 | `rjw_2d_render::debug_draw`（`DebugPainter`） |

---

## 8. 逐 crate 旧 → 新映射

### 8.1 `rjw_transform`

| 旧 | 新 |
|---|---|
| `Camera2D{position,rotation,zoom,viewport_pos,viewport_size}` | `Camera2D{region: Rect, transform: Transform2D}` + `Deref/DerefMut<Target=Transform2D>` |
| `Camera2D::new(size)` + `set_vp(size,pos)` | `Camera2D::new(region)` / `Camera2D::full(size)` / `set_region` / `region` |
| `Camera2D::move_by` / `walk_xy` / `walk_xplus` / `walk_yplus` | 删除 → `transform.move_by` / `transform.move_local`（经 `Deref`） |
| `view_matrix` / `projection_matrix` 公开 | `view_matrix()` 保留（= `transform.to_matrix()⁻¹`，精确）；`view_transform()`（对象级逆）**删除**——它在「非均匀 scale + 旋转」下与点级逆不一致；改用 `world_to_region_local()`（点级精确逆）或 `view_matrix()` |
| `screen_to_world` / `world_to_screen` | 语义不变，实现单源到 `Transform2D`；任意 pos/rot/非均匀 scale 下互为**精确**逆（点级）；新增 `world_to_region_local()` / `world_to_clip()` |
| `Viewport{pos,size}` + `new/vp_matrix/screen_to_world` | 删除（`Rect` + `Camera2D::new(rect)`） |
| `view_cull::ViewCull` trait | 删除（唯一实现并入 `Camera2D::view_aabb`） |
| `Transform2D::{with_move_by,with_walk_by,with_scale_by,with_rotate_by}` | 删除（`with_pos(pos + d)` 已足够） |
| `Transform2D::transform_components/inverse_transform_components`（分量散列） | 删除 → `compose(&Transform2D)` / `compose_inverse` |
| `Transform2D::with_transform` / `with_inverse_transform` | `compose` / `compose_inverse` |
| `Rect::intersects` 文档与实现不一致 | 修正文档 + 补 `touches()` |
| `Rect` 无元组构造 | 补 `From<(f32,f32,f32,f32)>`、`From<[f32;4]>` |
| 引擎 `dpi` 类型与 winit 同名，prelude 只能排除 | 引擎 `dpi` 类型进 prelude；winit 的留 `main::` |

### 8.2 `rjw_render`

| 旧 | 新 |
|---|---|
| `ClearConfig{color:Option<wgpu::Color>,depth:Option<f32>,stencil:Option<u32>}` | `Clear::{Keep, Color(ColorF64), ColorDepth(ColorF64,f32), Depth(f32), Stencil(u32)}` + `From<Color>` + `uses_depth()` |
| `RenderContext::new(window,&RenderConfig)`（safe + transmute） | `unsafe fn RenderContext::new(..)` |
| `resize(w,h)` | `resize(size: (u32,u32))` |
| `device()` / `queue()` / `surface()` | `gpu() -> &Gpu`；逃生口 `Gpu::{device,queue,layout}` |
| `RenderFrame::record(recorder, clear)` | 删除（与 `Render2D::record` 同名反向） |
| `RenderFrame::begin_pass(&ClearConfig, need_depth_stencil: bool)` | `RenderFrame::pass(clear) -> PassBuilder<'_>` |
| `begin_pass_to(target, clear, bool, depth)` | `pass_to(target: RenderTarget, clear)` |
| `PassScope::record`（need 需预查，否则 panic） | `PassBuilder::record(&mut R) -> Self`（收集后统一推导 need） |
| `PassScope` | 删除；`pass()` → `PassBuilder::escape()` |
| `RenderFrame::encoder()` | `escape_encoder()` |
| `TextureWrapped::from_rgba8(dev,queue,label,data,w,h)` | `Gpu::texture(label, Rgba8)`；`Gpu::mesh(label, MeshSpec) -> MeshId` |
| `MeshData`/`MESHES` 公开 | `MeshId` 公开；`MeshData`/`MESHES` 收 `gpu` 命名空间 |
| `TypedRegistry::{get_ref,get_ref_by_name}` 返回 `dashmap::Ref` | 返回 `Option<Arc<T>>` |
| 深度附件池 key `(w,h,fmt)` | key 加 `device_uid` |
| `has_depth_aspect` / `has_stencil_aspect` 公开 | `pub(crate)` |
| 无取帧注入口 | `trait FrameSource`（`RenderContext` 实现 + `Never`） |

### 8.3 `rjw_2d_render`

| 旧（14 条画一次 / 9 条结束一帧） | 新（11 条 / 3 条） |
|---|---|
| `sprite` / `solid` | 保留 |
| 手写 `TEXTURES.get(page_uid)` + `SpriteRect::with_uv_tex` + `sprite` | `region(AtlasSprite)`（由 `DynamicAtlas::sprite(&handle)` 产出） |
| `quads(&[VertexP3U2C4])` / `quads_with` | 保留（`Quad` 类型**不采纳**：`rjw_ui`/`rjw_tilemap` 直接产出顶点数组，再加 4 点 `Quad` 是第三份表达，违反 R13） |
| `mesh` / `mesh_with` / `mesh_with_cap` / `polygon` / `polygon_uv` / `polygon_with` | `mesh` / `mesh_with` / `polygon` / `polygon_with`；删 `mesh_with_cap`、`polygon_uv`（容量提示随之取消：顶点存储常驻复用、按需增长） |
| `static_mesh(u64, &tex)` | `static_mesh(MeshId, &tex)` |
| `custom(..)` | 保留 |
| `render(ctx,&ClearConfig)` / `record(frame,&ClearConfig)` / `record_to(..)` | `Render2D::render(&mut RenderContext, Clear)` / `Render2D::submit(&mut PassBuilder)`；离屏走 `RenderFrame::pass_to` |
| `set_mvp` / `set_camera` / `set_viewport` / `reset_viewport` / `set_scissor` | `Render2D` 不再持 VP：删 `set_mvp`/`set_camera`/`set_scissor`；保留 `viewport(Rect)` / `reset()` |
| `set_sort_mode` / `set_sorter` | `sort(SortMode)` / `sort_custom(Box<dyn SortPolicy>)` |
| `set_cull` / `set_cull_camera` / `cull()` / `culler_mut()` | `cull(impl Into<Cull>)` / `cull_mode()`；`Cull::from(&Camera2D)` 取代 `set_cull_camera` |
| `set_states` / `reset_states` / 构建器 12 糖（含 `depth(bool)`） | `states(RStates)`（唯一入口）+ 8 条收对象/枚举的糖（§2 R11）：`.blend(BlendMode)` / `.samp(FilterMode, AddressMode)` / `.cull(CullMode)` / `.depth(DepthState)` / `.stencil(StencilState)` / `.blend_state(BlendDesc)` / `.samp_state(SamplerDesc)` / `.raster_state(RasterState)`；`reset()` |
| `.color()`（语义随 kind 变）/ `.pos` / `.model` / `.done()` | `.tint(Color)` / `.at(..)` / `.matrix(Mat4)` / 删除 |
| `Layer` 只 `From<f32>/From<f64>` | 补 `From<i32>/From<u32>` |
| `debug_draw::{draw_line,draw_rect_outline,draw_circle_outline,draw_circle_filled,draw_grid,draw_cross}`（5–7 参） | `Render2D::debug(DebugStyle)` + `DebugPainter::{line,rect,circle,disc,cross,grid}` |
| `SortKey.rstates: Option<RStates>` | **保留 `Option`**（`None` = 该命令未显式设置状态、继承 `Render2D::states()`）。语义是「未设置」这一真实状态，不是模式开关（R3 不适用）；且默认状态在**绘制期**才解析（`unwrap_or(default_states)`），排序期「已解析」不可能正确 |
| `SpriteRectPx` | 删除 → `SpriteRect::with_uv_px(uv_tl_px, uv_wh_px, tex_size)` / `with_uv_tex(.., &tex)` / 链式 `uv_px(..)`；收窄并入 `shrink`（世界）/ `shrink_uv`（归一化） |
| `Index`/`TriIndicies`/`MeshStorage`/`Sprite`/`Mesh`/`StaticMesh`/`Custom`/`DrawKind` | `pub(crate)` 或 `#[doc(hidden)]`；`TriIndicies` → `TriIndices` |

### 8.4 `rjw_text`

| 旧（10 条绘制路径 / 3 份样式） | 新（1 条链 / 1 份样式） |
|---|---|
| `Text::new(dev,queue,layout)` | `Text::new(gfx: &Gpu)` |
| `draw_label`(10 参) / `draw_label_ex`(11 参) / `draw_label_with`(7 参) | `Text::label(text) -> Label` → `.draw(layer)` |
| `draw_text` / `draw_text_clipped` / 两种字形回调形状 | `Label::draw_with(|g: &Glyph| …)` |
| `create_buffer` / `create_buffer_policy` / `create_buffer_wrap` | `Text::label(..).into_buffer(&mut TextBuffer)`；UI 集成走 `Text::buffer(..)` |
| `render_from` / `TextRender` | 删除（`Label` 直接提交） |
| `into_render` / `into_render_with` / `TextLayout` | 删除 |
| `Style` / `TextStyle`（重复 setter 各 14 个） | `TextStyle`（唯一）+ `Text::style_mut()` |
| `Text::measure` / `measure_buffer` / `TextLayout::measure` / `TextRender::measure` | `Measure`（`Label::measure()` / `Text::measure_buffer(&Buffer)`） |
| `origin`（归一化）/ `origin_px` / `offset`（像素）三名混用 | `.anchor(Anchor)` / `.at(pos)` / `.center(pos)` |
| `.cull(bool)`（两处开关） | 默认开启 + `.no_cull()` |
| `RenderDefaults` / `AtlasKey` 公开 | 删除 / `pub(crate)` |
| — | **UI 稳定集成面**（非 happy path）：`buffer` / `glyphs` / `measure_buffer` / `lines` / `white_region` / `user_texture` |

### 8.5 `rjw_atlas`

| 旧 | 新 |
|---|---|
| `new(dev,queue,layout,config,page_size)` | `new(gfx: &Gpu, config: AtlasConfig)`（`page_size` 进 config） |
| `insert`/`insert_dyn`/`insert_permanent`/`insert_ex`/`insert_ex_permanent`/`insert_ex_origin`/`insert_no_clamp`/`insert_white` | `insert(key, Rgba8)` / `insert_with(key, Rgba8, InsertOpts)` / `insert_dynamic(key, size, regen)` / `white()` |
| `clamp_margin: bool`（8 种表达） | `InsertOpts::{origin,no_clamp,permanent}` |
| `get(&mut self)` / `get_or_revive` / `acquire` | `region(key)` / `region_or_revive(key)` / `handle(key)` |
| `RegionRef::resolve(&atlas)` + 手写 `TEXTURES.get` + `SpriteRect::with_uv_tex` | `atlas.sprite(&handle) -> AtlasSprite` → `Render2D::region(sprite)`（一处解析） |
| `end_frame()`（零调用） | `tick()`（引擎每渲染帧调用） |
| `page_count`/`total_free`/`largest_free`/`fragmentation`/`resolve_by_id`/`texture_uid_of` | `stats()` + `generation()` |
| `TextureRegenerator` | `SpriteSource` |
| `TOMLETEntry` / `parse_toml_entries` 公开 | `pub(crate)` |
| `StaticAtlas` + TOML 路径零调用 | 保留核心；TOML 收 feature `toml` 并补单测 |

### 8.6 `rjw_tilemap` / `rjw_collision`

| 旧 | 新 |
|---|---|
| `TileMap::draw(r2d, atlas, base_layer, cull: Option<&dyn Fn>)` | `draw(r2d, &atlas, layer)`（剔除复用 `r2d.cull_mode()`） |
| `solid_rects(&mut self)`（消费 `dirty`，导致丢 tile） | `solid_rects(&self) -> Ref<'_, [Rect]>`（缓存经 `RefCell` 内部可变）；脏标记拆为 **`mesh_dirty`（网格）+ `solid_dirty: Cell<bool>`（solid 缓存）**——chunk AABB 在 `push` 时增量维护，故不需要第三个 `layout_dirty`（避免死状态，R13） |
| `tiles()` 不可变却文档说「直接改 tile 字段」 | 新增 `tiles_mut() -> &mut [Tile]`（**自动置脏**）；`tiles()` 保持只读；删 `mark_dirty()` |
| `transform: Option<Transform2D>` + `Into<Option<..>>` | `transform()` / `set_transform(Transform2D)` |
| `visible_count(Option<&dyn Fn>)` | 删除（与 `draw` 剔除重复） |
| `Tile::whole_region(region,src,mtl,mwh)`（4 参） | `Tile::new(&RegionRef, pos, size)` + `.uv/.tint/.layer/.solid` |
| `Collider{Aabb}` + `collides(a,ta,b,tb)` | `Aabb{rect, transform}` + `Aabb::overlaps(&other)` |
| `move_and_collide(pos,size,vel,dt,obstacles)`（5 参） | `Aabb::slide(delta, obstacles)`（2 参） |

### 8.7 输入（`rjw_keystate` / `rjw_keyboard` / `rjw_mouse` / `rjw_time`）

| 旧 | 新 |
|---|---|
| `KeyState` 无法在用户签名中命名 | `rjw_keyboard::KeyState` / `rjw_mouse::KeyState` 公开 + prelude |
| 状态机在 keyboard/mouse 各写一遍 | `KeyStateMachine`（`rjw_keystate`），两设备共用 |
| `KeyState::{off_edge,set_sudden_up}` 公开 | `pub(crate)` |
| `KeyboardInput::get` | `key(KeyCode)`；`get_keys_iter` → `keys()`；`get_chars` → `chars()`；`get_ime_*` → `ime_*()` |
| `MouseInput::{get,get_mouse_button_state}` 完全重复 | `button(MouseButton)` |
| `get_mouse_position` / `get_mouse_delta` / `get_mouse_wheel_delta` / `get_pixel_wheel` / `get_wheel_line_delta` | `pos_px()` / `motion()` / `wheel()` / `in_window()` / `buttons()` |
| `ScrollDelta::to_pixel(Option<f64>)` + 两套线因子 | `to_pixel()` / `to_line()` + 单一 `LINE_FACTOR` |
| `ScrollDelta::is_pixel/is_line` | 保留 |
| `KeyboardInput::end_frame` / `MouseInput::end_frame` 公开 | `MainContext::next_frame()`；两者 `#[doc(hidden)]` |
| `DeltaTimer::dt() -> &DeltaTime` 后还要 `.get_f32()` | `dt() -> f32` / `dt_f64()` / `fps()`；`DeltaTime` 私有 |
| `DT_MAX` 定义但未使用 | `per_frame` 真正应用 clamp |

### 8.8 `rjw_ui`

| 旧 | 新 |
|---|---|
| `UiInit` + `capture/theme/base_layer/scale_factor/debug_layout/build` | `UiCtx` + `input/theme/base_layer/scale_factor/debug_layout()/without_debug_layout()/begin()` |
| `Ui::begin(window,text,state)` | 保留（唯一低层入口，3 参白名单） |
| `finish(&Viewport, &mut Render2D)` | `finish(&Rect, &mut Render2D)`（+ `SortMode::None` 断言） |
| `window_at` / `window_at_strict` / `window_at_w` / `window_at_strict_w` / `UiAdd::window*`（8） | `ui.window(id).pos(..).width(..).level(Level).placement(Placement).clamp(..).show(f)` + `UiAdd::window(id)`（四个变体已删除；`UiAdd` 侧同样只有 `window(id)` builder） |
| `modal_at` / `modal_at_w` / `Ui::modal`（3） | `ui.modal(id).pos(..).width(..).show(f)`（`modal_at*` 已删除，`modal_impl` 转私有） |
| `button_at` + `button_at_styled` + `UiAdd::button` + `button_at` + `Button` builder（5） | `Button` builder + `UiAdd::button`；删 `*_styled` |
| `checkbox_at_styled` / `slider_at_drag` | 删除（builder 覆盖） |
| `set_next_min` / `set_next_max` | 删除 → `UiAdd::min_size` / `max_size` |
| `WindowBuilder::topmost(bool)` / `strict()` | `level(Level)` / `placement(Placement)` |
| `resizable_text_*_at(.., show_handle: bool)` | `resize(Resize::None/Horizontal/Both)` |
| `debug_layout(bool)` ×2 | `.debug_layout()` / `.without_debug_layout()` |
| `child_rect_exp(expands: bool)` | `child_rect(Child::Fit/Child::Expand)` |
| `proc::gradient_rgba(vertical: bool)` | `GradientAxis` |
| `id: &str` / `group: &str`、两套分隔符（`/` 与 `::`） | **保留现状**（未统一 `impl Into<Id>`：控件签名已用 `&str`，改动面 >60 处入口且无功能收益） |
| `IdRelative`/`IdAbsolute`/`IdStack`/`WidgetId` | **保留两类型**（不合并为单一 `Id`）：`IdRelative`（未解析名字）/ `IdAbsolute`（完整键）在**编译期**阻止"拿原始名字查状态"这类漏前缀 bug（`IdStack::id_for` 只收前者、状态表只收后者）；`WidgetId` 是 `&str`/`u64`/标签三态的 `Into` 适配器。合并成 `Id` 会丢掉这层类型安全（收益 < 代价，R5/R6 优先） |
| `capturing_text()` = `focused.is_some()` | `text_focus() -> Option<TextFocus>`（**真正的文本焦点**：只有文本控件持焦点才为真，按钮/滑块 Tab 焦点不再吞应用快捷键） |
| `mouse_logical()`（返回物理像素，与 `mouse_screen()` 重复） | 删除；保留 `mouse()` |
| `Ui::theme` 公开字段（绕过 `scaled`） | `theme()` / `theme_mut()`（字段转 `pub(crate)`） |
| `Widget::resizable()`（零消费） | 删除（文档改 `resize_handle`） |
| `pub use proc::ProcTextures`（不可达） | 删除根导出（`proc::ProcTextures` 仍在，供 crate 内使用） |
| `layout::Frame` 的 `pub fn`（类型 `pub(crate)`） | 转 `pub(crate)` |
| `focus` 只对 Combo/文本框注册（Button/Checkbox/Radio/Slider 的 `key_click` 是死路径） | 四类控件补 `register_focus` ⇒ Tab/方向键导航与 Enter/Space 激活真正生效 |
| `rjw_ui::draw::GradientAxis` 与 `rjw_text::GradientAxis` 两份同名枚举 | 合并为一份：`pub use rjw_text::GradientAxis`（R13） |
| `proc::gradient_rgba(w, h, vertical: bool, stops)` | `gradient_rgba(w, h, axis: GradientAxis, stops)`；`ProcTextures::gradient(.., axis, ..)` |
| 11 子样式 ×(`dark()`+`scaled`+`with_*`) ≈120 方法 | `Theme::dark()` 预设 + 单内部宏实现 `scaled`；删子样式 `dark()` |
| `draw::TextAlign` / `TextVAlign` 半公开 | 统一 `rjw_text::Align`；`TextVAlign` 补根导出 |
| `WidgetState::{press_panel,press_mouse}` 一结构三义 | 拆分为 `press_rect` / `drag_value` 等具名状态 |

### 8.9 `rjw_main` / `rjw_krusie`

| 旧 | 新 |
|---|---|
| `rjw_main::App`（旧的 winit 适配 trait）/ `MainContext` / `MainHandler` / `run_app` | **删除**（零调用）：事件循环与窗口生命周期由 `rjw_krusie::runtime::Engine`（`ApplicationHandler` 实现）负责；游戏侧 trait 是 `rjw_krusie::App`。`rjw_main` 收敛为「平台底座」：winit / 输入 / 计时重导出 + `PRIMARY_WINDOW_TITLE` |
| `rjw_main` 重导出 winit dpi 类型到 prelude | 仅 `main::` 命名空间 |
| `rjw_main::Host` 改名方案 | **不采纳**：适配器已并入 `rjw_krusie::runtime`，`rjw_main` 不再持有第二份 winit 适配（R13 简并） |
| `rjw_krusie::{prelude, 命名空间}` 纯聚合 | `rjw_krusie::{prelude, runtime, 命名空间, escape}`：聚合 + 模块化运行时 |
| `PRIMARY_WINDOW_TITLE` 入 prelude 场景 | 入 `AppConfig` 默认标题（`main::PRIMARY_WINDOW_TITLE` 保留） |

### 8.10 `rjw_krusie::runtime`（新）

| 模块 | 责任 |
|---|---|
| `config.rs` | `AppConfig`（标题 / 逻辑尺寸 / vsync / clear / background / exit_after_frames） |
| `app.rs` | `trait App` + `run` / `run_with` |
| `engine.rs` | 组合根：窗口 + 事件循环 + 帧节拍 + 资源生命周期 |
| `gfx.rs` | `Gfx`：device/queue/layout + 资源工厂 |
| `ctx.rs` | `Ctx`：本帧宿主事实 + `frame()` / `frame_of()` |
| `frame.rs` | `Frame`：本帧渲染面（`draw`/`draw_ui`/`text`/`ui`/`submit`/`submit_ui`/`present`/`region`） |
| `window.rs` | `WindowId` + 窗口作用域 |
| `frame_source.rs` | `trait FrameSource` + `RenderContext` 实现 + `Never` |
| `pass.rs` | 画面 = pass：VP 槽分配、清队列 |
| `layers/text.rs` | feature `text`：`TextCtx` + 缓冲池 |
| `layers/ui.rs` | feature `ui`：`UiState` + `Theme` + 专用 `Render2D(SortMode::None)` |
| `layers/tilemap.rs` | feature `tilemap` |

---

## 9. `>2 参`白名单

R1 的唯一例外清单（除数据构造器外）：

| 入口 | 参数 | 理由 |
|---|---|---|
| `Ui::begin(window, &mut text, &mut state)` | 3 | 低层唯一入口；运行时路径 `Frame::ui` 无常驻样板 |

其余全部 ≤2 参。新增 >2 参入口必须在本表登记并说明理由。

---

## 10. 零调用 / 意外公开 → 降级清单

以下在重设计前属于「公开但仓库内零调用」或「公开但外部不可用」，按 R10 降级或删除：

`Text::{draw_text, draw_text_clipped, draw_label, draw_label_with, measure, page_size, load_font_data→保留}`、
`TextLayout::{draw_with, draw_sprite2d, draw_2d_gradient}`、`TextRender::{origin_px, clip_world, glyphs}`、
`DynamicAtlas::{generation, texture_uid_of, total_free, largest_free, fragmentation, end_frame, compact, get_or_revive, insert_dyn, insert_permanent, insert_ex_permanent, insert_ex_origin, insert_no_clamp}`、
`StaticAtlas::{from_toml, to_toml}`（保留但收 feature）、`TOMLETEntry`、`parse_toml_entries`、`proc::ProcTextures` 根导出、
`layout::Frame` 的 `pub fn`、`rjw_time` 的私有 `trait Get`、`MainHandler::new`、`rjw_2d_render` 的
`Index`/`TriIndicies`/`MeshStorage`/`DrawKind`/marker 类型。

降级方式：`pub(crate)`（内部消费）或 `#[doc(hidden)] pub`（类型别名需要）。

---

## 11. 维护约定（改 API 时）

1. 先查 §5 简并表：是否已有该概念？有则扩展现有类型，不要新增类型。
2. 新入口必须先过 R1–R3：≤2 参、无 bool、无 `Option` 开关。
3. 新类型进 prelude 的条件：happy path 必需 + 无同名冲突。新增需同步本文件 §6。
4. 破坏性变更：同步更新 `docs/API_REFERENCE.md`、`docs/ENGINE_GUIDE.md`、本文件映射表（§8）与受影响示例。
5. 示例即验收：任何 API 变更都必须反映到 `examples/`；`cargo check --workspace --all-targets` 必绿。
6. 文档中的每个签名都要能被 `cargo test --doc` 或示例编译验证；禁止文档里出现不存在的 API。

---

## 12. 实现进度（P1 完成时的实际状态）

> 本文件是**目标契约**；实现按阶段推进。以下是 P1 结束时的落地情况，供后续阶段对齐。

### 已落地（与本契约一致）

| 项 | 说明 |
|---|---|
| 构建器命名（P2） | `.color()` → **`.tint()`**、`.pos()` → **`.at()`**（`Draw2D` 全量改名，共 40+ 调用点；`TextStyle` / UI 控件 / `Tile` 的 `.color()` 不受影响） |
| 句柄与入口收敛（P2） | `Render2D::static_mesh(MeshId)`（不再收裸 `u64`）；删除 `mesh_with_cap` / `polygon_uv` / `create_texture` / `register_texture` / `register_mesh`（零调用或已被 `Gfx` 取代）；`Render2D::{device,queue,texture_layout}` 保留为**低层逃生口**（记入 §7） |
| `Layer` | 补 `From<i32> / From<u32> / From<i64>` ⇒ `layer(1)` 可直接写 |
| **图集收敛（P2）** | `DynamicAtlas::new(gfx, AtlasConfig)`（`page_size` 进 config）；`insert(key, Rgba8)` / `insert_with(key, Rgba8, InsertOpts)` / `insert_dynamic(key, size, SpriteSource)` / `white()`；`region()` / `region_or_revive()` / `handle()`；**`tick()`**（原 `end_frame`，改由运行时每渲染帧驱动 ⇒ 修 B3）；`stats()`；`sprite(&handle) -> AtlasSprite`（取代「查 TEXTURES + 手算像素 UV + `with_uv_tex`」）；`TextureRegenerator` → `SpriteSource`；TOML 收进 feature `toml` |
| 文本子系统构造 | `Text::new(gfx)`（取代 `(device, queue, layout)`）、新增 `Text::tick()`（字形图集寿命，运行时驱动） |
| `Gfx` | 新增 `Deref<Target = Gpu>`（`DynamicAtlas::new(gfx, cfg)` 等「收 `&Gpu`」的构造器可直接传 `gfx`）+ `format()/depth_format()/size()` |
| `Clear` | 取代 `ClearConfig`；`Clear::{Keep, Color, ColorDepth, Depth, Stencil}` + `From<Color>/From<ColorF64>` |
| `Gpu` / `Rgba8` / `MeshSpec` / `MeshId` | 能力对象 + 资源工厂；构造器收 `&Gpu`（三件套不再手动穿线） |
| `RenderFrame::pass(clear) -> PassBuilder` | 收集 recorder 后**自动推导**深度附件需求（B2 的 panic 路径消失） |
| `PassBuilder::{record, end, escape}` | `PassScope` 删除；`RenderFrame::record` 删除 |
| `unsafe RenderContext::new` + `resize((w,h))` + `acquire_frame` | 安全前提显式化；`FrameSource` 抽取出取帧接口（`Never` 供测试） |
| **帧级 VP 槽环** | 每个画面一个独立槽 + 动态偏移绑定（**修 B1 多画面 VP 串味**） |
| **深度附件兼容修复** | 同一 pass 内混用「用深度 / 不用深度」命令时，后者管线也必须声明同格式附件（`RStates::declared_depth_stencil`），否则 wgpu 报 `Render pipeline targets are incompatible with render pass` |
| 深度附件池 key | 加入 `device_key`（修 B7 多设备串用）；`MeshData` label 与 uid 一致（修 B8） |
| `Camera2D { region: Rect, transform: Transform2D }` + `Deref/DerefMut` | `Viewport`/`ViewCull` 删除；`zoom` 变派生视图；相机运动 API 不再复制 |
| `Transform2D` | 删 `*_by`/`*_components`；`with_transform`→`compose`、`with_inverse_transform`→`compose_inverse`；新增 `to_matrix`；`inverse()` 的「对象级 vs 点级」差异写进文档（点级精确逆走 `inverse_transform_point`） |
| `Rect` | 补 `From<(f32,f32,f32,f32)>`/`[f32;4]`、`touches()`、`pos/size/is_empty/inset/left_half/right_half/top_half/bottom_half`；修正 `intersects` 文档 |
| `Render2D` | 删相机状态（`set_mvp`/`set_camera`/`set_scissor`）；`submit(&mut PassBuilder, &Camera2D)`、`render(ctx, clear)`、`viewport(Rect)`/`scissor`/`reset`、`sort`/`sort_custom`、`cull`/`cull_mode`、`states_mut`、`discard` |
| 状态糖（部分） | `.depth(impl Into<DepthState>)`、`.stencil(impl Into<StencilState>)`；`DepthState/StencilState::{test_write, test_only, off}` + `From<bool>` |
| 输入 | `KeyState` 公开重导出；`KeyboardInput::{key, keys, chars, next_frame}`、`MouseInput::{button, pos_px, motion, wheel, next_frame}` |
| 运行时（模块化） | `runtime::{config, game, engine, gfx, ctx, frame, window, layers::ui}`：`App`/`run`/`run_with`/`AppConfig`/`Background`/`Ctx`/`Frame`/`Gfx`/`WindowId`/`FrameSource` |
| 帧语义 | `App::update` 每帧必调；应用用 `let Some(mut f) = ctx.frame() else { return };` 守卫；无帧按 `Background::Throttle(50)` 退避（B10）；`Frame` 析构自动 present（B6） |
| 提交语义 | `Frame::submit` 只置「本帧已被应用接管」（`Ctx::presented`）⇒ 帧尾**不再**补一次 `AppConfig::clear` 的 pass（否则那次 pass 会把刚提交的画面清掉；该 bug 由 RenderDoc 实测发现并修复） |
| **提交≠呈现（修 B11「能跑但没画面」）** | `Frame::submit` **不**置 `Frame::presented`：提交只把队列落成 pass，**呈现统一由 `Frame` 析构收尾**（`Ctx::present` 幂等；一帧多画面只能 present 一次）。曾经的写法在 `submit` 里置 `presented` ⇒ 析构跳过 present ⇒ `RenderFrame` 未 present 直接被丢弃 ⇒ **窗口永远空白**（`RUST_LOG=warn` 可见 `RenderFrame 未 present 就被丢弃`）。egHello / eg260731RPG 均由此黑屏 |
| 冒烟 = 画面证据（不再只数迭代） | `Ctx::presented_frames()` 统计**真正 present** 的帧；`--frames N` 收尾打印 `[OK] N iterations / N frames presented`，**0 呈现直接退出码 2 失败**——否则「跑满 N 帧但一帧没上屏」会伪装成通过（B11 的教训） |
| `Gfx` | 除资源工厂外还暴露 `format()` / `depth_format()` / `size()` ⇒ 自建管线可以在 `init` 期完成（不需要在第一帧懒建） |
| 诊断 | 自动提交路径有 `log::trace!`（`RUST_LOG=trace` 下可见）：应用已 `submit` 的帧**不应**出现该行 |
| prelude | 重排为零冲突清单（§6）；winit 类型移出到 `rjw_krusie::main` |
| `SpriteRectPx` 删除 | 像素 UV 归并入 `SpriteRect::with_uv_px`/`with_uv_tex`/`uv_px`；`expand`/`exceed`/`shrink_mesh` 一并删除（零外部调用），收窄只留 `shrink`（世界）/ `shrink_uv`（归一化），不再 clamp |
| 隐藏意外公开 | `Index`/`TriIndices`（顺带修拼写）/`MeshStorage` → `pub(crate)`；kind 标记（`Sprite`/`Mesh`/`StaticMesh`/`Custom`）与 `DrawKind` → `#[doc(hidden)]`（它们是 `Draw2D<K>` 的公开约束，不能降为 `pub(crate)`） |
| `Render2D::region(AtlasSprite)` | 图集直达入口（区域 + 页纹理一次拿到），`AtlasSprite::region` 直接映射为 `SpriteRect` |
| `rjw_main` 收敛为平台底座 | 删除零调用的 `App`/`MainContext`/`MainHandler`/`run_app`（与运行时的 `Engine` 重复）；保留 winit/输入/计时重导出 + `PRIMARY_WINDOW_TITLE` |
| **`DebugPainter`** | `debug_draw` 的 7 个 5~7 参自由函数删除；`r2d.debug(style)` 统一入口（`From<Color> for DebugStyle`）；`thick_line_quad` 保留给 `rjw_ui` |
| **UI 层坐标空间 = 物理像素、左上原点** | 运行时给 UI 层一个"平移了 `region.center()`"的 identity 相机（`Ctx::submit_with`）⇒ `f.draw_ui()` / `f.text_ui()` 里的 (0,0) 就是窗口左上角（旧实现是**居中坐标**，`f.draw_ui()` 画在 (12,12) 会跑到屏幕中央——RPG HUD 踩过这个坑）；`rjw_ui::screen_fixed_tf(anchor_px)` 随之简化为纯平移，`Ui::finish` 不再需要 `viewport` 参数 |
| **`Frame::text_ui(..)`** | UI 层文本入口（与 `f.draw_ui()` / `f.ui()` 同层）——修「屏幕固定 UI 文字只能画进世界层」的缺口（RPG HUD 文字此前走 `f.text()` ⇒ 层级错 + 位置靠相机反算） |
| **Mesh/quads 的 `.transform(tf)` 被静默忽略（修 B12「位置与视觉不一致」）** | `DrawKind for Mesh::commit` 旧实现只在 `.matrix(m)` 时写 `mat_idx`，**`.transform(tf)` 对 `mesh`/`polygon`/`quads` 完全不生效**（顶点被当作"已是世界坐标"）。世界层用法看不出（顶点本来就是世界坐标），但 `rjw_ui` 的窗口/控件四边形是**窗口局部顶点 + 屏幕固定变换** ⇒ 变换一丢，所有窗口都画在 UI 空间原点：**引擎里 `origin=(840,360)`，视觉恒在左上角**。修法：`Mesh::commit` 一律写矩阵（`.matrix(m)` 优先，否则 `transform2d_model(&b.transform)`）；回归测试 `mesh_transform_is_baked_into_matrix` |
| **`.model(Mat4)` → `.matrix(Mat4)`** | 与契约 §8.3 一致（旧名与内部字段同名，易混）；`Sprite`/`StaticMesh`/`Mesh` 三个 kind 共用 |
| **诊断：`origin` vs `submit`** | `Ui::debug_dump()` 增加 `submit`（**上一次 finish 实际提交用的平移量**，存 `UiState::debug_submit`）。`origin` 是引擎状态、`submit` 是真正喂给渲染的变换 —— **两者不一致即"位置与视觉不符"**，可一眼定位（B12 就是这样被抓出来的） |
| **调试工具（Rust 侧）** | `Ui::debug_dump()`（窗口 id/z/origin/submit/尺寸/拖拽状态/持久位置 + 鼠标/焦点，单行可 grep）、`Ctx::debug_inject_mouse(pos, down)`（**脚本化鼠标**，引擎合成按下/释放边沿）、`Ctx::presented_frames()`（冒烟"有画面"证据）、示例 `--ui-dump` / `--sim-drag`；`docs/DEBUGGING.md`（状态速查表 / 日志 / 断点 / LLDB·cdb / `.vscode` CodeLLDB 配置 / 图形层兜底）与 `.vscode/{launch,tasks}.json` |
| **多画面颜色清屏限定在画面矩形内（修 B13「左分屏完全未显示」）** | wgpu 的 load-op 颜色清屏作用于**整张附件** ⇒ 第二个画面「清成某色」会把第一个画面一起抹掉（左分屏变纯色）。运行时把**非首个画面**的颜色清屏降级为「`Clear::Keep` + 在世界层先画一块覆盖画面矩形的实心底」（`Clear::without_color()`，并在世界层用 `screen_space_transform(cam, region)` 把屏幕矩形映射成该相机下的世界矩形）；回归测试 `without_color_keeps_depth_and_stencil` + `screen_space_transform_matches_screen_to_world` |
| **画面边框（`AppConfig::viewport_borders`）** | 多画面 / 分屏调试辅助：帧末（present 之前）开**一个视口 = 整屏的 `Clear::Keep` overlay pass**，在**UI 层（最上层）**给本帧每个画面矩形描一圈（按序号配色）+ 左上角标注 `#序号 宽×高`。只描边不填充 ⇒ 不挡内容。⚠ 必须在**独立的全屏 pass** 里画：各画面自己的 pass 有各自的视口/裁剪矩形，在里面画别的画面会被裁掉 ⇒ 一律放在帧末收尾（`Frame::present` / `Frame::drop` / 自动提交路径），且不依赖 `surface`（present 后已不可用） |
| 诊断：画面矩形 | `RUST_LOG=rjw_krusie=debug` 打印每个画面的 `region` 与清屏意图（多画面"画到哪去了"第一手证据）；冒烟/边框 overlay 的调用也可由此观察 |
| **多画面的 UI 层语义（明确规则，非隐含行为）** | ① 每个画面的 UI 层坐标 = **该画面矩形左上角为原点的物理像素**（`ui_cam.transform.pos = region.center()`）⇒ 分屏各画各的 HUD 无需偏移换算；② `Render2D::submit` 是**提交即清帧**（世界层 + UI 层队列一起清）⇒ 多画面必须**交错写入**（录左屏 → `submit` 左屏 → 录右屏 → `submit` 右屏）；录完再连续 `submit` 两次时，第二次队列已空。两条都写进 `ENGINE_GUIDE.md` §4.4.1（此前的"UI 层在多画面下只出现在最后一个 pass"现象即由这两条共同解释：不是 VP 串味，而是队列清空 + 各画面 scissor 裁剪） |
| 冒烟 + 画面证据 | `--frames N` / `KRUSIE_SMOKE_FRAMES=N` 自动退出，收尾打印 `[OK] N iterations / N frames presented`，**0 呈现 = 退出码 2**；10 个示例实跑 30 帧全绿（各 30/30 呈现）；`egHello` / `eg260731RPG` / `eg260818UI` 另经 RenderDoc 抓帧导出 PNG 目视确认（地形 / 精灵 / 文本 / HUD / UI 窗口全部到位） |

### 尚未落地（后续阶段要做的）

| 项 | 阶段 | 说明 |
|---|---|---|
| ~~`Quad` 类型 + `quads(&[Quad])`~~ | **不采纳** | 与 `quads(&[VertexP3U2C4])`（低层顶点数组：`rjw_ui` / `rjw_tilemap` 直接产出顶点，无法用 4 点 `Quad` 表达多段）+ `quads_with(\|q\| q.quad(..))`（流式）三份表达重复（R13） |
| ~~**`rjw_text` 单链**（`Text::label` + `TextStyle` + `Glyph`；删 `TextRender`/`TextLayout`/`Style`/10 条绘制路径）~~ | 已落地 P2 | `TextCtx::{label, label_from, buffer, geometry, measure_buffer, lines, white_region, user_texture}` + `Label::{at, center, anchor, offset, clip, clip_world, no_cull, cache, gradient, map}` → 终点 `draw(layer)` / `draw_to(r2d, layer)` / `draw_with(&Glyph)` / `measure` / `into_buffer`；5 个示例已迁移 |
| ~~`rjw_tilemap`（`draw(r2d, &atlas, layer)`、`solid_rects(&self)`、dirty 拆分、`Tile::new` + `.tint`）~~ | 已落地 P2 | 修 B4（`push→solid_rects→draw` 丢 tile）+ 单测回归；`whole_region`/`visible_count`/`mark_dirty` 删除；chunk 粗剔复用 `r2d.culler()`（`r2d.cull(..)` 一处设置两处生效）；mesh 生成顺序按 (页, 层) 排序（`HashMap` 迭代序随机 → 确定性） |
| ~~`rjw_collision`（`Aabb { rect, transform }` + `slide`）~~ | 已落地 P2 | 取代 `Collider` / `collides`(4 参) / `move_and_collide`(5 参)；`Aabb::Default` 手写（`Rect: !Default`） |
| ~~输入 crate 的单一 `rjw_keystate`~~ | 已落地 P2 | `rjw_keyboard`/`rjw_mouse`/`rjw_ui` 之前依赖**registry** 的 `rjw_keystate 0.1.0`，与工作区成员 0.1.1 并存（两套 `KeyState` 类型）；改为 `path` 依赖，registry 副本从 `Cargo.lock` 消失；顺带删掉 `rjw_ui/Cargo.toml` 里被忽略的 `[profile.dev]`（警告） |
| ~~`rjw_ui` 收敛（入口/去重/裸布尔/焦点/样式 + 修 B5）~~ | 已落地 P3 | 窗口/模态入口统一为 `window(id)` / `modal(id)` builder（删 6 个 `*_at*` 变体 + `UiAdd` 侧 4 个）；裸布尔全部换枚举（`Level` / `Placement` / `Resize` / `Child`）；`text_focus()` 取代 `capturing_text()`（修"任何焦点都吞快捷键"）；Button/Checkbox/Radio/Slider 补 `register_focus`（Tab 导航 + Enter/Space 激活真正生效）；`GradientAxis` 与 `rjw_text` 合并为一份；`theme()`/`theme_mut()` 取代公开字段；`*_styled`/`slider_at_drag`/`set_next_min/max` 转 crate 内；删 `mouse_logical()`/`Widget::resizable()`/`pub use proc::ProcTextures`；`layout::Frame` 方法转 `pub(crate)`。**示例 `eg260818UI` 经 RenderDoc 抓帧目视确认**（窗口/按钮/滑块/输入框/数字框/勾选/多行文本/旋转染色窗口全部到位） |
| ~~旧输入别名清除（`get`/`get_chars`/`get_ime_*`/`end_frame` / `get_mouse_*`）~~ | 已落地 P4 | 公开面已无旧名：设备入口为 `ctx.key(..)` / `ctx.keys()`（`chars()` / `ime_commits()` / `ime_preedit()`）/ `ctx.mouse()`（`pos_px()` / `motion()` / `wheel()` / `in_window()` / `button(..)` / `buttons()`），边沿推进由运行时调用 `MainContext::next_frame()`（`end_frame` 转 `pub(crate)`）；`docs/` 内已无陈旧符号（保留 §8.3 旧→新映射表本身） |
| ~~文档全量对齐（B9）~~ | 已落地 P4 | `API_REFERENCE.md`（§11 UI：builder 链 + `Level`/`Placement`/`Resize`/`Child` 枚举表 + `text_focus()` + `finish(r2d)` + `debug_dump()`；§8.3 输入新名）、`ENGINE_GUIDE.md`（§4.4.1 多画面与每画面 `Clear` 语义 + §9 输入新名 + §14 `region`/`scale` + §18.2 运行时 `f.ui(..)` 与低层逃逸舱口）、`DEBUGGING.md`（§1 状态速查 / §3 多画面边框 / §6 RenderDoc 兜底）、`README.md` 及各 crate README |
| ~~版本号 0.x → 0.3.0~~ | 已落地 P4 | 16 个 crate + 10 个示例统一 `0.3.0`，内部 `path` 依赖的 `version` 要求同步为 `0.3.0`（`cargo check --workspace --all-targets` 绿） |
| ~~`cargo clippy --workspace --all-targets -D warnings`~~ | 已落地 P4 | **0 warning**。修掉全部真实告警（`sort_by_key` / `needless_range_loop` / `useless_vec` / `manual_checked_div` / `field_reassign_with_default` / `doc_lazy_continuation` / `unusual_byte_groupings` / 多余括号 / 空 doc 行）；仅对**结构性**告警留白名单：`rjw_ui` crate 级 `#![allow(too_many_arguments, type_complexity)]`（内部"布局/命中/绘制"坐标数学，不在公开面上）+ 4 处单点 `#[allow(.., reason = "..")]`（`rjw_text` 排版参数表、`rjw_atlas` 插入原语、RPG 示例绘制辅助、`chain.rs` 闭包字段） |
| 已接受偏差（记录在案） | — | ① `Id` 统一未做：仍保留 `IdRelative` / `IdAbsolute` / `IdStack` 三型（类型层面杜绝双重前缀与相对/绝对混用，收益大于"单一 `Id`"）；② `KeyboardSnapshot` / `MouseSnapshot` 保持公开（`capture` 产出的只读 DTO，方法名与设备类型一致；应用层无需命名，不构成入口混乱）；③ 一次性着色器/管线逃逸口 `custom(..)` 保留低层 `wgpu` 类型（L3 逃生口按契约允许） |

---

## 附录 A：`egHello`（冻结样例，目标形态）

```rust
use rjw_krusie::prelude::*;

#[derive(Default)]
struct Hello {
    cam: Camera2D, // 用户持有：region（矩形区域）+ transform（2D 变换）
    t: f32,
}

impl App for Hello {
    fn config(&self) -> AppConfig {
        AppConfig::new("egHello").size(1280.0, 720.0)
    }

    fn update(&mut self, ctx: &mut Ctx) {
        // 逻辑：无帧也执行
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }
        let (dt, fps) = (ctx.dt(), ctx.fps());
        self.t += dt;

        // 渲染：守卫在应用里
        let Some(mut f) = ctx.frame() else { return };

        self.cam.set_region(f.region());
        let cursor = self.cam.screen_to_world(f.mouse().pos_px());
        let pan = Vec2::new(
            key_axis(&f, KeyCode::KeyD, KeyCode::KeyA),
            key_axis(&f, KeyCode::KeyS, KeyCode::KeyW),
        );
        self.cam.move_local(pan * 400.0 * dt); // Deref → Transform2D

        f.draw()
            .solid(SpriteRect::new((-50.0, -50.0), (100.0, 100.0)))
            .at(Vec2::ZERO)
            .rot(self.t)
            .tint(Color::GREEN)
            .blend(BlendMode::Additive)
            .depth(DepthState::test_write(CompareFunc::LessEq))
            .layer(0.0);

        f.text(|t| {
            t.label(format!("FPS {fps:.0}   dt {:.1} ms", dt * 1000.0))
                .at(Vec2::new(-620.0, -340.0))
                .draw(10.0);
        });

        f.submit(&mut self.cam, Clear::color(Color::rgb(0.10, 0.11, 0.16)));
    }
}

fn key_axis(f: &Frame, plus: KeyCode, minus: KeyCode) -> f32 {
    (f.key(plus).pressed() as i32 - f.key(minus).pressed() as i32) as f32
}

fn main() -> Result<(), EventLoopError> {
    run(Hello::default())
}
```
