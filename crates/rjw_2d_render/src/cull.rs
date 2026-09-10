//! 剔除：可见性判定 + 对**命令索引数组**过滤。
//!
//! 设计要点（职责分离）：
//! - 剔除**只操作索引数组**（`Vec<usize>`）——不可见命令的下标被移除，命令数据不动；
//! - 本模块**不依赖 GPU**，纯几何与纯判定可单测；
//! - [`Render2D`](crate::Render2D) 只持有 [`Culler`] 并调用它，自身不含剔除逻辑。
//!
//! 坐标系：世界坐标（原点在视口中心、Y+ 向下），与 `rjw_transform::Camera2D` 一致。

use glam::{Mat4, Vec2};
use rjw_transform::{Camera2D, Rect, Transform2D};

use crate::data::SpriteRect;

// ─── 剔除模式 ─────────────────────────────────────────────────

/// 剔除模式（单一状态，无隐式联动）。
///
/// 旧 API 的 `culling: bool` + `cull_pred: Option<..>` 双字段会出现
/// "`set_culling(false)` 但判定闭包仍在"的矛盾态；此处合并为一个枚举。
pub enum Cull {
    /// 关闭剔除（默认）。
    Off,
    /// 使用 [`Culler::set_viewport`] 缓存的视口矩形（由 MVP 反推，正交相机下正确）。
    Viewport,
    /// 固定的世界矩形（如某块地图区域 / 冻结的相机视野）。
    Rect(Rect),
    /// 任意判定闭包：`f(世界 AABB) -> bool`，**可见返回 `true`**。
    Fn(Box<dyn Fn(&Rect) -> bool + Send + Sync>),
}

impl Default for Cull {
    #[inline]
    fn default() -> Self {
        Cull::Off
    }
}

impl std::fmt::Debug for Cull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cull::Off => f.write_str("Cull::Off"),
            Cull::Viewport => f.write_str("Cull::Viewport"),
            Cull::Rect(r) => write!(f, "Cull::Rect({r:?})"),
            Cull::Fn(_) => f.write_str("Cull::Fn(..)"),
        }
    }
}

impl From<Rect> for Cull {
    #[inline]
    fn from(r: Rect) -> Self {
        Cull::Rect(r)
    }
}

/// 以 2D 相机驱动剔除（冻结其当前 `view_aabb()`，不持有相机引用）。
impl From<&Camera2D> for Cull {
    #[inline]
    fn from(cam: &Camera2D) -> Self {
        Cull::Rect(cam.view_aabb())
    }
}

// ─── 剔除器 ───────────────────────────────────────────────────

/// 剔除器：持有模式与视口缓存，提供可见性判定与索引数组过滤。
#[derive(Debug)]
pub struct Culler {
    mode: Cull,
    /// `Cull::Viewport` 使用的世界矩形（由 `Render2D::set_mvp` 刷新）。
    viewport: Rect,
}

impl Default for Culler {
    #[inline]
    fn default() -> Self {
        Self {
            mode: Cull::Off,
            viewport: Rect::ZERO,
        }
    }
}

impl Culler {
    /// 以模式构造（`Viewport` 模式的视口由调用方随后 `set_viewport`）。
    #[inline]
    pub fn new(mode: Cull) -> Self {
        Self {
            mode,
            viewport: Rect::ZERO,
        }
    }

    /// 设置模式（链式）。
    #[inline]
    pub fn set(&mut self, mode: Cull) -> &mut Self {
        self.mode = mode;
        self
    }

    #[inline]
    pub fn mode(&self) -> &Cull {
        &self.mode
    }

    /// 刷新 `Cull::Viewport` 使用的世界矩形（[`viewport_world_rect`] 的结果）。
    #[inline]
    pub fn set_viewport(&mut self, viewport: Rect) -> &mut Self {
        self.viewport = viewport;
        self
    }

    #[inline]
    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    /// 是否关闭剔除（关闭时 [`Self::retain`] 直接返回，零判定开销）。
    #[inline]
    pub fn is_off(&self) -> bool {
        matches!(self.mode, Cull::Off)
    }

    /// 世界 AABB 是否可见。
    #[inline]
    pub fn visible(&self, aabb: &Rect) -> bool {
        match &self.mode {
            Cull::Off => true,
            Cull::Viewport => self.viewport.intersects(aabb),
            Cull::Rect(r) => r.intersects(aabb),
            Cull::Fn(f) => f(aabb),
        }
    }

    /// 就地过滤**索引数组**（保序）：`aabb_of(i)` 返回 `None` 的命令**不参与剔除**（保留）。
    ///
    /// `Render2D` 中只有 Sprite 命令提供 AABB（动态 Mesh / StaticMesh / Custom 恒保留），
    /// 与原实现"仅 Sprite 被剔除"的行为一致。
    #[inline]
    pub fn retain(&self, order: &mut Vec<usize>, aabb_of: impl Fn(usize) -> Option<Rect>) {
        if self.is_off() {
            return;
        }
        order.retain(|&i| match aabb_of(i) {
            Some(aabb) => self.visible(&aabb),
            None => true,
        });
    }
}

// ─── 纯几何 ───────────────────────────────────────────────────

/// `Transform2D` → 列主序 2D 模型矩阵（与 `InstanceData::from_sprite` 的推导一致）。
#[inline]
pub fn transform2d_model(t: &Transform2D) -> Mat4 {
    let (sin, cos) = t.rotation.sin_cos();
    Mat4::from_cols_array_2d(&[
        [cos * t.scale.x, sin * t.scale.x, 0.0, 0.0],
        [-sin * t.scale.y, cos * t.scale.y, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [t.pos.x, t.pos.y, 0.0, 1.0],
    ])
}

/// 精灵四角经 `model` 变换后的**世界 AABB**（保守：旋转取包围盒）。
#[inline]
pub fn sprite_world_aabb(rect: &SpriteRect, model: &Mat4) -> Rect {
    let tl = rect.mesh_tl;
    let wh = rect.mesh_wh;
    let corners = [
        tl,
        Vec2::new(tl.x + wh.x, tl.y),
        Vec2::new(tl.x, tl.y + wh.y),
        tl + wh,
    ];
    let mut pts = [Vec2::ZERO; 4];
    for (i, c) in corners.iter().enumerate() {
        let v = *model * glam::Vec4::new(c.x, c.y, 0.0, 1.0);
        pts[i] = Vec2::new(v.x / v.w, v.y / v.w);
    }
    Rect::from_point_slice(&pts)
}

/// 视口世界矩形：由 MVP 逆变换 clip 空间四角得到（正交相机下 z 取 0 即可）。
#[inline]
pub fn viewport_world_rect(mvp: &Mat4) -> Rect {
    let inv = mvp.inverse();
    let corners = [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)];
    let mut pts = [Vec2::ZERO; 4];
    for (i, (cx, cy)) in corners.iter().enumerate() {
        let v = inv * glam::Vec4::new(*cx, *cy, 0.0, 1.0);
        pts[i] = Vec2::new(v.x / v.w, v.y / v.w);
    }
    Rect::from_point_slice(&pts)
}

// ─── 单元测试 ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(tl: (f32, f32), wh: (f32, f32)) -> SpriteRect {
        SpriteRect::new(tl, wh)
    }

    /// 平移变换下 AABB 精确等于世界矩形。
    #[test]
    fn sprite_aabb_matches_translation() {
        let r = rect((-10.0, -20.0), (40.0, 30.0));
        let m = transform2d_model(&Transform2D::IDENTITY.with_pos(Vec2::new(100.0, 50.0)));
        let a = sprite_world_aabb(&r, &m);
        assert_eq!(a, Rect::new(90.0, 30.0, 40.0, 30.0));
    }

    /// 旋转变换下 AABB 为保守包围盒（不丢角点）。
    #[test]
    fn sprite_aabb_is_conservative_under_rotation() {
        let r = rect((-8.0, -8.0), (16.0, 16.0));
        let m = transform2d_model(&Transform2D::IDENTITY.with_rot(std::f32::consts::FRAC_PI_4));
        let a = sprite_world_aabb(&r, &m);
        for c in [
            Vec2::new(-8.0, -8.0),
            Vec2::new(8.0, -8.0),
            Vec2::new(-8.0, 8.0),
            Vec2::new(8.0, 8.0),
        ] {
            let v = m * glam::Vec4::new(c.x, c.y, 0.0, 1.0);
            let p = Vec2::new(v.x, v.y);
            assert!(a.contains_point(p), "旋转后角点 {p:?} 应落在 AABB {a:?} 内");
        }
    }

    /// 单位 MVP（clip [-1,1]² = 世界 [-1,1]²）反推视口。
    #[test]
    fn viewport_from_identity_mvp() {
        assert_eq!(
            viewport_world_rect(&Mat4::IDENTITY),
            Rect::new(-1.0, -1.0, 2.0, 2.0)
        );
    }

    /// `retain`：不可见项被移除，返回 `None` 的项恒保留，且保持原顺序。
    #[test]
    fn retain_filters_invisible_and_keeps_none() {
        let culler = Culler::new(Cull::Rect(Rect::new(0.0, 0.0, 100.0, 100.0)));
        let mut order = vec![0, 1, 2, 3];
        culler.retain(&mut order, |i| match i {
            0 => Some(Rect::new(0.0, 0.0, 10.0, 10.0)),     // 完全可见
            1 => Some(Rect::new(500.0, 500.0, 10.0, 10.0)), // 完全不可见
            2 => None,                                      // 不参与剔除
            _ => Some(Rect::new(90.0, 90.0, 20.0, 20.0)),   // 部分相交
        });
        assert_eq!(order, vec![0, 2, 3]);
    }

    /// `Off` 时不产生任何判定（`aabb_of` 不会被调用）。
    #[test]
    fn off_never_calls_aabb_fn() {
        let culler = Culler::new(Cull::Off);
        assert!(culler.is_off());
        let mut order = vec![0, 1];
        culler.retain(&mut order, |_| {
            panic!("culling off: aabb_of must not be called")
        });
        assert_eq!(order, vec![0, 1]);
    }

    /// 判定闭包模式 + 相机模式（`From<&Camera2D>`）+ `Viewport` 模式。
    #[test]
    fn fn_camera_and_viewport_modes() {
        let culler = Culler::new(Cull::Fn(Box::new(|a: &Rect| a.x > 0.0)));
        assert!(culler.visible(&Rect::new(1.0, 0.0, 1.0, 1.0)));
        assert!(!culler.visible(&Rect::new(-5.0, 0.0, 1.0, 1.0)));

        let cam = Camera2D::new(Vec2::new(800.0, 600.0));
        let c = Culler::new(Cull::from(&cam));
        assert!(!c.is_off());
        assert!(c.visible(&cam.view_aabb()));

        let mut vp = Culler::new(Cull::Viewport);
        vp.set_viewport(Rect::new(-50.0, -50.0, 100.0, 100.0));
        assert!(vp.visible(&Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert!(!vp.visible(&Rect::new(200.0, 200.0, 10.0, 10.0)));
    }
}
