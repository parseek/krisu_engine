use std::ops::{Deref, DerefMut};

use glam::{Mat4, Vec2, Vec4};

use crate::{Rect, Transform2D};

/// 2D 正交相机 = **矩形区域**（[`Self::region`]，屏幕像素）+ **2D 变换**（[`Self::transform`]，
/// 相机在世界中的位姿）。本类型不再自己实现位姿/坐标数学——一律经 [`Transform2D`]。
///
/// # 坐标系
///
/// ```text
/// ┌   T   ┐   O = 画面中心（世界原点映射到 `region` 中心）
///     |       X+ 右 / Y+ 下
/// L - O - R
///     |
/// └   B   ┘
/// ```
///
/// # 缩放语义
///
/// `transform.scale` = **世界单位 / 像素**（越大视野越广）。用户惯用的 `zoom`（越大越放大）
/// 是它的派生视图：`zoom = 1 / scale`，见 [`Self::zoom`] / [`Self::set_zoom`]。
/// **不再有第二份 zoom 状态**。
///
/// # 位姿方法
///
/// `Camera2D` 经 [`Deref`] / [`DerefMut`] 直接暴露 [`Transform2D`] 的方法：
/// `cam.move_by(..)`、`cam.move_local(..)`、`cam.transform_point(..)`、`cam.inverse()` …
/// 字段赋值写 `cam.transform.pos = ..`（`Deref` 不穿透字段）。
#[derive(Debug, Clone, Copy)]
pub struct Camera2D {
    /// 屏幕像素矩形：画面在窗口中的位置与大小（左上原点）。
    pub region: Rect,
    /// 相机在世界中的变换：`pos` = 位置、`rotation` = 朝向、`scale` = 世界单位/像素。
    pub transform: Transform2D,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self {
            region: Rect::new(0.0, 0.0, 1.0, 1.0),
            transform: Transform2D::IDENTITY,
        }
    }
}

impl Deref for Camera2D {
    type Target = Transform2D;
    #[inline]
    fn deref(&self) -> &Transform2D {
        &self.transform
    }
}

impl DerefMut for Camera2D {
    #[inline]
    fn deref_mut(&mut self) -> &mut Transform2D {
        &mut self.transform
    }
}

impl Camera2D {
    /// 以画面矩形构造（位姿 = [`Transform2D::IDENTITY`]：zoom 1、无旋转、世界原点居中）。
    #[inline]
    pub fn new(region: Rect) -> Self {
        Self { region, transform: Transform2D::IDENTITY }
    }

    /// 全窗口画面：`region = (0, 0, w, h)`（接受 `Vec2` 或 `(w, h)`）。
    #[inline]
    pub fn full(size: impl Into<Vec2>) -> Self {
        let s = size.into();
        Self::new(Rect::new(0.0, 0.0, s.x, s.y))
    }

    /// 当前画面矩形（屏幕像素）。
    #[inline]
    pub fn region(&self) -> Rect {
        self.region
    }

    /// 设置画面矩形（分屏 / 画中画）。
    #[inline]
    pub fn set_region(&mut self, region: Rect) {
        self.region = region;
    }

    /// 派生缩放视图：`1 / transform.scale`（越大越放大）。**不是状态**。
    #[inline]
    pub fn zoom(&self) -> Vec2 {
        Vec2::new(1.0 / self.transform.scale.x, 1.0 / self.transform.scale.y)
    }

    /// 设置缩放（越大越放大）：写入 `transform.scale = 1 / zoom`。
    #[inline]
    pub fn set_zoom(&mut self, zoom: impl Into<Vec2>) {
        let z = zoom.into();
        self.transform.scale = Vec2::new(1.0 / z.x, 1.0 / z.y);
    }

    /// View-Projection 矩阵（列主序），可直接交给渲染器。
    ///
    /// `vp = ortho(region) * transform⁻¹`：世界 → 画面居中像素 → NDC（含 Y 翻转）。
    #[inline]
    pub fn vp_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// 视图矩阵（世界 → 画面居中像素）= `transform.to_matrix()⁻¹`。
    #[inline]
    pub fn view_matrix(&self) -> Mat4 {
        self.transform.to_matrix().inverse()
    }

    /// 正交投影矩阵（画面居中像素 → NDC，含 Y 翻转）。
    ///
    /// 保护：矩形宽高为 0 时按 `f32::EPSILON` 处理，避免不可逆矩阵产生 NaN。
    #[inline]
    pub fn projection_matrix(&self) -> Mat4 {
        let half_w = (self.region.w * 0.5).max(f32::EPSILON);
        let half_h = (self.region.h * 0.5).max(f32::EPSILON);
        glam::camera::rh::proj::directx::orthographic(-half_w, half_w, half_h, -half_h, 0.0, 1.0)
    }

    /// 可见半宽高（世界单位）：`region.size * 0.5 * transform.scale`（非均匀逐分量）。
    #[inline]
    pub fn view_half_size(&self) -> Vec2 {
        Vec2::new(
            self.region.w * 0.5 * self.transform.scale.x,
            self.region.h * 0.5 * self.transform.scale.y,
        )
    }

    /// 世界视口**保守 AABB**（含旋转）：把画面矩形四角经 [`Self::transform`] 变换后取包围盒。
    ///
    /// 旋转时是旋转矩形的超集——剔除**不误杀**（保守，多绘一点）。
    #[inline]
    pub fn view_aabb(&self) -> Rect {
        let half = Vec2::new(self.region.w * 0.5, self.region.h * 0.5);
        let pts = [
            Vec2::new(-half.x, -half.y),
            Vec2::new(half.x, -half.y),
            Vec2::new(-half.x, half.y),
            half,
        ];
        Rect::from_point_slice(&self.transform.transform_points(&pts))
    }

    /// 窗口像素 → 世界。
    ///
    /// 实现单源到 [`Transform2D::transform_point`]：`world = transform(px - region.center())`。
    /// 与 [`Self::world_to_screen`] 在任意位置/旋转/非均匀缩放下**互为精确逆**。
    #[inline]
    pub fn screen_to_world(&self, screen_px: Vec2) -> Vec2 {
        self.transform.transform_point(screen_px - self.region.center())
    }

    /// 世界 → 窗口像素（[`Self::screen_to_world`] 的精确逆）。
    #[inline]
    pub fn world_to_screen(&self, world_pos: Vec2) -> Vec2 {
        self.transform.inverse_transform_point(world_pos) + self.region.center()
    }

    /// 世界 → **画面居中像素**（不含 `region` 偏移；UI / 屏幕固定绘制用）。
    ///
    /// 点级精确逆（与 [`Self::screen_to_world`] 互为逆）。注意：**不要**用
    /// [`Transform2D::inverse`] 的「对象级逆」来做屏幕固定锚点——`Transform2D` 的参数化固定为
    /// 「先缩放后旋转」，无法表达其逆的「先旋转后缩放」，两者仅在**均匀 scale** 下一致。
    #[inline]
    pub fn world_to_region_local(&self, world_pos: Vec2) -> Vec2 {
        self.transform.inverse_transform_point(world_pos)
    }

    /// 逃逸口：直接取世界 → NDC 的裁剪坐标（诊断 / 自定义剔除用）。
    #[inline]
    pub fn world_to_clip(&self, world_pos: Vec2) -> Vec4 {
        self.vp_matrix() * Vec4::new(world_pos.x, world_pos.y, 0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    const W: f32 = 1280.0;
    const H: f32 = 720.0;
    const EPS: f32 = 1e-3;

    fn cam() -> Camera2D {
        Camera2D::full(Vec2::new(W, H))
    }

    #[test]
    fn center_maps_to_origin() {
        let world = cam().screen_to_world(Vec2::new(W * 0.5, H * 0.5));
        assert!((world - Vec2::ZERO).length() < EPS, "center should map to origin, got {world:?}");
    }

    #[test]
    fn screen_bottom_is_plus_y() {
        let world = cam().screen_to_world(Vec2::new(W * 0.5, H));
        assert!((world.y - H * 0.5).abs() < EPS, "bottom should be +half_h, got {}", world.y);
        assert!((world.x - 0.0).abs() < EPS, "bottom-center x should be 0, got {}", world.x);
    }

    #[test]
    fn screen_top_is_minus_y() {
        let world = cam().screen_to_world(Vec2::new(W * 0.5, 0.0));
        assert!((world.y + H * 0.5).abs() < EPS, "top should be -half_h, got {}", world.y);
    }

    #[test]
    fn screen_right_is_plus_x() {
        let world = cam().screen_to_world(Vec2::new(W, H * 0.5));
        assert!((world.x - W * 0.5).abs() < EPS, "right should be +half_w, got {}", world.x);
    }

    #[test]
    fn sub_region_offsets_mapping() {
        // 半屏画面（左半边）：画面中心 = 窗口 (W/4, H/2)
        let mut c = cam();
        c.set_region(Rect::new(0.0, 0.0, W * 0.5, H));
        let world = c.screen_to_world(Vec2::new(W * 0.25, H * 0.5));
        assert!(world.length() < EPS, "子画面中心应映射到世界原点，got {world:?}");
        // 同一个窗口像素在半屏相机下世界 x 减半
        let world_r = c.screen_to_world(Vec2::new(W * 0.5, H * 0.5));
        assert!((world_r.x - W * 0.25).abs() < EPS, "got {}", world_r.x);
    }

    #[test]
    fn roundtrip_world_screen_all_poses() {
        let mut c = cam();
        c.transform.pos = Vec2::new(30.0, -20.0);
        c.transform.rotation = 0.4;
        c.transform.scale = Vec2::new(0.75, 1.5); // 非均匀（= zoom (1.333, 0.667)）
        for p in [Vec2::ZERO, Vec2::new(123.0, -456.0), Vec2::new(-640.0, 360.0)] {
            let px = c.world_to_screen(p);
            let back = c.screen_to_world(px);
            assert!(
                (back - p).length() < EPS,
                "roundtrip failed for {p:?}: px={px:?} back={back:?}"
            );
        }
    }

    #[test]
    fn set_zoom_is_reciprocal_of_scale() {
        let mut c = cam();
        c.set_zoom(Vec2::splat(2.0));
        assert!((c.transform.scale - Vec2::splat(0.5)).length() < EPS, "scale = 1/zoom");
        assert!((c.zoom() - Vec2::splat(2.0)).length() < EPS, "zoom 往返应还原");
    }

    #[test]
    fn zoom_scales_world() {
        let mut c = cam();
        c.set_zoom(Vec2::splat(2.0));
        // zoom=2 → 世界范围减半：屏幕底部中心 → world.y = +half_h / 2
        let world = c.screen_to_world(Vec2::new(W * 0.5, H));
        assert!((world.y - H * 0.25).abs() < EPS, "zoomed bottom should be +half_h/2, got {}", world.y);
    }

    #[test]
    fn region_local_mapping_is_exact_inverse_all_poses() {
        // 点级映射（screen_to_world ↔ world_to_region_local）在任意位置/旋转/非均匀缩放下精确互逆。
        let mut c = cam();
        c.transform.pos = Vec2::new(30.0, -20.0);
        c.transform.rotation = 0.4;
        c.transform.scale = Vec2::new(0.75, 1.5); // 非均匀
        for w in [Vec2::ZERO, Vec2::new(123.0, -456.0), Vec2::new(-640.0, 360.0)] {
            let local = c.world_to_region_local(w);
            let back = c.transform.transform_point(local);
            assert!((back - w).length() < EPS, "point roundtrip 应还原 {w:?} → {back:?}");
        }
    }

    #[test]
    fn view_matrix_is_matrix_inverse_of_transform() {
        // view_matrix 必须是 transform.to_matrix() 的精确矩阵逆（GPU 路径一致性）
        let mut c = cam();
        c.transform.pos = Vec2::new(30.0, -20.0);
        c.transform.rotation = 0.4;
        c.transform.scale = Vec2::new(0.75, 1.5);
        let m = c.transform.to_matrix() * c.view_matrix();
        let i = Mat4::IDENTITY;
        for r in 0..4 {
            for col in 0..4 {
                assert!(
                    (m.col(r)[col] - i.col(r)[col]).abs() < 1e-4,
                    "transform * view 应为单位矩阵，差异在 [{r}][{col}]"
                );
            }
        }
    }

    #[test]
    fn view_half_size_follows_region_and_scale() {
        let mut c = cam();
        c.transform.scale = Vec2::splat(0.5); // zoom = 2
        let h = c.view_half_size();
        assert!((h - Vec2::new(W * 0.25, H * 0.25)).length() < EPS, "got {h:?}");
    }

    #[test]
    fn view_aabb_is_conservative_when_rotated() {
        let mut c = cam();
        c.transform.rotation = std::f32::consts::FRAC_PI_4; // 45°
        let a = c.view_aabb();
        let half = Vec2::new(W, H) * 0.5;
        for p in [
            Vec2::new(-half.x, -half.y),
            Vec2::new(half.x, -half.y),
            Vec2::new(-half.x, half.y),
            half,
        ] {
            let world = c.transform.transform_point(p);
            assert!(a.contains_point(world), "旋转后视口角点 {p:?} 应在保守 AABB 内");
        }
        // 未旋转：AABB 应贴着 ±half
        c.transform.rotation = 0.0;
        let a0 = c.view_aabb();
        assert!((a0.x + W * 0.5).abs() < EPS && (a0.w - W).abs() < EPS, "未旋转时 AABB 应贴合画面");
    }

    /// 屏幕固定文本（UI）的变换数学：局部点 `local` 经
    /// `{ pos: anchor_world, rotation: +cam.rotation, scale: 1/zoom }` 到世界，再 world_to_screen，
    /// 应等于 `anchor_px + local`（1:1、不旋转、不缩放）。
    #[test]
    fn screen_fixed_transform_maps_local_to_screen_1to1() {
        let mut c = cam();
        c.transform.pos = Vec2::new(100.0, -50.0);
        c.transform.rotation = 0.7;
        c.transform.scale = Vec2::new(0.6667, 1.3333);
        let anchor_px = Vec2::new(40.0, 30.0);
        let anchor_world = c.screen_to_world(anchor_px);
        let t = Transform2D::IDENTITY
            .with_pos(anchor_world)
            .with_rot(c.transform.rotation)
            .with_scale(c.transform.scale);
        for local in [
            Vec2::new(0.0, 0.0),
            Vec2::new(50.0, 12.0),
            Vec2::new(-20.0, 100.0),
            Vec2::new(300.0, -40.0),
        ] {
            let world = t.transform_point(local);
            let screen = c.world_to_screen(world);
            let expect = anchor_px + local;
            assert!(
                (screen - expect).length() < 0.05,
                "local {local:?} → screen {screen:?} 应等于 {expect:?}（rot={} scale={:?}）",
                c.transform.rotation,
                c.transform.scale
            );
        }
    }

    #[test]
    fn deref_exposes_transform_motion() {
        let mut c = cam();
        c.move_local(Vec2::new(10.0, 0.0)); // 经 DerefMut → Transform2D::move_local
        assert!((c.transform.pos - Vec2::new(10.0, 0.0)).length() < EPS, "got {:?}", c.transform.pos);
    }

    #[test]
    fn vp_matrix_maps_region_corners_to_ndc() {
        let mut c = cam();
        c.transform.pos = Vec2::new(100.0, 50.0);
        c.transform.rotation = 0.3;
        c.transform.scale = Vec2::splat(0.5);
        // 画面四角（屏幕像素）→ vp → NDC 应落在 ±1（Y 翻转）
        for (px, ndc) in [
            (Vec2::new(0.0, 0.0), Vec2::new(-1.0, 1.0)),
            (Vec2::new(W, 0.0), Vec2::new(1.0, 1.0)),
            (Vec2::new(0.0, H), Vec2::new(-1.0, -1.0)),
            (Vec2::new(W, H), Vec2::new(1.0, -1.0)),
        ] {
            let world = c.screen_to_world(px);
            let clip = c.world_to_clip(world);
            let got = Vec2::new(clip.x / clip.w, clip.y / clip.w);
            assert!(
                (got - ndc).length() < 1e-3,
                "屏幕角点 {px:?} → NDC {got:?}，应为 {ndc:?}"
            );
        }
    }
}
