//! 帧与 pass 生命周期：`RenderFrame`（帧级资源 / 附件 / 提交）+ `PassBuilder`（开 pass）。
//!
//! # 责任划分
//!
//! | 状态 | 归属 |
//! |---|---|
//! | surface 纹理、`CommandEncoder`、`present`、**帧级深度/模板附件** | [`RenderFrame`] |
//! | pass 边界（颜色 / 深度 / 模板的 `Load` / `Clear`）、同 pass 多次提交 | [`PassBuilder`] |
//! | 管线 / 视口 / scissor / 排序 / 合批 | 上层渲染器（实现 [`PassRecorder`]，如 `rjw_2d_render::Render2D`） |
//!
//! # 用法
//!
//! ```ignore
//! let Some(frame) = ctx.acquire_frame() else { return };   // 取不到 ⇒ 跳过本帧
//!
//! // ① 一画面一 pass：世界 + UI 同 pass（一次 Load/Store），深度附件需求自动推导
//! let mut pass = frame.pass(Clear::color(bg));
//! pass.record(&mut r2d_world);
//! pass.record(&mut r2d_ui);
//! // pass 在这里 Drop ⇒ 开 pass、写入、关闭
//!
//! frame.present();
//! ```
//!
//! **深度附件需求不再由调用方手算**：`PassBuilder` 收集全部 recorder，随后用
//! `recorder.uses_depth_stencil() || clear.uses_depth_stencil()` 统一推导，因此旧实现里
//! 「忘记先查 need ⇒ `PassScope::record` panic」的坑消失。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use crate::format::{has_depth_aspect, has_stencil_aspect};
use crate::Clear;

// ─── 帧级 VP 动态偏移环 ───────────────────────────────────────

/// VP 缓冲默认槽位数（每个 pass 一个槽；超出时自动扩容）。
const DEFAULT_VP_SLOTS: usize = 16;

/// 列主序 4×4 VP 矩阵（避免给 `rjw_render` 引入 glam 依赖）。
pub type Matrix4x4 = [[f32; 4]; 4];

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct VpUniform {
    vp: Matrix4x4,
}

/// **帧级** VP 槽环：每个 pass 占一个 256 字节对齐的槽，绑定各自的动态偏移。
///
/// 动机（修 B1）：旧实现只有一个 offset-0 的 VP uniform + 一个 bind group，
/// `set_mvp` 在 CPU 端立即 `write_buffer`，而所有 pass 直到 `present` 才进同一个
/// submit ⇒ 一帧内多个画面**全部读到最后一个矩阵**。槽环让每个 pass 有独立数据。
pub(crate) struct VpRing {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    capacity: usize,
    align: u32,
}

impl VpRing {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout) -> Self {
        let align = device.limits().min_uniform_buffer_offset_alignment.max(1);
        let (buffer, bind_group) = Self::alloc(device, queue, layout, align, DEFAULT_VP_SLOTS);
        Self { buffer, bind_group, capacity: DEFAULT_VP_SLOTS, align }
    }

    fn alloc(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        align: u32,
        capacity: usize,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let size = align as u64 * capacity as u64;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("krusie: frame VP slots"),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("krusie: frame VP bind group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    // 动态偏移由 `set_bind_group(.., &[offset])` 提供。
                    size: std::num::NonZeroU64::new(std::mem::size_of::<VpUniform>() as u64),
                }),
            }],
        });
        (buffer, bind_group)
    }

    /// 写入第 `slot` 个槽并返回其动态偏移（像素级 O(1)）。
    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, slot: usize, vp: Matrix4x4) -> u32 {
        if slot >= self.capacity {
            let capacity = (slot + 1).next_power_of_two();
            let (buffer, bind_group) = Self::alloc(device, queue, layout, self.align, capacity);
            self.buffer = buffer;
            self.bind_group = bind_group;
            self.capacity = capacity;
        }
        let offset = slot as u32 * self.align;
        queue.write_buffer(&self.buffer, offset as u64, bytemuck::bytes_of(&VpUniform { vp }));
        offset
    }

    #[inline]
    fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }
}

// ─── 帧级深度附件（按 设备 + 尺寸 + 格式 池化） ───────────────

/// 帧级深度 / 模板附件：同一个帧内多个渲染器、多个画面**共享同一份**。
pub struct DepthTarget {
    view: wgpu::TextureView,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    /// 新创建（内容未初始化）⇒ 首个使用它的 pass 强制清理，避免读到未初始化深度。
    fresh: AtomicBool,
}

impl DepthTarget {
    /// 附件视图。
    #[inline]
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// 附件尺寸（像素）。
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// 附件格式。
    #[inline]
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// 取出"未初始化"标记：`true` **只会返回一次**（首个绑定它的 pass）。
    #[inline]
    pub fn take_fresh(&self) -> bool {
        self.fresh.swap(false, Ordering::Relaxed)
    }
}

/// 全局深度附件池的 key：`(device_key, w, h, format)`。
type DepthTargetKey = (usize, u32, u32, wgpu::TextureFormat);

/// 全局深度附件池：key = [`DepthTargetKey`]。
///
/// `device_key` 必须参与 key（旧实现只有 `(w,h,format)` ⇒ 多 `RenderContext` / 多设备时
/// 会跨设备复用同一附件）。
static DEPTH_TARGETS: LazyLock<Mutex<HashMap<DepthTargetKey, Arc<DepthTarget>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn depth_target(
    device: &wgpu::Device,
    device_key: usize,
    size: (u32, u32),
    format: wgpu::TextureFormat,
) -> Arc<DepthTarget> {
    let (w, h) = (size.0.max(1), size.1.max(1));
    let key = (device_key, w, h, format);
    let mut pool = DEPTH_TARGETS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(t) = pool.get(&key) {
        return t.clone();
    }
    // 同设备同格式只保留最新尺寸（窗口缩放不会让池无限增长）。
    pool.retain(|&(dev, _, _, f), _| dev != device_key || f != format);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("krusie: frame depth-stencil"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target = Arc::new(DepthTarget {
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        size: (w, h),
        format,
        fresh: AtomicBool::new(true),
    });
    pool.insert(key, target.clone());
    target
}

// ─── 取帧抽象 ─────────────────────────────────────────────────

/// 帧源：一次提供一帧。
///
/// 抽出来是为了让「取不到表面」可注入、可测试（[`Never`]），也让运行时不必关心
/// 具体是 `RenderContext` 还是别的输出目标。
pub trait FrameSource {
    /// 取一帧；`None` = 本帧不可呈现（最小化 / 遮挡 / 超时 / surface 丢失或过期），
    /// 调用方应跳过渲染（但**逻辑仍应继续**，见 `App::update`）。
    fn acquire_frame(&mut self) -> Option<RenderFrame>;
}

/// 永远取不到帧的帧源（测试用：验证「无帧也执行 update」与退避）。
pub struct Never;

impl FrameSource for Never {
    #[inline]
    fn acquire_frame(&mut self) -> Option<RenderFrame> {
        None
    }
}

// ─── PassRecorder ─────────────────────────────────────────────

/// recorder 写入 pass 时需要的上下文。
pub struct PassContext<'a> {
    /// pass 目标尺寸（像素），供渲染器换算视口 / scissor。
    pub target_size: (u32, u32),
    /// 本 pass 是否绑定了深度 / 模板附件。
    ///
    /// 渲染器据此决定管线**必须**声明的附件集合：绑定附件时，即使用不到深度的管线
    /// 也要声明同格式附件（见 `RStates::declared_depth_stencil`），否则 wgpu 会报
    /// 「Render pipeline targets are incompatible with render pass」。
    pub has_depth_stencil: bool,
    /// 帧级 VP bind group（动态偏移槽环）。
    pub vp_bind_group: &'a wgpu::BindGroup,
    /// 本 recorder 专属的 VP 槽动态偏移（绑定 group 0 时传入）。
    pub vp_offset: u32,
}

/// 能把自身录制的绘制命令写入**已打开的 pass**（由渲染器实现）。
pub trait PassRecorder {
    /// 本批命令是否声明了深度 / 模板状态。
    ///
    /// 由 [`PassBuilder`] 统一读取（调用方不再手算，也不再需要提前调用）。
    fn uses_depth_stencil(&self) -> bool;

    /// 本批命令使用的 View-Projection 矩阵（**列主序**）。
    ///
    /// [`PassBuilder`] 为每个 recorder 分配一个独立的 VP 槽并写入该值，
    /// 因此一帧内的多个画面（多个相机）互不干扰。
    fn view_projection(&self) -> Matrix4x4;

    /// 排序 / 剔除 / 合批后写入 pass（**由渲染器负责自己的视口 / 管线 / 绑定**）。
    fn record_into(&mut self, pass: &mut wgpu::RenderPass<'_>, ctx: PassContext<'_>);
}

// ─── 离屏目标 ─────────────────────────────────────────────────

/// pass 目标（颜色视图 + 可选外部深度视图）。
#[derive(Clone, Copy)]
pub struct RenderTarget<'a> {
    /// 颜色附件视图。
    pub view: &'a wgpu::TextureView,
    /// 外部深度 / 模板视图（`None` = 按需使用帧级附件）。
    pub depth: Option<&'a wgpu::TextureView>,
}

impl<'a> RenderTarget<'a> {
    /// 只有颜色附件。
    #[inline]
    pub fn color(view: &'a wgpu::TextureView) -> Self {
        Self { view, depth: None }
    }

    /// 颜色 + 外部深度附件。
    #[inline]
    pub fn color_depth(view: &'a wgpu::TextureView, depth: &'a wgpu::TextureView) -> Self {
        Self { view, depth: Some(depth) }
    }
}

// ─── PassBuilder ──────────────────────────────────────────────

enum PassTarget<'f> {
    /// 帧自身的 surface 颜色附件。
    Frame,
    /// 离屏 / 外部目标。
    External(RenderTarget<'f>),
}

/// 正在构造的一个 pass：收集 recorder，Drop（或 [`Self::end`]）时开 pass 并写入。
///
/// 典型用法：
/// ```ignore
/// let mut pass = frame.pass(Clear::color(bg));
/// pass.record(&mut r2d_world);
/// pass.record(&mut r2d_ui);
/// ```
pub struct PassBuilder<'f> {
    frame: &'f mut RenderFrame,
    clear: Clear,
    target: PassTarget<'f>,
    recorders: Vec<&'f mut dyn PassRecorder>,
    flushed: bool,
}

impl<'f> PassBuilder<'f> {
    /// 收集一个 recorder（可多次调用；顺序 = 提交顺序）。
    #[inline]
    pub fn record<R: PassRecorder + 'f>(&mut self, recorder: &'f mut R) -> &mut Self {
        self.recorders.push(recorder);
        self
    }

    /// 本 pass 覆盖的目标尺寸（像素）。
    #[inline]
    pub fn target_size(&self) -> (u32, u32) {
        match &self.target {
            PassTarget::Frame => self.frame.size(),
            PassTarget::External(t) => {
                let s = t.view.texture().size();
                (s.width, s.height)
            }
        }
    }

    /// 本 pass 是否会绑定深度 / 模板附件（收集 recorder 后推导）。
    pub fn uses_depth_stencil(&self) -> bool {
        self.clear.uses_depth_stencil() || self.recorders.iter().any(|r| r.uses_depth_stencil())
    }

    /// 逃生口：拿到**已打开的**原生 `wgpu::RenderPass` 做自定义绘制。
    ///
    /// `hook` 在本 pass 的附件配置下执行，随后才写入已收集的 recorder（顺序固定）。
    pub fn escape(mut self, hook: impl FnOnce(&mut wgpu::RenderPass<'_>)) {
        let mut hook = Some(hook);
        self.flush_with(&mut |pass| {
            if let Some(hook) = hook.take() {
                hook(pass);
            }
        });
    }

    /// 显式结束（等价 Drop）。
    #[inline]
    pub fn end(mut self) {
        self.flush_with(&mut |_| {});
    }

    fn flush_with(&mut self, hook: &mut dyn FnMut(&mut wgpu::RenderPass<'_>)) {
        if self.flushed {
            return;
        }
        self.flushed = true;

        let Self { frame, clear, target, recorders, .. } = self;
        let RenderFrame {
            encoder,
            color,
            size,
            depth,
            device,
            device_key,
            depth_format,
            format,
            vp_layout,
            vp_ring,
            vp_cursor,
            queue,
            ..
        } = &mut **frame;

        let encoder = encoder.as_mut().expect("RenderFrame: encoder 已被 present 消费");

        // 目标视图 + 尺寸
        let (view, tsize, ext_depth) = match target {
            PassTarget::Frame => (&*color, *size, None),
            PassTarget::External(t) => {
                let es = t.view.texture().size();
                let tsize = (es.width, es.height);
                assert!(
                    t.view.texture().format() == *format,
                    "RenderFrame::pass_to: 目标格式 {:?} 与帧格式 {format:?} 不一致（管线按帧格式烘焙）",
                    t.view.texture().format()
                );
                if let Some(d) = t.depth {
                    assert!(
                        d.texture().format() == *depth_format,
                        "RenderFrame::pass_to: 外部深度格式 {:?} 与帧深度格式 {depth_format:?} 不一致",
                        d.texture().format()
                    );
                }
                (t.view, tsize, t.depth)
            }
        };

        let need = clear.uses_depth_stencil() || recorders.iter().any(|r| r.uses_depth_stencil());
        let (dv, fresh) = if !need {
            (None, false)
        } else if let Some(d) = ext_depth {
            (Some(d), false)
        } else {
            let arc: &Arc<DepthTarget> =
                depth.get_or_insert_with(|| depth_target(device, *device_key, tsize, *depth_format));
            (Some(arc.view()), arc.take_fresh())
        };

        let mut pass = begin_pass_impl(encoder, view, *clear, dv, fresh);
        hook(&mut pass);

        // 每个 recorder = 一个画面 = 一个 VP 槽（B1：不再共享单一 offset-0 uniform）。
        for r in recorders.iter_mut() {
            if vp_ring.is_none() {
                *vp_ring = Some(VpRing::new(device, queue, vp_layout));
            }
            let ring = vp_ring.as_mut().expect("vp ring 刚创建");
            let offset = ring.write(device, queue, vp_layout, *vp_cursor, r.view_projection());
            *vp_cursor += 1;
            let ctx = PassContext {
                target_size: tsize,
                has_depth_stencil: dv.is_some(),
                vp_bind_group: ring.bind_group(),
                vp_offset: offset,
            };
            r.record_into(&mut pass, ctx);
        }
    }
}

impl Drop for PassBuilder<'_> {
    #[inline]
    fn drop(&mut self) {
        self.flush_with(&mut |_| {});
    }
}

// ─── RenderFrame ──────────────────────────────────────────────

/// 一帧：surface 纹理 + `CommandEncoder` + 帧级深度附件 + `present`。
///
/// - 由 [`FrameSource::acquire_frame`]（`RenderContext` 实现）创建；
/// - 帧内可开任意多个 pass（[`Self::pass`] / [`Self::pass_to`]）；
/// - [`Self::escape_encoder`] 为手动编码 / 后处理的逃生口；
/// - [`Self::present`] 提交并呈现；**忘记 present** 会由 `Drop` 记一条 warning（不再静默丢帧）。
pub struct RenderFrame {
    device: Arc<wgpu::Device>,
    /// 设备身份（深度附件池 key 的一部分）。
    device_key: usize,
    queue: Arc<wgpu::Queue>,
    surface_tex: Option<wgpu::SurfaceTexture>,
    encoder: Option<wgpu::CommandEncoder>,
    color: wgpu::TextureView,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    /// 帧级深度附件（惰性创建；同帧多个渲染器 / 画面共享）。
    depth: Option<Arc<DepthTarget>>,
    /// VP bind group layout（由 [`Gpu`] 提供，与渲染器管线布局一致）。
    vp_layout: wgpu::BindGroupLayout,
    /// 帧级 VP 槽环（惰性创建）。
    vp_ring: Option<VpRing>,
    /// 本帧已分配的 VP 槽数（每画面 +1）。
    vp_cursor: usize,
    presented: bool,
}

impl RenderFrame {
    /// 由已取得的 surface 纹理构造帧（[`crate::RenderContext`] 内部使用）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_surface_texture(
        device: Arc<wgpu::Device>,
        device_key: usize,
        queue: Arc<wgpu::Queue>,
        texture: wgpu::SurfaceTexture,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        vp_layout: wgpu::BindGroupLayout,
    ) -> Self {
        let size = texture.texture.size();
        let color = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("krusie: frame encoder"),
        });
        Self {
            device,
            device_key,
            queue,
            surface_tex: Some(texture),
            encoder: Some(encoder),
            color,
            size: (size.width, size.height),
            format,
            depth_format,
            depth: None,
            vp_layout,
            vp_ring: None,
            vp_cursor: 0,
            presented: false,
        }
    }

    /// 帧尺寸（像素）。
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// 颜色附件格式（= surface 格式）。
    #[inline]
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// 深度 / 模板附件格式（构造期固定）。
    #[inline]
    pub fn depth_format(&self) -> wgpu::TextureFormat {
        self.depth_format
    }

    /// 颜色附件视图（surface view）。
    #[inline]
    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.color
    }

    /// 设备（逃生口）。
    #[inline]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// 队列（逃生口）。
    #[inline]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// 帧级深度附件（按需创建；同帧内多次调用返回**同一份**）。
    pub fn depth_target(&mut self) -> Arc<DepthTarget> {
        let (device, key, size, format) = (&self.device, self.device_key, self.size, self.depth_format);
        self.depth
            .get_or_insert_with(|| depth_target(device, key, size, format))
            .clone()
    }

    /// 帧级深度附件视图（按需创建）。
    pub fn depth_view(&mut self) -> &wgpu::TextureView {
        self.depth_target();
        self.depth.as_ref().map(|t| t.view()).expect("depth target 刚创建")
    }

    /// 逃生口：直接使用帧的 `CommandEncoder`（手动编码 / 后处理）。
    ///
    /// 注意：`wgpu::RenderPass` 借自该 encoder，**同一时刻只能有一个 pass**。
    #[inline]
    pub fn escape_encoder(&mut self) -> &mut wgpu::CommandEncoder {
        self.encoder.as_mut().expect("RenderFrame: encoder 已被 present 消费")
    }

    /// 打开一个 pass（目标 = 本帧 surface 颜色附件）。
    ///
    /// 深度 / 模板附件是否绑定由 [`PassBuilder`] 在写入时**自动推导**。
    #[inline]
    pub fn pass(&mut self, clear: impl Into<Clear>) -> PassBuilder<'_> {
        PassBuilder {
            frame: self,
            clear: clear.into(),
            target: PassTarget::Frame,
            recorders: Vec::new(),
            flushed: false,
        }
    }

    /// 打开一个 pass 渲染到**任意目标视图**（离屏 / 外部深度）。
    #[inline]
    pub fn pass_to<'a>(&'a mut self, target: RenderTarget<'a>, clear: impl Into<Clear>) -> PassBuilder<'a> {
        PassBuilder {
            frame: self,
            clear: clear.into(),
            target: PassTarget::External(target),
            recorders: Vec::new(),
            flushed: false,
        }
    }

    /// 提交编码结果并呈现本帧（无 surface 时仅提交）。
    pub fn present(mut self) {
        let encoder = self.encoder.take().expect("RenderFrame: encoder 已被 present 消费");
        let queue = self.queue.clone();
        queue.submit(std::iter::once(encoder.finish()));
        if let Some(tex) = self.surface_tex.take() {
            queue.present(tex);
        }
        self.presented = true;
    }
}

impl Drop for RenderFrame {
    fn drop(&mut self) {
        if !self.presented {
            log::warn!(
                "RenderFrame 未 present 就被丢弃：本帧不会提交/呈现（记得调用 `frame.present()`）"
            );
        }
    }
}

// ─── 内部：统一构造 pass ─────────────────────────────────────

fn begin_pass_impl<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    color: &'a wgpu::TextureView,
    clear: Clear,
    depth: Option<&'a wgpu::TextureView>,
    depth_fresh: bool,
) -> wgpu::RenderPass<'a> {
    let depth_stencil_attachment = depth.map(|view| {
        let fmt = view.texture().format();
        assert!(
            has_depth_aspect(fmt) || !clear.uses_depth(),
            "深度附件格式 {fmt:?} 无深度切面，但 Clear 要求清深度"
        );
        assert!(
            has_stencil_aspect(fmt) || !clear.uses_stencil(),
            "深度附件格式 {fmt:?} 无模板切面，但 Clear 要求清模板"
        );
        wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: if has_depth_aspect(fmt) {
                Some(ops(clear.depth_value(), depth_fresh, 1.0))
            } else {
                None
            },
            stencil_ops: if has_stencil_aspect(fmt) {
                Some(ops(clear.stencil_value(), depth_fresh, 0))
            } else {
                None
            },
        }
    });

    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("krusie: pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: color,
            depth_slice: None,
            resolve_target: None,
            ops: ops(clear.color_value().map(Into::into), false, wgpu::Color::BLACK),
        })],
        depth_stencil_attachment,
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    })
}

/// 附件操作：`Some` = `Clear(值)`；`None` = `Load`（保留旧内容），
/// 但附件为**新建**（`fresh`，内容未初始化）时回退为 `Clear(fallback)`。
#[inline]
fn ops<T: Copy>(clear: Option<T>, fresh: bool, fallback: T) -> wgpu::Operations<T> {
    wgpu::Operations {
        load: match clear {
            Some(v) => wgpu::LoadOp::Clear(v),
            None if fresh => wgpu::LoadOp::Clear(fallback),
            None => wgpu::LoadOp::Load,
        },
        store: wgpu::StoreOp::Store,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Clear;

    /// 不依赖 GPU 的 recorder：验证 need 推导与 VP 契约。
    struct StubRecorder {
        needs_depth: bool,
        calls: usize,
    }

    impl PassRecorder for StubRecorder {
        fn uses_depth_stencil(&self) -> bool {
            self.needs_depth
        }
        fn view_projection(&self) -> Matrix4x4 {
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        }
        fn record_into(&mut self, _pass: &mut wgpu::RenderPass<'_>, _ctx: PassContext<'_>) {
            self.calls += 1;
        }
    }

    #[test]
    fn format_aspects_are_correct() {
        use wgpu::TextureFormat as F;
        assert!(has_depth_aspect(F::Depth24PlusStencil8));
        assert!(has_stencil_aspect(F::Depth24PlusStencil8));
        assert!(has_depth_aspect(F::Depth32Float));
        assert!(!has_stencil_aspect(F::Depth32Float), "Depth32Float 无模板切面");
        assert!(has_stencil_aspect(F::Stencil8));
        assert!(!has_depth_aspect(F::Stencil8), "Stencil8 无深度切面");
        assert!(!has_depth_aspect(F::Rgba8UnormSrgb));
        assert!(!has_stencil_aspect(F::Rgba8UnormSrgb));
    }

    #[test]
    fn clear_attachment_need_is_derived_from_value_or_recorder() {
        // Clear 本身决定 need
        assert!(Clear::color_depth(crate::ColorF64::BLACK, 1.0).uses_depth_stencil());
        assert!(Clear::depth(1.0).uses_depth_stencil());
        assert!(Clear::stencil(0).uses_depth_stencil());
        assert!(!Clear::default().uses_depth_stencil());
        assert!(!Clear::Keep.uses_depth_stencil());

        // recorder 决定 need（两个 recorder 取或）
        let a = StubRecorder { needs_depth: false, calls: 0 };
        let b = StubRecorder { needs_depth: true, calls: 0 };
        assert!(!a.uses_depth_stencil());
        assert!(b.uses_depth_stencil());
    }

    #[test]
    fn render_target_constructors() {
        // 仅验证构造签名（真实视图需 GPU）
        fn _assert_sig(view: &wgpu::TextureView, depth: &wgpu::TextureView) {
            let _ = RenderTarget::color(view);
            let _ = RenderTarget::color_depth(view, depth);
        }
    }
}
