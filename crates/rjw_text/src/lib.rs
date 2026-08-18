//! 文本渲染：基于 `cosmic-text` 排版 + `swash` 字形光栅化 + `DynamicAtlas` 字形缓存。
//!
//! - `Text`：持有字体系统（cosmic-text `FontSystem`）、字形缓存图集（key=`cosmic_text::CacheKey`）。
//! - 性能：**LRU 排版缓存**（[`MAX_LAYOUT_CACHE`]）按 O(1) 签名预过滤命中同一输入，返回共享
//!   `Arc<Buffer>`（不深拷贝）；空格等无图字形（`no_image`）只判定一次；字形图集去碎片重排后自动同步区域。
//! - `measure` / `measure_buffer`：排版内容宽高（GUI 布局用）。
//! - `draw_text(buffer, callback)` / `draw_label_with(..., callback)`：字形回调遍历，不绑定渲染器。
//! - `draw_label` / `draw_label_ex`：一行文本直接渲染到 `Render2D`（feature = `rjw_2d_render`，默认开启）。
//! - 责任链：`text(..).size(..)...try_stack().origin(..).draw_with(..)`（[`TextLayout`] / [`TextRender`]）。

mod chain;
pub use chain::*;

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

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
use swash::zeno::{Angle, Format, Transform, Vector};

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

/// **Release 构建下**的排版缓存文本长度上限（字节）：超过此值的文本不入缓存、每帧直接整形。
///
/// Debug 构建（`cfg!(debug_assertions)`）恒缓存——Debug 整形慢 10-100 倍，缓存是刚需；
/// Release 下大文本多为动态/低频（日志、聊天、终端），缓存必然 miss 且挤占 LRU/内存，
/// 而 Release 整形又足够快，故跳过。静态大文本请由用户保存 `Arc<Buffer>` 后经
/// [`Text::render_from`] 走责任链渲染（存一次、每帧复用）。
pub const LARGE_TEXT_CACHE_LIMIT: usize = 512;

/// Release 下 `len` 字节的文本是否值得缓存。
#[inline]
fn release_caches_len(len: usize) -> bool {
    len <= LARGE_TEXT_CACHE_LIMIT
}

/// 是否对该文本启用排版缓存：Debug 恒真；Release 仅小文本。
#[inline]
fn should_use_layout_cache(len: usize) -> bool {
    cfg!(debug_assertions) || release_caches_len(len)
}

/// 按缓存策略决定是否使用内部 LRU 排版缓存（`create_buffer_policy` 用）。
#[inline]
fn should_cache_with_policy(len: usize, policy: CachePolicy) -> bool {
    match policy {
        CachePolicy::Auto => should_use_layout_cache(len),
        CachePolicy::Always => true,
        CachePolicy::Never | CachePolicy::User => false,
    }
}

/// 字形（左上角 `tl`、尺寸 `size`）是否在裁剪区 `clip` 内。
///
/// `None` = 不裁剪，恒可见；`Some` = 与裁剪区有交集才可见（区间判定，容忍负尺寸）。
#[inline]
pub(crate) fn glyph_in_clip(clip: Option<Rect>, tl: Vec2, size: Vec2) -> bool {
    match clip {
        None => true,
        Some(c) => {
            !(tl.x >= c.x + c.w || tl.x + size.x <= c.x || tl.y >= c.y + c.h || tl.y + size.y <= c.y)
        }
    }
}

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

/// cosmic-text `Attrs` 的轻量摘要（只用于缓存签名预过滤，不保证唯一；桶内仍做完整比较）。
#[inline]
fn attrs_hash(attrs: &Attrs<'_>) -> u64 {
    let mut h = DefaultHasher::new();
    attrs.weight.0.hash(&mut h);
    (match attrs.style {
        cosmic_text::Style::Normal => 0u8,
        cosmic_text::Style::Italic => 1,
        cosmic_text::Style::Oblique => 2,
    })
    .hash(&mut h);
    (match attrs.stretch {
        cosmic_text::Stretch::UltraCondensed => 0u8,
        cosmic_text::Stretch::ExtraCondensed => 1,
        cosmic_text::Stretch::Condensed => 2,
        cosmic_text::Stretch::SemiCondensed => 3,
        cosmic_text::Stretch::Normal => 4,
        cosmic_text::Stretch::SemiExpanded => 5,
        cosmic_text::Stretch::Expanded => 6,
        cosmic_text::Stretch::ExtraExpanded => 7,
        cosmic_text::Stretch::UltraExpanded => 8,
    })
    .hash(&mut h);
    attrs.metadata.hash(&mut h);
    h.finish()
}

/// 排版缓存键：影响 cosmic-text 排版的全部输入（文本 / 字号 / 行高 / 对齐 / 完整 attrs）。
///
/// `sig` 是 **O(1) 签名**（长度 + 首尾字节 + 各数值字段 + attrs 摘要，不哈希全文、不分配），
/// 用于缓存桶预过滤；命中路径无需 `to_owned` 拷贝全文。
///
/// 命中缓存时返回共享的 [`Arc<Buffer>`]，跳过昂贵的 `Shaping::Advanced` 整形。
/// 注意：缓存仅按上述输入区分；若之后调用 [`Text::load_font_data`] 追加字体，
/// 已缓存排版不会自动使用新字体（新文本自然不受影响）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct LayoutCacheKey {
    text: String,
    size_bits: u32,
    line_height_bits: u32,
    align: u8,
    attrs: AttrsOwned,
    sig: u64,
}

impl LayoutCacheKey {
    fn new(text: &str, attrs: &Attrs<'_>, size: f32, line_height: f32, align: Align) -> Self {
        let size_bits = size.to_bits();
        let line_height_bits = line_height.to_bits();
        let align = align_disc(align);
        let sig = Self::sig_of(text, attrs, size_bits, line_height_bits, align);
        Self { text: text.to_owned(), size_bits, line_height_bits, align, attrs: AttrsOwned::new(attrs), sig }
    }

    /// O(1) 签名：长度 + 首尾字节 + 数值字段 + attrs 摘要。
    fn sig_of(text: &str, attrs: &Attrs<'_>, size_bits: u32, line_height_bits: u32, align: u8) -> u64 {
        let mut h = DefaultHasher::new();
        text.len().hash(&mut h);
        if let Some(&b) = text.as_bytes().first() { b.hash(&mut h); }
        if let Some(&b) = text.as_bytes().last() { b.hash(&mut h); }
        size_bits.hash(&mut h);
        line_height_bits.hash(&mut h);
        align.hash(&mut h);
        attrs_hash(attrs).hash(&mut h);
        h.finish()
    }
}

/// 缓存条目。
struct LayoutCacheEntry {
    key: LayoutCacheKey,
    buffer: Arc<Buffer>,
    last: u64,
}

/// 排版缓存容器：**LRU 回收**。按 O(1) 签名分桶（`sig -> entries`，桶内条目极少），
/// 命中路径不构造完整键（无全文拷贝/哈希），桶内才做完整字段比较。
struct LayoutCache {
    buckets: HashMap<u64, Vec<LayoutCacheEntry>>,
    seq: u64,
    cap: usize,
}

impl LayoutCache {
    fn new() -> Self {
        Self::with_cap(MAX_LAYOUT_CACHE)
    }

    fn with_cap(cap: usize) -> Self {
        Self { buckets: HashMap::new(), seq: 0, cap }
    }

    fn len(&self) -> usize {
        self.buckets.values().map(|b| b.len()).sum()
    }

    /// 命中则刷新该条目的最近使用序号，返回共享 `Arc<Buffer>`（O(1)）。
    fn find(&mut self, sig: u64, matches: impl Fn(&LayoutCacheKey) -> bool) -> Option<Arc<Buffer>> {
        let bucket = self.buckets.get_mut(&sig)?;
        let pos = bucket.iter().position(|e| matches(&e.key))?;
        let entry = &mut bucket[pos];
        self.seq = self.seq.wrapping_add(1);
        entry.last = self.seq;
        Some(entry.buffer.clone())
    }

    /// 插入；已满时淘汰全局最久未使用的条目。
    fn insert(&mut self, key: LayoutCacheKey, buf: Arc<Buffer>) {
        if self.len() >= self.cap {
            let mut evict: Option<(u64, usize)> = None;
            let mut min_last = u64::MAX;
            for (sig, bucket) in &self.buckets {
                for (i, e) in bucket.iter().enumerate() {
                    if e.last < min_last {
                        min_last = e.last;
                        evict = Some((*sig, i));
                    }
                }
            }
            if let Some((sig, i)) = evict {
                let bucket = self.buckets.get_mut(&sig).expect("bucket must exist");
                bucket.remove(i);
                if bucket.is_empty() {
                    self.buckets.remove(&sig);
                }
            }
        }
        self.seq = self.seq.wrapping_add(1);
        let sig = key.sig;
        self.buckets.entry(sig).or_default().push(LayoutCacheEntry { key, buffer: buf, last: self.seq });
    }

    fn clear(&mut self) {
        self.buckets.clear();
    }
}

// ─── Text ──────────────────────────────────────────────────────

// ─── 字形光栅化（swash） ────────────────────────────────────────

/// 光栅化单个字形（swash）。与 cosmic-text 自带光栅化保持一致：
/// `FAKE_ITALIC` 标记（cosmic-text 在字体无斜体字面时为 italic 排版打上）→ 对字形做
/// **14° 斜切**合成伪斜体；字体自带斜体字面时则由排版层直接选中斜体字形，本函数无需变换。
///
/// 返回 `None` 表示该字形无法产生像素（缺字体 / swash 渲染失败 / 零尺寸）。
fn swash_render_image(
    font_system: &mut FontSystem,
    context: &mut ScaleContext,
    cache_key: cosmic_text::CacheKey,
) -> Option<swash::scale::image::Image> {
    let font = font_system.get_font(cache_key.font_id, cache_key.font_weight)?;
    let mut scaler = context
        .builder(font.as_swash())
        .size(f32::from_bits(cache_key.font_size_bits))
        .hint(!cache_key.flags.contains(cosmic_text::CacheKeyFlags::DISABLE_HINTING))
        .build();
    let offset = Vector::new(cache_key.x_bin.as_float(), cache_key.y_bin.as_float());
    Render::new(&[
        Source::ColorOutline(0),
        Source::ColorBitmap(swash::scale::StrikeWith::BestFit),
        Source::Outline,
    ])
    .format(Format::Alpha)
    .offset(offset)
    .transform(if cache_key.flags.contains(cosmic_text::CacheKeyFlags::FAKE_ITALIC) {
        Some(Transform::skew(Angle::from_degrees(14.0), Angle::from_degrees(0.0)))
    } else {
        None
    })
    .render(&mut scaler, cache_key.glyph_id)
}

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

    /// 排版文本为共享 `Arc<Buffer>`（cosmic-text），内容随后经 [`Self::draw_text`] 等遍历。
    ///
    /// 相同输入（文本 / 字号 / 行高 / 对齐 / attrs）会命中内部排版缓存：**O(1) 签名预过滤**
    /// 后返回共享的 [`Arc<Buffer>`]（不深拷贝排版结果）。缓存达到 [`MAX_LAYOUT_CACHE`] 时按 LRU
    /// 淘汰最久未用条目。
    ///
    /// 缓存启用规则（见 [`LARGE_TEXT_CACHE_LIMIT`]）：**Debug 恒缓存**；**Release 仅缓存
    /// ≤ 512 字节的小文本**——大文本（多为动态/低频）不入缓存、每帧直接整形。静态大文本请
    /// 保存本方法返回的 `Arc<Buffer>`，每帧经 [`Text::render_from`] 走责任链渲染（存一次、复用）。
    ///
    /// 返回值为共享只读布局；需要修改的调用方用 [`Arc::make_mut`]（仅当缓存仍持有时才深拷贝）。
    pub fn create_buffer(
        &mut self, text: &str, attrs: Attrs<'_>, size: f32, line_height: f32, align: Align,
    ) -> Arc<Buffer> {
        self.create_buffer_policy(text, attrs, size, line_height, align, CachePolicy::Auto)
    }

    /// 同 [`Self::create_buffer`]，但排版缓存策略由调用方指定（默认 [`CachePolicy::Auto`] 即上述规则）。
    ///
    /// - [`CachePolicy::Always`]：强制进 LRU（含大文本）；
    /// - [`CachePolicy::Never`] / [`CachePolicy::User`]：不写 LRU，每帧重新整形
    ///   （`User` 语义：配合 [`Text::render_from`] / [`TextLayout::into_render_with`] 由用户持缓冲）。
    pub fn create_buffer_policy(
        &mut self, text: &str, attrs: Attrs<'_>, size: f32, line_height: f32, align: Align,
        policy: CachePolicy,
    ) -> Arc<Buffer> {
        let use_cache = should_cache_with_policy(text.len(), policy);
        if use_cache {
            let size_bits = size.to_bits();
            let line_height_bits = line_height.to_bits();
            let align_u8 = align_disc(align);
            let sig = LayoutCacheKey::sig_of(text, &attrs, size_bits, line_height_bits, align_u8);
            let matches = |k: &LayoutCacheKey| {
                k.sig == sig
                    && k.size_bits == size_bits
                    && k.line_height_bits == line_height_bits
                    && k.align == align_u8
                    && k.text == text
                    && k.attrs.as_attrs() == attrs
            };
            if let Some(cached) = self.layout_cache.find(sig, matches) {
                return cached;
            }
        }
        let metrics = Metrics::new(size, line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let wrap_width = 1024.0f32.max(size * text.len() as f32);
        // 高度传 None：cosmic-text 会按 height_opt 裁剪超出范围的行，
        // 若设成单行高度会导致多行文本只保留第一行。
        buffer.set_size(Some(wrap_width), None);
        buffer.set_text(text, &attrs, Shaping::Advanced, Some(align));
        buffer.shape_until_scroll(&mut self.font_system, false);
        let arc = Arc::new(buffer);
        if use_cache {
            let key = LayoutCacheKey::new(text, &attrs, size, line_height, align);
            self.layout_cache.insert(key, arc.clone());
        }
        arc
    }

    /// 统一字形遍历内核：先确保全部字形已渲染入图集，再逐个回调。
    ///
    /// `callback(region, world_pos, world_size)`：
    /// - `world_pos` — 字形精灵**左上角**坐标（已含 bearing），相对文本**视觉原点**
    ///   （第一个字形 bearing 恢复后的左上角），再叠加 `base` 偏移。
    /// - `world_size` — 字形精灵像素宽高。
    fn visit_glyphs<F>(&mut self, buffer: &Buffer, base: Vec2, clip: Option<Rect>, mut callback: F)
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
        // pass 2：逐个字形回调（精灵左上角 = 排版位置 + bearing 偏移）；
        // `clip`（相对文本视觉原点的局部坐标）为 Some 时跳过区外字形。
        for run in buffer.layout_runs() {
            let line_y = run.line_y;
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                if let Some(loc) = self.locations.get(&physical.cache_key) {
                    // 字形相对文本视觉原点的偏移：**全部操作数为整数**——
                    // `physical.x` / `loc.left` / `loc.top` 为整型，`line_y` 先取整，
                    // `origin`（[`Self::buffer_origin`]）同为整数。整数加减法无小数
                    // 误差累加；最终再 `ceil` 只吸收外部 `base`（世界放置）的小数。
                    let glyph_pos = Vec2::new(
                        physical.x as f32 + loc.left as f32,
                        line_y.ceil() - loc.top as f32,
                    );
                    let world_tl = base + glyph_pos - origin;
                    let glyph_size = Vec2::new(loc.region.wh_px.0 as f32, loc.region.wh_px.1 as f32);
                    let tl = world_tl - base; // 相对文本视觉原点
                    if !glyph_in_clip(clip, tl, glyph_size) {
                        continue;
                    }
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
        self.visit_glyphs(buffer, Vec2::ZERO, None, callback);
    }

    /// 同 [`Self::draw_text`]，但 `clip`（相对文本视觉原点的局部坐标）为 `Some` 时
    /// 跳过裁剪区外的字形（回调只收到可见字形）。
    pub fn draw_text_clipped<F>(&mut self, buffer: &Buffer, clip: Option<Rect>, callback: F)
    where F: FnMut(&AtlasRegion, Vec2, Vec2)
    {
        self.visit_glyphs(buffer, Vec2::ZERO, clip, callback);
    }

    /// 内部：渲染+打包一个 swash 字形，写入 DynamicAtlas。
    ///
    /// 无法产生像素的字形（空格/零尺寸、缺字体、swash 渲染失败）会记入 [`Self::no_image`]，
    /// 避免每帧重复光栅化（Debug 下 swash 渲染是主要开销）。
    /// 渲染细节见 [`swash_render_image`]（含 `FAKE_ITALIC` 伪斜体 14° 斜切）。
    fn rasterize_and_pack(&mut self, cache_key: cosmic_text::CacheKey) {
        let Some(image) = swash_render_image(&mut self.font_system, &mut self.scale_context, cache_key) else {
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
        self.visit_glyphs(&buf, pos - offset, None, callback);
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
    ///
    /// **整数不变量**：返回坐标均为**整数像素**（y 轴对 `line_y` 先 `ceil` 再减整型
    /// bearing）——与 [`Self::visit_glyphs`] / [`collect_glyphs`] 的字形坐标一致，
    /// 保证后续所有加减法操作数都是整数（无小数误差累加、无亚像素摆放）。
    fn buffer_origin(&self, buffer: &Buffer) -> Vec2 {
        for run in buffer.layout_runs() {
            if let Some(g) = run.glyphs.first() {
                let physical = g.physical((0.0, 0.0), 1.0);
                if let Some(loc) = self.locations.get(&physical.cache_key) {
                    return Vec2::new(
                        // **笔位 x**（不 + 左侧 bearing）：字形按各自 bearing 相对笔位
                        // 摆放。若以首字形墨迹左缘为原点，首字形的大 left bearing
                        // （如全角 ！ 的 ~9px 空位）会把**后续所有字形整体左移**
                        // 相同距离（叠进前段字距空间）；中文开头 bearing 小而一致
                        // 所以不明显。
                        physical.x as f32,
                        // y 先取整再参与减法：`ceil(line_y) - loc.top` 是整数，
                        // 与字形坐标（同为取整后整数）相减不会引入小数。
                        run.line_y.ceil() - loc.top as f32,
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
        assert_eq!(k1.sig, k2.sig, "相同输入签名应一致");
        assert_ne!(k1, k3, "字号不同应区分");
        assert_ne!(k1, k4, "对齐不同应区分");
        assert_ne!(k1, k5, "文本不同应区分");
        assert_ne!(k1, k6, "font family 不同应区分");
        // O(1) 签名应区分大部分不同输入（预过滤失效也不会错，桶内会完整比较）。
        assert_ne!(k1.sig, k3.sig, "字号不同签名应区分");
        assert_ne!(k1.sig, k4.sig, "对齐不同签名应区分");
    }

    #[test]
    fn layout_cache_release_large_text_rule() {
        // Release 构建下：≤ LARGE_TEXT_CACHE_LIMIT 的文本才缓存，更大文本不缓存。
        // （`cfg!(debug_assertions)` 是编译期常量，测试构建恒真；此测试直接验证纯规则函数。）
        assert!(release_caches_len(0), "空/短文本应缓存");
        assert!(release_caches_len(LARGE_TEXT_CACHE_LIMIT), "恰好等于上限应缓存");
        assert!(!release_caches_len(LARGE_TEXT_CACHE_LIMIT + 1), "超过上限不应缓存（Release）");
        assert!(!release_caches_len(4096), "大文本不应缓存（Release）");
        // Debug 构建应恒缓存（`should_use_layout_cache` 由编译期开关决定，这里只验证结构性正确）。
        assert!(cfg!(debug_assertions) || !release_caches_len(1024));
    }

    #[test]
    fn layout_cache_lru_evicts_least_recent() {
        fn make(fs: &mut FontSystem, text: &str) -> (LayoutCacheKey, Arc<Buffer>) {
            let attrs = Attrs::new();
            let metrics = Metrics::new(14.0, 20.0);
            let mut buf = Buffer::new(fs, metrics);
            buf.set_size(Some(1024.0), None);
            buf.set_text(text, &attrs, Shaping::Advanced, Some(Align::Left));
            buf.shape_until_scroll(fs, false);
            (LayoutCacheKey::new(text, &attrs, 14.0, 20.0, Align::Left), Arc::new(buf))
        }
        fn hit(cache: &mut LayoutCache, key: &LayoutCacheKey) -> Option<Arc<Buffer>> {
            cache.find(key.sig, |k| k == key)
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
        assert_eq!(cache.len(), 3);
        // 命中 k1 → k1 变为最新，且返回同一共享 Arc（不深拷贝）。
        let hit1 = hit(&mut cache, &k1).expect("k1 应命中");
        let hit2 = hit(&mut cache, &k1).expect("k1 再次应命中");
        assert!(Arc::ptr_eq(&hit1, &hit2), "缓存命中应返回同一 Arc<Buffer>");
        // 插入 k4 → 已满，应淘汰最久未使用的 k2。
        cache.insert(k4.clone(), b4);
        assert_eq!(cache.len(), 3, "满容量后插入应淘汰一个条目");
        assert!(hit(&mut cache, &k1).is_some(), "k1 应保留（最近命中过）");
        assert!(hit(&mut cache, &k3).is_some(), "k3 应保留");
        assert!(hit(&mut cache, &k4).is_some(), "k4 应保留（新插入）");
        assert!(hit(&mut cache, &k2).is_none(), "LRU 应淘汰最久未使用的 k2");
        cache.clear();
        assert_eq!(cache.len(), 0);
    }

    /// italic 在 cosmic-text 排版层的真实行为（视觉呈现由光栅化端决定）：
    /// - 字体带斜体字面（如 Times New Roman）→ 选中真正的斜体字形（font_id/glyph_id 变化）；
    /// - 字体无斜体字面（如 SimHei）→ 字形不变，但 cache key 打上 `FAKE_ITALIC` 标记，
    ///   本 crate 的光栅化据此做 14° 斜切合成伪斜体（见 [`swash_render_image`] 与下个测试）。
    #[test]
    fn italic_selection_depends_on_face() {
        fn shape(fs: &mut FontSystem, family: &str, italic: bool) -> (Vec<(u64, u32)>, bool) {
            let metrics = Metrics::new(14.0, 20.0);
            let mut buf = Buffer::new(fs, metrics);
            buf.set_size(Some(1024.0), None);
            let style = if italic {
                cosmic_text::Style::Italic
            } else {
                cosmic_text::Style::Normal
            };
            let attrs = Attrs::new().family(Family::Name(family)).style(style);
            buf.set_text("Hello", &attrs, Shaping::Advanced, Some(Align::Left));
            buf.shape_until_scroll(fs, false);
            let mut ids = Vec::new();
            let mut fake = false;
            for run in buf.layout_runs() {
                for g in run.glyphs.iter() {
                    let k = g.physical((0.0, 0.0), 1.0).cache_key;
                    // fontdb::ID 是不透明 key，哈希后比较（同一 FontSystem 内确定性一致）
                    let mut h = DefaultHasher::new();
                    k.font_id.hash(&mut h);
                    ids.push((h.finish(), k.glyph_id as u32));
                    fake |= k.flags.contains(cosmic_text::CacheKeyFlags::FAKE_ITALIC);
                }
            }
            (ids, fake)
        }
        let mut fs = FontSystem::new();
        fs.db_mut().load_system_fonts();
        // 有斜体字面 → 真正切换字形，且不需要伪斜体标记
        let (kn, faken) = shape(&mut fs, "Times New Roman", false);
        let (ki, fakei) = shape(&mut fs, "Times New Roman", true);
        assert!(!kn.is_empty(), "Times New Roman 应命中字形");
        assert_ne!(kn, ki, "Times New Roman：italic 应切换到真正的斜体字形");
        assert!(!faken && !fakei, "Times New Roman：正常/斜体均不应打 FAKE_ITALIC");
        // 无斜体字面 → 字形不变，仅打 FAKE_ITALIC 标记（光栅化端据其做 14° 斜切，见下个测试）
        let (sn, faken2) = shape(&mut fs, "SimHei", false);
        let (si, fakei2) = shape(&mut fs, "SimHei", true);
        assert!(!sn.is_empty(), "SimHei 应命中字形");
        assert_eq!(sn, si, "SimHei 无斜体字面：字形身份应相同（复用 regular 字形）");
        assert!(!faken2, "SimHei normal 不应打 FAKE_ITALIC");
        assert!(fakei2, "SimHei italic 应打 FAKE_ITALIC（由光栅化端决定是否合成斜切）");
    }

    /// 像素级验证：无斜体字面的 SimHei 走 `FAKE_ITALIC` 时，[`swash_render_image`]
    /// 应输出**不同的像素**（14° 斜切伪斜体），证明 italic 在光栅化端真正生效。
    #[test]
    fn fake_italic_skews_simhei_glyphs() {
        let mut fs = FontSystem::new();
        fs.db_mut().load_system_fonts();
        let metrics = Metrics::new(32.0, 40.0);
        let mut buf = Buffer::new(&mut fs, metrics);
        buf.set_size(Some(1024.0), None);
        let attrs = Attrs::new()
            .family(Family::Name("SimHei"))
            .style(cosmic_text::Style::Italic);
        buf.set_text("A", &attrs, Shaping::Advanced, Some(Align::Left));
        buf.shape_until_scroll(&mut fs, false);
        let mut key = None;
        for run in buf.layout_runs() {
            for g in run.glyphs.iter() {
                key = Some(g.physical((0.0, 0.0), 1.0).cache_key);
            }
        }
        let key = key.expect("SimHei 'A' 应命中字形");
        assert!(key.flags.contains(cosmic_text::CacheKeyFlags::FAKE_ITALIC), "italic 应带 FAKE_ITALIC 标记");
        let mut ctx = ScaleContext::new();
        // 同一个字形，仅去掉 FAKE_ITALIC 标记 → 应渲染出不同的像素
        let normal = swash_render_image(&mut fs, &mut ctx, cosmic_text::CacheKey {
            flags: cosmic_text::CacheKeyFlags::empty(),
            ..key
        }).expect("normal 渲染应成功");
        let italic = swash_render_image(&mut fs, &mut ctx, key).expect("fake italic 渲染应成功");
        assert!(!normal.data.is_empty() && !italic.data.is_empty(), "两种渲染都应有像素");
        assert_ne!(normal.data, italic.data, "FAKE_ITALIC 斜切应改变 SimHei 光栅化像素");
    }

    #[test]
    fn glyph_in_clip_culls_outside_glyphs() {
        let clip = Some(Rect::new(0.0, 0.0, 100.0, 50.0));
        // 完全在区内 → 可见
        assert!(glyph_in_clip(clip, Vec2::new(10.0, 10.0), Vec2::new(20.0, 20.0)));
        // 部分相交 → 可见（保守）
        assert!(glyph_in_clip(clip, Vec2::new(90.0, 0.0), Vec2::new(20.0, 20.0)));
        // 完全在区外 → 剔除
        assert!(!glyph_in_clip(clip, Vec2::new(200.0, 0.0), Vec2::new(20.0, 20.0)), "右侧外");
        assert!(!glyph_in_clip(clip, Vec2::new(0.0, 100.0), Vec2::new(20.0, 20.0)), "下方外");
        // 边界接触（区间为半开：接触不算相交）→ 剔除
        assert!(!glyph_in_clip(clip, Vec2::new(100.0, 0.0), Vec2::new(10.0, 10.0)), "右沿接触应剔除");
        assert!(!glyph_in_clip(clip, Vec2::new(0.0, 50.0), Vec2::new(10.0, 10.0)), "下沿接触应剔除");
        // None = 不裁剪
        assert!(glyph_in_clip(None, Vec2::new(99999.0, 99999.0), Vec2::new(1.0, 1.0)));
    }

    #[test]
    fn cache_policy_rules() {
        // Auto：Debug 恒缓存；Release 仅小文本
        assert!(should_cache_with_policy(10, CachePolicy::Auto) == cfg!(debug_assertions) || cfg!(debug_assertions));
        assert!(should_cache_with_policy(LARGE_TEXT_CACHE_LIMIT + 1, CachePolicy::Auto) == cfg!(debug_assertions));
        // Always 强制缓存；Never/User 不缓存
        assert!(should_cache_with_policy(usize::MAX, CachePolicy::Always));
        assert!(!should_cache_with_policy(0, CachePolicy::Never));
        assert!(!should_cache_with_policy(0, CachePolicy::User));
    }
}
