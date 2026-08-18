//! 程序化纹理（UI 渲染增强）：圆角矩形 / 线性渐变 / WHITE 基础纹理。
//!
//! - **纯生成函数**（无 GPU，可单测）：[`rounded_rect_rgba`]（抗锯齿圆角填充）、
//!   [`gradient_rgba`]（水平 / 垂直线性渐变）、[`WHITE_RGBA`]；
//! - **`ProcTextures`**：把这些程序化纹理**塞进动态 Atlas**（[`rjw_atlas::DynamicAtlas`]，
//!   与字形图集同机制：Guillotine 打包、页纹理自动注册进 `rjw_render::TEXTURES`、
//!   `insert_permanent` 永久保留、`clamp_margin` 防采样透色），
//!   惰性初始化（首次取纹理时用 `Ui` 传入的 device/queue/layout）。
//!
//! 圆角矩形用 **9-patch** 绘制（见 [`rounded_9patch`]）：四角原样、四边/中心拉伸，
//! 任意矩形尺寸下圆弧不畸变。渐变矩形直接拉伸采样（主轴 64 级已足够平滑）。
//!
//! 绘制入口在 `Ui`（[`crate::Ui::rounded_rect_at`] / [`crate::Ui::gradient_rect`]），
//! 控件样式集成见 [`crate::style`]（`PanelStyle::radius` / `ButtonStyle::radius` 等）。

use glam::Vec2;
use rjw_atlas::{AtlasConfig, AtlasRegion, DynamicAtlas};
use rjw_color::Color;
use rjw_render::wgpu;
use rjw_transform::Rect;

/// 圆角矩形纹理尺寸（px；9-patch 用，四角 + 拉伸边）。
pub const ROUNDED_TEX_SIZE: u32 = 32;
/// 渐变纹理主轴采样级数（px）。
pub const GRADIENT_TEX_LEN: u32 = 64;

/// 1×1 白像素（RGBA）——`ProcTextures::white` 用（UI 实心矩形基础纹理）。
pub const WHITE_RGBA: [u8; 4] = [255, 255, 255, 255];

/// Color → RGBA8。
#[inline]
fn rgba8(c: Color) -> [u8; 4] {
    let f: [f32; 4] = c.into();
    [
        (f[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (f[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (f[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        (f[3].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

/// 抗锯齿圆角矩形填充纹理（`size × size` RGBA；四角按 `radius` 挖圆，边缘 1px 平滑）。
///
/// 纯函数（可单测）：`radius` 会 clamp 到 `size/2 - 1`；`size = 0` 返回空。
pub fn rounded_rect_rgba(size: u32, radius: f32, color: Color) -> Vec<u8> {
    if size == 0 {
        return Vec::new();
    }
    let n = size as i32;
    let r = radius.clamp(0.0, n as f32 * 0.5 - 1.0).max(0.0);
    let [cr, cg, cb, ca] = rgba8(color);
    let mut out = vec![0u8; (n * n * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            // 到最近边的距离（0 在边缘）。
            let dx = x.min(n - 1 - x) as f32;
            let dy = y.min(n - 1 - y) as f32;
            // 只要有一侧在圆角半径之外（dx >= r 或 dy >= r）→ 落在中心矩形或边带，实心；
            // 只有**双角区**（dx < r 且 dy < r）才做圆弧判定（抗锯齿边缘 1px）。
            let alpha = if dx >= r || dy >= r {
                1.0
            } else {
                let ccx = if x < r as i32 { r } else { (n - 1) as f32 - r };
                let ccy = if y < r as i32 { r } else { (n - 1) as f32 - r };
                let dist = ((x as f32 - ccx).powi(2) + (y as f32 - ccy).powi(2)).sqrt();
                (r - dist + 0.5).clamp(0.0, 1.0)
            };
            let i = ((y * n + x) * 4) as usize;
            out[i] = cr;
            out[i + 1] = cg;
            out[i + 2] = cb;
            out[i + 3] = (ca as f32 * alpha) as u8;
        }
    }
    out
}

/// 线性渐变纹理（`w × h` RGBA）：`vertical = true` 时颜色沿 y（0 = 顶部），否则沿 x。
///
/// `stops`：`(t ∈ [0,1], color)` 至少一项；t 超出范围 clamp。纯函数（可单测）。
pub fn gradient_rgba(w: u32, h: u32, vertical: bool, stops: &[(f32, Color)]) -> Vec<u8> {
    let n = (if vertical { h } else { w }).max(1) as f32;
    let mut out = vec![0u8; (w * h * 4) as usize];
    for i in 0..(w * h) as usize {
        let x = (i as u32 % w) as f32;
        let y = (i as u32 / w) as f32;
        let t = if vertical { y / n } else { x / n };
        let c = sample_stops(t, stops);
        let [r, g, b, a] = rgba8(c);
        out[i * 4] = r;
        out[i * 4 + 1] = g;
        out[i * 4 + 2] = b;
        out[i * 4 + 3] = a;
    }
    out
}

/// 在颜色停靠点之间线性插值（`t` clamp 到 [0,1]；`stops` 为空返回白色）。
#[inline]
pub fn sample_stops(t: f32, stops: &[(f32, Color)]) -> Color {
    let t = t.clamp(0.0, 1.0);
    if stops.is_empty() {
        return Color::WHITE;
    }
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t <= t1 {
            let k = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
            return lerp_color(c0, c1, k);
        }
    }
    stops.last().expect("non-empty").1
}

/// 颜色线性插值。
#[inline]
fn lerp_color(a: Color, b: Color, k: f32) -> Color {
    let af: [f32; 4] = a.into();
    let bf: [f32; 4] = b.into();
    let mut o = [0f32; 4];
    for i in 0..4 {
        o[i] = af[i] + (bf[i] - af[i]) * k;
    }
    Color::from(o)
}

/// 渐变停靠点的稳定 key（图集缓存键；含颜色值，改变停靠即换纹理）。
pub(crate) fn gradient_key(vertical: bool, stops: &[(f32, Color)]) -> String {
    let mut s = String::from(if vertical { "grad_v_" } else { "grad_h_" });
    for (i, (t, c)) in stops.iter().enumerate() {
        if i > 0 {
            s.push('_');
        }
        let [r, g, b, a] = rgba8(*c);
        s.push_str(&format!("{t:.3}@{r:02x}{g:02x}{b:02x}{a:02x}"));
    }
    s
}

/// **9-patch 分段**：把 `rect`（屏幕逻辑像素）按圆角 `radius` 切成 9 块，
/// 返回每块的 `(网格矩形, UV 左上角, UV 尺寸)`（UV 相对 `tex_size × tex_size` 圆角纹理，
/// 纹理半径 = `tex_radius`）。四角原样采样、四边/中心拉伸，任意尺寸圆弧不畸变。
///
/// 纯函数（可单测）：`radius` 会按 `rect` 尺寸 clamp；`rect` 过小（< 2r）时半径收缩。
pub fn rounded_9patch(
    rect: Rect,
    radius: f32,
    tex_size: u32,
    tex_radius: f32,
) -> [(Rect, Vec2, Vec2); 9] {
    let r = radius.clamp(0.0, rect.w.min(rect.h) * 0.5).max(0.0);
    let ts = tex_size as f32;
    let tr = tex_radius.clamp(0.0, ts * 0.5 - 1.0).max(0.0);
    // 纹理内角部尺寸 = 纹理半径比例 × 网格半径（保持比例一致）。
    let corner_tex = tr / ts;
    // 9 块（行优先）：[左上, 上, 右上, 左, 中, 右, 左下, 下, 右下]。
    let zero = Rect::new(0.0, 0.0, 0.0, 0.0);
    let mut out = [(zero, Vec2::ZERO, Vec2::ZERO); 9];
    let (x0, y0, x1, y1) = (rect.x, rect.y, rect.x + rect.w, rect.y + rect.h);
    let xm = x0 + r;
    let ym = y0 + r;
    let xm2 = x1 - r;
    let ym2 = y1 - r;
    // 网格矩形（相对 rect）与对应 UV。
    let meshes = [
        (x0, y0, r, r),            // 左上
        (xm, y0, rect.w - 2.0 * r, r), // 上
        (xm2, y0, r, r),           // 右上
        (x0, ym, r, rect.h - 2.0 * r), // 左
        (xm, ym, rect.w - 2.0 * r, rect.h - 2.0 * r), // 中
        (xm2, ym, r, rect.h - 2.0 * r), // 右
        (x0, ym2, r, r),           // 左下
        (xm, ym2, rect.w - 2.0 * r, r), // 下
        (xm2, ym2, r, r),          // 右下
    ];
    let uvs = [
        (0.0, 0.0, corner_tex, corner_tex),
        (corner_tex, 0.0, 1.0 - 2.0 * corner_tex, corner_tex),
        (1.0 - corner_tex, 0.0, corner_tex, corner_tex),
        (0.0, corner_tex, corner_tex, 1.0 - 2.0 * corner_tex),
        (corner_tex, corner_tex, 1.0 - 2.0 * corner_tex, 1.0 - 2.0 * corner_tex),
        (1.0 - corner_tex, corner_tex, corner_tex, 1.0 - 2.0 * corner_tex),
        (0.0, 1.0 - corner_tex, corner_tex, corner_tex),
        (corner_tex, 1.0 - corner_tex, 1.0 - 2.0 * corner_tex, corner_tex),
        (1.0 - corner_tex, 1.0 - corner_tex, corner_tex, corner_tex),
    ];
    for (i, ((mx, my, mw, mh), (ux, uy, uw, uh))) in meshes.iter().zip(uvs.iter()).enumerate() {
        out[i] = (
            Rect::new(*mx, *my, *mw, *mh),
            Vec2::new(*ux, *uy),
            Vec2::new(*uw, *uh),
        );
    }
    out
}

// ─── ProcTextures：动态 Atlas 缓存 ─────────────────────────────

/// UI 程序化纹理缓存（`UiState` 持有，跨帧）：圆角矩形 / 渐变 / WHITE 纹理
/// **塞进动态 Atlas**（`insert_permanent` 永久保留，不参与 LRU 逐出）。
///
/// 惰性初始化：首次取纹理时用传入的 device/queue/layout 创建图集；
/// 纹理数量少（几种半径 + 若干渐变 + white），单页 2048² 绰绰有余。
///
/// `Clone` 会**丢弃**图集（`DynamicAtlas` 不可克隆；克隆后的实例下次取纹理时重建）。
#[derive(Default)]
pub struct ProcTextures {
    atlas: Option<DynamicAtlas<String>>,
}

impl Clone for ProcTextures {
    fn clone(&self) -> Self {
        Self { atlas: None }
    }
}

impl std::fmt::Debug for ProcTextures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcTextures")
            .field("atlas_initialized", &self.atlas.is_some())
            .finish()
    }
}

impl ProcTextures {
    fn atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> &mut DynamicAtlas<String> {
        self.atlas.get_or_insert_with(|| {
            DynamicAtlas::new(
                device,
                queue,
                layout,
                AtlasConfig { max_pages: 4, padding: 1, ..Default::default() },
                2048,
            )
        })
    }

    /// WHITE 基础纹理（1×1 白；UI 实心矩形 / 默认填充用）。
    pub fn white(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
    ) -> Option<(u64, AtlasRegion)> {
        let a = self.atlas(device, queue, layout);
        let key = "white".to_string();
        let region = a
            .insert_permanent(key.clone(), &WHITE_RGBA, 1, 1, (0, 0), true)
            .or_else(|| a.get(&key).copied())?;
        Some((region.page_uid, region))
    }

    /// 圆角矩形纹理（`ROUNDED_TEX_SIZE`，clamp margin 防采样透色）。
    ///
    /// 纹理只存 **白色 + alpha**（`rounded_rect_rgba` 固定白色）：绘制时用**顶点色 tint**
    /// 得到任意颜色（key 只含半径，同半径共享一张纹理，不随颜色膨胀图集）。
    pub fn rounded(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        radius: f32,
    ) -> Option<(u64, AtlasRegion)> {
        let r = radius.clamp(0.0, ROUNDED_TEX_SIZE as f32 * 0.5 - 1.0).max(0.0);
        let key = format!("rounded_r{}", r as u32);
        let a = self.atlas(device, queue, layout);
        let region = a
            .insert_permanent(
                key.clone(),
                &rounded_rect_rgba(ROUNDED_TEX_SIZE, r, Color::WHITE),
                ROUNDED_TEX_SIZE,
                ROUNDED_TEX_SIZE,
                (0, 0),
                true,
            )
            .or_else(|| a.get(&key).copied())?;
        Some((region.page_uid, region))
    }

    /// 线性渐变纹理（主轴 [`GRADIENT_TEX_LEN`] 级；`vertical` 沿 y 否则沿 x）。
    pub fn gradient(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        vertical: bool,
        stops: &[(f32, Color)],
    ) -> Option<(u64, AtlasRegion)> {
        let key = gradient_key(vertical, stops);
        let (w, h) = if vertical { (1, GRADIENT_TEX_LEN) } else { (GRADIENT_TEX_LEN, 1) };
        let a = self.atlas(device, queue, layout);
        let region = a
            .insert_permanent(
                key.clone(),
                &gradient_rgba(w, h, vertical, stops),
                w,
                h,
                (0, 0),
                true,
            )
            .or_else(|| a.get(&key).copied())?;
        Some((region.page_uid, region))
    }

    /// 清理：显式释放图集（一般无需调用）。
    pub fn clear(&mut self) {
        self.atlas = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_rect_rgba_corner_alpha_and_interior() {
        let size = 32;
        let rgba = rounded_rect_rgba(size, 8.0, Color::WHITE);
        assert_eq!(rgba.len(), (size * size * 4) as usize);
        // 中心像素：完全不透明白色。
        let c = &rgba[((16 * size + 16) * 4) as usize..((16 * size + 16) * 4 + 4) as usize];
        assert_eq!(c, &[255, 255, 255, 255], "中心应不透明");
        // 角落像素（0,0）：圆角外 → 透明（alpha = 0）。
        let corner = &rgba[0..4];
        assert_eq!(corner[3], 0, "圆角外角点应透明");
        // 沿边的中点（16, 0）：在圆角半径内 → 不透明。
        let edge = &rgba[((0 * size + 16) * 4) as usize..((0 * size + 16) * 4 + 4) as usize];
        assert_eq!(edge[3], 255, "边中点应在圆内（半径 8 > x=16 距离）");
    }

    #[test]
    fn gradient_rgba_vertical_interpolates() {
        let w = 1;
        let h = 64;
        let stops = [(0.0, Color::BLACK), (1.0, Color::WHITE)];
        let rgba = gradient_rgba(w, h, true, &stops);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // 顶行 = 黑，底行 ≈ 白（t = 63/64 ≈ 0.984 → ~251）。
        assert_eq!(&rgba[0..3], &[0, 0, 0], "顶部应为黑");
        let last = &rgba[((h - 1) * 4) as usize..((h - 1) * 4 + 3) as usize];
        for v in last {
            assert!(*v >= 250, "底部应接近白，实际 {last:?}");
        }
        // 中点近似灰（±2）。
        let mid = &rgba[((h / 2) * 4) as usize..((h / 2) * 4 + 3) as usize];
        for v in mid {
            assert!((*v as i32 - 128).abs() <= 2, "中点应为 ~128，实际 {mid:?}");
        }
    }

    #[test]
    fn gradient_key_changes_with_stops() {
        let a = gradient_key(true, &[(0.0, Color::BLACK), (1.0, Color::WHITE)]);
        let b = gradient_key(true, &[(0.0, Color::BLACK), (1.0, Color::RED)]);
        assert_ne!(a, b, "停靠点颜色不同 → key 应不同（换纹理）");
        let c = gradient_key(false, &[(0.0, Color::BLACK), (1.0, Color::WHITE)]);
        assert_ne!(a, c, "方向不同 → key 应不同");
    }

    #[test]
    fn rounded_9patch_layout() {
        let rect = Rect::new(10.0, 20.0, 100.0, 60.0);
        let radius = 8.0;
        let parts = rounded_9patch(rect, radius, 32, 8.0);
        // 9 块：四角 r×r；上/下 (w-2r)×r；左/右 r×(h-2r)；中心 (w-2r)×(h-2r)。
        let [tl, top, tr, left, mid, right, bl, bottom, br] = parts;
        assert_eq!((tl.0.w, tl.0.h), (8.0, 8.0));
        assert_eq!((tr.0.w, tr.0.h), (8.0, 8.0));
        assert_eq!((bl.0.w, bl.0.h), (8.0, 8.0));
        assert_eq!((br.0.w, br.0.h), (8.0, 8.0));
        assert_eq!((top.0.w, top.0.h), (100.0 - 16.0, 8.0));
        assert_eq!((bottom.0.w, bottom.0.h), (100.0 - 16.0, 8.0));
        assert_eq!((left.0.w, left.0.h), (8.0, 60.0 - 16.0));
        assert_eq!((right.0.w, right.0.h), (8.0, 60.0 - 16.0));
        assert_eq!((mid.0.w, mid.0.h), (100.0 - 16.0, 60.0 - 16.0));
        // 九块拼合 = 原矩形（左上角 + 尺寸边界）。
        assert_eq!(tl.0.x, 10.0);
        assert_eq!(tl.0.y, 20.0);
        assert_eq!(br.0.x + br.0.w, 110.0);
        assert_eq!(br.0.y + br.0.h, 80.0);
        // UV：四角 = 纹理角部（corner_tex = 8/32 = 0.25）。
        assert!((tl.1.x - 0.0).abs() < 1e-4 && (tl.2.x - 0.25).abs() < 1e-4);
        assert!((tr.1.x - 0.75).abs() < 1e-4, "右上 UV.x = 1 - 0.25");
        // 半径过大 clamp：rect 高 60 → r ≤ 30。
        let clamped = rounded_9patch(rect, 999.0, 32, 8.0);
        assert!(clamped[0].0.w <= 30.0, "半径应 clamp 到 min(w,h)/2");
    }
}
