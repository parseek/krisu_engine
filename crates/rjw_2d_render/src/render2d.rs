//! `Render2D`：2D 批渲染器门面（录制 → 排序 → 剔除 → 合批 → 提交）。
//!
//! # 责任划分
//!
//! | 职责 | 位置 |
//! |---|---|
//! | 录制（画什么） | 本文件入口方法 → [`crate::draw::Draw2D`] 统一 Builder |
//! | 排序（索引数组重排） | [`crate::sort`]（[`SortPolicy`] / [`SortMode`]） |
//! | 剔除（索引数组过滤） | [`crate::cull`]（[`Culler`] / [`Cull`]） |
//! | 全局默认状态 | [`Render2D::states`] / [`Render2D::set_states`]（`RStates` 唯一状态语言） |
//! | 提交 | [`Render2D::render`] / [`Render2D::record`] / [`Render2D::encode`] / [`Render2D::acquire_frame`] |
//! | 资源 | [`Render2D::create_texture`] / [`Render2D::register_mesh`] / [`Render2D::texture_layout`] |
//!
//! # 每帧流程
//!
//! ```text
//! set_mvp / set_cull / set_sort_mode（可选）
//!   → sprite/... 入口录制命令（链式 .layer().color()...）
//!   → render(&ClearConfig)：prepare() 排序 + 剔除 + 分页 → draw() → submit → present
//! ```
//!
//! 坐标系（与 `rjw_transform::Camera2D` 一致）：原点在视口中心、X+ 右、Y+ 下。

use std::{collections::HashMap, sync::Arc};

use rjw_render::{ArcTextureWrapped, MeshData, MESHES, TEXTURES, TextureWrapped};
use rjw_transform::Camera2D;

use crate::command::{DrawCommand, DrawCommandQueue, Layer};
use crate::cull::{self, Cull, Culler};
use crate::data::{
    Index, MeshSink, MeshStorage, QUAD_TRI_INDICIES, SpriteRect, TriIndicies, VertexP3U2C4,
};
use crate::draw::{Custom, Draw2D, Mesh, Sprite, StaticMesh};
use crate::draw_page::{
    DEPTH_FORMAT, DrawOp, DrawPage, InstanceData, MAX_INSTANCES_PER_DRAW, MAX_MESH_VERTS,
};
use crate::rstates::RStates;
use crate::sort::{SortKey, SortMode, SortPolicy};

// 对外可见的绘制相关类型（入口签名 / 闭包 sink）。
pub use crate::draw::{CustomDraw, PolygonSink, QuadSink};

// ─── Builder 类型别名（4 个 kind 共用同一实现） ───────────────

/// `sprite` / `solid` 返回的 Builder。
pub type SpriteBuilder<'a> = Draw2D<'a, Sprite>;
/// `mesh` / `mesh_with` / `polygon` / `quads` 返回的 Builder。
pub type MeshBuilder<'a> = Draw2D<'a, Mesh>;
/// `static_mesh` 返回的 Builder。
pub type StaticMeshBuilder<'a> = Draw2D<'a, StaticMesh>;
/// `custom` 返回的 Builder。
pub type CustomBuilder<'a> = Draw2D<'a, Custom>;

// ─── Clear 配置 ───────────────────────────────────────────────

/// 清屏配置：`None` = 保留旧内容。
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

// ─── 合批中间项 ───────────────────────────────────────────────

/// `prepare` 阶段的合批中间项。
///
/// - `mesh_id`: `Some(uid)` 为注册表网格（Sprite / StaticMesh）；`None` 为动态缓冲段
///   （`mesh*` / `polygon*` / `quads*` 入口，此时 `index_range` 为该段在动态索引缓冲中的范围）。
/// - `dyn_seq`: 动态段每帧递增的唯一序号（`0` 表示静态项）。
///   每个动态段恰好一个 identity 实例；seq 唯一保证**不同动态段绝不互相合批**。
/// - `layer`: 绘制层级（越小越先绘制）。排序键以 layer 为主，**保证跨层级合批
///   不会打乱图层顺序**。
/// - `index_range`: 索引范围（静态网格 = `0..index_count`；动态段 = `tri*3` 范围）。
struct BatchItem {
    mesh_id: Option<u64>,
    dyn_seq: u32,
    layer: Layer,
    index_range: std::ops::Range<u32>,
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

    /// 排序模式（[`Render2D::set_sort_mode`]，默认 [`SortMode::LayerAndStates`]）。
    sort_mode: SortMode,
    /// 自定义排序策略（[`Render2D::set_sorter`]）：`Some` 时覆盖 `sort_mode`。
    sorter: Option<Box<dyn SortPolicy>>,
    /// 剔除器（[`Render2D::set_cull`]，默认关闭）。
    culler: Culler,
    /// 全局默认渲染状态（[`Render2D::set_states`]）：未链式设置状态的命令继承它。
    default_states: RStates,

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
    /// 排序键常驻缓冲（每帧复用，零堆分配）。
    buf_sort_keys: Vec<SortKey>,
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

        // 视口缓存初始化为单位 MVP 对应的世界矩形（未调用 set_mvp 时 Cull::Viewport 也可用）。
        let mut culler = Culler::new(Cull::Off);
        culler.set_viewport(cull::viewport_world_rect(&glam::Mat4::IDENTITY));

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
            sort_mode: SortMode::LayerAndStates,
            sorter: None,
            culler,
            default_states: RStates::default(),
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
            buf_sort_keys: Vec::new(),
        }
    }

    // ── 相机 / 排序 / 剔除 / 全局状态 ───────────────────────

    /// 设置 VP（视图投影）矩阵（每帧渲染前调用）；同时刷新 `Cull::Viewport` 的视口矩形。
    pub fn set_mvp(&mut self, vp: glam::Mat4) -> &mut Self {
        self.mvp = vp;
        self.culler.set_viewport(cull::viewport_world_rect(&vp));
        self.draw_page.update_vp(&self.queue, vp);
        self
    }

    /// 命令排序模式（默认 [`SortMode::LayerAndStates`]）。
    ///
    /// - [`SortMode::LayerAndStates`]：按 `(layer, states)` 排序后合批（引擎默认）；
    /// - [`SortMode::LayerOnly`]：仅按 layer 稳定排序（同层保持录制顺序），UI 适用；
    /// - [`SortMode::None`]：完全按录制顺序（相邻同状态仍合批）。
    #[inline]
    pub fn set_sort_mode(&mut self, mode: SortMode) -> &mut Self {
        self.sort_mode = mode;
        self
    }

    /// 自定义排序策略（覆盖 [`Self::set_sort_mode`]；传 `None` 恢复内置模式）。
    #[inline]
    pub fn set_sorter(&mut self, sorter: Option<Box<dyn SortPolicy>>) -> &mut Self {
        self.sorter = sorter;
        self
    }

    /// 当前排序模式（设置自定义策略后仍返回最后一次设置的内置模式）。
    #[inline]
    pub fn sort_mode(&self) -> SortMode {
        self.sort_mode
    }

    /// 剔除模式（**单一入口**，无隐式联动）：
    /// [`Cull::Off`] / [`Cull::Viewport`] / [`Cull::Rect`] / [`Cull::Fn`]。
    #[inline]
    pub fn set_cull(&mut self, cull: impl Into<Cull>) -> &mut Self {
        self.culler.set(cull.into());
        self
    }

    /// 以 2D 相机剔除（`None` = 关闭；等价 `set_cull(Cull::from(&cam))`）。
    #[inline]
    pub fn set_cull_camera(&mut self, cam: Option<&Camera2D>) -> &mut Self {
        self.set_cull(cam.map(Cull::from).unwrap_or(Cull::Off))
    }

    /// 当前剔除模式。
    #[inline]
    pub fn cull(&self) -> &Cull {
        self.culler.mode()
    }

    /// 剔除器（只读；供下游复用同一套可见性判定）。
    #[inline]
    pub fn culler(&self) -> &Culler {
        &self.culler
    }

    /// 剔除器（可变；可刷新视口 / 直接 `retain` 过滤索引数组）。
    #[inline]
    pub fn culler_mut(&mut self) -> &mut Culler {
        &mut self.culler
    }

    /// 全局默认渲染状态（未链式设置状态的命令继承它）。
    #[inline]
    pub fn states(&self) -> RStates {
        self.default_states
    }

    /// 设置全局默认渲染状态（**唯一状态入口**）：
    /// `r2d.set_states(RStates::new().blend(Additive).depth_test(true))`。
    #[inline]
    pub fn set_states(&mut self, states: RStates) -> &mut Self {
        self.default_states = states;
        self
    }

    /// 重置全局默认状态为出厂默认（全零 bitfield）。
    #[inline]
    pub fn reset_states(&mut self) -> &mut Self {
        self.default_states = RStates::default();
        self
    }

    // ── 资源 ────────────────────────────────────────────────

    /// 纹理 bind group layout（自建 bind group 的下游用：`rjw_text` / `rjw_ui` 等）。
    #[inline]
    pub fn texture_layout(&self) -> &wgpu::BindGroupLayout {
        &self.tex_bind_group_layout
    }

    /// 创建 RGBA8 纹理并注册进全局 `TEXTURES`
    /// （`data.len()` 必须等于 `w * h * 4`，否则 panic）。
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

    /// 注册已有纹理进全局 `TEXTURES`。
    #[inline]
    pub fn register_texture(&self, tex: ArcTextureWrapped) {
        TEXTURES.register(tex);
    }

    /// 注册静态网格到全局 `MESHES`，返回可复用 `mesh_id`。
    ///
    /// 相同内容的网格应**复用同一个** `Arc<MeshData>` 注册，否则无法合批。
    #[inline]
    pub fn register_mesh(&self, mesh: Arc<MeshData>) -> u64 {
        MESHES.register(mesh)
    }

    /// 1×1 白色纹理（纯色绘制用；`solid` 内部即用它）。
    #[inline]
    pub fn white_texture(&self) -> &ArcTextureWrapped {
        &self.white_texture
    }

    #[inline]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    #[inline]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// 当前 VP 矩阵。
    #[inline]
    pub fn mvp(&self) -> glam::Mat4 {
        self.mvp
    }

    // ── 录制入口（画什么）：数据版 ──────────────────────────

    /// 贴纹理精灵：`r2d.sprite(rect, &tex).color(..).transform(..).layer(..)`。
    #[inline]
    pub fn sprite(
        &mut self,
        rect: impl Into<SpriteRect>,
        texture: &ArcTextureWrapped,
    ) -> SpriteBuilder<'_> {
        Draw2D::sprite(
            &mut self.command_queue,
            &mut self.mesh_storage,
            rect.into(),
            texture.uid,
        )
    }

    /// 纯色精灵（内部用 1×1 白纹理）：`r2d.solid(rect).color(..)`。
    #[inline]
    pub fn solid(&mut self, rect: impl Into<SpriteRect>) -> SpriteBuilder<'_> {
        let uid = self.white_texture.uid;
        Draw2D::sprite(&mut self.command_queue, &mut self.mesh_storage, rect.into(), uid)
    }

    /// 四边形段（顶点由调用者提供，顺序 **TL, TR, BL, BR**）：
    /// `r2d.quads(&verts, &tex).transform(tf).color(tint).layer(l)`。
    #[inline]
    pub fn quads(
        &mut self,
        vertices: &[VertexP3U2C4],
        texture: &ArcTextureWrapped,
    ) -> MeshBuilder<'_> {
        Draw2D::quads(
            &mut self.command_queue,
            &mut self.mesh_storage,
            vertices,
            texture.uid,
        )
    }

    /// 四边形段（**流式构造**，零临时 `Vec`）：
    /// `r2d.quads_with(|q| { q.quad(tl, tr, bl, br); }, &tex)`。
    #[inline]
    pub fn quads_with<F>(&mut self, f: F, texture: &ArcTextureWrapped) -> MeshBuilder<'_>
    where
        F: FnOnce(&mut QuadSink<'_>),
    {
        Draw2D::quads_with(
            &mut self.command_queue,
            &mut self.mesh_storage,
            f,
            texture.uid,
        )
    }

    // ── 录制入口（画什么）：网格 / 多边形 / 流式 ────────────

    /// 显式顶点 + 三角形索引（世界坐标；默认白纹理）：
    /// `r2d.mesh(&verts, &tris).color(..).transform(..).layer(..)`。
    #[inline]
    pub fn mesh(&mut self, vertices: &[glam::Vec2], tri_indices: &[u16]) -> MeshBuilder<'_> {
        Draw2D::mesh(
            &mut self.command_queue,
            &mut self.mesh_storage,
            vertices,
            tri_indices,
            None,
        )
    }

    /// **流式构造网格**（零临时 `Vec`；自定三角化 / 逐顶点 UV）：
    /// `r2d.mesh_with(|s| { let a = s.push_vertex(p); ... s.push_tri(a, b, c); })`。
    #[inline]
    pub fn mesh_with<F>(&mut self, f: F) -> MeshBuilder<'_>
    where
        F: FnOnce(&mut MeshSink<'_>),
    {
        Draw2D::mesh_with(&mut self.command_queue, &mut self.mesh_storage, f, None)
    }

    /// 预分配流式构造网格（已知顶点 / 三角形数时的零重分配快路径）。
    #[inline]
    pub fn mesh_with_cap<F>(&mut self, max_verts: usize, max_tris: usize, f: F) -> MeshBuilder<'_>
    where
        F: FnOnce(&mut [VertexP3U2C4], &mut [TriIndicies]) -> (usize, usize),
    {
        Draw2D::mesh_with_cap(
            &mut self.command_queue,
            &mut self.mesh_storage,
            max_verts,
            max_tris,
            f,
            None,
        )
    }

    /// 多边形（**fan 三角化**：首顶点为中心）：`r2d.polygon(&verts).color(..).layer(..)`。
    #[inline]
    pub fn polygon(&mut self, vertices: &[glam::Vec2]) -> MeshBuilder<'_> {
        Draw2D::polygon(
            &mut self.command_queue,
            &mut self.mesh_storage,
            vertices,
            None,
            None,
        )
    }

    /// 带 UV 的多边形（`vertices` 与 `uvs` 等长；fan 三角化）。
    #[inline]
    pub fn polygon_uv(&mut self, vertices: &[glam::Vec2], uvs: &[glam::Vec2]) -> MeshBuilder<'_> {
        Draw2D::polygon(
            &mut self.command_queue,
            &mut self.mesh_storage,
            vertices,
            Some(uvs),
            None,
        )
    }

    /// **流式构造多边形**（闭包结束自动 fan 三角化，零临时 `Vec`）：
    /// `r2d.polygon_with(|p| { p.vertex(a); p.vertex(b); p.vertex(c); })`。
    #[inline]
    pub fn polygon_with<F>(&mut self, f: F) -> MeshBuilder<'_>
    where
        F: FnOnce(&mut PolygonSink<'_>),
    {
        Draw2D::polygon_with(&mut self.command_queue, &mut self.mesh_storage, f, None)
    }

    /// 静态网格实例（`MESHES` 注册表 + 实例化合批）：
    /// `r2d.static_mesh(id, &tex).color(..).transform(tf).layer(..)`。
    #[inline]
    pub fn static_mesh(
        &mut self,
        mesh_id: u64,
        texture: &ArcTextureWrapped,
    ) -> StaticMeshBuilder<'_> {
        debug_assert!(
            MESHES.contains_uid(mesh_id),
            "mesh {mesh_id} is not registered in MESHES"
        );
        Draw2D::static_mesh(
            &mut self.command_queue,
            &mut self.mesh_storage,
            mesh_id,
            texture.uid,
        )
    }

    /// 外部绘制（逃逸舱口）：`r2d.custom(|pass| { ... }).layer(..)`。
    ///
    /// 闭包在 `render()` / `record()` 的 `draw()` 阶段被调用，此时 `RenderPass` 已打开——
    /// **不要**在闭包内 `begin_render_pass`。
    #[inline]
    pub fn custom(&mut self, cd: impl CustomDraw + 'static) -> CustomBuilder<'_> {
        let idx = self.buf_custom_draws.len();
        self.buf_custom_draws.push(Arc::new(cd));
        Draw2D::custom(&mut self.command_queue, &mut self.mesh_storage, idx)
    }

    // ── 提交（何时画） ─────────────────────────────────────

    /// 清空本帧录制内容（命令队列 / 动态网格 / 外部绘制句柄）。
    ///
    /// 三个提交出口（[`Self::render`] / [`Self::record`] / [`Self::encode`]）都以此收尾，
    /// 契约一致：**提交即清帧**。
    #[inline]
    fn clear_frame(&mut self) {
        self.command_queue.clear();
        self.mesh_storage.clear();
        self.buf_custom_draws.clear();
    }

    /// 全流程提交：`acquire_frame` → `prepare` → 绘制 → submit → `present`。
    ///
    /// 取帧失败（超时 / 丢失）时仅清帧并返回，不 panic。
    pub fn render(&mut self, clear: &ClearConfig) -> &mut Self {
        let Some((st, view)) = self.acquire_frame() else {
            self.clear_frame();
            return self;
        };
        self.prepare();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render2D: encoder"),
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
            let co = color_ops(clear.color);
            let dsa = depth_attachment(dv, clear);
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
        self.clear_frame();
        self
    }

    /// 把当前队列**只录制进用户自建的 `wgpu::RenderPass`**（不编码 / 不提交）。
    ///
    /// 适合离屏渲染 / 自定义 pass 组合；录制后清帧（与 [`Self::render`] 一致）。
    pub fn record(&mut self, pass: &mut wgpu::RenderPass<'_>) {
        self.prepare();
        self.draw(pass);
        self.clear_frame();
    }

    /// 把当前队列**只编码为 `wgpu::CommandBuffer`**（不提交 / 不 present）。
    ///
    /// - `target`：渲染目标纹理视图（离屏纹理 / surface view 均可）；
    /// - `depth`：可选外部深度/模板视图；传 `None` 且 `clear` 需要深度时，
    ///   自动按 `target` 尺寸创建 / 复用内部深度纹理。
    ///
    /// 编码后清帧（与 [`Self::render`] / [`Self::record`] 一致）。
    pub fn encode(
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
            let co = color_ops(clear.color);
            let dsa = depth_attachment(dv, clear);
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
        self.clear_frame();
        cb
    }

    /// 取当前表面帧（`None` = 取帧失败 / 丢失，调用方应跳过本帧）。
    pub fn acquire_frame(&mut self) -> Option<(wgpu::SurfaceTexture, wgpu::TextureView)> {
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

    // ── prepare：排序（索引数组）→ 剔除（索引数组）→ 合批 → 上传 ──

    /// 对**索引数组**应用排序策略与剔除器（原地操作，零额外分配）。
    fn apply_order(&mut self) {
        if self.command_queue.is_empty() {
            return;
        }
        // 先取排序键（此时索引数组仍与命令一一对应），再移交索引数组的所有权。
        self.command_queue.fill_sort_keys(&mut self.buf_sort_keys);
        let mut order = self.command_queue.take_order();
        match &self.sorter {
            Some(policy) => policy.sort(&mut order, &self.buf_sort_keys),
            None => self.sort_mode.sort(&mut order, &self.buf_sort_keys),
        }
        if !self.culler.is_off() {
            let q = &self.command_queue;
            self.culler.retain(&mut order, |i| q.cull_aabb(i));
        }
        self.command_queue.set_order(order);
    }

    fn prepare(&mut self) {
        self.apply_order();

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
        let mut dyn_seg_mat: Option<usize> = None;
        let mut dyn_seg_color: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
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
                            instance: match dyn_seg_mat {
                                Some(mi) => {
                                    let m = self.command_queue.matrices[mi];
                                    InstanceData::from_model_tinted(m, dyn_seg_color)
                                }
                                None => InstanceData::identity(),
                            },
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

        // 排序与剔除已在 `apply_order()` 阶段作用于索引数组（见 `crate::sort` / `crate::cull`），
        // 此处只按最终顺序生成实例与绘制操作。
        for (cmd, layer, states) in self.command_queue.iter() {
            let tu = states.texture_uid;
            let rr = states.rstates.unwrap_or(self.default_states).raw();
            match cmd {
                DrawCommand::Sprite2D {
                    rect,
                    color,
                    transform,
                } => {
                    flush_dyn!();
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
                DrawCommand::Mesh { vert, tri_index, mat_idx } => {
                    let vn = vert.end - vert.start;
                    let tn = tri_index.end - tri_index.start;
                    // 状态/纹理/**变换**变化 → 关闭当前动态段，重新打开
                    // （不同 transform 的段必须分开，各自 identity 实例带自己的 model）
                    if dyn_seg_tri_start.is_some()
                        && (dyn_seg_rr != rr || dyn_seg_tu != tu || dyn_seg_mat != *mat_idx)
                    {
                        flush_dyn!();
                    }
                    if dyn_seg_tri_start.is_none() {
                        dyn_seg_tri_start = Some(dyn_accum_tris);
                        dyn_seg_rr = rr;
                        dyn_seg_tu = tu;
                        dyn_seg_mat = *mat_idx;
                        dyn_seg_color = [1.0, 1.0, 1.0, 1.0];
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
                DrawCommand::MeshStyled { vert, tri_index, mat_idx, color } => {
                    // **已提前合批**：自成一整段一次 draw（前后 flush），实例带 model +
                    // 整段混合色（shader 里 顶点色×实例色）。不参与跨段合批比较。
                    flush_dyn!();
                    dyn_seg_tri_start = Some(dyn_accum_tris);
                    dyn_seg_rr = rr;
                    dyn_seg_tu = tu;
                    dyn_seg_mat = *mat_idx;
                    dyn_seg_color = *color;
                    dyn_seg_layer = layer;
                    let vn = vert.end - vert.start;
                    let tn = tri_index.end - tri_index.start;
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
                    flush_dyn!();
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
}

// ─── Pass 附件辅助（render / encode 共用） ─────────────────────

/// 颜色附件操作（`None` = 保留旧内容）。
fn color_ops(color: Option<wgpu::Color>) -> wgpu::Operations<wgpu::Color> {
    match color {
        Some(c) => wgpu::Operations {
            load: wgpu::LoadOp::Clear(c),
            store: wgpu::StoreOp::Store,
        },
        None => wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
        },
    }
}

/// 深度/模板附件（`clear` 未要求清除或视图为空时返回 `None`）。
fn depth_attachment<'a>(
    view: Option<&'a wgpu::TextureView>,
    clear: &ClearConfig,
) -> Option<wgpu::RenderPassDepthStencilAttachment<'a>> {
    if clear.depth.is_none() && clear.stencil.is_none() {
        return None;
    }
    let view = view?;
    Some(wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: clear.depth.map(|d| wgpu::Operations {
            load: wgpu::LoadOp::Clear(d),
            store: wgpu::StoreOp::Store,
        }),
        stencil_ops: clear.stencil.map(|s| wgpu::Operations {
            load: wgpu::LoadOp::Clear(s),
            store: wgpu::StoreOp::Store,
        }),
    })
}
