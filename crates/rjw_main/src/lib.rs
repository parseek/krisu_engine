//! 入口基础设施：winit / 输入 / 计时的**统一重导出** + 默认窗口标题。
//!
//! 责任边界（`docs/API_DESIGN.md` §8.9）：
//! - 本 crate **只**提供"平台与窗口底座"：winit 类型、`DeltaTimer`、`KeyboardInput`、`MouseInput`；
//! - 事件循环与窗口生命周期由 [`rjw_krusie::runtime`] 的 `Engine`（`ApplicationHandler` 实现）负责，
//!   游戏侧只实现 [`rjw_krusie::App`]。
//!
//! 旧版 `rjw_main::{App, MainContext, MainHandler, run_app}` 已删除（零调用，职责与运行时重复）。

// --- Re-exports ---

pub use rjw_time::DeltaTimer;
pub use rjw_keyboard::{KeyboardInput, KeyState};
pub use rjw_mouse::{MouseInput, ScrollDelta};

pub use winit;
pub use winit::keyboard::KeyCode;
pub use winit::event::MouseButton;
pub use winit::event_loop::EventLoop;
pub use winit::event_loop::ActiveEventLoop;
pub use winit::error::EventLoopError;
pub use winit::event::WindowEvent;
pub use winit::event::DeviceEvent;
pub use winit::dpi::Size;
pub use winit::dpi::LogicalPosition;
pub use winit::dpi::LogicalSize;
pub use winit::dpi::PhysicalPosition;
pub use winit::dpi::PhysicalSize;
pub use winit::window::Window;
pub use winit::window::WindowAttributes;

use std::sync::LazyLock;

/// 取不到可执行文件名时的默认窗口标题。
pub const PRIMARY_WINDOW_TITLE_DEFAULT: &str = "rjw primary window";

/// 默认窗口标题：取当前可执行文件名（`rjw_krusie::runtime::default_title()` 使用）。
pub static PRIMARY_WINDOW_TITLE: LazyLock<String> = LazyLock::new(|| {
    match std::env::current_exe() {
        Ok(path) => match path.file_name() {
            Some(name) => name
                .to_str()
                .unwrap_or(PRIMARY_WINDOW_TITLE_DEFAULT)
                .to_owned(),
            None => PRIMARY_WINDOW_TITLE_DEFAULT.to_owned(),
        },
        Err(_) => PRIMARY_WINDOW_TITLE_DEFAULT.to_owned(),
    }
});
