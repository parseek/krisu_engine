//! 2D 批渲染器：Sprite / Mesh 统一管线，按 (layer, states) 排序合批。
//!
//! # 设计总览（职责分离）
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`draw`] | 录制：`Draw2D<K>` 统一 Builder（4 个 kind）+ 流式 sink |
//! | [`sort`] | 排序：对**命令索引数组**重排的策略（`SortMode` / `SortPolicy` / `SortKey`） |
//! | [`cull`] | 剔除：可见性判定 + 对**索引数组**过滤（`Cull` / `Culler` + 纯几何） |
//! | [`render2d`] | 门面：`Render2D`（录制入口 / 全局状态 / 提交 / 资源） |
//! | [`command`] | 命令队列（命令 + layer + states + 索引数组），不含策略 |
//! | [`data`] | 几何/数据类型（`SpriteRect` / `VertexP3U2C4` / `MeshStorage` / `MeshSink`） |
//! | [`draw_page`] | GPU 实例分页缓冲与统一管线缓存 |
//! | [`debug_draw`] | 调试图元（`DebugStyle` + `DebugPainter`：线段 / 矩形框 / 圆 / 十字 / 网格） |
//!
//! # 最小用法
//!
//! ```no_run
//! # use rjw_2d_render::{ArcTextureWrapped, Clear, Color, Render2D, SpriteRect};
//! # use rjw_transform::{Camera2D, Rect, Transform2D};
//! # use rjw_render::RenderContext;
//! # let (mut r2d, mut ctx, tex, cam): (Render2D, RenderContext, ArcTextureWrapped, Camera2D) = unimplemented!();
//! // 取一帧（`None` = 本帧不可呈现 ⇒ 跳过；逻辑可继续）
//! let Some(mut frame) = ctx.acquire_frame() else { return };
//!
//! // 贴纹理精灵：入口 2 参，其余全在链上（默认 IDENTITY / WHITE / layer 0）
//! r2d.sprite(SpriteRect::new((-50.0, -50.0), (100.0, 100.0)), &tex)
//!     .tint(Color::WHITE)
//!     .transform(Transform2D::IDENTITY.with_pos(glam::Vec2::new(10.0, 20.0)))
//!     .layer(1.0);
//!
//! // 纯色多边形（流式构造，零临时 Vec）
//! r2d.polygon_with(|p| {
//!     p.vertex(glam::Vec2::new(0.0, 0.0));
//!     p.vertex(glam::Vec2::new(40.0, 0.0));
//!     p.vertex(glam::Vec2::new(20.0, 30.0));
//! })
//! .tint(Color::CYAN);
//!
//! // 一个画面 = 一次 submit(相机, clear) = 一个 pass（深度附件需求自动推导）
//! let mut pass = frame.pass(Clear::default());
//! r2d.submit(&mut pass, &cam);
//! drop(pass);
//!
//! frame.present();
//! ```
//!
//! # 画面与相机
//!
//! - **相机由调用方持有**（`Render2D` 不含相机状态）：`submit(.., &cam)` 把画面矩形写回
//!   `cam.region` 并读取 `cam.vp_matrix()`；
//! - 一帧内多次 `frame.pass(..)` + `submit` = 多个画面，每个画面在**帧级 VP 槽环**里占
//!   一个独立槽（动态偏移绑定），互不串味。见 `examples/egMultiView`。
//!
//! 坐标系（与 `rjw_transform::Camera2D` 一致）：原点在画面中心、X+ 为右、Y+ 为下。

// ─── 模块声明 ─────────────────────────────────────────────────

pub mod command;
pub mod cull;
pub mod data;
pub mod debug_draw;
pub mod draw;
pub mod draw_page;
pub mod render2d;
pub mod rstates;
pub mod sort;

// ─── 对外重导出 ───────────────────────────────────────────────

pub use command::Layer;
pub use cull::{Cull, Culler, sprite_world_aabb, transform2d_model, viewport_world_rect};
pub use data::{Edges, SpriteRect, Vertex, VertexP3U2C4};
pub use draw_page::{DEPTH_FORMAT, MAX_INSTANCES_PER_DRAW};
pub use draw::{
    Custom, CustomDraw, Draw2D, DrawKind, Mesh, PolygonSink, QuadSink, Sprite, StaticMesh,
};
pub use render2d::{
    CustomBuilder, MeshBuilder, Render2D, SpriteBuilder, StaticMeshBuilder,
};
// 帧 / pass 生命周期（实现于 rjw_render：帧级资源 + pass 边界）。
pub use rjw_render::{Clear, PassBuilder, PassContext, PassRecorder, RenderFrame, RenderTarget};
pub use rstates::{
    AddressMode, BlendDesc, BlendMode, CompareFunc, CullMode, DepthState, FilterMode,
    FrontFaceWinding, PolygonMode, RStates, RasterState, SamplerDesc, StencilState,
};
pub use sort::{SortKey, SortMode, SortPolicy};

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

        {
            // 作用域结束即释放对 storage 的借用（`MeshSink` 无 `Drop`）。
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
        }

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
        {
            // 作用域结束即释放借用（`MeshSink` 无 `Drop`）。
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
        }
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
        Camera2D::full(glam::Vec2::new(W, H))
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
        let rect = SpriteRect::new((-50.0, -50.0), (100.0, 100.0));
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
        let rect = SpriteRect::new((-10.0, -10.0), (20.0, 20.0));
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
        let rect = SpriteRect::new((-50.0, -50.0), (100.0, 100.0));
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
        let rect = SpriteRect::new(glam::Vec2::ZERO, (16.0, 16.0));
        let id = draw_page::InstanceData::from_sprite(&rect, Color::WHITE, tf);
        let expected = model_matrix(&tf).to_cols_array_2d();
        for (row, exp_row) in expected.iter().enumerate() {
            for (col, exp) in exp_row.iter().enumerate() {
                assert!(
                    (id.model[row][col] - exp).abs() < EPS,
                    "model mismatch at [{row}][{col}]: {} vs {}",
                    id.model[row][col],
                    exp
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

/// `SpriteRect` 像素 UV 单元测试：像素 → 归一化换算、链式收窄（世界 / 归一化 UV）。
#[cfg(test)]
mod pixel_uv_tests {
    use super::*;

    const EPS: f32 = 1e-4;

    /// 64×64 纹理的整张贴图精灵
    fn px64() -> SpriteRect {
        SpriteRect::with_uv_px(
            glam::Vec2::ZERO,
            (32.0, 32.0),
            glam::Vec2::ZERO,
            (64.0, 64.0),
            (64.0, 64.0),
        )
    }

    #[test]
    fn full_texture_px_maps_to_unit_uv() {
        let r = px64();
        assert_eq!(r.uv_tl, glam::Vec2::ZERO);
        assert!((r.uv_wh - glam::Vec2::ONE).length() < EPS);
        // 世界矩形原样保留
        assert_eq!(r.mesh_tl, glam::Vec2::ZERO);
        assert_eq!(r.mesh_wh, glam::Vec2::splat(32.0));
    }

    #[test]
    fn pixel_subregion_normalizes_correctly() {
        let r = SpriteRect::with_uv_px(
            (-16.0, -8.0),
            (32.0, 16.0),
            (16.0, 32.0),
            (64.0, 32.0),
            (256.0, 128.0),
        );
        assert!((r.uv_tl - glam::Vec2::new(16.0 / 256.0, 32.0 / 128.0)).length() < EPS);
        assert!((r.uv_wh - glam::Vec2::new(64.0 / 256.0, 32.0 / 128.0)).length() < EPS);
    }

    #[test]
    fn uv_px_chain_touches_uv_only() {
        let r = SpriteRect::new((10.0, 20.0), (32.0, 32.0))
            .uv_px((8.0, 8.0), (16.0, 16.0), (64.0, 64.0));
        assert_eq!(r.mesh_tl, glam::Vec2::new(10.0, 20.0));
        assert_eq!(r.mesh_wh, glam::Vec2::splat(32.0));
        assert!((r.uv_tl - glam::Vec2::splat(0.125)).length() < EPS);
        assert!((r.uv_wh - glam::Vec2::splat(0.25)).length() < EPS);
    }

    #[test]
    fn zero_texture_size_stays_finite() {
        let r = SpriteRect::with_uv_px(
            glam::Vec2::ZERO,
            (1.0, 1.0),
            glam::Vec2::ZERO,
            (1.0, 1.0),
            glam::Vec2::ZERO,
        );
        assert!(r.uv_tl.is_finite() && r.uv_wh.is_finite(), "{r:?}");
    }

    #[test]
    fn shrink_narrows_world_rect_keeping_uv() {
        let r = px64();
        let s = r.shrink(8.0);
        assert_eq!(s.mesh_tl, glam::Vec2::splat(8.0));
        assert_eq!(s.mesh_wh, glam::Vec2::splat(16.0));
        assert_eq!(s.uv_tl, r.uv_tl);
        assert_eq!(s.uv_wh, r.uv_wh);
        // 分轴：左右各 4、上下各 2
        let xy = r.shrink((4.0, 2.0));
        assert_eq!(xy.mesh_tl, glam::Vec2::new(4.0, 2.0));
        assert_eq!(xy.mesh_wh, glam::Vec2::new(24.0, 28.0));
        // 负值即外扩（不 clamp，越界由调用方负责）
        let out = r.shrink(Edges::new().left(-4.0));
        assert_eq!(out.mesh_tl.x, -4.0);
        assert_eq!(out.mesh_wh.x, 36.0);
    }

    #[test]
    fn shrink_uv_uses_normalized_units_keeping_mesh() {
        let r = SpriteRect::with_uv((0.0, 0.0), (32.0, 32.0), (0.25, 0.5), (0.5, 0.25));
        let s = r.shrink_uv(Edges::lrtb(0.125, 0.0, 0.0, 0.125));
        assert!((s.uv_tl - glam::Vec2::new(0.375, 0.5)).length() < EPS, "{:?}", s.uv_tl);
        assert!((s.uv_wh - glam::Vec2::new(0.375, 0.125)).length() < EPS, "{:?}", s.uv_wh);
        assert_eq!(s.mesh_tl, r.mesh_tl);
        assert_eq!(s.mesh_wh, r.mesh_wh);
    }
}
