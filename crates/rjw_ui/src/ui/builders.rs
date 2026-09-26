//! 责任链 builder：`WindowBuilder` / `PanelBuilder` / `ModalBuilder` / `RowBuilder`、
//! 窗口外框配件 `WindowChrome`，以及 `Ui` 的 builder 入口（`window` / `panel` /
//! `scroll_area` / `modal`）。
//!
//! 维护者笔记：builder 只收集选项，`.show(f)` 才落盘到 `*_impl`；选项缺省值必须与
//! 「不启用该特性」逐像素一致。

use super::*;
use super::window::{resolve_resizable_axes, v_axis_is_viewport};

use glam::Vec2;

use crate::draw::{
    Position, Size,
};
use crate::layout::{Frame, PackSide};
use crate::state::UiState;
use crate::style::PanelStyle;

/// **窗口责任链 builder**：[`Ui::window`] 返回。选项链式设置（`.pos` / `.width` /
/// `.height` / `.level` / `.placement` / `.style` / `.clamp` / `.resize` / `.vscroll` /
/// `.hscroll` / `.title` / `.close_button` / `.collapsible`）后以 `.show(f)` 终结执行。
///
/// # 设计理念：**无顾虑地使用**
///
/// **什么都不写也必须是对的**：位置自动级联、宽高**由内容撑开**（`pad + 内容 + pad`）、
/// 不裁切、无装饰、不抢焦点。只有**想控制**时才写：
///
/// | 想要 | 写什么 | 之后谁定尺寸 |
/// |---|---|---|
/// | 固定宽 / 高 | `.width(w)` / `.height(h)` | 应用（**初始值**，见下） |
/// | 长内容 + 竖向滚动条 | `.vscroll(true)`（+ `.height(h)` 给有界视口） | 视口高 = `.height` 或 `min(内容, 屏幕剩余)` |
/// | 长行 + 横向滚动条 | `.hscroll(true)`（+ `.width(w)` 给有界视口） | 视口宽 = `.width`，内容保持自然宽 |
/// | 按窗口宽折行 | `.hscroll(false)`（默认） | 内容被压进可用宽 |
/// | 让用户拖大小 | `.resize(true)` | **用户**（拖过即持久，`.width()/.height()` 只是初值） |
///
/// ⚠ **不许出现"必须写某个选项否则坏掉"的组合**：任何"本帧用上一帧结果反推尺寸"的解算都
/// 必须有**不依赖上一帧**的引导值（典例：自动宽窗口的滚动视口首帧曾取 1px ⇒ 内容按 1px
/// 折行 ⇒ 窗口塌成一条缝、且**永远**是缝）。完整理念与四条硬规矩见
/// `docs/UI_ARCHITECTURE.md` §0。
pub struct WindowBuilder<'ui, 'a> {
    pub(super) ui: &'ui mut Ui<'a>,
    pub(super) id: &'ui str,
    pub(super) o: WindowOptions,
    /// 标题栏文字（`None` = 不画标题栏）。
    pub(super) title: Option<&'ui str>,
    /// 关闭按钮绑定的开关（点 × ⇒ 置 `false`；为 `false` 时整个窗口不录制）。
    pub(super) close: Option<&'ui mut bool>,
    /// 收起（折叠）按钮：`(是否画按钮, 收起状态)`；状态 `None` = **引擎托管**
    /// （存 [`UiState::collapsed`]，点 ⌃ 由引擎翻转）。
    pub(super) collapsible: Option<(bool, Option<&'ui mut bool>)>,
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
    pub(super) fn bar_on(&self) -> bool {
        self.title.is_some()
            || self.close.is_some()
            || self.collapsible.as_ref().is_some_and(|(s, _)| *s)
    }

    /// 本帧是否画 ⌃ 按钮。
    pub(super) fn show_collapse(&self) -> bool {
        self.collapsible.as_ref().is_some_and(|(s, _)| *s)
    }

    /// 本帧是否**收起**（只留标题栏）。
    ///
    /// `Some(&mut bool)` = 应用持有；`None` = **引擎托管** ⇒ 读 [`UiState::collapsed`]
    /// （`abs` = 该窗口的**绝对 ID**）。
    pub(super) fn collapsed(&self, state: &UiState, abs: &str) -> bool {
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
    /// **垂直轴的溢出策略**（传 `bool` 或 [`ScrollMode`]；见 [`ScrollParam`]）：
    /// `true`（`Scroll`）= 视口 + 滚动条（滚轮 / 拖 thumb / 点轨道）；
    /// `false`（`NoClip`，默认）= 内容撑高窗口；`ScrollMode::ClipOnly` = 只裁不滚。
    ///
    /// 显式设置**覆盖** [`Self::placement`]。`Scroll` 时窗口高度取"上一帧结算高"
    /// 作为视口（首次 = 内容高），之后由用户拖拽 / 内容变化驱动。
    ///
    /// ```ignore
    /// .width(320.0).height(420.0).vscroll(true)   // 有界视口 + 右侧滚动条
    /// ```
    pub fn vscroll(mut self, mode: impl ScrollParam) -> Self {
        self.o.vscroll = Some(mode.scroll_mode());
        self
    }
    /// **水平轴的溢出策略**（同 [`Self::vscroll`]）：`true` = 视口 + **底部滚动条**
    /// （内容保持**自然宽、不折行**，超出横向滚）；`false` = 内容按窗口宽**压缩 / 折行**。
    ///
    /// ```ignore
    /// .width(200.0).hscroll(true)   // 固定 200 宽视口 + 横向滚动条（长行不折行）
    /// ```
    pub fn hscroll(mut self, mode: impl ScrollParam) -> Self {
        self.o.hscroll = Some(mode.scroll_mode());
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
    /// **能不能拖拽改大小**（只有一个布尔；egui 风 —— 允许的**轴**由 `vscroll` /
    /// `.height(..)` 推导，内容"压缩还是裁切 / 滚动"由 `hscroll` 决定）：
    ///
    /// | 垂直轴 | `.resize(true)` 允许的轴 | 水平轴内容 |
    /// |---|---|---|
    /// | 视口（`.vscroll(true)` / `ClipOnly`，或给了 `.height(..)`） | **垂直 + 水平** | `hscroll(true)` ⇒ **横向滚**（自然宽）；否则按宽**压缩** |
    /// | 不是视口（默认，高度由内容定） | **只有水平** | 同上 |
    ///
    /// - **不调本方法** = 旧行为：`.width(..)` 的窗口可**横向**拖拽缩放，否则没有柄；
    /// - `resize(false)`：**不画柄、也不响应拖拽**，但 `.width(..)` 仍是布局固定宽
    ///   （菜单 / 下拉浮层这类"尺寸由内容定"的窗口就是这么用的）；
    /// - `resize(true)`：右下角柄（`↔` / `↖↘` 光标随轴），拖出来的尺寸**跨帧持久**
    ///   （宽 → `UiState::window_widths`、高 → `UiState::window_heights`），
    ///   且固定轴不再参与内容撑开（高度被拖过 ⇒ 窗口成为固定尺寸视口）。
    ///
    /// **为什么垂直轴要"先有视口"才能拖**：高度由内容决定时（默认）拖高没有意义 ——
    /// 内容当帧就把它顶回去；先给它一个有界视口（`.height(..)` / `.vscroll(..)` /
    /// `.placement(Clip)`），拖出来的高才是真正生效的那个值。
    ///
    /// ⚠ **轴向不可显式指定**（本轮起：`bool` 只有一个）：需要"只可调高"这类窗口时，
    /// 给它一个纵向视口（`.height(..)` / `.vscroll(true)`）即可；控件级缩放
    /// （[`TextEditor::resize`](crate::TextEditor::resize)）仍用 [`Resize`] 枚举选轴。
    pub fn resize(mut self, allow: bool) -> Self {
        self.o.resize = Some(allow);
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
        // 枚举 → 内部两个开关（公开面不再出现裸布尔）。
        let topmost = o.level == Level::Topmost;
        let strict = o.placement == Placement::Clip;
        // **egui 风推导**（`.resize(bool)`）：允许的轴由"垂直轴有没有视口"决定
        // （判定表见 [`Self::resize`]）。`None`（没调）保持老语义：有 `.width(..)`
        // 就能横向拖（见 `resolve_window_resize`）。
        //
        // ⚠ "垂直轴是视口"的三个来源都算：`.vscroll(非 NoClip)` / `.height(..)` /
        // `.placement(Clip)`（后者两条轴都裁 ⇒ 高度被裁而非被内容顶开，"拖高"因此有意义）。
        let resize = o.resize.map(|allow| {
            (
                allow,
                resolve_resizable_axes(
                    allow,
                    v_axis_is_viewport(o.vscroll, o.height.is_some() || strict),
                ),
            )
        });
        let height = o.height.map(|h| h.to_physical(ui.scale));
        let content_gap = o.gap.map(|g| g.to_physical(ui.scale));
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
    pub(super) ui: &'ui mut Ui<'a>,
    pub(super) pos: Position,
    pub(super) drag: Option<&'ui str>,
    pub(super) style: Option<PanelStyle>,
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
    pub(super) ui: &'ui mut Ui<'a>,
    pub(super) id: &'ui str,
    pub(super) pos: Position,
    pub(super) width: Option<Size<f32>>,
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
    pub(super) ui: &'ui mut Ui<'a>,
    pub(super) min_h: Option<Size<f32>>,
    pub(super) max_h: Option<Size<f32>>,
    pub(super) gap: Option<Size<f32>>,
    pub(super) pad: f32,
    /// **是否请求自动换行**（只有请求了才折行 —— 见 [`RowBuilder::wrap_w`] / [`RowBuilder::wrap`]）。
    ///
    /// ⚠ 这条是**刻意**的：`avail_w()` 兜底若对所有 `row` 生效，既有 `row` 在窄容器里会从
    /// "子项被压窄"（`Frame::remaining_w`）变成"折行"，等于**悄悄改了现有布局**。
    pub(super) wrap: bool,
    /// **行宽上限**（[`Size<f32>`]）：`Some` ⇒ 用它（与父级可用宽取 min）；
    /// `None`（配合 `wrap = true`）⇒ **自动**：用父级可用宽。
    pub(super) wrap_w: Option<Size<f32>>,
    /// **折行的行间距**（[`Size<f32>`]；不调 = 与 `gap` 相同）。
    pub(super) line_gap: Option<Size<f32>>,
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

    /// **行宽上限 ⇒ 开启自动换行**（[`Size<f32>`]：逻辑（默认）/ 物理）。
    ///
    /// - 子项**按自然宽**申请，放不下且本行已有子项时**收行**（行间距 `line_gap`）；
    /// - **只换行、不压缩**：开启后不再报"行内剩余宽"（否则 `Label` 这类
    ///   `LimitedInParent` 子项会被压扁而不是换行）；
    /// - 与**父级可用宽取 min**（嵌套在窄容器里时不许排到外面）；
    /// - **不调本方法 / [`RowBuilder::wrap`] 的 `row` 行为一字不变**（宽 = 内容，窄容器里
    ///   仍旧走"压窄"那条路）。
    ///
    /// ```ignore
    /// // ⚠ `ignore`：容器闭包里才拿得到 `UiAdd`。
    /// ui.row_builder().wrap_w(240.0).show(|r| { for t in tags { r.button(t, t); } });
    /// ```
    pub fn wrap_w(mut self, w: impl Into<Size<f32>>) -> Self {
        self.wrap = true;
        self.wrap_w = Some(w.into());
        self
    }

    /// **开启自动换行、行宽 = 父级可用宽**（不写死宽度）。
    ///
    /// ⚠ 自动宽窗口**首帧** `avail_w()` 还是 `None` ⇒ 本帧不折行（次帧拿到窗口内容宽后才
    /// 折）—— 与 `Label` 的换行同口径；要"首帧就折"就显式给 [`RowBuilder::wrap_w`]。
    pub fn wrap(mut self) -> Self {
        self.wrap = true;
        self.wrap_w = None;
        self
    }

    /// **折行的行间距**（[`Size<f32>`]；不调 = 与 `gap` 相同）。
    pub fn line_gap(mut self, g: impl Into<Size<f32>>) -> Self {
        self.line_gap = Some(g.into());
        self
    }

    /// 终结：执行闭包并返回行结算尺寸（物理像素；已按 `min_h` / `max_h` 夹过）。
    pub fn show(self, f: impl FnOnce(&mut Pack<'_, '_>)) -> Vec2 {
        let Self { ui, min_h, max_h, gap, pad, wrap, wrap_w, line_gap } = self;
        let scale = ui.scale();
        let std_h = min_h.map_or_else(|| ui.theme().row_h, |h| h.to_physical(scale));
        let max_h = max_h.map(|h| h.to_physical(scale));
        let gap = gap.map_or_else(|| ui.theme().gap, |g| g.to_physical(scale));
        let line_gap = line_gap.map_or(gap, |g| g.to_physical(scale));
        // **行宽上限**（物理像素）：只有**请求了折行**才算 ——
        // `.wrap_w(w)` ⇒ `min(w, 父级可用宽)`；`.wrap()` ⇒ 父级可用宽（首帧可能为 `None`）；
        // 都没请求 ⇒ `None`（**不折行**，既有 `row` 行为一字不变）。
        let avail = ui.avail_w().filter(|&w| w > 0.0);
        let explicit = wrap_w.map(|w| w.to_physical(scale)).filter(|&w| w > 0.0);
        let limit = if wrap {
            match explicit {
                Some(w) => Some(avail.map_or(w, |a| w.min(a))),
                None => avail,
            }
        } else {
            None
        };
        let origin = ui.cursor_pos();
        let (size, _) = ui.container(origin, Frame::new_stack(PackSide::Left, gap, pad), |ctx| {
            let fr = ctx.ui.frames.last_mut().expect("row frame");
            fr.set_force_h_all(std_h);
            fr.set_row_bounds(Some(std_h), max_h);
            // 折行 + 行宽上限同时作为**内容最大宽** ⇒ 子项拿到的 `avail_w` 是**整行宽**
            // （不是"行内剩余"，见 `Frame::remaining_w` 的折行分支）。
            fr.set_wrap(limit, line_gap);
            if let Some(l) = limit {
                fr.set_max_w(Some(l));
            }
            let mut p = Pack { ui: ctx.ui };
            f(&mut p);
        });
        // 结算后补记父容器光标（占一行）。
        if let Some(fr) = ui.frames.last_mut() {
            fr.place_external(size);
        }
        trace_row(scale, wrap, explicit, avail, limit, std_h, line_gap, size);
        size
    }
}

/// `RJ_ROW_TRACE=1`：打印**请求了折行的** `row` 的宽度来源与结算尺寸（没请求的静默）。
///
/// 为什么留：折行"有没有真的发生"只能从高度看出来（`size.y > std_h` 即折了）；而 `limit`
/// 是"显式值与父级可用宽取 min"的结果 —— 这两条正是排查"怎么没折 / 折多了"的第一手数据
/// （`limit = None` ⇒ 本帧没折：既没给 `.wrap_w(..)`，`.wrap()` 时父级也没有可用宽）。
fn trace_row(
    scale: f32,
    wrap: bool,
    explicit: Option<f32>,
    avail: Option<f32>,
    limit: Option<f32>,
    std_h: f32,
    line_gap: f32,
    size: Vec2,
) {
    if !wrap || std::env::var_os("RJ_ROW_TRACE").is_none() {
        return;
    }
    eprintln!(
        "row_trace: scale={scale:.2} explicit={explicit:?} avail={avail:?} limit={limit:?} \
         std_h={std_h:.1} line_gap={line_gap:.1} size=({:.1}x{:.1}) 折行={}",
        size.x,
        size.y,
        if size.y > std_h + 0.5 { "是" } else { "否（一行装得下）" }
    );
}

