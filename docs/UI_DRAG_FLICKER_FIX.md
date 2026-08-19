# rjw_ui 拖动闪烁修复 + 性能测量与优化记录

> 2026-02 · eg260818UI（RTX 5070 笔记本，165Hz 显示器，Windows / wgpu D3D12）
> 涉及改动：`Cargo.toml`（dev profile 优化）、`crates/rjw_ui/src/{state,ui,draw,lib}.rs`、
> `examples/eg260818UI/src/main.rs`（计时器 + `--auto-drag`）。

---

## 一、闪烁 BUG：根因与修复

### 根因（窗口顶点缓存"重建帧不提交"）

`Ui::finish()` 对 `window_at` 窗口做**内容签名 → 顶点缓存**：内容不变（签名命中）时直接
复用缓存的**窗口局部顶点**（移动窗口只改提交变换，顶点不重建）。修复前的未命中分支：

```rust
// 未命中：收集该窗口命令为局部顶点，写入缓存
for ((_, elem, g, tex), verts) in q.quads {
    grp.push((elem, g, tex, verts));   // 只进缓存，没进本帧提交列表！
}
self.state.window_quads.insert(id, (sig, grp));
```

新顶点只写入 `state.window_quads`，**没有加入本帧的提交列表 `cached`** → 该窗口这一帧
一个顶点都不画（整窗消失 1 帧）；下一帧内容稳定 → 缓存命中 → 窗口出现。于是
"内容一变 → 消失一帧 → 出现 → 再变 → 再消失" = **"消失与显示瞬间交替"闪烁**。

触发场景（全是"内容逐帧变化"）：
- 拖动含输入框的窗口，**光标闪烁**（每 30 帧切换）→ 每 30 帧消失/出现；
- 拖动中 hover / 滚动 / 动画 → 内容每帧变 → 拖动期间整窗不可见；
- 窗口首次创建（缓存冷启动）→ 弹出前闪一帧。

（此前另有一个已修复的问题：早前"零拷贝两阶段读取"在窗口 z / id 映射变化时有整窗
不提交的竞态，现改为克隆后已消除。）

### 修复

未命中分支**缓存一份 + 本帧提交一份**（`grp` 存克隆、`cached` 收原顶点）：

```rust
for ((_, elem, g, tex), verts) in q.quads {
    grp.push((elem, g, tex, verts.clone()));
    cached.push((win, elem, g, tex, verts));
}
```

不变量：**无论命中/未命中，本帧每个录制的窗口都提交**。重建帧多一次窗口顶点克隆
（仅在内容变化帧发生，拖动中约 +50µs，可忽略）。

---

## 二、性能测量（计时器）

### 计时器挂载位置

1. **`rjw_ui` 库内**（`Ui::finish()` 分段，帧末写入 `UiState.stats`，类型 `UiStats`）：
   - `sort_us`：队列排序 + 按窗口分组
   - `sig_us`：窗口内容签名（摘要 + 全量哈希）
   - `collect_us`：缓存未命中 → 顶点重建（`collect_cmds`，含文本整形）
   - `clone_us`：缓存命中 → 提交列表组装（顶点克隆）
   - `submit_us`：提交（ordered 排序 + `add_quads` 循环）
   - `finish_us` / `ui_frame_us`（begin → finish 结束）
   - 计数：`cmd_count` / `win_count` / `cache_hit` / `cache_miss`
2. **示例 `eg260818UI`**（stdout 每 120 帧打印一行 `[perf]`）：
   - 整帧 `about_to_wait` 耗时、UI 各阶段、渲染细分
   - `render` 细分：`begin`（`begin_frame`/交换链 acquire）、`encode`（2 个
     `render_command_buffer`）、`submit`、`present`
   - `--auto-drag`：自动圆周拖动 win_b + 每帧改内容（等价"拖动中内容逐帧变化"最坏路径）

### 测量结果（每帧均值）

| 场景 | fps | frame | ui | finish | collect(miss) | render 细分（begin/encode/submit/present） |
|---|---|---|---|---|---|---|
| debug O0 静态（改前） | ~95 | 10.0ms | 4.5ms | 4.0ms | 0~450µs | 5.0ms（未细分） |
| debug O0 拖动（改前） | ~88 | 13~18ms | **8~12ms** | 4.2~5.7ms | **560~900µs** | 4.5ms（未细分） |
| release 静态（改前） | 165 | 5.2~6.0ms | 0.5ms | 0.35ms | ~0 | 4.3~5.5ms（未细分） |
| release 拖动（改前） | ~170 | 5.2~5.7ms | 0.9~1.1ms | 0.45~0.55ms | ~55µs | 4.0~4.8ms（未细分） |
| **debug O2 静态（改后）** | **165~190** | 5.0~5.7ms | 0.5~0.8ms | 0.37~0.42ms | ~7µs | 2.7 / 1.1 / 0.7 / 0.5ms |
| **debug O2 拖动（改后）** | **165~168** | 6.0ms | **0.73ms** | 0.45ms | **60µs** | 2.7 / 1.1 / 0.8 / 0.5ms |
| **release 静态（改后）** | 165 | 6.0ms | 0.42ms | 0.32ms | ~0 | **4.5** / 0.37 / 0.25 / 0.37ms |
| **release 拖动（改后）** | 165 | 6.0ms | 0.63ms | 0.39ms | 50µs | **4.2** / 0.38 / 0.28 / 0.4ms |

### 结论

1. **debug O2 已追平 release**：`finish` 4.0ms → 0.4ms、拖动录制 8~12ms → 0.73ms，
   fps ~90 → 165（显示器 vsync 上限）。游戏 Debug 不再"没法用"。
2. **rjw_ui CPU 不再是瓶颈**：release 下整个 UI 帧（begin→finish）< 0.7ms。
3. **帧瓶颈在交换链 vsync 等待**：`render.begin_frame`（`AutoVsync`/Fifo）约 4.2~4.5ms，
   是等待上一帧显示完的垂直同步节拍；encode+submit+present 合计仅 ~1ms。**这是预期行为，
   不是可优化的 CPU 开销**——想解锁更高帧率/更低延迟可改用 `Mailbox`/`AutoNoVsync`
   （`RenderConfig.vsync`），代价是可能撕裂。
4. 拖动（内容每帧变化）的最坏路径：miss 重建 + 提交 ≈ 60µs（debug O2）/ 50µs（release），
   对 165Hz 无压力。

---

## 三、本轮落地的优化

### 1. dev profile 开编译期优化（保留调试信息）—— `Cargo.toml`

```toml
[profile.dev]
opt-level = 2
debug = true
```

- `opt-level = 2`：debug 运行速度接近 release（本库实测 UI 提交路径快 ~10 倍）；
- `debug = true`：保留完整调试信息（行号/变量/类型），断点/单步不受影响；
- `overflow-checks` 保持默认开启（debug 专属，release 关闭）。
- 代价：首次全量重编变久；若更看重单步可读性可降 `opt-level = 1`。

### 2. 文本缓冲缓存抖动修复 —— `state.rs` / `ui.rs`

原策略：`TEXT_BUFFER_CACHE_CAP(128)` 满则**整表清空**。示例有 ~130 个唯一文本，
FPS/计数类标签每帧变化 → 缓存每帧全清 → **下一帧全部标签重新整形**（debug 下录制
耗时翻倍、拖动场景 ui 高达 8~12ms 的主因）。

新策略（帧级近似 LRU）：
- 值类型改为 `(Arc<Buffer>, 最后使用帧号)`；命中时刷新帧号；
- 超容量时**只驱逐本帧未使用**的条目（静态标签全部保留）；仍满则驱逐最旧一条；
- 容量提升至 256。动态文本不再冲掉静态缓存，消除每帧全量整形。

### 3. 性能计时器（可长期保留）

- `rjw_ui`：`UiStats` + `UiState.stats`（finish 各阶段 + 计数）；
- `eg260818UI`：`[perf]` 周期打印（整帧/UI/渲染细分）+ `--auto-drag` 压力场景。

---

## 四、后续可做（按性价比，未做）

| 项 | 预期收益 | 说明/风险 |
|---|---|---|
| PresentMode 策略（`Mailbox`/`AutoNoVsync`） | 解锁 >165fps / 降延迟 | 属渲染配置决策，非 rjw_ui；可能撕裂 |
| rjw_ui 摘要快速路径（跳过每帧全量 SipHash） | 静态帧 sig 15µs → ~2µs | 需防碰撞设计（含颜色/文本首尾采样）；收益小 |
| 窗口顶点缓存改 `Arc<Vec<...>>` 零拷贝 | 命中帧免 memcpy（当前 ~10µs） | 低风险，收益小 |
| `win=0` 非窗口内容顶点缓存 | 顶层静态面板/标签免每帧重建 | 需重构"局部顶点+窗口变换"语义（涉及可拖动面板） |
| 文本缓存键免每帧 String 分配 | 每帧省 ~200 次小分配 | 需与 `Buffer` 内容校验防碰撞，正确性敏感 |
| Render2D `add_quads` staging 拷贝 | 提交路径再省一次拷贝 | 渲染器侧，GPU 上传前必需，改动大 |

---

## 五、验证

- `cargo test -p rjw_ui`：54 单测 + 3 doctest 全过；
- `cargo build -p eg260818UI`（debug O2）/ `--release`：通过；
- 4 组场景（debug/release × 静态/`--auto-drag`）各跑 20s，数据见上文表格；
- 闪烁修复的判定：拖动（内容逐帧变化）时 `cache_miss=1` 帧仍提交绘制（代码路径
  `cached.push`），窗口不再周期性消失。

---

## 六、窗口/面板位置责任链（`Ui::pos_handler`）

位置解析从"固定两源（`panel_pos` ?? `pos`）"升级为**责任链**（见 `ui.rs`）：

```
应用脚本处理器（priority 降序，越大约先问）
  → 内置"用户拖拽状态" panel_pos（固定 priority 0）
  → 调用者传入的 pos（终端兜底，恒最后）
```

- `priority < 0`（如 `-10`）：动画/自动布局，**用户拖拽优先**——拖过就赢，松开停在
  用户放置处；未拖过时脚本"填空"提供位置；
- `priority > 0`（如 `+10`）：**脚本锁定位置**（切场景锁窗口/剧情镜头），脚本返回
  `None` 即交还控制权；
- 闭包须 **`'static`**（捕获拥有值 / `Copy` 值 / `Arc`）——避免 `Ui` 内存储带借用
  的闭包触发 dropck，把 `ui.finish()` 后的 `self` 访问都判为借用冲突（库回归风险，
  已用该约束规避）；
- 示例：`eg260818UI --script-pos`（脚本驱动 win_a 摆动 + 可拖动）。

验证：`pos_chain_resolves_by_priority` 单测覆盖"脚本优先级 → 拖拽优先 → pos 兜底 →
脚本按 id 选择性让位"四条语义；`--script-pos` 实跑 165fps 无异常。
