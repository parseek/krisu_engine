//! 输入快照：`Ui` 自持的键盘 / 鼠标状态（与输入设备解耦）。
//!
//! `Ui` 不再借用 [`rjw_keyboard::KeyboardInput`] / [`rjw_mouse::MouseInput`]：
//! [`UiInit::capture`](crate::ui::UiInit::capture) 在帧开始时把设备状态**拷贝**成
//! 快照（`KeyboardSnapshot` / `MouseSnapshot`），之后 `Ui` 只读自己的快照——
//!
//! - 录制 UI 的阶段**不依赖设备存在**（可先建 `Ui`、喂输入、最后绘制）；
//! - 不调用 `capture` = 空输入（headless：纯布局 / 纯绘制，无交互）；
//! - 设备本身仍由 `rjw_main` 每帧喂事件并在帧末 `next_frame()` 结算边沿，
//!   快照拿到的是**完整一帧**的键/鼠状态（边沿值已结算）。
//!
//! 快照的方法名与设备类型**一致**（`key` / `chars` / `ime_*` / `pos_px` / `button` /
//! `wheel`），`Ui` 内部调用点与设备路径同名同义。

use rjw_keystate::{KeyState, KEY_STATE_RELEASED};
use rjw_keyboard::key_code::key_code_index;
use rjw_keyboard::KeyboardInput;
use rjw_mouse::{MouseButton, MouseInput};
use winit::keyboard::KeyCode;

/// 键盘快照（`Ui` 自持；方法名对齐 [`KeyboardInput`]）。
#[derive(Clone, Debug)]
pub struct KeyboardSnapshot {
    /// 按键状态表：下标 = `KeyCode` 判别值（与 `rjw_keyboard` 的按键表一致），O(1) 读写。
    /// 仅槽位 0..`KEY_CODE_COUNT` 对应真实按键；未记录的键恒为 `KEY_STATE_RELEASED`。
    keys: [KeyState; 256],
    chars: Vec<char>,
    ime_commits: Vec<String>,
    ime_preedit: Option<String>,
    ime_preedit_caret: Option<usize>,
}

// `[KeyState; 256]` 无 `Default` 实现（数组的 `Default` 仅到长度 32），故手动实现。
impl Default for KeyboardSnapshot {
    /// 全表初始化为 `KEY_STATE_RELEASED`（所有键未按下）。
    fn default() -> Self {
        Self {
            keys: [KEY_STATE_RELEASED; 256],
            chars: Vec::new(),
            ime_commits: Vec::new(),
            ime_preedit: None,
            ime_preedit_caret: None,
        }
    }
}

impl KeyboardSnapshot {
    /// 从设备拷贝本帧状态（含 IME 组合 / 上屏 / 输入字符）。
    pub fn capture(kb: &KeyboardInput) -> Self {
        let mut keys = [KEY_STATE_RELEASED; 256];
        for (k, s) in kb.keys() {
            keys[key_code_index(k)] = s;
        }
        Self {
            keys,
            chars: kb.chars().to_vec(),
            ime_commits: kb.ime_commits().to_vec(),
            ime_preedit: kb.ime_preedit().map(|s| s.to_owned()),
            ime_preedit_caret: kb.ime_preedit_caret(),
        }
    }

    /// 按键状态（未按下的键 = [`KeyState::default`]：全 false）。
    #[inline]
    pub fn key(&self, key_code: KeyCode) -> KeyState {
        self.keys[key_code_index(key_code)]
    }

    /// 本帧输入的字符（非 IME 路径，如英文/数字直接输入）。
    #[inline]
    pub fn chars(&self) -> &[char] {
        &self.chars
    }

    /// 本帧 IME 上屏的文本。
    #[inline]
    pub fn ime_commits(&self) -> &[String] {
        &self.ime_commits
    }

    /// 当前 IME 组合串（拼音等未上屏文本）。
    #[inline]
    pub fn ime_preedit(&self) -> Option<&str> {
        self.ime_preedit.as_deref()
    }

    /// IME 组合串内光标位置。
    #[inline]
    pub fn ime_preedit_caret(&self) -> Option<usize> {
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
            pos: (m.pos_px().x as f64, m.pos_px().y as f64),
            in_window: m.in_window(),
            left: m.button(MouseButton::Left),
            right: m.button(MouseButton::Right),
            middle: m.button(MouseButton::Middle),
            wheel: match m.wheel() {
                rjw_mouse::ScrollDelta::Line(d) => d,
                rjw_mouse::ScrollDelta::Pixel(d) => d,
            },
        }
    }

    /// 鼠标**物理屏幕坐标**（像素）。
    #[inline]
    pub fn pos_px(&self) -> (f64, f64) {
        self.pos
    }

    /// 鼠标是否在窗口内。
    #[inline]
    pub fn in_window(&self) -> bool {
        self.in_window
    }

    /// 指定按键状态。
    #[inline]
    pub fn button(&self, button: MouseButton) -> KeyState {
        match button {
            MouseButton::Left => self.left,
            MouseButton::Right => self.right,
            MouseButton::Middle => self.middle,
            _ => KeyState::default(),
        }
    }

    /// 本帧滚轮累计增量（物理像素；y 向上为正）。
    #[inline]
    pub fn wheel(&self) -> (f64, f64) {
        self.wheel
    }
}

/// 测试构造器（`KeyboardSnapshot` 字段私有，仅 `edit` 单测用）。
#[cfg(test)]
impl KeyboardSnapshot {
    /// 设置一个按键状态（其余字段默认）。
    pub(crate) fn with_key(mut self, key: KeyCode, s: KeyState) -> Self {
        self.keys[key_code_index(key)] = s;
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
