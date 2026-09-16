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

use crate::draw::{Icon, Position, Size, TextAlign, TextVAlign};
use crate::hit::{hit_test, update_interact};
use crate::layout::Child;
use crate::style::PanelStyle;
use crate::ui::{Level, Ui, UiAdd as _, WindowClamp, WIN_TOPMOST};
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
        // **左内边距 = 勾选列 + 文字左留白**：这样"菜单项文字"与 `caption` / `row` /
        // 自己画的分割线**天然同列**（都从内容原点起排），不需要任何"给下一子项缩进"
        // 的花招（垂直栈里子项 x 恒等于内容原点，缩进宽度是无效的）。
        let pad = cs.item_pad_x + Self::check_w(font_size);
        let border_w = self.ui.theme.panel.border_w;
        let pad_total = pad + border_w;
        let style = PanelStyle {
            bg: cs.menu_bg.into(),
            border: cs.menu_border,
            padding: pad,
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
        // **固定宽 = 上一帧的结算宽**（首帧自然宽，次帧起精确——菜单内容通常跨帧不变）：
        // 固定宽让 `avail_w()` 有值（`LimitedInParent` 控件与 `Divider` 才有正确可用宽），
        // 且子项请求"极宽"时会被 clamp 到内容宽 ⇒ **菜单项高亮满宽**（不再取决于
        // "谁是当前最宽子项"——长标题会把面板撑宽，高亮却只有标题宽）。
        // ⚠ `WindowBuilder::width` 收的是**内容宽**，而 `settle = fixed_w + 2*pad_total`
        // ⇒ 传 `prev - 2*pad_total` 才能保持上一帧的宽度（次帧起收敛）。
        let prev = self.ui.state().window_sizes.get(popup_id.as_str()).map(|s| s.x);
        // `fill = 固定宽已知`（第 2 帧起）：此时子项请求"极宽"会被 clamp 到内容宽 ⇒
        // 高亮 / 分割线**铺满面板**；第 1 帧（自动宽）必须请求**自然宽**——否则 1e6 的
        // 请求会把"自然尺寸"撑成一百万，面板宽度就再也收敛不回来了。
        let fill = prev.is_some();
        if std::env::var_os("RJ_MENU_TRACE").is_some() {
            eprintln!("menu[popup {popup_id}] prev={prev:?} pad_total={pad_total} fill={fill}");
        }
        let mut close = false;
        let mut b = self
            .ui
            .window(&popup_id)
            .pos(Position::Physical(pos))
            // **锁定位置**：菜单面板不该能被拖动——拖走了就与触发器脱节，
            // 点菜单项的命中按窗口走、视觉却跑别处（"控件严重错位"）。
            .clamp(WindowClamp::Locked)
            .level(Level::Normal)
            .style(style);
        if let Some(w) = prev {
            b = b.width(Size::Physical((w - pad_total * 2.0).max(40.0)));
        }
        let size = b.show(|w| {
            let mut m = MenuCtx { w, font_size, pad, fill, close: &mut close };
            content(&mut m);
        });
        // 面板里点了菜单项 ⇒ 收起（`finish` 统一写 `UiState::menu_open`）。
        if close {
            self.item_clicked = true;
        }
        self.popup = Some(Rect::new(pos.x, pos.y, size.x, size.y));
    }

    /// **勾选列宽度**（图标 + 与文字的间隙；菜单项文字相对面板内缘右移的量）。
    ///
    /// 面板的左内边距取 `item_pad_x + 勾选列宽`，于是"勾选列"落在内边距里，
    /// 菜单项 / 标题 / 分割线全部从**同一个内容原点**起排 ⇒ 天然对齐。
    #[inline]
    fn check_w(font_size: f32) -> f32 {
        font_size + 6.0
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
    /// 面板的左内边距（= `item_pad_x + 勾选列宽`）——菜单项高亮要往左扩这么多才铺满内缘。
    pad: f32,
    /// **面板宽度已固定**（第 2 帧起）：子项请求"极宽"由窗口 clamp 到内容宽 ⇒ 铺满。
    /// 第 1 帧（自动宽）为 `false`：必须请求自然宽，否则会把自然尺寸撑爆。
    fill: bool,
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

    /// 菜单里的一条**分割线**（满内容宽、与菜单项文字同列）。
    ///
    /// 自己画而不是用 [`Divider`](crate::Divider)：`Divider` 的宽 = `avail_w()`，
    /// 而**自动宽**窗口里那是 `None`（退回固定 120）⇒ 线又短又不在该在的位置
    /// （用户实测："Menu 分割线错位"）。这里请求"极宽"，由窗口把子项 clamp 到
    /// 内容宽 ⇒ 线恒等于面板内容宽、起点也在内容原点。
    pub fn separator(&mut self) {
        let (t, m) = {
            let d = self.w.ui_mut().theme.divider.clone();
            (d.thickness, d.margin)
        };
        let h = t + m * 2.0;
        let r = self.row_rect(120.0, h);
        let y = r.y + (r.h - t) * 0.5;
        let color = self.w.ui_mut().theme.divider.color;
        self.w.ui_mut().push_solid_rect(Rect::new(r.x, y, r.w, t), color);
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
        let w = if self.fill { 1.0e6 } else { natural_w };
        self.w.ui_mut().child_rect(w, h, Child::Expand)
    }

    /// `RJ_MENU_TRACE=1`：打印每个下拉内容的**行矩形**（`x/y/w/h`）。
    ///
    /// 为什么留这条通道：菜单的"对齐"是纯几何约定（标题 / 菜单项 / 分割线必须同 x、
    /// 同宽），而它**看不出来**——肉眼看"差不多"，出问题时只是"有点怪"。有了这三个数，
    /// "分割线错位"这类问题一眼可判（`w` 应等于菜单项的 `w`，`x` 也应相同）。
    fn trace(&self, kind: &str, r: Rect) {
        if std::env::var_os("RJ_MENU_TRACE").is_some() {
            eprintln!(
                "menu[{kind}] x={:.0} y={:.0} w={:.0} h={:.0}",
                r.x, r.y, r.w, r.h
            );
        }
    }

    /// 菜单项公共实现：`check = Some(是否勾选)` 时左侧画勾选标记。
    ///
    /// ⚠ 名字刻意不叫 `row`：`MenuCtx` 经 `Deref` 到 [`Window`]，同名私有方法会**遮蔽**
    /// `Window::row`（deref 方法优先级更低）⇒ 应用的 `m.row(..)` 会去调私有那个。
    fn item_row(&mut self, label: &str, check: Option<bool>) -> bool {
        let (cs, font_size, item_h, natural_w) = {
            let t = self.w.ui_mut().theme.clone();
            let fs = self.font_size.max(1.0);
            let check_w = MenuBar::check_w(fs);
            let tw = self.w.ui_mut().text_size(label, fs, t.combo.font_family.as_deref()).x;
            let item_h = (fs * 1.3).round() + 6.0;
            let natural = (check_w + tw + t.combo.item_pad_x * 3.0).max(t.combo.item_min_w);
            (t.combo.clone(), fs, item_h, natural)
        };
        // **宽度已固定 ⇒ 请求极宽**：窗口把子项 clamp 到内容宽 ⇒ 高亮满宽（与面板等宽），
        // 不再受"当前最宽子项"影响（长标题撑宽面板时高亮仍满宽）。
        let rect = self.row_rect(natural_w, item_h);
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
        // 高亮**满内宽**：`rect` 是内容区，把它的左右两侧各扩一个 `pad` 就铺满面板内缘
        // （`pad` = 面板左内边距，右内边距同值）。
        // `elem_hint()`：装饰画在本元素背景之上（写死小 elem 会被自己的背景盖住）。
        let hl = Rect::new(rect.x - self.pad, rect.y, rect.w + self.pad * 2.0, rect.h);
        ui.push_panel_like(hl, bg, cs.menu_bg, 0.0, 0.0, ui.elem_hint());
        if let Some(on) = check
            && on
        {
            // 勾选标记**只在勾选时画**：勾选列由面板左内边距**恒留位**，未勾选时什么都不画
            // （曾经给它画一个"背景色图标"占位——悬停高亮时那块背景色会在高亮上显形）。
            let d = font_size;
            ui.icon_at(
                // 勾选列在**左内边距里**（内容原点左侧一个勾选列宽）：
                // 于是所有文字都在同一条竖线上，勾选图标不占文字位置。
                Position::Physical(Vec2::new(
                    rect.x - MenuBar::check_w(font_size),
                    rect.y + (rect.h - d) * 0.5,
                )),
                crate::draw::Size::Physical(Vec2::splat(d)),
                Icon::Check,
                cs.fg_mark,
            );
        }
        ui.push_text_rect(
            Rect::new(rect.x, rect.y, rect.w, rect.h),
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
