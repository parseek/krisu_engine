//! 文本渲染：基于 `cosmic-text` 排版 + `swash` 字形光栅化 + `DynamicAtlas` 字形缓存。
//!
//! - `Text`：持有字体系统（cosmic-text `FontSystem`）、字形缓存图集（key=`cosmic_text::CacheKey`）。
//! - `draw_label(r2d, text, ...)`：最简一行文本渲染，无需手动排版。
//! - `draw_text(buffer, callback)`：遍历已排版 Buffer 的每个字形，查询/渲染缓存，调用闭包。

use std::collections::HashMap;

pub use cosmic_text::{Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glam::Vec2;
use rjw_2d_render::{Layer, Render2D, SpriteRect};
use rjw_atlas::{AtlasConfig, AtlasRegion, DynamicAtlas};
use rjw_color::Color;
use rjw_render::TEXTURES;
use rjw_transform::Transform2D;
pub use cosmic_text;
use swash::scale::{
    Render, ScaleContext, Source,
    image::Content as SwashContent,
};
use swash::zeno::{Format, Vector};

pub const DEFAULT_GLYPH_ATLAS_SIZE: u32 = 1024;

// ─── GlyphLocation ─────────────────────────────────────────────

struct GlyphLocation {
    region: AtlasRegion,
    /// bearing-x for sprite positioning
    left: i32,
    /// bearing-y (positive = up from baseline)
    top: i32,
    /// image content type (for compact/repack)
    #[allow(dead_code)]
    content: SwashContent,
}

// ─── Text ──────────────────────────────────────────────────────

pub struct Text {
    font_system: FontSystem,
    scale_context: ScaleContext,
    glyph_cache: DynamicAtlas<cosmic_text::CacheKey>,
    locations: HashMap<cosmic_text::CacheKey, GlyphLocation>,
}

impl Text {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout) -> Self {
        let glyph_cache = DynamicAtlas::new(
            device, queue, layout,
            AtlasConfig { max_pages: 4, padding: 1, ..Default::default() },
            DEFAULT_GLYPH_ATLAS_SIZE,
        );
        let mut font_system = FontSystem::new();
        font_system.db_mut().load_system_fonts();
        Self {
            font_system,
            scale_context: ScaleContext::new(),
            glyph_cache,
            locations: HashMap::new(),
        }
    }

    /// 加载额外的字体数据（如自定义 ttf/otf 文件）；已加载的系统字体不受影响。
    pub fn load_font_data(&mut self, data: Vec<u8>) {
        self.font_system.db_mut().load_font_data(data);
    }

    pub fn create_buffer(
        &mut self, text: &str, attrs: Attrs<'_>, size: f32, line_height: f32, align: Align,
    ) -> Buffer {
        let metrics = Metrics::new(size, line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let wrap_width = 1024.0f32.max(size * text.len() as f32 * 0.7);
        buffer.set_size(Some(wrap_width), Some(line_height));
        buffer.set_text(text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer
    }

    /// 遍历已排版 `Buffer` 中的每个字形，`callback(region, world_pos, world_size)`。
    /// `world_pos` 是字形精灵的**左上角**世界坐标（已含 bearing）。
    pub fn draw_text<F>(&mut self, buffer: &Buffer, mut callback: F)
    where F: FnMut(&AtlasRegion, Vec2, Vec2)
    {
        for run in buffer.layout_runs() {
            let line_y = run.line_y;
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let cache_key = physical.cache_key;

                // 渲染/打包（如未缓存）
                if !self.locations.contains_key(&cache_key) {
                    self.rasterize_and_pack(cache_key);
                }

                if let Some(loc) = self.locations.get(&cache_key) {
                    // 字形精灵左上角 = 排版位置 + bearing 偏移
                    let pos = Vec2::new(
                        physical.x as f32 + loc.left as f32,
                        line_y - loc.top as f32,
                    );
                    let sz = Vec2::new(loc.region.wh_px.0 as f32, loc.region.wh_px.1 as f32);
                    callback(&loc.region, pos, sz);
                }
            }
        }
    }

    /// 内部：渲染+打包一个 swash 字形，写入 DynamicAtlas。
    fn rasterize_and_pack(&mut self, cache_key: cosmic_text::CacheKey) {
        let Some(font) = self.font_system.get_font(cache_key.font_id, cache_key.font_weight) else { return; };
        let mut scaler = self.scale_context
            .builder(font.as_swash())
            .size(f32::from_bits(cache_key.font_size_bits))
            .hint(!cache_key.flags.contains(cosmic_text::CacheKeyFlags::DISABLE_HINTING))
            .build();
        let offset = Vector::new(cache_key.x_bin.as_float(), cache_key.y_bin.as_float());
        let image = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
            Source::Outline,
        ])
        .format(Format::Alpha)
        .offset(offset)
        .render(&mut scaler, cache_key.glyph_id);
        let Some(image) = image else { return; };

        let w = image.placement.width as u32;
        let h = image.placement.height as u32;
        if w == 0 || h == 0 { return; }

        let mut rgba = vec![0u8; (w * h * 4) as usize];
        match image.content {
            SwashContent::Mask => {
                for row in 0..h as usize {
                    for col in 0..w as usize {
                        let src = row * w as usize + col;
                        let dst = src * 4;
                        if src < image.data.len() {
                            rgba[dst] = 0xFF;
                            rgba[dst + 1] = 0xFF;
                            rgba[dst + 2] = 0xFF;
                            rgba[dst + 3] = image.data[src];
                        }
                    }
                }
            }
            _ => {
                // Color / SubpixelMask → 原始 RGBA 数据
                for row in 0..h as usize {
                    for col in 0..w as usize {
                        let src = (row * w as usize + col) * 4;
                        let dst = src;
                        if src + 3 < image.data.len() {
                            rgba[dst..dst + 4].copy_from_slice(&image.data[src..src + 4]);
                        }
                    }
                }
            }
        }

        // 写入 DynamicAtlas（使用 insert_no_clamp 避免边界 padding 二次挤压）
        if let Some(region) = self.glyph_cache.insert(
            cache_key, &rgba, w, h, (0, 0), false
        ) {
            self.locations.insert(cache_key, GlyphLocation {
                region,
                left: image.placement.left,
                top: image.placement.top,
                content: image.content,
            });
        }
    }

    /// 简化便捷方法：一键渲染文本精灵。
    ///
    /// `pos` — 文本左上角世界坐标。
    /// `family` 支持自定义 family（如 `"SimHei"`）；传空字符串或无效名时自动回退到系统字体。
    pub fn draw_label(
        &mut self, r2d: &mut Render2D, text: &str, color: Color,
        size: f32, line_height: f32, pos: Vec2, family: &str, align: Align, layer: impl Into<Layer> + Clone,
    ) -> Vec2 {
        self.draw_label_ex(r2d, text, color, size, line_height, pos, family, align, layer, Vec2::ZERO)
    }

    /// 扩展版：`origin` 以内容宽高为单位，归一化到 [0,1]。
    /// `origin = (0,0)` 为左上角，`origin = (0.5,0.5)` 为中心点。
    pub fn draw_label_ex(
        &mut self, r2d: &mut Render2D, text: &str, color: Color,
        size: f32, line_height: f32, pos: Vec2, family: &str, align: Align, layer: impl Into<Layer> + Clone,
        origin: Vec2,
    ) -> Vec2 {
        let attrs = if family.is_empty() {
            Attrs::new()
        } else {
            Attrs::new().family(Family::Name(family))
        };
        let buf = self.create_buffer(text, attrs, size, line_height, align);
        // 先渲染所有字形，填充 self.locations
        for run in buf.layout_runs() {
            for glyph in run.glyphs.iter() {
                let cache_key = glyph.physical((0.0, 0.0), 1.0).cache_key;
                if !self.locations.contains_key(&cache_key) {
                    self.rasterize_and_pack(cache_key);
                }
            }
        }
        let visual_origin = self.buffer_origin(&buf);
        let content_size = self.buffer_content_size(&buf);
        let offset = Vec2::new(content_size.x * origin.x, content_size.y * origin.y);
        let ps = self.glyph_cache.page_size() as f32;
        let inv = Vec2::new(1.0 / ps, 1.0 / ps);
        for run in buf.layout_runs() {
            let line_y = run.line_y;
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let cache_key = physical.cache_key;
                if let Some(loc) = self.locations.get(&cache_key) {
                    let glyph_pos = Vec2::new(
                        physical.x as f32 + loc.left as f32,
                        line_y - loc.top as f32,
                    );
                    let world_tl = pos + glyph_pos - visual_origin - offset;
                    let glyph_size = Vec2::new(loc.region.wh_px.0 as f32, loc.region.wh_px.1 as f32);
                    let rect = SpriteRect::from_texture_px(
                        world_tl, glyph_size,
                        Vec2::new(loc.region.tl_px.0 as f32, loc.region.tl_px.1 as f32),
                        Vec2::new(loc.region.wh_px.0 as f32, loc.region.wh_px.1 as f32),
                        inv,
                    );
                    if let Some(tex) = TEXTURES.get(loc.region.page_uid) {
                        r2d.add_sprite2d(rect, color, Transform2D::default(), layer.clone().into(), &tex);
                    }
                }
            }
        }
        content_size
    }

    fn buffer_content_size(&self, buffer: &Buffer) -> Vec2 {
        let mut w: f32 = 0.0;
        let mut bottom: f32 = 0.0;
        for run in buffer.layout_runs() {
            w = w.max(run.line_w);
            bottom = bottom.max(run.line_y + run.line_height);
        }
        Vec2::new(w.ceil(), bottom.ceil())
    }

    /// 计算文本的第一个视觉字形的左上角（bearing 恢复后），用于对齐。
    fn buffer_origin(&self, buffer: &Buffer) -> Vec2 {
        for run in buffer.layout_runs() {
            if let Some(g) = run.glyphs.first() {
                let physical = g.physical((0.0, 0.0), 1.0);
                if let Some(loc) = self.locations.get(&physical.cache_key) {
                    return Vec2::new(
                        physical.x as f32 + loc.left as f32,
                        run.line_y - loc.top as f32,
                    );
                }
            }
        }
        Vec2::ZERO
    }

    pub fn glyph_cache(&self) -> &DynamicAtlas<cosmic_text::CacheKey> { &self.glyph_cache }
    pub fn page_size(&self) -> u32 { self.glyph_cache.page_size() }
}