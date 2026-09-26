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
//!
//! # 模块布局（维护者笔记）
//!
//! 本文件是 **`ui` 模块的根**：只放「常量 / `UiInit` / `Ui` 结构 / `begin` 与帧状态搬运 /
//! 子模块声明与重导出」。实现按职责拆在 `ui/` 的 15 个子模块（`facade` / `prims` /
//! `interaction` / `scroll` / `panel` / `window` / `chrome` / `containers` / `builders` /
//! `controls` / `textedit` / `commit` / `cmds` / `foldable` / `namespace`），分工表与拆分
//! 规矩见 `docs/UI_ARCHITECTURE.md` §5.7。三条硬规矩：
//!
//! 1. **子模块私有 + 根重导出**：公开类型经本文件的 `pub use` 重导出 ⇒ `crate::ui::X`
//!    与 `rjw_ui::X` 两条路径都不变，`lib.rs` 无需改动；
//! 2. **跨子模块共享只用 `pub(super)`**（= `pub(in crate::ui)`）：子模块是 `crate::ui`
//!    的后代，仍能直接读写 `Ui` 的私有字段，**不要**为了搬运开 `pub(crate)` 后门；
//! 3. **搬运提交只改结构**：不夹带行为 / 视觉改动，`--sim-*` 的数值断言是"零行为变化"
//!    的证据（判据 7）。

use std::time::Instant;

use glam::Vec2;
use rjw_keyboard::KeyboardInput;
use rjw_mouse::MouseInput;
use rjw_text::Text;
use rjw_transform::Rect;
use winit::window::Window as WinitWindow;


use crate::focus::FocusEntry;
use crate::id::{IdAbsolute, IdStack};
use crate::input::{KeyboardSnapshot, MouseSnapshot};
use crate::layout::{Frame, PackSide};
use crate::painter::Painter;
use crate::state::UiState;
use crate::style::Theme;
use crate::ui::commit::z0_place_for_seq;

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
    Anchor, Level, PanelOptions, Placement, Resize, ScrollMode, ScrollOutcome, ScrollParam, UiCursor,
    WindowClamp, WindowFx, WindowOptions,
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
            text_valign: crate::widgets::TextVAlignMode::TopLeft,
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
    /// **多行文本编辑的垂直对齐模式**（`TextEditor::valign` 每次调用前写入、调用后还原；
    /// 见 [`crate::TextVAlignMode`]）。默认 [`crate::TextVAlignMode::TopLeft`]。
    ///
    /// 为什么走"帧内暂存 + 还原"而不是加参数：`text_area_impl` / `resizable_text_area_at`
    /// 是**公开 API**（旧 `Ui::text_area_at` 一族），加参数会破坏源码兼容；而它只影响本控件
    /// 的一次调用（`TextEditor::ui` 内部配对设置 / 还原，与 `theme.input` 的临时替换同一手法）。
    pub(crate) text_valign: crate::widgets::TextVAlignMode,
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
        // **区块级容器作用域内不新开放置**（见 [`Ui::container_scope`]）：`foldable` 的标题行
        // 与正文、`namespace` 的正文都是**同一个逻辑区块**，拆成两个 `place` 会让正文首项前面
        // 多带一个父级 `gap`（用户可见："展开的内容看起来悬空"），并多一个空缓存槽。
        if self.painter.q.depth == 0 && !self.painter.q.placement_locked() {
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

}


// ─── 子模块与重导出 ──────────────────────────────────────────────
//
// 顶层公开类型 / trait 的路径保持 `crate::ui::X` 不变（`lib.rs` 的 `pub use` 与
// 其它模块依赖这些路径）；子模块本身是私有的，只经由这里对外可见。

mod builders;
mod chrome;
mod cmds;
mod commit;
mod containers;
mod controls;
mod facade;
mod foldable;
mod interaction;
mod namespace;
mod panel;
mod prims;
mod scroll;
mod textedit;
mod window;

pub use self::builders::{ModalBuilder, PanelBuilder, RowBuilder, WindowBuilder};
pub use self::containers::{
    FlexCtx, Grid, Pack, PackEntry, Panel, Scroll, UiAdd, UiDebugDump, UiWindowInfo, Window,
};
pub use self::foldable::{FoldState, Foldable};
pub use self::namespace::Namespace;
pub(crate) use self::builders::WindowChrome;
pub(crate) use self::containers::ContainerCtx;

/// `ui` 模块的单元测试（独立文件，见该目录下的 `tests.rs`）。
#[cfg(test)]
#[path = "ui/tests.rs"]
mod tests;
