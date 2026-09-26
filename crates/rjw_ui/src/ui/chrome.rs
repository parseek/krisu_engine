//! 窗口标题栏：条高、布局解算、绘制与 `RJ_CHROME_TRACE` 诊断。
//!
//! 维护者笔记：标题栏按钮**不参与行内布局**——落点由纯函数 `title_bar_layout` 解算
//! （贴窗口**外框**右缘，见 `docs/ENGINE_GUIDE.md`）。三处数值（右缘 / inset / 夹取）
//! 都有单测钉住，改公式请同步 `ui/tests.rs`。

use super::*;

use glam::Vec2;
use rjw_transform::Rect;

use crate::draw::{
    CornerRadius, Icon, Position,
};
use crate::id::IdAbsolute;

/// **标题栏条高**（纯函数，可单测）= **一行** [`Theme::row_h`]。
///
/// 内容贴顶后不再需要"上内边距 + 一行"：标题行录在窗口顶边（`y = 0`），条高就是那一行。
/// ⚠ 条只是**背景装饰、不裁剪内容** —— 标题 / ▲ / ✕（边长 `row_h - 2`）允许**比条高再高一点**
/// （用户明确要求"内容可以比条高再高一点"），本函数**与按钮边长无关**。
#[inline]
pub(super) fn title_bar_h(row_h: f32) -> f32 {
    row_h
}

/// **标题栏按钮贴外缘的内缩**（物理像素；`0` = 贴窗口外框右缘 = Windows 风格）。
///
/// 单独提出来当常量、而不是散在算式里：这就是"Windows 风格贴角 / 传统内边距"的
/// 唯一开关（想留一条缝就改这一个数，`title_bar_layout` 已经参数化）。
pub(super) const TITLE_BUTTON_INSET: f32 = 0.0;

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
pub(super) struct TitleBarLayout {
    /// 标题可用宽（物理像素；`0` = 不截断成负宽）。
    pub(super) title_max: f32,
    /// 收起（⌃）按钮矩形（`None` = 不画）。
    pub(super) collapse: Option<Rect>,
    /// 关闭按钮矩形（`None` = 不画）。
    pub(super) close: Option<Rect>,
    /// 条宽：固定宽窗口 = 外框宽；自动宽窗口 = 由内容推导出的外框宽。
    pub(super) bar_w: f32,
}

pub(super) fn title_bar_layout(
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
pub(super) fn window_title_bar(
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

