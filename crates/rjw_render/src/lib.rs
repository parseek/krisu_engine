//! `rjw_render` —— 底层渲染上下文：surface / device / queue 管理 + 纹理 / 静态网格 / 全局注册表。
//!
//! # 分层（见 `docs/API_DESIGN.md`）
//!
//! | 类型 | 责任 |
//! |---|---|
//! | [`RenderContext`] | 拥有 surface / device / queue / swapchain；**唯一取帧入口**（[`FrameSource`]） |
//! | [`Gpu`] | 能力对象：`device` + `queue` + 纹理 layout + 资源工厂（收成一个参数，见 R5） |
//! | [`RenderFrame`] | 一帧：surface 纹理 + encoder + 帧级深度附件 + `present` |
//! | [`PassBuilder`] | 一个 pass：收集 recorder，自动推导深度附件需求 |
//! | [`Clear`] | 一次 pass 的清理意图（取代三 `Option` 的 `ClearConfig`） |
//! | [`TextureWrapped`] / [`MeshData`] | 资源；句柄 [`ArcTextureWrapped`] / [`MeshId`] |
//!
//! # 最小用法
//!
//! ```ignore
//! let ctx = unsafe { RenderContext::new(window, &RenderConfig::default()) };  // 运行时内部代劳
//! let mut r2d = Render2D::new(&ctx);
//! let Some(frame) = ctx.acquire_frame() else { return };
//! let mut pass = frame.pass(Clear::color(Color::BLACK));
//! pass.record(&mut r2d);
//! frame.present();
//! ```

pub mod clear;
pub mod format;
pub mod frame;
pub mod gpu;
pub mod mesh;
pub mod registry;
pub mod texture;

pub use clear::Clear;
pub use frame::{
    DepthTarget, FrameSource, Matrix4x4, Never, PassBuilder, PassContext, PassRecorder, RenderFrame,
    RenderTarget,
};
pub use format::{has_depth_aspect, has_stencil_aspect};
pub use gpu::{Gpu, Rgba8};
pub use mesh::{MeshData, MeshId, MeshRegistry, MeshSpec};
pub use registry::{HasUid, TypedRegistry};
pub use texture::{ArcTextureWrapped, TextureRegistry, TextureWrapped};

// Re-export wgpu in case of version mismatch.
pub use wgpu;

pub use rjw_color::{Color, ColorF64};

use std::sync::Arc;

use winit::window::Window;

/// 默认深度 / 模板附件格式（[`RenderConfig::depth_format`] 的默认值）。
///
/// **帧级常量**：构造期固定（`RenderConfig` → `RenderContext` → `RenderFrame`），
/// 运行期不切换 ⇒ **不进管线缓存 key**。
pub const DEFAULT_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

/// [`RenderContext::new`] 的初始化错误。
///
/// 这些都是**运行期可恢复的失败**（机器没有可用 GPU、驱动缺失、surface 创建不出来），
/// 因此返回 `Result` 而不是 panic —— 调用方可以降级、重试或给出可读提示。
#[derive(Debug, thiserror::Error)]
pub enum RenderInitError {
    /// `instance.create_surface` 失败（窗口与后端不兼容 / 驱动问题）。
    #[error("创建 surface 失败：{0}")]
    CreateSurface(String),
    /// 找不到满足要求的适配器（无 GPU / 驱动未安装 / 后端不匹配）。
    #[error("找不到可用适配器：{0}")]
    NoAdapter(String),
    /// 适配器无法创建逻辑设备（显存 / 驱动限制）。
    #[error("创建设备失败：{0}")]
    CreateDevice(String),
    /// surface 没有报告任何可用格式。
    #[error("surface 未提供任何可用格式")]
    NoSurfaceFormat,
}

/// 垂直同步策略（取代 `vsync: bool`，见 R2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Vsync {
    /// 开启（`PresentMode::AutoVsync`）。
    #[default]
    On,
    /// 关闭（`PresentMode::AutoNoVsync`）。
    Off,
}

impl Vsync {
    #[inline]
    pub const fn is_on(self) -> bool {
        matches!(self, Vsync::On)
    }
}

/// 渲染上下文构造配置。
#[derive(Clone, Debug)]
pub struct RenderConfig {
    /// 尝试使用的 wgpu 后端。
    pub backends: wgpu::Backends,
    /// 垂直同步策略。
    pub vsync: Vsync,
    /// 期望的 surface 格式；`None` = 用适配器的首选格式。
    pub desired_format: Option<wgpu::TextureFormat>,
    /// 深度 / 模板附件格式（帧级常量，默认 [`DEFAULT_DEPTH_FORMAT`]）。
    pub depth_format: wgpu::TextureFormat,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            backends: if cfg!(target_os = "windows") {
                wgpu::Backends::DX12 | wgpu::Backends::GL
            } else {
                wgpu::Backends::all()
            },
            vsync: Vsync::On,
            desired_format: None,
            depth_format: DEFAULT_DEPTH_FORMAT,
        }
    }
}

/// 基于 wgpu 的渲染上下文：管理 surface / device / queue / swapchain。
///
/// **它是取帧的唯一权威**（surface 只在这里）：[`FrameSource::acquire_frame`]。
pub struct RenderContext {
    surface: wgpu::Surface<'static>,
    /// GPU 能力对象 + **资源注册表**（纹理 / 网格）。经 `Arc` 共享，使 `Render2D` /
    /// `rjw_atlas` / `rjw_text` 等持有者可以超出 `&Gpu` 借用期，并让同一上下文的
    /// world / UI 两个渲染器共享同一套注册表。
    gpu: Arc<Gpu>,
    config: wgpu::SurfaceConfiguration,
    /// 深度 / 模板附件格式（构造期固定；转发给每帧的 [`RenderFrame`]）。
    depth_format: wgpu::TextureFormat,
}

impl RenderContext {
    /// 由 winit `Window` 与 [`RenderConfig`] 创建。
    ///
    /// # Safety
    ///
    /// 调用方必须保证 `window` 的存活期长于本 `RenderContext`（surface 内部把它当成
    /// `'static`）。在 `rjw_krusie::runtime` 里由 `Engine` 保证（窗口与上下文同生命周期）；
    /// 手写事件循环时必须自行保证窗口不被提前 drop / 移动后失效。
    ///
    /// 失败路径（无 surface / 无适配器 / 无 surface 格式 / 设备创建失败）返回
    /// [`RenderInitError`]，不再 panic —— 调用方可以降级或给出可读提示。
    pub unsafe fn new(window: &Window, config: &RenderConfig) -> Result<Self, RenderInitError> {
        // wgpu 30: InstanceDescriptor no longer implements Default.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: config.backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });

        // SAFETY: 由调用方保证（见上）。
        let window_static: &'static Window = unsafe { std::mem::transmute(window) };
        let surface = instance
            .create_surface(window_static)
            .map_err(|e| RenderInitError::CreateSurface(e.to_string()))?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| RenderInitError::NoAdapter(e.to_string()))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: Default::default(),
            memory_hints: Default::default(),
            trace: Default::default(),
        }))
        .map_err(|e| RenderInitError::CreateDevice(e.to_string()))?;

        let size = window.inner_size();
        let surface_caps = surface.get_capabilities(&adapter);
        let format = match config.desired_format {
            Some(f) => f,
            None => *surface_caps
                .formats
                .first()
                .ok_or(RenderInitError::NoSurfaceFormat)?,
        };

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: if config.vsync.is_on() {
                wgpu::PresentMode::AutoVsync
            } else {
                wgpu::PresentMode::AutoNoVsync
            },
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &surface_config);

        let gpu = Arc::new(Gpu::new(Arc::new(device), Arc::new(queue)));

        Ok(Self { surface, gpu, config: surface_config, depth_format: config.depth_format })
    }

    /// GPU 能力对象（资源工厂 + 逃生口 + 资源注册表）。
    #[inline]
    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    /// GPU 能力对象的**共享句柄**（供需要超出 `&self` 生命周期的持有者，如
    /// `Render2D`）。
    #[inline]
    pub fn gpu_arc(&self) -> &Arc<Gpu> {
        &self.gpu
    }

    /// 取一帧并开始录制（**唯一取帧入口**的固有方法）。
    ///
    /// 返回 `None` 表示本帧不可呈现（最小化 / 遮挡 / 超时 / surface 丢失或过期），
    /// 调用方应跳过渲染（但逻辑可继续）。`Outdated` / `Lost` 时内部已自动 reconfigure。
    pub fn acquire_frame(&mut self) -> Option<RenderFrame> {
        self.try_acquire_frame()
    }

    fn try_acquire_frame(&mut self) -> Option<RenderFrame> {
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                // 已取得，但 surface 配置不再匹配：重配并继续用这张纹理。
                self.surface.configure(self.gpu.device(), &self.config);
                t
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(self.gpu.device(), &self.config);
                return None;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                log::error!("wgpu surface lost; reconfiguring");
                self.surface.configure(self.gpu.device(), &self.config);
                return None;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return None,
        };
        Some(RenderFrame::from_surface_texture(
            self.gpu.device_arc(),
            self.gpu.device_key(),
            self.gpu.queue_arc(),
            texture,
            self.config.format,
            self.depth_format,
            self.gpu.vp_layout().clone(),
        ))
    }

    /// 深度 / 模板附件格式（构造期固定）。
    #[inline]
    pub fn depth_format(&self) -> wgpu::TextureFormat {
        self.depth_format
    }

    /// 窗口尺寸变化时重建 surface 配置。
    #[inline]
    pub fn resize(&mut self, size: (u32, u32)) {
        self.config.width = size.0.max(1);
        self.config.height = size.1.max(1);
        self.surface.configure(self.gpu.device(), &self.config);
    }

    /// 底层 surface（逃生口）。
    #[inline]
    pub fn surface(&self) -> &wgpu::Surface<'static> {
        &self.surface
    }

    /// 当前表面尺寸（像素）。
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// 当前表面格式。
    #[inline]
    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }
}

impl FrameSource for RenderContext {
    #[inline]
    fn acquire_frame(&mut self) -> Option<RenderFrame> {
        self.try_acquire_frame()
    }
}
