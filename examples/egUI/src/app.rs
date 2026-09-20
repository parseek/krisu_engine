pub mod global;

pub mod hellowindow;
pub mod base_information;
pub mod color_picker;

use std::{any::TypeId, collections::HashMap};

pub use rjw_krusie::{prelude::*};
// 菜单栏的 `width(..)` 收 `Size`（物理 / 逻辑像素）——prelude 里没有，显式引一下。
use rjw_krusie::ui::Size;

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

        // **菜单栏 = 一行 + 全宽背景**：`width(..)` 让背景铺满整个窗口宽度（不调 = 自然宽，
        // 就只有几个触发器、看不出"栏"）。`Size::Physical` 与 `window_physical_size` 同单位。
        let screen_w = ui.window_physical_size().0 as f32;
        ui.menu_bar("menu", vec2(0., 0.), |ui| {
            ui.width(Size::Physical(Vec2::new(screen_w, 0.0)));
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