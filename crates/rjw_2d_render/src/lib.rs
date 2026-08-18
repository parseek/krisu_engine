//! 2D Render using the Batching technique
//!
//! # Batch2D 架构
//!
//! 用户通过 `add_sprite2d*` / `add_mesh` 录制绘制命令 → 命令按 (layer, states) 排序 →
//! `render()` 或 `flush()` 中按相同状态合批，一次性写入实例缓冲并提交渲染。
//!
//! 坐标系（与 `rjw_transform::Camera2D` 一致）：原点在视口中心、X+ 为右、Y+ 为下。
//!
//! # 模块划分
//!
//! - [`data`]：几何/数据类型（`SpriteRect`/`SpriteRectPx`/`VertexP3U2C4`/`Index`/`TriIndicies`/`MeshStorage`/`MeshSink`）
//! - [`command`]：绘制命令枚举、层级 `Layer`、状态与命令队列
//! - [`draw_page`]：GPU 实例数据/缓冲页（`InstanceData`/`VPBuffer`/`MeshDrawItem`/`DrawOp`/`DrawPage`）
//! - [`render2d`]：`Render2D` 渲染器主体 + `ClearConfig`
//! - [`debug_draw`]：调试图元（线段 / 矩形框 / 圆 / 十字 / 网格）
//! - 纹理类型（`TextureWrapped`/`ArcTextureWrapped`）已移入 [`rjw_render`] crate
//!
//! [`rjw_render`]: https://docs.rs/rjw_render

// ─── 模块声明 ─────────────────────────────────────────────────

pub mod command;
pub mod data;
pub mod debug_draw;
pub mod draw_page;
pub mod render2d;
pub mod rstates;

// ─── 对外重导出（保持既有 API 不变） ──────────────────────────

pub use command::Layer;
pub use data::{Index, MeshSink, SpriteRect, SpriteRectPx, TriIndicies, Vertex, VertexP3U2C4};
pub use draw_page::MAX_INSTANCES_PER_DRAW;
pub use render2d::{
    ClearConfig, CustomBuilder, CustomDraw, MeshBuilder, Render2D, Sprite2DBuilder,
    StaticMeshBuilder,
};
pub use rstates::{
    AddressMode, BlendDesc, BlendMode, CompareFunc, CullMode, DepthState, FilterMode,
    FrontFaceWinding, PolygonMode, RStates, RasterState, SamplerDesc, StencilState,
};

// 纹理 / 网格 / 注册表类型重导出（兼容旧路径并暴露静态网格 API）。
pub use rjw_render::{
    ArcTextureWrapped, HasUid, MeshData, MESHES, TextureWrapped, TypedRegistry,
};

pub use rjw_color as color;
pub use rjw_color::{Color, ColorF64};

// ─── 单元测试 ─────────────────────────────────────────────────

/// `MeshSink` 重定位逻辑单元测试（无 GPU 依赖）：
/// 验证 `add_mesh_fn` 闭包写入的**局部索引**经 `push_tri` 正确重定位为**全局索引**。
#[cfg(test)]
mod mesh_sink_tests {
    use super::*;

    /// 构造一个空的 MeshStorage + MeshSink，验证 push 与重定位。
    #[test]
    fn push_tri_relocates_local_to_global() {
        let mut storage = data::MeshStorage::default();
        // 模拟第二个 mesh 从全局顶点 4 开始（前面已有 4 个顶点）。
        storage.vertices.resize(4, VertexP3U2C4::default());
        storage.tri_indices.clear();

        let mut sink = data::MeshSink {
            base: 4,
            verts: &mut storage.vertices,
            tris: &mut storage.tri_indices,
            color_arr: [1.0, 0.0, 0.0, 1.0],
        };

        let a = sink.push_vertex(glam::Vec2::new(0.0, 0.0));
        let b = sink.push_vertex(glam::Vec2::new(1.0, 0.0));
        let c = sink.push_vertex(glam::Vec2::new(0.0, 1.0));
        assert_eq!(
            [a, b, c],
            [0, 1, 2],
            "push_vertex should return local indices"
        );

        sink.push_tri(0, 1, 2);
        drop(sink);

        // 全局索引应 +4。
        assert_eq!(storage.tri_indices.len(), 1);
        let tri = storage.tri_indices[0];
        assert_eq!(tri.0.0, 4);
        assert_eq!(tri.1.0, 5);
        assert_eq!(tri.2.0, 6);

        // 顶点颜色取录制颜色。
        assert_eq!(storage.vertices[4].color, [1.0, 0.0, 0.0, 1.0]);
    }

    /// 验证 `push_vertex_uv_color` 写入逐顶点颜色（不取录制颜色）。
    #[test]
    fn push_vertex_uv_color_sets_vertex_color() {
        let mut storage = data::MeshStorage::default();
        let mut sink = data::MeshSink {
            base: 0,
            verts: &mut storage.vertices,
            tris: &mut storage.tri_indices,
            color_arr: [0.0, 1.0, 0.0, 1.0],
        };
        let idx = sink.push_vertex_uv_color(
            glam::Vec2::new(10.0, 20.0),
            glam::Vec2::new(0.25, 0.75),
            [1.0, 0.0, 0.0, 0.5],
        );
        assert_eq!(idx, 0);
        drop(sink);
        let v = storage.vertices[0];
        assert_eq!(v.pos, [10.0, 20.0, 0.0]);
        assert_eq!(v.uv, [0.25, 0.75]);
        assert_eq!(v.color, [1.0, 0.0, 0.0, 0.5]);
    }
}

/// 矩阵数学单元测试（无 GPU 依赖）。
#[cfg(test)]
mod matrix_tests {
    use super::*;
    use rjw_color::Color;
    use rjw_transform::{Camera2D, Transform2D};

    const W: f32 = 1280.0;
    const H: f32 = 720.0;
    const EPS: f32 = 1e-3;

    fn camera() -> Camera2D {
        let mut c = Camera2D::new(glam::Vec2::new(W, H));
        c.set_vp(glam::Vec2::new(W, H), glam::Vec2::ZERO);
        c
    }

    /// 构造与 `InstanceData::from_sprite` 相同的 2D model 矩阵（列主序）。
    fn model_matrix(transform: &Transform2D) -> glam::Mat4 {
        let (sin, cos) = transform.rotation.sin_cos();
        glam::Mat4::from_cols_array_2d(&[
            [cos * transform.scale.x, sin * transform.scale.x, 0.0, 0.0],
            [-sin * transform.scale.y, cos * transform.scale.y, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [transform.pos.x, transform.pos.y, 0.0, 1.0],
        ])
    }

    /// 复刻 shader vs_main：mesh_pos = mesh_tl + pos.xy * mesh_wh；clip = vp * model * vec4(mesh_pos, 0, 1)。
    fn mesh_ndc(
        vp: glam::Mat4,
        model: glam::Mat4,
        rect: &SpriteRect,
        local: glam::Vec2,
    ) -> glam::Vec2 {
        let mesh_pos = rect.mesh_tl + local * rect.mesh_wh;
        let clip = vp * model * glam::Vec4::new(mesh_pos.x, mesh_pos.y, 0.0, 1.0);
        glam::Vec2::new(clip.x / clip.w, clip.y / clip.w)
    }

    #[test]
    fn sprite_identity_at_center_maps_to_ndc_center() {
        // 单位变换、精灵中心在原点的顶点（local=(0.5,0.5)）应映射到 NDC 中心 (0,0)
        let vp = camera().vp_matrix();
        let model = model_matrix(&Transform2D::default());
        let rect = SpriteRect::from_texture(glam::Vec2::splat(-50.0), glam::Vec2::splat(100.0));
        let ndc = mesh_ndc(vp, model, &rect, glam::Vec2::new(0.5, 0.5));
        assert!(
            (ndc - glam::Vec2::ZERO).length() < EPS,
            "center mesh should map to NDC center, got {ndc:?}"
        );
    }

    #[test]
    fn world_positive_y_up_maps_to_ndc_negative() {
        // 主轴校验：世界 y = +half_h（视口底边）→ NDC y = -1；世界 y = -half_h（顶边）→ NDC y = +1
        let vp = camera().vp_matrix();
        let clip_bottom = vp * glam::Vec4::new(0.0, H * 0.5, 0.0, 1.0);
        let clip_top = vp * glam::Vec4::new(0.0, -H * 0.5, 0.0, 1.0);
        assert!(
            ((clip_bottom.y / clip_bottom.w) + 1.0).abs() < EPS,
            "bottom(+y) should be NDC -1, got {}",
            clip_bottom.y / clip_bottom.w
        );
        assert!(
            ((clip_top.y / clip_top.w) - 1.0).abs() < EPS,
            "top(-y) should be NDC +1, got {}",
            clip_top.y / clip_top.w
        );
    }

    #[test]
    fn sprite_translated_positive_x_y_appears_right_bottom() {
        // 平移 (100, 80) 的精灵中心 → NDC 右下（x>0, y<0，因 y 上为负）
        let vp = camera().vp_matrix();
        let model = model_matrix(&Transform2D::default().with_pos(glam::Vec2::new(100.0, 80.0)));
        let rect = SpriteRect::from_texture(glam::Vec2::splat(-10.0), glam::Vec2::splat(20.0));
        let ndc = mesh_ndc(vp, model, &rect, glam::Vec2::new(0.5, 0.5));
        assert!(
            ndc.x > 0.0,
            "translated +x should be NDC x>0, got {}",
            ndc.x
        );
        assert!(
            ndc.y < 0.0,
            "world +y (down) should be NDC y<0, got {}",
            ndc.y
        );
    }

    #[test]
    fn sprite_rotation_revolves_around_center() {
        // 旋转 90°：中心点不变（绕中心旋转）
        let vp = camera().vp_matrix();
        let model = model_matrix(&Transform2D::default().with_rot(std::f32::consts::PI / 2.0));
        let rect = SpriteRect::from_texture(glam::Vec2::splat(-50.0), glam::Vec2::splat(100.0));
        let ndc_center = mesh_ndc(vp, model, &rect, glam::Vec2::new(0.5, 0.5));
        assert!(
            (ndc_center - glam::Vec2::ZERO).length() < EPS,
            "center should stay at NDC center after rotation, got {ndc_center:?}"
        );
    }

    #[test]
    fn sprite_model_matrix_matches_instance_data() {
        // `InstanceData::from_sprite` 产出的 model（to_cols_array_2d）应与 `model_matrix` 一致
        let tf = Transform2D::default()
            .with_pos(glam::Vec2::new(10.0, -20.0))
            .with_rot(0.5)
            .with_scale(glam::Vec2::new(2.0, 3.0));
        let rect = SpriteRect::from_texture(glam::Vec2::ZERO, glam::Vec2::splat(16.0));
        let id = draw_page::InstanceData::from_sprite(&rect, Color::WHITE, tf);
        let expected = model_matrix(&tf).to_cols_array_2d();
        for row in 0..4 {
            for col in 0..4 {
                assert!(
                    (id.model[row][col] - expected[row][col]).abs() < EPS,
                    "model mismatch at [{row}][{col}]: {} vs {}",
                    id.model[row][col],
                    expected[row][col]
                );
            }
        }
    }

    #[test]
    fn mesh_vertices_use_world_coords_directly() {
        // Mesh 顶点为世界坐标，直接经 vp 变换（无 model）
        let vp = camera().vp_matrix();
        // 世界 (0,0) → NDC (0,0)
        let clip0 = vp * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!((clip0.x / clip0.w).abs() < EPS);
        assert!((clip0.y / clip0.w).abs() < EPS);
        // 世界 y=+200（下）→ NDC y<0
        let clip = vp * glam::Vec4::new(0.0, 200.0, 0.0, 1.0);
        assert!(
            (clip.y / clip.w) < 0.0,
            "world +y should map to NDC y<0 (down), got {}",
            clip.y / clip.w
        );
    }

    #[test]
    fn screen_center_of_sprite_world_aabb() {
        // 与 Camera2D::screen_to_world 交叉验证：世界原点 → 屏幕中心
        let c = camera();
        let center_px = c.world_to_screen(glam::Vec2::ZERO);
        assert!(
            (center_px - glam::Vec2::new(W * 0.5, H * 0.5)).length() < EPS,
            "world origin should map to screen center, got {center_px:?}"
        );
    }
}

/// `SpriteRectPx` 单元测试：像素 UV → 归一化换算、收缩 / 展开（Clamp）/ 越界（不 Clamp）行为。
#[cfg(test)]
mod sprite_rect_px_tests {
    use super::*;

    const EPS: f32 = 1e-4;

    /// 64×64 纹理的整张贴图精灵
    fn px64() -> SpriteRectPx {
        SpriteRectPx::from_texture(
            glam::Vec2::ZERO,
            glam::Vec2::splat(32.0),
            glam::Vec2::splat(64.0),
        )
    }

    #[test]
    fn from_texture_maps_full_region() {
        let r = px64();
        assert_eq!(r.uv_tl, glam::Vec2::ZERO);
        assert_eq!(r.uv_wh, glam::Vec2::splat(64.0));
        let s: SpriteRect = r.into();
        assert_eq!(s.uv_tl, glam::Vec2::ZERO);
        assert!((s.uv_wh - glam::Vec2::ONE).length() < EPS);
        // mesh 透传
        assert_eq!(s.mesh_tl, r.mesh_tl);
        assert_eq!(s.mesh_wh, r.mesh_wh);
    }

    #[test]
    fn pixel_subregion_normalizes_correctly() {
        let r = SpriteRectPx::new(
            glam::Vec2::new(-16.0, -8.0),
            glam::Vec2::new(32.0, 16.0),
            glam::Vec2::new(16.0, 32.0),
            glam::Vec2::new(64.0, 32.0),
            glam::Vec2::new(256.0, 128.0),
        );
        let s = r.to_sprite_rect();
        assert!((s.uv_tl - glam::Vec2::new(16.0 / 256.0, 32.0 / 128.0)).length() < EPS);
        assert!((s.uv_wh - glam::Vec2::new(64.0 / 256.0, 32.0 / 128.0)).length() < EPS);
    }

    #[test]
    fn shrink_uv_is_centered_and_clamped() {
        let r = px64();
        let s = r.shrink_uv_x(8.0);
        assert!((s.uv_tl.x - 8.0).abs() < EPS, "tl.x = {}", s.uv_tl.x);
        assert!((s.uv_wh.x - 48.0).abs() < EPS, "wh.x = {}", s.uv_wh.x);
        // 过窄：clamp 到 0，uv_tl 居中（不翻转）
        let c = r.shrink_uv_x(64.0);
        assert_eq!(c.uv_wh.x, 0.0);
        assert!((c.uv_tl.x - 32.0).abs() < EPS);
    }

    #[test]
    fn expand_clamps_at_texture_bounds() {
        // 子区 (0,16)-(48,32)，纹理 64×64
        let r = SpriteRectPx::new(
            glam::Vec2::ZERO,
            glam::Vec2::splat(32.0),
            glam::Vec2::new(0.0, 16.0),
            glam::Vec2::new(48.0, 32.0),
            glam::Vec2::splat(64.0),
        );
        // 左：tl.x 已贴 0，无可展开
        let e = r.expand_left(1000.0);
        assert_eq!(e.uv_tl.x, 0.0);
        assert!((e.uv_wh.x - 48.0).abs() < EPS);
        // 右：0+48=48，剩 16px
        let e = r.expand_right(1000.0);
        assert!((e.uv_wh.x - 64.0).abs() < EPS, "wh.x = {}", e.uv_wh.x);
        // 下：16+32=48，剩 16px
        let e = r.expand_down(1000.0);
        assert!((e.uv_wh.y - 48.0).abs() < EPS, "wh.y = {}", e.uv_wh.y);
        assert_eq!(e.uv_tl.y, 16.0);
        // 上：tl.y=16，可展开 16px
        let e = r.expand_up(1000.0);
        assert_eq!(e.uv_tl.y, 0.0);
        assert!((e.uv_wh.y - 48.0).abs() < EPS);
    }

    #[test]
    fn exceed_does_not_clamp() {
        let r = px64();
        let e = r.exceed_left(16.0).exceed_down(16.0);
        assert_eq!(e.uv_tl, glam::Vec2::new(-16.0, 0.0));
        assert_eq!(e.uv_wh, glam::Vec2::new(80.0, 80.0));
        let e = r.exceed_up(8.0).exceed_right(8.0);
        assert_eq!(e.uv_tl, glam::Vec2::new(0.0, -8.0));
        assert_eq!(e.uv_wh, glam::Vec2::new(72.0, 72.0));
    }

    #[test]
    fn shrink_single_side_clamps_to_zero() {
        let r = px64();
        let s = r.shrink_left(100.0);
        assert_eq!(s.uv_wh.x, 0.0);
        assert!((s.uv_tl.x - 64.0).abs() < EPS);
        let s = r.shrink_right(100.0);
        assert_eq!(s.uv_wh.x, 0.0);
        assert_eq!(s.uv_tl.x, 0.0);
        let s = r.shrink_down(100.0);
        assert_eq!(s.uv_wh.y, 0.0);
        assert_eq!(s.uv_tl.y, 0.0);
    }

    #[test]
    fn shrink_mesh_mirrors_sprite_rect() {
        let r = px64();
        let s = r.shrink_mesh_x(4.0).shrink_mesh_y(2.0);
        assert!((s.mesh_wh - glam::Vec2::new(24.0, 28.0)).length() < EPS);
        assert!((s.mesh_tl - glam::Vec2::new(4.0, 2.0)).length() < EPS);
        // UV 不受影响
        assert_eq!(s.uv_wh, glam::Vec2::splat(64.0));
    }
}
