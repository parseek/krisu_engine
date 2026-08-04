//! 运行时动态 / 静态纹理图集。
//!
//! - `DynamicAtlas<N>`：Skyline 打包器，运行时插入/踢出/compact/自动新建页 + TOML 批量导入 + 自动复活。
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

// ─── 纹理再生 / 源数据 ────────────────────────────────────────

/// 纹理再生器：精灵被图集踢出后，可通过此 trait 重新生成 RGBA 数据。
pub trait TextureRegenerator: Send + Sync {
    /// 返回 `(rgba_bytes, width, height)`。
    fn generate(&self) -> (Vec<u8>, u32, u32);
}

/// 精灵源数据，用于踢出后自动复活。
enum SourceData {
    /// 内联 RGBA 像素（最常用，无堆分配）
    Inline(Vec<u8>, u32, u32),
    /// 动态再生器（程序化纹理、延迟加载等）
    Dynamic(Box<dyn TextureRegenerator>),
}

impl SourceData {
    fn extract(&self) -> (Vec<u8>, u32, u32) {
        match self {
            Self::Inline(rgba, w, h) => (rgba.clone(), *w, *h),
            Self::Dynamic(regen) => regen.generate(),
        }
    }
}

/// 墓碑：被踢出精灵的残影，携带复活所需的全部信息。
struct Tombstone {
    source: SourceData,
    origin_px: (u32, u32),
    clamp_margin: bool,
}

// ─── DynamicAtlas ─────────────────────────────────────────────

struct AtlasEntry {
    region: AtlasRegion,
    lifetime: u32,
    /// `None` = 常驻精灵，不受 lifetime 踢出影响。
    source: Option<SourceData>,
}

pub struct DynamicAtlas<const PAGE_SIZE: u32 = DEFAULT_PAGE_SIZE> {
    pages: Vec<AtlasPage>,
    entries: HashMap<String, AtlasEntry>,
    tombstones: HashMap<String, Tombstone>,
    config: AtlasConfig,
    dirty: bool,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
}

impl<const N: u32> DynamicAtlas<N> {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, config: AtlasConfig) -> Self {
        let device = device.clone();
        let queue = queue.clone();
        let layout = layout.clone();
        let page = AtlasPage::new(&device, &queue, &layout, N);
        Self { pages: vec![page], entries: HashMap::new(), tombstones: HashMap::new(), config, dirty: false, device, queue, layout }
    }

    pub fn texture_uid_of(&self, name: &str) -> Option<u64> { self.entries.get(name).map(|e| e.region.page_uid) }

    // ── 获取 / 复活 ──

    /// 获取精灵区域并刷新寿命（不会触发复活）。
    pub fn get(&mut self, name: &str) -> Option<&AtlasRegion> {
        if let Some(e) = self.entries.get_mut(name) { e.lifetime = self.config.lifetime; Some(&e.region) }
        else { None }
    }

    /// 获取精灵区域；若已被踢出则使用保存的源数据自动复活（重新插入图集）。
    ///
    /// 常驻精灵（`source: None`）不会被踢出，永远可命中原 `get` 路径。
    pub fn get_or_revive(&mut self, name: &str) -> Option<&AtlasRegion> {
        // ① 先走现有 entries
        if self.entries.contains_key(name) {
            let e = self.entries.get_mut(name).unwrap();
            e.lifetime = self.config.lifetime;
            return Some(&e.region);
        }
        // ② 检查墓碑 → 复活
        let tomb = self.tombstones.remove(name)?;
        let (rgba, w, h) = tomb.source.extract();
        self.insert_inner(name, &rgba, w, h, tomb.origin_px, tomb.clamp_margin)?;
        // insert_inner 已写回 entries，再捞出引用
        Some(&self.entries[name].region)
    }

    // ── 插入 ──

    /// 插入/替换精灵（完整参数）。
    ///
    /// 自动保存 `SourceData::Inline`，可在被踢出后由 `get_or_revive()` 复活。
    pub fn insert(&mut self, name: &str, rgba: &[u8], w: u32, h: u32, origin_px: (u32, u32), clamp_margin: bool,
    ) -> Option<AtlasRegion> {
        self.insert_with_source(name, rgba, w, h, origin_px, clamp_margin, SourceData::Inline(rgba.to_vec(), w, h))
    }

    /// 插入动态再生精灵（不缓存 RGBA，每次复活时调用生成器）。
    pub fn insert_dyn(&mut self, name: &str, w: u32, h: u32, origin_px: (u32, u32), clamp_margin: bool,
        regen: Box<dyn TextureRegenerator>,
    ) -> Option<AtlasRegion> {
        let (rgba, _rw, _rh) = regen.generate();
        debug_assert_eq!(_rw, w);
        debug_assert_eq!(_rh, h);
        self.insert_with_source(name, &rgba, w, h, origin_px, clamp_margin, SourceData::Dynamic(regen))
    }

    /// 插入常驻精灵（不会过期踢出，无需复活数据）。
    pub fn insert_permanent(&mut self, name: &str, rgba: &[u8], w: u32, h: u32, origin_px: (u32, u32), clamp_margin: bool,
    ) -> Option<AtlasRegion> {
        if let Some(e) = self.entries.get_mut(name) { e.lifetime = self.config.lifetime; return Some(e.region); }
        self.tombstones.remove(name);
        let region = self.insert_inner(name, rgba, w, h, origin_px, clamp_margin)?;
        // source: None = 常驻，不会被踢出
        Some(region)
    }

    fn insert_with_source(&mut self, name: &str, rgba: &[u8], w: u32, h: u32, origin_px: (u32, u32), clamp_margin: bool,
        source: SourceData,
    ) -> Option<AtlasRegion> {
        if let Some(e) = self.entries.get_mut(name) { e.lifetime = self.config.lifetime; return Some(e.region); }
        // 移除旧墓碑（若存在）
        self.tombstones.remove(name);
        let region = self.insert_inner(name, rgba, w, h, origin_px, clamp_margin)?;
        // 更新 source（insert_inner 写入的 entry 无 source）
        if let Some(e) = self.entries.get_mut(name) { e.source = Some(source); }
        Some(region)
    }

    /// 仅执行 GPU 写入 + entries 插入，不处理 source/tombstones。
    fn insert_inner(&mut self, name: &str, rgba: &[u8], w: u32, h: u32, origin_px: (u32, u32), clamp_margin: bool,
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
                    self.pages.push(AtlasPage::new(&self.device, &self.queue, &self.layout, N));
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
        let region = AtlasRegion {
            tl_px: (x + margin_offs.0, y + margin_offs.1),
            wh_px: (w, h), origin_px, page_uid: page.texture.uid,
        };
        self.entries.insert(name.to_string(), AtlasEntry { region, lifetime: self.config.lifetime, source: None });
        Some(region)
    }

    fn try_alloc(&mut self, w: u32, h: u32, padding: u32) -> Option<(usize, u32, u32)> {
        for (i, page) in self.pages.iter_mut().enumerate() { if let Some((x, y)) = page.skyline.allocate(w, h, padding) { return Some((i, x, y)); } }
        if self.dirty { self.compact_inner(); }
        for (i, page) in self.pages.iter_mut().enumerate() { if let Some((x, y)) = page.skyline.allocate(w, h, padding) { return Some((i, x, y)); } }
        None
    }

    // ── 生命周期 / 踢出 ──

    pub fn end_frame(&mut self) {
        // 收集到期条目：有 source 的移入墓碑，无 source（常驻）的直接删除
        let mut to_tomb: Vec<(String, Tombstone)> = Vec::new();
        let mut remove_keys: Vec<String> = Vec::new();
        for (k, e) in &self.entries {
            if e.lifetime == 0 {
                if let Some(src) = &e.source {
                    to_tomb.push((k.clone(), Tombstone {
                        source: src.clone_inline(),
                        origin_px: e.region.origin_px,
                        clamp_margin: true,
                    }));
                }
                remove_keys.push(k.clone());
            }
        }
        for k in &remove_keys { self.entries.remove(k); }
        for (k, t) in to_tomb { self.tombstones.insert(k, t); }

        for e in self.entries.values_mut() { e.lifetime = e.lifetime.saturating_sub(1); }
        if !self.entries.is_empty() || self.dirty { self.dirty = true; }
    }

    // ── TOML 批量导入 ──

    /// 从 TOML 字符串批量导入精灵到图集。
    ///
    /// `rgba_provider` 用于按纹理名查找源纹理的完整 RGBA 数据 + 宽高（如 `("rjw2", (bytes, 512, 512))`）。
    /// 内部自动裁剪子区域并写入图集页。
    ///
    /// 返回成功导入的精灵数量。
    pub fn load_toml(
        &mut self,
        toml_str: &str,
        mut rgba_provider: impl FnMut(&str) -> Option<(Vec<u8>, u32, u32)>,
    ) -> Result<usize, AtlasLoadError> {
        let data: SpriteAtlasToml = toml::from_str(toml_str).map_err(AtlasLoadError::Toml)?;
        let mut count = 0;
        for (name, entry) in &data.entries {
            let (full_rgba, tex_w, _tex_h) = rgba_provider(&entry.tex)
                .ok_or_else(|| AtlasLoadError::TexNotFound(entry.tex.clone()))?;
            // 裁剪子区域
            let sub_rgba = crop_rgba(&full_rgba, tex_w as usize, entry.lt[0] as usize, entry.lt[1] as usize,
                entry.wh[0] as usize, entry.wh[1] as usize);
            match self.insert_ex(name, &sub_rgba, entry.wh[0], entry.wh[1]) {
                Some(_) => count += 1,
                None => return Err(AtlasLoadError::AtlasFull),
            }
        }
        Ok(count)
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

    // ── 便捷方法 ──

    /// 将当前所有活跃 entries 导出为 TOML 文本（`[p.name]` 格式，不含 tex 字段）。
    /// 用于编辑/排查/迁移图集到静态 atlas。
    #[cfg(feature = "serde")]
    pub fn export_toml(&self) -> Result<String, toml::ser::Error> {
        let mut data = SpriteAtlasToml { entries: HashMap::new() };
        for (name, e) in &self.entries {
            data.entries.insert(name.clone(), SpriteEntryToml {
                tex: String::new(),
                lt: [e.region.tl_px.0, e.region.tl_px.1],
                wh: [e.region.wh_px.0, e.region.wh_px.1],
                or: [e.region.origin_px.0, e.region.origin_px.1],
            });
        }
        toml::to_string(&data)
    }

    /// 插入 1×1 全白像素（与同页瓦片合批用）。
    pub fn insert_white(&mut self) -> AtlasRegion {
        self.insert("white", &[255,255,255,255], 1, 1, (0,0), true)
            .expect("white pixel should always fit in atlas")
    }

    /// 插入精灵（默认 origin=(0,0)，clamp_margin=true）。最常用的便捷方法。
    ///
    /// 自动保存源数据，可在被踢出后由 `get_or_revive()` 复活。
    pub fn insert_ex(&mut self, name: &str, rgba: &[u8], w: u32, h: u32) -> Option<AtlasRegion> {
        self.insert(name, rgba, w, h, (0, 0), true)
    }

    /// 插入常驻精灵（不会过期踢出）。
    pub fn insert_ex_permanent(&mut self, name: &str, rgba: &[u8], w: u32, h: u32) -> Option<AtlasRegion> {
        self.insert_permanent(name, rgba, w, h, (0, 0), true)
    }

    /// 插入精灵（指定原点，clamp_margin=true）。
    pub fn insert_ex_origin(&mut self, name: &str, rgba: &[u8], w: u32, h: u32, origin_px: (u32, u32)) -> Option<AtlasRegion> {
        self.insert(name, rgba, w, h, origin_px, true)
    }

    /// 插入精灵（默认 origin=(0,0)，clamp_margin=false）。用于不需要边界扩展的场景。
    pub fn insert_no_clamp(&mut self, name: &str, rgba: &[u8], w: u32, h: u32) -> Option<AtlasRegion> {
        self.insert(name, rgba, w, h, (0, 0), false)
    }
}

// ─── SourceData clone helpers ──────────────────────────────────

impl SourceData {
    fn clone_inline(&self) -> Self {
        match self {
            Self::Inline(rgba, w, h) => Self::Inline(rgba.clone(), *w, *h),
            Self::Dynamic(_) => {
                // Dynamic can't be cloned - store a placeholder, will fail on revive
                // This shouldn't happen in practice (insert_dyn always re-generates)
                Self::Inline(vec![], 0, 0)
            }
        }
    }
}

// ─── TOML 辅助 ────────────────────────────────────────────────

/// 裁剪 RGBA 子区域。
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
        match self {
            Self::Toml(e) => write!(f, "TOML parse error: {e}"),
            Self::TexNotFound(s) => write!(f, "source texture '{s}' not found in provider"),
            Self::AtlasFull => write!(f, "atlas is full"),
        }
    }
}

impl std::error::Error for AtlasLoadError {}

#[derive(Deserialize)]
#[cfg_attr(feature = "serde", derive(Serialize))]
struct SpriteEntryToml {
    tex: String,
    lt: [u32; 2],
    wh: [u32; 2],
    or: [u32; 2],
}

#[derive(Deserialize)]
#[cfg_attr(feature = "serde", derive(Serialize))]
struct SpriteAtlasToml {
    #[serde(flatten)]
    entries: HashMap<String, SpriteEntryToml>,
}

// ─── TOML 常用辅助方法 ────────────────────────────────────────

#[derive(Deserialize)]
pub struct TOMLEntry {
    pub tex: String,
    pub lt: [u32; 2],
    pub wh: [u32; 2],
    pub or: [u32; 2],
}

/// 解析 TOML 返回原始条目表（不依赖图集实例）。
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