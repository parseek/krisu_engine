//! 文本渲染：基于 `cosmic-text` 排版 + `swash` 字形光栅化 + `DynamicAtlas` 字形缓存。
//!
//! - `Text`：持有字体系统、字形缓存图集（key=GlyphKey）、字体数据。自动加载系统字体作为默认回退。
//! - `draw_label(r2d, text, ...)`：最简一行文本渲染，无需手动排版。
//! - `draw_text(buffer, callback)`：遍历已排版 Buffer 的每个字形，查询/渲染缓存，调用闭包。

use std::{collections::HashMap, hash::{Hash, Hasher}, sync::Arc};

pub use cosmic_text::{Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use glam::Vec2;
use rjw_2d_render::{Layer, Render2D, SpriteRect};
use rjw_atlas::{AtlasConfig, AtlasRegion, DynamicAtlas};
use rjw_color::Color;
use rjw_render::TEXTURES;
use rjw_transform::Transform2D;
pub use cosmic_text;
use swash::FontRef;

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

// ─── Text ──────────────────────────────────────────────────────

struct FontData { data: Arc<Vec<u8>> }

impl FontData {
    fn font_ref(&self) -> Option<FontRef<'_>> { FontRef::from_index(&self.data, 0) }
}

pub struct Text {
    font_system: FontSystem,
    glyph_cache: DynamicAtlas<GlyphKey>,
    fonts: HashMap<u64, FontData>,
    family_map: HashMap<String, u64>,
    next_font_id: u64,
    /// 默认字体 family（自动回退用）。
    default_family: String,
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

        let mut slf = Self {
            font_system, glyph_cache, fonts: HashMap::new(), family_map: HashMap::new(),
            next_font_id: 0, default_family: String::new(),
        };

        // 自动注册第一个系统字体（从 fontdb faces 迭代器）
        {
            let db = slf.font_system.db();
            let mut first_id = None;
            for x in db.faces() {
                first_id = Some(x.id);
                break;
            }
            if let Some(id) = first_id {
                let mut data: Option<Vec<u8>> = None;
                db.with_face_data(id, |font_data, _fi| { data = Some(font_data.to_vec()); });
                if let Some(bytes) = data {
                    let family = slf.load_font_data(bytes);
                    if let Some(fam) = family {
                        slf.default_family = fam;
                    }
                }
            }
        }
        slf
    }

    pub fn load_font_data(&mut self, data: Vec<u8>) -> Option<String> {
        let id = self.next_font_id; self.next_font_id += 1;
        self.font_system.db_mut().load_font_data(data.clone());
        let fr = FontRef::from_index(&data, 0)?;
        let family = fr.localized_strings()
            .find(|s| s.language().to_lowercase() == "en-us" || s.language().is_empty())
            .map(|s| s.to_string())
            .or_else(|| fr.localized_strings().next().map(|s| s.to_string()))?;
        self.fonts.insert(id, FontData { data: Arc::new(data) });
        self.family_map.insert(family.clone(), id);
        Some(family)
    }

    pub fn create_buffer(&mut self, text: &str, attrs: Attrs<'_>, size: f32, line_height: f32, align: Align) -> Buffer {
        let metrics = Metrics::new(size, line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let wrap_width = 1024.0f32.max(size * text.len() as f32 * 0.7);
        buffer.set_size(Some(wrap_width), Some(line_height));
        buffer.set_text(text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer
    }

    pub fn draw_text<F>(&mut self, buffer: &Buffer, mut callback: F)
    where F: FnMut(&AtlasRegion, Vec2, Vec2)
    {
        for run in buffer.layout_runs() {
            let line_y = run.line_y;
            for glyph in run.glyphs {
                let font_id = self.family_id_for_glyph(glyph.glyph_id);
                let key = GlyphKey { font_id, glyph_id: glyph.glyph_id, px_size: glyph.font_size as u16 };
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
        let px = key.px_size as f32;
        // swash 0.2.9 正确渲染 API
        use swash::scale::*;
        use swash::zeno::{Format, Vector};
        let glyph_id = swash::GlyphId::from(key.glyph_id);
        let mut ctx = ScaleContext::new();
        let mut scaler = ctx.builder(font_ref).size(px).hint(true).build();
        let image = Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ])
        .format(Format::Alpha)
        .offset(Vector::new(0.0, 0.0))
        .render(&mut scaler, glyph_id)?;
        let w = image.placement.width as u32;
        let h = image.placement.height as u32;
        if w == 0 || h == 0 { return None; }
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        match image.content {
            image::Content::Color => {
                for row in 0..h as usize {
                    for col in 0..w as usize {
                        let src = (row * w as usize + col) * 4;
                        let dst = src;
                        rgba[dst..dst + 4].copy_from_slice(&image.data[src..src + 4]);
                    }
                }
            }
            image::Content::Mask => {
                for row in 0..h as usize {
                    for col in 0..w as usize {
                        let src = row * w as usize + col;
                        let dst = (row * w as usize + col) * 4;
                        if src < image.data.len() {
                            let a = image.data[src];
                            rgba[dst] = 255;
                            rgba[dst + 1] = 255;
                            rgba[dst + 2] = 255;
                            rgba[dst + 3] = a;
                        }
                    }
                }
            }
            _ => {}
        }
        self.glyph_cache.insert(key.clone(), &rgba, w, h, (0, 0), false);
        self.glyph_cache.get(key)
    }

    fn family_id_for_glyph(&mut self, _glyph_id: u16) -> u64 {
        self.family_map.values().next().copied().unwrap_or(0)
    }

    pub fn glyph_cache(&self) -> &DynamicAtlas<GlyphKey> { &self.glyph_cache }
    pub fn page_size(&self) -> u32 { self.glyph_cache.page_size() }

    /// 简便方法：一键渲染文本精灵。
    ///
    /// `family` 是字体名（如 `"SimHei"`）；传空字符串自动回退到系统默认字体。
    /// `pos` 是文本左上角世界坐标。
    pub fn draw_label(
        &mut self, r2d: &mut Render2D, text: &str, color: Color, size: f32, line_height: f32,
        pos: Vec2, family: &str, align: Align, layer: impl Into<Layer> + Clone,
    ) {
        let default_family = self.default_family.clone();
        let family = if family.is_empty() { &default_family } else { family };
        let attrs = Attrs::new().family(Family::Name(family));
        let buf = self.create_buffer(text, attrs, size, line_height, align);
        let origin = self.buffer_origin(&buf);
        let ps = self.glyph_cache.page_size() as f32; let inv = Vec2::new(1.0 / ps, 1.0 / ps);
        self.draw_text(&buf, |region, glyph_pos, glyph_size| {
            let world_tl = pos + glyph_pos - origin;
            let rect = SpriteRect::from_texture_px(
                world_tl, glyph_size,
                Vec2::new(region.tl_px.0 as f32, region.tl_px.1 as f32),
                Vec2::new(region.wh_px.0 as f32, region.wh_px.1 as f32),
                inv,
            );
            if let Some(tex) = TEXTURES.get(region.page_uid) {
                r2d.add_sprite2d(rect, color, Transform2D::default(), layer.clone().into(), &tex);
            }
        });
    }

    fn buffer_origin(&self, buffer: &Buffer) -> Vec2 {
        for run in buffer.layout_runs() {
            if let Some(g) = run.glyphs.first() {
                let physical = g.physical((0.0, 0.0), 1.0);
                return Vec2::new(physical.x as f32, (run.line_y - g.y_offset).ceil());
            }
        }
        Vec2::ZERO
    }
}