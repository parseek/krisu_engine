//! `rjw_krusie::runtime`：模块化应用骨架。
//!
//! | 模块 | 责任 | 关键类型 |
//! |---|---|---|
//! | [`config`] | 窗口 / 渲染 / 清屏 / 后台策略 | [`AppConfig`] / [`Background`] / [`Vsync`] |
//! | [`game`] | 应用 trait 与主循环 | [`App`] / [`run`] / [`run_with`] |
//! | [`ctx`] | 本帧宿主事实 + 取帧 | [`Ctx`] / [`Escape`] |
//! | [`frame`] | 本帧渲染面 | [`Frame`] |
//! | [`gfx`] | 长期 GPU 能力对象 | [`Gfx`] |
//! | [`window`] | 窗口身份（多窗口就绪） | [`WindowId`] |
//! | `layers` | 可选层（按 feature） | `ui` / （后续）`text` / `tilemap` |
//!
//! 依赖方向：`runtime` 只向下用子系统门面（`rjw_render` / `rjw_2d_render` / `rjw_transform` / …），
//! 子系统不反向依赖 runtime。
//!
//! 模块化约定：每个模块**单一职责**、可单独使用（`Ctx` 可 headless 构造用于测试；
//! `Gfx` 无帧概念；`Frame` 只描述渲染面），无隐藏全局状态。

pub mod config;
pub mod ctx;
pub mod frame;
pub mod game;
pub mod gfx;
pub mod layers;
pub mod window;

pub use config::{AppConfig, Background, Clear, RenderConfig, ViewportBorders, Vsync};
pub use ctx::{Ctx, Escape};
pub use frame::Frame;
pub use game::{run, run_with, App};
pub use gfx::Gfx;
pub use window::WindowId;

pub use rjw_render::{FrameSource, Never};

/// 默认窗口标题（取自可执行文件名，与 `rjw_main::PRIMARY_WINDOW_TITLE` 一致）。
pub(crate) fn default_title() -> String {
    rjw_main::PRIMARY_WINDOW_TITLE.clone()
}
