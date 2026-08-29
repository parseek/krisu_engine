use winit::event::WindowEvent;
use winit::event::ElementState;
use winit::keyboard::PhysicalKey::Code;

use rjw_keystate::*;

/// 重导出键码类型（供下游 crate 使用 `KeyCode::Backspace` 等常量，无需直接依赖 winit）。
pub use winit::keyboard::KeyCode;

pub mod key_code;
use key_code::{key_code_from_index, key_code_index, KEY_CODE_COUNT};

pub struct KeyboardInput {
    // 按键状态表：下标 = `KeyCode` 判别值（见 [`key_code`] 模块），O(1) 读写。
    // 仅槽位 0..KEY_CODE_COUNT 对应真实按键；其余恒为 `KEY_STATE_RELEASED`。
    key_map: [KeyState; 256],
    /// 本帧收到的**字符输入**（来自 winit `KeyEvent.text`，已含 Shift 等组合；
    /// 多字节字符逐 `char` 收集）。`end_frame` 清空；帧内多次读取返回同一批字符。
    chars: Vec<char>,
    /// 本帧 IME **上屏**文本（如中文输入法选字后提交的完整字符串）。`end_frame` 清空。
    ime_commits: Vec<String>,
    /// 当前 IME **组合候选**串（如拼音未上屏时的候选）；`None` = 无组合。
    /// 持续保留直到上屏（Commit）或取消（Preedit 空 / Disabled）。
    ime_preedit: Option<String>,
    /// 组合候选内**光标位置**（winit `Ime::Preedit(text, Some((start, end)))` 的 `end`，
    /// 字节偏移；`None` = 光标在组合末尾）。随候选串实时更新，跨帧保留。
    ime_preedit_caret: Option<usize>,
}

// `[KeyState; 256]` 无 `Default` 实现（数组的 `Default` 仅到长度 32），故手动实现。
impl Default for KeyboardInput {
    /// 全表初始化为 `KEY_STATE_RELEASED`（所有键未按下）。
    fn default() -> Self {
        Self {
            key_map: [KEY_STATE_RELEASED; 256],
            chars: Vec::new(),
            ime_commits: Vec::new(),
            ime_preedit: None,
            ime_preedit_caret: None,
        }
    }
}

impl KeyboardInput {
    #[allow(unused)]
    pub fn get(&self, key_code: winit::keyboard::KeyCode) -> KeyState {
        self.key_map[key_code_index(key_code)]
    }
    #[allow(unused)]
    pub fn get_keys_iter(&self) -> impl Iterator<Item = (winit::keyboard::KeyCode, KeyState)> + '_ {
        // 遍历按键表的前 `KEY_CODE_COUNT` 个有效槽位，仅产出当前有状态的键
        // （状态非 `KEY_STATE_RELEASED`；边沿位也算"有状态"，不会漏掉释放边沿）。
        self.key_map
            .iter()
            .take(KEY_CODE_COUNT)
            .enumerate()
            .filter(|(_, s)| **s != KEY_STATE_RELEASED)
            .map(|(i, s)| (key_code_from_index(i), *s))
    }

    /// 本帧的字符输入（`TextInput` 等文本编辑用）。
    ///
    /// - 逐帧清空（见 [`Self::end_frame`]）；帧内多次调用返回**相同**内容。
    /// - 已处理 Shift 组合（`Shift + a` → `'A'`）；`KeyEvent.text` 为空的按键
    ///   （如 F1、方向键）不会产生字符——请用 [`Self::get`] 读 `KeyCode`。
    /// - 控制字符（退格/回车等）已被过滤——退格/回车请用 [`Self::get`] 读边沿。
    #[allow(unused)]
    pub fn get_chars(&self) -> &[char] {
        &self.chars
    }

    /// 本帧 IME 上屏文本（中文输入法选字/回车确认后的完整字符串）。
    ///
    /// 逐帧清空；每项为一个完整提交（可能含多个字符）。优先级高于
    /// [`Self::get_chars`]（IME 组合期间普通按键通常不产生字符）。
    #[allow(unused)]
    pub fn get_ime_commits(&self) -> &[String] {
        &self.ime_commits
    }

    /// 当前 IME 组合候选串（拼音未上屏时实时更新）；`None` = 无组合。
    /// 用于输入框绘制候选（内联在光标后 + 下划线）。
    #[allow(unused)]
    pub fn get_ime_preedit(&self) -> Option<&str> {
        self.ime_preedit.as_deref()
    }

    /// 组合候选内**光标位置**（字节偏移，相对候选串；`None` = 组合末尾）。
    /// 内联组合绘制时把输入光标偏移到组合内该位置（"移动光标"）。
    #[allow(unused)]
    pub fn get_ime_preedit_caret(&self) -> Option<usize> {
        self.ime_preedit_caret
    }

    /// 内部：更新单个物理键的状态机（原逻辑，拆出以便无 GPU/无窗口单元测试）。
    fn process_key(&mut self, key_code: winit::keyboard::KeyCode, state: ElementState) {
        // 按键表 O(1) 寻址；"按键常驻"语义天然成立（槽位永久存在，仅状态变化）。
        let key_state = &mut self.key_map[key_code_index(key_code)];
        let new_key_state = match state {
            ElementState::Pressed => {
                if key_state.pressed() {
                    KEY_STATE_DOWN_EDGE
                } else {
                    KEY_STATE_DOWN_TRUE_EDGE
                }
            }
            ElementState::Released => {
                if key_state.released() {
                    KEY_STATE_UP_EDGE
                } else {
                    if key_state.down_true_edge() {
                        key_state.set_sudden_up()
                    } else {
                        KEY_STATE_UP_TRUE_EDGE
                    }
                }
            }
        };
        *key_state = new_key_state;
    }

    /// 内部：按 `state` 收集字符输入（仅在按下时收集，`text` 为空则忽略）。
    ///
    /// **过滤控制字符**（`char::is_control`）：部分平台退格/回车等会给出控制字符
    /// （如 `\u{8}`），这些不应作为"输入字符"进入文本编辑——退格/回车请用
    /// [`Self::get`] 读 `KeyCode` 边沿处理。
    fn collect_chars(&mut self, state: ElementState, text: Option<&str>) {
        if state == ElementState::Pressed
            && let Some(t) = text
        {
            for c in t.chars().filter(|c| !c.is_control()) {
                self.chars.push(c);
            }
        }
    }

    /// 内部：处理 IME 事件（中文输入法等）。
    fn process_ime(&mut self, ime: &winit::event::Ime) {
        match ime {
            winit::event::Ime::Enabled => {}
            winit::event::Ime::Preedit(text, cursor) => {
                if text.is_empty() {
                    self.ime_preedit = None;
                    self.ime_preedit_caret = None;
                } else {
                    self.ime_preedit = Some(text.clone());
                    // 光标取选择区间末端（字节偏移；`None` = 组合末尾）
                    self.ime_preedit_caret = cursor.map(|(_, end)| end);
                }
            }
            winit::event::Ime::Commit(text) => {
                self.ime_commits.push(text.clone());
                self.ime_preedit = None;
                self.ime_preedit_caret = None;
            }
            winit::event::Ime::Disabled => {
                self.ime_preedit = None;
                self.ime_preedit_caret = None;
            }
        }
    }

    pub fn end_frame(&mut self) {
        for key_state in self.key_map.iter_mut() {
            // turn off the edge bit, but keep the pressed bit.
            *key_state = key_state.off_edge();
            if key_state.sudden_up() {
                *key_state = KEY_STATE_UP_TRUE_EDGE
            }
        }
        self.chars.clear();
        self.ime_commits.clear();
    }

    pub fn window_event(&mut self, event: &winit::event::WindowEvent) {
        match event {
            WindowEvent::KeyboardInput {
                device_id: _,
                event,
                is_synthetic: _,
            } => {
                // 物理键 → KeyState 状态机（仅 `Code` 可映射，与旧行为一致）。
                if let Code(key_code) = event.physical_key {
                    self.process_key(key_code, event.state);
                }
                // 字符输入：与 physical key 是否为 `Code` 无关，无条件收集
                // （`KeyEvent.text` 已含 Shift 组合，空则忽略；控制字符已过滤）。
                self.collect_chars(event.state, event.text.as_deref());
            }
            // IME（中文输入法等）：组合候选 + 上屏提交。
            WindowEvent::Ime(ime) => {
                self.process_ime(ime);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    #[test]
    fn press_release_key_state_machine() {
        let mut kb = KeyboardInput::default();
        // 按下 → down_true_edge
        kb.process_key(KeyCode::KeyA, ElementState::Pressed);
        let s = kb.get(KeyCode::KeyA);
        assert!(s.pressed() && s.down_true_edge(), "首次按下应 down_true_edge: {s}");
        // end_frame → off_edge：仍 pressed，无 edge
        kb.end_frame();
        let s = kb.get(KeyCode::KeyA);
        assert!(s.pressed() && !s.edge() && !s.down_edge(), "帧末应清除 edge: {s}");
        // 释放 → up_true_edge（此前 down_true_edge 已消费，走 sudden_up 路径）
        kb.process_key(KeyCode::KeyA, ElementState::Released);
        let s = kb.get(KeyCode::KeyA);
        assert!(s.released() && s.up_edge(), "释放应 up_edge: {s}");
        // 再按 → 再次 down_true_edge
        kb.process_key(KeyCode::KeyA, ElementState::Pressed);
        let s = kb.get(KeyCode::KeyA);
        assert!(s.pressed() && s.down_true_edge(), "再次按下应 down_true_edge: {s}");
    }

    #[test]
    fn key_table_tracks_multiple_keys_and_keeps_released() {
        // 按键表存储：多键各自独立跟踪，且释放后槽位保留（与旧 HashMap 语义一致）。
        let mut kb = KeyboardInput::default();
        assert_eq!(kb.key_map.len(), 256, "按键表长度应为 256");
        assert_eq!(kb.get(KeyCode::KeyA), KEY_STATE_RELEASED, "未按过 = released");
        kb.process_key(KeyCode::KeyA, ElementState::Pressed);
        kb.process_key(KeyCode::KeyB, ElementState::Pressed);
        // 两键都在表中（状态非 RELEASED）
        assert_eq!(kb.get_keys_iter().count(), 2, "两个键都应有状态");
        assert!(kb.get(KeyCode::KeyA).down_true_edge());
        assert!(kb.get(KeyCode::KeyB).down_true_edge());
        // 遍历产出的 (KeyCode, KeyState) 与 `get` 一致
        for (k, s) in kb.get_keys_iter() {
            assert_eq!(s, kb.get(k), "迭代器与 get 应一致");
        }
        // 未触碰的键恒为 RELEASED
        assert_eq!(kb.get(KeyCode::Space), KEY_STATE_RELEASED, "未触碰键 = released");
        // 释放后槽位不删除，仅状态变化（帧内立即释放 → sudden_up 标记，仍算 pressed）
        kb.process_key(KeyCode::KeyA, ElementState::Released);
        assert_eq!(kb.get_keys_iter().count(), 2, "释放后（边沿未清）仍应产出");
        assert!(
            kb.get(KeyCode::KeyA).sudden_up() && kb.get(KeyCode::KeyA).pressed(),
            "帧内按下-释放应打 sudden_up 标记"
        );
        // end_frame 把 sudden_up 转换为真正的释放边沿
        kb.end_frame();
        assert!(kb.get(KeyCode::KeyA).up_edge(), "end_frame 后 sudden_up 转为释放");
        // 释放后再次按下 → 再次 down_true_edge（非重复边沿）
        kb.process_key(KeyCode::KeyA, ElementState::Pressed);
        assert!(kb.get(KeyCode::KeyA).down_true_edge());
        // end_frame 对表中所有键统一清边沿
        kb.end_frame();
        assert!(kb.get(KeyCode::KeyA).pressed() && !kb.get(KeyCode::KeyA).down_edge());
        assert!(kb.get(KeyCode::KeyB).pressed() && !kb.get(KeyCode::KeyB).down_edge());
    }

    #[test]
    fn collect_chars_only_on_press() {
        let mut kb = KeyboardInput::default();
        kb.collect_chars(ElementState::Pressed, Some("A"));
        assert_eq!(kb.get_chars(), &['A']);
        // 释放不收集
        kb.collect_chars(ElementState::Released, Some("B"));
        assert_eq!(kb.get_chars(), &['A'], "释放事件不应收集字符");
    }

    #[test]
    fn collect_chars_multibyte_and_shift() {
        let mut kb = KeyboardInput::default();
        // 多字节（中文字符）逐 char 收集
        kb.collect_chars(ElementState::Pressed, Some("中"));
        // Shift 组合已由 winit 处理（text = "A"）
        kb.collect_chars(ElementState::Pressed, Some("A"));
        kb.collect_chars(ElementState::Pressed, Some("!")); // Shift+1
        assert_eq!(kb.get_chars(), &['中', 'A', '!']);
    }

    #[test]
    fn collect_chars_ignores_none_text() {
        let mut kb = KeyboardInput::default();
        // 方向键等 `text = None` 的按键不产生字符
        kb.collect_chars(ElementState::Pressed, None);
        assert!(kb.get_chars().is_empty(), "text=None 不应产生字符");
    }

    #[test]
    fn collect_chars_filters_control_characters() {
        let mut kb = KeyboardInput::default();
        // 部分平台退格给出 \u{8}、回车给出 \n 等控制字符——必须过滤，
        // 否则会作为"字符"插入文本（表现为退格/回车插入乱码或无效）。
        kb.collect_chars(ElementState::Pressed, Some("a\u{8}b"));
        kb.collect_chars(ElementState::Pressed, Some("\n"));
        kb.collect_chars(ElementState::Pressed, Some("\u{7f}"));
        assert_eq!(kb.get_chars(), &['a', 'b'], "控制字符应被过滤");
        // 正常可打印字符（含多字节、空格）保留
        kb.collect_chars(ElementState::Pressed, Some("中 文!"));
        assert_eq!(kb.get_chars(), &['a', 'b', '中', ' ', '文', '!']);
    }

    #[test]
    fn end_frame_clears_chars_and_frames_are_independent() {
        let mut kb = KeyboardInput::default();
        kb.collect_chars(ElementState::Pressed, Some("hi"));
        assert_eq!(kb.get_chars(), &['h', 'i']);
        kb.end_frame();
        assert!(kb.get_chars().is_empty(), "end_frame 应清空字符");
        // 帧内多次读取返回相同内容（借用语义）
        kb.collect_chars(ElementState::Pressed, Some("x"));
        assert_eq!(kb.get_chars(), kb.get_chars());
    }

    #[test]
    fn window_event_path_collects_text() {
        // 走真实 window_event 分支（跳过构造 KeyEvent：platform_specific 字段不可构造，
        // 因此此处直接验证 process_key + collect_chars 的组合路径）。
        let mut kb = KeyboardInput::default();
        kb.process_key(KeyCode::Enter, ElementState::Pressed);
        kb.collect_chars(ElementState::Pressed, Some("ok"));
        assert!(kb.get(KeyCode::Enter).down_true_edge());
        assert_eq!(kb.get_chars(), &['o', 'k']);
        kb.end_frame();
        assert!(kb.get(KeyCode::Enter).pressed() && !kb.get(KeyCode::Enter).down_edge());
    }

    #[test]
    fn ime_commit_and_preedit_lifecycle() {
        let mut kb = KeyboardInput::default();
        // 组合开始：preedit 实时更新（保留跨帧）
        kb.process_ime(&winit::event::Ime::Preedit("ni".into(), Some((2, 2))));
        assert_eq!(kb.get_ime_preedit(), Some("ni"));
        assert_eq!(kb.get_ime_preedit_caret(), Some(2), "组合内光标（区间末端）");
        kb.end_frame(); // preedit 不清
        assert_eq!(kb.get_ime_preedit(), Some("ni"), "组合候选跨帧保留");
        assert_eq!(kb.get_ime_preedit_caret(), Some(2), "组合内光标跨帧保留");
        // 更新候选 + 光标（winit 给 (start, end) 区间 → 取 end）
        kb.process_ime(&winit::event::Ime::Preedit("你好".into(), Some((3, 6))));
        assert_eq!(kb.get_ime_preedit(), Some("你好"));
        assert_eq!(kb.get_ime_preedit_caret(), Some(6));
        // None = 组合末尾
        kb.process_ime(&winit::event::Ime::Preedit("你好世界".into(), None));
        assert_eq!(kb.get_ime_preedit_caret(), None, "None = 光标在组合末尾");
        // 上屏：commit 入队、preedit 与组合内光标清除
        kb.process_ime(&winit::event::Ime::Commit("你好世界".into()));
        assert!(kb.get_ime_preedit().is_none(), "上屏后候选清除");
        assert!(kb.get_ime_preedit_caret().is_none(), "上屏后组合内光标清除");
        assert_eq!(kb.get_ime_commits(), &["你好世界".to_owned()]);
        // end_frame 清空 commits（preedit 已空）
        kb.end_frame();
        assert!(kb.get_ime_commits().is_empty(), "end_frame 清空上屏队列");
    }

    #[test]
    fn ime_preedit_cancel_via_empty_or_disabled() {
        let mut kb = KeyboardInput::default();
        kb.process_ime(&winit::event::Ime::Preedit("ab".into(), None));
        assert_eq!(kb.get_ime_preedit(), Some("ab"));
        // Preedit 空 → 取消组合
        kb.process_ime(&winit::event::Ime::Preedit(String::new(), None));
        assert!(kb.get_ime_preedit().is_none());
        // Disabled → 清除
        kb.process_ime(&winit::event::Ime::Preedit("cd".into(), None));
        kb.process_ime(&winit::event::Ime::Disabled);
        assert!(kb.get_ime_preedit().is_none());
        // Enabled 不清除（无状态变化）
        kb.process_ime(&winit::event::Ime::Enabled);
        assert!(kb.get_ime_preedit().is_none());
    }
}
