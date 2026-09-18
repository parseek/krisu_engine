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

use crate::draw::{Icon, Position, Size};
use crate::ui::Ui;
use crate::widgets::{Response, Sense, Widget};
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
    fn ui(self, ui: &mut Ui) -> Response {
        // 方形：边长 = 行高扣掉上下各 2px（`row` 会把行内子项强制成 `row_h`，
        // 这里给一个略小的宽度，视觉上不与标题文字贴边）。
        let h = ui.theme().row_h;
        // 申请 + 收交互：`Sense::DRAG` 让按下即 `claim_press()`——标题栏按钮上的按下
        // **不会**被当成"拖窗口"的基准（与滚动条同一机制）。
        let (rect, resp) = ui.allocate_sense(self.id, Vec2::new(h - 2.0, h), Sense::DRAG);
        // 被裁剪层完全剔除 ⇒ 直接 return（不镶嵌、不入段）。
        if resp.culled {
            return resp;
        }
        let st = ui.theme().button.clone();
        let bg = st.pick_bg(resp.pressed, resp.hovered);
        let fg = if resp.hovered { st.fg } else { ui.theme().palette.text_muted };
        // 图标居中、留 4px 边距，等比（`icon_at` 内部取居中方块 ⇒ 不会拉扁）。
        // 一个绘制块一个 painter：面板背景与图标各自取当时的 `elem_hint`（与旧写法等价）。
        let d = (rect.w.min(rect.h) - 8.0).max(6.0);
        let p = ui.painter();
        p.panel(rect, bg, st.border, st.border_w, st.radius);
        p.icon_at(
            Position::Physical(Vec2::new(
                rect.x + (rect.w - d) * 0.5,
                rect.y + (rect.h - d) * 0.5,
            )),
            Size::Physical(Vec2::splat(d)),
            self.icon,
            fg,
        );
        resp
    }
}
