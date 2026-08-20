//! **数字输入框**（组合控件，只依赖公开 API）。
//!
//! - **拖拽调值**：按住右侧手柄**水平拖动**（**向右拖 = 增加**），拖到窗口边缘自动
//!   **warp**（光标跳到对侧继续拖，增量连续），松开结束；手柄悬停/拖拽显示
//!   [`crate::UiCursor::EwResize`]（↔）；
//! - **输入模式**：点击文本框 → 禁用拖拽 + **全选** + I 型光标键盘输入；直到失去
//!   焦点 → 恢复拖拽模式；
//! - 非数字输入被屏蔽（只留数字 / 负号 / 小数点 / 空格）；拖拽基准用独立状态 ID
//!   `{id}::grip`（不与 `text_input_at` 共用 `WidgetState`，否则 press_mouse 互相覆盖）。

use glam::Vec2;
use rjw_transform::Rect;

use crate::hit::update_drag;
use crate::{FocusKind, Response, Ui, UiCursor, Widget};

/// 数字输入框（文本框 + 右侧拖拽调值手柄）。
pub struct NumberInput<'a> {
    id: &'a str,
    text: &'a mut String,
    value: &'a mut f32,
    pub min: f32,
    pub max: f32,
    /// 数值 / 物理像素（默认 0.1：拖动 10px = ±1）。
    pub step: f32,
}

impl<'a> NumberInput<'a> {
    pub fn new(id: &'a str, text: &'a mut String, value: &'a mut f32) -> Self {
        Self { id, text, value, min: 0.0, max: 100.0, step: 0.1 }
    }

    pub fn range(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }
}

impl Widget for NumberInput<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        Vec2::new(ui.theme.input.min_w, ui.theme.input.height)
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        const GRIP_W: f32 = 16.0;
        let grip = Rect::new(rect.x + rect.w - GRIP_W, rect.y, GRIP_W, rect.h);
        let text_rect = Rect::new(rect.x, rect.y, (rect.w - GRIP_W).max(0.0), rect.h);
        let grip_hit = ui.hit_abs(&grip);
        let btn = ui.mouse_left();
        ui.register_focus(self.id, rect, FocusKind::TextInput);
        let focused = ui.state().focused.as_deref() == Some(self.id);
        // 输入模式（聚焦）→ 禁用拖拽；拖拽模式（失焦）→ 手柄水平调值
        let drag_enabled = !focused;
        if btn.down_edge() && grip_hit {
            ui.claim_press();
        }
        let drag_id = format!("{}::grip", self.id);
        let mut dragging = false;
        // 先拷出鼠标/窗口尺寸（ws 借用期间不能再借 ui）
        let mx = ui.mouse_screen().x;
        let my = ui.mouse_screen().y;
        let win_w = ui.window_physical_size().0 as f32;
        // warp 待办（ws 释放后执行 set_cursor_position）
        let mut warp_to: Option<(f32, f32)> = None;
        if drag_enabled {
            let ws = ui.state_mut().widget(&drag_id);
            dragging = update_drag(ws, grip_hit, btn);
            if btn.down_edge() && grip_hit {
                // 拖拽基准：物理 x + 起始值
                ws.press_mouse = Some(Vec2::new(mx, 0.0));
                ws.press_panel = Some(Vec2::new(0.0, *self.value));
            }
            if dragging {
                let pm = ws.press_mouse.unwrap_or(Vec2::new(mx, 0.0)).x;
                let base = ws.press_panel.unwrap_or(Vec2::ZERO).y;
                // warp：鼠标越过窗口左右缘 → 光标跳对侧，拖拽基准同步偏移（增量连续）
                let warp = if mx > win_w {
                    -win_w
                } else if mx < 0.0 {
                    win_w
                } else {
                    0.0
                };
                if warp != 0.0 {
                    ws.press_mouse = Some(Vec2::new(pm + warp, 0.0));
                    warp_to = Some((mx + warp, my));
                }
                let v = (base + (mx - pm) * self.step).clamp(self.min, self.max);
                *self.value = v;
                *self.text = format!("{v:.0}");
            }
        }
        if let Some((nx, ny)) = warp_to {
            ui.set_cursor_position(nx, ny);
        }
        // 光标：手柄悬停/拖拽 → ↔（EwResize）；文本框 → 内置 I 型
        if grip_hit || dragging {
            ui.set_cursor(UiCursor::EwResize);
        }
        // 文本框：打字写入持久 `self.text`，随后屏蔽非数字输入并解析回数值
        ui.text_input_at(self.id, text_rect, self.text);
        self.text.retain(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == '+' || c == ' ');
        // 输入模式：**仅首次聚焦时全选**（之后可正常用鼠标部分选择文本；
        // 失焦后 focused_prev 复位，下次聚焦再全选）。
        let now_focused = ui.state().focused.as_deref() == Some(self.id);
        {
            let ws = ui.state_mut().widget(self.id);
            let just_focused = now_focused && !ws.focused_prev;
            ws.focused_prev = now_focused;
            if just_focused {
                ws.sel_anchor = Some(0);
                ws.caret = self.text.chars().count();
            }
        }
        if let Ok(v) = self.text.trim().parse::<f32>() {
            *self.value = v.clamp(self.min, self.max);
        }
        // 拖拽手柄（公开绘制原语）
        let (border, fg, font_size) = {
            let st = &ui.theme.input;
            (st.border, st.fg, st.font_size)
        };
        ui.push_panel_like(grip, border, border, 1.0, 0.0, 1);
        ui.push_text_rect(
            grip,
            "≡",
            font_size,
            fg,
            None,
            crate::TextAlign::Center,
            crate::draw::TextVAlign::Center,
            None,
            None,
        );
        Response::default()
    }
}
