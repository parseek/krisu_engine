//! `rjw_ui`：krusie 引擎的 UI 模块。
//!
//! # 特性
//!
//! - **hybrid 模式**：外观立即录制（每帧 `Ui::begin` → 控件 → `finish`），
//!   交互状态（hover / 按下 / 焦点 / 输入内容 / 拖拽 / 单选组 / grid 单元格缓存）
//!   经 **ID** 持久化在 [`UiState`]（应用持有，跨帧复用）。
//! - **DOM 风格自动尺寸**：叶子控件由内容测量自然撑开，容器（panel / pack / grid）
//!   闭包结束时按子控件结算自身尺寸——默认无需手写宽高；任何控件可传显式 `Rect` 覆盖。
//! - **Tkinter 风格几何管理器**：[`PackSide`] 堆叠（`pack_at`）、均匀网格（`grid_at`）、
//!   绝对定位（`*_at`）。
//! - **屏幕空间**：坐标一律为屏幕像素（左上角原点、Y+ 向下）；内部经视口
//!   （[`rjw_transform::Rect`]：屏幕矩形）的屏幕固定变换绘制，命中测试
//!   直接在屏幕像素进行（旋转/缩放的世界相机不影响 UI——UI 恒为 identity）。
//! - **Debug UI / DebugDraw**：`debug_layout` 开关为每个控件/容器的布局矩形与命中
//!   区域画描边（调试 `rjw_ui` 自身）；[`Ui::debug_line`] / [`Ui::debug_rect_outline`] /
//!   [`Ui::debug_circle_outline`] / [`Ui::debug_cross`] / [`Ui::debug_grid`] 提供
//!   **屏幕空间**调试图元（覆盖在 UI 内容之上）。世界坐标调试图元（游戏场景）
//!   见 `rjw_2d_render::debug_draw`，示例见 `examples/egDebugDraw`。
//! - **调试样式**（[`Theme::debug`] / [`crate::style::DebugStyle`]）：`debug_layout`
//!   描边的颜色与宽度可配置（`theme.debug.layout_outline` / `layout_outline_width`，
//!   宽度为物理像素）；DebugDraw 图元（`ui.debug_*`）的样式 = 每次调用显式传
//!   `color` + `width`（逻辑像素）。
//! - **窗口诊断**（重叠点击排查）：[`Ui::window_order`]（z 序）、
//!   [`Ui::window_under_mouse`]（鼠标下最上层窗口）、[`UiState::last_press_window`]
//!   （上次按下接收窗口）、[`UiState::occluded_hits`]（被窗口遮挡拦截的命中次数）——
//!   示例 `eg260818UI` 右上角有实时诊断面板。
//! - **渲染增强（圆角 / 渐变）**：`Theme` 子样式的 `radius`（面板 / 窗口 / 按钮 /
//!   输入框，逻辑像素，0 = 直角）与绘制原语 [`Ui::rounded_rect_at`] /
//!   [`Ui::gradient_rect_at`]。**两者都不需要纹理，也不需要改着色器**：
//!   渐变把四角颜色放进顶点色，由光栅化器双线性插值（`pure` / `vertical` /
//!   `horizontal` / `rotated` / `corners`）；圆角由 CPU **镶嵌**成三角形
//!   （`tess` 模块：单位四分之一圆弧表 + 步长抽样 + 沿整圈带状化的羽化边缘，
//!   含四条直边；羽化宽见 [`Theme::feather`]，0 = 硬边）。硬体 `alpha = 1`、外环
//!   `alpha = 0`，靠插值得到抗锯齿边缘。**背景颜色按顶点位置双线性 lerp**——圆角弧上的
//!   顶点也按位置取色，所以整块渐变与矩形渐变一致（不会"两端发平、渐变被拉长"）。
//!   四角可同色亦可各异 ⇒ 「圆角 + 渐变」自然成立。
//!   边框颜色是 [`Palette`] 令牌，边框宽度用 `Theme::with_border_w`。
//!   图形与文字同页同纹理时仍合批。
//! - **提交粒度**：一个批次 = 一个实例 = `(窗口 × 纹理)`，顶点 + 索引直出三角形；
//!   提交分组 `(win, 图形/文字组, 纹理)` 保证"先图形后文字"。
//! - **窗口级合批 + 窗口级 FX**：`finish` 提交按窗口聚合（每窗口每纹理合并成整段、
//!   一次 `draw_indexed`，命中 Render2D 的 QuadVertices 合批；不同纹理/状态/超顶点上限
//!   自动切段，**尽力而为**）。[`Ui::window_fx`]（[`WindowFx`]：tint + transform override）
//!   给每个窗口整窗混合色与叠加变换——顶点缓存不变、仅提交时应用，支撑整窗口动画
//!   （淡入淡出 / 整体位移缩放旋转 / 整窗染色）。
//! - **控件 trait（非宏）**：[`widgets::Widget`] + 属性化 builder（[`widgets::Label`] /
//!   [`widgets::Button`] / [`widgets::Checkbox`] / [`widgets::Divider`]）——新控件 = 普通
//!   Rust 结构体实现 trait，无 `macro_rules!` 展开（报错定位精确、可单测）；逐控件覆盖
//!   颜色 / 字号 / 字体 / 内边距等属性（未设置回落全局 [`Theme`]），统一
//!   [`widgets::Response`] 响应，经 [`Ui::add`] / [`Ui::add_at`]（容器包装见
//!   [`ui::UiAdd`]）放置。
//! - **Widget 尺寸契约**：[`widgets::SizeConstraints`]（`min_w/max_w/min_h/max_h` 四字段
//!   全 `Option<f32>`）+ [`widgets::Expansion`]（`DisableAutoExpansion` /
//!   `LimitedInParent` / `UnlimitedExpansion`）+ [`widgets::Widget::resizable`]
//!   （可选拖拽缩放，[`Ui::resize_handle`] 通用原语 + [`UiState::sizes`] 持久尺寸）——
//!   `Ui::add` 统一 clamp / 膨胀调整。
//! - **View 沙箱**（[`view`]）：闭包作用域 [`Ui::view_at`]，`ViewMode::{Expand, Clip}`——
//!   裁剪分层（**强制层** = ScrollView 可视区 / Clip 沙箱，所有绘制含 noclip 都服从；
//!   **软层** = 控件自身内容边界，自洽控件可跳过）、可用宽度（[`Ui::avail_w`]）、
//!   命中过滤（Clip 沙箱外命中失效并入 `hit_abs`）。**ScrollView**（[`Ui::scroll_at`]、
//!   文本编辑框）与严格窗口（[`Ui::window`] + `Placement::Clip`）共用底座。
//! - **滚动容器（ScrollView）**：[`Ui::scroll_at`]——内容在可视区内堆叠 + 滚轮 /
//!   滚动条（拖 thumb、点轨道翻页）滚动，可视区外**强制裁剪**；滚动偏移持久于
//!   [`UiState::scrolls`]（**物理像素**——内部计算一律物理，DPI 只在 API 边界换算；
//!   对外单位可选参数用 [`draw::Metric`]）。沙箱内 `avail_w()` = 可视区宽。
//! - **键盘导航**：**Tab / Shift+Tab / 方向键**遍历焦点链（[`UiState::focused`]），
//!   **Enter / Space** 激活焦点控件（按钮 / 勾选 / 单选 / 下拉框），滑块用左右方向键
//!   调值、下拉框展开时上下方向键切换选项，**Esc** 收起浮层 / 取消焦点；焦点控件
//!   画描边（`Theme::focus`，[`crate::style::FocusStyle`]）。
//! - **布局增强**：[`Ui::label_wrap_at`]（宽度内自动**换行**的标签，含容器内
//!   `p.label_wrap`）、**min/max 尺寸约束**（`p.min_size` / `p.max_size`，作用于下一
//!   子项）、**flex 权重**（[`Ui::flex_at`]：固定总高按权重等分子项，同帧精确分配）、
//!   **分割线**（`p.divider()` / [`Ui::divider_at`] / [`widgets::Divider`]，
//!   `Theme.divider`）、**水平行**（`p.row(...)`：`{Label} {Input} {Button}` 占一行的
//!   水平排列，宽 = 子项结算、撑大父级）。
//! - **文本输入增强**：单行输入框**超长滚动跟随光标**（滚轮自由滚动不被光标拉回；
//!   指针离开输入框不再滚动）、**拖选 + Ctrl+C/V/X 复制粘贴剪切 + 双击按词选择**
//!   （[`crate::edit`] 纯逻辑：`apply_frame_edits` / `caret_horiz` / `word_range` /
//!   `extend_word_caret`）、**多行 TextArea**（[`Ui::text_area_at`]：Enter 换行 /
//!   ↑↓ 跨行 / 自动换行 + 垂直滚动 + **滚动条**）、**IME 组合候选浮动提示框**（preedit
//!   画在输入框下方浮动小框，不再占行内）；**Label 溢出**：默认在父级可用宽内自动
//!   换行，[`widgets::Label::ellipsis`] 显式"…"省略，Button/勾选/下拉文本自动省略
//!   （绘制 noclip 变体 [`Ui::push_text_rect_noclip`]：内容自洽，仍服从 ScrollView
//!   强制层）。
//!
//! # 快速上手
//!
//! ```no_run
//! # let viewport = todo!(); let mouse = todo!(); let keyboard = todo!();
//! # let text = todo!(); let state = todo!(); let window = todo!();
//! use rjw_ui::{PackSide, RecordingBackend, Theme, Ui, UiAdd};
//! # let mut backend = RecordingBackend::default();   // 真实项目用 `rjw_krusie` 的桥接后端
//! let mut ui = Ui::begin(&window, &mut text, &mut state)
//!     .capture(&mouse, &keyboard)
//!     .theme(Theme::dark())
//!     .base_layer(1e7)
//!     .build();
//!
//! ui.label_at(glam::Vec2::new(500.0, 20.0), "FPS: 60");
//! ui.pack_at(glam::Vec2::new(24.0, 24.0), PackSide::Top, |p| {
//!     if p.button("btn_start", "开始游戏").clicked() {
//!         // 开始游戏……
//!     }
//!     let volume = p.slider("vol", 0.0..=1.0, 0.5);
//!     p.checkbox("fs", "全屏", false);
//!     let mut name = String::new();
//!     p.text_input("name", &mut name);
//! });
//! ui.grid_at(glam::Vec2::new(320.0, 24.0), 3, "inv", |g| {
//!     g.button("slot_0", "A");
//!     g.button("slot_1", "B");
//!     g.button("slot_2", "C");
//! });
//! ui.finish(&mut backend);
//! ```
//!
//! # 模块
//!
//! - [`ui`]：`Ui` 主体 / `Panel` / `Pack` / `Grid` / `UiAdd` 容器控件 API
//! - [`backend`]：绘制后端抽象（[`UiBackend`] / [`UiBatch`] / [`Tri`] /
//!   [`RecordingBackend`]）——`rjw_ui` 只输出批次数据，不调用任何渲染器
//! - [`id`]：ID 命名空间（[`IdRelative`] 原始名字 / [`IdAbsolute`] 完整状态键 / [`IdStack`]）
//! - [`layout`]：容器布局（Frame / PackSide）
//! - [`style`]：`Theme` 样式系统 + [`Palette`]（配色令牌）+ [`Brush`]（背景刷）
//! - [`state`]：`UiState` 持久状态 + `ButtonState` / `CheckboxState`
//! - [`hit`]：命中测试与交互状态机
//! - [`focus`]：键盘导航（焦点链 / [`focus_step`]）
//! - [`draw`]：屏幕固定变换与绘制命令 + [`Metric`]（物理/逻辑单位包装）
//! - [`edit`]：文本编辑纯逻辑（编辑状态机 / 词边界 / 省略号 / 剪贴板）
//! - [`view`]：**View 沙箱**（裁剪分层 / 可用宽度 / 命中过滤；`ViewMode`）
//! - [`widgets`]：`Widget` trait + 尺寸契约 + 属性化 builder（非宏添加控件）
//!
//! 内部模块（`pub(crate)`，不构成公开面）：`gpu_batch`（几何收集 + 切段裁决）、
//! `tess`（圆角 CPU 镶嵌：单位弧表 + 羽化带 / 环带）、`ui_types`（窗口与面板选项枚举）。

// 结构性复杂度豁免（**仅内部实现**，不在公开 API 面上）：
// `rjw_ui` 的实现是"布局 / 命中 / 绘制"三段式的坐标数学，参数表
// （`x, y, w, h, id, frame, depth, ...`）本就是这些原语的天然形状；为它们包一层
// `struct` 只会把参数搬运到构造点，降低调用点可读性。公开面（`ui.window(..)` 等
// 入口）不受影响：那部分的参数已按 R1（≤2 参数）+ 枚举状态糖收敛。
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

pub mod backend;
pub mod draw;
pub mod edit;
pub mod focus;
pub(crate) mod gpu_batch;
pub mod id;
pub mod hit;
pub mod input;
pub mod layout;
pub mod state;
pub mod style;
pub(crate) mod tess;
pub mod ui;
pub mod view;
pub mod widgets;
pub(crate) mod ui_types;

pub use backend::{RecordingBackend, Tri, UiBackend, UiBatch, UiBatchSource};
pub use draw::{CornerRadius, Gradient, Metric, Position, Size, TextAlign, lerp_color};
pub use focus::FocusKind;
pub use id::{IdAbsolute, IdRelative, IdStack};
pub use hit::{hit_test, InteractEvents};
pub use layout::{Child, PackSide};
pub use state::{ButtonState, CheckboxState, TextFocus, UiState, UiStats, WidgetState};
pub use style::{
    Brush, ButtonStyle, CheckboxStyle, ComboStyle, DividerStyle, FocusStyle, InputStyle,
    LabelStyle, ModalStyle, Palette, PanelStyle, SliderStyle, Theme, bevel_raised, bevel_sunken,
    hgrad, vgrad,
};
pub use input::{KeyboardSnapshot, MouseSnapshot};
pub use ui::{Anchor, Grid, Level, ModalBuilder, Pack, Panel, PanelBuilder, PanelOptions, Placement, Resize, Ui, UiAdd, UiCursor, UiDebugDump, UiInit, UiWindowInfo, Window, WindowBuilder, WindowClamp, WindowFx, WindowOptions};
pub use view::{ViewCtx, ViewMode};
pub use widgets::{
    Button, Checkbox, ColorPicker, Divider, FontModal, Label, NumberInput, Response, Slider,
    Widget, WidgetId, color_hex, parse_hex,
};

/// **UI 文本模块**（公开）：`rjw_ui` 里与文字渲染相关的全部公开面。
///
/// 供两类场景：
/// 1. **自定义控件 / 自定义容器**——需要自己测文字、摆文字、画裁剪文本、
///    取共享排版缓冲（避免每帧重复整形）；
/// 2. **自己实现 `UiBackend` 或另建 UI 层**——需要 `TextAlign` / `TextVAlign` /
///    `text_block_offset` 来复现同样的文本块对齐语义。
///
/// 这里的条目都是**重导出**（定义仍在 `draw` / `edit` / `ui` 模块），
/// 因此 `rjw_ui::draw::TextAlign` 与 `rjw_ui::text::TextAlign` 是同一个类型。
///
/// 文字**形状与字形图集**在 `rjw_text`：`Text::{label, measure_buffer, lines,
/// white_region, user_texture, glyph_cache, glyph_cache_mut}`。
pub mod text {
    // ── 对齐与文本块定位（定义在 `draw`）──
    pub use crate::draw::{
        TextAlign, TextVAlign, text_block_offset, text_cmd,
    };

    // ── 纯逻辑文本编辑 / 测量辅助（定义在 `edit`；无 UI 状态依赖，可独立单测）──
    pub use crate::edit::{
        byte_to_char, caret_at_visual_click, caret_index_by_width, char_to_byte, delete_range,
        ellipsize, extend_word_caret, index_of_line_col, insert_str_at, line_col_of,
        move_caret_line, scroll_follow_caret, sel_range, selected_text, vline_of_byte, word_range,
    };

    // ── 逐字形 / 逐行文本图元与测量（`rjw_text` 的形状层）──
    // 文字渐变（多段 stops 的能力在这里——矩形渐变 `crate::Gradient` 不支持多段）
    pub use rjw_text::Gradient as TextGradient;
    pub use rjw_text::{
        Align, GradientAxis, LineSpace, TextBuffer, TextStyle, VisualLine,
    };
}
