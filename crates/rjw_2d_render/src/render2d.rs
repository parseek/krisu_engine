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
//! | 提交（录进帧 / pass） | [`Render2D::render`] / [`Render2D::record`] / [`Render2D::record_to`] |
//! | 帧 / pass 生命周期 | `rjw_render::{RenderContext::begin_frame, RenderFrame, PassScope}` |
//! | 资源 | [`Render2D::create_texture`] / [`Render2D::register_mesh`] / [`Render2D::texture_layout`] |
//!
//! # 每帧流程
//!
//! ```text
//! set_camera / set_mvp / set_viewport / set_cull / set_sort_mode（可选）
//!   → sprite/... 入口录制命令（链式 .layer().tint()...）
//!   → record(&mut frame, &ClearConfig)：prepare() 排序 + 剔除 + 分页 → draw()
//!   → render(&mut ctx, &ClearConfig)：= begin_frame + record + present（一行糖）
//! ```
//!
//! **多画面 / 多视口**：一个画面 = 一次 `record` = 一个 pass（`set_camera` 同时带入该画面的
//! mvp 与屏幕矩形，绘制时自动 `set_viewport` / `set_scissor_rect`）；
//! 若要让多个渲染器共用同一个 pass（省一次 Load/Store，共用深度附件），用
//! `frame.begin_pass(..)` + [`rjw_render::PassScope::record`] 多次提交。
//!
//! 坐标系（与 `rjw_transform::Camera2D` 一致）：原点在视口中心、X+ 右、Y+ 下。

use std::{collections::HashMap, sync::Arc};

use rjw_render::{
    ArcTextureWrapped, Gpu, MeshData, MeshId, MeshRegistry, PassBuilder, PassContext, PassRecorder,
    TextureRegistry, TextureWrapped,
};
#[cfg(feature = "rjw_atlas")]
use rjw_atlas::AtlasSprite;
use rjw_transform::{Camera2D, Rect};

use crate::command::{DrawCommand, DrawCommandQueue, Layer};
use crate::cull::{self, Cull, Culler};
use crate::debug_draw::{DebugPainter, DebugStyle};
use crate::data::{
    Index, MeshSink, MeshStorage, QUAD_TRI_INDICIES, SpriteRect, TriIndices, VertexP3U2C4,
};
use crate::draw::{Custom, Draw2D, Mesh, Sprite, StaticMesh};
use crate::draw_page::{
    DrawOp, DrawPage, InstanceData, MAX_INSTANCES_PER_DRAW, MAX_MESH_VERTS,
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
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// 颜色附件格式（= surface 格式；构造期固定）。
    format: wgpu::TextureFormat,
    /// 深度 / 模板附件格式（构造期固定；**不进管线缓存 key**）。
    depth_format: wgpu::TextureFormat,
    tex_bind_group_layout: wgpu::BindGroupLayout,
    /// **本上下文**的 GPU 能力对象（注册表所有者）。经 `Arc` 共享，使渲染器可以
    /// 持有比 `&Gpu` 更长的生命周期，并把同一套注册表交给 `rjw_text` / `rjw_ui`。
    gpu: Arc<Gpu>,
    /// 本上下文的纹理注册表（`gpu.textures()` 的克隆；绘制期唯一解析点）。
    textures: Arc<TextureRegistry>,
    /// 本上下文的静态网格注册表（`gpu.meshes()` 的克隆；绘制期唯一解析点）。
    meshes: Arc<MeshRegistry>,
    white_texture: ArcTextureWrapped,
    mesh_storage: MeshStorage,
    command_queue: DrawCommandQueue,
    draw_page: DrawPage,
    /// 本画面的 VP（由 [`Render2D::camera`] / [`Render2D::submit`] 设定；
    /// 提交时经 [`rjw_render::PassRecorder::view_projection`] 交给**帧级 VP 槽环**）。
    vp: glam::Mat4,
    /// 本画面的屏幕矩形（像素、左上原点）：`Some` ⇒ 绘制时 `set_viewport` + `set_scissor_rect`；
    /// `None`（默认）= 全屏、不调用（与旧行为逐位等价、零开销）。多画面用
    /// [`Render2D::camera`] 或 [`Render2D::viewport`] 设定。
    viewport: Option<Rect>,
    /// scissor 覆盖（`None` = 跟随 [`Self::viewport`]；防止旋转相机 / 大几何越界串味）。
    scissor: Option<Rect>,

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
    buf_all_tris: Vec<TriIndices>,
    buf_padded: Vec<u8>,
    buf_custom_draws: Vec<Arc<dyn CustomDraw>>,
    /// 排序键常驻缓冲（每帧复用，零堆分配）。
    buf_sort_keys: Vec<SortKey>,
}

impl Render2D {
    /// 从 [`rjw_render::RenderContext`] 构造（取 `Gpu` 能力对象 + 格式）。
    ///
    /// **不持有 surface / VP**：取帧与 present 走 `RenderContext`（唯一取帧权威）；
    /// VP 由帧级槽环按 pass 提供（见 [`Self::submit`]）。
    pub fn new(render: &rjw_render::RenderContext) -> Self {
        let gpu = render.gpu_arc().clone();
        let device = gpu.device();
        let queue = gpu.queue();
        let surface_format = render.format();
        let depth_format = render.depth_format();

        // 纹理 / VP bind group layout 由 `Gpu` 统一提供（子系统间可复用 bind group）。
        let vp_bl = gpu.vp_layout().clone();
        let tex_bl = gpu.texture_layout().clone();
        // 注册表：本上下文的共享句柄（world / UI 两个渲染器共用同一套）。
        let textures = gpu.texture_registry().clone();
        let meshes = gpu.mesh_registry().clone();

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Render2D: Default Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
        });
        let draw_page = DrawPage::new(
            device,
            &vp_bl,
            &tex_bl,
            shader,
            surface_format,
            depth_format,
            MAX_INSTANCES_PER_DRAW,
        );
        // 注册四边形为静态网格（Sprite 与 StaticMesh 共用实例化绘制路径）。
        let quad_mesh_id = meshes.register(Arc::new(MeshData::from_buffers(
            draw_page.quad_vb.clone(),
            draw_page.quad_ib.clone(),
            QUAD_TRI_INDICIES.len() as u32,
        )));
        let white_texture = Arc::new(TextureWrapped::from_rgba8(
            device,
            queue,
            "Render2D: White Texture",
            &[255, 255, 255, 255],
            1,
            1,
        ));
        textures.register(white_texture.clone());

        // 默认采样器（RStates::default()：线性 + ClampToEdge），samp_key == 0 零开销路径。
        let default_sampler = device.create_sampler(&RStates::default().to_sampler_desc());

        // 视口缓存初始化为单位 MVP 对应的世界矩形（未调用 set_mvp 时 Cull::Viewport 也可用）。
        let mut culler = Culler::new(Cull::Off);
        culler.set_viewport(cull::viewport_world_rect(&glam::Mat4::IDENTITY));

        Self {
            device: device.clone(),
            queue: queue.clone(),
            format: surface_format,
            depth_format,
            tex_bind_group_layout: tex_bl,
            gpu,
            textures,
            meshes,
            white_texture,
            mesh_storage: MeshStorage::default(),
            command_queue: DrawCommandQueue::default(),
            draw_page,
            vp: glam::Mat4::IDENTITY,
            viewport: None,
            scissor: None,
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

    // ── 画面（多视口） / 排序 / 剔除 / 全局状态 ─────────────

    /// 以 2D 相机设定**本画面**：VP（= `cam.vp_matrix()`）与屏幕矩形（= `cam.region`）。
    ///
    /// 这是"一个画面 = 一次 [`Self::submit`]"的准备步骤；[`Self::submit`] 会自动调用它。
    /// 相机**由调用方持有**（`Render2D` 不存储相机）；VP 只在提交那一刻被读取 ⇒
    /// 没有「先设相机还是先改姿态」的隐式顺序契约。
    pub fn camera(&mut self, cam: &Camera2D) -> &mut Self {
        self.vp = cam.vp_matrix();
        self.culler.set_viewport(cull::viewport_world_rect(&self.vp));
        self.viewport = Some(cam.region.normalized());
        self
    }

    /// 设定本画面的屏幕矩形（像素、左上原点；UI 等"屏幕固定"渲染用）。
    ///
    /// 绘制时 `set_viewport` 并把 scissor 同步为该矩形；矩形会按目标尺寸钳制 / 取整。
    pub fn viewport(&mut self, rect: Rect) -> &mut Self {
        self.viewport = Some(rect.normalized());
        self
    }

    /// 恢复默认：全屏、无 scissor、出厂渲染状态（零开销，等价单画面旧行为）。
    pub fn reset(&mut self) -> &mut Self {
        self.viewport = None;
        self.scissor = None;
        self.default_states = RStates::default();
        self.culler = Culler::new(Cull::Off);
        self
    }

    /// 覆盖 scissor 矩形（`None` = 跟随 [`Self::viewport`]）。
    pub fn scissor(&mut self, rect: Option<Rect>) -> &mut Self {
        self.scissor = rect.map(|r| r.normalized());
        self
    }

    /// 当前画面矩形（`None` = 全屏）。
    #[inline]
    pub fn current_viewport(&self) -> Option<Rect> {
        self.viewport
    }

    // ── 格式 / 附件需求 ─────────────────────────────────────

    /// 颜色附件格式（= surface 格式，构造期固定）。
    #[inline]
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// 深度 / 模板附件格式（构造期固定；**不进管线缓存 key**）。
    #[inline]
    pub fn depth_format(&self) -> wgpu::TextureFormat {
        self.depth_format
    }

    /// 当前录制的命令里是否有声明 depth / stencil 状态者 ⇒ **需要深度附件**。
    ///
    /// 注：正常路径**不需要**调用方判断——[`PassBuilder`] 在写入时自动推导。
    /// 本方法保留为诊断 / 自定义 pass 组合用。
    #[inline]
    pub fn will_use_depth_stencil(&self) -> bool {
        self.command_queue.requires_depth_stencil()
    }

    /// 命令排序模式（默认 [`SortMode::LayerAndStates`]）。
    ///
    /// - [`SortMode::LayerAndStates`]：按 `(layer, states)` 排序后合批（引擎默认）；
    /// - [`SortMode::LayerOnly`]：仅按 layer 稳定排序（同层保持录制顺序），UI 适用；
    /// - [`SortMode::None`]：完全按录制顺序（相邻同状态仍合批）。
    #[inline]
    pub fn sort(&mut self, mode: SortMode) -> &mut Self {
        self.sort_mode = mode;
        self
    }

    /// 自定义排序策略（覆盖 [`Self::sort`]）。
    #[inline]
    pub fn sort_custom(&mut self, sorter: Box<dyn SortPolicy>) -> &mut Self {
        self.sorter = Some(sorter);
        self
    }

    /// 当前排序模式（设置自定义策略后仍返回最后一次设置的内置模式）。
    #[inline]
    pub fn sort_mode(&self) -> SortMode {
        self.sort_mode
    }

    /// 剔除模式（**单一入口**，无隐式联动）：
    /// [`Cull::Off`] / [`Cull::Viewport`] / [`Cull::Rect`] / [`Cull::Fn`]。
    ///
    /// 以相机剔除：`r2d.cull(Cull::from(&cam))`。
    #[inline]
    pub fn cull(&mut self, cull: impl Into<Cull>) -> &mut Self {
        self.culler.set(cull.into());
        self
    }

    /// 当前剔除模式。
    #[inline]
    pub fn cull_mode(&self) -> &Cull {
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
    /// `r2d.states(RStates::new().blend(Additive).depth_test(true))`。
    #[inline]
    pub fn states_mut(&mut self, states: RStates) -> &mut Self {
        self.default_states = states;
        self
    }

    // ── 资源（低层逃生口；happy path 一律走 `Gfx::{texture,mesh}`） ──

    /// 纹理 bind group layout（自建 bind group 的下游用：`rjw_text` / `rjw_ui` 等）。
    ///
    /// 与 `Gfx::texture_layout()` 是同一份 layout（同一设备 → 可互相复用 bind group）。
    #[inline]
    pub fn texture_layout(&self) -> &wgpu::BindGroupLayout {
        &self.tex_bind_group_layout
    }

    /// 1×1 白色纹理（纯色绘制用；`solid` 内部即用它）。
    #[inline]
    pub fn white_texture(&self) -> &ArcTextureWrapped {
        &self.white_texture
    }

    /// **本上下文的纹理注册表**（绘制期唯一解析点）。
    ///
    /// `rjw_text` / `rjw_ui` / `rjw_tilemap` 等下游在绘制时经此解析 `tex_uid`，
    /// 不再触碰任何进程级全局表。
    #[inline]
    pub fn textures(&self) -> &TextureRegistry {
        &self.textures
    }

    /// **本上下文的静态网格注册表**（绘制期唯一解析点）。
    #[inline]
    pub fn meshes(&self) -> &MeshRegistry {
        &self.meshes
    }

    /// 本上下文的能力对象（`device` / `queue` / `texture_layout` / 注册表）。
    ///
    /// 供下游在**运行时**惰性创建资源——这些资源的创建时机晚于 `App::init`，
    /// 拿不到 `Gfx`（例如自建纹理 / 网格、或 `rjw_text::Text::white_region` 之外
    /// 需要 `&Gpu` 的构造器）。
    #[inline]
    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    /// 设备（低层逃生口：自建缓冲 / 管线）。
    #[inline]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// 队列（低层逃生口）。
    #[inline]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// 当前 VP 矩阵（本画面的；未设相机时为 [`glam::Mat4::IDENTITY`]）。
    #[inline]
    pub fn current_vp(&self) -> glam::Mat4 {
        self.vp
    }

    // ── 录制入口（画什么）：数据版 ──────────────────────────

    /// 贴纹理精灵：`r2d.sprite(rect, &tex).tint(..).transform(..).layer(..)`。
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

    /// 纯色精灵（内部用 1×1 白纹理）：`r2d.solid(rect).tint(..)`。
    #[inline]
    pub fn solid(&mut self, rect: impl Into<SpriteRect>) -> SpriteBuilder<'_> {
        let uid = self.white_texture.uid;
        Draw2D::sprite(&mut self.command_queue, &mut self.mesh_storage, rect.into(), uid)
    }

    /// **图集直达**：把 `DynamicAtlas::sprite(&handle)` 的产物一次提交。
    ///
    /// 取代「`TEXTURES.get(page_uid)` + 手算像素→归一化 UV + `SpriteRect::with_uv_tex`」三步：
    ///
    /// ```ignore
    /// if let Some(spr) = atlas.sprite(&handle) {
    ///     f.draw().region(spr).tint(Color::WHITE).layer(0.0);
    /// }
    /// ```
    ///
    /// 初始 mesh 左上角在原点、尺寸 = 图集区域尺寸；用 `.at(..)` / `.transform(..)` 摆位。
    ///
    /// 需要 `rjw_atlas` feature（`AtlasSprite` 来自该 crate）；关闭后本入口不存在，
    /// 其余 2D 绘制能力不受影响。
    #[cfg(feature = "rjw_atlas")]
    pub fn region(&mut self, sprite: AtlasSprite) -> SpriteBuilder<'_> {
        let region = sprite.region;
        let tex_w = (sprite.texture.width as f32).max(1.0);
        let tex_h = (sprite.texture.height as f32).max(1.0);
        let (uw, uh) = (region.wh_px.0 as f32, region.wh_px.1 as f32);
        let rect = SpriteRect {
            mesh_tl: glam::Vec2::ZERO,
            mesh_wh: glam::Vec2::new(uw, uh),
            uv_tl: glam::Vec2::new(region.tl_px.0 as f32 / tex_w, region.tl_px.1 as f32 / tex_h),
            uv_wh: glam::Vec2::new(uw / tex_w, uh / tex_h),
        };
        Draw2D::sprite(
            &mut self.command_queue,
            &mut self.mesh_storage,
            rect,
            sprite.texture.uid,
        )
    }

    /// 四边形段（顶点由调用者提供，顺序 **TL, TR, BL, BR**）：
    /// `r2d.quads(&verts, &tex).transform(tf).tint(tint).layer(l)`。
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

    /// **顶点 + 显式三角形索引段**（整段实例色，与 [`Self::quads`] 同语义）：
    /// `r2d.mesh_indexed(&verts, &tris, &tex).transform(tf).tint(c).layer(l)`。
    ///
    /// 供调用方**自行三角化**的几何使用（例如 UI 的圆角 + 羽化镶嵌：顶点自带
    /// 四角渐变色与羽化 alpha）。与 [`Self::mesh_with`] 的区别在 **`tint` 的作用位置**：
    /// 本方法把 `tint` 作为**整段实例色**（顶点色 × tint），`mesh_with` 则把它写进
    /// **逐顶点色**（会覆盖顶点自带色）。需要「保留逐顶点渐变 / 羽化 alpha，同时整段
    /// 染色」时用本方法。
    #[inline]
    pub fn mesh_indexed(
        &mut self,
        vertices: &[VertexP3U2C4],
        indices: &[[u16; 3]],
        texture: &ArcTextureWrapped,
    ) -> MeshBuilder<'_> {
        Draw2D::mesh_indexed(
            &mut self.command_queue,
            &mut self.mesh_storage,
            vertices,
            indices,
            texture.uid,
        )
    }

    // ── 录制入口（画什么）：网格 / 多边形 / 流式 ────────────

    /// 显式顶点 + 三角形索引（世界坐标；默认白纹理）：
    /// `r2d.mesh(&verts, &tris).tint(..).transform(..).layer(..)`。
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

    /// 多边形（**fan 三角化**：首顶点为中心）：`r2d.polygon(&verts).tint(..).layer(..)`。
    ///
    /// 需要 UV 的多边形用 [`Self::polygon_with`]（`p.vertex_uv(..)`）。
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
    /// `r2d.static_mesh(id, &tex).tint(..).transform(tf).layer(..)`。
    ///
    /// `id` 由 `Gfx::mesh(..)`（或 `MESHES.register`）产出——类型化句柄，不再收裸 `u64`。
    #[inline]
    pub fn static_mesh(
        &mut self,
        mesh_id: MeshId,
        texture: &ArcTextureWrapped,
    ) -> StaticMeshBuilder<'_> {
        debug_assert!(
            self.meshes.contains_uid(mesh_id.uid()),
            "mesh {mesh_id:?} is not registered in this context's mesh registry"
        );
        Draw2D::static_mesh(
            &mut self.command_queue,
            &mut self.mesh_storage,
            mesh_id.uid(),
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

    // ── 录制入口（画什么）：调试图元 ────────────────────────

    /// 调试图元：`r2d.debug(DebugStyle::new(Color::RED).width(2.0)).line(a, b);`
    ///
    /// 返回的 [`DebugPainter`] 借用本渲染器（独占），其上一个 `debug(..)` 段内的图元
    /// 共享同一份 [`DebugStyle`]（颜色 / 线宽 / 层级 / 圆分段数）——同一风格的连续图元
    /// 只写一次样式。几何与层级语义见 [`crate::debug_draw`]。
    #[inline]
    pub fn debug(&mut self, style: impl Into<DebugStyle>) -> DebugPainter<'_> {
        DebugPainter::new(self, style.into())
    }

    // ── 提交（何时画） ─────────────────────────────────────

    /// 清空已录制内容（命令队列 / 动态网格 / 外部绘制句柄）。
    ///
    /// 所有出口（[`Self::render`] / [`Self::submit`]）都以此收尾，契约一致：
    /// **提交即清帧**（下一次 `submit` 从空队列开始 = 下一个画面）。
    #[inline]
    fn clear_frame(&mut self) {
        self.command_queue.clear();
        self.mesh_storage.clear();
        self.buf_custom_draws.clear();
    }

    /// **丢弃**未提交的录制（命令队列 / 动态网格 / 外部绘制句柄）。
    ///
    /// 用于「无帧帧」或应用主动放弃本帧渲染时：录制不会跨帧累积。
    #[inline]
    pub fn discard(&mut self) {
        self.clear_frame();
    }

    /// 全流程一行糖（单画面）：取帧 → 用当前相机提交 → present。
    ///
    /// 取帧失败（最小化 / 遮挡 / 超时 / 丢失）时仅清帧并返回，不 panic。
    /// 相机由 [`Self::camera`] 预先设定（未设 = 单位 VP + 全屏）。
    pub fn render(
        &mut self,
        render: &mut rjw_render::RenderContext,
        clear: impl Into<rjw_render::Clear>,
    ) -> &mut Self {
        let clear = clear.into();
        let Some(mut frame) = render.acquire_frame() else {
            self.clear_frame();
            return self;
        };
        let vp = self.view_projection();
        {
            let mut pass = frame.pass(clear);
            let _ = vp; // VP 由 PassRecorder 契约提供
            pass.record(self);
        }
        frame.present();
        self
    }

    /// 把当前队列提交为**一个画面**（= 一个 pass，使用 `cam` 的 VP 与屏幕矩形），随后清帧。
    ///
    /// 多画面 = 多次调用（各自 `cam`），每次一个独立的 VP 槽 ⇒ 互不干扰（修 B1）。
    pub fn submit<'f>(&'f mut self, pass: &mut PassBuilder<'f>, cam: &Camera2D) {
        let _ = self.camera(cam);
        pass.record(self);
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
                    let Some(mesh) = self.resolve_mesh(*mesh_id) else { continue };
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
                    let Some(mesh) = self.resolve_mesh(*mesh_id) else { continue };
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
                            self.buf_all_tris.push(TriIndices(
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
                            self.buf_all_tris.push(TriIndices(
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
                self.buf_instances.len().div_ceil(MAX_INSTANCES_PER_DRAW);
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
        // 用户调用 `meshes/textures.remove(uid)` 后，缓存条目在此剔除，
        // 其持有的 Arc<Texture> 与 BindGroup 一并 drop，GPU 资源正确释放。
        if !self.tex_bind_group_cache.is_empty() {
            self.tex_bind_group_cache
                .retain(|&(tex_uid, _), _| self.textures.contains_uid(tex_uid));
        }
    }

    /// 采样器位域取出（RStates bits 8..24，与 rstates.rs 的采样器域一致）。
    const SAMPLER_KEY_MASK: u64 = 0xFF_FF00;

    /// 解析**本上下文**注册表里的静态网格。
    ///
    /// 返回 `None` 表示该 uid 不在本上下文的网格注册表里（未注册 / 已被 `remove` /
    /// 属于另一个 `RenderContext`）。调用方**跳过该命令并告警**，而不是在 pass 中途
    /// panic —— `prepare` 已经容忍移除（见上方 bind group 缓存清理），两半行为对齐。
    #[inline]
    fn resolve_mesh(&self, mesh_id: u64) -> Option<Arc<MeshData>> {
        let mesh = self.meshes.get(mesh_id);
        if mesh.is_none() {
            log::warn!(
                "Render2D: 网格 uid {mesh_id} 不在本上下文注册表中，跳过该绘制命令\
                 （未注册 / 已被 remove / 属于另一个 RenderContext）"
            );
        }
        mesh
    }

    /// 解析纹理 uid → `Arc<TextureWrapped>`（`None` 使用白纹理）。
    ///
    /// 解析不到时**告警并回退到白纹理**（而非 panic）：与 `resolve_mesh` 同一策略，
    /// 使「纹理被注销后仍被绘制」不再在 pass 中途崩溃。
    fn resolve_tex(&self, tex_uid: Option<u64>) -> ArcTextureWrapped {
        match tex_uid {
            Some(uid) => self.textures.get(uid).unwrap_or_else(|| {
                log::warn!(
                    "Render2D: 纹理 uid {uid} 不在本上下文注册表中，回退白纹理\
                     （未注册 / 已被 remove / 属于另一个 RenderContext）"
                );
                self.white_texture.clone()
            }),
            None => self.white_texture.clone(),
        }
    }

    /// 绑定 group(1) 纹理 bind group（纹理 + 采样器缓存复用）。
    /// bind group 缓存持有 `Arc<Texture>` —— 纹理被 `textures.remove` 后由 prepare 末尾清理，资源正确释放。
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

    /// 应用本画面的 viewport / scissor（像素、左上原点；按目标尺寸钳制并取整）。
    ///
    /// - `viewport = None`（默认 / [`Render2D::reset`]）⇒ 不调用，wgpu 默认即全目标
    ///   （与单画面旧行为逐位等价、零开销）；
    /// - scissor 默认与视口一致：旋转相机 / 大几何画出矩形之外时，防止画面互相串味。
    fn apply_viewport(&self, pass: &mut wgpu::RenderPass<'_>, target_size: (u32, u32)) {
        let Some(v) = self.viewport else {
            return;
        };
        let Some((vp, sc)) = clamp_viewport(v, self.scissor, target_size) else {
            return;
        };
        pass.set_viewport(vp.0, vp.1, vp.2, vp.3, 0.0, 1.0);
        pass.set_scissor_rect(sc.0, sc.1, sc.2, sc.3);
    }

    /// 把 `prepare()` 的合批结果写进 pass。
    ///
    /// `ctx` 提供 pass 目标尺寸（视口钳制用）与**本 recorder 专属的 VP 槽**
    /// （bind group + 动态偏移；一帧内多画面互不干扰）。
    fn draw(&mut self, pass: &mut wgpu::RenderPass<'_>, ctx: PassContext<'_>) {
        let target_size = ctx.target_size;
        self.apply_viewport(pass, target_size);
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
                    if count != 0
                        && let Some(mesh) = self.resolve_mesh(mesh_id)
                    {
                        let pipeline = self.draw_page.get_or_create_pipeline(
                            &self.device,
                            rstates,
                            self.depth_format,
                            ctx.has_depth_stencil,
                        );
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, ctx.vp_bind_group, &[ctx.vp_offset]);
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
                        let pipeline = self.draw_page.get_or_create_pipeline(
                            &self.device,
                            rstates,
                            self.depth_format,
                            ctx.has_depth_stencil,
                        );
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, ctx.vp_bind_group, &[ctx.vp_offset]);
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

}

// ─── PassRecorder：把队列录进任意 pass（同 pass 多次提交的支持点）──

impl PassRecorder for Render2D {
    #[inline]
    fn uses_depth_stencil(&self) -> bool {
        self.will_use_depth_stencil()
    }

    #[inline]
    fn view_projection(&self) -> rjw_render::Matrix4x4 {
        self.vp.to_cols_array_2d()
    }

    fn record_into(&mut self, pass: &mut wgpu::RenderPass<'_>, ctx: PassContext<'_>) {
        self.prepare();
        self.draw(pass, ctx);
        self.clear_frame();
    }
}

// ─── 视口换算（纯函数：便于单测） ──────────────────────────────

/// 把画面矩形换算成 `set_viewport` / `set_scissor_rect` 的参数：
/// 按目标尺寸钳制 + 取整；矩形与目标不相交（宽或高 < 1px）时返回 `None`（不设置）。
///
/// `(viewport, scissor)` 的 wgpu 原始形态：`viewport = (x, y, w, h)` 浮点、
/// `scissor = (x, y, w, h)` 无符号整数。
pub(crate) type ViewportScissor = ((f32, f32, f32, f32), (u32, u32, u32, u32));

/// 把画面矩形与裁剪矩形夹到渲染目标范围内（含 DPI / 越界钳制 / 空矩形判定）。
///
/// 返回 `(viewport(x, y, w, h), scissor(x, y, w, h))`。
pub(crate) fn clamp_viewport(
    viewport: Rect,
    scissor: Option<Rect>,
    target_size: (u32, u32),
) -> Option<ViewportScissor> {
    let (tw, th) = (target_size.0 as f32, target_size.1 as f32);
    let v = viewport.normalized();
    let x = v.x.clamp(0.0, tw);
    let y = v.y.clamp(0.0, th);
    let w = v.w.clamp(0.0, tw - x);
    let h = v.h.clamp(0.0, th - y);
    if w < 1.0 || h < 1.0 {
        return None;
    }
    let sc = scissor.unwrap_or(Rect::new(x, y, w, h)).normalized();
    let sx = sc.x.clamp(0.0, tw).round() as u32;
    let sy = sc.y.clamp(0.0, th).round() as u32;
    let sw = sc.w.clamp(0.0, tw - sx as f32).round().max(1.0) as u32;
    let sh = sc.h.clamp(0.0, th - sy as f32).round().max(1.0) as u32;
    Some(((x, y, w, h), (sx, sy, sw, sh)))
}

#[cfg(test)]
mod viewport_tests {
    use super::*;

    #[test]
    fn splits_screen_without_clamping() {
        // 1280×720 的左半屏 / 右半屏：视口与 scissor 一致、数值不变。
        let l = clamp_viewport(Rect::new(0.0, 0.0, 640.0, 720.0), None, (1280, 720)).unwrap();
        assert_eq!(l.0, (0.0, 0.0, 640.0, 720.0));
        assert_eq!(l.1, (0, 0, 640, 720));
        let r = clamp_viewport(Rect::new(640.0, 0.0, 640.0, 720.0), None, (1280, 720)).unwrap();
        assert_eq!(r.0, (640.0, 0.0, 640.0, 720.0));
        assert_eq!(r.1, (640, 0, 640, 720));
    }

    #[test]
    fn clamps_to_target_bounds() {
        // 左上越界：原点钳到 (0,0)，尺寸保留（在目标内时不缩小）。
        let c = clamp_viewport(Rect::new(-40.0, -10.0, 400.0, 300.0), None, (1280, 720)).unwrap();
        assert_eq!(c.0, (0.0, 0.0, 400.0, 300.0));
        // 右下越界：尺寸被钳到剩余空间（80×20）。
        let f = clamp_viewport(Rect::new(1200.0, 700.0, 400.0, 400.0), None, (1280, 720)).unwrap();
        assert_eq!(f.0, (1200.0, 700.0, 80.0, 20.0));
    }

    #[test]
    fn none_when_fully_outside_or_degenerate() {
        assert!(clamp_viewport(Rect::new(2000.0, 0.0, 100.0, 100.0), None, (1280, 720)).is_none());
        assert!(clamp_viewport(Rect::new(0.0, 0.0, 0.5, 0.5), None, (1280, 720)).is_none());
    }

    #[test]
    fn negative_size_is_normalized() {
        let n = clamp_viewport(Rect::new(100.0, 100.0, -40.0, -30.0), None, (1280, 720)).unwrap();
        assert_eq!(n.0, (60.0, 70.0, 40.0, 30.0));
    }

    #[test]
    fn scissor_override_wins() {
        let (vp, sc) = clamp_viewport(
            Rect::new(0.0, 0.0, 640.0, 720.0),
            Some(Rect::new(16.0, 16.0, 200.0, 100.0)),
            (1280, 720),
        )
        .unwrap();
        assert_eq!(vp, (0.0, 0.0, 640.0, 720.0));
        assert_eq!(sc, (16, 16, 200, 100));
    }
}
