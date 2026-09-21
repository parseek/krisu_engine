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
│                  7805 行 / 123 pub fn / 39 字段                          │
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

**兑现方式（本轮的证据）**：`crates/rjw_ui/src` 现有 **70 处** `to_physical` 全在边界
（`ui.rs` 33 / `painter/prim.rs` 8 / `widgets/*` 18 / `draw.rs` 定义与单测）；**没有**任何
`Size::from(..)`；`Size<..>` / `Position<..>` 类型的字段**全部**是 builder 的
`Option<Size<..>>`（转发例外），内部状态（`UiState` / `UiFrameState`）**不存单位**——
一律物理像素。新控件照 `WIDGET_GUIDE.md` §3 的写法写即自动合规。

### 5.1 两套控件 API 并存
`widgets/`（17 文件 3.7k 行，`Widget::ui` 单方法协议）与 `ui.rs[5646..]` 的遗留
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
`ui.rs` 里 `text_input_at` + `text_area_impl` 约 **1000 行**（含 IME 候选框、选择、
剪贴板、换行、滚动条），是唯一没被提取的控件主体。`edit.rs` 已经把纯逻辑拆干净了，
但**绘制与交互仍在 `ui.rs` 中央**——这是 `ui.rs` 变胖的最大单一贡献者。

### 5.3 `Ui` 39 字段 / `UiState` 42 字段（其中 20 张 ID 表）→ **已按模块公开**
`Ui` 每帧重建，字段是"帧内事实 + 一次性覆盖 + 光标意图 + 责任链"的混合体；
`UiState` 里 `widgets / radio_groups / panel_pos / window_z / window_rects /
window_origins / grid_cells / text_buffers / window_quads / z0_quads / scrolls /
window_widths / window_sizes / window_heights / panel_sizes / sizes / window_fx /
debug_submit / debug_clip / widget_strs` 共 **20 张以 ID 或 z 为键的表**并存。
它们的字段注释都在解释"为什么不能用另一张表"（z vs id、帧 vs 跨帧、局部 vs 绝对）——
文档很完整，但这本身就是**状态空间过大的信号**。

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
