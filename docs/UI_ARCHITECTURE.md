# `rjw_ui` 架构分析（含成本模型与取舍决策）

> 目的：在动下一刀之前，先把 `rjw_ui` 的**分层、数据流、真实成本、结构欠账**摊开，
> 让"接下来做什么"是一个有依据的决定，而不是上一轮结论的惯性延续。
>
> 数据来源：`examples/eg260818UI --frames 520`（稳态 120 帧滑窗均值）+ 一次
> **受控实验**（只改一个常量）。复现命令见文末。**未提交任何估算值**——每一行都是实测。

---

## 1. 分层与依赖方向

```text
应用（examples / 游戏 / 编辑器）
   │  UiAdd（容器闭包内 &mut Ui 的方法表）/ Ui::*  →  add/add_at(Widget)
   ▼
┌─ rjw_ui ────────────────────────────────────────────────────────────────┐
│ ui.rs            Ui：录制 + 布局 + 命中 + 帧级结算 + 提交编排              │
│                  7467 行 / 125 pub fn / 39 字段                          │
│ widgets/         Widget::ui 单方法协议 + 属性 builder（17 文件 4.1k 行）  │
│ ui.rs[5646..]    遗留 *_at 控件（button_at/slider_at/text_input_at/…）     │
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

## 5. 结构欠账（与性能无关，但决定可维护性）

### 5.1 两套控件 API 并存
`widgets/`（17 文件 3.7k 行，`Widget::ui` 单方法协议）与 `ui.rs[5646..]` 的遗留
`*_at` 控件（`button_at` / `slider_at` / `checkbox_at` / `radio_at` / `combo` /
`text_input_at` / `text_area_impl` …）**同时对外暴露**，`UiAdd` 再用 ~330 行
一行转发把两边的名字对齐。同一能力有三条入口（`Ui::button` / `Ui::button_at` /
`widgets::Button`）。

### 5.2 文本框没有搬进 `widgets/`
`ui.rs` 里 `text_input_at` + `text_area_impl` 约 **1000 行**（含 IME 候选框、选择、
剪贴板、换行、滚动条），是唯一没被提取的控件主体。`edit.rs` 已经把纯逻辑拆干净了，
但**绘制与交互仍在 `ui.rs` 中央**——这是 `ui.rs` 变胖的最大单一贡献者。

### 5.3 `Ui` 39 字段 / `UiState` 39 字段（其中 20 张 ID 表）
`Ui` 每帧重建，字段是"帧内事实 + 一次性覆盖 + 光标意图 + 责任链"的混合体；
`UiState` 里 `widgets / radio_groups / panel_pos / window_z / window_rects /
window_origins / grid_cells / text_buffers / window_quads / z0_quads / scrolls /
window_widths / window_sizes / window_heights / panel_sizes / sizes / window_fx /
debug_submit / debug_clip / widget_strs` 共 **20 张以 ID 或 z 为键的表**并存。
它们的字段注释都在解释"为什么不能用另一张表"（z vs id、帧 vs 跨帧、局部 vs 绝对）——
文档很完整，但这本身就是**状态空间过大的信号**。

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

---

## 6. 决策

### 6.1 明确不做

| 候选 | 实测收益 | 不做的理由 |
|---|---|---|
| **去掉缓存克隆（借用重构）** | `clone` 169µs | 见 6.2——**换个做法能顺带把它和 `asm` 一起吃掉**，单独的借用重构是走弯路 |
| **切分 `z0 group 0` 缓存槽** | 0（稳态 `cache_miss=0.0`） | 上一轮残留的 0.4 次/帧在长窗口下已消失，问题不存在 |
| **⑥ GPU 持久缓冲** | — | 已按既定范围移除 |
| **微优化（单组 move / scratch 复用 / `sort_unstable`）** | 落在噪声内 | 已做完并如实记为"无可宣称收益"，不再重复 |

### 6.2 采纳：**缓存"提交计划"而不是"几何碎片"**（唯一剩余的结构性收益）

> **前置已落地（阶段 8）**：`place`（顶层放置序）作为排序维度。没有它，一个 **win=0
> 放置** 的几何就无法用**单元级 key** 表达序——`elem` 在 win=0 里跨放置交错，
> 合并成"计划"之后序就丢了。见 §5.6。

现在的缓存粒度是 `(win, place, elem, group, tex, clip) → Geom`，于是**每帧都要重新合并一次**：
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

- C1（`clone` ~140µs）**消失**；
- C2 的 `append` 与 `ordered`/排序/切段**消失**（只在 miss 时做一次）；
- `sort`（分桶）与 `sig` 保留（那是索引，不是几何）；
- 上一轮担心的"零拷贝两阶段读取导致整窗不提交闪烁"**不再适用**：数据是 `remove`
  出来的**所有权值**，z→id 映射只解一次，不存在"按旧 z 读新表"。

代价：`UiBatch` 的顶点/索引必须改**借用**（`&[VertexP3U2C4]` / `&[Tri]`），否则命中路径
又要在后端边界复制一份；`segs`（批次候选数）会小幅上升，但**真实 draw call 不变**
（`Render2D` 会把相邻同 `(rstates, texture, transform, scissor)` 的 `Mesh` 合成一个动态段）。

预期：`finish` 砍掉 C1+C2 ≈ **−300µs**，**渲染结果逐像素不变**
（同一套几何、同一套切段规则、同一套 scissor）。风险集中在缓存生命周期，
而本项目有 16 个 `--sim-*` + 291 个单测 + `--sim-clip` / `--sim-zorder` 可以钉住它。

### 6.3 记账不实施：**SDF / 实例化圆角**（L1）

§4 证明顶点量级才是数量级杠杆（37%+ 的顶点是弧采样）。但：
1. 需要渲染器提供圆角 + 抗锯齿（新着色器或新顶点/实例格式）——**这违反 `tess.rs`
   开头写明的"零着色器改动"不变量**，属于跨 crate 的路线变更；
2. 圆角 + 四角渐变 + 四角各自半径 + 环带边框 + 软阴影都要重新实现；
3. `VertexP3U2C4` 的 `pos[2]` 空着、`uv` 已被白纹理/字形占用，**现有顶点格式装不下**
   （圆心/半宽/圆角至少还要 3 个分量）。

所以本轮只把它记为**唯一的方向性选项**，等 6.2 落地后再评估。

### 6.4 可维护性优先级（不阻塞性能，但建议排进近期）

1. **把 `text_input_at` / `text_area_impl`（~1000 行）搬进 `widgets/textinput.rs`**——
   收益最直接（`ui.rs` 掉 13%），风险低（纯搬家 + 保持 `Ui::text_input_at` 为薄包装）。
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
