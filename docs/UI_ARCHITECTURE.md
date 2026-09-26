# `rjw_ui` 架构分析（含成本模型与取舍决策）

> 目的：在动下一刀之前，先把 `rjw_ui` 的**分层、数据流、真实成本、结构欠账**摊开，
> 让"接下来做什么"是一个有依据的决定，而不是上一轮结论的惯性延续。
>
> 数据来源：`examples/eg260818UI --frames 520`（稳态 120 帧滑窗均值）+ 一次
> **受控实验**（只改一个常量）。复现命令见文末。**未提交任何估算值**——每一行都是实测。

---

## 0. 设计理念：**无顾虑地使用**（use without concern）

**不写任何选项时，窗口 / 控件必须自己看起来是对的。** 应用只在**想要控制**时才写
`.width()` / `.height()` / `.pos()` / `.vscroll(..)` / `.hscroll(..)` / `.resize(..)`。任何"**必须写某个
选项否则坏掉**"的组合都是**缺陷**，不是"用法要求"。

由此推出的四条硬规矩（改引擎时必须守住，改坏了就是这一条被违反）：

1. **未指定的轴 = 由内容定**（自动撑开）：不写 `.width()` ⇒ 宽 = 内容自然宽（`pad + 内容 + pad`）；
   不写 `.height()` ⇒ 高 = 内容高（`vscroll(true)` 时再与"屏幕剩余"取 min ⇒ 矮内容不撑满、
   高内容长到屏幕底再滚）。写了 ⇒ **固定**；用户拖过 ⇒ **持久值接管**（`.width()/.height()`
   只是**初始值**）。这条链是**单向**的：内容 → 显式 → 用户拖拽，越靠后越优先。
2. **不许有"自我强化"的塌陷**：任何"本帧用上一帧结果反推本帧尺寸"的解算都必须有一个
   **不依赖上一帧**的引导值。反例（实测过两次）：滚动视口在**首帧**取 1px ⇒ 内容按 1px 折行
   ⇒ 窗口 = `2×pad` ⇒ 下一帧照它再算 ⇒ **永远一条缝**（用户："调色板编辑器在不指定 width 的
   情况下无法使用"）。引导值一律取**屏幕剩余**，随后收敛到内容尺寸。
3. **不写选项不该改变别人的行为**：不调 `.resize(..)` / `.vscroll(..)` / `.hscroll(..)` 时，
   窗口与"没这个特性之前"逐像素一致（老语义由 `resolve_*` 纯函数显式表达，不靠默认值暗改）。
4. **诊断先于猜测**：几何争议（"标题栏算不算进 scissor"、"谁把宽度撑大了"）必须能用
   `--ui-dump` / `RJ_*_TRACE` 看到数字，而不是靠读代码推断。规则见 `docs/DEBUGGING.md`。

> **横向滚动的一条附加硬规矩**：`hscroll(true)`（= `Scroll`）的那条轴**不上报可用宽**
> （`scroll_axes_avail_w`）⇒ 子项保持**自然宽、不折行**。这是"内容真的能横着滚"的前提：
> 一旦上报可用宽，`LimitedInParent` 控件就把自己压进视口 ⇒ 内容永远不溢出、横条永远不出现。
> 要按窗口宽折行就写 `hscroll(false)`（默认）。

> 验收这类理念不能靠"看着像"——每个反例都要落成一条 sim 判据（见 `docs/DEBUGGING.md` §2：
> `[无 width 也要能用]` / `[短内容视口]` / `[调色板四态]` / `[第二次拖柄不弹宽]`）。

---

## 1. 分层与依赖方向

```text
应用（examples / 游戏 / 编辑器）
   │  UiAdd（容器闭包内 &mut Ui 的方法表）/ Ui::*  →  add/add_at(Widget)
   ▼
┌─ rjw_ui ────────────────────────────────────────────────────────────────┐
│ ui.rs            Ui 模块根：`Ui` 结构 + 常量 + `begin`/帧状态搬运       │
│                  733 行 / 39 字段 + 13 个 ui/ 子模块（分工见 §5.7）      │
│ ui/*.rs          facade / prims / interaction / scroll / panel / window /│
│                  chrome / containers / builders / controls / textedit /  │
│                  commit / cmds —— 按职责分包，公开路径逐字不变            │
│ widgets/         Widget::ui 单方法协议 + 属性 builder（17 文件 4.1k 行）  │
│ layout hit focus edit view id   纯逻辑，不依赖 Ui                         │
│ painter/         Painter + DrawQueue（绘制原语与播放头，3D 无关）          │
│ draw.rs          UiDraw 命令 + 单位换算 + Gradient/Icon/ImageBg           │
│ tess.rs          圆角矩形 CPU 镶嵌（1227 行，质量与顶点数的唯一旋钮）      │
│ style.rs         Theme 令牌（2439 行）+ theme_toml 序列化                 │
│ gpu_batch.rs     几何收集 + 切段规则 + 内容签名                           │
│ backend.rs       UiBackend / UiBatch / Tri                               │
└────────────────────────────────────────────────────────────────────────┘
   │  顶点格式 VertexP3U2C4 借自渲染器（硬依赖，见 §5.5）
   ▼
rjw_krusie::Render2dUiBackend（唯一的桥）→ rjw_2d_render::Render2D → wgpu
```

**依赖方向是干净的**：`rjw_ui` 不反向依赖 `rjw_krusie`；渲染器只被桥接层认识。
`layout / hit / focus / edit / view / id` 是**纯逻辑层**（可无 GPU、无 `Ui` 单测），
`tess / gpu_batch / theme_toml` 亦然——这是本项目测试密度的来源，值得保持。

---

## 2. 一帧的数据流（含每一次顶点拷贝）

```text
① 录制    Ui::begin → 控件 push   → DrawQueue.queue: Vec<UiDraw>（384 条）
                                  记录（win, elem, depth, seq, rect, clip, kind）
② 分桶    bucket_cmds              → per-win → per-depth 桶（顺序保持 = 免全量排序）
③ 派生    z0_place_for_seq          → win=0 的**顶层放置序 place**（排序空间，见 §5.6）
③′ 签名   hash_cmds(全量, anchor)   → 逐窗 / 逐置放内容签名（含图集世代 + 行距/字重）
④ 命中    geom.clone()  ────────► cached: Vec<CachedQuad>      【拷贝 C1】864KB
   未命中  collect_cmds → 镶嵌（圆角/羽化/阴影/字形）→ 写回缓存（只在变化时）
⑤ 装配    ordered.extend(cached)（move）；按 (win, place, elem, g, tex, clip) 排序；
          segment_runs 切段；跨条 append                          【拷贝 C2】≤864KB
⑥ 交后端  flush_seg → UiBackend::submit(UiBatch)               （move，无拷贝）
⑦ 桥接    Render2dUiBackend → mesh_indexed → MeshStorage.extend 【拷贝 C3】864KB
⑧ 渲染器  Render2D::build → buf_all_verts.extend_from_slice     【拷贝 C4】864KB
⑨ 上传    write_buffer(buf_all_verts)                          【拷贝 C5】864KB + 191KB 索引
```

一帧 **384 条命令 / 7 扇窗**，同一份几何（`23988 顶点 × 36B = 864KB` + `31913 × 6B = 191KB`
索引）**最多被完整复制 4 次（C1–C4）再上传 1 次**。

⚠ 其中 **C2 是上界**：单条成段走 `mem::take`（无拷贝）、多条段才 `append`，具体比例未单独
测量（`asm` 里还含排序与切段）。**C1 / C3 / C4 是确定的整份拷贝**——三处 `clone()` /
`extend_from_slice()` 都在代码里逐字可见。这就是"顶点放大"叠加"拷贝链"的结果。

---

## 3. 成本模型（实测）

> ⚠ **下面的绝对值是"某一次测量"的现场**：同一台机器的整机速度会漂 ±30%
> （同一份代码不同时段测出的 `finish` 能从 0.50ms 到 0.77ms）。**跨天比较无效**，
> 任何结论都要**同机交错 A/B**（`ENGINE_GUIDE.md` §18.21 的表就是这么测的）。

稳态（520 帧，后 3 个 120 帧滑窗一致）：

```text
[perf] frame=2.03ms ui=0.83ms (prologue=0.01 record=0.25 finish=0.57)
       | ui: sort=26.7us sig=61.4us collect=2.7us clone=168.9us
             submit=289.3us(asm=164.8 flush=124.5)
       | render: total=2.03ms encode=829.6us submit=351.0us present=846.7us
       | cmds=384 wins=7 cache_hit=14.0 cache_miss=0.0 clip_batches=11
         segs=40 verts=23988 tris=31913
```

| 阶段 | µs/帧 | 性质 | 归属 |
|---|---|---|---|
| `prologue` | 10 | 开场 | 帧级，常数 |
| `record` | 250 | 应用录制 + 主题构造 | **应用侧**，不是引擎账 |
| `sort`（分桶） | 27 | O(n)，免全量排序 | 引擎，已被压到极限 |
| `sig`（签名） | 61 | 384 条命令全量哈希 ≈ 160ns/条 | 引擎，缓存的**门票** |
| `collect`（镶嵌） | **3** | 稳态几乎不镶嵌 | 缓存生效，几乎为零 |
| `clone` | 169 | **拷贝 C1** | 引擎，纯搬运 |
| `submit·asm` | 165 | **拷贝 C2** + 排序 + 切段 | 引擎，纯搬运 |
| `submit·flush` | 125 | **拷贝 C3** + 40 次 builder | 桥接/渲染器 |
| `finish` 合计 | **570** | | |
| （`render submit`） | 351 | **拷贝 C4** + `write_buffer` | 渲染器，在 UI 计时之外 |
| （`encode`） | 830 | GPU 编码 | 与顶点数正相关（见 §4） |

**结论一**：`finish` 的 570µs 里，**C1+C2 = 334µs（59%）纯粹是 `rjw_ui` 自己的顶点搬运**；
再算上 C3，**459µs（81%）是"同一份数据搬三次"**。缓存已经把 `collect` 压到 3µs，
剩下的成本不是"算"出来的，是"搬"出来的。

**结论二**：`sig=61µs` 是这套缓存机制的固定开销（每帧全量哈希）。它是**必要**的
（轻量摘要漏颜色位的历史 bug、图集世代、行距/字重都要求全量），但也意味着
"缓存"不是免费的——顶点数再降，签名开销不会降。

**结论三**：165Hz 下 `present=847µs` 是垂直同步等待，整帧 2.03ms 里 CPU 实做约 1.2ms，
UI 占 0.83ms。**当前 UI 不是瓶颈**——这一轮优化的价值全在"余量"，不在帧率。

---

## 2.5 分层契约（谁负责什么、谁能用什么）

> 这一节是**规矩**，不是描述：任何搬运 / 新增控件 / 扩展公开面都必须先过 §2.5.2 的判据。
> 起因是"为了把某块代码搬进 `widgets/` 而把引擎内部暴露成公开 API"——那是**职责被 API
> 倒逼**：公开面一旦为了搬运而开洞，边界就再也收不回来。

### 2.5.1 每一层 owns / 禁止

| 层 | 拥有（owns） | 禁止（must not） |
|---|---|---|
| `draw.rs` | 绘制**数据类型**：`UiDraw` / `DrawKind` / `Gradient` / `Icon` / `Size` / `Position` / `CornerRadius` / `text_cmd` | 布局、状态、镶嵌 |
| `tess.rs` | 形状 → 顶点（圆角 / 环带 / 羽化 / 阴影）；纯几何 | 认识控件、读 `Ui` / `WidgetState` |
| `gpu_batch.rs` / `backend.rs` | 几何收集 + 切段 + 批次契约 | 认识控件 |
| `painter/` | **唯一的公开绘制入口**：`elem` / `seq` / `clip` 推进 + 原语（`panel` / `text` / `rect` / `shadow` / `grip`…） | 布局决策、跨帧状态 |
| `layout` / `hit` / `focus` / `edit` / `id` / `view` | 纯逻辑内核（**无 `Ui`、可单测**）：光标与结算 / 命中与遮挡 / 焦点链 / 文本编辑状态机 / ID / 裁剪分层 | 拿 `Ui`、画东西、持久状态 |
| `style.rs` / `theme_toml.rs` | 主题令牌 + 序列化 | 行为 |
| `state.rs` | **跨帧**状态（`UiState` + 9 个模块**视图**，见 §5.3） | 每帧事实（那在 `Ui`） |
| `ui.rs` + `ui/*`（`Ui`） | **每帧事实 + 编排**：录制序、容器 / 窗口、布局责任链、命中裁决、z-order、提交、诊断（子模块分工见 §5.7） | 实现控件**外观**（那是 `widgets/`）；push 原始绘制队列 / 调 `next_seq` |
| `widgets/` | **控件**：`Widget::ui` 协议、每控件状态、外观、交互语义；**只用公开面** | 读 `ui.theme` 字段 / 写引擎状态（`ui.theme.x = ..`）/ 调 `UiAdd::ui_mut()` / 碰 `tess` / `gpu_batch` / `painter.q` |
| `UiAdd` | 容器闭包内的便捷方法 | 被**控件**调用（`ui_mut()` 只属于容器包装：`Panel` / `Pack` / `Grid` / `Window` / `Scroll` / `FlexCtx` / `ViewCtx`，以及本身就是容器的 `MenuCtx` / `MenuBar`） |

依赖方向（单向，无环）：`widgets → ui → {painter, layout, hit, focus, edit, view, id, state, style}`；
`widgets` **不得**依赖 `tess` / `gpu_batch` / `backend`；`ui` 不得依赖 `wgpu`
（§5.5 的 `VertexP3U2C4` / `SpriteRect` 例外保留，理由见该节）。

**第三方控件能用的公开面**（白名单，详见 `docs/WIDGET_GUIDE.md`）：
`Widget` / `Response` / `Sense` / `Expansion` / `SizeConstraints` +
`allocate` / `allocate_mode` / `allocate_at` / `allocate_sense*` / `interact` / `culled` /
`note_placed` / `resolved_size` / `child_rect` + `theme()` / `state()`（9 个模块视图）/
`scale()` / `mouse_*()` / `key_down*()` / `hit_abs` / `register_focus` / `claim_press` /
`set_cursor` / `elem_hint` / `painter()` + `push_panel_like*` / `push_solid_rect` /
`push_border_rect` / `push_text_rect*` / `icon_at` / `image_at` / `push_resize_grip(_at)`。

### 2.5.2 责任判据（逐条引用；违反哪条就写哪条）

1. **纯 vs 有状态**：形状→顶点、命令→批次、值→布局各自纯；出现 `Ui` / `WidgetState` 即越界。
2. **每帧 vs 跨帧**：`Ui` 的字段是每帧重建的事实；`UiState` 是跨帧持久。别把跨帧值塞进 `Ui`（每帧重建 = 丢），也别把每帧值塞进 `UiState`（会残留）。
3. **公开面只暴露"语义"，不暴露"事实"**：新增公开 API 必须能写出"控件作者为什么需要它"。**"只有让某次搬运能编译"是唯一理由 ⇒ 拒绝**——要么把该件的**职责**判给正确的一层，要么在正确的一层补**语义化**原语（例：需要"播放头" ⇒ 给 `Painter` 补一个 push 原语，而不是公开 `Ui::next_seq`）。
4. **控件不得写引擎**：要传配置就给函数参数（`&InputStyle`），不许 `ui.theme.x = ..`。
5. **引擎不得画控件**：`ui/*` 里不得 push 原始队列 / 调 `next_seq`；要画就走 `Painter` 原语。
6. **`ui_mut()` 只属于容器**；控件走申请 / 交互 / 绘制原语。
7. **搬运提交只改结构、不改行为**：同一提交不夹带视觉 / 交互变更；行为改动单独提交并由 `--sim-*` 守着（15+1 个仿真是"零行为变化"的证据）。

### 2.5.3 现状越界（2026-xx 审计，实测 `file:line`）与处置

| # | 现象 | 证据 | 判据 | 处置 |
|---|---|---|---|---|
| 1 | 控件读 `ui.theme` **字段**（crate 私有）而不是公开的 `ui.theme()` | 曾 ~36 处读 + 9 处 `&ui.theme`（`menu.rs` / `colorpicker*` / `dropdown.rs` / `checkbox.rs` / `segmented.rs` / `label.rs` / `numberinput.rs` / `slider.rs` / `fontmodal.rs` / `texteditor.rs` / `button.rs` / `divider.rs`） | 3 | ✅ **已修**：全部改成 `ui.theme()`（同一事实只留一条入口；`texteditor.rs` 的**写入**属 #2） |
| 2 | 控件**写引擎帧内主题**把样式传给引擎里的核心 | `texteditor.rs:377` `mem::replace(&mut ui.theme.input, style)`、`:393` 还原 | 4 | 核心改为收 `&InputStyle` 参数（D3 第一步）——**尚未修**，注释已指向本条 |
| 3 | 控件调 `UiAdd::ui_mut()` | `fontmodal.rs:148/195`（要 `child_rect` / `cursor_pos` / `wrap_buffer` / `push_*`） | 6 | **记为 D4 已知缺口**：`UiAdd` 目前不提供这些"组合控件自己排版"的原语；本轮只清理 `ui.theme`（#1），要把它们做成公开 compose 面是独立一轮。`MenuCtx` / `MenuBar` 的 `ui_mut` 是**容器**实现 ⇒ 白名单 |
| 4 | 控件读引擎几何事实表 | 曾 `menu.rs:377` `ui.state().window_rects` | 2/3 | ✅ **已修**：`UiState::windows()` 模块视图新增只读 `rects()`（与 `debug_dump` 同口径），`menu.rs` 改用它 |
| 5 | 引擎自己画控件外观（原始队列 + `next_seq`） | `ui/controls.rs::draw_check_common` | 5 | 改走 `Painter` / 公开矩形原语（D2 第一步） |
| 6 | 文本编辑核心（~1000 行）住在引擎侧 | `ui/textedit.rs::text_input_core` / `text_area_impl` | 5 | 搬进 `widgets/texteditor.rs`（D3） |
| 7 | 公开绘制面不完整：`Ui::push_draw` 与 `ellipsized` 是 `pub(crate)` | `ui/prims.rs::push_draw` / `ui/prims.rs::ellipsized` | 3 | **记为已知缺口**：第三方"组合控件"（如 `ColorPicker` 那种自绘面板）目前写不出来；补公开面是独立一轮（D4），不在搬运算内 |

### 2.5.4 边界守卫（机器可查）

`crates/rjw_ui/src/ui/tests.rs` 的 `widget_boundary_guard`：对每个 `widgets/*.rs` 用
`include_str!` 断言**不含**禁用子串（`painter.q` / `next_seq` / `ui.theme.` / `ui_mut()` /
`crate::tess` / `crate::gpu_batch`），白名单逐条给出理由（容器实现、`ColorPicker` 的
`push_draw` 等）。它守"明显越界"，失败信息直接指向本节与判据编号。

---

## 4. 受控实验：顶点到底花在哪

`tess.rs` 的圆角矩形是 **CPU 镶嵌**的同心轮廓扇形 + 羽化带：
顶点数 = `2 × 4(segs+1) + 1`，`segs ∈ {2,4,8,16,32}`（由半径与 `ARC_STEP_PX` 决定）
⇒ **一个圆角矩形 = 25 / 41 / 73 / 137 / 265 个顶点**。

只把 `ARC_STEP_PX` 从 `2.0` 改到 `4.0`（弧距粗一倍，段数减半，其余全不动）：

| 指标 | 基线 | 粗弧距 | Δ |
|---|---|---|---|
| `verts` | 23988 | 15192 | **−37%** |
| `tris` | 31913 | 18603 | **−42%** |
| `clone` | 169µs | 131µs | −22% |
| `submit·asm` | 165µs | 128µs | −23% |
| `submit·flush` | 125µs | 82µs | −34% |
| `finish` | 570µs | 450µs | **−21%** |
| `encode`（GPU） | 830µs | 704µs | **−15%** |

**结论四**：**至少 37% 的顶点是圆角弧的采样点**，而且这只是"两倍弧距"——一个圆角矩形
若走 SDF（1 个 quad = 4 顶点）理论上是 6–66 倍的差。这部分顶点还**穿透整条链**：
每一份都要被复制 4 次、上传 1 次、并由 GPU 逐顶点处理（`encode` 同步降了 15%）。

**结论五**：`verts/cmd = 62`、`tris/cmd = 83`。对"7 扇窗、384 条命令"的界面，这是
10–20 倍的顶点放大。**这才是数量级的杠杆，而不是少一次 memcpy。**

---

## 5. 结构欠账与硬规矩（与性能无关，但决定可维护性）

### 5.0 单位纪律（规矩，不是欠账；API 实现者必读）
`Size<T>` / `Position<T>`（`draw.rs`）把"逻辑像素 / 物理像素"搬到**类型里**，但**编译器
不区分两者**——`Logical` 被当成物理像素用只是静默错位。所以这里有一条**必须遵守**的纪律
（正典在 `Size` 的 rustdoc；内置控件与用户自定义控件同一套）：

1. **解释必须显式**：把带单位值变成数值（布局 / 命中 / 绘制）只能用
   `to_physical(scale)` 或 `match`，且**紧邻**参数解包：`let w = w.into().to_physical(scale);`。
   禁止 `let w = w.into(); … w.0`（读 `.0` 时单位就丢了）。
2. **构造必须指名单位**：实现体内造值写 `Size::Logical(..)` / `Size::Physical(..)`，
   不许 `220.0.into()`。**主题值 / 跨帧持久化值 / 已乘过 DPI 的值一律 `Physical`**
   ——`Theme` 在 `Ui` 内部已被 DPI 预乘，控件复用主题值时必须显式声明物理像素。
3. **转发是唯一例外**：收 `impl Into<Size<..>>` / 存 `Option<Size<..>>` 的 setter 可以
   `Some(s.into())` 原样存调用者的选择（例如 `Button::font_size` 存 `Option<Size<f32>>`，
   到 `resolve()` 才 `to_physical`）。除此之外实现体内不出现隐式糖。

`From<f32>` / `From<Vec2>`（⇒ `Logical`）**保留**：它是给**调用点**的源码兼容糖
（`width(220.0)`、`radius(6.0)`），不是实现者的工具。它成立的前提是上面三条被遵守——
糖只负责"调用点少写几个字"，单位解释一律发生在 API 边界。

**兑现方式（本轮的证据）**：`crates/rjw_ui/src` 现有 **86 处** `to_physical` 全在边界
（`ui/*` 13 子模块 34 / `painter/prim.rs` 9 / `widgets/*` 21 / `draw.rs` 定义与单测 22）；
**没有**任何
`Size::from(..)`；`Size<..>` / `Position<..>` 类型的字段**全部**是 builder 的
`Option<Size<..>>`（转发例外），内部状态（`UiState` / `UiFrameState`）**不存单位**——
一律物理像素。新控件照 `WIDGET_GUIDE.md` §3 的写法写即自动合规。

### 5.1 两套控件 API 并存
`widgets/`（17 文件 3.7k 行，`Widget::ui` 单方法协议）与 `ui/controls.rs` +
`ui/textedit.rs` 的遗留
`*_at` 控件（`button_at` / `slider_at` / `checkbox_at` / `radio_at` / `combo` /
`text_input_at` / `text_area_impl` …）**同时对外暴露**，`UiAdd` 再用 ~330 行
一行转发把两边的名字对齐。同一能力有三条入口（`Ui::button` / `Ui::button_at` /
`widgets::Button`）。

> **本轮进展（B：行语义 + `RowBuilder`）**：`row` 里的高度约束过去是"**覆盖一切**"
> （`force_h_all`）——多行 `TextEditor` 被压成一行高、内容只能滚动。现在按**尺寸类**分开：
> `Widget::size_class()`（默认 `SingleLine`，**带默认实现 ⇒ 外部控件不破**）——
> 单行子项被**钉到**行的标准高（旧行为，文字中心线对齐不变），多行子项以它为**下限**、
> 可把行撑高；行自身的高度上下限由新的 `RowBuilder`（`min_h`/`max_h`/`height`/`gap`/`pad`）
> 设置，在 `Frame::settle_size` 末尾夹取。`menu_bar` / 窗口标题栏共用的 `force_h_all` 子项
> 全是单行 ⇒ 几何逐像素不变（`--sim-menu` / `--sim-chrome` 守着）。
> 新仿真 `--sim-ta-resize` 的现场（150% DPI）：编辑器 300×135 → 420×215（+120/+80）时
> **窗口高 292 → 372（同步 +80）**；行高 = 215（标准行高 39）⇒ 多行确实撑高了行；往左上
> 过拖后 = 210×39 = **默认下限**（拖不到 0）；`min_h(60)` 行里那个 `.height(90)` 的单行
> 控件被钉到 90 物理（= 60 逻辑）⇒ 单行标准高生效。

> **搬运配方（P2a，已开始执行）**：把遗留核心的实现体**逐块搬进 `widgets/<name>.rs`
> 的 `impl Ui<'_>` 块**——公开路径不变（`Ui::button_at` 还在），但 `ui.rs` 不再持有
> 实现；`impl Ui` 可以写在同 crate 任何模块，因此**不需要留一行转发**。
> 两条硬约束（本轮定的）：
> 1. 只能用 `Ui` 的**公开**方法（`widgets` 是 `ui` 的兄弟模块，私有字段 / 私有方法
>    不可见）。缺什么就**扩展公开面**并写清理由，不开 `pub(crate)` 后门；
> 2. 搬运提交里**只允许出现 `use` / 路径 / 位置变化**，行为改动单独提交——这样
>    15 个 sim 的数值断言就是"零行为变化"的证据。
>
> ⚠ **配额与边界由 §2.5 决定**：第 1 条"扩展公开面"只允许**语义化**扩展（要能写出
> "控件作者为什么需要它"）；**"只有让某次搬运能编译"是唯一理由 ⇒ 拒绝**——那说明该件的
> 职责判错了层，应该在正确的一层补原语（例：核心需要播放头 ⇒ 给 `Painter` 补 push 原语，
> 而不是公开 `Ui::next_seq`）。
>
> **本轮进展（A：TextEditor 行为修复）**：`TextEditor::allocate_rect` 过去只算默认尺寸，
> 而绘制用的尺寸由尺寸责任链解出（`resizable_text_*` 内部）⇒ **申请尺寸 ≠ 绘制尺寸**：
> 拖大后窗口不跟着长、后面的控件不动、框溢出父级。修法是公开 `Ui::resolved_size`
> （与既有的 `Ui::resize_handle` 配对：一个读、一个写）并让自动申请**先问责任链**；
> 同时把 `.resize(..)` 的默认下限从 `Vec2::ZERO` 改成 `(InputStyle::min_w,
> InputStyle::height)`（一行文字标准高，与 `Theme::row_h` 同一套标准）⇒ 拖不到 0。
>
> **本轮进展（P2a 切片 1）**：`button_at` / `button_at_styled` → `widgets/button.rs`，
> `combo` / `combo_at` → `widgets/dropdown.rs`；为此公开 `Ui::note_placed`
> （"显式 rect 也要算进容器尺寸"，自定义控件同样需要）。`ui.rs` **7901 → 7805 行**；
> 15 个 sim 全 `[OK]`（`--sim-*` 数值断言 + smoke，零行为变化）。
> 剩余：`slider_at_styled` / `slider_at_drag`（需公开 `next_seq` 之类的绘制原语或在
> `Painter` 上补公开 push）、`checkbox_at_styled` / `radio_at` / `draw_check_common`
> （深触 `painter.q` 原始队列）、`radio_groups` 读写的语义化 API，最后是
> `text_input_core` + `text_area_impl`（~1000 行，见 §5.2）。

> **进展（更早）**：**菜单栏**的触发器交互并入 `widgets` 协议（`allocate_sense` +
> `Response`，不再手写 `hit_abs` + `update_interact`），且 `MenuBar` 现在 `Deref` 到
> `Pack` —— 与 `MenuCtx` `Deref` 到 `Window` 同一套模式（栏里能塞 `UiAdd` 的任何控件）。
> 同时 `Divider` 增加 `axis` / `vertical()`（公开 `DividerAxis`），并新增**独立的主题样式组**
> `Theme::menubar: MenubarStyle`（栏底 / 触发器 / 竖分割线；不再借用 `Theme::button`——借用
> 会让菜单条看起来像一排按钮）。窗口外框的收起状态也支持**引擎托管**：
> `.collapsible(show, None)` ⇒ 状态进 `UiState::collapsed`（应用用
> `UiState::{is_collapsed, set_collapsed, toggle_collapsed}` 读写）。**`NumberInput` 也泛型化**
> 到 `T: SliderValue`（与 `Slider` 同一套类型；`SliderValue` 只新增**带默认实现**的方法 ⇒
> 外部自定义实现不破），拖拽数学改 `f64` + 十进制格点吸附（详见 `NumberInput` 模块文档）。
> 几处都是**加** API、没有新增"第三条入口"。

### 5.2 文本框没有搬进 `widgets/`
`ui/textedit.rs` 里 `text_input_core` + `text_area_impl` 约 **1000 行**（含 IME 候选框、选择、
剪贴板、换行、滚动条），是唯一没被提取的控件主体。`edit.rs` 已经把纯逻辑拆干净了，
但**绘制与交互仍在引擎侧**——这是 `ui/` 变胖的最大单一贡献者。

### 5.3 `Ui` 39 字段 / `UiState` 43 字段（其中 21 张 ID 表）→ **已按模块公开**
`Ui` 每帧重建，字段是"帧内事实 + 一次性覆盖 + 光标意图 + 责任链"的混合体；
`UiState` 里 `widgets / radio_groups / panel_pos / window_z / window_rects /
window_origins / grid_cells / text_buffers / window_quads / z0_quads / scrolls /
window_widths / window_sizes / window_heights / panel_sizes / sizes / window_fx /
debug_submit / debug_clip / widget_strs / folded` 共 **21 张以 ID 或 z 为键的表**（外加
`auto_pos` 这张 id→位置的自动级联表；再外加 `collapsed` / `hit_regions` 等同族集合）并存。
它们的字段注释都在解释"为什么不能用另一张表"（z vs id、帧 vs 跨帧、局部 vs 绝对）——
文档很完整，但这本身就是**状态空间过大的信号**。

> **本轮新增**：`folded`（**区块折叠状态**：绝对 ID → `bool` 的**表**，服务 `ui.foldable(..)`）
> 与窗口的 `collapsed` **刻意分开**：键空间通常不重叠，但语义不同（窗口收起改整窗尺寸与缩放
> 链路，区块折叠只是"本帧不录制正文"），合成一张表会让同名窗口 / 区块互相干扰、且任一侧都
> 无法独立演进。它是**表而不是集合**：曾经的 `HashSet`（只记"折叠"）无法表达"用户已经展开"，
> 于是 `open(false)`（默认折叠）的区块**点开后下一帧又折回去**；现在表里存**明确态**
> （`false` = 记住展开），`Foldable` 在首帧把 `open(..)` 的默认态落盘 ⇒ `is_folded` 与画面
> 从第一帧起同口径。新增一张表时**必须**一并加进 `reset_windows()`（单测
> `reset_clears_every_module_after_dirtying` 会当场抓出漏清）。

> **进展（C：模块化 + pub 化）**：`UiState` 的 42 个字段现在按关注点分成 **9 个模块**，
> 以**公开只读视图**的形式对外（`state.frame() / widgets() / windows() / texts() /
> scrolls() / popups() / hits() / caches()`，外加 `stats()`）；**写**走语义化方法
> （`sizes_mut()` / `color_picker_mut()` / `set_menu_open()` / `set_combo_open()` /
> `close_popups()`），不把内部表暴露成 `pub` 字段。
>
> 为什么用视图而不是把**存储**拆成 9 个子结构：字段被引擎侧 ~200 处直接访问，拆存储会
> 牵动每一个调用点，而应用真正需要的是**稳定的公开面**——视图把公开面与内部布局解耦
> （以后内部怎么挪都不破坏应用），同时**模块边界就是文档边界**。
>
> 附带一条**真正的安全性修复**：`reset()` 改为逐个转调 `reset_frame/widgets/windows/
> texts/scrolls/popups/hits/caches`，于是"某模块新增字段却忘了清"不可能再发生——
> 新单测 `reset_clears_every_module_after_dirtying`（脏化 8 个模块 → `reset()` → 逐个
> 模块断言为空）**当场抓出旧 `reset()` 漏清 `widget_strs`**（以及漏清 `debug_submit` /
> `debug_clip` / `window_widths` / `window_heights` / scratch 缓冲），已一并修掉。
> 行为零变化：16 个 `--sim-*` 全绿。

### 5.4 `UiAdd` 的方法表是重复而非抽象
容器闭包需要 `&mut Ui`，所以 `Panel / Pack / Grid / Window / Scroll / FlexCtx` 各自
`impl UiAdd`，方法体一律是 `self.ui_mut().xxx(..)`。抽象只省了调用点的 `p.` 前缀，
**没有收敛任何行为**；代价是每个新 API 要写两遍（inherent + trait）。

### 5.5 "解耦"是名义上的
`UiBackend` / `UiBatch` 承诺"UI 只产出批次、不调渲染器"，但 `UiBatch::vertices` 的类型
`VertexP3U2C4` 直接来自 `rjw_2d_render`，`draw.rs` 也直接用 `SpriteRect`、
`debug_draw::thick_line_quad`。所以**"UI 不依赖渲染器"只在调用方向上成立**，
在数据格式上是耦合的——这正是 §6 里 L1 方案的门槛所在。

### 5.6 win=0 曾**没有自己的排序空间**（已在阶段 8 修掉，但教训值得留着）
提交序原为 `(win, elem, group, tex, clip)`，而 `elem = 0` 的语义是"画在**本容器**元素
之下"——**只有"每个容器一个排序空间"才成立**。窗口天然满足（一扇窗 = 一个 `win` 桶），
但**所有非窗口内容共享 `win = 0`**：`panel_impl` 的投影/底色与 `scroll_at` 的滚动条
（全仓仅这两处 win=0 装饰用 `elem = 0`）于是落进**整个 win=0 的最底**，被任何别的
win=0 内容穿透 ⇒ 用户看到的"可拖动面板闪烁 / ScrollBar 闪烁"。

修法是在 `win` 之后插入**顶层放置序** `place`（`z0_place_for_seq`），排序键变为
`(win, place, elem, group, tex, clip)`。**这条改造同时是 §6.2 的前提**：只有当"一个放置
= 一个有序单元"时，合并结果才能作为单元整体缓存。

教训（写进 `DEBUGGING.md` §8.3）：**排序键少一维时，症状是"闪烁"而不是"顺序乱"**，
而且只有内容在动的地方才看得见——所以先看**引擎自己的提交序**（`RJ_ORDER_TRACE`），
不要靠截图猜。

### 5.7 `ui.rs` 拆分（已落地）

`ui.rs` 曾 **8871 行**（全仓最大文件），现拆成**模块根 + 15 个子模块**；公开路径
（`rjw_ui::Ui` / `crate::ui::UiAdd` / `ui::WindowBuilder` …）逐字不变：

| 子模块 | 行数 | owns |
|---|---|---|
| `ui/facade.rs` | 511 | 状态 / 主题访问、帧级事实搬运、光标意图、焦点注册、诊断（`debug_dump` / `window_order` / `window_under_mouse`） |
| `ui/prims.rs` | 786 | 绘制原语（调试图元 / 圆角 / 渐变 / 图标 / 背景图 / panel / shadow / grip）、文本度量与排版缓冲缓存、`label_at` / `label_wrap_at`、`From<Align>` |
| `ui/interaction.rs` | 625 | 命中（`hit_abs` 家族）、申请（`allocate*`）、`interact`、`view_at`、容器原语、`avail_w`、`divider_at` |
| `ui/scroll.rs` | 632 | ScrollView（`scroll_at` / `scroll_axes_at` / `list_at`）、滚动条 + 滚动几何纯函数 |
| `ui/panel.rs` | 308 | 面板（`panel_at` / `drag_panel_at` / `panel_impl`）、位置 / 尺寸责任链、`with_id` / `id_for` |
| `ui/window.rs` | 1106 | `window_impl` / `menu_bar` / `modal_impl` + 窗口几何纯函数（溢出策略 / 裁剪 / 限位） |
| `ui/chrome.rs` | 244 | 标题栏（条高 / 布局解算 / 绘制 / `RJ_CHROME_TRACE`） |
| `ui/containers.rs` | 750 | `UiAdd` trait + 容器类型 + 诊断快照 + 容器入口（`pack_at` / `flex_at` / `grid_at` / `min_size` / `max_size`） |
| `ui/builders.rs` | 459 | `WindowBuilder` / `PanelBuilder` / `ModalBuilder` / `RowBuilder` / `WindowChrome` + builder 入口 |
| `ui/controls.rs` | 565 | 遗留 `*_at` 控件（滑块 / 勾选 / 单选 / 勾选绘制 / 缩放柄 / 文本框包装） |
| `ui/textedit.rs` | 1037 | 文本编辑核心（单行 + 多行：选择 / 剪贴板 / IME / 换行 / 滚动条） |
| `ui/commit.rs` | 876 | `finish` / `end_frame` / 提交单元与计划缓存 / 批次冲刷 / Debug 叠加 / 光标定夺与复位 |
| `ui/cmds.rs` | 673 | 命令 → 几何（镶嵌 / 内容签名 / 字形四边形）+ 拖拽激活与键盘帧末 |
| `ui/foldable.rs` | 516 | **可收缩区块**（`Foldable` / `FoldState` / `Ui::foldable` / `Ui::foldable_custom`）+ 标题行几何与翻转判据纯函数（`fold_state` / `fold_should_toggle` / `fold_header_w` / `fold_icon_rect` / `fold_label_rect` / `fold_guide_rect`）+ **标题=标准容器**（自定义标题在"固定宽 / 高度自然"的装饰容器里跑，`Ui::ornament_entry_natural_h` 返回内容高、标题行据此加高）+ 正文缩进与引导线 / 渐隐 + `RJ_FOLD_TRACE` 诊断 |
| `ui/namespace.rs` | 105 | **ID 命名空间区块**（`Namespace` / `Ui::namespace`）+ 结算纯函数（`namespace_size`） |
| `ui.rs`（根） | 739 | 常量、`UiInit`、`Ui` 结构、`begin` 与帧状态搬运、子模块声明与重导出 |

**本轮定的三条规矩**：

1. **子模块私有 + 根重导出**：`mod` 全部私有，公开类型经 `ui.rs` 的 `pub use` /
   `pub(crate) use` 重导出（`UiAdd` / `WindowBuilder` / `Panel` / `WindowChrome` …）
   ⇒ `crate::ui::X` 与 `rjw_ui::X` 两条路径都不变，`lib.rs` 的 `pub use` 无需改动。
2. **可见性只用 `pub(super)`**：`impl Ui` 可以写在同 crate 任何模块（§5.1 的搬运配方），
   子模块是 `crate::ui` 的后代 ⇒ 仍直接读写 `Ui` 的私有字段；真正需要跨子模块共享的
   只有 **42 处 `fn` / `const`**（外加 25 处需跨文件构造的结构体字段），它们提到
   **`pub(super)`**（= `pub(in crate::ui)`），**不开 `pub(crate)` 后门**，公开面一个
   字节没变。收敛方式：搬运时先统一提升，再全部降为私有、按编译器报错逐项提升——
   两轮收敛到 0 错误（`cargo check --all-targets`）。
3. **搬运提交只改结构**：拆分前后 `fn` 定义 **284 = 284**、类型 / 常量 **36 = 36**
   （按名字与重数逐项相同，脚本核对）；覆盖校验 = 8871 行中 8870 行按行搬入子模块，
   唯一"丢弃"的是 `impl Ui<'_> {` 那一行（由包装代替）；`cargo test -p rjw_ui` 与
   拆分前基线同为 **352 通过 / 3 失败**（那 3 个 `style.rs` 失败在拆分**前**就存在，
   与本次无关），`cargo clippy --workspace --all-targets` 零警告；**22 个 `--sim-*`
   全 `[OK]`**（含 `--sim-clip` / `--sim-cover` / `--sim-dropdown` / `--sim-menu` /
   `--sim-resize` / `--sim-tuner` / `--sim-weight-modal` 的数值断言）。

> **本轮新增控件（`LabelEx`）**：`crates/rjw_ui/src/widgets/label_ex.rs` —— 高度自定义标签
> （整组 `TextStyle` + 字段级覆盖 + **首末两色渐变**）。它**只走公开面**（判据：
> `widget_boundary_guard` 的 `FILES` 已补 `label_ex.rs`）：`Ui::cache_buffer_styled` /
> `Ui::text_size_styled`（本轮由 `pub(crate)` 提为 `pub` 的"控件作者公开面"，与
> `text_size` / `wrap_buffer` 同级）、`Ui::push_text_rect_ramp`、`crate::edit::ellipsize`；
> 不读 `ui.theme.` 字段、不碰 painter 队列 / `tess` / `gpu_batch`。支撑改动落在**引擎侧**：
> 绘制命令 `DrawKind::Text` 增 `ramp: Option<TextRamp>`（`TextRamp` = 首末两色 + 轴 + **域**
> `Glyph` / `Line` / `Frame`；逐字形四角取顶点色，**域与采样点都在文本视觉原点系**、
> 采样用**未裁剪**字形几何 ⇒ 裁剪不改变颜色；⇒ `cmd_sig_hash` **必须**一起哈希）、
> 排版缓存键由五元组提为 `state::TextKey`（有名字段：字号 / 行高 / 行距 / 字体族 / 字重 /
> 斜体 / 拉伸 / 字距 / 换行宽 / 版本；**颜色与对齐不进**——它们属绘制期）。改键的动机与
> §5.6 的教训同源：**少一个字段 = 改了样式不刷新**。

---

## 6. 决策

### 6.1 明确不做

| 候选 | 实测收益 | 不做的理由 |
|---|---|---|
| **去掉缓存克隆（借用重构）** | `clone` 169µs | 见 6.2——**换个做法能顺带把它和 `asm` 一起吃掉**，单独的借用重构是走弯路 |
| **切分 `z0 group 0` 缓存槽** | 0（稳态 `cache_miss=0.0`） | 上一轮残留的 0.4 次/帧在长窗口下已消失，问题不存在 |
| **⑥ GPU 持久缓冲** | — | 已按既定范围移除 |
| **微优化（单组 move / scratch 复用 / `sort_unstable`）** | 落在噪声内 | 已做完并如实记为"无可宣称收益"，不再重复 |

### 6.2 **已落地（阶段 9）**：缓存"提交计划"而不是"几何碎片"

> **前置（阶段 8）**：`place`（顶层放置序）作为排序维度。没有它，一个 **win=0 放置**
> 的几何就无法用**单元级 key** 表达序——`elem` 在 win=0 里跨放置交错，
> 合并成"计划"之后序就丢了。见 §5.6。

改之前的缓存粒度是 `(win, place, elem, group, tex, clip) → Geom`，于是**每帧都要重新合并一次**：
命中 → 逐条克隆（C1）→ 组装 `ordered` → 排序 → `segment_runs` → 跨条 `append`（C2）→ 再交后端。

但合并结果只取决于**帧内稳定的排序键** `(place, elem, group, tex, clip)` 与 `MAX_UI_SEG_VERTS`，
而段永远不跨 `(win, place)` ⇒ **合并结果本身可以缓存**：

```text
window_quads[id]        : (sig, Vec<BatchPlan>)      // 窗口 = 一个 place
z0_quads[(segment, place)] : (sig, Vec<BatchPlan>)   // win=0 的一个放置
BatchPlan { texture, clip, geom: Geom, elements: Vec<u32> }
```

命中路径变成：`HashMap::remove(&key)`（**move 一个 Vec 指针，O(1)、零顶点拷贝**）→
按 `(win, place)` 排一遍**单元**（~15 个）→ 逐个 `flush_seg(&plan)` → 再 `insert` 回去。于是：

- C1（`clone`，实测 169–194µs）**消失**；
- C2 的 `append` 与 `ordered`/排序/切段**消失**（只在 miss 时做一次）；
- `sort`（分桶）与 `sig` 保留（那是索引，不是几何）；
- 上一轮担心的"零拷贝两阶段读取导致整窗不提交闪烁"**不再适用**：数据是 `remove`
  出来的**所有权值**，z→id 映射只解一次，不存在"按旧 z 读新表"。

代价：`UiBatch` 的顶点/索引必须改**借用**（`&[VertexP3U2C4]` / `&[Tri]`），否则命中路径
又要在后端边界复制一份；`segs`（批次候选数）会小幅上升 ⇒ **真实 draw call 同步上升**
（实测 40 → 44，`UiBatch` 数 = UI 层 `draw_op_count()`）。⚠ 一度以为 `Render2D` 会把相邻
同状态批次合回去——**实测不会**：每个 `mesh_indexed(..)` 带自己的矩阵下标（`Mesh { mat_idx }`），
而动态段合批条件含 `dyn_seg_mat` ⇒ UI 批次之间永不合并（`draw_ops == segs` 坐实）。
这是"用 +10% 的 draw call 换掉每帧两次全量顶点 memcpy"的自觉取舍。

**实测（同机交错 A/B，3 轮 × 480 帧，取中位数）**：

| 指标 | 改前（HEAD） | 改后 | 说明 |
|---|---|---|---|
| `finish` | 0.68ms | **0.36ms（−47%）** | |
| `clone` | 193.6µs | **0.0µs** | C1 消失 |
| `asm` | 169.6µs | **22.8µs** | C2 的排序/切段/`append` 消失 |
| `submit` | 345.1µs | 244.2µs | = asm + flush（`flush` 因 44 次批次略涨） |
| `segs` / `draw_ops` | 40 / 40 | 44 / 44 | **+4 draw call（+10%）** |
| `verts / tris` | 23936 / 31887 | 23936 / 31887 | **逐位不变** |

**渲染结果逐像素不变**（同一套几何、同一套切段规则、同一套 scissor），16 个 `--sim-*`
全部通过。风险集中在缓存生命周期——这次的"零拷贝"是**所有权值**（`remove` → 提交 →
`insert`），不是"两阶段按 z 读同一张表"，所以历史上那类"整窗不提交"的竞态结构上不存在。

### 6.3 记账不实施：**SDF / 实例化圆角**（L1）

§4 证明顶点量级才是数量级杠杆（37%+ 的顶点是弧采样）。但：
1. 需要渲染器提供圆角 + 抗锯齿（新着色器或新顶点/实例格式）——**这违反 `tess.rs`
   开头写明的"零着色器改动"不变量**，属于跨 crate 的路线变更；
2. 圆角 + 四角渐变 + 四角各自半径 + 环带边框 + 软阴影都要重新实现；
3. `VertexP3U2C4` 的 `pos[2]` 空着、`uv` 已被白纹理/字形占用，**现有顶点格式装不下**
   （圆心/半宽/圆角至少还要 3 个分量）。

所以本轮只把它记为**唯一的方向性选项**，等 6.2 落地后再评估。

### 6.4 可维护性优先级（不阻塞性能，但建议排进近期）

1. **把 `ui/textedit.rs`（`text_input_core` / `text_area_impl`，~1000 行）搬进
   `widgets/textinput.rs`**——收益最直接（`ui/` 掉约 12%），风险低（纯搬家 + 保持
   `Ui::text_input_at` 为薄包装）。
2. **收敛控件 API**：遗留 `*_at` 保留为薄包装，但把"实现"统一到 `widgets/`，
   让 `UiAdd` 只剩一层。
3. **`Ui` 字段分块**：把"帧内事实 / 一次性覆盖 / 光标意图"拆成内嵌 struct，
   39 字段降到可读量级（纯重构，不改行为）。

---

## 7. 复现

```powershell
$env:CARGO_TARGET_DIR="C:\rust-targets\"
# 稳态基线（120 帧滑窗，取后 3 个）
cargo run -p eg260818UI --offline -- --frames 520
# 顶点归因实验：crates/rjw_ui/src/tess.rs 的 ARC_STEP_PX 2.0 → 4.0（测完务必改回）
# 门禁
cargo test --workspace
cargo clippy --workspace --all-targets
cargo run -p eg260818UI --offline -- --frames 30 --sim-clip
```

`[perf]` 里 `submit=…(asm=… flush=…)` 是本次为做这份分析新加的**两笔账**
（`UiStats::submit_asm_us` / `submit_flush_us`）：前者是 `rjw_ui` 自己的顶点装配，
后者是 `flush_seg` → `UiBackend::submit`。**不分这两笔，就无法判断那 0.3ms 该算谁**——
上一轮把整块 `submit` 当成"一次拷贝"就是这么来的。
