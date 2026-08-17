//! 任意图集区域贴片（`rjw_tilemap`）v2 —— 物件化 + Chunk+AABB。
//!
//! - **任意区域贴片**：每个 tile 引用一个 [`RegionRef`]（`rjw_atlas` 稳定句柄，
//!   动态图集**去碎片重排后仍可用**、被引用条目不逐出），任意位置/尺寸粘贴。
//! - **矩形仅位移+缩放**：tile 是轴对齐矩形，`size` 为负 = 翻转（负宽水平镜像、负高垂直镜像）；
//!   任意变换（旋转/缩放）由 **TileMap 整体 `transform`**（物件化）承担。
//! - **Chunk + AABB**：`chunk_size` 可配置（默认 512 世界像素）；tile 按**左上角**归属 chunk，
//!   chunk 的 AABB = 块内所有 tile 的**并集**（含跨界部分）——剔除按 AABB 相交判定，
//!   不依赖"选择框"索引，跨界 tile 不会漏绘/错绘。
//! - **剔除直接收相机**：[`TileMap::draw`] 接收 `&Camera2D`（`None` = 不剔除），内部用
//!   [`Camera2D::view_aabb`]（含旋转/缩放的保守世界 AABB）逆变换到地图局部做两级剔除。
//! - 渲染复用 `rjw_2d_render::Render2D` 实例化 sprite 路径，按 `page_uid` 分组。

use std::collections::HashMap;
use std::hash::Hash;

use glam::Vec2;
use rjw_atlas::{AtlasRegion, DynamicAtlas, RegionRef};
use rjw_color::Color;
use rjw_render::TEXTURES;
use rjw_transform::{Camera2D, Rect, Transform2D};
use rjw_2d_render::{Layer, Render2D, SpriteRect};

/// 默认 chunk 尺寸（世界像素）。
pub const DEFAULT_CHUNK_SIZE: f32 = 512.0;

/// 一张贴片：图集区域句柄 + 局部位置/尺寸（轴对齐；`size` 负 = 翻转）。
#[derive(Debug, Clone)]
pub struct Tile {
    /// 图集区域句柄（RAII 保活；重排后经 `resolve` 取最新 UV）。
    pub region: RegionRef,
    /// 地图局部坐标左上角（整体变换由 [`TileMap::transform`] 承担）。
    pub pos: Vec2,
    /// 局部尺寸（负 = 翻转；AABB/剔除按归一化）。
    pub size: Vec2,
    /// 着色（彩色内容建议 `Color::WHITE`）。
    pub color: Color,
    /// 相对基础层的层级偏移。
    pub layer: f32,
    /// 是否参与碰撞（`solid_rects` 收集）。
    pub solid: bool,
}

impl Tile {
    #[inline]
    pub fn new(region: RegionRef, pos: Vec2, size: Vec2) -> Self {
        Self {
            region,
            pos,
            size,
            color: Color::WHITE,
            layer: 0.0,
            solid: false,
        }
    }

    /// 局部 AABB（负尺寸归一化）。
    #[inline]
    pub fn aabb_local(&self) -> Rect {
        Rect::new(self.pos.x, self.pos.y, self.size.x, self.size.y).normalized()
    }
}

/// chunk：块内按 `region_id` **预分组**的 tile 索引 + 局部并集 AABB（含跨界部分）。
///
/// 预分组让渲染时**每组只 resolve 一次**图集区域（region_id 稳定，重排/换页不影响分组），
/// 组内 tile 共享同一 UV / page_uid——避免每 tile 重复 HashMap 查找。
#[derive(Debug, Default)]
struct Chunk {
    aabb: Option<Rect>,
    /// region_id → 该 chunk 内使用此图集条目的 tile 索引。
    groups: HashMap<u64, Vec<usize>>,
}

/// 贴片集合：Chunk 组织 + 可选整体变换（物件化）+ 脏标记缓存。
#[derive(Debug)]
pub struct TileMap {
    tiles: Vec<Tile>,
    chunks: HashMap<(i32, i32), Chunk>,
    chunk_size: f32,
    /// 整体世界变换（`None` = 单位；旋转/缩放整个地图）。
    transform: Option<Transform2D>,
    /// 结构/变换脏标记：置位后 `solid_rects` 缓存重建。
    dirty: bool,
    /// solid 世界 AABB 缓存（静态地图时每帧零计算）。
    solid_cache: Vec<Rect>,
    /// draw 内部 scratch（复用容量，避免每帧堆分配）。
    scratch_pages: Vec<(u64, Vec<(usize, AtlasRegion)>)>,
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
            scratch_pages: Vec::new(),
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

    /// 手动置脏（若你通过 [`Self::tiles`] 直接修改了 tile 字段，需调用本方法刷新缓存）。
    #[inline]
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    #[inline]
    pub fn clear(&mut self) {
        self.tiles.clear();
        self.chunks.clear();
        self.dirty = true;
    }

    /// 追加贴片：按左上角归属所在 chunk；chunk 内按 `region_id` 预分组 + 增量合并 AABB。
    #[inline]
    pub fn push(&mut self, tile: Tile) {
        let idx = self.tiles.len();
        let chunk_pos = self.chunk_of(tile.pos);
        let aabb = tile.aabb_local();
        let region_id = tile.region.region_id();
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

    /// 相机下可见贴片数（`None` = 全部）：chunk 粗剔 + tile 精剔，与 [`Self::draw`] 同逻辑。
    #[inline]
    pub fn visible_count(&self, cam: Option<&Camera2D>) -> usize {
        let Some(local_view) = self.local_view(cam) else {
            return self.tiles.len();
        };
        self.chunks
            .values()
            .filter(|c| c.aabb.map_or(false, |a| a.intersects(&local_view)))
            .flat_map(|c| c.groups.values().flatten())
            .filter(|&&i| self.tiles[i].aabb_local().intersects(&local_view))
            .count()
    }

    /// 相机世界视口 AABB → 地图局部空间（保守逆变换；`None` = 不剔除）。
    fn local_view(&self, cam: Option<&Camera2D>) -> Option<Rect> {
        let world_view = cam?.view_aabb();
        match self.transform {
            Some(t) => Some(world_view.transform(&t.inverse())),
            None => Some(world_view),
        }
    }

    /// 渲染。`atlas` 提供 region 解析（动态图集重排后取最新 UV）；
    /// `cam` 为 `Some` 时按 [`Camera2D::view_aabb`] 世界 AABB 剔除（chunk 粗剔 + tile 精剔）。
    ///
    /// 内部优化：
    /// - chunk 内按 `region_id` **预分组** → 每组只 resolve 一次图集区域，组内 tile 共享 UV/page；
    /// - 复用内部 scratch 缓冲（`&mut self`），**每帧零堆分配**；
    /// - 按 `page_uid` 分组提交（一个 mesh 只绑一张纹理页）；整体变换作用于每个 tile quad。
    pub fn draw<K: Hash + Eq + Clone>(
        &mut self,
        r2d: &mut Render2D,
        atlas: &DynamicAtlas<K>,
        base_layer: impl Into<Layer>,
        cam: Option<&Camera2D>,
    ) {
        if self.tiles.is_empty() {
            return;
        }
        let base: f64 = base_layer.into().as_f64();
        let local_view = self.local_view(cam);
        let map_t = self.transform.unwrap_or_default();

        // 收集可见 tile（chunk 粗剔 + group 内精剔），每组 resolve 一次 region。
        self.scratch_pages.clear();
        for chunk in self.chunks.values() {
            let Some(ca) = chunk.aabb else { continue };
            if let Some(lv) = local_view {
                if !ca.intersects(&lv) {
                    continue;
                }
            }
            for (&region_id, idxs) in &chunk.groups {
                let Some(region) = atlas.resolve_by_id(region_id) else { continue };
                let uid = region.page_uid;
                for &i in idxs {
                    let tile = &self.tiles[i];
                    if let Some(lv) = local_view {
                        if !tile.aabb_local().intersects(&lv) {
                            continue;
                        }
                    }
                    match self.scratch_pages.iter_mut().find(|(u, _)| *u == uid) {
                        Some((_, v)) => v.push((i, region)),
                        None => self.scratch_pages.push((uid, vec![(i, region)])),
                    }
                }
            }
        }

        for (uid, idxs) in &self.scratch_pages {
            let Some(tex) = TEXTURES.get(*uid) else { continue };
            let inv = Vec2::new(1.0 / tex.width as f32, 1.0 / tex.height as f32);
            for &(i, region) in idxs {
                let tile = &self.tiles[i];
                let rect = SpriteRect::from_texture_px(
                    tile.pos,
                    tile.size,
                    Vec2::new(region.tl_px.0 as f32, region.tl_px.1 as f32),
                    Vec2::new(region.wh_px.0 as f32, region.wh_px.1 as f32),
                    inv,
                );
                r2d.add_sprite2d(
                    rect,
                    tile.color,
                    map_t,
                    Layer::from(base + tile.layer as f64),
                    &tex,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(region_id: u64, page_uid: u64, x: f32, y: f32, w: f32, h: f32) -> Tile {
        Tile::new(RegionRef::from_parts(region_id, page_uid), Vec2::new(x, y), Vec2::new(w, h))
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
        // 不同 chunk 的 tile 分开
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
        // 整体平移（缓存按脏标记重建）
        m.set_transform(Transform2D::IDENTITY.with_pos(Vec2::new(100.0, 50.0)));
        let first = m.solid_rects()[0];
        assert_eq!(first, Rect::new(100.0, 50.0, 10.0, 10.0), "平移后世界 AABB");
        // 结构未变 → 缓存复用（结果一致且不重建：两次调用值相同）
        assert_eq!(m.solid_rects()[0], first, "未置脏应复用缓存");
        // 整体旋转 45° → 保守 AABB 包含变换后的角点
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
        assert_eq!(m.visible_count(None), 2);
    }
}
