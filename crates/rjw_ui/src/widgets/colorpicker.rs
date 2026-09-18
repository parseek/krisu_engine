//! **颜色选择器**（组合控件，只依赖公开 API）。
//!
//! # 形态：内联色块 + 点击弹出的取色面板
//!
//! 内联部分只占**一行**（色块 + 当前颜色的十六进制 + 右侧 `⌄`），点一下才在控件
//! **正下方**弹出取色面板。这样：
//!
//! - 一行里能并排放好几个取色器，不会像"内联铺开一堆滑条"那样把所在窗口撑高、
//!   并把后续行挤到一起（控件的 `size()` 与实际绘制内容一旦不一致，布局就会重叠）；
//! - 面板是**独立置顶窗口**（`z = WIN_TOPMOST`，与下拉框浮层同一机制），不受父容器
//!   裁剪 / 换行约束，也不会撑大父级。
//!
//! 点击面板以外（且不在色块上）收起；`Esc` 收起——但**面板内有文本框持焦点时
//! 只失焦、不收起**（否则"编辑到一半连面板一起消失"）。
//!
//! # 模块划分（刻意不写成单个大文件）
//!
//! | 文件 | 职责 |
//! |---|---|
//! | [`format`] | 文本格式（u8 / HEX / F）的呈现与**自动识别**解析（纯函数，可单测） |
//! | [`hsv`] | RGB↔HSV、SV 平面 / 色相条的几何与取参（纯函数，可单测） |
//! | [`state`] | **全局跨帧数据** [`ColorPickerState`]（所有实例共用一份） |
//! | [`panel`] | 弹出面板：布局、绘制、交互 |
//! | 本文件 | 内联色块 + 开关 + 面板窗口的挂载（`Widget` 实现） |
//!
//! # 跨帧数据：全局唯一，且**定义在本模块里**
//!
//! 面板的跨帧数据放在 [`ColorPickerState`]（[`state`] 子模块，**不散进 `ui.rs`**），
//! 由 [`UiState`](crate::UiState) 持有**一份**，所有 `ColorPicker` 通用：
//!
//! - `mode`：呈现模式（在一个取色器里切到 HEX，另一个也是 HEX——用户偏好是一致的）；
//! - `text`：**替补输入缓冲**——调用方没有传 `&mut String`
//!   （[`ColorPicker::with_hex`]）时用它，于是绝大多数调用点不必自己持有 `String`；
//! - `open`：当前展开的面板。**同时只有一个**（共享文本缓冲必须只有一个所有者，
//!   否则两个面板会互相覆写屏幕上的值）；
//! - `hsv` 缓存：RGB→HSV 在 V=0 / S=0 时**丢色相**，缓存让"把明度拖到 0 再拖回来"
//!   的色相不跳（这是它存在的唯一理由）。
//!
//! 面板内容、格式表与 HSV 推导细节见各自子模块文档。
//!
//! # 无内部状态（颜色仍直接写在 `&mut Color` 上）
//!
//! "这一帧颜色变了吗"由调用方前后比较——与 `ui.slider_at(..)` 返回新值同一个路子，
//! 因此不需要往 [`Response`] 加字段。
//!
//! ⚠ 控件协议里的尺寸一律是**物理像素**（`Widget::size` 的返回值会被直接当作物理
//! 像素用于布局；`Theme` 在 `Ui` 内已按 DPI 预乘），本控件与 [`NumberInput`](super::NumberInput)
//! 一致。
//!
//! ```no_run
//! # use rjw_ui::{ColorPicker, Ui};
//! # fn demo(ui: &mut Ui, mut color: rjw_color::Color) {
//! let before = color;
//! ui.add(ColorPicker::new("tint", &mut color).alpha(true));
//! if color != before {
//!     // 颜色变了 —— 做点什么
//! }
//! # }
//! ```

mod format;
mod hsv;
mod panel;
mod state;

pub use format::{
    ColorFormat, color_hex, format_color, format_f, format_u8, ink_on, luma, parse_color,
    parse_hex,
};
pub use state::ColorPickerState;

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::draw::{CornerRadius, DrawKind, Icon, TextVAlign};
use crate::id::IdAbsolute;
use crate::{Response, TextAlign, Ui, Widget};

/// 内联色块高度（物理像素）。
const SWATCH_H: f32 = 22.0;
/// 色块 / 面板内矩形的圆角（物理像素）。
pub(super) const SWATCH_RADIUS: f32 = 3.0;

/// 颜色选择器（内联色块 → 点击弹出取色面板）。
pub struct ColorPicker<'a> {
    id: &'a str,
    color: &'a mut Color,
    /// 是否在面板里显示 Alpha 行。
    alpha: bool,
    /// 可选的文本编辑缓冲（跨帧由调用方持有；`None` = 用全局跨帧缓冲）。
    hex: Option<&'a mut String>,
    /// 弹出面板宽度（物理像素；`None` = 内联宽度的 1.9 倍与"通道行最小宽"取大）。
    popup_w: Option<f32>,
}

impl<'a> ColorPicker<'a> {
    /// 主构造：颜色直接写在 `&mut Color` 上。
    pub fn new(id: &'a str, color: &'a mut Color) -> Self {
        Self { id, color, alpha: false, hex: None, popup_w: None }
    }

    /// 面板里显示 Alpha 行（默认不显示：多数取色只关心 RGB）。
    pub fn alpha(mut self, on: bool) -> Self {
        self.alpha = on;
        self
    }

    /// 可选：把**顶部文本框**绑到调用方持有的缓冲（形如 `String::new()`）。
    ///
    /// **不传则用全局跨帧缓冲**（[`ColorPickerState::text`]）——绝大多数调用点不需要
    /// 自己持有 `String`；需要从外部读显示文本时再传。文本框的内容会被本控件每帧
    /// 重写成当前颜色，除非正在编辑（聚焦）。
    pub fn with_hex(mut self, buf: &'a mut String) -> Self {
        self.hex = Some(buf);
        self
    }

    /// 弹出面板宽度（物理像素）。
    pub fn popup_width(mut self, w: f32) -> Self {
        self.popup_w = Some(w);
        self
    }
}

/// 画一个"色块"（圆角矩形填充 + 居中的十六进制文本）——内联部分用。
fn push_swatch(ui: &mut Ui, rect: Rect, color: Color, with_alpha: bool, font_size: f32) {
    let border = ui.theme.input.border;
    ui.push_panel_like(rect, color, border, 1.0, CornerRadius::all(SWATCH_RADIUS), 1);
    ui.push_text_rect(
        rect,
        &color_hex(color, with_alpha),
        font_size,
        ink_on(color),
        None,
        TextAlign::Center,
        TextVAlign::Center,
        None,
        None,
    );
}

/// 焦点 id 是否落在本取色器的面板内（面板窗口 id = `{abs}::popup`）。
fn focus_inside(ui: &Ui, abs: &IdAbsolute<'_>) -> bool {
    let prefix = format!("{}::popup", abs.as_str());
    ui.state()
        .focused
        .as_ref()
        .is_some_and(|f| f.as_str().starts_with(&prefix))
}

impl Widget for ColorPicker<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        // 申请：内联只占**一行**（面板弹出，不参与这里的尺寸结算）。
        let rect = ui.allocate(Vec2::new(ui.theme.input.min_w, SWATCH_H));
        // 先解构：`color`（&mut Color）与 `hex`（Option<&mut String>）是**互不相干**的
        // 借用，颜色值在外面读写成 `Copy` 的 `Color`，两边不打架。
        let ColorPicker { id, color, alpha, hex, popup_w } = self;
        let color_in = *color;
        let id_for = ui.id_for(id);
        let abs = id_for.to_static();
        let btn = ui.mouse_left();
        let hit = ui.hit_abs(&abs, &rect);

        // ── 内联色块（整行）：当前色 + 十六进制 + 右侧 ⌄ 提示 ──
        let label_fs = ui.theme.label.font_size;
        if hit {
            ui.set_cursor(crate::UiCursor::Default);
        }
        push_swatch(ui, rect, color_in, alpha, label_fs);
        let ink = ink_on(color_in);
        let mut open = ui.state().color_picker.is_open(&abs);
        // 展开箭头用**矢量图标**（与字体无关）。
        ui.push_draw(
            DrawKind::Icon {
                icon: if open { Icon::ChevronUp } else { Icon::ChevronDown },
                color: ink,
            },
            Rect::new(rect.x + rect.w - 16.0, rect.y + (rect.h - 12.0) * 0.5, 12.0, 12.0),
            ui.elem_hint(),
        );

        // 点色块 = 开关面板。`claim_press` 阻止外层窗口把这次按下当作窗口拖拽基准。
        if btn.down_edge() && hit {
            ui.claim_press();
            ui.state_mut().color_picker.toggle(&abs);
            open = ui.state().color_picker.is_open(&abs);
        }
        // `Esc`：面板内有文本框持焦点时**只失焦不收起**（否则编辑到一半面板就没了）；
        // 否则收起面板。
        if open && ui.key_down_edge(winit::keyboard::KeyCode::Escape) && !focus_inside(ui, &abs) {
            ui.state_mut().color_picker.close(&abs);
            open = false;
        }

        if !open {
            return Response { rect, hovered: hit, ..Default::default() };
        }

        // ── 弹出面板（尺寸解算 / 绘制 / 交互都在 `panel` 子模块）──
        let (color_out, inside) = panel::show_popup(ui, id, &abs, rect, color_in, alpha, hex, popup_w);
        *color = color_out;

        // 点面板外（且不在色块上）→ 收起。
        if btn.down_edge() && !hit && !inside {
            ui.state_mut().color_picker.close(&abs);
        }
        Response { rect, hovered: hit, ..Default::default() }
    }
}
