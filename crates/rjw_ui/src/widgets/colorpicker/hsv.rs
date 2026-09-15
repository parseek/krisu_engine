//! **HSV 换算与 2D 选择区几何**——取色器面板的"平面 / 色相条"层。
//!
//! 纯数学：无 `Ui`、无 GPU、可单测。
//!
//! # 为什么 HSV 平面可以用一个四角顶点色矩形精确表达
//!
//! 平面上的颜色（色相固定为 `h`）= `V · lerp(white, hue_rgb, S)`；把它与 HSV 公式
//! 逐通道对照：最亮通道 `= V`，最暗通道 `= V(1-S)`，中间通道 `= V(1-S(1-t))`
//! ——**与 HSV 定义逐项相等**。于是平面 = 四角 `[白, 纯色相, 黑, 黑]` 的圆角矩形，
//! 双线性插值即精确结果（零纹理、零着色器改动、零额外 draw call）。
//!
//! # 色相条
//!
//! `HUE_STOPS` 是 7 个停靠点（红→黄→绿→青→蓝→品红→红），两两之间是**线性**斜坡
//! ⇒ 6 段两端色渐变即可精确表达；首/末段用逐角圆角（[`hue_seg_radius`]）拼成一根胶囊。
//!
//! # 色相缓存（调用方负责）
//!
//! `RGB → HSV` 在 `V = 0`（黑）或 `S = 0`（灰）时**色相无定义**：直接往返会丢掉色相，
//! "把明度拖到 0 再拖回来"就会跳到红。因此拖动时以调用方缓存的 HSV 为准
//! （见 [`crate::widgets::ColorPickerState`] 的 `hsv` / `hsv_src`）。

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::hit::{normalize_x, normalize_y};

/// 色相条分段数（红→黄→绿→青→蓝→品红→红 = 6 段 7 个停靠点）。
pub const HUE_SEGS: usize = 6;

/// 色相条 7 个停靠点的 RGB（`[0,1]`；0°/60°/.../360°）。
pub const HUE_STOPS: [[f32; 3]; HUE_SEGS + 1] = [
    [1.0, 0.0, 0.0],
    [1.0, 1.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 1.0, 1.0],
    [0.0, 0.0, 1.0],
    [1.0, 0.0, 1.0],
    [1.0, 0.0, 0.0],
];

/// RGB → HSV（`h ∈ [0,1)` 归一化色相，`s` / `v ∈ [0,1]`；灰/黑时 `h = 0`）。
pub fn rgb_to_hsv(c: Color) -> [f32; 3] {
    let a: [f32; 4] = c.into();
    let (r, g, b) = (a[0], a[1], a[2]);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= f32::EPSILON {
        0.0
    } else if max == r {
        ((((g - b) / d) % 6.0) + 6.0) % 6.0
    } else if max == g {
        ((b - r) / d) + 2.0
    } else {
        ((r - g) / d) + 4.0
    } / 6.0;
    let s = if max <= f32::EPSILON { 0.0 } else { d / max };
    [h, s, max]
}

/// HSV → RGB（`h` 取小数部分、`s` / `v` clamp；alpha 原样带入）。
pub fn hsv_to_rgb(h: f32, s: f32, v: f32, alpha: f32) -> Color {
    let h6 = (h.fract() + 1.0).fract() * 6.0;
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let i = h6.floor() as i32 % 6;
    let f = h6 - h6.floor();
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let (r, g, b) = match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color::from([r, g, b, alpha])
}

/// 平面上的一点 → `(s, v)`：横向 = 饱和度、**纵向向下 = 明度降低**（顶边最亮）。
///
/// 越界 clamp（拖出面板也保持有效）。
#[inline]
pub fn sv_at(rect: Rect, local: Vec2) -> (f32, f32) {
    (normalize_x(&rect, local.x), 1.0 - normalize_y(&rect, local.y))
}

/// 色相条上的一点 → `h ∈ [0,1]`（顶 = 0 = 红，底 = 1 = 红）。
#[inline]
pub fn hue_at(rect: Rect, local: Vec2) -> f32 {
    normalize_y(&rect, local.y)
}

/// 色相条第 `i` 段的两端色（上 / 下）。
#[inline]
pub fn hue_seg_colors(i: usize) -> (Color, Color) {
    let top = HUE_STOPS[i.min(HUE_SEGS)];
    let bot = HUE_STOPS[(i + 1).min(HUE_SEGS)];
    (
        Color::from([top[0], top[1], top[2], 1.0]),
        Color::from([bot[0], bot[1], bot[2], 1.0]),
    )
}

/// SV 平面的**网格列数 / 行数**。
///
/// 平面颜色场在 S、V 上都是双线性（有交叉项 `S·V`），而光栅化器只做**逐三角形线性**
/// 插值：整块画（两个三角形或从中心扇形铺开）会在三角形边界留下可见的**折痕/条纹**。
/// 切成 `SV_COLS × SV_ROWS` 小格后，每格的交叉项误差按格面积缩小（∝ 1/格数²），
/// 肉眼即平滑渐变（8×4 = 32 个四边形，全部进窗口顶点缓存）。
pub const SV_COLS: u32 = 8;
/// 见 [`SV_COLS`]。
pub const SV_ROWS: u32 = 4;

/// **SV 平面切成网格**：返回每格的矩形与四角色 `[TL, TR, BL, BR]`（由 HSV 公式精确取值）。
///
/// 纯函数（无 `Ui`、无 GPU），可单测"网格化后的最大颜色偏差"。
pub fn sv_plane_cells(rect: Rect, hue: f32) -> Vec<(Rect, [Color; 4])> {
    let mut out = Vec::with_capacity((SV_COLS * SV_ROWS) as usize);
    let cw = rect.w / SV_COLS as f32;
    let ch = rect.h / SV_ROWS as f32;
    for r in 0..SV_ROWS {
        // 行 0 在**顶部** = 明度最高（V 向下递减）。
        let v_top = 1.0 - r as f32 / SV_ROWS as f32;
        let v_bot = 1.0 - (r + 1) as f32 / SV_ROWS as f32;
        for c in 0..SV_COLS {
            let s0 = c as f32 / SV_COLS as f32;
            let s1 = (c + 1) as f32 / SV_COLS as f32;
            let cell = Rect::new(
                rect.x + cw * c as f32,
                rect.y + ch * r as f32,
                cw,
                ch,
            );
            let corners = [
                hsv_to_rgb(hue, s0, v_top, 1.0), // TL
                hsv_to_rgb(hue, s1, v_top, 1.0), // TR
                hsv_to_rgb(hue, s0, v_bot, 1.0), // BL
                hsv_to_rgb(hue, s1, v_bot, 1.0), // BR
            ];
            out.push((cell, corners));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_hsv_round_trip_covers_primaries_and_gray() {
        for c in [
            Color::rgba_u8(255, 0, 0, 255),
            Color::rgba_u8(0, 255, 0, 255),
            Color::rgba_u8(0, 0, 255, 255),
            Color::rgba_u8(255, 255, 0, 255),
            Color::rgba_u8(0, 255, 255, 255),
            Color::rgba_u8(255, 0, 255, 255),
            Color::rgba_u8(255, 255, 255, 255),
            Color::rgba_u8(0, 0, 0, 255),
            Color::rgba_u8(128, 128, 128, 255),
            Color::rgba_u8(12, 200, 64, 255),
        ] {
            let hsv = rgb_to_hsv(c);
            let back = hsv_to_rgb(hsv[0], hsv[1], hsv[2], 1.0);
            let (a, b): ([f32; 4], [f32; 4]) = (c.into(), back.into());
            for i in 0..3 {
                assert!(
                    (a[i] - b[i]).abs() <= 0.005,
                    "往返失真：{c:?} → {hsv:?} → {back:?}（分量 {i}）"
                );
            }
            assert!(
                (0.0..=1.0).contains(&hsv[0])
                    && (0.0..=1.0).contains(&hsv[1])
                    && (0.0..=1.0).contains(&hsv[2])
            );
        }
        // 灰色的色相未定义：约定 0，饱和度 0。
        let gray = rgb_to_hsv(Color::rgba_u8(128, 128, 128, 255));
        assert_eq!(gray[0], 0.0);
        assert!(gray[1].abs() < 1e-6, "灰色饱和度应为 0");
    }

    #[test]
    fn hsv_cache_keeps_hue_when_value_hits_zero() {
        // 回归：把明度拖到 0（黑）时 HSV→RGB 丢掉色相，再拖回来必须还是同一色相
        // ——这正是 `ColorPickerState::hsv` 缓存存在的唯一理由。
        let start = hsv_to_rgb(0.6, 0.8, 0.9, 1.0);
        let mut hsv = rgb_to_hsv(start);
        let hue0 = hsv[0];
        hsv[2] = 0.0;
        let _black = hsv_to_rgb(hsv[0], hsv[1], hsv[2], 1.0);
        assert_eq!(rgb_to_hsv(Color::BLACK)[0], 0.0, "黑色自身已无色相（重派生会丢）");
        hsv[2] = 0.9;
        let back = hsv_to_rgb(hsv[0], hsv[1], hsv[2], 1.0);
        assert!((rgb_to_hsv(back)[0] - hue0).abs() < 1e-3, "色相应保持不变");
    }

    #[test]
    fn sv_plane_corners_match_hsv_definition() {
        // 平面四角 = 白 / 纯色相 / 黑 / 黑（这就是"[白, 纯色相, 黑, 黑]"的由来）。
        let hue = hsv_to_rgb(0.33, 1.0, 1.0, 1.0);
        let white = hsv_to_rgb(0.33, 0.0, 1.0, 1.0);
        let black = hsv_to_rgb(0.33, 1.0, 0.0, 1.0);
        let (a, b, c): ([f32; 4], [f32; 4], [f32; 4]) =
            (white.into(), hue.into(), black.into());
        assert!((a[0] - 1.0).abs() < 1e-5 && (a[1] - 1.0).abs() < 1e-5 && (a[2] - 1.0).abs() < 1e-5);
        assert!(b[1] > b[0] && b[1] > b[2], "0.33 色相应偏绿");
        assert!(c[0] < 1e-5 && c[1] < 1e-5 && c[2] < 1e-5);
    }

    #[test]
    fn sv_and_hue_mapping_matches_corners_and_clamps() {
        let rect = Rect::new(10.0, 20.0, 100.0, 50.0);
        // SV 平面：左上 = (s=0, v=1)，右下 = (s=1, v=0)。
        assert_eq!(sv_at(rect, Vec2::new(10.0, 20.0)), (0.0, 1.0));
        assert_eq!(sv_at(rect, Vec2::new(110.0, 70.0)), (1.0, 0.0));
        let (s, v) = sv_at(rect, Vec2::new(60.0, 45.0));
        assert!((s - 0.5).abs() < 1e-5 && (v - 0.5).abs() < 1e-5, "中心 = (0.5, 0.5)");
        // 越界 clamp（拖出面板也保持有效）。
        assert_eq!(sv_at(rect, Vec2::new(-999.0, -999.0)), (0.0, 1.0));
        assert_eq!(sv_at(rect, Vec2::new(999.0, 999.0)), (1.0, 0.0));
        // 色相条：顶 = 0（红）、底 = 1。
        assert_eq!(hue_at(rect, Vec2::new(0.0, 20.0)), 0.0);
        assert_eq!(hue_at(rect, Vec2::new(0.0, 70.0)), 1.0);
        assert!((hue_at(rect, Vec2::new(0.0, 45.0)) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn hue_bar_segments_close_the_loop_without_seams() {
        // 首尾停靠点同色（红）⇒ 色相条首尾无缝。
        assert_eq!(HUE_STOPS[0], HUE_STOPS[HUE_SEGS]);
        // 相邻段首尾相接：第 i 段的"下" = 第 i+1 段的"上"（段间无跳变）。
        for i in 0..HUE_SEGS - 1 {
            let (_, bot) = hue_seg_colors(i);
            let (top, _) = hue_seg_colors(i + 1);
            assert_eq!(bot, top, "第 {i} 段与第 {} 段必须无缝", i + 1);
        }
    }

    /// 网格化的 SV 平面在某点的颜色——按**光栅化器的真实插值**重建：每格被对角线
    /// `TL→BR` 切成两个三角形（与 `tess::push_plain_quad` 的 `[TL,TR,BR]` /
    /// `[BR,BL,TL]` 一致），三角形内是**线性**插值（不是双线性！）。
    fn sample_grid(cells: &[(Rect, [Color; 4])], p: Vec2, _rect: Rect) -> [f32; 4] {
        let hit = cells.iter().find(|(cell, _)| {
            p.x >= cell.x - 1e-4
                && p.x <= cell.x + cell.w + 1e-4
                && p.y >= cell.y - 1e-4
                && p.y <= cell.y + cell.h + 1e-4
        });
        let Some((cell, c)) = hit else { return [0.0; 4] };
        let u = ((p.x - cell.x) / cell.w).clamp(0.0, 1.0);
        let v = ((p.y - cell.y) / cell.h).clamp(0.0, 1.0);
        let (tl, tr, bl, br) = (c[0], c[1], c[2], c[3]);
        if v < u {
            tri_color(
                (tl, Vec2::ZERO),
                (tr, Vec2::new(1.0, 0.0)),
                (br, Vec2::new(1.0, 1.0)),
                Vec2::new(u, v),
            )
        } else {
            tri_color(
                (br, Vec2::new(1.0, 1.0)),
                (bl, Vec2::new(0.0, 1.0)),
                (tl, Vec2::ZERO),
                Vec2::new(u, v),
            )
        }
    }

    /// 三角形 `(a, b, c)` 内 `p` 处的颜色（重心插值）。
    fn tri_color(a: (Color, Vec2), b: (Color, Vec2), c: (Color, Vec2), p: Vec2) -> [f32; 4] {
        let (ca, pa) = a;
        let (cb, pb) = b;
        let (cc, pc) = c;
        let (v0, v1, v2) = (pb - pa, pc - pa, p - pa);
        let den = v0.x * v1.y - v1.x * v0.y;
        let (u, w) = if den.abs() < 1e-9 {
            (0.0, 0.0)
        } else {
            (
                (v2.x * v1.y - v1.x * v2.y) / den,
                (v0.x * v2.y - v2.x * v0.y) / den,
            )
        };
        let fa: [f32; 4] = ca.into();
        let fb: [f32; 4] = cb.into();
        let fc: [f32; 4] = cc.into();
        let mut out = [0.0f32; 4];
        for k in 0..4 {
            out[k] = fa[k] * (1.0 - u - w) + fb[k] * u + fc[k] * w;
        }
        out
    }

    #[test]
    fn sv_plane_grid_keeps_the_gradient_smooth() {
        // 回归："过渡有问题"——SV 平面曾用**一整块**四角渐变（等于两个三角形），
        // 而颜色场含交叉项 `S·V`，三角形内的线性插值偏离真实 HSV 值 ⇒ 边界出现折痕。
        // 网格化后逐点误差应显著下降，且落在肉眼不可见的量级。
        let rect = Rect::new(10.0, 20.0, 360.0, 110.0);
        let hue = 0.62; // 蓝紫色相（交叉项强的区域）
        let max_err = |cells: &[(Rect, [Color; 4])]| {
            let mut worst = 0.0f32;
            for i in 0..=40 {
                for j in 0..=14 {
                    let p = Vec2::new(
                        rect.x + rect.w * i as f32 / 40.0,
                        rect.y + rect.h * j as f32 / 14.0,
                    );
                    let got = sample_grid(cells, p, rect);
                    let s = (p.x - rect.x) / rect.w;
                    let v = 1.0 - (p.y - rect.y) / rect.h;
                    let want: [f32; 4] = hsv_to_rgb(hue, s, v, 1.0).into();
                    for k in 0..3 {
                        worst = worst.max((got[k] - want[k]).abs());
                    }
                }
            }
            worst
        };
        let coarse = sv_plane_cells_with(rect, hue, 1, 1);
        let fine = sv_plane_cells(rect, hue);
        let (e_coarse, e_fine) = (max_err(&coarse), max_err(&fine));
        assert!(
            e_fine < e_coarse * 0.2,
            "网格化应把交叉项误差压到 1/5 以下：粗 {e_coarse:.4} → 细 {e_fine:.4}"
        );
        assert!(
            e_fine < 0.01,
            "细网格的最大通道偏差应 < 1%（肉眼不可见），实际 {e_fine:.4}"
        );
        assert_eq!(fine.len(), (SV_COLS * SV_ROWS) as usize, "格子数 = 列×行");
    }

    /// 任意列/行的变体（供上面的误差对比用）。
    fn sv_plane_cells_with(rect: Rect, hue: f32, cols: u32, rows: u32) -> Vec<(Rect, [Color; 4])> {
        let mut out = Vec::new();
        let (cw, ch) = (rect.w / cols as f32, rect.h / rows as f32);
        for r in 0..rows {
            let v_top = 1.0 - r as f32 / rows as f32;
            let v_bot = 1.0 - (r + 1) as f32 / rows as f32;
            for c in 0..cols {
                let s0 = c as f32 / cols as f32;
                let s1 = (c + 1) as f32 / cols as f32;
                out.push((
                    Rect::new(rect.x + cw * c as f32, rect.y + ch * r as f32, cw, ch),
                    [
                        hsv_to_rgb(hue, s0, v_top, 1.0),
                        hsv_to_rgb(hue, s1, v_top, 1.0),
                        hsv_to_rgb(hue, s0, v_bot, 1.0),
                        hsv_to_rgb(hue, s1, v_bot, 1.0),
                    ],
                ));
            }
        }
        out
    }
}
