//! 主题样式：`Theme` + 各控件子样式（默认 / dark 两套预设，可 clone 覆盖）。
//!
//! # 序列化
//!
//! `serde` feature 开时全部样式类型可 `Serialize` / `Deserialize`（`Theme::to_toml` /
//! `from_toml` / `apply_toml`，见 [`crate::theme_toml`]）：
//! - 每个样式结构体都带 `#[serde(default)]` ⇒ **缺字段回落 `Default`**（手写小文件只写要改的节）；
//! - `PanelStyle::bg_image` **不序列化**（纹理 uid 不可移植）；`Theme::font_weight` 用
//!   `u16` 代理（`Weight` 在 `rjw_text`，不能在此 derive）。

use std::sync::Arc;

use glam::Vec2;
use rjw_color::Color;
use rjw_text::{Align, Weight};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::draw::CornerRadius;

/// `Theme::font_weight` 的 serde 代理。
///
/// `Weight` 定义在 `rjw_text`（本 crate 不能给它 derive）⇒ 用**数值**存（`400` 这种，
/// 人读 TOML 也一眼认得出）；未知数值不报错（`Weight` 是任意 `u16`）。
#[cfg(feature = "serde")]
mod weight_serde {
    use rjw_text::Weight;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(w: &Weight, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u16(w.0)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Weight, D::Error> {
        Ok(Weight(u16::deserialize(d)?))
    }
}

/// `LabelStyle::align` 的 serde 代理（`Align` 同样在 `rjw_text`）。
///
/// 存成小写名字（`"left"` / `"center"` / `"right"` / `"justified"`）；认不出的名字回落
/// `Center`（"主题文件里多写了个我不认识的对齐"不该让整份主题加载失败）。
#[cfg(feature = "serde")]
mod align_serde {
    use rjw_text::Align;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(a: &Align, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match a {
            Align::Left => "left",
            Align::Right => "right",
            Align::Justified => "justified",
            _ => "center",
        })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Align, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "left" => Align::Left,
            "right" => Align::Right,
            "justified" => Align::Justified,
            _ => Align::Center,
        })
    }
}

/// **[`Brush`] 的 TOML 表示**（手写友好 + 无歧义）：
///
/// ```toml
/// [theme.panel]
/// bg = { kind = "vertical", colors = [{ r = .., g = .., b = .., a = .. }, { .. }] }
/// ```
///
/// ⚠ **为什么不用 `derive` 的默认枚举表示**：serde 对外部标签枚举的默认写法是
/// `bg = { Vertical = [颜色, 颜色] }`，TOML 会把它写成**数组表** `[[…bg.Vertical]]`
/// —— 读回来时 `toml` 的枚举反序列化会报
/// "wanted exactly 1 element, more than 1 element"（用户实测的
/// "主题导入失败：主题字段不合法 … in `button.bg`"）。
/// 显式 `{ kind, colors }` 既躲开这个坑，也让人一眼看懂/手改。
#[cfg(feature = "serde")]
#[derive(Serialize, Deserialize)]
struct BrushRepr {
    /// `"solid"` / `"vertical"` / `"horizontal"`。
    kind: String,
    /// 颜色：`solid` 要 1 个；`vertical`（上→下）/ `horizontal`（左→右）要 2 个。
    colors: Vec<Color>,
}

#[cfg(feature = "serde")]
impl From<Brush> for BrushRepr {
    fn from(b: Brush) -> Self {
        let (kind, colors) = match b {
            Brush::Solid(c) => ("solid", vec![c]),
            Brush::Vertical(t, b) => ("vertical", vec![t, b]),
            Brush::Horizontal(l, r) => ("horizontal", vec![l, r]),
        };
        BrushRepr { kind: kind.to_owned(), colors }
    }
}

#[cfg(feature = "serde")]
impl TryFrom<BrushRepr> for Brush {
    type Error = String;

    /// 校验"种类名 + 颜色个数"（错误消息里列出可选值，用户能自己改对）。
    fn try_from(r: BrushRepr) -> Result<Self, Self::Error> {
        match (r.kind.as_str(), r.colors.as_slice()) {
            ("solid", [c]) => Ok(Brush::Solid(*c)),
            ("vertical", [t, b]) => Ok(Brush::Vertical(*t, *b)),
            ("horizontal", [l, r]) => Ok(Brush::Horizontal(*l, *r)),
            ("solid", _) => Err(format!("solid 需要 1 个颜色，收到 {} 个", r.colors.len())),
            ("vertical" | "horizontal", _) => Err(format!(
                "{} 需要 2 个颜色（起 / 止），收到 {} 个",
                r.kind,
                r.colors.len()
            )),
            (other, _) => Err(format!(
                "不认识的刷子种类 {other:?}（可选：solid / vertical / horizontal）"
            )),
        }
    }
}

/// "胶囊"哨兵半径：交给 [`CornerRadius::fit`] 夹成 `min(w, h) / 2`（半圆端 / 正圆）。
///
/// 用一个大而有限的值而不是 `INFINITY`——`fit` 里有 `len / sum`，`INFINITY` 会算出 NaN。
pub const PILL_RADIUS: f32 = 1.0e3;

/// **背景刷**：纯色 / 两端色渐变。
///
/// # 为什么只有"两端色"
///
/// 渐变由光栅化器对**顶点色**做双线性插值产生——`DrawKind::Rect` 走四边形四角色，
/// 圆角走 CPU 镶嵌的逐顶点色（见 `crate::tess`）。**不需要任何渐变纹理**，
/// 因此背景刷不引入新的纹理 / 图集压力。
///
/// 只有两端色是刻意的：多段色标（stops）的能力在 [`crate::Gradient`]（显式绘制原语，
/// 支持 `rotated`）与 `rjw_text::Gradient`（文字，支持多段 + 逐字形 / 逐行 / 整块）。
/// 单控件背景刷要的是"一个便宜的默认值"，多了反而让主题维护成本翻倍。
///
/// # 与圆角共存
///
/// `Brush` 给出**四角颜色**；圆角镶嵌直接吃四角色 ⇒ 「圆角 + 渐变」自然成立。
/// 需要四角各异（对角渐变）时用 [`crate::Gradient::corners`] 走绘制原语，
/// 不在主题里表达。
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "BrushRepr", try_from = "BrushRepr"))]
pub enum Brush {
    /// 纯色（直角时只产生 4 个顶点，最省）。
    Solid(Color),
    /// 垂直两端色（**上 → 下**）。
    Vertical(Color, Color),
    /// 水平两端色（**左 → 右**）。
    Horizontal(Color, Color),
}

impl Brush {
    /// 四角颜色 `[TL, TR, BL, BR]`——所有绘制路径的统一输入。
    #[inline]
    pub fn corners(self) -> [Color; 4] {
        match self {
            Brush::Solid(c) => [c; 4],
            Brush::Vertical(t, b) => [t, t, b, b],
            Brush::Horizontal(l, r) => [l, r, l, r],
        }
    }

    /// 与纯色等价时返回该颜色（否则 `None`）——用于选更省的绘制路径 / 诊断。
    #[inline]
    pub fn as_solid(self) -> Option<Color> {
        match self {
            Brush::Solid(c) => Some(c),
            Brush::Vertical(t, b) if t == b => Some(t),
            Brush::Horizontal(l, r) if l == r => Some(l),
            _ => None,
        }
    }

    /// 换一个"整体色调"：纯色直接替换；渐变把两端色都替换为该色（= 退化成纯色）。
    /// 供主题级 `with_*` 便捷方法使用。
    #[inline]
    pub fn map_colors(self, f: impl Fn(Color) -> Color) -> Self {
        match self {
            Brush::Solid(c) => Brush::Solid(f(c)),
            Brush::Vertical(t, b) => Brush::Vertical(f(t), f(b)),
            Brush::Horizontal(l, r) => Brush::Horizontal(f(l), f(r)),
        }
    }
}

impl From<Color> for Brush {
    #[inline]
    fn from(c: Color) -> Self {
        Brush::Solid(c)
    }
}

/// 与纯色比较（渐变在与两端同色时也等价于纯色）——让 `theme.panel.bg == Color::RED`
/// 这类断言与用户代码直接可写。
impl PartialEq<Color> for Brush {
    #[inline]
    fn eq(&self, other: &Color) -> bool {
        self.as_solid() == Some(*other)
    }
}

impl Default for Brush {
    fn default() -> Self {
        Brush::Solid(Color::WHITE)
    }
}

/// 垂直两端色渐变刷（上 → 下）的便捷构造糖。
#[inline]
pub fn vgrad(top: Color, bottom: Color) -> Brush {
    Brush::Vertical(top, bottom)
}

/// 水平两端色渐变刷（左 → 右）的便捷构造糖。
#[inline]
pub fn hgrad(left: Color, right: Color) -> Brush {
    Brush::Horizontal(left, right)
}

/// 全局 UI 主题：所有控件样式 + 通用间距。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Theme {
    pub label: LabelStyle,
    pub panel: PanelStyle,
    pub button: ButtonStyle,
    pub slider: SliderStyle,
    pub input: InputStyle,
    pub checkbox: CheckboxStyle,
    /// **分割线样式**（[`Ui::divider_at`](crate::ui::Ui::divider_at) / 容器 `divider()`）。
    pub divider: DividerStyle,
    /// **菜单栏样式**（[`Ui::menu_bar`](crate::ui::Ui::menu_bar) 的"一行 + 全宽背景"：
    /// 栏底 / 触发器 / 竖分割线三组令牌；见 [`MenubarStyle`]）。
    pub menubar: MenubarStyle,
    /// 调试样式（debug_layout 描边等；DebugDraw 图元的样式 = 每次调用显式传参）。
    pub debug: DebugStyle,
    /// **焦点样式**（键盘导航）：当前焦点控件的描边（`finish` 绘制）。
    pub focus: FocusStyle,
    /// **模态对话框样式**（[`Ui::modal`](crate::ui::Ui::modal) 遮罩）。
    pub modal: ModalStyle,
    /// **下拉框（combo）样式**：触发按钮用 [`ButtonStyle`]；选项浮层 = 现代右键菜单外观。
    pub combo: ComboStyle,
    /// **可收缩区块样式**（[`Ui::foldable`](crate::ui::Ui::foldable) 的**标题行**：
    /// 常态透明底 + 悬停/按下高亮 + 三角图标 + 文本）。
    pub foldable: FoldableStyle,
    /// **单行控件统一高度**（逻辑像素）：水平行容器（`p.row(...)`）内所有子项强制
    /// 等高——Label/Button/输入框各自内容垂直居中 → 文字中心线对齐（近似基线）。
    pub row_h: f32,
    /// pack / grid 默认子项间距（像素）。
    pub gap: f32,
    /// **边缘羽化宽度**（**逻辑像素**；0 = 关闭，得到硬边）。
    ///
    /// 圆角矩形与圆角边框的抗锯齿靠"顶点 alpha 由 1 插值到 0"实现，梯度以几何边缘
    /// 为中心（硬体内缩 `f/2`、外环外扩 `f/2`）⇒ **视觉尺寸不变**。
    /// `1.0`（默认）≈ 标准 1px 抗锯齿；调大 = 更软的边（背景带一点朦胧感），
    /// 调小 / 归零 = 完全硬边。由 [`Theme::scaled`] 按 DPI 预乘为物理像素。
    pub feather: f32,
    /// **多行行距倍率**（行高 = 字号 × 该值；默认 [`DEFAULT_LINE_SPACING`] = 1.2）。
    ///
    /// 作用于**可能换行的文本**（TextArea / 自动换行标签 / 换行预览）；单行文本的盒子
    /// 高度仍是字号 ⇒ 不受影响。**不是** DPI 量（[`Theme::scaled`] 不缩放它），因为它
    /// 本来就是"相对字号"的倍率。见 [`Theme::with_line_spacing`]。
    pub line_spacing: f32,
    /// **全局字重**（[`Weight`]；默认 `Weight::NORMAL` = 400）。
    ///
    /// 与 `line_spacing` 同类：**主题级文本令牌**，作用于 `Ui` 里**所有**排版
    /// （标签 / 按钮 / 输入框 / 下拉 / 换行文本……）——因为它们都经
    /// `Ui::cache_buffer_wrap` / `Ui::ensure_text_buf` 这两个出口建缓冲。
    /// 不是 DPI 量（[`Theme::scaled`] 不缩放它）。字体没有该字重时由 cosmic-text
    /// 按最接近的字面回落（`fontdb` 匹配），不会变成豆腐块。见 [`Theme::with_font_weight`]。
    #[cfg_attr(feature = "serde", serde(with = "weight_serde"))]
    pub font_weight: Weight,
    /// **本主题的调色板**（换肤 / 回退 / 诊断用；由 [`Theme::themed`] 记录）。
    ///
    /// 手工改过子样式字段后它可能与实际颜色不一致——它记录的是"组装来源"，
    /// 不是从现有字段反推的结果。
    pub palette: Palette,
}

/// **下拉框（combo）样式**：触发按钮用 [`ButtonStyle`]；选项浮层 = **现代右键菜单
/// 外观**——扁平列表项（无边框）、hover / 选中整行高亮、✓ 选中标记、浮层面板细边框
/// 小圆角。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ComboStyle {
    /// 浮层面板背景（浅色主题 = 白 / 浅灰；dark = 深灰）。
    pub menu_bg: Color,
    /// 浮层面板边框。
    pub menu_border: Color,
    /// 浮层圆角（小圆角，如 6）。
    pub menu_radius: CornerRadius,
    /// 浮层上下留白（让菜单"飘"起来）。
    pub menu_pad_v: f32,
    /// 菜单项 hover 整行高亮（浅蓝）。
    pub item_hover: Color,
    /// 选中项高亮（略深 / 同 hover）。
    pub item_selected: Color,
    /// 菜单项左右内边距。
    pub item_pad_x: f32,
    /// 菜单项最小宽。
    pub item_min_w: f32,
    /// 菜单项文本色。
    pub fg: Color,
    /// ✓ 选中标记色。
    pub fg_mark: Color,
    pub font_size: f32,
    pub font_family: Option<Arc<str>>,
}

impl Default for ComboStyle {
    fn default() -> Self {
        Self {
            menu_bg: Color::rgba_u8(250, 250, 252, 255),
            menu_border: Color::rgba_u8(180, 185, 195, 255),
            menu_radius: CornerRadius::all(6.0),
            menu_pad_v: 4.0,
            item_hover: Color::rgba_u8(230, 242, 255, 255),
            item_selected: Color::rgba_u8(208, 228, 255, 255),
            item_pad_x: 4.0,
            item_min_w: 140.0,
            fg: Color::rgba_u8(40, 40, 40, 255),
            fg_mark: Color::rgba_u8(30, 108, 198, 255),
            font_size: 14.0,
            font_family: None,
        }
    }
}

// ─── FoldableStyle ────────────────────────────────────────

/// **可收缩区块样式**（[`Ui::foldable`](crate::ui::Ui::foldable) 的**标题行**）。
///
/// 与 [`ButtonStyle`] 分开的理由：标题行的常态**没有底色、没有边框**（只留文字 + 三角
/// 图标，像列表分组标题），而按钮是"抬升的小方块"。混用会让每个区块看起来像一排按钮
/// （与 [`MenubarStyle`] 的分工同一条理由：菜单条也因此独立成组）。
///
/// ```toml
/// [theme.foldable]
/// bg        = { r = 0.0, g = 0.0, b = 0.0, a = 0.0 }   # 常态透明
/// bg_hover  = { r = 0.24, g = 0.25, b = 0.28, a = 1.0 }
/// fg        = { r = 0.90, g = 0.90, b = 0.92, a = 1.0 }
/// pad_x     = 4.0
/// icon_w    = 18.0
/// icon_h    = 10.0
/// font_size = 14.0
/// ```
///
/// ⚠ 颜色写出 0–1 归一化浮点（`to_toml` 导出的就是这个形式）：手写 `{ r = 32, g = 34 }`
/// 这种 0–255 整数会被当成 **>1 的分量**，渲染时被夹到**全白**。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct FoldableStyle {
    /// 标题行**常态**底色（默认**全透明**：区块不抢视觉，只留文字与三角）。
    pub bg: Brush,
    /// 悬停底色（默认 [`Palette::surface_hover`]）。
    pub bg_hover: Brush,
    /// 按下底色（默认 [`Palette::surface_active`]）。
    pub bg_pressed: Brush,
    /// 标题文本色（默认 [`Palette::text`]）。
    pub fg: Color,
    /// **三角图标**色（默认 [`Palette::text_muted`]——图标是次要信息，比文字弱一档）。
    pub mark: Color,
    /// 标题行边框色（默认 [`Palette::border`]；`border_w = 0` 时不画）。
    pub border: Color,
    /// 边框宽（逻辑像素；默认 0 = 无边框）。
    pub border_w: f32,
    /// 标题行圆角（逻辑像素）。
    pub radius: CornerRadius,
    /// 标题字号（逻辑像素）。
    pub font_size: f32,
    /// 标题字体族（`None` = 系统默认；[`Theme::with_font_family`] 会级联到它）。
    pub font_family: Option<Arc<str>>,
    /// 标题行左右内边距（逻辑像素）。
    pub pad_x: f32,
    /// **三角预留宽**（逻辑像素）：从左边距起算，含图标与文字之间的间距。
    pub icon_w: f32,
    /// 三角**边长**（逻辑像素；`icon_at` 内部按 `min(w,h)` 居中等比 ⇒ 永不形变）。
    pub icon_h: f32,
    // ── 正文的"归属提示"（缩进 + 左侧竖引导线）──────────────────────────────
    /// **正文左缩进**（逻辑像素；默认 12）：正文整体右移这么多 ⇒ 一眼看出"这段属于上面那个
    /// 标题行"。缩进同时作用于**绘制、命中与裁剪**（命令整体平移），故"点得到的就是看得见的"。
    pub body_indent: f32,
    /// **正文左缘的竖引导线颜色**（默认 [`Palette::border`]）。
    pub guide: Color,
    /// 竖引导线**宽**（逻辑像素；默认 1.0；**`0` = 不画**）。
    pub guide_w: f32,
    /// 引导线在正文**上方 / 下方**各多画一截（逻辑像素；默认 2.0）——让"线"看起来是从标题行
    /// 拉下来的括号，而不是浮在正文旁边的一段孤线。
    pub guide_tail: f32,
    /// **收缩范围的上下"渐隐"高度**（逻辑像素；**默认 0 = 关闭**）。
    ///
    /// `> 0` 时在正文**上缘 / 下缘**各画一条由 [`Self::fade`] 渐变到**全透明**的矩形
    /// （各一条 `Gradient` 命令，CPU 镶嵌、零纹理、不增加 draw call）——
    /// 就是"区块是一个可收缩范围"的软提示（用户给的形态：整个收缩范围的上下两端，
    /// 由阴影色渐变到完全透明）。折叠态**不画**（正文没录制）。
    pub fade_h: f32,
    /// 渐隐用的颜色（默认 [`Palette::shadow`]；通常带 alpha）。
    pub fade: Color,
}

impl Default for FoldableStyle {
    fn default() -> Self {
        Self {
            bg: Brush::Solid(Color::TRANSPARENT),
            bg_hover: Brush::Solid(Color::rgba_u8(230, 236, 245, 255)),
            bg_pressed: Brush::Solid(Color::rgba_u8(205, 220, 240, 255)),
            fg: Color::rgba_u8(30, 30, 30, 255),
            mark: Color::rgba_u8(120, 120, 120, 255),
            border: Color::TRANSPARENT,
            border_w: 0.0,
            radius: CornerRadius::default(),
            font_size: 14.0,
            font_family: None,
            pad_x: 4.0,
            icon_w: 18.0,
            icon_h: 10.0,
            body_indent: 12.0,
            guide: Color::rgba_u8(120, 120, 120, 255),
            guide_w: 1.0,
            guide_tail: 2.0,
            // 默认**关闭**渐隐（保持"竖引导线"这一种提示；开了才画那两条渐变）。
            fade_h: 0.0,
            fade: Color::TRANSPARENT,
        }
    }
}

impl FoldableStyle {
    /// **按交互态挑背景刷**：按下 > 悬停 > 常态（与 [`ButtonStyle::pick_bg`] 同一口径与
    /// 理由——漏掉悬停态会让"鼠标移上去毫无变化"）。
    #[inline]
    pub fn pick_bg(&self, pressed: bool, hovered: bool) -> Brush {
        if pressed {
            self.bg_pressed
        } else if hovered {
            self.bg_hover
        } else {
            self.bg
        }
    }

    /// 常态底色（接受 [`Color`] 或 [`Brush`]）。
    pub fn with_bg(mut self, c: impl Into<Brush>) -> Self {
        self.bg = c.into();
        self
    }
    /// 悬停底色。
    pub fn with_bg_hover(mut self, c: impl Into<Brush>) -> Self {
        self.bg_hover = c.into();
        self
    }
    /// 按下底色。
    pub fn with_bg_pressed(mut self, c: impl Into<Brush>) -> Self {
        self.bg_pressed = c.into();
        self
    }
    /// 标题文字色。
    pub fn with_fg(mut self, c: Color) -> Self {
        self.fg = c;
        self
    }
    /// 三角图标色。
    pub fn with_mark(mut self, c: Color) -> Self {
        self.mark = c;
        self
    }
    /// 边框色。
    pub fn with_border(mut self, c: Color) -> Self {
        self.border = c;
        self
    }
    /// 边框宽（逻辑像素）。
    pub fn with_border_w(mut self, w: f32) -> Self {
        self.border_w = w;
        self
    }
    /// 圆角半径（**逻辑像素**；0 = 直角）。
    pub fn with_radius(mut self, r: impl Into<CornerRadius>) -> Self {
        self.radius = r.into();
        self
    }
    /// 字号（逻辑像素）。
    pub fn with_font_size(mut self, s: f32) -> Self {
        self.font_size = s;
        self
    }
    /// 字体族（`None` = 系统默认）。
    pub fn with_font_family(mut self, f: impl AsRef<str>) -> Self {
        self.font_family = Some(Arc::from(f.as_ref()));
        self
    }
    /// 标题行左右内边距（逻辑像素）。
    pub fn with_padding(mut self, pad_x: f32) -> Self {
        self.pad_x = pad_x.max(0.0);
        self
    }
    /// 三角预留宽 / 边长（逻辑像素）。
    pub fn with_icon(mut self, w: f32, h: f32) -> Self {
        self.icon_w = w.max(0.0);
        self.icon_h = h.max(0.0);
        self
    }
    /// **正文左缩进**（逻辑像素）与**左侧竖引导线**（颜色 / 线宽 / 上下各延伸多少）。
    ///
    /// `guide_w = 0` 关掉引导线（只留缩进）；`indent = 0` + `guide_w = 0` = 回到"没有归属提示"
    /// 的观感（与 `Foldable` 首次落地时逐像素一致）。
    pub fn with_body_guide(
        mut self,
        indent: f32,
        guide: Color,
        guide_w: f32,
        guide_tail: f32,
    ) -> Self {
        self.body_indent = indent.max(0.0);
        self.guide = guide;
        self.guide_w = guide_w.max(0.0);
        self.guide_tail = guide_tail.max(0.0);
        self
    }

    /// **收缩范围的上下渐隐**（`h <= 0` = 关闭，默认关闭）：在正文上 / 下缘各画一条由
    /// `fade` 渐变到全透明的矩形（用户给的形态："整个收缩范围的上下两端，由阴影色渐变到
    /// 完全透明"）。
    ///
    /// ```no_run
    /// # use rjw_ui::{FoldableStyle, Theme};
    /// use rjw_color::Color;
    /// let t = Theme::dark().with_foldable(
    ///     Theme::dark().foldable.with_body_fade(12.0, Color::rgba_u8(0, 0, 0, 90)),
    /// );
    /// # let _ = t;
    /// ```
    pub fn with_body_fade(mut self, h: f32, fade: Color) -> Self {
        self.fade_h = h.max(0.0);
        self.fade = fade;
        self
    }

    /// **预设：标题行的"按钮块"外观**（用户给的形态："整个标题容器都可以算作类按钮"）。
    ///
    /// 把常态底色从**全透明**改成 `surface_raised`、加一层描边与圆角（悬停 / 按下沿用
    /// `set_palette` 给的两态）⇒ 标题行看起来就是一个可点的块。**不改行为**：整行本来就是
    /// 命中区（点空白处也翻转），行内控件自己认领按下、不受影响。
    ///
    /// ```no_run
    /// # use rjw_ui::{FoldableStyle, Theme};
    /// let t = Theme::dark().with_foldable(FoldableStyle::button_like(&Theme::dark().palette));
    /// # let _ = t;
    /// ```
    pub fn button_like(p: &Palette) -> Self {
        let mut s = Self::themed(p);
        s.bg = Brush::Solid(p.surface_raised);
        s.border = p.border_strong;
        s.border_w = 1.0;
        s.radius = CornerRadius::all(6.0);
        s
    }
}

impl FoldableStyle {
    /// **原地应用调色板**：只改颜色，尺寸 / 字号 / 圆角 / 字体族保留（换肤口径同其它子样式）。
    pub fn set_palette(&mut self, p: &Palette) {
        // **常态透明**：区块标题不是按钮（见结构体文档）。
        self.bg = Brush::Solid(Color::TRANSPARENT);
        self.bg_hover = Brush::Solid(p.surface_hover);
        self.bg_pressed = Brush::Solid(p.surface_active);
        self.fg = p.text;
        self.mark = p.text_muted;
        self.border = p.border;
        // 正文左缘的竖引导线：与面板描边同色的弱线（"分组括号"不该抢视觉）。
        self.guide = p.border;
        // 渐隐色取调色板的投影色（它本身就是"半透明黑"语义；`fade_h = 0` 时不画）。
        self.fade = p.shadow;
    }

    /// 从调色板派生（= `Default` 起步 + `set_palette`）。
    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── 子样式 DPI 预乘：每个子样式都有 `scaled(s)`（尺寸 / 字号字段 × s 取整；
// 颜色 / 字体族不变）。`Theme::scaled` 逐一调用——单一职责、便于各样式独立复用。 ───

impl LabelStyle {
    /// 预乘 DPI scale：字号 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        self.font_size = (self.font_size * s).round();
        self
    }
}

impl PanelStyle {
    /// 预乘 DPI scale：边框宽 / 内边距 / 圆角 / 投影 / **缩放柄** × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.border_w = m(self.border_w);
        self.padding = m(self.padding);
        self.radius = self.radius.scaled_rounded(s);
        self.shadow = self.shadow.scaled(s);
        self.grip = self.grip.scaled(s);
        self
    }
}

impl ButtonStyle {
    /// 预乘 DPI scale：边框宽 / 圆角 / 内边距 / 字号 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.border_w = m(self.border_w);
        self.radius = self.radius.scaled_rounded(s);
        self.padding.x = m(self.padding.x);
        self.padding.y = m(self.padding.y);
        self.font_size = m(self.font_size);
        self
    }
}

impl SliderStyle {
    /// 预乘 DPI scale：轨道高 / 手柄宽 / 控件高 / 最小宽 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.track_h = m(self.track_h);
        self.handle_w = m(self.handle_w);
        self.height = m(self.height);
        self.min_w = m(self.min_w);
        self.radius = self.radius.map(|r| if r >= PILL_RADIUS { r } else { m(r) });
        self
    }
}

impl InputStyle {
    /// 预乘 DPI scale：边框宽 / 内边距 / 圆角 / 高 / 最小宽 / 字号 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.border_w = m(self.border_w);
        self.padding_x = m(self.padding_x);
        self.padding_y = m(self.padding_y);
        self.radius = self.radius.scaled_rounded(s);
        self.height = m(self.height);
        self.min_w = m(self.min_w);
        self.font_size = m(self.font_size);
        self.grip = self.grip.scaled(s);
        self
    }
}

impl CheckboxStyle {
    /// 预乘 DPI scale：方框 / 圆角 / 边框宽 / 字号 / 间距 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.box_size = m(self.box_size);
        self.radius = self.radius.scaled_rounded(s);
        self.border_w = m(self.border_w);
        self.font_size = m(self.font_size);
        self.gap = m(self.gap);
        self
    }
}

impl DividerStyle {
    /// 预乘 DPI scale：线厚 / 留白 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.thickness = m(self.thickness);
        self.margin = m(self.margin);
        self
    }
}

impl MenubarStyle {
    /// 预乘 DPI scale：边框宽 / 圆角 / 内边距 / 间距 / 字号 × s 取整（颜色不变）。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.border_w = m(self.border_w);
        self.radius = self.radius.scaled_rounded(s);
        self.padding = m(self.padding);
        self.gap = m(self.gap);
        self.font_size = m(self.font_size);
        self.trigger_radius = self.trigger_radius.scaled_rounded(s);
        self.trigger_pad_x = m(self.trigger_pad_x);
        self.separator_w = m(self.separator_w);
        self.separator_margin = m(self.separator_margin);
        self
    }
}

impl FoldableStyle {
    /// 预乘 DPI scale：边框宽 / 圆角 / 字号 / 内边距 / 图标尺寸 × s 取整（颜色不变）。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.border_w = m(self.border_w);
        self.radius = self.radius.scaled_rounded(s);
        self.font_size = m(self.font_size);
        self.pad_x = m(self.pad_x);
        self.icon_w = m(self.icon_w);
        self.icon_h = m(self.icon_h);
        // 正文缩进 / 引导线：与标题行几何一起按 DPI 物理化（不取整的只有颜色）。
        self.body_indent = m(self.body_indent);
        self.guide_w = m(self.guide_w);
        self.guide_tail = m(self.guide_tail);
        self.fade_h = m(self.fade_h);
        self
    }
}

impl DebugStyle {
    /// 预乘 DPI scale：布局描边宽度 × s 取整（颜色不变）。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        self.layout_outline_width = (self.layout_outline_width * s).round();
        self
    }
}

impl FocusStyle {
    /// 预乘 DPI scale：焦点描边宽度 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        self.width = (self.width * s).round();
        self
    }
}

impl ModalStyle {
    /// 预乘 DPI scale：遮罩尺寸 × s 取整（`size = None` 全屏不变）。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        if let Some(sz) = &mut self.size {
            sz.x = (sz.x * s).round();
            sz.y = (sz.y * s).round();
        }
        self
    }
}

impl ComboStyle {
    /// 预乘 DPI scale：浮层圆角 / 上下留白 / 项内边距 / 项最小宽 / 字号 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.menu_radius = self.menu_radius.scaled_rounded(s);
        self.menu_pad_v = m(self.menu_pad_v);
        self.item_pad_x = m(self.item_pad_x);
        self.item_min_w = m(self.item_min_w);
        self.font_size = m(self.font_size);
        self
    }
}

// ─── 配色令牌（Palette）：换肤 / 新主题的唯一入口 ───

/// 明度缩放（`k > 1` 变亮、`< 1` 变暗；alpha 不变、分量 clamp 到 [0,1]）。
#[inline]
fn shade(c: Color, k: f32) -> Color {
    let a: [f32; 4] = c.into();
    Color::from([
        (a[0] * k).clamp(0.0, 1.0),
        (a[1] * k).clamp(0.0, 1.0),
        (a[2] * k).clamp(0.0, 1.0),
        a[3],
    ])
}

/// 把纯色变成**上亮下暗**的微渐变（"凸起 / 抬升"感）。`k = 0` 时退化为纯色
/// （[`Brush::as_solid`] 会把两端同色的渐变当纯色走更省的路径）。
#[inline]
pub fn bevel_raised(c: Color, k: f32) -> Brush {
    if k <= 0.0 {
        return Brush::Solid(c);
    }
    Brush::Vertical(shade(c, 1.0 + k), shade(c, 1.0 - k))
}

/// 把纯色变成**上暗下亮**的微渐变（"凹陷 / 可编辑"感），与 [`bevel_raised`] 反向。
#[inline]
pub fn bevel_sunken(c: Color, k: f32) -> Brush {
    if k <= 0.0 {
        return Brush::Solid(c);
    }
    Brush::Vertical(shade(c, 1.0 - k), shade(c, 1.0 + k))
}

/// **配色令牌**——主题的调色板。
///
/// # 为什么按"层次"命名而不是按控件
///
/// 字段名描述的是**表面的高度**（`surface` / `surface_raised` / `surface_overlay` …），
/// 不是"面板色 / 按钮色 / 菜单色"。控件样式引用层次，于是：
///
/// - 一套明暗阶梯服务全部控件，"面板 vs 卡片 vs 菜单"不会各自漂移；
/// - 新增控件直接挑一个层次，不需要发明新颜色；
/// - 换肤 = 换一份 `Palette`，不必逐子样式改 11 处字面量。
///
/// # 明暗阶梯（`dark` 预设，亮度单调递增）
///
/// ```text
/// surface_sunken  输入框 / 滑轨槽（凹陷，比 surface 更暗）
/// surface_dim     画布底 / 遮罩基色
/// surface         面板 / 窗口
/// surface_raised  卡片 / 工具栏
/// surface_overlay 浮层（菜单 / tooltip）
/// surface_hover   悬停
/// surface_active  按下 / 激活
/// ```
///
/// 注意 `surface_sunken` 比 `surface` **更暗**是刻意的（"可编辑"的通用暗示，
/// 与 VS Code / Discord / egui 一致）；`surface_raised` 及以上才是越抬越亮。
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Palette {
    // ── 表面（暗 → 亮）──
    /// 最底：窗口之外的画布 / 模态遮罩基色。
    pub surface_dim: Color,
    /// 凹陷表面：输入框 / 滑轨槽（刻意比 [`Self::surface`] 更暗）。
    pub surface_sunken: Color,
    /// 面板 / 窗口底。
    pub surface: Color,
    /// 抬升表面：卡片 / 工具栏。
    pub surface_raised: Color,
    /// 浮层表面：下拉菜单 / tooltip。
    pub surface_overlay: Color,
    /// 悬停态表面。
    pub surface_hover: Color,
    /// 按下 / 激活态表面。
    pub surface_active: Color,
    // ── 描边 ──
    /// 常规描边。
    pub border: Color,
    /// 强描边（需要更清晰的分隔 / 焦点邻域）。
    pub border_strong: Color,
    // ── 前景 ──
    /// 正文。
    pub text: Color,
    /// 次级文字（占位符 / 说明 / 未上屏候选）。
    pub text_muted: Color,
    /// 极弱文字（禁用 / 轴标注）。
    pub text_dim: Color,
    // ── 强调 ──
    /// 强调色（焦点描边 / 滑轨填充 / 选中标记 / 勾选填充）。
    pub accent: Color,
    /// 强调色悬停。
    pub accent_hover: Color,
    /// 强调色按下 / 已激活。
    pub accent_active: Color,
    /// 文本选择背景。
    pub selection: Color,
    /// 危险 / 错误。
    pub danger: Color,
    // ── 非层次性的零星色 ──
    /// 高亮前景（滑块手柄 / 勾选标记等"浮在深色上"的浅色）。
    pub handle: Color,
    /// 调试描边（`debug_layout`）。
    pub debug_outline: Color,
    /// 模态遮罩。
    pub scrim: Color,
    /// **投影**（窗口 / 面板 / 浮层的软阴影；`PanelStyle::shadow.color`）。
    ///
    /// 语义是"本体下方的暗部"：浅色主题用淡黑（投影落在浅底上很显眼），深色主题要更黑
    /// （落在深底上否则看不出来）。
    pub shadow: Color,
    /// **表面微渐变强度**（0 = 纯平色）。见 [`bevel_raised`] / [`bevel_sunken`]。
    ///
    /// 深色下一点明暗差能把相邻表面"分"开；浅色主题通常取更小值或 0（扁平观感）。
    pub bevel: f32,
}

impl Default for Palette {
    /// 浅色（= [`Self::light`]）：保持既有浅色主题观感，渐变强度取极轻。
    fn default() -> Self {
        Self::light()
    }
}

impl Palette {
    /// **浅色**调色板：白底 + 蓝强调。
    pub fn light() -> Self {
        Self {
            surface_dim: Color::rgba_u8(228, 230, 234, 255),
            surface_sunken: Color::rgba_u8(255, 255, 255, 255),
            surface: Color::rgba_u8(245, 245, 245, 255),
            surface_raised: Color::rgba_u8(255, 255, 255, 255),
            surface_overlay: Color::rgba_u8(250, 250, 252, 255),
            surface_hover: Color::rgba_u8(228, 238, 252, 255),
            surface_active: Color::rgba_u8(206, 224, 246, 255),
            border: Color::rgba_u8(180, 180, 180, 255),
            border_strong: Color::rgba_u8(150, 150, 150, 255),
            text: Color::rgba_u8(30, 30, 30, 255),
            text_muted: Color::rgba_u8(120, 120, 120, 255),
            text_dim: Color::rgba_u8(150, 150, 150, 255),
            accent: Color::rgba_u8(80, 140, 220, 255),
            accent_hover: Color::rgba_u8(58, 122, 208, 255),
            accent_active: Color::rgba_u8(40, 100, 184, 255),
            selection: Color::rgba_u8(190, 214, 245, 255),
            danger: Color::rgba_u8(214, 69, 69, 255),
            handle: Color::rgba_u8(240, 240, 240, 255),
            debug_outline: Color::CYAN,
            scrim: Color::rgba_u8(0, 0, 0, 140),
            shadow: Color::rgba_u8(0, 0, 0, 48),
            bevel: 0.02,
        }
    }

    /// **深色**调色板：低饱和冷灰阶梯 + 明亮蓝强调。
    ///
    /// 比历史上的深色预设**整体更暗**（面板 ~9% 亮度而非 ~16%），并且
    /// **层次单调递增**：此前 `input`(28,32,40) 比 `panel`(38,42,52) 更暗、
    /// 又和 `menu_bg`(40,45,55) 几乎同色，导致"输入框 / 菜单 / 面板"三者在
    /// 深色下难以分辨；现在它们分别落在 `surface_sunken` / `surface_overlay` /
    /// `surface`，间距明确。
    pub fn dark() -> Self {
        Self {
            surface_dim:        Color::from_hex_rgb ("#0E0F13"), // Color::rgba_u8(14, 15, 19, 255),
            surface_sunken:     Color::from_hex_rgb ("#0f1014"), // Color::rgba_u8(16, 18, 24, 255),
            surface:            Color::from_hex_rgb ("#080a0c"), // Color::rgba_u8(23, 25, 31, 255),
            surface_raised:     Color::from_hex_rgb ("#14171e"), // Color::rgba_u8(34, 37, 45, 255),
            surface_overlay:    Color::from_hex_rgb ("#1c1f29"), // Color::rgba_u8(43, 47, 57, 255),
            surface_hover:      Color::from_hex_rgb ("#242933"), // Color::rgba_u8(51, 56, 69, 255),
            surface_active:     Color::from_hex_rgb ("#2d3445"), // Color::rgba_u8(61, 68, 83, 255),
            border:             Color::from_hex_rgb ("#2f3134"), // Color::rgba_u8(58, 63, 75, 255),
            border_strong:      Color::from_hex_rgb ("#383b41"), // Color::rgba_u8(74, 81, 98, 255),
            text:               Color::from_hex_rgb ("#E8EAF0"), // Color::rgba_u8(232, 234, 240, 255),
            text_muted:         Color::from_hex_rgb ("#9AA3B2"), // Color::rgba_u8(154, 163, 178, 255),
            text_dim:           Color::from_hex_rgb ("#6B7280"), // Color::rgba_u8(107, 114, 128, 255),
            accent:             Color::from_hex_rgb ("#6EA8FF"), // Color::rgba_u8(110, 168, 255, 255),
            accent_hover:       Color::from_hex_rgb ("#8CBCFF"), // Color::rgba_u8(140, 188, 255, 255),
            accent_active:      Color::from_hex_rgb ("#558CDB"), // Color::rgba_u8(85, 140, 219, 255),
            selection:          Color::from_hex_rgb ("#2B4A78"), // Color::rgba_u8(43, 74, 120, 255),
            danger:             Color::from_hex_rgb ("#FF6B6B"), // Color::rgba_u8(255, 107, 107, 255),
            handle:             Color::from_hex_rgb ("#C8D0DC"), // Color::rgba_u8(200, 208, 220, 255),
            debug_outline:      Color::from_hex_rgb ("#60C8FF"), // Color::rgba_u8(96, 200, 255, 255),
            scrim:              Color::from_hex_rgba("#000000B4"), // Color::rgba_u8(0, 0, 0, 180),
            shadow:             Color::from_hex_rgba("#00000078"), // Color::rgba_u8(0, 0, 0, 120),
            bevel: 0.10,
        }
    }

    /// **历史深色调色板**：逐字段复刻改造前的硬编码深色配色（`bevel = 0`）。
    /// 供不想被新配色影响的下游一键回退：`Theme::themed(&Palette::legacy_dark())`。
    pub fn legacy_dark() -> Self {
        Self {
            surface_dim: Color::rgba_u8(0, 0, 0, 255),
            surface_sunken: Color::rgba_u8(28, 32, 40, 255),
            surface: Color::rgba_u8(38, 42, 52, 255),
            surface_raised: Color::rgba_u8(52, 58, 70, 255),
            surface_overlay: Color::rgba_u8(40, 45, 55, 255),
            surface_hover: Color::rgba_u8(66, 76, 96, 255),
            surface_active: Color::rgba_u8(90, 110, 150, 255),
            border: Color::rgba_u8(70, 78, 96, 255),
            border_strong: Color::rgba_u8(90, 98, 118, 255),
            text: Color::rgba_u8(230, 230, 230, 255),
            text_muted: Color::rgba_u8(150, 158, 176, 255),
            text_dim: Color::rgba_u8(120, 132, 150, 255),
            accent: Color::rgba_u8(96, 150, 220, 255),
            accent_hover: Color::rgba_u8(110, 160, 230, 255),
            accent_active: Color::rgba_u8(70, 120, 190, 255),
            selection: Color::rgba_u8(70, 120, 190, 255),
            danger: Color::rgba_u8(220, 80, 80, 255),
            handle: Color::rgba_u8(200, 210, 225, 255),
            debug_outline: Color::rgba_u8(96, 200, 255, 255),
            scrim: Color::rgba_u8(0, 0, 0, 180),
            shadow: Color::rgba_u8(0, 0, 0, 120),
            bevel: 0.0,
        }
    }
}


/// 模态对话框样式（[`Ui::modal`](crate::ui::Ui::modal) 的全屏遮罩）。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ModalStyle {
    /// 遮罩颜色（默认半透明黑，遮住背后内容）。
    pub dim: Color,
    /// 遮罩尺寸（**逻辑像素**；`None` = 全屏）。
    pub size: Option<glam::Vec2>,
}

impl Default for ModalStyle {
    fn default() -> Self {
        Self {
            dim: Color::rgba_u8(0, 0, 0, 140),
            size: None,
        }
    }
}

/// 纯文本标签样式。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct LabelStyle {
    /// 字体族（`None` = 系统默认；空串同默认）。
    pub font_family: Option<Arc<str>>,
    pub font_size: f32,
    pub color: Color,
    /// 水平对齐（垂直恒居中）。
    #[cfg_attr(feature = "serde", serde(with = "align_serde"))]
    pub align: Align,
}

impl Default for LabelStyle {
    fn default() -> Self {
        Self {
            font_family: None,
            font_size: 14.0,
            color: Color::rgba_u8(40, 40, 40, 255),
            align: Align::Left,
        }
    }
}

/// **投影样式**（窗口 / 面板的**顶点色软阴影**：无纹理、无着色器、不增 draw call）。
///
/// 实现见 `crate::tess::push_rounded_shadow`：从面板矩形向外 `blur` 像素铺若干同心圆角带，
/// alpha 按二次曲线渐隐到 0。因为完全是顶点色，它**进窗口顶点缓存**——窗口内容不变时
/// 零额外开销，也不增加 draw call（与背景同纹理同变换，合批成一段）。
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ShadowStyle {
    /// 向外渐隐宽度（**逻辑像素**；**0 = 不画投影**）。
    pub blur: f32,
    /// 最外圈相对本体的偏移（逻辑像素；模拟光从上方来，投影**整体**偏下）。
    ///
    /// ⚠ 不是"把内轮廓整体下移"：偏移按圈数线性分摊（第 `t` 圈偏 `offset·t`），
    /// 于是本体边缘处浓度最高且**立即开始衰减**——不会在本体下缘留下一条等浓度暗带
    /// （那会看起来像"阴影下方突出"）。
    pub offset: Vec2,
    /// 投影颜色（含 alpha；一般半透明黑，见 [`Palette::shadow`]）。
    pub color: Color,
}

impl Default for ShadowStyle {
    fn default() -> Self {
        Self {
            blur: 14.0,
            offset: Vec2::new(0.0, 3.0),
            color: Color::rgba_u8(0, 0, 0, 110),
        }
    }
}

impl ShadowStyle {
    /// 预乘 DPI scale（模糊宽 / 偏移 × s 取整；颜色不变）。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.blur = m(self.blur);
        self.offset = Vec2::new(m(self.offset.x), m(self.offset.y));
        self
    }

    /// 是否会产生几何（`blur <= 0` 或全透明色 ⇒ 不画）。
    #[inline]
    pub fn is_visible(&self) -> bool {
        self.blur > 0.0 && <[f32; 4]>::from(self.color)[3] > 0.0
    }
}

/// **默认多行行距倍率**（行高 = 字号 × 该值）。
///
/// 多行编辑框 / 换行文本的默认行高倍率，也是 [`Theme::line_spacing`] 的初值；
/// 可经 [`Theme::with_line_spacing`] 或 [`Theme::density`] 调整。
pub const DEFAULT_LINE_SPACING: f32 = 1.2;

/// **UI 密度预设**（[`Theme::density`]）：一趟把间距令牌与字号按比例缩放，
/// 让同一套界面在小屏上紧凑、在大屏 / 触屏上疏朗。
///
/// 用枚举而非裸 bool（R2）：三档语义明确，且以后加档位不改签名。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Density {
    /// **紧凑**：间距 ×0.84、字号 ×0.92、行距 ×1.10（工具面板 / 小屏 / 高信息密度表格）。
    Compact,
    /// **标准**（默认）：保持各子样式的默认值不变。
    #[default]
    Cozy,
    /// **宽松**：间距 ×1.18、字号 ×1.08、行距 ×1.30（触屏 / 演示 / 大屏）。
    Spacious,
}

impl Density {
    /// 三档的 `(间距倍率, 字号倍率, 行距倍率)`。
    #[inline]
    pub fn scales(self) -> (f32, f32, f32) {
        match self {
            Density::Compact => (0.84, 0.92, 1.10),
            Density::Cozy => (1.0, 1.0, DEFAULT_LINE_SPACING),
            Density::Spacious => (1.18, 1.08, 1.30),
        }
    }
}

/// **缩放柄的形状**（[`GripStyle::shape`]）。
///
/// 用枚举而不是多个裸 bool（R2）：形状是**互斥**的几档，且以后加档位不改签名。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum GripShape {
    /// **三条递减小方块**（历史观感：沿右下对角线逐级内缩；默认见
    /// [`GripShape::Diagonal`]）。
    Squares,
    /// **三条横线**（内置矢量图标 [`Icon::Grip`]，画在 `size × count` 的方框里，
    /// 与字体无关、缺字形也不会变形）。
    Bars,
    /// **三条斜线**（45°，从左下到右上；内置矢量图标 [`Icon::GripDiagonal`]）——
    /// 经典"缩放角"观感。⚠ 画的方框是 [`GripShape::Bars`] 的 **1.5×**：三条斜线挤在
    /// `size × count` 的小方框里间距太小，会被羽化糊成一片。
    #[default]
    Diagonal,
    /// **不画图案**（命中区照旧 —— 仍可拖动缩放，适合"干净"的界面）。
    Hidden,
}

/// **窗口右下角缩放柄样式**（[`PanelStyle::grip`]）。
///
/// 只对**固定宽窗口**（`WindowBuilder::width(..)`）生效 —— 那是唯一带缩放柄的容器；
/// 会话里的"拖拽按钮"指的就是它。
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct GripStyle {
    /// 形状（默认 [`GripShape::default()`] = [`GripShape::Diagonal`] **三条斜线**）。
    pub shape: GripShape,
    /// 颜色（默认 = 面板边框色，由 [`PanelStyle::themed`] 从 `Palette::border` 灌入）。
    pub color: Color,
    /// 单个图元边长（**逻辑像素**，默认 4.0）。
    pub size: f32,
    /// 图元间距 / 对角递进步长（**逻辑像素**，默认 5.0）。
    pub step: f32,
    /// 图元个数（默认 3；[`GripShape::Bars`] 下表示三条横线所在方框的边长倍数）。
    pub count: u32,
}

impl Default for GripStyle {
    fn default() -> Self {
        Self {
            shape: GripShape::default(),
            // 默认色与 `PanelStyle::default().border` 一致；走主题时由 `themed` 覆盖。
            color: Color::rgba_u8(180, 180, 180, 255),
            size: 4.0,
            step: 5.0,
            count: 3,
        }
    }
}

impl GripStyle {
    /// 预乘 DPI scale（`size` / `step` × s 取整；形状 / 颜色 / 个数不变）。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.size = m(self.size).max(1.0);
        self.step = m(self.step).max(1.0);
        self
    }

    /// 是否真的画图案（[`GripShape::Hidden`] ⇒ 不画，但**命中区照旧**）。
    #[inline]
    pub fn is_visible(&self) -> bool {
        self.shape != GripShape::Hidden && <[f32; 4]>::from(self.color)[3] > 0.0
    }

    /// **图案占用的方形边长**（逻辑像素）：从窗口右下角往左上量，图案全部落在这个方框内。
    ///
    /// 也是命中区的下限来源 —— 柄画得大，抓取范围就该跟着大。
    #[inline]
    pub fn extent(&self) -> f32 {
        match self.shape {
            GripShape::Hidden => 0.0,
            GripShape::Squares => self.step * self.count as f32 + self.size,
            GripShape::Bars => self.size * self.count as f32 + self.step,
            // 斜线版画的方框是横线版的 1.5×（见 [`GripShape::Diagonal`] 的文档）。
            GripShape::Diagonal => self.size * self.count as f32 * 1.5 + self.step,
        }
    }
}

/// 面板（背景 + 边框）样式。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct PanelStyle {
    /// 背景刷（纯色 / 两端色渐变；与 `radius` 组合即「圆角 + 渐变」）。
    pub bg: Brush,
    pub border: Color,
    pub border_w: f32,
    /// 内容区内边距（像素）。
    pub padding: f32,
    /// 圆角半径（**逻辑像素**；0 = 直角；四角可各自不同，见 [`CornerRadius`]）。
    pub radius: CornerRadius,
    /// **背景图**（可选）：画在 `bg` **之上**、内容**之下**，用面板矩形与 `radius`
    /// 作为圆角遮罩（`ImageBg::radius` 被忽略、恒用面板的 `radius`）。
    ///
    /// 典型用途：窗口 / 面板的纹理底（木纹、纸张、渐变图、平铺图案）。`bg` 仍可
    /// 作为底色（图片半透明时透出）。
    ///
    /// ⚠ **不参与序列化**（`serde(skip)`）：纹理 `uid` 跨进程 / 跨资源不可移植，
    /// 加载主题后该字段回落 `None`——要贴图请应用自己在加载后 `.with_bg_image(..)`。
    #[cfg_attr(feature = "serde", serde(skip))]
    pub bg_image: Option<crate::draw::ImageBg>,
    /// **投影**（窗口 / 面板 / 浮层的软阴影；`blur = 0` = 不画）。
    ///
    /// 画在本体**之下**、**更低 z 的窗口之上**（"投影落在下面的窗口上"）。
    pub shadow: ShadowStyle,
    /// **右下角缩放柄样式**（只对固定宽窗口生效；见 [`GripStyle`]）。
    pub grip: GripStyle,
}

impl Default for PanelStyle {
    fn default() -> Self {
        Self {
            bg: Brush::Solid(Color::rgba_u8(245, 245, 245, 255)),
            border: Color::rgba_u8(180, 180, 180, 255),
            border_w: 1.0,
            padding: 8.0,
            radius: CornerRadius::default(),
            bg_image: None,
            shadow: ShadowStyle::default(),
            grip: GripStyle::default(),
        }
    }
}

/// 按钮样式（normal / hover / pressed 三态）。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ButtonStyle {
    /// 常态背景刷。默认给一点纵向微渐变：纯平色在深色主题下偏死板，
    /// 一点明暗差即有体积感，且**零纹理成本**。
    pub bg: Brush,
    /// 悬停背景刷。
    pub bg_hover: Brush,
    /// 按下背景刷。
    pub bg_pressed: Brush,
    pub fg: Color,
    pub border: Color,
    pub border_w: f32,
    /// 圆角半径（**逻辑像素**；0 = 直角；四角可各自不同，见 [`CornerRadius`]）。
    pub radius: CornerRadius,
    /// 内边距（x = 水平，y = 垂直）。
    pub padding: glam::Vec2,
    pub font_size: f32,
    pub font_family: Option<Arc<str>>,
}

impl Default for ButtonStyle {
    fn default() -> Self {
        Self {
            bg: Brush::Solid(Color::rgba_u8(225, 225, 225, 255)),
            bg_hover: Brush::Solid(Color::rgba_u8(205, 225, 250, 255)),
            bg_pressed: Brush::Solid(Color::rgba_u8(170, 200, 235, 255)),
            fg: Color::rgba_u8(30, 30, 30, 255),
            border: Color::rgba_u8(150, 150, 150, 255),
            border_w: 1.0,
            radius: CornerRadius::default(),
            padding: glam::Vec2::new(12.0, 6.0),
            font_size: 14.0,
            font_family: None,
        }
    }
}

/// 滑块样式。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct SliderStyle {
    /// 轨道刷（纯色 / 两端色渐变；接受 [`Color`]）。
    ///
    /// **渐变轨道**让"颜色滑块"（取色器的通道行）成为可能：轨道画的是该通道
    /// 0→最大 的颜色斜坡，而值是靠手柄位置读的——不需要任何纹理 / 着色器。
    pub track: Brush,
    /// 已填充部分刷。取色器的通道行把它设成**透明**（否则纯色填充会盖掉斜坡）。
    pub fill: Brush,
    pub handle: Color,
    pub handle_border: Color,
    /// 轨道高度（像素）。
    pub track_h: f32,
    /// 手柄宽度（像素）。
    pub handle_w: f32,
    /// 控件总高（含点击区）。
    pub height: f32,
    /// 控件最小宽（pack 内自动尺寸用）。
    pub min_w: f32,
    /// 轨道 / 填充 / 手柄的圆角（逻辑像素）。
    ///
    /// 默认值**很大** ⇒ 被 [`CornerRadius::fit`] 夹成**胶囊**：轨道两端半圆、
    /// 手柄正圆（`r = min(w,h)/2`）。设小即变成圆角矩形，设 0 = 直角。
    pub radius: CornerRadius,
}

impl Default for SliderStyle {
    fn default() -> Self {
        Self {
            track: Color::rgba_u8(190, 190, 190, 255).into(),
            fill: Color::rgba_u8(80, 140, 220, 255).into(),
            handle: Color::rgba_u8(240, 240, 240, 255),
            handle_border: Color::rgba_u8(120, 120, 120, 255),
            track_h: 6.0,
            handle_w: 12.0,
            height: 20.0,
            min_w: 120.0,
            radius: CornerRadius::all(PILL_RADIUS),
        }
    }
}

/// 文本输入框样式。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct InputStyle {
    /// 背景刷（默认略微纵向渐变的凹陷感）。
    pub bg: Brush,
    pub border: Color,
    pub border_focus: Color,
    pub fg: Color,
    pub caret: Color,
    /// IME 组合候选串颜色（如拼音未上屏时的灰色候选）。
    pub preedit: Color,
    /// **文本选择高亮**（背景色；选中文本拖拽区域）。
    pub sel_bg: Color,
    /// **缩放柄 / 拖动框颜色**（可调整大小/宽度的文本输入框右下角缩放手柄标记）。
    pub resize_handle: Color,
    /// **缩放柄的形状 / 尺寸 / 颜色**（可调整大小/宽度的文本输入框；默认
    /// [`GripShape::Diagonal`] —— **三条斜线**，经典"右下角可拖"的观感，见下）。
    ///
    /// 形状取自**主题**（不再写死在引擎里）：`Diagonal`（默认，三条 45° 斜线）、
    /// `Bars`（三条横线）、`Squares`（历史观感：沿对角线递减的小方块）、
    /// `Hidden`（不画图案，**命中区照旧** —— 仍能拖）。
    /// 颜色默认 = [`Self::resize_handle`]（`themed` 里跟随 `Palette::text_dim`）。
    pub grip: GripStyle,
    pub border_w: f32,
    /// 内容水平内边距。
    pub padding_x: f32,
    /// **内容垂直内边距**（逻辑像素；默认 3）：文本 / 光标相对文本框上缘的**垫高**。
    ///
    /// 为什么需要它：多行编辑是**顶对齐**（`.valign` 可改中心对齐），而没有垫高时文字会
    /// 贴着上边框、与单行输入框的垂直位置**不一致**（用户实测："多行被拉高时位置也不会
    /// 发生变化 ⇒ 必须默认 TopLeft，但要把文字和光标往下拉几个像素"）。
    /// 单行输入框（`TextVAlign::Center`）与多行顶对齐都按它垫高 ⇒ 两者视觉一致。
    pub padding_y: f32,
    /// 圆角半径（**逻辑像素**；0 = 直角；四角可各自不同，见 [`CornerRadius`]）。
    pub radius: CornerRadius,
    /// 控件总高。
    pub height: f32,
    /// 控件最小宽。
    pub min_w: f32,
    pub font_size: f32,
    pub font_family: Option<Arc<str>>,
}

impl Default for InputStyle {
    fn default() -> Self {
        Self {
            bg: Brush::Solid(Color::rgba_u8(255, 255, 255, 255)),
            border: Color::rgba_u8(160, 160, 160, 255),
            border_focus: Color::rgba_u8(80, 140, 220, 255),
            fg: Color::rgba_u8(30, 30, 30, 255),
            caret: Color::rgba_u8(30, 30, 30, 255),
            preedit: Color::rgba_u8(120, 120, 120, 255),
            sel_bg: Color::rgba_u8(140, 190, 245, 255),
            resize_handle: Color::rgba_u8(120, 130, 150, 255),
            // 缩放柄默认 = `GripShape::default()`（**三条斜线 `Diagonal`**，全局默认；
            // 这里只覆盖颜色为 `resize_handle` 同色）。换形状改主题：
            // `Bars` 三条横线 / `Squares` 历史观感的小方块 / `Hidden` 不画图案（仍能拖）。
            grip: GripStyle {
                color: Color::rgba_u8(120, 130, 150, 255),
                ..GripStyle::default()
            },
            border_w: 1.0,
            padding_x: 6.0,
            padding_y: 3.0,
            radius: CornerRadius::default(),
            height: 26.0,
            min_w: 140.0,
            font_size: 14.0,
            font_family: None,
        }
    }
}

/// 分割线样式（[`Ui::divider_at`](crate::ui::Ui::divider_at) / [`crate::widgets::Divider`]）。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct DividerStyle {
    /// 线颜色。
    pub color: Color,
    /// 线厚度（逻辑像素）。
    pub thickness: f32,
    /// 上下留白（逻辑像素；占光标行高 = thickness + 2 × margin）。
    pub margin: f32,
}

impl Default for DividerStyle {
    fn default() -> Self {
        Self {
            color: Color::rgba_u8(150, 150, 150, 255),
            thickness: 1.0,
            margin: 4.0,
        }
    }
}

/// **菜单栏样式**（[`Ui::menu_bar`](crate::ui::Ui::menu_bar) 的"一行 + 全宽背景"用它）。
///
/// 为什么**单独一组**而不是复用 [`ButtonStyle`]：菜单栏的观感与按钮**相反**——触发器常态
/// **没有底色、没有边框**（纯文字 + 圆角悬停高亮，像 Windows / VS Code 的菜单条），而
/// `ButtonStyle` 是"抬升的小方块"。混用会让菜单栏看起来像**一排按钮**（用户实测反馈：
/// "图一菜单栏的视觉效果并不美观"）。所以这里把"栏底 / 触发器 / 竖分割线"三组令牌分开。
///
/// ```toml
/// [theme.menubar]
/// bg = { r = 0.13, g = 0.13, b = 0.15, a = 1.0 }
/// padding = 4.0
/// trigger_pad_x = 10.0
/// trigger_hover = { r = 0.24, g = 0.25, b = 0.28, a = 1.0 }
/// separator_margin = 6.0
/// ```
///
/// ⚠ **颜色写出 0–1 归一化浮点**（`to_toml` 导出的就是这个形式）。手写 `{ r = 32, g = 34 }`
/// 这种 0–255 整数会被当成 **>1 的分量**，渲染时被夹到**全白** —— 手写主题的经典坑。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct MenubarStyle {
    /// 栏背景（默认 `Palette::surface_raised`）。
    pub bg: Color,
    /// **底边分隔线**颜色（默认 `Palette::border`）。
    pub border: Color,
    /// 底边分隔线宽（逻辑像素；`0` = 不画）。⚠ 只画**底边**（不是四边环）：通栏菜单条
    /// 只需要一条"下沿"，画四边会在屏幕边缘出现多余的两条竖线。线画在栏内下沿。
    pub border_w: f32,
    /// 栏圆角（逻辑像素；默认 **0 = 通栏直角**）。不参与 [`Theme::with_radius`] 级联
    /// （通栏条圆角化会露出底下的内容）。
    pub radius: CornerRadius,
    /// 栏**内边距**（四边同值，逻辑像素；默认 4）：横向 = 首 / 末子项与栏边的距离；
    /// 纵向 = 栏比内容高出来的那截（触发器等子项恒为 [`Theme::row_h`] 高）。
    pub padding: f32,
    /// 触发器之间的间距（逻辑像素；默认 2 —— 菜单条要**紧**，不是工具栏的 `Theme::gap`）。
    pub gap: f32,
    /// 触发器字号（逻辑像素）。
    pub font_size: f32,
    /// 触发器字体族（`None` = 系统默认；[`Theme::with_font_family`] 会级联到它）。
    pub font_family: Option<Arc<str>>,
    /// 触发器文字色（默认 `Palette::text`）。
    pub fg: Color,
    /// 触发器**常态底色**（默认**全透明** ⇒ 看起来是纯文字菜单，而不是一排按钮）。
    pub trigger_bg: Color,
    /// 触发器**悬停**底色。
    pub trigger_hover: Color,
    /// 触发器**按下 / 当前展开**底色（展开的菜单要一直亮着，见 [`crate::widgets::MenuBar`]）。
    pub trigger_pressed: Color,
    /// 触发器圆角（逻辑像素；默认 4 —— 悬停高亮是小圆角块）。
    pub trigger_radius: CornerRadius,
    /// 触发器左右内边距（逻辑像素；默认 10）：触发器宽 = 文字实测宽 + 2 × 它。
    pub trigger_pad_x: f32,
    /// 竖分割线颜色（默认 `Palette::border`）。
    pub separator: Color,
    /// 竖分割线线宽（逻辑像素；默认 1）。
    pub separator_w: f32,
    /// 竖分割线两侧留白（逻辑像素；默认 6）：占位宽 = 线宽 + 2 × 它，线长 = 行高 − 2 × 它。
    pub separator_margin: f32,
}

impl Default for MenubarStyle {
    fn default() -> Self {
        Self {
            bg: Color::rgba_u8(244, 244, 246, 255),
            border: Color::rgba_u8(200, 200, 204, 255),
            border_w: 1.0,
            radius: CornerRadius::default(),
            padding: 4.0,
            gap: 2.0,
            font_size: 13.0,
            font_family: None,
            fg: Color::rgba_u8(28, 28, 30, 255),
            // **常态透明**：菜单条不是一排按钮（这一条就是"图二观感"的关键）。
            trigger_bg: Color::TRANSPARENT,
            trigger_hover: Color::rgba_u8(228, 228, 232, 255),
            trigger_pressed: Color::rgba_u8(214, 214, 220, 255),
            trigger_radius: CornerRadius::all(4.0),
            trigger_pad_x: 10.0,
            separator: Color::rgba_u8(200, 200, 204, 255),
            separator_w: 1.0,
            separator_margin: 6.0,
        }
    }
}

/// 勾选框 / 单选样式。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct CheckboxStyle {
    /// 方框边长。
    pub box_size: f32,
    /// 方框圆角半径（**逻辑像素**；0 = 直角；四角可各自不同，见 [`CornerRadius`]）。
    ///
    /// 无边框（`border_w == 0`）时无效——此时填充是直角小方块。由
    /// [`Theme::with_radius`] 级联为全局半径的一半（勾选框比按钮小，同半径会显得过圆）。
    pub radius: CornerRadius,
    pub box_border: Color,
    /// 方框边框宽（逻辑像素；中心填充 = 外框 shrink(border_w + [`CHECKBOX_INNER`](crate::ui) 内边距)）。
    pub border_w: f32,
    pub checked_fill: Color,
    pub fg: Color,
    pub font_size: f32,
    pub font_family: Option<Arc<str>>,
    /// 文本与方框间距。
    pub gap: f32,
}

impl Default for CheckboxStyle {
    fn default() -> Self {
        Self {
            box_size: 16.0,
            radius: CornerRadius::default(),
            box_border: Color::rgba_u8(140, 140, 140, 255),
            border_w: 1.0,
            checked_fill: Color::rgba_u8(80, 140, 220, 255),
            fg: Color::rgba_u8(30, 30, 30, 255),
            font_size: 14.0,
            font_family: None,
            gap: 6.0,
        }
    }
}

/// 调试样式（Debug UI / DebugDraw）。
///
/// - **`layout_outline` / `layout_outline_width`**：`debug_layout`（[`crate::Ui::debug_layout`]）
///   给每个控件/容器矩形画描边时的颜色与宽度（宽度为**物理像素**）；
/// - DebugDraw 屏幕空间图元（[`crate::Ui::debug_line`] 等）的样式 = **每次调用显式传参**
///   （`color` + `width`，逻辑像素）——需要统一样式时，可自建常量/结构体保存后传入。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct DebugStyle {
    /// `debug_layout` 布局描边颜色（默认青色）。
    pub layout_outline: Color,
    /// `debug_layout` 布局描边宽度（**物理像素**，默认 1.0）。
    pub layout_outline_width: f32,
}

impl Default for DebugStyle {
    fn default() -> Self {
        Self {
            layout_outline: Color::CYAN,
            layout_outline_width: 1.0,
        }
    }
}

/// **焦点样式**（键盘导航）：`finish` 给当前焦点控件画的描边（颜色 / 宽度）。
/// 宽度为**逻辑像素**（内部 × scale 后取整）；默认取调色板强调色、宽 2.0
/// ——1.0 在深色高 DPI 下几乎看不出来，键盘导航"看不见焦点"是硬伤。
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct FocusStyle {
    pub color: Color,
    pub width: f32,
}

impl Default for FocusStyle {
    fn default() -> Self {
        Self { color: Color::CYAN, width: Self::DEFAULT_WIDTH }
    }
}

impl FocusStyle {
    /// 默认焦点描边宽（**逻辑像素**）。
    pub const DEFAULT_WIDTH: f32 = 2.0;
}

impl LabelStyle {
    /// 字体族（`None` = 系统默认）。
    pub fn with_font_family(mut self, f: impl AsRef<str>) -> Self {
        self.font_family = Some(f.as_ref().into());
        self
    }
    pub fn with_font_size(mut self, s: f32) -> Self {
        self.font_size = s;
        self
    }
    pub fn with_color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
    /// 水平对齐（垂直恒居中）。
    pub fn with_align(mut self, a: Align) -> Self {
        self.align = a;
        self
    }
}

impl PanelStyle {
    /// 背景刷（接受 [`Color`] 或 [`Brush`]：`Color: Into<Brush>` 等价纯色）。
    pub fn with_bg(mut self, c: impl Into<Brush>) -> Self {
        self.bg = c.into();
        self
    }
    pub fn with_border(mut self, c: Color) -> Self {
        self.border = c;
        self
    }
    pub fn with_border_w(mut self, w: f32) -> Self {
        self.border_w = w;
        self
    }
    /// 内容区内边距（像素）。
    pub fn with_padding(mut self, p: f32) -> Self {
        self.padding = p;
        self
    }
    /// 圆角半径（**逻辑像素**；0 = 直角）。
    pub fn with_radius(mut self, r: impl Into<CornerRadius>) -> Self {
        self.radius = r.into();
        self
    }
    /// **背景图**（画在 `bg` 之上、内容之下；圆角遮罩恒用面板 `radius`）。
    pub fn with_bg_image(mut self, img: crate::draw::ImageBg) -> Self {
        self.bg_image = Some(img);
        self
    }
    /// **投影**（整对象替换：模糊宽 / 偏移 / 颜色一起换）。
    pub fn with_shadow(mut self, shadow: ShadowStyle) -> Self {
        self.shadow = shadow;
        self
    }
    /// **投影颜色**（只改色，模糊宽 / 偏移沿用现值）。
    pub fn with_shadow_color(mut self, c: Color) -> Self {
        self.shadow.color = c;
        self
    }
    /// **关闭投影**（等价 `blur = 0`；语义与 `debug_layout()` / `without_debug_layout()` 同风格）。
    pub fn without_shadow(mut self) -> Self {
        self.shadow.blur = 0.0;
        self
    }
    /// **右下角缩放柄样式**（整对象替换；只对固定宽窗口生效）。
    pub fn with_grip(mut self, grip: GripStyle) -> Self {
        self.grip = grip;
        self
    }
    /// **缩放柄颜色**（只改色，形状 / 尺寸沿用现值）。
    pub fn with_grip_color(mut self, c: Color) -> Self {
        self.grip.color = c;
        self
    }
    /// **缩放柄形状**（只改形状，颜色 / 尺寸沿用现值）。
    pub fn with_grip_shape(mut self, shape: GripShape) -> Self {
        self.grip.shape = shape;
        self
    }
    /// **不画缩放柄**（等价 `shape = Hidden`；**仍可拖动缩放**，只是没有图案）。
    pub fn without_grip(mut self) -> Self {
        self.grip.shape = GripShape::Hidden;
        self
    }
}

impl ButtonStyle {
    /// **按交互态挑背景刷**：按下 > 悬停 > 常态。
    ///
    /// 抽成一个方法而不是各处手写 `if`：下拉框触发按钮曾写成
    /// `if open { bg_pressed } else { bg }`——**完全没有 hover 反馈**，鼠标移上去毫无
    /// 变化，与按钮 / 滑条的观感不一致。现在按钮与下拉框共用这一处，漏掉悬停态会在
    /// 两边同时暴露。
    #[inline]
    pub fn pick_bg(&self, pressed: bool, hovered: bool) -> Brush {
        if pressed {
            self.bg_pressed
        } else if hovered {
            self.bg_hover
        } else {
            self.bg
        }
    }

    /// 常态背景刷（接受 [`Color`] 或 [`Brush`]）。
    pub fn with_bg(mut self, c: impl Into<Brush>) -> Self {
        self.bg = c.into();
        self
    }
    /// 悬停背景刷。
    pub fn with_bg_hover(mut self, c: impl Into<Brush>) -> Self {
        self.bg_hover = c.into();
        self
    }
    /// 按下背景刷。
    pub fn with_bg_pressed(mut self, c: impl Into<Brush>) -> Self {
        self.bg_pressed = c.into();
        self
    }
    /// 文本前景色。
    pub fn with_fg(mut self, c: Color) -> Self {
        self.fg = c;
        self
    }
    pub fn with_border(mut self, c: Color) -> Self {
        self.border = c;
        self
    }
    pub fn with_border_w(mut self, w: f32) -> Self {
        self.border_w = w;
        self
    }
    /// 圆角半径（**逻辑像素**；0 = 直角）。
    pub fn with_radius(mut self, r: impl Into<CornerRadius>) -> Self {
        self.radius = r.into();
        self
    }
    /// 内边距（x = 水平，y = 垂直）。
    pub fn with_padding(mut self, p: impl Into<glam::Vec2>) -> Self {
        let p = p.into();
        self.padding = p;
        self
    }
    pub fn with_font_size(mut self, s: f32) -> Self {
        self.font_size = s;
        self
    }
    pub fn with_font_family(mut self, f: impl AsRef<str>) -> Self {
        self.font_family = Some(f.as_ref().into());
        self
    }
}

impl SliderStyle {
    /// 轨道刷（接受 [`Color`] 或 [`Brush`]：`Color: Into<Brush>` 等价纯色）。
    pub fn with_track(mut self, c: impl Into<Brush>) -> Self {
        self.track = c.into();
        self
    }
    /// 已填充部分刷（接受 [`Color`] 或 [`Brush`]）。
    pub fn with_fill(mut self, c: impl Into<Brush>) -> Self {
        self.fill = c.into();
        self
    }
    /// 手柄颜色。
    pub fn with_handle(mut self, c: Color) -> Self {
        self.handle = c;
        self
    }
    /// 手柄边框颜色。
    pub fn with_handle_border(mut self, c: Color) -> Self {
        self.handle_border = c;
        self
    }
    /// 轨道高度（像素）。
    pub fn with_track_h(mut self, h: f32) -> Self {
        self.track_h = h;
        self
    }
    /// 手柄宽度（像素）。
    pub fn with_handle_w(mut self, w: f32) -> Self {
        self.handle_w = w;
        self
    }
    /// 控件总高（含点击区）。
    pub fn with_height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
    /// 控件最小宽（pack 内自动尺寸用）。
    pub fn with_min_w(mut self, w: f32) -> Self {
        self.min_w = w;
        self
    }
    /// 轨道 / 填充 / 手柄的圆角（接受 `f32` 或 [`CornerRadius`]；`0` = 直角）。
    pub fn with_radius(mut self, r: impl Into<CornerRadius>) -> Self {
        self.radius = r.into();
        self
    }
}

impl InputStyle {
    /// 背景刷（接受 [`Color`] 或 [`Brush`]）。
    pub fn with_bg(mut self, c: impl Into<Brush>) -> Self {
        self.bg = c.into();
        self
    }
    pub fn with_border(mut self, c: Color) -> Self {
        self.border = c;
        self
    }
    /// 聚焦时的边框颜色。
    pub fn with_border_focus(mut self, c: Color) -> Self {
        self.border_focus = c;
        self
    }
    pub fn with_fg(mut self, c: Color) -> Self {
        self.fg = c;
        self
    }
    pub fn with_caret(mut self, c: Color) -> Self {
        self.caret = c;
        self
    }
    /// IME 组合候选串颜色。
    pub fn with_preedit(mut self, c: Color) -> Self {
        self.preedit = c;
        self
    }
    /// 文本选择高亮背景色。
    pub fn with_sel_bg(mut self, c: Color) -> Self {
        self.sel_bg = c;
        self
    }
    /// 缩放柄 / 拖动框颜色（可调整大小/宽度的文本输入框）。
    pub fn with_resize_handle(mut self, c: Color) -> Self {
        self.resize_handle = c;
        self
    }
    /// 缩放柄完整样式（形状 / 颜色 / 尺寸 / 个数；见 [`InputStyle::grip`]）。
    pub fn with_grip(mut self, grip: GripStyle) -> Self {
        self.grip = grip;
        self
    }
    /// 缩放柄**形状**（`Diagonal` 默认三条斜线 / `Bars` 三条横线 / `Squares` 历史观感 /
    /// `Hidden`——`Hidden` 只是不画图案，命中区照旧 ⇒ 仍能拖）。
    pub fn with_grip_shape(mut self, shape: GripShape) -> Self {
        self.grip.shape = shape;
        self
    }
    pub fn with_border_w(mut self, w: f32) -> Self {
        self.border_w = w;
        self
    }
    /// 内容水平内边距。
    pub fn with_padding_x(mut self, p: f32) -> Self {
        self.padding_x = p;
        self
    }
    /// 圆角半径（**逻辑像素**；0 = 直角）。
    pub fn with_radius(mut self, r: impl Into<CornerRadius>) -> Self {
        self.radius = r.into();
        self
    }
    /// 控件总高。
    pub fn with_height(mut self, h: f32) -> Self {
        self.height = h;
        self
    }
    /// 控件最小宽。
    pub fn with_min_w(mut self, w: f32) -> Self {
        self.min_w = w;
        self
    }
    pub fn with_font_size(mut self, s: f32) -> Self {
        self.font_size = s;
        self
    }
    pub fn with_font_family(mut self, f: impl AsRef<str>) -> Self {
        self.font_family = Some(f.as_ref().into());
        self
    }
}

impl DividerStyle {
    pub fn with_color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
    /// 线厚度（逻辑像素）。
    pub fn with_thickness(mut self, t: f32) -> Self {
        self.thickness = t;
        self
    }
    /// 上下留白（逻辑像素）。
    pub fn with_margin(mut self, m: f32) -> Self {
        self.margin = m;
        self
    }
}

impl CheckboxStyle {
    /// 方框边长。
    pub fn with_box_size(mut self, s: f32) -> Self {
        self.box_size = s;
        self
    }
    pub fn with_box_border(mut self, c: Color) -> Self {
        self.box_border = c;
        self
    }
    pub fn with_border_w(mut self, w: f32) -> Self {
        self.border_w = w;
        self
    }
    pub fn with_checked_fill(mut self, c: Color) -> Self {
        self.checked_fill = c;
        self
    }
    pub fn with_fg(mut self, c: Color) -> Self {
        self.fg = c;
        self
    }
    pub fn with_font_size(mut self, s: f32) -> Self {
        self.font_size = s;
        self
    }
    pub fn with_font_family(mut self, f: impl AsRef<str>) -> Self {
        self.font_family = Some(f.as_ref().into());
        self
    }
    /// 文本与方框间距。
    pub fn with_gap(mut self, g: f32) -> Self {
        self.gap = g;
        self
    }
}

impl DebugStyle {
    /// `debug_layout` 布局描边颜色。
    pub fn with_layout_outline(mut self, c: Color) -> Self {
        self.layout_outline = c;
        self
    }
    /// `debug_layout` 布局描边宽度（**物理像素**）。
    pub fn with_layout_outline_width(mut self, w: f32) -> Self {
        self.layout_outline_width = w;
        self
    }
}

impl FocusStyle {
    pub fn with_color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
    /// 焦点描边宽度（逻辑像素）。
    pub fn with_width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }
}

impl ModalStyle {
    /// 遮罩颜色（默认半透明黑）。
    pub fn with_dim(mut self, c: Color) -> Self {
        self.dim = c;
        self
    }
    /// 遮罩尺寸（**逻辑像素**）。
    pub fn with_size(mut self, s: impl Into<glam::Vec2>) -> Self {
        let s = s.into();
        self.size = Some(s);
        self
    }
    /// 全屏遮罩（默认）。
    pub fn with_fullscreen(mut self) -> Self {
        self.size = None;
        self
    }
}

/// 浅色主题（默认）：实现标准 [`Default`] 特化（`Theme::default()` 即浅色）。
impl Default for Theme {
    fn default() -> Self {
        Self::light()
    }
}

impl Theme {
    /// **从调色板组装整套主题**（换肤 / 新主题的唯一入口）。
    ///
    /// 全部子样式经各自的 `themed(&Palette)` 派生 ⇒ 新增一个主题 = 提供一份
    /// [`Palette`]，不必逐子样式写字面量。
    ///
    /// ```no_run
    /// # use rjw_ui::{Palette, Theme};
    /// // 只要改强调色：仍走 dark 的层次，只换 accent
    /// let mut p = Palette::dark();
    /// p.accent = rjw_color::Color::rgba_u8(255, 120, 200, 255);
    /// let theme = Theme::themed(&p);
    /// # let _ = theme;
    /// ```
    pub fn themed(p: &Palette) -> Self {
        Self {
            label: LabelStyle::themed(p),
            panel: PanelStyle::themed(p),
            button: ButtonStyle::themed(p),
            slider: SliderStyle::themed(p),
            input: InputStyle::themed(p),
            checkbox: CheckboxStyle::themed(p),
            divider: DividerStyle::themed(p),
            menubar: MenubarStyle::themed(p),
            debug: DebugStyle::themed(p),
            focus: FocusStyle::themed(p),
            modal: ModalStyle::themed(p),
            combo: ComboStyle::themed(p),
            foldable: FoldableStyle::themed(p),
            row_h: 26.0,
            gap: 6.0,
            feather: crate::tess::DEFAULT_FEATHER,
            line_spacing: DEFAULT_LINE_SPACING,
            font_weight: Weight::NORMAL,
            palette: *p,
        }
    }

    /// **边缘羽化宽度**（逻辑像素；0 = 硬边）。见 [`Theme::feather`] 字段文档。
    pub fn with_feather(mut self, px: f32) -> Self {
        self.feather = px.max(0.0);
        self
    }

    /// **全局投影**（窗口 / 面板 / 浮层的软阴影；整对象替换 `blur` + `offset` + `color`）。
    ///
    /// 只改 `panel.shadow`——**不级联**：只有 [`PanelStyle`] 带投影（按钮 / 输入框
    /// 这类薄控件加投影会糊成一团）。与 [`Self::with_border_w`] 的"级联到多个子样式"
    /// 语义不同，故这里刻意分开命名。
    ///
    /// ```no_run
    /// # use rjw_ui::{Palette, ShadowStyle, Theme};
    /// # use glam::Vec2;
    /// let t = Theme::themed(&Palette::light()).with_shadow(ShadowStyle {
    ///     blur: 24.0,
    ///     offset: Vec2::new(0.0, 8.0),
    ///     ..Default::default()
    /// });
    /// let flat = Theme::light().without_shadow();   // 平面风格：blur = 0
    /// # let _ = (t, flat);
    /// ```
    pub fn with_shadow(mut self, s: ShadowStyle) -> Self {
        self.panel.shadow = s;
        self
    }

    /// **关闭全局投影**（等价 `blur = 0`；与 `PanelStyle::without_shadow` 同风格，
    /// 约定 R3：用"0 = 关闭"而不是 `Option`）。
    pub fn without_shadow(mut self) -> Self {
        self.panel.shadow.blur = 0.0;
        self
    }

    /// **浅色主题**（= [`Default`]）：[`Palette::light`] 组装。
    pub fn light() -> Self {
        Self::themed(&Palette::light())
    }

    /// **深色主题**：[`Palette::dark`] 组装（低饱和冷灰阶梯 + 明亮蓝强调）。
    ///
    /// 比历史深色预设整体更暗、且层次单调递增——见 [`Palette::dark`] 的说明。
    pub fn dark() -> Self {
        Self::themed(&Palette::dark())
    }

    /// **历史深色主题**：复刻改造前的硬编码深色配色（`bevel = 0`，纯平色）。
    /// 供不接受新配色的下游一键回退。
    pub fn dark_legacy() -> Self {
        Self::themed(&Palette::legacy_dark())
    }

    /// 本主题使用的调色板（从当前子样式反推**不可行**，故只记录"最近一次用于
    /// 组装的主题来源"）。`Theme` 由 `themed` 组装时记录；手工改过字段后可能与
    /// 实际颜色不一致，仅作诊断/回退用途。
    pub fn palette(&self) -> Palette {
        self.palette
    }

    // ── with 链（责任链语义：链上后设覆盖先设；可级联的全局参数） ──

    /// **全局字体族**：级联到全部文本子样式（`label` / `button` / `checkbox` /
    /// `input` / `combo` / `menubar` / `foldable`——滑块无文本、面板无字体）。`None` = 系统默认。
    ///
    /// ```no_run
    /// # use rjw_ui::Theme;
    /// let theme = Theme::dark().with_font_family("Microsoft YaHei");
    /// # let _ = theme;
    /// ```
    pub fn with_font_family(mut self, family: impl AsRef<str>) -> Self {
        let f = Some(Arc::from(family.as_ref()));
        self.label.font_family = f.clone();
        self.button.font_family = f.clone();
        self.checkbox.font_family = f.clone();
        self.input.font_family = f.clone();
        self.combo.font_family = f.clone();
        self.menubar.font_family = f.clone();
        self.foldable.font_family = f;
        self
    }

    /// **全局字号**：级联到全部文本子样式（`label` / `button` / `checkbox` / `input` /
    /// `combo` / `menubar` / `foldable`）。
    pub fn with_font_size(mut self, size: f32) -> Self {
        self.label.font_size = size;
        self.button.font_size = size;
        self.checkbox.font_size = size;
        self.input.font_size = size;
        self.combo.font_size = size;
        self.menubar.font_size = size;
        self.foldable.font_size = size;
        self
    }

    /// **圆角半径**（逻辑像素；0 = 直角；可传 `f32` 或 [`CornerRadius`] 指定**只圆某些角**）。
    ///
    /// 级联到**全部有圆角的子样式**：`panel` / `button` / `input` / `checkbox` /
    /// `combo.menu_radius`。此前只覆盖前三个，于是"全局设了圆角但勾选框 / 下拉菜单
    /// 仍是直角"，看起来像 bug。
    ///
    /// 浮层（`combo`）用的是**更小的**圆角：逐角 `min(r, 6)`——菜单是贴边弹出的浮层，
    /// 与按钮同半径会显得笨重。
    ///
    /// ```no_run
    /// # use rjw_ui::{CornerRadius, Theme};
    /// // 只圆上面两个角（标签页 / 附着在工具栏下方的面板）
    /// let theme = Theme::dark().with_radius(CornerRadius { tl: 8.0, tr: 8.0, br: 0.0, bl: 0.0 });
    /// # let _ = theme;
    /// ```
    pub fn with_radius(mut self, radius: impl Into<CornerRadius>) -> Self {
        let r = radius.into();
        self.panel.radius = r;
        self.button.radius = r;
        self.input.radius = r;
        self.checkbox.radius = r.map(|v| v * 0.5);
        self.combo.menu_radius = r.map(|v| v.min(6.0));
        // 菜单栏：只级联**触发器**圆角（栏本身是通栏条，圆角恒 0 —— 见 `MenubarStyle::radius`）。
        self.menubar.trigger_radius = r.map(|v| v.min(6.0));
        // 可收缩区块：标题行是行内控件（悬停高亮块），与触发器同口径取 min(r, 6)。
        self.foldable.radius = r.map(|v| v.min(6.0));
        self
    }

    /// **边框宽度**（逻辑像素；0 = 不画边框）。
    ///
    /// 级联到全部**有边框**的子样式：`panel` / `button` / `input` / `checkbox` /
    /// `menubar`（栏的**底边线**宽）。
    /// 边框**颜色**不是主题标量而是调色板令牌（[`Palette::border`] 常规 /
    /// [`Palette::border_strong`] 强描边）——换色请改调色板，这样"面板 / 按钮 /
    /// 输入框"的描边深浅关系不会各自漂移。
    pub fn with_border_w(mut self, w: f32) -> Self {
        let w = w.max(0.0);
        self.panel.border_w = w;
        self.button.border_w = w;
        self.input.border_w = w;
        self.checkbox.border_w = w;
        self.menubar.border_w = w;
        self.foldable.border_w = w;
        self
    }

    /// pack / grid 默认子项间距（像素）。
    pub fn with_gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// **单行控件统一高度**（水平行 `row(...)` 内子项强制等高；默认 26）。
    pub fn with_row_h(mut self, row_h: f32) -> Self {
        self.row_h = row_h;
        self
    }

    /// **UI 密度预设**（紧凑 / 标准 / 宽松）：一次性把"间距类"令牌与字号按比例缩放，
    /// 于是同一个界面既可以在小屏上排得下，也可以在大屏上更舒展。
    ///
    /// - [`Density::Compact`]：间距 ×0.84、字号 ×0.92、行距 ×1.10；
    /// - [`Density::Cozy`]（默认）：×1.0 / ×1.0 / ×[`DEFAULT_LINE_SPACING`]（= 今天的样子）；
    /// - [`Density::Spacious`]：间距 ×1.18、字号 ×1.08、行距 ×1.30。
    ///
    /// ⚠ 与其它 `with_*` 一样是**责任链**（链上后设覆盖先设，且**叠乘**）：先
    /// `.density(..)` 再 `.with_gap(20.0)` ⇒ gap = 20；反序则 gap 会被密度再缩放。
    /// 只想调某一维时用 [`Self::with_font_scale`] / [`Self::with_spacing_scale`] /
    /// [`Self::with_line_spacing`]。
    ///
    /// ```no_run
    /// # use rjw_ui::{Density, Theme};
    /// let compact = Theme::dark().density(Density::Compact);   // 小巧：小屏 / 工具面板
    /// let loose = Theme::dark().density(Density::Spacious);    // 宽松：触屏 / 演示
    /// # let _ = (compact, loose);
    /// ```
    pub fn density(self, d: Density) -> Self {
        let (sp, fs, ls) = d.scales();
        self.with_spacing_scale(sp)
            .with_font_scale(fs)
            .with_line_spacing(ls)
    }

    /// **只缩字号**（全部文本子样式：`label` / `button` / `checkbox` / `input` / `combo`）。
    ///
    /// 与 [`Self::with_font_size`] 的区别：后者是**绝对值**，本方法是**倍率**（在现值上叠乘）。
    pub fn with_font_scale(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).max(1.0).round();
        self.label.font_size = m(self.label.font_size);
        self.button.font_size = m(self.button.font_size);
        self.checkbox.font_size = m(self.checkbox.font_size);
        self.input.font_size = m(self.input.font_size);
        self.combo.font_size = m(self.combo.font_size);
        self
    }

    /// **只缩间距类令牌**（间距 / 行高 / 内边距 / 控件尺寸 / 浮层留白）。
    ///
    /// **不碰**：圆角（用户的造型选择）、边框宽 / 羽化（亚像素级观感）、颜色、阴影。
    pub fn with_spacing_scale(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.gap = m(self.gap).max(1.0);
        self.row_h = m(self.row_h).max(1.0);
        self.panel.padding = m(self.panel.padding);
        self.button.padding = Vec2::new(m(self.button.padding.x), m(self.button.padding.y));
        self.slider.track_h = m(self.slider.track_h).max(1.0);
        self.slider.handle_w = m(self.slider.handle_w).max(2.0);
        self.slider.height = m(self.slider.height).max(1.0);
        self.slider.min_w = m(self.slider.min_w);
        self.input.padding_x = m(self.input.padding_x);
        self.input.height = m(self.input.height).max(1.0);
        self.input.min_w = m(self.input.min_w);
        self.checkbox.box_size = m(self.checkbox.box_size).max(2.0);
        self.checkbox.gap = m(self.checkbox.gap);
        self.divider.margin = m(self.divider.margin);
        self.combo.menu_pad_v = m(self.combo.menu_pad_v);
        self.combo.item_pad_x = m(self.combo.item_pad_x);
        self.combo.item_min_w = m(self.combo.item_min_w);
        self
    }

    /// **多行行距倍率**（行高 = 字号 × 该值；默认 [`DEFAULT_LINE_SPACING`] = 1.2）。
    ///
    /// 作用于**可能换行的文本**：TextArea、自动换行标签、换行预览（单行文本的盒子高度
    /// 仍是字号，不受影响）。调小 = 更紧凑的多行排版，调大 = 更疏朗。
    pub fn with_line_spacing(mut self, mult: f32) -> Self {
        self.line_spacing = mult.max(0.5);
        self
    }

    /// **全局字重**（[`Weight`]；见 [`Theme::font_weight`]）。
    ///
    /// ```no_run
    /// # use rjw_ui::{Theme, Weight};
    /// let bold = Theme::dark().with_font_weight(Weight::BOLD);      // 700
    /// let semi = Theme::dark().with_font_weight(Weight(600));       // 任意数值
    /// ```
    ///
    /// ⚠ 字重会改变**字形与步进宽度**（不只是"看着粗一点"）⇒ 布局随之变化。引擎在
    /// 三处保证不串味：排版缓冲缓存键（`UiState.text_buffers` / `WidgetState::text_buf`）
    /// 含字重、窗口 / win=0 子槽的几何签名以字重为前缀（[`crate::ui::Ui`] 的
    /// `hash_cmds`，与 `line_spacing` 同一机制）——漏掉后者会"改了字重但窗口几何仍命中
    /// 旧顶点缓存"（固定矩形里的居中文本尤其明显）。
    pub fn with_font_weight(mut self, w: Weight) -> Self {
        self.font_weight = w;
        self
    }

    /// 整体替换子样式（链上最后一个 `with_xxx` 生效）。
    pub fn with_label(mut self, s: LabelStyle) -> Self {
        self.label = s;
        self
    }
    pub fn with_panel(mut self, s: PanelStyle) -> Self {
        self.panel = s;
        self
    }
    pub fn with_button(mut self, s: ButtonStyle) -> Self {
        self.button = s;
        self
    }
    pub fn with_slider(mut self, s: SliderStyle) -> Self {
        self.slider = s;
        self
    }
    pub fn with_input(mut self, s: InputStyle) -> Self {
        self.input = s;
        self
    }
    pub fn with_checkbox(mut self, s: CheckboxStyle) -> Self {
        self.checkbox = s;
        self
    }
    pub fn with_divider(mut self, s: DividerStyle) -> Self {
        self.divider = s;
        self
    }
    /// **菜单栏样式**（[`Ui::menu_bar`](crate::ui::Ui::menu_bar)：栏底 / 触发器 / 竖分割线）。
    ///
    /// ```no_run
    /// # use rjw_ui::{MenubarStyle, Theme};
    /// // 整组替换，或只改想要的字段（`..Theme::dark().menubar` 保留其余默认）
    /// let t = Theme::dark().with_menubar(MenubarStyle {
    ///     border_w: 0.0,   // 不要底边线
    ///     padding: 6.0,    // 栏更厚一点
    ///     ..Theme::dark().menubar
    /// });
    /// # let _ = t;
    /// ```
    pub fn with_menubar(mut self, s: MenubarStyle) -> Self {
        self.menubar = s;
        self
    }
    pub fn with_focus(mut self, s: FocusStyle) -> Self {
        self.focus = s;
        self
    }
    pub fn with_debug(mut self, s: DebugStyle) -> Self {
        self.debug = s;
        self
    }
    pub fn with_modal(mut self, s: ModalStyle) -> Self {
        self.modal = s;
        self
    }
    pub fn with_combo(mut self, s: ComboStyle) -> Self {
        self.combo = s;
        self
    }
    /// **可收缩区块样式**（[`Ui::foldable`](crate::ui::Ui::foldable) 的标题行）。
    ///
    /// ```no_run
    /// use rjw_color::Color;
    /// # use rjw_ui::{Brush, FoldableStyle, Theme};
    /// // 整组替换，或只改想要的字段（`..Theme::dark().foldable` 保留其余默认）
    /// let t = Theme::dark().with_foldable(FoldableStyle {
    ///     bg_hover: Brush::Solid(Color::TRANSPARENT), // 关掉悬停高亮
    ///     ..Theme::dark().foldable
    /// });
    /// # let _ = t;
    /// ```
    pub fn with_foldable(mut self, s: FoldableStyle) -> Self {
        self.foldable = s;
        self
    }

    /// **预乘 DPI scale**：逐一调用每个子样式的 [`scaled`](LabelStyle::scaled)（尺寸 /
    /// 字号字段 × `s` 取整，保布局整数不变量）+ 主题级 `gap` / `row_h`。
    ///
    /// 由 `Ui::begin(..).scale_factor(s).build()` 内部调用——此后 Ui 内部以
    /// **物理像素**为单位（布局 / 命中 / 绘制零 scale 换算）。颜色 / 字体族不变；
    /// `s <= 0` 视为 1（不缩放）。**顺序无关**：无论先 `with_*` 还是先 `scale_factor`，
    /// 最终 `build()` 统一预乘一次。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).floor();
        self.label = self.label.scaled(s);
        self.panel = self.panel.scaled(s);
        self.button = self.button.scaled(s);
        self.slider = self.slider.scaled(s);
        self.input = self.input.scaled(s);
        self.checkbox = self.checkbox.scaled(s);
        self.divider = self.divider.scaled(s);
        self.menubar = self.menubar.scaled(s);
        self.debug = self.debug.scaled(s);
        self.focus = self.focus.scaled(s);
        self.modal = self.modal.scaled(s);
        self.combo = self.combo.scaled(s);
        self.foldable = self.foldable.scaled(s);
        self.gap = m(self.gap);
        self.row_h = m(self.row_h);
        // 羽化宽也按 DPI 物理化；但不取整（亚像素级软边，取整会让 0.5 逻辑像素消失）。
        self.feather *= s;
        self
    }
}

// ─── LabelStyle ───────────────────────────────────────────
impl LabelStyle {
    /// **原地应用调色板**：只改颜色，字号 / 字体族 / 对齐等保留。
    pub fn set_palette(&mut self, p: &Palette) {
        self.color = p.text;
    }

    /// 从调色板派生（= `Default` 起步 + `set_palette`）。
    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── PanelStyle ───────────────────────────────────────────
impl PanelStyle {
    pub fn set_palette(&mut self, p: &Palette) {
        self.bg = bevel_raised(p.surface, p.bevel);
        self.border = p.border;
        // 只覆盖 shadow.color / grip.color，保留 blur / offset / shape / size 等
        self.shadow.color = p.shadow;
        self.grip.color = p.border;
    }

    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── ButtonStyle ──────────────────────────────────────────
impl ButtonStyle {
    pub fn set_palette(&mut self, p: &Palette) {
        self.bg = bevel_raised(p.surface_raised, p.bevel);
        self.bg_hover = bevel_raised(p.surface_hover, p.bevel);
        self.bg_pressed = bevel_raised(p.surface_active, p.bevel);
        self.fg = p.text;
        self.border = p.border_strong;
    }

    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── SliderStyle ──────────────────────────────────────────
impl SliderStyle {
    pub fn set_palette(&mut self, p: &Palette) {
        self.track = p.surface_sunken.into();
        self.fill = p.accent.into();
        self.handle = p.handle;
        self.handle_border = p.border_strong;
    }

    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── InputStyle ───────────────────────────────────────────
impl InputStyle {
    pub fn set_palette(&mut self, p: &Palette) {
        self.bg = bevel_sunken(p.surface_sunken, p.bevel);
        self.border = p.border_strong;
        self.border_focus = p.accent;
        self.fg = p.text;
        self.caret = p.text;
        self.preedit = p.text_muted;
        self.sel_bg = p.selection;
        self.resize_handle = p.text_dim;
        self.grip.color = p.text_dim;
    }

    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── CheckboxStyle ────────────────────────────────────────
impl CheckboxStyle {
    pub fn set_palette(&mut self, p: &Palette) {
        self.box_border = p.border_strong;
        self.checked_fill = p.accent;
        self.fg = p.text;
    }

    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── DividerStyle ─────────────────────────────────────────
impl DividerStyle {
    pub fn set_palette(&mut self, p: &Palette) {
        self.color = p.border;
    }

    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── MenubarStyle ─────────────────────────────────────────
impl MenubarStyle {
    pub fn set_palette(&mut self, p: &Palette) {
        self.bg = p.surface_raised;
        self.border = p.border;
        self.fg = p.text;
        // **常态透明**：菜单条不是一排按钮（见结构体文档）。
        self.trigger_bg = Color::TRANSPARENT;
        self.trigger_hover = p.surface_hover;
        self.trigger_pressed = p.surface_active;
        self.separator = p.border;
    }

    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── DebugStyle ───────────────────────────────────────────
impl DebugStyle {
    pub fn set_palette(&mut self, p: &Palette) {
        self.layout_outline = p.debug_outline;
    }

    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── FocusStyle ───────────────────────────────────────────
impl FocusStyle {
    pub fn set_palette(&mut self, p: &Palette) {
        self.color = p.accent;
    }

    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── ModalStyle ───────────────────────────────────────────
impl ModalStyle {
    pub fn set_palette(&mut self, p: &Palette) {
        self.dim = p.scrim;
    }

    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}

// ─── ComboStyle ───────────────────────────────────────────
impl ComboStyle {
    pub fn set_palette(&mut self, p: &Palette) {
        self.menu_bg = p.surface_overlay;
        self.menu_border = p.border_strong;
        self.item_hover = p.surface_hover;
        self.item_selected = p.selection;
        self.fg = p.text;
        self.fg_mark = p.accent;
    }

    pub fn themed(p: &Palette) -> Self {
        let mut s = Self::default();
        s.set_palette(p);
        s
    }
}


impl Theme {
    /// **原地换肤**：把调色板应用到当前主题的**每个子样式**，只改颜色类字段。
    ///
    /// 与 `Theme::themed(p)` 的区别：
    /// - `Theme::themed(p)`：**从 `Default` 重建**整套主题（尺寸 / 圆角 / 字号回到出厂值）；
    /// - `Theme::set_palette(p)`：**只覆盖颜色**，保留现有造型
    ///   （圆角 / padding / 字号 / 字体族 / 密度 / 阴影 blur / grip 形状 …）。
    ///
    /// 典型用法：运行中切 light / dark，而保留用户对密度、字体、圆角的自定义。
    ///
    /// ```no_run
    /// # use rjw_ui::{Palette, Theme};
    /// let mut t = Theme::light()
    ///     .with_radius(8.0)
    ///     .with_font_size(16.0)
    ///     .density(rjw_ui::Density::Compact);
    ///
    /// // 用户点了"切深色"：
    /// t.set_palette(&Palette::dark());
    /// // t 仍是 8 圆角 / 16 字号 / Compact，但颜色全部变深色。
    /// ```
    ///
    /// ⚠ 之前通过 `with_*` 手工改过的**颜色**字段会被本次覆盖——这正是"换肤"的
    /// 应有之义：颜色由调色板决定。`self.palette` 会更新为 `*p`。
    pub fn set_palette(&mut self, p: &Palette) {
        self.label.set_palette(p);
        self.panel.set_palette(p);
        self.button.set_palette(p);
        self.slider.set_palette(p);
        self.input.set_palette(p);
        self.checkbox.set_palette(p);
        self.divider.set_palette(p);
        self.menubar.set_palette(p);
        self.debug.set_palette(p);
        self.focus.set_palette(p);
        self.modal.set_palette(p);
        self.combo.set_palette(p);
        self.foldable.set_palette(p);
        self.palette = *p;
    }

    /// 链式版：`t.with_palette(&p)` 等价于 `{ t.set_palette(&p); t }`。
    pub fn with_palette(mut self, p: &Palette) -> Self {
        self.set_palette(p);
        self
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_chain_cascades_font_family_and_size() {
        let t = Theme::dark()
            .with_font_family("Microsoft YaHei")
            .with_font_size(16.0);
        // 级联：四个文本子样式全部生效
        assert_eq!(t.label.font_family.as_deref(), Some("Microsoft YaHei"));
        assert_eq!(t.button.font_family.as_deref(), Some("Microsoft YaHei"));
        assert_eq!(t.checkbox.font_family.as_deref(), Some("Microsoft YaHei"));
        assert_eq!(t.input.font_family.as_deref(), Some("Microsoft YaHei"));
        // 菜单栏触发器也是"文本子样式"：漏了它就会出现"全局换字体、菜单条还是旧字体"。
        assert_eq!(t.menubar.font_family.as_deref(), Some("Microsoft YaHei"));
        assert_eq!(t.label.font_size, 16.0);
        assert_eq!(t.button.font_size, 16.0);
        assert_eq!(t.checkbox.font_size, 16.0);
        assert_eq!(t.input.font_size, 16.0);
        assert_eq!(t.menubar.font_size, 16.0);
        // 无文本子样式不受影响
        assert_eq!(t.slider.track, Theme::dark().slider.track);
    }

    /// **菜单栏样式**（本轮新增）：从调色板派生（常态底色必须**透明**，否则菜单条变"一排
    /// 按钮"）、DPI 预乘只动尺寸、全局 `with_border_w` / `with_font_size` 级联到它。
    #[test]
    fn menubar_style_themes_scales_and_cascades() {
        let p = Palette::dark();
        let t = Theme::themed(&p);
        assert_eq!(
            <[f32; 4]>::from(t.menubar.trigger_bg)[3],
            0.0,
            "触发器常态底色必须全透明（纯文字菜单条）"
        );
        assert_eq!(t.menubar.bg, p.surface_raised);
        assert_eq!(t.menubar.trigger_hover, p.surface_hover);
        assert_eq!(t.menubar.trigger_pressed, p.surface_active);
        assert_eq!(t.menubar.fg, p.text);
        assert_eq!(t.menubar.border, p.border);
        assert_eq!(t.menubar.separator, p.border);

        // DPI 预乘：尺寸 × s 取整、颜色不变（`Ui` 内部一律物理像素）。
        let s = t.clone().scaled(1.5);
        assert_eq!(s.menubar.padding, (4.0f32 * 1.5).round());
        assert_eq!(s.menubar.gap, (2.0f32 * 1.5).round());
        assert_eq!(s.menubar.trigger_pad_x, (10.0f32 * 1.5).round());
        assert_eq!(s.menubar.separator_margin, (6.0f32 * 1.5).round());
        assert_eq!(s.menubar.border_w, (1.0f32 * 1.5).round());
        assert_eq!(s.menubar.bg, t.menubar.bg);

        // 全局级联：`with_border_w(0)` 也必须把菜单条的**底边线**关掉（否则"全局扁平化、
        // 菜单条还留一条线"）；圆角只级联到**触发器**（栏本身是通栏条，恒 0）。
        let flat = Theme::dark().with_border_w(0.0).with_radius(8.0);
        assert_eq!(flat.menubar.border_w, 0.0);
        assert_eq!(flat.menubar.radius, CornerRadius::default(), "栏圆角恒定（通栏条）");
        assert_eq!(flat.menubar.trigger_radius, CornerRadius::all(6.0), "触发器取 min(r, 6)");
    }

    /// **可收缩区块样式**（[`FoldableStyle`]）：从调色板派生（常态底色必须**透明**，
    /// 否则每个区块标题看起来都是一个按钮）、三态 `pick_bg` 优先级、DPI 预乘只动尺寸、
    /// 全局 `with_radius` / `with_border_w` / `with_font_*` 级联到它。
    #[test]
    fn foldable_style_themes_scales_and_cascades() {
        let p = Palette::dark();
        let t = Theme::themed(&p);
        // 常态必须全透明（区块标题不是按钮）——与 `MenubarStyle.trigger_bg` 同一口径。
        // ⚠ 底色是 `Brush`（纯色 / 渐变），取纯色分量必须 match（`Brush` 没有 `Into<[f32;4]>`）。
        let Brush::Solid(bg) = t.foldable.bg else { panic!("标题行常态底色应是纯色刷") };
        assert_eq!(<[f32; 4]>::from(bg), [0.0, 0.0, 0.0, 0.0], "标题行常态底色必须全透明");
        assert_eq!(t.foldable.fg, p.text);
        assert_eq!(t.foldable.mark, p.text_muted, "三角比文字弱一档");
        assert!(matches!(t.foldable.bg_hover, Brush::Solid(c) if c == p.surface_hover));

        // 三态优先级：按下 > 悬停 > 常态（与 `ButtonStyle::pick_bg` 同一处纪律）。
        assert_eq!(t.foldable.pick_bg(false, false), t.foldable.bg);
        assert_eq!(t.foldable.pick_bg(false, true), t.foldable.bg_hover);
        assert_eq!(t.foldable.pick_bg(true, true), t.foldable.bg_pressed);
        assert_eq!(t.foldable.pick_bg(true, false), t.foldable.bg_pressed);

        // DPI 预乘：尺寸 / 字号按 floor × s，颜色不变。
        let s = t.clone().scaled(1.5);
        assert_eq!(s.foldable.font_size, (14.0f32 * 1.5).floor());
        assert_eq!(s.foldable.pad_x, (4.0f32 * 1.5).floor());
        assert_eq!(s.foldable.icon_w, (18.0f32 * 1.5).floor());
        assert_eq!(s.foldable.icon_h, (10.0f32 * 1.5).floor());
        assert_eq!(s.foldable.bg, t.foldable.bg, "颜色不随 DPI 变");

        // 全局级联：圆角取 min(r, 6)（行内高亮块，与菜单触发器同口径）、边框宽可关、
        // 字体族 / 字号一起走。
        let c = Theme::dark()
            .with_radius(8.0)
            .with_border_w(0.0)
            .with_font_family("Microsoft YaHei")
            .with_font_size(16.0);
        assert_eq!(c.foldable.radius, CornerRadius::all(6.0));
        assert_eq!(c.foldable.border_w, 0.0);
        assert_eq!(c.foldable.font_family.as_deref(), Some("Microsoft YaHei"));
        assert_eq!(c.foldable.font_size, 16.0);

        // 逐字段覆盖（builder）与整组替换（`with_foldable`）都要能改掉调色板给的颜色。
        let st = FoldableStyle::default().with_bg(Color::RED).with_mark(Color::BLUE);
        assert_eq!(st.bg, Brush::Solid(Color::RED));
        assert_eq!(st.mark, Color::BLUE);
        let t2 = Theme::dark().with_foldable(FoldableStyle::default().with_font_size(20.0));
        assert_eq!(t2.foldable.font_size, 20.0);

        // **正文归属提示**（缩进 + 左侧竖引导线）：默认有缩进与线；线色随调色板；
        // DPI 预乘把缩进 / 线宽 / 尾巴一起物理化；`guide_w = 0` 是"关掉引导线"的开关。
        assert!(t.foldable.body_indent > 0.0 && t.foldable.guide_w > 0.0, "默认开着归属提示");
        assert_eq!(t.foldable.guide, p.border, "引导线取面板描边色（弱线，不抢视觉）");
        assert_eq!(s.foldable.body_indent, (12.0f32 * 1.5).floor());
        assert_eq!(s.foldable.guide_w, (1.0f32 * 1.5).floor());
        assert_eq!(s.foldable.guide_tail, (2.0f32 * 1.5).floor());
        assert_eq!(s.foldable.guide, t.foldable.guide, "颜色不随 DPI 变");
        let off = FoldableStyle::default().with_body_guide(0.0, Color::WHITE, 0.0, 0.0);
        assert_eq!((off.body_indent, off.guide_w), (0.0, 0.0), "可整体关掉归属提示");
    }

    #[test]
    fn with_chain_last_link_wins() {
        // 责任链语义：后设覆盖先设
        let t = Theme::default()
            .with_radius(6.0)
            .with_radius(0.0)
            .with_gap(8.0);
        assert_eq!(t.panel.radius, 0.0);
        assert_eq!(t.button.radius, 0.0);
        assert_eq!(t.input.radius, 0.0);
        assert_eq!(t.gap, 8.0);
    }

    #[test]
    fn with_substyle_replaces_whole_style() {
        let mut button = Theme::dark().button;
        button.bg = Color::RED.into();
        let t = Theme::dark().with_button(button);
        assert_eq!(t.button.bg, Color::RED);
        // 未替换的子样式仍是 dark 预设
        assert_eq!(t.panel.bg, Theme::dark().panel.bg);
    }

    #[test]
    fn substyle_builders_chain_apply() {
        // 面板/按钮/滑块等子样式的 with_* 责任链：只改链上字段，其余回落默认。
        let p = PanelStyle::default()
            .with_bg(Color::RED)
            .with_radius(8.0)
            .with_padding(10.0);
        assert_eq!(p.bg, Color::RED);
        assert_eq!(p.radius, 8.0);
        assert_eq!(p.padding, 10.0);
        assert_eq!(p.border, PanelStyle::default().border, "未设字段回落默认");

        let b = ButtonStyle::default()
            .with_bg(Color::RED)
            .with_bg_hover(Color::BLUE)
            .with_radius(6.0)
            .with_font_family("Microsoft YaHei");
        assert_eq!(b.bg, Color::RED);
        assert_eq!(b.bg_hover, Color::BLUE);
        assert_eq!(b.radius, 6.0);
        assert_eq!(b.font_family.as_deref(), Some("Microsoft YaHei"));
        assert_eq!(b.bg_pressed, ButtonStyle::default().bg_pressed);

        let s = SliderStyle::default()
            .with_track(Color::BLACK)
            .with_fill(Color::WHITE)
            .with_handle_border(Color::RED)
            .with_min_w(200.0);
        assert_eq!(s.track, Color::BLACK);
        assert_eq!(s.fill, Color::WHITE);
        assert_eq!(s.handle_border, Color::RED);
        assert_eq!(s.min_w, 200.0);
        assert_eq!(s.height, SliderStyle::default().height, "未设字段回落默认");
        // 轨道 / 填充是**刷**（纯色或两端色渐变）：默认必须是纯色，
        // 这样滚动条与普通滑块的绘制仍走最省的 `Solid` 路径（不是渐变四角）。
        assert!(
            SliderStyle::default().track.as_solid().is_some(),
            "默认轨道应为纯色（as_solid 命中）"
        );
        // 渐变轨（取色器通道行）用 `with_track(Brush::Horizontal(..))`。
        let g = SliderStyle::default().with_track(Brush::Horizontal(Color::BLACK, Color::WHITE));
        assert!(g.track.as_solid().is_none(), "两端色不同 ⇒ 不是纯色");
        assert_eq!(g.track.corners(), [Color::BLACK, Color::WHITE, Color::BLACK, Color::WHITE]);

        let i = InputStyle::default().with_radius(4.0).with_sel_bg(Color::RED);
        assert_eq!(i.radius, 4.0);
        assert_eq!(i.sel_bg, Color::RED);
        assert_eq!(i.height, InputStyle::default().height);

        let l = LabelStyle::default().with_font_size(16.0).with_color(Color::RED);
        assert_eq!(l.font_size, 16.0);
        assert_eq!(l.color, Color::RED);

        let m = ModalStyle::default()
            .with_dim(Color::BLACK)
            .with_size(glam::Vec2::new(100.0, 80.0));
        assert_eq!(m.dim, Color::BLACK);
        assert_eq!(m.size, Some(glam::Vec2::new(100.0, 80.0)));
        // with_fullscreen 恢复全屏遮罩。
        assert_eq!(ModalStyle::default().with_fullscreen().size, None);
    }

    // ─── Brush（背景刷） ──────────────────────────────────────

    #[test]
    fn brush_corners_map_endpoints_to_the_right_quadrants() {
        let (t, b) = (Color::RED, Color::BLUE);
        // 垂直 = 上 → 下：TL/TR 取上端色，BL/BR 取下端色
        assert_eq!(
            Brush::Vertical(t, b).corners(),
            [t, t, b, b],
            "垂直渐变的两端必须落在上下两行"
        );
        // 水平 = 左 → 右：TL/BL 取左端色，TR/BR 取右端色
        assert_eq!(
            Brush::Horizontal(t, b).corners(),
            [t, b, t, b],
            "水平渐变的两端必须落在左右两列"
        );
        assert_eq!(Brush::Solid(t).corners(), [t; 4]);
    }

    #[test]
    fn brush_as_solid_detects_degenerate_gradients() {
        assert_eq!(Brush::Solid(Color::RED).as_solid(), Some(Color::RED));
        // 两端同色的渐变等价于纯色（用于挑更省的绘制路径）
        assert_eq!(Brush::Vertical(Color::RED, Color::RED).as_solid(), Some(Color::RED));
        assert_eq!(Brush::Horizontal(Color::RED, Color::RED).as_solid(), Some(Color::RED));
        assert_eq!(Brush::Vertical(Color::RED, Color::BLUE).as_solid(), None);
    }

    #[test]
    fn brush_compares_against_plain_color() {
        // 主题断言 / 用户代码可直接与 Color 比：纯色等价才算相等。
        assert_eq!(Brush::Solid(Color::RED), Color::RED);
        assert_ne!(Brush::Vertical(Color::RED, Color::BLUE), Color::RED);
        assert_eq!(Brush::Vertical(Color::RED, Color::RED), Color::RED);
    }

    #[test]
    fn brush_from_color_and_map_colors() {
        let b: Brush = Color::GREEN.into();
        assert_eq!(b, Color::GREEN);
        // map_colors 对渐变逐端施加
        let mapped = Brush::Vertical(Color::RED, Color::BLUE).map_colors(|_| Color::BLACK);
        assert_eq!(mapped, Brush::Vertical(Color::BLACK, Color::BLACK));
    }

    #[test]
    fn dark_theme_backgrounds_are_gradients_not_flat() {
        // 深色主题的默认背景是**纵向微渐变**（上亮下暗 / 输入框反向）。
        let t = Theme::dark();
        let panel = t.panel.bg.corners();
        assert_eq!(panel[0], panel[1], "面板上沿同色");
        assert_eq!(panel[2], panel[3], "面板下沿同色");
        assert_ne!(panel[0], panel[2], "面板上下不同色（确有渐变）");
        // 输入框反向：上暗下亮
        let input = t.input.bg.corners();
        assert!(luma(input[0]) < luma(input[2]), "输入框应上暗下亮（凹陷感）");
        // 按钮正向：上亮下暗
        let btn = t.button.bg.corners();
        assert!(luma(btn[0]) > luma(btn[2]), "按钮应上亮下暗（凸起感）");
    }

    /// 近似亮度（仅用于比较明暗方向）。
    fn luma(c: Color) -> f32 {
        let a: [f32; 4] = c.into();
        0.2126 * a[0] + 0.7152 * a[1] + 0.0722 * a[2]
    }

    // ─── Palette（配色令牌） ────────────────────────────────

    #[test]
    fn dark_palette_elevation_is_monotonic() {
        // 深色主题的层次必须单调递增（这是"能分辨面板 / 卡片 / 菜单"的前提）。
        let p = Palette::dark();
        let dim = luma(p.surface_dim);
        let s = luma(p.surface);
        let raised = luma(p.surface_raised);
        let overlay = luma(p.surface_overlay);
        let hover = luma(p.surface_hover);
        let active = luma(p.surface_active);
        assert!(dim < s, "画布底应比面板更暗：{dim} < {s}");
        assert!(s < raised, "面板 < 抬升：{s} < {raised}");
        assert!(raised < overlay, "抬升 < 浮层：{raised} < {overlay}");
        assert!(overlay < hover, "浮层 < 悬停：{overlay} < {hover}");
        assert!(hover < active, "悬停 < 激活：{hover} < {active}");
        // 凹陷表面刻意比面板更暗（"可编辑"的通用暗示）
        assert!(luma(p.surface_sunken) < s, "输入框应比面板更暗（凹陷）");
    }

    #[test]
    fn dark_theme_is_actually_dark() {
        // 用户反馈"dark 主题不够暗"：面板的感知亮度必须低于 ~15%。
        let p = Palette::dark();
        assert!(luma(p.surface) < 0.15, "面板太亮：{}", luma(p.surface));
        assert!(luma(p.surface_overlay) < 0.25, "浮层太亮");
        // 但正文必须足够亮以保持对比度（> 0.7 的感知亮度）。
        assert!(luma(p.text) > 0.7, "正文太暗：{}", luma(p.text));
        // 强调色是"冷蓝"而非洗白的蓝：蓝分量显著高于红。
        let a: [f32; 4] = p.accent.into();
        assert!(a[2] > a[0] + 0.3, "强调色应偏蓝：{a:?}");
    }

    #[test]
    fn dark_and_light_palettes_differ_in_every_surface_token() {
        // 同一份 `themed()` 映射要能服务两套调色板 ⇒ 每个表面令牌都必须不同，
        // 否则说明某个令牌被漏掉、两套主题在该处会撞色。
        let (d, l) = (Palette::dark(), Palette::light());
        assert_ne!(d.surface_dim, l.surface_dim);
        assert_ne!(d.surface_sunken, l.surface_sunken);
        assert_ne!(d.surface, l.surface);
        assert_ne!(d.surface_raised, l.surface_raised);
        assert_ne!(d.surface_overlay, l.surface_overlay);
        assert_ne!(d.surface_hover, l.surface_hover);
        assert_ne!(d.surface_active, l.surface_active);
        assert_ne!(d.text, l.text);
        assert_ne!(d.border, l.border);
        assert_ne!(d.scrim, l.scrim);
    }

    #[test]
    fn surface_sunken_is_darker_than_surface_in_dark_but_brighter_in_light() {
        // "凹陷"在深色 = 更暗、在浅色 = 更亮（白底输入框 vs 灰面板）。
        assert!(luma(Palette::dark().surface_sunken) < luma(Palette::dark().surface));
        assert!(luma(Palette::light().surface_sunken) > luma(Palette::light().surface));
    }

    #[test]
    fn themed_records_palette_and_legacy_reproduces_old_dark() {
        let t = Theme::dark();
        assert_eq!(t.palette, Palette::dark());
        // legacy：bevel = 0 ⇒ 纯平色（不是"两端同色的渐变"）
        let lg = Theme::dark_legacy();
        assert_eq!(lg.palette, Palette::legacy_dark());
        assert_eq!(lg.panel.bg, Brush::Solid(Color::rgba_u8(38, 42, 52, 255)));
        assert_eq!(lg.input.bg, Brush::Solid(Color::rgba_u8(28, 32, 40, 255)));
        assert_eq!(lg.button.bg_hover, Brush::Solid(Color::rgba_u8(66, 76, 96, 255)));
        // 新 dark 则是真渐变（两端不同色）
        assert!(Theme::dark().panel.bg.as_solid().is_none(), "新 dark 面板应有微渐变");
    }

    #[test]
    fn light_theme_is_the_default_and_matches_palette_light() {
        assert_eq!(Theme::default().palette, Palette::light());
        assert_eq!(Theme::light().palette, Palette::light());
    }

    #[test]
    fn with_radius_cascades_to_every_rounded_substyle() {
        // 历史 BUG：只级联 panel/button/input，勾选框与下拉菜单仍是直角。
        let t = Theme::dark().with_radius(8.0);
        assert_eq!(t.panel.radius, 8.0);
        assert_eq!(t.button.radius, 8.0);
        assert_eq!(t.input.radius, 8.0);
        assert_eq!(t.checkbox.radius, 4.0, "勾选框取全局半径的一半");
        assert_eq!(t.combo.menu_radius, 6.0, "浮层圆角上限 6");
        // 小半径时浮层跟随全局半径
        assert_eq!(Theme::dark().with_radius(3.0).combo.menu_radius, 3.0);
    }

    #[test]
    fn pick_bg_covers_all_three_interaction_states() {
        // 回归：下拉框触发按钮曾写成 `if open { bg_pressed } else { bg }` ⇒ 没有悬停反馈。
        // 三态必须各自可达，且优先级 = 按下 > 悬停 > 常态。
        let s = ButtonStyle {
            bg: Brush::Solid(Color::RED),
            bg_hover: Brush::Solid(Color::GREEN),
            bg_pressed: Brush::Solid(Color::BLUE),
            ..ButtonStyle::default()
        };
        assert_eq!(s.pick_bg(false, false), Color::RED, "常态");
        assert_eq!(s.pick_bg(false, true), Color::GREEN, "**悬停必须可达**");
        assert_eq!(s.pick_bg(true, false), Color::BLUE, "按下");
        assert_eq!(s.pick_bg(true, true), Color::BLUE, "按下优先于悬停");
    }

    #[test]
    fn with_border_w_cascades_and_border_colour_comes_from_the_palette() {
        let t = Theme::dark().with_border_w(3.0);
        assert_eq!(t.panel.border_w, 3.0);
        assert_eq!(t.button.border_w, 3.0);
        assert_eq!(t.input.border_w, 3.0);
        assert_eq!(t.checkbox.border_w, 3.0);
        // 0 = 无边框（`push_panel_like` 会跳过 Border 命令）；负数夹到 0。
        assert_eq!(Theme::dark().with_border_w(0.0).panel.border_w, 0.0);
        assert_eq!(Theme::dark().with_border_w(-5.0).button.border_w, 0.0);
        // 边框**颜色**是调色板令牌：面板取 `border`、按钮/输入框/勾选框取 `border_strong`。
        let p = Palette::dark();
        let t = Theme::themed(&p);
        assert_eq!(t.panel.border, p.border);
        assert_eq!(t.divider.color, p.border);
        assert_eq!(t.button.border, p.border_strong);
        assert_eq!(t.input.border, p.border_strong);
        assert_eq!(t.checkbox.box_border, p.border_strong);
        // 缩放同时作用于宽度（DPI 预乘）。
        assert_eq!(Theme::dark().with_border_w(2.0).scaled(2.0).panel.border_w, 4.0);
    }

    #[test]
    fn focus_and_checkbox_defaults_are_usable() {
        // 1px 焦点描边在高 DPI 下几乎看不见，键盘导航"看不见焦点"是硬伤 ⇒ 取 2。
        assert_eq!(FocusStyle::default().width, 2.0);
        // 勾选框默认直角（保持既有观感），只有显式 with_radius 才变圆
        assert_eq!(CheckboxStyle::default().radius, 0.0);
    }

    #[test]
    fn scaled_preserves_palette_and_scales_checkbox_radius() {
        let t = Theme::dark().with_radius(8.0).scaled(2.0);
        assert_eq!(t.palette, Palette::dark(), "缩放不改变配色来源记录");
        assert_eq!(t.checkbox.radius, 8.0, "半径 ×2（逻辑 4 → 物理 8）");
        assert_eq!(t.combo.menu_radius, 12.0);
    }

    #[test]
    fn bevel_helpers_degrade_to_solid_at_zero() {
        assert_eq!(bevel_raised(Color::RED, 0.0), Brush::Solid(Color::RED));
        assert_eq!(bevel_sunken(Color::RED, 0.0), Brush::Solid(Color::RED));
        assert_eq!(bevel_raised(Color::RED, -1.0), Brush::Solid(Color::RED));
        // 抬升 = 上亮下暗；凹陷 = 上暗下亮
        let r = bevel_raised(Color::rgba_u8(100, 100, 100, 255), 0.1).corners();
        assert!(luma(r[0]) > luma(r[2]));
        let s = bevel_sunken(Color::rgba_u8(100, 100, 100, 255), 0.1).corners();
        assert!(luma(s[0]) < luma(s[2]));
    }

    #[test]
    fn substyle_builder_last_link_wins() {
        // 责任链语义：后设覆盖先设。
        let b = ButtonStyle::default().with_radius(6.0).with_radius(0.0);
        assert_eq!(b.radius, 0.0);
        let p = PanelStyle::default().with_bg(Color::RED).with_bg(Color::BLUE);
        assert_eq!(p.bg, Color::BLUE);
    }

    #[test]
    fn theme_scaled_premultiplies_dimensions() {
        // 全部尺寸 / 字号字段 × scale 并取整（布局整数不变量）；颜色 / 字体族不变。
        let t = Theme::default().scaled(1.5);
        assert_eq!(t.label.font_size, (14.0_f32 * 1.5).round());
        assert_eq!(t.button.padding.x, (12.0_f32 * 1.5).round());
        assert_eq!(t.button.font_size, (14.0_f32 * 1.5).round());
        assert_eq!(t.input.height, (26.0_f32 * 1.5).round());
        assert_eq!(t.gap, (6.0_f32 * 1.5).round());
        assert_eq!(t.panel.bg, Theme::default().panel.bg, "颜色不受预乘影响");
        assert_eq!(t.label.font_family, Theme::default().label.font_family, "字体族不受预乘影响");
        // 先 with_* 再预乘：覆盖值 ×scale（`scale_factor` 只存值、`build` 统一预乘，
        // 故"先 with_*、后 scale_factor"的顺序无关）。
        let a = Theme::default().with_radius(6.0).scaled(1.5);
        assert_eq!(a.panel.radius, 9.0);
        assert_eq!(a.button.radius, 9.0);
        // s <= 0 → 不缩放。
        assert_eq!(Theme::default().scaled(0.0).label.font_size, Theme::default().label.font_size);
        // **行距是倍率、不是尺寸**：预乘 DPI 不该改它（1.2 × 1.5 = 1.8 会把多行
        // 排版拉稀）。行高最终仍随字号变高，因为行高 = 字号 × 行距。
        assert_eq!(
            Theme::default().with_line_spacing(1.2).scaled(1.5).line_spacing,
            1.2,
            "行距倍率不随 DPI 预乘"
        );
    }

    #[test]
    fn density_cozy_is_the_identity() {
        // 默认档 = 今天的观感：间距 / 字号 / 行距全部原样（既有的 sim 坐标、
        // 截图对比因此不受主题扩展影响）。
        let base = Theme::themed(&Palette::light());
        let cozy = base.clone().density(Density::Cozy);
        assert_eq!(cozy.gap, base.gap);
        assert_eq!(cozy.row_h, base.row_h);
        assert_eq!(cozy.panel.padding, base.panel.padding);
        assert_eq!(cozy.button.padding, base.button.padding);
        assert_eq!(cozy.input.height, base.input.height);
        assert_eq!(cozy.label.font_size, base.label.font_size);
        assert_eq!(cozy.button.font_size, base.button.font_size);
        assert_eq!(cozy.combo.font_size, base.combo.font_size);
        assert_eq!(cozy.line_spacing, DEFAULT_LINE_SPACING);
        // 默认 Density 就是 Cozy（枚举 Default derive）。
        assert_eq!(Density::default(), Density::Cozy);
    }

    #[test]
    fn density_scales_spacing_font_and_line_height_monotonically() {
        let base = Theme::themed(&Palette::light());
        let (c, cozy, s) = (
            base.clone().density(Density::Compact),
            base.clone().density(Density::Cozy),
            base.clone().density(Density::Spacious),
        );
        // 单调：紧凑 < 标准 < 宽松（间距 / 字号 / 行距三个维度都要满足，
        // 否则"更紧凑"只是变窄却没变矮，或反之）。
        for (lo, mid, hi) in [
            (c.gap, cozy.gap, s.gap),
            (c.row_h, cozy.row_h, s.row_h),
            (c.panel.padding, cozy.panel.padding, s.panel.padding),
            (c.input.height, cozy.input.height, s.input.height),
            (c.combo.item_min_w, cozy.combo.item_min_w, s.combo.item_min_w),
            (c.label.font_size, cozy.label.font_size, s.label.font_size),
            (c.input.font_size, cozy.input.font_size, s.input.font_size),
            (c.line_spacing, cozy.line_spacing, s.line_spacing),
        ] {
            assert!(lo < mid, "紧凑应小于标准：{lo} !< {mid}");
            assert!(mid < hi, "标准应小于宽松：{mid} !< {hi}");
        }
        // 具体倍率：间距 ×0.84、字号 ×0.92（取整）。
        assert_eq!(c.gap, (base.gap * 0.84).round().max(1.0));
        assert_eq!(c.label.font_size, (base.label.font_size * 0.92).round().max(1.0));
        assert_eq!(s.line_spacing, 1.30);
    }

    #[test]
    fn theme_shadow_builders_replace_and_disable() {
        // 预设自带投影（浅色 alpha 48 / 深色 120），且只挂在 panel 上。
        let light = Theme::light();
        assert!(light.panel.shadow.is_visible());
        assert_eq!(light.panel.shadow.color, Palette::light().shadow);
        assert_eq!(light.panel.shadow.blur, ShadowStyle::default().blur);
        // 整对象替换（blur / offset / color 一起换）。
        let lifted = Theme::light().with_shadow(ShadowStyle {
            blur: 24.0,
            offset: Vec2::new(0.0, 8.0),
            color: Color::rgba_u8(0, 0, 0, 90),
        });
        assert_eq!(lifted.panel.shadow.blur, 24.0);
        assert_eq!(lifted.panel.shadow.offset, Vec2::new(0.0, 8.0));
        // without_shadow = blur 0 = 不画（不产生几何），其余字段原样保留。
        let flat = Theme::light().without_shadow();
        assert_eq!(flat.panel.shadow.blur, 0.0);
        assert!(!flat.panel.shadow.is_visible());
        assert_eq!(flat.panel.shadow.color, light.panel.shadow.color);
        // 只碰 panel：其它子样式本来就没有 shadow 字段（编译期保证），
        // 这里断言"没顺手改坏 padding / 圆角"。
        assert_eq!(flat.panel.padding, light.panel.padding);
        assert_eq!(flat.panel.radius, light.panel.radius);
    }

    #[test]
    fn grip_style_extends_scales_and_hides() {
        // 缩放柄：颜色走调色板、尺寸随 DPI 预乘、`Hidden` 不画但**仍可拖**
        // （命中区在 `window_impl` 里另有 14px 下限 ⇒ 数据层只负责"画不画"与"多大"）。
        let p = Palette::dark();
        let panel = PanelStyle::themed(&p);
        assert_eq!(panel.grip.color, p.border, "柄色取自调色板边框色");
        assert_eq!(
            panel.grip.shape,
            GripShape::Diagonal,
            "默认形状 = GripShape::default()（三条斜线）"
        );
        assert!(panel.grip.is_visible());
        // 图案占位：从右下角往左上的方框边长（命中区下限的来源）——按形状各有公式。
        let sq = panel.clone().with_grip_shape(GripShape::Squares).grip;
        assert_eq!(sq.extent(), sq.step * sq.count as f32 + sq.size);
        let dia = panel.grip;
        assert_eq!(
            dia.extent(),
            dia.size * dia.count as f32 * 1.5 + dia.step,
            "斜线版的方框是横线版的 1.5×"
        );
        let bars = panel.clone().with_grip_shape(GripShape::Bars);
        assert_eq!(bars.grip.extent(), bars.grip.size * bars.grip.count as f32 + bars.grip.step);
        // 预乘 DPI：尺寸类字段 ×s，形状 / 颜色 / 个数不变。
        let scaled = panel.grip.scaled(2.0);
        assert_eq!(scaled.size, (panel.grip.size * 2.0).round());
        assert_eq!(scaled.step, (panel.grip.step * 2.0).round());
        assert_eq!(scaled.count, panel.grip.count);
        assert_eq!(scaled.color, panel.grip.color);
        // Hidden：不画（`is_visible` 为假）、`extent` 归零；全透明色同理。
        let hidden = panel.clone().without_grip();
        assert_eq!(hidden.grip.shape, GripShape::Hidden);
        assert!(!hidden.grip.is_visible());
        assert_eq!(hidden.grip.extent(), 0.0);
        assert!(!panel.clone().with_grip_color(Color::rgba_u8(0, 0, 0, 0)).grip.is_visible());
        // `scaled` 也走 Theme → PanelStyle 这条链（避免只测了子样式、漏了主题预乘）。
        assert_eq!(Theme::dark().scaled(1.5).panel.grip.size, (4.0_f32 * 1.5).round());
    }

    #[test]
    fn input_grip_is_themed_and_defaults_to_diagonal() {
        // 文本框的缩放柄过去写死在引擎里（3 个斜向小方块、无视主题）；现在走
        // `InputStyle::grip`（与窗口柄同一套 `GripShape`），默认 = 三条斜线。
        let p = Palette::dark();
        let input = InputStyle::themed(&p);
        assert_eq!(input.grip.shape, GripShape::Diagonal, "默认三条斜线");
        assert_eq!(input.grip.color, p.text_dim, "柄色取自调色板");
        assert_eq!(input.grip.color, input.resize_handle, "与 resize_handle 同色");
        assert!(input.grip.is_visible());
        // setter：换形状 / 换整份样式；`Hidden` 只是不画图案（命中区由引擎另给）。
        assert_eq!(
            input.clone().with_grip_shape(GripShape::Bars).grip.shape,
            GripShape::Bars
        );
        assert_eq!(
            input.clone().with_grip_shape(GripShape::Squares).grip.shape,
            GripShape::Squares
        );
        assert!(!input.clone().with_grip_shape(GripShape::Hidden).grip.is_visible());
        assert_eq!(
            input.clone().with_grip(GripStyle { count: 2, ..GripStyle::default() }).grip.count,
            2
        );
        // `Theme::scaled` 必须把这把柄一起预乘（漏了就是"1.5× 下柄比别的小"）。
        assert_eq!(
            Theme::dark().scaled(1.5).input.grip.size,
            (4.0_f32 * 1.5).round()
        );
    }

    #[test]
    fn density_leaves_radius_border_colour_and_shadow_alone() {
        // 密度只负责"排布疏密"：造型（圆角）、亚像素观感（边框宽 / 羽化）、
        // 颜色、投影都不该被密度改动——那是用户的造型选择。
        let base = Theme::themed(&Palette::dark()).with_radius(7.0);
        let c = base.clone().density(Density::Compact);
        assert_eq!(c.panel.radius, base.panel.radius);
        assert_eq!(c.button.radius, base.button.radius);
        assert_eq!(c.panel.border_w, base.panel.border_w);
        assert_eq!(c.feather, base.feather);
        assert_eq!(c.panel.bg, base.panel.bg);
        assert_eq!(c.panel.shadow.blur, base.panel.shadow.blur);
        // 圆角即便在宽松档也保持 7（间距档不碰圆角）。
        assert_eq!(base.density(Density::Spacious).panel.radius, 7.0);
    }

    #[test]
    fn with_line_spacing_clamps_and_partial_scales_compose() {
        // 单维调节：只改行距时字号 / 间距原样。
        let t = Theme::themed(&Palette::light()).with_line_spacing(1.6);
        assert_eq!(t.line_spacing, 1.6);
        assert_eq!(t.gap, Theme::themed(&Palette::light()).gap);
        // 下界 0.5：行高 < 字号的一半会行盒重叠、光标定位失去意义。
        assert_eq!(Theme::default().with_line_spacing(0.0).line_spacing, 0.5);
        assert_eq!(Theme::default().with_line_spacing(-3.0).line_spacing, 0.5);
        // 非法倍率（<= 0）视为"不改"，不产生 NaN / 0 尺寸。
        let b = Theme::default();
        assert_eq!(b.clone().with_font_scale(0.0).label.font_size, b.label.font_size);
        assert_eq!(b.clone().with_spacing_scale(-1.0).gap, b.gap);
        assert_eq!(b.clone().with_font_scale(-2.0).input.height, b.input.height);
    }

    #[test]
    fn font_weight_is_a_font_choice_not_a_size_token() {
        // 默认 400：**不设字重 = 与扩展前逐像素一致**（既有 sim 坐标 / 截图基线不受影响）。
        assert_eq!(Theme::default().font_weight, Weight::NORMAL);
        assert_eq!(Theme::themed(&Palette::dark()).font_weight, Weight::NORMAL);
        // 显式设置原样保留（任意数值，不吸附到某个档位）。
        assert_eq!(Theme::default().with_font_weight(Weight(550)).font_weight, Weight(550));
        // **不是尺寸量**：DPI 预乘（`scaled`）与密度档（`density`）都不得改它——
        // 它们是"逻辑像素尺寸 / 间距"，字重是"选哪个字面"。写错会让换 DPI 悄悄变字重。
        let t = Theme::default()
            .with_font_weight(Weight::BOLD)
            .density(Density::Spacious)
            .scaled(2.0);
        assert_eq!(t.font_weight, Weight::BOLD);
        // 对照：同一趟链上字号被 ×2 预乘（**尺寸类**），行距只被密度档改、不被 DPI 改
        // （**倍率类**）——字重跟行距一类。
        let base = Theme::default().density(Density::Spacious);
        assert_eq!(t.line_spacing, base.line_spacing, "行距是倍率：DPI 不缩放它");
        assert_eq!(
            t.label.font_size,
            (base.label.font_size * 2.0).round(),
            "字号是尺寸：DPI ×2（取整）"
        );
    }
}

