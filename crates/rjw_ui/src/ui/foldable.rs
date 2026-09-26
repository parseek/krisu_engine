//! **可收缩区块**（[`Ui::foldable`](crate::Ui::foldable) / [`Ui::foldable_custom`] /
//! [`Foldable`]）：一行标题栏 + 可折叠正文。
//!
//! # 维护者笔记（改这个文件前先读）
//!
//! 1. **标题行是"一整格"**：高 [`Theme::row_h`](crate::Theme::row_h)、宽尽量铺满当前容器
//!    的内容宽（[`fold_header_w`]：可用宽 → **同容器已排布内容的最宽宽** → 兜底 120）。
//!    第二条兜底是必需的：自动宽窗口**首帧**拿不到 `avail_w`（`Frame::pass_through_w`
//!    还没值），只按"图标 + 文字实测宽"排会缩成半截（历史现象："标题行缩在半截、与上下
//!    几行不对齐"），自定义标题更会退化成 `pad*2 + icon`（闭包里的控件整排溢出）。
//! 2. **翻转判据只有"按下边沿 + 命中"与"键盘 Enter / Space"**（[`fold_should_toggle`]）：
//!    **不要**再加 [`Response::clicked`] —— 它在**释放帧**成立，而按下边沿在**按下帧**，
//!    两者都判会让"一次点击翻两次"（脚本化点击恰好 t 帧按下 / t+1 帧释放 ⇒ 净效果为零，
//!    表现为"有些区块点不开"）。
//! 3. **折叠 = 正文完全不录制**：不占高、不参与布局、不进命中表、不产生顶点。正文内部的
//!    跨帧状态（滚动偏移 / 输入内容 / 焦点）**一律不动** ⇒ 展开回来还是原样。
//! 4. **正文是"当前容器的子项"**（不另开 frame，与 [`Ui::namespace`](crate::Ui::namespace)
//!    同一手法）：`container_scope` + `with_id` 保证"标题 → 正文"之间不多推进一个 `gap`、
//!    且正文里的同名控件互不干扰（[`crate::DrawQueue::placement_locked`] 的由来）。
//! 5. **正文缩进走"光标 + 内容最大宽"**（[`FoldableStyle::body_indent`]）：正文录制期间把
//!    当前 frame 的光标 `x` 与本容器内容最大宽一起右移，录完还原（`y` 保留推进量）。
//!    这样命令 / 命中区 / `last_interact_rect` 全部由光标与 `abs_base` 派生 ⇒ **三者天然
//!    一致**；换成"事后整体平移命令"就会让命中区差一个 `indent`（"看得见点不着"）。
//! 6. **标题栏里的控件自己认领按下**（`Sense::DRAG` 的 `claim_press`）：所以点标题里的
//!    勾选框 / 按钮不会连带折叠，也**不会**把这次按下当成外层窗口的拖动基准。
//! 7. 纯几何 / 判据都抽成本模块的 `fold_*` 自由函数：本仓单测不构造 `Ui`（没有字形图集与
//!    GPU），能测的只有它们。

use super::*;
use super::namespace::namespace_size;

use rjw_color::Color;
use rjw_transform::Rect;

use crate::draw::{Gradient, Icon, Position, Size, TextAlign, TextVAlign};
use crate::focus::FocusKind;
use crate::layout::Child;
use crate::style::FoldableStyle;
use crate::widgets::{Response, Sense};

/// **没有任何宽度线索时的标题行兜底宽**（物理像素）——与 `divider` 的兜底同源。
pub const HEADER_FALLBACK_W: f32 = 120.0;

/// **折叠态**（三角图标）：收起 ⇒ ▶、展开 ⇒ ▼（树控件惯例）。
const ICON_SHUT: Icon = Icon::ChevronRight;
/// 见 [`ICON_SHUT`]。
const ICON_OPEN: Icon = Icon::ChevronDown;

// ─── 纯函数（判据 / 几何；可单测）─────────────────────────────────

/// **本帧是否折叠**（纯函数，可单测）：表里有记录就用它（**明确态**），没有就用 `open`
/// 默认值（`open = false` = 默认折叠）。
///
/// `recorded` 来自 [`UiState::folded`](crate::UiState::folded)（`None` = 该区块**首次**录制）；
/// [`Foldable::show`] 在首帧把结果**落盘**，于是 [`UiState::is_folded`](crate::UiState::is_folded)
/// 与画面从第一帧起就同口径。
#[inline]
pub fn fold_state(recorded: Option<bool>, open: bool) -> bool {
    recorded.unwrap_or(!open)
}

/// **本帧是否翻转折叠态**（纯函数，可单测）：按下边沿落在标题行上（鼠标）**或**
/// `Enter` / `Space` 激活（键盘）。
///
/// ⚠ 刻意**不**接收 [`Response::clicked`]：它在释放帧成立 ⇒ 与按下边沿叠加会让一次点击
/// 翻两次（见模块维护者笔记 2）。
#[inline]
pub fn fold_should_toggle(pressed_edge_on_header: bool, key_click: bool) -> bool {
    pressed_edge_on_header || key_click
}

/// **标题行宽度**（纯函数，可单测）：可用宽 → 同容器**已排布内容的最宽宽** → `fallback`。
///
/// 三条的来源与理由见模块维护者笔记 1；`fallback` 只在"容器里除了本区块还没有任何内容"
/// 时生效（首个控件的首帧）。
#[inline]
pub fn fold_header_w(avail: Option<f32>, widest_child: f32, fallback: f32) -> f32 {
    avail
        .filter(|w| *w > 0.0)
        .or_else(|| (widest_child > 0.0).then_some(widest_child))
        .unwrap_or(fallback)
        .max(0.0)
}

/// **三角图标的方框**（纯函数，可单测）：左边距 `pad_x` 起、边长 `icon_h`、垂直居中
/// （`icon_at` 内部按 `min(w, h)` 居中等比 ⇒ 方框恒等比，图标永不形变）。
#[inline]
pub fn fold_icon_rect(header: Rect, style: &FoldableStyle) -> Rect {
    let side = style.icon_h.min(header.h).max(0.0);
    Rect::new(
        header.x + style.pad_x,
        header.y + (header.h - side) * 0.5,
        side,
        side,
    )
}

/// **标题文本区**（纯函数，可单测）：`pad_x + icon_w` 起、到右内边距为止（窄容器下夹到
/// `0` 宽，不产生负宽 —— 负宽会让文本绘制与省略号测量算出垃圾）。
#[inline]
pub fn fold_label_rect(header: Rect, style: &FoldableStyle) -> Rect {
    let x = header.x + style.pad_x + style.icon_w;
    Rect::new(x, header.y, (header.x + header.w - style.pad_x - x).max(0.0), header.h)
}

/// **正文左缘竖引导线**（纯函数，可单测）：`body_top` / `body_h` 为正文在**当前容器局部**
/// 空间的顶部与高度；线宽 [`FoldableStyle::guide_w`]，上下各延伸
/// [`FoldableStyle::guide_tail`]（让它看起来是"从标题行拉下来的括号"）。
///
/// `guide_w <= 0`（关掉）或正文为空 ⇒ [`Rect::ZERO`]（调用方据此跳过绘制）。
#[inline]
pub fn fold_guide_rect(body_top: f32, body_h: f32, style: &FoldableStyle) -> Rect {
    if style.guide_w <= 0.0 || body_h <= 0.0 {
        return Rect::ZERO;
    }
    let w = style.guide_w;
    Rect::new(
        ((style.body_indent - w) * 0.5).max(0.0),
        body_top - style.guide_tail,
        w,
        body_h + style.guide_tail * 2.0,
    )
}

// ─── 结果 ────────────────────────────────────────────────────────

/// **[`Foldable::show`] 的结算结果**（诊断 / 脚本化测试用）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoldState {
    /// 标题行的交互响应（`rect` = 标题行矩形，`hovered` / `pressed` / `clicked`）。
    pub header: Response,
    /// **录制开头**读到的折叠态（点标题只写状态、下一帧才改变布局 ⇒ 与"本帧画出来的"
    /// 一致，而不是与"刚点完的状态"一致）。
    pub folded: bool,
    /// 标题行矩形（当前容器局部坐标，物理像素）。
    pub header_rect: Rect,
    /// 正文结算高度（物理像素；折叠 / 空正文 = `0`）。
    pub body_h: f32,
}

// ─── builder ─────────────────────────────────────────────────────

/// **标题内容**：普通文本，或在"长宽已确定的装饰容器"里跑的自定义闭包。
enum Title<'ui> {
    Text(String),
    /// 闭包拿到 [`PackEntry`]（实现了 [`UiAdd`]）——里面可放任意控件
    /// （`label` / `checkbox_mut` / `button` / `add` …）；它们自己认领按下 ⇒ 点它们不会
    /// 连带折叠标题。**不参与**本容器尺寸结算（[`Ui::ornament_at`] 的装饰语义）。
    Custom(Box<dyn FnOnce(&mut PackEntry<'_, '_>) + 'ui>),
}

/// **可收缩区块 builder**（[`Ui::foldable`](crate::Ui::foldable) 返回；容器内用
/// [`UiAdd::foldable`](crate::ui::UiAdd::foldable) / [`UiAdd::foldable_custom`](crate::ui::UiAdd::foldable_custom)）。
///
/// ```no_run
/// # use rjw_ui::{Ui, UiAdd};
/// # fn demo(ui: &mut Ui, kw: &mut String) {
/// ui.foldable("perf", "性能统计").show(|ui| {
///     ui.label("FPS 60");                  // 折叠时这段**完全不录制**
/// });
/// ui.foldable("advanced", "高级").open(true).show(|ui| { /* 首次就展开 */ });
/// ui.foldable_custom("filters", |t| {
///     t.label("过滤");                      // 标题里放控件（自己认领按下）
/// }).show(|ui| { ui.text_input("kw", kw); });
/// # }
/// ```
pub struct Foldable<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
    /// 区块的相对 ID（同时是正文的 ID 命名空间段）。
    id: String,
    title: Title<'ui>,
    /// **首次**渲染的默认态（`None` = 默认折叠，即 `false`）——一旦该区块被点过
    /// （或应用 `set_folded` 过），它就由 [`UiState::folded`](crate::UiState::folded) 说了算。
    open: Option<bool>,
}

impl<'ui, 'a> Foldable<'ui, 'a> {
    /// 文本标题的区块（[`Ui::foldable`](crate::Ui::foldable) / [`UiAdd::foldable`] 用）。
    pub fn new(ui: &'ui mut Ui<'a>, id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            ui,
            id: id.into(),
            title: Title::Text(label.into()),
            open: None,
        }
    }

    /// **自定义标题**的区块（首参是**裸 [`Ui`]**；容器闭包里请用
    /// [`UiAdd::foldable_custom`](crate::ui::UiAdd::foldable_custom)）。
    ///
    /// 闭包在"长宽已确定的装饰容器"（[`Ui::ornament_at`] 的装饰语义，**不** `note_content`）
    /// 里跑 ⇒ 标题内容绝不反过来撑大容器；控件 ID 仍在本区块的命名空间里。
    pub fn custom(
        ui: &'ui mut Ui<'a>,
        id: impl Into<String>,
        title: impl FnOnce(&mut PackEntry<'_, '_>) + 'ui,
    ) -> Self {
        Self {
            ui,
            id: id.into(),
            title: Title::Custom(Box::new(title)),
            open: None,
        }
    }

    /// **首次**渲染是否展开（不调 = 默认折叠）。**只影响从未被点过的区块** —— 之后由
    /// [`UiState::folded`](crate::UiState::folded) 说了算（点标题翻转即写它）。
    pub fn open(mut self, open: bool) -> Self {
        self.open = Some(open);
        self
    }

    /// 首次渲染**折叠**（= 默认；语义同 `.open(false)`）。
    pub fn closed(self) -> Self {
        self.open(false)
    }

    /// **录制本区块**：标题行 + （未折叠时的）正文，返回 [`FoldState`]。
    ///
    /// - 正文录在**本区块的 ID 命名空间**里（`id/` 前缀）⇒ 两个区块里的同名控件互不干扰；
    /// - 折叠时 `body` **根本不执行**（正文一个命令、一个状态键都不产生）；
    /// - 返回的 `folded` 是**录制开头**读到的值（点标题当帧几何不变、下一帧才生效）。
    pub fn show(self, body: impl FnOnce(&mut PackEntry<'_, '_>)) -> FoldState {
        let Self { ui, id, title, open } = self;
        let style = ui.theme.foldable.clone();
        let abs = ui.id_for(id.as_str());

        // ── ① 折叠态：本帧几何在录制开头就定（点标题只写状态、下一帧才改变布局 —— 与
        //    窗口 ⌃ 同一口径）；首帧把 `open` 默认态**落盘** ⇒ `UiState::is_folded` 从
        //    第一帧起与画面同口径。 ──────────────────────────────────────────
        let recorded = ui.state.folded.get(abs.as_str()).copied();
        let folded = fold_state(recorded, open.unwrap_or(false));
        if recorded.is_none() {
            ui.state.folded.insert(abs.to_static(), folded);
        }

        // ── ② 标题行：一整格（占光标、撑大父级宽）──────────────────────────
        let row_h = ui.theme.row_h;
        let widest = ui.frames.last().map(|f| f.max_child_w()).unwrap_or(0.0);
        let header_w = fold_header_w(ui.avail_w(), widest, HEADER_FALLBACK_W);
        // **标题行铺满可用宽、但不决定容器宽**（`Child::Fill`）：标题行是**整格装饰**
        // （整行可点 / hover 高亮），让它参与宽度结算会在**带 `vscroll` 的自动宽窗口**里形成
        // **正反馈锁定**：视口宽首帧取"屏幕剩余宽"（`window_impl` 里为防视口塌成 1px 的引导值）
        // ⇒ 标题行按它铺满 ⇒ 内容宽 = 视口宽 ⇒ 下一帧视口仍按内容反推 ⇒ **窗口一打开就被撑到
        // 屏幕大小**（用户实测："分割线会默认水平撑开到屏幕大小 …… 而是其他内容有多少就该多宽"，
        // 分割线与标题行是同一条链路上的两个用户）。`Fill` ⇒ 容器宽由**其他内容**决定，
        // 标题行照样铺满（高度照常参与，见 `Child` 文档）。
        let header_rect = ui.child_rect(header_w, row_h, Child::Fill);
        // `Sense::DRAG`：按下边沿即 `claim_press` ⇒ 点标题不会把这次按下当成外层
        // 窗口 / 面板的拖动基准；同时进焦点链（`Tab` 可到、`Enter` / `Space` 激活）。
        let btn = ui.mouse_left();
        let header = ui.interact(&abs, header_rect, Sense::DRAG.focus(FocusKind::Button));
        let key_click = ui.key_click(&abs, FocusKind::Button);
        if fold_should_toggle(header.hovered && btn.down_edge(), key_click) {
            ui.state.toggle_folded(abs.as_str());
        }
        // 标题行被裁剪层完全剔除 ⇒ 省掉它的几何（**正文照录**：布局不随裁剪变化）。
        let mut header_rect = header_rect;
        if !header.culled {
            // 标题内容也录在**本区块的命名空间**里 ⇒ 自定义标题里的控件 id 带 `id/` 前缀，
            // 与正文同一口径（标题行自身的 id 在 `with_id` **之前**解析 ⇒ 仍是区块 id）。
            let mut title_h = header_rect.h;
            ui.with_id(id.as_str(), |ui| {
                title_h = draw_header(ui, title, header_rect, folded, &header, &style);
            });
            // **多行标题**（自定义标题里放了 Row / 多行内容）：标题行按内容加高，并把父容器
            // 光标补推同样多（正文因此从其下方开始，且不再多一个 `gap`），整块记进内容包围盒
            // —— 否则父容器的高度装不下它（自动宽窗口会被"标题只有一行高"骗过去）。
            // ⚠ 命中区仍是**首行**（`interact` 在内容之前、此时高度还未知）：点首行翻转。
            if title_h > header_rect.h {
                let extra = title_h - header_rect.h;
                header_rect.h = title_h;
                if let Some(f) = ui.frames.last_mut() {
                    f.cursor.y += extra;
                }
                // 与 `Child::Fill` 同口径：**只把高度并进容器**（宽记 0）——否则这一记会把
                // 标题行的宽带进容器尺寸，等于把 `Fill` 的语义又抵消掉。
                ui.note_placed(Rect::new(header_rect.x, header_rect.y, 0.0, header_rect.h));
            }
        }

        // ── ③ 正文：折叠 ⇒ 完全不录制 ────────────────────────────────────
        let mut body_h = 0.0;
        if !folded {
            let gap = ui.theme.gap;
            let before = ui.cursor_pos();
            // 缩进：光标 x + 本容器内容最大宽一起右移（见模块维护者笔记 5）。
            let saved = push_body_indent(ui, style.body_indent);
            ui.container_scope(|ui| {
                ui.with_id(id.as_str(), |ui| {
                    body(&mut PackEntry::new(ui));
                });
            });
            pop_body_indent(ui, saved);
            let after = ui.cursor_pos();
            body_h = namespace_size(before, after, gap).y;
            // 归属提示：左侧竖引导线 + （可选）上下端"阴影色 → 全透明"渐隐。
            // 都在正文录完后才知道高度 ⇒ 命令排在正文之后（`elem` 更大，压在正文之上）。
            if !header.culled && body_h > 0.0 {
                draw_body_hints(ui, before.y, body_h, header_rect.w, &style);
            }
        }

        trace_fold(&id, folded, header_rect, body_h, &style, &header);
        FoldState {
            header,
            folded,
            header_rect,
            body_h,
        }
    }
}

impl<'a> Ui<'a> {
    /// **可收缩区块**（文本标题）：返回 [`Foldable`] builder，链式设置 `.open(bool)` /
    /// `.closed()` 后以 `.show(|ui| …)` 录制正文（返回 [`FoldState`]）。
    ///
    /// ```no_run
    /// # use rjw_ui::{Ui, UiAdd};
    /// # let mut ui: Ui = todo!(); let mut kw = String::new();
    /// ui.foldable("perf", "性能统计").show(|ui| {
    ///     ui.label("FPS 60");                 // 折叠时**完全不录制**
    /// });
    /// ui.foldable("advanced", "高级").open(true).show(|ui| { /* 首次就展开 */ });
    /// ```
    ///
    /// - 标题行 **= 一整行**（高 [`Theme::row_h`]、占满可用宽：▶ / ▼ + 文本）；
    ///   **点它即翻转**（当帧几何不变、**下一帧**生效）；`Tab` 可聚焦、`Enter` / `Space` 翻转；
    /// - **默认折叠**（首次只见标题行）；`.open(true)` 改首次展开（只影响从未被点过的区块）；
    /// - **折叠状态引擎托管**（[`UiState::folded`](crate::UiState::folded)）：应用不必多一个
    ///   `bool`，要读 / 改用 [`UiState::is_folded`](crate::UiState::is_folded) /
    ///   [`UiState::set_folded`](crate::UiState::set_folded) /
    ///   [`UiState::toggle_folded`](crate::UiState::toggle_folded)；
    /// - 样式取 [`Theme::foldable`](crate::Theme::foldable)（[`FoldableStyle`]）。
    pub fn foldable<'s>(&'s mut self, id: &str, label: &str) -> Foldable<'s, 'a> {
        Foldable::new(self, id, label)
    }

    /// **可收缩区块（自定义标题）**：标题内容交给闭包（拿到实现了 [`UiAdd`] 的
    /// [`PackEntry`]）——里面可放任意控件，它们自己认领按下 ⇒ 点它们**不会**连带折叠。
    ///
    /// ```no_run
    /// # use rjw_ui::{Ui, UiAdd};
    /// # let mut ui: Ui = todo!(); let mut kw = String::new(); let mut all = false;
    /// ui.foldable_custom("filters", |t| {
    ///     t.label("过滤");
    ///     t.checkbox_mut("all", "全选", &mut all);
    /// }).show(|ui| { ui.text_input("kw", &mut kw); });
    /// ```
    pub fn foldable_custom<'s>(
        &'s mut self,
        id: &str,
        title: impl FnOnce(&mut PackEntry<'_, '_>) + 's,
    ) -> Foldable<'s, 'a> {
        Foldable::custom(self, id, title)
    }
}

// ─── 绘制 / 内部辅助 ─────────────────────────────────────────────

/// 标题行：▶ / ▼ 图标 + 文本**或**自定义内容 + 底（常态透明 / 悬停 / 按下）。
///
/// **返回标题内容的自然高度**（文本标题恒 = `header.h`；自定义标题可能多行 ⇒ 调用方据此
/// 把标题行矩形加高）。两条硬约定：
///
/// 1. **自定义内容排在"文本区"里**（[`fold_label_rect`]：`pad_x + icon_w` 之后）——
///    与文本标题同一个 x。从标题行**左缘**起排会压住三角图标（历史形态：标题里的
///    勾选框盖住 ▶）；
/// 2. **底最后画、且 `elem = 0`**：自定义标题的高度要等闭包跑完才知道，底只能后画；
///    `elem = 0` = 本容器装饰层 ⇒ 它仍垫在本容器**所有元素之下**（晚录不等于压在上面）。
fn draw_header(
    ui: &mut Ui<'_>,
    title: Title<'_>,
    header: Rect,
    folded: bool,
    resp: &Response,
    style: &FoldableStyle,
) -> f32 {
    // 图标：垂直居中于**首行**（多行标题下它仍在第一行，不会飘到整块中间）。
    let icon_rect = fold_icon_rect(header, style);
    if icon_rect.w > 0.0 && icon_rect.h > 0.0 {
        let icon = if folded { ICON_SHUT } else { ICON_OPEN };
        ui.icon_at(
            Position::Physical(icon_rect.min()),
            Size::Physical(icon_rect.size()),
            icon,
            style.mark,
        );
    }
    let area = fold_label_rect(header, style);
    let content_h = match title {
        Title::Text(text) => {
            if area.w > 0.0 {
                // 超宽 ⇒ "…"省略（内容自洽：宽 = 分配宽，不撑大容器、不溢出）。
                let ellipsized =
                    ui.ellipsized(&text, style.font_size, style.font_family.as_deref(), area.w);
                let draw: &str = ellipsized.as_deref().unwrap_or(&text);
                ui.push_text_rect(
                    area,
                    draw,
                    style.font_size,
                    style.fg,
                    style.font_family.clone(),
                    TextAlign::Left,
                    TextVAlign::Center,
                    None,
                    None,
                );
            }
            header.h
        }
        // 装饰容器（固定宽 = 文本区宽、**高度自然**）：里面放 Row / 多行内容都会撑开
        // （"标题可以视作一个标准容器"）；内容**不参与**父容器尺寸结算。
        Title::Custom(f) => ui.ornament_entry_natural_h(area, f).max(header.h),
    };
    let bg_rect = Rect::new(header.x, header.y, header.w, content_h);
    ui.push_panel_like(
        bg_rect,
        style.pick_bg(resp.pressed, resp.hovered),
        style.border,
        style.border_w,
        style.radius,
        0,
    );
    content_h
}

/// 正文的**归属提示**：左侧竖引导线 + （可选）上下端渐隐。
fn draw_body_hints(
    ui: &mut Ui<'_>,
    body_top: f32,
    body_h: f32,
    content_w: f32,
    style: &FoldableStyle,
) {
    let guide = fold_guide_rect(body_top, body_h, style);
    if guide.w > 0.0 && guide.h > 0.0 {
        ui.push_solid_rect(guide, style.guide);
    }
    // 渐隐（`fade_h > 0` 才画；默认关闭）：各一条**零纹理**的顶点色渐变
    // （"整个收缩范围的上下两端，由阴影色渐变到完全透明"）。
    if style.fade_h > 0.0 {
        let h = style.fade_h.min(body_h);
        let w = content_w.max(0.0);
        if w > 0.0 && h > 0.0 {
            let top = Rect::new(0.0, body_top, w, h);
            ui.painter().gradient_at(
                Position::Physical(top.min()),
                Size::Physical(top.size()),
                Gradient::vertical(style.fade, Color::TRANSPARENT),
            );
            let bottom = Rect::new(0.0, body_top + body_h - h, w, h);
            ui.painter().gradient_at(
                Position::Physical(bottom.min()),
                Size::Physical(bottom.size()),
                Gradient::vertical(Color::TRANSPARENT, style.fade),
            );
        }
    }
}

/// **进入正文缩进**（见模块维护者笔记 5）：当前 frame 的光标 `x` 右移 `indent`，
/// 同时把本容器**内容最大宽**扣掉 `indent`（正文因此排在 `[indent, 内容宽]` 内，
/// 不会把容器撑宽）；返回录完后要还原的旧值（`indent <= 0` ⇒ `None`，什么都不做）。
fn push_body_indent(ui: &mut Ui<'_>, indent: f32) -> Option<(f32, Option<f32>)> {
    if indent <= 0.0 {
        return None;
    }
    let f = ui.frames.last_mut()?;
    let old_x = f.cursor.x;
    let (_, old_max_w) = f.width_debug();
    f.cursor.x = old_x + indent;
    f.set_max_w(old_max_w.map(|w| (w - indent).max(0.0)));
    Some((old_x, old_max_w))
}

/// **退出正文缩进**（还原光标 `x` 与内容最大宽；`y` **保留**推进量 —— 正文占的高度不能丢）。
fn pop_body_indent(ui: &mut Ui<'_>, saved: Option<(f32, Option<f32>)>) {
    let Some((old_x, old_max_w)) = saved else {
        return;
    };
    if let Some(f) = ui.frames.last_mut() {
        f.cursor.x = old_x;
        f.set_max_w(old_max_w);
    }
}

/// `RJ_FOLD_TRACE=1`：打印每个区块的关键几何、折叠态与**背景色来源**。
///
/// 为什么留一个开关：标题行的**宽**有三个来源（可用宽 / 最宽子项 / 兜底），**高**有两个
/// 来源（一行 `row_h` / 自定义标题的自然高），折叠态有两个来源（表 / `open` 默认），
/// **底色**也有两个来源（悬停 / 按下 vs 主题给的常态底色）——"标题行缩在半截""标题里的控件
/// 压住三角""点不开""为什么这行的底色和那行不一样"这类问题时，第一件事就是看这几个数。
///
/// `bg_alpha` 是**本帧实际用的底色**的 alpha：`0.00` = 常态透明（`Theme::foldable.bg` 默认
/// 全透明，区块"不抢视觉"）、`> 0` = 悬停 / 按下高亮**或**主题用了
/// [`FoldableStyle::button_like`](crate::FoldableStyle::button_like)（后者**恒**有底色）。
/// 想区分这两种"有底色"，把本行与鼠标位置对照看：**只有鼠标下的那一行** `hovered=true`。
fn trace_fold(
    id: &str,
    folded: bool,
    header: Rect,
    body_h: f32,
    style: &FoldableStyle,
    resp: &Response,
) {
    if std::env::var_os("RJ_FOLD_TRACE").is_some() {
        let bg = style.pick_bg(resp.pressed, resp.hovered);
        let alpha = bg
            .as_solid()
            .map(|c| <[f32; 4]>::from(c)[3])
            .unwrap_or(1.0);
        eprintln!(
            "fold[{id}] folded={folded} hovered={} pressed={} bg_alpha={alpha:.2} \
             header=({:.1},{:.1} {:.1}x{:.1}) body_h={body_h:.1} indent={:.1} \
             guide_w={:.1} fade_h={:.1}",
            resp.hovered,
            resp.pressed,
            header.x,
            header.y,
            header.w,
            header.h,
            style.body_indent,
            style.guide_w,
            style.fade_h
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::FoldableStyle;

    /// 首次（表里没记录）用 `open` 默认；有记录则**记录说了算**（`false` = 记住展开）。
    #[test]
    fn fold_state_prefers_record_over_default() {
        assert!(fold_state(None, false), "默认（open=false）⇒ 折叠");
        assert!(!fold_state(None, true), "open(true) ⇒ 首次展开");
        assert!(fold_state(Some(true), true), "记录优先：点的结果盖过 open");
        assert!(!fold_state(Some(false), false), "记录优先：展开也能被记住");
    }

    /// 翻转判据：鼠标（按下边沿 + 命中）与键盘（Enter/Space）各自成立；两者都不成立 = 不翻。
    #[test]
    fn fold_should_toggle_is_edge_or_keyboard_only() {
        assert!(fold_should_toggle(true, false), "按下边沿 + 命中 ⇒ 翻");
        assert!(fold_should_toggle(false, true), "键盘 Enter/Space ⇒ 翻");
        assert!(fold_should_toggle(true, true));
        assert!(!fold_should_toggle(false, false), "释放帧 / 悬停 ⇒ 不翻（否则一次点击翻两次）");
    }

    /// 标题行宽三条来源的优先级。
    #[test]
    fn fold_header_w_prefers_avail_then_widest_child_then_fallback() {
        assert_eq!(fold_header_w(Some(320.0), 200.0, 120.0), 320.0, "可用宽优先");
        assert_eq!(fold_header_w(None, 210.0, 120.0), 210.0, "自动宽容器：最宽子项");
        assert_eq!(fold_header_w(None, 0.0, 120.0), 120.0, "容器还空着：兜底");
        assert_eq!(fold_header_w(Some(0.0), 0.0, 120.0), 120.0, "可用宽 0 = 没线索");
        assert_eq!(fold_header_w(None, -5.0, 120.0), 120.0, "负的最宽子项不入账");
    }

    /// 图标 / 文本区在极窄标题行里**不产生负宽**（否则文本绘制与省略号测量会算出垃圾）。
    #[test]
    fn header_sub_rects_never_go_negative() {
        let s = FoldableStyle::default().with_padding(4.0).with_icon(18.0, 10.0);
        let header = Rect::new(10.0, 20.0, 300.0, 26.0);
        let icon = fold_icon_rect(header, &s);
        assert_eq!(icon, Rect::new(14.0, 28.0, 10.0, 10.0), "图标 = 左内边距起、边长 icon_h、垂直居中");
        let label = fold_label_rect(header, &s);
        assert_eq!(label.x, 10.0 + 4.0 + 18.0, "文本接在图标槽之后");
        assert_eq!(label.w, 300.0 - 4.0 * 2.0 - 18.0, "右缘留一个内边距");
        // 比图标槽还窄的标题行：文本区宽夹到 0（不是负数）。
        let tiny = Rect::new(0.0, 0.0, 12.0, 4.0);
        assert_eq!(fold_icon_rect(tiny, &s).w, 4.0, "图标边长夹到标题行高");
        assert_eq!(fold_label_rect(tiny, &s).w, 0.0);
    }

    /// 引导线：关掉（`guide_w = 0`）或正文为空 ⇒ 不画；否则包住正文并上下各延伸 `tail`。
    #[test]
    fn fold_guide_rect_wraps_body_or_disappears() {
        let s = FoldableStyle::default()
            .with_body_guide(12.0, Color::WHITE, 1.0, 2.0);
        let r = fold_guide_rect(100.0, 40.0, &s);
        assert_eq!((r.y, r.h), (98.0, 44.0), "上下各延伸 guide_tail");
        assert_eq!(r.w, 1.0);
        assert!(r.x > 0.0 && r.x < 12.0, "线落在缩进区内（标题三角下方）");
        assert_eq!(fold_guide_rect(100.0, 0.0, &s), Rect::ZERO, "空正文不画线");
        let off = s.with_body_guide(12.0, Color::WHITE, 0.0, 2.0);
        assert_eq!(fold_guide_rect(100.0, 40.0, &off), Rect::ZERO, "guide_w = 0 ⇒ 关掉");
    }
}
