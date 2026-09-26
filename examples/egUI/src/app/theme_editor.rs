use rjw_krusie::prelude::*;

use crate::app;

#[derive(Default)]
pub struct ThemeEditor {
    open_palette: bool,
}

const TITLE: &str = "ThemeEditor 主题编辑器";

impl app::Demo for ThemeEditor {
    fn id(&self) -> &'static str {
        TITLE
    }

    fn demo(&mut self, ui: &mut app::Ui, enable: &mut bool, global: &mut app::global::GlobalData) {
        let theme = &mut global.theme;
        super::demo_window(ui, std::any::type_name::<Self>(), TITLE, enable)
        .show(|ui| {
            ui.checkbox("checkbox_palette", "打开调色板", self.open_palette).toggled().then(|| {self.open_palette = !self.open_palette});
        });

        if self.open_palette {
            let mut pal = theme.palette;
            super::demo_window(ui, concat!("ThemeEditor", "::Palette"), "调色板", &mut self.open_palette)
            .vscroll(true)
            .hscroll(true) // 为 false 时会突出而不是限制
            .resize(true)
//          .width(200.0)
            // **`.height(..)` = 有界视口**：不写它时"视口高 = 屏幕剩下的高"——21 行内容
            // 一打开就把窗口撑到屏幕底（"最开始打开调色板编辑器时仍然会把高度撑到窗口底端，
            // 直到手动拉开大小"）。拖过之后由用户接管（持久于 `UiState::window_heights`），
            // 所以这只是**初始值**。
//          .height(340.0)
            .show(|ui| {
                for (color, name) in [
                    (&mut pal.surface_dim, "最底表面"),
                    (&mut pal.surface_sunken, "凹陷表面"),
                    (&mut pal.surface, "面板表面"),
                    (&mut pal.surface_raised, "抬升表面"),
                    (&mut pal.surface_overlay, "浮层表面"),
                    (&mut pal.surface_hover, "悬停表面"),
                    (&mut pal.surface_active, "激活表面"),
                    (&mut pal.border, "常规描边"),
                    (&mut pal.border_strong, "强描边"),
                    (&mut pal.text, "正文"),
                    (&mut pal.text_muted, "次级文字"),
                    (&mut pal.text_dim, "极弱文字"),
                    (&mut pal.accent, "强调色"),
                    (&mut pal.accent_hover, "强调色悬停"),
                    (&mut pal.accent_active, "强调色激活"),
                    (&mut pal.selection, "文本选择背景"),
                    (&mut pal.danger, "危险色"),
                    (&mut pal.handle, "高亮前景"),
                    (&mut pal.debug_outline, "调试描边"),
                    (&mut pal.scrim, "模态遮罩"),
                    (&mut pal.shadow, "投影"),
                ] {
                    ui.row(|ui| {
                        ui.label(&format!("{name}："));
                        ui.add(ColorPicker::new(name, color).alpha(true));
                    });
                }
            });
            theme.set_palette(&pal);
        }
    }
}