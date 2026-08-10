# rjw_text

中文：
`rjw_text` 是基于 `cosmic-text` 排版 + `swash` 字形光栅化 + `DynamicAtlas` 字形缓存的文本渲染 crate。

English：
`rjw_text` renders text using `cosmic-text` layout, `swash` glyph rasterization and a `DynamicAtlas` glyph cache.

---

## 功能特性 / Features

中文：
- `Text`：持有 `FontSystem` / `ScaleContext` / 字形缓存图集（key = `cosmic_text::CacheKey`）。
- `measure` / `measure_buffer`：排版内容宽高（GUI 布局用）。
- `draw_text`：遍历已排版 `Buffer` 的字形，回调 `(region, world_pos, world_size)` 自定义绘制。
- `draw_label_with`：回调版标签渲染，不绑定渲染器（GUI 自定义绘制用）。
- `draw_label` / `draw_label_ex`：一行文本直接渲染到 `Render2D`，支持 family / align / origin（feature = `rjw_2d_render`，默认开启）。
- 责任链：`text(..)..into_render()/into_render_with(&mut TextBuffer)..origin(..)..draw_with(..)`（`TextLayout` / `TextRender`；`draw_sprite2d` / `draw_2d_gradient` 需默认 feature，渐变支持横向/竖向）。
- `create_buffer`：cosmic-text 排版（Metrics / Attrs / Shaping / Align），内部按输入做 **LRU 排版缓存**（上限 [`MAX_LAYOUT_CACHE`]=128），相同输入经 O(1) 签名命中后返回共享 `Arc<Buffer>`（不深拷贝），跳过重复整形。
- 空格 / 零尺寸 / 渲染失败字形记入 `no_image` 只判定一次，避免每帧重复光栅化。
- `load_font_data`：加载自定义 ttf / otf 字体。
- `DEFAULT_GLYPH_ATLAS_SIZE`：字形图集默认尺寸（1024）。

English：
- `Text`: owns `FontSystem` / `ScaleContext` / glyph cache atlas (key = `cosmic_text::CacheKey`).
- `measure` / `measure_buffer`: layout content width/height (for GUI layout).
- `draw_text`: iterate layout glyphs of a `Buffer`, call back `(region, world_pos, world_size)` for custom drawing.
- `draw_label_with`: callback-based label rendering, not bound to any renderer (for GUI custom drawing).
- `draw_label` / `draw_label_ex`: render a single line directly to `Render2D` with family / align / origin (feature = `rjw_2d_render`, enabled by default).
- Chain API: `text(..)..into_render()/into_render_with(&mut TextBuffer)..origin(..)..draw_with(..)` (`TextLayout` / `TextRender`; `draw_sprite2d` / `draw_2d_gradient` need the default feature).
- `create_buffer`: cosmic-text layout (Metrics / Attrs / Shaping / Align), with an internal **LRU layout cache** (cap [`MAX_LAYOUT_CACHE`]=128); repeated inputs hit via an O(1) signature and return a shared `Arc<Buffer>` (no deep copy).
- Zero-size / missing-font / failed glyphs are recorded in `no_image` and judged once, avoiding per-frame re-rasterization.
- `load_font_data`: load custom ttf / otf fonts.
- `DEFAULT_GLYPH_ATLAS_SIZE`: default glyph atlas size (1024).

## 特性开关 / Features

- `rjw_2d_render`（默认开启 / enabled by default）：提供 `draw_label` / `draw_label_ex` 便捷方法（直接渲染到 `Render2D`）。
  关闭后仅保留核心排版/测量/回调 API：`create_buffer` / `measure` / `measure_buffer` / `draw_text` / `draw_label_with`，
  `rjw_text` 不再依赖 `rjw_2d_render` / `rjw_render` / `rjw_color`（`rjw_transform` 为核心常驻数学依赖，提供 `Transform2D`）。
- `rjw_2d_render` (default): provides the `draw_label` / `draw_label_ex` convenience methods (render directly to `Render2D`).
  Without it, only the core layout/measure/callback API remains (`create_buffer` / `measure` / `measure_buffer` / `draw_text` / `draw_label_with`),
  and `rjw_text` no longer depends on `rjw_2d_render` / `rjw_render` / `rjw_color` (`rjw_transform` stays as a core math dependency).

```toml
rjw_text = { path = "../crates/rjw_text" }                    # 默认（含 rjw_2d_render）
rjw_text = { path = "../crates/rjw_text", default-features = false }
```

---

## 示例代码 / Example

```rust
use rjw_color::Color;
use rjw_text::{Align, Text};
use rjw_transform::Vec2;

let mut font = Text::new(render2d.device(), render2d.queue(), render2d.tex_bind_group_layout());
font.draw_label(
    render2d, "Hello 世界", Color::WHITE,
    24.0, 32.0, Vec2::ZERO, "", Align::Left, 0.0,
);
```

---
## 责任链 / Chain API

两阶段责任链：`TextLayout`（排版配置）→ `TextRender`（渲染配置），转换方向单向；
`Style` / `TextStyle` 把重复的字体 / 字号 / 行距提取为可复用样式。

```rust
use rjw_text::{Align, GradientAxis, GradientMode, LineSpace, Style, Text, TextStyle, Transform2D};

let mut font = Text::new(device, queue, layout);

// ── 1. TextStyle / Style：可复用样式，只写差异 ──
let mut ui = font.build_style()
    .font_family("SimHei")
    .size(14.0)
    .line_space(LineSpace::Multiple(1.5))
    .align(Align::Left)
    .color(Color::WHITE);                 // 样式默认；克隆继承 base.clone().size(..)

ui.text(format!("❤HP: {} / {}", hp, max_hp))
    .offset(bar_pos + vec2(0.0, -18.0))
    .draw_sprite2d(r2d, LAYER_UI + 0.5);

// ── 1b. Style 责任链（与 Text 解耦）+ with_style / set_style ──
let base = Style::default()
    .font_family("SimHei")
    .size(16.0)
    .weight(cosmic_text::Weight::BOLD)   // OwnedAttrs 设置：字重/斜体/字距…
    .color(Color::WHITE);
let warn = base.clone().size(20.0).color(Color::RED); // 克隆继承：只改差异
let mut ui2 = TextStyle::with_style(&mut font, &warn);
ui2.text("warn").draw_sprite2d(r2d, LAYER_UI + 0.2);
ui2.set_style(&base);                     // 切换样式
ui2.text("base").draw_sprite2d(r2d, LAYER_UI + 0.2);

// ── 2. TextLayout → TextRender（直接链） ──
font.text("Hello 世界")
    .size(24.0).align(Align::Center)
    .into_render()                        // 多标签并存用 into_render_with(&mut TextBuffer)
    .origin(Vec2::new(0.5, 0.5))
    .color(Color::WHITE)
    .draw_sprite2d(r2d, LAYER_UI);

// ── 3. 渲染级 transform + 渐变 ──
font.text("GAME OVER")
    .size(30.0).align(Align::Center)
    .into_render()
    .transform(Transform2D::default().with_pos(cam.position).with_rot(0.35))
    .draw_2d_gradient(r2d, LAYER_UI, GradientMode::Line, GradientAxis::Horizontal,
                      &[(0.0, Color::RED), (1.0, Color::YELLOW)]);

// ── 4. draw_with：回调收到逐字形 Transform2D（世界坐标） ──
font.text("draw_with")
    .into_render()
    .draw_with(|m: &rjw_text::MeasureInfo,
               ln: &rjw_text::LineMeasureInfo,
               r: &rjw_atlas::AtlasRegion,
               tr: rjw_text::Transform2D| {   // tr.pos = 字形世界锚点
        if let Some(tex) = rjw_render::TEXTURES.get(r.page_uid) {
            // 自定义绘制
        }
    });

// ── 5. map：逐字形修改 ──
font.text("MAP 动画 ✨")
    .size(28.0).align(Align::Center)
    .into_render()
    .map(|g: &mut rjw_text::GlyphData| {
        if g.glyph_type == rjw_text::GlyphType::Color {
            g.color = [1.0, 1.0, 1.0, 1.0];   // Emoji 保持原色
        }
        let _ = g.glyph_str();                // 对应字符
    })
    .draw_sprite2d(r2d, LAYER_UI);
```

> 完整可运行示例：`examples/eg260810TextChain`。

中文说明：
- `TextLayout`：`text` / `size` / `line_height` / `line_space` / `align` / `attrs` / `font_family` + 渲染默认 `color` / `origin` / `offset` / `transform`；及 `measure` / `into_buffer` / `precache` / `into_render` / `into_render_with` 与便捷绘制。
- `TextRender`：`origin` / `origin_px` / `offset` / `color` / `transform` / `map` / `draw_with`；默认 feature 下还有 `draw_sprite2d` / `draw_2d_gradient`（`GradientMode::{Glyph,Line,Frame}` × `GradientAxis::{Horizontal,Vertical}`）。
- `Style` 设置：`font_family` / `attrs` / `weight` / `italic` / `stretch` / `letter_spacing`（`OwnedAttrs` = cosmic-text `AttrsOwned`，无借用）+ `size` / `line_height` / `line_space` / `align` + 渲染默认 `color` / `origin` / `offset` / `transform`；克隆继承 `base.clone().size(..)`。
- 常量字符串经 `TextStorage` 内联（`ArrayVec`）存储，零堆分配；`TextRender` 借用缓冲（`Text` 内部默认或用户 `TextBuffer`），跨帧 clear+填充复用容量，无栈内大数组。
- `GlyphData`：`glyph_type`（`Normal` 可染色 / `Color` 如 Emoji）、`glyph_str()`（对应字符）、`layer`（层级偏移）、`transform`（逐字形变换）。

English:
- `TextLayout`: layout chain (`text` / `size` / `line_height` / `line_space` / `align` / `attrs` / `font_family`) + render defaults (`color` / `origin` / `offset` / `transform`); plus `measure` / `into_buffer` / `precache` / `into_render` / `into_render_with` and convenience draws.
- `TextRender`: `origin` / `origin_px` / `offset` / `color` / `transform` / `map` / `draw_with`; with default feature also `draw_sprite2d` / `draw_2d_gradient` (`GradientMode::{Glyph,Line,Frame}` × `GradientAxis::{Horizontal,Vertical}`).
- `Style` settings: `font_family` / `attrs` / `weight` / `italic` / `stretch` / `letter_spacing` (`OwnedAttrs` = cosmic-text `AttrsOwned`) + `size` / `line_height` / `line_space` / `align` + render defaults `color` / `origin` / `offset` / `transform`; clone-inheritance `base.clone().size(..)`.
- Constant strings are stored inline via `TextStorage` (`ArrayVec`); `TextRender` borrows buffers (`Text` internal default or user `TextBuffer`), cleared and refilled each frame (capacity reused), no inline large arrays.
- `GlyphData`: `glyph_type` (`Normal` tintable / `Color` like Emoji), `glyph_str()` (character), `layer` (z-offset), `transform` (per-glyph transform).

---

## 许可 / License

MIT © 2026 KrisuRJW
