//! 剔除/可见性抽象：2D 相机 → 保守 AABB；3D 相机（后续）→ 视锥体测试。

use crate::{Camera2D, Rect};

/// 可见性判定抽象（供剔除使用），**为 3D 扩展预留**：
///
/// - 2D（[`Camera2D`]）：世界可见区是（可能旋转的）矩形，保守实现为
///   [`Camera2D::view_aabb`]（AABB 相交，不误杀）；
/// - 3D（后续 `Camera3D`，正交/透视）：可见区是**视锥体**，实现
///   [`ViewCull::is_aabb_visible`] 为 6 平面测试即可，调用方（`rjw_tilemap` / `Render2D`）
///   无需感知维度差异。
pub trait ViewCull {
    /// 世界空间可见区的**保守 AABB**（粗剔除；旋转/缩放下为包围盒超集）。
    fn world_view_aabb(&self) -> Rect;

    /// 世界 AABB 是否**可能可见**（默认用 `world_view_aabb` 相交）。
    ///
    /// 3D 实现可覆写为精确的视锥体-包围盒测试（更紧、少提交）。
    fn is_aabb_visible(&self, aabb: &Rect) -> bool {
        aabb.intersects(&self.world_view_aabb())
    }
}

impl ViewCull for Camera2D {
    #[inline]
    fn world_view_aabb(&self) -> Rect {
        self.view_aabb()
    }
}
