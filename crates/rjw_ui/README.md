# rjw_ui

krusie 引擎的 UI 模块：**hybrid 模式**（立即外观 + ID 持久状态）+ **DOM 风格自动布局** + **Tkinter 风格几何管理器**（`place` / `pack` / `grid`）。

## 设计要点

- **立即外观**：每帧 `Ui::begin(...)` → 录制控件 → `finish()` 深度排序后一次提交绘制（与 `Render2D` 逐帧录制架构一致）。
- **状态持久**：交互控件（按钮/滑块/勾选/输入框）通过 **ID**（`&str`）把 hover / 按下 / 焦点 / 输入内容 / 拖拽标记持久化在 `UiState` 中（应用持有，跨帧复用）。
- **自动尺寸**（DOM 风格）：叶子控件由内容测量（`rjw_text::Text::measure` + padding）自然撑开，容器（panel / pack / grid）在闭包结束时按子控件结算自身尺寸——**默认无需手写宽高**；任何控件可显式 `.size(w, h)` 或传 `Rect` 覆盖。
- **屏幕空间**：控件坐标一律为屏幕像素（左上角原点、Y+ 向下），内部经相机屏幕固定变换绘制，命中测试直接在屏幕像素进行（旋转/缩放相机依然准确）。

## 快速上手

```rust
let mut ui = Ui::begin(&cam, &ctx.mouse, &ctx.keyboard, &mut text, &mut r2d, &mut self.ui_state)
    .theme(Theme::dark())
    .base_layer(LAYER_UI)
    .build();

ui.label_at(Vec2::new(500.0, 20.0), "FPS: 60");       // 内容自动撑开
ui.pack_at(Vec2::new(24.0, 24.0), |p| {
    if p.button("开始游戏").clicked() { /* ... */ }   // 文本 + padding 自动宽高
    p.slider("volume", 0.0..=1.0, vol);               // 宽度 = 容器内宽
    p.checkbox("fs", "全屏", fs);
    p.text_input("name", &mut player_name);           // 自动高度
});
ui.grid_at(Vec2::new(320.0, 24.0), 3, |g| {           // 3 列自动均匀网格
    g.button("A"); g.button("B"); g.button("C");
});
ui.finish();
```

## 控件

| 控件 | 交互状态（ID 持久） | 返回值 |
|---|---|---|
| `panel` | 无（纯背景 + 边框） | `()` |
| `label` | 无（纯文本） | `()` |
| `button` | hover / 按下 | `ButtonState`（`.clicked()` 等） |
| `slider` | 拖拽标记 + 值 | `f32`（更新后的值） |
| `checkbox` | 勾选值 | `CheckboxState`（`.checked()` / `.toggled()`） |
| `radio` | 组内互斥（同组 ID 前缀） | `CheckboxState` |
| `text_input` | 焦点 / 光标 / 内容 | `()`（写入 `&mut String`） |

## 布局

- `*_at(pos)`：绝对定位 + 内容自然尺寸（place）
- `pack_at(pos, |p| ...)`：按 `side`（默认 Top）堆叠，宽度 = 最大子控件自然宽
- `grid_at(pos, cols, |g| ...)`：均匀单元格网格，尺寸 = 最大子控件自然尺寸

## 依赖

`rjw_2d_render`（绘制）/ `rjw_text`（测量与渲染）/ `rjw_transform`（屏幕固定变换）/ `rjw_color` / `rjw_mouse`（鼠标）/ `rjw_keyboard`（字符输入）/ `glam`

## 许可

MIT
