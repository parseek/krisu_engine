# krusie GUI 指南（rjw_ui）

> 立即模式 UI：外观每帧录制（`Ui::begin` → 控件 → `finish`），交互状态按 ID 持久于
> `UiState`。坐标一律为**屏幕逻辑像素**（左上角原点、Y+ 向下），内部自动换算物理像素。

---

## 1. 快速上手

```rust
use rjw_ui::{PackSide, Theme, Ui, UiState};

// 应用持有跨帧状态
let mut ui_state = UiState::new();

// 每帧：begin → 录制控件 → finish（输入经 capture 快照；相机/渲染器延迟到 finish 传入）
let mut ui = Ui::begin(window, &mut text, &mut ui_state)
    .capture(&mouse, &keyboard)
    .theme(Theme::dark())
    .base_layer(1e7)                        // UI 层（高于世界层）
    .scale_factor(ctx.scale_factor().unwrap_or(1.0))
    .build();

// 顶层绝对定位（place）
ui.label_at(Vec2::new(16.0, 12.0), "Hello UI");

// 容器内（pack 垂直堆叠，占光标）
ui.pack_at(Vec2::new(16.0, 90.0), PackSide::Top, |p| {
    p.label("主菜单");
    if p.button("btn_start", "开始游戏").clicked() {
        // …
    }
    let vol = p.slider("vol", 0.0..=1.0, 0.6);
});

ui.finish(&viewport, r2d); // 视口/渲染器在此延迟传入（UI 无需相机，仅视口大小+位置）
// r2d 提交（UI 的 Render2D 必须 set_sorting(false)；set_mvp 用 viewport.vp_matrix()）
```

---

## 2. 布局容器

| 容器 | 语义 | 方法 |
|---|---|---|
| `pack_at` | 子项按 `PackSide` 堆叠（Top/Left/…），尺寸自动 | `ui.pack_at(pos, side, \|p\| …)` |
| `grid_at` | 均匀网格，单元格尺寸跨帧缓存 | `ui.grid_at(pos, cols, id, \|g\| …)` |
| `flex_at` | 固定总高按权重等分子项 | `ui.flex_at(pos, h, &[1,2,1], \|f, i\| …)` |
| `scroll_at` | 垂直滚动容器（滚轮/滚动条 + 可视区裁剪） | `ui.scroll_at(pos, view, id, \|s\| …)` |
| `list_at` | 选择列表（scroll + 逐项回调） | `ui.list_at(pos, view, id, n, sel, \|s,i,sel\| …)` |
| `panel_at` / `drag_panel_at` | 面板（背景+边框，可拖拽） | `ui.panel_at(pos, \|p\| …)` |
| `window_at` | 可重叠窗口（z-order + 点击置顶 + 可拖拽） | `ui.window_at(id, pos, \|w\| …)` |
| `window` builder | 窗口统一入口（`.width` 固定宽 / `.strict` 裁剪 / `.style` 逐窗样式） | `ui.window(id).pos(..).width(..).strict().style(..).show(\|w\| …)` |
| `panel` builder | 面板统一入口（`.drag(id)` 可拖拽 / `.style` 覆盖） | `ui.panel().pos(..).drag(id).show(\|p\| …)` |
| `modal` builder | 模态对话框统一入口（`.width` 固定宽） | `ui.modal(id).pos(..).width(..).show(\|m\| …)` |

- 容器内子项**占光标**堆叠；`*_at` 形式**绝对定位**（相对当前容器内容原点，不占光标）。
- 尺寸约束：`p.min_size(w, h)` / `p.max_size(w, h)` 作用于下一子项。

---

## 3. 控件

### 3.1 旧 API（宏生成，容器包装上直接调用，保持可用）

`p.label` / `p.button` / `p.checkbox` / `p.radio` / `p.slider` / `p.combo` /
`p.text_input` / `p.text_area` / `p.label_wrap`，以及对应 `*_at`（显式矩形）变体。

### 3.2 新 API（`Widget` trait + 属性化 builder，**推荐新控件用这个**）

```rust
use rjw_ui::{Button, Checkbox, Label, UiAdd}; // UiAdd 提供容器上的 add/add_at

ui.pack_at(Vec2::new(16.0, 16.0), PackSide::Top, |p| {
    if p.add(Button::new("btn_ok", "确定").color(Color::WHITE).radius(6.0)).clicked() { … }
    p.add(Checkbox::new("cb", "勾选我", self.checked)).toggled();
    p.add(Label::new("红色大字").color(Color::RED).font_size(20.0));
});
ui.add_at(Vec2::new(400.0, 40.0), Label::new("HUD"));
```

- **添加新控件**：定义 builder 结构体（属性 = `Option` 覆盖字段）+ 实现 `Widget`
  （`size` 测量 / `ui` 渲染交互）——普通 Rust，无宏，报错精确可单测。
  详见 `docs/WIDGET_GUIDE.md`。
- 统一响应 `Response`：`hovered() / pressed() / clicked() / released() / toggled()`。

### 3.3 文本输入

- **单行** `text_input`：超长滚动跟随光标（左/右缘双向）、拖选、剪贴板、IME；
- **多行** `text_area`：Enter 换行、自动换行 + 垂直滚动、↑/↓ 跨**视觉行**（保持列）、
  Home/End、拖选、剪贴板、IME；
- **IME 内联组合**：组合候选融入文本流（后续文本右移），组合段带下划线，光标随
  组合内偏移移动，组合较长时滚动跟随（不裁切）；
- 剪贴板 Ctrl+C/V/X/A 需按住 Ctrl；选择 + 退格/Delete = 删除选择（不多删）；
  无 Shift 的方向键取消选择。

---

## 4. 窗口系统

- **容器责任链 builder**：`ui.window(id).pos(..).width(..).strict().style(..).show(..)` /
  `ui.panel().pos(..).drag(id).show(..)` / `ui.modal(id).pos(..).width(..).show(..)`——
  统一旧 `window_at*` / `panel_at` / `modal_at*` 的选项组合（后者保留，薄委托）；
- **z-order**：点击窗口置顶（z+1）；重叠区域只有**最上层**窗口可交互（点击穿透已修复）；
- **拖拽**：按住窗口/面板移动 ≥ 3 物理像素进入拖拽（纯点击不拖拽，子控件正常响应）；
- **位置责任链**（`Ui::pos_handler`）：脚本/动画提供者（优先级降序）→ 用户拖拽状态
  （优先级 0）→ 传入 pos（兜底）。`priority < 0` = 拖拽优先（动画让步）；
  `priority > 0` = 脚本锁定。闭包须 `'static`（捕获拥有值 / `Instant` / `Arc`）；
- **置顶浮层**：IME 组合、下拉浮层用 `WIN_TOPMOST` 哨兵 z——恒在一切窗口之上绘制与
  命中（普通窗口 z 分配/置顶运算排除哨兵）。

---

## 5. 样式 / 主题 / 字体 / 颜色

三层机制：

1. **全局主题** `Theme`（`Theme::default()` / `Theme::dark()`，可 clone 后逐字段覆盖）：
   ```rust
   let mut theme = Theme::dark();
   theme.button.bg = Color::rgba_u8(20, 90, 160, 255);
   theme.label.font_size = 16.0;
   ```
2. **逐控件属性**（builder setter）：未设置回落主题；
3. **新控件内部**：`Theme` 子样式 + builder `Option` 合并（复用现有控件时给 `Ui` 加
   `xxx_at_styled(…, &Style)` 变体，旧方法委托）。

字体：`None` = 系统默认；传字体名启用指定族；字号为逻辑像素（内部 × scale 取整）。

---

## 6. 光标 / 调试 / 性能

- **光标**：悬停文本输入框 = I 型；拖动 = 普通箭头（Resize 手柄未来用方向光标）。
- **调试**：`ui.debug_layout(true)` 画每个控件布局矩形描边；`ui.debug_line/rect/cross/
  grid` 屏幕空间调试图元；窗口诊断 `ui.window_order()` / `ui.window_under_mouse()`；
  `UiState::occluded_hits()` / `last_press_window()`。
- **性能**：`UiState::stats`（`UiStats`：sort/sig/collect/clone/submit/finish 各阶段 µs +
  命令数/窗口数/缓存命中）；示例 `eg260818UI` 每 120 帧打印 `[perf]`，`--auto-drag`
  压力场景。开发构建已开 `opt-level=2`（保留调试信息）。
- **窗口顶点缓存**：内容签名 = 逐命令全量哈希（含颜色等一切渲染字段）——hover/click
  变色即时刷新；移动窗口只改变换不重建顶点。

---

## 7. 常见坑

| 现象 | 原因 / 处理 |
|---|---|
| UI 文字被图形盖住 / 顺序错乱 | UI 的 Render2D 必须 `set_sorting(false)`（UI 自管顺序） |
| 高 DPI 下控件模糊/错位 | 用 `.scale_factor(ctx.scale_factor())`，坐标按逻辑像素 |
| 窗口内 hover/click 不刷新 | 旧版摘要缓存漏颜色——已修（全量签名）；升级即可 |
| 下拉浮层背后按钮响应 | 已修（window_rects 存绝对坐标） |
| 窗口拖不动 / 按钮点不了 | 输入框按下会占用按压（选择拖拽优先）；从空白处拖动 |
| 输入框内 ↑/↓ 跳到别的控件 | 已修：文本输入框持有焦点时 ↑/↓ 归输入框处理（Tab 仍遍历焦点） |
