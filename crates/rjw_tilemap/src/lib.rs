//! 任意图集区域贴片（`rjw_tilemap`）v4 —— 物件化 + Chunk 预生成顶点 + 单一剔除语言。
//!
//! - **Tile = 源裁剪 + 目标网格**：`{ src: RegionRef, src_tl/src_wh: 源内裁剪（像素，相对 AtlasRegion 左上角）,
//!   mesh_tl/mesh_wh: 目标位置/尺寸（局部坐标，负 = 翻转）}`——可从同一张图集精灵裁出任意子矩形贴片；
//! - **RegionRef**（`rjw_atlas`）：稳定 id + RAII 保活，动态图集**重排后仍可用**；
//! - **物件化**：`TileMap` 整体 `transform`（位移/旋转/缩放整个地图）；tile 矩形保持轴对齐（仅位移缩放）；
//! - **Chunk 预生成顶点数据**：每个 chunk 按（页, 层）预生成 GPU 静态 mesh（`MeshData`），
//!   结构变更或图集重排（`generation` 变化）时按脏标记重建；每帧绘制 = 可见 chunk 的
//!   `static_mesh`（draw call ≈ 可见 chunk 数），**每帧零分组 / 零 resolve / 零堆分配**；
//! - **剔除只有一个语言**：[`TileMap::draw`] 直接复用 [`Render2D::culler`] 的当前模式
//!   （`Cull::Off` / `Rect` / `Viewport` / `Fn`）做 chunk 级粗剔——想剔除就
//!   `r2d.cull(Cull::Rect(cam.view_aabb()))`，无需再传闭包（旧 `Option<&dyn Fn>` 已删除）。
//!
//! 责任边界（`docs/API_DESIGN.md` §8.6）：
//! - `TileMap` 只负责「贴片集合 → 可见 chunk 的静态网格提交」；
//! - 图集归属、纹理页、寿命由 [`DynamicAtlas`] 负责；本类型只经 [`RegionRef::resolve`] 读最新区域。

use std::cell::{Cell, Ref, RefCell};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use glam::Vec2;
use rjw_atlas::{DynamicAtlas, RegionRef};
use rjw_color::Color;
use rjw_render::{MeshData, MeshId};
use rjw_transform::{Rect, Transform2D};
use rjw_2d_render::{Layer, Render2D, VertexP3U2C4};

/// 默认 chunk 尺寸（世界像素）。
///
/// 权衡：chunk 越大 → 粗剔粒度越粗（draw call 数 ≈ 可见 chunk 数更少），但单 chunk
/// 顶点缓冲/重建成本更高；高分辨率（大视口）下取 1024 比 512 更合适（视口 1920×1080
/// 时 512 → 约 4×3 个 chunk，1024 → 2×2 个）。
pub const DEFAULT_CHUNK_SIZE: f32 = 1024.0;

/// 一张贴片：源图集子矩形 → 目标网格矩形（轴对齐，负尺寸 = 翻转）。
#[derive(Debug, Clone)]
pub struct Tile {
    /// 源图集条目（RAII 保活；重排后经 `resolve` 取最新 `AtlasRegion`）。
    pub src: RegionRef,
    /// 源内裁剪起点（**像素**，相对 `AtlasRegion.tl_px`；`(0,0)` = 整张精灵）。
    pub src_tl: Vec2,
    /// 源内裁剪尺寸（**像素**，可负 = 翻转；`(0,0)` 表示整张精灵）。
    pub src_wh: Vec2,
    /// 目标位置（地图局部坐标左上角）。
    pub mesh_tl: Vec2,
    /// 目标尺寸（可负 = 翻转；AABB/剔除按归一化）。
    pub mesh_wh: Vec2,
    /// 着色（烘焙进顶点颜色）。
    pub tint: Color,
    /// 相对基础层的层级偏移（按 (页, 层) 分 mesh）。
    pub layer: f32,
    /// 是否参与碰撞（[`TileMap::solid_rects`] 收集）。
    pub solid: bool,
}

impl Tile {
    /// **最常用**：一个贴片（**整张精灵**）。`src` 会被克隆（`RegionRef` 是 RAII 保活句柄）。
    ///
    /// 位置 / 尺寸接受 `Vec2` 或 `(x, y)`；其余字段用链式覆盖：
    ///
    /// ```ignore
    /// map.push(Tile::new(&grass, (x * 64.0, y * 64.0), (64.0, 64.0)));
    /// map.push(Tile::new(&stone, pos, (64.0, 64.0)).solid(true).tint(Color::rgba(1.0, 1.0, 1.0, 0.8)));
    /// ```
    #[inline]
    pub fn new(src: &RegionRef, mesh_tl: impl Into<Vec2>, mesh_wh: impl Into<Vec2>) -> Self {
        Self {
            src: src.clone(),
            src_tl: Vec2::ZERO,
            // (0,0) = 整张精灵（渲染时按 `AtlasRegion` 全尺寸解析）。
            src_wh: Vec2::ZERO,
            mesh_tl: mesh_tl.into(),
            mesh_wh: mesh_wh.into(),
            tint: Color::WHITE,
            layer: 0.0,
            solid: false,
        }
    }

    /// 源内裁剪（像素，相对 `AtlasRegion.tl_px`；可负 = 翻转；`(0,0)` 表示整张精灵）。
    #[inline]
    pub fn uv(mut self, src_tl: impl Into<Vec2>, src_wh: impl Into<Vec2>) -> Self {
        self.src_tl = src_tl.into();
        self.src_wh = src_wh.into();
        self
    }

    /// 着色（烘焙进顶点颜色）。
    #[inline]
    pub fn tint(mut self, color: Color) -> Self {
        self.tint = color;
        self
    }

    /// 相对基础层的层级偏移（按 (页, 层) 分 mesh）。
    #[inline]
    pub fn layer(mut self, layer: f32) -> Self {
        self.layer = layer;
        self
    }

    /// 是否参与碰撞（[`TileMap::solid_rects`] 收集）。
    #[inline]
    pub fn solid(mut self, solid: bool) -> Self {
        self.solid = solid;
        self
    }

    /// 局部 AABB（负尺寸归一化）。
    #[inline]
    pub fn aabb_local(&self) -> Rect {
        Rect::new(self.mesh_tl.x, self.mesh_tl.y, self.mesh_wh.x, self.mesh_wh.y).normalized()
    }
}

/// 预生成顶点网格（静态 mesh，GPU 已上传）。
#[derive(Debug)]
struct ChunkMesh {
    page_uid: u64,
    mesh_id: MeshId,
    layer: f32,
}

/// chunk：局部并集 AABB + 块内 tile 索引 + 预生成网格。
#[derive(Debug, Default)]
struct Chunk {
    aabb: Option<Rect>,
    /// 该 chunk 内的 tile 索引（`push` 时增量维护；重建 mesh 用）。
    indices: Vec<usize>,
    /// 预生成顶点网格（每 (页, 层) 一个）。
    meshes: Vec<ChunkMesh>,
}

/// 贴片集合：Chunk 组织 + 预生成顶点 + 整体变换（物件化）+ 脏标记缓存。
#[derive(Debug)]
pub struct TileMap {
    tiles: Vec<Tile>,
    chunks: HashMap<(i32, i32), Chunk>,
    chunk_size: f32,
    /// 整体世界变换（默认 [`Transform2D::IDENTITY`]；旋转/缩放整个地图）。
    transform: Transform2D,
    /// 网格脏（结构 / 内容 / 图集世代变化）→ 重建静态网格。
    ///
    /// **与 solid 缓存分离**：旧实现两者共用 `dirty`，`solid_rects()` 先跑到就把它清掉，
    /// 随后 `draw()` 认为无需重建 ⇒ `push` 进去的 tile 永远不出现（B4）。
    mesh_dirty: bool,
    /// solid 世界 AABB 缓存（`solid_rects(&self)` 需内部可变）。
    solid_cache: RefCell<Vec<Rect>>,
    /// solid 缓存脏标记（`Cell`：`solid_rects` 只收 `&self`）。
    solid_dirty: Cell<bool>,
    /// 上次 chunk mesh 重建时的图集**区域失效世代号**
    /// （`DynamicAtlas::revision`：重排搬动**或**空闲槽位重新可分配都会推进）——
    /// 网格里烘着 UV，图集一变就必须重建（用 `generation()` 会漏掉"逐出 + 槽位复用"，
    /// 导致贴片采样到别的精灵像素）。
    atlas_gen: Option<u64>,
    /// 每帧可见网格的复用缓冲（避免与 `r2d` 的可变借用冲突，且零分配）。
    draw_buf: Vec<(u64, MeshId, f32)>,
    /// **待回收的网格句柄**：`clear()` 拿不到 `r2d`（注册表归所属 `RenderContext`），
    /// 因此把注销推迟到下一次 `draw` —— 那时有 `r2d.meshes()` 可用。
    ///
    /// 旧实现直接调全局 `MESHES.remove`，在注册表实例化后不再可能。
    pending_mesh_reap: Vec<MeshId>,
}

impl Default for TileMap {
    fn default() -> Self {
        Self::new(DEFAULT_CHUNK_SIZE)
    }
}

impl TileMap {
    #[inline]
    pub fn new(chunk_size: f32) -> Self {
        Self {
            tiles: Vec::new(),
            chunks: HashMap::new(),
            chunk_size: chunk_size.max(1.0),
            transform: Transform2D::IDENTITY,
            mesh_dirty: false,
            solid_cache: RefCell::new(Vec::new()),
            solid_dirty: Cell::new(true),
            atlas_gen: None,
            draw_buf: Vec::new(),
            pending_mesh_reap: Vec::new(),
        }
    }

    /// 整体世界变换（物件化：整个地图可位移 / 旋转 / 缩放）。
    #[inline]
    pub fn with_transform(mut self, transform: Transform2D) -> Self {
        self.set_transform(transform);
        self
    }

    /// 整体世界变换（默认 [`Transform2D::IDENTITY`]）。
    #[inline]
    pub fn transform(&self) -> Transform2D {
        self.transform
    }

    /// 设置整体世界变换。
    ///
    /// 变换只在**绘制期**作用到网格上，故不需要重建 mesh；但 solid 世界 AABB 会变。
    #[inline]
    pub fn set_transform(&mut self, transform: Transform2D) -> &mut Self {
        self.transform = transform;
        self.solid_dirty.set(true);
        self
    }

    /// 可变访问全部贴片（读用 [`Self::tiles`]）：**自动置脏**（网格 + solid 缓存）。
    #[inline]
    pub fn tiles_mut(&mut self) -> &mut [Tile] {
        self.mesh_dirty = true;
        self.solid_dirty.set(true);
        &mut self.tiles
    }

    /// 清空全部贴片与 chunk。
    ///
    /// 网格从**所属 `RenderContext` 的**网格注册表注销需要 `r2d`（本方法没有），
    /// 因此句柄入队 [`Self::pending_mesh_reap`]，在下一次 [`Self::draw`] 开头统一注销。
    #[inline]
    pub fn clear(&mut self) {
        // 先把句柄收集进局部变量（`pending_mesh_reap` 与 `chunks` 都是 `self` 的字段，
        // 直接在同一次 `values_mut()` 迭代里写会同时可变借用 `self`）。
        let reaped: Vec<MeshId> = self
            .chunks
            .values_mut()
            .flat_map(|chunk| std::mem::take(&mut chunk.meshes))
            .map(|m| m.mesh_id)
            .collect();
        self.pending_mesh_reap.extend(reaped);
        self.tiles.clear();
        self.chunks.clear();
        self.mesh_dirty = true;
        self.solid_dirty.set(true);
    }

    /// 追加贴片：按左上角归属所在 chunk；chunk 内增量合并 AABB + 记录索引。
    #[inline]
    pub fn push(&mut self, tile: Tile) {
        let idx = self.tiles.len();
        let chunk_pos = self.chunk_of(tile.mesh_tl);
        let aabb = tile.aabb_local();
        self.tiles.push(tile);
        let chunk = self.chunks.entry(chunk_pos).or_default();
        chunk.aabb = Some(match chunk.aabb {
            Some(a) => a.union(&aabb),
            None => aabb,
        });
        chunk.indices.push(idx);
        self.mesh_dirty = true;
        self.solid_dirty.set(true);
    }

    #[inline]
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    #[inline]
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    #[inline]
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    #[inline]
    fn chunk_of(&self, pos: Vec2) -> (i32, i32) {
        (
            (pos.x / self.chunk_size).floor() as i32,
            (pos.y / self.chunk_size).floor() as i32,
        )
    }

    /// 世界空间 solid 贴片 AABB（**缓存**：结构 / 变换未变时每帧零计算零分配）。
    ///
    /// 直接返回内部缓存切片（供 `rjw_collision::Aabb::slide` 等使用）。只收 `&self`
    /// （缓存经 `RefCell` 内部可变）。⚠ 持有返回值期间**不要**再调用 `&mut self` 方法
    /// （`push` / `set_transform` / `tiles_mut` / `clear`）——那会 panic（借用冲突）。
    pub fn solid_rects(&self) -> Ref<'_, [Rect]> {
        if self.solid_dirty.get() {
            let mut cache = self.solid_cache.borrow_mut();
            cache.clear();
            cache.extend(
                self.tiles
                    .iter()
                    .filter(|tile| tile.solid)
                    .map(|tile| tile.aabb_local().transform(&self.transform)),
            );
            self.solid_dirty.set(false);
        }
        Ref::map(self.solid_cache.borrow(), |v| v.as_slice())
    }

    /// 渲染：把**可见 chunk** 的预生成网格提交给 `r2d`。
    ///
    /// - 剔除复用 `r2d` 的当前剔除模式（`r2d.cull(..)`；`Cull::Off` = 不剔除）：
    ///   在**世界空间**按 chunk AABB 粗剔，与命令级剔除（只作用于 Sprite 命令）互不干扰；
    /// - 顶点数据在**结构变更 / 图集重排**时按脏标记预生成（静态 mesh），每帧仅做
    ///   「chunk AABB 判定 + `static_mesh` 提交」（draw call ≈ 可见 chunk 数）；
    /// - 每帧零分组 / 零 resolve / 零堆分配（可见网格收集进复用缓冲）。
    pub fn draw<K: Hash + Eq + Clone>(
        &mut self,
        r2d: &mut Render2D,
        atlas: &DynamicAtlas<K>,
        layer: impl Into<Layer>,
    ) {
        // 先注销 `clear()` 排队的网格句柄——**必须在下面的提前 return 之前**：
        // 「clear 之后此帧无贴片」时也要回收，否则网格永久驻留注册表。
        if !self.pending_mesh_reap.is_empty() {
            for mesh_id in self.pending_mesh_reap.drain(..) {
                r2d.meshes().remove(mesh_id.uid());
            }
        }
        if self.tiles.is_empty() {
            return;
        }
        // 重建预生成网格（结构 / 内容变更，或图集重排导致 UV 过期）。
        if self.mesh_dirty || self.atlas_gen != Some(atlas.revision()) {
            self.rebuild_meshes(r2d, atlas);
        }
        let base: f64 = layer.into().as_f64();
        let map_t = self.transform;

        // ① 判定 + 收集：只读借用 `r2d` 的剔除器；`self.draw_buf` 与 `self.chunks` 是不相交字段。
        self.draw_buf.clear();
        {
            let culler = r2d.culler();
            for chunk in self.chunks.values() {
                let Some(ca) = chunk.aabb else { continue };
                if !culler.visible(&ca.transform(&map_t)) {
                    continue;
                }
                for m in &chunk.meshes {
                    self.draw_buf.push((m.page_uid, m.mesh_id, m.layer));
                }
            }
        }

        // ② 提交（可变借用 `r2d`）
        for &(page_uid, mesh_id, mesh_layer) in &self.draw_buf {
            let Some(tex) = r2d.textures().get(page_uid) else { continue };
            r2d.static_mesh(mesh_id, &tex)
                .tint(Color::WHITE)
                .transform(map_t)
                .layer(Layer::from(base + mesh_layer as f64));
        }
    }

    /// 重建全部 chunk 的预生成顶点网格（注销旧 mesh → 按 (页, 层) 生成顶点 → 注册新 mesh）。
    fn rebuild_meshes<K: Hash + Eq + Clone>(&mut self, r2d: &mut Render2D, atlas: &DynamicAtlas<K>) {
        {
            // 收集后统一注销：`r2d.meshes()` 是不可变借用，与 `self.chunks` 的可变借用
            // 不冲突（`r2d` 与 `self` 是不同对象），但为清晰起见仍先收集。
            let old: Vec<MeshId> = self
                .chunks
                .values_mut()
                .flat_map(|chunk| std::mem::take(&mut chunk.meshes))
                .map(|m| m.mesh_id)
                .collect();
            for mesh_id in old {
                r2d.meshes().remove(mesh_id.uid());
            }
        }
        for (chunk_pos, chunk) in self.chunks.iter_mut() {
            if chunk.aabb.is_none() || chunk.indices.is_empty() {
                continue;
            }
            // 按 (页, 层) 分组：组内共享纹理与绘制层级
            let mut buckets: HashMap<(u64, u32), Vec<usize>> = HashMap::new();
            for &i in &chunk.indices {
                let tile = &self.tiles[i];
                let Some(region) = tile.src.resolve(atlas) else { continue };
                buckets.entry((region.page_uid, tile.layer.to_bits())).or_default().push(i);
            }
            // 确定性顺序（`HashMap` 迭代序随机）：按 (页, 层) 升序生成 mesh。
            let mut keys: Vec<(u64, u32)> = buckets.keys().copied().collect();
            keys.sort_unstable();
            for key in keys {
                let (page_uid, layer_bits) = key;
                let idxs = &buckets[&key];
                let Some(tex) = r2d.textures().get(page_uid) else { continue };
                let pw = tex.width as f32;
                let ph = tex.height as f32;
                let mut verts: Vec<VertexP3U2C4> = Vec::with_capacity(idxs.len() * 4);
                let mut indices: Vec<u16> = Vec::with_capacity(idxs.len() * 6);
                for &i in idxs {
                    let tile = &self.tiles[i];
                    let Some(region) = tile.src.resolve(atlas) else { continue };
                    let u0 = (region.tl_px.0 as f32 + tile.src_tl.x) / pw;
                    let v0 = (region.tl_px.1 as f32 + tile.src_tl.y) / ph;
                    // `src_wh` 为 `(0,0)` 时 = 整张精灵（按 `AtlasRegion` 全尺寸解析）。
                    let sw = if tile.src_wh.x == 0.0 { region.wh_px.0 as f32 } else { tile.src_wh.x };
                    let sh = if tile.src_wh.y == 0.0 { region.wh_px.1 as f32 } else { tile.src_wh.y };
                    let uw = sw / pw;
                    let vh = sh / ph;
                    let tl = tile.mesh_tl;
                    let wh = tile.mesh_wh;
                    let c: [f32; 4] = tile.tint.into();
                    // **索引是 u16**：一个 (chunk, 页, 层) 段最多 65535 个顶点。
                    // 旧实现直接 `verts.len() as u16` —— 超过后**静默回绕**，产出垃圾三角形。
                    // 默认 chunk 1024px + 8px 瓦片 = 16384 quad（恰好越界），所以这是可达路径。
                    assert!(
                        verts.len() + 4 <= u16::MAX as usize,
                        "tilemap: 单个 (chunk, 页, 层) 段的顶点数 {} 超过 u16 索引上限 {}——\
                         请减小 `chunk_size` 或增大瓦片尺寸（一个 chunk 内同页同层的瓦片不能超过 {} 个）",
                        verts.len() + 4,
                        u16::MAX,
                        (u16::MAX as usize) / 4
                    );
                    let base = verts.len() as u16;
                    verts.push(VertexP3U2C4 { pos: [tl.x, tl.y, 0.0], uv: [u0, v0], color: c });
                    verts.push(VertexP3U2C4 { pos: [tl.x + wh.x, tl.y, 0.0], uv: [u0 + uw, v0], color: c });
                    verts.push(VertexP3U2C4 { pos: [tl.x, tl.y + wh.y, 0.0], uv: [u0, v0 + vh], color: c });
                    verts.push(VertexP3U2C4 { pos: [tl.x + wh.x, tl.y + wh.y, 0.0], uv: [u0 + uw, v0 + vh], color: c });
                    indices.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
                }
                if verts.is_empty() {
                    continue;
                }
                let label = format!("tilemap chunk {chunk_pos:?} page {page_uid}");
                let mesh = MeshData::from_pod(r2d.device(), &verts, &indices, &label);
                let mesh_id = MeshId::new(r2d.meshes().register(Arc::new(mesh)));
                chunk.meshes.push(ChunkMesh { page_uid, mesh_id, layer: f32::from_bits(layer_bits) });
            }
        }
        self.mesh_dirty = false;
        self.atlas_gen = Some(atlas.revision());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(region_id: u64, page_uid: u64, x: f32, y: f32, w: f32, h: f32) -> Tile {
        Tile {
            src: RegionRef::from_parts(region_id, page_uid),
            src_tl: Vec2::ZERO,
            src_wh: Vec2::new(64.0, 64.0),
            mesh_tl: Vec2::new(x, y),
            mesh_wh: Vec2::new(w, h),
            tint: Color::WHITE,
            layer: 0.0,
            solid: false,
        }
    }

    #[test]
    fn tile_new_defaults_to_whole_sprite() {
        // `Tile::new` = 整张精灵（`src_wh = (0,0)` 由渲染侧解析为 `AtlasRegion` 全尺寸）+
        // 位置 / 尺寸接受 `Vec2` 或 `(x, y)`；链式覆盖其余字段（`src` 句柄被克隆）。
        let src = RegionRef::from_parts(1, 2);
        let t = Tile::new(&src, (10.0, 20.0), (64.0, 64.0));
        assert_eq!(t.src.region_id(), src.region_id());
        assert_eq!(t.src_tl, Vec2::ZERO);
        assert_eq!(t.src_wh, Vec2::ZERO, "(0,0) = 整张精灵");
        assert_eq!(t.mesh_tl, Vec2::new(10.0, 20.0));
        assert_eq!(t.mesh_wh, Vec2::splat(64.0));
        assert_eq!(t.tint, Color::WHITE);
        assert_eq!(t.layer, 0.0);
        assert!(!t.solid);

        let t = t
            .uv((4.0, 8.0), (16.0, 16.0))
            .solid(true)
            .layer(2.0)
            .tint(Color::RED);
        assert_eq!(t.src_tl, Vec2::new(4.0, 8.0));
        assert_eq!(t.src_wh, Vec2::new(16.0, 16.0));
        assert!(t.solid);
        assert_eq!(t.layer, 2.0);
        assert_eq!(t.tint, Color::RED);
    }

    #[test]
    fn tile_aabb_normalizes_negative_size() {
        let t = tile(1, 1, 10.0, 20.0, -8.0, -4.0);
        assert_eq!(t.aabb_local(), Rect::new(2.0, 16.0, 8.0, 4.0), "负尺寸应归一化");
    }

    #[test]
    fn chunk_assigns_by_top_left_and_unions_crossing_tiles() {
        let mut m = TileMap::new(512.0);
        // tile 左上角在 chunk (0,0)，但尺寸跨入 chunk (1,0) → aabb 覆盖跨界部分
        m.push(tile(1, 1, 500.0, 10.0, 40.0, 40.0));
        assert_eq!(m.chunk_count(), 1, "跨界 tile 仍归属左上角所在 chunk");
        let c = m.chunks.get(&(0, 0)).unwrap();
        assert_eq!(c.aabb.unwrap(), Rect::new(500.0, 10.0, 40.0, 40.0), "跨界部分计入 chunk AABB");
        assert_eq!(c.indices.len(), 1);
        m.push(tile(1, 1, 600.0, 10.0, 40.0, 40.0));
        assert_eq!(m.chunk_count(), 2, "600 归属 chunk (1,0)");
        assert_eq!(m.tile_count(), 2);
    }

    #[test]
    fn solid_rects_apply_map_transform_conservatively() {
        let mut m = TileMap::new(512.0);
        let mut t = tile(1, 1, 0.0, 0.0, 10.0, 10.0);
        t.solid = true;
        m.push(t);
        // 默认变换 = IDENTITY：世界 AABB = 局部 AABB
        assert_eq!(m.solid_rects()[0], Rect::new(0.0, 0.0, 10.0, 10.0));
        m.set_transform(Transform2D::IDENTITY.with_pos(Vec2::new(100.0, 50.0)));
        let first = m.solid_rects()[0];
        assert_eq!(first, Rect::new(100.0, 50.0, 10.0, 10.0), "平移后世界 AABB");
        assert_eq!(m.solid_rects()[0], first, "未置脏应复用缓存");
        m.set_transform(Transform2D::IDENTITY.with_pos(Vec2::ZERO).with_rot(std::f32::consts::FRAC_PI_4));
        let r0 = m.solid_rects()[0];
        let t = m.transform();
        for c in [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0), Vec2::new(10.0, 10.0)] {
            assert!(r0.contains_point(t.transform_point(c)), "旋转后角点 {c:?} 应在保守 AABB 内");
        }
    }

    /// **B4 回归**：`solid_rects()` 只清 solid 缓存，**不得**清网格脏标记
    /// （旧实现两者共用 `dirty` ⇒ `push → solid_rects → draw` 丢 tile）。
    #[test]
    fn solid_rects_does_not_consume_mesh_dirty() {
        let mut m = TileMap::new(512.0);
        let mut t = tile(1, 1, 0.0, 0.0, 10.0, 10.0);
        t.solid = true;
        m.push(t);
        assert!(m.mesh_dirty, "push 后网格应脏");
        assert!(m.solid_dirty.get(), "push 后 solid 缓存应脏");

        assert_eq!(m.solid_rects().len(), 1);
        assert!(m.mesh_dirty, "solid_rects 不应清除网格脏标记（B4）");
        assert!(!m.solid_dirty.get(), "solid 缓存应已刷新");
    }

    #[test]
    fn tiles_mut_marks_dirty() {
        let mut m = TileMap::new(512.0);
        m.push(tile(1, 1, 0.0, 0.0, 64.0, 64.0));
        let _ = m.solid_rects(); // 清 solid 脏
        m.mesh_dirty = false;
        m.tiles_mut()[0].solid = true;
        assert!(m.mesh_dirty, "tiles_mut 应置网格脏");
        assert!(m.solid_dirty.get(), "tiles_mut 应置 solid 脏");
    }

    #[test]
    fn transform_change_does_not_dirty_meshes() {
        let mut m = TileMap::new(512.0);
        m.push(tile(1, 1, 0.0, 0.0, 64.0, 64.0));
        m.mesh_dirty = false;
        m.set_transform(Transform2D::IDENTITY.with_pos(Vec2::new(5.0, 5.0)));
        assert!(!m.mesh_dirty, "地图变换在绘制期应用，不需要重建 mesh");
        assert!(m.solid_dirty.get(), "地图变换会改变 solid 世界 AABB");
    }
}
