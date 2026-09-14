//! eg260731 —— Render2D 精灵 / 多边形 / 网格演示（重设计后的写法）。
//!
//! 与旧版对比，本示例不再手写：`RenderContext` / `Render2D::new` / `on_resized` 里的
//! `resize` + `set_vp` / `set_mvp` / `ClearConfig` 字面量 / `begin_frame` / `present`。
//! 这些全部由 `rjw_krusie::runtime` 接管，示例只剩「逻辑 + 绘制 + 提交」。

use rjw_krusie::prelude::*;

#[derive(Default)]
struct SpriteDemo {
    cam: Camera2D,
    tex: Option<ArcTextureWrapped>,
    /// 累计经过时间（秒），驱动精灵动画。
    t: f32,
}

/// 生成 16×16 棋盘格 RGBA8（黄 / 白）。
fn checker_rgba(w: u32, h: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let check = ((x / 8 + y / 8) % 2) == 0;
            if check {
                data.extend_from_slice(&[255, 200, 0, 255]); // 黄色
            } else {
                data.extend_from_slice(&[255, 255, 255, 255]); // 白色
            }
        }
    }
    data
}

impl App for SpriteDemo {
    fn config(&self) -> AppConfig {
        AppConfig::new("eg260731 - Render2D Sprites").size(1280.0, 720.0)
    }

    fn init(&mut self, gfx: &Gfx) -> Result<(), AppInitError> {
        let px = checker_rgba(16, 16);
        self.tex = Some(gfx.texture("checkboard", Rgba8::new(&px, (16, 16))));
        Ok(())
    }

    fn update(&mut self, ctx: &mut Ctx) {
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }

        let dt = ctx.dt();
        self.t += dt;

        // 鼠标滚轮缩放、方向键平移（坐标系 X+ 右、Y+ 下：W ↔ 上/Y-，S ↔ 下/Y+）。
        let wheel = ctx.mouse().wheel().to_pixel();
        if wheel.1 != 0.0 {
            let zoom = self.cam.zoom() * Vec2::splat(1.1_f32.powf(wheel.1 as f32 * 0.01));
            self.cam.set_zoom(zoom.clamp(Vec2::splat(0.01), Vec2::splat(100.0)));
        }
        let move_speed = 400.0 * dt;
        self.cam.move_local(Vec2::new(
            axis(ctx, KeyCode::KeyD, KeyCode::KeyA) * move_speed,
            axis(ctx, KeyCode::KeyS, KeyCode::KeyW) * move_speed,
        ));
        self.cam.transform.rotation += axis(ctx, KeyCode::KeyE, KeyCode::KeyQ) * dt * 180.0_f32.to_radians();

        let t = self.t;

        // ── 渲染半程（无帧不执行）──
        let Some(mut f) = ctx.frame() else { return };
        self.cam.set_region(f.region());

        let size = f.region().size();
        let (half_w, half_h) = (size.x * 0.5, size.y * 0.5);
        let axis_len = half_w.max(half_h) * 1.05;

        // ── 坐标系指示线（验证 X+ 右 / Y+ 下）──
        f.draw()
            .solid(SpriteRect::new((0.0, -2.0), (axis_len, 4.0)))
            .tint(Color::RED)
            .layer(95.0); // X+ 右（红）
        f.draw()
            .solid(SpriteRect::new((-axis_len, -2.0), (axis_len, 4.0)))
            .tint(Color::rgba(0.5, 0.0, 0.0, 1.0))
            .layer(95.0); // X- 左（暗红）
        f.draw()
            .solid(SpriteRect::new((-2.0, 0.0), (4.0, axis_len)))
            .tint(Color::GREEN)
            .layer(95.0); // Y+ 下（绿）
        f.draw()
            .solid(SpriteRect::new((-2.0, -axis_len), (4.0, axis_len)))
            .tint(Color::rgba(0.0, 0.4, 0.0, 1.0))
            .layer(95.0); // Y- 上（暗绿）

        // 1. 带纹理的精灵（棋盘格），绕中心旋转 + 缩放。
        if let Some(tex) = &self.tex {
            let rect = SpriteRect::new((-96.0, -96.0), (192.0, 192.0));
            let tf = Transform2D::default()
                .with_pos(Vec2::new(0.0, 0.0))
                .with_rot(t * 0.8)
                .with_scale(Vec2::splat(1.0 + 0.2 * t.sin()));
            f.draw().sprite(rect, tex).tint(Color::WHITE).transform(tf);
        }

        // 2. 纯色矩形阵列（不同层级 / 颜色 / 旋转）。
        const COUNTW: i32 = 28;
        const COUNTH: i32 = 28;
        for i in 0..COUNTW * COUNTH {
            let center = Vec2::new(
                ((i % COUNTW) as f32 - (COUNTW as f32 - 1.0) * 0.5) * 80.0,
                (t * 0.5 + (i / COUNTH) as f32).sin() * 20.0
                    + ((i / COUNTH) as f32 - (COUNTH as f32 - 1.0) * 0.5) * 50.0,
            );
            let rect = SpriteRect::new((-40.0, -40.0), (80.0, 80.0));
            let tf = Transform2D::default().with_pos(center).with_rot(t * 0.3 + i as f32);
            let i_mapped = i as f32 / (COUNTW * COUNTH) as f32;
            let color = Color::rgba(0.3 + (i_mapped * 0.7), 0.6, 0.9 - i_mapped * 0.7, 0.7);
            f.draw()
                .solid(rect)
                .tint(color)
                .transform(tf)
                .layer((i % COUNTW) as f32 / COUNTW as f32 * 192.0 + 1.0);
        }

        // 3. 凸多边形便捷接口（auto fan；世界坐标顶点）。
        let triangle = [
            Vec2::new(-80.0, -60.0),
            Vec2::new(80.0, -60.0),
            Vec2::new(0.0, 100.0),
            Vec2::new(220.0, 200.0),
            Vec2::new(280.0, 100.0),
        ];
        f.draw().polygon(&triangle).tint(Color::CYAN).layer(96.0);

        // 3b. 通用 Mesh（显式索引；四边形 4 顶点 2 三角形）。
        let quad_verts = [
            Vec2::new(-45.0, 180.0),
            Vec2::new(45.0, 180.0),
            Vec2::new(45.0, 260.0),
            Vec2::new(-45.0, 260.0),
        ];
        f.draw()
            .mesh(&quad_verts, &[0, 1, 2, 0, 2, 3])
            .tint(Color::PURPLE)
            .layer(96.0);

        // 4. 左上 UI 面板（最大层级，最上层）—— 世界坐标左上角 (-half_w+10, -half_h+10)。
        let ui_tl = Vec2::new(-half_w + 10.0, -half_h + 10.0);
        f.draw()
            .solid(SpriteRect::new(ui_tl, (220.0, 60.0)))
            .tint(Color::rgba(0.1, 0.1, 0.1, 0.8))
            .layer(100.0);

        if let Some(w) = f.window_handle() {
            w.set_title(&format!("FPS: {:.02}; zoom: {:.02}", f.fps(), self.cam.zoom().x));
        }

        // 提交：一个画面 = 一次 submit(相机, clear)。
        f.submit(&mut self.cam, Clear::color(Color::rgb(0.13, 0.13, 0.19)));
    }
}

/// 键位轴：`plus` 按下记 +1，`minus` 按下记 -1。
fn axis(ctx: &Ctx, plus: KeyCode, minus: KeyCode) -> f32 {
    (ctx.key(plus).pressed() as i32 - ctx.key(minus).pressed() as i32) as f32
}

fn main() -> Result<(), RunError> {
    env_logger::init();
    run(SpriteDemo::default())
}