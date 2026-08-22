//! 主题样式：`Theme` + 各控件子样式（默认 / dark 两套预设，可 clone 覆盖）。

use std::sync::Arc;

use rjw_color::Color;
use rjw_text::Align;

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
    /// **模态对话框样式**（[`Ui::modal_at`](crate::ui::Ui::modal_at) 遮罩）。
    pub modal: ModalStyle,
    /// **下拉框（combo）样式**：触发按钮用 [`ButtonStyle`]；选项浮层 = 现代右键菜单外观。
    pub combo: ComboStyle,
    /// **单行控件统一高度**（逻辑像素）：水平行容器（`p.row(...)`）内所有子项强制
    /// 等高——Label/Button/输入框各自内容垂直居中 → 文字中心线对齐（近似基线）。
    pub row_h: f32,
    /// pack / grid 默认子项间距（像素）。
    pub gap: f32,
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
    pub menu_radius: f32,
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
            menu_radius: 6.0,
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
        self.radius = m(self.radius);
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
        self.radius = m(self.radius);
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
        self.radius = m(self.radius);
        self.height = m(self.height);
        self.min_w = m(self.min_w);
        self.font_size = m(self.font_size);
        self
    }
}

impl CheckboxStyle {
    /// 预乘 DPI scale：方框 / 边框宽 / 字号 / 间距 × s 取整。
    pub fn scaled(mut self, s: f32) -> Self {
        if s <= 0.0 {
            return self;
        }
        let m = |v: f32| (v * s).round();
        self.box_size = m(self.box_size);
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
        self.menu_radius = m(self.menu_radius);
        self.menu_pad_v = m(self.menu_pad_v);
        self.item_pad_x = m(self.item_pad_x);
        self.item_min_w = m(self.item_min_w);
        self.font_size = m(self.font_size);
        self
    }
}

// ─── 子样式深色预设：每个子样式都有 `dark()`（深色配色，尺寸同 [`Default`]）。
// `Theme::dark()` 逐一组装——与 `scaled` 同样的单一职责。 ───

impl LabelStyle {
    /// 深色主题预设：浅色文字。
    pub fn dark() -> Self {
        Self { color: Color::rgba_u8(225, 225, 225, 255), ..Self::default() }
    }
}

impl PanelStyle {
    /// 深色主题预设：深灰面板 + 边框。
    pub fn dark() -> Self {
        Self {
            bg: Color::rgba_u8(38, 42, 52, 255),
            border: Color::rgba_u8(70, 78, 96, 255),
            ..Self::default()
        }
    }
}

impl ButtonStyle {
    /// 深色主题预设：深色三态背景。
    pub fn dark() -> Self {
        Self {
            bg: Color::rgba_u8(52, 58, 70, 255),
            bg_hover: Color::rgba_u8(66, 76, 96, 255),
            bg_pressed: Color::rgba_u8(90, 110, 150, 255),
            fg: Color::rgba_u8(230, 230, 230, 255),
            border: Color::rgba_u8(90, 98, 118, 255),
            ..Self::default()
        }
    }
}

impl SliderStyle {
    /// 深色主题预设：深色轨道 + 亮填充 / 手柄。
    pub fn dark() -> Self {
        Self {
            track: Color::rgba_u8(58, 64, 78, 255),
            fill: Color::rgba_u8(96, 150, 220, 255),
            handle: Color::rgba_u8(200, 210, 225, 255),
            handle_border: Color::rgba_u8(120, 132, 150, 255),
            ..Self::default()
        }
    }
}

impl InputStyle {
    /// 深色主题预设：深色输入框 + 亮边框 / 光标 / 选择。
    pub fn dark() -> Self {
        Self {
            bg: Color::rgba_u8(28, 32, 40, 255),
            border: Color::rgba_u8(80, 88, 104, 255),
            border_focus: Color::rgba_u8(110, 160, 230, 255),
            fg: Color::rgba_u8(230, 230, 230, 255),
            caret: Color::rgba_u8(230, 230, 230, 255),
            preedit: Color::rgba_u8(150, 158, 176, 255),
            sel_bg: Color::rgba_u8(70, 120, 190, 255),
            ..Self::default()
        }
    }
}

impl CheckboxStyle {
    /// 深色主题预设：深色方框 + 亮填充 / 文字。
    pub fn dark() -> Self {
        Self {
            box_border: Color::rgba_u8(150, 158, 176, 255),
            checked_fill: Color::rgba_u8(96, 150, 220, 255),
            fg: Color::rgba_u8(225, 225, 225, 255),
            ..Self::default()
        }
    }
}

impl DividerStyle {
    /// 深色主题预设：深灰分割线（深色背景上可见）。
    pub fn dark() -> Self {
        Self { color: Color::rgba_u8(70, 78, 96, 255), ..Self::default() }
    }
}

impl DebugStyle {
    /// 深色主题预设：亮青布局描边。
    pub fn dark() -> Self {
        Self { layout_outline: Color::rgba_u8(96, 200, 255, 255), ..Self::default() }
    }
}

impl FocusStyle {
    /// 深色主题预设：亮蓝焦点描边（深色下更易辨认）。
    pub fn dark() -> Self {
        Self { color: Color::rgba_u8(96, 200, 255, 255), ..Self::default() }
    }
}

impl ModalStyle {
    /// 深色主题预设：更深遮罩。
    pub fn dark() -> Self {
        Self { dim: Color::rgba_u8(0, 0, 0, 180), ..Self::default() }
    }
}

impl ComboStyle {
    /// 深色主题预设：深色浮层 + 深蓝菜单项高亮。
    pub fn dark() -> Self {
        Self {
            menu_bg: Color::rgba_u8(40, 45, 55, 255),
            menu_border: Color::rgba_u8(72, 80, 96, 255),
            item_hover: Color::rgba_u8(52, 82, 122, 255),
            item_selected: Color::rgba_u8(45, 72, 108, 255),
            fg: Color::rgba_u8(228, 228, 228, 255),
            fg_mark: Color::rgba_u8(110, 180, 255, 255),
            ..Self::default()
        }
    }
}

/// 模态对话框样式（`modal_at` 的全屏遮罩）。
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
    pub bg: Color,
    pub border: Color,
    pub border_w: f32,
    /// 内容区内边距（像素）。
    pub padding: f32,
    /// 圆角半径（**逻辑像素**；0 = 直角）。背景与边框都按此半径 9-patch 绘制。
    pub radius: f32,
}

impl Default for PanelStyle {
    fn default() -> Self {
        Self {
            bg: Color::rgba_u8(245, 245, 245, 255),
            border: Color::rgba_u8(180, 180, 180, 255),
            border_w: 1.0,
            padding: 8.0,
            radius: 0.0,
        }
    }
}

/// 按钮样式（normal / hover / pressed 三态）。
#[derive(Clone, Debug)]
pub struct ButtonStyle {
    pub bg: Color,
    pub bg_hover: Color,
    pub bg_pressed: Color,
    pub fg: Color,
    pub border: Color,
    pub border_w: f32,
    /// 圆角半径（**逻辑像素**；0 = 直角）。
    pub radius: f32,
    /// 内边距（x = 水平，y = 垂直）。
    pub padding: glam::Vec2,
    pub font_size: f32,
    pub font_family: Option<Arc<str>>,
}

impl Default for ButtonStyle {
    fn default() -> Self {
        Self {
            bg: Color::rgba_u8(225, 225, 225, 255),
            bg_hover: Color::rgba_u8(205, 225, 250, 255),
            bg_pressed: Color::rgba_u8(170, 200, 235, 255),
            fg: Color::rgba_u8(30, 30, 30, 255),
            border: Color::rgba_u8(150, 150, 150, 255),
            border_w: 1.0,
            radius: 0.0,
            padding: glam::Vec2::new(12.0, 6.0),
            font_size: 14.0,
            font_family: None,
        }
    }
}

/// 滑块样式。
#[derive(Clone, Debug)]
pub struct SliderStyle {
    pub track: Color,
    pub fill: Color,
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
}

impl Default for SliderStyle {
    fn default() -> Self {
        Self {
            track: Color::rgba_u8(190, 190, 190, 255),
            fill: Color::rgba_u8(80, 140, 220, 255),
            handle: Color::rgba_u8(240, 240, 240, 255),
            handle_border: Color::rgba_u8(120, 120, 120, 255),
            track_h: 6.0,
            handle_w: 12.0,
            height: 20.0,
            min_w: 120.0,
        }
    }
}

/// 文本输入框样式。
#[derive(Clone, Debug)]
pub struct InputStyle {
    pub bg: Color,
    pub border: Color,
    pub border_focus: Color,
    pub fg: Color,
    pub caret: Color,
    /// IME 组合候选串颜色（如拼音未上屏时的灰色候选）。
    pub preedit: Color,
    /// **文本选择高亮**（背景色；选中文本拖拽区域）。
    pub sel_bg: Color,
    pub border_w: f32,
    /// 内容水平内边距。
    pub padding_x: f32,
    /// 圆角半径（**逻辑像素**；0 = 直角）。
    pub radius: f32,
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
            bg: Color::rgba_u8(255, 255, 255, 255),
            border: Color::rgba_u8(160, 160, 160, 255),
            border_focus: Color::rgba_u8(80, 140, 220, 255),
            fg: Color::rgba_u8(30, 30, 30, 255),
            caret: Color::rgba_u8(30, 30, 30, 255),
            preedit: Color::rgba_u8(120, 120, 120, 255),
            sel_bg: Color::rgba_u8(140, 190, 245, 255),
            border_w: 1.0,
            padding_x: 6.0,
            radius: 0.0,
            height: 26.0,
            min_w: 140.0,
            font_size: 14.0,
            font_family: None,
        }
    }
}

/// 分割线样式（[`Ui::divider_at`](crate::ui::Ui::divider_at) / [`crate::widget::Divider`]）。
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
/// 宽度为**逻辑像素**（内部 × scale 后取整）；默认青色 1.0，`Theme::dark` 下偏亮。
#[derive(Clone, Debug)]
pub struct FocusStyle {
    pub color: Color,
    pub width: f32,
}

impl Default for FocusStyle {
    fn default() -> Self {
        Self { color: Color::CYAN, width: 1.0 }
    }
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
    pub fn with_bg(mut self, c: Color) -> Self {
        self.bg = c;
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
    pub fn with_radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
}

impl ButtonStyle {
    /// 常态背景。
    pub fn with_bg(mut self, c: Color) -> Self {
        self.bg = c;
        self
    }
    /// 悬停背景。
    pub fn with_bg_hover(mut self, c: Color) -> Self {
        self.bg_hover = c;
        self
    }
    /// 按下背景。
    pub fn with_bg_pressed(mut self, c: Color) -> Self {
        self.bg_pressed = c;
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
    pub fn with_radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    /// 内边距（x = 水平，y = 垂直）。
    pub fn with_padding(mut self, p: glam::Vec2) -> Self {
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
    /// 轨道颜色。
    pub fn with_track(mut self, c: Color) -> Self {
        self.track = c;
        self
    }
    /// 已填充部分颜色。
    pub fn with_fill(mut self, c: Color) -> Self {
        self.fill = c;
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
}

impl InputStyle {
    pub fn with_bg(mut self, c: Color) -> Self {
        self.bg = c;
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
    pub fn with_radius(mut self, r: f32) -> Self {
        self.radius = r;
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
    pub fn with_size(mut self, s: glam::Vec2) -> Self {
        self.size = Some(s);
        self
    }
    /// 全屏遮罩（默认）。
    pub fn with_fullscreen(mut self) -> Self {
        self.size = None;
        self
    }
}

impl Theme {
    /// 浅色主题（默认）。
    pub fn default() -> Self {
        Self {
            label: LabelStyle::default(),
            panel: PanelStyle::default(),
            button: ButtonStyle::default(),
            slider: SliderStyle::default(),
            input: InputStyle::default(),
            checkbox: CheckboxStyle::default(),
            divider: DividerStyle::default(),
            debug: DebugStyle::default(),
            focus: FocusStyle::default(),
            modal: ModalStyle::default(),
            combo: ComboStyle::default(),
            row_h: 26.0,
            gap: 6.0,
        }
    }

    /// 深色主题：逐一组装每个子样式的 [`dark()`](LabelStyle::dark) 预设
    /// （深色配色、尺寸同 `Default`）+ 主题级 `row_h` / `gap`。
    pub fn dark() -> Self {
        Self {
            label: LabelStyle::dark(),
            panel: PanelStyle::dark(),
            button: ButtonStyle::dark(),
            slider: SliderStyle::dark(),
            input: InputStyle::dark(),
            checkbox: CheckboxStyle::dark(),
            divider: DividerStyle::dark(),
            debug: DebugStyle::dark(),
            focus: FocusStyle::dark(),
            modal: ModalStyle::dark(),
            combo: ComboStyle::dark(),
            row_h: 26.0,
            gap: 6.0,
        }
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

    /// **圆角半径**：级联到 `panel` / `button` / `input`（逻辑像素；0 = 直角）。
    pub fn with_radius(mut self, radius: f32) -> Self {
        self.panel.radius = radius;
        self.button.radius = radius;
        self.input.radius = radius;
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
        button.bg = Color::RED;
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
