//! **绘制器**（`Painter`）：一个**独立于 [`Ui`](crate::Ui) 的组件**。
//!
//! # 为什么独立
//!
//! 绘制器**只拥有录制状态**（[`DrawQueue`]：命令队列 + 播放头）与 API 边界的尺寸换算
//! （`scale`）——**不引用 `Ui`、不需要字体图集、不需要 GPU**。因此：
//!
//! - 「画出来的是哪几条命令」可以在**普通单测**里断言：`Painter::new(1.0)` → 画 →
//!   读 [`Painter::commands`]（本仓历史上所有绘制 bug 都只能靠示例截图发现，
//!   因为 `Ui` 必须有字形图集才能构造）；
//! - 未来离屏 / 自定义后端可以直接拿一个 `Painter` 出命令，不必造一个 `Ui`。
//!
//! # 用法：一个绘制块一个 painter
//!
//! ```no_run
//! # use rjw_transform::Rect;
//! # use rjw_ui::Ui;
//! # fn demo(ui: &mut Ui, rect: Rect) {
//! let mut p = ui.painter();
//! p.panel(rect, rjw_color::Color::BLACK, rjw_color::Color::WHITE, 1.0, 4.0);
//! p.text(rect, "标签", 14.0, rjw_color::Color::WHITE, None,
//!        rjw_ui::TextAlign::Center, rjw_ui::draw::TextVAlign::Center, None, None);
//! # }
//! ```
//!
//! **量尺寸 / 命中 / 申请布局要与绘制交错时，分块取 painter**：
//! `ui.painter()` 借的是 `&mut Ui`，该借用在 painter 存活期内独占（任何 `ui.text_size` /
//! `ui.hit_abs` 都需要 `&mut self`）。所以顺序是「先量 → 再画 → 再量」，
//! 而不是像 egui 那样（`Painter` 持 `Context` 克隆）随便交错——本引擎**不用
//! `Arc<Mutex<_>>` 换交错能力**，以保住"零堆分配、无锁"的既有约定。
//!
//! # 元素序（`elem`）约定
//!
//! 原语默认**逐条**取 [`DrawQueue::elem_hint`]（= 当前 `seq + 1`），与旧的
//! `Ui::push_*` 逐位一致：同一元素内按 `seq` 排序，**背景/图形先于文字**。
//! ⚠ 因此**不要**把一个 painter 的 `elem` 想成"冻结的元素号"：需要"装饰压住自家已有
//! 内容"（拖拽柄 / 展开箭头 / 分隔线）时，**再取一次 `ui.painter()`**——此时
//! `elem_hint()` 已经更大，装饰自然排在被压内容之后。
//!
//! 容器装饰（阴影 / 窗口背景 / 边框）用 `elem = 0`：见 [`Painter::draw`] /
//! [`Painter::panel_elem`] / [`Painter::panel_img_elem`]。

mod debug;
mod prim;
mod queue;
mod text;

use rjw_transform::Rect;

pub use queue::DrawQueue;

use crate::draw::DrawKind;

/// **绘制器**：拥有录制状态（[`DrawQueue`]）+ 尺寸换算（`scale`）。
///
/// 由 [`Ui`](crate::Ui) 持有（`Ui::painter()` 借出），也可 [`Painter::new`] 独立构造
/// （单测 / 离屏工具）。
pub struct Painter {
    /// 录制状态（命令队列 + 播放头）。
    pub(crate) q: DrawQueue,
    /// DPI scale factor：**仅 API 边界** [`Size`](crate::Size) /
    /// [`Position`](crate::Position) 的 Logical→Physical 换算用（与 `Ui::scale` 同源，
    /// 由 `Ui` 开场写入）。
    scale: f32,
}

impl Painter {
    /// 独立构造（`scale` = DPI 换算用；单测可传 `1.0`）。
    #[inline]
    pub fn new(scale: f32) -> Self {
        Self { q: DrawQueue::default(), scale }
    }

    /// 本帧已录制的**内容命令**（只读；测试 / 离屏工具用）。
    #[inline]
    pub fn commands(&self) -> &[crate::draw::UiDraw] {
        self.q.commands()
    }

    /// 本帧已录制的**调试命令**（只读）。
    #[inline]
    pub fn debug_commands(&self) -> &[crate::draw::UiDraw] {
        self.q.debug_commands()
    }

    /// **当前强制裁剪层**（绝对逻辑屏幕坐标；`None` = 不裁剪）。
    ///
    /// 这**已经包含**当前所在的 Clip 沙箱（`Ui::view_at(ViewMode::Clip)`）/ ScrollView
    /// 可视区 / 严格窗口内容裁剪——`view_at` / `scroll_at` 进入沙箱时就是把这一层写进
    /// [`DrawQueue`]。因此**沙箱里的控件什么都不用做**：`ui.painter()` 拿到的就是
    /// "被裁的 painter"，录出的每条命令自带该裁剪（`UiDraw.clip`）。
    ///
    /// 只有"要一层**与当前不同**的裁剪"时才需要 [`Painter::clipped`]
    /// （更窄：本控件自己再裁一刀；或 `None`：主动不裁）。
    #[inline]
    pub fn clip(&self) -> Option<Rect> {
        self.q.clip
    }

    /// DPI scale factor（API 边界换算用）。
    #[inline]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// **元素序提示**：当前录制位置的下一个元素序（`seq + 1`）。
    ///
    /// 与 [`DrawQueue::elem_hint`] 同一语义；原语默认就用它。
    #[inline]
    pub fn elem_hint(&self) -> u32 {
        self.q.elem_hint()
    }

    /// **闭包作用域的裁剪覆盖**：块内命令用 `clip`，块外恢复原裁剪层。
    ///
    /// 用途**只有"覆盖"**两种情况：
    /// - 更窄：本控件把内容裁到自己的框内，但**不改** `Ui` 的强制裁剪层
    ///   （改它会连带子控件 / 兄弟控件）；
    /// - [`None`]：主动不裁（跳过外层沙箱）。
    ///
    /// ⚠ **已经在 Clip 沙箱里时不需要它**：沙箱的裁剪就在 [`Painter::clip`] /
    /// 每条命令的 `UiDraw.clip` 上（`view_at` 进入沙箱时写进 [`DrawQueue`]）。
    /// 零成本（只存一个 `Option<Rect>`），无 RAII guard。
    pub fn clipped<R>(&mut self, clip: Option<Rect>, f: impl FnOnce(&mut Painter) -> R) -> R {
        let saved = self.q.clip;
        self.q.clip = clip;
        let out = f(self);
        self.q.clip = saved;
        out
    }

    /// **录制一条绘制命令**（底层入口；`elem` 由调用方显式给）。
    ///
    /// `elem = 0` = 容器装饰层（画在本容器**所有元素之下**，如窗口背景 / 边框）；
    /// 控件自绘装饰请传 [`Painter::elem_hint`]（或在需要"压在自家已有内容之上"时
    /// **重新取一个 painter** 用默认值）。
    #[inline]
    pub fn draw(&mut self, kind: DrawKind, rect: Rect, elem: u32) {
        self.q.push(kind, rect, elem);
    }

    /// 录制一条命令，`elem` = [`Painter::elem_hint`]（原语的默认行为）。
    #[inline]
    pub(crate) fn draw_hint(&mut self, kind: DrawKind, rect: Rect) {
        let elem = self.q.elem_hint();
        self.q.push(kind, rect, elem);
    }

    /// 推进并返回全局序号（`seq`；同深度内排序键）。
    #[inline]
    pub(crate) fn next_seq(&mut self) -> u32 {
        self.q.next_seq()
    }
}
