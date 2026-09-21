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
use crate::draw::CornerRadius;

/// 细表取样点数（每个四分之一圆弧）。2 的幂 —— `stride` 取 2 的幂即可整除本值。
pub(crate) const N_FINE: u32 = 32;

/// 相邻轮廓顶点的**目标弧距**（物理像素）：越小越圆滑、顶点越多。
pub(crate) const ARC_STEP_PX: f32 = 2.0;

/// 默认边缘羽化宽度（**逻辑像素**）——见 [`crate::style::Theme::feather`]。
/// 1 逻辑像素 ≈ 标准 1px 抗锯齿。
pub(crate) const DEFAULT_FEATHER: f32 = 1.0;

/// 圆角软阴影的**影调段数**（同心轮廓圈数 − 1）。
///
/// 单段（内外两圈）的 alpha 是**线性**斜坡，边缘一圈能看出"硬边"；两段折线已足够接近
/// 高斯的观感（`alpha(t) = a·(1−t)²`），再多就只是顶点数——阴影是**窗口顶点缓存**里
/// 的静态几何，但每多一圈就多 `4·(segs+1)` 个顶点。
pub(crate) const SHADOW_STEPS: u32 = 4;

/// **圆角软阴影**（**顶点色**软阴影；无纹理、无着色器、不增 draw call）。
///
/// 从内轮廓（`rect`，颜色 = `color`）向外 `blur` 像素铺 [`SHADOW_STEPS`] 段同心圆角带，
/// 每段颜色 RGB 不变、**alpha 按二次曲线**（`a·(1−t)²`）渐隐到 0 ⇒ 光栅化器在段内做
/// 线性插值，整条影调是"二次折线"，落在 0 上的最外圈天然完成抗锯齿（不需要额外羽化圈）。
///
/// - `rect` = **本体矩形**（阴影的**内轮廓恒在本体边缘**：alpha 从本体边开始往外衰减，
///   **没有"等浓度平台"**——见下）；
/// - `blur` = 向外渐隐宽度；`offset` = 最外圈相对本体的偏移（光源方向的反向：光从上方来
///   ⇒ `(0, +3)`）——**偏移按圈数线性分摊**（第 `t` 圈偏 `offset·t`），于是投影整体向下
///   偏、上方更窄，而**本体边缘处浓度最高且立即开始衰减**；
/// - `radius` = 本体圆角（`0` 会被夹到 [`MIN_AA_RADIUS`]：直角窗口的投影走同一套弧表，
///   0.5px 的圆角在视觉上与直角无异，却能避免内圈点重合产生零面积三角形）；
/// - `blur <= 0`、`color` 全透明、或退化矩形 ⇒ 不产生任何几何。
///
/// ⚠ **不要**把"偏移"做成"把内轮廓整体下移"：那样本体下缘到内轮廓之间是一段**等浓度**
/// 暗带（本体底边像贴了一条硬黑边——"窗口阴影下方突出"）。本实现让偏移随圈数分摊，
/// 浓度从本体边缘单调下降，是正常的柔和投影。
///
/// 之所以用这个原语：窗口投影若用"大一圈的半透明实心圆角矩形"会得到一个**硬边**黑框，
/// 而用纹理 / 着色器模糊又违背本仓"UI 只走顶点色 + CPU 镶嵌"的路线。
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_rounded_shadow(
    verts: &mut Vec<VertexP3U2C4>,
    tris: &mut Vec<Tri>,
    table: &CornerTable,
    rect: Rect,
    radius: CornerRadius,
    blur: f32,
    offset: Vec2,
    color: Color,
    uv: [f32; 2],
) -> TessOutput {
    let (w, h) = (rect.w, rect.h);
    if w <= 0.0 || h <= 0.0 || blur <= 0.0 {
        return TessOutput { verts: 0, tris: 0 };
    }
    let base: [f32; 4] = color.into();
    if base[3] <= 0.0 {
        return TessOutput { verts: 0, tris: 0 };
    }
    let verts_before = verts.len();
    let tris_before = tris.len();

    // 半径先夹到放得下；直角走 `MIN_AA_RADIUS`（见函数文档）。
    let r0 = {
        let r = if radius.is_zero() {
            CornerRadius::all(MIN_AA_RADIUS)
        } else {
            radius
        };
        r.fit(w, h)
    };
    // 弧段数由**最外圈**半径决定（那一圈弧最长，需要的段数最多）。
    let r_out = r0.max() + blur;
    let stride = table.stride_for(r_out);
    let segs = CornerTable::segs_of(stride);
    let n = (4 * (segs + 1)) as u16;

    // 逐圈写入：第 i 圈（i = 0 = 本体边缘）外扩 `blur·t`、再按 `offset·t` 偏移，
    // alpha = a·(1−t)²（本体边缘 = a，最外圈 = 0）。
    let mut rings: Vec<u16> = Vec::with_capacity(SHADOW_STEPS as usize + 1);
    for i in 0..=SHADOW_STEPS {
        let t = i as f32 / SHADOW_STEPS as f32;
        let d = blur * t;
        let o = offset * t;
        let rr = Rect::new(
            rect.x - d + o.x,
            rect.y - d + o.y,
            w + d * 2.0,
            h + d * 2.0,
        );
        let rad = r0.map(|r| r + d).fit(rr.w, rr.h);
        let mut c = base;
        c[3] = base[3] * (1.0 - t) * (1.0 - t);
        let col = |_i: usize, _p: Vec2| c;
        rings.push(push_outline(
            verts,
            table,
            stride,
            segs,
            &corners_of(rr, rad),
            &|_p| uv,
            col,
        ));
    }
    // 相邻两圈之间成带：内圈在前（`push_band` 的绕序约定与边框环带一致）。
    for pair in rings.windows(2) {
        push_band(tris, pair[0], pair[1], n);
    }

    TessOutput {
        verts: verts.len() - verts_before,
        tris: tris.len() - tris_before,
    }
}

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
    /// **四角各自的圆角半径**（物理像素，逻辑半径已由调用方换算）。四角可以不同——
    /// 常见需求是"只圆上面两个角"（标签页 / 附着在工具栏下方的面板）。
    ///
    /// 镶嵌器内部按 CSS `border-radius` 规则把四角**等比收缩**（`tl + tr ≤ w` 等），
    /// 见 [`CornerRadius::fit`]。
    pub radius: CornerRadius,
    /// **边缘羽化宽度**（物理像素，0 = 不羽化 ⇒ 硬边）。来自
    /// [`crate::style::Theme::feather`]（逻辑像素）× DPI。
    ///
    /// 梯度以几何边缘为中心（硬体内缩 `f/2`、外环外扩 `f/2`），因此视觉尺寸恒等于
    /// `rect`；`1.0` 物理像素 ≈ 标准 1px 抗锯齿，调大即"更软"。
    pub feather: f32,
    /// 四角色 `[TL, TR, BL, BR]`（⚠ 与 `radius` 的具名字段顺序不同，注意别串）。
    ///
    /// **必须支持四角各异**——这正是「圆角 + 渐变」需要逐顶点色、不能走实例单色的原因。
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

/// **双线性取色**：按点在 `rect` 中的归一化位置 `(u, v)` 插值四角色。
///
/// 这是"背景渐变靠 lerp"的**唯一**取色入口——**每一个**顶点（含圆角弧上的）都用自己的
/// 位置算色，而不是整段弧直接带"本角颜色"。
///
/// 为什么必须这样：若弧上顶点直接带本角颜色，渐变的**两端会被钉在弧的跨度上**。
/// 例：200×36 的胶囊 + 半径 18 + 水平渐变 `L → R`，左侧 18px 全是纯 `L`、右侧 18px
/// 全是纯 `R`，整条 `L→R` 斜坡被压进中间 164px —— 肉眼就是"两端发平、渐变被拉长"。
/// 按位置 lerp 后，弧上的颜色恰好是该处应有的渐变值，整块颜色场与矩形渐变一致。
///
/// `rect` 退化（宽 / 高 ≤ 0）时回退到左上角色（调用方保证不会走到这里）。
#[inline]
fn bilinear_color(corners: [Color; 4], rect: Rect, p: Vec2) -> Color {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return corners[0];
    }
    let u = ((p.x - rect.x) / rect.w).clamp(0.0, 1.0);
    let v = ((p.y - rect.y) / rect.h).clamp(0.0, 1.0);
    let top = crate::draw::lerp_color(corners[0], corners[1], u);
    let bot = crate::draw::lerp_color(corners[2], corners[3], u);
    crate::draw::lerp_color(top, bot, v)
}

/// 写入**一圈圆角轮廓**（4 角 × `segs + 1` 点），返回该圈首顶点的下标。
///
/// `color_at(点序号, 位置)` 决定每个顶点的颜色（含 alpha）：背景传"按位置双线性取色"，
/// 羽化环传"复制主轮廓同序号点的颜色 + `alpha = 0`"（保证纯 alpha 斜坡、不夹带色偏），
/// 边框传固定色。
fn push_outline<F>(
    verts: &mut Vec<VertexP3U2C4>,
    table: &CornerTable,
    stride: u32,
    segs: u32,
    corners: &[Corner; 4],
    uv_at: &dyn Fn(Vec2) -> [f32; 2],
    color_at: F,
) -> u16
where
    F: Fn(usize, Vec2) -> [f32; 4],
{
    let start = verts.len() as u16;
    let mut i = 0usize;
    for c in corners {
        for k in 0..=segs {
            let (cos_t, sin_t) = table.sample(stride, k);
            let pos = c.center + c.dir(cos_t, sin_t) * c.radius;
            verts.push(VertexP3U2C4 {
                pos: [pos.x, pos.y, 0.0],
                uv: uv_at(pos),
                color: color_at(i, pos),
            });
            i += 1;
        }
    }
    start
}

/// 取已写入的 `[start, start+n)` 顶点色，把 alpha 覆盖为 `alpha`。
fn copy_colors(verts: &[VertexP3U2C4], start: u16, n: u16, alpha: f32) -> Vec<[f32; 4]> {
    verts[start as usize..start as usize + n as usize]
        .iter()
        .map(|v| {
            let mut c = v.color;
            c[3] = alpha;
            c
        })
        .collect()
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

// ─── 凸多边形（图标等任意形状）────────────────────────────────

/// **凸多边形 + 边缘羽化**（图标 / 箭头 / 勾选等任意形状）。
///
/// `points` 必须是**凸**多边形且按**屏幕顺时针**（Y 向下）给出——与圆角矩形同一套
/// 约定（扇形三角形 `(pᵢ, pᵢ₊₁, 重心)` 的叉积为正）。
///
/// # 羽化怎么做（与圆角矩形完全同一套机制）
///
/// 硬体 = 原多边形（`alpha = 1`），外环 = 每个顶点沿**角平分线**外扩 `feather`
/// （`alpha = 0`），两者沿**整圈**配成带状三角形（复用 [`push_band`]）⇒ 由光栅化器
/// 插值出 `1 → 0` 的过渡。同样**不改着色器**。
///
/// 顶点外扩量按**真 Minkowski 偏移**算：沿角平分线走 `feather / cos(θ)`（`θ` = 该顶点
/// 的角平分线与边法线的夹角），于是外环与每条边相距**恰好** `feather`（不是"每个顶点
/// 都走 feather"，那会让尖角处外环距离变远、羽化宽度不均）。尖角处按 `MAX_MITER`
/// 截断（避免长针状外环）。
pub(crate) fn push_convex(
    verts: &mut Vec<VertexP3U2C4>,
    tris: &mut Vec<Tri>,
    points: &[Vec2],
    feather: f32,
    color: Color,
    uv: [f32; 2],
) -> TessOutput {
    if points.len() < 3 {
        return TessOutput { verts: 0, tris: 0 };
    }
    // 与圆角矩形同一条约定：硬体的 alpha **就是**调用方给的 alpha（半透明图标必须真的
    // 半透明），羽化外环再从它降到 0。
    let col: [f32; 4] = color.into();
    let mut fade = col;
    fade[3] = 0.0;

    let verts_before = verts.len();
    let tris_before = tris.len();
    let n = points.len() as u16;

    // 硬体：轮廓 + 重心（扇形三角化的轴）。
    let hard_start = verts.len() as u16;
    for p in points {
        verts.push(VertexP3U2C4 { pos: [p.x, p.y, 0.0], uv, color: col });
    }
    let centroid = points.iter().fold(Vec2::ZERO, |a, p| a + *p) / points.len() as f32;
    let center_idx = verts.len() as u16;
    verts.push(VertexP3U2C4 { pos: [centroid.x, centroid.y, 0.0], uv, color: col });
    for i in 0..n {
        tris.push([hard_start + i, hard_start + (i + 1) % n, center_idx]);
    }

    // 外环（alpha = 0）：沿角平分线做真偏移。
    //
    // ⚠ **羽化宽必须按形状厚度夹一次**：图标常是"细条"（`Grip` 的三横在 12px 图标上
    // 只有 ~1.2px 高），若照搬 `Theme::feather`（1.0~1.5px），AA 斜坡会和形状本身的
    // 粗细同量级甚至更宽 ⇒ 整个笔画被"糊"成一片渐变（用户描述："图标像是近视一样"）。
    // 夹到**到边距离的一半**：斜坡最多占到形状厚度的一半，剩下仍是实心。
    let f = feather.max(0.0).min(min_edge_distance(points) * 0.5);
    if f > 0.0 {
        let outer_start = verts.len() as u16;
        for (i, p) in points.iter().enumerate() {
            // 相邻两条边的**单位外法线**。顺时针（Y 向下）时，边 `a → b` 的外法线
            // 是 `(dy, -dx)` 归一化（与 `CHECK_L` 等图标表的构造同一约定，有单测钉住）。
            let prev = points[(i + points.len() - 1) % points.len()];
            let next = points[(i + 1) % points.len()];
            let n_prev = outward_normal(*p - prev);
            let n_next = outward_normal(next - *p);
            let bis = n_prev + n_next;
            let bis = if bis.length_squared() < 1e-8 { n_next } else { bis.normalize() };
            // 沿平分线走多少才让"到两条边的距离"都等于 f：`f / cos(θ)`，`θ` 是平分线与
            // 法线的夹角。`dot` 就是 `cos θ`，下限 0.35 ≈ 70° 的半角（即尖角 20°）。
            let k = (1.0 / bis.dot(n_next).max(0.35)).min(MAX_MITER);
            let off = *p + bis * (f * k);
            verts.push(VertexP3U2C4 { pos: [off.x, off.y, 0.0], uv, color: fade });
        }
        push_band(tris, hard_start, outer_start, n);
    }

    TessOutput {
        verts: verts.len() - verts_before,
        tris: tris.len() - tris_before,
    }
}

/// 顺时针（Y 向下）多边形的边 `d` 的**单位外法线**。
#[inline]
fn outward_normal(d: Vec2) -> Vec2 {
    let l = d.length();
    if l < 1e-6 {
        Vec2::ZERO
    } else {
        Vec2::new(d.y, -d.x) / l
    }
}

/// 重心到各边的**最小距离**（≈ 形状的"半厚"；细条上就是半宽）。
///
/// 用途：[`push_convex`] 的羽化宽按它夹一次——AA 斜坡不能和形状本身一样宽，
/// 否则细笔画被糊成一片（"图标像是近视一样"）。
#[inline]
fn min_edge_distance(points: &[Vec2]) -> f32 {
    if points.len() < 3 {
        return 0.0;
    }
    let c = points.iter().fold(Vec2::ZERO, |a, p| a + *p) / points.len() as f32;
    let mut min = f32::INFINITY;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        let d = (b - a).normalize_or_zero();
        // 点到直线 ab 的距离（凸多边形内点到边的最短距离 = 垂距）。
        let n = Vec2::new(d.y, -d.x);
        min = min.min((c - a).dot(n).abs());
    }
    if min.is_finite() { min } else { 0.0 }
}

/// 尖角处外环的**斜接上限**（`miter limit`）：顶点最远只外扩 `feather × 此值`，
/// 避免锐角处外环被拉成长针（同 Canvas 的 `miterLimit` 语义）。
const MAX_MITER: f32 = 2.0;


/// 四角（屏幕顺时针 TL → TR → BR → BL）的圆心与基向量。
///
/// `radii` 已 clamp / [`CornerRadius::fit`] 过（四角可**各自不同**）。**不带颜色**——
/// 每个顶点的颜色由 [`push_outline`] 的 `color_at` 按**位置**算
/// （见 [`bilinear_color`]）；四角颜色是"颜色场"的参数，不是"角"的属性。
#[inline]
fn corners_of(rect: Rect, radii: CornerRadius) -> [Corner; 4] {
    let Rect { x, y, w, h } = rect;
    let (tl, tr, br, bl) = (radii.tl, radii.tr, radii.br, radii.bl);
    let centers = [
        Vec2::new(x + tl, y + tl),                 // TL
        Vec2::new(x + w - tr, y + tr),             // TR
        Vec2::new(x + w - br, y + h - br),         // BR
        Vec2::new(x + bl, y + h - bl),             // BL
    ];
    let rs = [tl, tr, br, bl];
    let mut a = Vec2::new(-1.0, 0.0);
    let mut b = Vec2::new(0.0, -1.0);
    let mut out = [Corner { center: Vec2::ZERO, radius: 0.0, a, b }; 4];
    for (i, slot) in out.iter_mut().enumerate() {
        slot.center = centers[i];
        slot.radius = rs[i];
        slot.a = a;
        slot.b = b;
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
    uv_at: &dyn Fn(Vec2) -> [f32; 2],
) -> TessOutput {
    let Rect { x, y, w, h } = rect;
    let base = verts.len() as u16;
    let [tl, tr, bl, br] = corners.map(Into::<[f32; 4]>::into);
    // UV 逐点：纯色（白纹理）四个角同值；贴图时四角各异——一个四边形即可精确承载
    // **仿射** UV 映射（拉伸 / 等比裁剪 / 居中都属此类）。
    let (vtl, vtr, vbl, vbr) = (
        uv_at(Vec2::new(x, y)),
        uv_at(Vec2::new(x + w, y)),
        uv_at(Vec2::new(x, y + h)),
        uv_at(Vec2::new(x + w, y + h)),
    );
    verts.push(VertexP3U2C4 { pos: [x, y, 0.0], uv: vtl, color: tl });
    verts.push(VertexP3U2C4 { pos: [x + w, y, 0.0], uv: vtr, color: tr });
    verts.push(VertexP3U2C4 { pos: [x, y + h, 0.0], uv: vbl, color: bl });
    verts.push(VertexP3U2C4 { pos: [x + w, y + h, 0.0], uv: vbr, color: br });
    tris.push([base, base + 1, base + 3]);
    tris.push([base + 3, base + 2, base]);
    TessOutput { verts: 4, tris: 2 }
}

/// **带子区域 UV 的矩形四边形**（贴图**平铺**的单块）：UV 起点 / 终点显式给出——
/// 平铺时每块恒取 `0..1`，边缘的**部分块**按比例截断（`uv1 < 1`）。
///
/// 返回的 `TessOutput` 恒为 `{ verts: 4, tris: 2 }`（尺寸退化时为 0）。
pub(crate) fn push_plain_uv(
    verts: &mut Vec<VertexP3U2C4>,
    tris: &mut Vec<Tri>,
    rect: Rect,
    corners: [Color; 4],
    uv0: Vec2,
    uv1: Vec2,
) -> TessOutput {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return TessOutput { verts: 0, tris: 0 };
    }
    let Rect { x, y, w, h } = rect;
    let base = verts.len() as u16;
    let [tl, tr, bl, br] = corners.map(Into::<[f32; 4]>::into);
    verts.push(VertexP3U2C4 { pos: [x, y, 0.0], uv: [uv0.x, uv0.y], color: tl });
    verts.push(VertexP3U2C4 { pos: [x + w, y, 0.0], uv: [uv1.x, uv0.y], color: tr });
    verts.push(VertexP3U2C4 { pos: [x, y + h, 0.0], uv: [uv0.x, uv1.y], color: bl });
    verts.push(VertexP3U2C4 { pos: [x + w, y + h, 0.0], uv: [uv1.x, uv1.y], color: br });
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
/// # 颜色（背景渐变）
///
/// 硬体的**每个**顶点（含圆角弧上的）都按自己在 `rect` 中的位置做**双线性取色**
/// （[`bilinear_color`]）——整块颜色场与矩形渐变一致。若弧上顶点直接带"本角颜色"，
/// 渐变两端会被钉在弧的跨度上（200px 宽的胶囊 + 半径 18，两侧各 18px 变成纯端色，
/// 整条斜坡被压进中间 164px，肉眼就是"两端发平、渐变被拉长"）。
///
/// 中心顶点取中心的双线性值（= 四角均值），扇形内部为重心插值——对纵向 / 横向这类
/// 可分离渐变是精确的，四角各异的对角渐变是可接受的近似。
///
/// 外环直接**复制**硬体同序号顶点的颜色、只把 alpha 置 0 ⇒ 羽化是纯 alpha 斜坡，
/// 不会夹带色偏。
pub(crate) fn push_rounded_rect(
    verts: &mut Vec<VertexP3U2C4>,
    tris: &mut Vec<Tri>,
    table: &CornerTable,
    spec: RoundedRectSpec,
) -> TessOutput {
    let uv = spec.uv;
    push_rounded_rect_uv(verts, tris, table, spec, &move |_p| uv)
}

/// 同 [`push_rounded_rect`]，但**每个顶点按自己的位置取 UV**（`uv_at`）——**贴图**用
/// （实心背景恒走 [`RoundedRectSpec::uv`] 的定值版本）。
///
/// 线性 UV 映射（拉伸 / 等比裁剪 / 居中，即 [`crate::draw::ImageFit`] 的前三种）在
/// 扇形三角化下的重心插值是**精确**的（仿射映射），因此贴图与圆角遮罩天然共存、
/// 零额外 draw call、零着色器改动（与「圆角 + 渐变」同一套机制）。
pub(crate) fn push_rounded_rect_uv(
    verts: &mut Vec<VertexP3U2C4>,
    tris: &mut Vec<Tri>,
    table: &CornerTable,
    spec: RoundedRectSpec,
    uv_at: &dyn Fn(Vec2) -> [f32; 2],
) -> TessOutput {
    let Rect { w, h, .. } = spec.rect;
    if w <= 0.0 || h <= 0.0 {
        return TessOutput { verts: 0, tris: 0 };
    }
    // 四角按 CSS 规则收缩（`tl + tr ≤ w` 等），保证弧互不重叠。
    // 直角（半径 0）的角也要参与带状化，否则两级轮廓的**点数不一致**、带状会错位；
    // 给它 0.5px 的下限（1× 下看不出圆，但点数与其它角一致）。
    if spec.radius.is_zero() {
        return push_plain_quad(verts, tris, spec.rect, spec.corners, uv_at);
    }
    let floor = |r: f32| if r > 0.0 { r.max(MIN_AA_RADIUS) } else { MIN_AA_RADIUS };
    // ⚠ 顺序：**先抬下限，再 `fit`**。反过来的话下限会把已经收缩好的半径又抬回超界值，
    // 于是"两角半径之和 > 边长"、角心次序颠倒、轮廓变逆时针 ⇒ 负面积三角形。
    let radii = spec.radius.map(floor).fit(w, h);

    // 羽化宽：小控件自动收紧（避免糊成一团）。
    let f = spec.feather.min(w.min(h) * 0.25);
    let aa = f > 0.0 && (radii.min() - f * 0.5) >= 0.0;

    let verts_before = verts.len();
    let tris_before = tris.len();

    // 硬体（alpha = 1）与外环（alpha = 0）都锚在 `rect` 的边缘上。
    let half = if aa { f * 0.5 } else { 0.0 };
    let hard_rect = grow(spec.rect, -half);
    let hard_radii = radii
        .map(|r| (r - half).max(MIN_AA_RADIUS))
        .fit(hard_rect.w, hard_rect.h);
    let hard = corners_of(hard_rect, hard_radii);
    // 段数由**最大的角**决定（最细），四角共用同一 `segs`——两级轮廓必须逐点对应。
    let stride = table.stride_for(radii.max() + half);
    let segs = CornerTable::segs_of(stride);
    let n = (4 * (segs + 1)) as u16;

    // 每个顶点按**自己在 rect 中的位置**取色（不是按角取色）。
    //
    // ⚠ **不能把 alpha 写死成 1**：硬体的 alpha 就是调用方给的 alpha，羽化带再用
    // `copy_colors(.., 0.0)` 把它压到 0 ⇒ 抗锯齿斜坡从"该色的 alpha"降到 0。
    // 曾经这里写死 `c[3] = 1.0`，于是**半透明圆角矩形全部变成不透明**
    // （取色器的 alpha 色块按 `#RRGGBBAA` 显示却完全不透 —— 实测像素等于纯色）。
    let rect = spec.rect;
    let cols = spec.corners;
    let hard_start = push_outline(verts, table, stride, segs, &hard, uv_at, |_i, p| {
        bilinear_color(cols, rect, p).into()
    });
    debug_assert_eq!((verts.len() as u16) - hard_start, n);

    // ── 硬体填充：以矩形中心为轴的扇形三角化 ──
    // 轮廓天然闭合（最后一个点连回第一个点，跨过四条直边）。中心色 = 中心的双线性值
    // （= 四角均值），内部为重心插值。
    let center = Vec2::new(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
    let center_idx = verts.len() as u16;
    verts.push(VertexP3U2C4 {
        pos: [center.x, center.y, 0.0],
        uv: uv_at(center),
        color: bilinear_color(cols, rect, center).into(),
    });
    for i in 0..n {
        let a = hard_start + i;
        let b = hard_start + (i + 1) % n;
        tris.push([a, b, center_idx]);
    }

    // ── 羽化带：外环 alpha = 0（颜色照抄硬体同序号点），沿**整圈**成带 ──
    if aa {
        let outer_rect = grow(spec.rect, half);
        // ⚠ **加完 `half` 必须重新 `fit`**：`radii` 的那次 `fit` 是对**原位图**做的，
        // 加宽后若不再夹一次，很窄的矩形会拿到"两角半径之和 > 边长"的外圈 ⇒ 角心次序
        // 颠倒 ⇒ 外圈变成逆时针 ⇒ 带状三角形出现负面积（条纹/黑洞）。
        let outer_radii = radii.map(|r| r + half).fit(outer_rect.w, outer_rect.h);
        let outer = corners_of(outer_rect, outer_radii);
        let fade = copy_colors(verts, hard_start, n, 0.0);
        let outer_start =
            push_outline(verts, table, stride, segs, &outer, uv_at, |i, _p| fade[i]);
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
/// | 硬体外沿（`B`） | `r_outer − f/2` | 1 |
/// | 硬体内沿（`C`） | `r_inner + f/2` | 1 |
/// | 内羽化（`D`） | `r_inner − f/2` | 0 |
///
/// 三组带子 `A-B` / `B-C` / `C-D`。`f <= 0` 时只留 `B-C`（硬体直接落在视觉边界上）。
///
/// ⚠ **两条边界的斜坡各以视觉边界为中心**（外边界 = `r_outer`、内边界 = `r_inner`）——
/// 与 [`push_rounded_rect`]（背景填充：硬体内缩 `f/2`、外环外扩 `f/2`）**同一约定**，
/// 于是同一块面板的"填充边"与"边框边"alpha 剖面重合，`f/2` 不会鼓到填充之外。
/// 硬体宽 = `width − f`（靠 `f ≤ width` 保证非负）。
///
/// # 羽化宽度（`feather`）与"边框看起来多粗"
///
/// 环带的内外**两条**边界各带 `f/2` 的斜坡 ⇒ 墨迹粗度 = `width + f`。所以 `f` 被夹到
/// **不超过边框宽度**（`f ≤ width`）：否则 1 物理像素的边框会被画成近 2 像素
/// （见 [`Theme::feather`](crate::style::Theme::feather) 的 1.2 逻辑像素 × 150% DPI = 1.8
/// 这个现实取值）。宽边框（`width ≥ f`）不受影响。
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
    radius: CornerRadius,
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
    // 四角按 CSS 规则收缩后，内轮廓 = 各角半径减 `width`（与 CSS 内侧半径同规则）。
    let ro = radius.fit(w, h);
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
            RoundedRectSpec { rect, radius: ro, feather, corners: [color; 4], uv },
        );
    }

    // 直角边框：四条轴对齐矩形条（圆弧带子在零半径下会退化成重合点）。
    if ro.is_zero() {
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

    // 内半径太小 ⇒ 内角是直角。夹到 0.5px（视觉上与直角无异），避免内圈点重合导致
    // 零面积三角形、以及两级轮廓点数不一致。
    let ri = ro.map(|r| (r - width).max(MIN_AA_RADIUS)).fit(inner_rect.w, inner_rect.h);
    // **羽化带宽不得超过边框本身宽度**：硬体（B→C）宽 = `width − f`，`f > width` 会让内外
    // 两条边界的斜坡互相越过 ⇒ 轮廓次序颠倒（负面积三角形）；同时这也是"1 物理像素的边框
    // 不该被 AA 画成 2 像素"的那条夹取（见本文档末尾"羽化宽度与边框看起来多粗"）。
    let f = feather.max(0.0).min(width).min(w.min(h) * 0.25);
    let half = f * 0.5;
    // 内羽化需要 `ri - half` 仍是有效半径（否则跳过内羽化，只保外羽化）。
    let inner_aa = half > 0.0 && ri.min() - half >= 0.0;
    let outer_aa = half > 0.0;

    let stride = table.stride_for(ro.max() + half);
    let segs = CornerTable::segs_of(stride);
    let n = (4 * (segs + 1)) as u16;
    // ⚠ 同 `push_rounded_rect`：硬轮廓的 alpha **就是调用方给的 alpha**（半透明边框
    // 必须真的半透明），两级羽化圈再用 `copy_colors(.., 0.0)` 压到 0。
    let solid: [f32; 4] = color.into();
    let flat = |_i: usize, _p: Vec2| solid;

    // ⚠ **硬体同样按 `half` 内缩 / 外扩**，与 `push_rounded_rect`（背景填充）**同一约定**：
    // 视觉外边界在 `rect` ⇒ 硬体外沿在 `rect − half`、外羽化圈在 `rect + half`；
    // 视觉内边界在 `inner_rect` ⇒ 硬体内沿在 `inner_rect + half`、内羽化圈在
    // `inner_rect − half`。于是两条边界的 alpha 斜坡**各以视觉边界为中心**，剖面与填充
    // 的边缘完全一致。
    //
    // 旧实现把硬体直接落在 `rect` / `inner_rect` 上：外边界少了这 `half` 的内缩 ⇒
    // ① 边框墨迹比填充的视觉边缘**鼓出 `f/2`**（圆角处最明显，观感"糊了一层"）；
    // ② 最外那半像素上边框是 alpha 1、而同处填充只有 ~0.5 ⇒ 半透明背景（窗口 / 背景图
    // tint）下会**透出背后内容**（用户实测："圆角观感不好"、"向内羽化像变成透明"）。
    let b_rect = grow(rect, -half);
    let b_radii = ro.map(|r| (r - half).max(MIN_AA_RADIUS)).fit(b_rect.w, b_rect.h);
    let c_rect = grow(inner_rect, half);
    // ⚠ 扩张后半径要 `+half` 并**重新 `fit`**（同 `push_rounded_rect` 外圈的注释：窄矩形
    // 会出现"两角半径之和 > 边长" ⇒ 角心次序颠倒 ⇒ 轮廓变逆时针 ⇒ 负面积三角形）。
    let c_radii = ri.map(|r| r + half).fit(c_rect.w, c_rect.h);

    // 由外向内写：A（外羽化，0）→ B（硬体外沿，1）→ C（硬体内沿，1）→ D（内羽化，0）。
    // 羽化圈的**颜色照抄对应主轮廓的同序号点**（只把 alpha 置 0）⇒ 纯 alpha 斜坡，
    // 不夹带色偏。
    let b_start = push_outline(
        verts,
        table,
        stride,
        segs,
        &corners_of(b_rect, b_radii),
        &|_p| uv,
        flat,
    );
    let c_start = push_outline(
        verts,
        table,
        stride,
        segs,
        &corners_of(c_rect, c_radii),
        &|_p| uv,
        flat,
    );
    let a_start = if outer_aa {
        let orc = grow(rect, half);
        let cs = corners_of(orc, ro.map(|r| r + half).fit(orc.w, orc.h));
        let fade = copy_colors(verts, b_start, n, 0.0);
        Some(push_outline(verts, table, stride, segs, &cs, &|_p| uv, |i, _p| fade[i]))
    } else {
        None
    };
    let d_start = if inner_aa {
        let irc = grow(inner_rect, -half);
        let cs = corners_of(
            irc,
            ri.map(|r| (r - half).max(MIN_AA_RADIUS)).fit(irc.w, irc.h),
        );
        let fade = copy_colors(verts, c_start, n, 0.0);
        Some(push_outline(verts, table, stride, segs, &cs, &|_p| uv, |i, _p| fade[i]))
    } else {
        None
    };

    // B-C 是边框本体；A-B / C-D 是两条羽化斜坡。
    //
    // ⚠ 硬体宽 = `width − f`（内缩/外扩各 `half` 之后）。`f == width`（细边框 + 大羽化，
    // 例如 1 物理像素边框 + 1.0 羽化）时它**退化成零宽**——峰值只剩一条线，内外两条
    // 斜坡直接接在同一条轮廓上。此时**不推中间那条带子**：零面积三角形在数值上无害，
    // 但会被"每个三角形绕序必须为正"的单测判为失败，也会白占三角形。
    if width - f > 1e-4 {
        push_band(tris, c_start, b_start, n);
    }
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
            radius: r.into(),
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

    /// 按位置找一个顶点（位置完全相同才命中）。
    fn vertex_at(v: &[VertexP3U2C4], p: Vec2) -> Option<&VertexP3U2C4> {
        v.iter()
            .find(|x| (x.pos[0] - p.x).abs() < 1e-3 && (x.pos[1] - p.y).abs() < 1e-3)
    }

    // ─── 背景颜色靠 lerp（弧上顶点按位置取色） ──────────────

    #[test]
    fn bilinear_color_hits_corners_center_and_clamps() {
        let rect = Rect::new(10.0, 20.0, 100.0, 50.0);
        let c = [Color::RED, Color::GREEN, Color::BLUE, Color::YELLOW];
        assert_eq!(bilinear_color(c, rect, Vec2::new(10.0, 20.0)), Color::RED);
        assert_eq!(bilinear_color(c, rect, Vec2::new(110.0, 20.0)), Color::GREEN);
        assert_eq!(bilinear_color(c, rect, Vec2::new(10.0, 70.0)), Color::BLUE);
        assert_eq!(bilinear_color(c, rect, Vec2::new(110.0, 70.0)), Color::YELLOW);
        // 中心 = 四角均值
        let mid: [f32; 4] = bilinear_color(c, rect, Vec2::new(60.0, 45.0)).into();
        let avg: [f32; 4] = c
            .iter()
            .fold([0.0f32; 4], |mut a, x| {
                let y: [f32; 4] = (*x).into();
                for i in 0..4 {
                    a[i] += y[i] * 0.25;
                }
                a
            });
        for i in 0..4 {
            assert!((mid[i] - avg[i]).abs() < 1e-5, "中心应为四角均值");
        }
        // 超界 clamp（羽化外环会落到 rect 外，此时应取边缘色而不是外推）
        assert_eq!(bilinear_color(c, rect, Vec2::new(-50.0, -50.0)), Color::RED);
        assert_eq!(bilinear_color(c, rect, Vec2::new(500.0, 500.0)), Color::YELLOW);
    }

    #[test]
    fn arc_vertices_are_coloured_by_position_not_by_corner() {
        // 回归：圆角弧上的顶点曾直接带"本角颜色"。水平渐变 + 大半径时渐变的**两端会被
        // 钉在弧的跨度上**——200px 宽、半径 18 的胶囊，左右各 18px 全是纯端色，
        // 整条 L→R 斜坡被压进中间 164px（肉眼："两端发平、渐变被拉长"）。
        // 按位置做双线性取色后，弧上顶点恰好是该处应有的渐变值。
        let t = table();
        let (l, r) = (Color::RED, Color::BLUE);
        let rect = Rect::new(0.0, 0.0, 200.0, 36.0);
        let mut v = Vec::new();
        let mut tr = Vec::new();
        // `feather = 0` ⇒ 硬体轮廓就是 rect + radius 本身，顶点位置好算。
        push_rounded_rect(
            &mut v,
            &mut tr,
            &t,
            RoundedRectSpec {
                rect,
                radius: 18.0.into(),
                feather: 0.0,
                corners: [l, r, l, r],
                uv: TEST_UV,
            },
        );
        // TL 弧的末点 = 上边 y=0 处 x=18 ⇒ u = 18/200 = 0.09。
        let got = vertex_at(&v, Vec2::new(18.0, 0.0)).expect("TL 弧末点应存在");
        let mut pure_l: [f32; 4] = l.into();
        pure_l[3] = 1.0;
        assert_ne!(got.color, pure_l, "弧上顶点不得是纯端色（应已 lerp）");
        let want: [f32; 4] = crate::draw::lerp_color(l, r, 0.09).into();
        for i in 0..3 {
            assert!(
                (got.color[i] - want[i]).abs() < 0.01,
                "上边 x=18 处应约为 9% 混色：实际 {:?}，期望 {:?}",
                got.color,
                want
            );
        }
        // 对照：同一行最左端（左中点在 x=0）仍是纯左端色。
        let left: [f32; 4] = vertex_at(&v, Vec2::new(0.0, 18.0)).expect("左中点").color;
        assert!((left[0] - pure_l[0]).abs() < 0.01 && (left[2] - pure_l[2]).abs() < 0.01);
    }

    #[test]
    fn vertical_gradient_edge_colours_are_exact() {
        // 纵向两端色：上下边缘必须是**精确**的端色（不能被平均拉走）。
        let t = table();
        let (top, bot) = (Color::WHITE, Color::BLACK);
        let rect = Rect::new(0.0, 0.0, 80.0, 40.0);
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_rect(
            &mut v,
            &mut tr,
            &t,
            RoundedRectSpec {
                rect,
                radius: 8.0.into(),
                feather: 0.0,
                corners: [top, top, bot, bot],
                uv: TEST_UV,
            },
        );
        let w: [f32; 4] = top.into();
        let b: [f32; 4] = bot.into();
        // ⚠ 直边**不细分**：只能取弧的端点（x = r 与 x = w - r），中间没有顶点。
        for x in [8.0f32, 72.0] {
            let tp = vertex_at(&v, Vec2::new(x, 0.0)).expect("上边弧端点");
            assert!((tp.color[0] - w[0]).abs() < 1e-4, "上边应为纯上端色（x={x}）");
        }
        let bp = vertex_at(&v, Vec2::new(8.0, 40.0)).expect("下边弧端点");
        assert!((bp.color[0] - b[0]).abs() < 1e-4, "下边应为纯下端色");
    }

    #[test]
    fn feather_ring_copies_hard_body_colours() {
        // 羽化必须是**纯 alpha 斜坡**：外环顶点色 = 硬体同序号点，只有 alpha 变 0。
        let t = table();
        let f = 3.0;
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_rect(
            &mut v,
            &mut tr,
            &t,
            RoundedRectSpec {
                rect: Rect::new(0.0, 0.0, 120.0, 40.0),
                radius: 10.0.into(),
                feather: f,
                corners: [Color::RED, Color::BLUE, Color::RED, Color::BLUE],
                uv: TEST_UV,
            },
        );
        let stride = t.stride_for(10.0 + f * 0.5);
        let n = 4 * (CornerTable::segs_of(stride) + 1);
        // 布局：[硬体 n][中心 1][外环 n]
        assert_eq!(v.len() as u32, n * 2 + 1);
        for i in 0..n {
            let h = v[i as usize].color;
            let o = v[(n + 1 + i) as usize].color;
            assert_eq!(h[3], 1.0);
            assert_eq!(o[3], 0.0);
            for ch in 0..3 {
                assert!(
                    (h[ch] - o[ch]).abs() < 1e-6,
                    "外环不得夹带色偏（i={i}）：{:?} vs {:?}",
                    h,
                    o
                );
            }
        }
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
            RoundedRectSpec { rect, radius: 10.0.into(), feather: f, corners: [Color::WHITE; 4], uv: TEST_UV },
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
    fn tiny_radius_is_floored_and_stays_well_formed() {
        // 极小的圆角被夹到 0.5px（1× 下看不出圆），但**照样走羽化**：四级轮廓的点数一致，
        // 不会像早期实现那样在半径塌缩时产生零面积三角形 / 绕序翻反。
        let t = table();
        for (r, f) in [(0.3_f32, 1.0_f32), (0.5, 2.0), (0.4, 0.5)] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            let mut s = spec(40.0, 24.0, r);
            s.feather = f;
            push_rounded_rect(&mut v, &mut tr, &t, s);
            assert!(!v.is_empty() && !tr.is_empty());
            assert_well_formed(&v, &tr);
            assert!(v.iter().all(|x| x.color[3] == 0.0 || x.color[3] == 1.0));
            for tri in &tr {
                assert!(cross(&v, tri) >= 0.0, "半径 {r} 羽化 {f}：出现负面积三角形");
            }
        }
    }

    #[test]
    fn all_zero_radius_is_a_plain_quad_without_aa() {
        // 四角**全**为 0 ⇒ 纯直角四边形（不镶嵌、不羽化）——保持既有语义。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let out = push_rounded_rect(&mut v, &mut tr, &t, spec(40.0, 24.0, 0.0));
        assert_eq!(out.verts, 4);
        assert_eq!(out.tris, 2);
        assert!(v.iter().all(|x| x.color[3] == 1.0));
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
            push_rounded_ring(&mut v, &mut tr, &t, Rect::new(3.0, 5.0, w, h), r.into(), 2.0, f, Color::WHITE, TEST_UV);
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
                    radius: r.into(),
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
                r.into(),
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
        push_rounded_ring(&mut v, &mut tr, &t, rect, 8.0.into(), 2.0, DEFAULT_FEATHER, Color::WHITE, TEST_UV);
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
            RoundedRectSpec { rect, radius: 8.0.into(), feather: DEFAULT_FEATHER, corners: [Color::WHITE; 4], uv: TEST_UV },
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
    fn per_vertex_colour_spans_the_full_gradient() {
        // 「圆角 + 渐变」靠逐顶点色（这正是不能走实例单色的原因）。但**顶点不带
        // "本角颜色"**——它带的是该位置的双线性值 ⇒ 四角**原始色**可能一个都不出现在
        // 顶点里（弧的端点已经离角 r 远）。这里断言的是"色场确实铺满整个渐变区间"。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        // 水平红→绿（另一项测试覆盖四角各异的对角情形）。
        let mut s = spec(40.0, 24.0, 6.0);
        s.corners = [Color::RED, Color::GREEN, Color::RED, Color::GREEN];
        push_rounded_rect(&mut v, &mut tr, &t, s);
        let alphas: Vec<f32> = v.iter().map(|x| x.color[3]).collect();
        assert!(alphas.contains(&1.0), "硬体顶点 alpha 必须为 1");
        assert!(alphas.contains(&0.0), "羽化环顶点 alpha 必须为 0");
        // 红分量应铺满 [0, 1) 的绝大部分（弧端点只比角内缩约 r/w，占比很小）。
        let reds: Vec<f32> = v.iter().map(|x| x.color[0]).collect();
        let hi = reds.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let lo = reds.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(hi > 0.95, "红分量应接近上界（实际最大 {hi}）");
        assert!(lo < 0.05, "红分量应接近下界（实际最小 {lo}）");
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
    fn ring_feather_is_capped_by_the_border_width() {
        // 边框的**视觉粗度 = width + f**（内外两条边界各占 f/2 的斜坡）。f 一旦大于
        // width，1 物理像素的边框就会被画成 1.8 像素的墨迹——用户实测的"边框显得很宽"：
        // `border_w 1.0 × 1.5 → floor → 1 物理像素`，而 `DEFAULT_FEATHER 1.2 × 1.5 = 1.8`。
        // 夹到 `f ≤ width` 后墨迹 = 1 + 1 = 2（斜坡各 0.5，仍是标准 AA）。
        let t = table();
        let rect = Rect::new(10.0, 10.0, 60.0, 36.0);
        let feather = 1.8_f32;
        let width = 1.0_f32;
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_ring(&mut v, &mut tr, &t, rect, 8.0.into(), width, feather, Color::RED, TEST_UV);
        assert!(tr.iter().all(|x| x.len() == 3));
        let half = width * 0.5; // = min(feather, width) / 2
        let min_x = v.iter().map(|p| p.pos[0]).fold(f32::INFINITY, f32::min);
        let max_x = v.iter().map(|p| p.pos[0]).fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (rect.x - min_x - half).abs() < 0.05,
            "外羽化只该探出 {half}px（实际 {}）—— 夹取没生效就是 1 像素边框被画粗",
            rect.x - min_x
        );
        assert!(
            (max_x - (rect.max().x + half)).abs() < 0.05,
            "右边对称（实际探出 {}）",
            max_x - rect.max().x
        );
        // **宽边框不受影响**（f 仍完全生效）：8px 边框 + 1.8 羽化 ⇒ 仍是 0.9px 斜坡。
        let mut v2 = Vec::new();
        let mut tr2 = Vec::new();
        push_rounded_ring(&mut v2, &mut tr2, &t, rect, 8.0.into(), 8.0, feather, Color::RED, TEST_UV);
        let min_x2 = v2.iter().map(|p| p.pos[0]).fold(f32::INFINITY, f32::min);
        assert!(
            (rect.x - min_x2 - feather * 0.5).abs() < 0.05,
            "宽边框的羽化不该被夹（实际 {}）",
            rect.x - min_x2
        );
    }

    #[test]
    fn ring_and_fill_share_the_same_edge_alpha_profile() {
        // **不变量**：同一块面板的"背景填充"与"边框环带"在**外边界**上必须有同一套
        // alpha 剖面——填充的硬体内缩 f/2、外环外扩 f/2（视觉边 = rect）；环带也必须
        // 如此。旧实现让环带的硬体直接落在 rect 上 ⇒ 边框比填充的视觉边缘鼓出 f/2，
        // 且最外半像素是 alpha 1（填充只有 ~0.5）⇒ 圆角发糊 / 半透明背景下透出背后。
        let t = table();
        let rect = Rect::new(10.0, 10.0, 60.0, 36.0);
        let f = 1.0_f32;
        let half = f * 0.5;
        // `rect.grow(half)` 是"含羽化的最外沿"（A 圈），`rect.grow(-half)` 是硬体（B 圈）。
        let outer = |v: &[VertexP3U2C4]| v.iter().map(|p| p.pos[0]).fold(f32::INFINITY, f32::min);
        let hard = |v: &[VertexP3U2C4]| {
            v.iter()
                .filter(|p| p.color[3] >= 0.999)
                .map(|p| p.pos[0])
                .fold(f32::INFINITY, f32::min)
        };
        // 填充（圆角实心矩形）
        let mut fv = Vec::new();
        let mut ft = Vec::new();
        push_rounded_rect(
            &mut fv,
            &mut ft,
            &t,
            RoundedRectSpec {
                rect,
                radius: 8.0.into(),
                feather: f,
                corners: [Color::RED; 4],
                uv: TEST_UV,
            },
        );
        // 环带（同 rect / 同圆角 / 1px 边框）
        let mut rv = Vec::new();
        let mut rt = Vec::new();
        push_rounded_ring(&mut rv, &mut rt, &t, rect, 8.0.into(), 1.0, f, Color::RED, TEST_UV);
        assert!(
            (outer(&fv) - outer(&rv)).abs() < 1e-3,
            "最外沿（外羽化圈）必须重合：填充 {:.3} vs 环带 {:.3}",
            outer(&fv),
            outer(&rv)
        );
        assert!(
            (hard(&fv) - hard(&rv)).abs() < 1e-3,
            "硬体外沿必须重合：填充 {:.3} vs 环带 {:.3}",
            hard(&fv),
            hard(&rv)
        );
        assert!(
            (hard(&rv) - (rect.x + half)).abs() < 1e-3,
            "环带硬体外沿 = rect + f/2（**内缩**半像素，与填充一致；实际 {}）",
            hard(&rv) - rect.x
        );
        assert!(
            (outer(&rv) - (rect.x - half)).abs() < 1e-3,
            "最外沿 = rect − f/2（环带与填充一致；实际 {}）",
            outer(&rv) - rect.x
        );
    }

    #[test]
    fn tess_cache_reuses_one_table() {
        let mut cache = TessCache::default();
        let a = cache.table();
        let b = cache.table();
        assert!(Rc::ptr_eq(&a, &b), "同一缓存必须复用同一张表");
        assert_eq!(a.pts.len(), N_FINE as usize + 1, "表含两端点");
    }

    #[test]
    fn ring_handles_heavily_trimmed_rects() {
        // **回归（"窗口被拖到视口边缘时整个边框变方"）**：环带必须自己在窄矩形里把半径
        // 夹进去，而不是让调用方"被裁剪了就退回直角四边条"。调用方那条 `trimmed`
        // 判断实际上**几乎总是成立**——文本输入框把 `self.clip` 设成自己的矩形（未取整），
        // 而 `collect_cmds` 用 `snap_rect` 取整过的矩形求交，1px 的差就足以判定"被裁剪"，
        // 于是输入框的聚焦边框一直是 4 条直角边条（截图里的"这个框是方形的"）。
        let t = table();
        for (w, h, r, bw) in [
            (2.0, 30.0, 8.0, 1.0),
            (30.0, 2.0, 8.0, 1.0),
            (3.0, 3.0, 8.0, 1.0),
            (40.0, 1.0, 12.0, 1.0),
            (1.0, 1.0, 4.0, 2.0),
            (100.0, 30.0, 40.0, 1.0),
        ] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            push_rounded_ring(
                &mut v,
                &mut tr,
                &t,
                Rect::new(0.0, 0.0, w, h),
                r.into(),
                bw,
                DEFAULT_FEATHER,
                Color::WHITE,
                TEST_UV,
            );
            assert!(!v.is_empty() && !tr.is_empty(), "w={w} h={h} 应仍产生几何");
            assert_well_formed(&v, &tr);
            for tri in &tr {
                // 半径恰好等于半宽时相邻轮廓点会**重合**（胶囊的左右两端），此时三角形
                // 面积恒为 0，符号由浮点噪声决定（实测 -1e-6 量级）⇒ 留一个与像素尺度
                // 无关的绝对容差。真正"绕序翻反"的三角形面积在 1 量级以上，不会被放过。
                assert!(
                    cross(&v, tri) >= -1e-3,
                    "窄矩形 {w}×{h} 出现负面积：{tri:?} = {:?} / {:?} / {:?}",
                    v[tri[0] as usize].pos,
                    v[tri[1] as usize].pos,
                    v[tri[2] as usize].pos
                );
            }
        }
    }

    #[test]
    fn convex_icons_are_clockwise_and_feather_outward() {
        // 图标几何从 `Icon::parts()` 来（单位方框）；这里验证**每一个**分片都满足
        // 镶嵌器的前提：凸多边形的绕序为正，且羽化外环确实在外面。
        for icon in [
            crate::draw::Icon::ChevronDown,
            crate::draw::Icon::ChevronUp,
            crate::draw::Icon::ChevronLeft,
            crate::draw::Icon::ChevronRight,
            crate::draw::Icon::Check,
            crate::draw::Icon::Grip,
            crate::draw::Icon::GripDiagonal,
            crate::draw::Icon::Warning,
            crate::draw::Icon::Close,
        ] {
            for part in icon.parts() {
                // 缩放到 20×20 的方框。
                let pts: Vec<Vec2> = part
                    .iter()
                    .map(|p| Vec2::new(p.x * 20.0, p.y * 20.0))
                    .collect();
                let mut v = Vec::new();
                let mut tr = Vec::new();
                push_convex(&mut v, &mut tr, &pts, DEFAULT_FEATHER, Color::WHITE, TEST_UV);
                assert_well_formed(&v, &tr);
                for tri in &tr {
                    assert!(
                        cross(&v, tri) > 0.0,
                        "{icon:?} 的分片绕序反了：{tri:?}"
                    );
                }
                // 顶点数 = 硬体 (n + 1 重心) + 外环 n
                assert_eq!(v.len(), pts.len() * 2 + 1, "{icon:?} 顶点数");
                // 外环（alpha = 0）必须落在硬体**之外**：逐点距离应大于 0。
                let m = pts.len();
                for i in 0..m {
                    let d = (Vec2::new(v[m + 1 + i].pos[0], v[m + 1 + i].pos[1])
                        - Vec2::new(v[i].pos[0], v[i].pos[1]))
                    .length();
                    assert!(d > 0.0, "{icon:?} 第 {i} 点外环没外扩");
                    assert!(d <= DEFAULT_FEATHER * MAX_MITER + 1e-3, "斜接超上限");
                }
            }
        }
    }

    #[test]
    fn convex_without_feather_is_just_the_polygon() {
        let pts = [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(5.0, 8.0)];
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let out = push_convex(&mut v, &mut tr, &pts, 0.0, Color::WHITE, TEST_UV);
        assert_eq!(out.verts, 4, "3 顶点 + 重心");
        assert_eq!(out.tris, 3);
        assert!(v.iter().all(|x| x.color[3] == 1.0), "不羽化 ⇒ 无 alpha=0 顶点");
        // 退化输入：点数 < 3 不产生几何。
        let out = push_convex(&mut v, &mut tr, &pts[..2], 1.0, Color::WHITE, TEST_UV);
        assert_eq!(out.verts, 0);
    }

    #[test]
    fn convex_feather_is_clamped_on_thin_shapes() {
        // 回归："图标像是近视一样"——细笔画（`Icon::Grip` 的三横在 12px 图标上只有
        // ~1.2px 高）若照搬 `Theme::feather`（1.0~1.5px），AA 斜坡会和笔画本身一样宽，
        // 整条笔画被糊成一片渐变。羽化宽必须夹到"到边距离的一半"，给实心留一半。
        let thin_h = 1.2f32;
        let bar = [
            Vec2::new(0.0, 0.0),
            Vec2::new(6.0, 0.0),
            Vec2::new(6.0, thin_h),
            Vec2::new(0.0, thin_h),
        ];
        assert!((min_edge_distance(&bar) - thin_h * 0.5).abs() < 1e-4, "细条半厚 = {thin_h}/2");
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_convex(&mut v, &mut tr, &bar, 1.5, Color::WHITE, TEST_UV);
        // 外环（alpha = 0）相对硬体的外扩量 ≤ 半厚（而不是 1.5px）。
        let n = bar.len();
        let mut max_off = 0.0f32;
        for i in 0..n {
            let a = Vec2::new(v[i].pos[0], v[i].pos[1]);
            let b = Vec2::new(v[n + 1 + i].pos[0], v[n + 1 + i].pos[1]);
            max_off = max_off.max((b - a).length());
        }
        assert!(
            max_off <= thin_h * 0.5 * 1.05,
            "细条的羽化外扩应被夹到半厚以内，实际 {max_off:.3}px（原 feather = 1.5px）"
        );
        // 粗形状不受影响：羽化仍按原值走（三角形内切半径足够大）。
        let big = [Vec2::new(0.0, 0.0), Vec2::new(40.0, 0.0), Vec2::new(20.0, 30.0)];
        let mut v2 = Vec::new();
        let mut tr2 = Vec::new();
        push_convex(&mut v2, &mut tr2, &big, 1.5, Color::WHITE, TEST_UV);
        let max2 = (0..3)
            .map(|i| {
                let a = Vec2::new(v2[i].pos[0], v2[i].pos[1]);
                let b = Vec2::new(v2[4 + i].pos[0], v2[4 + i].pos[1]);
                (b - a).length()
            })
            .fold(0.0f32, f32::max);
        assert!(max2 >= 1.4, "粗形状的羽化外扩应接近 1.5px，实际 {max2:.3}px");
    }

    #[test]
    fn outward_normal_points_out_of_a_clockwise_polygon() {
        // 顺时针（Y 向下）三角形的边 `a → b`：外法线应背离重心。
        let pts = [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(5.0, 8.0)];
        let c = Vec2::new(5.0, 8.0 / 3.0);
        for i in 0..3 {
            let (a, b) = (pts[i], pts[(i + 1) % 3]);
            let n = outward_normal(b - a);
            let mid = (a + b) * 0.5;
            assert!((mid + n - c).length() > (mid - c).length(), "法线应朝外");
        }
    }

    // ─── 四角各自的半径 ────────────────────────────────────

    /// 离某个角最近的顶点距离（用于区分"这个角是圆的还是直的"）。
    fn nearest_vertex_dist(v: &[VertexP3U2C4], corner: Vec2) -> f32 {
        v.iter()
            .map(|x| (Vec2::new(x.pos[0], x.pos[1]) - corner).length())
            .fold(f32::INFINITY, f32::min)
    }

    #[test]
    fn only_some_corners_are_rounded() {
        // 「只圆上面两个角」：TL / TR 有 12px 弧，BL / BR 是直角。
        // 判据：圆角的最近顶点离角 = `r(√2 - 1)` ≈ 0.414r（12px ⇒ 4.97px）；
        // 直角的几乎贴角（≈ 0.2px，来自 0.5px 的半径下限）。
        let t = table();
        let rect = Rect::new(0.0, 0.0, 120.0, 60.0);
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_rect(
            &mut v,
            &mut tr,
            &t,
            RoundedRectSpec {
                rect,
                radius: CornerRadius { tl: 12.0, tr: 12.0, br: 0.0, bl: 0.0 },
                feather: 0.0,
                corners: [Color::WHITE; 4],
                uv: TEST_UV,
            },
        );
        assert_well_formed(&v, &tr);
        let dg = |c: Vec2| nearest_vertex_dist(&v, c);
        let tl = dg(Vec2::new(rect.x, rect.y));
        let br = dg(Vec2::new(rect.x + rect.w, rect.y + rect.h));
        let bl = dg(Vec2::new(rect.x, rect.y + rect.h));
        assert!(tl > 3.0, "TL 应为 12px 圆角（最近顶点距离 {tl}）");
        assert!(br < 1.5, "BR 应为直角（最近顶点距离 {br}）");
        assert!(bl < 1.5, "BL 应为直角（最近顶点距离 {bl}）");
        // 弧位于上边：`x = 12` 处的上边点必须存在（圆角切掉了左上角）。
        assert!(vertex_at(&v, Vec2::new(12.0, 0.0)).is_some(), "TL 弧的上端点应存在");
        assert!(vertex_at(&v, Vec2::new(108.0, 0.0)).is_some(), "TR 弧的上端点应存在");
        // 包围盒仍等于 rect（直角那两角把外沿顶满）。
        let xs: Vec<f32> = v.iter().map(|x| x.pos[0]).collect();
        let ys: Vec<f32> = v.iter().map(|x| x.pos[1]).collect();
        assert!((xs.iter().copied().fold(f32::INFINITY, f32::min) - rect.x).abs() < 1e-3);
        assert!((xs.iter().copied().fold(f32::NEG_INFINITY, f32::max) - (rect.x + rect.w)).abs() < 1e-3);
        assert!((ys.iter().copied().fold(f32::NEG_INFINITY, f32::max) - (rect.y + rect.h)).abs() < 1e-3);
    }

    #[test]
    fn oversized_corner_radii_shrink_equally() {
        // 四角之和超过边长 ⇒ 等比收缩（不是各自 clamp 把形状削平）。
        // 40 宽 + 上下各 30 ⇒ 比例 2/3 ⇒ 20/20。
        let t = table();
        let rect = Rect::new(0.0, 0.0, 40.0, 40.0);
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_rect(
            &mut v,
            &mut tr,
            &t,
            RoundedRectSpec {
                rect,
                radius: CornerRadius { tl: 30.0, tr: 30.0, br: 30.0, bl: 30.0 },
                feather: 0.0,
                corners: [Color::WHITE; 4],
                uv: TEST_UV,
            },
        );
        assert_well_formed(&v, &tr);
        // 收缩到 r = 20 ⇒ 左上角的最近顶点距离 = 20(√2 - 1) ≈ 8.28（未收缩的 30 会是 12.4）。
        let d = nearest_vertex_dist(&v, Vec2::new(0.0, 0.0));
        assert!((d - 8.284).abs() < 0.5, "收缩后应为 r=20 的弧（距离 {d}）");
    }

    #[test]
    fn ring_supports_per_corner_radii() {
        let t = table();
        for radii in [
            CornerRadius { tl: 10.0, tr: 10.0, br: 0.0, bl: 0.0 },
            CornerRadius { tl: 12.0, tr: 0.0, br: 12.0, bl: 0.0 },
            CornerRadius::all(8.0),
        ] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            push_rounded_ring(
                &mut v,
                &mut tr,
                &t,
                Rect::new(0.0, 0.0, 80.0, 40.0),
                radii,
                2.0,
                DEFAULT_FEATHER,
                Color::WHITE,
                TEST_UV,
            );
            assert!(!v.is_empty() && !tr.is_empty());
            assert_well_formed(&v, &tr);
            for tri in &tr {
                assert!(cross(&v, tri) >= 0.0, "环带出现负面积：{tri:?}");
            }
        }
    }

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
                r.into(),
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
        // 羽化抗锯齿的前提：**不透明色**的硬体 alpha = 1、外环 alpha = 0
        // （光栅化器插值出过渡）。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_rect(&mut v, &mut tr, &t, spec(60.0, 36.0, 8.0));
        let hard = v.iter().filter(|x| x.color[3] == 1.0).count();
        let feather = v.iter().filter(|x| x.color[3] == 0.0).count();
        assert!(hard > 0 && feather > 0);
        assert_eq!(hard + feather, v.len(), "只应有 alpha 1 与 0 两类顶点");
    }

    #[test]
    fn translucent_rounded_rect_keeps_its_alpha() {
        // 回归："半透明不完全"——硬体曾把 alpha 写死成 1，于是**半透明圆角矩形全变成
        // 不透明**（实测像素：取色器的 `#6EA8FF0A` 色块渲染成纯 `#6EA8FF`，4% 透明度
        // 完全丢失）。硬体的 alpha 必须**就是**调用方给的值，羽化环再从它降到 0。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let mut s = spec(60.0, 36.0, 8.0);
        s.corners = [Color::rgba(1.0, 0.0, 0.0, 0.25); 4];
        push_rounded_rect(&mut v, &mut tr, &t, s);
        assert!(!v.is_empty());
        let solid = v.iter().filter(|x| (x.color[3] - 0.25).abs() < 1e-6).count();
        let fade = v.iter().filter(|x| x.color[3] == 0.0).count();
        assert!(solid > 0, "硬体顶点必须保留 0.25 的 alpha");
        assert!(fade > 0, "羽化环仍是 0");
        assert_eq!(solid + fade, v.len(), "只应有 0.25 与 0 两类顶点");
        // 直角（radius = 0）走四边形捷径，同样不能改 alpha。
        let mut v2 = Vec::new();
        let mut tr2 = Vec::new();
        let mut s2 = spec(60.0, 36.0, 0.0);
        s2.corners = [Color::rgba(0.0, 1.0, 0.0, 0.5); 4];
        push_rounded_rect(&mut v2, &mut tr2, &t, s2);
        assert!(v2.iter().all(|x| (x.color[3] - 0.5).abs() < 1e-6), "直角路径也不能改 alpha");
    }

    #[test]
    fn shadow_is_alpha_ramped_ring_within_blur_bounds() {
        // **顶点色软阴影**：内轮廓 alpha = 给定值，向外按二次曲线渐隐到 0；
        // 所有顶点都落在"本体向外 blur + 最外圈偏移"的范围内（否则会把别的窗口涂黑）。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let rect = Rect::new(100.0, 50.0, 200.0, 120.0);
        let (blur, a) = (16.0, 0.6);
        let offset = Vec2::new(0.0, 4.0);
        let out = push_rounded_shadow(
            &mut v,
            &mut tr,
            &t,
            rect,
            CornerRadius::all(8.0),
            blur,
            offset,
            Color::rgba(0.0, 0.0, 0.0, a),
            TEST_UV,
        );
        assert!(out.verts > 0 && out.tris > 0, "应产生几何");
        assert_well_formed(&v, &tr);
        // 顶点必须全在 `rect` 外扩 blur、再按 offset 偏移的矩形内（含 0.5px 浮点余量）。
        let (lo, hi) = (
            Vec2::new(rect.x - blur + offset.x, rect.y - blur + offset.y),
            Vec2::new(
                rect.x + rect.w + blur + offset.x,
                rect.y + rect.h + blur + offset.y,
            ),
        );
        for x in &v {
            let p = Vec2::new(x.pos[0], x.pos[1]);
            assert!(p.x >= lo.x - 0.5 && p.y >= lo.y - 0.5, "顶点越界（外扩）{p:?}");
            assert!(p.x <= hi.x + 0.5 && p.y <= hi.y + 0.5, "顶点越界（外扩）{p:?}");
        }
        // **本体边缘处浓度最高、且没有等浓度平台**（"窗口阴影下方突出"的根因）：
        // 本体下缘正下方 1px 处的 alpha 必须**小于**内轮廓的 `a`，且随距离单调下降。
        let alpha_below = |dy: f32| -> f32 {
            // 取本体下缘中点正下方 dy 处的 alpha（按圈插值：找包含该点的相邻两圈）。
            let y = rect.y + rect.h + dy;
            let mut best = 0.0f32;
            for i in 0..SHADOW_STEPS {
                let t0 = i as f32 / SHADOW_STEPS as f32;
                let t1 = (i + 1) as f32 / SHADOW_STEPS as f32;
                let y0 = rect.y + rect.h + blur * t0 + offset.y * t0;
                let y1 = rect.y + rect.h + blur * t1 + offset.y * t1;
                if y >= y0 && y <= y1 && (y1 - y0) > 1e-3 {
                    let k = (y - y0) / (y1 - y0);
                    let a0 = a * (1.0 - t0) * (1.0 - t0);
                    let a1 = a * (1.0 - t1) * (1.0 - t1);
                    best = best.max(a0 + (a1 - a0) * k);
                }
            }
            best
        };
        let (a1, a2, a3) = (alpha_below(1.0), alpha_below(6.0), alpha_below(12.0));
        assert!(a1 > 0.0 && a1 < a, "本体边缘外 1px 必须已低于内轮廓浓度（无平台）");
        assert!(a1 > a2 && a2 > a3, "浓度必须随离本体距离单调下降：{a1} {a2} {a3}");
        // alpha 只有 `a·(1−t)²` 这 `SHADOW_STEPS + 1` 档，且最外圈必须为 0（天然 AA）。
        let mut alphas: Vec<f32> = v.iter().map(|x| x.color[3]).collect();
        alphas.sort_by(|p, q| p.partial_cmp(q).unwrap());
        alphas.dedup_by(|p, q| (*p - *q).abs() < 1e-6);
        assert_eq!(alphas.len(), SHADOW_STEPS as usize + 1, "影调档数 = 段数 + 1");
        assert!((alphas[0]).abs() < 1e-6, "最外圈 alpha 必须为 0");
        assert!((alphas[alphas.len() - 1] - a).abs() < 1e-5, "内轮廓 alpha = 调用方给的值");
        // RGB 全程不变（纯 alpha 斜坡，不夹带色偏）。
        assert!(v.iter().all(|x| x.color[0] == 0.0 && x.color[1] == 0.0 && x.color[2] == 0.0));
        // 直角窗口（radius = 0）走同一套弧表，不得产生退化三角形。
        let mut v2 = Vec::new();
        let mut tr2 = Vec::new();
        push_rounded_shadow(
            &mut v2,
            &mut tr2,
            &t,
            rect,
            CornerRadius::default(),
            blur,
            offset,
            Color::rgba(0.0, 0.0, 0.0, 0.5),
            TEST_UV,
        );
        assert_well_formed(&v2, &tr2);
        assert!(v2.iter().all(|x| x.color[3] <= 0.5 + 1e-6));
    }

    #[test]
    fn shadow_keeps_the_callers_rgb_and_alpha() {
        // **阴影颜色**（`ShadowStyle::color` / `Palette::shadow`）是任意色，不只是黑：
        // 顶点 RGB 必须**原样**带出（只有 alpha 按圈衰减）——否则"投影颜色"调了没反应。
        // 内轮廓 alpha 必须等于调用方给的值（颜色 alpha 不被吞、也不被放大）。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        let c = Color::rgba(0.25, 0.5, 0.9, 0.35);
        push_rounded_shadow(
            &mut v,
            &mut tr,
            &t,
            Rect::new(20.0, 20.0, 80.0, 40.0),
            CornerRadius::all(4.0),
            10.0,
            Vec2::new(0.0, 2.0),
            c,
            TEST_UV,
        );
        assert!(!v.is_empty(), "应产生几何");
        assert!(
            v.iter().all(|x| (x.color[0] - 0.25).abs() < 1e-6
                && (x.color[1] - 0.5).abs() < 1e-6
                && (x.color[2] - 0.9).abs() < 1e-6),
            "顶点 RGB 必须与调用方给的颜色一致（阴影可以是任意色）"
        );
        let max_a = v.iter().map(|x| x.color[3]).fold(0.0f32, f32::max);
        assert!((max_a - 0.35).abs() < 1e-5, "内轮廓 alpha = 调用方给的值，实际 {max_a}");
    }

    #[test]
    fn shadow_skips_when_invisible_or_degenerate() {
        let t = table();
        let rect = Rect::new(0.0, 0.0, 100.0, 60.0);
        let run = |blur: f32, color: Color, r: Rect| {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            push_rounded_shadow(
                &mut v,
                &mut tr,
                &t,
                r,
                CornerRadius::all(6.0),
                blur,
                Vec2::ZERO,
                color,
                TEST_UV,
            )
        };
        assert_eq!(run(0.0, Color::rgba(0.0, 0.0, 0.0, 0.5), rect).verts, 0, "blur = 0 不画");
        assert_eq!(run(12.0, Color::rgba(0.0, 0.0, 0.0, 0.0), rect).verts, 0, "全透明不画");
        assert_eq!(run(12.0, Color::rgba(0.0, 0.0, 0.0, 0.5), Rect::new(0.0, 0.0, 0.0, 60.0)).verts, 0, "退化矩形不画");
    }

    #[test]
    fn translucent_ring_keeps_its_alpha() {        // 同一条约定作用于边框环带：半透明边框必须真的半透明。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_ring(
            &mut v,
            &mut tr,
            &t,
            Rect::new(0.0, 0.0, 60.0, 36.0),
            CornerRadius::all(8.0),
            2.0,
            DEFAULT_FEATHER,
            Color::rgba(1.0, 1.0, 1.0, 0.5),
            TEST_UV,
        );
        assert!(!v.is_empty());
        let solid = v.iter().filter(|x| (x.color[3] - 0.5).abs() < 1e-6).count();
        let fade = v.iter().filter(|x| x.color[3] == 0.0).count();
        assert!(solid > 0 && fade > 0, "硬轮廓 0.5 / 羽化圈 0");
        assert_eq!(solid + fade, v.len());
    }

    // ─── 背景图（逐顶点 UV 的仿射映射） ───────────────────────

    #[test]
    fn per_vertex_uv_is_affine_through_the_rounded_path() {
        // 贴图与圆角遮罩共存的前提：**每个顶点**的 UV 等于它自己在 rect 中的线性映射。
        // 仿射映射在扇形三角化下由重心插值**精确**再现（无需细分、无需着色器），
        // 因此圆角处的 UV 也是对的（图片在圆角边缘被 alpha 羽化裁掉，而不是被拉伸）。
        let t = table();
        let rect = Rect::new(10.0, 20.0, 100.0, 50.0);
        let (uv0, uv1) = (Vec2::new(0.25, 0.5), Vec2::new(0.75, 1.0));
        let uv_at = move |p: Vec2| {
            let k = Vec2::new((uv1.x - uv0.x) / rect.w, (uv1.y - uv0.y) / rect.h);
            let u = uv0 + (p - rect.min()) * k;
            [u.x, u.y]
        };
        for radius in [0.0f32, 8.0] {
            let mut v = Vec::new();
            let mut tr = Vec::new();
            let out = push_rounded_rect_uv(
                &mut v,
                &mut tr,
                &t,
                RoundedRectSpec {
                    rect,
                    radius: radius.into(),
                    feather: 1.0,
                    corners: [Color::WHITE; 4],
                    uv: TEST_UV, // 逐顶点 UV 生效时该兜底值不应出现在任何顶点上
                },
                &uv_at,
            );
            assert!(out.verts > 0 && out.tris > 0, "radius={radius}: 应有几何");
            assert_well_formed(&v, &tr);
            for x in &v {
                let p = Vec2::new(x.pos[0], x.pos[1]);
                let want = uv_at(p);
                assert!(
                    (x.uv[0] - want[0]).abs() < 1e-5 && (x.uv[1] - want[1]).abs() < 1e-5,
                    "radius={radius}: 顶点 {p:?} 的 UV 应为线性映射值 {want:?}，实际 {:?}",
                    x.uv
                );
                assert_ne!(x.uv, TEST_UV, "绝不能落到兜底 UV（那是字形页的采样点）");
            }
            // radius = 0 走四边形捷径：四角 UV 恰好是 (uv0, uv0/uv1, uv1)。
            if radius == 0.0 {
                assert_eq!(out.verts, 4, "直角贴图 = 一个四边形");
                let corner = |x: f32, y: f32| uv_at(Vec2::new(x, y));
                assert_eq!(vertex_at(&v, rect.min()).map(|q| q.uv), Some(corner(rect.x, rect.y)));
                assert_eq!(
                    vertex_at(&v, Vec2::new(rect.x + rect.w, rect.y + rect.h)).map(|q| q.uv),
                    Some(corner(rect.x + rect.w, rect.y + rect.h))
                );
            }
        }
    }

    #[test]
    fn fixed_uv_still_applies_when_no_per_vertex_map_is_used() {
        // 非贴图路径（纯色 / 白纹理）必须继续**逐顶点同值**——回归"所有顶点都带调用方给的
        // 采样点"（曾有 UV 写死 (0,0) → 圆角背景整体消失的 bug）。
        let t = table();
        let mut v = Vec::new();
        let mut tr = Vec::new();
        push_rounded_rect(&mut v, &mut tr, &t, spec(60.0, 36.0, 8.0));
        assert!(v.iter().all(|x| x.uv == TEST_UV), "定值路径：全部顶点同一个 UV");
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
            0.0.into(),
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
                r.into(),
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
            12.0.into(),
            1.0,
            DEFAULT_FEATHER,
            Color::WHITE,
            TEST_UV,
        );
        let segs = CornerTable::segs_of(t.stride_for(12.0 + DEFAULT_FEATHER * 0.5)) as usize;
        // 外羽化 + 外/内轮廓 + 内羽化 ⇒ 4 圈（内羽化在 i - f/2 有效时才画）
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
                4.0.into(),
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
