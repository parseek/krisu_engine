//! **按钮下拉菜单**（`Dropdown`）：一个触发器按钮 + 点开的**下拉面板**。
//!
//! 这是「下拉框」与「菜单栏下拉」**简并后**的唯一入口 —— 它就是一个普通
//! [`Widget`](crate::Widget)，因此**任何容器里都能用 `UiAdd::add` 加**：
//! `ui.add(..)` / `p.add(..)` / `r.add(..)` / `ui.add_at(pos, ..)`。
//!
//! ```no_run
//! # use rjw_ui::{Dropdown, PopupSide, Ui, UiAdd};
//! # fn f(ui: &mut Ui, mut idx: u32, mut sub: u32, mut filter: String) {
//! // ① 选项列表模式（图一）：引擎自己排菜单项 —— 选中行打勾 + 整行高亮 + 点击写回索引。
//! ui.add(Dropdown::options("diff", "普通", &mut idx, &["简单", "普通", "困难"]));
//!
//! // ② 富内容模式（图二）：菜单内容自己写 —— **菜单内又是 `UiAdd`**
//! //   （文本输入 / 分割线 / 菜单项 / 横向排版 / 甚至再嵌一个下拉当子菜单）。
//! ui.add(Dropdown::new("file", "文件名过滤").menu(|m| {
//!     m.text_input("filter", &mut filter);                       // ← UiAdd
//!     m.separator();                                             // ← 菜单语义
//!     if m.item("导入图片…") { /* 点完自动收起 */ }
//!     m.add(Dropdown::options("sub", "更多", &mut sub, &["A", "B"]) // ← 菜单里再 add
//!             .side(PopupSide::Right));                          //   子菜单（开在右侧）
//! }));
//! # }
//! ```
//!
//! # 两种模式的分工
//!
//! | 模式 | 构造 | 菜单体 |
//! |---|---|---|
//! | 选项列表 | [`Dropdown::options`]（`&mut u32` + `&[&str]`） | 引擎排：每项一行，选中行打勾 + 高亮，点击写回索引并收起；键盘 ↑/↓ 循环 |
//! | 富内容 | [`Dropdown::new`] + [`.menu(..)`](Dropdown::menu) | 应用写的闭包，参数是 [`MenuCtx`]（`Deref` 到 [`Window`] ⇒ 全部 `UiAdd` 方法） |
//!
//! 两者共用同一套浮层录制器与菜单项渲染（[`crate::widgets::menu`]）——
//! 菜单栏（[`Ui::menu_bar`](crate::Ui::menu_bar)）用的也是同一套，只是触发器排成一行。
//!
//! # 语义
//!
//! - 展开状态跨帧持久于 [`UiState::combo_open`](crate::UiState::combo_open)（**本控件的
//!   绝对 ID**）：同一时刻只有一个下拉开着，点另一个 = 旧的关新的开；
//! - 点触发器切换；点菜单项 → 执行 + 收起；点面板外 / Esc → 收起；
//! - 面板是 [`Level::Normal`] 的**浮层窗口**：`WindowClamp::Locked`（拖不动）+
//!   `.resize(false, Resize::None)`（不画缩放柄）+ z 强制成
//!   [`WIN_TOPMOST`](crate::ui::WIN_TOPMOST) 哨兵（恒在一切窗口之上）。
//!
//! # 为什么触发器不用 `Button` 控件
//!
//! 触发器要能**一直显示"我这个下拉正开着"**（鼠标移开也看得出哪个下拉开着），还要在
//! 触发器上做"点击切换 / 点外收起"的判定；直接用绘制原语 + `update_interact` 比
//! "先量尺寸再 `add_at` 一个 Button 再补状态"更直接（菜单栏触发器同一套理由）。

use glam::Vec2;
use rjw_transform::Rect;

use crate::draw::{Icon, Position, Size, TextAlign, TextVAlign};
use crate::focus::FocusKind;
use crate::hit::update_interact;
use crate::style::{ButtonStyle, Theme};
use crate::ui::Ui;
use crate::widgets::menu::{self, MenuContent, MenuCtx, MenuFn, PopupSide, PopupSpec};
use crate::widgets::{Response, Widget};

/// 选项列表模式的**选择状态**（两种入口共用同一套渲染与键盘逻辑）。
enum Sel<'a> {
    /// [`Dropdown::options`]：调用方持有 `&mut u32`。
    One(&'a mut u32),
    /// [`Ui::combo_at`](crate::Ui::combo_at) 的旧语义：可能"一项都没选中"。
    Opt(&'a mut Option<u32>),
}

impl Sel<'_> {
    /// 当前选中项（`None` = 没有一项被标记为选中）。
    #[inline]
    fn selected(&self) -> Option<u32> {
        match self {
            Sel::One(v) => Some(**v),
            Sel::Opt(v) => **v,
        }
    }

    /// 写回选中项。
    #[inline]
    fn set(&mut self, i: u32) {
        match self {
            Sel::One(v) => **v = i,
            Sel::Opt(v) => **v = Some(i),
        }
    }
}

/// 选项列表的来源（`&[&str]` 新入口 / `&[String]` 旧入口）——两种都不分配。
#[derive(Clone, Copy)]
enum Opts<'a> {
    /// `Dropdown::options`：`&["简单", "普通", "困难"]`。
    Strs(&'a [&'a str]),
    /// `Ui::combo_at`：既有的 `&[String]`。
    Owned(&'a [String]),
}

impl Opts<'_> {
    #[inline]
    fn len(&self) -> usize {
        match self {
            Opts::Strs(s) => s.len(),
            Opts::Owned(s) => s.len(),
        }
    }

    #[inline]
    fn get(&self, i: usize) -> &str {
        match self {
            Opts::Strs(s) => s[i],
            Opts::Owned(s) => s[i].as_str(),
        }
    }

    /// 按序迭代选项文本（内部索引实现：两种来源共用一条路径）。
    #[inline]
    fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.len()).map(move |i| self.get(i))
    }
}

/// **按钮下拉菜单**（见模块文档）。
///
/// `F` = 菜单内容渲染器：`()`（选项列表模式 / 空菜单）或 [`MenuFn`]（应用闭包）——
/// 前者由 [`MenuContent`] 的空实现兜底，见 [`MenuContent`] 的 coherence 说明。
pub struct Dropdown<'a, F = ()> {
    id: &'a str,
    /// 触发器上的文字（选项列表模式一般传"当前选项"）。
    label: &'a str,
    /// 选项列表模式的选择状态（`None` = 富内容模式：菜单体由 `content` 写）。
    sel: Option<Sel<'a>>,
    opts: Opts<'a>,
    content: F,
    side: PopupSide,
    /// 触发器固定宽（`None` = 按文字自动：`max(文字宽 + 20, 90) + 2×padding`）。
    width: Option<Size<f32>>,
    /// 字号覆盖（默认 `ButtonStyle::font_size`）。
    font_size: Option<Size<f32>>,
}

impl<'a> Dropdown<'a, ()> {
    /// 空内容的按钮下拉菜单（配 [`.menu(..)`](Dropdown::menu) 用富内容模式）。
    pub fn new(id: &'a str, label: &'a str) -> Self {
        Self {
            id,
            label,
            sel: None,
            opts: Opts::Strs(&[]),
            content: (),
            side: PopupSide::Below,
            width: None,
            font_size: None,
        }
    }

    /// **选项列表模式**：`sel` = 当前选中索引（跨帧由调用方持有），`options` = 选项文字。
    ///
    /// 菜单项由引擎排（选中行打勾 + 高亮），点击写回 `*sel` 并收起；键盘 ↑/↓ 循环切换。
    pub fn options(id: &'a str, label: &'a str, sel: &'a mut u32, options: &'a [&'a str]) -> Self {
        Self {
            id,
            label,
            sel: Some(Sel::One(sel)),
            opts: Opts::Strs(options),
            content: (),
            side: PopupSide::Below,
            width: None,
            font_size: None,
        }
    }

    /// **选项列表模式（可"无选中"）**：供 [`Ui::combo_at`](crate::Ui::combo_at) 的旧入口
    /// 使用（`*sel = None` 时没有一行打勾）。
    pub(crate) fn opt(
        id: &'a str,
        label: &'a str,
        sel: &'a mut Option<u32>,
        options: &'a [String],
    ) -> Self {
        Self {
            id,
            label,
            sel: Some(Sel::Opt(sel)),
            opts: Opts::Owned(options),
            content: (),
            side: PopupSide::Below,
            width: None,
            font_size: None,
        }
    }

    /// **富内容模式**：菜单体用闭包写（参数 [`MenuCtx`] —— 菜单内又是 `UiAdd`）。
    ///
    /// ⚠ 约束 `G: FnOnce(&mut MenuCtx<'_, '_, '_>)` **必须写在这里**（不能只在
    /// `MenuFn<G>: MenuContent` 的 impl 上）：闭包的参数类型靠这个约束推断，少了它
    /// 调用方会报 "type annotations needed（请给闭包参数加类型）"。
    pub fn menu<G>(self, f: G) -> Dropdown<'a, MenuFn<G>>
    where
        G: FnOnce(&mut MenuCtx<'_, '_, '_>),
    {
        Dropdown {
            id: self.id,
            label: self.label,
            sel: self.sel,
            opts: self.opts,
            content: MenuFn::new(f),
            side: self.side,
            width: self.width,
            font_size: self.font_size,
        }
    }
}

impl<'a, F> Dropdown<'a, F> {
    /// **面板方位**（默认 [`PopupSide::Below`]；子菜单用 [`PopupSide::Right`]）。
    pub fn side(mut self, side: PopupSide) -> Self {
        self.side = side;
        self
    }

    /// 触发器**固定宽**（[`Size<f32>`]：逻辑 / 物理；不设 = 按文字自动）。
    pub fn width(mut self, w: impl Into<Size<f32>>) -> Self {
        self.width = Some(w.into());
        self
    }

    /// 字号覆盖（[`Size<f32>`]：逻辑 / 物理；默认 `ButtonStyle::font_size`）。
    pub fn font_size(mut self, s: impl Into<Size<f32>>) -> Self {
        self.font_size = Some(s.into());
        self
    }
}

// ⚠ 只有"能录内容"的 `F` 才有 `show_in` / `Widget`（`()` 与 `MenuFn<闭包>` 两种，
// 见 [`MenuContent`]）——`side` / `width` / `font_size` 这些**纯几何** builder 不受限。
impl<'a, F: MenuContent> Dropdown<'a, F> {
    /// 解析样式与字号（未覆盖回落 [`Theme::button`]）。
    fn resolve(&self, theme: &Theme, scale: f32) -> (ButtonStyle, f32) {
        let st = theme.button.clone();
        let fs = self
            .font_size
            .map(|s| s.to_physical(scale))
            .unwrap_or(st.font_size);
        (st, fs)
    }

    /// 触发器尺寸（`.width(..)` 显式宽 / 自动宽）。
    fn trigger_size(&self, ui: &mut Ui<'_>, st: &ButtonStyle, fs: f32) -> Vec2 {
        let t = ui.text_size(self.label, fs, st.font_family.as_deref());
        let w = self
            .width
            .map(|w| w.to_physical(ui.scale()))
            .unwrap_or((t.x + 20.0).max(90.0) + st.padding.x * 2.0);
        Vec2::new(w, t.y + st.padding.y * 2.0)
    }

    /// 在显式 `rect` 里录控件（[`Widget::ui`] 与 [`Ui::combo_at`](crate::Ui::combo_at) 共用）。
    pub(crate) fn show_in(self, ui: &mut Ui<'_>, rect: Rect) -> Response {
        let (st, fs) = self.resolve(&ui.theme, ui.scale());
        let fam = st.font_family.clone();
        // `id` / `label` 是 `&str`（Copy）——先取出，后面要把 `self` 拆开喂给闭包。
        let id = self.id;
        let label = self.label;
        let Dropdown {
            sel,
            opts,
            content,
            side,
            ..
        } = self;

        let abs = ui.id_for(id);
        let open = ui
            .state()
            .combo_open
            .as_ref()
            .is_some_and(|o| o.as_str() == abs.as_str());
        // 登记焦点链（Tab 可到；Enter/Space 展开收起；↑/↓ 切选项）。
        ui.register_focus(&abs, rect, FocusKind::Combo);
        let hit = ui.hit_abs(&abs, &rect);
        let btn = ui.mouse_left();
        let key_click = ui.key_click(&abs, FocusKind::Combo);
        let mut ev = {
            let ws = ui.state_mut().widget(&abs);
            let ev = update_interact(ws, hit, btn);
            if key_click {
                ws.pressed = true;
            }
            ev
        };
        if key_click {
            ev.clicked = true;
        }
        if ev.pressed {
            ui.note_press_handled();
        }
        if ev.clicked {
            // 切换：开着 ⇒ 收起；关着 ⇒ 展开到本控件（⇒ 同时只可能有一个下拉开着）。
            ui.state_mut().combo_open = if open { None } else { Some(abs.to_static()) };
        }
        let is_open = ui
            .state()
            .combo_open
            .as_ref()
            .is_some_and(|o| o.as_str() == abs.as_str());
        // ── 键盘：焦点下 ↑/↓ 循环切换选项（**选中即收起**，与旧 `combo` 行为一致）──
        // 先取边沿、先写回 `sel`：本帧面板的"选中行"就是切换后的那一行（不再落后一帧）。
        let mut sel = sel;
        let mut key_picked = false;
        if is_open && ui.focused_is(&abs) {
            let n = opts.len() as u32;
            if let Some(s) = sel.as_mut()
                && n > 0
            {
                let cur = s.selected().unwrap_or(0).min(n - 1);
                if ui.key_down_edge(winit::keyboard::KeyCode::ArrowUp) {
                    s.set(if cur == 0 { n - 1 } else { cur - 1 });
                    key_picked = true;
                }
                if ui.key_down_edge(winit::keyboard::KeyCode::ArrowDown) {
                    s.set(if cur + 1 >= n { 0 } else { cur + 1 });
                    key_picked = true;
                }
            }
        }
        // ── 触发器绘制（三态：按下 / 展开 > 悬停 > 常态）──
        let elem = ui.elem_hint();
        let bg = st.pick_bg(ev.pressed || is_open, hit);
        ui.push_panel_like(rect, bg, st.border, st.border_w, st.radius, elem);
        // 文字从 `padding.x` 起（不贴左缘），右侧留 18px 给 ▼；超宽自动省略号截断。
        let text_w = (rect.w - 18.0 - st.padding.x).max(0.0);
        let text_rect = Rect::new(rect.x + st.padding.x, rect.y, text_w, rect.h);
        let ell = ui.ellipsized(label, fs, st.font_family.as_deref(), text_w);
        let shown: &str = ell.as_deref().unwrap_or(label);
        ui.push_text_rect(
            text_rect,
            shown,
            fs,
            st.fg,
            fam,
            TextAlign::Left,
            TextVAlign::Center,
            None,
            None,
        );
        // 箭头用**矢量图标**（不用 "▼" 字形：字体缺字形会走 fallback，宽度也随字体变）。
        ui.icon_at(
            Position::Physical(Vec2::new(rect.x + rect.w - 18.0, rect.y)),
            Size::Physical(Vec2::new(18.0, rect.h)),
            Icon::ChevronDown,
            st.fg,
        );

        // ── 展开：录下拉面板（唯一实现见 `widgets::menu`）──
        if is_open {
            let popup_id = format!("{id}::popup");
            let esc = ui.key_down_edge(winit::keyboard::KeyCode::Escape);
            let min_w = rect.w.max(ui.theme().combo.item_min_w);
            let spec = PopupSpec {
                id: &popup_id,
                trigger: rect,
                side,
                font_size: fs,
                min_w,
            };
            let res = menu::popup_show(ui, &spec, MenuFn::new(move |m: &mut MenuCtx<'_, '_, '_>| {
                match sel.as_mut() {
                    // 选项列表：引擎排（勾选框只在选中行画；选中行整行高亮）。
                    Some(s) => {
                        let cur = s.selected();
                        for (i, opt) in opts.iter().enumerate() {
                            if m.option(opt, menu::option_marked(i, cur)) {
                                // 写回选中索引 ⇒ 收起由 `MenuCtx::item*` 上报
                                // （`res.item_clicked`）——`combo_at` 也靠这次写回判"选了没"。
                                s.set(i as u32);
                            }
                        }
                    }
                    // 富内容：应用自己写（菜单内又是 `UiAdd`）。
                    None => content.render(m),
                }
            }));
            if key_picked
                || menu::dropdown_should_close(res.item_clicked, res.down_outside, hit, esc)
            {
                // 键盘选中即收起（旧 `combo` 语义）；其余按关闭规则四条。
                ui.state_mut().combo_open = None;
            }
        }
        Response {
            hovered: hit,
            pressed: ev.pressed,
            clicked: ev.clicked,
            ..Default::default()
        }
    }
}

impl<F: MenuContent> Widget for Dropdown<'_, F> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (st, fs) = self.resolve(&ui.theme, ui.scale());
        // ① 先量（触发器尺寸：文字宽 + 内边距，或显式 `.width(..)`）
        let size = self.trigger_size(ui, &st, fs);
        // ② 申请：**触发器在窄容器里被截断**（文字走省略号），不把父级撑破 ⇒
        //    `LimitedInParent`（旧 `Widget::expansion()` 的等价物）。
        let rect = ui.allocate_mode(size, crate::widgets::Expansion::LimitedInParent);
        // 被裁剪层完全剔除 ⇒ 直接 return（不镶嵌、不入段：scissor 只省片元）。
        if ui.culled(rect) {
            return Response { rect, culled: true, ..Default::default() };
        }
        // ③ 交互 + 绘制 + 面板（与 `Ui::combo_at` 共用 `show_in`）
        self.show_in(ui, rect)
    }
}

// ─── `Ui` 的显式 rect 入口（**实现体就近放控件自己的文件**）────────

// 搬运说明（`ui.rs` → `widgets/`，路线图 P2a）：公开路径不变（仍是 `Ui::combo_at` /
// `Ui::combo`），实现体搬到本文件；`ui.rs` 只保留"引擎"逻辑。参见
// `widgets/button.rs` 顶部的搬运说明与 `docs/UI_ARCHITECTURE.md` §5.1。
impl Ui<'_> {
    /// **下拉框**（显式 rect；`rect` 为相对当前容器 origin 的局部坐标）。
    ///
    /// ⚠ 本方法现在是 [`Dropdown`](crate::Dropdown) 的**糖**（只有一套浮层实现，
    /// 见 [`crate::widgets::menu`]）——新代码请直接用
    /// `p.add(Dropdown::options(..))`（自动尺寸触发器）或
    /// `p.add(Dropdown::new(..).menu(|m| ..))`（菜单内容自己写，菜单内又是 `UiAdd`）。
    ///
    /// 行为与旧版一致：按钮显示 `current`；点击展开选项浮层；点选项 / 点浮层外 / `Esc`
    /// 收起；`selected` 为当前选中（选中行画方框勾 + 整行高亮）。
    /// 返回本帧新选中的索引（`None` = 无选择 / 未展开）。
    pub fn combo_at(
        &mut self,
        id: &str,
        rect: Rect,
        current: &str,
        options: &[String],
        selected: Option<u32>,
    ) -> Option<u32> {
        // 只有"本帧真的点了某一项"才返回 `Some`（`selected` 可能是 `None`，
        // 也可能被上层夹住；用前后对比而不是"有没有值"）。
        let mut sel = selected;
        Dropdown::opt(id, current, &mut sel, options).show_in(self, rect);
        match sel {
            Some(i) if Some(i) != selected => Some(i),
            _ => None,
        }
    }

    /// **下拉框**（顶层定位：`pos` 相对当前容器内容原点，绝对定位；尺寸自动）。
    pub fn combo(
        &mut self,
        id: &str,
        pos: Vec2,
        current: &str,
        options: &[String],
        selected: Option<u32>,
    ) -> Option<u32> {
        let style = self.theme().button.clone();
        let tsize = self.text_size(current, style.font_size, style.font_family.as_deref());
        let w = (tsize.x + 20.0).max(90.0) + style.padding.x * 2.0;
        let h = style.padding.y * 2.0 + tsize.y;
        let rect = Rect::new(pos.x, pos.y, w, h);
        self.combo_at(id, rect, current, options, selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_state_reads_and_writes_both_modes() {
        // `One`：一定有一项被标记选中。
        let mut idx = 1u32;
        let mut s = Sel::One(&mut idx);
        assert_eq!(s.selected(), Some(1));
        s.set(2);
        assert_eq!(idx, 2);

        // `Opt`：可以"一项都没选中"（旧 `Ui::combo_at(selected = None)` 语义）。
        let mut opt = None::<u32>;
        let mut s = Sel::Opt(&mut opt);
        assert_eq!(s.selected(), None);
        s.set(1);
        assert_eq!(opt, Some(1));
    }

    #[test]
    fn option_source_never_allocates_and_reads_both_forms() {
        let strs: &[&str] = &["a", "b"];
        let owned: Vec<String> = vec!["x".into(), "y".into()];
        let a = Opts::Strs(strs);
        let b = Opts::Owned(&owned);
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 2);
        assert_eq!(a.get(1), "b");
        assert_eq!(b.get(1), "y");
        assert_eq!(a.iter().collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(b.iter().collect::<Vec<_>>(), vec!["x", "y"]);
    }

    #[test]
    fn empty_content_renders_nothing() {
        // 类型级断言：`()` 必须实现 `MenuContent`（否则 `Dropdown::new` 的默认 `F` 不成立）。
        // ⚠ 闭包那半边由编译器把守：`menu(..)` 的 HRTB 边界不成立时，**所有文档示例与
        // 示例程序都编译不过**（`cargo test --doc` 就是它的机器化检查）。
        fn assert_content<T: MenuContent>() {}
        assert_content::<()>();
    }
}
