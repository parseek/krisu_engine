//! egDebugDraw —— DebugDraw + Debug UI 示例。
//!
//! 展示：
//! - **调试 rjw_ui 自身**（`rjw_ui` 的 Debug UI）：`debug_layout` 开关 —— 每个控件 /
//!   容器的**布局矩形与命中区域**画青色描边（覆盖在 UI 内容之上）；面板勾选实时切换。
//! - **rjw_ui 的 DebugDraw**（屏幕空间）：[`Ui::debug_line`] / [`Ui::debug_circle_outline`] /
//!   [`Ui::debug_cross`] —— 鼠标十字 + 跟随圆圈（绝对逻辑屏幕像素，覆盖在 UI 之上）。
//! - **世界 DebugDraw**（[`rjw_2d_render::debug_draw`]）：网格（`draw_grid`）、碰撞盒
//!   （`draw_rect_outline`）、圆形轮廓与实心圆、点标记（`draw_cross`）、速度矢量
//!   （`draw_line`）——世界坐标，用于游戏场景调试。
//! - 独立 UI 渲染器 + 合并提交（与 `eg260818UI` 相同的世界 → UI 一次 present）。
//!
//! 操作：`F1` 开关调试面板 · 拖动调试窗口 · `Esc` 退出。

use glam::Vec2;
use rjw_2d_render::{
    debug_draw::{
        draw_circle_filled, draw_circle_outline, draw_cross, draw_grid, draw_line,
        draw_rect_outline,
    },
    ClearConfig, Render2D, SpriteRect,
};
use rjw_color::Color;
use rjw_main::*;
use rjw_render::{wgpu, RenderConfig, RenderContext};
use rjw_text::Text;
use rjw_transform::{Rect, Transform2D, Viewport};
use rjw_ui::{Theme, Ui, UiAdd, UiState};

const LAYER_UI: f64 = 10_000_000.0;
/// DebugDraw 覆盖层的基准层级（世界场景之上、UI 之下）。
const LAYER_DEBUG: f64 = 5_000.0;

/// 场景中的障碍物（实心矩形 + 碰撞盒）。
struct Obstacle {
    rect: Rect,
}

struct DebugApp {
    render: Option<RenderContext>,
    render2d: Option<Render2D>,
    render2d_ui: Option<Render2D>,
    font: Option<Text>,
    viewport: Viewport,
    ui_state: UiState,
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
            render: None,
            render2d: None,
            render2d_ui: None,
            font: None,
            viewport: Viewport::new(Vec2::new(1280.0, 720.0), Vec2::ZERO),
            ui_state: UiState::new(),
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
    fn primary_window_attrib(&self) -> WindowAttributes {
        WindowAttributes::default()
            .with_title("egDebugDraw — DebugDraw + Debug UI 示例")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
    }

    fn on_init(&mut self, ctx: &mut MainContext) {
        let window = ctx.primary_window().expect("window");
        self.render = Some(RenderContext::new(window, &RenderConfig::default()));
        let render = self.render.as_ref().unwrap();
        let render2d = Render2D::new(render);
        // 独立 UI 渲染器：关闭 Render2D 排序（UI 自行管理绘制顺序）。
        let mut render2d_ui = Render2D::new(render);
        render2d_ui.set_sorting(false);
        let font = Text::new(render2d.device(), render2d.queue(), render2d.tex_bind_group_layout());
        let (w, h) = render.size();
        let viewport = Viewport::new(Vec2::new(w as f32, h as f32), Vec2::ZERO);
        self.render2d = Some(render2d);
        self.render2d_ui = Some(render2d_ui);
        self.font = Some(font);
        self.viewport = viewport;
    }

    fn on_resized(&mut self, _ctx: &mut MainContext, width: u32, height: u32) {
        if let Some(r) = &mut self.render {
            r.resize(width, height);
        }
        self.viewport = Viewport::new(Vec2::new(width as f32, height as f32), Vec2::ZERO);
        // 视口尺寸变化后世界边界跟随（球在视口内反弹）。
        self.world = Rect::new(
            -(width as f32) * 0.5,
            -(height as f32) * 0.5,
            width as f32,
            height as f32,
        );
    }

    fn about_to_wait(&mut self, ctx: &mut MainContext) {
        // ── 快捷键（输入框聚焦时屏蔽，见 eg260818UI） ──────────
        if !self.ui_state.capturing_text() {
            if ctx.keyboard.get(KeyCode::Escape).down_edge() {
                ctx.request_exit();
            }
            if ctx.keyboard.get(KeyCode::F1).down_edge() {
                self.debug_visible = !self.debug_visible;
            }
        }

        // ── 场景更新：球匀速运动 + 视口反弹 ─────────────────────
        let dt = ctx.timer.dt().get_f32().min(0.05);
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

        let Some(r2d) = &mut self.render2d else {
            return;
        };
        r2d.set_mvp(self.viewport.vp_matrix());
        let font = self.font.as_mut().unwrap();

        // ── 世界层：背景 + 障碍物（实心矩形） ───────────────────
        let world_tf = Transform2D::default();
        r2d.add_sprite2d_solid(
            SpriteRect::from_texture(
                Vec2::new(-self.world.w * 0.5, -self.world.h * 0.5),
                Vec2::new(self.world.w, self.world.h),
            ),
            Color::rgba_u8(22, 26, 36, 255),
            world_tf,
            0.0,
        );
        for (i, o) in self.obstacles.iter().enumerate() {
            r2d.add_sprite2d_solid(
                SpriteRect::from_texture(Vec2::new(o.rect.x, o.rect.y), Vec2::new(o.rect.w, o.rect.h)),
                Color::rgba_u8(44 + i as u8 * 16, 58, 92, 255),
                world_tf,
                1.0,
            );
        }
        // 球本体（实心圆 = 三角扇）。
        draw_circle_filled(
            r2d,
            self.ball_pos,
            self.ball_radius,
            48,
            Color::rgba_u8(120, 190, 120, 255),
            LAYER_DEBUG + 1.0,
        );

        // ── DebugDraw 覆盖层（世界坐标；开关由 Debug UI 控制） ──
        let w = self.line_width;
        if self.show_grid {
            draw_grid(
                r2d,
                &self.world,
                80.0,
                w * 0.5,
                Color::rgba_u8(90, 100, 120, 90),
                LAYER_DEBUG,
            );
        }
        if self.show_hitboxes {
            // 视口边界 + 障碍物碰撞盒
            draw_rect_outline(r2d, &self.world, w, Color::rgba_u8(140, 160, 200, 160), LAYER_DEBUG + 1.0);
            for o in &self.obstacles {
                draw_rect_outline(r2d, &o.rect, w, Color::YELLOW, LAYER_DEBUG + 1.0);
            }
            // 球：轮廓 + 中心十字 + 速度矢量
            draw_circle_outline(r2d, self.ball_pos, self.ball_radius, 48, w, Color::GREEN, LAYER_DEBUG + 1.0);
            draw_cross(r2d, self.ball_pos, 8.0, w, Color::WHITE, LAYER_DEBUG + 1.0);
            let vel_end = self.ball_pos + self.ball_vel * 0.1;
            draw_line(r2d, self.ball_pos, vel_end, w, Color::RED, LAYER_DEBUG + 1.0);
        }

        // ── Debug UI 层（rjw_ui 调试窗口；F1 开关） ─────────────
        let r2d_ui = self.render2d_ui.as_mut().unwrap();
        r2d_ui.set_mvp(self.viewport.vp_matrix());
        let window = ctx.primary_window().expect("window");
        let mut ui = Ui::begin(window, font, &mut self.ui_state)
            .capture(&ctx.mouse, &ctx.keyboard)
            .theme(Theme::dark())
            .base_layer(LAYER_UI)
            .scale_factor(ctx.scale_factor().unwrap_or(1.0))
            .build();
        // 调试 rjw_ui 自身：布局矩形 / 命中区域描边（本帧生效）。
        ui.debug_layout(self.ui_debug_layout);

        if self.debug_visible {
            ui.window_at("debug_panel", Vec2::new(24.0, 24.0), |w| {
                w.label("Debug 面板（F1 关闭）");
                w.label(&format!("FPS: {:.0}", ctx.timer.get_fps()));
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
            let (mx, my) = ctx.mouse.get_mouse_position();
            let mouse = Vec2::new(mx as f32, my as f32);
            // 鼠标十字 + 跟随圆圈
            ui.debug_cross(mouse, 10.0, 1.5, Color::ORANGE);
            ui.debug_circle_outline(mouse, 24.0, 40, 1.5, Color::ORANGE);
            // 屏幕中心 → 鼠标 连线
            let center = Vec2::new(640.0, 360.0);
            ui.debug_line(center, mouse, 1.0, Color::rgba_u8(255, 200, 100, 200));
            // 调试面板矩形框（若面板可见）
            if self.debug_visible {
                ui.debug_rect_outline(Rect::new(24.0, 24.0, 190.0, 240.0), 1.5, Color::MAGENTA);
            }
        }
        ui.finish(&self.viewport, r2d_ui);

        // ── 合并提交：世界（含 DebugDraw）→ UI → 一次 present ──
        let Some((surface_tex, view)) = r2d.begin_frame() else {
            return;
        };
        let cb_world = r2d.render_command_buffer(
            &ClearConfig {
                color: Some(wgpu::Color { r: 0.08, g: 0.09, b: 0.12, a: 1.0 }),
                depth: None,
                stencil: None,
            },
            &view,
            None,
        );
        let cb_ui = r2d_ui.render_command_buffer(
            &ClearConfig { color: None, depth: None, stencil: None },
            &view,
            None,
        );
        r2d.queue().submit([cb_world, cb_ui]);
        r2d.queue().present(surface_tex);
    }
}

fn main() -> Result<(), EventLoopError> {
    run_app(DebugApp::new())
}
