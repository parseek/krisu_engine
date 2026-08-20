//! 输入快照：`Ui` 自持的键盘 / 鼠标状态（与输入设备解耦）。
//!
//! `Ui` 不再借用 [`rjw_keyboard::KeyboardInput`] / [`rjw_mouse::MouseInput`]：
//! [`UiInit::capture`](crate::ui::UiInit::capture) 在帧开始时把设备状态**拷贝**成
//! 快照（`KeyboardSnapshot` / `MouseSnapshot`），之后 `Ui` 只读自己的快照——
//!
//! - 录制 UI 的阶段**不依赖设备存在**（可先建 `Ui`、喂输入、最后绘制）；
//! - 不调用 `capture` = 空输入（headless：纯布局 / 纯绘制，无交互）；
//! - 设备本身仍由 `rjw_main` 每帧喂事件并在帧末 `end_frame()` 结算边沿，
//!   快照拿到的是**完整一帧**的键/鼠状态（边沿值已结算）。
//!
//! 快照的方法名与设备类型对齐，`Ui` 内部调用点无需改动。

use std::collections::HashMap;

use rjw_keystate::KeyState;
use rjw_keyboard::KeyboardInput;
use rjw_mouse::{MouseButton, MouseInput};
use winit::keyboard::KeyCode;

/// 键盘快照（`Ui` 自持；方法名对齐 [`KeyboardInput`]）。
#[derive(Clone, Debug, Default)]
pub struct KeyboardSnapshot {
    keys: HashMap<KeyCode, KeyState>,
    chars: Vec<char>,
    ime_commits: Vec<String>,
    ime_preedit: Option<String>,
    ime_preedit_caret: Option<usize>,
}

impl KeyboardSnapshot {
    /// 从设备拷贝本帧状态（含 IME 组合 / 上屏 / 输入字符）。
    pub fn capture(kb: &KeyboardInput) -> Self {
        Self {
            keys: kb.get_keys_iter().map(|(k, s)| (k, s)).collect(),
            chars: kb.get_chars().to_vec(),
            ime_commits: kb.get_ime_commits().to_vec(),
            ime_preedit: kb.get_ime_preedit().map(|s| s.to_owned()),
            ime_preedit_caret: kb.get_ime_preedit_caret(),
        }
    }

    /// 按键状态（未按下的键 = [`KeyState::default`]：全 false）。
    #[inline]
    pub fn get(&self, key_code: KeyCode) -> KeyState {
        self.keys.get(&key_code).copied().unwrap_or_default()
    }

    /// 本帧输入的字符（非 IME 路径，如英文/数字直接输入）。
    #[inline]
    pub fn get_chars(&self) -> &[char] {
        &self.chars
    }

    /// 本帧 IME 上屏的文本。
    #[inline]
    pub fn get_ime_commits(&self) -> &[String] {
        &self.ime_commits
    }

    /// 当前 IME 组合串（拼音等未上屏文本）。
    #[inline]
    pub fn get_ime_preedit(&self) -> Option<&str> {
        self.ime_preedit.as_deref()
    }

    /// IME 组合串内光标位置。
    #[inline]
    pub fn get_ime_preedit_caret(&self) -> Option<usize> {
        self.ime_preedit_caret
    }
}

/// 鼠标快照（`Ui` 自持；方法名对齐 [`MouseInput`]）。
#[derive(Clone, Debug, Default)]
pub struct MouseSnapshot {
    pos: (f64, f64),
    in_window: bool,
    left: KeyState,
    right: KeyState,
    middle: KeyState,
    wheel: (f64, f64),
}

impl MouseSnapshot {
    /// 从设备拷贝本帧状态（位置为物理屏幕坐标；按钮含本帧边沿）。
    pub fn capture(m: &MouseInput) -> Self {
        Self {
            pos: m.get_mouse_position(),
            in_window: m.in_window(),
            left: m.get(MouseButton::Left),
            right: m.get(MouseButton::Right),
            middle: m.get(MouseButton::Middle),
            wheel: m.get_mouse_wheel_delta(),
        }
    }

    /// 鼠标物理屏幕坐标。
    #[inline]
    pub fn get_mouse_position(&self) -> (f64, f64) {
        self.pos
    }

    /// 鼠标是否在窗口内。
    #[inline]
    pub fn in_window(&self) -> bool {
        self.in_window
    }

    /// 指定按键状态。
    #[inline]
    pub fn get(&self, button: MouseButton) -> KeyState {
        match button {
            MouseButton::Left => self.left,
            MouseButton::Right => self.right,
            MouseButton::Middle => self.middle,
            _ => KeyState::default(),
        }
    }

    /// 本帧滚轮累计增量（物理像素；y 向上为正）。
    #[inline]
    pub fn get_mouse_wheel_delta(&self) -> (f64, f64) {
        self.wheel
    }
}

/// 测试构造器（`KeyboardSnapshot` 字段私有，仅 `edit` 单测用）。
#[cfg(test)]
impl KeyboardSnapshot {
    /// 设置一个按键状态（其余字段默认）。
    pub(crate) fn with_key(mut self, key: KeyCode, s: KeyState) -> Self {
        self.keys.insert(key, s);
        self
    }
    /// 设置本帧输入字符。
    pub(crate) fn with_chars(mut self, chars: Vec<char>) -> Self {
        self.chars = chars;
        self
    }
    /// 设置本帧 IME 上屏文本。
    pub(crate) fn with_commits(mut self, commits: Vec<String>) -> Self {
        self.ime_commits = commits;
        self
    }
}
