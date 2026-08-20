use glam::{Mat4, Vec2, Vec3, Vec4};

/// A 2D orthographic camera.
///
/// Handles the View-Projection matrix and coordinates conversion between
/// screen space (pixels) and world space.
/// 
/// 坐标系：\
/// ```text
/// ┌   T   ┐  O 为原点 (0, 0)
///     |      Y+ 为下
/// L - O - R  X+ 为右
///     |    
/// └   B   ┘
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Camera2D {
    /// Camera position in world space.
    pub position: Vec2,
    /// Camera rotation (radians).
    pub rotation: f32,
    /// Camera zoom (Vec2 for non-uniform scaling).
    pub zoom: Vec2,
    /// Top-left corner of the viewport in window pixels.
    pub viewport_pos: Vec2,
    /// Size of the viewport in pixels.
    pub viewport_size: Vec2,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self { position: Vec2::ZERO, rotation: 0.0, zoom: Vec2::ONE, viewport_pos: Vec2::ZERO, viewport_size: Vec2::ONE }
    }
}

#[allow(unused)]
impl Camera2D {
    #[inline]
    pub fn move_by(&mut self, position: Vec2) {
        self.position += position;
    }
    #[inline]
    pub fn walk_xy(&mut self, xy: Vec2) {
        let (sin, cos) = self.rotation.sin_cos();
        self.position += Vec2::new(xy.x * cos - xy.y * sin, xy.x * sin + xy.y * cos);
    }
    #[inline]
    pub fn walk_xplus(&mut self, xplus: f32) {
        let (sin, cos) = self.rotation.sin_cos();
        self.position += Vec2::new(cos, sin ) * xplus;
    }
    #[inline]
    pub fn walk_yplus(&mut self, yplus: f32) {
        let (sin, cos) = self.rotation.sin_cos();
        self.position += Vec2::new(-sin, cos ) * yplus;
    }
}

#[allow(unused)]
impl Camera2D {
    /// Create a camera that covers the entire window.
    #[inline]
    pub fn new(window_size_px: Vec2) -> Self {
        Self {
            position: Vec2::ZERO,
            rotation: 0.0,
            zoom: Vec2::ONE,
            viewport_pos: Vec2::ZERO,
            viewport_size: window_size_px,
        }
    }

    #[inline]
    pub fn set_vp(&mut self, viewport_size: Vec2, viewport_pos: Vec2) {
        self.viewport_size = viewport_size;
        self.viewport_pos = viewport_pos;
    }

    /// --- Matrix helpers ---

    /// Full View-Projection matrix: P × V.
    ///
    /// Note: Returns the matrix in a form ready to be used with
    /// `batch.set_mvp(gfx, &vp.transpose())`.
    #[inline]
    pub fn vp_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// View matrix: inverse of camera transform.
    #[inline]
    pub fn view_matrix(&self) -> Mat4 {
        let t = Mat4::from_translation(Vec3::new(-self.position.x, -self.position.y, 0.0));
        let r = Mat4::from_rotation_z(-self.rotation);
        let s = Mat4::from_scale(Vec3::new(self.zoom.x, self.zoom.y, 1.0));
        s * r * t
    }

    /// Orthographic projection matrix (built from viewport size).
    ///
    /// 按坐标系规范：原点居中、X+ 右、Y+ 下。
    /// `top = -half_h < bottom = +half_h` 使 `y_ndc = 2y/(top-bottom)` 分母为负 → Y 翻转成向下。
    #[inline]
    pub fn projection_matrix(&self) -> Mat4 {
        // 保护：视口尺寸为 0 → left==right && top==bottom → 正交矩阵不可逆（NaN）。
        let vp_size = self.viewport_size.max(Vec2::splat(f32::EPSILON));
        let half_w = vp_size.x * 0.5;
        let half_h = vp_size.y * 0.5;
        glam::camera::rh::proj::directx::orthographic(-half_w, half_w, half_h, -half_h, 0.0, 1.0)
    }

    /// --- Viewport state application ---
    // todo

    /// 可见半宽高（世界单位）：`viewport_size * 0.5 / zoom`（非均匀缩放逐分量）。
    #[inline]
    pub fn view_half_size(&self) -> Vec2 {
        Vec2::new(
            self.viewport_size.x * 0.5 / self.zoom.x,
            self.viewport_size.y * 0.5 / self.zoom.y,
        )
    }

    /// 未旋转相机时的世界视口矩形：`position ± view_half_size()`。
    #[inline]
    pub fn world_view_rect(&self) -> crate::Rect {
        let half = self.view_half_size();
        crate::Rect::new(self.position.x - half.x, self.position.y - half.y, half.x * 2.0, half.y * 2.0)
    }

    /// 世界视口**保守 AABB**（含旋转）：把相机局部视口矩形四角经
    /// [`Self::world_transform`]（pos + 旋转 + 1/zoom 缩放）变换后取包围盒。
    ///
    /// 旋转相机时视口是旋转矩形，此 AABB 是其超集——剔除**不误杀**（保守，多绘一点）。
    #[inline]
    pub fn view_aabb(&self) -> crate::Rect {
        let half = self.viewport_size * 0.5;
        let t = self.world_transform();
        let pts = [
            Vec2::new(-half.x, -half.y),
            Vec2::new(half.x, -half.y),
            Vec2::new(-half.x, half.y),
            half,
        ];
        crate::Rect::from_point_slice(&t.transform_points(&pts))
    }

    /// --- Camera as a world transform ---

    /// View the camera as a parent world transform.
    /// 把相机视为一个父级世界变换（`Transform2D`）。
    ///
    /// 返回 `Transform2D { pos: position, rot: rotation, scale: 1/zoom }`，满足
    /// `world_transform().transform_point(local) == view_matrix⁻¹ * local`。
    /// 也就是说，用相机局部坐标（屏幕空间像素）经该变换即可得世界坐标。
    ///
    /// 用于 UI 层级：把相机与面板/控件统一到 `Transform2D` 一层做反父级运算
    /// （例如 `child.with_inverse_transform(&camera.world_transform())`）。
    #[inline]
    pub fn world_transform(&self) -> crate::Transform2D {
        crate::Transform2D {
            pos: self.position,
            scale: Vec2::new(1.0 / self.zoom.x, 1.0 / self.zoom.y),
            rotation: self.rotation,
        }
    }

    /// 世界 → 相机局部（view 变换，= [`Self::world_transform`] 的**逆**）。
    ///
    /// 输出 = `R(-rotation)·((w - position)·zoom)`（相机局部坐标，Y+ 向下）。
    ///
    /// **精度说明**：`Transform2D { pos, scale, rotation }` 的参数化固定"先缩放后旋转"，
    /// 其逆也只能是"先缩放后旋转"结构；而 `view_matrix`（先旋转后缩放）在**非均匀缩放 +
    /// 旋转**下与前者不可交换——此时 `view_transform` 是近似（往返有偏差）。
    /// **均匀缩放**（`zoom.x == zoom.y`）下完全精确，且满足
    /// `world_to_screen(w) - viewport_pos - vp/2 == view_transform(w)`。
    /// 需要旋转 + 非均匀缩放的精确坐标反算时，请用矩阵路径（`view_matrix` / `vp_matrix`）。
    ///
    /// 供坐标反算：把世界点/矩形变换到相机空间（屏幕固定文本、命中测试、精确剔除等）。
    #[inline]
    pub fn view_transform(&self) -> crate::Transform2D {
        self.world_transform().inverse()
    }

    /// --- Coordinate conversion ---

    /// Convert a window pixel coordinate to world space.
    ///
    /// Screen Y goes top→bottom (0 at top of window), world Y goes
    /// bottom→top (-half_h at bottom, +half_h at top per orthographic_rh).
    /// This method flips Y accordingly.
    #[inline]
    pub fn screen_to_world(&self, screen_px: Vec2) -> Vec2 {
        // 保护：视口尺寸为 0 会导致除零 → 产生 inf/NaN 而非 panic。
        let vp_size = self.viewport_size.max(Vec2::splat(f32::EPSILON));
        // 1. Window pixel → viewport-local pixel
        let local_px = screen_px - self.viewport_pos;
        // 2. Viewport-local pixel → NDC [-1, 1], flipping Y (screen Y↓ vs NDC Y↑)
        let ndc = Vec2::new(
            (local_px.x / vp_size.x) * 2.0 - 1.0,
            1.0 - (local_px.y / vp_size.y) * 2.0, // Y flip
        );
        // 3. NDC → world via VP⁻¹
        let vp_inv = self.vp_matrix().inverse();
        let clip = Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
        let world = vp_inv * clip;
        Vec2::new(world.x / world.w, world.y / world.w)
    }

    /// Convert a world coordinate to window pixel coordinate.
    ///
    /// Undoes the Y flip done in screen_to_world.
    #[inline]
    pub fn world_to_screen(&self, world_pos: Vec2) -> Vec2 {
        let clip = self.vp_matrix() * Vec4::new(world_pos.x, world_pos.y, 0.0, 1.0);
        let ndc = Vec2::new(clip.x / clip.w, clip.y / clip.w);
        let vp_size = self.viewport_size.max(Vec2::splat(f32::EPSILON));
        let local_px = Vec2::new(
            (ndc.x + 1.0) * 0.5 * vp_size.x,
            (1.0 - ndc.y) * 0.5 * vp_size.y, // Y flip back
        );
        local_px + self.viewport_pos
    }
}

/// **视口**（UI 专用）："窗口像素 → 世界"的映射（**identity 相机特例**：
/// pos=0 / rot=0 / zoom=1，UI 不旋转/缩放，无需相机语义）。
///
/// `rjw_ui` 的 [`Ui::finish`](crate::ui::Ui::finish) 用它替代 [`Camera2D`]：
/// - [`Self::vp_matrix`]：Render2D 的 `set_mvp`（= identity 相机的 View-Projection）；
/// - [`Self::screen_to_world`]：屏幕固定文本/图形变换的锚点换算。
///
/// 数学直接委托恒等 [`Camera2D`]（`new(size)` + `set_vp(size, pos)`），零重复。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// 视口左上角（窗口像素）。
    pub pos: Vec2,
    /// 视口尺寸（像素）。
    pub size: Vec2,
}

impl Viewport {
    pub const fn new(size: Vec2, pos: Vec2) -> Self {
        Self { pos, size }
    }

    /// identity 相机的 View-Projection 矩阵（含 Y flip）——Render2D `set_mvp` 用。
    #[inline]
    pub fn vp_matrix(&self) -> Mat4 {
        let mut c = Camera2D::new(self.size);
        c.set_vp(self.size, self.pos);
        c.vp_matrix()
    }

    /// 窗口像素 → 世界（identity 相机特例，数学同 [`Camera2D::screen_to_world`]）。
    #[inline]
    pub fn screen_to_world(&self, screen_px: Vec2) -> Vec2 {
        let mut c = Camera2D::new(self.size);
        c.set_vp(self.size, self.pos);
        c.screen_to_world(screen_px)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    const W: f32 = 1280.0;
    const H: f32 = 720.0;
    const EPS: f32 = 1e-4;

    fn cam() -> Camera2D {
        let mut c = Camera2D::new(Vec2::new(W, H));
        c.set_vp(Vec2::new(W, H), Vec2::ZERO);
        c
    }

    #[test]
    fn center_maps_to_origin() {
        // 屏幕中心像素 → 世界原点 (0,0)
        let world = cam().screen_to_world(Vec2::new(W * 0.5, H * 0.5));
        assert!((world - Vec2::ZERO).length() < EPS, "center should map to origin, got {world:?}");
    }

    #[test]
    fn screen_bottom_is_plus_y() {
        // 规范：Y+ 为下 → 屏幕底部中点应映射为 world.y = +half_h
        let world = cam().screen_to_world(Vec2::new(W * 0.5, H));
        assert!((world.y - H * 0.5).abs() < EPS, "bottom should be +half_h, got {}", world.y);
        assert!((world.x - 0.0).abs() < EPS, "bottom-center x should be 0, got {}", world.x);
    }

    #[test]
    fn screen_top_is_minus_y() {
        // 规范：Y+ 为下 → 屏幕顶部中点应映射为 world.y = -half_h
        let world = cam().screen_to_world(Vec2::new(W * 0.5, 0.0));
        assert!((world.y + H * 0.5).abs() < EPS, "top should be -half_h, got {}", world.y);
    }

    #[test]
    fn screen_right_is_plus_x() {
        // 规范：X+ 为右 → 屏幕右边缘中点应映射为 world.x = +half_w
        let world = cam().screen_to_world(Vec2::new(W, H * 0.5));
        assert!((world.x - W * 0.5).abs() < EPS, "right should be +half_w, got {}", world.x);
    }

    #[test]
    fn roundtrip_world_screen() {
        let c = cam();
        for p in [
            Vec2::new(0.0, 0.0),
            Vec2::new(123.0, -456.0),
            Vec2::new(-640.0, 360.0),
        ] {
            let px = c.world_to_screen(p);
            let back = c.screen_to_world(px);
            assert!(
                (back - p).length() < EPS,
                "roundtrip failed for {p:?}: px={px:?} back={back:?}"
            );
        }
    }

    #[test]
    fn zoom_scales_world() {
        let mut c = cam();
        c.zoom = Vec2::splat(2.0);
        // zoom=2 → 世界范围减半：屏幕底部中心 → world.y = +half_h / 2
        let world = c.screen_to_world(Vec2::new(W * 0.5, H));
        assert!((world.y - H * 0.25).abs() < EPS, "zoomed bottom should be +half_h/2, got {}", world.y);
    }

    #[test]
    fn world_transform_matches_view_inverse() {
        // world_transform().transform_point(local) 应等于 view_matrix⁻¹ * local
        let mut c = cam();
        c.position = Vec2::new(30.0, -20.0);
        c.rotation = 0.4;
        c.zoom = Vec2::new(1.5, 0.75);

        let wt = c.world_transform();
        let local = Vec2::new(100.0, 50.0);
        let via_tf = wt.transform_point(local);

        let view_inv = c.view_matrix().inverse();
        let clip = view_inv * Vec4::new(local.x, local.y, 0.0, 1.0);
        let via_mat = Vec2::new(clip.x / clip.w, clip.y / clip.w);

        assert!(
            (via_tf - via_mat).length() < EPS,
            "world_transform vs view⁻¹ mismatch: tf={via_tf:?} mat={via_mat:?}"
        );
    }

    #[test]
    fn world_view_rect_follows_position_and_zoom() {
        let mut c = cam();
        c.position = Vec2::new(100.0, -50.0);
        c.zoom = Vec2::splat(2.0);
        let r = c.world_view_rect();
        assert!((r.x - (100.0 - W * 0.25)).abs() < EPS, "left = pos.x - half/zoom");
        assert!((r.y - (-50.0 - H * 0.25)).abs() < EPS, "top = pos.y - half/zoom");
        assert!((r.w - W * 0.5).abs() < EPS, "width = viewport/zoom");
        assert!((r.h - H * 0.5).abs() < EPS, "height = viewport/zoom");
    }

    #[test]
    fn view_aabb_is_conservative_when_rotated() {
        let mut c = cam();
        c.position = Vec2::ZERO;
        c.rotation = 0.785398; // 45°
        let a = c.view_aabb();
        let half = Vec2::new(W, H) * 0.5;
        let t = c.world_transform();
        for p in [
            Vec2::new(-half.x, -half.y),
            Vec2::new(half.x, -half.y),
            Vec2::new(-half.x, half.y),
            half,
        ] {
            assert!(a.contains_point(t.transform_point(p)), "旋转后视口角点 {p:?} 应在保守 AABB 内");
        }
        // 未旋转：view_aabb == world_view_rect
        c.rotation = 0.0;
        let a0 = c.view_aabb();
        let r0 = c.world_view_rect();
        assert!((a0.x - r0.x).abs() < EPS && (a0.w - r0.w).abs() < EPS, "未旋转时二者应一致");
    }

    /// 屏幕固定文本（UI）的变换数学：局部点 local 经
    /// `{ pos: screen_to_world(anchor), rotation: +cam.rotation, scale: 1/zoom }`
    /// 到世界、再 world_to_screen，应等于 `anchor + local`（1:1、不旋转、不缩放）。
    ///
    /// 曾犯错误：用 `-cam.rotation` 会双重旋转（文字转 -2×rot）。
    #[test]
    fn screen_fixed_transform_maps_local_to_screen_1to1() {
        let mut c = cam();
        c.position = Vec2::new(100.0, -50.0);
        c.rotation = 0.7;
        c.zoom = Vec2::new(1.5, 0.75);
        let anchor_px = Vec2::new(40.0, 30.0);
        let anchor_world = c.screen_to_world(anchor_px);
        let t = crate::Transform2D::IDENTITY
            .with_pos(anchor_world)
            .with_rot(c.rotation)
            .with_scale(Vec2::new(1.0 / c.zoom.x, 1.0 / c.zoom.y));
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
                "local {local:?} → screen {screen:?} 应等于 {expect:?}（相机 rot={} zoom={:?}）",
                c.rotation, c.zoom
            );
        }
    }

    #[test]
    fn view_transform_roundtrip_uniform_zoom() {
        // 均匀缩放下 view_transform ↔ world_transform 互为精确逆
        let mut c = cam();
        c.position = Vec2::new(30.0, -20.0);
        c.rotation = 0.4;
        c.zoom = Vec2::splat(1.5);
        for w in [Vec2::ZERO, Vec2::new(123.0, -456.0), Vec2::new(-640.0, 360.0)] {
            let local = c.view_transform().transform_point(w);
            let back = c.world_transform().transform_point(local);
            assert!((back - w).length() < 0.05, "view→world 往返应还原 {w:?} → {back:?}");
        }
        // 非均匀缩放 + 旋转：Transform2D 参数化固有局限（先缩后旋 vs 先旋后缩），
        // view_transform 为近似——仅验证不 panic，不做精确断言。
        c.zoom = Vec2::new(1.5, 0.75);
        let _ = c.view_transform().transform_point(Vec2::new(123.0, -456.0));
    }

    #[test]
    fn view_transform_matches_screen_offset_uniform_zoom() {
        // 均匀缩放下：view_transform(w) == world_to_screen(w) - viewport_pos - vp/2
        let mut c = cam();
        c.position = Vec2::new(30.0, -20.0);
        c.rotation = 0.4;
        c.zoom = Vec2::splat(1.5);
        let vp = c.viewport_size;
        for w in [Vec2::ZERO, Vec2::new(123.0, -456.0), Vec2::new(-640.0, 360.0)] {
            let via_tf = c.view_transform().transform_point(w);
            let screen = c.world_to_screen(w);
            let expect = screen - c.viewport_pos - vp * 0.5;
            assert!(
                (via_tf - expect).length() < 0.05,
                "均匀缩放 view_transform({w:?})={via_tf:?} 应等于屏幕偏移 {expect:?}"
            );
        }
    }

    /// Viewport = identity 相机的"窗口像素 → 世界"（1:1 屏幕固定数学）。
    #[test]
    fn viewport_screen_to_world_matches_identity_camera() {
        let vp = Viewport::new(Vec2::new(W, H), Vec2::new(40.0, 30.0));
        // 与同参恒等 Camera2D 结果一致
        let mut c = Camera2D::new(Vec2::new(W, H));
        c.set_vp(Vec2::new(W, H), Vec2::new(40.0, 30.0));
        for px in [Vec2::ZERO, Vec2::new(640.0, 360.0), Vec2::new(W, H), Vec2::new(123.0, 456.0)] {
            assert!(
                (vp.screen_to_world(px) - c.screen_to_world(px)).length() < 1e-4,
                "screen_to_world({px:?}) 应等于恒等相机"
            );
        }
    }

    /// Viewport 屏幕固定变换：局部点 → 世界 → world_to_screen = 锚点 + 局部（1:1）。
    #[test]
    fn viewport_screen_fixed_transform_maps_local_to_screen_1to1() {
        let vp = Viewport::new(Vec2::new(W, H), Vec2::ZERO);
        let anchor_px = Vec2::new(40.0, 30.0);
        let t = crate::Transform2D::IDENTITY.with_pos(vp.screen_to_world(anchor_px));
        // 用与 Viewport 同参的 Camera2D 做 world_to_screen 往返
        let mut c = Camera2D::new(Vec2::new(W, H));
        c.set_vp(Vec2::new(W, H), Vec2::ZERO);
        for local in [Vec2::ZERO, Vec2::new(50.0, 12.0), Vec2::new(300.0, -40.0), Vec2::new(-20.0, 100.0)] {
            let world = t.transform_point(local);
            let screen = c.world_to_screen(world);
            let expect = anchor_px + local;
            assert!(
                (screen - expect).length() < 0.05,
                "local {local:?} → screen {screen:?} 应等于 {expect:?}"
            );
        }
    }
}
