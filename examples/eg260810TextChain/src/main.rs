//! eg260810TextChain —— `rjw_text` **唯一文本链** API 演示（契约 §8.4）。
//!
//! 展示：
//! - **绑定的运行时路径**：`f.text(|t: &mut TextCtx| ..)` —— `t.label(..)` 已绑定世界层
//!   `Render2D`，终点 `draw(layer)` **只收 1 个参数**；
//! - **独立路径**：`Text::label(..)`（不绑定）→ `Label::draw_to(&mut r2d, layer)`（**2 参**）
//!   与 `Label::draw_with(|g: &Glyph| ..)`（逐字形回调 + 自绘）；
//! - `TextStyle`：**唯一样式类型**（owned / `Clone` / 可存字段）；全局默认经
//!   `TextCtx::style_mut()`，单个标签经 `Label::style(..)`；克隆继承 `base.clone().size(..)`；
//! - 定位语言：`at`（左上角）/ `center`（中心）/ `anchor`（归一化锚点）/ `offset`（像素微调）；
//! - 渐变六种组合：`Gradient::{glyph,line,frame}_{h,v}`；
//! - `map` 逐字形动画 + `Glyph::{top_left, glyph_str, glyph_type, color_mut, translate}`；
//! - 剔除默认开启 + `.no_cull()` / `clip`（局部）/ `clip_world`（世界）/ `cache(CachePolicy)`；
//! - **UI 稳定集成面**：`Text::buffer` / `Text::label_from` / `Text::lines` /
//!   `Text::measure_buffer`。
//!
//! 驱动层：`App::{config, init, update}` + `ctx.frame()` 守卫 + `f.submit(..)`。

use rjw_krusie::gpu::TEXTURES;
use rjw_krusie::prelude::*;
use rjw_krusie::text::{
    Glyph, GlyphType, LineSpace, TextBuffer, TextStyle, cosmic_text,
};

#[derive(Default)]
struct ChainDemo {
    /// 应用自持 `Text`（独立路径演示用；与运行时 `Ctx` 内的文本子系统相互独立）。
    font: Option<Text>,
    cam: Camera2D,
    t: f32,
    /// 上次观测到的字形图集页数（变化时打印，用于验证“页未满却开新页”修复）
    last_page_count: usize,
}

fn vec2_u32tup(t: (u32, u32)) -> glam::Vec2 {
    vec2(t.0 as f32, t.1 as f32)
}

impl App for ChainDemo {
    fn config(&self) -> AppConfig {
        AppConfig::new("eg260810TextChain - rjw_text 唯一文本链").size(1280.0, 720.0)
    }

    fn init(&mut self, gfx: &Gfx) {
        eprintln!("MARK: init");
        self.font = Some(gfx.text());
        // 相机默认即可（`Camera2D::default()`：zoom 1、世界原点居中）。
        self.cam.set_zoom(Vec2::ONE);
    }

    fn update(&mut self, ctx: &mut Ctx) {
        // ── 逻辑半程：无帧也执行 ──────────────────────────────
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }
        self.t += ctx.dt();
        let t = self.t;

        // ── 渲染半程：守卫在应用里 ────────────────────────────
        let Some(mut f) = ctx.frame() else { return };
        self.cam.set_region(f.region());
        let half_w = f.region().size().x * 0.5;
        let half_h = f.region().size().y * 0.5;

        // ① 绑定路径：`TextCtx::label(..)` 已绑定世界层 Render2D ⇒ `draw(layer)` 1 参。
        // 顺带读取字形图集页数（`TextCtx::glyph_cache()`，低层诊断）。
        let mut pages_now = 0usize;
        f.text(|tc| {
            draw_chain_demos(tc, t, half_w, half_h);
            pages_now = tc.glyph_cache().page_count();
        });

        // ② 独立路径 + UI 集成面：应用自持 `Text` + 世界层 `Render2D`（`f.draw()`）。
        if let Some(font) = self.font.as_mut() {
            let r2d = f.draw();
            draw_independent_demos(font, r2d, t, half_w, half_h);
        }

        // 字形图集页数变化时打印（修复前该示例会无谓地开新页）。
        if pages_now != self.last_page_count {
            eprintln!("glyph atlas pages: {pages_now} (changed)");
            self.last_page_count = pages_now;
        }

        f.submit(&mut self.cam, Clear::color(Color::rgb(0.09, 0.11, 0.16)));
    }
}

// ─── ① 绑定路径：唯一链 `label(..) -> Label -> draw(layer)` ─────────

fn draw_chain_demos(t: &mut TextCtx<'_>, time: f32, half_w: f32, half_h: f32) {
    // ── 1. 全局默认样式：`TextCtx::style_mut()`（`Label` 从中继承） ──
    {
        let base = t
            .style()
            .clone()
            .font_family("SimHei")
            .size(16.0)
            .line_space(LineSpace::Multiple(1.4))
            .align(Align::Left)
            .color(Color::WHITE);
        *t.style_mut() = base;
        t.label("eg260810TextChain — rjw_text 唯一文本链")
            .at(vec2(-half_w + 14.0, -half_h + 14.0))
            .draw(100.0);
        t.label("全局默认样式：TextCtx::style_mut() → Label 继承")
            .at(vec2(-half_w + 14.0, -half_h + 40.0))
            .color(Color::YELLOW)
            .draw(100.0);
    }

    // ── 1b. TextStyle：owned 可存字段 + 克隆继承 + `Label::style(..)` 套用 ──
    let base = TextStyle::new()
        .font_family("SimHei")
        .size(16.0)
        .weight(cosmic_text::Weight::BOLD)
        .line_space(LineSpace::Multiple(1.4))
        .align(Align::Left)
        .color(Color::WHITE);
    let warn = base.clone().size(20.0).color(Color::RED); // 克隆继承：只改差异
    let fancy = TextStyle::new()
        .font_family("SimHei")
        .size(18.0)
        .italic(true)
        .letter_spacing(2.0)
        .color(Color::ORANGE);

    t.label("TextStyle → Label::style(warn)：weight+color 继承")
        .style(warn)
        .at(vec2(-half_w + 14.0, -half_h + 70.0).round())
        .draw(100.0);
    t.label("style(fancy)：italic + letter_spacing")
        .style(fancy)
        .at(vec2(-half_w + 14.0, -half_h + 95.0).round())
        .draw(100.0);

    // ── 2. 定位语言：at / center / anchor / offset ──
    t.label("A) anchor(0.5,0) + at()：内容顶边中点落在 at")
        .size(18.0)
        .align(Align::Center)
        .anchor(vec2(0.5, 0.0))
        .at(vec2(0.0, -half_h + 125.0).round())
        .color(Color::CYAN)
        .draw(99.0);
    t.label("A2) center()")
        .size(16.0)
        .center(vec2(half_w - 130.0, -half_h + 125.0))
        .color(Color::rgba(0.6, 1.0, 0.6, 1.0))
        .draw(99.0);
    t.label("A3) at() + offset() 像素微调")
        .size(14.0)
        .at(vec2(-half_w + 14.0, -half_h + 152.0))
        .offset(vec2(24.0, 0.0))
        .color(Color::GRAY)
        .draw(99.0);

    // ── 3. 渐变：Glyph / Line × 横向 ──
    let stops_h = [(0.0, Color::RED), (0.5, Color::YELLOW), (1.0, Color::ORANGE)];
    t.label("B) Gradient::line_h")
        .size(22.0)
        .anchor(vec2(0.5, 0.0))
        .at(vec2(0.0, -half_h + 180.0).round())
        .gradient(Gradient::line_h(&stops_h))
        .draw(98.0);
    t.label("B2) Gradient::glyph_h")
        .size(22.0)
        .anchor(vec2(0.5, 0.0))
        .at(vec2(0.0, -half_h + 212.0).round())
        .gradient(Gradient::glyph_h(&stops_h))
        .draw(98.0);

    // ── 4. 多行 + 竖向渐变：Frame / Line 模式 ──
    let stops_v = [(0.0, Color::CYAN), (1.0, Color::BLUE)];
    t.label("C) 竖向渐变\nFrame 模式")
        .font_family("站酷快乐体2016修订版")
        .size(26.0)
        .align(Align::Center)
        .center(vec2(-half_w * 0.5, -half_h + 300.0))
        .gradient(Gradient::frame_v(&stops_v))
        .draw(97.0);
    t.label("C2) 竖向渐变\nLine 模式")
        .font_family("站酷快乐体2016修订版")
        .size(26.0)
        .align(Align::Center)
        .center(vec2(-half_w * 0.5, -half_h + 390.0))
        .gradient(Gradient::line_v(&[(0.0, Color::CYAN), (1.0, Color::ALICEBLUE)]))
        .draw(97.0);

    // ── 5. map：逐字形动画 + glyph_str / glyph_type / color_mut / translate ──
    t.label("E) map 逐字形 ✨😀🔵❤️💖😍👌🤞👻☠️🤖👾🙉")
        .font_family("站酷快乐体2016修订版")
        .size(30.0)
        .align(Align::Center)
        .anchor(vec2(0.5, 0.0))
        .at(vec2(0.0, -half_h + 470.0))
        .map(|g: &mut Glyph| {
            let dy = (time * 5.0 + g.top_left().x * 0.04).sin() * 8.0;
            g.translate(vec2(0.0, dy));
            if g.glyph_type() == GlyphType::Color {
                *g.color_mut() = [1.0, 1.0, 1.0, 1.0]; // Emoji 保持原色
            } else {
                let i = g.glyph_str().chars().next().unwrap_or(' ') as i32;
                *g.color_mut() = [0.9, 0.6 + 0.4 * ((i % 3) as f32 / 2.0), 0.9, 1.0];
            }
        })
        .draw(95.0);

    // ── 6. clip（默认剔除） + no_cull 对照 + clip_world + CachePolicy ──
    {
        let mut log = String::new();
        for i in 0..12 {
            log.push_str(&format!("日志行 {i}: clip 收集期剔除演示\n"));
        }
        // clip 为文本局部坐标（相对字形 top_left）。剔除**默认开启**（旧 `.cull(true)`）：
        // 只收集前 ~3 行可见字形（60px / 行高）；`CachePolicy::Always` 强制进 LRU。
        t.label(log.as_str())
            .size(14.0)
            .align(Align::Left)
            .clip(Rect::new(0.0, 0.0, 300.0, 60.0))
            .cache(CachePolicy::Always)
            .at(vec2(-half_w + 14.0, -half_h + 540.0).round())
            .color(Color::GREEN)
            .draw(93.0);
        // 对照 1：`.no_cull()` 关闭剔除 → 12 行全部收集绘制（`CachePolicy::Never` 不入 LRU）。
        t.label(log.as_str())
            .size(14.0)
            .align(Align::Left)
            .clip(Rect::new(0.0, 0.0, 300.0, 60.0))
            .no_cull()
            .cache(CachePolicy::Never)
            .at(vec2(-half_w + 330.0, -half_h + 540.0).round())
            .color(Color::GRAY)
            .draw(93.0);
        // 对照 2：`clip_world`（世界坐标裁剪；整块 + 逐字形保守剔除）。
        t.label(log.as_str())
            .size(14.0)
            .align(Align::Left)
            .clip_world(Rect::new(-half_w + 640.0, -half_h + 540.0, 300.0, 60.0))
            .at(vec2(-half_w + 660.0, -half_h + 540.0).round())
            .color(Color::rgba(0.7, 0.7, 1.0, 1.0))
            .draw(93.0);
    }
}

// ─── ② 独立路径：`draw_to`（2 参）/ `draw_with` + UI 稳定集成面 ─────

fn draw_independent_demos(
    font: &mut Text,
    r2d: &mut Render2D,
    _time: f32,
    half_w: f32,
    half_h: f32,
) {
    // ── 7. draw_with：回调收到逐字形 `Glyph`（世界变换），可自绘叠加 ──
    font.label("D) draw_with(|g: &Glyph|) + 自绘黄色高亮（字形下移 1px 重绘）")
        .font_family("SimHei")
        .size(18.0)
        .align(Align::Center)
        .center(vec2(half_w * 0.5, -half_h + 125.0))
        .draw_with(|g: &Glyph| {
            let region = g.region();
            let Some(tex) = TEXTURES.get(region.page_uid) else { return };
            // `g.transform()` = 世界变换（含字形位置）：ZERO 即字形左上角世界坐标。
            let pos = g.transform().transform_point(Vec2::ZERO) + vec2(0.0, 1.0);
            let w = region.wh_px.0 as f32;
            let h = region.wh_px.1 as f32;
            r2d.sprite(
                SpriteRect::with_uv_tex(
                    pos,
                    vec2(w, h),
                    vec2_u32tup(region.tl_px),
                    vec2_u32tup(region.wh_px),
                    &tex,
                ),
                &tex,
            )
            .tint(Color::rgba(1.0, 0.9, 0.2, 0.35))
            .layer(96.0);
        });

    // ── 8. 独立路径终点：`draw_to(r2d, layer)`（2 参） ──
    font.label("F) Text::label(..).draw_to(r2d, layer)  —— 独立路径 2 参")
        .font_family("SimHei")
        .size(18.0)
        .align(Align::Center)
        .anchor(vec2(0.5, 0.0))
        .at(vec2(0.0, half_h - 60.0))
        .color(Color::rgba(0.7, 1.0, 0.7, 1.0))
        .draw_to(r2d, 94.0);

    // ── 9. UI 稳定集成面：buffer / lines / measure_buffer / label_from ──
    {
        let style = TextStyle::new().font_family("SimHei").size(16.0).line_height(16.0);
        // 排版缓冲（`wrap <= 0` = 不换行；`CachePolicy::User` = UI 自持缓冲，不进 LRU）。
        let buf = font.buffer("Text::buffer(style, wrap=0, User) → label_from(&buf)", &style, 0.0, CachePolicy::User);
        let lines = Text::lines(&buf);
        let sz = Text::measure_buffer(&buf);
        // 已排版缓冲直接进入唯一链（不重新整形）。
        font.label_from(&buf)
            .at(vec2(-half_w + 14.0, half_h - 60.0))
            .color(Color::ALICEBLUE)
            .draw_to(r2d, 92.0);
        font.label(format!("lines() = {}, measure_buffer() = {:.0}×{:.0}", lines.len(), sz.x, sz.y))
            .font_family("SimHei")
            .size(14.0)
            .at(vec2(-half_w + 14.0, half_h - 40.0))
            .color(Color::GRAY)
            .draw_to(r2d, 92.0);
    }

    // ── 10. `into_buffer`（用户持缓冲复用）：收集到自有 `TextBuffer` ──
    {
        let mut scratch = TextBuffer::default();
        let hp = format!("HP {}/{}", 120, 120);
        let buf = font.label(hp.as_str()).size(24.0).into_buffer(&mut scratch);
        // `scratch`（复用缓冲，跨帧 clear+填充）里是本帧收集到的字形/行。
        let (ng, nl) = (scratch.glyphs.len(), scratch.lines.len());
        font.label_from(&buf)
            .at(vec2(-half_w + 14.0, half_h - 20.0))
            .color(Color::GREEN)
            .draw_to(r2d, 94.0);
        font.label(format!("into_buffer: glyphs={ng} lines={nl}"))
            .font_family("SimHei")
            .size(12.0)
            .at(vec2(-half_w + 200.0, half_h - 20.0))
            .color(Color::GRAY)
            .draw_to(r2d, 92.0);
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    run(ChainDemo::default())
}
