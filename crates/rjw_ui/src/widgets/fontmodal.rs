//! **字体切换模态对话框**（组合控件，只依赖公开 API）。
//!
//! 布局：`modal_at_w` 固定宽对话框 → `Input`（输入字体名）+ **字重下拉** +
//! `PreviewInput`（用当前输入的名字 + 字重**实时预览**渲染示例文本，字体不存在回落
//! 默认）+ **确定 / 取消**（`PackSide::Left` 水平排列、`min_size` 撑开 spacer **右对齐**）。
//!
//! 确定 → `apply(字体名, 字重)`（demo 里 `theme.with_font_family(name)` +
//! `theme.with_font_weight(w)`）；取消 / Esc → 关闭。

use glam::Vec2;

use crate::ui::Ui;
use crate::{Child, PackSide, UiAdd, Weight};

/// 字重下拉的档位（CSS 常用九档里的七档；`fontdb`/`cosmic-text` 按最接近的字面回落）。
pub const FONT_WEIGHT_CHOICES: [Weight; 7] = [
    Weight::LIGHT,
    Weight::NORMAL,
    Weight::MEDIUM,
    Weight::SEMIBOLD,
    Weight::BOLD,
    Weight::EXTRA_BOLD,
    Weight::BLACK,
];

/// 字重的显示名（下拉项 + 触发按钮文本）。
pub fn weight_label(w: Weight) -> String {
    let name = match w {
        Weight::LIGHT => "细",
        Weight::NORMAL => "常规",
        Weight::MEDIUM => "中等",
        Weight::SEMIBOLD => "半粗",
        Weight::BOLD => "粗",
        Weight::EXTRA_BOLD => "特粗",
        Weight::BLACK => "黑",
        _ => "字重",
    };
    format!("{name} {}", w.0)
}

/// 字体切换模态对话框。
pub struct FontModal<'a> {
    /// 输入框内容（应用侧持有，跨帧持久；`trim()` 后为待应用字体名，空 = 系统默认）。
    pub input: &'a mut String,
    /// 字重下拉的**选中值**（应用侧持有，跨帧持久）——下拉项来自
    /// [`FONT_WEIGHT_CHOICES`]，选中索引持久于 `UiState.combo`。
    pub weight: &'a mut Weight,
    /// 确定回调（收到输入框内的字体名 + 下拉选中的字重）。
    pub apply: &'a mut dyn FnMut(&str, Weight),
}

impl FontModal<'_> {
    /// 显示对话框。`open` 为跨帧开关（本方法负责关闭：确定 / 取消 / Esc）。
    pub fn show(self, ui: &mut Ui, open: &mut bool) {
        if !*open {
            return;
        }
        // Esc 关闭
        if ui.key_down_edge(winit::keyboard::KeyCode::Escape) {
            *open = false;
            return;
        }
        // 主题/测量值先拷出（Copy / owned），闭包内不再借用 `ui`
        let (font_size, fg, gap) = {
            let t = &ui.theme;
            (t.input.font_size, t.input.fg, t.gap)
        };
        let width = 340.0_f32;
        let bsz = |ui: &mut Ui, s: &str| -> f32 {
            let t = ui.theme.button.clone();
            ui.text_size(s, t.font_size, t.font_family.as_deref()).x + t.padding.x * 2.0
        };
        let btn_w = bsz(ui, "确定") + bsz(ui, "取消") + gap;
        // 预览示例（字体不存在时 rjw_text 回落默认）
        let mut ok = false;
        let mut cancel = false;
        // 字重下拉：选中索引（`ui.combo` 的第 4 参 = 跨帧持久索引）。
        let weight_now = *self.weight;
        let weight_idx = FONT_WEIGHT_CHOICES.iter().position(|w| *w == weight_now).map(|i| i as u32);
        let choices: Vec<String> = FONT_WEIGHT_CHOICES.iter().map(|w| weight_label(*w)).collect();
        let mut picked_weight = None;
        ui.modal("font_modal").pos(Vec2::new(460.0, 220.0)).width(width).show(|m| {
            // 内容区宽 = 窗口**内容可用宽**——`avail_w()` 已扣除窗口内边距
            // （`Frame::fixed_avail_w = w − pad_total×2`，`pad_total = padding + border_w`），
            // **不要再减 padding**（旧实现重复扣减导致预览框/按钮右缘比内容区窄 2×padding，
            // 窗口缩窄时 `content_w − btn_w` 提前为负 → spacer 0 → 按钮溢出窗口）。
            // 窗口缩放柄改宽后 `avail_w()` 返回持久值，预览换行框与按钮右对齐随之跟随。
            let content_w = m.ui_mut().avail_w().unwrap_or(width);
            m.label("字体切换：输入字体名预览，确定生效（空 = 默认）");
            // Input：输入字体名
            m.text_input("font_modal_input", self.input);
            // 字重：下拉（触发按钮文字 = 当前档位）。
            if let Some(i) = m.combo("font_modal_weight", &weight_label(weight_now), &choices, weight_idx) {
                picked_weight = FONT_WEIGHT_CHOICES.get(i as usize).copied();
            }
            // PreviewInput：面板底 + 用当前输入的名字渲染示例文本，**按宽度换行、
            // 自动改大小**（超长字体名不裁剪，对话框随预览长高）。
            let name = self.input.trim().to_owned();
            let psize = font_size * 2.0;
            let example = format!("字体预览：Aa 中 123 {name}");
            let inner_w = (content_w - 12.0).max(0.0);
            let th = {
                let ui = m.ui_mut();
                let fam = (!name.is_empty()).then_some(name.as_str());
                ui.text_size_wrap(&example, psize, fam, inner_w).y
            };
            let pbox = m.ui_mut().child_rect(content_w, th + 12.0, Child::Expand);
            {
                let ui = m.ui_mut();
                let st = &ui.theme.input;
                ui.push_panel_like(pbox, st.bg, st.border, 1.0, 0.0, 1);
                // 换行排版缓冲（预览文本超宽自动换行，不裁剪）
                let buf = ui.wrap_buffer(
                    &example,
                    psize,
                    (!name.is_empty()).then_some(name.as_str()),
                    inner_w,
                );
                ui.push_text_rect(
                    rjw_transform::Rect::new(pbox.x + 6.0, pbox.y + 6.0, inner_w, th),
                    &example,
                    psize,
                    fg,
                    (!name.is_empty()).then(|| name.clone().into()),
                    crate::TextAlign::Left,
                    crate::draw::TextVAlign::Top,
                    None,
                    Some(buf),
                );
            }
            // 确定 / 取消：水平排列、右对齐（spacer 用 min_size 撑满剩余宽）。
            // ⚠ `row_y` 是窗口内容光标（**物理**），`pack_at` 收带单位 Position——
            // 必须显式 `Position::Physical`，否则默认 Logical 会被 ×scale 二次换算
            // （scale≠1 时按钮行跑到窗口外面）。
            let row_y = m.ui_mut().cursor_pos().y;
            let row = m.pack_at(
                crate::draw::Position::Physical(Vec2::new(0.0, row_y)),
                PackSide::Left,
                |row| {
                    row.min_size((content_w - btn_w).max(0.0), 0.0);
                    row.label("");
                    if row.button("font_modal_ok", "确定").clicked() {
                        ok = true;
                    }
                    if row.button("font_modal_cancel", "取消").clicked() {
                        cancel = true;
                    }
                },
            );
            // pack_at 不占父光标：占一个与行同高的子项，窗口高度自然结算
            m.ui_mut().child_rect(0.0, row.y, Child::Expand);
        });
        if ok {
            let name = self.input.trim().to_owned();
            // 下拉本帧选中的档位写回应用持久的字重（跨帧值），再一起交给回调——
            // 于是"只改字重、不改字体名"也是一次确定的提交。
            if let Some(w) = picked_weight {
                *self.weight = w;
            }
            (self.apply)(&name, *self.weight);
            *open = false;
        } else if cancel {
            *open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_choices_are_the_seven_ordered_steps_and_labeled_uniquely() {
        // 七档：细 / 常规 / 中等 / 半粗 / 粗 / 特粗 / 黑（CSS 常用九档去掉 100/200）。
        assert_eq!(FONT_WEIGHT_CHOICES.len(), 7);
        assert!(FONT_WEIGHT_CHOICES.contains(&Weight::NORMAL), "必须含常规 400");
        assert!(FONT_WEIGHT_CHOICES.contains(&Weight::BOLD), "必须含粗 700");
        assert!(
            FONT_WEIGHT_CHOICES.windows(2).all(|w| w[0] < w[1]),
            "下拉项必须**由细到粗**递增（选择行顺序 = 视觉顺序）"
        );
        // 显示名唯一（同名两项在下拉里分不清）。
        let names: Vec<String> = FONT_WEIGHT_CHOICES.iter().map(|w| weight_label(*w)).collect();
        let mut uniq = names.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), names.len(), "字重显示名不得重复：{names:?}");
        // 显示名带数值（应用/主题里排查时一眼对得上）。
        assert!(weight_label(Weight::NORMAL).contains("400"));
    }
}
