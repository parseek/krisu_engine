/// 将十六进制字符转为数值 (0-15)
const fn hex_val(c: u8) -> u8 {
    if c >= b'0' && c <= b'9' {
        c - b'0'
    } else if c >= b'A' && c <= b'F' {
        c - b'A' + 10
    } else if c >= b'a' && c <= b'f' {
        c - b'a' + 10
    } else {
        panic!("invalid hex digit")
    }
}

/// 从字节切片中解析两个十六进制字符为一个字节
const fn parse_byte(bytes: &[u8], start: usize) -> u8 {
    let hi = hex_val(bytes[start]);
    let lo = hex_val(bytes[start + 1]);
    hi * 16 + lo
}

/// 解析 "#RRGGBB" 格式，返回 (R, G, B)
pub const fn hex_rgb(s: &str) -> (u8, u8, u8) {
    let bytes = s.as_bytes();
    // 格式检查（长度 7，以 '#' 开头）
    if bytes.len() != 7 || bytes[0] != b'#' {
        panic!("invalid RGB color format");
    }
    let r = parse_byte(bytes, 1);
    let g = parse_byte(bytes, 3);
    let b = parse_byte(bytes, 5);
    (r, g, b)
}

/// 解析 "#RRGGBBAA" 格式，返回 (R, G, B, A)
pub const fn hex_rgba(s: &str) -> (u8, u8, u8, u8) {
    let bytes = s.as_bytes();
    if bytes.len() != 9 || bytes[0] != b'#' {
        panic!("invalid RGBA color format");
    }
    let r = parse_byte(bytes, 1);
    let g = parse_byte(bytes, 3);
    let b = parse_byte(bytes, 5);
    let a = parse_byte(bytes, 7);
    (r, g, b, a)
}

use super::{Color, ColorF64};

impl Color {
    /// 从 "#RRGGBB" 格式的十六进制字符串创建 Color
    pub const fn from_hex_rgb(s: &str) -> Self {
        let (r, g, b) = hex_rgb(s);
        Self::rgb_u8(r, g, b)
    }

    /// 从 "#RRGGBBAA" 格式的十六进制字符串创建 Color
    pub const fn from_hex_rgba(s: &str) -> Self {
        let (r, g, b, a) = hex_rgba(s);
        Self::rgba_u8(r, g, b, a)
    }
}

impl ColorF64 {
    /// 从 "#RRGGBB" 格式的十六进制字符串创建 ColorF64
    pub const fn from_hex_rgb(s: &str) -> Self {
        let (r, g, b) = hex_rgb(s);
        Self::rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0)
    }

    /// 从 "#RRGGBBAA" 格式的十六进制字符串创建 ColorF64
    pub const fn from_hex_rgba(s: &str) -> Self {
        let (r, g, b, a) = hex_rgba(s);
        Self::rgba(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a as f64 / 255.0)
    }
}