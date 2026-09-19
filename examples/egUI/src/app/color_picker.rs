use rjw_krusie::{prelude::*, ui::widgets};

use crate::app;

#[derive(Default)]
pub struct ColorPicker {
    color: Color,   
}

const TITLE: &str = "ColorPicker 颜色选择器";

impl app::Demo for ColorPicker {
    fn id(&self) -> &str {
        TITLE
    }

    fn demo(&mut self, ui: &mut app::Ui, enable: &mut bool, _global: &mut app::global::GlobalData) {
        ui.window(std::any::type_name::<Self>())
        .title(TITLE)
        .close_button(enable)
        .show(|ui| {
            ui.add(widgets::ColorPicker::new("cp", &mut self.color));
        });
    }
}