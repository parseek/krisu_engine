//! **下拉面板**：按钮下拉菜单（[`Dropdown`](crate::Dropdown)）与菜单栏
//! （[`MenuBar`](crate::MenuBar)）**共用**的一套浮层实现 + 菜单内容容器（[`MenuCtx`]）。
//!
//! 本模块只有两件事：
//!
//! 1. [`MenuCtx`] —— **菜单内容的容器上下文**（`Deref` 到 [`Window`] ⇒ 菜单里也能用
//!    全部 [`UiAdd`](crate::UiAdd) 方法：`add` / `text_input` / `separator` / `row` /
//!    再嵌一个 `Dropdown`……），外加菜单语义的 `item` / `item_checked` / `caption` /
//!    `separator`；
//! 2. [`popup_show`]（`pub(crate)`）—— **唯一的下拉浮层录制器**：哨兵 z、锁定位置、
//!    不画缩放柄、面板样式、宽度收敛、按内容结算尺寸、点外 / Esc / 点项的收起判定。
//!
//! # 为什么要合一
//!
//! 此前 `Ui::combo_at`（下拉框）与 `MenuBar::popup`（菜单栏）各自录一个浮层窗口、
//! 各自画一套菜单项——几何公式与观感已经开始漂移（一边是 ✓ 图标、一边是方框勾选；
//! 行高 / 内边距各写一遍）。现在**只有这一处**实现浮层，两边的差别只剩"触发器长什么样"
//! 与"菜单内容谁来写"（选项列表由 `Dropdown::options` 生成，或应用用闭包自己写）。
//!
//! # 几何契约（三条一起才对齐，缺一条就"看着有点怪"）
//!
//! 1. **内边距 = [`popup_padding`]**（`ComboStyle::item_pad_x`，四边同值）：
//!    菜单项 / `caption` / `separator` / `row` 全部从**同一个内容原点**起排 ⇒ 天然同列。
//!    （别用"给下一子项缩进"的花招：垂直栈里子项 `x` **恒等于内容原点**，缩进宽度无效。）
//! 2. **面板宽 = 上一帧的结算宽**（`UiState::window_sizes`，首帧自然宽、次帧起精确）：
//!    固定宽已知（`fill = true`）时子项请求"极宽"由窗口 clamp 到内容宽 ⇒ **高亮 /
//!    分割线铺满面板**；首帧必须请求**自然宽**，否则 1e6 的请求会把自然尺寸撑成一百万、
//!    面板宽度再也收不回来。见 [`popup_content_w`] / [`popup_child_w`]。
//! 3. **分割线自己画**，不用 [`Divider`](crate::Divider)：`Divider` 的宽 = `avail_w()`，
//!    而**自动宽**窗口里那是 `None` ⇒ 退回固定 120 ⇒ 线又短又不在该在的位置
//!    （用户实测："Menu 分割线错位"）。自己画时请求"极宽"由窗口 clamp ⇒ 恒等于内容宽。
//!
//! # 关闭规则（一条都不许丢）
//!
//! | 事件 | 行为 |
//! |---|---|
//! | 点了菜单项（`MenuCtx::item*` 上报） | 收起 |
//! | `Esc` | 收起（应用自己的 Esc 语义先看 `UiState::menu_open()` / `combo_open()`） |
//! | 左键按下在**面板外** | 收起 |
//! | 左键按下在**面板内 / 触发器上 / 任意 `WIN_TOPMOST` 浮层上** | **不**收起 |
//!
//! 最后那条"任意 `WIN_TOPMOST` 浮层"是给**子菜单**留的：菜单里再 `add` 一个
//! `Dropdown`（`PopupSide::Right`）时，点子菜单不该被外层菜单当成"点面板外"而把外层一起关掉。
//!
//! 排查通道：`RJ_MENU_TRACE=1` 打印每个下拉内容的**行矩形**（`x/y/w/h`）——正确时
//! 菜单项 / 分割线 / 标题必须同 `x`、同 `w`（实测 `x=8 w=234`）。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::draw::{Icon, Position, Size, TextAlign, TextVAlign};
use crate::hit::{hit_test, update_interact};
use crate::layout::Child;
use crate::style::{ComboStyle, PanelStyle, Theme};
use crate::ui::{Level, Ui, UiAdd as _, WindowClamp, WIN_TOPMOST};
use crate::Window;

// ─── 公开几何助手（示例 / 仿真算坐标用，别在脚本里抄魔数） ──────

/// **菜单项行高**（菜单项 / 选项行的统一高度）：`font_size × 1.3` 取整 + 上下各 1px。
///
/// 公开的理由：示例 / 脚本化仿真要按同一公式算"点哪一行"——与引擎同源才不会
/// "改了行高脚本就点空"（`--sim-menu` / `--sim-dropdown` 都用它）。
#[inline]
pub fn item_h(font_size: f32) -> f32 {
    (font_size.max(1.0) * 1.3).round() + 2.0
}

/// **下拉面板的内边距**（= `ComboStyle::item_pad_x`；四边同值，`PanelStyle::padding` 是标量）。
///
/// 与 [`item_h`] 同理公开：面板内第一行的原点 = 面板原点 + 本值（+ 边框）。
#[inline]
pub fn popup_padding(theme: &Theme) -> f32 {
    theme.combo.item_pad_x
}

/// **面板相对触发器的方位**。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PopupSide {
    /// 触发器正下方（默认；按钮下拉菜单）。
    #[default]
    Below,
    /// 触发器右侧（子菜单：菜单里再 `add` 一个 `Dropdown` 时用它）。
    Right,
}

/// **面板原点**（触发器矩形 + 方位 ⇒ 面板左上角；纯函数，可单测）。
///
/// 两档之间留 2px 缝隙（面板与触发器不粘连，一眼看出"这是浮层"）。
#[inline]
pub fn popup_origin(trigger: Rect, side: PopupSide) -> Vec2 {
    match side {
        PopupSide::Below => Vec2::new(trigger.x, trigger.y + trigger.h + 2.0),
        PopupSide::Right => Vec2::new(trigger.x + trigger.w + 2.0, trigger.y),
    }
}

// ─── 内容渲染器（`Dropdown` 的泛型约束） ────────────────────────

/// **菜单内容**：往 [`MenuCtx`] 里录内容的东西。
///
/// 两种情况：
/// - `()`：内容为空（`Dropdown` 的选项列表模式由引擎自己排，不需要闭包）；
/// - [`MenuFn`]：应用给的闭包。
///
/// ⚠ 为什么用 `MenuFn` 包装而不是直接 `impl<F: FnOnce(..)> MenuContent for F`：
/// 那会与 `impl MenuContent for ()` **撞 coherence**（编译器不允许对"可能实现的
/// blanket + 具体类型"同时下断言）。零成本新类型把两条 impl 彻底分开。
pub trait MenuContent {
    /// 把内容录进菜单上下文（`Dropdown` 的 `menu` 闭包与空内容都实现它）。
    fn render(self, m: &mut MenuCtx<'_, '_, '_>);
}

/// **菜单内容闭包**的零成本包装（见 [`MenuContent`] 的说明）。
pub struct MenuFn<F>(F);

impl<F> MenuFn<F> {
    /// 包装一个内容闭包。
    #[inline]
    pub fn new(f: F) -> Self {
        Self(f)
    }
}

impl MenuContent for () {
    #[inline]
    fn render(self, _m: &mut MenuCtx<'_, '_, '_>) {}
}

impl<F> MenuContent for MenuFn<F>
where
    F: FnOnce(&mut MenuCtx<'_, '_, '_>),
{
    #[inline]
    fn render(self, m: &mut MenuCtx<'_, '_, '_>) {
        (self.0)(m);
    }
}

// ─── 浮层录制器（唯一实现） ─────────────────────────────────────

/// 一个下拉浮层的规格（[`popup_show`] 的入参）。
pub(crate) struct PopupSpec<'a> {
    /// 浮层窗口 id（**相对当前容器**；绝对 id 自动带命名空间前缀）。
    ///
    /// `Dropdown` 用 `<控件 id>::popup`、`MenuBar` 用 `<栏 id>::<菜单名>`——
    /// `debug_dump()` 按这个 id 找面板，脚本化仿真也按它匹配。
    pub id: &'a str,
    /// 触发器矩形（**当前容器局部**坐标）。
    pub trigger: Rect,
    /// 面板相对触发器的方位。
    pub side: PopupSide,
    /// 菜单字号（菜单项 / `caption`）。
    pub font_size: f32,
    /// 面板**最小总宽**（调用方一般给 `max(触发器宽, ComboStyle::item_min_w)`）。
    pub min_w: f32,
}

/// 浮层录制结果（收起判定所需的原始事实，由调用方拍板）。
pub(crate) struct PopupResult {
    /// 面板矩形（**当前容器局部**坐标）。
    pub rect: Rect,
    /// 内容里点了菜单项（`MenuCtx::item*` 上报）。
    pub item_clicked: bool,
    /// 本帧左键按下落在**面板之外**（且不在任何 `WIN_TOPMOST` 浮层上）。
    pub down_outside: bool,
}

/// **录一个下拉浮层窗口**（唯一实现，[`Dropdown`](crate::Dropdown) 与
/// [`MenuBar`](crate::MenuBar) 共用）。
///
/// 返回 [`PopupResult`]：面板矩形 + 两条收起事实。收起与否由调用方决定
/// （菜单栏要按"栏内任意触发器"判定，下拉控件只看自家触发器——见
/// [`dropdown_should_close`]）。
pub(crate) fn popup_show(
    ui: &mut Ui<'_>,
    spec: &PopupSpec<'_>,
    content: impl MenuContent,
) -> PopupResult {
    let cs = ui.theme.combo.clone();
    let pad = popup_padding(ui.theme());
    let border_w = ui.theme.panel.border_w;
    let pad_total = pad + border_w;
    // 面板样式 = **菜单外观**（`ComboStyle`）+ 主题面板的投影 / 边框宽（`border_w` 走
    // 主题令牌，不再像早期那样硬编码 1.0）。
    let style = PanelStyle {
        bg: cs.menu_bg.into(),
        border: cs.menu_border,
        padding: pad,
        radius: cs.menu_radius,
        bg_image: None,
        ..ui.theme.panel.clone()
    };
    let pos = popup_origin(spec.trigger, spec.side);
    // **强制哨兵 z**：下拉浮层恒在一切窗口之上，这样"菜单栏 / 下拉录在哪里"都不影响
    // 遮挡。⚠ 键必须是**绝对 id**（`window_impl` 内部按当前栈解析出同一前缀）。
    let abs = ui.id_for(spec.id);
    ui.state_mut()
        .window_z
        .insert(abs.to_static(), WIN_TOPMOST);
    // 固定宽 = **上一帧的结算宽**（首帧自然宽，次帧起精确）。
    // ⚠ `WindowBuilder::width` 收的是**内容宽**，而 `settle = fixed_w + 2*pad_total`
    // ⇒ 传 `prev - 2*pad_total` 才是"保持上一帧的面板宽"。
    let prev = ui
        .state()
        .window_sizes
        .get(abs.as_str())
        .map(|s| s.x);
    let fill = prev.is_some();
    let content_w = popup_content_w(fill, prev, spec.min_w, pad_total);
    if std::env::var_os("RJ_MENU_TRACE").is_some() {
        eprintln!(
            "menu[popup {}] prev={prev:?} pad={pad:?} border_w={border_w} min_w={} fill={fill} content_w={content_w:?}",
            spec.id, spec.min_w
        );
    }
    let mut close = false;
    let mut b = ui
        .window(spec.id)
        .pos(Position::Physical(pos))
        // **锁定位置 + 不允许拖拽缩放**：浮层是"某个控件的展开部分"，拖走了就与触发
        // 器脱节（命中按窗口走、视觉却跑别处 —— 用户实测"控件严重错位"）；尺寸由内容
        // 定，也不该出现缩放柄。`.width(..)` 只用来把内容宽钉在上一帧结算值上。
        .clamp(WindowClamp::Locked)
        .resize(false, crate::Resize::None)
        .level(Level::Normal)
        .style(style);
    if let Some(w) = content_w {
        b = b.width(Size::Physical(w));
    }
    let size = b.show(|w| {
        let mut m = MenuCtx {
            w,
            font_size: spec.font_size,
            pad,
            fill,
            close: &mut close,
        };
        content.render(&mut m);
    });
    let rect = Rect::new(pos.x, pos.y, size.x, size.y);
    // 点外判定：面板矩形与鼠标**同一坐标系**（都是当前容器局部），用 `mouse_local()`；
    // "落在任意 WIN_TOPMOST 浮层上"用公开的 `window_under_mouse()`（子菜单不关外层）。
    let mouse = ui.mouse_local();
    let on_popup = hit_test(&rect, mouse);
    let on_overlay = ui
        .window_under_mouse()
        .is_some_and(|(_, z)| z == WIN_TOPMOST);
    let down_outside = ui.mouse_left().down_edge() && !on_popup && !on_overlay;
    PopupResult {
        rect,
        item_clicked: close,
        down_outside,
    }
}

/// 面板**内容宽**（`None` = 首帧自动宽；`Some` = 固定宽）。
///
/// 纯函数（可单测）：`max(上一帧面板宽, 最小面板宽) − 2 × (横向内边距 + 边框)`，
/// 下限 40（窄到点不中的面板没有意义）。
#[inline]
fn popup_content_w(fill: bool, prev: Option<f32>, min_w: f32, pad_total: f32) -> Option<f32> {
    if !fill {
        return None;
    }
    let panel_w = prev.unwrap_or(min_w).max(min_w);
    Some((panel_w - pad_total * 2.0).max(40.0))
}

/// 子项的**请求宽**：固定宽已知 ⇒ 请求"极宽"（被窗口 clamp 到内容宽 ⇒ 高亮 / 分割线
/// 铺满面板）；否则返回自然宽（首帧靠它决定面板自然尺寸）。
///
/// ⚠ 首帧**必须**用自然宽：早期实现首帧也请求 1e6，结果自然尺寸被撑成一百万，
/// 面板宽度再也收敛不回来。
#[inline]
fn popup_child_w(fill: bool, natural: f32) -> f32 {
    if fill { 1.0e6 } else { natural }
}

/// **下拉控件（`Dropdown`）的收起判定**（纯函数，见模块文档的关闭规则表）。
///
/// 与菜单栏的差别：菜单栏的"点外部"要按**栏内任意触发器**判定（点另一个触发器 =
/// 切换菜单，不是收起），所以那条留在 `MenuBar::finish`；本函数只看自家触发器。
#[inline]
pub(crate) fn dropdown_should_close(item_clicked: bool, down_outside: bool, on_trigger: bool, esc: bool) -> bool {
    item_clicked || esc || (down_outside && !on_trigger)
}

/// 菜单项 / 选项行的**底色**（真值表）：选中 > 按下/悬停 > 常态。
#[inline]
fn row_bg(selected: bool, pressed: bool, hovered: bool, cs: &ComboStyle) -> Color {
    if selected {
        cs.item_selected
    } else if pressed || hovered {
        cs.item_hover
    } else {
        cs.menu_bg
    }
}

/// 选项列表模式下第 `i` 行是否打勾（`sel = None` ⇒ 全部不打勾）。
#[inline]
pub(crate) fn option_marked(i: usize, sel: Option<u32>) -> bool {
    sel == Some(i as u32)
}

// ─── 菜单内容上下文 ─────────────────────────────────────────────

/// **下拉面板的内容上下文**（`Dropdown::menu(..)` / `MenuBar::menu(..)` 的 `m`）。
///
/// [`Deref`](std::ops::Deref) 到内层 [`Window`] ⇒ `add` / `label` / `button` / `divider` /
/// `row` / `text_input` / `text_area` / `panel` …… **全部可用**（"菜单里也能放文本输入、
/// 分割线、按钮、横向排版，甚至再嵌一个下拉菜单"）；另外提供菜单语义的
/// [`Self::item`] / [`Self::item_checked`] / [`Self::caption`] / [`Self::separator`]。
pub struct MenuCtx<'w, 'a, 'b> {
    w: &'w mut Window<'a, 'b>,
    font_size: f32,
    /// 面板的左内边距（菜单项高亮往左右各扩这么多才铺满内缘）。
    pad: f32,
    /// **面板宽度已固定**（第 2 帧起）：子项请求"极宽"由窗口 clamp 到内容宽 ⇒ 铺满。
    /// 第 1 帧（自动宽）为 `false`：必须请求自然宽，否则会把自然尺寸撑爆。
    fill: bool,
    /// 点了菜单项 ⇒ 收起（由 [`popup_show`] 汇总）。
    close: &'w mut bool,
}

impl<'w, 'a, 'b> std::ops::Deref for MenuCtx<'w, 'a, 'b> {
    type Target = Window<'a, 'b>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.w
    }
}

impl<'w, 'a, 'b> std::ops::DerefMut for MenuCtx<'w, 'a, 'b> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.w
    }
}

impl MenuCtx<'_, '_, '_> {
    /// **菜单项**（占一行）：返回本帧是否被点击；点击即"执行 + 收起"。
    ///
    /// 整行可点（不是只有文字），hover / 按下整行高亮（与选项行同一观感）。
    pub fn item(&mut self, label: &str) -> bool {
        self.item_core(label, None, false)
    }

    /// **带勾选的菜单项**（视图显隐这类）：点击直接翻转 `&mut bool` 并收起。
    ///
    /// 勾选标记是**方框**（矢量画的圆角方框 + 勾号，`border_w = 0` 时退化成实心底）——
    /// 不用 "✓" 字形：字体缺字形时菜单里会出现豆腐块。
    pub fn item_checked(&mut self, label: &str, checked: &mut bool) -> bool {
        let clicked = self.item_core(label, Some(*checked), false);
        if clicked {
            *checked = !*checked;
        }
        clicked
    }

    /// **选项行**（选项列表模式；`Dropdown::options` 用）：`marked` = 本行是当前选中。
    ///
    /// 与 [`Self::item_checked`] 的差别只有两点：勾选框**只在选中时画**（不选中不画空框，
    /// 与旧下拉框的 ✓ 观感一致），选中行整行用 `ComboStyle::item_selected` 高亮。
    /// 方框列**恒留位**（两类项文字都对得齐）。
    pub(crate) fn option(&mut self, label: &str, marked: bool) -> bool {
        self.item_core(label, marked.then_some(true), marked)
    }

    /// 菜单里的一条**分割线**（满内容宽、与菜单项文字同列）。
    pub fn separator(&mut self) {
        let (t, m) = {
            let d = self.w.ui_mut().theme.divider.clone();
            (d.thickness, d.margin)
        };
        let h = t + m * 2.0;
        let r = self.row_rect(120.0, h);
        let y = r.y + (r.h - t) * 0.5;
        let color = self.w.ui_mut().theme.divider.color;
        self.w
            .ui_mut()
            .push_solid_rect(Rect::new(r.x, y, r.w, t), color);
        self.trace("separator", r);
    }

    /// 菜单里的**纯文本行**（不可点：分组标题 / 说明）——比菜单项字号略小、颜色更淡，
    /// 一眼看出"这是分组标题而不是可点的项"。
    pub fn caption(&mut self, text: &str) {
        let (fs, fam, color, h, tw) = {
            let t = self.w.ui_mut().theme.clone();
            let fs = (self.font_size * 0.85).round().max(1.0);
            let fam = t.combo.font_family.clone();
            let tw = self.w.ui_mut().text_size(text, fs, fam.as_deref()).x;
            (fs, fam, t.palette.text_muted, fs.max(14.0) + 4.0, tw)
        };
        let r = self.row_rect(tw, h);
        self.w.ui_mut().push_text_rect(
            r,
            text,
            fs,
            color,
            fam,
            TextAlign::Left,
            TextVAlign::Center,
            None,
            None,
        );
        self.trace("caption", r);
    }

    /// 占一行光标并取回该行矩形：**宽度已固定时请求"极宽"**（由窗口 clamp 到内容宽 ⇒
    /// 高亮 / 分割线铺满面板），否则用调用方给的**自然宽**（第 1 帧决定面板自然尺寸）。
    fn row_rect(&mut self, natural_w: f32, h: f32) -> Rect {
        let w = popup_child_w(self.fill, natural_w);
        self.w.ui_mut().child_rect(w, h, Child::Expand)
    }

    /// `RJ_MENU_TRACE=1`：打印每个下拉内容的**行矩形**（`x/y/w/h`）。
    ///
    /// 为什么留这条通道：菜单的"对齐"是纯几何约定（菜单项 / 分割线 / 标题必须同 x、
    /// 同宽），而它**看不出来**——肉眼看"差不多"，出问题时只是"有点怪"。有了这几个数，
    /// "分割线错位"这类问题一眼可判（`w` 应等于菜单项的 `w`，`x` 也应相同）。
    fn trace(&self, kind: &str, r: Rect) {
        if std::env::var_os("RJ_MENU_TRACE").is_some() {
            eprintln!(
                "menu[{kind}] x={:.0} y={:.0} w={:.0} h={:.0}",
                r.x, r.y, r.w, r.h
            );
        }
    }

    /// 菜单项 / 选项行的公共实现：`check = Some(是否勾选)` 时左侧画一个**方框勾选框**。
    ///
    /// 勾选标记是**方框**（与 [`crate::Checkbox`] 同一观感：圆角方框 + 勾选时填
    /// `CheckboxStyle::checked_fill` + 矢量勾号），画在**菜单项内容里**、方框列**恒留位**
    /// （勾选与否文字都对齐）。为什么不用"左内边距里放一个 ✓"：那需要很大的左内边距
    /// （用户实测："边距太大"），而方框只占 `checkbox.box_size`，可以贴着小边距放。
    ///
    /// ⚠ 名字刻意不叫 `row`：`MenuCtx` 经 `Deref` 到 [`Window`]，同名私有方法会**遮蔽**
    /// `Window::row`（deref 方法优先级更低）⇒ 应用的 `m.row(..)` 会去调私有那个。
    fn item_core(&mut self, label: &str, check: Option<bool>, selected: bool) -> bool {
        let (cs, font_size, ih, natural_w, box_size, box_gap, cb) = {
            let t = self.w.ui_mut().theme.clone();
            let fs = self.font_size.max(1.0);
            let cb = t.checkbox.clone();
            let ih = item_h(fs);
            let box_size = cb.box_size;
            let tw = self
                .w
                .ui_mut()
                .text_size(label, fs, t.combo.font_family.as_deref())
                .x;
            let natural = (box_size + cb.gap + tw + t.combo.item_pad_x * 2.0).max(t.combo.item_min_w);
            (t.combo.clone(), fs, ih, natural, box_size, cb.gap, cb)
        };
        // **宽度已固定 ⇒ 请求极宽**：窗口把子项 clamp 到内容宽 ⇒ 高亮满宽（与面板等宽）。
        let rect = self.row_rect(natural_w, ih);
        let item_id = format!("item::{label}");
        let abs = self.w.ui_mut().id_for(item_id.as_str());
        let ui = self.w.ui_mut();
        let hit = ui.hit_abs(&abs, &rect);
        let btn = ui.mouse_left();
        if btn.down_edge() && hit {
            // 菜单项自己消费按下：别让浮层窗口把这次按下当成"拖窗口"。
            ui.claim_press();
        }
        let ev = {
            let ws = ui.state_mut().widget(&abs);
            update_interact(ws, hit, btn)
        };
        // 高亮**满内宽**：`rect` 是内容区，左右各扩一个 `pad` 就铺满面板内缘。
        // `elem_hint()`：装饰画在本元素背景之上（写死小 elem 会被自己的背景盖住）。
        let bg = row_bg(selected, ev.pressed, hit, &cs);
        let hl = Rect::new(rect.x - self.pad, rect.y, rect.w + self.pad * 2.0, rect.h);
        ui.push_panel_like(hl, bg, cs.menu_bg, 0.0, 0.0, ui.elem_hint());
        // **方框勾选列**（恒留位）：只有需要画框的行画；其余行仍然留白
        // （`check = None` → 文字起点与勾选行一致，两类项文字对齐）。
        let cbox = Rect::new(rect.x, rect.y + (rect.h - box_size) * 0.5, box_size, box_size);
        if let Some(on) = check {
            let elem = ui.elem_hint();
            let mark_color = ui.theme().palette.surface;
            if on {
                // 勾选：实心 + 矢量勾号（缺字形也不会变豆腐块）。
                ui.push_panel_like(
                    cbox,
                    cb.checked_fill,
                    cb.box_border,
                    cb.border_w,
                    cb.radius,
                    elem,
                );
                let d = box_size * 0.78;
                ui.icon_at(
                    Position::Physical(Vec2::new(
                        cbox.x + (cbox.w - d) * 0.5,
                        cbox.y + (cbox.h - d) * 0.5,
                    )),
                    Size::Physical(Vec2::splat(d)),
                    Icon::Check,
                    mark_color,
                );
            } else if cb.border_w <= 0.0 {
                // **边框宽 = 0 的兜底**：空心框只靠描边存在，边框关掉就整个消失 ⇒
                // 退化成实心底（与 `Checkbox` 同一套规则，见 `Ui::draw_check_common`）。
                let bg = if hit {
                    ui.theme().palette.surface_hover
                } else {
                    ui.theme().palette.surface_sunken
                };
                ui.push_panel_like(cbox, bg, Color::TRANSPARENT, 0.0, cb.radius, elem);
            } else {
                // 空心方框（背景透明，只描边）。
                ui.push_panel_like(
                    cbox,
                    Color::TRANSPARENT,
                    cb.box_border,
                    cb.border_w,
                    cb.radius,
                    elem,
                );
            }
        }
        let text_x = rect.x + box_size + box_gap;
        ui.push_text_rect(
            Rect::new(text_x, rect.y, (rect.w - (text_x - rect.x)).max(0.0), rect.h),
            label,
            font_size,
            cs.fg,
            cs.font_family.clone(),
            TextAlign::Left,
            TextVAlign::Center,
            None,
            None,
        );
        if ev.clicked {
            *self.close = true;
        }
        self.trace("item", rect);
        ev.clicked
    }

    /// 面板内容的字号（应用自绘时对齐用）。
    #[inline]
    pub fn font_size(&self) -> f32 {
        self.font_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn item_height_is_the_documented_formula() {
        // 行高有**脚本依赖**（`--sim-menu` / `--sim-dropdown` 按它算"点哪一行"）：
        // 改了公式而忘了改脚本，症状是"点空 / 点到隔壁项"，肉眼看不出来。
        assert_eq!(item_h(14.0), 20.0); // (14*1.3).round()+2 = 18+2
        assert_eq!(item_h(10.0), 15.0); // 13+2
        assert_eq!(item_h(0.0), 3.0, "字号非法时按 1 兜底"); // (1*1.3).round()+2
    }

    #[test]
    fn popup_origin_follows_the_side() {
        let t = rect(100.0, 200.0, 80.0, 24.0);
        assert_eq!(popup_origin(t, PopupSide::Below), Vec2::new(100.0, 226.0));
        assert_eq!(popup_origin(t, PopupSide::Right), Vec2::new(182.0, 200.0));
        // 默认档是 Below（按钮下拉菜单）。
        assert_eq!(PopupSide::default(), PopupSide::Below);
    }

    #[test]
    fn first_frame_uses_the_natural_width() {
        // 首帧（无 prev）**不**定宽 —— 否则自然尺寸会被 1e6 撑爆、再也收敛不回来。
        assert_eq!(popup_content_w(false, None, 140.0, 5.0), None);
        assert_eq!(popup_child_w(false, 123.0), 123.0, "首帧请求自然宽");
        assert_eq!(popup_child_w(true, 123.0), 1.0e6, "定宽后请求极宽（被 clamp 到内容宽）");
    }

    #[test]
    fn content_width_keeps_the_previous_panel_width() {
        // 第 2 帧起：内容宽 = 上一帧面板宽 − 2×(内边距+边框) ⇒ 面板宽**逐帧稳定**。
        assert_eq!(popup_content_w(true, Some(300.0), 140.0, 5.0), Some(290.0));
        // 面板最小宽（触发器宽 / item_min_w）是**总宽**：内容宽扣掉两侧内边距。
        assert_eq!(popup_content_w(true, Some(40.0), 140.0, 5.0), Some(130.0));
        // 下限 40：再窄的高亮 / 文字没有意义。
        assert_eq!(popup_content_w(true, Some(0.0), 0.0, 100.0), Some(40.0));
    }

    #[test]
    fn close_rules_cover_every_combination() {
        // 点菜单项 / Esc ⇒ 一定收起。
        assert!(dropdown_should_close(true, false, false, false));
        assert!(dropdown_should_close(false, false, false, true));
        assert!(dropdown_should_close(true, true, true, true), "项优先于其它条件");
        // 按下在面板外、且不在触发器上（也不在别的浮层上）⇒ 收起。
        assert!(dropdown_should_close(false, true, false, false));
        // 按下在触发器上（切换）或触发器之外的面板 / 子菜单上 ⇒ 不收起。
        assert!(!dropdown_should_close(false, false, true, false));
        assert!(!dropdown_should_close(false, false, false, false), "没按下也没 Esc");
        // ⚠ 子菜单那条：`down_outside` 已经把"落在任意 WIN_TOPMOST 浮层上"排除掉了
        // （见 `popup_show`），所以这里传进来的是 `false` —— 外层菜单不许关。
        assert!(!dropdown_should_close(false, false, true, false));
    }

    #[test]
    fn row_background_prefers_selection_then_hover() {
        let cs = ComboStyle::default();
        assert_eq!(row_bg(true, false, false, &cs), cs.item_selected);
        assert_eq!(row_bg(true, true, true, &cs), cs.item_selected, "选中 > 按下/悬停");
        assert_eq!(row_bg(false, true, false, &cs), cs.item_hover);
        assert_eq!(row_bg(false, false, true, &cs), cs.item_hover);
        assert_eq!(row_bg(false, false, false, &cs), cs.menu_bg);
    }

    #[test]
    fn option_marks_only_the_selected_row() {
        assert!(!option_marked(0, None), "未选择 ⇒ 全不打勾");
        assert!(option_marked(2, Some(2)));
        assert!(!option_marked(1, Some(2)));
    }

    #[test]
    fn popup_padding_comes_from_the_combo_style() {
        let t = Theme::default();
        assert_eq!(popup_padding(&t), t.combo.item_pad_x);
    }
}
