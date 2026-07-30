use rjw_color::ColorF64;
use rjw_main::*;
use rjw_render::{RenderConfig, RenderContext};

struct ClearScreen {
    render: Option<RenderContext>,
}

impl ClearScreen {
    fn new() -> Self {
        Self { render: None }
    }
}

impl App for ClearScreen {
    fn primary_window_attrib(&self) -> WindowAttributes {
        WindowAttributes::default()
            .with_title("eg260729 - Clear Screen")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
    }

    fn on_init(&mut self, ctx: &mut MainContext) {
        let window = ctx.primary_window().expect("primary window must exist during on_init");
        self.render = Some(RenderContext::new(window, &RenderConfig::default()));
    }

    fn on_resized(&mut self, _ctx: &mut MainContext, width: u32, height: u32) {
        if let Some(render) = &mut self.render {
            render.resize(width, height);
        }
    }

    fn about_to_wait(&mut self, ctx: &mut MainContext) {
        if ctx.keyboard.get(KeyCode::Escape).down_edge() {
            ctx.request_exit();
        }

        let Some(render) = &mut self.render else {
            return;
        };

        let Some((surface_texture, view)) = render.begin_frame() else {
            return;
        };


        if let Some(w) = ctx.primary_window() {
            w.set_title(&format!("FPS: {:.02}; wheel: {:?}", ctx.timer.get_fps(), ctx.mouse.get_wheel_delta().to_pixel(None)));
        }

        // Use ColorF64 which can convert directly to wgpu::Color.
        let clear = ColorF64::rgba(0.1, 0.2, 0.4 + ctx.mouse.get_wheel_delta().to_pixel(None).1 * 0.03, 1.0).into();

        let mut encoder = render.device().create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("clear encoder"),
        });

        {
            let mut _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            // RenderPass drops here, finishing the pass.
        }

        render.end_frame(surface_texture, encoder);
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    log::info!("APP: {}", *rjw_main::PRIMARY_WINDOW_TITLE);
    run_app(ClearScreen::new())
}