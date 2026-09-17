//! **菜单栏**（横向）：一排触发器 + 点开的**下拉面板**；面板内容是一个闭包上下文
//! （[`MenuCtx`]：`item` / `item_checked` / `caption` / `separator`，并 `Deref` 到
//! [`Window`](crate::Window)，所以 `add` / `label` / `button` / `divider` / `row` /
//! `text_input` 都能用）。
//!
//! ⚠ 下拉面板的**录制、样式、宽度、关闭规则全部来自 [`crate::widgets::menu`]**
//! （与 [`Dropdown`](crate::Dropdown) **同一套实现**）——本模块只负责"横向一排触发器"：
//! 排布、触发器常亮、以及"点栏外收起"里"栏"的判定（点另一个触发器 = 切换，不是收起）。
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
//! - **点栏外**（既不在任何触发器上、也不在下拉面板 / 任何浮层里）→ 收起；
//! - **Esc** → 收起。应用自己的 Esc 语义（如退出）请先看
//!   [`UiState::menu_open`](crate::UiState::menu_open)：菜单开着的那一帧不要抢；
//! - 下拉面板是 [`Level::Normal`](crate::Level) 的**浮层窗口**（点它不置顶、也不与
//!   "点击置顶"抢 z 序），z 被强制成 [`WIN_TOPMOST`](crate::ui::WIN_TOPMOST) 哨兵 ——
//!   **菜单栏录在哪里都盖得住别的窗口**（不必强求录在各窗口之后）。
//!
//! # 为什么触发器不用 `Button` 控件
//!
//! 菜单栏要按"当前展开项"把那个触发器**一直画成按下态**（鼠标移开也能看出哪个菜单开着），
//! 还要把触发器矩形收集起来做"点栏外收起"判定。直接用绘制原语 + `update_interact`
//! （与 `Dropdown` 触发器同一套写法）比"先量尺寸再 `add_at`"更直接。
//!
//! # 为什么菜单栏**手排**而不是用 `pack` / 一排 `Dropdown`
//!
//! 每个 `menu()` 可能在**同一帧内**立刻录下拉窗口（展开时）；栏还要按标签收集"哪些矩形
//! 属于栏"做点外判定，并且**同一时刻只有一个菜单开着**（`UiState::menu_open`）。手排一个
//! `x` 光标 + 自己的展开槽是最直接的写法，且位置语义与顶层 `*_at` 一致（顶层
//! `abs_base = 0` ⇒ 局部坐标即屏幕坐标）。
//!
//! 想要"普通的按钮下拉菜单"请用 [`Dropdown`](crate::Dropdown)（`p.add(..)` 即可，
//! 菜单内容同样能用 [`MenuCtx`] 的那套 `UiAdd` 方法）。

use glam::Vec2;
use rjw_transform::Rect;

use crate::draw::{TextAlign, TextVAlign};
use crate::hit::update_interact;
use crate::ui::Ui;
use crate::widgets::menu::{self, MenuFn, PopupSide, PopupSpec};

/// 菜单内容上下文（定义在 [`crate::widgets::menu`]，此处重导出以保持
/// `crate::widgets::menubar::MenuCtx` 这一旧路径可用）。
pub use crate::widgets::menu::MenuCtx;

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
    /// 本帧鼠标是否落在**某个触发器**上（点栏外判定的一部分）。
    on_trigger: bool,
    /// 本帧面板里点了菜单项（⇒ 收起）。
    item_clicked: bool,
    /// 本帧左键按下落在**面板之外**（且不在任何 `WIN_TOPMOST` 浮层上）——共享浮层上报。
    down_outside: bool,
    /// 本帧下拉面板矩形（**屏幕物理坐标**；`RJ_MENU_TRACE` 诊断用）。
    popup: Option<Rect>,
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
            item_clicked: false,
            down_outside: false,
            popup: None,
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

        // 展开时录下拉面板（**共享实现**：见 `crate::widgets::menu`）。
        if is_open {
            let popup_id = format!("{}::{label}", self.id);
            let spec = PopupSpec {
                id: &popup_id,
                trigger: rect,
                side: PopupSide::Below,
                font_size,
                // 面板至少与触发器同宽（`item_min_w` 由控件侧保证，这里补触发器宽）。
                min_w: rect.w,
            };
            let res = menu::popup_show(self.ui, &spec, MenuFn::new(content));
            self.item_clicked |= res.item_clicked;
            self.down_outside |= res.down_outside;
            self.popup = Some(res.rect);
        }
    }

    /// 收尾（由 [`Ui::menu_bar`](crate::Ui::menu_bar) 调用）：定展开状态 + 返回栏尺寸。
    ///
    /// 收起条件（任一命中即收起）：点了菜单项 / 点栏外 / Esc。
    /// ⚠ "点栏外"必须用**本栏的**判定：按下虽然在面板外（共享浮层已排除"面板内 / 任意
    /// `WIN_TOPMOST` 浮层上"），但若落在**另一个触发器**上，那是"切换菜单"而不是收起
    /// ——`action` 已在 `menu()` 里定好，这里不能再把它清零（否则"点另一个菜单"会变成
    /// "全部收起"）。
    pub(crate) fn finish(mut self) -> (Vec2, Option<Option<String>>) {
        let esc = self.ui.key_down_edge(winit::keyboard::KeyCode::Escape);
        let outside = self.down_outside && !self.on_trigger;
        // 只在"本帧真有面板 / 真有点击"时打（否则每帧一行会淹没菜单项的**行矩形**打印，
        // 而那才是对齐问题的诊断通道）。
        if (self.popup.is_some() || self.item_clicked || outside)
            && std::env::var_os("RJ_MENU_TRACE").is_some()
        {
            eprintln!(
                "menu[bar {}] on_trigger={} down_outside={} popup={:?} esc={esc} item_clicked={}",
                self.id, self.on_trigger, self.down_outside, self.popup, self.item_clicked
            );
        }
        if self.item_clicked || outside || esc {
            self.action = Some(None);
        }
        (self.size, self.action)
    }
}
