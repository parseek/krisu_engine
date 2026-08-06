//! 运行时动态 / 静态纹理图集。
//!
//! - `DynamicAtlas<K=String>`：Skyline 打包器，运行时插入/踢出/compact/自动新建页 + TOML 批量导入 + 自动复活。
//!   `K` 泛型键（默认 `String`），`String` 特化支持 TOML 导入/导出 + 便捷方法。
//! - `StaticAtlas`：从 TOML 反序列化预排布图集（`spr.toml`）。
//! - `AtlasRegion`：图集内精灵坐标（像素左上角 + 尺寸 + 原点偏移 + 页 uid）。
//!
//! 依赖全局纹理注册表 `rjw_render::TEXTURES`（DashMap），完全解耦 `rjw_2d_render`。

use std::{collections::HashMap, hash::Hash, sync::Arc};

use rjw_render::{ArcTextureWrapped, TextureWrapped, TEXTURES};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// 默认单页尺寸（像素）。
pub const DEFAULT_PAGE_SIZE: u32 = 2048;
/// 默认精灵寿命（帧）。
pub const DEFAULT_LIFETIME: u32 = 200;

// ─── 配置 ─────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct AtlasConfig {
    pub max_pages: usize,
    pub padding: u32,
    pub lifetime: u32,
}

impl Default for AtlasConfig {
    fn default() -> Self {
        Self { max_pages: 8, padding: 0, lifetime: DEFAULT_LIFETIME }
    }
}

// ─── AtlasRegion ──────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct AtlasRegion {
    pub tl_px: (u32, u32),
    pub wh_px: (u32, u32),
    pub origin_px: (u32, u32),
    pub page_uid: u64,
}

// ─── Skyline ──────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct SkySegment { x: u32, y: u32, w: u32 }

struct Skyline {
    segments: Vec<SkySegment>,
    page_size: u32,
}

impl Skyline {
    fn new(page_size: u32) -> Self {
        Self { segments: vec![SkySegment { x: 0, y: 0, w: page_size }], page_size }
    }

    fn allocate(&mut self, w: u32, h: u32, padding: u32) -> Option<(u32, u32)> {
        let needed = w + padding * 2;
        let mut best_idx: Option<usize> = None;
        let mut best_y = u32::MAX;
        let mut best_surplus = u32::MAX;
        for (i, seg) in self.segments.iter().enumerate() {
            if seg.w >= needed {
                let surplus = seg.w - needed;
                if seg.y < best_y || (seg.y == best_y && surplus < best_surplus) {
                    best_y = seg.y;
                    best_surplus = surplus;
                    best_idx = Some(i);
                }
            }
        }
        let best_idx = best_idx?;
        if best_y + h + padding * 2 > self.page_size { return None; }

        let seg = self.segments[best_idx];
        let x = seg.x + padding;
        let y = seg.y + padding;
        self.segments.remove(best_idx);

        let right_w = seg.w - needed;
        if right_w > 0 {
            self.segments.push(SkySegment { x: x + w + padding, y: seg.y, w: right_w });
        }
        self.segments.push(SkySegment { x: seg.x, y: y + h + padding, w: needed });
        self.segments.sort_by_key(|s| s.x);
        self.merge_adjacent();
        Some((x, y))
    }

    fn merge_adjacent(&mut self) {
        let mut i = 0;
        while i + 1 < self.segments.len() {
            let a = self.segments[i]; let b = self.segments[i + 1];
            if a.y == b.y && a.x + a.w == b.x { self.segments[i].w = a.w + b.w; self.segments.remove(i + 1); }
            else { i += 1; }
        }
    }

    fn from_occupied(page_size: u32, occupied: &[(u32, u32, u32, u32)], padding: u32) -> Self {
        let mut segments = vec![SkySegment { x: 0, y: 0, w: page_size }];
        let mut sorted: Vec<_> = occupied.iter().collect();
        sorted.sort_by_key(|&&(ox, oy, _, _)| (oy, ox));
        for &&(ox, oy, ow, oh) in &sorted {
            let px = ox.saturating_sub(padding); let py = oy.saturating_sub(padding);
            let pw = ow + padding * 2; let ph = oh + padding * 2;
            let mut i = 0;
            while i < segments.len() {
                let s = segments[i];
                let right = px + pw; let s_right = s.x + s.w;
                if s.x < right && px < s_right && s.y < py + ph && py < s.y.saturating_add(1).min(page_size) {
                    segments.remove(i);
                    if s.x < px { segments.insert(i, SkySegment { x: s.x, y: s.y, w: px - s.x }); i += 1; }
                    if s_right > right { segments.insert(i, SkySegment { x: right, y: s.y, w: s_right - right }); i += 1; }
                    if py + ph > s.y + 1 && py + ph < page_size { segments.insert(i, SkySegment { x: s.x, y: py + ph, w: pw }); i += 1; }
                } else { i += 1; }
            }
        }
        let mut slf = Self { segments, page_size };
        slf.merge_adjacent();
        slf
    }

    fn free_area(&self) -> u64 {
        self.segments.iter().map(|seg| (seg.w as u64) * self.height_below(*seg)).sum()
    }

    fn largest_free_area(&self) -> u64 {
        self.segments.iter().map(|seg| (seg.w as u64) * self.height_below(*seg)).max().unwrap_or(0)
    }

    fn height_below(&self, seg: SkySegment) -> u64 {
        let next_y = self.segments.iter().filter(|o| o.y > seg.y).map(|o| o.y).min().unwrap_or(self.page_size);
        (next_y - seg.y) as u64
    }
}

// ─── AtlasPage ────────────────────────────────────────────────

struct AtlasPage {
    texture: ArcTextureWrapped,
    skyline: Skyline,
}

impl AtlasPage {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, _layout: &wgpu::BindGroupLayout, page_size: u32) -> Self {
        let size = (page_size * page_size * 4) as usize;
        let clear = vec![0u8; size];
        let label = if cfg!(debug_assertions) { 
            static PAGE_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let f = format!("DynamicAtlas page ID: {:0>4} {}x{}", PAGE_COUNTER.load(std::sync::atomic::Ordering::Relaxed), page_size, page_size);
            PAGE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            f
        } else { 
            format!("DynamicAtlas page {}x{}", page_size, page_size) 
        };
        let tex = Arc::new(TextureWrapped::from_rgba8(device, queue, &label, &clear, page_size, page_size));
        TEXTURES.register(tex.clone());
        Self { texture: tex, skyline: Skyline::new(page_size) }
    }
}

// ─── 纹理再生 / 源数据 ────────────────────────────────────────

/// 纹理再生器：精灵被图集踢出后，可通过此 trait 重新生成 RGBA 数据。
pub trait TextureRegenerator: Send + Sync {
    fn generate(&self) -> (Vec<u8>, u32, u32);
}

enum SourceData {
    Inline(Vec<u8>, u32, u32),
    Dynamic(Box<dyn TextureRegenerator>),
}

impl SourceData {
    fn extract(&self) -> (Vec<u8>, u32, u32) {
        match self {
            Self::Inline(rgba, w, h) => (rgba.clone(), *w, *h),
            Self::Dynamic(regen) => regen.generate(),
        }
    }
    fn clone_inline(&self) -> Self {
        match self {
            Self::Inline(rgba, w, h) => Self::Inline(rgba.clone(), *w, *h),
            Self::Dynamic(_) => Self::Inline(vec![], 0, 0),
        }
    }
}

struct Tombstone {
    source: SourceData,
    origin_px: (u32, u32),
    clamp_margin: bool,
}

// ─── DynamicAtlas (K 泛型) ────────────────────────────────────

struct AtlasEntry {
    region: AtlasRegion,
    lifetime: u32,
    source: Option<SourceData>,
    alloc_tl: (u32, u32),
    alloc_wh: (u32, u32),
}

pub struct DynamicAtlas<K = String> {
    pages: Vec<AtlasPage>,
    entries: HashMap<K, AtlasEntry>,
    tombstones: HashMap<K, Tombstone>,
    config: AtlasConfig,
    page_size: u32,
    dirty: bool,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
}

/// 通用泛型方法（所有 K）。
impl<K: Hash + Eq + Clone> DynamicAtlas<K> {
    pub fn new(
        device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout,
        config: AtlasConfig, page_size: u32,
    ) -> Self {
        let device = device.clone();
        let queue = queue.clone();
        let layout = layout.clone();
        let page = AtlasPage::new(&device, &queue, &layout, page_size);
        Self { pages: vec![page], entries: HashMap::new(), tombstones: HashMap::new(), config, page_size, dirty: false, device, queue, layout }
    }

    pub fn texture_uid_of(&self, key: &K) -> Option<u64> { self.entries.get(key).map(|e| e.region.page_uid) }

    pub fn get(&mut self, key: &K) -> Option<&AtlasRegion> {
        if let Some(e) = self.entries.get_mut(key) { e.lifetime = self.config.lifetime; Some(&e.region) }
        else { None }
    }

    pub fn get_or_revive(&mut self, key: &K) -> Option<&AtlasRegion> {
        if self.entries.contains_key(key) {
            let e = self.entries.get_mut(key).unwrap();
            e.lifetime = self.config.lifetime;
            return Some(&e.region);
        }
        let tomb = self.tombstones.remove(key)?;
        let (rgba, w, h) = tomb.source.extract();
        self.insert_inner(key.clone(), &rgba, w, h, tomb.origin_px, tomb.clamp_margin)?;
        Some(&self.entries[key].region)
    }

    pub fn insert(
        &mut self, key: K, rgba: &[u8], w: u32, h: u32, origin_px: (u32, u32), clamp_margin: bool,
    ) -> Option<AtlasRegion> {
        self.insert_with_source(key, rgba, w, h, origin_px, clamp_margin, SourceData::Inline(rgba.to_vec(), w, h))
    }

    pub fn insert_dyn(
        &mut self, key: K, w: u32, h: u32, origin_px: (u32, u32), clamp_margin: bool,
        regen: Box<dyn TextureRegenerator>,
    ) -> Option<AtlasRegion> {
        let (rgba, _rw, _rh) = regen.generate();
        debug_assert_eq!(_rw, w);
        debug_assert_eq!(_rh, h);
        self.insert_with_source(key, &rgba, w, h, origin_px, clamp_margin, SourceData::Dynamic(regen))
    }

    pub fn insert_permanent(
        &mut self, key: K, rgba: &[u8], w: u32, h: u32, origin_px: (u32, u32), clamp_margin: bool,
    ) -> Option<AtlasRegion> {
        if let Some(e) = self.entries.get_mut(&key) { e.lifetime = self.config.lifetime; return Some(e.region); }
        self.tombstones.remove(&key);
        self.insert_inner(key, rgba, w, h, origin_px, clamp_margin)
    }

    fn insert_with_source(
        &mut self, key: K, rgba: &[u8], w: u32, h: u32, origin_px: (u32, u32), clamp_margin: bool, source: SourceData,
    ) -> Option<AtlasRegion> {
        if let Some(e) = self.entries.get_mut(&key) { e.lifetime = self.config.lifetime; return Some(e.region); }
        self.tombstones.remove(&key);
        let region = self.insert_inner(key.clone(), rgba, w, h, origin_px, clamp_margin)?;
        if let Some(e) = self.entries.get_mut(&key) { e.source = Some(source); }
        Some(region)
    }

    fn insert_inner(
        &mut self, key: K, rgba: &[u8], w: u32, h: u32, origin_px: (u32, u32), clamp_margin: bool,
    ) -> Option<AtlasRegion> {
        if let Some(e) = self.entries.get_mut(&key) { e.lifetime = self.config.lifetime; return Some(e.region); }
        let padding = self.config.padding;
        let (expanded_rgba, alloc_w, alloc_h, margin_offs) = if clamp_margin {
            (expand_clamp_margin(rgba, w, h), w + 2, h + 2, (1u32, 1u32))
        } else {
            (rgba.to_vec(), w, h, (0u32, 0u32))
        };
        let (page_idx, x, y) = loop {
            match self.try_alloc(alloc_w, alloc_h, padding) {
                Some(res) => break res,
                None if self.pages.len() < self.config.max_pages => {
                    self.pages.push(AtlasPage::new(&self.device, &self.queue, &self.layout, self.page_size));
                }
                _ => return None,
            }
        };
        let page = &self.pages[page_idx];
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: page.texture.raw_texture(), mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            &expanded_rgba, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(alloc_w * 4), rows_per_image: Some(alloc_h) },
            wgpu::Extent3d { width: alloc_w, height: alloc_h, depth_or_array_layers: 1 },
        );
        let region = AtlasRegion { tl_px: (x + margin_offs.0, y + margin_offs.1), wh_px: (w, h), origin_px, page_uid: page.texture.uid };
        let alloc_tl = (x - padding, y - padding);
        let alloc_wh = (alloc_w + padding * 2, alloc_h + padding * 2);
        self.entries.insert(key, AtlasEntry { region, lifetime: self.config.lifetime, source: None, alloc_tl, alloc_wh });
        Some(region)
    }

    fn try_alloc(&mut self, w: u32, h: u32, padding: u32) -> Option<(usize, u32, u32)> {
        for (i, page) in self.pages.iter_mut().enumerate() { if let Some((x, y)) = page.skyline.allocate(w, h, padding) { return Some((i, x, y)); } }
        if self.dirty { self.compact_inner(); }
        for (i, page) in self.pages.iter_mut().enumerate() { if let Some((x, y)) = page.skyline.allocate(w, h, padding) { return Some((i, x, y)); } }
        None
    }

    pub fn end_frame(&mut self) {
        let mut to_tomb: Vec<(K, Tombstone)> = Vec::new();
        let mut remove_keys: Vec<K> = Vec::new();
        for (k, e) in &self.entries {
            if e.lifetime == 0 {
                if let Some(src) = &e.source {
                    to_tomb.push((k.clone(), Tombstone { source: src.clone_inline(), origin_px: e.region.origin_px, clamp_margin: true }));
                    remove_keys.push(k.clone());
                }
            }
        }
        for k in &remove_keys { self.entries.remove(k); }
        for (k, t) in to_tomb { self.tombstones.insert(k, t); }
        for e in self.entries.values_mut() { if e.source.is_some() { e.lifetime = e.lifetime.saturating_sub(1); } }
        if !self.entries.is_empty() || self.dirty { self.dirty = true; }
    }

    pub fn compact(&mut self) { self.compact_inner(); }

    fn compact_inner(&mut self) {
        let ps = self.page_size;
        for page in &mut self.pages {
            let occupied: Vec<_> = self.entries.values().filter(|e| e.region.page_uid == page.texture.uid)
                .map(|e| (e.alloc_tl.0, e.alloc_tl.1, e.alloc_wh.0, e.alloc_wh.1)).collect();
            page.skyline = Skyline::from_occupied(ps, &occupied, 0);
        }
        self.dirty = false;
    }

    pub fn page_count(&self) -> usize { self.pages.len() }
    pub fn page_size(&self) -> u32 { self.page_size }

    pub fn total_free(&self) -> u64 { self.pages.iter().map(|p| p.skyline.free_area()).sum() }
    pub fn largest_free(&self) -> u64 { self.pages.iter().map(|p| p.skyline.largest_free_area()).max().unwrap_or(0) }

    pub fn fragmentation(&self) -> f32 {
        let total = self.total_free();
        if total == 0 { return 0.0; }
        let largest = self.largest_free();
        if largest == 0 { return 1.0; }
        1.0 - (largest as f32) / (total as f32)
    }
}

// ─── String 特化方法（向后兼容） ────────────────────────────────

impl DynamicAtlas<String> {
    pub fn load_toml(
        &mut self, toml_str: &str,
        mut rgba_provider: impl FnMut(&str) -> Option<(Vec<u8>, u32, u32)>,
    ) -> Result<usize, AtlasLoadError> {
        let data: SpriteAtlasToml = toml::from_str(toml_str).map_err(AtlasLoadError::Toml)?;
        let mut count = 0;
        for (name, entry) in &data.entries {
            let (full_rgba, tex_w, _tex_h) = rgba_provider(&entry.tex)
                .ok_or_else(|| AtlasLoadError::TexNotFound(entry.tex.clone()))?;
            let sub_rgba = crop_rgba(&full_rgba, tex_w as usize, entry.lt[0] as usize, entry.lt[1] as usize,
                entry.wh[0] as usize, entry.wh[1] as usize);
            match self.insert_ex(name, &sub_rgba, entry.wh[0], entry.wh[1]) {
                Some(_) => count += 1,
                None => return Err(AtlasLoadError::AtlasFull),
            }
        }
        Ok(count)
    }

    #[cfg(feature = "serde")]
    pub fn export_toml(&self) -> Result<String, toml::ser::Error> {
        let mut data = SpriteAtlasToml { entries: HashMap::new() };
        for (name, e) in &self.entries {
            data.entries.insert(name.clone(), SpriteEntryToml {
                tex: String::new(), lt: [e.region.tl_px.0, e.region.tl_px.1],
                wh: [e.region.wh_px.0, e.region.wh_px.1], or: [e.region.origin_px.0, e.region.origin_px.1],
            });
        }
        toml::to_string(&data)
    }

    pub fn insert_white(&mut self) -> AtlasRegion {
        self.insert("white".to_string(), &[255,255,255,255], 1, 1, (0,0), true)
            .expect("white pixel should always fit in atlas")
    }

    pub fn insert_ex(&mut self, name: &str, rgba: &[u8], w: u32, h: u32) -> Option<AtlasRegion> {
        self.insert(name.to_string(), rgba, w, h, (0, 0), true)
    }

    pub fn insert_ex_permanent(&mut self, name: &str, rgba: &[u8], w: u32, h: u32) -> Option<AtlasRegion> {
        self.insert_permanent(name.to_string(), rgba, w, h, (0, 0), true)
    }

    pub fn insert_ex_origin(&mut self, name: &str, rgba: &[u8], w: u32, h: u32, origin_px: (u32, u32)) -> Option<AtlasRegion> {
        self.insert(name.to_string(), rgba, w, h, origin_px, true)
    }

    pub fn insert_no_clamp(&mut self, name: &str, rgba: &[u8], w: u32, h: u32) -> Option<AtlasRegion> {
        self.insert(name.to_string(), rgba, w, h, (0, 0), false)
    }
}

// ─── TOML 辅助 ────────────────────────────────────────────────

fn crop_rgba(full: &[u8], tex_w: usize, x: usize, y: usize, w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    for row in 0..h {
        let src_start = ((y + row) * tex_w + x) * 4;
        let dst_start = row * w * 4;
        out[dst_start..dst_start + w * 4].copy_from_slice(&full[src_start..src_start + w * 4]);
    }
    out
}

#[derive(Debug)]
pub enum AtlasLoadError {
    Toml(toml::de::Error),
    TexNotFound(String),
    AtlasFull,
}

impl std::fmt::Display for AtlasLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Toml(e) => write!(f, "TOML parse error: {e}"), Self::TexNotFound(s) => write!(f, "source texture '{s}' not found in provider"), Self::AtlasFull => write!(f, "atlas is full") }
    }
}
impl std::error::Error for AtlasLoadError {}

#[derive(Deserialize)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub(crate) struct SpriteEntryToml { pub(crate) tex: String, pub(crate) lt: [u32; 2], pub(crate) wh: [u32; 2], pub(crate) or: [u32; 2] }

#[derive(Deserialize)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub(crate) struct SpriteAtlasToml { #[serde(flatten)] pub(crate) entries: HashMap<String, SpriteEntryToml> }

#[derive(Deserialize)]
pub struct TOMLEntry { pub tex: String, pub lt: [u32; 2], pub wh: [u32; 2], pub or: [u32; 2] }

pub fn parse_toml_entries(toml_str: &str) -> Result<HashMap<String, TOMLEntry>, AtlasLoadError> {
    let data: SpriteAtlasToml = toml::from_str(toml_str).map_err(AtlasLoadError::Toml)?;
    Ok(data.entries.into_iter().map(|(k, v)| (k, TOMLEntry { tex: v.tex, lt: v.lt, wh: v.wh, or: v.or })).collect())
}

// ─── StaticAtlas ──────────────────────────────────────────────

pub struct StaticAtlas { regions: HashMap<String, AtlasRegion> }

impl StaticAtlas {
    pub fn from_toml(toml_str: &str) -> Result<Self, StaticAtlasError> {
        let data: SpriteAtlasToml = toml::from_str(toml_str)?;
        let mut regions = HashMap::new();
        for (name, entry) in &data.entries {
            let uid = TEXTURES.uid_by_name(&entry.tex).ok_or_else(|| StaticAtlasError::TexNotFound(entry.tex.clone()))?;
            regions.insert(name.clone(), AtlasRegion { tl_px: (entry.lt[0], entry.lt[1]), wh_px: (entry.wh[0], entry.wh[1]), origin_px: (entry.or[0], entry.or[1]), page_uid: uid });
        }
        Ok(Self { regions })
    }
    #[cfg(feature = "serde")]
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        let mut data = SpriteAtlasToml { entries: HashMap::new() };
        for (name, region) in &self.regions {
            data.entries.insert(name.clone(), SpriteEntryToml { tex: String::new(), lt: [region.tl_px.0, region.tl_px.1], wh: [region.wh_px.0, region.wh_px.1], or: [region.origin_px.0, region.origin_px.1] });
        }
        toml::to_string(&data)
    }
    pub fn get(&self, name: &str) -> Option<&AtlasRegion> { self.regions.get(name) }
}

#[derive(Debug)]
pub enum StaticAtlasError { Toml(toml::de::Error), TexNotFound(String) }

impl From<toml::de::Error> for StaticAtlasError { fn from(e: toml::de::Error) -> Self { Self::Toml(e) } }
impl std::fmt::Display for StaticAtlasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Toml(e) => write!(f, "TOML: {e}"), Self::TexNotFound(s) => write!(f, "tex '{s}' not found") }
    }
}
impl std::error::Error for StaticAtlasError {}

// ─── clamp margin ──────────────────────────────────────────────

fn expand_clamp_margin(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let nw = w + 2; let nh = h + 2;
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..h {
        let src = (y * w * 4) as usize;
        let dst = ((y + 1) * nw * 4 + 4) as usize;
        out[dst..dst + (w * 4) as usize].copy_from_slice(&rgba[src..src + (w * 4) as usize]);
    }
    for x in 0..w { let s = (x * 4) as usize; let d = (x as usize + 1) * 4; out[d..d + 4].copy_from_slice(&rgba[s..s + 4]); }
    for x in 0..w { let s = (((h - 1) * w + x) * 4) as usize; let d = (((nh - 1) * nw + x + 1) * 4) as usize; out[d..d + 4].copy_from_slice(&rgba[s..s + 4]); }
    for y in 0..h { let s = (y * w * 4) as usize; let d = ((y + 1) * nw * 4) as usize; out[d..d + 4].copy_from_slice(&rgba[s..s + 4]); }
    for y in 0..h { let s = ((y * w + w - 1) * 4) as usize; let d = (((y + 1) * nw + nw - 1) * 4) as usize; out[d..d + 4].copy_from_slice(&rgba[s..s + 4]); }
    { let s = 0usize; let d = 0usize; out[d..d + 4].copy_from_slice(&rgba[s..s + 4]); }
    { let s = ((w - 1) * 4) as usize; let d = (nw - 1) * 4; let d = d as usize; out[d..d + 4].copy_from_slice(&rgba[s..s + 4]); }
    { let s = (((h - 1) * w) * 4) as usize; let d = ((nh - 1) * nw) * 4; let d = d as usize; out[d..d + 4].copy_from_slice(&rgba[s..s + 4]); }
    { let s = (((h - 1) * w + w - 1) * 4) as usize; let d = ((nh - 1) * nw + nw - 1) * 4; let d = d as usize; out[d..d + 4].copy_from_slice(&rgba[s..s + 4]); }
    out
}