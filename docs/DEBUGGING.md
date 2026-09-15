# 调试指南（Rust 代码级）

> 面向**引擎 / 应用代码**的调试（断点、日志、状态快照、脚本化复现）。
> 图形层专项（抓帧、像素、渲染目标导出）在 §6，但**优先用 §1–§5 的 Rust 侧手段**：
> 大多数"没画面 / 位置不对 / 拖不动"的根因在状态，而不在 GPU。

---

## 1. 先看状态：`Ui::debug_dump()` / `Ctx` 事实

引擎把"自己眼里的世界"暴露成可打印结构，**不要靠猜**。

```rust
// 应用侧（eg260818UI 的 `--ui-dump` 即此法）
f.ui(theme, |ui| {
    // ... 录制 UI ...
    eprintln!("{}", ui.debug_dump());   // 或 log::info!(..)
});
```

输出是单行可 grep 格式：

```
ui[frame=21 scale=1.50 viewport=(1920,1080) mouse=(1180,200) in_win=true focus=None text_focus=None]
  | inv_panel z=1 origin=(450,135) size=(339,49) drag=false press=None stored=None
  | win_a     z=2 origin=(840,360) size=(348,254) drag=false press=None stored=None
  | win_b     z=3 origin=(1140,180) size=(534,337) drag=true press=Some(Vec2(1140.0, 180.0)) stored=None
```

字段含义（`rjw_ui::UiWindowInfo`）：

| 字段 | 读法 |
|---|---|
| `origin` | **本帧提交原点**（物理像素、屏幕左上原点）= 渲染时 `screen_fixed_tf(origin)` 的平移量 ⇒ **这就是窗口在屏幕上的位置** |
| `size` | 结算尺寸（物理像素） |
| `z` | 层级（越大越上；点击置顶会 +1） |
| `drag` / `press` | 拖拽是否激活 / 按下时的窗口左上角（`None` = 本帧没有拖拽基准） |
| `stored` | **跨帧持久位置**（`UiState::panel_pos`；`None` = 从未拖过，用传入 `pos`） |
| `mouse` / `in_win` | 引擎收到的鼠标物理坐标 / 是否在窗口内 |
| `focus` / `text_focus` | 键盘焦点 / **文本焦点**（只有文本控件持焦点才非 `None`） |

运行时事实：`f.dt()` / `f.fps()` / `f.scale()` / `f.region()` / `f.frames()` /
`f.presented_frames()`；`Ctx::ui_state()`（取帧前读上一帧诊断）。

### 判定速查

| 现象 | 先看什么 | 结论 |
|---|---|---|
| 窗口位置不对 | `origin` 是否符合预期（= 传入 `pos` × DPI，或被 `WindowClamp::Screen` 夹过） | `origin` 对 ⇒ 问题在渲染/变换；不对 ⇒ 问题在 `pos` / 责任链 / clamp |
| **拖不动** | 拖拽时 `drag` 是否为 `true`、`press` 是否有值、`origin` 是否跟着鼠标变 | `drag=false` ⇒ 命中被挡（`window_occluded` / `press_claimed`）或鼠标坐标没更新；`drag=true` 但 `origin` 不变 ⇒ 责任链优先级或被 clamp |
| 松手后弹回 | `stored` 是否被写入新的位置 | `stored=None` ⇒ 拖拽没进入激活态（位移 < `DRAG_ACTIVATE_PX` / 基准被清） |
| 层级不对 | `z` 排序；世界内容是否本就在**世界层** | 屏幕固定 UI 应走 `f.draw_ui()` / `f.text_ui()` / `f.ui()`（UI 层恒在世界层之后提交） |

---

## 2. 脚本化复现交互：`debug_inject_mouse`（不需要真实鼠标）

```rust
fn update(&mut self, ctx: &mut Ctx) {
    let Some(mut f) = ctx.frame() else { return };
    let n = f.frames();
    // 第 20 帧在 (1180,200) 按下，随后每帧右移 6px（物理像素），第 80 帧释放
    match n {
        20 => f.debug_inject_mouse(Vec2::new(1180.0, 200.0), true),
        21..=79 => f.debug_inject_mouse(Vec2::new(1180.0 + (n - 20) as f32 * 6.0, 200.0), true),
        _ => {}
    }
    // ... 录制 ...
}
```

- 引擎按真实设备语义**合成边沿**（按下帧 `down_edge`、按住 `pressed`、释放帧 `up_edge`），
  所以命中 / 拖拽 / 点击 / 选中逻辑无差别生效；
- 配合 `--ui-dump` 即可**打印**交互过程中的引擎状态（无需人手操作、无需图形调试器）；
- 示例：`cargo run -p eg260818UI -- --sim-drag --ui-dump --frames 90`。
- **弹出面板也要这样守**：`ColorPicker` 的面板（模式行 / 文本框 / 警告按钮 / SV 平面 /
  色相条 / 通道行）只有**交互**才会录制，普通冒烟跑不到它的代码路径。示例的
  `--sim-picker` 就是为此：脚本化开面板 → 拖 SV 平面/色相条/通道滑块 → 聚焦文本框并注入
  非法文本 → 按警告按钮恢复 → 切呈现模式 → 点面板外收起，并打印
  `sim-picker: frame=… mode=… text=…` 与最终颜色作为证据：
  `cargo run -p eg260818UI -- --sim-picker --frames 110`。
  ⚠ 脚本坐标必须**由主题解算**（`Theme` 在 `Ui` 内才按 DPI 预乘）——写死像素在非 1.0
  DPI 下会点空。
- **重叠控件的命中归属**（"点了 A 却连 B 也触发"）：示例的 `--sim-overlap` 把鼠标压在两个
  **故意重叠**的控件交集中心（坐标由 `examples/eg260818UI/src/overlap.rs` 与绘制同源解算），
  按下 + 释放后打印

  ```
  sim-overlap: below=0 above=1 widget_occluded_hits=2 -> [OK] 重叠处只有最上层控件被触发
  ```

  （`cargo run -p eg260818UI -- --sim-overlap --frames 62`）。判定口径：下层计数必须是 `0`、
  上层必须是 `1`、`UiState::widget_occluded_hits()` 必须 `> 0`（否则 `[FAIL]`）。**这条断言
  真的能抓 bug**：把 `Ui::hit_abs` 的控件级遮挡去掉，输出立刻变成 `below=1 above=1 -> [FAIL]`
  ——正是"两个控件被一起触发"。
  ⚠ 注入**必须每帧重注**（注入只对下一帧生效，真实鼠标一动就顶掉它），否则断言会随
  真人手抖而随机失败。`RJ_OVERLAP_TRACE=1` 可打印两个探针逐帧的矩形 / 鼠标 / 命中 / 拦截数。
- **"这一像素到底是谁的"**（重叠 / 相邻控件边界、跨窗口遮挡、滚动条条带）：

  ```
  cargo run -p eg260818UI -- --sim-click 500,216 --frames 50      # 背包第 1 列两格共享边
  ```

  `--sim-click X,Y` 在指定**屏幕物理点**按下 + 释放（第 20/21 帧，之后停在原地到第 40 帧），
  并打印"这次点击切换了几个控件"（`背包已选中 N 个`）。配合 `RJ_HIT_TRACE=1`（引擎逐次
  打印命中归属：`OK` / `BLOCKED-by-widget` / `--`）就能一眼看出谁赢：

  ```
  hit[frame 22] inv_panel/inv/slot_0 BLOCKED-by-widget rect=(450,177,96,39) mouse=(500,216)
  hit[frame 22] inv_panel/inv/slot_3 OK                 rect=(450,216,96,39) mouse=(500,216)
  sim-click: 背包已选中 1 个 / 控件遮挡拦截 1 / 窗口遮挡拦截 0 [OK] 一次点击最多切换一个控件
  ```

  同一命令在"控件级遮挡"关掉时输出 `背包已选中 2 个 … [FAIL]`——相邻格子的**共享边**
  被两个格子同时命中（`hit_test` 含边界）正是"一次点击切换两个物品"的现场。
  排查别的控件同理：换成它的中心 / 边缘坐标即可。
- **加载外部文件**（看真实素材下的布局 / 图片铺排 / 字体）：`--image <路径>` 用你自己的
  图片当 `ImageBg`（PNG / JPEG / BMP / GIF，四个窗口分别演示 Fill / Tile 等铺排），
  `--font-file <路径>` 把 ttf / otf / ttc 加载进运行时文本子系统（随后在 `字体…` 弹窗里
  输入该字体的**族名**即可全局换字）：

  ```
  cargo run -p eg260818UI -- --image .\shot.png --font-file C:\Windows\Fonts\consola.ttf
  ```

---

## 3. 多画面 / 分屏：先看"哪块是哪块"

```rust
fn config(&self) -> AppConfig {
    AppConfig::new("游戏").size(1280.0, 720.0)
        // 调试辅助：帧末在**最上层**给每个画面矩形描一圈（按序号配色）
        // + 左上角标注 `#序号 宽×高`
        .viewport_borders(ViewportBorders::On)
}
```

- 只**描边不填充** ⇒ 不挡内容；颜色按画面序号循环（红 / 绿 / 蓝 / 黄）；
- 实现要点：**独立的全屏 overlay pass**（`Clear::Keep`），在 present 之前画
  （各画面自己的 pass 有各自视口/裁剪矩形，在里面画别的画面会被裁掉）；
- 每个画面的矩形与清屏意图：`RUST_LOG=rjw_krusie=debug`（打印 `画面 #N region=(...) clear=...`）；
- 参考：`cargo run -p egMultiView`（左右分屏 + 画中画，已开启边框）。

> **每画面的 `Clear` 语义**：`Clear::color(..)` 在**同一帧的多个画面**里只对**第一个**生效
> （wgpu 的 load-op 清屏作用于整张附件）；后续画面的颜色清屏由运行时降级为
> 「`Keep` + 在画面矩形内画一块画面底色」⇒ 语义仍是"这个画面清成该颜色"，且不会抹掉别的画面。
> 只清深度（重叠画面）用 `Clear::depth(1.0)`。

---

## 4. 日志与断言

```powershell
$env:RUST_LOG="trace"                  # 全量（含 wgpu 校验层）
$env:RUST_LOG="rjw_krusie=trace,rjw_ui=debug,rjw_2d_render=debug"
$env:RUST_BACKTRACE="1"                # panic 栈
cargo run -p egHello -- --frames 60
```

- **"未 present 就被丢弃"**（`RenderFrame` 的 warn）：本帧没有上屏 ⇒ 画面永远空白。
  提交（`f.submit`）与呈现（`f.present` / `Frame` 析构）是两件事，见 `docs/ENGINE_GUIDE.md` §4。
- **冒烟开关**：`--frames N` 跑满 N 次迭代后退出，并打印
  `krusie smoke: [OK] N iterations / N frames presented`；**0 呈现 = 退出码 2**
  （挡住"能跑但没画面"的回归）。
- 帧内诊断钩子：`Ui::debug_dump()`、`UiState::{stats, occluded_hits, last_press_window}`
  、`Render2D::{will_use_depth_stencil, sort_mode, cull_mode}`。

---

## 5. 断点调试（编辑器 / 命令行）

已提供 `.vscode/launch.json`（CodeLLDB）与 `.vscode/tasks.json`：

- **VSCode**：安装 `vadimcn.vscode-lldb` → 打开仓库 → 选 "调试 egHello / eg260818UI / …" → F5。
  断点可打在 `App::update`、`Ui::window`、`window_impl`、`Render2D::submit` 等任意位置。
- **命令行（LLDB）**：
  ```powershell
  cargo build -p eg260818UI
  rust-lldb -- .\target\debug\eg260818UI.exe --ui-dump
  (lldb) breakpoint set --name rjw_ui::ui::Ui::debug_dump
  (lldb) run
  ```
- **WinDbg / cdb**（Windows 原生）：
  ```powershell
  & "C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\cdb.exe" -g -G .\target\debug\egHello.exe
  ```
- 需要在断点里打印状态：`p ui.debug_dump()`（LLDB 可直接调用 `Debug` 类型的方法）。

> 构建配置：workspace 的 `[profile.dev]` 是 `opt-level = 2, debug = true` ——
> **变量 / 断点 / 单步完全可用**（速度也够跑实时示例）。

### MCP / 自动化
- 本仓库已把"可被外部工具读取"的状态做成**结构化 API**（`Ui::debug_dump()` /
  `Ctx` 事实 / `presented_frames`）——把它们接到任意 MCP 或 DAP 客户端即可
  （例如让 agent 直接读 `ui.debug_dump()` 的输出定位问题，而不需要图形调试器）。
- 会话内可用的图形 MCP（RenderDoc）见 §6，但它只回答"GPU 收到了什么"。

---

## 6. 图形层（兜底）：RenderDoc

只有当 §1–§5 已确认**状态正确**而画面仍不符时，才需要抓帧：

```powershell
# CLI 抓帧（需要 RenderDoc；`-w` 等目标退出）
& "C:\Program Files\RenderDoc\renderdoccmd.exe" capture -w `
    -c "C:\captures\hello" -d .\target\debug --opt-api-validation `
    ".\target\debug\egHello.exe" --frames 90
```

抓帧后检查顺序：
1. **Draw 数**是否符合预期（少了 ⇒ 录制/剔除问题；例如 `Render2D` 的剔除把 sprite 全滤掉）；
2. **Pass 边界**与 marker（`krusie: pass`）——一帧应有且仅有一次 present；
3. 导出渲染目标 PNG 目视（`export_render_target`）；
4. 采样具体像素（`pick_pixel`）确认颜色 = 预期（注意 sRGB 编码：清屏色 0.10 线性 ≈ 0.349 sRGB）。

---

## 7. 常用排查脚本

```powershell
# 1) 全工作区编译 + 测试
cargo check --workspace --all-targets; cargo test --workspace

# 2) 每个示例"是否真的有画面"（冒烟 harness 会给出呈现帧数）
foreach ($p in @('egHello','eg260731RPG','eg260818UI')) {
  cargo run -q -p $p -- --frames 30 2>&1 | Select-String "krusie smoke"
}

# 3) UI 交互脚本化复现 + 状态打印
cargo run -p eg260818UI -- --sim-drag --ui-dump --frames 90 2>&1 | Select-String "win_b"
```
