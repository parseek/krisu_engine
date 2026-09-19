pub mod global;

pub mod hellowindow;
pub mod base_information;
pub mod color_picker;

use std::{any::TypeId, collections::HashMap};

pub use rjw_krusie::{prelude::*};

pub trait Demo {
    fn id(&self) -> &str {
        std::any::type_name::<Self>()
    }
    fn demo(&mut self, ui: &mut Ui, enable: &mut bool, global: &mut global::GlobalData);
}

#[derive(Default)]
pub struct RJWApp {
    demos: HashMap<TypeId, (bool, Box<dyn Demo>)>,
    global: global::GlobalData,
}

impl RJWApp {
    pub fn register_demo<T: Demo + 'static>(&mut self, demo: T) {
        self.demos.insert(TypeId::of::<T>(), (false, Box::new(demo)));
    }
}

impl App for RJWApp {
    fn update(&mut self, ctx: &mut Ctx) {
        let Some(mut frame) = ctx.frame() else { return; };

        let mut ui = frame.ui(Theme::dark().with_font_family("Sarasa Mono SC").with_border_w(0.));

        ui.label_at(vec2(0., 40.), "↑点击选择要看的窗口");
        
        ui.menu_bar("menu", vec2(0., 0.), |ui| {
            ui.menu("窗口", |ui| {
                for (visible, demo) in self.demos.values_mut() {
                    ui.item_checked(demo.id(), visible);
                }
            });
        });

        for (visible, demo) in self.demos.values_mut() {
            if *visible {
                demo.demo(&mut ui, visible, &mut self.global);
            }
        }
    }
}