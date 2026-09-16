//! **菜单栏**（横向）：一排触发器 + 点开的**下拉面板**；面板内容是一个闭包上下文
//! （`item` / `item_checked` / `caption` / `separator`，并 `Deref` 到 [`Window`]，
//! 所以 `label` / `button` / `divider` / `row` / `text_input` / `add(..)` 都能用）。
//!
//! # 用法
//!
//! ```no_run
//! # use rjw_ui::{Ui, UiAdd};   // `UiAdd`：`m.label(..)` / `m.row(..)` / `m.text_input(..)` 等
//! # use glam::Vec2;
//! # fn f(ui: &mut Ui, mut show_a: bool, mut density: u8) {
//! ui.menu_bar("menubar", Vec2::new(620.0, 12.0), |bar| {
//!     bar.menu("文件", |m| {
//!         if m.item("导入图片…") { /* … */ }
//!         m.separator();
//!         if m.item("退出") { /* … */ }
//!     });
//!     bar.menu("视图", |m| {
//!         m.item_checked("显示窗口 A", &mut show_a);
//!         m.caption("密度");
//!         m.row(|r| {                       // ← `Deref` 到 `Window`：横向排版照旧
//!             if r.button("d0", "紧凑").clicked() { density = 0; }
//!             if r.button("d2", "宽松").clicked() { density = 2; }
//!         });
//!     });
//! });
//! # }
//! ```
//!
//! # 语义
//!
//! - **同一时刻只有一个菜单展开**（状态在 `UiState::menu_open`：触发器的**绝对 ID**）；
//!   点另一个触发器 = 切换（旧关新开），再点自己 = 收起；
//! - **点菜单项 → 执行 + 自动收起**（[`MenuCtx::item`] / [`MenuCtx::item_checked`] 内部处理）；
//! - **点栏外**（既不在任何触发器上、也不在下拉面板里）→ 收起；
//! - **Esc** → 收起。应用自己的 Esc 语义（如退出）请先看
//!   [`UiState::menu_open`](crate::UiState::menu_open)：菜单开着的那一帧不要抢；
//! - 下拉面板是 [`Level::Normal`] 的**浮层窗口**（点它不置顶、也不与"点击置顶"抢 z 序）。
//!   ⚠ 想让它盖住别的窗口就**把菜单栏录在各模块之后**（窗口 z 在**首次录制**时按
//!   `max+1` 分配）——示例里菜单栏录在段 2 的模块之后、字体弹窗之前。
//!
//! # 为什么触发器不用 `Button` 控件
//!
//! 菜单栏要按"当前展开项"把那个触发器**一直画成按下态**（鼠标移开也能看出哪个菜单开着），
//! 还要把触发器矩形收集起来做"点栏外收起"判定。直接用绘制原语 + `update_interact`
//! （与 `combo` 触发按钮同一套写法）比"先量尺寸再 `add_at`"更直接。
//!
//! # 为什么菜单栏**手排**而不是用 `pack`
//!
//! 每个 `menu()` 可能在**同一帧内**立刻录下拉窗口（展开时）——而 `pack_at` 的闭包里录
//! 窗口会把浮层算进 pack 的结算尺寸（栏被撑成浮层那么大）。手排一个 `x` 光标就没有这个
//! 问题，且位置语义与顶层 `*_at` 一致（顶层 `abs_base = 0` ⇒ 局部坐标即屏幕坐标）。

use glam::Vec2;
use rjw_transform::Rect;

use crate::draw::{Icon, Position, TextAlign, TextVAlign};
use crate::hit::{hit_test, update_interact};
use crate::layout::Child;
use crate::style::PanelStyle;
use crate::ui::{Level, Ui, UiAdd, WindowClamp, WIN_TOPMOST};
use crate::Window;

/// 菜单栏（由 [`Ui::menu_bar`](crate::Ui::menu_bar) 构造，见模块文档）。
pub struct MenuBar<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
    /// 栏 ID（触发器 / 下拉窗口 id 的前缀）。
    id: String,
    /// 下一个触发器的左上角（**屏幕物理坐标**；顶层 `abs_base = 0` ⇒ 局部即屏幕）。
    cursor: Vec2,
    /// 栏顶 y（换行用：本版本单行，留字段给以后多行）。
    row_y: f32,
    /// 本帧开始时展开的菜单（触发器绝对 ID）。
    open: Option<String>,
    /// 本帧的展开状态变更（`None` = 不改）。
    action: Option<Option<String>>,
    /// 本帧鼠标是否落在某个触发器上（点栏外判定）。
    on_trigger: bool,
    /// 本帧下拉面板的屏幕矩形（点栏外判定）。
    popup: Option<Rect>,
    /// 本帧点了菜单项（⇒ 收起）。
    item_clicked: bool,
    /// 栏尺寸（返回给调用方，供布局 / 命中参考）。
    size: Vec2,
}

impl<'ui, 'a> MenuBar<'ui, 'a> {
    /// 由 [`Ui::menu_bar`](crate::Ui::menu_bar) 调用（应用不直接构造）。
    pub(crate) fn new(ui: &'ui mut Ui<'a>, id: &str, pos: Vec2, open: Option<String>) -> Self {
        Self {
            ui,
            id: id.to_owned(),
            cursor: pos,
            row_y: pos.y,
            open,
            action: None,
            on_trigger: false,
            popup: None,
            item_clicked: false,
            size: Vec2::ZERO,
        }
    }

    /// 加一个菜单：`label` = 栏上的文字，`content` = 展开时下拉面板的内容（闭包上下文）。
    pub fn menu(&mut self, label: &str, content: impl FnOnce(&mut MenuCtx<'_, '_, '_>)) {
        let (gap, font_size, family, w, h) = {
            let t = self.ui.theme.clone();
            let pad_x = t.button.padding.x;
            let fs = t.button.font_size;
            let fam = t.button.font_family.clone();
            let tw = self.ui.text_size(label, fs, fam.as_deref()).x;
            (t.gap, fs, fam, tw + pad_x * 2.0, t.row_h)
        };
        let rect = Rect::new(self.cursor.x, self.row_y, w, h);
        let trigger_id = format!("{}::{}", self.id, label);
        let abs = self.ui.id_for(trigger_id.as_str());
        // 命中 / 交互（`hit_abs` 内部处理窗口遮挡 / clip / 控件级遮挡登记）。
        let hit = self.ui.hit_abs(&abs, &rect);
        let btn = self.ui.mouse_left();
        if btn.down_edge() && hit {
            // 触发器上的按下是"菜单自己的按下"：不要被外层容器当成拖拽基准。
            self.ui.claim_press();
        }
        let ev = {
            let ws = self.ui.state_mut().widget(&abs);
            update_interact(ws, hit, btn)
        };
        self.on_trigger |= hit;
        if ev.clicked {
            let next = if self.open.as_deref() == Some(abs.as_str()) {
                None
            } else {
                Some(abs.as_str().to_owned())
            };
            self.action = Some(next);
        }
        // 展开状态：本帧刚点击则以 `action` 为准（点了立刻开 / 关）。
        let is_open = match &self.action {
            Some(Some(id)) => id == abs.as_str(),
            Some(None) => false,
            None => self.open.as_deref() == Some(abs.as_str()),
        };
        // 触发器外观：开着的**一直亮**（按下态底色），否则 hover / 常态。
        let st = self.ui.theme.button.clone();
        let bg = if is_open || ev.pressed {
            st.bg_pressed
        } else if hit {
            st.bg_hover
        } else {
            st.bg
        };
        self.ui
            .push_panel_like(rect, bg, st.border, st.border_w, st.radius, 1);
        self.ui.push_text_rect(
            rect,
            label,
            font_size,
            st.fg,
            family,
            TextAlign::Center,
            TextVAlign::Center,
            None,
            None,
        );
        // 推进横向光标 + 记栏尺寸。
        self.size.x = (rect.x + rect.w) - self.cursor.x;
        self.size.y = h.max(self.size.y);
        self.cursor.x = rect.x + rect.w + gap;

        if is_open {
            self.popup(label, content, rect, font_size);
        }
    }

    /// 录下拉面板（`Level::Normal` 浮层窗口）：位置 = 触发器左下（屏幕物理坐标）。
    fn popup(
        &mut self,
        label: &str,
        content: impl FnOnce(&mut MenuCtx<'_, '_, '_>),
        trigger: Rect,
        font_size: f32,
    ) {
        // 浮层面板样式：`ComboStyle` 的菜单外观（现代扁平菜单）+ 面板 bg / radius；
        // **内边距 0**（菜单项自己带 padding），边框取 `menu_border`。
        let cs = self.ui.theme.combo.clone();
        let style = PanelStyle {
            bg: cs.menu_bg.into(),
            border: cs.menu_border,
            padding: 0.0,
            radius: cs.menu_radius,
            bg_image: None,
            ..self.ui.theme.panel.clone()
        };
        let pos = Vec2::new(trigger.x, trigger.y + trigger.h + 2.0);
        let popup_id = format!("{}::{label}", self.id);
        // **强制哨兵 z**（与 combo 浮层同一招）：菜单下拉恒在一切窗口之上，
        // 这样"菜单栏录在哪里"就不再影响遮挡（不必强求录在各窗口之后）。
        self.ui
            .state_mut()
            .window_z
            .insert(crate::id::IdAbsolute::owned(popup_id.clone()), WIN_TOPMOST);
        let mut close = false;
        let size = self
            .ui
            .window(&popup_id)
            .pos(Position::Physical(pos))
            // **锁定位置**：菜单面板不该能被拖动——拖走了就与触发器脱节，
            // 点菜单项的命中按窗口走、视觉却跑别处（"控件严重错位"）。
            .clamp(WindowClamp::Locked)
            .level(Level::Normal)
            .style(style)
            .show(|w| {
                let mut m = MenuCtx { w, font_size, close: &mut close };
                content(&mut m);
            });
        // 面板里点了菜单项 ⇒ 收起（`finish` 统一写 `UiState::menu_open`）。
        if close {
            self.item_clicked = true;
        }
        self.popup = Some(Rect::new(pos.x, pos.y, size.x, size.y));
    }

    /// 收尾（由 [`Ui::menu_bar`](crate::Ui::menu_bar) 调用）：定展开状态 + 返回栏尺寸。
    pub(crate) fn finish(mut self) -> (Vec2, Option<Option<String>>) {
        // 鼠标 / 触发器 / 面板都在**物理像素**空间（`hit_test` 与 `mouse_screen` 同空间）。
        let mouse = self.ui.mouse_screen();
        let btn = self.ui.mouse_left();
        let esc = self.ui.key_down_edge(winit::keyboard::KeyCode::Escape);
        // 收起条件（任一命中即收起）：点了菜单项 / 点栏外 / Esc。
        // ⚠ "点栏外"必须排除**下拉面板**上的按下——面板是独立窗口，`on_trigger` 不含它。
        let on_popup = self.popup.is_some_and(|r| hit_test(&r, mouse));
        let outside = btn.down_edge() && !self.on_trigger && !on_popup;
        if self.item_clicked || outside || esc {
            self.action = Some(None);
        }
        (self.size, self.action)
    }
}

/// **下拉面板的内容上下文**（`bar.menu(label, |m| ..)` 的 `m`）。
///
/// [`Deref`](std::ops::Deref) 到内层 [`Window`]：`label` / `button` / `divider` / `row` /
/// `text_input` / `add(..)` 等**全部可用**（"菜单里也能放文本输入、分割线、按钮、横向排版"）；
/// 另外提供菜单语义的 [`Self::item`] / [`Self::item_checked`] / [`Self::caption`] /
/// [`Self::separator`]。
pub struct MenuCtx<'w, 'a, 'b> {
    w: &'w mut Window<'a, 'b>,
    font_size: f32,
    /// 点了菜单项 ⇒ 收起（由 [`MenuBar::finish`] 统一判定）。
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
    /// 整行可点（不是只有文字），hover / 按下整行高亮（与 `combo` 下拉同一观感）。
    pub fn item(&mut self, label: &str) -> bool {
        self.item_row(label, None)
    }

    /// **带勾选的菜单项**（视图显隐这类）：点击直接翻转 `&mut bool` 并收起。
    ///
    /// 勾选标记用矢量 `Icon::Check`（不是 "✓" 字形——缺字形时菜单会出现豆腐块）。
    pub fn item_checked(&mut self, label: &str, checked: &mut bool) -> bool {
        let clicked = self.item_row(label, Some(*checked));
        if clicked {
            *checked = !*checked;
        }
        clicked
    }

    /// 菜单里的一条**分割线**。
    pub fn separator(&mut self) {
        self.indent();
        self.w.divider();
    }

    /// 菜单里的**纯文本行**（不可点：分组标题 / 说明）。
    pub fn caption(&mut self, text: &str) {
        self.indent();
        self.w.label(text);
    }

    /// **横向排版**（= `Window::row`，但先按"勾选列"缩进）。
    ///
    /// 刻意遮蔽 `Deref` 出来的 [`Window::row`]：菜单里所有内容都该与**菜单项文字**
    /// 同一列起排，否则标题 / 按钮行会贴到面板左缘、与菜单项错开一格图标位
    /// （"控件严重错位"的观感就是这么来的）。
    ///
    /// ⚠ 内层 `Window::row` 用 `UiAdd::row`（trait 方法）显式调用，避免递归。
    pub fn row(&mut self, f: impl FnOnce(&mut crate::ui::Pack<'_, '_>)) -> Vec2 {
        self.indent();
        UiAdd::row(self.w, f)
    }

    /// 把光标推到"菜单项文字列"（`item_pad_x + 勾选列宽`）。
    ///
    /// 用 `child_rect(w, 0)` 占位：`Window` 的内容栈会把它当成一个零高子项
    /// （垂直只前进一个 `gap`，正好当作"缩进 + 行距"；宽度小于菜单项，不会撑宽面板）。
    fn indent(&mut self) {
        let (pad_x, check_w) = {
            let t = self.w.ui_mut().theme.clone();
            let fs = self.font_size.max(1.0);
            (t.combo.item_pad_x, fs + 6.0)
        };
        self.w
            .ui_mut()
            .child_rect(pad_x + check_w, 0.0, Child::Expand);
    }

    /// 菜单项公共实现：`check = Some(是否勾选)` 时左侧画勾选标记。
    ///
    /// ⚠ 名字刻意不叫 `row`：`MenuCtx` 经 `Deref` 到 [`Window`]，而 `Window::row`（横向排版）
    /// 是给应用用的——同名私有方法会**遮蔽**它（trait / deref 方法优先级更低），
    /// 于是 `m.row(|r| ..)` 会去调这个私有 helper（编译报错 "method `row` is private"）。
    fn item_row(&mut self, label: &str, check: Option<bool>) -> bool {
        let (cs, font_size, pad_x, item_h, check_w) = {
            let t = self.w.ui_mut().theme.clone();
            let cs = t.combo.clone();
            let fs = self.font_size.max(1.0);
            (cs, fs, t.combo.item_pad_x, (fs * 1.3).round() + 6.0, fs + 6.0)
        };
        let tw = self.w.ui_mut().text_size(label, font_size, cs.font_family.as_deref()).x;
        let w = (check_w + tw + pad_x * 2.0).max(cs.item_min_w);
        let rect = self.w.ui_mut().child_rect(w, item_h, Child::Expand);
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
        let bg = if ev.pressed || hit { cs.item_hover } else { cs.menu_bg };
        // `elem_hint()`：装饰画在本元素背景之上（写死小 elem 会被自己的背景盖住）。
        ui.push_panel_like(rect, bg, cs.menu_bg, 0.0, 0.0, ui.elem_hint());
        if let Some(on) = check {
            let d = font_size;
            ui.icon_at(
                Position::Physical(Vec2::new(rect.x + pad_x, rect.y + (rect.h - d) * 0.5)),
                crate::draw::Size::Physical(Vec2::splat(d)),
                Icon::Check,
                if on { cs.fg } else { cs.menu_bg },
            );
        }
        ui.push_text_rect(
            Rect::new(rect.x + pad_x + check_w, rect.y, (rect.w - pad_x * 2.0 - check_w).max(0.0), rect.h),
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
        ev.clicked
    }

    /// 面板内容的字号（应用自绘时对齐用）。
    #[inline]
    pub fn font_size(&self) -> f32 {
        self.font_size
    }
}
