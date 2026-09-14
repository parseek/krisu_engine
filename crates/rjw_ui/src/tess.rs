//! **圆角矩形 CPU 镶嵌**（硬体 + 1 物理像素羽化带）——不依赖任何着色器改动。
//!
//! # 为什么不改着色器
//!
//! 抗锯齿不靠 SDF，靠**光栅化器对顶点 alpha 的线性插值**：
//! 硬体边界点 `alpha = 1`，同一点沿法线外扩 `feather` 得到外环点 `alpha = 0`，
//! 两者配成带状三角形后自然插出 `1 → 0` 的过渡——与 `RStates` 默认的直通 Alpha
//! 混合配合即为羽化边缘。因此 `sprite.wgsl` 与 `InstanceData` **零改动**，
//! 也不需要第二条管线（`CustomDraw` 是合批屏障且拿不到 `PassContext`）。
//!
//! # 为什么用「单位弧表 + 步长抽样」（而不是每半径一张表）
//!
//! 圆角弧长 = `radius × π/2`，所以「弧距约 2px」对应的**段数随半径变化**。若按半径
//! 缓存整张轮廓表，就等于「每半径一张表」。改法是把二者解耦：
//!
//! ```text
//! 缓存一次：UNIT[N_FINE]                  // 一张细表，服务所有半径
//! 步长 stride = N_FINE / segs            // 由半径推出
//! 抽样：k = 0, 1, …, segs → UNIT[k * stride]
//! ```
//!
//! `N_FINE` 取 2 的幂 ⇒ `stride` 也只能是 2 的幂 ⇒ `segs = N_FINE / stride` 必然整除
//! `N_FINE`，于是 `k = segs` 恰好落在表的端点上（`UNIT[N_FINE]`），四角弧首尾严丝合缝。
//!
//! # 羽化环与硬体轮廓的对齐
//!
//! 羽化环可以用比硬体**更粗**的段数（羽化是渐隐的，对锐利度要求低，这是顶点数的
//! 主要旋钮），但环上每个采样点必须**精确**落在硬体轮廓的某个点上，否则带子错位。
//! 约束：`feather_segs` 整除 `segs`。两侧都是 2 的幂 ⇒ 恒成立
//! （`segs ≤ 2` 时取 `feather_segs = segs`；`segs ≥ 4` 时取 `FEATHER_SEGS_MAX = 4`）。
//!
//! # 索引不缓存、不共用
//!
//! 索引与顶点**同源**（同一个抽样循环、同一个轮廓长度），每次内联构造并平移基址。
//! 这是**正确性**要求而非性能选择：若两者各算一遍，任何「点数 / 是否闭合成环」的
//! 判断偏差都会产生错乱三角形，且**不会 panic**，只是画面错。
//!
//! 也**不使用静态网格**：静态网格把几何冻结在 GPU 缓冲里，与 UI「每帧重新录制」
//! 的立即模式冲突（窗口内容一变就得重建注册网格，还要管生命周期）；且羽化带是
//! 1 物理像素、**不可被实例缩放**，本来就必须逐矩形产生——省下的只有硬体那一半，
//! 不足以抵偿注册与生命周期管理的复杂度。
//!
//! # 顶点绕序
//!
//! 轮廓沿**屏幕顺时针**（Y 向下）绕行：左中 → 左上 → 右上 → 右中 → 右下 → 左下 →
//! 回到左中。硬体扇形三角形 (轮廓ᵢ, 轮廓ᵢ₊₁, 中心) 与四边形的
//! (TL, TR, BR) / (BR, BL, TL) 约定同向（叉积同号），因此 `cull` 状态下的可见性
//! 与旧四边形路径一致。羽化带的带状三角形是反向的（外环在硬体之外），已按
//! `(inner0, outer1, inner1)` / `(inner0, outer0, outer1)` 翻转回来。

use std::rc::Rc;

use glam::Vec2;
use rjw_2d_render::VertexP3U2C4;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::backend::Tri;

/// 细表取样点数（每个四分之一圆弧）。2 的幂 —— `stride` 取 2 的幂即可整除本值。
pub(crate) const N_FINE: u32 = 32;

/// 相邻轮廓顶点的**目标弧距**（物理像素）：越小越圆滑、顶点越多。
pub(crate) const ARC_STEP_PX: f32 = 2.0;

/// 羽化带宽（物理像素）。
pub(crate) const FEATHER_PX: f32 = 1.0;

/// 羽化带**自己的**段数上限：羽化是渐隐的，对锐利度要求低于硬体边缘，
/// 所以可以用比硬体更粗的段数。
pub(crate) const FEATHER_SEGS_MAX: u32 = 4;

/// 单位四分之一圆弧上的一个点：`(cos t, sin t)`，`t ∈ [0, π/2]`。
///
/// 圆上点的单位外法线就是该点方向本身，故不重复存。
type ArcPoint = (f32, f32);

/// **单位四分之一圆弧表**：一张表服务所有半径。只含 `sin_cos` 的计算结果。
#[derive(Debug)]
pub(crate) struct CornerTable {
    /// `N_FINE + 1` 个点：`t = i / N_FINE * π/2`，`i ∈ 0..=N_FINE`。
    pts: Vec<ArcPoint>,
}

/// 允许的抽样步长（`N_FINE` 的约数；`N_FINE = 32` ⇒ 2 的幂）。
const STRIDES: [u32; 5] = [1, 2, 4, 8, 16];

impl CornerTable {
    /// 构造细表。只有建表时算 `sin_cos`（每帧上百个矩形复用同一张表）。
    ///
    /// 存 `(cos t, sin t)`——注意 `f32::sin_cos()` 返回 `(sin, cos)`，顺序要换过来，
    /// 否则弧的旋向整体反转（轮廓变成逆时针、羽化带落错位置）。
    fn build() -> Self {
        let mut pts = Vec::with_capacity(N_FINE as usize + 1);
        for i in 0..=N_FINE {
            let t = i as f32 / N_FINE as f32 * std::f32::consts::FRAC_PI_2;
            let (sin_t, cos_t) = t.sin_cos();
            pts.push((cos_t, sin_t));
        }
        Self { pts }
    }

    /// 半径 → 抽样步长（2 的幂）。目标：相邻轮廓顶点弧距 ≈ [`ARC_STEP_PX`]。
    ///
    /// 段数 = `N_FINE / stride`，弧长 = `radius × π/2`，
    /// 故 `want = radius × π / (2 × ARC_STEP_PX)`；在 2 的幂里取最接近的一个。
    #[inline]
    fn stride_for(&self, radius_px: f32) -> u32 {
        let want = (radius_px * std::f32::consts::PI / (2.0 * ARC_STEP_PX)).max(1.0);
        let ideal = N_FINE as f32 / want;
        // `STRIDES` 全为 2 的幂 ⇒ 取最接近的 2 的幂即为最优。
        let mut best = STRIDES[0];
        let mut best_d = f32::INFINITY;
        for &s in &STRIDES {
            let d = (s as f32).log2() - ideal.log2();
            let d = d * d;
            if d < best_d {
                best_d = d;
                best = s;
            }
        }
        best
    }

    /// 步长 → 段数。因为 `N_FINE` 是 2 的幂、`stride` 也是，除法必然整除。
    #[inline]
    fn segs_of(stride: u32) -> u32 {
        N_FINE / stride
    }

    /// 按 `stride` 抽样出的第 `k` 个点（`k ∈ 0..=segs`）。
    #[inline]
    fn sample(&self, stride: u32, k: u32) -> ArcPoint {
        let i = (k * stride).min(N_FINE) as usize;
        self.pts[i]
    }
}

/// 跨帧的镶嵌缓存。住 [`crate::UiState`]（`Ui` 每帧重建，放它里面等于每帧重建表）。
///
/// 只有**一张**表 ⇒ 不需要 LRU、不需要容量上限、不需要版本号。
#[derive(Debug, Default, Clone)]
pub(crate) struct TessCache {
    table: Option<Rc<CornerTable>>,
}

impl TessCache {
    /// 取（或建）单位弧表。
    #[inline]
    pub(crate) fn table(&mut self) -> Rc<CornerTable> {
        self.table
            .get_or_insert_with(|| Rc::new(CornerTable::build()))
            .clone()
    }
}

/// 一个待镶嵌圆角矩形的参数。
#[derive(Debug, Clone, Copy)]
pub(crate) struct RoundedRectSpec {
    /// 目标矩形（局部 / 屏幕物理像素；左上原点）。
    pub rect: Rect,
    /// 圆角半径（物理像素，逻辑半径已由调用方换算）。
    pub radius: f32,
    /// 四角色 `[TL, TR, BL, BR]`。
    ///
    /// **必须支持四角各异**——这正是「圆角 + 渐变」需要逐顶点色、不能走实例单色的原因：
    /// 硬体四角各取本角颜色，中心取四角均值，光栅化器在扇形三角形内做重心插值。
    pub corners: [Color; 4],
}

/// 镶嵌结果：本次追加的顶点数与三角形数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TessOutput {
    pub verts: usize,
    pub tris: usize,
}

/// 一个圆角：圆心 + 该角弧的两条**单位基向量**。
///
/// 弧上点 = `center + radius * (cos t · a + sin t · b)`，`t ∈ [0, π/2]` 沿屏幕顺时针。
/// 四角依次为（顺时针 TL → TR → BR → BL）：
///
/// | 角 | `a`（t=0 方向） | `b`（t=π/2 方向） | 弧 |
/// |---|---|---|---|
/// | TL | `(-1, 0)` 左 | `(0, -1)` 上 | 左中 → 左上 |
/// | TR | `(0, -1)` 上 | `(1, 0)` 右 | 左上 → 右中 |
/// | BR | `(1, 0)` 右 | `(0, 1)` 下 | 右中 → 右下 |
/// | BL | `(0, 1)` 下 | `(-1, 0)` 左 | 左下 → 左下 |
///
/// 递推规律 `(a, b) ← (b, -a)`（每次顺时针转 90°），因此可以循环生成而不是手写四份。
#[derive(Debug, Clone, Copy)]
struct Corner {
    center: Vec2,
    a: Vec2,
    b: Vec2,
    color: Color,
}

impl Corner {
    /// 弧上参数 `t = k / segs * π/2` 处的**单位外法线**（= 径向单位向量）。
    #[inline]
    fn dir(&self, cos_t: f32, sin_t: f32) -> Vec2 {
        self.a * cos_t + self.b * sin_t
    }
}

/// 四角（顺时针 TL → TR → BR → BL）的圆心与基向量。
///
/// `radius` 已 clamp 过，调用方保证 `0 < radius ≤ min(w, h) / 2`。
#[inline]
fn corners_of(rect: Rect, radius: f32, colors: [Color; 4]) -> [Corner; 4] {
    let Rect { x, y, w, h } = rect;
    let centers = [
        Vec2::new(x + radius, y + radius),                 // TL
        Vec2::new(x + w - radius, y + radius),             // TR
        Vec2::new(x + w - radius, y + h - radius),         // BR
        Vec2::new(x + radius, y + h - radius),             // BL
    ];
    let mut a = Vec2::new(-1.0, 0.0);
    let mut b = Vec2::new(0.0, -1.0);
    let mut out = [Corner {
        center: Vec2::ZERO,
        a,
        b,
        color: colors[0],
    }; 4];
    for (i, slot) in out.iter_mut().enumerate() {
        slot.center = centers[i];
        slot.a = a;
        slot.b = b;
        slot.color = colors[i];
        // 顺时针转 90°：`(a, b) ← (b, -a)`。
        let na = b;
        let nb = Vec2::new(-a.x, -a.y);
        a = na;
        b = nb;
    }
    out
}

/// 羽化环第 `k` 个采样点对应到硬体轮廓（该角内）的点号。
///
/// 硬体轮廓每角 `segs + 1` 个点（点号 `0..=segs`），羽化环每角
/// `feather_segs + 1` 个点。`feather_segs` 整除 `segs` ⇒ `k * segs / feather_segs`
/// 是整数，环上每点都**精确**落在硬体轮廓的某个点上，带子不会错位。
#[inline]
fn corner_inner_base(segs: u32, feather_segs: u32, k: u32) -> u32 {
    if feather_segs == 0 {
        return 0;
    }
    (k * segs / feather_segs).min(segs)
}

/// 直角矩形（`radius == 0`）：一个四边形，四角各异颜色由光栅化器双线性插值。
///
/// 顶点顺序 `[TL, TR, BL, BR]`，索引与 `QUAD_TRI_INDICIES` 同约定。
fn push_plain_quad(
    verts: &mut Vec<VertexP3U2C4>,
    tris: &mut Vec<Tri>,
    rect: Rect,
    corners: [Color; 4],
) -> TessOutput {
    let Rect { x, y, w, h } = rect;
    let base = verts.len() as u16;
    let [tl, tr, bl, br] = corners.map(Into::<[f32; 4]>::into);
    verts.push(VertexP3U2C4 { pos: [x, y, 0.0], uv: [0.0, 0.0], color: tl });
    verts.push(VertexP3U2C4 { pos: [x + w, y, 0.0], uv: [0.0, 0.0], color: tr });
    verts.push(VertexP3U2C4 { pos: [x, y + h, 0.0], uv: [0.0, 0.0], color: bl });
    verts.push(VertexP3U2C4 { pos: [x + w, y + h, 0.0], uv: [0.0, 0.0], color: br });
    tris.push([base, base + 1, base + 3]);
    tris.push([base + 3, base + 2, base]);
    TessOutput { verts: 4, tris: 2 }
}

/// **把一个圆角矩形镶嵌成三角形**（硬体 + 羽化带），追加进 `verts` / `tris`。
///
/// - 顶点坐标为**调用方给定的坐标系**（`spec.rect` 所在空间）。UI 传窗口局部物理像素，
///   窗口级变换仍由批次实例应用（顶点缓存不因窗口 FX 失效）。
/// - 追加到非空缓冲时索引按**现有顶点数**自动偏移 ⇒ 可连续拼接多个矩形。
/// - 羽化宽 = `min(FEATHER_PX, min(w, h) / 4)`：小控件自动收紧，避免糊成一团。
/// - 半径 clamp 成胶囊（与 CSS / egui 语义一致）。
pub(crate) fn push_rounded_rect(
    verts: &mut Vec<VertexP3U2C4>,
    tris: &mut Vec<Tri>,
    table: &CornerTable,
    spec: RoundedRectSpec,
) -> TessOutput {
    let Rect { x, y, w, h } = spec.rect;
    if w <= 0.0 || h <= 0.0 {
        return TessOutput { verts: 0, tris: 0 };
    }
    let radius = spec.radius.clamp(0.0, w.min(h) * 0.5);
    if radius <= 0.0 {
        return push_plain_quad(verts, tris, spec.rect, spec.corners);
    }

    let stride = table.stride_for(radius);
    let segs = CornerTable::segs_of(stride);
    // 羽化段数：2 的幂且整除 segs（`segs ≤ 2` 时取 segs，否则取上限）。
    let feather_segs = segs.min(FEATHER_SEGS_MAX);
    let fstride = N_FINE / feather_segs;
    let feather = FEATHER_PX.min(w.min(h) * 0.25);

    let verts_before = verts.len();
    let tris_before = tris.len();
    let corners = corners_of(spec.rect, radius, spec.corners);

    // ── 硬体轮廓（alpha = 1）──
    // 每角 `segs + 1` 个点；相邻两角之间不经直边中点——矩形直边无需细分
    // （光栅化器自身覆盖，无锯齿），角的末点与下一角的首点用一条直连边表达。
    let outline_start = verts.len();
    for c in &corners {
        let mut col: [f32; 4] = c.color.into();
        col[3] = 1.0;
        for k in 0..=segs {
            let (cos_t, sin_t) = table.sample(stride, k);
            let pos = c.center + c.dir(cos_t, sin_t) * radius;
            verts.push(VertexP3U2C4 { pos: [pos.x, pos.y, 0.0], uv: [0.0, 0.0], color: col });
        }
    }
    let outline_len = verts.len() - outline_start;
    debug_assert_eq!(outline_len, 4 * (segs as usize + 1));

    // ── 硬体填充：以矩形中心为轴的扇形三角化 ──
    // 轮廓天然闭合（最后一个点连回第一个点）。四角色各异 ⇒ 中心 = 四角均值，
    // 扇形内做重心插值，等价于双线性渐变的可接受近似（矩形内部本来就是这种场）。
    let mut center_col = [0.0f32; 4];
    for c in &corners {
        let a: [f32; 4] = c.color.into();
        for (dst, s) in center_col.iter_mut().zip(a) {
            *dst += s * 0.25;
        }
    }
    let center_idx = verts.len() as u16;
    verts.push(VertexP3U2C4 {
        pos: [x + w * 0.5, y + h * 0.5, 0.0],
        uv: [0.0, 0.0],
        color: center_col,
    });
    for i in 0..outline_len as u16 {
        let a = outline_start as u16 + i;
        let b = outline_start as u16 + (i + 1) % outline_len as u16;
        tris.push([a, b, center_idx]);
    }

    // ── 羽化带：外环 alpha = 0，与硬体同角轮廓配成带状 ──
    if feather > 0.0 {
        for (ci, c) in corners.iter().enumerate() {
            let ring_start = verts.len() as u16;
            let mut col: [f32; 4] = c.color.into();
            col[3] = 0.0;
            for k in 0..=feather_segs {
                let (cos_t, sin_t) = table.sample(fstride, k);
                let pos = c.center + c.dir(cos_t, sin_t) * (radius + feather);
                verts.push(VertexP3U2C4 { pos: [pos.x, pos.y, 0.0], uv: [0.0, 0.0], color: col });
            }
            let corner_base = outline_start as u16 + ci as u16 * (segs as u16 + 1);
            for k in 0..feather_segs {
                let inner0 = corner_base + corner_inner_base(segs, feather_segs, k) as u16;
                let inner1 = corner_base + corner_inner_base(segs, feather_segs, k + 1) as u16;
                let outer0 = ring_start + k as u16;
                let outer1 = ring_start + (k + 1) as u16;
                // 外环在硬体之外 ⇒ 带状三角形需相对硬体扇形翻转绕序。
                tris.push([inner0, outer1, inner1]);
                tris.push([inner0, outer0, outer1]);
            }
        }
    }

    TessOutput {
        verts: verts.len() - verts_before,
        tris: tris.len() - tris_before,
    }
}

/// **圆角环带**（圆角矩形的边框）：外轮廓与内轮廓之间的一圈带子。
///
/// 前提：`inner = outer` 各边内缩 `width`、且 `r_inner = max(0, r_outer - width)`
/// ——此时两轮廓的每对角弧**同心**，带子不会自交（与 CSS `border-radius`
/// 的内侧半径规则一致）。
///
/// 退化情形：
/// - `r_outer == 0`（直角边框）走**四条轴对齐矩形条**的专用路径（16 顶点），
///   不让圆弧带子在零半径下退化成 32 个重合点 + 零面积三角形；
/// - `r_inner == 0`（边框宽 ≥ 外半径，如细边框的小方框）时内角为**直角**——
///   这与 CSS `border-radius` 的内侧半径规则一致；该角内侧两点重合，
///   零面积的那一个三角形被跳过。
///
/// 之所以要这个原语：`push_panel_like` 的"外圈 border 色圆角 + 内圈背景圆角"
/// 是**两块实心**叠加，圆角处的抗锯齿边缘会各混合一次；环带只画一次边界。
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_rounded_ring(
    verts: &mut Vec<VertexP3U2C4>,
    tris: &mut Vec<Tri>,
    table: &CornerTable,
    rect: Rect,
    radius: f32,
    width: f32,
    color: Color,
) -> TessOutput {
    let Rect { w, h, .. } = rect;
    if w <= 0.0 || h <= 0.0 || width <= 0.0 {
        return TessOutput { verts: 0, tris: 0 };
    }
    let width = width.min(w.min(h) * 0.5);
    let ro = radius.clamp(0.0, w.min(h) * 0.5);
    let inner_rect = rect.shrink(width);
    let ri = (ro - width).max(0.0).min(inner_rect.w.min(inner_rect.h) * 0.5);

    let mut col: [f32; 4] = color.into();
    col[3] = 1.0;
    let verts_before = verts.len();
    let tris_before = tris.len();

    // 直角边框（内外半径都为 0）：四条轴对齐矩形条。走专用路径而不是让圆弧带子
    // 在零半径下退化（那会产生一堆零面积三角形，且顶点数从 16 涨到 32）。
    if ro <= 0.0 {
        let mut push = |r: Rect| {
            let b = verts.len() as u16;
            for p in [
                (r.x, r.y),
                (r.x + r.w, r.y),
                (r.x, r.y + r.h),
                (r.x + r.w, r.y + r.h),
            ] {
                verts.push(VertexP3U2C4 { pos: [p.0, p.1, 0.0], uv: [0.0, 0.0], color: col });
            }
            tris.push([b, b + 1, b + 3]);
            tris.push([b + 3, b + 2, b]);
        };
        for r in crate::draw::border_rects(&rect, width) {
            if r.w > 0.0 && r.h > 0.0 {
                push(r);
            }
        }
        return TessOutput {
            verts: verts.len() - verts_before,
            tris: tris.len() - tris_before,
        };
    }

    // 内轮廓在前、外轮廓在后（带状三角形的绕序按下标区分，见下）。
    //
    // `ri == 0`（边框宽 ≥ 外半径）时内轮廓**塌缩**：每角的 `segs + 1` 个点落在同一
    // 位置上。此时只为每角留**一个**内角点，改用它向该角的外轮廓扇形展开
    // （`[ip, oₖ, oₖ₊₁]`，绕序与带状相反）——否则会退化成零面积三角形。
    let collapsed = ri <= 0.0;
    let outer_corners = corners_of(rect, ro, [color; 4]);
    let stride = table.stride_for(ro);
    let segs = CornerTable::segs_of(stride);
    let per_corner = if collapsed { 1u16 } else { segs as u16 + 1 };

    let inner_corners = corners_of(inner_rect, ri, [color; 4]);
    let inner_start = verts.len();
    for c in &inner_corners {
        for k in 0..per_corner {
            let (cos_t, sin_t) = if collapsed {
                (1.0, 0.0)
            } else {
                table.sample(stride, k as u32)
            };
            let pos = c.center + c.dir(cos_t, sin_t) * ri;
            verts.push(VertexP3U2C4 { pos: [pos.x, pos.y, 0.0], uv: [0.0, 0.0], color: col });
        }
    }
    let outer_start = verts.len();
    for c in &outer_corners {
        for k in 0..=segs {
            let (cos_t, sin_t) = table.sample(stride, k);
            let pos = c.center + c.dir(cos_t, sin_t) * ro;
            verts.push(VertexP3U2C4 { pos: [pos.x, pos.y, 0.0], uv: [0.0, 0.0], color: col });
        }
    }

    // 四个角各自成带（角的末点与下一角的首点之间是直边，由同一条带子跨过）。
    for ci in 0..4usize {
        let ib = inner_start as u16 + ci as u16 * per_corner;
        let ob = outer_start as u16 + ci as u16 * (segs as u16 + 1);
        if collapsed {
            // 内角为直角：从该单一内角点向本角的外轮廓扇形展开。
            for k in 0..segs {
                tris.push([ib, ob + k as u16, ob + k as u16 + 1]);
            }
            continue;
        }
        for k in 0..segs {
            let (inner0, inner1) = (ib + k as u16, ib + k as u16 + 1);
            let (outer0, outer1) = (ob + k as u16, ob + k as u16 + 1);
            // 与羽化带同向：外轮廓在硬体之外 ⇒ 需相对内轮廓翻转绕序。
            if inner0 != inner1 {
                tris.push([inner0, outer1, inner1]);
            }
            tris.push([inner0, outer0, outer1]);
        }
    }

    TessOutput {
        verts: verts.len() - verts_before,
        tris: tris.len() - tris_before,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> CornerTable {
        CornerTable::build()
    }

    fn spec(w: f32, h: f32, r: f32) -> RoundedRectSpec {
        RoundedRectSpec {
            rect: Rect::new(3.0, 5.0, w, h),
            radius: r,
            corners: [Color::RED, Color::GREEN, Color::BLUE, Color::YELLOW],
        }
    }

    /// 镶嵌的通用不变量：索引不越界、无退化三角形。
    fn assert_well_formed(v: &[VertexP3U2C4], tr: &[Tri]) {
        let n = v.len() as u16;
        for t in tr {
            for &i in t.iter() {
                assert!(i < n, "索引 {i} 越界（顶点数 {n}）");
            }
            assert!(
                !(t[0] == t[1] || t[1] == t[2] || t[0] == t[2]),
                "退化三角形 {t:?}"
            );
        }
    }

    /// 三角形绕序叉积（屏幕 Y 向下；与四边形约定 `(TL, TR, BR)` 同号 = 正）。
    fn cross(v: &[VertexP3U2C4], t: &Tri) -> f32 {
        let p = |i: u16| Vec2::new(v[i as usize].pos[0], v[i as usize].pos[1]);
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        (b - a).perp_dot(c - a)
    }

    #[test]
    fn stride_is_a_power_of_two_and_segs_divides_n_fine() {
        let t = table();
        for r in [0.5_f32, 1.0, 2.0, 4.0, 8.0, 16.0, 64.0, 400.0] {
            let s = t.stride_for(r);
            assert!(s.is_power_of_two(), "stride {s} 必须是 2 的幂");
            assert_eq!(N_FINE % s, 0, "stride 必须整除 N_FINE");
            let segs = CornerTable::segs_of(s);
            assert_eq!(segs * s, N_FINE, "segs × stride 必须恰好覆盖整张表");
            assert!(segs >= 2, "segs 下限 2（r={r}）");
        }
    }

    #[test]
    fn arc_step_stays_near_target() {
        // 弧距 = π/2 × r / segs，应落在 [1, 2×ARC_STEP_PX] 内——除非段数已封顶
        // （stride 最小为 1 ⇒ segs = N_FINE，半径再大也只能变粗）。
        let t = table();
        for r in [2.0_f32, 3.0, 5.0, 8.0, 13.0, 26.0, 100.0] {
            let stride = t.stride_for(r);
            let segs = CornerTable::segs_of(stride) as f32;
            let step = std::f32::consts::FRAC_PI_2 * r / segs;
            assert!(
                (1.0..=2.0 * ARC_STEP_PX).contains(&step) || stride == 1,
                "r={r} 弧距 {step} 越界（stride={stride}）"
            );
        }
    }

    #[test]
    fn segs_grow_with_radius() {
        let t = table();
        let mut prev = 0;
        for r in [2.0_f32, 4.0, 8.0, 16.0, 64.0] {
            let segs = CornerTable::segs_of(t.stride_for(r));
            assert!(segs >= prev, "段数应随半径单调不减（r={r}）");
            prev = segs;
        }
        assert_eq!(CornerTable::segs_of(t.stride_for(1e6)), N_FINE, "大半径封顶");
    }

    #[test]
    fn feather_points_land_exactly_on_outline_points() {
        // 关键不变量：羽化环每点都精确落在硬体轮廓的某个点上（否则带子错位）。
        let t = table();
        for r in [1.0_f32, 2.0, 5.0, 12.0, 40.0, 200.0] {
            let segs = CornerTable::segs_of(t.stride_for(r));
            let fs = segs.min(FEATHER_SEGS_MAX);
            assert_eq!(segs % fs, 0, "feather_segs 必须整除 segs（r={r}）");
            for k in 0..=fs {
                assert_eq!(k * segs % fs, 0, "映射必须为整数（k={k}）");
                assert!(corner_inner_base(segs, fs, k) <= segs);
            }
            assert_eq!(corner_inner_base(segs, fs, 0), 0);
            assert_eq!(corner_inner_base(segs, fs, fs), segs);
        }
    }

    #[test]
    fn outline_winds_clockwise_and_matches_quad_convention() {
        // 硬体扇形与四边形约定必须同向，否则 cull 状态下圆角矩形会整块消失。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_rect(&mut v, &mut tr, &t, spec(60.0, 36.0, 8.0));
        for tri in &tr {
            assert!(cross(&v, tri) > 0.0, "三角形 {tri:?} 绕序与四边形约定相反");
        }
    }

    #[test]
    fn outline_start_is_the_left_mid_point() {
        // 轮廓第一个点 = TL 弧 t=0 = 左中点 (x, y + r)；最后一点 = BL 弧 t=π/2 = 左中点
        // 上方一点（保证闭合边在左侧直边上）。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let s = spec(60.0, 36.0, 8.0);
        push_rounded_rect(&mut v, &mut tr, &t, s);
        let first = v[0].pos;
        assert!((first[0] - s.rect.x).abs() < 1e-4, "首点应在左边（x={}）", first[0]);
        assert!(
            (first[1] - (s.rect.y + 8.0)).abs() < 1e-4,
            "首点应为左中点 y={}（期望 {}）",
            first[1],
            s.rect.y + 8.0
        );
    }

    #[test]
    fn degenerate_rect_produces_nothing() {
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let out = push_rounded_rect(&mut v, &mut tr, &t, spec(0.0, 10.0, 4.0));
        assert_eq!(out.verts, 0);
        assert!(v.is_empty() && tr.is_empty(), "零尺寸不得产生几何");
    }

    #[test]
    fn right_angle_falls_back_to_single_quad() {
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let out = push_rounded_rect(&mut v, &mut tr, &t, spec(10.0, 10.0, 0.0));
        assert_eq!(out.verts, 4, "直角应为 1 个四边形");
        assert_eq!(out.tris, 2);
        assert_well_formed(&v, &tr);
    }

    #[test]
    fn radius_clamps_to_capsule() {
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        // 半径远大于半高 → clamp 成胶囊；不得 panic、不得退化
        push_rounded_rect(&mut v, &mut tr, &t, spec(100.0, 20.0, 999.0));
        assert!(!v.is_empty() && !tr.is_empty());
        assert_well_formed(&v, &tr);
    }

    #[test]
    fn output_is_well_formed_across_sizes_and_radii() {
        let t = table();
        for (w, h, r) in [
            (40.0, 24.0, 6.0),
            (10.0, 10.0, 5.0),
            (200.0, 30.0, 15.0),
            (8.0, 8.0, 3.0),
            (17.0, 43.0, 9.0),
            (1.0, 1.0, 0.5),
        ] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            push_rounded_rect(&mut v, &mut tr, &t, spec(w, h, r));
            assert_well_formed(&v, &tr);
        }
    }

    #[test]
    fn vertex_count_is_bounded() {
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let r = 8.0;
        push_rounded_rect(&mut v, &mut tr, &t, spec(60.0, 36.0, r));
        let segs = CornerTable::segs_of(t.stride_for(r));
        let fs = segs.min(FEATHER_SEGS_MAX);
        // 硬体 4*(segs+1) + 1 中心点；羽化带 4*(fs+1)
        let bound = 4 * (segs as usize + 1) + 1 + 4 * (fs as usize + 1);
        assert!(v.len() <= bound, "顶点数 {} 超过上界 {bound}", v.len());
    }

    #[test]
    fn feather_shrinks_on_tiny_widgets() {
        assert_eq!(FEATHER_PX.min(4.0_f32.min(20.0) * 0.25), 1.0);
        assert!(FEATHER_PX.min(2.0_f32.min(20.0) * 0.25) < 1.0, "2px 宽应收紧羽化");
    }

    #[test]
    fn appending_offsets_indices_by_existing_vertex_count() {
        // 连续拼接两个矩形：第二个的索引必须相对其自身基点。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        for i in 0..2 {
            let mut s = spec(40.0, 24.0, 6.0);
            s.rect.x = i as f32 * 50.0;
            push_rounded_rect(&mut v, &mut tr, &t, s);
        }
        assert_well_formed(&v, &tr);
    }

    #[test]
    fn corners_are_carried_into_vertices() {
        // 「圆角 + 渐变」靠逐顶点色：四角不同的输入必须体现在顶点里
        // （这正是不能走实例单色的原因）。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_rect(&mut v, &mut tr, &t, spec(40.0, 24.0, 6.0));
        let alphas: Vec<f32> = v.iter().map(|x| x.color[3]).collect();
        assert!(alphas.contains(&1.0), "硬体顶点 alpha 必须为 1");
        assert!(alphas.contains(&0.0), "羽化环顶点 alpha 必须为 0");
        // 四角色确实进了硬体顶点（不再只有单一 tint）
        let colors: Vec<[f32; 4]> = v.iter().map(|x| x.color).collect();
        let red: [f32; 4] = Color::RED.into();
        assert!(colors.contains(&red), "TL 角色应出现在顶点里");
    }

    #[test]
    fn feather_ring_overlaps_hard_body_by_exactly_the_feather_width() {
        // 外环点 = 同方向 (radius + feather) 处：验证起点与硬体起点共线。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let s = spec(60.0, 36.0, 8.0);
        push_rounded_rect(&mut v, &mut tr, &t, s);
        let segs = CornerTable::segs_of(t.stride_for(8.0));
        let inner = v[0].pos; // 硬体 TL 起点 = 左中点
        let ring0 = v[(4 * (segs + 1) + 1) as usize].pos; // 羽化带第 0 个顶点
        assert!((inner[0] - ring0[0] - FEATHER_PX).abs() < 1e-4, "外环应再向左 1px");
        assert!((inner[1] - ring0[1]).abs() < 1e-4, "外环与内点同 y");
    }

    #[test]
    fn tess_cache_reuses_one_table() {
        let mut cache = TessCache::default();
        let a = cache.table();
        let b = cache.table();
        assert!(Rc::ptr_eq(&a, &b), "同一缓存必须复用同一张表");
        assert_eq!(a.pts.len(), N_FINE as usize + 1, "表含两端点");
    }

    // ─── 圆角环带（边框） ────────────────────────────────────

    #[test]
    fn ring_right_angle_matches_four_rect_strips() {
        // r = 0 走专用路径：四条轴对齐矩形条（与 `border_rects` 一致），
        // 而不是让圆弧带子在零半径下退化成重合点。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let out = push_rounded_ring(
            &mut v,
            &mut tr,
            &t,
            Rect::new(0.0, 0.0, 40.0, 20.0),
            0.0,
            2.0,
            Color::WHITE,
        );
        assert_eq!(out.verts, 16, "四条矩形条 = 4×4 顶点");
        assert_eq!(out.tris, 8);
        assert_well_formed(&v, &tr);
    }

    #[test]
    fn ring_with_radius_is_well_formed_and_clockwise() {
        let t = table();
        for (w, h, r, bw) in [
            (40.0, 20.0, 8.0, 2.0),
            (16.0, 16.0, 4.0, 1.0),
            (60.0, 60.0, 30.0, 3.0),
            // 边框宽 ≥ 半径 ⇒ 内角为直角（CSS 语义），不得产生零面积三角形
            (20.0, 20.0, 3.0, 5.0),
            (10.0, 10.0, 5.0, 5.0),
        ] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            push_rounded_ring(&mut v, &mut tr, &t, Rect::new(2.0, 3.0, w, h), r, bw, Color::WHITE);
            assert_well_formed(&v, &tr);
            for tri in &tr {
                assert!(cross(&v, tri) > 0.0, "环带三角形 {tri:?} 绕序反了");
            }
        }
    }

    #[test]
    fn ring_vertices_are_bounded() {
        // 环带 = 内外两圈轮廓：2 × 4 × (segs + 1) 顶点，segs ≤ N_FINE。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_ring(
            &mut v,
            &mut tr,
            &t,
            Rect::new(0.0, 0.0, 100.0, 40.0),
            12.0,
            1.0,
            Color::WHITE,
        );
        let segs = CornerTable::segs_of(t.stride_for(12.0)) as usize;
        assert!(v.len() <= 2 * 4 * (segs + 1), "环带顶点数 {} 超界", v.len());
    }

    #[test]
    fn ring_degenerate_inputs_produce_nothing() {
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        // 零宽 / 零高 / 零边框宽都要安全返回
        for (w, h, bw) in [(0.0, 20.0, 2.0), (20.0, 0.0, 2.0), (20.0, 20.0, 0.0)] {
            let out = push_rounded_ring(
                &mut v,
                &mut tr,
                &t,
                Rect::new(0.0, 0.0, w, h),
                4.0,
                bw,
                Color::WHITE,
            );
            assert_eq!(out.verts, 0);
        }
        assert!(v.is_empty() && tr.is_empty());
    }
}
