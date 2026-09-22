//! `Ui` 主体：控件录制 + 深度排序 + 提交绘制。
//!
//! 一个 `Ui` = 一帧里的**一段**录制（一帧可多段；段间帧级事实由
//! [`UiState::frame_state`] 共享）。用法（见 crate 文档与示例）：
//! ```no_run
//! # let viewport = todo!(); let mouse = todo!(); let keyboard = todo!();
//! # let text = todo!(); let mut state: rjw_ui::UiState = todo!(); let mut backend = rjw_ui::RecordingBackend::default(); let window = todo!();
//! use rjw_ui::{Theme, Ui};
//! state.begin_frame();                       // 每帧一次（运行时路径由 Frame::ui 懒开场）
//! let mut ui = Ui::begin(&window, &mut text, &mut state)
//!     .capture(&mouse, &keyboard)
//!     .theme(Theme::dark())
//!     .base_layer(1e7)
//!     .build();
//! ui.label_at(glam::Vec2::new(20.0, 20.0), "Hello UI");
//! ui.finish(&mut backend);                   // 段收尾（可多段）
//! ui.end_frame(&mut backend);                // 帧收尾：焦点导航/描边 + 光标 + 统计 + 复位
//! ```
//!
//! - [`Ui::finish`] = **段收尾**（分桶 → 顶点 → 提交，一帧可多次）；
//! - [`Ui::end_frame`] = **帧收尾**（每帧一次；输入结算 / 焦点导航 + 描边 / 光标定夺 /
//!   统计写回 / 帧级暂存关场）；
//! - 运行时路径（`rjw_krusie::Frame::ui(theme)`）自动处理这两件事。
//!
//! 坐标语义：所有位置为**屏幕逻辑像素**（左上角原点，Y+ 向下）；容器内 `*_at` 的 `pos`
//! 相对**当前容器内容原点**（顶层即屏幕原点）；交互命中在逻辑坐标进行（内部经 DPI 换算）。

use std::ops::RangeInclusive;
use std::sync::Arc;
use std::time::Instant;

use glam::Vec2;
use rjw_color::Color;
use rjw_keyboard::{KeyCode, KeyboardInput};
use rjw_keystate::KeyState;
use rjw_mouse::{MouseButton, MouseInput};
use rjw_text::{Align, Buffer, CachePolicy, Text, TextStyle, VisualLine};
use rjw_transform::{Rect, Transform2D};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::window::Window as WinitWindow;

use crate::backend::{UiBackend, UiBatch, UiBatchSource};
use crate::draw::{
    CornerRadius, DebugShape, DrawKind, Gradient, Icon, ImageBg, Position, Size, TextAlign,
    TextVAlign,
    UiDraw, border_rects, centered_square, debug_shape_segments, intersect_rect,
    screen_fixed_tf, snap_rect, text_block_offset, text_cmd,
};// 顶点收集 / 合批机制（原在此文件，见 `gpu_batch` 模块文档）。
use crate::gpu_batch::{
    BatchPlan, CacheSlot, CacheStats, CachedQuad, Geom, QuadCollector, SubmitUnit, clip_rect,
    cmd_sig_hash, debug_layout_outline, fully_outside, line_row_at_y,
    resample_gradient_local, safe_line_slice, segment_runs, text_visible_rect, vertex_p3u2c4,
};
use crate::edit::{
    byte_to_char, caret_at_visual_click, caret_index_by_width, char_to_byte, insert_char_at,
    scroll_follow_caret, sel_range, vline_of_byte,
};
use crate::focus::{focus_step, FocusEntry, FocusKind};
use crate::hit::{
    HitRegion, clear_frame_flags, hit_test, id_hash, normalize_x, update_drag, update_interact,
    widget_occluded, window_occluded,
};
use crate::id::{IdAbsolute, IdRelative, IdStack};
use crate::input::{KeyboardSnapshot, MouseSnapshot};
use crate::layout::{Child, Frame, PackSide};
use crate::painter::Painter;
use crate::state::{ButtonState, CheckboxState, TEXT_BUFFER_CACHE_CAP, UiState, WidgetState};
use crate::style::{CheckboxStyle, GripShape, GripStyle, PanelStyle, Theme};
use crate::view::{clip_for_view, ViewCtx, ViewMode};

// ─── 文本编辑辅助（纯函数，可单测） ─────────────────────────────

/// 滚动条**条带**宽（物理像素；`scroll_at` 与文本编辑框共用）。文本编辑框的滚动条
/// 条带排除（按下滚动条不建立文本选择）也用它。
///
/// 条带 = 占位 + 命中 / 翻页热区，比可见滑块宽 ⇒ **两侧留白**（观感更轻，抓取
/// 却更容易）。
pub(crate) const SCROLLBAR_W: f32 = 14.0;

/// 滚动条**可见滑块**宽（物理像素，居中于 [`SCROLLBAR_W`] 条带内）。
pub(crate) const SCROLLBAR_BAR_W: f32 = 10.0;

/// 滚动条轨道**上下留白**（物理像素）：胶囊两端不贴可视区边缘。
pub(crate) const SCROLLBAR_MARGIN: f32 = 3.0;

/// 滚动条滑块**最小长度**（物理像素）；轨道比它还矮时以轨道为准。
pub(crate) const SCROLLBAR_MIN_THUMB: f32 = 18.0;

/// **双击判定时间窗**：两次按下间隔 ≤ 该时长且位移 < [`DOUBLE_CLICK_DIST`] 视为双击。
/// 用 `Instant`（时间）而非帧数——高帧率（O2 优化 / 144Hz）下"20 帧"时间窗口会缩水
/// 导致双击难以触发（成功判定窗口与帧率绑定，不同机器不一致）。
pub(crate) const DOUBLE_CLICK_TIME: std::time::Duration = std::time::Duration::from_millis(500);

/// **双击判定位移阈值**（物理像素）。
pub(crate) const DOUBLE_CLICK_DIST: f32 = 4.0;

/// **勾选框中心填充内边距**（物理像素）：中心蓝色矩形 = 外框 shrink
/// `floor(border_w·scale) + floor(CHECKBOX_INNER·scale)`（减法内缩，非写死偏移）。
pub(crate) const CHECKBOX_INNER: f32 = 1.0;

/// **窗口合批单段顶点数上限**（尽力而为：u16 索引上限 65535，留余量取 65000，
/// 且为 4 的倍数——四边形一组）。窗口内容超限时自动切段（顺序连续，层级不变）。
pub(crate) const MAX_UI_SEG_VERTS: usize = 65000;

/// **浮层 z 基址**：IME 候选框、下拉浮层、子菜单、取色面板等**顶层浮层**的 z 从它起，
/// 按**嵌套层数**递增（见 [`overlay_z`]）——恒在一切真实窗口之上。
/// 真实窗口的 z 分配 / 置顶运算必须**排除浮层区间**（`filter(|&z| z < WIN_TOPMOST)`、
/// `saturating_add`），避免普通窗口递增碰撞到这个区间。
///
/// ⚠ **为什么不是一个单一哨兵值**：父浮层（下拉 / 菜单）里还能开**子浮层**（子菜单）。
/// 同一个 z ⇒ 两层命令落在同一个 `(win, elem)` 分组里排序，而窗口的**阴影 / 背景 / 边框**
/// 用的是 `elem = 0`、控件用 `elem ≥ 1` ⇒ **子层的阴影会被父层的控件盖住**
/// （用户实测："下级 popup 阴影被绘制在了上级控件后面"）。
/// 分层 z ⇒ 子层整段（含阴影）排在父层之后；顺带让 `win_origins` / `win_ids` 不再撞键
/// （它们按 z 键），每个浮层用自己的提交原点。
pub(crate) const WIN_TOPMOST: u32 = u32::MAX - OVERLAY_Z_SPAN;

/// 浮层 z 可用层数（够深了；超出后 clamp 到最后一层）。
pub(crate) const OVERLAY_Z_SPAN: u32 = 1024;

/// **浮层 z** = `WIN_TOPMOST + 嵌套层数`（`depth = 0` = 最外层浮层；超出层数上限时 clamp）。
#[inline]
pub(crate) fn overlay_z(depth: u32) -> u32 {
    WIN_TOPMOST + depth.min(OVERLAY_Z_SPAN - 1)
}

/// `z` 是否落在**浮层区间**（`z >= WIN_TOPMOST`）——"鼠标在任意浮层上"这类判定用它，
/// 而不是 `z == WIN_TOPMOST`（浮层现在是一段带层数的区间）。
#[inline]
pub(crate) fn is_overlay_z(z: u32) -> bool {
    z >= WIN_TOPMOST
}

// 纯文本编辑函数（字符插入/删除/剪贴板/编辑状态机）已迁入 [`crate::edit`]：
// `insert_char_at` / `remove_before` / `remove_at` / `clipboard_shortcuts` /
// `apply_frame_edits` / `caret_horiz`。

// ─── 文本排版版本号 ─────────────────────────────────────────────

/// 文本缓冲区行高版本号。当行高计算方式变更时递增，使旧缓存失效。
/// 版本 1：行高 = 字号（原为 1.2 倍字号，导致英文字母在文本框内偏上）。
///
/// `pub(crate)`：内容签名哈希（`gpu_batch::cmd_sig_hash`）必须把它算进签名，
/// 否则改行高策略后窗口顶点缓存不会失效。
pub(crate) const TEXT_LINE_HEIGHT_VERSION: u8 = 1;

// ─── Ui 门面类型（Anchor / WindowClamp / WindowFx / UiCursor / Level /
//     Placement / Resize / WindowOptions / PanelOptions）已拆到 `crate::ui_types`，
//     在此重导出以保持 `crate::ui::X` 路径不变。 ─────────────────────
pub use crate::ui_types::{
    Anchor, Level, PanelOptions, Placement, Resize, ScrollMode, UiCursor, WindowClamp, WindowFx,
    WindowOptions,
};

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
    /// 传入后 `build()` 把 **Theme 预乘 scale**（全部样式尺寸 / 字号 × scale 取整，
    /// 见 [`Theme::scaled`](crate::style::Theme::scaled)）——Ui 内部以**物理像素**为
    /// 单位（布局 / 绘制 / 命中零 scale 换算）；本值仅保留给公开 API 边界的
    /// [`Size`](crate::draw::Size) / [`Position`](crate::draw::Position) 单位换算。
    /// 取值：`ctx.scale_factor().unwrap_or(1.0)`（`rjw_main::MainContext`）。
    pub fn scale_factor(mut self, scale: f64) -> Self {
        self.scale = scale.max(f64::EPSILON) as f32;
        self
    }

    /// **调试 UI 布局**（默认 **关闭**）：开启后 `finish` 为**每一个录制命令的矩形**
    /// 画一圈青色描边（覆盖在 UI 内容之上）——可视化每个控件/容器的布局矩形与
    /// 命中区域，用于调试 `rjw_ui` 自身的布局。可在帧内用 [`Ui::debug_layout`] /
    /// [`Ui::without_debug_layout`] 切换。
    pub fn debug_layout(mut self) -> Self {
        self.debug_layout = true;
        self
    }

    /// 调试 UI 布局**关闭**（默认；用于显式覆盖）。
    pub fn without_debug_layout(mut self) -> Self {
        self.debug_layout = false;
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
        // 段开场计时：只覆盖**引擎**的段开场工作（懒开场 / 冻结输入 / 装载帧级事实 /
        // 建根容器 / Theme 预乘），用于把 `ui_frame_us` 三分解成
        // `prologue + 应用录制 + finish`（见 `UiStats::prologue_us`）。
        let t_prologue = Instant::now();
        // ── 帧级账（**懒开场**）──────────────────────────────────────────
        // 本帧第一段才开帧：帧号 +1 / 命中区表翻页 / 帧级暂存清零 + **冻结输入快照**。
        // 后续段（同帧第二次起录制）**不再开场**——否则帧号 +2、命中表二次翻页、
        // 上一段录下的窗口原点 / 焦点链 / 责任链 / 按下归属全被清掉。
        // ⚠ 开场必须留到"第一段"而不是帧首（`Ctx::begin_update`）：应用在 `update`
        // 里先 `debug_inject_mouse(..)`（脚本化鼠标）再录 UI，快照早了就丢了点击边沿。
        if !state.frame_open() {
            state.begin_frame();
            state
                .frame_state
                .freeze_input(mouse.clone(), keyboard.clone());
        }
        // **段号**（帧内第几段，从 1 起）：给 win=0 放置子槽的缓存键加段前缀。
        // 组号是"段内第几个顶层放置"（各段从 0 起、兜底组恒为 0），不加段前缀会让
        // 不同段的同号槽互相覆盖 ⇒ 每帧都判 miss（见 `UiState::z0_quads`）。
        state.frame_state.segment += 1;
        let segment = state.frame_state.segment;
        // 本帧统一使用**开场时冻结**的快照（段间注入只影响下一帧）。
        let frame_mouse = state.frame_state.mouse.clone();
        let frame_keyboard = state.frame_state.keyboard.clone();
        // **Theme 预乘 scale**：样式尺寸 / 字号 × scale 取整——Ui 内部此后以物理像素
        // 为单位（布局 / 绘制 / 命中零 scale 换算）；`scale` 仅保留给 API 边界
        // `Size` / `Position` 的 Logical→Physical 换算。
        let theme = theme.scaled(scale);
        let (mx, my) = frame_mouse.pos_px();
        let mouse_screen = Vec2::new(mx as f32, my as f32);
        let mouse_in_window = frame_mouse.in_window();
        let mut ui = Ui {
            window,
            text,
            state,
            mouse: frame_mouse,
            keyboard: frame_keyboard,
            theme,
            base_layer,
            scale,
            debug_layout,
            frames: Vec::new(),
            painter: Painter::new(scale),
            avail_stack: Vec::new(),
            abs_base: Vec2::ZERO,
            place_once: None,
            cur_win_id: None,
            win_hit_bounds: None,
            cur_win_hit_limit: None,
            z0_ranges: Vec::new(),
            segment,
            // 鼠标屏幕坐标：物理（拖拽 / IME 基准与命中测试统一物理像素，无逻辑之分）
            mouse_screen,
            mouse_logical: mouse_screen,
            mouse_in_window,
            any_pressed: false,
            overlay_depth: 0,
            press_claimed: false,
            drag_panel: None,
            win_press_top: None,
            win_origins: std::collections::HashMap::new(),
            win_ids: std::collections::HashMap::new(),
            focusables: Vec::new(),
            // UI 帧起点（帧收尾计算 ui_frame_us = 开场 → 收尾的整帧耗时）
            frame_t0: Instant::now(),
            // 位置责任链：预置内置"用户拖拽状态"环（优先级 0）
            pos_chain: vec![(0, PosLink::Drag)],
            // 尺寸责任链：预置内置"用户拖拽缩放"环（优先级 0）
            size_chain: vec![(0, SizeLink::Drag)],
            cursor_text: false,
            cursor_grab: false,
            cursor_grabbing: false,
            cursor_window_drag: false,
            cursor_custom: None,
            ids: IdStack::new(),
        };
        // 帧级事实从暂存**装载**回本段视图（第一段 = 刚开场的默认值，等价于不装载）。
        ui.load_frame_state();
        // 根容器（顶层流式布局）：pack 控件（`label` / `button` / `slider` …）在
        // `finish` 前**任何位置**可调用——顶层直接自顶向下堆叠（原 `child_rect` 在
        // 无容器时 panic）；固定宽 = 视口物理宽 → 顶层 `avail_w()` = 视口宽
        // （`LimitedInParent` 控件自动换行自洽）。容器 / 窗口各自 push 自己的帧，
        // root 只作栈底，`finish` 不读 frames，无副作用。
        let vw = ui.window_physical_size().0 as f32;
        let mut root = Frame::new_stack(PackSide::Top, ui.theme.gap, 0.0);
        root.set_fixed_w(vw);
        ui.frames.push(root);
        // 段开场耗时累加进帧级统计（`ui_frame_us` 三分解的引擎那一份）。
        ui.state.frame_state.stats.prologue_us += t_prologue.elapsed().as_secs_f64() * 1e6;
        ui
    }
}

/// **自动窗口位置（`CW_USEDEFAULT`）的级联步长**（**逻辑**像素）：没写 `.pos()` 的
/// 窗口按首次出现顺序，每级右下偏移这么多（同 Win32 的层叠窗口）。
pub const AUTO_POS_STEP: f32 = 28.0;

/// `hscroll(Scroll)` 的降级提示只打一次（避免每帧刷屏）。
static HS_SCROLL_WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// **自动窗口位置**：第 `n` 个未指定 `.pos()` 的窗口落在哪（纯函数，可单测）。
///
/// 策略 = Win32 层叠：从 `(margin, margin)` 起每级右下偏移 [`AUTO_POS_STEP`]（× `scale`
/// 换算物理），越过视口右下可用范围时**回绕**（取模）——窗口再多也不会全跑出屏幕。
/// `viewport` = 客户区**物理**尺寸（内部一律物理），`margin` 留出窗口标题可点的边距。
fn auto_pos_slot(n: u32, viewport: Vec2, scale: f32) -> Vec2 {
    let margin = 16.0 * scale;
    let step = AUTO_POS_STEP * scale;
    // 可用级联跨度：至少一格（视口比一格还小时就不回绕，够用即可）。
    let span = Vec2::new(
        (viewport.x - margin * 2.0 - step).max(step),
        (viewport.y - margin * 2.0 - step).max(step),
    );
    let d = n as f32 * step;
    Vec2::new(margin + d % span.x, margin + d % span.y)
}

/// **取 / 分配自动位置**（纯函数，可单测）：`id` 已有记录 ⇒ 返回记录值（**跨帧稳定**，
/// 不会每帧顺着级联往下漂）；否则分配下一个槽位并记进 `map`，`next` 自增。
fn auto_pos_take(
    map: &mut std::collections::HashMap<IdAbsolute<'static>, Vec2>,
    next: &mut u32,
    id: &IdAbsolute<'_>,
    viewport: Vec2,
    scale: f32,
) -> Vec2 {
    if let Some(p) = map.get(id.as_str()) {
        return *p;
    }
    let p = auto_pos_slot(*next, viewport, scale);
    *next += 1;
    map.insert(id.to_static(), p);
    p
}

/// 窗口/面板**位置责任链**一环：
/// - [`PosLink::Script`]：应用注册的脚本/动画/布局处理器（见 [`Ui::pos_handler`]；
///   **`'static` 闭包**——可捕获拥有值 / `Copy` 值 / `Arc`，需要共享可变状态时用
///   `Arc<Mutex<_>>`；约束闭包不借用 `self`，避免拖长 `Ui` 的借用导致
///   `ui.finish()` 后无法再访问应用状态）；
/// - [`PosLink::Drag`]：内置"用户拖拽状态"（[`UiState::panel_pos`]，固定优先级 `0`）。
///
/// 存在 [`UiState`] 的帧级暂存里（`pub(crate)`）：一帧可有多段 UI，责任链必须跨段存活
/// （第一段注册的脚本处理器对第二段录制的窗口同样生效）。
pub(crate) enum PosLink {
    Script(Box<dyn Fn(&str) -> Option<Vec2> + 'static>),
    Drag,
}

/// 按**优先级降序**解析窗口/面板位置：第一个返回 `Some` 的环生效；
/// 全部落空（含用户未拖过）则回退 `pos`（调用者传入的初始位置）。
/// `id` 为**绝对 ID**（脚本处理器收到的是完整命名空间串）。
fn resolve_pos_link(
    chain: &[(i32, PosLink)],
    panel_pos: &std::collections::HashMap<IdAbsolute<'static>, Vec2>,
    id: &IdAbsolute<'_>,
    pos: Vec2,
) -> Vec2 {
    for (_, link) in chain {
        match link {
            PosLink::Script(f) => {
                if let Some(p) = f(id.as_str()) {
                    return p;
                }
            }
            PosLink::Drag => {
                if let Some(p) = panel_pos.get(id.as_str()) {
                    return *p;
                }
            }
        }
    }
    pos
}

/// 可调尺寸控件（可调整大小/宽度的文本输入框）的**尺寸责任链**一环：
/// - [`SizeLink::Script`]：应用注册的脚本/动画/布局尺寸处理器（[`Ui::size_handler`]；
///   语义与 [`PosLink::Script`] 相同：`'static` 闭包，不借用 `self`）；
/// - [`SizeLink::Drag`]：内置"用户拖拽缩放"（[`UiState::sizes`]，固定优先级 `0`）。
///
/// 存于 [`UiState`] 的帧级暂存（跨段存活，理由同 [`PosLink`]）。
pub(crate) enum SizeLink {
    Script(Box<dyn Fn(&str) -> Option<Vec2> + 'static>),
    Drag,
}

/// 按**优先级降序**解析控件尺寸：第一个返回 `Some` 的环生效；全部落空（含用户未拖过）
/// 回退 `fallback`（调用方传入的初始尺寸）。`id` 为**绝对 ID**。
fn resolve_size_link(
    chain: &[(i32, SizeLink)],
    sizes: &std::collections::HashMap<IdAbsolute<'static>, Vec2>,
    id: &IdAbsolute<'_>,
    fallback: Vec2,
) -> Vec2 {
    for (_, link) in chain {
        match link {
            SizeLink::Script(f) => {
                if let Some(s) = f(id.as_str()) {
                    return s;
                }
            }
            SizeLink::Drag => {
                if let Some(s) = sizes.get(id.as_str()) {
                    return *s;
                }
            }
        }
    }
    fallback
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
    /// 主题样式（**crate 内公开**：widget 层 / 同 crate 控件合并全局样式与逐控件覆盖用；
    /// 帧内可改，影响后续录制）。跨 crate 请用 [`Ui::theme`] / [`Ui::theme_mut`]。
    pub(crate) theme: Theme,
    base_layer: f64,
    /// DPI scale factor（**仅公开 API 边界** [`Size`](crate::draw::Size) /
    /// [`Position`](crate::draw::Position) 的 Logical→Physical 换算用；布局 / 命中 /
    /// 绘制一律物理像素、零 scale 换算——Theme 已在 `build()` 预乘）。
    scale: f32,
    /// 容器帧栈（当前容器在栈顶）。
    frames: Vec<Frame>,
    /// **绘制器**（独立组件，见 [`crate::painter`]）：拥有录制状态（命令队列 + 播放头）
    /// 与 API 边界的 DPI 换算。`Ui` 只持有一个，绘制经 [`Ui::painter`] 借出。
    painter: Painter,
    /// **调试 UI 布局开关**（[`UiInit::debug_layout`] / [`Self::debug_layout`]）：
    /// 开启后每个录制命令的矩形都会画青色描边（布局 / 命中区域可视化）。
    debug_layout: bool,
    /// **可用宽度栈**（逻辑像素）：`view_at` 沙箱进入时压入沙箱宽，弹出恢复。
    /// [`Self::avail_w`] 的唯一沙箱来源（容器固定宽经 `Frame::fixed_avail_w` 兜底）。
    avail_stack: Vec<Option<f32>>,
    /// 当前容器绝对原点（命中测试用，逻辑像素）。
    abs_base: Vec2,
    /// **一次性放置覆盖**（[`Self::add_at`] 写入、`allocate*` 消费后即清）：
    /// 控件不在光标处申请，而落在指定的绝对坐标（相对当前容器内容原点）。
    /// ⚠ 只作用于控件的**第一次**申请（见 [`Self::add_at`] 文档）。
    place_once: Option<Vec2>,
    /// **当前窗口的绝对 ID**（非窗口内容 = `None`；嵌套窗口进出时保存/恢复）。
    ///
    /// 用途：`hit_impl` 记下"这次按下认领属于哪扇窗"时**必须存 ID 而不是 z**——z 会在帧末
    /// 被"点击置顶"改，帧末复核要解它的**当前** z（见 `resolve_widget_press`）。
    cur_win_id: Option<IdAbsolute<'static>>,
    /// **当前窗口内"可交互控件的命中区"并集**（绝对坐标）：窗口退出时与窗口盒子
    /// 并起来写进 `UiState::window_rects[z]` ⇒ **遮挡判定按"看得见的范围"走**。
    ///
    /// 为什么需要：窗口盒子与"实际可见 / 可点的内容"可能不一致（绝对放置的内容、
    /// 固定尺寸容器里溢出的控件）。若遮挡矩形只取盒子，则
    /// 1. 盒子**外**的控件会被**更高 z 窗口**正确拦下（它确实压在内容上），却会被
    ///    **更低 z 窗口**"穿透"——同一处交互随 z 变化而失真；
    /// 2. `window_under_mouse()` / 诊断面板也会指错窗口。
    ///
    /// 记录子控件命中区（而不是所有绘制命令）是**有意为之**：装饰（阴影 / 描边）
    /// 不该扩大交互范围。窗口进入时保存、退出时恢复（浮层是嵌套窗口）。
    win_hit_bounds: Option<Rect>,
    /// **当前窗口的"可命中区域"限制**（绝对矩形；`None` = 不限制）。
    ///
    /// 只在窗口**内容被裁切**时设置（`window_content_clipped`：`Placement::Clip` 或
    /// 高度已被用户拖过 ⇒ 窗口成了固定尺寸视口）：此时超出窗口矩形的子控件**画不出来**
    /// （命令行的 clip 会裁掉它），但它们仍然登记命中区 ⇒ 窗口外面那一圈"看得见面板、
    /// 看不见控件"的地方能点到**幽灵控件**（用户实测的"Vertical 缩放幽灵控件"）。
    /// 命中判定加上"鼠标点必须在限制矩形内"即可：裁掉的部分不可点，可见的部分照旧可点。
    ///
    /// 与 [`Self::win_hit_bounds`] 一样随窗口进出保存 / 恢复（浮层是嵌套窗口）。
    cur_win_hit_limit: Option<Rect>,
    /// **本帧 win=0（非窗口）各顶层放置的起始 `seq`**（按录制序）。由顶层放置入口在
    /// `depth == 0` 时记录（[`Self::begin_top_placement`]）。
    ///
    /// 这份表同时供**两个**用途，二者都必须有它：
    ///
    /// 1. **绘制序**（[`Self::z0_place_for_seq`]）：给每条 win=0 命令一个"放置序"
    ///    `place`，提交排序键是 `(win, place, elem, group, tex, clip)`。
    ///    ⚠ **没有它 win=0 就没有独立排序空间**：所有非窗口内容共享 `win = 0`，
    ///    而 `elem = 0` 的语义是"画在本容器元素之下"——只有"每个放置一个排序空间"
    ///    才成立。历史 bug 就是踩了这条：可拖动面板的底色（`elem = 0`）被**更早录制**的
    ///    win=0 内容（FPS 标签）穿透、`scroll_at` 的滚动条（`elem = 0`）被列表项盖住，
    ///    两者都表现为**闪烁**；
    /// 2. **缓存槽键**：`finish` 按 `place` 把 win=0 命令归槽，逐槽做**全量签名**顶点
    ///    缓存，使值/交互变化只重建对应放置，其余 win=0 内容复用。
    ///
    /// ⚠ `z0_ranges` 是**段内**的（`Ui` 视图每段新建）：`place` 只在段内唯一——
    /// 缓存键必须带上 [`Self::segment`]（见 `UiState::z0_quads`）。
    ///
    /// 不需要"结束 `seq`"：放置之间不可能嵌套（本入口只在 `depth == 0` 记录，
    /// 嵌套调用什么也不做），所以"下一条命令属于哪个放置"只由 `start` 决定。
    z0_ranges: Vec<u32>,
    /// **本段段号**（帧内第几段，从 1 起）——win=0 放置子槽缓存键的段前缀。
    segment: u32,
    /// 鼠标屏幕坐标（**物理像素**；命中测试用——内部坐标全物理）。
    mouse_logical: Vec2,
    /// 鼠标屏幕坐标（**物理像素**，面板拖拽 / IME 基准用）。
    mouse_screen: Vec2,
    mouse_in_window: bool,
    /// 本帧是否有控件被按下（空白点击清焦点用）。
    any_pressed: bool,
    /// **当前正在录制的浮层层数**（0 = 不在浮层里）：浮层 z = [`overlay_z`]`(本值)`，
    /// 进入 / 退出一层用 [`Self::push_overlay_z`] / [`Self::pop_overlay`]。
    /// 嵌套（下拉里开子菜单）时递增 ⇒ 子层 z 更大、整段画在父层之上（含阴影）。
    overlay_depth: u32,
    /// **本帧按下是否被文本输入控件占用**（选择拖拽优先于窗口/面板拖拽）：
    /// 输入框/TextArea 在按下响应时置位，`window_at` / `panel_impl` 据此**不建立**
    /// 拖拽基准——从输入框上拖拽 = 选择文本，而不是拖动窗口。
    press_claimed: bool,
    /// 当前拖拽中的面板 / 窗口 **绝对 ID**（拖动期间抑制子控件交互）。
    drag_panel: Option<IdAbsolute<'static>>,
    /// 本帧按下命中的**最上层窗口**（重叠点击裁决：只让最高 z 窗口拖拽与置顶）。
    win_press_top: Option<(IdAbsolute<'static>, u32)>,
    /// 窗口 z → 窗口左上角（**逻辑**坐标）：QuadVertices 顶点相对此原点存储，
    /// 提交时用 `screen_fixed_tf(原点物理)` 变换到世界（移动窗口只改变换，顶点不变）。
    win_origins: std::collections::HashMap<u32, Vec2>,
    /// 窗口 z → 窗口 **绝对 ID**（四边形缓存 key 用；`window` 记录）。
    win_ids: std::collections::HashMap<u32, IdAbsolute<'static>>,
    /// **本帧焦点链**（键盘导航）：交互控件录制时注册（[`Self::register_focus`]），
    /// `finish` 按 (win, 注册序) 排序后处理 Tab / 方向键遍历并绘制焦点描边。
    focusables: Vec<FocusEntry>,
    /// UI 帧起点（`UiInit::build` 记录；`finish` 据此计算 `ui_frame_us`）。
    frame_t0: Instant,
    /// **窗口/面板位置责任链**：应用脚本处理器（优先级降序）+ 内置拖拽状态
    /// （优先级 0），见 [`Self::pos_handler`]；一帧一建，随 Ui 释放。
    pos_chain: Vec<(i32, PosLink)>,
    /// **尺寸责任链**（优先级降序）：可调整大小/宽度的文本输入框用（脚本/布局指定
    /// vs 用户拖拽缩放，见 [`Self::size_handler`] / [`UiState::sizes`]）；固定优先级 0 =
    /// 用户拖拽缩放结果。
    size_chain: Vec<(i32, SizeLink)>,
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
    /// **ID 命名空间栈**（[`IdStack`]）：容器进入 `push`、退出 `pop`；控件经
    /// [`Self::id_for`] 从相对 id 生成**绝对 id**（状态键 / 焦点 id）。
    ids: IdStack,
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

    /// **装载帧级事实**（段起始，`UiInit::build()` 末尾）：把 [`UiState::frame_state`]
    /// 里的一帧一份的事实搬进本段视图。
    ///
    /// 一帧一 `Ui` 的旧模型下这些字段本来就是"本段构造 = 本帧事实"；分成多段后必须
    /// 显式搬运，否则第二段会以**空**的按下归属 / 窗口原点 / 焦点链 / 责任链开始录制。
    fn load_frame_state(&mut self) {
        let fs = &mut self.state.frame_state;
        self.mouse_in_window = fs.mouse_in_window;
        self.mouse_screen = fs.mouse_screen;
        self.mouse_logical = fs.mouse_logical;
        self.any_pressed = fs.any_pressed;
        self.press_claimed = fs.press_claimed;
        self.cursor_text = fs.cursor_text;
        self.cursor_grab = fs.cursor_grab;
        self.cursor_grabbing = fs.cursor_grabbing;
        self.cursor_window_drag = fs.cursor_window_drag;
        self.cursor_custom = fs.cursor_custom;
        self.frame_t0 = fs.frame_t0;
        self.segment = fs.segment;
        // 集合 / `Option` 用 take（`save_frame_state` 会原样换回去）：段与帧之间**搬移**
        // 而不是克隆——窗口原点表 / 焦点链在大 UI 下可不小。
        self.drag_panel = fs.drag_panel.take();
        self.win_press_top = fs.win_press_top.take();
        self.win_origins = std::mem::take(&mut fs.win_origins);
        self.win_ids = std::mem::take(&mut fs.win_ids);
        self.focusables = std::mem::take(&mut fs.focusables);
        self.pos_chain = std::mem::take(&mut fs.pos_chain);
        self.size_chain = std::mem::take(&mut fs.size_chain);
    }

    /// **回存帧级事实**（段收尾，`Ui::finish()` 末尾）：本段的修改成为本帧的真值，
    /// 供后续段（以及帧收尾的输入结算 / 光标定夺 / 统计）继续使用。
    fn save_frame_state(&mut self) {
        let fs = &mut self.state.frame_state;
        fs.mouse_in_window = self.mouse_in_window;
        fs.mouse_screen = self.mouse_screen;
        fs.mouse_logical = self.mouse_logical;
        fs.any_pressed = self.any_pressed;
        fs.press_claimed = self.press_claimed;
        fs.cursor_text = self.cursor_text;
        fs.cursor_grab = self.cursor_grab;
        fs.cursor_grabbing = self.cursor_grabbing;
        fs.cursor_window_drag = self.cursor_window_drag;
        fs.cursor_custom = self.cursor_custom;
        fs.drag_panel = self.drag_panel.take();
        fs.win_press_top = self.win_press_top.take();
        std::mem::swap(&mut fs.win_origins, &mut self.win_origins);
        std::mem::swap(&mut fs.win_ids, &mut self.win_ids);
        std::mem::swap(&mut fs.focusables, &mut self.focusables);
        std::mem::swap(&mut fs.pos_chain, &mut self.pos_chain);
        std::mem::swap(&mut fs.size_chain, &mut self.size_chain);
    }

    #[inline]
    fn next_seq(&mut self) -> u32 {
        self.painter.next_seq()
    }

    /// **顶层放置分组入口**：`depth == 0` 时开一个新 win=0 放置（记录它**第一条命令的
    /// `seq`**）；嵌套（`depth > 0`，即当前放置的子容器）不记录（并入外层放置）。
    ///
    /// ⚠ **记的是 `seq + 1`（下一条命令的序号），不是当前 `seq`**：`begin_top_placement`
    /// 在容器入口调用，此时队列里最后一条命令属于**上一个**放置（例如 `scroll_at` 的滚动条
    /// 由 `next_seq()` 取号，紧挨着后面的容器的起点）。若记当前的 `seq`，那条命令会被算进
    /// **新**放置（`place(seq) = #{start ≤ seq}` 会用 `≤` 命中），同一个容器的命令被拆进
    /// 两个排序空间 ⇒ 序不稳、又变成闪烁。记 `seq + 1` 后语义是"本放置第一条命令的序号"。
    ///
    /// 记录的东西只有一个用途：给每条 win=0 命令算出它的**放置序**
    /// （[`Self::z0_place_for_seq`]），进而决定绘制序与缓存槽键
    /// （见 [`Self::z0_ranges`]）。不记录也不会漏——未调用的放置（独立顶层
    /// `label_at` 等）落在"上一个放置"里，见 [`Self::z0_place_for_seq`]。
    #[inline]
    fn begin_top_placement(&mut self) {
        if self.painter.q.depth == 0 {
            // 不变量：队列播放头 = 已分配的最大序号。少数"直接写队列"的入口若一条命令
            // 之后没再取号（历史 bug：滚动条两条命令共用一次 `next_seq()`），下一条命令会
            // 拿到**重复序号** ⇒ 以 `seq` 归一化的放置序会把同一容器拆成两个排序空间。
            // 这里把它变成显式失败，而不是"序偶尔错一帧"。
            debug_assert!(
                self.painter.q
                    .queue
                    .last()
                    .is_none_or(|d| d.seq <= self.painter.q.seq),
                "seq 播放头落后于已入队命令（直接写队列的入口必须每条命令各取一次 next_seq）"
            );
            self.z0_ranges.push(self.painter.q.seq + 1);
        }
    }

    /// **放置序**（`place`）：到 `seq` 这条命令为止**已经开始了几个顶层放置**。
    ///
    /// 这是 win=0 的**唯一**排序维度（见 [`Self::z0_ranges`] 为什么必须有它）：
    ///
    /// - 同一放置内的全部命令（含**最后录制**的投影 / 底色、以及 `scroll_at` 的滚动条）
    ///   得到同一个 `place` ⇒ 放置内仍由 `elem` 排序 ⇒ `elem = 0` 的装饰正好排到
    ///   "本放置自己的子内容之下"（这才是 `elem = 0` 的本意）；
    /// - 相邻放置由 `place` 分开 ⇒ **后录制者在上**（与窗口的 z 序同构）；
    /// - **散装顶层命令**（未开放置的 `label_at` / `add_at`）按计数落进"上一个放置"
    ///   的 `place`：它录制更晚 ⇒ `elem` 更大 ⇒ 画在那个放置之上（正确）。
    ///
    /// `z0_ranges` 按录制序压栈 ⇒ `start` 单调 ⇒ 数一下即可（且可提前 `break`）。
    /// 判定逻辑抽成自由函数 [`z0_place_for_seq`]，使语义能在**没有 `Ui` 实例**时单测。
    #[inline]
    fn z0_place_for_seq(&self, seq: u32) -> u32 {
        z0_place_for_seq(&self.z0_ranges, seq)
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

    /// 主题（只读；每帧由 [`UiInit::theme`] / `Frame::ui` 传入）。
    ///
    /// 需要逐控件改样式时优先用 widget builder（`.color(..)` 等）；确实要改全局主题
    /// 用 [`Self::theme_mut`]（只影响**本帧后续**录制——每帧 `Ui::begin` 会重新传入）。
    #[inline]
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// 主题（可变；**只影响本帧后续录制**——每帧 [`Ui::begin`] 会重新传入主题）。
    #[inline]
    pub fn theme_mut(&mut self) -> &mut Theme {
        &mut self.theme
    }

    /// DPI scale factor（物理像素 / 逻辑像素，如 1.0 / 1.5 / 2.0）。
    ///
    /// **仅公开 API 边界** [`Size`](crate::draw::Size) / [`Position`](crate::draw::Position)
    /// 的 Logical→Physical 换算用——内部布局 / 命中 / 绘制一律物理像素（Theme 已预乘，
    /// 见 [`Theme::scaled`](crate::style::Theme::scaled)）。
    #[inline]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// 鼠标**物理**屏幕坐标（warp 边缘判定 / 物理像素增量拖拽用）。
    #[inline]
    pub fn mouse_screen(&self) -> Vec2 {
        self.mouse_screen
    }

    /// 按键本帧按下边沿（控件作者键盘交互用，如 Esc 关闭模态对话框）。
    #[inline]
    pub fn key_down_edge(&self, key: winit::keyboard::KeyCode) -> bool {
        self.keyboard.key(key).down_edge()
    }

    /// 按键**当前是否按住**（控件作者键盘交互用，如 Shift/Ctrl 修饰拖拽速度）。
    #[inline]
    pub fn key_down(&self, key: winit::keyboard::KeyCode) -> bool {
        self.keyboard.key(key).pressed()
    }

    /// 当前容器光标位置（局部坐标；自写"占光标"式组合布局时用于放置子容器）。
    #[inline]
    pub fn cursor_pos(&self) -> Vec2 {
        self.frames.last().map(|f| f.cursor).unwrap_or(Vec2::ZERO)
    }

    /// **当前容器内容原点的绝对坐标**（物理像素；`abs_base`）——**组合控件作者公开面**。
    ///
    /// 自绘浮层（取色器面板 / 第三方下拉）要把"当前容器局部"的锚点换算成**屏幕绝对**
    /// 坐标才能判断"放不放得下"；`Ui::abs_rect` 是它的常用封装。
    #[inline]
    pub fn content_origin(&self) -> Vec2 {
        self.abs_base
    }

    /// **局部矩形 → 绝对矩形**（物理像素；与 [`Self::content_origin`] 配套）。
    ///
    /// 用途：组合控件在"当前容器局部"里拿到自己的矩形后，需要绝对坐标去做**屏幕级**
    /// 决策（浮层该朝哪边翻、会不会出屏幕）。引擎侧同名的绝对量是 `abs_base`。
    #[inline]
    pub fn abs_rect(&self, local: Rect) -> Rect {
        Rect::new(
            self.abs_base.x + local.x,
            self.abs_base.y + local.y,
            local.w,
            local.h,
        )
    }

    /// **声明本次按下归本控件**：自定义交互控件（自身有**拖拽语义**，如数字输入的
    /// 拖动手柄）在 `down_edge && hit` 时调用——阻止外层窗口/面板把本次按下当作
    /// **窗口拖拽基准**（否则窗口内拖滑块/手柄会连窗口一起动）。内置滑块 / 滚动条
    /// / 文本框已自行调用。
    #[inline]
    pub fn claim_press(&mut self) {
        self.press_claimed = true;
    }

    /// **声明本次按下被某个控件响应**（控件作者用）：帧末"点空白处清焦点"的判定要
    /// 区分"按在了控件上"与"按在了空白处"——自定义控件（如 [`crate::Dropdown`] 的
    /// 触发器）按下时调它。
    #[inline]
    pub(crate) fn note_press_handled(&mut self) {
        self.any_pressed = true;
    }

    /// **进入一层浮层**（控件作者用：下拉 / 菜单 / 取色面板…）：返回本层该用的 z
    /// （[`overlay_z`]`(嵌套层数)`），并把层数 +1。配套 [`Self::pop_overlay`]。
    ///
    /// 用法：`let z = ui.push_overlay_z(); state.window_z.insert(id, z); …录浮层窗口… ;
    /// ui.pop_overlay();`
    ///
    /// ⚠ 浮层里再开浮层（子菜单 / 菜单里的取色器）**必须**走这一对方法：同 z 会让两层的
    /// 绘制命令落进同一个 `(win, elem)` 分组，子层的阴影 / 背景（`elem = 0`）被父层控件
    /// （`elem ≥ 1`）盖住（用户实测："下级 popup 阴影被绘制在了上级控件后面"）。
    #[inline]
    pub(crate) fn push_overlay_z(&mut self) -> u32 {
        let z = overlay_z(self.overlay_depth);
        self.overlay_depth += 1;
        z
    }

    /// **退出一层浮层**（与 [`Self::push_overlay_z`] 配对）。
    #[inline]
    pub(crate) fn pop_overlay(&mut self) {
        self.overlay_depth = self.overlay_depth.saturating_sub(1);
    }

    /// **通用拖拽缩放柄**（控件作者原语）：`handle` 为**当前容器局部坐标**的柄矩形
    /// （通常右下角）。按住拖拽把 `current` 改为新尺寸（返回 `Some(new)`；`None` =
    /// 本帧无变化）；范围 clamp 到 `min`。拖动中置位 `press_claimed`（阻止外层
    /// 窗口/面板把本次按下当拖拽基准），悬停/拖拽显示 `cursor`（如 ↔ / ↖↘）。
    ///
    /// 持久尺寸由调用方写入（推荐 [`UiState::sizes`]）；可缩放 widget 的 `size()`
    /// 优先读持久值，配合 [`crate::widgets::Widget::resizable`] 声明。
    /// `window(width)` / `Placement::Clip` 的宽度缩放即基于本原语。
    ///
    /// = [`Self::resize_handle_hit`]（命中 + 光标，**应在内容之前**）+ 本体的拖拽应用。
    /// 需要"让内容里的控件优先"的调用方（窗口右下角柄 vs 面板内最后一个控件的柄重叠）
    /// 要把这两步**拆开**：命中登记早、应用晚（见 [`Self::window_impl`]）。
    pub fn resize_handle(
        &mut self,
        id: &str,
        handle: Rect,
        current: Vec2,
        min: Vec2,
        cursor: crate::UiCursor,
    ) -> Option<Vec2> {
        let hit = self.resize_handle_hit(id, &handle, cursor);
        self.resize_handle_apply(id, current, min, cursor, hit)
    }

    /// [`Self::resize_handle`] 的**上半**：命中判定 + 光标（登记命中区）。
    ///
    /// ⚠ **登记顺序就是优先级**：控件级遮挡按"谁后登记谁在上"判定 ⇒ 想让内容里的控件
    /// 压过窗口自己的柄，窗口必须在**内容之前**调本方法（后登记的内容控件因此获胜）。
    /// ⚠ **本方法不置位 `press_claimed`**（认领在 [`Self::resize_handle_apply`]）：
    /// 窗口路径要在内容之后凭 `!press_claimed` 判断"内容有没有抢走这次按下"，若这里就
    /// 认领，窗口会把自己挡住（实测：`--sim-resize` 变成"柄点不动"）。
    pub(crate) fn resize_handle_hit(
        &mut self,
        id: &str,
        handle: &Rect,
        cursor: crate::UiCursor,
    ) -> bool {
        let abs = self.id_for(id);
        let hhit = self.hit_abs(&abs, handle);
        if hhit {
            self.set_cursor(cursor);
        }
        hhit
    }

    /// [`Self::resize_handle`] 的**下半**：把拖拽落到尺寸上（`hit` 来自上半）。
    ///
    /// 调用方通常**只看 `!press_claimed` 再调**：内容里的控件已经认领这次按下时，
    /// 外层（窗口）的柄必须让位——否则"点面板内最后一个控件的缩放柄"会连带把窗口也缩了。
    pub(crate) fn resize_handle_apply(
        &mut self,
        id: &str,
        current: Vec2,
        min: Vec2,
        cursor: crate::UiCursor,
        hit: bool,
    ) -> Option<Vec2> {
        let abs = self.id_for(id);
        let hbtn = self.mouse_left();
        if hbtn.down_edge() && hit {
            // 缩放柄自身有拖拽语义：阻止外层窗口/面板把本次按下当作拖拽基准。
            self.press_claimed = true;
        }
        let mut new = current;
        let active = {
            let ws = self.state.widgets.entry(abs.to_static()).or_default();
            // **松手即清基准**：基准属于"某一次按住"，跨次复用会让下一次拖拽以上一次的
            // 基准结算 ⇒ 尺寸每帧按位移**连乘**下去（用户实测："窗口收缩高度应当为恒定值"：
            // 往上收缩时高度一档一档掉到下限，明明鼠标已经停了）。
            if !hbtn.pressed() {
                ws.press_mouse = None;
                ws.press_panel = None;
            }
            let was_dragging = ws.dragging;
            let a = update_drag(ws, hit, hbtn);
            // 基准**只在按下边沿**捕获（`hbtn.pressed()` 也要为真）。
            //
            // ⚠ 曾经这里还有 `|| !was_dragging` 的"兜底补捕获"：一旦按键状态在某帧读到
            // "未按下"（真实鼠标在窗口外、注入式输入、丢帧），`dragging` 被清掉 ⇒ 下一帧
            // 又满足 `!was_dragging` ⇒ **拿当前尺寸 + 当前鼠标重新立基准**，于是每帧再加
            // 一次位移 ⇒ 尺寸**无限增长**（用户实测："加了 resize Both 则无限增高"）。
            // 现在：没有基准的那一帧就**不生效**（少走一帧），而不是重新立基准。
            let _ = was_dragging;
            if hbtn.down_edge() && hbtn.pressed() && hit {
                ws.press_mouse = Some(self.mouse_screen.round());
                ws.press_panel = Some(current);
            }
            if a {
                let pm = ws.press_mouse.unwrap_or(self.mouse_screen);
                let base = ws.press_panel.unwrap_or(current);
                let d = (self.mouse_screen - pm).round();
                new = Vec2::new((base.x + d.x).max(min.x), (base.y + d.y).max(min.y));
            }
            if std::env::var_os("RJ_GRIP_TRACE").is_some() {
                eprintln!(
                    "grip[{id}] hit={hit} edge={} drag={a} cur={current:?} press_m={:?} press_p={:?} mouse={:?} -> new={new:?}",
                    hbtn.down_edge(),
                    ws.press_mouse,
                    ws.press_panel,
                    self.mouse_screen
                );
            }
            a
        };
        if active {
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

    /// **窗口整窗口特效**：`tint`（混合色，淡入淡出/整窗染色）+ `transform`
    /// override（位移/缩放/旋转动画）。**每帧可改**；窗口顶点缓存不变，仅提交时
    /// 应用到窗口段实例（矩阵/颜色）。默认无 fx（`WindowFx::default()`）。
    pub fn window_fx(&mut self, id: &str, fx: WindowFx) {
        let abs = self.id_for(id);
        self.state.window_fx.insert(abs.to_static(), fx);
    }

    /// 视口**物理**尺寸（窗口客户区物理像素；锚定布局 / 全屏遮罩用）。
    #[inline]
    pub fn viewport_size(&self) -> Vec2 {
        let s = self.window.inner_size();
        Vec2::new(s.width as f32, s.height as f32)
    }

    /// **调试快照**（Rust 侧诊断）：把本帧"引擎眼里的世界"整成一个可打印的结构——
    /// 每个窗口的 **id / z / 本帧提交原点（`win_origins`）/ 尺寸 / 拖拽状态 / 持久位置**，
    /// 加焦点、文本焦点、鼠标、DPI、视口、帧号。
    ///
    /// 用途（`docs/DEBUGGING.md`）：
    /// - 排查"位置 / 层级 / 拖拽看着不对"时，**先看引擎状态**再怀疑渲染：
    ///   `log::info!("{}", ui.debug_dump())`（`RUST_LOG=rjw_ui=info`）或 `eprintln!`；
    /// - 也可交给外部工具（MCP / DAP 断点里 `println!("{}", ui.debug_dump())`）。
    ///
    /// 必须在**录制期**调用（`win_origins` 是帧内状态，`finish` 后清空）；
    /// 在 `f.ui(|ui| { ...; ui.debug_dump() })` 闭包末尾调用可拿到全部窗口。
    pub fn debug_dump(&self) -> UiDebugDump {
        let mut windows: Vec<UiWindowInfo> = Vec::new();
        // ⚠ 遍历**本帧录制过的窗口 id**（帧级、各段累加），不是 `Ui` 帧内的 `win_ids`
        // （**按 z 键** ⇒ 同一帧的多个 `WIN_TOPMOST` 浮层只会留下最后一个，嵌套子菜单
        // 从 dump 里消失）。原点取 `state.window_origins`（**按 id**）。
        for id in &self.state.frame_state.window_ids_seen {
            let Some(z) = self.state.window_z.get(id.as_str()).copied() else {
                continue;
            };
            let origin = self
                .state
                .window_origins
                .get(id.as_str())
                .copied()
                .unwrap_or(Vec2::ZERO);
            let size = self
                .state
                .window_sizes
                .get(id.as_str())
                .copied()
                .or_else(|| {
                    self.state
                        .window_rects
                        .get(id.as_str())
                        .map(|r| Vec2::new(r.w, r.h))
                })
                .unwrap_or(Vec2::ZERO);
            let ws = self.state.widgets.get(id.as_str());
            windows.push(UiWindowInfo {
                id: id.as_str().to_owned(),
                z,
                origin,
                size,
                dragging: ws.is_some_and(|w| w.dragging),
                press_panel: ws.and_then(|w| w.press_panel),
                stored_pos: self.state.panel_pos.get(id.as_str()).copied(),
                submit_pos: self.state.debug_submit.get(&z).copied(),
                clip: self.state.debug_clip.get(&z).copied(),
            });
        }
        windows.sort_by_key(|w| w.z);
        UiDebugDump {
            frame: self.state.frame,
            scale: self.scale,
            viewport: self.viewport_size(),
            mouse_px: self.mouse_screen,
            mouse_in_window: self.mouse_in_window,
            focused: self.state.focused.as_ref().map(|f| f.as_str().to_owned()),
            text_focus: self.state.text_focus().map(|f| f.id.as_str().to_owned()),
            windows,
        }
    }

    /// 按锚点计算**绝对物理 pos**（顶层容器用）：内容尺寸 `size` 在视口内按
    /// `anchor` 停靠、距视口边 `margin`（均为**物理像素**；内容超视口时 clamp 到
    /// 视口内不溢出）。返回 [`Position::Physical`]——直接传给 `label_at` / `add_at`
    /// 等（不参与 Logical→Physical 二次换算）。纯几何见 [`Self::anchor_pos_in`]。
    #[inline]
    pub fn anchor_pos(&self, a: Anchor, size: Vec2, margin: Vec2) -> Position {
        Position::Physical(Self::anchor_pos_in(self.viewport_size(), a, size, margin))
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

    // （`phys_rect` / `phys_f` 已删除：内部坐标一律物理像素，`self.scale` 仅用于
    // API 边界 `Size` / `Position` 的 Logical→Physical 换算，不参与布局/命中/绘制。）

    // ── 键盘导航（焦点链） ────────────────────────────────────

    /// 把控件登记进本帧焦点链（键盘导航用）。`rect` 为**相对当前容器**的局部矩形，
    /// 内部转成**绝对逻辑坐标**（焦点描边绘制 / 排序用）。**交互控件必须调用**。
    /// `id` 为**绝对 ID**（控件内 `self.id_for(..)` 所得——状态键 / 焦点 id 必须一致）。
    pub fn register_focus(&mut self, id: &IdAbsolute<'_>, rect: Rect, kind: FocusKind) {
        let abs = Rect::new(
            self.abs_base.x + rect.x,
            self.abs_base.y + rect.y,
            rect.w,
            rect.h,
        );
        // 记录焦点控件的**类型**（`UiState::text_focus` 据此区分"文本焦点"与
        // "按钮/滑块焦点"：后者不应阻断应用快捷键）。Tab 改焦点时在 `finish` 同步。
        if self.focused_is(id) {
            self.state.focused_kind = Some(kind);
        }
        self.focusables.push(FocusEntry {
            id: id.to_static(),
            win: self.painter.q.cur_win,
            kind,
            depth: self.painter.q.depth,
            rect: abs,
            clip: self.painter.q.clip,
        });
    }

    /// 本控件是否持有键盘焦点（`UiState.focused == id`；`id` 为**绝对 ID**）。
    #[inline]
    pub(crate) fn focused_is(&self, id: &IdAbsolute<'_>) -> bool {
        self.state.focused.as_ref().is_some_and(|f| f.as_str() == id.as_str())
    }

    /// **键盘激活**：Enter / Space 在本帧按下、本控件持有焦点且不在 IME 组合中
    /// → 视为一次点击（按钮 / 勾选 / 单选 / 下拉框用）。文本输入框与滑块不参与
    /// （前者走打字路径，后者用方向键调值）。`id` 为**绝对 ID**。
    pub fn key_click(&self, id: &IdAbsolute<'_>, kind: FocusKind) -> bool {
        if !self.focused_is(id) || kind == FocusKind::TextInput || kind == FocusKind::Slider {
            return false;
        }
        let composing = self
            .keyboard
            .ime_preedit()
            .is_some_and(|p| !p.is_empty());
        if composing {
            return false;
        }
        self.keyboard.key(KeyCode::Enter).down_edge()
            || self.keyboard.key(KeyCode::Space).down_edge()
    }

    // ── Debug UI / DebugDraw（调试 rjw_ui 自身 + 屏幕空间调试图元） ──

    /// 调试 UI 布局**开启**（运行时切换；等价于 [`UiInit::debug_layout`]）。
    ///
    /// 开启后 `finish` 为**每一个录制命令的矩形**画青色描边（覆盖在 UI 内容之上）——
    /// 可视化每个控件 / 容器的布局矩形与命中区域。
    #[inline]
    pub fn debug_layout(&mut self) -> &mut Self {
        self.debug_layout = true;
        self
    }

    /// 调试 UI 布局**关闭**（默认）。
    #[inline]
    pub fn without_debug_layout(&mut self) -> &mut Self {
        self.debug_layout = false;
        self
    }

    /// 屏幕空间线段（**绝对逻辑屏幕像素**，Y+ 向下；覆盖在 UI 内容之上；接受 `Vec2` 或 `(x, y)`）。
    pub fn debug_line(&mut self, a: impl Into<Vec2>, b: impl Into<Vec2>, width: f32, color: Color) {
        self.push_debug(
            DebugShape::Line {
                a: a.into(),
                b: b.into(),
                width,
            },
            color,
        );
    }

    /// 屏幕空间矩形边框（逻辑像素）。
    pub fn debug_rect_outline(&mut self, rect: Rect, width: f32, color: Color) {
        self.push_debug(DebugShape::RectOutline { rect, width }, color);
    }

    /// 屏幕空间圆环（`segments` 段折线近似；逻辑像素；接受 `Vec2` 或 `(x, y)`）。
    pub fn debug_circle_outline(
        &mut self,
        center: impl Into<Vec2>,
        radius: f32,
        segments: usize,
        width: f32,
        color: Color,
    ) {
        self.push_debug(
            DebugShape::CircleOutline {
                center: center.into(),
                radius,
                segments,
                width,
            },
            color,
        );
    }

    /// 屏幕空间十字标记（逻辑像素；接受 `Vec2` 或 `(x, y)`）。
    pub fn debug_cross(&mut self, center: impl Into<Vec2>, half: f32, width: f32, color: Color) {
        self.push_debug(
            DebugShape::Cross {
                center: center.into(),
                half,
                width,
            },
            color,
        );
    }

    /// 屏幕空间网格（`rect` 范围内按 `spacing` 竖线 + 横线；每方向最多 512 条）。
    pub fn debug_grid(&mut self, rect: Rect, spacing: f32, width: f32, color: Color) {
        self.push_debug(DebugShape::Grid { rect, spacing, width }, color);
    }

    /// 录制一条屏幕空间调试图元（进 `debug_queue`，坐标 = 绝对逻辑像素）。
    fn push_debug(&mut self, shape: DebugShape, color: Color) {
        self.painter.debug(shape, color);
    }

    /// **圆角矩形**（背景填充原语；绝对定位，`radius` 带单位）。
    ///
    /// **无纹理、无着色器改动**：CPU 把矩形镶嵌成三角形（硬体 + 边缘羽化带，
    /// 见 `crate::tess`）。半径接受任意值（含小数）；四角之和超过边长时按 CSS 规则
    /// **等比收缩**（[`CornerRadius::fit`]）；四角**全**为 0 时退化成普通四边形。
    ///
    /// `radius` 接受 `f32`（四角相同）或 [`CornerRadius`]（**只圆某些角**）：
    ///
    /// ```no_run
    /// # use rjw_ui::{CornerRadius, Position, Size, Ui};
    /// # fn demo(ui: &mut Ui, pos: Position, size: Size<glam::Vec2>) {
    /// // 四角相同
    /// ui.rounded_rect_at(pos, size, 8.0, rjw_color::Color::RED);
    /// // 只圆上面两个角（标签页 / 附着在工具栏下方的面板）
    /// ui.rounded_rect_at(
    ///     pos,
    ///     size,
    ///     CornerRadius { tl: 10.0, tr: 10.0, br: 0.0, bl: 0.0 },
    ///     rjw_color::Color::RED,
    /// );
    /// # }
    /// ```
    pub fn rounded_rect_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        radius: impl Into<Size<CornerRadius>>,
        color: Color,
    ) {
        self.painter.rounded_at(pos, size, radius, color);
    }

    /// **矢量图标**（绝对定位；`size` 为图标方框）。
    ///
    /// 图标是**画出来的几何**（[`Icon`]，单位方框内的凸多边形分片），与字体无关——
    /// `▾` / `✓` / `≡` 这类字符的可用性与宽度全由字体决定，字体缺字形就走 fallback。
    /// 几何按 [`Theme::feather`] 做边缘羽化（与圆角矩形同一套顶点 alpha 插值）。
    ///
    /// 框非方形时按 `min(w, h)` **居中等比**（图标永不形变）。
    ///
    /// ```no_run
    /// # use rjw_ui::{Icon, Position, Size, Ui};
    /// # fn demo(ui: &mut Ui, pos: Position, size: Size<glam::Vec2>) {
    /// ui.icon_at(pos, size, Icon::ChevronDown, rjw_color::Color::WHITE);
    /// # }
    /// ```
    pub fn icon_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        icon: Icon,
        color: Color,
    ) {
        self.painter.icon_at(pos, size, icon, color);
    }

    /// **矢量图标**（随布局流排布；与 [`Self::icon_at`] 同语义，位置来自当前容器游标）。
    ///
    /// 在 [`crate::UiAdd::row`] 容器内连续调用即得一条工具栏；`size` 决定图标方框
    /// （非方形时按 `min(w, h)` 居中等比，笔画不形变）。
    ///
    /// ```no_run
    /// # use rjw_ui::{Icon, Ui};
    /// # fn demo(ui: &mut Ui, c: rjw_color::Color) {
    /// ui.icon(glam::Vec2::new(16.0, 16.0), Icon::Check, c);
    /// # }
    /// ```
    pub fn icon(&mut self, size: impl Into<Size<Vec2>>, icon: Icon, color: Color) {
        let size = size.into().to_physical(self.scale);
        let pos = self.child_rect(size.x, size.y, Child::Expand).min();
        self.painter.icon_at(Position::Physical(pos), Size::Physical(size), icon, color);
    }

    /// **背景图**（绝对定位；`ImageBg` 决定铺排 / 染色 / 圆角遮罩）。
    ///
    /// 与 [`Self::rounded_rect_at`] 同一条 CPU 镶嵌路径：`Stretch` / `Fill` / `Center`
    /// 的 UV 是仿射映射 ⇒ **贴图与圆角遮罩共存**，且不额外产生 draw call（图片按纹理
    /// 切段，与字形 / 白纹理各一段）。
    ///
    /// ```no_run
    /// # use rjw_ui::{ImageBg, ImageFit, Position, Size, Ui};
    /// # fn demo(ui: &mut Ui, pos: Position, size: Size<glam::Vec2>, tex: u64) {
    /// // 等比覆盖 + 圆角遮罩
    /// let bg = ImageBg::new(tex, glam::Vec2::new(64.0, 64.0)).fit(ImageFit::Fill).radius(8.0);
    /// ui.image_at(pos, size, bg);
    /// # }
    /// ```
    pub fn image_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        bg: ImageBg,
    ) {
        self.painter.image_at(pos, size, bg);
    }

    /// **背景图**（随布局流排布；与 [`Self::image_at`] 同语义，位置来自当前容器游标）。
    pub fn image(&mut self, size: impl Into<Size<Vec2>>, bg: ImageBg) {
        let size = size.into().to_physical(self.scale);
        let pos = self.child_rect(size.x, size.y, Child::Expand).min();
        self.painter.image_at(Position::Physical(pos), Size::Physical(size), bg);
    }

    /// **矩形渐变**（绝对定位；背景填充原语）。
    ///
    /// `gradient` 接受 [`Gradient`] 或 `Color`（`Color: Into<Gradient>`，等价纯色）：
    ///
    /// ```ignore
    /// ui.gradient_rect_at(pos, size, Color::RED);                                  // 纯色
    /// ui.gradient_rect_at(pos, size, Gradient::vertical(Color::RED, Color::BLUE)); // 上下
    /// ui.gradient_rect_at(pos, size, Gradient::horizontal(a, b));                  // 左右
    /// ui.gradient_rect_at(pos, size, Gradient::rotated(a, b, 0.5));                // 任意角
    /// ui.gradient_rect_at(pos, size, Gradient::corners(tl, tr, bl, br));           // 四角
    /// ```
    ///
    /// **无纹理**：四角颜色经顶点色由光栅化器双线性插值（详见 [`Gradient`]）。
    pub fn gradient_rect_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        gradient: impl Into<Gradient>,
    ) {
        self.painter.gradient_at(pos, size, gradient);
    }

    /// **矩形渐变**（随布局流排布；与 [`Self::gradient_rect_at`] 同语义，位置来自当前容器游标）。
    pub fn gradient_rect(
        &mut self,
        size: impl Into<Size<Vec2>>,
        gradient: impl Into<Gradient>,
    ) {
        let size = size.into().to_physical(self.scale);
        let pos = self.child_rect(size.x, size.y, Child::Expand).min();
        self.painter.gradient_at(Position::Physical(pos), Size::Physical(size), gradient);
    }

    /// **元素序提示**（控件作者用）：取"当前录制位置"的元素序（`seq + 1`）。
    ///
    /// `elem` 决定**同一窗口内**命令的绘制顺序（排序键 `(win, depth, elem, group, seq)`）：
    /// `push_panel_like` / `push_text_rect` / `slider_at` 等原语各自取"录制时的
    /// `seq + 1`"，而 [`Self::push_draw`] 默认写 `elem = 0`（容器装饰，画在本容器
    /// **所有元素之下**，如窗口背景/边框）。
    ///
    /// **组合控件里"后画的装饰"必须用本方法**：展开箭头、拖拽手柄、分隔线若写死
    /// `0` / `1`，就会被本控件自己的背景 / 文本框盖住（历史 bug：`NumberInput` 的
    /// 拖拽手柄与分隔线、`ColorPicker` 的展开箭头整块看不见——元素序小的先画）。
    ///
    /// 等价于 `ui.painter().elem_hint()`（[`crate::Painter`] 的默认 elem 就是它）。
    #[inline]
    pub fn elem_hint(&self) -> u32 {
        self.painter.elem_hint()
    }

    /// **借出绘制器**（[`crate::Painter`]）：一个**绘制块**取一次，块内所有原语
    /// 不再逐参数传 `elem` / 环境裁剪。
    ///
    /// ```no_run
    /// # use rjw_transform::Rect;
    /// # use rjw_ui::Ui;
    /// # fn demo(ui: &mut Ui, rect: Rect) {
    /// let mut p = ui.painter();
    /// p.panel(rect, rjw_color::Color::BLACK, rjw_color::Color::WHITE, 1.0, 4.0);
    /// # }
    /// ```
    ///
    /// **沙箱内直接可用**：Clip 沙箱 / ScrollView 可视区 / 严格窗口内容裁剪就是
    /// [`Painter::clip`] 上的**当前强制层**（`view_at` / `scroll_at` 进入时写入），
    /// 录出的命令自带它 ⇒ 沙箱里的控件**不需要**任何额外动作。只有"要一层与当前
    /// **不同**的裁剪"时才用 [`Self::painter_clipped`]（更窄，或 `None` 主动不裁）。
    ///
    /// ⚠ **绘制与 `ui.*` 不能交错**：本方法借 `&mut self`，painter 存活期内
    /// `ui.text_size` / `ui.hit_abs` / `ui.allocate*` 都借不到 `ui`。
    /// 顺序是「先量 → 画 → 再量」，不是一个 painter 画到底。
    #[inline]
    pub fn painter(&mut self) -> &mut Painter {
        &mut self.painter
    }

    /// **借出绘制器并覆盖环境裁剪层**（只影响闭包内录的命令，块外自动恢复）。
    ///
    /// 等价于 `ui.painter().clipped(clip, |p| …)`；`clip = None` = 不裁剪。
    /// 用于"本控件内容裁到自己框内、但**不改** `Ui` 的强制裁剪层"——改那一层会连带
    /// 影响子控件 / 兄弟控件（`view_at` / `scroll_at` 就是这么用的）。
    ///
    /// ⚠ **已经在 Clip 沙箱里时不需要它**：沙箱裁剪已经是 painter 的当前层，
    /// 见 [`Self::painter`]。
    #[inline]
    pub fn painter_clipped<R>(
        &mut self,
        clip: Option<Rect>,
        f: impl FnOnce(&mut Painter) -> R,
    ) -> R {
        self.painter.clipped(clip, f)
    }

    /// 录制一条绘制命令（`elem` 由调用方给：`0` = 容器装饰层，画在本容器元素之下）。
    ///
    /// ⚠ 组合控件内"画在自家背景之上"的装饰传 [`Self::elem_hint`]，**不要**写死 `0`。
    pub(crate) fn push_draw(&mut self, kind: DrawKind, rect: Rect, elem: u32) {
        self.painter.draw(kind, rect, elem);
    }

    /// 按样式 push **背景 + 边框**。
    ///
    /// - `radius > 0`：外圈 border 色圆角 + 内圈背景刷圆角（内缩 `border_w`），
    ///   即"圆角边框"。两层都由 CPU 镶嵌（`crate::tess`），因此**圆角与渐变可共存**。
    /// - `radius == 0`：`Solid` + `Border`；背景刷为渐变时走 `Rect(Gradient)`。
    ///
    /// `bg` 接受 [`Color`] 或 [`crate::Brush`]。渐变**锚定在 `rect` 上**——
    /// 内圈即使内缩 `border_w`，颜色按其在 `rect` 中的相对位置重采样，
    /// 不会整体平移（`resample_gradient`）。
    ///
    /// `elem`：元素序（装饰背景传 0；控件背景传 `self.painter.q.seq + 1`）。
    /// **控件作者绘制原语**（逻辑坐标，内部 ×scale 取整到物理像素）。
    #[allow(clippy::too_many_arguments)]
    pub fn push_panel_like(
        &mut self,
        rect: Rect,
        bg: impl Into<crate::style::Brush>,
        border: Color,
        border_w: f32,
        radius: impl Into<CornerRadius>,
        elem: u32,
    ) {
        self.push_panel_like_img(rect, bg, None, border, border_w, radius, elem);
    }

    /// **投影**（窗口 / 面板的**顶点色软阴影**；控件作者绘制原语）。
    ///
    /// 画在 `rect`（本体矩形）**之下**：命令 `elem = 0`（容器装饰层，先于本体背景入队）
    /// ⇒ 本体背景、内容、边框都盖在它上面；而窗口自己的整段提交在更低 z 的窗口之后 ⇒
    /// **投影落在下面的窗口上**（正确的投影观感）。
    ///
    /// `rect` 即本体矩形（**内轮廓恒在本体边缘**，浓度从本体边向外单调衰减，没有
    /// "等浓度暗带"——`shadow.offset` 由镶嵌器按圈数分摊，见 [`crate::tess`]）。
    /// 纯顶点色 + CPU 镶嵌：无纹理、无新 draw call，且**进窗口顶点缓存**（静态窗口零开销）。
    /// `shadow.blur <= 0`（或全透明色）时不产生任何命令。
    pub fn push_panel_shadow(
        &mut self,
        rect: Rect,
        shadow: &crate::style::ShadowStyle,
        radius: impl Into<CornerRadius>,
    ) {
        self.painter.shadow(rect, shadow, radius);
    }

    /// **右下角缩放柄图案**（窗口/面板局部坐标；`size` = 本体尺寸）。
    ///
    /// 只画图案 —— **命中区不在这里**（在 `window_impl` 里由 [`Self::resize_handle`] 建立，
    /// 其大小跟随 `grip.extent()`）。所以 [`GripShape::Hidden`] 时"看不见但仍能拖"。
    ///
    /// 形状 / 颜色 / 尺寸 / 个数全部来自 [`GripStyle`]：`Squares` = 沿右下对角线的
    /// 递减小方块（历史观感）、`Bars` = 内置矢量图标 [`Icon::Grip`]（三条横线，
    /// 与字体无关）、`Hidden` = 不画。
    pub fn push_resize_grip(&mut self, size: Vec2, grip: &GripStyle) {
        self.push_resize_grip_at(Rect::new(0.0, 0.0, size.x, size.y), grip);
    }

    /// 同 [`Self::push_resize_grip`]，但**指定本体矩形**（当前容器局部坐标）：
    /// 图案画在该矩形的**右下角内侧**——文本输入框（`rect` 不为 `(0,0)`）用它。
    pub fn push_resize_grip_at(&mut self, rect: Rect, grip: &GripStyle) {
        if !grip.is_visible() {
            return;
        }
        let (x, y) = (rect.x, rect.y);
        let (w, h) = (rect.w, rect.h);
        match grip.shape {
            GripShape::Hidden => {}
            GripShape::Squares => {
                for k in 0..grip.count {
                    let o = grip.step * (k as f32 + 1.0);
                    self.push_solid_rect(
                        Rect::new(x + w - o, y + h - o, grip.size, grip.size),
                        grip.color,
                    );
                }
            }
            GripShape::Bars => {
                // 三条**实心横杠**（宽 = `size*count`，高 = `size`，间距 = `step`）。
                //
                // ⚠ 不用 `Icon::Grip` 图标：图标走 `push_icon`，每条杠只有 `size` 高
                // （默认 4 逻辑像素），再叠上 `Theme::feather` 的羽化带（默认 1 逻辑像素
                // ⇒ 每侧 0.5）就把三条糊成一坨（用户实测："三横看起来是斜的一坨"）。
                // 实心矩形没有羽化，任意尺寸都读得出三条。
                let bw = grip.size * grip.count as f32;
                let right = x + w - grip.step;
                for k in 0..grip.count {
                    let o = grip.step * (k as f32 + 1.0);
                    self.push_solid_rect(
                        Rect::new(right - bw, y + h - o, bw, grip.size),
                        grip.color,
                    );
                }
            }
            GripShape::Diagonal => {
                // 三条**斜线**（45°，从左下到右上）：一条斜线没法用 `push_solid_rect`
                // （它只有轴对齐矩形）⇒ 走矢量图标 `Icon::GripDiagonal`。
                // 方框取 `size*count*1.5`（比横线版大 1.5×）：三条斜线在 `size*count`
                // 的小方框里间距只有 ~1px，羽化会把它们糊成一片。
                let d = grip.size * grip.count as f32 * 1.5;
                let m = grip.step;
                self.icon_at(
                    Position::Physical(Vec2::new(x + w - d - m, y + h - d - m)),
                    Size::Physical(Vec2::splat(d)),
                    Icon::GripDiagonal,
                    grip.color,
                );
            }
        }
    }

    /// 同 [`Self::push_panel_like`]，另带可选**背景图**（[`ImageBg`]）。
    ///
    /// 绘制层次：**背景刷 → 背景图 → 边框**（图在刷之上，半透明图能透出底色；边框
    /// 恒盖住图的边缘）。圆角遮罩**恒用面板 `radius`**（图片自带的 `radius` 被忽略，
    /// 免得"图与面板圆角不一致"这种要靠肉眼发现的错）。
    ///
    /// ⚠ **背景图与边框在两种半径下都要画**：曾经它们被写在"圆角分支"里，于是
    /// `radius == 0`（直角）的面板**静默丢掉背景图**——演示里"Tile（1:1 平铺，直角）"
    /// 那个窗口就是受害者（平铺为了 UV 环绕特意用直角）。现在"背景刷"按
    /// `(是否直角, 是否纯色)` 一次 `match` 决定，图与边框移出分支。
    #[allow(clippy::too_many_arguments)]
    pub fn push_panel_like_img(
        &mut self,
        rect: Rect,
        bg: impl Into<crate::style::Brush>,
        img: Option<ImageBg>,
        border: Color,
        border_w: f32,
        radius: impl Into<CornerRadius>,
        elem: u32,
    ) {
        self.painter.panel_img_elem(rect, bg, img, border, border_w, radius, elem);
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
    /// 测量文本自然尺寸（**物理像素**，ceil 取整；控件作者在 [`Widget::size`] 测量用；
    /// 内部排版缓冲按物理字号缓存，`family = None` = 系统默认字体）。
    pub fn text_size(&mut self, s: &str, size: f32, family: Option<&str>) -> Vec2 {
        let buf = self.cache_buffer(s, size, family);
        Text::measure_buffer(&buf).ceil()
    }

    /// 按**换行宽度**测量文本自然尺寸：`wrap > 0` 时文本在宽度内自动换行
    /// （宽 = min(自然宽, wrap)，高 = 行数 × 行高）；否则同 [`Self::text_size`]。
    pub fn text_size_wrap(&mut self, s: &str, size: f32, family: Option<&str>, wrap: f32) -> Vec2 {
        let buf = self.cache_buffer_wrap(s, size, family, wrap);
        Text::measure_buffer(&buf).ceil()
    }

    // 剪贴板快捷键（Ctrl+C/V/X/A）共用实现已迁至 `crate::edit::clipboard_shortcuts`
    // （纯逻辑，单行 / 多行输入框共用），本处不再保留。

    /// **控件自持排版缓冲**：`WidgetState::text_buf` 命中复用（key 含文本/字号/字体/
    /// 换行宽/**行距**/版本），未命中直接构建——**不写** `UiState::text_buffers` 全局缓存
    /// （文本频繁变化的输入框不污染静态标签缓存）。
    ///
    /// `line_mult`：行高 = 字号 × 行距倍率（1.0 = 无行距；TextArea 多行用
    /// `Theme::line_spacing`，默认 [`crate::DEFAULT_LINE_SPACING`] = 1.2 加行距）。
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
        // 字号 / 换行宽**物理**（内部全物理；取整到像素，防亚像素模糊）。
        let size_px = size.round();
        let wrap_px = wrap_logical.round().max(0.0);
        let mult_bits = line_mult.to_bits();
        let weight = self.theme.font_weight;
        let key = format!(
            "{s}\u{1}{size_px}\u{1}{}\u{1}{wrap_px}\u{1}{mult_bits}\u{1}{}\u{1}{TEXT_LINE_HEIGHT_VERSION}",
            family.unwrap_or(""),
            weight.0
        );
        if let Some((k, b)) = self.state.widgets.get(id).and_then(|w| w.text_buf.as_ref())
            && *k == key {
                return b.clone();
            }
        let lh = (size_px * line_mult.max(1.0)).round();
        // 样式：字号 / 行高（行高 = 字号 × 行距倍率）/ 左对齐 / 可选字体族 / 全局字重。
        // （`TextStyle` 是唯一文本样式类型；此处只做机械适配，排版输入与旧
        //  `Text::create_buffer_wrap` 完全一致。）
        let style = match family {
            Some(f) if !f.is_empty() => TextStyle::new().font_family(f),
            _ => TextStyle::new(),
        }
        .size(size_px)
        .line_height(lh)
        .weight(weight)
        .align(Align::Left);
        let buf = self.text.buffer(s, &style, wrap_px, CachePolicy::User);
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
    pub(crate) fn ellipsized(
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
    /// `wrap <= 0` = 不换行（默认宽裕宽度）；`> 0` = 按该**物理像素**宽度换行
    /// （换行宽度参与缓存键，不同宽度各自缓存）。
    ///
    /// 行高 = 字号（`size_px`）× 主题行距倍率（`wrap <= 0` 的单行文本不受行距影响：
    /// 盒子高度仍是字号，保证文本框内字形垂直居中的位置正确）。
    /// 缓存键包含 `TEXT_LINE_HEIGHT_VERSION` 与行距本身，修改行高策略后旧缓存自动失效。
    fn cache_buffer_wrap(
        &mut self,
        s: &str,
        size: f32,
        family: Option<&str>,
        wrap: f32,
    ) -> Arc<Buffer> {
        // 物理字号取整：字形在像素网格上，避免亚像素渲染模糊；测量/绘制/缓存键一致
        let size_px = size.round();
        let wrap_px = wrap.round().max(0.0);
        // 换行文本按主题行距排版（单行不受影响）；倍率进缓存键（不同行距各自缓存）。
        let mult = if wrap_px > 0.0 { self.theme.line_spacing } else { 1.0 };
        // 全局字重：改字形 + 步进宽度 ⇒ **必须进缓存键**（否则换字重后旧排版缓冲被判命中）。
        let weight = self.theme.font_weight;
        let key = (
            s.to_owned(),
            size_px.to_bits(),
            family.map(|f| f.to_owned()),
            wrap_px.to_bits(),
            (mult.to_bits(), weight.0, TEXT_LINE_HEIGHT_VERSION),
        );
        if let Some(b) = self.state.text_buffers.get_mut(&key) {
            // 命中：刷新"最后使用帧号"（帧级近似 LRU 驱逐依据）
            b.1 = self.state.frame;
            return b.0.clone();
        }
        // 行高 = 字号 × 行距倍率（单行 = 字号，字形在行盒中垂直居中更准确）
        let lh = (size_px * mult.max(1.0)).round();
        let style = match family {
            Some(f) if !f.is_empty() => TextStyle::new().font_family(f),
            _ => TextStyle::new(),
        }
        .size(size_px)
        .line_height(lh)
        .weight(weight)
        .align(Align::Left);
        let buf = self.text.buffer(s, &style, wrap_px, CachePolicy::User);
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
    ///
    /// 等价于 `ui.painter().solid(rect, color)`；保留为方法是为了不改既有调用点。
    #[inline]
    pub fn push_solid_rect(&mut self, rect: Rect, color: Color) {
        self.painter.solid(rect, color);
    }

    /// **控件作者绘制原语**：矩形边框（逻辑坐标；画在矩形内边缘，宽度取整到物理像素）。
    ///
    /// 等价于 `ui.painter().border(rect, color, width)`。
    #[inline]
    pub fn push_border_rect(&mut self, rect: Rect, color: Color, width: f32) {
        self.painter.border(rect, color, width);
    }

    /// 推送一条文本绘制命令（供 widget 层与 `*_at` 方法共用；`clip` 为文本局部裁剪，
    /// 外层裁剪自动取当前容器 `self.painter.q.clip`；`buf = Some` 时直接用预排版缓冲）。
    /// **控件作者绘制原语**（逻辑坐标；`family` 传 `None` = 系统默认字体）。
    pub fn push_text_rect(
        &mut self,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
        family: Option<Arc<str>>,
        align: TextAlign,
        valign: TextVAlign,
        clip: Option<Rect>,
        buf: Option<Arc<Buffer>>,
    ) {
        self.painter.text(rect, text, size, color, family, align, valign, clip, buf);
    }

    /// **不服从内容裁剪的文本绘制**（控件作者原语）：
    ///
    /// 语义 = [`Self::push_text_rect`] 且**不附加任何软层（内容裁剪）**——调用方
    /// 承诺文本**内容自洽**（自动换行后高 = 自然高、"…"省略后宽 = 分配宽、滚动
    /// 内容受限），无需按控件边界裁剪。**仍服从强制层**（ScrollView 可视区 /
    /// Clip 沙箱，即 `self.painter.q.clip`）：父级如 ScrollView 强制裁切时躲不掉；无 Scroll
    /// 的普通容器本来就没有强制层 → 自洽内容画出界（自洽内容本就不会出界）。
    pub fn push_text_rect_noclip(
        &mut self,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
        family: Option<Arc<str>>,
        align: TextAlign,
        valign: TextVAlign,
        buf: Option<Arc<Buffer>>,
    ) {
        self.push_text_rect(rect, text, size, color, family, align, valign, None, buf)
    }

    /// 鼠标左键状态（含本帧边沿；控件作者交互判断用）。
    #[inline]
    pub fn mouse_left(&self) -> KeyState {
        self.mouse.button(MouseButton::Left)
    }

    /// **记入当前容器**：绝对放置的控件矩形也要算进容器尺寸（`Frame::content_bounds`）。
    ///
    /// 所有"显式 rect"的公开控件入口（`button_at_styled` / `slider_at_drag` /
    /// `checkbox_at_styled` / `radio_at` / `text_input_at` / `text_area_impl` /
    /// `label_at`）都调它——这样"把控件放在容器外"这件事**要么让容器长大、要么
    /// 被 `Clip` 裁掉**，不会出现"看得见点得着却不在容器矩形里"的中间态。
    /// 与鼠标无关（布局期调用），故布局不会随鼠标漂移。
    ///
    /// **公开理由**（控件作者公开面）：控件核心正在往 `widgets/<name>.rs` 搬，而
    /// `widgets` 是 `ui` 的**兄弟模块** ⇒ 私有方法对它们不可见。与其开 `pub(crate)`
    /// 后门，不如把它作为"显式 rect 的控件都该调"的公开原语（自定义控件同理）。
    #[inline]
    pub fn note_placed(&mut self, rect: Rect) {
        if let Some(frame) = self.frames.last_mut() {
            frame.note_content(rect);
        }
    }

    /// 鼠标绝对坐标 → 当前容器局部坐标（逻辑像素，字段运算避免方法借用）。
    ///
    /// **控件作者公开面**：自定义拖拽区（如取色器的 SV 平面 / 色相条）拿它做
    /// "点哪取哪"的绝对映射；[`Self::mouse_screen`] 是屏幕坐标，只有配合容器原点才有意义。
    #[inline]
    pub fn mouse_local(&self) -> Vec2 {
        self.mouse_logical - self.abs_base
    }

    /// 鼠标局部坐标的 x（内部热路径用；语义同 [`Self::mouse_local`]）。
    #[inline]
    fn mouse_local_x(&self) -> f32 {
        self.mouse_local().x
    }

    /// 局部矩形（逻辑）→ 命中测试（与逻辑鼠标坐标比较；含窗口外判定、窗口遮挡与
    /// **控件级遮挡**）。
    ///
    /// `owner` = 本控件的**绝对 ID**：用于控件级遮挡的身份判定（同一控件的多个区域
    /// 互不遮挡）。控件作者交互判断用（与 [`Self::mouse_left`] /
    /// [`hit::update_interact`](crate::hit::update_interact) 组合）。
    ///
    /// # 遮挡（"重叠控件被一起触发"修复）
    ///
    /// 同一窗口 / 面板内，**后录制 = 画在上面**的控件优先：鼠标下若有别的控件
    /// 记录得比我晚且覆盖此处，本控件**不响应**（点击 / 悬停 / 拖拽都不响应），
    /// 重叠区域只有最上层那一个控件被触发。判定用**上一帧**登记的区域
    /// （[`crate::hit::widget_occluded`]）——本帧后面的控件还没录制，无法参与判定。
    ///
    /// ⚠ **自定义控件务必传自己的绝对 ID**（不是容器的 id）：传容器 id 会让同容器内
    /// 的所有控件"互不遮挡"，传别人的 id 会让自己永远被那个控件挡住。
    ///
    /// # 其余拦截
    ///
    /// - **窗口遮挡**：鼠标下若有更高 z 的窗口盖住本控件所在窗口 → 不响应，
    ///   累加 [`UiState::occluded_hits`](crate::UiState::occluded_hits)（诊断）；
    /// - **强制裁剪层**：鼠标在 [`Self::clip`]（ScrollView 可视区 / Clip 沙箱）
    ///   之外时不命中——修复"滚出可视区的控件边缘仍可交互"缺口。
    #[inline]
    pub fn hit_abs(&mut self, owner: &IdAbsolute<'_>, local: &Rect) -> bool {
        self.hit_impl(Some(owner), local)
    }

    /// **"本体"命中**（窗口 / 面板 / 浮层的**整块区域**，不是控件）：与 [`Self::hit_abs`]
    /// 的区别是**不参与控件级遮挡**——本体**包含**它内部的子控件，若也走控件级遮挡，
    /// 鼠标停在子控件上时本体就会被自己的子控件判成"被挡住"（浮层会误判"点在面板外"
    /// 而收起）。窗口 / 面板本体的层级由**窗口遮挡**（z-order）负责，与控件级遮挡正交。
    ///
    /// 也不登记为遮挡区域：本体不需要挡住别的控件（那是窗口遮挡的事）。
    #[inline]
    pub fn hit_body_abs(&mut self, local: &Rect) -> bool {
        self.hit_impl(None, local)
    }

    /// [`Self::hit_abs`] / [`Self::hit_body_abs`] 的公共实现
    /// （`owner = None` ⇒ 本体：不做控件级遮挡、也不登记）。
    fn hit_impl(&mut self, owner: Option<&IdAbsolute<'_>>, local: &Rect) -> bool {
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
        if let Some(c) = self.painter.q.clip
            && !c.contains_point(self.mouse_logical) {
                return false;
            }
        // **窗口自身的可命中范围**（内容被裁切时才有）：裁掉的部分不得命中 ——
        // 否则"看不见的控件"还能点到（幽灵控件）。放在**登记命中区之前**：越界的命中
        // 既不响应，也不会把窗口的遮挡矩形撑到窗口外面去。
        if let Some(limit) = self.cur_win_hit_limit
            && !limit.contains_point(self.mouse_logical)
        {
            self.state.occluded_hits += 1;
            return false;
        }
        // **窗口遮挡**（点击穿透修复）：鼠标下若有更高 z 的窗口（`win=0` 内容被任意
        // 窗口）覆盖本控件所在窗口 → 本窗口不得响应——重叠区域只让最上层窗口交互，
        // 背后窗口的控件不会误触发。窗口矩形来自 [`UiState::window_rects`]（跨帧缓存）。
        if window_occluded(self.painter.q.cur_win, self.mouse_logical, self.window_rects_iter()) {
            // 命中但被遮挡 → 记录诊断计数（未响应）。
            self.state.occluded_hits += 1;
            return false;
        }
        let Some(owner) = owner else {
            return true;
        };
        // 当前窗口的**可见交互范围**并集（`window_rects[z]` 用它扩展，见字段文档）。
        // ⚠ 这里只做"遮挡范围"的记录，**不**参与容器尺寸（容器尺寸由
        // `Frame::note_content` 在布局期记，与鼠标无关——否则布局会随鼠标漂移）。
        self.win_hit_bounds = Some(match self.win_hit_bounds {
            Some(b) => b.union(&abs),
            None => abs,
        });
        // **控件级遮挡**：登记自己（供下一帧判定"谁盖住谁"），并检查上一帧里是否有
        // 更上层的别的控件覆盖此处。登记只在"几何命中"之后发生——遮挡判定只关心
        // 鼠标下那一处，鼠标不在自己矩形内时无需登记。
        let me = id_hash(owner);
        let key = self.painter.q.seq;
        let blocked = widget_occluded(
            me,
            key,
            self.mouse_logical,
            self.state.prev_hit_regions.iter().copied(),
        );
        self.state.hit_regions.push(HitRegion {
            owner: me,
            key,
            rect: abs,
            clip: self.painter.q.clip,
        });
        let traced = std::env::var_os("RJ_HIT_TRACE").is_some();
        if blocked {
            self.state.widget_occluded_hits += 1;
            if traced {
                eprintln!(
                    "hit[frame {}] {} BLOCKED-by-widget rect=({},{},{},{}) mouse=({},{})",
                    self.state.frame, owner.as_str(), abs.x, abs.y, abs.w, abs.h,
                    self.mouse_logical.x, self.mouse_logical.y
                );
            }
            return false;
        }
        if traced {
            eprintln!(
                "hit[frame {}] {} OK rect=({},{},{},{}) mouse=({},{})",
                self.state.frame, owner.as_str(), abs.x, abs.y, abs.w, abs.h,
                self.mouse_logical.x, self.mouse_logical.y
            );
        }
        // **记下"本帧由谁认领了按下"**（帧末复核用，见 `resolve_widget_press`）：
        // 连同**所在窗口的绝对 ID**一起记（复核时要解它的**当前** z——不能在复核时拿认领
        // 时的旧 z 比，那样窗口会把自己判成"被别人盖住"，把窗口内所有拖拽都撤掉）。
        // 只记第一个认领者——正常情况下（控件级遮挡生效）也只有一个。
        if self.mouse_left().down_edge() && self.state.frame_state.press_widget.is_none() {
            self.state.frame_state.press_widget =
                Some((owner.to_static(), self.cur_win_id.clone()));
        }
        true
    }

    /// **窗口遮挡判定用的窗口矩形迭代器**（`(z, rect)`；逻辑像素）。
    ///
    /// ⚠ 表按**窗口绝对 ID** 键（[`UiState::window_rects`]），这里把每条的 z **解成"当前
    /// z"**：矩形是上一帧录下的，而 z 可能在帧末被"点击置顶"改过——若按矩形写入时的旧 z
    /// 参与比较，被抬高的那个窗口就会**在它下面的窗口面前"消失"一帧**（它画在上面却挡不住
    /// 别人）。按 ID 查当前 z 从根上消除这处错位。
    #[inline]
    fn window_rects_iter(&self) -> impl Iterator<Item = (u32, Rect)> + '_ {
        self.state.window_rects.iter().map(|(id, &r)| {
            (
                self.state.window_z.get(id.as_str()).copied().unwrap_or(0),
                r,
            )
        })
    }

    /// **诊断**：当前窗口 z-order（按 z 升序）：`(id, z)`。
    pub fn window_order(&self) -> Vec<(String, u32)> {
        let mut v: Vec<(String, u32)> = self
            .state
            .window_z
            .iter()
            .map(|(id, &z)| (id.as_str().to_owned(), z))
            .collect();
        v.sort_by_key(|&(_, z)| z);
        v
    }

    /// **诊断**：鼠标下**最上层**的窗口（`id, z`）——重叠点击时唯一可交互的窗口；
    /// 鼠标不在任何窗口上时返回 `None`。窗口矩形来自跨帧缓存（含本帧已录制的窗口）。
    pub fn window_under_mouse(&self) -> Option<(String, u32)> {
        let mut best: Option<(String, u32)> = None;
        for (id, &z) in &self.state.window_z {
            if let Some(r) = self.state.window_rects.get(id.as_str())
                && r.contains_point(self.mouse_logical) {
                    match &best {
                        Some((_, bz)) if *bz >= z => {}
                        _ => best = Some((id.as_str().to_owned(), z)),
                    }
                }
        }
        best
    }

    /// 当前容器为子项分配局部矩形（`finish` 前任何位置可用：顶层由**根容器**
    /// （[`Ui::begin`] 内建，可用宽 = 视口物理宽）流式堆叠，容器 / 窗口内
    /// 用各自的帧）。控件作者做"占光标"式自定义容器时用（相对当前容器内容原点）。
    ///
    /// `child`：[`Child::Expand`]（默认，撑大父级）/ [`Child::Fit`]（不撑大父级，
    /// 对应 [`crate::widgets::Expansion::DisableAutoExpansion`]）——取代旧的裸布尔参数。
    pub fn child_rect(&mut self, w: f32, h: f32, child: Child) -> Rect {
        self.frames
            .last_mut()
            .expect("root frame")
            .child_rect_exp(w, h, child == Child::Expand)
    }

    /// **放置控件**（[`crate::widgets::Widget`] trait）：容器内**占光标**；尺寸由控件
    /// 自己在 `ui()` 里就地申请（[`Self::allocate`] 一族），因此这里只是把 `Ui` 交给它。
    ///
    /// 属性化 builder 示例：`ui.add(Button::new("ok", "确定").color(Color::WHITE))`。
    /// 容器包装（`Panel` / `Pack` / `Grid` / `Window` / `Scroll` / `FlexCtx`）经
    /// [`UiAdd`] 提供同样的 `add` / `add_at` 与全部便捷方法（`p.button` / `p.label` 等）。
    pub fn add(&mut self, w: impl crate::widgets::Widget) -> crate::widgets::Response {
        // 尺寸类（`Widget::size_class`）：在 `ui()` 之前交给当前 frame —— 水平行据此
        // 决定"单行钉标准高 / 多行可撑高"（见 `crate::widgets::SizeClass`）。
        if let Some(f) = self.frames.last_mut() {
            f.set_next_class(w.size_class());
        }
        let resp = w.ui(self);
        // 兜底清掉：控件可能一次 `child_rect` 都没走（`add_at` 的绝对定位 / 自绘控件），
        // 留着会让**下一个**子项拿到错的尺寸类。
        if let Some(f) = self.frames.last_mut() {
            f.set_next_class(crate::widgets::SizeClass::SingleLine);
        }
        resp
    }

    /// **绝对定位放置控件**（`pos` 相对当前容器内容原点；**不占光标**）。
    ///
    /// 实现 = 给 `Ui` 打一个**一次性放置覆盖**，控件的第一次申请（[`Self::allocate`] /
    /// [`Self::allocate_sense`] / …）消费它 ⇒ 控件自身的 `ui()` 不必关心"我是被 `add`
    /// 还是 `add_at` 放的"。
    ///
    /// ⚠ 只作用于**第一次**申请：控件要摆多个矩形时用 [`Self::allocate_at`]。
    pub fn add_at(
        &mut self,
        pos: impl Into<Position>,
        w: impl crate::widgets::Widget,
    ) -> crate::widgets::Response {
        self.place_once = Some(pos.into().to_physical(self.scale));
        if let Some(f) = self.frames.last_mut() {
            f.set_next_class(w.size_class());
        }
        let resp = w.ui(self);
        // 兜底清掉（控件可能一次申请都没做）。
        self.place_once = None;
        if let Some(f) = self.frames.last_mut() {
            f.set_next_class(crate::widgets::SizeClass::SingleLine);
        }
        resp
    }

    // ── 申请布局（控件作者用：参考 egui 的 allocate_*） ──────────
    //
    // ⚠ **尺寸一律是物理像素 `Vec2`**（与"`Theme` 已预乘、内部全物理像素"的约定一致）；
    // 要写逻辑单位就在自己的 API 边界换算（`Size::Logical(x).to_physical(ui.scale())`）。
    // 这里**刻意不收** `impl Into<Size<Vec2>>`：那会把已经是物理像素的测量结果
    // （`text_size` / 主题常量）再乘一次 DPI —— 实测 bug：scale = 1.5 时所有控件
    // 长到 1.5 倍，固定宽窗口的尺寸也跟着变。

    /// **占光标申请矩形**（撑大父级；**物理像素**）。
    ///
    /// 控件 `ui()` 的第一步。⚠ **先量后申请**：`avail_w()` / `text_size()` 要在本调用
    /// **之前**取值——申请会推进容器光标，之后的 `avail_w()` 是"下一项"的约束。
    #[inline]
    pub fn allocate(&mut self, size: Vec2) -> Rect {
        self.allocate_mode(size, crate::widgets::Expansion::UnlimitedExpansion)
    }

    /// **占光标申请矩形**，并指定**膨胀模式**（取代旧 `Widget::expansion()`）：
    /// - [`Expansion::UnlimitedExpansion`]：撑大父级（默认）；
    /// - [`Expansion::DisableAutoExpansion`]：不撑大父级（装饰件 / 分隔线）；
    /// - [`Expansion::LimitedInParent`]：宽度压到父级可用宽（`avail_w`）。
    pub fn allocate_mode(&mut self, size: Vec2, mode: crate::widgets::Expansion) -> Rect {
        self.allocate_rect(size, mode)
    }

    /// **绝对定位申请矩形**（不占光标；`pos` 相对当前容器内容原点，`size` 物理像素）。
    ///
    /// 与 [`Self::add_at`] 的区别：这是控件**自己在 `ui()` 里**决定位置（比如一个控件
    /// 要摆多块），`add_at` 是调用方决定控件的落点。
    pub fn allocate_at(&mut self, pos: impl Into<Position>, size: Vec2) -> Rect {
        let pos = pos.into().to_physical(self.scale);
        let rect = Rect::new(pos.x, pos.y, size.x, size.y);
        self.note_placed(rect);
        rect
    }

    /// **申请 + 一次性收交互**（egui `allocate_exact_size` 的对应物；`size` 物理像素）：
    /// 命中测试 / 焦点链 / 按下认领（`sense.drag`）/ 跨帧状态一次做完。
    #[inline]
    pub fn allocate_sense(
        &mut self,
        id: &str,
        size: Vec2,
        sense: crate::widgets::Sense,
    ) -> (Rect, crate::widgets::Response) {
        self.allocate_sense_mode(id, size, crate::widgets::Expansion::UnlimitedExpansion, sense)
    }

    /// [`Self::allocate_sense`] + 膨胀模式（见 [`Self::allocate_mode`]）。
    pub fn allocate_sense_mode(
        &mut self,
        id: &str,
        size: Vec2,
        mode: crate::widgets::Expansion,
        sense: crate::widgets::Sense,
    ) -> (Rect, crate::widgets::Response) {
        let rect = self.allocate_mode(size, mode);
        let abs = self.id_for(id);
        let resp = self.interact(&abs, rect, sense);
        (rect, resp)
    }

    /// **绝对定位申请 + 收交互**（不占光标）。
    pub fn allocate_sense_at(
        &mut self,
        pos: impl Into<Position>,
        id: &str,
        size: Vec2,
        sense: crate::widgets::Sense,
    ) -> (Rect, crate::widgets::Response) {
        let rect = self.allocate_at(pos, size);
        let abs = self.id_for(id);
        let resp = self.interact(&abs, rect, sense);
        (rect, resp)
    }

    /// 申请的核心（已换算物理像素）：按模式调整 → 消费一次性放置覆盖 → 占光标。
    fn allocate_rect(&mut self, mut size: Vec2, mode: crate::widgets::Expansion) -> Rect {
        use crate::widgets::Expansion;
        // `LimitedInParent`：宽度压到父级可用宽（`avail_w`）；无可用宽 = 自然尺寸。
        if mode == Expansion::LimitedInParent
            && let Some(avail) = self.avail_w()
            && avail < size.x
        {
            size.x = avail;
        }
        // `add_at` 的一次性覆盖：不占光标，直接落在指定坐标。
        if let Some(pos) = self.place_once.take() {
            let rect = Rect::new(pos.x, pos.y, size.x, size.y);
            self.note_placed(rect);
            return rect;
        }
        let child = if mode == Expansion::DisableAutoExpansion {
            Child::Fit
        } else {
            Child::Expand
        };
        self.child_rect(size.x, size.y, child)
    }

    /// **本矩形是否被裁剪层完全剔除**（分配 → 判一次 → 直接 return 的判据）。
    ///
    /// 语义 = "在当前**强制裁剪层**（Clip 沙箱 / ScrollView 可视区 / 文本框盒；不含
    /// 窗口结算后才知道的严格裁剪）里，这个矩形**一个像素都看不见**"。
    /// `rect` 传**当前容器局部**矩形（就是 `allocate*` 给的那个）。
    ///
    /// ⚠ 与 batch scissor 分工：scissor 裁**像素**（部分重叠时保住圆角/投影原形），
    /// 本判据裁**整条命令**（省镶嵌 + 顶点 + 段）。两者互补，都要有。
    #[inline]
    pub fn culled(&self, rect: Rect) -> bool {
        // 命令是先按容器局部录制、弹出时统一平移成绝对坐标；裁剪层是**绝对**坐标 ⇒
        // 比较前先把矩形抬到绝对空间（`abs_base` 就是当前容器的绝对原点）。
        let abs = Rect::new(
            rect.x + self.abs_base.x,
            rect.y + self.abs_base.y,
            rect.w,
            rect.h,
        );
        fully_outside(abs, self.painter.q.clip)
    }

    /// **收交互**（控件作者用；[`Self::allocate_sense`] 的底层）：按 [`crate::widgets::Sense`]
    /// 把"命中 → 焦点 → 按下认领 → 跨帧状态机"一次做完，返回本帧 [`crate::widgets::Response`]。
    ///
    /// 与既有内置控件逐条对齐（`button_at_styled` / `checkbox_at_styled` 等就是这套组合）：
    /// 1. `hit = hit_abs(id, rect)`（含窗口遮挡 / 控件级遮挡 / 强制裁剪层）；
    /// 2. `sense.focus` ⇒ `register_focus(id, rect, kind)`（Tab 可到 + 焦点描边）；
    /// 3. `sense.focus` ⇒ `key_click(id, kind)`（Enter/Space 合成点击）；
    /// 4. `sense.drag && 按下边沿 && hit` ⇒ `claim_press()`（外层窗口/面板不再把这次按下
    ///    当作拖动基准）；
    /// 5. `update_interact`（hover/pressed/clicked/released 跨帧状态）+ 键盘点击合成；
    /// 6. `pressed` ⇒ `note_press_handled()`（帧末"点空白清焦点"要区分按下与空白）；
    /// 7. `sense.drag` ⇒ `update_drag`（拖拽态；基准用 `WidgetState::{press_mouse,press_panel}`）。
    ///
    /// ⚠ 有拖拽语义的控件**仍要自己**维护拖拽基准（按下时写 `press_mouse` / `press_panel`）
    /// 与数值映射——本方法只负责状态机与认领。
    pub fn interact(
        &mut self,
        id: &IdAbsolute<'_>,
        rect: Rect,
        sense: crate::widgets::Sense,
    ) -> crate::widgets::Response {
        use crate::widgets::Response;
        let hit = sense.needs_hit() && self.hit_abs(id, &rect);
        if let Some(kind) = sense.focus {
            self.register_focus(id, rect, kind);
        }
        let btn = self.mouse_left();
        let key_click = sense.focus.is_some_and(|kind| self.key_click(id, kind));
        // 拖拽语义：按下边沿 + 命中 ⇒ 认领本次按压（阻止窗口/面板的拖动基准）。
        if sense.drag && hit && btn.down_edge() {
            self.claim_press();
        }
        let mut ev = {
            let ws = self.state_mut().widget(id);
            let ev = update_interact(ws, hit, btn);
            if key_click {
                // 键盘激活：合成"按下"，与鼠标路径同形（见各内置控件的 key_click 处理）。
                ws.pressed = true;
            }
            ev
        };
        if key_click {
            ev.clicked = true;
        }
        if ev.pressed || key_click {
            self.note_press_handled();
        }
        if sense.drag {
            let ws = self.state_mut().widget(id);
            update_drag(ws, hit, btn);
        }
        // `pressed` = **持续按住**（与 `ButtonState::pressed` 同义），不是"本帧按下边沿"：
        // 三态配色（常态/悬停/按下）靠它，读边沿会让按住期间退回悬停色。
        let held = self.state().widgets.get(id.as_str()).is_some_and(|ws| ws.pressed);
        Response {
            rect,
            // 分配处剔除：完全看不见的控件直接把信号交给控件（它应立刻 return）。
            culled: self.culled(rect),
            hovered: hit,
            pressed: held,
            clicked: ev.clicked,
            released: ev.released,
            toggled: false,
        }
    }

    /// 通用容器：push 帧 → 闭包 → 结算（返回尺寸与最大子尺寸）→ 平移子命令 → pop。
    fn container<F>(&mut self, pos: Vec2, frame: Frame, f: F) -> (Vec2, Vec2)
    where
        F: FnOnce(&mut ContainerCtx<'_, '_>),
    {
        let start = self.painter.q.queue.len();
        self.begin_top_placement();
        let saved_base = self.abs_base;
        self.abs_base = saved_base + pos;
        // **可用宽下传**（嵌套容器不许把内容排到固定宽父级外面）：
        // 子容器自己没有固定宽时，继承父级的可用宽（扣除子容器自己的内边距）。
        // 见 `Frame::max_w` 与 [`Self::avail_w`]。
        let mut frame = frame;
        if !frame.has_fixed_w() {
            let parent_avail = self.avail_w();
            frame.set_max_w(content_max_w(parent_avail, frame.pad_total));
        }
        self.frames.push(frame);
        self.painter.q.depth += 1;
        f(&mut ContainerCtx { ui: self });
        let frame = self.frames.pop().expect("container frame");
        let size = frame.settle_size();
        let max_child = frame.max_child;
        let inner_bounds = frame.content_bounds();
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        for d in &mut self.painter.q.queue[start..] {
            d.translate(pos);
        }
        // 绝对放置的容器整体也要算进**父级**尺寸（否则父容器/窗口仍会"只有标题那么高"）；
        // 连同容器**内部**的内容包围盒一起平移上报（自然尺寸可能低估子控件范围）。
        if let Some(parent) = self.frames.last_mut() {
            parent.note_content(Rect::new(pos.x, pos.y, size.x, size.y));
            if let Some(ib) = inner_bounds {
                parent.note_content(Rect::new(ib.x + pos.x, ib.y + pos.y, ib.w, ib.h));
            }
        }
        (size, max_child)
    }

    /// **长宽已确定的装饰容器**（内部原语）：在 `rect`（**当前容器局部**坐标、尺寸固定）里跑一个
    /// `Pack`，命令留在当前容器的局部空间（随后由调用方统一平移），**不参与父级尺寸结算**。
    ///
    /// 与 [`Self::container`] 的区别只有一条，但很关键：**不 `note_content` 到父 frame**
    /// —— 装饰（标题栏 / 状态条）是在**尺寸已经结算之后**才跑的，它的内容**不得**反过来
    /// 影响任何容器尺寸（否则"结算后再画"就自相矛盾）。
    ///
    /// 用途：窗口标题栏（[`Self::window_impl`] 在 `settle_size` 之后、**按下裁决之前**
    /// 用最终外框宽跑它 ⇒ 标题可居中、按钮可贴外缘）。用户侧想要同一形状可以用公开的
    /// [`Self::view_at`]（`ViewMode::Expand` 的沙箱：提供 `avail_w`、不裁剪、内容可溢出）。
    fn ornament_at(&mut self, rect: Rect, f: impl FnOnce(&mut Pack<'_, '_>)) {
        let start = self.painter.q.queue.len();
        let saved_base = self.abs_base;
        self.abs_base = saved_base + Vec2::new(rect.x, rect.y);
        let mut frame = Frame::new_stack(PackSide::Top, self.theme.gap, 0.0);
        // 固定长宽：子项因此拿到确定的 `avail_w`（省略号 / `LimitedInParent` 用得上）。
        frame.set_fixed_w(rect.w);
        frame.set_fixed_h(rect.h);
        self.frames.push(frame);
        // 深度与"同一窗口里的内容"持平 ⇒ `elem` 语义与旧实现（标题/按钮录在窗口 frame 里）
        // 完全一致：晚入队 ⇒ 排在面板底色（`elem = 0`，在窗口外一层画）之上、内容同级之上。
        self.painter.q.depth += 1;
        let mut p = Pack { ui: self };
        f(&mut p);
        // 丢弃它的结算尺寸：装饰不参与父级尺寸（`Frame::settle_size` 只有调用者关心）。
        let _ = self.frames.pop().expect("ornament frame");
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        if rect.x != 0.0 || rect.y != 0.0 {
            for d in &mut self.painter.q.queue[start..] {
                d.translate(Vec2::new(rect.x, rect.y));
            }
        }
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
    /// 文本编辑框）与严格窗口（[`Self::window`] + `Placement::Clip`）的公共底座。
    pub fn view_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        mode: ViewMode,
        f: impl FnOnce(&mut ViewCtx<'_, '_>),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        let size = size.into().to_physical(self.scale);
        let saved_clip = self.painter.q.clip;
        let saved_base = self.abs_base;
        let view_rel = Rect::new(pos.x, pos.y, size.x.max(0.0), size.y.max(0.0));
        let view_abs = Rect::new(
            saved_base.x + view_rel.x,
            saved_base.y + view_rel.y,
            view_rel.w,
            view_rel.h,
        );
        // 强制裁剪层（Clip 模式：外层 ∩ 可视区；Expand：原样传递）。
        self.painter.q.clip = clip_for_view(saved_clip, view_abs, mode);
        // 可用宽度栈：沙箱内 avail_w() = 沙箱宽。
        self.avail_stack.push(Some(view_rel.w));
        let start = self.painter.q.queue.len();
        self.begin_top_placement();
        self.abs_base = saved_base + pos;
        self.frames.push(Frame::new_stack(PackSide::Top, self.theme.gap, 0.0));
        self.painter.q.depth += 1;
        f(&mut ViewCtx { ui: self });
        let frame = self.frames.pop().expect("view frame");
        let content = frame.settle_size();
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        self.avail_stack.pop();
        self.painter.q.clip = saved_clip;
        for d in &mut self.painter.q.queue[start..] {
            d.translate(pos);
        }
        content
    }

    /// 当前可用的**内容宽度**（物理像素）：沙箱宽 → **由内向外**第一个给出宽度约束的
    /// 容器 frame（固定宽窗口 / 从父级继承的内容最大宽）→ 下一子项 max 约束，取最小；
    /// 无任何约束 = `None`（内容自然宽度）。供 `LimitedInParent` 控件（如
    /// [`crate::widgets::Label`]）自洽（自动换行 / 省略号）。
    ///
    /// ⚠ **必须由内向外扫整个 frame 栈**（不是只看 `frames.last()`）：`row` 是独立
    /// frame、自己不设固定宽，只看最内层会让 `row` 里的 `Label` 拿不到外层窗口的可用宽
    /// ⇒ 按自然宽把整行排到**固定宽窗口外面**（用户实测："指定 width 里控件会突出去，
    /// 直到你去拖拽缩放"）。
    #[inline]
    pub fn avail_w(&self) -> Option<f32> {
        let base = self
            .avail_stack
            .last()
            .copied()
            .flatten()
            .or_else(|| crate::layout::stack_avail_w(&self.frames));
        // **下一子项**的两条约束：显式 `max_size`，以及水平堆叠的**剩余宽**
        // （一行里后面的控件拿到的可用宽必须扣掉前面已经用掉的）。
        let nm = self.frames.last().map(|f| f.next_max_w()).unwrap_or(0.0);
        let rem = self.frames.last().and_then(|f| f.remaining_w());
        let mut out = base;
        if nm > 0.0 {
            out = Some(out.map_or(nm, |b| b.min(nm)));
        }
        if let Some(r) = rem {
            out = Some(out.map_or(r, |b| b.min(r)));
        }
        out
    }

    /// **分割线**（绝对定位水平线）：`pos` 相对当前容器内容原点，宽 `w`（逻辑像素）。
    /// 线画在 `pos.y + margin`（上下留白由调用方行高体现）。样式取
    /// [`Theme::divider`](crate::style::Theme::divider)。
    pub fn divider_at(&mut self, pos: impl Into<Position>, w: impl Into<Size<f32>>) {
        let pos = pos.into().to_physical(self.scale);
        let w = w.into().to_physical(self.scale);
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
        pos: impl Into<Position>,
        view_size: impl Into<Size<Vec2>>,
        id: &str,
        f: impl FnOnce(&mut Scroll<'_, '_>),
    ) -> Vec2 {
        // 公开形式 = 两轴都 `Scroll`（内容超出即滚动；与加按轴 API 之前逐行为一致）。
        self.scroll_at_axes(pos, view_size, id, ScrollMode::Scroll, ScrollMode::Scroll, |ui| {
            f(&mut Scroll { ui })
        })
    }

    /// [`Self::scroll_at`] 的**核心**（按轴版本）：把 `f` 录进一个滚动视口，返回可视区尺寸。
    ///
    /// `v` / `h` 决定**哪条轴**是滚动轴：
    /// - `NoClip`：那条轴**不裁**（内容溢出可见，也不参与滚动偏移）；
    /// - `ClipOnly` / `Scroll`：那条轴是视口（裁切；`Scroll` 时还画滚动条、吃滚轮）。
    ///
    /// 抽出来是为了让**窗口**（`.vscroll(Scroll)`）与 `scroll_at` 共用同一实现：窗口把用户
    /// 闭包包进来即可（`f` 收 `&mut Ui`，两边各自包成 `Scroll` / `Window`）。
    pub(crate) fn scroll_at_axes(
        &mut self,
        pos: impl Into<Position>,
        view_size: impl Into<Size<Vec2>>,
        id: &str,
        v: ScrollMode,
        h: ScrollMode,
        f: impl FnOnce(&mut Ui),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        let view_size = view_size.into().to_physical(self.scale);
        // 滚动容器自身也是命名空间边界：内部子控件 ID 自动带 `id` 前缀。
        let abs = self.id_for(id);
        let saved_clip = self.painter.q.clip;
        let saved_base = self.abs_base;
        let (sw, sh) = (
            self.window.inner_size().width as f32,
            self.window.inner_size().height as f32,
        );
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
        // 强制裁剪层 = 外层裁剪 ∩ 本可视区 —— **按轴**（`NoClip` 那条轴不动）。
        self.painter.q.clip =
            clip_for_axes(saved_clip, view_abs, Rect::new(0.0, 0.0, sw, sh), v, h);
        // 滚动偏移（**物理像素**，跨帧状态；先 Copy 读出，`f` 结束再写回——避免
        // 借用冲突）。以整物理像素步进（滚轮 / 拖 thumb 均取整）。
        let mut offset_px = self
            .state
            .scrolls
            .get(abs.as_str())
            .map(|s| s.offset)
            .unwrap_or(0.0);
        // 可用宽度栈：滚动容器内 avail_w() = 可视区宽（LimitedInParent 控件自洽）。
        self.avail_stack.push(Some(view_rel.w));
        // 内容 pack 堆叠（手动管理帧栈：平移 = pos - offset_px/scale，而非 container 的 pos）。
        let start = self.painter.q.queue.len();
        self.begin_top_placement();
        // abs_base = 内容**渲染**原点（已含 -offset 滚动偏移）——`hit_abs`（点击
        // 命中）/ `register_focus`（焦点描边）/ IME 光标定位都经 abs_base 换算，
        // 必须与平移后的绘制位置一致，否则点击位置跟不上滚动视图（offset ≠ 0 时
        // 命中落在未滚动坐标上）。
        self.abs_base = saved_base + pos - Vec2::new(0.0, offset_px);
        self.frames.push(Frame::new_stack(PackSide::Top, self.theme.gap, 0.0));
        self.painter.q.depth += 1;
        // ID 命名空间：滚动容器进入压栈、退出弹栈（闭包作用域保证配对）。
        self.with_id(id, |ui| f(ui));
        let frame = self.frames.pop().expect("scroll frame");
        let content_size = frame.settle_size();
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        self.avail_stack.pop();
        let max_off_px = (content_size.y - view_size.y).max(0.0).round();
        offset_px = offset_px.clamp(0.0, max_off_px);
        // 滚轮（鼠标在可视区内且未被窗口遮挡；wheel y 向上为正 → offset 减小）。
        // 每格 40 物理像素取整（trackpad 连续增量同样按格取整步进）。
        let hit = hit_test(&view_abs, self.mouse_logical)
            && self.mouse_in_window
            && !window_occluded(self.painter.q.cur_win, self.mouse_logical, self.window_rects_iter());
        if hit {
            let (_, wy) = self.mouse.wheel();
            if wy != 0.0 {
                offset_px = (offset_px - (wy as f32 * 40.0).round()).clamp(0.0, max_off_px);
            }
        }
        // 平移内容子命令：局部坐标 → 绝对（`UiDraw::clip` 已是绝对，不随平移——见其
        // 文档）。offset 为物理像素 → 刚性平移。
        for d in &mut self.painter.q.queue[start..] {
            d.translate(pos - Vec2::new(0.0, offset_px));
        }
        // 滚动条（内容超出可视区时显示；拖 thumb / 点轨道翻页）——在**当前容器局部
        // 坐标**绘制，**不参与**上面的内容平移；随外层容器弹出统一平移成绝对坐标。
        if content_size.y > view_size.y + 1.0 && view_size.y > 0.0 {
            // `elem` = **本放置的下一个元素序**（`elem_hint()`）：滚动条在**内容之后**录制，
            // 于是它排在本容器子内容**之上**——与 `scrollbar` 的文档 / 它注册的控件级遮挡
            // （"点滚动条不该连带触发被压住的列表项"）一致。
            //
            // ⚠ 这里**曾经传 `0`**（当成"容器装饰"）。但 `elem = 0` 的语义是"画在本容器
            // 元素**之下**"，于是列表项（`elem ≥ 1`）把滑块整块盖住 —— 用户报的
            // "ScrollBar 闪烁"（滑块只在条目间隙里露一条，滚动时忽隐忽现）。
            // 容器**底色 / 投影**用 `elem = 0` 是对的（它们确实该在最底），滚动条不是。
            offset_px = self.scrollbar(
                &abs,
                &view_rel,
                view_size.y,
                content_size.y,
                offset_px,
                max_off_px,
                saved_clip,
                self.painter.q.seq + 1,
            );
        }
        // 写回滚动状态（`f` 借用已结束；offset 为物理像素）。
        let st = self.state.scrolls.entry(abs.to_static()).or_default();
        st.offset = offset_px;
        st.content_h = content_size.y;
        st.content_w = content_size.x;
        self.painter.q.clip = saved_clip;
        view_size
    }

    /// **选择列表**：`scroll_at` + 逐项回调（选中态由调用方维护）。
    ///
    /// `item` 回调 `(容器, 索引, 是否选中) -> bool`：返回 `true` 表示该项被点击。
    /// 返回本帧被点击的索引（`None` = 无）。
    pub fn list_at<F>(
        &mut self,
        pos: impl Into<Position>,
        view_size: impl Into<Size<Vec2>>,
        id: &str,
        count: usize,
        selected: Option<u32>,
        mut item: F,
    ) -> Option<u32>
    where
        F: FnMut(&mut Scroll<'_, '_>, u32, bool) -> bool,
    {
        let pos = pos.into().to_physical(self.scale);
        let view_size = view_size.into().to_physical(self.scale);
        let mut clicked = None;
        // 内部已是物理：显式 `Physical`（避免默认 Logical 二次换算）。
        self.scroll_at(Position::Physical(pos), Size::Physical(view_size), id, |s| {
            for i in 0..count as u32 {
                if item(s, i, selected == Some(i)) && clicked.is_none() {
                    clicked = Some(i);
                }
            }
        });
        clicked
    }

    /// 滚动条：右侧竖条（轨道 + 胶囊滑块）。返回更新后的滚动偏移（**物理像素**）。
    ///
    /// 观感（本轮起）：**常驻**、比旧版更粗的**胶囊**滑块，居中于 [`SCROLLBAR_W`]
    /// 条带内 ⇒ **两侧留白**；配色取调色板的弱色（`text_dim`，悬停 / 拖拽转
    /// `text_muted`）而不再用近白的 `slider.handle`——深 / 浅两色都不刺眼。
    /// 条带（含留白）即命中 / 翻页热区，比可见滑块宽 ⇒ 抓取更容易。
    ///
    /// `view` 为**当前容器局部坐标**的可视区（与内容同空间，**不随内容滚动**；
    /// 由外层容器弹出统一平移成绝对坐标）；命中用局部坐标鼠标（`mouse_logical −
    /// abs_base`），遮挡判定仍用绝对鼠标。滑块几何在物理像素里取整（
    /// [`scroll_thumb`]），拖拽按 **整物理像素 1:1** 步进——内容与滑块刚性移动
    /// （非整数 DPI 不抖）。
    /// `elem`：所属元素序（`scroll_at` 传 `elem_hint()` ⇒ 覆盖在本容器内容之上；
    /// 文本编辑框传自己的 `elem`，同样在文本之上——**不要传 `0`**：那是"画在内容之下"，
    /// 会被自家内容盖住，见 `scroll_at` 里的说明）。
    #[allow(clippy::too_many_arguments)]
    fn scrollbar(
        &mut self,
        id: &IdAbsolute<'_>,
        view: &Rect,
        view_h: f32,
        content_h: f32,
        offset_px: f32,
        max_off_px: f32,
        outer_clip: Option<Rect>,
        elem: u32,
    ) -> f32 {
        let mut offset_px = offset_px;
        // 条带（占位 + 命中 / 翻页热区）与**可见**轨道（居中、上下留白 ⇒ 胶囊不贴边）。
        let (strip, track) = scrollbar_rects(view, view_h);
        let track_h = track.h;
        // 滑块几何：**物理像素**计算（整像素步进 → 刚性；纯函数可单测）。
        let (thumb_h_px, travel_px, thumb_y_px) =
            scroll_thumb(track_h, view_h, content_h, offset_px, max_off_px);
        // 滑块顶 = 轨道顶（局部坐标）+ 轨道内偏移（物理像素）
        let thumb = Rect::new(track.x, track.y + thumb_y_px, track.w, thumb_h_px);
        // 交互判定必须在**绘制前**求出（滑块颜色取决于悬停 / 拖拽状态）。
        // 局部坐标鼠标 = 绝对鼠标 − 当前容器绝对原点（abs_base 已恢复为外层值）。
        let depth = self.painter.q.depth;
        let win = self.painter.q.cur_win;
        let mouse_rel = self.mouse_logical - self.abs_base;
        let bar_id = IdAbsolute::owned(format!("{}::bar", id.as_str()));
        let on_top = self.mouse_in_window
            && !window_occluded(win, self.mouse_logical, self.window_rects_iter());
        // **控件级遮挡**（与 `hit_abs` 同一套）：滚动条画在内容**之上**，
        // - 它自己参与遮挡链（鼠标在条带内时，条带下方的控件不得响应——否则
        //   "点滚动条"会连带触发被压住的列表项 / 文本插入符）；
        // - 同时也要能被**更晚录制**的控件挡住（对称处理，不搞特例）。
        // 登记用**条带**（滑块 + 两侧留白 + 上下留白）：热区即占位区。
        let me = id_hash(&bar_id);
        let key = self.painter.q.seq;
        let strip_abs = Rect::new(
            self.abs_base.x + strip.x,
            self.abs_base.y + strip.y,
            strip.w,
            strip.h,
        );
        if on_top && hit_test(&strip_abs, self.mouse_logical) {
            self.state.hit_regions.push(HitRegion {
                owner: me,
                key,
                rect: strip_abs,
                clip: self.painter.q.clip,
            });
        }
        let on_top = on_top
            && !widget_occluded(
                me,
                key,
                self.mouse_logical,
                self.state.prev_hit_regions.iter().copied(),
            );
        let bar_hit = on_top && hit_test(&thumb, mouse_rel);
        let strip_hit = on_top && hit_test(&strip, mouse_rel);
        let btn = self.mouse_left();
        // 滚动条自身有拖拽语义：按下（滑块 / 条带）置位 press_claimed，
        // 阻止外层窗口把本次按下当作窗口拖拽基准（窗口内拖滚动条不连窗口一起动）。
        if btn.down_edge() && strip_hit {
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
        // 绘制：轨道 + 滑块（白纹理图形，`elem` 所属元素）。胶囊 = 半径取半宽。
        //
        // ⚠ **每条命令各取一次 `next_seq()`**：这里是少数"直接写队列"的入口之一，
        // 队列播放头 `DrawQueue::seq` 必须恒等于已分配的最大序号——否则下一条命令会
        // 拿到**重复序号**，而"放置序"（`z0_place_for_seq`）是按 `seq` 归一化的
        // ⇒ 重复序号跨越放置边界时会把同一容器的命令拆进两个排序空间（实测：滚动条的
        // 轨道与滑块被拆成 place=1 / place=2，滑块被列表项盖住 = 又变成闪烁）。
        // （旧写法 `let seq = next_seq(); push(seq); push(seq + 1)` 就是这样漏掉一次的。）
        let track_seq = self.next_seq();
        let thumb_seq = self.next_seq();
        let radius = CornerRadius::all(SCROLLBAR_BAR_W * 0.5);
        let pal = self.theme.palette;
        let thumb_col = if bar_hit || grab.0 {
            pal.text_muted
        } else {
            pal.text_dim
        };
        self.painter.q.queue.push(UiDraw {
            depth,
            seq: track_seq,
            win,
            elem,
            rect: track,
            clip: outer_clip,
            // 滚动条轨道用主题滑块轨道刷（可能是渐变：纯色 → **胶囊**圆角矩形；
            // 渐变无法圆角 → 退化成四角顶点色的 `Rect`。两者都是一条命令、无纹理）。
            kind: match self.theme.slider.track.as_solid() {
                Some(c) => DrawKind::RoundedRect {
                    corners: [c; 4],
                    radius,
                },
                None => DrawKind::Rect(Gradient::corners(
                    self.theme.slider.track.corners()[0],
                    self.theme.slider.track.corners()[1],
                    self.theme.slider.track.corners()[2],
                    self.theme.slider.track.corners()[3],
                )),
            },
        });
        self.painter.q.queue.push(UiDraw {
            depth,
            seq: thumb_seq,
            win,
            elem,
            rect: thumb,
            clip: outer_clip,
            kind: DrawKind::RoundedRect {
                corners: [thumb_col; 4],
                radius,
            },
        });
        if grab.0 {
            let pm = self
                .state
                .widgets
                .get(bar_id.as_str())
                .and_then(|w| w.press_mouse)
                .unwrap_or(self.mouse_screen);
            // 滑块**跟随鼠标 1:1**（保持按下时的抓取点偏移），滚动偏移由滑块
            // 位置反推——否则滑块按比例慢于鼠标（内容越高越明显，"不同步"）。
            let dy_px = (self.mouse_screen.y - pm.y).round();
            let thumb_y_px_new = grab.1.x + dy_px; // grab.1.x = 按下时 thumb_y_px
            offset_px = scroll_offset_for_thumb(thumb_y_px_new, travel_px, max_off_px);
        }
        // 光标：视口滑条（滑块 / 条带）保持普通 Arrow（UI_NEEDS：滑条不用 <->）。
        // 条带点击（滑块外）→ 翻页（整物理像素步长）。
        let page_px = view_h.round();
        if btn.down_edge() && strip_hit && !bar_hit {
            if mouse_rel.y < thumb.y {
                offset_px = (offset_px - page_px).max(0.0);
            } else if mouse_rel.y > thumb.y + thumb.h {
                offset_px = (offset_px + page_px).min(max_off_px);
            }
        }
        offset_px
    }

    // ── 顶层入口（*_at：位置显式，尺寸自动） ─────────────────

    /// 绝对定位标签（`pos` 相对当前容器内容原点；顶层即屏幕原点）。
    pub fn label_at(&mut self, pos: impl Into<Position>, text: &str) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        let elem = self.painter.q.seq + 1;
        let seq = self.next_seq();
        let style = self.theme.label.clone();
        let size = self.text_size(text, style.font_size, style.font_family.as_deref());
        let rect = Rect::new(pos.x, pos.y, size.x, size.y);
        self.note_placed(rect);
        self.painter.q.queue.push(text_cmd(
            self.painter.q.depth,
            seq,
            self.painter.q.cur_win,
            elem,
            rect,
            Arc::from(text),
            style.font_size,
            style.color,
            TextAlign::from(style.align),
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.painter.q.clip,
        None,
        ));
        size
    }

    /// **自动换行标签**：`max_w`（逻辑像素）内按词/字换行，返回自然尺寸
    /// （宽 = min(自然宽, max_w)，高 = 行数 × 行高）。`max_w <= 0` = 不换行（同 [`Self::label_at`]）。
    ///
    /// 换行宽度参与排版缓存键（不同宽度各自缓存）；多行文本垂直居中于矩形。
    pub fn label_wrap_at(&mut self, pos: impl Into<Position>, max_w: impl Into<Size<f32>>, text: &str) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        let max_w = max_w.into().to_physical(self.scale);
        let elem = self.painter.q.seq + 1;
        let seq = self.next_seq();
        let style = self.theme.label.clone();
        let size = self.text_size_wrap(text, style.font_size, style.font_family.as_deref(), max_w);
        let rect = Rect::new(pos.x, pos.y, size.x, size.y);
        self.note_placed(rect);
        // 换行标签：直接传预排版缓冲（渲染与测量同一缓冲）——否则绘制期按不换行
        // 排版，长文本会单行溢出而非自动换行。
        let buf = if max_w > 0.0 {
            Some(self.wrap_buffer(text, style.font_size, style.font_family.as_deref(), max_w))
        } else {
            None
        };
        self.painter.q.queue.push(text_cmd(
            self.painter.q.depth,
            seq,
            self.painter.q.cur_win,
            elem,
            rect,
            Arc::from(text),
            style.font_size,
            style.color,
            TextAlign::from(style.align),
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.painter.q.clip,
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
    /// # let text = todo!(); let mut backend = rjw_ui::RecordingBackend::default(); let state = todo!(); let window = todo!();
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
    /// ui.window("hud").pos(glam::Vec2::new(400.0, 40.0)).show(|w| { w.label("HUD"); });
    /// ui.finish(&mut backend);
    /// ```
    pub fn pos_handler(&mut self, priority: i32, f: impl Fn(&str) -> Option<Vec2> + 'static) {
        self.pos_chain.push((priority, PosLink::Script(Box::new(f))));
        // 优先级降序（稳定排序：同优先级保持注册顺序）
        self.pos_chain.sort_by_key(|e| std::cmp::Reverse(e.0));
    }

    /// 责任链解析窗口/面板位置（见 [`Self::pos_handler`]）。
    #[inline]
    fn resolve_pos(&self, id: &IdAbsolute<'_>, pos: Vec2) -> Vec2 {
        resolve_pos_link(&self.pos_chain, &self.state.panel_pos, id, pos)
    }

    /// **自动窗口位置**（Win32 `CW_USEDEFAULT` 语义）：没写 `.pos()` 的窗口由引擎
    /// 按**首次出现顺序**级联分配（见 [`auto_pos_slot`]），并记进
    /// [`UiState::auto_pos`] 跨帧稳定。
    ///
    /// 优先级：`pos_handler` 脚本 > 用户拖拽（[`UiState::panel_pos`]）> **本自动位置**
    /// ——自动位置只是"初值"，拖过之后停在用户放置处（`reset()` 会连同记录一起清空）。
    fn auto_window_pos(&mut self, id: &IdAbsolute<'_>, viewport: Vec2, scale: f32) -> Vec2 {
        auto_pos_take(
            &mut self.state.auto_pos,
            &mut self.state.auto_next,
            id,
            viewport,
            scale,
        )
    }

    /// **尺寸责任链**：注册可调尺寸控件（[`Self::resizable_text_area_at`] /
    /// [`Self::resizable_text_input_at`]）的**尺寸**处理器（如脚本/动画/外部布局约束）。
    ///
    /// `priority` 高的优先；返回 `Some(size)` 即生效，`None` 落到低优先级 /
    /// 用户拖拽缩放（[`UiState::sizes`]）/ 传入 rect 尺寸兜底。语义同 [`Self::pos_handler`]：
    /// `'static` 闭包，不借用 `self`（共享可变状态用 `Arc<Mutex<_>>`）。
    pub fn size_handler(&mut self, priority: i32, f: impl Fn(&str) -> Option<Vec2> + 'static) {
        self.size_chain.push((priority, SizeLink::Script(Box::new(f))));
        self.size_chain.sort_by_key(|e| std::cmp::Reverse(e.0));
    }

    /// 责任链解析可调尺寸控件尺寸（见 [`Self::size_handler`]）。
    #[inline]
    fn resolve_size(&self, id: &IdAbsolute<'_>, fallback: Vec2) -> Vec2 {
        resolve_size_link(&self.size_chain, &self.state.sizes, id, fallback)
    }

    /// **解析可调尺寸控件的**当前**尺寸**（尺寸责任链 → 用户拖拽持久值
    /// [`UiState::sizes`] → `fallback`）——**控件作者公开面**。
    ///
    /// 与 [`Self::resize_handle`] 配对使用：`resize_handle` 负责**改**尺寸（拖拽 +
    /// 写持久值），本方法负责**读**它。自定义"可拖拽缩放"控件必须在 `ui()` 里
    /// **申请之前**调用它，把上一帧的持久尺寸并进本帧的申请尺寸——否则会出现
    /// "画的是拖大的框、申请的却是默认尺寸"：容器不跟着长、后面的控件不动，
    /// 而框本身溢出父级（`TextEditor` 就踩过这条）。
    ///
    /// ```no_run
    /// # use glam::Vec2;
    /// # use rjw_ui::{Ui, Resize};
    /// # fn demo(ui: &mut Ui, id: &str, value: &mut String) {
    /// // ① 想好默认尺寸（物理像素）
    /// let want = Vec2::new(240.0, 60.0);
    /// // ② 责任链 / 用户拖拽值优先
    /// let size = ui.resolved_size(id, want);
    /// // ③ 用解析后的尺寸申请（容器尺寸随它走）
    /// let rect = ui.allocate(size);
    /// // ④ 交给 `resize_handle` 拖拽（`current` 传解析后的尺寸）
    /// # let _ = (rect, Resize::Both);
    /// # }
    /// ```
    pub fn resolved_size(&mut self, id: &str, fallback: Vec2) -> Vec2 {
        let abs = self.id_for(id);
        self.resolve_size(&abs, fallback)
    }

    /// 面板：背景 + 边框 + 内容垂直堆叠（pack Top）；尺寸自动包裹内容。
    pub fn panel_at(
        &mut self,
        pos: impl Into<Position>,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        self.panel_impl(pos, None, None, f)
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
        pos: impl Into<Position>,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        self.panel_impl(pos, Some(id), None, f)
    }

    /// 面板公共实现：`drag = Some(id)` 时启用拖拽；`style` 逐面板覆盖（`None` = 全局
    /// [`Theme::panel`]）。
    fn panel_impl(
        &mut self,
        pos: Vec2,
        drag: Option<&str>,
        style: Option<&PanelStyle>,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        // 拖拽面板的位置从**责任链**读取（脚本处理器 → 用户拖拽状态 → 传入 pos，
        // 见 pos_handler）：首次 / 从未拖过时用传入 pos
        // 面板自身也是命名空间边界（可拖拽面板有稳定 id）——先解析绝对 id 供状态键用。
        let abs = drag.map(|id| self.id_for(id));
        let origin = match abs.as_ref() {
            Some(a) => self.resolve_pos(a, pos),
            None => pos,
        };
        let start = self.painter.q.queue.len();
        self.begin_top_placement();
        let style = style
            .cloned()
            .unwrap_or_else(|| self.theme.panel.clone());
        let (pad_total, gap) = (style.padding + style.border_w, self.theme.gap);
        let saved_base = self.abs_base;
        // ─── ① 位置与交互**先于内容录制**求解（同 `window_impl`）──────────
        // 鼠标事件是针对**屏幕上已有的几何**（上一帧结算的 `panel_sizes`）产生的，
        // 故命中 / 拖拽基准用上一帧矩形；由此 `display_pos` 在录制前已知，
        // `abs_base` 与随后 `translate(display_pos)` 的几何**当帧一致**。
        // 旧实现 `abs_base` 用上一帧位置、几何用本帧位置：拖动面板时面板内文本框
        // 的 `box_clip` / 光标 / 滑块基准落后一帧（快速拖动时文字被裁、点击错位）。
        let prev_size = abs
            .as_ref()
            .and_then(|a| self.state.panel_sizes.get(a.as_str()).copied());
        let panel_rect = prev_size.map(|ps| Rect::new(origin.x, origin.y, ps.x, ps.y));
        let btn = self.mouse_left();
        // 窗口遮挡：面板是 win=0 内容（绘制在所有窗口之下），被任意窗口覆盖时不可拖拽。
        // 首帧无 `prev_size` ⇒ 本帧不参与交互（面板尚未被看到）。
        let hit = panel_rect.is_some_and(|r| {
            hit_test(&r, self.mouse_logical)
                && self.mouse_in_window
                && !window_occluded(0, self.mouse_logical, self.window_rects_iter())
        });
        let press_here = btn.down_edge() && hit;
        // 拖拽交互：**物理像素粒度**拖动基准（见下方说明）。按下帧先无条件建立
        // 基准；面板内子控件随后声明本次按下（`press_claimed`）时在 ② 清除。
        let (active, display_pos) = match abs.as_ref() {
            Some(a) => {
                let ws = self.state.widgets.entry(a.to_static()).or_default();
                resolve_drag(ws, hit, btn, self.mouse_screen, origin)
            }
            None => (false, origin),
        };
        // 内容基准 = **本帧显示基准**（`display_pos`）——录制期的绝对空间量与几何一致。
        self.abs_base = saved_base + display_pos;
        self.frames.push(Frame::new_stack(PackSide::Top, gap, pad_total));
        self.painter.q.depth += 1;
        let mut panel = Panel { ui: self };
        f(&mut panel);
        let frame = self.frames.pop().expect("panel frame");
        let size = frame.settle_size();
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        // ─── ② 内容录完后：按下裁决 + 位置持久 ──────────────────────────
        if let Some(a) = abs.as_ref() {
            if press_here {
                let ws = self.state.widgets.entry(a.to_static()).or_default();
                if self.press_claimed {
                    // 文本框等子控件按下（选择拖拽优先）：清除基准（见 `window_impl`）。
                    clear_drag_base(ws);
                }
            }
            // 结算尺寸跨帧持久：下帧命中 / 拖拽基准 = 屏幕上那个矩形。
            self.state.panel_sizes.insert(a.to_static(), size);
            if active {
                self.drag_panel = Some(a.to_static());
                // 仅位置变化时写入（滞回：同一位置不重写）
                if self.state.panel_pos.get(a.as_str()) != Some(&display_pos) {
                    self.state.panel_pos.insert(a.to_static(), display_pos);
                }
            } else if self
                .drag_panel
                .as_ref()
                .is_some_and(|d| d.as_str() == a.as_str())
            {
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
        }
        // 背景 + 边框（depth = 进入前深度，画在子控件之下；radius > 0 走圆角双层矩形）
        let bg_rect = Rect::new(0.0, 0.0, size.x, size.y);
        self.push_panel_shadow(bg_rect, &style.shadow, style.radius);
        self.push_panel_like_img(bg_rect, style.bg, style.bg_image, style.border, style.border_w, style.radius, 0);
        // 平移全部（子命令 + 背景/边框）：
        // 用 `display_pos`（拖拽中 = 本帧新位置）→ 文字/矩形**当帧生效**。
        for d in &mut self.painter.q.queue[start..] {
            d.translate(display_pos);
        }
        size
    }

    /// 在 `id_relative` 命名空间内执行 `f`：进入压栈、退出弹栈。
    /// **闭包作用域保证配对**（借用检查器 + 栈帧语义，`?`/panic 也安全）——
    /// 消除手动 `push_id`/`pop_id` 的漏配对/多弹出风险。容器实现
    /// （window / scroll / grid）用。`id_relative` 为相对名字（容器的命名空间段）。
    pub(crate) fn with_id<'s>(
        &mut self,
        id_relative: impl Into<IdRelative<'s>>,
        f: impl FnOnce(&mut Ui<'_>),
    ) {
        self.ids.push(id_relative.into());
        f(self);
        self.ids.pop();
    }

    /// 根据当前命名空间栈与相对 id 生成**绝对 id**（状态键 / 焦点 id 用）。
    ///
    /// - 顶层（栈空）返回 `Borrowed`——**零拷贝零分配**；
    /// - 嵌套返回 `Owned`（一次拼接）。
    ///
    /// 类型安全：`id_relative` 只接受相对名字（[`IdRelative`] / `&str`），已解析的
    /// 绝对 id **无法**再传进来（双重前缀编译期报错）。
    ///
    /// 生命周期 `'l` 绑定**传入的名字**（而非 `&mut self` 的 `'s`）：调用后 `self` 借用
    /// 释放，返回值可继续用于后续 `&mut self` 操作（`register_focus` / 状态读写）。
    pub fn id_for<'s, 'l>(&'s mut self, id_relative: impl Into<IdRelative<'l>>) -> IdAbsolute<'l> {
        self.ids.id_for(id_relative.into())
    }

    /// **窗口**容器实现（**非公开**：公开入口是 [`Self::window`] 责任链 builder）。
    ///
    /// - **可重叠**：多个窗口按 **z-order** 排列（`UiState.window_z`），
    ///   点击窗口即**置顶**（焦点，`topmost = true`）；z 越大越靠上。
    /// - **遮挡隔离**（点击穿透修复）：重叠区域只让**鼠标下最上层**的窗口响应
    ///   （见 [`crate::hit::window_occluded`]）。
    /// - **可拖拽**：按住窗口任意处移动 ≥ 3 物理像素进入拖拽（位置持久于
    ///   `UiState.panel_pos`）；纯点击不拖拽，窗口内子控件正常响应。
    /// - 绘制顺序由 [`Ui::finish`] 保证：**背景/图形严格先于文字**。
    ///
    /// 参数：`width = Some(w)` 固定宽（高度自然、右下角可缩放、跨帧持久）；
    /// `topmost` 点击是否置顶（modal 用 `false`）；`strict` 内容强制裁剪到窗口矩形
    /// （`Placement`）；`style` 逐窗口覆盖（`None` = 全局 [`Theme::panel`]）；
    /// `clamp` 位置约束模式（见 [`WindowClamp`]）。
    fn window_impl(
        &mut self,
        id: &str,
        pos: Vec2,
        width: Option<f32>,
        // **固定高**（物理；`None` = 由内容决定）——`.height(..)` 的物理值。
        height: Option<f32>,
        // 内容子项间距（物理；`None` = `Theme::gap`）。下拉 / 菜单浮层传更紧的值。
        content_gap: Option<f32>,
        topmost: bool,
        strict: bool,
        style: Option<&PanelStyle>,
        clamp: WindowClamp,
        // 两条轴的溢出策略（显式设置；`None` = 按 `strict` / "该轴是否被拖过"解算）。
        scroll: (Option<ScrollMode>, Option<ScrollMode>),
        chrome: &mut WindowChrome<'_>,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        let id_for = self.id_for(id);
        // **本帧生效的尺寸** = 持久值（缩放柄拖出来的，跨帧保持）**优先**于传入的
        // `.width(..)` / `.height(..)`（那只是**初始值**）。统一在此读取——`window_at_w` /
        // `modal_at_w` / `WindowBuilder::{width,height}` 都不必各自处理。
        //
        // ⚠ **必须"持久优先"**，否则第二次拖柄会从**旧值**重新起算 ⇒ 那条轴弹回原位
        // （用户实测："拖拽缩放柄到别的地方，然后下一次点击 x 坐标弹回"——`eg260818UI`
        // 里只有**唯一没有 `.width()`** 的 `strict_win` 不弹，正是这条的反证）。
        let explicit_w = width;
        let persisted_w = self.state.window_widths.get(id_for.as_str()).copied();
        // 用于 clamp / 缩放柄 / 持久化判断的"有效宽"。
        let width = persisted_w.or(explicit_w);
        // **高度**：`.height(..)`（固定高）与持久高（拖过）同口径（持久优先）；
        // **收起态例外**（收起 = 只剩一行标题栏 ⇒ 忽略固定高 / 持久高）。这里**必须在
        // 缩放柄之前**算好：柄的基准 / 当前尺寸要用它。
        let explicit_h = height;
        let persisted_h = self.state.window_heights.get(id_for.as_str()).copied();
        let eff_h = persisted_h.or(explicit_h);
        let collapsed_now = chrome.collapsed(self.state, id_for.as_str());
        let fixed_h = if collapsed_now { None } else { eff_h };
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
            *self.state.window_z.entry(id_for.to_static()).or_insert(max_z + 1)
        };
        let saved_win = std::mem::replace(&mut self.painter.q.cur_win, z);
        // 当前窗口 ID（嵌套窗口 = 下拉浮层进出时保存/恢复）：`hit_impl` 记按下归属要用它，
        // 因为 z 会在帧末被"点击置顶"改，而复核要用**当前** z（见 `resolve_widget_press`）。
        let saved_win_id = self.cur_win_id.replace(id_for.to_static());
        // 当前窗口的"可交互内容范围"并集：进入时从零开始，退出时并进遮挡矩形
        // （浮层是嵌套窗口 ⇒ 保存/恢复，浮层的内容不该算进外层窗口）。
        let saved_hit_bounds = self.win_hit_bounds.take();
        let saved_clip = self.painter.q.clip;
        // **窗口不继承外层的裁剪层**（与"窗口是布局根"同一条边界规则）：窗口是**浮层** ——
        // 它的内容（以及它内部开的浮层：下拉 / 取色面板 / 子菜单）不该被外层沙箱裁掉。
        // 历史 bug：`vscroll(Scroll)` 窗口里开一个取色面板，面板的绘制命令带着**外层视口**
        // 的 scissor ⇒ 面板被"不经意地"裁掉（用户截图批注："子窗口被不经意地裁掉"）。
        // 本函数末尾恢复：外层沙箱后续的内容仍要按它的 scissor 走。
        self.painter.q.clip = None;
        // 位置经**责任链**解析（脚本处理器 → 用户拖拽状态 → 传入 pos，见 pos_handler）
        let origin = self.resolve_pos(&id_for, pos);
        let start = self.painter.q.queue.len();
        let style = style
            .cloned()
            .unwrap_or_else(|| self.theme.panel.clone());
        let (pad_total, gap) = (
            style.padding + style.border_w,
            content_gap.unwrap_or(self.theme.gap),
        );
        let saved_base = self.abs_base;
        let (sw, sh) = (
            self.window.inner_size().width as f32,
            self.window.inner_size().height as f32,
        );
        // 窗口尺寸（clamp 用）：**按 id 跨帧持久**（`window_sizes`）——点击置顶
        // z+1 后尺寸不丢 → clamp 边界稳定（消除"按下即跳变"）；`window_rects[z]`
        // 仅兜底；首帧（均无记录）= `None`。
        let prev_size = self
            .state
            .window_sizes
            .get(id_for.as_str())
            .copied()
            .or_else(|| {
                self.state
                    .window_rects
                    .get(id_for.as_str())
                    .map(|r| Vec2::new(r.w, r.h))
            });
        // ─── ① 位置与交互**先于内容录制**求解 ────────────────────────────
        // 鼠标事件是针对**屏幕上已有的几何**（上一帧结算的 `prev_size`）产生的，
        // 故命中 / 拖拽 / clamp 全用 `prev_size`；由此 `display_pos` 在录制前已知，
        // `abs_base` 与随后 `translate(display_pos)` 的几何**当帧一致**。
        //
        // ⚠ 旧实现 `abs_base` 取自上一帧位置（`base_pos`）、几何用本帧位置
        // （`display_pos`），于是**拖拽期间**每个走 `abs_base` 的绝对空间量都落后
        // 一帧：文本/多行框的 `box_clip`（文字被裁）、IME 光标定位、滑块拖拽基准、
        // 下拉浮层位置。位移越大错得越多 → 快速拖动时"文字/点击瞬间偏移"。
        //
        // 非首帧用持久尺寸 clamp（主窗口缩小 / 内容变化后窗口被拉回屏幕内 → 照常
        // 可点可拖）；**首帧（尺寸未知）不 clamp**——窗口出现在应用指定位置，当帧
        // 显示已 clamp，次帧收敛一致（消除首帧跳变）。
        let base_pos = match (clamp, prev_size) {
            (WindowClamp::Screen, Some(ps)) => {
                clamp_window_pos(saved_base + origin, ps, sw, sh) - saved_base
            }
            _ => origin,
        };
        // 命中矩形 = 屏幕上那个矩形（首帧无 `prev_size` ⇒ 本帧不参与交互）。
        let panel_rect = prev_size.map(|ps| Rect::new(base_pos.x, base_pos.y, ps.x, ps.y));
        // **拖拽缩放**（右下角柄）：宽度跨帧持久于 `UiState::window_widths`、高度持久于
        // `UiState::window_heights`。⚠ 交互须在窗口拖拽判定**之前**（`claim_press`
        // 阻止按下缩放柄时同时建立窗口拖拽基准）。基于通用 [`Self::resize_handle`]。
        // handle 为**外层容器局部坐标**（此处 `abs_base` 仍为外层原点）；用 clamp 后
        // 位置 `base_pos`（而非 origin）——与显示一致，贴边窗口缩放柄可命中。
        //
        // `(allow, axes)`：`None` = 旧行为（**有 `.width(..)` 就能横向拖**）；
        // `allow = false` ⇒ 不画柄也不响应拖拽（`.width(..)` 仍作布局固定宽）；
        // `axes`：`Horizontal` 只调宽 / `Vertical` 只调高 / `Both` 宽高同调（光标随之）。
        let (allow_resize, resize_axes) = resolve_window_resize(chrome.resize, width.is_some());
        // ⚠ **收起态没有尺寸可调**：收起 = 一行标题栏（固定高都被忽略），此时——
        // ① 柄的命中区（右下角 14~35px 方块）**压住标题栏最右那两个按钮**：短窗口（收起后
        //    只有一行高）里 ⌃/✕ 会被柄抢走 ⇒ 用户实测"resizable 的窗口在点击收起按钮时
        //    仍然不会收起"（点下去是在拖尺寸）；
        // ② 柄的**按下种子**会把"收起后的那一行高"写进 `window_heights`（`or_insert` 是
        //    永久的）⇒ 展开回来时窗口变成一条缝，看起来就是"点一下柄就弹回/跳回去了"。
        // 故收起态整条缩放链路关掉（不画、不命中、不种子、不应用）。
        let resize_on = allow_resize && resize_axes != Resize::None && !collapsed_now;
        // **窗口柄：命中在这里（内容之前），应用推迟到内容 + 标题栏之后** ——
        // 面板内最后一个控件的缩放柄（如 `TextEditor::resize(Both)`）常常和窗口柄叠在
        // 同一个右下角：内容后登记 ⇒ 控件级遮挡上它在上；应用再等 `!press_claimed` ⇒
        // "点控件的柄"不会连带把窗口也缩了（用户实测："在文本编辑器控件上缩放柄无法使用"）。
        // 同一机制修掉收起态里"窗口柄抢 ⌃/✕ 按钮"的既有缺陷（按钮在装饰段里 claim）。
        let mut grip = None;
        if resize_on
            && let Some(ps) = prev_size
        {
            // **命中区跟随柄的图案尺寸**（`GripStyle::extent`），下限 14px（太小的柄点不中）；
            // `GripShape::Hidden` 时退回下限 —— 图案可以不画，但**缩放能力保留**。
            let hw = style.grip.extent().max(14.0);
            let handle = Rect::new(base_pos.x + ps.x - hw, base_pos.y + ps.y - hw, hw, hw);
            let h_id = format!("{id}::resize");
            let cursor = match resize_axes {
                Resize::Both => crate::UiCursor::NwseResize,
                Resize::Vertical => crate::UiCursor::NsResize,
                _ => crate::UiCursor::EwResize,
            };
            let hit = self.resize_handle_hit(&h_id, &handle, cursor);
            // **按下即落持久值**（种子 = 当前尺寸）：`fixed_w/h` 与 `axis_dragged` 都在
            // **本函数前面**读过，不种这一下，第一次拖拽本帧不生效（要等下一帧）——
            // 症状正是"点击缩小窗口后不会缩小"（尤其内容比它高时，本帧仍按内容撑开）。
            if hit && self.mouse_left().down_edge() {
                // ⚠ 两个槽的**单位不一样**、必须按各自口径写：
                // - `window_widths` = **内容宽**（`set_fixed_w` 的语义，外框 = 内容 + 2×pad）；
                // - `window_heights` = **外框高**（`Frame::fixed_h` 就是结算高本身）。
                // 早先把外框宽写进内容宽槽 ⇒ 每次按下/拖拽都白涨 2×pad，**越拖越宽**
                // （用户实测："gallery 咋变这么宽"）。
                if resize_fixes_width(resize_axes) && explicit_w.is_none() {
                    self.state
                        .window_widths
                        .entry(id_for.to_static())
                        .or_insert((ps.x - pad_total * 2.0).max(1.0));
                }
                if resize_fixes_height(resize_axes) {
                    self.state
                        .window_heights
                        .entry(id_for.to_static())
                        .or_insert(ps.y);
                }
            }
            // 当前尺寸 = 屏幕上那个 = **本帧生效的尺寸**（`width` / `eff_h` 已是"持久优先"；
            // 两者都没有 = 自动尺寸，由上一帧外框换算）。**宽是内容宽、高是外框高** ——
            // 单位不同，混用会让每拖一次涨 2×pad；用**显式值**当基准则会让第二次拖拽
            // 从旧值起算（那条轴"弹回原位"）。
            let cur_w = width.unwrap_or((ps.x - pad_total * 2.0).max(1.0));
            let cur_h = eff_h.unwrap_or(ps.y);
            grip = Some((
                h_id,
                Vec2::new(cur_w, cur_h),
                cursor,
                hit,
                // 最小尺寸：宽 120；高至少装得下一行 + 上下内边距（拖到 0 高的窗口
                // 会变成"一条线"，既点不中柄也看不出是什么）。
                Vec2::new(120.0, (self.theme.row_h + pad_total * 2.0).max(40.0)),
            ));
        }
        let btn = self.mouse_left();
        // 窗口遮挡：被更高 z 的窗口覆盖的区域，本窗口不响应拖拽 / 置顶 /
        // 子控件交互（点击穿透修复——重叠区域只让最上层窗口可交互）。
        let hit = panel_rect.is_some_and(|r| hit_test(&r, self.mouse_logical))
            && self.mouse_in_window
            && !window_occluded(z, self.mouse_logical, self.window_rects_iter());
        let press_here = btn.down_edge() && hit;
        // 拖拽基准：按下帧**先无条件**建立（基准 = 屏幕上那个矩形）。窗口内子控件
        // （输入框选择 / 滑块 / 滚动条）随后声明本次按下（`press_claimed`）时，
        // 在内容录制后清除基准（见 ②）——判定顺序与旧版一致。
        let (active, new_pos, drag_clamp_size) = if clamp == WindowClamp::Locked {
            // 锁定：位置固定（不建立拖拽基准、不激活拖拽；点击置顶 / 子控件仍有效）。
            (false, origin, prev_size)
        } else {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            let (active, pos) = resolve_drag(ws, hit, btn, self.mouse_screen, base_pos);
            // 拖拽中 clamp 边界 = 按下帧尺寸（固定）→ 位置纯跟手、不因内容尺寸
            // 变化被推回（消除"拖动单帧跳变"）；非拖拽帧用持久尺寸（命中基准一致）。
            let clamp_size = if active { ws.press_size.or(prev_size) } else { prev_size };
            (active, pos, clamp_size)
        };
        // **Screen 限位**：窗口 clamp 到画面（窗口客户区）内——拖拽 / 脚本定位后
        // 的位置都被限制（绝对坐标 clamp 后回容器局部）；`Free` / `Locked` 不 clamp
        // （Locked 本身位置固定）。**clamp 尺寸**：拖拽中 = 按下帧尺寸（固定，
        // 边界稳定 → 无单帧跳变）；非拖拽 = 持久尺寸（与命中基准一致，
        // 主窗口缩小 / 窗口比画面大也不会"看得见拖不动"）；首帧无记录 = 本帧
        // 显示不 clamp（与旧版一致：`prev_size` 为 `None` ⇒ 用 `origin`）。
        let display_pos = match (clamp, drag_clamp_size) {
            (WindowClamp::Screen, Some(cs)) => {
                clamp_window_pos(saved_base + new_pos, cs, sw, sh) - saved_base
            }
            _ => new_pos,
        };
        // 内容基准 = **本帧显示基准**（`display_pos`）——录制期的绝对空间量与几何一致。
        self.abs_base = saved_base + display_pos;
        let saved_hit_limit = self.cur_win_hit_limit.take();
        // ── 按轴解算溢出策略（显式 `.vscroll/.hscroll` > `Placement::Clip` > "该轴被拖过"）
        // `axis_dragged` = 该轴**已有持久尺寸**（用户拖过 ⇒ 老语义里"由用户接管"）。
        let v_dragged = self.state.window_heights.contains_key(id_for.as_str());
        let h_dragged = self.state.window_widths.contains_key(id_for.as_str());
        let vs = resolve_scroll_mode(scroll.0, strict, v_dragged);
        let hs = resolve_scroll_mode(scroll.1, strict, h_dragged);
        // ⚠ **水平滚动条还没接上**（`ScrollState` 的第二条轴 + 横向 thumb）：`hscroll(Scroll)`
        // 先按 `ClipOnly` 处理并**打印一次**提示 —— 静默降级比"没实现"更难查。
        // `vscroll(Scroll)` 是**真的**（窗口内容包进滚动视口，见下面的 `viewport`）。
        let hs = if hs == ScrollMode::Scroll {
            if !HS_SCROLL_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!(
                    "ui: hscroll(Scroll) 暂按 ClipOnly 处理（水平滚动条 / 第二条滚动状态还没做，见 docs/UI_NEEDS.md）"
                );
            }
            ScrollMode::ClipOnly
        } else {
            hs
        };
        let mut frame = Frame::new_stack(PackSide::Top, gap, pad_total);
        // ── 宽度：**一律固定**（不再有"拖出来的是下限"这条）────────────────────────
        //
        // 老语义里 `NoClip` + 持久宽 = **下限**（内容可以把窗口撑得更宽）。它有两个
        // 后果，用户都实测过：拖过一次之后窗口被内容顶得**越拖越宽**（"gallery 咋变
        // 这么宽"）；而"压缩内容"根本没发生。egui 语义（用户给的判定表）是：
        // 水平轴要么**压缩**内容（`NoClip`）、要么**裁切**内容（`ClipOnly`/`Scroll`），
        // 两条都用**给定的宽**（`.width(..)` 或用户拖出来的持久宽）。
        // **有宽就固定宽**（`.width(..)` 的初始值 / 用户拖出来的持久值都已含在 `width` 里）；
        // 没宽 = 自动宽（由内容结算）。
        if let Some(w) = width {
            frame.set_fixed_w(w);
        }
        // **该轴是视口** ⇒ 子项按自然宽排布、不压缩（超出由下面的 clip 层裁掉）。
        // 这就是"**裁切内容**"与"**压缩内容**"的唯一开关（见 `Frame::set_clip_w`）。
        frame.set_clip_w(hs != ScrollMode::NoClip);
        // **窗口 = 布局根**：内容宽只能由本窗口给（`fixed_w` / 内容自然宽），**不许**
        // 向窗口外的容器借宽（根 frame 的固定宽是"视口宽"，那是给顶层 `win=0` 内容用的）。
        // 不设它 ⇒ 自动宽窗口里的 `divider()` / `Label` 拿到视口宽 ⇒ 整窗被撑成屏幕宽。
        frame.set_layout_root(true);
        // `vscroll(Scroll)` 的**视口高**在下面算（要等标题栏占位之后才拿得到内容原点）；
        // 这里先记下"该轴是滚动视口"，`set_fixed_h` 在那里做。
        // `fixed_h` / `eff_h` / `collapsed_now` 已在缩放柄之前算好（柄要用它）。
        match (vs, fixed_h) {
            // 视口轴（ClipOnly）：固定高，内容不撑高它。
            (ScrollMode::ClipOnly, Some(h)) => frame.set_fixed_h(h),
            // `NoClip` 轴：拖出来的高度是**下限**（内容仍可撑高）⇒ "大小必须能呈现所有内容"。
            (ScrollMode::NoClip, Some(h)) => frame.set_row_bounds(Some(h), None),
            _ => {}
        }
        // **可命中限制**：内容会被裁切的那条轴把命中范围钉在窗口矩形内 ⇒ 溢出到面板外的
        // **幽灵控件**既看不见也点不到（`NoClip` 的轴不限制：溢出可见就该可点）。
        // 用 `prev_size`（上一帧结算尺寸）：尺寸要等录完才知道，而鼠标事件本来就是针对
        // "屏幕上已有的几何"产生的（与命中 / 拖拽 / clamp 同一口径）。
        if (vs != ScrollMode::NoClip || hs != ScrollMode::NoClip)
            && let Some(ps) = prev_size
            && ps.x > 0.0
            && ps.y > 0.0
        {
            let w = saved_base.x + display_pos.x;
            let h = saved_base.y + display_pos.y;
            // 与外层窗口的限制求交（浮层是嵌套窗口）；"不裁"那条轴的兜底 = **屏幕**
            // （不裁 ≠ 无限，只是别用窗口边界去裁）。
            self.cur_win_hit_limit = clip_for_axes(
                saved_hit_limit,
                Rect::new(w, h, ps.x, ps.y),
                Rect::new(0.0, 0.0, sw, sh),
                vs,
                hs,
            );
        }
        self.frames.push(frame);
        self.painter.q.depth += 1;

        // ID 命名空间：窗口进入压栈、退出弹栈（闭包作用域保证配对——取代手动
        // push_id/pop_id，杜绝漏配对/多弹出）。窗口内子控件 ID 自动带窗口前缀。
        //
        // **标题栏 = "长宽已确定的容器"，在窗口结算之后再跑**（见 [`Self::ornament_at`]）。
        // 第一遍只**占位**：留出条高，让用户内容从条下开始（与旧实现同一个 y）。
        // 通条高度 = **一行**（条贴窗口顶边，起点 0）：内容上抬 `pad_total`、
        // 条高不含上内边距，下面的内容与窗口高度随之各少一个 `pad_total`。
        // ⚠ 条只是**背景装饰、不裁剪内容**：标题 / ▲ / ✕ 可以比条高再高一点（用户要求）。
        let bar_h = if chrome.bar_on() { title_bar_h(self.theme.row_h) } else { 0.0 };
        let collapsed = chrome.collapsed(self.state, id_for.as_str());
        // **占位宽度**：展开态由**内容**决定（标题不再撑宽窗口 —— 标题改为在最终宽度里
        // 居中 / 省略）；**没有内容**时（收起态、且未给 `.width()`）用"标题 + 按钮簇 +
        // 内边距"兜底，否则窗口会塌成 `2×pad`、标题被省略成空。
        // ⚠ 固定宽窗口（`.width(..)`）由 `fixed_w` 决定宽度 ⇒ 这里不需要占位宽。
        // （展开态但内容为空的自动宽窗口仍是 `2×pad`：那种窗口没有内容可依据，属已知取舍。）
        let reserve_w = if bar_h > 0.0 && collapsed && width.is_none() {
            let (row_h, gap) = (self.theme.row_h, self.theme.gap);
            // 先拷 held 值再调 `&mut self` 的方法（`font_family` 借用不能跨调用）。
            let (font_size, family) =
                (self.theme.label.font_size, self.theme.label.font_family.clone());
            let title_w = self
                .text_size(chrome.title.unwrap_or(""), font_size, family.as_deref())
                .x;
            // `None` = "没有外框宽"那条解算 ⇒ 返回"标题 + 按钮簇 + 内边距"的最低宽度。
            title_bar_layout(
                None,
                pad_total,
                row_h,
                gap,
                chrome.show_collapse(),
                chrome.close.is_some(),
                TITLE_BUTTON_INSET,
                title_w,
            )
            .bar_w
        } else {
            0.0
        };
        if bar_h > 0.0 {
            // **占位记录在窗口顶边**（`y = 0`，x 保持内容左缘）——与旧实现"把内容光标临时
            // 抬到 `y = 0` 再录标题行"同一口径：内容因此从 `bar_h + gap` 开始（**不是**
            // `pad_total + bar_h + gap`），窗口高度不会多出一个 `pad_total`。
            if let Some(fr) = self.frames.last_mut() {
                fr.cursor.y = 0.0;
            }
            self.child_rect(reserve_w, bar_h, Child::Expand);
        }
        // **结算 + 标题栏容器都在窗口 ID 命名空间内**（`with_id` 作用域里）：
        // 装饰控件的 id 必须带窗口前缀，否则帧末"被更高窗口遮挡"的复核会把标题栏按钮
        // 当成 **win=0 内容**而撤销它的按下（实测：`hit[..] ::collapse OK` 但点不动、
        // 收起状态永远不翻转）。
        let mut size = Vec2::ZERO;
        // **内容命令区间的右端**（见下面 `content_end = ..` 的赋值）：裁切层只盖这段。
        let mut content_end = start;
        // **内容原点**（窗口局部坐标）= 标题栏占位之后的游标。窗口内容盒的左上角就是它；
        // 裁切用的 scissor 必须**只有内容**（不含标题栏 / 内边距 —— 用户要求：
        // "裁切内容的绘制用 Scissor 矩形范围应该只有内容，没有标题栏"）。
        let content_origin = self
            .frames
            .last()
            .map(|f| f.cursor)
            .unwrap_or(Vec2::splat(pad_total));
        // ─── `vscroll(Scroll)`：把内容录进一个**滚动视口**（窗口内滚动条）───────────
        // 视口 = 窗口内容盒（标题栏之下），高 = 固定高（`.height(..)` / 拖过）**或**
        // "窗口顶到屏幕底还剩多少"（`Scroll` 的自然含义：不许跑出屏幕，多出来的滚）。
        //
        // ⚠ **收起态不建视口**（`!collapsed`）：收起就是"只剩一行标题栏"，而视口会
        // `set_fixed_h(视口高 + pad)` 把窗口重新顶高 ⇒ 用户实测的"resizable 窗口点 ⌃
        // 仍然不收起"（状态翻转了、内容也没了，但那个空面板还是原来那么高）。
        let scroll_axis_on = !collapsed && (vs == ScrollMode::Scroll || hs == ScrollMode::Scroll);
        // 该滚动容器的**跨帧状态**（上一帧的内容尺寸）：视口宽 / 内容定高 / 自动宽窗口的
        // 结算都用它（`ScrollState` 是 `Copy` 的 ⇒ 拷出来用，不与后面的 `&mut self` 打架）。
        let prev_scroll = self
            .state
            .scrolls
            .get(format!("{}/scroll", id_for.as_str()).as_str())
            .copied();
        let viewport = if scroll_axis_on {
            let top_left = content_origin;
            // ⚠ 用 `fixed_h`（**已按收起态清零**）而不是 `eff_h`：同一条"收起忽略固定高"
            // 的规则必须在这里也生效，否则视口会把忽略掉的那个高又拿回来。
            let avail_h = if let Some(h) = fixed_h {
                // 固定高（`.height(..)` / 拖过）：外框高 ⇒ 视口 = 它扣掉标题栏与内边距。
                h
            } else {
                // **没给固定高也没拖过 ⇒ 按内容定高**：视口 = min(内容需要的总高, 屏幕剩余)。
                // 只用"屏幕剩余"会把**内容很矮**的窗口也撑到屏幕底（用户实测："最开始打开
                // 调色板编辑器时仍然会把高度撑到窗口底端"的同一根因）。
                // 内容高从**上一帧**的滚动状态读（`ScrollState.content_h`；它是滚动容器结算
                // 出来的内容高，与视口无关）——首次会话未知 ⇒ 先按屏幕剩余，次帧收敛。
                let top_abs = saved_base.y + display_pos.y;
                let screen_avail = (sh - top_abs).max(0.0);
                match prev_scroll.map(|s| s.content_h) {
                    Some(ch) if ch > 0.0 => screen_avail.min(ch + top_left.y + pad_total),
                    _ => screen_avail,
                }
            };
            let h = avail_h - top_left.y - pad_total;
            // ⚠ 视口**横跨窗口内容盒**：`width` 已经是**内容宽**（`set_fixed_w` 的语义，
            // 外框宽 = `width + 2×pad_total`），再减一次内边距会让视口窄 2×pad（实测：
            // 条带跟着左移 26px ⇒ 脚本按"窗口右缘 − pad − 7"点的条带落空、`offset` 恒 0）。
            //
            // **自动宽窗口**（`width == None`）的视口宽：首选取**内容自己的自然宽**
            // （上一帧 `ScrollState.content_w`），其次上一帧结算宽反推，首帧用**屏幕剩余宽**
            // 当引导值 —— **绝不能是 1px**：视口 1px 宽 ⇒ 内容按 1px 折行（`Label` 一个字
            // 一行、`row` 里的控件被压到下限）⇒ 窗口塌成 `2×pad` 的一条缝，下一帧照它再算
            // ⇒ **永远一条缝**（用户实测："调色板编辑器在不指定 width 的情况下无法使用"）。
            let w = width.unwrap_or_else(|| {
                prev_scroll
                    .map(|s| s.content_w)
                    .filter(|w| *w > 0.0)
                    .or_else(|| prev_size.map(|s| (s.x - pad_total * 2.0).max(1.0)))
                    .unwrap_or_else(|| (sw - (saved_base.x + display_pos.x)).max(1.0))
            });
            Some(Rect::new(
                top_left.x,
                top_left.y,
                w.max(1.0),
                h.max(1.0),
            ))
        } else {
            None
        };
        self.with_id(id, |ui| {
            {
                // 显式重借用：`Window` 拿走 `&mut Ui`，块结束后 `ui` 仍可用。
                if let Some(vp) = viewport {
                    // 视口高固定 ⇒ 窗口尺寸 = 视口 + 内边距（内容再高也不撑大它）。
                    if let Some(fr) = ui.frames.last_mut() {
                        fr.set_fixed_h(vp.y + vp.h + pad_total);
                    }
                    // ⚠ 滚动容器的 id 用**相对名**（这里已经在 `with_id(id)` 的窗口命名空间里；
                    // 写 `{id}::scroll` 会变成 `win/win::scroll` —— 双重前缀）。
                    ui.scroll_at_axes(
                        Position::Physical(Vec2::new(vp.x, vp.y)),
                        Size::Physical(Vec2::new(vp.w, vp.h)),
                        "scroll",
                        vs,
                        hs,
                        |ui| {
                            let mut w = Window { ui };
                            if !collapsed {
                                f(&mut w);
                            }
                        },
                    );
                    // **自动宽窗口必须"看见"内容**（设计理念：**无顾虑地使用** —— 不写
                    // `.width(..)` 也要能用）：`scroll_at_axes` 是沙箱、**自己不 note**
                    // （它不知道调用方要不要这块占位），于是窗口帧的 `max_child.x` 恒 0 ⇒
                    // 窗口宽 = `2×pad` 的一条缝、内容按它折行。用刚写回的 `content_w/h`
                    // 补一条内容矩形 ⇒ 窗口撑到**内容的自然尺寸**（高那条会被上面的
                    // `set_fixed_h`/视口覆盖，不影响定高视口）。
                    if width.is_none()
                        && let Some(fr) = ui.frames.last_mut()
                        && let Some(st) = ui
                            .state
                            .scrolls
                            .get(format!("{}/scroll", id_for.as_str()).as_str())
                            .copied()
                    {
                        fr.note_content(Rect::new(vp.x, vp.y, st.content_w, st.content_h));
                    }
                } else {
                    let mut w = Window { ui: &mut *ui };
                    if !collapsed {
                        f(&mut w);
                    }
                }
            }
            // 结算尺寸（装饰之前）：标题/按钮不得反过来影响窗口尺寸。
            size = ui.frames.last().expect("window frame").settle_size();
            // **内容绘制的命令区间**（`[start, content_end)`）：裁切层只该盖**内容** ——
            // 标题栏（`ornament_at`）与面板底色 / 边框 / 缩放柄都在这之后录，**不参与**裁切。
            content_end = ui.painter.q.queue.len();
            // **标题栏容器**：宽 = 最终外框宽 ⇒ 按钮贴外缘、标题可按最终宽度居中/省略。
            // 插在**按下裁决之前**（裁决在 `with_id` 之后）：按钮的 `claim_press()` 必须
            // 早于窗口自己的裁决，否则"按按钮把窗口拖走"复现。
            if bar_h > 0.0 && size.x > 0.0 {
                let (chrome, id_for) = (&mut *chrome, &id_for);
                ui.ornament_at(Rect::new(0.0, 0.0, size.x, bar_h), |bar| {
                    // `style.radius`：最右按钮的右上角取**面板圆角**（贴外缘才嵌得进圆角）；
                    // `id_for`（绝对 ID）：引擎托管的收起状态（`collapsible: None`）按它存取。
                    window_title_bar(bar, chrome, id_for, collapsed, size.x, pad_total, style.radius);
                });
            }
        });

        // 弹出窗口 frame。**沿用上面已经算好的 `size`**：`ornament_at` 不 note_content
        // ⇒ 装饰不可能改变尺寸（语义保证：装饰绝不参与尺寸结算）。
        self.frames.pop().expect("window frame");
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        // 记录窗口尺寸（按 id 持久；点击置顶 z 变化后下帧 prev_size 仍可取）。
        self.state.window_sizes.insert(id_for.to_static(), size);
        // ─── ①b 窗口缩放柄的**应用**（内容 + 标题栏之后）────────────────────
        // 命中早在内容之前就算过（`grip.3`，为的是把命中区登记顺序排在内容之前 ⇒
        // 内容里的控件在控件级遮挡上获胜）；这里只在**没人认领这次按下**时把拖拽落到
        // 尺寸上：`TextEditor` 自己的缩放柄认领了就轮不到窗口。
        if let Some((h_id, cur, cursor, grip_hit, min)) = grip
            && !self.press_claimed
            && let Some(new_size) =
                self.resize_handle_apply(&h_id, cur, min, cursor, grip_hit)
        {
            // 新尺寸下帧生效（`width` / 高度于本函数开头读取）——与旧版一致，避免
            // 同帧内布局尺寸与 clamp 尺寸互相矛盾。
            if resize_fixes_width(resize_axes) {
                self.state.window_widths.insert(id_for.to_static(), new_size.x);
            }
            if resize_fixes_height(resize_axes) {
                self.state
                    .window_heights
                    .insert(id_for.to_static(), new_size.y);
            }
        }
        // ─── ② 内容录完后：按下裁决 ──────────────────────────────────────
        // 窗口内子控件（文本框选择 / 滑块 / 滚动条）在录制期可能已声明本次按下
        // （`press_claimed`）——此时**清除**拖拽基准，否则 `update_drag` 已置
        // dragging=true，残留的 press_mouse 会被 drag_moved 当作基准算出巨大位移
        // → 窗口"瞬移"（从输入框上拖拽 = 选择文本；窗口改从空白/标题区拖动）。
        if press_here {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            if self.press_claimed {
                // 输入框等文本控件按下（选择拖拽优先）：**清除拖拽基准**——
                // 否则 `update_drag` 已置 dragging=true，残留的 press_mouse 会被
                // drag_moved 当作基准，算出巨大位移 → 窗口"瞬移"。
                clear_drag_base(ws);
            } else {
                // 按下帧窗口尺寸：拖拽中 clamp 边界**固定**——内容尺寸变化不推窗。
                ws.press_size = Some(size);
            }
        }
        // 点击置顶（modal 对话框**不主动置顶**——它已最上，且避免 z 漂移/与浮层冲突）。
        // 重叠区域点击按下时记录"本帧按下命中的**最上层**窗口"（win_press_top），
        // `finish::resolve_win_press` 只保留它的拖拽与置顶——避免同时拖动多个窗口。
        if topmost
            && press_here
            && self
                .win_press_top
                .as_ref()
                .is_none_or(|(_, top_z)| self.painter.q.cur_win > *top_z)
        {
            self.win_press_top = Some((id_for.to_static(), self.painter.q.cur_win));
        }
        if active {
            self.drag_panel = Some(id_for.to_static());
            // 持久化 **clamp 后**的位置（下帧 origin 已限位，视觉与状态一致）。
            if self.state.panel_pos.get(id_for.as_str()) != Some(&display_pos) {
                self.state.panel_pos.insert(id_for.to_static(), display_pos);
            }
        } else if self
            .drag_panel
            .as_ref()
            .is_some_and(|d| d.as_str() == id_for.as_str())
        {
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
        // 记录窗口原点（顶点局部化基准；win=0 非窗口默认 (0,0)）与窗口 id（缓存 key）
        self.win_origins.insert(z, display_pos);
        // **按 id 再记一份**：同一帧可以有多个 z 相同的 `WIN_TOPMOST` 浮层（下拉里开子菜单），
        // z 键表只剩最后一个 ⇒ `debug_dump` 会漏掉嵌套浮层（渲染不受影响：采集减、提交加
        // 用的是同一个 z 键值）。id 键表让每个浮层都能被诊断到。
        self.state
            .window_origins
            .insert(id_for.to_static(), display_pos);
        self.win_ids.insert(z, id_for.to_static());
        // 窗口矩形入遮挡判定缓存（跨帧；finish 末尾只保留本帧录制的窗口）。
        // ⚠ 存**绝对**坐标：嵌套窗口 / 下拉浮层在容器内时 `display_pos` 是容器
        // 局部坐标，须加容器绝对原点（`saved_base`）——否则遮挡判定用绝对鼠标
        // 比局部矩形恒不命中，浮层背后的控件仍响应 hover/click（"下拉菜单选项
        // 悬停时背后按钮一起 Hover"）。
        //
        // **遮挡矩形 = 窗口盒子 ∪ 本帧子控件的命中区**（`win_hit_bounds`）：
        // 容器尺寸已保证包住子控件（见 `Frame::content_bounds`），这里是**兜底**——
        // 固定尺寸容器 / 有意溢出的装饰 / 未来新增的绝对放置 API 都还能保住
        // "看得见就能点"：遮挡判定按内容的实际范围走，而不是按边框盒子。
        let win_abs = Rect::new(
            saved_base.x + display_pos.x,
            saved_base.y + display_pos.y,
            size.x,
            size.y,
        );
        let occl = match self.win_hit_bounds {
            Some(b) => win_abs.union(&b),
            None => win_abs,
        };
        // **键 = 窗口绝对 ID**（不是 z）：z 会在帧末被"点击置顶"改，而矩形是这一刻录下的；
        // 按 ID 存 + 查询时解当前 z，才能让"刚被抬高的窗口"立刻按**新 z**参与遮挡判定。
        let wid = id_for.to_static();
        self.state.window_rects.insert(wid.clone(), occl);
        // 记入**帧级**"本帧录过的窗口"清单：帧末视图的 `win_ids` 会被 `save_frame_state`
        // 换空，只有这里记下的清单能在**下一帧开场**用来清陈旧（见 `window_ids_seen`）。
        if !self.state.frame_state.window_ids_seen.contains(&wid) {
            self.state.frame_state.window_ids_seen.push(wid);
        }
        // 严格裁剪（`window_at_strict`）：窗口内容**强制裁剪**到窗口矩形——结算后
        // 统一改写本窗口命令的裁剪层（录制期窗口尺寸未知，背景/子控件命令都覆盖；
        // 命中裁剪由窗口遮挡机制负责）。默认窗口为 Expand 语义（不裁剪）。
        // **按轴**强制裁剪：结算后统一改写本窗口命令的裁剪层（录制期窗口尺寸未知，
        // 背景 / 子控件命令都覆盖；命中侧由 `cur_win_hit_limit` 负责）。
        //
        // 每条轴独立：`NoClip` 的轴**不裁**（内容溢出可见，也能点），另一条轴照裁 ——
        // 这正是"只要纵向滚动条、横向不裁"能成立的原因。触发条件：
        // ① `.vscroll/.hscroll(ClipOnly|Scroll)`（显式）；
        // ② `.placement(Placement::Clip)`（老的一体化开关 ⇒ 两条轴都裁）；
        // ③ **该轴被用户拖过尺寸**（固定尺寸视口：内容再撑不开它，不裁就会画到面板外
        //    —— 用户实测的 TTT 窗口 bug）。
        if vs != ScrollMode::NoClip || hs != ScrollMode::NoClip {
            // **裁切矩形 = 窗口的"内容盒"**（不含标题栏、不含内边距）——用户要求：
            // "裁切内容的绘制用 Scissor 矩形范围应该只有内容，没有标题栏"。
            // 局部坐标 = `[content_origin, size − pad_total]`（`content_origin` 已在标题栏
            // 占位之后取过；无标题栏时它就是 `(pad, pad)`）。
            let content_box = Rect::new(
                saved_base.x + display_pos.x + content_origin.x,
                saved_base.y + display_pos.y + content_origin.y,
                (size.x - content_origin.x - pad_total).max(1.0),
                (size.y - content_origin.y - pad_total).max(1.0),
            );
            let clip = clip_for_axes(saved_clip, content_box, Rect::new(0.0, 0.0, sw, sh), vs, hs);
            // 诊断（`RJ_WINCLIP_TRACE=1`）：内容盒 scissor 的**输入与输出**——
            // "内容 scissor 不含标题栏"这类几何争议直接看这三行，不必猜。
            if std::env::var_os("RJ_WINCLIP_TRACE").is_some() {
                eprintln!(
                    "winclip[{id}] win=({:.0},{:.0},{:.0},{:.0}) content_origin=({:.0},{:.0}) box=({:.0},{:.0},{:.0},{:.0}) v={vs:?} h={hs:?} -> clip={clip:?}",
                    saved_base.x + display_pos.x,
                    saved_base.y + display_pos.y,
                    size.x,
                    size.y,
                    content_origin.x,
                    content_origin.y,
                    content_box.x,
                    content_box.y,
                    content_box.w,
                    content_box.h,
                );
            }
            // ⚠ **只盖内容自己的绘制，且只盖本窗口**：
            // - 区间 `[start, content_end)`：标题栏（`ornament_at`）与面板底色 / 边框 /
            //   缩放柄都在那之后录 ⇒ 标题栏**不会**被内容 scissor 裁掉；
            // - `d.win == z`：本窗范围内录的**嵌套浮层**（下拉 / 取色面板 / 子菜单的 z 是
            //   `WIN_TOPMOST`）各有自己的层级与裁剪，被父窗口的视口顺带裁掉就是
            //   "子窗口被不经意地裁掉"；
            // - **与已有的内层裁剪求交**（`clip_and`）：内容里的文本框盒 / 内层滚动视口
            //   必须保留自己更窄的 scissor（旧实现直接覆盖 ⇒ 内层裁剪被吃掉）。
            for d in &mut self.painter.q.queue[start..content_end] {
                if d.win == z {
                    d.clip = clip_and(d.clip, clip);
                }
            }
        }
        // 背景 + 边框（win = z，画在窗口子控件之下；radius > 0 走圆角双层矩形）
        let bg_rect = Rect::new(0.0, 0.0, size.x, size.y);
        self.push_panel_shadow(bg_rect, &style.shadow, style.radius);
        self.push_panel_like_img(bg_rect, style.bg, style.bg_image, style.border, style.border_w, style.radius, 0);
        // **拖拽缩放柄图案**：只在"允许拖拽 + 有轴"时画（`GripShape::Hidden` 时
        // `push_resize_grip` 自己短路）——菜单 / 下拉浮层用 `resize(false, Resize::None)`
        // 拿到"固定宽但不画柄、不可拖"的效果。
        if resize_on {
            self.push_resize_grip(size, &style.grip);
        }
        // **标题栏通条**（整窗宽、含面板内边距 ⇒ 通条观感）：在**这里**画（`size` 已知）、
        // `elem = 0` 且晚于面板背景入队 ⇒ 按 `(elem, seq)` 排在面板背景**之上**、
        // 所有控件（`elem ≥ 1`）**之下**。
        //
        // 一次 `push_panel_like` 就够：底色 `surface_raised` + 面板同色同宽边框 ⇒
        // 上/左/右三段边框与面板边框**连续**（不会"标题栏把上边框啃掉"），底边那条
        // 就是 1px 分隔线。圆角取面板的**上面两角**（只有下面两角贴直角的面板，
        // 通条才不会在圆角处出框）。
        if bar_h > 0.0 && size.x > 0.0 {
            let bar = Rect::new(0.0, 0.0, size.x, bar_h);
            let top_radius = CornerRadius {
                tl: style.radius.tl,
                tr: style.radius.tr,
                br: 0.0,
                bl: 0.0,
            };
            self.push_panel_like(
                bar,
                self.theme.palette.surface_raised,
                style.border,
                style.border_w,
                top_radius,
                0,
            );
        }
        for d in &mut self.painter.q.queue[start..] {
            d.translate(display_pos);
        }
        // 恢复外层的**裁剪层**（见入口处的说明：窗口不继承外层的裁剪）。
        self.painter.q.clip = saved_clip;
        self.painter.q.cur_win = saved_win;
        self.cur_win_id = saved_win_id;
        // 恢复外层窗口的"可交互内容范围"（本窗口已并进自己的遮挡矩形）。
        self.win_hit_bounds = saved_hit_bounds;
        // 恢复外层窗口的可命中限制（浮层 = 嵌套窗口）。
        self.cur_win_hit_limit = saved_hit_limit;
        size
    }

    /// **菜单栏**（横向；见 [`crate::widgets::MenuBar`] 的模块文档与用法示例）。
    ///
    /// **本质 = 一行（`row`）+ 一条覆盖整栏宽度的背景**：
    /// `f` 里 `bar.menu(label, |m| ..)` 加菜单（下拉面板内容用闭包写：`item` /
    /// `item_checked` / `caption` / `separator`，并 `Deref` 到 [`Window`] ⇒ 文本输入 /
    /// 分割线 / 按钮 / 横向排版都能放）；[`MenuBar`](crate::widgets::MenuBar) 又 `Deref` 到
    /// [`Pack`] ⇒ `bar.add(..)` / `bar.button(..)` / `bar.label(..)` / `bar.text_input(..)`
    /// / `bar.separator_v()`（**竖向分割线**）都能直接放进栏里。返回栏尺寸。
    ///
    /// - `pos` 是栏左上角（带单位，顶层放置时即屏幕坐标）；栏**不占父容器光标**（浮在顶层）；
    /// - `bar.width(..)` = 背景铺多宽（不调 = 子项撑多大就多大，如整条屏幕宽）；
    /// - 展开状态跨帧持久于 [`UiState::menu_open`]（触发器的**绝对 ID**）；
    /// - **同一时刻只有一个菜单开着**；点菜单项 / 点**栏外** / Esc 都会收起——
    ///   点栏内空白 / 栏里的竖分割线或别的控件**不会**收起（纯函数
    ///   `widgets::menubar::menu_bar_should_close`，逐组合单测）；
    /// - 下拉面板是 [`Level::Normal`] 浮层窗口 —— 想让它盖住别的窗口就把菜单栏录在
    ///   **各窗口之后**（窗口 z 在首次录制时按 `max+1` 分配）。背景同理只压同 `win` 的底层。
    pub fn menu_bar(
        &mut self,
        id: &str,
        pos: impl Into<Position>,
        f: impl FnOnce(&mut crate::widgets::MenuBar<'_, '_>),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        let open = self.state.menu_open.as_ref().map(|s| s.as_str().to_owned());
        // **栏 = 一行**：`PackSide::Left` + `Theme::menubar` 的内边距 / 间距（默认 4 / 2）、
        // `force_h_all(row_h)` ⇒ 触发器 / 竖分割线 / 塞进来的控件同一个行高。
        // 内边距为 0 时第一个触发器正好从 `pos` 起（`--sim-menu` 的坐标解算按主题取值）。
        let (mb_gap, mb_pad) = (self.theme.menubar.gap, self.theme.menubar.padding);
        let mut facts = None;
        let (size, _) = self.container(
            pos,
            Frame::new_stack(PackSide::Left, mb_gap, mb_pad),
            |ctx| {
                // 重借用：`Pack` 要**拿走**一个 `&mut Ui`（`MenuBar` 靠 `Deref` 到它拿 `UiAdd`），
                // 而 `ctx` 只是 `&mut ContainerCtx` ⇒ 借用 `*ctx.ui`（生命周期到闭包结束，
                // `bar.finish()` 就地消费，不逃出闭包）。
                let ui: &mut Ui<'_> = &mut *ctx.ui;
                let row_h = ui.theme.row_h;
                if let Some(fr) = ui.frames.last_mut() {
                    fr.set_force_h_all(row_h);
                }
                let mut bar = crate::widgets::MenuBar::new(Pack::new(ui), id, open);
                f(&mut bar);
                facts = Some(bar.finish());
            },
        );
        let facts = facts.expect("menu_bar: 闭包总是执行");
        // 栏矩形（**父容器局部**，与 `mouse_local()` 同一坐标系）：宽 = `bar.width(..)` 或内容宽。
        let bar_rect = Rect::new(pos.x, pos.y, facts.width.unwrap_or(size.x), size.y);
        let on_bar = hit_test(&bar_rect, self.mouse_local());
        let close = crate::widgets::menubar::menu_bar_should_close(
            facts.item_clicked,
            facts.down_outside,
            facts.on_trigger,
            on_bar,
            facts.esc,
        );
        if std::env::var_os("RJ_MENU_TRACE").is_some() {
            eprintln!(
                "menu[bar {id}] content=({:.0},{:.0}) bar=({:.0},{:.0} {:.0}x{:.0}) on_trigger={} \
                 on_bar={on_bar} down_outside={} item_clicked={} esc={} popup={:?} close={close}",
                facts.content.x,
                facts.content.y,
                bar_rect.x,
                bar_rect.y,
                bar_rect.w,
                bar_rect.h,
                facts.on_trigger,
                facts.down_outside,
                facts.item_clicked,
                facts.esc,
                facts.popup,
            );
        }
        // 展开 / 收起：`action`（本帧点了触发器 = 切换）优先；否则按收起规则。
        self.state.menu_open = match (facts.action, close) {
            (Some(a), _) => a.map(IdAbsolute::owned),
            (None, true) => None,
            (None, false) => self.state.menu_open.clone(),
        };
        // **全宽背景**：在子项**之后**录（`elem = 0`，但 seq 更大）⇒ 压在同深度底层绘制之上、
        // 所有控件之下（子项在 `container` 里 depth + 1 ⇒ 天然画在它之上）。
        // ⚠ 底边线**单独画**（不是四边环）：通栏条只要一条"下沿"，画环会在屏幕边缘多出两条
        // 竖线；线画在栏内下沿。`bar.border_w(0)` / 主题 `border_w = 0` ⇒ 不画。
        if bar_rect.w > 0.0 && bar_rect.h > 0.0 {
            let bg = facts.bg;
            self.push_panel_like(bar_rect, bg.bg, Color::TRANSPARENT, 0.0, bg.radius, 0);
            if bg.border_w > 0.0 {
                self.push_solid_rect(
                    Rect::new(
                        bar_rect.x,
                        bar_rect.y + bar_rect.h - bg.border_w,
                        bar_rect.w,
                        bg.border_w,
                    ),
                    bg.border,
                );
            }
        }
        size
    }

    /// **模态对话框**：全屏半透明遮罩（[`Theme::modal`](crate::style::Theme::modal)
    /// 的颜色/尺寸，默认全屏半透明黑）置于最上层，背后一切交互被遮挡（遮罩矩形
    /// 经窗口遮挡判定阻断，含顶层 win=0 内容）；对话框（可拖拽）浮于遮罩之上。
    /// `pos` 为对话框左上角（逻辑，相对当前容器原点；按顶层使用）。`Esc` 关闭由
    /// 调用方处理（见 [`crate::widgets::FontModal`]）。
    ///
    /// ⚠ **应在帧末（其它窗口之后）调用**：遮罩/对话框 z 每帧重写为"当前最大+1/+2"，
    /// 但本帧**之后**录制的窗口会分到更高 z 并绘制在其上——先录制窗口、最后录制
    /// modal，才能保证 Modal 恒在最上。
    ///
    /// 公开入口是 [`Self::modal`]（责任链 builder）。
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
        let dim_abs = self.id_for(dim_id.as_str());
        let z_dim = max_z + 1;
        // 遮罩与对话框 z **每帧强制重写**（不是 or_insert）——Modal 打开期间恒在最上：
        // ① 遮罩不被后置顶的窗口盖住；② 对话框不被自家遮罩盖住（or_insert 会保留
        // 旧 z，其它窗口置顶后遮罩 max+1 反超对话框旧 z → 字体窗口跑到遮罩后面）。
        self.state.window_z.insert(dim_abs.to_static(), z_dim);
        let dlg_abs = self.id_for(id);
        self.state.window_z.insert(dlg_abs.to_static(), z_dim + 1);
        // 遮罩矩形（**绝对物理坐标**；默认全屏 = 窗口客户区物理尺寸，
        // 可被 [`Theme::modal`] 的 `size` 覆盖）。
        let (mw, mh) = match self.theme.modal.size {
            Some(s) => (s.x, s.y),
            None => {
                let s = self.window.inner_size();
                (s.width as f32, s.height as f32)
            }
        };
        let dim_rect = Rect::new(0.0, 0.0, mw, mh);
        // 遮罩录制（win = z_dim；按顶层使用，局部 == 绝对）。
        let saved_win = std::mem::replace(&mut self.painter.q.cur_win, z_dim);
        let seq = self.next_seq();
        let depth = self.painter.q.depth;
        self.painter.q.queue.push(UiDraw {
            depth,
            seq,
            win: z_dim,
            elem: 0,
            rect: dim_rect,
            clip: self.painter.q.clip,
            kind: DrawKind::Solid(self.theme.modal.dim),
        });
        // 遮罩窗口矩形（遮挡判定用；绝对）。键按**遮罩的绝对 ID**（同 `window_impl`）。
        let dim_wid = dim_abs.to_static();
        self.state.window_rects.insert(dim_wid.clone(), dim_rect);
        if !self.state.frame_state.window_ids_seen.contains(&dim_wid) {
            self.state.frame_state.window_ids_seen.push(dim_wid);
        }
        self.win_ids.insert(z_dim, dim_abs.to_static());
        self.win_origins.insert(z_dim, Vec2::ZERO);
        self.painter.q.cur_win = saved_win;
        // 对话框窗口（window_impl 按 max+1 分配 → z = z_dim + 1，浮于遮罩之上；
        // **不主动置顶**——点击对话框/背景不触发 z 提升）。
        self.window_impl(
            id,
            pos,
            width,
            // modal 不设固定高（高度由内容定）。
            None,
            None,
            false,
            false,
            None,
            WindowClamp::Screen,
            // modal 不做按轴滚动 / 裁切（要滚动就在对话框内容里嵌 `scroll_at`）。
            (None, None),
            &mut WindowChrome::none(),
            f,
        )
    }

    // ── 容器责任链 builder 入口（window / panel / modal） ──────────

    /// 建**窗口**（可重叠 + 焦点置顶 + 可拖拽）：返回 [`WindowBuilder`]，选项链式设置后
    /// 以 `.show(f)` 执行（返回窗口尺寸）。
    ///
    /// **唯一窗口入口**（旧 `window_at` / `window_at_w` / `window_at_strict` /
    /// `window_at_strict_w` 四个变体已删除——选项改由枚举表达，无裸布尔）：
    ///
    /// ```no_run
    /// # use rjw_ui::{Level, Placement, Ui, UiAdd};
    /// # let mut ui: rjw_ui::Ui = todo!();
    /// ui.window("hud")
    ///     .pos(glam::Vec2::new(400.0, 40.0))
    ///     .width(280.0)                       // 可选固定宽（右下角可缩放，跨帧持久）
    ///     .placement(Placement::Clip)         // 可选严格裁剪（默认 Expand = 不裁剪）
    ///     .level(Level::Normal)               // 可选：点击不置顶（默认 Topmost）
    ///     .style(rjw_ui::PanelStyle::default().with_radius(8.0)) // 可选逐窗口样式
    ///     .show(|w| { w.label("HUD"); });
    /// ```
    pub fn window<'s>(&'s mut self, id: &'s str) -> WindowBuilder<'s, 'a> {
        WindowBuilder {
            ui: self,
            id,
            o: WindowOptions::default(),
            title: None,
            close: None,
            collapsible: None,
        }
    }

    /// 建**面板**（背景 + 边框 + 内容垂直堆叠）：返回 [`PanelBuilder`]，链式设置后
    /// 以 `.show(f)` 执行。统一 [`Self::panel_at`] / [`Self::drag_panel_at`]。
    pub fn panel<'s>(&'s mut self) -> PanelBuilder<'s, 'a> {
        PanelBuilder { ui: self, pos: Position::Logical(Vec2::ZERO), drag: None, style: None }
    }

    /// 建**模态对话框**（全屏半透明遮罩 + 对话框）：返回 [`ModalBuilder`]，链式设置后
    /// 以 `.show(f)` 执行。统一 [`Self::modal_at`] / [`Self::modal_at_w`]。
    pub fn modal<'s>(&'s mut self, id: &'s str) -> ModalBuilder<'s, 'a> {
        ModalBuilder {
            ui: self,
            id,
            pos: Position::Logical(Vec2::ZERO),
            width: None,
        }
    }

    /// pack 容器：按 `side` 堆叠，尺寸自动。
    pub fn pack_at(
        &mut self,
        pos: impl Into<Position>,
        side: PackSide,
        f: impl FnOnce(&mut Pack<'_, '_>),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        let gap = self.theme.gap;
        self.container(pos, Frame::new_stack(side, gap, 0.0), |ctx| {
            let mut p = Pack { ui: ctx.ui };
            f(&mut p);
        })
        .0
    }

    /// 当前容器**下一子项**的最小尺寸约束（`0` = 该轴不约束；一次性）。
    /// 容器内便捷方法：`p.min_size(120.0, 0.0)`（见 [`crate::Ui`] 文档 / 示例）。
    fn set_next_min(&mut self, min: impl Into<Vec2>) {
        self.frames
            .last_mut()
            .expect("min_size 需在容器内调用（顶层请用 *_at 定位）")
            .set_next_min(min.into());
    }

    /// 当前容器**下一子项**的最大尺寸约束（`0` = 该轴不约束；一次性）。
    fn set_next_max(&mut self, max: impl Into<Vec2>) {
        self.frames
            .last_mut()
            .expect("max_size 需在容器内调用（顶层请用 *_at 定位）")
            .set_next_max(max.into());
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
        pos: impl Into<Position>,
        total_h: impl Into<Size<f32>>,
        weights: &[u32],
        mut f: F,
    ) -> Vec2
    where
        F: FnMut(&mut FlexCtx<'_, '_>, usize),
    {
        let pos = pos.into().to_physical(self.scale);
        let total_h = total_h.into().to_physical(self.scale);
        let gap = self.theme.gap;
        let start = self.painter.q.queue.len();
        self.begin_top_placement();
        let saved_base = self.abs_base;
        self.abs_base = saved_base + pos;
        let mut frame = Frame::new_stack(PackSide::Top, gap, 0.0);
        frame.set_fixed_h(total_h);
        self.frames.push(frame);
        self.painter.q.depth += 1;
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
        let inner_bounds = frame.content_bounds();
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        for d in &mut self.painter.q.queue[start..] {
            d.translate(pos);
        }
        if let Some(parent) = self.frames.last_mut() {
            parent.note_content(Rect::new(pos.x, pos.y, size.x, size.y));
            if let Some(ib) = inner_bounds {
                parent.note_content(Rect::new(ib.x + pos.x, ib.y + pos.y, ib.w, ib.h));
            }
        }
        size
    }

    /// grid 容器：`cols` 列均匀网格，单元格尺寸跨帧缓存（`id` 须稳定）。
    pub fn grid_at(
        &mut self,
        pos: impl Into<Position>,
        cols: usize,
        id: &str,
        f: impl FnOnce(&mut Grid<'_, '_>),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        assert!(cols > 0, "grid cols must be > 0");
        // grid 容器是命名空间边界：内部子控件（如背包格子按钮）ID 自动带前缀。
        let abs = self.id_for(id);
        let cell = self.state.grid_cells.get(abs.as_str()).copied().unwrap_or(Vec2::ZERO);
        let mut result = (Vec2::ZERO, Vec2::ZERO);
        self.with_id(id, |ui| {
            result = ui.container(pos, Frame::new_grid(cols, cell, 0.0), |ctx| {
                let mut g = Grid { ui: ctx.ui };
                f(&mut g);
            });
        });
        let (size, max_child) = result;
        // 回写单元格缓存：**内容变化时同时允许扩大与缩小**（未达到 max 的控件按
        // 内容自动改大小——如背包格子文字变长/变短；内容不变时布局依旧跨帧稳定）。
        self.state.grid_cells.insert(abs.to_static(), max_child);
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
    /// **提交方式**（不使用 Sprite）：全部图元（背景 / 圆角 / 渐变 / 控件背景 / 文字）
    /// 转为**顶点 + 三角形索引**（圆角由 `crate::tess` 镶嵌），按 `(窗口, 纹理, 变换)`
    /// 分组后**由 UI 自行决定提交顺序**（不依赖 Render2D 排序）：
    /// `(win 升序, 白纹理图形组 → 字形文字组, 纹理 uid)`——
    /// 1. 非窗口内容（`win=0`）最底，窗口按 z 从下到上（`layer = base + z`）；
    /// 2. 同一窗口内**"背景/图形 → 文字"严格成立**（白纹理组先于字形图集组），
    ///    跨帧稳定、与 Render2D 任意排序模式结果一致。
    ///
    /// **UI 的 Render2D 必须关闭排序**（`sort(SortMode::None)`，完全按提交顺序绘制）：
    /// UI 自行管理绘制顺序，排序键 `(win, depth, elem, group, seq)` 依赖**提交顺序**
    /// 生效（图形组在文字组之前提交）。⚠ `SortMode::LayerAndStates`
    /// 会在同一 layer 内按 `(rstates, texture_uid)` 重排——字形图集页先于程序化纹理页
    /// （圆角/渐变）注册，重排后**圆角/渐变图形会盖住文字**；`SortMode::LayerOnly`
    /// （稳定按 layer 排序）可接受（同层保持提交顺序）。
    /// 提交本帧 UI 到渲染器。**视口与渲染器在此延迟传入**（`begin` 时不需要）——
    /// 录制阶段可完全独立于绘制资源；`viewport`（屏幕矩形，见 [`rjw_transform::Rect`]）
    /// 提供屏幕固定变换，`r2d` 接收四边形。UI 不需要相机（恒为 identity：不旋转/缩放），
    /// 仅需视口矩形。
    /// **段收尾**：把本段录制的命令分桶 → 生成 / 复用顶点 → 提交到 `backend`
    /// （`rjw_ui` 只产出 `UiBatch`；渲染器由调用方适配）。**一帧可调用多次**（多段 UI），
    /// 帧级收尾在 [`Self::end_frame`]。
    ///
    /// 流水线（职责见各自文档注释）：
    /// 1. 命令分桶 + WHITE 纹理解析（本函数内联）；
    /// 2. 按窗口生成可提交顶点 / 顶点缓存（`cache_all_windows` → 子槽 `cache_z0_window`
    ///    / 窗口 `cache_window`；签名 `hash_cmds`）；
    /// 3. 排序 + 连续运行合批提交（`submit_quads` / `flush_seg`）；
    /// 4. Debug 叠加（`submit_debug`：debug_queue / 布局描边 / 焦点描边，恒覆盖在最上）；
    /// 5. 段统计累加进帧级暂存 + 帧级事实回存（`save_frame_state`）。
    pub fn finish(&mut self, backend: &mut dyn UiBackend) {
        let t_finish = Instant::now();
        // 提交序 = (窗口 z → 深度 → 元素序 → 元素内图形/文字 → 命令序)。**免全量排序**：
        // 命令按录制序（seq）生成，同深度内 `(elem, group, seq)` 天然有序（元素随录制
        // 递增、同元素"背景/图形"先于文字录制）；唯一乱序维度是 `depth`（容器嵌套
        // 进出）与 `win`（跨帧 z 缓存使录制序 ≠ z 序）→ 按 win 分桶 + 桶内 depth
        // 分桶、桶内保持录制序，与 `sort_by_key((win, depth, elem, group, seq))`
        // **完全等价**（O(n + 桶数)，免每帧 O(n log n)）。
        let t_sort = Instant::now();
        let cmd_count = self.painter.q.queue.len() as u32;
        let queue = std::mem::take(&mut self.painter.q.queue);
        // **WHITE 基础纹理优先取字形图集页**（`Text::white_region`，1×1 clamp_margin）：
        // 实心填充（Solid / 边框 / 光标）与字形**同页同纹理** → 同窗口内"图形组 → 文字组"
        // 相邻且同纹理，后端的连续段合批合成单次 draw call，省去图形↔文字的纹理状态切换。
        // 兜底：`rjw_text` 不可用时用当前帧第一个命令引用的纹理（整纹理 UV）。
        let (white_uid, white_uv_tl, white_uv_wh) = match self.text.white_region() {
            Some(r) => {
                let inv = match backend.texture(r.page_uid) {
                    Some(t) => 1.0 / t.width as f32,
                    None => 1.0,
                };
                let tl = Vec2::new(r.tl_px.0 as f32, r.tl_px.1 as f32) * inv;
                let wh = Vec2::new(r.wh_px.0 as f32, r.wh_px.1 as f32) * inv;
                (r.page_uid, tl, wh)
            }
            None => (0, Vec2::ZERO, Vec2::ONE),
        };
        // 按窗口分组 + 组内 depth 分桶（桶内保持录制序）：非窗口（win=0）每帧重建；
        // 窗口按**内容签名**缓存局部顶点，内容不变时复用（移动窗口只改变换，顶点不重建）。
        let mut groups = bucket_cmds(queue);
        let mut wins: Vec<u32> = groups.keys().copied().collect();
        wins.sort_unstable();
        let sort_us = t_sort.elapsed().as_secs_f64() * 1e6;
        // —— 计时累加器（本帧各阶段 µs 统计，finish 末尾写入 UiState.stats） ——
        let mut stats = CacheStats::default();
        // 收集器：**未命中**的单元在这里镶嵌（含 `debug_layout` 的布局描边——那条路径
        // 每帧重建，见 `collect_units`）。命中单元不进这里：它们直接在缓存里。
        let mut quads = QuadCollector::new(white_uid, white_uv_tl, white_uv_wh);
        // —— 收集本帧的**提交单元**（窗口 / win=0 顶层放置）——
        //
        // 每个单元 = `(win, place)`：一扇窗，或 win=0 的一个顶层放置。命中 ⇒ 计划从缓存
        // **move 出来**（零顶点拷贝）；未命中 ⇒ 现场镶嵌 + 合并（`build_plans`），本帧
        // 照常提交它，提交完再 move 回缓存（下面的回填循环）⇒ **"重建帧照常绘制、
        // 不消失一帧"由结构保证**（见 `collect_units` 的文档）。
        let t_asm = Instant::now();
        let mut units: Vec<SubmitUnit> = std::mem::take(&mut self.state.scratch_units);
        units.clear();
        self.collect_units(&mut groups, &wins, &mut units, &mut quads, &mut stats);
        // 单元级排序（每帧 ~15 个）：`(win, place)`——非窗口内容（win=0）按**放置序**
        // （后录制者在上），窗口按 z（非窗口恒在窗口之下）。**单元内**的顺序已在计划里
        // 烘好（`BatchPlan`）：`elem`（后录控件覆盖先录）+ `g`（图形先于文字）+ `tex` +
        // `clip`（一次 draw 只能一个 scissor）。
        units.sort_unstable_by_key(|u| (u.win, u.place));
        // 「装配」= 收集 + 合并 + 单元排序，扣掉已单独计时的 `sig` / `collect`
        // （那是"索引"与"镶嵌"的账，不是几何搬运）。
        let submit_asm_us =
            (t_asm.elapsed().as_secs_f64() * 1e6 - stats.sig_us - stats.collect_us).max(0.0);
        // ── 提交：每个计划一次 `flush_seg`。计划是**借来的**（`&BatchPlan`）⇒ 命中路径
        //    全程零拷贝；`UiBatch` 借用切片交给后端，不做"再拷一份"。
        //
        // ⚠ **UI 自行管理绘制顺序**：UI 的 Render2D 必须 `set_sort_mode(SortMode::None)`
        // （关闭排序，完全按提交顺序绘制）；`LayerOnly`（稳定排序）同层保持提交顺序也可。
        // 不要用 `LayerAndStates`：它按 `(rstates, texture_uid)` 重排，字形图集页 uid <
        // 程序化纹理页 uid → 圆角/渐变会被排在文字之后绘制，盖住文字。
        //
        // transform = 屏幕固定变换（窗口原点物理像素）→ 局部顶点映射到世界。
        //
        // **绘制序追踪**（诊断，`RJ_ORDER_TRACE=<帧号>|all`）：按**真实提交顺序**逐条打印
        // 批次计划。为什么需要它：`elem = 0` 的语义是"画在**本容器**元素之下"，只有
        // "每个顶层放置一个排序空间"（`place`）才成立；"某个 win=0 控件被别的 win=0 内容
        // 穿透 / 看错层级"这类现象只能靠这份序核对。
        //
        // 阶段 9 起计划是"单元内已合并好的"，所以打的是**计划级**：`(win, place, tex,
        // clip, verts, 元素区间, 首顶点)`——`v0` 用来认出这是谁（如面板底 `(30,30)` vs
        // FPS 标签 `(25,23)`）。**单元内**（`elem = 0` 的装饰 vs 自家内容）的顺序由
        // `merge_plans` 的排序保证，单测
        // `submit_order_puts_a_placement_above_earlier_content_and_below_its_own_children` 钉住。
        //
        // ⚠ 一帧 ~50 行且 `eprintln!` 无缓冲：`all` 时**必须重定向到真文件**（经
        // PowerShell 管道重定向会顶满缓冲把应用压到 ~1fps——实测踩过，与引擎无关）。
        let trace = match std::env::var("RJ_ORDER_TRACE") {
            Ok(v) if v == "all" => true,
            Ok(v) => v.parse::<u64>().map(|f| f == self.state.frame).unwrap_or(false),
            Err(_) => false,
        };
        if trace {
            let mut i = 0usize;
            for u in &units {
                for plan in &u.plans {
                    let v0 = plan
                        .geom
                        .verts
                        .first()
                        .map(|v| (v.pos[0].round(), v.pos[1].round()));
                    eprintln!(
                        "order[frame {}] i={i} win={} place={} tex={} clip={} verts={} elems={}..{} v0={v0:?}",
                        self.state.frame,
                        u.win,
                        u.place,
                        plan.texture,
                        if plan.clip.is_some() { 1 } else { 0 },
                        plan.geom.verts.len(),
                        plan.elements.first().copied().unwrap_or(0),
                        plan.elements.last().copied().unwrap_or(0),
                    );
                    i += 1;
                }
            }
        }
        let t_flush = Instant::now();
        let layer_base = self.base_layer;
        for u in &units {
            for plan in &u.plans {
                if self.flush_seg(backend, layer_base, u.win, plan) {
                    // 段统计（`[perf] segs=/verts=/tris=`）：**段数 = draw call 候选数**，
                    // 顶点/三角数是"这一帧到底提交了多少"的直接度量。
                    let acc = &mut self.state.frame_state.stats;
                    acc.seg_count = acc.seg_count.saturating_add(1);
                    acc.vert_count = acc.vert_count.saturating_add(plan.geom.verts.len() as u32);
                    acc.tri_count = acc.tri_count.saturating_add(plan.geom.tris.len() as u32);
                }
            }
        }
        let submit_flush_us = t_flush.elapsed().as_secs_f64() * 1e6;
        let submit_us = submit_asm_us + submit_flush_us;
        // ── 计划回填缓存（**move**，不是克隆）：命中与未命中都写"本帧的计划 + 本帧的
        //    签名"——未命中时这份就是刚建好的新计划，于是"本帧提交的"与"缓存里的"逐位
        //    一致，下一帧直接命中。
        for u in &mut units {
            match u.slot.take() {
                // ⚠ 同帧同 id 出现两次时以**后者**为准（与旧实现的 `insert` 语义一致）。
                Some(CacheSlot::Window(id)) => {
                    self.state
                        .window_quads
                        .insert(id, (u.sig, std::mem::take(&mut u.plans)));
                }
                Some(CacheSlot::Z0(slot)) => {
                    self.state
                        .z0_quads
                        .insert(slot, (u.sig, std::mem::take(&mut u.plans)));
                }
                None => {}
            }
        }
        units.clear();
        self.state.scratch_units = units;

        // ── Debug 叠加（DebugDraw / debug_layout 描边 / 焦点描边）────────────
        // 在**全部 UI 内容之后**提交（`submit_debug`：合并 debug_queue 与布局描边、
        // 按 win 分组、白纹理、屏幕固定变换提交——不进窗口缓存、恒覆盖在最上）。
        self.submit_debug(backend, &mut quads, white_uid, layer_base);
        // ── 段统计**累加**进帧级暂存 ─────────────────────────────────────
        // （`UiStats` 是每帧聚合值：帧号 / `ui_frame_us` 由 `Ui::end_frame` 写回。）
        let acc = &mut self.state.frame_state.stats;
        acc.cmd_count = acc.cmd_count.saturating_add(cmd_count);
        acc.win_count = acc.win_count.saturating_add(stats.win_count);
        acc.cache_hits = acc.cache_hits.saturating_add(stats.cache_hits);
        acc.cache_misses = acc.cache_misses.saturating_add(stats.cache_misses);
        acc.sort_us += sort_us;
        acc.sig_us += stats.sig_us;
        acc.collect_us += stats.collect_us;
        acc.clone_us += stats.clone_us;
        acc.submit_us += submit_us;
        acc.submit_asm_us += submit_asm_us;
        acc.submit_flush_us += submit_flush_us;
        acc.finish_us += t_finish.elapsed().as_secs_f64() * 1e6;
        // 记录 IME 组合状态（供下一帧退格判定，见 text_input_at）
        self.state.ime_composing =
            self.keyboard.ime_preedit().is_some_and(|p| !p.is_empty());
        // **段收尾**：本段的帧级事实（按下归属 / 窗口原点 / 焦点链 / 责任链 / 光标意图 /
        // z0 组号）回存暂存，供同帧后续段与帧收尾使用。
        // ⚠ 帧级收尾（输入结算 / 焦点导航与描边 / 光标定夺 / 统计写回 / 复位）**不在这里**：
        // 那是每帧一次的事，搬到 [`Self::end_frame`]。
        self.save_frame_state();
    }

    /// **帧收尾**（**每帧一次**，由运行时在 UI 队列被提交之前调用；低层手动路径自己调）。
    ///
    /// 与 [`Self::finish`]（段收尾：分桶 → 顶点 → 提交）分工明确：
    /// 1. [`Self::finish_pre_input`]：空白点击清焦点 / 清一次性边沿 / 窗口按下裁决
    ///    （`resolve_win_press`）/ 键盘导航（Tab·方向键 / Esc）；
    /// 2. 焦点描边（`handle_focus_keys` 内，走 `debug_queue`：不进窗口缓存、恒覆盖在最上）；
    /// 3. `finish(backend)`：把上面追加的描边命令 flush 出去（本视图自身命令通常为空）；
    /// 4. [`Self::finalize_cursor_and_reset`]：系统光标定夺 / `window_rects` 清理 /
    ///    统计写回（帧号 +1、`ui_frame_us` = 开场 → 收尾）/ 帧级字段复位；
    /// 5. [`UiState::end_frame`]：帧级暂存关场（下次开场重新清零）。
    pub fn end_frame(&mut self, backend: &mut dyn UiBackend) {
        self.finish_pre_input();
        self.finish(backend);
        self.finalize_cursor_and_reset();
        self.state.end_frame();
    }

    /// **帧末输入结算**（`finish` 起始）：空白点击清焦点（本帧按下且无控件响应）、
    /// 清除一次性边沿、窗口按下裁决（重叠点击只让最上层窗口拖拽与置顶）、键盘导航
    /// （Tab/方向键遍历焦点链、Esc 关浮层/失焦、焦点描边）。须在本帧命令取走
    /// （录制队列 `mem::take`）**之前**调用——焦点描边会追加到队列。
    fn finish_pre_input(&mut self) {
        // 空白点击清焦点（本帧按下且无控件响应）
        if self.mouse_left().down_edge() && !self.any_pressed && self.state.focused.is_some() {
            self.state.focused = None;
            self.state.focused_kind = None;
        }
        // 清除一次性边沿
        for ws in self.state.widgets.values_mut() {
            clear_frame_flags(ws);
        }
        // 窗口按下裁决：重叠点击只让**最上层**窗口获得拖拽与置顶（见 window_at）
        self.resolve_win_press();
        // 控件按下裁决：**用帧末完备的遮挡表复核**本帧认领按下的控件（见该方法文档）。
        self.resolve_widget_press();
        // 键盘导航：Tab / Shift+Tab / 方向键遍历焦点链、Esc 关浮层/失焦、焦点描边。
        self.handle_focus_keys();
    }

    /// **控件按下裁决**（帧末复核）：本帧有控件认领了按下，但**帧末**看它所在的窗口并非
    /// 鼠标下最上层（被更高 z 的窗口盖住）⇒ 撤销这次认领。
    ///
    /// # 为什么需要"帧末复核"
    ///
    /// 窗口遮挡判定用的是 `UiState::window_rects`——一张**录制期逐步写入**的表：本帧还没
    /// 录到的窗口，表里是**上一帧**的矩形。于是"盖住我的那个窗口本帧才移过来 / 本帧才被抬高
    /// z"这两种情况下，控件命中时会被判成"没被遮挡"而收下按下。命中的那一刻几何还没录全，
    /// **唯一能拿到完备几何的时机就是帧末**（所有窗口都录完了），所以复核放在这里。
    ///
    /// # ⚠ 比较基准必须是**窗口的当前 z**，不是认领时的旧 z
    ///
    /// `resolve_win_press` 在帧末会把**被点的窗口**抬到 `max+1`；若这里拿"认领时的旧 z"去比，
    /// 那个窗口的**新 z 比自己的旧 z 大**、且它的矩形当然覆盖鼠标（鼠标就在它里面）⇒
    /// **窗口把自己判成"被别人盖住"**，于是**每一次**窗口内控件的按下都被撤销：滑块 /
    /// 滚动条 / 文本选择全都拖不动（真实回归，已由 `--sim-cover` 的正对照守住）。
    /// 用当前 z 后，"自己"与"自己"相等，而 `window_occluded` 是**严格大于**判定 ⇒ 自己永远
    /// 不遮挡自己，同时"确实被别人盖住"依旧成立。
    ///
    /// 代价与边界：一次按下已在命中那一帧被应用侧读到（动作已发生）——本条只保证
    /// **状态不再延续**（`pressed` / `clicked` / `dragging` 全清、`press_claimed` 保持），
    /// 因此不会出现"被盖住的控件一直拖到释放"。同帧内按下 + 抬起（极快点击）不受保护。
    fn resolve_widget_press(&mut self) {
        let Some((id, win_id)) = self.state.frame_state.press_widget.take() else {
            return;
        };
        // 本控件所在窗口的**当前** z（`None` = win=0 非窗口内容 ⇒ z=0）。
        let my_z = win_id
            .as_ref()
            .and_then(|w| self.state.window_z.get(w.as_str()).copied())
            .unwrap_or(0);
        if !window_occluded(my_z, self.mouse_logical, self.window_rects_iter()) {
            return;
        }
        let ws = self.state.widgets.entry(id).or_default();
        ws.pressed = false;
        ws.clicked = false;
        ws.dragging = false;
        self.state.press_cancelled_by_window = self.state.press_cancelled_by_window.saturating_add(1);
    }

    /// **收集本帧的提交单元**（`finish` 的核心缓存步骤）：遍历分桶后的窗口，把每个
    /// **提交单元**（一扇窗 / win=0 的一个顶层放置）变成一份 [`BatchPlan`] 列表：
    ///
    /// - **命中 ⇒ `remove` 出计划**（move 一个 `Vec`，**零顶点拷贝**）；
    /// - **未命中 ⇒ 现场镶嵌 + 合并**（[`Self::build_plans`]），本帧照常提交它，提交完
    ///   再把计划 move 回缓存（`finish` 末尾的回填循环）。
    ///
    /// "重建帧照常绘制、不消失一帧"这条**历史不变量由结构本身保证**：数据是当前帧亲手
    /// 从缓存里拿出来的**所有权值**，z→id 映射只解一次，不存在"按旧 z 读新表"的竞态
    /// （那是更早的"零拷贝两阶段读取"版本踩过的坑）。
    ///
    /// `debug_layout` 开启时**每帧重建**（布局描边是调试视图，跳过缓存）。
    /// 各阶段耗时 / 计数累加到 `stats`（`finish` 末尾写入 [`UiStats`]）。
    fn collect_units(
        &mut self,
        groups: &mut std::collections::HashMap<u32, Vec<Vec<UiDraw>>>,
        wins: &[u32],
        units: &mut Vec<SubmitUnit>,
        quads: &mut QuadCollector,
        stats: &mut CacheStats,
    ) {
        for &win in wins {
            let cmds = groups.remove(&win).expect("group exists");
            if win == 0 {
                // 非窗口内容：按**顶层放置**切单元（`place` 既是排序空间也是缓存键）。
                self.collect_z0_units(&cmds, units, quads, stats);
                continue;
            }
            let Some(id) = self.win_ids.get(&win).cloned() else {
                // 没有 id 的窗口（理论上不会有）⇒ 只现场建计划，不进缓存。
                let plans = self.build_plans(win, &cmds, quads, 0, stats);
                units.push(SubmitUnit { win, place: 0, sig: 0, slot: None, plans });
                continue;
            };
            stats.win_count += 1;
            let t_sig = Instant::now();
            // **anchor = 本窗原点**：缓存里存的是窗口局部顶点，签名也必须按局部坐标算，
            // 否则"窗口移动 = 内容变化"⇒ 拖动/滚动每帧整窗重镶嵌。
            let anchor = self.win_origins.get(&win).copied().unwrap_or(Vec2::ZERO);
            let sig = self.hash_cmds(cmds.iter().flatten(), anchor);
            stats.sig_us += t_sig.elapsed().as_secs_f64() * 1e6;
            let hit = !self.debug_layout
                && self
                    .state
                    .window_quads
                    .get(&id)
                    .is_some_and(|(s, _)| *s == sig);
            let plans = if hit {
                stats.cache_hits += 1;
                // **move 出缓存**（不是克隆）：本帧提交完再原样放回去。
                self.state.window_quads.remove(&id).expect("just checked").1
            } else {
                stats.cache_misses += 1;
                self.build_plans(win, &cmds, quads, 0, stats)
            };
            units.push(SubmitUnit {
                win,
                place: 0,
                sig,
                slot: (!self.debug_layout).then_some(CacheSlot::Window(id)),
                plans,
            });
        }
    }

    /// **win=0：按顶层放置收集提交单元**（[`Self::z0_place_for_seq`] 的 `place`）。
    ///
    /// 每个放置一个单元 ⇒ 单元内 `place` 恒定 ⇒ 计划能整体缓存；跨单元的序由
    /// `(win, place)` 在 `finish` 里排（~15 个单元，代价可忽略）。
    fn collect_z0_units(
        &mut self,
        cmds: &[Vec<UiDraw>],
        units: &mut Vec<SubmitUnit>,
        quads: &mut QuadCollector,
        stats: &mut CacheStats,
    ) {
        let mut by_place: Vec<(u32, Vec<&UiDraw>)> = Vec::new();
        for d in cmds.iter().flatten() {
            let place = self.z0_place_for_seq(d.seq);
            match by_place.iter_mut().find(|(pp, _)| *pp == place) {
                Some((_, v)) => v.push(d),
                None => by_place.push((place, vec![d])),
            }
        }
        // **放置槽布局**（诊断，`RJ_ORDER_TRACE=slot`）：逐帧打印每个放置槽的 `seq` 区间。
        // 用来回答"同一个容器是不是被拆进了两个排序空间"。
        if std::env::var("RJ_ORDER_TRACE").is_ok_and(|v| v == "slot") {
            for (place, refs) in &by_place {
                let lo = refs.iter().map(|d| d.seq).min().unwrap_or(0);
                let hi = refs.iter().map(|d| d.seq).max().unwrap_or(0);
                eprintln!(
                    "z0slot[frame {}] seg={} place={place} n={} seq={lo}..{hi}",
                    self.state.frame,
                    self.segment,
                    refs.len()
                );
            }
            eprintln!(
                "z0starts[frame {}] seg={} starts={:?}",
                self.state.frame, self.segment, self.z0_ranges
            );
        }
        for (place, refs) in by_place {
            // **槽 = (段号, 放置序)**：`place` 是段内派生的计数（各段从 0 起），故必须带
            // 段前缀，否则不同段的同号槽共用一个缓存条目、每帧交替覆盖 → 永远 miss。
            // ⚠ `place` **逐帧重算**（不存进 `UiDraw`）：放置增删会让后面的 `place` 整体
            // 平移，键跟着变 ⇒ 直接判 miss 重建，绝不会"命中一个 `place` 已过期的条目"
            // 而把绘制序搞错一帧（那正是阶段 8 修掉的闪烁形态）。
            let slot = (self.segment, place);
            if !self.state.frame_state.z0_seen.contains(&slot) {
                self.state.frame_state.z0_seen.push(slot);
            }
            let t_sig = Instant::now();
            // 非窗口内容的 anchor = 0（其坐标本就是绝对屏幕坐标）。
            let sig = self.hash_cmds(refs.iter().copied(), Vec2::ZERO);
            stats.sig_us += t_sig.elapsed().as_secs_f64() * 1e6;
            let hit = !self.debug_layout
                && self.state.z0_quads.get(&slot).is_some_and(|(s, _)| *s == sig);
            let plans = if hit {
                stats.cache_hits += 1;
                // **move 出缓存**（不是克隆）：本帧提交完再原样放回去。
                self.state.z0_quads.remove(&slot).expect("just checked").1
            } else {
                stats.cache_misses += 1;
                // 该放置重建（克隆命令为 owned 单桶传入 `collect_cmds`）。
                let owned: Vec<UiDraw> = refs.iter().map(|d| (*d).clone()).collect();
                self.build_plans(
                    0,
                    std::slice::from_ref(&owned),
                    quads,
                    place,
                    stats,
                )
            };
            units.push(SubmitUnit {
                win: 0,
                place,
                sig,
                slot: (!self.debug_layout).then_some(CacheSlot::Z0(slot)),
                plans,
            });
        }
        // 陈旧放置的清理**不在这里**：本函数每段跑一次、只见到本段的槽，按段清会把同帧
        // 其它段刚写好的缓存删掉（那些槽于是每帧 miss）。这里只把本段见到的槽记进帧级
        // 暂存（`z0_seen`），由 `UiState::begin_frame` 在**下一帧开场**按完整集合清一次。
    }

    /// **现场建计划**（缓存未命中）：镶嵌（`collect_cmds`）+ 合并成该单元的
    /// [`BatchPlan`] 列表。`place` 只用于诊断归因（`RJ_CACHE_TRACE`）。
    fn build_plans(
        &mut self,
        win: u32,
        cmds: &[Vec<UiDraw>],
        quads: &mut QuadCollector,
        place: u32,
        stats: &mut CacheStats,
    ) -> Vec<BatchPlan> {
        let t_collect = Instant::now();
        self.collect_cmds(quads, win, cmds);
        stats.collect_us += t_collect.elapsed().as_secs_f64() * 1e6;
        self.trace_cache_miss(
            &format!("win {win} place {place}"),
            cmds.iter().map(|v| v.len()).sum(),
            t_collect,
        );
        self.merge_plans(quads, win, place)
    }

    /// **把收集器里属于 `(win, place)` 的几何合并成提交计划**（`finish` 的装配步骤）。
    ///
    /// 只在**未命中**时调用一次；命中路径直接用缓存里的计划（零拷贝）。步骤：
    /// 取出本单元的条目 → 按 `(elem, group, tex, clip)` 排序 → `segment_runs` 切段 →
    /// 每段几何拼成一段（单条段直接 move，多条段 `append` 一次）→ 附上元素序列表。
    ///
    /// ⚠ 段**不跨单元**合并：两个相邻单元即使 `(win, tex, clip)` 相同也各出一次批次
    /// ⇒ **真实 draw call 会小幅上升**（实测 40 → 44；`Render2D` 的相邻合批对 UI 批次
    /// 不生效，因为每个 `mesh_indexed` 带自己的矩阵下标）。取舍与理由见 [`BatchPlan`]。
    fn merge_plans(&mut self, quads: &mut QuadCollector, win: u32, place: u32) -> Vec<BatchPlan> {
        let mut ordered: Vec<CachedQuad> = std::mem::take(&mut self.state.scratch_ordered);
        ordered.clear();
        // 取出**本单元**的条目（`(win, place)` 唯一定位；其余条目留给别的单元）。
        let keys: Vec<crate::gpu_batch::QuadKey> = quads
            .quads
            .keys()
            .copied()
            .filter(|k| k.0 == win && k.1 == place)
            .collect();
        for k in keys {
            let geom = quads.quads.remove(&k).expect("just listed");
            let elems = quads.elems.remove(&k).unwrap_or_default();
            ordered.push((k.0, k.1, k.2, k.3, k.4, k.5.map(clip_rect), geom, elems));
        }
        ordered.sort_unstable_by_key(crate::gpu_batch::submit_sort_key);
        let runs = segment_runs(
            ordered.iter().map(|q| (q.0, q.4, q.5, q.6.verts.len())),
            MAX_UI_SEG_VERTS,
        );
        let mut plans: Vec<BatchPlan> = Vec::with_capacity(runs.len());
        let mut next = 0usize;
        // 复用同一个 `Geom` scratch（每个单元 1 次分配，而不是每段 1 次）。
        let mut scratch_seg = Geom::default();
        for run in runs {
            let geom = if run.quads == 1 {
                // 单条：直接 move（省一次 `append` 全量拷贝）——`ordered` 是 scratch。
                Geom {
                    verts: std::mem::take(&mut ordered[next].6.verts),
                    tris: std::mem::take(&mut ordered[next].6.tris),
                }
            } else {
                scratch_seg.verts.clear();
                scratch_seg.tris.clear();
                scratch_seg.verts.reserve(run.verts);
                for q in &ordered[next..next + run.quads] {
                    // 索引按已累计顶点数平移（`Geom::append`）——各段索引从 0 起。
                    scratch_seg.append(&q.6);
                }
                Geom {
                    verts: std::mem::take(&mut scratch_seg.verts),
                    tris: std::mem::take(&mut scratch_seg.tris),
                }
            };
            // 本段覆盖的**元素序集合**（去重；`UiBatchSource::elements` 的来源）。
            let mut elements: Vec<u32> = ordered[next..next + run.quads]
                .iter()
                .flat_map(|q| q.7.iter().copied())
                .collect();
            elements.sort_unstable();
            elements.dedup();
            next += run.quads;
            if geom.is_empty() {
                continue;
            }
            plans.push(BatchPlan {
                texture: run.texture,
                clip: run.clip,
                geom,
                elements,
            });
        }
        self.state.scratch_ordered = ordered;
        plans
    }

    /// **缓存未命中的逐窗 / 逐放置归因**（诊断，`RJ_CACHE_TRACE=1`）：窗口 id（或 win=0
    /// 放置序）、命令条数、本次重建耗时。用来回答"稳态下到底是哪几扇窗 / 哪几个 win=0
    /// 放置每帧重镶嵌、各占多少"——顶点缓存 miss 的代价远高于命中（整块重新镶嵌，含
    /// 投影的同心环）。
    ///
    /// 只在 miss 分支调用（命中路径零开销）；env 读取也只发生在未命中时。
    /// 历史战绩：稳态 `cache_miss` 恒等于槽数、`collect` ≈ 0.6ms 时，靠它一眼看出
    /// "miss 全在 win=0 槽、一扇窗都没 miss"，从而定位到槽键跨段冲突（见
    /// [`crate::UiState::z0_quads`]）。
    fn trace_cache_miss(&self, what: &str, cmds: usize, t_collect: Instant) {
        if std::env::var_os("RJ_CACHE_TRACE").is_none() {
            return;
        }
        eprintln!(
            "cache[frame {}] MISS {what} cmds={cmds} collect={:.1}us",
            self.state.frame,
            t_collect.elapsed().as_secs_f64() * 1e6
        );
    }

    /// 对一组命令做**全量内容签名**（`cmd_sig` 哈希）：窗口 / win=0 放置顶点缓存的 key。
    ///
    /// 签名里**并入字形图集的区域失效世代号**（[`rjw_text::Text::atlas_revision`]）：
    /// 本缓存烘的是**最终 UV**（字形 + WHITE 基础纹理都取自字形图集），图集一旦重排
    /// 或复用已逐出字形的槽位，旧 UV 就指向**别的像素**（"陈旧文字"/"背景消失"），
    /// 而命令内容没变 ⇒ 只靠命令哈希永远不失效。世代号只由图集整理（分配失败触发）
    /// 推进，不是每帧变化。
    ///
    /// 签名还**以主题行距 + 字重为前缀**（[`Theme::line_spacing`] / [`Theme::font_weight`]）：
    /// `DrawKind::Text` 的 `buf` 是排版结果、按设计**不参与哈希**（命令内容相同），而换行
    /// 文本的实际行高 / 整体高度取决于行距、字形与步进宽度取决于字重 ⇒ 不并入就会被
    /// "改了行距 / 字重但窗口几何仍命中旧缓存"卡住（固定矩形里的居中文本尤其明显）。
    /// 两者都是主题令牌、只在主题变更时改，代价可忽略。
    ///
    /// `anchor` = 该窗口 / 放置的原点（物理像素）：见 [`cmd_sig_hash`]——按局部坐标入签名，
    /// 于是**窗口移动 / 滚动不会让签名变化**（拖动窗口不再每帧整窗重镶嵌）。
    fn hash_cmds<'c>(
        &self,
        cmds: impl IntoIterator<Item = &'c UiDraw>,
        anchor: Vec2,
    ) -> u64 {
        use std::hash::Hasher;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        h.write_u32(self.theme.line_spacing.to_bits());
        h.write_u16(self.theme.font_weight.0);
        for d in cmds {
            self.cmd_sig(&mut h, d, anchor);
        }
        geom_cache_sig(h.finish(), self.text.atlas_revision())
    }

    /// **冲刷一个批次计划**：把 [`BatchPlan`] 作为 [`UiBatch`] 提交（单一窗口的
    /// `screen_fixed_tf` 变换 + 窗口级 FX tint/transform override + **batch scissor**）。
    ///
    /// `plan.clip` = 本段的环境裁剪层（窗口 / 放置**局部**坐标）⇒ 经
    /// [`crate::view::batch_scissor`] 映射成该批次的屏幕像素 scissor。
    ///
    /// ⚠ **几何按引用交给后端**（`UiBatch` 借用切片）：计划是缓存里的东西，提交期不能
    /// 把它的顶点搬走——这正是阶段 9 去掉"每帧克隆一遍几何"的关键。
    ///
    /// **解耦**：不直接调 `Render2D`，只产出数据；后端决定如何提交。
    /// 返回是否真的提交了（纹理缺失 / 空几何 ⇒ `false`，供段统计口径一致）。
    fn flush_seg(
        &mut self,
        backend: &mut dyn UiBackend,
        layer_base: f64,
        win: u32,
        plan: &BatchPlan,
    ) -> bool {
        if plan.geom.is_empty() {
            return false;
        }
        let Some(texture) = backend.texture(plan.texture) else {
            return false;
        };
        let anchor_px = self.win_origins.get(&win).copied().unwrap_or(Vec2::ZERO);
        let base_tf = screen_fixed_tf(anchor_px);
        // 窗口级 FX：tint（淡入淡出/染色）+ transform override（位移/缩放/旋转，
        // 绕**归一化锚点** `fx.anchor`）。`transform = IDENTITY` 时结果恒 = `base_tf`
        // （锚点不影响位置）。
        let fx = self
            .win_ids
            .get(&win)
            .and_then(|id| self.state.window_fx.get(id))
            .copied()
            .unwrap_or_default();
        let tf = match fx.transform {
            Some(t) => {
                // 绕锚点：`base_tf · (T_anc · t · T_anc⁻¹)`——先平移 -锚点（局部），
                // 应用 t，再平移回锚点，最后基础屏幕固定。组合验证：
                // `T_anc⁻¹.with_transform(&t)` = t·T_anc⁻¹；再 `.with_transform(&T_anc)`
                // = T_anc·t·T_anc⁻¹。t = IDENTITY 时 = T_anc·T_anc⁻¹ = IDENTITY ✓。
                // 遮挡表按**窗口 ID** 键，这里只有 z（win）⇒ 先经 `win_ids` 解出 ID。
                let size = self
                    .win_ids
                    .get(&win)
                    .and_then(|id| self.state.window_rects.get(id.as_str()))
                    .map(|r| Vec2::new(r.w, r.h))
                    .unwrap_or(Vec2::ZERO);
                let anchor_local = Vec2::new(fx.anchor.x * size.x, fx.anchor.y * size.y);
                let t_anc = Transform2D::IDENTITY.with_pos(anchor_local);
                let t_anc_inv = Transform2D::IDENTITY.with_pos(-anchor_local);
                let c = t_anc_inv.compose(&t).compose(&t_anc);
                c.compose(&base_tf)
            }
            None => base_tf,
        };
        // 诊断：记录本窗口**实际提交用的平移量**（`debug_dump` 的 `submit` 字段）——
        // 与 `origin` 比对即可判定"引擎状态 vs 视觉"是否一致。
        self.state.debug_submit.insert(win, tf.pos);
        // **batch scissor**：窗口/放置**局部**裁剪层 → 本批次的屏幕像素矩形（见
        // [`crate::view::batch_scissor`]）。`tf` 通常只是"平移到窗口原点"⇒ 结果 = 局部
        // 裁剪 + 窗口原点；窗口 FX（缩放/旋转）下走保守 AABB（宁可多画一点，绝不误裁）。
        // ⚠ 裁剪层必须与**缓存里的顶点同空间**（局部）：否则窗口一动，命中的旧缓存会把
        // scissor 留在旧位置（"scissor 不跟内容一起移动"，用户实测）。
        let clip = plan.clip.map(|c| crate::view::batch_scissor(c, &tf));
        if let Some(c) = clip {
            // 诊断 + 计数：本帧带 scissor 的批次数（`[perf] clip_batches`）。
            self.state.debug_clip.insert(win, c);
            self.state.frame_state.stats.clip_batches =
                self.state.frame_state.stats.clip_batches.saturating_add(1);
        }
        backend.submit(UiBatch {
            texture,
            vertices: &plan.geom.verts,
            indices: &plan.geom.tris,
            transform: tf,
            tint: fx.tint,
            layer: layer_base + win as f64 * 1.0,
            clip,
            source: UiBatchSource {
                window: win,
                elements: plan.elements.len() as u32,
                debug: false,
            },
        });
        true
    }

    /// **提交 Debug 叠加**（`finish` 末尾，全部 UI 内容之后）：合并 `debug_queue`
    /// （屏幕空间调试图元）与 `collect_cmds` 期间产生的布局描边（`quads.debug`），按 win
    /// 分组、白纹理、屏幕固定变换提交——不进窗口缓存、同 layer 后提交 → 恒覆盖在最上。
    fn submit_debug(
        &mut self,
        backend: &mut dyn UiBackend,
        quads: &mut QuadCollector,
        white_uid: u64,
        layer_base: f64,
    ) {
        // 1. 收集 debug_queue（[`Self::debug_line`] 等屏幕空间调试图元），与布局描边合并。
        let mut debug_groups: std::collections::HashMap<u32, Vec<UiDraw>> =
            std::collections::HashMap::new();
        for d in self.painter.q.debug_queue.drain(..) {
            debug_groups.entry(d.win).or_default().push(d);
        }
        let mut dwins: Vec<u32> = debug_groups.keys().copied().collect();
        dwins.sort_unstable();
        for win in dwins {
            let cmds = debug_groups.remove(&win).expect("group exists");
            self.collect_cmds(quads, win, &[cmds]);
        }
        // 2. 提交 quads.debug 顶点（白纹理 + 屏幕固定变换）。
        let mut dwins: Vec<u32> = quads.debug.keys().copied().collect();
        dwins.sort_unstable();
        for win in dwins {
            let geom = quads.debug.remove(&win).expect("debug group exists");
            if geom.is_empty() {
                continue;
            }
            let Some(texture) = backend.texture(white_uid) else {
                continue;
            };
            let anchor_px = self.win_origins.get(&win).copied().unwrap_or(Vec2::ZERO);
            let tf = screen_fixed_tf(anchor_px);
            self.state.debug_submit.insert(win, tf.pos);
            backend.submit(UiBatch {
                texture,
                vertices: &geom.verts,
                indices: &geom.tris,
                transform: tf,
                tint: Color::WHITE,
                layer: layer_base + win as f64 * 1.0,
                // 调试叠加**不受内容裁剪**（诊断图元要看得见）：scissor = None。
                clip: None,
                source: UiBatchSource { window: win, elements: 1, debug: true },
            });
        }
    }

    /// **光标定夺 + 帧复位**（`finish` 末尾）：按优先级选择系统光标（窗口拖拽 Arrow >
    /// 抓握 > 控件作者自定义 > 文本 I 型 > 可拖拽 Grab > 默认），无 UI 光标意图时抑制
    /// （保留应用自定义光标，如游戏准星；上一帧设过则清一次回 Default）；随后清空本帧
    /// 帧级状态（光标位 / depth / seq / cur_win / 窗口映射 / 焦点链等），下一帧从干净起点录制。
    fn finalize_cursor_and_reset(&mut self) {
        // **统计写回**（每帧一次）：各段累加值 + 帧号 + 整帧跨度（开场 → 收尾）。
        let mut stats = std::mem::take(&mut self.state.frame_state.stats);
        stats.frame = self.state.stats.frame.wrapping_add(1);
        stats.ui_frame_us = self.state.frame_state.frame_t0.elapsed().as_secs_f64() * 1e6;
        self.state.stats = stats;
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
        // ⚠ `window_rects` 的陈旧清理在**帧末**（这里），但数据源必须是
        // `frame_state.window_ids_seen`——**不能**用本视图的 `win_ids` / `win_origins`：
        // 它们已被上面 `finish()` 末尾的 `save_frame_state` 换走（空表），按它清会把整张
        // 遮挡表清光 ⇒ 遮挡判定退化成"只看本帧已录制的窗口"，上层窗口就挡不住背后窗口的
        // 控件（用户报的"被遮挡的控件仍被触发"）。这是那次事故的根因，别再改回去。
        let seen_wins = &self.state.frame_state.window_ids_seen;
        self.state
            .window_rects
            .retain(|id, _| seen_wins.iter().any(|w| w == id));
        self.painter.q.depth = 0;
        self.painter.q.seq = 0;
        self.painter.q.cur_win = 0;
        self.any_pressed = false;
        self.drag_panel = None;
        self.win_press_top = None;
        self.win_origins.clear();
        self.win_ids.clear();
        // `debug_submit` **故意不清**：`debug_dump()` 通常在本帧**录制期**调用（那时
        // 还没提交），保留上一次 `finish` 的提交平移量才能与 `origin` 对照。
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
        cmds: &[Vec<UiDraw>],
    ) {
        let anchor_px = self
            .win_origins
            .get(&win)
            .copied()
            .unwrap_or(Vec2::ZERO);
        let dbg = if self.debug_layout {
            // debug_layout 描边样式：读 Theme::debug（Copy 值先取出，
            // 避免与循环内 `&mut self` 调用（draw_text_quads）的借用冲突）。
            Some((self.theme.debug.layout_outline, self.theme.debug.layout_outline_width))
        } else {
            None
        };
        // depth 桶展平（桶序 = depth 升序，桶内录制序）：免排序下仍满足提交序。
        for d in cmds.iter().flatten() {
            // **当前放置序**：win=0 的排序空间（窗口内恒 0——窗口本身就是一个排序空间）。
            // 见 `Self::z0_ranges`：没有它，`elem = 0` 的容器装饰会被别的 win=0 放置穿透。
            quads.cur_place = if win == 0 { self.z0_place_for_seq(d.seq) } else { 0 };
            // 当前元素序：push 方法按其分组（控件级提交顺序——见 QuadCollector）。
            quads.cur_elem = d.elem;
            // **当前裁剪层的绝对矩形**（`d.clip` 已是绝对坐标）：**剔除**用它。
            let clip_abs = d.clip.map(|c| snap_rect(&c));
            // **环境裁剪层**（**窗口局部**；内容已随容器平移成绝对坐标后减本窗原点）。
            //
            // ⚠ 这里**不再切割几何**（旧实现逐命令 `clipped(..)`）：环境裁剪改由 batch
            // scissor 在 GPU 侧执行（见 `UiBatch::clip` / `view::batch_scissor`）。收益：
            // ① 圆角 / 环带不再被切平；② 省掉每命令的矩形求交与渐变重采样；
            // ③ 投影不再需要"部分可见就整块跳过"的特例（像素级裁边更干净）。
            //
            // ⚠ **必须存局部坐标**：它与缓存里的顶点同空间 ⇒ 窗口移动时缓存键与裁剪
            // 一起"跟着走"（存绝对值会让"窗口移动"污染分组键，并在缓存命中时拿到
            // 过期矩形——实测拖动窗口后 scissor 偏了一个位移量）。
            quads.cur_clip = clip_abs.map(|c| local_of(c, anchor_px));
            // **兜底剔除**：完全在裁剪层之外的命令**不镶嵌、不入段**。
            //
            // 主剔除在**分配处**（`Ui::culled` / `Response::culled`：控件自己直接 return
            // ——省的是它内部的全部命令）；这里兜住两类"分配处看不见的情况"：
            // ① 自绘装饰溢出到裁剪层外（控件本体可见、装饰不可见）；
            // ② 没检查 `culled` 的控件（含第三方）。
            // ⚠ **只剔"全外"**：**部分**重叠照整条画，越界像素交给 batch scissor。
            // ⚠ **空间**：`d.rect` 此刻已是**绝对**坐标（容器弹出时平移过），所以要与
            // **绝对**的 `clip_abs` 比——`quads.cur_clip` 是窗口**局部**的（缓存键用），
            // 拿它比会把内容整块误剔（实测：严格窗口内容全没了、子菜单面板不见了）。
            let cull_rect = match &d.kind {
                // 投影向外溢出本体：按**外沿**判可见性（否则贴边窗口的投影被误剔）。
                DrawKind::Shadow { blur, offset, .. } => {
                    let p = snap_rect(&d.rect);
                    Rect::new(
                        p.x - *blur + offset.x,
                        p.y - *blur + offset.y,
                        p.w + (*blur + offset.x.abs()) * 2.0,
                        p.h + (*blur + offset.y.abs()) * 2.0,
                    )
                }
                // ⚠ 文本的 `rect` 是**排版锚点矩形**（宽/高 = 容器盒），内容却随滚动平移
                // ⇒ 必须用**可见窗口**（软裁剪层，锚在盒子上）判，见 `text_visible_rect`。
                // 用 `rect` 会把"滚过头但明明可见"的整条文本剔掉（用户报的 BUG）。
                DrawKind::Text { clip, .. } => {
                    text_visible_rect(snap_rect(&d.rect), clip.map(|c| snap_rect(&c)))
                }
                _ => snap_rect(&d.rect),
            };
            if fully_outside(cull_rect, clip_abs) {
                if matches!(d.kind, DrawKind::Text { .. }) {
                    let acc = &mut self.state.frame_state.stats;
                    acc.culled_text = acc.culled_text.saturating_add(1);
                }
                continue;
            }
            match &d.kind {
                DrawKind::Solid(color) => {
                    if d.rect.w > 0.0 && d.rect.h > 0.0 {
                        let pr = snap_rect(&d.rect);
                        if pr.w > 0.0 && pr.h > 0.0 {
                            quads.push_white(win, local_of(pr, anchor_px), *color);
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                    }
                }
                DrawKind::RoundedRect { corners, radius } => {
                    let pr = snap_rect(&d.rect);
                    let local = local_of(pr, anchor_px);
                    if local.w > 0.0 && local.h > 0.0 {
                            // **无纹理、无着色器改动**：CPU 把圆角矩形镶嵌成三角形
                            // （硬体 + 1 物理像素羽化带，见 `crate::tess`）。
                            // 半径不做取整 / 9-patch clamp——镶嵌器接受任意半径并把
                            // 超出半高的半径 clamp 成胶囊；羽化带随控件尺寸自动收紧。
                            //
                            // 四角可各异 ⇒「圆角 + 渐变」自然成立。裁剪时按四角在
                            // **原矩形**中的相对位置重采样，保证渐变锚定不被裁剪平移。
                            let grad = Gradient::corners(
                                corners[0], corners[1], corners[2], corners[3],
                            );
                            let c = resample_gradient_local(grad, local, pr, anchor_px);
                            let table = self.state.tess.table();
                            // 采样 UV 由 `push_rounded` 填成白纹理 region 中心
                            // （写错 `(0,0)` 会静默采到字形页左上角的字形像素）。
                            quads.push_rounded(win, &table, local, *radius, self.theme.feather, c);
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                }
                DrawKind::Shadow { color, blur, offset, radius } => {
                    // **顶点色软阴影**（无纹理 / 无着色器 / 不增 draw call）。
                    // `rect` = 本体矩形（内轮廓恒在本体边缘，无"等浓度平台"）。
                    let pr = snap_rect(&d.rect);
                    // ⚠ **不再需要"部分可见就整块跳过"**：环境裁剪由 scissor 在像素级执行，
                    // 几何不会被形变（旧实现切割矩形会让投影的内轮廓错位成一条暗带，
                    // 于是只能在"被裁掉一部分"时整块放弃）。被裁的部分由 scissor 裁掉即可。
                    let local = local_of(pr, anchor_px);
                    if local.w > 0.0 && local.h > 0.0 && *blur > 0.0 {
                        let table = self.state.tess.table();
                        quads.push_rounded_shadow(
                            win, &table, local, *radius, *blur, *offset, *color,
                        );
                    }
                }
                DrawKind::Rect(gradient) => {
                    let pr = snap_rect(&d.rect);
                    let local = local_of(pr, anchor_px);
                    if local.w > 0.0 && local.h > 0.0 {
                            // **无纹理**：四角颜色直接进顶点色（光栅化器双线性插值）。
                            // 四角色按它在**原矩形**中的相对位置采样，保证渐变锚定不变。
                            let c = resample_gradient_local(*gradient, local, pr, anchor_px);
                            quads.push_white_quad(win, local, c);
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                }
                DrawKind::Border { color, width, radius } => {
                    let pr = snap_rect(&d.rect);
                    if pr.w > 0.0 && pr.h > 0.0 {
                        let local = local_of(pr, anchor_px);
                        if !radius.is_zero() {
                            // 圆角环带：只画一次边界，圆角处不会像"外圈实心 + 内圈实心"
                            // 那样把抗锯齿边缘混合两次。
                            //
                            // ⚠ **不要**因为"矩形被裁剪过"就退回直角四边条：窗口被拖到
                            // 视口边缘（或父裁剪区内侧）时被裁掉一部分，退回直角会让
                            // **整个窗口的边框瞬间变方**（"拖动变方"）。裁剪后的矩形交给
                            // 环带自己处理即可——`push_rounded_ring` 内部会
                            // `CornerRadius::fit(w, h)` 把半径夹到放得下，内轮廓塌缩时
                            // 也会退化成一块实心圆角矩形。
                            let table = self.state.tess.table();
                            quads.push_rounded_ring(
                                win,
                                &table,
                                local,
                                *radius,
                                *width,
                                self.theme.feather,
                                *color,
                            );
                        } else {
                            for br in border_rects(&local, (*width).round()) {
                                if br.w > 0.0 && br.h > 0.0 {
                                    quads.push_white(win, br, *color);
                                }
                            }
                        }
                        debug_layout_outline(quads, win, anchor_px, pr, dbg);
                    }
                }
                DrawKind::Icon { icon, color } => {
                    // 矢量图标：单位方框内的凸分片映射到 `rect`（窗口局部），
                    // 并按 `Theme::feather` 做边缘羽化——与圆角矩形同一套 AA 机制。
                    let pr = snap_rect(&d.rect);
                    let local = local_of(pr, anchor_px);
                    if local.w > 0.0 && local.h > 0.0 {
                            // **等比**：图标分片画在 `[0,1]²` 的方形域里，把 `rect` 直接映射过去
                            // 会在非方形框里被拉扁（`row` 内 `force_h_all` 就会把 18×26 的框
                            // 交给这里）。故取 `min(w,h)` 的**居中方块**——图标永不形变。
                            quads.push_icon(
                                win,
                                centered_square(local),
                                *icon,
                                *color,
                                self.theme.feather,
                            );
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
                    );
                    let pr = snap_rect(&d.rect);
                    debug_layout_outline(quads, win, anchor_px, pr, dbg);
                }
                DrawKind::Image(bg) => {
                    // 背景图：与实心背景同一条镶嵌路径（CPU 直出三角形 + 羽化），
                    // 只多一个"逐顶点 UV 的仿射映射"（见 `QuadCollector::push_image`）。
                    let pr = snap_rect(&d.rect);
                    let local = local_of(pr, anchor_px);
                    if local.w > 0.0 && local.h > 0.0 {
                            let table = self.state.tess.table();
                            quads.push_image(win, &table, local, *bg, self.theme.feather);
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                }
                DrawKind::Caret { color, width } => {
                    let r = Rect::new(d.rect.x, d.rect.y, *width, d.rect.h);
                    let pr = snap_rect(&r);
                    if pr.w > 0.0 && pr.h > 0.0 {
                            quads.push_white(win, local_of(pr, anchor_px), *color);
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                }
                DrawKind::Debug { color, shape } => {
                    // 屏幕空间调试图元：逻辑像素 → 物理像素线段，转窗口局部写入 debug 叠加。
                    for ([a, b], w) in debug_shape_segments(shape) {
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
    fn cmd_sig(&self, h: &mut std::collections::hash_map::DefaultHasher, d: &UiDraw, anchor: Vec2) {
        cmd_sig_hash(h, d, anchor);
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
        if let Some(fid) = &self.state.focused
            && !chain.iter().any(|e| e.id == *fid) {
                self.state.focused = None;
            }
        // 移动：Tab（+1）/ Shift+Tab（-1）/ Down（+1）/ Up（-1）。
        // ⚠ IME 组合中（preedit 非空或上帧在组合）**禁止方向键/Tab 移动焦点**——
        // 中文输入法用 ↑/↓ 切换候选、Enter 上屏，焦点被移走会立刻打断输入（文本框"失效"）。
        let composing = self
            .keyboard
            .ime_preedit()
            .is_some_and(|p| !p.is_empty())
            || self.state.ime_composing;
        let shift = self.keyboard.key(KeyCode::ShiftLeft).pressed()
            || self.keyboard.key(KeyCode::ShiftRight).pressed();
        // **文本输入框持有焦点时，↑/↓ 由输入框自身处理**（多行跨视觉行移动光标、
        // 单行无操作）——全局焦点遍历只接管 Tab / Shift+Tab；否则按 ↑/↓ 会把焦点
        // 跳到别的控件（多行文本框内"上下键跳走"的 bug）。
        let focus_is_text = chain
            .iter()
            .find(|e| {
                self.state
                    .focused
                    .as_ref()
                    .is_some_and(|f| f.as_str() == e.id.as_str())
            })
            .is_some_and(|e| e.kind == FocusKind::TextInput);
        let dir: i32 = if composing {
            0
        } else if self.keyboard.key(KeyCode::Tab).down_edge() {
            if shift { -1 } else { 1 }
        } else if self.keyboard.key(KeyCode::ArrowDown).down_edge() && !focus_is_text {
            1
        } else if self.keyboard.key(KeyCode::ArrowUp).down_edge() && !focus_is_text {
            -1
        } else {
            0
        };
        if dir != 0 {
            let next = focus_step(&chain, self.state.focused.as_ref(), dir);
            // 同步焦点控件的**类型**（文本焦点判定用）：查链拿到该控件的 `FocusKind`。
            self.state.focused_kind = next
                .as_ref()
                .and_then(|id| chain.iter().find(|e| e.id == *id).map(|e| e.kind));
            self.state.focused = next;
        }
        // Esc：优先收起下拉框，否则取消焦点。
        if self.keyboard.key(KeyCode::Escape).down_edge() {
            if self.state.combo_open.is_some() {
                self.state.combo_open = None;
            } else if self.state.focused.is_some() {
                self.state.focused = None;
                self.state.focused_kind = None;
            }
        }
        // 焦点描边：对当前焦点控件画一圈 Border。
        // ⚠ 走 **`debug_queue`**（`submit_debug` 路径：不进窗口顶点缓存、恒覆盖在最上）：
        // 一帧多段后帧收尾视图的 `seq` 从 0 重开，若沿用"`elem = seq + 1` 最大 ⇒ 画在
        // 窗口内容之上"的老写法，描边会被压到窗口内容下面（`elem` 比各段的都小）。
        if let Some(fid) = &self.state.focused {
            let Some(entry) = chain.iter().find(|e| e.id == *fid) else {
                return;
            };
            // 先拷贝字段，结束对 chain 的借用（随后需要 &mut self）。
            let (win, depth, rect, clip) = (entry.win, entry.depth, entry.rect, entry.clip);
            let focus = self.theme.focus.clone();
            let elem = self.painter.q.seq + 1;
            let seq = self.next_seq();
            self.painter.q.debug_queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect,
                clip,
                kind: DrawKind::Border {
                    color: focus.color,
                    width: focus.width,
                    radius: CornerRadius::default(),
                },
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
    ) {
        if rect.w <= 0.0 || rect.h <= 0.0 || text.is_empty() {
            return;
        }
        // 全部换算物理像素（锚点 / 裁剪区），与屏幕固定变换 1:1 匹配；
        // 排版缓冲：控件自持（输入框）或按需缓存（静态标签）。
        let pr = snap_rect(rect);
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
        let win_anchor_world = anchor_px;
        let buf = match buf {
            Some(b) => b,
            None => self.cache_buffer(text, size, family),
        };
        // 排版几何（UI 稳定集成面）：内容尺寸 / 首行行盒顶 / 图集页尺寸。
        // 取代旧 `Text::render_from(&buf)` + `tr.content_size()/lines()/page_size()`。
        let geo = self.text.geometry(&buf);
        // 垂直定位按**行盒**（行高），而非字形墨迹内容：
        // 以首行行顶（相对文本视觉原点）为参考，使行盒中心对准锚点（矩形垂直中心）。
        // 字形在行盒内按基线排布——矮小写字母（如 "a"，无 descender）落在基线上，
        // 不再因"以墨迹顶为参考"（旧实现把 `[视觉原点, 视觉原点+行高]` 当块居中，
        // 行盒整体上移约一个 top-bearing）而浮在行盒上部偏上显示。
        let content = geo.content_size;
        // `first_line_top` 为**整数**（rjw_text 收集期已对行顶取整）。
        let first_line_top = geo.first_line_top;
        // **整数加法不变量**：`block_tl = anchor + off` 的两侧均为整数——
        // 锚点（上方逐项取整）、`text_block_offset`（content / first_line_top 均为
        // 整数，内部只有整数加减与边界 `round`）。0.5px 小数（居中奇数宽 / 行盒偏移）
        // 在 `round` 边界被一次性消化，不会流入加法链 → 无误差累加；
        // 字形四边形角点 = block_tl + 整数字形偏移 = 整数屏幕像素 → 采样精确落在
        // 纹素中心，消除 1:1 图集双线性采样的亚像素模糊。
        let block_tl = anchor + text_block_offset(align, valign, content, first_line_top);
        let tf = screen_fixed_tf(block_tl);
        // 裁剪区：相对命令矩形（`merged`，逻辑）→ **窗口局部**。
        // ⚠ 相对基准是命令矩形物理 `pr`，**不是**文本块原点 `block_tl`：
        // `merged` 相对 `d.rect`，而字形窗口局部坐标以 `anchor_px` 为原点——
        // clip 窗口局部 = merged×scale + (pr - anchor_px)。用 block_tl 会整体错位
        // (block_tl - pr)（Top 对齐 / 多行时垂直偏差数像素）→ 滚动后文本不消失/错位。
        let clip_local: Option<Rect> = clip.map(|c| {
            let pc = snap_rect(&c);
            Rect::new(
                pc.x + pr.x - anchor_px.x,
                pc.y + pr.y - anchor_px.y,
                pc.w,
                pc.h,
            )
        });
        // 像素级裁剪完全由下方逐字形求交完成（完全在外 → 剔除；部分相交 → "半消失"）。
        // 不再使用 rjw_text 的 clip/cull（其坐标系相对字形 top_left，语义易错位）。
        let page_size = geo.page_size;
        let inv_page = 1.0 / page_size;
        let ca: [f32; 4] = color.into();
        // 唯一文本链：`label_from`（复用已排版缓冲，不重新整形）→ `transform(tf)` → 逐字形回调。
        // 旧写法 `render_from(&buf).origin(ZERO).transform(tf).color(color).draw_with(..)`
        // 在此机械适配（`origin(ZERO)` = 默认定位，`color` 由下方顶点色统一施加）。
        self.text.label_from(&buf).transform(tf).draw_with(|g| {
            let region = g.region();
            let transform = g.transform();
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

/// 拖拽激活所需的最小**物理像素**位移：按下后鼠标位移 ≥ 此值才视为“拖拽”。
///
/// 纯点击（无位移）不激活拖拽 → 窗口 / 可拖拽面板内的子控件（按钮 / 勾选框 /
/// 输入框等）**正常响应点击**；真正拖动中才抑制子控件交互（防止误触）。
const DRAG_ACTIVATE_PX: f32 = 3.0;

/// **几何缓存签名** = 命令内容哈希 ⊕ 字形图集区域失效世代号
/// （[`rjw_text::Text::atlas_revision`](rjw_text::Text::atlas_revision)）。
///
/// 窗口 / win=0 子槽的顶点缓存烘的是**最终 UV**（字形 + WHITE 基础纹理都取自字形
/// 图集）。图集一旦**重排**（搬动区域）或**复用已逐出条目的槽位**，旧 UV 就指向
/// 别的像素（"陈旧文字" / "背景消失"），而命令内容没变 ⇒ 只靠命令哈希**永远不失效**。
/// 故把世代号并入缓存键：图集一变，全部几何缓存自动重建（重建期会重新解析字形区域）。
#[inline]
fn geom_cache_sig(cmd_hash: u64, atlas_revision: u64) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write_u64(cmd_hash);
    h.write_u64(atlas_revision);
    h.finish()
}

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

/// **窗口 / 可拖拽面板的位置求解**（纯函数，`window_impl` / `panel_impl` 共用）。
///
/// - `origin`：责任链（脚本 → 拖拽状态 → 传入 pos）解析出的本帧基准位置；
/// - `mouse_screen`：本帧鼠标物理坐标；
/// - `hit`：鼠标是否在本体（由**上一帧结算尺寸**构造的矩形判定，见调用方）。
///
/// 语义：
/// 1. 按下帧（`down_edge && hit`）**先无条件**建立拖拽基准
///    `(press_panel = origin, press_mouse = 本帧鼠标)`；内容录制后若发现子控件
///    声明了本次按下（`press_claimed`），调用方须 [`clear_drag_base`] 清除
///    ——这样"从输入框上拖拽 = 选择文本"与"窗口从空白处拖动"两者都对，且
///    基准判定不依赖内容录制（`display_pos` 得以在录制**前**求出）。
/// 2. `active` = 拖动标记（[`crate::hit::update_drag`]）+ **有基准** +
///    位移 ≥ [`DRAG_ACTIVATE_PX`]（纯点击不拖拽）。
/// 3. 位置**只由按下帧基准 + 当前鼠标位移决定**，与上一帧位置无关——这是
///    `abs_base` 能与几何（`translate(pos)`）当帧一致、拖拽不落后一帧的前提。
fn resolve_drag(
    ws: &mut WidgetState,
    hit: bool,
    btn: KeyState,
    mouse_screen: Vec2,
    origin: Vec2,
) -> (bool, Vec2) {
    let dragging = update_drag(ws, hit, btn);
    if btn.down_edge() && hit {
        // 物理像素粒度基准（取整消除鼠标静止噪声；DPI 1.5 下也不会"移动 1.5px 才动"）。
        ws.press_panel = Some(origin);
        ws.press_mouse = Some(mouse_screen.round());
    }
    let active =
        dragging && ws.press_mouse.is_some() && drag_moved(mouse_screen.round(), ws.press_mouse);
    let pos = if active {
        let pp = ws.press_panel.unwrap_or(origin);
        let pm = ws.press_mouse.unwrap_or(mouse_screen);
        // 物理像素增量（round：对噪声滞回，静止时不变）→ 物理位移
        pp + (mouse_screen - pm).round()
    } else {
        origin
    };
    (active, pos)
}

/// **清除拖拽基准**：本帧按下被窗口 / 面板内的子控件声明（`press_claimed`——
/// 输入框选择、滑块调值、滚动条拖拽），窗口 / 面板**不得**跟随移动。
fn clear_drag_base(ws: &mut WidgetState) {
    ws.press_panel = None;
    ws.press_mouse = None;
    ws.press_size = None;
}

/// **滚动条几何**（纯函数，物理像素）：返回 `(条带, 可见轨道)`。
///
/// - 条带 = 右缘 [`SCROLLBAR_W`] 宽的全高矩形：**占位 + 命中 / 翻页热区**（比
///   可见滑块宽 ⇒ 抓取更容易，也避免了"滑块太细点不中"）；
/// - 可见轨道 = 条带**居中**的 [`SCROLLBAR_BAR_W`] 宽胶囊（两侧各留
///   `(SCROLLBAR_W − SCROLLBAR_BAR_W) / 2` 的空白），上下各留 [`SCROLLBAR_MARGIN`]
///   （胶囊两端不贴可视区边缘）。
pub(crate) fn scrollbar_rects(view: &Rect, view_h: f32) -> (Rect, Rect) {
    let strip = Rect::new(view.x + view.w - SCROLLBAR_W, view.y, SCROLLBAR_W, view_h);
    let track = Rect::new(
        strip.x + (SCROLLBAR_W - SCROLLBAR_BAR_W) * 0.5,
        strip.y + SCROLLBAR_MARGIN,
        SCROLLBAR_BAR_W,
        (view_h - SCROLLBAR_MARGIN * 2.0).max(1.0),
    );
    (strip, track)
}

/// **滚动条滑块几何**（纯函数，物理像素，全部取整 ⇒ 整像素步进、不抖）。
///
/// 输入：`track_h` 轨道高、`view_h` / `content_h` 可视高 / 内容高、`offset_px` /
/// `max_off_px` 当前 / 最大滚动偏移。返回 `(thumb_h, travel, thumb_y)`：
/// 滑块长、滑块行程（= 轨道高 − 滑块长）、滑块顶端相对**轨道顶端**的偏移。
///
/// 不变量：内容装得下（`ratio ≥ 1`）⇒ 滑块铺满轨道、行程 0；`offset_px ==
/// max_off_px` ⇒ `thumb_y == travel`（滑块底端与轨道底端对齐）——滑块与内容
/// 刚性对应，这正是"滚到底"时视觉上真的贴底的原因。
pub(crate) fn scroll_thumb(
    track_h: f32,
    view_h: f32,
    content_h: f32,
    offset_px: f32,
    max_off_px: f32,
) -> (f32, f32, f32) {
    let track_h = track_h.round().max(1.0);
    let ratio = if content_h > 0.0 {
        (view_h / content_h).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let thumb_h = (track_h * ratio)
        .max(SCROLLBAR_MIN_THUMB.min(track_h))
        .min(track_h)
        .round();
    let travel = (track_h - thumb_h).max(0.0);
    let thumb_y = if max_off_px > 1e-6 && travel > 1.0 {
        (offset_px / max_off_px * travel).round().clamp(0.0, travel)
    } else {
        0.0
    };
    (thumb_h, travel, thumb_y)
}

/// **滚动条滑块位置 → 滚动偏移**（[`scroll_thumb`] 的逆，拖拽用；物理像素取整）。
#[inline]
pub(crate) fn scroll_offset_for_thumb(thumb_y: f32, travel: f32, max_off_px: f32) -> f32 {
    if travel > 1.0 {
        (thumb_y.clamp(0.0, travel) / travel * max_off_px)
            .round()
            .clamp(0.0, max_off_px)
    } else {
        0.0
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

    /// 在容器内**占光标**放置 [`crate::widgets::Widget`] 控件（尺寸由控件自己在
    /// `ui()` 里申请，见 [`Ui::allocate`]）。
    fn add(&mut self, w: impl crate::widgets::Widget) -> crate::widgets::Response {
        self.ui_mut().add(w)
    }

    /// **绝对定位**放置 [`crate::widgets::Widget`] 控件（`pos` 相对当前容器内容原点；
    /// 不占光标）。
    fn add_at(&mut self, pos: impl Into<Position>, w: impl crate::widgets::Widget) -> crate::widgets::Response {
        self.ui_mut().add_at(pos, w)
    }

    /// 在容器内建**窗口**（可重叠 + 置顶 + 可拖拽；[`WindowBuilder`] 链，`.show(f)` 执行）。
    fn window<'s>(&'s mut self, id: &'s str) -> WindowBuilder<'s, 'a> {
        self.ui_mut().window(id)
    }

    /// 在容器内建**面板**（背景 + 边框；[`PanelBuilder`] 链，`.show(f)` 执行）。
    fn panel<'s>(&'s mut self) -> PanelBuilder<'s, 'a> {
        self.ui_mut().panel()
    }

    /// 标签（占光标，内容自然尺寸；默认 `LimitedInParent`——在父级可用宽内
    /// **自动换行**，Resizable 窗口缩窄后不溢出）。
    fn label(&mut self, text: &str) -> Vec2 {
        let ui = self.ui_mut();
        // 尺寸由控件申请；`Response::rect` 就是它最终占的矩形（见 `Widget::ui`）。
        let resp = crate::widgets::Widget::ui(crate::widgets::Label::new(text), ui);
        resp.rect.size()
    }

    /// 绝对定位标签（`pos` 相对当前容器内容原点）。
    fn label_at(&mut self, pos: impl Into<Position>, text: &str) -> Vec2 {
        self.ui_mut().label_at(pos, text)
    }

    /// **矢量图标**（占光标）：容器内按当前游标放置，方块 `size`（等比缩放笔画）。
    /// 与 [`crate::Ui::icon`] 同语义——`row` 内连续调用即得一条工具栏。
    fn icon(&mut self, size: impl Into<Size<Vec2>>, icon: Icon, color: Color) {
        self.ui_mut().icon(size, icon, color)
    }

    /// **矢量图标**（绝对定位；`pos` 相对当前容器内容原点）。
    fn icon_at(&mut self, pos: impl Into<Position>, size: impl Into<Size<Vec2>>, icon: Icon, color: Color) {
        self.ui_mut().icon_at(pos, size, icon, color)
    }

    /// **背景图**（占光标）：容器内按当前游标放置，尺寸 `size`。
    fn image(&mut self, size: impl Into<Size<Vec2>>, bg: ImageBg) {
        self.ui_mut().image(size, bg)
    }

    /// **背景图**（绝对定位；`pos` 相对当前容器内容原点）。
    fn image_at(&mut self, pos: impl Into<Position>, size: impl Into<Size<Vec2>>, bg: ImageBg) {
        self.ui_mut().image_at(pos, size, bg)
    }

    /// **自动换行标签**（占光标）：`max_w` 逻辑像素内按词/字换行；
    /// 返回自然尺寸（宽 = min(自然宽, max_w)，高 = 行数 × 行高）。
    /// `max_w <= 0` = 不换行（同 `label`）。
    fn label_wrap(&mut self, max_w: f32, text: &str) -> Vec2 {
        let ui = self.ui_mut();
        let style = ui.theme.label.clone();
        let size = ui.text_size_wrap(text, style.font_size, style.font_family.as_deref(), max_w);
        let rect = ui.child_rect(size.x, size.y, Child::Expand);
        ui.painter().text(
            rect,
            text,
            style.font_size,
            style.color,
            style.font_family.clone(),
            TextAlign::from(style.align),
            TextVAlign::Center,
            None,
            None,
        );
        size
    }

    /// **水平行容器**（占光标）：子项按 [`PackSide::Left`] 水平堆叠
    /// （`{Label} {Input} {Button}` 排列），整体在父容器（垂直 pack 等）中**占一行**：
    /// 宽 = 子项结算、撑大父级。
    ///
    /// **行高**（= 默认形态的 `row_builder`）：
    /// - **单行子项**（[`crate::widgets::SizeClass::SingleLine`]，默认）被**钉到标准行高**
    ///   [`Theme::row_h`]，各自内容垂直居中 → 文字中心线对齐（Label 不偏上）；
    /// - **多行子项**（`TextEditor::multiline()`）以标准行高为**下限**，可以**撑高整行**；
    /// - 子项**左上角对齐、沿 X 推进**；行高随最高的子项长。
    ///
    /// 要自定义行高上下限 / 间距 / 内边距用 [`Self::row_builder`]。
    fn row(&mut self, f: impl FnOnce(&mut Pack<'_, '_>)) -> Vec2 {
        self.row_builder().show(f)
    }

    /// **水平行 Builder**（占光标）：`row(f)` 的可配置形态——行高下限 / 上限 / 子项间距 /
    /// 行内边距，最后 `.show(|r| ..)` 执行并返回结算尺寸（见 [`crate::RowBuilder`]）。
    ///
    /// ```ignore
    /// // ⚠ `ignore`：`rjw_ui` 的 doctest 里拿不到"容器"——应用侧是
    /// // `Frame::ui()` 给的 `UiSession`（它实现本 trait），`Ui` 自身不实现 `UiAdd`
    /// // （所以这里不能用 `&mut Ui` 调 trait 方法）。行为验收见 `--sim-ta-resize`。
    /// ui.row_builder().height(60.0).show(|r| { r.label("高一行"); });
    /// ui.row_builder().min_h(60.0).max_h(200.0).pad(6.0).gap(12.0).show(|r| { r.label("…"); });
    /// ```
    fn row_builder(&mut self) -> RowBuilder<'_, 'a> {
        RowBuilder { ui: self.ui_mut(), min_h: None, max_h: None, gap: None, pad: 0.0 }
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
        let rect = ui.child_rect(w, h, Child::Expand);
        ui.divider_at(Position::Physical(Vec2::new(rect.x, rect.y)), Size::Physical(w));
    }

    /// **多行文本输入框**（占光标；默认约 200×90，可 `text_area_at` 显式尺寸）。
    /// Enter 换行、↑/↓ 跨行、Home/End 行首尾；自动换行 + 垂直滚动；选择/复制/
    /// 粘贴/剪切（Ctrl+C/V/X）；IME 支持。返回 `()`（内容写回 `value`）。
    ///
    /// 属性（尺寸 / 缩放柄 / 样式 / 不自动换行）请用责任链
    /// [`crate::widgets::TextEditor`]（本方法 = `ui.add(TextEditor::new(id, value).multiline())`）。
    fn text_area(&mut self, id: &str, value: &mut String) {
        let ui = self.ui_mut();
        crate::widgets::Widget::ui(
            crate::widgets::TextEditor::new(id, value).multiline(),
            ui,
        );
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
        crate::widgets::Widget::ui(
            crate::widgets::TextEditor::new(id, value).multiline().no_wrap(),
            ui,
        );
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
        let rect = ui.child_rect(w, h, Child::Expand);
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
        let rect = ui.child_rect(size.x, size.y, Child::Expand);
        // 直接走 styled 变体：避免 button_at 内再 clone 一次样式。
        ui.button_at_styled(id, rect, label, &style)
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
        let rect = ui.child_rect(size.x, size.y, Child::Expand);
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
        let rect = ui.child_rect(size.x, size.y, Child::Expand);
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
    /// `id` 灵活指定（[`crate::widgets::WidgetId`]，经 [`From`] 转换）：
    /// - `None` → 以 `label` 文本为 ID（同容器内标签唯一时最简）；
    /// - `Some("id")` / `"id"` → 显式字符串 ID；
    /// - `42u64` → 数字 ID（如列表行索引 `i as u64`）。
    ///
    /// 用法：`w.checkbox_mut(None, "窗口 A 选项", &mut self.win_a_checked);`
    fn checkbox_mut<'x>(
        &mut self,
        id: impl Into<crate::widgets::WidgetId<'x>>,
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
        let rect = ui.child_rect(size.x, size.y, Child::Expand);
        ui.radio_at(id, group, rect, label)
    }

    /// 显式尺寸单选（逃生舱）。
    fn radio_at(&mut self, id: &str, group: &str, rect: Rect, label: &str) -> CheckboxState {
        self.ui_mut().radio_at(id, group, rect, label)
    }

    /// 文本输入框（内容写入 `value`；自动尺寸：高度固定，宽度取样式最小宽）。
    ///
    /// 等价 `ui.add(TextEditor::new(id, value))`——宽 / 高 / 缩放柄 / 样式请用
    /// [`crate::widgets::TextEditor`] 责任链。
    fn text_input(&mut self, id: &str, value: &mut String) {
        let ui = self.ui_mut();
        crate::widgets::Widget::ui(crate::widgets::TextEditor::new(id, value), ui);
    }

    /// 显式尺寸文本输入框（逃生舱）。
    fn text_input_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        self.ui_mut().text_input_at(id, rect, value);
    }

    /// 嵌套面板（`pos` 相对当前容器内容原点；不占光标）。
    fn panel_at(&mut self, pos: impl Into<Position>, f: impl FnOnce(&mut Panel<'_, '_>)) -> Vec2 {
        self.ui_mut().panel_at(pos, f)
    }

    /// 嵌套**可拖拽**面板（位置持久于 `UiState.panel_pos`）。
    fn drag_panel_at(
        &mut self,
        id: &str,
        pos: impl Into<Position>,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().drag_panel_at(id, pos, f)
    }

    /// **嵌套模态对话框**（全屏遮罩 + 对话框，背后交互被阻断）——责任链 builder。
    fn modal<'s>(&'s mut self, id: &'s str) -> ModalBuilder<'s, 'a> {
        self.ui_mut().modal(id)
    }

    /// 嵌套 pack（`pos` 相对当前容器内容原点；不占光标）。
    fn pack_at(
        &mut self,
        pos: impl Into<Position>,
        side: PackSide,
        f: impl FnOnce(&mut Pack<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().pack_at(pos, side, f)
    }

    /// 嵌套 grid（`pos` 相对当前容器内容原点；不占光标）。
    fn grid_at(
        &mut self,
        pos: impl Into<Position>,
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
impl<'ui, 'a> Pack<'ui, 'a> {
    /// **在容器闭包里构造**（组合控件 / 骨架用；应用侧要 pack 走 [`UiAdd::row`]）。
    ///
    /// 唯一用户是 [`Ui::menu_bar`]：它把"一行 + 全宽背景"的栏做成
    /// `ui.container(pos, Frame::new_stack(PackSide::Left, gap, 0.0), ..)` 里的这个 `Pack`，
    /// 再交给 [`crate::widgets::MenuBar`]（后者 `Deref` 到 `Pack`，于是 `bar.add(..)` /
    /// `bar.button(..)` / `bar.label(..)` / `bar.text_input(..)` 全部直接可用）。
    pub(crate) fn new(ui: &'ui mut Ui<'a>) -> Self {
        Self { ui }
    }
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

/// 窗口容器（可重叠 + 焦点置顶 + 可拖拽；见 [`Ui::window`]）。
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

// ─── 容器责任链 builder（window / panel / modal） ─────────────────

// `Resize` / `Level` / `Placement` / `WindowOptions` / `PanelOptions` 已拆到
// `crate::ui_types`，在此重导出（builder 本体留在本文件：它与 `Ui` 内部方法强相关）。

/// **单个窗口的调试信息**（[`Ui::debug_dump`]）。
#[derive(Clone, Debug)]
pub struct UiWindowInfo {
    /// 窗口 **绝对 ID**。
    pub id: String,
    /// z 序（越大越上）。
    pub z: u32,
    /// **本帧提交原点**（物理像素）—— 即 `win_origins[z]`。
    ///
    /// ⚠ 它是**相对直接容器**的原点（顶点管线"采集时减去本窗口 origin、提交时再加回来"，
    /// 所以两者一致 ⇒ 渲染正确）：
    /// - **顶层窗口**（`abs_base = 0` 处录制）= **屏幕坐标**，直接可用；
    /// - **嵌套窗口**（如对话框里的下拉浮层）**不是**屏幕坐标 —— 想用它算屏幕位置，
    ///   要把每层外层窗口的 `origin` 累加（实测：对话框内下拉 dump 里是 `(14,162)`，
    ///   屏幕上在 `(704,492)` = 对话框 `(690,330)` + 它）。
    pub origin: Vec2,
    /// 结算尺寸（物理像素）。
    pub size: Vec2,
    /// 本帧是否处于拖拽激活态。
    pub dragging: bool,
    /// 拖拽按下时的窗口左上角（`None` = 本帧无拖拽基准）。
    pub press_panel: Option<Vec2>,
    /// **跨帧持久位置**（`UiState::panel_pos`；`None` = 从未拖过，用传入 `pos`）。
    pub stored_pos: Option<Vec2>,
    /// **本帧实际提交用的平移量**（`flush_seg` 记录）。
    ///
    /// 与 `origin` 不一致 ⇒ "引擎状态 vs 视觉"不一致（渲染/变换路径问题；
    /// 历史 bug：Mesh/quads 命令忽略 `.transform(..)` ⇒ 提交平移恒为 0，窗口全在左上角）。
    pub submit_pos: Option<Vec2>,
    /// **本窗最近一次提交的批次 scissor**（屏幕物理像素；`None` = 该窗内容不裁剪）。
    ///
    /// 引擎把环境裁剪层（严格窗口 / 滚动可视区 / Clip 沙箱 / 文本框盒）作为 **batch
    /// scissor** 交给 GPU（不再切割几何）——这个字段就是"到底裁到哪"的直接证据
    /// （见 [`crate::UiBatch::clip`] 与 `docs/ENGINE_GUIDE.md` §18.20）。
    pub clip: Option<Rect>,
}

/// **UI 引擎状态快照**（[`Ui::debug_dump`]；`Display` 为单行可 grep 格式）。
#[derive(Clone, Debug)]
pub struct UiDebugDump {
    pub frame: u64,
    /// DPI scale（物理 / 逻辑）。
    pub scale: f32,
    /// 视口物理尺寸。
    pub viewport: Vec2,
    /// 鼠标物理屏幕坐标。
    pub mouse_px: Vec2,
    pub mouse_in_window: bool,
    /// 键盘焦点（绝对 ID）。
    pub focused: Option<String>,
    /// **文本焦点**（只有文本控件持焦点才非 `None`）。
    pub text_focus: Option<String>,
    pub windows: Vec<UiWindowInfo>,
}

impl std::fmt::Display for UiDebugDump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ui[frame={} scale={:.2} viewport=({:.0},{:.0}) mouse=({:.0},{:.0}) in_win={} focus={:?} text_focus={:?}]",
            self.frame,
            self.scale,
            self.viewport.x,
            self.viewport.y,
            self.mouse_px.x,
            self.mouse_px.y,
            self.mouse_in_window,
            self.focused,
            self.text_focus,
        )?;
        for w in &self.windows {
            write!(
                f,
                " | {} z={} origin=({:.0},{:.0}) submit={:?} clip={:?} size=({:.0},{:.0}) drag={} press={:?} stored={:?}",
                w.id, w.z, w.origin.x, w.origin.y, w.submit_pos, w.clip, w.size.x, w.size.y, w.dragging, w.press_panel, w.stored_pos
            )?;
        }
        Ok(())
    }
}

/// **窗口责任链 builder**：[`Ui::window`] 返回。选项链式设置（`.pos` / `.width` /
/// `.height` / `.level` / `.placement` / `.style` / `.clamp` / `.resizable` / `.vscroll` /
/// `.title` / `.close_button` / `.collapsible`）后以 `.show(f)` 终结执行。
///
/// # 设计理念：**无顾虑地使用**
///
/// **什么都不写也必须是对的**：位置自动级联、宽高**由内容撑开**（`pad + 内容 + pad`）、
/// 不裁切、无装饰、不抢焦点。只有**想控制**时才写：
///
/// | 想要 | 写什么 | 之后谁定尺寸 |
/// |---|---|---|
/// | 固定宽 / 高 | `.width(w)` / `.height(h)` | 应用（**初始值**，见下） |
/// | 长内容 + 滚动条 | `.vscroll(ScrollMode::Scroll)`（+ `.height(h)` 给有界视口） | 内容（视口高 = `min(内容, 屏幕剩余)`） |
/// | 裁剪 / 换行二选一 | `.hscroll(ScrollMode::{ClipOnly, NoClip})` | 同上 |
/// | 让用户拖大小 | `.resizable(true)` | **用户**（拖过即持久，`.width()/.height()` 只是初值） |
///
/// ⚠ **不许出现"必须写某个选项否则坏掉"的组合**：任何"本帧用上一帧结果反推尺寸"的解算都
/// 必须有**不依赖上一帧**的引导值（典例：自动宽窗口的滚动视口首帧曾取 1px ⇒ 内容按 1px
/// 折行 ⇒ 窗口塌成一条缝、且**永远**是缝）。完整理念与四条硬规矩见
/// `docs/UI_ARCHITECTURE.md` §0。
pub struct WindowBuilder<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
    id: &'ui str,
    o: WindowOptions,
    /// 标题栏文字（`None` = 不画标题栏）。
    title: Option<&'ui str>,
    /// 关闭按钮绑定的开关（点 × ⇒ 置 `false`；为 `false` 时整个窗口不录制）。
    close: Option<&'ui mut bool>,
    /// 收起（折叠）按钮：`(是否画按钮, 收起状态)`；状态 `None` = **引擎托管**
    /// （存 [`UiState::collapsed`]，点 ⌃ 由引擎翻转）。
    collapsible: Option<(bool, Option<&'ui mut bool>)>,
}

/// **窗口外框部件**（标题栏 / 关闭 / 收起）：由 [`WindowBuilder`] 收集后交给
/// `window_impl`。单独成结构体是为了不再往那个已经很长的参数表里加东西。
pub(crate) struct WindowChrome<'c> {
    pub title: Option<&'c str>,
    pub close: Option<&'c mut bool>,
    /// `(是否画按钮, 收起状态)`；状态 `None` = **引擎托管**（[`UiState::collapsed`]）。
    pub collapsible: Option<(bool, Option<&'c mut bool>)>,
    /// **拖拽缩放** `(是否允许拖动, 允许的轴)`；`None` = 旧行为（**有 `.width(..)` 就能横向拖**）。
    pub resize: Option<(bool, Resize)>,
}

impl WindowChrome<'_> {
    /// 空的窗口外框（无标题栏、无按钮、缩放走旧行为）：modal 这类"已有自己外框"的路径用。
    pub(crate) const fn none() -> WindowChrome<'static> {
        WindowChrome { title: None, close: None, collapsible: None, resize: None }
    }

    /// 是否需要**标题栏**：三者都不给 ⇒ 不画（与不启用本特性时逐像素一致）。
    ///
    /// ⚠ 只给 `collapsible(false, None)` 时**不画标题栏**，但收起状态照旧生效
    /// （"按钮不画、状态仍管布局"）——这是两个参数分开的用处。
    fn bar_on(&self) -> bool {
        self.title.is_some()
            || self.close.is_some()
            || self.collapsible.as_ref().is_some_and(|(s, _)| *s)
    }

    /// 本帧是否画 ⌃ 按钮。
    fn show_collapse(&self) -> bool {
        self.collapsible.as_ref().is_some_and(|(s, _)| *s)
    }

    /// 本帧是否**收起**（只留标题栏）。
    ///
    /// `Some(&mut bool)` = 应用持有；`None` = **引擎托管** ⇒ 读 [`UiState::collapsed`]
    /// （`abs` = 该窗口的**绝对 ID**）。
    fn collapsed(&self, state: &UiState, abs: &str) -> bool {
        self.collapsible
            .as_ref()
            .is_some_and(|(_, c)| match c {
                Some(c) => **c,
                None => state.is_collapsed(abs),
            })
    }
}

impl<'ui, 'a> WindowBuilder<'ui, 'a> {
    /// 窗口左上角（[`Position`]：`Logical`（默认，× scale）/ `Physical` 原样；
    /// 相对当前容器内容原点）。
    ///
    /// **不调本方法 = 引擎自动分配位置**（Win32 `CW_USEDEFAULT` 语义）：
    /// 按窗口**首次出现顺序**级联（默认每级右下 `AUTO_POS_STEP` 逻辑像素），
    /// 位置**跨帧记忆**于 [`UiState::auto_pos`]（不会每帧漂移），用户拖过之后
    /// 停在用户放置处（拖拽态优先于自动位置）。
    pub fn pos(mut self, p: impl Into<Position>) -> Self {
        self.o.pos = Some(p.into());
        self
    }
    /// 固定宽（[`Size<f32>`]：`Logical`（默认，× scale）/ `Physical` 原样；高度自动，
    /// 右下角可鼠标缩放，跨帧持久于 `UiState::window_widths`）。
    pub fn width(mut self, w: impl Into<Size<f32>>) -> Self {
        self.o.width = Some(w.into());
        self
    }
    /// **固定高**（[`Size<f32>`]：`Logical`（默认）/ `Physical`）。
    ///
    /// 用途：**长内容窗口**（列表 / 调色板 / 日志）要的是"有界的视口"，而不是"被内容撑到
    /// 和屏幕一样高"。常与 [`Self::vscroll`]`(ScrollMode::Scroll)` 合用：
    /// `.width(320.0).height(420.0).vscroll(Scroll)` ⇒ 视口 420 高 + 窗口内滚动条。
    ///
    /// 语义与 [`Self::width`] 一致：拖过（`Resize::Vertical`/`Both` 的柄）之后由用户接管，
    /// 持久值（`UiState::window_heights`）优先；收起态**不生效**（收起就是一行标题栏）。
    pub fn height(mut self, h: impl Into<Size<f32>>) -> Self {
        self.o.height = Some(h.into());
        self
    }
    /// **层级**（点击是否置顶；默认 [`Level::Topmost`]）。
    pub fn level(mut self, level: Level) -> Self {
        self.o.level = level;
        self
    }
    /// **内容排布**（默认 [`Placement::Expand`]；[`Placement::Clip`] = 两条轴都裁到窗口矩形）。
    ///
    /// 想要"只裁一条轴"或"滚动条"请用 [`Self::vscroll`] / [`Self::hscroll`]（它们**优先**）。
    pub fn placement(mut self, placement: Placement) -> Self {
        self.o.placement = placement;
        self
    }
    /// **垂直轴的溢出策略**（[`ScrollMode`]）：`NoClip`（默认，内容撑高）/ `ClipOnly`
    /// （视口，裁掉超出部分）/ `Scroll`（视口 + 滚动条 + 滚轮）。
    ///
    /// 显式设置**覆盖** [`Self::placement`]。`Scroll` 时窗口高度取"上一帧结算高"
    /// 作为视口（首次 = 内容高），之后由用户拖拽 / 内容变化驱动。
    pub fn vscroll(mut self, mode: ScrollMode) -> Self {
        self.o.vscroll = Some(mode);
        self
    }
    /// **水平轴的溢出策略**（同 [`Self::vscroll`]）。
    ///
    /// ⚠ 当前实现里 `Scroll` 只覆盖**垂直**轴：`hscroll(Scroll)` 暂按 `ClipOnly` 处理并
    /// 打印一次提示（水平滚动条 + 第二条滚动状态还没做，见 `docs/UI_NEEDS.md`）。
    pub fn hscroll(mut self, mode: ScrollMode) -> Self {
        self.o.hscroll = Some(mode);
        self
    }
    /// **逐窗口样式覆盖**（默认 `None` = 全局 [`Theme::panel`]）。
    pub fn style(mut self, s: PanelStyle) -> Self {
        self.o.style = Some(s);
        self
    }
    /// **位置约束模式**（默认 [`WindowClamp::Screen`]：窗口整体不跑出屏幕）。
    pub fn clamp(mut self, mode: WindowClamp) -> Self {
        self.o.clamp = mode;
        self
    }
    /// **拖拽缩放**：`allow` = 是否允许用户**拖动右下角柄**改尺寸，`axes` = 允许的轴。
    ///
    /// - **不调本方法** = 旧行为：`.width(..)` 的窗口可**横向**拖拽缩放，否则没有柄；
    /// - `resize(false, Resize::None)`：**不画柄、也不响应拖拽**，但 `.width(..)` 仍是
    ///   布局固定宽（菜单 / 下拉浮层这类"尺寸由内容定"的窗口就是这么用的）；
    /// - `resize(true, Resize::Both)`：右下角柄**宽高同调**（高度跨帧持久于
    ///   `UiState::window_heights`，`↖↘` 光标）——高度一旦被拖过就固定下来，
    ///   与宽度同理（固定轴不参与内容撑开；要裁剪请配 `Placement::Clip`）。
    ///
    /// `allow` 是**显式 bool**（用户要的签名）：枚举只表达"哪条轴"，"允不允许"用布尔更直白。
    ///
    /// ⚠ 想要 **egui 那样"只给一个布尔、轴自己推导"** 请用 [`Self::resizable`]
    /// （它会**覆盖**本方法）。
    pub fn resize(mut self, allow: bool, axes: Resize) -> Self {
        self.o.resize = Some((allow, axes));
        self
    }
    /// **能不能拖拽改大小**（只有一个布尔；egui 风 —— 允许的轴由 `vscroll` / `.height(..)`
    /// 推导，内容"压缩还是裁切"由 `hscroll` 决定）：
    ///
    /// | 垂直轴 | `resizable(true)` 允许的轴 | 水平轴内容 |
    /// |---|---|---|
    /// | 视口（`.vscroll(ClipOnly/Scroll)` 或给了 `.height(..)`） | **垂直 + 水平** | `hscroll` 给了非 `NoClip` ⇒ **裁切**；否则**压缩** |
    /// | 不是视口（默认，高度由内容定） | **只有水平** | 同上 |
    ///
    /// `resizable(false)` = 不画柄也不响应拖拽（轴无所谓）。
    ///
    /// **为什么垂直轴要"先有视口"才能拖**：高度由内容决定时（默认）拖高没有意义 ——
    /// 内容当帧就把它顶回去；先给它一个有界视口（`.height(..)` / `.vscroll(..)`），
    /// 拖出来的高才是真正生效的那个值。
    ///
    /// **本方法覆盖 [`Self::resize`]**（`None` = 未调 ⇒ 用 `resize` 的老语义）。
    pub fn resizable(mut self, allow: bool) -> Self {
        self.o.resizable = Some(allow);
        self
    }
    /// **内容子项间距**（[`Size<f32>`]：`Logical`（默认，× scale 取整）/ `Physical` 原样）：
    /// 本窗口内容**垂直栈的行距**（不调 = [`Theme::gap`]）。
    ///
    /// 用途：**下拉 / 菜单这类"内容行紧挨着"的浮层**需要比主题更紧的行距
    /// （见 [`crate::widgets::menu::popup_gap`]：逻辑 1px）。容器自身的 `Theme::gap`
    /// 是给普通窗口内容用的，菜单行按它排会"每两行之间空一大截"（用户实测）。
    pub fn gap(mut self, g: impl Into<Size<f32>>) -> Self {
        self.o.gap = Some(g.into());
        self
    }
    /// **标题栏**（可选）：画一条标题栏作为窗口内容**第一行** —— 底色
    /// `Palette::surface_raised`、底边 1px `PanelStyle::border` 分隔线、文字用 `label` 样式。
    ///
    /// 标题栏空白处**仍可拖动窗口**；不调本方法就完全没有标题栏（默认零影响）。
    pub fn title(mut self, t: &'ui str) -> Self {
        self.title = Some(t);
        self
    }
    /// **关闭按钮**（可选）：标题栏右侧画一个 × ；点击把 `*open` 置 `false`。
    ///
    /// `*open == false` 时**整个窗口不录制** —— 不产生绘制命令、不写窗口原点 / 尺寸、
    /// 也**不占遮挡矩形**（不会留下"看不见却挡点击"的窗口）。重新打开由调用方把
    /// `*open` 置回 `true`（例如菜单里勾回来）。
    pub fn close_button(mut self, open: &'ui mut bool) -> Self {
        self.close = Some(open);
        self
    }
    /// **收起（折叠）按钮**（可选）：`show` = 是否画 ⌃ 按钮，`collapsed` = 收起状态
    /// （`true` = 只留标题栏、跳过内容闭包）。
    ///
    /// 两种所有权：
    /// - `Some(&mut bool)`：**应用持有**（`show = false` 时按钮不画，但 `*c` 照旧生效 ⇒
    ///   可由菜单项 / 代码收起展开，而不必在标题栏上放按钮）；点 ⌃ 把 `*c` 取反；
    /// - `None`：**引擎托管** —— 状态存 [`UiState::collapsed`]（按窗口**绝对 ID**），
    ///   点 ⌃ 由引擎翻转；应用想读 / 清 / 代码收起就用
    ///   [`UiState::is_collapsed`] / [`UiState::set_collapsed`] / [`UiState::toggle_collapsed`]，
    ///   [`UiState::reset`] 一并清空。
    ///
    /// 两种语义**逐帧一致**：点击当帧不变、**下一帧**生效（本帧布局在录制开头就定了）。
    ///
    /// ```no_run
    /// # use rjw_ui::{Ui, UiAdd, UiState};
    /// # fn f(ui: &mut Ui) {
    /// // 应用自己持有：可持久化 / 可与别的状态联动
    /// let mut folded = false;
    /// ui.window("a").title("A").collapsible(true, Some(&mut folded)).show(|w| { w.label("…"); });
    /// // 引擎托管：应用不必多一个字段
    /// ui.window("b").title("B").collapsible(true, None).show(|w| { w.label("…"); });
    /// // 代码里也能收起（引擎托管的那些）
    /// ui.state_mut().set_collapsed("b", true);
    /// # let _ = &mut folded;
    /// # }
    /// ```
    pub fn collapsible(mut self, show: bool, collapsed: Option<&'ui mut bool>) -> Self {
        self.collapsible = Some((show, collapsed));
        self
    }
    /// 终结：录制窗口内容并返回窗口结算尺寸（`Vec2`，物理像素）。
    ///
    /// 关闭（`close_button` 绑定的开关为 `false`）时返回 `Vec2::ZERO` 且**不录制任何东西**。
    pub fn show(self, f: impl FnOnce(&mut Window<'_, '_>)) -> Vec2 {
        let Self { ui, id, o, title, close, collapsible } = self;
        // **关闭**：整窗短路。放在最前面：连 z 分配 / 位置解析都不做 —— 关闭的窗口
        // 不该在 `UiState` 里留下任何本帧痕迹。
        if let Some(open) = &close
            && !**open
        {
            return Vec2::ZERO;
        }
        // API 边界换算：Logical → Physical（内部布局/绘制全物理）。
        // **没写 `.pos()`** ⇒ 引擎分配位置（CW_USEDEFAULT 语义，见 [`Self::auto_window_pos`]）。
        let pos = match o.pos {
            Some(p) => p.to_physical(ui.scale),
            None => {
                let abs = ui.id_for(id).to_static();
                let (vw, vh) = ui.window_physical_size();
                ui.auto_window_pos(&abs, Vec2::new(vw as f32, vh as f32), ui.scale)
            }
        };
        let width = o.width.map(|w| w.to_physical(ui.scale));
        // **egui 风推导**（`.resizable(bool)`）：允许的轴由"垂直轴有没有视口"决定，
        // 覆盖 `.resize(allow, axes)` 的老写法（判定表见 [`Self::resizable`]）。
        let resize = match o.resizable {
            Some(allow) => Some((
                allow,
                resolve_resizable_axes(allow, v_axis_is_viewport(o.vscroll, o.height.is_some())),
            )),
            None => o.resize,
        };
        let height = o.height.map(|h| h.to_physical(ui.scale));
        let content_gap = o.gap.map(|g| g.to_physical(ui.scale));
        // 枚举 → 内部两个开关（公开面不再出现裸布尔）。
        let topmost = o.level == Level::Topmost;
        let strict = o.placement == Placement::Clip;
        let mut chrome = WindowChrome { title, close, collapsible, resize };
        ui.window_impl(
            id,
            pos,
            width,
            height,
            content_gap,
            topmost,
            strict,
            o.style.as_ref(),
            o.clamp,
            (o.vscroll, o.hscroll),
            &mut chrome,
            f,
        )
    }
}

/// **面板责任链 builder**：[`Ui::panel`] 返回。选项链式设置（`.pos` / `.drag` /
/// `.style`）后以 `.show(f)` 终结执行。
pub struct PanelBuilder<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
    pos: Position,
    drag: Option<&'ui str>,
    style: Option<PanelStyle>,
}

impl<'ui, 'a> PanelBuilder<'ui, 'a> {
    /// 面板左上角（[`Position`]：`Logical`（默认）/ `Physical` 原样；相对当前容器内容
    /// 原点，默认 `Logical(0,0)`）。
    pub fn pos(mut self, p: impl Into<Position>) -> Self {
        self.pos = p.into();
        self
    }
    /// 可拖拽面板：按住面板任意处移动 ≥ 3 物理像素可拖动（位置持久于
    /// `UiState.panel_pos`；`id` 须稳定）。
    pub fn drag(mut self, id: &'ui str) -> Self {
        self.drag = Some(id);
        self
    }
    /// **逐面板样式覆盖**（默认 `None` = 全局 [`Theme::panel`]）。
    pub fn style(mut self, s: PanelStyle) -> Self {
        self.style = Some(s);
        self
    }
    /// 终结：录制面板内容并返回面板结算尺寸（`Vec2`，物理像素）。
    pub fn show(self, f: impl FnOnce(&mut Panel<'_, '_>)) -> Vec2 {
        let Self { ui, pos, drag, style } = self;
        let pos = pos.to_physical(ui.scale);
        ui.panel_impl(pos, drag, style.as_ref(), f)
    }
}

/// **模态对话框责任链 builder**：[`Ui::modal`] 返回。选项链式设置（`.pos` /
/// `.width`）后以 `.show(f)` 终结执行。
pub struct ModalBuilder<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
    id: &'ui str,
    pos: Position,
    width: Option<Size<f32>>,
}

impl<'ui, 'a> ModalBuilder<'ui, 'a> {
    /// 对话框左上角（[`Position`]：`Logical`（默认）/ `Physical` 原样；相对当前容器
    /// 内容原点，默认 `Logical(0,0)`）。
    pub fn pos(mut self, p: impl Into<Position>) -> Self {
        self.pos = p.into();
        self
    }
    /// 固定宽（[`Size<f32>`]：`Logical`（默认）/ `Physical` 原样；高度自动）。
    pub fn width(mut self, w: impl Into<Size<f32>>) -> Self {
        self.width = Some(w.into());
        self
    }
    /// 终结：录制遮罩 + 对话框并返回对话框结算尺寸（`Vec2`，物理像素）。
    pub fn show(self, f: impl FnOnce(&mut Window<'_, '_>)) -> Vec2 {
        let Self { ui, id, pos, width } = self;
        let pos = pos.to_physical(ui.scale);
        let width = width.map(|w| w.to_physical(ui.scale));
        ui.modal_impl(id, pos, width, f)
    }
}

// ─── 水平行 Builder ─────────────────────────────────────────────

/// **水平行责任链 builder**：[`UiAdd::row_builder`](crate::UiAdd::row_builder) 返回。
///
/// `row(f)` 等价于"默认形态"：`min_h = Theme::row_h`、无上限、`gap = Theme::gap`、无内边距。
/// 链式设置后用 `.show(|r| ..)` 终结（闭包内拿到 [`Pack`]，与 `row` 一致）。
///
/// 语义（与 [`crate::widgets::SizeClass`] 配合）：
/// - `min_h` 同时是**单行子项的标准高**（被钉住）与**行高下限**；多行子项以它为下限、
///   可把行撑高；
/// - `max_h` 是**行高上限**（超出的子项照录，内容溢出可见）；`min_h > max_h` 时 min 胜；
/// - 子项**左上角对齐、沿 X 推进**，间距 = `gap`，整体内缩 `pad`。
pub struct RowBuilder<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
    min_h: Option<Size<f32>>,
    max_h: Option<Size<f32>>,
    gap: Option<Size<f32>>,
    pad: f32,
}

impl<'ui, 'a> RowBuilder<'ui, 'a> {
    /// **行高下限**（[`Size<f32>`]：`Logical`（默认）/ `Physical`）——同时也是单行子项的
    /// 标准高（不调 = [`Theme::row_h`](crate::Theme::row_h)）。
    pub fn min_h(mut self, h: impl Into<Size<f32>>) -> Self {
        self.min_h = Some(h.into());
        self
    }

    /// **行高上限**（多行子项撑高到此为止；不调 = 不限）。
    pub fn max_h(mut self, h: impl Into<Size<f32>>) -> Self {
        self.max_h = Some(h.into());
        self
    }

    /// **固定行高**（= `min_h(h).max_h(h)`）：单行子项钉到 `h`、多行子项也被夹到 `h`。
    pub fn height(self, h: impl Into<Size<f32>>) -> Self {
        let h = h.into();
        self.min_h(h).max_h(h)
    }

    /// 子项间距（不调 = [`Theme::gap`](crate::Theme::gap)）。
    pub fn gap(mut self, g: impl Into<Size<f32>>) -> Self {
        self.gap = Some(g.into());
        self
    }

    /// 行**内边距**（物理像素；每侧都留这么多；默认 0）。
    pub fn pad(mut self, p: f32) -> Self {
        self.pad = p.max(0.0);
        self
    }

    /// 终结：执行闭包并返回行结算尺寸（物理像素；已按 `min_h` / `max_h` 夹过）。
    pub fn show(self, f: impl FnOnce(&mut Pack<'_, '_>)) -> Vec2 {
        let Self { ui, min_h, max_h, gap, pad } = self;
        let scale = ui.scale();
        let std_h = min_h.map_or_else(|| ui.theme().row_h, |h| h.to_physical(scale));
        let max_h = max_h.map(|h| h.to_physical(scale));
        let gap = gap.map_or_else(|| ui.theme().gap, |g| g.to_physical(scale));
        let origin = ui.cursor_pos();
        let (size, _) = ui.container(origin, Frame::new_stack(PackSide::Left, gap, pad), |ctx| {
            let fr = ctx.ui.frames.last_mut().expect("row frame");
            fr.set_force_h_all(std_h);
            fr.set_row_bounds(Some(std_h), max_h);
            let mut p = Pack { ui: ctx.ui };
            f(&mut p);
        });
        // 结算后补记父容器光标（占一行）。
        if let Some(fr) = ui.frames.last_mut() {
            fr.place_external(size);
        }
        size
    }
}

// ─── 控件实现（Ui 内部方法） ────────────────────────────────────

impl Ui<'_> {
    /// 滑块（显式 rect；拖拽灵敏度 = 1，值随鼠标 1:1）。
    pub fn slider_at(
        &mut self,
        id: &str,
        rect: Rect,
        range: RangeInclusive<f32>,
        value: f32,
    ) -> f32 {
        self.slider_at_drag(id, rect, range, value, 1.0)
    }

    /// 滑块（显式 rect；`sens` = 拖拽灵敏度，每像素数值 = 轨道全值 / 宽 × `sens`）。
    ///
    /// - `sens = 1.0`：值随鼠标 1:1（点击轨道即定位，拖拽从按下位置值增量）；
    /// - `sens > 1` 更快、`< 1` 更慢（`widgets::Slider` 的 `drag_sensitivity`）；
    /// - Shift/Ctrl 速度倍率由控件作者并入 `sens`（如 `widgets::Slider` 的
    ///   `shift_speed` / `ctrl_speed`）。
    ///
    /// 非公开：灵敏度只由 widget builder 决定，避免"同一种控件两条入口"。
    pub(crate) fn slider_at_drag(
        &mut self,
        id: &str,
        rect: Rect,
        range: RangeInclusive<f32>,
        value: f32,
        sens: f32,
    ) -> f32 {
        let style = self.theme.slider.clone();
        self.slider_at_styled(id, rect, range, value, sens, &style)
    }

    /// 滑块（显式 rect + **样式可覆盖** + 灵敏度）——widget 层 / 组合控件经此合并主题
    /// 与逐控件属性（[`Self::slider_at_drag`] 委托本方法）。
    ///
    /// 非公开：样式必须来自 [`crate::style::Theme`] 或控件 builder，避免"同一种控件两条入口"。
    /// 取色器的**通道颜色滑块**靠它给出"该通道 0→最大"的水平渐变轨道
    /// （[`crate::Brush::Horizontal`]）——渐变轨道是刷，不是单色，故必须能逐次覆盖样式。
    pub(crate) fn slider_at_styled(
        &mut self,
        id: &str,
        rect: Rect,
        range: RangeInclusive<f32>,
        value: f32,
        sens: f32,
        style: &crate::style::SliderStyle,
    ) -> f32 {
        let id_for = self.id_for(id);

        self.note_placed(rect);
        let hit = self.hit_abs(&id_for, &rect);
        let btn = self.mouse_left();
        // 登记焦点链（键盘导航：Tab 可到；焦点下左右方向键调值 —— 见下方键盘分支）。
        self.register_focus(&id_for, rect, FocusKind::Slider);
        // 滑块自身有拖拽语义：按下即置位 press_claimed——阻止外层窗口/面板把本次
        // 按下当作拖拽基准（窗口内拖滑块不再连窗口一起动）。
        if btn.down_edge() && hit {
            self.press_claimed = true;
        }
        let (lo, hi) = (*range.start(), *range.end());
        let span = hi - lo;
        // 按下位置的轨道比例（点击即定位；拖拽基准用）。
        let t0 = if span.abs() > f32::EPSILON {
            normalize_x(&rect, self.mouse_local_x()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut new_value = value;
        // 按下基准值先取出（&self 读取，避免与下方 self.state 可变借用冲突）。
        let press_mx = self.mouse_local_x();
        let active = {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            let dragging = update_drag(ws, hit, btn);
            if btn.down_edge() && hit {
                // 拖拽基准：按下鼠标**局部 x**（与增量计算 `mouse_local_x` 同单位——
                // 旧实现混用屏幕坐标导致按下瞬间 dx = -abs_base → 值跳 0）
                // + 按下位置对应值（点击轨道即定位）。
                ws.press_mouse = Some(Vec2::new(press_mx, 0.0));
                ws.press_panel = Some(Vec2::new(press_mx, lo + t0 * span));
            } else if ws.drag_sens != 0.0 && ws.drag_sens != sens {
                // 灵敏度（Shift/Ctrl）变化 → 重设拖拽基准：从**当前值**继续增量
                // （否则 `Δx × 新 sens` 使值瞬间跳变）。
                ws.press_mouse = Some(Vec2::new(press_mx, 0.0));
                ws.press_panel = Some(Vec2::new(press_mx, new_value));
            }
            ws.drag_sens = sens;
            dragging
        };
        // 光标：滑块悬停 / 拖拽 → **左右箭头**（↔，EwResize）——水平调值语义。
        // 用 `cursor_custom`（而非 `cursor_grab/grabbing`）以保证拖拽中也显示 ↔
        // （finish 里 `cursor_grabbing` 优先于 `cursor_custom`）。
        if active || hit {
            self.cursor_custom = Some(UiCursor::EwResize.to_winit());
        }
        if active {
            self.any_pressed = true;
            if span.abs() > f32::EPSILON {
                // **增量拖拽**：从按下位置对应值开始，每像素数值 = 全值/宽 × `sens`。
                // （旧实现"绝对位置"无法表达灵敏度，且拖拽必须落点在轨道内。）
                let (px, pv) = {
                    let ws = self.state.widgets.get(id_for.as_str());
                    let pm = ws.and_then(|w| w.press_mouse).unwrap_or(self.mouse_screen);
                    let pp = ws
                        .and_then(|w| w.press_panel)
                        .unwrap_or(Vec2::new(self.mouse_local_x(), new_value));
                    (pm.x, pp.y)
                };
                let dx = self.mouse_local_x() - px;
                // 增量拖拽：从按下位置对应值开始；**clamp 到 [lo, hi]**（防越界）。
                new_value = (pv + dx * (span / rect.w.max(1.0)) * sens).clamp(lo, hi);
            }
        }
        // 键盘：焦点下滑块用左右方向键调值（步进 = 范围的 5%，即时生效）。
        if self.focused_is(&id_for) && span.abs() > f32::EPSILON {
            let step = span * 0.05;
            if self.keyboard.key(KeyCode::ArrowLeft).down_edge() {
                new_value = (new_value - step).clamp(lo, hi);
            }
            if self.keyboard.key(KeyCode::ArrowRight).down_edge() {
                new_value = (new_value + step).clamp(lo, hi);
            }
        }
        let t = if span.abs() > f32::EPSILON {
            ((new_value - lo) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let elem = self.painter.q.seq + 1;
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
        // 轨道 / 填充 / 手柄都是**圆角**矩形（`SliderStyle::radius`，默认胶囊）。
        // 填充画在手柄**左缘**且与手柄同高——两者都是胶囊时左右端自然接成一条。
        //
        // 轨道是**刷**（可能是"该通道 0→最大"的水平渐变）：无边框（`border_w = 0`，
        // 与旧行为一致，边框色不参与绘制）。填充刷**全透明时不画**——取色器的通道行
        // 靠轨道本身表达色彩，纯色填充会盖掉斜坡，而且省一段几何。
        self.push_panel_like(track_rect, style.track, Color::TRANSPARENT, 0.0, style.radius, elem);
        let fill_alpha = style.fill.as_solid().map(|c| {
            let a: [f32; 4] = c.into();
            a[3]
        });
        if fill_alpha != Some(0.0) {
            self.push_panel_like(
                fill_rect,
                style.fill,
                Color::TRANSPARENT,
                0.0,
                style.radius,
                elem,
            );
        }
        self.push_panel_like(
            handle_rect,
            style.handle,
            style.handle_border,
            if style.handle_w > 2.0 { 1.0 } else { 0.0 },
            style.radius,
            elem,
        );
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

    /// 勾选框（显式 rect + **样式可覆盖**——widget 层 [`crate::widgets::Checkbox`] 经此
    /// 合并主题与逐控件属性；[`Self::checkbox_at`] 委托本方法）。
    ///
    /// 非公开：样式必须来自 [`Theme`] 或 widget builder，避免"同一种控件两条入口"。
    pub(crate) fn checkbox_at_styled(
        &mut self,
        id: &str,
        rect: Rect,
        label: &str,
        checked: bool,
        style: &CheckboxStyle,
    ) -> CheckboxState {
        let abs = self.id_for(id);
        self.note_placed(rect);
        // 命中 / 焦点链 / 键盘激活 / 跨帧状态机：一句话（勾选框没有拖拽语义）。
        let resp =
            self.interact(&abs, rect, crate::widgets::Sense::CLICK.focus(FocusKind::Checkbox));
        self.draw_check_common(rect, label, checked, resp.hovered, style);
        CheckboxState {
            hovered: resp.hovered,
            pressed: resp.pressed,
            checked,
            toggled: resp.clicked,
            clicked: resp.clicked,
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
        // 单选 id 也参与命名空间（组名 `group` 不前缀——跨窗口复用组语义保留）。
        let abs = self.id_for(id);
        self.note_placed(rect);
        let hit = self.hit_abs(&abs, &rect);
        let btn = self.mouse_left();
        // 登记焦点链（键盘导航：Tab 可到；Enter/Space 选中）。
        self.register_focus(&abs, rect, FocusKind::Radio);
        let key_click = self.key_click(&abs, FocusKind::Radio);
        if key_click {
            self.any_pressed = true;
        }
        let mut ev = {
            let ws = self.state.widgets.entry(abs.to_static()).or_default();
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
            .is_some_and(|s| s.as_str() == abs.as_str());
        if ev.clicked {
            self.state
                .radio_groups
                .insert(group.to_owned(), abs.to_static());
        }
        let checked = self
            .state
            .radio_groups
            .get(group)
            .is_some_and(|s| s.as_str() == abs.as_str());
        let style = self.theme.checkbox.clone();
        let (hovered, pressed) = {
            let ws = self.state.widgets.get(abs.as_str()).expect("radio ws");
            (ws.hovered, ws.pressed)
        };
        self.draw_check_common(rect, label, checked, hovered, &style);
        CheckboxState {
            hovered,
            pressed,
            checked,
            toggled: ev.clicked && !was_checked,
            clicked: ev.clicked,
        }
    }

    /// 勾选框 / 单选公共绘制：方框 +（选中时）填充 + 标签文本（样式可覆盖）。
    fn draw_check_common(
        &mut self,
        rect: Rect,
        label: &str,
        checked: bool,
        hovered: bool,
        style: &CheckboxStyle,
    ) {
        let depth = self.painter.q.depth;
        let win = self.painter.q.cur_win;
        let elem = self.painter.q.seq + 1;
        let box_rect = Rect::new(
            rect.x,
            rect.y + (rect.h - style.box_size) * 0.5,
            style.box_size,
            style.box_size,
        );
        let seq = self.next_seq();
        // **边框宽 = 0 时的兜底底色**：未勾选的方框本来只画一圈描边——主题把
        // `border_w` 拖到 0（"平面风格"）后它**整个消失**，标签看起来像"没有控件"
        // （用户实测："控件严重错位"：两个勾选框一个有一个没有）。
        // 这里退化成**实心底**（下沉色 / 悬停色），保证任何主题下都看得见方框。
        let flat = style.border_w <= 0.0;
        if !checked && flat {
            let bg = if hovered {
                self.theme.palette.surface_hover
            } else {
                self.theme.palette.surface_sunken
            };
            let seq = self.next_seq();
            self.painter.q.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: box_rect,
                clip: self.painter.q.clip,
                kind: DrawKind::RoundedRect { corners: [bg; 4], radius: style.radius },
            });
        } else {
            self.painter.q.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: box_rect,
                clip: self.painter.q.clip,
                kind: DrawKind::Border {
                    // 悬停时方框描边转向强调色——与按钮 / 下拉框的悬停反馈一致
                    // （此前勾选框 hover 毫无变化，鼠标移上去看不出"可以点"）。
                    color: if hovered { self.theme.focus.color } else { style.box_border },
                    width: style.border_w,
                    radius: style.radius,
                },
            });
        }
        if checked {
            // 中心填充 = 外框 **内缩**（减法，非写死偏移）：
            // inset（物理像素）= floor(border_w) + floor(CHECKBOX_INNER)，
            // 内缩量与边框一致，任意缩放不溢出。
            let inset_px = style.border_w.floor() + CHECKBOX_INNER.floor();
            let inner = box_rect.shrink(inset_px);
            if inner.w > 0.0 && inner.h > 0.0 {
                let seq = self.next_seq();
                // 填充与外框同心的内圆角（`radius - inset`，clamp 到 0）——
                // 与外框环带的内侧半径取同一套规则，两者贴合不留缝。
                let fill_radius = style.radius.map(|r| (r - inset_px).max(0.0));
                self.painter.q.queue.push(UiDraw {
                    depth,
                    seq,
                    win,
                    elem,
                    rect: inner,
                    clip: self.painter.q.clip,
                    kind: DrawKind::RoundedRect {
                        corners: [style.checked_fill; 4],
                        radius: fill_radius,
                    },
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
        self.painter.q.queue.push(text_cmd(
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
            self.painter.q.clip,
        None,
        ));
    }

    /// 绘制**右下角缩放柄**（拖动框；形状 / 颜色 / 尺寸全部来自
    /// [`InputStyle::grip`](crate::style::InputStyle::grip)，默认三条横线）。
    /// `rect` 为文本框本体矩形（当前容器局部坐标）；交互由 [`Self::resize_handle`] 处理。
    fn draw_resize_grip(&mut self, rect: Rect, grip: &GripStyle) {
        self.push_resize_grip_at(rect, grip);
    }

    /// **可调整宽度的文本输入框**（单行）：右下角拖拽改宽度（高度固定），尺寸跨帧
    /// 持久于 [`UiState::sizes`]，也可由 [`Self::size_handler`]（尺寸责任链）指定/覆盖。
    ///
    /// - `rect`：**初始**矩形（`x/y` 为位置，`w` 为初始宽；高度取 `rect.h`）；
    ///   `min_w`：最小宽；`resize`：[`Resize`]（单行输入框只支持 [`Resize::None`] /
    ///   [`Resize::Horizontal`]，`Both` 等价 `Horizontal`——高度由行高固定）。
    /// - 拖拽结果当帧生效（下一帧起按新宽布局，同 `window(width)` 的 1 帧滞后）。
    pub fn resizable_text_input_at(
        &mut self,
        id: &str,
        rect: Rect,
        value: &mut String,
        min_w: f32,
        resize: Resize,
    ) {
        let id_for = self.id_for(id);
        // 尺寸责任链解析宽度（脚本/布局/用户拖拽覆盖），高度固定 = 传入 rect 高。
        let w = self.resolve_size(&id_for, Vec2::new(rect.w, rect.h)).x.max(min_w);
        let input_rect = Rect::new(rect.x, rect.y, w, rect.h);
        self.text_input_core(id, input_rect, value);
        if resize != Resize::None {
            let style = self.theme.input.clone();
            let hw = 14.0_f32;
            let handle = Rect::new(
                input_rect.x + input_rect.w - hw,
                input_rect.y + input_rect.h - hw,
                hw,
                hw,
            );
            let h_id = format!("{id}::resize");
            if let Some(new) = self.resize_handle(
                &h_id,
                handle,
                Vec2::new(w, input_rect.h),
                Vec2::new(min_w, input_rect.h),
                crate::UiCursor::EwResize,
            ) {
                self.state
                    .sizes
                    .insert(id_for.to_static(), Vec2::new(new.x, input_rect.h));
            }
            self.draw_resize_grip(input_rect, &style.grip);
        }
    }

    /// **可调整大小的文本输入框（多行 TextArea）**：右下角拖拽改尺寸，尺寸跨帧
    /// 持久于 [`UiState::sizes`]，也可由 [`Self::size_handler`]（尺寸责任链）指定/覆盖。
    ///
    /// - `rect`：**初始**矩形；`min`：最小尺寸（`(min_w, min_h)`）；`resize`：[`Resize`]
    ///   （[`Resize::None`] 不显示缩放柄 / [`Resize::Horizontal`] 只调宽 /
    ///   [`Resize::Both`] 宽高同调）。
    /// - 自动换行（`wrap=true`，同 [`Self::text_area_at`]）；宽度变化会触发重新换行。
    pub fn resizable_text_area_at(
        &mut self,
        id: &str,
        rect: Rect,
        value: &mut String,
        min: Vec2,
        resize: Resize,
    ) {
        let id_for = self.id_for(id);
        let resolved = self.resolve_size(&id_for, Vec2::new(rect.w, rect.h));
        let area_rect = Rect::new(rect.x, rect.y, resolved.x.max(min.x), resolved.y.max(min.y));
        self.text_area_impl(id, area_rect, value, true);
        if resize != Resize::None {
            let style = self.theme.input.clone();
            let hw = 14.0_f32;
            let handle = Rect::new(
                area_rect.x + area_rect.w - hw,
                area_rect.y + area_rect.h - hw,
                hw,
                hw,
            );
            // 只调宽：高度锁死为当前高（`min.y` 也取当前高）。
            let (min_size, cursor) = match resize {
                Resize::Horizontal => (Vec2::new(min.x, area_rect.h), crate::UiCursor::EwResize),
                _ => (min, crate::UiCursor::NwseResize),
            };
            let h_id = format!("{id}::resize");
            if let Some(new) = self.resize_handle(
                &h_id,
                handle,
                Vec2::new(area_rect.w, area_rect.h),
                min_size,
                cursor,
            ) {
                self.state.sizes.insert(id_for.to_static(), new);
            }
            self.draw_resize_grip(area_rect, &style.grip);
        }
    }

    /// **单行文本输入框**（显式 `rect`）——[`crate::widgets::TextEditor`] 的薄封装。
    ///
    /// 需要宽 / 高 / 缩放柄 / 字号等属性时直接用责任链
    /// [`crate::widgets::TextEditor`]（本方法等价 `TextEditor::new(id, value).at(rect)`）。
    /// 编辑能力见 [`Self::text_input_core`]。
    pub fn text_input_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        crate::widgets::Widget::ui(crate::widgets::TextEditor::new(id, value).at(rect), self);
    }

    /// **单行文本编辑核心**（[`crate::widgets::TextEditor`] 的实现；调用方请用
    /// [`Self::text_input_at`] / [`UiAdd::text_input`]）。
    ///
    /// 增强能力：
    /// - **超长文本滚动跟随光标**：文本超出内容区时左移，光标始终可见（`WidgetState::text_scroll`）；
    /// - **文本选择**：按住拖拽选择（`WidgetState::sel_anchor`），选择优先于窗口/面板拖拽
    ///   （按下时置位 `press_claimed`）；Ctrl+C/V/X 复制/粘贴/剪切；选择后打字/退格替换选择；
    /// - **IME 组合候选移入浮动提示框**：组合串（preedit）画在输入框下方浮动小框中（不再占行内）。
    pub(crate) fn text_input_core(&mut self, id: &str, rect: Rect, value: &mut String) {
        let id_for = self.id_for(id);
        self.note_placed(rect);
        let hit = self.hit_abs(&id_for, &rect);
        if hit {
            // 鼠标悬停在输入框上 → 本帧系统光标设为 I 型（finish 统一设置）
            self.cursor_text = true;
        }
        let btn = self.mouse_left();
        // 登记焦点链（Tab/方向键可遍历到输入框）。
        self.register_focus(&id_for, rect, FocusKind::TextInput);
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
            .get(id_for.as_str())
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
                cx + prev_scroll,
            ))
        } else {
            None
        };
        let drag_caret = if btn.pressed() && !btn.down_edge() {
            Some(self.caret_index_at_width(
                value,
                input_style.font_size,
                input_style.font_family.as_deref(),
                cx_raw + prev_scroll,
            ))
        } else {
            None
        };
        // 记录帧首光标：仅"光标移动"（打字/方向键/点击/拖选）时做滚动跟随——
        // 滚轮滚动不移动光标 → 不跟随（滚轮自由滚动、光标可滚出视图，不被拉回）。
        let prev_caret = self.state.widgets.get(id_for.as_str()).map(|w| w.caret);
        let caret_est = {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            let ev = update_interact(ws, hit, btn);
            if ev.pressed {
                self.any_pressed = true;
                // 输入框按下占用该次按压：从输入框拖拽 = 选择文本（窗口/面板不建立拖拽基准）
                self.press_claimed = true;
                self.state.focused = Some(id_for.to_static());
                self.state.focused_kind = Some(FocusKind::TextInput);
                if let Some(c) = click_caret {
                    ws.caret = c;
                }
                // 双击检测（同控件时间间隔 ≤ DOUBLE_CLICK_TIME 且位移 < 阈值）：
                // 第二击选中光标所在"词"并进入词模式——按住继续拖拽按词扩散。
                let is_dbl = {
                    let (pt, pp) = (ws.last_click_time, ws.last_click_pos);
                    ws.last_click_time = Some(std::time::Instant::now());
                    ws.last_click_pos = self.mouse_logical;
                    pt.is_some_and(|t| t.elapsed() <= DOUBLE_CLICK_TIME)
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
                if moved
                    && let Some(c) = drag_caret {
                        if ws.sel_word {
                            // 词模式（双击后拖拽）：按词边界扩散选择
                            let anchor = ws.sel_anchor.unwrap_or(c);
                            ws.caret = crate::edit::extend_word_caret(value, anchor, c);
                        } else {
                            ws.caret = c;
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
            let focused = self
                .state
                .focused
                .as_ref()
                .is_some_and(|f| f.as_str() == id_for.as_str());
            if focused {
                // IME 组合中（preedit 非空）**或刚结束的帧**（上一帧在组合）：
                // 退格/删除/方向键由 **IME 系统**处理（缩短组合串、结束组合、移动
                // 组合光标）——本地处理会误删已有文本（组合结束帧 Preedit("") 先清空
                // 候选、随后退格键到达，只看当前帧会误判为非组合而误删）。
                let in_ime_compose =
                    self.keyboard.ime_preedit().is_some_and(|p| !p.is_empty());
                let ime_owns_keys = in_ime_compose || self.state.ime_composing;
                // 编辑状态机（单行）：剪贴板 Ctrl+C/V/X/A（粘贴过滤换行）、选择替换、
                // IME 上屏、普通字符、退格/删除（见 [`crate::edit::apply_frame_edits`]）。
                crate::edit::apply_frame_edits(&self.keyboard, ws, value, false, ime_owns_keys);
                // Shift + ←/→：扩展/收缩选择（无 Shift 取消选择）。
                let shift = self.keyboard.key(KeyCode::ShiftLeft).pressed()
                    || self.keyboard.key(KeyCode::ShiftRight).pressed();
                if self.keyboard.key(KeyCode::ArrowLeft).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, -1, shift);
                }
                if self.keyboard.key(KeyCode::ArrowRight).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, 1, shift);
                }
                if self.keyboard.key(KeyCode::Enter).down_edge() {
                    self.state.focused = None;
                }
                // Esc：取消输入焦点（不再把 Esc 传给应用层快捷键）
                if self.keyboard.key(KeyCode::Escape).down_edge() {
                    self.state.focused = None;
                }
            }
            (focused, ws.caret)
        };
        let (focused, caret) = caret_est;
        // 绘制
        let style = self.theme.input.clone();
        let depth = self.painter.q.depth;
        let win = self.painter.q.cur_win;
        let elem = self.painter.q.seq + 1;
        let border = if focused { style.border_focus } else { style.border };
        // 视觉框绝对矩形（Clip 沙箱用）：**整个输入框**——高亮/光标/文本命令
        // 受其强制裁剪（滚出视图不画出框，且外层 ScrollView 裁切一并生效）。
        let box_clip = Rect::new(self.abs_base.x + rect.x, self.abs_base.y + rect.y, rect.w, rect.h);
        // **Clip 子沙箱**（控件内）：强制裁剪层 = 外层强制 ∩ 输入框矩形。
        let saved_clip = self.painter.q.clip;
        self.painter.q.clip = clip_for_view(saved_clip, box_clip, ViewMode::Clip);
        // 背景 + 边框（radius > 0 走圆角双层矩形）。
        // 圆角来自 `style.radius`（主题值，或 [`crate::widgets::TextEditor::radius`] 的
        // 逐控件覆盖——`NumberInput` 靠它只圆左侧两角，与右侧拖拽手柄拼成直边）。
        let panel_radius = style.radius;
        self.push_panel_like(rect, style.bg, border, style.border_w, panel_radius, elem);
        let content_w = (rect.w - style.padding_x * 2.0).max(0.0);
        let content_rect = Rect::new(rect.x + style.padding_x, rect.y, content_w, rect.h);
        // **IME 组合内联融入**：显示串 = value[..caret] + preedit + value[caret..]——
        // 后续文本（"xXXXXAAAA" 的 AAAA）右移而非被组合盖住；组合较长时滚动跟随
        // 组合光标（提示文字不裁切）。无组合时全部回落到 value（零开销路径）。
        // IME 组合串先拷出（owned）：闭包内要 &mut self（text_size 测量），
        // 与自持快照字段 self.keyboard 的借用不能共存。
        let preedit = self.keyboard.ime_preedit().map(|p| p.to_owned());
        let preedit_caret = self.keyboard.ime_preedit_caret();
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
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            let (wx, _) = self.mouse.wheel();
            if hit && wx != 0.0 {
                let max_h_px = (text_w - content_w).max(0.0).round();
                ws.text_scroll = (ws.text_scroll - (wx as f32 * 40.0).round())
                    .clamp(0.0, max_h_px);
            } else if Some(caret) != prev_caret {
                ws.text_scroll = scroll_follow_caret(
                    ws.text_scroll,
                    caret_x,
                    content_w,
                    text_w,
                    8.0,
                );
            }
            ws.text_scroll
        };
        let text_dx = -scroll;
        // 文本选择高亮（在文本之下绘制：同一 elem 的图形组先于文字组）。
        if let Some((lo, hi)) = sel_range(
            self.state.widgets.get(id_for.as_str()).and_then(|w| w.sel_anchor),
            caret,
        ) {
            // 选择高亮（**只覆盖选中文字本身**；空行用一格宽兜底，见
            // [`crate::edit::selection_highlight_w`]）。
            let space_w = self
                .text_size(" ", style.font_size, style.font_family.as_deref())
                .x;
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
                content_rect.y + 3.0,
                crate::edit::selection_highlight_w(hi_x - lo_x, space_w),
                (content_rect.h - 6.0).max(0.0),
            );
            if sel_rect.w > 0.0 && sel_rect.h > 0.0 {
                let seq = self.next_seq();
                self.painter.q.queue.push(UiDraw {
                    depth,
                    seq,
                    win,
                    elem,
                    rect: sel_rect,
                    // 选择高亮受输入框强制裁剪（不溢出输入框 / 外层滚动容器）。
                    clip: self.painter.q.clip,
                    // **圆角 + 上下留白**：原来是整块无圆角实心（上下各只缩 1px），
                    // 在圆角输入框里看起来就是一个"方框顶着边框"。现在贴近文字行高，
                    // 小圆角（顺带吃到羽化抗锯齿）。
                    kind: DrawKind::RoundedRect {
                        corners: [style.sel_bg; 4],
                        radius: CornerRadius::all((sel_rect.h * 0.22).min(4.0)),
                    },
                });
            }
        }
        // 文本（左移 scroll；**裁剪窗口固定在视觉框**：clip 相对移动后的 rect 起点 =
        // scroll/scale - padding_x，绝对位置 = 框左缘 —— 若 clip.x=0 会随 rect 一起
        // 左移，始终显示文本开头且偏离文本框；缓冲控件自持）。
        let clip = Rect::new(scroll - style.padding_x, 0.0, rect.w, rect.h);
        // 绘制文本：组合时画**显示串**（preedit 已融入）；缓冲控件自持（按键变化重排）。
        let (draw_text, buf) = match &composed {
            Some((disp, ..)) => {
                let buf = self.ensure_text_buf(
                    id_for.as_str(),
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
                    id_for.as_str(),
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
        self.painter.q.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            // ⚠ 命令矩形 = **墨迹范围**（宽取全文宽 `text_w`，不是框内宽 `content_w`）：
            // `draw_text_quads` 只用它的 x/y（对齐锚点 + 软裁剪的相对基准），所以观感不变；
            // 但"排版矩形 = 墨迹范围"这条不变量让**兜底剔除**与 `debug_layout` 描边都正确
            // （见 `gpu_batch::text_visible_rect` 的文档：框宽会让滚过头的文本被误剔）。
            Rect::new(content_rect.x + text_dx, content_rect.y, text_w, rect.h),
            Arc::from(draw_text),
            style.font_size,
            style.fg,
            TextAlign::Left,
            TextVAlign::Center,
            style.font_family.clone(),
            Some(clip),
            self.painter.q.clip,
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
                self.painter.q.queue.push(UiDraw {
                    depth,
                    seq: useq,
                    win,
                    elem,
                    rect: ul,
                    clip: self.painter.q.clip,
                    kind: DrawKind::Solid(style.preedit),
                });
            }
        }
        // **IME 候选框定位**：跟随组合光标（窗口客户区物理像素；无组合 = 输入光标）。
        if focused {
            let ime_x = (self.abs_base.x + content_rect.x + caret_x + text_dx) as i32;
            let ime_y = (self.abs_base.y + rect.y) as i32;
            let ime_w = rect.w.max(1.0) as u32;
            let ime_h = rect.h.max(1.0) as u32;
            self.window.set_ime_cursor_area(
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
            self.painter.q.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: caret_rect,
                clip: self.painter.q.clip,
                kind: DrawKind::Caret {
                    color: style.caret,
                    width: 1.0,
                },
            });
        }
        // 退出 Clip 子沙箱（恢复外层强制裁剪层）。
        self.painter.q.clip = saved_clip;
    }

    /// **多行文本输入框（自动换行，显式 `rect`）**——[`crate::widgets::TextEditor`] 的薄封装。
    ///
    /// 等价 `TextEditor::new(id, value).at(rect).multiline()`；宽 / 高 / 缩放柄 / 字号等
    /// 属性请用责任链。编辑能力见 [`Self::text_area_impl`]。
    pub fn text_area_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        crate::widgets::Widget::ui(
            crate::widgets::TextEditor::new(id, value).at(rect).multiline(),
            self,
        );
    }

    /// **多行文本输入框（不自动换行，显式 `rect`）**——等价
    /// `TextEditor::new(id, value).at(rect).multiline().no_wrap()`。
    pub fn text_area_at_nw(&mut self, id: &str, rect: Rect, value: &mut String) {
        crate::widgets::Widget::ui(
            crate::widgets::TextEditor::new(id, value).at(rect).multiline().no_wrap(),
            self,
        );
    }

    /// **多行文本编辑核心**（[`crate::widgets::TextEditor`] 的实现；调用方请用
    /// [`Self::text_area_at`] / [`Self::text_area_at_nw`] / [`UiAdd::text_area`]）。
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
    /// 跟随光标（同单行输入框），垂直滚动不变。
    pub(crate) fn text_area_impl(&mut self, id: &str, rect: Rect, value: &mut String, wrap: bool) {
        let id_for = self.id_for(id);

        self.note_placed(rect);
        let hit = self.hit_abs(&id_for, &rect);
        if hit {
            // 鼠标悬停在输入框上 → 本帧系统光标设为 I 型（finish 统一设置）
            self.cursor_text = true;
        }
        let btn = self.mouse_left();
        self.register_focus(&id_for, rect, FocusKind::TextInput);
        let style = self.theme.input.clone();
        // **行距**：多行行高 = 字号 × `Theme::line_spacing`（与排版缓冲 `line_mult`
        // 一致；cosmic 行盒按此递增，光标/高亮按视觉行序号 × 行高对齐）。
        // 行距是主题令牌：`Theme::density` 的紧凑 / 宽松就是改它（默认
        // `DEFAULT_LINE_SPACING` = 1.2）。
        let line_mult = self.theme.line_spacing;
        let line_h = (style.font_size * line_mult).max(1.0);
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
            id_for.as_str(),
            value,
            style.font_size,
            style.font_family.as_deref(),
            wrap_w,
            line_mult,
        );
        let vlines = Text::lines(&vbuf);
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
            .get(id_for.as_str())
            .map(|w| w.scroll_y)
            .unwrap_or(0.0);
        // 不换行模式：点击列要加**水平滚动**（同单行输入框；换行模式 hscroll = 0）。
        // `scroll_y` / `text_scroll` 均为**物理像素**（内部计算一律物理）→ 文本坐标
        // = 视口坐标 + 物理/scale。
        let hscroll = if wrap {
            0.0
        } else {
            self.state.widgets.get(id_for.as_str()).map(|w| w.text_scroll).unwrap_or(0.0)
        };
        // 滚动条条带排除：内容超出可视区（预估，编辑前后高度变化微小）时，右缘
        // `SCROLLBAR_W` 条带属于滚动条——按下不建立文本选择（锚点为空，拖拽分支
        // 由 `sel_anchor.is_some()` 守卫，滚动条自身交互不受影响）。
        let content_h_pre = Text::measure_buffer(&vbuf).y;
        let sb_w = if content_h_pre > rect.h + 1.0 && rect.h > 0.0 {
            SCROLLBAR_W
        } else {
            0.0
        };
        let hit_text = hit && (self.mouse_local_x() - rect.x) < rect.w - sb_w;
        let click_caret = if btn.down_edge() && hit_text {
            // 行号按**真实行顶**定位（见 line_row_at_y：line_h 每行差 ~0.2px，
            // 长文本累积会错行 / 视图卡住）。
            let row = line_row_at_y(&vlines, mouse_local_y - rect.y + prev_scroll, line_h);
            let cx = (self.mouse_local_x() - rect.x - style.padding_x + hscroll).max(0.0);
            Some(caret_at_visual_click(value, &vlines, row, cx, |s| {
                self.text_size(s, style.font_size, style.font_family.as_deref()).x
            }))
        } else {
            None
        };
        let drag_caret = if btn.pressed() && !btn.down_edge() {
            let row = line_row_at_y(&vlines, mouse_local_y - rect.y + prev_scroll, line_h);
            let cx = (self.mouse_local_x() - rect.x - style.padding_x + hscroll).max(0.0);
            Some(caret_at_visual_click(value, &vlines, row, cx, |s| {
                self.text_size(s, style.font_size, style.font_family.as_deref()).x
            }))
        } else {
            None
        };
        // 记录帧首光标：仅"光标移动"（打字/方向键/点击/拖选）时做光标跟随——
        // 滚轮滚动不移动光标 → 不跟随（滚轮自由滚动、光标可滚出视图，不被拉回）。
        let prev_caret = self.state.widgets.get(id_for.as_str()).map(|w| w.caret);
        let caret_est = {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            let ev = update_interact(ws, hit, btn);
            if ev.pressed {
                self.any_pressed = true;
                // 仅按下在**文本区**（滚动条条带外）时建立文本选择/焦点：
                // 按下滚动条由滚动条自身交互处理（anchor 保持 None）。
                if hit_text {
                    self.press_claimed = true;
                    self.state.focused = Some(id_for.to_static());
                    self.state.focused_kind = Some(FocusKind::TextInput);
                    if let Some(c) = click_caret {
                        ws.caret = c;
                    }
                    // 双击检测（同单行输入框）：第二击选中"词"并进入词模式。
                    let is_dbl = {
                        let (pt, pp) = (ws.last_click_time, ws.last_click_pos);
                        ws.last_click_time = Some(std::time::Instant::now());
                        ws.last_click_pos = self.mouse_logical;
                        pt.is_some_and(|t| t.elapsed() <= DOUBLE_CLICK_TIME)
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
                if moved
                    && let Some(c) = drag_caret {
                        if ws.sel_word {
                            // 词模式（双击后拖拽）：按词边界扩散选择
                            let anchor = ws.sel_anchor.unwrap_or(c);
                            ws.caret = crate::edit::extend_word_caret(value, anchor, c);
                        } else {
                            ws.caret = c;
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
            let focused = self
                .state
                .focused
                .as_ref()
                .is_some_and(|f| f.as_str() == id_for.as_str());
            if focused {
                let in_ime_compose =
                    self.keyboard.ime_preedit().is_some_and(|p| !p.is_empty());
                let ime_owns_keys = in_ime_compose || self.state.ime_composing;
                // 编辑状态机（多行）：剪贴板保留换行、选择替换（Enter 计入）、
                // IME 上屏、普通字符（过滤 '\n'）、退格/删除。
                crate::edit::apply_frame_edits(&self.keyboard, ws, value, true, ime_owns_keys);
                // 换行（Enter；TextArea 语义：插入 '\n'，Esc 失焦）——选择替换已由
                // apply_frame_edits 在 Enter 计入 edit_pending 时先消费。
                if self.keyboard.key(KeyCode::Enter).down_edge() {
                    insert_char_at(value, ws.caret, '\n');
                    ws.caret = (ws.caret + 1).min(value.chars().count());
                }
                // Shift + 方向键/Home/End：扩展选择
                let shift = self.keyboard.key(KeyCode::ShiftLeft).pressed()
                    || self.keyboard.key(KeyCode::ShiftRight).pressed();
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
                if self.keyboard.key(KeyCode::ArrowLeft).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, -1, shift);
                }
                if self.keyboard.key(KeyCode::ArrowRight).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, 1, shift);
                }
                if self.keyboard.key(KeyCode::ArrowUp).down_edge() && !ime_owns_keys {
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
                if self.keyboard.key(KeyCode::ArrowDown).down_edge() && !ime_owns_keys {
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
                if self.keyboard.key(KeyCode::Home).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    shift_clear(ws);
                    let li = vline_of_byte(&vlines, char_to_byte(value, ws.caret));
                    ws.caret =
                        byte_to_char(value, value.floor_char_boundary(vlines[li].byte_start));
                    shift_shrink(ws);
                }
                if self.keyboard.key(KeyCode::End).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    shift_clear(ws);
                    let li = vline_of_byte(&vlines, char_to_byte(value, ws.caret));
                    ws.caret =
                        byte_to_char(value, value.floor_char_boundary(vlines[li].byte_end.min(value.len())));
                    shift_shrink(ws);
                }
                if self.keyboard.key(KeyCode::Escape).down_edge() {
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
            id_for.as_str(),
            value,
            style.font_size,
            style.font_family.as_deref(),
            wrap_w,
            line_mult,
        );
        let vlines = Text::lines(&vbuf);
        // **IME 组合内联融入**（多行）：显示串 = value[..caret] + preedit + value[caret..]，
        // 按内容宽度**重新换行**——组合后的后续文本右移/换行而非被盖住；组合较长时
        // 垂直滚动跟随组合光标。无组合时回落 value（零开销路径）。
        // IME 组合串先拷出（owned）：闭包内要 &mut self（text_size 测量），
        // 与自持快照字段 self.keyboard 的借用不能共存。
        let preedit = self.keyboard.ime_preedit().map(|p| p.to_owned());
        let preedit_caret = self.keyboard.ime_preedit_caret();
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
                        id_for.as_str(),
                        disp,
                        style.font_size,
                        style.font_family.as_deref(),
                        wrap_w,
                        line_mult,
                    );
                    (b.clone(), Text::lines(&b), Some(disp.clone()))
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
        let (wx, wy) = self.mouse.wheel();
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
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            if hit && wx != 0.0 {
                // 水平滚轮（触控板）：自由滚动（可超出光标），clamp 到内容宽
                let max_h_px = (line_w - content_w).max(0.0).round();
                ws.text_scroll = (ws.text_scroll - (wx as f32 * 40.0).round())
                    .clamp(0.0, max_h_px);
            } else if caret_moved {
                // 光标移动（打字/方向键/点击/拖选）→ 跟随；滚轮滚动不跟随
                ws.text_scroll = scroll_follow_caret(
                    ws.text_scroll,
                    caret_x,
                    content_w,
                    line_w,
                    8.0,
                );
            }
            -ws.text_scroll
        };
        // 光标 y = **真实行顶**（`VisualLine.top` **物理像素**）——与渲染行网格
        // 完全一致；`行号 × line_h` 每行差 ~0.2px，长文本累积后光标/滚动目标漂移
        // （视图卡在短于真正底部的纵轴范围内）。`caret_y`（逻辑）供绘制用。
        let caret_y_px = draw_vlines[caret_line].top;
        let caret_y = caret_y_px;
        // 垂直滚动：滚轮 + 光标跟随。内容高用**实际排版缓冲**（组合时 = 显示串缓冲）。
        let content_h = Text::measure_buffer(&draw_buf).y;
        let max_scroll_px = (content_h - rect.h).max(0.0).round();
        let scroll = {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            // 滚轮（**仅鼠标在框内时**——指针离开输入框后不再滚动；拖选中不滚轮）。
            if hit && !ws.pressed
                && wy != 0.0 {
                    ws.scroll_y = (ws.scroll_y - (wy as f32 * 30.0).round())
                        .clamp(0.0, max_scroll_px);
                }
            // **拖选 edge-scroll**：鼠标越出可视区上下缘时按越出量持续滚动
            // （光标随后一帧按新滚动重新定位 → 选择持续延伸，直至文本两端）。
            if ws.pressed {
                let y = mouse_local_y - rect.y;
                if y > rect.h {
                    ws.scroll_y = (ws.scroll_y + (y - rect.h)).min(max_scroll_px);
                } else if y < 0.0 {
                    ws.scroll_y = (ws.scroll_y + y).max(0.0);
                }
            }
            // 光标跟随（仅**光标移动**时，如打字/方向键/点击/拖选）：光标行滚出
            // 可视区时调整——滚轮滚动不移动光标 → 不跟随（滚轮自由滚动、光标可
            // 滚出视图，且不被下一帧拉回）。
            if caret_moved {
                if caret_y_px < ws.scroll_y {
                    ws.scroll_y = caret_y_px;
                } else if caret_y_px + line_h > ws.scroll_y + rect.h {
                    ws.scroll_y =
                        (caret_y_px + line_h - rect.h)
                            .min(max_scroll_px);
                }
            }
            ws.scroll_y = ws.scroll_y.clamp(0.0, max_scroll_px);
            ws.scroll_y
        };
        // 绘制
        let depth = self.painter.q.depth;
        let win = self.painter.q.cur_win;
        let elem = self.painter.q.seq + 1;
        let border = if focused { style.border_focus } else { style.border };
        // **Clip 子沙箱**（控件内）：强制裁剪层 = 外层强制 ∩ 输入框矩形。
        // 光标 / 高亮 / 文本命令自动受其裁剪（滚出视图不画出框）。
        let saved_clip = self.painter.q.clip;
        self.painter.q.clip = clip_for_view(saved_clip, box_clip, ViewMode::Clip);
        self.push_panel_like(rect, style.bg, border, style.border_w, style.radius, elem);
        // 选择高亮（逐**视觉行**；x = 行内前缀宽度，y = 视觉行序号 × 行高——与显示一致）
        if let Some((lo, hi)) = sel_range(
            self.state.widgets.get(id_for.as_str()).and_then(|w| w.sel_anchor),
            caret,
        ) {
            let lo_byte = char_to_byte(value, lo);
            let hi_byte = char_to_byte(value, hi);
            // 一个空格宽度：**行尾/空行提示**——高亮向右多留一格（见循环内说明）。
            let space_w = self
                .text_size(" ", style.font_size, style.font_family.as_deref())
                .x;
            // 用**重新排版后**的 vlines（编辑后行区间才与当前文本一致，否则高亮
            // 错位/消失；见上方重排注释）。
            let lo_li = vline_of_byte(&vlines, lo_byte);
            let hi_li = vline_of_byte(&vlines, hi_byte);
            for (i, line) in vlines[lo_li..=hi_li].iter().enumerate() {
                let li = lo_li + i;
                let ls = line.byte_start.min(value.len());
                let le = line.byte_end.min(value.len());
                let c0b = if li == lo_li { lo_byte.max(ls).min(le) } else { ls };
                let c1b = if li == hi_li { hi_byte.max(ls).min(le) } else { le };
                let x0 = self
                    .text_size(&value[ls..c0b], style.font_size, style.font_family.as_deref())
                    .x;
                let x1 = self
                    .text_size(&value[ls..c1b], style.font_size, style.font_family.as_deref())
                    .x;
                // **只覆盖选中文字本身**；空行（x0==x1，原逻辑 `c1b<=c0b` 直接跳过）
                // 用一格宽兜底，标出该空行已在选中范围内（见
                // [`crate::edit::selection_highlight_w`]）。
                let sel_w = crate::edit::selection_highlight_w(x1 - x0, space_w);
                // y 随垂直滚动上移（-scroll/scale）；clip = 输入框强制层（选择高亮
                // 受裁剪，不溢出输入框 / 外层滚动容器）。
                // 行顶用真实 `VisualLine.top`（与文本行网格一致，长文本不漂移）。
                let sel_rect = Rect::new(
                    content_rect.x + x0 + text_dx,
                    rect.y + vlines[li].top - scroll,
                    sel_w,
                    line_h,
                );
                if sel_rect.w > 0.0 {
                    let seq = self.next_seq();
                    self.painter.q.queue.push(UiDraw {
                        depth,
                        seq,
                        win,
                        elem,
                        rect: sel_rect,
                        clip: self.painter.q.clip,
                        // 与单行输入框一致：圆角高亮（不是硬边实心块）。
                        kind: DrawKind::RoundedRect {
                            corners: [style.sel_bg; 4],
                            radius: CornerRadius::all((sel_rect.h * 0.22).min(4.0)),
                        },
                    });
                }
            }
        }
        // 文本（换行 + 垂直滚动；clip 相对文本块：上缘 = scroll/scale，高 = 可视区；
        // 缓冲控件自持——组合时用显示串缓冲，否则复用 `vbuf`）。
        let seq = self.next_seq();
        self.painter.q.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            // ⚠ 命令矩形 = **墨迹范围**（高取全文高 `content_h`，不是框高 `rect.h`）：
            // `draw_text_quads` 只用它的 x/y（对齐锚点 + 软裁剪的相对基准），观感不变；
            // 但"排版矩形 = 墨迹范围"让**兜底剔除**正确（框高会让纵向滚过头的文本被误剔，
            // 见 `gpu_batch::text_visible_rect`）。
            Rect::new(
                content_rect.x + text_dx,
                content_rect.y - scroll,
                content_w,
                content_h,
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
                scroll,
                rect.w,
                rect.h,
            )),
            self.painter.q.clip,
            Some(draw_buf),
        ));
        // **组合下划线**：覆盖组合文本段（显示串 `[span]`，可能跨视觉行），
        // 受内容区裁剪。组合文本已融入显示串（后续文本右移/换行），无需单独绘制文字。
        if let Some((disp, span, _, _)) = &composed {
            let s_li = crate::edit::vline_of_byte(&draw_vlines, span.start);
            let e_li = crate::edit::vline_of_byte(&draw_vlines, span.end.saturating_sub(1));
            for (i, line) in draw_vlines[s_li..=e_li].iter().enumerate() {
                let li = s_li + i;
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
                    rect.y + draw_vlines[li].top + line_h - 3.0 - scroll,
                    (x1 - x0).max(0.0),
                    2.0,
                );
                if ul.w > 0.0 && ul.h > 0.0 {
                    let useq = self.next_seq();
                    self.painter.q.queue.push(UiDraw {
                        depth,
                        seq: useq,
                        win,
                        elem,
                        rect: ul,
                        clip: self.painter.q.clip,
                        kind: DrawKind::Solid(style.preedit),
                    });
                }
            }
        }
        // **IME 候选框定位**：跟随组合光标（窗口客户区物理像素；无组合 = 输入光标）。
        if focused {
            let ime_x = (self.abs_base.x + content_rect.x + caret_x + text_dx) as i32;
            let ime_y = (self.abs_base.y + rect.y + caret_y_px - scroll) as i32;
            let ime_w = rect.w.max(1.0) as u32;
            let ime_h = line_h.max(1.0) as u32;
            self.window.set_ime_cursor_area(
                PhysicalPosition::new(ime_x, ime_y),
                PhysicalSize::new(ime_w, ime_h),
            );
        }
        // 光标（组合时 = 显示串内的组合光标）
        if focused && self.state.caret_blink_on() {
            let caret_rect = Rect::new(
                content_rect.x + caret_x + text_dx,
                rect.y + caret_y - scroll,
                1.0,
                line_h,
            );
            let seq = self.next_seq();
            self.painter.q.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: caret_rect,
                clip: self.painter.q.clip,
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
                &IdAbsolute::owned(format!("{}::vbar", id_for.as_str())),
                &Rect::new(rect.x, rect.y, rect.w, rect.h),
                rect.h,
                content_h,
                scroll,
                max_scroll_px,
                self.painter.q.clip,
                elem,
            );
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            ws.scroll_y = new_px;
        }
        // 退出 Clip 子沙箱（恢复外层强制裁剪层）。
        self.painter.q.clip = saved_clip;
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

// ─── 窗口标题栏（窗口外框部件） ──────────────────────────────────

/// **标题栏条高**（纯函数，可单测）= **一行** [`Theme::row_h`]。
///
/// 内容贴顶后不再需要"上内边距 + 一行"：标题行录在窗口顶边（`y = 0`），条高就是那一行。
/// ⚠ 条只是**背景装饰、不裁剪内容** —— 标题 / ▲ / ✕（边长 `row_h - 2`）允许**比条高再高一点**
/// （用户明确要求"内容可以比条高再高一点"），本函数**与按钮边长无关**。
#[inline]
fn title_bar_h(row_h: f32) -> f32 {
    row_h
}

/// **标题栏按钮贴外缘的内缩**（物理像素；`0` = 贴窗口外框右缘 = Windows 风格）。
///
/// 单独提出来当常量、而不是散在算式里：这就是"Windows 风格贴角 / 传统内边距"的
/// 唯一开关（想留一条缝就改这一个数，`title_bar_layout` 已经参数化）。
const TITLE_BUTTON_INSET: f32 = 0.0;

/// **标题栏布局解算**（纯函数，可单测；坐标原点 = **窗口外框左上角**）。
///
/// 为什么要有这个函数：旧实现把按钮当**行内子项**排，靠 `spacer = 内容宽 − 标题宽`
/// 把它们推到**内容右缘**——而内容右缘比**窗口外框右缘**整整少一个 `pad`（还有一段
/// 硬编码的 4px 余量），并且 `spacer` 随标题实测宽变化（字体 / 字号 / 文本测量一变就漂）。
/// 实测（scale = 1.5 的 `win_a`）：`✕ 右缘 = 340`，外框右缘 `= 358` ⇒ **偏左 18px**，
/// 而 `18 = pad(14) + 4` 恰好落在面板右上圆角（12 逻辑 = 18 物理）上——巧合掩盖了偏差。
///
/// 现在按钮**绝对定位在窗口外框上**（`Ui::add_at`），位置只由本函数解算：
/// - 固定尺寸窗口（`.width(..)`）：簇右缘 = `bar_w − inset_right` ⇒ **贴外框右缘**；
/// - 自动宽窗口（无 `.width()`）：簇**跟随标题**（`cluster_x = pad + title_w + gap`）——
///   宽度由内容决定时没有"外框右缘"可贴（与旧版自动宽窗口的行为一致）；
/// - 标题可用宽 = 簇左缘 − `gap` − 左内边距（`0` = 没地方放标题，不产生负宽）；
/// - 簇左缘夹到 `≥ pad`：窗口比按钮还窄时，按钮不左越内容左缘；
/// - 顺序 `[收起, 关闭]` ⇒ **关闭在最右**（Windows 语义：✕ 恒在最右上角）。
#[derive(Clone, Copy, Debug, PartialEq)]
struct TitleBarLayout {
    /// 标题可用宽（物理像素；`0` = 不截断成负宽）。
    title_max: f32,
    /// 收起（⌃）按钮矩形（`None` = 不画）。
    collapse: Option<Rect>,
    /// 关闭按钮矩形（`None` = 不画）。
    close: Option<Rect>,
    /// 条宽：固定宽窗口 = 外框宽；自动宽窗口 = 由内容推导出的外框宽。
    bar_w: f32,
}

fn title_bar_layout(
    bar_w: Option<f32>,
    pad: f32,
    row_h: f32,
    gap: f32,
    show_collapse: bool,
    show_close: bool,
    inset_right: f32,
    title_w: f32,
) -> TitleBarLayout {
    // 按钮边长与 `TitleIconButton::size` 一致（`row_h - 2`，下限 12）。
    let btn = (row_h - 2.0).max(12.0);
    let n = (show_collapse as u32 + show_close as u32) as f32;
    let cluster_w = if n > 0.0 { n * btn + (n - 1.0) * gap } else { 0.0 };
    // 簇左缘：固定宽窗口贴外框右缘 − inset；自动宽窗口跟随标题。
    let cluster_x = match bar_w {
        Some(w) => (w - inset_right - cluster_w).max(pad),
        None => pad + title_w + gap,
    };
    let at = |i: f32| Rect::new(cluster_x + i * (btn + gap), 0.0, btn, row_h);
    // 索引 0 = 收起、1 = 关闭 ⇒ 关闭**最右**（只有一个按钮时它自然占 0 号位）。
    let (collapse, close) = match (show_collapse, show_close) {
        (true, true) => (Some(at(0.0)), Some(at(1.0))),
        (true, false) => (Some(at(0.0)), None),
        (false, true) => (None, Some(at(0.0))),
        (false, false) => (None, None),
    };
    TitleBarLayout {
        title_max: (cluster_x - gap - pad).max(0.0),
        collapse,
        close,
        bar_w: bar_w.unwrap_or(cluster_x + cluster_w + pad),
    }
}

/// **窗口标题栏**（窗口内容**第一行**）：标题文字 + 右侧"收起 / 关闭"图标按钮。
///
/// 要素：
/// - 走 `Window`/`UiAdd::row`（与用户内容同一个 `Frame` 结算）⇒ 窗口高度自然包含标题栏，
///   收起时只留它一条；通条底色由 `window_impl` 在 `size` 已知后补画（见那里的注释）；
/// - **按钮绝对定位在窗口外框上**（[`title_bar_layout`]）：右缘 = 外框右缘 −
///   [`TITLE_BUTTON_INSET`]（`0` ⇒ Windows 风格的贴顶右角）；最右按钮的**右上角取面板
///   圆角**，贴外缘时才不会方角戳出圆角轮廓。旧版"行内子项 + spacer"的做法已删除
///   （它永远差一个 `pad + 4px`，见 `title_bar_layout` 的说明）；
/// - 按钮是 [`TitleIconButton`]（**几何图标**，不是 `×` / `_` 字形 —— 换字体不变形），
///   且按下时 `claim_press()` ⇒ **按按钮不会建立窗口拖拽基准**；标题栏空白处仍可拖窗口；
/// - 点击效果：关闭 ⇒ `*close = false`（该窗口**下一帧**整体不录）；收起 ⇒ 状态取反
///   （`Some(&mut bool)` 写回应用；`None` 写进 [`UiState::collapsed`]，**下一帧**生效）。
///
/// ⚠ **它现在跑在一个"长宽已确定的容器"里**（[`Ui::ornament_at`]，`rect = (0, 0, size.x, bar_h)`）：
/// 由 `window_impl` 在**窗口尺寸结算之后、按下裁决之前**调用 ⇒ 这里拿到的 `bar_w` 是
/// **最终外框宽**，所以按钮能贴外缘、标题能按最终宽度居中/省略（旧实现在第一遍录，
/// 自动宽窗口不知道最终宽度，只能回退"跟随标题"）。
fn window_title_bar(
    bar: &mut Pack<'_, '_>,
    chrome: &mut WindowChrome<'_>,
    abs_id: &IdAbsolute<'_>,
    collapsed: bool,
    bar_w: f32,
    pad_total: f32,
    panel_radius: CornerRadius,
) {
    // 行内尺寸全部取**缩放后**主题（`Ui::theme` 已按 DPI 预乘）。
    let (gap, row_h, font_size, family, btn_radius) = {
        let ui = bar.ui_mut();
        (
            ui.theme.gap,
            ui.theme.row_h,
            ui.theme.label.font_size,
            ui.theme.label.font_family.clone(),
            ui.theme.button.radius,
        )
    };
    let title = chrome.title.unwrap_or("");
    let show_close = chrome.close.is_some();
    let show_collapse = chrome.show_collapse();
    let natural = bar.ui_mut().text_size(title, font_size, family.as_deref()).x;
    // `Some(bar_w)`：宽度**恒为已知终值** ⇒ 不再走"跟随标题"那条回退分支。
    let layout = title_bar_layout(
        Some(bar_w),
        pad_total,
        row_h,
        gap,
        show_collapse,
        show_close,
        TITLE_BUTTON_INSET,
        natural,
    );

    let mut close_clicked = false;
    let mut collapse_clicked = false;
    // **标题行贴窗口顶边**，x 取**内容左缘**（`pad_total`）：旧实现靠"窗口 frame 的初始
    // 光标 x = `pad_total`"隐式得到；现在跑在装饰容器里（局部原点 = 外框左上角、光标 x = 0），
    // 所以必须**显式**给回 `pad_total`，否则标题会左移一个 `pad_total`
    // （`--ui-dump` 看不到——它只列窗口矩形、不列控件矩形；`RJ_CHROME_TRACE` 的 `title_at`
    // 就是为核对这一条加的）。
    if let Some(fr) = bar.ui_mut().frames.last_mut() {
        fr.cursor = Vec2::new(pad_total, 0.0);
    }
    let title_at = bar.ui_mut().cursor_pos();
    trace_title_bar(collapsed, natural, &layout, title_at);
    // **只有标题留在"行"里**：`row` 负责条高（`force_h_all(row_h)`）与标题的垂直居中；
    // 空标题也照录（它 + `force_h_all` 就是"一行标题栏"的高度来源，否则无标题窗口少一行）。
    bar.row(|r| {
        // 省略号模式 + 簇左缘的硬上限 ⇒ 过长时截断（不撑宽窗口、不挤走按钮）；
        // 不超长时绘制与普通 `label` 完全一致。
        if layout.title_max > 0.0 {
            r.max_size(layout.title_max, 0.0);
        }
        r.add(crate::widgets::Label::new(title).ellipsis());
    });
    // **按钮绝对定位**：本容器局部 `(0, 0)` 就是**外框左上角**（见 [`Ui::ornament_at`] 的
    // `rect = (0, 0, size.x, bar_h)` 与 `window_impl` 末尾统一 `translate(display_pos)`）
    // ⇒ 落点直接是外框坐标，**不需要任何 pad 补偿**。
    // `add_at` 走"一次性放置覆盖"，控件本身照旧 `allocate_sense`（尺寸 = `row_h - 2` ×
    // `row_h`，与解算一致），命中 / 按下认领路径完全不变。
    // 最右按钮的右上角 = **面板右上圆角**：贴外缘必有部分落在面板圆角区，同半径才嵌进去。
    let btn_radius_tr = CornerRadius { tr: panel_radius.tr, ..btn_radius };
    if let Some(rect) = layout.collapse {
        // 收起时显示"展开"箭头（↓），展开时显示"收起"箭头（↑）。
        let icon = if collapsed { Icon::ChevronDown } else { Icon::ChevronUp };
        // 只有"收起在最右"（= 没画关闭按钮）时它才需要接面板圆角。
        let corners = if show_close { None } else { Some(btn_radius_tr) };
        collapse_clicked = bar
            .ui_mut()
            .add_at(
                Position::Physical(Vec2::new(rect.x, rect.y)),
                crate::widgets::title_button::TitleIconButton::new("::collapse", icon)
                    .corners(corners),
            )
            .clicked();
    }
    if let Some(rect) = layout.close {
        close_clicked = bar
            .ui_mut()
            .add_at(
                Position::Physical(Vec2::new(rect.x, rect.y)),
                crate::widgets::title_button::TitleIconButton::new("::close", Icon::Close)
                    .corners(Some(btn_radius_tr)),
            )
            .clicked();
    }
    // 收起状态的所有权：`Some(&mut bool)` 写回应用；`None` = **引擎托管**（写
    // `UiState::collapsed`，键 = 本窗口**绝对 ID**）——两条路都是"下一帧生效"。
    if collapse_clicked && let Some((_, c)) = chrome.collapsible.as_mut() {
        match c {
            Some(c) => **c = !**c,
            None => {
                bar.ui_mut().state_mut().toggle_collapsed(abs_id.as_str());
            }
        }
    }
    if close_clicked && let Some(open) = chrome.close.as_mut() {
        **open = false;
    }
}

/// `RJ_CHROME_TRACE=1`：打印标题栏布局解算（外框宽 / 标题实测宽 / 标题可用宽 / 按钮矩形 /
/// **标题落点** `title_at`）。
///
/// 为什么留一个开关而不是删掉临时打印：标题栏的**贴右缘**依赖"外框宽 + 实测标题宽"，
/// 而文本测量随字体 / 字号 / DPI 变化——出问题时第一件事就是看这几个数。
/// ⚠ 这里打的是**解算结果**（`title_bar_layout` 的输出）而不是中间量：断言口径就是
/// "按钮右缘 == `bar_w − inset`"，所以打印必须包含**能直接核对这条的矩形**。
/// `title_at` = 标题行的落点（含 `pad_total` 的左内边距）——`--ui-dump` 只列窗口矩形、
/// **看不到控件矩形**，所以标题的位置只能靠这一行核对。
fn trace_title_bar(collapsed: bool, title_w: f32, layout: &TitleBarLayout, title_at: Vec2) {
    if std::env::var_os("RJ_CHROME_TRACE").is_some() {
        let r = |o: Option<Rect>| match o {
            Some(r) => format!("[{:.1},{:.1} {:.1}x{:.1}]", r.x, r.y, r.w, r.h),
            None => "-".to_owned(),
        };
        eprintln!(
            "chrome[collapsed={collapsed}] bar_w={:.1} title_w={title_w:.1} title_max={:.1} \
             title_at=({:.1},{:.1}) collapse={} close={} inset={TITLE_BUTTON_INSET:.1}",
            layout.bar_w,
            layout.title_max,
            title_at.x,
            title_at.y,
            r(layout.collapse),
            r(layout.close),
        );
    }
}

// ─── 面板命令（纯函数） ─────────────────────────────────────────
//
// 实现已移到 `draw.rs`（`PanelCmdCtx` / `push_panel_img_cmds`）：它们只依赖
// [`UiDraw`] / [`DrawKind`]，绘制器 [`crate::Painter`] 与单测共用。

// ─── 单元测试（无 GPU） ─────────────────────────────────────────

/// **窗口拖拽缩放的有效开关**（纯函数，可单测）：`opt` = `WindowBuilder::resize` 的显式设置。
///
/// - `Some((allow, axes))` ⇒ 用调用方的（`allow = false` ⇒ 不画柄、不响应拖拽）；
/// - `None` ⇒ **旧行为**：有 `.width(..)` 就能横向拖（`Resize::Horizontal`），没有就不拖。
///
/// 抽成纯函数是为了把"允许 / 轴 / 没设置"三种情况的判定钉在测试里——
/// 这类"开关没接上"的 bug 在 GUI 里很难肉眼发现（柄画了但不响应、或没画却响应）。
fn resolve_window_resize(opt: Option<(bool, Resize)>, has_width: bool) -> (bool, Resize) {
    match opt {
        Some((allow, axes)) => (allow, axes),
        None => (has_width, Resize::Horizontal),
    }
}

/// **该轴向是否让"宽度"由用户接管**（拖过即固定宽；纯函数，可单测）。
///
/// `Horizontal` / `Both` ⇒ `true`；`Vertical` / `None` ⇒ `false`（宽度仍由内容决定）。
#[inline]
fn resize_fixes_width(axes: Resize) -> bool {
    matches!(axes, Resize::Horizontal | Resize::Both)
}

/// **该轴向是否让"高度"由用户接管**（拖过即固定高；纯函数，可单测）。
///
/// `Vertical` / `Both` ⇒ `true`（高度跨帧持久于 `UiState::window_heights`，窗口成为
/// 固定尺寸视口 ⇒ 内容被裁切 / 可配滚动）；`Horizontal` / `None` ⇒ `false`。
#[inline]
fn resize_fixes_height(axes: Resize) -> bool {
    matches!(axes, Resize::Vertical | Resize::Both)
}

/// **单轴的内容溢出策略解算**（纯函数，可单测）。
///
/// 输入：应用的显式选择（`.vscroll(..)` / `.hscroll(..)`，`None` = 没给）、老的
/// [`Placement::Clip`] 一体化开关、以及**这条轴是否被用户拖过尺寸**。输出：该轴怎么处理。
///
/// 规则（顺序即优先级）：
/// 1. **显式设置胜**（新 API 覆盖 `Placement`）；
/// 2. 否则 `Placement::Clip` ⇒ `ClipOnly`（两条轴都裁，老行为）；
/// 3. 否则**被拖过的轴** ⇒ `ClipOnly` —— 这是既有语义："高度一旦被拖过，窗口就是固定
///    尺寸视口，内容不再撑高它"（用户实测的 TTT 窗口 bug 就是这么修的）；
/// 4. 其余 ⇒ `NoClip`（内容撑大窗口，与不加本 API 之前逐像素一致）。
fn resolve_scroll_mode(explicit: Option<ScrollMode>, placement_clip: bool, axis_dragged: bool) -> ScrollMode {
    match explicit {
        Some(m) => m,
        None if placement_clip => ScrollMode::ClipOnly,
        None if axis_dragged => ScrollMode::ClipOnly,
        None => ScrollMode::NoClip,
    }
}

/// **垂直轴是不是"视口"**（纯函数，可单测）：显式 `.vscroll(ClipOnly|Scroll)`，或给了
/// `.height(..)`（固定高 = 有界视口）。
///
/// `NoClip`（默认）= 高度由内容决定 —— 此时"拖高"没有意义（内容当帧就把它顶回去）。
fn v_axis_is_viewport(vscroll: Option<ScrollMode>, has_height: bool) -> bool {
    has_height || vscroll.is_some_and(|m| m != ScrollMode::NoClip)
}

/// **`.resizable(bool)` 的判定表**（纯函数，可单测；egui 风）：
/// 垂直轴是视口 ⇒ **垂直 + 水平**都能拖；否则**只有水平**（高度由内容定，拖它没意义）。
///
/// `allow = false` ⇒ [`Resize::None`]（不画柄也不响应）。
fn resolve_resizable_axes(allow: bool, v_is_viewport: bool) -> Resize {
    match (allow, v_is_viewport) {
        (false, _) => Resize::None,
        (true, true) => Resize::Both,
        (true, false) => Resize::Horizontal,
    }
}

/// **按轴求交裁剪层**（纯函数，可单测）：`ScrollMode::NoClip` 的那条轴**不裁**
/// （另一条轴照裁）——于是"只裁纵向"的窗口内容仍能横向溢出（反之亦然）。
///
/// - 两条轴都 `NoClip` 且无外层裁剪 ⇒ 返回 `None`（**与不加本 API 之前逐字节一致**）；
/// - 只有一条轴裁 ⇒ 另一条轴沿用外层裁剪；没有外层裁剪时用 `fallback`
///   （调用方传**屏幕矩形**：不裁 ≠ 无限，只是"别用窗口边界去裁"）；
/// - 函数名里的"按轴"是重点：旧的 `clip_for_view(.., Clip)` 会把两条轴一起裁，
///   那正是"只想要纵向滚动条，却被横向也裁掉"的原因。
fn clip_for_axes(
    saved: Option<Rect>,
    win: Rect,
    fallback: Rect,
    v: ScrollMode,
    h: ScrollMode,
) -> Option<Rect> {
    if v == ScrollMode::NoClip && h == ScrollMode::NoClip {
        return saved;
    }
    let mut out = saved.unwrap_or(fallback);
    if h != ScrollMode::NoClip {
        // 横向收窄到窗口的 x 范围，纵向保持外层的。
        out = Rect::new(win.x, out.y, win.w, out.h);
    }
    if v != ScrollMode::NoClip {
        out = Rect::new(out.x, win.y, out.w, win.h);
    }
    if let Some(s) = saved {
        let x0 = out.x.max(s.x);
        let y0 = out.y.max(s.y);
        let x1 = (out.x + out.w).min(s.x + s.w);
        let y1 = (out.y + out.h).min(s.y + s.h);
        if x1 <= x0 || y1 <= y0 {
            return Some(Rect::new(x0, y0, 0.0, 0.0));
        }
        out = Rect::new(x0, y0, x1 - x0, y1 - y0);
    }
    Some(out)
}

/// **两个裁剪层求交**（纯函数，可单测）：窗口的"内容盒 scissor"与命令**已经带着**的
/// 内层裁剪（文本框盒 / 内层滚动视口）必须**求交**而不是覆盖 —— 覆盖会把内层更窄的
/// scissor 吃掉（文字画到框外）。任一为 `None` ⇒ 取另一个。
fn clip_and(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) => {
            let x0 = a.x.max(b.x);
            let y0 = a.y.max(b.y);
            let x1 = (a.x + a.w).min(b.x + b.w);
            let y1 = (a.y + a.h).min(b.y + b.h);
            if x1 <= x0 || y1 <= y0 {
                return Some(Rect::new(x0, y0, 0.0, 0.0));
            }
            Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
        }
    }
}

/// **嵌套容器的内容最大宽**（纯函数，可单测）：父级可用宽扣掉本容器内边距。
///
/// `None`（父级不限宽）/ 结果为 `<= 0` ⇒ `None`（不限）——"父级可用宽 0"不该把子项
/// 压成 0 宽（那会让内容彻底看不见），交回原来的自然排版。
///
/// 这一条是"**指定 width 的窗口里，嵌套容器（`row` / `panel` / `view`）不会把内容排到
/// 窗口外面**"的关键：子容器继承该上限后，子项被 clamp、`LimitedInParent` 控件拿到
/// [`Ui::avail_w`] 后自动换行（用户实测："width 较小，控件会突出去，直到你去拖拽缩放"）。
fn content_max_w(parent_avail: Option<f32>, pad_total: f32) -> Option<f32> {
    parent_avail.and_then(|w| {
        let inner = w - pad_total * 2.0;
        (inner > 0.0).then_some(inner)
    })
}

/// **窗口内容是否强制裁剪**（纯函数，可单测；**旧的"一体化"判据**）。
///
/// - `strict`（`.placement(Placement::Clip)`）：应用的显式选择；
/// - `fixed_h = Some(..)`：**高度被用户拖过**（`Resize::Both` 的柄）⇒ 窗口成了"固定
///   尺寸视口"，内容撑不高它；不裁剪就会画到窗口外面（用户实测的 TTT 窗口 bug）。
///   `.width(..)` 不触发本项：固定宽但高度自然时，内容在垂直方向不会溢出。
///
/// ⚠ `window_impl` 现在走**按轴**的 [`resolve_scroll_mode`] + [`clip_for_axes`]；本函数
/// **只剩测试用途**（等价性基准：两条轴都按老判据解算时必须与它一致）⇒ `#[cfg(test)]`，
/// 免得"没人用的旧判据"留在生产代码里被误用。
#[cfg(test)]
fn window_content_clipped(strict: bool, fixed_h: Option<f32>) -> bool {
    strict || fixed_h.is_some()
}

/// 把绘制命令按 `win` 分组、组内按 `depth` 分桶（桶内保持**录制序**）。
/// 免全量排序的提交序基础：与 `sort_by_key((win, depth, elem, group, seq))`
/// **完全等价**（命令同深度内 `(elem, group, seq)` 天然有序——元素随录制递增、
/// 同元素"背景/图形"先于文字录制；唯一乱序维度是 depth 与 win）。
fn bucket_cmds(queue: Vec<UiDraw>) -> std::collections::HashMap<u32, Vec<Vec<UiDraw>>> {
    let mut groups: std::collections::HashMap<u32, Vec<Vec<UiDraw>>> =
        std::collections::HashMap::new();
    for d in queue {
        let buckets = groups.entry(d.win).or_default();
        while buckets.len() as u32 <= d.depth {
            buckets.push(Vec::new());
        }
        buckets[d.depth as usize].push(d);
    }
    groups
}

/// 窗口限位（`WindowClamp::Screen` 模式）：把绝对位置 clamp 到窗口客户区内。
/// - 窗口 ≤ 屏幕：整体在屏幕内，左上角 ∈ `[0, sw-size]`（贴边）；
/// - 窗口 > 屏幕：允许左上角 ∈ `[sw-size, 0]`（窗口**仍覆盖屏幕**，可拖动）——
///   否则窗口比画面大时被钉死在左上角、永远拖不走。
fn clamp_window_pos(abs: Vec2, size: Vec2, sw: f32, sh: f32) -> Vec2 {
    let min_x = (sw - size.x).min(0.0);
    let max_x = (sw - size.x).max(0.0);
    let min_y = (sh - size.y).min(0.0);
    let max_y = (sh - size.y).max(0.0);
    Vec2::new(abs.x.clamp(min_x, max_x), abs.y.clamp(min_y, max_y))
}

/// **绝对矩形 → 窗口局部矩形**（提交 / 镶嵌用的窗口局部物理坐标）。
///
/// 只是平移（减窗口原点）：**环境裁剪不再切割几何**——裁剪改由 batch scissor 在
/// GPU 侧执行（见 [`UiBatch::clip`] / `crate::view::batch_scissor`）。
#[inline]
pub(crate) fn local_of(pr: Rect, anchor_px: Vec2) -> Rect {
    Rect::new(pr.x - anchor_px.x, pr.y - anchor_px.y, pr.w, pr.h)
}

/// **顶层放置序**（纯函数，见 [`Ui`] 的 `z0_ranges` 字段文档）：到 `seq` 这条命令为止
/// 已经开始过的顶层放置个数。
///
/// `starts` = 各顶层放置**第一条命令的 `seq`**（**按录制序**，因此单调不减）；
/// 提前 `break` 依赖这个单调性。散装顶层命令（不属于任何放置）得到"上一个放置"的序
/// —— 它们录制更晚、`elem` 更大，于是仍画在那个放置之上。
///
/// ⚠ 记的是"**第一条命令**的序号"（`begin_top_placement` 传 `queue.seq + 1`），
/// 不是容器入口时的 `seq`——否则紧靠在容器之前录制的那条命令（如 `scroll_at` 的滚动条，
/// 由 `next_seq()` 取号）会被算进新放置，同一容器被拆进两个排序空间。
///
/// 抽成自由函数是为了让这条语义能在**没有 `Ui` 实例**的情况下单测（`Ui` 需要字形图集
/// 与 winit 窗口，构造不出来；见 `ui/tests.rs`）。
pub(crate) fn z0_place_for_seq(starts: &[u32], seq: u32) -> u32 {
    let mut n = 0;
    for &start in starts {
        if start <= seq {
            n += 1;
        } else {
            break;
        }
    }
    n
}

/// `ui` 模块的单元测试（独立文件，见该目录下的 `tests.rs`）。
#[cfg(test)]
#[path = "ui/tests.rs"]
mod tests;
