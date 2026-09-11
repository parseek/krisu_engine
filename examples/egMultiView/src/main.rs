//! egMultiView —— 多画面（左右分屏 + 画中画）示例（重设计后的写法）。
//!
//! 本示例是 **B1（多画面 VP 串味）** 的验收载体：
//!
//! 旧实现只有一个 offset-0 的 VP uniform + 一个 bind group，`set_mvp` 在 CPU 侧立即
//! `write_buffer`，而所有 pass 直到 `present` 才进同一个 submit ⇒ 一帧内三个相机
//! **全部读到最后一个矩阵**（三分屏其实同投影）。
//!
//! 现在**每个 pass 在帧级 VP 槽环里占一个独立槽**并通过动态偏移绑定：
//!
//! 1. 一个画面 = 一次 `f.submit(&mut cam, clear)` = 一个 pass = 一个 VP 槽；
//! 2. `f.set_region(rect)` 切换画面矩形，`submit` 把它写回相机（`Camera2D` 自己存视口）；
//! 3. 画面不重叠 ⇒ 共享帧级深度附件、不必清深度；重叠（画中画）⇒ 独立 pass 清深度
//!    （`Clear::color_depth(..)`：保留已绘制的颜色、只清深度）。
//!
//! 操作：`Esc` 退出。

use rjw_krusie::prelude::*;

#[derive(Default)]
struct MultiViewApp {
    /// 左 / 右分屏相机 + 右上角画中画（小地图）相机。
    cam_left: Camera2D,
    cam_right: Camera2D,
    cam_mini: Camera2D,
    /// 累计时间（驱动旋转标记，证明两个画面用的是同一份场景）。
    t: f32,
}

impl MultiViewApp {
    /// 依据当前画面矩形算出三个画面的屏幕矩形（左上原点、像素）。
    fn layout(&mut self, window: Rect) -> (Rect, Rect, Rect) {
        let (w, h) = (window.size().x, window.size().y);
        let half = w * 0.5;
        let left = Rect::new(0.0, 0.0, half, h);
        let right = Rect::new(half, 0.0, half, h);
        let mw = w * 0.28;
        let mh = h * 0.34;
        let mini = window.inset(Rect::new(w - mw - 12.0, 12.0, mw, mh));
        (left, right, mini)
    }
}

/// 同一份世界内容（三个画面各自调用一次：画面 = 一次 submit）。
fn draw_world(r2d: &mut Render2D, t: f32) {
    // 背景板
    r2d.solid(SpriteRect::new((-420.0, -260.0), (840.0, 520.0)))
        .tint(Color::rgba_u8(26, 30, 42, 255))
        .layer(0.0);
    // 彩色方块网格
    for iy in 0..5 {
        for ix in 0..7 {
            let x = -360.0 + ix as f32 * 120.0;
            let y = -200.0 + iy as f32 * 100.0;
            let c = Color::rgba_u8(40 + ix as u8 * 26, 60 + iy as u8 * 32, 130, 255);
            r2d.solid(SpriteRect::new((x, y), (96.0, 76.0))).tint(c).layer(1.0);
        }
    }
    // 随时间旋转的标记（黄点，沿椭圆运动）
    let a = t * 1.6;
    let p = Vec2::new(a.cos() * 280.0, a.sin() * 160.0);
    r2d.solid(SpriteRect::new(p - Vec2::splat(16.0), (32.0, 32.0)))
        .tint(Color::YELLOW)
        .layer(2.0);
}

impl App for MultiViewApp {
    fn config(&self) -> AppConfig {
        AppConfig::new("egMultiView — 多画面（左右分屏 + 画中画）")
            .size(1280.0, 720.0)
            // 调试辅助：给每个画面画一圈彩色描边 + 左上角 `#序号 宽×高`，
            // 一眼看出三块画面（左 / 右 / 画中画）的边界与归属。
            .viewport_borders(ViewportBorders::On)
    }

    fn update(&mut self, ctx: &mut Ctx) {
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }
        self.t += ctx.dt();

        let Some(mut f) = ctx.frame() else { return };

        let window = f.region();
        let (left, right, mini) = self.layout(window);

        // 左 / 右分屏：各占半屏（互不重叠 ⇒ 可共享同一份深度附件）。
        self.cam_left.set_region(left);
        // 画中画小地图：与右侧画面**重叠** ⇒ 独立 pass 清深度。
        self.cam_right.set_region(right);
        self.cam_mini.set_region(mini);
        // 小地图视野更大（缩小），一眼看清「同一份世界、不同相机」。
        self.cam_mini.set_zoom(Vec2::splat(0.5));
        self.cam_right.set_zoom(Vec2::splat(1.6));
        self.cam_right.transform.pos = Vec2::new(120.0, 60.0);

        // ── 画面 1：左半屏 ──
        f.set_region(left);
        draw_world(f.draw(), self.t);
        f.submit(&mut self.cam_left, Clear::color(Color::rgb(0.07, 0.09, 0.13)));

        // ── 画面 2：右半屏（同一份世界、另一个相机；VP 槽独立 ⇒ 投影正确）──
        f.set_region(right);
        draw_world(f.draw(), self.t);
        f.submit(&mut self.cam_right, Clear::color(Color::rgb(0.10, 0.07, 0.09)));

        // ── 画面 3：画中画小地图（与画面 2 重叠；保留颜色、只清深度）──
        f.set_region(mini);
        draw_world(f.draw(), self.t);
        f.submit(&mut self.cam_mini, Clear::depth(1.0));
    }
}

fn main() -> Result<(), EventLoopError> {
    // 日志：`RUST_LOG=rjw_krusie=debug` 可看到每个画面的矩形与清屏意图（多画面排查用）。
    env_logger::init();
    run(MultiViewApp::default())
}
