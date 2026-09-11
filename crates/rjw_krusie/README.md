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

#[derive(Default)]
struct Game {
    cam: Camera2D,
}

impl App for Game {
    fn config(&self) -> AppConfig {
        AppConfig::new("my app").size(1280.0, 720.0)
    }

    fn update(&mut self, ctx: &mut Ctx) {
        // 无帧（最小化 / 遮挡）时渲染代码一行都不执行；逻辑照常
        let Some(mut f) = ctx.frame() else { return };

        self.cam.set_region(f.region());
        f.draw()
            .solid(SpriteRect::new((-50.0, -50.0), (100.0, 100.0)))
            .tint(Color::GREEN)
            .layer(0.0);

        f.submit(&mut self.cam, Clear::color(Color::rgb(0.10, 0.11, 0.16)));
        // 不必手动 present：`Frame` 析构自动 present
    }
}

fn main() -> Result<(), EventLoopError> {
    run(Game::default())
}
```

> 窗口 / 渲染 / 清屏 / 后台策略都在 `App::config() -> AppConfig` 里描述；
> `App::init(&Gfx)` 建资源，`App::update(&mut Ctx)` 每帧必调。

低层 / 冲突名按需取命名空间：

```rust
use rjw_krusie::atlas::RegionRef;                 // 低层句柄
use rjw_krusie::collision::Aabb;                  // 碰撞体（Aabb::slide 扫掠滑动）
use rjw_krusie::render2d::{CustomDraw, VertexP3U2C4};
use rjw_krusie::ui::Window;                       // UI 容器（prelude 里的 Window 是 winit 的）
use rjw_krusie::transform::LogicalSize;           // 引擎 DIP 尺寸（与 winit 同名）
```

## 两层结构 / Two layers

| 层 | 内容 |
|---|---|
| `rjw_krusie::prelude` | 运行时（`App` / `AppConfig` / `run` / `Ctx` / `Frame` / `Gfx`）、`Render2D` + `SpriteRect` + `Color` + `RStates` + 排序/剔除、相机与几何、图集、文本、UI 控件、瓦片 |
| 命名空间 | `main` / `gpu` / `render2d` / `transform` / `color` / `atlas` / `text` / `ui` / `tilemap` / `collision`（同时保留原 crate 名，如 `rjw_krusie::rjw_ui::Ui`） |

命名冲突约定：prelude **不含** `Window` / `Size` / `LogicalSize` 等（winit 与 `rjw_transform` / `rjw_ui` 同名）——
需要时写命名空间路径，**不做 `as` 改名**（同一类型不出现两个名字）。

## 裁剪依赖 / Feature flags

```toml
# 只做 2D 绘制（不装 UI / 文本 / 图集 / 瓦片）
rjw_krusie = { version = "0.3", default-features = false }
```
`default = ["atlas", "text", "ui", "tilemap", "collision"]`

---

## 许可 / License

MIT © 2026 KrisuRJW
