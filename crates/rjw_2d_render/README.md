# rjw_2d_render

中文：
`rjw_2d_render` 是 krusie 引擎的 2D 批渲染器：Sprite / Mesh 统一管线，按 (layer, states) 排序合批。

English：
`rjw_2d_render` is the 2D batched renderer of the krusie engine: a unified Sprite/Mesh pipeline batched by (layer, states).

> 整套库入口：`use rjw_krusie::prelude::*;`（聚合 + 运行时，见 [`crates/rjw_krusie`](../rjw_krusie/README.md)）；本 crate 也可单独使用。

---

## 模块划分 / Modules

| 模块 | 职责 |
|---|---|
| `draw` | 录制：`Draw2D<K>` 统一 Builder（Sprite / Mesh / StaticMesh / Custom）+ 流式 sink |
| `sort` | 排序：对**命令索引数组**重排（`SortMode` / `SortPolicy` / `SortKey`，纯函数可单测） |
| `cull` | 剔除：可见性判定 + 对**索引数组**过滤（`Cull` / `Culler` + 纯几何，纯函数可单测） |
| `render2d` | 门面：`Render2D`（录制入口 / 全局状态 / 提交） |
| `debug_draw` | 调试图元：`DebugStyle` + `DebugPainter`（样式对象 + ≤2 参方法） |

## 功能特性 / Features

- **`Render2D` 不含相机 / 不含 VP**：相机由调用方持有（`Camera2D { region, transform }`），
  提交时 `submit(&mut pass, &cam)` 把画面矩形写回相机并读取 VP。
- **提交只有两个入口**：`submit(&mut PassBuilder, &Camera2D)`（一个画面 = 一个 pass = 一个 VP 槽）
  与 `render(&mut RenderContext, clear)`（单画面一行糖）。深度附件需求由 `PassBuilder` 自动推导。
- 录制入口只有两类形态：**数据版** `sprite/solid/region/quads/mesh/polygon/static_mesh/custom`，
  **流式版** `*_with`（零临时 `Vec`）。
- 图集直达：`Render2D::region(AtlasSprite)`（`atlas.sprite(&handle)` 的产物）——一处解析，不再手算像素 UV。
- 装饰一律走链：`.layer(..) .tint(..) .transform(..)/.at/.rot/.scale .matrix(mat) .texture(..) .states(RStates)`；
  状态糖（`.blend` / `.sampler` / `.cull` / `.raster` / `.depth` / `.stencil`）收**对象/枚举**，不收裸 bool。
- `RStates`：Blend / Sampler / Cull+Raster / Depth / Stencil 位域（u64）；全局默认走 `states_mut(RStates)`。
- 排序：`sort(SortMode)` / `sort_custom(Box<dyn SortPolicy>)`；剔除：`cull(Cull::Off | Viewport | Rect | Fn)`（`Cull::from(&cam)`）。
- 调试图元：`r2d.debug(DebugStyle::new(Color::RED).width(2.0)).line(a, b)`。
- 句柄类型化：`static_mesh(MeshId, &tex)`；资源创建走 `Gpu::{texture, mesh}`（`rjw_render`）。

## 示例代码 / Example

```rust
use rjw_2d_render::{Clear, Color, Render2D, SpriteRect};

// 取帧 → 一个画面 = 一次 pass（相机由调用方持有）
let Some(mut frame) = ctx.acquire_frame() else { return };

// 精灵：入口 2 参，其余全在链上（默认 IDENTITY / WHITE / layer 0）
r2d.sprite(SpriteRect::new((-50.0, -50.0), (100.0, 100.0)), &tex)
    .tint(Color::WHITE)
    .at(pos)
    .layer(1.0);

// 纯色多边形（流式构造，零临时 Vec）
r2d.polygon_with(|p| { p.vertex(a); p.vertex(b); p.vertex(c); })
    .tint(Color::CYAN);

// 图集直达（region + 页纹理一次拿到）
if let Some(spr) = atlas.sprite(&handle) {
    r2d.region(spr).layer(2.0);
}

// 全局默认状态（唯一状态语言）
r2d.states_mut(rjw_2d_render::RStates::new().blend(rjw_2d_render::BlendMode::Additive));
r2d.sort(rjw_2d_render::SortMode::LayerOnly);
r2d.cull(rjw_2d_render::Cull::Viewport);

// 提交本画面（写回 cam.region + 取 VP + 开 pass）
let mut pass = frame.pass(Clear::default());
r2d.submit(&mut pass, &cam);
drop(pass);
frame.present();
```

---

## 许可 / License

MIT © 2026 KrisuRJW
