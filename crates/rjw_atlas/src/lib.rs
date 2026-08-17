//! 运行时动态 / 静态纹理图集。
//!
//! - `DynamicAtlas<K=String>`：Guillotine 空闲矩形打包器，运行时插入/踢出/compact 去碎片重排/自动新建页
//!   + TOML 批量导入 + 自动复活；`compact()` 重排后 `generation()` 递增，缓存区域者据此刷新。
//!   `K` 泛型键（默认 `String`），`String` 特化支持 TOML 导入/导出 + 便捷方法。
//! - `StaticAtlas<K=String>`：从 TOML 反序列化预排布图集（`spr.toml`），泛型与 `DynamicAtlas` 一致。
//! - `DynamicAtlas` / `StaticAtlas` 均实现 `Index` / `IndexMut`：`atlas[&key]` 直接读写区域。
//! - `AtlasRegion`：图集内精灵坐标（像素左上角 + 尺寸 + 原点偏移 + 页 uid）。
//!
//! 依赖全局纹理注册表 `rjw_render::TEXTURES`（DashMap），完全解耦 `rjw_2d_render`。

use std::{
    borrow::Borrow,
    collections::HashMap,
    hash::Hash,
    ops::{Index, IndexMut},
    sync::Arc,
};

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

// ─── AtlasRegion / RegionRef ──────────────────────────────────

#[derive(Clone, Copy, Debug)]
pub struct AtlasRegion {
    pub tl_px: (u32, u32),
    pub wh_px: (u32, u32),
    pub origin_px: (u32, u32),
    pub page_uid: u64,
}

/// 图集条目的稳定句柄（**独立于** [`AtlasRegion`] 的值拷贝）。
///
/// - `region_id`：条目唯一且**跨去碎片重排稳定**（重排只改 `tl_px`，id 不变）；
/// - **RAII 引用计数**：`RegionRef` 持有条目 keepalive 的 `Arc` 克隆，drop 自动释放；
///   被引用的条目不参与 LRU 逐出（`end_frame` 时保活）——外部持有句柄期间条目保证可用；
/// - 重排后经 [`RegionRef::resolve`] 取最新 UV（无需外部自行同步 generation）。
#[derive(Clone, Debug)]
pub struct RegionRef {
    region_id: u64,
    page_uid: u64,
    /// RAII 保活：仅持有（drop 自动释放），不直接读取。
    #[allow(dead_code)]
    keepalive: Arc<()>,
}

impl RegionRef {
    #[inline]
    pub fn region_id(&self) -> u64 {
        self.region_id
    }
    #[inline]
    pub fn page_uid(&self) -> u64 {
        self.page_uid
    }
    /// 离线/测试句柄：不保活任何条目，`resolve` 仅按 id 查 atlas。
    #[inline]
    pub fn from_parts(region_id: u64, page_uid: u64) -> Self {
        Self { region_id, page_uid, keepalive: Arc::new(()) }
    }
    /// 在 `atlas` 中解析该条目**当前**（含重排后）的区域。
    ///
    /// 被引用的条目保证存在（保活不逐出），因此通常恒为 `Some`。
    #[inline]
    pub fn resolve<K: Hash + Eq + Clone>(&self, atlas: &DynamicAtlas<K>) -> Option<AtlasRegion> {
        atlas.resolve_by_id(self.region_id)
    }
}

// ─── 空闲矩形打包器（Guillotine）──────────────────────────────

/// 空闲矩形：页内一块未被占用的区域 `[x, x+w) × [y, y+h)`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FreeRect { x: u32, y: u32, w: u32, h: u32 }

/// 空闲矩形列表打包器（替换旧“天空线”：天空线无法表达“行下方整宽自由区”，
/// 混合高度时会把页碎片化成窄列，导致“页未满却开新页”）。
///
/// 算法：best-fit（最低 y → 最小面积）选择一个能容纳的空闲矩形，放置后切分：
/// - **整宽矩形**（`w == page_size`，即某行/新页的行矩形）→ **水平切分**：下方保留整宽行，
///   保证下一行始终可放宽字形（即使行内字形高度交错）；
/// - 其余矩形 → 沿剩余较长方向（`rh >= rw` 水平 / `rw > rh` 竖直）切分，保持随机负载密度。
/// 并合并相邻空闲矩形。
#[derive(Clone)]
struct Guillotine {
    segments: Vec<FreeRect>,
    page_size: u32,
}

impl Guillotine {
    fn new(page_size: u32) -> Self {
        Self { segments: vec![FreeRect { x: 0, y: 0, w: page_size, h: page_size }], page_size }
    }

    fn allocate(&mut self, w: u32, h: u32, padding: u32) -> Option<(u32, u32)> {
        let nw = w + padding * 2;
        let nh = h + padding * 2;
        // best-fit：最低 y（优先填当前行），其次最小面积（减少浪费）。
        let mut best: Option<(usize, u32, u32)> = None;
        for (i, r) in self.segments.iter().enumerate() {
            if r.w >= nw && r.h >= nh {
                let area = r.w * r.h;
                let better = match best {
                    None => true,
                    Some((_, by, ba)) => r.y < by || (r.y == by && area < ba),
                };
                if better { best = Some((i, area, r.w)); }
            }
        }
        let (idx, _, _) = best?;
        let rect = self.segments.remove(idx);
        let x = rect.x + padding;
        let y = rect.y + padding;
        let rw = rect.w - nw; // 右侧剩余宽
        let rh = rect.h - nh; // 下方剩余高
        // 整宽矩形 → 水平切分（下方保留整宽行，下一行始终可放宽字形）。
        let full_width_row = rect.w >= self.page_size;
        if rh == 0 {
            // 字形恰好填满该矩形高度 → 右侧整高列。
            if rw > 0 { self.segments.push(FreeRect { x: rect.x + nw, y: rect.y, w: rw, h: rect.h }); }
        } else if full_width_row || rh >= rw {
            // 水平切分：下方整宽行 + 右侧 `nh` 高条带。
            if rh > 0 { self.segments.push(FreeRect { x: rect.x, y: rect.y + nh, w: rect.w, h: rh }); }
            if rw > 0 { self.segments.push(FreeRect { x: rect.x + nw, y: rect.y, w: rw, h: nh }); }
        } else {
            // 竖直切分：右侧整高列 + 下方 `nw` 宽条带。
            if rw > 0 { self.segments.push(FreeRect { x: rect.x + nw, y: rect.y, w: rw, h: rect.h }); }
            if rh > 0 { self.segments.push(FreeRect { x: rect.x, y: rect.y + nh, w: nw, h: rh }); }
        }
        self.merge();
        Some((x, y))
    }

    /// 合并相邻空闲矩形：同 y 同高 x 相邻 → 横向合并；同 x 同宽 y 相邻 → 纵向合并。
    fn merge(&mut self) {
        if self.segments.len() < 2 { return; }
        self.segments.sort_by_key(|r| (r.y, r.x));
        let mut i = 0;
        while i + 1 < self.segments.len() {
            let a = self.segments[i]; let b = self.segments[i + 1];
            if a.y == b.y && a.h == b.h && a.x + a.w == b.x {
                self.segments[i].w += b.w;
                self.segments.remove(i + 1);
            } else { i += 1; }
        }
        self.segments.sort_by_key(|r| (r.x, r.y));
        let mut i = 0;
        while i + 1 < self.segments.len() {
            let a = self.segments[i]; let b = self.segments[i + 1];
            if a.x == b.x && a.w == b.w && a.y + a.h == b.y {
                self.segments[i].h += b.h;
                self.segments.remove(i + 1);
            } else { i += 1; }
        }
    }

    /// 从已占用的矩形重建空闲矩形列表：x 扫描线，每个跨度取覆盖它的占用矩形的 y 区间之并，
    /// 求 `[0, page_size)` 的补集得到自由区间，最后合并相邻跨度。
    ///
    /// 与增量 [`Self::allocate`] 维护的空闲矩形**等价**（自由区完全一致），供 `compact` 重建使用。
    fn from_occupied(page_size: u32, occupied: &[(u32, u32, u32, u32)], padding: u32) -> Self {
        let mut xs: Vec<u32> = vec![0, page_size];
        let mut rects: Vec<(u32, u32, u32, u32)> = Vec::new(); // (x0, y0, x1, y1) 含 padding 扩展
        for &(ox, oy, ow, oh) in occupied {
            let x0 = ox.saturating_sub(padding);
            let y0 = oy.saturating_sub(padding);
            let x1 = x0.saturating_add(ow + padding * 2).min(page_size);
            let y1 = y0.saturating_add(oh + padding * 2).min(page_size);
            if x1 > x0 && y1 > y0 {
                rects.push((x0, y0, x1, y1));
                xs.push(x0);
                xs.push(x1);
            }
        }
        xs.sort_unstable();
        xs.dedup();

        let mut segments: Vec<FreeRect> = Vec::new();
        for span in xs.windows(2) {
            let (xa, xb) = (span[0], span[1]);
            if xa >= xb { continue; }
            // 覆盖整个跨度 [xa, xb) 的占用矩形（事件点来自矩形边界，跨度为最大子区间）。
            let mut ivs: Vec<(u32, u32)> = rects.iter()
                .filter(|r| r.0 <= xa && r.2 >= xb)
                .map(|r| (r.1, r.3))
                .collect();
            ivs.sort_unstable();
            // 占用区间之并 → [0, page_size) 的补集 = 自由区间。
            let mut cur = 0u32;
            for &(y0, y1) in &ivs {
                if y0 > cur { segments.push(FreeRect { x: xa, y: cur, w: xb - xa, h: y0.min(page_size) - cur }); }
                cur = cur.max(y1);
                if cur >= page_size { break; }
            }
            if cur < page_size { segments.push(FreeRect { x: xa, y: cur, w: xb - xa, h: page_size - cur }); }
        }
        let mut slf = Self { segments, page_size };
        slf.merge();
        slf
    }

    fn free_area(&self) -> u64 {
        self.segments.iter().map(|r| (r.w as u64) * (r.h as u64)).sum()
    }

    fn largest_free_area(&self) -> u64 {
        self.segments.iter().map(|r| (r.w as u64) * (r.h as u64)).max().unwrap_or(0)
    }
}

// ─── AtlasPage ────────────────────────────────────────────────

struct AtlasPage {
    texture: ArcTextureWrapped,
    allocator: Guillotine,
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
        Self { texture: tex, allocator: Guillotine::new(page_size) }
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
    clamp_margin: bool,
    /// 稳定条目 id（跨重排不变）。
    region_id: u64,
    /// keepalive：条目自身持 1 个强引用；外部 `RegionRef` 持有克隆 → `strong_count() > 1` 表示被引用。
    keepalive: Arc<()>,
}

pub struct DynamicAtlas<K = String> {
    pages: Vec<AtlasPage>,
    entries: HashMap<K, AtlasEntry>,
    tombstones: HashMap<K, Tombstone>,
    config: AtlasConfig,
    page_size: u32,
    dirty: bool,
    /// 去碎片重排世代号：每次 `compact` 真正搬动条目时 +1，持有缓存区域者据此刷新。
    generation: u64,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    /// region_id 分配器。
    next_region_id: u64,
    /// region_id → 键（`resolve_by_id` / `acquire_by_id` 用）。
    by_id: HashMap<u64, K>,
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
        Self { pages: vec![page], entries: HashMap::new(), tombstones: HashMap::new(), config, page_size, dirty: false, generation: 0, device, queue, layout, next_region_id: 1, by_id: HashMap::new() }
    }

    /// 按稳定 `region_id` 解析当前区域（重排后仍可用；条目被逐出则返回 `None`）。
    pub fn resolve_by_id(&self, region_id: u64) -> Option<AtlasRegion> {
        let key = self.by_id.get(&region_id)?;
        self.entries.get(key).map(|e| e.region)
    }

    /// 获取条目的稳定句柄（RAII 引用计数：drop 自动释放；被引用条目不逐出）。
    pub fn acquire(&mut self, key: &K) -> Option<RegionRef> {
        let e = self.entries.get_mut(key)?;
        e.lifetime = self.config.lifetime;
        Some(RegionRef {
            region_id: e.region_id,
            page_uid: e.region.page_uid,
            keepalive: e.keepalive.clone(),
        })
    }

    /// 去碎片重排世代号（每次搬动条目 +1；未搬动则不变）。
    pub fn generation(&self) -> u64 { self.generation }

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
        let region_id = self.next_region_id;
        self.next_region_id += 1;
        let keepalive = Arc::new(());
        self.entries.insert(key.clone(), AtlasEntry {
            region, lifetime: self.config.lifetime, source: None, alloc_tl, alloc_wh, clamp_margin,
            region_id, keepalive,
        });
        self.by_id.insert(region_id, key);
        Some(region)
    }

    fn try_alloc(&mut self, w: u32, h: u32, padding: u32) -> Option<(usize, u32, u32)> {
        for (i, page) in self.pages.iter_mut().enumerate() { if let Some((x, y)) = page.allocator.allocate(w, h, padding) { return Some((i, x, y)); } }
        // 第一遍失败：无条件整理（重建空闲矩形、合并碎片），避免“页未满却开新页”。
        self.compact_inner();
        for (i, page) in self.pages.iter_mut().enumerate() { if let Some((x, y)) = page.allocator.allocate(w, h, padding) { return Some((i, x, y)); } }
        None
    }

    pub fn end_frame(&mut self) {
        let mut to_tomb: Vec<(K, Tombstone)> = Vec::new();
        let mut remove_keys: Vec<K> = Vec::new();
        for (k, e) in &self.entries {
            // 被外部 RegionRef 引用的条目保活（keepalive strong_count > 1），不逐出。
            if e.lifetime == 0 && Arc::strong_count(&e.keepalive) == 1 {
                if let Some(src) = &e.source {
                    to_tomb.push((k.clone(), Tombstone { source: src.clone_inline(), origin_px: e.region.origin_px, clamp_margin: e.clamp_margin }));
                    remove_keys.push(k.clone());
                }
            }
        }
        for k in &remove_keys {
            if let Some(e) = self.entries.remove(k) {
                self.by_id.remove(&e.region_id);
            }
        }
        for (k, t) in to_tomb { self.tombstones.insert(k, t); }
        for e in self.entries.values_mut() { if e.source.is_some() { e.lifetime = e.lifetime.saturating_sub(1); } }
        if !self.entries.is_empty() || self.dirty { self.dirty = true; }
    }

    pub fn compact(&mut self) { self.compact_inner(); }

    /// 去碎片整理：优先尝试**全量重排**（所有带源条目按面积降序重排到最少页，真正消除碎片）；
    /// 若存在无法搬动的无源条目（永久精灵）则退回按页重建空闲矩形（配合 [`Guillotine::from_occupied`]）。
    fn compact_inner(&mut self) {
        if self.repack_all() {
            self.dirty = false;
            return;
        }
        let ps = self.page_size;
        for page in &mut self.pages {
            let occupied: Vec<_> = self.entries.values().filter(|e| e.region.page_uid == page.texture.uid)
                .map(|e| (e.alloc_tl.0, e.alloc_tl.1, e.alloc_wh.0, e.alloc_wh.1)).collect();
            page.allocator = Guillotine::from_occupied(ps, &occupied, 0);
        }
        self.dirty = false;
    }

    /// 把全部带源条目按面积降序重排进最少页（复用现有页纹理），重传纹理并更新条目区域。
    ///
    /// - 任何条目无源（永久精灵，无法重新上传）→ 放弃重排，返回 `false`。
    /// - 重排成功 → `generation` 递增（外部持有区域缓存者据此刷新），返回 `true`。
    /// - 重排后仍超出 `max_pages` → 放弃，返回 `false`。
    fn repack_all(&mut self) -> bool {
        if self.entries.is_empty() { return true; }
        if self.entries.values().any(|e| e.source.is_none()) { return false; }

        struct Item<K> {
            key: K,
            rgba: Vec<u8>,
            alloc_w: u32,
            alloc_h: u32,
            margin_offs: (u32, u32),
            wh_px: (u32, u32),
            origin_px: (u32, u32),
            lifetime: u32,
        }
        let mut items: Vec<Item<K>> = Vec::with_capacity(self.entries.len());
        for (key, e) in &self.entries {
            let (rgba, w, h) = e.source.as_ref().expect("source checked above").extract();
            let (expanded, alloc_w, alloc_h, margin_offs) = if e.clamp_margin {
                (expand_clamp_margin(&rgba, w, h), w + 2, h + 2, (1u32, 1u32))
            } else {
                (rgba, w, h, (0u32, 0u32))
            };
            items.push(Item { key: key.clone(), rgba: expanded, alloc_w, alloc_h, margin_offs, wh_px: (w, h), origin_px: e.region.origin_px, lifetime: e.lifetime });
        }
        // 高优先 → 同高字形聚成整行，契合行式（整宽行）分配器，密度更高。
        items.sort_by(|a, b| b.alloc_h.cmp(&a.alloc_h).then((b.alloc_h * b.alloc_w).cmp(&(a.alloc_h * a.alloc_w))));

        let padding = self.config.padding;
        let mut allocators: Vec<Guillotine> = vec![Guillotine::new(self.page_size)];
        let mut slots: Vec<(usize, u32, u32)> = Vec::with_capacity(items.len());
        'outer: for it in &items {
            for (si, sky) in allocators.iter_mut().enumerate() {
                if let Some((x, y)) = sky.allocate(it.alloc_w, it.alloc_h, padding) {
                    slots.push((si, x, y));
                    continue 'outer;
                }
            }
            if allocators.len() >= self.config.max_pages { return false; }
            let mut sky = Guillotine::new(self.page_size);
            let (x, y) = sky.allocate(it.alloc_w, it.alloc_h, padding).expect("fresh page always fits");
            allocators.push(sky);
            slots.push((allocators.len() - 1, x, y));
        }

        while self.pages.len() < allocators.len() {
            self.pages.push(AtlasPage::new(&self.device, &self.queue, &self.layout, self.page_size));
        }
        for (it, (pi, x, y)) in items.into_iter().zip(slots.into_iter()) {
            let page = &self.pages[pi];
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: page.texture.raw_texture(), mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
                &it.rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(it.alloc_w * 4), rows_per_image: Some(it.alloc_h) },
                wgpu::Extent3d { width: it.alloc_w, height: it.alloc_h, depth_or_array_layers: 1 },
            );
            let e = self.entries.get_mut(&it.key).expect("entry must exist");
            e.region = AtlasRegion {
                tl_px: (x + it.margin_offs.0, y + it.margin_offs.1),
                wh_px: it.wh_px,
                origin_px: it.origin_px,
                page_uid: page.texture.uid,
            };
            e.alloc_tl = (x - padding, y - padding);
            e.alloc_wh = (it.alloc_w + padding * 2, it.alloc_h + padding * 2);
            e.lifetime = it.lifetime;
        }
        self.pages.truncate(allocators.len());
        for (page, sky) in self.pages.iter_mut().zip(allocators.into_iter()) {
            page.allocator = sky;
        }
        self.generation += 1;
        true
    }

    pub fn page_count(&self) -> usize { self.pages.len() }
    pub fn page_size(&self) -> u32 { self.page_size }

    pub fn total_free(&self) -> u64 { self.pages.iter().map(|p| p.allocator.free_area()).sum() }
    pub fn largest_free(&self) -> u64 { self.pages.iter().map(|p| p.allocator.largest_free_area()).max().unwrap_or(0) }

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

// ─── Index / IndexMut ─────────────────────────────────────────

impl<K, Q> Index<&Q> for DynamicAtlas<K>
where
    K: Hash + Eq + Borrow<Q>,
    Q: Hash + Eq + ?Sized,
{
    type Output = AtlasRegion;
    fn index(&self, key: &Q) -> &AtlasRegion {
        &self.entries.get(key).expect("DynamicAtlas: region not found for key").region
    }
}

impl<K, Q> IndexMut<&Q> for DynamicAtlas<K>
where
    K: Hash + Eq + Borrow<Q>,
    Q: Hash + Eq + ?Sized,
{
    fn index_mut(&mut self, key: &Q) -> &mut AtlasRegion {
        &mut self.entries.get_mut(key).expect("DynamicAtlas: region not found for key").region
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

/// 静态预排布图集（如 `spr.toml` 精灵表）：`K` 泛型键（默认 `String`），与 `DynamicAtlas` 泛型一致。
///
/// 实现 `Index<&Q>` / `IndexMut<&Q>`（`K: Borrow<Q>`）：`atlas["sprite"]` 直接读写区域。
#[derive(Debug, Clone)]
pub struct StaticAtlas<K = String> {
    regions: HashMap<K, AtlasRegion>,
}

impl<K> Default for StaticAtlas<K> {
    fn default() -> Self {
        Self { regions: HashMap::new() }
    }
}

impl<K: Hash + Eq> From<HashMap<K, AtlasRegion>> for StaticAtlas<K> {
    fn from(value: HashMap<K, AtlasRegion>) -> Self {
        Self { regions: value }
    }
}

impl<K: Hash + Eq> StaticAtlas<K> {
    /// 查找区域（接受可借用键：`&str` / `&String` / 其它 `K: Borrow<Q>`）。
    pub fn get<Q>(&self, key: &Q) -> Option<&AtlasRegion>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.regions.get(key)
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.regions.contains_key(key)
    }
}

// String 特化：TOML 导入/导出（serde feature），向后兼容。
impl StaticAtlas<String> {
    #[cfg(feature = "serde")]
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
}

impl<K, Q> Index<&Q> for StaticAtlas<K>
where
    K: Hash + Eq + Borrow<Q>,
    Q: Hash + Eq + ?Sized,
{
    type Output = AtlasRegion;
    fn index(&self, key: &Q) -> &AtlasRegion {
        self.regions.get(key).expect("StaticAtlas: region not found for key")
    }
}

impl<K, Q> IndexMut<&Q> for StaticAtlas<K>
where
    K: Hash + Eq + Borrow<Q>,
    Q: Hash + Eq + ?Sized,
{
    fn index_mut(&mut self, key: &Q) -> &mut AtlasRegion {
        self.regions.get_mut(key).expect("StaticAtlas: region not found for key")
    }
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

#[cfg(test)]
mod static_atlas_tests {
    use super::*;

    fn sample() -> StaticAtlas<String> {
        let mut m = HashMap::new();
        m.insert(
            "a".to_string(),
            AtlasRegion { tl_px: (1, 2), wh_px: (8, 8), origin_px: (0, 0), page_uid: 42 },
        );
        m.insert(
            "b".to_string(),
            AtlasRegion { tl_px: (10, 20), wh_px: (4, 4), origin_px: (0, 0), page_uid: 42 },
        );
        StaticAtlas::from(m)
    }

    #[test]
    fn index_and_index_mut() {
        let mut atlas = sample();
        // Index<&str>（String: Borrow<str>）
        assert_eq!(atlas["a"].tl_px, (1, 2));
        // Index<&String>
        let key = "b".to_string();
        assert_eq!(atlas[&key].wh_px, (4, 4));
        // IndexMut：直接改写区域
        atlas["b"].origin_px = (5, 6);
        assert_eq!(atlas[&key].origin_px, (5, 6));
    }

    #[test]
    fn get_accepts_borrowed_keys() {
        let atlas = sample();
        assert_eq!(atlas.get("a").unwrap().page_uid, 42);
        let key = "b".to_string();
        assert!(atlas.get(&key).is_some());
        assert!(atlas.get("missing").is_none());
        assert!(atlas.contains_key("a"));
        assert_eq!(atlas.len(), 2);
        assert!(!atlas.is_empty());
    }

    #[test]
    fn generic_key_type() {
        // K = u32（与 DynamicAtlas 一致的泛型能力）
        let mut m = HashMap::new();
        m.insert(
            7u32,
            AtlasRegion { tl_px: (0, 0), wh_px: (2, 2), origin_px: (0, 0), page_uid: 1 },
        );
        let atlas = StaticAtlas::<u32>::from(m);
        assert_eq!(atlas[&7].wh_px, (2, 2));
        assert!(atlas.get(&7).is_some());
        // 泛型 Default
        let empty: StaticAtlas<u32> = StaticAtlas::default();
        assert!(empty.is_empty());
    }
}
#[cfg(test)]
mod guillotine_tests {
    use super::*;

    /// 按顺序分配一组矩形（padding 可设），返回分配器与放置结果 `(x, y, w, h)`。
    fn allocate_all(page: u32, padding: u32, items: &[(u32, u32)]) -> (Guillotine, Vec<(u32, u32, u32, u32)>) {
        let mut sky = Guillotine::new(page);
        let mut placed = Vec::new();
        for &(w, h) in items {
            if let Some((x, y)) = sky.allocate(w, h, padding) {
                placed.push((x, y, w, h));
            }
        }
        (sky, placed)
    }

    /// 检查两个分配器的自由区总面积一致（重建应与增量分配等价）。
    fn assert_same_free_space(a: &Guillotine, b: &Guillotine) {
        assert_eq!(a.free_area(), b.free_area(), "重建后自由面积应与增量分配一致");
    }

    #[test]
    fn compact_roundtrip_preserves_free_space() {
        // 混合尺寸（含高低差形成阶梯）下，compact 重建的空闲矩形应与增量分配的自由区等价。
        let items: Vec<(u32, u32)> = vec![
            (10, 10), (10, 10), (10, 10), (20, 8), (8, 20),
            (30, 16), (16, 30), (4, 4), (4, 4), (4, 4),
            (64, 64), (10, 10), (8, 8), (22, 22), (5, 40),
        ];
        let (sky, placed) = allocate_all(1024, 0, &items);
        let rebuilt = Guillotine::from_occupied(1024, &placed, 0);
        assert_same_free_space(&sky, &rebuilt);
        // 重建后还能放下与增量分配相同的探测矩形。
        for &(w, h) in &[(12, 4), (40, 20), (5, 5)] {
            let mut s = sky.clone();
            let mut r = rebuilt.clone();
            assert_eq!(s.allocate(w, h, 0).is_some(), r.allocate(w, h, 0).is_some(),
                "探测矩形 ({w}, {h}) 的可用性应一致");
        }
    }

    #[test]
    fn padded_roundtrip_matches_compact_inner() {
        // 模拟文本字形：padding=1 放置，compact_inner 传 alloc 矩形（tl=(x-1,y-1), wh=(w+2,h+2)）。
        let page = 512;
        let items: [(u32, u32); 7] = [(10, 10), (12, 8), (30, 30), (8, 20), (16, 16), (4, 4), (20, 12)];
        let mut sky = Guillotine::new(page);
        let mut placed = Vec::new();
        for &(w, h) in &items {
            let (x, y) = sky.allocate(w, h, 1).expect("应能放入");
            placed.push((x - 1, y - 1, w + 2, h + 2));
        }
        let rebuilt = Guillotine::from_occupied(page, &placed, 0);
        assert_same_free_space(&sky, &rebuilt);
    }

    #[test]
    fn same_row_free_band_is_full_width() {
        // 同一行三个 4px 字形：增量分配后行下方应保留**整宽**空闲矩形（0,4,64,60）——
        // 旧Guillotine 空闲矩形会碎片化成窄列导致开新页；空闲矩形模型不会。
        let (sky, placed) = allocate_all(64, 0, &[(4, 4), (4, 4), (4, 4)]);
        assert!(sky.segments.iter().any(|s| s.x == 0 && s.y == 4 && s.w == 64 && s.h == 60),
            "增量分配后行下方应为整宽自由区: {:?}", sky.segments);
        // 重建是“极大化”表示：字形下方 [0,12)×[4,64) 一条带 + [12,64) 从顶部即自由。
        let rebuilt = Guillotine::from_occupied(64, &placed, 0);
        assert!(rebuilt.segments.iter().any(|s| s.x == 0 && s.y == 4 && s.w == 12 && s.h == 60),
            "重建后字形行下方 [0,12) 应从 y=4 自由: {:?}", rebuilt.segments);
        assert!(rebuilt.segments.iter().any(|s| s.x == 12 && s.y == 0 && s.w == 52 && s.h == 64),
            "重建后 [12,64) 应从 y=0 自由: {:?}", rebuilt.segments);
        // 12px 宽的字形必须能放进下一行。
        let mut r = rebuilt;
        assert!(r.allocate(12, 4, 0).is_some());
    }

    #[test]
    fn offset_and_stacked_bands_are_preserved() {
        // R1=[0,10)×[0,10)，R2=[5,10)×[10,15)：自由区必须保留 x∈[5,10) 从 y=15 起的带，
        // 且空闲矩形之间不允许重叠。
        let placed = vec![(0u32, 0u32, 10u32, 10u32), (5, 10, 5, 5)];
        let rebuilt = Guillotine::from_occupied(20, &placed, 0);
        assert!(rebuilt.segments.iter().any(|s| s.x == 0 && s.y == 10 && s.w == 5 && s.h == 10),
            "x∈[0,5) 从 y=10 起应自由: {:?}", rebuilt.segments);
        assert!(rebuilt.segments.iter().any(|s| s.x == 5 && s.y == 15 && s.w == 5 && s.h == 5),
            "x∈[5,10) 从 y=15 起应自由: {:?}", rebuilt.segments);
        assert!(rebuilt.segments.iter().any(|s| s.x == 10 && s.y == 0 && s.w == 10 && s.h == 20),
            "x∈[10,20) 整列应自由: {:?}", rebuilt.segments);
        for i in 0..rebuilt.segments.len() {
            for j in (i + 1)..rebuilt.segments.len() {
                let a = rebuilt.segments[i];
                let b = rebuilt.segments[j];
                let ox = a.x < b.x + b.w && b.x < a.x + a.w;
                let oy = a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(!(ox && oy), "空闲矩形重叠: {:?} vs {:?}", a, b);
            }
        }
    }

    #[test]
    fn padded_pack_and_rebuild_keep_usage_correct() {
        // 伪随机混合尺寸 + padding=1：放置不重叠、重建后 free_area + 占用面积 ≤ 页面积，
        // 且“大块优先重排”（即 repack 策略）也必须能容纳全部矩形。
        let page = 512;
        let mut sky = Guillotine::new(page);
        let mut placed = Vec::new();
        let mut seed = 0x9E37_79B9u32;
        let mut glyph_sizes: Vec<(u32, u32)> = Vec::new();
        for _ in 0..400 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let w = 4 + seed % 48;
            let h = 4 + (seed >> 8) % 48;
            if let Some((x, y)) = sky.allocate(w, h, 1) {
                glyph_sizes.push((w, h));
                // 与 `compact_inner` 一致：记录 alloc 矩形（tl=(x-1,y-1), wh=(w+2,h+2)）。
                placed.push((x - 1, y - 1, w + 2, h + 2));
            }
        }
        for i in 0..placed.len() {
            for j in (i + 1)..placed.len() {
                let a = placed[i]; let b = placed[j];
                let ox = a.0 < b.0 + b.2 && b.0 < a.0 + a.2;
                let oy = a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
                assert!(!(ox && oy), "alloc 矩形重叠: {:?} vs {:?}", a, b);
            }
        }
        let rebuilt = Guillotine::from_occupied(page, &placed, 0);
        assert_same_free_space(&sky, &rebuilt);
        // 占用面积按 alloc 矩形（含 padding）计，自由区与占用区互补不超页面积。
        let occupied_area: u64 = placed.iter().map(|&(_, _, w, h)| w as u64 * h as u64).sum();
        assert!(rebuilt.free_area() + occupied_area <= (page as u64) * (page as u64),
            "free_area + 占用面积不应超过页面积");
        // 反向：高优先 + 全新分配（与 `repack_all` 相同的策略）也必须全部容纳。
        let mut sorted = glyph_sizes;
        sorted.sort_by(|a, b| (b.1).cmp(&(a.1)).then((b.1 * b.0).cmp(&(a.1 * a.0))));
        let mut repacked = Guillotine::new(page);
        for &(w, h) in &sorted {
            assert!(repacked.allocate(w, h, 1).is_some(), "高优先重排应能放下 (w={w}, h={h})");
        }
    }

    #[test]
    fn next_row_keeps_full_width_band() {
        // 回归：行内字形**高度交错**（20/24/22/26）时，旧“沿剩余较长方向”切分会把下一行
        // 碎成互不合并的窄条，宽字形放不下 → “页未满却开新页”。
        // 新策略（水平切分优先，整宽行始终保留）下，宽字形应总能放入下一行。
        let page = 128u32;
        let mut sky = Guillotine::new(page);
        // 填满前几行：14 个 8px 宽、高度交错的窄字形（20/22/24/26 循环）。
        // 高度交错会让旧“沿较长方向”切分产生互不合并的窄条；新策略保持整宽下一行。
        for i in 0..14u32 {
            let h = 20 + (i % 4) * 2; // 20/22/24/26 循环
            sky.allocate(8, h, 0).expect("窄字形应能放入");
        }
        // 宽字形 (100×20)：旧策略下最大空闲矩形仅 80 宽会失败；新策略必落入某行的整宽矩形。
        assert!(sky.allocate(100, 20, 0).is_some(),
            "高度交错填满后，宽字形必须能放入下一行的整宽自由区");
    }

    #[test]
    fn text_like_glyph_mix_fits_single_page() {
        // 模拟 eg260810TextChain 示例：字形按文本块到达（26px 大块 → 20px → 30px+emoji → 16px → 24px）。
        // 空闲矩形分配器按行堆放，**到达顺序即可全部放入单张 1024² 页**，无需重排——
        // 这是“页未满却开新页”的回归测试。
        let page = 1024u32;
        let mut sky = Guillotine::new(page);
        let mut placed = Vec::new();
        let mut seed = 0x5DE_ECE_66u32;
        let mut failed = 0usize;

        fn next(seed: &mut u32) -> u32 {
            *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *seed
        }
        for &(sz, count) in &[(26.0f32, 120usize), (20.0, 35), (30.0, 40), (16.0, 40), (24.0, 16)] {
            for _ in 0..count {
                let r = next(&mut seed);
                let w = (sz * (0.4 + (r % 120) as f32 / 100.0)).round() as u32;
                let h = (sz * (1.0 + ((r >> 8) % 25) as f32 / 100.0)).round() as u32;
                match sky.allocate(w, h, 1) {
                    Some((x, y)) => placed.push((x, y, w, h)),
                    None => failed += 1,
                }
            }
        }
        for _ in 0..2 {
            let r = next(&mut seed);
            let w = 60 + r % 10;
            let h = 60 + (r >> 8) % 10;
            match sky.allocate(w, h, 1) {
                Some((x, y)) => placed.push((x, y, w, h)),
                None => failed += 1,
            }
        }

        assert_eq!(failed, 0, "到达顺序应全部放入单页，失败 {failed} 个");
        assert!(placed.len() >= 250, "应放入全部字形，实际 {}", placed.len());
        let max_bottom = placed.iter().map(|&(_, y, _, h)| y + h).max().unwrap();
        assert!(max_bottom <= page, "全部字形应保持在单页内，最低底边 {max_bottom} > {page}");
    }
}
