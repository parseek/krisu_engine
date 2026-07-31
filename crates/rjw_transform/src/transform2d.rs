use glam::Vec2;

// 在另一 crate "rjw_transform" 定义
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
            rotation: 0.0
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

    /// Builder: set position. / 构建器模式：设置位置。
    #[inline]
    pub fn with_pos(mut self, pos: Vec2) -> Self {
        self.pos = pos;
        self
    }

    /// Builder: set scale. / 构建器模式：设置缩放。
    #[inline]
    pub fn with_scale(mut self, scale: Vec2) -> Self {
        self.scale = scale;
        self
    }

    /// Builder: set rotation. / 构建器模式：设置旋转。
    #[inline]
    pub fn with_rot(mut self, rot: f32) -> Self {
        self.rotation = rot;
        self
    }

    /// Builder: translate by `pos`. / 构建器模式：位移。
    #[inline]
    pub fn with_move_by(mut self, pos: Vec2) -> Self {
        self.pos += pos;
        self
    }

    /// Builder: translate by rotated `pos`. / 构建器模式：按旋转位移。
    ///
    /// 位移向量 `pos` 先按当前旋转角 `self.rotation` 旋转，再应用到位置。
    /// 等价于 `pos * R(rotation)`，与 `Camera2D::walk_xy` 的旋转约定一致。
    #[inline]
    pub fn with_walk_by(mut self, pos: Vec2) -> Self {
        self.pos += Vec2::from_angle(self.rotation).rotate(pos);
        self
    }

    /// Builder: scale by `scale`. / 构建器模式：缩放。
    #[inline]
    pub fn with_scale_by(mut self, scale: Vec2) -> Self {
        self.scale *= scale;
        self
    }

    /// Builder: rotate by `rot` radians. / 构建器模式：旋转（弧度）。
    #[inline]
    pub fn with_rotate_by(mut self, rot: f32) -> Self {
        self.rotation += rot;
        self
    }

    /// Compose with a parent transform: `result = parent * self`.
    /// 与父级变换组合：`result = parent * self`。
    ///
    /// The child is first rotated, scaled, translated in its own local space,
    /// then placed into the parent's space.
    /// 子级先在其局部空间旋转、缩放、平移，然后放入父级空间。
    ///
    /// Mathematically / 数学表达式:
    ///   `result.pos  = parent.pos + rotate(self.pos, parent.rot) * parent.scale`  
    ///   `result.scale = self.scale * parent.scale`  
    ///   `result.rot   = self.rotation + parent.rot`
    #[inline]
    pub fn with_transform(&self, parent: &Transform2D) -> Self {
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

    /// Convenience: compose with raw components. / 便捷方法：与原始组件组合。
    #[inline]
    pub fn transform_components(&self, pos: Vec2, scale: Vec2, rotation: f32) -> Self {
        self.with_transform(&Self { pos, scale, rotation })
    }

    /// Transform a point from this entity's local space to parent space.
    /// 将点从实体的局部空间变换到父级空间。
    ///
    /// `world_point = pos + rotate(local_point * scale, rot)`
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

    /// Inverse: transform a point from parent space back to local space.
    /// 反向变换：将点从父级空间变换回局部空间。
    ///
    /// `local_point = rotate(world_point - pos, -rot) / scale`
    #[inline]
    pub fn inverse_transform_point(&self, world_point: Vec2) -> Vec2 {
        let (sin, cos) = (-self.rotation).sin_cos();
        let translated = world_point - self.pos;
        Vec2::new(
            (translated.x * cos - translated.y * sin) / self.scale.x,
            (translated.x * sin + translated.y * cos) / self.scale.y,
        )
    }

    /// Return the inverse transform object.
    /// 返回逆变换对象。
    ///
    /// `self.with_transform(&self.inverse()) ≈ IDENTITY`（忽略浮点误差）。
    /// 数学：`scale' = 1/scale`、`rot' = -rot`、`pos' = -rotate(pos, -rot) / scale`。
    ///
    /// # Note / 注意
    ///
    /// 与 `inverse_transform_point` 的约定一致：scale 分量为 0 时产生 inf/NaN（不 panic）。
    #[inline]
    pub fn inverse(&self) -> Self {
        let (sin, cos) = (-self.rotation).sin_cos();
        let inv_scale = Vec2::new(1.0 / self.scale.x, 1.0 / self.scale.y);
        // pos' = -rotate(pos, -rot) / scale = -(rotate(pos, -rot) * inv_scale)
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
    ///
    /// `parent_vec = rotate(local_vec * scale, rot)`
    #[inline]
    pub fn transform_vec(&self, local_vec: Vec2) -> Vec2 {
        Vec2::from_angle(self.rotation).rotate(local_vec * self.scale)
    }

    /// Transform a direction vector from parent space back to local space.
    /// 将方向向量从父级空间变换回局部空间（`transform_vec` 的逆）。
    ///
    /// `local_vec = rotate(parent_vec, -rot) / scale`
    ///
    /// # Note / 注意
    ///
    /// scale 分量为 0 时产生 inf/NaN（不 panic），与 `inverse_transform_point` 一致。
    #[inline]
    pub fn inverse_transform_vec(&self, parent_vec: Vec2) -> Vec2 {
        Vec2::from_angle(-self.rotation).rotate(parent_vec) / self.scale
    }

    /// Inverse parent compose: `result = parent⁻¹ * self`.
    /// 反父级组合：`result = parent⁻¹ * self`。
    ///
    /// 把当前变换放入 `parent` 的**逆**空间（等价 `parent.inverse().with_transform(self)`）。
    /// 用于 UI：把子元素的变换转换到某个祖先面板的局部坐标系，以做命中检测 / 布局。
    #[inline]
    pub fn with_inverse_transform(&self, parent: &Transform2D) -> Self {
        parent.inverse().with_transform(self)
    }

    /// Convenience: inverse compose with raw components. / 便捷方法：与原始组件反组合。
    #[inline]
    pub fn inverse_transform_components(&self, pos: Vec2, scale: Vec2, rotation: f32) -> Self {
        self.with_inverse_transform(&Self { pos, scale, rotation })
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
        let composed = t.with_transform(&t.inverse());
        let p = Vec2::new(5.0, -7.0);
        let mapped = composed.transform_point(p);
        assert!(
            (mapped - p).length() < EPS,
            "t * t⁻¹ should be identity: mapped={mapped:?} vs p={p:?}"
        );
        // 对象级逆也应基本还原字段。
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
    fn inverse_compose_equals_inverse_with_transform() {
        // with_inverse_transform(parent) 应等价于 parent.inverse().with_transform(self)
        let parent = sample();
        let child = Transform2D::IDENTITY
            .with_pos(Vec2::new(10.0, 20.0))
            .with_rot(0.3)
            .with_scale(Vec2::splat(2.0));
        let a = child.with_inverse_transform(&parent);
        let b = parent.inverse().with_transform(&child);
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
        // 面板局部 (10, 20) 在世界空间 → 再反变换回来应等于 (10, 20)
        let world = panel.transform_point(Vec2::new(10.0, 20.0));
        let local = panel.inverse_transform_point(world);
        assert!((local - Vec2::new(10.0, 20.0)).length() < EPS, "panel hit-test local mismatch: {local:?}");
    }
}
