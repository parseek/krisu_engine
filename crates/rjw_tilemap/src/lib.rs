//! 任意图集区域贴片（`rjw_tilemap`）v3 —— 物件化 + Chunk 预生成顶点 + 相机剔除。
//!
//! - **Tile = 源裁剪 + 目标网格**：`{ src: RegionRef, src_tl/src_wh: 源内裁剪（像素，相对 AtlasRegion 左上角）,
//!   mesh_tl/mesh_wh: 目标位置/尺寸（局部坐标，负 = 翻转）}`——可从同一张图集精灵裁出任意子矩形贴片；
//! - **RegionRef**（`rjw_atlas`）：稳定 id + RAII 保活，动态图集**重排后仍可用**；
//! - **物件化**：`TileMap` 整体 `transform`（位移/旋转/缩放整个地图）；tile 矩形保持轴对齐（仅位移缩放）；
//! - **Chunk 预生成顶点数据**：每个 chunk 按（页, 层）预生成 GPU 静态 mesh（`MeshData`），
//!   结构/变换变更或图集重排（`generation` 变化）时按脏标记重建；每帧绘制 = 可见 chunk 的
//!   `add_static_mesh`（draw call ≈ 可见 chunk 数），**每帧零收集 / 零分组 / 零 resolve / 零堆分配**；
//! - **剔除抽象为 [`ViewCull`]**：[`TileMap::draw`] 接收 `Option<&impl ViewCull>`
//!   （2D 传 `&Camera2D`，`None` = 不剔除；3D 相机实现 trait 即可），内部用
//!   [`ViewCull::world_view_aabb`]（含旋转/缩放保守世界 AABB）逆变换到地图局部，按 chunk AABB 粗剔。

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use glam::Vec2;
use rjw_atlas::{AtlasRegion, DynamicAtlas, RegionRef};
use rjw_color::Color;
use rjw_render::{MeshData, MESHES, TEXTURES};
use rjw_transform::{Rect, Transform2D, ViewCull};
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
    /// 源图集条目（RAII 保活；重排后经 `resolve` 取最新 AtlasRegion）。
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
    pub color: Color,
    /// 相对基础层的层级偏移（按 (页, 层) 分 mesh）。
    pub layer: f32,
    /// 是否参与碰撞（`solid_rects` 收集）。
    pub solid: bool,
}

impl Tile {
    /// 从整张 `region` 贴片：`src_tl = (0,0)`、`src_wh = region.wh_px`。
    #[inline]
    pub fn whole_region(region: AtlasRegion, src: RegionRef, mesh_tl: Vec2, mesh_wh: Vec2) -> Self {
        Self {
            src,
            src_tl: Vec2::ZERO,
            src_wh: Vec2::new(region.wh_px.0 as f32, region.wh_px.1 as f32),
            mesh_tl,
            mesh_wh,
            color: Color::WHITE,
            layer: 0.0,
            solid: false,
        }
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
    mesh_id: u64,
    layer: f32,
}

/// chunk：块内按 `region_id` 预分组 + 局部并集 AABB + 预生成网格。
#[derive(Debug, Default)]
struct Chunk {
    aabb: Option<Rect>,
    /// region_id → 该 chunk 内使用此图集条目的 tile 索引（visible_count / 重建用）。
    groups: HashMap<u64, Vec<usize>>,
    /// 预生成顶点网格（每 (页, 层) 一个）。
    meshes: Vec<ChunkMesh>,
}

/// 贴片集合：Chunk 组织 + 预生成顶点 + 可选整体变换（物件化）+ 脏标记缓存。
#[derive(Debug)]
pub struct TileMap {
    tiles: Vec<Tile>,
    chunks: HashMap<(i32, i32), Chunk>,
    chunk_size: f32,
    /// 整体世界变换（`None` = 单位；旋转/缩放整个地图）。
    transform: Option<Transform2D>,
    /// 结构/变换脏标记：置位后 solid 缓存与 chunk mesh 重建。
    dirty: bool,
    /// solid 世界 AABB 缓存（静态地图时每帧零计算）。
    solid_cache: Vec<Rect>,
    /// 上次 chunk mesh 重建时的图集 generation（重排后自动重建）。
    atlas_gen: Option<u64>,
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
            transform: None,
            dirty: false,
            solid_cache: Vec::new(),
            atlas_gen: None,
        }
    }

    /// 整体世界变换（物件化：整个地图可位移/旋转/缩放）。
    #[inline]
    pub fn with_transform(mut self, transform: impl Into<Option<Transform2D>>) -> Self {
        self.set_transform(transform);
        self
    }

    #[inline]
    pub fn transform(&self) -> Option<Transform2D> {
        self.transform
    }

    #[inline]
    pub fn set_transform(&mut self, transform: impl Into<Option<Transform2D>>) -> &mut Self {
        self.transform = transform.into();
        self.dirty = true;
        self
    }

    /// 手动置脏（若你通过 [`Self::tiles`] 直接修改了 tile 字段，需调用本方法刷新缓存与网格）。
    #[inline]
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    #[inline]
    pub fn clear(&mut self) {
        for chunk in self.chunks.values_mut() {
            for m in std::mem::take(&mut chunk.meshes) {
                MESHES.remove(m.mesh_id);
            }
        }
        self.tiles.clear();
        self.chunks.clear();
        self.dirty = true;
    }

    /// 追加贴片：按左上角归属所在 chunk；chunk 内按 `region_id` 预分组 + 增量合并 AABB。
    #[inline]
    pub fn push(&mut self, tile: Tile) {
        let idx = self.tiles.len();
        let chunk_pos = self.chunk_of(tile.mesh_tl);
        let aabb = tile.aabb_local();
        let region_id = tile.src.region_id();
        self.tiles.push(tile);
        let chunk = self.chunks.entry(chunk_pos).or_default();
        chunk.aabb = Some(match chunk.aabb {
            Some(a) => a.union(&aabb),
            None => aabb,
        });
        chunk.groups.entry(region_id).or_default().push(idx);
        self.dirty = true;
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

    /// 世界空间 solid 贴片 AABB（缓存：结构/变换未变时每帧零计算零分配）。
    ///
    /// 直接返回内部缓存切片（供 `rjw_collision::move_and_collide` 等使用）。
    pub fn solid_rects(&mut self) -> &[Rect] {
        if self.dirty {
            self.solid_cache.clear();
            self.solid_cache.extend(
                self.tiles
                    .iter()
                    .filter(|tile| tile.solid)
                    .map(|tile| match self.transform {
                        Some(t) => tile.aabb_local().transform(&t),
                        None => tile.aabb_local(),
                    }),
            );
            self.dirty = false;
        }
        &self.solid_cache
    }

    /// 视图下可见贴片数（`None` = 全部）：chunk 粗剔后按组计数（与 mesh 提交粒度一致）。
    ///
    /// `view` 抽象为 [`ViewCull`]（2D = `&Camera2D`，3D = 视锥体实现）；剔除在
    /// **地图局部空间**做（视图 `world_view_aabb` 逆变换到局部后与 chunk AABB 相交）。
    #[inline]
    pub fn visible_count(&self, view: Option<&impl ViewCull>) -> usize {
        let Some(local_view) = self.local_view(view) else {
            return self.tiles.len();
        };
        self.chunks
            .values()
            .filter(|c| c.aabb.map_or(false, |a| a.intersects(&local_view)))
            .flat_map(|c| c.groups.values().flatten())
            .count()
    }

    /// 视图可见区（世界保守 AABB）→ 地图局部空间（保守逆变换；`None` = 不剔除）。
    fn local_view(&self, view: Option<&impl ViewCull>) -> Option<Rect> {
        let world_view = view?.world_view_aabb();
        match self.transform {
            Some(t) => Some(world_view.transform(&t.inverse())),
            None => Some(world_view),
        }
    }

    /// 渲染。`atlas` 提供 region 解析；`view` 为 `Some` 时按 [`ViewCull::world_view_aabb`]
    /// 世界 AABB 剔除（**chunk 粒度**粗剔，地图局部空间判定）。
    ///
    /// `view` 抽象为 [`ViewCull`]：2D 传 `&Camera2D`；3D 相机（视锥体）实现 trait 后可直接传入。
    /// 顶点数据在**首次绘制 / 结构或变换变更 / 图集重排**时按脏标记预生成（静态 mesh），
    /// 每帧仅做：chunk AABB 剔除 + `add_static_mesh` 提交（draw call ≈ 可见 chunk 数）。
    pub fn draw<K: Hash + Eq + Clone>(
        &mut self,
        r2d: &mut Render2D,
        atlas: &DynamicAtlas<K>,
        base_layer: impl Into<Layer>,
        view: Option<&impl ViewCull>,
    ) {
        if self.tiles.is_empty() {
            return;
        }
        // 重建预生成网格（结构/变换变更，或图集重排导致 UV 过期）。
        if self.dirty || self.atlas_gen != Some(atlas.generation()) {
            self.rebuild_meshes(r2d, atlas);
        }
        let base: f64 = base_layer.into().as_f64();
        let local_view = self.local_view(view);
        let map_t = self.transform.unwrap_or_default();

        for chunk in self.chunks.values() {
            let Some(ca) = chunk.aabb else { continue };
            if let Some(lv) = local_view {
                if !ca.intersects(&lv) {
                    continue;
                }
            }
            for m in &chunk.meshes {
                let Some(tex) = TEXTURES.get(m.page_uid) else { continue };
                r2d.add_static_mesh(
                    m.mesh_id,
                    Color::WHITE,
                    map_t,
                    Layer::from(base + m.layer as f64),
                    &tex,
                );
            }
        }
    }

    /// 重建全部 chunk 的预生成顶点网格（注销旧 mesh → 按 (页, 层) 生成顶点 → 注册新 mesh）。
    fn rebuild_meshes<K: Hash + Eq + Clone>(&mut self, r2d: &mut Render2D, atlas: &DynamicAtlas<K>) {
        for chunk in self.chunks.values_mut() {
            for m in std::mem::take(&mut chunk.meshes) {
                MESHES.remove(m.mesh_id);
            }
        }
        for (chunk_pos, chunk) in self.chunks.iter_mut() {
            let Some(_) = chunk.aabb else { continue };
            // 收集本 chunk 全部 tile 索引（region_id 分组展开）
            let all_idx: Vec<usize> = chunk.groups.values().flatten().copied().collect();
            if all_idx.is_empty() {
                continue;
            }
            // 按 (页, 层) 分组：组内共享纹理与绘制层级
            let mut buckets: HashMap<(u64, u32), Vec<usize>> = HashMap::new();
            for &i in &all_idx {
                let tile = &self.tiles[i];
                let Some(region) = tile.src.resolve(atlas) else { continue };
                buckets.entry((region.page_uid, tile.layer.to_bits())).or_default().push(i);
            }
            for ((page_uid, layer_bits), idxs) in buckets {
                let Some(tex) = TEXTURES.get(page_uid) else { continue };
                let pw = tex.width as f32;
                let ph = tex.height as f32;
                let mut verts: Vec<VertexP3U2C4> = Vec::with_capacity(idxs.len() * 4);
                let mut indices: Vec<u16> = Vec::with_capacity(idxs.len() * 6);
                for &i in &idxs {
                    let tile = &self.tiles[i];
                    let Some(region) = tile.src.resolve(atlas) else { continue };
                    let u0 = (region.tl_px.0 as f32 + tile.src_tl.x) / pw;
                    let v0 = (region.tl_px.1 as f32 + tile.src_tl.y) / ph;
                    let uw = tile.src_wh.x / pw;
                    let vh = tile.src_wh.y / ph;
                    let tl = tile.mesh_tl;
                    let wh = tile.mesh_wh;
                    let c: [f32; 4] = tile.color.into();
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
                let mesh_id = MESHES.register(Arc::new(mesh));
                chunk.meshes.push(ChunkMesh { page_uid, mesh_id, layer: f32::from_bits(layer_bits) });
            }
        }
        self.dirty = false;
        self.atlas_gen = Some(atlas.generation());
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
            color: Color::WHITE,
            layer: 0.0,
            solid: false,
        }
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
        m.set_transform(Transform2D::IDENTITY.with_pos(Vec2::new(100.0, 50.0)));
        let first = m.solid_rects()[0];
        assert_eq!(first, Rect::new(100.0, 50.0, 10.0, 10.0), "平移后世界 AABB");
        assert_eq!(m.solid_rects()[0], first, "未置脏应复用缓存");
        m.set_transform(Transform2D::IDENTITY.with_pos(Vec2::ZERO).with_rot(0.785398));
        let r0 = m.solid_rects()[0];
        let t = m.transform().unwrap();
        for c in [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0), Vec2::new(10.0, 10.0)] {
            assert!(r0.contains_point(t.transform_point(c)), "旋转后角点 {c:?} 应在保守 AABB 内");
        }
    }

    #[test]
    fn visible_count_without_camera_is_all() {
        let mut m = TileMap::new(512.0);
        m.push(tile(1, 1, 0.0, 0.0, 64.0, 64.0));
        m.push(tile(1, 1, 1000.0, 1000.0, 64.0, 64.0));
        assert_eq!(m.visible_count(None::<&rjw_transform::Camera2D>), 2);
    }
}
