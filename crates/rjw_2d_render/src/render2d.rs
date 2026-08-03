//! `Render2D`：Batch2D 渲染器主体 + Clear 配置。

use std::{ops::Range, sync::Arc};

use rjw_color::Color;
use rjw_render::{ArcTextureWrapped, TextureWrapped};
use rjw_transform::Transform2D;

use crate::command::{DrawCommand, DrawCommandQueue, Layer, States};
use crate::data::{
    Index, MeshSink, MeshStorage, SpriteRect, TriIndicies, VertexP3U2C4, QUAD_TRI_INDICIES,
};
use crate::draw_page::{
    DrawOp, DrawPage, InstanceData, MAX_INSTANCES_PER_DRAW, MAX_MESH_VERTS, DEPTH_FORMAT,
};
use crate::rstates::{
    RStates, BlendMode, BlendDesc, FilterMode, AddressMode, SamplerDesc,
    CullMode, PolygonMode, FrontFaceWinding, CompareFunc, DepthState, StencilState, RasterState,
};

// ─── Clear 配置 ───────────────────────────────────────────────

/// 每帧 Clear/Load 配置（`Render2D::render` 使用）。
#[derive(Debug, Clone, Copy)]
pub struct ClearConfig {
    pub color: Option<wgpu::Color>,
    pub depth: Option<f32>,
    pub stencil: Option<u32>,
}

impl Default for ClearConfig {
    fn default() -> Self {
        Self {
            color: Some(wgpu::Color::TRANSPARENT),
            depth: None,
            stencil: None,
        }
    }
}

// ─── Builder：责任链模式 ────────────────────────────────────

/// Sprite 绘制 Builder（由 `add_sprite2d` / `add_sprite2d_solid` 返回）。
///
/// 链方法设置渲染状态；`Drop` 时自动 push 到 `DrawCommandQueue`。
/// 不链式调用 = 使用 `Render2D.default_rstates`（`rstates: None`）。
pub struct Sprite2DBuilder<'a> {
    queue: &'a mut DrawCommandQueue,
    cmd: Option<DrawCommand>,
    layer: Layer,
    rstates: RStates,
    texture_uid: Option<u64>,
    has_rstates: bool,
}

impl<'a> Sprite2DBuilder<'a> {
    pub fn blend(mut self, mode: BlendMode) -> Self { self.rstates = self.rstates.blend(mode); self.has_rstates = true; self }
    pub fn samp_mag(mut self, f: FilterMode) -> Self { self.rstates = self.rstates.samp_mag(f); self.has_rstates = true; self }
    pub fn samp_min(mut self, f: FilterMode) -> Self { self.rstates = self.rstates.samp_min(f); self.has_rstates = true; self }
    pub fn samp_mip(mut self, f: FilterMode) -> Self { self.rstates = self.rstates.samp_mip(f); self.has_rstates = true; self }
    pub fn samp_addr_u(mut self, a: AddressMode) -> Self { self.rstates = self.rstates.samp_addr_u(a); self.has_rstates = true; self }
    pub fn samp_addr_v(mut self, a: AddressMode) -> Self { self.rstates = self.rstates.samp_addr_v(a); self.has_rstates = true; self }
    pub fn samp_addr_w(mut self, a: AddressMode) -> Self { self.rstates = self.rstates.samp_addr_w(a); self.has_rstates = true; self }
    pub fn cull(mut self, c: CullMode) -> Self { self.rstates = self.rstates.cull(c); self.has_rstates = true; self }
    pub fn polygon(mut self, p: PolygonMode) -> Self { self.rstates = self.rstates.polygon(p); self.has_rstates = true; self }
    pub fn front_face(mut self, f: FrontFaceWinding) -> Self { self.rstates = self.rstates.front_face(f); self.has_rstates = true; self }
    pub fn conservative_raster(mut self, b: bool) -> Self { self.rstates = self.rstates.conservative_raster(b); self.has_rstates = true; self }
    pub fn depth_test(mut self, b: bool) -> Self { self.rstates = self.rstates.depth_test(b); self.has_rstates = true; self }
    pub fn depth_write(mut self, b: bool) -> Self { self.rstates = self.rstates.depth_write(b); self.has_rstates = true; self }
    pub fn depth_compare(mut self, f: CompareFunc) -> Self { self.rstates = self.rstates.depth_compare(f); self.has_rstates = true; self }
    pub fn stencil_test(mut self, b: bool) -> Self { self.rstates = self.rstates.stencil_test(b); self.has_rstates = true; self }
    pub fn stencil_write(mut self, b: bool) -> Self { self.rstates = self.rstates.stencil_write(b); self.has_rstates = true; self }
    pub fn stencil_compare(mut self, f: CompareFunc) -> Self { self.rstates = self.rstates.stencil_compare(f); self.has_rstates = true; self }

    pub fn blend_state(mut self, d: BlendDesc) -> Self { self.rstates = self.rstates.blend_state(d); self.has_rstates = true; self }
    pub fn samp_state(mut self, d: SamplerDesc) -> Self { self.rstates = self.rstates.samp_state(d); self.has_rstates = true; self }
    pub fn raster_state(mut self, s: RasterState) -> Self { self.rstates = self.rstates.raster_state(s); self.has_rstates = true; self }
    pub fn depth_state(mut self, s: DepthState) -> Self { self.rstates = self.rstates.depth_state(s); self.has_rstates = true; self }
    pub fn stencil_state(mut self, s: StencilState) -> Self { self.rstates = self.rstates.stencil_state(s); self.has_rstates = true; self }
}

impl Drop for Sprite2DBuilder<'_> {
    fn drop(&mut self) {
        let rstates = if self.has_rstates { Some(self.rstates) } else { None };
        if let Some(cmd) = self.cmd.take() {
            self.queue.push(
                cmd,
                self.layer,
                States { rstates, texture_uid: self.texture_uid },
            );
        }
    }
}

/// Mesh 绘制 Builder（由 `add_mesh` / `add_polygon_fan` 等返回）。
///
/// 在 `Sprite2DBuilder` 基础上增加 `.set_texture()`。
pub struct MeshBuilder<'a> {
    queue: &'a mut DrawCommandQueue,
    cmd: Option<DrawCommand>,
    layer: Layer,
    rstates: RStates,
    texture_uid: Option<u64>,
    has_rstates: bool,
}

impl<'a> MeshBuilder<'a> {
    pub fn set_texture(mut self, texture: &ArcTextureWrapped) -> Self {
        self.texture_uid = Some(texture.uid);
        self
    }

    pub fn blend(mut self, mode: BlendMode) -> Self { self.rstates = self.rstates.blend(mode); self.has_rstates = true; self }
    pub fn samp_mag(mut self, f: FilterMode) -> Self { self.rstates = self.rstates.samp_mag(f); self.has_rstates = true; self }
    pub fn samp_min(mut self, f: FilterMode) -> Self { self.rstates = self.rstates.samp_min(f); self.has_rstates = true; self }
    pub fn samp_mip(mut self, f: FilterMode) -> Self { self.rstates = self.rstates.samp_mip(f); self.has_rstates = true; self }
    pub fn samp_addr_u(mut self, a: AddressMode) -> Self { self.rstates = self.rstates.samp_addr_u(a); self.has_rstates = true; self }
    pub fn samp_addr_v(mut self, a: AddressMode) -> Self { self.rstates = self.rstates.samp_addr_v(a); self.has_rstates = true; self }
    pub fn samp_addr_w(mut self, a: AddressMode) -> Self { self.rstates = self.rstates.samp_addr_w(a); self.has_rstates = true; self }
    pub fn cull(mut self, c: CullMode) -> Self { self.rstates = self.rstates.cull(c); self.has_rstates = true; self }
    pub fn polygon(mut self, p: PolygonMode) -> Self { self.rstates = self.rstates.polygon(p); self.has_rstates = true; self }
    pub fn front_face(mut self, f: FrontFaceWinding) -> Self { self.rstates = self.rstates.front_face(f); self.has_rstates = true; self }
    pub fn conservative_raster(mut self, b: bool) -> Self { self.rstates = self.rstates.conservative_raster(b); self.has_rstates = true; self }
    pub fn depth_test(mut self, b: bool) -> Self { self.rstates = self.rstates.depth_test(b); self.has_rstates = true; self }
    pub fn depth_write(mut self, b: bool) -> Self { self.rstates = self.rstates.depth_write(b); self.has_rstates = true; self }
    pub fn depth_compare(mut self, f: CompareFunc) -> Self { self.rstates = self.rstates.depth_compare(f); self.has_rstates = true; self }
    pub fn stencil_test(mut self, b: bool) -> Self { self.rstates = self.rstates.stencil_test(b); self.has_rstates = true; self }
    pub fn stencil_write(mut self, b: bool) -> Self { self.rstates = self.rstates.stencil_write(b); self.has_rstates = true; self }
    pub fn stencil_compare(mut self, f: CompareFunc) -> Self { self.rstates = self.rstates.stencil_compare(f); self.has_rstates = true; self }

    pub fn blend_state(mut self, d: BlendDesc) -> Self { self.rstates = self.rstates.blend_state(d); self.has_rstates = true; self }
    pub fn samp_state(mut self, d: SamplerDesc) -> Self { self.rstates = self.rstates.samp_state(d); self.has_rstates = true; self }
    pub fn raster_state(mut self, s: RasterState) -> Self { self.rstates = self.rstates.raster_state(s); self.has_rstates = true; self }
    pub fn depth_state(mut self, s: DepthState) -> Self { self.rstates = self.rstates.depth_state(s); self.has_rstates = true; self }
    pub fn stencil_state(mut self, s: StencilState) -> Self { self.rstates = self.rstates.stencil_state(s); self.has_rstates = true; self }
}

impl Drop for MeshBuilder<'_> {
    fn drop(&mut self) {
        let rstates = if self.has_rstates { Some(self.rstates) } else { None };
        if let Some(cmd) = self.cmd.take() {
            self.queue.push(
                cmd,
                self.layer,
                States { rstates, texture_uid: self.texture_uid },
            );
        }
    }
}

// ─── Render2D ─────────────────────────────────────────────────

/// Batch2D 渲染器：命令录制 → 排序合批 → 统一提交
///
/// 统一管线：Sprite 与 Mesh 共用同一 `vs_main` 入口 + slot0/slot1 布局。
/// Mesh 通过"身份实例数据"（mesh_tl=0, mesh_wh=1, model=I）使世界坐标直通 VP。
pub struct Render2D {
    surface: &'static wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,

    tex_bind_group_layout: wgpu::BindGroupLayout,
    white_texture: ArcTextureWrapped,
    textures: Vec<ArcTextureWrapped>,

    mesh_storage: MeshStorage,
    command_queue: DrawCommandQueue,
    draw_page: DrawPage,

    /// 懒创建深度/模板纹理及其视图
    depth_view: Option<wgpu::TextureView>,
    depth_size: (u32, u32),

    /// 当前 VP（列主序，由 `set_mvp` 透传）。
    #[allow(unused)]
    mvp: glam::Mat4,

    /// 全局默认渲染状态（`add_*` 不链式调用时使用）。
    default_rstates: RStates,

    // ── 每帧复用缓冲池（避免堆分配） ──
    buf_mesh_cmds: Vec<(Range<usize>, Range<usize>)>,
    buf_instances: Vec<InstanceData>,
    buf_ops: Vec<DrawOp>,
    buf_all_verts: Vec<VertexP3U2C4>,
    buf_all_tris: Vec<TriIndicies>,
    buf_padded: Vec<u8>,
}

impl Render2D {
    pub fn new(render: &rjw_render::RenderContext) -> Self {
        let device = render.device().clone();
        let queue = render.queue().clone();
        let surface_format = render.format();

        let surface: &'static wgpu::Surface<'static> =
            unsafe { std::mem::transmute(render.surface()) };

        let vp_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vp bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let tex_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
        });

        let draw_page = DrawPage::new(
            &device,
            &vp_bind_group_layout,
            &tex_bind_group_layout,
            shader,
            surface_format,
            MAX_INSTANCES_PER_DRAW,
            glam::Mat4::IDENTITY,
        );

        // 1×1 白色默认纹理（纯色 sprite / mesh 使用）。
        let white_texture = Arc::new(TextureWrapped::from_rgba8(
            &device,
            &queue,
            &tex_bind_group_layout,
            "white",
            &[255, 255, 255, 255],
            1,
            1,
        ));

        Self {
            surface,
            device,
            queue,
            tex_bind_group_layout,
            white_texture,
            textures: Vec::new(),
            mesh_storage: MeshStorage::default(),
            command_queue: DrawCommandQueue::default(),
            draw_page,
            depth_view: None,
            depth_size: (0, 0),
            mvp: glam::Mat4::IDENTITY,
            default_rstates: RStates::default(),
            buf_mesh_cmds: Vec::new(),
            buf_instances: Vec::new(),
            buf_ops: Vec::new(),
            buf_all_verts: Vec::new(),
            buf_all_tris: Vec::new(),
            buf_padded: Vec::new(),
        }
    }

    pub fn set_mvp(&mut self, vp: glam::Mat4) {
        self.mvp = vp;
        self.draw_page.update_vp(&self.queue, vp);
    }

    // ── 全局默认渲染状态（责任链） ──

    pub fn reset_default_state(&mut self) -> &mut Self {
        self.default_rstates = RStates::default();
        self
    }

    pub fn default_blend(&mut self, mode: BlendMode) -> &mut Self { self.default_rstates = self.default_rstates.blend(mode); self }
    pub fn default_samp_mag(&mut self, f: FilterMode) -> &mut Self { self.default_rstates = self.default_rstates.samp_mag(f); self }
    pub fn default_samp_min(&mut self, f: FilterMode) -> &mut Self { self.default_rstates = self.default_rstates.samp_min(f); self }
    pub fn default_samp_mip(&mut self, f: FilterMode) -> &mut Self { self.default_rstates = self.default_rstates.samp_mip(f); self }
    pub fn default_samp_addr_u(&mut self, a: AddressMode) -> &mut Self { self.default_rstates = self.default_rstates.samp_addr_u(a); self }
    pub fn default_samp_addr_v(&mut self, a: AddressMode) -> &mut Self { self.default_rstates = self.default_rstates.samp_addr_v(a); self }
    pub fn default_samp_addr_w(&mut self, a: AddressMode) -> &mut Self { self.default_rstates = self.default_rstates.samp_addr_w(a); self }
    pub fn default_cull(&mut self, c: CullMode) -> &mut Self { self.default_rstates = self.default_rstates.cull(c); self }
    pub fn default_polygon(&mut self, p: PolygonMode) -> &mut Self { self.default_rstates = self.default_rstates.polygon(p); self }
    pub fn default_front_face(&mut self, f: FrontFaceWinding) -> &mut Self { self.default_rstates = self.default_rstates.front_face(f); self }
    pub fn default_conservative_raster(&mut self, b: bool) -> &mut Self { self.default_rstates = self.default_rstates.conservative_raster(b); self }
    pub fn default_depth_test(&mut self, b: bool) -> &mut Self { self.default_rstates = self.default_rstates.depth_test(b); self }
    pub fn default_depth_write(&mut self, b: bool) -> &mut Self { self.default_rstates = self.default_rstates.depth_write(b); self }
    pub fn default_depth_compare(&mut self, f: CompareFunc) -> &mut Self { self.default_rstates = self.default_rstates.depth_compare(f); self }
    pub fn default_stencil_test(&mut self, b: bool) -> &mut Self { self.default_rstates = self.default_rstates.stencil_test(b); self }
    pub fn default_stencil_write(&mut self, b: bool) -> &mut Self { self.default_rstates = self.default_rstates.stencil_write(b); self }
    pub fn default_stencil_compare(&mut self, f: CompareFunc) -> &mut Self { self.default_rstates = self.default_rstates.stencil_compare(f); self }

    pub fn default_blend_state(&mut self, d: BlendDesc) -> &mut Self { self.default_rstates = self.default_rstates.blend_state(d); self }
    pub fn default_samp_state(&mut self, d: SamplerDesc) -> &mut Self { self.default_rstates = self.default_rstates.samp_state(d); self }
    pub fn default_raster_state(&mut self, s: RasterState) -> &mut Self { self.default_rstates = self.default_rstates.raster_state(s); self }
    pub fn default_depth_state(&mut self, s: DepthState) -> &mut Self { self.default_rstates = self.default_rstates.depth_state(s); self }
    pub fn default_stencil_state(&mut self, s: StencilState) -> &mut Self { self.default_rstates = self.default_rstates.stencil_state(s); self }

    pub fn create_texture(&mut self, label: &str, data: &[u8], width: u32, height: u32) -> ArcTextureWrapped {
        let expected = (width as usize) * (height as usize) * 4;
        assert_eq!(
            data.len(),
            expected,
            "RGBA8 data length mismatch: expected {expected} (w*h*4), got {} (label={label})",
            data.len()
        );
        let tex = Arc::new(TextureWrapped::from_rgba8(
            &self.device,
            &self.queue,
            &self.tex_bind_group_layout,
            label,
            data,
            width,
            height,
        ));
        self.textures.push(tex.clone());
        tex
    }

    // ── 绘制命令录制（返回 Builder） ──

    pub fn add_sprite2d(
        &mut self,
        rect: SpriteRect,
        color: Color,
        transform: Transform2D,
        layer: impl Into<Layer>,
        texture: &ArcTextureWrapped,
    ) -> Sprite2DBuilder<'_> {
        Sprite2DBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::Sprite2D { rect, color, transform }),
            layer: layer.into(),
            rstates: RStates::default(),
            texture_uid: Some(texture.uid),
            has_rstates: false,
        }
    }

    pub fn add_sprite2d_solid(
        &mut self,
        rect: SpriteRect,
        color: Color,
        transform: Transform2D,
        layer: impl Into<Layer>,
    ) -> Sprite2DBuilder<'_> {
        let white = self.white_texture.clone();
        self.add_sprite2d(rect, color, transform, layer, &white)
    }

    pub fn add_mesh(
        &mut self,
        vertices: &[glam::Vec2],
        tri_indices: &[u16],
        color: Color,
        layer: impl Into<Layer>,
    ) -> MeshBuilder<'_> {
        assert!(vertices.len() > 0, "mesh requires at least 1 vertex");
        assert!(tri_indices.len() % 3 == 0, "tri_indices length must be a multiple of 3");
        assert!(
            tri_indices.iter().all(|&i| (i as usize) < vertices.len()),
            "tri_indices out of bounds for vertices"
        );
        assert!(
            vertices.len() as u64 <= u16::MAX as u64 + 1,
            "single mesh has too many vertices for u16 indices: {} (max {})",
            vertices.len(),
            u16::MAX as u64 + 1
        );

        let vert_start = self.mesh_storage.vertices.len();
        let tri_start = self.mesh_storage.tri_indices.len();
        let color_arr: [f32; 4] = color.into();

        for p in vertices {
            self.mesh_storage.vertices.push(VertexP3U2C4 {
                pos: [p.x, p.y, 0.0],
                uv: [0.0, 0.0],
                color: color_arr,
            });
        }
        for chunk in tri_indices.chunks_exact(3) {
            self.mesh_storage.tri_indices.push(TriIndicies(
                Index((chunk[0] as u32 + vert_start as u32) as u16),
                Index((chunk[1] as u32 + vert_start as u32) as u16),
                Index((chunk[2] as u32 + vert_start as u32) as u16),
            ));
        }

        let vert_range = vert_start..self.mesh_storage.vertices.len();
        let tri_range = tri_start..self.mesh_storage.tri_indices.len();
        MeshBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::Mesh { vert: vert_range, tri_index: tri_range }),
            layer: layer.into(),
            rstates: RStates::default(),
            texture_uid: None,
            has_rstates: false,
        }
    }

    pub fn add_mesh_fn_prealloc<F>(
        &mut self,
        max_vertices: usize,
        max_triangles: usize,
        color: Color,
        layer: impl Into<Layer>,
        f: F,
    ) -> MeshBuilder<'_>
    where
        F: FnOnce(&mut [VertexP3U2C4], &mut [TriIndicies]) -> (usize, usize),
    {
        assert!(max_vertices > 0, "add_mesh_fn_prealloc requires at least 1 vertex");
        assert!(max_vertices <= MAX_MESH_VERTS, "mesh vertex count exceeds u16 limit");

        let v_off = self.mesh_storage.vertices.len();
        let i_off = self.mesh_storage.tri_indices.len();
        let color_arr: [f32; 4] = color.into();

        self.mesh_storage.vertices.resize(v_off + max_vertices, VertexP3U2C4::default());
        self.mesh_storage.tri_indices.resize(i_off + max_triangles, TriIndicies::default());

        let (used_verts, used_tris) = {
            let v_slice = &mut self.mesh_storage.vertices[v_off..v_off + max_vertices];
            let i_slice = &mut self.mesh_storage.tri_indices[i_off..i_off + max_triangles];
            f(v_slice, i_slice)
        };

        self.mesh_storage.vertices.truncate(v_off + used_verts);
        self.mesh_storage.tri_indices.truncate(i_off + used_tris);

        for v in &mut self.mesh_storage.vertices[v_off..v_off + used_verts] {
            v.color = color_arr;
        }

        if used_tris != 0 {
            let base = v_off as u32;
            for t in &mut self.mesh_storage.tri_indices[i_off..i_off + used_tris] {
                *t = TriIndicies(
                    Index((t.0 .0 as u32 + base) as u16),
                    Index((t.1 .0 as u32 + base) as u16),
                    Index((t.2 .0 as u32 + base) as u16),
                );
            }
        }

        MeshBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::Mesh {
                vert: v_off..v_off + used_verts,
                tri_index: i_off..i_off + used_tris,
            }),
            layer: layer.into(),
            rstates: RStates::default(),
            texture_uid: None,
            has_rstates: false,
        }
    }

    pub fn add_mesh_fn<F>(
        &mut self,
        color: Color,
        layer: impl Into<Layer>,
        f: F,
    ) -> MeshBuilder<'_>
    where
        F: FnOnce(&mut MeshSink<'_>),
    {
        let vert_start = self.mesh_storage.vertices.len();
        let tri_start = self.mesh_storage.tri_indices.len();
        let color_arr: [f32; 4] = color.into();

        {
            let mut sink = MeshSink {
                base: vert_start as u32,
                verts: &mut self.mesh_storage.vertices,
                tris: &mut self.mesh_storage.tri_indices,
                color_arr,
            };
            f(&mut sink);
        }

        let vert_range = vert_start..self.mesh_storage.vertices.len();
        let tri_range = tri_start..self.mesh_storage.tri_indices.len();
        MeshBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::Mesh { vert: vert_range, tri_index: tri_range }),
            layer: layer.into(),
            rstates: RStates::default(),
            texture_uid: None,
            has_rstates: false,
        }
    }

    pub fn add_polygon_fan(
        &mut self,
        vertices: &[glam::Vec2],
        color: Color,
        layer: impl Into<Layer>,
    ) -> MeshBuilder<'_> {
        debug_assert!(vertices.len() >= 3, "polygon requires at least 3 vertices");
        let n = vertices.len();
        self.add_mesh_fn_prealloc(n, n - 2, color, layer, |v_slice, t_slice| {
            for (dst, src) in v_slice.iter_mut().zip(vertices.iter()) {
                dst.pos = [src.x, src.y, 0.0];
            }
            for (i, t) in t_slice.iter_mut().enumerate() {
                *t = TriIndicies::new(0, (i + 1) as u16, (i + 2) as u16);
            }
            (n, n - 2)
        })
    }

    pub fn add_polygon_strip(
        &mut self,
        vertices: &[glam::Vec2],
        color: Color,
        layer: impl Into<Layer>,
    ) -> MeshBuilder<'_> {
        debug_assert!(vertices.len() >= 3, "polygon requires at least 3 vertices");
        let n = vertices.len();
        self.add_mesh_fn_prealloc(n, n - 2, color, layer, |v_slice, t_slice| {
            for (dst, src) in v_slice.iter_mut().zip(vertices.iter()) {
                dst.pos = [src.x, src.y, 0.0];
            }
            for (i, t) in t_slice.iter_mut().enumerate() {
                *t = TriIndicies::new(0, (i + 1) as u16, (i + 2) as u16);
            }
            (n, n - 2)
        })
    }

    #[allow(unused)]
    pub fn white_texture(&self) -> &ArcTextureWrapped {
        &self.white_texture
    }

    // ── 提交 ──

    pub fn render(&mut self, clear: &ClearConfig) {
        let Some((surface_texture, view)) = self.begin_frame() else {
            self.command_queue.clear();
            self.mesh_storage.clear();
            return;
        };

        self.prepare();

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render2d encoder"),
        });

        let needs_depth = clear.depth.is_some() || clear.stencil.is_some();
        let size = self
            .surface
            .get_configuration()
            .map(|c| (c.width, c.height))
            .unwrap_or((1, 1));
        if needs_depth {
            self.ensure_depth(size.0, size.1);
        }
        let depth_view = if needs_depth { self.depth_view.as_ref() } else { None };

        {
            let color_ops = match clear.color {
                Some(c) => wgpu::Operations { load: wgpu::LoadOp::Clear(c), store: wgpu::StoreOp::Store },
                None => wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            };

            let depth_stencil_attachment = match depth_view {
                Some(dv) => Some(wgpu::RenderPassDepthStencilAttachment {
                    view: dv,
                    depth_ops: clear.depth.map(|d| wgpu::Operations {
                        load: wgpu::LoadOp::Clear(d),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: clear.stencil.map(|s| wgpu::Operations {
                        load: wgpu::LoadOp::Clear(s),
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                None => None,
            };

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render2d pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: color_ops,
                })],
                depth_stencil_attachment,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });

            self.draw(&mut pass);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(surface_texture);
        self.command_queue.clear();
        self.mesh_storage.clear();
    }

    pub fn flush(&mut self, pass: &mut wgpu::RenderPass<'_>) {
        self.prepare();
        self.draw(pass);
        self.command_queue.clear();
        self.mesh_storage.clear();
    }

    pub fn begin_frame(&mut self) -> Option<(wgpu::SurfaceTexture, wgpu::TextureView)> {
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return None,
        };
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        Some((texture, view))
    }

    // ── 内部：排序 / 聚合 / 绘制 ──

    fn prepare(&mut self) {
        self.command_queue.sort_layer_then_states();

        self.buf_mesh_cmds.clear();
        self.buf_instances.clear();
        self.buf_ops.clear();
        self.buf_all_verts.clear();
        self.buf_all_tris.clear();

        let mut current_tex: Option<u64> = None;
        let mut current_rst: Option<u64> = None;
        let mut run_start: usize = 0;
        let mut page: u32 = 0;
        let mut page_start: usize = 0;

        macro_rules! close_run {
            () => {{
                if self.buf_instances.len() > run_start {
                    self.buf_ops.push(DrawOp::Sprite {
                        page,
                        tex_uid: current_tex,
                        range: ((run_start - page_start) as u32)..((self.buf_instances.len() - page_start) as u32),
                        rstates: current_rst.unwrap_or(0),
                    });
                }
            }};
        }

        for (cmd, _layer, states) in self.command_queue.iter() {
            let tex_uid = states.texture_uid;
            let raw_rst = states
                .rstates
                .unwrap_or(self.default_rstates)
                .raw();

            match cmd {
                DrawCommand::Sprite2D { rect, color, transform } => {
                    let page_full = self.buf_instances.len() - page_start >= MAX_INSTANCES_PER_DRAW;
                    if tex_uid != current_tex || Some(raw_rst) != current_rst || page_full {
                        close_run!();
                        current_tex = tex_uid;
                        current_rst = Some(raw_rst);
                        run_start = self.buf_instances.len();
                        if page_full {
                            page += 1;
                            page_start = self.buf_instances.len();
                        }
                    }
                    let data = InstanceData::from_sprite(rect, *color, *transform);
                    self.buf_instances.push(data);
                }
                DrawCommand::Mesh { vert, tri_index } => {
                    close_run!();
                    current_tex = None;
                    current_rst = None;
                    run_start = self.buf_instances.len();
                    self.buf_ops.push(DrawOp::MeshPlaceholder);
                    self.buf_mesh_cmds.push((vert.clone(), tri_index.clone()));
                }
            }
        }
        close_run!();

        let page_count = page as usize + 1;
        self.draw_page.ensure_instance_pages(&self.device, page_count);

        {
            let mut page_idx = 0usize;
            let mut start = 0usize;
            while start < self.buf_instances.len() {
                let end = (start + MAX_INSTANCES_PER_DRAW).min(self.buf_instances.len());
                self.draw_page.update_instances_page(&self.queue, page_idx, &self.buf_instances[start..end]);
                page_idx += 1;
                start = end;
            }
        }

        if !self.buf_mesh_cmds.is_empty() {
            let total_verts: usize = self.buf_mesh_cmds.iter().map(|(v, _)| v.end - v.start).sum();
            let total_tris: usize = self.buf_mesh_cmds.iter().map(|(_, t)| t.end - t.start).sum();
            assert!(total_verts <= MAX_MESH_VERTS, "mesh vertices exceed u16 index limit");

            self.draw_page.ensure_mesh_capacity(&self.device, total_verts, total_tris + 1);

            let mut v_cursor: usize = 0;
            let mut i_cursor: usize = 0;
            let mut op_plh = self.buf_ops.iter_mut().filter_map(|op| match op {
                DrawOp::MeshPlaceholder => Some(op),
                _ => None,
            });

            for (vert, tri_index) in &self.buf_mesh_cmds {
                let vcount = vert.end - vert.start;
                let icount = tri_index.end - tri_index.start;

                if vcount != 0 {
                    self.buf_all_verts.extend_from_slice(&self.mesh_storage.vertices[vert.clone()]);
                }
                if vcount != 0 && icount != 0 {
                    let reloc_base = (v_cursor as i64) - (vert.start as i64);
                    for t in &self.mesh_storage.tri_indices[tri_index.clone()] {
                        self.buf_all_tris.push(TriIndicies(
                            Index((t.0 .0 as i64 + reloc_base) as u16),
                            Index((t.1 .0 as i64 + reloc_base) as u16),
                            Index((t.2 .0 as i64 + reloc_base) as u16),
                        ));
                    }
                }

                let item = crate::draw_page::MeshDrawItem {
                    first_vertex: v_cursor as u32,
                    vertex_count: vcount as u16,
                    tri_index_start: i_cursor as u32,
                    tri_index_count: icount as u16,
                };
                if let Some(op) = op_plh.next() {
                    // Mesh rstates 从 command queue 取出（已在排序阶段 resolve）
                    *op = DrawOp::Mesh { item, rstates: 0 };
                }

                v_cursor += vcount;
                i_cursor += icount;
            }

            if !self.buf_all_verts.is_empty() {
                self.queue.write_buffer(&self.draw_page.mesh_vb, 0, bytemuck::cast_slice(&self.buf_all_verts));
            }
            if !self.buf_all_tris.is_empty() {
                let bytes = bytemuck::cast_slice(&self.buf_all_tris);
                let padded_len = (bytes.len() + 3) & !3;
                self.buf_padded.clear();
                self.buf_padded.extend_from_slice(bytes);
                self.buf_padded.resize(padded_len, 0u8);
                self.queue.write_buffer(&self.draw_page.mesh_ib, 0, &self.buf_padded);
            }
        }
    }

    fn draw(&mut self, pass: &mut wgpu::RenderPass<'_>) {
        if self.buf_ops.is_empty() {
            return;
        }

        let mut i = 0usize;
        while i < self.buf_ops.len() {
            match &self.buf_ops[i] {
                DrawOp::Sprite { page, tex_uid, range, rstates } => {
                    let count = range.end - range.start;
                    if count != 0 {
                        let pipeline = self.draw_page.get_or_create_pipeline(&self.device, *rstates);
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, &self.draw_page.vp_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.draw_page.quad_vb.slice(..));
                        pass.set_vertex_buffer(1, self.draw_page.instance_page_buffer(*page as usize).slice(..));
                        pass.set_index_buffer(self.draw_page.quad_ib.slice(..), wgpu::IndexFormat::Uint16);

                        let bg = match tex_uid {
                            Some(uid) if *uid == self.white_texture.uid => &self.white_texture.bind_group,
                            Some(uid) => {
                                let tex = self.textures.iter().find(|t| t.uid == *uid).expect("texture uid not found");
                                &tex.bind_group
                            }
                            None => &self.white_texture.bind_group,
                        };
                        pass.set_bind_group(1, bg, &[]);
                        pass.draw_indexed(0..QUAD_TRI_INDICIES.len() as u32, 0, range.clone());
                    }
                    i += 1;
                }
                DrawOp::Mesh { item: _, rstates } => {
                    let mut seg_start: Option<u32> = None;
                    let mut seg_count: u32 = 0;
                    let mut seg_rstates = *rstates;

                    while i < self.buf_ops.len() {
                        match &self.buf_ops[i] {
                            DrawOp::Mesh { item, rstates: r } => {
                                if item.tri_index_count != 0 {
                                    if seg_start.is_none() {
                                        seg_start = Some(item.tri_index_start * 3);
                                        seg_rstates = *r;
                                    }
                                    seg_count += item.tri_index_count as u32 * 3;
                                }
                                i += 1;
                            }
                            _ => break,
                        }
                    }

                    if let Some(start) = seg_start {
                        let pipeline = self.draw_page.get_or_create_pipeline(&self.device, seg_rstates);
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, &self.draw_page.vp_bind_group, &[]);
                        pass.set_bind_group(1, &self.white_texture.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.draw_page.mesh_vb.slice(..));
                        pass.set_vertex_buffer(1, self.draw_page.identity_instance_buffer().slice(..));
                        pass.set_index_buffer(self.draw_page.mesh_ib.slice(..), wgpu::IndexFormat::Uint16);
                        pass.draw_indexed(start..start + seg_count, 0, 0..1);
                    }
                }
                DrawOp::MeshPlaceholder => {
                    debug_assert!(false, "unreplaced MeshPlaceholder in buf_ops");
                    i += 1;
                }
            }
        }
    }

    fn ensure_depth(&mut self, width: u32, height: u32) {
        let inited = self
            .depth_view
            .as_ref()
            .is_some_and(|_| self.depth_size == (width.max(1), height.max(1)));
        if inited {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render2d depth-stencil"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.depth_view = Some(view);
        self.depth_size = (width.max(1), height.max(1));
    }

    #[allow(unused)]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    #[allow(unused)]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}