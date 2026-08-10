//! 文本渲染：基于 `cosmic-text` 排版 + `swash` 字形光栅化 + `DynamicAtlas` 字形缓存。
//!
//! - `Text`：持有字体系统（cosmic-text `FontSystem`）、字形缓存图集（key=`cosmic_text::CacheKey`）。
//! - 性能：**LRU 排版缓存**（[`MAX_LAYOUT_CACHE`]）跨帧复用同一输入的 cosmic-text 排版；空格等无图字形
//!   （`no_image`）只判定一次；字形图集去碎片重排后自动同步区域。
//! - `measure` / `measure_buffer`：排版内容宽高（GUI 布局用）。
//! - `draw_text(buffer, callback)` / `draw_label_with(..., callback)`：字形回调遍历，不绑定渲染器。
//! - `draw_label` / `draw_label_ex`：一行文本直接渲染到 `Render2D`（feature = `rjw_2d_render`，默认开启）。
//! - 责任链：`text(..).size(..)...try_stack().origin(..).draw_with(..)`（[`TextLayout`] / [`TextRender`]）。

mod chain;
pub use chain::*;

use std::collections::HashMap;

pub use cosmic_text::{Align, Attrs, AttrsOwned, Buffer, Family, FontSystem, Metrics, Shaping};
use glam::Vec2;
#[cfg(feature = "rjw_2d_render")]
use rjw_2d_render::{Layer, Render2D, SpriteRect};
use rjw_atlas::{AtlasConfig, AtlasRegion, DynamicAtlas};
#[cfg(feature = "rjw_2d_render")]
use rjw_color::Color;
#[cfg(feature = "rjw_2d_render")]
use rjw_render::TEXTURES;
#[cfg(feature = "rjw_2d_render")]
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

// ─── 排版缓存 ──────────────────────────────────────────────────

/// 排版缓存条目数上限：达到后按 **LRU** 淘汰最久未使用的条目（静态标签通常远小于此值）。
pub const MAX_LAYOUT_CACHE: usize = 128;

/// cosmic-text `Align` 的 u8 判别（`Align` 未实现 `Hash`，缓存键需可哈希表示）。
#[inline]
fn align_disc(align: Align) -> u8 {
    match align {
        Align::Left => 0,
        Align::Right => 1,
        Align::Center => 2,
        Align::Justified => 3,
        Align::End => 4,
    }
}

/// 排版缓存键：影响 cosmic-text 排版的全部输入（文本 / 字号 / 行高 / 对齐 / 完整 attrs）。
///
/// 命中缓存时直接克隆已排版的 [`Buffer`]，跳过昂贵的 `Shaping::Advanced` 整形——
/// 这是 Debug 构建下静态文本每帧重新排版（约 10–20ms）的主要瓶颈。
///
/// 注意：缓存仅按上述输入区分；若之后调用 [`Text::load_font_data`] 追加字体，
/// 已缓存排版不会自动使用新字体（新文本自然不受影响）。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct LayoutCacheKey {
    text: String,
    size_bits: u32,
    line_height_bits: u32,
    align: u8,
    attrs: AttrsOwned,
}

impl LayoutCacheKey {
    fn new(text: &str, attrs: &Attrs<'_>, size: f32, line_height: f32, align: Align) -> Self {
        Self {
            text: text.to_owned(),
            size_bits: size.to_bits(),
            line_height_bits: line_height.to_bits(),
            align: align_disc(align),
            attrs: AttrsOwned::new(attrs),
        }
    }
}

/// 排版缓存容器：**LRU 回收**。值带单调“最近使用序号”，达到 `cap` 时淘汰序号最小（最久未用）的条目。
struct LayoutCache {
    map: HashMap<LayoutCacheKey, (Buffer, u64)>,
    seq: u64,
    cap: usize,
}

impl LayoutCache {
    fn new() -> Self {
        Self::with_cap(MAX_LAYOUT_CACHE)
    }

    fn with_cap(cap: usize) -> Self {
        Self { map: HashMap::new(), seq: 0, cap }
    }

    /// 命中则刷新该条目的最近使用序号，返回缓存 Buffer。
    fn get(&mut self, key: &LayoutCacheKey) -> Option<&Buffer> {
        let entry = self.map.get_mut(key)?;
        self.seq = self.seq.wrapping_add(1);
        entry.1 = self.seq;
        Some(&entry.0)
    }

    /// 插入；已满时先淘汰最久未使用的条目。
    fn insert(&mut self, key: LayoutCacheKey, buf: Buffer) {
        if self.map.len() >= self.cap {
            if let Some(evict) = self.map
                .iter()
                .min_by_key(|&(_, &(_, last))| last)
                .map(|(k, _)| k.clone())
            {
                self.map.remove(&evict);
            }
        }
        self.seq = self.seq.wrapping_add(1);
        self.map.insert(key, (buf, self.seq));
    }

    fn clear(&mut self) {
        self.map.clear();
    }
}

// ─── Text ──────────────────────────────────────────────────────

pub struct Text {
    font_system: FontSystem,
    scale_context: ScaleContext,
    glyph_cache: DynamicAtlas<cosmic_text::CacheKey>,
    locations: HashMap<cosmic_text::CacheKey, GlyphLocation>,
    /// 无法产生像素的字形（空格 / 缺字体 / swash 渲染失败）：避免每帧重复光栅化。
    no_image: std::collections::HashSet<cosmic_text::CacheKey>,
    buf: TextBuffer,
    /// 排版结果缓存（LRU）：同一 (文本, 字号, 行高, 对齐, attrs) 跨帧复用，跳过重复整形。
    layout_cache: LayoutCache,
    /// 上次同步过的字形图集重排世代号（见 [`Self::sync_atlas_regions`]）。
    atlas_generation: u64,
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
            no_image: std::collections::HashSet::new(),
            buf: TextBuffer::default(),
            layout_cache: LayoutCache::new(),
            atlas_generation: 0,
        }
    }

    /// 若字形图集发生过“去碎片重排”（[`rjw_atlas::DynamicAtlas::generation`] 变化），
    /// 从图集重新拉取所有已缓存字形的 `AtlasRegion`，避免旧区域指向已搬动的像素。
    ///
    /// 在排版/光栅化循环之后（`buffer_origin` / 收集字形之前）调用。
    fn sync_atlas_regions(&mut self) {
        let generation = self.glyph_cache.generation();
        if generation == self.atlas_generation {
            return;
        }
        self.atlas_generation = generation;
        let keys: Vec<cosmic_text::CacheKey> = self.locations.keys().copied().collect();
        for key in keys {
            if let Some(region) = self.glyph_cache.get(&key) {
                if let Some(loc) = self.locations.get_mut(&key) {
                    loc.region = *region;
                }
            }
        }
    }

    /// 加载额外的字体数据（如自定义 ttf/otf 文件）；已加载的系统字体不受影响。
    pub fn load_font_data(&mut self, data: Vec<u8>) {
        self.font_system.db_mut().load_font_data(data);
        // 追加字体后旧排版与“无图字形”判定可能不再正确，作废缓存。
        self.layout_cache.clear();
        self.no_image.clear();
    }

    /// 排版文本为 cosmic-text `Buffer`（内容随后经 [`Self::draw_text`] 等遍历）。
    ///
    /// 相同输入（文本 / 字号 / 行高 / 对齐 / attrs）会命中内部排版缓存，直接克隆已排版结果，
    /// 显著降低 Debug 构建下静态文本每帧排版的成本。缓存达到 [`MAX_LAYOUT_CACHE`] 时按 LRU
    /// 淘汰最久未使用的条目，避免缓存无限增长。
    pub fn create_buffer(
        &mut self, text: &str, attrs: Attrs<'_>, size: f32, line_height: f32, align: Align,
    ) -> Buffer {
        let key = LayoutCacheKey::new(text, &attrs, size, line_height, align);
        if let Some(buf) = self.layout_cache.get(&key) {
            return buf.clone();
        }
        let metrics = Metrics::new(size, line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let wrap_width = 1024.0f32.max(size * text.len() as f32);
        // 高度传 None：cosmic-text 会按 height_opt 裁剪超出范围的行，
        // 若设成单行高度会导致多行文本只保留第一行。
        buffer.set_size(Some(wrap_width), None);
        buffer.set_text(text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.font_system, false);
        self.layout_cache.insert(key, buffer.clone());
        buffer
    }

    /// 统一字形遍历内核：先确保全部字形已渲染入图集，再逐个回调。
    ///
    /// `callback(region, world_pos, world_size)`：
    /// - `world_pos` — 字形精灵**左上角**坐标（已含 bearing），相对文本**视觉原点**
    ///   （第一个字形 bearing 恢复后的左上角），再叠加 `base` 偏移。
    /// - `world_size` — 字形精灵像素宽高。
    fn visit_glyphs<F>(&mut self, buffer: &Buffer, base: Vec2, mut callback: F)
    where F: FnMut(&AtlasRegion, Vec2, Vec2)
    {
        // pass 1：确保所有字形已渲染/打包（buffer_origin 依赖 bearing 数据）
        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let cache_key = glyph.physical((0.0, 0.0), 1.0).cache_key;
                if !self.locations.contains_key(&cache_key) && !self.no_image.contains(&cache_key) {
                    self.rasterize_and_pack(cache_key);
                }
            }
        }
        // 光栅化过程中图集可能触发去碎片重排（搬动字形），同步各字形区域。
        self.sync_atlas_regions();
        let origin = self.buffer_origin(buffer);
        // pass 2：逐个字形回调（精灵左上角 = 排版位置 + bearing 偏移）
        for run in buffer.layout_runs() {
            let line_y = run.line_y;
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                if let Some(loc) = self.locations.get(&physical.cache_key) {
                    let glyph_pos = Vec2::new(
                        physical.x as f32 + loc.left as f32,
                        line_y - loc.top as f32,
                    );
                    let world_tl = base + glyph_pos - origin;
                    let glyph_size = Vec2::new(loc.region.wh_px.0 as f32, loc.region.wh_px.1 as f32);
                    callback(&loc.region, world_tl.ceil(), glyph_size);
                }
            }
        }
    }

    /// 遍历已排版 `Buffer` 中的每个字形，`callback(region, world_pos, world_size)`。
    /// `world_pos` 是字形精灵的**左上角**坐标（已含 bearing），相对文本视觉原点。
    pub fn draw_text<F>(&mut self, buffer: &Buffer, callback: F)
    where F: FnMut(&AtlasRegion, Vec2, Vec2)
    {
        self.visit_glyphs(buffer, Vec2::ZERO, callback);
    }

    /// 内部：渲染+打包一个 swash 字形，写入 DynamicAtlas。
    ///
    /// 无法产生像素的字形（空格/零尺寸、缺字体、swash 渲染失败）会记入 [`Self::no_image`]，
    /// 避免每帧重复光栅化（Debug 下 swash 渲染是主要开销）。
    fn rasterize_and_pack(&mut self, cache_key: cosmic_text::CacheKey) {
        let Some(font) = self.font_system.get_font(cache_key.font_id, cache_key.font_weight) else {
            self.no_image.insert(cache_key);
            return;
        };
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
        let Some(image) = image else {
            self.no_image.insert(cache_key);
            return;
        };

        let w = image.placement.width;
        let h = image.placement.height;
        if w == 0 || h == 0 {
            self.no_image.insert(cache_key);
            return;
        }

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

    /// 回调版标签渲染：不绑定 `Render2D`，GUI 可自定义每个字形的绘制方式。
    ///
    /// 回调签名与 [`Self::draw_text`] 一致：`(region, world_pos, world_size)`，
    /// `world_pos` 是字形精灵**左上角**的世界坐标（已含 bearing）。
    ///
    /// `origin` 以内容宽高为单位，归一化到 [0,1]（`(0,0)` 左上角，`(0.5,0.5)` 居中）。
    /// 返回内容宽高。
    #[allow(clippy::too_many_arguments)]
    pub fn draw_label_with<F>(
        &mut self, text: &str, size: f32, line_height: f32,
        pos: Vec2, family: &str, align: Align, origin: Vec2,
        callback: F,
    ) -> Vec2
    where F: FnMut(&AtlasRegion, Vec2, Vec2)
    {
        let attrs = if family.is_empty() {
            Attrs::new()
        } else {
            Attrs::new().family(Family::Name(family))
        };
        let buf = self.create_buffer(text, attrs, size, line_height, align);
        let content_size = Text::measure_buffer(&buf);
        let offset = Vec2::new(content_size.x * origin.x, content_size.y * origin.y);
        self.visit_glyphs(&buf, pos - offset, callback);
        content_size
    }

    /// 简化便捷方法：一键渲染文本精灵（feature = `rjw_2d_render`，默认开启）。
    ///
    /// `pos` — 文本左上角世界坐标。
    /// `family` 支持自定义 family（如 `"SimHei"`）；传空字符串或无效名时自动回退到系统字体。
    #[cfg(feature = "rjw_2d_render")]
    #[allow(clippy::too_many_arguments)]
    pub fn draw_label(
        &mut self, r2d: &mut Render2D, text: &str, color: Color,
        size: f32, line_height: f32, pos: Vec2, family: &str, align: Align, layer: impl Into<Layer> + Clone,
    ) -> Vec2 {
        self.draw_label_ex(r2d, text, color, size, line_height, pos, family, align, layer, Vec2::ZERO)
    }

    /// 扩展版：`origin` 以内容宽高为单位，归一化到 [0,1]。
    /// `origin = (0,0)` 为左上角，`origin = (0.5,0.5)` 为中心点。
    ///
    /// 依赖默认 feature `rjw_2d_render`；不想要渲染绑定时改用 [`Self::draw_label_with`]。
    #[cfg(feature = "rjw_2d_render")]
    #[allow(clippy::too_many_arguments)]
    pub fn draw_label_ex(
        &mut self, r2d: &mut Render2D, text: &str, color: Color,
        size: f32, line_height: f32, pos: Vec2, family: &str, align: Align, layer: impl Into<Layer> + Clone,
        origin: Vec2,
    ) -> Vec2 {
        let ps = self.glyph_cache.page_size() as f32;
        let inv = Vec2::new(1.0 / ps, 1.0 / ps);
        self.draw_label_with(text, size, line_height, pos, family, align, origin, |region, world_tl, wh| {
            // 默认绘制：rjw_2d_render 直接方法
            let rect = SpriteRect::from_texture_px(
                world_tl, wh,
                Vec2::new(region.tl_px.0 as f32, region.tl_px.1 as f32),
                Vec2::new(region.wh_px.0 as f32, region.wh_px.1 as f32),
                inv,
            );
            if let Some(tex) = TEXTURES.get(region.page_uid) {
                r2d.add_sprite2d(rect, color, Transform2D::default(), layer.clone().into(), &tex);
            }
        })
    }

    /// 排版内容宽高（完整行盒，未滚动）：宽 = max(`line_w`)，高 = max(`line_top + line_height`) − min(`line_top`)。
    /// 无字形时返回 `Vec2::ZERO`。
    pub fn measure_buffer(buffer: &Buffer) -> Vec2 {
        let mut w: f32 = 0.0;
        let mut top = f32::MAX;
        let mut bottom = f32::MIN;
        let mut glyphs = 0usize;
        for run in buffer.layout_runs() {
            glyphs += run.glyphs.len();
            w = w.max(run.line_w);
            top = top.min(run.line_top);
            bottom = bottom.max(run.line_top + run.line_height);
        }
        if glyphs == 0 || !top.is_finite() {
            return Vec2::ZERO;
        }
        Vec2::new(w.ceil(), (bottom - top).ceil())
    }

    /// 排版 + 测量一步到位：返回内容宽高，供 GUI 布局使用（widget 尺寸）。
    pub fn measure(
        &mut self, text: &str, attrs: Attrs<'_>, size: f32, line_height: f32, align: Align,
    ) -> Vec2 {
        let buffer = self.create_buffer(text, attrs, size, line_height, align);
        Text::measure_buffer(&buffer)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn shaped(text: &str, size: f32, line_height: f32, align: Align) -> Buffer {
        let mut fs = FontSystem::new();
        fs.db_mut().load_system_fonts();
        let metrics = Metrics::new(size, line_height);
        let mut buf = Buffer::new(&mut fs, metrics);
        buf.set_size(Some(1024.0), None);
        buf.set_text(text, &Attrs::new(), Shaping::Advanced, Some(align));
        buf.shape_until_scroll(&mut fs, false);
        buf
    }

    #[test]
    fn measure_empty_returns_zero() {
        let sz = Text::measure_buffer(&shaped("", 14.0, 20.0, Align::Left));
        assert_eq!(sz, Vec2::ZERO);
    }

    #[test]
    fn measure_single_line_height_is_line_height() {
        let sz = Text::measure_buffer(&shaped("Hello", 14.0, 20.0, Align::Left));
        assert!(sz.x > 0.0, "width should be > 0");
        // 行盒高度 = 行高；旧实现（line_y + line_height）会高估约一个 ascent
        assert!((sz.y - 20.0).abs() < 1.0, "single-line height = {}", sz.y);
    }

    #[test]
    fn measure_multiline_height_is_n_lines() {
        let sz = Text::measure_buffer(&shaped("Hello\nWorld", 14.0, 20.0, Align::Left));
        assert!((sz.y - 40.0).abs() < 1.0, "two-line height = {}", sz.y);
    }

    #[test]
    fn measure_width_is_max_line_width() {
        let multi = Text::measure_buffer(&shaped("aa\naaa", 10.0, 14.0, Align::Left));
        let single = Text::measure_buffer(&shaped("aaa", 10.0, 14.0, Align::Left));
        assert!(multi.x > 0.0);
        assert!((multi.x - single.x).abs() < 1.0, "width = {} vs {}", multi.x, single.x);
        assert!(single.y > 0.0);
    }

    #[test]
    fn layout_cache_key_distinguishes_inputs() {
        let k1 = LayoutCacheKey::new("Hello", &Attrs::new(), 14.0, 20.0, Align::Left);
        let k2 = LayoutCacheKey::new("Hello", &Attrs::new(), 14.0, 20.0, Align::Left);
        let k3 = LayoutCacheKey::new("Hello", &Attrs::new(), 16.0, 20.0, Align::Left);
        let k4 = LayoutCacheKey::new("Hello", &Attrs::new(), 14.0, 20.0, Align::Center);
        let k5 = LayoutCacheKey::new("World", &Attrs::new(), 14.0, 20.0, Align::Left);
        let k6 = LayoutCacheKey::new("Hello", &Attrs::new().family(Family::Monospace), 14.0, 20.0, Align::Left);
        assert_eq!(k1, k2, "相同输入应命中同一缓存键");
        assert_ne!(k1, k3, "字号不同应区分");
        assert_ne!(k1, k4, "对齐不同应区分");
        assert_ne!(k1, k5, "文本不同应区分");
        assert_ne!(k1, k6, "font family 不同应区分");
    }

    #[test]
    fn layout_cache_lru_evicts_least_recent() {
        fn make(fs: &mut FontSystem, text: &str) -> (LayoutCacheKey, Buffer) {
            let attrs = Attrs::new();
            let metrics = Metrics::new(14.0, 20.0);
            let mut buf = Buffer::new(fs, metrics);
            buf.set_size(Some(1024.0), None);
            buf.set_text(text, &attrs, Shaping::Advanced, Some(Align::Left));
            buf.shape_until_scroll(fs, false);
            (LayoutCacheKey::new(text, &attrs, 14.0, 20.0, Align::Left), buf)
        }
        let mut fs = FontSystem::new();
        fs.db_mut().load_system_fonts();
        let mut cache = LayoutCache::with_cap(3);
        let (k1, b1) = make(&mut fs, "aaa");
        let (k2, b2) = make(&mut fs, "bbb");
        let (k3, b3) = make(&mut fs, "ccc");
        let (k4, b4) = make(&mut fs, "ddd");
        cache.insert(k1.clone(), b1);
        cache.insert(k2.clone(), b2);
        cache.insert(k3.clone(), b3);
        assert_eq!(cache.map.len(), 3);
        // 命中 k1 → k1 变为最新。
        assert!(cache.get(&k1).is_some());
        // 插入 k4 → 已满，应淘汰最久未使用的 k2。
        cache.insert(k4.clone(), b4);
        assert_eq!(cache.map.len(), 3, "满容量后插入应淘汰一个条目");
        assert!(cache.get(&k1).is_some(), "k1 应保留（最近命中过）");
        assert!(cache.get(&k3).is_some(), "k3 应保留");
        assert!(cache.get(&k4).is_some(), "k4 应保留（新插入）");
        assert!(cache.get(&k2).is_none(), "LRU 应淘汰最久未使用的 k2");
        cache.clear();
        assert_eq!(cache.map.len(), 0);
    }
}
