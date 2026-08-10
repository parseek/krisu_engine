# rjw_atlas

中文：
`rjw_atlas` 提供运行时动态图集（Guillotine 空闲矩形打包）与静态预排布图集，把多张精灵纹理合入一或数张大纹理页，使同页绘制天然满足合批条件。

English：
`rjw_atlas` provides a runtime dynamic atlas (Guillotine free-rect packing) and a static pre-arranged atlas, packing many sprites into one or more large texture pages so same-page draws batch naturally.

---

## 功能特性 / Features

中文：
- `DynamicAtlas<K = String>`：运行时插入 / 踢出 / 自动复活（tombstone）/ compact / 自动新建页；`K` 泛型键。
- 打包器：`Guillotine` 空闲矩形列表（best-fit + 古莱丁切分），按行堆放，混合尺寸也不会碎片化到“页未满却开新页”。
- 去碎片重排：`compact()` 把带源条目全量重排到最少页并重传纹理；`generation()` 世代号供缓存区域者刷新。
- 寿命管理：`get()` 刷新寿命，`end_frame()` 到期转墓碑，`get_or_revive()` 自动重插。
- `TextureRegenerator`：被踢出精灵可通过生成器按需重新光栅化。
- `StaticAtlas<K = String>`：从 TOML（`spr.toml`）反序列化静态精灵表；泛型与 `DynamicAtlas` 一致。
- `Index` / `IndexMut`：`DynamicAtlas` 与 `StaticAtlas` 均支持 `atlas[&key]` 直接读写区域。
- TOML 导入 / 导出（`serde` feature，默认开启）。
- `clamp_margin`：纹理边缘扩张 1px，避免线性过滤出血。

English：
- `DynamicAtlas<K = String>`: runtime insert / evict / auto-revive (tombstone) / compact / auto new page; generic key `K`.
- Packer: `Guillotine` free-rect list (best-fit + guillotine split), row-based stacking that avoids fragmenting into narrow columns.
- Defragmentation: `compact()` re-packs all source-backed entries into the fewest pages and re-uploads textures; `generation()` bumps for cached-region holders.
- Lifetime management: `get()` refreshes lifetime, `end_frame()` moves expired entries to tombstones, `get_or_revive()` re-inserts automatically.
- `TextureRegenerator`: evicted sprites can be re-rasterized on demand through a generator.
- `StaticAtlas<K = String>`: deserializes a static sprite sheet from TOML (`spr.toml`); generic like `DynamicAtlas`.
- `Index` / `IndexMut`: both `DynamicAtlas` and `StaticAtlas` support `atlas[&key]` to read/write regions directly.
- TOML import / export (`serde` feature, enabled by default).
- `clamp_margin`: expands texture edges by 1px to avoid linear-filter bleeding.

---

## 示例代码 / Example

```rust
use rjw_atlas::{AtlasConfig, DynamicAtlas};

let mut atlas = DynamicAtlas::new(device, queue, layout, AtlasConfig::default(), 2048);
if let Some(region) = atlas.insert_ex("player", &rgba, 64, 64) {
    // 绘制时使用 region.tl_px / region.wh_px / region.page_uid
    let r = atlas["player"]; // 等价 Index：&AtlasRegion
}
atlas.get_or_revive("player");
```

---

## 许可 / License

MIT © 2026 KrisuRJW
