//! **滚动容器 builder**（[`ScrollArea`]）：把"可视区 + 两条轴的溢出策略"收成一条责任链。
//!
//! # 维护者笔记
//!
//! - **它只是 [`Ui::scroll_axes_at`](crate::Ui::scroll_axes_at) 的语法糖**：不持有任何
//!   状态（滚动偏移跨帧持久于 [`UiState::scrolls`](crate::UiState::scrolls)），
//!   不参与绘制顺序 —— 真正的沙箱 / 裁剪 / 滚动条都在 `Ui::scroll_at_axes` 里。
//!   于是"改滚动行为"永远只改那一处，本文件只负责**默认值与可读性**。
//! - **两条轴的默认值 = 公开的 [`Ui::scroll_at`](crate::Ui::scroll_at)**：纵向
//!   [`ScrollMode::Scroll`]（滚动条）/ 横向 [`ScrollMode::ClipOnly`]（只裁不滚，长行按
//!   视口宽折行）。要"长行不折行 + 横向滚动条"就必须显式 `.hscroll(true)` ——
//!   `h == Scroll` 时沙箱不再上报可用宽（见 `scroll_axes_avail_w`），这是横向能滚的前提。
//! - **入参用 [`ScrollParam`]**（`bool` 或 [`ScrollMode`]）：与
//!   [`WindowBuilder`](crate::WindowBuilder) 的 `.vscroll` / `.hscroll` 同一套语义，
//!   两处不会漂移。
//! - 它是**容器**：`show` 的闭包拿到 [`Scroll`](crate::Scroll)，其上也实现了
//!   [`UiAdd`](crate::UiAdd)，于是 `label` / `row` / 嵌套 `scroll_area` 都能直接用。

use glam::Vec2;

use crate::ScrollMode;
use crate::draw::{Position, Size};
use crate::ui::{Scroll, ScrollOutcome, ScrollParam, Ui};

/// **滚动容器 builder**（[`Ui::scroll_area`] / [`UiAdd::scroll_area`](crate::UiAdd::scroll_area) 返回）。
///
/// ```ignore
/// ui.scroll_area("log", Size::Logical(Vec2::new(240.0, 320.0)))
///     .pos(Vec2::new(8.0, 8.0))   // 可选：可视区左上角（相对当前容器内容原点）
///     .vscroll(true)              // 竖向滚动条（默认值）
///     .hscroll(true)              // 横向滚动条（长行不折行）
///     .show(|s| { s.label("一行很长的内容 …"); });
/// ```
pub struct ScrollArea<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
    id: &'ui str,
    pos: Position,
    size: Size<Vec2>,
    v: ScrollMode,
    h: ScrollMode,
}

impl<'ui, 'a> ScrollArea<'ui, 'a> {
    /// 新建（[`Ui::scroll_area`] 用的入口）：`id` = 滚动偏移状态键（跨帧持久），
    /// `size` = **视口**尺寸（内容超出部分靠滚动看）。
    pub(crate) fn new(ui: &'ui mut Ui<'a>, id: &'ui str, size: Size<Vec2>) -> Self {
        Self {
            ui,
            id,
            pos: Position::Logical(Vec2::ZERO),
            size,
            // 默认与 `Ui::scroll_at` 逐像素一致：纵向滚动 + 横向裁切。
            v: ScrollMode::Scroll,
            h: ScrollMode::ClipOnly,
        }
    }

    /// 可视区左上角（[`Position`]：`Logical`（默认 `(0,0)`，× scale）/ `Physical` 原样；
    /// 相对当前容器内容原点）。
    pub fn pos(mut self, p: impl Into<Position>) -> Self {
        self.pos = p.into();
        self
    }

    /// **垂直轴的溢出策略**（`bool` / [`ScrollMode`]，见 [`ScrollParam`]）：
    /// `true` = 视口 + 滚动条 + 滚轮；`false` = 内容撑开（不裁不滚）。
    pub fn vscroll(mut self, mode: impl ScrollParam) -> Self {
        self.v = mode.scroll_mode();
        self
    }

    /// **水平轴的溢出策略**：`true` = 视口 + 滚动条（内容保持**自然宽、不折行**）；
    /// `false` = 内容按视口宽**折行 / 压缩**。
    pub fn hscroll(mut self, mode: impl ScrollParam) -> Self {
        self.h = mode.scroll_mode();
        self
    }

    /// 终结：录制内容并返回 [`ScrollOutcome`]（视口 + 内容结算尺寸）。
    pub fn show(self, f: impl FnOnce(&mut Scroll<'_, '_>)) -> ScrollOutcome {
        let Self { ui, id, pos, size, v, h } = self;
        ui.scroll_axes_at(pos, size, id, v, h, f)
    }
}
