use glam::{Mat4, Vec2};

/// 2D 变换：位置 / 缩放 / 旋转。
///
/// 语义（与 [`crate::Camera2D`]、渲染器、`InstanceData` 完全一致）：
/// `parent_point = pos + rotate(local_point * scale, rotation)`，即**先缩放 → 再旋转 → 再平移**。
///
/// 约定：`with_*` 是**消费式**构建器（返回新值）；`move_by` / `move_local` 是**就地**修改。
#[derive(Debug, Clone, Copy)]
pub struct Transform2D {
    pub pos: glam::Vec2,
    pub scale: glam::Vec2,
    pub rotation: f32,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            pos: Vec2::splat(0.0),
            scale: Vec2::splat(1.0),
            rotation: 0.0,
        }
    }
}

impl Transform2D {
    /// Identity transform: position (0,0), scale (1,1), rotation 0.
    /// 单位变换：位置 (0,0)，缩放 (1,1)，旋转 0。
    pub const IDENTITY: Self = Self {
        pos: Vec2::ZERO,
        scale: Vec2::ONE,
        rotation: 0.0,
    };

    /// Builder: set position. / 构建器模式：设置位置（接受 `Vec2` 或 `(x, y)`）。
    #[inline]
    pub fn with_pos(mut self, pos: impl Into<Vec2>) -> Self {
        self.pos = pos.into();
        self
    }

    /// Builder: set scale. / 构建器模式：设置缩放（接受 `Vec2` 或 `(x, y)`）。
    #[inline]
    pub fn with_scale(mut self, scale: impl Into<Vec2>) -> Self {
        self.scale = scale.into();
        self
    }

    /// Builder: set rotation. / 构建器模式：设置旋转。
    #[inline]
    pub fn with_rot(mut self, rot: f32) -> Self {
        self.rotation = rot;
        self
    }

    /// 就地位移（世界轴）：`pos += delta`。
    #[inline]
    pub fn move_by(&mut self, delta: impl Into<Vec2>) {
        self.pos += delta.into();
    }

    /// 就地位移（局部轴）：`delta` 先按 `rotation` 旋转，再应用到 `pos`。
    ///
    /// 沿自身"朝向"前后左右移动用它；世界轴移动用 [`Self::move_by`]。
    #[inline]
    pub fn move_local(&mut self, delta: impl Into<Vec2>) {
        self.pos += Vec2::from_angle(self.rotation).rotate(delta.into());
    }

    /// 组合父级变换：`result = parent * self`（等价 `parent.compose(self)`）。
    ///
    /// `result.pos  = parent.pos + rotate(self.pos, parent.rot) * parent.scale`
    /// `result.scale = self.scale * parent.scale`
    /// `result.rot   = self.rotation + parent.rot`
    #[inline]
    pub fn compose(&self, parent: &Transform2D) -> Self {
        let (sin, cos) = parent.rotation.sin_cos();
        let rotated = Vec2::new(
            self.pos.x * cos - self.pos.y * sin,
            self.pos.x * sin + self.pos.y * cos,
        ) * parent.scale;
        Self {
            pos: parent.pos + rotated,
            scale: self.scale * parent.scale,
            rotation: self.rotation + parent.rotation,
        }
    }

    /// 反父级组合：`result = parent⁻¹ * self`（= `parent.inverse().compose(self)`）。
    ///
    /// 把当前变换放进 `parent` 的**逆**空间。UI 用它把子元素变换换算到祖先面板局部坐标系
    /// （命中检测 / 布局）。
    #[inline]
    pub fn compose_inverse(&self, parent: &Transform2D) -> Self {
        parent.inverse().compose(self)
    }

    /// 列主序模型矩阵（列向量约定：`m * vec4(local, 0, 1)` == [`Self::transform_point`]）。
    ///
    /// 渲染器 / 剔除 / 相机全部经这一个矩阵出口，避免各处手搓矩阵。
    #[inline]
    pub fn to_matrix(&self) -> Mat4 {
        let (sin, cos) = self.rotation.sin_cos();
        Mat4::from_cols_array_2d(&[
            [cos * self.scale.x, sin * self.scale.x, 0.0, 0.0],
            [-sin * self.scale.y, cos * self.scale.y, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [self.pos.x, self.pos.y, 0.0, 1.0],
        ])
    }

    /// Transform a point from this entity's local space to parent space.
    /// 将点从实体的局部空间变换到父级空间。
    ///
    /// `parent_point = pos + rotate(local_point * scale, rot)`
    #[inline]
    pub fn transform_point(&self, local_point: Vec2) -> Vec2 {
        let (sin, cos) = self.rotation.sin_cos();
        let scaled = local_point * self.scale;
        self.pos
            + Vec2::new(
                scaled.x * cos - scaled.y * sin,
                scaled.x * sin + scaled.y * cos,
            )
    }

    /// Transform an array of points from local space to parent space.
    /// 将一组点从局部空间变换到父级空间（逐个 [`Self::transform_point`]）。
    #[inline]
    pub fn transform_points<const N: usize>(&self, local: &[Vec2; N]) -> [Vec2; N] {
        let mut out = [Vec2::ZERO; N];
        for (i, p) in local.iter().enumerate() {
            out[i] = self.transform_point(*p);
        }
        out
    }

    /// Transform an array of points and return their bounding box.
    /// 变换一组点并返回其轴对齐包围盒（AABB，保守）。
    #[inline]
    pub fn transform_points_aabb<const N: usize>(&self, local: &[Vec2; N]) -> crate::Rect {
        crate::Rect::from_point_slice(&self.transform_points(local))
    }

    /// Inverse: transform a point from parent space back to local space.
    /// 反向变换：将点从父级空间变换回局部空间。
    ///
    /// `local_point = rotate(parent_point - pos, -rot) / scale`
    #[inline]
    pub fn inverse_transform_point(&self, parent_point: Vec2) -> Vec2 {
        let (sin, cos) = (-self.rotation).sin_cos();
        let translated = parent_point - self.pos;
        Vec2::new(
            (translated.x * cos - translated.y * sin) / self.scale.x,
            (translated.x * sin + translated.y * cos) / self.scale.y,
        )
    }

    /// Return the inverse transform object.
    /// 返回**对象级**逆变换：满足 `t.compose(&t.inverse()) == IDENTITY`。
    ///
    /// 数学：`scale' = 1/scale`、`rot' = -rot`、`pos' = -rotate(pos, -rot) / scale`。
    ///
    /// # 与点级逆的区别（重要）
    ///
    /// [`Self::inverse_transform_point`] 是 [`Self::transform_point`] 的**点级精确逆**；
    /// 而本方法的 `transform_point` 只在**均匀 scale** 下与它一致——`Transform2D` 的参数化固定为
    /// 「先缩放 → 再旋转 → 再平移」，无法表达其逆的「先旋转 → 再缩放」次序，
    /// 所以非均匀 scale + 旋转时二者有偏差。需要的场景：
    ///
    /// - 点/矩形往返、命中检测、屏幕↔世界 → 用 [`Self::inverse_transform_point`]（精确）；
    /// - GPU 矩阵（VP / 模型） → 用 [`Self::to_matrix`]`.inverse()`（精确）。
    ///
    /// # Note / 注意
    ///
    /// scale 分量为 0 时产生 inf/NaN（不 panic）。
    #[inline]
    pub fn inverse(&self) -> Self {
        let (sin, cos) = (-self.rotation).sin_cos();
        let inv_scale = Vec2::new(1.0 / self.scale.x, 1.0 / self.scale.y);
        let rotated = Vec2::new(
            self.pos.x * cos - self.pos.y * sin,
            self.pos.x * sin + self.pos.y * cos,
        );
        Self {
            pos: -rotated * inv_scale,
            scale: inv_scale,
            rotation: -self.rotation,
        }
    }

    /// Transform a direction vector from local space to parent space.
    /// 将方向向量从局部空间变换到父级空间（只旋转 + 缩放，不位移）。
    #[inline]
    pub fn transform_vec(&self, local_vec: Vec2) -> Vec2 {
        Vec2::from_angle(self.rotation).rotate(local_vec * self.scale)
    }

    /// Transform a direction vector from parent space back to local space.
    /// 将方向向量从父级空间变换回局部空间（`transform_vec` 的逆）。
    ///
    /// # Note / 注意
    ///
    /// scale 分量为 0 时产生 inf/NaN（不 panic），与 `inverse_transform_point` 一致。
    #[inline]
    pub fn inverse_transform_vec(&self, parent_vec: Vec2) -> Vec2 {
        Vec2::from_angle(-self.rotation).rotate(parent_vec) / self.scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    const EPS: f32 = 1e-4;

    fn sample() -> Transform2D {
        Transform2D::IDENTITY
            .with_pos(Vec2::new(120.0, -80.0))
            .with_rot(0.6)
            .with_scale(Vec2::new(1.5, 0.5))
    }

    #[test]
    fn point_roundtrip_inverse_transform() {
        let t = sample();
        let local = Vec2::new(30.0, -12.0);
        let world = t.transform_point(local);
        let back = t.inverse_transform_point(world);
        assert!(
            (back - local).length() < EPS,
            "roundtrip failed: local={local:?} world={world:?} back={back:?}"
        );
    }

    #[test]
    fn inverse_object_composes_to_identity() {
        let t = sample();
        let composed = t.compose(&t.inverse());
        let p = Vec2::new(5.0, -7.0);
        let mapped = composed.transform_point(p);
        assert!(
            (mapped - p).length() < EPS,
            "t * t⁻¹ should be identity: mapped={mapped:?} vs p={p:?}"
        );
        assert!((composed.pos).length() < EPS, "pos should be ~0, got {:?}", composed.pos);
        assert!((composed.scale - Vec2::ONE).length() < EPS, "scale should be ~1, got {:?}", composed.scale);
        assert!(composed.rotation.abs() < EPS, "rot should be ~0, got {}", composed.rotation);
    }

    #[test]
    fn vec_roundtrip_inverse_transform_vec() {
        let t = sample();
        let local = Vec2::new(3.0, 4.0);
        let world = t.transform_vec(local);
        let back = t.inverse_transform_vec(world);
        assert!(
            (back - local).length() < EPS,
            "vec roundtrip failed: local={local:?} world={world:?} back={back:?}"
        );
    }

    #[test]
    fn compose_inverse_equals_inverse_compose() {
        let parent = sample();
        let child = Transform2D::IDENTITY
            .with_pos(Vec2::new(10.0, 20.0))
            .with_rot(0.3)
            .with_scale(Vec2::splat(2.0));
        let a = child.compose_inverse(&parent);
        let b = parent.inverse().compose(&child);
        let p = Vec2::new(-4.0, 9.0);
        let pa = a.transform_point(p);
        let pb = b.transform_point(p);
        assert!((pa - pb).length() < EPS, "inverse compose mismatch: {pa:?} vs {pb:?}");
    }

    #[test]
    fn inverse_transform_point_used_for_panel_hit_test() {
        // UI 命中检测：世界坐标 → 面板局部坐标
        let panel = Transform2D::IDENTITY
            .with_pos(Vec2::new(100.0, 50.0))
            .with_rot(0.0)
            .with_scale(Vec2::splat(2.0));
        let world = panel.transform_point(Vec2::new(10.0, 20.0));
        let local = panel.inverse_transform_point(world);
        assert!((local - Vec2::new(10.0, 20.0)).length() < EPS, "panel hit-test local mismatch: {local:?}");
    }

    #[test]
    fn move_by_and_move_local_differ_by_rotation() {
        let mut t = Transform2D::IDENTITY.with_rot(std::f32::consts::FRAC_PI_2);
        t.move_by(Vec2::new(10.0, 0.0));
        assert!((t.pos - Vec2::new(10.0, 0.0)).length() < EPS, "世界轴位移不受旋转影响");

        let mut t2 = Transform2D::IDENTITY.with_rot(std::f32::consts::FRAC_PI_2);
        t2.move_local(Vec2::new(10.0, 0.0));
        // 90° 旋转后，局部 +x 指向世界 +y
        assert!((t2.pos - Vec2::new(0.0, 10.0)).length() < EPS, "局部轴位移应被旋转：{}", t2.pos);
    }

    #[test]
    fn to_matrix_matches_transform_point() {
        let t = sample();
        let p = Vec2::new(13.0, -7.5);
        let via_matrix = t.to_matrix() * glam::Vec4::new(p.x, p.y, 0.0, 1.0);
        let via_point = t.transform_point(p);
        assert!(
            (Vec2::new(via_matrix.x, via_matrix.y) - via_point).length() < EPS,
            "to_matrix 与 transform_point 必须一致"
        );
    }
}
