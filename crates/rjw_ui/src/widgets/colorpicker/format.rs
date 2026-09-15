//! **颜色文本格式**（呈现 + 解析）与颜色分量工具——取色器面板的"文本 ↔ 颜色"层。
//!
//! 三种呈现格式（[`ColorFormat`]）：
//!
//! | 模式 | 呈现 | 例子 |
//! |---|---|---|
//! | [`ColorFormat::U8`] | 十进制整数 `0..=255` | `255, 0, 0` |
//! | [`ColorFormat::Hex`] | 十六进制（CSS 3/4/6/8 位） | `#FF00AA` / `#F0A` |
//! | [`ColorFormat::F`] | 归一化浮点（两位小数） | `1.00, 0.00, 0.67` |
//!
//! 输入侧一律走 [`parse_color`]（**自动识别格式**：先按当前模式，失败再试另两种），
//! 于是"在 u8 模式里粘贴 `#ff00aa`"也能work。识别不了返回 `None`——调用方**不动颜色**，
//! 只把输入框标成非法（取色器据此显示警告按钮并把文本恢复成有效值）。
//!
//! 本文件只做纯文本 ↔ `Color` 的换算：无 `Ui`、无 GPU、可单测。

use rjw_color::Color;

/// 顶部输入框 / 通道数值框的**呈现格式**。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorFormat {
    /// 十进制整数 `0..=255`（默认）。
    #[default]
    U8,
    /// 十六进制 `#RRGGBB` / `#RRGGBBAA`。
    Hex,
    /// 归一化浮点 `0..=1`（两位小数）。
    F,
}

impl ColorFormat {
    /// 全部模式（面板上的按钮顺序）。
    pub const ALL: [ColorFormat; 3] = [ColorFormat::U8, ColorFormat::Hex, ColorFormat::F];

    /// 按钮标签 / 文本前缀（`u8` / `HEX` / `F`）。
    #[inline]
    pub fn label(self) -> &'static str {
        match self {
            ColorFormat::U8 => "u8",
            ColorFormat::Hex => "HEX",
            ColorFormat::F => "F",
        }
    }

    /// 通道数值框是否按 `0..=255` 整数呈现（`u8` / `HEX`；`F` 为 `0..=1`）。
    #[inline]
    pub fn integer_channels(self) -> bool {
        matches!(self, ColorFormat::U8 | ColorFormat::Hex)
    }
}

// ─── 颜色分量工具 ───────────────────────────────────────────────

/// 颜色分量 → `0..=1` 的 `[f32; 4]`。
#[inline]
pub fn rgba(c: Color) -> [f32; 4] {
    c.into()
}

/// `0..=1` 分量 → `u8`。
#[inline]
pub fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
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

/// 把 `src` 的 alpha 写进 `dst`（未开 Alpha 行时保持原透明度）。
#[inline]
pub fn rgba_keep_alpha(dst: &mut Color, src: Color) {
    let mut d = rgba(*dst);
    d[3] = rgba(src)[3];
    *dst = Color::from(d);
}

// ─── 呈现 ───────────────────────────────────────────────────────

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

/// 按 `u8` 呈现：`255, 0, 0`（`with_alpha` 时 `255, 0, 0, 128`）。
pub fn format_u8(c: Color, with_alpha: bool) -> String {
    let a = rgba(c);
    if with_alpha {
        format!(
            "{}, {}, {}, {}",
            to_u8(a[0]),
            to_u8(a[1]),
            to_u8(a[2]),
            to_u8(a[3])
        )
    } else {
        format!("{}, {}, {}", to_u8(a[0]), to_u8(a[1]), to_u8(a[2]))
    }
}

/// 按 `F`（归一化浮点）呈现：`1.00, 0.00, 0.00`（`with_alpha` 时追加 `0.50`）。
pub fn format_f(c: Color, with_alpha: bool) -> String {
    let a = rgba(c);
    if with_alpha {
        format!("{:.2}, {:.2}, {:.2}, {:.2}", a[0], a[1], a[2], a[3])
    } else {
        format!("{:.2}, {:.2}, {:.2}", a[0], a[1], a[2])
    }
}

/// 按指定模式呈现（[`ColorFormat::Hex`] 复用 [`color_hex`]）。
pub fn format_color(c: Color, mode: ColorFormat, with_alpha: bool) -> String {
    match mode {
        ColorFormat::U8 => format_u8(c, with_alpha),
        ColorFormat::Hex => color_hex(c, with_alpha),
        ColorFormat::F => format_f(c, with_alpha),
    }
}

// ─── 解析 ───────────────────────────────────────────────────────

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

/// 按分隔符切分（`,` `;` `/` 空白；连续分隔符视为一个）。
fn split_parts(s: &str) -> Vec<&str> {
    s.split(|c: char| c == ',' || c == ';' || c == '/' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect()
}

/// 按 `u8` 解析 3/4 段（全 0..=255 整数）。
fn parse_u8_parts(parts: &[&str]) -> Option<Color> {
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let mut v = [255u8; 4];
    for (i, p) in parts.iter().enumerate() {
        let n: u32 = p.parse().ok()?;
        if n > 255 {
            return None;
        }
        v[i] = n as u8;
    }
    Some(Color::rgba_u8(v[0], v[1], v[2], v[3]))
}

/// 按 `F` 解析 3/4 段（全 0..=1 浮点）。
fn parse_f_parts(parts: &[&str]) -> Option<Color> {
    if !(3..=4).contains(&parts.len()) {
        return None;
    }
    let mut v = [1.0f32; 4];
    for (i, p) in parts.iter().enumerate() {
        let n: f32 = p.parse().ok()?;
        if !(0.0..=1.0).contains(&n) {
            return None;
        }
        v[i] = n;
    }
    Some(Color::from(v))
}

/// **自动识别格式**解析文本（先按 `mode`，失败再试另外两种）。
///
/// `None` = 无法识别为颜色（调用方**不动颜色**，取色器据此显示警告按钮）。
///
/// 规则：
/// - `#` 前缀 ⇒ 只当十六进制（用户显式表达"这是 hex"，3/4/6/8 位均可）；
/// - 有分隔符的 3/4 段 ⇒ 先按当前模式，再按另一种数值形态（全 0..255 整数 = u8，
///   全 0..1 浮点 = F）；
/// - 无分隔符 ⇒ 全十六进制字符且长度 3/4/6/8 才算 hex（**单个十进制段不判为 hex**：
///   `255` 若当 hex 就是 `#225555`，歧义太大）；
/// - 第 4 段是 alpha；是否采用由调用方决定（未开 Alpha 时用 [`rgba_keep_alpha`] 保留原值）。
pub fn parse_color(text: &str, mode: ColorFormat) -> Option<Color> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if t.starts_with('#') {
        return parse_hex(t);
    }
    let parts = split_parts(t);
    let order: [ColorFormat; 3] = match mode {
        ColorFormat::F => [ColorFormat::F, ColorFormat::U8, ColorFormat::Hex],
        _ => [ColorFormat::U8, ColorFormat::F, ColorFormat::Hex],
    };
    for m in order {
        let hit = match m {
            ColorFormat::U8 => parse_u8_parts(&parts),
            ColorFormat::F => parse_f_parts(&parts),
            ColorFormat::Hex => {
                // 无分隔符的单段才算 hex（有分隔符的分段是数值形态）；长度必须像 hex。
                if parts.len() != 1 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
                    None
                } else {
                    // ⚠ 3 位**纯十进制数字**与"十进制数值"歧义太大（`255` 不该变成 `#225555`）：
                    // 这种只认带 `#` 的写法；含 a-f 的（`f0a`）或 4/6/8 位仍可直接写。
                    let numeric = t.chars().all(|c| c.is_ascii_digit());
                    match t.len() {
                        3 if numeric => None,
                        3 | 4 | 6 | 8 => parse_hex(t),
                        _ => None,
                    }
                }
            }
        };
        if hit.is_some() {
            return hit;
        }
    }
    None
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
        // 未开 Alpha 行时，RGB 面板不得把半透明色变成不透明。
        let src = Color::rgba_u8(10, 20, 30, 0x40);
        let mut dst = Color::rgba_u8(200, 100, 50, 255);
        rgba_keep_alpha(&mut dst, src);
        assert_eq!(to_u8(rgba(dst)[3]), 0x40, "alpha 应保持不变");
        assert_eq!(to_u8(rgba(dst)[0]), 200, "RGB 不应被改动");
    }

    #[test]
    fn format_color_covers_all_three_modes() {
        let c = Color::rgba_u8(255, 0, 170, 128);
        assert_eq!(format_color(c, ColorFormat::U8, false), "255, 0, 170");
        assert_eq!(format_color(c, ColorFormat::U8, true), "255, 0, 170, 128");
        assert_eq!(format_color(c, ColorFormat::Hex, false), "#FF00AA");
        assert_eq!(format_color(c, ColorFormat::Hex, true), "#FF00AA80");
        assert_eq!(format_color(c, ColorFormat::F, false), "1.00, 0.00, 0.67");
        assert_eq!(format_color(c, ColorFormat::F, true), "1.00, 0.00, 0.67, 0.50");
    }

    #[test]
    fn format_color_round_trips_per_mode() {
        // 三模式各自往返：u8 有 1/255 量化误差，F 有两位小数误差。
        for c in [
            Color::rgba_u8(0, 0, 0, 255),
            Color::rgba_u8(255, 255, 255, 255),
            Color::rgba_u8(12, 200, 64, 255),
            Color::rgba_u8(255, 0, 170, 128),
        ] {
            for mode in ColorFormat::ALL {
                for with_alpha in [false, true] {
                    let text = format_color(c, mode, with_alpha);
                    let back = parse_color(&text, mode)
                        .unwrap_or_else(|| panic!("{mode:?} 呈现 {text:?} 应能解析回来"));
                    let (a, b) = (rgba(c), rgba(back));
                    let tol = if mode == ColorFormat::F { 0.006 } else { 0.002 };
                    for i in 0..3 {
                        assert!(
                            (a[i] - b[i]).abs() <= tol,
                            "{mode:?} 分量 {i} 往返失真：{} vs {}（{text:?}）",
                            a[i],
                            b[i]
                        );
                    }
                    if with_alpha {
                        assert!((a[3] - b[3]).abs() <= tol, "{mode:?} alpha 往返失真");
                    }
                }
            }
        }
    }

    #[test]
    fn parse_color_prefers_current_mode_then_falls_back() {
        // "1, 0, 0" 在 u8 模式是 (1,0,0)，在 F 模式是纯红 —— 模式优先，另一形态兜底。
        let u8_hit = parse_color("1, 0, 0", ColorFormat::U8).expect("u8");
        assert_eq!(to_u8(rgba(u8_hit)[0]), 1);
        let f_hit = parse_color("1, 0, 0", ColorFormat::F).expect("F");
        assert_eq!(to_u8(rgba(f_hit)[0]), 255);
        // 十六进制在**任何**模式都能识别（自动识别格式）。
        for mode in ColorFormat::ALL {
            let c = parse_color("#FF00AA", mode).expect("hex 自动识别");
            assert_eq!(color_hex(c, false), "#FF00AA", "{mode:?}");
        }
        // 当前模式优先：`1 0 0` 在 u8/HEX 下是 (1,0,0)，在 F 下是纯红。
        assert_eq!(to_u8(rgba(parse_color("1 0 0", ColorFormat::U8).unwrap())[0]), 1);
        assert_eq!(to_u8(rgba(parse_color("1 0 0", ColorFormat::Hex).unwrap())[0]), 1);
        assert_eq!(to_u8(rgba(parse_color("1 0 0", ColorFormat::F).unwrap())[0]), 255);
        // 兜底：`0.5 0 0` 在 u8 模式里不是合法整数，但 F 兜底能识别（≈127）。
        let fb = parse_color("0.5 0 0", ColorFormat::U8).expect("F 兜底");
        assert!((to_u8(rgba(fb)[0]) as i32 - 127).abs() <= 1, "应约为 127，实际 {fb:?}");
        // 无分隔符的纯十六进制（6 位）识别为 hex。
        assert_eq!(parse_color("ff00aa", ColorFormat::U8), parse_color("#ff00aa", ColorFormat::U8));
        // 含 a-f 的 3 位简写也认（`f0a`），4 位同理。
        assert_eq!(parse_color("f0a", ColorFormat::U8), parse_color("#f0a", ColorFormat::U8));
        assert_eq!(parse_color("1234", ColorFormat::U8), parse_color("#1234", ColorFormat::U8));
        // 但**纯十进制数字**的 3 段不判为 hex（避免 "255" 变成 #225555）。
        assert_eq!(parse_color("255", ColorFormat::U8), None);
        assert_eq!(parse_color("255", ColorFormat::F), None);
        // 4 段带 alpha。
        let a = parse_color("1, 0, 0, 0.5", ColorFormat::F).expect("带 alpha");
        assert!((rgba(a)[3] - 0.5).abs() < 0.01);
    }

    #[test]
    fn parse_color_rejects_partial_and_garbage() {
        for (s, m) in [
            ("", ColorFormat::U8),
            ("#", ColorFormat::Hex),
            ("255, 0", ColorFormat::U8),
            ("1,2,3,4,5", ColorFormat::F),
            ("1 0", ColorFormat::F),
            ("300, 0, 0", ColorFormat::U8),
            ("1.5, 0, 0", ColorFormat::F),
            ("hello", ColorFormat::U8),
        ] {
            assert_eq!(parse_color(s, m), None, "{s:?} / {m:?} 应判为非法");
        }
    }

    #[test]
    fn mode_labels_and_channel_precision() {
        assert_eq!(ColorFormat::ALL.map(|m| m.label()), ["u8", "HEX", "F"]);
        assert!(ColorFormat::U8.integer_channels() && ColorFormat::Hex.integer_channels());
        assert!(!ColorFormat::F.integer_channels());
        assert_eq!(ColorFormat::default(), ColorFormat::U8, "默认呈现模式 = u8");
    }
}
