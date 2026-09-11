//! UI 层（feature = `ui`）：引擎持有跨帧 UI 状态与主题，`Frame::ui` 负责每帧起止。

use rjw_ui::{Theme, UiState};

/// 引擎持有的 UI 状态。
///
/// - `state`：跨帧持久（焦点 / 滚动 / 拖拽 / 输入内容 / 窗口尺寸…），**必须**跨帧复用；
/// - `theme`：最近一次 `Frame::ui` 传入的主题（不带 theme 的后续帧可复用）。
#[derive(Default)]
pub struct UiLayer {
    pub state: UiState,
    pub theme: Theme,
}

