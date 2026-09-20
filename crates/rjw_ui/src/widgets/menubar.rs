//! **菜单栏**（横向）：**一行 + 全宽背景** —— 本质就是 `row`（`PackSide::Left`）多了一条
//! 覆盖整个栏宽（可以是一整块面板 / 整个屏幕）的背景；里面既能放**菜单触发器**，也能放
//! **任何控件**（按钮 / 标签 / 文本输入 / 分割线，含**竖向分割线** [`MenuBar::separator_v`]）。
//!
//! [`MenuBar`] `Deref` 到 [`Pack`](crate::ui::Pack) ⇒ [`UiAdd`](crate::UiAdd) 的方法
//! （`add` / `label` / `button` / `text_input` / `divider` / `min_size` …）**直接可用**，
//! 与 [`MenuCtx`](crate::widgets::menu::MenuCtx) `Deref` 到 [`Window`](crate::Window)
//! 是同一套模式。
//!
//! ⚠ 下拉面板的**录制、样式、宽度、关闭规则全部来自 [`crate::widgets::menu`]**
//! （与 [`Dropdown`](crate::Dropdown) **同一套实现**）——本模块只负责"横向那一行"：
//! 排布、触发器常亮、以及"点栏外收起"里"栏"的判定（点另一个触发器 = 切换，不是收起）。
//!
//! # 用法
//!
//! ```no_run
//! # use rjw_ui::{Ui, UiAdd, Divider, Size};   // `UiAdd`：`bar.label(..)` / `bar.button(..)` 等
//! # use glam::Vec2;
//! # fn f(ui: &mut Ui, mut show_a: bool, mut density: u8) {
//! ui.menu_bar("menubar", Vec2::new(0.0, 0.0), |bar| {
//!     bar.width(Size::Physical(Vec2::new(1280.0, 0.0)));   // 栏宽 = 背景铺多宽
//!     bar.menu("文件", |m| {
//!         if m.item("导入图片…") { /* … */ }
//!         m.separator();
//!         if m.item("退出") { /* … */ }
//!     });
//!     bar.separator_v();                     // ← 竖向分割线（分隔两组菜单）
//!     bar.menu("视图", |m| {
//!         m.item_checked("显示窗口 A", &mut show_a);
//!         m.row(|r| {                        // ← 面板里 `Deref` 到 `Window`：横向排版照旧
//!             if r.button("d0", "紧凑").clicked() { density = 0; }
//!             if r.button("d2", "宽松").clicked() { density = 2; }
//!         });
//!     });
//!     bar.add(Divider::new().vertical());    // ← 等价写法（`separator_v` 就是它）
//!     bar.label("状态：就绪");                 // ← 栏内放任何控件都行
//!     bar.min_size(120.0, 0.0);              // ← 右推用"撑开剩余宽 + 空标签"（同标题栏）
//!     bar.label("");
//! });
//! # }
//! ```
//!
//! # 语义
//!
//! - **同一时刻只有一个菜单展开**（状态在 `UiState::menu_open`：触发器的**绝对 ID**）；
//!   点另一个触发器 = 切换（旧关新开），再点自己 = 收起；
//! - **点菜单项 → 执行 + 自动收起**（[`MenuCtx::item`] / [`MenuCtx::item_checked`] 内部处理）；
//! - **收起条件**（纯函数 [`menu_bar_should_close`]）：点菜单项 / **Esc** / 点**栏外**
//!   （按下不在面板上、不在任何浮层上，**也不在栏矩形内**）—— 点**栏内空白**、点栏里的
//!   竖分割线或别的控件都**不会**收起（"栏 = 一行容器"带来的语义）；
//! - 应用自己的 Esc 语义（如退出）请先看 [`UiState::menu_open`](crate::UiState::menu_open)：
//!   菜单开着的那一帧不要抢；
//! - 下拉面板是 [`Level::Normal`](crate::Level) 的**浮层窗口**（点它不置顶、也不与
//!   "点击置顶"抢 z 序），z 被强制成 [`WIN_TOPMOST`](crate::ui::WIN_TOPMOST) 哨兵 ——
//!   **菜单栏录在哪里都盖得住别的窗口**（不必强求录在各窗口之后）。
//!
//! # 为什么触发器不用 `Button` 控件
//!
//! 菜单栏要按"当前展开项"把那个触发器**一直画成按下态**（鼠标移开也能看出哪个菜单开着）。
//! 用"绘制原语 + `allocate_sense`"比"先量尺寸再 `add_at`"更直接，也省掉一层"`Button`
//! 自己的行高 / 内边距"覆写（栏里统一 `Theme::row_h`）。
//!
//! # 背景画在哪、为什么压得住
//!
//! 背景由 [`Ui::menu_bar`](crate::Ui::menu_bar) 在**子项全部录完之后**才画（`elem = 0`、
//! 但 seq 更大）⇒ 按 `(win, depth, elem, seq)` 排序：压在**同深度已有的底层绘制**（全屏底图 /
//! 面板底色）**之上**、所有控件（子项在 `container` 里 depth + 1）**之下**。
//! ⚠ 它**不**盖在普通窗口（`win > 0`）之上：要"栏恒在最上"就把它录在最后一段（与旧版一致）。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::draw::{CornerRadius, Size, TextAlign, TextVAlign};
use crate::ui::{Pack, UiAdd};
use crate::widgets::menu::{self, MenuFn, PopupSide, PopupSpec};
use crate::widgets::Sense;
use std::ops::{Deref, DerefMut};

/// 菜单内容上下文（定义在 [`crate::widgets::menu`]，此处重导出以保持
/// `crate::widgets::menubar::MenuCtx` 这一旧路径可用）。
pub use crate::widgets::menu::MenuCtx;

/// **栏背景的解算结果**（由 [`Ui::menu_bar`](crate::Ui::menu_bar) 在结算后画）。
pub(crate) struct MenuBarBg {
    pub bg: Color,
    pub border: Color,
    pub border_w: f32,
    pub radius: CornerRadius,
}

/// **栏的收尾事实**（收起判定所需的原始事实 + 背景样式；由 `ui.rs::menu_bar` 拍板）。
pub(crate) struct MenuBarFacts {
    pub item_clicked: bool,
    pub down_outside: bool,
    pub on_trigger: bool,
    pub esc: bool,
    pub action: Option<Option<String>>,
    pub content: Vec2,
    pub popup: Option<Rect>,
    pub width: Option<f32>,
    pub bg: MenuBarBg,
}

/// **收起判定**（纯函数，可单测）：点了菜单项 / Esc / 点**栏外** ⇒ 收起。
///
/// `on_bar` = 按下落在**栏矩形**内（触发器、竖分割线、塞进来的按钮 / 输入框、栏内空白都算）
/// —— 这是"栏"从"一排触发器"升级成"一行容器"后的语义：点栏里的非触发器区域不再关菜单。
/// ⚠ 点**另一个触发器**是"切换"而不是收起：那条路走 `action`（在 `menu()` 里定好），
/// 本函数**不**参与（`ui.rs` 里 `action` 优先）。
#[inline]
pub(crate) fn menu_bar_should_close(
    item_clicked: bool,
    down_outside: bool,
    on_trigger: bool,
    on_bar: bool,
    esc: bool,
) -> bool {
    item_clicked || esc || (down_outside && !on_trigger && !on_bar)
}

/// 菜单栏（由 [`Ui::menu_bar`](crate::Ui::menu_bar) 构造，见模块文档）。
pub struct MenuBar<'ui, 'a> {
    /// **一行** pack（`Deref` 目标：`bar.add(..)` / `bar.button(..)` / `bar.label(..)` /
    /// `bar.text_input(..)` / `bar.divider()` / `bar.min_size(..)` 全部直接可用）。
    pack: Pack<'ui, 'a>,
    /// 栏 ID（触发器 / 下拉窗口 id 的前缀）。
    id: String,
    /// 本帧开始时展开的菜单（触发器绝对 ID）。
    open: Option<String>,
    /// 本帧的展开状态变更（`None` = 不改）。
    action: Option<Option<String>>,
    /// 本帧鼠标是否落在**某个触发器**上（点栏外判定的一部分）。
    on_trigger: bool,
    /// 本帧面板里点了菜单项（⇒ 收起）。
    item_clicked: bool,
    /// 本帧左键按下落在**面板之外**（且不在任何 `WIN_TOPMOST` 浮层上）——共享浮层上报。
    down_outside: bool,
    /// 本帧下拉面板矩形（**栏内局部坐标**；`RJ_MENU_TRACE` 诊断用）。
    popup: Option<Rect>,
    /// 栏**内容**尺寸（子项撑出来的宽 / 高；背景宽度见 `width`）。
    content: Vec2,
    /// 栏宽（**背景覆盖到哪**；`None` = 内容宽）。⚠ 不 clamp 子项宽度。
    width: Option<f32>,
    /// 背景样式覆盖（`None` = `Palette::surface_raised` + `Theme::panel` 的边框 / 圆角）。
    bg: Option<Color>,
    border: Option<Color>,
    border_w: Option<f32>,
    radius: Option<CornerRadius>,
}

impl<'ui, 'a> Deref for MenuBar<'ui, 'a> {
    type Target = Pack<'ui, 'a>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.pack
    }
}

impl<'ui, 'a> DerefMut for MenuBar<'ui, 'a> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.pack
    }
}

impl<'ui, 'a> MenuBar<'ui, 'a> {
    /// 由 [`Ui::menu_bar`](crate::Ui::menu_bar) 调用（应用不直接构造）。
    pub(crate) fn new(pack: Pack<'ui, 'a>, id: &str, open: Option<String>) -> Self {
        Self {
            pack,
            id: id.to_owned(),
            open,
            action: None,
            on_trigger: false,
            item_clicked: false,
            down_outside: false,
            popup: None,
            content: Vec2::ZERO,
            width: None,
            bg: None,
            border: None,
            border_w: None,
            radius: None,
        }
    }

    /// **栏宽**（背景铺到哪；不调 = 子项撑多大就多大）。链式。
    ///
    /// 典型用法是"整条面板 / 屏幕宽"。⚠ 它**只决定背景宽度**，不 clamp 子项：子项仍按自然宽
    /// 从左排；想把某块推到右端就用 `bar.min_size(剩余宽, 0.0); bar.label("")`（同标题栏）。
    pub fn width(&mut self, w: impl Into<Size<Vec2>>) -> &mut Self {
        let w = match w.into() {
            Size::Logical(v) => v * self.pack.ui_mut().scale(),
            Size::Physical(v) => v,
        };
        self.width = Some(w.x.max(0.0));
        self
    }

    /// 背景色覆盖（默认 `Palette::surface_raised`，与窗口标题栏通条同一令牌）。链式。
    pub fn bg(&mut self, c: Color) -> &mut Self {
        self.bg = Some(c);
        self
    }
    /// 背景边框色覆盖（默认 `Theme::panel.border`）。链式。
    pub fn border(&mut self, c: Color) -> &mut Self {
        self.border = Some(c);
        self
    }
    /// 背景边框宽覆盖（默认 `Theme::panel.border_w`；`0` = 不画框）。链式。
    pub fn border_w(&mut self, w: f32) -> &mut Self {
        self.border_w = Some(w.max(0.0));
        self
    }
    /// 背景圆角覆盖（默认 `Theme::panel.radius`；通栏一般给 0 = 直角）。链式。
    pub fn radius(&mut self, r: impl Into<CornerRadius>) -> &mut Self {
        self.radius = Some(r.into());
        self
    }

    /// 加一个菜单：`label` = 栏上的文字，`content` = 展开时下拉面板的内容（闭包上下文）。
    pub fn menu(&mut self, label: &str, content: impl FnOnce(&mut MenuCtx<'_, '_, '_>)) {
        // 触发器样式 = **独立的** `Theme::menubar`（不是 `Theme::button`）：菜单条的触发器
        // 常态**无底色 / 无边框**、只有圆角悬停高亮 —— 用按钮样式会看起来像"一排按钮"
        // （用户实测反馈）。
        let (mb, row_h) = {
            let t = &self.pack.ui_mut().theme;
            (t.menubar.clone(), t.row_h)
        };
        let (fs, fam) = (mb.font_size, mb.font_family.clone());
        let tw = self.pack.ui_mut().text_size(label, fs, fam.as_deref()).x;
        let w = tw + mb.trigger_pad_x * 2.0;
        let trigger_id = format!("{}::{}", self.id, label);
        // **占光标 + 收交互一次做完**（`Sense::DRAG` ⇒ 触发器上的按下即 `claim_press`，
        // 与旧版显式 `claim_press()` 等价）：矩形 = 行内自然宽 × 行高。
        let (rect, resp) =
            self.pack
                .ui_mut()
                .allocate_sense(&trigger_id, Vec2::new(w, row_h), Sense::DRAG);
        self.on_trigger |= resp.hovered;
        if resp.clicked {
            let abs = self.pack.ui_mut().id_for(trigger_id.as_str());
            let next = if self.open.as_deref() == Some(abs.as_str()) {
                None
            } else {
                Some(abs.as_str().to_owned())
            };
            self.action = Some(next);
        }
        // 展开状态：本帧刚点击则以 `action` 为准（点了立刻开 / 关）。
        let abs = self.pack.ui_mut().id_for(trigger_id.as_str());
        let is_open = match &self.action {
            Some(Some(id)) => id == abs.as_str(),
            Some(None) => false,
            None => self.open.as_deref() == Some(abs.as_str()),
        };
        // 触发器外观：开着的**一直亮**（按下态底色），否则 hover / 常态。
        // 常态底色默认**全透明** ⇒ 不推背景命令（少一条命令、也是"纯文字菜单"的观感）。
        let bg = if is_open || resp.pressed {
            mb.trigger_pressed
        } else if resp.hovered {
            mb.trigger_hover
        } else {
            mb.trigger_bg
        };
        if <[f32; 4]>::from(bg)[3] > 0.0 {
            self.pack.ui_mut().push_panel_like(
                rect,
                bg,
                Color::TRANSPARENT,
                0.0,
                mb.trigger_radius,
                1,
            );
        }
        self.pack.ui_mut().push_text_rect(
            rect,
            label,
            fs,
            mb.fg,
            fam,
            TextAlign::Center,
            TextVAlign::Center,
            None,
            None,
        );
        // 记栏内容尺寸（背景宽度缺省 = 它）。
        self.content.x = (rect.x + rect.w).max(self.content.x);
        self.content.y = self.content.y.max(rect.h);

        // 展开时录下拉面板（**共享实现**：见 `crate::widgets::menu`）。
        if is_open {
            let popup_id = format!("{}::{label}", self.id);
            let spec = PopupSpec {
                id: &popup_id,
                trigger: rect,
                side: PopupSide::Below,
                font_size: fs,
                // 面板至少与触发器同宽（`item_min_w` 由控件侧保证，这里补触发器宽）。
                min_w: rect.w,
            };
            let res = menu::popup_show(self.pack.ui_mut(), &spec, MenuFn::new(content));
            self.item_clicked |= res.item_clicked;
            self.down_outside |= res.down_outside;
            self.popup = Some(res.rect);
        }
    }

    /// **竖向分割线**（分隔两组菜单 / 控件）：占位宽 = 线宽 + 2×留白、高 = 一行。
    ///
    /// 就是 `bar.add(Divider::new().vertical())`（`Deref` 到 `Pack` ⇒ `add` 本来也能用），
    /// 单独给一个方法只为**可发现性**；样式取 [`Theme::menubar`](crate::style::Theme::menubar)
    /// 的分割线令牌（不是 `Theme::divider`：菜单条里的竖线更短、更淡）。
    /// ⚠ `Size::Physical`：主题**已按 DPI 预乘**，再走 `Size::Logical` 会被乘第二次。
    pub fn separator_v(&mut self) {
        let mb = self.pack.ui_mut().theme.menubar.clone();
        self.pack.add(
            crate::widgets::Divider::new()
                .vertical()
                .color(mb.separator)
                .thickness(Size::Physical(mb.separator_w))
                .margin(Size::Physical(mb.separator_margin)),
        );
    }

    /// 收尾（由 [`Ui::menu_bar`](crate::Ui::menu_bar) 调用）：返回原始事实 + 背景样式。
    pub(crate) fn finish(mut self) -> MenuBarFacts {
        let (mb, radius) = {
            let t = &self.pack.ui_mut().theme;
            (t.menubar.clone(), t.menubar.radius)
        };
        let esc = self.pack.ui_mut().key_down_edge(winit::keyboard::KeyCode::Escape);
        MenuBarFacts {
            item_clicked: self.item_clicked,
            down_outside: self.down_outside,
            on_trigger: self.on_trigger,
            esc,
            action: self.action.take(),
            content: self.content,
            popup: self.popup,
            width: self.width,
            bg: MenuBarBg {
                bg: self.bg.unwrap_or(mb.bg),
                border: self.border.unwrap_or(mb.border),
                border_w: self.border_w.unwrap_or(mb.border_w),
                radius: self.radius.unwrap_or(radius),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **收起规则逐组合**：只有"点了菜单项 / Esc / （按下在面板外且不在栏内、不在触发器上）"
    /// 收起。这条是"栏"从"一排触发器"升级成"一行容器"后最容易错的地方：点**栏内空白 /
    /// 竖分割线 / 栏里的按钮**都**不该**关菜单（`on_bar = true`）；点另一个触发器是切换，
    /// 由 `action` 处理（本函数不参与）。
    #[test]
    fn close_rules_cover_every_combination() {
        // 点了菜单项 ⇒ 收起（无论别的事实）。
        for down_outside in [false, true] {
            for on_trigger in [false, true] {
                for on_bar in [false, true] {
                    for esc in [false, true] {
                        assert!(
                            menu_bar_should_close(true, down_outside, on_trigger, on_bar, esc),
                            "item_clicked 必须收起"
                        );
                    }
                }
            }
        }
        // Esc ⇒ 收起（优先于一切）。
        assert!(menu_bar_should_close(false, false, false, false, true));
        assert!(menu_bar_should_close(false, true, true, true, true));
        // 点栏外（面板外 + 不在栏内 + 不在触发器上）⇒ 收起。
        assert!(menu_bar_should_close(false, true, false, false, false));
        // 按下落在面板外，但落在**栏内**（空白 / 分割线 / 别的控件）⇒ **不**收起。
        assert!(!menu_bar_should_close(false, true, false, true, false), "栏内空白不收起");
        assert!(!menu_bar_should_close(false, true, true, false, false), "触发器上不收起");
        assert!(!menu_bar_should_close(false, true, true, true, false));
        // 根本没按下（`down_outside = false`）⇒ 怎么都不收起。
        assert!(!menu_bar_should_close(false, false, false, false, false));
        assert!(!menu_bar_should_close(false, false, false, true, false));
    }
}
