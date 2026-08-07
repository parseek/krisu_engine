# rjw_keystate
# rjw_keystate

一个轻量级、无依赖的 Rust 库，用于表示键盘和鼠标按键状态，并提供详细的边缘检测。
A minimal, no‑dependency Rust library for representing keyboard and mouse button states with detailed edge detection.

---

## 特性
## Features

- **按下 / 释放** – 当前物理状态。
- **Pressed / Released** – current physical state.
- **边缘** – 当前帧内发生的状态转换（由操作系统提供）。
- **Edge** – a state transition occurred in the current frame (provided by OS).
- **真边缘** – 非合成重复事件的可靠边缘（适用于文本输入或游戏动作）。
- **True Edge** – a reliable edge that is not a synthetic repeat event (useful for text input or game actions).
- **突然释放** – 用于处理竞态条件的特殊标志，标记一个之前按住的键被释放。
- **Sudden Up** – a special flag for releasing a key that was previously held, to handle race conditions.
- 提供 `up_edge`、`down_edge`、`up_true_edge`、`down_true_edge` 和 `sudden_up` 等查询方法。
- Utility methods to query `up_edge`, `down_edge`, `up_true_edge`, `down_true_edge`, and `sudden_up`.
- `off_edge()` 和 `set_sudden_up()` 用于每帧修改状态。
- `off_edge()` and `set_sudden_up()` to modify the state per frame.

---

## 示例
## Example

```rust
use rjw_keystate::*;

let mut state = KEY_STATE_RELEASED;
assert!(!state.pressed());

state = KEY_STATE_DOWN_TRUE_EDGE;
assert!(state.pressed());
assert!(state.edge());
assert!(state.true_edge());
assert!(state.down_true_edge());

// 处理完一帧后，清除边缘标志
// After processing a frame, clear the edge flags
state = state.off_edge();
assert!(state.pressed());
assert!(!state.edge());
assert!(!state.true_edge());
```