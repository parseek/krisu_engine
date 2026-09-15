//! **取色面板**（弹出的置顶窗口）：布局、绘制与交互。
//!
//! 与 `ColorPicker`（内联色块，见 [`super`]）分开：这里只关心"面板长什么样、
//! 拖动时怎么改颜色"，颜色最终由调用方从 `&mut Color` 比较得出。
//!
//! ```text
//! ┌──────────────────────────────────────────┐
//! │ [u8] [HEX] [F]                            │  ① 呈现模式（三选一）
//! │ ┌──────────────────────────┐ ┌──┐         │
//! │ │ 255, 0, 0                │ │⚠ │         │  ② 文本框（按模式呈现）+ 非法时的警告按钮
//! │ └──────────────────────────┘ └──┘         │
//! │ ┌───────────────────────┐ ┌┐              │
//! │ │        SV 平面        │ ││              │  ③ HSV 选择区（平面 + 竖向色相条）
//! │ └───────────────────────┘ └┘              │
//! │ R ▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬ [ 255 ]               │  ④ 通道行：颜色滑块（渐变轨）+ NumberInput
//! │ A ▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬▬ [ 255 ]  ← 可选        │
//! └──────────────────────────────────────────┘
//! ```
//!
//! ⚠ **求值顺序 ≠ 视觉顺序**：③ HSV 区先求值，② 文本框后求值（绘制顺序相反也无妨，
//! 各控件矩形互不重叠）。这样拖 SV 平面时文本框**同一帧**就显示新值，而不是落后一帧。

use glam::Vec2;
use rjw_color::Color;
use rjw_keystate::KeyState;
use rjw_transform::Rect;

use crate::draw::{CornerRadius, DrawKind, Gradient, Icon, Position, Size, TextVAlign};
use crate::hit::update_drag;
use crate::id::IdAbsolute;
use crate::layout::Child;
use crate::style::{Brush, PanelStyle, SliderStyle};
use crate::ui::{UiAdd, WIN_TOPMOST};
use crate::{TextAlign, Ui};

use super::SWATCH_RADIUS;
use super::format::{ColorFormat, format_color, parse_color, rgba, rgba_keep_alpha};
use super::hsv::{
    HUE_SEGS, hue_at, hue_seg_colors, hsv_to_rgb, rgb_to_hsv, sv_at, sv_plane_cells,
};

/// 面板内边距（物理像素）。
const PAD: f32 = 6.0;
/// 面板内行间距（物理像素）。
const GAP: f32 = 6.0;
/// 通道标签宽度（物理像素）。
const LABEL_W: f32 = 14.0;
/// 色相条宽度（物理像素）。
const HUE_W: f32 = 14.0;
/// 通道滑块的最小宽（物理像素）——决定面板最小宽的一部分。
const SLIDER_MIN_W: f32 = 90.0;
/// SV 平面的最小边长（物理像素）——窄面板下也不至于被压成一条。
const SV_MIN_SIDE: f32 = 90.0;

/// **SV 平面边长**：平面取**正方形**（HSV 选择器的通行形态，也与设计稿一致）——
/// 边长 = 面板内容宽去掉色相条与间隙。于是"面板越宽，平面越大"，不用手调常量。
#[inline]
fn sv_side(body_w: f32) -> f32 {
    (body_w - HUE_W - GAP).max(SV_MIN_SIDE)
}

/// 弹出面板：尺寸解算 + 置顶窗口 + 主体录制。
///
/// 返回 `(本帧新颜色, 面板矩形是否被点中)`——后者用于"点面板外收起"。
#[allow(clippy::too_many_arguments)]
pub(super) fn show_popup(
    ui: &mut Ui,
    id: &str,
    abs: &IdAbsolute<'static>,
    anchor: Rect,
    color_in: Color,
    alpha: bool,
    ext_buf: Option<&mut String>,
    popup_w: Option<f32>,
) -> (Color, bool) {
    let row = ui.theme.row_h;
    let input_h = ui.theme.input.height;
    let field_w = ui.theme.input.min_w;
    // 面板最小宽 = 一行通道所需（标签 + 滑块最小宽 + 数值框）：
    // 主题 `input.min_w` 变大也不会把行挤爆。
    let body_min = PAD * 2.0 + LABEL_W + GAP + SLIDER_MIN_W + GAP + field_w;
    let pw = popup_w.unwrap_or_else(|| (anchor.w * 1.9).max(body_min));
    let n_ch = 3 + if alpha { 1 } else { 0 };
    // SV 平面取正方（见 `sv_side`）：面板高随之变化，故先算边长再算高。
    let sv = sv_side(pw - PAD * 2.0);
    let ph = PAD * 2.0
        + row
        + GAP
        + input_h
        + GAP
        + sv
        + GAP
        + row * n_ch as f32;
    let popup_pos = Vec2::new(anchor.x, anchor.y + anchor.h + 2.0);
    let popup_raw = format!("{id}::popup");

    // 强制哨兵 z：`window_at` 的 `entry().or_insert()` 会保留既有值。
    ui.state_mut()
        .window_z
        .insert(IdAbsolute::owned(format!("{}::popup", abs.as_str())), WIN_TOPMOST);

    let panel_style = popup_panel_style(ui);
    let mut inside = false;
    let mut color_out = color_in;
    ui.window(&popup_raw)
        .pos(Position::Physical(popup_pos))
        .style(panel_style)
        .show(|w| {
            let ui = w.ui_mut();
            // 面板**本体**（不是控件）：用 `hit_body_abs`——它包含面板内的子控件，
            // 不能被自己的子控件判成"被挡住"（那会让点面板内也变成"点在面板外"）。
            inside = ui.hit_body_abs(&Rect::new(0.0, 0.0, pw, ph));
            let btn = ui.mouse_left();
            // 按在面板任意处都算"面板内的按下"（点面板外才收起）。
            if btn.down_edge() && inside {
                ui.claim_press();
            }
            color_out = popup_body(
                ui, id, color_in, alpha, ext_buf, pw, row, input_h, field_w, n_ch,
            );
            // 面板内容尺寸（自动宽 = pw，高 = ph）。
            ui.child_rect(pw, ph, Child::Expand);
        });
    (color_out, inside)
}

/// 面板背景样式（菜单底 + 细边框 + 小圆角，与下拉框浮层一致）。
fn popup_panel_style(ui: &Ui) -> PanelStyle {
    let cs = ui.theme.combo.clone();
    PanelStyle {
        bg: cs.menu_bg.into(),
        border: cs.menu_border,
        border_w: 1.0,
        padding: 0.0,
        radius: cs.menu_radius,
        bg_image: None,
    }
}

/// 面板主体：录制所有控件 + 解析所有交互，返回本帧的新颜色。
#[allow(clippy::too_many_arguments)]
fn popup_body(
    ui: &mut Ui,
    id: &str,
    color_in: Color,
    alpha: bool,
    mut ext_buf: Option<&mut String>,
    pw: f32,
    row: f32,
    input_h: f32,
    field_w: f32,
    n_ch: usize,
) -> Color {
    let body_w = pw - PAD * 2.0;
    let mut c = rgba(color_in);
    let mut y = PAD;

    // ── ① 呈现模式（三选一） ──
    let mode_now = ui.state().color_picker.mode;
    let mode_w = (body_w - GAP * 2.0) / 3.0;
    let mut mode_changed = false;
    for (i, m) in ColorFormat::ALL.iter().enumerate() {
        let r = Rect::new(PAD + (mode_w + GAP) * i as f32, y, mode_w, row);
        let pal = ui.theme.palette;
        let mut style = ui.theme.button.clone();
        if *m == mode_now {
            // 选中态 = 强调色实心（与"当前模式"这个事实一一对应）。
            style.bg = pal.accent.into();
            style.bg_hover = pal.accent_hover.into();
            style.bg_pressed = pal.accent_active.into();
        }
        let mid = format!("mode{i}");
        if ui.button_at_styled(&mid, r, m.label(), &style).clicked() {
            ui.state_mut().color_picker.mode = *m;
            mode_changed = true;
        }
    }
    let mode = ui.state().color_picker.mode;
    y += row + GAP;

    // ── ③ HSV 区（先求值：见模块文档的"求值顺序"）──
    // 平面与色相条**同高**，且平面取正方 ⇒ 两者拼成一块方形选择区。
    let sv = sv_side(body_w);
    let sv_rect = Rect::new(PAD, y + input_h + GAP, sv, sv);
    let hue_rect = Rect::new(PAD + sv + GAP, y + input_h + GAP, HUE_W, sv);
    let mut hsv = {
        let st = &ui.state().color_picker;
        if st.hsv_src == color_in {
            st.hsv
        } else {
            rgb_to_hsv(color_in)
        }
    };
    let mut hsv_dirty = false;
    let btn = ui.mouse_left();
    let mouse = ui.mouse_local();
    let sv_id = ui.id_for("sv").to_static();
    let hue_id = ui.id_for("hue").to_static();
    if drag_region_active(ui, &sv_id, &sv_rect, btn) {
        let (s, v) = sv_at(sv_rect, mouse);
        hsv[1] = s;
        hsv[2] = v;
        hsv_dirty = true;
    }
    if drag_region_active(ui, &hue_id, &hue_rect, btn) {
        hsv[0] = hue_at(hue_rect, mouse);
        hsv_dirty = true;
    }
    if hsv_dirty {
        // SV / 色相只改 HSV，alpha 保持当前值。
        c = rgba(hsv_to_rgb(hsv[0], hsv[1], hsv[2], c[3]));
    }
    // 绘制：平面 = 一张 **N×M 网格**的纯渐变四边形（每格四角色由 HSV 公式算出）。
    //
    // ⚠ 为什么不是**一块**四角渐变：四角顶点色经光栅化器只做**逐三角形线性**插值，
    // 而平面的颜色场是 `V·lerp(白, 色相, S)` —— 在 S、V 上都是双线性（有交叉项），
    // 于是"整块两三角 / 从中心扇形铺开"都会在三角形边界处产生可见的**折痕/条纹**
    // （对角或放射状），实测用户看到的就是"过渡有问题"。切成小格后每格的交叉项误差
    // 按格面积缩小（误差 ∝ 1/格数² 量级）⇒ 视觉上就是平滑渐变。
    // 用**纯四边形**（`DrawKind::Rect`）而不是圆角矩形：同一几何里的相邻格子**零羽化**、
    // 边缘严格相接，不会像"多个 RoundedRect 拼网格"那样在缝上二次衰减出一条线。
    for (cell, corners) in sv_plane_cells(sv_rect, hsv[0]) {
        ui.push_draw(DrawKind::Rect(Gradient::corners(corners[0], corners[1], corners[2], corners[3])), cell, ui.elem_hint());
    }
    let seg_h = sv / HUE_SEGS as f32;
    for i in 0..HUE_SEGS {
        let seg = Rect::new(hue_rect.x, hue_rect.y + i as f32 * seg_h, HUE_W, seg_h);
        let (top, bot) = hue_seg_colors(i);
        // ⚠ 段间**不羽化**（纯四边形，`DrawKind::Rect`）：相邻段各自带羽化时，两侧的
        // alpha 斜坡都降到 0 ⇒ 共享边上会透出一条面板底色的缝（"过渡有问题"的另一半）。
        // 硬边 + 严格相接反而无缝（段内仍是两端色渐变）；代价是色相条两端是直角。
        ui.push_draw(
            DrawKind::Rect(Gradient::corners(top, top, bot, bot)),
            seg,
            ui.elem_hint(),
        );
    }
    // 当前位置标记：平面上一个小方框（描边取底色的可读墨色）+ 色相条上一道横线。
    //
    // ⚠ 元素序必须取 `elem_hint()`（画在平面 / 色相条**之上**）：写死 `1` 时平面
    // （`elem = seq + 1`，大得多）会盖住标记 —— 表现就是"指针在色带后面"
    // （色相条上的标记只剩两侧露出的两个小角）。
    let cur = hsv_to_rgb(hsv[0], hsv[1], hsv[2], 1.0);
    let mark = Rect::new(
        sv_rect.x + hsv[1] * sv_rect.w - 3.0,
        sv_rect.y + (1.0 - hsv[2]) * sv_rect.h - 3.0,
        6.0,
        6.0,
    );
    ui.push_panel_like(
        mark,
        Color::TRANSPARENT,
        super::format::ink_on(cur),
        2.0,
        CornerRadius::all(3.0),
        ui.elem_hint(),
    );
    let hue_mark = Rect::new(
        hue_rect.x - 2.0,
        hue_rect.y + hsv[0] * hue_rect.h - 1.0,
        HUE_W + 4.0,
        3.0,
    );
    ui.push_panel_like(
        hue_mark,
        Color::TRANSPARENT,
        Color::WHITE,
        2.0,
        CornerRadius::all(1.5),
        ui.elem_hint(),
    );

    // ── ② 文本框（按模式呈现 + 自动识别格式 + 非法时的警告按钮） ──
    let text_id = format!("{id}::text");
    let text_abs = ui.id_for(text_id.as_str()).to_static();
    let text_focused = ui
        .state()
        .focused
        .as_ref()
        .is_some_and(|f| f.as_str() == text_abs.as_str());
    // 缓冲来源：外部传入优先，否则用**全局跨帧缓冲**（take/写回，避免与 `&mut Ui` 双借用）。
    let mut owned: Option<String> = None;
    if ext_buf.is_none() {
        let mut t = std::mem::take(&mut ui.state_mut().color_picker.text);
        if !text_focused || mode_changed {
            // 非聚焦时按当前颜色 + 当前模式重写；**切模式也重写**（呈现方式变了，文本框
            // 应立刻换成新格式，而不是留着上一种格式直到失焦）。
            // 其余聚焦情形保留用户输入（含不可解析的）。
            t = format_color(Color::from(c), mode, alpha);
        }
        owned = Some(t);
    } else if mode_changed {
        // 外部缓冲同样在切模式时重排。
        *ext_buf.as_deref_mut().expect("ext_buf 已判定为 Some") =
            format_color(Color::from(c), mode, alpha);
    }
    let buf: &mut String = match ext_buf.as_deref_mut() {
        Some(b) => b,
        None => owned.as_mut().expect("缺省缓冲已初始化"),
    };
    let invalid = parse_color(buf, mode).is_none();
    let warn_w = if invalid { input_h + GAP } else { 0.0 };
    let text_rect = Rect::new(PAD, y, (body_w - warn_w).max(1.0), input_h);
    ui.text_input_at(text_id.as_str(), text_rect, buf);
    if let Some(p) = parse_color(buf, mode) {
        // 未开 Alpha 行时保留原透明度（与 `parse_hex` 既有行为一致）。
        let mut parsed = p;
        if !alpha {
            rgba_keep_alpha(&mut parsed, Color::from(c));
        }
        c = rgba(parsed);
    }
    if invalid {
        // 警告按钮：按下 → 文本恢复成当前颜色的有效值（颜色本身一直没被动过）。
        let wrect = Rect::new(PAD + text_rect.w + GAP, y, input_h, input_h);
        // 警告按钮自己的 id（`hit_abs` 的硬约定：传**自己的绝对 id**，引擎按它做
        // 控件级遮挡——本按钮与文本框**相邻**不重叠，但一旦布局变化导致重叠，
        // 归属立刻正确，不需要再改这里）。
        let warn_abs = IdAbsolute::owned(format!("{id}::warn"));
        let wh = ui.hit_abs(&warn_abs, &wrect);
        let pal = ui.theme.palette;
        let danger = pal.danger;
        let bg = if wh {
            pal.surface_hover.into()
        } else {
            ui.theme.button.bg
        };
        // ⚠ 元素序同样取 `elem_hint()`：警告按钮画在文本框**之后**（文本框用
        // `elem = seq + 1`，写死 `1` 会被它压住）。
        ui.push_panel_like(
            wrect,
            bg,
            danger,
            1.0,
            CornerRadius::all(SWATCH_RADIUS),
            ui.elem_hint(),
        );
        ui.icon_at(
            Position::Physical(Vec2::new(
                wrect.x + (wrect.w - 14.0) * 0.5,
                wrect.y + (wrect.h - 14.0) * 0.5,
            )),
            Size::Physical(Vec2::splat(14.0)),
            Icon::Warning,
            danger,
        );
        if btn.down_edge() && wh {
            ui.claim_press();
            let restore = format_color(Color::from(c), mode, alpha);
            *buf = restore;
        }
    }
    // 全局缓冲写回（未聚焦时上面的重写已是最新；聚焦时保留编辑内容）。
    if ext_buf.is_none() {
        let t = owned.take().unwrap_or_default();
        ui.state_mut().color_picker.text = t;
    }
    y += input_h + GAP;

    // ── ④ 通道行：颜色滑块（渐变轨）+ NumberInput ──
    y += sv + GAP;
    let labels = ["R", "G", "B", "A"];
    let int_mode = mode.integer_channels();
    let lfs = ui.theme.label.font_size;
    let lfg = ui.theme.label.color;
    for i in 0..n_ch {
        let (range_max, val) = if int_mode {
            (255.0, (c[i] * 255.0).round())
        } else {
            (1.0, c[i])
        };
        // 标签
        ui.push_text_rect(
            Rect::new(PAD, y, LABEL_W, row),
            labels[i],
            lfs,
            lfg,
            None,
            TextAlign::Left,
            TextVAlign::Center,
            None,
            None,
        );
        // 颜色滑块：轨道 = 该通道 0 → 最大，**右端与数值框齐平**（中间不留缝：
        // 用户看到的就是"条缺了几个像素"——轨道短了一截，行看起来断成两节）。
        let style =
            channel_slider_style(&ui.theme, channel_color(c, i, 0.0), channel_color(c, i, 1.0));
        let slide_w = (body_w - LABEL_W - GAP - field_w).max(1.0);
        let srect = Rect::new(PAD + LABEL_W + GAP, y, slide_w, row);
        let sid = format!("ch{i}");
        let nv = ui.slider_at_styled(&sid, srect, 0.0..=range_max, val, 1.0, &style);
        if (nv - val).abs() > f32::EPSILON {
            let raw = if int_mode { nv.round() / 255.0 } else { nv };
            c[i] = raw.clamp(0.0, 1.0);
        }
        // 数值框（NumberInput：可输入、可拖手柄）；值变了才写回（u8 模式不静默量化）。
        let frect = Rect::new(PAD + body_w - field_w, y, field_w, input_h);
        let shown = if int_mode { (c[i] * 255.0).round() } else { c[i] };
        let mut nv2 = shown;
        let nid = format!("num{i}");
        let mut ni = crate::widgets::NumberInput::new(nid.as_str(), &mut nv2);
        ni = if int_mode {
            ni.range(0.0, 255.0).step(1.0)
        } else {
            ni.range(0.0, 1.0).step(0.01)
        };
        ui.add_at(Position::Physical(Vec2::new(frect.x, frect.y)), ni);
        if (nv2 - shown).abs() > f32::EPSILON {
            let raw = if int_mode { nv2.round() / 255.0 } else { nv2 };
            c[i] = raw.clamp(0.0, 1.0);
        }
        y += row;
    }

    // ── 记录 HSV 缓存 ──
    // 用户拖过 SV / 色相条 ⇒ 用我们手里的 hsv（保住色相）；其它来源改过颜色 ⇒ 重派生。
    let final_color = Color::from(c);
    let hsv_out = if hsv_dirty || final_color == color_in {
        hsv
    } else {
        rgb_to_hsv(final_color)
    };
    let st = &mut ui.state_mut().color_picker;
    st.hsv = hsv_out;
    st.hsv_src = final_color;
    final_color
}

/// 2D 拖拽区（SV 平面 / 色相条）的通用交互：按下即生效、拖拽中持续生效。
///
/// 自带拖拽语义 ⇒ 按下时 `claim_press`（阻止外层窗口/面板把这次按下当作拖拽基准）。
fn drag_region_active(ui: &mut Ui, id_abs: &IdAbsolute<'static>, rect: &Rect, btn: KeyState) -> bool {
    let hit = ui.hit_abs(id_abs, rect);
    if btn.down_edge() && hit {
        ui.claim_press();
    }
    let ws = ui.state_mut().widget(id_abs);
    update_drag(ws, hit, btn)
}

/// 通道滑块的渐变轨道色：只把第 `i` 个分量设成 `v`，其余保持（于是轨道就是
/// "该通道从 0 到最大"的颜色斜坡）。
#[inline]
fn channel_color(c: [f32; 4], i: usize, v: f32) -> Color {
    let mut d = c;
    d[i] = v;
    Color::from(d)
}

/// 通道滑块的样式：渐变轨道 + **透明填充**（填充会盖掉斜坡）。
fn channel_slider_style(theme: &crate::style::Theme, lo: Color, hi: Color) -> SliderStyle {
    SliderStyle {
        track: Brush::Horizontal(lo, hi),
        fill: Color::TRANSPARENT.into(),
        ..theme.slider.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_color_only_touches_its_own_component() {
        let c = [0.1, 0.2, 0.3, 0.4];
        let lo: [f32; 4] = channel_color(c, 1, 0.0).into();
        let hi: [f32; 4] = channel_color(c, 1, 1.0).into();
        assert_eq!([lo[0], lo[2], lo[3]], [0.1, 0.3, 0.4], "其它分量不得被改动");
        assert_eq!(lo[1], 0.0);
        assert_eq!(hi[1], 1.0);
        // alpha 行也用同一套（i = 3）。
        let a: [f32; 4] = channel_color(c, 3, 0.0).into();
        assert_eq!([a[0], a[1], a[2]], [0.1, 0.2, 0.3]);
        assert_eq!(a[3], 0.0);
    }

    #[test]
    fn channel_slider_style_is_a_gradient_with_transparent_fill() {
        let theme = crate::style::Theme::dark();
        let s = channel_slider_style(
            &theme,
            Color::rgba_u8(0, 40, 80, 255),
            Color::rgba_u8(255, 40, 80, 255),
        );
        assert!(s.track.as_solid().is_none(), "通道轨道必须是渐变（不是纯色）");
        assert_eq!(
            s.fill.as_solid().map(|c| { let a: [f32; 4] = c.into(); a[3] }),
            Some(0.0),
            "填充必须透明，否则会盖掉斜坡"
        );
        // 手柄 / 圆角等仍跟随主题。
        assert_eq!(s.handle, theme.slider.handle);
        assert_eq!(s.radius, theme.slider.radius);
    }
}
