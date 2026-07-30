use winit::window::Window;

/// Configuration for the render context.
#[derive(Clone, Debug)]
pub struct RenderConfig {
    /// The wgpu backends to try (default: DX12 | Vulkan on Windows).
    /// 尝试使用的 wgpu 后端。
    pub backends: wgpu::Backends,
    /// Enable vsync (present mode: AutoVsync)
    pub vsync: bool,
    /// The desired format for the swapchain surface.
    /// If None, the preferred format for the adapter is used.
    pub desired_format: Option<wgpu::TextureFormat>,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            // DX12-Only when using windows
            backends: if cfg!(target_os = "windows") { wgpu::Backends::DX12 } else { wgpu::Backends::all() },
            vsync: true,
            desired_format: None,
        }
    }
}

/// A wgpu-based render context that manages the surface, device, queue, and swapchain.
///
/// Usage:
/// ```ignore
/// let mut render = RenderContext::new(&window, &RenderConfig::default());
/// let (texture, view) = render.begin_frame();
/// // ... draw commands ...
/// render.end_frame(encoder);
/// ```
pub struct RenderContext {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
}

impl RenderContext {
    /// Create a new `RenderContext` from a winit `Window` and a `RenderConfig`.
    ///
    /// # Safety
    ///
    /// The caller must ensure the `Window` outlives this `RenderContext`.
    /// In typical usage with `rjw_main`, this is guaranteed because the
    /// window lives until the event loop exits.
    pub fn new(window: &Window, config: &RenderConfig) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: config.backends,
            ..Default::default()
        });

        // SAFETY: The framework (rjw_main) holds the Window alive until
        // event loop termination, which outlives all RenderContext usage.
        let window_static: &'static Window = unsafe { std::mem::transmute(window) };
        let surface = instance.create_surface(window_static).expect("Failed to create surface");

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("Failed to find a suitable adapter");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))
        .expect("Failed to create device");

        let size = window.inner_size();
        let surface_caps = surface.get_capabilities(&adapter);
        let format = config.desired_format.unwrap_or_else(|| {
            *surface_caps.formats.first().expect("No surface formats available")
        });

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: if config.vsync {
                wgpu::PresentMode::AutoVsync
            } else {
                wgpu::PresentMode::AutoNoVsync
            },
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        Self {
            surface,
            device,
            queue,
            config: surface_config,
        }
    }

    /// Begin a new frame. Returns `None` if the surface is outdated (e.g. window is minimized/resized).
    /// Returns `Some((surface_texture, texture_view))` on success.
    pub fn begin_frame(&mut self) -> Option<(wgpu::SurfaceTexture, wgpu::TextureView)> {
        let texture = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(wgpu::SurfaceError::Outdated) => {
                // Reconfigure the surface and skip this frame.
                self.surface.configure(&self.device, &self.config);
                return None;
            }
            Err(e) => {
                log::error!("wgpu get_current_texture error: {e:?}");
                return None;
            }
        };
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        Some((texture, view))
    }

    /// Submit a command encoder and present the frame.
    pub fn end_frame(&mut self, surface_texture: wgpu::SurfaceTexture, encoder: wgpu::CommandEncoder) {
        self.queue.submit(std::iter::once(encoder.finish()));
        surface_texture.present();
    }

    /// Recreate the swapchain when the window is resized.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Access the device (e.g. for creating buffers, shaders, pipelines).
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Access the queue (e.g. for writing to buffers, submitting).
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Current surface format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }
}