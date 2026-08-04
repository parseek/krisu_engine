//! 运行时动态 / 静态纹理图集。
//!
//! - `DynamicAtlas<N>`：Skyline 打包器，运行时插入/踢出/compact/自动新建页。
//! - `StaticAtlas`：从 TOML 反序列化预排布图集（`spr.toml`）。
//! - `AtlasRegion`：图集内精灵坐标（像素左上角 + 尺寸 + 原点偏移 + 页 uid）。
//!
//! 依赖全局纹理注册表 `rjw_render::TEXTURES`（DashMap），完全解耦 `rjw_2d_render`。

use std::{collections::HashMap, sync::Arc};

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
        let mut best_idx = 0;
        let mut best_y = u32::MAX;
        for (i, seg) in self.segments.iter().enumerate() {
            if seg.w >= needed && seg.y < best_y { best_y = seg.y; best_idx = i; }
        }
        if best_y == u32::MAX || best_y + h + padding * 2 > self.page_size { return None; }

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
}

// ─── AtlasPage ────────────────────────────────────────────────

struct AtlasPage {
    texture: ArcTextureWrapped,
    skyline: Skyline,
}

impl AtlasPage {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, page_size: u32) -> Self {
        let size = (page_size * page_size * 4) as usize;
        let clear = vec![0u8; size];
        let tex = Arc::new(TextureWrapped::from_rgba8(device, queue, layout, "atlas_page", &clear, page_size, page_size));
        TEXTURES.register(tex.clone());
        Self { texture: tex, skyline: Skyline::new(page_size) }
    }
}

// ─── DynamicAtlas ─────────────────────────────────────────────

struct AtlasEntry {
    region: AtlasRegion,
    lifetime: u32,
}

pub struct DynamicAtlas<const PAGE_SIZE: u32 = DEFAULT_PAGE_SIZE> {
    pages: Vec<AtlasPage>,
    entries: HashMap<String, AtlasEntry>,
    config: AtlasConfig,
    dirty: bool,
}

impl<const N: u32> DynamicAtlas<N> {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, config: AtlasConfig) -> Self {
        let page = AtlasPage::new(device, queue, layout, N);
        Self { pages: vec![page], entries: HashMap::new(), config, dirty: false }
    }

    pub fn texture_uid_of(&self, name: &str) -> Option<u64> { self.entries.get(name).map(|e| e.region.page_uid) }

    pub fn get(&mut self, name: &str) -> Option<&AtlasRegion> {
        if let Some(e) = self.entries.get_mut(name) { e.lifetime = self.config.lifetime; Some(&e.region) }
        else { None }
    }

    pub fn insert(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout,
        name: &str, rgba: &[u8], w: u32, h: u32, origin_px: (u32, u32), clamp_margin: bool,
    ) -> Option<AtlasRegion> {
        if let Some(e) = self.entries.get_mut(name) { e.lifetime = self.config.lifetime; return Some(e.region); }
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
                    self.pages.push(AtlasPage::new(device, queue, layout, N));
                }
                _ => return None,
            }
        };
        let page = &self.pages[page_idx];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: page.texture.raw_texture(), mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
            &expanded_rgba, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(alloc_w * 4), rows_per_image: Some(alloc_h) },
            wgpu::Extent3d { width: alloc_w, height: alloc_h, depth_or_array_layers: 1 },
        );
        let region = AtlasRegion {
            tl_px: (x + margin_offs.0, y + margin_offs.1),
            wh_px: (w, h), origin_px, page_uid: page.texture.uid,
        };
        self.entries.insert(name.to_string(), AtlasEntry { region, lifetime: self.config.lifetime });
        Some(region)
    }

    fn try_alloc(&mut self, w: u32, h: u32, padding: u32) -> Option<(usize, u32, u32)> {
        for (i, page) in self.pages.iter_mut().enumerate() { if let Some((x, y)) = page.skyline.allocate(w, h, padding) { return Some((i, x, y)); } }
        if self.dirty { self.compact_inner(); }
        for (i, page) in self.pages.iter_mut().enumerate() { if let Some((x, y)) = page.skyline.allocate(w, h, padding) { return Some((i, x, y)); } }
        None
    }

    pub fn end_frame(&mut self) {
        let to_remove: Vec<_> = self.entries.iter().filter_map(|(k, e)| if e.lifetime == 0 { Some(k.clone()) } else { None }).collect();
        for k in to_remove { self.entries.remove(&k); }
        for e in self.entries.values_mut() { e.lifetime = e.lifetime.saturating_sub(1); }
        if !self.entries.is_empty() || self.dirty { self.dirty = true; }
    }

    pub fn compact(&mut self) { self.compact_inner(); }

    fn compact_inner(&mut self) {
        for page in &mut self.pages {
            let occupied: Vec<_> = self.entries.values().filter(|e| e.region.page_uid == page.texture.uid)
                .map(|e| (e.region.tl_px.0, e.region.tl_px.1, e.region.wh_px.0, e.region.wh_px.1)).collect();
            page.skyline = Skyline::from_occupied(N, &occupied, self.config.padding);
        }
        self.dirty = false;
    }

    pub fn page_count(&self) -> usize { self.pages.len() }

    pub fn page_size(&self) -> u32 { N }

    /// 插入 1×1 全白像素（与同页瓦片合批用）。
    pub fn insert_white(&mut self, device: &wgpu::Device, queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> AtlasRegion {
        self.insert(device, queue, layout, "white", &[255,255,255,255], 1, 1, (0,0), true)
            .expect("white pixel should always fit in atlas")
    }
}

/// 在 `w×h` RGBA 四周各扩展 1px（复制边界像素），返回 `(w+2)×(h+2)` 大小的数据。
fn expand_clamp_margin(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let nw = w + 2;
    let nh = h + 2;
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    // 中心区域（原始数据）
    for y in 0..h {
        let src = (y * w * 4) as usize;
        let dst = ((y + 1) * nw * 4 + 4) as usize;
        out[dst..dst + (w * 4) as usize].copy_from_slice(&rgba[src..src + (w * 4) as usize]);
    }
    // 上边（复制第 0 行）
    for x in 0..w {
        let s = (x * 4) as usize;
        let d = (x as usize + 1) * 4;
        out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
    }
    // 下边（复制第 h-1 行）
    for x in 0..w {
        let s = (((h - 1) * w + x) * 4) as usize;
        let d = (((nh - 1) * nw + x + 1) * 4) as usize;
        out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
    }
    // 左边（复制第 0 列）
    for y in 0..h {
        let s = (y * w * 4) as usize;
        let d = ((y + 1) * nw * 4) as usize;
        out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
    }
    // 右边（复制第 w-1 列）
    for y in 0..h {
        let s = ((y * w + w - 1) * 4) as usize;
        let d = (((y + 1) * nw + nw - 1) * 4) as usize;
        out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
    }
    // 左上角
    {
        let s = 0 as usize;
        let d = 0 as usize;
        out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
    }
    // 右上角
    {
        let s = ((w - 1) * 4) as usize;
        let d = (nw - 1) * 4; let d = d as usize;
        out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
    }
    // 左下角
    {
        let s = (((h - 1) * w) * 4) as usize;
        let d = ((nh - 1) * nw) * 4; let d = d as usize;
        out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
    }
    // 右下角
    {
        let s = (((h - 1) * w + w - 1) * 4) as usize;
        let d = ((nh - 1) * nw + nw - 1) * 4; let d = d as usize;
        out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
    }
    out
}

// ─── StaticAtlas ──────────────────────────────────────────────

#[cfg(feature = "serde")]
pub struct StaticAtlas { regions: HashMap<String, AtlasRegion> }

#[cfg(feature = "serde")]
impl StaticAtlas {
    pub fn from_toml(toml_str: &str) -> Result<Self, StaticAtlasError> {
        let data: SpriteAtlasToml = toml::from_str(toml_str)?;
        let mut regions = HashMap::new();
        for (name, entry) in &data.p {
            let uid = TEXTURES.uid_by_name(&entry.tex).ok_or_else(|| StaticAtlasError::TexNotFound(entry.tex.clone()))?;
            regions.insert(name.clone(), AtlasRegion { tl_px: (entry.lt[0], entry.lt[1]), wh_px: (entry.wh[0], entry.wh[1]), origin_px: (entry.or[0], entry.or[1]), page_uid: uid });
        }
        Ok(Self { regions })
    }
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        let mut data = SpriteAtlasToml { p: HashMap::new() };
        for (name, region) in &self.regions {
            data.p.insert(name.clone(), SpriteEntryToml { tex: String::new(), lt: [region.tl_px.0, region.tl_px.1], wh: [region.wh_px.0, region.wh_px.1], or: [region.origin_px.0, region.origin_px.1] });
        }
        toml::to_string(&data)
    }
    pub fn get(&self, name: &str) -> Option<&AtlasRegion> { self.regions.get(name) }
}

#[cfg(feature = "serde")]
#[derive(Debug)]
pub enum StaticAtlasError { Toml(toml::de::Error), TexNotFound(String) }

#[cfg(feature = "serde")]
impl From<toml::de::Error> for StaticAtlasError { fn from(e: toml::de::Error) -> Self { Self::Toml(e) } }

#[cfg(feature = "serde")]
impl std::fmt::Display for StaticAtlasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Toml(e) => write!(f, "TOML: {e}"), Self::TexNotFound(s) => write!(f, "tex '{s}' not found") }
    }
}
#[cfg(feature = "serde")]
impl std::error::Error for StaticAtlasError {}

#[cfg(feature = "serde")]
#[derive(Serialize, Deserialize)]
pub struct SpriteEntryToml { tex: String, lt: [u32; 2], wh: [u32; 2], or: [u32; 2] }

#[cfg(feature = "serde")]
#[derive(Serialize, Deserialize)]
pub struct SpriteAtlasToml {
    #[serde(flatten)]
    pub p: HashMap<String, SpriteEntryToml>,
}