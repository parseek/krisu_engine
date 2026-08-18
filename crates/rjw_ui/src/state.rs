//! UI 持久状态：`UiState`（应用持有）+ 控件返回给用户的状态视图（`ButtonState` / `CheckboxState`）。

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec2;
use rjw_2d_render::VertexP3U2C4;
use rjw_text::Buffer;
use rjw_transform::Rect;

use crate::proc::ProcTextures;

/// 控件文本 `Arc<Buffer>` 缓存容量：超出时整体清空重建（静态标签通常远小于此值）。
pub const TEXT_BUFFER_CACHE_CAP: usize = 128;

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
    /// 文本输入框光标位置（char 索引）。
    pub caret: usize,
    /// 文本选择锚点（char 索引；`Some` = 有选择，范围 = [min(anchor,caret), max)）。
    pub sel_anchor: Option<usize>,
    /// 单行输入框**水平滚动偏移**（逻辑像素）：超长文本时文本左移、光标跟随可见。
    pub text_scroll: f32,
    /// 多行输入框（TextArea）**垂直滚动偏移**（逻辑像素）。
    pub scroll_y: f32,
}

/// 滚动容器状态（`UiState.scrolls`，跨帧持久）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollState {
    /// 垂直滚动偏移（内容顶部相对可视区顶部；逻辑像素，已 clamp）。
    pub offset: f32,
    /// 内容总高（逻辑像素；clamp 上限 = max(0, content_h - view_h)）。
    pub content_h: f32,
}

/// UI 全局持久状态（由应用持有，跨帧复用；一个 `UiState` 可对应多个 `Ui`）。
#[derive(Clone, Debug, Default)]
pub struct UiState {
    /// 控件 ID → 持久状态。
    pub widgets: HashMap<String, WidgetState>,
    /// 当前持有焦点的控件 ID（文本输入框等）。
    pub focused: Option<String>,
    /// 单选组：组名 → 当前选中的控件 ID。
    pub radio_groups: HashMap<String, String>,
    /// 可拖拽面板 / 窗口：ID → 左上角位置（屏幕逻辑像素，跨帧持久）。
    pub panel_pos: HashMap<String, Vec2>,
    /// **窗口 z-order**：窗口 ID → z 值（越大越靠上；点击窗口置顶 = z+1）。
    /// 窗口命令按 z 升序绘制（焦点窗口最后画 → 最上层）。
    pub window_z: HashMap<String, u32>,
    /// **窗口矩形缓存**：窗口 z → 屏幕矩形（**逻辑像素**），跨帧保留。
    ///
    /// 用途：窗口**遮挡判定**（[`crate::hit::window_occluded`]）——控件命中测试时
    /// 检查鼠标下是否有更高 z 的窗口覆盖，修复"点击穿透"（背后窗口的控件在重叠
    /// 区域不响应）。录制窗口时更新（[`Ui::window_at`]），`finish` 末尾只保留
    /// **本帧录制过**的窗口（销毁/停用窗口自动清除，z 变化时旧条目随帧清理）。
    pub(crate) window_rects: HashMap<u32, Rect>,
    /// grid 容器：ID → 结算后的单元格尺寸（跨帧缓存，保证布局稳定）。
    pub grid_cells: HashMap<String, Vec2>,
    /// 控件文本排版缓存：`(文本, 字号位模式, 字体族, 换行宽度位模式, 版本)` → 共享 `Arc<Buffer>`。
    ///
    /// 用 [`rjw_text::CachePolicy::User`] 创建——**不推入 rjw_text 内部 LRU**
    /// （避免 UI 标签挤占其 128 容量），由本缓存自持；静态标签每帧命中零排版。
    /// 超出 [`TEXT_BUFFER_CACHE_CAP`] 时整体清空（简单策略，动态文本低频触发）。
    /// 
    /// 版本号用于强制刷新缓存（如行高计算方式变更），避免新旧缓存混用导致布局错乱。
    pub(crate) text_buffers: HashMap<(String, u32, Option<String>, u32, u8), Arc<Buffer>>,
    /// 帧计数（光标闪烁相位用）。
    pub frame: u64,
    /// 上一帧是否处于 IME 组合中（text_input 退格判定用）：
    /// 组合结束的那一帧，`Preedit("")` 事件先清空候选，随后退格键事件到达——
    /// 若只看当前帧候选会误判为"非组合"而执行本地退格（误删已有文本）。
    /// 组合中或刚结束的帧，退格/删除/方向键一律交给 IME 系统处理。
    pub(crate) ime_composing: bool,
    /// **窗口四边形缓存**：窗口 id → (内容签名, 按 (组, 纹理) 分组的**局部顶点**)。
    /// 组：`0` = 图形（白纹理 / 圆角 / 渐变 / 边框）、`1` = 文字（字形图集）。
    /// 窗口内容不变时复用（`finish` 按签名命中），**移动窗口只改变换、顶点不重建**；
    /// 任何内容变化（hover 变色、文字编辑等）都会使签名变化而自动重建。
    pub(crate) window_quads: HashMap<String, (u64, Vec<(u8, u64, Vec<VertexP3U2C4>)>)>,
    /// **诊断**：本帧**命中但被更高窗口遮挡而未响应**的控件次数
    /// （点击穿透拦截计数；`Ui::hit_abs` 累加，`begin_frame` 清零）。
    pub(crate) occluded_hits: u32,
    /// **诊断**：最近一次按下由哪个窗口接收（`finish::resolve_win_press` 写入；
    /// 即重叠点击时被置顶/可拖拽的**最上层**窗口）。跨帧保留直至下一次按下。
    pub(crate) last_press_window: Option<(String, u32)>,
    /// **程序化纹理缓存**（圆角矩形 / 渐变 / WHITE）：塞进动态 Atlas，跨帧复用。
    pub(crate) proc: ProcTextures,
    /// **滚动容器状态**：`scroll_at` 的 id → (偏移, 内容高)，跨帧持久。
    pub(crate) scrolls: HashMap<String, ScrollState>,
    /// **下拉框展开状态**：当前展开的 `combo` 的 id（`None` = 全部收起）。
    pub(crate) combo_open: Option<String>,
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

    /// 取（或创建）某控件的持久状态。
    pub fn widget(&mut self, id: &str) -> &mut WidgetState {
        self.widgets.entry(id.to_owned()).or_default()
    }

    /// 移除某个控件状态（控件消失/复用 ID 时）。
    pub fn remove(&mut self, id: &str) {
        self.widgets.remove(id);
        if self.focused.as_deref() == Some(id) {
            self.focused = None;
        }
        for group in self.radio_groups.values_mut() {
            if group == id {
                *group = String::new();
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
    /// 当前是否勾选（绘制用；checkbox 由用户维护，radio 由组内互斥决定）。
    pub(crate) checked: bool,
    /// 本帧发生了"切换"（点击且状态翻转）。
    pub(crate) toggled: bool,
    /// 本帧完成一次点击。
    pub(crate) clicked: bool,
}

impl CheckboxState {
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