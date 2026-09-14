//! **圆角矩形 CPU 镶嵌**（硬体 + 边缘羽化带）——不依赖任何着色器改动。
//!
//! # 为什么不改着色器
//!
//! 抗锯齿不靠 SDF，靠**光栅化器对顶点 alpha 的线性插值**：
//! 硬体轮廓 `alpha = 1`，同心的外圈轮廓 `alpha = 0`，两者沿**整圈**配成带状三角形后
//! 自然插出 `1 → 0` 的过渡——与 `RStates` 默认的直通 Alpha 混合配合即为羽化边缘。
//! 因此 `sprite.wgsl` 与 `InstanceData` **零改动**，也不需要第二条管线
//! （`CustomDraw` 是合批屏障且拿不到 `PassContext`）。
//!
//! 梯度**以几何边缘为中心**：硬体内缩 `f/2`、外环外扩 `f/2`（`f` = 羽化宽），
//! 于是渲染出的视觉尺寸恒等于给定矩形。只向外扩会让每个矩形胖 `f` 像素，
//! 相邻矩形重叠处会多混一次 alpha 而露出一条缝。
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
//! # 一圈轮廓 + 整圈带状化（直边的关键）
//!
//! 一圈轮廓 = 4 角 × `(segs + 1)` 个点，**角的末点与下一角的首点在全局点号上相邻**，
//! 两者之间就是那条直边。所以「两圈轮廓之间沿整圈推进成带」（[`push_band`]）会自动
//! **覆盖四条直边**；反过来按角分别成带就会漏掉它们——那正是"边框只剩 4 个圆角孤岛、
//! 四条边整个消失"与"羽化只在角上有效"的成因。
//!
//! 正因如此，参与带状化的两圈必须用**同一套** `stride`/`segs`（逐点对应），
//! 顶点数由半径统一决定，不再有"羽化环用更粗段数"的额外旋钮。
//!
//! # 索引不缓存、不共用
//!
//! 索引与顶点**同源**（同一个抽样循环、同一个轮廓长度），每次内联构造并平移基址。
//! 这是**正确性**要求而非性能选择：若两者各算一遍，任何「点数 / 是否闭合成环」的
//! 判断偏差都会产生错乱三角形，且**不会 panic**，只是画面错。
//!
//! 也**不使用静态网格**：静态网格把几何冻结在 GPU 缓冲里，与 UI「每帧重新录制」
//! 的立即模式冲突（窗口内容一变就得重建注册网格，还要管生命周期）；且羽化带是
//! 亚像素级的软边、**不可被实例缩放**，本来就必须逐矩形产生——省下的只有硬体那一半，
//! 不足以抵偿注册与生命周期管理的复杂度。
//!
//! # 顶点绕序
//!
//! 轮廓沿**屏幕顺时针**（Y 向下）绕行：左中 → 左上 → 右上 → 右中 → 右下 → 左下 →
//! 回到左中。硬体扇形三角形 (轮廓ᵢ, 轮廓ᵢ₊₁, 中心) 与四边形的
//! (TL, TR, BR) / (BR, BL, TL) 约定同向（叉积同号），因此 `cull` 状态下的可见性
//! 与旧四边形路径一致。带状三角形 `(a₀, b₁, a₁)` / `(a₀, b₀, b₁)`（`a` = 半径较小
//! 的那圈）与之同向——直边段同样成立（有单测断言整圈叉积为正）。

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

/// 默认边缘羽化宽度（**逻辑像素**）——见 [`crate::style::Theme::feather`]。
/// 1 逻辑像素 ≈ 标准 1px 抗锯齿。
pub(crate) const DEFAULT_FEATHER: f32 = 1.0;

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
    /// **边缘羽化宽度**（物理像素，0 = 不羽化 ⇒ 硬边）。来自
    /// [`crate::style::Theme::feather`]（逻辑像素）× DPI。
    ///
    /// 梯度以几何边缘为中心（硬体内缩 `f/2`、外环外扩 `f/2`），因此视觉尺寸恒等于
    /// `rect`；`1.0` 物理像素 ≈ 标准 1px 抗锯齿，调大即"更软"。
    pub feather: f32,
    /// 四角色 `[TL, TR, BL, BR]`。
    ///
    /// **必须支持四角各异**——这正是「圆角 + 渐变」需要逐顶点色、不能走实例单色的原因：
    /// 硬体四角各取本角颜色，中心取四角均值，光栅化器在扇形三角形内做重心插值。
    pub corners: [Color; 4],
    /// **采样 UV**（所有顶点同值）。
    ///
    /// ⚠ **必须落在白纹理 region 内**。绝不能图省事填 `(0, 0)`：UI 的图形与字形共用同一张
    /// 图集页，`(0, 0)` 是**字形页左上角**，采到的是某个字形的像素（通常 alpha = 0）
    /// ⇒ 整块圆角矩形变成透明/乱码，看起来就是"背景完全消失"。
    ///
    /// 这个字段是**故意的**：让"忘了给 UV"变成编译错误，而不是一个只能靠肉眼发现的
    /// 静默渲染错误。调用方传白纹理 region 的**中心**
    /// （`white_uv_tl + white_uv_wh * 0.5`，见 `QuadCollector::white_uv_center`）。
    pub uv: [f32; 2],
}

/// 镶嵌结果：本次追加的顶点数与三角形数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TessOutput {
    pub verts: usize,
    pub tris: usize,
}

/// 一个圆角：圆心 + 半径 + 该角弧的两条**单位基向量**。
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
///
/// ⚠ **关键性质**：角的末点（`k = segs`）与**下一角的首点**（`k = 0`）在全局点号上相邻，
/// 中间那段直边直接由"沿整圈推进的带状化"覆盖——**不要**按角分别成带，那会漏掉
/// 四条直边（曾经的表现：边框只剩 4 个圆角孤岛）。
#[derive(Debug, Clone, Copy)]
struct Corner {
    center: Vec2,
    radius: f32,
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

/// 宽度 ≤ 0 时视为"不羽化"。太小的圆角放不下两个同心轮廓 ⇒ 也退回不羽化
/// （`MIN_AA_RADIUS`）：半径 < 1px 的圆角本来就看不出锯齿。
const MIN_AA_RADIUS: f32 = 0.5;

/// 写入**一圈圆角轮廓**（4 角 × `segs + 1` 点），返回该圈首顶点的下标。
///
/// `colors` 是四角色（支持四角渐变）；`alpha` 覆盖第 4 分量——羽化就是靠"硬体
/// `alpha = 1` / 外环 `alpha = 0`"由光栅化器插值出过渡实现的。
fn push_outline(
    verts: &mut Vec<VertexP3U2C4>,
    table: &CornerTable,
    stride: u32,
    segs: u32,
    corners: &[Corner; 4],
    colors: [Color; 4],
    alpha: f32,
    uv: [f32; 2],
) -> u16 {
    let start = verts.len() as u16;
    for (c, col) in corners.iter().zip(colors) {
        let mut a: [f32; 4] = col.into();
        a[3] = alpha;
        for k in 0..=segs {
            let (cos_t, sin_t) = table.sample(stride, k);
            let pos = c.center + c.dir(cos_t, sin_t) * c.radius;
            verts.push(VertexP3U2C4 { pos: [pos.x, pos.y, 0.0], uv, color: a });
        }
    }
    start
}

/// **两圈同心轮廓之间的带状几何**，沿**整圈**推进（`n = 4 × (segs + 1)`）。
///
/// `inner` 必须是**半径较小**的那一圈（`outer` 较大）——绕序靠这个前提成立，
/// 与硬体扇形同为屏幕顺时针。
///
/// 沿整圈推进（`i → (i+1) % n`）而不是逐角成带，是四条**直边**被覆盖的唯一原因：
/// 角的末点与下一角的首点在全局点号上相邻，那一步天然跨过直边。
fn push_band(tris: &mut Vec<Tri>, inner: u16, outer: u16, n: u16) {
    for i in 0..n {
        let j = (i + 1) % n;
        let (a0, a1) = (inner + i, inner + j);
        let (b0, b1) = (outer + i, outer + j);
        tris.push([a0, b1, a1]);
        tris.push([a0, b0, b1]);
    }
}

/// 各边**外扩** `d`（`d < 0` = 内缩）；正数变大，别搞反。
#[inline]
fn grow(r: Rect, d: f32) -> Rect {
    Rect::new(r.x - d, r.y - d, r.w + d * 2.0, r.h + d * 2.0)
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
        radius,
        a,
        b,
        color: colors[0],
    }; 4];
    for (i, slot) in out.iter_mut().enumerate() {
        slot.center = centers[i];
        slot.radius = radius;
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

/// 直角矩形（`radius == 0`）：一个四边形，四角各异颜色由光栅化器双线性插值。
///
/// 顶点顺序 `[TL, TR, BL, BR]`，索引与 `QUAD_TRI_INDICIES` 同约定。
fn push_plain_quad(
    verts: &mut Vec<VertexP3U2C4>,
    tris: &mut Vec<Tri>,
    rect: Rect,
    corners: [Color; 4],
    uv: [f32; 2],
) -> TessOutput {
    let Rect { x, y, w, h } = rect;
    let base = verts.len() as u16;
    let [tl, tr, bl, br] = corners.map(Into::<[f32; 4]>::into);
    verts.push(VertexP3U2C4 { pos: [x, y, 0.0], uv, color: tl });
    verts.push(VertexP3U2C4 { pos: [x + w, y, 0.0], uv, color: tr });
    verts.push(VertexP3U2C4 { pos: [x, y + h, 0.0], uv, color: bl });
    verts.push(VertexP3U2C4 { pos: [x + w, y + h, 0.0], uv, color: br });
    tris.push([base, base + 1, base + 3]);
    tris.push([base + 3, base + 2, base]);
    TessOutput { verts: 4, tris: 2 }
}

/// **把一个圆角矩形镶嵌成三角形**（硬体 + 羽化带），追加进 `verts` / `tris`。
///
/// - 顶点坐标为**调用方给定的坐标系**（`spec.rect` 所在空间）。UI 传窗口局部物理像素，
///   窗口级变换仍由批次实例应用（顶点缓存不因窗口 FX 失效）。
/// - 追加到非空缓冲时索引按**现有顶点数**自动偏移 ⇒ 可连续拼接多个矩形。
/// - 半径 clamp 成胶囊（与 CSS / egui 语义一致）。
///
/// # 羽化（抗锯齿）怎么做的
///
/// **不改着色器**：硬体轮廓 `alpha = 1`，同心的外圈轮廓 `alpha = 0`，两者沿**整圈**
/// （含四条直边）配成带状三角形，由光栅化器插值出 `1 → 0` 的过渡。
///
/// 梯度**以几何边缘为中心**：硬体在 `rect` 内缩 `f/2`、外环在外扩 `f/2`，于是
/// 视觉尺寸恒等于 `rect`（若只向外扩，每个矩形都会胖 `f` 像素，相邻矩形会互相
/// 叠出一条缝）。
///
/// `f = spec.feather`（物理像素，由 `Theme::feather` 逻辑值 × DPI 而来）；`f <= 0`
/// 或半径小到放不下两个同心轮廓时不羽化（硬体 = 原矩形，半径原样）。
pub(crate) fn push_rounded_rect(
    verts: &mut Vec<VertexP3U2C4>,
    tris: &mut Vec<Tri>,
    table: &CornerTable,
    spec: RoundedRectSpec,
) -> TessOutput {
    let Rect { w, h, .. } = spec.rect;
    if w <= 0.0 || h <= 0.0 {
        return TessOutput { verts: 0, tris: 0 };
    }
    let radius = spec.radius.clamp(0.0, w.min(h) * 0.5);
    if radius <= 0.0 {
        return push_plain_quad(verts, tris, spec.rect, spec.corners, spec.uv);
    }
    let uv = spec.uv;

    // 羽化宽：小控件自动收紧（避免糊成一团），且必须放得下两个同心轮廓。
    let f = spec.feather.min(w.min(h) * 0.25);
    let aa = f > 0.0 && radius - f * 0.5 >= MIN_AA_RADIUS;

    let verts_before = verts.len();
    let tris_before = tris.len();

    // 硬体（alpha = 1）与外环（alpha = 0）都锚在 `rect` 的边缘上。
    let half = if aa { f * 0.5 } else { 0.0 };
    let hard_rect = grow(spec.rect, -half);
    let hard_radius = (radius - half).max(0.0);
    let hard = corners_of(hard_rect, hard_radius, spec.corners);
    let stride = table.stride_for(radius + half);
    let segs = CornerTable::segs_of(stride);
    let n = (4 * (segs + 1)) as u16;

    let hard_start = push_outline(verts, table, stride, segs, &hard, spec.corners, 1.0, uv);
    let hard_len = (verts.len() as u16) - hard_start;
    debug_assert_eq!(hard_len, n);

    // ── 硬体填充：以矩形中心为轴的扇形三角化 ──
    // 轮廓天然闭合（最后一个点连回第一个点，跨过四条直边）。四角色各异 ⇒ 中心取
    // 四角均值，扇形内做重心插值（矩形内部本来就是双线性场，近似足够）。
    let mut center_col = [0.0f32; 4];
    for c in &hard {
        let a: [f32; 4] = c.color.into();
        for (dst, s) in center_col.iter_mut().zip(a) {
            *dst += s * 0.25;
        }
    }
    let center_idx = verts.len() as u16;
    verts.push(VertexP3U2C4 {
        pos: [
            spec.rect.x + spec.rect.w * 0.5,
            spec.rect.y + spec.rect.h * 0.5,
            0.0,
        ],
        uv,
        color: center_col,
    });
    for i in 0..n {
        let a = hard_start + i;
        let b = hard_start + (i + 1) % n;
        tris.push([a, b, center_idx]);
    }

    // ── 羽化带：外环 alpha = 0，沿**整圈**与硬体配成带子（含直边）──
    if aa {
        let outer_rect = grow(spec.rect, half);
        let outer = corners_of(outer_rect, radius + half, spec.corners);
        let outer_start =
            push_outline(verts, table, stride, segs, &outer, spec.corners, 0.0, uv);
        // `hard` 半径更小 ⇒ 是 inner。
        push_band(tris, hard_start, outer_start, n);
    }

    TessOutput {
        verts: verts.len() - verts_before,
        tris: tris.len() - tris_before,
    }
}

/// **圆角环带**（圆角矩形的边框）：外轮廓与内轮廓之间的一圈带子，**含四条直边**。
///
/// 前提：`inner_rect = rect` 各边内缩 `width`、且 `r_inner = max(0, r_outer - width)`
/// ——此时两轮廓的每对角弧**同心**，带子不会自交（与 CSS `border-radius`
/// 的内侧半径规则一致）。
///
/// # 四条直边
///
/// 带子沿**整圈**推进（角的末点与下一角的首点在全局点号上相邻）⇒ 直边自动被覆盖。
/// 早期实现按角分别成带，直边完全没有几何——表现是"边框只剩 4 个圆角孤岛，
/// 四条边整个消失"。
///
/// # 两侧都做羽化
///
/// 边框有**两条**可见边界（外侧轮廓与内侧轮廓），两侧都要软边，否则细边框的
/// 内侧会是一条硬邦邦、锯齿明显的线。因此最多四圈同心轮廓：
///
/// | 圈 | 半径 | alpha |
/// |---|---|---|
/// | 外羽化（`A`） | `r_outer + f/2` | 0 |
/// | 外轮廓（`B`） | `r_outer` | 1 |
/// | 内轮廓（`C`） | `r_inner` | 1 |
/// | 内羽化（`D`） | `r_inner - f/2` | 0 |
///
/// 三组带子 `A-B` / `B-C` / `C-D`。`f <= 0` 时只留 `B-C`。
///
/// # 退化
///
/// - `r_outer == 0`（直角边框）走**四条轴对齐矩形条**的专用路径（16 顶点），
///   不让圆弧带子在零半径下退化。
/// - `r_inner` 太小时（边框宽 ≥ 外半径）内角即直角；此时内圈的 `segs + 1` 个点
///   会重合为一点，`A-B`/`B-C` 的三角形会退化 ⇒ 把内半径夹到 [`MIN_AA_RADIUS`]
///   并把直边仍在的 `C-D` 内羽化跳过。
///
/// 之所以要这个原语：`push_panel_like` 早期用"外圈 border 色实心圆角 + 内圈背景
/// 实心圆角"两块叠加，圆角处的抗锯齿边缘会各混合一次；环带只画边界。
///
/// `uv` 同 [`RoundedRectSpec::uv`]：**必须落在白纹理 region 内**。
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_rounded_ring(
    verts: &mut Vec<VertexP3U2C4>,
    tris: &mut Vec<Tri>,
    table: &CornerTable,
    rect: Rect,
    radius: f32,
    width: f32,
    feather: f32,
    color: Color,
    uv: [f32; 2],
) -> TessOutput {
    let Rect { w, h, .. } = rect;
    if w <= 0.0 || h <= 0.0 || width <= 0.0 {
        return TessOutput { verts: 0, tris: 0 };
    }
    let width = width.min(w.min(h) * 0.5);
    let ro = radius.clamp(0.0, w.min(h) * 0.5);
    let inner_rect = rect.shrink(width);

    let verts_before = verts.len();
    let tris_before = tris.len();

    // 边框宽 ≥ 半边尺寸 ⇒ 内轮廓塌缩（甚至反向）。此时语义就是"整个盒子都是边框"，
    // 直接画一块实心圆角矩形收工——不这么做的话 `corners_of` 会拿到尺寸为 0 的矩形，
    // 四个角心相对顺序颠倒 ⇒ 内轮廓变成**逆时针**，带状三角形绕序整体翻反。
    if inner_rect.w <= 0.0 || inner_rect.h <= 0.0 {
        return push_rounded_rect(
            verts,
            tris,
            table,
            RoundedRectSpec { rect, radius, feather, corners: [color; 4], uv },
        );
    }
    let ri_raw = (ro - width).max(0.0).min(inner_rect.w.min(inner_rect.h) * 0.5);

    // 直角边框：四条轴对齐矩形条（圆弧带子在零半径下会退化成重合点）。
    if ro <= 0.0 {
        let mut col: [f32; 4] = color.into();
        col[3] = 1.0;
        let push = |r: Rect, verts: &mut Vec<VertexP3U2C4>, tris: &mut Vec<Tri>| {
            let b = verts.len() as u16;
            for p in [
                (r.x, r.y),
                (r.x + r.w, r.y),
                (r.x, r.y + r.h),
                (r.x + r.w, r.y + r.h),
            ] {
                verts.push(VertexP3U2C4 { pos: [p.0, p.1, 0.0], uv, color: col });
            }
            tris.push([b, b + 1, b + 3]);
            tris.push([b + 3, b + 2, b]);
        };
        for r in crate::draw::border_rects(&rect, width) {
            if r.w > 0.0 && r.h > 0.0 {
                push(r, verts, tris);
            }
        }
        return TessOutput {
            verts: verts.len() - verts_before,
            tris: tris.len() - tris_before,
        };
    }

    // 内半径太小 ⇒ 内角是直角。夹到一个可见但非零的小半径，避免内圈点重合导致
    // 零面积三角形（视觉上与直角无异）。
    let ri = ri_raw.max(MIN_AA_RADIUS.min(ro * 0.5));
    let f = feather.max(0.0).min(w.min(h) * 0.25);
    let half = f * 0.5;
    // 内羽化需要 `ri - half` 仍是有效半径（否则跳过内羽化，只保外羽化）。
    let inner_aa = half > 0.0 && ri - half >= MIN_AA_RADIUS;
    let outer_aa = half > 0.0;

    let stride = table.stride_for(ro + half);
    let segs = CornerTable::segs_of(stride);
    let n = (4 * (segs + 1)) as u16;
    let flat = [color; 4];

    // 由外向内写：A（外羽化，0）→ B（外轮廓，1）→ C（内轮廓，1）→ D（内羽化，0）。
    let a_start = if outer_aa {
        let cs = corners_of(grow(rect, half), ro + half, flat);
        Some(push_outline(verts, table, stride, segs, &cs, flat, 0.0, uv))
    } else {
        None
    };
    let b_start = push_outline(verts, table, stride, segs, &corners_of(rect, ro, flat), flat, 1.0, uv);
    let c_start = push_outline(
        verts,
        table,
        stride,
        segs,
        &corners_of(inner_rect, ri, flat),
        flat,
        1.0,
        uv,
    );
    let d_start = if inner_aa {
        let cs = corners_of(grow(inner_rect, -half), (ri - half).max(0.0), flat);
        Some(push_outline(verts, table, stride, segs, &cs, flat, 0.0, uv))
    } else {
        None
    };

    // B-C 是边框本体；A-B / C-D 是两条羽化斜坡。
    push_band(tris, c_start, b_start, n);
    if let Some(a) = a_start {
        push_band(tris, b_start, a, n);
    }
    if let Some(d) = d_start {
        push_band(tris, d, c_start, n);
    }

    TessOutput {
        verts: verts.len() - verts_before,
        tris: tris.len() - tris_before,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用采样 UV（**刻意不是 `(0,0)`**：`(0,0)` 会采到字形图集页左上角的字形像素，
    /// 这正是曾让所有镶嵌图形整块变透明的那个 bug）。
    const TEST_UV: [f32; 2] = [0.25, 0.75];

    fn table() -> CornerTable {
        CornerTable::build()
    }

    fn spec(w: f32, h: f32, r: f32) -> RoundedRectSpec {
        RoundedRectSpec {
            rect: Rect::new(3.0, 5.0, w, h),
            radius: r,
            feather: DEFAULT_FEATHER,
            corners: [Color::RED, Color::GREEN, Color::BLUE, Color::YELLOW],
            uv: TEST_UV,
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
    fn feather_gradient_is_centered_on_the_geometry_edge() {
        // 羽化以几何边缘为中心：硬体内缩 f/2、外环外扩 f/2 ⇒ 视觉尺寸恒等于 rect。
        // （若只向外扩，每个矩形会胖 f 像素，相邻矩形重叠处多混一次 alpha 露出缝。）
        let t = table();
        let rect = Rect::new(0.0, 0.0, 60.0, 40.0);
        let f = 4.0;
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_rect(
            &mut v,
            &mut tr,
            &t,
            RoundedRectSpec { rect, radius: 10.0, feather: f, corners: [Color::WHITE; 4], uv: TEST_UV },
        );
        let xs: Vec<f32> = v.iter().map(|x| x.pos[0]).collect();
        let (min, max) = (
            xs.iter().copied().fold(f32::INFINITY, f32::min),
            xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        );
        assert!((min + f * 0.5).abs() < 1e-3, "最左顶点应是 rect.x - f/2，实际 {min}");
        assert!((max - (rect.w + f * 0.5)).abs() < 1e-3, "最右顶点应是 rect.w + f/2，实际 {max}");
        // 硬体（alpha = 1）的上沿在 rect.y + f/2，外环（alpha = 0）的上沿在 rect.y - f/2。
        // 注意直边**不做细分**，所以不能拿 x = 30 去筛顶点——要看 alpha 分组后的极值。
        let top = |alpha: f32| {
            v.iter()
                .filter(|x| x.color[3] == alpha)
                .map(|x| x.pos[1])
                .fold(f32::INFINITY, f32::min)
        };
        assert!(
            (top(1.0) - f * 0.5).abs() < 1e-3,
            "硬体上沿应在 +f/2，实际 {}",
            top(1.0)
        );
        assert!(
            (top(0.0) + f * 0.5).abs() < 1e-3,
            "外环上沿应在 -f/2，实际 {}",
            top(0.0)
        );
    }

    #[test]
    fn feather_zero_disables_the_ramp() {
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let mut s = spec(60.0, 36.0, 8.0);
        s.feather = 0.0;
        push_rounded_rect(&mut v, &mut tr, &t, s);
        assert!(v.iter().all(|x| x.color[3] == 1.0), "不羽化时全部顶点 alpha = 1");
        assert_well_formed(&v, &tr);
    }

    #[test]
    fn tiny_radius_skips_aa_instead_of_degenerating() {
        // 半径小到放不下两个同心轮廓时退回不羽化，而不是产生零面积三角形。
        let t = table();
        for (r, f) in [(0.3_f32, 1.0_f32), (0.5, 2.0), (0.4, 0.5)] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            let mut s = spec(40.0, 24.0, r);
            s.feather = f;
            push_rounded_rect(&mut v, &mut tr, &t, s);
            assert_well_formed(&v, &tr);
            assert!(
                v.iter().all(|x| x.color[3] == 1.0),
                "半径 {r} + 羽化 {f}：应退回不羽化（全 alpha = 1）"
            );
        }
    }

    #[test]
    fn outline_winds_clockwise_and_matches_quad_convention() {
        // 硬体扇形、羽化带、环带（含**四条直边**）都必须与四边形约定同向，
        // 否则 cull 状态下圆角矩形 / 边框会整块或整边消失。
        let t = table();
        for (w, h, r, f) in [(60.0, 36.0, 8.0, 1.0), (60.0, 36.0, 8.0, 3.0), (40.0, 40.0, 19.0, 0.0)] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            let mut s = spec(w, h, r);
            s.feather = f;
            push_rounded_rect(&mut v, &mut tr, &t, s);
            assert_well_formed(&v, &tr);
            for tri in &tr {
                assert!(cross(&v, tri) > 0.0, "矩形三角形 {tri:?} 绕序反了");
            }

            let mut v = Vec::new();
            let mut tr = Vec::new();
            push_rounded_ring(&mut v, &mut tr, &t, Rect::new(3.0, 5.0, w, h), r, 2.0, f, Color::WHITE, TEST_UV);
            assert_well_formed(&v, &tr);
            for tri in &tr {
                assert!(cross(&v, tri) > 0.0, "环带三角形 {tri:?} 绕序反了");
            }
        }
    }

    /// 半径 = `min(w, h) / 2` 且 `w == h` 时四段 90° 弧正好拼成整圆，**接缝处的点重合**
    /// ⇒ 会产生零面积三角形。几何上没错（GPU 上零面积三角形不产生片元），但必须
    /// 确认不 panic、不越界、不产生负面积（那才说明绕序真的反了）。
    #[test]
    fn exact_circle_radius_yields_only_zero_area_seams() {
        let t = table();
        for (r, bw, f) in [(20.0_f32, 1.0_f32, 1.0_f32), (30.0, 3.0, 1.0), (20.0, 0.0, 0.0)] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            push_rounded_rect(
                &mut v,
                &mut tr,
                &t,
                RoundedRectSpec {
                    rect: Rect::new(0.0, 0.0, 40.0, 40.0),
                    radius: r,
                    feather: f,
                    corners: [Color::WHITE; 4],
                    uv: TEST_UV,
                },
            );
            let mut v = Vec::new();
            let mut tr = Vec::new();
            push_rounded_ring(
                &mut v,
                &mut tr,
                &t,
                Rect::new(0.0, 0.0, 40.0, 40.0),
                r,
                bw,
                f,
                Color::WHITE,
                TEST_UV,
            );
            assert_well_formed(&v, &tr);
            for tri in &tr {
                assert!(cross(&v, tri) >= 0.0, "整圆接缝处不得出现负面积：{tri:?}");
            }
        }
    }

    /// **直边必须有几何**（回归：边框只剩 4 个圆角孤岛、四条边整个消失）。
    ///
    /// 判据：镶嵌结果的三角形**并集包围盒**必须覆盖整条边——若只画角，中点处会没有
    /// 覆盖。这里取四条边的中点，断言它落在某个三角形内（用重心坐标判定）。
    #[test]
    fn straight_edges_are_covered_by_geometry() {
        let t = table();
        let rect = Rect::new(0.0, 0.0, 60.0, 40.0);
        // 环带：边框宽 2 ⇒ 上边中点的中心线在 y = 1 处。
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_ring(&mut v, &mut tr, &t, rect, 8.0, 2.0, DEFAULT_FEATHER, Color::WHITE, TEST_UV);
        for p in [
            Vec2::new(30.0, 1.0),  // 上边
            Vec2::new(30.0, 39.0), // 下边
            Vec2::new(1.0, 20.0),  // 左边
            Vec2::new(59.0, 20.0), // 右边
        ] {
            assert!(
                point_in_tris(&v, &tr, p),
                "边框直边中点 {p:?} 未被任何三角形覆盖（直边几何缺失）"
            );
        }
        // 角上也要有（反面对照：不能因为覆盖直边而把角弄丢）——
        // 取 45° 方向边框带的中线：距圆心 7.5，方向 (-0.707, -0.707)。
        let c = Vec2::new(8.0, 8.0) + Vec2::new(-0.707, -0.707) * 7.5;
        assert!(point_in_tris(&v, &tr, c), "左上角弧应被覆盖（{c:?}）");

        // 圆角矩形：四条直边的**外侧**（边缘内 0.5px）必须被硬体覆盖，
        // 且边缘外 0.5px 处**不应**被 alpha=1 的硬体覆盖（那是羽化区）。
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_rect(
            &mut v,
            &mut tr,
            &t,
            RoundedRectSpec { rect, radius: 8.0, feather: DEFAULT_FEATHER, corners: [Color::WHITE; 4], uv: TEST_UV },
        );
        for p in [
            Vec2::new(30.0, 0.75),
            Vec2::new(30.0, 39.25),
            Vec2::new(0.75, 20.0),
            Vec2::new(59.25, 20.0),
        ] {
            assert!(point_in_tris(&v, &tr, p), "圆角矩形直边 {p:?} 未被覆盖");
        }
    }

    /// 点是否落在某个三角形的内部（重心坐标；边界算命中）。
    ///
    /// 镶嵌出的三角形绕序一致为正叉积（屏幕 Y 向下），因此内部点对三条边的叉积
    /// 都应为正（留 `-1e-3` 容差覆盖边界）。
    fn point_in_tris(v: &[VertexP3U2C4], tr: &[Tri], p: Vec2) -> bool {
        let q = |i: u16| Vec2::new(v[i as usize].pos[0], v[i as usize].pos[1]);
        tr.iter().any(|t| {
            let (a, b, c) = (q(t[0]), q(t[1]), q(t[2]));
            (b - a).perp_dot(p - a) >= -1e-3
                && (c - b).perp_dot(p - b) >= -1e-3
                && (a - c).perp_dot(p - c) >= -1e-3
        })
    }

    #[test]
    fn outline_start_is_the_left_mid_point() {
        // 轮廓第一个点 = TL 弧 t=0 = 硬体的左中点。硬体在 `rect` 内缩 f/2
        // ⇒ 左中点 = (rect.x + f/2, rect.y + r)。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let f = DEFAULT_FEATHER;
        let s = spec(60.0, 36.0, 8.0);
        push_rounded_rect(&mut v, &mut tr, &t, s);
        let first = v[0].pos;
        assert!(
            (first[0] - (s.rect.x + f * 0.5)).abs() < 1e-4,
            "首点应在硬体左边（x={}, 期望 {}）",
            first[0],
            s.rect.x + f * 0.5
        );
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
        // 硬体 4*(segs+1) + 1 中心点；羽化带与硬体**同段数**（整圈逐点对应）⇒ 再来一圈。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let (r, f) = (8.0_f32, DEFAULT_FEATHER);
        push_rounded_rect(&mut v, &mut tr, &t, spec(60.0, 36.0, r));
        let segs = CornerTable::segs_of(t.stride_for(r + f * 0.5));
        let bound = 2 * 4 * (segs as usize + 1) + 1;
        assert!(v.len() <= bound, "顶点数 {} 超过上界 {bound}", v.len());
    }

    #[test]
    fn feather_width_is_clamped_on_tiny_widgets() {
        // 小控件自动收紧羽化（避免糊成一团）：min(f, min(w,h)/4)。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let mut s = spec(2.0, 20.0, 1.0);
        s.feather = 4.0;
        push_rounded_rect(&mut v, &mut tr, &t, s);
        // 收紧后 f = 0.5 ⇒ 外环只在 rect 外 0.25px
        let min_x = v.iter().map(|x| x.pos[0]).fold(f32::INFINITY, f32::min);
        assert!(
            (min_x - (s.rect.x - 0.25)).abs() < 1e-3,
            "2px 宽控件应把羽化收紧到 0.5（实际最左 {min_x}）"
        );
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
    fn feather_ring_is_one_feather_width_outside_the_hard_body() {
        // 外环点与硬体同方向、半径相差 f（硬体 r - f/2、外环 r + f/2）⇒ 带子宽 = f。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let f = 2.0;
        let mut s = spec(60.0, 36.0, 8.0);
        s.feather = f;
        push_rounded_rect(&mut v, &mut tr, &t, s);
        let segs = CornerTable::segs_of(t.stride_for(8.0 + f * 0.5));
        let n = 4 * (segs + 1);
        let hard0 = v[0].pos; // 硬体 TL 起点 = 左中点
        let ring0 = v[(n + 1) as usize].pos; // 羽化带第 0 个顶点（跳过一个中心点）
        assert!((hard0[0] - ring0[0] - f).abs() < 1e-4, "外环应再向左 {f}px");
        assert!((hard0[1] - ring0[1]).abs() < 1e-4, "外环与内点同 y");
    }

    #[test]
    fn tess_cache_reuses_one_table() {
        let mut cache = TessCache::default();
        let a = cache.table();
        let b = cache.table();
        assert!(Rc::ptr_eq(&a, &b), "同一缓存必须复用同一张表");
        assert_eq!(a.pts.len(), N_FINE as usize + 1, "表含两端点");
    }

    // ─── 采样 UV（曾经让所有镶嵌图形整块变透明的 BUG） ────────

    #[test]
    fn every_vertex_carries_the_given_uv() {
        // 回归：`push_rounded_rect` 曾把 UV 写死成 (0,0)。图形与字形共用同一张图集页，
        // (0,0) 是**字形页左上角**——采到的是某个字形的像素（通常 alpha = 0），
        // 于是所有圆角矩形（窗口/面板/按钮背景）整块变透明 = "背景完全消失"。
        let t = table();
        let uv = [0.375, 0.625];
        for (w, h, r) in [(60.0, 36.0, 8.0), (30.0, 30.0, 0.0), (200.0, 20.0, 999.0)] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            let mut s = spec(w, h, r);
            s.uv = uv;
            push_rounded_rect(&mut v, &mut tr, &t, s);
            assert!(!v.is_empty());
            for x in &v {
                assert_eq!(x.uv, uv, "顶点 UV 必须是调用方给的采样点");
                assert_ne!(x.uv, [0.0, 0.0], "绝不能落到字形页左上角");
            }
        }
    }

    #[test]
    fn ring_vertices_carry_the_given_uv() {
        // 同上：圆角边框环带也必须带正确 UV（否则边框连同背景一起不可见）。
        let t = table();
        let uv = [0.375, 0.625];
        for (r, bw) in [(8.0, 2.0), (0.0, 1.0), (4.0, 9.0)] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            push_rounded_ring(
                &mut v,
                &mut tr,
                &t,
                Rect::new(0.0, 0.0, 40.0, 24.0),
                r,
                bw,
                DEFAULT_FEATHER,
                Color::WHITE,
                uv,
            );
            assert!(!v.is_empty());
            for x in &v {
                assert_eq!(x.uv, uv, "环带顶点 UV 必须是调用方给的采样点");
            }
        }
    }

    #[test]
    fn hard_body_alpha_is_one_and_feather_is_zero() {
        // 羽化抗锯齿的前提：硬体 alpha = 1、外环 alpha = 0（光栅化器插值出过渡）。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_rect(&mut v, &mut tr, &t, spec(60.0, 36.0, 8.0));
        let hard = v.iter().filter(|x| x.color[3] == 1.0).count();
        let feather = v.iter().filter(|x| x.color[3] == 0.0).count();
        assert!(hard > 0 && feather > 0);
        assert_eq!(hard + feather, v.len(), "只应有 alpha 1 与 0 两类顶点");
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
            DEFAULT_FEATHER,
            Color::WHITE,
            TEST_UV,
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
            // 刻意**不取** r = min(w,h)/2：那正好是整圆，接缝处会多出零面积三角形，
            // 由 `exact_circle_radius_yields_only_zero_area_seams` 单独覆盖。
            (60.0, 60.0, 28.0, 3.0),
            // 边框宽 ≥ 半径 ⇒ 内角为直角（CSS 语义），不得产生零面积三角形
            (20.0, 20.0, 3.0, 5.0),
            (10.0, 10.0, 4.0, 5.0),
        ] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            push_rounded_ring(
                &mut v,
                &mut tr,
                &t,
                Rect::new(2.0, 3.0, w, h),
                r,
                bw,
                DEFAULT_FEATHER,
                Color::WHITE,
                TEST_UV,
            );
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
            DEFAULT_FEATHER,
            Color::WHITE,
            TEST_UV,
        );
        let segs = CornerTable::segs_of(t.stride_for(12.0 + DEFAULT_FEATHER * 0.5)) as usize;
        // 外羽化 + 外/内轮廓 + 内羽化 ⇒ 4 圈（内羽化在 i - f/2 有效时才画）
        assert!(v.len() <= 4 * 4 * (segs + 1), "环带顶点数 {} 超界", v.len());
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
                DEFAULT_FEATHER,
                Color::WHITE,
                TEST_UV,
            );
            assert_eq!(out.verts, 0);
        }
        assert!(v.is_empty() && tr.is_empty());
    }
}
