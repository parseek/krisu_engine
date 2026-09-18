//! **View 沙箱**：裁剪 / 坐标 / 可用宽度的闭包作用域（入口 [`crate::Ui::view_at`]）。
//!
//! 把"进入一个视图空间"封装成显式类型：进入沙箱 = 压栈（裁剪层、坐标原点、可用宽度），
//! 沙箱内录制的命令在弹出时统一平移 `pos`，恢复外层状态。沙箱是 **ScrollView**（
//! [`crate::Ui::scroll_at`]、文本编辑框）与未来严格窗口裁剪的公共底座。
//!
//! 两种模式（[`ViewMode`]）：
//! - [`ViewMode::Expand`]：**不裁剪**（默认）。内容自然尺寸，可溢出沙箱并撑大外层
//!   容器；沙箱仅提供"可用宽度"提示（`avail_w`），供 `LimitedInParent` 控件自洽
//!   （自动换行 / "…"省略）。反例语义：无 Scroll 的普通容器不产生强制裁剪层，
//!   noclip 绘制的内容画出界。
//! - [`ViewMode::Clip`]：**严格裁剪**。内容超出可视区被裁（**强制层**，所有绘制命令
//!   含 noclip 变体都服从），沙箱外的鼠标命中失效（命中裁剪）。
//!
//! 裁剪分层（详见 [`crate::draw`] 与 `ui.rs` 的 `push_*_noclip` 文档）：
//! - **强制层**（本模块 `clip_for_view` 计算）：ScrollView 可视区 / Clip 沙箱；
//! - **软层**（内容裁剪）：控件自身内容边界，由调用方显式传参，自洽控件可跳过。

use crate::draw::intersect_rect;
use crate::ui::{Ui, UiAdd};
use rjw_transform::{Rect, Transform2D};

/// View 沙箱模式（见[模块文档](self)）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    /// 不裁剪（默认）：内容自然尺寸，可溢出沙箱并撑大外层容器；
    /// 沙箱仅提供"可用宽度"提示（`avail_w`）。
    Expand,
    /// 严格裁剪：内容超出可视区被裁（强制层），沙箱外命中失效。
    Clip,
}

/// 计算进入沙箱后的**强制裁剪层**：
/// - [`ViewMode::Expand`]：不产生强制层（外层 `outer` 原样传递）；
/// - [`ViewMode::Clip`]：`outer ∩ view_abs`（无外层 = `view_abs`）。
///
/// 纯几何（可单测）。`view_abs` = 沙箱**绝对**逻辑矩形（裁剪层坐标系为绝对）。
#[inline]
pub(crate) fn clip_for_view(outer: Option<Rect>, view_abs: Rect, mode: ViewMode) -> Option<Rect> {
    match mode {
        ViewMode::Expand => outer,
        ViewMode::Clip => match outer {
            Some(c) => intersect_rect(&c, &view_abs),
            None => Some(view_abs),
        },
    }
}

/// 沙箱闭包上下文（经 [`crate::ui::UiAdd`] 提供容器内全部便捷方法）。
pub struct ViewCtx<'ui, 'a> {
    pub(crate) ui: &'ui mut Ui<'a>,
}

/// **环境裁剪层 → 批次的屏幕 scissor**（纯函数，可单测）。
///
/// 批次的顶点是**窗口局部坐标**，提交时经 `tf`（屏幕固定变换 + 窗口 FX）映射到屏幕；
/// 而环境裁剪层 `clip` 是**绝对屏幕坐标**（不含 FX）。所以：
///
/// 1. 先把 `clip` 减窗口原点 `anchor`（回到顶点所在空间）；
/// 2. 再用 `tf` 映射回屏幕（`Rect::transform` 取四角保守 AABB）。
///
/// - `tf` 是纯平移（**绝大多数帧**）⇒ 结果**逐位等于** `clip`（只是减法加法各一次）；
/// - `tf` 含缩放 ⇒ 裁剪区跟着缩放（与几何一致）；
/// - `tf` 含旋转 ⇒ 取保守 AABB（**宁可多画一点，绝不误裁**）。
pub(crate) fn batch_scissor(clip: Rect, anchor: glam::Vec2, tf: &Transform2D) -> Rect {
    let local = Rect::new(clip.x - anchor.x, clip.y - anchor.y, clip.w, clip.h);
    local.transform(tf)
}

impl<'ui, 'a> UiAdd<'a> for ViewCtx<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_passes_outer_clip_through() {
        let outer = Some(Rect::new(10.0, 20.0, 300.0, 200.0));
        let view = Rect::new(50.0, 60.0, 100.0, 80.0);
        // Expand：不产生强制层 → 外层原样
        assert_eq!(clip_for_view(outer, view, ViewMode::Expand), outer);
        // Expand + 无外层 → 仍无强制层（普通容器不裁切）
        assert_eq!(clip_for_view(None, view, ViewMode::Expand), None);
    }

    #[test]
    fn clip_intersects_with_outer() {
        let outer = Some(Rect::new(10.0, 20.0, 300.0, 200.0));
        let view = Rect::new(50.0, 60.0, 100.0, 80.0);
        // Clip：外层 ∩ 可视区 = 可视区（在层内）
        assert_eq!(clip_for_view(outer, view, ViewMode::Clip), Some(view));
        // 嵌套 Clip：外层更小 → 取交集
        let outer_small = Some(Rect::new(60.0, 70.0, 40.0, 30.0));
        assert_eq!(
            clip_for_view(outer_small, view, ViewMode::Clip),
            Some(Rect::new(60.0, 70.0, 40.0, 30.0))
        );
        // 不相交 → None（全裁）
        let outer_away = Some(Rect::new(500.0, 500.0, 10.0, 10.0));
        assert_eq!(clip_for_view(outer_away, view, ViewMode::Clip), None);
        // Clip + 无外层 → 可视区本身
        assert_eq!(clip_for_view(None, view, ViewMode::Clip), Some(view));
    }

    /// **批次的 scissor 映射**：纯平移逐位相等；缩放跟着走；旋转取保守 AABB。
    #[test]
    fn batch_scissor_maps_through_the_batch_transform() {
        let clip = Rect::new(100.0, 50.0, 200.0, 100.0);
        let anchor = glam::Vec2::new(40.0, 30.0);
        // 纯平移（窗口原点）⇒ 与输入**逐位相等**
        let tf = Transform2D::IDENTITY.with_pos(anchor);
        assert_eq!(batch_scissor(clip, anchor, &tf), clip);
        // 平移 + 缩放 2×（锚点处）⇒ 裁剪区跟着放大
        let tf2 = Transform2D::IDENTITY
            .with_pos(anchor)
            .with_scale(glam::Vec2::splat(2.0));
        let s = batch_scissor(clip, anchor, &tf2);
        assert_eq!(s, Rect::new(160.0, 70.0, 400.0, 200.0));
        // 旋转 ⇒ 保守 AABB（比原矩形大，绝不误裁）
        let tf3 = Transform2D::IDENTITY.with_pos(anchor).with_rot(std::f32::consts::FRAC_PI_4);
        let r = batch_scissor(clip, anchor, &tf3);
        assert!(r.w > clip.w && r.h > clip.h, "旋转下取保守 AABB：{:?}", r);
    }
}
