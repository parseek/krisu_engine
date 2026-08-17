//! eg260810TextChain —— `rjw_text` 责任链 API 演示。
//!
//! 展示：
//! - `Text::text(..)` 责任链：排版配置 → `into_render` / `try_stack` → 渲染设置 → 绘制
//! - `TextStyle` / `Style`：可复用样式（DRY 字体/字号/行距），克隆继承 `base.clone().size(..)`
//! - `draw_sprite2d` / `draw_with` / `draw_2d_gradient`（Glyph/Line/Frame × 横/竖）
//! - `map` 逐字形动画 + `glyph_str()` / `glyph_type`

use glam::{Vec2, vec2};
use rjw_2d_render::{ClearConfig, Render2D, SpriteRect};
use rjw_color::Color;
use rjw_main::*;
use rjw_render::{RenderConfig, RenderContext, TEXTURES, wgpu};
use rjw_text::{
    Align, GlyphData, GlyphType, GradientAxis, GradientMode, LineSpace, Style, Text, TextBuffer, TextStyle,
    Transform2D, cosmic_text,
};
use rjw_transform::Camera2D;

struct ChainDemo {
    render: Option<RenderContext>,
    render2d: Option<Render2D>,
    font: Option<Text>,
    cam: Camera2D,
    t: f32,
    /// 帧计数（FPS 统计）
    // frame: u64,
    /// FPS 统计窗口累计时间
    // fps_time: f32,
    /// FPS 统计窗口帧数
    // fps_frames: u32,
    /// 上次观测到的字形图集页数（变化时打印，用于验证“页未满却开新页”修复）
    last_page_count: usize,
}

impl ChainDemo {
    fn new() -> Self {
        Self {
            render: None,
            render2d: None,
            font: None,
            cam: Camera2D::new(Vec2::ZERO),
            t: 0.0,
            // frame: 0,
            // fps_time: 0.0,
            // fps_frames: 0,
            last_page_count: 0,
        }
    }
}

fn vec2_u32tup(t: (u32, u32)) -> glam::Vec2 {
    vec2(t.0 as f32, t.1 as f32)
}

impl App for ChainDemo {
    fn primary_window_attrib(&self) -> WindowAttributes {
        WindowAttributes::default()
            .with_title("eg260810TextChain - rjw_text 责任链")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
    }

    fn on_init(&mut self, ctx: &mut MainContext) {
        eprintln!("MARK: on_init");
        let window = ctx.primary_window().expect("primary window must exist during on_init");
        self.render = Some(RenderContext::new(window, &RenderConfig::default()));
        let render = self.render.as_ref().expect("render must be initialized");

        let render2d = Render2D::new(render);
        let (w, h) = render.size();
        self.cam = Camera2D::new(Vec2::new(w as f32, h as f32));

        self.font = Some(Text::new(
            render2d.device(),
            render2d.queue(),
            render2d.tex_bind_group_layout(),
        ));
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
        let dt = ctx.timer.dt().get_f32();
        self.t += dt;
        let t = self.t;
        // self.frame += 1;
        // self.fps_time += dt;
        // self.fps_frames += 1;
        // if self.fps_time >= 1.0 {
        //     eprintln!("FPS: {:.0} (frame {})", self.fps_frames as f32 / self.fps_time, self.frame);
        //     self.fps_time = 0.0;
        //     self.fps_frames = 0;
        // }

        let Some(r2d) = &mut self.render2d else { return };
        let Some(font) = &mut self.font else { return };
        r2d.set_mvp(self.cam.vp_matrix());

        draw_text_demos(r2d, font, t, self.cam.viewport_size.x * 0.5, self.cam.viewport_size.y * 0.5);

        let clear = ClearConfig {
            color: Some(wgpu::Color { r: 0.10, g: 0.11, b: 0.16, a: 1.0 }),
            depth: None,
            stencil: None,
        };
        r2d.render(&clear);

        // 字形图集页数变化时打印（修复前该示例会无谓地开多页）。
        let pages = font.glyph_cache().page_count();
        if pages != self.last_page_count {
            eprintln!("glyph atlas pages: {pages} (changed)");
            self.last_page_count = pages;
        }
    }
}

/// 责任链演示：六种用法。
fn draw_text_demos(r2d: &mut Render2D, font: &mut Text, t: f32, half_w: f32, half_h: f32) {
    // eprintln!("MARK: demos");
    // ── 1. TextStyle / Style：可复用样式，多处只写差异 ──
    {
        let mut ui = font
            .build_style()
            .font_family("SimHei")
            .size(16.0)
            .line_space(LineSpace::Multiple(1.4))
            .align(Align::Left)
            .color(Color::WHITE);
        ui.text("eg260810TextChain — rjw_text 责任链演示")
            .offset(Vec2::new(-half_w + 14.0, -half_h + 14.0))
            .draw_sprite2d(r2d, 100.0);
        ui.text("左上角：TextStyle 复用样式（克隆继承 base.clone().size(..)）")
            .offset(Vec2::new(-half_w + 14.0, -half_h + 40.0))
            .color(Color::YELLOW)
            .draw_sprite2d(r2d, 100.0);
    }

    // ── 1b. Style 责任链（与 Text 解耦） + with_style / set_style ──
    let base = Style::default()
        .font_family("SimHei")
        .size(16.0)
        .weight(cosmic_text::Weight::BOLD)
        .line_space(LineSpace::Multiple(1.4))
        .align(Align::Left)
        .color(Color::WHITE);
    let warn = base.clone().size(20.0).color(Color::RED);   // 克隆继承：只改差异
    let fancy = Style::default()
        .font_family("SimHei")
        .size(18.0)
        .italic(true)
        .letter_spacing(2.0)
        .color(Color::ORANGE);

    {
        let mut title = TextStyle::with_style(font, &warn);
        title.text("Style 责任链 → with_style(&warn)：weight+color 继承")
            .offset(Vec2::new(-half_w + 14.0, -half_h + 70.0).round())
            .draw_sprite2d(r2d, 100.0);
    }
    {
        let mut sub = TextStyle::with_style(font, &base);
        sub.set_style(&fancy);
        sub.text("set_style(&fancy)：italic + letter_spacing")
            .offset(Vec2::new(-half_w + 14.0, -half_h + 95.0).round())
            .draw_sprite2d(r2d, 100.0);
    }

    // ── 2. 责任链 A：text()..size()..align()..into_render()..origin()..draw_sprite2d ──
    font.text("A) text().size().into_render().origin()")
        .size(22.0)
        .align(Align::Center)
        .into_render()
        .origin(Vec2::new(0.5, 0.0))
        .offset(Vec2::new(0.0, -half_h + 100.0).round())
        .color(Color::CYAN)
        .draw_sprite2d(r2d, 99.0);

    // ── 3. 责任链 B：try_stack + 渲染级 transform + 横向渐变 ──
    font.text("B) into_render().transform().gradient")
        .size(30.0)
        .align(Align::Center)
        .into_render()
        .transform(Transform2D::default().with_pos(Vec2::new(0.0, 60.0).round()).with_rot(0.05))
        .origin(Vec2::new(0.5, 0.0))
        .draw_2d_gradient(
            r2d, 98.0,
            GradientMode::Line, GradientAxis::Horizontal,
            &[(0.0, Color::RED), (0.5, Color::YELLOW), (1.0, Color::ORANGE)],
        );

    // ── 4. 多行 + 竖向渐变（Frame 模式） ──
    font.text("C) 竖向渐变\nFrame 模式")
        .font_family("站酷快乐体2016修订版")
        .size(26.0)
        .align(Align::Center)
        .into_render()
        .origin(Vec2::new(0.5, 0.5))
        .offset(Vec2::new(0.0, -half_h + 190.0).round())
        .draw_2d_gradient(
            r2d, 97.0,
            GradientMode::Frame, GradientAxis::Vertical,
            &[(0.0, Color::CYAN), (1.0, Color::BLUE)],
        );

    // ── 4. 多行 + 竖向渐变（Line 模式） ──
    font.text("C2) 竖向渐变\nLine 模式")
        .font_family("站酷快乐体2016修订版")
        .size(26.0)
        .align(Align::Center)
        .into_render()
        .origin(Vec2::new(0.5, 0.5))
        .offset(Vec2::new(0.0, -half_h + 280.0).round())
        .draw_2d_gradient(
            r2d, 97.0,
            GradientMode::Line, GradientAxis::Vertical,
            &[(0.0, Color::CYAN), (1.0, Color::ALICEBLUE)],
        );

    // ── 5. draw_with：回调收到逐字形 Transform2D（世界坐标） ──
    font.text("D) draw_with(region, transform)\n回调收到逐字形 Transform2D")
        .size(20.0)
        .align(Align::Center)
        .into_render()
        .origin(Vec2::new(0.5, 0.5))
        .offset(Vec2::new(0.0, 0.0).round())
        .draw_with(|_m, _ln, region, tr| {
            // tr.pos = 字形世界锚点；在字形下方画一条黄色下划线
            if let Some(tex) = TEXTURES.get(region.page_uid) {
                let w = region.wh_px.0 as f32;
                let h = region.wh_px.1 as f32;
                r2d.add_sprite2d(
                    SpriteRect::from_texture_px(Vec2::new(tr.pos.x, tr.pos.y), Vec2::new(w, h),
                vec2_u32tup(region.tl_px), vec2_u32tup(region.wh_px), 1.0 / vec2(tex.width as f32, tex.height as f32)),
                    Color::rgba(1.0, 0.9, 0.2, 0.7),
                    Transform2D::default(),
                    96.0,
                    &tex,
                );
            }
        });

    // ── 6. map：逐字形动画 + glyph_str / glyph_type ──
    font.text("E) map 逐字形 ✨😀🔵❤️💖😍👌🤞👻☠️🤖👾🙉")
        .font_family("站酷快乐体2016修订版")
        .size(30.0)
        .align(Align::Center)
        .into_render()
        .origin(Vec2::new(0.5, 0.0))
        .offset(Vec2::new(0.0, -half_h + 400.0))
        .map(|g: &mut GlyphData| {
            g.top_left.y += (t * 5.0 + g.top_left.x * 0.04).sin() * 8.0;
            if g.glyph_type == GlyphType::Color {
                g.color = [1.0, 1.0, 1.0, 1.0]; // Emoji 保持原色
            } else {
                let i = g.glyph_str().chars().next().unwrap_or(' ') as i32;
                g.color = [0.9, 0.6 + 0.4 * ((i % 3) as f32 / 2.0), 0.9, 1.0];
            }
        })
        .draw_sprite2d(r2d, 95.0);
    // ── 7. precache 预缓存 + into_render_with（用户持缓冲，多标签） ──
    {
        let mut hp_buf = TextBuffer::default();
        let mut mp_buf = TextBuffer::default();
        // precache：字形先入图集（预热缓存），再正式渲染
        let _ = font.text("precache 预热：G). precache + into_render_with").size(16.0).precache();
        font.text(format!("HP {}/{}", 120, 120)).size(24.0)
            .into_render_with(&mut hp_buf)
            .offset(Vec2::new(-half_w + 14.0, -half_h + 490.0))
            .color(Color::GREEN)
            .draw_sprite2d(r2d, 94.0);
        font.text(format!("MP {}/{}", 80, 80)).size(24.0)
            .into_render_with(&mut mp_buf)
            .offset(Vec2::new(-half_w + 14.0, -half_h + 520.0))
            .color(Color::CYAN)
            .draw_sprite2d(r2d, 94.0);
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    rjw_main::run_app(ChainDemo::new())
}
