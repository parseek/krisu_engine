use rjw_keystate::*;
use winit::dpi::PhysicalPosition;
use winit::event::WindowEvent;
use winit::event::MouseButton;

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
    pub const DEFAULT_LINE_FACTOR:f64 = 300.0;
    #[inline]
    pub fn to_pixel(&self, line_factor: Option<f64>) -> (f64, f64) {
        match self {
            Self::Pixel(f) => *f,
            Self::Line((x, y)) => {
                let line_factor = line_factor.unwrap_or(Self::DEFAULT_LINE_FACTOR);
                (*x * line_factor, *y * line_factor)
            }
        }
    }
    #[inline]
    pub fn to_line(&self, line_factor: Option<f64>) -> (f64, f64) {
        match self {
            Self::Line(f) => *f,
            Self::Pixel((x, y)) => {
                let line_factor = line_factor.unwrap_or(Self::DEFAULT_LINE_FACTOR);
                let line_factor = 1.0 / line_factor;
                (*x * line_factor, *y * line_factor)
            }
        }
    }

    #[inline]
    pub fn is_pixel(&self) -> bool {
        match self {
            Self::Pixel(_) => true,
            _ => false
        }
    }
    #[inline]
    pub fn is_line(&self) -> bool {
        match self {
            Self::Line(_) => true,
            _ => false
        }
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
    #[inline]
    #[allow(unused)]
    pub fn get_mouse_position(&self) -> (f64, f64) {
        self.mouse_position
    }
    #[inline]
    #[allow(unused)]
    pub fn get_mouse_delta(&self) -> (f64, f64) {
        self.mouse_delta
    }
    #[inline]
    #[allow(unused)]
    pub fn get_mouse_button_state(&self, button: winit::event::MouseButton) -> KeyState {
        self.mouse_buttons[mb_to_idx(button)]
    }
    #[inline]
    #[allow(unused)]
    pub fn get_mouse_wheel_delta(&self) -> (f64, f64) {
        self.mouse_wheel_delta
    }
    #[inline]
    #[allow(unused)]
    pub fn in_window(&self) -> bool {
        self.in_window
    }
    #[inline]
    #[allow(unused)]
    pub fn get_pixel_wheel(&self) -> Option<(f64, f64)> {
        self.pixel_wheel
    }
    #[inline]
    #[allow(unused)]
    pub fn get_wheel_line_delta(&self) -> (f64, f64) {
        if let Some(pixel) = self.get_pixel_wheel() {
            pixel
        } else {
            let (x, y) = self.get_mouse_wheel_delta();
            const LINE_DELTA: f64 = 15.0;
            (x * LINE_DELTA, y * LINE_DELTA)
        }
    }
    #[inline]
    #[allow(unused)]
    pub fn get_wheel_delta(&self) -> ScrollDelta {
        if let Some(d) = self.get_pixel_wheel() {
            ScrollDelta::Pixel(d)
        } else {
            let d = self.get_mouse_wheel_delta();
            ScrollDelta::Line(d)
        }
    }
    pub fn end_frame(&mut self) {
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

    #[inline]
    #[allow(unused)]
    pub fn get_mouse_button_states_iter(
        &self,
    ) -> impl Iterator<Item = (winit::event::MouseButton, KeyState)> + '_ {
        self.mouse_buttons.iter().enumerate().map(|(idx, s)| {(idx_to_mb(idx), *s)})
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
        match event {
            winit::event::DeviceEvent::MouseMotion { delta } => {
                // Frame delta is accumulated, and will be reset at the end of the frame.
                self.mouse_delta.0 += delta.0;
                self.mouse_delta.1 += delta.1;
            }
            // winit::event::DeviceEvent::Button { button, state } =>
            // winit::event::DeviceEvent::MouseWheel { delta } => {
            //     match delta {
            //         winit::event::MouseScrollDelta::LineDelta(x, y) => {
            //             self.mouse_wheel_delta.0 += *x as f64;
            //             self.mouse_wheel_delta.1 += *y as f64;
            //         }
            //         // winit::event::MouseScrollDelta::PixelDelta(pos) => {
            //         //     self.mouse_wheel_delta.0 += pos.x;
            //         //     self.mouse_wheel_delta.1 += pos.y;
            //         // }
            //         _ => {}
            //     }
            // }
            _ => {}
        }
    }
}