# rjw_render

中文：
`rjw_render` 是基于 wgpu 30 的底层渲染上下文，管理 surface / device / queue / swapchain，并提供纹理包装、静态网格与线程安全全局注册表。

English：
`rjw_render` is the low-level wgpu 30 based render context that manages the surface, device, queue and swapchain, plus texture wrappers, static meshes and thread-safe global registries.

---

## 功能特性 / Features

中文：
- `RenderContext`：surface / device / queue / swapchain 生命周期管理；`begin_frame()` / `end_frame()` / `resize()`。
- `RenderConfig`：后端选择（默认 Windows 下 DX12 | GL）、垂直同步、表面格式。
- `TextureWrapped` / `ArcTextureWrapped`：RGBA8 纹理包装，内置 `width` / `height` / `uid`，与采样器解耦。
- `TypedRegistry` + `TEXTURES` / `MESHES`：线程安全全局注册表（`register` / `get` / `remove` / `rename` …）。
- `MeshData`：静态网格（GPU 顶点 / 索引缓冲 + uid）。
- `pub use wgpu;`：重导出 wgpu，避免版本不一致。

English：
- `RenderContext`: manages the surface, device, queue and swapchain lifecycle; `begin_frame()` / `end_frame()` / `resize()`.
- `RenderConfig`: backend selection (default DX12 | GL on Windows), vsync and surface format.
- `TextureWrapped` / `ArcTextureWrapped`: RGBA8 texture wrapper with built-in `width` / `height` / `uid`, decoupled from samplers.
- `TypedRegistry` + `TEXTURES` / `MESHES`: thread-safe global registries (`register` / `get` / `remove` / `rename` …).
- `MeshData`: static mesh (GPU vertex/index buffers + uid).
- `pub use wgpu;`: re-exports wgpu to avoid version mismatches.

---

## 示例代码 / Example

```rust
use rjw_render::{RenderContext, RenderConfig};

let mut render = RenderContext::new(&window, &RenderConfig::default());
if let Some((surface_texture, view)) = render.begin_frame() {
    // ... 编码绘制命令到 encoder ...
    render.end_frame(surface_texture, encoder);
}
render.resize(width, height);
```

---

## 许可 / License

MIT © 2026 KrisuRJW
