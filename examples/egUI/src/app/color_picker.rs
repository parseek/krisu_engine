use rjw_krusie::{prelude::*, ui::widgets};

use crate::app;

#[derive(Default)]
pub struct ColorPicker {
    color: Color,   
}

const TITLE: &str = "ColorPicker 颜色选择器";

impl app::Demo for ColorPicker {
    fn id(&self) -> &'static str {
        TITLE
    }

    fn demo(&mut self, ui: &mut app::Ui, enable: &mut bool, _global: &mut app::global::GlobalData) {
        super::demo_window(ui, std::any::type_name::<Self>(), TITLE, enable)
        .show(|ui| {
            ui.add(widgets::ColorPicker::new("cp", &mut self.color));
        });
    }
}