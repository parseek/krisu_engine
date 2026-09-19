use rjw_krusie::{prelude::*};

use crate::app;

#[derive(Default)]
pub struct BaseInformation;

const TITLE: &str = "BaseInformation 基本信息";

impl app::Demo for BaseInformation {
    fn id(&self) -> &str {
        TITLE
    }

    fn demo(&mut self, ui: &mut app::Ui, enable: &mut bool, global: &mut app::global::GlobalData) {
        let viewport_size = ui.viewport_size();

        ui.window(std::any::type_name::<Self>())
        .title(TITLE)
        .close_button(enable)
        .show(|ui| {
            ui.label(&format!("窗口大小: {:?}", viewport_size));
            ui.row(|ui| {
                ui.label("你的名字：");
                ui.text_input("ti", &mut global.your_name);
            });
        });
    }
}