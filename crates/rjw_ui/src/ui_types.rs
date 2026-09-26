//! **UI 门面枚举与选项**（从 `ui.rs` 拆出）。
//!
//! 这些类型是 `Ui` 的**公开配置与枚举**：不持有 `Ui`、不参与录制，可独立构造 /
//! 复用 / 跨帧缓存。拆出后 `ui.rs` 只留「录制 + 布局 + 提交」的主体逻辑。
//!
//! | 类型 | 用途 |
//! |---|---|
//! | [`Anchor`] | 视口锚点（内容在客户区内的停靠位置） |
//! | [`WindowClamp`] | 窗口位置约束模式（取代裸布尔） |
//! | [`WindowFx`] | 整窗口特效（tint + 叠加变换 + 归一化锚点） |
//! | [`UiCursor`] | 系统光标意图（控件作者设置，`finish` 统一应用） |
//! | [`Level`] | 窗口层级（点击是否置顶） |
//! | [`Placement`] | 窗口内容排布（Expand / Clip） |
//! | [`Resize`] | 可调整尺寸控件的缩放方向（取代 `show_handle: bool`） |
//! | [`ScrollMode`] / [`ScrollParam`] | 单轴溢出策略（三态）与统一入参（`bool` / `ScrollMode`） |
//! | [`ScrollOutcome`] | 滚动容器结算结果（视口 + 内容尺寸） |
//! | [`WindowOptions`] / [`PanelOptions`] | 责任链 builder 的数据载体 |
//!
//! 注：诊断快照（`UiWindowInfo` / `UiDebugDump`）与 builder 本体留在 `ui.rs`——
//! 它们与 `Ui::debug_dump` / `Ui` 内部字段的形态强相关，拆开只会制造双向依赖。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Transform2D;

use crate::draw::{Position, Size};
use crate::style::PanelStyle;

/// 视口锚点（`Ui::anchor_pos`）：内容在视口（窗口客户区）内的停靠位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

/// **窗口位置约束模式**（`WindowBuilder::clamp` / `Ui::window` 责任链传入；
/// 默认 [`WindowClamp::Screen`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowClamp {
    /// **限制在画面内**（默认）：窗口**整体**不跑出屏幕——拖拽 / 脚本定位后的位置被
    /// clamp 到窗口客户区内；窗口比画面大时仍可拖动（左上角允许到 `负 = 屏幕-尺寸`，
    /// 窗口始终覆盖画面，不会钉死）。
    Screen,
    /// **完全自由**：窗口可拖出屏幕（不做限位）。
    Free,
    /// **锁定**：位置固定（用户拖拽无效；脚本 / 传入位置仍生效，点击置顶仍有效）。
    Locked,
}

/// **窗口整窗口特效**（`Ui::window_fx`）：整窗口混合色 + 叠加变换。
/// 每帧可改；窗口顶点缓存不变，仅提交时应用实例矩阵/颜色。
#[derive(Clone, Copy, Debug)]
pub struct WindowFx {
    /// 整窗口混合色（默认白 = 不染色；用于淡入淡出 / 整窗染色）。
    pub tint: Color,
    /// 叠加变换（基础屏幕固定 × 此变换；默认 `None`；用于位移/缩放/旋转动画）。
    pub transform: Option<Transform2D>,
    /// **归一化变换锚点**（`x, y ∈ [0.0, 1.0]`，相对窗口内容区；`(0.5, 0.5)` =
    /// 窗口中心）。变换绕该锚点进行（位移/旋转/缩放以锚点为基准）。
    /// **无论锚点何值，`transform = IDENTITY` 时位置恒为窗口原位置**。
    pub anchor: Vec2,
}

impl Default for WindowFx {
    fn default() -> Self {
        Self { tint: Color::WHITE, transform: None, anchor: Vec2::ZERO }
    }
}

/// 系统光标（控件作者经 `Ui::set_cursor` 设置；`finish` 统一应用，优先级低于
/// 内置拖拽抓握）。窗体悬停/拖动保持 [`UiCursor::Default`]（Arrow）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiCursor {
    /// 默认箭头（窗体悬停 / 拖动）。
    Default,
    /// 文本输入 I 型（内置：输入框悬停）。
    Text,
    /// 可拖拽悬停（张手；内置：滚动条 thumb）。
    Grab,
    /// 正在拖拽（抓握；内置：滚动条拖拽中）。
    Grabbing,
    /// 水平双向箭头（↔；滑块 / 拖拽调值手柄 / 窗口宽度缩放柄）。
    EwResize,
    /// 垂直双向箭头（↕；窗口高度缩放柄）。
    NsResize,
    /// 对角线双向箭头（↖↘；可调整大小的 TextArea 右下角缩放柄）。
    NwseResize,
}

impl UiCursor {
    pub(crate) fn to_winit(self) -> winit::window::CursorIcon {
        match self {
            UiCursor::Default => winit::window::CursorIcon::Default,
            UiCursor::Text => winit::window::CursorIcon::Text,
            UiCursor::Grab => winit::window::CursorIcon::Grab,
            UiCursor::Grabbing => winit::window::CursorIcon::Grabbing,
            UiCursor::EwResize => winit::window::CursorIcon::EwResize,
            UiCursor::NsResize => winit::window::CursorIcon::NsResize,
            UiCursor::NwseResize => winit::window::CursorIcon::NwseResize,
        }
    }
}

/// **窗口层级**（点击是否置顶；取代旧的 `.topmost(bool)` 裸布尔）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Level {
    /// **点击置顶**（默认）：点击窗口即提到最上层（焦点）。
    #[default]
    Topmost,
    /// **不置顶**：点击不改变 z-order（浮层 / 常驻 HUD 用）。
    Normal,
}

/// **单轴的内容溢出策略**（egui 风：`.vscroll(..)` / `.hscroll(..)` 各管一条轴）。
///
/// 一条轴上的语义：
/// - [`ScrollMode::NoClip`]（默认）：**不许裁切** —— 该轴的大小必须**装得下全部内容**
///   （内容撑大窗口；用户拖过该轴时，拖出的大小是**下限**而不是固定值）；
/// - [`ScrollMode::ClipOnly`]：该轴是**视口** —— 超出部分被裁掉，但没有滚动条；
/// - [`ScrollMode::Scroll`]：该轴是**视口 + 滚动条**（滚轮 / 拖 thumb 能翻到被裁掉的部分）。
///
/// ⚠ 与 [`Placement`] 的关系：`Placement::{Expand, Clip}` 是**老的一体化开关**
/// （两条轴一起），仍然保留；**显式的 `.vscroll(..)` / `.hscroll(..)` 覆盖它**。
/// 都没给时按老语义解算：不裁切，但**被用户拖过尺寸的那条轴自动成为视口**
/// （`ClipOnly`，这正是"拖过高度之后内容会被裁掉"的既有行为，见
/// [`WindowBuilder::resize`](crate::WindowBuilder::resize)）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScrollMode {
    /// **不裁切**：该轴装得下所有内容（默认）。
    #[default]
    NoClip,
    /// **只裁切**：视口语义，无滚动条。
    ClipOnly,
    /// **裁切 + 滚动条**（滚轮 / 拖 thumb / 点轨道）。
    ///
    /// 两条轴**各自独立**且都真的实现了：`vscroll(Scroll)` 与 `hscroll(Scroll)` 都会
    /// 建视口 + 画该轴的滚动条（见 [`ScrollState::offset`](crate::ScrollState::offset) /
    /// [`offset_x`](crate::ScrollState::offset_x)）。
    Scroll,
}

/// **单轴滚动开关的统一入参**（[`WindowBuilder::vscroll`](crate::WindowBuilder::vscroll) /
/// [`hscroll`](crate::WindowBuilder::hscroll) / [`ScrollArea`](crate::ScrollArea) 都用它）。
///
/// 两个实现：
/// - `bool`：`true` = [`ScrollMode::Scroll`]（视口 + 滚动条），`false` = [`ScrollMode::NoClip`]
///   （不裁 —— 内容自然尺寸撑开 / 按给定宽压缩）；
/// - [`ScrollMode`]：三态原样透传（低层逃生舱：只裁不滚的 [`ScrollMode::ClipOnly`]
///   只能这么写，或由 `Placement::Clip` / "该轴被用户拖过"解算出来）。
///
/// 例：`.vscroll(true)` ≡ `.vscroll(ScrollMode::Scroll)`、`.hscroll(false)` ≡
/// `.hscroll(ScrollMode::NoClip)`。
pub trait ScrollParam: Copy {
    /// 归一成三态 [`ScrollMode`]。
    fn scroll_mode(self) -> ScrollMode;
}

impl ScrollParam for ScrollMode {
    #[inline]
    fn scroll_mode(self) -> ScrollMode {
        self
    }
}

impl ScrollParam for bool {
    #[inline]
    fn scroll_mode(self) -> ScrollMode {
        if self { ScrollMode::Scroll } else { ScrollMode::NoClip }
    }
}

/// **滚动容器的结算结果**（[`Ui::scroll_axes_at`](crate::Ui::scroll_axes_at) /
/// [`ScrollArea::show`](crate::ScrollArea::show) 返回）。
///
/// - [`Self::view`]：**视口**尺寸（调用方传进来的可用区，物理像素）；
/// - [`Self::content`]：**内容**结算尺寸（物理像素；超出视口的部分靠滚动看）。
///
/// 为什么单列一个结构体：`Ui::scroll_at` 的既有签名只能回一个 `Vec2`（视口），而
/// "内容到底多宽多高"是布局决策的真正常用量（要不要给滚动条、视口该多大）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollOutcome {
    /// 视口尺寸（物理像素）。
    pub view: Vec2,
    /// 内容结算尺寸（物理像素；可大于视口）。
    pub content: Vec2,
}

/// **窗口内容排布方式**（取代旧的 `.strict()` 开关）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Placement {
    /// **Expand**（默认）：内容自然尺寸撑高窗口，**不裁剪**（子项自动换行）。
    #[default]
    Expand,
    /// **Clip**：内容**强制裁剪**到窗口矩形（Clip 沙箱：超出部分被裁、含 noclip
    /// 绘制；外层 ScrollView 的裁切一并生效）。命中仍由窗口遮挡机制隔离。
    ///
    /// = 两条轴都 [`ScrollMode::ClipOnly`]（新 API 的简写；显式 `.vscroll(..)` /
    /// `.hscroll(..)` 优先）。
    Clip,
}

/// **可调整尺寸控件的缩放方向**（取代旧的 `show_handle: bool` 裸布尔）。
///
/// 用于 [`Ui::resizable_text_input_at`](crate::Ui::resizable_text_input_at) /
/// [`Ui::resizable_text_area_at`](crate::Ui::resizable_text_area_at)，以及
/// [`WindowBuilder::resize`](crate::WindowBuilder::resize)（窗口右下角柄）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Resize {
    /// **不显示缩放柄**（尺寸只由尺寸责任链 / 持久值决定）。
    #[default]
    None,
    /// 显示右下角缩放柄，**只可调宽**（单行输入框 / 窗口宽度；`↔` 光标）。
    Horizontal,
    /// 显示右下角缩放柄，**只可调高**（窗口高度；`↕` 光标）。
    ///
    /// 窗口专用语义：宽度仍由内容决定（自动宽），拖出来的高度**跨帧持久**
    /// （`UiState::window_heights`）——与 [`Self::Both`] 的高度轴一致。
    Vertical,
    /// 显示右下角缩放柄，**宽高同调**（多行 TextArea / 窗口；`⤡` 光标）。
    Both,
}

/// **窗口选项**（`Ui::window` / `WindowBuilder` 的数据载体，可独立构造/复用）。
///
/// - `pos`：窗口左上角（[`Position`]：逻辑/物理，相对当前容器内容原点；顶层 = 屏幕原点）；
///   **`None`（默认）= 引擎自动分配**（Win32 `CW_USEDEFAULT` 语义，见下）；
/// - `width`：`Some` = 固定宽（[`Size<f32>`]：逻辑/物理，高度自动，右下角可鼠标缩放，
///   跨帧持久）；`None` = 自动宽（内容自然结算）；
/// - `level`：点击是否置顶（默认 [`Level::Topmost`]）；
/// - `placement`：内容排布（默认 [`Placement::Expand`]；[`Placement::Clip`] = 严格裁剪）；
/// - `style`：逐窗口样式覆盖（默认 `None` = 全局 `Theme::panel`）；
/// - `clamp`：位置约束模式（默认 [`WindowClamp::Screen`]：窗口整体不跑出屏幕）；
/// - `resize`：**能不能拖拽改大小**（只有布尔；轴由 `vscroll` / `.height(..)` 推导 ——
///   见 [`WindowBuilder::resize`](crate::WindowBuilder::resize)）；`None` = 旧行为
///   （**有 `.width(..)` 就能横向拖**）。
#[derive(Clone, Debug)]
pub struct WindowOptions {
    /// 窗口左上角；**`None`（默认）= 引擎自动分配**（级联 + 跨帧记忆，Win32
    /// `CW_USEDEFAULT` 语义；见 [`WindowBuilder::pos`](crate::WindowBuilder::pos)）。
    pub pos: Option<Position>,
    pub width: Option<Size<f32>>,
    /// **固定高**（[`WindowBuilder::height`](crate::WindowBuilder::height)；`None` = 由内容
    /// 决定）。窗口此前只有"固定宽"，长内容（列表 / 调色板）只能把窗口撑成屏幕那么高 ——
    /// 与 `.vscroll(Scroll)` 合用才有"有界视口 + 滚动条"。
    pub height: Option<Size<f32>>,
    pub level: Level,
    pub placement: Placement,
    pub style: Option<PanelStyle>,
    pub clamp: WindowClamp,
    /// **能不能拖拽改大小**（[`WindowBuilder::resize`](crate::WindowBuilder::resize) 的入参）：
    /// `Some(false)` = 不画柄也不响应拖拽（`.width(..)` 仍作布局固定宽，菜单 / 下拉浮层用）；
    /// `Some(true)` = 画柄 + 可拖，允许的**轴**按判定表推导（有视口 ⇒ 垂直 + 水平，否则只有
    /// 水平）；`None`（默认）= **旧语义**："有 `.width(..)` 就能横向拖"。
    pub resize: Option<bool>,
    /// **内容子项间距**（[`WindowBuilder::gap`](crate::WindowBuilder::gap)；`None` = 用
    /// [`Theme::gap`](crate::Theme::gap)）。
    ///
    /// 有了它，**下拉 / 菜单这类"内容行紧挨着"的浮层**才能拿到比主题更紧的行距
    /// （见 [`crate::widgets::menu::popup_gap`]）——否则每一行之间都空出一个 `Theme::gap`
    /// （用户实测：菜单项之间的空位太大）。
    pub gap: Option<Size<f32>>,
    /// **垂直轴**的内容溢出策略（`None` = 按 `placement` + "该轴是否被拖过"解算；
    /// 见 [`ScrollMode`]）。
    pub vscroll: Option<ScrollMode>,
    /// **水平轴**的内容溢出策略（同上）。
    pub hscroll: Option<ScrollMode>,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            // `None` = **引擎自动分配位置**（CW_USEDEFAULT 语义；见 `Ui::window` 文档）。
            pos: None,
            width: None,
            height: None,
            level: Level::Topmost,
            placement: Placement::Expand,
            style: None,
            clamp: WindowClamp::Screen,
            resize: None,
            gap: None,
            vscroll: None,
            hscroll: None,
        }
    }
}

/// **面板选项**（`Ui::panel` / `PanelBuilder` 的数据载体）。
#[derive(Clone, Debug)]
pub struct PanelOptions {
    pub pos: Position,
    pub drag: Option<String>,
    pub style: Option<PanelStyle>,
}

impl Default for PanelOptions {
    fn default() -> Self {
        Self { pos: Position::Logical(Vec2::ZERO), drag: None, style: None }
    }
}
