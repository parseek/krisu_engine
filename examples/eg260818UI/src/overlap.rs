//! 「重叠控件」演示 + **控件级遮挡**的自证（`--sim-overlap`）。
//!
//! 语义（用户手绘标注）：**灰 = 不会被触发，蓝 = 会被触发**。屏幕上两个控件重叠时
//! **只有画在上面（后录制）的那个**能收到点击 / 悬停——重叠处点一下**不会两个一起响应**。
//! 这条规则由引擎在 `Ui::hit_abs` 里统一实施（`rjw_ui::hit::widget_occluded`：
//! 按控件**绝对 id** 区分身份、按**录制次序**比较层级，判定输入是上一帧登记的区域表），
//! 见 `docs/ENGINE_GUIDE.md` §18.5「点击穿透（控件遮挡）」与 `widgets` 模块的硬约定 7。
//!
//! 本模块提供**可脚本化验证**的一对探针：两个固定矩形**故意重叠**，各自统计被点击次数。
//! `--sim-overlap` 在重叠区中心按下 + 释放，期望 `下层 = 0 / 上层 = 1`（修复前是
//! `1 / 1`：两个控件被一起触发）。坐标由 [`OverlapDemo::rects`] / [`OverlapDemo::overlap_point`]
//! 解算（**不写死**），故脚本与绘制永远对得上。

use rjw_krusie::prelude::*;
use rjw_krusie::ui::draw::TextVAlign;
use rjw_krusie::ui::{Position, Response, Sense, TextAlign, Widget};

/// 探针尺寸（**物理像素**，固定值 ⇒ 脚本算得出重叠区）。
fn probe_size() -> Vec2 {
    Vec2::new(220.0, 24.0)
}

/// 下层探针位置（物理像素，屏幕坐标；放在演示窗口全部下方 ⇒ 不与任何窗口竞争）。
fn below_pos() -> Vec2 {
    Vec2::new(24.0, 1000.0)
}

/// 上层探针位置：**故意**与下层重叠（右移 + 下移各 12~20px）。
fn above_pos() -> Vec2 {
    Vec2::new(124.0, 1014.0)
}

/// 灰 = 不会被触发（重叠处它被上层挡住）。
const GREY: Color = Color::rgba_u8(122, 122, 122, 255);
/// 蓝 = 会被触发（重叠处只有它响应）。
const BLUE: Color = Color::rgba_u8(0, 162, 232, 255);

/// 重叠控件演示（状态 = 两个探针各自被点击的次数）。
#[derive(Default)]
pub struct OverlapDemo {
    /// 下层探针被触发的次数（重叠处点击后期望仍为 **0**）。
    pub below_clicks: u32,
    /// 上层探针被触发的次数（重叠处点击后期望为 **1**）。
    pub above_clicks: u32,
}

impl OverlapDemo {
    /// 两个探针的**屏幕矩形**（物理像素）——绘制、脚本化点击、断言共用同一份解算。
    pub fn rects() -> (Rect, Rect) {
        let s = probe_size();
        let b = below_pos();
        let a = above_pos();
        (
            Rect::new(b.x, b.y, s.x, s.y),
            Rect::new(a.x, a.y, s.x, s.y),
        )
    }

    /// 重叠区**中心**（物理像素）：两个探针都覆盖它 ⇒ 点这里才检验"谁赢"。
    pub fn overlap_point() -> Vec2 {
        let (below, above) = Self::rects();
        let x0 = below.x.max(above.x);
        let x1 = (below.x + below.w).min(above.x + above.w);
        let y0 = below.y.max(above.y);
        let y1 = (below.y + below.h).min(above.y + above.h);
        Vec2::new((x0 + x1) * 0.5, (y0 + y1) * 0.5)
    }

    /// 录制两个重叠探针（**先下层、后上层** ⇒ 上层画在上面、也只有它会被触发）。
    pub fn ui(&mut self, ui: &mut Ui) {
        ui.add_at(
            Position::Physical(below_pos()),
            HitProbe {
                id: "ov_below",
                label: "下层控件（灰：重叠处不会被触发）",
                idle: GREY,
                hot: Color::rgba_u8(152, 152, 152, 255),
                hits: &mut self.below_clicks,
            },
        );
        ui.add_at(
            Position::Physical(above_pos()),
            HitProbe {
                id: "ov_above",
                label: "上层控件（蓝：重叠处只有它被触发）",
                idle: BLUE,
                hot: Color::rgba_u8(64, 190, 255, 255),
                hits: &mut self.above_clicks,
            },
        );
    }
}

/// 固定尺寸的可点击探针（**最小自定义控件**：命中 + 跨帧状态 + 自绘）。
struct HitProbe<'a> {
    id: &'a str,
    label: &'a str,
    idle: Color,
    hot: Color,
    hits: &'a mut u32,
}

impl Widget for HitProbe<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        // ① 申请 + 收交互（`Sense::CLICK`：命中 / 跨帧状态机一句话；探针不认领按下，
        //    所以不用 `Sense::DRAG`——外层窗口仍可拖）。
        let (rect, resp) = ui.allocate_sense(self.id, probe_size(), Sense::CLICK);
        // 被裁剪层完全剔除 ⇒ 直接 return（不镶嵌、不入段）。
        if resp.culled {
            return resp;
        }
        // 排障开关（`RJ_OVERLAP_TRACE=1`）：打印两个探针每帧的矩形 / 鼠标 / 命中 /
        // 本帧被控件级遮挡拦下的次数——"为什么这个控件不响应"最快的一条线索。
        if std::env::var_os("RJ_OVERLAP_TRACE").is_some() {
            let st = ui.state();
            if (18..=26).contains(&st.frame) {
                eprintln!(
                    "  probe {}: frame={} rect=({},{},{},{}) mouse=({},{}) hit={} blocked={}",
                    self.id,
                    st.frame,
                    rect.x,
                    rect.y,
                    rect.w,
                    rect.h,
                    ui.mouse_local().x,
                    ui.mouse_local().y,
                    resp.hovered,
                    st.widget_occluded_hits(),
                );
            }
        }
        if resp.clicked {
            *self.hits += 1;
        }
        // ② 自绘：可交互时用亮一档的颜色（灰/蓝的语义之外再给一点反馈）。
        let bg = if resp.hovered || resp.pressed { self.hot } else { self.idle };
        let p = ui.painter();
        p.panel(rect, bg, Color::rgba_u8(20, 24, 32, 255), 1.0, CornerRadius::all(6.0));
        p.text(
            rect,
            self.label,
            12.0,
            Color::rgba_u8(255, 255, 255, 255),
            None,
            TextAlign::Center,
            TextVAlign::Center,
            None,
            None,
        );
        resp
    }
}
