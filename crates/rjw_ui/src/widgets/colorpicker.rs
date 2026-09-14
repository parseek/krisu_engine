//! **颜色选择器**（组合控件，只依赖公开 API）。
//!
//! - **色块预览**：显示当前颜色，并在色块中央用对比色（黑/白按亮度自动选）写出
//!   `#RRGGBB`（或带 Alpha 的 `#RRGGBBAA`）——一眼能看出颜色**和**它的十六进制；
//! - **R/G/B(/A) 滑条**：`0..=1` 逐通道调，直接写回 `&mut Color`；
//! - **可选十六进制输入框**（[`ColorPicker::with_hex`]）：输入 `#3af` / `3af` /
//!   `#33AAFF` / `#33AAFF80`（`#` 与大小写都可省），失焦 / 每次编辑即解析回颜色；
//!   非法输入只更新文本、**不动颜色**（打字过程中不会闪回）；
//! - **无内部状态**：颜色直接写在 `&mut Color` 上。"这一帧颜色变了吗"由调用方前后比较
//!   ——与 `ui.slider_at(..)` 返回新值同一个路子，因此不需要往 [`Response`] 上加字段。
//!
//! ⚠ 控件协议里的尺寸一律是**物理像素**（`Widget::size` 的返回值会被直接当作物理
//! 像素用于布局；`Theme` 在 `Ui` 内已按 DPI 预乘），本控件与 [`NumberInput`](super::NumberInput)
//! 一致。
//!
//! ```no_run
//! # use rjw_ui::{ColorPicker, Ui};
//! # fn demo(ui: &mut Ui, mut color: rjw_color::Color) {
//! let before = color;
//! ui.add(ColorPicker::new("tint", &mut color).alpha(true));
//! if color != before {
//!     // 颜色变了 —— 做点什么
//! }
//! # }
//! ```

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::draw::{CornerRadius, TextVAlign};
use crate::{Response, TextAlign, Ui, Widget};

/// 预览色块高度（物理像素）。
const PREVIEW_H: f32 = 22.0;
/// 通道标签宽度（物理像素）。
const LABEL_W: f32 = 14.0;
/// 预览色块圆角（物理像素）。
const SWATCH_RADIUS: f32 = 3.0;

/// 颜色选择器（预览色块 + R/G/B(/A) 滑条 + 可选十六进制输入）。
pub struct ColorPicker<'a> {
    id: &'a str,
    color: &'a mut Color,
    /// 是否显示 Alpha 滑条。
    alpha: bool,
    /// 可选的十六进制编辑缓冲（跨帧由调用方持有）。
    hex: Option<&'a mut String>,
}

impl<'a> ColorPicker<'a> {
    /// 主构造：颜色直接写在 `&mut Color` 上。
    pub fn new(id: &'a str, color: &'a mut Color) -> Self {
        Self { id, color, alpha: false, hex: None }
    }

    /// 显示 Alpha 滑条（默认不显示：多数取色只关心 RGB）。
    pub fn alpha(mut self, on: bool) -> Self {
        self.alpha = on;
        self
    }

    /// 追加一个十六进制输入框（缓冲由调用方跨帧持有，形如 `String::new()`）。
    ///
    /// 里面的内容会被本控件每帧重写成当前颜色——除非输入非法（那时只保留文本）。
    pub fn with_hex(mut self, buf: &'a mut String) -> Self {
        self.hex = Some(buf);
        self
    }
}

/// 颜色分量 → `0..=1` 的 `[f32; 4]`。
#[inline]
fn rgba(c: Color) -> [f32; 4] {
    c.into()
}

/// 感知亮度（决定色块上的文字用黑还是白）。
#[inline]
fn luma(c: Color) -> f32 {
    let a = rgba(c);
    0.2126 * a[0] + 0.7152 * a[1] + 0.0722 * a[2]
}

/// `0..=1` 分量 → `u8`。
#[inline]
fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// 格式化为 `#RRGGBB`（`alpha` 为真时 `#RRGGBBAA`）——**大写**，便于与设计稿对照。
pub fn color_hex(c: Color, with_alpha: bool) -> String {
    let a = rgba(c);
    if with_alpha {
        format!(
            "#{:02X}{:02X}{:02X}{:02X}",
            to_u8(a[0]),
            to_u8(a[1]),
            to_u8(a[2]),
            to_u8(a[3])
        )
    } else {
        format!("#{:02X}{:02X}{:02X}", to_u8(a[0]), to_u8(a[1]), to_u8(a[2]))
    }
}

/// 解析十六进制颜色（`#` 可省、大小写不敏感；接受 3 / 4 / 6 / 8 位）。
///
/// 3 位（`f0a`）与 4 位（`f0a8`）按 CSS 规则把每位**重复**一次展开
/// （`f` → `ff`）。位数不对或出现非十六进制字符时返回 `None`（调用方据此**不动颜色**）。
pub fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#').trim();
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let b: Vec<u8> = s
        .as_bytes()
        .iter()
        .map(|c| (*c as char).to_digit(16).map(|d| d as u8))
        .collect::<Option<_>>()?;
    let (r, g, b_, a) = match b.len() {
        3 => (b[0] * 17, b[1] * 17, b[2] * 17, 255),
        4 => (b[0] * 17, b[1] * 17, b[2] * 17, b[3] * 17),
        6 => (b[0] * 16 + b[1], b[2] * 16 + b[3], b[4] * 16 + b[5], 255),
        8 => (
            b[0] * 16 + b[1],
            b[2] * 16 + b[3],
            b[4] * 16 + b[5],
            b[6] * 16 + b[7],
        ),
        _ => return None,
    };
    Some(Color::rgba_u8(r, g, b_, a))
}

impl Widget for ColorPicker<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        let row = ui.theme.row_h;
        let rows = 3.0 + if self.alpha { 1.0 } else { 0.0 } + if self.hex.is_some() { 1.0 } else { 0.0 };
        Vec2::new(ui.theme.input.min_w, PREVIEW_H + row * rows)
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        let row = ui.theme.row_h;
        let mut y = rect.y;

        // ── 预览色块：当前色 + 对比色十六进制文本 ──
        let preview = Rect::new(rect.x, y, rect.w, PREVIEW_H);
        let (border, font_size) = {
            let st = &ui.theme.input;
            (st.border, st.font_size)
        };
        let hovered = ui.hit_abs(&preview);
        ui.push_panel_like(
            preview,
            *self.color,
            border,
            1.0,
            CornerRadius::all(SWATCH_RADIUS),
            1,
        );
        // 色块上的文字用对比色：亮底黑字、暗底白字（否则读不出来）。
        let ink = if luma(*self.color) > 0.55 {
            Color::rgba_u8(20, 20, 20, 255)
        } else {
            Color::rgba_u8(240, 240, 240, 255)
        };
        ui.push_text_rect(
            preview,
            &color_hex(*self.color, self.alpha),
            font_size,
            ink,
            None,
            TextAlign::Center,
            TextVAlign::Center,
            None,
            None,
        );
        y += PREVIEW_H;

        // ── 逐通道滑条 ──
        let mut c = rgba(*self.color);
        let labels = ["R", "G", "B", "A"];
        let n = if self.alpha { 4 } else { 3 };
        for (i, lab) in labels.iter().take(n).enumerate() {
            let bar = Rect::new(rect.x + LABEL_W, y, (rect.w - LABEL_W).max(1.0), row);
            let label_rect = Rect::new(rect.x, y, LABEL_W, row);
            let (fg, label_fs) = (ui.theme.label.color, ui.theme.label.font_size);
            ui.push_text_rect(
                label_rect,
                lab,
                label_fs,
                fg,
                None,
                TextAlign::Left,
                TextVAlign::Center,
                None,
                None,
            );
            // 滑条 id 带通道后缀（`{id}::r` 等），避免四个滑条共用一个状态键。
            let sid = format!("{}::{}", self.id, lab.to_ascii_lowercase());
            c[i] = ui.slider_at(&sid, bar, 0.0..=1.0, c[i]);
            y += row;
        }
        *self.color = Color::from(c);

        // ── 可选十六进制输入 ──
        if let Some(buf) = self.hex {
            let hid = format!("{}::hex", self.id);
            let hrect = Rect::new(rect.x, y, rect.w, row);
            // 非聚焦时把文本同步成当前颜色；聚焦时保留用户输入（否则打字会被覆盖）。
            let focus_key = ui.id_for(hid.as_str());
            let focused = ui
                .state()
                .focused
                .as_ref()
                .is_some_and(|f| f.as_str() == focus_key.as_str());
            if !focused {
                let want = color_hex(*self.color, self.alpha);
                if *buf != want {
                    *buf = want;
                }
            }
            ui.text_input_at(&hid, hrect, buf);
            // 只在**解析成功**时写回颜色：打字中途（`#3a`）不闪回上一个颜色。
            if let Some(parsed) = parse_hex(buf) {
                let mut p = rgba(parsed);
                // 不显示 Alpha 时不改动原 Alpha（否则会把不透明色变成透明）。
                if !self.alpha {
                    p[3] = rgba(*self.color)[3];
                }
                *self.color = Color::from(p);
            }
        }

        Response { hovered, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_is_uppercase() {
        let c = Color::rgba_u8(0x33, 0xAA, 0xFF, 0x80);
        assert_eq!(color_hex(c, false), "#33AAFF");
        assert_eq!(color_hex(c, true), "#33AAFF80");
        // 往返：解析回同一颜色（±1/255 的量化误差以内）
        let back = parse_hex(&color_hex(c, true)).unwrap();
        let (a, b) = (rgba(c), rgba(back));
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() <= 1.0 / 255.0, "分量 {i} 往返失真");
        }
    }

    #[test]
    fn parse_hex_accepts_the_css_forms() {
        // 6 位 / 3 位（每位重复）/ 4 位 / 8 位；`#` 与大小写可省。
        let want = Color::rgba_u8(0xFF, 0x00, 0xAA, 255);
        for s in ["#ff00aa", "FF00AA", " ff00aa ", "#f0a", "f0a"] {
            assert_eq!(parse_hex(s), Some(want), "应接受 {s:?}");
        }
        assert_eq!(parse_hex("#f0a8"), Some(Color::rgba_u8(0xFF, 0x00, 0xAA, 0x88)));
        assert_eq!(parse_hex("none"), None);
    }

    #[test]
    fn parse_hex_rejects_bad_input_without_panicking() {
        // 打字中途的输入必须是"返回 None"，**不能 panic、不能误解成别的颜色**。
        for s in ["", "#", "#3", "#33", "#33333", "#3333333", "gg", "#zzz", "0x33"] {
            assert_eq!(parse_hex(s), None, "{s:?} 应判为非法");
        }
    }

    #[test]
    fn luma_picks_readable_ink() {
        assert!(luma(Color::WHITE) > 0.55, "白底应配黑字");
        assert!(luma(Color::BLACK) < 0.55, "黑底应配白字");
        // 中性灰：两侧都可读，只要阈值把明暗分开即可
        assert!(luma(Color::rgba_u8(230, 230, 230, 255)) > 0.55);
        assert!(luma(Color::rgba_u8(20, 20, 20, 255)) < 0.55);
    }
}
