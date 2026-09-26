use rjw_krusie::prelude::*;

use crate::app;

#[derive(Default)]
pub struct HelloWindow;

const TITLE: &str = "HelloWindow";

impl app::Demo for HelloWindow {
    fn id(&self) -> &'static str {
        TITLE
    }

    fn demo(&mut self, ui: &mut app::Ui, enable: &mut bool, global: &mut app::global::GlobalData) {
        super::demo_window(ui, std::any::type_name::<Self>(), TITLE, enable)
        .show(|ui| {
            ui.label("Hello world!!!");
            ui.label("(●'◡'●)");

            if !global.your_name.is_empty() {
                ui.label(&global.your_name);
            }
        });
    }
}