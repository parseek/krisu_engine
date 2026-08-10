//! 责任链 API：`Text::text(..)` → [`TextLayout`]（排版配置）→ [`TextRender`]（渲染配置）。
//!
//! 转换方向单向：`TextLayout` 可经 `into_render()`（用 `Text` 内部缓冲）或 `into_render_with(&mut TextBuffer)`
//! （用户持缓冲，多标签并存）转为 [`TextRender`]，反向不可。
//!
//! - [`TextLayout`]（阶段一）：`text` / `size` / `line_height` / `line_space` / `align` / `attrs` / `font_family`，
//!   及 `measure` / `into_buffer` / `precache` / `into_render` / `into_render_with`。
//! - [`TextRender`]（阶段二，借用缓冲）：`origin` / `origin_px` / `offset` / `color` / `transform` / `map` / `draw_with`；
//!   默认 feature `rjw_2d_render` 下额外提供 `draw_sprite2d` / `draw_2d_gradient`（含横向/竖向渐变）。
//! - [`Style`] / [`TextStyle`]：与 `Text` 解耦的可复用样式（family 用 `AttrsOwned` 无借用），克隆继承。
//!
//! 存储：`TextRender` 借用 `Vec`（`Text` 内部默认缓冲或用户 `TextBuffer`），跨帧 clear+填充复用容量，
//! 无栈内大数组。常量字符串经 [`TextStorage`] 内联存储。
//!
//! 性能：`Text` 内部对 cosmic-text 排版做 **LRU 缓存**（[`MAX_LAYOUT_CACHE`]）——相同
//! （文本/字号/行高/对齐/attrs）输入经 O(1) 签名命中后返回共享 `Arc<Buffer>`（不深拷贝），
//! 跳过每帧重复整形；空格等无图字形只判定一次；字形图集去碎片重排后自动同步各字形区域。

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use arrayvec::ArrayVec;
use glam::Vec2;
use rjw_atlas::AtlasRegion;

#[cfg(feature = "rjw_2d_render")]
use rjw_2d_render::{Layer, Render2D, SpriteRect};
#[cfg(feature = "rjw_2d_render")]
use rjw_color::Color;
#[cfg(feature = "rjw_2d_render")]
use rjw_render::TEXTURES;
use swash::scale::image::Content as SwashContent;
pub use rjw_transform::Transform2D;

use cosmic_text::{AttrsOwned, FamilyOwned, Stretch, Weight};
use crate::{Align, Attrs, Buffer, Family, GlyphLocation, Text};

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

/// 渐变应用方式（`TextRender::draw_2d_gradient`，feature = `rjw_2d_render`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientMode {
    /// 每个字形自身渐变
    Glyph,
    /// 整行渐变（同一行所有字形共享行跨度）
    Line,
    /// 整个文本块渐变（跨行）
    Frame,
}

/// 渐变方向（`TextRender::draw_2d_gradient`，feature = `rjw_2d_render`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientAxis {
    /// 横向渐变（左 → 右）
    Horizontal,
    /// 竖向渐变（上 → 下）
    Vertical,
}

/// 整体测量信息（`draw_with` 回调参数）。
#[derive(Clone, Copy, Debug)]
pub struct MeasureInfo {
    /// 排版内容宽高（行盒）
    pub content_size: Vec2,
    /// 行数
    pub line_count: usize,
    /// 字形数
    pub glyph_count: usize,
}

/// 单行测量信息（`draw_with` 回调参数）。
#[derive(Clone, Debug)]
pub struct LineMeasureInfo {
    /// 原始文本行索引
    pub line_i: usize,
    /// 行盒左上角（相对文本视觉原点；绘制时叠加 `origin` / `offset`）
    pub top_left: Vec2,
    /// 行内容宽（像素）
    pub width: f32,
    /// 行高（到下一行顶的步进）
    pub line_height: f32,
    /// 基线 y（相对行盒顶，正数向下）
    pub baseline: f32,
    /// 该行在 `TextRender::glyphs()` 中的字形范围
    pub glyph_range: Range<usize>,
}

/// 字形类型（`GlyphData::glyph_type`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphType {
    /// 普通字形（单色 mask / 亚像素，可染色）
    Normal,
    /// 彩色字形（Emoji 等内嵌位图，保留原始 RGBA）
    Color,
}

/// 单个字形渲染记录（`map` 可原地修改）。
#[derive(Clone, Debug)]
pub struct GlyphData {
    /// 所在行（`TextRender::lines()` 数组索引）
    pub line: usize,
    /// 字形精灵左上角（相对文本视觉原点；绘制时叠加 `origin` / `offset`）
    pub top_left: Vec2,
    /// 字形像素宽高
    pub size: Vec2,
    /// 图集区域（像素坐标 + 页 uid）
    pub region: AtlasRegion,
    /// 字形颜色（RGBA，默认白色；最终颜色 = 全局 `color` × 此值）
    pub color: [f32; 4],
    /// 相对层级偏移（叠加到 `draw_sprite2d` 传入的基础层上；渐变忽略）
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
}// ─── 样式 Style / TextStyle ─────────────────────────────────────

/// 无借用的完整文本属性（cosmic-text 0.19 `AttrsOwned`，family 为 `FamilyOwned`，可长期存储）。
pub type OwnedAttrs = AttrsOwned;

/// 渲染默认设置（color / origin / offset / transform，均可继承/覆盖）。
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderDefaults {
    /// 全局颜色（RGBA；None = 白色）
    pub color: Option<[f32; 4]>,
    /// 归一化原点（None = (0,0)）
    pub origin: Option<Vec2>,
    /// 像素偏移（None = (0,0)）
    pub offset: Option<Vec2>,
    /// 渲染级变换（None = 单位）
    pub transform: Option<Transform2D>,
}

/// 与 `Text` 完全解耦的可复用样式：可独立存储、克隆继承（`base.clone().size(..)`）。
#[derive(Clone, Debug)]
pub struct Style {
    /// 完整无借用文本属性（family 为 `FamilyOwned::Name`，无生命周期）
    pub attrs: OwnedAttrs,
    /// 字号（像素），默认 14.0
    pub size: f32,
    /// 显式行高（None = 引擎默认）
    pub line_height: Option<f32>,
    /// 行距（None = 引擎默认）
    pub line_space: Option<LineSpace>,
    /// 对齐，默认 Left
    pub align: Align,
    /// 渲染默认
    pub render: RenderDefaults,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            attrs: AttrsOwned::new(&Attrs::new()),
            size: 14.0,
            line_height: None,
            line_space: None,
            align: Align::Left,
            render: RenderDefaults::default(),
        }
    }
}

impl Style {
    /// 字体族名称（转 `FamilyOwned::Name(SmolStr)`，owned 可长期存储）。
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

    /// 字重。
    #[inline]
    pub fn weight(mut self, weight: Weight) -> Self {
        self.attrs.weight = weight;
        self
    }

    /// 斜体。
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

    /// 字号（像素）。
    #[inline]
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// 显式行高。
    #[inline]
    pub fn line_height(mut self, value: f32) -> Self {
        self.line_height = Some(value);
        self
    }

    /// 行距（像素或倍率）。
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
        self.render.color = Some(color.into());
        self
    }

    /// 归一化原点。
    #[inline]
    pub fn origin(mut self, origin: Vec2) -> Self {
        self.render.origin = Some(origin);
        self
    }

    /// 像素偏移。
    #[inline]
    pub fn offset(mut self, offset: Vec2) -> Self {
        self.render.offset = Some(offset);
        self
    }

    /// 渲染级变换。
    #[inline]
    pub fn transform(mut self, transform: impl Into<Option<Transform2D>>) -> Self {
        self.render.transform = transform.into();
        self
    }
}

/// 临时持有的 `Text` + [`Style`]：简化重复的字体/字号/行距设置（`Text::build_style` 构造，可复用）。
pub struct TextStyle<'a> {
    text: &'a mut Text,
    style: Style,
}

impl Text {
    /// 构建可复用样式（临时持有 `&mut Text`；配置可用 [`Style`] 独立保存/克隆继承）。
    #[inline]
    pub fn build_style(&mut self) -> TextStyle<'_> {
        TextStyle { text: self, style: Style::default() }
    }
}

impl<'a> TextStyle<'a> {
    /// 从独立 [`Style`] 构造（样式继承：先建 `Style` 再套用）。
    #[inline]
    pub fn with_style(text: &'a mut Text, style: &Style) -> TextStyle<'a> {
        TextStyle { text, style: style.clone() }
    }

    /// 替换为给定样式。
    #[inline]
    pub fn set_style(&mut self, style: &Style) {
        self.style = style.clone();
    }

    /// 当前样式引用。
    #[inline]
    pub fn style(&self) -> &Style {
        &self.style
    }

    /// 应用文本 → `TextLayout`（继承 style 的布局与渲染默认）。
    #[inline]
    pub fn text(&mut self, text: impl Into<TextStorage>) -> TextLayout<'_> {
        let style = &self.style;
        TextLayout {
            text: &mut *self.text,
            string: text.into(),
            family: None,
            attrs: Some(style.attrs.as_attrs()),
            size: style.size,
            line_height: style.line_height,
            line_space: style.line_space,
            align: style.align,
            render: style.render,
        }
    }

    /// 字体族名称。
    #[inline]
    pub fn font_family(mut self, family: impl Into<String>) -> Self {
        self.style = self.style.font_family(family);
        self
    }
    /// 完整文本属性。
    #[inline]
    pub fn attrs(mut self, attrs: OwnedAttrs) -> Self {
        self.style = self.style.attrs(attrs);
        self
    }
    /// 字重。
    #[inline]
    pub fn weight(mut self, weight: Weight) -> Self {
        self.style = self.style.weight(weight);
        self
    }
    /// 斜体。
    #[inline]
    pub fn italic(mut self, italic: bool) -> Self {
        self.style = self.style.italic(italic);
        self
    }
    /// 拉伸。
    #[inline]
    pub fn stretch(mut self, stretch: Stretch) -> Self {
        self.style = self.style.stretch(stretch);
        self
    }
    /// 字距。
    #[inline]
    pub fn letter_spacing(mut self, letter_spacing: f32) -> Self {
        self.style = self.style.letter_spacing(letter_spacing);
        self
    }
    /// 字号。
    #[inline]
    pub fn size(mut self, size: f32) -> Self {
        self.style = self.style.size(size);
        self
    }
    /// 显式行高。
    #[inline]
    pub fn line_height(mut self, value: f32) -> Self {
        self.style = self.style.line_height(value);
        self
    }
    /// 行距。
    #[inline]
    pub fn line_space(mut self, value: impl Into<LineSpace>) -> Self {
        self.style = self.style.line_space(value);
        self
    }
    /// 对齐。
    #[inline]
    pub fn align(mut self, align: Align) -> Self {
        self.style = self.style.align(align);
        self
    }
    /// 全局颜色。
    #[inline]
    pub fn color(mut self, color: impl Into<[f32; 4]>) -> Self {
        self.style = self.style.color(color);
        self
    }
    /// 归一化原点。
    #[inline]
    pub fn origin(mut self, origin: Vec2) -> Self {
        self.style = self.style.origin(origin);
        self
    }
    /// 像素偏移。
    #[inline]
    pub fn offset(mut self, offset: Vec2) -> Self {
        self.style = self.style.offset(offset);
        self
    }
    /// 渲染级变换。
    #[inline]
    pub fn transform(mut self, transform: impl Into<Option<Transform2D>>) -> Self {
        self.style = self.style.transform(transform);
        self
    }
}// ─── TextBuffer / TextLayout（阶段一：排版配置） ─────────────────

/// 用户可持有的可复用字形/行缓冲（`into_render_with` 使用；跨帧 clear+填充，容量保留）。
#[derive(Clone, Debug, Default)]
pub struct TextBuffer {
    /// 字形记录
    pub glyphs: Vec<GlyphData>,
    /// 行信息
    pub lines: Vec<LineMeasureInfo>,
}

/// 阶段一：排版配置责任链。持有 `&mut Text` 借用；可 `measure` / `into_buffer` / `precache` / `into_render` / `into_render_with`。
pub struct TextLayout<'a> {
    text: &'a mut Text,
    string: TextStorage,
    family: Option<String>,
    attrs: Option<Attrs<'a>>,
    size: f32,
    line_height: Option<f32>,
    line_space: Option<LineSpace>,
    align: Align,
    render: RenderDefaults,
}

impl Text {
    /// 启动一条文本责任链（阶段一：排版配置）。常量字符串内联存储，不堆分配。
    #[inline]
    pub fn text<'a>(&'a mut self, text: impl Into<TextStorage>) -> TextLayout<'a> {
        TextLayout {
            text: self,
            string: text.into(),
            family: None,
            attrs: None,
            size: 14.0,
            line_height: None,
            line_space: None,
            align: Align::Left,
            render: RenderDefaults::default(),
        }
    }
}

impl<'a> TextLayout<'a> {
    /// 替换文本内容。
    #[inline]
    pub fn text(mut self, text: impl Into<TextStorage>) -> Self {
        self.string = text.into();
        self
    }

    /// 字号（像素）。默认 14.0。
    #[inline]
    pub fn size(mut self, value: f32) -> Self {
        self.size = value;
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

    /// 对齐方式。
    #[inline]
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// 完整文本属性；设置后 `font_family` 被忽略。
    #[inline]
    pub fn attrs(mut self, attrs: Attrs<'a>) -> Self {
        self.attrs = Some(attrs);
        self
    }

    /// 字体族名称（如 `"SimHei"`）；传空字符串回退系统默认。
    #[inline]
    pub fn font_family(mut self, family: impl Into<String>) -> Self {
        self.family = Some(family.into());
        self
    }

    /// 渲染默认：颜色（转换到 `TextRender` 时应用）。
    #[inline]
    pub fn color(mut self, color: impl Into<[f32; 4]>) -> Self {
        self.render.color = Some(color.into());
        self
    }

    /// 渲染默认：归一化原点。
    #[inline]
    pub fn origin(mut self, origin: Vec2) -> Self {
        self.render.origin = Some(origin);
        self
    }

    /// 渲染默认：像素偏移。
    #[inline]
    pub fn offset(mut self, offset: Vec2) -> Self {
        self.render.offset = Some(offset);
        self
    }

    /// 渲染默认：渲染级变换。
    #[inline]
    pub fn transform(mut self, transform: impl Into<Option<Transform2D>>) -> Self {
        self.render.transform = transform.into();
        self
    }

    /// 便捷绘制：内部 `into_render()` 后逐字形回调 `(measure, line, region, transform)`。
    #[inline]
    pub fn draw_with<F>(self, callback: F)
    where F: FnMut(&MeasureInfo, &LineMeasureInfo, &AtlasRegion, Transform2D) {
        self.into_render().draw_with(callback)
    }

    /// 便捷绘制：内部 `into_render()` 后直接渲染到 `Render2D`（feature = `rjw_2d_render`）。
    #[cfg(feature = "rjw_2d_render")]
    #[inline]
    pub fn draw_sprite2d(self, r2d: &mut Render2D, layer: impl Into<Layer>) {
        self.into_render().draw_sprite2d(r2d, layer)
    }

    /// 便捷绘制：内部 `into_render()` 后渐变渲染（feature = `rjw_2d_render`）。
    #[cfg(feature = "rjw_2d_render")]
    #[inline]
    pub fn draw_2d_gradient(
        self,
        r2d: &mut Render2D,
        layer: impl Into<Layer>,
        mode: GradientMode,
        axis: GradientAxis,
        stops: &[(f32, Color)],
    ) {
        self.into_render().draw_2d_gradient(r2d, layer, mode, axis, stops)
    }

    /// 排版 + 测量：返回内容宽高（不消费链）。
    #[inline]
    pub fn measure(&mut self) -> Vec2 {
        let attrs: Attrs<'_> = match &self.attrs {
            Some(a) => a.clone(),
            None => match self.family.as_deref() {
                Some(f) => Attrs::new().family(Family::Name(f)),
                None => Attrs::new(),
            },
        };
        let lh = effective_line_height(self.size, self.line_height, self.line_space);
        let string = self.string.as_str();
        let size = self.size;
        let align = self.align;
        let text = &mut *self.text;
        let buffer = text.create_buffer(string, attrs, size, lh, align);
        Text::measure_buffer(&buffer)
    }

    /// 排版并交出共享 `Arc<Buffer>`（cosmic-text；缓存命中间接共享，不深拷贝），消费链。
    #[inline]
    pub fn into_buffer(self) -> Arc<Buffer> {
        let TextLayout { text, string, family, attrs, size, line_height, line_space, align, .. } = self;
        let attrs: Attrs<'_> = match attrs {
            Some(a) => a,
            None => match family.as_deref() {
                Some(f) => Attrs::new().family(Family::Name(f)),
                None => Attrs::new(),
            },
        };
        let lh = effective_line_height(size, line_height, line_space);
        text.create_buffer(string.as_str(), attrs, size, lh, align)
    }

    /// 预缓存：排版 + 光栅化（字形入图集），**不收集数据**。返回自身，可稍后 `into_render` / `into_render_with`。
    #[inline]
    pub fn precache(self) -> Self {
        let attrs: Attrs<'_> = match &self.attrs {
            Some(a) => a.clone(),
            None => match self.family.as_deref() {
                Some(f) => Attrs::new().family(Family::Name(f)),
                None => Attrs::new(),
            },
        };
        let lh = effective_line_height(self.size, self.line_height, self.line_space);
        let _ = shape_and_rasterize(&mut *self.text, self.string.as_str(), attrs, self.size, lh, self.align);
        self
    }

    /// 转为阶段二 [`TextRender`]，消费链。**用 `Text` 内部默认缓冲**（单标签快速路径，跨帧复用容量）。
    #[inline]
    pub fn into_render(self) -> TextRender<'a> {
        let TextLayout { text, string, family, attrs, size, line_height, line_space, align, render } = self;
        let attrs: Attrs<'_> = match attrs {
            Some(a) => a,
            None => match family.as_deref() {
                Some(f) => Attrs::new().family(Family::Name(f)),
                None => Attrs::new(),
            },
        };
        let lh = effective_line_height(size, line_height, line_space);
        let buffer = shape_and_rasterize(&mut *text, string.as_str(), attrs, size, lh, align);
        let visual_origin = text.buffer_origin(&buffer);
        let page_size = text.glyph_cache.page_size() as f32;
        let (content_size, measure) = collect_glyphs(
            &text.locations, &buffer, visual_origin,
            &mut text.buf.glyphs, &mut text.buf.lines,
        );
        let glyphs = &mut text.buf.glyphs;
        let lines = &mut text.buf.lines;
        let mut tr = TextRender {
            glyphs, lines, content_size, measure,
            origin: Vec2::ZERO, offset: Vec2::ZERO, color: [1.0; 4], transform: None, page_size,
        };
        tr.color = render.color.unwrap_or([1.0; 4]);
        tr.origin = render.origin.unwrap_or(Vec2::ZERO);
        tr.offset = render.offset.unwrap_or(Vec2::ZERO);
        tr.transform = render.transform;
        tr
    }

    /// 转为阶段二 [`TextRender`]，消费链。**用用户提供的缓冲**（多标签并存互不冲突；跨帧复用容量）。
    #[inline]
    pub fn into_render_with<'b>(self, buf: &'b mut TextBuffer) -> TextRender<'b> {
        let TextLayout { text, string, family, attrs, size, line_height, line_space, align, render } = self;
        let attrs: Attrs<'_> = match attrs {
            Some(a) => a,
            None => match family.as_deref() {
                Some(f) => Attrs::new().family(Family::Name(f)),
                None => Attrs::new(),
            },
        };
        let lh = effective_line_height(size, line_height, line_space);
        let buffer = shape_and_rasterize(&mut *text, string.as_str(), attrs, size, lh, align);
        let visual_origin = text.buffer_origin(&buffer);
        let page_size = text.glyph_cache.page_size() as f32;
        let (content_size, measure) = collect_glyphs(
            &text.locations, &buffer, visual_origin,
            &mut buf.glyphs, &mut buf.lines,
        );
        let glyphs = &mut buf.glyphs;
        let lines = &mut buf.lines;
        let mut tr = TextRender {
            glyphs, lines, content_size, measure,
            origin: Vec2::ZERO, offset: Vec2::ZERO, color: [1.0; 4], transform: None, page_size,
        };
        tr.color = render.color.unwrap_or([1.0; 4]);
        tr.origin = render.origin.unwrap_or(Vec2::ZERO);
        tr.offset = render.offset.unwrap_or(Vec2::ZERO);
        tr.transform = render.transform;
        tr
    }
}

// ─── 排版 + 收集（阶段一 → 阶段二） ─────────────────────────────

/// 排版 + 光栅化（字形入图集），返回共享 `Arc<Buffer>` 供收集。
fn shape_and_rasterize(
    text: &mut Text,
    string: &str,
    attrs: Attrs<'_>,
    size: f32,
    line_height: f32,
    align: Align,
) -> Arc<Buffer> {
    let buffer = text.create_buffer(string, attrs, size, line_height, align);
    // 确保所有字形已渲染入图集（buffer_origin 依赖 bearing 数据）；无图像字形只判定一次。
    for run in buffer.layout_runs() {
        for glyph in run.glyphs.iter() {
            let cache_key = glyph.physical((0.0, 0.0), 1.0).cache_key;
            if !text.locations.contains_key(&cache_key) && !text.no_image.contains(&cache_key) {
                text.rasterize_and_pack(cache_key);
            }
        }
    }
    // 光栅化过程中图集可能触发去碎片重排（搬动字形），同步各字形区域。
    text.sync_atlas_regions();
    buffer
}

/// 把排版结果收集进 `glyphs` / `lines`（先 clear，复用容量），返回内容宽高与测量。
fn collect_glyphs(
    locations: &HashMap<cosmic_text::CacheKey, GlyphLocation>,
    buffer: &Buffer,
    visual_origin: Vec2,
    glyphs: &mut Vec<GlyphData>,
    lines: &mut Vec<LineMeasureInfo>,
) -> (Vec2, MeasureInfo) {
    glyphs.clear();
    lines.clear();

    for run in buffer.layout_runs() {
        let line_idx = lines.len();
        let glyph_start = glyphs.len();
        for glyph in run.glyphs.iter() {
            let physical = glyph.physical((0.0, 0.0), 1.0);
            if let Some(loc) = locations.get(&physical.cache_key) {
                let glyph_pos = Vec2::new(
                    physical.x as f32 + loc.left as f32,
                    run.line_y - loc.top as f32,
                );
                glyphs.push(GlyphData {
                    line: line_idx,
                    top_left: (glyph_pos - visual_origin).ceil(),
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
                run.line_top - visual_origin.y,
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
}// ─── TextRender（阶段二：渲染配置，借用缓冲） ───────────────────

/// 阶段二：渲染配置责任链。**借用** `Text` 内部默认缓冲或用户 `TextBuffer`，
/// 通过 `TextLayout::into_render` / `into_render_with` 构造，无法反向转换。
#[derive(Debug)]
pub struct TextRender<'a> {
    glyphs: &'a mut Vec<GlyphData>,
    lines: &'a mut Vec<LineMeasureInfo>,
    content_size: Vec2,
    measure: MeasureInfo,
    origin: Vec2,
    offset: Vec2,
    color: [f32; 4],
    transform: Option<Transform2D>,
    page_size: f32,
}

impl TextRender<'_> {
    /// 归一化原点（相对内容宽高，[0,1]；`(0,0)` 左上角，`(0.5,0.5)` 居中）。
    #[inline]
    pub fn origin(&mut self, norm: Vec2) -> &mut Self {
        self.origin = norm;
        self
    }

    /// 像素原点（相对内容左上角的偏移量）。
    #[inline]
    pub fn origin_px(&mut self, px: Vec2) -> &mut Self {
        self.origin = Vec2::new(
            if self.content_size.x > 0.0 { px.x / self.content_size.x } else { 0.0 },
            if self.content_size.y > 0.0 { px.y / self.content_size.y } else { 0.0 },
        );
        self
    }

    /// 额外像素偏移（叠加在 origin 之后）。
    #[inline]
    pub fn offset(&mut self, px: Vec2) -> &mut Self {
        self.offset = px;
        self
    }

    /// 全局颜色（RGBA，默认白色）；最终字形颜色 = 全局色 × [`GlyphData::color`]。
    #[inline]
    pub fn color(&mut self, color: impl Into<[f32; 4]>) -> &mut Self {
        self.color = color.into();
        self
    }

    /// 渲染级变换（`None` = 单位；作用于整个文本块，旋转/缩放以文本锚点为原点）。
    #[inline]
    pub fn transform(&mut self, transform: impl Into<Option<Transform2D>>) -> &mut Self {
        self.transform = transform.into();
        self
    }

    /// 遍历并修改每个字形渲染记录（可改 `top_left` / `size` / `color` / `layer` / `transform` / `glyph_type`）。
    #[inline]
    pub fn map<F>(&mut self, mut f: F) -> &mut Self
    where F: FnMut(&mut GlyphData) {
        for g in self.glyphs.as_mut_slice() {
            f(g);
        }
        self
    }

    /// 整体测量信息。
    #[inline]
    pub fn measure(&self) -> MeasureInfo { self.measure }
    /// 内容宽高（行盒）。
    #[inline]
    pub fn content_size(&self) -> Vec2 { self.content_size }
    /// 行信息切片。
    #[inline]
    pub fn lines(&self) -> &[LineMeasureInfo] { self.lines.as_slice() }
    /// 字形记录切片。
    #[inline]
    pub fn glyphs(&self) -> &[GlyphData] { self.glyphs.as_slice() }
    /// 字形图集页尺寸（像素；UV 换算用）。
    #[inline]
    pub fn page_size(&self) -> f32 { self.page_size }

    /// origin / offset 解析后的叠加量（最终位置 = 字形相对坐标 + 此值）。
    #[inline]
    fn render_delta(&self) -> Vec2 {
        Vec2::new(
            -self.content_size.x * self.origin.x,
            -self.content_size.y * self.origin.y,
        ) + self.offset
    }

    /// 遍历每个字形调用闭包 `(measure, line, region, transform)`。
    ///
    /// 回调**不**携带纹理：字形所属图集页由 `region.page_uid` 标识，调用方自行经
    /// `rjw_render::TEXTURES.get(page_uid)`（`rjw_atlas` → `rjw_render`）查找。
    #[inline]
    pub fn draw_with<F>(&self, mut callback: F)
    where F: FnMut(&MeasureInfo, &LineMeasureInfo, &AtlasRegion, Transform2D)
    {
        let delta = self.render_delta();
        let render = self.transform;
        let lines = self.lines.as_slice();
        for g in self.glyphs.as_slice() {
            let line = &lines[g.line];
            let tl = g.top_left + delta;
            let tr = match g.transform {
                Some(t) => t,
                None => Transform2D::IDENTITY,
            }.with_move_by(tl);
            let tr = match render {
                Some(t) => tr.with_transform(&t),
                None => tr,
            };
            callback(&self.measure, line, &g.region, tr);
        }
    }

    /// 直接渲染字形精灵到 `Render2D`（feature = `rjw_2d_render`）。
    ///
    /// 每个字形的最终层级 = 传入的基础层 + [`GlyphData::layer`]；
    /// 每个字形可带独立 [`GlyphData::transform`]（`None` = 单位变换）。
    #[cfg(feature = "rjw_2d_render")]
    #[inline]
    pub fn draw_sprite2d(&self, r2d: &mut Render2D, layer: impl Into<Layer>) {
        let delta = self.render_delta();
        let inv = Vec2::new(1.0 / self.page_size, 1.0 / self.page_size);
        let base: f64 = layer.into().as_f64();
        let render = self.transform;
        for g in self.glyphs.as_slice() {
            let Some(tex) = TEXTURES.get(g.region.page_uid) else { continue };
            let rect = SpriteRect::from_texture_px(
                g.top_left + delta, g.size,
                Vec2::new(g.region.tl_px.0 as f32, g.region.tl_px.1 as f32),
                Vec2::new(g.region.wh_px.0 as f32, g.region.wh_px.1 as f32),
                inv,
            );
            let color = if g.glyph_type == GlyphType::Color {
                // 彩色字形（Emoji）：保留自身 RGBA，不叠加全局 tint
                Color::from(g.color)
            } else {
                Color::from(mul_color(self.color, g.color))
            };
            let layer = Layer::from(base + g.layer);
            let transform = match (render, g.transform) {
                (Some(rt), Some(gt)) => gt.with_transform(&rt),
                (Some(rt), None) => rt,
                (None, Some(gt)) => gt,
                (None, None) => Transform2D::default(),
            };
            r2d.add_sprite2d(rect, color, transform, layer, &tex);
        }
    }

    /// 渐变渲染字形（动态 mesh，逐顶点颜色；feature = `rjw_2d_render`）。
    ///
    /// `axis` 选择渐变方向，`stops`：`(t ∈ [0,1], color)` 至少两项。按 `mode` 决定渐变域：
    /// - [`GradientMode::Glyph`]：字形自身跨度；
    /// - [`GradientMode::Line`]：整行跨度（同一行所有字形共享）；
    /// - [`GradientMode::Frame`]：整个文本块跨度（跨行）。
    ///
    /// 逐字形 `layer` / `transform` 不影响渐变（渐变始终用字形 `top_left` / `size`）。
    #[cfg(feature = "rjw_2d_render")]
    pub fn draw_2d_gradient(
        &self,
        r2d: &mut Render2D,
        layer: impl Into<Layer>,
        mode: GradientMode,
        axis: GradientAxis,
        stops: &[(f32, Color)],
    ) {
        assert!(stops.len() >= 2, "draw_2d_gradient needs at least 2 stops");
        let glyphs = self.glyphs.as_slice();
        if glyphs.is_empty() {
            return;
        }
        let layer: Layer = layer.into();
        let delta = self.render_delta();
        let render = self.transform;
        let f32_stops: Vec<(f32, [f32; 4])> = stops.iter().map(|&(t, c)| (t, c.into())).collect();

        // 渐变域（相对坐标，未含 delta）
        let (mut frame_l, mut frame_r) = (f32::MAX, f32::MIN);
        let (mut frame_t, mut frame_b) = (f32::MAX, f32::MIN);
        let n = self.lines.as_slice().len();
        let mut line_l = vec![f32::MAX; n];
        let mut line_r = vec![f32::MIN; n];
        let mut line_t = vec![f32::MAX; n];
        let mut line_b = vec![f32::MIN; n];
        for g in glyphs {
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
        for (i, g) in glyphs.iter().enumerate() {
            match pages.iter_mut().find(|(uid, _)| *uid == g.region.page_uid) {
                Some((_, idxs)) => idxs.push(i),
                None => pages.push((g.region.page_uid, vec![i])),
            }
        }

        for (uid, idxs) in pages {
            let Some(tex) = TEXTURES.get(uid) else { continue };
            r2d.add_mesh_fn(Color::WHITE, layer, |sink| {
                for &i in &idxs {
                    let g = &glyphs[i];
                    let tl = g.top_left + delta;
                    let br = tl + g.size;
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
                        g.region.tl_px.0 as f32 / self.page_size,
                        g.region.tl_px.1 as f32 / self.page_size,
                    );
                    let uv1 = uv0 + Vec2::new(
                        g.region.wh_px.0 as f32 / self.page_size,
                        g.region.wh_px.1 as f32 / self.page_size,
                    );
                    let col = |c: f32| mul_color(sample_gradient(&f32_stops, frac_t(s0, s1, c)), g.color);
                    let tl_w = match render { Some(t) => t.transform_point(tl), None => tl };
                    let tr_w = match render { Some(t) => t.transform_point(Vec2::new(br.x, tl.y)), None => Vec2::new(br.x, tl.y) };
                    let bl_w = match render { Some(t) => t.transform_point(Vec2::new(tl.x, br.y)), None => Vec2::new(tl.x, br.y) };
                    let br_w = match render { Some(t) => t.transform_point(br), None => br };
                    let i0 = sink.push_vertex_uv_color(tl_w, uv0, col(t_tl));
                    let i1 = sink.push_vertex_uv_color(tr_w, Vec2::new(uv1.x, uv0.y), col(t_tr));
                    let i2 = sink.push_vertex_uv_color(bl_w, Vec2::new(uv0.x, uv1.y), col(t_bl));
                    let i3 = sink.push_vertex_uv_color(br_w, uv1, col(t_br));
                    sink.push_tri(i0, i1, i2);
                    sink.push_tri(i1, i3, i2);
                }
            })
            .set_texture(&tex);
        }
    }
}// ─── 内部工具函数 ───────────────────────────────────────────────

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
fn effective_line_height(size: f32, line_height: Option<f32>, line_space: Option<LineSpace>) -> f32 {
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
