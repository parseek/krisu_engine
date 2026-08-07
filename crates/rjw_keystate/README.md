# rjw_keystate

中文：  
`rjw_keystate` 是一个轻量级、无依赖的 Rust 库，用于表示键盘或鼠标按键的状态，并提供详细的边沿检测功能。

English:  
`rjw_keystate` is a minimal, no‑dependency Rust library for representing keyboard and mouse button states with detailed edge detection.

---

## 功能特性 / Features

中文：
- **按下 / 释放** – 当前的物理状态。
- **边沿（Edge）** – 由操作系统提供的、在当前帧内发生的状态转变。
- **真边沿（True Edge）** – 非合成重复事件的可靠边沿（适用于文本输入或游戏动作）。
- **突然释放（Sudden Up）** – 特殊标记，用于处理“同一帧内按下后又松开”的竞态条件。
- 工具方法：`up_edge`、`down_edge`、`up_true_edge`、`down_true_edge`、`sudden_up`。
- `off_edge()` 和 `set_sudden_up()` 用于每帧修改状态。

English：
- **Pressed / Released** – current physical state.
- **Edge** – a state transition occurred in the current frame (provided by OS).
- **True Edge** – a reliable edge that is not a synthetic repeat event (useful for text input or game actions).
- **Sudden Up** – a special flag for releasing a key that was previously held, to handle race conditions.
- Utility methods to query `up_edge`, `down_edge`, `up_true_edge`, `down_true_edge`, and `sudden_up`.
- `off_edge()` and `set_sudden_up()` to modify the state per frame.

---

## 示例代码 / Example

```rust
use rjw_keystate::*;

let mut state = KEY_STATE_RELEASED;
assert!(!state.pressed());

state = KEY_STATE_DOWN_TRUE_EDGE;
assert!(state.pressed());
assert!(state.edge());
assert!(state.true_edge());
assert!(state.down_true_edge());

// 帧结束后清除边沿标记
// After processing a frame, clear the edge flags
state = state.off_edge();
assert!(state.pressed());
assert!(!state.edge());
assert!(!state.true_edge());
```

---

## 关于 `SuddenUp` 的说明 / About `SuddenUp`

中文：  
`SuddenUp` 标记用于处理一种特殊情况：在同一个输入帧内，操作系统先报告了按下事件，随后又报告了释放事件（例如极快速的点击）。  
此时状态机可能会错误地认为按键仍处于按下状态，因为释放事件被当作“真边沿”处理。`set_sudden_up()` 会为当前状态附加一个标记，在下一帧的 `off_edge()` 中，该标记会被转换为 `KEY_STATE_UP_TRUE_EDGE`，从而正确表示按键已经释放。  
这种机制能避免在快速点击时出现“卡键”现象。

English：  
The `SuddenUp` flag addresses a specific race condition: within a single frame, the OS may report a press and then a release (e.g., a very fast tap).  
The state machine might incorrectly treat the key as still pressed, because the release event is processed as a "true edge". `set_sudden_up()` attaches a marker to the current state; in the next frame's `off_edge()`, this marker is translated into `KEY_STATE_UP_TRUE_EDGE`, correctly indicating that the key has been released.  
This prevents “stuck key” issues during rapid tapping.

---

## 许可 / License

MIT © 2026 KrisuRJW