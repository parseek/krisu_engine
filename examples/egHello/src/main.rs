//! egHello —— 重设计后的最小完整应用（`docs/API_DESIGN.md` 附录 A 的落地版）。
//!
//! 演示的要点：
//!
//! 1. **一行起步**：`use rjw_krusie::prelude::*;` —— 窗口 / 相机 / 输入 / 帧 / 清屏 /
//!    提交 / present 全部由运行时接管；
//! 2. **守卫在应用里**：`let Some(f) = ctx.frame() else { return };` —— 取不到表面
//!    （最小化 / 遮挡 / 超时）时渲染代码**一行都不执行**，但 `update` 照常跑（后台模拟）；
//! 3. **相机自己存视口**：`Camera2D { region, transform }`，提交时把画面矩形写回相机；
//! 4. **入口 ≤2 参**：`f.submit(&mut cam, Clear::color(..))`；
//! 5. **状态糖收对象**：`.blend(BlendMode::Additive)` / `.depth(DepthState::…)`，无裸 bool。

use std::sync::LazyLock;

use rjw_krusie::prelude::*;

#[derive(Default)]
struct Hello {
    /// 相机：`region`（矩形区域）+ `transform`（2D 变换）。
    cam: Camera2D,
    /// 累计时间（驱动旋转）。
    t: f32,
    /// `--no-submit`：**故意不**调 `Frame::submit`，验证帧尾"自动清屏 + 提交"路径
    /// （`RUST_LOG=krusie=trace` 应出现"应用未提交本帧"；冒烟仍须有画面）。
    no_submit: bool,
}

const CIRCLE_VERTS: LazyLock<[Vec2; 96]> = LazyLock::new(|| {
    let mut verts = [Vec2::ZERO; 96];
    for i in 0..verts.len() {
        let a = i as f32 / verts.len() as f32 * std::f32::consts::TAU;
        verts[i] = Vec2::new(a.cos(), a.sin()) * 200.0;
    }
    verts
});

impl App for Hello {
    fn config(&self) -> AppConfig {
        AppConfig::new("egHello — krusie 一行起步").size(1280.0, 720.0)
    }

    fn update(&mut self, ctx: &mut Ctx) {
        let no_submit = self.no_submit;
        // ── 逻辑半程：无帧也执行（后台模拟 / 计时 / 输入状态持续）──
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }
        let (dt, fps) = (ctx.dt(), ctx.fps());
        self.t += dt;

        // ── 渲染半程：守卫在应用里 ──
        let Some(mut f) = ctx.frame() else { return };

        // 相机：把画面矩形写回相机（相机自己存视口），随后可用它做屏幕 ↔ 世界换算。
        self.cam.set_region(f.region());
        let cursor = self.cam.screen_to_world(f.mouse().pos_px());

        // 位姿：`Camera2D` 经 Deref 直接可用 `Transform2D` 的方法。
        let pan = Vec2::new(
            key_axis(&f, KeyCode::KeyD, KeyCode::KeyA),
            key_axis(&f, KeyCode::KeyS, KeyCode::KeyW),
        );
        self.cam.move_local(pan * 400.0 * dt);
        let zoom_dir = key_axis(&f, KeyCode::KeyE, KeyCode::KeyQ);
        if zoom_dir != 0.0 {
            self.cam.set_zoom(self.cam.zoom() * Vec2::splat(1.0 + zoom_dir * dt));
        }

        // 世界：旋转方块（纯色）+ 流式多边形
        // 状态统一走对象糖：`.tint(..)` / `.blend(..)` / `.depth(..)`（无裸 bool）。
        f.draw()
            .solid(SpriteRect::new((-50.0, -50.0), (100.0, 100.0)))
            .at(Vec2::ZERO)
            .rot(self.t)
            .tint(Color::GREEN)
            .blend(BlendMode::Additive)
            .depth(DepthState::test_write(CompareFunc::LessEq))
            .layer(0.0);

        f.draw()
            .polygon_with(|p| {
                for v in CIRCLE_VERTS.iter() {
                    p.vertex(*v);
                }
            })
            .tint(Color::rgba(0.0, 1.0, 1.0, 0.35))
            .layer(1.0);

        // 光标处的小方块（世界坐标）
        f.draw()
            .solid(SpriteRect::new((-8.0, -8.0), (16.0, 16.0)))
            .at(cursor)
            .tint(Color::YELLOW.with_a(0.4))
            .blend(BlendMode::Additive)
            .layer(2.0);

        // 窗口标题显示帧率（低层逃生口：经 `Frame` 的 Deref 取 winit 窗口句柄）
        if let Some(win) = f.window_handle() {
            win.set_title(&format!("egHello — FPS {fps:.0}  dt {:.1}ms", dt * 1000.0));
        }

        // ── 提交：一个画面 = 一次 submit(相机, clear) ──
        // （`--no-submit` 只作验证用：跳过显式提交，帧尾应走"自动清屏 + 提交"路径。）
        if !no_submit {
            f.submit(&mut self.cam, Clear::color(Color::rgb(0.10, 0.11, 0.16)));
        }
        // 不调用 present 也行：`Frame` 析构会自动 present。
    }
}

/// 键位轴：`plus` 按下记 +1，`minus` 按下记 -1。
fn key_axis(f: &Frame<'_>, plus: KeyCode, minus: KeyCode) -> f32 {
    (f.key(plus).pressed() as i32 - f.key(minus).pressed() as i32) as f32
}

fn main() -> Result<(), RunError> {
    env_logger::init();
    let no_submit = std::env::args().any(|a| a == "--no-submit");
    run(Hello { no_submit, ..Hello::default() })
}
