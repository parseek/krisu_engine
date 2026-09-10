# rjw_krusie

中文：
`rjw_krusie` 是 rjw_krusie 引擎的**统一入口**（聚合 crate，不含实现）：**一行 prelude 起步**，低层与命名冲突走命名空间。

English：
`rjw_krusie` is the unified entry point of the krusie engine (a thin aggregator crate, no implementation):
a **one-line prelude** for the happy path, with per-crate namespaces for lower-level or name-conflicting items.

---

## 用法 / Usage

```rust
use rjw_krusie::prelude::*;

struct App2D {
    render: Option<RenderContext>,
    r2d: Option<Render2D>,
    cam: Camera2D,
}

impl App for App2D {
    fn primary_window_attrib(&self) -> WindowAttributes {
        WindowAttributes::default().with_inner_size(LogicalSize::new(1280.0, 720.0))
    }
    // on_init: RenderContext::new(window, &RenderConfig::default()) → Render2D::new(&render)
}

fn main() -> Result<(), EventLoopError> {
    run_app(App2D { render: None, r2d: None, cam: Camera2D::new(Vec2::splat(1280.0)) })
}
```

低层 / 冲突名按需取命名空间：

```rust
use rjw_krusie::atlas::RegionRef;                 // 低层句柄
use rjw_krusie::collision::move_and_collide;      // 自由函数
use rjw_krusie::render2d::{CustomDraw, VertexP3U2C4};
use rjw_krusie::ui::Window;                       // UI 容器（prelude 里的 Window 是 winit 的）
use rjw_krusie::transform::LogicalSize;           // 引擎 DIP 尺寸（与 winit 同名）
```

## 两层结构 / Two layers

| 层 | 内容 |
|---|---|
| `rjw_krusie::prelude` | 应用骨架（winit / 输入 / 计时）、`RenderContext` + `wgpu` + `glam`、`Render2D` + `SpriteRect` + `Color` + `RStates` + 排序/剔除、相机与几何、图集、文本、UI 控件、瓦片 |
| 命名空间 | `main` / `gpu` / `render2d` / `transform` / `color` / `atlas` / `text` / `ui` / `tilemap` / `collision`（同时保留原 crate 名，如 `rjw_krusie::rjw_ui::Ui`） |

命名冲突约定：prelude **不含** `Window` / `Size` / `LogicalSize` 等（winit 与 `rjw_transform` / `rjw_ui` 同名）——
需要时写命名空间路径，**不做 `as` 改名**（同一类型不出现两个名字）。

## 裁剪依赖 / Feature flags

```toml
# 只做 2D 绘制（不装 UI / 文本 / 图集 / 瓦片）
rjw_krusie = { version = "0.2", default-features = false }
```
`default = ["atlas", "text", "ui", "tilemap", "collision"]`

---

## 许可 / License

MIT © 2026 KrisuRJW
