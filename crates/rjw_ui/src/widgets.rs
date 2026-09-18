//! 控件 trait 与**属性化 builder**（非宏、可调试的控件扩展方式）+ 内置组合控件。
//!
//! # 为什么
//!
//! 旧控件 API 由 `widget_api!` 宏统一生成（`p.label` / `p.button` …）——加一个新控件要
//! 改宏（报错指向宏展开、难以调试），且控件属性（颜色 / 字号 / 字体 / 内边距）只能
//! 跟随全局 [`Theme`](crate::style::Theme)，无法逐控件覆盖。该宏现已移除，
//! 容器便捷方法统一由 [`crate::ui::UiAdd`] trait 提供（默认方法，无需宏展开）。
//!
//! 本模块提供：
//! - [`Widget`] trait：**新控件 = 一个实现该 trait 的 builder 结构体**——普通 Rust，
//!   无宏展开，报错定位精确，可单测；
//! - 属性化 builder（**按控件拆分到独立子文件**）：[`label`]`::Label` /
//!   [`button`]`::Button` / [`checkbox`]`::Checkbox` / [`divider`]`::Divider` /
//!   [`slider`]`::Slider`——`Option` 覆盖字段 + 链式 setter，未设置的属性回落到全局
//!   [`Theme`](crate::style::Theme)；
//! - 内置**组合控件**：`numberinput`（[`NumberInput`]）/ `colorpicker`（[`ColorPicker`]）
//!   / `fontmodal`（[`FontModal`]），由基础原语组合而成，同时是"跨 crate 自定义控件"
//!   的真实范例；
//! - 统一响应 [`Response`]（hover / pressed / clicked / released / toggled）；
//! - 放置方式：[`Ui::add`](crate::ui::Ui::add)（容器内占光标）/
//!   [`Ui::add_at`](crate::ui::Ui::add_at)（绝对定位）；容器包装（`Panel` / `Pack` /
//!   `Grid` / `Window` / `Scroll` / `FlexCtx`）经 [`crate::ui::UiAdd`] 提供同样的
//!   `add` / `add_at` 与全部便捷方法。
//!
//! `lib.rs` 在 crate 根重导出全部控件（`rjw_ui::Button` 等），下游通常无需直接引用
//! 本模块路径。
//!
//! # 自定义 `Widget` 指南：Easy → Powerful
//!
//! ## 0. 30 秒版（Easy）
//!
//! 1. `struct MyWidget<'a> { id: &'a str, /* 属性用 Option 存差异 */ }`
//!    —— **builder 约定**：没设的属性回落 [`Theme`](crate::style::Theme)，于是主题一改
//!    所有控件一起变（内置控件全是这个形状）；
//! 2. `impl Widget for MyWidget`，只需要两个方法：
//!    - `fn size(&self, ui) -> Vec2`：量尺寸（文字用 `ui.text_size`，其它读主题常量）；
//!    - `fn ui(self, ui, rect) -> Response`：在 `rect` 里画 + 收交互。
//! 3. 放进去：`ui.add(w)`（容器内占光标）/ `ui.add_at(pos, w)`（绝对定位）；
//!    容器包装（`Pack` / `Panel` / `Grid` / `Window` / `Scroll`）经
//!    [`crate::ui::UiAdd`] 提供同样的 `add` / `add_at`。
//!
//! **没有宏、没有注册表、没有 trait object、没有生命周期魔法**——就是一个普通 Rust
//! 结构体 + 一个 trait 实现（这也是旧 `widget_api!` 宏被移除的原因：宏展开的报错
//! 指向宏内部，而这里报错直接指向你自己的字段）。
//!
//! ### 例子：标签块（可直接编译）
//!
//! ```no_run
//! use glam::Vec2;
//! use rjw_color::Color;
//! use rjw_transform::Rect;
//! use rjw_ui::draw::TextVAlign;
//! use rjw_ui::{Response, TextAlign, Ui, Widget};
//!
//! /// 标签块：圆角底 + 居中文字（**最小可用自定义控件**）。
//! pub struct Tag<'a> {
//!     id: &'a str,
//!     text: &'a str,
//!     /// 逐控件覆盖；`None` = 跟主题。
//!     fg: Option<Color>,
//! }
//!
//! impl<'a> Tag<'a> {
//!     pub fn new(id: &'a str, text: &'a str) -> Self {
//!         Self { id, text, fg: None }
//!     }
//!     /// 属性化 builder：只存差异，未设的回落主题。
//!     pub fn color(mut self, c: Color) -> Self {
//!         self.fg = Some(c);
//!         self
//!     }
//! }
//!
//! impl Widget for Tag<'_> {
//!     fn size(&self, ui: &mut Ui) -> Vec2 {
//!         let st = ui.theme().label.clone();
//!         let t = ui.text_size(self.text, st.font_size, st.font_family.as_deref());
//!         Vec2::new(t.x + 16.0, ui.theme().row_h) // 文字宽 + 左右内边距
//!     }
//!
//!     fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
//!         // 身份 = **绝对 id**（控件级遮挡按它区分"谁盖住谁"，见第 7 条硬约定）。
//!         let abs = ui.id_for(self.id);
//!         // 命中（含窗口遮挡 / 控件级遮挡 / 强制裁剪层）。
//!         let hovered = ui.hit_abs(&abs, &rect);
//!         let st = ui.theme().button.clone();
//!         ui.push_panel_like(rect, st.pick_bg(false, hovered), st.border, st.border_w, st.radius, 1);
//!         ui.push_text_rect(
//!             rect,
//!             self.text,
//!             st.font_size,
//!             self.fg.unwrap_or(st.fg),
//!             None,
//!             TextAlign::Center,
//!             TextVAlign::Center,
//!             None,
//!             None,
//!         );
//!         Response { hovered, ..Default::default() }
//!     }
//! }
//!
//! # fn demo(ui: &mut Ui) {
//! ui.add(Tag::new("tag_demo", "新").color(Color::WHITE));
//! # }
//! ```
//!
//! ### 什么时候需要重写 `constraints()` / `expansion()`
//!
//! | 需求 | 用 |
//! |---|---|
//! | 内容可能超出父级（长文本 / 列表）：取 `min(内容, 可用宽)` 后**自己**自洽绘制 | [`Expansion::LimitedInParent`]（配 `ui.avail_w()` 换行 / 省略） |
//! | 不撑大父级（分隔线 / 装饰件） | [`Expansion::DisableAutoExpansion`] |
//! | 有压缩下限 / 上限（可缩放输入框） | [`Widget::constraints`] + [`SizeConstraints`] |
//!
//! ## 1. Powerful：把交互做完整
//!
//! | 想要 | 用什么 |
//! |---|---|
//! | 悬停 / 按下 / 点击 | `ui.hit_abs(&abs, &rect)` + `ui.mouse_left()` + [`hit::update_interact`](crate::hit::update_interact) → [`InteractEvents`](crate::hit::InteractEvents) |
//! | 拖拽（自带语义） | 按下时 `ui.claim_press()` + [`hit::update_drag`](crate::hit::update_drag) + 基准存 `WidgetState::{press_panel, press_mouse}` |
//! | 2D / 竖向"点哪取哪" | `ui.mouse_local()` + [`hit::normalize_x`](crate::hit::normalize_x) / [`hit::normalize_y`](crate::hit::normalize_y) |
//! | 键盘 / Tab 导航 | `ui.register_focus(&ui.id_for(id), rect, [`FocusKind`])` + `ui.key_click(&abs, kind)` |
//! | 跨帧状态（交互） | `ui.state_mut().widget(&abs)` → [`WidgetState`](crate::state::WidgetState)（hovered/pressed/dragging/caret/scroll…） |
//! | 跨帧数据（**自己的类型**） | 定义在**控件自己的模块**里、挂到 [`UiState`](crate::UiState)（范例：`ColorPickerState`）——别把控件层的事实塞进 `ui.rs` |
//! | 文本输入（焦点 / 选择 / IME / 剪贴板全套） | `ui.text_input_at(id, rect, &mut String)`；不想让调用方持有 `String` 就学 `NumberInput`：编辑缓冲 take/写回 `WidgetState` |
//! | 浮层 / 弹出面板 | `ui.window("id::popup")` + z 哨兵 `WIN_TOPMOST`（`ui.state_mut().window_z.insert(..)`），范例见 `colorpicker/panel.rs` |
//! | 绘制原语 | `push_panel_like`（圆角 + 刷 + 边框）/ `rounded_rect_at` / `gradient_rect_at` / `icon_at` / `image_at` / `push_text_rect` / `push_solid_rect` / `push_border_rect` / `debug_*` |
//! | **画在自家背景之上的装饰**（手柄 / 箭头 / 分隔线） | `ui.elem_hint()` + `push_panel_like(.., elem)` —— 见下面第 6 条硬约定 |
//! | 裁剪 | 容器强制层（`Scroll` / Clip 沙箱）自动生效；`push_text_rect` 另加内容裁剪，自洽内容用 `push_text_rect_noclip` |
//! | 光标 | `ui.set_cursor(UiCursor::EwResize)`（悬停 / 拖拽时；`finish` 统一落到系统光标） |
//! | 数值 / 颜色等热路径数学 | 抽成**自由函数**放自己的子模块里单测（范例：`colorpicker/{format,hsv}.rs`） |
//!
//! ### 例子：旋钮（拖拽 + 键盘 + 跨帧状态，可直接编译）
//!
//! ```no_run
//! use glam::Vec2;
//! use rjw_transform::Rect;
//! use rjw_ui::draw::TextVAlign;
//! use rjw_ui::hit::{update_drag, update_interact};
//! use rjw_ui::{FocusKind, Response, TextAlign, Ui, Widget};
//! use winit::keyboard::KeyCode;
//!
//! /// 旋钮：**竖直拖动改值** + `↑`/`↓` 微调（演示交互状态机 / 焦点 / 跨帧状态）。
//! pub struct Knob<'a> {
//!     id: &'a str,
//!     value: &'a mut f32,
//!     range: (f32, f32),
//! }
//!
//! impl<'a> Knob<'a> {
//!     pub fn new(id: &'a str, value: &'a mut f32, range: (f32, f32)) -> Self {
//!         Self { id, value, range }
//!     }
//! }
//!
//! impl Widget for Knob<'_> {
//!     fn size(&self, _ui: &mut Ui) -> Vec2 {
//!         Vec2::new(30.0, 30.0)
//!     }
//!
//!     fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
//!         // ① 状态键必须是**绝对 id**（`id_for` 返回 `IdAbsolute`；嵌套里同名相对 id 会互相踩）。
//!         let abs = ui.id_for(self.id);
//!         // ② 进焦点链：Tab 可到、焦点描边由引擎画。
//!         ui.register_focus(&abs, rect, FocusKind::Slider);
//!
//!         let hit = ui.hit_abs(&abs, &rect);
//!         let btn = ui.mouse_left();
//!         // ③ 拖拽语义：按下就占用本次按压，外层窗口/面板不会把它当成窗口拖动。
//!         if btn.down_edge() && hit {
//!             ui.claim_press();
//!         }
//!         // ④ 先拷出鼠标（`&self`），再借 `state_mut()`——借用顺序不能颠倒。
//!         let mouse = ui.mouse_screen();
//!         let ev = {
//!             let ws = ui.state_mut().widget(&abs);
//!             let ev = update_interact(ws, hit, btn);
//!             if btn.down_edge() && hit {
//!                 // 拖拽基准：按下时的鼠标 + 按下时的值（与 `NumberInput` 手柄同一套）。
//!                 ws.press_mouse = Some(mouse);
//!                 ws.press_panel = Some(Vec2::new(0.0, *self.value));
//!             }
//!             ev
//!         };
//!         let dragging = {
//!             let ws = ui.state_mut().widget(&abs);
//!             update_drag(ws, hit, btn)
//!         };
//!         if dragging {
//!             let (base, pm) = {
//!                 let ws = ui.state_mut().widget(&abs);
//!                 (ws.press_panel.unwrap_or_default().y, ws.press_mouse.unwrap_or(mouse))
//!             };
//!             // Shift = 细调（10×）：**按下时改倍率会跳**，实际控件会重设基准（见 NumberInput）。
//!             let fine = if ui.key_down(KeyCode::ShiftLeft) { 0.1 } else { 1.0 };
//!             let span = self.range.1 - self.range.0;
//!             let d = (mouse.y - pm.y) * span * 0.005 * fine;
//!             *self.value = (base - d).clamp(self.range.0, self.range.1);
//!         }
//!         // ⑤ 键盘：焦点在自己身上时用 `key_down_edge`（纯输入，状态自己维护）。
//!         let focused = ui.state().focused.as_ref().is_some_and(|f| f.as_str() == abs.as_str());
//!         if focused {
//!             let step = (self.range.1 - self.range.0) * 0.02;
//!             if ui.key_down_edge(KeyCode::ArrowDown) {
//!                 *self.value = (*self.value - step).clamp(self.range.0, self.range.1);
//!             }
//!             if ui.key_down_edge(KeyCode::ArrowUp) {
//!                 *self.value = (*self.value + step).clamp(self.range.0, self.range.1);
//!             }
//!         }
//!         // ⑥ 绘制：公开原语组合（圆角 + 指针 + 数值文本），坐标 = `rect` 局部。
//!         let t = ((*self.value - self.range.0) / (self.range.1 - self.range.0)).clamp(0.0, 1.0);
//!         ui.rounded_rect_at(
//!             Vec2::new(rect.x + 1.0, rect.y + 1.0),
//!             Vec2::new(rect.w - 2.0, rect.h - 2.0),
//!             7.0,
//!             ui.theme().palette.surface_raised,
//!         );
//!         ui.rounded_rect_at(
//!             Vec2::new(rect.x + (rect.w - 6.0) * t, rect.y + rect.h - 8.0),
//!             Vec2::new(6.0, 5.0),
//!             2.0,
//!             ui.theme().palette.accent,
//!         );
//!         ui.push_text_rect(
//!             rect,
//!             &format!("{:.0}", *self.value * 100.0),
//!             11.0,
//!             ui.theme().label.color,
//!             None,
//!             TextAlign::Center,
//!             TextVAlign::Top,
//!             None,
//!             None,
//!         );
//!         Response { hovered: hit, pressed: dragging, clicked: ev.clicked, ..Default::default() }
//!     }
//! }
//!
//! # fn demo(ui: &mut Ui, v: &mut f32) {
//! ui.add(Knob::new("gain", v, (0.0, 1.0)));
//! # }
//! ```
//!
//! ## 2. 八条硬约定（都是踩过的坑）
//!
//! 1. **尺寸一律物理像素**：`Theme` 在 [`Ui`] 内已按 DPI 预乘；要收逻辑单位就用
//!    [`Position`](crate::Position) / [`Size`](crate::Size)（或 [`Metric`](crate::Metric)）。
//! 2. **状态键必须绝对 id**：`ui.id_for(id)` 得到 `IdAbsolute`；同一控件的子部件用
//!    `"{id}::grip"` 这类派生名（`::` 是约定）——**不要**多个部件共用同一个 id，
//!    否则 `press_mouse` / `caret` 互相覆盖（`NumberInput` 的手柄另起 `::grip` 就是这个原因）。
//! 3. **输入 ≠ 状态**：`hit_abs` / `mouse_left` / `key_down_edge` 只是"这一帧的事实"；
//!    悬停 / 按下 / 拖拽要经 `update_interact` / `update_drag` 落到 `WidgetState`。
//! 4. **有拖拽语义就必须 `claim_press()`**：否则外层窗口 / 面板会把这次按下当作拖动基准
//!    （窗口里的滑块会连窗口一起动）。反之，纯点击控件**不要**调用它。
//! 5. **`size()` 每帧都会跑**：别在里面做重活；文本测量走 `ui.text_size`（内部有缓存），
//!    id 拼接用 `&str` + 必要时一次 `format!`（热路径控件学 `NumberInput`：`fmt_step` 一次成型）。
//! 6. **"画在自家背景之上"的装饰要取 `ui.elem_hint()`**：`elem` 决定同一窗口内的绘制
//!    顺序（元素序小的**先画**）。`push_panel_like` / `push_text_rect` / 滑块都取
//!    "录制时的 `seq + 1`"，而 `push_draw` 默认 `elem = 0`（容器装饰，画在所有元素之下）；
//!    手柄 / 箭头 / 分隔线若写死 `0` 或 `1`，会被本控件自己的背景或文本框**整块盖住**
//!    （两个真实 bug：`NumberInput` 的拖拽手柄与分隔线、`ColorPicker` 的展开箭头
//!    全都看不见）。
//! 7. **`hit_abs` 必须传自己的绝对 id**：`ui.hit_abs(&ui.id_for(id), &rect)`。它不只是
//!    命中测试——引擎按这个身份做**控件级遮挡**：同窗口内**后录制的控件画在上面**，
//!    重叠处只有最上层那个控件被触发（"点滚动条却连带选中了下面的列表项 / 两个控件
//!    被一起触发"就是这么修的，诊断计数 `UiState::widget_occluded_hits`）。
//!    ⚠ 传**容器** id 会让同容器内所有控件"互不遮挡"（重叠时又一起触发）；传**别人**
//!    的 id 会让自己永远被那个控件挡住。同一控件的多个可交互区（轨道 + 手柄）**共用**
//!    同一个 id，才不会被自己挡住。
//! 8. **画在 `rect` 之外的部分不算你的**：容器尺寸按"子项矩形"结算
//!    （`Frame::content_bounds`），绝对放置（`add_at` / `*_at` / 绝对容器）也会被上报；
//!    但**自己往 rect 外画的装饰**不会撑大容器 ⇒ 可能落在窗口外面（被别的窗口
//!    遮挡、或裁剪掉）。要装饰溢出就自己扩宽 `size()` / 用 `Expansion::UnlimitedExpansion`，
//!    或者接受它只是"看得见点不着"的视觉装饰。
//!
//! ## 3. 测试与调试
//!
//! - **把纯逻辑抽成自由函数**（数值映射 / 解析 / 几何换算）放进自己的子模块，配
//!   `#[cfg(test)] mod tests`：本仓的单测**不需要 GPU**，`cargo test -p rjw_ui` 直接跑
//!   （范例：`ColorPicker` 的 `format.rs` / `hsv.rs` / `state.rs`）。
//! - `ui.debug_layout(true)`：给每个绘制命令的矩形画青色描边——布局矩形与你以为的
//!   命中区不一致时一眼可见（控件 `size()` 与实际绘制内容不一致是经典的布局重叠来源）。
//! - `ui.debug_dump()`（示例 `--ui-dump`）：窗口 id / z / 本帧提交原点 / 尺寸 / 拖拽状态。
//! - 命中不生效？先查**窗口遮挡**（`UiState::occluded_hits`，重叠窗口只让最上层可交互）、
//!   **控件级遮挡**（`UiState::widget_occluded_hits`，同窗口内重叠控件只让最上层可交互）
//!   与**强制裁剪层**（`Scroll` / Clip 沙箱外命中失效）。
//! - **别在 `down_edge` 上直接执行一次性动作**：命中那一刻本帧几何可能还没录完（盖住你的
//!   窗口本帧才移过来 / 才被抬高 z），引擎会在**帧末**用完备的遮挡表复核并撤销这次认领
//!   （清 `pressed` / `clicked` / `dragging`，计数 `UiState::press_cancelled_by_window()`）
//!   ——**状态**会被回滚，但你在那一帧已经执行的副作用回不来。用
//!   `hit::update_interact` + `Response::clicked`（释放帧才成立）就天然安全。


use glam::Vec2;
use rjw_transform::Rect;

use crate::state::ButtonState;
use crate::ui::Ui;

mod button;
mod checkbox;
mod colorpicker;
mod divider;
/// **按钮下拉菜单**（`Widget`：`UiAdd::add(Dropdown::…)`；菜单内又是 `UiAdd`）。
pub mod dropdown;
mod fontmodal;
mod label;
pub mod menu;
mod numberinput;
mod slider;
pub(crate) mod title_button;
/// **菜单栏**（横向触发器 + 闭包下拉面板；状态在 `UiState.menu_open`）。
pub mod menubar;
/// **分段按钮组**（互斥选项拼在一起；分隔线与 `border_w` 解耦）。
mod segmented;

pub use button::Button;
pub use checkbox::Checkbox;
pub use colorpicker::{
    ColorFormat, ColorPicker, ColorPickerState, color_hex, format_color, format_f, format_u8,
    ink_on, luma, parse_color, parse_hex,
};
pub use divider::Divider;
pub use dropdown::Dropdown;
pub use fontmodal::{FONT_WEIGHT_CHOICES, FontModal, weight_label};
pub use menu::{
    Item, MENU_GAP, MenuClick, MenuContent, MenuCtx, MenuFn, PopupSide, item_h, popup_gap,
    popup_origin, popup_padding,
};
pub use menubar::MenuBar;
pub use segmented::Segmented;
pub use label::Label;
pub use numberinput::{GRIP_W, NumberInput};
pub use slider::{Slider, SliderValue};

// ─── 统一响应 ───────────────────────────────────────────────────

/// 控件统一交互响应（hover / pressed / clicked / released / toggled）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Response {
    /// 鼠标悬停在本体（含按下时）。
    pub hovered: bool,
    /// 处于按下状态（按下后未释放）。
    pub pressed: bool,
    /// 本帧完成一次点击（按下 + 释放均在本体内；键盘 Enter/Space 激活同理）。
    pub clicked: bool,
    /// 本帧释放（无论释放位置）。
    pub released: bool,
    /// 勾选 / 单选类控件：本帧是否切换（其余控件恒 `false`）。
    pub toggled: bool,
}

impl Response {
    #[inline]
    pub fn hovered(&self) -> bool {
        self.hovered
    }
    #[inline]
    pub fn pressed(&self) -> bool {
        self.pressed
    }
    #[inline]
    pub fn clicked(&self) -> bool {
        self.clicked
    }
    #[inline]
    pub fn released(&self) -> bool {
        self.released
    }
    #[inline]
    pub fn toggled(&self) -> bool {
        self.toggled
    }
}

impl From<ButtonState> for Response {
    fn from(s: ButtonState) -> Self {
        Self {
            hovered: s.hovered,
            pressed: s.pressed,
            clicked: s.clicked,
            released: s.released,
            toggled: false,
        }
    }
}

// ─── Widget trait ───────────────────────────────────────────────

/// **控件尺寸约束**（每轴可选；`None` = 该轴不约束）。见 [`Widget::constraints`]。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizeConstraints {
    pub min_w: Option<f32>,
    pub max_w: Option<f32>,
    pub min_h: Option<f32>,
    pub max_h: Option<f32>,
}

/// **控件膨胀模式**（内容尺寸相对父级空间的行为）。见 [`Widget::expansion`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Default)]
pub enum Expansion {
    /// 内容按自身尺寸（clamp min/max），**不撑大父级**——内容可能溢出父级，
    /// 由控件用 noclip 绘制 / 省略自洽（如装饰性分隔线）。
    DisableAutoExpansion,
    /// **限制在父级可用空间内**：取 min(内容, 沙箱可用宽，`Ui::avail_w`)，超出
    /// 部分由控件自处理（Label 自动换行 / "…"省略、Button 省略、TextArea 滚动）；
    /// 无可用空间时退化为 [`Expansion::UnlimitedExpansion`]。
    LimitedInParent,
    /// 内容自然尺寸（clamp min/max），**撑大父级**（默认，DOM 语义）。
    #[default]
    UnlimitedExpansion,
}


/// **控件 trait**：新控件 = 实现此 trait 的 builder 结构体（普通 Rust，无宏）。
///
/// - [`Widget::size`]：期望尺寸（**逻辑像素**；内容测量可调用 `ui.text_size` /
///   `ui.text_size_wrap`，或读取样式常量）；
/// - [`Widget::ui`]：在分配好的矩形内渲染 + 交互，返回 [`Response`]。
///
/// 放置：[`Ui::add`](crate::ui::Ui::add)（容器内占光标）/
/// [`Ui::add_at`](crate::ui::Ui::add_at)（绝对定位）；容器包装经
/// [`crate::ui::UiAdd`] 提供同样的 `add` / `add_at` 与全部便捷方法（`p.button` 等）。
pub trait Widget {
    /// 期望尺寸（逻辑像素；内容测量可经 `&mut Ui` 排版/缓存）。
    fn size(&self, ui: &mut Ui) -> Vec2;

    /// 在 `rect`（相对当前容器内容原点，逻辑像素）内渲染 + 交互。
    fn ui(self, ui: &mut Ui, rect: Rect) -> Response;

    /// **尺寸约束**（每轴 `Option<f32>`；默认全 `None`，不约束）。
    /// `Ui::add` 在布局前对 `size()` 结果按此 clamp。
    fn constraints(&self) -> SizeConstraints {
        SizeConstraints::default()
    }

    /// **膨胀模式**（默认 [`Expansion::UnlimitedExpansion`]）：
    /// 决定内容是否撑大父级 / 是否限制在父级可用空间内（见 [`Expansion`]）。
    fn expansion(&self) -> Expansion {
        Expansion::UnlimitedExpansion
    }
}

/// 应用尺寸约束：`natural` 每轴 clamp 到 `min`/`max`（纯函数，可单测）。
/// 顺序 = 先压 max 再抬 min（**min 恒优先**：`min > max` 时结果为 min）。
#[inline]
pub fn apply_constraints(natural: Vec2, c: SizeConstraints) -> Vec2 {
    let clamp = |v: f32, lo: Option<f32>, hi: Option<f32>| {
        let v = match hi {
            Some(hi) => v.min(hi),
            None => v,
        };
        match lo {
            Some(lo) => v.max(lo),
            None => v,
        }
    };
    Vec2::new(
        clamp(natural.x, c.min_w, c.max_w),
        clamp(natural.y, c.min_h, c.max_h),
    )
}

// ─── 控件 ID（Label 派生 / 字符串 / 数字） ──────────────────────

/// 控件 ID（跨帧状态键，如勾选框勾选状态）的三种来源。
///
/// - [`WidgetId::Label`]：用 **label 文本本身**作 ID——同容器内标签唯一时最简；
/// - [`WidgetId::String`]：显式字符串 ID（原 `&str` 参数）；
/// - [`WidgetId::Int`]：数字 ID（如列表行索引 `i as u64`）。
///
/// 便捷转换（[`From`]）：`None` → `Label`；`Some("id")` / `"id"` → `String`；
/// `42u64` → `Int`。示例见 [`crate::ui::UiAdd::checkbox_mut`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetId<'a> {
    /// ID = label 文本本身（唯一标签无需显式 ID）。
    Label,
    /// 显式字符串 ID。
    String(&'a str),
    /// 数字 ID。
    Int(u64),
}

impl<'a> From<Option<&'a str>> for WidgetId<'a> {
    #[inline]
    fn from(id: Option<&'a str>) -> Self {
        match id {
            Some(s) => WidgetId::String(s),
            None => WidgetId::Label,
        }
    }
}
impl<'a> From<&'a str> for WidgetId<'a> {
    #[inline]
    fn from(s: &'a str) -> Self {
        WidgetId::String(s)
    }
}
impl From<u64> for WidgetId<'_> {
    #[inline]
    fn from(i: u64) -> Self {
        WidgetId::Int(i)
    }
}

impl WidgetId<'_> {
    /// 解析为实际 ID 字符串（`Label` 用 label 文本；`Int` 转十进制）。
    pub(crate) fn resolve(&self, label: &str) -> String {
        match self {
            WidgetId::Label => label.to_owned(),
            WidgetId::String(s) => (*s).to_owned(),
            WidgetId::Int(i) => i.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_id_resolve() {
        // Label：以 label 文本为 ID
        assert_eq!(WidgetId::Label.resolve("窗口 A 选项"), "窗口 A 选项");
        // String：显式字符串 ID（忽略 label）
        assert_eq!(WidgetId::String("win_b_cb").resolve("任意"), "win_b_cb");
        // Int：数字 ID
        assert_eq!(WidgetId::Int(7).resolve("任意"), "7");
        // From 转换：None → Label、Some/&str → String、u64 → Int
        assert_eq!(WidgetId::from(None::<&str>).resolve("label-x"), "label-x");
        assert_eq!(WidgetId::from(Some("id-y")).resolve("label-x"), "id-y");
        assert_eq!(WidgetId::from("id-z").resolve("label-x"), "id-z");
        assert_eq!(WidgetId::from(42u64).resolve("label-x"), "42");
        // 区分度：同容器内不同 label 的 Label ID 互不相同
        assert_ne!(
            WidgetId::Label.resolve("窗口 A"),
            WidgetId::Label.resolve("窗口 B")
        );
    }

    #[test]
    fn apply_constraints_clamps_each_axis() {
        // 无约束：原样
        assert_eq!(apply_constraints(Vec2::new(10.0, 20.0), SizeConstraints::default()), Vec2::new(10.0, 20.0));
        // min 抬升
        let c = SizeConstraints { min_w: Some(30.0), min_h: Some(40.0), ..Default::default() };
        assert_eq!(apply_constraints(Vec2::new(10.0, 20.0), c), Vec2::new(30.0, 40.0));
        // max 压缩
        let c = SizeConstraints { max_w: Some(50.0), max_h: Some(60.0), ..Default::default() };
        assert_eq!(apply_constraints(Vec2::new(100.0, 200.0), c), Vec2::new(50.0, 60.0));
        // 单轴独立
        let c = SizeConstraints { max_w: Some(50.0), ..Default::default() };
        assert_eq!(apply_constraints(Vec2::new(100.0, 20.0), c), Vec2::new(50.0, 20.0));
        // min > max 时 min 优先（clamp 顺序）
        let c = SizeConstraints { min_w: Some(80.0), max_w: Some(50.0), ..Default::default() };
        assert_eq!(apply_constraints(Vec2::new(10.0, 0.0), c), Vec2::new(80.0, 0.0));
    }

    #[test]
    fn expansion_default_is_unlimited() {
        assert_eq!(Expansion::default(), Expansion::UnlimitedExpansion);
    }
}
