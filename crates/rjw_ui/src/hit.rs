//! 命中测试与交互状态机（纯函数，无 GPU 依赖，可单测）。
//!
//! 状态机语义：
//! - `pressed`：按下（`down_edge` 且命中）后持续到释放；
//! - `clicked`：本帧**按下 + 释放均在本体内**；
//! - `released`：本帧释放（不论释放位置）；
//! - 拖出本体后仍保持 `pressed`（释放时若已拖出则不算 clicked，符合常规 UI 直觉）。

use glam::Vec2;
use rjw_keystate::KeyState;
use rjw_transform::Rect;

use crate::state::WidgetState;

/// 屏幕矩形命中测试（含边界）。
#[inline]
pub fn hit_test(rect: &Rect, mouse: Vec2) -> bool {
    rect.contains_point(mouse)
}

/// **窗口遮挡判定**（点击穿透修复）：是否存在 `z' > z` 的窗口矩形包含 `mouse`。
///
/// - `z = 0`：非窗口内容（面板 / 顶层控件，绘制在所有窗口之下）——被**任意**窗口
///   （`z' >= 1`）遮挡；
/// - `z >= 1`：只被**更高 z** 的窗口遮挡（自身窗口不遮挡自己）。
///
/// 遮挡区域内的控件不得响应点击 / 悬停——只有鼠标下**最上层**的窗口可交互，
/// 背后窗口的控件在重叠区域不会误触发（点击穿透）。
#[inline]
pub fn window_occluded(
    z: u32,
    mouse: Vec2,
    mut windows: impl Iterator<Item = (u32, Rect)>,
) -> bool {
    windows.any(|(wz, r)| wz > z && r.contains_point(mouse))
}

/// 一次交互帧产生的事件（返回给控件，再映射为用户可见状态）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InteractEvents {
    pub pressed: bool,
    pub clicked: bool,
    pub released: bool,
}

/// 更新按钮 / 勾选框 / 单选 / 输入框的交互状态机。
///
/// - `hit`：本帧鼠标是否在本体（已考虑窗口内外）；
/// - `btn`：鼠标主键的 `KeyState`（本帧边沿 + 当前按下）。
///
/// 返回本帧事件；`ws` 被原地更新（跨帧保持 `pressed` 等）。
#[inline]
pub fn update_interact(ws: &mut WidgetState, hit: bool, btn: KeyState) -> InteractEvents {
    let mut ev = InteractEvents::default();
    if btn.down_edge() && hit {
        ws.pressed = true;
        ev.pressed = true;
    }
    if ws.pressed && !btn.pressed() {
        // 本帧任意时刻释放
        ws.pressed = false;
        ev.released = true;
        if hit {
            ws.clicked = true;
            ev.clicked = true;
        }
    }
    ws.hovered = hit;
    ev
}

/// 滑块拖拽状态机：按下且命中 → 开始拖拽；释放 → 结束拖拽。
/// 返回 `active`（本帧应跟随鼠标更新值）。
#[inline]
pub fn update_drag(ws: &mut WidgetState, hit: bool, btn: KeyState) -> bool {
    if ws.dragging && !btn.pressed() {
        ws.dragging = false;
    }
    if btn.down_edge() && hit {
        ws.dragging = true;
    }
    ws.dragging
}

/// 值归一化：把 `mx` 映射到 `rect` 横向的 [0,1]（clamp）。
#[inline]
pub fn normalize_x(rect: &Rect, mx: f32) -> f32 {
    if rect.w <= f32::EPSILON {
        return 0.0;
    }
    ((mx - rect.x) / rect.w).clamp(0.0, 1.0)
}

/// 帧末清除一次性边沿（clicked 等），由 `Ui::finish` 调用。
#[inline]
pub fn clear_frame_flags(ws: &mut WidgetState) {
    ws.clicked = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rjw_keystate::{
        KEY_STATE_DOWN_EDGE, KEY_STATE_PRESSING, KEY_STATE_RELEASED, KEY_STATE_UP_EDGE,
    };

    const RECT: Rect = Rect::new(10.0, 20.0, 100.0, 40.0);

    /// 用公开常量构造测试用 `KeyState`（pressed/edge 两维）。
    fn btn_state(pressed: bool, edge: bool) -> KeyState {
        match (pressed, edge) {
            (true, true) => KEY_STATE_DOWN_EDGE,
            (true, false) => KEY_STATE_PRESSING,
            (false, true) => KEY_STATE_UP_EDGE,
            (false, false) => KEY_STATE_RELEASED,
        }
    }

    #[test]
    fn hit_test_bounds_inclusive() {
        assert!(hit_test(&RECT, glam::Vec2::new(10.0, 20.0)), "左上角含边界");
        assert!(hit_test(&RECT, glam::Vec2::new(110.0, 60.0)), "右下角含边界");
        assert!(!hit_test(&RECT, glam::Vec2::new(9.0, 20.0)), "左外");
        assert!(!hit_test(&RECT, glam::Vec2::new(50.0, 61.0)), "下外");
    }

    #[test]
    fn window_occluded_blocks_behind_windows_only() {
        // 两个重叠窗口：B(z=1) 在 (0,0,100,100)，A(z=2) 在 (50,50,100,100) 覆盖其右下。
        let rects = [
            (1u32, Rect::new(0.0, 0.0, 100.0, 100.0)),
            (2u32, Rect::new(50.0, 50.0, 100.0, 100.0)),
        ];
        // 顶层窗口（z=2）：不被任何窗口遮挡（没有更高 z）
        assert!(!window_occluded(2, Vec2::new(60.0, 60.0), rects.iter().copied()));
        // 背后窗口（z=1）在重叠区域：被 z=2 遮挡 → 不得响应（点击穿透修复）
        assert!(window_occluded(1, Vec2::new(60.0, 60.0), rects.iter().copied()));
        // 背后窗口在非重叠区域：可见可交互
        assert!(!window_occluded(1, Vec2::new(10.0, 10.0), rects.iter().copied()));
        // 非窗口内容（z=0）：被任意窗口遮挡
        assert!(window_occluded(0, Vec2::new(10.0, 10.0), rects.iter().copied()));
        // 鼠标在窗口外：不遮挡
        assert!(!window_occluded(1, Vec2::new(200.0, 200.0), rects.iter().copied()));
        assert!(!window_occluded(0, Vec2::new(200.0, 200.0), rects.iter().copied()));
        // 更高 z 的窗口也不遮挡自己
        assert!(!window_occluded(3, Vec2::new(60.0, 60.0), rects.iter().copied()));
        // 无任何窗口：恒不遮挡
        assert!(!window_occluded(0, Vec2::new(10.0, 10.0), std::iter::empty()));
        // 单窗口：自身不遮挡，但遮挡 z=0 内容
        assert!(!window_occluded(5, Vec2::new(50.0, 40.0), [(5u32, RECT)].into_iter()));
        assert!(window_occluded(0, Vec2::new(50.0, 40.0), [(5u32, RECT)].into_iter()));
    }

    #[test]
    fn press_inside_release_inside_clicked() {
        let mut ws = WidgetState::default();
        // 按下（down_edge + hit）
        let ev = update_interact(&mut ws, true, btn_state(true, true));
        assert!(ev.pressed && !ev.clicked);
        assert!(ws.pressed && ws.hovered);
        // 持续按住（无 edge）
        let ev = update_interact(&mut ws, true, btn_state(true, false));
        assert!(!ev.pressed && !ev.clicked && !ev.released);
        // 释放（up_edge + hit）
        let ev = update_interact(&mut ws, true, btn_state(false, true));
        assert!(ev.released && ev.clicked, "按下+释放均在体内应 clicked");
        assert!(!ws.pressed);
        assert!(ws.clicked, "ws.clicked 应置位（finish 时清除）");
    }

    #[test]
    fn drag_out_then_release_not_clicked() {
        let mut ws = WidgetState::default();
        update_interact(&mut ws, true, btn_state(true, true));
        // 拖出本体
        let ev = update_interact(&mut ws, false, btn_state(true, false));
        assert!(!ev.clicked && !ev.released);
        assert!(ws.pressed, "拖出仍保持 pressed");
        // 在体外释放
        let ev = update_interact(&mut ws, false, btn_state(false, true));
        assert!(ev.released && !ev.clicked, "体外释放不算 clicked");
    }

    #[test]
    fn press_outside_ignored() {
        let mut ws = WidgetState::default();
        let ev = update_interact(&mut ws, false, btn_state(true, true));
        assert!(!ev.pressed && !ws.pressed, "体外按下不进入 pressed");
    }

    #[test]
    fn hover_tracks_mouse() {
        let mut ws = WidgetState::default();
        update_interact(&mut ws, true, btn_state(false, false));
        assert!(ws.hovered);
        update_interact(&mut ws, false, btn_state(false, false));
        assert!(!ws.hovered);
    }

    #[test]
    fn normalize_x_maps_and_clamps() {
        let r = Rect::new(100.0, 0.0, 200.0, 10.0);
        assert!((normalize_x(&r, 100.0) - 0.0).abs() < 1e-5);
        assert!((normalize_x(&r, 200.0) - 0.5).abs() < 1e-5);
        assert!((normalize_x(&r, 300.0) - 1.0).abs() < 1e-5);
        assert!((normalize_x(&r, 0.0) - 0.0).abs() < 1e-5, "越界 clamp 到 0");
        assert!((normalize_x(&r, 999.0) - 1.0).abs() < 1e-5, "越界 clamp 到 1");
    }

    #[test]
    fn slider_drag_lifecycle() {
        let mut ws = WidgetState::default();
        assert!(!update_drag(&mut ws, true, btn_state(false, false)));
        assert!(update_drag(&mut ws, true, btn_state(true, true)), "按下且命中开始拖拽");
        assert!(update_drag(&mut ws, false, btn_state(true, false)), "拖拽中移出仍 active");
        assert!(!update_drag(&mut ws, false, btn_state(false, true)), "释放结束拖拽");
    }

    #[test]
    fn clear_frame_flags_resets_clicked() {
        let mut ws = WidgetState::default();
        update_interact(&mut ws, true, btn_state(true, true));
        update_interact(&mut ws, true, btn_state(false, true));
        assert!(ws.clicked);
        clear_frame_flags(&mut ws);
        assert!(!ws.clicked);
        assert!(ws.hovered, "clear 只清一次性边沿，保留 hover");
    }
}
