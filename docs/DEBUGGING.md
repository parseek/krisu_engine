# 调试指南（Rust 代码级）

> 面向**引擎 / 应用代码**的调试（断点、日志、状态快照、脚本化复现）。
> 图形层专项（抓帧、像素、渲染目标导出）在 §6，但**优先用 §1–§5 的 Rust 侧手段**：
> 大多数"没画面 / 位置不对 / 拖不动"的根因在状态，而不在 GPU。

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
  **真的去点**窗口 A 标题栏那两个按钮（坐标由主题 + DPI 解算，不写死像素）：

  ```
  cargo run -p eg260818UI -- --sim-chrome --frames 100
  sim-chrome: scale=1.5 ⌃=Vec2(339.0, 738.0) ×=Vec2(385.0, 738.0)
  sim-chrome: win_a open=true  collapsed=false pos=(560,240) size=(358,320)
  sim-chrome: win_a open=true  collapsed=true  pos=(40,470) size=(358,67)    # 点 ⌃：只剩标题栏
  sim-chrome: win_a open=false collapsed=true  pos=(40,470) size=(0,0)       # 点 ×：整窗短路
  sim-chrome: win_a open=true  collapsed=true  pos=(40,470) size=(358,67)    # 应用重开
  sim-chrome: win_a open=true  collapsed=false pos=(40,470) size=(358,320)   # 应用展开
  sim-chrome[四态]: 关闭=true / 收起=true / 重开+展开=true / 窗口没被拖动=true [OK] 标题栏按钮三态都走通
  ```

  脚本先把窗口 A 挪到**没有别的窗口压着**的空位（默认布局里 `win_b` / `chishi` 正盖着它的
  右上角——那正是"点击置顶"演示；被压住时点不到是引擎**正确**的遮挡行为）。
  `size` 是 `.show(..)` 的返回值，`pos` 全程不变 ⇒ 按钮上的按下**没有**变成窗口拖拽
  （`claim_press` 生效）。坐标算错时点空的典型症状：`size` 只在"点错的那个按钮"的动作上变，
  或干脆一行都不打。两边都排查：`RJ_CHROME_TRACE=1`（引擎侧打印
  `content_w / title_w / spacer / btn`）+ `RJ_HIT_TRACE=1`（这个像素到底命中谁）。

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

- **"菜单栏点了没反应 / 菜单项点了菜单不收"**：示例的 `--sim-menu` 脚本化走两阶段
  （点「视图」触发器 → 点下拉里第一个菜单项），坐标同样**运行时解算**（触发器按主题
  尺寸算、菜单项按**下拉窗口原点**算，见 `update` 里那段注释）：

  ```
  cargo run -p eg260818UI -- --sim-menu --frames 62
  sim-menu: menu_open=None · 主题调节窗口=关 [OK] 点触发器开菜单 + 点菜单项执行并自动收起
  sim-menu: 面板原点=Some(Vec2(105.0, 59.0)) · 期望=Some(Vec2(105.0, 59.0)) · 菜单仍开=true · 拖拽后没跑位=true [OK] 菜单面板不会被拖动
  ```

  判定同时看**引擎状态**（`UiState::menu_open` 必须已收起）与**应用状态**
  （菜单项真的改了 `theme_tuner.open`）；阶段 2 在**面板空白处按住拖 600+px**，
  面板原点必须不变（`WindowClamp::Locked`）。三个典型失败：

  | 症状 | 成因 |
  |---|---|
  | 触发器点不着 | 坐标错（栏位置 / 触发器宽 = 文字宽 + `button.padding.x`）；`RJ_HIT_TRACE=1` 看这个像素命中谁（`menubar::视图` / `menubar::视图/item::…`） |
  | 菜单开着但点菜单项后不收 | `MenuCtx` 的 `close` 标志没被读回（`popup_show` → `PopupResult::item_clicked`）⇒ `MenuBar::finish()` 不会写 `menu_open = None` |
  | 阶段 2 `[FAIL] 面板被拖走了` | 下拉面板漏了 `WindowClamp::Locked`（**A/B 实测**：去掉后原点被拖到 x=1682）⇒ 面板与触发器脱节，命中按窗口走、视觉跑别处（"控件严重错位"） |
  | **分割线又短又偏 / 标题与菜单项错列** | 面板内边距与菜单项起排不一致（内边距 = `menu::popup_padding(theme)` = `item_pad_x`；勾选**框**在项**内容里**）；或分割线用了 `Divider`（它的宽 = `avail_w()`，**自动宽**窗口里是 `None` ⇒ 退回固定 120）。`RJ_MENU_TRACE=1` 一跑就知道：正确时 `item` / `separator` / `caption` 三者**同 `x` 同 `w`**（实测 `x=8 w=224`，DPI 1.5） |
  | 下拉被别的窗口压住 | 面板 z 没走 `WIN_TOPMOST` 哨兵（走哨兵后与录制顺序无关） |
  | 面板宽度每帧都在变 / 变成一百万宽 | 首帧就请求了"极宽"（1e6）。必须**首帧自然宽**、第 2 帧起用 `prev` 定宽：`RJ_MENU_TRACE=1` 看 `prev=None → Some(226) → Some(240)`（最后应稳定） |
  | 菜单里再嵌的下拉一点就把外层菜单关了 | "点外"判定没排除**任意 `WIN_TOPMOST` 浮层**（`Ui::window_under_mouse()` 的 z） |

- **"下拉菜单点不开 / 选项点了不选中 / 菜单里的文本输入拿不到焦点"**：用 `--sim-dropdown`
  （五段，全 `[OK]` 即通路正常）。坐标**运行时解算**：触发器按常量位置 + `Dropdown::width` +
  主题尺寸；面板内行按**面板窗口原点** + **公开助手** `popup_padding` / `item_h`：

  ```
  cargo run -p eg260818UI -- --sim-dropdown --frames 90
  sim-dropdown: ① combo_open=Some("dd_opt")（期望 dd_opt）面板原点=Some(Vec2(990.0, 59.0)) 期望=Some(Vec2(990.0, 59.0)) [OK] 点触发器开下拉（面板在触发器正下方）
  sim-dropdown: ② 选中索引=0（期望 0）combo_open=None 面板消失=true [OK] 点选项 ⇒ 选中 + 自动收起
  sim-dropdown: ③ 富内容下拉 combo_open=Some("dd_file")（期望 dd_file）面板原点=Some(Vec2(1290.0, 59.0)) 期望=Some(Vec2(1290.0, 59.0)) [OK] 同一个控件也能开富内容菜单
  sim-dropdown: ④ text_focus=Some("dd_file::popup/dd_filter") [OK] 菜单里的文本输入真的可聚焦（菜单内又是 UiAdd）
  sim-dropdown: ⑤ 菜单项点击次数=1（期望 1）combo_open=None 面板消失=true [OK] 点菜单项 ⇒ 执行 + 自动收起
  ```

  ⚠ 两个易踩点：`UiState::combo_open()` 记的是**控件（触发器）的绝对 ID**（不是面板窗口 id——
  面板是它加 `::popup`）；面板里点不动时先看**这个像素命中的是谁**（`RJ_HIT_TRACE=1`）——
  win=0 内容会被任何窗口盖住（脚本因此先把自己的窗口收起来）。

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

