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
    /// 就地申请空间（`ui.allocate*`）→ 绘制（`ui.painter()`）→ 收交互（`ui.interact`）。
    fn ui(self, ui: &mut Ui) -> Response;
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
| `Divider` | `Divider::new()` / `.horizontal()` / `.vertical()` | `axis(DividerAxis)` `color` `thickness` `margin`（占光标分割线）。**水平**（默认）：宽 = 容器可用宽，行高 = 线厚 + 2×留白；**竖直**（`vertical()`，`row`/菜单栏里用）：宽 = 线厚 + 2×留白、高 = **一行**（`Theme::row_h`），线在占位矩形里垂直居中、长度 = 高 − 2×留白 |
| `TextEditor` | `TextEditor::new(id, &mut String)` | `multiline()` `no_wrap()` `resize(Resize)` `width` `height` `min_size` `max_size`（**物理像素**）`font_size` `font_family` `text_color` `caret_color` `selection_color` `preedit_color` `background` `border` `border_focus` `border_w` `radius` `padding_x`；`at(rect)` = 绝对定位。**开了 `.resize(..)` 时**：默认尺寸 = 单行 `min_w×height` / 多行 `max(min_w,200)×90`（**逻辑像素**）、下限默认 = `(min_w, height)`（一行文字标准高，拖不到 0）、尺寸跨帧持久于 `UiState::sizes`；自动申请会**先问尺寸责任链**（`Ui::resolved_size`）⇒ 拖大后容器与后续控件跟着长；柄形状取自 `Theme::input.grip`（默认三条横线，`Diagonal`/`Hidden` 可选，`Hidden` 仍可拖） |

**Label 溢出策略**（Resizable 窗口缩窄）：默认在父级可用宽内**自动换行**；
`.ellipsis()` 切换为单行"…"省略。Button / 勾选 / 下拉的文本超出分配矩形时
**自动省略**（内容自洽，noclip 绘制）。

### Widget 尺寸契约（**就地申请**，v0.3 起）

`Widget` 只有**一个方法**（`fn ui(self, ui) -> Response`）——**尺寸不再单独声明**，
而是在 `ui()` 里就地申请（参考 egui 的 `allocate_exact_size`）：

| 申请方式 | 语义 | 旧 `Expansion` 对应 |
|---|---|---|
| `ui.allocate(size)` | 占光标，**撑大父级** | `UnlimitedExpansion`（默认） |
| `ui.allocate_mode(size, Expansion::DisableAutoExpansion)` | 不撑大父级（装饰件） | `DisableAutoExpansion` |
| `ui.allocate_mode(size, Expansion::LimitedInParent)` | 宽度压到父级 `avail_w()` | `LimitedInParent` |
| `ui.allocate_at(pos, size)` | 绝对定位（不占光标） | — |
| `ui.allocate_sense(id, size, sense)` | 申请 + **一次收交互** | — |

- 尺寸一律是**物理像素 `Vec2`**（与"`Theme` 已预乘、内部全物理像素"一致）；要写逻辑
  单位**必须**显式换算 `Size::Logical(x).to_physical(ui.scale())`（正典：`Size` 的
  「单位纪律」rustdoc）；
- **单位纪律（自定义控件同样适用）**：收 `impl Into<Size<..>>` / `Option<Size<..>>`
  的 setter 可以原样转发（`self.font_size = Some(s.into())`），但**解释必须显式**——
  在画 / 量 / 命中的地方 `to_physical(scale)` 或 `match`，**禁止**先 `into()` 再读 `.0`
  （那是无声把逻辑当物理）；实现体内造值只写 `Size::Logical(..)` / `Size::Physical(..)`，
  主题值 / 持久化值一律 `Physical`；
- **min/max 尺寸**：`apply_constraints(desired, c)` 后交给 `allocate`（不再有 trait 钩子）；
- 拖拽缩放：`Ui::resize_handle(id, handle, current, min, cursor)` 通用原语 +
  `UiState::sizes` 持久尺寸（`ui.window(id).width(w)` 宽度缩放即基于它）。

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

### 3.2 实现 `Widget`（**只写一个方法**）

```rust
impl Widget for TagButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        // ① 先量（文本测量 / 读主题）——必须在 allocate 之前（申请会推进光标）
        let st = ui.theme.button.clone();          // 主题回落值先拷出（owned），避免借用冲突
        let size = self.font_size
            .map(|s| Size::Logical(s).to_physical(ui.scale()))
            .unwrap_or(st.font_size);
        let tsize = ui.text_size(self.label, size, st.font_family.as_deref());
        let desired = Vec2::new(tsize.x + 24.0, tsize.y + 10.0);
        // ② 申请 + 收交互（一句话：id_for / 命中 / 焦点 / 按下认领 / update_interact）
        let (rect, resp) = ui.allocate_sense(self.id, desired, Sense::CLICK.focus(FocusKind::Button));
        // ②' **被裁剪层完全剔除 ⇒ 直接 return**（scissor 只省片元，这里省镶嵌/顶点/段）。
        //     只用 allocate（只要矩形）时用 `ui.culled(rect)` 自己判一次。
        if resp.culled {
            return resp;
        }
        // ③ 画（一个绘制块一个 painter）
        let p = ui.painter();
        // 悬停 / 按下三态：`Brush` 支持逐控件覆盖（`bg`），这里给"有覆盖就用覆盖"的最简式
        let bg = self.bg.unwrap_or_else(|| st.pick_bg(resp.pressed, resp.hovered));
        p.panel(rect, bg, st.border, st.border_w, st.radius);
        p.text(rect, self.label, size, st.fg, st.font_family.clone(),
               TextAlign::Center, TextVAlign::Center, None, None);
        resp
    }
}
```

- 想要"复用现成控件的交互 + 自绘"就调 `ui.button_at_styled(id, rect, label, &style)` /
  `ui.checkbox_at_styled(..)`（**显式 rect** 入口）——它们的 `*State` 可 `Response::from(..)`；
- 想自写命中：`ui.hit_abs(&ui.id_for(self.id), &rect)` 的绝对 ID 决定**控件级遮挡**
  （同窗口内重叠控件只有最上层响应）；`Sense` 的 `drag` 就是"按下即 `claim_press()`"。

### 3.3 放置即用

```rust
if ui.add(TagButton::new("t1", "标签").bg(Color::ORANGE)).clicked() { … }
```

### 3.4 跨 crate 自定义控件（公开接口清单）

`Widget` trait 与下列 `Ui` 公开原语组成**控件作者接口**——你的 crate 里 `impl Widget`
即可用（完整可编译模板：滑块 + 数字输入（含拖动调值）见 `crate::widget` 模块文档）：

| 分类 | 公开原语（`Ui` 方法） | 用途 |
|---|---|---|
| 主题 | `theme()` / `theme_mut()`（crate 内为字段） | 样式取值 / 逐控件覆盖合并 |
| 测量 | `text_size` / `text_size_wrap` / `wrap_buffer` | 内容测量（**物理像素**；在 `allocate` **之前**调用） |
| 申请 | `allocate` / `allocate_mode` / `allocate_at` / `allocate_sense` / `allocate_sense_at` | 就地在 `ui()` 里申请矩形（物理像素；`Response::rect` = 最终矩形） |
| 交互 | `interact(&绝对ID, rect, Sense)` | 命中 / 焦点 / 按下认领 / `update_interact` / `update_drag` 一次做完（返回 `Response`） |
| 布局 | `child_rect` / `avail_w` / `mouse_local` | 自写"占光标"容器 / 父级可用宽 / 局部鼠标 |
| 命中 | `hit_abs(&绝对ID, &Rect)` / `mouse_left()` / `mouse_logical()` | 点中判定（含窗口遮挡 + **控件级遮挡** + 裁剪过滤）/ 左键状态 / 拖拽基准 |
| 按下归属 | `claim_press()`（或 `Sense::drag`） | **自身有拖拽语义的控件**在按下时调用——阻止外层窗口把本次按下当窗口拖拽基准 |
| 焦点 | `register_focus(&id_for, rect, FocusKind)` / `key_click(&id_for, kind)`（`id_for = ui.id_for(id)` 为**绝对 ID**） | 键盘导航（Tab/Enter/方向键）接入 |
| 状态 | `state_mut().widget(&id_for)` → `WidgetState` + [`hit::update_drag`](crate::hit::update_drag) / [`hit::update_interact`](crate::hit::update_interact) | 跨帧交互状态机（hover/按下/拖拽基准）；**收绝对 ID** |
| 绘制 | `painter()` → `panel / panel_elem / panel_img / solid / border / rounded_at / gradient_at / icon_at / image_at / text`（`push_panel_like` 等旧方法保留为薄包装） | 背景边框 / 文本 / 实心 / 描边（逻辑坐标） |
| 复用 | `button_at_styled` / `checkbox_at_styled` / `slider_at` / `text_input_at` / `text_area_at` / `radio_at` / `combo_at`（= `Dropdown` 的糖） | 委托现有控件（**内置控件同路径**） |
| 浮层 | `widgets::menu::popup_show`（`pub(crate)`）+ `MenuCtx` | **下拉 / 菜单类浮层一律走这里**（哨兵 z / 锁定位置 / 无缩放柄 / 面板样式 / 宽度收敛 / 点外·Esc·点项收起都在里面）；`MenuCtx` 是给应用写菜单内容的上下文（`Deref` 到 `Window` ⇒ 全部 `UiAdd`） |

约定：`rect` 均为**相对当前容器 origin 的局部坐标**；`Theme` 已预乘 DPI ⇒ 内部一律
**物理像素**（`Size` / `Position` 只在公开 API 边界做换算）；交互前先拷出主题值
（Copy / owned）避免借用冲突；**一个绘制块一个 painter**（`ui.painter()` 借 `&mut self`）。

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
   **子样式也全是责任链**：每个子样式结构都有 `with_*` setter（只改链上字段，其余回落
   默认）——逐容器覆盖主题样式无需 clone 整份 Theme：
   ```rust
   let panel = PanelStyle::default().with_radius(8.0).with_bg(Color::rgba_u8(40, 44, 62, 255));
   let btn = ButtonStyle::default().with_bg(c).with_radius(6.0);
   let slider = SliderStyle::default().with_track(c).with_fill(c);
   ```
2. **逐控件属性**（builder setter）：只覆盖你设置的字段，其余回落主题；数值字段接收
   [`Size`](crate::draw::Size)（默认 `Logical`，× scale 换算取整；`Size::Physical` 原样）：
   ```rust
   ui.add(Button::new("b", "红底白字").bg(Color::RED).color(Color::WHITE));
   ui.add(Label::new("16px").font_size(Size::Physical(16.0))); // 显式物理字号
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

- ~~`Slider` / `NumberInput` 的 builder 化~~（已完成：`widget::Slider` 支持 `drag_sensitivity`
  / `shift_speed` / `ctrl_speed`；`NumberInput::new(id, &mut value)` 内部管理显示文本，
  支持 `.range()` / `.step()` / `.shift_speed()` / `.ctrl_speed()`）；
- ~~`NumberInput` 泛型化~~（已完成：与 `Slider` **同一套 `T: SliderValue`** —— `f32` 默认 /
  `f64` / 全部整数类型，由 `&mut T` 推断；整数：默认步进 1、无小数显示、只收整数文本、
  边界饱和。**精度四条**：内部数学用 `f64`；拖动吸附到 `step` 格点后**按十进制位数取整**
  （`v == 0.1` 这类比较成立）；**先吸附再 clamp**；**拖动吸附、打字不吸附且如实显示**。
  细节见 `rjw_ui::NumberInput` 的模块文档 + `docs/API_REFERENCE.md`）；
- `TextInput` / `TextArea` / `Combo` 的 builder 化（同样走 `*_at_styled`）；
- `Response` 扩展（如滑块新值 `Option<f32>`、`drag_delta`）；
- widget 级 `disabled` / `tooltip` 等通用属性。

## 7. 裁剪分层与 noclip 绘制（已落地）

- **强制层（硬裁剪）**：ScrollView 可视区 / Clip 沙箱（`Ui::view_at(…, ViewMode::Clip)`、
  `Placement::Clip`）。`UiDraw.clip` 恒为该层，**所有绘制（含 noclip 变体）都服从**；
- **软层（内容裁剪）**：控件自身内容边界，由调用方显式传参（`push_text_rect` 的局部
  `clip`、文本框内容区）。内容自洽的控件（自动换行 / "…"省略 / 滚动）用
  `push_*_noclip` **跳过软层**——但 ScrollView 强制裁切躲不掉（反例：无 Scroll 的
  普通容器本无强制层，noclip 内容画出界 = 自洽内容本就不会出界）；
- **View 沙箱**（`crate::view`）：闭包作用域 `ui.view_at(pos, size, mode, |v| …)`，
  统一"裁剪层 / 坐标原点 / 可用宽度 / 命中过滤"，`scroll_at`、文本编辑框、严格窗口共用。
- **内部计算一律物理像素**（滚动偏移 / 取整 / 命中）；对外单位可选参数用
  [`Metric`](crate::draw::Metric)（`Physical` / `Logical`，内部换算物理）。
