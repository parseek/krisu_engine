//! `Ui` 主体：控件录制 + 深度排序 + 提交绘制。
//!
//! 用法（见 crate 文档与示例）：
//! ```no_run
//! # let viewport = todo!(); let mouse = todo!(); let keyboard = todo!();
//! # let text = todo!(); let r2d = todo!(); let state = todo!(); let window = todo!();
//! use rjw_ui::{Theme, Ui};
//! let mut ui = Ui::begin(&window, &mut text, &mut state)
//!     .capture(&mouse, &keyboard)
//!     .theme(Theme::dark())
//!     .base_layer(1e7)
//!     .build();
//! ui.label_at(glam::Vec2::new(20.0, 20.0), "Hello UI");
//! ui.finish(&viewport, &mut r2d);
//! ```
//!
//! 坐标语义：所有位置为**屏幕逻辑像素**（左上角原点，Y+ 向下）；容器内 `*_at` 的 `pos`
//! 相对**当前容器内容原点**（顶层即屏幕原点）；交互命中在逻辑坐标进行（内部经 DPI 换算）。

use std::ops::RangeInclusive;
use std::sync::Arc;
use std::time::Instant;

use glam::Vec2;
use rjw_2d_render::{Layer, Render2D, VertexP3U2C4};
use rjw_color::Color;
use rjw_keyboard::{KeyCode, KeyboardInput};
use rjw_keystate::KeyState;
use rjw_mouse::{MouseButton, MouseInput};
use rjw_render::TEXTURES;
use rjw_text::{Align, Attrs, Buffer, CachePolicy, Family, Text, VisualLine};
use rjw_transform::{Rect, Viewport};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::window::Window as WinitWindow;

use crate::draw::{
    border_rects, clipped, debug_shape_segments, intersect_rect, screen_fixed_tf, snap_rect,
    text_block_offset, text_cmd, DebugShape, DrawKind, GradientAxis, TextAlign, TextVAlign, UiDraw,
};
use crate::edit::{
    byte_to_char, caret_at_visual_click, caret_index_by_width, char_to_byte, insert_char_at,
    scroll_follow_caret, sel_range, vline_of_byte,
};
use crate::focus::{focus_step, FocusEntry, FocusKind};
use crate::hit::{
    clear_frame_flags, hit_test, normalize_x, update_drag, update_interact, window_occluded,
};
use crate::input::{KeyboardSnapshot, MouseSnapshot};
use crate::layout::{Frame, PackSide};
use crate::state::{ButtonState, CheckboxState, TEXT_BUFFER_CACHE_CAP, UiState, UiStats, WidgetState};
use crate::style::{ButtonStyle, CheckboxStyle, Theme};
use crate::view::{clip_for_view, ViewCtx, ViewMode};
use crate::widget::Widget as _;

// ─── 文本编辑辅助（纯函数，可单测） ─────────────────────────────

/// TextArea **行距倍率**：行高 = 字号 × 该值（1.2 = 略宽松，多行可读性；与
/// `ensure_text_buf` 的 `line_mult` 一致，cosmic 行盒按此递增）。
pub(crate) const TEXT_AREA_LINE_SPACING: f32 = 1.2;

/// 滚动条宽度（**逻辑像素**；`scroll_at` 与文本编辑框共用）。文本编辑框的滚动条
/// 条带排除（按下滚动条不建立文本选择）也用它。
pub(crate) const SCROLLBAR_W: f32 = 8.0;

/// **双击判定帧数**：两次按下间隔 ≤ 该帧数（≈330ms @60fps，与光标闪烁同用
/// `UiState.frame` 时钟）且位移 < [`DOUBLE_CLICK_DIST`] 视为双击。
pub(crate) const DOUBLE_CLICK_FRAMES: u64 = 20;

/// **双击判定位移阈值**（逻辑像素）。
pub(crate) const DOUBLE_CLICK_DIST: f32 = 4.0;

/// **勾选框中心填充内边距**（物理像素）：中心蓝色矩形 = 外框 shrink
/// `floor(border_w·scale) + floor(CHECKBOX_INNER·scale)`（减法内缩，非写死偏移）。
pub(crate) const CHECKBOX_INNER: f32 = 1.0;

/// **置顶哨兵 z 值**：IME 组合候选提示框、下拉浮层等**顶层浮层**用它——
/// 绘制（win 升序排序恒最后）与命中（`window_occluded` 无更高 z）都恒在一切窗口之上。
/// 真实窗口的 z 分配 / 置顶运算必须**排除本值**（`filter(|&z| z < WIN_TOPMOST)`、
/// `saturating_add`），避免普通窗口递增碰撞到哨兵。
pub(crate) const WIN_TOPMOST: u32 = u32::MAX;

// 纯文本编辑函数（字符插入/删除/剪贴板/编辑状态机）已迁入 [`crate::edit`]：
// `insert_char_at` / `remove_before` / `remove_at` / `clipboard_shortcuts` /
// `apply_frame_edits` / `caret_horiz`。

// ─── 文本排版版本号 ─────────────────────────────────────────────

/// 文本缓冲区行高版本号。当行高计算方式变更时递增，使旧缓存失效。
/// 版本 1：行高 = 字号（原为 1.2 倍字号，导致英文字母在文本框内偏上）。
const TEXT_LINE_HEIGHT_VERSION: u8 = 1;

// ─── Ui ─────────────────────────────────────────────────────────

/// 视口锚点（[`Ui::anchor_pos`]）：内容在视口（窗口客户区）内的停靠位置。
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

/// 系统光标（控件作者经 [`Ui::set_cursor`] 设置；`finish` 统一应用，优先级低于
/// 内置拖拽抓握）。窗体悬停/拖动保持 [`UiCursor::Default`]（Arrow）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiCursor {
    /// 默认箭头（窗体悬停 / 拖动）。
    Default,
    /// 文本输入 I 型（内置：输入框悬停）。
    Text,
    /// 可拖拽悬停（张手；内置：滑块 / 滚动条 thumb）。
    Grab,
    /// 正在拖拽（抓握；内置：滑块 / 滚动条拖拽中）。
    Grabbing,
    /// 水平双向箭头（↔；拖拽调值手柄 / 窗口宽度缩放柄）。
    EwResize,
}

impl UiCursor {
    pub(crate) fn to_winit(self) -> winit::window::CursorIcon {
        match self {
            UiCursor::Default => winit::window::CursorIcon::Default,
            UiCursor::Text => winit::window::CursorIcon::Text,
            UiCursor::Grab => winit::window::CursorIcon::Grab,
            UiCursor::Grabbing => winit::window::CursorIcon::Grabbing,
            UiCursor::EwResize => winit::window::CursorIcon::EwResize,
        }
    }
}

/// `Ui::begin` 返回的构建器：捕获输入快照 / 设置主题 / 基层层级 / scale_factor /
/// 调试开关后 `build()`。
///
/// **输入与绘制解耦**：`Ui` 不借用键盘 / 鼠标设备（[`Self::capture`] 拷贝快照），
/// 相机与渲染器**延迟到 [`Ui::finish`] 传入**——`begin` 只依赖 IME 窗口 / 文本 /
/// 持久状态。
pub struct UiInit<'a> {
    window: &'a WinitWindow,
    text: &'a mut Text,
    state: &'a mut UiState,
    mouse: MouseSnapshot,
    keyboard: KeyboardSnapshot,
    theme: Theme,
    base_layer: f64,
    scale: f32,
    debug_layout: bool,
}

impl<'a> UiInit<'a> {
    /// **捕获输入快照**（帧开始时调用一次）：把键盘 / 鼠标设备的完整状态
    /// （按键边沿 / IME / 鼠标位置与滚轮）拷贝为 `Ui` 自持数据——之后 `Ui` 与设备
    /// 解耦，可独立存在；**不调用 = 空输入**（headless：纯布局 / 纯绘制，无交互）。
    pub fn capture(mut self, mouse: &MouseInput, keyboard: &KeyboardInput) -> Self {
        self.mouse = MouseSnapshot::capture(mouse);
        self.keyboard = KeyboardSnapshot::capture(keyboard);
        self
    }

    /// 主题（默认浅色）。
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// 基层层级（默认 `1e7`，与 RPG 示例 UI 层一致）。
    pub fn base_layer(mut self, layer: f64) -> Self {
        self.base_layer = layer;
        self
    }

    /// DPI scale factor（物理像素 / 逻辑像素，如 1.0 / 1.5 / 2.0；默认 1.0）。
    ///
    /// 传入后，**全部控件坐标 / 字号按逻辑像素**使用，内部自动换算物理像素
    /// 绘制与命中；窗口高 DPI 下文字与控件同步缩放。
    /// 取值：`ctx.scale_factor().unwrap_or(1.0)`（`rjw_main::MainContext`）。
    pub fn scale_factor(mut self, scale: f64) -> Self {
        self.scale = scale.max(f64::EPSILON) as f32;
        self
    }

    /// **调试 UI 布局**（默认 **false**）：开启后 `finish` 为**每一个录制命令的矩形**
    /// 画一圈青色描边（覆盖在 UI 内容之上）——可视化每个控件/容器的布局矩形与
    /// 命中区域，用于调试 `rjw_ui` 自身的布局。可在帧内用 [`Ui::debug_layout`] 开关。
    pub fn debug_layout(mut self, on: bool) -> Self {
        self.debug_layout = on;
        self
    }

    pub fn build(self) -> Ui<'a> {
        let UiInit {
            window,
            text,
            state,
            mouse,
            keyboard,
            theme,
            base_layer,
            scale,
            debug_layout,
        } = self;
        state.begin_frame();
        let (mx, my) = mouse.get_mouse_position();
        let mouse_screen = Vec2::new(mx as f32, my as f32);
        let mouse_in_window = mouse.in_window();
        Ui {
            window,
            text,
            state,
            mouse,
            keyboard,
            theme,
            base_layer,
            scale,
            debug_layout,
            frames: Vec::new(),
            queue: Vec::new(),
            debug_queue: Vec::new(),
            clip: None,
            avail_stack: Vec::new(),
            abs_base: Vec2::ZERO,
            depth: 0,
            seq: 0,
            cur_win: 0,
            // 鼠标屏幕坐标：物理（拖拽/IME 基准用）与逻辑（命中测试用）各存一份
            mouse_screen,
            mouse_logical: mouse_screen / scale,
            mouse_in_window,
            any_pressed: false,
            press_claimed: false,
            drag_panel: None,
            win_press_top: None,
            win_origins: std::collections::HashMap::new(),
            win_ids: std::collections::HashMap::new(),
            focusables: Vec::new(),
            // UI 帧起点（finish 计算 ui_frame_us = begin → finish 结束的整帧耗时）
            frame_t0: Instant::now(),
            // 位置责任链：预置内置"用户拖拽状态"环（优先级 0）
            pos_chain: vec![(0, PosLink::Drag)],
            cursor_text: false,
            cursor_grab: false,
            cursor_grabbing: false,
            cursor_window_drag: false,
            cursor_custom: None,
        }
    }
}

/// 窗口/面板**位置责任链**一环：
/// - [`PosLink::Script`]：应用注册的脚本/动画/布局处理器（见 [`Ui::pos_handler`]；
///   **`'static` 闭包**——可捕获拥有值 / `Copy` 值 / `Arc`，需要共享可变状态时用
///   `Arc<Mutex<_>>`；约束闭包不借用 `self`，避免拖长 `Ui` 的借用导致
///   `ui.finish()` 后无法再访问应用状态）；
/// - [`PosLink::Drag`]：内置"用户拖拽状态"（[`UiState::panel_pos`]，固定优先级 `0`）。
enum PosLink {
    Script(Box<dyn Fn(&str) -> Option<Vec2> + 'static>),
    Drag,
}

/// 按**优先级降序**解析窗口/面板位置：第一个返回 `Some` 的环生效；
/// 全部落空（含用户未拖过）则回退 `pos`（调用者传入的初始位置）。
fn resolve_pos_link(
    chain: &[(i32, PosLink)],
    panel_pos: &std::collections::HashMap<String, Vec2>,
    id: &str,
    pos: Vec2,
) -> Vec2 {
    for (_, link) in chain {
        match link {
            PosLink::Script(f) => {
                if let Some(p) = f(id) {
                    return p;
                }
            }
            PosLink::Drag => {
                if let Some(p) = panel_pos.get(id) {
                    return *p;
                }
            }
        }
    }
    pos
}

/// UI 录制器（借用窗口 / 文本 / 状态；**输入为自持快照**，相机 / 渲染器延迟到
/// [`Ui::finish`] 传入——一帧一用）。
pub struct Ui<'a> {
    window: &'a WinitWindow,
    /// 键盘快照（[`UiInit::capture`] 拷贝，与设备解耦）。
    mouse: MouseSnapshot,
    /// 鼠标快照（同上）。
    keyboard: KeyboardSnapshot,
    text: &'a mut Text,
    state: &'a mut UiState,
    /// 主题样式（**公开**：widget 层 / 跨 crate 控件合并全局样式与逐控件覆盖用；
    /// 帧内可改，影响后续录制）。
    pub theme: Theme,
    base_layer: f64,
    /// DPI scale factor（物理 / 逻辑）。
    scale: f32,
    /// 容器帧栈（当前容器在栈顶）。
    frames: Vec<Frame>,
    /// 录制命令（坐标 = 相对当前容器 origin 的局部坐标，**逻辑像素**）。
    queue: Vec<UiDraw>,
    /// **调试命令队列**（[`Self::debug_line`] 等；坐标 = **绝对逻辑屏幕像素**）。
    /// 不进窗口缓存、不参与内容排序——`finish` 时在 UI 内容**之后**提交（恒覆盖在最上）。
    debug_queue: Vec<UiDraw>,
    /// **调试 UI 布局开关**（[`UiInit::debug_layout`] / [`Self::debug_layout`]）：
    /// 开启后每个录制命令的矩形都会画青色描边（布局 / 命中区域可视化）。
    debug_layout: bool,
    /// **当前裁剪区**（**绝对逻辑屏幕坐标**；滚动容器 [`Self::scroll_at`] 等设置）。
    /// 录制命令时存入 `UiDraw.clip`，收集期与内容求交（越界剔除）。
    /// 语义 = **强制裁剪层**（ScrollView 可视区 / Clip 沙箱）：所有绘制命令
    /// （含 `push_*_noclip` 变体）都服从；普通容器（Expand）不产生强制层。
    clip: Option<Rect>,
    /// **可用宽度栈**（逻辑像素）：`view_at` 沙箱进入时压入沙箱宽，弹出恢复。
    /// [`Self::avail_w`] 的唯一沙箱来源（容器固定宽经 `Frame::fixed_avail_w` 兜底）。
    avail_stack: Vec<Option<f32>>,
    /// 当前容器绝对原点（命中测试用，逻辑像素）。
    abs_base: Vec2,
    /// 当前录制深度（容器嵌套层数）。
    depth: u32,
    /// 全局递增序号（同深度内排序）。
    seq: u32,
    /// 当前窗口 z 序（[`Self::window_at`]；非窗口内容 = 0）。
    cur_win: u32,
    /// 鼠标屏幕坐标（逻辑像素 = 物理 ÷ scale，命中测试用）。
    mouse_logical: Vec2,
    /// 鼠标屏幕坐标（**物理像素**，面板拖拽 / IME 基准用）。
    mouse_screen: Vec2,
    mouse_in_window: bool,
    /// 本帧是否有控件被按下（空白点击清焦点用）。
    any_pressed: bool,
    /// **本帧按下是否被文本输入控件占用**（选择拖拽优先于窗口/面板拖拽）：
    /// 输入框/TextArea 在按下响应时置位，`window_at` / `panel_impl` 据此**不建立**
    /// 拖拽基准——从输入框上拖拽 = 选择文本，而不是拖动窗口。
    press_claimed: bool,
    /// 当前拖拽中的面板 / 窗口 ID（拖动期间抑制子控件交互）。
    drag_panel: Option<String>,
    /// 本帧按下命中的**最上层窗口**（重叠点击裁决：只让最高 z 窗口拖拽与置顶）。
    win_press_top: Option<(String, u32)>,
    /// 窗口 z → 窗口左上角（**逻辑**坐标）：QuadVertices 顶点相对此原点存储，
    /// 提交时用 `screen_fixed_tf(原点物理)` 变换到世界（移动窗口只改变换，顶点不变）。
    win_origins: std::collections::HashMap<u32, Vec2>,
    /// 窗口 z → 窗口 id（四边形缓存 key 用；window_at 记录）。
    win_ids: std::collections::HashMap<u32, String>,
    /// **本帧焦点链**（键盘导航）：交互控件录制时注册（[`Self::register_focus`]），
    /// `finish` 按 (win, 注册序) 排序后处理 Tab / 方向键遍历并绘制焦点描边。
    focusables: Vec<FocusEntry>,
    /// UI 帧起点（`UiInit::build` 记录；`finish` 据此计算 `ui_frame_us`）。
    frame_t0: Instant,
    /// **窗口/面板位置责任链**：应用脚本处理器（优先级降序）+ 内置拖拽状态
    /// （优先级 0），见 [`Self::pos_handler`]；一帧一建，随 Ui 释放。
    pos_chain: Vec<(i32, PosLink)>,
    /// **本帧鼠标是否悬停在文本输入框上**（`finish` 据此把系统光标设为 I 型）。
    cursor_text: bool,
    /// 悬停可拖拽对象（滑块/滚动条 thumb）→ 手型光标（`finish` 应用）。
    cursor_grab: bool,
    /// 正在拖拽（滑块/滚动条）→ 抓握光标（`finish` 应用）。
    cursor_grabbing: bool,
    /// 窗口/面板**正在被拖拽** → 强制普通 Arrow（UI_NEEDS：窗体拖动无需 <->）。
    cursor_window_drag: bool,
    /// 控件作者经 [`Self::set_cursor`] 设置的自定义光标（如数字输入拖动手柄的 ↔）。
    cursor_custom: Option<winit::window::CursorIcon>,
}

impl<'a> Ui<'a> {
    /// 一帧一次。`window` 用于 IME 候选框定位（[`winit::window::Window::set_ime_cursor_area`]）
    /// 与光标图标。**不接收输入设备与绘制资源**：
    /// - 输入：`UiInit::capture(&mouse, &keyboard)` 快照（自持，可省略）；
    /// - 绘制（相机 / 渲染器）：延迟到 [`Self::finish`] 传入。
    pub fn begin(
        window: &'a WinitWindow,
        text: &'a mut Text,
        state: &'a mut UiState,
    ) -> UiInit<'a> {
        UiInit {
            window,
            text,
            state,
            mouse: MouseSnapshot::default(),
            keyboard: KeyboardSnapshot::default(),
            theme: Theme::default(),
            base_layer: 1e7,
            scale: 1.0,
            debug_layout: false,
        }
    }

    // ── 内部工具 ─────────────────────────────────────────────

    #[inline]
    fn next_seq(&mut self) -> u32 {
        self.seq += 1;
        self.seq
    }

    // ── 控件作者公开 API（跨 crate 自定义控件用） ─────────────

    /// 跨帧 UI 状态（只读；交互控件状态持久于此）。
    #[inline]
    pub fn state(&self) -> &UiState {
        self.state
    }

    /// 跨帧 UI 状态（可变；控件作者用 [`UiState::widget`] 读写指定 ID 的状态）。
    #[inline]
    pub fn state_mut(&mut self) -> &mut UiState {
        self.state
    }

    /// 鼠标**逻辑**屏幕坐标（命中测试 / 拖拽基准用；= 物理坐标 ÷ scale）。
    #[inline]
    pub fn mouse_logical(&self) -> Vec2 {
        self.mouse_logical
    }

    /// 鼠标**物理**屏幕坐标（warp 边缘判定 / 物理像素增量拖拽用）。
    #[inline]
    pub fn mouse_screen(&self) -> Vec2 {
        self.mouse_screen
    }

    /// 按键本帧按下边沿（控件作者键盘交互用，如 Esc 关闭模态对话框）。
    #[inline]
    pub fn key_down_edge(&self, key: winit::keyboard::KeyCode) -> bool {
        self.keyboard.get(key).down_edge()
    }

    /// 当前容器光标位置（局部坐标；自写"占光标"式组合布局时用于放置子容器）。
    #[inline]
    pub fn cursor_pos(&self) -> Vec2 {
        self.frames.last().map(|f| f.cursor).unwrap_or(Vec2::ZERO)
    }

    /// **声明本次按下归本控件**：自定义交互控件（自身有**拖拽语义**，如数字输入的
    /// 拖动手柄）在 `down_edge && hit` 时调用——阻止外层窗口/面板把本次按下当作
    /// **窗口拖拽基准**（否则窗口内拖滑块/手柄会连窗口一起动）。内置滑块 / 滚动条
    /// / 文本框已自行调用。
    #[inline]
    pub fn claim_press(&mut self) {
        self.press_claimed = true;
    }

    /// **通用拖拽缩放柄**（控件作者原语）：`handle` 为**当前容器局部坐标**的柄矩形
    /// （通常右下角）。按住拖拽把 `current` 改为新尺寸（返回 `Some(new)`；`None` =
    /// 本帧无变化）；范围 clamp 到 `min`。拖动中置位 `press_claimed`（阻止外层
    /// 窗口/面板把本次按下当拖拽基准），悬停/拖拽显示 `cursor`（如 ↔ / ↖↘）。
    ///
    /// 持久尺寸由调用方写入（推荐 [`UiState::sizes`]）；可缩放 widget 的 `size()`
    /// 优先读持久值，配合 [`crate::widget::Widget::resizable`] 声明。
    /// `window_at_w` / [`Self::window_at_strict_w`] 的宽度缩放即基于本原语。
    pub fn resize_handle(
        &mut self,
        id: &str,
        handle: Rect,
        current: Vec2,
        min: Vec2,
        cursor: crate::UiCursor,
    ) -> Option<Vec2> {
        let hhit = self.hit_abs(&handle);
        let hbtn = self.mouse_left();
        if hbtn.down_edge() && hhit {
            // 缩放柄自身有拖拽语义：阻止外层窗口/面板把本次按下当作拖拽基准。
            self.press_claimed = true;
        }
        let mut new = current;
        let active = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let a = update_drag(ws, hhit, hbtn);
            if hbtn.down_edge() && hhit {
                ws.press_mouse = Some(self.mouse_screen.round());
                ws.press_panel = Some(current);
            }
            if a {
                let pm = ws.press_mouse.unwrap_or(self.mouse_screen);
                let base = ws.press_panel.unwrap_or(current);
                let d = (self.mouse_screen - pm).round() / self.scale;
                new = Vec2::new((base.x + d.x).max(min.x), (base.y + d.y).max(min.y));
            }
            a
        };
        if active || hhit {
            self.set_cursor(cursor);
        }
        active.then_some(new)
    }

    /// **设置本帧系统光标**（控件作者用）：如数字输入拖动手柄悬停/拖拽时
    /// [`UiCursor::EwResize`]（↔），点击文本框时由内置逻辑显示 I 型。优先级低于
    /// 内置拖拽（滑块/滚动条抓握）、高于 I 型文本光标；窗体悬停/拖动保持默认箭头。
    #[inline]
    pub fn set_cursor(&mut self, icon: UiCursor) {
        self.cursor_custom = Some(icon.to_winit());
    }

    /// 窗口客户区**物理尺寸**（`(w, h)` 像素；拖拽调值的 warp 边缘判定用）。
    #[inline]
    pub fn window_physical_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width, s.height)
    }

    /// 视口**逻辑**尺寸（窗口客户区物理 ÷ scale；锚定布局 / 全屏遮罩用）。
    #[inline]
    pub fn viewport_size(&self) -> Vec2 {
        let s = self.window.inner_size();
        Vec2::new(s.width as f32, s.height as f32) / self.scale
    }

    /// 按锚点计算**绝对逻辑 pos**（顶层容器用）：内容尺寸 `size` 在视口内按
    /// `anchor` 停靠、距视口边 `margin`（逻辑像素；内容超视口时 clamp 到视口内
    /// 不溢出）。纯几何（[`Self::anchor_pos_in`]，可单测）。
    #[inline]
    pub fn anchor_pos(&self, a: Anchor, size: Vec2, margin: Vec2) -> Vec2 {
        Self::anchor_pos_in(self.viewport_size(), a, size, margin)
    }

    /// 锚定位置纯计算：`vp` 视口内按 `anchor` 停靠（可单测）。
    pub fn anchor_pos_in(vp: Vec2, a: Anchor, size: Vec2, margin: Vec2) -> Vec2 {
        let m = margin.max(Vec2::ZERO);
        let sx = (vp.x - m.x * 2.0).max(0.0);
        let sy = (vp.y - m.y * 2.0).max(0.0);
        let x = match a {
            Anchor::TopLeft | Anchor::CenterLeft | Anchor::BottomLeft => m.x,
            Anchor::TopCenter | Anchor::Center | Anchor::BottomCenter => m.x + (sx - size.x) * 0.5,
            Anchor::TopRight | Anchor::CenterRight | Anchor::BottomRight => {
                (vp.x - m.x - size.x).max(m.x)
            }
        };
        let y = match a {
            Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => m.y,
            Anchor::CenterLeft | Anchor::Center | Anchor::CenterRight => m.y + (sy - size.y) * 0.5,
            Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => {
                (vp.y - m.y - size.y).max(m.y)
            }
        };
        Vec2::new(x, y)
    }

    /// 设置鼠标光标**物理屏幕位置**（warp 用：拖到窗口边缘跳到对侧继续拖；
    /// 下一帧输入快照生效，配合拖拽基准偏移保持增量连续）。
    #[inline]
    pub fn set_cursor_position(&mut self, x: f32, y: f32) {
        let _ = self
            .window
            .set_cursor_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    }

    #[inline]
    fn phys_rect(&self, r: &Rect) -> Rect {
        Rect::new(r.x * self.scale, r.y * self.scale, r.w * self.scale, r.h * self.scale)
    }

    #[inline]
    fn phys_f(&self, f: f32) -> f32 {
        f * self.scale
    }

    // ── 键盘导航（焦点链） ────────────────────────────────────

    /// 把控件登记进本帧焦点链（键盘导航用）。`rect` 为**相对当前容器**的局部矩形，
    /// 内部转成**绝对逻辑坐标**（焦点描边绘制 / 排序用）。**交互控件必须调用**。
    pub fn register_focus(&mut self, id: &str, rect: Rect, kind: FocusKind) {
        let abs = Rect::new(
            self.abs_base.x + rect.x,
            self.abs_base.y + rect.y,
            rect.w,
            rect.h,
        );
        self.focusables.push(FocusEntry {
            id: id.to_owned(),
            win: self.cur_win,
            kind,
            depth: self.depth,
            rect: abs,
            clip: self.clip,
        });
    }

    /// 本控件是否持有键盘焦点（`UiState.focused == id`）。
    #[inline]
    fn focused_is(&self, id: &str) -> bool {
        self.state.focused.as_deref() == Some(id)
    }

    /// **键盘激活**：Enter / Space 在本帧按下、本控件持有焦点且不在 IME 组合中
    /// → 视为一次点击（按钮 / 勾选 / 单选 / 下拉框用）。文本输入框与滑块不参与
    /// （前者走打字路径，后者用方向键调值）。
    pub fn key_click(&self, id: &str, kind: FocusKind) -> bool {
        if !self.focused_is(id) || kind == FocusKind::TextInput || kind == FocusKind::Slider {
            return false;
        }
        let composing = self
            .keyboard
            .get_ime_preedit()
            .is_some_and(|p| !p.is_empty());
        if composing {
            return false;
        }
        self.keyboard.get(KeyCode::Enter).down_edge()
            || self.keyboard.get(KeyCode::Space).down_edge()
    }

    // ── Debug UI / DebugDraw（调试 rjw_ui 自身 + 屏幕空间调试图元） ──

    /// 调试 UI 布局开关（运行时切换；等价于 [`UiInit::debug_layout`]）。
    ///
    /// 开启后 `finish` 为**每一个录制命令的矩形**画青色描边（覆盖在 UI 内容之上）——
    /// 可视化每个控件 / 容器的布局矩形与命中区域。
    #[inline]
    pub fn debug_layout(&mut self, on: bool) -> &mut Self {
        self.debug_layout = on;
        self
    }

    /// 屏幕空间线段（**绝对逻辑屏幕像素**，Y+ 向下；覆盖在 UI 内容之上）。
    pub fn debug_line(&mut self, a: Vec2, b: Vec2, width: f32, color: Color) {
        self.push_debug(DebugShape::Line { a, b, width }, color);
    }

    /// 屏幕空间矩形边框（逻辑像素）。
    pub fn debug_rect_outline(&mut self, rect: Rect, width: f32, color: Color) {
        self.push_debug(DebugShape::RectOutline { rect, width }, color);
    }

    /// 屏幕空间圆环（`segments` 段折线近似；逻辑像素）。
    pub fn debug_circle_outline(
        &mut self,
        center: Vec2,
        radius: f32,
        segments: usize,
        width: f32,
        color: Color,
    ) {
        self.push_debug(DebugShape::CircleOutline { center, radius, segments, width }, color);
    }

    /// 屏幕空间十字标记（逻辑像素）。
    pub fn debug_cross(&mut self, center: Vec2, half: f32, width: f32, color: Color) {
        self.push_debug(DebugShape::Cross { center, half, width }, color);
    }

    /// 屏幕空间网格（`rect` 范围内按 `spacing` 竖线 + 横线；每方向最多 512 条）。
    pub fn debug_grid(&mut self, rect: Rect, spacing: f32, width: f32, color: Color) {
        self.push_debug(DebugShape::Grid { rect, spacing, width }, color);
    }

    /// 录制一条屏幕空间调试图元（进 `debug_queue`，坐标 = 绝对逻辑像素）。
    fn push_debug(&mut self, shape: DebugShape, color: Color) {
        let seq = self.next_seq();
        let depth = self.depth;
        let win = self.cur_win;
        self.debug_queue.push(UiDraw {
            depth,
            seq,
            win,
            elem: 0,
            // 调试形状自带几何（DebugShape），rect 字段未用。
            rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            clip: None,
            kind: DrawKind::Debug { color, shape },
        });
    }

    /// **圆角矩形**（背景填充原语；绝对定位，`radius` 逻辑像素，9-patch 绘制——
    /// 程序化纹理进动态 Atlas，颜色顶点色 tint）。
    pub fn rounded_rect_at(&mut self, pos: Vec2, size: Vec2, radius: f32, color: Color) {
        self.push_draw(
            DrawKind::RoundedRect { color, radius },
            Rect::new(pos.x, pos.y, size.x, size.y),
        );
    }

    /// **线性渐变矩形**（背景填充原语；绝对定位，`stops` 沿 `axis`——
    /// 程序化渐变纹理进动态 Atlas，按主轴拉伸采样）。
    pub fn gradient_rect_at(
        &mut self,
        pos: Vec2,
        size: Vec2,
        axis: GradientAxis,
        stops: Vec<(f32, Color)>,
    ) {
        self.push_draw(
            DrawKind::Gradient { axis, stops },
            Rect::new(pos.x, pos.y, size.x, size.y),
        );
    }

    /// 录制一条绘制命令（`elem = 0` 装饰层，画在本窗口元素之下——如背景/边框）。
    fn push_draw(&mut self, kind: DrawKind, rect: Rect) {
        let seq = self.next_seq();
        let depth = self.depth;
        let win = self.cur_win;
        self.queue.push(UiDraw { depth, seq, win, elem: 0, rect, clip: self.clip, kind });
    }

    /// 按样式 push **背景 + 边框**（`radius > 0` 走双层圆角矩形：外圈 border 色、
    /// 内圈 bg 色内缩 `border_w`，近似圆角边框；否则原 Solid + Border 路径）。
    /// `elem`：元素序（装饰背景传 0；控件背景传 `self.seq + 1`）。
    /// **控件作者绘制原语**（逻辑坐标，内部 ×scale 取整到物理像素）。
    #[allow(clippy::too_many_arguments)]
    pub fn push_panel_like(
        &mut self,
        rect: Rect,
        bg: Color,
        border: Color,
        border_w: f32,
        radius: f32,
        elem: u32,
    ) {
        let seq = self.next_seq();
        let depth = self.depth;
        let win = self.cur_win;
        let clip = self.clip;
        if radius > 0.0 {
            // 圆角边框 ≈ 外圈 border 色圆角 + 内圈 bg 色圆角（内缩 border_w）。
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect,
                clip,
                kind: DrawKind::RoundedRect { color: border, radius: radius + border_w },
            });
            let bw = border_w.min(rect.w * 0.5).min(rect.h * 0.5);
            let inner = Rect::new(
                rect.x + bw,
                rect.y + bw,
                (rect.w - bw * 2.0).max(0.0),
                (rect.h - bw * 2.0).max(0.0),
            );
            if inner.w > 0.0 && inner.h > 0.0 {
                self.queue.push(UiDraw {
                    depth,
                    seq: seq + 1,
                    win,
                    elem,
                    rect: inner,
                    clip,
                    kind: DrawKind::RoundedRect { color: bg, radius },
                });
            }
        } else {
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect,
                clip,
                kind: DrawKind::Solid(bg),
            });
            if border_w > 0.0 {
                self.queue.push(UiDraw {
                    depth,
                    seq: seq + 1,
                    win,
                    elem,
                    rect,
                    clip,
                    kind: DrawKind::Border { color: border, width: border_w },
                });
            }
        }
    }

    /// 取（或创建）文本排版缓冲，并测量其自然尺寸（**逻辑像素**，宽 = 内容宽，高 = 内容高）。
    ///
    /// 排版缓冲按**物理字号**（`size × scale` 取整到像素）创建并自持于
    /// [`UiState::text_buffers`]（[`CachePolicy::User`]，不推入 `rjw_text` 内部 LRU）：
    /// 静态标签每帧命中缓存，跳过重复整形；测量结果 ÷ scale 返回逻辑尺寸。
    ///
    /// **整数不变量**：物理尺寸（[`Text::measure_buffer`]，已取整）÷ scale 后**再取整**
    /// （`ceil`）返回——布局光标累加（`child_rect` 的 `cursor += h + gap`）与后续
    /// 加法链的操作数全部为整数（scale = 1.0 时测量结果本就是整数，无任何变化）。
    /// 
    /// **行高版本**：行高 = 字号（而非 1.2 倍），保证字形在文本框内垂直居中位置正确。
    /// 版本号改变时，所有缓存会自动失效，避免新旧行高混用。
    /// 取（或创建）文本排版缓冲，并测量其自然尺寸（**逻辑像素**，宽 = 内容宽，高 = 内容高）。
    ///
    /// 排版缓冲按**物理字号**（`size × scale` 取整到像素）创建并自持于
    /// [`UiState::text_buffers`]（[`CachePolicy::User`]，不推入 `rjw_text` 内部 LRU）：
    /// 静态标签每帧命中缓存，跳过重复整形；测量结果 ÷ scale 返回逻辑尺寸。
    ///
    /// **整数不变量**：物理尺寸（[`Text::measure_buffer`]，已取整）÷ scale 后**再取整**
    /// （`ceil`）返回——布局光标累加（`child_rect` 的 `cursor += h + gap`）与后续
    /// 加法链的操作数全部为整数（scale = 1.0 时测量结果本就是整数，无任何变化）。
    /// 
    /// **行高版本**：行高 = 字号（而非 1.2 倍），保证字形在文本框内垂直居中位置正确。
    /// 版本号改变时，所有缓存会自动失效，避免新旧行高混用。
    /// 测量文本自然尺寸（**逻辑像素**，ceil 取整；控件作者在 [`Widget::size`] 测量用；
    /// 内部排版缓冲按物理字号缓存，`family = None` = 系统默认字体）。
    pub fn text_size(&mut self, s: &str, size: f32, family: Option<&str>) -> Vec2 {
        let buf = self.cache_buffer(s, size, family);
        (Text::measure_buffer(&buf) / self.scale).ceil()
    }

    /// 按**换行宽度**测量文本自然尺寸：`wrap_logical > 0` 时文本在宽度内自动换行
    /// （宽 = min(自然宽, wrap)，高 = 行数 × 行高）；否则同 [`Self::text_size`]。
    pub fn text_size_wrap(&mut self, s: &str, size: f32, family: Option<&str>, wrap_logical: f32) -> Vec2 {
        let buf = self.cache_buffer_wrap(s, size, family, wrap_logical);
        (Text::measure_buffer(&buf) / self.scale).ceil()
    }

    /// 剪贴板快捷键（Ctrl+C/V/X/A）共用实现已迁至 [`crate::edit::clipboard_shortcuts`]
    /// （纯逻辑，单行 / 多行输入框共用），本处不再保留。

    /// **控件自持排版缓冲**：`WidgetState::text_buf` 命中复用（key 含文本/字号/字体/
    /// 换行宽/**行距**/版本），未命中直接构建——**不写** `UiState::text_buffers` 全局缓存
    /// （文本频繁变化的输入框不污染静态标签缓存）。
    ///
    /// `line_mult`：行高 = 字号 × 行距倍率（1.0 = 无行距；TextArea 多行用 1.2 加行距）。
    /// 用两次独立借用实现（`self.state` 与 `self.text` 不能同时可变借用）。
    fn ensure_text_buf(
        &mut self,
        id: &str,
        s: &str,
        size: f32,
        family: Option<&str>,
        wrap_logical: f32,
        line_mult: f32,
    ) -> Arc<Buffer> {
        let size_px = (size * self.scale).round();
        let wrap_px = (wrap_logical * self.scale).round().max(0.0);
        let mult_bits = line_mult.to_bits();
        let key = format!(
            "{s}\u{1}{size_px}\u{1}{}\u{1}{wrap_px}\u{1}{mult_bits}\u{1}{TEXT_LINE_HEIGHT_VERSION}",
            family.unwrap_or("")
        );
        if let Some((k, b)) = self.state.widgets.get(id).and_then(|w| w.text_buf.as_ref()) {
            if *k == key {
                return b.clone();
            }
        }
        let lh = (size_px as f32 * line_mult.max(1.0)).round();
        let attrs = match family {
            Some(f) if !f.is_empty() => Attrs::new().family(Family::Name(f)),
            _ => Attrs::new(),
        };
        let buf = self
            .text
            .create_buffer_wrap(s, attrs, size_px, lh, Align::Left, wrap_px, CachePolicy::User);
        if let Some(ws) = self.state.widgets.get_mut(id) {
            ws.text_buf = Some((key, buf.clone()));
        }
        buf
    }

    /// 按字符**实际宽度**把点击位置（相对内容左缘，逻辑像素）映射为最近的光标 char 索引。
    ///
    /// 用"前缀宽度"二分（宽度随前缀长度单调不减）——混合中英文（字宽不同）时
    /// 比等比估算（`total_w × k / n`）精确；纯中文（等宽）两者一致。
    fn caret_index_at_width(
        &mut self,
        value: &str,
        size: f32,
        family: Option<&str>,
        cx: f32,
    ) -> usize {
        let chars: Vec<char> = value.chars().collect();
        let n = chars.len();
        caret_index_by_width(n, cx, |k| {
            let s: String = chars[..k].iter().collect();
            self.text_size(&s, size, family).x
        })
    }

    /// 文本超宽时省略（内容自洽，noclip）：宽度 > `max_w` → 返回 "…" 截断串；
    /// 否则 `None`（原样绘制）。按钮 / 勾选 / 下拉等固定 rect 控件的文本自动省略
    /// （Resizable 窗口缩窄 / max 约束下不溢出）。
    fn ellipsized(
        &mut self,
        s: &str,
        size: f32,
        family: Option<&str>,
        max_w: f32,
    ) -> Option<String> {
        let natural = self.text_size(s, size, family).x;
        if natural > max_w {
            Some(
                crate::edit::ellipsize(s, max_w, |t| self.text_size(t, size, family).x)
                    .into_owned(),
            )
        } else {
            None
        }
    }

    /// 取（或创建）共享排版缓冲（物理字号取整到像素；`CachePolicy::User`：不进 rjw_text LRU）。
    /// 
    /// 行高 = 字号（`size_px`），保证文本框内字形垂直居中位置正确。
    /// 缓存键包含 `TEXT_LINE_HEIGHT_VERSION`，修改行高策略后旧缓存自动失效。
    fn cache_buffer(&mut self, s: &str, size: f32, family: Option<&str>) -> Arc<Buffer> {
        self.cache_buffer_wrap(s, size, family, 0.0)
    }

    /// 取（或创建）共享排版缓冲（物理字号取整到像素；`CachePolicy::User`：不进 rjw_text LRU）。
    ///
    /// `wrap_logical <= 0` = 不换行（默认宽裕宽度）；`> 0` = 按该**逻辑像素**宽度换行
    /// （物理像素取整后传给 rjw_text；换行宽度参与缓存键，不同宽度各自缓存）。
    ///
    /// 行高 = 字号（`size_px`），保证文本框内字形垂直居中位置正确。
    /// 缓存键包含 `TEXT_LINE_HEIGHT_VERSION`，修改行高策略后旧缓存自动失效。
    fn cache_buffer_wrap(
        &mut self,
        s: &str,
        size: f32,
        family: Option<&str>,
        wrap_logical: f32,
    ) -> Arc<Buffer> {
        // 物理字号取整：字形在像素网格上，避免亚像素渲染模糊；测量/绘制/缓存键一致
        let size_px = (size * self.scale).round();
        let wrap_px = (wrap_logical * self.scale).round().max(0.0);
        let key = (
            s.to_owned(),
            size_px.to_bits(),
            family.map(|f| f.to_owned()),
            wrap_px.to_bits(),
            TEXT_LINE_HEIGHT_VERSION,
        );
        if let Some(b) = self.state.text_buffers.get_mut(&key) {
            // 命中：刷新"最后使用帧号"（帧级近似 LRU 驱逐依据）
            b.1 = self.state.frame;
            return b.0.clone();
        }
        // 行高 = 字号（精确 1:1），让字形在行盒中垂直居中更准确
        let lh = size_px;
        let attrs = match family {
            Some(f) if !f.is_empty() => Attrs::new().family(Family::Name(f)),
            _ => Attrs::new(),
        };
        let buf = self.text.create_buffer_wrap(
            s,
            attrs,
            size_px,
            lh,
            Align::Left,
            wrap_px,
            CachePolicy::User,
        );
        // 满容量：先驱逐**本帧未使用**的条目（保留静态标签），仍满（本帧全在用）
        // 则驱逐最旧一条。**不再整表清空**——否则动态文本（FPS 计数、日志等）每帧
        // 变化会连带全部静态标签每帧重新整形（缓存抖动，debug 下可致录制耗时翻倍）。
        if self.state.text_buffers.len() >= TEXT_BUFFER_CACHE_CAP {
            let frame = self.state.frame;
            self.state.text_buffers.retain(|_, (_, used)| *used == frame);
            if self.state.text_buffers.len() >= TEXT_BUFFER_CACHE_CAP {
                let oldest = self
                    .state
                    .text_buffers
                    .iter()
                    .min_by_key(|(_, (_, used))| *used)
                    .map(|(k, _)| k.clone());
                if let Some(k) = oldest {
                    self.state.text_buffers.remove(&k);
                }
            }
        }
        self.state.text_buffers.insert(key, (buf.clone(), self.state.frame));
        buf
    }

    /// 取（或创建）**自动换行**排版缓冲（`wrap_logical > 0`；`<= 0` = 不换行同
    /// [`Self::cache_buffer`]）。供 widget 层与绘制路径按"渲染与测量同缓冲"使用。
    /// 取（或创建）**按宽度换行**的排版缓冲（`wrap_logical <= 0` = 不换行）；
    /// 控件作者画多行/换行文本时用（配 [`Self::push_text_rect`] 的 `buf` 参数）。
    pub fn wrap_buffer(
        &mut self,
        s: &str,
        size: f32,
        family: Option<&str>,
        wrap_logical: f32,
    ) -> Arc<Buffer> {
        self.cache_buffer_wrap(s, size, family, wrap_logical)
    }

    /// **控件作者绘制原语**：实心矩形（逻辑坐标；`w/h <= 0` 跳过）。
    pub fn push_solid_rect(&mut self, rect: Rect, color: Color) {
        if rect.w > 0.0 && rect.h > 0.0 {
            let elem = self.seq + 1;
            let seq = self.next_seq();
            let depth = self.depth;
            let win = self.cur_win;
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect,
                clip: self.clip,
                kind: DrawKind::Solid(color),
            });
        }
    }

    /// **控件作者绘制原语**：矩形边框（逻辑坐标；画在矩形内边缘，宽度取整到物理像素）。
    pub fn push_border_rect(&mut self, rect: Rect, color: Color, width: f32) {
        if rect.w > 0.0 && rect.h > 0.0 {
            let elem = self.seq + 1;
            let seq = self.next_seq();
            let depth = self.depth;
            let win = self.cur_win;
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect,
                clip: self.clip,
                kind: DrawKind::Border { color, width },
            });
        }
    }

    /// 推送一条文本绘制命令（供 widget 层与 `*_at` 方法共用；`clip` 为文本局部裁剪，
    /// 外层裁剪自动取当前容器 `self.clip`；`buf = Some` 时直接用预排版缓冲）。
    /// **控件作者绘制原语**（逻辑坐标；`family` 传 `None` = 系统默认字体）。
    pub fn push_text_rect(
        &mut self,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
        family: Option<String>,
        align: TextAlign,
        valign: TextVAlign,
        clip: Option<Rect>,
        buf: Option<Arc<Buffer>>,
    ) {
        let elem = self.seq + 1;
        let seq = self.next_seq();
        let depth = self.depth;
        self.queue.push(text_cmd(
            depth,
            seq,
            self.cur_win,
            elem,
            rect,
            Arc::from(text),
            size,
            color,
            align,
            valign,
            family,
            clip,
            self.clip,
            buf,
        ));
    }

    /// **不服从内容裁剪的文本绘制**（控件作者原语）：
    ///
    /// 语义 = [`Self::push_text_rect`] 且**不附加任何软层（内容裁剪）**——调用方
    /// 承诺文本**内容自洽**（自动换行后高 = 自然高、"…"省略后宽 = 分配宽、滚动
    /// 内容受限），无需按控件边界裁剪。**仍服从强制层**（ScrollView 可视区 /
    /// Clip 沙箱，即 `self.clip`）：父级如 ScrollView 强制裁切时躲不掉；无 Scroll
    /// 的普通容器本来就没有强制层 → 自洽内容画出界（自洽内容本就不会出界）。
    pub fn push_text_rect_noclip(
        &mut self,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
        family: Option<String>,
        align: TextAlign,
        valign: TextVAlign,
        buf: Option<Arc<Buffer>>,
    ) {
        self.push_text_rect(rect, text, size, color, family, align, valign, None, buf)
    }

    /// 鼠标左键状态（含本帧边沿；控件作者交互判断用）。
    #[inline]
    pub fn mouse_left(&self) -> KeyState {
        self.mouse.get(MouseButton::Left)
    }

    /// 鼠标绝对坐标 → 当前容器局部坐标（逻辑像素，字段运算避免方法借用）。
    #[inline]
    fn mouse_local_x(&self) -> f32 {
        self.mouse_logical.x - self.abs_base.x
    }

    /// 局部矩形（逻辑）→ 命中测试（与逻辑鼠标坐标比较；含窗口外判定与窗口遮挡）。
    ///
    /// 遮挡时若控件矩形本身命中鼠标，累加 [`UiState::occluded_hits`]（诊断机制：
    /// 告诉你"本帧有多少次点击被窗口遮挡拦截"——见示例 `eg260818UI` 的诊断面板）。
    /// **强制裁剪层命中**：鼠标在 [`Self::clip`]（ScrollView 可视区 / Clip 沙箱）
    /// 之外时**不命中**——修复"滚出可视区的控件边缘仍可交互"缺口。
    /// 控件作者交互判断用（与 [`Self::mouse_left`] / `hit::update_interact` 组合）。
    #[inline]
    pub fn hit_abs(&mut self, local: &Rect) -> bool {
        if !self.mouse_in_window {
            return false;
        }
        // 面板/窗口**真正拖拽中**（按下后位移 ≥ DRAG_ACTIVATE_PX）抑制子控件交互
        // （防止拖动中误触按钮等）；纯点击不进入拖拽，子控件正常响应。
        if self.drag_panel.is_some() {
            return false;
        }
        let abs = Rect::new(
            self.abs_base.x + local.x,
            self.abs_base.y + local.y,
            local.w,
            local.h,
        );
        if !hit_test(&abs, self.mouse_logical) {
            return false;
        }
        // 强制裁剪层（Clip 沙箱 / ScrollView 可视区）：层外命中失效。
        if let Some(c) = self.clip {
            if !c.contains_point(self.mouse_logical) {
                return false;
            }
        }
        // **窗口遮挡**（点击穿透修复）：鼠标下若有更高 z 的窗口（`win=0` 内容被任意
        // 窗口）覆盖本控件所在窗口 → 本窗口不得响应——重叠区域只让最上层窗口交互，
        // 背后窗口的控件不会误触发。窗口矩形来自 [`UiState::window_rects`]（跨帧缓存）。
        if window_occluded(self.cur_win, self.mouse_logical, self.window_rects_iter()) {
            // 命中但被遮挡 → 记录诊断计数（未响应）。
            self.state.occluded_hits += 1;
            return false;
        }
        true
    }

    /// 窗口遮挡判定用的窗口矩形迭代器（`(z, rect)`；逻辑像素）。
    #[inline]
    fn window_rects_iter(&self) -> impl Iterator<Item = (u32, Rect)> + '_ {
        self.state.window_rects.iter().map(|(&z, &r)| (z, r))
    }

    /// **诊断**：当前窗口 z-order（按 z 升序）：`(id, z)`。
    pub fn window_order(&self) -> Vec<(String, u32)> {
        let mut v: Vec<(String, u32)> = self
            .state
            .window_z
            .iter()
            .map(|(id, &z)| (id.clone(), z))
            .collect();
        v.sort_by_key(|&(_, z)| z);
        v
    }

    /// **诊断**：鼠标下**最上层**的窗口（`id, z`）——重叠点击时唯一可交互的窗口；
    /// 鼠标不在任何窗口上时返回 `None`。窗口矩形来自跨帧缓存（含本帧已录制的窗口）。
    pub fn window_under_mouse(&self) -> Option<(String, u32)> {
        let mut best: Option<(String, u32)> = None;
        for (id, &z) in &self.state.window_z {
            if let Some(r) = self.state.window_rects.get(&z) {
                if r.contains_point(self.mouse_logical) {
                    match &best {
                        Some((_, bz)) if *bz >= z => {}
                        _ => best = Some((id.clone(), z)),
                    }
                }
            }
        }
        best
    }

    /// 当前容器为子项分配局部矩形（顶层无容器时 panic，顶层请用 `*_at`）。
    /// 控件作者做"占光标"式自定义容器时用（相对当前容器内容原点）。
    pub fn child_rect(&mut self, w: f32, h: f32) -> Rect {
        self.frames
            .last_mut()
            .expect("顶层控件请用 *_at(pos, ...) 定位（容器内才可用无 pos 形式）")
            .child_rect(w, h)
    }

    /// 同 [`Self::child_rect`]，但 `expands = false` 时该子项**不撑大父级**
    /// （[`crate::widget::Expansion::DisableAutoExpansion`] 控件用）。
    pub fn child_rect_exp(&mut self, w: f32, h: f32, expands: bool) -> Rect {
        self.frames
            .last_mut()
            .expect("顶层控件请用 *_at(pos, ...) 定位（容器内才可用无 pos 形式）")
            .child_rect_exp(w, h, expands)
    }

    /// **放置控件**（[`crate::widget::Widget`] trait）：容器内**占光标**（尺寸 = 控件
    /// 测量值经 [`crate::widget::SizeConstraints`] clamp 与膨胀模式调整）；返回统一
    /// 交互响应 [`crate::widget::Response`]。顶层无容器时请用 [`Self::add_at`]。
    ///
    /// 属性化 builder 示例：`ui.add(Button::new("ok", "确定").color(Color::WHITE))`。
    /// 容器包装（`Panel` / `Pack` / `Grid` / `Window` / `Scroll` / `FlexCtx`）经
    /// [`UiAdd`] 提供同样的 `add` / `add_at` 与全部便捷方法（`p.button` / `p.label` 等）。
    pub fn add(&mut self, w: impl crate::widget::Widget) -> crate::widget::Response {
        let (size, expands) = self.widget_size(&w);
        let rect = self.child_rect_exp(size.x, size.y, expands);
        w.ui(self, rect)
    }

    /// **绝对定位放置控件**（`pos` 相对当前容器内容原点；不占光标）。
    pub fn add_at(
        &mut self,
        pos: Vec2,
        w: impl crate::widget::Widget,
    ) -> crate::widget::Response {
        let (size, _) = self.widget_size(&w);
        w.ui(self, Rect::new(pos.x, pos.y, size.x, size.y))
    }

    /// 测量控件最终放置尺寸：`size()` 自然值 → `SizeConstraints` clamp → 按
    /// `Expansion` 模式调整（`LimitedInParent` 限制在父级可用宽内），并返回该
    /// 控件是否**撑大父级**（`DisableAutoExpansion` = 否）。
    fn widget_size(&mut self, w: &impl crate::widget::Widget) -> (Vec2, bool) {
        let natural = w.size(self);
        let c = w.constraints();
        let mut size = crate::widget::apply_constraints(natural, c);
        let expands = match w.expansion() {
            crate::widget::Expansion::DisableAutoExpansion => false,
            crate::widget::Expansion::LimitedInParent => {
                if let Some(avail) = self.avail_w() {
                    if avail < size.x {
                        size.x = avail;
                    }
                }
                true
            }
            crate::widget::Expansion::UnlimitedExpansion => true,
        };
        (size, expands)
    }

    /// 通用容器：push 帧 → 闭包 → 结算（返回尺寸与最大子尺寸）→ 平移子命令 → pop。
    fn container<F>(&mut self, pos: Vec2, frame: Frame, f: F) -> (Vec2, Vec2)
    where
        F: FnOnce(&mut ContainerCtx<'_, '_>),
    {
        let start = self.queue.len();
        let saved_base = self.abs_base;
        self.abs_base = saved_base + pos;
        self.frames.push(frame);
        self.depth += 1;
        f(&mut ContainerCtx { ui: self });
        let frame = self.frames.pop().expect("container frame");
        let size = frame.settle_size();
        let max_child = frame.max_child;
        self.depth -= 1;
        self.abs_base = saved_base;
        for d in &mut self.queue[start..] {
            d.translate(pos);
        }
        (size, max_child)
    }

    /// **View 沙箱**（闭包作用域，见 [`crate::view`]）：进入沙箱后——
    ///
    /// - [`ViewMode::Clip`]：内容超出沙箱**强制裁剪**（外层裁剪 ∩ 沙箱可视区），
    ///   沙箱外的鼠标**命中失效**（`hit_abs` 带沙箱判定）；
    /// - [`ViewMode::Expand`]：不裁剪，内容自然尺寸可溢出沙箱并撑大外层容器；
    ///   沙箱提供"可用宽度"（[`Self::avail_w`]），供 `LimitedInParent` 控件自洽
    ///   （自动换行 / "…"省略）。
    ///
    /// 沙箱内录制的命令随弹出统一平移 `pos`（相对当前容器内容原点，不占父光标）。
    /// 返回内容结算尺寸（`Expand` 下可大于 `size`）。**ScrollView**（[`Self::scroll_at`]、
    /// 文本编辑框）与严格窗口（[`Self::window_at_strict`]）的公共底座。
    pub fn view_at(
        &mut self,
        pos: Vec2,
        size: Vec2,
        mode: ViewMode,
        f: impl FnOnce(&mut ViewCtx<'_, '_>),
    ) -> Vec2 {
        let saved_clip = self.clip;
        let saved_base = self.abs_base;
        let view_rel = Rect::new(pos.x, pos.y, size.x.max(0.0), size.y.max(0.0));
        let view_abs = Rect::new(
            saved_base.x + view_rel.x,
            saved_base.y + view_rel.y,
            view_rel.w,
            view_rel.h,
        );
        // 强制裁剪层（Clip 模式：外层 ∩ 可视区；Expand：原样传递）。
        self.clip = clip_for_view(saved_clip, view_abs, mode);
        // 可用宽度栈：沙箱内 avail_w() = 沙箱宽。
        self.avail_stack.push(Some(view_rel.w));
        let start = self.queue.len();
        self.abs_base = saved_base + pos;
        self.frames.push(Frame::new_stack(PackSide::Top, self.theme.gap, 0.0));
        self.depth += 1;
        f(&mut ViewCtx { ui: self });
        let frame = self.frames.pop().expect("view frame");
        let content = frame.settle_size();
        self.depth -= 1;
        self.abs_base = saved_base;
        self.avail_stack.pop();
        self.clip = saved_clip;
        for d in &mut self.queue[start..] {
            d.translate(pos);
        }
        content
    }

    /// 当前可用的**内容宽度**（逻辑像素）：沙箱宽 → 容器固定宽（`window_at_w` 等，
    /// 经 `Frame::fixed_avail_w`）→ 下一子项 max 约束，取最小；无任何约束 = `None`
    /// （内容自然宽度）。供 `LimitedInParent` 控件（如 [`crate::widget::Label`]）自洽
    /// 溢出（自动换行 / 省略号）。
    #[inline]
    pub fn avail_w(&self) -> Option<f32> {
        let base = self
            .avail_stack
            .last()
            .copied()
            .flatten()
            .or_else(|| self.frames.last().and_then(|f| f.fixed_avail_w()));
        let nm = self.frames.last().map(|f| f.next_max_w()).unwrap_or(0.0);
        match (base, nm) {
            (Some(b), n) if n > 0.0 => Some(b.min(n)),
            (b, _) => b,
        }
    }

    /// **分割线**（绝对定位水平线）：`pos` 相对当前容器内容原点，宽 `w`（逻辑像素）。
    /// 线画在 `pos.y + margin`（上下留白由调用方行高体现）。样式取
    /// [`Theme::divider`](crate::style::Theme::divider)。
    pub fn divider_at(&mut self, pos: Vec2, w: f32) {
        let st = self.theme.divider.clone();
        if w > 0.0 && st.thickness > 0.0 {
            self.push_solid_rect(Rect::new(pos.x, pos.y + st.margin, w, st.thickness), st.color);
        }
    }

    /// **滚动容器（ScrollView）**：内容在 `view_size` 可视区内垂直堆叠（pack Top），
    /// 超出部分滚动查看——**滚轮**滚动 + 右侧**滚动条**（拖 thumb / 点轨道翻页）。
    ///
    /// - `id`：滚动偏移状态键（[`UiState::scrolls`]，跨帧持久）；
    /// - 内容子项照常录制（`s.label` / `s.button` 等，占光标堆叠）；
    /// - 可视区之外的图形/文字**强制裁剪**（Clip 沙箱：`UiDraw.clip` 绝对逻辑矩形，
    ///   收集期求交，**含 noclip 绘制**）；
    /// - 沙箱内 `avail_w()` = 可视区宽（`LimitedInParent` 控件自洽）；
    /// - 返回 `view_size`（内容尺寸超出时可经 [`UiState::scrolls`] 读取）。
    ///
    /// **内部计算一律物理像素**（DPI 只在该换算处出现一次）：滚动偏移
    /// [`ScrollState::offset`] 为**物理像素**整数步进（滚轮 / 拖 thumb 均取整），
    /// 内容按 `offset_px / scale` 逻辑平移后 ×scale 回到整物理像素——
    /// **整体刚性移动**，非整数 DPI（125%/150%）下相邻元素取整相位不抖。
    pub fn scroll_at(
        &mut self,
        pos: Vec2,
        view_size: Vec2,
        id: &str,
        f: impl FnOnce(&mut Scroll<'_, '_>),
    ) -> Vec2 {
        let saved_clip = self.clip;
        let saved_base = self.abs_base;
        // 可视区（**相对**当前容器 origin：内容 / 滚动条命令都录在容器局部坐标，
        // 随外层容器弹出统一平移成绝对坐标）。
        let view_rel = Rect::new(pos.x, pos.y, view_size.x.max(0.0), view_size.y.max(0.0));
        // 可视区（**绝对**逻辑屏幕坐标：裁剪 / 滚轮命中用）。
        let view_abs = Rect::new(
            saved_base.x + pos.x,
            saved_base.y + pos.y,
            view_size.x.max(0.0),
            view_size.y.max(0.0),
        );
        // 强制裁剪层 = 外层裁剪 ∩ 本可视区（View 沙箱 Clip 语义）。
        self.clip = clip_for_view(saved_clip, view_abs, ViewMode::Clip);
        // 滚动偏移（**物理像素**，跨帧状态；先 Copy 读出，`f` 结束再写回——避免
        // 借用冲突）。以整物理像素步进（滚轮 / 拖 thumb 均取整）。
        let mut offset_px = self
            .state
            .scrolls
            .get(id)
            .map(|s| s.offset)
            .unwrap_or(0.0);
        // 可用宽度栈：滚动容器内 avail_w() = 可视区宽（LimitedInParent 控件自洽）。
        self.avail_stack.push(Some(view_rel.w));
        // 内容 pack 堆叠（手动管理帧栈：平移 = pos - offset_px/scale，而非 container 的 pos）。
        let start = self.queue.len();
        // abs_base = 内容**渲染**原点（已含 -offset 滚动偏移）——`hit_abs`（点击
        // 命中）/ `register_focus`（焦点描边）/ IME 光标定位都经 abs_base 换算，
        // 必须与平移后的绘制位置一致，否则点击位置跟不上滚动视图（offset ≠ 0 时
        // 命中落在未滚动坐标上）。
        self.abs_base = saved_base + pos - Vec2::new(0.0, offset_px / self.scale);
        self.frames.push(Frame::new_stack(PackSide::Top, self.theme.gap, 0.0));
        self.depth += 1;
        f(&mut Scroll { ui: self });
        let frame = self.frames.pop().expect("scroll frame");
        let content_size = frame.settle_size();
        self.depth -= 1;
        self.abs_base = saved_base;
        self.avail_stack.pop();
        let max_off_px = ((content_size.y - view_size.y).max(0.0) * self.scale).round();
        offset_px = offset_px.clamp(0.0, max_off_px);
        // 滚轮（鼠标在可视区内且未被窗口遮挡；wheel y 向上为正 → offset 减小）。
        // 每格 40 逻辑像素 → 物理像素取整（trackpad 连续增量同样按格取整步进）。
        let hit = hit_test(&view_abs, self.mouse_logical)
            && self.mouse_in_window
            && !window_occluded(self.cur_win, self.mouse_logical, self.window_rects_iter());
        if hit {
            let (_, wy) = self.mouse.get_mouse_wheel_delta();
            if wy != 0.0 {
                offset_px = (offset_px - (wy as f32 * 40.0 * self.scale).round())
                    .clamp(0.0, max_off_px);
            }
        }
        // 平移内容子命令：局部坐标 → 绝对（`UiDraw::clip` 已是绝对，不随平移——见其
        // 文档）。offset_px/scale 为逻辑小数：绘制时 ×scale 回到整物理像素 → 刚性平移。
        for d in &mut self.queue[start..] {
            d.translate(pos - Vec2::new(0.0, offset_px / self.scale));
        }
        // 滚动条（内容超出可视区时显示；拖 thumb / 点轨道翻页）——在**当前容器局部
        // 坐标**绘制，**不参与**上面的内容平移；随外层容器弹出统一平移成绝对坐标。
        if content_size.y > view_size.y + 1.0 && view_size.y > 0.0 {
            offset_px = self.scrollbar(
                id,
                &view_rel,
                view_size.y,
                content_size.y,
                offset_px,
                max_off_px,
                saved_clip,
                0,
            );
        }
        // 写回滚动状态（`f` 借用已结束；offset 为物理像素）。
        let st = self.state.scrolls.entry(id.to_owned()).or_default();
        st.offset = offset_px;
        st.content_h = content_size.y;
        self.clip = saved_clip;
        view_size
    }

    /// **选择列表**：`scroll_at` + 逐项回调（选中态由调用方维护）。
    ///
    /// `item` 回调 `(容器, 索引, 是否选中) -> bool`：返回 `true` 表示该项被点击。
    /// 返回本帧被点击的索引（`None` = 无）。
    pub fn list_at<F>(
        &mut self,
        pos: Vec2,
        view_size: Vec2,
        id: &str,
        count: usize,
        selected: Option<u32>,
        mut item: F,
    ) -> Option<u32>
    where
        F: FnMut(&mut Scroll<'_, '_>, u32, bool) -> bool,
    {
        let mut clicked = None;
        self.scroll_at(pos, view_size, id, |s| {
            for i in 0..count as u32 {
                if item(s, i, selected == Some(i)) && clicked.is_none() {
                    clicked = Some(i);
                }
            }
        });
        clicked
    }

    /// 滚动条：右侧竖条（轨道 + thumb）。返回更新后的滚动偏移（**物理像素**）。
    ///
    /// `view` 为**当前容器局部坐标**的可视区（与内容同空间，**不随内容滚动**；
    /// 由外层容器弹出统一平移成绝对坐标）；命中用局部坐标鼠标（`mouse_logical −
    /// abs_base`），遮挡判定仍用绝对鼠标。thumb 几何在物理像素里取整，拖拽按
    /// **整物理像素 1:1** 步进——内容与 thumb 刚性移动（非整数 DPI 不抖）。
    /// `elem`：所属元素序（`scroll_at` 传 `0` 装饰层；文本编辑框传 `seq+1` 使
    /// 滚动条覆盖在文本之上）。
    #[allow(clippy::too_many_arguments)]
    fn scrollbar(
        &mut self,
        id: &str,
        view: &Rect,
        view_h: f32,
        content_h: f32,
        offset_px: f32,
        max_off_px: f32,
        outer_clip: Option<Rect>,
        elem: u32,
    ) -> f32 {
        let mut offset_px = offset_px;
        let track = Rect::new(view.x + view.w - SCROLLBAR_W, view.y, SCROLLBAR_W, view_h);
        let ratio = (view_h / content_h).clamp(0.0, 1.0);
        // thumb 几何：**物理像素**计算（整像素步进 → 刚性），绘制转回逻辑坐标。
        let scale = self.scale;
        let view_h_px = (view_h * scale).round();
        let thumb_h_px = (view_h_px * ratio).max(16.0 * scale).round();
        let thumb_y_px = if max_off_px > 1e-6 {
            (offset_px / max_off_px * (view_h_px - thumb_h_px)).round()
        } else {
            0.0
        };
        // thumb 顶 = 可视区顶（view.y，局部坐标）+ 视图内偏移（物理像素 / scale）
        let thumb_y = view.y + thumb_y_px / scale;
        let thumb = Rect::new(track.x, thumb_y, track.w, thumb_h_px / scale);
        // 绘制：轨道 + thumb（白纹理图形，`elem` 所属元素）。
        let depth = self.depth;
        let win = self.cur_win;
        let seq = self.next_seq();
        self.queue.push(UiDraw {
            depth,
            seq,
            win,
            elem,
            rect: track,
            clip: outer_clip,
            kind: DrawKind::Solid(self.theme.slider.track),
        });
        self.queue.push(UiDraw {
            depth,
            seq: seq + 1,
            win,
            elem,
            rect: thumb,
            clip: outer_clip,
            kind: DrawKind::Solid(self.theme.slider.handle),
        });
        // 交互：thumb 拖拽（复用 WidgetState.press_panel/press_mouse 基准）。
        // 局部坐标鼠标 = 绝对鼠标 − 当前容器绝对原点（abs_base 已恢复为外层值）。
        let mouse_rel = self.mouse_logical - self.abs_base;
        let bar_id = format!("{id}::bar");
        let bar_hit = hit_test(&thumb, mouse_rel)
            && self.mouse_in_window
            && !window_occluded(win, self.mouse_logical, self.window_rects_iter());
        let btn = self.mouse_left();
        // 滚动条自身有拖拽语义：按下（thumb / 轨道区域）置位 press_claimed，
        // 阻止外层窗口把本次按下当作窗口拖拽基准（窗口内拖滚动条不连窗口一起动）。
        if btn.down_edge()
            && hit_test(&track, mouse_rel)
            && self.mouse_in_window
            && !window_occluded(win, self.mouse_logical, self.window_rects_iter())
        {
            self.press_claimed = true;
        }
        let grab = {
            let ws = self.state.widgets.entry(bar_id.clone()).or_default();
            let dragging = update_drag(ws, bar_hit, btn);
            if btn.down_edge() && bar_hit {
                ws.press_mouse = Some(self.mouse_screen.round());
                ws.press_panel = Some(Vec2::new(thumb_y_px, offset_px));
            }
            (dragging, ws.press_panel.unwrap_or(Vec2::ZERO))
        };
        if grab.0 {
            let pm = self
                .state
                .widgets
                .get(&bar_id)
                .and_then(|w| w.press_mouse)
                .unwrap_or(self.mouse_screen);
            // thumb **跟随鼠标 1:1**（保持按下时的抓取点偏移），滚动偏移由 thumb
            // 位置反推——否则 thumb 按比例慢于鼠标（内容越高越明显，"不同步"）。
            let dy_px = (self.mouse_screen.y - pm.y).round();
            let thumb_y_px_new = grab.1.x + dy_px; // grab.1.x = 按下时 thumb_y_px
            offset_px = if view_h_px - thumb_h_px > 1.0 {
                (thumb_y_px_new / (view_h_px - thumb_h_px) * max_off_px)
                    .round()
                    .clamp(0.0, max_off_px)
            } else {
                0.0
            };
        }
        // 光标：视口滑条（thumb/轨道）保持普通 Arrow（UI_NEEDS：滑条不用 <->）。
        // 轨道点击（thumb 外）→ 翻页（整物理像素步长）。
        let hit_track = hit_test(&track, mouse_rel)
            && self.mouse_in_window
            && !window_occluded(win, self.mouse_logical, self.window_rects_iter());
        let page_px = (view_h * scale).round();
        if btn.down_edge() && hit_track && !bar_hit {
            if mouse_rel.y < thumb_y {
                offset_px = (offset_px - page_px).max(0.0);
            } else if mouse_rel.y > thumb_y + thumb_h_px / scale {
                offset_px = (offset_px + page_px).min(max_off_px);
            }
        }
        offset_px
    }

    // ── 顶层入口（*_at：位置显式，尺寸自动） ─────────────────

    /// 绝对定位标签（`pos` 相对当前容器内容原点；顶层即屏幕原点）。
    pub fn label_at(&mut self, pos: Vec2, text: &str) -> Vec2 {
        let elem = self.seq + 1;
        let seq = self.next_seq();
        let style = self.theme.label.clone();
        let size = self.text_size(text, style.font_size, style.font_family.as_deref());
        let rect = Rect::new(pos.x, pos.y, size.x, size.y);
        self.queue.push(text_cmd(
            self.depth,
            seq,
            self.cur_win,
            elem,
            rect,
            Arc::from(text),
            style.font_size,
            style.color,
            TextAlign::from(style.align),
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.clip,
        None,
        ));
        size
    }

    /// **自动换行标签**：`max_w`（逻辑像素）内按词/字换行，返回自然尺寸
    /// （宽 = min(自然宽, max_w)，高 = 行数 × 行高）。`max_w <= 0` = 不换行（同 [`Self::label_at`]）。
    ///
    /// 换行宽度参与排版缓存键（不同宽度各自缓存）；多行文本垂直居中于矩形。
    pub fn label_wrap_at(&mut self, pos: Vec2, max_w: f32, text: &str) -> Vec2 {
        let elem = self.seq + 1;
        let seq = self.next_seq();
        let style = self.theme.label.clone();
        let size = self.text_size_wrap(text, style.font_size, style.font_family.as_deref(), max_w);
        let rect = Rect::new(pos.x, pos.y, size.x, size.y);
        // 换行标签：直接传预排版缓冲（渲染与测量同一缓冲）——否则绘制期按不换行
        // 排版，长文本会单行溢出而非自动换行。
        let buf = if max_w > 0.0 {
            Some(self.wrap_buffer(text, style.font_size, style.font_family.as_deref(), max_w))
        } else {
            None
        };
        self.queue.push(text_cmd(
            self.depth,
            seq,
            self.cur_win,
            elem,
            rect,
            Arc::from(text),
            style.font_size,
            style.color,
            TextAlign::from(style.align),
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.clip,
            buf,
        ));
        size
    }

    /// **窗口/面板位置责任链**：注册一个位置解析器（脚本 / 动画 / 自动布局提供者）。
    ///
    /// 解析顺序（**优先级降序**，第一个返回 `Some` 的生效）：
    /// 1. 应用注册的处理器（`priority` 越大越先问）；
    /// 2. 内置**用户拖拽状态**（[`UiState::panel_pos`]，固定优先级 `0`）——用户拖过
    ///    就永远赢过负优先级脚本，松开后停在用户放置处；
    /// 3. 调用者传入的 `pos`（终端兜底，恒最后）。
    ///
    /// 优先级选择：
    /// - `priority < 0`（如 `-10`）：动画 / 自动布局——**用户拖拽优先**（拖拽中
    ///   `panel_pos` 先于脚本被询问，窗口跟手；脚本不阻塞拖动）；
    /// - `priority > 0`（如 `+10`）：**脚本锁定位置**——程序控制优先，拖拽被覆盖
    ///   （切场景锁窗口 / 剧情镜头等）；脚本返回 `None` 即交还控制权。
    ///
    /// **闭包须 `'static`**：可捕获拥有值 / `Copy` 值（如 [`std::time::Instant`] 时间
    /// 基准）/ `Arc`；需要与主循环共享可变状态时用 `Arc<Mutex<_>>`。这保证处理器
    /// 不借用 `self`——`ui.finish()` 之后应用仍可正常访问自己的状态。
    ///
    /// 示例（HUD 自动左右摆动，但用户仍可拖动——`-10 < 0` 拖拽优先）：
    /// ```no_run
    /// # let viewport = todo!(); let mouse = todo!(); let keyboard = todo!();
    /// # let text = todo!(); let r2d = todo!(); let state = todo!(); let window = todo!();
    /// use rjw_ui::{Theme, Ui, UiAdd};
    /// let mut ui = Ui::begin(&window, &mut text, &mut state)
    ///     .capture(&mouse, &keyboard)
    ///     .theme(Theme::dark())
    ///     .build();
    /// let t0 = std::time::Instant::now();
    /// ui.pos_handler(-10, move |id| {
    ///     if id == "hud" {
    ///         let t = t0.elapsed().as_secs_f64();
    ///         Some(glam::Vec2::new(400.0 + 120.0 * (t * 2.0).sin() as f32, 40.0))
    ///     } else {
    ///         None
    ///     }
    /// });
    /// ui.window_at("hud", glam::Vec2::new(400.0, 40.0), |w| { w.label("HUD"); });
    /// ui.finish(&viewport, &mut r2d);
    /// ```
    pub fn pos_handler(&mut self, priority: i32, f: impl Fn(&str) -> Option<Vec2> + 'static) {
        self.pos_chain.push((priority, PosLink::Script(Box::new(f))));
        // 优先级降序（稳定排序：同优先级保持注册顺序）
        self.pos_chain.sort_by(|a, b| b.0.cmp(&a.0));
    }

    /// 责任链解析窗口/面板位置（见 [`Self::pos_handler`]）。
    #[inline]
    fn resolve_pos(&self, id: &str, pos: Vec2) -> Vec2 {
        resolve_pos_link(&self.pos_chain, &self.state.panel_pos, id, pos)
    }

    /// 面板：背景 + 边框 + 内容垂直堆叠（pack Top）；尺寸自动包裹内容。
    pub fn panel_at(&mut self, pos: Vec2, f: impl FnOnce(&mut Panel<'_, '_>)) -> Vec2 {
        self.panel_impl(pos, None, f)
    }

    /// **可拖拽**面板：同 [`Self::panel_at`]，且按住面板任意处**移动 ≥ 3 物理像素**
    /// 可拖动（纯点击不拖拽，面板内子控件正常响应）。
    ///
    /// - 位置持久化于 `UiState.panel_pos`（`id` 须稳定），跨帧跟随鼠标；
    ///   也可经**位置责任链**（[`Self::pos_handler`]）由脚本/动画提供——用户拖拽
    ///   始终优先于负优先级脚本；
    /// - 真正拖动期间**抑制面板内子控件交互**（不会误触发按钮点击）；
    /// - `pos` 为初始位置（首次）；`UiState::reset()` 可复位。
    pub fn drag_panel_at(
        &mut self,
        id: &str,
        pos: Vec2,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        self.panel_impl(pos, Some(id), f)
    }

    /// 面板公共实现：`drag = Some(id)` 时启用拖拽。
    fn panel_impl(
        &mut self,
        pos: Vec2,
        drag: Option<&str>,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        // 拖拽面板的位置从**责任链**读取（脚本处理器 → 用户拖拽状态 → 传入 pos，
        // 见 pos_handler）：首次 / 从未拖过时用传入 pos
        let origin = match drag {
            Some(id) => self.resolve_pos(id, pos),
            None => pos,
        };
        let start = self.queue.len();
        let (pad_total, gap) = {
            let p = &self.theme.panel;
            (p.padding + p.border_w, self.theme.gap)
        };
        let saved_base = self.abs_base;
        self.abs_base = saved_base + origin;
        self.frames.push(Frame::new_stack(PackSide::Top, gap, pad_total));
        self.depth += 1;
        let mut panel = Panel { ui: self };
        f(&mut panel);
        let frame = self.frames.pop().expect("panel frame");
        let size = frame.settle_size();
        self.depth -= 1;
        self.abs_base = saved_base;
        // 拖拽交互：**物理像素粒度**拖动基准（见下方说明）。
        // `display_pos`：本帧实际使用的面板位置（拖拽中 = 新位置，当帧生效，无帧延迟）。
        let display_pos = if let Some(id) = drag {
            let panel_rect = Rect::new(origin.x, origin.y, size.x, size.y);
            // 窗口遮挡：面板是 win=0 内容（绘制在所有窗口之下），被任意窗口覆盖时不可拖拽。
            let hit = hit_test(&panel_rect, self.mouse_logical)
                && self.mouse_in_window
                && !window_occluded(0, self.mouse_logical, self.window_rects_iter());
            let btn = self.mouse_left();
            let press_here = btn.down_edge() && hit;
            // 输入框按下（选择拖拽）不建立面板拖拽基准。
            let drag_here = press_here && !self.press_claimed;
            let (active, new_pos) = {
                let ws = self.state.widgets.entry(id.to_owned()).or_default();
                let dragging = update_drag(ws, hit, btn);
                if drag_here {
                    // 拖动基准：面板位置（逻辑）+ 鼠标物理坐标（**取整**）。
                    // 取整消除鼠标静止噪声（滞回）；拖拽中按**物理像素增量**移动：
                    // 粒度 1 物理 px，DPI 1.5 下也不会出现"移动 1.5px 才动"的粘滞感。
                    ws.press_panel = Some(panel_rect.min());
                    ws.press_mouse = Some(self.mouse_screen.round());
                } else if press_here {
                    // 文本框等子控件按下：清除旧拖拽基准（防窗口"瞬移"，见 window_at）。
                    ws.press_panel = None;
                    ws.press_mouse = None;
                }
                // 拖拽需实际位移（≥ DRAG_ACTIVATE_PX）且**有本帧基准**才激活：纯点击不拖拽，
                // 面板内子控件（按钮/勾选/输入框）正常响应（见 hit_abs 抑制条件）。
                let active = dragging
                    && ws.press_mouse.is_some()
                    && drag_moved(self.mouse_screen.round(), ws.press_mouse);
                let np = if active {
                    let pp = ws.press_panel.unwrap_or(origin);
                    let pm = ws.press_mouse.unwrap_or(self.mouse_screen);
                    // 物理像素增量（round：对噪声滞回，静止时不变）→ 逻辑位移
                    let d = (self.mouse_screen - pm).round();
                    pp + d / self.scale
                } else {
                    origin
                };
                (active, np)
            };
            if active {
                self.drag_panel = Some(id.to_owned());
                // 仅位置变化时写入（滞回：同一位置不重写）
                if self.state.panel_pos.get(id) != Some(&new_pos) {
                    self.state.panel_pos.insert(id.to_owned(), new_pos);
                }
            } else if self.drag_panel.as_deref() == Some(id) {
                self.drag_panel = None;
            }
            if press_here || active {
                // 按下面板（或拖拽中）都算"已响应按下"——避免空白点击清焦点
                self.any_pressed = true;
            }
            // 面板拖动激活 → 强制普通 Arrow（UI_NEEDS：窗体拖动无需 <->）。
            if active {
                self.cursor_window_drag = true;
            }
            new_pos
        } else {
            origin
        };
        // 背景 + 边框（depth = 进入前深度，画在子控件之下；radius > 0 走圆角双层矩形）
        let style = self.theme.panel.clone();
        let bg_rect = Rect::new(0.0, 0.0, size.x, size.y);
        self.push_panel_like(bg_rect, style.bg, style.border, style.border_w, style.radius, 0);
        // 平移全部（子命令 + 背景/边框）：
        // 用 `display_pos`（拖拽中 = 本帧新位置）→ 文字/矩形**当帧生效**。
        for d in &mut self.queue[start..] {
            d.translate(display_pos);
        }
        size
    }

    /// **窗口**容器（可重叠 + 焦点置顶 + 可拖拽）。
    ///
    /// - **可重叠**：多个窗口按 **z-order** 排列（`UiState.window_z`），
    ///   点击窗口即**置顶**（焦点）；z 越大越靠上。
    /// - **遮挡隔离**（点击穿透修复）：重叠区域只让**鼠标下最上层**的窗口响应——
    ///   背后窗口的控件在该区域不命中、不可拖拽/置顶（见 [`crate::hit::window_occluded`]）。
    /// - **可拖拽**：按住窗口任意处**移动 ≥ 3 物理像素**进入拖拽（位置持久于
    ///   `UiState.panel_pos`）；**纯点击不拖拽**——窗口内子控件（按钮 / 勾选框 /
    ///   输入框）正常响应；拖动期间抑制窗口内子控件交互。位置也可经**责任链**
    ///   （[`Self::pos_handler`]）由脚本/动画提供——用户拖拽优先于负优先级脚本，
    ///   正优先级脚本可锁定窗口位置。
    /// - 窗口内绘制顺序由 [`Ui::finish`] 保证：**背景/图形严格先于文字**
    ///   （[`crate::draw::DrawKind::group`]；白纹理组先于字形图集组提交），
    ///   不做窗口内元素重叠处理。
    /// - 窗口内容（含背景/边框）带 `win = z` 标记，`finish` 时按
    ///   `(win, depth, kind, seq)` 排序后，按 `(win 升序, 图形 → 文字, 纹理)` 提交。
    pub fn window_at(
        &mut self,
        id: &str,
        pos: Vec2,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        self.window_impl(id, pos, None, true, false, f)
    }

    /// **严格裁剪窗口**：同 [`Self::window_at`]（可重叠 / 置顶 / 可拖拽 / 自动尺寸），
    /// 且窗口内容**强制裁剪**到窗口矩形（Clip 沙箱：超出窗口的内容被裁，含 noclip
    /// 绘制；外层 ScrollView 裁切一并生效）。命中仍由窗口遮挡机制隔离。
    ///
    /// 默认的 [`Self::window_at`] / [`Self::window_at_w`] 是 **Expand 语义**（不裁剪，
    /// 内容自然尺寸自动换行 / 撑高窗口）——需要严格裁剪时显式用本方法。
    pub fn window_at_strict(
        &mut self,
        id: &str,
        pos: Vec2,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        self.window_impl(id, pos, None, true, true, f)
    }

    /// **固定宽窗口**（宽度指定、**高度自动**，如同 egui；**右下角可鼠标缩放**）：
    /// 内容子项宽度 clamp 到 `width`（未达宽的内容按内容自动排布），高度按内容
    /// 自然结算；拖动右下角缩放柄改宽度（跨帧持久于 `UiState::window_widths`）。
    /// 其余语义同 [`Self::window_at`]（可重叠 / 置顶 / 可拖拽）。
    pub fn window_at_w(
        &mut self,
        id: &str,
        pos: Vec2,
        width: f32,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        // 宽度取持久值（鼠标缩放后的结果；首次 = 传入 width）
        let w = *self.state.window_widths.get(id).unwrap_or(&width);
        self.window_impl(id, pos, Some(w), true, false, f)
    }

    /// **固定宽严格裁剪窗口**：同 [`Self::window_at_w`]，内容强制裁剪到窗口矩形。
    pub fn window_at_strict_w(
        &mut self,
        id: &str,
        pos: Vec2,
        width: f32,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        let w = *self.state.window_widths.get(id).unwrap_or(&width);
        self.window_impl(id, pos, Some(w), true, true, f)
    }

    /// 窗口公共实现（`width = Some(w)` 时固定宽、高度自然；`topmost`：点击是否置顶——
    /// modal 对话框为 `false`，见 [`Self::modal_at`]；`strict`：窗口内容强制裁剪）。
    fn window_impl(
        &mut self,
        id: &str,
        pos: Vec2,
        width: Option<f32>,
        topmost: bool,
        strict: bool,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        // z-order：首次分配 max+1；点击置顶在拖拽判定处处理
        let z = {
            // z-order：首次分配 max+1；点击置顶在拖拽判定处处理。
            // ⚠ 排除置顶哨兵（WIN_TOPMOST）——浮层不参与普通窗口的 z 递增。
            let max_z = self
                .state
                .window_z
                .values()
                .copied()
                .filter(|&z| z < WIN_TOPMOST)
                .max()
                .unwrap_or(0);
            *self.state.window_z.entry(id.to_owned()).or_insert(max_z + 1)
        };
        let saved_win = std::mem::replace(&mut self.cur_win, z);
        let saved_clip = self.clip;
        // 位置经**责任链**解析（脚本处理器 → 用户拖拽状态 → 传入 pos，见 pos_handler）
        let origin = self.resolve_pos(id, pos);
        let start = self.queue.len();
        let (pad_total, gap) = {
            let p = &self.theme.panel;
            (p.padding + p.border_w, self.theme.gap)
        };
        let saved_base = self.abs_base;
        self.abs_base = saved_base + origin;
        let mut frame = Frame::new_stack(PackSide::Top, gap, pad_total);
        if let Some(w) = width {
            frame.set_fixed_w(w);
        }
        self.frames.push(frame);
        self.depth += 1;
        let mut w = Window { ui: self };
        f(&mut w);
        let frame = self.frames.pop().expect("window frame");
        let size = frame.settle_size();
        self.depth -= 1;
        self.abs_base = saved_base;
        // 拖拽 + 按下裁决（物理像素粒度拖动基准，同 panel_impl）：
        // 重叠区域点击按下时，记录"本帧按下命中的**最上层**窗口"（win_press_top），
        // `finish::resolve_win_press` 只保留它的拖拽与置顶——避免同时拖动多个窗口。
        let panel_rect = Rect::new(origin.x, origin.y, size.x, size.y);
        // 固定宽窗口：右下角**缩放柄**（鼠标拖动改宽度，高度自动；跨帧持久于
        // `UiState::window_widths`）。⚠ 交互须在窗口拖拽判定**之前**（claim_press
        // 阻止按下缩放柄时同时建立窗口拖拽基准）。基于通用 [`Self::resize_handle`]。
        if let Some(w) = width {
            let hw = 14.0_f32;
            // handle 为**外层容器局部坐标**（此处 abs_base 已恢复为外层原点）。
            let handle = Rect::new(origin.x + size.x - hw, origin.y + size.y - hw, hw, hw);
            let h_id = format!("{id}::resize");
            if let Some(new_size) = self.resize_handle(
                &h_id,
                handle,
                Vec2::new(w, size.y),
                Vec2::new(120.0, size.y),
                crate::UiCursor::EwResize,
            ) {
                self.state.window_widths.insert(id.to_owned(), new_size.x);
            }
        }
        // 窗口遮挡：被更高 z 的窗口覆盖的区域，本窗口不响应拖拽 / 置顶 /
        // 子控件交互（点击穿透修复——重叠区域只让最上层窗口可交互）。
        let hit = hit_test(&panel_rect, self.mouse_logical)
            && self.mouse_in_window
            && !window_occluded(z, self.mouse_logical, self.window_rects_iter());
        let btn = self.mouse_left();
        let press_here = btn.down_edge() && hit;
        // 输入框等文本控件按下时置位 press_claimed：**不建立窗口拖拽基准**
        // （从输入框上拖拽 = 选择文本；窗口改从空白/标题区拖动）。
        let drag_here = press_here && !self.press_claimed;
        // 点击置顶（modal 对话框**不主动置顶**——它已最上，且避免 z 漂移/与浮层冲突）
        if topmost && press_here {
            if self                .win_press_top
                .as_ref()
                .is_none_or(|(_, top_z)| self.cur_win > *top_z)
            {
                self.win_press_top = Some((id.to_owned(), self.cur_win));
            }
        }
        let (active, new_pos) = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let dragging = update_drag(ws, hit, btn);
            if drag_here {
                ws.press_panel = Some(panel_rect.min());
                ws.press_mouse = Some(self.mouse_screen.round());
            } else if press_here {
                // 文本框等子控件按下（选择拖拽优先）：**清除旧拖拽基准**——
                // 否则 `update_drag` 已置 dragging=true，残留的 press_mouse 会被
                // drag_moved 当作基准，算出巨大位移 → 窗口"瞬移"。
                ws.press_panel = None;
                ws.press_mouse = None;
            }
            // 拖拽需实际位移（≥ DRAG_ACTIVATE_PX）且**有本帧基准**才激活：纯点击不拖拽，
            // 窗口内子控件（按钮/勾选/输入框）正常响应（见 hit_abs 抑制条件）。
            let active = dragging
                && ws.press_mouse.is_some()
                && drag_moved(self.mouse_screen.round(), ws.press_mouse);
            let np = if active {
                let pp = ws.press_panel.unwrap_or(origin);
                let pm = ws.press_mouse.unwrap_or(self.mouse_screen);
                let d = (self.mouse_screen - pm).round();
                pp + d / self.scale
            } else {
                origin
            };
            (active, np)
        };
        if active {
            self.drag_panel = Some(id.to_owned());
            if self.state.panel_pos.get(id) != Some(&new_pos) {
                self.state.panel_pos.insert(id.to_owned(), new_pos);
            }
        } else if self.drag_panel.as_deref() == Some(id) {
            self.drag_panel = None;
        }
        if press_here || active {
            // 按下窗口（或拖拽中）都算"已响应按下"——避免空白点击清焦点
            self.any_pressed = true;
        }
        // 窗口拖动激活 → 强制普通 Arrow（UI_NEEDS：移动窗口时无需 <->，是 BUG）。
        if active {
            self.cursor_window_drag = true;
        }
        let display_pos = new_pos;
        // 记录窗口原点（顶点局部化基准；win=0 非窗口默认 (0,0)）与窗口 id（缓存 key）
        self.win_origins.insert(z, display_pos);
        self.win_ids.insert(z, id.to_owned());
        // 窗口矩形入遮挡判定缓存（跨帧；finish 末尾只保留本帧录制的窗口）。
        // ⚠ 存**绝对**坐标：嵌套窗口 / 下拉浮层在容器内时 `display_pos` 是容器
        // 局部坐标，须加容器绝对原点（`saved_base`）——否则遮挡判定用绝对鼠标
        // 比局部矩形恒不命中，浮层背后的控件仍响应 hover/click（"下拉菜单选项
        // 悬停时背后按钮一起 Hover"）。
        self.state
            .window_rects
            .insert(
                z,
                Rect::new(
                    saved_base.x + display_pos.x,
                    saved_base.y + display_pos.y,
                    size.x,
                    size.y,
                ),
            );
        // 严格裁剪（`window_at_strict`）：窗口内容**强制裁剪**到窗口矩形——结算后
        // 统一改写本窗口命令的裁剪层（录制期窗口尺寸未知，背景/子控件命令都覆盖；
        // 命中裁剪由窗口遮挡机制负责）。默认窗口为 Expand 语义（不裁剪）。
        if strict {
            let win_abs = Rect::new(
                saved_base.x + display_pos.x,
                saved_base.y + display_pos.y,
                size.x,
                size.y,
            );
            for d in &mut self.queue[start..] {
                d.clip = clip_for_view(saved_clip, win_abs, ViewMode::Clip);
            }
        }
        // 背景 + 边框（win = z，画在窗口子控件之下；radius > 0 走圆角双层矩形）
        let style = self.theme.panel.clone();
        let bg_rect = Rect::new(0.0, 0.0, size.x, size.y);
        self.push_panel_like(bg_rect, style.bg, style.border, style.border_w, style.radius, 0);
        // 固定宽窗口：右下角缩放柄图案（3 条递减小斜杠；窗口局部坐标，随窗口平移）
        if width.is_some() {
            let grip = style.border;
            for k in 0..3 {
                let o = 5.0 * (k as f32 + 1.0);
                self.push_solid_rect(
                    Rect::new(size.x - o, size.y - o, 4.0, 4.0),
                    grip,
                );
            }
        }
        for d in &mut self.queue[start..] {
            d.translate(display_pos);
        }
        self.cur_win = saved_win;
        size
    }

    /// **模态对话框**：全屏半透明遮罩（[`Theme::modal`](crate::style::Theme::modal)
    /// 的颜色/尺寸，默认全屏半透明黑）置于最上层，背后一切交互被遮挡（遮罩矩形
    /// 经窗口遮挡判定阻断，含顶层 win=0 内容）；对话框（可拖拽）浮于遮罩之上。
    /// `pos` 为对话框左上角（逻辑，相对当前容器原点；按顶层使用）。`Esc` 关闭由
    /// 调用方处理（见 [`crate::builtin::FontModal`]）。
    ///
    /// ⚠ **应在帧末（其它窗口之后）调用**：遮罩/对话框 z 每帧重写为"当前最大+1/+2"，
    /// 但本帧**之后**录制的窗口会分到更高 z 并绘制在其上——先录制窗口、最后录制
    /// modal，才能保证 Modal 恒在最上。
    pub fn modal_at(
        &mut self,
        id: &str,
        pos: Vec2,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        self.modal_impl(id, pos, None, f)
    }

    /// **固定宽模态对话框**（宽度指定、高度自动，如同 egui）。
    pub fn modal_at_w(
        &mut self,
        id: &str,
        pos: Vec2,
        width: f32,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        self.modal_impl(id, pos, Some(width), f)
    }

    /// 模态对话框公共实现：遮罩窗口（低 z）+ 对话框窗口（高 z）。
    fn modal_impl(
        &mut self,
        id: &str,
        pos: Vec2,
        width: Option<f32>,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        // 遮罩 z = 当前最大 + 1（普通窗口之上）；对话框 z 再 +1（window_impl 自动
        // 分配）。**每帧强制重写**（不是 or_insert）——Modal 打开期间恒在最上，
        // 不会被其它后置顶的窗口盖住；点击对话框/背景不触发额外 z 提升。
        let max_z = self
            .state
            .window_z
            .values()
            .copied()
            .filter(|&z| z < WIN_TOPMOST)
            .max()
            .unwrap_or(0);
        let dim_id = format!("{id}::dim");
        let z_dim = max_z + 1;
        // 遮罩与对话框 z **每帧强制重写**（不是 or_insert）——Modal 打开期间恒在最上：
        // ① 遮罩不被后置顶的窗口盖住；② 对话框不被自家遮罩盖住（or_insert 会保留
        // 旧 z，其它窗口置顶后遮罩 max+1 反超对话框旧 z → 字体窗口跑到遮罩后面）。
        self.state.window_z.insert(dim_id.clone(), z_dim);
        self.state.window_z.insert(id.to_owned(), z_dim + 1);
        // 遮罩矩形（**绝对逻辑坐标**；默认全屏 = 窗口客户区物理 ÷ scale，
        // 可被 [`Theme::modal`] 的 `size` 覆盖）。
        let (mw, mh) = match self.theme.modal.size {
            Some(s) => (s.x, s.y),
            None => {
                let s = self.window.inner_size();
                (s.width as f32 / self.scale, s.height as f32 / self.scale)
            }
        };
        let dim_rect = Rect::new(0.0, 0.0, mw, mh);
        // 遮罩录制（win = z_dim；按顶层使用，局部 == 绝对）。
        let saved_win = std::mem::replace(&mut self.cur_win, z_dim);
        let seq = self.next_seq();
        let depth = self.depth;
        self.queue.push(UiDraw {
            depth,
            seq,
            win: z_dim,
            elem: 0,
            rect: dim_rect,
            clip: self.clip,
            kind: DrawKind::Solid(self.theme.modal.dim),
        });
        // 遮罩窗口矩形（遮挡判定用；绝对）。
        self.state.window_rects.insert(z_dim, dim_rect);
        self.win_ids.insert(z_dim, dim_id);
        self.win_origins.insert(z_dim, Vec2::ZERO);
        self.cur_win = saved_win;
        // 对话框窗口（window_impl 按 max+1 分配 → z = z_dim + 1，浮于遮罩之上；
        // **不主动置顶**——点击对话框/背景不触发 z 提升）。
        self.window_impl(id, pos, width, false, false, f)
    }

    /// pack 容器：按 `side` 堆叠，尺寸自动。
    pub fn pack_at(
        &mut self,
        pos: Vec2,
        side: PackSide,
        f: impl FnOnce(&mut Pack<'_, '_>),
    ) -> Vec2 {
        let gap = self.theme.gap;
        self.container(pos, Frame::new_stack(side, gap, 0.0), |ctx| {
            let mut p = Pack { ui: ctx.ui };
            f(&mut p);
        })
        .0
    }

    /// 当前容器**下一子项**的最小尺寸约束（`0` = 该轴不约束；一次性）。
    /// 容器内便捷方法：`p.min_size(120.0, 0.0)`（见 [`crate::Ui`] 文档 / 示例）。
    pub fn set_next_min(&mut self, min: Vec2) {
        self.frames
            .last_mut()
            .expect("min_size 需在容器内调用（顶层请用 *_at 定位）")
            .set_next_min(min);
    }

    /// 当前容器**下一子项**的最大尺寸约束（`0` = 该轴不约束；一次性）。
    pub fn set_next_max(&mut self, max: Vec2) {
        self.frames
            .last_mut()
            .expect("max_size 需在容器内调用（顶层请用 *_at 定位）")
            .set_next_max(max);
    }

    /// **flex 容器**：固定总高 `total_h`（逻辑像素），子项按 `weights` 权重**等分高度**
    /// （扣掉子项间距后按权重分配；权重全 0 时子项高为 0），回调按索引布局——
    /// 同帧精确分配，无需跨帧缓存；返回 `(最大子项宽, total_h)`。
    ///
    /// 子项内可放任意控件（`f.label` / `f.button` 等占光标，高度被强制为分配值）；
    /// 内容超高时**溢出可见**（需要滚动时在子项内嵌 [`Self::scroll_at`]）。
    /// `pos` 相对当前容器内容原点（顶层即屏幕原点），不占父容器光标。
    pub fn flex_at<F>(
        &mut self,
        pos: Vec2,
        total_h: f32,
        weights: &[u32],
        mut f: F,
    ) -> Vec2
    where
        F: FnMut(&mut FlexCtx<'_, '_>, usize),
    {
        let gap = self.theme.gap;
        let start = self.queue.len();
        let saved_base = self.abs_base;
        self.abs_base = saved_base + pos;
        let mut frame = Frame::new_stack(PackSide::Top, gap, 0.0);
        frame.set_fixed_h(total_h);
        self.frames.push(frame);
        self.depth += 1;
        let sum: u32 = weights.iter().sum();
        let gaps = gap * weights.len().saturating_sub(1) as f32;
        let usable = (total_h - gaps).max(0.0);
        {
            let mut fc = FlexCtx { ui: self };
            for (i, &w) in weights.iter().enumerate() {
                let h = if sum > 0 { usable * w as f32 / sum as f32 } else { 0.0 };
                fc.ui.frames.last_mut().expect("flex frame").force_next_h(h);
                f(&mut fc, i);
            }
        }
        let frame = self.frames.pop().expect("flex frame");
        let size = frame.settle_size();
        self.depth -= 1;
        self.abs_base = saved_base;
        for d in &mut self.queue[start..] {
            d.translate(pos);
        }
        size
    }

    /// grid 容器：`cols` 列均匀网格，单元格尺寸跨帧缓存（`id` 须稳定）。
    pub fn grid_at(
        &mut self,
        pos: Vec2,
        cols: usize,
        id: &str,
        f: impl FnOnce(&mut Grid<'_, '_>),
    ) -> Vec2 {
        assert!(cols > 0, "grid cols must be > 0");
        let cell = self.state.grid_cells.get(id).copied().unwrap_or(Vec2::ZERO);
        let (size, max_child) = self.container(pos, Frame::new_grid(cols, cell, 0.0), |ctx| {
            let mut g = Grid { ui: ctx.ui };
            f(&mut g);
        });
        // 回写单元格缓存：**内容变化时同时允许扩大与缩小**（未达到 max 的控件按
        // 内容自动改大小——如背包格子文字变长/变短；内容不变时布局依旧跨帧稳定）。
        self.state.grid_cells.insert(id.to_owned(), max_child);
        size
    }

    // ── 提交 ─────────────────────────────────────────────────

    /// 排序并提交全部绘制命令；随后清空帧状态（下次 `begin` 复用）。
    ///
    /// **命令排序键**：`(win, depth, elem, kind_group, seq)`
    /// - `win`：窗口 z 序（焦点窗口靠后 → 最上层）；
    /// - `depth`：容器嵌套深度；
    /// - `elem`：**元素序**（控件开始录制时的序号）——元素间按录制顺序，
    ///   重叠时后录元素覆盖先录元素（层级正确）；
    /// - `kind_group`：**元素内**"背景/图形（0）先于文字（1）"——文字不被
    ///   自身图形覆盖（[`crate::draw::DrawKind::group`]）；
    /// - `seq`：同元素内同类命令保持录制顺序。
    ///
    /// **提交方式**（不使用 Sprite）：全部图元（背景 / 控件背景 / 文字）转为
    /// **四边形顶点**（[`Render2D::add_quads`]），按 `(窗口, 纹理)` 分组后
    /// **由 UI 自行决定提交顺序**（不依赖 Render2D 排序）：
    /// `(win 升序, 白纹理图形组 → 字形文字组, 纹理 uid)`——
    /// 1. 非窗口内容（`win=0`）最底，窗口按 z 从下到上（`layer = base + z`）；
    /// 2. 同一窗口内**"背景/图形 → 文字"严格成立**（白纹理组先于字形图集组），
    ///    跨帧稳定、与 Render2D 任意排序模式结果一致。
    ///
    /// **UI 的 Render2D 必须关闭排序**（`set_sorting(false)`，完全按提交顺序绘制）：
    /// UI 自行管理绘制顺序，排序键 `(win, depth, elem, group, seq)` 依赖**提交顺序**
    /// 生效（图形组在文字组之前提交）。⚠ `set_sorting(true)`（`SortMode::LayerAndStates`）
    /// 会在同一 layer 内按 `(rstates, texture_uid)` 重排——字形图集页先于程序化纹理页
    /// （圆角/渐变）注册，重排后**圆角/渐变图形会盖住文字**；`set_layer_sort(true)`
    /// （`SortMode::LayerOnly`，稳定按 layer 排序）可接受（同层保持提交顺序）。
    /// 提交本帧 UI 到渲染器。**视口与渲染器在此延迟传入**（`begin` 时不需要）——
    /// 录制阶段可完全独立于绘制资源；`viewport`（大小 + 位置）提供屏幕固定变换，
    /// `r2d` 接收四边形。UI 不需要相机（恒为 identity：不旋转/缩放），仅需视口。
    pub fn finish(&mut self, viewport: &Viewport, r2d: &mut Render2D) {
        let t_finish = Instant::now();
        // 空白点击清焦点（本帧按下且无控件响应）
        if self.mouse_left().down_edge() && !self.any_pressed && self.state.focused.is_some() {
            self.state.focused = None;
        }
        // 清除一次性边沿
        for ws in self.state.widgets.values_mut() {
            clear_frame_flags(ws);
        }
        // 窗口按下裁决：重叠点击只让**最上层**窗口获得拖拽与置顶（见 window_at）
        self.resolve_win_press();
        // 键盘导航：Tab / Shift+Tab / 方向键遍历焦点链、Esc 关浮层/失焦、焦点描边。
        self.handle_focus_keys();
        // 排序（窗口 z → 深度 → 元素序 → 元素内图形/文字 → 命令序）：
        // 元素间按录制顺序（后录元素覆盖先录元素，重叠层级正确），
        // 元素内"背景/图形 → 文字"（DrawKind::group）——文字不被自身图形覆盖。
        let t_sort = Instant::now();
        self.queue
            .sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
        let cmd_count = self.queue.len() as u32;
        let queue = std::mem::take(&mut self.queue);
        // **WHITE 基础纹理优先取字形图集页**（`Text::white_region`，1×1 clamp_margin）：
        // 实心填充（Solid / 边框 / 光标）与字形**同页同纹理** → 同窗口内"图形组 → 文字组"
        // 相邻且同纹理，Render2D 合批为单个 draw call，省去图形↔文字的纹理状态切换。
        // 兜底：渲染器自带白纹理（整纹理 UV）。
        let (white_uid, white_uv_tl, white_uv_wh) = match self.text.white_region() {
            Some(r) => {
                let inv = match TEXTURES.get(r.page_uid) {
                    Some(t) => 1.0 / t.width as f32,
                    None => 1.0,
                };
                let tl = Vec2::new(r.tl_px.0 as f32, r.tl_px.1 as f32) * inv;
                let wh = Vec2::new(r.wh_px.0 as f32, r.wh_px.1 as f32) * inv;
                (r.page_uid, tl, wh)
            }
            None => {
                let uid = r2d.white_texture().uid;
                (uid, Vec2::ZERO, Vec2::ONE)
            }
        };
        // 按窗口分组：非窗口（win=0）每帧重建；窗口按**内容签名**缓存局部顶点，
        // 内容不变时复用（移动窗口只改变换，顶点不重建）。
        let mut groups: std::collections::HashMap<u32, Vec<UiDraw>> =
            std::collections::HashMap::new();
        for d in queue {
            groups.entry(d.win).or_default().push(d);
        }
        let mut wins: Vec<u32> = groups.keys().copied().collect();
        wins.sort_unstable();
        let sort_us = t_sort.elapsed().as_secs_f64() * 1e6;
        // —— 计时累加器（本帧各阶段 µs 统计，finish 末尾写入 UiState.stats） ——
        let mut sig_us = 0.0f64;
        let mut collect_us = 0.0f64;
        let mut clone_us = 0.0f64;
        let mut cache_hits = 0u32;
        let mut cache_misses = 0u32;
        let mut win_count = 0u32;
        let mut quads = QuadCollector::new(white_uid, white_uv_tl, white_uv_wh); // 非窗口 + 缓存 miss 重建
        // 缓存命中：克隆局部顶点到提交列表（简单可靠——零拷贝两阶段读取在窗口 z /
        // id 映射变化时有"整窗不提交"的竞态风险，曾导致拖动/内容变化时窗口
        // "消失与显示交替"闪烁）。
        let mut cached: Vec<(u32, u32, u8, u64, Vec<VertexP3U2C4>)> = Vec::new();
        for win in wins {
            let cmds = groups.remove(&win).expect("group exists");
            // debug_layout：每帧重建（布局描边是调试视图，跳过窗口顶点缓存）。
            if win == 0 || self.debug_layout {
                self.collect_cmds(&mut quads, win, &cmds, viewport, r2d);
                continue;
            }
            let Some(id) = self.win_ids.get(&win).cloned() else {
                self.collect_cmds(&mut quads, win, &cmds, viewport, r2d);
                continue;
            };
            win_count += 1;
            // 内容签名：窗口命令的 (kind, rect, color, 文本…) **全量哈希**。
            // ⚠ 不做"轻量摘要快速路径"：摘要若漏字段（如 hover/click 变色的颜色位、
            // 边框宽、圆角、对齐、光标/选择），会在内容变化时误判"未变" → 复用陈旧
            // 顶点 → 窗口内 hover/click 效果不刷新（曾致下拉框、背包、窗口 A/B 的
            // Hover/Click 失效——win=0 顶层内容每帧重建所以正常）。全量签名每帧
            // ~15µs（O2 下），正确性优先。
            let t_sig = Instant::now();
            let sig = {
                use std::hash::Hasher;
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for d in &cmds {
                    self.cmd_sig(&mut h, d);
                }
                h.finish()
            };
            sig_us += t_sig.elapsed().as_secs_f64() * 1e6;
            // 命中缓存：直接用缓存的局部顶点（分组复制到提交列表），跳过重建
            {
                let entry = self
                    .state
                    .window_quads
                    .entry(id.clone())
                    .or_insert((0, Vec::new()));
                if entry.0 == sig {
                    cache_hits += 1;
                    let t_clone = Instant::now();
                    for (elem, g, tex, verts) in &entry.1 {
                        cached.push((win, *elem, *g, *tex, verts.clone()));
                    }
                    clone_us += t_clone.elapsed().as_secs_f64() * 1e6;
                    continue;
                }
            }
            cache_misses += 1;
            // 未命中：收集该窗口命令为局部顶点，写入缓存
            let t_collect = Instant::now();
            let mut q = QuadCollector::new(white_uid, white_uv_tl, white_uv_wh);
            self.collect_cmds(&mut q, win, &cmds, viewport, r2d);
            collect_us += t_collect.elapsed().as_secs_f64() * 1e6;
            let mut grp: Vec<(u32, u8, u64, Vec<VertexP3U2C4>)> = Vec::new();
            for ((_, elem, g, tex), verts) in q.quads {
                // 缓存存克隆、本帧提交原顶点（各一份）——**重建帧窗口照常绘制**：
                // 否则窗口内容一变就"消失 1 帧"（缓存冷启动 / 拖动中 hover、光标
                // 闪烁、滚动等逐帧变化 → 窗口每帧重建、每帧消失 → "消失与显示
                // 瞬间交替"闪烁）。
                grp.push((elem, g, tex, verts.clone()));
                cached.push((win, elem, g, tex, verts));
            }
            // 缓存组顺序与提交顺序一致：控件序 → 元素内图形 → 文字 → 纹理——跨帧稳定。
            grp.sort_by_key(|&(elem, g, tex, _)| (elem, g, tex));
            self.state.window_quads.insert(id, (sig, grp));
        }
        // 提交：**UI 自行管理绘制顺序**，UI 的 Render2D 必须 `set_sorting(false)`
        // （关闭排序，完全按提交顺序绘制）；`set_layer_sort(true)`（LayerOnly，稳定排序）
        // 同层保持提交顺序也可。⚠ 不要用 `set_sorting(true)`（LayerAndStates）：
        // 它按 `(rstates, texture_uid)` 重排，字形图集页 uid < 程序化纹理页 uid →
        // 圆角/渐变会被排在文字之后绘制，盖住文字。
        //
        // 统一排序键 `(win, 元素序, 图形/文字组, 纹理 uid)`，每 (窗口, 元素, 组, 纹理)
        // 一次 add_quads：
        // 1. **win 升序**：非窗口内容（win=0，layer = base）最底，窗口按 z 从下到上
        //    （layer = base + z）——后提交的窗口覆盖先提交的；
        // 2. **窗口内按元素序（控件录制序）**：后录控件覆盖先录控件（重叠层级正确）；
        //    **元素内"背景/图形 → 文字"**（`g`）——文字不被自身图形覆盖。
        //    ⚠ 不可按 (win, g, tex) 提交：那会把所有背景排到所有文字之前，后录控件的
        //    背景会被先录控件的文字盖住（白纹理合批后语义仍错）。
        //
        // transform = 屏幕固定变换（窗口原点物理像素）→ 局部顶点映射到世界。
        let t_submit = Instant::now();
        let layer_base = self.base_layer;
        let mut ordered: Vec<(u32, u32, u8, u64, Vec<VertexP3U2C4>)> =
            Vec::with_capacity(cached.len() + quads.quads.len());
        // mem::take：只移走内容四边形，`quads.debug`（调试叠加）留待最后提交。
        for ((win, elem, g, tex_uid), verts) in std::mem::take(&mut quads.quads) {
            ordered.push((win, elem, g, tex_uid, verts));
        }
        ordered.extend(cached);
        ordered.sort_by_key(|&(win, elem, g, tex_uid, _)| (win, elem, g, tex_uid));
        for (win, _elem, _g, tex_uid, verts) in ordered {
            let Some(tex) = TEXTURES.get(tex_uid) else {
                continue;
            };
            let anchor_px = self
                .win_origins
                .get(&win)
                .copied()
                .unwrap_or(Vec2::ZERO)
                * self.scale;
            let tf = screen_fixed_tf(viewport, anchor_px);
            let layer = Layer::from(layer_base + win as f64 * 1.0);
            r2d.add_quads(&verts, tf, layer, &tex);
            // MeshBuilder Drop 即提交 ✓
        }

        // ── Debug 叠加（DebugDraw / debug_layout 描边）────────────
        // 在**全部 UI 内容之后**提交（同 layer 后提交 → 恒覆盖在最上）：
        // 1. 收集 `debug_queue`（[`Self::debug_line`] 等屏幕空间调试图元，
        //    坐标 = 绝对逻辑像素 → 窗口局部物理四边形）；
        // 2. 与 `collect_cmds` 期间产生的布局描边（`quads.debug`）合并；
        // 3. 按 win 分组、白纹理、屏幕固定变换提交（不进窗口缓存）。
        {
            let mut debug_groups: std::collections::HashMap<u32, Vec<UiDraw>> =
                std::collections::HashMap::new();
            for d in self.debug_queue.drain(..) {
                debug_groups.entry(d.win).or_default().push(d);
            }
            let mut dwins: Vec<u32> = debug_groups.keys().copied().collect();
            dwins.sort_unstable();
            for win in dwins {
                let cmds = debug_groups.remove(&win).expect("group exists");
                self.collect_cmds(&mut quads, win, &cmds, viewport, r2d);
            }
        }
        let mut dwins: Vec<u32> = quads.debug.keys().copied().collect();
        dwins.sort_unstable();
        for win in dwins {
            let verts = quads.debug.remove(&win).expect("debug group exists");
            let Some(tex) = TEXTURES.get(white_uid) else {
                continue;
            };
            let anchor_px = self
                .win_origins
                .get(&win)
                .copied()
                .unwrap_or(Vec2::ZERO)
                * self.scale;
            let tf = screen_fixed_tf(viewport, anchor_px);
            let layer = Layer::from(layer_base + win as f64 * 1.0);
            r2d.add_quads(&verts, tf, layer, &tex);
        }
        // 记录 IME 组合状态（供下一帧退格判定，见 text_input_at）
        self.state.ime_composing =
            self.keyboard.get_ime_preedit().is_some_and(|p| !p.is_empty());
        // 窗口矩形遮挡缓存只保留**本帧录制过**的窗口（z 变化 / 窗口销毁的旧条目随帧清理）。
        // 性能统计（本帧各阶段 µs；写入 UiState.stats，示例/诊断读取）
        self.state.stats = UiStats {
            frame: self.state.stats.frame.wrapping_add(1),
            cmd_count,
            win_count,
            cache_hits,
            cache_misses,
            sort_us,
            sig_us,
            collect_us,
            clone_us,
            submit_us: t_submit.elapsed().as_secs_f64() * 1e6,
            finish_us: t_finish.elapsed().as_secs_f64() * 1e6,
            ui_frame_us: self.frame_t0.elapsed().as_secs_f64() * 1e6,
        };
        // 系统光标图案（优先级：窗口拖拽(Arrow) > 内置拖拽抓握 > 控件作者自定义 >
        // 文本输入 > 可拖拽悬停 > 默认）。窗体悬停/拖动保持普通 Arrow（UI_NEEDS）；
        // 移动窗口时即使悬停数字手柄/输入框也强制 Arrow（修复拖动中 <-> 的 BUG）。
        // **抑制**：本帧没有任何 UI 光标意图（未悬停任何 UI 内容）时不主动设置——
        // 保留应用自定义光标（如游戏准星）；仅当上一帧设过时清一次回 Default。
        let intent = self.cursor_text
            || self.cursor_grab
            || self.cursor_grabbing
            || self.cursor_window_drag
            || self.cursor_custom.is_some();
        let icon = if self.cursor_window_drag {
            winit::window::CursorIcon::Default
        } else if self.cursor_grabbing {
            winit::window::CursorIcon::Grabbing
        } else if let Some(icon) = self.cursor_custom {
            icon
        } else if self.cursor_text {
            winit::window::CursorIcon::Text
        } else if self.cursor_grab {
            winit::window::CursorIcon::Grab
        } else {
            winit::window::CursorIcon::Default
        };
        if intent {
            self.window.set_cursor(icon);
            self.state.cursor_was_set = true;
        } else if self.state.cursor_was_set {
            // 无 UI 光标意图但上一帧设过 → 清一次回 Default（避免残留 I 型等）
            self.window.set_cursor(winit::window::CursorIcon::Default);
            self.state.cursor_was_set = false;
        }
        self.cursor_text = false;
        self.cursor_grab = false;
        self.cursor_grabbing = false;
        self.cursor_window_drag = false;
        self.cursor_custom = None;
        self.state.window_rects.retain(|z, _| self.win_origins.contains_key(z));
        self.depth = 0;
        self.seq = 0;
        self.cur_win = 0;
        self.any_pressed = false;
        self.drag_panel = None;
        self.win_press_top = None;
        self.win_origins.clear();
        self.win_ids.clear();
        self.frames.clear();
        self.focusables.clear();
        self.press_claimed = false;
    }

    /// 把一组命令收集为四边形顶点（**相对窗口原点的局部物理像素**；
    /// `win` 决定局部化基准，非窗口 win=0 基准 (0,0)）。
    ///
    /// `debug_layout` 开启时，每个命令的矩形同时向 `quads.debug` 追加**青色描边**
    /// （调试 rjw_ui 自身的布局 / 命中区域）；`DrawKind::Debug` 命令（[`Self::debug_line`]
    /// 等屏幕空间调试图元）则只写入 `quads.debug`（覆盖在 UI 内容之上）。
    fn collect_cmds(
        &mut self,
        quads: &mut QuadCollector,
        win: u32,
        cmds: &[UiDraw],
        viewport: &Viewport,
        r2d: &Render2D,
    ) {
        let anchor_px = self
            .win_origins
            .get(&win)
            .copied()
            .unwrap_or(Vec2::ZERO)
            * self.scale;
        let dbg = if self.debug_layout {
            // debug_layout 描边样式：读 Theme::debug（Copy 值先取出，
            // 避免与循环内 `&mut self` 调用（draw_text_quads）的借用冲突）。
            Some((self.theme.debug.layout_outline, self.theme.debug.layout_outline_width))
        } else {
            None
        };
        for d in cmds {
            // 当前元素序：push 方法按其分组（控件级提交顺序——见 QuadCollector）。
            quads.cur_elem = d.elem;
            // 裁剪区（绝对物理；内容已随容器平移成绝对逻辑坐标）。
            let clip_abs = d.clip.map(|c| snap_rect(&self.phys_rect(&c)));
            match &d.kind {
                DrawKind::Solid(color) => {
                    if d.rect.w > 0.0 && d.rect.h > 0.0 {
                        let pr = snap_rect(&self.phys_rect(&d.rect));
                        if let Some(r) = clipped(pr, clip_abs) {
                            quads.push_white(
                                win,
                                Rect::new(r.x - anchor_px.x, r.y - anchor_px.y, r.w, r.h),
                                *color,
                            );
                            debug_layout_outline(quads, win, anchor_px, r, dbg);
                        }
                    }
                }
                DrawKind::RoundedRect { color, radius } => {
                    let pr = snap_rect(&self.phys_rect(&d.rect));
                    if let Some(local) = clipped(pr, clip_abs).map(|r| {
                        Rect::new(r.x - anchor_px.x, r.y - anchor_px.y, r.w, r.h)
                    }) {
                        if local.w > 0.0 && local.h > 0.0 {
                            // 半径转物理像素并 clamp（与生成纹理时的 clamp 一致）。
                            let r = (*radius * self.scale)
                                .clamp(0.0, crate::proc::ROUNDED_TEX_SIZE as f32 * 0.5 - 1.0)
                                .max(0.0);
                            if r <= 0.0 {
                                quads.push_white(win, local, *color);
                            } else {
                                let device = r2d.device();
                                let queue = r2d.queue();
                                let layout = r2d.tex_bind_group_layout();
                                if let Some((tex_uid, region)) =
                                    self.state.proc.rounded(device, queue, layout, r)
                                {
                                    if let Some(tex) = TEXTURES.get(tex_uid) {
                                        let inv_page = 1.0 / tex.width as f32;
                                        let base = Vec2::new(
                                            region.tl_px.0 as f32,
                                            region.tl_px.1 as f32,
                                        ) * inv_page;
                                        let tex_wh = Vec2::new(
                                            region.wh_px.0 as f32,
                                            region.wh_px.1 as f32,
                                        ) * inv_page;
                                        // 9-patch：四角原样、四边/中心拉伸（任意尺寸圆弧不畸变）。
                                        for (mr, uvtl, uvwh) in crate::proc::rounded_9patch(
                                            local,
                                            r,
                                            crate::proc::ROUNDED_TEX_SIZE,
                                            r,
                                        ) {
                                            if mr.w <= 0.0 || mr.h <= 0.0 {
                                                continue;
                                            }
                                            quads.push_tex_rect(
                                                win,
                                                tex_uid,
                                                base + uvtl * tex_wh,
                                                uvwh * tex_wh,
                                                mr,
                                                *color,
                                            );
                                        }
                                    }
                                } else {
                                    quads.push_white(win, local, *color);
                                }
                            }
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                    }
                }
                DrawKind::Gradient { axis, stops } => {
                    let pr = snap_rect(&self.phys_rect(&d.rect));
                    if let Some(local) = clipped(pr, clip_abs).map(|r| {
                        Rect::new(r.x - anchor_px.x, r.y - anchor_px.y, r.w, r.h)
                    }) {
                        if local.w > 0.0 && local.h > 0.0 {
                            let vertical = matches!(axis, GradientAxis::Vertical);
                            let device = r2d.device();
                            let queue = r2d.queue();
                            let layout = r2d.tex_bind_group_layout();
                            if let Some((tex_uid, region)) =
                                self.state.proc.gradient(device, queue, layout, vertical, stops)
                            {
                                if let Some(tex) = TEXTURES.get(tex_uid) {
                                    let inv_page = 1.0 / tex.width as f32;
                                    let base = Vec2::new(
                                        region.tl_px.0 as f32,
                                        region.tl_px.1 as f32,
                                    ) * inv_page;
                                    let tex_wh = Vec2::new(
                                        region.wh_px.0 as f32,
                                        region.wh_px.1 as f32,
                                    ) * inv_page;
                                    // 整矩形拉伸采样渐变纹理（主轴 64 级已平滑）。
                                    quads.push_tex_rect(
                                        win,
                                        tex_uid,
                                        base,
                                        tex_wh,
                                        local,
                                        Color::WHITE,
                                    );
                                }
                            }
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                    }
                }
                DrawKind::Border { color, width } => {
                    let pr = snap_rect(&self.phys_rect(&d.rect));
                    if let Some(r) = clipped(pr, clip_abs) {
                        let local = Rect::new(r.x - anchor_px.x, r.y - anchor_px.y, r.w, r.h);
                        for br in border_rects(&local, self.phys_f(*width).round()) {
                            if br.w > 0.0 && br.h > 0.0 {
                                quads.push_white(win, br, *color);
                            }
                        }
                        debug_layout_outline(quads, win, anchor_px, pr, dbg);
                    }
                }
                DrawKind::Text {
                    text,
                    size,
                    color,
                    align,
                    valign,
                    family,
                    clip,
                    buf,
                } => {
                    // 外层裁剪（绝对逻辑）→ 相对文本块左上角（与 DrawKind::Text::clip 同空间），
                    // 与命令自带裁剪求交后传给 draw_text_quads。
                    let merged = match d.clip.map(|c| {
                        Rect::new(c.x - d.rect.x, c.y - d.rect.y, c.w, c.h)
                    }) {
                        Some(outer) => match clip {
                            Some(inner) => intersect_rect(&outer, inner),
                            None => Some(outer),
                        },
                        None => *clip,
                    };
                    self.draw_text_quads(
                        quads,
                        win,
                        anchor_px,
                        &d.rect,
                        text,
                        *size,
                        *color,
                        *align,
                        *valign,
                        family.as_deref(),
                        merged,
                        buf.as_ref().map(Arc::clone),
                        viewport,
                    );
                    let pr = snap_rect(&self.phys_rect(&d.rect));
                    debug_layout_outline(quads, win, anchor_px, pr, dbg);
                }
                DrawKind::Caret { color, width } => {
                    let r = Rect::new(d.rect.x, d.rect.y, *width, d.rect.h);
                    let pr = snap_rect(&self.phys_rect(&r));
                    if let Some(rr) = clipped(pr, clip_abs) {
                        if rr.w > 0.0 && rr.h > 0.0 {
                            quads.push_white(
                                win,
                                Rect::new(rr.x - anchor_px.x, rr.y - anchor_px.y, rr.w, rr.h),
                                *color,
                            );
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                    }
                }
                DrawKind::Debug { color, shape } => {
                    // 屏幕空间调试图元：逻辑像素 → 物理像素线段，转窗口局部写入 debug 叠加。
                    for ([a, b], w) in debug_shape_segments(shape, self.scale) {
                        quads.push_debug_line(win, a - anchor_px, b - anchor_px, w, *color);
                    }
                }
            }
        }
    }

    /// 窗口内容签名：命令的 `(kind, rect, color, 文本…)` 哈希（窗口顶点缓存 key 用）。
    /// 忽略 `win/seq`（窗口内固定）；任何影响渲染的内容变化都会改变签名。
    /// 
    /// **包含文本缓存版本号**：`TEXT_LINE_HEIGHT_VERSION` 变化时，窗口缓存自动失效，
    /// 避免新旧行高混用导致布局错乱。
    ///
    /// ⚠ **必须覆盖一切渲染相关字段**（颜色 / 边框宽 / 圆角 / 对齐 / 光标 / 选择 /
    /// 文本内容）——曾用"轻量摘要"跳过它，漏掉颜色位导致 hover/click 变色时
    /// 缓存不失效、窗口内交互效果不刷新（见 [`crate::state::UiState::window_quads`] 文档）。
    fn cmd_sig(&self, h: &mut std::collections::hash_map::DefaultHasher, d: &UiDraw) {
        use std::hash::Hash;
        d.depth.hash(h);
        d.elem.hash(h);
        d.rect.x.to_bits().hash(h);
        d.rect.y.to_bits().hash(h);
        d.rect.w.to_bits().hash(h);
        d.rect.h.to_bits().hash(h);
        match &d.kind {
            DrawKind::Solid(c) => {
                0u8.hash(h);
                color_bits(*c).hash(h);
            }
            DrawKind::RoundedRect { color, radius } => {
                5u8.hash(h);
                color_bits(*color).hash(h);
                radius.to_bits().hash(h);
            }
            DrawKind::Gradient { axis, stops } => {
                6u8.hash(h);
                match axis {
                    GradientAxis::Vertical => 0u8.hash(h),
                    GradientAxis::Horizontal => 1u8.hash(h),
                }
                stops.len().hash(h);
                for (t, c) in stops {
                    t.to_bits().hash(h);
                    color_bits(*c).hash(h);
                }
            }
            DrawKind::Border { color, width } => {
                1u8.hash(h);
                color_bits(*color).hash(h);
                width.to_bits().hash(h);
            }
            DrawKind::Text {
                text,
                size,
                color,
                align,
                valign,
                family,
                clip,
                buf: _,
            } => {
                2u8.hash(h);
                text.hash(h);
                size.to_bits().hash(h);
                color_bits(*color).hash(h);
                (*align as u8).hash(h);
                (*valign as u8).hash(h);
                family.hash(h);
                // 文本缓存版本号影响排版结果，必须包含在签名中
                TEXT_LINE_HEIGHT_VERSION.hash(h);
                if let Some(c) = clip {
                    c.x.to_bits().hash(h);
                    c.y.to_bits().hash(h);
                    c.w.to_bits().hash(h);
                    c.h.to_bits().hash(h);
                }
            }
            DrawKind::Caret { color, width } => {
                3u8.hash(h);
                color_bits(*color).hash(h);
                width.to_bits().hash(h);
            }
            DrawKind::Debug { color, shape } => {
                4u8.hash(h);
                color_bits(*color).hash(h);
                // 形状参数逐字段哈希（DebugShape 未实现 Hash）。
                match shape {
                    DebugShape::Line { a, b, width } => {
                        0u8.hash(h);
                        a.x.to_bits().hash(h);
                        a.y.to_bits().hash(h);
                        b.x.to_bits().hash(h);
                        b.y.to_bits().hash(h);
                        width.to_bits().hash(h);
                    }
                    DebugShape::RectOutline { rect, width } => {
                        1u8.hash(h);
                        rect.x.to_bits().hash(h);
                        rect.y.to_bits().hash(h);
                        rect.w.to_bits().hash(h);
                        rect.h.to_bits().hash(h);
                        width.to_bits().hash(h);
                    }
                    DebugShape::CircleOutline {
                        center,
                        radius,
                        segments,
                        width,
                    } => {
                        2u8.hash(h);
                        center.x.to_bits().hash(h);
                        center.y.to_bits().hash(h);
                        radius.to_bits().hash(h);
                        segments.hash(h);
                        width.to_bits().hash(h);
                    }
                    DebugShape::Cross { center, half, width } => {
                        3u8.hash(h);
                        center.x.to_bits().hash(h);
                        center.y.to_bits().hash(h);
                        half.to_bits().hash(h);
                        width.to_bits().hash(h);
                    }
                    DebugShape::Grid { rect, spacing, width } => {
                        4u8.hash(h);
                        rect.x.to_bits().hash(h);
                        rect.y.to_bits().hash(h);
                        rect.w.to_bits().hash(h);
                        rect.h.to_bits().hash(h);
                        spacing.to_bits().hash(h);
                        width.to_bits().hash(h);
                    }
                }
            }
        }
    }

    /// 窗口按下裁决：本帧若有窗口被按下（重叠区域点击），**只保留最上层窗口**
    /// 的拖拽（其余取消，修复"重叠时同时拖动两个窗口"），且仅最上层窗口置顶。
    fn resolve_win_press(&mut self) {
        let Some((top_id, old_z)) = self.win_press_top.clone() else {
            return;
        };
        // 只保留最高 z 命中窗口的拖拽：按下新窗口时停止**其它窗口**（含本帧未按下
        // 的旧窗口）的拖拽。⚠ 只清**窗口 id**（`win_ids`）的拖拽状态——不能碰控件
        // 自身的拖拽（滑块 / 滚动条 / 窗口缩放柄等），否则窗口内控件的拖拽会被
        // finish 误清（Resize 手柄按下后下一帧即失效）。
        for (wid, ws) in self.state.widgets.iter_mut() {
            if wid != &top_id
                && self.win_ids.values().any(|i| i == wid)
                && ws.dragging
                && !ws.pressed
            {
                ws.dragging = false;
            }
        }
        // 仅最上层命中窗口置顶（z+1；本帧命令仍按旧 z，下一帧生效）。
        // ⚠ 排除置顶哨兵（WIN_TOPMOST）并 saturating：浮层恒顶，真实窗口 z 不会
        // 递增碰撞到哨兵。
        let max_z = self
            .state
            .window_z
            .values()
            .copied()
            .filter(|&z| z < WIN_TOPMOST)
            .max()
            .unwrap_or(0);
        let new_z = max_z.saturating_add(1);
        self.state.window_z.insert(top_id.clone(), new_z);
        // **焦点归属清理**：焦点控件若在**其他窗口**（本次置顶的窗口之外）——
        // 清除焦点。否则旧输入框在窗口被盖住后仍持焦点（点击被遮挡无法再聚焦、
        // 打字落入不可见输入框），表现为"使用其他窗口后文本框失效"。
        // ⚠ 与**置顶前**的 z（`old_z`，win_press_top 记录点击时的窗口 z）比较——
        // 焦点条目本帧以旧 z 录制；拿置顶后的 `new_z` 比会恒不相等 → 点击输入框
        // （其所在窗口同时置顶）焦点被立即清除，窗口内文本框"无法使用"。
        if let Some(fid) = &self.state.focused {
            let fwin = self.focusables.iter().find(|e| e.id == *fid).map(|e| e.win);
            if fwin.is_some_and(|w| w != 0 && w != old_z) {
                self.state.focused = None;
            }
        }
        // 诊断：记录本次按下由哪个窗口接收（重叠点击时"赢家"）。
        self.state.last_press_window = Some((top_id, new_z));
    }

    /// **键盘导航**（`finish` 末尾调用）：
    ///
    /// - **Tab / Shift+Tab / 方向键**：按 `(win, 注册序)` 排序的焦点链遍历
    ///   （[`focus_step`]），更新 `UiState.focused`；焦点控件本帧未录制时自动清除；
    /// - **Esc**：优先收起展开的下拉框，否则取消焦点；
    /// - **焦点描边**：对当前焦点控件画一圈描边（[`crate::style::FocusStyle`]，
    ///   `Theme::focus`；elem 取全局最大 → 画在窗口内容之上，裁剪沿用控件自身）。
    fn handle_focus_keys(&mut self) {
        // 链排序：按 (win, 注册序) 稳定排序（非窗口 0 在前，窗口按 z 从下到上）。
        let mut chain: Vec<&FocusEntry> = self.focusables.iter().collect();
        chain.sort_by_key(|e| e.win);
        // 焦点控件本帧未录制（所在窗口关闭 / 控件移除）→ 清除焦点。
        if let Some(fid) = &self.state.focused {
            if !chain.iter().any(|e| e.id == *fid) {
                self.state.focused = None;
            }
        }
        // 移动：Tab（+1）/ Shift+Tab（-1）/ Down（+1）/ Up（-1）。
        // ⚠ IME 组合中（preedit 非空或上帧在组合）**禁止方向键/Tab 移动焦点**——
        // 中文输入法用 ↑/↓ 切换候选、Enter 上屏，焦点被移走会立刻打断输入（文本框"失效"）。
        let composing = self
            .keyboard
            .get_ime_preedit()
            .is_some_and(|p| !p.is_empty())
            || self.state.ime_composing;
        let shift = self.keyboard.get(KeyCode::ShiftLeft).pressed()
            || self.keyboard.get(KeyCode::ShiftRight).pressed();
        // **文本输入框持有焦点时，↑/↓ 由输入框自身处理**（多行跨视觉行移动光标、
        // 单行无操作）——全局焦点遍历只接管 Tab / Shift+Tab；否则按 ↑/↓ 会把焦点
        // 跳到别的控件（多行文本框内"上下键跳走"的 bug）。
        let focus_is_text = chain
            .iter()
            .find(|e| self.state.focused.as_deref() == Some(e.id.as_str()))
            .is_some_and(|e| e.kind == FocusKind::TextInput);
        let dir: i32 = if composing {
            0
        } else if self.keyboard.get(KeyCode::Tab).down_edge() {
            if shift { -1 } else { 1 }
        } else if self.keyboard.get(KeyCode::ArrowDown).down_edge() && !focus_is_text {
            1
        } else if self.keyboard.get(KeyCode::ArrowUp).down_edge() && !focus_is_text {
            -1
        } else {
            0
        };
        if dir != 0 {
            let next = focus_step(&chain, self.state.focused.as_deref(), dir);
            self.state.focused = next;
        }
        // Esc：优先收起下拉框，否则取消焦点。
        if self.keyboard.get(KeyCode::Escape).down_edge() {
            if self.state.combo_open.is_some() {
                self.state.combo_open = None;
            } else if self.state.focused.is_some() {
                self.state.focused = None;
            }
        }
        // 焦点描边：对当前焦点控件画一圈 Border（elem 全局最大 → 画在窗口内容之上）。
        if let Some(fid) = &self.state.focused {
            let Some(entry) = chain.iter().find(|e| e.id == *fid) else {
                return;
            };
            // 先拷贝字段，结束对 chain 的借用（随后需要 &mut self）。
            let (win, depth, rect, clip) = (entry.win, entry.depth, entry.rect, entry.clip);
            let focus = self.theme.focus.clone();
            let elem = self.seq + 1;
            let seq = self.next_seq();
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect,
                clip,
                kind: DrawKind::Border { color: focus.color, width: focus.width },
            });
        }
    }

    /// 文本 → 字形四边形（收集到 `quads`；按字形图集页纹理分组）。
    ///
    /// **精确裁切**（"半消失"）：字形与裁剪区求交，相交部分生成裁剪后的四边形
    /// （UV 按比例同步缩放）——字形在裁剪线处被**部分绘制**，而非整字形保留/消失
    /// （`rjw_text` 的 `cull` 只做整字形剔除，像素级裁剪在此完成）。
    #[allow(clippy::too_many_arguments)]
    fn draw_text_quads(
        &mut self,
        quads: &mut QuadCollector,
        win: u32,
        anchor_px: Vec2,
        rect: &Rect,
        text: &str,
        size: f32,
        color: Color,
        align: TextAlign,
        valign: TextVAlign,
        family: Option<&str>,
        clip: Option<Rect>,
        buf: Option<Arc<Buffer>>,
        viewport: &Viewport,
    ) {
        if rect.w <= 0.0 || rect.h <= 0.0 || text.is_empty() {
            return;
        }
        // 全部换算物理像素（锚点 / 裁剪区），与屏幕固定变换 1:1 匹配；
        // 排版缓冲：控件自持（输入框）或按需缓存（静态标签）。
        let pr = snap_rect(&self.phys_rect(rect));
        // 矩形锚点（**整数像素**，逐项取整，无小数参与加法）：
        // 水平：左 = 左缘、中 = 左缘 + 半宽取整、右 = 右缘；
        // 垂直：Center = 上缘 + 半高取整，Top = 上缘（TextArea 多行顶对齐）。
        let anchor = Vec2::new(
            match align {
                TextAlign::Left => pr.x,
                TextAlign::Center => pr.x + (pr.w * 0.5).round(),
                TextAlign::Right => pr.x + pr.w,
            },
            match valign {
                TextVAlign::Top => pr.y,
                TextVAlign::Center => pr.y + (pr.h * 0.5).round(),
            },
        );
        // 局部化基准 = **窗口原点**（anchor_px），不是文字锚点：
        // 字形世界坐标 - 窗口原点世界 = 相对窗口原点的局部顶点，
        // 提交时经窗口 transform（screen_fixed_tf(窗口原点)）映射回世界。
        let win_anchor_world = viewport.screen_to_world(anchor_px);
        let buf = match buf {
            Some(b) => b,
            None => self.cache_buffer(text, size, family),
        };
        let mut tr = self.text.render_from(&buf);
        // 垂直定位按**行盒**（行高），而非字形墨迹内容：
        // 以首行行顶（相对文本视觉原点）为参考，使行盒中心对准锚点（矩形垂直中心）。
        // 字形在行盒内按基线排布——矮小写字母（如 "a"，无 descender）落在基线上，
        // 不再因"以墨迹顶为参考"（旧实现把 `[视觉原点, 视觉原点+行高]` 当块居中，
        // 行盒整体上移约一个 top-bearing）而浮在行盒上部偏上显示。
        let content = tr.content_size();
        // `l.top_left.y` 为**整数**（rjw_text 收集期已对行顶取整）。
        let first_line_top = match tr.lines().first() {
            Some(l) => l.top_left.y,
            None => 0.0,
        };
        // **整数加法不变量**：`block_tl = anchor + off` 的两侧均为整数——
        // 锚点（上方逐项取整）、`text_block_offset`（content / first_line_top 均为
        // 整数，内部只有整数加减与边界 `round`）。0.5px 小数（居中奇数宽 / 行盒偏移）
        // 在 `round` 边界被一次性消化，不会流入加法链 → 无误差累加；
        // 字形四边形角点 = block_tl + 整数字形偏移 = 整数屏幕像素 → 采样精确落在
        // 纹素中心，消除 1:1 图集双线性采样的亚像素模糊。
        let block_tl = anchor + text_block_offset(align, valign, content, first_line_top);
        let tf = screen_fixed_tf(viewport, block_tl);
        tr.origin(Vec2::ZERO).transform(tf).color(color);
        // 裁剪区：相对命令矩形（`merged`，逻辑）→ **窗口局部**。
        // ⚠ 相对基准是命令矩形物理 `pr`，**不是**文本块原点 `block_tl`：
        // `merged` 相对 `d.rect`，而字形窗口局部坐标以 `anchor_px` 为原点——
        // clip 窗口局部 = merged×scale + (pr - anchor_px)。用 block_tl 会整体错位
        // (block_tl - pr)（Top 对齐 / 多行时垂直偏差数像素）→ 滚动后文本不消失/错位。
        let clip_local: Option<Rect> = clip.map(|c| {
            let pc = snap_rect(&Rect::new(
                c.x * self.scale,
                c.y * self.scale,
                c.w * self.scale,
                c.h * self.scale,
            ));
            Rect::new(
                pc.x + pr.x - anchor_px.x,
                pc.y + pr.y - anchor_px.y,
                pc.w,
                pc.h,
            )
        });
        // 像素级裁剪完全由下方逐字形求交完成（完全在外 → 剔除；部分相交 → "半消失"）。
        // 不再使用 rjw_text 的 clip/cull（其坐标系相对字形 top_left，语义易错位）。
        let page_size = tr.page_size();
        let inv_page = 1.0 / page_size;
        let ca: [f32; 4] = color.into();
        tr.draw_with(|_m, _line, region, transform| {
            // 字形精灵（轴对齐四边形）：四角经 transform 到世界坐标，再转窗口局部 + 图集 UV
            let w = region.wh_px.0 as f32;
            let h = region.wh_px.1 as f32;
            let tl = transform.transform_point(Vec2::new(0.0, 0.0)) - win_anchor_world;
            let tr_p = transform.transform_point(Vec2::new(w, 0.0)) - win_anchor_world;
            let bl = transform.transform_point(Vec2::new(0.0, h)) - win_anchor_world;
            let uv_tl = Vec2::new(
                region.tl_px.0 as f32 * inv_page,
                region.tl_px.1 as f32 * inv_page,
            );
            let uv_wh = Vec2::new(w * inv_page, h * inv_page);
            // 字形窗口局部 AABB（轴对齐；屏幕固定变换下无旋转）。
            let gx = tl.x;
            let gy = tl.y;
            let gw = tr_p.x - tl.x;
            let gh = bl.y - tl.y;
            let (qx0, qy0, qx1, qy1) = match &clip_local {
                Some(c) => {
                    // 与裁剪区求交：无交集 → 整字形剔除（"完全消失"）；
                    // 部分相交 → 生成裁剪后四边形（"半消失"，UV 按比例缩放）。
                    let ix0 = gx.max(c.x);
                    let iy0 = gy.max(c.y);
                    let ix1 = (gx + gw).min(c.x + c.w);
                    let iy1 = (gy + gh).min(c.y + c.h);
                    if ix1 <= ix0 || iy1 <= iy0 {
                        return;
                    }
                    (ix0, iy0, ix1, iy1)
                }
                None => (gx, gy, gx + gw, gy + gh),
            };
            let nw = qx1 - qx0;
            let nh = qy1 - qy0;
            let u0 = uv_tl.x + (qx0 - gx) / gw * uv_wh.x;
            let v0 = uv_tl.y + (qy0 - gy) / gh * uv_wh.y;
            let u1 = u0 + nw / gw * uv_wh.x;
            let v1 = v0 + nh / gh * uv_wh.y;
            let quad = [
                vertex_p3u2c4(Vec2::new(qx0, qy0), [u0, v0], ca),
                vertex_p3u2c4(Vec2::new(qx1, qy0), [u1, v0], ca),
                vertex_p3u2c4(Vec2::new(qx0, qy1), [u0, v1], ca),
                vertex_p3u2c4(Vec2::new(qx1, qy1), [u1, v1], ca),
            ];
            quads.push_tex_quad(win, region.page_uid, quad);
        });
    }
}

// ─── 四边形收集器（QuadVertices） ──────────────────────────────

/// 分组维度（提交排序用，见 [`submit_group_key`]）：
/// - `0` = 图形（Solid / RoundedRect / Gradient / Border / Caret——白纹理与程序化纹理）；
/// - `1` = 文字（字形图集纹理）。
pub(crate) const GROUP_GRAPHIC: u8 = 0;
pub(crate) const GROUP_TEXT: u8 = 1;

/// 按 `(窗口 z, 元素序, 图形/文字组, 纹理 uid)` 分组的四边形顶点收集器（finish 提交用）。
///
/// **控件级提交顺序**：`(win, elem, g, tex)`——同窗内按**元素序**（后录控件覆盖
/// 先录控件），元素内"背景/图形 → 文字"（`g`）。与队列排序键一致；白纹理与字形
/// 同页时，控件背景+文字相邻同纹理 → Render2D 合批（单窗口一次 DrawCall）。
///
/// `debug`：**屏幕调试叠加**（DebugDraw 图元 + debug_layout 布局描边）——
/// 按 `win` 分组、恒用白纹理，`finish` 时在全部 UI 内容**之后**提交。
struct QuadCollector {
    quads: std::collections::HashMap<(u32, u32, u8, u64), Vec<VertexP3U2C4>>,
    /// 调试叠加顶点（白纹理；窗口局部物理坐标）。
    debug: std::collections::HashMap<u32, Vec<VertexP3U2C4>>,
    white_uid: u64,
    /// WHITE 纹理区域 UV（字形图集页白纹理 region；兜底为整纹理 [0,1)）。
    white_uv_tl: Vec2,
    white_uv_wh: Vec2,
    /// **当前元素序**（collect_cmds 每处理一个命令设置；push 方法按其分组）。
    cur_elem: u32,
}

impl QuadCollector {
    fn new(white_uid: u64, white_uv_tl: Vec2, white_uv_wh: Vec2) -> Self {
        Self {
            quads: std::collections::HashMap::new(),
            debug: std::collections::HashMap::new(),
            white_uid,
            white_uv_tl,
            white_uv_wh,
            cur_elem: 0,
        }
    }

    /// 白色纹理四边形（背景 / 边框 / 光标；WHITE region UV；图形组）。
    fn push_white(&mut self, win: u32, r: Rect, c: Color) {
        self.push_tex_rect(win, self.white_uid, self.white_uv_tl, self.white_uv_wh, r, c);
    }

    /// 带 UV 子区域的矩形四边形（图形组；圆角 9-patch / 渐变 / WHITE region 用）。
    fn push_tex_rect(
        &mut self,
        win: u32,
        tex: u64,
        uv_tl: Vec2,
        uv_wh: Vec2,
        r: Rect,
        c: Color,
    ) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let ca: [f32; 4] = c.into();
        let uv_br = uv_tl + uv_wh;
        let quad = [
            vertex_p3u2c4(Vec2::new(r.x, r.y), [uv_tl.x, uv_tl.y], ca),
            vertex_p3u2c4(Vec2::new(r.x + r.w, r.y), [uv_br.x, uv_tl.y], ca),
            vertex_p3u2c4(Vec2::new(r.x, r.y + r.h), [uv_tl.x, uv_br.y], ca),
            vertex_p3u2c4(Vec2::new(r.x + r.w, r.y + r.h), [uv_br.x, uv_br.y], ca),
        ];
        self.push_tex_quad_group(win, tex, quad, GROUP_GRAPHIC);
    }

    /// 追加一个带 UV 的四边形（字形用；文字组）。
    fn push_tex_quad(&mut self, win: u32, tex: u64, quad: [VertexP3U2C4; 4]) {
        self.push_tex_quad_group(win, tex, quad, GROUP_TEXT);
    }

    fn push_tex_quad_group(
        &mut self,
        win: u32,
        tex: u64,
        quad: [VertexP3U2C4; 4],
        group: u8,
    ) {
        let elem = self.cur_elem;
        self.quads
            .entry((win, elem, group, tex))
            .or_default()
            .extend_from_slice(&quad);
    }

    /// 调试叠加：一条带厚度线段（白纹理实心色；UV 取 WHITE region 中心）。
    /// 几何复用 [`rjw_2d_render::debug_draw::thick_line_quad`]；退化线段（零长 / 零宽）跳过。
    fn push_debug_line(&mut self, win: u32, a: Vec2, b: Vec2, width: f32, c: Color) {
        let Some([tl, tr, bl, br]) = rjw_2d_render::debug_draw::thick_line_quad(a, b, width) else {
            return;
        };
        let ca: [f32; 4] = c.into();
        // UV 必须落在 WHITE region 内（[0,0] 会采样字形页左上角，可能是字形像素）。
        let uv = self.white_uv_tl + self.white_uv_wh * 0.5;
        let quad = [
            vertex_p3u2c4(tl, [uv.x, uv.y], ca),
            vertex_p3u2c4(tr, [uv.x, uv.y], ca),
            vertex_p3u2c4(bl, [uv.x, uv.y], ca),
            vertex_p3u2c4(br, [uv.x, uv.y], ca),
        ];
        self.debug.entry(win).or_default().extend_from_slice(&quad);
    }

    /// 调试叠加：矩形描边（4 条带厚度线段；`width` 为物理像素）。
    fn push_debug_rect(&mut self, win: u32, r: Rect, width: f32, c: Color) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let tl = Vec2::new(r.x, r.y);
        let tr = Vec2::new(r.x + r.w, r.y);
        let br = Vec2::new(r.x + r.w, r.y + r.h);
        let bl = Vec2::new(r.x, r.y + r.h);
        self.push_debug_line(win, tl, tr, width, c);
        self.push_debug_line(win, tr, br, width, c);
        self.push_debug_line(win, br, bl, width, c);
        self.push_debug_line(win, bl, tl, width, c);
    }
}

/// 取视觉行的文本切片（**字符边界安全**）：视觉行的字节区间来自排版缓冲，
/// 编辑（粘贴/打字/IME）同帧改写文本后可能过期——按 char 边界对齐并防
/// `start > end`，避免 `&value[a..b]` 落在多字节字符中间 panic（短暂错位，
/// 次帧重新排版后自愈）。
#[inline]
fn safe_line_slice<'a>(value: &'a str, line: &VisualLine) -> &'a str {
    let s = value.floor_char_boundary(line.byte_start);
    let e = value.floor_char_boundary(line.byte_end.min(value.len()));
    if s < e {
        &value[s..e]
    } else {
        ""
    }
}

/// 文本坐标 y（逻辑像素）→ 视觉行序号：按**真实行顶**（`VisualLine.top`，物理像素）
/// 定位。不要用 `行号 × line_h`——排版行高是 `round(font×1.2)`（取整），逻辑 `line_h`
/// 每行差 ~0.2px，长文本累积后点击/拖选错行、视图滚不到真正的底部（"卡在纵轴
/// 范围内"）。
#[inline]
fn line_row_at_y(vlines: &[VisualLine], y: f32, line_h: f32, scale: f32) -> usize {
    if vlines.is_empty() {
        return 0;
    }
    let py = y * scale;
    let lh_px = (line_h * scale).round();
    vlines
        .iter()
        .position(|l| py < l.top + lh_px)
        .unwrap_or(vlines.len() - 1)
}

/// 构造一个顶点（世界坐标 + UV + 颜色）。
#[inline]
fn vertex_p3u2c4(pos: Vec2, uv: [f32; 2], color: [f32; 4]) -> VertexP3U2C4 {
    VertexP3U2C4 {
        pos: [pos.x, pos.y, 0.0],
        uv,
        color,
    }
}

/// 颜色位模式（签名哈希用）。
#[inline]
fn color_bits(c: Color) -> [u32; 4] {
    let a: [f32; 4] = c.into();
    [
        a[0].to_bits(),
        a[1].to_bits(),
        a[2].to_bits(),
        a[3].to_bits(),
    ]
}

/// debug_layout 模式：给已取整的物理矩形画一圈描边（转窗口局部坐标后写入 debug 叠加）。
/// `dbg = None` 时零开销；`Some((color, width))` 取 [`Theme::debug`] 样式。
#[inline]
fn debug_layout_outline(
    quads: &mut QuadCollector,
    win: u32,
    anchor_px: Vec2,
    pr: Rect,
    dbg: Option<(Color, f32)>,
) {
    let Some((color, width)) = dbg else {
        return;
    };
    let r = Rect::new(pr.x - anchor_px.x, pr.y - anchor_px.y, pr.w, pr.h);
    quads.push_debug_rect(win, r, width, color);
}

/// 拖拽激活所需的最小**物理像素**位移：按下后鼠标位移 ≥ 此值才视为“拖拽”。
///
/// 纯点击（无位移）不激活拖拽 → 窗口 / 可拖拽面板内的子控件（按钮 / 勾选框 /
/// 输入框等）**正常响应点击**；真正拖动中才抑制子控件交互（防止误触）。
const DRAG_ACTIVATE_PX: f32 = 3.0;

/// 是否已产生足以激活拖拽的位移（`current_px` / `press_px` 均为**物理像素**，
/// 已取整；`None` = 无按下基准，未激活）。
#[inline]
fn drag_moved(current_px: Vec2, press_px: Option<Vec2>) -> bool {
    match press_px {
        Some(p) => {
            (current_px - p).length_squared() >= DRAG_ACTIVATE_PX * DRAG_ACTIVATE_PX
        }
        None => false,
    }
}

// ─── 容器控件 API（UiAdd trait，替代旧的 widget_api! 宏） ─────────

/// **容器控件 API**：全部容器包装（[`Panel`] / [`Pack`] / [`Grid`] / [`Window`] /
/// [`Scroll`] / [`FlexCtx`]）共享的便捷方法，**替代旧的 `widget_api!` 宏**。
///
/// - 唯一必需方法 [`UiAdd::ui_mut`]（返回容器持有的 `Ui`）——**新容器只需一行 impl**
///   即可获得全部方法；
/// - 全部便捷方法都是**默认方法**（占光标、内容自动尺寸；`*_at` 变体为显式尺寸
///   逃生舱）——**新增控件便捷方法 = 在本 trait 加一个默认方法**，所有容器自动获得，
///   无需改宏；
/// - 顶层（无容器）请用 `Ui` 的 `*_at` 绝对定位方法（如 [`Ui::add_at`] /
///   [`Ui::label_at`]）。
pub trait UiAdd<'a> {
    /// 容器持有的 `Ui`（包装字段，仅本 crate 内实现）。
    fn ui_mut(&mut self) -> &mut Ui<'a>;

    /// 在容器内**占光标**放置 [`crate::widget::Widget`] 控件（尺寸 = 控件测量值
    /// 经约束 clamp 与膨胀模式调整）。
    fn add(&mut self, w: impl crate::widget::Widget) -> crate::widget::Response {
        let ui = self.ui_mut();
        let (size, expands) = ui.widget_size(&w);
        let rect = ui.child_rect_exp(size.x, size.y, expands);
        w.ui(ui, rect)
    }

    /// **绝对定位**放置 [`crate::widget::Widget`] 控件（`pos` 相对当前容器内容原点；
    /// 不占光标）。
    fn add_at(&mut self, pos: Vec2, w: impl crate::widget::Widget) -> crate::widget::Response {
        let ui = self.ui_mut();
        let (size, _) = ui.widget_size(&w);
        w.ui(ui, Rect::new(pos.x, pos.y, size.x, size.y))
    }

    /// 标签（占光标，内容自然尺寸；默认 `LimitedInParent`——在父级可用宽内
    /// **自动换行**，Resizable 窗口缩窄后不溢出）。
    fn label(&mut self, text: &str) -> Vec2 {
        let ui = self.ui_mut();
        let l = crate::widget::Label::new(text);
        let size = l.size(ui);
        let rect = ui.child_rect(size.x, size.y);
        l.ui(ui, rect);
        size
    }

    /// 绝对定位标签（`pos` 相对当前容器内容原点）。
    fn label_at(&mut self, pos: Vec2, text: &str) -> Vec2 {
        self.ui_mut().label_at(pos, text)
    }

    /// **自动换行标签**（占光标）：`max_w` 逻辑像素内按词/字换行；
    /// 返回自然尺寸（宽 = min(自然宽, max_w)，高 = 行数 × 行高）。
    /// `max_w <= 0` = 不换行（同 `label`）。
    fn label_wrap(&mut self, max_w: f32, text: &str) -> Vec2 {        let ui = self.ui_mut();
        let style = ui.theme.label.clone();
        let size = ui.text_size_wrap(text, style.font_size, style.font_family.as_deref(), max_w);
        let rect = ui.child_rect(size.x, size.y);
        let elem = ui.seq + 1;
        let seq = ui.next_seq();
        let depth = ui.depth;
        ui.queue.push(text_cmd(
            depth,
            seq,
            ui.cur_win,
            elem,
            rect,
            Arc::from(text),
            style.font_size,
            style.color,
            TextAlign::from(style.align),
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            ui.clip,
            None,
        ));
        size
    }

    /// **水平行容器**（占光标）：子项按 [`PackSide::Left`] 水平堆叠
    /// （`{Label} {Input} {Button}` 排列），整体在父容器（垂直 pack 等）中**占一行**：
    /// 宽 = 子项结算、撑大父级。**行内所有子项强制等高**（[`Theme::row_h`]，
    /// 含单行情况的多行文本框/TextArea——多行内容走垂直滚动）——各自内容垂直居中
    /// → 文字中心线对齐（近似基线，Label 不再偏上）。
    fn row(&mut self, f: impl FnOnce(&mut Pack<'_, '_>)) -> Vec2 {
        let ui = self.ui_mut();
        let origin = ui.cursor_pos();
        let gap = ui.theme.gap;
        let row_h = ui.theme.row_h;
        let (size, _) = ui.container(
            origin,
            Frame::new_stack(PackSide::Left, gap, 0.0),
            |ctx| {
                ctx.ui.frames.last_mut().expect("row frame").set_force_h_all(row_h);
                let mut p = Pack { ui: ctx.ui };
                f(&mut p);
            },
        );
        // 结算后补记父容器光标（占一行）。
        if let Some(fr) = ui.frames.last_mut() {
            fr.place_external(size);
        }
        size
    }

    /// **分割线**（占光标）：容器内占一行（高 = 线厚 + 上下留白），水平线宽 =
    /// 可用宽（容器固定宽 / 沙箱 `avail_w`），否则当前最宽子项，再否则默认 120。
    fn divider(&mut self) {
        let ui = self.ui_mut();
        let st = ui.theme.divider.clone();
        let w = ui.avail_w().unwrap_or_else(|| {
            ui.frames
                .last()
                .map(|f| f.max_child_w())
                .filter(|&w| w > 0.0)
                .unwrap_or(120.0)
        });
        let h = st.thickness + st.margin * 2.0;
        let rect = ui.child_rect(w, h);
        ui.divider_at(Vec2::new(rect.x, rect.y), w);
    }

    /// **多行文本输入框**（占光标；默认约 200×90，可 `text_area_at` 显式尺寸）。
    /// Enter 换行、↑/↓ 跨行、Home/End 行首尾；自动换行 + 垂直滚动；选择/复制/
    /// 粘贴/剪切（Ctrl+C/V/X）；IME 支持。返回 `()`（内容写回 `value`）。
    fn text_area(&mut self, id: &str, value: &mut String) {
        let ui = self.ui_mut();
        let style = ui.theme.input.clone();
        let w = style.min_w.max(200.0);
        let rect = ui.child_rect(w, 90.0);
        ui.text_area_at(id, rect, value);
    }

    /// **多行文本输入框**（显式 `Rect`）。
    fn text_area_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        self.ui_mut().text_area_at(id, rect, value);
    }

    /// **多行文本输入框（不自动换行）**（占光标；默认约 200×90）：行宽不限
    /// （显式 `\n` 分行），超出内容区**水平滚动**跟随光标；垂直滚动/选择/IME
    /// 与 [`UiAdd::text_area`] 一致。
    fn text_area_nw(&mut self, id: &str, value: &mut String) {
        let ui = self.ui_mut();
        let style = ui.theme.input.clone();
        let w = style.min_w.max(200.0);
        let rect = ui.child_rect(w, 90.0);
        ui.text_area_at_nw(id, rect, value);
    }

    /// **多行文本输入框（不自动换行）**（显式 `Rect`）。
    fn text_area_at_nw(&mut self, id: &str, rect: Rect, value: &mut String) {
        self.ui_mut().text_area_at_nw(id, rect, value);
    }

    /// **下一子项的最小尺寸约束**（`0` = 该轴不约束；一次性，作用于紧接着的下一个子项）。
    fn min_size(&mut self, w: f32, h: f32) {
        self.ui_mut().set_next_min(glam::Vec2::new(w, h));
    }

    /// **下一子项的最大尺寸约束**（`0` = 该轴不约束；一次性，作用于紧接着的下一个子项）。
    fn max_size(&mut self, w: f32, h: f32) {
        self.ui_mut().set_next_max(glam::Vec2::new(w, h));
    }

    /// **下拉框**（占光标，自动尺寸）：按钮 + 展开选项浮层；返回本帧新选中索引。
    fn combo(
        &mut self,
        id: &str,
        current: &str,
        options: &[String],
        selected: Option<u32>,
    ) -> Option<u32> {
        let ui = self.ui_mut();
        let style = ui.theme.button.clone();
        let tsize = ui.text_size(current, style.font_size, style.font_family.as_deref());
        let w = (tsize.x + 20.0).max(90.0) + style.padding.x * 2.0;
        let h = style.padding.y * 2.0 + tsize.y;
        let rect = ui.child_rect(w, h);
        ui.combo_at(id, rect, current, options, selected)
    }

    /// 按钮（文本 + padding 自动尺寸）。
    fn button(&mut self, id: &str, label: &str) -> ButtonState {
        let ui = self.ui_mut();
        let style = ui.theme.button.clone();
        let tsize = ui.text_size(label, style.font_size, style.font_family.as_deref());
        let size = Vec2::new(
            tsize.x + style.padding.x * 2.0,
            tsize.y + style.padding.y * 2.0,
        );
        let rect = ui.child_rect(size.x, size.y);
        ui.button_at(id, rect, label)
    }

    /// 显式尺寸按钮（逃生舱）。
    fn button_at(&mut self, id: &str, rect: Rect, label: &str) -> ButtonState {
        self.ui_mut().button_at(id, rect, label)
    }

    /// 滑块（自动尺寸：高度固定，宽度取样式最小宽）。
    fn slider(&mut self, id: &str, range: RangeInclusive<f32>, value: f32) -> f32 {
        let ui = self.ui_mut();
        let style = ui.theme.slider.clone();
        let size = Vec2::new(style.min_w.max(40.0), style.height);
        let rect = ui.child_rect(size.x, size.y);
        ui.slider_at(id, rect, range, value)
    }

    /// 显式尺寸滑块（逃生舱）。
    fn slider_at(
        &mut self,
        id: &str,
        rect: Rect,
        range: RangeInclusive<f32>,
        value: f32,
    ) -> f32 {
        self.ui_mut().slider_at(id, rect, range, value)
    }

    /// 勾选框（勾选值由用户维护，返回含 `toggled` 的状态）。
    fn checkbox(&mut self, id: &str, label: &str, checked: bool) -> CheckboxState {
        let ui = self.ui_mut();
        let style = ui.theme.checkbox.clone();
        let tsize = ui.text_size(label, style.font_size, style.font_family.as_deref());
        let size = Vec2::new(
            style.box_size + style.gap + tsize.x,
            style.box_size.max(tsize.y),
        );
        let rect = ui.child_rect(size.x, size.y);
        ui.checkbox_at(id, rect, label, checked)
    }

    /// 显式尺寸勾选框（逃生舱）。
    fn checkbox_at(
        &mut self,
        id: &str,
        rect: Rect,
        label: &str,
        checked: bool,
    ) -> CheckboxState {
        self.ui_mut().checkbox_at(id, rect, label, checked)
    }

    /// 勾选框（**状态自持**）：`checked` 由调用方持有，点击时本方法**直接翻转**，
    /// 无需手动 `toggled()` 维护。
    ///
    /// `id` 灵活指定（[`crate::widget::WidgetId`]，经 [`From`] 转换）：
    /// - `None` → 以 `label` 文本为 ID（同容器内标签唯一时最简）；
    /// - `Some("id")` / `"id"` → 显式字符串 ID；
    /// - `42u64` → 数字 ID（如列表行索引 `i as u64`）。
    ///
    /// 用法：`w.checkbox_mut(None, "窗口 A 选项", &mut self.win_a_checked);`
    fn checkbox_mut<'x>(
        &mut self,
        id: impl Into<crate::widget::WidgetId<'x>>,
        label: &str,
        checked: &mut bool,
    ) -> CheckboxState {
        let id = id.into().resolve(label);
        let st = self.checkbox(&id, label, *checked);
        if st.toggled() {
            *checked = !*checked;
        }
        st
    }

    /// 单选（同组 ID 互斥；返回 `checked` / `toggled`）。
    fn radio(&mut self, id: &str, group: &str, label: &str) -> CheckboxState {
        let ui = self.ui_mut();
        let style = ui.theme.checkbox.clone();
        let tsize = ui.text_size(label, style.font_size, style.font_family.as_deref());
        let size = Vec2::new(
            style.box_size + style.gap + tsize.x,
            style.box_size.max(tsize.y),
        );
        let rect = ui.child_rect(size.x, size.y);
        ui.radio_at(id, group, rect, label)
    }

    /// 显式尺寸单选（逃生舱）。
    fn radio_at(&mut self, id: &str, group: &str, rect: Rect, label: &str) -> CheckboxState {
        self.ui_mut().radio_at(id, group, rect, label)
    }

    /// 文本输入框（内容写入 `value`；自动尺寸：高度固定，宽度取样式最小宽）。
    fn text_input(&mut self, id: &str, value: &mut String) {
        let ui = self.ui_mut();
        let style = ui.theme.input.clone();
        let size = Vec2::new(style.min_w, style.height);
        let rect = ui.child_rect(size.x, size.y);
        ui.text_input_at(id, rect, value);
    }

    /// 显式尺寸文本输入框（逃生舱）。
    fn text_input_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        self.ui_mut().text_input_at(id, rect, value);
    }

    /// 嵌套面板（`pos` 相对当前容器内容原点；不占光标）。
    fn panel_at(&mut self, pos: Vec2, f: impl FnOnce(&mut Panel<'_, '_>)) -> Vec2 {
        self.ui_mut().panel_at(pos, f)
    }

    /// 嵌套**可拖拽**面板（位置持久于 `UiState.panel_pos`）。
    fn drag_panel_at(
        &mut self,
        id: &str,
        pos: Vec2,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().drag_panel_at(id, pos, f)
    }

    /// 嵌套**窗口**（可重叠 + 焦点置顶 + 可拖拽）。
    fn window_at(
        &mut self,
        id: &str,
        pos: Vec2,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().window_at(id, pos, f)
    }

    /// 嵌套**固定宽窗口**（宽度指定、高度自动，如同 egui）。
    fn window_at_w(
        &mut self,
        id: &str,
        pos: Vec2,
        width: f32,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().window_at_w(id, pos, width, f)
    }

    /// 嵌套**模态对话框**（全屏遮罩 + 对话框，背后交互被阻断）。
    fn modal_at(
        &mut self,
        id: &str,
        pos: Vec2,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().modal_at(id, pos, f)
    }

    /// 嵌套**固定宽模态对话框**（宽度指定、高度自动）。
    fn modal_at_w(
        &mut self,
        id: &str,
        pos: Vec2,
        width: f32,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().modal_at_w(id, pos, width, f)
    }

    /// 嵌套 pack（`pos` 相对当前容器内容原点；不占光标）。
    fn pack_at(
        &mut self,
        pos: Vec2,
        side: PackSide,
        f: impl FnOnce(&mut Pack<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().pack_at(pos, side, f)
    }

    /// 嵌套 grid（`pos` 相对当前容器内容原点；不占光标）。
    fn grid_at(
        &mut self,
        pos: Vec2,
        cols: usize,
        id: &str,
        f: impl FnOnce(&mut Grid<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().grid_at(pos, cols, id, f)
    }
}

/// 容器闭包上下文（内部类型）。
pub(crate) struct ContainerCtx<'ui, 'a> {
    pub(crate) ui: &'ui mut Ui<'a>,
}

/// 面板容器（背景 + 边框 + 垂直堆叠内容）。
pub struct Panel<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> UiAdd<'a> for Panel<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

/// pack 容器（无背景，纯布局）。
pub struct Pack<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> UiAdd<'a> for Pack<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

/// grid 容器（无背景，均匀网格）。
pub struct Grid<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> UiAdd<'a> for Grid<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

/// 窗口容器（可重叠 + 焦点置顶 + 可拖拽；见 [`Ui::window_at`]）。
pub struct Window<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> UiAdd<'a> for Window<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

/// 滚动容器（内容在可视区内堆叠 + 滚动；见 [`Ui::scroll_at`]）。
pub struct Scroll<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> UiAdd<'a> for Scroll<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

/// **flex 容器上下文**（[`Ui::flex_at`]）：子项高度已按权重分配（强制），
/// 内部可调用任意控件方法占光标（`f.label` / `f.button` 等）。
pub struct FlexCtx<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> UiAdd<'a> for FlexCtx<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

// ─── 控件实现（Ui 内部方法） ────────────────────────────────────

impl Ui<'_> {
    /// **下拉框**（显式 rect；`rect` 为相对当前容器 origin 的局部坐标）。
    ///
    /// 按钮显示 `current`；点击展开**选项浮层**（临时窗口置顶，自动尺寸包裹选项），
    /// 点击选项选中并收起，点击浮层外收起。`selected` 为当前选中（用于 ✓ 标记）。
    /// 返回本帧新选中的索引（`None` = 无选择/未展开）。
    pub fn combo_at(
        &mut self,
        id: &str,
        rect: Rect,
        current: &str,
        options: &[String],
        selected: Option<u32>,
    ) -> Option<u32> {
        let mut picked = None;
        let open = self.state.combo_open.as_deref() == Some(id);
        // 登记焦点链（键盘导航：Tab 可到；Enter/Space 展开收起；方向键切换选项）。
        self.register_focus(id, rect, FocusKind::Combo);
        // 按钮交互（点击 toggle）。
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        let key_click = self.key_click(id, FocusKind::Combo);
        let mut ev = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let ev = update_interact(ws, hit, btn);
            if key_click {
                ws.pressed = true;
            }
            ev
        };
        if key_click {
            ev.clicked = true;
        }
        if ev.pressed {
            self.any_pressed = true;
        }
        if ev.clicked {
            self.state.combo_open = if open { None } else { Some(id.to_owned()) };
        }
        // 键盘：焦点下展开时，上下方向键切换选项（选中即关闭浮层）；Esc 收起。
        if open {
            if self.focused_is(id) {
                let n = options.len() as u32;
                if n > 0 {
                    let cur = selected.unwrap_or(0).min(n - 1);
                    if self.keyboard.get(KeyCode::ArrowUp).down_edge() {
                        picked = Some(if cur == 0 { n - 1 } else { cur - 1 });
                    }
                    if self.keyboard.get(KeyCode::ArrowDown).down_edge() {
                        picked = Some(if cur + 1 >= n { 0 } else { cur + 1 });
                    }
                }
            }
            if self.keyboard.get(KeyCode::Escape).down_edge() {
                self.state.combo_open = None;
            }
        }
        // 按钮绘制（展开时用 pressed 态背景）。
        let style = self.theme.button.clone();
        let elem = self.seq + 1;
        let bg = if open { style.bg_pressed } else { style.bg };
        self.push_panel_like(rect, bg, style.border, style.border_w, style.radius, elem);
        let text_rect = Rect::new(rect.x, rect.y, (rect.w - 18.0).max(0.0), rect.h);
        // 按钮文本自动省略（缩窄 / max 约束下不溢出，内容自洽）。
        let cur_owned = self.ellipsized(
            current,
            style.font_size,
            style.font_family.as_deref(),
            text_rect.w,
        );
        let draw_current: &str = cur_owned.as_deref().unwrap_or(current);
        let seq = self.next_seq();
        self.queue.push(text_cmd(
            self.depth,
            seq,
            self.cur_win,
            elem,
            text_rect,
            Arc::from(draw_current),
            style.font_size,
            style.fg,
            TextAlign::Left,
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.clip,
        None,
        ));
        let arrow_rect = Rect::new(rect.x + rect.w - 18.0, rect.y, 18.0, rect.h);
        let seq = self.next_seq();
        self.queue.push(text_cmd(
            self.depth,
            seq,
            self.cur_win,
            elem,
            arrow_rect,
            Arc::from("▼"),
            style.font_size,
            style.fg,
            TextAlign::Center,
            TextVAlign::Center,
            None,
            None,
            self.clip,
        None,
        ));
        // 展开的选项浮层：临时窗口，**显式置顶**（z = WIN_TOPMOST → 覆盖一切，
        // 不受其他窗口置顶书签影响）；自动尺寸包裹选项。
        if open {
            let popup_pos = Vec2::new(rect.x, rect.y + rect.h + 2.0);
            let popup_id = format!("{id}::popup");
            // 强制哨兵 z：window_at 的 entry().or_insert() 保留现有值。
            self.state.window_z.insert(popup_id.clone(), WIN_TOPMOST);
            let popup_size = self.window_at(&popup_id, popup_pos, |w| {
                for (i, opt) in options.iter().enumerate() {
                    let sel = selected == Some(i as u32);
                    let label = if sel { format!("✓ {opt}") } else { opt.clone() };
                    if w.button(&format!("{id}::opt_{i}"), &label).clicked() {
                        picked = Some(i as u32);
                    }
                }
            });
            // 点击浮层外（且不在按钮上）→ 收起。
            // ⚠ popup_pos / rect 是**相对当前容器**的局部坐标，必须转**绝对**再与
            // 绝对鼠标坐标比较——否则容器有偏移时（如 pack_at(16,90)）判定错位，
            // 点选项会被误判为"点外部"导致浮层收起且不选中。
            let popup_abs = Rect::new(
                self.abs_base.x + popup_pos.x,
                self.abs_base.y + popup_pos.y,
                popup_size.x,
                popup_size.y,
            );
            let btn_abs = Rect::new(
                self.abs_base.x + rect.x,
                self.abs_base.y + rect.y,
                rect.w,
                rect.h,
            );
            if btn.down_edge()
                && !hit_test(&popup_abs, self.mouse_logical)
                && !hit_test(&btn_abs, self.mouse_logical)
            {
                self.state.combo_open = None;
            }
        }
        if picked.is_some() {
            self.state.combo_open = None;
        }
        picked
    }

    /// **下拉框**（顶层定位：`pos` 相对当前容器内容原点，绝对定位；尺寸自动）。
    pub fn combo(
        &mut self,
        id: &str,
        pos: Vec2,
        current: &str,
        options: &[String],
        selected: Option<u32>,
    ) -> Option<u32> {
        let style = self.theme.button.clone();
        let tsize = self.text_size(current, style.font_size, style.font_family.as_deref());
        let w = (tsize.x + 20.0).max(90.0) + style.padding.x * 2.0;
        let h = style.padding.y * 2.0 + tsize.y;
        let rect = Rect::new(pos.x, pos.y, w, h);
        self.combo_at(id, rect, current, options, selected)
    }

    /// 按钮（显式 rect；样式取全局 `Theme::button`）。
    pub fn button_at(&mut self, id: &str, rect: Rect, label: &str) -> ButtonState {
        let style = self.theme.button.clone();
        self.button_at_styled(id, rect, label, &style)
    }

    /// 按钮（显式 rect + **样式可覆盖**——widget 层 [`crate::widget::Button`] 经此
    /// 合并主题与逐控件属性；旧 `button_at` 委托本方法）。
    pub fn button_at_styled(
        &mut self,
        id: &str,
        rect: Rect,
        label: &str,
        style: &ButtonStyle,
    ) -> ButtonState {
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        // 键盘激活（Enter/Space + 焦点）→ 视为点击；先取出（不借用 self）
        let key_click = self.key_click(id, FocusKind::Button);
        if key_click {
            self.any_pressed = true;
        }
        {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let mut ev = update_interact(ws, hit, btn);
            if key_click {
                // 焦点键盘点击：合成 pressed + clicked（触发本帧回调）。
                ws.pressed = true;
                ev.clicked = true;
            }
            if ev.pressed {
                self.any_pressed = true;
            }
            let (pressed, hovered) = (ws.pressed, ws.hovered);
            // 记录绘制（ws 借用已结束）
            let bg = if pressed {
                style.bg_pressed
            } else if hovered {
                style.bg_hover
            } else {
                style.bg
            };
            let depth = self.depth;
            let win = self.cur_win;
            let elem = self.seq + 1;
            // 背景 + 边框（radius > 0 走圆角双层矩形）。
            self.push_panel_like(rect, bg, style.border, style.border_w, style.radius, elem);
            // 按钮文本自动省略（Resizable 窗口缩窄 / max 约束下不溢出）：
            // 文本超出可用区（rect 宽 - 水平内边距）→ "…"截断（内容自洽，noclip）。
            let label_owned = self.ellipsized(
                label,
                style.font_size,
                style.font_family.as_deref(),
                (rect.w - style.padding.x * 2.0).max(0.0),
            );
            let draw_label: &str = label_owned.as_deref().unwrap_or(label);
            let text_seq = self.next_seq();
            self.queue.push(text_cmd(
                depth,
                text_seq,
                win,
                elem,
                rect,
                Arc::from(draw_label),
                style.font_size,
                style.fg,
                TextAlign::Center,
                TextVAlign::Center,
                style.font_family.clone(),
                None,
                self.clip,
            None,
            ));
            ButtonState {
                hovered,
                pressed,
                clicked: ev.clicked,
                released: ev.released,
            }
        }
    }

    /// 滑块（显式 rect）。
    pub fn slider_at(
        &mut self,
        id: &str,
        rect: Rect,
        range: RangeInclusive<f32>,
        value: f32,
    ) -> f32 {
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        // 滑块自身有拖拽语义：按下即置位 press_claimed——阻止外层窗口/面板把本次
        // 按下当作拖拽基准（窗口内拖滑块不再连窗口一起动）。
        if btn.down_edge() && hit {
            self.press_claimed = true;
        }
        let active = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            update_drag(ws, hit, btn)
        };
        // 拖动光标：拖拽中 = 抓握；悬停滑块 = 张手。
        if active {
            self.cursor_grabbing = true;
        } else if hit {
            self.cursor_grab = true;
        }
        if active {
            self.any_pressed = true;
        }
        let (lo, hi) = (*range.start(), *range.end());
        let span = hi - lo;
        let mut new_value = value;
        if active && span.abs() > f32::EPSILON {
            let t = normalize_x(&rect, self.mouse_local_x());
            new_value = lo + t * span;
        }
        // 键盘：焦点下滑块用左右方向键调值（步进 = 范围的 5%，即时生效）。
        if self.focused_is(id) && span.abs() > f32::EPSILON {
            let step = span * 0.05;
            if self.keyboard.get(KeyCode::ArrowLeft).down_edge() {
                new_value = (new_value - step).clamp(lo, hi);
            }
            if self.keyboard.get(KeyCode::ArrowRight).down_edge() {
                new_value = (new_value + step).clamp(lo, hi);
            }
        }
        let t = if span.abs() > f32::EPSILON {
            ((new_value - lo) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let style = self.theme.slider.clone();
        let depth = self.depth;
        let win = self.cur_win;
        let elem = self.seq + 1;
        let track_rect =
            Rect::new(rect.x, rect.y + (rect.h - style.track_h) * 0.5, rect.w, style.track_h);
        // 手柄**中心**夹在轨道两端之内（t=0/1 时手柄不伸出轨道/控件外）；
        // 填充画到手柄**左缘**（与手柄无缝衔接，而非只到手柄中心——消除"错位"）。
        let handle_cx = rect.x + style.handle_w * 0.5 + (rect.w - style.handle_w) * t;
        let fill_w = (handle_cx - style.handle_w * 0.5 - rect.x).max(0.0);
        let fill_rect = Rect::new(rect.x, track_rect.y, fill_w, style.track_h);
        let handle_rect = Rect::new(
            handle_cx - style.handle_w * 0.5,
            rect.y + (rect.h - style.handle_w) * 0.5,
            style.handle_w,
            style.handle_w,
        );
        let push_solid = |ui: &mut Ui<'_>, r: Rect, c: Color| {
            if r.w > 0.0 && r.h > 0.0 {
                let seq = ui.next_seq();
                ui.queue.push(UiDraw {
                    depth,
                    seq,
                    win,
                    elem,
                    rect: r,
                    clip: ui.clip,
                    kind: DrawKind::Solid(c),
                });
            }
        };
        push_solid(self, track_rect, style.track);
        push_solid(self, fill_rect, style.fill);
        push_solid(self, handle_rect, style.handle);
        if style.handle_w > 2.0 {
            let seq = self.next_seq();
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: handle_rect,
                clip: self.clip,
                kind: DrawKind::Border {
                    color: style.handle_border,
                    width: 1.0,
                },
            });
        }
        new_value
    }

    /// 勾选框（显式 rect；样式取全局 `Theme::checkbox`）。
    pub fn checkbox_at(
        &mut self,
        id: &str,
        rect: Rect,
        label: &str,
        checked: bool,
    ) -> CheckboxState {
        let style = self.theme.checkbox.clone();
        self.checkbox_at_styled(id, rect, label, checked, &style)
    }

    /// 勾选框（显式 rect + **样式可覆盖**——widget 层 [`crate::widget::Checkbox`] 经此
    /// 合并主题与逐控件属性；旧 `checkbox_at` 委托本方法）。
    pub fn checkbox_at_styled(
        &mut self,
        id: &str,
        rect: Rect,
        label: &str,
        checked: bool,
        style: &CheckboxStyle,
    ) -> CheckboxState {
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        let key_click = self.key_click(id, FocusKind::Checkbox);
        if key_click {
            self.any_pressed = true;
        }
        let mut ev = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let ev = update_interact(ws, hit, btn);
            if key_click {
                ws.pressed = true;
            }
            ev
        };
        if key_click {
            ev.clicked = true;
        }
        if ev.pressed {
            self.any_pressed = true;
        }
        let (hovered, pressed) = {
            let ws = self.state.widgets.get(id).expect("checkbox ws");
            (ws.hovered, ws.pressed)
        };
        self.draw_check_common(rect, label, checked, style);
        CheckboxState {
            hovered,
            pressed,
            checked,
            toggled: ev.clicked,
            clicked: ev.clicked,
        }
    }

    /// 单选（显式 rect）。
    pub fn radio_at(
        &mut self,
        id: &str,
        group: &str,
        rect: Rect,
        label: &str,
    ) -> CheckboxState {
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        let key_click = self.key_click(id, FocusKind::Radio);
        if key_click {
            self.any_pressed = true;
        }
        let mut ev = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let ev = update_interact(ws, hit, btn);
            if key_click {
                ws.pressed = true;
            }
            ev
        };
        if key_click {
            ev.clicked = true;
        }
        if ev.pressed {
            self.any_pressed = true;
        }
        let was_checked = self
            .state
            .radio_groups
            .get(group)
            .map(|s| s == id)
            .unwrap_or(false);
        if ev.clicked {
            self.state.radio_groups.insert(group.to_owned(), id.to_owned());
        }
        let checked = self
            .state
            .radio_groups
            .get(group)
            .map(|s| s == id)
            .unwrap_or(false);
        let style = self.theme.checkbox.clone();
        self.draw_check_common(rect, label, checked, &style);
        let (hovered, pressed) = {
            let ws = self.state.widgets.get(id).expect("radio ws");
            (ws.hovered, ws.pressed)
        };
        CheckboxState {
            hovered,
            pressed,
            checked,
            toggled: ev.clicked && !was_checked,
            clicked: ev.clicked,
        }
    }

    /// 勾选框 / 单选公共绘制：方框 +（选中时）填充 + 标签文本（样式可覆盖）。
    fn draw_check_common(&mut self, rect: Rect, label: &str, checked: bool, style: &CheckboxStyle) {
        let depth = self.depth;
        let win = self.cur_win;
        let elem = self.seq + 1;
        let box_rect = Rect::new(
            rect.x,
            rect.y + (rect.h - style.box_size) * 0.5,
            style.box_size,
            style.box_size,
        );
        let seq = self.next_seq();
        self.queue.push(UiDraw {
            depth,
            seq,
            win,
            elem,
            rect: box_rect,
            clip: self.clip,
            kind: DrawKind::Border {
                color: style.box_border,
                width: style.border_w,
            },
        });
        if checked {
            // 中心填充 = 外框 **内缩**（减法，非写死偏移）：
            // inset（物理像素）= floor(border_w·scale) + floor(CHECKBOX_INNER·scale)，
            // 转回逻辑后 shrink —— 内缩量与边框/DPI 一致，任意缩放不溢出。
            let inset_px = (style.border_w * self.scale).floor()
                + (CHECKBOX_INNER * self.scale).floor();
            let inner = box_rect.shrink(inset_px / self.scale);
            if inner.w > 0.0 && inner.h > 0.0 {
                let seq = self.next_seq();
                self.queue.push(UiDraw {
                    depth,
                    seq,
                    win,
                    elem,
                    rect: inner,
                    clip: self.clip,
                    kind: DrawKind::Solid(style.checked_fill),
                });
            }
        }
        let text_rect = Rect::new(
            box_rect.x + style.box_size + style.gap,
            rect.y,
            (rect.w - style.box_size - style.gap).max(0.0),
            rect.h,
        );
        // 标签文本自动省略（缩窄 / max 约束下不溢出，内容自洽）。
        let label_owned = self.ellipsized(label, style.font_size, style.font_family.as_deref(), text_rect.w);
        let draw_label: &str = label_owned.as_deref().unwrap_or(label);
        let seq = self.next_seq();
        self.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            text_rect,
            Arc::from(draw_label),
            style.font_size,
            style.fg,
            TextAlign::Left,
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.clip,
        None,
        ));
    }

    /// 文本输入框（显式 rect，**单行**）。
    ///
    /// 增强能力：
    /// - **超长文本滚动跟随光标**：文本超出内容区时左移，光标始终可见（`WidgetState::text_scroll`）；
    /// - **文本选择**：按住拖拽选择（`WidgetState::sel_anchor`），选择优先于窗口/面板拖拽
    ///   （按下时置位 `press_claimed`）；Ctrl+C/V/X 复制/粘贴/剪切；选择后打字/退格替换选择；
    /// - **IME 组合候选移入浮动提示框**：组合串（preedit）画在输入框下方浮动小框中（不再占行内）。
    pub fn text_input_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        let hit = self.hit_abs(&rect);
        if hit {
            // 鼠标悬停在输入框上 → 本帧系统光标设为 I 型（finish 统一设置）
            self.cursor_text = true;
        }
        let btn = self.mouse_left();
        // 登记焦点链（Tab/方向键可遍历到输入框）。
        self.register_focus(id, rect, FocusKind::TextInput);
        let mouse_local_x = self.mouse_local_x();
        // 提前测量（避免在 ws 借用期间调用 &mut self 方法）
        let input_style = self.theme.input.clone();
        // 光标定位（按字符**实际宽度**，前缀测量二分——混合中英文精确落位）。
        // **单击与拖选都按"文本坐标"（视口 cx + 水平滚动偏移）**：横向滚动后点击
        // 视口内的 J-K 位置 → 映射到全文 J-K（而非文本前部 A-B），光标落在点击处、
        // 视图不跳回起点。（曾用纯视口 cx：滚动后点击会定位到文本起点附近，随后
        // scroll 跟随把视图拉回开头——"点击右侧视图，视图跳回 A-B"）。
        // `text_scroll` 为**物理像素**（内部计算一律物理），文本坐标 = cx + 物理/scale。
        let prev_scroll = self
            .state
            .widgets
            .get(id)
            .map(|w| w.text_scroll)
            .unwrap_or(0.0);
        // cx_raw 允许**负值**（鼠标拖出左缘）——拖选时左缘持续滚动（edge-scroll）；
        // 单击才 clamp 到 0（点击最左 = 光标在可视区起点）。
        let cx_raw = mouse_local_x - rect.x - input_style.padding_x;
        let cx = cx_raw.max(0.0);
        let click_caret = if btn.down_edge() && hit {
            Some(self.caret_index_at_width(
                value,
                input_style.font_size,
                input_style.font_family.as_deref(),
                cx + prev_scroll / self.scale,
            ))
        } else {
            None
        };
        let drag_caret = if btn.pressed() && !btn.down_edge() {
            Some(self.caret_index_at_width(
                value,
                input_style.font_size,
                input_style.font_family.as_deref(),
                cx_raw + prev_scroll / self.scale,
            ))
        } else {
            None
        };
        // 记录帧首光标：仅"光标移动"（打字/方向键/点击/拖选）时做滚动跟随——
        // 滚轮滚动不移动光标 → 不跟随（滚轮自由滚动、光标可滚出视图，不被拉回）。
        let prev_caret = self.state.widgets.get(id).map(|w| w.caret);
        let caret_est = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let ev = update_interact(ws, hit, btn);
            if ev.pressed {
                self.any_pressed = true;
                // 输入框按下占用该次按压：从输入框拖拽 = 选择文本（窗口/面板不建立拖拽基准）
                self.press_claimed = true;
                self.state.focused = Some(id.to_owned());
                if let Some(c) = click_caret {
                    ws.caret = c;
                }
                // 双击检测（同控件帧间隔 ≤ DOUBLE_CLICK_FRAMES 且位移 < 阈值）：
                // 第二击选中光标所在"词"并进入词模式——按住继续拖拽按词扩散。
                let is_dbl = {
                    let (pf, pp) = (ws.last_click_frame, ws.last_click_pos);
                    ws.last_click_frame = self.state.frame;
                    ws.last_click_pos = self.mouse_logical;
                    pf != 0
                        && self.state.frame.wrapping_sub(pf) <= DOUBLE_CLICK_FRAMES
                        && (self.mouse_logical - pp).length() < DOUBLE_CLICK_DIST
                };
                // 拖选位移基准（物理像素；微动不触发拖选 → 单击保持插入模式）
                ws.press_mouse = Some(self.mouse_screen.round());
                if is_dbl {
                    let (w0, w1) = crate::edit::word_range(value, ws.caret);
                    ws.sel_anchor = Some(w0);
                    ws.caret = w1;
                    ws.sel_word = true;
                } else {
                    ws.sel_word = false;
                    ws.sel_anchor = Some(ws.caret);
                }
            } else if ws.pressed && btn.pressed() {
                // 拖拽选择：**位移 ≥ 3 物理像素**才扩展选择（单击微动不误选）；
                // 光标跟随鼠标（**即使拖出输入框**——edge-scroll 持续滚动），
                // 选择范围 = [anchor, caret)。
                self.press_claimed = true;
                let moved = ws
                    .press_mouse
                    .map(|p| (self.mouse_screen.round() - p).length_squared() >= 9.0)
                    .unwrap_or(false);
                if moved {
                    if let Some(c) = drag_caret {
                        if ws.sel_word {
                            // 词模式（双击后拖拽）：按词边界扩散选择
                            let anchor = ws.sel_anchor.unwrap_or(c);
                            ws.caret = crate::edit::extend_word_caret(value, anchor, c);
                        } else {
                            ws.caret = c;
                        }
                    }
                }
            }
            if ev.released {
                // 纯点击（无位移）：anchor == caret，无实际选择 → 清理，避免残留
                // anchor 在后续无 Shift 方向键移动时"突然变成多选"。
                if ws.sel_anchor == Some(ws.caret) {
                    ws.sel_anchor = None;
                }
                // 释放后退出词模式（选择保留；下次单击/双击重开）。
                ws.sel_word = false;
            }
            let focused = self.state.focused.as_deref() == Some(id);
            if focused {
                // IME 组合中（preedit 非空）**或刚结束的帧**（上一帧在组合）：
                // 退格/删除/方向键由 **IME 系统**处理（缩短组合串、结束组合、移动
                // 组合光标）——本地处理会误删已有文本（组合结束帧 Preedit("") 先清空
                // 候选、随后退格键到达，只看当前帧会误判为非组合而误删）。
                let in_ime_compose =
                    self.keyboard.get_ime_preedit().is_some_and(|p| !p.is_empty());
                let ime_owns_keys = in_ime_compose || self.state.ime_composing;
                // 编辑状态机（单行）：剪贴板 Ctrl+C/V/X/A（粘贴过滤换行）、选择替换、
                // IME 上屏、普通字符、退格/删除（见 [`crate::edit::apply_frame_edits`]）。
                crate::edit::apply_frame_edits(&self.keyboard, ws, value, false, ime_owns_keys);
                // Shift + ←/→：扩展/收缩选择（无 Shift 取消选择）。
                let shift = self.keyboard.get(KeyCode::ShiftLeft).pressed()
                    || self.keyboard.get(KeyCode::ShiftRight).pressed();
                if self.keyboard.get(KeyCode::ArrowLeft).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, -1, shift);
                }
                if self.keyboard.get(KeyCode::ArrowRight).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, 1, shift);
                }
                if self.keyboard.get(KeyCode::Enter).down_edge() {
                    self.state.focused = None;
                }
                // Esc：取消输入焦点（不再把 Esc 传给应用层快捷键）
                if self.keyboard.get(KeyCode::Escape).down_edge() {
                    self.state.focused = None;
                }
            }
            (focused, ws.caret)
        };
        let (focused, caret) = caret_est;
        // 绘制
        let style = self.theme.input.clone();
        let depth = self.depth;
        let win = self.cur_win;
        let elem = self.seq + 1;
        let border = if focused { style.border_focus } else { style.border };
        // 视觉框绝对矩形（Clip 沙箱用）：**整个输入框**——高亮/光标/文本命令
        // 受其强制裁剪（滚出视图不画出框，且外层 ScrollView 裁切一并生效）。
        let box_clip = Rect::new(self.abs_base.x + rect.x, self.abs_base.y + rect.y, rect.w, rect.h);
        // **Clip 子沙箱**（控件内）：强制裁剪层 = 外层强制 ∩ 输入框矩形。
        let saved_clip = self.clip;
        self.clip = clip_for_view(saved_clip, box_clip, ViewMode::Clip);
        // 背景 + 边框（radius > 0 走圆角双层矩形）。
        self.push_panel_like(rect, style.bg, border, style.border_w, style.radius, elem);
        let content_w = (rect.w - style.padding_x * 2.0).max(0.0);
        let content_rect = Rect::new(rect.x + style.padding_x, rect.y, content_w, rect.h);
        // **IME 组合内联融入**：显示串 = value[..caret] + preedit + value[caret..]——
        // 后续文本（"xXXXXAAAA" 的 AAAA）右移而非被组合盖住；组合较长时滚动跟随
        // 组合光标（提示文字不裁切）。无组合时全部回落到 value（零开销路径）。
        // IME 组合串先拷出（owned）：闭包内要 &mut self（text_size 测量），
        // 与自持快照字段 self.keyboard 的借用不能共存。
        let preedit = self.keyboard.get_ime_preedit().map(|p| p.to_owned());
        let preedit_caret = self.keyboard.get_ime_preedit_caret();
        let composed: Option<(String, std::ops::Range<usize>, f32, usize)> = if focused {
            preedit
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let insert_b = char_to_byte(value, caret);
                    let disp = format!("{}{}{}", &value[..insert_b], p, &value[insert_b..]);
                    let w = self.text_size(&p, style.font_size, style.font_family.as_deref()).x;
                    // 组合内光标：字节 → 显示串偏移（None = 组合末尾）
                    let caret_b = preedit_caret
                        .map(|b| p.floor_char_boundary(b.min(p.len())))
                        .unwrap_or(p.len());
                    (disp, insert_b..insert_b + p.len(), w, insert_b + caret_b)
                })
        } else {
            None
        };
        // 文本自然宽（水平滚动上限）与光标 x（前缀宽度）——都基于**显示串**。
        let text_w = match &composed {
            Some((disp, ..)) => {
                self.text_size(disp, style.font_size, style.font_family.as_deref()).x
            }
            None => self.text_size(value, style.font_size, style.font_family.as_deref()).x,
        };
        let caret_x = match &composed {
            Some((disp, _, _, caret_disp)) => self
                .text_size(&disp[..*caret_disp], style.font_size, style.font_family.as_deref())
                .x,
            None => {
                let prefix: String = value.chars().take(caret).collect();
                self.text_size(&prefix, style.font_size, style.font_family.as_deref()).x
            }
        };
        // 水平滚动（**物理像素**）：**水平滚轮（触控板）优先**（自由滚动，可把光标
        // 滚出视图，**仅鼠标在框内时**——指针离开输入框后不再滚动）；
        // 否则仅**光标移动**（打字/方向键/点击/拖选）时跟随光标（右侧保留 8 逻辑
        // 像素；滚轮自由滚动后不被光标拉回；组合时跟随组合光标）。
        let scroll = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let (wx, _) = self.mouse.get_mouse_wheel_delta();
            if hit && wx != 0.0 {
                let max_h_px = ((text_w - content_w).max(0.0) * self.scale).round();
                ws.text_scroll = (ws.text_scroll - (wx as f32 * 40.0 * self.scale).round())
                    .clamp(0.0, max_h_px);
            } else if Some(caret) != prev_caret {
                ws.text_scroll = scroll_follow_caret(
                    ws.text_scroll,
                    caret_x * self.scale,
                    content_w * self.scale,
                    text_w * self.scale,
                    8.0 * self.scale,
                );
            }
            ws.text_scroll
        };
        let text_dx = -scroll / self.scale;
        // 文本选择高亮（在文本之下绘制：同一 elem 的图形组先于文字组）。
        if let Some((lo, hi)) = sel_range(
            self.state.widgets.get(id).and_then(|w| w.sel_anchor),
            caret,
        ) {
            let lo_x = {
                let p: String = value.chars().take(lo).collect();
                self.text_size(&p, style.font_size, style.font_family.as_deref()).x
            };
            let hi_x = {
                let p: String = value.chars().take(hi).collect();
                self.text_size(&p, style.font_size, style.font_family.as_deref()).x
            };
            let sel_rect = Rect::new(
                content_rect.x + lo_x + text_dx,
                content_rect.y + 1.0,
                (hi_x - lo_x).max(0.0),
                (content_rect.h - 2.0).max(0.0),
            );
            if sel_rect.w > 0.0 && sel_rect.h > 0.0 {
                let seq = self.next_seq();
                self.queue.push(UiDraw {
                    depth,
                    seq,
                    win,
                    elem,
                    rect: sel_rect,
                    // 选择高亮受输入框强制裁剪（不溢出输入框 / 外层滚动容器）。
                    clip: self.clip,
                    kind: DrawKind::Solid(style.sel_bg),
                });
            }
        }
        // 文本（左移 scroll；**裁剪窗口固定在视觉框**：clip 相对移动后的 rect 起点 =
        // scroll/scale - padding_x，绝对位置 = 框左缘 —— 若 clip.x=0 会随 rect 一起
        // 左移，始终显示文本开头且偏离文本框；缓冲控件自持）。
        let clip = Rect::new(scroll / self.scale - style.padding_x, 0.0, rect.w, rect.h);
        // 绘制文本：组合时画**显示串**（preedit 已融入）；缓冲控件自持（按键变化重排）。
        let (draw_text, buf) = match &composed {
            Some((disp, ..)) => {
                let buf = self.ensure_text_buf(
                    id,
                    disp,
                    style.font_size,
                    style.font_family.as_deref(),
                    0.0,
                    1.0,
                );
                (disp.as_str(), buf)
            }
            None => {
                let buf = self.ensure_text_buf(
                    id,
                    value,
                    style.font_size,
                    style.font_family.as_deref(),
                    0.0,
                    1.0,
                );
                (value.as_str(), buf)
            }
        };
        let seq = self.next_seq();
        self.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            Rect::new(content_rect.x + text_dx, content_rect.y, content_w, rect.h),
            Arc::from(draw_text),
            style.font_size,
            style.fg,
            TextAlign::Left,
            TextVAlign::Center,
            style.font_family.clone(),
            Some(clip),
            self.clip,
            Some(buf),
        ));
        // **组合下划线**：覆盖组合文本段（显示串 `[span]`），受内容区裁剪。
        // 组合文本已融入显示串（后续文本右移），无需单独绘制文字。
        if let Some((disp, span, preedit_w, _)) = &composed {
            let prefix_x =
                self.text_size(&disp[..span.start], style.font_size, style.font_family.as_deref())
                    .x;
            let ul = Rect::new(
                content_rect.x + prefix_x + text_dx,
                content_rect.y + content_rect.h - 3.0,
                *preedit_w,
                2.0,
            );
            if ul.w > 0.0 && ul.h > 0.0 {
                let useq = self.next_seq();
                self.queue.push(UiDraw {
                    depth,
                    seq: useq,
                    win,
                    elem,
                    rect: ul,
                    clip: self.clip,
                    kind: DrawKind::Solid(style.preedit),
                });
            }
        }
        // **IME 候选框定位**：跟随组合光标（窗口客户区物理像素；无组合 = 输入光标）。
        if focused {
            let ime_x = ((self.abs_base.x + content_rect.x + caret_x + text_dx) * self.scale)
                as i32;
            let ime_y = ((self.abs_base.y + rect.y) * self.scale) as i32;
            let ime_w = (rect.w * self.scale).max(1.0) as u32;
            let ime_h = (rect.h * self.scale).max(1.0) as u32;
            let _ = self.window.set_ime_cursor_area(
                PhysicalPosition::new(ime_x, ime_y),
                PhysicalSize::new(ime_w, ime_h),
            );
        }
        // 光标（跟随水平滚动；组合时 = 显示串内的组合光标）
        if focused && self.state.caret_blink_on() {
            let caret_rect = Rect::new(
                content_rect.x + caret_x + text_dx,
                content_rect.y + 2.0,
                1.0,
                (content_rect.h - 4.0).max(1.0),
            );
            let seq = self.next_seq();
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: caret_rect,
                clip: self.clip,
                kind: DrawKind::Caret {
                    color: style.caret,
                    width: 1.0,
                },
            });
        }
        // 退出 Clip 子沙箱（恢复外层强制裁剪层）。
        self.clip = saved_clip;
    }

    /// 多行文本输入框（显式 rect，**TextArea**）。
    ///
    /// - **编辑**：Enter 换行、↑/↓ 跨**视觉行**（保持列）、Home/End 行首/行尾、
    ///   ←/→ 字符移动、Backspace/Delete、选择替换；Esc 失焦；
    /// - **渲染**：文本按内容区宽度**自动换行**（[`rjw_text::Text::create_buffer_wrap`]）；
    ///   超出高度**垂直滚动**（滚轮 + 光标跟随，`WidgetState::scroll_y`）；
    /// - **光标 / 点击 / 选择按"视觉行"（自动换行后）定位**——与显示完全一致
    ///   （[`rjw_text::Text::visual_lines`]：每个 `LayoutRun` 一行，含字节范围）；
    /// - **选择 / 复制 / 粘贴 / 剪切**（Ctrl+C/V/X）跨视觉行，高亮逐行绘制。
    /// - **IME**：组合候选浮动提示框 + 候选框定位到光标。
    ///
    /// 自动换行模式（`wrap = true`）：行宽 = 内容区宽，超出自动换行，仅垂直滚动。
    /// 不自动换行模式（`wrap = false`）：行宽不限（显式 `\n` 分行），**水平滚动**
    /// 跟随光标（同单行输入框），垂直滚动不变。对应公开入口
    /// [`Self::text_area_at`]（换行）/ [`Self::text_area_at_nw`]（不换行）。
    pub fn text_area_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        self.text_area_impl(id, rect, value, true)
    }

    /// **多行文本输入框（不自动换行）**：同 [`Self::text_area_at`]，但行宽不限
    /// （显式 `\n` 分行），超出内容区**水平滚动**跟随光标（光标右侧保留 8 逻辑像素）；
    /// 垂直滚动/选择/IME 与换行模式一致。
    pub fn text_area_at_nw(&mut self, id: &str, rect: Rect, value: &mut String) {
        self.text_area_impl(id, rect, value, false)
    }

    /// 多行文本输入框公共实现（`wrap`：是否按内容区宽自动换行；`false` = 水平滚动）。
    fn text_area_impl(&mut self, id: &str, rect: Rect, value: &mut String, wrap: bool) {
        let hit = self.hit_abs(&rect);
        if hit {
            // 鼠标悬停在输入框上 → 本帧系统光标设为 I 型（finish 统一设置）
            self.cursor_text = true;
        }
        let btn = self.mouse_left();
        self.register_focus(id, rect, FocusKind::TextInput);
        let style = self.theme.input.clone();
        // **行距**：多行行高 = 字号 × 1.2（与排版缓冲 `line_mult` 一致；cosmic 行盒
        // 按此递增，光标/高亮按视觉行序号 × 行高对齐）。
        let line_h = (style.font_size * TEXT_AREA_LINE_SPACING).max(1.0);
        let content_w = (rect.w - style.padding_x * 2.0).max(0.0);
        let content_rect = Rect::new(rect.x + style.padding_x, rect.y, content_w, rect.h);
        // 视觉框裁剪（**绝对坐标**：`UiDraw.clip` 收集期按绝对逻辑矩形求交；局部
        // content_rect 随容器平移后会错位——高亮/下划线因此被裁到错误区域"看不见"）。
        // 裁剪 = 整个输入框（"完全对应视觉文本框大小"）。
        let box_clip = Rect::new(self.abs_base.x + rect.x, self.abs_base.y + rect.y, rect.w, rect.h);
        let mouse_local_y = self.mouse_logical.y - self.abs_base.y;
        // 排版换行宽：换行模式 = 内容区宽；不换行模式 = 0（不限宽）。
        let wrap_w = if wrap { content_w } else { 0.0 };
        // **视觉行**（自动换行后）：光标/点击/选择/Home-End/↑↓ 全部按它定位，与显示一致。
        let vbuf = self.ensure_text_buf(
            id,
            value,
            style.font_size,
            style.font_family.as_deref(),
            wrap_w,
            TEXT_AREA_LINE_SPACING,
        );
        let vlines = Text::visual_lines(&vbuf);
        // 字节 → 视觉行（半开区间 + 换行边界归属修正，见 edit::vline_of_byte）
        // 注意：不用闭包捕获 `vlines`——编辑改写文本后会重新排版出**新的** vlines，
        // 闭包会一直引用旧绑定导致行号/行区间错位（选择高亮消失、切片 panic）。
        // 鼠标位置 → 光标（视觉行 + 行内列）。**单击与拖选都按"文本坐标"**
        // （视口 y + 垂直滚动）：长内容（自动换行后多行）滚动后点击，行号 = 视口行 +
        // 滚动行——否则点击可视区任意行都会定位到文本前部、光标行随即被滚动跟随
        // 拉回视口顶部（"自动换行后的行鼠标无法定位"）。拖选 + 滚动 = edge-scroll。
        let prev_scroll = self
            .state
            .widgets
            .get(id)
            .map(|w| w.scroll_y)
            .unwrap_or(0.0);
        // 不换行模式：点击列要加**水平滚动**（同单行输入框；换行模式 hscroll = 0）。
        // `scroll_y` / `text_scroll` 均为**物理像素**（内部计算一律物理）→ 文本坐标
        // = 视口坐标 + 物理/scale。
        let hscroll = if wrap {
            0.0
        } else {
            self.state.widgets.get(id).map(|w| w.text_scroll).unwrap_or(0.0)
        };
        // 滚动条条带排除：内容超出可视区（预估，编辑前后高度变化微小）时，右缘
        // `SCROLLBAR_W` 条带属于滚动条——按下不建立文本选择（锚点为空，拖拽分支
        // 由 `sel_anchor.is_some()` 守卫，滚动条自身交互不受影响）。
        let content_h_pre = Text::measure_buffer(&vbuf).y / self.scale;
        let sb_w = if content_h_pre > rect.h + 1.0 && rect.h > 0.0 {
            SCROLLBAR_W
        } else {
            0.0
        };
        let hit_text = hit && (self.mouse_local_x() - rect.x) < rect.w - sb_w;
        let click_caret = if btn.down_edge() && hit_text {
            // 行号按**真实行顶**定位（见 line_row_at_y：逻辑 line_h 每行差 ~0.2px，
            // 长文本累积会错行 / 视图卡住）。
            let row = line_row_at_y(&vlines, mouse_local_y - rect.y + prev_scroll / self.scale, line_h, self.scale);
            let cx = (self.mouse_local_x() - rect.x - style.padding_x + hscroll / self.scale).max(0.0);
            Some(caret_at_visual_click(value, &vlines, row, cx, |s| {
                self.text_size(s, style.font_size, style.font_family.as_deref()).x
            }))
        } else {
            None
        };
        let drag_caret = if btn.pressed() && !btn.down_edge() {
            let row = line_row_at_y(&vlines, mouse_local_y - rect.y + prev_scroll / self.scale, line_h, self.scale);
            let cx = (self.mouse_local_x() - rect.x - style.padding_x + hscroll / self.scale).max(0.0);
            Some(caret_at_visual_click(value, &vlines, row, cx, |s| {
                self.text_size(s, style.font_size, style.font_family.as_deref()).x
            }))
        } else {
            None
        };
        // 记录帧首光标：仅"光标移动"（打字/方向键/点击/拖选）时做光标跟随——
        // 滚轮滚动不移动光标 → 不跟随（滚轮自由滚动、光标可滚出视图，不被拉回）。
        let prev_caret = self.state.widgets.get(id).map(|w| w.caret);
        let caret_est = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let ev = update_interact(ws, hit, btn);
            if ev.pressed {
                self.any_pressed = true;
                // 仅按下在**文本区**（滚动条条带外）时建立文本选择/焦点：
                // 按下滚动条由滚动条自身交互处理（anchor 保持 None）。
                if hit_text {
                    self.press_claimed = true;
                    self.state.focused = Some(id.to_owned());
                    if let Some(c) = click_caret {
                        ws.caret = c;
                    }
                    // 双击检测（同单行输入框）：第二击选中"词"并进入词模式。
                    let is_dbl = {
                        let (pf, pp) = (ws.last_click_frame, ws.last_click_pos);
                        ws.last_click_frame = self.state.frame;
                        ws.last_click_pos = self.mouse_logical;
                        pf != 0
                            && self.state.frame.wrapping_sub(pf) <= DOUBLE_CLICK_FRAMES
                            && (self.mouse_logical - pp).length() < DOUBLE_CLICK_DIST
                    };
                    ws.press_mouse = Some(self.mouse_screen.round());
                    if is_dbl {
                        let (w0, w1) = crate::edit::word_range(value, ws.caret);
                        ws.sel_anchor = Some(w0);
                        ws.caret = w1;
                        ws.sel_word = true;
                    } else {
                        ws.sel_word = false;
                        ws.sel_anchor = Some(ws.caret);
                    }
                }
            } else if ws.pressed && btn.pressed() && ws.sel_anchor.is_some() {
                // 拖拽选择：**位移 ≥ 3 物理像素**才扩展选择（单击微动不误选）；
                // 光标跟随鼠标（**即使拖出输入框**——edge-scroll 持续滚动）。
                self.press_claimed = true;
                let moved = ws
                    .press_mouse
                    .map(|p| (self.mouse_screen.round() - p).length_squared() >= 9.0)
                    .unwrap_or(false);
                if moved {
                    if let Some(c) = drag_caret {
                        if ws.sel_word {
                            // 词模式（双击后拖拽）：按词边界扩散选择
                            let anchor = ws.sel_anchor.unwrap_or(c);
                            ws.caret = crate::edit::extend_word_caret(value, anchor, c);
                        } else {
                            ws.caret = c;
                        }
                    }
                }
            }
            if ev.released {
                // 纯点击（无位移）→ 清理 anchor（无实际选择），避免残留 anchor 在
                // 后续无 Shift 方向键移动时"突然变成多选"。
                if ws.sel_anchor == Some(ws.caret) {
                    ws.sel_anchor = None;
                }
                // 释放后退出词模式（选择保留；下次单击/双击重开）。
                ws.sel_word = false;
            }
            let focused = self.state.focused.as_deref() == Some(id);
            if focused {
                let in_ime_compose =
                    self.keyboard.get_ime_preedit().is_some_and(|p| !p.is_empty());
                let ime_owns_keys = in_ime_compose || self.state.ime_composing;
                // 编辑状态机（多行）：剪贴板保留换行、选择替换（Enter 计入）、
                // IME 上屏、普通字符（过滤 '\n'）、退格/删除。
                crate::edit::apply_frame_edits(&self.keyboard, ws, value, true, ime_owns_keys);
                // 换行（Enter；TextArea 语义：插入 '\n'，Esc 失焦）——选择替换已由
                // apply_frame_edits 在 Enter 计入 edit_pending 时先消费。
                if self.keyboard.get(KeyCode::Enter).down_edge() {
                    insert_char_at(value, ws.caret, '\n');
                    ws.caret = (ws.caret + 1).min(value.chars().count());
                }
                // Shift + 方向键/Home/End：扩展选择
                let shift = self.keyboard.get(KeyCode::ShiftLeft).pressed()
                    || self.keyboard.get(KeyCode::ShiftRight).pressed();
                let shift_start = |ws: &mut WidgetState| {
                    if shift && ws.sel_anchor.is_none() {
                        ws.sel_anchor = Some(ws.caret);
                    }
                };
                // 无 Shift 的方向键：**取消选择**（否则 Shift 多选后松开再按 ←/→/↑/↓
                // 残留 anchor 会继续扩展选择）。
                let shift_clear = |ws: &mut WidgetState| {
                    if !shift {
                        ws.sel_anchor = None;
                    }
                };
                let shift_shrink = |ws: &mut WidgetState| {
                    if ws.sel_anchor == Some(ws.caret) {
                        ws.sel_anchor = None;
                    }
                };
                // ←/→：字符移动（Shift 扩展，共用状态机）。
                if self.keyboard.get(KeyCode::ArrowLeft).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, -1, shift);
                }
                if self.keyboard.get(KeyCode::ArrowRight).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, 1, shift);
                }
                if self.keyboard.get(KeyCode::ArrowUp).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    shift_clear(ws);
                    // 跨**视觉行**（保持列；列 = 相对行首的 char 数）
                    let cur_byte = char_to_byte(value, ws.caret);
                    let li = vline_of_byte(&vlines, cur_byte);
                    let col = byte_to_char(value, cur_byte) - byte_to_char(value, vlines[li].byte_start);
                    let tgt = li.saturating_sub(1);
                    let line = &vlines[tgt];
                    // 编辑同帧改写文本后 vlines 可能过期：字节边界对齐防 panic
                    // （短暂错位次帧重排后自愈）。
                    let ls = value.floor_char_boundary(line.byte_start);
                    let ltxt = safe_line_slice(value, line);
                    let col = col.min(ltxt.chars().count());
                    ws.caret = byte_to_char(value, ls + char_to_byte(ltxt, col));
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::ArrowDown).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    shift_clear(ws);
                    let cur_byte = char_to_byte(value, ws.caret);
                    let li = vline_of_byte(&vlines, cur_byte);
                    let col = byte_to_char(value, cur_byte) - byte_to_char(value, vlines[li].byte_start);
                    let tgt = (li + 1).min(vlines.len().saturating_sub(1));
                    let line = &vlines[tgt];
                    let ls = value.floor_char_boundary(line.byte_start);
                    let ltxt = safe_line_slice(value, line);
                    let col = col.min(ltxt.chars().count());
                    ws.caret = byte_to_char(value, ls + char_to_byte(ltxt, col));
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::Home).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    shift_clear(ws);
                    let li = vline_of_byte(&vlines, char_to_byte(value, ws.caret));
                    ws.caret =
                        byte_to_char(value, value.floor_char_boundary(vlines[li].byte_start));
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::End).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    shift_clear(ws);
                    let li = vline_of_byte(&vlines, char_to_byte(value, ws.caret));
                    ws.caret =
                        byte_to_char(value, value.floor_char_boundary(vlines[li].byte_end.min(value.len())));
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::Escape).down_edge() {
                    self.state.focused = None;
                }
            }
            (focused, ws.caret)
        };
        let (focused, caret) = caret_est;
        // 光标是否移动（编辑/点击/拖选/方向键）；false = 纯滚轮滚动 → 不做光标跟随
        let caret_moved = Some(caret) != prev_caret;
        // 编辑（粘贴/打字/IME/退格/删除）可能已改写 `value` → **重新排版**：
        // 视觉行字节区间必须对齐新文本，否则显示/光标定位用旧区间切片会落在
        // 多字节字符中间（如粘贴中文时 `&value[a..b]` panic）。`ws` 已随
        // `caret_est` 结束释放，可安全 `&mut self` 重排。
        let vbuf = self.ensure_text_buf(
            id,
            value,
            style.font_size,
            style.font_family.as_deref(),
            wrap_w,
            TEXT_AREA_LINE_SPACING,
        );
        let vlines = Text::visual_lines(&vbuf);
        // **IME 组合内联融入**（多行）：显示串 = value[..caret] + preedit + value[caret..]，
        // 按内容宽度**重新换行**——组合后的后续文本右移/换行而非被盖住；组合较长时
        // 垂直滚动跟随组合光标。无组合时回落 value（零开销路径）。
        // IME 组合串先拷出（owned）：闭包内要 &mut self（text_size 测量），
        // 与自持快照字段 self.keyboard 的借用不能共存。
        let preedit = self.keyboard.get_ime_preedit().map(|p| p.to_owned());
        let preedit_caret = self.keyboard.get_ime_preedit_caret();
        let composed: Option<(String, std::ops::Range<usize>, f32, usize)> = if focused {
            preedit
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let insert_b = char_to_byte(value, caret);
                    let disp = format!("{}{}{}", &value[..insert_b], p, &value[insert_b..]);
                    let w = self.text_size(&p, style.font_size, style.font_family.as_deref()).x;
                    let caret_b = preedit_caret
                        .map(|b| p.floor_char_boundary(b.min(p.len())))
                        .unwrap_or(p.len());
                    (disp, insert_b..insert_b + p.len(), w, insert_b + caret_b)
                })
        } else {
            None
        };
        // 组合时：显示串重新排版（换行随组合变化）；否则复用 value 的 vbuf/vlines。
        let (draw_buf, draw_vlines, draw_disp): (Arc<Buffer>, Vec<VisualLine>, Option<String>) =
            match &composed {
                Some((disp, ..)) => {
                    let b = self.ensure_text_buf(
                        id,
                        disp,
                        style.font_size,
                        style.font_family.as_deref(),
                        wrap_w,
                        TEXT_AREA_LINE_SPACING,
                    );
                    (b.clone(), Text::visual_lines(&b), Some(disp.clone()))
                }
                None => (vbuf.clone(), vlines.clone(), None),
            };
        // 光标所在**视觉行** → 光标 x / y（基于**显示串**视觉行；y = 行序号 × 行高）。
        let draw_text: &str = draw_disp.as_deref().unwrap_or(value);
        let caret_disp = composed
            .as_ref()
            .map(|(_, _, _, c)| *c)
            .unwrap_or_else(|| char_to_byte(value, caret));
        let caret_line = crate::edit::vline_of_byte(&draw_vlines, caret_disp);
        let caret_x = {
            let line = &draw_vlines[caret_line];
            let end = caret_disp.min(line.byte_end).max(line.byte_start);
            let prefix = &draw_text[line.byte_start..end];
            self.text_size(prefix, style.font_size, style.font_family.as_deref()).x
        };
        // 不换行模式：**水平滚动**跟随光标（同单行输入框；光标右侧保留 8 逻辑像素）；
        // **水平滚轮（触控板）优先**——自由滚动（可把光标滚出视图，**仅鼠标在框内
        // 时**），否则仅光标移动时跟随。换行模式：无水平滚动（text_dx = 0）。
        let (wx, wy) = self.mouse.get_mouse_wheel_delta();
        let text_dx = if wrap {
            0.0
        } else {
            let line = &draw_vlines[caret_line];
            let ls = value.floor_char_boundary(line.byte_start);
            let le = value.floor_char_boundary(line.byte_end.min(value.len()));
            let line_w = if ls < le {
                self.text_size(&draw_text[ls..le], style.font_size, style.font_family.as_deref())
                    .x
            } else {
                0.0
            };
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            if hit && wx != 0.0 {
                // 水平滚轮（触控板）：自由滚动（可超出光标），clamp 到内容宽
                let max_h_px = ((line_w - content_w).max(0.0) * self.scale).round();
                ws.text_scroll = (ws.text_scroll - (wx as f32 * 40.0 * self.scale).round())
                    .clamp(0.0, max_h_px);
            } else if caret_moved {
                // 光标移动（打字/方向键/点击/拖选）→ 跟随；滚轮滚动不跟随
                ws.text_scroll = scroll_follow_caret(
                    ws.text_scroll,
                    caret_x * self.scale,
                    content_w * self.scale,
                    line_w * self.scale,
                    8.0 * self.scale,
                );
            }
            -ws.text_scroll / self.scale
        };
        // 光标 y = **真实行顶**（`VisualLine.top` **物理像素**）——与渲染行网格
        // 完全一致；`行号 × line_h` 每行差 ~0.2px，长文本累积后光标/滚动目标漂移
        // （视图卡在短于真正底部的纵轴范围内）。`caret_y`（逻辑）供绘制用。
        let caret_y_px = draw_vlines[caret_line].top;
        let caret_y = caret_y_px / self.scale;
        // 垂直滚动：滚轮 + 光标跟随。内容高用**实际排版缓冲**（组合时 = 显示串缓冲）。
        let content_h = Text::measure_buffer(&draw_buf).y / self.scale;
        let max_scroll_px = ((content_h - rect.h).max(0.0) * self.scale).round();
        let scroll = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            // 滚轮（**仅鼠标在框内时**——指针离开输入框后不再滚动；拖选中不滚轮）。
            if hit && !ws.pressed {
                if wy != 0.0 {
                    ws.scroll_y = (ws.scroll_y - (wy as f32 * 30.0 * self.scale).round())
                        .clamp(0.0, max_scroll_px);
                }
            }
            // **拖选 edge-scroll**：鼠标越出可视区上下缘时按越出量持续滚动
            // （光标随后一帧按新滚动重新定位 → 选择持续延伸，直至文本两端）。
            if ws.pressed {
                let y = mouse_local_y - rect.y;
                if y > rect.h {
                    ws.scroll_y = (ws.scroll_y + (y - rect.h) * self.scale).min(max_scroll_px);
                } else if y < 0.0 {
                    ws.scroll_y = (ws.scroll_y + y * self.scale).max(0.0);
                }
            }
            // 光标跟随（仅**光标移动**时，如打字/方向键/点击/拖选）：光标行滚出
            // 可视区时调整——滚轮滚动不移动光标 → 不跟随（滚轮自由滚动、光标可
            // 滚出视图，且不被下一帧拉回）。
            if caret_moved {
                if caret_y_px < ws.scroll_y {
                    ws.scroll_y = caret_y_px;
                } else if caret_y_px + line_h * self.scale > ws.scroll_y + rect.h * self.scale {
                    ws.scroll_y =
                        (caret_y_px + line_h * self.scale - rect.h * self.scale)
                            .min(max_scroll_px);
                }
            }
            ws.scroll_y = ws.scroll_y.clamp(0.0, max_scroll_px);
            ws.scroll_y
        };
        // 绘制
        let depth = self.depth;
        let win = self.cur_win;
        let elem = self.seq + 1;
        let border = if focused { style.border_focus } else { style.border };
        // **Clip 子沙箱**（控件内）：强制裁剪层 = 外层强制 ∩ 输入框矩形。
        // 光标 / 高亮 / 文本命令自动受其裁剪（滚出视图不画出框）。
        let saved_clip = self.clip;
        self.clip = clip_for_view(saved_clip, box_clip, ViewMode::Clip);
        self.push_panel_like(rect, style.bg, border, style.border_w, style.radius, elem);
        // 选择高亮（逐**视觉行**；x = 行内前缀宽度，y = 视觉行序号 × 行高——与显示一致）
        if let Some((lo, hi)) = sel_range(
            self.state.widgets.get(id).and_then(|w| w.sel_anchor),
            caret,
        ) {
            let lo_byte = char_to_byte(value, lo);
            let hi_byte = char_to_byte(value, hi);
            // 用**重新排版后**的 vlines（编辑后行区间才与当前文本一致，否则高亮
            // 错位/消失；见上方重排注释）。
            let lo_li = vline_of_byte(&vlines, lo_byte);
            let hi_li = vline_of_byte(&vlines, hi_byte);
            for li in lo_li..=hi_li {
                let line = &vlines[li];
                let ls = line.byte_start.min(value.len());
                let le = line.byte_end.min(value.len());
                let c0b = if li == lo_li { lo_byte.max(ls).min(le) } else { ls };
                let c1b = if li == hi_li { hi_byte.max(ls).min(le) } else { le };
                if c1b <= c0b {
                    continue;
                }
                let x0 = self
                    .text_size(&value[ls..c0b], style.font_size, style.font_family.as_deref())
                    .x;
                let x1 = self
                    .text_size(&value[ls..c1b], style.font_size, style.font_family.as_deref())
                    .x;
                // y 随垂直滚动上移（-scroll/scale）；clip = 输入框强制层（选择高亮
                // 受裁剪，不溢出输入框 / 外层滚动容器）。
                // 行顶用真实 `VisualLine.top`（与文本行网格一致，长文本不漂移）。
                let sel_rect = Rect::new(
                    content_rect.x + x0 + text_dx,
                    rect.y + vlines[li].top / self.scale - scroll / self.scale,
                    (x1 - x0).max(0.0),
                    line_h,
                );
                if sel_rect.w > 0.0 {
                    let seq = self.next_seq();
                    self.queue.push(UiDraw {
                        depth,
                        seq,
                        win,
                        elem,
                        rect: sel_rect,
                        clip: self.clip,
                        kind: DrawKind::Solid(style.sel_bg),
                    });
                }
            }
        }
        // 文本（换行 + 垂直滚动；clip 相对文本块：上缘 = scroll/scale，高 = 可视区；
        // 缓冲控件自持——组合时用显示串缓冲，否则复用 `vbuf`）。
        let seq = self.next_seq();
        self.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            Rect::new(
                content_rect.x + text_dx,
                content_rect.y - scroll / self.scale,
                content_w,
                rect.h,
            ),
            Arc::from(draw_text),
            style.font_size,
            style.fg,
            TextAlign::Left,
            // 多行编辑：**顶对齐**（行盒顶 = 内容区顶），与光标/点击的 TopLeft 定位一致
            TextVAlign::Top,
            style.font_family.clone(),
            // 内层裁剪相对**移动后**的文本 rect（含水平滚动）：窗口固定在视觉框
            Some(Rect::new(
                -style.padding_x - text_dx,
                scroll / self.scale,
                rect.w,
                rect.h,
            )),
            self.clip,
            Some(draw_buf),
        ));
        // **组合下划线**：覆盖组合文本段（显示串 `[span]`，可能跨视觉行），
        // 受内容区裁剪。组合文本已融入显示串（后续文本右移/换行），无需单独绘制文字。
        if let Some((disp, span, _, _)) = &composed {
            let s_li = crate::edit::vline_of_byte(&draw_vlines, span.start);
            let e_li = crate::edit::vline_of_byte(&draw_vlines, span.end.saturating_sub(1));
            for li in s_li..=e_li {
                let line = &draw_vlines[li];
                let ls = line.byte_start.min(disp.len());
                let le = line.byte_end.min(disp.len());
                let x0b = span.start.max(ls).min(le);
                let x1b = span.end.max(ls).min(le);
                if x1b <= x0b {
                    continue;
                }
                let x0 = self
                    .text_size(&disp[ls..x0b], style.font_size, style.font_family.as_deref())
                    .x;
                let x1 = self
                    .text_size(&disp[ls..x1b], style.font_size, style.font_family.as_deref())
                    .x;
                let ul = Rect::new(
                    content_rect.x + x0 + text_dx,
                    rect.y + draw_vlines[li].top / self.scale + line_h - 3.0
                        - scroll / self.scale,
                    (x1 - x0).max(0.0),
                    2.0,
                );
                if ul.w > 0.0 && ul.h > 0.0 {
                    let useq = self.next_seq();
                    self.queue.push(UiDraw {
                        depth,
                        seq: useq,
                        win,
                        elem,
                        rect: ul,
                        clip: self.clip,
                        kind: DrawKind::Solid(style.preedit),
                    });
                }
            }
        }
        // **IME 候选框定位**：跟随组合光标（窗口客户区物理像素；无组合 = 输入光标）。
        if focused {
            let ime_x = ((self.abs_base.x + content_rect.x + caret_x + text_dx) * self.scale) as i32;
            let ime_y =
                ((self.abs_base.y + rect.y) * self.scale + caret_y_px - scroll) as i32;
            let ime_w = (rect.w * self.scale).max(1.0) as u32;
            let ime_h = (line_h * self.scale).max(1.0) as u32;
            let _ = self.window.set_ime_cursor_area(
                PhysicalPosition::new(ime_x, ime_y),
                PhysicalSize::new(ime_w, ime_h),
            );
        }
        // 光标（组合时 = 显示串内的组合光标）
        if focused && self.state.caret_blink_on() {
            let caret_rect = Rect::new(
                content_rect.x + caret_x + text_dx,
                rect.y + caret_y - scroll / self.scale,
                1.0,
                line_h,
            );
            let seq = self.next_seq();
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: caret_rect,
                clip: self.clip,
                kind: DrawKind::Caret {
                    color: style.caret,
                    width: 1.0,
                },
            });
        }
        // **垂直滚动条**（内容超出可视区时显示；拖 thumb / 点轨道翻页）——
        // 复用 `scroll_at` 的滚动条（物理像素偏移；`elem` = 本控件 → 覆盖在文本
        // 之上）；状态 ID 独立（`{id}::vbar`），拖拽与文本选择互不干扰。
        if content_h > rect.h + 1.0 && rect.h > 0.0 {
            let new_px = self.scrollbar(
                &format!("{id}::vbar"),
                &Rect::new(rect.x, rect.y, rect.w, rect.h),
                rect.h,
                content_h,
                scroll,
                max_scroll_px,
                self.clip,
                elem,
            );
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            ws.scroll_y = new_px;
        }
        // 退出 Clip 子沙箱（恢复外层强制裁剪层）。
        self.clip = saved_clip;
    }
}

// ─── 对齐转换 ───────────────────────────────────────────────────

impl From<Align> for TextAlign {
    fn from(a: Align) -> Self {
        match a {
            Align::Left => TextAlign::Left,
            Align::Right => TextAlign::Right,
            _ => TextAlign::Center,
        }
    }
}

impl From<TextAlign> for Align {
    fn from(a: TextAlign) -> Self {
        match a {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        }
    }
}

// ─── 单元测试（无 GPU） ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{remove_at, remove_before};

    #[test]
    fn insert_char_at_handles_caret() {
        let mut s = String::from("abc");
        insert_char_at(&mut s, 1, 'X');
        assert_eq!(s, "aXbc");
        insert_char_at(&mut s, 0, '!');
        assert_eq!(s, "!aXbc");
        insert_char_at(&mut s, 99, 'z');
        assert_eq!(s, "!aXbcz", "caret 越界 clamp 到末尾");
        // 多字节字符按 char 索引
        let mut s = String::from("中文ab");
        insert_char_at(&mut s, 2, '字');
        assert_eq!(s, "中文字ab");
    }

    #[test]
    fn remove_before_and_at() {
        let mut s = String::from("abc");
        assert_eq!(remove_before(&mut s, 2), 1);
        assert_eq!(s, "ac");
        remove_at(&mut s, 0);
        assert_eq!(s, "c");
        // 多字节
        let mut s = String::from("中文");
        assert_eq!(remove_before(&mut s, 2), 1);
        assert_eq!(s, "中", "caret=2 删除前一个字符（'文'）");
        remove_at(&mut s, 0);
        assert_eq!(s, "");
    }

    #[test]
    fn text_align_conversion_roundtrip() {
        assert_eq!(Align::from(TextAlign::Left), Align::Left);
        assert_eq!(Align::from(TextAlign::Center), Align::Center);
        assert_eq!(Align::from(TextAlign::Right), Align::Right);
        assert_eq!(TextAlign::from(Align::Left), TextAlign::Left);
        assert_eq!(TextAlign::from(Align::Center), TextAlign::Center);
        assert_eq!(TextAlign::from(Align::Right), TextAlign::Right);
        // 其他 Align（Justified/End）归入 Center
        assert_eq!(TextAlign::from(Align::Justified), TextAlign::Center);
        assert_eq!(TextAlign::from(Align::End), TextAlign::Center);
    }

    #[test]
    fn draw_kind_group_graphic_before_text() {
        // 同一 layer 内：图形（Solid/Border/Caret）分组 0，文字（Text）分组 1
        assert_eq!(DrawKind::Solid(Color::WHITE).group(), 0);
        assert_eq!(
            DrawKind::Border {
                color: Color::WHITE,
                width: 1.0,
            }
            .group(),
            0
        );
        assert_eq!(DrawKind::Caret { color: Color::WHITE, width: 1.0 }.group(), 0);
        assert_eq!(
            DrawKind::Text {
                text: "x".into(),
                size: 14.0,
                color: Color::WHITE,
                align: TextAlign::Left,
                family: None,
                valign: TextVAlign::Center,
                clip: None,
                buf: None,
            }
            .group(),
            1
        );
        // finish 排序键顺序：win → depth → elem → group → seq
        let mut cmds = vec![
            UiDraw {
                depth: 0,
                seq: 2,
                win: 0,
                elem: 1,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                clip: None,                kind: DrawKind::Text {                    text: "t".into(),
                    size: 14.0,
                    color: Color::WHITE,
                    align: TextAlign::Left,
                    family: None,
                    valign: TextVAlign::Center,
                    clip: None,
                    buf: None,
                },
            },
            UiDraw {
                depth: 0,
                seq: 1,
                win: 0,
                elem: 1,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                kind: DrawKind::Solid(Color::WHITE),
                clip: None,
            },
            UiDraw {
                depth: 0,
                seq: 3,
                win: 1,
                elem: 2,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                kind: DrawKind::Solid(Color::WHITE),
                clip: None,
            },
            UiDraw {
                depth: 0,
                seq: 4,
                win: 1,
                elem: 2,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                clip: None,                kind: DrawKind::Text {                    text: "w".into(),
                    size: 14.0,
                    color: Color::WHITE,
                    align: TextAlign::Left,
                    family: None,
                    valign: TextVAlign::Center,
                    clip: None,
                    buf: None,
                },
            },
        ];
        cmds.sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
        // 期望：win0 图形(elem1) → win0 文字(elem1) → win1 图形(elem2) → win1 文字(elem2)
        let order: Vec<u32> = cmds.iter().map(|d| d.seq).collect();
        assert_eq!(order, vec![1, 2, 3, 4], "窗口 z 升序；元素内图形先、文字后");
    }

    #[test]
    fn submit_order_graphics_before_text_per_window() {
        // 回归：窗口图形/文字绘制顺序抖动——同一窗口内图形组（白纹理 / 圆角 / 渐变）
        // 必须先于字形文字组，且跨帧稳定（不随 HashMap 迭代顺序 / 纹理 uid 分配变化）。
        let g = GROUP_GRAPHIC;
        let t = GROUP_TEXT;
        // 模拟 finish() 的提交列表：win=0 非窗口内容 + 窗口 1/2（各含图形组与文字组）。
        // 故意用乱序 + 反序 uid 输入（如 HashMap 迭代顺序）。
        let mut groups = vec![
            (2u32, t, 3u64),
            (1u32, t, 3u64),
            (0u32, t, 2u64),
            (2u32, g, 2u64),
            (0u32, g, 1u64),
            (1u32, g, 1u64),
            (1u32, g, 2u64),
        ];
        groups.sort_by_key(|&(w, gr, tex)| (w, gr, tex));
        // 期望：win 升序；同一窗口内图形组先于文字组；组内按纹理 uid 稳定。
        assert_eq!(
            groups,
            vec![
                (0, g, 1),
                (0, t, 2),
                (1, g, 1),
                (1, g, 2),
                (1, t, 3),
                (2, g, 2),
                (2, t, 3),
            ],
            "win 升序 + 窗口内图形先于文字（含程序化纹理），跨帧确定"
        );
    }

    #[test]
    fn drag_needs_movement_so_clicks_work() {
        // 回归：窗口/可拖拽面板内 CheckBox / 输入框失效——按下即激活拖拽导致
        // `drag_panel` 在释放帧抑制子控件，点击被吞。修复：位移 ≥ DRAG_ACTIVATE_PX
        // 才视为拖拽；纯点击（无位移 / 微小抖动）不激活。
        let press = Vec2::new(100.0, 200.0); // 按下基准（物理像素，已取整）
        // 静止（纯点击）：未激活
        assert!(!drag_moved(press, Some(press)), "按下无位移不激活拖拽");
        // 微小抖动（< 阈值）：仍视为点击
        assert!(!drag_moved(press + Vec2::new(1.0, 1.0), Some(press)), "1px 抖动不激活");
        assert!(!drag_moved(press + Vec2::new(2.0, 2.0), Some(press)), "~2.8px 位移不激活");
        // 恰好达到阈值
        assert!(
            drag_moved(press + Vec2::new(3.0, 0.0), Some(press)),
            "3px 水平位移激活拖拽"
        );
        // 明显位移：激活
        assert!(
            drag_moved(press + Vec2::new(0.0, -8.0), Some(press)),
            "8px 竖直位移激活拖拽"
        );
        // 无按下基准（如窗口未命中时）：不激活
        assert!(!drag_moved(press, None), "无按下基准不激活");
    }

    #[test]
    fn text_block_placement_is_integer_with_line_box_top() {
        // 回归：UI 文本亚像素模糊——垂直行盒对齐偏移经**整数运算**后，
        // `block_tl = anchor + off` 的两侧操作数均为整数（浮点整数不变量）：
        // 小数（行盒顶 / 奇数宽的一半）只在 `round` 边界被一次性消化，
        // 不流入加法链 → 无误差累加、字形四边形角点恒为整数屏幕像素。
        // 数据来自 14px 排版实测：行高 16.8→content_h 17，"a" 行盒顶 = -7.2
        // （rjw_text 收集期取整为 -7.0）。
        let anchor = Vec2::new(100.0, 200.0);
        let content = Vec2::new(14.0, 17.0);
        let first_line_top = -7.0;
        // 左对齐标签：block = anchor + off，两项均为整数
        let block = anchor + text_block_offset(TextAlign::Left, TextVAlign::Center, content, first_line_top);
        assert_eq!(block.x.fract(), 0.0, "标签块 x 必须为整数像素，实际 {}", block.x);
        assert_eq!(block.y.fract(), 0.0, "标签块 y 必须为整数像素，实际 {}", block.y);
        // 行盒中心对准锚点（±0.5px 量化，奇数 content_h 的固有半像素）：
        // block.y + first_line_top + content_h/2 ≈ anchor.y
        let center = block.y + first_line_top + content.y * 0.5;
        assert!(
            (center - anchor.y).abs() <= 0.5,
            "行盒中心应贴近锚点，偏差 {}",
            center - anchor.y
        );
        // 居中按钮 + 奇数物理宽（21px，如 DPI 1.5 下）：水平对齐亦为整数
        let btn = anchor + text_block_offset(TextAlign::Center, TextVAlign::Center, Vec2::new(21.0, 17.0), first_line_top);
        assert_eq!(btn.x.fract(), 0.0, "按钮块 x 必须为整数像素，实际 {}", btn.x);
        assert_eq!(btn.y.fract(), 0.0, "按钮块 y 必须为整数像素，实际 {}", btn.y);
    }

    #[test]
    fn overlapping_elements_follow_record_order() {
        // 元素重叠层级正确性：后录元素（elem 大）覆盖先录元素（elem 小），
        // **即使后录元素是图形（group 0）、先录元素是文字（group 1）**——
        // 元素序优先于图形/文字分组。
        let mut cmds = vec![
            // 元素 A（先录）：文字
            UiDraw {
                depth: 1,
                seq: 1,
                win: 1,
                elem: 1,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                clip: None,                kind: DrawKind::Text {                    text: "a".into(),
                    size: 14.0,
                    color: Color::WHITE,
                    align: TextAlign::Left,
                    family: None,
                    valign: TextVAlign::Center,
                    clip: None,
                    buf: None,
                },
            },
            // 元素 B（后录）：图形——应覆盖 A 的文字
            UiDraw {
                depth: 1,
                seq: 2,
                win: 1,
                elem: 2,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                kind: DrawKind::Solid(Color::WHITE),
                clip: None,
            },
        ];
        cmds.sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
        let order: Vec<u32> = cmds.iter().map(|d| d.seq).collect();
        assert_eq!(order, vec![1, 2], "后录元素（B 图形）应覆盖先录元素（A 文字）");
        // 元素内：同一 elem 的图形先于文字
        let mut inner = vec![
            UiDraw {
                depth: 1,
                seq: 3,
                win: 1,
                elem: 3,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                clip: None,                kind: DrawKind::Text {                    text: "x".into(),
                    size: 14.0,
                    color: Color::WHITE,
                    align: TextAlign::Left,
                    family: None,
                    valign: TextVAlign::Center,
                    clip: None,
                    buf: None,
                },
            },
            UiDraw {
                depth: 1,
                seq: 2,
                win: 1,
                elem: 3,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                kind: DrawKind::Solid(Color::WHITE),
                clip: None,
            },
        ];
        inner.sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
        let order: Vec<u32> = inner.iter().map(|d| d.seq).collect();
        assert_eq!(order, vec![2, 3], "元素内：图形(seq2)先画，文字(seq3)后画（文字覆盖图形）");
    }

    #[test]
    fn capturing_text_reflects_focus() {
        let mut st = UiState::new();
        assert!(!st.capturing_text());
        st.focused = Some("name".to_owned());
        assert!(st.capturing_text(), "有输入焦点时应捕获键盘");
        st.focused = None;
        assert!(!st.capturing_text());
    }

    #[test]
    fn anchor_pos_covers_all_corners_and_clamps() {
        let vp = Vec2::new(1280.0, 720.0);
        let size = Vec2::new(200.0, 20.0);
        let m = Vec2::new(16.0, 16.0);
        // 四角 + 边距
        assert_eq!(Ui::anchor_pos_in(vp, Anchor::TopLeft, size, m), Vec2::new(16.0, 16.0));
        assert_eq!(
            Ui::anchor_pos_in(vp, Anchor::TopRight, size, m),
            Vec2::new(1280.0 - 16.0 - 200.0, 16.0)
        );
        assert_eq!(
            Ui::anchor_pos_in(vp, Anchor::BottomLeft, size, m),
            Vec2::new(16.0, 720.0 - 16.0 - 20.0)
        );
        assert_eq!(
            Ui::anchor_pos_in(vp, Anchor::BottomRight, size, m),
            Vec2::new(1280.0 - 16.0 - 200.0, 720.0 - 16.0 - 20.0)
        );
        // 居中（上下 / 左右边距对称）
        assert_eq!(
            Ui::anchor_pos_in(vp, Anchor::Center, size, m),
            Vec2::new(16.0 + (1280.0 - 32.0 - 200.0) * 0.5, 16.0 + (720.0 - 32.0 - 20.0) * 0.5)
        );
        // 底中对齐
        assert_eq!(
            Ui::anchor_pos_in(vp, Anchor::BottomCenter, size, m),
            Vec2::new(16.0 + (1280.0 - 32.0 - 200.0) * 0.5, 720.0 - 16.0 - 20.0)
        );
        // 内容超视口（+边距）→ clamp 贴边（左上/左下角，不产生负坐标；
        // 内容本身比视口宽时无法完全放入，贴边即可）
        let big = Vec2::new(2000.0, 1000.0);
        let p = Ui::anchor_pos_in(vp, Anchor::BottomRight, big, m);
        assert!(p.x >= 0.0 && p.y >= 0.0, "不产生负坐标");
        assert_eq!(Ui::anchor_pos_in(vp, Anchor::TopLeft, big, m), Vec2::new(16.0, 16.0), "左上角不 clamp");
    }

    #[test]
    fn topmost_win_never_occluded() {
        use crate::hit::window_occluded;
        // 任意普通窗口叠放时，置顶哨兵 z（浮层）恒不被遮挡 → 浮层始终可交互。
        let rects = [
            (1u32, Rect::new(0.0, 0.0, 100.0, 100.0)),
            (5u32, Rect::new(0.0, 0.0, 100.0, 100.0)),
        ];
        assert!(
            !window_occluded(WIN_TOPMOST, Vec2::new(10.0, 10.0), rects.iter().copied()),
            "置顶哨兵 z 恒不被遮挡（IME 候选框 / 下拉浮层可交互）"
        );
        // 对照：普通窗口仍被更高 z 遮挡
        assert!(window_occluded(1, Vec2::new(10.0, 10.0), rects.iter().copied()));
    }

    #[test]
    fn pos_chain_resolves_by_priority() {
        use std::collections::HashMap;
        // 责任链解析顺序：脚本（优先级降序）→ 内置拖拽状态（优先级 0）→ 调用者 pos 兜底
        let mut chain: Vec<(i32, PosLink)> = vec![(0, PosLink::Drag)];
        chain.push((10, PosLink::Script(Box::new(|_| Some(Vec2::new(1.0, 1.0))))));
        chain.push((-10, PosLink::Script(Box::new(|_| Some(Vec2::new(2.0, 2.0))))));
        chain.sort_by(|a, b| b.0.cmp(&a.0)); // 高优先级在前（pos_handler 内部同款排序）
        let mut panel_pos = HashMap::new();
        panel_pos.insert("w".to_owned(), Vec2::new(3.0, 3.0));
        // 高优先级脚本胜出
        assert_eq!(
            resolve_pos_link(&chain, &panel_pos, "w", Vec2::ZERO),
            Vec2::new(1.0, 1.0)
        );
        // 去掉高优先级脚本 → 内置拖拽状态（优先级 0 > -10）先被询问 → 用户拖拽优先
        chain.retain(|(p, _)| *p != 10);
        chain.sort_by(|a, b| b.0.cmp(&a.0));
        assert_eq!(
            resolve_pos_link(&chain, &panel_pos, "w", Vec2::ZERO),
            Vec2::new(3.0, 3.0),
            "拖拽状态优先级 0 高于负优先级脚本 → 用户拖过就赢过动画"
        );
        // 负优先级脚本在用户**未拖过**时兜底提供位置（动画"填空"语义）
        assert_eq!(
            resolve_pos_link(&chain, &panel_pos, "not_dragged", Vec2::ZERO),
            Vec2::new(2.0, 2.0)
        );
        // 全部脚本落空 → 内置用户拖拽状态胜出
        chain.retain(|(p, _)| *p != -10);
        chain.sort_by(|a, b| b.0.cmp(&a.0));
        assert_eq!(
            resolve_pos_link(&chain, &panel_pos, "w", Vec2::ZERO),
            Vec2::new(3.0, 3.0)
        );
        // 用户未拖过 → 调用者传入 pos 兜底
        assert_eq!(
            resolve_pos_link(&chain, &panel_pos, "other", Vec2::new(9.0, 9.0)),
            Vec2::new(9.0, 9.0)
        );
        // 脚本按 id 选择性响应：返回 None 即交还下一环
        let mut chain2: Vec<(i32, PosLink)> = vec![(0, PosLink::Drag)];
        chain2.push((5, PosLink::Script(Box::new(|id| {
            if id == "scripted" {
                Some(Vec2::new(7.0, 7.0))
            } else {
                None
            }
        }))));
        chain2.sort_by(|a, b| b.0.cmp(&a.0));
        assert_eq!(
            resolve_pos_link(&chain2, &panel_pos, "scripted", Vec2::ZERO),
            Vec2::new(7.0, 7.0)
        );
        assert_eq!(
            resolve_pos_link(&chain2, &panel_pos, "w", Vec2::ZERO),
            Vec2::new(3.0, 3.0),
            "脚本对 id 返回 None → 落到用户拖拽状态"
        );
    }

    #[test]
    fn safe_line_slice_never_panics_on_stale_byte_ranges() {
        // 回归：TextArea 粘贴中文后，旧视觉行字节区间落在新文本多字节字符中间
        // （"start byte index 64 is not a char boundary; inside '尾'" panic）。
        // safe_line_slice 必须对齐到字符边界且不 panic。
        // "窗口 A 选项\n尾部文字"：'项' = bytes 12..15（3 字节/字符）
        let value = "窗口 A 选项\n尾部文字";
        // 模拟过期 vlines：区间起点/终点落在 '项'（12..15）中间
        let stale = VisualLine { byte_start: 13, byte_end: 14, top: 0.0, width: 10.0 };
        let s = safe_line_slice(value, &stale);
        // 13 → floor 到 '项' 起点 12；14 → floor 到 12 → s == e → 空串（不 panic）
        assert_eq!(s, "");
        // 区间越过末尾：byte_end 超 len → clamp 到 len 并取整
        let over = VisualLine { byte_start: 0, byte_end: 999, top: 0.0, width: 10.0 };
        assert_eq!(safe_line_slice(value, &over), value);
        // 正常区间原样返回（0..6 = "窗口"）
        let ok = VisualLine { byte_start: 0, byte_end: 6, top: 0.0, width: 10.0 };
        assert_eq!(safe_line_slice(value, &ok), "窗口");
    }
}
