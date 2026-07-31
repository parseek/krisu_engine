use rjw_2d_render::{ArcTextureWrapped, ClearConfig, Render2D, SpriteRect};
use rjw_color::Color;
use rjw_main::*;
use rjw_render::{RenderConfig, RenderContext};
use rjw_transform::{Camera2D, Transform2D};

struct SpriteDemo {
    render: Option<RenderContext>,
    render2d: Option<Render2D>,
    cam: Camera2D,
    tex: Option<ArcTextureWrapped>,
    /// 累计经过时间（秒），驱动精灵动画。
    t_elapsed: f32,
}

impl SpriteDemo {
    fn new() -> Self {
        Self {
            render: None,
            render2d: None,
            cam: Camera2D::new(glam::Vec2::splat(0.0)),
            tex: None,
            t_elapsed: 0.0,
        }
    }
}

impl App for SpriteDemo {
    fn primary_window_attrib(&self) -> WindowAttributes {
        WindowAttributes::default()
            .with_title("eg260731 - Render2D Sprites")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
    }

    fn on_init(&mut self, ctx: &mut MainContext) {
        let window = ctx.primary_window().expect("primary window must exist during on_init");
        // 先放入 self.render（堆上稳定地址），再创建 Render2D。
        // ⚠️ Render2D 持有 surface 的 'static 引用；若 RenderContext 事后被移动/替换将导致悬垂。
        self.render = Some(RenderContext::new(window, &RenderConfig::default()));
        let render = self.render.as_ref().expect("render must be initialized");

        // 创建 Render2D：内部 clone device/queue、引用 surface。
        let mut render2d = Render2D::new(render);

        // 创建一张小纹理（棋盘格图案），演示纹理贴图 sprite。
        const TEX_W: u32 = 16;
        const TEX_H: u32 = 16;
        let mut data = Vec::with_capacity((TEX_W * TEX_H) as usize * 4);
        for y in 0..TEX_H {
            for x in 0..TEX_W {
                let check = ((x / 8 + y / 8) % 2) == 0;
                if check {
                    data.extend_from_slice(&[255, 200, 0, 255]); // 黄色
                } else {
                    data.extend_from_slice(&[255, 255, 255, 255]); // 白色
                }
            }
        }
        let tex = render2d.create_texture("checkboard", &data, TEX_W, TEX_H);

        let (w, h) = render.size();
        self.cam = Camera2D::new(glam::Vec2::new(w as f32, h as f32));

        self.render2d = Some(render2d);
        self.tex = Some(tex);
    }

    fn on_resized(&mut self, _ctx: &mut MainContext, width: u32, height: u32) {
        if let Some(render) = &mut self.render {
            render.resize(width, height);
        }
        // 摄像机视口跟随窗口。
        self.cam.set_vp(
            glam::Vec2::new(width as f32, height as f32),
            glam::Vec2::ZERO,
        );
    }

    fn about_to_wait(&mut self, ctx: &mut MainContext) {
        if ctx.keyboard.get(KeyCode::Escape).down_edge() {
            ctx.request_exit();
        }

        // 帧间隔（秒）。
        let dt = ctx.timer.dt().get_f32();
        self.t_elapsed += dt;

        // 鼠标滚轮缩放、方向键平移（坐标系 X+ 右、Y+ 下：W ↔ 上/Y-，S ↔ 下/Y+）。
        let wheel = ctx.mouse.get_wheel_delta().to_pixel(None);
        if wheel.1 != 0.0 {
            self.cam.zoom *= glam::Vec2::splat(1.1_f32.powf(wheel.1 as f32 * dt));
            self.cam.zoom = self.cam.zoom.clamp(glam::Vec2::splat(0.01), glam::Vec2::splat(100.0));
        }
        let move_speed = 400.0 * dt;
        self.cam.walk_xy(glam::Vec2::new(
            (ctx.keyboard.get(KeyCode::KeyD).pressed() as i32 as f32
                - ctx.keyboard.get(KeyCode::KeyA).pressed() as i32 as f32)
                * move_speed,
            (ctx.keyboard.get(KeyCode::KeyS).pressed() as i32 as f32
                - ctx.keyboard.get(KeyCode::KeyW).pressed() as i32 as f32)
                * move_speed,
        ));
        self.cam.rotation += (ctx.keyboard.get(KeyCode::KeyE).pressed() as i32 - ctx.keyboard.get(KeyCode::KeyQ).pressed() as i32) as f32 * dt * 180.0_f32.to_radians();

        let t = self.t_elapsed;

        let Some(render2d) = &mut self.render2d else {
            return;
        };

        // 外部相机提供 VP 矩阵（列主序，直接透传）。
        render2d.set_mvp(self.cam.vp_matrix());

        let half_w = self.cam.viewport_size.x * 0.5;
        let half_h = self.cam.viewport_size.y * 0.5;
        let axis_len = half_w.max(half_h) * 1.05;

        // ── 坐标系指示线（验证 X+ 右 / Y+ 下）──
        // X+ 右（红）
        render2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(glam::Vec2::new(0.0, -2.0), glam::Vec2::new(axis_len, 4.0)),
            Color::RED,
            Transform2D::default(),
            95.0,
        );
        // X- 左（暗红）
        render2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(glam::Vec2::new(-axis_len, -2.0), glam::Vec2::new(axis_len, 4.0)),
            Color::rgba(0.5, 0.0, 0.0, 1.0),
            Transform2D::default(),
            95.0,
        );
        // Y+ 下（绿）
        render2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(glam::Vec2::new(-2.0, 0.0), glam::Vec2::new(4.0, axis_len)),
            Color::GREEN,
            Transform2D::default(),
            95.0,
        );
        // Y- 上（暗绿）
        render2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(glam::Vec2::new(-2.0, -axis_len), glam::Vec2::new(4.0, axis_len)),
            Color::rgba(0.0, 0.4, 0.0, 1.0),
            Transform2D::default(),
            95.0,
        );

        // 1. 带纹理的精灵（棋盘格），绕中心旋转 + 缩放（验证变换正确）。
        if let Some(tex) = &self.tex {
            let rect = SpriteRect::from_texture(
                glam::Vec2::new(-96.0, -96.0),
                glam::Vec2::new(192.0, 192.0),
            );
            let tf = Transform2D::default()
                .with_pos(glam::Vec2::new(0.0, 0.0))
                .with_rot(t * 0.8)
                .with_scale(glam::Vec2::splat(1.0 + 0.2 * t.sin()));
            render2d.add_sprite2d_default(rect, Color::WHITE, tf, 0.0, tex);
        }

        // 2. 8 个纯色矩形（不同层级），验证 solid 包装（原 BUG：只有 7 个）。
        const COUNTW:i32 = 28;
        const COUNTH:i32 = 28;
        for i in 0..COUNTW*COUNTH {
            let center = glam::Vec2::new(
                ((i%COUNTW) as f32 - (COUNTW as f32 - 1.0) * 0.5) * 80.0,
                (t * 0.5 + (i/COUNTH) as f32).sin() * 20.0 + ((i/COUNTH) as f32 - (COUNTH as f32 - 1.0) * 0.5) * 50.0,
            );
            let rect = SpriteRect::from_texture(
                glam::Vec2::splat(-40.0),
                glam::Vec2::new(80.0, 80.0),
            );
            let tf = Transform2D::default()
                .with_pos(center)
                .with_rot(t * 0.3 + i as f32);
            let i_mapped = i as f32 / (COUNTW*COUNTH) as f32;
            let color = Color::rgba(
                0.3 + (i_mapped * 0.7),
                0.6,
                0.9 - i_mapped * 0.7,
                0.7,
            );
            render2d.add_sprite2d_default_solid(rect, color, tf, (i%COUNTW) as f32 / COUNTW as f32 * 192.0 + 1.0);
        }

        // 3. 凸多边形便捷接口（auto fan；世界坐标顶点）。
        let triangle = [
            glam::Vec2::new(-80.0, -60.0),
            glam::Vec2::new(80.0, -60.0),
            glam::Vec2::new(0.0, 100.0),
            glam::Vec2::new(220.0, 200.0),
            glam::Vec2::new(280.0, 100.0),
        ];
        render2d.add_polygon_fan(&triangle, Color::CYAN, 96.0);

        // 3b. 通用 Mesh（显式索引；四边形 4 顶点 2 三角形）。
        let quad_verts = [
            glam::Vec2::new(-45.0, 180.0),
            glam::Vec2::new(45.0, 180.0),
            glam::Vec2::new(45.0, 260.0),
            glam::Vec2::new(-45.0, 260.0),
        ];
        render2d.add_mesh(
            &quad_verts,
            &[0, 1, 2,
              0, 2, 3],
            Color::PURPLE,
            96.0,
        );

        // 4. 左上 UI 面板（最大层级，最上层）—— 世界坐标左上角 (-half_w+10, -half_h+10)。
        let ui_tl = glam::Vec2::new(-half_w + 10.0, -half_h + 10.0);
        render2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(ui_tl, glam::Vec2::new(220.0, 60.0)),
            Color::rgba(0.1, 0.1, 0.1, 0.8),
            Transform2D::default(),
            100.0,
        );

        if let Some(w) = ctx.primary_window() {
            w.set_title(&format!(
                "FPS: {:.02}; zoom: {:.02}",
                ctx.timer.get_fps(),
                self.cam.zoom.x
            ));
        }

        // 演示 render(&ClearConfig)：可选 Clear color / depth / stencil。
        let clear = ClearConfig {
            color: Some(wgpu::Color { r: 0.13, g: 0.13, b: 0.19, a: 1.0 }),
            depth: None,   // 需要深度纹理时在此传入 Some(1.0)
            stencil: None,
        };
        // 全流程：begin_frame → 内部创建 RenderPass（按 clear）→ draw → submit/present。
        render2d.render(&clear);
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    log::info!("APP: {}", *rjw_main::PRIMARY_WINDOW_TITLE);
    run_app(SpriteDemo::new())
}