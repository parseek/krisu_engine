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
//! - **屏幕空间**：坐标一律为屏幕像素（左上角原点、Y+ 向下）；内部经相机
//!   屏幕固定变换绘制，命中测试直接在屏幕像素进行（旋转/缩放相机依然准确）。
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
//!   [`Ui::gradient_rect_at`]——程序化纹理（圆角 9-patch / 渐变 / WHITE）**塞进动态
//!   Atlas**（[`ProcTextures`] → `UiState` 持有），圆角纹理只存白色 + alpha（颜色顶点色
//!   tint），提交分组升级为 `(win, 图形/文字组, 纹理)` 保证"先图形后文字"。
//! - **滚动容器**：[`Ui::scroll_at`]——内容在可视区内堆叠 + 滚轮 / 滚动条
//!   （拖 thumb、点轨道翻页）滚动，可视区外**裁剪**（`UiDraw.clip` 绝对逻辑矩形，
//!   收集期与内容求交）；滚动偏移持久于 [`UiState::scrolls`]。
//! - **键盘导航**：**Tab / Shift+Tab / 方向键**遍历焦点链（[`UiState::focused`]），
//!   **Enter / Space** 激活焦点控件（按钮 / 勾选 / 单选 / 下拉框），滑块用左右方向键
//!   调值、下拉框展开时上下方向键切换选项，**Esc** 收起浮层 / 取消焦点；焦点控件
//!   画描边（`Theme::focus`，[`crate::style::FocusStyle`]）。
//! - **布局增强**：[`Ui::label_wrap_at`]（宽度内自动**换行**的标签，含容器内
//!   `p.label_wrap`）、**min/max 尺寸约束**（`p.min_size` / `p.max_size`，作用于下一
//!   子项）、**flex 权重**（[`Ui::flex_at`]：固定总高按权重等分子项，同帧精确分配）。
//! - **文本输入增强**：单行输入框**超长滚动跟随光标**、**拖选 + Ctrl+C/V/X 复制粘贴
//!   剪切**（[`crate::edit`] 纯逻辑）、**多行 TextArea**（[`Ui::text_area_at`]：Enter
//!   换行 / ↑↓ 跨行 / 自动换行 + 垂直滚动）、**IME 组合候选浮动提示框**（preedit 画在
//!   输入框下方浮动小框，不再占行内）。
//!
//! # 快速上手
//!
//! ```no_run
//! # let cam = todo!(); let mouse = todo!(); let keyboard = todo!();
//! # let text = todo!(); let r2d = todo!(); let state = todo!(); let window = todo!();
//! use rjw_ui::{PackSide, Theme, Ui};
//! let mut ui = Ui::begin(&window, &cam, &mouse, &keyboard, &mut text, &mut r2d, &mut state)
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
//! ui.finish();
//! ```
//!
//! # 模块
//!
//! - [`ui`]：`Ui` 主体 / `Panel` / `Pack` / `Grid`
//! - [`layout`]：容器布局（Frame / PackSide）
//! - [`style`]：`Theme` 样式系统
//! - [`state`]：`UiState` 持久状态 + `ButtonState` / `CheckboxState`
//! - [`hit`]：命中测试与交互状态机
//! - [`focus`]：键盘导航（焦点链 / [`focus_step`]）
//! - [`draw`]：屏幕固定变换与绘制命令

pub mod draw;
pub mod edit;
pub mod focus;
pub mod hit;
pub mod layout;
pub mod proc;
pub mod state;
pub mod style;
pub mod ui;

pub use draw::{GradientAxis, TextAlign};
pub use proc::ProcTextures;
pub use hit::{hit_test, InteractEvents};
pub use layout::PackSide;
pub use state::{ButtonState, CheckboxState, UiState, WidgetState};
pub use style::{ButtonStyle, CheckboxStyle, InputStyle, LabelStyle, PanelStyle, SliderStyle, Theme};
pub use ui::{Grid, Pack, Panel, Ui, UiInit, Window};
