//! 主题样式：`Theme` + 各控件子样式（默认 / dark 两套预设，可 clone 覆盖）。

use std::sync::Arc;

use rjw_color::Color;
use rjw_text::Align;

use crate::draw::CornerRadius;

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
pub struct Theme {
    pub label: LabelStyle,
    pub panel: PanelStyle,
    pub button: ButtonStyle,
    pub slider: SliderStyle,
    pub input: InputStyle,
    pub checkbox: CheckboxStyle,
    /// **分割线样式**（[`Ui::divider_at`](crate::ui::Ui::divider_at) / 容器 `divider()`）。
    pub divider: DividerStyle,
    /// 调试样式（debug_layout 描边等；DebugDraw 图元的样式 = 每次调用显式传参）。
    pub debug: DebugStyle,
    /// **焦点样式**（键盘导航）：当前焦点控件的描边（`finish` 绘制）。
    pub focus: FocusStyle,
    /// **模态对话框样式**（[`Ui::modal`](crate::ui::Ui::modal) 遮罩）。
    pub modal: ModalStyle,
    /// **下拉框（combo）样式**：触发按钮用 [`ButtonStyle`]；选项浮层 = 现代右键菜单外观。
    pub combo: ComboStyle,
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
            item_pad_x: 12.0,
            item_min_w: 140.0,
            fg: Color::rgba_u8(40, 40, 40, 255),
            fg_mark: Color::rgba_u8(30, 108, 198, 255),
            font_size: 14.0,
            font_family: None,
        }
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
    /// 预乘 DPI scale：边框宽 / 内边距 / 圆角 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).round();
        self.border_w = m(self.border_w);
        self.padding = m(self.padding);
        self.radius = self.radius.scaled_rounded(s);
        self
    }
}

impl ButtonStyle {
    /// 预乘 DPI scale：边框宽 / 圆角 / 内边距 / 字号 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).round();
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
        let m = |v: f32| (v * s).round();
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
        let m = |v: f32| (v * s).round();
        self.border_w = m(self.border_w);
        self.padding_x = m(self.padding_x);
        self.radius = self.radius.scaled_rounded(s);
        self.height = m(self.height);
        self.min_w = m(self.min_w);
        self.font_size = m(self.font_size);
        self
    }
}

impl CheckboxStyle {
    /// 预乘 DPI scale：方框 / 圆角 / 边框宽 / 字号 / 间距 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).round();
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
        let m = |v: f32| (v * s).round();
        self.thickness = m(self.thickness);
        self.margin = m(self.margin);
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
        let m = |v: f32| (v * s).round();
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
            surface_dim: Color::rgba_u8(14, 15, 19, 255),
            surface_sunken: Color::rgba_u8(16, 18, 24, 255),
            surface: Color::rgba_u8(23, 25, 31, 255),
            surface_raised: Color::rgba_u8(34, 37, 45, 255),
            surface_overlay: Color::rgba_u8(43, 47, 57, 255),
            surface_hover: Color::rgba_u8(51, 56, 69, 255),
            surface_active: Color::rgba_u8(61, 68, 83, 255),
            border: Color::rgba_u8(58, 63, 75, 255),
            border_strong: Color::rgba_u8(74, 81, 98, 255),
            text: Color::rgba_u8(232, 234, 240, 255),
            text_muted: Color::rgba_u8(154, 163, 178, 255),
            text_dim: Color::rgba_u8(107, 114, 128, 255),
            accent: Color::rgba_u8(110, 168, 255, 255),
            accent_hover: Color::rgba_u8(140, 188, 255, 255),
            accent_active: Color::rgba_u8(85, 140, 219, 255),
            selection: Color::rgba_u8(43, 74, 120, 255),
            danger: Color::rgba_u8(255, 107, 107, 255),
            handle: Color::rgba_u8(200, 208, 220, 255),
            debug_outline: Color::rgba_u8(96, 200, 255, 255),
            scrim: Color::rgba_u8(0, 0, 0, 180),
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
            bevel: 0.0,
        }
    }
}

// ─── 子样式配色预设：`themed(&Palette)` 从调色板派生（尺寸同 [`Default`]）。
// `Theme::{light,dark,themed}` 逐一组装——与 `scaled` 同样的单一职责。 ───

impl LabelStyle {
    /// 从调色板派生：正文色的标签。
    pub fn themed(p: &Palette) -> Self {
        Self { color: p.text, ..Self::default() }
    }
}

impl PanelStyle {
    /// 从调色板派生：`surface` 面板 + 常规描边（背景带调色板强度的微渐变）。
    pub fn themed(p: &Palette) -> Self {
        Self {
            bg: bevel_raised(p.surface, p.bevel),
            border: p.border,
            ..Self::default()
        }
    }
}

impl ButtonStyle {
    /// 从调色板派生：`surface_raised` 三态按钮（常态 → 悬停 → 激活逐级抬升）。
    pub fn themed(p: &Palette) -> Self {
        Self {
            bg: bevel_raised(p.surface_raised, p.bevel),
            bg_hover: bevel_raised(p.surface_hover, p.bevel),
            bg_pressed: bevel_raised(p.surface_active, p.bevel),
            fg: p.text,
            border: p.border_strong,
            ..Self::default()
        }
    }
}

impl SliderStyle {
    /// 从调色板派生：`surface_sunken` 轨道 + 强调色填充 + `handle` 手柄。
    pub fn themed(p: &Palette) -> Self {
        Self {
            track: p.surface_sunken.into(),
            fill: p.accent.into(),
            handle: p.handle,
            handle_border: p.border_strong,
            ..Self::default()
        }
    }
}

impl InputStyle {
    /// 从调色板派生：`surface_sunken` 输入框（反向微渐变 = 凹陷感）+ 强调色聚焦边框。
    pub fn themed(p: &Palette) -> Self {
        Self {
            bg: bevel_sunken(p.surface_sunken, p.bevel),
            border: p.border_strong,
            border_focus: p.accent,
            fg: p.text,
            caret: p.text,
            preedit: p.text_muted,
            sel_bg: p.selection,
            resize_handle: p.text_dim,
            ..Self::default()
        }
    }
}

impl CheckboxStyle {
    /// 从调色板派生：强描边方框 + 强调色勾选填充。
    pub fn themed(p: &Palette) -> Self {
        Self {
            box_border: p.border_strong,
            checked_fill: p.accent,
            fg: p.text,
            ..Self::default()
        }
    }
}

impl DividerStyle {
    /// 从调色板派生：常规描边色的分割线。
    pub fn themed(p: &Palette) -> Self {
        Self { color: p.border, ..Self::default() }
    }
}

impl DebugStyle {
    /// 从调色板派生：调试描边色。
    pub fn themed(p: &Palette) -> Self {
        Self { layout_outline: p.debug_outline, ..Self::default() }
    }
}

impl FocusStyle {
    /// 从调色板派生：强调色焦点描边（宽度取 `DEFAULT_WIDTH`）。
    pub fn themed(p: &Palette) -> Self {
        Self { color: p.accent, ..Self::default() }
    }
}

impl ModalStyle {
    /// 从调色板派生：遮罩色。
    pub fn themed(p: &Palette) -> Self {
        Self { dim: p.scrim, ..Self::default() }
    }
}

impl ComboStyle {
    /// 从调色板派生：`surface_overlay` 浮层 + 悬停 / 选中态高亮 + 强调色 ✓ 标记。
    pub fn themed(p: &Palette) -> Self {
        Self {
            menu_bg: p.surface_overlay,
            menu_border: p.border_strong,
            item_hover: p.surface_hover,
            item_selected: p.selection,
            fg: p.text,
            fg_mark: p.accent,
            ..Self::default()
        }
    }
}


/// 模态对话框样式（[`Ui::modal`](crate::ui::Ui::modal) 的全屏遮罩）。
#[derive(Clone, Debug)]
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
pub struct LabelStyle {
    /// 字体族（`None` = 系统默认；空串同默认）。
    pub font_family: Option<Arc<str>>,
    pub font_size: f32,
    pub color: Color,
    /// 水平对齐（垂直恒居中）。
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

/// 面板（背景 + 边框）样式。
#[derive(Clone, Debug)]
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
    pub bg_image: Option<crate::draw::ImageBg>,
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
        }
    }
}

/// 按钮样式（normal / hover / pressed 三态）。
#[derive(Clone, Debug)]
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
    pub border_w: f32,
    /// 内容水平内边距。
    pub padding_x: f32,
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
            border_w: 1.0,
            padding_x: 6.0,
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

/// 勾选框 / 单选样式。
#[derive(Clone, Debug)]
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
            debug: DebugStyle::themed(p),
            focus: FocusStyle::themed(p),
            modal: ModalStyle::themed(p),
            combo: ComboStyle::themed(p),
            row_h: 26.0,
            gap: 6.0,
            feather: crate::tess::DEFAULT_FEATHER,
            palette: *p,
        }
    }

    /// **边缘羽化宽度**（逻辑像素；0 = 硬边）。见 [`Theme::feather`] 字段文档。
    pub fn with_feather(mut self, px: f32) -> Self {
        self.feather = px.max(0.0);
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
    /// `input`——滑块无文本、面板无字体）。`None` = 系统默认。
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
        self.combo.font_family = f;
        self
    }

    /// **全局字号**：级联到全部文本子样式（`label` / `button` / `checkbox` / `input` /
    /// `combo`）。
    pub fn with_font_size(mut self, size: f32) -> Self {
        self.label.font_size = size;
        self.button.font_size = size;
        self.checkbox.font_size = size;
        self.input.font_size = size;
        self.combo.font_size = size;
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
        self
    }

    /// **边框宽度**（逻辑像素；0 = 不画边框）。
    ///
    /// 级联到全部**有边框**的子样式：`panel` / `button` / `input` / `checkbox`。
    /// 边框**颜色**不是主题标量而是调色板令牌（[`Palette::border`] 常规 /
    /// [`Palette::border_strong`] 强描边）——换色请改调色板，这样"面板 / 按钮 /
    /// 输入框"的描边深浅关系不会各自漂移。
    pub fn with_border_w(mut self, w: f32) -> Self {
        let w = w.max(0.0);
        self.panel.border_w = w;
        self.button.border_w = w;
        self.input.border_w = w;
        self.checkbox.border_w = w;
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
        let m = |v: f32| (v * s).round();
        self.label = self.label.scaled(s);
        self.panel = self.panel.scaled(s);
        self.button = self.button.scaled(s);
        self.slider = self.slider.scaled(s);
        self.input = self.input.scaled(s);
        self.checkbox = self.checkbox.scaled(s);
        self.divider = self.divider.scaled(s);
        self.debug = self.debug.scaled(s);
        self.focus = self.focus.scaled(s);
        self.modal = self.modal.scaled(s);
        self.combo = self.combo.scaled(s);
        self.gap = m(self.gap);
        self.row_h = m(self.row_h);
        // 羽化宽也按 DPI 物理化；但不取整（亚像素级软边，取整会让 0.5 逻辑像素消失）。
        self.feather *= s;
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
        assert_eq!(t.label.font_size, 16.0);
        assert_eq!(t.button.font_size, 16.0);
        assert_eq!(t.checkbox.font_size, 16.0);
        assert_eq!(t.input.font_size, 16.0);
        // 无文本子样式不受影响
        assert_eq!(t.slider.track, Theme::dark().slider.track);
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
    }
}

