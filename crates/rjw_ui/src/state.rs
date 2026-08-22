//! UI 持久状态：`UiState`（应用持有）+ 控件返回给用户的状态视图（`ButtonState` / `CheckboxState`）。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec2;
use rjw_2d_render::VertexP3U2C4;
use rjw_text::Buffer;
use rjw_transform::Rect;

use crate::id::IdAbsolute;
use crate::proc::ProcTextures;

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

/// UI 帧性能统计（`Ui::finish` 各阶段耗时，µs；debug/release 对比与优化决策用）。
/// 每次 `finish` 覆盖写入——示例里读到的是**上一帧**的统计值。
#[derive(Clone, Debug, Default)]
pub struct UiStats {
    /// 统计帧号（每次 `finish` 自增）。
    pub frame: u64,
    /// 本帧录制命令数（排序前队列长度）。
    pub cmd_count: u32,
    /// 本帧提交的窗口数（win > 0）。
    pub win_count: u32,
    /// 窗口顶点缓存命中 / 未命中次数。
    pub cache_hits: u32,
    pub cache_misses: u32,
    /// 队列排序 + 按窗口分组耗时（µs）。
    pub sort_us: f64,
    /// 窗口内容签名（摘要 + 全量哈希）耗时（µs）。
    pub sig_us: f64,
    /// 缓存未命中 → 顶点重建（collect_cmds）耗时（µs）。
    pub collect_us: f64,
    /// 缓存命中 → 提交列表组装（顶点克隆）耗时（µs）。
    pub clone_us: f64,
    /// 提交（ordered 排序 + add_quads）耗时（µs）。
    pub submit_us: f64,
    /// `Ui::finish` 总耗时（µs）。
    pub finish_us: f64,
    /// 整个 UI 帧（`begin` → `finish` 结束）耗时（µs）。
    pub ui_frame_us: f64,
}

/// UI 全局持久状态（由应用持有，跨帧复用；一个 `UiState` 可对应多个 `Ui`）。
#[derive(Clone, Debug, Default)]
pub struct UiState {
    /// 控件 **绝对 ID** → 持久状态。
    pub widgets: HashMap<IdAbsolute<'static>, WidgetState>,
    /// 当前持有焦点的控件 **绝对 ID**（文本输入框等）。
    pub focused: Option<IdAbsolute<'static>>,
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
    /// 区域不响应）。录制窗口时更新（[`Ui::window_at`]），`finish` 末尾只保留
    /// **本帧录制过**的窗口（销毁/停用窗口自动清除，z 变化时旧条目随帧清理）。
    pub(crate) window_rects: HashMap<u32, Rect>,
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
    /// 版本号用于强制刷新缓存（如行高计算方式变更），避免新旧缓存混用导致布局错乱。
    pub(crate) text_buffers: HashMap<(String, u32, Option<String>, u32, u8), (Arc<Buffer>, u64)>,
    /// 帧计数（光标闪烁相位用）。
    pub frame: u64,
    /// 上一帧是否处于 IME 组合中（text_input 退格判定用）：
    /// 组合结束的那一帧，`Preedit("")` 事件先清空候选，随后退格键事件到达——
    /// 若只看当前帧候选会误判为"非组合"而执行本地退格（误删已有文本）。
    /// 组合中或刚结束的帧，退格/删除/方向键一律交给 IME 系统处理。
    pub(crate) ime_composing: bool,
    /// **窗口四边形缓存**：窗口 **绝对 ID** → (内容签名, 按 **(元素序, 组, 纹理)** 分组的**局部顶点**)。
    /// 组：`0` = 图形（白纹理 / 圆角 / 渐变 / 边框）、`1` = 文字（字形图集）。
    /// 窗口内容不变时复用（`finish` 按**全量签名**命中），**移动窗口只改变换、顶点不重建**；
    /// 任何内容变化（hover 变色、点击按下、文字编辑、滚动等）都会使签名变化而自动重建。
    ///
    /// ⚠ 签名必须是**逐命令全量哈希**（含颜色 / 边框宽 / 圆角 / 对齐 / 光标 / 选择）——
    /// 轻量摘要曾漏掉颜色位，hover/click 变色被误判"内容未变" → 复用陈旧顶点，
    /// 窗口内交互效果不刷新（下拉框 / 背包 / 窗口按钮失效）。
    pub(crate) window_quads: HashMap<IdAbsolute<'static>, (u64, Vec<(u32, u8, u64, Vec<VertexP3U2C4>)>)>,
    /// **诊断**：本帧**命中但被更高窗口遮挡而未响应**的控件次数
    /// （点击穿透拦截计数；`Ui::hit_abs` 累加，`begin_frame` 清零）。
    pub(crate) occluded_hits: u32,
    /// **诊断**：最近一次按下由哪个窗口接收（`finish::resolve_win_press` 写入；
    /// 即重叠点击时被置顶/可拖拽的**最上层**窗口）。跨帧保留直至下一次按下。
    pub(crate) last_press_window: Option<(IdAbsolute<'static>, u32)>,
    /// **程序化纹理缓存**（圆角矩形 / 渐变 / WHITE）：塞进动态 Atlas，跨帧复用。
    pub(crate) proc: ProcTextures,
    /// **滚动容器状态**：`scroll_at` 的 **绝对 ID** → (偏移, 内容高)，跨帧持久。
    pub(crate) scrolls: HashMap<IdAbsolute<'static>, ScrollState>,
    /// **下拉框展开状态**：当前展开的 `combo` 的 **绝对 ID**（`None` = 全部收起）。
    pub(crate) combo_open: Option<IdAbsolute<'static>>,
    /// **固定宽窗口的宽度**（`window_at_w` 鼠标缩放：**绝对 ID** → 逻辑宽度，跨帧持久）。
    pub(crate) window_widths: HashMap<IdAbsolute<'static>, f32>,
    /// **窗口结算尺寸**（**绝对 ID** → 物理尺寸，跨帧持久）。clamp（`WindowClamp`）
    /// 用——按 **id** 而非窗口 z 索引：**点击置顶 z+1 后尺寸不丢** → clamp 边界稳定
    /// （消除"按下即跳变"）；`window_rects[z]` 仅作兜底。
    pub(crate) window_sizes: HashMap<IdAbsolute<'static>, Vec2>,
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

    /// 进入新的一帧（内部自动调用，应用无需手动）。
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        // 遮挡拦截计数按帧清零（诊断机制：读的是"上一帧"的累计值）。
        self.occluded_hits = 0;
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
        self.radio_groups.clear();
        self.grid_cells.clear();
        self.panel_pos.clear();
        self.window_z.clear();
        self.window_rects.clear();
        self.text_buffers.clear();
        self.frame = 0;
        self.ime_composing = false;
        self.window_quads.clear();
        self.occluded_hits = 0;
        self.last_press_window = None;
        self.scrolls.clear();
        self.combo_open = None;
        self.sizes.clear();
        self.window_fx.clear();
        self.stats = UiStats::default();
    }

    /// 是否正在**捕获键盘输入**（有文本输入框持有焦点）。
    ///
    /// 应用应在处理自己的按键逻辑（如 `R` 重置、`Esc` 退出）前检查并跳过：
    /// ```no_run
    /// # let ui_state: rjw_ui::UiState = rjw_ui::UiState::new();
    /// if !ui_state.capturing_text() {
    ///     // 处理游戏/应用快捷键……
    /// }
    /// ```
    #[inline]
    pub fn capturing_text(&self) -> bool {
        self.focused.is_some()
    }

    /// 光标是否处于"亮"相位（每 30 帧切换）。
    pub fn caret_blink_on(&self) -> bool {
        (self.frame / 30) % 2 == 0
    }

    /// **诊断**：上一帧**命中但被更高窗口遮挡而未响应**的控件次数
    /// （点击穿透拦截计数——大于 0 说明鼠标下有窗口叠放、背后控件被正确抑制）。
    #[inline]
    pub fn occluded_hits(&self) -> u32 {
        self.occluded_hits
    }

    /// **诊断**：最近一次按下由哪个窗口接收（`(id, z)`；重叠点击时置顶/可拖拽的
    /// **最上层**窗口）。跨帧保留至下一次按下。
    #[inline]
    pub fn last_press_window(&self) -> Option<(&str, u32)> {
        self.last_press_window
            .as_ref()
            .map(|(id, z)| (id.as_str(), *z))
    }
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