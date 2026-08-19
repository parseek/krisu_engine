# egui 融入计划（krusie engine）

> 状态：**草案，待拍板**。目标：把 [egui](https://github.com/emilk/egui)（立即模式 GUI）以
> **可选 crate** 的形式融入本工作区，与既有 `rjw_ui` **并存**，互不干扰。

## 0. 结论速览（TL;DR）

- 新增可选 crate **`rjw_egui`**，封装 `egui` + `egui-wgpu` + `egui-winit` 三件套，面向引擎的
  API 风格（`Egui2D::new → on_window_event → begin_frame → 用户画 UI → render_command_buffer`）。
- 引擎本体只改一处：**`rjw_main::App` trait 增加一个原始 `WindowEvent` 转发钩子**（默认空实现，
  现有示例零改动）。egui-winit 需要原始事件，现有 `KeyboardInput`/`MouseInput` 是加工态，不够。
- 渲染走"**第三条命令缓冲**"：`cb_world → cb_ui(rjw_ui) → cb_egui` → 一次 `submit` + 一次
  `present`，与 `eg260818UI` 的多缓冲合并模式完全一致，**`rjw_2d_render` 零改动**。
- ⚠ **最大前置风险：wgpu 版本对齐**。egui-wgpu 依赖的 wgpu 必须与引擎的 **wgpu 30.0.0**
  **完全同版本**（`egui_wgpu::Renderer::new(device,…)` 要拿引擎的 device，两个 wgpu 大版本
  无法在同一进程共存）。M0 必须先解决这一点。

## 1. 背景与定位

| 项 | 现状 |
|---|---|
| 渲染栈 | wgpu **30.0.0**（`rjw_render` 重导出）、winit **0.30.13**（`rjw_main` 重导出）、edition 2024 |
| 自有 UI | `rjw_ui`：立即外观 + ID 持久状态、pack/grid/place、中文 IME、键盘导航——引擎原生轻量 UI |
| egui 定位（建议） | **工具型 UI**：调试面板 / 属性编辑器 / 相机参数滑条 / 资源浏览器 / 帧统计；**快速原型 HUD** |
| 不做什么 | 不替换 `rjw_ui`；不改变 Camera2D 坐标系；不把 egui 变成"唯一 UI" |

**成功标准**：`cargo run -p eg260819Egui` 一屏同时看到 游戏场景 + rjw_ui + egui 面板；
中文输入正常；高 DPI 下无模糊；RenderDoc 里三条 pass（world / rjw_ui / egui）清晰可辨。

## 2. M0 — 版本对齐（首要任务，需联网验证）

**已知信息**（2026 年中）：

- egui 0.33 ↔ wgpu **27**（[ruffle 升级提交](https://github.com/ruffle-rs/ruffle/commit/067bdffdf233ebedef04805a72a2baa8f381b3d7)）；
- egui 0.34.3 已发布（[newreleases](https://newreleases.io/project/github/emilk/egui/release/0.34.3)）；
- 本仓库 wgpu 30.0.0（[wgpu v30 CHANGELOG](https://raw.githubusercontent.com/gfx-rs/wgpu/v30.0.0/CHANGELOG.md)）。
- 注：本计划写作时沙箱无法访问 crates.io（SSL 凭证错误），以下需在可联网环境执行。

**决策树**：

1. 查最新 `egui-wgpu` 的 wgpu 依赖：`cargo add egui-wgpu --dry-run` 或 deps.rs/docs.rs。
2. **已有支持 wgpu 30 的版本** → 直接 pin（最优，零引擎改动）。
3. **没有** → 用 git 依赖 `emilk/egui` master（egui 通常紧跟最新 wgpu）。
4. **都不能** → 引擎整体降级 wgpu 到 egui 支持的最新版（如 27/28/29），波及
   `rjw_render` / `rjw_2d_render` / `rjw_atlas` / `rjw_text` / `rjw_ui`——wgpu 小版本 API
   差异有限，改动可控，但**属于全局决策，需作者拍板**。

**验收**：`cargo tree -i wgpu` 工作区只有一份 wgpu；`cargo check --workspace` 通过。

## 3. 架构设计

### 3.1 新 crate：`crates/rjw_egui`

```
crates/rjw_egui/
├── Cargo.toml      # rjw_render, rjw_main, egui, egui-wgpu, egui-winit, wgpu(与 rjw_render 对齐), log
└── src/
    ├── lib.rs      # crate 文档 + 重导出 egui / egui_wgpu / egui_winit（同 rjw_render 重导出 wgpu 思路）
    ├── egui2d.rs   # Egui2D 后端（见下）
    └── input.rs    # 输入路由策略辅助（wants_input、屏蔽建议）
```

**`Egui2D` 核心 API（草案，最终以所选 egui 版本 API 为准）**：

```rust
pub struct Egui2D { /* egui::Context + egui_winit::State + egui_wgpu::Renderer */ }

impl Egui2D {
    /// 在 on_init 中创建：由 RenderContext 拿 device/queue/format，由 Window 拿 scale_factor。
    pub fn new(render: &RenderContext, window: &Window) -> Self;

    /// 透传全部原始 WindowEvent（CursorMoved/MouseInput/KeyboardInput/Ime/ModifiersChanged/MouseWheel/…）。
    /// 必须在 App::on_window_event 里调用。
    pub fn on_window_event(&mut self, event_loop: &ActiveEventLoop, event: &WindowEvent) -> EventResponse;

    /// 帧内入口：结算输入 → 返回 egui::Context 供用户 show 面板 → finish 出 FullOutput。
    pub fn begin_frame(&mut self, ctx: &MainContext) -> egui::FullOutput;

    /// 编码 egui 渲染为独立 CommandBuffer（color load=Load/store=Store，不碰引擎深度）。
    pub fn render_command_buffer(&mut self, target: &wgpu::TextureView) -> wgpu::CommandBuffer;

    /// 处理 PlatformOutput：光标 icon、剪贴板（arboard，rjw_ui 已在用）、IME 窗口命令。
    pub fn handle_platform_output(&mut self, window: &Window, output: egui::PlatformOutput);

    /// egui 是否想消费输入（wants_pointer_input / wants_keyboard_input）。
    pub fn wants_input(&self) -> InputWants;

    /// 引擎纹理 → egui 图像（M5）。
    pub fn register_texture(&mut self, view: &wgpu::TextureView, filter: FilterMode) -> egui::TextureId;
    pub fn unregister_texture(&mut self, id: egui::TextureId);
}
```

### 3.2 `rjw_main` 的最小改动（事件转发钩子）

```rust
pub trait App {
    // ……既有方法不变……
    /// 原始窗口事件转发（默认空实现；egui / 自定义输入系统用）。
    /// 在 MainHandler::window_event 的最前面调用，之后仍执行既有的
    /// keyboard/mouse 吸收与 CloseRequested/Resized 处理（小重构：match 改按引用）。
    fn on_window_event(
        &mut self,
        ctx: &mut MainContext,
        event_loop: &ActiveEventLoop,
        event: &WindowEvent,
    ) { let _ = (ctx, event_loop, event); }
}
```

- 签名带 `&ActiveEventLoop` 是因为 egui-winit `State::on_window_event` 需要它。
- 可选同步加 `on_device_event`（鼠标增量等，egui 默认不需要）。
- 现有 8 个示例全部零改动（默认空实现）。

### 3.3 输入路由策略（关键设计点）

egui-winit 是**观察者**：它处理事件但不吞掉，引擎的 `KeyboardInput`/`MouseInput` 依然会记录
同一批事件。因此"谁消费"由**应用层**每帧裁决：

- 推荐模式（完全仿照 `rjw_ui::UiState::capturing_text()` 的既有惯例）：`about_to_wait` 开头
  查 `egui.wants_input()`；egui 悬停控件 / 聚焦输入框时，**跳过游戏快捷键与角色移动**。
- `rjw_ui` 与 egui 并存时：**不要同时让两个系统聚焦文本输入**——IME 事件只会被 egui-winit
  接走；聚焦切换前先失焦另一方。
- 鼠标命中同理：egui 面板区域内的点击不再发给游戏（`wants_pointer_input` 覆盖该帧游戏
  `MouseInput` 的使用）。

### 3.4 渲染集成（对 `rjw_2d_render` 零改动）

```rust
let Some((surface_tex, view)) = r2d.begin_frame() else { return };
let cb_world = r2d.render_command_buffer(&ClearConfig{ color: Some(…), .. }, &view, None);
let cb_ui    = r2d_ui.render_command_buffer(&ClearConfig{ color: None, .. }, &view, None);
let cb_egui  = egui2d.render_command_buffer(&view);          // egui 内部自建 encoder + RenderPass
r2d.queue().submit([cb_world, cb_ui, cb_egui]);              // 顺序即层级
r2d.queue().present(surface_tex);
```

- egui pass：`label: "Egui: RenderPass"`，`load=Load / store=Store`，无深度（可选接深度附件
  用于 3D 交互，默认 None）。
- 需要 egui 画在 rjw_ui **之下**时：交换 `cb_ui` / `cb_egui` 顺序即可（文档说明，不做配置项）。
- 与 `Render2D::flush(pass)` 的关系：`render_command_buffer` 模式与现有示例一致，无需新机制。

### 3.5 DPI 与坐标系

- egui 用 **points**（逻辑点），内部 × `pixels_per_point` 得物理像素；创建 `State` 时用
  `window.scale_factor()`，`ScaleFactorChanged` / `Resized` 时同步。
- egui 只作**屏幕空间叠加层**（左上原点、Y+ 向下，同 rjw_ui 口径），不做世界坐标换算——
  引擎场景坐标（Camera2D 中心原点、Y+ 向下）与 egui 无关。

### 3.6 中文字体（中文项目，必做）

- egui 默认字体**不含 CJK**。方案：启动时加载系统字体——Windows
  `C:\Windows\Fonts\msyh.ttc`（`egui::FontData` 支持 `index` 选 ttc 子字体）或随包带
  Noto Sans SC；追加进 `FontDefinitions.font_data` 与 proportional/monospace 回退族。

## 4. 实施里程碑

| 里程碑 | 内容 | 验收 |
|---|---|---|
| **M0** | 版本对齐（§2 决策树，需联网） | `cargo tree -i wgpu` 唯一版本；`cargo check --workspace` |
| **M1** | `rjw_main` 事件转发钩子（§3.2） | 现有示例全部回归通过 |
| **M2** | `rjw_egui` 最小闭环 + 示例 `eg260819Egui`：FPS 面板 | world→ui→egui 三缓冲同帧可见；RenderDoc 三 pass 可辨 |
| **M3** | 输入全量转发 + `wants_input` 屏蔽策略 + `handle_platform_output`（光标/剪贴板/IME） | egui TextEdit 中文 IME 输入正常；egui 聚焦时游戏快捷键不触发 |
| **M4** | 中文字体（§3.6）+ DPI/resize 正确性 + `request_repaint` 说明（`ControlFlow::Poll` 下每帧刷新，天然满足） | 125%/150% 缩放无模糊；中文面板渲染正常 |
| **M5** | 纹理互通（引擎纹理 → egui `image`；可选 egui 离屏 → 引擎精灵）+ 文档（README 模块地图、ENGINE_GUIDE 新章节、API_REFERENCE）+ 清理 | 示例完整演示；文档齐全 |

## 5. 风险与对策

| 风险 | 对策 |
|---|---|
| **wgpu 版本不匹配**（最大） | M0 决策树；git 依赖兜底；必要时全局降级（需作者拍板） |
| 事件双重处理（游戏 vs egui） | `wants_input()` 门控 + 文档化"谁消费"策略；仿 `capturing_text()` 惯例 |
| IME 竞争（rjw_ui 输入框 vs egui TextEdit） | 不同时聚焦；聚焦切换前失焦另一方 |
| DPI 口径混乱（points vs 物理像素 vs 引擎逻辑像素） | 统一换算说明写入 ENGINE_GUIDE；egui 只走 points |
| egui 依赖树大，编译时间/体积上升 | 可选 crate（不进默认依赖），文档注明 |
| RenderDoc 调试 | egui pass/encoder 加 label；示例支持 `--egui-below` 之类参数复现层级（仿 eg260818UI 的 `--win-a` 模式） |
| 与 rjw_ui 并存的心理模型 | 文档讲清"何时用哪个"：游戏内轻量 HUD → rjw_ui；工具/编辑器 → egui |

## 6. 待拍板决策点

1. **egui 定位**：工具型 UI 优先（推荐）还是游戏内 HUD 优先？（影响渲染顺序、输入策略优先级）
2. **与 rjw_ui 的关系**：并存（推荐）还是逐步替换？
3. **wgpu 降级**：若 M0 发现最新 egui-wgpu 尚不支持 wgpu 30，是否接受引擎整体降级？
4. **示例命名**：建议 `eg260819Egui`（沿用日期命名约定）。
