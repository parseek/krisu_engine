//! 绘制原语：实心矩形 / 边框 / 面板 / 投影 / 圆角 / 渐变 / 图标 / 背景图。
//!
//! 全部是**记录式**（写进 [`DrawQueue`](super::DrawQueue)，实际镶嵌 / 提交在
//! `Ui::finish`）——因此本文件可以在无 GPU 的单测里直接断言产出。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use super::Painter;
use crate::draw::{
    CornerRadius, DrawKind, Gradient, Icon, ImageBg, PanelCmdCtx, Position, Size, push_panel_img_cmds,
};
use crate::style::{Brush, ShadowStyle};

impl Painter {
    /// **实心矩形**（逻辑坐标；`w/h <= 0` 跳过）。
    pub fn solid(&mut self, rect: Rect, color: Color) {
        if rect.w > 0.0 && rect.h > 0.0 {
            self.draw_hint(DrawKind::Solid(color), rect);
        }
    }

    /// **矩形边框**（画在矩形内边缘，宽度在镶嵌期取整到物理像素）。
    pub fn border(&mut self, rect: Rect, color: Color, width: f32) {
        if rect.w > 0.0 && rect.h > 0.0 {
            self.draw_hint(
                DrawKind::Border { color, width, radius: CornerRadius::default() },
                rect,
            );
        }
    }

    /// **面板**（背景刷 + 边框；圆角 > 0 时走"外圈圆角边框 + 内圈背景刷"）。
    ///
    /// `elem` 走 [`Painter::elem_hint`]（控件背景）；容器装饰请用
    /// [`Painter::panel_elem`] 传 `0`。
    pub fn panel(
        &mut self,
        rect: Rect,
        bg: impl Into<Brush>,
        border: Color,
        border_w: f32,
        radius: impl Into<CornerRadius>,
    ) {
        let elem = self.elem_hint();
        self.panel_img_elem(rect, bg, None, border, border_w, radius, elem);
    }

    /// 同 [`Painter::panel`]，但**显式给 `elem`**（`0` = 容器装饰层）。
    pub fn panel_elem(
        &mut self,
        rect: Rect,
        bg: impl Into<Brush>,
        border: Color,
        border_w: f32,
        radius: impl Into<CornerRadius>,
        elem: u32,
    ) {
        self.panel_img_elem(rect, bg, None, border, border_w, radius, elem);
    }

    /// **面板 + 背景图**（层次：背景刷 → 背景图 → 边框；圆角遮罩恒用面板 `radius`）。
    pub fn panel_img(
        &mut self,
        rect: Rect,
        bg: impl Into<Brush>,
        img: Option<ImageBg>,
        border: Color,
        border_w: f32,
        radius: impl Into<CornerRadius>,
    ) {
        let elem = self.elem_hint();
        self.panel_img_elem(rect, bg, img, border, border_w, radius, elem);
    }

    /// 同 [`Painter::panel_img`]，但**显式给 `elem`**。
    #[allow(clippy::too_many_arguments)]
    pub fn panel_img_elem(
        &mut self,
        rect: Rect,
        bg: impl Into<Brush>,
        img: Option<ImageBg>,
        border: Color,
        border_w: f32,
        radius: impl Into<CornerRadius>,
        elem: u32,
    ) {
        let seq = self.next_seq();
        // **播放头必须追上**：`push_panel_img_cmds` 用 `seq` / `seq + 1` / `seq + 2`
        // （背景刷 / 背景图 / 边框），而 `next_seq()` 只推进了 1 ⇒ 不补的话下一条命令会
        // 拿到**重复序号**（`place` 归一化按 `seq` 切分 ⇒ 同一容器被拆进两个排序空间），
        // 并且 `Ui::begin_top_placement` 的"播放头 = 已分配最大序号"不变量会被打破
        // （显式 `debug_assert`；实测由 `menu_bar` 的容器入口先踩到）。
        let last = seq + if img.is_some() { 2 } else { 1 };
        let (depth, win, clip) = (self.q.depth, self.q.cur_win, self.q.clip);
        push_panel_img_cmds(
            &mut self.q.queue,
            PanelCmdCtx { depth, win, elem, rect, clip, seq },
            &bg.into(),
            img,
            border,
            border_w,
            radius.into(),
        );
        self.q.advance_seq_to(last);
    }

    /// **顶点色软阴影**（`elem = 0`：画在本体之下、内容之下）。
    ///
    /// `shadow.blur <= 0` 或全透明时不产生任何命令。
    pub fn shadow(&mut self, rect: Rect, shadow: &ShadowStyle, radius: impl Into<CornerRadius>) {
        if !shadow.is_visible() {
            return;
        }
        self.draw(
            DrawKind::Shadow {
                color: shadow.color,
                blur: shadow.blur,
                offset: shadow.offset,
                radius: radius.into(),
            },
            rect,
            0,
        );
    }

    /// **圆角矩形**（绝对定位；`radius` 带单位，四角可各异）。
    pub fn rounded_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        radius: impl Into<Size<CornerRadius>>,
        color: Color,
    ) {
        let pos = pos.into().to_physical(self.scale());
        let size = size.into().to_physical(self.scale());
        let radius = radius.into().to_physical(self.scale());
        self.draw_hint(
            DrawKind::RoundedRect { corners: [color; 4], radius },
            Rect::new(pos.x, pos.y, size.x, size.y),
        );
    }

    /// **矢量图标**（绝对定位；框非方形时按 `min(w, h)` 居中等比）。
    pub fn icon_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        icon: Icon,
        color: Color,
    ) {
        let pos = pos.into().to_physical(self.scale());
        let size = size.into().to_physical(self.scale());
        self.draw_hint(DrawKind::Icon { icon, color }, Rect::new(pos.x, pos.y, size.x, size.y));
    }

    /// **背景图**（绝对定位；`ImageBg` 决定铺排 / 染色 / 圆角遮罩）。
    pub fn image_at(&mut self, pos: impl Into<Position>, size: impl Into<Size<Vec2>>, bg: ImageBg) {
        let pos = pos.into().to_physical(self.scale());
        let size = size.into().to_physical(self.scale());
        self.draw_hint(DrawKind::Image(bg), Rect::new(pos.x, pos.y, size.x, size.y));
    }

    /// **矩形渐变**（绝对定位；`gradient` 接受 [`Gradient`] 或 [`Color`]）。
    pub fn gradient_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        gradient: impl Into<Gradient>,
    ) {
        let pos = pos.into().to_physical(self.scale());
        let size = size.into().to_physical(self.scale());
        self.draw_hint(
            DrawKind::Rect(gradient.into()),
            Rect::new(pos.x, pos.y, size.x, size.y),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::new(1.0, 2.0, 30.0, 40.0)
    }

    /// **默认 `elem` 逐条取 `elem_hint()`**（与旧 `Ui::push_*` 逐位一致）：
    /// 装饰要压在自家内容之上时，靠的是"重新取一个 painter"（`elem_hint` 已增大），
    /// 而不是把一个 painter 的 elem 冻结——冻结会让同 elem 内"图形先于文字"的
    /// 组序把后画的图形（拖拽柄）排到文字之前（历史 bug：手柄看不见）。
    #[test]
    fn primitives_take_a_fresh_elem_per_command() {
        let mut p = Painter::new(1.0);
        p.solid(rect(), Color::WHITE);
        p.border(rect(), Color::BLACK, 1.0);
        let elems: Vec<u32> = p.commands().iter().map(|d| d.elem).collect();
        assert_eq!(elems, vec![1, 2], "每条命令各自取当前 elem_hint");
        assert_eq!(p.elem_hint(), 3);
        // 重新取一个 painter 只是拿到同一个队列的下一个 elem_hint
        assert_eq!(p.commands().len(), 2);
    }

    /// 容器装饰层用**显式 elem = 0**（阴影 / 面板背景 / 边框）。
    #[test]
    fn decor_uses_elem_zero() {
        use crate::style::ShadowStyle;
        let mut p = Painter::new(1.0);
        p.shadow(rect(), &ShadowStyle::default(), 4.0);
        p.panel_elem(rect(), Color::BLACK, Color::WHITE, 1.0, 4.0, 0);
        assert!(p.commands().iter().all(|d| d.elem == 0), "装饰层恒 elem = 0");
    }

    /// `w/h <= 0` 的实心矩形 / 边框不产生命令（旧 `push_solid_rect` 的守卫）。
    #[test]
    fn degenerate_rects_produce_nothing() {
        let mut p = Painter::new(1.0);
        p.solid(Rect::new(0.0, 0.0, 0.0, 10.0), Color::WHITE);
        p.solid(Rect::new(0.0, 0.0, 10.0, -1.0), Color::WHITE);
        p.border(Rect::new(0.0, 0.0, 0.0, 0.0), Color::WHITE, 1.0);
        assert!(p.commands().is_empty(), "退化矩形不应入队");
    }

    /// 面板 = 背景刷 + （可选图） + 边框，且**直角时也不丢图**（历史 bug 回归）。
    #[test]
    fn panel_keeps_image_and_border_at_zero_radius() {
        let mut p = Painter::new(1.0);
        p.panel_img_elem(
            rect(),
            Color::BLACK,
            Some(ImageBg::new(7, Vec2::new(32.0, 32.0))),
            Color::WHITE,
            1.0,
            0.0,
            0,
        );
        let kinds: Vec<&str> = p
            .commands()
            .iter()
            .map(|d| match d.kind {
                DrawKind::Solid(_) => "solid",
                DrawKind::Rect(_) => "grad",
                DrawKind::RoundedRect { .. } => "rounded",
                DrawKind::Image(_) => "image",
                DrawKind::Border { .. } => "border",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["solid", "image", "border"], "直角面板：刷 + 图 + 边框");
        // seq 递增保证同 elem 内层次 = 刷 → 图 → 边框
        let seqs: Vec<u32> = p.commands().iter().map(|d| d.seq).collect();
        assert!(seqs.windows(2).all(|w| w[1] > w[0]));
    }

    /// **面板推了 3 条命令（`seq` / `+1` / `+2`）后播放头必须追上**：否则下一条命令拿到
    /// **重复序号**（`place` 归一化按 `seq` 切分 ⇒ 同一容器被拆进两个排序空间），并且
    /// `Ui::begin_top_placement` 的"播放头 = 已分配最大序号"不变量直接失败。
    /// 实测现场：带背景图的窗口录完之后，`menu_bar` 的容器入口 `debug_assert` 炸了。
    #[test]
    fn panel_with_image_advances_the_seq_playhead() {
        let mut p = Painter::new(1.0);
        p.panel_img_elem(
            rect(),
            Color::BLACK,
            Some(ImageBg::new(7, Vec2::new(32.0, 32.0))),
            Color::WHITE,
            1.0,
            0.0,
            0,
        );
        let last = p.commands().iter().map(|d| d.seq).max().unwrap_or(0);
        assert_eq!(p.q.seq, last, "播放头 = 已分配的最大序号（{last}）");
        // 再取号：不许与任何已入队命令重复。
        let next = p.next_seq();
        assert!(
            p.commands().iter().all(|d| d.seq != next),
            "序号不能重复（next={next}）"
        );
        // 没有背景图时只推 2 条，同样要追上。
        let mut p = Painter::new(1.0);
        p.panel_img_elem(rect(), Color::BLACK, None, Color::WHITE, 1.0, 0.0, 0);
        let last = p.commands().iter().map(|d| d.seq).max().unwrap_or(0);
        assert_eq!(p.q.seq, last);
    }

    /// `clipped` 只影响块内的命令，块外恢复（配 `Painter::clip` 读回）。
    #[test]
    fn clipped_is_scoped_to_the_block() {
        let clip = Rect::new(5.0, 5.0, 10.0, 10.0);
        let mut p = Painter::new(1.0);
        p.solid(rect(), Color::WHITE);
        assert_eq!(p.clip(), None);
        p.clipped(Some(clip), |p| {
            assert_eq!(p.clip(), Some(clip));
            p.solid(rect(), Color::WHITE);
        });
        assert_eq!(p.clip(), None, "块外恢复");
        assert_eq!(p.commands()[0].clip, None);
        assert_eq!(p.commands()[1].clip, Some(clip));
    }

    /// **已经在沙箱里 ⇒ painter 直接就是被裁的**（不需要 `painter_clipped`）。
    ///
    /// `Ui::view_at(ViewMode::Clip)` / `scroll_at` / 严格窗口把强制裁剪层写进
    /// `DrawQueue::clip`（绝对逻辑屏幕坐标）。沙箱内取到的 painter 的 [`Painter::clip`]
    /// **就是那一层**，录出的每条命令自带它 ⇒ 控件什么都不用做。
    /// `clipped(..)` 的用途是**覆盖**：临时更窄（本控件自己再裁一刀），或传 `None` 主动不裁。
    ///
    /// `Expand` 沙箱不产生强制层（`clip_for_view` 原样传递外层），因此那里的 painter
    /// 报的是外层（通常是 `None`）——也是对的。
    #[test]
    fn sandbox_clip_is_what_painter_already_reports() {
        let sandbox = Rect::new(10.0, 20.0, 300.0, 200.0);
        let mut p = Painter::new(1.0);
        // 模拟 `view_at(Clip)`：进入沙箱 = 写这个字段
        p.q.clip = Some(sandbox);
        assert_eq!(p.clip(), Some(sandbox), "沙箱内 painter 直接给出沙箱裁剪");
        p.solid(rect(), Color::WHITE);
        p.text(
            rect(),
            "x",
            10.0,
            Color::WHITE,
            None,
            crate::draw::TextAlign::Left,
            crate::draw::TextVAlign::Center,
            None,
            None,
        );
        for d in p.commands() {
            assert_eq!(d.clip, Some(sandbox), "沙箱内录出的命令都自带沙箱裁剪");
        }
        // 覆盖：只影响块内
        let narrower = Rect::new(15.0, 25.0, 10.0, 10.0);
        p.clipped(Some(narrower), |p| {
            p.solid(rect(), Color::WHITE);
        });
        assert_eq!(
            p.commands().last().expect("just pushed").clip,
            Some(narrower),
            "覆盖只作用在块内"
        );
        assert_eq!(p.clip(), Some(sandbox), "块外回到沙箱层");
    }

    /// `Position` / `Size` 的 Logical 换算用 painter 自己的 `scale`。
    #[test]
    fn scale_applies_at_the_api_boundary() {
        let mut p = Painter::new(2.0);
        p.rounded_at(Position::Logical(Vec2::new(3.0, 4.0)), Size::Logical(Vec2::new(5.0, 6.0)), 0.0, Color::WHITE);
        assert_eq!(p.commands()[0].rect, Rect::new(6.0, 8.0, 10.0, 12.0));
    }
}
