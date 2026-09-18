//! **顶点收集与合批机制**（从 `ui.rs` 拆出）。
//!
//! 这一模块是 `Ui::finish` 的**产出端**：把已录制的绘制命令变成「可提交的顶点段」，
//! 再切成批次交给 [`crate::UiBackend`]。它与控件树 / 布局 / 命中测试**无耦合**——
//! 只依赖 `draw` 的命令类型与 `rjw_2d_render` 的顶点格式。
//!
//! | 类型 / 函数 | 职责 |
//! |---|---|
//! | [`Geom`] | 一段几何：顶点 + 三角形索引（索引相对本段 `verts`） |
//! | [`QuadCollector`] | 按 `(win, elem, group, tex)` 收集几何（四边形 / 镶嵌圆角矩形） |
//! | [`CachedQuad`] | 一条可提交几何段（含其覆盖的元素序，供 `UiBatchSource::elements`） |
//! | [`segment_runs`] / [`SegRun`] | **切段规则**（= draw call 数）的唯一裁决点 |
//! | [`CacheStats`] | `finish` 各阶段耗时/计数累加器 |
//! | [`cmd_sig_hash`] | 内容签名哈希（窗口顶点缓存失效判定） |
//! | [`resample_gradient`] | 裁剪后按双线性重采样渐变的四角色 |
//! | [`safe_line_slice`] | 视觉行 → 文本切片（字符边界安全） |
//! | [`debug_layout_outline`] | `debug_layout` 描边写入调试叠加 |

use glam::Vec2;
use rjw_2d_render::VertexP3U2C4;
use rjw_color::Color;
use rjw_text::VisualLine;
use rjw_transform::Rect;

use crate::backend::Tri;
use crate::draw::{CornerRadius, DebugShape, DrawKind, Gradient, Icon, ImageBg, UiDraw};
use crate::ui::TEXT_LINE_HEIGHT_VERSION;

// ─── 分组维度 ─────────────────────────────────────────────────

/// 分组维度（提交排序用）：
/// - `0` = 图形（Solid / RoundedRect / Rect(渐变) / Border / Caret——白纹理）；
/// - `1` = 文字（字形图集纹理）。
pub(crate) const GROUP_GRAPHIC: u8 = 0;
pub(crate) const GROUP_TEXT: u8 = 1;

// ─── 几何段（顶点 + 索引） ────────────────────────────────────

/// 一段几何：顶点 + 三角形索引，索引**相对本段 `verts`**（从 0 起）。
///
/// **为什么索引与顶点同段而不分开缓存**：索引与顶点同源（同一个抽样循环 / 同一个
/// 四边形），任何"点数 / 是否闭合成环"的判断偏差都会产生错乱三角形且**不会 panic**。
/// 同段存放使二者永不脱节；代价只是切段时多一次 `u16` 加法。
///
/// `u16` 索引 ⇒ 单段顶点数上限 `u16::MAX`（由 [`segment_runs`] 的预算兜底）。
#[derive(Debug, Default, Clone)]
pub(crate) struct Geom {
    pub(crate) verts: Vec<VertexP3U2C4>,
    pub(crate) tris: Vec<Tri>,
}

impl Geom {
    /// 追加一个四边形（顶点顺序 `[TL, TR, BL, BR]`；三角形约定与
    /// `rjw_2d_render::QUAD_TRI_INDICIES` 一致）。
    pub(crate) fn push_quad(&mut self, quad: &[VertexP3U2C4; 4]) {
        let b = self.verts.len() as u16;
        self.verts.extend_from_slice(quad);
        self.tris.push([b, b + 1, b + 3]);
        self.tris.push([b + 3, b + 2, b]);
    }

    /// 追加另一段几何，**索引按当前顶点数平移**。
    pub(crate) fn append(&mut self, other: &Geom) {
        let base = self.verts.len() as u16;
        self.verts.extend_from_slice(&other.verts);
        self.tris
            .extend(other.tris.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
    }

    /// 是否无几何可提交。
    pub(crate) fn is_empty(&self) -> bool {
        self.verts.is_empty() || self.tris.is_empty()
    }
}

// ─── 可提交顶点段 ─────────────────────────────────────────────

/// 按 `(窗口 z, 放置序, 元素序, 图形/文字组, 纹理 uid, 裁剪)` 分组的四边形顶点段的来源。
///
/// **提交顺序**：`(win, place, elem, g, tex, clip)`——
/// 1. `win` 升序：非窗口（0）最底，窗口按 z 从下到上；
/// 2. `place`（**顶层放置序**）升序：win=0 内"后录制者在上"（见 [`crate::Ui`] 的
///    `z0_ranges`）；窗口内恒为同一个 `place`，不影响窗口序；
/// 3. `elem`（元素序）升序：同放置内后录控件覆盖先录控件，`elem = 0` 的容器装饰最先；
/// 4. `g`：元素内"背景/图形 → 文字"；`tex`：白纹理与字形同页时相邻合批；
/// 5. `clip`：不同 scissor 不能同批（一次 draw 只能一个 scissor）。
///
/// 一条**可提交的几何段**：`(win, place, elem, g, tex, clip, geom, 元素序列表)`。
/// 由窗口 / win=0 放置的**顶点缓存命中**或**本帧重建**产生。`win` 决定窗口原点
/// （局部顶点 → 世界变换）与 layer。
///
/// 末位是**本片段覆盖的元素序列表**（[`crate::UiBatchSource::elements`] 的来源；
/// 缓存命中路径在采集期已统计）。
///
/// `clip` = 本段的环境裁剪层（绝对逻辑坐标；`None` = 不裁剪）——提交时按它算
/// [`crate::UiBatch::clip`]（batch scissor），几何本身**不再被切割**。
pub(crate) type CachedQuad = (u32, u32, u32, u8, u64, Option<Rect>, Geom, Vec<u32>);


/// **提交排序键**（纯函数）：`(win, place, elem, group, tex, clip)`。
///
/// 抽出来是为了让"谁画在谁上面"这条规则能在**没有 `Ui` / 没有 GPU** 的情况下断言
/// （见 `gpu_batch::batch_contract_tests`）：`place` 在任何 `elem` 之前比较，
/// 才使"每个顶层放置一个排序空间"成立——没有它，一个放置的 `elem = 0` 装饰会被
/// **更早录制**的另一个放置的内容穿透（实测：可拖动面板底色被 FPS 标签穿透、
/// `scroll_at` 的滚动条被列表项盖住，两者都表现为闪烁）。
///
/// `clip` 用量化整数键比较（[`ClipKey`]；`Rect` 是 `f32`，不能直接做 `Ord` 键）。
#[inline]
pub(crate) fn submit_sort_key(q: &CachedQuad) -> (u32, u32, u32, u8, u64, Option<ClipKey>) {
    (q.0, q.1, q.2, q.3, q.4, clip_key(q.5))
}

/// 一个合批段（= 一个 [`crate::UiBatch`] = 一次 draw call 的候选）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SegRun {
    /// 段所属窗口。
    pub(crate) window: u32,
    /// 段使用的纹理 uid。
    pub(crate) texture: u64,
    /// 本段的**环境裁剪层**（绝对逻辑坐标；`None` = 不裁剪）。
    /// 段内必须一致：一次 draw 只能有一个 scissor。
    pub(crate) clip: Option<Rect>,
    /// 本段包含的四边形条目数。
    pub(crate) quads: usize,
    /// 本段顶点总数。
    pub(crate) verts: usize,
}

/// **环境裁剪层的量化键**（绝对逻辑坐标 → 1px 整数四元组 `(x, y, w, h)`）。
///
/// 为什么需要量化而不用 `Rect` 本身：几何缓存 / 合批的分组键必须 `Eq + Hash + Ord`，
/// 而 `Rect` 是 `f32`。量化到 1px 与"最终 scissor 本来就是整数像素"的口径一致。
pub(crate) type ClipKey = (i32, i32, i32, i32);

/// 由绝对逻辑矩形构造量化键（归一化 + 四舍五入到 1px）；`None` 原样传递。
#[inline]
pub(crate) fn clip_key(clip: Option<Rect>) -> Option<ClipKey> {
    clip.map(|c| {
        let r = c.normalized();
        (
            r.x.round() as i32,
            r.y.round() as i32,
            r.w.round() as i32,
            r.h.round() as i32,
        )
    })
}

/// 量化键还原成矩形（1px 精度）。**量化误差无妨**：最终 scissor 本来就按整数像素取整。
#[inline]
pub(crate) fn clip_rect(key: ClipKey) -> Rect {
    Rect::new(key.0 as f32, key.1 as f32, key.2 as f32, key.3 as f32)
}

/// **切段规则**（纯函数，可无 GPU 单测）：把已按 `(win, elem, group, tex, clip)` 排好序的
/// 四边形序列切成若干段，使每段满足：
///
/// 1. **同一 `(window, texture, clip)`**——窗口不同则实例变换 / tint 不同（无法合并）；
///    纹理不同则 bind group 不同（一次 draw call 只能绑一个纹理）；
///    **裁剪不同则 scissor 不同**（一次 draw 只能一个 scissor，见 `crate::UiBatch::clip`）；
/// 2. **顶点数不超过 `max_verts`**——受 u16 索引上限约束。
///
/// 返回顺序 = 绘制顺序。**段数 = draw call 数**，因此本函数是「尽量减少 DrawCall」
/// 这一目标的唯一裁决点，抽出以便直接断言。
pub(crate) fn segment_runs(
    quads: impl Iterator<Item = (u32, u64, Option<Rect>, usize)>,
    max_verts: usize,
) -> Vec<SegRun> {
    let mut runs: Vec<SegRun> = Vec::new();
    for (window, texture, clip, verts) in quads {
        let fits = runs.last().is_some_and(|r| {
            r.window == window
                && r.texture == texture
                && r.clip == clip
                && r.verts + verts <= max_verts
        });
        if fits {
            let r = runs.last_mut().expect("just checked");
            r.quads += 1;
            r.verts += verts;
        } else {
            runs.push(SegRun { window, texture, clip, quads: 1, verts });
        }
    }
    runs
}

/// **命令是否完全落在裁剪层之外**（⇒ 不必镶嵌、不必入段、不必提交）。
///
/// ⚠ 与"几何切割"是**两件事**（阶段 3 之后尤其要分清）：
/// - **剔除（本函数）**：整条命令的矩形与裁剪层**无交集** ⇒ 一个像素都看不见 ⇒ 直接跳过。
///   省的是 **CPU 镶嵌 + 顶点/索引 + 段（draw call）**；半开相交（边沿接触 = 零面积可见）；
/// - **裁剪（batch scissor）**：**部分**重叠 ⇒ 整条照画，越界像素交给 GPU scissor
///   （圆角 / 环带 / 投影因此保持原形——不能用几何求交，那会把圆角切平）。
///
/// 所以这里**只判"全外"**，绝不做部分求交。`clip = None`（无强制层）恒不剔。
#[inline]
pub(crate) fn fully_outside(rect: Rect, clip: Option<Rect>) -> bool {
    match clip {
        None => false,
        Some(c) => !c.intersects(&rect),
    }
}

/// `finish` 顶点缓存各阶段**累计**（µs / 计数）：拆出的缓存/提交子函数共享一个
/// `&mut CacheStats` 累加，`finish` 末尾统一写入 `UiStats`（示例/诊断读取）。
#[derive(Default)]
pub(crate) struct CacheStats {
    /// 内容签名（`cmd_sig` 全量哈希）耗时（µs）。
    pub(crate) sig_us: f64,
    /// 缓存未命中 → 顶点重建（`collect_cmds`）耗时（µs）。
    pub(crate) collect_us: f64,
    /// 缓存命中 → 提交列表组装（顶点克隆）耗时（µs）。
    pub(crate) clone_us: f64,
    pub(crate) cache_hits: u32,
    pub(crate) cache_misses: u32,
    pub(crate) win_count: u32,
}

// ─── 四边形收集器 ─────────────────────────────────────────────

/// **四边形分组键**：`(win, place, elem, group, tex, clip)`。
///
/// `place` = 顶层放置序（见 [`crate::Ui::z0_ranges`](crate::ui::Ui)）：win=0 的排序空间。
/// 末位是**环境裁剪层**（绝对逻辑坐标的量化键，见 [`ClipKey`]）——裁剪不同的几何
/// **不能合进同一段**（一次 draw 只能一个 scissor，见 [`crate::UiBatch::clip`]）。
pub(crate) type QuadKey = (u32, u32, u32, u8, u64, Option<ClipKey>);

/// 按 `(win, place, elem, group, tex, clip)` 分组的几何收集器（`finish` 提交用）。
///
/// ⚠ `place`（顶层放置序）在**键的第 2 位**：win=0 里所有非窗口内容共享 `win = 0`，
/// 没有它就没有独立排序空间，`elem = 0` 的装饰（面板底色 / 滚动条）会被**别的**
/// win=0 放置的内容穿透（历史 bug，见 [`crate::Ui::z0_ranges`](crate::ui::Ui)）。
///
/// `debug` 是**屏幕调试叠加**（DebugDraw 图元 + debug_layout 布局描边）——
/// 按 `win` 分组、恒用白纹理，在全部 UI 内容**之后**提交。
pub(crate) struct QuadCollector {
    pub(crate) quads: std::collections::HashMap<QuadKey, Geom>,
    /// 调试叠加几何（白纹理；窗口局部物理坐标）。
    pub(crate) debug: std::collections::HashMap<u32, Geom>,
    white_uid: u64,
    /// WHITE 纹理区域 UV（字形图集页白纹理 region；兜底为整纹理 [0,1)）。
    white_uv_tl: Vec2,
    white_uv_wh: Vec2,
    /// **当前放置序**（`collect_cmds` 每处理一个命令设置；`win > 0` 恒 0）。
    pub(crate) cur_place: u32,
    /// **当前元素序**（`collect_cmds` 每处理一个命令设置；push 方法按其分组）。
    pub(crate) cur_elem: u32,
    /// **当前环境裁剪层**（绝对逻辑坐标；`collect_cmds` 每处理一个命令设置）——
    /// 与 `cur_place` / `cur_elem` 一起进入分组键。
    pub(crate) cur_clip: Option<Rect>,
    /// **每条几何所属的元素序**（键与 `quads` 同构，供提交期统计
    /// 「一个批次覆盖了多少个元素」——即 [`crate::UiBatchSource::elements`]）。
    pub(crate) elems: std::collections::HashMap<QuadKey, Vec<u32>>,
}

impl QuadCollector {
    pub(crate) fn new(white_uid: u64, white_uv_tl: Vec2, white_uv_wh: Vec2) -> Self {
        Self {
            quads: std::collections::HashMap::new(),
            debug: std::collections::HashMap::new(),
            white_uid,
            white_uv_tl,
            white_uv_wh,
            cur_place: 0,
            cur_elem: 0,
            cur_clip: None,
            elems: std::collections::HashMap::new(),
        }
    }

    /// **本帧当前分组键**（`win` + 当前放置序 + 当前元素序 + 组 + 纹理 + 当前裁剪层）。
    #[inline]
    pub(crate) fn key(&self, win: u32, group: u8, tex: u64) -> QuadKey {
        (win, self.cur_place, self.cur_elem, group, tex, clip_key(self.cur_clip))
    }

    /// 取（或建）某分组键的几何段，并登记本段所属的元素序。
    fn geom(&mut self, key: QuadKey) -> &mut Geom {
        let elem = key.2;
        self.elems.entry(key).or_default().push(elem);
        self.quads.entry(key).or_default()
    }

    /// **白纹理 region 的中心 UV**——所有"纯色顶点"几何必须用它采样。
    ///
    /// ⚠ 绝不能用 `(0, 0)`：图形与字形共用同一张图集页，`(0, 0)` 是字形页左上角，
    /// 采到的是某个字形的像素（通常 `alpha = 0`）⇒ 整块图形变成透明/乱码。
    /// 取中心而非左上角，是为了在过滤采样下也稳稳落在白纹素内。
    #[inline]
    fn white_uv_center(&self) -> [f32; 2] {
        let uv = self.white_uv_tl + self.white_uv_wh * 0.5;
        [uv.x, uv.y]
    }

    /// 白色纹理四边形（背景 / 边框 / 光标；WHITE region UV；图形组）。
    pub(crate) fn push_white(&mut self, win: u32, r: Rect, c: Color) {
        self.push_tex_rect(win, self.white_uid, self.white_uv_tl, self.white_uv_wh, r, c);
    }

    /// **白纹理 + 四角各异的顶点色四边形**（矩形渐变；图形组）。
    ///
    /// 顶点顺序 `[TL, TR, BL, BR]`（与 `QUAD_TRI_INDICIES` 一致），颜色由光栅化器
    /// 双线性插值 ⇒ **无需渐变纹理**。实例 tint 统一由提交端给（窗口 FX tint），
    /// shader 里为 `顶点色 × 实例色`，所以未设 FX 时结果就是这里的顶点色。
    pub(crate) fn push_white_quad(&mut self, win: u32, r: Rect, corners: [Color; 4]) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let [c_tl, c_tr, c_bl, c_br]: [[f32; 4]; 4] =
            [corners[0].into(), corners[1].into(), corners[2].into(), corners[3].into()];
        let uv_tl = self.white_uv_tl;
        let uv_br = uv_tl + self.white_uv_wh;
        let quad = [
            vertex_p3u2c4(Vec2::new(r.x, r.y), [uv_tl.x, uv_tl.y], c_tl),
            vertex_p3u2c4(Vec2::new(r.x + r.w, r.y), [uv_br.x, uv_tl.y], c_tr),
            vertex_p3u2c4(Vec2::new(r.x, r.y + r.h), [uv_tl.x, uv_br.y], c_bl),
            vertex_p3u2c4(Vec2::new(r.x + r.w, r.y + r.h), [uv_br.x, uv_br.y], c_br),
        ];
        let key = self.key(win, GROUP_GRAPHIC, self.white_uid);
        self.geom(key).push_quad(&quad);
    }

    /// 带 UV 子区域的矩形四边形（图形组；WHITE region / 图集区域用）。
    pub(crate) fn push_tex_rect(
        &mut self,
        win: u32,
        tex: u64,
        uv_tl: Vec2,
        uv_wh: Vec2,
        r: Rect,
        c: Color,
    ) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let ca: [f32; 4] = c.into();
        let uv_br = uv_tl + uv_wh;
        let quad = [
            vertex_p3u2c4(Vec2::new(r.x, r.y), [uv_tl.x, uv_tl.y], ca),
            vertex_p3u2c4(Vec2::new(r.x + r.w, r.y), [uv_br.x, uv_tl.y], ca),
            vertex_p3u2c4(Vec2::new(r.x, r.y + r.h), [uv_tl.x, uv_br.y], ca),
            vertex_p3u2c4(Vec2::new(r.x + r.w, r.y + r.h), [uv_br.x, uv_br.y], ca),
        ];
        let key = self.key(win, GROUP_GRAPHIC, tex);
        self.geom(key).push_quad(&quad);
    }

    /// **CPU 镶嵌的圆角矩形**（图形组；白纹理 + 逐顶点色）。
    ///
    /// 硬体 + 羽化带由 [`crate::tess::push_rounded_rect`] 产生；索引直接内联写入
    /// 同一段几何（同源，永不脱节）。`corners` 四角各异 ⇒ 支持「圆角 + 渐变」。
    ///
    /// **不接收 `uv`**：采样 UV 由本方法填成白纹理 region 中心——调用方没有机会写错
    /// （写错 `(0,0)` 会静默采到字形像素，整块图形变透明）。
    pub(crate) fn push_rounded(
        &mut self,
        win: u32,
        table: &crate::tess::CornerTable,
        rect: Rect,
        radius: CornerRadius,
        feather: f32,
        corners: [Color; 4],
    ) -> crate::tess::TessOutput {
        let key = self.key(win, GROUP_GRAPHIC, self.white_uid);
        let uv = self.white_uv_center();
        let g = self.geom(key);
        crate::tess::push_rounded_rect(
            &mut g.verts,
            &mut g.tris,
            table,
            crate::tess::RoundedRectSpec { rect, radius, feather, corners, uv },
        )
    }

    /// **背景图**（图形组；用户纹理 + 逐顶点 UV 的仿射映射）。
    ///
    /// 与实心圆角矩形**同一条镶嵌路径**（[`crate::tess::push_rounded_rect_uv`]）：
    /// - `Stretch` / `Fill` / `Center`：UV 是位置的仿射函数 ⇒ 扇形三角化的重心插值
    ///   精确再现映射，因此**贴图与圆角遮罩共存**且只多一个四边形的几何；
    /// - `Tile`：1:1 平铺 = `tile × tile` 个四边形（每块 UV `0..1`），`radius` 被忽略
    ///   （UV 需要环绕才能让圆角扇形正确取样，见 [`ImageFit::Tile`]）。
    ///
    /// 顶点色恒为 `bg.tint`（乘在纹理上）；羽化带的 alpha 斜坡照旧 ⇒ 圆角边缘是
    /// **图片 alpha 渐隐**（真正的圆角遮罩，而不是把直角图片贴在圆角上）。
    pub(crate) fn push_image(
        &mut self,
        win: u32,
        table: &crate::tess::CornerTable,
        rect: Rect,
        bg: ImageBg,
        feather: f32,
    ) -> crate::tess::TessOutput {
        let Some(layout) = bg.layout(rect) else {
            return crate::tess::TessOutput { verts: 0, tris: 0 };
        };
        let mut total = crate::tess::TessOutput { verts: 0, tris: 0 };
        // `spec.uv` 恒不被使用（逐顶点 UV 优先），但**必须给合法值**——「绝不写 (0,0)」
        // 是这条路径的硬约定（写错会静默采到字形像素）。
        let fallback_uv = self.white_uv_center();
        let g = self.geom(self.key(win, GROUP_GRAPHIC, bg.tex));
        // 铺排矩形左上角 → UV 的线性映射（`uv_at(p) = uv0 + (p - min) * k`）。
        let uv_at = move |p: Vec2| {
            let k = Vec2::new(
                (layout.uv1.x - layout.uv0.x) / layout.rect.w.max(f32::EPSILON),
                (layout.uv1.y - layout.uv0.y) / layout.rect.h.max(f32::EPSILON),
            );
            let u = layout.uv0 + (p - layout.rect.min()) * k;
            [u.x, u.y]
        };
        match layout.tile.and_then(|t| crate::draw::tile_grid(rect, t)) {
            // ── 平铺：逐块四边形（UV 恒 0..1；部分块按比例截断）──
            Some(grid) => {
                for (tr, uv1) in grid {
                    let out = crate::tess::push_plain_uv(
                        &mut g.verts,
                        &mut g.tris,
                        tr,
                        [bg.tint; 4],
                        Vec2::ZERO,
                        uv1,
                    );
                    total.verts += out.verts;
                    total.tris += out.tris;
                }
            }
            // ── 拉伸 / 裁剪 / 居中：圆角（+羽化）路径 ──
            None => {
                total = crate::tess::push_rounded_rect_uv(
                    &mut g.verts,
                    &mut g.tris,
                    table,
                    crate::tess::RoundedRectSpec {
                        rect: layout.rect,
                        radius: bg.radius,
                        feather,
                        corners: [bg.tint; 4],
                        uv: fallback_uv,
                    },
                    &uv_at,
                );
            }
        }
        total
    }

    /// **CPU 镶嵌的圆角边框（环带）**（图形组；白纹理 + 纯色）。
    ///
    /// `feather` 同 [`crate::tess::RoundedRectSpec::feather`]：边框的**内外两条边界**
    /// 都会做羽化斜坡。
    pub(crate) fn push_rounded_ring(
        &mut self,
        win: u32,
        table: &crate::tess::CornerTable,
        rect: Rect,
        radius: CornerRadius,
        width: f32,
        feather: f32,
        color: Color,
    ) -> crate::tess::TessOutput {
        let key = self.key(win, GROUP_GRAPHIC, self.white_uid);
        let uv = self.white_uv_center();
        let g = self.geom(key);
        crate::tess::push_rounded_ring(
            &mut g.verts,
            &mut g.tris,
            table,
            rect,
            radius,
            width,
            feather,
            color,
            uv,
        )
    }

    /// **CPU 镶嵌的圆角软阴影**（图形组；白纹理 + 顶点色 alpha 渐隐）。
    ///
    /// `rect` = 本体矩形、`blur` = 向外渐隐宽度、`offset` = 最外圈偏移；见
    /// [`crate::tess::push_rounded_shadow`]（无纹理、无着色器、不增 draw call）。
    pub(crate) fn push_rounded_shadow(
        &mut self,
        win: u32,
        table: &crate::tess::CornerTable,
        rect: Rect,
        radius: CornerRadius,
        blur: f32,
        offset: Vec2,
        color: Color,
    ) -> crate::tess::TessOutput {
        let key = self.key(win, GROUP_GRAPHIC, self.white_uid);
        let uv = self.white_uv_center();
        let g = self.geom(key);
        crate::tess::push_rounded_shadow(
            &mut g.verts,
            &mut g.tris,
            table,
            rect,
            radius,
            blur,
            offset,
            color,
            uv,
        )
    }

    /// **CPU 镶嵌的矢量图标**（图形组；白纹理 + 纯色）。
    ///
    /// 每个分片是单位方框里的**凸**多边形，这里映射到 `rect` 再交给
    /// [`crate::tess::push_convex`]（硬体 + 沿角平分线外扩的羽化环）。
    pub(crate) fn push_icon(
        &mut self,
        win: u32,
        rect: Rect,
        icon: Icon,
        color: Color,
        feather: f32,
    ) -> crate::tess::TessOutput {
        let key = self.key(win, GROUP_GRAPHIC, self.white_uid);
        let uv = self.white_uv_center();
        let g = self.geom(key);
        let v0 = g.verts.len();
        let t0 = g.tris.len();
        for part in icon.parts() {
            if part.len() < 3 {
                continue;
            }
            let pts: Vec<Vec2> = part
                .iter()
                .map(|p| Vec2::new(rect.x + p.x * rect.w, rect.y + p.y * rect.h))
                .collect();
            crate::tess::push_convex(&mut g.verts, &mut g.tris, &pts, feather, color, uv);
        }
        crate::tess::TessOutput {
            verts: g.verts.len() - v0,
            tris: g.tris.len() - t0,
        }
    }

    /// 追加一个带 UV 的四边形（字形用；文字组）。
    pub(crate) fn push_tex_quad(&mut self, win: u32, tex: u64, quad: [VertexP3U2C4; 4]) {        let key = self.key(win, GROUP_TEXT, tex);
        self.geom(key).push_quad(&quad);
    }

    /// 调试叠加：一条带厚度线段（白纹理实心色；UV 取 WHITE region 中心）。
    /// 几何复用 [`rjw_2d_render::debug_draw::thick_line_quad`]；退化线段（零长 / 零宽）跳过。
    pub(crate) fn push_debug_line(&mut self, win: u32, a: Vec2, b: Vec2, width: f32, c: Color) {
        let Some([tl, tr, bl, br]) = rjw_2d_render::debug_draw::thick_line_quad(a, b, width) else {
            return;
        };
        let ca: [f32; 4] = c.into();
        // UV 必须落在 WHITE region 内（[0,0] 会采样字形页左上角，可能是字形像素）。
        let uv = self.white_uv_tl + self.white_uv_wh * 0.5;
        let quad = [
            vertex_p3u2c4(tl, [uv.x, uv.y], ca),
            vertex_p3u2c4(tr, [uv.x, uv.y], ca),
            vertex_p3u2c4(bl, [uv.x, uv.y], ca),
            vertex_p3u2c4(br, [uv.x, uv.y], ca),
        ];
        self.debug.entry(win).or_default().push_quad(&quad);
    }

    /// 调试叠加：矩形描边（4 条带厚度线段；`width` 为物理像素）。
    pub(crate) fn push_debug_rect(&mut self, win: u32, r: Rect, width: f32, c: Color) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let tl = Vec2::new(r.x, r.y);
        let tr = Vec2::new(r.x + r.w, r.y);
        let br = Vec2::new(r.x + r.w, r.y + r.h);
        let bl = Vec2::new(r.x, r.y + r.h);
        self.push_debug_line(win, tl, tr, width, c);
        self.push_debug_line(win, tr, br, width, c);
        self.push_debug_line(win, br, bl, width, c);
        self.push_debug_line(win, bl, tl, width, c);
    }
}

// ─── 文本切片 / 顶点构造 ──────────────────────────────────────

/// 取视觉行的文本切片（**字符边界安全**）：视觉行的字节区间来自排版缓冲，
/// 编辑（粘贴/打字/IME）同帧改写文本后可能过期——按 char 边界对齐并防
/// `start > end`，避免 `&value[a..b]` 落在多字节字符中间 panic（短暂错位，
/// 次帧重新排版后自愈）。
#[inline]
pub(crate) fn safe_line_slice<'a>(value: &'a str, line: &VisualLine) -> &'a str {
    let s = value.floor_char_boundary(line.byte_start);
    let e = value.floor_char_boundary(line.byte_end.min(value.len()));
    if s < e {
        &value[s..e]
    } else {
        ""
    }
}

/// 文本坐标 y（逻辑像素）→ 视觉行序号：按**真实行顶**（`VisualLine.top`，物理像素）
/// 定位。不要用 `行号 × line_h`——排版行高是 `round(font×1.2)`（取整），逻辑 `line_h`
/// 每行差 ~0.2px，长文本累积后点击/拖选错行、视图滚不到真正的底部（"卡在纵轴
/// 范围内"）。
#[inline]
pub(crate) fn line_row_at_y(vlines: &[VisualLine], y: f32, line_h: f32) -> usize {
    if vlines.is_empty() {
        return 0;
    }
    let lh_px = line_h.round();
    vlines
        .iter()
        .position(|l| y < l.top + lh_px)
        .unwrap_or(vlines.len() - 1)
}

/// 构造一个顶点（世界坐标 + UV + 颜色）。
#[inline]
pub(crate) fn vertex_p3u2c4(pos: Vec2, uv: [f32; 2], color: [f32; 4]) -> VertexP3U2C4 {
    VertexP3U2C4 {
        pos: [pos.x, pos.y, 0.0],
        uv,
        color,
    }
}

/// **按双线性重采样渐变的四角色**：`local` 是（可能被裁剪的）目标矩形，`orig` 是
/// 渐变原本锚定的矩形。
///
/// `local == orig` 时原样返回四角色；被裁剪时按 `local` 各角在 `orig` 中的相对位置
/// 插值，使**颜色的空间锚定不变**（否则裁剪会让整条渐变平移）。`orig` 退化（宽/高为 0）
/// 时回退到 `gradient` 自身。
#[inline]
pub(crate) fn resample_gradient(gradient: Gradient, local: Rect, orig: Rect) -> [Color; 4] {
    if local.x == orig.x && local.y == orig.y && local.w == orig.w && local.h == orig.h {
        return [gradient.tl, gradient.tr, gradient.bl, gradient.br];
    }
    if orig.w <= 0.0 || orig.h <= 0.0 {
        return [gradient.tl, gradient.tr, gradient.bl, gradient.br];
    }
    let at = |x: f32, y: f32| -> Color {
        let u = (x - orig.x) / orig.w;
        let v = (y - orig.y) / orig.h;
        let top = crate::draw::lerp_color(gradient.tl, gradient.tr, u);
        let bot = crate::draw::lerp_color(gradient.bl, gradient.br, u);
        crate::draw::lerp_color(top, bot, v)
    };
    let (l, r) = (local.x, local.x + local.w);
    let (t, b) = (local.y, local.y + local.h);
    [at(l, t), at(r, t), at(l, b), at(r, b)]
}

/// **局部空间的渐变重采样**（[`resample_gradient`] 的调用点助手）。
///
/// `local` 是**窗口局部**物理矩形（已减 `anchor_px`），而调用点手里通常只有命令的
/// **绝对**矩形 `abs`。必须先把 `abs` 换算到同一空间再重采样：
/// ⚠ 直接把绝对矩形当 `orig` 传给 [`resample_gradient`]，`u = (local.x - orig.x)/w`
/// 就会整体偏心 `anchor_px`（窗口内容的渐变被平移，甚至外推出界——`lerp_color`
/// 不过滤，颜色会越界）。这条不变量集中在这里，避免两个调用点各错一次。
#[inline]
pub(crate) fn resample_gradient_local(
    gradient: Gradient,
    local: Rect,
    abs: Rect,
    anchor_px: Vec2,
) -> [Color; 4] {
    let orig = Rect::new(abs.x - anchor_px.x, abs.y - anchor_px.y, abs.w, abs.h);
    resample_gradient(gradient, local, orig)
}

// ─── 内容签名（窗口顶点缓存失效判定） ─────────────────────────

/// **窗口/放置内容签名哈希**（提取为自由函数，便于无 `Ui` 实例的单元测试）。
///
/// 逐命令哈希**一切渲染相关字段**（颜色 / 边框宽 / 圆角 / 对齐 / 光标 / 选择 / 文本 /
/// 环境裁剪层），忽略 `win/seq`。任何影响绘制的内容变化（含 hover/click 变色、传入值
/// 改变）都会改变签名 → 对应窗口 / win=0 放置子槽缓存自动失效重建。⚠ 不可退回"轻量摘要"
/// （曾漏颜色位，导致 hover/click 变色不刷新）。
///
/// **`anchor` = 该窗口/子槽的原点**（物理像素）：`rect` 与 `clip` 都按
/// `值 − anchor` 入签名。理由：顶点缓存里存的是**窗口局部坐标**
/// （`collect_cmds` 里 `r.x - anchor_px.x`），而命令的 `rect` 是**绝对坐标**——
/// 直接哈希绝对值会让"窗口位置变了、内容没变"（拖窗 / 滚窗）也判定为"内容变了"，
/// 每帧整窗重镶嵌（`cache_miss` 恒等于窗数、`collect_us` 不降）。减去 anchor 后
/// 签名只反映"内容"，与缓存里存的坐标口径一致。
pub(crate) fn cmd_sig_hash(
    h: &mut std::collections::hash_map::DefaultHasher,
    d: &UiDraw,
    anchor: Vec2,
) {
    use std::hash::Hash;
    d.depth.hash(h);
    d.elem.hash(h);
    // 局部坐标（与缓存里存的顶点同口径）
    (d.rect.x - anchor.x).to_bits().hash(h);
    (d.rect.y - anchor.y).to_bits().hash(h);
    d.rect.w.to_bits().hash(h);
    d.rect.h.to_bits().hash(h);
    // **环境裁剪层进签名**：它决定几何落到哪个缓存分组（`QuadKey` 末位），也是
    // 批次 scissor 的输入。⚠ 旧实现把 `d.clip` 当"收集期几何切割"的输入、却不哈希它
    // ——裁剪变了（严格窗口开关 / 滚动可视区变化）而命令内容相同时，窗口顶点缓存会
    // 命中旧分组，表现为"裁剪不生效 / 圆角仍是被切平的旧几何"。同样按 anchor 归一到
    // 局部空间（窗口移动不该让裁剪签名失效）。
    let clip_local = d.clip.map(|c| {
        rjw_transform::Rect::new(c.x - anchor.x, c.y - anchor.y, c.w, c.h)
    });
    match clip_key(clip_local) {
        Some(k) => k.hash(h),
        None => 0u8.hash(h),
    }
    match &d.kind {
        DrawKind::Solid(c) => {
            0u8.hash(h);
            color_bits(*c).hash(h);
        }
        DrawKind::RoundedRect { corners, radius } => {
            5u8.hash(h);
            for c in corners {
                color_bits(*c).hash(h);
            }
            radius.tl.to_bits().hash(h);
            radius.tr.to_bits().hash(h);
            radius.br.to_bits().hash(h);
            radius.bl.to_bits().hash(h);
        }
        DrawKind::Rect(g) => {
            6u8.hash(h);
            // 四角颜色是唯一的渲染输入（无纹理、无 stops）——逐角哈希即可。
            color_bits(g.tl).hash(h);
            color_bits(g.tr).hash(h);
            color_bits(g.bl).hash(h);
            color_bits(g.br).hash(h);
        }
        DrawKind::Icon { icon, color } => {
            8u8.hash(h);
            (*icon as u8).hash(h);
            color_bits(*color).hash(h);
        }
        DrawKind::Image(bg) => {
            9u8.hash(h);
            bg.tex.hash(h);
            bg.texel.x.to_bits().hash(h);
            bg.texel.y.to_bits().hash(h);
            (bg.fit as u8).hash(h);
            color_bits(bg.tint).hash(h);
            bg.radius.tl.to_bits().hash(h);
            bg.radius.tr.to_bits().hash(h);
            bg.radius.br.to_bits().hash(h);
            bg.radius.bl.to_bits().hash(h);
        }
        DrawKind::Border { color, width, radius } => {
            1u8.hash(h);
            color_bits(*color).hash(h);
            width.to_bits().hash(h);
            radius.tl.to_bits().hash(h);
            radius.tr.to_bits().hash(h);
            radius.br.to_bits().hash(h);
            radius.bl.to_bits().hash(h);
        }
        DrawKind::Shadow { color, blur, offset, radius } => {
            10u8.hash(h);
            color_bits(*color).hash(h);
            blur.to_bits().hash(h);
            offset.x.to_bits().hash(h);
            offset.y.to_bits().hash(h);
            radius.tl.to_bits().hash(h);
            radius.tr.to_bits().hash(h);
            radius.br.to_bits().hash(h);
            radius.bl.to_bits().hash(h);
        }
        DrawKind::Text {
            text,
            size,
            color,
            align,
            valign,
            family,
            clip,
            buf: _,
        } => {
            2u8.hash(h);
            text.hash(h);
            size.to_bits().hash(h);
            color_bits(*color).hash(h);
            (*align as u8).hash(h);
            (*valign as u8).hash(h);
            family.hash(h);
            // 文本缓存版本号影响排版结果，必须包含在签名中
            TEXT_LINE_HEIGHT_VERSION.hash(h);
            if let Some(c) = clip {
                c.x.to_bits().hash(h);
                c.y.to_bits().hash(h);
                c.w.to_bits().hash(h);
                c.h.to_bits().hash(h);
            }
        }
        DrawKind::Caret { color, width } => {
            3u8.hash(h);
            color_bits(*color).hash(h);
            width.to_bits().hash(h);
        }
        DrawKind::Debug { color, shape } => {
            4u8.hash(h);
            color_bits(*color).hash(h);
            // 形状参数逐字段哈希（DebugShape 未实现 Hash）。
            match shape {
                DebugShape::Line { a, b, width } => {
                    0u8.hash(h);
                    a.x.to_bits().hash(h);
                    a.y.to_bits().hash(h);
                    b.x.to_bits().hash(h);
                    b.y.to_bits().hash(h);
                    width.to_bits().hash(h);
                }
                DebugShape::RectOutline { rect, width } => {
                    1u8.hash(h);
                    rect.x.to_bits().hash(h);
                    rect.y.to_bits().hash(h);
                    rect.w.to_bits().hash(h);
                    rect.h.to_bits().hash(h);
                    width.to_bits().hash(h);
                }
                DebugShape::CircleOutline {
                    center,
                    radius,
                    segments,
                    width,
                } => {
                    2u8.hash(h);
                    center.x.to_bits().hash(h);
                    center.y.to_bits().hash(h);
                    radius.to_bits().hash(h);
                    segments.hash(h);
                    width.to_bits().hash(h);
                }
                DebugShape::Cross { center, half, width } => {
                    3u8.hash(h);
                    center.x.to_bits().hash(h);
                    center.y.to_bits().hash(h);
                    half.to_bits().hash(h);
                    width.to_bits().hash(h);
                }
                DebugShape::Grid { rect, spacing, width } => {
                    4u8.hash(h);
                    rect.x.to_bits().hash(h);
                    rect.y.to_bits().hash(h);
                    rect.w.to_bits().hash(h);
                    rect.h.to_bits().hash(h);
                    spacing.to_bits().hash(h);
                    width.to_bits().hash(h);
                }
            }
        }
    }
}

/// 颜色位模式（签名哈希用）。
#[inline]
pub(crate) fn color_bits(c: Color) -> [u32; 4] {
    let a: [f32; 4] = c.into();
    [
        a[0].to_bits(),
        a[1].to_bits(),
        a[2].to_bits(),
        a[3].to_bits(),
    ]
}

/// debug_layout 模式：给已取整的物理矩形画一圈描边（转窗口局部坐标后写入 debug 叠加）。
/// `dbg = None` 时零开销；`Some((color, width))` 取 `Theme::debug` 样式。
#[inline]
pub(crate) fn debug_layout_outline(
    quads: &mut QuadCollector,
    win: u32,
    anchor_px: Vec2,
    pr: Rect,
    dbg: Option<(Color, f32)>,
) {
    let Some((color, width)) = dbg else {
        return;
    };
    let r = Rect::new(pr.x - anchor_px.x, pr.y - anchor_px.y, pr.w, pr.h);
    quads.push_debug_rect(win, r, width, color);
}

// ─── DrawCall 契约测试 ────────────────────────────────────────

/// **DrawCall 契约测试**：`segment_runs` 是「一次交互产生几次 draw call」的唯一裁决点。
///
/// 这些测试把「尽量减少 DrawCall」变成**可回归断言**，而不是口头承诺——
/// 它们不需要 GPU（纯函数），因此能在 CI 里跑。
#[cfg(test)]
mod batch_contract_tests {
    use super::segment_runs;
    use crate::ui::MAX_UI_SEG_VERTS;

    /// 单窗口单纹理 ⇒ **恰好 1 段 = 1 次 draw call**（不管窗口里有多少控件）。
    #[test]
    fn one_window_one_texture_is_one_draw_call() {
        // 20 个四边形条目，全部同 (win=1, tex=100)。
        let quads = std::iter::repeat_n((1u32, 100u64, None, 4usize), 20);
        let runs = segment_runs(quads, MAX_UI_SEG_VERTS);
        assert_eq!(runs.len(), 1, "同窗口同纹理的 20 个控件必须合批成 1 段");
        assert_eq!(runs[0].quads, 20);
        assert_eq!(runs[0].verts, 80);
    }

    /// 容器 / 控件**不**额外产生 draw call：元素数只影响 `elements` 统计，不影响段数。
    #[test]
    fn elements_do_not_split_batches() {
        // 同 (win, tex) 下 200 个条目（模拟 200 个控件 + 容器背景）仍应 1 段。
        let quads = std::iter::repeat_n((7u32, 3u64, None, 4usize), 200);
        assert_eq!(segment_runs(quads, MAX_UI_SEG_VERTS).len(), 1);
    }

    /// **环境裁剪不同的几何必须切段**：一次 `draw_indexed` 只能有一个 scissor
    /// （见 [`crate::UiBatch::clip`]）。同裁剪仍合批。
    #[test]
    fn clip_change_splits_the_run() {
        let a = Some(rjw_transform::Rect::new(0.0, 0.0, 100.0, 100.0));
        let b = Some(rjw_transform::Rect::new(10.0, 10.0, 50.0, 50.0));
        // 同 clip ⇒ 仍 1 段（并把裁剪层带到段上）
        let same = [(1u32, 10u64, a, 4usize), (1, 10, a, 4)];
        let runs = segment_runs(same.into_iter(), MAX_UI_SEG_VERTS);
        assert_eq!(runs.len(), 1, "同窗口同纹理同裁剪 ⇒ 合批");
        assert_eq!(runs[0].clip, a);
        // clip 变 ⇒ 2 段
        let diff = [(1u32, 10u64, a, 4usize), (1, 10, b, 4)];
        let runs = segment_runs(diff.into_iter(), MAX_UI_SEG_VERTS);
        assert_eq!(runs.len(), 2, "裁剪不同 ⇒ 必须切段（scissor 不能中途改）");
        assert_eq!(runs[0].clip, a);
        assert_eq!(runs[1].clip, b);
        // 无裁剪 vs 有裁剪：同样切段
        let mixed = [(1u32, 10u64, None, 4usize), (1, 10, a, 4)];
        assert_eq!(segment_runs(mixed.into_iter(), MAX_UI_SEG_VERTS).len(), 2);
    }

    /// 窗口数 = 段数下界（不同窗口的实例变换 / tint 不同，无法合并）。
    #[test]
    fn each_window_needs_its_own_run() {
        // 3 个窗口，各 1 个纹理、各若干四边形。
        let quads = [(1u32, 10u64, None, 4usize), (2, 10, None, 4), (3, 10, None, 4)];
        let runs = segment_runs(quads.into_iter(), MAX_UI_SEG_VERTS);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs.iter().map(|r| r.window).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    /// 纹理切换必切段（一次 draw call 只能绑一个纹理）。
    #[test]
    fn texture_switch_splits() {
        // 同窗口，纹理从 10 → 11 → 10：必须 3 段（第 3 段无法与前两段合并）。
        let quads = [(1u32, 10u64, None, 4usize), (1, 11, None, 4), (1, 10, None, 4)];
        let runs = segment_runs(quads.into_iter(), MAX_UI_SEG_VERTS);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs.iter().map(|r| r.texture).collect::<Vec<_>>(), vec![10, 11, 10]);
    }

    /// 超顶点预算切段：段数 = ceil(总量 / 预算)。
    #[test]
    fn vertex_budget_splits_runs() {
        let budget = 100usize;
        // 每段 40 顶点：40+40 = 80 ≤ 100，再加 40 → 120 > 100 ⇒ 2 条/段。
        let quads = std::iter::repeat_n((1u32, 5u64, None, 40usize), 10);
        let runs = segment_runs(quads, budget);
        assert_eq!(runs.len(), 5, "10 条 × 40 顶点 / 预算 100 ⇒ 5 段");
        assert!(runs.iter().all(|r| r.verts <= budget), "每段都不得超预算");
        assert_eq!(runs.iter().map(|r| r.verts).sum::<usize>(), 400);
    }

    /// 单条就超预算的条目独占一段（不得被丢弃 / 不得越界合并）。
    #[test]
    fn oversized_single_quad_gets_its_own_run() {
        let budget = 10usize;
        let quads = [(1u32, 5u64, None, 400usize)];
        let runs = segment_runs(quads.into_iter(), budget);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].verts, 400, "超大条目原样成段（上游有 MAX_UI_SEG_VERTS 兜底）");
    }

    /// 空输入 ⇒ 零段（零 draw call）。
    #[test]
    fn no_quads_means_no_draw_calls() {
        assert!(segment_runs(std::iter::empty(), MAX_UI_SEG_VERTS).is_empty());
    }

    /// **纯色几何必须采样白纹理 region**（回归：屏幕背景整块消失）。
    ///
    /// 图形与字形共用同一张图集页 ⇒ UV `(0,0)` 是**字形页左上角**（某个字形的像素，
    /// 通常 `alpha = 0`）。曾把镶嵌顶点 UV 写死成 `(0,0)`，结果所有圆角矩形
    /// （窗口 / 面板 / 按钮背景）与圆角边框整块变透明——表现就是"窗口背景完全消失"。
    #[test]
    fn solid_geometry_samples_the_white_texel() {
        use super::QuadCollector;
        use crate::draw::CornerRadius;
        use crate::tess::{RoundedRectSpec, TessCache};
        use glam::Vec2;
        use rjw_color::Color;
        use rjw_transform::Rect;

        // 白 region 在页内某个非零位置（真实情况：字形已先占据左上角）。
        let (tl, wh) = (Vec2::new(0.25, 0.5), Vec2::new(0.03125, 0.03125));
        let center = tl + wh * 0.5;
        let mut q = QuadCollector::new(7, tl, wh);
        let table = TessCache::default().table();

        // 圆角矩形 + 圆角边框环带都会写入同一段几何。
        let f = crate::tess::DEFAULT_FEATHER;
        q.push_rounded(1, &table, Rect::new(0.0, 0.0, 60.0, 36.0), 8.0.into(), f, [Color::RED; 4]);
        q.push_rounded_ring(1, &table, Rect::new(0.0, 0.0, 60.0, 36.0), 8.0.into(), 1.0, f, Color::BLUE);

        let geom = q.quads.values().next().expect("至少一段几何");
        assert!(geom.verts.len() > 40, "应有镶嵌顶点");
        for v in &geom.verts {
            assert_eq!(v.uv, [center.x, center.y], "镶嵌顶点必须采样白纹素中心");
            assert_ne!(v.uv, [0.0, 0.0], "绝不能落到字形页左上角");
        }
        // 直角四边形（`push_white`）跨越白 region 的 tl→br——它本来就该这样，
        // 与镶嵌路径的单点 UV 不同。
        let mut q2 = QuadCollector::new(7, tl, wh);
        q2.push_white(1, Rect::new(0.0, 0.0, 4.0, 4.0), Color::WHITE);
        let g2 = q2.quads.values().next().expect("四边形");
        assert_eq!(g2.verts[0].uv, [tl.x, tl.y], "四边形左上角 = region 左上角");
        assert_eq!(g2.verts[3].uv, [tl.x + wh.x, tl.y + wh.y], "四边形右下角 = region 右下角");
        // `RoundedRectSpec` 仍要求显式 UV 与羽化宽：遗漏即编译错误。
        let _ = RoundedRectSpec {
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            radius: CornerRadius::default(),
            feather: 0.0,
            corners: [Color::WHITE; 4],
            uv: [1.0, 1.0],
        };
    }

    /// `Geom::append` 必须把被追加段的索引整体平移（否则拼接后三角形错乱且不 panic）。
    #[test]
    fn geom_append_rebases_indices() {
        use super::Geom;
        use crate::backend::VertexP3U2C4;
        let v = |x: f32| VertexP3U2C4 { pos: [x, 0.0, 0.0], uv: [0.0; 2], color: [0.0; 4] };
        let mut a = Geom::default();
        a.push_quad(&[v(0.0), v(1.0), v(2.0), v(3.0)]);
        let mut b = Geom::default();
        b.push_quad(&[v(4.0), v(5.0), v(6.0), v(7.0)]);
        // 单独一段的索引必然从 0 起
        assert_eq!(b.tris[0], [0, 1, 3]);
        a.append(&b);
        assert_eq!(a.verts.len(), 8);
        assert_eq!(a.tris.len(), 4);
        // 第二段被平移 4
        assert_eq!(a.tris[2], [4, 5, 7]);
        assert_eq!(a.tris[3], [7, 6, 4]);
        for t in &a.tris {
            assert!(t.iter().all(|&i| (i as usize) < a.verts.len()));
        }
    }

    /// 空段判定：顶点或索引任一为空 ⇒ 无几何可提交（防止后端发出零索引 draw）。
    #[test]
    fn geom_empty_detection() {
        use super::Geom;
        let mut g = Geom::default();
        assert!(g.is_empty());
        g.verts.push(Default::default());
        assert!(g.is_empty(), "只有顶点没有三角形不算几何");
        g.tris.push([0, 0, 0]);
        assert!(!g.is_empty());
    }

    /// **局部 / 绝对空间不变量**：渐变重采样结果不得随窗口原点（`anchor_px`）变化。
    ///
    /// 回归：曾把命令的**绝对**矩形直接当 `orig` 传给 `resample_gradient`，而
    /// `local` 已减去 `anchor_px` ⇒ u/v 整体偏心窗口原点，窗口内渐变被平移 / 外推出界。
    #[test]
    fn gradient_resample_is_anchor_invariant() {
        use super::resample_gradient_local;
        use crate::draw::Gradient;
        use glam::Vec2;
        use rjw_color::Color;
        use rjw_transform::Rect;
        let g = Gradient::horizontal(Color::RED, Color::BLUE);
        let size = Vec2::new(200.0, 40.0);
        // 未裁剪：四角色必须恰好是渐变两端色，与窗口原点无关。
        for anchor in [Vec2::ZERO, Vec2::new(37.0, 11.0), Vec2::new(-500.0, 250.0)] {
            let abs = Rect::new(anchor.x + 10.0, anchor.y + 5.0, size.x, size.y);
            let local = Rect::new(10.0, 5.0, size.x, size.y);
            let c = resample_gradient_local(g, local, abs, anchor);
            assert_eq!(c[0], Color::RED, "左上应为左端色（anchor={anchor:?}）");
            assert_eq!(c[2], Color::RED, "左下应为左端色");
            assert_eq!(c[1], Color::BLUE, "右上应为右端色");
            assert_eq!(c[3], Color::BLUE, "右下应为右端色");
        }
    }

    /// 裁剪后颜色仍锚定在**原矩形**上（不随裁剪平移）。
    #[test]
    fn gradient_resample_stays_anchored_when_clipped() {
        use super::resample_gradient_local;
        use crate::draw::Gradient;
        use glam::Vec2;
        use rjw_color::Color;
        use rjw_transform::Rect;
        let g = Gradient::horizontal(Color::RED, Color::BLUE);
        // 原矩形宽 200，裁掉左边 100 ⇒ 剩下右半，左边界应恰为 50% 混色。
        let local = Rect::new(100.0, 0.0, 100.0, 10.0);
        let orig = Rect::new(0.0, 0.0, 200.0, 10.0);
        let c = resample_gradient_local(g, local, orig, Vec2::ZERO);
        let mid = crate::draw::lerp_color(Color::RED, Color::BLUE, 0.5);
        let a: [f32; 4] = c[0].into();
        let b: [f32; 4] = mid.into();
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() < 1e-4, "裁剪后左边界应为 50% 混色，实际 {a:?}");
        }
        assert_eq!(c[1], Color::BLUE, "裁剪后右边界仍是右端色");
    }
}
