// --- Re-exports ---

pub use rjw_time::DeltaTimer;
pub use rjw_keyboard::KeyboardInput;
pub use rjw_mouse::MouseInput;

pub use winit;
pub use winit::keyboard::KeyCode;
pub use winit::event::MouseButton;
pub use winit::event_loop::EventLoop;
pub use winit::event_loop::ActiveEventLoop;
pub use winit::error::EventLoopError;
pub use winit::event::WindowEvent;
pub use winit::event::DeviceEvent;
pub use winit::dpi::Size;
pub use winit::dpi::LogicalPosition;
pub use winit::dpi::LogicalSize;
pub use winit::dpi::PhysicalPosition;
pub use winit::dpi::PhysicalSize;
pub use winit::window::Window;
pub use winit::window::WindowAttributes;

use std::sync::LazyLock;
pub const PRIMARY_WINDOW_TITLE_DEFAULT: &str =  "rjw primary window";
pub static PRIMARY_WINDOW_TITLE: LazyLock<String> = LazyLock::new(|| {
    match std::env::current_exe() {
        Ok(path) => {
            match path.file_name() {
                Some(name) => {
                    name.to_str().unwrap_or(PRIMARY_WINDOW_TITLE_DEFAULT).to_owned()
                }
                None => {
                    PRIMARY_WINDOW_TITLE_DEFAULT.to_owned()
                }
            }
        }
        Err(_) => {
            PRIMARY_WINDOW_TITLE_DEFAULT.to_owned()
        }
    }
});

pub trait App {
    #[must_use]
    fn primary_window_attrib(&self) -> WindowAttributes {
        WindowAttributes::default().with_title(&*PRIMARY_WINDOW_TITLE)
    }

    fn on_init(&mut self, ctx: &mut MainContext);
    fn about_to_wait(&mut self, ctx: &mut MainContext);

    /// Called when the primary window is resized.
    /// 当主窗口大小改变时调用。
    #[allow(unused_variables)]
    fn on_resized(&mut self, ctx: &mut MainContext, width: u32, height: u32) {}
}

pub struct MainContext {
    pub timer: DeltaTimer,
    pub keyboard: KeyboardInput,
    pub mouse: MouseInput,
    primary_window: Option<Window>,
    exit_requested: bool,
}

impl MainContext {
    /// Access the primary window.
    /// Returns `None` if the window has not been initialized yet
    /// (e.g. before `resumed` is called in the event loop).
    /// 如果窗口尚未初始化，返回 `None`（例如在事件循环的 `resumed` 被调用之前）。
    pub fn primary_window(&self) -> Option<&Window> {
        self.primary_window.as_ref()
    }

    /// Request the event loop to exit after the current frame.
    /// 请求在当帧结束后退出事件循环。
    pub fn request_exit(&mut self) {
        self.exit_requested = true;
    }
}

struct MainHandler<T: App> {
    app: T,
    ctx: MainContext,
    resized: Option<PhysicalSize<u32>>,
}

use winit::application::ApplicationHandler;
impl<T: App> ApplicationHandler for MainHandler<T> {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        // 防止 resumed 重复触发（部分平台会多次回调）时重复建窗 / 重复调用 on_init。
        if self.ctx.primary_window.is_some() {
            return;
        }
        let window = event_loop.create_window(self.app.primary_window_attrib()).expect("Initalizing the primary window failed");
        self.ctx.primary_window = Some(window);
        self.app.on_init(&mut self.ctx);
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    )
    {
        self.ctx.keyboard.window_event(&event);
        self.ctx.mouse.window_event(&event);

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(resized) => {
                self.resized = Some(resized);
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    )
    {
        self.ctx.mouse.device_event(&event);
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.ctx.timer.per_frame();
        if let Some(PhysicalSize { width, height }) = self.resized {
            if self.ctx.primary_window.is_some() {
                self.app.on_resized(&mut self.ctx, width, height);
            }
            self.resized = None;
        }
        self.app.about_to_wait(&mut self.ctx);

        if self.ctx.exit_requested {
            event_loop.exit();
        }

        self.ctx.keyboard.end_frame();
        self.ctx.mouse.end_frame();
    }
}

impl<T: App> MainHandler<T> {
    pub fn new(app: T) -> Self {
        Self {
            app,
            ctx: MainContext {
                timer: DeltaTimer::default(),
                keyboard: KeyboardInput::default(),
                mouse: MouseInput::default(),
                primary_window: None,
                exit_requested: false,
            },
            resized: None,
        }
    }
}

pub fn run_app(app: impl App) -> Result<(), EventLoopError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    
    let mut handler = MainHandler::new(app);
    event_loop.run_app(&mut handler)
}