# rjw_ui 控件系统指南（Widget trait + 属性化 builder + UiAdd 容器 API）

> 目标：**方便地添加新控件**（容器便捷方法由 `UiAdd` trait 提供，不再有 `widget_api!` 宏）、
> **逐控件设置属性**（颜色 / 字号 / 字体 / 内边距 / 圆角等）、**文档齐全**、报错可调试。

---

## 1. 为什么不用 `macro_rules!` 了

旧 API 由 `widget_api!` 宏一次性生成全部容器（`Panel` / `Pack` / `Grid` / `Window` /
`Scroll` / `FlexCtx`）上的 `label` / `button` / `checkbox` … 方法（**该宏已移除**）：

- **难调试**：编译错误指向宏展开的合成代码，定位要靠 `cargo expand`；
- **难扩展**：加一个控件 = 改宏（一处改动影响所有容器），且每个控件的定制逻辑
  混在宏体里，无法单独测试；
- **无属性**：样式只能跟随全局 `Theme`，无法逐控件覆盖。

新系统把"控件"建模为**普通 Rust 结构体 + trait 实现**，把"容器便捷方法"建模为
**trait 默认方法**：

```rust
pub trait Widget {
    fn size(&self, ui: &mut Ui) -> Vec2;          // 期望尺寸（内容测量）
    fn ui(self, ui: &mut Ui, rect: Rect) -> Response; // 在矩形内渲染 + 交互
}

// 容器 API（crate::ui::UiAdd）：唯一必需方法 ui_mut()，
// label / button / checkbox / slider / … 全是默认方法——新容器一行 impl 即全部获得。
pub trait UiAdd<'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a>;
    fn label(&mut self, text: &str) -> Vec2 { /* 默认实现 */ }
    fn button(&mut self, id: &str, label: &str) -> ButtonState { /* 默认实现 */ }
    // …
}
```

报错精确到你的结构体 / 方法，可单测，可组合。**扩展方式**：
- 加**控件**：定义 builder 结构体 + `impl Widget`（见 §3）；
- 加**容器便捷方法**：在 `UiAdd` 加一个默认方法，6 个容器自动获得；
- 加**新容器**：`impl UiAdd` 一行 `ui_mut()`，立即获得全部方法。

---

## 2. 快速上手（已有控件）

```rust
use rjw_ui::{Button, Checkbox, Label, UiAdd}; // UiAdd 提供容器上的 add/add_at 与全部便捷方法

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
| `Label` | `Label::new(text)` | `color` `font_size` `font_family` `wrap(max_w)` `ellipsis()`（超出可用宽以"…"省略） |
| `Button` | `Button::new(id, label)` | `color` `bg` `bg_hover` `bg_pressed` `border` `border_w` `radius` `padding` `font_size` `font_family` |
| `Checkbox` | `Checkbox::new(id, label, checked)` | `color` `box_border` `checked_fill` `font_size` `font_family` |
| `Divider` | `Divider::new()` | `color` `thickness` `margin`（占光标分割线；宽 = 容器可用宽） |

**Label 溢出策略**（Resizable 窗口缩窄）：默认在父级可用宽内**自动换行**；
`.ellipsis()` 切换为单行"…"省略。Button / 勾选 / 下拉的文本超出分配矩形时
**自动省略**（内容自洽，noclip 绘制）。

### Widget 尺寸契约（`Widget` trait 默认方法，现有 impl 零破坏）

```rust
fn constraints(&self) -> SizeConstraints { SizeConstraints::default() }  // min_w/max_w/min_h/max_h 全 Option<f32>
fn expansion(&self) -> Expansion { Expansion::UnlimitedExpansion }
fn resizable(&self) -> Option<(Vec2, Vec2)> { None }   // 可选拖拽缩放范围
```

- `UnlimitedExpansion`（默认）：内容自然尺寸（clamp min/max），撑大父级（DOM 语义）；
- `LimitedInParent`：取 min(内容, 父级可用宽 `Ui::avail_w()`)，超出由控件自处理
  （Label 换行 / 省略、Button 省略、TextArea 滚动）；
- `DisableAutoExpansion`：不撑大父级（内容溢出由控件用 noclip 自洽）。
- 拖拽缩放：`Ui::resize_handle(id, handle, current, min, cursor)` 通用原语 +
  `UiState::sizes` 持久尺寸（`window_at_w` 宽度缩放即基于它）。

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
        // 1) 交互：复用现成原语（推荐）或自写（hit_abs / mouse_left / register_focus /
        //    key_click / hit::update_interact 均为**公开**，可跨 crate）
        let st = ui.button_at_styled(self.id, rect, self.label, &ui.theme.button);
        // 2) 覆盖属性：你可以在 button_at_styled 前后追加自己的绘制命令
        //    （如 push_panel_like / push_text_rect / push_solid_rect 公开原语）
        Response::from(st)
    }
}
```

### 3.3 放置即用

```rust
if ui.add(TagButton::new("t1", "标签").bg(Color::ORANGE)).clicked() { … }
```

### 3.4 跨 crate 自定义控件（公开接口清单）

`Widget` trait 与下列 `Ui` 公开原语组成**控件作者接口**——你的 crate 里 `impl Widget`
即可用（完整可编译模板：滑块 + 数字输入（含拖动调值）见 `crate::widget` 模块文档）：

| 分类 | 公开原语（`Ui` 方法） | 用途 |
|---|---|---|
| 主题 | `ui.theme`（字段，可读可改） | 样式取值 / 逐控件覆盖合并 |
| 测量 | `text_size` / `text_size_wrap` | `Widget::size` 里内容测量（逻辑像素） |
| 布局 | `child_rect` | 自写"占光标"容器时分配子矩形 |
| 命中 | `hit_abs(&Rect)` / `mouse_left()` / `mouse_logical()` | 点中判定 / 左键状态 / 拖拽基准 |
| 按下归属 | `claim_press()` | **自身有拖拽语义的控件**在按下时调用——阻止外层窗口把本次按下当窗口拖拽基准 |
| 焦点 | `register_focus(id, rect, FocusKind)` / `key_click(id, kind)` | 键盘导航（Tab/Enter/方向键）接入 |
| 状态 | `state_mut().widget(id)` → `WidgetState` + `hit::update_drag` / `update_interact` | 跨帧交互状态机（hover/按下/拖拽基准） |
| 绘制 | `push_panel_like` / `push_text_rect` / `push_solid_rect` / `push_border_rect` | 背景边框 / 文本 / 实心 / 描边（逻辑坐标） |
| 复用 | `button_at_styled` / `checkbox_at_styled` / `slider_at` / `text_input_at` / `text_area_at` / `radio_at` / `combo_at` | 委托现有控件（**内置控件同路径**） |

约定：`rect` 均为**相对当前容器 origin 的局部坐标**；`push_*` 内部 ×scale 取整到
物理像素；交互前先拷出主题值（Copy / owned）避免借用冲突。

### 3.5 复用现有控件时的建议

需要给旧控件加"样式可覆盖"能力时，**给 `Ui` 增加 `xxx_at_styled(id, rect, …, &Style)`
变体，让旧 `xxx_at` 委托它**（参考 `button_at` / `checkbox_at` 的改造）：
- 旧 API 行为不变（传 `&theme.xxx`）；
- widget 层合并"主题 + 逐控件覆盖"后调用 `xxx_at_styled`。

---

## 4. 样式 / 字体 / 颜色怎么设置

三层机制，从全局到局部：

1. **全局主题**（[`Theme`](crate::style::Theme)）：`Theme::default()` / `Theme::dark()`
   预设全部子样式（`label` / `button` / `checkbox` / `input` / `panel` / `slider` /
   `focus`）。用 **`with_*` 责任链**构建（链上后设覆盖先设；可级联的全局参数
   自动落到所有相关子样式），或 clone 后逐字段改：
   ```rust
   let theme = Theme::dark()
       .with_font_family("Microsoft YaHei")   // 级联 label/button/checkbox/input
       .with_font_size(16.0)                   // 同上
       .with_radius(6.0)                       // 级联 panel/button/input
       .with_gap(8.0)                          // pack/grid 间距
       .with_button(ButtonStyle { bg: Color::rgba_u8(20, 90, 160, 255), ..Theme::dark().button }); // 整子样式替换
   let ui = Ui::begin(…).theme(theme).build();
   // 等价旧写法（仍可用）：
   // let mut theme = Theme::dark();
   // theme.button.bg = …;
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
  内容原点，不占光标）。容器包装经 `UiAdd` trait 提供全部方法
  （`use rjw_ui::UiAdd;`，需在作用域内——便捷方法现在来自 trait 而非宏）。
- **widget 是值**：`Widget::ui(self, …)` 消费自身（builder 模式）；需要复用请
  每次构造新的（immediate mode 惯例）。
- **回调不存 Ui**：与 `pos_handler` 不同，widget 不需要 `'static` 闭包——
  属性都是值类型，无借用。

---

## 6. 待办（后续可加）

- `Slider` / `TextInput` / `TextArea` / `Combo` 的 builder 化（同样走 `*_at_styled`）；
- `Response` 扩展（如滑块新值 `Option<f32>`、`drag_delta`）；
- widget 级 `disabled` / `tooltip` 等通用属性。

## 7. 裁剪分层与 noclip 绘制（已落地）

- **强制层（硬裁剪）**：ScrollView 可视区 / Clip 沙箱（`Ui::view_at(…, ViewMode::Clip)`、
  `window_at_strict`）。`UiDraw.clip` 恒为该层，**所有绘制（含 noclip 变体）都服从**；
- **软层（内容裁剪）**：控件自身内容边界，由调用方显式传参（`push_text_rect` 的局部
  `clip`、文本框内容区）。内容自洽的控件（自动换行 / "…"省略 / 滚动）用
  `push_*_noclip` **跳过软层**——但 ScrollView 强制裁切躲不掉（反例：无 Scroll 的
  普通容器本无强制层，noclip 内容画出界 = 自洽内容本就不会出界）；
- **View 沙箱**（`crate::view`）：闭包作用域 `ui.view_at(pos, size, mode, |v| …)`，
  统一"裁剪层 / 坐标原点 / 可用宽度 / 命中过滤"，`scroll_at`、文本编辑框、严格窗口共用。
- **内部计算一律物理像素**（滚动偏移 / 取整 / 命中）；对外单位可选参数用
  [`Metric`](crate::draw::Metric)（`Physical` / `Logical`，内部换算物理）。
