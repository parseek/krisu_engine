//! UI 层（feature = `ui`）：**自洽的一层**——跨帧状态 + 主题 + 专用渲染器。
//!
//! 责任（对齐 `docs/API_DESIGN.md` §3 / §8.10）：`UiState`（跨帧持久）+ `Theme`
//! （最近一次 `Frame::ui` 传入）+ **专用 `Render2D`**（`SortMode::None`：UI 自行管理
//! 绘制顺序，靠提交顺序，不依赖渲染器排序）+ 最近一次的 `base_layer` / `scale` /
//! `debug_layout`（帧收尾视图复用）。
//!
//! 帧语义（**一帧 = 开场 + N 段 + 收尾**，见 `Frame::ui` / `Ctx::ui_end_frame`）：
//! - **开场**：本帧第一段 `Ui::begin(..).build()` 懒开场（帧号 +1 / 命中区翻页 /
//!   输入快照冻结 / 责任链种入）——应用会先在 `update` 里 `debug_inject_mouse(..)`，
//!   故开场必须留到第一段，不能放在 `Ctx::begin_update`；
//! - **段**（0..N）：每次 `Frame::ui(theme)` 开一段，`UiSession::drop` → `Ui::finish`
//!   （分桶 → 顶点 → 提交到本层的 `r2d`）；段序即绘制序（后一段压在前一段之上）；
//! - **收尾**：`Ctx::ui_end_frame()`（幂等；在 UI 队列被提交之前调用）→ 焦点导航与
//!   描边、光标定夺、统计写回、帧级暂存关场。

use rjw_2d_render::{Render2D, SortMode};
use rjw_render::RenderContext;
use rjw_ui::{Theme, Ui, UiState};

use crate::runtime::layers::ui_backend::Render2dUiBackend;

/// **一段 UI 录制**（`Frame::ui(theme)` 的返回值；RAII：析构即段收尾）。
///
/// 「ui anywhere」的落点：一帧可开任意多段、位置随意（世界绘制之前 / 之间 / 之后），
/// `&mut UiSession` 可直接透传给任意函数 / 模块（`Deref` / `DerefMut` 到 [`Ui`]）。
///
/// - **段存活期间** `Frame` 被借用 ⇒ 不能再 `f.draw()` / `f.text()` / `f.submit()`
///   （编译期拦住；段之间可以交错）；
/// - **析构**调用 [`Ui::finish`]（段收尾：分桶 → 顶点 → 提交到本层 `r2d`）；
/// - **帧收尾**（焦点导航 + 描边 / 光标 / 统计 / 复位）**不在这里**：由运行时在提交前
///   调一次 `Ctx::ui_end_frame()`（每帧一次，幂等）。
pub struct UiSession<'a> {
    ui: Ui<'a>,
    r2d: &'a mut Render2D,
}

impl<'a> UiSession<'a> {
    pub(crate) fn new(ui: Ui<'a>, r2d: &'a mut Render2D) -> Self {
        Self { ui, r2d }
    }

    /// 显式结束本段（等价于执行析构；便于"早结束早让出 `f`"的写法）。
    pub fn finish(self) {
        drop(self); // `Drop` 里统一收尾，避免两条路径语义漂移
    }
}

impl<'a> std::ops::Deref for UiSession<'a> {
    type Target = Ui<'a>;
    #[inline]
    fn deref(&self) -> &Ui<'a> {
        &self.ui
    }
}

impl<'a> std::ops::DerefMut for UiSession<'a> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Ui<'a> {
        &mut self.ui
    }
}

impl Drop for UiSession<'_> {
    fn drop(&mut self) {
        self.ui.finish(&mut Render2dUiBackend::new(self.r2d));
    }
}

/// 引擎持有的 UI 层（状态 + 主题 + 专用渲染器 + 最近一次录制配置）。
pub struct UiLayer {
    /// 跨帧持久状态（焦点 / 滚动 / 拖拽 / 输入内容 / 窗口尺寸…），**必须**跨帧复用。
    pub state: UiState,
    /// 最近一次 `Frame::ui(theme)` 传入的主题（帧收尾的焦点描边等复用它）。
    pub theme: Theme,
    /// **专用 UI 渲染器**：`SortMode::None`——UI 自行管理绘制顺序（见 `Ui::finish` 的
    /// 提交序说明），`SortMode::LayerAndStates` 会把圆角/渐变排到文字之后盖住文字。
    ///
    /// `None` = 尚未接入渲染上下文（`Ctx::headless` / 单元测试）：此时只能承载状态，
    /// 不能录制提交（与 `Ctx` 原来的 `Option<Render2D>` 同一语义）。
    pub(crate) r2d: Option<Render2D>,
    /// 最近一段的基层层级（帧收尾视图用）。
    pub(crate) base_layer: f64,
    /// 最近一段的 DPI 倍率（帧收尾视图的 Theme 预乘用）。
    pub(crate) scale: f32,
    /// 最近一段的调试布局开关（帧收尾视图用）。
    pub(crate) debug_layout: bool,
}

impl UiLayer {
    /// 建层（`Ctx::attach` 内；UI 渲染器在此创建并关闭排序）。
    pub(crate) fn new(render: &RenderContext) -> Self {
        let mut r2d = Render2D::new(render);
        r2d.sort(SortMode::None);
        Self {
            state: UiState::new(),
            theme: Theme::default(),
            r2d: Some(r2d),
            base_layer: 1.0e7,
            scale: 1.0,
            debug_layout: false,
        }
    }

    /// 记录本段配置（`Frame::ui` 每次调用；帧收尾视图复用最近一次）。
    pub(crate) fn remember_view_config(&mut self, theme: Theme, base_layer: f64, scale: f32, debug_layout: bool) {
        self.theme = theme;
        self.base_layer = base_layer;
        self.scale = scale;
        self.debug_layout = debug_layout;
    }
}

impl Default for UiLayer {
    /// 无渲染器可用的兜底（headless 测试 / 单元测试）：**不可提交**，仅承载状态与主题。
    /// 真实路径一律走 [`UiLayer::new`]。
    fn default() -> Self {
        Self {
            state: UiState::new(),
            theme: Theme::default(),
            r2d: None,
            base_layer: 1.0e7,
            scale: 1.0,
            debug_layout: false,
        }
    }
}
