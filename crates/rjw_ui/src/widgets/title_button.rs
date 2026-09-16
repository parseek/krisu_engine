//! **标题栏图标按钮**（窗口关闭 / 收缩用；只依赖公开 API）。
//!
//! 为什么是一个 `Widget` 而不是 `r.button("×")`：
//! - 按钮文字是**字形**，缺字形 / 换字体就会变形甚至变豆腐块；标题栏图标是**几何**
//!   （[`Icon::Close`] / [`Icon::ChevronUp`] / [`Icon::ChevronDown`]），与字体无关；
//! - `Widget` 天然占一行内的位置（`row.add(..)`）、自带 `Response`（`clicked()` 等），
//!   不必手写"占位 + 命中 + 绘制"三件套。
//!
//! 按下时 `claim_press()`：标题栏按钮上的按下**不会**被当成"拖窗口"的基准
//! （与滚动条同一机制，见 `Ui::claim_press`）。

use rjw_transform::Rect;

use crate::draw::{Icon, Position, Size};
use crate::hit::update_interact;
use crate::ui::Ui;
use crate::widgets::{Response, Widget};
use glam::Vec2;

/// 标题栏上的方形图标按钮。
pub(crate) struct TitleIconButton<'a> {
    id: &'a str,
    icon: Icon,
}

impl<'a> TitleIconButton<'a> {
    /// 构造（`id` 用窗口内的相对 id 即可，容器会加前缀）。
    pub(crate) fn new(id: &'a str, icon: Icon) -> Self {
        Self { id, icon }
    }
}

impl Widget for TitleIconButton<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        // 方形：边长 = 行高扣掉上下各 2px（`row` 会把行内子项强制成 `row_h`，
        // 这里给一个略小的宽度，视觉上不与标题文字贴边）。
        let h = ui.theme().row_h;
        Vec2::new(h - 2.0, h)
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        let abs = ui.id_for(self.id);
        let hit = ui.hit_abs(&abs, &rect);
        let btn = ui.mouse_left();
        let (ev, pressed) = {
            let ws = ui.state_mut().widget(&abs);
            let ev = update_interact(ws, hit, btn);
            (ev, ws.pressed)
        };
        if ev.pressed {
            // 按下即声明"本次按下由我消费"：窗口不再建立拖拽基准（标题栏按钮拖动 = 点按钮）。
            ui.claim_press();
        }
        let st = ui.theme().button.clone();
        let bg = st.pick_bg(pressed, hit);
        // ⚠ `elem_hint()`：按钮画在本元素内其它命令（标题文字）之上/之下按录制序，
        // 写死 `1` 会被同元素更大的 elem 盖住（`NumberInput` 手柄那次就是这么没的）。
        ui.push_panel_like(rect, bg, st.border, st.border_w, st.radius, ui.elem_hint());
        // 图标居中、留 4px 边距，等比（`icon_at` 内部取居中方块 ⇒ 不会拉扁）。
        let d = (rect.w.min(rect.h) - 8.0).max(6.0);
        ui.icon_at(
            Position::Physical(Vec2::new(
                rect.x + (rect.w - d) * 0.5,
                rect.y + (rect.h - d) * 0.5,
            )),
            Size::Physical(Vec2::splat(d)),
            self.icon,
            if hit { st.fg } else { ui.theme().palette.text_muted },
        );
        Response {
            hovered: hit,
            pressed,
            clicked: ev.clicked,
            ..Default::default()
        }
    }
}
