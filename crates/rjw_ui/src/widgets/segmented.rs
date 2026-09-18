//! **分段按钮组**（segmented control）：一组**互斥**选项拼成一个整体——相邻段共享边、
//! 只有整组的**外侧角**是圆的，选中的那一段高亮。
//!
//! ```no_run
//! # use rjw_ui::{Position, Segmented, Ui, UiAdd};
//! # use glam::Vec2;
//! # fn f(ui: &mut Ui, mut idx: usize) {
//! // 任意容器里都能放（`pack` / `row` / `window`…），也可以绝对定位：
//! ui.add_at(
//!     Position::Logical(Vec2::new(16.0, 16.0)),
//!     Segmented::new("density", &["紧凑", "标准", "宽松"], &mut idx),
//! );
//! # }
//! ```
//!
//! # 为什么单独一个控件
//!
//! "拼在一起"不是三个 `Button` 相邻就能得到的：分段控件需要知道**自己在组里的位置**
//! （首/中/尾决定哪两个角是圆的、哪条边要画分隔线）。让每个按钮去猜布局不可能稳，
//! 所以由这个控件**一次画完整组**（宽度按各段文字实测 + 按 `rect.w` 归一）。
//!
//! # 边框宽 = 0 也要分得开
//!
//! 分隔线**独立于 `ButtonStyle::border_w`**：边框宽 > 0 时用 `border` 色，边框被关掉
//! （`border_w = 0`，平面风格）时退化成 `Palette::surface_dim`——否则三段同色连成一条，
//! 看不出这是三个选项（实测踩过：`border_w = 0` + 同一底色 = 一条长条）。
//! 组的**外侧轮廓**仍走 `border_w`（关掉就真的没有外框），与其它控件一致。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::draw::{CornerRadius, Position, Size, TextAlign, TextVAlign};
use crate::style::{ButtonStyle, Theme};
use crate::ui::Ui;
use crate::widgets::{Response, Sense, Widget};

/// 分段按钮组（互斥选项拼在一起；选中项写入 `&mut usize`）。
pub struct Segmented<'a> {
    id: &'a str,
    labels: &'a [&'a str],
    /// 选中段索引（越界自动夹住；点击后写回新索引）。
    selected: &'a mut usize,
    font_size: Option<Size<f32>>,
}

impl<'a> Segmented<'a> {
    /// `labels` = 各段文字（至少 1 个）；`selected` = 选中索引（跨帧由调用方持有）。
    pub fn new(id: &'a str, labels: &'a [&'a str], selected: &'a mut usize) -> Self {
        Self { id, labels, selected, font_size: None }
    }

    /// 字号（[`Size<f32>`]：逻辑（默认）或物理；默认 `ButtonStyle::font_size`）。
    pub fn font_size(mut self, s: impl Into<Size<f32>>) -> Self {
        self.font_size = Some(s.into());
        self
    }

    /// 解析样式与字号（未设置回落 [`Theme::button`]）。
    fn resolve(&self, theme: &Theme, scale: f32) -> (ButtonStyle, Option<std::sync::Arc<str>>, f32) {
        let st = theme.button.clone();
        let fs = self
            .font_size
            .map(|s| s.to_physical(scale))
            .unwrap_or(st.font_size);
        (st.clone(), st.font_family.clone(), fs)
    }
}

impl Widget for Segmented<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (st, fam, fs) = self.resolve(&ui.theme, ui.scale());
        let n = self.labels.len();
        // ① 先量（与旧 `size()` 同式：每段文字宽 + 内边距，再保证每段至少 `font_size` 宽）
        let size = {
            let nn = n.max(1);
            let mut w = 0.0;
            let mut h: f32 = 0.0;
            for l in self.labels {
                let t = ui.text_size(l, fs, fam.as_deref());
                w += t.x + st.padding.x * 2.0;
                h = h.max(t.y);
            }
            Vec2::new(w.max(fs * nn as f32), h + st.padding.y * 2.0)
        };
        // ② 申请（占光标；`add_at` 的绝对定位由 `place_once` 覆盖）
        let rect = ui.allocate(size);
        if n == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
            return Response { rect, ..Default::default() };
        }
        // 各段宽度：按文字实测，再整体缩放到 `rect.w`（两次测量必然同值，缩放只为消缝）。
        let mut widths: Vec<f32> = Vec::with_capacity(n);
        for l in self.labels {
            widths.push(ui.text_size(l, fs, fam.as_deref()).x + st.padding.x * 2.0);
        }
        let total: f32 = widths.iter().sum();
        let k = if total > 0.0 { rect.w / total } else { 1.0 };
        let r = st.radius;
        // ③ 逐段收交互（`Sense::DRAG`：段自己消费按下，别让外层容器当成"拖窗口/拖面板"）。
        //    先收完交互再画：绘制要用一个 painter 一把画完（painter 存活期内借不到 `ui`）。
        let mut segs: Vec<(Rect, Response)> = Vec::with_capacity(n);
        let mut x = rect.x;
        let mut clicked = false;
        let mut hovered_any = false;
        let mut pressed_any = false;
        for (i, w0) in widths.iter().enumerate() {
            let w = w0 * k;
            let seg = Rect::new(x, rect.y, w, rect.h);
            let seg_id = format!("{}::seg{i}", self.id);
            let (seg, resp) = ui.allocate_sense_at(
                Position::Physical(Vec2::new(seg.x, seg.y)),
                seg_id.as_str(),
                Vec2::new(seg.w, seg.h),
                Sense::DRAG,
            );            hovered_any |= resp.hovered;
            pressed_any |= resp.pressed;
            if resp.clicked {
                clicked = true;
                *self.selected = i;
            }
            segs.push((seg, resp));
            x += w;
        }
        // ④ 画（一个绘制块一个 painter；组底 / 高亮段 / 分隔线 / 文字都在这里）
        let sep = if st.border_w > 0.0 { st.border } else { ui.theme.palette.surface_dim };
        let fg = st.fg;
        let labels = self.labels;
        let selected = *self.selected;
        let p = ui.painter();
        // 组的整体底色 + 外框（一个圆角矩形 ⇒ 只有**外侧角**是圆的）。
        p.panel(rect, st.bg, st.border, st.border_w, r);
        for (i, (seg, resp)) in segs.iter().enumerate() {
            // 选中 / 悬停段高亮：圆角只取组的**外侧**两角（内侧直角，拼缝处不露底）。
            if selected == i || resp.hovered || resp.pressed {
                let bg = if selected == i { st.bg_pressed } else { st.bg_hover };
                let cr = CornerRadius {
                    tl: if i == 0 { r.tl } else { 0.0 },
                    bl: if i == 0 { r.bl } else { 0.0 },
                    tr: if i == n - 1 { r.tr } else { 0.0 },
                    br: if i == n - 1 { r.br } else { 0.0 },
                };
                p.panel(*seg, bg, Color::TRANSPARENT, 0.0, cr);
            }
            // 段间隔线（居中 1px；避开外框的宽度，短一截更像分隔而不是边框）。
            if i > 0 {
                let inset = st.border_w.max(0.0);
                p.solid(
                    Rect::new(seg.x - 0.5, rect.y + inset, 1.0, (rect.h - inset * 2.0).max(1.0)),
                    sep,
                );
            }
            p.text(
                *seg,
                labels[i],
                fs,
                fg,
                fam.clone(),
                TextAlign::Center,
                TextVAlign::Center,
                None,
                None,
            );
        }
        Response {
            rect,
            hovered: hovered_any,
            pressed: pressed_any,
            clicked,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 外侧角才圆、内侧直角——"拼在一起"的几何契约（纯函数式地重算一遍角点规则）。
    fn corners(i: usize, n: usize, r: CornerRadius) -> CornerRadius {
        CornerRadius {
            tl: if i == 0 { r.tl } else { 0.0 },
            bl: if i == 0 { r.bl } else { 0.0 },
            tr: if i == n - 1 { r.tr } else { 0.0 },
            br: if i == n - 1 { r.br } else { 0.0 },
        }
    }

    #[test]
    fn only_outer_corners_are_rounded() {
        let r = CornerRadius { tl: 6.0, tr: 6.0, br: 6.0, bl: 6.0 };
        let n = 3;
        // 首段：左上/左下圆，右上/右下直角。
        let a = corners(0, n, r);
        assert_eq!((a.tl, a.bl, a.tr, a.br), (6.0, 6.0, 0.0, 0.0));
        // 中段：四角全直角（否则会出现"内凹缺口"）。
        let b = corners(1, n, r);
        assert_eq!((b.tl, b.bl, b.tr, b.br), (0.0, 0.0, 0.0, 0.0));
        // 尾段：右上/右下圆。
        let c = corners(2, n, r);
        assert_eq!((c.tl, c.bl, c.tr, c.br), (0.0, 0.0, 6.0, 6.0));
        // 单段 = 整组（四角全圆）。
        let d = corners(0, 1, r);
        assert_eq!((d.tl, d.bl, d.tr, d.br), (6.0, 6.0, 6.0, 6.0));
    }
}
