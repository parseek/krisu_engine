//! **颜色选择器**（组合控件，只依赖公开 API）。
//!
//! # 形态：内联色块 + 点击弹出的调节面板
//!
//! 内联部分只占**一行**（色块 + 当前颜色的十六进制 + 右侧 `▾`），点一下才在控件
//! **正下方**弹出调节面板（R/G/B(/A) 滑条 + 可选十六进制输入）。这样：
//!
//! - 一行里能并排放好几个取色器，不会像"内联铺开三个滑条"那样把所在窗口撑高、
//!   并把后续行挤到一起（控件的 `size()` 与实际绘制内容一旦不一致，布局就会重叠）；
//! - 面板是**独立置顶窗口**（`z = WIN_TOPMOST`，与下拉框浮层同一机制），不受父容器
//!   裁剪 / 换行约束，也不会撑大父级。
//!
//! 点击面板以外（且不在色块上）收起；`Esc` 也收起。
//!
//! # 其它
//!
//! - **预览色块**显示当前颜色，并在中央用**对比色**（黑/白按亮度自动选，[`ink_on`]）
//!   写出 `#RRGGBB`（或带 Alpha 的 `#RRGGBBAA`）——一眼同时看到颜色与它的十六进制；
//! - **十六进制输入**（[`ColorPicker::with_hex`]）：接受 `#3af` / `3af` / `#33AAFF` /
//!   `#33AAFF80`（`#` 与大小写可省，3/4 位按 CSS 规则把每位重复一次展开）；
//!   **只在解析成功时写回颜色**——打字中途（`#3a`）只更新文本、不动颜色；
//! - **无内部状态**：颜色直接写在 `&mut Color` 上。"这一帧颜色变了吗"由调用方前后比较
//!   ——与 `ui.slider_at(..)` 返回新值同一个路子，因此不需要往 [`Response`] 加字段。
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

use crate::draw::{CornerRadius, DrawKind, Icon, Position, TextVAlign};
use crate::id::IdAbsolute;
use crate::layout::Child;
use crate::style::PanelStyle;
use crate::ui::{UiAdd, WIN_TOPMOST};
use crate::{Response, TextAlign, Ui, Widget};

/// 内联色块高度（物理像素）。
const SWATCH_H: f32 = 22.0;
/// 弹出面板内预览色块高度（物理像素）。
const POPUP_PREVIEW_H: f32 = 26.0;
/// 通道标签宽度（物理像素）。
const LABEL_W: f32 = 14.0;
/// 弹出面板内边距（物理像素）。
const POPUP_PAD: f32 = 6.0;
/// 色块圆角（物理像素）。
const SWATCH_RADIUS: f32 = 3.0;

/// 颜色选择器（内联色块 → 点击弹出一行滑条）。
pub struct ColorPicker<'a> {
    id: &'a str,
    color: &'a mut Color,
    /// 是否在面板里显示 Alpha 滑条。
    alpha: bool,
    /// 可选的十六进制编辑缓冲（跨帧由调用方持有）。
    hex: Option<&'a mut String>,
    /// 弹出面板宽度（物理像素；`None` = 内联宽度的 1.9 倍，至少 190）。
    popup_w: Option<f32>,
}

impl<'a> ColorPicker<'a> {
    /// 主构造：颜色直接写在 `&mut Color` 上。
    pub fn new(id: &'a str, color: &'a mut Color) -> Self {
        Self { id, color, alpha: false, hex: None, popup_w: None }
    }

    /// 面板里显示 Alpha 滑条（默认不显示：多数取色只关心 RGB）。
    pub fn alpha(mut self, on: bool) -> Self {
        self.alpha = on;
        self
    }

    /// 追加一个十六进制输入框（缓冲由调用方跨帧持有，形如 `String::new()`）。
    ///
    /// 里面的内容会被本控件每帧重写成当前颜色——除非正在编辑（聚焦）。
    pub fn with_hex(mut self, buf: &'a mut String) -> Self {
        self.hex = Some(buf);
        self
    }

    /// 弹出面板宽度（物理像素）。
    pub fn popup_width(mut self, w: f32) -> Self {
        self.popup_w = Some(w);
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
pub fn luma(c: Color) -> f32 {
    let a = rgba(c);
    0.2126 * a[0] + 0.7152 * a[1] + 0.0722 * a[2]
}

/// 在给定底色上可读的"墨色"（亮底黑字、暗底白字）。
#[inline]
pub fn ink_on(c: Color) -> Color {
    if luma(c) > 0.55 {
        Color::rgba_u8(20, 20, 20, 255)
    } else {
        Color::rgba_u8(240, 240, 240, 255)
    }
}

/// `0..=1` 分量 → `u8`。
#[inline]
fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// 格式化为 `#RRGGBB`（`with_alpha` 为真时 `#RRGGBBAA`）——**大写**，便于与设计稿对照。
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

/// 把 `src` 的 alpha 写进 `dst`（不显示 Alpha 滑条时保持原透明度）。
#[inline]
fn rgba_keep_alpha(dst: &mut Color, src: Color) {
    let mut d = rgba(*dst);
    d[3] = rgba(src)[3];
    *dst = Color::from(d);
}

/// 画一个"色块"（圆角矩形填充 + 居中的十六进制文本）。
fn push_swatch(ui: &mut Ui, rect: Rect, color: Color, with_alpha: bool, font_size: f32) {
    let border = ui.theme.input.border;
    ui.push_panel_like(rect, color, border, 1.0, CornerRadius::all(SWATCH_RADIUS), 1);
    ui.push_text_rect(
        rect,
        &color_hex(color, with_alpha),
        font_size,
        ink_on(color),
        None,
        TextAlign::Center,
        TextVAlign::Center,
        None,
        None,
    );
}

impl Widget for ColorPicker<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        // 内联只占**一行**：面板弹出，不参与这里的尺寸结算。
        Vec2::new(ui.theme.input.min_w, SWATCH_H)
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        // 先解构：`color`（&mut Color）与 `hex`（Option<&mut String>）是**互不相干**的
        // 借用，闭包只捕获 `hex`、颜色值在外面读写成 `Copy` 的 `Color`，两边不打架。
        let ColorPicker { id, color, alpha, hex, popup_w } = self;
        let color_in = *color;
        let id_for = ui.id_for(id);
        let abs = id_for.to_static();
        let btn = ui.mouse_left();
        let hit = ui.hit_abs(&rect);
        let mut open = ui.state_mut().widget(&id_for).popup_open;

        // ── 内联色块（整行）：当前色 + 十六进制 + 右侧 ▾ 提示 ──
        let label_fs = ui.theme.label.font_size;
        if hit {
            ui.set_cursor(crate::UiCursor::Default);
        }
        push_swatch(ui, rect, color_in, alpha, label_fs);
        let ink = ink_on(color_in);
        // 展开箭头用**矢量图标**（与字体无关）。
        ui.push_draw(
            DrawKind::Icon {
                icon: if open { Icon::ChevronUp } else { Icon::ChevronDown },
                color: ink,
            },
            Rect::new(rect.x + rect.w - 16.0, rect.y + (rect.h - 12.0) * 0.5, 12.0, 12.0),
        );

        // 点色块 = 开关面板。`claim_press` 阻止外层窗口把这次按下当作窗口拖拽基准。
        if btn.down_edge() && hit {
            ui.claim_press();
            open = !open;
            ui.state_mut().widget(&id_for).popup_open = open;
        }
        // Esc 收起（面板里没有焦点控件，直接看当前帧的边沿）。
        if open && ui.key_down_edge(winit::keyboard::KeyCode::Escape) {
            open = false;
            ui.state_mut().widget(&id_for).popup_open = false;
        }

        if !open {
            return Response { hovered: hit, ..Default::default() };
        }

        // ── 弹出面板：独立置顶窗口（与下拉框浮层同一机制）──
        let row = ui.theme.row_h;
        let pw = popup_w.unwrap_or_else(|| (rect.w * 1.9).max(190.0));
        let n_rows = 3.0 + if alpha { 1.0 } else { 0.0 } + if hex.is_some() { 1.0 } else { 0.0 };
        let ph = POPUP_PAD * 2.0 + POPUP_PREVIEW_H + row * n_rows;
        let popup_pos = Vec2::new(rect.x, rect.y + rect.h + 2.0);
        let popup_raw = format!("{id}::popup");

        // 强制哨兵 z：`window_at` 的 `entry().or_insert()` 会保留既有值。
        ui.state_mut()
            .window_z
            .insert(IdAbsolute::owned(format!("{}::popup", abs.as_str())), WIN_TOPMOST);

        let cs = ui.theme.combo.clone();
        let panel_style = PanelStyle {
            bg: cs.menu_bg.into(),
            border: cs.menu_border,
            border_w: 1.0,
            padding: 0.0,
            radius: cs.menu_radius,
            bg_image: None,
        };
        // 面板内容是否被点中（用于"点外部收起"）——闭包外读取。
        let mut inside = false;
        let mut color_out = color_in;
        ui.window(&popup_raw)
            .pos(Position::Physical(popup_pos))
            .style(panel_style)
            .show(|w| {
                let ui = w.ui_mut();
                inside = ui.hit_abs(&Rect::new(0.0, 0.0, pw, ph));
                if btn.down_edge() && inside {
                    ui.claim_press();
                }
                let body_w = pw - POPUP_PAD * 2.0;
                let mut y = POPUP_PAD;

                let fs = ui.theme.label.font_size;
                push_swatch(
                    ui,
                    Rect::new(POPUP_PAD, y, body_w, POPUP_PREVIEW_H),
                    color_in,
                    alpha,
                    fs,
                );
                y += POPUP_PREVIEW_H;

                // 逐通道滑条
                let mut c = rgba(color_in);
                let labels = ["R", "G", "B", "A"];
                for (i, lab) in labels.iter().take(if alpha { 4 } else { 3 }).enumerate() {
                    let (fg, lfs) = (ui.theme.label.color, ui.theme.label.font_size);
                    ui.push_text_rect(
                        Rect::new(POPUP_PAD, y, LABEL_W, row),
                        lab,
                        lfs,
                        fg,
                        None,
                        TextAlign::Left,
                        TextVAlign::Center,
                        None,
                        None,
                    );
                    const SLIDER_DIST: f32 = 3.0;
                    let bar = Rect::new(POPUP_PAD + LABEL_W + SLIDER_DIST, y, (body_w - LABEL_W - SLIDER_DIST).max(1.0), row);
                    let sid = format!("{id}::{}", lab.to_ascii_lowercase());
                    c[i] = ui.slider_at(&sid, bar, 0.0..=1.0, c[i]);
                    y += row;
                }
                let mut new_color = Color::from(c);
                // 不显示 Alpha 滑条时不改动原 Alpha（否则会把不透明色变成透明）。
                if !alpha {
                    rgba_keep_alpha(&mut new_color, color_in);
                }

                // 可选十六进制输入
                if let Some(buf) = hex {
                    let hrect = Rect::new(POPUP_PAD, y, body_w, row);
                    let hid = format!("{id}::hex");
                    let key = ui.id_for(hid.as_str());
                    let focused = ui
                        .state()
                        .focused
                        .as_ref()
                        .is_some_and(|f| f.as_str() == key.as_str());
                    if !focused {
                        let want = color_hex(color_in, alpha);
                        if *buf != want {
                            *buf = want;
                        }
                    }
                    ui.text_input_at(&hid, hrect, buf);
                    // 只在**解析成功**时写回：打字中途（`#3a`）不闪回上一个颜色。
                    if let Some(parsed) = parse_hex(buf) {
                        let mut p = rgba(parsed);
                        if !alpha {
                            p[3] = rgba(color_in)[3];
                        }
                        new_color = Color::from(p);
                    }
                }
                color_out = new_color;

                // 面板内容尺寸（自动宽 = pw，高 = ph）。
                ui.child_rect(pw, ph, Child::Expand);
            });
        *color = color_out;

        // 点面板外（且不在色块上）→ 收起。
        if btn.down_edge() && !hit && !inside {
            open = false;
            ui.state_mut().widget(&id_for).popup_open = false;
        }
        let _ = open;
        Response { hovered: hit, ..Default::default() }
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
    fn ink_is_readable_on_both_ends() {
        assert!(luma(ink_on(Color::WHITE)) < 0.55, "白底应配黑字");
        assert!(luma(ink_on(Color::BLACK)) > 0.55, "黑底应配白字");
    }

    #[test]
    fn hiding_alpha_keeps_the_existing_transparency() {
        // 不显示 Alpha 滑条时，RGB 面板不得把半透明色变成不透明。
        let src = Color::rgba_u8(10, 20, 30, 0x40);
        let mut dst = Color::rgba_u8(200, 100, 50, 255);
        rgba_keep_alpha(&mut dst, src);
        assert_eq!(to_u8(rgba(dst)[3]), 0x40, "alpha 应保持不变");
        assert_eq!(to_u8(rgba(dst)[0]), 200, "RGB 不应被改动");
    }
}
