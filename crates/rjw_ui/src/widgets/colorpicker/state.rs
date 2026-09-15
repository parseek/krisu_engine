//! **取色器的全局跨帧数据**（[`ColorPickerState`]）——定义在本控件模块里
//! （而不是 `ui.rs`：它是控件层的事实，属于本模块）。
//!
//! 设计要点：**所有 `ColorPicker` 共用一份**。
//!
//! - `mode` 是用户的呈现偏好（在一个取色器里切到 HEX，另一个也该是 HEX）；
//! - `text` 是**替补输入缓冲**：调用方没传 `&mut String`（`with_hex`）时用它，
//!   于是绝大多数调用点不必自己持有 `String`；
//! - `open` 最多一个面板——共享文本缓冲必须只有一个所有者，否则两个面板会互相覆写；
//! - `hsv` 缓存（+ `hsv_src` 记录它由哪个颜色派生）：`RGB → HSV` 在 V=0 / S=0 时
//!   丢色相，缓存让"把明度拖到 0 再拖回来"的色相不跳（见 [`super::hsv`]）。

use rjw_color::Color;

use crate::id::IdAbsolute;

use super::format::ColorFormat;

/// **颜色选择器的全局跨帧数据**（所有实例共用一份，[`crate::UiState`] 持有）。
#[derive(Clone, Debug, PartialEq)]
pub struct ColorPickerState {
    /// 顶部输入框的呈现格式（全局偏好：所有取色器一致）。
    pub mode: ColorFormat,
    /// **替补输入缓冲**：调用方未传 `&mut String`（`super::ColorPicker::with_hex`）时使用。
    pub text: String,
    /// 当前展开的面板（`None` = 全收起）。**最多一个**——共享文本缓冲只有一个所有者。
    pub open: Option<IdAbsolute<'static>>,
    /// HSV 缓存（见模块文档：V=0 / S=0 时保住色相）。
    pub(crate) hsv: [f32; 3],
    /// `hsv` 由哪个颜色派生（颜色被外部改动时重新派生）。
    pub(crate) hsv_src: Color,
}

impl Default for ColorPickerState {
    fn default() -> Self {
        Self {
            mode: ColorFormat::default(),
            text: String::new(),
            open: None,
            // 黑色的 HSV 就是 [0,0,0] ⇒ 初值自洽，首帧不必特判。
            hsv: [0.0, 0.0, 0.0],
            hsv_src: Color::BLACK,
        }
    }
}

impl ColorPickerState {
    /// 指定面板是否展开。
    #[inline]
    pub fn is_open(&self, id: &IdAbsolute<'_>) -> bool {
        self.open.as_ref().is_some_and(|o| o.as_str() == id.as_str())
    }

    /// 开关指定面板（**全局唯一**：打开它即收起别的）。
    pub fn toggle(&mut self, id: &IdAbsolute<'_>) {
        if self.is_open(id) {
            self.open = None;
        } else {
            self.open = Some(id.to_static());
        }
    }

    /// 若展开的是指定面板则收起（点面板外 / `Esc`）。
    pub fn close(&mut self, id: &IdAbsolute<'_>) {
        if self.is_open(id) {
            self.open = None;
        }
    }

    /// 收起任何面板。
    #[inline]
    pub fn close_all(&mut self) {
        self.open = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_open_state_is_global_and_exclusive() {
        let a = IdAbsolute::owned("a".to_owned());
        let b = IdAbsolute::owned("b".to_owned());
        let mut st = ColorPickerState::default();
        assert!(!st.is_open(&a) && !st.is_open(&b));
        st.toggle(&a);
        assert!(st.is_open(&a), "打开 a");
        // 打开 b 自动收起 a（共享文本缓冲只能有一个所有者）。
        st.toggle(&b);
        assert!(st.is_open(&b) && !st.is_open(&a), "同一时刻只有一个面板");
        // 再点 b = 收起。
        st.toggle(&b);
        assert!(!st.is_open(&b) && st.open.is_none());
        // close 只作用于指定面板。
        st.toggle(&a);
        st.close(&b);
        assert!(st.is_open(&a), "close(b) 不应影响 a");
        st.close_all();
        assert!(st.open.is_none());
    }

    #[test]
    fn picker_state_default_is_u8_with_empty_buffer() {
        let st = ColorPickerState::default();
        assert_eq!(st.mode, ColorFormat::U8, "默认呈现模式 = u8");
        assert!(st.text.is_empty());
        assert!(st.open.is_none());
    }
}
