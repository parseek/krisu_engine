//! **扩展标签**（[`LabelEx`] / [`Label::ex`](crate::Label::ex)）：高度自定义的文本控件——
//! 整组 [`TextStyle`] + 字段级覆盖 + **首末两色渐变**，用于彩色文本 / 富样式标签。
//!
//! # 维护者笔记（改这个文件前先读）
//!
//! 1. **它是"另一个 Label"，不是 Label 的替代**：`Label::new` 保持"最简 + 主题回落"，
//!    本控件给需要字重 / 斜体 / 字距 / 行高 / 渐变 / 逐标签样式的场合。两者**共用**
//!    绘制与测量管线（`Ui::cache_buffer_styled` / `TextAlign`），不各写一套排版。
//! 2. **文本缓冲必须"测量与绘制同一个"**：`push_text_rect_ramp(.., buf: Some(..))` ——
//!    绘制期若给 `buf: None`，`draw_text_quads` 会回落到 `cache_buffer(text, size, family)`，
//!    于是**字重 / 斜体 / 字距 / 行高全丢**（排版按主题默认重来）⇒ 文字与占用矩形不一致。
//! 3. **水平对齐走绘制期**（[`TextAlign`]，与 `Theme::label.align` 同口径）：排版缓冲的
//!    `align` 恒 `Left`（见 `Ui::cached_buffer_styled`）——排版期再对齐一次会让多行文本
//!    左右各偏一次。
//! 4. **行高回落与 `Label` 逐字一致**（[`LabelEx::resolve`] 第 ④ 步）：换行文本 = `字号 ×
//!    Theme::line_spacing`、单行 = 字号。少了这一步，同一主题下 `Label` 与 `LabelEx`
//!    的同文本尺寸会不同（换行标签会高一截）。
//! 5. **样式优先级固定**：字段级糖 > `.style(TextStyle)` > `Theme::label`——与**调用顺序无关**
//!    （`style` 与字段分开存，最后合并），免得"谁后写谁赢"这种要靠记忆的规则。
//! 6. 纯逻辑（样式合并 / 文本块渐变采样）抽成可单测的自由函数与纯方法（[`LabelEx::resolve`]
//!    不碰 `Ui`，`TextRamp::t_in` / `color_at` 在 `draw.rs` 里已有单测点位）。
//! 7. **渐变的三条口径在 `ui/cmds.rs::draw_text_quads`**（改渐变行为先读那里）：
//!    域与采样点都在**文本视觉原点系**（不要换算到窗口/绝对坐标——曾因此让渐变随控件位置
//!    漂移、被 clamp 成纯色）、采样用**未裁剪**字形几何（裁剪不改变颜色）、域三档
//!    `Glyph`/`Line`（默认）/`Frame`（整块 = "Text 域"）。本文件只负责把选择传给 `TextRamp`。

use std::borrow::Cow;
use std::sync::Arc;

use glam::Vec2;
use rjw_color::Color;
use rjw_text::cosmic_text::FamilyOwned;
use rjw_text::{Align, GradientAxis, GradientMode, Stretch, Text, TextStyle, Weight};
use rjw_transform::Rect;

use crate::draw::{Size, TextAlign, TextRamp, TextVAlign};
use crate::style::Theme;
use crate::ui::{Ui, UiAdd};
use crate::widgets::{Response, Widget};

/// **扩展标签**（属性化 builder；未设置的属性回落 [`Theme::label`]，样式优先级 =
/// 字段级 > [`LabelEx::style`] > `Theme::label`）。
///
/// ```no_run
/// # use rjw_ui::{Label, UiAdd};
/// # use rjw_ui::text::TextStyle;
/// # use rjw_ui::Weight;
/// # fn demo(ui: &mut rjw_ui::Ui, ts: TextStyle) {
/// // 一行写全：整组样式 + 字段级覆盖 + 渐变
/// Label::ex("HP 100")
///     .style(ts)
///     .tint(rjw_color::Color::GREEN)
///     .font_family("Sarasa Mono SC")
///     .weight(Weight::BOLD)
///     .gradient(rjw_color::Color::RED, rjw_color::Color::YELLOW)
///     .show(ui);                       // ← 裸 `Ui` 或任何 `UiAdd` 容器都能传
/// # }
/// ```
///
/// 常规写法两条：
/// - **裸 `Ui`**：`Label::ex("…").….show(ui)`；
/// - **容器闭包里**：`ui.label_ex("…").….show(ui)` 或 `ui.add(Label::ex("…"))`。
pub struct LabelEx<'a> {
    text: &'a str,
    /// 整组文本样式（[`LabelEx::style`]；`None` = 从 `Theme::label` 派生）。
    style: Option<TextStyle>,
    // ── 字段级覆盖（优先级最高；只存差异）────────────────────────────
    tint: Option<Color>,
    font_size: Option<Size<f32>>,
    font_family: Option<&'a str>,
    weight: Option<Weight>,
    italic: Option<bool>,
    stretch: Option<Stretch>,
    letter_spacing: Option<f32>,
    line_height: Option<Size<f32>>,
    align: Option<Align>,
    valign: Option<TextVAlign>,
    wrap: Option<Size<f32>>,
    ellipsis: bool,
    /// 首末两色渐变（`None` = 整段用 `tint` 单色）。
    ramp: Option<TextRamp>,
}

impl<'a> LabelEx<'a> {
    /// 新建（最简形态 = 与 [`Label::new`](crate::Label::new) 同观感）。
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            style: None,
            tint: None,
            font_size: None,
            font_family: None,
            weight: None,
            italic: None,
            stretch: None,
            letter_spacing: None,
            line_height: None,
            align: None,
            valign: None,
            wrap: None,
            ellipsis: false,
            ramp: None,
        }
    }

    /// **整组文本样式**（[`TextStyle`]）：给了就以它为准，字段级糖再在其上覆盖。
    ///
    /// ⚠ `origin` / `offset` / `transform` **被忽略**：`rjw_ui` 的文本位置由
    /// "分配矩形 + `align` / `valign`"决定（文本链的渲染级变换在这里没有对应物）。
    /// ⚠ 用 `TextStyle::attrs(..)` 塞进本控件未覆盖的字段（`font_features` /
    /// `text_decoration` 等）时，**排版缓存键不包含它们** ⇒ 改这些字段不会触发重排版。
    pub fn style(mut self, style: TextStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// **文本色 / 染色**（覆盖 `style` 里的颜色与 `Theme::label.color`）。
    ///
    /// 命名与 `Draw2D` 的 `.tint()` 一致（见 `docs/API_DESIGN.md` 的 P2 决策）。
    /// 有渐变时它是"基准色"：最终颜色 = 渐变采样 ×`tint`（白 = 纯渐变）。
    pub fn tint(mut self, c: Color) -> Self {
        self.tint = Some(c);
        self
    }

    /// 字号（[`Size<f32>`]：逻辑（默认，× scale）/ 物理原样；默认 `Theme::label.font_size`）。
    pub fn font_size(mut self, s: impl Into<Size<f32>>) -> Self {
        self.font_size = Some(s.into());
        self
    }

    /// 字体族（默认 `Theme::label.font_family`）。
    pub fn font_family(mut self, f: &'a str) -> Self {
        self.font_family = Some(f);
        self
    }

    /// 字重（默认 `Theme::font_weight`）。字体无该字重时由 cosmic-text 取最接近字面。
    pub fn weight(mut self, w: Weight) -> Self {
        self.weight = Some(w);
        self
    }

    /// 斜体（字体无斜体字面时由 cosmic-text 合成伪斜体）。
    pub fn italic(mut self, on: bool) -> Self {
        self.italic = Some(on);
        self
    }

    /// 拉伸（`Stretch` 九档；字体无该拉伸时回落最接近字面）。
    pub fn stretch(mut self, s: Stretch) -> Self {
        self.stretch = Some(s);
        self
    }

    /// 字距（**EM**，相对字号；`0.02` ≈ 2%）。
    pub fn letter_spacing(mut self, em: f32) -> Self {
        self.letter_spacing = Some(em);
        self
    }

    /// 显式行高（[`Size<f32>`]；默认按 `Theme::line_spacing` 折算，见模块维护者笔记 4）。
    pub fn line_height(mut self, h: impl Into<Size<f32>>) -> Self {
        self.line_height = Some(h.into());
        self
    }

    /// 水平对齐（默认 `Theme::label.align`）。
    pub fn align(mut self, a: Align) -> Self {
        self.align = Some(a);
        self
    }

    /// 垂直对齐（默认 [`TextVAlign::Center`]，与 `Label` 一致）。
    pub fn valign(mut self, v: TextVAlign) -> Self {
        self.valign = Some(v);
        self
    }

    /// 换行宽度（[`Size<f32>`]；`<= 0` = 不换行。不调用 = 在父级可用宽内自动换行）。
    pub fn wrap(mut self, max_w: impl Into<Size<f32>>) -> Self {
        self.wrap = Some(max_w.into());
        self
    }

    /// **省略模式**：超出可用 / 分配宽度时以 "…" 截断为单行（内容自洽）。
    pub fn ellipsis(mut self) -> Self {
        self.ellipsis = true;
        self
    }

    /// **横向渐变**（左 → 右，首末两色；域默认**逐行** —— 见 [`LabelEx::gradient_mode`]）。
    pub fn gradient(mut self, from: Color, to: Color) -> Self {
        self.ramp = Some(TextRamp::horizontal(from, to));
        self
    }

    /// **纵向渐变**（上 → 下，首末两色；域默认**逐行**）。
    pub fn gradient_v(mut self, from: Color, to: Color) -> Self {
        self.ramp = Some(TextRamp::vertical(from, to));
        self
    }

    /// 显式指定渐变方向（与 [`GradientAxis`] 共用：横 / 纵）。
    pub fn gradient_axis(mut self, from: Color, to: Color, axis: GradientAxis) -> Self {
        self.ramp = Some(TextRamp {
            from,
            to,
            axis,
            mode: GradientMode::Line,
        });
        self
    }

    /// **换渐变域**（`Glyph` 逐字形 / `Line` 逐行 / `Frame` 整块=`Text` 域）——保持两色与方向。
    ///
    /// 三档语义（单行文本时 Line 与 Frame **逐像素相同**）：
    /// - [`GradientMode::Glyph`]：每个字自身一条完整渐变（大字号下逐字重复）；
    /// - [`GradientMode::Line`]（默认）：每行一条完整渐变（多行时短行不会只吃到半条）；
    /// - [`GradientMode::Frame`]：**整块**一条渐变、跨行连续（短行只覆盖它对应的一段）。
    ///
    /// ```no_run
    /// # use rjw_ui::{Label, UiAdd};
    /// # use rjw_ui::text::GradientMode;
    /// # fn demo(ui: &mut rjw_ui::Ui) {
    /// Label::ex("多行文本\n第二行")
    ///     .gradient(rjw_color::Color::RED, rjw_color::Color::BLUE)
    ///     .gradient_mode(GradientMode::Frame)     // 跨行一个渐变
    ///     .show(ui);
    /// # }
    /// ```
    pub fn gradient_mode(mut self, mode: GradientMode) -> Self {
        self.ramp = Some(match self.ramp {
            Some(r) => r.mode(mode),
            None => TextRamp::horizontal(Color::WHITE, Color::WHITE).mode(mode),
        });
        self
    }

    /// **逐字形渐变**（横向；等价 `.gradient(..).gradient_mode(Glyph)`）。
    pub fn gradient_glyph(self, from: Color, to: Color) -> Self {
        self.gradient(from, to).gradient_mode(GradientMode::Glyph)
    }

    /// **逐行渐变**（横向；等价 `.gradient(..).gradient_mode(Line)`，即默认档）。
    pub fn gradient_line(self, from: Color, to: Color) -> Self {
        self.gradient(from, to).gradient_mode(GradientMode::Line)
    }

    /// **整块渐变（"Text 域"）**（横向；等价 `.gradient(..).gradient_mode(Frame)`）——
    /// 跨行连续的一条渐变。
    pub fn gradient_text(self, from: Color, to: Color) -> Self {
        self.gradient(from, to).gradient_mode(GradientMode::Frame)
    }

    /// 同 [`LabelEx::gradient_text`]（`Frame` 的另一个名字，取自 `rjw_text` 的枚举）。
    pub fn gradient_frame(self, from: Color, to: Color) -> Self {
        self.gradient_text(from, to)
    }

    /// **录制到裸 [`Ui`]**（返回 [`Response`]，`rect` = 本帧占用矩形）——链式写法的终点：
    ///
    /// ```no_run
    /// # use rjw_ui::{Label, UiAdd};
    /// # fn demo(ui: &mut rjw_ui::Ui) {
    /// Label::ex("HP 100").tint(rjw_color::Color::GREEN).show(ui);
    /// # }
    /// ```
    pub fn show(self, ui: &mut Ui) -> Response {
        ui.add(self)
    }

    /// **录制到容器**（任何 [`UiAdd`]：窗口 / 面板 / pack / row / grid / 滚动容器 …）——
    /// 容器闭包里 `ui.label_ex("…").….show_in(ui)`；等价于 `ui.add(Label::ex("…"))`。
    ///
    /// ⚠ **为什么不是一个 `show`**：`Ui` **刻意不实现** [`UiAdd`]（见 trait 文档：裸 `Ui`
    /// 走固有 `add` / `*_at`，容器走 trait 方法）——两种接收者无法被同一个泛型约束覆盖，
    /// 所以裸 `Ui` 用 [`LabelEx::show`]、容器用本方法。
    pub fn show_in<'ui, U: UiAdd<'ui>>(self, container: &mut U) -> Response {
        container.add(self)
    }

    /// **样式解析**（纯逻辑，可单测）：`Theme::label` 起步 → 整组 `style` → 字段级覆盖 →
    /// 颜色 → 行高回落。`wrap_px` = 本帧**实际**换行宽（物理像素；`<= 0` = 不换行），
    /// 只影响行高回落（换行文本按 `Theme::line_spacing` 行距）。
    ///
    /// 返回 `(排版样式, 颜色, 水平对齐, 垂直对齐)`。
    pub(crate) fn resolve(
        &self,
        theme: &Theme,
        scale: f32,
        wrap_px: f32,
    ) -> (TextStyle, Color, Align, TextVAlign) {
        // ① 主题起步（`Theme::label` 的四项 + 全局字重令牌）。
        let mut style = TextStyle::new()
            .size(theme.label.font_size)
            .weight(theme.font_weight)
            .align(Align::Left);
        if let Some(f) = &theme.label.font_family {
            style = style.font_family(f.as_ref());
        }
        let mut tint = theme.label.color;
        let mut align = theme.label.align;
        let mut valign = TextVAlign::Center;
        // ② 整组样式（全量结构：给了就以它为准）。
        if let Some(s) = &self.style {
            style = s.clone();
        }
        // ③ 字段级覆盖。
        if let Some(sz) = self.font_size {
            style.size = sz.to_physical(scale);
        }
        if let Some(f) = self.font_family {
            style = style.font_family(f);
        }
        if let Some(w) = self.weight {
            style = style.weight(w);
        }
        if let Some(on) = self.italic {
            style = style.italic(on);
        }
        if let Some(st) = self.stretch {
            style = style.stretch(st);
        }
        if let Some(ls) = self.letter_spacing {
            style = style.letter_spacing(ls);
        }
        if let Some(lh) = self.line_height {
            style.line_height = Some(lh.to_physical(scale));
        }
        if let Some(a) = self.align {
            align = a;
        }
        if let Some(v) = self.valign {
            valign = v;
        }
        // 颜色：`style.color`（若给）→ 字段级 `.tint(..)` 覆盖。
        if let Some(c) = style.color {
            tint = Color::from(c);
        }
        if let Some(c) = self.tint {
            tint = c;
        }
        // ④ 行高回落：用户既没给行高也没给行距时，**与 `Label` 同口径**折算
        //    （换行 = 字号 × Theme::line_spacing；单行 = 字号）。
        if style.line_height.is_none() && style.line_space.is_none() {
            let mult = if wrap_px > 0.0 { theme.line_spacing } else { 1.0 };
            style.line_height = Some((style.size * mult.max(1.0)).round());
        }
        (style, tint, align, valign)
    }
}

impl Widget for LabelEx<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let scale = ui.scale();
        // ① **先量**（申请之前；`avail_w()` 必须在 `allocate` 之前取）。
        let explicit_wrap = self.wrap.map(|w| w.to_physical(scale)).filter(|&w| w > 0.0);
        let avail = ui.avail_w().filter(|&w| w > 0.0);
        // **判"要不要换行"要先知道自然宽**——横向宽度与行高无关，所以先用"单行口径"的样式
        // 量一次即可。这一步是必需的：换行与否决定行高折算（见模块维护者笔记 4），若直接
        // 拿 `avail_w` 当换行宽，自动宽窗口会"首帧 `avail_w = None` ⇒ 行高 = 字号；次帧拿到
        // 内容宽 ⇒ 行高 = 字号 × 行距" ⇒ **首帧抖一下**，且与 `Label` 不同口径（`Label` 在
        // 文本不超宽时传 `wrap = 0`，行高恒为字号）。
        let (probe, probe_tint, probe_align, probe_valign) = self.resolve(ui.theme(), scale, 0.0);
        let natural_w = measure(ui, self.text, &probe, 0.0).0.x;
        let wrap_w = wrap_width(explicit_wrap, avail, natural_w);
        let (style, tint, align, valign) = if wrap_w > 0.0 {
            self.resolve(ui.theme(), scale, wrap_w)
        } else {
            (probe, probe_tint, probe_align, probe_valign)
        };
        let family = style_family(&style);
        // 测量与绘制**同一个缓冲**（见模块维护者笔记 2）：这里留住它，绘制期不再查缓存。
        let (full, full_buf) = measure(ui, self.text, &style, wrap_w);
        let natural = if self.ellipsis {
            match avail {
                Some(a) if a < full.x => Vec2::new(a, full.y),
                _ => full,
            }
        } else {
            full
        };
        // ② 申请（占光标）+ 剔除早退。
        let rect = ui.allocate(natural);
        if ui.culled(rect) {
            return Response { rect, culled: true, ..Default::default() };
        }
        // ③ 画：省略 ⇒ "…"截断（单行、内容自洽）；否则按换行宽取缓冲。
        let (draw_text, buf) = if self.ellipsis && natural.x < full.x {
            let cut = crate::edit::ellipsize(self.text, rect.w, |s| measure(ui, s, &style, 0.0).0.x);
            let (_, cut_buf) = measure(ui, cut.as_ref(), &style, 0.0);
            (cut, cut_buf)
        } else {
            (Cow::Borrowed(self.text), full_buf)
        };
        trace_label(self.text, &style, tint, self.ramp, wrap_w, rect);
        ui.push_text_rect_ramp(
            rect,
            draw_text.as_ref(),
            style.size,
            tint,
            family,
            TextAlign::from(align),
            valign,
            None,
            Some(buf),
            self.ramp,
        );
        Response { rect, ..Default::default() }
    }
}

/// **本帧实际换行宽**（纯函数，可单测）——三态：
///
/// - 显式 `wrap(w)`（`w > 0`）：**恒**返回 `w`（即使文本不超宽也按换行排版 —— 与
///   `Label::wrap(..)` 同口径，行距随之生效）；
/// - 未显式指定、且自然宽**超过**可用宽：返回可用宽（自动换行，`LimitedInParent` 语义）；
/// - 否则 `0`（**不换行**）——这一支是"行高 = 字号"的依据，也是**首帧不抖**的关键：
///   自动宽窗口首帧 `avail_w = None`、次帧才有内容宽，若把"有了宽"当成"要换行"，
///   行高会在次帧从 `字号` 跳到 `字号 × 行距`（用户看到"标签突然变高一截"）。
fn wrap_width(explicit: Option<f32>, avail: Option<f32>, natural_w: f32) -> f32 {
    match explicit {
        Some(w) => w,
        None => match avail {
            Some(a) if a > 0.0 && natural_w > a => a,
            _ => 0.0,
        },
    }
}

/// **字体族名**（从 [`TextStyle`] 的 attrs 取）：`FamilyOwned::Name` 取名字；具名族
/// （`Serif` / `Monospace` …）返回 `None`（rjw_ui 的绘制只认"族名 + 系统默认"两态）。
fn style_family(style: &TextStyle) -> Option<Arc<str>> {    match &style.attrs.family_owned {
        FamilyOwned::Name(n) if !n.is_empty() => Some(Arc::from(n.as_ref())),
        _ => None,
    }
}

/// 按样式测量文本（物理像素，ceil）并**留住排版缓冲**——绘制期直接用同一份
/// （见模块维护者笔记 2；也省掉"同一样式查两次缓存"）。
fn measure(ui: &mut Ui, text: &str, style: &TextStyle, wrap_px: f32) -> (Vec2, Arc<rjw_text::Buffer>) {
    let buf = ui.cache_buffer_styled(text, style, wrap_px);
    (Text::measure_buffer(&buf).ceil(), buf)
}

/// `RJ_LABEL_TRACE=1`：打印每个 `LabelEx` 的解析结果与落点。
///
/// 为什么要它：本控件有**三条样式来源**（Theme / `.style(..)` / 字段级糖）与两个容易
/// 搞错的口径（行高回落、渐变轴）——"字重没生效""渐变方向反了""换行标签比 `Label` 高
/// 一截"这类问题时，第一件事就是看这几个数（照 `RJ_FOLD_TRACE` 的口径）。
fn trace_label(
    text: &str,
    style: &TextStyle,
    tint: Color,
    ramp: Option<TextRamp>,
    wrap_w: f32,
    rect: Rect,
) {
    if std::env::var_os("RJ_LABEL_TRACE").is_some() {
        let c: [f32; 4] = tint.into();
        eprintln!(
            "label_ex[{text:?}] size={:.1} weight={} italic={} spacing={:?} \
             line_h={:?} line_space={:?} tint=({:.2},{:.2},{:.2},{:.2}) ramp={:?} \
             wrap={wrap_w:.1} rect=({:.1},{:.1},{:.1}x{:.1})",
            style.size,
            style.attrs.weight.0,
            matches!(style.attrs.style, rjw_text::cosmic_text::Style::Italic),
            style.attrs.letter_spacing_opt.map(|l| l.0),
            style.line_height,
            style.line_space,
            c[0],
            c[1],
            c[2],
            c[3],
            ramp,
            rect.x,
            rect.y,
            rect.w,
            rect.h,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Theme;

    fn base() -> Theme {
        Theme::dark()
    }

    /// **优先级固定**：字段级 > `.style(TextStyle)` > `Theme::label`。
    #[test]
    fn resolve_priority_is_field_then_style_then_theme() {
        let th = base();
        let theme_size = th.label.font_size;
        // 三档：什么都没设 ⇒ 主题。
        let plain = LabelEx::new("x").resolve(&th, 1.0, 0.0);
        assert_eq!(plain.0.size, theme_size, "未设 ⇒ 主题字号");
        assert_eq!(plain.1, th.label.color, "未设 ⇒ 主题颜色");
        // 整组 style ⇒ 盖过主题。
        let ts = TextStyle::new().size(20.0).color([1.0, 0.0, 0.0, 1.0]);
        let with_style = LabelEx::new("x").style(ts).resolve(&th, 1.0, 0.0);
        assert_eq!(with_style.0.size, 20.0);
        assert_eq!(with_style.1, Color::from([1.0, 0.0, 0.0, 1.0]));
        // 字段级 ⇒ 盖过 style（且与调用顺序无关：`.style` 写在字段糖之后也一样）。
        let overridden = LabelEx::new("x")
            .style(TextStyle::new().size(20.0))
            .font_size(30.0)
            .tint(Color::GREEN)
            .resolve(&th, 1.0, 0.0);
        assert_eq!(overridden.0.size, 30.0, "字段级字号压过 style");
        assert_eq!(overridden.1, Color::GREEN, "tint 压过 style.color 与主题");
        let same = LabelEx::new("x")
            .font_size(30.0)
            .tint(Color::GREEN)
            .style(TextStyle::new().size(20.0))
            .resolve(&th, 1.0, 0.0);
        assert_eq!(same.0.size, 30.0, "优先级与调用顺序无关（字段级恒胜）");
        assert_eq!(same.1, Color::GREEN);
    }

    /// **逻辑单位按 scale 换算**（`Size::Logical` 默认）。
    #[test]
    fn resolve_scales_logical_units() {
        let th = base();
        let (style, ..) = LabelEx::new("x")
            .font_size(Size::Logical(20.0))
            .line_height(Size::Logical(30.0))
            .resolve(&th, 1.5, 100.0);
        assert_eq!(style.size, 30.0, "20 逻辑 × 1.5 = 30 物理");
        assert_eq!(style.line_height, Some(45.0));
        let (phys, ..) = LabelEx::new("x")
            .font_size(Size::Physical(20.0))
            .resolve(&th, 1.5, 100.0);
        assert_eq!(phys.size, 20.0, "物理字号原样");
    }

    /// **行高回落与 `Label` 同口径**：换行 = `字号 × Theme::line_spacing`；单行 = 字号。
    #[test]
    fn resolve_line_height_follows_theme_like_label() {
        let th = base();
        let single = LabelEx::new("x").resolve(&th, 1.0, 0.0);
        assert_eq!(single.0.line_height, Some(th.label.font_size), "单行行高 = 字号");
        let wrapped = LabelEx::new("x").resolve(&th, 1.0, 200.0);
        assert_eq!(
            wrapped.0.line_height,
            Some((th.label.font_size * th.line_spacing.max(1.0)).round()),
            "换行行高 = 字号 × 主题行距"
        );
        // 显式行高 / 行距优先，回落不生效。
        let explicit = LabelEx::new("x")
            .line_height(Size::Logical(40.0))
            .resolve(&th, 1.0, 200.0);
        assert_eq!(explicit.0.line_height, Some(40.0));
    }

    /// 渐变的三种写法落到同一个 [`TextRamp`] 上。
    #[test]
    fn gradient_constructors_set_the_expected_axis() {
        let h = LabelEx::new("x").gradient(Color::RED, Color::BLUE);
        assert_eq!(h.ramp.map(|r| r.axis), Some(GradientAxis::Horizontal));
        let v = LabelEx::new("x").gradient_v(Color::RED, Color::BLUE);
        assert_eq!(v.ramp.map(|r| r.axis), Some(GradientAxis::Vertical));
        let a = LabelEx::new("x").gradient_axis(Color::RED, Color::BLUE, GradientAxis::Vertical);
        assert_eq!(a.ramp, v.ramp);
    }

    /// **换行宽判定**：显式恒生效；自动只在超宽时换；不超宽 ⇒ `0`（行高 = 字号）。
    ///
    /// 这条钉住的是**首帧不抖**：自动宽窗口首帧 `avail_w = None`（`0` 宽 = 不换行），
    /// 次帧拿到内容宽（`659`），但文本只有 `120` ⇒ 仍然不换行 ⇒ 行高不变。
    #[test]
    fn wrap_width_only_wraps_when_needed() {
        // 自动换行：文本不超宽 ⇒ 不换（哪怕可用宽已知且很大）。
        assert_eq!(wrap_width(None, Some(659.0), 120.0), 0.0, "不超宽 ⇒ 不换行（行高 = 字号）");
        assert_eq!(wrap_width(None, None, 120.0), 0.0, "首帧没有可用宽 ⇒ 不换行");
        // 自动换行：超宽 ⇒ 用可用宽。
        assert_eq!(wrap_width(None, Some(100.0), 120.0), 100.0, "超宽 ⇒ 按可用宽换行");
        assert_eq!(wrap_width(None, Some(0.0), 120.0), 0.0, "可用宽 0 = 没线索 ⇒ 不换行");
        // 显式 wrap：恒生效（即使文本很短 —— 与 `Label::wrap` 同口径，行距随之生效）。
        assert_eq!(wrap_width(Some(200.0), Some(659.0), 120.0), 200.0, "显式 wrap 优先且恒生效");
        assert_eq!(wrap_width(Some(80.0), None, 120.0), 80.0, "显式 wrap 不依赖可用宽");
    }
}
