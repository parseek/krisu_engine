//! UI 持久状态：`UiState`（应用持有）+ 控件返回给用户的状态视图（`ButtonState` / `CheckboxState`）。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec2;
use rjw_text::Buffer;
use rjw_transform::Rect;

use crate::focus::{FocusEntry, FocusKind};
use crate::id::IdAbsolute;

/// 控件文本 `Arc<Buffer>` 缓存容量上限：超出时按"本帧未使用"驱逐（帧级近似 LRU），
/// **不再整表清空**——高帧率动态文本（FPS 计数、日志、自动刷新的标签）不会把
/// 静态标签缓存全部冲掉，避免每帧全部重新整形（抖动）。
pub const TEXT_BUFFER_CACHE_CAP: usize = 256;

/// 单个交互控件的持久状态（按 ID 存放于 [`UiState::widgets`]，跨帧保留）。
#[derive(Clone, Debug, Default)]
pub struct WidgetState {
    /// 本帧鼠标是否位于控件本体（含按下时）。
    pub hovered: bool,
    /// 是否处于按下状态（按下后未释放）。
    pub pressed: bool,
    /// 本帧完成一次点击（按下 + 释放均在本体内）。
    pub clicked: bool,
    /// 滑块拖拽中。
    pub dragging: bool,
    /// 面板拖拽基准：按下时面板左上角（**逻辑**坐标）。
    /// 配合 [`Self::press_mouse`]（按下时鼠标**物理**坐标，取整）——
    /// 拖拽中面板位置 = `press_panel + round(鼠标物理增量) / scale`：
    /// **物理像素粒度**（1px 跟手，不受 DPI 逻辑量化的"粘滞"影响），
    /// 且增量取整对鼠标静止噪声滞回（不来回跳）。
    pub press_panel: Option<Vec2>,
    /// 面板拖拽基准：按下时鼠标物理坐标（取整），见 [`Self::press_panel`]。
    pub press_mouse: Option<Vec2>,
    /// **窗口拖拽基准**：按下帧窗口**结算尺寸**（`WindowClamp::Screen` 拖拽中
    /// clamp 边界固定用）。窗口内容尺寸在拖拽中变化时（换行 / 滚动条 / 动态文本），
    /// clamp 边界不随之每帧变 → 窗口位置**纯跟手**，不会在贴边时被推回产生
    /// "单帧跳变"。`None` = 无按下基准（非拖拽帧用上帧尺寸 clamp）。
    pub(crate) press_size: Option<Vec2>,
    /// 文本输入框光标位置（char 索引）。
    pub caret: usize,
    /// 文本选择锚点（char 索引；`Some` = 有选择，范围 = [min(anchor,caret), max)）。
    pub sel_anchor: Option<usize>,
    /// **双击检测**：上次按下时间（`None` = 无历史；两次按下间隔 ≤ [`crate::ui::DOUBLE_CLICK_TIME`]，
    /// 用 `Instant` 而非帧数——高帧率下帧窗口不缩水）。
    pub(crate) last_click_time: Option<std::time::Instant>,
    /// **双击检测**：上次按下位置（物理像素；位移 < 阈值才算同一点）。
    pub(crate) last_click_pos: Vec2,
    /// **词模式选择**：双击后按住拖拽按词边界扩散（`extend_word_caret`）。
    pub(crate) sel_word: bool,
    /// 上一帧是否持有键盘焦点（内置 NumberInput 用：**仅首次聚焦时全选**，
    /// 之后可正常用鼠标部分选择文本）。
    pub(crate) focused_prev: bool,
    /// 单行输入框**水平滚动偏移**（**物理像素**）：超长文本时文本左移、光标跟随可见。
    pub text_scroll: f32,
    /// 多行输入框（TextArea）**垂直滚动偏移**（**物理像素**）。
    pub scroll_y: f32,
    /// **控件自持的文本排版缓冲**：`(key, Arc<Buffer>)`，key 含文本/字号/字体/换行宽/
    /// 版本。文本频繁变化的输入框在此缓存（**不污染** `UiState::text_buffers` 全局缓存）；
    /// 静态标签仍走全局缓存（`CachePolicy::User`）。
    pub(crate) text_buf: Option<(String, Arc<Buffer>)>,
    /// **数字输入框内部持久编辑文本**（`NumberInput` 内部管理、无需调用方持有
    /// `String`）：聚焦时编辑缓冲、失焦清空（显示由 `value` 派生）；`None` = 未聚焦。
    pub(crate) input_text: Option<String>,
    /// **拖拽灵敏度记录**（`NumberInput` / `Slider` 用）：本控件上次拖拽的每像素速度
    /// 倍率；**变化时（按住 Shift/Ctrl 切换）重设拖拽基准**——从当前值继续、不跳变。
    pub(crate) drag_sens: f32,
    /// **子菜单是否展开**（菜单项控件 `Submenu` 的行自持状态；`reset()` 随 `widgets` 清空）。
    ///
    /// 为什么放在**行自己**身上、而不是 `UiState` 的全局槽位：一个菜单里可以有多层子菜单
    /// （子菜单里还能再 `submenu`），全局单槽会被内层覆盖 ⇒ 外层误判"自己没开"而收起。
    /// ⚠ 绝不能借用 `UiState::combo_open`——那是**父下拉**的槽位，早期"菜单里嵌 `Dropdown`"
    /// 的做法正是被它覆盖，症状是"点一下整条 popup 消失"。
    pub(crate) submenu_open: bool,
}

/// 滚动容器状态（`UiState.scrolls`，跨帧持久）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollState {
    /// 滚动偏移（**物理像素**，已 clamp）。以整物理像素步进（滚轮 / 拖 thumb 均
    /// 取整），保证非整数 DPI（125%/150%）下内容按 `offset / scale` 逻辑平移后，
    /// 每个元素绘制取整时**整体刚性移动**——否则相邻元素取整相位不同步，拖动
    /// 滚动条时内容相对位置逐帧交替（"抖动"，如勾选框的蓝色填充块）。
    pub offset: f32,
    /// 内容总高（逻辑像素；clamp 上限 = max(0, content_h - view_h)）。
    pub content_h: f32,
}

/// UI 帧性能统计（`Ui::finish` / `Ui::end_frame` 各阶段耗时，µs）。
///
/// **每帧聚合**（一帧可有多段 UI：[`crate::Ui::finish`] 逐段累加、[`crate::Ui::end_frame`]
/// 写回）：`frame` 每帧 +1，`cmd_count` / `win_count` / 缓存命中与各阶段耗时为**各段之和**，
/// `ui_frame_us` 为**开场 → 收尾**的整帧跨度。示例里读到的是**上一帧**的统计值。
#[derive(Clone, Debug, Default)]
pub struct UiStats {
    /// 统计帧号（每次 `finish` 自增）。
    pub frame: u64,
    /// 本帧录制命令数（排序前队列长度）。
    pub cmd_count: u32,
    /// 本帧提交的窗口数（win > 0）。
    pub win_count: u32,
    /// 本帧**带 scissor 的批次数**（环境裁剪：严格窗口 / 滚动可视区 / Clip 沙箱 /
    /// 文本框盒）。诊断用：它为 0 而界面里明明有 Clip 沙箱 ⇒ 裁剪没接上；
    /// 它远大于窗口数 ⇒ 裁剪区太碎（每个不同 scissor 都要单独一次 draw，见
    /// [`crate::UiBatch::clip`]）。
    pub clip_batches: u32,
    /// 本帧提交的**段数**（每段 = 一次 draw call 候选；由 `segment_runs` 裁决）。
    pub seg_count: u32,
    /// 本帧提交的顶点 / 三角总数（"这一帧到底镶嵌了多少"的直接度量；
    /// 缓存命中时它们与原顶点数相同，但 `collect_us` 会明显下降）。
    pub vert_count: u32,
    pub tri_count: u32,
    /// 窗口顶点缓存命中 / 未命中次数。
    pub cache_hits: u32,
    pub cache_misses: u32,
    /// 队列排序 + 按窗口分组耗时（µs）。
    pub sort_us: f64,
    /// 窗口内容签名（摘要 + 全量哈希）耗时（µs）。
    pub sig_us: f64,
    /// 缓存未命中 → 顶点重建（collect_cmds）耗时（µs）。
    pub collect_us: f64,
    /// 缓存命中 → 提交列表组装（**顶点克隆**）耗时（µs）。
    ///
    /// ⚠ **阶段 9 起恒为 0**：命中路径改成"把缓存的**提交计划** move 出来 → 借用提交 →
    /// move 回去"，不再逐条克隆几何。保留这个字段是**回归哨兵**（一旦它不为 0，就说明
    /// 又有人在命中路径上拷贝了顶点）。
    pub clone_us: f64,
    /// 提交耗时（µs）= [`Self::submit_asm_us`] + [`Self::submit_flush_us`]。
    pub submit_us: f64,
    /// **提交·装配**（收集单元 + 合并计划 + 单元排序，扣掉单列的 `sig` / `collect`）耗时（µs）。
    ///
    /// 这一半是 `rjw_ui` **自己**的账（纯 CPU 顶点搬运），与后半（交后端）分开，
    /// 才能回答"这 0.3ms 该算 UI 还是算渲染器"。
    pub submit_asm_us: f64,
    /// **提交·交后端**（`flush_seg` 循环：窗口变换 / batch scissor / `UiBackend::submit`）耗时（µs）。
    ///
    /// 真实后端（`rjw_krusie::Render2dUiBackend`）在这里调 `mesh_indexed`——**顶点进
    /// `rjw_2d_render` 暂存缓冲的那次拷贝**就发生在这一段里，故它是"UI 提交 vs 渲染器
    /// 吸收"的**唯一分界**。
    pub submit_flush_us: f64,
    /// `Ui::finish` 总耗时（µs）。
    pub finish_us: f64,
    /// **各段开场**（`UiInit::build()`：懒开场 + 冻结输入快照 + 装载帧级事实 + 建根容器）
    /// 累计耗时（µs）。
    ///
    /// 与 `finish_us` 一起把 `ui_frame_us` 三分解：
    /// `ui_frame_us ≈ prologue_us + (应用录制 + 主题构造) + finish_us`。
    /// 缺了它就只能看到"UI 一帧花了多少"，无法判断该算引擎还是应用的账——
    /// 这正是"UI 时间翻倍"排查时第一步需要的信息。
    pub prologue_us: f64,
    /// 整个 UI 帧（`begin` → `finish` 结束）耗时（µs）。
    pub ui_frame_us: f64,
}

/// **UI 帧级暂存**（每帧开场清零；**同一帧内的多段 UI 共享**）。
///
/// 为什么独立于 `Ui`：`Ui` 是**段**（一次 `Ui::begin(..).build()` → `Ui::finish`）的
/// 瞬态视图，而下列事实是**一帧一份**的——多段录制时必须跨段存活，否则
/// "第二段把第一段的按下归属 / 窗口原点 / 焦点链 / 责任链 / 统计全部清掉"。
/// 住在 [`UiState`] 里（而不是运行时），使 `Ui::begin(window, text, state)` 仍是
/// **3 参**（`docs/API_DESIGN.md` §9 白名单）且无自引用借用。
///
/// 生命周期：`UiState::begin_frame()`（帧首，运行时调用）→ [`Self::begin`]；
/// 各段读写；`Ui::end_frame()`（帧尾）→ [`Self::close`]。
pub(crate) struct UiFrameState {
    /// 本帧是否已开场（`UiInit::build()` 懒开场判据 + 帧收尾幂等判据）。
    pub open: bool,
    /// 本帧冻结的鼠标 / 键盘快照（**第一段开场时捕获一次**：段间注入只影响下一帧）。
    pub mouse: crate::input::MouseSnapshot,
    pub keyboard: crate::input::KeyboardSnapshot,
    /// 鼠标屏幕坐标（物理像素）与是否在窗口内。
    pub mouse_logical: Vec2,
    pub mouse_screen: Vec2,
    pub mouse_in_window: bool,
    /// 本帧是否有控件被按下（空白点击清焦点用）。
    pub any_pressed: bool,
    /// 本帧按下是否被文本输入控件占用（拖拽语义归属）。
    pub press_claimed: bool,
    /// 当前拖拽中的面板 / 窗口绝对 ID。
    pub drag_panel: Option<IdAbsolute<'static>>,
    /// 本帧按下命中的最上层窗口（重叠点击裁决）。
    pub win_press_top: Option<(IdAbsolute<'static>, u32)>,
    /// 窗口 z → 窗口左上角（逻辑坐标；顶点局部化基准）。
    pub win_origins: HashMap<u32, Vec2>,
    /// 窗口 z → 窗口绝对 ID（几何缓存 key）。
    pub win_ids: HashMap<u32, IdAbsolute<'static>>,
    /// 本帧焦点链（键盘导航；各段按录制序追加）。
    pub focusables: Vec<FocusEntry>,
    /// 本帧光标意图（帧尾由 `finalize_cursor_and_reset` 统一落到系统光标）。
    pub cursor_text: bool,
    pub cursor_grab: bool,
    pub cursor_grabbing: bool,
    pub cursor_window_drag: bool,
    pub cursor_custom: Option<winit::window::CursorIcon>,
    /// 位置 / 尺寸责任链（应用脚本处理器 + 内置拖拽环；跨段共享）。
    pub pos_chain: Vec<(i32, crate::ui::PosLink)>,
    pub size_chain: Vec<(i32, crate::ui::SizeLink)>,
    /// UI 帧起点（开场时刻；`ui_frame_us` = 收尾 − 起点）。
    pub frame_t0: std::time::Instant,
    /// 各段累加的统计（帧尾写回 `UiState::stats`）。
    pub stats: UiStats,
    /// **本段段号**（帧内第几段，从 `1` 起；帧首重置）。用于给 win=0 放置槽键
    /// 加**段前缀**——放置序 `place` 是段内派生的计数（各段都从 0 起、且第 0 个放置
    /// 恒是"开头的散装顶层命令"），只用 `place` 做缓存键会让不同段的同号槽互相覆盖
    /// （见 `UiState::z0_quads`）。
    pub segment: u32,
    /// **本帧已见到的 win=0 槽**（`(段号, 放置序)`；各段累加，帧首/帧末用于清陈旧槽）。
    ///
    /// 必须**按帧**汇总：`cache_z0_window` 每段各跑一次，段只见到自己那部分槽——按段清
    /// 会把同帧其它段刚写好的缓存删掉，那些槽于是每帧都 miss、每帧重镶嵌整个 win=0 几何。
    pub z0_seen: Vec<(u32, u32)>,
    /// **本帧录制过的窗口绝对 ID**（各段累加；帧首用它清 `window_rects` 里的陈旧项）。
    ///
    /// ⚠ 必须存在帧级暂存里、并在**下一帧开场**清：`Ui::finish` 末尾的 `save_frame_state`
    /// 会把视图上的 `win_ids` / `win_origins` **换走**（swap 回暂存），于是 `finish` 之后的
    /// `finalize_cursor_and_reset` 看到的是**空表**——曾经用它在帧末清 `window_rects`，
    /// 结果是**每帧把整张遮挡表清空**，窗口遮挡退化成"只看本帧已录制的窗口"：本帧录在
    /// 后面的窗口挡不住前面的窗口的控件（用户报的"上层窗口背后的控件仍被触发"）。
    pub window_ids_seen: Vec<IdAbsolute<'static>>,
    /// **本帧认领了按下的控件**（`(控件绝对 ID, 所在窗口的绝对 ID)`；`None` = 没有）。
    ///
    /// 由 `Ui::hit_impl` 在"命中且本帧按下沿"时记下**第一个**认领者；帧末由
    /// `Ui::resolve_widget_press` 用**完备的遮挡表**复核：若它所在窗口并非鼠标下最上层
    /// （被更高 z 的窗口盖住）则撤销这次认领。命中的那一刻几何还没录全，帧末才是唯一能
    /// 拿到完备几何的时机——这是"被盖住的控件仍被触发"的兜底修复。
    ///
    /// ⚠ 窗口**必须存 ID 而不是 z**：z 会在帧末被"点击置顶"抬到 `max+1`，复核时要解它的
    /// **当前** z；拿认领时的旧 z 比，窗口会把自己判成"被别人盖住"，把窗口内所有拖拽撤掉。
    pub press_widget: Option<(IdAbsolute<'static>, Option<IdAbsolute<'static>>)>,
}

impl Default for UiFrameState {
    fn default() -> Self {
        Self {
            open: false,
            mouse: Default::default(),
            keyboard: Default::default(),
            mouse_logical: Vec2::ZERO,
            mouse_screen: Vec2::ZERO,
            mouse_in_window: false,
            any_pressed: false,
            press_claimed: false,
            drag_panel: None,
            win_press_top: None,
            win_origins: HashMap::new(),
            win_ids: HashMap::new(),
            focusables: Vec::new(),
            cursor_text: false,
            cursor_grab: false,
            cursor_grabbing: false,
            cursor_window_drag: false,
            cursor_custom: None,
            pos_chain: vec![(0, crate::ui::PosLink::Drag)],
            size_chain: vec![(0, crate::ui::SizeLink::Drag)],
            frame_t0: std::time::Instant::now(),
            stats: UiStats::default(),
            segment: 0,
            z0_seen: Vec::new(),
            press_widget: None,
            window_ids_seen: Vec::new(),
        }
    }
}

/// 帧级暂存**不参与克隆**（`UiState::clone` 复制的是跨帧持久状态；帧内在新帧里本就该是空的）。
impl Clone for UiFrameState {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// 手写 `Debug`：责任链里装的是 `Box<dyn Fn>`，只打印"有没有 / 几环"。
impl std::fmt::Debug for UiFrameState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiFrameState")
            .field("open", &self.open)
            .field("mouse_in_window", &self.mouse_in_window)
            .field("any_pressed", &self.any_pressed)
            .field("press_claimed", &self.press_claimed)
            .field("drag_panel", &self.drag_panel)
            .field("win_press_top", &self.win_press_top)
            .field("wins", &self.win_origins.len())
            .field("focusables", &self.focusables.len())
            .field("pos_chain", &self.pos_chain.len())
            .field("size_chain", &self.size_chain.len())
            .finish()
    }
}

impl UiFrameState {
    /// 帧开场：清空帧级事实 + 冻结输入快照 + 种入内置责任链环。
    fn begin(&mut self) {
        *self = Self::default();
        self.open = true;
        self.frame_t0 = std::time::Instant::now();
    }

    /// 帧收尾：关场（暂存内容留到下次 `begin` 再清，便于诊断读取）。
    fn close(&mut self) {
        self.open = false;
    }

    /// 冻结本帧输入快照（**本帧第一段开场时调用一次**；后续段复用，段间注入只影响下一帧）。
    pub(crate) fn freeze_input(
        &mut self,
        mouse: crate::input::MouseSnapshot,
        keyboard: crate::input::KeyboardSnapshot,
    ) {
        self.mouse = mouse;
        self.keyboard = keyboard;
        let (mx, my) = self.mouse.pos_px();
        self.mouse_screen = Vec2::new(mx as f32, my as f32);
        self.mouse_logical = self.mouse_screen;
        self.mouse_in_window = self.mouse.in_window();
    }
}

/// UI 全局持久状态（由应用持有，跨帧复用；一个 `UiState` 可对应多个 `Ui`）。
#[derive(Clone, Debug, Default)]
pub struct UiState {
    /// **帧级暂存**（每帧开场清零；**同一帧内多段共享**，见 [`UiFrameState`]）。
    pub(crate) frame_state: UiFrameState,
    /// 控件 **绝对 ID** → 持久状态。
    pub widgets: HashMap<IdAbsolute<'static>, WidgetState>,
    /// 当前持有焦点的控件 **绝对 ID**（文本输入框等）。
    pub focused: Option<IdAbsolute<'static>>,
    /// 当前焦点控件的**类型**（与 `focused` 同步；[`UiState::text_focus`] 据此区分
    /// "文本焦点"与"按钮/滑块焦点"）。
    pub(crate) focused_kind: Option<FocusKind>,
    /// 单选组：组名 → 当前选中的控件 **绝对 ID**。
    pub radio_groups: HashMap<String, IdAbsolute<'static>>,
    /// 可拖拽面板 / 窗口：**绝对 ID** → 左上角位置（屏幕逻辑像素，跨帧持久）。
    pub panel_pos: HashMap<IdAbsolute<'static>, Vec2>,
    /// **窗口 z-order**：窗口 **绝对 ID** → z 值（越大越靠上；点击窗口置顶 = z+1）。
    /// 窗口命令按 z 升序绘制（焦点窗口最后画 → 最上层）。
    pub window_z: HashMap<IdAbsolute<'static>, u32>,
    /// **窗口矩形缓存**：窗口 z → 屏幕矩形（**逻辑像素**），跨帧保留。
    ///
    /// 用途：窗口**遮挡判定**（[`crate::hit::window_occluded`]）——控件命中测试时
    /// 检查鼠标下是否有更高 z 的窗口覆盖，修复"点击穿透"（背后窗口的控件在重叠
    /// 区域不响应）。录制窗口时更新（[`crate::Ui::window`]），`finish` 末尾只保留
    /// **本帧录制过**的窗口（销毁/停用窗口自动清除，z 变化时旧条目随帧清理）。
    /// **窗口遮挡矩形**：窗口**绝对 ID** → 本帧的遮挡矩形（逻辑像素、绝对坐标）。
    ///
    /// ⚠ **键是窗口 ID，不是 z**：z 会在帧末被"点击置顶"抬到 `max+1`，而矩形是那一刻
    /// 录下的。按 z 存会留下"旧 z → 新位置的矩形"，于是被抬高的窗口在**下一帧重录之前**
    /// 挡不住本来在它下面（但 z 大于它旧 z）的窗口——表现为"上层窗口背后的控件仍被点到"。
    /// 按键存 + 查询时把 ID 解成**当前 z**（[`crate::Ui::window_rects_iter`]）即可消除。
    ///
    /// 矩形 = 窗口盒子 ∪ 本帧子控件命中区（`Ui::win_hit_bounds`，见其字段文档）；
    /// 每帧录到时写入，帧末只保留本帧录过的窗口。
    pub(crate) window_rects: HashMap<IdAbsolute<'static>, Rect>,
    /// **窗口本帧提交原点**：窗口**绝对 ID** → `display_pos`（物理；**相对直接容器**）。
    ///
    /// ⚠ 与 `Ui` 的帧内 `win_origins`（**按 z 键**）并存是必要的：同一帧里可以有**多个
    /// `WIN_TOPMOST` 浮层**（下拉里再开子菜单），它们的 z 相同 ⇒ z 键表只剩最后一个，
    /// 而 id 键表能给出**每一个**浮层自己的原点（[`crate::Ui::debug_dump`] 用它；
    /// 渲染不受影响：采集时减、提交时加用的是同一个 z 键值）。
    pub(crate) window_origins: HashMap<IdAbsolute<'static>, Vec2>,
    /// grid 容器：**绝对 ID** → 结算后的单元格尺寸（跨帧缓存，保证布局稳定）。
    pub grid_cells: HashMap<IdAbsolute<'static>, Vec2>,
    /// 控件文本排版缓存：`(文本, 字号位模式, 字体族, 换行宽度位模式, 版本)` →
    /// `(共享 `Arc<Buffer>`, 最后使用帧号)`。
    ///
    /// 用 [`rjw_text::CachePolicy::User`] 创建——**不推入 rjw_text 内部 LRU**
    /// （避免 UI 标签挤占其 128 容量），由本缓存自持；静态标签每帧命中零排版。
    /// 超出 [`TEXT_BUFFER_CACHE_CAP`] 时**只驱逐本帧未使用的条目**（帧级近似 LRU），
    /// 不再整表清空——动态文本（FPS/日志等）不会冲掉静态标签缓存，消除每帧全量
    /// 重新整形的抖动。
    ///
    /// 版本号用于强制刷新缓存（如行高计算方式变更），避免新旧缓存混用导致布局错乱；
    /// 末元组是**换行行距倍率位 + 全局字重 + 版本号**——行距（[`crate::Theme::line_spacing`]）
    /// 与字重（[`crate::Theme::font_weight`]）都是运行时可变主题令牌，必须进键
    /// （不同行距 / 字重各自缓存，不会复用旧排版：字重会改字形**与步进宽度**）。
    pub(crate) text_buffers:
        HashMap<(String, u32, Option<String>, u32, (u32, u16, u8)), (Arc<Buffer>, u64)>,
    /// 帧计数（光标闪烁相位用）。
    pub frame: u64,
    /// 上一帧是否处于 IME 组合中（text_input 退格判定用）：
    /// 组合结束的那一帧，`Preedit("")` 事件先清空候选，随后退格键事件到达——
    /// 若只看当前帧候选会误判为"非组合"而执行本地退格（误删已有文本）。
    /// 组合中或刚结束的帧，退格/删除/方向键一律交给 IME 系统处理。
    pub(crate) ime_composing: bool,
    /// **窗口几何缓存**：窗口 **绝对 ID** → (内容签名, **提交计划**列表)。
    ///
    /// 计划 = 该窗内按提交序**合并好**的 `(纹理, 裁剪, 几何, 元素序)` 段（见
    /// [`crate::gpu_batch::BatchPlan`]）——**合并结果本身跨帧缓存**：命中的帧直接把这些
    /// 段交给后端（`UiBatch` 借用切片，**零顶点拷贝**），不必"克隆碎片 → 每帧重新合并"
    /// （那正是阶段 9 之前 `clone_us + asm_us ≈ 0.3ms` 的来源）。
    ///
    /// 窗口内容不变时复用（`finish` 按**全量签名**命中），**移动窗口只改变换、顶点不重建**；
    /// 任何内容变化（hover 变色、点击按下、文字编辑、滚动等）都会使签名变化而自动重建。
    ///
    /// ⚠ 签名必须是**逐命令全量哈希**（含颜色 / 边框宽 / 圆角 / 对齐 / 光标 / 选择）——
    /// 轻量摘要曾漏掉颜色位，hover/click 变色被误判"内容未变" → 复用陈旧顶点，
    /// 窗口内交互效果不刷新（下拉框 / 背包 / 窗口按钮失效）。
    ///
    /// ⚠ 签名还必须**并入字形图集的区域失效世代号**（`rjw_text::Text::atlas_revision`）：
    /// 顶点烘的是最终 UV，图集重排 / 复用已逐出条目的槽位会让旧 UV 指向别的像素
    /// （"陈旧文字" / "背景消失"），而命令内容不变 ⇒ 只靠命令哈希永不失效。
    /// 见 [`crate::ui::geom_cache_sig`](crate::ui) 与 `crate::Ui` 的 `hash_cmds`。
    pub(crate) window_quads:
        HashMap<IdAbsolute<'static>, (u64, Vec<crate::gpu_batch::BatchPlan>)>,
    /// **非窗口（win=0）内容的按放置几何缓存**：`(段号, 放置序)` → (内容签名, 提交计划)。
    /// 机制同 `window_quads`（**全量签名** → 命中复用 / 未命中重建），但针对
    /// **顶层非窗口放置**（pack / flex / scroll / list / drag_panel / container，以及
    /// 两段放置之间的散装顶层命令各成一段）。值/交互变化只重建对应放置，其余 win=0
    /// 放置仍命中复用（缓解"任何 win=0 变化 → 整区重建"）。
    ///
    /// **键 = `(段号, 放置序)`**（[`crate::UiState::frame_state`] 的 `segment`；
    /// 放置序见 [`crate::ui::Ui::z0_ranges`]）：放置序是**段内派生的计数**（各段都从 0
    /// 起），只用它会让不同段的同号槽互相覆盖：本帧段 1 写入 → 段 2 查同一个键、判 miss、
    /// 覆盖 → 下帧段 1 再判 miss…… 两个槽**永远命中不了**，每帧各自重镶嵌一遍（实测
    /// 7 个 win=0 槽每帧全量重建 ≈ 0.6ms，`cache_miss` 恒等于槽数）。加段前缀后同帧各段
    /// 互不干扰。
    ///
    /// ⚠ 放置序**逐帧重算**、绝不从缓存里读：放置增删会让后面的 `place` 整体平移，
    /// 键跟着变 ⇒ 直接判 miss 重建。反过来若把 `place` 当成缓存条目里的陈旧字段，
    /// 就会出现"命中一个序已过期的条目"⇒ **绘制序错一帧**（闪烁）。
    pub(crate) z0_quads:
        HashMap<(u32, u32), (u64, Vec<crate::gpu_batch::BatchPlan>)>,
    /// **圆角镶嵌缓存**：单位四分之一圆弧表（一张表服务所有半径）。
    ///
    /// 住这里而不是 `Ui`：`Ui` 每帧由 `begin` 重建，放它里面等于每帧重建表。
    /// 缓存只有一张表 ⇒ 不需 LRU、不需容量上限、不需版本号。
    pub(crate) tess: crate::tess::TessCache,
    /// **诊断**：本帧**命中但被更高窗口遮挡而未响应**的控件次数
    /// （点击穿透拦截计数；`Ui::hit_abs` 累加，`begin_frame` 清零）。
    pub(crate) occluded_hits: u32,
    /// **诊断**：本帧**命中但被同窗口内更上层的控件遮挡而未响应**的次数
    /// （控件级遮挡拦截计数；`Ui::hit_abs` 累加，`begin_frame` 清零）——
    /// 与 [`Self::occluded_hits`]（窗口级）分开，便于区分"被窗口挡"与"被控件挡"。
    pub(crate) widget_occluded_hits: u32,
    /// **诊断**：本帧**认领按下后被帧末复核撤销**的次数（`Ui::resolve_widget_press` 累加，
    /// `begin_frame` 清零）——说明该控件在命中那一刻被判"没被遮挡"，而帧末完备的遮挡表
    /// 表明它其实被更高 z 的窗口盖住了（"被盖住的控件仍被触发"的兜底修复触发次数）。
    pub(crate) press_cancelled_by_window: u32,
    /// **控件级遮挡登记**：本帧录制期写入的可交互控件命中区域（[`crate::hit::HitRegion`]）。
    ///
    /// 每帧重建；`begin_frame` 把**上一帧**整表翻页进 [`Self::prev_hit_regions`]。
    pub(crate) hit_regions: Vec<crate::hit::HitRegion>,
    /// **上一帧**的可交互控件命中区域（只读判定输入，见 [`crate::hit::widget_occluded`]）。
    pub(crate) prev_hit_regions: Vec<crate::hit::HitRegion>,
    /// **诊断**：最近一次按下由哪个窗口接收（`finish::resolve_win_press` 写入；
    /// 即重叠点击时被置顶/可拖拽的**最上层**窗口）。跨帧保留直至下一次按下。
    pub(crate) last_press_window: Option<(IdAbsolute<'static>, u32)>,
    /// **诊断**：窗口 z → 上一次 inish **实际提交用的平移量**（lush_seg 写）。
    ///
    /// Ui 每帧由 egin 重建，帧内诊断（Ui::debug_dump）常在本帧**录制期**调用，
    /// 故放在跨帧状态里；与 win_origins 对照即可判定"引擎状态 vs 视觉"是否一致。
    pub(crate) debug_submit: HashMap<u32, Vec2>,
    /// **提交期 scratch 缓冲**（每帧复用，免每帧新建 `Vec`）。
    ///
    /// - `scratch_units`：本帧的**提交单元**列表（窗口 / win=0 顶层放置，计划按值持有）；
    /// - `scratch_ordered`：`merge_plans` 合并一个单元时的"待切段条目"列表。
    ///
    /// 住 `UiState`（跨帧）而不是 `Ui`（每帧重建）才有意义——否则等于每帧新建。
    pub(crate) scratch_units: Vec<crate::gpu_batch::SubmitUnit>,
    pub(crate) scratch_ordered: Vec<crate::gpu_batch::CachedQuad>,
    /// **窗口 z → 最近一次提交的批次 scissor**（[`crate::UiBatch::clip`]；`flush_seg` 写）。
    /// 与 `debug_submit` 同源，回答"这一窗的裁剪到底是多少"。
    pub(crate) debug_clip: HashMap<u32, Rect>,
    /// **滚动容器状态**：`scroll_at` 的 **绝对 ID** → (偏移, 内容高)，跨帧持久。
    pub(crate) scrolls: HashMap<IdAbsolute<'static>, ScrollState>,
    /// **下拉菜单展开状态**：当前展开的 [`Dropdown`](crate::Dropdown)（或
    /// [`Ui::combo_at`](crate::Ui::combo_at) 糖）的 **绝对 ID**（`None` = 全部收起）。
    ///
    /// 单槽 ⇒ **同一时刻只可能有一个下拉开着**：点另一个下拉 = 旧的关新的开。
    /// 清除时机：点菜单项 / 点面板外 / `Esc` / 再点触发器（切换）。
    pub(crate) combo_open: Option<IdAbsolute<'static>>,
    /// **菜单栏展开状态**：当前展开的菜单（[`crate::Ui::menu_bar`] 的**触发器绝对 ID**；
    /// `None` = 全部收起）。点菜单项 / 点栏外 / Esc 都会清空。
    ///
    /// 与 `combo_open` 分开存：菜单栏的展开是"应用级 UI"（同一时刻只该有一个菜单开着），
    /// 而下拉框属于某个控件——混用一个槽会在"菜单开着时点下拉框"上打架。
    pub(crate) menu_open: Option<IdAbsolute<'static>>,
    /// **颜色选择器的全局跨帧数据**（呈现模式 / 替补输入缓冲 / 展开的面板 / HSV 缓存）。
    ///
    /// 类型定义在**控件自己的模块**里（[`crate::widgets::ColorPickerState`]，见
    /// `widgets/colorpicker/state.rs`）——本文件只放"通用 UI 状态"，控件层的事实不散进来。
    /// **所有 `ColorPicker` 共用这一份**：呈现模式是用户偏好，替补文本缓冲与展开面板
    /// 则必须全局唯一（同一时刻只有一个面板，共享缓冲才只有一个所有者）。
    pub color_picker: crate::widgets::ColorPickerState,
    /// **固定宽窗口的宽度**（`window_at_w` 鼠标缩放：**绝对 ID** → 逻辑宽度，跨帧持久）。
    pub(crate) window_widths: HashMap<IdAbsolute<'static>, f32>,
    /// **窗口结算尺寸**（**绝对 ID** → 物理尺寸，跨帧持久）。clamp（`WindowClamp`）
    /// 用——按 **id** 而非窗口 z 索引：**点击置顶 z+1 后尺寸不丢** → clamp 边界稳定
    /// （消除"按下即跳变"）；`window_rects[z]` 仅作兜底。
    pub(crate) window_sizes: HashMap<IdAbsolute<'static>, Vec2>,
    /// **窗口高度持久值**（**绝对 ID** → 高度；只被"宽高同调"（`Resize::Both`）拖拽过的
    /// 窗口写入）。
    ///
    /// 与 `window_widths` 对称：宽度是"固定宽窗口"的持久值，高度默认由内容自然结算；
    /// 用户拖过 `↖↘` 柄之后高度才固定下来（此后内容不再撑高窗口，要裁剪配 `Placement::Clip`）。
    pub(crate) window_heights: HashMap<IdAbsolute<'static>, f32>,
    /// **可拖拽面板的结算尺寸**（**绝对 ID** → 物理尺寸，跨帧持久）。
    ///
    /// 用途与 `window_sizes` 同：命中 / 拖拽 / clamp 用**上一帧屏幕上那个矩形**
    /// （鼠标事件是针对它产生的），从而 `abs_base` 与本帧 `display_pos` 一致——
    /// 面板内文本框在**拖动面板**时不会落后一帧（见 `Ui::panel_impl`）。
    pub(crate) panel_sizes: HashMap<IdAbsolute<'static>, Vec2>,
    /// **用户拖拽缩放的控件尺寸**（[`Ui::resize_handle`]：**绝对 ID** → 逻辑尺寸，跨帧持久）。
    /// 可缩放 widget 的 `size()` 优先读它（首次 = 内容自然尺寸）。
    pub sizes: HashMap<IdAbsolute<'static>, Vec2>,
    /// **窗口整窗口特效**（[`Ui::window_fx`](crate::ui::Ui::window_fx)：**绝对 ID** → tint +
    /// transform override，跨帧持久；默认无 fx）。不进窗口顶点缓存，提交时应用。
    pub(crate) window_fx: HashMap<IdAbsolute<'static>, crate::ui::WindowFx>,
    /// **上一帧 rjw_ui 是否设置过系统光标**（`finish` 光标抑制用：本帧无 UI 光标
    /// 意图且上一帧设过 → 清一次回 Default；从未设过 → 不碰系统光标，保留应用
    /// 自定义光标，如游戏准星）。
    pub(crate) cursor_was_set: bool,

    #[allow(unused)]
    /// 提供部分控件需要的字符串上下文，如数字输入（**绝对 ID** 键）。
    pub(crate) widget_strs: HashMap<IdAbsolute<'static>, String>,
    /// **UI 帧性能统计**（`finish` 各阶段耗时；示例/诊断读取）。
    pub stats: UiStats,
}

impl UiState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 进入新的一帧（**每帧一次**；段起始由 `UiInit::build()` 懒开场——`frame_open()`
    /// 为假才开，故同帧多段只开一次）。
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        // 遮挡拦截计数按帧清零（诊断机制：读的是"上一帧"的累计值）。
        self.occluded_hits = 0;
        self.widget_occluded_hits = 0;
        self.press_cancelled_by_window = 0;
        // 控件级遮挡登记**整表翻页**：同窗口内"后录制的控件画在上面"，而本帧录制到
        // 某控件时**后面的控件还没录制** → 只能用**上一帧**的区域判定谁盖住谁
        // （与窗口级 `window_rects` 同一思路）。swap 复用两块缓冲，无每帧分配。
        std::mem::swap(&mut self.hit_regions, &mut self.prev_hit_regions);
        self.hit_regions.clear();
        // **win=0 放置子槽缓存：按"上一帧的完整槽集合"清陈旧**——必须在
        // `frame_state.begin()` **之前**（begin 会把上一帧的 `z0_seen` 清空）。
        //
        // ⚠ 这一步**不能在段收尾做**：`cache_z0_window` 每段各跑一次，而一次调用只见到
        // **本段**录制的槽 —— 按段清会把同帧其它段刚写好的缓存删掉 ⇒ 那些槽每帧都 miss、
        // 每帧重镶嵌整个 win=0 几何（实测 7/7 个 win=0 槽恒 miss、`collect` ≈ 0.6ms）。
        let seen = std::mem::take(&mut self.frame_state.z0_seen);
        if !seen.is_empty() {
            self.z0_quads.retain(|slot, _| seen.contains(slot));
        }
        // 帧级暂存开场：清空 + 种入内置责任链环（输入快照由 `UiInit::build()` 冻结）。
        self.frame_state.begin();
    }

    /// 帧收尾（**每帧一次**，由 [`crate::Ui::end_frame`] 调用）：帧级暂存关场复位。
    pub(crate) fn end_frame(&mut self) {
        self.frame_state.close();
    }

    /// **本帧是否已开场**（帧级暂存是否有效）。
    ///
    /// 运行时的 UI 帧收尾据此判定"本帧是否录过 UI"（没录过则空操作，零开销）。
    #[inline]
    pub fn frame_open(&self) -> bool {
        self.frame_state.open
    }

    /// 取（或创建）某控件的持久状态。`id` 为**绝对 ID**（控件内 `ui.id_for(..)` 所得）。
    pub fn widget(&mut self, id: &IdAbsolute<'_>) -> &mut WidgetState {
        self.widgets.entry(id.to_static()).or_default()
    }

    /// 移除某个控件状态（控件消失/复用 ID 时）。`id` 为**绝对 ID**。
    pub fn remove(&mut self, id: &IdAbsolute<'_>) {
        self.widgets.remove(id.as_str());
        if self.focused.as_ref().is_some_and(|f| f.as_str() == id.as_str()) {
            self.focused = None;
            self.focused_kind = None;
        }
        for group in self.radio_groups.values_mut() {
            if group.as_str() == id.as_str() {
                *group = IdAbsolute::owned(String::new());
            }
        }
    }

    /// 清空全部状态（示例"重开"等场景）。
    pub fn reset(&mut self) {
        self.widgets.clear();
        self.focused = None;
        self.focused_kind = None;
        self.radio_groups.clear();
        self.grid_cells.clear();
        self.panel_pos.clear();
        self.panel_sizes.clear();
        self.window_z.clear();
        self.window_rects.clear();
        self.window_origins.clear();
        self.text_buffers.clear();
        self.frame = 0;
        self.ime_composing = false;
        self.window_quads.clear();
        self.z0_quads.clear();
        self.occluded_hits = 0;
        self.widget_occluded_hits = 0;
        self.press_cancelled_by_window = 0;
        self.hit_regions.clear();
        self.prev_hit_regions.clear();
        self.last_press_window = None;
        self.scrolls.clear();
        self.combo_open = None;
        self.menu_open = None;
        self.color_picker = crate::widgets::ColorPickerState::default();
        self.sizes.clear();
        self.window_fx.clear();
        self.stats = UiStats::default();
        // **帧级暂存也清**（含 `open = false`）：`reset` 常在**段内**调用（示例"R 重开"），
        // 若把 `open` 留成 true，下一帧第一段就不再开场（帧号不推进 / 命中区不翻页）。
        self.frame_state = UiFrameState::default();
    }

    /// **文本焦点**（`None` = 当前焦点不是文本控件 / 无焦点）。
    ///
    /// 应用应在处理自己的按键逻辑（如 `R` 重置、`Esc` 退出）前检查并跳过：
    /// ```no_run
    /// # let ui_state: rjw_ui::UiState = rjw_ui::UiState::new();
    /// if ui_state.text_focus().is_none() {
    ///     // 处理游戏/应用快捷键……
    /// }
    /// ```
    ///
    /// 与旧的 `capturing_text()`（= `focused.is_some()`，任何控件持焦点都为真）的区别：
    /// 只有**文本输入框**持焦点时才认定"键盘被文本捕获"——按钮 / 滑块 / 下拉框持焦点
    /// （Tab 导航）不该吞掉应用快捷键。
    #[inline]
    pub fn text_focus(&self) -> Option<TextFocus> {
        let id = self.focused.clone()?;
        if self.focused_kind != Some(FocusKind::TextInput) {
            return None;
        }
        Some(TextFocus { id })
    }

    /// 光标是否处于"亮"相位（每 30 帧切换）。
    pub fn caret_blink_on(&self) -> bool {
        (self.frame / 30).is_multiple_of(2)
    }

    /// **诊断**：上一帧**命中但被更高窗口遮挡而未响应**的控件次数
    /// （点击穿透拦截计数——大于 0 说明鼠标下有窗口叠放、背后控件被正确抑制）。
    #[inline]
    pub fn occluded_hits(&self) -> u32 {
        self.occluded_hits
    }

    /// **诊断**：上一帧**命中但被同窗口内更上层控件遮挡而未响应**的次数
    /// （控件级遮挡拦截计数——大于 0 说明鼠标下有**控件重叠**，下层控件被正确抑制：
    /// "两个控件被一起触发"已修复）。与 [`Self::occluded_hits`]（窗口级）分开计数。
    #[inline]
    pub fn widget_occluded_hits(&self) -> u32 {
        self.widget_occluded_hits
    }

    /// **诊断**：最近一次按下由哪个窗口接收（`(id, z)`；重叠点击时置顶/可拖拽的
    /// **最上层**窗口）。跨帧保留至下一次按下。
    #[inline]
    pub fn last_press_window(&self) -> Option<(&str, u32)> {
        self.last_press_window
            .as_ref()
            .map(|(id, z)| (id.as_str(), *z))
    }

    /// 当前**展开的下拉菜单**（[`Dropdown`](crate::Dropdown) 的绝对 ID；`None` = 全部收起）。
    ///
    /// 与 [`Self::menu_open`] 对称（菜单栏是"应用级 UI"、下拉属于某个控件，故两个槽分开
    /// 存——混用会在"菜单开着时点下拉"上打架）。应用侧用法同 `menu_open`：决定自己的
    /// `Esc` / 快捷键要不要让位。
    #[inline]
    pub fn combo_open(&self) -> Option<&str> {
        self.combo_open.as_ref().map(|s| s.as_str())
    }

    /// 当前**展开的菜单栏菜单**（触发器绝对 ID；`None` = 全部收起）。
    ///
    /// 应用侧的用法：别和菜单抢 `Esc`——
    /// ```no_run
    /// # use rjw_ui::UiState;
    /// # fn f(state: &UiState, esc: bool) -> bool {
    /// if state.menu_open().is_none() && esc { /* 应用自己的 Esc 语义（如退出） */ }
    /// # true }
    /// ```
    #[inline]
    pub fn menu_open(&self) -> Option<&str> {
        self.menu_open.as_ref().map(|s| s.as_str())
    }

    /// **诊断**：上一帧"认领按下后被帧末复核撤销"的次数（见 `Ui::resolve_widget_press`
    /// 与 `UiFrameState::press_widget`）。
    ///
    /// 大于 0 说明**命中那一刻**控件被判"没被遮挡"，而**帧末**完备的遮挡表表明它其实被
    /// 更高 z 的窗口盖住了——即"盖住它的窗口本帧才移过来 / 才被抬高 z"。这是兜底修复，
    /// 出现次数应当是 0；持续大于 0 说明还有更早的时序问题值得查。
    #[inline]
    pub fn press_cancelled_by_window(&self) -> u32 {
        self.press_cancelled_by_window
    }
}

/// **文本焦点**（[`UiState::text_focus`] 的产物）：持有焦点的**文本输入控件** id。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextFocus {
    /// 文本控件的**绝对 ID**（与状态键 / 焦点 id 一致）。
    pub id: IdAbsolute<'static>,
}

/// 按钮返回给用户的状态视图（复制值，非借用）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ButtonState {
    pub(crate) hovered: bool,
    pub(crate) pressed: bool,
    pub(crate) clicked: bool,
    pub(crate) released: bool,
}

impl ButtonState {
    #[inline]
    pub fn hovered(&self) -> bool {
        self.hovered
    }
    #[inline]
    pub fn pressed(&self) -> bool {
        self.pressed
    }
    /// 本帧完成一次点击（按下 + 释放均在本体内）。
    #[inline]
    pub fn clicked(&self) -> bool {
        self.clicked
    }
    /// 本帧释放（无论释放时是否在本体内）。
    #[inline]
    pub fn released(&self) -> bool {
        self.released
    }
}

/// 勾选框 / 单选返回给用户的状态视图。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CheckboxState {
    /// 鼠标悬停在本体（含按下时）。
    pub(crate) hovered: bool,
    /// 处于按下状态（按下后未释放）。
    pub(crate) pressed: bool,
    /// 当前是否勾选（绘制用；checkbox 由用户维护，radio 由组内互斥决定）。
    pub(crate) checked: bool,
    /// 本帧发生了"切换"（点击且状态翻转）。
    pub(crate) toggled: bool,
    /// 本帧完成一次点击。
    pub(crate) clicked: bool,
}

impl CheckboxState {
    #[inline]
    pub fn hovered(&self) -> bool {
        self.hovered
    }
    #[inline]
    pub fn pressed(&self) -> bool {
        self.pressed
    }
    #[inline]
    pub fn checked(&self) -> bool {
        self.checked
    }
    #[inline]
    pub fn toggled(&self) -> bool {
        self.toggled
    }
    #[inline]
    pub fn clicked(&self) -> bool {
        self.clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_open_is_readable_and_cleared_by_reset() {
        // 菜单栏的展开状态是**应用可见**的（`menu_open()`：应用据此决定 Esc / 快捷键要不要让位），
        // 且 `reset()` 必须清掉——否则重置 UI 状态后菜单"看起来还开着"、点击行为错乱。
        let mut st = UiState::new();
        assert_eq!(st.menu_open(), None, "初始收起");
        st.menu_open = Some(IdAbsolute::owned("menubar::文件".to_owned()));
        assert_eq!(st.menu_open(), Some("menubar::文件"), "读到触发器绝对 ID");
        st.reset();
        assert_eq!(st.menu_open(), None, "reset 清空菜单展开状态");
    }

    #[test]
    fn combo_open_is_readable_and_cleared_by_reset() {
        // 与 `menu_open()` 对称：下拉菜单的展开状态同样**应用可见**（`Dropdown` 是普通
        // 控件，应用要据此让出 Esc / 快捷键），且 `reset()` 必须清掉（否则"看起来还开着"）。
        let mut st = UiState::new();
        assert_eq!(st.combo_open(), None, "初始收起");
        st.combo_open = Some(IdAbsolute::owned("diff_dd::popup".to_owned()));
        assert_eq!(st.combo_open(), Some("diff_dd::popup"), "读到下拉的绝对 ID");
        st.reset();
        assert_eq!(st.combo_open(), None, "reset 清空下拉展开状态");
    }

    /// **帧级暂存**：一帧开场一次、收尾关场；`UiState::clone` 不带帧内暂存。
    ///
    /// 这些是"一帧可多段 UI"正确性的地基：若 `begin_frame` 被第二次调用（旧实现里
    /// 每段 `Ui::begin(..).build()` 都会调一次），帧号会 +2、命中区表二次翻页；
    /// 若 `clone` 把帧内暂存带过去，两份 `UiState` 会共享上一帧的按下归属。
    #[test]
    fn frame_state_opens_once_per_frame_and_is_not_cloned() {
        let mut state = UiState::new();
        assert!(!state.frame_open(), "初始（未开场）应为关");
        let f0 = state.frame;
        state.begin_frame();
        assert!(state.frame_open(), "begin_frame 后应处于开场状态");
        assert_eq!(state.frame, f0 + 1, "每帧帧号 +1");
        // 帧内暂存：责任链预置内置拖拽环（段间共享、跨段存活）。
        assert_eq!(state.frame_state.pos_chain.len(), 1);
        assert_eq!(state.frame_state.size_chain.len(), 1);
        // 段与段之间**不**再开场：开场状态由 `frame_open()` 决定，第二段复用暂存。
        state.frame_state.any_pressed = true;
        assert!(state.frame_open(), "第一段之后本帧仍是开场状态（第二段复用，不重置）");
        // 帧收尾：关场。
        state.end_frame();
        assert!(!state.frame_open(), "end_frame 后应关场");
        // 下一帧再开场 → 暂存被清零（上一帧的按下归属不残留）。
        state.begin_frame();
        assert!(!state.frame_state.any_pressed, "新帧暂存清零");
        assert_eq!(state.frame, f0 + 2);
        // clone 只复制跨帧持久状态：帧内暂存回到默认（未开场）。
        let cloned = state.clone();
        assert!(!cloned.frame_open(), "克隆的 UiState 帧内暂存为空（未开场）");
        assert!(state.frame_open(), "原 state 不受克隆影响");
    }

    /// **win=0 放置缓存：按帧清理，且同帧各段的槽都要留下**。
    ///
    /// 回归防线（实测过的一次真实性能事故）：`cache_z0_window` **每段各跑一次**，一段只
    /// 见到自己录制的槽。若在段收尾按"本段见到的集合"清理：
    /// ① 段 1 的槽被段 2 的清理删掉、段 2 的槽又被下帧段 1 删掉 ⇒ 每个槽永远命中不了、
    ///    每帧重镶嵌整个 win=0 几何（实测 `cache_miss` 恒 = 槽数、`collect` ≈ 0.6ms，
    ///    UI 帧时间翻倍）；
    /// ② 放置序 `place` 在每段都从 0 起，只用它做键还会让两段的同号槽**互相覆盖**。
    /// 故：键带段前缀 + 清理按帧（[`UiState::begin_frame`] 用上一帧的完整集合）。
    #[test]
    fn z0_slots_are_segment_scoped_and_pruned_once_per_frame() {
        let mut st = UiState::new();
        // 上一帧两个段各自的第 0 个放置（以及一个已经消失的陈旧槽）。
        st.z0_quads.insert((1, 0), (0x11, Vec::new()));
        st.z0_quads.insert((2, 0), (0x22, Vec::new()));
        st.z0_quads.insert((1, 9), (0x99, Vec::new()));
        st.frame_state.z0_seen = vec![(1, 0), (2, 0)];
        st.begin_frame();
        assert!(st.frame_state.z0_seen.is_empty(), "帧首清空上一帧的累计集");
        assert_eq!(st.z0_quads.len(), 2, "只清上一帧未出现的槽");
        assert!(
            st.z0_quads.contains_key(&(1, 0)) && st.z0_quads.contains_key(&(2, 0)),
            "同帧各段的槽都必须留下——按段清会删掉其中一段 ⇒ 那些槽每帧 miss"
        );
        assert!(!st.z0_quads.contains_key(&(1, 9)), "上一帧未出现的槽被清掉（防跨帧误复用）");
        // 键带段前缀：两段的同号放置是**不同槽**（否则互相覆盖、永远 miss）。
        assert_ne!((1u32, 0u32), (2u32, 0u32));
    }
}