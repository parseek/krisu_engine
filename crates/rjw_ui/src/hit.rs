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

/// 一处**可交互控件的命中区域**（控件级遮挡的登记项；逻辑像素、已含容器平移）。
///
/// 每帧由 [`crate::Ui::hit_abs`] 在**控件自身矩形命中鼠标时**登记，下一帧成为
/// [`widget_occluded`] 的判定输入——与窗口级遮挡（`UiState::window_rects`）同构：
/// **用已完成的一帧判定本帧**，才能在"后录制的控件画在上面"这个前提成立时
/// 一眼看出谁盖住谁（同一帧里后录制的控件还没录制，无法参与判定）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitRegion {
    /// **所属控件**（`IdAbsolute` 的 64 位哈希）：同一控件的多个区域（轨道 + 手柄、
    /// 输入框 + 手柄…）**互不遮挡**——否则滑块轨道会被自己的手柄挡掉。
    pub owner: u64,
    /// **绘制层级键**：同帧内单调递增 = 录制顺序 = 绘制先后（越大越在上）。
    pub key: u32,
    /// 命中矩形（逻辑像素，屏幕空间）。
    pub rect: Rect,
    /// 该区域的裁剪层（ScrollView 可视区 / Clip 沙箱）；鼠标在其外时**不算覆盖**
    /// （滚出可视区的控件不得挡住别人）。
    pub clip: Option<Rect>,
}

/// **控件级遮挡判定**（"重叠控件被一起触发"修复）：鼠标下是否存在**别的控件**、
/// **绘制层级更高**（`key` 更大 = 后录制 = 画在上面）且覆盖鼠标的命中区域。
///
/// `owner` / `key` 为被判定控件自身的身份与层级键；`regions` 是**上一帧**登记的全部
/// 可交互控件区域。被遮挡 ⇒ 该控件不得响应点击 / 悬停 / 拖拽。
///
/// 与 [`window_occluded`] 的分工：窗口级管**跨窗口**（更高 z 的窗口挡住背后窗口的
/// 控件），控件级管**同一窗口 / 面板内**（后录制的控件挡住先录制的）——两者都要过，
/// 重叠区域才只有**最上层那一个**控件响应。
#[inline]
pub fn widget_occluded(
    owner: u64,
    key: u32,
    mouse: Vec2,
    regions: impl Iterator<Item = HitRegion>,
) -> bool {
    let mut regions = regions;
    regions.any(|r| {
        r.owner != owner
            && r.key > key
            && r.rect.contains_point(mouse)
            && r.clip.is_none_or(|c| c.contains_point(mouse))
    })
}

/// **绝对 ID → 控件级遮挡用的身份哈希**（`IdAbsolute` 的 `Hash`）。
///
/// 只需"同 id 相等、不同 id 大概率不等"，64 位足够（撞了也只是两个控件互不遮挡，
/// 不会误判成遮挡——`!=` 分支才产生遮挡）。
#[inline]
pub fn id_hash(id: &crate::id::IdAbsolute<'_>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.as_str().hash(&mut h);
    h.finish()
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

/// 值归一化：把 `my` 映射到 `rect` 纵向的 [0,1]（clamp）——[`normalize_x`] 的镜像。
///
/// 竖向自定义拖拽区（如取色器的**色相条** / **SV 平面**）用：光靠 `normalize_x`
/// 只能表达一维取值，平面必须两个方向都能定位到参数。
#[inline]
pub fn normalize_y(rect: &Rect, my: f32) -> f32 {
    if rect.h <= f32::EPSILON {
        return 0.0;
    }
    ((my - rect.y) / rect.h).clamp(0.0, 1.0)
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
    fn widget_occluded_blocks_lower_widgets_only() {
        // 两个重叠控件：下层（key = 10）与上层（key = 20），重叠区 x ∈ [50,100]。
        let lower = HitRegion {
            owner: 1,
            key: 10,
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            clip: None,
        };
        let upper = HitRegion {
            owner: 2,
            key: 20,
            rect: Rect::new(50.0, 50.0, 100.0, 100.0),
            clip: None,
        };
        let inside = Vec2::new(60.0, 60.0);
        let outside = Vec2::new(10.0, 10.0);
        let regions = [upper];
        // 下层控件在重叠区：被上层遮挡 → 不响应
        assert!(widget_occluded(1, 10, inside, regions.iter().copied()));
        // 下层控件在非重叠区：可见可交互
        assert!(!widget_occluded(1, 10, outside, regions.iter().copied()));
        // 上层控件：不被任何更高层遮挡
        assert!(!widget_occluded(2, 20, inside, [lower].into_iter()));
        // **同一控件的多个区域互不遮挡**（否则滑块轨道被自己的手柄挡掉）
        assert!(!widget_occluded(1, 5, inside, [lower].into_iter()));
        // 无登记区域：恒不遮挡
        assert!(!widget_occluded(1, 10, inside, std::iter::empty()));
    }

    #[test]
    fn widget_occluded_respects_clip_and_equal_key() {
        let clipped = HitRegion {
            owner: 2,
            key: 20,
            rect: Rect::new(0.0, 0.0, 200.0, 200.0),
            clip: Some(Rect::new(0.0, 0.0, 100.0, 100.0)),
        };
        // 区域命中但在**裁剪层外**（滚出可视区）→ 不算覆盖
        assert!(!widget_occluded(1, 10, Vec2::new(150.0, 150.0), [clipped].into_iter()));
        assert!(widget_occluded(1, 10, Vec2::new(50.0, 50.0), [clipped].into_iter()));
        // 同 key（同层）：不算"更高"——不遮挡（只有严格更高才拦）
        assert!(!widget_occluded(1, 20, Vec2::new(50.0, 50.0), [clipped].into_iter()));
    }

    #[test]
    fn id_hash_is_stable_and_distinguishes() {
        use crate::id::IdAbsolute;
        let a = IdAbsolute::from("win/btn");
        let b = IdAbsolute::from("win/btn");
        let c = IdAbsolute::from("win/btn2");
        assert_eq!(id_hash(&a), id_hash(&b), "同一 id 必须同哈希（判定才稳定）");
        assert_ne!(id_hash(&a), id_hash(&c));
        // borrowed / owned 两种构造同值即同哈希（`--` 拼接路径 vs 字面量）
        assert_eq!(id_hash(&a), id_hash(&IdAbsolute::owned("win/btn".to_owned())));
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
    fn normalize_y_maps_and_clamps() {
        // 竖向镜像：色相条 / SV 平面取参数值用（顶部 = 0，底部 = 1）。
        let r = Rect::new(0.0, 50.0, 10.0, 200.0);
        assert!((normalize_y(&r, 50.0) - 0.0).abs() < 1e-5, "顶边 = 0");
        assert!((normalize_y(&r, 150.0) - 0.5).abs() < 1e-5, "中点 = 0.5");
        assert!((normalize_y(&r, 250.0) - 1.0).abs() < 1e-5, "底边 = 1");
        assert!((normalize_y(&r, -10.0) - 0.0).abs() < 1e-5, "越界 clamp 到 0");
        assert!((normalize_y(&r, 999.0) - 1.0).abs() < 1e-5, "越界 clamp 到 1");
        // 退化高度：返回 0 而不是 NaN / 除零。
        let flat = Rect::new(0.0, 0.0, 10.0, 0.0);
        assert_eq!(normalize_y(&flat, 5.0), 0.0);
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
