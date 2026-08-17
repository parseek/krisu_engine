//! `Render2D`：Batch2D 渲染器主体 + Clear 配置。

use std::{collections::HashMap, ops::Range, sync::Arc};

use rjw_color::Color;
use rjw_render::{ArcTextureWrapped, MeshData, MESHES, TEXTURES, TextureWrapped};
use rjw_transform::{Rect, Transform2D};

use crate::command::{DrawCommand, DrawCommandQueue, Layer, States};
use crate::data::{
    Index, MeshSink, MeshStorage, QUAD_TRI_INDICIES, SpriteRect, TriIndicies, VertexP3U2C4,
};
use crate::draw_page::{
    DEPTH_FORMAT, DrawOp, DrawPage, InstanceData, MAX_INSTANCES_PER_DRAW, MAX_MESH_VERTS,
};
use crate::rstates::{
    AddressMode, BlendDesc, BlendMode, CompareFunc, CullMode, DepthState, FilterMode,
    FrontFaceWinding, PolygonMode, RStates, RasterState, SamplerDesc, StencilState,
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
        Self {
            color: Some(wgpu::Color::BLACK),
            depth: None,
            stencil: None,
        }
    }
}

// ─── Builder：责任链模式 ────────────────────────────────────

pub struct Sprite2DBuilder<'a> {
    queue: &'a mut DrawCommandQueue,
    cmd: Option<DrawCommand>,
    layer: Layer,
    rstates: RStates,
    texture_uid: Option<u64>,
    has_rstates: bool,
}

impl<'a> Sprite2DBuilder<'a> {
    pub fn blend(mut self, mode: BlendMode) -> Self {
        self.rstates = self.rstates.blend(mode);
        self.has_rstates = true;
        self
    }
    pub fn samp_mag(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_mag(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_min(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_min(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_mip(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_mip(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_u(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_u(a);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_v(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_v(a);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_w(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_w(a);
        self.has_rstates = true;
        self
    }
    pub fn cull(mut self, c: CullMode) -> Self {
        self.rstates = self.rstates.cull(c);
        self.has_rstates = true;
        self
    }
    pub fn polygon(mut self, p: PolygonMode) -> Self {
        self.rstates = self.rstates.polygon(p);
        self.has_rstates = true;
        self
    }
    pub fn front_face(mut self, f: FrontFaceWinding) -> Self {
        self.rstates = self.rstates.front_face(f);
        self.has_rstates = true;
        self
    }
    pub fn conservative_raster(mut self, b: bool) -> Self {
        self.rstates = self.rstates.conservative_raster(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_test(mut self, b: bool) -> Self {
        self.rstates = self.rstates.depth_test(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_write(mut self, b: bool) -> Self {
        self.rstates = self.rstates.depth_write(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_compare(mut self, f: CompareFunc) -> Self {
        self.rstates = self.rstates.depth_compare(f);
        self.has_rstates = true;
        self
    }
    pub fn stencil_test(mut self, b: bool) -> Self {
        self.rstates = self.rstates.stencil_test(b);
        self.has_rstates = true;
        self
    }
    pub fn stencil_write(mut self, b: bool) -> Self {
        self.rstates = self.rstates.stencil_write(b);
        self.has_rstates = true;
        self
    }
    pub fn stencil_compare(mut self, f: CompareFunc) -> Self {
        self.rstates = self.rstates.stencil_compare(f);
        self.has_rstates = true;
        self
    }
    pub fn blend_state(mut self, d: BlendDesc) -> Self {
        self.rstates = self.rstates.blend_state(d);
        self.has_rstates = true;
        self
    }
    pub fn samp_state(mut self, d: SamplerDesc) -> Self {
        self.rstates = self.rstates.samp_state(d);
        self.has_rstates = true;
        self
    }
    pub fn raster_state(mut self, s: RasterState) -> Self {
        self.rstates = self.rstates.raster_state(s);
        self.has_rstates = true;
        self
    }
    pub fn depth_state(mut self, s: DepthState) -> Self {
        self.rstates = self.rstates.depth_state(s);
        self.has_rstates = true;
        self
    }
    pub fn stencil_state(mut self, s: StencilState) -> Self {
        self.rstates = self.rstates.stencil_state(s);
        self.has_rstates = true;
        self
    }
}

impl Drop for Sprite2DBuilder<'_> {
    fn drop(&mut self) {
        let rstates = if self.has_rstates {
            Some(self.rstates)
        } else {
            None
        };
        if let Some(cmd) = self.cmd.take() {
            self.queue.push(
                cmd,
                self.layer,
                States {
                    rstates,
                    texture_uid: self.texture_uid,
                },
            );
        }
    }
}

/// 静态网格 Builder（由 `add_static_mesh` / `add_static_mesh_matrix` 返回）。
/// 链式设置 RStates；`done()` 或 Drop 时 push `DrawCommand::StaticMesh*`。
pub struct StaticMeshBuilder<'a> {
    queue: &'a mut DrawCommandQueue,
    cmd: Option<DrawCommand>,
    layer: Layer,
    rstates: RStates,
    texture_uid: Option<u64>,
    has_rstates: bool,
}

impl<'a> StaticMeshBuilder<'a> {
    /// 消费 builder，立即提交命令（等价于直接 drop）。
    pub fn done(mut self) {
        // 无操作：Drop 实现自动 push 命令。
        let _ = &mut self;
    }

    pub fn blend(mut self, mode: BlendMode) -> Self {
        self.rstates = self.rstates.blend(mode);
        self.has_rstates = true;
        self
    }
    pub fn samp_mag(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_mag(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_min(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_min(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_mip(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_mip(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_u(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_u(a);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_v(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_v(a);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_w(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_w(a);
        self.has_rstates = true;
        self
    }
    pub fn cull(mut self, c: CullMode) -> Self {
        self.rstates = self.rstates.cull(c);
        self.has_rstates = true;
        self
    }
    pub fn polygon(mut self, p: PolygonMode) -> Self {
        self.rstates = self.rstates.polygon(p);
        self.has_rstates = true;
        self
    }
    pub fn front_face(mut self, f: FrontFaceWinding) -> Self {
        self.rstates = self.rstates.front_face(f);
        self.has_rstates = true;
        self
    }
    pub fn conservative_raster(mut self, b: bool) -> Self {
        self.rstates = self.rstates.conservative_raster(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_test(mut self, b: bool) -> Self {
        self.rstates = self.rstates.depth_test(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_write(mut self, b: bool) -> Self {
        self.rstates = self.rstates.depth_write(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_compare(mut self, f: CompareFunc) -> Self {
        self.rstates = self.rstates.depth_compare(f);
        self.has_rstates = true;
        self
    }
    pub fn stencil_test(mut self, b: bool) -> Self {
        self.rstates = self.rstates.stencil_test(b);
        self.has_rstates = true;
        self
    }
    pub fn stencil_write(mut self, b: bool) -> Self {
        self.rstates = self.rstates.stencil_write(b);
        self.has_rstates = true;
        self
    }
    pub fn stencil_compare(mut self, f: CompareFunc) -> Self {
        self.rstates = self.rstates.stencil_compare(f);
        self.has_rstates = true;
        self
    }
    pub fn blend_state(mut self, d: BlendDesc) -> Self {
        self.rstates = self.rstates.blend_state(d);
        self.has_rstates = true;
        self
    }
    pub fn samp_state(mut self, d: SamplerDesc) -> Self {
        self.rstates = self.rstates.samp_state(d);
        self.has_rstates = true;
        self
    }
    pub fn raster_state(mut self, s: RasterState) -> Self {
        self.rstates = self.rstates.raster_state(s);
        self.has_rstates = true;
        self
    }
    pub fn depth_state(mut self, s: DepthState) -> Self {
        self.rstates = self.rstates.depth_state(s);
        self.has_rstates = true;
        self
    }
    pub fn stencil_state(mut self, s: StencilState) -> Self {
        self.rstates = self.rstates.stencil_state(s);
        self.has_rstates = true;
        self
    }
}

impl Drop for StaticMeshBuilder<'_> {
    fn drop(&mut self) {
        let rstates = if self.has_rstates {
            Some(self.rstates)
        } else {
            None
        };
        if let Some(cmd) = self.cmd.take() {
            self.queue.push(
                cmd,
                self.layer,
                States {
                    rstates,
                    texture_uid: self.texture_uid,
                },
            );
        }
    }
}

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
    pub fn blend(mut self, mode: BlendMode) -> Self {
        self.rstates = self.rstates.blend(mode);
        self.has_rstates = true;
        self
    }
    pub fn samp_mag(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_mag(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_min(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_min(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_mip(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_mip(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_u(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_u(a);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_v(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_v(a);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_w(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_w(a);
        self.has_rstates = true;
        self
    }
    pub fn cull(mut self, c: CullMode) -> Self {
        self.rstates = self.rstates.cull(c);
        self.has_rstates = true;
        self
    }
    pub fn polygon(mut self, p: PolygonMode) -> Self {
        self.rstates = self.rstates.polygon(p);
        self.has_rstates = true;
        self
    }
    pub fn front_face(mut self, f: FrontFaceWinding) -> Self {
        self.rstates = self.rstates.front_face(f);
        self.has_rstates = true;
        self
    }
    pub fn conservative_raster(mut self, b: bool) -> Self {
        self.rstates = self.rstates.conservative_raster(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_test(mut self, b: bool) -> Self {
        self.rstates = self.rstates.depth_test(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_write(mut self, b: bool) -> Self {
        self.rstates = self.rstates.depth_write(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_compare(mut self, f: CompareFunc) -> Self {
        self.rstates = self.rstates.depth_compare(f);
        self.has_rstates = true;
        self
    }
    pub fn stencil_test(mut self, b: bool) -> Self {
        self.rstates = self.rstates.stencil_test(b);
        self.has_rstates = true;
        self
    }
    pub fn stencil_write(mut self, b: bool) -> Self {
        self.rstates = self.rstates.stencil_write(b);
        self.has_rstates = true;
        self
    }
    pub fn stencil_compare(mut self, f: CompareFunc) -> Self {
        self.rstates = self.rstates.stencil_compare(f);
        self.has_rstates = true;
        self
    }
    pub fn blend_state(mut self, d: BlendDesc) -> Self {
        self.rstates = self.rstates.blend_state(d);
        self.has_rstates = true;
        self
    }
    pub fn samp_state(mut self, d: SamplerDesc) -> Self {
        self.rstates = self.rstates.samp_state(d);
        self.has_rstates = true;
        self
    }
    pub fn raster_state(mut self, s: RasterState) -> Self {
        self.rstates = self.rstates.raster_state(s);
        self.has_rstates = true;
        self
    }
    pub fn depth_state(mut self, s: DepthState) -> Self {
        self.rstates = self.rstates.depth_state(s);
        self.has_rstates = true;
        self
    }
    pub fn stencil_state(mut self, s: StencilState) -> Self {
        self.rstates = self.rstates.stencil_state(s);
        self.has_rstates = true;
        self
    }
}

impl Drop for MeshBuilder<'_> {
    fn drop(&mut self) {
        let rstates = if self.has_rstates {
            Some(self.rstates)
        } else {
            None
        };
        if let Some(cmd) = self.cmd.take() {
            self.queue.push(
                cmd,
                self.layer,
                States {
                    rstates,
                    texture_uid: self.texture_uid,
                },
            );
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
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self(pass);
    }
}

/// 外部绘制 Builder（由 `add_custom` 返回）。
/// 链式设置 RStates；Drop 时 push `DrawCommand::Custom`。
pub struct CustomBuilder<'a> {
    queue: &'a mut DrawCommandQueue,
    cmd: Option<DrawCommand>,
    layer: Layer,
    rstates: RStates,
    has_rstates: bool,
}

impl<'a> CustomBuilder<'a> {
    pub fn blend(mut self, mode: BlendMode) -> Self {
        self.rstates = self.rstates.blend(mode);
        self.has_rstates = true;
        self
    }
    pub fn samp_mag(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_mag(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_min(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_min(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_mip(mut self, f: FilterMode) -> Self {
        self.rstates = self.rstates.samp_mip(f);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_u(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_u(a);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_v(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_v(a);
        self.has_rstates = true;
        self
    }
    pub fn samp_addr_w(mut self, a: AddressMode) -> Self {
        self.rstates = self.rstates.samp_addr_w(a);
        self.has_rstates = true;
        self
    }
    pub fn cull(mut self, c: CullMode) -> Self {
        self.rstates = self.rstates.cull(c);
        self.has_rstates = true;
        self
    }
    pub fn polygon(mut self, p: PolygonMode) -> Self {
        self.rstates = self.rstates.polygon(p);
        self.has_rstates = true;
        self
    }
    pub fn front_face(mut self, f: FrontFaceWinding) -> Self {
        self.rstates = self.rstates.front_face(f);
        self.has_rstates = true;
        self
    }
    pub fn conservative_raster(mut self, b: bool) -> Self {
        self.rstates = self.rstates.conservative_raster(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_test(mut self, b: bool) -> Self {
        self.rstates = self.rstates.depth_test(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_write(mut self, b: bool) -> Self {
        self.rstates = self.rstates.depth_write(b);
        self.has_rstates = true;
        self
    }
    pub fn depth_compare(mut self, f: CompareFunc) -> Self {
        self.rstates = self.rstates.depth_compare(f);
        self.has_rstates = true;
        self
    }
    pub fn stencil_test(mut self, b: bool) -> Self {
        self.rstates = self.rstates.stencil_test(b);
        self.has_rstates = true;
        self
    }
    pub fn stencil_write(mut self, b: bool) -> Self {
        self.rstates = self.rstates.stencil_write(b);
        self.has_rstates = true;
        self
    }
    pub fn stencil_compare(mut self, f: CompareFunc) -> Self {
        self.rstates = self.rstates.stencil_compare(f);
        self.has_rstates = true;
        self
    }
    pub fn blend_state(mut self, d: BlendDesc) -> Self {
        self.rstates = self.rstates.blend_state(d);
        self.has_rstates = true;
        self
    }
    pub fn samp_state(mut self, d: SamplerDesc) -> Self {
        self.rstates = self.rstates.samp_state(d);
        self.has_rstates = true;
        self
    }
    pub fn raster_state(mut self, s: RasterState) -> Self {
        self.rstates = self.rstates.raster_state(s);
        self.has_rstates = true;
        self
    }
    pub fn depth_state(mut self, s: DepthState) -> Self {
        self.rstates = self.rstates.depth_state(s);
        self.has_rstates = true;
        self
    }
    pub fn stencil_state(mut self, s: StencilState) -> Self {
        self.rstates = self.rstates.stencil_state(s);
        self.has_rstates = true;
        self
    }
}

impl Drop for CustomBuilder<'_> {
    fn drop(&mut self) {
        let rstates = if self.has_rstates {
            Some(self.rstates)
        } else {
            None
        };
        if let Some(cmd) = self.cmd.take() {
            self.queue.push(
                cmd,
                self.layer,
                States {
                    rstates,
                    texture_uid: None,
                },
            );
        }
    }
}

// ─── 合批中间项 ───────────────────────────────────────────────

/// `prepare` 阶段的合批中间项。
///
/// - `mesh_id`: `Some(uid)` 为注册表网格（Sprite / StaticMesh）；`None` 为动态缓冲段
///   （`add_mesh*` 系列，此时 `index_range` 为该段在动态索引缓冲中的范围）。
/// - `dyn_seq`: 动态段每帧递增的唯一序号（`0` 表示静态项）。
///   每个动态段恰好一个 identity 实例；seq 唯一保证**不同动态段绝不互相合批**，
///   否则多个 identity 实例会重复绘制整段动态缓冲。
/// - `layer`: 绘制层级（越小越先绘制）。排序键以 layer 为主，**保证跨层级合批
///   不会打乱图层顺序**。
/// - `index_range`: 索引范围（静态网格 = `0..index_count`；动态段 = `tri*3` 范围）。
struct BatchItem {
    mesh_id: Option<u64>,
    dyn_seq: u32,
    layer: Layer,
    index_range: Range<u32>,
    rstates: u64,
    tex_uid: Option<u64>,
    instance: InstanceData,
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
    /// 视口剔除开关（默认 **false**；开启后 Sprite 系列在提交前按世界 AABB 与视口矩形做剔除）。
    culling: bool,
    default_rstates: RStates,

    /// 四边形网格（Sprite 合批用）的全局注册表 uid。
    quad_mesh_id: u64,

    /// 采样器位域缓存：key = RStates 采样器位域（bits 8..24）。
    sampler_cache: HashMap<u64, wgpu::Sampler>,
    /// 默认采样器（RStates::default()：线性过滤 + ClampToEdge），samp_key == 0 的零开销路径。
    default_sampler: wgpu::Sampler,
    /// bind group 缓存：key = (tex_uid, samp_key)；value 持有 Arc<Texture> 防绑定组悬挂，
    /// prepare 末尾按 TEXTURES 存活情况清理失效条目。
    tex_bind_group_cache: HashMap<(u64, u64), (ArcTextureWrapped, wgpu::BindGroup)>,

    buf_items: Vec<BatchItem>,
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
        let surface: &'static wgpu::Surface<'static> =
            unsafe { std::mem::transmute(render.surface()) };

        let vp_bl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Render2D: VP bind group layout"),
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
        let tex_bl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Render2D: Texture bind group layout"),
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
            label: Some("Render2D: Default Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
        });
        let draw_page = DrawPage::new(
            &device,
            &vp_bl,
            &tex_bl,
            shader,
            surface_format,
            MAX_INSTANCES_PER_DRAW,
            glam::Mat4::IDENTITY,
        );
        // 注册四边形为静态网格（Sprite 与 StaticMesh 共用实例化绘制路径）。
        let quad_mesh_id = MESHES.register(Arc::new(MeshData::from_buffers(
            draw_page.quad_vb.clone(),
            draw_page.quad_ib.clone(),
            QUAD_TRI_INDICIES.len() as u32,
        )));
        let white_texture = Arc::new(TextureWrapped::from_rgba8(
            &device,
            &queue,
            "Render2D: White Texture",
            &[255, 255, 255, 255],
            1,
            1,
        ));
        TEXTURES.register(white_texture.clone());

        // 默认采样器（RStates::default()：线性 + ClampToEdge），samp_key == 0 零开销路径。
        let default_sampler = device.create_sampler(&RStates::default().to_sampler_desc());

        Self {
            surface,
            device,
            queue,
            tex_bind_group_layout: tex_bl,
            white_texture,
            mesh_storage: MeshStorage::default(),
            command_queue: DrawCommandQueue::default(),
            draw_page,
            depth_view: None,
            depth_size: (0, 0),
            mvp: glam::Mat4::IDENTITY,
            culling: false,
            default_rstates: RStates::default(),
            quad_mesh_id,
            sampler_cache: HashMap::new(),
            default_sampler,
            tex_bind_group_cache: HashMap::new(),
            buf_items: Vec::new(),
            buf_instances: Vec::new(),
            buf_ops: Vec::new(),
            buf_all_verts: Vec::new(),
            buf_all_tris: Vec::new(),
            buf_padded: Vec::new(),
            buf_custom_draws: Vec::new(),
        }
    }

    pub fn set_mvp(&mut self, vp: glam::Mat4) -> &mut Self {
        self.mvp = vp;
        self.draw_page.update_vp(&self.queue, vp);
        self
    }

    /// 视口剔除开关（默认 **false**）。
    ///
    /// 开启后，`add_sprite2d` / `add_sprite2d_matrix`（及经 `add_mesh` 提交的动态 mesh **除外**）
    /// 在写入实例缓冲前，按精灵世界 AABB 与视口世界矩形（由 [`Self::set_mvp`] 反推）相交测试，
    /// 剔除完全不可见的精灵——减少实例数与上传量，绘制结果不变（视口外的本来也看不见）。
    #[inline]
    pub fn set_culling(&mut self, culling: bool) -> &mut Self {
        self.culling = culling;
        self
    }

    /// 视口世界矩形：由当前 MVP 逆变换 clip 空间四角得到（正交相机下 z 取 0 即可）。
    fn viewport_world_rect(&self) -> Rect {
        let inv = self.mvp.inverse();
        let corners = [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)];
        let mut pts = [glam::Vec2::ZERO; 4];
        for (i, (cx, cy)) in corners.iter().enumerate() {
            let v = inv * glam::Vec4::new(*cx, *cy, 0.0, 1.0);
            pts[i] = glam::Vec2::new(v.x / v.w, v.y / v.w);
        }
        Rect::from_point_slice(&pts)
    }

    /// 精灵四角经 `model` 变换后的世界 AABB 是否与视口矩形相交（保守：旋转取包围盒）。
    fn sprite_in_viewport(rect: &SpriteRect, model: glam::Mat4, vp: &Rect) -> bool {
        let tl = rect.mesh_tl;
        let wh = rect.mesh_wh;
        let corners = [
            tl,
            glam::Vec2::new(tl.x + wh.x, tl.y),
            glam::Vec2::new(tl.x, tl.y + wh.y),
            tl + wh,
        ];
        let mut pts = [glam::Vec2::ZERO; 4];
        for (i, c) in corners.iter().enumerate() {
            let v = model * glam::Vec4::new(c.x, c.y, 0.0, 1.0);
            pts[i] = glam::Vec2::new(v.x / v.w, v.y / v.w);
        }
        Rect::from_point_slice(&pts).intersects(vp)
    }

    /// `Transform2D` → 列主序 2D model 矩阵（与 [`InstanceData::from_sprite`] 一致）。
    fn transform2d_model(t: &Transform2D) -> glam::Mat4 {
        let (sin, cos) = t.rotation.sin_cos();
        glam::Mat4::from_cols_array_2d(&[
            [cos * t.scale.x, sin * t.scale.x, 0.0, 0.0],
            [-sin * t.scale.y, cos * t.scale.y, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [t.pos.x, t.pos.y, 0.0, 1.0],
        ])
    }

    pub fn reset_default_state(&mut self) -> &mut Self {
        self.default_rstates = RStates::default();
        self
    }
    pub fn default_blend(&mut self, m: BlendMode) -> &mut Self {
        self.default_rstates = self.default_rstates.blend(m);
        self
    }
    pub fn default_samp_mag(&mut self, f: FilterMode) -> &mut Self {
        self.default_rstates = self.default_rstates.samp_mag(f);
        self
    }
    pub fn default_samp_min(&mut self, f: FilterMode) -> &mut Self {
        self.default_rstates = self.default_rstates.samp_min(f);
        self
    }
    pub fn default_samp_mip(&mut self, f: FilterMode) -> &mut Self {
        self.default_rstates = self.default_rstates.samp_mip(f);
        self
    }
    pub fn default_samp_addr_u(&mut self, a: AddressMode) -> &mut Self {
        self.default_rstates = self.default_rstates.samp_addr_u(a);
        self
    }
    pub fn default_samp_addr_v(&mut self, a: AddressMode) -> &mut Self {
        self.default_rstates = self.default_rstates.samp_addr_v(a);
        self
    }
    pub fn default_samp_addr_w(&mut self, a: AddressMode) -> &mut Self {
        self.default_rstates = self.default_rstates.samp_addr_w(a);
        self
    }
    pub fn default_samp_filter(&mut self, min: FilterMode, mag: FilterMode, mip: FilterMode) -> &mut Self {
        self.default_rstates = self
            .default_rstates
            .samp_min(min)
            .samp_mag(mag)
            .samp_mip(mip);
        self
    }
    pub fn default_samp_min_mag(&mut self, f: FilterMode) -> &mut Self {
        self.default_rstates = self.default_rstates.samp_min(f).samp_mag(f);
        self
    }
    pub fn default_samp_addr(&mut self, u: AddressMode, v: AddressMode, w: AddressMode) -> &mut Self {
        self.default_rstates = self
            .default_rstates
            .samp_addr_u(u)
            .samp_addr_v(v)
            .samp_addr_w(w);
        self
    }
    pub fn default_samp_addr_all(&mut self, a: AddressMode) -> &mut Self {
        self.default_rstates = self.default_rstates.samp_addr_u(a).samp_addr_v(a).samp_addr_w(a);
        self
    }

    pub fn default_cull(&mut self, c: CullMode) -> &mut Self {
        self.default_rstates = self.default_rstates.cull(c);
        self
    }
    pub fn default_polygon(&mut self, p: PolygonMode) -> &mut Self {
        self.default_rstates = self.default_rstates.polygon(p);
        self
    }
    pub fn default_front_face(&mut self, f: FrontFaceWinding) -> &mut Self {
        self.default_rstates = self.default_rstates.front_face(f);
        self
    }
    pub fn default_conservative_raster(&mut self, b: bool) -> &mut Self {
        self.default_rstates = self.default_rstates.conservative_raster(b);
        self
    }
    pub fn default_depth_test(&mut self, b: bool) -> &mut Self {
        self.default_rstates = self.default_rstates.depth_test(b);
        self
    }
    pub fn default_depth_write(&mut self, b: bool) -> &mut Self {
        self.default_rstates = self.default_rstates.depth_write(b);
        self
    }
    pub fn default_depth_compare(&mut self, f: CompareFunc) -> &mut Self {
        self.default_rstates = self.default_rstates.depth_compare(f);
        self
    }
    pub fn default_stencil_test(&mut self, b: bool) -> &mut Self {
        self.default_rstates = self.default_rstates.stencil_test(b);
        self
    }
    pub fn default_stencil_write(&mut self, b: bool) -> &mut Self {
        self.default_rstates = self.default_rstates.stencil_write(b);
        self
    }
    pub fn default_stencil_compare(&mut self, f: CompareFunc) -> &mut Self {
        self.default_rstates = self.default_rstates.stencil_compare(f);
        self
    }
    pub fn default_blend_state(&mut self, d: BlendDesc) -> &mut Self {
        self.default_rstates = self.default_rstates.blend_state(d);
        self
    }
    pub fn default_samp_state(&mut self, d: SamplerDesc) -> &mut Self {
        self.default_rstates = self.default_rstates.samp_state(d);
        self
    }
    pub fn default_raster_state(&mut self, s: RasterState) -> &mut Self {
        self.default_rstates = self.default_rstates.raster_state(s);
        self
    }
    pub fn default_depth_state(&mut self, s: DepthState) -> &mut Self {
        self.default_rstates = self.default_rstates.depth_state(s);
        self
    }
    pub fn default_stencil_state(&mut self, s: StencilState) -> &mut Self {
        self.default_rstates = self.default_rstates.stencil_state(s);
        self
    }

    pub fn tex_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.tex_bind_group_layout
    }

    pub fn default_rstates(&self) -> RStates {
        self.default_rstates
    }
    pub fn set_default_rstates(&mut self, r: RStates) -> &mut Self {
        self.default_rstates = r;
        self
    }

    pub fn create_texture(
        &mut self,
        label: &str,
        data: &[u8],
        w: u32,
        h: u32,
    ) -> ArcTextureWrapped {
        assert_eq!(
            data.len(),
            (w as usize) * (h as usize) * 4,
            "RGBA8 data length mismatch"
        );
        let tex = Arc::new(TextureWrapped::from_rgba8(
            &self.device,
            &self.queue,
            label,
            data,
            w,
            h,
        ));
        TEXTURES.register(tex.clone());
        tex
    }

    pub fn register_texture(&self, tex: ArcTextureWrapped) {
        TEXTURES.register(tex);
    }

    // ── 静态网格 API ────────────────────────────────────────

    /// 注册静态网格到全局 `MESHES` 注册表，返回可复用的 `mesh_id`。
    ///
    /// 相同内容的网格应**复用同一个** `Arc<MeshData>` 注册，否则无法合批。
    pub fn register_mesh(&self, mesh: Arc<MeshData>) -> u64 {
        MESHES.register(mesh)
    }

    /// 注册一个静态网格实例（带 `Transform2D` 变换，顶点自带 UV 采样纹理）。
    pub fn add_static_mesh(
        &mut self,
        mesh_id: u64,
        color: Color,
        transform: Transform2D,
        layer: impl Into<Layer>,
        texture: &ArcTextureWrapped,
    ) -> StaticMeshBuilder<'_> {
        debug_assert!(
            MESHES.contains_uid(mesh_id),
            "mesh {mesh_id} is not registered in MESHES"
        );
        StaticMeshBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::StaticMesh {
                mesh_id,
                color,
                transform,
            }),
            layer: layer.into(),
            rstates: RStates::default(),
            texture_uid: Some(texture.uid),
            has_rstates: false,
        }
    }

    /// 注册一个静态网格实例（直接列主序模型矩阵，顶点自带 UV 采样纹理）。
    pub fn add_static_mesh_matrix(
        &mut self,
        mesh_id: u64,
        color: Color,
        model: glam::Mat4,
        layer: impl Into<Layer>,
        texture: &ArcTextureWrapped,
    ) -> StaticMeshBuilder<'_> {
        debug_assert!(
            MESHES.contains_uid(mesh_id),
            "mesh {mesh_id} is not registered in MESHES"
        );
        let mat_idx = self.command_queue.matrices.len();
        self.command_queue.matrices.push(model);
        StaticMeshBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::StaticMeshMatrix {
                mesh_id,
                color,
                mat_idx,
            }),
            layer: layer.into(),
            rstates: RStates::default(),
            texture_uid: Some(texture.uid),
            has_rstates: false,
        }
    }

    // ── Sprite / Mesh / Custom API ───────────────────────────

    pub fn add_sprite2d(
        &mut self,
        rect: impl Into<SpriteRect>,
        color: Color,
        transform: Transform2D,
        layer: impl Into<Layer>,
        texture: &ArcTextureWrapped,
    ) -> Sprite2DBuilder<'_> {
        let rect = rect.into();
        Sprite2DBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::Sprite2D {
                rect,
                color,
                transform,
            }),
            layer: layer.into(),
            rstates: RStates::default(),
            texture_uid: Some(texture.uid),
            has_rstates: false,
        }
    }

    pub fn add_sprite2d_solid(
        &mut self,
        rect: impl Into<SpriteRect>,
        color: Color,
        transform: Transform2D,
        layer: impl Into<Layer>,
    ) -> Sprite2DBuilder<'_> {
        let w = self.white_texture.clone();
        self.add_sprite2d(rect, color, transform, layer, &w)
    }

    pub fn add_sprite2d_matrix(
        &mut self,
        rect: impl Into<SpriteRect>,
        color: Color,
        model: glam::Mat4,
        layer: impl Into<Layer>,
        texture: &ArcTextureWrapped,
    ) -> Sprite2DBuilder<'_> {
        let rect = rect.into();
        let mat_idx = self.command_queue.matrices.len();
        self.command_queue.matrices.push(model);
        Sprite2DBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::Sprite2DMatrix {
                rect,
                color,
                mat_idx,
            }),
            layer: layer.into(),
            rstates: RStates::default(),
            texture_uid: Some(texture.uid),
            has_rstates: false,
        }
    }

    pub fn add_mesh(
        &mut self,
        vertices: &[glam::Vec2],
        tri_indices: &[u16],
        color: Color,
        layer: impl Into<Layer>,
    ) -> MeshBuilder<'_> {
        assert!(
            vertices.len() > 0
                && tri_indices.len() % 3 == 0
                && tri_indices.iter().all(|&i| (i as usize) < vertices.len())
        );
        let vs = self.mesh_storage.vertices.len();
        let ts = self.mesh_storage.tri_indices.len();
        let ca: [f32; 4] = color.into();
        for p in vertices {
            self.mesh_storage.vertices.push(VertexP3U2C4 {
                pos: [p.x, p.y, 0.0],
                uv: [0.0, 0.0],
                color: ca,
            });
        }
        for c in tri_indices.chunks_exact(3) {
            self.mesh_storage.tri_indices.push(TriIndicies(
                Index((c[0] as u32 + vs as u32) as u16),
                Index((c[1] as u32 + vs as u32) as u16),
                Index((c[2] as u32 + vs as u32) as u16),
            ));
        }
        MeshBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::Mesh {
                vert: vs..self.mesh_storage.vertices.len(),
                tri_index: ts..self.mesh_storage.tri_indices.len(),
            }),
            layer: layer.into(),
            rstates: RStates::default(),
            texture_uid: None,
            has_rstates: false,
        }
    }

    pub fn add_mesh_fn_prealloc<F>(
        &mut self,
        max_v: usize,
        max_t: usize,
        color: Color,
        layer: impl Into<Layer>,
        f: F,
    ) -> MeshBuilder<'_>
    where
        F: FnOnce(&mut [VertexP3U2C4], &mut [TriIndicies]) -> (usize, usize),
    {
        assert!(max_v > 0 && max_v <= MAX_MESH_VERTS);
        let vo = self.mesh_storage.vertices.len();
        let io = self.mesh_storage.tri_indices.len();
        let ca: [f32; 4] = color.into();
        self.mesh_storage
            .vertices
            .resize(vo + max_v, VertexP3U2C4::default());
        self.mesh_storage
            .tri_indices
            .resize(io + max_t, TriIndicies::default());
        let (uv, ut) = {
            let vs = &mut self.mesh_storage.vertices[vo..vo + max_v];
            let ts = &mut self.mesh_storage.tri_indices[io..io + max_t];
            f(vs, ts)
        };
        self.mesh_storage.vertices.truncate(vo + uv);
        self.mesh_storage.tri_indices.truncate(io + ut);
        for v in &mut self.mesh_storage.vertices[vo..vo + uv] {
            v.color = ca;
        }
        if ut != 0 {
            let b = vo as u32;
            for t in &mut self.mesh_storage.tri_indices[io..io + ut] {
                *t = TriIndicies(
                    Index((t.0.0 as u32 + b) as u16),
                    Index((t.1.0 as u32 + b) as u16),
                    Index((t.2.0 as u32 + b) as u16),
                );
            }
        }
        MeshBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::Mesh {
                vert: vo..vo + uv,
                tri_index: io..io + ut,
            }),
            layer: layer.into(),
            rstates: RStates::default(),
            texture_uid: None,
            has_rstates: false,
        }
    }

    pub fn add_mesh_fn<F>(&mut self, color: Color, layer: impl Into<Layer>, f: F) -> MeshBuilder<'_>
    where
        F: FnOnce(&mut MeshSink<'_>),
    {
        let vs = self.mesh_storage.vertices.len();
        let ts = self.mesh_storage.tri_indices.len();
        let ca: [f32; 4] = color.into();
        {
            let mut sink = MeshSink {
                base: vs as u32,
                verts: &mut self.mesh_storage.vertices,
                tris: &mut self.mesh_storage.tri_indices,
                color_arr: ca,
            };
            f(&mut sink);
        }
        MeshBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::Mesh {
                vert: vs..self.mesh_storage.vertices.len(),
                tri_index: ts..self.mesh_storage.tri_indices.len(),
            }),
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
        debug_assert!(vertices.len() >= 3);
        let n = vertices.len();
        self.add_mesh_fn_prealloc(n, n - 2, color, layer, |vs, ts| {
            for (d, s) in vs.iter_mut().zip(vertices) {
                d.pos = [s.x, s.y, 0.0];
            }
            for (i, t) in ts.iter_mut().enumerate() {
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
        debug_assert!(vertices.len() >= 3);
        let n = vertices.len();
        self.add_mesh_fn_prealloc(n, n - 2, color, layer, |vs, ts| {
            for (d, s) in vs.iter_mut().zip(vertices) {
                d.pos = [s.x, s.y, 0.0];
            }
            for (i, t) in ts.iter_mut().enumerate() {
                *t = TriIndicies::new(0, (i + 1) as u16, (i + 2) as u16);
            }
            (n, n - 2)
        })
    }

    /// 带 UV 的多边形扇（fan triangulation：v0 作为中心，依次 v0, vi+1, vi+2）。
    /// `vertices` 与 `uvs` 需等长，每个顶点对应一个归一化 UV 坐标。
    pub fn add_polygon_fan_uv(
        &mut self,
        vertices: &[glam::Vec2],
        uvs: &[glam::Vec2],
        color: Color,
        layer: impl Into<Layer>,
    ) -> MeshBuilder<'_> {
        debug_assert!(vertices.len() >= 3 && vertices.len() == uvs.len());
        let n = vertices.len();
        self.add_mesh_fn_prealloc(n, n - 2, color, layer, |vs, ts| {
            for (d, (p, uv)) in vs.iter_mut().zip(vertices.iter().zip(uvs)) {
                d.pos = [p.x, p.y, 0.0];
                d.uv = [uv.x, uv.y];
            }
            for (i, t) in ts.iter_mut().enumerate() {
                *t = TriIndicies::new(0, (i + 1) as u16, (i + 2) as u16);
            }
            (n, n - 2)
        })
    }

    /// 带 UV 的多边形带（strip triangulation：v0 作为中心，依次 v0, vi+1, vi+2）。
    /// `vertices` 与 `uvs` 需等长，每个顶点对应一个归一化 UV 坐标。
    pub fn add_polygon_strip_uv(
        &mut self,
        vertices: &[glam::Vec2],
        uvs: &[glam::Vec2],
        color: Color,
        layer: impl Into<Layer>,
    ) -> MeshBuilder<'_> {
        debug_assert!(vertices.len() >= 3 && vertices.len() == uvs.len());
        let n = vertices.len();
        self.add_mesh_fn_prealloc(n, n - 2, color, layer, |vs, ts| {
            for (d, (p, uv)) in vs.iter_mut().zip(vertices.iter().zip(uvs)) {
                d.pos = [p.x, p.y, 0.0];
                d.uv = [uv.x, uv.y];
            }
            for (i, t) in ts.iter_mut().enumerate() {
                *t = TriIndicies::new(0, (i + 1) as u16, (i + 2) as u16);
            }
            (n, n - 2)
        })
    }

    pub fn add_custom(
        &mut self,
        layer: impl Into<Layer>,
        cd: impl CustomDraw + 'static,
    ) -> CustomBuilder<'_> {
        let idx = self.buf_custom_draws.len();
        self.buf_custom_draws.push(Arc::new(cd));
        CustomBuilder {
            queue: &mut self.command_queue,
            cmd: Some(DrawCommand::Custom { idx }),
            layer: layer.into(),
            rstates: RStates::default(),
            has_rstates: false,
        }
    }

    pub fn white_texture(&self) -> &ArcTextureWrapped {
        &self.white_texture
    }

    pub fn render(&mut self, clear: &ClearConfig) -> &mut Self {
        let Some((st, view)) = self.begin_frame() else {
            self.command_queue.clear();
            self.mesh_storage.clear();
            return self;
        };
        self.prepare();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render2d encoder"),
            });
        let nd = clear.depth.is_some() || clear.stencil.is_some();
        let size = self
            .surface
            .get_configuration()
            .map(|c| (c.width, c.height))
            .unwrap_or((1, 1));
        if nd {
            self.ensure_depth(size.0, size.1);
        }
        let dv = if nd { self.depth_view.as_ref() } else { None };
        {
            let co = match clear.color {
                Some(c) => wgpu::Operations {
                    load: wgpu::LoadOp::Clear(c),
                    store: wgpu::StoreOp::Store,
                },
                None => wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            };
            let dsa = dv.map(|dv| wgpu::RenderPassDepthStencilAttachment {
                view: dv,
                depth_ops: clear.depth.map(|d| wgpu::Operations {
                    load: wgpu::LoadOp::Clear(d),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: clear.stencil.map(|s| wgpu::Operations {
                    load: wgpu::LoadOp::Clear(s),
                    store: wgpu::StoreOp::Store,
                }),
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render2D: RenderPass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: co,
                })],
                depth_stencil_attachment: dsa,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            self.draw(&mut pass);
        }
        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(st);
        self.command_queue.clear();
        self.mesh_storage.clear();
        self.buf_custom_draws.clear();
        self
    }

    /// 将当前队列中的命令**只录制到用户自建的 `wgpu::RenderPass`**（仅传入 Pass，不编码/提交）。
    ///
    /// 适合离屏渲染 / 自定义 pass 组合；命令队列在录制完成后清空（与 [`Render2D::render`] 一致）。
    pub fn flush(&mut self, pass: &mut wgpu::RenderPass<'_>) {
        self.prepare();
        self.draw(pass);
        self.command_queue.clear();
        self.mesh_storage.clear();
        self.buf_custom_draws.clear();
    }

    /// 将当前队列中的命令**仅编码为 `wgpu::CommandBuffer`**（不提交、不 present）。
    ///
    /// 适合离屏渲染 / 多渲染器合并提交 / 自定义 submit 时机；用法：
    ///
    /// ```no_run
    /// # let mut render2d: rjw_2d_render::Render2D = unimplemented!();
    /// # let target: wgpu::TextureView = unimplemented!();
    /// let cb = render2d.render_command_buffer(
    ///     &rjw_2d_render::ClearConfig::default(),
    ///     &target,
    ///     None,
    /// );
    /// render2d.queue().submit(std::iter::once(cb));
    /// ```
    ///
    /// - `target`：渲染目标纹理视图（离屏纹理 / surface view 均可）。
    /// - `depth`：可选外部深度/模板视图；传 `None` 且 `clear` 需要深度时，自动按
    ///   `target` 尺寸创建 / 复用内部深度纹理。
    ///
    /// 编码完成后清空命令队列（与 [`Render2D::render`] / [`Render2D::flush`] 一致）。
    pub fn render_command_buffer(
        &mut self,
        clear: &ClearConfig,
        target: &wgpu::TextureView,
        depth: Option<&wgpu::TextureView>,
    ) -> wgpu::CommandBuffer {
        self.prepare();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render2D: command buffer encoder"),
            });
        let nd = clear.depth.is_some() || clear.stencil.is_some();
        if nd && depth.is_none() {
            let size = target.texture().size();
            self.ensure_depth(size.width, size.height);
        }
        let dv = if nd { depth.or(self.depth_view.as_ref()) } else { None };
        {
            let co = match clear.color {
                Some(c) => wgpu::Operations {
                    load: wgpu::LoadOp::Clear(c),
                    store: wgpu::StoreOp::Store,
                },
                None => wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            };
            let dsa = dv.map(|dv| wgpu::RenderPassDepthStencilAttachment {
                view: dv,
                depth_ops: clear.depth.map(|d| wgpu::Operations {
                    load: wgpu::LoadOp::Clear(d),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: clear.stencil.map(|s| wgpu::Operations {
                    load: wgpu::LoadOp::Clear(s),
                    store: wgpu::StoreOp::Store,
                }),
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render2D: command buffer pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: co,
                })],
                depth_stencil_attachment: dsa,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            self.draw(&mut pass);
        }
        let cb = encoder.finish();
        self.command_queue.clear();
        self.mesh_storage.clear();
        self.buf_custom_draws.clear();
        cb
    }

    pub fn begin_frame(&mut self) -> Option<(wgpu::SurfaceTexture, wgpu::TextureView)> {
        let t = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return None,
        };
        let v = t
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        Some((t, v))
    }

    fn prepare(&mut self) {
        self.command_queue.sort_layer_then_states();
        self.buf_instances.clear();
        self.buf_ops.clear();
        self.buf_all_verts.clear();
        self.buf_all_tris.clear();
        self.buf_items.clear();

        // ── 动态 Mesh 段累积状态（局部变量，便于宏内联访问） ──
        // 动态缓冲按排序后的命令顺序累积顶点/索引；
        // 相邻且 (rstates, tex_uid) 相同的 Mesh 命令合并为同一动态段（含多个 Mesh 命令）。
        let mut dyn_accum_verts = 0usize;
        let mut dyn_accum_tris = 0usize;
        let mut dyn_seg_tri_start: Option<usize> = None;
        let mut dyn_seg_rr: u64 = 0;
        let mut dyn_seg_tu: Option<u64> = None;
        let mut dyn_seg_layer: Layer = Layer::default();
        let mut dyn_seq_counter = 0u32;

        /// 关闭当前动态段（如果有）：push 一个 identity 实例的 BatchItem。
        /// 每个段分配唯一递增的 `dyn_seq`，保证不同动态段绝不互相合批。
        macro_rules! flush_dyn {
            () => {{
                if let Some(start) = dyn_seg_tri_start.take() {
                    let end = dyn_accum_tris;
                    if end > start {
                        dyn_seq_counter += 1;
                        self.buf_items.push(BatchItem {
                            mesh_id: None,
                            dyn_seq: dyn_seq_counter,
                            layer: dyn_seg_layer,
                            index_range: (start as u32 * 3)..(end as u32 * 3),
                            rstates: dyn_seg_rr,
                            tex_uid: dyn_seg_tu,
                            instance: InstanceData::identity(),
                        });
                    }
                }
            }};
        }

        /// 将当前 `buf_items` 按 (mesh_id, rstates, tex_uid) 排序并分组生成 DrawOp。
        /// 组内实例连续写入 `buf_instances` 并按 `MAX_INSTANCES_PER_DRAW` 分页。
        macro_rules! build_ops {
            () => {{
                if !self.buf_items.is_empty() {
                    self.buf_items.sort_by_key(|b| {
                        // 排序键：layer 为主（保证图层绘制顺序），其次为后台分组键。
                        (b.layer, b.mesh_id, b.dyn_seq, b.rstates, b.tex_uid)
                    });
                    let mut k = 0usize;
                    while k < self.buf_items.len() {
                        let mid = self.buf_items[k].mesh_id;
                        let seq = self.buf_items[k].dyn_seq;
                        let rr = self.buf_items[k].rstates;
                        let tu = self.buf_items[k].tex_uid;
                        // ── 跨层安全合批（不可移除） ──
                        // 分组键**刻意不含 layer**：当不同 layer 的元素（mesh_id + RStates + 纹理
                        // 完全相同）在按 layer 排序后的队列中**连续**（中间无其他 layer / 其他内容
                        // 插入）时，合批不会改变任何绘制顺序——因为它们在原队列中本就是相邻绘制的。
                        // 若中间夹有其他 layer 的元素，连续扫描会在此自然断开，不会误合批。
                        // 正确性由上方 sort_by_key（layer 主键保证总顺序）与 Custom 屏障共同保证。
                        // 注意：动态段按唯一 dyn_seq 分组，绝不跨段合批（否则 identity 实例会
                        // 重复绘制整段动态缓冲），此约束同样不可移除。
                        let mut j = k;
                        while j < self.buf_items.len()
                            && self.buf_items[j].mesh_id == mid
                            && self.buf_items[j].dyn_seq == seq
                            && self.buf_items[j].rstates == rr
                            && self.buf_items[j].tex_uid == tu
                        {
                            j += 1;
                        }
                        // 组内实例写入 buf_instances
                        let gs = self.buf_instances.len() as u32;
                        let n = (j - k) as u32;
                        for item in &self.buf_items[k..j] {
                            self.buf_instances.push(item.instance);
                        }
                        // 组内 index_range（静态网格组内一致；动态段每段一个 BatchItem）
                        let idx_range = self.buf_items[k].index_range.clone();
                        // 按 MAX_INSTANCES_PER_DRAW 分页
                        let first_page = gs / MAX_INSTANCES_PER_DRAW as u32;
                        let last_page = (gs + n - 1) / MAX_INSTANCES_PER_DRAW as u32;
                        for p in first_page..=last_page {
                            let ps = p * MAX_INSTANCES_PER_DRAW as u32;
                            let pe = ps + MAX_INSTANCES_PER_DRAW as u32;
                            let s = gs.max(ps);
                            let e = (gs + n).min(pe);
                            let op = if let Some(mid2) = mid {
                                DrawOp::InstancedMesh {
                                    mesh_id: mid2,
                                    page: p,
                                    instance_range: (s - ps)..(e - ps),
                                    index_range: idx_range.clone(),
                                    rstates: rr,
                                    tex_uid: tu,
                                }
                            } else {
                                DrawOp::DynamicMesh {
                                    page: p,
                                    instance_range: (s - ps)..(e - ps),
                                    index_range: idx_range.clone(),
                                    rstates: rr,
                                    tex_uid: tu,
                                }
                            };
                            self.buf_ops.push(op);
                        }
                        k = j;
                    }
                }
            }};
        }

        let vp_cull = if self.culling { Some(self.viewport_world_rect()) } else { None };
        for (cmd, layer, states) in self.command_queue.iter() {
            let tu = states.texture_uid;
            let rr = states.rstates.unwrap_or(self.default_rstates).raw();
            match cmd {
                DrawCommand::Sprite2D {
                    rect,
                    color,
                    transform,
                } => {
                    flush_dyn!();
                    // 视口剔除：世界 AABB 与视口无交集 → 跳过（不产生实例）。
                    if let Some(vp) = vp_cull {
                        let model = Self::transform2d_model(transform);
                        if !Self::sprite_in_viewport(rect, model, &vp) {
                            continue;
                        }
                    }
                    self.buf_items.push(BatchItem {
                        mesh_id: Some(self.quad_mesh_id),
                        dyn_seq: 0,
                        layer,
                        index_range: 0..QUAD_TRI_INDICIES.len() as u32,
                        rstates: rr,
                        tex_uid: tu,
                        instance: InstanceData::from_sprite(rect, *color, *transform),
                    });
                }
                DrawCommand::Sprite2DMatrix {
                    rect,
                    color,
                    mat_idx,
                } => {
                    flush_dyn!();
                    let m = self.command_queue.matrices[*mat_idx];
                    if let Some(vp) = vp_cull {
                        if !Self::sprite_in_viewport(rect, m, &vp) {
                            continue;
                        }
                    }
                    self.buf_items.push(BatchItem {
                        mesh_id: Some(self.quad_mesh_id),
                        dyn_seq: 0,
                        layer,
                        index_range: 0..QUAD_TRI_INDICIES.len() as u32,
                        rstates: rr,
                        tex_uid: tu,
                        instance: InstanceData::from_sprite_matrix(rect, *color, m),
                    });
                }
                DrawCommand::StaticMesh {
                    mesh_id,
                    color,
                    transform,
                } => {
                    flush_dyn!();
                    let mesh = MESHES.get(*mesh_id).expect("mesh not registered");
                    self.buf_items.push(BatchItem {
                        mesh_id: Some(*mesh_id),
                        dyn_seq: 0,
                        layer,
                        index_range: 0..mesh.index_count,
                        rstates: rr,
                        tex_uid: tu,
                        instance: InstanceData::from_static_transform(*color, *transform),
                    });
                }
                DrawCommand::StaticMeshMatrix {
                    mesh_id,
                    color,
                    mat_idx,
                } => {
                    flush_dyn!();
                    let m = self.command_queue.matrices[*mat_idx];
                    let mesh = MESHES.get(*mesh_id).expect("mesh not registered");
                    self.buf_items.push(BatchItem {
                        mesh_id: Some(*mesh_id),
                        dyn_seq: 0,
                        layer,
                        index_range: 0..mesh.index_count,
                        rstates: rr,
                        tex_uid: tu,
                        instance: InstanceData::from_static(*color, m),
                    });
                }
                DrawCommand::Mesh { vert, tri_index } => {
                    let vn = vert.end - vert.start;
                    let tn = tri_index.end - tri_index.start;
                    // 状态/纹理变化 → 关闭当前动态段，重新打开
                    if dyn_seg_tri_start.is_some() && (dyn_seg_rr != rr || dyn_seg_tu != tu) {
                        flush_dyn!();
                    }
                    if dyn_seg_tri_start.is_none() {
                        dyn_seg_tri_start = Some(dyn_accum_tris);
                        dyn_seg_rr = rr;
                        dyn_seg_tu = tu;
                        dyn_seg_layer = layer;
                    }
                    if vn != 0 {
                        self.buf_all_verts
                            .extend_from_slice(&self.mesh_storage.vertices[vert.clone()]);
                    }
                    if vn != 0 && tn != 0 {
                        let rb = (dyn_accum_verts as i64) - (vert.start as i64);
                        for t in &self.mesh_storage.tri_indices[tri_index.clone()] {
                            self.buf_all_tris.push(TriIndicies(
                                Index((t.0.0 as i64 + rb) as u16),
                                Index((t.1.0 as i64 + rb) as u16),
                                Index((t.2.0 as i64 + rb) as u16),
                            ));
                        }
                    }
                    dyn_accum_verts += vn;
                    dyn_accum_tris += tn;
                }
                DrawCommand::Custom { idx } => {
                    // Custom 是合批屏障：关闭动态段、冲刷已收集 items。
                    flush_dyn!();
                    build_ops!();
                    self.buf_items.clear();
                    // `idx` 由 `add_custom` 分配、随命令参与排序，
                    // 保证排序后仍指向 `buf_custom_draws` 中正确的闭包。
                    self.buf_ops.push(DrawOp::Custom { idx: *idx });
                }
            }
        }
        flush_dyn!();
        build_ops!();

        // ── 上传实例缓冲 ──
        if !self.buf_instances.is_empty() {
            let pc =
                (self.buf_instances.len() + MAX_INSTANCES_PER_DRAW - 1) / MAX_INSTANCES_PER_DRAW;
            self.draw_page.ensure_instance_pages(&self.device, pc);
            let mut pi = 0;
            let mut s = 0;
            while s < self.buf_instances.len() {
                let e = (s + MAX_INSTANCES_PER_DRAW).min(self.buf_instances.len());
                self.draw_page
                    .update_instances_page(&self.queue, pi, &self.buf_instances[s..e]);
                pi += 1;
                s = e;
            }
        }

        // ── 上传动态网格缓冲 ──
        if !self.buf_all_verts.is_empty() {
            assert!(self.buf_all_verts.len() <= MAX_MESH_VERTS);
            self.draw_page
                .ensure_mesh_capacity(&self.device, self.buf_all_verts.len(), self.buf_all_tris.len() + 1);
            self.queue
                .write_buffer(&self.draw_page.mesh_vb, 0, bytemuck::cast_slice(&self.buf_all_verts));
        }
        if !self.buf_all_tris.is_empty() {
            let bs = bytemuck::cast_slice(&self.buf_all_tris);
            let pl = (bs.len() + 3) & !3;
            self.buf_padded.clear();
            self.buf_padded.extend_from_slice(bs);
            self.buf_padded.resize(pl, 0u8);
            self.queue
                .write_buffer(&self.draw_page.mesh_ib, 0, &self.buf_padded);
        }

        // ── 清理失效 bind group 缓存 ──
        // 用户调用 `TEXTURES.remove(uid)` 后，缓存条目在此剔除，
        // 其持有的 Arc<Texture> 与 BindGroup 一并 drop，GPU 资源正确释放。
        if !self.tex_bind_group_cache.is_empty() {
            self.tex_bind_group_cache
                .retain(|&(tex_uid, _), _| TEXTURES.contains_uid(tex_uid));
        }
    }

    /// 采样器位域取出（RStates bits 8..24，与 rstates.rs 的采样器域一致）。
    const SAMPLER_KEY_MASK: u64 = 0xFF_FF00;

    /// 解析纹理 uid → `Arc<TextureWrapped>`（`None` 使用白纹理），并确保该纹理在注册表中。
    fn resolve_tex(&self, tex_uid: Option<u64>) -> ArcTextureWrapped {
        match tex_uid {
            Some(uid) => TEXTURES.get(uid).expect("tex not found in TEXTURES"),
            None => self.white_texture.clone(),
        }
    }

    /// 绑定 group(1) 纹理 bind group（纹理 + 采样器缓存复用）。
    /// bind group 缓存持有 `Arc<Texture>` —— 纹理被 `TEXTURES.remove` 后由 prepare 末尾清理，资源正确释放。
    fn bind_tex_group(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        tex_uid: Option<u64>,
        rstates: u64,
    ) {
        let tex = self.resolve_tex(tex_uid);
        let samp_key = rstates & Self::SAMPLER_KEY_MASK;
        let key = (tex.uid, samp_key);
        let bg = {
            let cache = &mut self.tex_bind_group_cache;
            match cache.entry(key) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut().1.clone(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let sampler = if samp_key == 0 {
                        self.default_sampler.clone()
                    } else {
                        self.sampler_cache
                            .entry(samp_key)
                            .or_insert_with(|| {
                                self.device.create_sampler(&RStates::from_raw(rstates).to_sampler_desc())
                            })
                            .clone()
                    };
                    let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Render2D: Tex bind group"),
                        layout: &self.tex_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(tex.view()),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&sampler),
                            },
                        ],
                    });
                    e.insert((tex, group)).1.clone()
                }
            }
        };
        pass.set_bind_group(1, &bg, &[]);
    }

    fn draw(&mut self, pass: &mut wgpu::RenderPass<'_>) {
        if self.buf_ops.is_empty() {
            return;
        }
        let mut i = 0usize;
        while i < self.buf_ops.len() {
            match &self.buf_ops[i] {
                DrawOp::InstancedMesh {
                    mesh_id,
                    page,
                    instance_range,
                    index_range,
                    rstates,
                    tex_uid,
                } => {
                    // 先复制字段，释放 `&self.buf_ops` 借用，再执行 `&mut self` 操作。
                    let (mesh_id, page, instance_range, index_range, rstates, tex_uid) = (
                        *mesh_id,
                        *page,
                        instance_range.clone(),
                        index_range.clone(),
                        *rstates,
                        *tex_uid,
                    );
                    let count = instance_range.end - instance_range.start;
                    if count != 0 {
                        let mesh = MESHES.get(mesh_id).expect("mesh not registered");
                        let pipeline = self
                            .draw_page
                            .get_or_create_pipeline(&self.device, rstates);
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, &self.draw_page.vp_bind_group, &[]);
                        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(
                            1,
                            self.draw_page
                                .instance_page_buffer(page as usize)
                                .slice(..),
                        );
                        pass.set_index_buffer(
                            mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        self.bind_tex_group(pass, tex_uid, rstates);
                        pass.draw_indexed(index_range, 0, instance_range);
                    }
                    i += 1;
                }
                DrawOp::DynamicMesh {
                    page,
                    instance_range,
                    index_range,
                    rstates,
                    tex_uid,
                } => {
                    // 先复制字段，释放 `&self.buf_ops` 借用，再执行 `&mut self` 操作。
                    let (page, instance_range, index_range, rstates, tex_uid) = (
                        *page,
                        instance_range.clone(),
                        index_range.clone(),
                        *rstates,
                        *tex_uid,
                    );
                    let count = instance_range.end - instance_range.start;
                    if count != 0 {
                        let pipeline = self
                            .draw_page
                            .get_or_create_pipeline(&self.device, rstates);
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, &self.draw_page.vp_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.draw_page.mesh_vb.slice(..));
                        pass.set_vertex_buffer(
                            1,
                            self.draw_page
                                .instance_page_buffer(page as usize)
                                .slice(..),
                        );
                        pass.set_index_buffer(
                            self.draw_page.mesh_ib.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        self.bind_tex_group(pass, tex_uid, rstates);
                        pass.draw_indexed(index_range, 0, instance_range);
                    }
                    i += 1;
                }
                DrawOp::Custom { idx } => {
                    let cd = Arc::clone(&self.buf_custom_draws[*idx]);
                    cd.draw(pass);
                    i += 1;
                }
            }
        }
    }

    fn ensure_depth(&mut self, w: u32, h: u32) {
        if self
            .depth_view
            .as_ref()
            .is_some_and(|_| self.depth_size == (w.max(1), h.max(1)))
        {
            return;
        }
        let t = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth-stencil"),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.depth_view = Some(t.create_view(&wgpu::TextureViewDescriptor::default()));
        self.depth_size = (w.max(1), h.max(1));
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}