//! 文本编辑纯逻辑（无 GPU，可单测）：行/列换算、光标跨行移动、选择范围、滚动跟随。
//!
//! 供 [`crate::Ui::text_input_at`]（单行）与 [`crate::Ui::text_area_at`]（多行）共用。

/// char 索引 → `(行, 列)`：行 = 前面 `\n` 的数量，列 = 该行内 char 数。
pub fn line_col_of(s: &str, idx: usize) -> (usize, usize) {
    let mut line = 0usize;
    let mut col = 0usize;
    for (i, ch) in s.chars().enumerate() {
        if i >= idx {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// `(行, 列)` → **char 索引**（`line` / `col` 越界 clamp：列超行尾 → 行尾，行超文本 → 末尾）。
/// 与光标语义一致（[`WidgetState::caret`] 是 char 索引）。
pub fn index_of_line_col(s: &str, line: usize, col: usize) -> usize {
    let n = s.chars().count();
    let mut cur_line = 0usize;
    let mut line_col = 0usize; // 当前行内已累计 char 数
    for (char_idx, ch) in s.chars().enumerate() {
        if cur_line > line {
            return n;
        }
        if cur_line == line {
            // 目标行内：到达目标列 → 返回该 char 之前；行尾 '\n' → 行尾
            if line_col >= col {
                return char_idx;
            }
            if ch == '\n' {
                return char_idx;
            }
            line_col += 1;
        } else if ch == '\n' {
            cur_line += 1;
        }
    }
    n
}

/// 光标跨行移动：`delta` 行（`+1` 下 / `-1` 上），保持列（超出目标行长度则 clamp 到行尾）。
pub fn move_caret_line(s: &str, idx: usize, delta: i32) -> usize {
    if delta == 0 {
        return idx;
    }
    let (line, col) = line_col_of(s, idx);
    let target = if delta > 0 { line + delta as usize } else { line.saturating_sub((-delta) as usize) };
    index_of_line_col(s, target, col)
}

/// 选择范围：`(anchor, caret)` → `(lo, hi)`（lo < hi；无选择返回 `None`）。
pub fn sel_range(anchor: Option<usize>, caret: usize) -> Option<(usize, usize)> {
    let a = anchor?;
    let lo = a.min(caret);
    let hi = a.max(caret);
    if lo == hi {
        None
    } else {
        Some((lo, hi))
    }
}

/// 选择文本（无选择返回空串）。
pub fn selected_text(s: &str, anchor: Option<usize>, caret: usize) -> String {
    match sel_range(anchor, caret) {
        Some((lo, hi)) => {
            let lo_b = s.char_indices().nth(lo).map(|(b, _)| b).unwrap_or(s.len());
            let hi_b = s.char_indices().nth(hi).map(|(b, _)| b).unwrap_or(s.len());
            s[lo_b..hi_b].to_owned()
        }
        None => String::new(),
    }
}

/// 单行输入框**水平滚动跟随光标**：光标右侧留 `margin`（逻辑像素），光标移出右侧
/// 时文本左移；clamp 到 `[0, max(0, text_w - content_w)]`（文本短于内容区时为 0）。
pub fn scroll_follow_caret(caret_x: f32, content_w: f32, text_w: f32, margin: f32) -> f32 {
    let max_scroll = (text_w - content_w).max(0.0);
    if max_scroll <= 0.0 {
        return 0.0;
    }
    ((caret_x - content_w + margin).max(0.0)).min(max_scroll)
}

/// 删除 `[lo, hi)` 的 char 范围（越界 clamp；`lo >= hi` 无操作）。
pub fn delete_range(s: &mut String, lo: usize, hi: usize) {
    let mut chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let lo = lo.min(n);
    let hi = hi.min(n);
    if lo < hi {
        chars.drain(lo..hi);
        *s = chars.into_iter().collect();
    }
}

/// 在 char 索引 `caret` 处插入整段文本（粘贴 / IME 上屏多字符）。
pub fn insert_str_at(s: &mut String, caret: usize, text: &str) {
    let mut chars: Vec<char> = s.chars().collect();
    let idx = caret.min(chars.len());
    for (k, ch) in text.chars().enumerate() {
        chars.insert(idx + k, ch);
    }
    *s = chars.into_iter().collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_roundtrip() {
        let s = "ab\ncd\nef";
        assert_eq!(line_col_of(s, 0), (0, 0));
        assert_eq!(line_col_of(s, 2), (0, 2));
        assert_eq!(line_col_of(s, 3), (1, 0), "换行符后 = 第 1 行 0 列");
        assert_eq!(line_col_of(s, 4), (1, 1));
        assert_eq!(line_col_of(s, 7), (2, 1));
        assert_eq!(line_col_of(s, 8), (2, 2));
    }

    #[test]
    fn index_of_line_col_basic() {
        let s = "ab\ncd\nef";
        assert_eq!(index_of_line_col(s, 0, 0), 0);
        assert_eq!(index_of_line_col(s, 0, 2), 2);
        assert_eq!(index_of_line_col(s, 1, 0), 3);
        assert_eq!(index_of_line_col(s, 2, 2), 8);
        // 列越界 → 行尾（该行 '\n' 前）
        assert_eq!(index_of_line_col(s, 1, 99), 5, "第 1 行末尾 = 'd' 后");
        // 行越界 → 文本末尾
        assert_eq!(index_of_line_col(s, 9, 0), 8);
        // 中文（多字节）按 char 计数
        let c = "你好\n世界";
        assert_eq!(line_col_of(c, 2), (0, 2));
        assert_eq!(line_col_of(c, 4), (1, 1));
        assert_eq!(index_of_line_col(c, 1, 1), 4, "第 1 行 1 列 = '世' 后");
        assert_eq!(index_of_line_col(c, 0, 2), 2, "第 0 行 2 列 = 换行前");
        assert_eq!(line_col_of(c, 3), (1, 0), "换行符后首字");
    }

    #[test]
    fn move_caret_line_keeps_column() {
        let s = "ab\ncd\nef";
        assert_eq!(move_caret_line(s, 1, 1), 4, "(0,1) → 下 → (1,1)");
        assert_eq!(move_caret_line(s, 4, -1), 1, "(1,1) → 上 → (0,1)");
        assert_eq!(move_caret_line(s, 2, 1), 5, "(0,2) → 下 → (1,2)");
        assert_eq!(move_caret_line(s, 4, 1), 7, "(1,1) → 下 → (2,1)");
        // 列超出目标行 → clamp 到行尾
        assert_eq!(move_caret_line(s, 5, -1), 2, "(1,2) → 上 → (0,2) 行尾");
        // 首行向上 / 末行向下 clamp（caret 可停在文本末尾 = 8）
        assert_eq!(move_caret_line(s, 1, -1), 1);
        assert_eq!(move_caret_line(s, 7, 1), 8);
    }

    #[test]
    fn selection_range_and_text() {
        assert_eq!(sel_range(None, 3), None);
        assert_eq!(sel_range(Some(3), 3), None, "anchor == caret 无选择");
        assert_eq!(sel_range(Some(2), 5), Some((2, 5)));
        assert_eq!(sel_range(Some(5), 2), Some((2, 5)), "反向选择归一化");
        assert_eq!(selected_text("abcdef", Some(1), 4), "bcd");
        assert_eq!(selected_text("abcdef", None, 2), "");
        assert_eq!(selected_text("你好世界", Some(1), 3), "好世");
    }

    #[test]
    fn scroll_follow_clamps() {
        assert_eq!(scroll_follow_caret(10.0, 100.0, 50.0, 8.0), 0.0, "文本短于内容区");
        assert_eq!(scroll_follow_caret(10.0, 100.0, 200.0, 8.0), 0.0, "光标在可见区");
        assert_eq!(scroll_follow_caret(91.0, 100.0, 200.0, 8.0), 0.0, "光标右侧仍 ≥ 8px");
        assert_eq!(scroll_follow_caret(93.0, 100.0, 200.0, 8.0), 1.0, "右侧 < 8px 开始跟随");
        assert_eq!(scroll_follow_caret(120.0, 100.0, 200.0, 8.0), 28.0);
        assert_eq!(scroll_follow_caret(260.0, 100.0, 200.0, 8.0), 100.0, "clamp 到最大滚动");
    }

    #[test]
    fn delete_and_insert_ranges() {
        let mut s = String::from("abcdef");
        delete_range(&mut s, 1, 4);
        assert_eq!(s, "aef", "删除 [1,4)");
        let mut s = String::from("中文测试");
        delete_range(&mut s, 1, 3);
        assert_eq!(s, "中试");
        delete_range(&mut s, 99, 100);
        assert_eq!(s, "中试", "越界 clamp 无操作");
        let mut s = String::from("ab");
        insert_str_at(&mut s, 1, "XY");
        assert_eq!(s, "aXYb");
        insert_str_at(&mut s, 99, "!");
        assert_eq!(s, "aXYb!", "caret 越界 → 末尾");
        let mut s = String::from("你好");
        insert_str_at(&mut s, 1, "啊");
        assert_eq!(s, "你啊好");
    }
}
