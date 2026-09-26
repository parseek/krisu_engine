use rjw_krusie::{prelude::*};

use crate::app;

#[derive(Default)]
pub struct BaseInformation {
    show_fps: bool,
}

const TITLE: &str = "BaseInformation 基本信息";

impl app::Demo for BaseInformation {
    fn id(&self) -> &'static str {
        TITLE
    }

    fn demo(&mut self, ui: &mut app::Ui, enable: &mut bool, global: &mut app::global::GlobalData) {
        let viewport_size = ui.viewport_size();

        super::demo_window(ui, std::any::type_name::<Self>(), TITLE, enable)
        .show(|ui| {
            ui.label(&format!("窗口大小: {:?}", viewport_size));
            ui.row(|ui| {
                ui.label("显示FPS：");
                ui.checkbox("checkbox_fps", "", self.show_fps).toggled().then(|| {self.show_fps = !self.show_fps});
                if self.show_fps {
                    ui.label(&format!("FPS: {:.2}", global.fps));
                }
            });
            ui.row(|ui| {
                ui.label("你的名字：");
                ui.text_input("ti", &mut global.your_name);
            });
        });
    }
}