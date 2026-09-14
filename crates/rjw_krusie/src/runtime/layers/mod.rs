//! 可选渲染层：按 feature 提供「引擎拥有、`Frame` 便捷访问」的子系统。
//!
//! 每层只做**接线**（谁拥有状态、每帧怎么起止），不含渲染实现——实现仍在各子系统 crate。

#[cfg(feature = "ui")]
pub mod ui;
#[cfg(feature = "ui")]
pub mod ui_backend;
