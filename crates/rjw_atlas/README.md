# rjw_atlas

中文：
`rjw_atlas` 提供运行时动态图集（Guillotine 空闲矩形打包）与静态预排布图集，把多张精灵纹理合入一或数张大纹理页，使同页绘制天然满足合批条件。

English：
`rjw_atlas` provides a runtime dynamic atlas (Guillotine free-rect packing) and a static pre-arranged atlas, packing many sprites into one or more large texture pages so same-page draws batch naturally.

---

## 功能特性 / Features

中文：
- `DynamicAtlas<K = String>`：运行时插入 / 逐出 / 自动复活（tombstone）/ compact / 自动新建页；`K` 泛型键。
- **构造与插入收参数**：`DynamicAtlas::new(gfx, AtlasConfig)`（页尺寸进配置）；插入只有两个入口
  `insert(key, Rgba8)` 与 `insert_with(key, Rgba8, InsertOpts)`（原点 / `no_clamp` / `permanent` 走选项），
  动态源用 `insert_dynamic(key, size, SpriteSource)`。
- 打包器：`Guillotine` 空闲矩形列表（best-fit + 古莱丁切分），按行堆放，混合尺寸也不会碎片化到“页未满却开新页”。
- 去碎片重排：`compact()` 把带源条目全量重排到最少页并重传纹理；`generation()` 世代号供缓存区域者刷新。
- **区域失效世代号 `revision()`**：`generation()` 的**超集**——"搬动条目"**或**"让空闲槽位重新
  可分配（去碎片重建空闲矩形 ⇒ 后续插入可能复用已逐出条目的槽位）"都 +1。
  **缓存了 UV 的消费者必须把它并入缓存键**：只跟 `generation()` 会漏掉"逐出 + 槽位复用"，
  旧 UV 会采样到别的字形像素（表现为"陈旧文字 / 背景消失"）而缓存永不失效。
  另提供不刷新寿命的只读查询 `region_peek()`（校验缓存区域用）。
- 寿命管理：`region()` 刷新寿命，**`tick()`**（引擎每渲染帧调用）到期转墓碑，`region_or_revive()` 自动重插。
- `SpriteSource`：被逐出精灵可通过生成器按需重新光栅化（原 `TextureRegenerator`）。
- **绘制直达**：`atlas.sprite(&handle)` 产出 `AtlasSprite`（区域 + 页纹理），交给 `Render2D::region(..)` 一次提交。
- `AtlasStats`（`stats()`）：页数 / 空闲 / 碎片度 / 世代，内省用。
- `StaticAtlas<K = String>`：从 TOML（`spr.toml`）反序列化静态精灵表；泛型与 `DynamicAtlas` 一致。
- `Index` / `IndexMut`：`DynamicAtlas` 与 `StaticAtlas` 均支持 `atlas[&key]` 直接读写区域。
- TOML 导入 / 导出（feature `toml`，默认开启）。
- `clamp_margin`：纹理边缘扩张 1px，避免线性过滤出血（`InsertOpts::no_clamp()` 关闭）。

English：
- `DynamicAtlas<K = String>`: runtime insert / evict / auto-revive (tombstone) / compact / auto new page; generic key `K`.
- **Fewer parameters**: `DynamicAtlas::new(gfx, AtlasConfig)` (page size lives in the config); insertion is just
  `insert(key, Rgba8)` / `insert_with(key, Rgba8, InsertOpts)` (`origin` / `no_clamp` / `permanent` as options),
  with `insert_dynamic(key, size, SpriteSource)` for regenerable sources.
- Packer: `Guillotine` free-rect list (best-fit + guillotine split), row-based stacking.
- Defragmentation: `compact()` re-packs source-backed entries into the fewest pages; `generation()` bumps for cached-region holders.
- **`revision()`** (superset of `generation()`): bumps when entries move **or** freed slots become
  allocatable again (defrag rebuilds the free-rect lists, so a later insert may reuse an evicted
  entry's pixels). **Consumers that cache UVs must fold it into their cache key** — `generation()`
  alone misses evict-then-reuse, leaving stale UVs pointing at another glyph's pixels.
  `region_peek()` is a non-refreshing read-only query for cache validation.
- Lifetime: `region()` refreshes, **`tick()`** (called by the engine every rendered frame) tombstones expired entries, `region_or_revive()` re-inserts.
- **Draw-ready**: `atlas.sprite(&handle)` yields `AtlasSprite` (region + page texture) for `Render2D::region(..)`.
- `AtlasStats` via `stats()`; `Index`/`IndexMut`; TOML under feature `toml`.

---

## 示例代码 / Example

```rust
use rjw_atlas::{AtlasConfig, DynamicAtlas, InsertOpts};
use rjw_render::Rgba8;

// `gfx` = rjw_krusie::runtime::Gfx（或任何 `&Gpu`）
let mut atlas = DynamicAtlas::new(gfx, AtlasConfig { max_pages: 4, padding: 1, page_size: 1024, ..Default::default() });

// 插入（默认 clamp_margin、非常驻）；要常驻 / 指定原点 / 关边距用 InsertOpts
let region = atlas.insert("player".to_string(), Rgba8::new(&rgba, (64, 64))).unwrap();
let handle = atlas.handle("player").unwrap();                 // RAII 句柄（保活 + 重排后仍有效）
atlas.insert_with("ui_white".to_string(), Rgba8::new(&[255, 255, 255, 255], (1, 1)),
                  InsertOpts::new().permanent());

// 绘制：区域 + 页纹理一次拿到
if let Some(spr) = atlas.sprite(&handle) {
    r2d.region(spr).tint(Color::WHITE).layer(0.0);
}

atlas.tick();   // 引擎每渲染帧调用；用户侧手动驱动时自行调用
let _ = region;
```

---

## 许可 / License

MIT © 2026 KrisuRJW
