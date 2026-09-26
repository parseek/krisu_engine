//! 文本编辑核心：`text_input_core`（单行）与 `text_area_impl`（多行）——选择 / 剪贴板 /
//! 双击选词 / IME 候选框 / 换行 / 滚动条跟随。
//!
//! 维护者笔记：纯编辑逻辑已抽到 `crate::edit`（可无 GPU 单测），**这里只留绘制与交互**。
//! 输入框的尺寸责任链读写在 `ui::panel`；滚动条几何在 `ui::scroll`。

use super::*;
use std::sync::Arc;

use rjw_keyboard::KeyCode;
use rjw_text::{Buffer, Text, VisualLine};
use rjw_transform::Rect;
use winit::dpi::{PhysicalPosition, PhysicalSize};

use crate::draw::{
    CornerRadius, DrawKind, TextAlign,
    TextVAlign,
    UiDraw, text_cmd,
};
// 顶点收集 / 合批机制（原在 `ui.rs`，见 `gpu_batch` 模块文档）。
use crate::gpu_batch::{
    line_row_at_y, safe_line_slice,
};
use crate::edit::{
    byte_to_char, caret_at_visual_click, char_to_byte, insert_char_at,
    scroll_follow_caret, sel_range, vline_of_byte,
};
use crate::focus::FocusKind;
use crate::hit::update_interact;
use crate::id::IdAbsolute;
use crate::state::WidgetState;
use crate::view::{clip_for_view, ViewMode};

impl Ui<'_> {
    /// **单行文本编辑核心**（[`crate::widgets::TextEditor`] 的实现；调用方请用
    /// [`Self::text_input_at`] / [`UiAdd::text_input`]）。
    ///
    /// 增强能力：
    /// - **超长文本滚动跟随光标**：文本超出内容区时左移，光标始终可见（`WidgetState::text_scroll`）；
    /// - **文本选择**：按住拖拽选择（`WidgetState::sel_anchor`），选择优先于窗口/面板拖拽
    ///   （按下时置位 `press_claimed`）；Ctrl+C/V/X 复制/粘贴/剪切；选择后打字/退格替换选择；
    /// - **IME 组合候选移入浮动提示框**：组合串（preedit）画在输入框下方浮动小框中（不再占行内）。
    pub(crate) fn text_input_core(&mut self, id: &str, rect: Rect, value: &mut String) {
        let id_for = self.id_for(id);
        self.note_placed(rect);
        let hit = self.hit_abs(&id_for, &rect);
        if hit {
            // 鼠标悬停在输入框上 → 本帧系统光标设为 I 型（finish 统一设置）
            self.cursor_text = true;
        }
        let btn = self.mouse_left();
        // 登记焦点链（Tab/方向键可遍历到输入框）。
        self.register_focus(&id_for, rect, FocusKind::TextInput);
        let mouse_local_x = self.mouse_local_x();
        // 提前测量（避免在 ws 借用期间调用 &mut self 方法）
        let input_style = self.theme.input.clone();
        // 光标定位（按字符**实际宽度**，前缀测量二分——混合中英文精确落位）。
        // **单击与拖选都按"文本坐标"（视口 cx + 水平滚动偏移）**：横向滚动后点击
        // 视口内的 J-K 位置 → 映射到全文 J-K（而非文本前部 A-B），光标落在点击处、
        // 视图不跳回起点。（曾用纯视口 cx：滚动后点击会定位到文本起点附近，随后
        // scroll 跟随把视图拉回开头——"点击右侧视图，视图跳回 A-B"）。
        // `text_scroll` 为**物理像素**（内部计算一律物理），文本坐标 = cx + 物理/scale。
        let prev_scroll = self
            .state
            .widgets
            .get(id_for.as_str())
            .map(|w| w.text_scroll)
            .unwrap_or(0.0);
        // cx_raw 允许**负值**（鼠标拖出左缘）——拖选时左缘持续滚动（edge-scroll）；
        // 单击才 clamp 到 0（点击最左 = 光标在可视区起点）。
        let cx_raw = mouse_local_x - rect.x - input_style.padding_x;
        let cx = cx_raw.max(0.0);
        let click_caret = if btn.down_edge() && hit {
            Some(self.caret_index_at_width(
                value,
                input_style.font_size,
                input_style.font_family.as_deref(),
                cx + prev_scroll,
            ))
        } else {
            None
        };
        let drag_caret = if btn.pressed() && !btn.down_edge() {
            Some(self.caret_index_at_width(
                value,
                input_style.font_size,
                input_style.font_family.as_deref(),
                cx_raw + prev_scroll,
            ))
        } else {
            None
        };
        // 记录帧首光标：仅"光标移动"（打字/方向键/点击/拖选）时做滚动跟随——
        // 滚轮滚动不移动光标 → 不跟随（滚轮自由滚动、光标可滚出视图，不被拉回）。
        let prev_caret = self.state.widgets.get(id_for.as_str()).map(|w| w.caret);
        let caret_est = {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            let ev = update_interact(ws, hit, btn);
            if ev.pressed {
                self.any_pressed = true;
                // 输入框按下占用该次按压：从输入框拖拽 = 选择文本（窗口/面板不建立拖拽基准）
                self.press_claimed = true;
                self.state.focused = Some(id_for.to_static());
                self.state.focused_kind = Some(FocusKind::TextInput);
                if let Some(c) = click_caret {
                    ws.caret = c;
                }
                // 双击检测（同控件时间间隔 ≤ DOUBLE_CLICK_TIME 且位移 < 阈值）：
                // 第二击选中光标所在"词"并进入词模式——按住继续拖拽按词扩散。
                let is_dbl = {
                    let (pt, pp) = (ws.last_click_time, ws.last_click_pos);
                    ws.last_click_time = Some(std::time::Instant::now());
                    ws.last_click_pos = self.mouse_logical;
                    pt.is_some_and(|t| t.elapsed() <= DOUBLE_CLICK_TIME)
                        && (self.mouse_logical - pp).length() < DOUBLE_CLICK_DIST
                };
                // 拖选位移基准（物理像素；微动不触发拖选 → 单击保持插入模式）
                ws.press_mouse = Some(self.mouse_screen.round());
                if is_dbl {
                    let (w0, w1) = crate::edit::word_range(value, ws.caret);
                    ws.sel_anchor = Some(w0);
                    ws.caret = w1;
                    ws.sel_word = true;
                } else {
                    ws.sel_word = false;
                    ws.sel_anchor = Some(ws.caret);
                }
            } else if ws.pressed && btn.pressed() {
                // 拖拽选择：**位移 ≥ 3 物理像素**才扩展选择（单击微动不误选）；
                // 光标跟随鼠标（**即使拖出输入框**——edge-scroll 持续滚动），
                // 选择范围 = [anchor, caret)。
                self.press_claimed = true;
                let moved = ws
                    .press_mouse
                    .map(|p| (self.mouse_screen.round() - p).length_squared() >= 9.0)
                    .unwrap_or(false);
                if moved
                    && let Some(c) = drag_caret {
                        if ws.sel_word {
                            // 词模式（双击后拖拽）：按词边界扩散选择
                            let anchor = ws.sel_anchor.unwrap_or(c);
                            ws.caret = crate::edit::extend_word_caret(value, anchor, c);
                        } else {
                            ws.caret = c;
                        }
                    }
            }
            if ev.released {
                // 纯点击（无位移）：anchor == caret，无实际选择 → 清理，避免残留
                // anchor 在后续无 Shift 方向键移动时"突然变成多选"。
                if ws.sel_anchor == Some(ws.caret) {
                    ws.sel_anchor = None;
                }
                // 释放后退出词模式（选择保留；下次单击/双击重开）。
                ws.sel_word = false;
            }
            let focused = self
                .state
                .focused
                .as_ref()
                .is_some_and(|f| f.as_str() == id_for.as_str());
            if focused {
                // IME 组合中（preedit 非空）**或刚结束的帧**（上一帧在组合）：
                // 退格/删除/方向键由 **IME 系统**处理（缩短组合串、结束组合、移动
                // 组合光标）——本地处理会误删已有文本（组合结束帧 Preedit("") 先清空
                // 候选、随后退格键到达，只看当前帧会误判为非组合而误删）。
                let in_ime_compose =
                    self.keyboard.ime_preedit().is_some_and(|p| !p.is_empty());
                let ime_owns_keys = in_ime_compose || self.state.ime_composing;
                // 编辑状态机（单行）：剪贴板 Ctrl+C/V/X/A（粘贴过滤换行）、选择替换、
                // IME 上屏、普通字符、退格/删除（见 [`crate::edit::apply_frame_edits`]）。
                crate::edit::apply_frame_edits(&self.keyboard, ws, value, false, ime_owns_keys);
                // Shift + ←/→：扩展/收缩选择（无 Shift 取消选择）。
                let shift = self.keyboard.key(KeyCode::ShiftLeft).pressed()
                    || self.keyboard.key(KeyCode::ShiftRight).pressed();
                if self.keyboard.key(KeyCode::ArrowLeft).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, -1, shift);
                }
                if self.keyboard.key(KeyCode::ArrowRight).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, 1, shift);
                }
                if self.keyboard.key(KeyCode::Enter).down_edge() {
                    self.state.focused = None;
                }
                // Esc：取消输入焦点（不再把 Esc 传给应用层快捷键）
                if self.keyboard.key(KeyCode::Escape).down_edge() {
                    self.state.focused = None;
                }
            }
            (focused, ws.caret)
        };
        let (focused, caret) = caret_est;
        // 绘制
        let style = self.theme.input.clone();
        let depth = self.painter.q.depth;
        let win = self.painter.q.cur_win;
        let elem = self.painter.q.seq + 1;
        let border = if focused { style.border_focus } else { style.border };
        // 视觉框绝对矩形（Clip 沙箱用）：**整个输入框**——高亮/光标/文本命令
        // 受其强制裁剪（滚出视图不画出框，且外层 ScrollView 裁切一并生效）。
        let box_clip = Rect::new(self.abs_base.x + rect.x, self.abs_base.y + rect.y, rect.w, rect.h);
        // **Clip 子沙箱**（控件内）：强制裁剪层 = 外层强制 ∩ 输入框矩形。
        let saved_clip = self.painter.q.clip;
        self.painter.q.clip = clip_for_view(saved_clip, box_clip, ViewMode::Clip);
        // 背景 + 边框（radius > 0 走圆角双层矩形）。
        // 圆角来自 `style.radius`（主题值，或 [`crate::widgets::TextEditor::radius`] 的
        // 逐控件覆盖——`NumberInput` 靠它只圆左侧两角，与右侧拖拽手柄拼成直边）。
        let panel_radius = style.radius;
        self.push_panel_like(rect, style.bg, border, style.border_w, panel_radius, elem);
        let content_w = (rect.w - style.padding_x * 2.0).max(0.0);
        let content_rect = Rect::new(rect.x + style.padding_x, rect.y, content_w, rect.h);
        // **IME 组合内联融入**：显示串 = value[..caret] + preedit + value[caret..]——
        // 后续文本（"xXXXXAAAA" 的 AAAA）右移而非被组合盖住；组合较长时滚动跟随
        // 组合光标（提示文字不裁切）。无组合时全部回落到 value（零开销路径）。
        // IME 组合串先拷出（owned）：闭包内要 &mut self（text_size 测量），
        // 与自持快照字段 self.keyboard 的借用不能共存。
        let preedit = self.keyboard.ime_preedit().map(|p| p.to_owned());
        let preedit_caret = self.keyboard.ime_preedit_caret();
        let composed: Option<(String, std::ops::Range<usize>, f32, usize)> = if focused {
            preedit
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let insert_b = char_to_byte(value, caret);
                    let disp = format!("{}{}{}", &value[..insert_b], p, &value[insert_b..]);
                    let w = self.text_size(&p, style.font_size, style.font_family.as_deref()).x;
                    // 组合内光标：字节 → 显示串偏移（None = 组合末尾）
                    let caret_b = preedit_caret
                        .map(|b| p.floor_char_boundary(b.min(p.len())))
                        .unwrap_or(p.len());
                    (disp, insert_b..insert_b + p.len(), w, insert_b + caret_b)
                })
        } else {
            None
        };
        // 文本自然宽（水平滚动上限）与光标 x（前缀宽度）——都基于**显示串**。
        let text_w = match &composed {
            Some((disp, ..)) => {
                self.text_size(disp, style.font_size, style.font_family.as_deref()).x
            }
            None => self.text_size(value, style.font_size, style.font_family.as_deref()).x,
        };
        let caret_x = match &composed {
            Some((disp, _, _, caret_disp)) => self
                .text_size(&disp[..*caret_disp], style.font_size, style.font_family.as_deref())
                .x,
            None => {
                let prefix: String = value.chars().take(caret).collect();
                self.text_size(&prefix, style.font_size, style.font_family.as_deref()).x
            }
        };
        // 水平滚动（**物理像素**）：**水平滚轮（触控板）优先**（自由滚动，可把光标
        // 滚出视图，**仅鼠标在框内时**——指针离开输入框后不再滚动）；
        // 否则仅**光标移动**（打字/方向键/点击/拖选）时跟随光标（右侧保留 8 逻辑
        // 像素；滚轮自由滚动后不被光标拉回；组合时跟随组合光标）。
        let scroll = {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            let (wx, _) = self.mouse.wheel();
            if hit && wx != 0.0 {
                let max_h_px = (text_w - content_w).max(0.0).round();
                ws.text_scroll = (ws.text_scroll - (wx as f32 * 40.0).round())
                    .clamp(0.0, max_h_px);
            } else if Some(caret) != prev_caret {
                ws.text_scroll = scroll_follow_caret(
                    ws.text_scroll,
                    caret_x,
                    content_w,
                    text_w,
                    8.0,
                );
            }
            ws.text_scroll
        };
        let text_dx = -scroll;
        // 文本选择高亮（在文本之下绘制：同一 elem 的图形组先于文字组）。
        if let Some((lo, hi)) = sel_range(
            self.state.widgets.get(id_for.as_str()).and_then(|w| w.sel_anchor),
            caret,
        ) {
            // 选择高亮（**只覆盖选中文字本身**；空行用一格宽兜底，见
            // [`crate::edit::selection_highlight_w`]）。
            let space_w = self
                .text_size(" ", style.font_size, style.font_family.as_deref())
                .x;
            let lo_x = {
                let p: String = value.chars().take(lo).collect();
                self.text_size(&p, style.font_size, style.font_family.as_deref()).x
            };
            let hi_x = {
                let p: String = value.chars().take(hi).collect();
                self.text_size(&p, style.font_size, style.font_family.as_deref()).x
            };
            let sel_rect = Rect::new(
                content_rect.x + lo_x + text_dx,
                content_rect.y + 3.0,
                crate::edit::selection_highlight_w(hi_x - lo_x, space_w),
                (content_rect.h - 6.0).max(0.0),
            );
            if sel_rect.w > 0.0 && sel_rect.h > 0.0 {
                let seq = self.next_seq();
                self.painter.q.queue.push(UiDraw {
                    depth,
                    seq,
                    win,
                    elem,
                    rect: sel_rect,
                    // 选择高亮受输入框强制裁剪（不溢出输入框 / 外层滚动容器）。
                    clip: self.painter.q.clip,
                    full_w: false,
                    // **圆角 + 上下留白**：原来是整块无圆角实心（上下各只缩 1px），
                    // 在圆角输入框里看起来就是一个"方框顶着边框"。现在贴近文字行高，
                    // 小圆角（顺带吃到羽化抗锯齿）。
                    kind: DrawKind::RoundedRect {
                        corners: [style.sel_bg; 4],
                        radius: CornerRadius::all((sel_rect.h * 0.22).min(4.0)),
                    },
                });
            }
        }
        // 文本（左移 scroll；**裁剪窗口固定在视觉框**：clip 相对移动后的 rect 起点 =
        // scroll/scale - padding_x，绝对位置 = 框左缘 —— 若 clip.x=0 会随 rect 一起
        // 左移，始终显示文本开头且偏离文本框；缓冲控件自持）。
        let clip = Rect::new(scroll - style.padding_x, 0.0, rect.w, rect.h);
        // 绘制文本：组合时画**显示串**（preedit 已融入）；缓冲控件自持（按键变化重排）。
        let (draw_text, buf) = match &composed {
            Some((disp, ..)) => {
                let buf = self.ensure_text_buf(
                    id_for.as_str(),
                    disp,
                    style.font_size,
                    style.font_family.as_deref(),
                    0.0,
                    1.0,
                );
                (disp.as_str(), buf)
            }
            None => {
                let buf = self.ensure_text_buf(
                    id_for.as_str(),
                    value,
                    style.font_size,
                    style.font_family.as_deref(),
                    0.0,
                    1.0,
                );
                (value.as_str(), buf)
            }
        };
        let seq = self.next_seq();
        self.painter.q.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            // ⚠ 命令矩形 = **墨迹范围**（宽取全文宽 `text_w`，不是框内宽 `content_w`）：
            // `draw_text_quads` 只用它的 x/y（对齐锚点 + 软裁剪的相对基准），所以观感不变；
            // 但"排版矩形 = 墨迹范围"这条不变量让**兜底剔除**与 `debug_layout` 描边都正确
            // （见 `gpu_batch::text_visible_rect` 的文档：框宽会让滚过头的文本被误剔）。
            Rect::new(content_rect.x + text_dx, content_rect.y, text_w, rect.h),
            Arc::from(draw_text),
            style.font_size,
            style.fg,
            TextAlign::Left,
            TextVAlign::Center,
            style.font_family.clone(),
            Some(clip),
            self.painter.q.clip,
            Some(buf),
        ));
        // **组合下划线**：覆盖组合文本段（显示串 `[span]`），受内容区裁剪。
        // 组合文本已融入显示串（后续文本右移），无需单独绘制文字。
        if let Some((disp, span, preedit_w, _)) = &composed {
            let prefix_x =
                self.text_size(&disp[..span.start], style.font_size, style.font_family.as_deref())
                    .x;
            let ul = Rect::new(
                content_rect.x + prefix_x + text_dx,
                content_rect.y + content_rect.h - 3.0,
                *preedit_w,
                2.0,
            );
            if ul.w > 0.0 && ul.h > 0.0 {
                let useq = self.next_seq();
                self.painter.q.queue.push(UiDraw {
                    depth,
                    seq: useq,
                    win,
                    elem,
                    rect: ul,
                    clip: self.painter.q.clip,
                    full_w: false,
                    kind: DrawKind::Solid(style.preedit),
                });
            }
        }
        // **IME 候选框定位**：跟随组合光标（窗口客户区物理像素；无组合 = 输入光标）。
        if focused {
            let ime_x = (self.abs_base.x + content_rect.x + caret_x + text_dx) as i32;
            let ime_y = (self.abs_base.y + rect.y) as i32;
            let ime_w = rect.w.max(1.0) as u32;
            let ime_h = rect.h.max(1.0) as u32;
            self.window.set_ime_cursor_area(
                PhysicalPosition::new(ime_x, ime_y),
                PhysicalSize::new(ime_w, ime_h),
            );
        }
        // 光标（跟随水平滚动；组合时 = 显示串内的组合光标）
        // 高度 = **字号**（与文本行盒同高，不再是"几乎整框高"的粗条）；垂直居中。
        if focused && self.state.caret_blink_on() {
            let ch = style.font_size.max(1.0);
            let caret_rect = Rect::new(
                content_rect.x + caret_x + text_dx,
                content_rect.y + ((content_rect.h - ch) * 0.5).max(0.0),
                1.0,
                ch,
            );
            let seq = self.next_seq();
            self.painter.q.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: caret_rect,
                clip: self.painter.q.clip,
                full_w: false,
                kind: DrawKind::Caret {
                    color: style.caret,
                    width: 1.0,
                },
            });
        }
        // 退出 Clip 子沙箱（恢复外层强制裁剪层）。
        self.painter.q.clip = saved_clip;
    }

}

impl Ui<'_> {
    /// **多行文本编辑核心**（[`crate::widgets::TextEditor`] 的实现；调用方请用
    /// [`Self::text_area_at`] / [`Self::text_area_at_nw`] / [`UiAdd::text_area`]）。
    ///
    /// - **编辑**：Enter 换行、↑/↓ 跨**视觉行**（保持列）、Home/End 行首/行尾、
    ///   ←/→ 字符移动、Backspace/Delete、选择替换；Esc 失焦；
    /// - **渲染**：文本按内容区宽度**自动换行**（[`rjw_text::Text::create_buffer_wrap`]）；
    ///   超出高度**垂直滚动**（滚轮 + 光标跟随，`WidgetState::scroll_y`）；
    /// - **光标 / 点击 / 选择按"视觉行"（自动换行后）定位**——与显示完全一致
    ///   （[`rjw_text::Text::visual_lines`]：每个 `LayoutRun` 一行，含字节范围）；
    /// - **选择 / 复制 / 粘贴 / 剪切**（Ctrl+C/V/X）跨视觉行，高亮逐行绘制。
    /// - **IME**：组合候选浮动提示框 + 候选框定位到光标。
    ///
    /// 自动换行模式（`wrap = true`）：行宽 = 内容区宽，超出自动换行，仅垂直滚动。
    /// 不自动换行模式（`wrap = false`）：行宽不限（显式 `\n` 分行），**水平滚动**
    /// 跟随光标（同单行输入框），垂直滚动不变。
    pub(crate) fn text_area_impl(&mut self, id: &str, rect: Rect, value: &mut String, wrap: bool) {
        let id_for = self.id_for(id);

        self.note_placed(rect);
        let hit = self.hit_abs(&id_for, &rect);
        if hit {
            // 鼠标悬停在输入框上 → 本帧系统光标设为 I 型（finish 统一设置）
            self.cursor_text = true;
        }
        let btn = self.mouse_left();
        self.register_focus(&id_for, rect, FocusKind::TextInput);
        let style = self.theme.input.clone();
        // **行距**：多行行高 = 字号 × `Theme::line_spacing`（与排版缓冲 `line_mult`
        // 一致；cosmic 行盒按此递增，光标/高亮按视觉行序号 × 行高对齐）。
        // 行距是主题令牌：`Theme::density` 的紧凑 / 宽松就是改它（默认
        // `DEFAULT_LINE_SPACING` = 1.2）。
        let line_mult = self.theme.line_spacing;
        let line_h = (style.font_size * line_mult).max(1.0);
        let content_w = (rect.w - style.padding_x * 2.0).max(0.0);
        let content_rect = Rect::new(rect.x + style.padding_x, rect.y, content_w, rect.h);
        // 视觉框裁剪（**绝对坐标**：`UiDraw.clip` 收集期按绝对逻辑矩形求交；局部
        // content_rect 随容器平移后会错位——高亮/下划线因此被裁到错误区域"看不见"）。
        // 裁剪 = 整个输入框（"完全对应视觉文本框大小"）。
        let box_clip = Rect::new(self.abs_base.x + rect.x, self.abs_base.y + rect.y, rect.w, rect.h);
        let mouse_local_y = self.mouse_logical.y - self.abs_base.y;
        // 排版换行宽：换行模式 = 内容区宽；不换行模式 = 0（不限宽）。
        let wrap_w = if wrap { content_w } else { 0.0 };
        // **视觉行**（自动换行后）：光标/点击/选择/Home-End/↑↓ 全部按它定位，与显示一致。
        let vbuf = self.ensure_text_buf(
            id_for.as_str(),
            value,
            style.font_size,
            style.font_family.as_deref(),
            wrap_w,
            line_mult,
        );
        let vlines = Text::lines(&vbuf);
        // **垂直定位**（文字 / 光标 / 选择 / IME / 点击行号共用**同一个** `v_offset`）：
        // - `TextVAlignMode::TopLeft`（默认）：从上缘往下垫 `InputStyle::padding_y` ——
        //   **框被拉高时文字位置不变**（用户要点）；
        // - `TextVAlignMode::CenterLeft`：内容装得下时在框内居中（装不下 = 顶对齐 + 可滚动）。
        // ⚠ 曾把 `v_offset` 只加在**个别**绘制点上（文字加了、光标与点击没加）⇒ "文字掉到框底、
        // 光标还在顶部、点文字点不中"（用户实测："这样很怪"）。**一处计算、五处共用**。
        let v_offset = {
            let content_h = Text::measure_buffer(&vbuf).y;
            text_v_offset(rect.h, content_h, vlines.len(), style.padding_y, self.text_valign)
        };
        // 字节 → 视觉行（半开区间 + 换行边界归属修正，见 edit::vline_of_byte）
        // 注意：不用闭包捕获 `vlines`——编辑改写文本后会重新排版出**新的** vlines，
        // 闭包会一直引用旧绑定导致行号/行区间错位（选择高亮消失、切片 panic）。
        // 鼠标位置 → 光标（视觉行 + 行内列）。**单击与拖选都按"文本坐标"**
        // （视口 y + 垂直滚动）：长内容（自动换行后多行）滚动后点击，行号 = 视口行 +
        // 滚动行——否则点击可视区任意行都会定位到文本前部、光标行随即被滚动跟随
        // 拉回视口顶部（"自动换行后的行鼠标无法定位"）。拖选 + 滚动 = edge-scroll。
        let prev_scroll = self
            .state
            .widgets
            .get(id_for.as_str())
            .map(|w| w.scroll_y)
            .unwrap_or(0.0);
        // 不换行模式：点击列要加**水平滚动**（同单行输入框；换行模式 hscroll = 0）。
        // `scroll_y` / `text_scroll` 均为**物理像素**（内部计算一律物理）→ 文本坐标
        // = 视口坐标 + 物理/scale。
        let hscroll = if wrap {
            0.0
        } else {
            self.state.widgets.get(id_for.as_str()).map(|w| w.text_scroll).unwrap_or(0.0)
        };
        // 滚动条条带排除：内容超出可视区（预估，编辑前后高度变化微小）时，右缘
        // `SCROLLBAR_W` 条带属于滚动条——按下不建立文本选择（锚点为空，拖拽分支
        // 由 `sel_anchor.is_some()` 守卫，滚动条自身交互不受影响）。
        let content_h_pre = Text::measure_buffer(&vbuf).y;
        let sb_w = if content_h_pre > rect.h + 1.0 && rect.h > 0.0 {
            SCROLLBAR_W
        } else {
            0.0
        };
        let hit_text = hit && (self.mouse_local_x() - rect.x) < rect.w - sb_w;
        // ⚠ 行号锚点必须用**同一份 `v_offset`**（文本画在哪，点击就按哪定位）——
        // 否则"文字被垫高 / 居中"时点在文字上会命中上一行或空行（所见与所点错位）。
        let click_caret = if btn.down_edge() && hit_text {
            // 行号按**真实行顶**定位（见 line_row_at_y：line_h 每行差 ~0.2px，
            // 长文本累积会错行 / 视图卡住）。
            let row = line_row_at_y(
                &vlines,
                mouse_local_y - rect.y - v_offset + prev_scroll,
                line_h,
            );
            let cx = (self.mouse_local_x() - rect.x - style.padding_x + hscroll).max(0.0);
            Some(caret_at_visual_click(value, &vlines, row, cx, |s| {
                self.text_size(s, style.font_size, style.font_family.as_deref()).x
            }))
        } else {
            None
        };
        let drag_caret = if btn.pressed() && !btn.down_edge() {
            let row = line_row_at_y(
                &vlines,
                mouse_local_y - rect.y - v_offset + prev_scroll,
                line_h,
            );
            let cx = (self.mouse_local_x() - rect.x - style.padding_x + hscroll).max(0.0);
            Some(caret_at_visual_click(value, &vlines, row, cx, |s| {
                self.text_size(s, style.font_size, style.font_family.as_deref()).x
            }))
        } else {
            None
        };
        // 记录帧首光标：仅"光标移动"（打字/方向键/点击/拖选）时做光标跟随——
        // 滚轮滚动不移动光标 → 不跟随（滚轮自由滚动、光标可滚出视图，不被拉回）。
        let prev_caret = self.state.widgets.get(id_for.as_str()).map(|w| w.caret);
        let caret_est = {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            let ev = update_interact(ws, hit, btn);
            if ev.pressed {
                self.any_pressed = true;
                // 仅按下在**文本区**（滚动条条带外）时建立文本选择/焦点：
                // 按下滚动条由滚动条自身交互处理（anchor 保持 None）。
                if hit_text {
                    self.press_claimed = true;
                    self.state.focused = Some(id_for.to_static());
                    self.state.focused_kind = Some(FocusKind::TextInput);
                    if let Some(c) = click_caret {
                        ws.caret = c;
                    }
                    // 双击检测（同单行输入框）：第二击选中"词"并进入词模式。
                    let is_dbl = {
                        let (pt, pp) = (ws.last_click_time, ws.last_click_pos);
                        ws.last_click_time = Some(std::time::Instant::now());
                        ws.last_click_pos = self.mouse_logical;
                        pt.is_some_and(|t| t.elapsed() <= DOUBLE_CLICK_TIME)
                            && (self.mouse_logical - pp).length() < DOUBLE_CLICK_DIST
                    };
                    ws.press_mouse = Some(self.mouse_screen.round());
                    if is_dbl {
                        let (w0, w1) = crate::edit::word_range(value, ws.caret);
                        ws.sel_anchor = Some(w0);
                        ws.caret = w1;
                        ws.sel_word = true;
                    } else {
                        ws.sel_word = false;
                        ws.sel_anchor = Some(ws.caret);
                    }
                }
            } else if ws.pressed && btn.pressed() && ws.sel_anchor.is_some() {
                // 拖拽选择：**位移 ≥ 3 物理像素**才扩展选择（单击微动不误选）；
                // 光标跟随鼠标（**即使拖出输入框**——edge-scroll 持续滚动）。
                self.press_claimed = true;
                let moved = ws
                    .press_mouse
                    .map(|p| (self.mouse_screen.round() - p).length_squared() >= 9.0)
                    .unwrap_or(false);
                if moved
                    && let Some(c) = drag_caret {
                        if ws.sel_word {
                            // 词模式（双击后拖拽）：按词边界扩散选择
                            let anchor = ws.sel_anchor.unwrap_or(c);
                            ws.caret = crate::edit::extend_word_caret(value, anchor, c);
                        } else {
                            ws.caret = c;
                        }
                    }
            }
            if ev.released {
                // 纯点击（无位移）→ 清理 anchor（无实际选择），避免残留 anchor 在
                // 后续无 Shift 方向键移动时"突然变成多选"。
                if ws.sel_anchor == Some(ws.caret) {
                    ws.sel_anchor = None;
                }
                // 释放后退出词模式（选择保留；下次单击/双击重开）。
                ws.sel_word = false;
            }
            let focused = self
                .state
                .focused
                .as_ref()
                .is_some_and(|f| f.as_str() == id_for.as_str());
            if focused {
                let in_ime_compose =
                    self.keyboard.ime_preedit().is_some_and(|p| !p.is_empty());
                let ime_owns_keys = in_ime_compose || self.state.ime_composing;
                // 编辑状态机（多行）：剪贴板保留换行、选择替换（Enter 计入）、
                // IME 上屏、普通字符（过滤 '\n'）、退格/删除。
                crate::edit::apply_frame_edits(&self.keyboard, ws, value, true, ime_owns_keys);
                // 换行（Enter；TextArea 语义：插入 '\n'，Esc 失焦）——选择替换已由
                // apply_frame_edits 在 Enter 计入 edit_pending 时先消费。
                if self.keyboard.key(KeyCode::Enter).down_edge() {
                    insert_char_at(value, ws.caret, '\n');
                    ws.caret = (ws.caret + 1).min(value.chars().count());
                }
                // Shift + 方向键/Home/End：扩展选择
                let shift = self.keyboard.key(KeyCode::ShiftLeft).pressed()
                    || self.keyboard.key(KeyCode::ShiftRight).pressed();
                let shift_start = |ws: &mut WidgetState| {
                    if shift && ws.sel_anchor.is_none() {
                        ws.sel_anchor = Some(ws.caret);
                    }
                };
                // 无 Shift 的方向键：**取消选择**（否则 Shift 多选后松开再按 ←/→/↑/↓
                // 残留 anchor 会继续扩展选择）。
                let shift_clear = |ws: &mut WidgetState| {
                    if !shift {
                        ws.sel_anchor = None;
                    }
                };
                let shift_shrink = |ws: &mut WidgetState| {
                    if ws.sel_anchor == Some(ws.caret) {
                        ws.sel_anchor = None;
                    }
                };
                // ←/→：字符移动（Shift 扩展，共用状态机）。
                if self.keyboard.key(KeyCode::ArrowLeft).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, -1, shift);
                }
                if self.keyboard.key(KeyCode::ArrowRight).down_edge() && !ime_owns_keys {
                    crate::edit::caret_horiz(ws, value, 1, shift);
                }
                if self.keyboard.key(KeyCode::ArrowUp).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    shift_clear(ws);
                    // 跨**视觉行**（保持列；列 = 相对行首的 char 数）
                    let cur_byte = char_to_byte(value, ws.caret);
                    let li = vline_of_byte(&vlines, cur_byte);
                    let col = byte_to_char(value, cur_byte) - byte_to_char(value, vlines[li].byte_start);
                    let tgt = li.saturating_sub(1);
                    let line = &vlines[tgt];
                    // 编辑同帧改写文本后 vlines 可能过期：字节边界对齐防 panic
                    // （短暂错位次帧重排后自愈）。
                    let ls = value.floor_char_boundary(line.byte_start);
                    let ltxt = safe_line_slice(value, line);
                    let col = col.min(ltxt.chars().count());
                    ws.caret = byte_to_char(value, ls + char_to_byte(ltxt, col));
                    shift_shrink(ws);
                }
                if self.keyboard.key(KeyCode::ArrowDown).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    shift_clear(ws);
                    let cur_byte = char_to_byte(value, ws.caret);
                    let li = vline_of_byte(&vlines, cur_byte);
                    let col = byte_to_char(value, cur_byte) - byte_to_char(value, vlines[li].byte_start);
                    let tgt = (li + 1).min(vlines.len().saturating_sub(1));
                    let line = &vlines[tgt];
                    let ls = value.floor_char_boundary(line.byte_start);
                    let ltxt = safe_line_slice(value, line);
                    let col = col.min(ltxt.chars().count());
                    ws.caret = byte_to_char(value, ls + char_to_byte(ltxt, col));
                    shift_shrink(ws);
                }
                if self.keyboard.key(KeyCode::Home).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    shift_clear(ws);
                    let li = vline_of_byte(&vlines, char_to_byte(value, ws.caret));
                    ws.caret =
                        byte_to_char(value, value.floor_char_boundary(vlines[li].byte_start));
                    shift_shrink(ws);
                }
                if self.keyboard.key(KeyCode::End).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    shift_clear(ws);
                    let li = vline_of_byte(&vlines, char_to_byte(value, ws.caret));
                    ws.caret =
                        byte_to_char(value, value.floor_char_boundary(vlines[li].byte_end.min(value.len())));
                    shift_shrink(ws);
                }
                if self.keyboard.key(KeyCode::Escape).down_edge() {
                    self.state.focused = None;
                }
            }
            (focused, ws.caret)
        };
        let (focused, caret) = caret_est;
        // 光标是否移动（编辑/点击/拖选/方向键）；false = 纯滚轮滚动 → 不做光标跟随
        let caret_moved = Some(caret) != prev_caret;
        // 编辑（粘贴/打字/IME/退格/删除）可能已改写 `value` → **重新排版**：
        // 视觉行字节区间必须对齐新文本，否则显示/光标定位用旧区间切片会落在
        // 多字节字符中间（如粘贴中文时 `&value[a..b]` panic）。`ws` 已随
        // `caret_est` 结束释放，可安全 `&mut self` 重排。
        let vbuf = self.ensure_text_buf(
            id_for.as_str(),
            value,
            style.font_size,
            style.font_family.as_deref(),
            wrap_w,
            line_mult,
        );
        let vlines = Text::lines(&vbuf);
        // **IME 组合内联融入**（多行）：显示串 = value[..caret] + preedit + value[caret..]，
        // 按内容宽度**重新换行**——组合后的后续文本右移/换行而非被盖住；组合较长时
        // 垂直滚动跟随组合光标。无组合时回落 value（零开销路径）。
        // IME 组合串先拷出（owned）：闭包内要 &mut self（text_size 测量），
        // 与自持快照字段 self.keyboard 的借用不能共存。
        let preedit = self.keyboard.ime_preedit().map(|p| p.to_owned());
        let preedit_caret = self.keyboard.ime_preedit_caret();
        let composed: Option<(String, std::ops::Range<usize>, f32, usize)> = if focused {
            preedit
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let insert_b = char_to_byte(value, caret);
                    let disp = format!("{}{}{}", &value[..insert_b], p, &value[insert_b..]);
                    let w = self.text_size(&p, style.font_size, style.font_family.as_deref()).x;
                    let caret_b = preedit_caret
                        .map(|b| p.floor_char_boundary(b.min(p.len())))
                        .unwrap_or(p.len());
                    (disp, insert_b..insert_b + p.len(), w, insert_b + caret_b)
                })
        } else {
            None
        };
        // 组合时：显示串重新排版（换行随组合变化）；否则复用 value 的 vbuf/vlines。
        let (draw_buf, draw_vlines, draw_disp): (Arc<Buffer>, Vec<VisualLine>, Option<String>) =
            match &composed {
                Some((disp, ..)) => {
                    let b = self.ensure_text_buf(
                        id_for.as_str(),
                        disp,
                        style.font_size,
                        style.font_family.as_deref(),
                        wrap_w,
                        line_mult,
                    );
                    (b.clone(), Text::lines(&b), Some(disp.clone()))
                }
                None => (vbuf.clone(), vlines.clone(), None),
            };
        // 光标所在**视觉行** → 光标 x / y（基于**显示串**视觉行；y = 行序号 × 行高）。
        let draw_text: &str = draw_disp.as_deref().unwrap_or(value);
        let caret_disp = composed
            .as_ref()
            .map(|(_, _, _, c)| *c)
            .unwrap_or_else(|| char_to_byte(value, caret));
        let caret_line = crate::edit::vline_of_byte(&draw_vlines, caret_disp);
        let caret_x = {
            let line = &draw_vlines[caret_line];
            let end = caret_disp.min(line.byte_end).max(line.byte_start);
            let prefix = &draw_text[line.byte_start..end];
            self.text_size(prefix, style.font_size, style.font_family.as_deref()).x
        };
        // 不换行模式：**水平滚动**跟随光标（同单行输入框；光标右侧保留 8 逻辑像素）；
        // **水平滚轮（触控板）优先**——自由滚动（可把光标滚出视图，**仅鼠标在框内
        // 时**），否则仅光标移动时跟随。换行模式：无水平滚动（text_dx = 0）。
        let (wx, wy) = self.mouse.wheel();
        let text_dx = if wrap {
            0.0
        } else {
            let line = &draw_vlines[caret_line];
            let ls = value.floor_char_boundary(line.byte_start);
            let le = value.floor_char_boundary(line.byte_end.min(value.len()));
            let line_w = if ls < le {
                self.text_size(&draw_text[ls..le], style.font_size, style.font_family.as_deref())
                    .x
            } else {
                0.0
            };
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            if hit && wx != 0.0 {
                // 水平滚轮（触控板）：自由滚动（可超出光标），clamp 到内容宽
                let max_h_px = (line_w - content_w).max(0.0).round();
                ws.text_scroll = (ws.text_scroll - (wx as f32 * 40.0).round())
                    .clamp(0.0, max_h_px);
            } else if caret_moved {
                // 光标移动（打字/方向键/点击/拖选）→ 跟随；滚轮滚动不跟随
                ws.text_scroll = scroll_follow_caret(
                    ws.text_scroll,
                    caret_x,
                    content_w,
                    line_w,
                    8.0,
                );
            }
            -ws.text_scroll
        };
        // 光标 y = **真实行顶**（`VisualLine.top` **物理像素**）——与渲染行网格
        // 完全一致；`行号 × line_h` 每行差 ~0.2px，长文本累积后光标/滚动目标漂移
        // （视图卡在短于真正底部的纵轴范围内）。`caret_y`（逻辑）供绘制用。
        // ⚠ `v_offset`（垂直对齐的总偏移）在**上文**已算好（与点击行号、文本、选择同一份）——
        // 光标必须与文字一起走，否则"看得见的文字"与"光标落在哪"分家。
        let caret_y_px = draw_vlines[caret_line].top;
        let caret_y = caret_y_px;
        // 垂直滚动：滚轮 + 光标跟随。内容高用**实际排版缓冲**（组合时 = 显示串缓冲）。
        let content_h = Text::measure_buffer(&draw_buf).y;
        let max_scroll_px = (content_h - rect.h).max(0.0).round();
        let scroll = {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            // 滚轮（**仅鼠标在框内时**——指针离开输入框后不再滚动；拖选中不滚轮）。
            if hit && !ws.pressed
                && wy != 0.0 {
                    ws.scroll_y = (ws.scroll_y - (wy as f32 * 30.0).round())
                        .clamp(0.0, max_scroll_px);
                }
            // **拖选 edge-scroll**：鼠标越出可视区上下缘时按越出量持续滚动
            // （光标随后一帧按新滚动重新定位 → 选择持续延伸，直至文本两端）。
            if ws.pressed {
                let y = mouse_local_y - rect.y;
                if y > rect.h {
                    ws.scroll_y = (ws.scroll_y + (y - rect.h)).min(max_scroll_px);
                } else if y < 0.0 {
                    ws.scroll_y = (ws.scroll_y + y).max(0.0);
                }
            }
            // 光标跟随（仅**光标移动**时，如打字/方向键/点击/拖选）：光标行滚出
            // 可视区时调整——滚轮滚动不移动光标 → 不跟随（滚轮自由滚动、光标可
            // 滚出视图，且不被下一帧拉回）。
            if caret_moved {
                if caret_y_px < ws.scroll_y {
                    ws.scroll_y = caret_y_px;
                } else if caret_y_px + line_h > ws.scroll_y + rect.h {
                    ws.scroll_y =
                        (caret_y_px + line_h - rect.h)
                            .min(max_scroll_px);
                }
            }
            ws.scroll_y = ws.scroll_y.clamp(0.0, max_scroll_px);
            ws.scroll_y
        };
        // 绘制
        let depth = self.painter.q.depth;
        let win = self.painter.q.cur_win;
        let elem = self.painter.q.seq + 1;
        let border = if focused { style.border_focus } else { style.border };
        // **Clip 子沙箱**（控件内）：强制裁剪层 = 外层强制 ∩ 输入框矩形。
        // 光标 / 高亮 / 文本命令自动受其裁剪（滚出视图不画出框）。
        let saved_clip = self.painter.q.clip;
        self.painter.q.clip = clip_for_view(saved_clip, box_clip, ViewMode::Clip);
        self.push_panel_like(rect, style.bg, border, style.border_w, style.radius, elem);
        // 选择高亮（逐**视觉行**；x = 行内前缀宽度，y = 视觉行序号 × 行高——与显示一致）
        if let Some((lo, hi)) = sel_range(
            self.state.widgets.get(id_for.as_str()).and_then(|w| w.sel_anchor),
            caret,
        ) {
            let lo_byte = char_to_byte(value, lo);
            let hi_byte = char_to_byte(value, hi);
            // 一个空格宽度：**行尾/空行提示**——高亮向右多留一格（见循环内说明）。
            let space_w = self
                .text_size(" ", style.font_size, style.font_family.as_deref())
                .x;
            // 用**重新排版后**的 vlines（编辑后行区间才与当前文本一致，否则高亮
            // 错位/消失；见上方重排注释）。
            let lo_li = vline_of_byte(&vlines, lo_byte);
            let hi_li = vline_of_byte(&vlines, hi_byte);
            for (i, line) in vlines[lo_li..=hi_li].iter().enumerate() {
                let li = lo_li + i;
                let ls = line.byte_start.min(value.len());
                let le = line.byte_end.min(value.len());
                let c0b = if li == lo_li { lo_byte.max(ls).min(le) } else { ls };
                let c1b = if li == hi_li { hi_byte.max(ls).min(le) } else { le };
                let x0 = self
                    .text_size(&value[ls..c0b], style.font_size, style.font_family.as_deref())
                    .x;
                let x1 = self
                    .text_size(&value[ls..c1b], style.font_size, style.font_family.as_deref())
                    .x;
                // **只覆盖选中文字本身**；空行（x0==x1，原逻辑 `c1b<=c0b` 直接跳过）
                // 用一格宽兜底，标出该空行已在选中范围内（见
                // [`crate::edit::selection_highlight_w`]）。
                let sel_w = crate::edit::selection_highlight_w(x1 - x0, space_w);
                // y 随垂直滚动上移（-scroll/scale）；clip = 输入框强制层（选择高亮
                // 受裁剪，不溢出输入框 / 外层滚动容器）。
                // 行顶用真实 `VisualLine.top`（与文本行网格一致，长文本不漂移）。
                let sel_rect = Rect::new(
                    content_rect.x + x0 + text_dx,
                    rect.y + v_offset + vlines[li].top - scroll,
                    sel_w,
                    line_h,
                );
                if sel_rect.w > 0.0 {
                    let seq = self.next_seq();
                    self.painter.q.queue.push(UiDraw {
                        depth,
                        seq,
                        win,
                        elem,
                        rect: sel_rect,
                        clip: self.painter.q.clip,
                    full_w: false,
                        // 与单行输入框一致：圆角高亮（不是硬边实心块）。
                        kind: DrawKind::RoundedRect {
                            corners: [style.sel_bg; 4],
                            radius: CornerRadius::all((sel_rect.h * 0.22).min(4.0)),
                        },
                    });
                }
            }
        }
        // 文本（换行 + 垂直滚动；clip 相对文本块：上缘 = scroll/scale，高 = 可视区；
        // 缓冲控件自持——组合时用显示串缓冲，否则复用 `vbuf`）。
        let seq = self.next_seq();
        self.painter.q.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            // ⚠ 命令矩形 = **墨迹范围**（高取全文高 `content_h`，不是框高 `rect.h`）：
            // `draw_text_quads` 只用它的 x/y（对齐锚点 + 软裁剪的相对基准），观感不变；
            // 但"排版矩形 = 墨迹范围"让**兜底剔除**正确（框高会让纵向滚过头的文本被误剔，
            // 见 `gpu_batch::text_visible_rect`）。
            Rect::new(
                content_rect.x + text_dx,
                content_rect.y + v_offset - scroll,
                content_w,
                content_h,
            ),
            Arc::from(draw_text),
            style.font_size,
            style.fg,
            TextAlign::Left,
            // 多行编辑：**顶对齐**（行盒顶 = 内容区顶），与光标/点击的 TopLeft 定位一致
            TextVAlign::Top,
            style.font_family.clone(),
            // 内层裁剪相对**移动后**的文本 rect（含水平滚动）：窗口固定在视觉框
            Some(Rect::new(
                -style.padding_x - text_dx,
                scroll,
                rect.w,
                rect.h,
            )),
            self.painter.q.clip,
            Some(draw_buf),
        ));
        // **组合下划线**：覆盖组合文本段（显示串 `[span]`，可能跨视觉行），
        // 受内容区裁剪。组合文本已融入显示串（后续文本右移/换行），无需单独绘制文字。
        if let Some((disp, span, _, _)) = &composed {
            let s_li = crate::edit::vline_of_byte(&draw_vlines, span.start);
            let e_li = crate::edit::vline_of_byte(&draw_vlines, span.end.saturating_sub(1));
            for (i, line) in draw_vlines[s_li..=e_li].iter().enumerate() {
                let li = s_li + i;
                let ls = line.byte_start.min(disp.len());
                let le = line.byte_end.min(disp.len());
                let x0b = span.start.max(ls).min(le);
                let x1b = span.end.max(ls).min(le);
                if x1b <= x0b {
                    continue;
                }
                let x0 = self
                    .text_size(&disp[ls..x0b], style.font_size, style.font_family.as_deref())
                    .x;
                let x1 = self
                    .text_size(&disp[ls..x1b], style.font_size, style.font_family.as_deref())
                    .x;
                let ul = Rect::new(
                    content_rect.x + x0 + text_dx,
                    rect.y + v_offset + draw_vlines[li].top + line_h - 3.0 - scroll,
                    (x1 - x0).max(0.0),
                    2.0,
                );
                if ul.w > 0.0 && ul.h > 0.0 {
                    let useq = self.next_seq();
                    self.painter.q.queue.push(UiDraw {
                        depth,
                        seq: useq,
                        win,
                        elem,
                        rect: ul,
                        clip: self.painter.q.clip,
                    full_w: false,
                        kind: DrawKind::Solid(style.preedit),
                    });
                }
            }
        }
        // **IME 候选框定位**：跟随组合光标（窗口客户区物理像素；无组合 = 输入光标）。
        if focused {
            let ime_x = (self.abs_base.x + content_rect.x + caret_x + text_dx) as i32;
            let ime_y = (self.abs_base.y + content_rect.y + caret_y_px - scroll) as i32;
            let ime_w = rect.w.max(1.0) as u32;
            let ime_h = line_h.max(1.0) as u32;
            self.window.set_ime_cursor_area(
                PhysicalPosition::new(ime_x, ime_y),
                PhysicalSize::new(ime_w, ime_h),
            );
        }
        // 光标（组合时 = 显示串内的组合光标）
        if focused && self.state.caret_blink_on() {
            let caret_rect = Rect::new(
                content_rect.x + caret_x + text_dx,
                content_rect.y + v_offset + caret_y - scroll,
                1.0,
                line_h,
            );
            let seq = self.next_seq();
            self.painter.q.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: caret_rect,
                clip: self.painter.q.clip,
                full_w: false,
                kind: DrawKind::Caret {
                    color: style.caret,
                    width: 1.0,
                },
            });
        }
        // **垂直滚动条**（内容超出可视区时显示；拖 thumb / 点轨道翻页）——
        // 复用 `scroll_at` 的滚动条（物理像素偏移；`elem` = 本控件 → 覆盖在文本
        // 之上）；状态 ID 独立（`{id}::vbar`），拖拽与文本选择互不干扰。
        if content_h > rect.h + 1.0 && rect.h > 0.0 {
            let new_px = self.scrollbar(
                &IdAbsolute::owned(format!("{}::vbar", id_for.as_str())),
                &Rect::new(rect.x, rect.y, rect.w, rect.h),
                rect.h,
                content_h,
                scroll,
                max_scroll_px,
                self.painter.q.clip,
                elem,
            );
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            ws.scroll_y = new_px;
        }
        // 退出 Clip 子沙箱（恢复外层强制裁剪层）。
        self.painter.q.clip = saved_clip;
    }
}

use crate::widgets::TextVAlignMode;

/// **多行编辑的垂直偏移**（纯函数，可单测；物理像素）：文字 / 光标 / 选择 / IME / 点击
/// 行号**共用**它 —— 一处计算、五处共用（分开算过一次，症状是"文字掉到框底、光标还在顶部"）。
///
/// - [`TextVAlignMode::TopLeft`]（**默认**）：`pad_y`（`InputStyle::padding_y` 垫高）。
///   **框拉高时偏移不变** ⇒ 文字从顶部开始、与单行输入框观感一致（用户要点）；
/// - [`TextVAlignMode::CenterLeft`]：内容**装得下**时 `(rect_h - content_h)/2`，否则退回
///   `pad_y`（装不下时居中会把首行挤出可视区，那时要的是顶对齐 + 可滚动）。
pub(crate) fn text_v_offset(
    rect_h: f32,
    content_h: f32,
    lines: usize,
    pad_y: f32,
    mode: TextVAlignMode,
) -> f32 {
    let _ = lines;
    match mode {
        TextVAlignMode::TopLeft => pad_y.max(0.0),
        TextVAlignMode::CenterLeft => {
            if content_h > rect_h {
                // 装不下 ⇒ 退回顶对齐 + `padding_y`（居中会把首行挤出可视区）。
                // ⚠ 这里**必须**是 `pad_y`（不是 0）：否则"切到 CenterLeft 后文字突然贴边"。
                pad_y.max(0.0)
            } else if content_h > 0.0 && rect_h.is_finite() {
                ((rect_h - content_h) * 0.5).max(0.0)
            } else {
                // 空内容（`content_h == 0` / NaN）⇒ 等价顶对齐：把光标垫到 `pad_y`。
                pad_y.max(0.0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{text_v_offset, TextVAlignMode};

    #[test]
    fn top_left_keeps_the_offset_when_the_box_grows() {
        // **默认 TopLeft**：框被拉高**不影响**文字/光标位置（用户要点："多行被拉高时位置
        // 也不会发生变化"）—— 偏移恒等于 `padding_y`。
        for h in [26.0, 90.0, 400.0] {
            assert_eq!(text_v_offset(h, 20.0, 1, 4.0, TextVAlignMode::TopLeft), 4.0);
        }
        // 负内边距夹到 0（不把文字拉到框外）。
        assert_eq!(text_v_offset(90.0, 20.0, 1, -5.0, TextVAlignMode::TopLeft), 0.0);
    }

    #[test]
    fn center_left_centers_only_when_content_fits() {
        // 装得下 ⇒ 居中（差多少补一半）。
        assert_eq!(text_v_offset(90.0, 40.0, 1, 4.0, TextVAlignMode::CenterLeft), 25.0);
        // 装不下 ⇒ 退回顶对齐 + `padding_y`（居中会把首行挤出去）。
        assert_eq!(text_v_offset(30.0, 40.0, 1, 4.0, TextVAlignMode::CenterLeft), 4.0);
        // 退化输入：**空内容**等价顶对齐（把光标垫到 `pad_y`，不是 0）。
        assert_eq!(text_v_offset(0.0, 0.0, 0, 3.0, TextVAlignMode::CenterLeft), 3.0);
        assert_eq!(text_v_offset(f32::NAN, 1.0, 1, 3.0, TextVAlignMode::CenterLeft), 3.0);
    }
}

