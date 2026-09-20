pub mod global;

pub mod hellowindow;
pub mod base_information;
pub mod color_picker;
pub mod gallery;

use std::{any::TypeId, collections::HashMap};

pub use rjw_krusie::{prelude::*};
// 菜单栏的 `width(..)` 收 `Size`（物理 / 逻辑像素）——prelude 里没有，显式引一下。
use rjw_krusie::ui::Size;

pub trait Demo {
    /// 菜单里显示的名字（`--demo` 的第一个匹配关键字）。
    fn id(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
    /// **完整类型名**（= 窗口 id，见 `demo()` 里的 `std::any::type_name::<Self>()`）。
    /// `--demo` 的第二个匹配关键字：`id()` 是给人看的中文名时，靠它也能按类型名选。
    fn type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
    fn demo(&mut self, ui: &mut Ui, enable: &mut bool, global: &mut global::GlobalData);
}

#[derive(Default)]
pub struct RJWApp {
    demos: HashMap<TypeId, (bool, Box<dyn Demo>)>,
    global: global::GlobalData,
    /// `--ui-dump`：每帧录制完把引擎状态（`Ui::debug_dump`）打到 stderr。
    pub ui_dump: bool,
}

impl RJWApp {
    pub fn register_demo<T: Demo + 'static>(&mut self, demo: T) {
        self.demos.insert(TypeId::of::<T>(), (false, Box::new(demo)));
    }

    /// `--demo <子串>`：把 **`id()` 或类型名**含 `needle`（不区分大小写）的那个 demo
    /// 置为可见，返回 `(id, 类型名)`。
    ///
    /// `None` = 没有匹配 ⇒ 调用方负责报错 + 非 0 退出（**绝不静默忽略**：打错的
    /// 参数会让进程"跑完了但什么都没验证"）。多个匹配时取第一个（`HashMap` 序不定
    /// ⇒ 需要确定性时用完整类型名）。
    pub fn show_demo(&mut self, needle: &str) -> Option<(&'static str, &'static str)> {
        let needle = needle.to_lowercase();
        for (visible, demo) in self.demos.values_mut() {
            let hit = [demo.id(), demo.type_name()]
                .iter()
                .any(|s| s.to_lowercase().contains(&needle));
            if hit {
                *visible = true;
                return Some((demo.id(), demo.type_name()));
            }
        }
        None
    }

    /// 已注册 demo 的 `id()`（排序；`--demo` 打错时的提示清单）。
    pub fn demo_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.demos.values().map(|(_, d)| d.id()).collect();
        names.sort_unstable();
        names
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
            if !self.global.your_name.is_empty() {
                ui.separator_v();
                ui.label(&format!("你好(≧∇≦)ﾉ，{}！", self.global.your_name));
            }
        });

        for (visible, demo) in self.demos.values_mut() {
            if *visible {
                demo.demo(&mut ui, visible, &mut self.global);
            }
        }

        // `--ui-dump`：必须在**录制期**（`finish` 前）调 —— `win_origins` 是帧内状态。
        if self.ui_dump {
            eprintln!("[ui] {}", ui.debug_dump());
        }
    }
}