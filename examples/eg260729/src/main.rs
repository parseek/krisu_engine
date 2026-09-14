//! eg260729 —— 最小清屏示例（重设计后的写法）。
//!
//! 与 `egHello` 的区别：本示例**不做任何绘制**，只用配置里的清屏颜色把整帧刷成
//! 随滚轮变化的颜色，用来验证「运行时接管 begin_pass / present」这条路径。
//!
//! 要点：
//! - 不调用 `submit` 时，帧尾会用 `AppConfig::clear` 自动开 pass、清屏并 present；
//! - `Ctx::frame()` 取不到表面（最小化 / 遮挡）时 `None`，渲染代码一行不执行；
//! - 需要原生 wgpu pass 时用 `f.escape()`（逃生口）。

use rjw_krusie::prelude::*;

#[derive(Default)]
struct ClearScreen {
    /// 滚轮累积的色调偏移（验证鼠标输入）。
    hue: f32,
}

impl App for ClearScreen {
    fn config(&self) -> AppConfig {
        AppConfig::new("eg260729 - Clear Screen")
            .size(1280.0, 720.0)
            .clear(Color::rgb(0.10, 0.20, 0.40))
    }

    fn update(&mut self, ctx: &mut Ctx) {
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }

        // 滚轮：`ScrollDelta` 可直接换算为像素。
        let (_, dy) = ctx.mouse().wheel().to_pixel();
        self.hue += dy as f32 * 0.03;

        let Some(mut f) = ctx.frame() else { return };

        // 只清屏（不提交任何绘制）：显式提交一个带颜色的 pass。
        f.submit(
            &mut Camera2D::full(f.region().size()),
            Clear::color(Color::rgb(0.10, 0.20, 0.40 + self.hue.clamp(-0.2, 0.5))),
        );

        // FPS / 输入诊断（低层逃生口；取到帧之后一律经 `f` 访问只读面）
        if let Some(w) = f.window_handle() {
            let (mx, my) = f.mouse().wheel().to_pixel();
            w.set_title(&format!("FPS: {:.02}; wheel: ({mx:.1}, {my:.1})", f.fps()));
        }
    }
}

fn main() -> Result<(), RunError> {
    env_logger::init();
    run(ClearScreen::default())
}
