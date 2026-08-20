//! 内置**组合控件**（由基础原语组合而成，只依赖 `Ui` 公开 API；基础原语在
//! crate 上一级，如 [`Ui::modal_at`](crate::ui::Ui::modal_at)）。
//!
//! - [`FontModal`]：字体切换模态对话框（Input + PreviewInput + 确定/取消右对齐）；
//! - [`NumberInput`]：数字输入框（文本框 + 拖拽调值手柄，含 warp 与输入模式）。
//!
//! 这些实现同时是"跨 crate 自定义控件"的真实范例（`crate::widget` 模块文档有
//! 更简的 doctest 模板）。

pub mod fontmodal;
pub mod numberinput;

pub use fontmodal::FontModal;
pub use numberinput::NumberInput;
