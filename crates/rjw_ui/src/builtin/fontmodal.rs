//! **字体切换模态对话框**（组合控件，只依赖公开 API）。
//!
//! 布局：`modal_at_w` 固定宽对话框 → `Input`（输入字体名）+ `PreviewInput`（用当前
//! 输入的名字**实时预览**渲染示例文本，字体不存在回落默认）+ **确定 / 取消**
//! （`PackSide::Left` 水平排列、`min_size` 撑开 spacer **右对齐**）。
//!
//! 确定 → `apply(字体名)`（demo 里 `theme.with_font_family(name)`）；取消 / Esc → 关闭。

use glam::Vec2;

use crate::ui::Ui;
use crate::{PackSide, UiAdd};

/// 字体切换模态对话框。
pub struct FontModal<'a> {
    /// 输入框内容（应用侧持有，跨帧持久；`trim()` 后为待应用字体名，空 = 系统默认）。
    pub input: &'a mut String,
    /// 确定回调（收到输入框内的字体名）。
    pub apply: &'a mut dyn FnMut(&str),
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
        let (font_size, fg, pad, gap) = {
            let t = &ui.theme;
            (t.input.font_size, t.input.fg, t.panel.padding, t.gap)
        };
        let width = 340.0_f32;
        let content_w = (width - pad * 2.0).max(0.0);
        let bsz = |ui: &mut Ui, s: &str| -> f32 {
            let t = ui.theme.button.clone();
            ui.text_size(s, t.font_size, t.font_family.as_deref()).x + t.padding.x * 2.0
        };
        let btn_w = bsz(ui, "确定") + bsz(ui, "取消") + gap;
        // 预览示例（字体不存在时 rjw_text 回落默认）
        let mut ok = false;
        let mut cancel = false;
        ui.modal_at_w("font_modal", Vec2::new(460.0, 220.0), width, |m| {
            m.label("字体切换：输入字体名预览，确定生效（空 = 默认）");
            // Input：输入字体名
            m.text_input("font_modal_input", self.input);
            // PreviewInput：面板底 + 用当前输入的名字渲染示例文本，**按宽度换行、
            // 自动改大小**（超长字体名不裁剪，对话框随预览长高）。
            let name = self.input.trim().to_owned();
            let psize = font_size * 2.0;
            let example = format!("字体预览：Aa 中 123 {name}");
            let inner_w = (content_w - 12.0).max(0.0);
            let th = {
                let ui = m.ui_mut();
                let fam = (!name.is_empty()).then(|| name.as_str());
                ui.text_size_wrap(&example, psize, fam, inner_w).y
            };
            let pbox = m.ui_mut().child_rect(content_w, th + 12.0);
            {
                let ui = m.ui_mut();
                let st = &ui.theme.input;
                ui.push_panel_like(pbox, st.bg, st.border, 1.0, 0.0, 1);
                // 换行排版缓冲（预览文本超宽自动换行，不裁剪）
                let buf = ui.wrap_buffer(
                    &example,
                    psize,
                    (!name.is_empty()).then(|| name.as_str()),
                    inner_w,
                );
                ui.push_text_rect(
                    rjw_transform::Rect::new(pbox.x + 6.0, pbox.y + 6.0, inner_w, th),
                    &example,
                    psize,
                    fg,
                    (!name.is_empty()).then(|| name.clone()),
                    crate::TextAlign::Left,
                    crate::draw::TextVAlign::Top,
                    None,
                    Some(buf),
                );
            }
            // 确定 / 取消：水平排列、右对齐（spacer 用 min_size 撑满剩余宽）
            let row_y = m.ui_mut().cursor_pos().y;
            let row = m.pack_at(Vec2::new(0.0, row_y), PackSide::Left, |row| {
                row.min_size((content_w - btn_w).max(0.0), 0.0);
                row.label("");
                if row.button("font_modal_ok", "确定").clicked() {
                    ok = true;
                }
                if row.button("font_modal_cancel", "取消").clicked() {
                    cancel = true;
                }
            });
            // pack_at 不占父光标：占一个与行同高的子项，窗口高度自然结算
            m.ui_mut().child_rect(0.0, row.y);
        });
        if ok {
            let name = self.input.trim().to_owned();
            (self.apply)(&name);
            *open = false;
        } else if cancel {
            *open = false;
        }
    }
}
