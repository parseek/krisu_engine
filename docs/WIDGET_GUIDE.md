# rjw_ui 控件系统指南（Widget trait + 属性化 builder）

> 目标：**方便地添加新控件**（替代 `widget_api!` 宏路径）、**逐控件设置属性**
> （颜色 / 字号 / 字体 / 内边距 / 圆角等）、**文档齐全**、报错可调试。

---

## 1. 为什么不用 `macro_rules!` 了

旧 API 由 `widget_api!` 宏一次性生成全部容器（`Panel` / `Pack` / `Grid` / `Window` /
`Scroll` / `FlexCtx`）上的 `label` / `button` / `checkbox` … 方法：

- **难调试**：编译错误指向宏展开的合成代码，定位要靠 `cargo expand`；
- **难扩展**：加一个控件 = 改宏（一处改动影响所有容器），且每个控件的定制逻辑
  混在宏体里，无法单独测试；
- **无属性**：样式只能跟随全局 `Theme`，无法逐控件覆盖。

新系统把"控件"建模为**普通 Rust 结构体 + trait 实现**：

```rust
pub trait Widget {
    fn size(&self, ui: &mut Ui) -> Vec2;          // 期望尺寸（内容测量）
    fn ui(self, ui: &mut Ui, rect: Rect) -> Response; // 在矩形内渲染 + 交互
}
```

报错精确到你的结构体 / 方法，可单测，可组合。

---

## 2. 快速上手（已有控件）

```rust
use rjw_ui::{Button, Checkbox, Label, UiAdd}; // UiAdd 提供容器上的 add/add_at

// 容器内占光标：
ui.pack_at(Vec2::new(16.0, 16.0), PackSide::Top, |p| {
    if p.add(Button::new("btn_ok", "确定").color(Color::WHITE)).clicked() {
        // …
    }
    p.add(Checkbox::new("cb", "勾选我", self.checked)).toggled();
    p.add(Label::new("红色大字").color(Color::RED).font_size(20.0));
});

// 顶层绝对定位（不占光标）：
ui.add_at(Vec2::new(400.0, 40.0), Label::new("HUD"));
```

- 统一响应 [`Response`]：`hovered()` / `pressed()` / `clicked()` / `released()` /
  `toggled()`；
- 所有属性都是**可选覆盖**：不设置就回落全局 `Theme` 对应子样式；
- 旧 `p.button(...)` / `ui.label_at(...)` 等 API **保持可用**（内部已委托新的
  `*_at_styled` 原语），新旧可混用。

### 已有控件与属性

| 控件 | 构造 | 属性（setter） |
|---|---|---|
| `Label` | `Label::new(text)` | `color` `font_size` `font_family` `wrap(max_w)` |
| `Button` | `Button::new(id, label)` | `color` `bg` `bg_hover` `bg_pressed` `border` `border_w` `radius` `padding` `font_size` `font_family` |
| `Checkbox` | `Checkbox::new(id, label, checked)` | `color` `box_border` `checked_fill` `font_size` `font_family` |

---

## 3. 添加一个新控件（完整步骤）

以"标签式按钮"（`TagButton`：胶囊背景 + 文本，可点）为例：

### 3.1 定义 builder 结构体（属性 = `Option` 覆盖字段）

```rust
// crates/rjw_ui/src/widget.rs（或你自己的 crate，若实现依赖 Ui 内部则放 rjw_ui 内）
pub struct TagButton<'a> {
    id: &'a str,
    label: &'a str,
    bg: Option<Color>,
    font_size: Option<f32>,
    // …
}

impl<'a> TagButton<'a> {
    pub fn new(id: &'a str, label: &'a str) -> Self { /* 全 None */ }
    pub fn bg(mut self, c: Color) -> Self { self.bg = Some(c); self }
    pub fn font_size(mut self, s: f32) -> Self { self.font_size = Some(s); self }
}
```

### 3.2 实现 `Widget`

```rust
impl Widget for TagButton<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        // 主题回落值先拷出（owned），再调用 &mut ui 测量，避免借用冲突
        let size = self.font_size.unwrap_or(ui.theme.button.font_size);
        let family = ui.theme.button.font_family.clone();
        let tsize = ui.text_size(self.label, size, family.as_deref());
        Vec2::new(tsize.x + 24.0, tsize.y + 10.0)
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        // 1) 交互：复用现成原语（推荐）或自写（hit_abs / update_interact 为 pub(crate)）
        let st = ui.button_at_styled(self.id, rect, self.label, &ui.theme.button);
        // 2) 覆盖属性：你可以在 button_at_styled 前后追加自己的绘制命令
        //    （如 push_panel_like / push_text_rect 原语）
        Response::from(st)
    }
}
```

### 3.3 放置即用

```rust
if ui.add(TagButton::new("t1", "标签").bg(Color::ORANGE)).clicked() { … }
```

### 3.4 复用现有控件时的建议

需要给旧控件加"样式可覆盖"能力时，**给 `Ui` 增加 `xxx_at_styled(id, rect, …, &Style)`
变体，让旧 `xxx_at` 委托它**（参考 `button_at` / `checkbox_at` 的改造）：
- 旧 API 行为不变（传 `&theme.xxx`）；
- widget 层合并"主题 + 逐控件覆盖"后调用 `xxx_at_styled`。

---

## 4. 样式 / 字体 / 颜色怎么设置

三层机制，从全局到局部：

1. **全局主题**（[`Theme`](crate::style::Theme)）：`Theme::default()` / `Theme::dark()`
   预设全部子样式（`label` / `button` / `checkbox` / `input` / `panel` / `slider` /
   `focus`），可 clone 后逐字段改：
   ```rust
   let mut theme = Theme::dark();
   theme.button.bg = Color::rgba_u8(20, 90, 160, 255);
   theme.label.font_size = 16.0;
   let ui = Ui::begin(…).theme(theme).build();
   ```
2. **逐控件属性**（builder setter）：只覆盖你设置的字段，其余回落主题：
   ```rust
   ui.add(Button::new("b", "红底白字").bg(Color::RED).color(Color::WHITE));
   ```
3. **新控件内部**：`Theme` 子样式 + builder `Option` 合并（见 `resolve()` 示例）——
   需要主题新增样式字段时，直接在 `style.rs` 的子样式结构体加字段并给默认值。

字体族：`None` = 系统默认；传入字体名（如 `"Microsoft YaHei"`）启用指定族；
`font_size` 为**逻辑像素**（内部 × scale 取整到物理像素排版）。

---

## 5. 与旧 API / 其他机制的约定

- **旧 API 保留**：`p.button` / `ui.label_at` 等继续可用，内部走同一绘制原语；
  示例中"开始游戏"已改用新 API 演示。
- **`Response` 与旧状态**：`Response` 由 `ButtonState` / `CheckboxState` 转换而来
  （`From`），旧方法返回的类型不变。
- **放置语义**：`add` = 占光标（容器布局内）；`add_at` = 绝对定位（相对当前容器
  内容原点，不占光标）。容器包装经 `UiAdd` trait 提供（`use rjw_ui::UiAdd;`）。
- **widget 是值**：`Widget::ui(self, …)` 消费自身（builder 模式）；需要复用请
  每次构造新的（immediate mode 惯例）。
- **回调不存 Ui**：与 `pos_handler` 不同，widget 不需要 `'static` 闭包——
  属性都是值类型，无借用。

---

## 6. 待办（后续可加）

- `Slider` / `TextInput` / `TextArea` / `Combo` 的 builder 化（同样走 `*_at_styled`）；
- `Response` 扩展（如滑块新值 `Option<f32>`、`drag_delta`）；
- widget 级 `disabled` / `tooltip` 等通用属性。
