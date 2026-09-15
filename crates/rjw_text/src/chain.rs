//! **唯一文本链**：`Text::label(..)` → [`Label`] → 终点（`draw` / `draw_with` / `measure` / `into_buffer`）。
//!
//! 取代旧的 10 条绘制路径与 3 份样式类型（`TextLayout` / `TextRender` / `Style` / `RenderDefaults`）：
//! - 唯一样式类型 [`TextStyle`]：owned / `Clone` / 可存字段，链式 setter 每项 ≤1 参；
//! - 唯一入口 [`Label`]：样式覆盖 + 定位 + 裁剪/缓存 + 终点（终点才提交）；
//! - 唯一字形载荷 [`Glyph`]：`map` 与 `draw_with` 共用一种回调形状（`&Glyph`）；
//! - 唯一定位语言：`at`（世界左上角）/ `center`（内容中心）/ `anchor`（归一化锚点）/ `offset`（像素微调）；
//! - 剔除默认**开启**（`.no_cull()` 关闭）。
//!
//! `Label` 有两种获得方式：
//! - **运行时（绑定世界层 `Render2D`）**：`Frame::text(|t| { t.label("HP").draw(10.0); })`
//!   —— `t: `[`TextCtx`] 同时持有 `&mut Text` 与 `&mut Render2D`，`draw(layer)` 只收 1 个参数；
//! - **独立（不绑定）**：`Text::label(..)` → [`Label::draw_to`]`(&mut r2d, layer)`（2 参）；
//!   未绑定而调用 `draw(layer)` 会 panic 并提示改用 `draw_to`。
//!
//! 低层类型 `GlyphData` / `GlyphType` / `MeasureInfo` / `LineMeasureInfo` 保留（`Glyph` 与 UI 集成面内部使用），
//! **不再出现在 happy path 上**。
//!
//! 存储：字形/行收集写入复用缓冲（`Text` 内部默认缓冲或用户 [`TextBuffer`]），跨帧 clear+填充复用容量。
//! 性能：`Text` 内部对 cosmic-text 排版做 **LRU 缓存**（[`crate::MAX_LAYOUT_CACHE`]）——相同
//! （文本/字号/行高/对齐/attrs）输入经 O(1) 签名命中后返回共享 `Arc<Buffer>`（不深拷贝），
//! 跳过每帧重复整形；空格等无图字形只判定一次；字形图集去碎片重排后自动同步各字形区域。

use std::ops::Range;
use std::sync::Arc;

use arrayvec::ArrayVec;
use glam::Vec2;
use rjw_atlas::AtlasRegion;

#[cfg(feature = "rjw_2d_render")]
use rjw_2d_render::{Layer, Render2D, SpriteRect};
#[cfg(feature = "rjw_2d_render")]
use rjw_color::Color;
use swash::scale::image::Content as SwashContent;
pub use rjw_transform::{Rect, Transform2D};

use cosmic_text::{AttrsOwned, FamilyOwned, Stretch, Weight};
use crate::{Align, Attrs, Buffer, GlyphLocation, Text};

// ─── 内联容量常量 ───────────────────────────────────────────────

/// 文本内联缓冲容量（字节）。
pub const TEXT_INLINE_CAP: usize = 128;
/// 字形簇内联缓冲容量（字节）。
pub const GLYPH_CLUSTER_CAP: usize = 32;

// ─── 公共类型 ─────────────────────────────────────────────────

/// 文本存储：常量/短字符串内联到栈缓冲（零堆分配），动态/长字符串走堆。
#[derive(Clone, Debug)]
pub enum TextStorage {
    /// 内联 UTF-8 字节缓冲
    Inline(ArrayVec<u8, TEXT_INLINE_CAP>),
    /// 堆字符串
    Heap(String),
}

impl TextStorage {
    #[inline]
    fn inline_or_owned(s: &str) -> Self {
        if s.len() <= TEXT_INLINE_CAP {
            let mut v = ArrayVec::new();
            v.try_extend_from_slice(s.as_bytes()).expect("length checked above");
            Self::Inline(v)
        } else {
            Self::Heap(s.to_owned())
        }
    }

    /// 取出文本（布局/测量用）。
    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Inline(v) => std::str::from_utf8(v.as_slice()).unwrap_or(""),
            Self::Heap(s) => s,
        }
    }
}

impl From<&str> for TextStorage {
    #[inline]
    fn from(s: &str) -> Self { Self::inline_or_owned(s) }
}
impl From<String> for TextStorage {
    #[inline]
    fn from(s: String) -> Self { Self::Heap(s) }
}

/// 行距设置：像素值或倍率。
#[derive(Clone, Copy, Debug)]
pub enum LineSpace {
    /// 额外行距（像素）：有效行高 = `size × 1.2 + px`
    Px(f32),
    /// 行距倍率（相对字号）：有效行高 = `size × multiple`
    Multiple(f32),
}

/// 排版缓存策略（**每个文本操作**可指定；默认 [`CachePolicy::Auto`] 保持现状）。
///
/// 作用于内部 LRU 排版缓存（见 [`crate::MAX_LAYOUT_CACHE`]）：
/// - [`CachePolicy::Auto`]：Debug 恒缓存；Release 仅缓存 ≤ [`crate::LARGE_TEXT_CACHE_LIMIT`] 字节的小文本；
/// - [`CachePolicy::Always`]：强制进 LRU（含大文本；注意会挤压 LRU 容量）；
/// - [`CachePolicy::Never`]：不缓存、不写 LRU（每帧整形；适合一次性/超低频文本）；
/// - [`CachePolicy::User`]：不使用内部 LRU（配合 [`Text::label_from`] 由用户持有 `Arc<Buffer>` 管理缓存）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CachePolicy {
    #[default]
    Auto,
    Always,
    Never,
    User,
}

/// 整体测量信息（低层；[`Glyph`] 内部使用）。
#[derive(Clone, Copy, Debug)]
pub struct MeasureInfo {
    /// 排版内容宽高（行盒）
    pub content_size: Vec2,
    /// 行数
    pub line_count: usize,
    /// 字形数
    pub glyph_count: usize,
}

/// 单行测量信息（低层；UI 集成面 [`Text::geometry`](crate::Text::geometry) 的对应实现基础）。
#[derive(Clone, Debug)]
pub struct LineMeasureInfo {
    /// 原始文本行索引
    pub line_i: usize,
    /// 行盒左上角（相对文本视觉原点；**整数像素**——行顶已取整，与字形 tl 一致）
    pub top_left: Vec2,
    /// 行内容宽（像素）
    pub width: f32,
    /// 行高（到下一行顶的步进）
    pub line_height: f32,
    /// 基线 y（相对行盒顶，正数向下）
    pub baseline: f32,
    /// 该行在收集缓冲中的字形范围
    pub glyph_range: Range<usize>,
}

/// 字形类型（[`Glyph::glyph_type`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphType {
    /// 普通字形（单色 mask / 亚像素，可染色）
    Normal,
    /// 彩色字形（Emoji 等内嵌位图，保留原始 RGBA）
    Color,
}

/// 单个字形渲染记录（低层存储；`map` 可原地修改）。
#[derive(Clone, Debug)]
pub struct GlyphData {
    /// 所在行（收集缓冲的 `lines` 数组索引）
    pub line: usize,
    /// 字形精灵左上角（相对文本视觉原点；**整数像素**——收集期对字形位置逐项取整）
    pub top_left: Vec2,
    /// 字形像素宽高
    pub size: Vec2,
    /// 图集区域（像素坐标 + 页 uid）
    pub region: AtlasRegion,
    /// 字形颜色（RGBA，默认白色；最终颜色 = 全局 `color` × 此值）
    pub color: [f32; 4],
    /// 相对层级偏移（叠加到 `draw(layer)` 传入的基础层上；渐变忽略）
    pub layer: f64,
    /// 可选逐字形变换（`None` = 单位变换；渐变忽略）
    pub transform: Option<Transform2D>,
    /// 字形类型（`Normal` 单色可染色 / `Color` 如 Emoji）
    pub glyph_type: GlyphType,
    /// 对应字符（簇）字节（内联）
    cluster: ArrayVec<u8, GLYPH_CLUSTER_CAP>,
}

impl GlyphData {
    /// 对应字符（簇）的 `&str`。
    #[inline]
    pub fn glyph_str(&self) -> &str {
        std::str::from_utf8(&self.cluster).unwrap_or("")
    }
}

// ─── TextStyle（唯一样式类型） ──────────────────────────────────

/// 无借用的完整文本属性（cosmic-text `AttrsOwned`，family 为 `FamilyOwned`，可长期存储）。
pub type OwnedAttrs = AttrsOwned;

/// **唯一样式类型**：owned / `Clone` / 可存字段；链式 setter 每个 ≤1 参。
///
/// 与 `Text` 解耦（可作字段保存、克隆继承：`base.clone().size(..)`）。
/// `Text::style()` / `Text::style_mut()` 持有全局默认样式，[`Label`] 从中继承；
/// 也可经 [`Label::style`] 把保存的样式套到单个标签上。
///
/// 字段语义（`Default`）：`size = 14.0`、`align = Left`、颜色白色（`color = None`）、
/// `origin` / `offset` / `transform` 不设置。
#[derive(Clone, Debug)]
pub struct TextStyle {
    /// 完整无借用文本属性（family 为 `FamilyOwned::Name`，无生命周期）
    pub attrs: OwnedAttrs,
    /// 字号（像素），默认 14.0
    pub size: f32,
    /// 显式行高（None = 由 `line_space` / 字号推导）
    pub line_height: Option<f32>,
    /// 行距（None = 引擎默认：`size × 1.2`）
    pub line_space: Option<LineSpace>,
    /// 对齐，默认 Left
    pub align: Align,
    /// 全局颜色（RGBA；None = 白色）
    pub color: Option<[f32; 4]>,
    /// 归一化锚点（None = (0,0)，即内容左上角）——等价于 [`Label::anchor`]
    pub origin: Option<Vec2>,
    /// 像素偏移（None = (0,0)）——等价于 [`Label::offset`]
    pub offset: Option<Vec2>,
    /// 渲染级变换（None = 单位）
    pub transform: Option<Transform2D>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            attrs: AttrsOwned::new(&Attrs::new()),
            size: 14.0,
            line_height: None,
            line_space: None,
            align: Align::Left,
            color: None,
            origin: None,
            offset: None,
            transform: None,
        }
    }
}

impl TextStyle {
    /// 等价 [`Default`]（字号 14.0 / 左对齐 / 白色）。
    #[inline]
    pub fn new() -> Self { Self::default() }

    /// 字体族名称（转 `FamilyOwned::Name`，owned 可长期存储）。
    #[inline]
    pub fn font_family(mut self, family: impl Into<String>) -> Self {
        self.attrs.family_owned = FamilyOwned::Name(family.into().into());
        self
    }

    /// 完整文本属性（全量覆盖）。
    #[inline]
    pub fn attrs(mut self, attrs: OwnedAttrs) -> Self {
        self.attrs = attrs;
        self
    }

    /// 字号（像素）。
    #[inline]
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// 显式行高（像素）。
    #[inline]
    pub fn line_height(mut self, value: f32) -> Self {
        self.line_height = Some(value);
        self
    }

    /// 行距（像素增量或字号倍率）。未设置 `line_height` 时生效。
    #[inline]
    pub fn line_space(mut self, value: impl Into<LineSpace>) -> Self {
        self.line_space = Some(value.into());
        self
    }

    /// 对齐。
    #[inline]
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// 全局颜色（RGBA）。
    #[inline]
    pub fn color(mut self, color: impl Into<[f32; 4]>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// 字重。
    #[inline]
    pub fn weight(mut self, weight: Weight) -> Self {
        self.attrs.weight = weight;
        self
    }

    /// 斜体；字体无斜体字面时，cosmic-text 打 `FAKE_ITALIC` 标记、光栅化做 14° 斜切合成伪斜体。
    #[inline]
    pub fn italic(mut self, italic: bool) -> Self {
        self.attrs.style = if italic { cosmic_text::Style::Italic } else { cosmic_text::Style::Normal };
        self
    }

    /// 拉伸。
    #[inline]
    pub fn stretch(mut self, stretch: Stretch) -> Self {
        self.attrs.stretch = stretch;
        self
    }

    /// 字距（EM）。
    #[inline]
    pub fn letter_spacing(mut self, letter_spacing: f32) -> Self {
        self.attrs.letter_spacing_opt = Some(cosmic_text::LetterSpacing(letter_spacing));
        self
    }

    /// 归一化锚点（[0,1]；`(0,0)` 左上角，`(0.5,0.5)` 居中）。等价 [`Label::anchor`]。
    #[inline]
    pub fn origin(mut self, origin: impl Into<Vec2>) -> Self {
        self.origin = Some(origin.into());
        self
    }

    /// 像素偏移。等价 [`Label::offset`]。
    #[inline]
    pub fn offset(mut self, offset: impl Into<Vec2>) -> Self {
        self.offset = Some(offset.into());
        self
    }

    /// 渲染级变换（`None` = 单位；作用于整个文本块）。
    #[inline]
    pub fn transform(mut self, transform: impl Into<Option<Transform2D>>) -> Self {
        self.transform = transform.into();
        self
    }
}

// ─── TextBuffer（复用缓冲） ─────────────────────────────────────

/// 用户可持有的可复用字形/行缓冲（[`Label::into_buffer`] / UI 集成面使用；
/// 跨帧 clear+填充，容量保留）。
#[derive(Clone, Debug, Default)]
pub struct TextBuffer {
    /// 字形记录
    pub glyphs: Vec<GlyphData>,
    /// 行信息
    pub lines: Vec<LineMeasureInfo>,
}

// ─── Gradient（唯一点阵渐变表达） ───────────────────────────────

/// 渐变应用方式（[`Gradient`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientMode {
    /// 每个字形自身渐变
    Glyph,
    /// 整行渐变（同一行所有字形共享行跨度）
    Line,
    /// 整个文本块渐变（跨行）
    Frame,
}

/// 渐变方向（[`Gradient`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientAxis {
    /// 横向渐变（左 → 右）
    Horizontal,
    /// 竖向渐变（上 → 下）
    Vertical,
}

/// 文本渐变：`mode`（Glyph/Line/Frame）× `axis`（H/V）+ 颜色停靠点。
///
/// 用命名构造器表达六种组合（`glyph_h` / `line_v` / `frame_h` …）；`stops` 至少两项，
/// `t ∈ [0,1]`（越界钳制，超出两端取端点色）。
#[cfg(feature = "rjw_2d_render")]
#[derive(Clone, Debug)]
pub struct Gradient {
    /// 渐变域（字形自身 / 整行 / 整块）
    pub mode: GradientMode,
    /// 渐变方向
    pub axis: GradientAxis,
    /// 颜色停靠点（`(t, color)`，至少两项）
    pub stops: Vec<(f32, Color)>,
}

#[cfg(feature = "rjw_2d_render")]
impl Gradient {
    fn of(mode: GradientMode, axis: GradientAxis, stops: &[(f32, Color)]) -> Self {
        Self { mode, axis, stops: stops.to_vec() }
    }

    /// 逐字形 × 横向（左 → 右）。
    pub fn glyph_h(stops: &[(f32, Color)]) -> Self { Self::of(GradientMode::Glyph, GradientAxis::Horizontal, stops) }
    /// 逐字形 × 竖向（上 → 下）。
    pub fn glyph_v(stops: &[(f32, Color)]) -> Self { Self::of(GradientMode::Glyph, GradientAxis::Vertical, stops) }
    /// 整行 × 横向。
    pub fn line_h(stops: &[(f32, Color)]) -> Self { Self::of(GradientMode::Line, GradientAxis::Horizontal, stops) }
    /// 整行 × 竖向。
    pub fn line_v(stops: &[(f32, Color)]) -> Self { Self::of(GradientMode::Line, GradientAxis::Vertical, stops) }
    /// 整块 × 横向（跨行）。
    pub fn frame_h(stops: &[(f32, Color)]) -> Self { Self::of(GradientMode::Frame, GradientAxis::Horizontal, stops) }
    /// 整块 × 竖向（跨行）。
    pub fn frame_v(stops: &[(f32, Color)]) -> Self { Self::of(GradientMode::Frame, GradientAxis::Vertical, stops) }
}

// ─── Glyph（唯一字形载荷） ─────────────────────────────────────

/// **唯一字形载荷**：`map`（可原地修改）与 `draw_with`（只读）共用同一种回调形状。
///
/// 只读访问器：`region` / `top_left` / `size` / `transform` / `line_index` / `glyph_str` /
/// `glyph_type` / `color` / `layer`；修改访问器：`color_mut` / `translate` / `set_transform` /
/// `set_layer`（仅在 [`Label::map`] 中有效——`draw_with` 收到的是只读快照语义的值）。
#[derive(Clone, Debug)]
pub struct Glyph {
    line: usize,
    top_left: Vec2,
    size: Vec2,
    region: AtlasRegion,
    color: [f32; 4],
    layer: f64,
    transform: Option<Transform2D>,
    glyph_type: GlyphType,
    cluster: ArrayVec<u8, GLYPH_CLUSTER_CAP>,
    /// 已组合的世界变换（`draw_with` 时Some；`map` 中为 None → 返回局部变换）。
    world: Option<Transform2D>,
}

impl Glyph {
    fn from_data(g: &GlyphData) -> Self {
        Self {
            line: g.line,
            top_left: g.top_left,
            size: g.size,
            region: g.region,
            color: g.color,
            layer: g.layer,
            transform: g.transform,
            glyph_type: g.glyph_type,
            cluster: g.cluster.clone(),
            world: None,
        }
    }

    fn write_back(&self, g: &mut GlyphData) {
        g.top_left = self.top_left;
        g.size = self.size;
        g.color = self.color;
        g.layer = self.layer;
        g.transform = self.transform;
        g.glyph_type = self.glyph_type;
    }

    /// 图集区域（像素坐标 + 页 uid）。
    #[inline]
    pub fn region(&self) -> AtlasRegion { self.region }

    /// 字形精灵左上角（相对文本视觉原点；`map` 中为局部坐标）。
    #[inline]
    pub fn top_left(&self) -> Vec2 { self.top_left }

    /// 字形像素宽高。
    #[inline]
    pub fn size(&self) -> Vec2 { self.size }

    /// 字形变换：
    /// - `draw_with`：**世界变换**（已含字形位置 + 渲染级 `transform`）——
    ///   `transform().transform_point(Vec2::ZERO)` 即字形左上角世界坐标；
    /// - `map`：局部变换（仅字形自身 `transform` + 位置，未叠加文本块定位）。
    #[inline]
    pub fn transform(&self) -> Transform2D {
        match self.world {
            Some(t) => t,
            None => {
                let base = self.transform.unwrap_or(Transform2D::IDENTITY);
                base.with_pos(base.pos + self.top_left)
            }
        }
    }

    /// 所在视觉行（0 = 首行）。
    #[inline]
    pub fn line_index(&self) -> usize { self.line }

    /// 对应字符（簇）。
    #[inline]
    pub fn glyph_str(&self) -> &str {
        std::str::from_utf8(&self.cluster).unwrap_or("")
    }

    /// 字形类型（`Color` = Emoji 等彩色字形，保留原色）。
    #[inline]
    pub fn glyph_type(&self) -> GlyphType { self.glyph_type }

    /// 字形颜色（RGBA；最终颜色 = 全局色 × 此值；彩色字形保留自身 RGBA）。
    #[inline]
    pub fn color(&self) -> [f32; 4] { self.color }

    /// 相对层级偏移（叠加到 `draw(layer)` 的基础上）。
    #[inline]
    pub fn layer(&self) -> f64 { self.layer }

    /// 修改字形颜色（[`Label::map`] 中用）。
    #[inline]
    pub fn color_mut(&mut self) -> &mut [f32; 4] { &mut self.color }

    /// 平移字形（[`Label::map`] 中用；叠加到收集期位置）。
    #[inline]
    pub fn translate(&mut self, delta: impl Into<Vec2>) { self.top_left += delta.into(); }

    /// 设置逐字形变换（`None` = 单位；[`Label::map`] 中用）。
    #[inline]
    pub fn set_transform(&mut self, transform: impl Into<Option<Transform2D>>) {
        self.transform = transform.into();
    }

    /// 设置相对层级偏移（[`Label::map`] 中用）。
    #[inline]
    pub fn set_layer(&mut self, layer: f64) { self.layer = layer; }
}

// ─── Label（唯一链） ───────────────────────────────────────────

/// 文本链句柄：文本 + 样式 + 终点选项（+ 可选 `&mut Render2D` 绑定）。
///
/// 由 [`Text::label`]、[`Text::label_from`] 或 [`TextCtx::label`] 构造；
/// 样式覆盖 / 定位 / 裁剪 / 缓存均为消费式链式方法，**终点才提交**。
#[cfg(feature = "rjw_2d_render")]
pub struct Label<'t> {
    text: &'t mut Text,
    string: TextStorage,
    /// 直接复用已排版缓冲（[`Text::label_from`]；不重新整形）。
    bound: Option<&'t Arc<Buffer>>,
    style: TextStyle,
    /// 定位点（`at` / `center` 设置；内容按 `anchor` 相对它摆放）。
    at: Vec2,
    /// 归一化锚点（[0,1]；样式 `origin` 继承）。
    anchor: Vec2,
    /// 像素偏移（叠加在锚点换算之后；样式 `offset` 继承）。
    offset: Vec2,
    /// 文本局部裁剪（相对字形 `top_left`，不含定位）。
    clip: Option<Rect>,
    /// 世界坐标裁剪（整块 + 逐字形保守剔除）。
    clip_world: Option<Rect>,
    /// 剔除开关（默认 **true**；`.no_cull()` 关闭）。
    cull: bool,
    /// 排版缓存策略（默认 [`CachePolicy::Auto`]）。
    cache: CachePolicy,
    /// 渐变（Some 时走渐变渲染路径）。
    gradient: Option<Gradient>,
    /// 逐字形修改闭包。
    #[allow(clippy::type_complexity, reason = "闭包类型本身即 `Option<Box<dyn FnMut>>`，别名不增益")]
    mapper: Option<Box<dyn FnMut(&mut Glyph) + 't>>,
    /// 绑定的世界层渲染器（[`TextCtx::label`] 提供；`None` = 独立用法）。
    r2d: Option<&'t mut Render2D>,
}

#[cfg(feature = "rjw_2d_render")]
impl<'t> Label<'t> {
    /// 内部构造：继承 `text` 的全局默认样式。
    pub(crate) fn new(text: &'t mut Text, string: TextStorage) -> Self {
        let style = text.style.clone();
        Self {
            anchor: style.origin.unwrap_or(Vec2::ZERO),
            offset: style.offset.unwrap_or(Vec2::ZERO),
            at: Vec2::ZERO,
            bound: None,
            gradient: None,
            mapper: None,
            r2d: None,
            clip: None,
            clip_world: None,
            cull: true,
            cache: CachePolicy::Auto,
            string,
            text,
            style,
        }
    }

    // ── 样式覆盖（与 [`TextStyle`] 同名同义） ──

    /// 绑定已排版缓冲（[`Text::label_from`](crate::Text::label_from) 内部使用）。
    #[inline]
    pub(crate) fn bind_buffer(mut self, buffer: &'t Arc<Buffer>) -> Self {
        self.bound = Some(buffer);
        self
    }

    /// 字号（像素）。
    #[inline]
    pub fn size(mut self, size: f32) -> Self { self.style = self.style.size(size); self }

    /// 全局颜色（RGBA）。
    #[inline]
    pub fn color(mut self, color: impl Into<[f32; 4]>) -> Self { self.style = self.style.color(color); self }

    /// 对齐。
    #[inline]
    pub fn align(mut self, align: Align) -> Self { self.style = self.style.align(align); self }

    /// 显式行高（像素）。
    #[inline]
    pub fn line_height(mut self, value: f32) -> Self { self.style = self.style.line_height(value); self }

    /// 行距（像素增量或字号倍率）。
    #[inline]
    pub fn line_space(mut self, value: impl Into<LineSpace>) -> Self { self.style = self.style.line_space(value); self }

    /// 字体族名称（空字符串回退系统默认）。
    #[inline]
    pub fn font_family(mut self, family: impl Into<String>) -> Self { self.style = self.style.font_family(family); self }

    /// 字重。
    #[inline]
    pub fn weight(mut self, weight: Weight) -> Self { self.style = self.style.weight(weight); self }

    /// 斜体（无斜体字面时由光栅化合成伪斜体）。
    #[inline]
    pub fn italic(mut self, italic: bool) -> Self { self.style = self.style.italic(italic); self }

    /// 字距（EM）。
    #[inline]
    pub fn letter_spacing(mut self, letter_spacing: f32) -> Self {
        self.style = self.style.letter_spacing(letter_spacing);
        self
    }

    /// 渲染级变换（`None` = 单位；作用于整个文本块）。
    #[inline]
    pub fn transform(mut self, transform: impl Into<Option<Transform2D>>) -> Self {
        self.style.transform = transform.into();
        self
    }

    /// **整体套用已保存的样式**（[`TextStyle`] 可存字段 / `Clone`；链上调用会覆盖之前的样式设置）。
    #[inline]
    pub fn style(mut self, style: TextStyle) -> Self {
        self.anchor = style.origin.unwrap_or(Vec2::ZERO);
        self.offset = style.offset.unwrap_or(Vec2::ZERO);
        self.style = style;
        self
    }

    // ── 定位 ──

    /// 定位点：内容按 `anchor`（默认左上角）摆放到该点。
    #[inline]
    pub fn at(mut self, pos: impl Into<Vec2>) -> Self { self.at = pos.into(); self }

    /// 以**内容中心**定位（等价 `anchor((0.5,0.5)).at(pos)`）。
    #[inline]
    pub fn center(mut self, pos: impl Into<Vec2>) -> Self {
        self.at = pos.into();
        self.anchor = Vec2::splat(0.5);
        self
    }

    /// 归一化锚点（`0..1`；`(0,0)` 左上角，`(0.5,0.5)` 居中）——锚点落在 `at` 上。
    #[inline]
    pub fn anchor(mut self, anchor: impl Into<Vec2>) -> Self { self.anchor = anchor.into(); self }

    /// 像素偏移（在锚点换算之后叠加）。
    #[inline]
    pub fn offset(mut self, offset: impl Into<Vec2>) -> Self { self.offset = offset.into(); self }

    // ── 裁剪 / 剔除 / 缓存 / 渐变 / 逐字形 ──

    /// 文本**局部**裁剪（相对字形 `top_left` 坐标，不含定位/变换）。
    #[inline]
    pub fn clip(mut self, clip: impl Into<Option<Rect>>) -> Self { self.clip = clip.into(); self }

    /// **世界坐标**裁剪（整块 + 逐字形保守剔除）。
    #[inline]
    pub fn clip_world(mut self, clip: impl Into<Option<Rect>>) -> Self { self.clip_world = clip.into(); self }

    /// 关闭裁剪剔除（默认开启）。
    ///
    /// 默认开启时 `clip` / `clip_world` 生效（收集期 + 提交期剔除）；关闭后两者都被忽略。
    /// 注：视口剔除由 `Render2D` 自身的剔除模式负责（与本开关无关）。
    #[inline]
    pub fn no_cull(mut self) -> Self { self.cull = false; self }

    /// 排版缓存策略（默认 [`CachePolicy::Auto`]）。
    #[inline]
    pub fn cache(mut self, policy: CachePolicy) -> Self { self.cache = policy; self }

    /// 渐变渲染（替代纯色；见 [`Gradient`]）。
    #[inline]
    pub fn gradient(mut self, gradient: Gradient) -> Self { self.gradient = Some(gradient); self }

    /// 逐字形修改（收集后、提交/回调前应用；可改颜色 / 位置 / 层级 / 变换）。
    #[inline]
    pub fn map<F>(mut self, f: F) -> Self
    where F: FnMut(&mut Glyph) + 't {
        self.mapper = Some(Box::new(f));
        self
    }

    // ── 终点 ──

    /// 提交到**绑定的** `Render2D`（[`TextCtx::label`] 提供的链）。
    ///
    /// 独立用法（`Text::label(..)`，未绑定）请改用 [`Self::draw_to`]；未绑定而调用本方法会 panic。
    pub fn draw(self, layer: impl Into<Layer>) {
        let mut label = self;
        match label.r2d.take() {
            Some(r2d) => label.draw_to(r2d, layer),
            None => panic!(
                "Label::draw(layer) 需要绑定的 Render2D：\
                 运行时路径用 `Frame::text(|t| t.label(..).draw(layer))`；\
                 独立路径改用 `Label::draw_to(&mut r2d, layer)`"
            ),
        }
    }

    /// 提交到给定 `Render2D`（独立用法；两参）。
    pub fn draw_to(mut self, r2d: &mut Render2D, layer: impl Into<Layer>) {
        let layer: Layer = layer.into();
        let buffer = self.resolve_buffer();
        let clip = if self.cull { self.clip } else { None };
        let (content_size, _measure, page_size) = collect_into_scratch(&mut *self.text, &buffer, clip);
        let delta = block_delta(content_size, self.at, self.anchor, self.offset);
        if let Some(m) = self.mapper.as_mut() {
            apply_map(&mut self.text.buf.glyphs, m);
        }
        let gradient = self.gradient.take();
        let resolved = Resolved {
            glyphs: &self.text.buf.glyphs,
            lines: &self.text.buf.lines,
            content_size,
            delta,
            render: self.style.transform,
            clip: self.clip,
            clip_world: self.clip_world,
            cull: self.cull,
            page_size,
        };
        match gradient {
            Some(g) => resolved.draw_gradient(r2d, &g, layer),
            None => resolved.draw_sprites(r2d, self.style.color.unwrap_or([1.0; 4]), layer.as_f64()),
        }
    }

    /// 逐字形回调（不绘制）：回调收到**世界坐标**语义的 [`Glyph`]。
    pub fn draw_with<F: FnMut(&Glyph)>(mut self, mut f: F) {
        let buffer = self.resolve_buffer();
        let clip = if self.cull { self.clip } else { None };
        let (content_size, _measure, page_size) = collect_into_scratch(&mut *self.text, &buffer, clip);
        let _ = page_size;
        let delta = block_delta(content_size, self.at, self.anchor, self.offset);
        if let Some(m) = self.mapper.as_mut() {
            apply_map(&mut self.text.buf.glyphs, m);
        }
        let render = self.style.transform;
        if self.cull
            && let Some(cw) = self.clip_world
                && !block_world_rect(content_size, delta, render).intersects(&cw) {
                    return;
                }
        for g in self.text.buf.glyphs.iter() {
            let tl = g.top_left + delta;
            if self.cull
                && let Some(c) = self.clip
                    && !Rect::new(g.top_left.x, g.top_left.y, g.size.x, g.size.y).intersects(&c) {
                        continue;
                    }
            let world = world_transform(g, tl, render);
            if self.cull
                && let Some(cw) = self.clip_world
                    && !world_aabb(g, world).intersects(&cw) {
                        continue;
                    }
            let mut view = Glyph::from_data(g);
            view.world = Some(world);
            f(&view);
        }
    }

    /// 测量：排版内容宽高（行盒；不提交、不光栅化）。
    pub fn measure(mut self) -> Vec2 {
        let buffer = self.resolve_buffer();
        Text::measure_buffer(&buffer)
    }

    /// 排版并交出共享 `Arc<Buffer>`（cosmic-text；缓存命中间接共享，不深拷贝），消费链。
    ///
    /// 字形/行信息收集到 `buf`（跨帧 clear+填充，容量保留）。
    pub fn into_buffer(mut self, buf: &mut TextBuffer) -> Arc<Buffer> {
        let buffer = self.resolve_buffer();
        rasterize_all(&mut *self.text, &buffer);
        let visual_origin = self.text.buffer_origin(&buffer);
        let clip = if self.cull { self.clip } else { None };
        collect_glyphs(
            &self.text.locations,
            &buffer,
            visual_origin,
            &mut buf.glyphs,
            &mut buf.lines,
            clip,
        );
        buffer
    }

    /// 排版缓冲：已绑定 `Buffer` 直接复用（不重新整形），否则按样式整形（走 LRU 排版缓存）。
    fn resolve_buffer(&mut self) -> Arc<Buffer> {
        if let Some(b) = self.bound {
            return Arc::clone(b);
        }
        let attrs = self.style.attrs.as_attrs();
        let lh = effective_line_height(self.style.size, self.style.line_height, self.style.line_space);
        self.text.create_buffer_policy(
            self.string.as_str(),
            attrs,
            self.style.size,
            lh,
            self.style.align,
            self.cache,
        )
    }
}

// ─── TextCtx（运行时绑定） ──────────────────────────────────────

/// 运行时文本上下文：同时持有 `&mut Text` 与 `&mut Render2D`（世界层）。
///
/// 由 [`Frame::text`](crate::Text) 提供；[`Self::label`] 返回的 [`Label`] 已绑定该 `Render2D`，
/// 因此终点 `draw(layer)` **只收 1 个参数**。
#[cfg(feature = "rjw_2d_render")]
pub struct TextCtx<'a> {
    text: &'a mut Text,
    r2d: &'a mut Render2D,
}

#[cfg(feature = "rjw_2d_render")]
impl<'a> TextCtx<'a> {
    /// 构造（运行时 [`Frame::text`](crate::Text) 使用）。
    #[inline]
    pub fn new(text: &'a mut Text, r2d: &'a mut Render2D) -> Self { Self { text, r2d } }

    /// 起链：文本 → [`Label`]（已绑定本上下文的 `Render2D`）。
    #[inline]
    pub fn label<'t>(&'t mut self, text: impl Into<TextStorage>) -> Label<'t> {
        let mut label = Label::new(&mut *self.text, text.into());
        label.r2d = Some(&mut *self.r2d);
        label
    }

    /// 从**已排版** `Arc<Buffer>` 起链（不重新整形；已绑定本上下文的 `Render2D`）。
    ///
    /// 语义同 [`Text::label_from`]，但终点仍是一参的 `draw(layer)`。
    #[inline]
    pub fn label_from<'t>(&'t mut self, buffer: &'t Arc<Buffer>) -> Label<'t> {
        let mut label = Label::new(&mut *self.text, TextStorage::from("")).bind_buffer(buffer);
        label.r2d = Some(&mut *self.r2d);
        label
    }

    /// **UI / 大文本集成面**：排版缓冲（语义与 [`Text::buffer`] 完全一致）。
    #[inline]
    pub fn buffer(
        &mut self,
        text: &str,
        style: &TextStyle,
        wrap: f32,
        policy: CachePolicy,
    ) -> Arc<Buffer> {
        self.text.buffer(text, style, wrap, policy)
    }

    /// **UI 集成面**：排版几何（语义与 [`Text::geometry`] 完全一致）。
    #[inline]
    pub fn geometry(&mut self, buffer: &Buffer) -> crate::TextGeometry {
        self.text.geometry(buffer)
    }

    /// **UI 集成面**：排版内容宽高（语义与 [`Text::measure_buffer`] 完全一致）。
    #[inline]
    pub fn measure_buffer(buffer: &Buffer) -> Vec2 {
        Text::measure_buffer(buffer)
    }

    /// **UI 集成面**：已排版 Buffer 的视觉行（语义与 [`Text::lines`] 完全一致）。
    #[inline]
    pub fn lines(buffer: &Buffer) -> Vec<crate::VisualLine> {
        Text::lines(buffer)
    }

    /// **UI 集成面**：字形图集内的 WHITE region（语义与 [`Text::white_region`] 完全一致）。
    #[inline]
    pub fn white_region(&mut self) -> Option<AtlasRegion> {
        self.text.white_region()
    }

    /// **UI 集成面**：往字形图集插入用户纹理（语义与 [`Text::user_texture`] 完全一致）。
    #[inline]
    pub fn user_texture(&mut self, id: u64, px: rjw_render::Rgba8<'_>) -> Option<AtlasRegion> {
        self.text.user_texture(id, px)
    }

    /// 全局默认样式（[`Text`] 持有；[`Label`] 从中继承）。
    #[inline]
    pub fn style(&self) -> &TextStyle { self.text.style() }

    /// 全局默认样式（可变；改动影响后续所有 [`Label`]）。
    #[inline]
    pub fn style_mut(&mut self) -> &mut TextStyle { self.text.style_mut() }

    /// 字形图集（低层诊断用，如 `page_count()`）——语义同 [`Text::glyph_cache`](crate::Text::glyph_cache)。
    #[inline]
    pub fn glyph_cache(&self) -> &rjw_atlas::DynamicAtlas<crate::AtlasKey> { self.text.glyph_cache() }
}

// ─── 收集 / 提交内核 ───────────────────────────────────────────

/// 文本块定位量：最终位置 = 字形相对坐标 + `delta`。
#[inline]
fn block_delta(content_size: Vec2, at: Vec2, anchor: Vec2, offset: Vec2) -> Vec2 {
    at - Vec2::new(content_size.x * anchor.x, content_size.y * anchor.y) + offset
}

/// 确保 `buffer` 的全部字形已入图集，并同步去碎片重排后的区域。
fn rasterize_all(text: &mut Text, buffer: &Buffer) {
    // **先**同步：图集整理过 ⇒ 已被逐出的字形位置会被丢弃（其槽位可能已被别的字形
    // 复用），下面的循环据此重新光栅化，避免把**指向别人像素**的旧 UV 再烘进顶点。
    text.sync_atlas_regions();
    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            let cache_key = glyph.physical((0.0, 0.0), 1.0).cache_key;
            if text.no_image.contains(&cache_key) {
                continue;
            }
            // 位置存在**且**图集里还在才算可用：只查 `locations` 会漏掉"条目被逐出、
            // 槽位被复用"（缓存 UV 采样到别的字形 → 陈旧文字）。
            let usable = location_usable(
                text.locations.contains_key(&cache_key),
                text.glyph_cache
                    .region_peek(&crate::AtlasKey::Glyph(cache_key))
                    .is_some(),
            );
            if !usable {
                text.rasterize_and_pack(cache_key);
            }
        }
    }
    // 本次插入可能触发整理（搬动 / 页回收）⇒ **再**同步一次，吸收搬动后的区域。
    text.sync_atlas_regions();
}

/// 光栅化 + 收集到 `Text` 内部缓冲，返回 `(内容宽高, 测量, 图集页尺寸)`。
fn collect_into_scratch(text: &mut Text, buffer: &Buffer, clip: Option<Rect>) -> (Vec2, MeasureInfo, f32) {
    rasterize_all(text, buffer);
    let visual_origin = text.buffer_origin(buffer);
    let page_size = text.glyph_cache.page_size() as f32;
    let (content_size, measure) = collect_glyphs(
        &text.locations,
        buffer,
        visual_origin,
        &mut text.buf.glyphs,
        &mut text.buf.lines,
        clip,
    );
    (content_size, measure, page_size)
}

/// 对已收集的字形应用 `map` 闭包（按值包装 → 写回）。
fn apply_map(glyphs: &mut [GlyphData], m: &mut (dyn FnMut(&mut Glyph) + '_)) {
    for g in glyphs.iter_mut() {
        let mut view = Glyph::from_data(g);
        m(&mut view);
        view.write_back(g);
    }
}

/// 文本块世界包围盒（`content_size` 四角经 `delta` 与渲染变换后的保守 AABB）。
fn block_world_rect(content_size: Vec2, delta: Vec2, render: Option<Transform2D>) -> Rect {
    let c = content_size;
    let pts = [
        Vec2::new(0.0, 0.0) + delta,
        Vec2::new(c.x, 0.0) + delta,
        Vec2::new(0.0, c.y) + delta,
        c + delta,
    ];
    match render {
        Some(t) => Rect::from_point_slice(&t.transform_points(&pts)),
        None => Rect::from_point_slice(&pts),
    }
}

/// `draw_with` 语义的逐字形世界变换：`translate(tl) ∘ 字形变换 ∘ 渲染变换`。
fn world_transform(g: &GlyphData, tl: Vec2, render: Option<Transform2D>) -> Transform2D {
    let base = g.transform.unwrap_or(Transform2D::IDENTITY);
    let tr = base.with_pos(base.pos + tl);
    match render {
        Some(t) => tr.compose(&t),
        None => tr,
    }
}

/// `draw_with` 语义的逐字形世界 AABB（世界变换作用于字形局部矩形）。
fn world_aabb(g: &GlyphData, world: Transform2D) -> Rect {
    let local = [
        Vec2::ZERO,
        Vec2::new(g.size.x, 0.0),
        Vec2::new(0.0, g.size.y),
        g.size,
    ];
    Rect::from_point_slice(&world.transform_points(&local))
}

/// 精灵路径的逐字形变换（`Render2D::sprite` 对 rect 施加的变换；不含字形平移）。
fn sprite_transform(g: &GlyphData, render: Option<Transform2D>) -> Transform2D {
    match (render, g.transform) {
        (Some(rt), Some(gt)) => gt.compose(&rt),
        (Some(rt), None) => rt,
        (None, Some(gt)) => gt,
        (None, None) => Transform2D::default(),
    }
}

/// 已解析的提交上下文（字形切片 + 定位 + 裁剪，供纯色 / 渐变两条提交路径共用）。
struct Resolved<'a> {
    glyphs: &'a [GlyphData],
    lines: &'a [LineMeasureInfo],
    content_size: Vec2,
    delta: Vec2,
    render: Option<Transform2D>,
    clip: Option<Rect>,
    clip_world: Option<Rect>,
    cull: bool,
    page_size: f32,
}

#[cfg(feature = "rjw_2d_render")]
impl Resolved<'_> {
    /// 整块世界剔除：不可见 → 整个跳过。
    #[inline]
    fn block_visible(&self) -> bool {
        if !self.cull {
            return true;
        }
        match self.clip_world {
            Some(cw) => block_world_rect(self.content_size, self.delta, self.render).intersects(&cw),
            None => true,
        }
    }

    /// 局部裁剪剔除（字形自身矩形）。
    #[inline]
    fn clipped_out(&self, g: &GlyphData) -> bool {
        if !self.cull {
            return false;
        }
        match self.clip {
            Some(c) => !Rect::new(g.top_left.x, g.top_left.y, g.size.x, g.size.y).intersects(&c),
            None => false,
        }
    }

    /// 纯色路径：逐字形精灵提交。
    fn draw_sprites(&self, r2d: &mut Render2D, color: [f32; 4], layer: f64) {
        if !self.block_visible() {
            return;
        }
        for g in self.glyphs {
            let tl = g.top_left + self.delta;
            if self.clipped_out(g) {
                continue;
            }
            let Some(tex) = r2d.textures().get(g.region.page_uid) else { continue };
            let rect = SpriteRect::with_uv_tex(
                tl,
                g.size,
                Vec2::new(g.region.tl_px.0 as f32, g.region.tl_px.1 as f32),
                Vec2::new(g.region.wh_px.0 as f32, g.region.wh_px.1 as f32),
                &tex,
            );
            let color = if g.glyph_type == GlyphType::Color {
                // 彩色字形（Emoji）：保留自身 RGBA，不叠加全局 tint
                Color::from(g.color)
            } else {
                Color::from(mul_color(color, g.color))
            };
            let transform = sprite_transform(g, self.render);
            if self.cull
                && let Some(cw) = self.clip_world {
                    // 世界坐标 = transform 作用于 rect 四角（mesh_tl + local*mesh_wh）
                    let local = [
                        tl,
                        Vec2::new(tl.x + g.size.x, tl.y),
                        Vec2::new(tl.x, tl.y + g.size.y),
                        tl + g.size,
                    ];
                    let aabb = Rect::from_point_slice(&transform.transform_points(&local));
                    if !aabb.intersects(&cw) {
                        continue;
                    }
                }
            r2d.sprite(rect, &tex)
                .tint(color)
                .transform(transform)
                .layer(Layer::from(layer + g.layer));
        }
    }

    /// 渐变路径：逐字形动态 mesh（逐顶点颜色）。
    fn draw_gradient(&self, r2d: &mut Render2D, gradient: &Gradient, layer: Layer) {
        assert!(gradient.stops.len() >= 2, "Gradient 需要至少 2 个颜色停靠点");
        if self.glyphs.is_empty() {
            return;
        }
        if !self.block_visible() {
            return;
        }
        let (mode, axis) = (gradient.mode, gradient.axis);
        let f32_stops: Vec<(f32, [f32; 4])> =
            gradient.stops.iter().map(|&(t, c)| (t, c.into())).collect();

        // 渐变域（相对坐标，未含 delta）
        let (mut frame_l, mut frame_r) = (f32::MAX, f32::MIN);
        let (mut frame_t, mut frame_b) = (f32::MAX, f32::MIN);
        let n = self.lines.len();
        let mut line_l = vec![f32::MAX; n];
        let mut line_r = vec![f32::MIN; n];
        let mut line_t = vec![f32::MAX; n];
        let mut line_b = vec![f32::MIN; n];
        for g in self.glyphs {
            frame_l = frame_l.min(g.top_left.x);
            frame_r = frame_r.max(g.top_left.x + g.size.x);
            frame_t = frame_t.min(g.top_left.y);
            frame_b = frame_b.max(g.top_left.y + g.size.y);
            line_l[g.line] = line_l[g.line].min(g.top_left.x);
            line_r[g.line] = line_r[g.line].max(g.top_left.x + g.size.x);
            line_t[g.line] = line_t[g.line].min(g.top_left.y);
            line_b[g.line] = line_b[g.line].max(g.top_left.y + g.size.y);
        }

        // 按图集页分组（一个 mesh 只绑一张纹理）
        let mut pages: Vec<(u64, Vec<usize>)> = Vec::new();
        for (i, g) in self.glyphs.iter().enumerate() {
            match pages.iter_mut().find(|(uid, _)| *uid == g.region.page_uid) {
                Some((_, idxs)) => idxs.push(i),
                None => pages.push((g.region.page_uid, vec![i])),
            }
        }

        let delta = self.delta;
        for (uid, idxs) in pages {
            let Some(tex) = r2d.textures().get(uid) else { continue };
            let page_size = self.page_size;
            r2d.mesh_with(|sink| {
                for &i in &idxs {
                    let g = &self.glyphs[i];
                    let tl = g.top_left + delta;
                    let br = tl + g.size;
                    // 剔除（文本局部 + 世界）：渐变域已算完，仅跳过提交。
                    if self.clipped_out(g) {
                        continue;
                    }
                    let tl_w = match self.render { Some(t) => t.transform_point(tl), None => tl };
                    let tr_w = match self.render { Some(t) => t.transform_point(Vec2::new(br.x, tl.y)), None => Vec2::new(br.x, tl.y) };
                    let bl_w = match self.render { Some(t) => t.transform_point(Vec2::new(tl.x, br.y)), None => Vec2::new(tl.x, br.y) };
                    let br_w = match self.render { Some(t) => t.transform_point(br), None => br };
                    if self.cull
                        && let Some(cw) = self.clip_world {
                            let aabb = Rect::from_point_slice(&[tl_w, tr_w, bl_w, br_w]);
                            if !aabb.intersects(&cw) {
                                continue;
                            }
                        }
                    // (渐变域起, 渐变域止, TL角轴坐标, TR角轴坐标, BL角轴坐标, BR角轴坐标)
                    let (s0, s1, t_tl, t_tr, t_bl, t_br) = match (axis, mode) {
                        (GradientAxis::Horizontal, GradientMode::Glyph) => (tl.x, br.x, tl.x, br.x, tl.x, br.x),
                        (GradientAxis::Horizontal, GradientMode::Line) => (
                            line_l[g.line] + delta.x, line_r[g.line] + delta.x,
                            tl.x, br.x, tl.x, br.x,
                        ),
                        (GradientAxis::Horizontal, GradientMode::Frame) => (
                            frame_l + delta.x, frame_r + delta.x,
                            tl.x, br.x, tl.x, br.x,
                        ),
                        (GradientAxis::Vertical, GradientMode::Glyph) => (tl.y, br.y, tl.y, tl.y, br.y, br.y),
                        (GradientAxis::Vertical, GradientMode::Line) => (
                            line_t[g.line] + delta.y, line_b[g.line] + delta.y,
                            tl.y, tl.y, br.y, br.y,
                        ),
                        (GradientAxis::Vertical, GradientMode::Frame) => (
                            frame_t + delta.y, frame_b + delta.y,
                            tl.y, tl.y, br.y, br.y,
                        ),
                    };
                    let uv0 = Vec2::new(
                        g.region.tl_px.0 as f32 / page_size,
                        g.region.tl_px.1 as f32 / page_size,
                    );
                    let uv1 = uv0 + Vec2::new(
                        g.region.wh_px.0 as f32 / page_size,
                        g.region.wh_px.1 as f32 / page_size,
                    );
                    let col = |c: f32| mul_color(sample_gradient(&f32_stops, frac_t(s0, s1, c)), g.color);
                    let i0 = sink.push_vertex_uv_color(tl_w, uv0, col(t_tl));
                    let i1 = sink.push_vertex_uv_color(tr_w, Vec2::new(uv1.x, uv0.y), col(t_tr));
                    let i2 = sink.push_vertex_uv_color(bl_w, Vec2::new(uv0.x, uv1.y), col(t_bl));
                    let i3 = sink.push_vertex_uv_color(br_w, uv1, col(t_br));
                    sink.push_tri(i0, i1, i2);
                    sink.push_tri(i1, i3, i2);
                }
            })
            .tint(Color::WHITE)
            .layer(layer)
            .texture(&tex);
        }
    }
}

// ─── 排版 + 收集 ───────────────────────────────────────────────

/// 缓存的字形位置是否**仍可用**——`cached`（位置表里有）与 `atlas_has`（图集里还在）
/// **两者都成立**才可直接复用。
///
/// 只查位置表会漏掉"条目被逐出、槽位被别的字形复用"：此时旧 `AtlasRegion` 会采样到
/// **别人的像素**（表现为"陈旧文字"），而命令内容签名不变、UI 顶点缓存也不会失效。
/// 只要图集里没了，就必须重新光栅化（重新入图集 + 取新区域）。
#[inline]
pub(crate) fn location_usable(cached: bool, atlas_has: bool) -> bool {
    cached && atlas_has
}

/// 把排版结果收集进 `glyphs` / `lines`（先 clear，复用容量），返回内容宽高与测量。
///
/// `clip`（文本局部坐标，相对字形 `top_left`）：`Some` 时启用**收集期剔除**（轨道 A）——
/// 行/字形与裁剪区无交集的跳过收集；坐标与测量不受影响（剔除只是"不收集"）。
fn collect_glyphs(
    locations: &std::collections::HashMap<cosmic_text::CacheKey, GlyphLocation>,
    buffer: &Buffer,
    visual_origin: Vec2,
    glyphs: &mut Vec<GlyphData>,
    lines: &mut Vec<LineMeasureInfo>,
    clip: Option<Rect>,
) -> (Vec2, MeasureInfo) {
    glyphs.clear();
    lines.clear();

    for run in buffer.layout_runs() {
        let line_idx = lines.len();
        let glyph_start = glyphs.len();
        // 行级（垂直）预剔除：行盒与 clip 无交集 → 整行跳过（仍保留空行信息）。
        if let Some(c) = clip {
            let top = run.line_top - visual_origin.y;
            let bottom = top + run.line_height;
            if top >= c.y + c.h || bottom <= c.y {
                lines.push(LineMeasureInfo {
                    line_i: run.line_i,
                    top_left: Vec2::new(0.0, run.line_top.ceil() - visual_origin.y),
                    width: run.line_w,
                    line_height: run.line_height,
                    baseline: run.line_y - run.line_top,
                    glyph_range: glyph_start..glyph_start,
                });
                continue;
            }
        }
        for glyph in run.glyphs.iter() {
            let physical = glyph.physical((0.0, 0.0), 1.0);
            if let Some(loc) = locations.get(&physical.cache_key) {
                // 字形相对文本视觉原点的偏移：**全部操作数为整数**——`physical.x` /
                // `loc.left` / `loc.top` 为整型，`line_y` 先 `ceil` 再减，`visual_origin`
                // （[`Text::buffer_origin`](crate::Text)）同为整数。整数加减法不会产生小数
                // 误差累加，结果 `tl` 恒为整数（下方 `debug_assert` 兜底）。
                let glyph_pos = Vec2::new(
                    physical.x as f32 + loc.left as f32,
                    run.line_y.ceil() - loc.top as f32 + physical.y as f32,
                );
                let tl = glyph_pos - visual_origin;
                debug_assert!(
                    tl.x.fract() == 0.0 && tl.y.fract() == 0.0,
                    "glyph tl must be integer (整数不变量)，实际 {tl:?}"
                );
                // 字形级（水平 + 垂直）剔除：与 clip 无交集 → 跳过。
                if !crate::glyph_in_clip(clip, tl, Vec2::new(loc.region.wh_px.0 as f32, loc.region.wh_px.1 as f32)) {
                    continue;
                }
                glyphs.push(GlyphData {
                    line: line_idx,
                    top_left: tl,
                    size: Vec2::new(loc.region.wh_px.0 as f32, loc.region.wh_px.1 as f32),
                    region: loc.region,
                    color: [1.0; 4],
                    layer: 0.0,
                    transform: None,
                    glyph_type: glyph_type_of(loc.content),
                    cluster: cluster_of(&run.text[glyph.start..glyph.end]),
                });
            }
        }
        let glyph_end = glyphs.len();
        let min_x = glyphs[glyph_start..glyph_end]
            .iter()
            .map(|g| g.top_left.x)
            .fold(f32::MAX, f32::min);
        lines.push(LineMeasureInfo {
            line_i: run.line_i,
            top_left: Vec2::new(
                if glyph_start < glyph_end { min_x } else { 0.0 },
                // 行盒顶取整到整数（与字形 tl 一致）；行盒顶 = 排版行顶 - 视觉原点 y。
                run.line_top.ceil() - visual_origin.y,
            ),
            width: run.line_w,
            line_height: run.line_height,
            baseline: run.line_y - run.line_top,
            glyph_range: glyph_start..glyph_end,
        });
    }

    let content_size = Text::measure_buffer(buffer);
    let measure = MeasureInfo {
        content_size,
        line_count: lines.len(),
        glyph_count: glyphs.len(),
    };
    (content_size, measure)
}

// ─── 内部工具函数 ───────────────────────────────────────────────

#[inline]
fn glyph_type_of(content: SwashContent) -> GlyphType {
    match content {
        SwashContent::Mask | SwashContent::SubpixelMask => GlyphType::Normal,
        SwashContent::Color => GlyphType::Color,
    }
}

#[inline]
fn cluster_of(s: &str) -> ArrayVec<u8, GLYPH_CLUSTER_CAP> {
    let mut v = ArrayVec::new();
    if v.try_extend_from_slice(s.as_bytes()).is_err() {
        // 超长簇（如长 ZWJ 序列）：截断到合法 UTF-8 前缀
        let mut end = GLYPH_CLUSTER_CAP;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        v.try_extend_from_slice(&s.as_bytes()[..end]).ok();
    }
    v
}

#[inline]
pub(crate) fn effective_line_height(size: f32, line_height: Option<f32>, line_space: Option<LineSpace>) -> f32 {
    let v = match (line_height, line_space) {
        (Some(lh), _) => lh,
        (None, Some(LineSpace::Px(px))) => size * 1.2 + px,
        (None, Some(LineSpace::Multiple(m))) => size * m,
        (None, None) => size * 1.2,
    };
    v.max(0.001)
}

#[cfg(feature = "rjw_2d_render")]
#[inline]
fn mul_color(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

#[cfg(feature = "rjw_2d_render")]
#[inline]
fn frac_t(l: f32, r: f32, x: f32) -> f32 {
    if (r - l).abs() < 1e-6 { 0.0 } else { ((x - l) / (r - l)).clamp(0.0, 1.0) }
}

#[cfg(feature = "rjw_2d_render")]
#[inline]
fn sample_gradient(stops: &[(f32, [f32; 4])], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t <= t1 {
            let f = if (t1 - t0).abs() < 1e-6 { 0.0 } else { (t - t0) / (t1 - t0) };
            return [
                c0[0] + (c1[0] - c0[0]) * f,
                c0[1] + (c1[1] - c0[1]) * f,
                c0[2] + (c1[2] - c0[2]) * f,
                c0[3] + (c1[3] - c0[3]) * f,
            ];
        }
    }
    stops[stops.len() - 1].1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool { (a - b).abs() < 1e-3 }

    #[test]
    fn line_height_resolution() {
        assert!(close(effective_line_height(10.0, Some(18.0), None), 18.0));
        assert!(close(effective_line_height(10.0, None, Some(LineSpace::Multiple(1.5))), 15.0));
        assert!(close(effective_line_height(10.0, None, Some(LineSpace::Px(4.0))), 16.0));
        assert!(close(effective_line_height(10.0, None, None), 12.0));
    }

    #[test]
    fn text_storage_inline_and_heap() {
        let s = TextStorage::from("hello");
        assert!(matches!(s, TextStorage::Inline(_)));
        assert_eq!(s.as_str(), "hello");

        let long = "x".repeat(300);
        let s = TextStorage::from(long.clone());
        assert!(matches!(s, TextStorage::Heap(_)));
        assert_eq!(s.as_str(), long.as_str());

        let s = TextStorage::from(long.as_str());
        assert!(matches!(s, TextStorage::Heap(_)));
        assert_eq!(s.as_str(), long.as_str());
    }

    #[test]
    fn cluster_inline_and_truncate() {
        let c = cluster_of("你");
        assert_eq!(c.as_slice(), "你".as_bytes());
        let long = "x".repeat(50);
        let c = cluster_of(&long);
        assert_eq!(c.len(), GLYPH_CLUSTER_CAP);
        assert_eq!(std::str::from_utf8(c.as_slice()).unwrap(), &long[..GLYPH_CLUSTER_CAP]);
    }

    /// `TextStyle` 默认与旧 `Style::default()` 逐位一致（字号 14 / 左对齐 / 无颜色覆盖）。
    #[test]
    fn text_style_default_matches_old_style() {
        let s = TextStyle::default();
        assert_eq!(s.size, 14.0);
        assert_eq!(s.align, Align::Left);
        assert!(s.line_height.is_none() && s.line_space.is_none());
        assert!(s.color.is_none() && s.origin.is_none() && s.offset.is_none());
        assert!(s.transform.is_none());
        assert_eq!(s.attrs.as_attrs(), Attrs::new());
        // `new()` ≡ `default()`
        let n = TextStyle::new();
        assert_eq!(n.size, s.size);
        assert_eq!(n.align, s.align);
    }

    /// 样式克隆继承：`base.clone().size(..)` 只改差异。
    #[test]
    fn text_style_clone_inherits() {
        let base = TextStyle::new().font_family("SimHei").size(16.0).align(Align::Center);
        let warn = base.clone().size(20.0).color([1.0, 0.0, 0.0, 1.0]);
        assert_eq!(warn.size, 20.0);
        assert_eq!(warn.align, Align::Center);
        assert_eq!(base.size, 16.0);
        assert_eq!(warn.attrs.as_attrs(), base.attrs.as_attrs());
    }

    /// 定位语言：`at` / `center` / `anchor` / `offset` 的换算与旧 `origin`+`offset` 等价。
    #[test]
    fn block_delta_matches_old_origin_offset() {
        let content = Vec2::new(100.0, 20.0);
        // 旧：delta = pos - content * origin（origin 归一化）
        let old = |pos: Vec2, origin: Vec2| pos - Vec2::new(content.x * origin.x, content.y * origin.y);
        // `at` = 左上角（anchor 默认 (0,0)）
        assert_eq!(block_delta(content, Vec2::new(5.0, 6.0), Vec2::ZERO, Vec2::ZERO), old(Vec2::new(5.0, 6.0), Vec2::ZERO));
        // `center` = (0.5,0.5)
        assert_eq!(
            block_delta(content, Vec2::new(5.0, 6.0), Vec2::splat(0.5), Vec2::ZERO),
            old(Vec2::new(5.0, 6.0), Vec2::splat(0.5))
        );
        // `anchor` 任意归一化点
        let a = Vec2::new(0.0, 1.0);
        assert_eq!(block_delta(content, Vec2::new(5.0, 6.0), a, Vec2::ZERO), old(Vec2::new(5.0, 6.0), a));
        // `offset` 在锚点换算之后叠加（= 旧 origin + offset 组合）
        assert_eq!(
            block_delta(content, Vec2::new(5.0, 6.0), Vec2::splat(0.5), Vec2::new(1.0, -2.0)),
            old(Vec2::new(5.0, 6.0), Vec2::splat(0.5)) + Vec2::new(1.0, -2.0)
        );
    }

    #[cfg(feature = "rjw_2d_render")]
    #[test]
    fn gradient_named_constructors() {
        let stops = [(0.0, Color::BLACK), (1.0, Color::WHITE)];
        assert_eq!(Gradient::glyph_h(&stops).mode, GradientMode::Glyph);
        assert_eq!(Gradient::glyph_v(&stops).axis, GradientAxis::Vertical);
        assert_eq!(Gradient::line_h(&stops).mode, GradientMode::Line);
        assert_eq!(Gradient::line_v(&stops).axis, GradientAxis::Vertical);
        assert_eq!(Gradient::frame_h(&stops).mode, GradientMode::Frame);
        assert_eq!(Gradient::frame_v(&stops).axis, GradientAxis::Vertical);
        assert_eq!(Gradient::line_h(&stops).stops.len(), 2);
    }

    #[cfg(feature = "rjw_2d_render")]
    #[test]
    fn gradient_sample_and_lerp() {
        let stops = [(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 1.0, 1.0, 1.0])];
        assert_eq!(sample_gradient(&stops, 0.0), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(sample_gradient(&stops, 0.5), [0.5, 0.5, 0.5, 1.0]);
        assert_eq!(sample_gradient(&stops, 1.0), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(sample_gradient(&stops, 2.0), [1.0, 1.0, 1.0, 1.0]); // 越界钳制
        assert_eq!(frac_t(10.0, 20.0, 15.0), 0.5);
    }
}
