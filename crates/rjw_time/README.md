# rjw_time

中文：
`rjw_time` 提供帧间隔时间（`DeltaTime`）与平滑 FPS 计时器（`DeltaTimer`）。

English：
`rjw_time` provides frame delta time (`DeltaTime`) and a smoothed FPS timer (`DeltaTimer`).

---

## 功能特性 / Features

中文：
- `DeltaTime`：同时缓存 `Duration` / `f32` / `f64` 三种间隔表示，`get_f32()` / `get_f64()`。
- `DeltaTimer`：`per_frame()` 计算帧间隔并推进时间戳；`get_fps()` 返回 EMA 平滑 FPS（α = 0.1）。
- `DT_MAX`：帧间隔上限常量（默认 0.1 秒），防止大卡顿把 `dt` 拉爆。

English：
- `DeltaTime`: caches the interval as `Duration` / `f32` / `f64`, `get_f32()` / `get_f64()`.
- `DeltaTimer`: `per_frame()` computes the frame delta and advances the stamp; `get_fps()` returns an EMA-smoothed FPS (α = 0.1).
- `DT_MAX`: upper clamp constant for the frame delta (default 0.1s) to guard against hitches.

---

## 示例代码 / Example

```rust
use rjw_time::DeltaTimer;

let mut timer = DeltaTimer::default();
// 每帧开头调用
timer.per_frame();
let dt = timer.dt().get_f32();
let fps = timer.get_fps();
```

---

## 许可 / License

MIT © 2026 KrisuRJW
