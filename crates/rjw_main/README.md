# rjw_main

中文：
`rjw_main` 是**平台底座**：把 winit（窗口 / 事件循环类型）、输入（键盘 / 鼠标）与计时统一重导出，并提供默认窗口标题。
事件循环与窗口生命周期由 [`rjw_krusie`](https://crates.io/crates/rjw_krusie) 的 `runtime`（`Engine`，winit `ApplicationHandler` 实现）负责，
游戏侧只实现 `rjw_krusie::App`。

English：
`rjw_main` is the **platform base layer**: it re-exports winit (window / event-loop types), input (keyboard / mouse) and timing,
plus the default window title. The event loop and window lifecycle live in `rjw_krusie`'s `runtime`
(an `Engine` implementing winit's `ApplicationHandler`); games only implement `rjw_krusie::App`.

> 整套库入口：`use rjw_krusie::prelude::*;` 已含运行时骨架（`App` / `run` / `Ctx` / `Frame`）/ 输入 / 计时；
> 低层（`Window` / `Size` / `PRIMARY_WINDOW_TITLE`…）走 `rjw_krusie::main::*`。

---

## 功能特性 / Features

中文：
- 重导出 winit：`Window` / `WindowAttributes` / `EventLoop` / `ActiveEventLoop` / `WindowEvent` / `DeviceEvent` / `KeyCode` / `MouseButton` 与 `dpi` 类型。
- 重导出 `rjw_time::DeltaTimer`、`rjw_keyboard::KeyboardInput`（+ `KeyState`）、`rjw_mouse::MouseInput`（+ `ScrollDelta`）。
- `PRIMARY_WINDOW_TITLE`（`LazyLock<String>`，取当前可执行文件名）/ `PRIMARY_WINDOW_TITLE_DEFAULT`。

English：
- Re-exports winit: `Window` / `WindowAttributes` / `EventLoop` / `ActiveEventLoop` / `WindowEvent` / `DeviceEvent` / `KeyCode` / `MouseButton` and `dpi` types.
- Re-exports `rjw_time::DeltaTimer`, `rjw_keyboard::KeyboardInput` (+ `KeyState`), `rjw_mouse::MouseInput` (+ `ScrollDelta`).
- `PRIMARY_WINDOW_TITLE` (`LazyLock<String>`, from the executable name) / `PRIMARY_WINDOW_TITLE_DEFAULT`.

> **旧 API 已删除**（`docs/API_DESIGN.md` §8.9）：`rjw_main::{App, MainContext, MainHandler, run_app}`
> 与运行时的 `Engine` 职责重复且零调用。请改用 `rjw_krusie::{App, run, Ctx, Frame}`。

---

## 示例代码 / Example

```rust
// 游戏侧：只实现 rjw_krusie 的 App
use rjw_krusie::prelude::*;

struct MyApp;

impl App for MyApp {
    fn config(&self) -> AppConfig {
        AppConfig::new("my app").size(1280.0, 720.0)
    }

    fn update(&mut self, ctx: &mut Ctx) {
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }
        let Some(mut f) = ctx.frame() else { return };
        // ... 录制绘制 + f.submit(&mut cam, clear) ...
    }
}

fn main() -> Result<(), EventLoopError> {
    run(MyApp)
}
```

平台底座本身的用法（一般不必直接依赖）：

```rust
use rjw_main::{KeyboardInput, KeyCode, MouseInput, DeltaTimer};

let mut keys = KeyboardInput::default();
let mut mouse = MouseInput::default();
let mut timer = DeltaTimer::default();
let _ = keys.key(KeyCode::Space).down_edge();
let _ = mouse.pos_px();
timer.per_frame();
```

---

## 许可 / License

MIT © 2026 KrisuRJW
