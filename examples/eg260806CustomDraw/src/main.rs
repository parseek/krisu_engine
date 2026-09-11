//! eg260806CustomDraw —— 演示引擎的「逃逸舱口」`custom` / `CustomDraw`
//!
//! 展示能力：
//! - **结构体形式**：实现 `CustomDraw` trait，持有自建管线 + 顶点缓冲，
//!   在引擎已打开 RenderPass 内直接 `set_pipeline + set_vertex_buffer + draw`。
//! - **闭包形式**：`custom(move |pass| ...).layer(layer)`，blanket impl 自动实现。
//! - 与引擎自带的 Sprite 批处理**混排**（custom 三角形夹在两个 Sprite 层之间），
//!   证明 `custom` 参与 (layer, states) 排序并按需执行。
//!
//! # 迁移说明（旧 driver → 新 runtime API）
//!
//! - `App::primary_window_attrib` → `App::config`（`AppConfig::new(title).size(w, h)`）；
//! - `App::on_init(&mut MainContext)` → `App::init(&Gfx)`；`about_to_wait` → `update(&mut Ctx)`；
//! - `on_resized` → 运行时接管（`resize` 自动重配 surface，`f.submit` 自动把画面矩形写回
//!   `Camera2D::region`，因此示例不再需要 `resized` 钩子）；
//! - `render2d.set_mvp(..)` + `render2d.render(&mut rctx, &ClearConfig)` →
//!   `f.submit(&mut cam, Clear::color(Color))`；
//! - `run_app(..)` → `run(..)`；`ClearConfig { color: Some(wgpu::Color { .. }) }` → `Clear::color(Color)`。
//!
//! ⚠️ **已知 API 缺口（未自造）**：自建渲染管线需要 **surface 格式**，而 `Gfx` 只暴露
//! `device` / `queue` / `texture_layout`（`RenderContext::format()` 没有对应入口）。
//! 因此四个三角形在**拿到第一帧时**用 `Render2D::format()` 构建（见 [`CustomDrawApp::ensure_tris`]）——
//! 这是当前能拿到真实 surface 格式的最近入口；若 `Gfx` 补上 `format()`，构建可移回 `init`。

// `wgpu` 不在本示例的依赖里：一切 wgpu 类型都经 `rjw_krusie` 的重导出取用。
use rjw_krusie::gpu::wgpu;
use rjw_krusie::gpu::wgpu::util::DeviceExt;
use rjw_krusie::prelude::*;
use rjw_krusie::render2d::CustomDraw;

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
/// `Clone` 可行（wgpu 资源句柄内部 Arc），便于传入多个 `custom`。
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
        color: [Color; 3],
    ) -> Self {
        // 交错：pos(2×f32) + color(4×f32) = 24 字节 / 顶点
        let mut verts = Vec::with_capacity(3 * 6);
        for (&(x, y), &color) in pts.iter().zip(color.iter()) {
            verts.extend_from_slice(&[x, y]);
            let v: [f32; 4] = color.into();
            verts.extend_from_slice(&v);
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

/// 结构体形式：实现 `CustomDraw` trait（签名不变）。
impl CustomDraw for Tri {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_to(pass);
    }
}

/// 应用状态：相机 + 自建三角形。
///
/// 旧版的 `render: Option<RenderContext>` / `render2d: Option<Render2D>` 已删除——
/// 渲染器与表面由运行时（`Engine`）持有，应用只保留自建资源。
#[derive(Default)]
struct CustomDrawApp {
    /// 相机：`region`（画面矩形，`submit` 每帧写回）+ `transform`（位姿）。
    cam: Camera2D,
    /// 自绘三角形（首帧用真实 surface 格式构建）。
    tris: Vec<Tri>,
    /// 累计时间（驱动 Sprite 旋转）。
    t_elapsed: f32,
}

impl CustomDrawApp {
    /// 首帧构建自建管线 / 顶点缓冲；返回 `false` 表示本帧还不具备绘制条件。
    ///
    /// 需要用 surface 格式建管线：`Render2D::format()` 是当前唯一入口
    /// （`Gfx` 未暴露格式，见文件头说明）。设备 / 队列走 `Frame::escape_device_queue()`。
    fn ensure_tris(&mut self, f: &mut Frame<'_>) -> bool {
        if self.tris.is_empty() {
            let surface_format = f.draw().format();
            let Some((device, _queue)) = f.escape_device_queue() else {
                return false;
            };

            // 三个自绘三角形（NDC 坐标直接指定，与相机无关）。
            // 左：红色；中：绿色；右：蓝色，位置固定。
            self.tris.push(Tri::new(
                device,
                surface_format,
                [(-0.9, -0.7), (-0.55, 0.7), (-0.2, -0.4)],
                [[1.0, 0.2, 0.2, 1.0].into(); 3],
            ));
            self.tris.push(Tri::new(
                device,
                surface_format,
                [(0.1, 0.7), (0.6, 0.2), (0.1, -0.6)],
                [[0.2, 0.9, 0.3, 1.0].into(); 3],
            ));
            self.tris.push(Tri::new(
                device,
                surface_format,
                [(0.55, -0.75), (0.95, 0.0), (0.5, 0.6)],
                [[0.25, 0.45, 1.0, 1.0].into(); 3],
            ));
            self.tris.push(Tri::new(
                device,
                surface_format,
                [(-0.20, -0.20), (-0.50, -0.40), (-0.05, -0.90)],
                [
                    [1.0, 0.2, 0.2, 1.0].into(),
                    [0.2, 0.9, 0.3, 1.0].into(),
                    Color::AQUA,
                ],
            ));
        }
        !self.tris.is_empty()
    }
}

impl App for CustomDrawApp {
    fn config(&self) -> AppConfig {
        AppConfig::new("eg260806CustomDraw").size(1280.0, 720.0)
    }

    fn update(&mut self, ctx: &mut Ctx) {
        // ── 逻辑半程：无帧也执行（后台模拟 / 计时 / 输入状态持续）──
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }
        self.t_elapsed += ctx.dt().min(0.05);
        let t = self.t_elapsed;

        // ── 渲染半程：守卫在应用里 ──
        let Some(mut f) = ctx.frame() else {
            return;
        };

        // 自建管线（首帧用真实 surface 格式构建；见 `ensure_tris` 说明）。
        if !self.ensure_tris(&mut f) {
            return;
        }

        // ── 引擎自己的 Sprite（layer 0：底层）── 旋转的蓝色方块
        let board = SpriteRect::new((-70.0, -70.0), (140.0, 140.0));
        f.draw()
            .solid(board)
            .tint(Color::rgba(0.12, 0.28, 0.6, 1.0))
            .transform(Transform2D::default().with_pos(Vec2::ZERO).with_rot(t * 0.7))
            .layer(LAYER_BACK);

        // ── 结构体形式：三个自绘三角形（layer 1，夹在 Sprite 之间）──
        for tri in &self.tris[1..] {
            f.draw().custom(tri.clone()).layer(LAYER_MID); // CustomDraw: Send + Sync，Arc 句柄可 clone
        }

        // ── 闭包形式：等价写法（blanket impl：Fn(&mut RenderPass) + Send + Sync）──
        // 这里让第一个三角形再画一次，验证同一资源可多路复用。
        let tri0 = self.tris[0].clone();
        f.draw()
            .custom(move |pass: &mut wgpu::RenderPass<'_>| {
                tri0.draw_to(pass);
            })
            .layer(LAYER_MID - 0.1);

        // ── 引擎自己的 Sprite（layer 2：顶层）── 半透明黄色条盖住 overlap 部分
        let top = SpriteRect::new((-40.0, -240.0), (80.0, 480.0));
        f.draw()
            .solid(top)
            .tint(Color::rgba(1.0, 0.85, 0.2, 0.55))
            .transform(Transform2D::default().with_pos(Vec2::splat(150.0)).with_rot(0.4))
            .layer(LAYER_TOP);

        // 窗口标题显示帧率（低层逃生口：经 `Frame` 的 Deref 取 winit 窗口句柄）
        if let Some(w) = f.window_handle() {
            w.set_title(&format!(
                "eg260731CustomDraw  FPS {:.0}  |  红色三角形由结构体形式绘制；闭包形式重复绘制一次；蓝色/绿色/彩色为自创管线",
                f.fps()
            ));
        }

        // ── 提交：一个画面 = 一次 submit(相机, clear) ──
        // 清屏色与旧的 `wgpu::Color { r: 0.1, g: 0.1, b: 0.14, a: 1.0 }` 逐分量一致。
        //
        // 不需要显式 `present()`：`Frame` 析构会自动呈现，且 `submit` 已把本帧标记为
        // 「应用已接管」，帧尾不会再用 `AppConfig::clear` 补一个 pass。
        f.submit(&mut self.cam, Clear::color(Color::rgb(0.1, 0.1, 0.14)));
    }
}

const LAYER_BACK: f32 = 0.0;
const LAYER_MID: f32 = 1.0;
const LAYER_TOP: f32 = 2.0;

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    run(CustomDrawApp::default())
}
