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

/// 字重下拉的档位（CSS 常用九档里的九档；`fontdb`/`cosmic-text` 按最接近的字面回落）。
pub const FONT_WEIGHT_CHOICES: [Weight; 9] = [
    Weight::THIN,
    Weight::EXTRA_LIGHT,
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
        Weight::THIN => "超细",
        Weight::EXTRA_LIGHT => "特细",
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
    /// 字重下拉的**草稿值**（应用侧持有，跨帧持久）——下拉项来自
    /// [`FONT_WEIGHT_CHOICES`]。**选中那一帧就写回这里**（不是等"确定"），
    /// 所以应用应当把它与"**已应用**字重"分开：
    /// - 打开对话框时把"已应用值"拷进草稿；
    /// - `确定` 时 `apply(name, 草稿)` 提交（应用在回调里落到"已应用值"）；
    /// - `取消` 直接丢弃草稿（下次打开再拷一次）。
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
            let t = ui.theme();
            (t.input.font_size, t.input.fg, t.gap)
        };
        let width = 340.0_f32;
        let bsz = |ui: &mut Ui, s: &str| -> f32 {
            let t = ui.theme().button.clone();
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
            //
            // ⚠ **选中的那一帧就写回** `*self.weight`，不能攒到点"确定"再用：
            // 下拉是"点一次就收起"的控件（`MenuCtx::item*` 点了即关浮层），而"确定"多半在
            // 之后的另一帧 —— 攒在局部变量里的选择在下一帧就被重置了，症状正是
            // 用户报的"**选不中任何其他字重**"（勾/触发文字永远回到旧档位）。
            // `weight` 是**草稿**：确定才 `apply`，取消就丢弃（应用侧负责把草稿重置）。
            if let Some(i) = m.combo("font_modal_weight", &weight_label(weight_now), &choices, weight_idx)
                && let Some(w) = FONT_WEIGHT_CHOICES.get(i as usize)
            {
                *self.weight = *w;
            }
            // PreviewInput：面板底 + 用当前输入的名字渲染示例文本，**按宽度换行、
            // 自动改大小**（超长字体名不裁剪，对话框随预览长高）。
            //
            // ⚠ 预览文本**不含字体名**：名字就在上面的输入框里，再拼进预览等于把同一串
            // 文本画两遍（用户实测："文本输入重复"）。这里只放固定样本。
            let name = self.input.trim().to_owned();
            let psize = font_size * 2.0;
            let example = "字体预览：Aa 中 123".to_owned();
            let inner_w = (content_w - 12.0).max(0.0);
            // ⚠⚠ **预览的字重 = 下拉草稿**（`所见即所选`）：字重在引擎里是**主题令牌**
            // （`Theme::font_weight`，它是排版输入：既改字形也改步进宽度），没有 per-text
            // 字重参数 —— 所以这里**临时把主题字重换成草稿**，预览测高（`text_size_wrap`）
            // 与绘制（`wrap_buffer` + `push_text_rect`）**必须用同一个值**（否则框高与内容
            // 不一致），画完**立刻还原**（别让本帧后面的段落继承对话框的草稿）。
            //
            // 历史 bug：预览只跟随**已应用**字重 ⇒ 在下拉里选"粗 700"预览纹丝不动
            // （用户实测："字重无法在预览中体现"）。
            let saved_weight = m.ui_mut().theme.font_weight;
            m.ui_mut().theme.font_weight = *self.weight;
            let th = {
                let ui = m.ui_mut();
                let fam = (!name.is_empty()).then_some(name.as_str());
                ui.text_size_wrap(&example, psize, fam, inner_w).y
            };
            if std::env::var_os("RJ_FONT_TRACE").is_some() {
                let (w, fs) = {
                    let ui = m.ui_mut();
                    let fam = (!name.is_empty()).then_some(name.as_str());
                    (
                        ui.text_size(&example, psize, fam).x,
                        ui.theme().font_weight.0,
                    )
                };
                eprintln!(
                    "font[preview] weight={fs}（草稿）size={psize} 样本宽={w:.1} 折行宽={inner_w:.0} 高={th:.1}"
                );
            }
            let pbox = m.ui_mut().child_rect(content_w, th + 12.0, Child::Expand);
            {
                let ui = m.ui_mut();
                let st = &ui.theme().input;
                ui.push_panel_like(pbox, st.bg, st.border, 1.0, 0.0, 1);
                // 换行排版缓冲（预览文本超宽自动换行，不裁剪）——缓存键含**主题字重**
                // （= 上面的草稿）⇒ 换档位必得另一份缓冲，不会拿到旧字重的排版。
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
            // 预览画完：**还原主题字重**（后面录的按钮行等仍是"已应用"字重）。
            m.ui_mut().theme.font_weight = saved_weight;
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
            // 下拉选中的档位已**在选中那一帧**写回 `*self.weight`（草稿）——这里把它与
            // 字体名一起交给回调提交（"只改字重、不改字体名"也是一次确定的提交）。
            (self.apply)(&name, *self.weight);
            *open = false;
        } else if cancel {
            // **取消 = 丢弃草稿**：`weight` 由应用侧在下次打开时从"已应用值"重置
            // （示例里 `font_weight_draft` 与 `font_weight` 分开，见 `TopBar`）。
            *open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_choices_are_the_nine_ordered_steps_and_labeled_uniquely() {
        // 九档：超细 100 / 特细 200 / 细 300 / 常规 400 / 中等 500 / 半粗 600 / 粗 700 /
        // 特粗 800 / 黑 900（CSS 常用九档；`fontdb`/`cosmic-text` 按最接近的字面回落）。
        assert_eq!(FONT_WEIGHT_CHOICES.len(), 9);
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
