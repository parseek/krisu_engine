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
| `row(closure)` | 水平行：子项 `PackSide::Left` 排列，**行内全部强制等高** `Theme::row_h`（`force_h_all`），在父容器中占一行 |
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
- `set_force_h_all(h)`：**持续作用于本 frame 全部子项**（`row` 行等高用，`Theme::row_h`；
  优先级最高，覆盖自然高度与一次性 `force_next_h`）。

### 3.6 固定宽 / 固定高（`fixed_w` / `fixed_h`）

- `window_at_w`：设 `fixed_w` —— 子项宽度 **clamp 到容器固定宽**（内容按固定宽排布、
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
ui.window_at("win_b", pos, |w| { … });                   // 绝对定位一个窗口
```

`add_at(pos, widget)` 同理。位置参数可用 `Logical` / `Physical` 单位（`Position`），
按 DPI 缩放换算成物理像素。

### 4.4 window / panel —— 可重叠 / 可拖拽容器

- **window**（`ui.window(id)` / `ui.window_at`）：可重叠 + 点击置顶（焦点 z-order）+
  可拖拽；内容顶点按**内容签名**缓存（移动窗口只改变换、不重建顶点）；`window_at_w`
  右下角缩放柄改宽；`window_at_strict` 内容强制裁剪（Clip 沙箱）；
- **panel**（`ui.panel()` / `ui.drag_panel_at`）：背景 + 边框 + 内边距的容器；
- 窗口 / 面板**自带命名空间边界**：内部子控件 ID 自动带 `id` 前缀。

### 4.5 scroll / list —— 滚动容器（ScrollView）

- **scroll_at**：内容在可视区内堆叠，滚轮 / 滚动条滚动，可视区外**强制裁剪**
  （Clip 沙箱）；滚动偏移持久于 `UiState::scrolls`（物理像素）；
- **list_at**：`scroll_at` + 逐项回调（选中态由调用方维护），返回被点击的索引。

### 4.6 row —— 水平行（等高管线）

`p.row(|r| { … })`：子项按 `PackSide::Left` 水平排列、**全部强制等高** `Theme::row_h`
（内容各自垂直居中 → 文字中心线对齐），整体在父容器中占一行（宽 = 子项结算、撑大父级）。

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
| window | `window_at` / `window` | 可重叠 / 置顶 / 拖拽 / 顶点缓存 / 可选缩放 |
| panel | `panel_at` / `drag_panel_at` | 背景 + 边框 + 内边距 |
| scroll | `scroll_at` / `list_at` | 滚动容器（强制裁剪） |
| row | `row(… )` | 水平等高管线，占一行 |

**一句话**：默认用 **pack** 自上而下搭骨架；复杂排布用 grid / flex / row；需要独立
定位或浮层用 `*_at` / window / panel；长内容用 scroll / list。
