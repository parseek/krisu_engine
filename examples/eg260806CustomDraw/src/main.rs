//! eg260731CustomDraw —— 演示引擎的「逃逸舱口」`add_custom` / `CustomDraw`
//!
//! 展示能力：
//! - **结构体形式**：实现 `CustomDraw` trait，持有自建管线 + 顶点缓冲，
//!   在引擎已打开 RenderPass 内直接 `set_pipeline + set_vertex_buffer + draw`。
//! - **闭包形式**：`add_custom(layer, move |pass| ...)`，blanket impl 自动实现。
//! - 与引擎自带的 Sprite 批处理**混排**（custom 三角形夹在两个 Sprite 层之间），
//!   证明 `add_custom` 参与 (layer, states) 排序并按需执行。

use glam::Vec2;
use rjw_2d_render::{ClearConfig, CustomDraw, Render2D, SpriteRect};
use rjw_color::Color;
use rjw_main::*;
use rjw_render::{RenderConfig, RenderContext, wgpu, wgpu::util::DeviceExt};
use rjw_transform::{Camera2D, Transform2D};

/// 自定义绘制指令着色器：顶点位置直接用 NDC 坐标（不经过 engine 的 VP 统一缓冲）。
const CUSTOM_WGSL: &str = r#"
struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec4<f32>,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};
@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = vec4<f32>(in.pos, 0.0, 1.0);
    out.color = in.color;
    return out;
}
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

/// 一个自绘三角形：独立管线 + 顶点缓冲（位置 + 颜色交错）。
/// `Clone` 可行（wgpu 资源句柄内部 Arc），便于传入多个 `add_custom`。
#[derive(Clone)]
struct Tri {
    pipeline: wgpu::RenderPipeline,
    vbo: wgpu::Buffer,
    n_verts: u32,
}

impl Tri {
    fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        pts: [(f32, f32); 3],
        color: [f32; 4],
    ) -> Self {
        // 交错：pos(2×f32) + color(4×f32) = 24 字节 / 顶点
        let mut verts = Vec::with_capacity(3 * 6);
        for &(x, y) in &pts {
            verts.extend_from_slice(&[x, y]);
            verts.extend_from_slice(&color);
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CustomDraw: shader"),
            source: wgpu::ShaderSource::Wgsl(CUSTOM_WGSL.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("CustomDraw: empty layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });

        let attributes = wgpu::vertex_attr_array![
            0 => Float32x2, // pos
            1 => Float32x4, // color
        ];
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: 24,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("CustomDraw: triangle pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None, // 不透明三角，直接覆盖
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let vbo = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("CustomDraw: tri vbo"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Self {
            pipeline,
            vbo,
            n_verts: 3,
        }
    }

    /// 底层绘制：供结构体实现与闭包形式共用。
    fn draw_to(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vbo.slice(..));
        pass.draw(0..self.n_verts, 0..1);
    }
}

/// 结构体形式：实现 `CustomDraw` trait。
impl CustomDraw for Tri {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_to(pass);
    }
}

struct CustomDrawApp {
    render: Option<RenderContext>,
    render2d: Option<Render2D>,
    cam: Camera2D,
    tris: Vec<Tri>,
    t_elapsed: f32,
}

impl CustomDrawApp {
    fn new() -> Self {
        Self {
            render: None,
            render2d: None,
            cam: Camera2D::new(Vec2::splat(0.0)),
            tris: Vec::new(),
            t_elapsed: 0.0,
        }
    }
}

impl App for CustomDrawApp {
    fn primary_window_attrib(&self) -> WindowAttributes {
        WindowAttributes::default()
            .with_title("eg260731CustomDraw - add_custom / CustomDraw")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
    }

    fn on_init(&mut self, ctx: &mut MainContext) {
        let window = ctx.primary_window().expect("primary window must exist during on_init");
        self.render = Some(RenderContext::new(window, &RenderConfig::default()));
        let render = self.render.as_ref().expect("render must be initialized");
        let surface_format = render.format();

        let render2d = Render2D::new(render);

        // 三个自绘三角形（NDC 坐标直接指定，与相机无关）。
        // 左：红色；中：绿色；右：蓝色，位置固定。
        self.tris.push(Tri::new(
            render2d.device(),
            surface_format,
            [(-0.9, -0.7), (-0.55, 0.7), (-0.2, -0.4)],
            [1.0, 0.2, 0.2, 1.0],
        ));
        self.tris.push(Tri::new(
            render2d.device(),
            surface_format,
            [(0.1, 0.7), (0.6, 0.2), (0.1, -0.6)],
            [0.2, 0.9, 0.3, 1.0],
        ));
        self.tris.push(Tri::new(
            render2d.device(),
            surface_format,
            [(0.55, -0.75), (0.95, 0.0), (0.5, 0.6)],
            [0.25, 0.45, 1.0, 1.0],
        ));

        let (w, h) = render.size();
        self.cam = Camera2D::new(Vec2::new(w as f32, h as f32));

        self.render2d = Some(render2d);
    }

    fn on_resized(&mut self, _ctx: &mut MainContext, width: u32, height: u32) {
        if let Some(render) = &mut self.render {
            render.resize(width, height);
        }
        self.cam.set_vp(Vec2::new(width as f32, height as f32), Vec2::ZERO);
    }

    fn about_to_wait(&mut self, ctx: &mut MainContext) {
        if ctx.keyboard.get(KeyCode::Escape).down_edge() {
            ctx.request_exit();
        }
        self.t_elapsed += ctx.timer.dt().get_f32().min(0.05);
        let t = self.t_elapsed;

        let Some(render2d) = &mut self.render2d else {
            return;
        };
        render2d.set_mvp(self.cam.vp_matrix());

        // ── 引擎自己的 Sprite（layer 0：底层）── 旋转的蓝色方块
        let board = SpriteRect::from_texture(Vec2::splat(-70.0), Vec2::splat(140.0));
        render2d.add_sprite2d_solid(
            board,
            Color::rgba(0.12, 0.28, 0.6, 1.0),
            Transform2D::default().with_pos(Vec2::ZERO).with_rot(t * 0.7),
            LAYER_BACK,
        );

        // ── 结构体形式：三个自绘三角形（layer 1，夹在 Sprite 之间）──
        for tri in &self.tris {
            render2d.add_custom(LAYER_MID, tri.clone()); // CustomDraw: Send + Sync，Arc 句柄可 clone
        }

        // ── 闭包形式：等价写法（blanket impl：Fn(&mut RenderPass) + Send + Sync）──
        // 这里让第一个三角形再画一次，验证同一资源可多路复用。
        let tri0 = self.tris[0].clone();
        render2d.add_custom(LAYER_MID + 0.1, move |pass: &mut wgpu::RenderPass<'_>| {
            tri0.draw_to(pass);
        });

        // ── 引擎自己的 Sprite（layer 2：顶层）── 半透明黄色条盖住 overlap 部分
        let top = SpriteRect::from_texture(Vec2::new(-40.0, -240.0), Vec2::new(80.0, 480.0));
        render2d
            .add_sprite2d_solid(
                top,
                Color::rgba(1.0, 0.85, 0.2, 0.55),
                Transform2D::default().with_pos(Vec2::splat(150.0)).with_rot(0.4),
                LAYER_TOP,
            );

        if let Some(w) = ctx.primary_window() {
            w.set_title(&format!(
                "eg260731CustomDraw  FPS {:.0}  |  红色三角形由结构体形式绘制；闭包形式重复绘制一次；蓝色/绿色为自创管线",
                ctx.timer.get_fps()
            ));
        }

        render2d.render(&ClearConfig {
            color: Some(wgpu::Color { r: 0.1, g: 0.1, b: 0.14, a: 1.0 }),
            depth: None,
            stencil: None,
        });
    }
}

const LAYER_BACK: f32 = 0.0;
const LAYER_MID: f32 = 1.0;
const LAYER_TOP: f32 = 2.0;

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    rjw_main::run_app(CustomDrawApp::new())
}