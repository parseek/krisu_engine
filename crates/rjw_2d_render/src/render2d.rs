//! `Render2D`：Batch2D 渲染器主体 + Clear 配置。

use std::{ops::Range, sync::Arc};

use rjw_color::Color;
use rjw_render::{ArcTextureWrapped, TextureWrapped, TEXTURES};
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

#[derive(Debug, Clone, Copy)]
pub struct ClearConfig {
    pub color: Option<wgpu::Color>,
    pub depth: Option<f32>,
    pub stencil: Option<u32>,
}

impl Default for ClearConfig {
    fn default() -> Self {
        Self { color: Some(wgpu::Color::BLACK), depth: None, stencil: None }
    }
}

// ─── Builder：责任链模式 ────────────────────────────────────

pub struct Sprite2DBuilder<'a> {
    queue: &'a mut DrawCommandQueue, cmd: Option<DrawCommand>,
    layer: Layer, rstates: RStates, texture_uid: Option<u64>, has_rstates: bool,
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
            self.queue.push(cmd, self.layer, States { rstates, texture_uid: self.texture_uid });
        }
    }
}

pub struct MeshBuilder<'a> {
    queue: &'a mut DrawCommandQueue, cmd: Option<DrawCommand>,
    layer: Layer, rstates: RStates, texture_uid: Option<u64>, has_rstates: bool,
}

impl<'a> MeshBuilder<'a> {
    pub fn set_texture(mut self, texture: &ArcTextureWrapped) -> Self { self.texture_uid = Some(texture.uid); self }
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
            self.queue.push(cmd, self.layer, States { rstates, texture_uid: self.texture_uid });
        }
    }
}

/// 外部绘制 trait：实现此 trait 的结构体/闭包可通过 `add_custom` 注入绘制队列。
/// 渲染器持有 `Arc<dyn CustomDraw>`，`draw()` 中可安全共享引用。
pub trait CustomDraw: Send + Sync {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>);
}

/// 闭包的 blanket impl——直接传 `|pass| { ... }` 即可。
impl<F: Fn(&mut wgpu::RenderPass<'_>) + Send + Sync> CustomDraw for F {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) { self(pass); }
}

/// 外部绘制 Builder（由 `add_custom` 返回）。
/// 链式设置 RStates；Drop 时 push `DrawCommand::Custom`。
pub struct CustomBuilder<'a> {
    queue: &'a mut DrawCommandQueue,
    layer: Layer, rstates: RStates, has_rstates: bool,
}

impl<'a> CustomBuilder<'a> {
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

impl Drop for CustomBuilder<'_> {
    fn drop(&mut self) {
        let rstates = if self.has_rstates { Some(self.rstates) } else { None };
        self.queue.push(DrawCommand::Custom, self.layer, States { rstates, texture_uid: None });
    }
}

// ─── Render2D ─────────────────────────────────────────────────

pub struct Render2D {
    surface: &'static wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    tex_bind_group_layout: wgpu::BindGroupLayout,
    white_texture: ArcTextureWrapped,
    mesh_storage: MeshStorage,
    command_queue: DrawCommandQueue,
    draw_page: DrawPage,
    depth_view: Option<wgpu::TextureView>,
    depth_size: (u32, u32),
    mvp: glam::Mat4,
    default_rstates: RStates,

    buf_mesh_cmds: Vec<(Range<usize>, Range<usize>)>,
    buf_instances: Vec<InstanceData>,
    buf_ops: Vec<DrawOp>,
    buf_all_verts: Vec<VertexP3U2C4>,
    buf_all_tris: Vec<TriIndicies>,
    buf_padded: Vec<u8>,
    buf_custom_draws: Vec<Arc<dyn CustomDraw>>,
}

impl Render2D {
    pub fn new(render: &rjw_render::RenderContext) -> Self {
        let device = render.device().clone();
        let queue = render.queue().clone();
        let surface_format = render.format();
        let surface: &'static wgpu::Surface<'static> = unsafe { std::mem::transmute(render.surface()) };

        let vp_bl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vp"), entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
        });
        let tex_bl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex"), entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sprite shader"), source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()) });
        let draw_page = DrawPage::new(&device, &vp_bl, &tex_bl, shader, surface_format, MAX_INSTANCES_PER_DRAW, glam::Mat4::IDENTITY);
        let white_texture = Arc::new(TextureWrapped::from_rgba8(&device, &queue, &tex_bl, "white", &[255,255,255,255], 1, 1));

        Self {
            surface, device, queue, tex_bind_group_layout: tex_bl, white_texture,
            mesh_storage: MeshStorage::default(), command_queue: DrawCommandQueue::default(), draw_page,
            depth_view: None, depth_size: (0,0), mvp: glam::Mat4::IDENTITY, default_rstates: RStates::default(),
            buf_mesh_cmds: Vec::new(), buf_instances: Vec::new(), buf_ops: Vec::new(),
            buf_all_verts: Vec::new(), buf_all_tris: Vec::new(), buf_padded: Vec::new(), buf_custom_draws: Vec::new(),
        }
    }

    pub fn set_mvp(&mut self, vp: glam::Mat4) { self.mvp = vp; self.draw_page.update_vp(&self.queue, vp); }

    pub fn reset_default_state(&mut self) -> &mut Self { self.default_rstates = RStates::default(); self }
    pub fn default_blend(&mut self, m: BlendMode) -> &mut Self { self.default_rstates = self.default_rstates.blend(m); self }
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

    pub fn tex_bind_group_layout(&self) -> &wgpu::BindGroupLayout { &self.tex_bind_group_layout }

    pub fn default_rstates(&self) -> RStates { self.default_rstates }
    pub fn set_default_rstates(&mut self, r: RStates) -> &mut Self { self.default_rstates = r; self }

    pub fn create_texture(&mut self, label: &str, data: &[u8], w: u32, h: u32) -> ArcTextureWrapped {
        assert_eq!(data.len(), (w as usize)*(h as usize)*4, "RGBA8 data length mismatch");
        let tex = Arc::new(TextureWrapped::from_rgba8(&self.device, &self.queue, &self.tex_bind_group_layout, label, data, w, h));
        TEXTURES.register(tex.clone());
        tex
    }

    pub fn register_texture(&self, tex: ArcTextureWrapped) { TEXTURES.register(tex); }

    pub fn add_sprite2d(&mut self, rect: SpriteRect, color: Color, transform: Transform2D, layer: impl Into<Layer>, texture: &ArcTextureWrapped) -> Sprite2DBuilder<'_> {
        Sprite2DBuilder { queue: &mut self.command_queue, cmd: Some(DrawCommand::Sprite2D { rect, color, transform }), layer: layer.into(), rstates: RStates::default(), texture_uid: Some(texture.uid), has_rstates: false }
    }

    pub fn add_sprite2d_solid(&mut self, rect: SpriteRect, color: Color, transform: Transform2D, layer: impl Into<Layer>) -> Sprite2DBuilder<'_> {
        let w = self.white_texture.clone();
        self.add_sprite2d(rect, color, transform, layer, &w)
    }

    pub fn add_sprite2d_matrix(&mut self, rect: SpriteRect, color: Color, model: glam::Mat4, layer: impl Into<Layer>, texture: &ArcTextureWrapped) -> Sprite2DBuilder<'_> {
        let mat_idx = self.command_queue.matrices.len();
        self.command_queue.matrices.push(model);
        Sprite2DBuilder { queue: &mut self.command_queue, cmd: Some(DrawCommand::Sprite2DMatrix { rect, color, mat_idx }), layer: layer.into(), rstates: RStates::default(), texture_uid: Some(texture.uid), has_rstates: false }
    }

    pub fn add_mesh(&mut self, vertices: &[glam::Vec2], tri_indices: &[u16], color: Color, layer: impl Into<Layer>) -> MeshBuilder<'_> {
        assert!(vertices.len() > 0 && tri_indices.len() % 3 == 0 && tri_indices.iter().all(|&i| (i as usize) < vertices.len()));
        let vs = self.mesh_storage.vertices.len();
        let ts = self.mesh_storage.tri_indices.len();
        let ca: [f32;4] = color.into();
        for p in vertices { self.mesh_storage.vertices.push(VertexP3U2C4 { pos: [p.x,p.y,0.0], uv: [0.0,0.0], color: ca }); }
        for c in tri_indices.chunks_exact(3) { self.mesh_storage.tri_indices.push(TriIndicies(Index((c[0] as u32+vs as u32) as u16), Index((c[1] as u32+vs as u32) as u16), Index((c[2] as u32+vs as u32) as u16))); }
        MeshBuilder { queue: &mut self.command_queue, cmd: Some(DrawCommand::Mesh { vert: vs..self.mesh_storage.vertices.len(), tri_index: ts..self.mesh_storage.tri_indices.len() }), layer: layer.into(), rstates: RStates::default(), texture_uid: None, has_rstates: false }
    }

    pub fn add_mesh_fn_prealloc<F>(&mut self, max_v: usize, max_t: usize, color: Color, layer: impl Into<Layer>, f: F) -> MeshBuilder<'_>
    where F: FnOnce(&mut [VertexP3U2C4], &mut [TriIndicies]) -> (usize, usize) {
        assert!(max_v > 0 && max_v <= MAX_MESH_VERTS);
        let vo = self.mesh_storage.vertices.len(); let io = self.mesh_storage.tri_indices.len(); let ca: [f32;4] = color.into();
        self.mesh_storage.vertices.resize(vo+max_v, VertexP3U2C4::default()); self.mesh_storage.tri_indices.resize(io+max_t, TriIndicies::default());
        let (uv, ut) = { let vs = &mut self.mesh_storage.vertices[vo..vo+max_v]; let ts = &mut self.mesh_storage.tri_indices[io..io+max_t]; f(vs, ts) };
        self.mesh_storage.vertices.truncate(vo+uv); self.mesh_storage.tri_indices.truncate(io+ut);
        for v in &mut self.mesh_storage.vertices[vo..vo+uv] { v.color = ca; }
        if ut != 0 { let b = vo as u32; for t in &mut self.mesh_storage.tri_indices[io..io+ut] { *t = TriIndicies(Index((t.0.0 as u32+b) as u16), Index((t.1.0 as u32+b) as u16), Index((t.2.0 as u32+b) as u16)); } }
        MeshBuilder { queue: &mut self.command_queue, cmd: Some(DrawCommand::Mesh { vert: vo..vo+uv, tri_index: io..io+ut }), layer: layer.into(), rstates: RStates::default(), texture_uid: None, has_rstates: false }
    }

    pub fn add_mesh_fn<F>(&mut self, color: Color, layer: impl Into<Layer>, f: F) -> MeshBuilder<'_>
    where F: FnOnce(&mut MeshSink<'_>) {
        let vs = self.mesh_storage.vertices.len(); let ts = self.mesh_storage.tri_indices.len(); let ca: [f32;4] = color.into();
        { let mut sink = MeshSink { base: vs as u32, verts: &mut self.mesh_storage.vertices, tris: &mut self.mesh_storage.tri_indices, color_arr: ca }; f(&mut sink); }
        MeshBuilder { queue: &mut self.command_queue, cmd: Some(DrawCommand::Mesh { vert: vs..self.mesh_storage.vertices.len(), tri_index: ts..self.mesh_storage.tri_indices.len() }), layer: layer.into(), rstates: RStates::default(), texture_uid: None, has_rstates: false }
    }

    pub fn add_polygon_fan(&mut self, vertices: &[glam::Vec2], color: Color, layer: impl Into<Layer>) -> MeshBuilder<'_> {
        debug_assert!(vertices.len() >= 3); let n = vertices.len();
        self.add_mesh_fn_prealloc(n, n-2, color, layer, |vs, ts| { for (d,s) in vs.iter_mut().zip(vertices) { d.pos = [s.x,s.y,0.0]; } for (i,t) in ts.iter_mut().enumerate() { *t = TriIndicies::new(0, (i+1) as u16, (i+2) as u16); } (n, n-2) })
    }

    pub fn add_polygon_strip(&mut self, vertices: &[glam::Vec2], color: Color, layer: impl Into<Layer>) -> MeshBuilder<'_> {
        debug_assert!(vertices.len() >= 3); let n = vertices.len();
        self.add_mesh_fn_prealloc(n, n-2, color, layer, |vs, ts| { for (d,s) in vs.iter_mut().zip(vertices) { d.pos = [s.x,s.y,0.0]; } for (i,t) in ts.iter_mut().enumerate() { *t = TriIndicies::new(0, (i+1) as u16, (i+2) as u16); } (n, n-2) })
    }

    pub fn add_custom(&mut self, layer: impl Into<Layer>, cd: impl CustomDraw + 'static) -> CustomBuilder<'_> {
        self.buf_custom_draws.push(Arc::new(cd));
        CustomBuilder { queue: &mut self.command_queue, layer: layer.into(), rstates: RStates::default(), has_rstates: false }
    }

    pub fn white_texture(&self) -> &ArcTextureWrapped { &self.white_texture }

    pub fn render(&mut self, clear: &ClearConfig) {
        let Some((st, view)) = self.begin_frame() else { self.command_queue.clear(); self.mesh_storage.clear(); return; };
        self.prepare();
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render2d encoder") });
        let nd = clear.depth.is_some() || clear.stencil.is_some();
        let size = self.surface.get_configuration().map(|c| (c.width, c.height)).unwrap_or((1,1));
        if nd { self.ensure_depth(size.0, size.1); }
        let dv = if nd { self.depth_view.as_ref() } else { None };
        {
            let co = match clear.color { Some(c) => wgpu::Operations { load: wgpu::LoadOp::Clear(c), store: wgpu::StoreOp::Store }, None => wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } };
            let dsa = dv.map(|dv| wgpu::RenderPassDepthStencilAttachment { view: dv, depth_ops: clear.depth.map(|d| wgpu::Operations { load: wgpu::LoadOp::Clear(d), store: wgpu::StoreOp::Store }), stencil_ops: clear.stencil.map(|s| wgpu::Operations { load: wgpu::LoadOp::Clear(s), store: wgpu::StoreOp::Store }) });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("render2d pass"), color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: co })], depth_stencil_attachment: dsa, occlusion_query_set: None, timestamp_writes: None, multiview_mask: None });
            self.draw(&mut pass);
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(st);
        self.command_queue.clear();
        self.mesh_storage.clear();
        self.buf_custom_draws.clear();
    }

    pub fn flush(&mut self, pass: &mut wgpu::RenderPass<'_>) { self.prepare(); self.draw(pass); self.command_queue.clear(); self.mesh_storage.clear(); self.buf_custom_draws.clear(); }

    pub fn begin_frame(&mut self) -> Option<(wgpu::SurfaceTexture, wgpu::TextureView)> {
        let t = match self.surface.get_current_texture() { wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t, _ => return None };
        let v = t.texture.create_view(&wgpu::TextureViewDescriptor::default());
        Some((t, v))
    }

    fn prepare(&mut self) {
        self.command_queue.sort_layer_then_states();
        self.buf_mesh_cmds.clear(); self.buf_instances.clear(); self.buf_ops.clear(); self.buf_all_verts.clear(); self.buf_all_tris.clear();
        let mut ct: Option<u64> = None; let mut cr: Option<u64> = None; let mut rs: usize = 0; let mut page: u32 = 0; let mut ps: usize = 0; let mut ci: usize = 0;
        macro_rules! close { () => {{ if self.buf_instances.len() > rs { self.buf_ops.push(DrawOp::Sprite { page, tex_uid: ct, range: ((rs-ps) as u32)..((self.buf_instances.len()-ps) as u32), rstates: cr.unwrap_or(0) }); } }}; }
        for (cmd, _layer, states) in self.command_queue.iter() {
            let tu = states.texture_uid;
            let rr = states.rstates.unwrap_or(self.default_rstates).raw();
            match cmd {
                DrawCommand::Sprite2D { rect, color, transform } => {
                    let pf = self.buf_instances.len() - ps >= MAX_INSTANCES_PER_DRAW;
                    if tu != ct || Some(rr) != cr || pf { close!(); ct = tu; cr = Some(rr); rs = self.buf_instances.len(); if pf { page+=1; ps = self.buf_instances.len(); } }
                    self.buf_instances.push(InstanceData::from_sprite(rect, *color, *transform));
                }
                DrawCommand::Sprite2DMatrix { rect, color, mat_idx } => {
                    let m = self.command_queue.matrices[*mat_idx];
                    let pf = self.buf_instances.len() - ps >= MAX_INSTANCES_PER_DRAW;
                    if tu != ct || Some(rr) != cr || pf { close!(); ct = tu; cr = Some(rr); rs = self.buf_instances.len(); if pf { page+=1; ps = self.buf_instances.len(); } }
                    self.buf_instances.push(InstanceData::from_sprite_matrix(rect, *color, m));
                }
                DrawCommand::Mesh { vert, tri_index } => { close!(); ct = None; cr = None; rs = self.buf_instances.len(); self.buf_ops.push(DrawOp::MeshPlaceholder); self.buf_mesh_cmds.push((vert.clone(), tri_index.clone())); }
                DrawCommand::Custom => { close!(); ct = None; cr = None; rs = self.buf_instances.len(); self.buf_ops.push(DrawOp::CustomPlaceholder); }
            }
        }
        close!();
        let pc = page as usize + 1; self.draw_page.ensure_instance_pages(&self.device, pc);
        { let mut pi = 0; let mut s = 0; while s < self.buf_instances.len() { let e = (s+MAX_INSTANCES_PER_DRAW).min(self.buf_instances.len()); self.draw_page.update_instances_page(&self.queue, pi, &self.buf_instances[s..e]); pi+=1; s = e; } }

        if !self.buf_mesh_cmds.is_empty() {
            let tv: usize = self.buf_mesh_cmds.iter().map(|(v,_)| v.end - v.start).sum();
            let tt: usize = self.buf_mesh_cmds.iter().map(|(_,t)| t.end - t.start).sum();
            assert!(tv <= MAX_MESH_VERTS);
            self.draw_page.ensure_mesh_capacity(&self.device, tv, tt+1);
            let mut vc: usize = 0; let mut ic: usize = 0;
            let mut opp = self.buf_ops.iter_mut().filter_map(|o| match o { DrawOp::MeshPlaceholder => Some(o), _ => None });
            for (v, t) in &self.buf_mesh_cmds {
                let vn = v.end - v.start; let tn = t.end - t.start;
                if vn != 0 { self.buf_all_verts.extend_from_slice(&self.mesh_storage.vertices[v.clone()]); }
                if vn != 0 && tn != 0 { let rb = (vc as i64) - (v.start as i64); for t in &self.mesh_storage.tri_indices[t.clone()] { self.buf_all_tris.push(TriIndicies(Index((t.0.0 as i64+rb) as u16), Index((t.1.0 as i64+rb) as u16), Index((t.2.0 as i64+rb) as u16))); } }
                if let Some(o) = opp.next() { *o = DrawOp::Mesh { item: crate::draw_page::MeshDrawItem { first_vertex: vc as u32, vertex_count: vn as u16, tri_index_start: ic as u32, tri_index_count: tn as u16 }, rstates: 0 }; }
                vc += vn; ic += tn;
            }
            if !self.buf_all_verts.is_empty() { self.queue.write_buffer(&self.draw_page.mesh_vb, 0, bytemuck::cast_slice(&self.buf_all_verts)); }
            if !self.buf_all_tris.is_empty() { let bs = bytemuck::cast_slice(&self.buf_all_tris); let pl = (bs.len()+3)&!3; self.buf_padded.clear(); self.buf_padded.extend_from_slice(bs); self.buf_padded.resize(pl,0u8); self.queue.write_buffer(&self.draw_page.mesh_ib, 0, &self.buf_padded); }
        }

        let mut oc = self.buf_ops.iter_mut().filter_map(|o| match o { DrawOp::CustomPlaceholder => Some(o), _ => None });
        while let Some(o) = oc.next() { *o = DrawOp::Custom { idx: ci }; ci += 1; }
    }

    fn draw(&mut self, pass: &mut wgpu::RenderPass<'_>) {
        if self.buf_ops.is_empty() { return; }
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
                        let tex_arc;
                        let bg = match tex_uid {
                            Some(uid) if *uid == self.white_texture.uid => &self.white_texture.bind_group,
                            Some(uid) => { tex_arc = TEXTURES.get(*uid).expect("tex not found"); &tex_arc.bind_group }
                            None => &self.white_texture.bind_group,
                        };
                        pass.set_bind_group(1, bg, &[]);
                        pass.draw_indexed(0..QUAD_TRI_INDICIES.len() as u32, 0, range.clone());
                    }
                    i += 1;
                }
                DrawOp::Mesh { item: _, rstates } => {
                    let mut ss: Option<u32> = None; let mut sc: u32 = 0; let mut sr = *rstates;
                    while i < self.buf_ops.len() { match &self.buf_ops[i] { DrawOp::Mesh { item, rstates: r } => { if item.tri_index_count != 0 { if ss.is_none() { ss = Some(item.tri_index_start * 3); sr = *r; } sc += item.tri_index_count as u32 * 3; } i += 1; } _ => break, } }
                    if let Some(start) = ss {
                        let pipeline = self.draw_page.get_or_create_pipeline(&self.device, sr);
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, &self.draw_page.vp_bind_group, &[]);
                        pass.set_bind_group(1, &self.white_texture.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.draw_page.mesh_vb.slice(..));
                        pass.set_vertex_buffer(1, self.draw_page.identity_instance_buffer().slice(..));
                        pass.set_index_buffer(self.draw_page.mesh_ib.slice(..), wgpu::IndexFormat::Uint16);
                        pass.draw_indexed(start..start + sc, 0, 0..1);
                    }
                }
                DrawOp::Custom { idx } => {
                    let cd = Arc::clone(&self.buf_custom_draws[*idx]);
                    cd.draw(pass);
                    i += 1;
                }
                DrawOp::MeshPlaceholder => { debug_assert!(false); i += 1; }
                DrawOp::CustomPlaceholder => { debug_assert!(false); i += 1; }
            }
        }
    }

    fn ensure_depth(&mut self, w: u32, h: u32) {
        if self.depth_view.as_ref().is_some_and(|_| self.depth_size == (w.max(1), h.max(1))) { return; }
        let t = self.device.create_texture(&wgpu::TextureDescriptor { label: Some("depth-stencil"), size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: DEPTH_FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] });
        self.depth_view = Some(t.create_view(&wgpu::TextureViewDescriptor::default()));
        self.depth_size = (w.max(1), h.max(1));
    }

    pub fn device(&self) -> &wgpu::Device { &self.device }
    pub fn queue(&self) -> &wgpu::Queue { &self.queue }
}