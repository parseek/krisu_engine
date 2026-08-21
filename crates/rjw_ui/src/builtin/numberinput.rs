//! **数字输入框**（组合控件，只依赖公开 API）。
//!
//! - **拖拽调值**：按住右侧手柄**水平拖动**（**向右拖 = 增加**），拖到窗口边缘自动
//!   **warp**（光标跳到对侧继续拖，增量连续），松开结束；手柄悬停/拖拽显示
//!   [`crate::UiCursor::EwResize`]（↔）；**窗口最大化时同样生效**（鼠标无法越出窗口，
//!   用"边缘检测"触发 warp，指针跳到对侧内侧继续拖）；
//! - **输入模式**：点击文本框 → 禁用拖拽 + **全选** + I 型光标键盘输入；直到失去
//!   焦点 → 恢复拖拽模式；
//! - **显示文本内部管理**：[`NumberInput::new`] 只需数值引用——失焦显示由 `value`
//!   派生，聚焦编辑缓冲跨帧持久于 `WidgetState`（无需调用方持有 `String`）；也可
//!   [`NumberInput::with_text`] 绑定外部 `&mut String`（旧形态）；
//! - 非数字输入被屏蔽（只留数字 / 负号 / 小数点 / 空格）；拖拽基准用独立状态 ID
//!   `{id}::grip`（不与 `text_input_at` 共用 `WidgetState`，否则 press_mouse 互相覆盖）。

use glam::Vec2;
use rjw_transform::Rect;

use crate::hit::update_drag;
use crate::id::IdAbsolute;
use crate::{FocusKind, Response, Ui, UiCursor, Widget};

/// 数字输入框（文本框 + 右侧拖拽调值手柄）。
pub struct NumberInput<'a> {
    id: &'a str,
    value: &'a mut f32,
    /// 外部绑定的显示文本（`None` = 内部跨帧持久管理，无需调用方持有）。
    text: Option<&'a mut String>,
    pub min: f32,
    pub max: f32,
    /// 数值 / 物理像素（默认 0.1：拖动 10px = ±1）。
    pub step: f32,
    /// 按住 **Shift** 拖拽的速度倍率（默认 10：细调）。
    pub shift_speed: f32,
    /// 按住 **Ctrl** 拖拽的速度倍率（默认 0.1：精调）。
    pub ctrl_speed: f32,
}

impl<'a> NumberInput<'a> {
    /// 主构造：只需数值引用——显示文本由内部跨帧持久管理（失焦显示由 `value` 派生，
    /// 聚焦时编辑缓冲存于 `WidgetState`），**无需调用方持有 `&mut String`**。
    pub fn new(id: &'a str, value: &'a mut f32) -> Self {
        Self {
            id,
            value,
            text: None,
            min: f32::MIN,
            max: f32::MAX,
            step: 0.1,
            shift_speed: 10.0,
            ctrl_speed: 0.1,
        }
    }

    /// 可选：绑定外部显示文本（旧形态，显式持有 / 需要从外部读显示值的场景）。
    pub fn with_text(mut self, text: &'a mut String) -> Self {
        self.text = Some(text);
        self
    }

    pub fn range(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// 拖拽**精度**：每像素数值（默认 0.1：拖 10px = ±1）。
    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// 按住 **Shift** 拖拽的速度倍率（默认 10）。
    pub fn shift_speed(mut self, s: f32) -> Self {
        self.shift_speed = s;
        self
    }

    /// 按住 **Ctrl** 拖拽的速度倍率（默认 0.1）。
    pub fn ctrl_speed(mut self, s: f32) -> Self {
        self.ctrl_speed = s;
        self
    }
}

/// 按 `step` 精度格式化（step < 1 时保留足够小数位，如 0.25 → 2 位；step ≥ 1 或 ≤ 0
/// 取整）——显示不掩盖 step 步进（旧 `{:.0}` 把 0.25 步进取整成整数，观感"step 没生效"）。
fn fmt_step(v: f32, step: f32) -> String {
    if step > 0.0 && step < 1.0 {
        let mut dec = ((-step.log10()).ceil() as usize).max(1);
        // 步进值不是 10 的整幂时多留一位（0.25 → 2 位；0.1 → 1 位；0.05 → 2 位）。
        while dec < 6 {
            let mag = step * 10f32.powi(dec as i32);
            if (mag - mag.round()).abs() < 1e-4 {
                break;
            }
            dec += 1;
        }
        format!("{v:.dec$}")
    } else {
        format!("{v:.0}")
    }
}

impl Widget for NumberInput<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        Vec2::new(ui.theme.input.min_w, ui.theme.input.height)
    }

    fn ui(mut self, ui: &mut Ui, rect: Rect) -> Response {
        let id_for = ui.id_for(self.id);
        let focused = ui
            .state()
            .focused
            .as_ref()
            .is_some_and(|f| f.as_str() == id_for.as_str());

        const GRIP_W: f32 = 16.0;
        let grip = Rect::new(rect.x + rect.w - GRIP_W, rect.y, GRIP_W, rect.h);
        let text_rect = Rect::new(rect.x, rect.y, (rect.w - GRIP_W).max(0.0), rect.h);
        let grip_hit = ui.hit_abs(&grip);
        let btn = ui.mouse_left();
        ui.register_focus(&id_for, rect, FocusKind::TextInput);

        // 显示 / 编辑文本来源：
        // - 外部绑定（`with_text`）→ 直接使用（旧形态）；
        // - 内部管理 → 聚焦时从 `WidgetState::input_text` 取出（首次用 `value` 派生
        //   初始化）；失焦时用 `value` 派生文本（无需存储）。
        let mut owned: Option<String> = None;
        if self.text.is_none() {
            owned = Some(if focused {
                let ws = ui.state_mut().widget(&id_for);
                ws.input_text
                    .take()
                    .unwrap_or_else(|| fmt_step(*self.value, self.step))
            } else {
                fmt_step(*self.value, self.step)
            });
        }
        // 统一编辑缓冲：外部绑定或内部 owned（字段分离借用：text 字段 vs value 字段）。
        let edit_text: &mut String = match &mut self.text {
            Some(ext) => ext,
            None => owned.as_mut().expect("internal text initialized"),
        };

        // 输入模式（聚焦）也可拖动手柄调值：手柄在右侧 grip 区（text_rect 之外），
        // 聚焦时点手柄拖动 → 调值并更新编辑缓冲；点文本框 → 打字。两者区域分离。
        let drag_enabled = true;
        if btn.down_edge() && grip_hit {
            ui.claim_press();
        }
        let drag_id = IdAbsolute::owned(format!("{}::grip", id_for.as_str()));
        let mut dragging = false;
        // 先拷出鼠标/窗口尺寸（ws 借用期间不能再借 ui）
        let mx = ui.mouse_screen().x;
        let my = ui.mouse_screen().y;
        let win_w = ui.window_physical_size().0 as f32;
        // warp 待办（ws 释放后执行 set_cursor_position）
        let mut warp_to: Option<(f32, f32)> = None;
        if drag_enabled {
            // 速度倍率：Shift = 细调（默认 ×10），Ctrl = 精调（默认 ×0.1）。
            let speed = if ui.key_down(winit::keyboard::KeyCode::ShiftLeft)
                || ui.key_down(winit::keyboard::KeyCode::ShiftRight)
            {
                self.shift_speed
            } else if ui.key_down(winit::keyboard::KeyCode::ControlLeft)
                || ui.key_down(winit::keyboard::KeyCode::ControlRight)
            {
                self.ctrl_speed
            } else {
                1.0
            };
            let ws = ui.state_mut().widget(&drag_id);
            dragging = update_drag(ws, grip_hit, btn);
            if btn.down_edge() && grip_hit {
                // 拖拽基准：物理 x + 起始值
                ws.press_mouse = Some(Vec2::new(mx, 0.0));
                ws.press_panel = Some(Vec2::new(0.0, *self.value));
            }
            if dragging {
                let mut pm = ws.press_mouse.unwrap_or(Vec2::new(mx, 0.0)).x;
                let mut base = ws.press_panel.unwrap_or(Vec2::ZERO).y;
                // 速度（Shift/Ctrl）变化 → 重设拖拽基准：从**当前值**继续增量，
                // 避免 `Δx × 新 speed` 使值瞬间跳变。
                let prev_sens = ws.drag_sens;
                if prev_sens != 0.0 && prev_sens != speed {
                    pm = mx;
                    base = *self.value;
                    ws.press_mouse = Some(Vec2::new(pm, 0.0));
                    ws.press_panel = Some(Vec2::new(0.0, base));
                }
                ws.drag_sens = speed;
                // warp：鼠标到达窗口左右**边缘**即 wrap（而非"越出窗口"）——窗口
                // **最大化**时鼠标无法越出窗口（窗口填满显示器），越界检测永不触发；
                // 边缘检测在拖到窗口边缘的瞬间生效。指针跳转到对侧**内侧** `o`（距
                // 对侧边缘 3px，非边缘 → 跳转后不反复触发，且够近保持连续）；拖拽
                // 基准同步偏移（增量连续：`v = base + (mx - pm)` 用本帧旧 `pm`）。
                const EDGE: f32 = 1.0;
                let o = 3.0;
                let (warp, new_mx) = if mx >= win_w - EDGE {
                    (-(win_w - o), o)
                } else if mx <= EDGE {
                    (win_w - o, win_w - o)
                } else {
                    (0.0, mx)
                };
                if warp != 0.0 {
                    ws.press_mouse = Some(Vec2::new(pm + warp, 0.0));
                    warp_to = Some((new_mx, my));
                }
                let raw = (base + (mx - pm) * self.step * speed).clamp(self.min, self.max);
                // step 吸附：值按 `step` 步进（拖动时可见台阶；step ≤ 0 = 连续）。
                let v = if self.step > 0.0 {
                    (raw / self.step).round() * self.step
                } else {
                    raw
                };
                *self.value = v;
                *edit_text = fmt_step(v, self.step);
            }
        }
        if let Some((nx, ny)) = warp_to {
            ui.set_cursor_position(nx, ny);
        }
        // 光标：手柄悬停/拖拽 → ↔（EwResize）；文本框 → 内置 I 型
        if grip_hit || dragging {
            ui.set_cursor(UiCursor::EwResize);
        }
        // 文本框：打字写入 edit_text，随后屏蔽非数字输入并解析回数值
        ui.text_input_at(self.id /* 内部处理 id_for */, text_rect, edit_text);
        edit_text.retain(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == '+' || c == ' ');
        // 输入模式：**仅首次聚焦时全选**（之后可正常用鼠标部分选择文本；
        // 失焦后 focused_prev 复位，下次聚焦再全选）。
        let now_focused = ui
            .state()
            .focused
            .as_ref()
            .is_some_and(|f| f.as_str() == id_for.as_str());
        {
            let ws = ui.state_mut().widget(&id_for);
            let just_focused = now_focused && !ws.focused_prev;
            ws.focused_prev = now_focused;
            if just_focused {
                ws.sel_anchor = Some(0);
                ws.caret = edit_text.chars().count();
            }
        }
        if let Ok(v) = edit_text.trim().parse::<f32>() {
            *self.value = v.clamp(self.min, self.max);
        }
        // 内部管理：编辑缓冲写回持久（聚焦时）；失焦清空（显示由 value 派生）。
        if self.text.is_none() {
            let ws = ui.state_mut().widget(&id_for);
            ws.input_text = if now_focused {
                Some(owned.take().unwrap_or_default())
            } else {
                None
            };
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
