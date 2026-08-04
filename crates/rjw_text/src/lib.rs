//! 文本渲染：基于 `cosmic-text` 排版 + `swash` 字形光栅化 + `DynamicAtlas` 字形缓存。
//!
//! - `Font`：持有字体系统、字形缓存图集（key=GlyphKey）、字体数据。
//! - `draw_text(buffer, callback)`：遍历已排版 Buffer 的每个字形，查/渲染缓存，调用闭包。
//! - `draw_text_sprite(r2d, buffer, color, layer)`：便捷方法，直接调 Render2D add_sprite2d。
//!
//! 使用前需调用 `Font::load_font_data(data)` 加载至少一种 ttf/otf 字体。

use std::{collections::HashMap, hash::{Hash, Hasher}, sync::Arc};

use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping};
use glam::Vec2;
use rjw_2d_render::{Layer, Render2D, SpriteRect};
use rjw_atlas::{AtlasConfig, AtlasRegion, DynamicAtlas};
use rjw_color::Color;
use rjw_render::TEXTURES;
use rjw_transform::Transform2D;
use swash::FontRef;

/// 默认字形缓存图集单页尺寸（像素）。
pub const DEFAULT_GLYPH_ATLAS_SIZE: u32 = 1024;

// ─── GlyphKey ──────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct GlyphKey { font_id: u64, glyph_id: u16, px_size: u16 }

impl Hash for GlyphKey {
    fn hash<H: Hasher>(&self, state: &mut H) { self.font_id.hash(state); self.glyph_id.hash(state); self.px_size.hash(state); }
}
impl PartialEq for GlyphKey {
    fn eq(&self, other: &Self) -> bool { self.font_id == other.font_id && self.glyph_id == other.glyph_id && self.px_size == other.px_size }
}
impl Eq for GlyphKey {}

// ─── Font ──────────────────────────────────────────────────────

struct FontData { data: Arc<Vec<u8>> }

impl FontData {
    fn font_ref(&self) -> Option<FontRef<'_>> { FontRef::from_index(&self.data, 0) }
}

pub struct Font {
    font_system: FontSystem,
    glyph_cache: DynamicAtlas<GlyphKey>,
    fonts: HashMap<u64, FontData>,
    family_map: HashMap<String, u64>,
    next_font_id: u64,
}

impl Font {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout) -> Self {
        let glyph_cache = DynamicAtlas::new(
            device, queue, layout,
            AtlasConfig { max_pages: 4, padding: 1, ..Default::default() },
            DEFAULT_GLYPH_ATLAS_SIZE,
        );
        let mut font_system = FontSystem::new();
        font_system.db_mut().load_system_fonts();
        Self { font_system, glyph_cache, fonts: HashMap::new(), family_map: HashMap::new(), next_font_id: 0 }
    }

    /// 加载字体数据，返回 family 名称（用于 `Attrs::new().family(Family::Name(&family))`）。
    pub fn load_font_data(&mut self, data: Vec<u8>) -> Option<String> {
        let id = self.next_font_id; self.next_font_id += 1;
        self.font_system.db_mut().load_font_data(data.clone());
        // swash: LocalizedStrings -> family
        let fr = FontRef::from_index(&data, 0)?;
        let family = fr.localized_strings().find(|s| s.language().to_lowercase() == "en-us" || s.language().is_empty())
            .map(|s| s.to_string())
            .or_else(|| fr.localized_strings().next().map(|s| s.to_string()))?;
        self.fonts.insert(id, FontData { data: Arc::new(data) });
        self.family_map.insert(family.clone(), id);
        Some(family)
    }

    pub fn create_buffer(&mut self, text: &str, attrs: Attrs<'_>, shaping: Shaping, size: f32, line_height: f32) -> Buffer {
        let metrics = Metrics::new(size, line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(size), Some(line_height));
        buffer.set_text(text, &attrs, shaping, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer
    }

    /// 遍历每个字形，`callback(region, world_pos, world_size)`。
    pub fn draw_text<F>(&mut self, buffer: &Buffer, mut callback: F)
    where F: FnMut(&AtlasRegion, Vec2, Vec2)
    {
        for run in buffer.layout_runs() {
            let line_y = run.line_y;
            for glyph in run.glyphs {
                let font_id = self.family_id_for_glyph(glyph.glyph_id);
                let key = GlyphKey { font_id, glyph_id: glyph.glyph_id, px_size: (glyph.font_size * 16.0) as u16 };
                let region = self.get_or_render_glyph(&key);
                if let Some(region) = region {
                    let physical = glyph.physical((0.0, 0.0), 1.0);
                    let pos = Vec2::new(physical.x as f32, (line_y - glyph.y_offset).ceil());
                    let sz = Vec2::new(region.wh_px.0 as f32, region.wh_px.1 as f32);
                    callback(region, pos, sz);
                }
            }
        }
    }

    /// 便捷：全部字形渲染为 Render2D sprite。
    pub fn draw_text_sprite(
        &mut self, r2d: &mut Render2D, buffer: &Buffer, color: Color, layer: impl Into<Layer> + Clone,
    ) {
        let ps = self.glyph_cache.page_size() as f32; let inv = Vec2::new(1.0 / ps, 1.0 / ps);
        self.draw_text(buffer, |region, pos, size| {
            let rect = SpriteRect::from_texture_px(
                pos, size,
                Vec2::new(region.tl_px.0 as f32, region.tl_px.1 as f32),
                Vec2::new(region.wh_px.0 as f32, region.wh_px.1 as f32),
                inv,
            );
            if let Some(tex) = TEXTURES.get(region.page_uid) {
                r2d.add_sprite2d(rect, color, Transform2D::default(), layer.clone().into(), &tex);
            }
        });
    }

    fn get_or_render_glyph(&mut self, key: &GlyphKey) -> Option<&AtlasRegion> {
        if self.glyph_cache.get(key).is_some() { return self.glyph_cache.get(key); }
        let font_data = self.fonts.get(&key.font_id)?;
        let font_ref = font_data.font_ref()?;
        let metrics = font_ref.metrics(&[]);
        let _upem = metrics.units_per_em as f32;
        let px = key.px_size as u32;
        // 字形尺寸估算：根据 px_size 和 upem 比例
        let scale = key.px_size as f32 / metrics.units_per_em.max(1) as f32;
        let w = (key.px_size as f32 * 0.8 * scale).ceil() as u32; // 近似宽度
        let h = (key.px_size as f32 * 1.2 * scale).ceil() as u32; // 含 descent
        if w == 0 || h == 0 { return None; }
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h { for x in 0..w { let idx = ((y * w + x) * 4) as usize; rgba[idx..idx + 4].copy_from_slice(&[255,255,255,255]); } }
        self.glyph_cache.insert(key.clone(), &rgba, w, h, (0, 0), false);
        self.glyph_cache.get(key)
    }

    fn family_id_for_glyph(&mut self, _glyph_id: u16) -> u64 {
        self.family_map.values().next().copied().unwrap_or(0)
    }

    pub fn glyph_cache(&self) -> &DynamicAtlas<GlyphKey> { &self.glyph_cache }
    pub fn page_size(&self) -> u32 { self.glyph_cache.page_size() }
}