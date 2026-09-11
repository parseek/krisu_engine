//! 应用骨架：`App` trait + `run` / `run_with` + 组合根 `Engine`。
//!
//! # 帧循环（守卫在应用里）
//!
//! ```text
//! loop {
//!     取帧（可能 None：最小化 / 遮挡 / 超时 / 丢失）
//!     App::update(&mut ctx)         ← 无帧也执行（允许后台模拟）
//!     ctx.frame() → Some ⇒ 应用渲染；None ⇒ 应用自己 return（渲染代码一行不执行）
//!     帧尾：未提交则用配置清屏自动提交 + present；清空残留录制
//!     无帧连续 ⇒ 按 AppConfig::background 退避（避免 Poll 空转）
//! }
//! ```

use rjw_main::winit;
use rjw_main::winit::application::ApplicationHandler;
use rjw_main::{
    ActiveEventLoop, DeviceEvent, EventLoop, EventLoopError, LogicalSize, WindowAttributes,
    WindowEvent,
};
use rjw_render::{FrameSource, Never, RenderContext};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::runtime::config::{AppConfig, Background};
use crate::runtime::ctx::Ctx;
use crate::runtime::gfx::Gfx;

/// 应用入口 trait。
///
/// - [`App::update`] **每帧必调**（含取不到表面的帧）——后台逻辑 / 计时 / 输入状态持续；
/// - 渲染代码写在 `update` 内、由 `let Some(mut f) = ctx.frame() else { return };` 守卫；
/// - 其余方法都有默认实现（最小应用只需实现 `update`）。
pub trait App: Sized + 'static {
    /// 窗口 / 渲染 / 清屏 / 后台策略配置。
    fn config(&self) -> AppConfig {
        AppConfig::default()
    }

    /// 窗口就绪后调用一次：用 [`Gfx`] 建长期资源（纹理 / 网格 / 文本子系统…）。
    fn init(&mut self, _gfx: &Gfx<'_>) {}

    /// 每帧调用（含无帧帧）。
    fn update(&mut self, ctx: &mut Ctx);

    /// 窗口尺寸变化（可选；画面矩形已自动跟随，这里用于重建依赖尺寸的资源）。
    fn resized(&mut self, _ctx: &mut Ctx) {}

    /// 退出前调用一次（可选）。
    fn close(&mut self) {}
}

/// 运行应用（自建渲染上下文）。
pub fn run<A: App>(app: A) -> Result<(), EventLoopError> {
    run_engine(app, None::<Never>)
}

/// 运行应用并注入自定义**帧源**（测试 / 自定义输出目标）。
///
/// ```ignore
/// run_with(app, Never);   // 强制永不出帧：验证「无帧也执行 update」
/// ```
pub fn run_with<A: App, S: FrameSource>(app: A, source: S) -> Result<(), EventLoopError> {
    run_engine(app, Some(source))
}

fn run_engine<A: App, S: FrameSource>(app: A, injected: Option<S>) -> Result<(), EventLoopError> {
    let mut config = app.config();
    // 冒烟开关：`--frames N` 或 `KRUSIE_SMOKE_FRAMES=N` ⇒ 跑满 N 次迭代自动退出。
    if let Some(frames) = smoke_frames_arg() {
        config.exit_after_frames = Some(frames);
    }
    let ctx = Ctx::new(&config);
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut engine = Engine { app, config, injected, render: None, ctx, resized: None, smoke_reported: false };
    event_loop.run_app(&mut engine)
}

/// 解析冒烟帧数：`--frames N` 优先，其次 `KRUSIE_SMOKE_FRAMES=N`。
fn smoke_frames_arg() -> Option<u64> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--frames" {
            return args.next().and_then(|v| v.parse().ok());
        }
    }
    std::env::var("KRUSIE_SMOKE_FRAMES").ok().and_then(|v| v.parse().ok())
}

/// 组合根：窗口 + 事件循环 + 渲染上下文 + 每帧上下文 + 用户应用。
struct Engine<A: App, S: FrameSource> {
    app: A,
    config: AppConfig,
    /// 注入的帧源（`Some` 时忽略自建渲染上下文的取帧）。
    injected: Option<S>,
    render: Option<RenderContext>,
    ctx: Ctx,
    resized: Option<(u32, u32)>,
    /// 冒烟收尾是否已报告（`event_loop.exit()` 之后仍可能再跑一次迭代）。
    smoke_reported: bool,
}

impl<A: App, S: FrameSource> ApplicationHandler for Engine<A, S> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // 部分平台会多次回调 `resumed`：只初始化一次。
        if self.ctx.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title(&self.config.title)
            .with_inner_size(LogicalSize::new(self.config.size.0 as f64, self.config.size.1 as f64));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("krusie: 创建主窗口失败"),
        );
        // IME（中文输入法等）：否则部分平台（Windows）不产生 `WindowEvent::Ime`。
        window.set_ime_allowed(true);

        // SAFETY: window 是 `Arc`，与 `RenderContext` 一同活到事件循环结束。
        let render = unsafe { RenderContext::new(&window, &self.config.render) };
        self.ctx.attach(window, &render);
        self.render = Some(render);

        let render_ref = self.render.as_ref().expect("render 刚创建");
        let gfx = Gfx::new(
            render_ref.gpu(),
            render_ref.format(),
            render_ref.depth_format(),
            render_ref.size(),
        );
        self.app.init(&gfx);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        self.ctx.keyboard.window_event(&event);
        self.ctx.mouse.window_event(&event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resized = Some((size.width, size.height)),
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        self.ctx.mouse.device_event(&event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // 尺寸变化：重配 surface + 刷新 DPI（画面矩形随 `Ctx::region` 自动跟随）
        if let Some((w, h)) = self.resized.take() {
            if let Some(render) = self.render.as_mut() {
                render.resize((w, h));
            }
            self.ctx.size = (w.max(1), h.max(1));
            self.ctx.scale = self
                .ctx
                .window
                .as_ref()
                .map(|win| win.scale_factor() as f32)
                .unwrap_or(1.0);
            self.app.resized(&mut self.ctx);
        }

        // 取帧（注入的帧源优先；否则用自建渲染上下文）
        self.ctx.surface = match (&mut self.injected, &mut self.render) {
            (Some(source), _) => source.acquire_frame(),
            (None, Some(render)) => render.acquire_frame(),
            _ => None,
        };

        // 宿主事实 → 应用逻辑（无帧也执行）→ 帧尾收尾
        self.ctx.begin_update();
        self.app.update(&mut self.ctx);
        self.ctx.end_update();

        // 输入边沿在本帧消费完之后推进
        self.ctx.keyboard.next_frame();
        self.ctx.mouse.next_frame();

        if self.ctx.exit_requested {
            self.ctx.exit_requested = false;
            event_loop.exit();
        }
        if let Some(limit) = self.config.exit_after_frames
            && !self.smoke_reported && self.ctx.frames >= limit {
                self.smoke_reported = true;
                // 冒烟收尾：报告迭代数与**实际呈现**帧数。
                // 只数迭代数会漏掉「提交了但从未 present ⇒ 画面一直是空白」这类回归
                // （egHello / eg260731RPG 曾因 `Frame::submit` 误置 presented 而黑屏），
                // 因此 0 呈现直接以退出码 2 失败，让冒烟套件能挡住"能跑但没画面"。
                let (frames, presented) = (self.ctx.frames, self.ctx.presented_frames());
                if presented == 0 {
                    eprintln!(
                        "krusie smoke: [FAIL] {frames} iterations but 0 frames presented \
                         (画面不会更新：检查 Frame::submit / present 路径)"
                    );
                    std::process::exit(2);
                }
                eprintln!("krusie smoke: [OK] {frames} iterations / {presented} frames presented");
                event_loop.exit();
            }

        // 无帧退避：避免「后台执行」变成 Poll 空转烧 CPU
        use winit::event_loop::ControlFlow;
        if self.ctx.no_frame_streak == 0 {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            match self.config.background {
                Background::Poll => event_loop.set_control_flow(ControlFlow::Poll),
                Background::Throttle(ms) => event_loop
                    .set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(ms as u64))),
                Background::Wait => event_loop.set_control_flow(ControlFlow::Wait),
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.app.close();
    }
}
