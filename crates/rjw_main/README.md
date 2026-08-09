# rjw_main

中文：
`rjw_main` 是应用入口：基于 winit `ApplicationHandler` 的事件循环，统一管理主窗口、计时器、键盘与鼠标，并在每帧调用用户实现的 `App` trait。

English：
`rjw_main` is the app entry point: a winit `ApplicationHandler` event loop that manages the primary window, timer, keyboard and mouse, calling into the user `App` trait every frame.

---

## 功能特性 / Features

中文：
- `App` trait：`on_init` / `about_to_wait` / `on_resized` / `primary_window_attrib`。
- `MainContext`：`timer`（`DeltaTimer`）、`keyboard`（`KeyboardInput`）、`mouse`（`MouseInput`）、`primary_window()`、`request_exit()`。
- `run_app(app)`：一行启动；`ControlFlow::Poll` 循环，帧末自动清理输入边沿（`end_frame()`）。
- 重导出 `winit` 常用类型与 `KeyCode` / `MouseButton` 等。

English：
- `App` trait: `on_init` / `about_to_wait` / `on_resized` / `primary_window_attrib`.
- `MainContext`: `timer` (`DeltaTimer`), `keyboard` (`KeyboardInput`), `mouse` (`MouseInput`), `primary_window()`, `request_exit()`.
- `run_app(app)`: one-line startup; `ControlFlow::Poll` loop with automatic input edge cleanup (`end_frame()`).
- Re-exports common `winit` types plus `KeyCode` / `MouseButton` etc.

---

## 示例代码 / Example

```rust
use rjw_main::*;

struct MyApp;

impl App for MyApp {
    fn on_init(&mut self, ctx: &mut MainContext) {
        // 创建 RenderContext / Render2D / 资源...
    }
    fn about_to_wait(&mut self, ctx: &mut MainContext) {
        if ctx.keyboard.get(KeyCode::Escape).down_edge() {
            ctx.request_exit();
        }
        let dt = ctx.timer.dt().get_f32();
        // ... 更新逻辑 + 渲染 ...
    }
}

fn main() -> Result<(), EventLoopError> {
    run_app(MyApp)
}
```

---

## 许可 / License

MIT © 2026 KrisuRJW
