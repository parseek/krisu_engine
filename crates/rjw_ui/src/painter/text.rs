//! 文本绘制原语（记录式：写 `DrawKind::Text`，排版由 `Ui` 侧的字体缓存完成）。

use std::sync::Arc;

use rjw_color::Color;
use rjw_text::Buffer;
use rjw_transform::Rect;

use super::Painter;
use crate::draw::{TextRamp, TextAlign, TextVAlign, text_cmd, text_cmd_ramp};

impl Painter {
    /// **文本绘制**（逻辑坐标）。
    ///
    /// `soft_clip` = **文本内容裁剪**（相对文本块左上角；与 [`Painter::clip`] 的环境裁剪
    /// 求交后交给排版/绘制）。内容自洽（换行 / "…"省略后不出界）时传 `None`。
    /// `buf = Some` 时直接用预排版缓冲（见 `Ui::wrap_buffer`）。
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
        family: Option<Arc<str>>,
        align: TextAlign,
        valign: TextVAlign,
        soft_clip: Option<Rect>,
        buf: Option<Arc<Buffer>>,
    ) {
        let elem = self.q.elem_hint();
        let seq = self.q.next_seq();
        let (depth, win, outer) = (self.q.depth, self.q.cur_win, self.q.clip);
        self.q.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            rect,
            Arc::from(text),
            size,
            color,
            align,
            valign,
            family,
            soft_clip,
            outer,
            buf,
        ));
    }

    /// **不附加内容裁剪的文本绘制**：调用方承诺文本内容自洽（换行 / 省略后不出界）。
    ///
    /// **仍服从环境裁剪层**（[`Painter::clip`]：ScrollView 可视区 / Clip 沙箱）。
    #[allow(clippy::too_many_arguments)]
    pub fn text_noclip(
        &mut self,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
        family: Option<Arc<str>>,
        align: TextAlign,
        valign: TextVAlign,
        buf: Option<Arc<Buffer>>,
    ) {
        self.text(rect, text, size, color, family, align, valign, None, buf);
    }

    /// **带首末两色渐变的文本绘制**（[`TextRamp`]）：与 [`Painter::text`] 同语义，
    /// 只是把逐字形的顶点色换成"在文本块内的相对位置取色 × `color`"。
    ///
    /// `ramp = None` 时与 [`Painter::text`] 逐字等价（调用方不必分支）。
    #[allow(clippy::too_many_arguments)]
    pub fn text_ramp(
        &mut self,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
        family: Option<Arc<str>>,
        align: TextAlign,
        valign: TextVAlign,
        soft_clip: Option<Rect>,
        buf: Option<Arc<Buffer>>,
        ramp: Option<TextRamp>,
    ) {
        let elem = self.q.elem_hint();
        let seq = self.q.next_seq();
        let (depth, win, outer) = (self.q.depth, self.q.cur_win, self.q.clip);
        self.q.queue.push(text_cmd_ramp(
            depth,
            seq,
            win,
            elem,
            rect,
            Arc::from(text),
            size,
            color,
            align,
            valign,
            family,
            soft_clip,
            outer,
            buf,
            ramp,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 文本命令：`elem` 取 `elem_hint`、`seq` 递增、环境裁剪进 `UiDraw.clip`、
    /// 软裁剪进 `DrawKind::Text.clip`——两条裁剪**各就各位**（历史 bug：
    /// 外层裁剪与文本软裁剪混为一谈，滚动容器里的文本被裁到屏幕外）。
    #[test]
    fn text_records_both_clip_layers() {
        let outer = Rect::new(10.0, 10.0, 100.0, 50.0);
        let soft = Rect::new(0.0, 0.0, 40.0, 20.0);
        let mut p = Painter::new(1.0);
        p.clipped(Some(outer), |p| {
            p.text(
                Rect::new(12.0, 14.0, 60.0, 20.0),
                "hello",
                14.0,
                Color::WHITE,
                None,
                TextAlign::Left,
                TextVAlign::Center,
                Some(soft),
                None,
            );
        });
        let d = &p.commands()[0];
        assert_eq!(d.elem, 1);
        assert_eq!(d.seq, 1);
        assert_eq!(d.clip, Some(outer), "环境裁剪进 UiDraw.clip");
        match &d.kind {
            crate::draw::DrawKind::Text { clip, text, .. } => {
                assert_eq!(*clip, Some(soft), "文本软裁剪留在 DrawKind::Text::clip");
                assert_eq!(&**text, "hello");
            }
            other => panic!("应为文本命令，实际 {other:?}"),
        }
    }

    /// `text_noclip` 只去掉软裁剪，**环境裁剪仍在**（父级 ScrollView 躲不掉）。
    #[test]
    fn text_noclip_keeps_the_ambient_clip() {
        let outer = Rect::new(1.0, 1.0, 10.0, 10.0);
        let mut p = Painter::new(1.0);
        p.clipped(Some(outer), |p| {
            p.text_noclip(
                Rect::new(1.0, 1.0, 5.0, 5.0),
                "x",
                10.0,
                Color::WHITE,
                None,
                TextAlign::Left,
                TextVAlign::Center,
                None,
            );
        });
        let d = &p.commands()[0];
        assert_eq!(d.clip, Some(outer));
        assert!(matches!(&d.kind, crate::draw::DrawKind::Text { clip: None, .. }));
    }

    /// **`text_ramp` 把渐变写进命令**，且 `ramp = None` 时与 [`Painter::text`] 逐位等价
    /// （调用方不必为"没有渐变"分叉）。
    #[test]
    fn text_ramp_records_the_ramp_and_none_matches_plain_text() {
        let mut p = Painter::new(1.0);
        let r = Rect::new(0.0, 0.0, 40.0, 20.0);
        let ramp = TextRamp::horizontal(Color::RED, Color::BLUE);
        p.text_ramp(
            r, "x", 14.0, Color::WHITE, None, TextAlign::Left, TextVAlign::Center, None, None,
            Some(ramp),
        );
        p.text(r, "x", 14.0, Color::WHITE, None, TextAlign::Left, TextVAlign::Center, None, None);
        p.text_ramp(
            r, "x", 14.0, Color::WHITE, None, TextAlign::Left, TextVAlign::Center, None, None, None,
        );
        let cmds = p.commands();
        let ramp_of = |i: usize| match &cmds[i].kind {
            crate::draw::DrawKind::Text { ramp, .. } => *ramp,
            other => panic!("应为文本命令，实际 {other:?}"),
        };
        assert_eq!(ramp_of(0), Some(ramp), "渐进入命令");
        assert_eq!(ramp_of(1), None, "单色路径不带渐变");
        assert_eq!(ramp_of(2), None, "显式 None 与单色路径一致");
        // 除渐变外逐位相同（同 rect；`elem`/`seq` 按录制序递增，不能比）。
        assert_eq!(cmds[0].rect, cmds[2].rect);
        assert_eq!(cmds[0].seq + 1, cmds[1].seq, "序号逐条递增");
    }
}
