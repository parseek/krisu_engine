# rjw_text

中文：
`rjw_text` 是基于 `cosmic-text` 排版 + `swash` 字形光栅化 + `DynamicAtlas` 字形缓存的文本渲染 crate。

English：
`rjw_text` renders text using `cosmic-text` layout, `swash` glyph rasterization and a `DynamicAtlas` glyph cache.

---

## 功能特性 / Features

中文：
- `Text`：持有 `FontSystem` / `ScaleContext` / 字形缓存图集（key = `cosmic_text::CacheKey`）。
- `draw_label` / `draw_label_ex`：一行文本直接渲染到 `Render2D`，支持 family / align / origin。
- `draw_text`：遍历已排版 `Buffer` 的字形，回调 `(region, world_pos, world_size)` 自定义绘制。
- `create_buffer`：cosmic-text 排版（Metrics / Attrs / Shaping / Align）。
- `load_font_data`：加载自定义 ttf / otf 字体。
- `DEFAULT_GLYPH_ATLAS_SIZE`：字形图集默认尺寸（1024）。

English：
- `Text`: owns `FontSystem` / `ScaleContext` / glyph cache atlas (key = `cosmic_text::CacheKey`).
- `draw_label` / `draw_label_ex`: render a single line directly to `Render2D` with family / align / origin.
- `draw_text`: iterate layout glyphs of a `Buffer`, call back `(region, world_pos, world_size)` for custom drawing.
- `create_buffer`: cosmic-text layout (Metrics / Attrs / Shaping / Align).
- `load_font_data`: load custom ttf / otf fonts.
- `DEFAULT_GLYPH_ATLAS_SIZE`: default glyph atlas size (1024).

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

## 许可 / License

MIT © 2026 KrisuRJW
