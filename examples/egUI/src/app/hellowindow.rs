use rjw_krusie::prelude::*;

use crate::app;

#[derive(Default)]
pub struct HelloWindow;

const TITLE: &str = "HelloWindow";

impl app::Demo for HelloWindow {
    fn id(&self) -> &str {
        TITLE
    }

    fn demo(&mut self, ui: &mut app::Ui, enable: &mut bool, global: &mut app::global::GlobalData) {
        ui.window(std::any::type_name::<Self>())
        .title(TITLE)
        .close_button(enable)
        .show(|ui| {
            ui.label("Hello world!!!");
            ui.label("(●'◡'●)");

            if !global.your_name.is_empty() {
                ui.label(&global.your_name);
            }
        });
    }
}