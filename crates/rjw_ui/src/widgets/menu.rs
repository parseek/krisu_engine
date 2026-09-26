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
//! # 几何契约（四条一起才对齐，缺一条就"看着有点怪"）
//!
//! 1. **内边距 = [`popup_padding`]**（`ComboStyle::item_pad_x`，四边同值）：
//!    菜单项 / `caption` / `separator` / `row` 全部从**同一个内容原点**起排 ⇒ 天然同列。
//!    （别用"给下一子项缩进"的花招：垂直栈里子项 `x` **恒等于内容原点**，缩进宽度无效。）
//! 2. **行距 = [`popup_gap`]**（`MENU_GAP` 逻辑 1px ⇒ `(1.0 × scale).floor()` 物理）：
//!    **不是** [`Theme::gap`] —— 那个是给普通窗口内容用的，菜单行按它排会"每两行之间
//!    空一大截"（用户实测："空位可以缩小为逻辑 1px"）。窗口侧靠
//!    [`WindowBuilder::gap`](crate::WindowBuilder::gap) 传进来。
//! 3. **面板宽 = 上一帧的结算宽**（`UiState::window_sizes`，首帧自然宽、次帧起精确）：
//!    固定宽已知（`fill = true`）时子项请求"极宽"由窗口 clamp 到内容宽 ⇒ **高亮 /
//!    分割线铺满面板**；首帧必须请求**自然宽**，否则 1e6 的请求会把自然尺寸撑成一百万、
//!    面板宽度再也收不回来。见 [`popup_content_w`] / [`popup_child_w`]。
//! 4. **分割线自己画**，不用 [`Divider`](crate::Divider)：`Divider` 的宽 = `avail_w()`，
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
//! 菜单项 / 分割线 / 标题必须同 `x`、同 `w`（实测 `x=8 w=224`，DPI 1.5）。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::draw::{Icon, Position, Size, TextAlign, TextVAlign};
use crate::hit::{hit_test, update_interact};
use crate::layout::Child;
use crate::style::{ComboStyle, PanelStyle, Theme};
use crate::ui::{Level, Ui, UiAdd as _, WindowClamp, is_overlay_z};
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

/// **菜单行间距**（**逻辑**像素）：比 [`Theme::gap`] 紧得多。
///
/// 为什么单独一个值：`Theme::gap` 是给普通窗口内容用的（`--density` 还会再放大它），
/// 菜单行按它排会"每两行之间空一大截"（用户实测："空位可以缩小为逻辑 1px"）。
pub const MENU_GAP: f32 = 1.0;

/// **菜单行间距的物理值** = `(MENU_GAP × scale).floor()`。
///
/// ⚠ 用 **floor 而不是 round**（用户指定的换算）：`1.0 逻辑 @1.5 = 1.5 → 1px`
/// （`.round()` 会变成 2px，行距立刻又"松"了）。公开它是因为示例 / 脚本算"下一行在哪"
/// 要用同一个值（见 `--sim-dropdown` / `--sim-weight-modal`）。
#[inline]
pub fn popup_gap(scale: f32) -> f32 {
    (MENU_GAP * scale).floor()
}

/// **面板相对触发器的方位**。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PopupSide {
    /// 触发器正下方（默认；按钮下拉菜单）。
    #[default]
    Below,
    /// 触发器**正上方**（下方装不下时 [`popup_place`] 会翻到这边）。
    Above,
    /// 触发器右侧（子菜单：菜单里再 `add` 一个 `Dropdown` 时用它）。
    Right,
    /// 触发器**左侧**。
    Left,
}

impl PopupSide {
    /// **对面**那一侧（`Below ↔ Above`、`Right ↔ Left`）——翻转的候选顺序用它。
    pub fn flip(self) -> Self {
        match self {
            PopupSide::Below => PopupSide::Above,
            PopupSide::Above => PopupSide::Below,
            PopupSide::Right => PopupSide::Left,
            PopupSide::Left => PopupSide::Right,
        }
    }
}

/// **面板原点**（触发器矩形 + 方位 ⇒ 面板左上角；纯函数，可单测）。
///
/// 各档之间留 2px 缝隙（面板与触发器不粘连，一眼看出"这是浮层"）。
///
/// ⚠ **只对 `Below` / `Right` 直接可用**（这两侧的原点只由触发器决定）；`Above` / `Left`
/// 要**面板自身的长宽**才能算左上角 ⇒ 请用 [`popup_place`]（它接 `size` 并会挑方位）。
/// 这里对那两侧按"0 尺寸"给退化值（= 触发器那条边 + 2px），仅为 API 完整。
#[inline]
pub fn popup_origin(trigger: Rect, side: PopupSide) -> Vec2 {
    match side {
        PopupSide::Below => Vec2::new(trigger.x, trigger.y + trigger.h + 2.0),
        PopupSide::Above => Vec2::new(trigger.x, trigger.y - 2.0),
        PopupSide::Right => Vec2::new(trigger.x + trigger.w + 2.0, trigger.y),
        PopupSide::Left => Vec2::new(trigger.x - 2.0, trigger.y),
    }
}

/// **把浮层放进屏幕**（纯函数，可单测）：`anchor` = 触发器（**绝对**矩形，物理像素）、
/// `size` = 面板尺寸、`screen` = 屏幕尺寸、`prefer` = 首选方位。
///
/// 返回 `(面板左上角, 实际用的方位)`。规则：
/// 1. **按候选顺序试**：`prefer` → `prefer.flip()` → 其余两个方向；
/// 2. 命中条件 = 面板**完整落进屏幕**（横向允许贴着左右缘）；
/// 3. 横向位置会**夹进屏幕**（先按原 x，超出右缘就左移贴边）——比"整块被挪走"可预期；
/// 4. 四个方向都装不下 ⇒ 用 `prefer` 的位置再夹一次（宁可压边也不跑到屏幕外）。
///
/// 为什么需要它：`ColorPicker` 曾经直接给 `.pos(锚点 + 下移)`，装不下时被
/// `WindowClamp::Screen` **整块搬到别处**（实测：面板高 676 时被翻到屏幕上方，
/// 脚本坐标全落空、点面板外还会把面板关掉）。这里把"该翻到哪"变成**可测的纯函数**。
pub fn popup_place(
    anchor: Rect,
    size: Vec2,
    screen: Vec2,
    prefer: PopupSide,
) -> (Vec2, PopupSide) {
    // 候选顺序：首选 → 对面 → 另外两侧（`Right/Left` 优先于 `Below/Above` 之外的组合）。
    let mut candidates: Vec<PopupSide> = vec![prefer, prefer.flip()];
    for s in [PopupSide::Below, PopupSide::Above, PopupSide::Right, PopupSide::Left] {
        if !candidates.contains(&s) {
            candidates.push(s);
        }
    }
    // 无尺寸信息也能算 Below/Right；Above/Left 要用 size。
    let place = |side: PopupSide| -> Vec2 {
        match side {
            PopupSide::Below => Vec2::new(anchor.x, anchor.y + anchor.h + 2.0),
            PopupSide::Above => Vec2::new(anchor.x, anchor.y - size.y - 2.0),
            PopupSide::Right => Vec2::new(anchor.x + anchor.w + 2.0, anchor.y),
            PopupSide::Left => Vec2::new(anchor.x - size.x - 2.0, anchor.y),
        }
    };
    // 横向夹进屏幕（保持"贴边"而不是被整块挪走）；纵向不夹 —— 纵向靠**换方位**解决。
    let clamp_x = |p: Vec2| Vec2::new(p.x.clamp(0.0, (screen.x - size.x).max(0.0)), p.y);
    let fits = |p: Vec2| {
        p.x >= 0.0 && p.x + size.x <= screen.x && p.y >= 0.0 && p.y + size.y <= screen.y
    };
    for side in candidates {
        let p = clamp_x(place(side));
        if fits(p) {
            return (p, side);
        }
    }
    (clamp_x(place(prefer)), prefer)
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

/// **子菜单内容**（[`Item::submenu`] 里装箱的那种 `for<>` trait object）。
///
/// 装箱的形态让 [`Item`] 不必带泛型参数（⇒ `m.item(Item::new(..).submenu(..))` 与
/// `m.item("文本")` 能共用同一个入口）。
impl MenuContent for Box<dyn for<'w, 'x, 'y> FnOnce(&mut MenuCtx<'w, 'x, 'y>) + '_> {
    #[inline]
    fn render(self, m: &mut MenuCtx<'_, '_, '_>) {
        (self)(m);
    }
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
    let cs = ui.theme().combo.clone();
    let pad = popup_padding(ui.theme());
    let border_w = ui.theme().panel.border_w;
    let pad_total = pad + border_w;
    // 面板样式 = **菜单外观**（`ComboStyle`）+ 主题面板的投影 / 边框宽（`border_w` 走
    // 主题令牌，不再像早期那样硬编码 1.0）。
    let style = PanelStyle {
        bg: cs.menu_bg.into(),
        border: cs.menu_border,
        padding: pad,
        radius: cs.menu_radius,
        bg_image: None,
        ..ui.theme().panel.clone()
    };
    let pos = popup_origin(spec.trigger, spec.side);
    // **浮层 z（基址 + 嵌套层数）**：恒在一切窗口之上，且**子浮层整段画在父浮层之上**
    // （同一 z 会让两层的命令落进同一个 `(win, elem)` 分组，子层阴影被父层控件盖住 ——
    // 用户实测："下级 popup 阴影被绘制在了上级控件后面"）。
    // ⚠ 键必须是**绝对 id**（`window_impl` 内部按当前栈解析出同一前缀）。
    let abs = ui.id_for(spec.id);
    let overlay = ui.push_overlay_z();
    ui.state_mut()
        .window_z
        .insert(abs.to_static(), overlay);
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
    // 行距先算好（`ui.scale()` 要在 `ui.window(..)` 的可变借用**之前**取）。
    let menu_gap = popup_gap(ui.scale());
    let mut b = ui
        .window(spec.id)
        .pos(Position::Physical(pos))
        // **锁定位置 + 不允许拖拽缩放**：浮层是"某个控件的展开部分"，拖走了就与触发
        // 器脱节（命中按窗口走、视觉却跑别处 —— 用户实测"控件严重错位"）；尺寸由内容
        // 定，也不该出现缩放柄。`.width(..)` 只用来把内容宽钉在上一帧结算值上。
        .clamp(WindowClamp::Locked)
        .resize(false)
        // **行距 = 逻辑 1px 的物理值**（不是 `Theme::gap`）：菜单行必须紧挨着。
        .gap(Size::Physical(menu_gap))
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
    ui.pop_overlay();
    let rect = Rect::new(pos.x, pos.y, size.x, size.y);
    // 点外判定：面板矩形与鼠标**同一坐标系**（都是当前容器局部），用 `mouse_local()`；
    // "落在**任意浮层**上"（`z >= WIN_TOPMOST`，含子菜单 / 更深层）用公开的
    // `window_under_mouse()`——子菜单不关外层。
    let mouse = ui.mouse_local();
    let on_popup = hit_test(&rect, mouse);
    let on_overlay = ui
        .window_under_mouse()
        .is_some_and(|(_, z)| is_overlay_z(z));
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

/// **子菜单该不该展开**（纯函数，真值表）：
/// `hit`（鼠标在本行）⇒ 开；已开且鼠标还在**本子面板或其后代**里 ⇒ 保持；否则收起。
///
/// ⇒ 悬停即开；鼠标移进子面板保持；移到同菜单其他行 / 移出菜单自动收起
/// （不需要其他行配合，也不需要一个全局"当前子菜单"槽位）。
#[inline]
fn submenu_open_now(hit: bool, was_open: bool, inside_tree: bool) -> bool {
    hit || (was_open && inside_tree)
}

/// `id` 是否等于 `root` 或位于 `root` 的**窗口子树**里。
///
/// 窗口 id 用 `/` 分段（`with_id` 压栈时拼 `前缀/相对名`），所以祖先判定必须**按 `/` 边界**：
/// 裸 `starts_with` 会把兄弟窗口 `a/b/subX` 误判成 `a/b/sub` 的后代
/// （⇒ 鼠标停在兄弟面板上时子菜单不会收起）。
#[inline]
fn id_in_window_tree(id: &str, root: &str) -> bool {
    id == root
        || (id.len() > root.len()
            && id.starts_with(root)
            && id.as_bytes()[root.len()] == b'/')
}

/// 鼠标是否落在**某个窗口子树**（`root` = 窗口绝对 id）的任何窗口矩形内。
///
/// 用 [`UiState::windows()`](crate::UiState::windows) 模块视图的 `rects()`（**绝对**矩形，
/// 键 = 窗口绝对 ID）——它在录制期写入、帧末只保留本帧录制的窗口 ⇒ 面板消失后自然不再
/// 命中（不需要额外清理）。
fn mouse_in_window_tree(ui: &Ui<'_>, root: &str) -> bool {
    let m = ui.mouse_screen();
    ui.state()
        .windows()
        .rects()
        .any(|(id, r)| id_in_window_tree(id.as_str(), root) && r.contains_point(m))
}

/// 一行的几何 / 交互结果（[`MenuCtx::item`] 用它录子菜单面板）。
struct RowInfo {
    /// 行矩形（**当前容器局部**坐标）。
    rect: Rect,
    /// 行的**相对 id**（子面板 id = `<它>::sub`）。
    rel_id: String,
    /// 本帧是否被点击。
    clicked: bool,
    /// 鼠标是否在本行上（Hover）。
    hit: bool,
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

/// **菜单项点击行为**（责任链上的一档：`.click_behavior(..)`）。
///
/// 用户建议的"行为 flag"：点完这一项之后，整个 popup 是**收起**还是**保留**。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuClick {
    /// 点击后**收起整个 popup**（普通菜单项；默认）。
    #[default]
    Close,
    /// 点击后**保留 popup**（"点了还要继续操作"的项：开关 / 连续多选 / 微调）。
    ///
    /// ⚠ [`Item::submenu`] 恒为"不收起"（子菜单靠 Hover 展开，点击不该关掉整条链），
    /// 与该档位无关。
    Keep,
}

/// **菜单项**（责任链 builder）：`m.item(Item::new("导入图片…"))`。
///
/// ```no_run
/// # use rjw_ui::{Item, MenuClick, Ui};
/// # use rjw_ui::widgets::menu::MenuCtx;
/// # fn f(m: &mut MenuCtx<'_, '_, '_>, mut show_a: bool) {
/// if m.item("导入图片…") { /* 旧写法：点击即收起 */ }
/// // 责任链：点击后保留 popup
/// if m.item(Item::new("保持打开").click_behavior(MenuClick::Keep)) { }
/// // 责任链：勾选项（自持状态，点击直接翻转）
/// m.item(Item::new("窗口 A 显示").checked(&mut show_a));
/// // 责任链：**子菜单**（`Submenu` 形态）—— Hover 在原 popup **右侧**展开
/// m.item(Item::new("编码").submenu(|s| { if s.item("UTF-8") { } }));
/// # }
/// ```
///
/// # 为什么是 builder 而不是 `Widget`
///
/// 菜单行必须与 [`MenuCtx`] 手里的语义绑定：行高 `item_h` 与内边距、高亮铺满内面板、
/// "点击是否写 `popup_show` 的 `close` 句柄"、子菜单的跨帧悬停状态、`fill` 宽度收敛。
/// 做成 `Widget` 就得给 `UiState` 再开一条"点击请求 → 由 popup 消费"的隐式通道。
/// 所以 `Item` 只承载**配置**（责任链），执行留在 [`MenuCtx::item`]。
///
/// `From<&str>`：旧的 `m.item("文本")` 写法继续可用（等价 `Item::new("文本")`）。
pub struct Item<'a> {
    label: &'a str,
    /// 点击行为（默认 [`MenuClick::Close`]）。
    click: MenuClick,
    /// 勾选项：当前是否勾选（点击时**由 `MenuCtx::item` 翻转**）。
    checked: Option<&'a mut bool>,
    /// **子菜单**（`Submenu` 形态）的内容。
    ///
    /// 用**显式 `for<>` 的 trait object`**（不是泛型参数）：闭包进结构体字段时，`'_` 形式的
    /// HRTB 边界不好写；显式 late-bound 是标准且一定可行的写法（每个子菜单项每帧一次分配，
    /// 数量级可忽略）。
    submenu: Option<Box<dyn for<'w, 'x, 'y> FnOnce(&mut MenuCtx<'w, 'x, 'y>) + 'a>>,
}

impl<'a> Item<'a> {
    /// 普通菜单项（点击即收起）。
    pub fn new(label: &'a str) -> Self {
        Self { label, click: MenuClick::Close, checked: None, submenu: None }
    }

    /// **点击行为**（[`MenuClick::Close`]（默认）/ [`MenuClick::Keep`]）。
    pub fn click_behavior(mut self, b: MenuClick) -> Self {
        self.click = b;
        self
    }

    /// **勾选项**：左侧方框勾选（`&mut bool` 自持），点击时**直接翻转**。
    pub fn checked(mut self, on: &'a mut bool) -> Self {
        self.checked = Some(on);
        self
    }

    /// **子菜单**（`Submenu` 形态）：行右侧画 ▸，**Hover 时在行的右边**展开 `content`。
    ///
    /// 与 [`Self::checked`] 互斥（同时用会 `debug_assert` 失败）。
    pub fn submenu(mut self, f: impl FnOnce(&mut MenuCtx<'_, '_, '_>) + 'a) -> Self {
        debug_assert!(self.checked.is_none(), "Submenu 与 checked 不能同时设置");
        self.submenu = Some(Box::new(f));
        self
    }

    /// 本项是否是子菜单（`Submenu`）形态。
    #[inline]
    pub fn is_submenu(&self) -> bool {
        self.submenu.is_some()
    }
}

impl<'a> From<&'a str> for Item<'a> {
    #[inline]
    fn from(label: &'a str) -> Self {
        Item::new(label)
    }
}

impl std::fmt::Debug for Item<'_> {
    /// 手写 `Debug`：字段里的 boxed `FnOnce` 没有 `Debug`（只打印"有没有"）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Item")
            .field("label", &self.label)
            .field("click", &self.click)
            .field("checked", &self.checked.is_some())
            .field("submenu", &self.submenu.is_some())
            .finish()
    }
}

impl MenuCtx<'_, '_, '_> {
    /// **菜单项**（占一行；责任链配置见 [`Item`]）：返回本帧是否被点击。
    ///
    /// - 整行可点（不是只有文字），hover / 按下整行高亮（与选项行同一观感）；
    /// - 点击后是否收起整个 popup 由 [`Item::click_behavior`] 决定（默认 [`MenuClick::Close`]）；
    /// - [`Item::checked`]：勾选框自绘 + 点击翻转 `&mut bool`；
    /// - [`Item::submenu`]：右侧 ▸ + Hover 展开（点击**不**收起）。
    pub fn item<'i>(&mut self, item: impl Into<Item<'i>>) -> bool {
        let Item { label, click, checked, submenu } = item.into();
        // 勾选框状态：`Some(当前值)` ⇒ 画方框（勾/空），否则整列留白（文字仍对齐）。
        let check = checked.as_ref().map(|b| **b);
        // **Submenu**：行右画 ▸，且**点击不收起**（子菜单靠 Hover 展开，点它不该关掉整条链）。
        let trailing = submenu.is_some().then_some(Icon::ChevronRight);
        let close_on_click = click == MenuClick::Close && submenu.is_none();
        let row = self.item_core(label, check, false, close_on_click, trailing);
        if row.clicked && let Some(b) = checked {
            *b = !*b;
        }
        if let Some(content) = submenu {
            let clicked = row.clicked;
            self.submenu_panel(row, content);
            return clicked;
        }
        row.clicked
    }

    /// **带勾选的菜单项**（视图显隐这类）：点击直接翻转 `&mut bool` 并收起。
    ///
    /// 勾选标记是**方框**（矢量画的圆角方框 + 勾号，`border_w = 0` 时退化成实心底）——
    /// 不用 "✓" 字形：字体缺字形时菜单里会出现豆腐块。
    ///
    /// = `m.item(Item::new(label).checked(checked))`（责任链写法见 [`Item`]）。
    pub fn item_checked(&mut self, label: &str, checked: &mut bool) -> bool {
        self.item(Item::new(label).checked(checked))
    }

    /// **子菜单**（`Submenu`）：`label` 行的右边 Hover 展开 `content`。
    ///
    /// = `m.item(Item::new(label).submenu(content))`；语义见 [`Self::item`] 与模块文档。
    pub fn submenu(&mut self, label: &str, content: impl FnOnce(&mut MenuCtx<'_, '_, '_>)) {
        self.item(Item::new(label).submenu(content));
    }

    /// **选项行**（选项列表模式；`Dropdown::options` 用）：`marked` = 本行是当前选中。
    ///
    /// 与 [`Self::item_checked`] 的差别只有两点：勾选框**只在选中时画**（不选中不画空框，
    /// 与旧下拉框的 ✓ 观感一致），选中行整行用 `ComboStyle::item_selected` 高亮。
    /// 方框列**恒留位**（两类项文字都对得齐）。
    pub(crate) fn option(&mut self, label: &str, marked: bool) -> bool {
        self.item_core(label, marked.then_some(true), marked, true, None).clicked
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

    /// 菜单项 / 选项行 / 子菜单行的公共实现：`check = Some(是否勾选)` 时左侧画一个
    /// **方框勾选框**，`trailing = Some(icon)` 时在行右缘画一个尾部图标（子菜单的 ▸）。
    ///
    /// 勾选标记是**方框**（与 [`crate::Checkbox`] 同一观感：圆角方框 + 勾选时填
    /// `CheckboxStyle::checked_fill` + 矢量勾号），画在**菜单项内容里**、方框列**恒留位**
    /// （勾选与否文字都对齐）。为什么不用"左内边距里放一个 ✓"：那需要很大的左内边距
    /// （用户实测："边距太大"），而方框只占 `checkbox.box_size`，可以贴着小边距放。
    ///
    /// `close_on_click = false` ⇒ 点击**不**收起 popup（[`MenuClick::Keep`] / 子菜单行）。
    ///
    /// ⚠ 名字刻意不叫 `row`：`MenuCtx` 经 `Deref` 到 [`Window`]，同名私有方法会**遮蔽**
    /// `Window::row`（deref 方法优先级更低）⇒ 应用的 `m.row(..)` 会去调私有那个。
    fn item_core(
        &mut self,
        label: &str,
        check: Option<bool>,
        selected: bool,
        close_on_click: bool,
        trailing: Option<Icon>,
    ) -> RowInfo {
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
        // **尾部图标**（子菜单的 ▸）：贴内容**右缘**、垂直居中、与项文字同色。
        if let Some(icon) = trailing {
            let d = (box_size * 0.8).max(8.0);
            ui.icon_at(
                Position::Physical(Vec2::new(
                    rect.x + rect.w - box_size,
                    rect.y + (rect.h - d) * 0.5,
                )),
                Size::Physical(Vec2::splat(d)),
                icon,
                cs.fg,
            );
        }
        if ev.clicked && close_on_click {
            *self.close = true;
        }
        self.trace("item", rect);
        RowInfo { rect, rel_id: item_id, clicked: ev.clicked, hit }
    }

    /// **子菜单面板**（[`Item::submenu`] / [`Self::submenu`] 的实现）：
    /// Hover 在**行的右侧**（[`PopupSide::Right`]）展开、跨帧保持。
    ///
    /// 状态**每行自持**（[`WidgetState::submenu_open`](crate::WidgetState)）——
    /// ⚠ **绝不碰** `UiState::combo_open`：那是**父下拉**的槽位，早期"菜单里嵌一个
    /// `Dropdown`"的做法就是被它覆盖，症状是"点一下整条 popup 消失"。
    fn submenu_panel(
        &mut self,
        row: RowInfo,
        content: Box<dyn for<'w, 'x, 'y> FnOnce(&mut MenuCtx<'w, 'x, 'y>) + '_>,
    ) {
        let sub_rel = format!("{}::sub", row.rel_id);
        let min_w = self.w.ui_mut().theme.combo.item_min_w;
        // ① 展开状态：Hover 即开；已开且鼠标还在**本子面板或其子孙窗口**里 ⇒ 保持；否则收起。
        let abs = self.w.ui_mut().id_for(row.rel_id.as_str());
        let sub_abs = self.w.ui_mut().id_for(sub_rel.as_str());
        let inside = mouse_in_window_tree(self.w.ui_mut(), sub_abs.as_str());
        let was = self.w.ui_mut().state_mut().widget(&abs).submenu_open;
        let open = submenu_open_now(row.hit, was, inside);
        self.w.ui_mut().state_mut().widget(&abs).submenu_open = open;
        if !open {
            return;
        }
        // ② 录子面板：触发器 = 行矩形，方位 = 行**右侧**（与行顶对齐，2px 缝隙）。
        let spec = PopupSpec {
            id: &sub_rel,
            trigger: row.rect,
            side: PopupSide::Right,
            font_size: self.font_size,
            min_w,
        };
        let res = popup_show(self.w.ui_mut(), &spec, content);
        // ③ **链式收起**：子面板里点了项 ⇒ 冒泡给父的 `close` ⇒ 整条链（子 + 父）一起收。
        if res.item_clicked {
            *self.close = true;
        }
        // ④ 点在子面板外（但仍在父面板里）⇒ 只收子菜单（父菜单按自己的规则处理）。
        if res.down_outside {
            self.w.ui_mut().state_mut().widget(&abs).submenu_open = false;
        }
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
    fn popup_place_prefers_below_then_flips_above_then_clamps() {
        let screen = Vec2::new(1000.0, 800.0);
        let size = Vec2::new(200.0, 300.0);
        // 下方装得下 ⇒ 用首选（Below），且 x 与锚点对齐。
        let a = Rect::new(100.0, 100.0, 80.0, 30.0);
        assert_eq!(popup_place(a, size, screen, PopupSide::Below), (Vec2::new(100.0, 132.0), PopupSide::Below));
        // 下方装不下（锚点贴近屏幕底）⇒ **翻到上方**（这正是旧的 `WindowClamp` 会
        // "整块搬到别处"、脚本坐标全落空的场景）。
        let b = Rect::new(100.0, 700.0, 80.0, 30.0);
        assert_eq!(popup_place(b, size, screen, PopupSide::Below), (Vec2::new(100.0, 398.0), PopupSide::Above));
        // 贴右缘 ⇒ **横向夹进屏幕**（贴边而不是整块挪走）。
        let c = Rect::new(950.0, 100.0, 40.0, 30.0);
        let (p, side) = popup_place(c, size, screen, PopupSide::Below);
        assert_eq!(side, PopupSide::Below);
        assert_eq!(p.x, 800.0, "右缘贴边 = screen.w − size.w");
        // 到处都装不下（面板比屏幕还高）⇒ 仍用首选方位、只夹横向（宁可压边不出屏）。
        let huge = Vec2::new(200.0, 900.0);
        let (p, side) = popup_place(a, huge, screen, PopupSide::Below);
        assert_eq!(side, PopupSide::Below);
        assert!(p.x >= 0.0 && p.x + huge.x <= screen.x);
    }

    #[test]
    fn popup_gap_is_floor_not_round() {
        // 菜单行距 = **逻辑 1px** 的物理值，**向下取整**：1.0 逻辑 @1.5 = 1px。
        // （若用 `.round()` 会得到 2px —— 行距立刻又"松"回去，正是用户报的那个观感。）
        assert_eq!(popup_gap(1.0), 1.0);
        assert_eq!(popup_gap(1.5), 1.0, "1.5 → floor = 1（round 会是 2）");
        assert_eq!(popup_gap(2.0), 2.0);
        assert_eq!(popup_gap(1.25), 1.0);
        // 全物理、与主题 gap 无关：主题调密度也不影响菜单行距。
        assert_eq!(popup_gap(0.9), 0.0, "小于 1 物理像素时贴紧（0 = 行挨着行）");
    }

    #[test]
    fn popup_padding_comes_from_the_combo_style() {
        let t = Theme::default();
        assert_eq!(popup_padding(&t), t.combo.item_pad_x);
    }

    #[test]
    fn submenu_opens_on_hover_and_stays_while_inside() {
        // 真值表：`hit`（悬停在行上）⇒ 开；已开且鼠标还在本子面板（或后代）里 ⇒ 保持；
        // 其余 ⇒ 收起（鼠标移到同菜单其他行 / 移出菜单时自动收起，无需其他行配合）。
        assert!(submenu_open_now(true, false, false), "悬停即开");
        assert!(submenu_open_now(true, true, false), "悬停且已开 ⇒ 保持");
        assert!(submenu_open_now(false, true, true), "鼠标进子面板 ⇒ 保持");
        assert!(!submenu_open_now(false, true, false), "鼠标离开 ⇒ 收起");
        assert!(!submenu_open_now(false, false, true), "没悬停过就不会被'在里面'打开");
    }

    #[test]
    fn window_tree_prefix_respects_the_slash_boundary() {
        // 祖先判定必须按 `/` 边界：裸 `starts_with` 会把**兄弟**窗口误判成后代
        // （⇒ 鼠标停在兄弟面板上时子菜单不会收起）。
        let root = "dd_file::popup/item::编码::sub";
        assert!(id_in_window_tree(root, root), "自己是自己的子树");
        assert!(
            id_in_window_tree(&format!("{root}/item::ASCII::sub"), root),
            "子孙窗口"
        );
        assert!(!id_in_window_tree(&format!("{root}X"), root), "同名前缀的兄弟（无 /）");
        assert!(!id_in_window_tree("dd_file::popup/item::其他", root), "无关窗口");
        assert!(!id_in_window_tree("dd_file::popup", root), "祖先不算后代");
    }

    #[test]
    fn item_builder_defaults_are_the_plain_closing_item() {
        let it = Item::new("导入图片…");
        assert_eq!(it.click, MenuClick::Close, "默认点击即收起");
        assert!(it.checked.is_none() && it.submenu.is_none());
        assert!(!it.is_submenu());
        // 责任链各档：`Keep` / `checked` / `submenu`。
        assert_eq!(
            Item::new("x").click_behavior(MenuClick::Keep).click,
            MenuClick::Keep
        );
        let mut on = false;
        assert!(Item::new("x").checked(&mut on).checked.is_some());
        let sub = Item::new("编码").submenu(|_s| {});
        assert!(sub.is_submenu());
        // `From<&str>`：旧写法 `m.item("文本")` 与 builder 等价。
        assert_eq!(Item::from("文本").label, "文本");
        assert_eq!(MenuClick::default(), MenuClick::Close, "默认档是 Close");
    }
}
