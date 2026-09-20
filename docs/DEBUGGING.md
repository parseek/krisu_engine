# 调试指南（Rust 代码级）

> 面向**引擎 / 应用代码**的调试（断点、日志、状态快照、脚本化复现）。
> 图形层专项（抓帧、像素、渲染目标导出）在 §6，但**优先用 §1–§5 的 Rust 侧手段**：
> 大多数"没画面 / 位置不对 / 拖不动"的根因在状态，而不在 GPU。

---

## 0. 门禁命令表（每次提交前全跑）

改动**任何** UI 相关代码（含"只改文档/注释"）都跑这套；**红一条就不算过**。覆盖
"编译 → 静态检查 → 单测 → 交互行为 → 冒烟"五条通路，缺一条就会出现"编译过、行为错"。

```powershell
# 0) 环境：本机产物不在 ./target
$env:CARGO_TARGET_DIR = "C:\rust-targets\"

# 1) 编译 / 静态检查 / 单测
cargo check  --offline --workspace --all-targets          # 期望 0 warning
cargo clippy --offline -p rjw_ui --all-targets            # 期望 0 warning
cargo test   --offline --workspace                        # 全绿（rjw_ui 基线：324 lib + 40 doc）

# 2) 交互行为：15 个脚本化仿真（判定打在 stderr，全 [OK] 才算过）
#    ⚠ 每个 sim 自己打印判定帧，`--frames` 只要"超过最后一个判定帧"；260 覆盖全部（现网最大 240）。
foreach ($s in "drag","picker","overlap","cover","chrome","weight","shadow","clip",
              "zorder","text-cull","tuner","menu","dropdown","weight-modal","resize") {
    cargo run --offline -p eg260818UI -- "--sim-$s" --frames 260
}

# 3) 冒烟：帧循环跑满 N 帧自动退出
cargo run --offline -p eg260818UI -- --frames 240   # => krusie smoke: [OK] 240/240
cargo run --offline -p egUI -- --frames 60          # => krusie smoke: [OK] 60/60

# 4) egUI 单窗口诊断（demo **默认全关** ⇒ 不带 --demo 的冒烟跑的是空屏）
cargo run --offline -p egUI -- --demo Gallery --ui-dump --frames 60
```

| 命令 | 守什么 |
|---|---|
| `check` / `clippy` / `test` | 公开面没坏、单测期望（几何 / 命中 / 文本 / 主题）没变、无新增告警 |
| 15 个 `--sim-*` | **交互行为**（拖拽 / 收起 / 遮挡 / 层级 / 裁剪 / 菜单 / 下拉 / 尺寸责任链…）；数值断言写在各 sim 的判定行里 |
| `--frames N` 冒烟 | 帧循环 / 资源生命周期（present 满 N 帧；退出路径无 panic） |
| `--demo … --ui-dump` | **指定代码路径真的被跑到**：`--ui-dump` 的窗口行 = 那个 demo 的窗口（`--demo` 打错 ⇒ 打清单 + **非 0 退出**，不静默） |

诊断专用（不进上表）：`--sim-import <图片>` / `--sim-theme <toml>` / `--sim-click X,Y`（配
`RJ_HIT_TRACE=1`）/ `--auto-drag` / `--script-pos` / `RJ_CHROME_TRACE` / `RJ_MENU_TRACE` /
`RJ_ORDER_TRACE=<frame>`。各 sim 的**判定口径与现场**见 §2–§5。

---

## 1. 先看状态：`Ui::debug_dump()` / `Ctx` 事实

引擎把"自己眼里的世界"暴露成可打印结构，**不要靠猜**。

```rust
// 应用侧（eg260818UI 的 `--ui-dump` 即此法）
let mut ui = f.ui(theme);
// ... 录制 UI ...
eprintln!("{}", ui.debug_dump());   // 或 log::info!(..)；**任一段**都能调
ui.finish();
```

> 一帧多段时：帧级暂存（窗口原点 / z / 拖拽状态）**跨段共享**，所以后一段的 dump 能看到
> 前一段录的窗口，且两段打印的 `frame=` **相同**（帧号每帧只 +1）。示例里的 `[段 1]` /
> `[段 2]` 两行就是这个对照。

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
| **裁剪没生效**（内容画到窗口/沙箱外） | `--ui-dump` 里该窗的 `clip=`；`[perf] ... clip_batches=` | `clip=None` 而该窗本该裁 ⇒ 强制层没建立（`Placement::Clip` / 固定高 / `view_at(Clip)`）或该批次没带 scissor；`clip_batches=0` ⇒ 裁剪完全没接上（渲染器/后端链路） |
| **裁剪位置不对**（裁多了/裁少了/整块不见） | `clip=` 的矩形 vs 该窗 `origin` + `size` | 不等 ⇒ `batch_scissor` 的映射错了（窗口 FX 变换 / anchor）；`clip` 为空 ⇒ 空 scissor 会**整条跳过**（不是"不裁"） |
| **圆角被切平**（窗口拖到视口边缘时边框变方） | 是不是把环境裁剪又写回几何切割了 | 环境裁剪应走 batch scissor（`UiBatch.clip`），几何保持原形——见 `docs/ENGINE_GUIDE.md` §18.20 |
| **draw call 变多** | `[perf] clip_batches=` 与 `cmds/wins` | 每个**不同** scissor 至少要一段（一次 draw 一个 scissor）；`clip_batches` 远大于窗口数 ⇒ 控件给自己套了太多层裁剪 |

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
- **"被上层窗口盖住的控件仍被触发"**（窗口遮挡与本帧几何不同步）：示例的 `--sim-cover`
  摆两扇窗口 + 一个拖拽探针（`examples/eg260818UI/src/cover.rs`），脚本化跑三段：

  ```
  cargo run -p eg260818UI -- --sim-cover --frames 80
  sim-cover[0 正对照]: 拖拽活着=14 帧（探针未被盖住，按住约 14 帧） [OK] 窗口内控件拖得动
  sim-cover[A 同帧移动]: 认领按下=2 / 被盖住却还在拖=0 帧 / 移动窗确实盖住探针=true [OK] 被盖住的控件没留下按下状态
  sim-cover[B 应用改 z]: 段 B 新增认领=0 / 累计被盖住却还在拖=0 帧 / 移动窗确实盖住探针=true [OK] 抬高 z 后背后的控件不再被触发
  ```

  - **段 0（正对照）**：移动窗让开 ⇒ 走"窗口内控件**必须拖得动**"这条正路。它守的是
    帧末复核**别误撤合法按下**（`resolve_widget_press` 若拿"认领时的旧 z"比，被点的窗口
    会被抬到 `max+1`，于是窗口**把自己**判成被遮挡 ⇒ 每次按下都被撤，拖拽只剩 1 帧 ⇒ FAIL）；
  - 段 A「同帧移动」：盖住探针的窗口**本帧才移过去**，而按下也在同一帧 ⇒ 命中那一刻
    本帧几何还没录完；
  - 段 B「应用改 z」：录制前把该窗口 z 抬到最前（与"点击置顶在帧末改 z"同类），
    本帧它画在探针之上，而遮挡表里还是旧 z；
  - 判定口径：`被盖住却还在拖` 必须是 `0` 帧（帧末复核把错误认领的按下压掉），
    段 B 的**新增认领**必须是 `0`（遮挡表按窗口 ID 跨帧存活后不该再误判）。
  `RJ_COVER_TRACE=1` 打印探针逐帧的 `hit / down_edge / dragging / window_under_mouse`。
  ⚠ 这段脚本依赖"注入只对下一帧生效"：几何变化与按下**必须同一帧**，故注入帧号与几何
  变化帧号要对齐（见 `main.rs` 里的注释）；每段之间要**复位两窗 z**，否则上一段的按下
  会把被点窗口置顶、下一段的前置条件就不成立了。
- **"改了字重 / 行距但画面不动"**（主题级文本令牌没进缓存键 / 几何签名）：示例的
  `--sim-weight` 在第 30 帧把全局字重从 400 切到 700，并在切换前后量同一串文本：

  ```
  cargo run -p eg260818UI -- --sim-weight --frames 50
  sim-weight: frame=20 weight=400 '字重 Aa 123' width=112.0 (label size 21)
  sim-weight: frame=40 weight=700 '字重 Aa 123' width=116.0 (label size 21)
  sim-weight: weight 400 → 700 : width 112.0 → 116.0 [OK] 字重真的改变了字形 / 步进宽度
  ```

  宽度不变（`[FAIL] 字重没进排版输入`）说明 `.weight(..)` 没落到建缓冲那条路——
  **A/B 已实测**：把 `cache_buffer_wrap` 里的 `.weight(weight)` 去掉，输出立刻变成
  `width 112.0 → 112.0 [FAIL]`。宽度变了但画面（按钮 / 输入框里的居中文本）仍是旧字形，
  则是**几何签名前缀**漏了字重——`Ui::hash_cmds` 与 `UiState::text_buffers` /
  `WidgetState::text_buf` 三处键都在 `docs/ENGINE_GUIDE.md` §18.7 列着。

- **"投影颜色调了没反应"**（色块 → `ShadowStyle::color` → 主题这条线断在哪）：
  示例的 `--sim-shadow` 在第 30 帧把投影换成半透明红，前后打印主题里的值：

  ```
  cargo run -p eg260818UI -- --sim-shadow --frames 50
  sim-shadow: frame=20 panel.shadow.color = (0.00,0.00,0.00,a0.47)     # 深色预设 rgba_u8(0,0,0,120)
  sim-shadow: frame=40 panel.shadow.color = (0.90,0.15,0.10,a0.55)
  sim-shadow: 主题投影色 已跟随色块变化 [OK] 阴影颜色进了主题
  ```

  主题值不变 = 应用没把色块喂进 `ShadowStyle`；主题值变了但画面颜色没变 = 镶嵌层把
  颜色丢了（引擎侧由单测 `tess::tests::shadow_keeps_the_callers_rgb_and_alpha` 守着：
  顶点 RGB 必须与调用方给的颜色逐位相同）。

- **"角落那个数字条点上去没反应"**（主题调节窗口的"滑杆 + 数字条"两件套）：示例的
  `--sim-tuner` 实测两件套，坐标**运行时解算**（窗口原点取 `ui.debug_dump()`，行内 x
  由主题尺寸 + 实测标签宽推出 ⇒ 换 DPI / 字体 / 密度档都不会点空）：

  ```
  cargo run -p eg260818UI -- --sim-tuner --frames 70
  sim-tuner: 拖数字条后 radius=18.0（期望 ~18 = 8 + 20px × step 0.5）· 主题圆角 tl=18.0 [OK] 数字条能改值，且进了主题
  sim-tuner: 数字条 18.0 → 拖滑杆到最左 0.0 · 主题圆角 tl=0.0 [OK] 滑杆与数字条绑同一个值
  sim-tuner: 点预设第 3 段后 preset=2 [OK] 分段按钮组可点（拼在一起的那组）
  ```

  判定读的是**主题里**的 `panel.radius`（不是控件自己的字段）⇒ 覆盖"滑杆 / 数字条 →
  应用字段 → 主题 → 画面"整条线。两种典型失败与它们的分辨办法：

  | 症状 | 成因 | 看什么 |
  |---|---|---|
  | 命中正常、值不变 | 点在**文本框**上（那是"进入编辑模式"，本来就不调值；只有最右 `GRIP_W`（20px）那条手柄能拖） | `RJ_NUM_TRACE=1`：`手柄 x=430..450` |
  | 命中正常、`拖拽中=false` | `down_edge` 没到 ⇒ `update_drag` 不开始拖 | `RJ_NUM_TRACE=1`：`down_edge=false` |

  ⚠ **脚本注入必须在各段 `f.ui(..)` 之前**：`MouseInput::end_frame` 每帧末清边沿位
  （`off_edge`）⇒ 在段末 / 收尾之后注入的 `down_edge` 会被**同一帧的收尾吃掉**，
  下一帧只剩 `pressed`（症状：命中、悬停都对，拖拽永远不开始）。这条是 `--sim-tuner`
  第一版踩出来的（点得中、拖不动）。
  `RJ_NUM_TRACE=1` 打印每个数字条的矩形切分与拖拽状态机（`docs/DEBUGGING.md` 本节）。

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

- **"标题栏的 × / ⌃ 点不中、或者点按钮把窗口拖走了"**：示例的 `--sim-chrome` 脚本化
  **真的去点**窗口 A 标题栏那两个按钮（坐标**从 `debug_dump` 的 `win_a` 外框矩形推导**，
  不写死像素）：

  ```
  cargo run -p eg260818UI -- --sim-chrome --frames 100
  sim-chrome: scale=1.5 ⌃=Vec2(353.5, 714.75) ×=Vec2(399.5, 714.75)
  sim-chrome: win_a open=true  collapsed=false pos=(560,240) size=(358,306)
  sim-chrome: win_a open=true  collapsed=true  pos=(40,470) size=(358,53)    # 点 ⌃：只剩标题栏
  sim-chrome: win_a open=false collapsed=true  pos=(40,470) size=(0,0)       # 点 ×：整窗短路
  sim-chrome: win_a open=true  collapsed=true  pos=(40,470) size=(358,53)    # 应用重开
  sim-chrome: win_a open=true  collapsed=false pos=(40,470) size=(358,306)   # 应用展开
  sim-chrome[四态]: 关闭=true / 收起=true / 重开+展开=true / 窗口没被拖动=true [OK] 标题栏按钮三态都走通
  ```

  脚本先把窗口 A 挪到**没有别的窗口压着**的空位（默认布局里 `win_b` / `chishi` 正盖着它的
  右上角——那正是"点击置顶"演示；被压住时点不到是引擎**正确**的遮挡行为）。
  `size` 是 `.show(..)` 的返回值，`pos` 全程不变、五段 `size` 对称 ⇒ 按钮上的按下**没有**
  变成窗口拖拽（`claim_press` 生效），且按钮右移**没有**改变任何窗口尺寸。坐标算错时点空的
  典型症状：`size` 只在"点错的那个按钮"的动作上变，或干脆一行都不打。两边都排查：
  `RJ_CHROME_TRACE=1`（引擎侧打印**解算结果**：`bar_w / title_w / title_max / collapse / close /
  inset`；断言口径就是 `close.x + close.w == bar_w − inset`）+ `RJ_HIT_TRACE=1`（这个像素
  到底命中谁）。

  ⚠ **两个坑（都在本轮踩过）**：
  1. **按钮位置不能用"内容右缘 + 硬编码余量"算**。`Ui::avail_w()` 是 `fixed_w − 2×pad`、
     内容右缘比**外框右缘**又少一个 `pad`，旧实现还额外留 4px ⇒ ✕ 离窗口右缘 `pad + 4`
     （实测 win_a @150% DPI：✕ 右缘 340 / 外框 358，**偏左 18px**）；而 18 = `pad + 4` 又
     恰好等于面板右上圆角半径（12 逻辑），于是"看起来差不多"，掩盖了很久。
     现在按钮走 `Ui::add_at` **绝对定位在外框坐标系**（`ui.rs::title_bar_layout` 纯函数 +
     单测），贴右缘；脚本的点击点也改成 `right = origin.x + size.x`（外框右缘）。
  2. **收起态里缩放柄会盖到按钮上**：窗口只有一行高时，右下角柄的命中区
     （`GripStyle::extent()`，本机 150% DPI 实测 `35×35`、起点 `y = 723`）往左上延伸到
     `×` / `⌃` 所在的一行（✕ 矩形 `(381,705,37,39)`），而柄在 `window_impl` 里**先于**按钮
     判定 ⇒ 点行中心会被柄抢走。实证：`RJ_HIT_TRACE=1` 打出
     `hit[frame 41] win_a::resize OK rect=(383,723,35,35) mouse=(399.5,724.5)`，同时
     `RJ_CHROME_TRACE` 的 `bar_w` 从 358 变 357（`window_widths` 被拖了 1px）。所以脚本点
     按钮的**上半部**（`y = origin.y + row_h * 0.25`，y < 723）绕开柄。柄 / 按钮重叠本身是
     既有缺陷（另案）。

  同一命令在"控件级遮挡"关掉时输出 `背包已选中 2 个 … [FAIL]`——相邻格子的**共享边**
  被两个格子同时命中（`hit_test` 含边界）正是"一次点击切换两个物品"的现场。
  排查别的控件同理：换成它的中心 / 边缘坐标即可。
- **加载外部文件**（看真实素材下的布局 / 图片铺排 / 字体）：`--image <路径>` 用你自己的
  图片当 `ImageBg`（PNG / JPEG / BMP / GIF，四个窗口分别演示 Fill / Tile 等铺排），
  `--font-file <路径>` 把 ttf / otf / ttc 加载进运行时文本子系统（随后在 `字体…` 弹窗里
  输入该字体的**族名**即可全局换字），**`--theme <路径>` 启动时载入主题**（TOML 序列化
  格式；文件里出现的字段**覆盖**在旋钮组装出的主题上 ⇒ 两行的手写文件也能当启动皮肤）：

  ```
  cargo run -p eg260818UI -- --image .\shot.png --font-file C:\Windows\Fonts\consola.ttf
  cargo run -p eg260818UI -- --theme C:\my-theme.toml
  theme: 启动载入 C:\my-theme.toml
  ```

- **"导出的主题导入回来不一样 / 主题文件加载报错"**：主题走 TOML 序列化
  （`Theme::to_toml` / `from_toml` / `apply_toml`）；界面上的入口是顶栏
  「导出主题…」（另存为）与「导入主题…」（在**当前**主题上合并覆盖），命令行入口是
  `--theme <路径>`。脚本化等价开关 **`--sim-theme <路径>`**（两段，不弹对话框；
  ⚠ 它**会写 / 覆盖**该路径——请给一个临时文件，别指向你的真实主题）：

  ```
  cargo run -p eg260818UI -- --sim-theme C:\rust-targets\sim-theme.toml --frames 70
  sim-theme: ① 7403 字节 · row_h=26 gap=6 字重=400 · 再导出逐字相同=true [OK] 主题导出 → 文件 → 导入：字段级往返一致
  sim-theme: ② 导入前 row_h=26 → 引擎侧 row_h=27（期望 27）[OK] 导入的主题真的进了引擎（全量文件改一行 ⇒ 只那一项变）
  ```

  典型失败与成因：

  | 症状 | 成因 |
  |---|---|
  | `主题字段不合法：wanted exactly 1 element, more than 1 element in 'button.bg'` | **渐变刷**（`Brush::Vertical` / `Horizontal`）曾被 serde 的默认"外部标签枚举"写成 TOML 数组表 `[[…bg.Vertical]]`，读回来必失败（纯色 `Solid` 因为只有 1 个元素反而没事）。现在刷子是显式 `{ kind = "vertical", colors = [色, 色] }`，且加载时**形状翻译**旧写法 ⇒ 旧文件直接能导入。自检：`--sim-theme` ① 会故意塞一个渐变刷进导出主题（"渐变刷往返=…"） |
  | 导入后**什么都没变** | 文件写成了别的表名（`[Theme]` / 顶层平铺字段名拼错）⇒ 逐键忽略是**故意**的（向前兼容）。先看 `--sim-theme` ① 的"再导出逐字相同"，再对着 `to_toml` 的输出改文件 |
  | `主题格式版本 N 不受支持` | 文件比本引擎新。**加字段**不需要抬版本（缺字段回落默认）；**改名 / 删字段**才需要，并同时抬 `THEME_FORMAT_VERSION` |
  | `主题字段不合法：invalid type …` | 字段类型写错（如 `radius = {tl = "6"}`）。`CornerRadius` 收标量也收表；`Weight` 是**数值**（`font_weight = 700`） |
  | 导出的主题**丢了窗口贴图** | `PanelStyle::bg_image` **故意不序列化**（纹理 uid 跨进程不可移植）⇒ 载入后回落 `None`，要贴图由应用自己灌 |
  | 导入后旋钮**拖了没反应** | 导入的主题当**基底**生效（旋钮暂不生效）——主题调节窗口里点「恢复调节」即可回到旋钮 |

- **"下级 popup 的阴影被上级 popup 的控件盖住"**（浮层嵌套）：所有浮层的 z 现在是
  **基址 + 嵌套层数**（`ui::overlay_z`，进入 / 退出用 `Ui::push_overlay_z` / `pop_overlay`），
  子浮层整段（含**阴影**）画在父浮层之后。写死同一个哨兵 z 会让两层命令落进同一个
  `(win, elem)` 分组排序，而窗口阴影 / 背景是 `elem = 0`、控件是 `elem ≥ 1` ⇒ 阴影被盖。
  硬断言在 `--sim-dropdown` ⑤（`z(父/子)=…/…+1 子层z更大=true`）+ 单测
  `ui/tests.rs::overlay_z_bands_by_nesting_depth`。"是否在浮层上"用区间判定
  `is_overlay_z(z)`，别写 `z == WIN_TOPMOST`。

- **"菜单栏点了没反应 / 菜单项点了菜单不收"**：示例的 `--sim-menu` 脚本化走**三段**
  （① 点「视图」触发器 → 点下拉里第一个菜单项；② 面板空白处按住拖走 ⇒ 面板不许动；
  ③ 点**栏内空白** ⇒ 菜单必须**仍开着**），坐标同样**运行时解算**（触发器按主题尺寸算、
  菜单项按**下拉窗口原点**算，见 `update` 里那段注释）：

  ```
  cargo run -p eg260818UI -- --sim-menu --frames 90
  sim-menu: menu_open=None · 主题调节窗口=关 [OK] 点触发器开菜单 + 点菜单项执行并自动收起
  sim-menu: 面板原点=Some(Vec2(245.0, 59.0)) · 期望=Some(Vec2(245.0, 59.0)) · 菜单仍开=true · 拖拽后没跑位=true [OK] 菜单面板不会被拖动
  sim-menu: 点栏内空白后 menu_open=true [OK] 点栏内空白不收起菜单
  ```

  判定同时看**引擎状态**（`UiState::menu_open`）与**应用状态**（菜单项真的改了
  `theme_tuner.open`）；阶段 2 在**面板空白处按住拖 600+px**，面板原点必须不变
  （`WindowClamp::Locked`）。典型失败：

  | 症状 | 成因 |
  |---|---|
  | 触发器点不着 | 坐标错（栏位置 / 触发器宽 = 文字宽 + `button.padding.x`；**栏里若有竖分割线，触发器 x 还要加上 `线厚 + 2×留白 + gap`**）；`RJ_HIT_TRACE=1` 看这个像素命中谁（`menubar::视图` / `menubar::视图/item::…`） |
  | **菜单打不开、随后"点栏内空白把菜单关了"** | 下拉面板现在是**嵌套窗口**（录在栏容器里）⇒ `debug_dump()` 的 `origin` **相对栏**，脚本算屏幕坐标漏了叠加栏原点 ⇒ 点到栏外，菜单当场收起（连锁症状：后面每段判定都错位） |
  | 菜单开着但点菜单项后不收 | `MenuCtx` 的 `close` 标志没被读回（`popup_show` → `PopupResult::item_clicked`）⇒ `MenuBar::finish()` 不会写 `menu_open = None` |
  | 阶段 2 `[FAIL] 面板被拖走了` | 下拉面板漏了 `WindowClamp::Locked`（**A/B 实测**：去掉后原点被拖到 x=1682）⇒ 面板与触发器脱节，命中按窗口走、视觉跑别处（"控件严重错位"） |
  | 阶段 3 `[FAIL] 点栏内空白把菜单关了` | "栏"的判定退化成"只认触发器"（旧行为）⇒ 得按**栏矩形**判（`menu_bar_should_close(.., on_bar, ..)`；栏矩形 = `pos + (bar.width 或内容宽)`）。`RJ_MENU_TRACE=1` 打印 `on_bar=`，一眼看出按下有没有算进栏内 |
  | **分割线又短又偏 / 标题与菜单项错列** | 面板内边距与菜单项起排不一致（内边距 = `menu::popup_padding(theme)` = `item_pad_x`；勾选**框**在项**内容里**）；或在**自动宽**下拉面板里用了 `Divider` 的**水平**模式（宽 = `avail_w()` = `None` ⇒ 退回固定 120）。⚠ 反过来：**栏里**（`row` 里）的竖分割线就该用 `Divider::vertical()`。`RJ_MENU_TRACE=1` 一跑就知道：正确时 `item` / `separator` / `caption` 三者**同 `x` 同 `w`**（实测 `x=8 w=224`，DPI 1.5） |
  | 下拉被别的窗口压住 | 面板 z 没走 `WIN_TOPMOST` 哨兵（走哨兵后与录制顺序无关） |
  | 面板宽度每帧都在变 / 变成一百万宽 | 首帧就请求了"极宽"（1e6）。必须**首帧自然宽**、第 2 帧起用 `prev` 定宽：`RJ_MENU_TRACE=1` 看 `prev=None → Some(226) → Some(240)`（最后应稳定） |
  | 菜单里再嵌的下拉一点就把外层菜单关了 | "点外"判定没排除**任意 `WIN_TOPMOST` 浮层**（`Ui::window_under_mouse()` 的 z） |
  | **数字条绕窗（warp）一拖到边缘值就"飞走"** | 拖拽基准**在请求 warp 的当帧就平移了**：`set_cursor_position` 只是请求，光标真正挪到对侧要等 OS 事件（可能晚一两帧、甚至不生效）⇒ 那几帧**每帧再平移一次**，值以"一个窗宽 / 帧"狂奔（实机与 `--sim-*` 都能复现）。正确口径：**请求**只管请求，**补偿**等观察到"跨窗跳变"（`|mx − 请求点| > 半个窗宽`）再按同样的量平移基准（`numberinput::warp_step`，4 个单测含端到端序列）。排查通道：`RJ_NUM_TRACE=1` 打印手柄命中 / `down_edge` / 按住 / 拖拽中 / 屏幕 x / 当前值——"值每帧暴涨且鼠标 x 卡在边缘"就是这个 BUG |
  | **菜单栏看不到"栏"（只有几个触发器）/ 还是一排按钮** | ① `bar.width(..)` 没给 ⇒ 背景 = **自然宽**（就触发器那么宽，看不出栏；egUI 示例踩过）；② 样式没生效：触发器常态底色看 `Theme::menubar.trigger_bg`（**默认全透明**，若被改成实色就会像一排按钮）、栏底看 `bg` + **底边线** `border/border_w`；③ 全局 `with_border_w(0)` 会把底边线也关掉（**这是有意的级联**）。`RJ_MENU_TRACE=1` 打印 `bar=(x,y w×h)`，栏宽对不对一眼可见 |
  | **主题里写了颜色但界面变成白块 / 颜色完全不对** | 手写 TOML 的颜色是 **0–1 归一化浮点**：写 `{ r = 32, g = 34, b = 38, a = 255 }` 会被当成"分量 > 1"⇒ 渲染时**夹到全白**（**不报错**，所以极难发现）。实测：`{ r = 10, g = 20, b = 30, a = 255 }` 载入后是 `Color { r: 10.0, .. }`。想确认就直接 `println!("{:?}", theme.menubar.bg)`：**归一化值应 ≤ 1.0** |

- **"下拉菜单点不开 / 选项点了不选中 / 菜单里的文本输入拿不到焦点 / 子菜单一点就整条消失"**：
  用 `--sim-dropdown`（**7 段，全 `[OK]` 即通路正常**）。坐标**运行时解算**：触发器按常量位置 +
  `Dropdown::width` + 主题尺寸；面板内行按**面板窗口原点** + **公开助手** `popup_padding` /
  `popup_gap` / `item_h`：

  ```
  cargo run -p eg260818UI -- --sim-dropdown --frames 130
  sim-dropdown: ① combo_open=Some("dd_opt")（期望 dd_opt）面板原点=Some(Vec2(990.0, 59.0)) 期望=Some(Vec2(990.0, 59.0)) [OK] 点触发器开下拉（面板在触发器正下方）
  sim-dropdown: ② 选中索引=0（期望 0）combo_open=None 面板消失=true [OK] 点选项 ⇒ 选中 + 自动收起
  sim-dropdown: ③ 富内容下拉 combo_open=Some("dd_file")（期望 dd_file）面板原点=Some(Vec2(1290.0, 59.0)) 期望=Some(Vec2(1290.0, 59.0)) [OK] 同一个控件也能开富内容菜单
  sim-dropdown: ④ text_focus=Some("dd_file::popup/dd_filter") [OK] 菜单里的文本输入真的可聚焦（菜单内又是 UiAdd）
  sim-dropdown: ⑤ 子面板原点=Some(Vec2(1557.0, 107.0)) 期望=Some(Vec2(1557.0, 107.0)) 面板在 dump=true combo_open=Some("dd_file") [OK] Hover 在 item 右边展开子菜单，且父 popup 不消失
  sim-dropdown: ⑥ 子菜单选中=Some(1)（期望 Some(1)=UTF-8）combo_open=None 子面板消失=true 父面板消失=true [OK] 点子菜单项 ⇒ 执行 + 整条链一起收起
  sim-dropdown: ⑦ Keep 项点击次数=1（期望 1）combo_open=Some("dd_file") 面板还在=true [OK] 点"保留 popup"的项 ⇒ 执行 + 不收起
  ```

  典型失败与成因：

  | 症状 | 成因 |
  |---|---|
  | **子菜单点一下就整条 popup 消失** | 菜单里嵌的是 `Dropdown`：它点击时写 `UiState::combo_open`，而那是**父下拉**的槽位 ⇒ 父 popup 当帧被判"没开"。正解 = **`Item::submenu(..)` / `m.submenu(..)`**（Hover 展开，状态挂**行自己**的 `WidgetState::submenu_open`） |
  | 子菜单位置不对 / 和父面板一样宽 | 方位必须 `PopupSide::Right`（`popup_origin(行, Right)`）；面板最小宽用 `ComboStyle::item_min_w`，**别用行宽**（行宽 = 父面板内容宽） |
  | 鼠标进子面板后子菜单就收起 | "保持"判定要按**窗口子树**（`state.window_rects` 绝对矩形 + `id_in_window_tree` 的 `/` 边界前缀），只看本行面板会漏掉更深一层 |
  | 子面板在 `debug_dump` 里查不到 | 同一帧多个 `WIN_TOPMOST` 浮层 **z 相同**，dump 旧实现按 z 键遍历（只留最后一个）。现已按**本帧录制过的窗口 ID** 列 |
  | 嵌套浮层的坐标差一个外层原点 | `UiWindowInfo::origin` 是**相对直接容器**的原点（顶层窗口才 = 屏幕坐标）：对话框里的下拉、下拉里的子菜单都要**叠加外层窗口原点** |

  ⚠ 两个易踩点：`UiState::combo_open()` 记的是**控件（触发器）的绝对 ID**（不是面板窗口 id——
  面板是它加 `::popup`）；面板里点不动时先看**这个像素命中的是谁**（`RJ_HIT_TRACE=1`）——
  win=0 内容会被任何窗口盖住（脚本因此先把自己的窗口收起来）。

- **"字重选不中 / 换了字重预览不变"**：`--sim-weight-modal`（字体对话框路径）与
  `RJ_FONT_TRACE=1`（**预览用的字重** + 样本实测宽）：

  ```
  cargo run -p eg260818UI -- --sim-weight-modal --frames 130
  sim-weight-modal: ① 草稿=100（期望 100 = 第 1 档）已应用=400（期望 400）combo_open=None [OK] 字重下拉真的选中了其他档位（草稿已改、尚未提交）
  sim-weight-modal: ② 已应用字重=100（期望 100）对话框还开着=false [OK] 确定后字重提交到应用状态（主题随之重建）

  RJ_FONT_TRACE=1 … → font[preview] weight=400（草稿）size=42 样本宽=392.0 …
                       font[preview] weight=100（草稿）size=42 样本宽=381.0 …
  ```

  ①的失败形态曾是"草稿永远回到旧档位"（选中的索引只进**局部变量**、等「确定」才写回，而局部
  变量每帧重置）⇒ 正解是**选中那一帧就写回 `*weight`（草稿）**；
  ②的失败形态曾是"预览不跟字重"⇒ 预览必须**用草稿字重排版**（临时代换 `Theme::font_weight`
  再还原，测高与绘制用同一个值）。档位表是公共常量 `FONT_WEIGHT_CHOICES`（**九档 100…900**），
  脚本按 `FONT_WEIGHT_CHOICES[0]` 断言而不是写死数值 —— 加档不会让脚本变成假失败。

  `RJ_MENU_TRACE=1`（引擎侧）打印下拉面板每一行的矩形与"宽度是否已固定"：

  ```
  menu[popup menubar::视图] prev=None pad_total=47 fill=false     # 首帧：自然宽
  menu[item]      x=47 y=47  w=210 h=33
  menu[separator] x=47 y=173 w=120 h=14                           # 首帧分割线按自然宽
  menu[popup menubar::视图] prev=Some(328.0) pad_total=47 fill=true
  menu[item]      x=47 y=47  w=234 h=33                           # 次帧起：全部铺满内容宽
  menu[separator] x=47 y=173 w=234 h=14
  menu[caption]   x=47 y=196 w=234 h=22
  ```

- **「导入图片…」/「导入字体…」这条运行时通路**（系统文件选择器 → 字节 → 纹理 / 字体）：
  选择器是**阻塞**调用、无头环境里没法跑，所以脚本化的等价开关是 `--sim-import <路径>`
  ——**不弹对话框**，直接走同一条 `apply_import`：

  ```
  # 字体：新族名必须被识别出来 + 真的参与整形
  cargo run -p eg260818UI -- --sim-import C:\Users\me\Documents\MyFont.ttf --frames 40
  sim-import: 排版实测「字体导入测试 ABCDEFG 0123456789」默认族 340.0 vs 导入族 "SJnenglianghei" 348.0 [OK] 导入的字体真的参与了整形
  sim-import: status="字体：MyFont.ttf → SJnenglianghei" · 字体族="SJnenglianghei" · 背景纹理=32×32 [OK] 导入通路走通

  # 图片：纹理尺寸从内建棋盘的 32×32 变成图片自己的尺寸
  cargo run -p eg260818UI -- --sim-import .\shot.png --frames 40
  sim-import: status="图片：shot.png 1920×1200" · 字体族="" · 背景纹理=1920×1200 [OK] 导入通路走通
  ```

  错误路径同样可脚本化（**这些是"应当失败"的用例**，输出 `[FAIL] 导入没成功` 才对）：
  `--sim-import Cargo.toml` → `不认得的文件类型：…`；
  `--sim-import C:\nope.png` → `图片导入失败：图片打不开：…`。
  ⚠ 用**系统字体自己的文件**（如 `C:\Windows\Fonts\arial.ttf`）测会得到
  `字体：arial.ttf（该族已在库里）`——`FontSystem::new()` 启动时已索引系统字体，
  要验证"新增族名"必须用**系统字体目录之外**的字体文件。
  真人在窗口里点按钮那条链路（按钮 → `import_request` → 帧外弹选择器）不走脚本；
  它的两个前提由代码结构保证：请求只在帧外消费（`rfd` 阻塞，且录制期 `f` 借着 `ctx`）、
  图片应用在**帧内**（`Gpu` 只能从 `f.draw()` 拿）。

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
- **"未 submit ⇒ 帧尾自动清屏提交"路径**（`Frame::present` / `Frame::drop`）：用
  `egHello --no-submit` 复现——它故意跳过 `f.submit`，配合 `RUST_LOG=rjw_krusie=trace`
  应看到 `krusie: 应用未提交本帧，使用 AppConfig::clear 自动清屏并呈现` + `画面 #1 region=…`，
  且冒烟仍 `[OK] N iterations / N frames presented`；正常（有 `submit`）的帧**不应**出现该行。
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

# 4) UI 帧时间三分解（引擎开场 / 应用录制 / 引擎收尾）+ 顶点缓存归因
cargo run -p eg260818UI -- --frames 240 2>&1 | Select-String "\[perf\]"   # 稳态取最后一行
$env:RJ_CACHE_TRACE="1"; cargo run -p eg260818UI -- --frames 40            # 哪扇窗/哪个槽在重镶嵌

# 5) **绘制序**（"谁画在谁上面"）——控件被别的 win=0 内容穿透 / 看错层级时用它
$env:RJ_ORDER_TRACE="150"; cargo run -p eg260818UI -- --frames 200 --sim-zorder   # 只打第 150 帧
#   `all`  = 每帧都打（⚠ 一帧 ~200 行且 stderr 无缓冲：**必须**重定向到真文件，
#            经 PowerShell 管道重定向会顶满缓冲把应用压到 ~1fps —— 与引擎无关）
#   `slot` = 打每个 win=0 **放置槽**的 seq 区间（"同一容器是不是被拆进两个排序空间"）
```

---

## 8. 两个"看起来像玄学"的历史 bug（排查套路可复用）

### 8.1 `ui=` 一帧莫名翻倍 —— 缓存**键**跨段冲突 + 按段清理

现象：UI 帧时间从 ~0.6ms 变成 ~1.2ms，`[perf]` 里 `collect` ≈ 0.6ms、
`cache_hit=9 cache_miss=7`（稳态**恒定** 7 次未命中，与内容变化无关）。

排查：`RJ_CACHE_TRACE=1` 一眼看出"miss **全在 win=0 放置槽**、9 扇窗口全命中"——
于是问题不在几何本身，而在槽的键/清理：

- `z0_quads` 的键当时只有**组号**，而组号是"**段内**第几个顶层放置"（各段都从 0 起、
  兜底组恒为 0）⇒ 段 1 的 `g=0` 与段 2 的 `g=0` 共用一个条目，**每帧互相覆盖**；
- 陈旧清理写在**段收尾**，而一次调用只见到**本段**的槽 ⇒ 段 2 把段 1 刚写好的缓存删掉。

修法：键 = `(段号, 组号)`；清理改到**帧首**按"上一帧的完整槽集合"做一次。
验证：`collect` 602µs → 2µs、`cache_miss` 7 → 0、`ui` 1.40ms → 0.88ms。

### 8.2 上层窗口挡不住背后窗口的控件 —— 清理用错了**数据源**

现象：被更高 z 窗口盖住的控件**有时**仍被点到 / 仍被拖动。

排查：`--sim-cover` 能稳定复现；`window_under_mouse()` 在录制期返回 `None`（连鼠标下
那扇窗都找不到）是决定性线索——说明**遮挡表是空的**。

根因：`window_rects` 的陈旧清理写在 `finalize_cursor_and_reset`，用的是本视图的
`win_ids` / `win_origins`；而这两张表在**同一帧更早**的 `finish()` 末尾被
`save_frame_state()` **swap 走了**（视图上剩下空表）⇒ 每帧把整张遮挡表清光，
遮挡判定退化成"只看本帧已录制的窗口"。于是**本帧录在后面的窗口**（示例里 `theme_tuner`
恒最后）挡不住前面窗口的控件。

修法（三件一起才完整）：
1. 遮挡表键改**窗口绝对 ID**（查询时把 ID 解成**当前 z**）——"点击置顶"在帧末改 z，
   按旧 z 比较会让刚抬高的窗口"消失一帧"；
2. 陈旧清理的数据源改 `UiFrameState::window_ids_seen`（帧级、不会被 swap 换走）；
3. 帧末按完备的遮挡表**复核按下归属**（`Ui::resolve_widget_press`），把命中那一刻
   判错的按下撤销（清除 `pressed` / `clicked` / `dragging`），诊断计数
   `UiState::press_cancelled_by_window()`。

**接着踩的第二个坑（同一处的反面）**：第 3 条第一版拿"**认领时的旧 z**"去比，而被点的窗口
在帧末已被抬到 `max+1` ⇒ 窗口**把自己**判成被更高 z 的窗口盖住 ⇒ **每一次**窗口内控件的按下
都被撤销：滑块 / 滚动条 / 文本选择**全都拖不动**。修法是把 `press_widget` 记成
`(控件 ID, 所在窗口 ID)`、复核时现解窗口的**当前 z**（`window_occluded` 是**严格大于**判定，
自己不会遮挡自己）。教训：**"谁比谁高"这类判定，两边必须用同一时刻的序**——一边用旧值、
一边用新值，就会出现"自己 ≥ 自己"的荒诞结果；而这类 bug 只有**正对照**（"该能拖的时候拖得动"）
才抓得住，只写"不该拖的时候拖不动"的负向断言会一直绿灯。

**教训**：跨帧缓存/表的"清理"必须问一句"这份数据在这一刻还在不在"——帧级暂存里的
字段可能已被 `save_frame_state` 换走；写"按帧清"的代码时，数据源要比位置更重要。

### 8.3 可拖动面板 / 滚动条"闪烁" —— win=0 **没有自己的排序空间**

现象（用户报告）：① 把「玩家名（可拖动）」面板拖到顶部 FPS / 点击次数标签上时，标签
**从面板里透出来**并随 FPS 文本逐帧抖；② 列表滚动时 ScrollBar 忽隐忽现（只在条目间隙露）。

排查：`RJ_ORDER_TRACE=<帧号>` 打印**实际提交顺序**（含 `place` / `elem` / 首顶点坐标），
一眼看出"面板的投影+底色（`elem = 0`）排在 FPS 标签**之前**"——先画 = 在下。
（`v0=(30,30)` 正是被拖到 (30,18) 的面板；`v0=(25,23)` 是 FPS 标签。）

根因：`elem = 0` 的语义是"画在**本容器**元素之下"，而**所有非窗口内容共享 `win = 0`**——
没有"每个容器一个排序空间"这回事，于是容器的底装饰掉到**整个 win=0 的最底**。
全仓 win=0 里用 `elem = 0` 的只有面板装饰与 `scroll_at` 滚动条两处，正好对上两个症状。

修法：在 `win` 之后插入**顶层放置序 `place`**（排序键 `(win, place, elem, …)`），
并让滚动条改用 `elem_hint()`（它本来就该在自家内容**之上**）。详见
`ENGINE_GUIDE.md` §18.23。

两个连带踩到的坑（都由 `RJ_ORDER_TRACE=slot` 定位——它打每个放置槽的 `seq` 区间，
"同一容器被拆成两个 `place`"一眼可见）：
1. `begin_top_placement` 记的必须是"本放置**第一条命令**的 `seq`"（`queue.seq + 1`）：
   紧靠容器之前录的那条命令（滚动条）的 `seq` 恰好等于容器入口的 `seq`，用 `≤` 会被算进
   新放置；
2. `scrollbar` 直接写队列时**两条命令只取了一次 `next_seq()`** ⇒ 序号重复；`place` 按
   `seq` 归一化，重复序号跨放置边界时同样拆容器。现在每条命令各取一次号，并在
   `begin_top_placement` 加 `debug_assert` 把"播放头落后"变成显式失败。
3. **同类的第三种写法**：`Painter::panel_img_elem` 一次调用推**三条**命令
   （背景刷 / 背景图 / 边框），序号用 `seq` / `seq + 1` / `seq + 2` —— 播放头只推进了 1
   ⇒ 下一条命令**重号**（`place` 拆容器），并且上面的 `debug_assert` 会**立刻炸**。
   实测现场：带**背景图**的窗口录完之后，`menu_bar` 新的容器入口把它踩了出来
   （`thread 'main' panicked … seq 播放头落后于已入队命令`）。
   修法：`DrawQueue::advance_seq_to(n)`（只增不减）按实际用掉的最大序号补齐 ——
   **凡是"一次调用推多条命令"的入口都要补**；配单测
   `panel_with_image_advances_the_seq_playhead`（"播放头 = 已分配最大序号" + 不重号）。
   **教训**：把不变量写成 `debug_assert` 的收益就在这里 —— 一个 3 年前就存在的重号，
   被新代码一条不变量检查当场抓住，而不是"偶尔闪一帧"。

**教训**：排序键缺一维时，症状不是"顺序乱了"，而是"**看起来像闪烁**"（内容在动的地方
一闪一闪）。这类问题**不要靠截图猜**：先问"这帧的绘制序是什么"，再拿引擎自己的提交序
对账。

### 8.4 单行输入框"滚过头文字消失" —— **排版矩形 ≠ 墨迹矩形**

现象（用户报告）：单行文本编辑器里内容一多、**横向滚过一定范围**后**整段文字消失**
（新 demo `egUI` 的 `BaseInformation` 窗口里输长名字即可复现）；多行 / 不换行多行
纵向滚过一屏后同理。

排查：`[perf]` 的 `culled_text=`（本帧被兜底剔除的**文本命令**条数）+ `--sim-text-cull`。
实测修前：未滚动 `verts=24052 culled_text=0` → 滚到 1000px `verts=24008 culled_text=1`
（那条输入框的文本命令被**整条丢掉**，顶点掉 44 —— 正好是它可见字形的那部分）。

根因：`collect_cmds` 的**兜底剔除**拿 `d.rect` 与裁剪层比"全外"，而**文本命令的 `d.rect`
是排版锚点矩形，宽/高取的是容器盒**（单行 = 框内宽 `content_w`、多行 = 框高 `rect.h`），
内容却随滚动**平移**（`text_dx = -text_scroll` / `rect.y - scroll_y`）：
滚过 `padding + content_w`（或 `+ rect.h`）后，这个矩形**整个滑出盒子** ⇒ `fully_outside`
⇒ 连裁剪都轮不到，文字整条没了。**软裁剪层不是这个毛病**——它锚在盒子上（调用方用
`scroll - padding` / `+scroll` 做了补偿），恒等于可见窗口；`draw_text_quads` 的逐字形
求交也一直是对的（与滚动无关）。

修法（`ENGINE_GUIDE.md` §18.22 有表）：文本剔除改用 `gpu_batch::text_visible_rect(rect, soft)`
= **软裁剪层的绝对位置**（`None` ⇒ 退回 `rect`，保持"内容自洽文本"的旧行为）；并让
文本命令的矩形**名副其实**（单行宽取全文宽、多行高取全文高）——后者观感不变
（`draw_text_quads` 只用 rect 的 x/y 做对齐锚点与软裁剪基准），但让"排版矩形 = 墨迹范围"
这条不变量成立，`debug_layout` 描边也跟着对。

**教训**：*"矩形"在 UI 里有两种含义*——**盒子的矩形**（布局/命中）与**内容的墨迹矩形**。
任何"拿矩形判可见性"的代码都要先问一句"这个 rect 是哪一个"；把盒矩形当墨迹用在
**会滚动**的控件上，症状就是"滚到某个位置内容凭空消失"。

### 8.5 选择高亮"多出来的空格" —— 一行 `+ space_w` 的代价

现象（用户报告，截图圈出蓝框）：文本域里选中文字后，**高亮比文字多出一格**（蓝色 =
`Theme::input.sel_bg`），看起来像选区里混了空格。

根因（一行代码，两处调用点）：高亮宽度写成
`(x1 - x0).max(0.0) + space_w`——注释是"行尾提示：选区延伸到行尾外一格"。于是
**每次选到行尾 / Ctrl+A**，高亮都比文字多一个空格宽；多行还逐视觉行各多一格，
几行的高亮块**连起来像一个大蓝框**（截图里那个"蓝色东西"）。

修法：抽出纯函数 `edit::selection_highlight_w(ink_w, space_w)`——`ink_w > 0` 用 `ink_w`，
只有 `ink_w <= 0`（**空行** / 零宽选区）才用一格宽兜底；单行 / 多行两个绘制点都用它。
兜底必须留：`w == 0` 的高亮块会被调用方 `if sel_rect.w > 0.0` 丢掉，选区里的**空行**
就完全看不见了（这是 `+ space_w` 当初想解决的问题，方向对、代价大）。

**教训**：*"提示性多余 1~2 像素/一格"这类视觉妥协，要问"它乘了几次"*——单处 1 格
尚可接受，**逐行相乘**后就成了明显的错误观感。选区跨行时的"行尾"语义本来就不需要
行内提示（下一行的高亮已经在表达"换行符也在选区里"）。


