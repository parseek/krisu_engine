use rjw_keystate::*;
use winit::dpi::PhysicalPosition;
use winit::event::WindowEvent;
pub use rjw_keystate::KeyState;

/// 重导出鼠标按键类型（无需直接依赖 winit）。
pub use winit::event::MouseButton;

fn mb_to_idx(button: MouseButton) -> usize {
    match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(_) => 5,
    }
}

fn idx_to_mb(idx: usize) -> MouseButton {
    match idx {
        0 => MouseButton::Left,
        1 => MouseButton::Right,
        2 => MouseButton::Middle,
        3 => MouseButton::Back,
        4 => MouseButton::Forward,
        x => MouseButton::Other(x as u16),
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ScrollDelta {
    /// Touchpad
    Pixel((f64, f64)),
    Line((f64, f64)),
}

impl ScrollDelta {
    /// 行 → 像素的**唯一**换算因子（不再有第二套线因子）。
    pub const LINE_FACTOR: f64 = 300.0;

    /// 换算为像素增量（像素滚轮原样返回）。
    #[inline]
    pub fn to_pixel(&self) -> (f64, f64) {
        match self {
            Self::Pixel(f) => *f,
            Self::Line((x, y)) => (*x * Self::LINE_FACTOR, *y * Self::LINE_FACTOR),
        }
    }

    /// 换算为行增量（行滚轮原样返回）。
    #[inline]
    pub fn to_line(&self) -> (f64, f64) {
        match self {
            Self::Line(f) => *f,
            Self::Pixel((x, y)) => {
                let k = 1.0 / Self::LINE_FACTOR;
                (*x * k, *y * k)
            }
        }
    }

    #[inline]
    pub fn is_pixel(&self) -> bool {
        matches!(self, Self::Pixel(_))
    }
    #[inline]
    pub fn is_line(&self) -> bool {
        matches!(self, Self::Line(_))
    }
}


#[derive(Default)]
pub struct MouseInput {
    mouse_position: (f64, f64),
    mouse_delta: (f64, f64),
    /// Maps winit::event::MouseButton -> KeyState
    mouse_buttons: [KeyState; 6], // idx=5 always Released
    mouse_wheel_delta: (f64, f64), // LineDelta (x, y), accumulated
    pixel_wheel: Option<(f64, f64)>, // PixelDelta, accumulated per frame
    in_window: bool,
}

impl MouseInput {
    /// 鼠标按键状态。
    #[inline]
    pub fn button(&self, button: MouseButton) -> KeyState {
        self.mouse_buttons[mb_to_idx(button)]
    }

    /// 光标位置（**物理像素**，左上原点）。
    #[inline]
    pub fn pos_px(&self) -> glam::Vec2 {
        glam::Vec2::new(self.mouse_position.0 as f32, self.mouse_position.1 as f32)
    }

    /// 本帧鼠标位移（`DeviceEvent::MouseMotion` 累加；帧末归零）。
    #[inline]
    pub fn motion(&self) -> glam::Vec2 {
        glam::Vec2::new(self.mouse_delta.0 as f32, self.mouse_delta.1 as f32)
    }

    /// 本帧滚轮增量（像素 / 行）。
    #[inline]
    pub fn wheel(&self) -> ScrollDelta {
        if let Some(d) = self.pixel_wheel {
            ScrollDelta::Pixel(d)
        } else {
            ScrollDelta::Line(self.mouse_wheel_delta)
        }
    }

    /// 本帧滚轮增量（**行**为单位；像素滚轮按 [`ScrollDelta::to_line`] 换算）。
    #[inline]
    pub fn wheel_lines(&self) -> (f64, f64) {
        match self.pixel_wheel {
            Some(pixel) => pixel,
            None => {
                let (x, y) = self.mouse_wheel_delta;
                (x * ScrollDelta::LINE_FACTOR, y * ScrollDelta::LINE_FACTOR)
            }
        }
    }

    /// 光标是否在窗口内。
    #[inline]
    pub fn in_window(&self) -> bool {
        self.in_window
    }

    /// 全部鼠标按键状态遍历（`(按键, 状态)`）。
    pub fn buttons(&self) -> impl Iterator<Item = (winit::event::MouseButton, KeyState)> + '_ {
        self.mouse_buttons.iter().enumerate().map(|(idx, s)| (idx_to_mb(idx), *s))
    }

    /// 帧末结算（引擎每帧调用；用户不应自行调用）。
    #[inline]
    pub fn next_frame(&mut self) {
        self.end_frame();
    }

    /// **调试注入**（脚本化复现交互；见 `docs/DEBUGGING.md`）：直接把"光标位置 +
    /// 左键状态"写进快照——不需要真实鼠标 / 不需要窗口事件。
    ///
    /// 用途：无鼠标环境（CI / 远程）复现"拖动没反应""点击穿透"这类**交互**问题，
    /// 或用脚本驱动一条确定的鼠标轨迹（回归测试）。
    /// `left` 传入合成的 `KeyState`（含边沿：按下帧用 `down_edge`、按住用 `pressed`、
    /// 释放帧用 `up_edge`——见 `Ctx::debug_inject_mouse` 的自动合成）。
    pub fn debug_inject(&mut self, pos_px: (f64, f64), left: KeyState) {
        self.mouse_position = pos_px;
        self.mouse_buttons[mb_to_idx(MouseButton::Left)] = left;
        self.in_window = true;
    }

    /// **调试注入（含边沿合成）**：`down` = 本帧左键是否按住，`was_down` = 上一帧是否按住
    /// ——本函数按与真实设备一致的状态机语义合成 `KeyState`（按下帧带 `down_edge`、
    /// 按住给 `pressed`、释放帧带 `up_edge`），因此命中 / 拖拽 / 点击逻辑无差别生效。
    ///
    /// 供 `rjw_krusie::Ctx::debug_inject_mouse` 使用（脚本化复现交互；见 `docs/DEBUGGING.md`）。
    pub fn debug_inject_press(&mut self, pos_px: (f64, f64), down: bool, was_down: bool) {
        let state = match (was_down, down) {
            (false, true) => KEY_STATE_DOWN_TRUE_EDGE,
            (true, true) => KEY_STATE_PRESSING,
            (true, false) => KEY_STATE_UP_TRUE_EDGE,
            (false, false) => KEY_STATE_RELEASED,
        };
        self.debug_inject(pos_px, state);
    }

    /// 帧末结算实现（**非公开**：引擎每帧经 [`Self::next_frame`] 调用）。
    pub(crate) fn end_frame(&mut self) {
        for button_state in self.mouse_buttons.iter_mut() {
            *button_state = button_state.off_edge();
            if button_state.sudden_up()
            {
                *button_state = KEY_STATE_UP_TRUE_EDGE
            }
        }
        self.mouse_delta = (0.0, 0.0);
        self.mouse_wheel_delta = (0.0, 0.0);
        self.pixel_wheel = None;
    }

    pub fn window_event(&mut self, event: &winit::event::WindowEvent) {
        match event {
            winit::event::WindowEvent::CursorMoved { position, .. } => {
                // Fun fact: If you move the mouse from inside the window to outside the window, you will not get a CursorMoved event,
                //     but if you do so while you are holding down a mouse button, you will get a CursorMoved event. This is because
                //     the OS sends mouse move events to the window that has captured the mouse, which is usually the window that has
                //     the mouse button pressed.
                self.mouse_position = (position.x, position.y);
            }
            #[allow(unused)]
            winit::event::WindowEvent::MouseWheel {
                device_id,
                delta,
                phase,
            } => {
                match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        self.mouse_wheel_delta.0 += *x as f64;
                        self.mouse_wheel_delta.1 += *y as f64;
                    }
                    winit::event::MouseScrollDelta::PixelDelta(pos) => {
                        let PhysicalPosition{ x, y } = *pos;
                        if let Some((ox, oy)) = self.pixel_wheel {
                            self.pixel_wheel = Some((x+ox, y+oy));
                        } else {
                            self.pixel_wheel = Some((x, y));
                        }
                    }
                    _ => {}
                }
            }
            #[allow(unused)]
            winit::event::WindowEvent::CursorEntered { device_id } => {
                self.in_window = true;
            }
            #[allow(unused)]
            winit::event::WindowEvent::CursorLeft { device_id } => {
                self.in_window = false;
            }
            #[allow(unused)]
            WindowEvent::MouseInput {
                device_id,
                state,
                button,
            } => {
                let button_state = &mut self.mouse_buttons[mb_to_idx(*button)];
                let new_state = match state {
                    winit::event::ElementState::Pressed => {
                        if button_state.pressed() {
                            KEY_STATE_DOWN_EDGE
                        } else {
                            KEY_STATE_DOWN_TRUE_EDGE
                        }
                    }
                    winit::event::ElementState::Released => {
                        if button_state.released() {
                            KEY_STATE_UP_EDGE
                        } else {
                            if button_state.down_true_edge() {
                                button_state.set_sudden_up()
                            }
                            else {
                                KEY_STATE_UP_TRUE_EDGE
                            }
                        }
                    }
                };
                *button_state = new_state;
            }
            _ => {}
        }
    }
    pub fn device_event(&mut self, event: &winit::event::DeviceEvent) {
        // 只关心鼠标运动（`MouseMotion`）：帧内累加，帧末（`next_frame`）归零。
        if let winit::event::DeviceEvent::MouseMotion { delta } = event {
            self.mouse_delta.0 += delta.0;
            self.mouse_delta.1 += delta.1;
        }
    }
}