# rjw_2d_render

中文：
`rjw_2d_render` 是 Krisu 引擎的 2D 批渲染器：Sprite / Mesh 统一管线，按 (layer, states) 排序合批，内置 `RStates` 渲染状态位域、Builder 责任链、分页实例缓冲与统一管线缓存。

English：
`rjw_2d_render` is the 2D batched renderer of the Krisu engine: a unified Sprite/Mesh pipeline batched by (layer, states), with the `RStates` bitfield, builder chain, paged instance buffers and a unified pipeline cache.

---

## 功能特性 / Features

中文：
- `Render2D`：录制 → 排序合批 → `render()` 全流程提交；`flush()` 录制到用户自建 pass；`render_command_buffer()` 仅生成 `CommandBuffer`。
- `SpriteRect` / `SpriteRectPx`：归一化 / 像素 UV 精灵矩形，支持 shrink / expand / exceed 等裁剪特效。
- `RStates`：Blend / Sampler / Cull / Depth / Stencil 6 域 bitfield（u64），三级控制（全局默认 → 单条绘制 → 批量描述符）。
- Builder 责任链：`add_sprite2d(...).blend(Additive).depth_test(true)`；不链式 = 继承全局默认。
- Mesh：`add_mesh` / `add_polygon_fan/strip(±_uv)` / `add_mesh_fn(_prealloc)`，动态段合批。
- StaticMesh：注册到全局 `MESHES`，实例化合并绘制。
- `add_custom` / `CustomDraw`：注入原生 wgpu 绘制调用。

English：
- `Render2D`: record → sort & batch → `render()` full submission; `flush()` records into a user pass; `render_command_buffer()` produces a `CommandBuffer` only.
- `SpriteRect` / `SpriteRectPx`: normalized / pixel-space UV sprite rects with shrink / expand / exceed clipping effects.
- `RStates`: 6-domain bitfield (u64) for Blend / Sampler / Cull / Depth / Stencil with three-level control.
- Builder chain: `add_sprite2d(...).blend(Additive).depth_test(true)`; no chain = global default.
- Mesh: `add_mesh` / `add_polygon_fan/strip(±_uv)` / `add_mesh_fn(_prealloc)` with dynamic segment batching.
- StaticMesh: registered into global `MESHES`, drawn as instanced batches.
- `add_custom` / `CustomDraw`: inject native wgpu draw calls.

---

## 示例代码 / Example

```rust
use rjw_2d_render::{ClearConfig, Render2D, SpriteRect, Color};
use rjw_transform::{Transform2D, Vec2};

render2d.set_mvp(cam.vp_matrix());
render2d.add_sprite2d(
    SpriteRect::from_texture(Vec2::splat(-50.0), Vec2::splat(100.0)),
    Color::WHITE,
    Transform2D::default(),
    0.0,
    &tex,
);
render2d.render(&ClearConfig::default());
```

---

## 许可 / License

MIT © 2026 KrisuRJW
