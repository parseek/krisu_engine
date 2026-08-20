//! 主题样式：`Theme` + 各控件子样式（默认 / dark 两套预设，可 clone 覆盖）。

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
    /// **单行控件统一高度**（逻辑像素）：水平行容器（`p.row(...)`）内所有子项强制
    /// 等高——Label/Button/输入框各自内容垂直居中 → 文字中心线对齐（近似基线）。
    pub row_h: f32,
    /// pack / grid 默认子项间距（像素）。
    pub gap: f32,
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
    pub font_family: Option<String>,
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
    pub font_family: Option<String>,
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
    pub font_family: Option<String>,
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
    pub font_family: Option<String>,
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
            row_h: 26.0,
            gap: 6.0,
        }
    }

    /// 深色主题。
    pub fn dark() -> Self {
        let mut t = Self::default();
        t.label.color = Color::rgba_u8(225, 225, 225, 255);
        t.label.font_size = 14.0;
        t.panel.bg = Color::rgba_u8(38, 42, 52, 255);
        t.panel.border = Color::rgba_u8(70, 78, 96, 255);
        t.button.bg = Color::rgba_u8(52, 58, 70, 255);
        t.button.bg_hover = Color::rgba_u8(66, 76, 96, 255);
        t.button.bg_pressed = Color::rgba_u8(90, 110, 150, 255);
        t.button.fg = Color::rgba_u8(230, 230, 230, 255);
        t.button.border = Color::rgba_u8(90, 98, 118, 255);
        t.slider.track = Color::rgba_u8(58, 64, 78, 255);
        t.slider.fill = Color::rgba_u8(96, 150, 220, 255);
        t.slider.handle = Color::rgba_u8(200, 210, 225, 255);
        t.slider.handle_border = Color::rgba_u8(120, 132, 150, 255);
        t.input.bg = Color::rgba_u8(28, 32, 40, 255);
        t.input.border = Color::rgba_u8(80, 88, 104, 255);
        t.input.border_focus = Color::rgba_u8(110, 160, 230, 255);
        t.input.fg = Color::rgba_u8(230, 230, 230, 255);
        t.input.caret = Color::rgba_u8(230, 230, 230, 255);
        t.input.preedit = Color::rgba_u8(150, 158, 176, 255);
        t.input.sel_bg = Color::rgba_u8(70, 120, 190, 255);
        t.checkbox.box_border = Color::rgba_u8(150, 158, 176, 255);
        t.checkbox.checked_fill = Color::rgba_u8(96, 150, 220, 255);
        t.checkbox.fg = Color::rgba_u8(225, 225, 225, 255);
        // 焦点描边：深色主题下更亮，便于看清焦点位置
        t.focus.color = Color::rgba_u8(96, 200, 255, 255);
        t
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
    pub fn with_font_family(mut self, family: impl Into<String>) -> Self {
        let f = Some(family.into());
        self.label.font_family = f.clone();
        self.button.font_family = f.clone();
        self.checkbox.font_family = f.clone();
        self.input.font_family = f;
        self
    }

    /// **全局字号**：级联到全部文本子样式（`label` / `button` / `checkbox` / `input`）。
    pub fn with_font_size(mut self, size: f32) -> Self {
        self.label.font_size = size;
        self.button.font_size = size;
        self.checkbox.font_size = size;
        self.input.font_size = size;
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
}
