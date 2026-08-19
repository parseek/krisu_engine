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

/// 单行输入框**水平滚动跟随光标**：把**当前滚动**投影进"光标可见区间"
/// `[caret_x - content_w + margin, caret_x]`（右缘留 `margin` 逻辑像素）。
///
/// - 光标仍在可视区内 → 视图**不动**（←/→ 在区内移动不抖动）；
/// - 光标走出**右缘**（右侧不足 `margin`）→ 左移让光标回到右缘内；
/// - 光标走出**左缘** → 右移让光标贴住左缘（如视图 KLMN、光标在 M-N，按 ← 到
///   K-L 仍在可视区 → 视图不动；再 ← 到 J-K 走出左缘 → 视图才左移）。
///
/// `current`：当前滚动（跨帧持久）；`caret_x`：光标 x（前缀宽度，逻辑像素）；
/// 结果 clamp 到 `[0, max(0, text_w - content_w)]`。
pub fn scroll_follow_caret(
    current: f32,
    caret_x: f32,
    content_w: f32,
    text_w: f32,
    margin: f32,
) -> f32 {
    let max_scroll = (text_w - content_w).max(0.0);
    if max_scroll <= 0.0 {
        return 0.0;
    }
    let lo = ((caret_x - content_w + margin).max(0.0)).min(max_scroll);
    let hi = caret_x.min(max_scroll);
    if lo <= hi {
        current.clamp(lo, hi)
    } else {
        hi // 内容区极窄（< margin）等异常：取右边界
    }
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

/// char 索引 → 字节偏移（越界 → 末尾字节）。
pub fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(s.len())
}

/// 字节偏移 → char 索引（`byte` 需落在 char 边界；越界 clamp）。
pub fn byte_to_char(s: &str, byte: usize) -> usize {
    s[..byte.min(s.len())].chars().count()
}

/// 按前缀宽度把点击 x（相对文本左缘）映射为最近的光标 char 索引。
///
/// `width_of(k)` = 前 `k` 个字符的总宽度（单调不减）。二分找第一个
/// `width_of(k) >= cx` 的 k，再与 `k-1` 比较取更近者（纯函数，可单测）。
pub fn caret_index_by_width(n: usize, cx: f32, mut width_of: impl FnMut(usize) -> f32) -> usize {
    if n == 0 {
        return 0;
    }
    let mut lo = 1usize;
    let mut hi = n;
    let mut k = n; // 默认：点击在文本末尾之后 → 光标在末尾
    while lo <= hi {
        let mid = (lo + hi) / 2;
        if width_of(mid) >= cx {
            k = mid;
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
    }
    let w_k = width_of(k);
    let w_prev = if k > 0 { width_of(k - 1) } else { 0.0 };
    if (w_k - cx).abs() < (cx - w_prev).abs() {
        k
    } else {
        k - 1
    }
}

/// 字节偏移 → **视觉行**索引（自动换行后，与显示一致）。
///
/// 归属规则（半开区间 + 边界修正）：
/// - `byte ∈ [start, end)` → 本行；
/// - `byte == end` 且下一行 `start > end`（**换行间隙**：行尾 / `'\n'` 前 / 文本末尾）
///   → 归**本行**（光标在行尾）；
/// - `byte == end` 且下一行 `start == end`（**自动换行边界**：如 "…LK…" 中 L 后 = K 前）
///   → 归**下一行**（视觉行行首）——否则光标/↑↓ 会把边界字节算到上一行末尾
///   （"按↓到 L 后（上一行末尾）而不是 K 前（换行后的下一行行首）"）。
pub fn vline_of_byte(vlines: &[rjw_text::VisualLine], byte: usize) -> usize {
    for (i, l) in vlines.iter().enumerate() {
        if byte >= l.byte_start && byte < l.byte_end {
            return i;
        }
        // 行尾且不是下一行行首（换行间隙 / 文本末尾）→ 本行
        if byte == l.byte_end
            && (i + 1 == vlines.len() || vlines[i + 1].byte_start > l.byte_end)
        {
            return i;
        }
    }
    vlines.len().saturating_sub(1)
}

/// 多行文本框点击（可视区局部坐标 + 垂直滚动）→ 光标 char 索引。
///
/// 按**视觉行**（自动换行后）定位，与显示一致：
/// - `vlines`：[`rjw_text::Text::visual_lines`] 输出（每行含**全文字节范围**）；
/// - `row`：**文本坐标**行号 = (视口 y + 垂直滚动) / 行高（向下取整）——长内容滚动后
///   点击必须**加回滚动偏移**，否则会定位到文本前部、光标行随即被滚动跟随拉回视口
///   顶部（"自动换行后的行鼠标无法定位"）；
/// - `cx`：行内 x（逻辑像素，相对内容区左缘）；
/// - `width_of(text)`：测量文本前缀宽度（单调不减，二分用）。
pub fn caret_at_visual_click(
    value: &str,
    vlines: &[rjw_text::VisualLine],
    row: usize,
    cx: f32,
    mut width_of: impl FnMut(&str) -> f32,
) -> usize {
    if vlines.is_empty() {
        return value.chars().count(); // 空文本：点击 → 末尾（= 0）
    }
    let li = row.min(vlines.len().saturating_sub(1));
    let line = &vlines[li];
    let ls = line.byte_start.min(value.len());
    let le = line.byte_end.min(value.len());
    let ltxt = &value[ls..le];
    let chars: Vec<char> = ltxt.chars().collect();
    let n = chars.len();
    let col = caret_index_by_width(n, cx, |k| width_of(&chars[..k].iter().collect::<String>()));
    let col_byte = char_to_byte(ltxt, col);
    byte_to_char(value, ls + col_byte)
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
        assert_eq!(scroll_follow_caret(0.0, 10.0, 100.0, 50.0, 8.0), 0.0, "文本短于内容区");
        assert_eq!(scroll_follow_caret(0.0, 10.0, 100.0, 200.0, 8.0), 0.0, "光标在可见区");
        assert_eq!(scroll_follow_caret(0.0, 91.0, 100.0, 200.0, 8.0), 0.0, "光标右侧仍 ≥ 8px");
        assert_eq!(scroll_follow_caret(0.0, 93.0, 100.0, 200.0, 8.0), 1.0, "右侧 < 8px 开始跟随");
        assert_eq!(scroll_follow_caret(0.0, 120.0, 100.0, 200.0, 8.0), 28.0);
        assert_eq!(scroll_follow_caret(0.0, 260.0, 100.0, 200.0, 8.0), 100.0, "clamp 到最大滚动");
        // 左缘跟随（回归）：ABCDEFGHIJKLMN（14 字符 ×8px = 112），视口 4 字符（32px），
        // 滚动 80 → 视图 KLMN、光标在 M-N（96px）。按 ← 到 K-L（80px）仍在可视区
        // （区间 [56,80]）→ 视图不动；再 ← 到 J-K（72px）走出左缘 → 视图才左移。
        assert_eq!(scroll_follow_caret(80.0, 96.0, 32.0, 112.0, 8.0), 80.0, "光标 M-N 视图不动");
        assert_eq!(scroll_follow_caret(80.0, 88.0, 32.0, 112.0, 8.0), 80.0, "光标 L-M 视图不动");
        assert_eq!(scroll_follow_caret(80.0, 80.0, 32.0, 112.0, 8.0), 80.0, "光标 K-L 仍在左缘");
        assert_eq!(scroll_follow_caret(80.0, 72.0, 32.0, 112.0, 8.0), 72.0, "光标 J-K 走出左缘 → 视图左移");
        // 右缘：光标走到文本末尾后 → 视图已贴末尾（clamp 到 max_scroll=80，KLMN 全显、光标在右缘）
        assert_eq!(scroll_follow_caret(80.0, 112.0, 32.0, 112.0, 8.0), 80.0, "末尾 → 已贴最大滚动");
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

    #[test]
    fn char_byte_roundtrip() {
        let s = "你好ab";
        assert_eq!(char_to_byte(s, 0), 0);
        assert_eq!(char_to_byte(s, 1), 3);
        assert_eq!(char_to_byte(s, 3), 7);
        assert_eq!(char_to_byte(s, 99), 8, "越界 → 文本末尾字节（len）");
        assert_eq!(byte_to_char(s, 0), 0);
        assert_eq!(byte_to_char(s, 3), 1);
        assert_eq!(byte_to_char(s, 7), 3);
        assert_eq!(byte_to_char(s, 999), 4);
    }

    #[test]
    fn caret_index_by_width_nearest_boundary() {
        let w = |k: usize| k as f32 * 8.0; // 等宽 8px/字符
        assert_eq!(caret_index_by_width(5, 0.0, w), 0, "点击最左 → 行首");
        assert_eq!(caret_index_by_width(5, 8.0, w), 1);
        assert_eq!(caret_index_by_width(5, 12.0, w), 1, "12px 距第 1 边界 4px、距第 2 边界 4px → 取更近者（第 1）");
        assert_eq!(caret_index_by_width(5, 20.0, w), 2);
        assert_eq!(caret_index_by_width(5, 999.0, w), 5, "点击在末尾之后 → 末尾");
        assert_eq!(caret_index_by_width(0, 10.0, w), 0, "空文本 → 0");
        // 混合宽度（模拟中英文不等宽）：前缀宽度单调不减
        let mixed = |k: usize| [0.0, 16.0, 24.0, 32.0, 48.0][k]; // 中,英,英,中
        assert_eq!(caret_index_by_width(4, 21.0, mixed), 2, "21px 在 [16,24) → 距第 2 边界 3px < 5px → 2");
        assert_eq!(caret_index_by_width(4, 44.0, mixed), 4, "44px 距第 4 边界 4px < 12px → 4");
        assert_eq!(caret_index_by_width(4, 20.0, mixed), 1, "20px 等距（|24-20|==|20-16|）→ 取 k-1 = 1");
        // 回归：混合中英文（字宽不同）时用等比估算会把光标落在错误的字符边界
        // （点击/打字插错位置）。宽度表：你=2.0 好=2.0 a=1.0 b=1.0 c=1.0
        let widths = [2.0f32, 2.0, 1.0, 1.0, 1.0]; // "你好abc"
        let w = |k: usize| -> f32 { widths[..k].iter().sum() };
        assert_eq!(caret_index_by_width(5, 1.5, w), 1, "'你' 左半 → 1（等比估算会错）");
        assert_eq!(caret_index_by_width(5, 2.0, w), 1, "'你' 右缘 → 1");
        assert_eq!(caret_index_by_width(5, 3.5, w), 2, "'好' 中 → 2");
        assert_eq!(caret_index_by_width(5, 4.9, w), 3, "'a' 右缘 → 3");
        assert_eq!(caret_index_by_width(5, 6.6, w), 5, "文本末尾之后 → 5");
    }

    #[test]
    fn cut_deletes_full_selection_both_directions() {
        // Ctrl+X 剪切删除范围的回归：反向选择（anchor > caret，向左拖选）必须删
        // 整个 [lo,hi)，不能只删到 caret——曾用 caret.max(lo) 导致反向选择
        // "只复制不删字"。
        let mut s = String::from("ABCDE");
        let (lo, hi) = sel_range(Some(1), 4).unwrap();
        delete_range(&mut s, lo, hi);
        assert_eq!(s, "AE", "正向选择（anchor=1,caret=4）剪切删 BCD");
        let mut s = String::from("ABCDE");
        let (lo, hi) = sel_range(Some(4), 1).unwrap();
        delete_range(&mut s, lo, hi);
        assert_eq!(s, "AE", "反向选择（anchor=4,caret=1）剪切同样删 BCD");
        // 选择 + 退格：先删选择 [lo,hi)，退格不再多删（sel_consumed 语义）
        let mut s = String::from("ABCDE");
        let (lo, hi) = sel_range(Some(1), 4).unwrap();
        delete_range(&mut s, lo, hi);
        assert_eq!(s, "AE", "退格消费选择后不应再 remove_before（'A' 保留）");
    }

    #[test]
    fn vline_of_byte_boundary_attribution() {
        use rjw_text::VisualLine;
        // "ABCDEF\nBCDEFGHIJLKMASDASD\nXXX"：K 自动换入下一行
        // 行0 "ABCDEF" [0,6)　行1 "BCDEFGHIJL" [7,17)　行2 "KMASDASD" [17,25)　行3 "XXX" [25,28)
        let v = vec![
            VisualLine { byte_start: 0, byte_end: 6, top: 0.0, width: 0.0 },
            VisualLine { byte_start: 7, byte_end: 17, top: 18.0, width: 0.0 },
            VisualLine { byte_start: 17, byte_end: 25, top: 36.0, width: 0.0 },
            VisualLine { byte_start: 25, byte_end: 28, top: 54.0, width: 0.0 },
        ];
        assert_eq!(vline_of_byte(&v, 0), 0, "行首");
        assert_eq!(vline_of_byte(&v, 5), 0, "行内");
        assert_eq!(vline_of_byte(&v, 6), 0, "行尾（'\n' 前）归本行");
        assert_eq!(vline_of_byte(&v, 7), 1, "第二行行首");
        assert_eq!(vline_of_byte(&v, 16), 1, "第二行行内（L）");
        assert_eq!(vline_of_byte(&v, 17), 2, "自动换行边界（L 后 = K 前）归**下一行**");
        assert_eq!(vline_of_byte(&v, 24), 2, "第三行行内");
        assert_eq!(vline_of_byte(&v, 25), 3, "第四行行首");
        assert_eq!(vline_of_byte(&v, 28), 3, "文本末尾归最后一行");
        // 空行（'\n\n'）：行区间 [n, n) 无字形
        let v2 = vec![VisualLine { byte_start: 0, byte_end: 0, top: 0.0, width: 0.0 }];
        assert_eq!(vline_of_byte(&v2, 0), 0, "空行");
    }

    #[test]
    fn caret_at_visual_click_wrapped_and_multibyte() {
        use rjw_text::VisualLine;
        let w = |s: &str| s.chars().count() as f32 * 8.0; // 等宽 mock（逻辑测试用）
        // "ABCDEFGHIJKLMNOPQRSTUVWXYZ" 自动换行成 3 个视觉行 [0,8) [8,16) [16,26)
        let value = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let vlines = vec![
            VisualLine { byte_start: 0, byte_end: 8, top: 0.0, width: 64.0 },
            VisualLine { byte_start: 8, byte_end: 16, top: 18.0, width: 64.0 },
            VisualLine { byte_start: 16, byte_end: 26, top: 36.0, width: 80.0 },
        ];
        // 点击第 2 视觉行（全文第 9 字符起）第 2 个字符边界 → 全文索引 10（K 前）
        assert_eq!(caret_at_visual_click(value, &vlines, 1, 16.0, w), 10);
        // 第 3 视觉行行首 → 16
        assert_eq!(caret_at_visual_click(value, &vlines, 2, 0.0, w), 16);
        // 行号越界（点击可视区最末行下方）→ clamp 到最后视觉行
        assert_eq!(caret_at_visual_click(value, &vlines, 99, 0.0, w), 16);
        // 空文本 → 0（不 panic）
        assert_eq!(caret_at_visual_click("", &[], 0, 0.0, w), 0);
        // 混合中英文（多字节）：字节范围与 char 索引换算正确
        // "你好AB世界CD"：你好=6B A=1B B=1B 世界=6B C=1B D=1B → 全长 16B、8 字符
        let cjk = "你好AB世界CD";
        let v3 = vec![
            VisualLine { byte_start: 0, byte_end: 8, top: 0.0, width: 0.0 },   // 你好AB
            VisualLine { byte_start: 8, byte_end: 16, top: 18.0, width: 0.0 }, // 世界CD
        ];
        assert_eq!(caret_at_visual_click(cjk, &v3, 1, 8.0, w), 5, "'世' 后（char 5）");
        assert_eq!(caret_at_visual_click(cjk, &v3, 1, 24.0, w), 7, "'C' 后（char 7）");
        assert_eq!(caret_at_visual_click(cjk, &v3, 0, 16.0, w), 2, "'好' 后（char 2）");
    }
}
