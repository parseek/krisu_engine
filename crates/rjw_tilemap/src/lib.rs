//! 任意图集区域贴片（`rjw_tilemap`）。
//!
//! GameMaker 式矩形 tilemap 的**泛化**：瓦片不绑定固定网格/尺寸——任意
//! [`AtlasRegion`]（来自 `StaticAtlas` 或 **运行时插入的 `DynamicAtlas`**）都可
//! 以任意位置、任意尺寸（默认等于 region 像素尺寸）、任意颜色/变换贴到世界空间。
//! 固定 16/32 网格只是"等尺寸 + 网格坐标"的特例。
//!
//! 渲染复用 `rjw_2d_render::Render2D` 的实例化 sprite 路径：每个 tile = 一个
//! 四边形 + 图集 UV 子矩形；按 `page_uid` 分组（一张 mesh 只绑一张纹理页）；
//! `draw` 可传入视口世界矩形做 **AABB 剔除**（旋转 tile 用保守包围盒）。
//!
//! 碰撞：`Tile::solid` 标记 + [`TileMap::solid_rects`] 输出世界 AABB，
//! 配合 `rjw_collision::move_and_collide` 做玩家移动解析。

use glam::Vec2;
use rjw_atlas::AtlasRegion;
use rjw_color::Color;
use rjw_render::TEXTURES;
use rjw_transform::{Rect, Transform2D};
use rjw_2d_render::{Layer, Render2D, SpriteRect};

/// 一张贴片：图集区域 + 世界位置/尺寸 + 可选 tint/变换 + solid 标记。
#[derive(Debug, Clone, Copy)]
pub struct Tile {
    /// 图集区域（UV 子矩形 + 所属页）。
    pub region: AtlasRegion,
    /// 世界坐标左上角。
    pub pos: Vec2,
    /// 世界尺寸（默认 [`Tile::new`] 时取 region 像素尺寸）。
    pub size: Vec2,
    /// 着色（彩色内容如 emoji/带色纹理建议 `Color::WHITE` 原样输出）。
    pub color: Color,
    /// 每 tile 变换（翻转/旋转/缩放；`None` = 单位）。剔除用其保守 AABB。
    pub transform: Option<Transform2D>,
    /// 相对基础层的层级偏移。
    pub layer: f32,
    /// 是否参与碰撞（`solid_rects` 会收集）。
    pub solid: bool,
}

impl Tile {
    /// 以 region 像素尺寸为世界尺寸创建贴片（白色、单位变换、非 solid）。
    #[inline]
    pub fn new(region: AtlasRegion, pos: Vec2) -> Self {
        Self {
            region,
            pos,
            size: Vec2::new(region.wh_px.0 as f32, region.wh_px.1 as f32),
            color: Color::WHITE,
            transform: None,
            layer: 0.0,
            solid: false,
        }
    }

    /// 世界空间 AABB（保守：有变换时取变换后包围盒）。
    #[inline]
    pub fn aabb(&self) -> Rect {
        match self.transform {
            Some(t) => Rect::new(self.pos.x, self.pos.y, self.size.x, self.size.y).transform(&t),
            None => Rect::new(self.pos.x, self.pos.y, self.size.x, self.size.y),
        }
    }
}

/// 贴片集合（MVP：线性存储；大量静态 tile 可后续演进为空间网格/chunk + 脏标记实例缓存）。
#[derive(Debug, Default)]
pub struct TileMap {
    tiles: Vec<Tile>,
}

impl TileMap {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.tiles.clear();
    }

    #[inline]
    pub fn push(&mut self, tile: Tile) {
        self.tiles.push(tile);
    }

    #[inline]
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    #[inline]
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    /// 全部 solid 贴片的世界 AABB（供 `rjw_collision::move_and_collide` 等使用）。
    #[inline]
    pub fn solid_rects(&self) -> Vec<Rect> {
        self.tiles
            .iter()
            .filter(|t| t.solid)
            .map(|t| t.aabb())
            .collect()
    }

    /// 视口剔除下可见的贴片数（`None` = 全部）。调试/统计用。
    #[inline]
    pub fn visible_count(&self, viewport: Option<Rect>) -> usize {
        match viewport {
            None => self.tiles.len(),
            Some(vp) => self.tiles.iter().filter(|t| t.aabb().intersects(&vp)).count(),
        }
    }

    /// 渲染：`viewport` 为 `Some` 时按世界 AABB 剔除视口外贴片（默认 `None` 全量绘制）。
    ///
    /// 内部按 `page_uid` 分组（一个 mesh 只绑一张纹理页），组内逐 tile
    /// `add_sprite2d`（进入 Render2D 实例化批处理）。
    #[inline]
    pub fn draw(&self, r2d: &mut Render2D, base_layer: impl Into<Layer>, viewport: Option<Rect>) {
        if self.tiles.is_empty() {
            return;
        }
        let base: f64 = base_layer.into().as_f64();

        // 按图集页分组（MVP 线性扫描；剔除先行，不可见 tile 不分组）。
        let mut pages: Vec<(u64, Vec<usize>)> = Vec::new();
        for (i, t) in self.tiles.iter().enumerate() {
            if let Some(vp) = viewport {
                if !t.aabb().intersects(&vp) {
                    continue;
                }
            }
            match pages.iter_mut().find(|(uid, _)| *uid == t.region.page_uid) {
                Some((_, idxs)) => idxs.push(i),
                None => pages.push((t.region.page_uid, vec![i])),
            }
        }

        for (uid, idxs) in pages {
            let Some(tex) = TEXTURES.get(uid) else { continue };
            let inv = Vec2::new(1.0 / tex.width as f32, 1.0 / tex.height as f32);
            for &i in &idxs {
                let t = &self.tiles[i];
                let rect = SpriteRect::from_texture_px(
                    t.pos,
                    t.size,
                    Vec2::new(t.region.tl_px.0 as f32, t.region.tl_px.1 as f32),
                    Vec2::new(t.region.wh_px.0 as f32, t.region.wh_px.1 as f32),
                    inv,
                );
                r2d.add_sprite2d(
                    rect,
                    t.color,
                    t.transform.unwrap_or_default(),
                    Layer::from(base + t.layer as f64),
                    &tex,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(x: u32, y: u32, w: u32, h: u32) -> AtlasRegion {
        AtlasRegion { tl_px: (x, y), wh_px: (w, h), origin_px: (0, 0), page_uid: 1 }
    }

    #[test]
    fn tile_default_size_is_region_px() {
        let t = Tile::new(region(0, 0, 32, 64), Vec2::new(10.0, 20.0));
        assert_eq!(t.size, Vec2::new(32.0, 64.0));
        assert_eq!(t.aabb(), Rect::new(10.0, 20.0, 32.0, 64.0));
    }

    #[test]
    fn tile_aabb_is_conservative_with_transform() {
        let mut t = Tile::new(region(0, 0, 10, 10), Vec2::ZERO);
        t.transform = Some(Transform2D::IDENTITY.with_pos(Vec2::new(100.0, 0.0)).with_rot(0.785398));
        let a = t.aabb();
        // 变换后的角点应全在包围盒内
        let tr = t.transform.unwrap();
        for c in [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0), Vec2::new(10.0, 10.0)] {
            assert!(a.contains_point(tr.transform_point(c)), "角点 {c:?} 应在包围盒内");
        }
    }

    #[test]
    fn solid_rects_collects_only_solid() {
        let mut m = TileMap::new();
        m.push(Tile::new(region(0, 0, 10, 10), Vec2::ZERO));
        let mut solid = Tile::new(region(0, 0, 10, 10), Vec2::new(50.0, 0.0));
        solid.solid = true;
        m.push(solid);
        let rects = m.solid_rects();
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0], Rect::new(50.0, 0.0, 10.0, 10.0));
    }
}
