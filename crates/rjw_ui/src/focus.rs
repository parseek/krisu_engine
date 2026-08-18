//! 键盘导航：焦点链（可聚焦控件注册 → `Ui::finish` 按 Tab / 方向键遍历）。
//!
//! - **注册**：交互控件（按钮 / 勾选 / 单选 / 滑块 / 输入框 / 下拉框）录制时调用
//!   [`crate::Ui`] 内部的 `register_focus`，把自身 `(id, 窗口 z, 类型, 矩形, 裁剪)` 记入
//!   本帧焦点链（[`FocusEntry`]）；
//! - **遍历**：[`Ui::finish`] 对链按 `(win, 注册序)` 稳定排序后，用 [`focus_step`]
//!   处理 Tab / Shift+Tab / 方向键——`UiState.focused` 即当前焦点 id（跨帧持久）；
//! - **激活**：Enter / Space 由焦点控件录制时即时响应（视为点击）；滑块用左右方向键
//!   调值；下拉框展开时用上下方向键切换选项；
//! - **可视反馈**：`finish` 对焦点控件画一圈描边（[`crate::style::FocusStyle`]，
//!   `Theme::focus`），裁剪沿用控件自身裁剪（滚动容器内正确裁剪）。

use rjw_transform::Rect;

/// 可聚焦控件类型（键盘行为差异：Enter/Space 激活、方向键调值等）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusKind {
    /// 按钮（Enter / Space 激活）。
    Button,
    /// 勾选框（Enter / Space 切换）。
    Checkbox,
    /// 单选（Enter / Space 选中）。
    Radio,
    /// 滑块（左右方向键调值；不参与 Enter/Space 激活）。
    Slider,
    /// 文本输入框（打字路径自处理；不参与 Enter/Space 激活）。
    TextInput,
    /// 下拉框（Enter / Space 展开/收起；展开时上下方向键切换选项）。
    Combo,
}

/// 本帧注册的可聚焦控件（焦点链条目）。
#[derive(Clone, Debug)]
pub struct FocusEntry {
    pub id: String,
    /// 所在窗口 z（非窗口内容 = 0；焦点链按 (win, 注册序) 排序）。
    pub win: u32,
    pub kind: FocusKind,
    /// 容器嵌套深度（焦点描边命令排序用，与控件命令同深度）。
    pub depth: u32,
    /// **绝对逻辑矩形**（焦点描边绘制位置）。
    pub rect: Rect,
    /// 录制时的裁剪区（**绝对逻辑**；焦点描边同样裁剪，滚动容器内正确）。
    pub clip: Option<Rect>,
}

/// 焦点链移动：从当前焦点（`current`，可为 `None`）按 `dir`（`+1` 下一个 / `-1`
/// 上一个）取下一个焦点 id。
///
/// - `chain` 需已按 `(win, 注册序)` 排序；
/// - `current` 在链中 → 取相邻条目（越界环绕）；
/// - `current` 为 `None` 或不在链中（焦点控件本帧未录制）→ `dir > 0` 从链首开始、
///   `dir < 0` 从链尾开始；
/// - 链空返回 `None`。
pub fn focus_step<'a>(
    chain: &[&'a FocusEntry],
    current: Option<&str>,
    dir: i32,
) -> Option<String> {
    if chain.is_empty() {
        return None;
    }
    let n = chain.len();
    match current.and_then(|id| chain.iter().position(|e| e.id == id)) {
        Some(i) => {
            let ni = ((i as i64 + dir as i64).rem_euclid(n as i64)) as usize;
            Some(chain[ni].id.clone())
        }
        None => {
            let idx = if dir >= 0 { 0 } else { n - 1 };
            Some(chain[idx].id.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, win: u32) -> FocusEntry {
        FocusEntry {
            id: id.to_owned(),
            win,
            kind: FocusKind::Button,
            depth: 0,
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            clip: None,
        }
    }

    fn refs(chain: &[FocusEntry]) -> Vec<&FocusEntry> {
        chain.iter().collect()
    }

    #[test]
    fn focus_step_forward_wraps() {
        let chain = [entry("a", 0), entry("b", 0), entry("c", 0)];
        let c = refs(&chain);
        assert_eq!(focus_step(&c, Some("a"), 1).as_deref(), Some("b"));
        assert_eq!(focus_step(&c, Some("b"), 1).as_deref(), Some("c"));
        assert_eq!(focus_step(&c, Some("c"), 1).as_deref(), Some("a"), "末尾环绕到链首");
    }

    #[test]
    fn focus_step_backward_wraps() {
        let chain = [entry("a", 0), entry("b", 0), entry("c", 0)];
        let c = refs(&chain);
        assert_eq!(focus_step(&c, Some("c"), -1).as_deref(), Some("b"));
        assert_eq!(focus_step(&c, Some("a"), -1).as_deref(), Some("c"), "链首反向环绕到链尾");
    }

    #[test]
    fn focus_step_no_current_starts_at_ends() {
        let chain = [entry("a", 0), entry("b", 0)];
        let c = refs(&chain);
        assert_eq!(focus_step(&c, None, 1).as_deref(), Some("a"), "无焦点向前从链首开始");
        assert_eq!(focus_step(&c, None, -1).as_deref(), Some("b"), "无焦点向后从链尾开始");
    }

    #[test]
    fn focus_step_current_missing_restarts() {
        let chain = [entry("a", 0), entry("b", 0)];
        let c = refs(&chain);
        // 焦点控件本帧未录制（如所在窗口关闭）→ 重新开始
        assert_eq!(focus_step(&c, Some("gone"), 1).as_deref(), Some("a"));
        assert_eq!(focus_step(&c, Some("gone"), -1).as_deref(), Some("b"));
    }

    #[test]
    fn focus_step_empty_chain() {
        let chain: [FocusEntry; 0] = [];
        let c = refs(&chain);
        assert_eq!(focus_step(&c, None, 1), None);
        assert_eq!(focus_step(&c, Some("x"), 1), None);
    }

    #[test]
    fn stable_sort_by_win_preserves_registration_order() {
        // 注册序：win0 a, win1 b, win0 c → 排序后 win0 内 a 先于 c（稳定）。
        let mut chain = vec![entry("a", 0), entry("b", 1), entry("c", 0)];
        chain.sort_by_key(|e| e.win);
        let ids: Vec<&str> = chain.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["a", "c", "b"]);
    }
}
