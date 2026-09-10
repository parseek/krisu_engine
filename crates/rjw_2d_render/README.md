# rjw_2d_render

中文：
`rjw_2d_render` 是 Krisu 引擎的 2D 批渲染器：Sprite / Mesh 统一管线，按 (layer, states) 排序合批。
**v0.2.0 重新设计**：统一 `Draw2D` 责任链（少参数）、`RStates` 唯一状态语言、排序/剔除从渲染器分离为纯函数模块。

English：
`rjw_2d_render` is the 2D batched renderer of the Krisu engine: a unified Sprite/Mesh pipeline batched by (layer, states).
**v0.2.0 redesign**: one `Draw2D` builder chain (few parameters), `RStates` as the only state language, and sorting/culling split out of the renderer into pure-function modules.

> 整套库入口：`use rjw_krusie::prelude::*;`（聚合 crate，见 [`crates/rjw_krusie`](../rjw_krusie/README.md)）；本 crate 也可单独使用根导入。

---

## 模块划分 / Modules

| 模块 | 职责 |
|---|---|
| `draw` | 录制：`Draw2D<K>` 统一 Builder（Sprite / Mesh / StaticMesh / Custom）+ 流式 sink |
| `sort` | 排序：对**命令索引数组**重排（`SortMode` / `SortPolicy` / `SortKey`，纯函数可单测） |
| `cull` | 剔除：可见性判定 + 对**索引数组**过滤（`Cull` / `Culler` + 纯几何，纯函数可单测） |
| `render2d` | 门面：`Render2D`（录制入口 / 全局状态 / 提交 / 资源） |

## 功能特性 / Features

- `Render2D`：录制 → 排序 → 剔除 → 分页合批 → `render()` 全流程；`record()` 录进用户 pass；`encode()` 只产 `CommandBuffer`；`acquire_frame()` 取表面。
- 录制入口只有两类形态：**数据版** `sprite/solid/quads/mesh/polygon/static_mesh/custom`，**流式版** `*_with`（零临时 `Vec`）。
- 装饰一律走链：`.layer(..) .color(..) .transform(..)/.pos/.rot/.scale .model(mat) .texture(..) .states(RStates)`。
- `RStates`：Blend / Sampler / Cull+Raster / Depth / Stencil 位域（u64），三级控制（全局默认 `set_states` → 单条 `.states()` → 描述符 `*_state()`）。
- 排序：`set_sort_mode(SortMode)`，或 `set_sorter(Box<dyn SortPolicy>)` 注入自定义键。
- 剔除：`set_cull(Cull::Off | Viewport | Rect | Fn)`，单一状态无隐式联动；`Culler` 可被下游复用。
- Mesh / StaticMesh / Custom 逃逸舱口：`mesh_with` / `static_mesh` / `custom`。

## 示例代码 / Example

```rust
use rjw_2d_render::{ClearConfig, Color, Render2D, SpriteRect, SortMode};
use rjw_transform::Transform2D;

// 精灵：入口 2 参，其余全在链上（默认 IDENTITY / WHITE / layer 0）
r2d.set_mvp(cam.vp_matrix());
r2d.sprite(SpriteRect::new((-50.0, -50.0), (100.0, 100.0)), &tex)
    .color(Color::WHITE)
    .transform(Transform2D::IDENTITY.with_pos(pos))
    .layer(1.0);

// 纯色多边形（流式构造，零临时 Vec）
r2d.polygon_with(|p| {
    p.vertex(a);
    p.vertex(b);
    p.vertex(c);
})
.color(Color::CYAN);

// 全局默认状态（唯一状态入口）
r2d.set_states(rjw_2d_render::RStates::new().blend(rjw_2d_render::BlendMode::Additive));
// 排序 / 剔除
r2d.set_sort_mode(SortMode::LayerOnly);
r2d.set_cull(rjw_2d_render::Cull::Viewport);

r2d.render(&ClearConfig::default());
```

---

## 迁移要点 / Migration (0.1 → 0.2)

| 0.1 | 0.2 |
|---|---|
| `add_sprite2d(rect, color, tf, layer, &tex)` | `sprite(rect, &tex).color(color).transform(tf).layer(layer)` |
| `add_sprite2d_solid(rect, color, tf, layer)` | `solid(rect).color(color).transform(tf).layer(layer)` |
| `add_sprite2d_matrix(rect, color, mat, layer, &tex)` | `sprite(rect, &tex).color(color).model(mat).layer(layer)` |
| `add_mesh` / `add_mesh_transform` | `mesh(&verts, &tris).color(c).transform(tf)` |
| `add_mesh_fn` / `add_mesh_fn_prealloc` | `mesh_with(\|s\| ..)` / `mesh_with_cap(v, t, \|vs, ts\| ..)` |
| `add_polygon_fan` / `_fan_uv` / `_strip*` | `polygon(&verts)` / `polygon_uv(&verts, &uvs)` / `polygon_with(\|p\| ..)` |
| `add_quads` / `add_quads_styled` | `quads(&verts, &tex).transform(tf)` / `+ .color(tint)` |
| `add_static_mesh` / `_matrix` | `static_mesh(id, &tex).color(c).transform(tf)` / `+ .model(mat)` |
| `add_custom(layer, cd)` | `custom(cd).layer(layer)` |
| `default_blend(..).default_depth_test(..)` | `set_states(RStates::new().blend(..).depth_test(..))` |
| `set_sorting(bool)` / `set_layer_sort(bool)` | `set_sort_mode(SortMode::None / LayerOnly)` |
| `set_culling` / `set_cull_with` / `set_cull_camera` | `set_cull(Cull::Viewport / Fn(..) / from(&cam))` |
| `flush(pass)` / `render_command_buffer(..)` / `begin_frame()` | `record(pass)` / `encode(..)` / `acquire_frame()` |
| `tex_bind_group_layout()` | `texture_layout()` |
| `SpriteRect::from_texture(tl, wh)` | `SpriteRect::new(tl, wh)` |
| `SpriteRect::from_texture_px(.., inv_tex_wh)` | `SpriteRect::with_uv_px(.., tex_wh)` / `with_uv_tex(.., &tex)` |
| `SpriteRect::new(tl, wh, uv_tl, uv_wh)` | `SpriteRect::with_uv(tl, wh, uv_tl, uv_wh)` |
| `SpriteRect::shrink_mesh_x/y/(x,y)` | `SpriteRect::shrink(每边量)`（`f32` 或 `(x, y)`） |
| `SpriteRect::shrink_uv_x/y/(u,v)` | `SpriteRect::shrink_uv(每边量)` |
| `SpriteRectPx::from_texture / from_tex / from_tex_px / from_tex_wh_px` | `SpriteRectPx::new(tl, wh, tex_wh)` / `of_tex(.., &tex)` / `with_uv_px(..)` / `with_uv_tex(.., &tex)` |
| `SpriteRectPx::shrink_left/right/up/down`、`expand_*`、`exceed_*`、`shrink_uv_*`（16 个） | `shrink(Edges)` / `expand(Edges)` / `exceed(Edges)`（`Edges::new().left(8.0)`、`shrink(4.0)`、`shrink((4.0, 2.0))`） |

---

## 许可 / License

MIT © 2026 KrisuRJW

