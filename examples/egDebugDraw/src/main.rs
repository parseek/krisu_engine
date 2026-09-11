//! egDebugDraw —— DebugDraw + Debug UI 示例（新运行时驱动）。
//!
//! 展示：
//! - **调试 rjw_ui 自身**（`rjw_ui` 的 Debug UI）：`debug_layout` 开关 —— 每个控件 /
//!   容器的**布局矩形与命中区域**画青色描边（覆盖在 UI 内容之上）；面板勾选实时切换。
//! - **rjw_ui 的 DebugDraw**（屏幕空间）：[`Ui::debug_line`] / [`Ui::debug_circle_outline`] /
//!   [`Ui::debug_cross`] —— 鼠标十字 + 跟随圆圈（绝对逻辑屏幕像素，覆盖在 UI 之上）。
//! - **世界 DebugDraw**（[`rjw_krusie::render2d::debug_draw`]）：网格（`DebugPainter::grid`）、
//!   碰撞盒（`DebugPainter::rect`）、圆形轮廓与实心圆（`circle` / `disc`）、点标记（`cross`）、
//!   速度矢量（`line`）——一份 `DebugStyle`（颜色 / 线宽 / 层级 / 圆分段数）+ 一个画笔，
//!   同款样式的图元共用一次 `r2d.debug(..)`；世界坐标，用于游戏场景调试。
//!
//! 运行时接管（旧→新）：
//! - 窗口 / 渲染上下文 / 双 `Render2D`（世界层 + UI 层）/ 文本子系统 / 单 pass 合并提交
//!   全部由 [`rjw_krusie::prelude::run`] 与 [`Frame::submit`] 拥有——应用不再持
//!   `RenderContext` / `Render2D` / `Text` / `Viewport`，也没有 `begin_pass` / `record`。
//! - UI 由 [`Frame::ui`] 接管 `Ui::begin / capture / theme / scale_factor / finish`
//!   （UI 渲染器排序已由引擎设为 `SortMode::None`），应用只写闭包。
//!
//! 操作：`F1` 开关调试面板 · 拖动调试窗口 · `Esc` 退出。

use rjw_krusie::prelude::*;
use rjw_krusie::render2d::debug_draw::DebugStyle;

/// DebugDraw 覆盖层的基准层级（世界场景之上）。
const LAYER_DEBUG: f64 = 5_000.0;

/// 场景中的障碍物（实心矩形 + 碰撞盒）。
struct Obstacle {
    rect: Rect,
}

struct DebugApp {
    /// 相机：`region`（画面矩形，提交时由运行时写回）+ `transform`（位姿）。
    cam: Camera2D,
    // ── 调试面板状态 ──
    debug_visible: bool,
    show_hitboxes: bool,
    show_grid: bool,
    /// 调试 rjw_ui 自身：给每个控件/容器的布局矩形画青色描边（debug_layout）。
    ui_debug_layout: bool,
    /// 屏幕空间调试图元开关（rjw_ui 的 DebugDraw）。
    ui_debug_shapes: bool,
    line_width: f32,
    // ── 场景状态 ──
    ball_pos: Vec2,
    ball_vel: Vec2,
    ball_radius: f32,
    obstacles: Vec<Obstacle>,
    /// 视口边界（世界坐标，球反弹范围）。
    world: Rect,
}

impl DebugApp {
    fn new() -> Self {
        Self {
            cam: Camera2D::default(),
            debug_visible: true,
            show_hitboxes: true,
            show_grid: true,
            ui_debug_layout: false,
            ui_debug_shapes: true,
            line_width: 2.0,
            ball_pos: Vec2::new(0.0, 0.0),
            ball_vel: Vec2::new(320.0, 240.0),
            ball_radius: 24.0,
            obstacles: vec![
                Obstacle { rect: Rect::new(-420.0, -180.0, 160.0, 90.0) },
                Obstacle { rect: Rect::new(260.0, 40.0, 200.0, 120.0) },
                Obstacle { rect: Rect::new(-80.0, 200.0, 140.0, 80.0) },
            ],
            world: Rect::new(-640.0, -360.0, 1280.0, 720.0),
        }
    }
}

impl App for DebugApp {
    fn config(&self) -> AppConfig {
        AppConfig::new("egDebugDraw — DebugDraw + Debug UI 示例").size(1280.0, 720.0)
    }

    fn update(&mut self, ctx: &mut Ctx) {
        // ── 逻辑半程：无帧也执行（输入 / 计时 / 模拟） ──────────
        // 注：新运行时拥有 `UiState`（不再暴露给应用），旧代码里 `capturing_text()`
        // 屏蔽快捷键的做法不可用；本示例面板没有文本输入框，故无实际影响。
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }
        if ctx.key(KeyCode::F1).down_edge() {
            self.debug_visible = !self.debug_visible;
        }

        // ── 场景更新：球匀速运动 + 视口反弹 ─────────────────────
        let dt = ctx.dt().min(0.05);
        self.ball_pos += self.ball_vel * dt;
        let half = self.world.w * 0.5 - self.ball_radius;
        let half_h = self.world.h * 0.5 - self.ball_radius;
        if self.ball_pos.x < -half || self.ball_pos.x > half {
            self.ball_vel.x = -self.ball_vel.x;
            self.ball_pos.x = self.ball_pos.x.clamp(-half, half);
        }
        if self.ball_pos.y < -half_h || self.ball_pos.y > half_h {
            self.ball_vel.y = -self.ball_vel.y;
            self.ball_pos.y = self.ball_pos.y.clamp(-half_h, half_h);
        }

        // ── 渲染半程：守卫在应用里 ─────────────────────────────
        let Some(mut f) = ctx.frame() else { return };

        // 画面矩形写回相机（相机自己存视口）；世界边界跟随视口（球在视口内反弹）。
        let region = f.region();
        self.cam.set_region(region);
        self.world = Rect::new(-region.w * 0.5, -region.h * 0.5, region.w, region.h);

        // 取 `f` 上的只读事实（闭包内不能再借用 `f`）。
        let fps = f.fps();
        let mouse = f.mouse().pos_px();

        // ── 世界层：背景 + 障碍物（实心矩形）+ 球本体 ───────────
        {
            let r2d = f.draw();
            let world_tf = Transform2D::default();
            r2d.solid(SpriteRect::new(
                (-self.world.w * 0.5, -self.world.h * 0.5),
                (self.world.w, self.world.h),
            ))
            .tint(Color::rgba_u8(22, 26, 36, 255))
            .transform(world_tf)
            .layer(0.0);
            for (i, o) in self.obstacles.iter().enumerate() {
                r2d.solid(SpriteRect::new((o.rect.x, o.rect.y), (o.rect.w, o.rect.h)))
                    .tint(Color::rgba_u8(44 + i as u8 * 16, 58, 92, 255))
                    .transform(world_tf)
                    .layer(1.0);
            }
            // 球本体（实心圆 = 三角扇；`disc` 不用线宽，只取颜色 / 层级 / 分段数）。
            r2d.debug(
                DebugStyle::new(Color::rgba_u8(120, 190, 120, 255))
                    .segments(48)
                    .layer(LAYER_DEBUG + 1.0),
            )
            .disc(self.ball_pos, self.ball_radius);

            // ── DebugDraw 覆盖层（世界坐标；开关由 Debug UI 控制） ──
            let w = self.line_width;
            if self.show_grid {
                r2d.debug(
                    DebugStyle::new(Color::rgba_u8(90, 100, 120, 90))
                        .width(w * 0.5)
                        .layer(LAYER_DEBUG),
                )
                .grid(self.world, 80.0);
            }
            if self.show_hitboxes {
                // 视口边界
                r2d.debug(
                    DebugStyle::new(Color::rgba_u8(140, 160, 200, 160))
                        .width(w)
                        .layer(LAYER_DEBUG + 1.0),
                )
                .rect(self.world);
                // 障碍物碰撞盒：同款样式（黄 / 同线宽 / 同层级）共用一个画笔批量提交
                {
                    let mut p =
                        r2d.debug(DebugStyle::new(Color::YELLOW).width(w).layer(LAYER_DEBUG + 1.0));
                    for o in &self.obstacles {
                        p.rect(o.rect);
                    }
                }
                // 球：轮廓 + 中心十字 + 速度矢量
                r2d.debug(
                    DebugStyle::new(Color::GREEN)
                        .width(w)
                        .segments(48)
                        .layer(LAYER_DEBUG + 1.0),
                )
                .circle(self.ball_pos, self.ball_radius);
                r2d.debug(
                    DebugStyle::new(Color::WHITE)
                        .width(w)
                        .layer(LAYER_DEBUG + 1.0),
                )
                .cross(self.ball_pos, 8.0);
                let vel_end = self.ball_pos + self.ball_vel * 0.1;
                r2d.debug(DebugStyle::new(Color::RED).width(w).layer(LAYER_DEBUG + 1.0))
                    .line(self.ball_pos, vel_end);
            }
        }

        // ── Debug UI 层（rjw_ui 调试窗口；F1 开关） ─────────────
        // `Frame::ui` 接管 begin / capture / theme / scale_factor / finish，
        // UI 渲染器为引擎第二个 `Render2D`（排序已关闭）。
        f.ui(Theme::dark(), |ui| {
            // 调试 rjw_ui 自身：布局矩形 / 命中区域描边（本帧生效；`_layout()` / `without_` 收枚举语义）。
            if self.ui_debug_layout {
                ui.debug_layout();
            } else {
                ui.without_debug_layout();
            }

            if self.debug_visible {
                ui.window("debug_panel").pos(Vec2::new(24.0, 24.0)).show(|w| {
                    w.label("Debug 面板（F1 关闭）");
                    w.label(&format!("FPS: {fps:.0}"));
                    w.label(&format!(
                        "球: ({:.0}, {:.0})  vel=({:.0}, {:.0})",
                        self.ball_pos.x, self.ball_pos.y, self.ball_vel.x, self.ball_vel.y
                    ));
                    if w.checkbox("dbg_hitbox", "显示碰撞盒", self.show_hitboxes).toggled() {
                        self.show_hitboxes = !self.show_hitboxes;
                    }
                    if w.checkbox("dbg_grid", "显示网格", self.show_grid).toggled() {
                        self.show_grid = !self.show_grid;
                    }
                    if w.checkbox("dbg_ui_layout", "调试 UI 布局", self.ui_debug_layout).toggled() {
                        self.ui_debug_layout = !self.ui_debug_layout;
                    }
                    if w.checkbox("dbg_ui_shapes", "屏幕调试图元", self.ui_debug_shapes).toggled() {
                        self.ui_debug_shapes = !self.ui_debug_shapes;
                    }
                    self.line_width = w.slider("dbg_width", 1.0..=6.0, self.line_width);
                    w.label(&format!("线宽: {:.1}px", self.line_width));
                });
            }
            ui.label_at(
                Vec2::new(16.0, 690.0),
                &format!(
                    "F1 开关调试面板 · 拖动调试窗口 · {} · Esc 退出",
                    if self.debug_visible { "勾选切换调试图元" } else { "调试面板已隐藏" }
                ),
            );

            // ── rjw_ui 的 DebugDraw（屏幕空间；物理像素，覆盖在 UI 之上） ──
            if self.ui_debug_shapes {
                // 鼠标十字 + 跟随圆圈
                ui.debug_cross(mouse, 10.0, 1.5, Color::ORANGE);
                ui.debug_circle_outline(mouse, 24.0, 40, 1.5, Color::ORANGE);
                // 屏幕中心 → 鼠标 连线
                let center = Vec2::new(region.w * 0.5, region.h * 0.5);
                ui.debug_line(center, mouse, 1.0, Color::rgba_u8(255, 200, 100, 200));
                // 调试面板矩形框（若面板可见）
                if self.debug_visible {
                    ui.debug_rect_outline(Rect::new(24.0, 24.0, 190.0, 240.0), 1.5, Color::MAGENTA);
                }
            }
        });

        // ── 提交：世界层（含 DebugDraw）与 UI 层进同一个 pass，一次 present ──
        f.submit(&mut self.cam, Clear::color(Color::rgb(0.08, 0.09, 0.12)));
    }
}

fn main() -> Result<(), EventLoopError> {
    run(DebugApp::new())
}
