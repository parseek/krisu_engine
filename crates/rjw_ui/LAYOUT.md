# rjw_ui 布局系统

> 本文讲解 `rjw_ui` 的布局模型，重点是最常用、也是默认的 **pack** 布局，并覆盖
> grid / flex / place / window / panel / scroll 等其余几何模式。示例见
> `examples/eg260818UI`（主菜单 / 背包 / 窗口 / 列表 / flex / 状态栏）。

---

## 1. 总体心智模型：DOM 式自动尺寸 + 「占光标」

`rjw_ui` 采用 **DOM 风格自动尺寸**（控件由内容自然撑开），核心是 **「占光标」
（cursor）**模型：

- **叶子控件**（`label` / `button` / `slider` / `text_input` …）：按自身内容测量
  出自然尺寸，调用 `child_rect(w, h)` 在**当前容器内占据一个位置并推进光标**；
- **容器**（`pack` / `grid` / `flex` / `window` / `panel` …）：在自身 Frame 内放置
  子项，闭包结束时按已放置子项**结算自身尺寸**（`settle_size`）——pack 取最大宽 /
  累加高，grid 取单元格与行数；
- **嵌套**：子容器结算尺寸后，由父容器**补记占位**（`place_external`），因此一个
  `row`（水平行）在垂直 pack 里只占「一行」的高度。

坐标系：**左上角为原点、Y+ 向下**（与屏幕/相机一致）；所有控件坐标一律是
**相对当前容器 origin 的局部坐标**，容器弹出时由 `Ui` 统一平移成绝对坐标。

---

## 2. 顶层即默认 pack

`Ui::build()` 会压入一个**根容器**：`PackSide::Top`（垂直 pack）+ `fixed_w = 视口宽`。
所以：

```rust
// 顶层直接堆叠（默认垂直 pack）——无需手动包一层 pack：
ui.label("标题");
ui.button("btn", "按钮");
ui.slider("vol", 0.0..=1.0, 0.5);
```

顶层自动换行标签依赖 `avail_w()` = 视口宽（`LimitedInParent` 模式）。

---

## 3. pack —— 核心布局

### 3.1 两种方向（`PackSide`）

```rust
ui.pack_at(pos, PackSide::Top,     |p| { /* 子项自上而下，左对齐 */ });
ui.pack_at(pos, PackSide::Left,    |p| { /* 子项自左而右，顶对齐 */ });
ui.pack_at(pos, PackSide::Bottom,  |p| { /* 子项自下而上（向上长），左对齐 */ });
ui.pack_at(pos, PackSide::Right,   |p| { /* 子项自右而左（向左长），顶对齐 */ });
```

| `PackSide` | 方向 | `pos` 锚定边 | 光标推进 | 容器结算 |
|---|---|---|---|---|
| `Top` | 垂直堆叠，子项**左对齐** | pack 左上原点 | `cursor.y += 子项高 + gap` | **宽 = 最大子项宽**，高 = 累加子项高 + 间距 |
| `Left` | 水平堆叠，子项**顶对齐** | pack 左上原点 | `cursor.x += 子项宽 + gap` | **高 = 最大子项高**，宽 = 累加子项宽 + 间距 |
| `Bottom` | 垂直堆叠，**自下而上**（向上长），子项左对齐 | pack **下边缘** | `cursor.y -= 子项高 + gap` | 同上（高取负向光标绝对值） |
| `Right` | 水平堆叠，**自右而左**（向左长），子项顶对齐 | pack **右边缘** | `cursor.x -= 子项宽 + gap` | 同上（宽取负向光标绝对值） |

- 子项间距 = `Theme::gap`（四个方向统一）。
- `Bottom` / `Right` 的 `pack_at(pos, …)` 中 `pos` 对准 pack 的**下/右边缘**（内容向负区
  生长），因此 0 边即锚定边、首个子项贴该边——适合页脚 / 右对齐菜单栏 / 标题栏按钮簇：
  ```rust
  ui.pack_at(Vec2::new(0.0, viewport_h - 28.0), PackSide::Bottom, |p| { /* 页脚，向上长 */ });
  ui.pack_at(Vec2::new(viewport_w - 120.0, 0.0), PackSide::Right, |p| { /* 右栏，向左长 */ });
  ```
- 每个子项实际矩形 = `child_rect(测量宽, 测量高)`；光标随之推进，`max_child` 记录
  已放子项的最大尺寸（供结算）。

### 3.2 一个垂直 pack 的逐步推演

```rust
ui.pack_at(Vec2::new(16.0, 90.0), PackSide::Top, |p| {
    p.label("主菜单");                          // 高 20
    p.button("btn", "开始游戏");                // 高 34
    p.slider("vol", 0.0..=1.0, 0.5);           // 高 24
});
// gap = 6：
//   第 1 个子项占 (0,0, w,20)，光标 y → 26
//   第 2 个子项占 (0,26, w,34)，光标 y → 66
//   第 3 个子项占 (0,66, w,24)，光标 y → 96
// settle_size = (最大子项宽, 96 - 6)   // 高 = 光标末 - 最后一个 gap
```

### 3.3 容器内常用方法（`Pack` / `Panel` / `Window` / `Grid` / `Scroll` / `FlexCtx` 共用）

这些容器都实现 `UiAdd` trait，占光标方法一致：

| 方法 | 说明 |
|---|---|
| `label / button / slider / checkbox / text_input / …` | 占光标放置（内容自然尺寸） |
| `add(Widget)` / `add_at(pos, Widget)` | 属性化 builder 控件（`Label` / `Button` / `Slider`…） |
| `label_wrap(max_w, …)` | 宽内自动换行的标签 |
| `row(closure)` | 水平行：子项 `PackSide::Left` 排列（**左上角对齐、沿 X 推进**），在父容器中占一行。行高 = 子项最高的那个；**单行子项**（[`SizeClass::SingleLine`]，默认）被**钉到行的标准高**（默认 [`Theme::row_h`](crate::Theme::row_h)），**多行子项**（[`SizeClass::Multiline`]，如多行 `TextEditor`）以它为下限、可撑高整行 |
| `row_builder()` | 行的可配置形态（[`crate::RowBuilder`]）：`min_h`（行高下限 + 单行子项标准高）/ `max_h`（上限）/ `height`（固定行高）/ `gap` / `pad` / **`wrap_w`（行宽上限 ⇒ 自动换行）** / **`wrap`（用父级可用宽自动换行）** / **`line_gap`（折行的行间距，默认 = `gap`）**，最后 `.show(closure)`；容器内也有 `ui.row_wrap(w, ..)` 便捷入口 |
| `min_size / max_size(w, h)` | 一次性约束**下一个子项**（`set_next_min/max`） |
| `divider()` | 分割线（占光标；宽 = 容器当前可用宽） |

### 3.4 尺寸约束（min / max，一次性）

- `p.min_size(min_w, min_h)`：下一子项宽度/高度**抬升**到最小值；
- `p.max_size(max_w, max_h)`：下一子项宽度/高度**压缩**到最大值（`0` = 该轴不约束）。
- 应用顺序：**先压 max 再抬 min**（`min` 恒优先：`min > max` 时结果为 `min`）；
- **一次性**：作用于下一个子项后自动清零，后续子项恢复自然尺寸。

```rust
p.min_size(160.0, 0.0);
p.button("btn_min", "min 宽");   // 这个按钮宽 ≥ 160
p.max_size(120.0, 0.0);
p.button("btn_max", "max 宽");   // 这个按钮宽 ≤ 120
```

### 3.5 强制高度（flex 权重 / 行等高）

`Frame` 提供两种「覆盖测量高度」的机制：

- `force_next_h(h)`：**一次性**强制下一子项高度（`flex_at` 按权重分配各子项高度用）；
- `set_force_h_all(h)`：**持续作用于本 frame 全部子项**（`row` 的标准行高用，默认
  `Theme::row_h`）。语义按子项的**尺寸类**（[`crate::widgets::SizeClass`]）分两种：
  **单行子项被钉到 `h`**（覆盖自然高度与一次性 `force_next_h`），**多行子项以 `h` 为下限**
  （`h.max(自然高)` ⇒ 可以撑高整行）。尺寸类由 `Ui::add` 在调 `Widget::ui` 之前从
  [`crate::widgets::Widget::size_class`] 读取（一次性，`child_rect` 消费）。
- 行级 min/max（只由 `RowBuilder` 设置）：`Frame::set_row_bounds(min, max)`，在
  `settle_size` 末尾按"先压 max 再抬 min"夹取容器**自身**高度（min 胜）。

### 3.6 固定宽 / 固定高（`fixed_w` / `fixed_h`）

- `ui.window(id).width(w)`：设 `fixed_w` —— 子项宽度 **clamp 到容器固定宽**（内容按固定宽排布、
  高度自然，如同 egui），`settle_size` 宽 = `fixed_w + 2*pad`；
- `flex_at`：设 `fixed_h` —— 结算高度固定为 `total_h`（覆盖自然高度）。

---

## 4. 其它布局模式

### 4.1 grid —— 网格（`Ui::grid_at` / 背包）

```rust
ui.window("inv").show(|p| {
    p.grid_at(Vec2::new(0.0, 28.0), 3, "inv", |g| {   // 3 列
        for i in 0..9 { g.button(&format!("slot_{i}"), &label); }
    });
});
```

- 按 `cols` 列、单元格尺寸 `cell` 定位：第 `i` 个子项在 `(col = i % cols, row = i / cols)`，
  矩形 = `(pad + col*cell.w, pad + row*cell.h)`；
- **单元格尺寸跨帧缓存**（`UiState::grid_cells`，保证布局稳定），但会**就地渐进扩格**
  容纳当前子项（缓存值不足时本帧扩大，位置即时一致）；
- 结算尺寸 = `(cols * cell.w + 2*pad, 行数 * cell.h + 2*pad)`。

### 4.2 flex —— 固定高按权重等分（`Ui::flex_at`）

```rust
ui.flex_at(vec2(880.0, 450.0), 150.0, &[1, 2, 1], |f, i| {
    f.button(&format!("flex_row_{i}"), "…");
});
```

- 固定总高 `total_h`，子项按 `weights` **权重等分高度**（扣子项间距后分配）；
- 同帧精确分配，无需跨帧缓存；内容超高时溢出可见（需要滚动时在子项内嵌 `scroll_at`）。

### 4.3 place —— 绝对定位（`*_at`）

所有 `*_at` 方法**不占光标**，按绝对位置（相对当前容器内容原点）放置：

```rust
ui.label_at(Vec2::new(16.0, 12.0), "FPS: 60");          // 顶部状态栏
ui.pack_at(Vec2::new(16.0, 90.0), PackSide::Top, …);     // 绝对定位一个 pack
ui.window("win_b").pos(pos).show(|w| { … });             // 绝对定位一个窗口
```

`add_at(pos, widget)` 同理。位置参数可用 `Logical` / `Physical` 单位（`Position`），
按 DPI 缩放换算成物理像素。

### 4.4 window / panel —— 可重叠 / 可拖拽容器

- **window**（`ui.window(id)`，**唯一入口**）：可重叠 + 点击置顶（焦点 z-order）+
  可拖拽；内容顶点按**内容签名**缓存（移动窗口只改变换、不重建顶点）；`.width(w)`
  右下角缩放柄改宽；`.placement(Placement::Clip)` 内容强制裁剪（Clip 沙箱）；
  `.level(Level)` 控制点击是否置顶；
- **panel**（`ui.panel()` / `ui.drag_panel_at`）：背景 + 边框 + 内边距的容器；
- 窗口 / 面板**自带命名空间边界**：内部子控件 ID 自动带 `id` 前缀。

### 4.5 scroll / list —— 滚动容器（ScrollView）

- **scroll_at**：内容在可视区内堆叠，滚轮 / 滚动条滚动，可视区外**强制裁剪**
  （Clip 沙箱）；滚动偏移持久于 `UiState::scrolls`（物理像素）；
- **list_at**：`scroll_at` + 逐项回调（选中态由调用方维护），返回被点击的索引。

### 4.6 row —— 水平行（等高管线）

`p.row(|r| { … })`：子项按 `PackSide::Left` 水平排列、**左上角对齐**；**单行子项**被钉到
行的标准高 [`Theme::row_h`](crate::Theme::row_h)（内容各自垂直居中 → 文字中心线对齐），
**多行子项**（`TextEditor::multiline()`）可以把行**撑高**（行高 = 最高的子项）。整体在父
容器中占一行（宽 = 子项结算、撑大父级）。要自定义行高上下限用 `row_builder()`（见 §3.3）。

**自动换行**（`row_builder().wrap_w(w)` / `.wrap()` / `ui.row_wrap(w, |r| …)`）：给行一个
**行宽上限**，塞不下就**收行**（`cursor.y += 行高 + line_gap`，回到行首继续沿 X 排）：

- **只换行、不压缩**：开启后本 frame 不再报"行内剩余宽"（[`Frame::remaining_w`]）——
  否则 `Label` 这类 `LimitedInParent` 子项会被**压扁**而不是换到下一行（那是未开启折行的
  `row` 的既有语义，两者互斥）；
- **行内左上角对齐**：行高 = 该行**已见**最大子项高（单遍流式的必然：先放的子项不会因为
  后放的高子项而重新居中），且不小于标准行高（`min_h` / `Theme::row_h`）；
- **空行不折**：首个子项比行宽还宽 ⇒ 它自己占一行并溢出（不会先折出一个空行、也不死循环）；
- `.wrap_w(..)` = 显式上限（与**父级可用宽取 min**）；`.wrap()` = 用父级可用宽（自动宽窗口
  **首帧**没有可用宽 ⇒ 本帧不折，次帧起折，与 `Label` 的换行同口径）；
- **不调它们 ⇒ 与旧行为一字不变**（宽 = 内容，窄容器里走"压窄"那条路）；
- 结算：宽 = **最长行右缘**、高 = 各行高 + 行间距（靠既有的 `content_bounds` + 末行兜底）；
- `RJ_ROW_TRACE=1` 打印每个"请求了折行的" row 的宽度来源（`explicit` / `avail` / `limit`）
  与结算尺寸（`size.y > std_h` ⇒ 确实折了）。

### 4.6.1 `Child::Fill` —— 整格装饰的第三态

布局契约 [`Child`] 有三态（`Ui::child_rect(w, h, child)` 的第三个参数）：

| 值 | 宽度 | 高度 | 用途 |
|---|---|---|---|
| `Expand`（默认） | 计入父级 | 计入父级 | 普通控件 / 容器 |
| `Fit` | **不计入** | **不计入** | `DisableAutoExpansion`（内容自洽、溢出可见） |
| **`Fill`** | **计入观感（按申请宽铺满），但不计入父级** | 计入父级 | **整格装饰**：`divider()` 的线、`foldable` 的标题行 |

为什么需要 `Fill`：整格装饰"天生要铺满可用宽"，一旦让它们**参与容器宽结算**，在
**带 `vscroll` 的自动宽窗口**里会**正反馈锁定** —— 滚动视口的宽首帧取"屏幕剩余宽"
（`window_impl` 里为防视口塌成 1px 的引导值），装饰按它铺满 ⇒ 内容宽 = 视口宽 ⇒ 下一帧
视口又按内容反推 ⇒ **窗口一打开就被撑到屏幕大小**（用户实测："分割线会默认水平撑开到屏幕
大小 …… 而是**其他内容有多少就该多宽**"）。用 `Fill` 后容器宽由**其他内容**决定，装饰跟着
铺满即可（实测 Gallery 窗口宽 1920 → 683）。

### 4.7 namespace —— 只要 ID 命名空间，不要容器

```rust
ui.namespace("left", |ui| {
    ui.text_input("kw", &mut left);      // 状态键 = "left/kw"
});
ui.namespace("right", |ui| {
    ui.text_input("kw", &mut right);     // 状态键 = "right/kw"（与上面互不干扰）
});
```

- **不做任何布局**：没有背景 / 内边距 / 裁剪，也不另开 frame —— 正文与"不用它"时**逐像素
  相同**，只是内部控件的**绝对 ID** 多了 `id/` 前缀（见 [`crate::id`]）；
- 正文是**当前容器的子项**：录在光标处，`avail_w` 与 `gap` 照旧生效；父光标由正文各项
  自己推进（本方法不额外占位）；
- **返回正文结算尺寸**（`Vec2`；空内容 = `(0,0)`）；
- 嵌套顺序拼接：`"a"` 里的 `"b"` 里的 `"kw"` ⇒ `"a/b/kw"`。

什么时候需要它：两处 UI（两个子表单 / 动态列表的每一行 / 两段并列的排版）都会出现 `"ok"` /
`"kw"` 这类**相对名**，没有命名空间时它们的跨帧状态（输入内容 / 焦点 / 勾选）会互相覆盖。
窗口 / 面板 / 滚动容器 / grid / **区块**各自已经是命名空间边界；本入口给"只想要命名空间、
不要任何容器"的场合。

### 4.8 foldable —— 可收缩区块

```rust
ui.foldable("perf", "性能统计").show(|ui| {
    ui.label(&format!("FPS {fps}"));       // 折叠时这段**完全不录制**
});
ui.foldable("advanced", "高级").open(true).show(|ui| { /* … */ });   // 首次就展开
```

- **标题行** = 一整行（高 `Theme::row_h`、宽铺满容器内容宽）：▶ / ▼ + 文本；**点它即翻转**
  （当帧几何不变、**下一帧**生效，与窗口 ⌃ 同口径）；`Tab` 可聚焦、`Enter` / `Space` 翻转；
- **默认折叠**（首次只见标题行）；`.open(true)` 改首次展开 —— 只影响**从未被点过**的区块：
  首帧把默认态落盘到 [`UiState::folded`]（**绝对 ID → bool 的表**，存"明确态"），
  之后点标题翻转写的就是它，所以默认折叠的区块点开后**不会**被默认值折回去；
- **折叠 = 正文完全不录制**：不占高、不参与布局、不进命中表、不产生顶点；正文内部的跨帧
  状态（滚动偏移 / 输入内容 / 焦点）**不清**，展开回来还是原样；
- **折叠状态引擎托管**（[`UiState::folded`]）：应用不必多一个 `bool` 字段，要读 / 改用
  `UiState::{is_folded, set_folded, toggle_folded}`（`set_folded(id, false)` = **记住展开**）；
- 正文录在**本区块的 ID 命名空间**里 ⇒ 两个区块里的同名控件互不干扰；
- **正文的归属提示**：正文整体**左缩进** `FoldableStyle::body_indent`（默认 12 逻辑像素；
  缩进同时作用于绘制与命中 ⇒ "点得到的就是看得见的"，而容器尺寸不变），并在标题三角下方画一条
  **竖引导线**（`guide` / `guide_w` / `guide_tail`；`guide_w = 0` 关掉）；
  **可选**：`fade_h > 0` 时在正文上下缘各画一条"阴影色 → 全透明"的矩形
  （`.with_body_fade(12.0, Color::rgba_u8(0,0,0,90))`，两条 `Gradient`，零纹理）；
  标题行想要"类按钮"的块状外观用 `FoldableStyle::button_like(&palette)`
  （常态底色 + 描边 + 圆角；**行为不变** —— 整行本来就是命中区）；
- **标题里的控件，用 `Foldable::custom(ui, id, |t| …)`**（或容器里的
  `foldable_custom(id, |t| …)`）—— 闭包拿到与正文同一个上下文类型（`PackEntry`，实现了
  `UiAdd`），**标题由此成为"标准容器"**：`t.label(..)` / `t.row(|r| ..)` / `t.button(..)` /
  `t.text_input(..)` 都能用；
  - 内容从**标题文本区**（`pad_x + icon_w` 之后）起排 ⇒ 不压三角图标；
  - 容器**只固定宽、高度自然** ⇒ 放 `Row` / 多行内容时**标题块自己长高**（正文随之让位：
    标题行矩形加高 + 父容器光标补推同样多）；
  - 里面的控件自己认领按下 ⇒ 点它们**不会**连带折叠标题（也不会把这次按下当成外层窗口 /
    面板的拖动基准）。⚠ 命中区恒为**首行**（交互发生在内容之前，高度那时还不知道）。

---

## 5. 尺寸契约与膨胀模式（`Widget` trait）

属性化控件（`Label` / `Button` / `Slider`…）实现 `Widget`：

- **`size()`**：返回内容自然尺寸（可经 `ui.text_size` 测量）；
- **`constraints()`**：每轴 `min_w/max_w/min_h/max_h`（默认全 `None`）——`Ui::add`
  布局前对 `size()` 结果按此 clamp；
- **`expansion()`** 膨胀模式：
  - `UnlimitedExpansion`（默认）：内容自然尺寸，**撑大父级**（DOM 语义）；
  - `LimitedInParent`：**限制在父级可用空间内**（`min(内容, 沙箱可用宽, avail_w)`），
    超出部分由控件自处理（Label 自动换行 / "…"省略、Button 省略、TextArea 滚动）；
  - `DisableAutoExpansion`：内容按自身尺寸，**不撑大父级**（溢出自洽，如分割线）。

`Ui::add` 流程：`widget_size(w)` → `apply_constraints(natural, c)` → `child_rect_exp(...)`
（按膨胀模式决定是否更新 `max_child`）→ `w.ui(ui, rect)`。

---

## 6. 单位与 DPI

- 内部坐标、布局、命中、绘制一律**物理像素**；
- `Ui::begin(..).scale_factor(s).build()` 会**预乘 Theme**（字号 / 尺寸 × scale 取整），
  之后内部零 scale 换算；
- 公开 API 边界（`Size` / `Position`）支持 `Logical` / `Physical` 单位，按 scale 换算；
- `ui.avail_w()`：沙箱/固定宽容器扣除内边距后的可用宽（`LimitedInParent` 自动换行用）。

---

## 7. 小结

| 模式 | 入口 | 特点 |
|---|---|---|
| **pack**（默认） | 顶层直接堆叠 / `pack_at(pos, side, …)` | 四个方向（Top/Bottom 垂直、Left/Right 水平）堆叠，子项自然尺寸，容器取最大宽/高；Bottom/Right 的 `pos` 锚定下/右边缘 |
| grid | `grid_at` | 固定列数，单元格尺寸跨帧缓存 |
| flex | `flex_at` | 固定总高，按权重等分高度 |
| place | `*_at`（`label_at`/`pack_at`/`add_at`…） | 绝对定位，不占光标 |
| window | `ui.window(id)` builder | 可重叠 / 置顶（`.level`）/ 拖拽 / 顶点缓存 / 可选缩放（`.width`）/ 裁剪（`.placement`） |
| panel | `panel_at` / `drag_panel_at` | 背景 + 边框 + 内边距 |
| scroll | `scroll_at` / `list_at` | 滚动容器（强制裁剪） |
| row | `row(… )` | 水平等高管线，占一行 |
| namespace | `namespace(id, \|ui\| …)` | **只加 ID 前缀**，不新增容器 / 不占额外光标 |
| foldable | `foldable(id, label)` / `Foldable::custom(…)` | **可收缩区块**：一行标题 + 可折叠正文（折叠时正文不录制） |

**一句话**：默认用 **pack** 自上而下搭骨架；复杂排布用 grid / flex / row；需要独立
定位或浮层用 `*_at` / window / panel；长内容用 scroll / list；**分节 / 分栏**用
foldable（可折叠）与 namespace（只要 ID 隔离）。
