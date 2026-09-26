pub mod global;

pub mod hellowindow;
pub mod base_information;
pub mod color_picker;
pub mod gallery;
pub mod theme_editor;

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
    pub global: global::GlobalData,
    /// `--ui-dump`：每帧录制完把引擎状态（`Ui::debug_dump`）打到 stderr。
    pub ui_dump: bool,
    /// `--sim-fold`：脚本化自证折叠语义（注册的是 `Gallery::new_sim_fold()`；
    /// 判据是**引擎状态**里正文控件的绝对 ID 在不在，见 `gallery.rs` 的 `sim_id_in_window`）。
    pub sim_fold: bool,
    /// `--sim-fold`：本进程的帧计数（`Frame` 不暴露帧号 ⇒ 自己数；注入时序用）。
    pub sim_frame: u64,
    /// `--fold-style`：给**区块标题行**换一套观感演示（`FoldableStyle::button_like`）+
    /// 打开正文上下端的**渐隐提示**（`with_body_fade`）—— 用于目视核对"类按钮"与"阴影渐隐"。
    pub fold_style: bool,
}

impl RJWApp {
    pub fn register_demo<T: Demo + 'static>(&mut self, enable: bool, demo: T) {
        self.demos.insert(TypeId::of::<T>(), (enable, Box::new(demo)));
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

        self.global.fps = frame.fps();

        // ── `--sim-fold`：脚本化注入鼠标 ─────────────────────────────────────────
        // 注入点由录制端**运行时解算**（窗口原点 + 区块标题行中心，见 `gallery::sim_store_point`）
        // —— 窗口位置是引擎自动级联给的，写死会随主题 / 布局漂移。
        //
        // ⚠ 字段顺序就是时序：本单元在 `frame.ui(..)` **之前**，而 ui 的输入快照是在
        // `UiInit::build`（开段）时冻结的 —— 开段发生在 `frame.ui(..)` 里，所以这里注入的
        // 边沿**当帧**就能被标题行看到。会话的第一段之外还有一段（帧末收尾）不走这里。
        if self.sim_fold {
            self.sim_frame += 1;
            let f = self.sim_frame;
            // 每一轮 = 两次注入：`t` 按下、`t+1` 释放（`toggle_folded` 只在**按下边沿**触发，
            // 同一次按下不会翻两次）。
            for (i, t) in [(0usize, 50u64), (1, 60), (2, 70), (0, 80)] {
                if (f == t || f == t + 1) && let Some(p) = gallery::sim_fold_point(i) {
                    frame.debug_inject_mouse(p, f == t);
                }
            }
        }

        let mut ui = frame.ui(self.global.theme.clone());

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

pub fn demo_window<'ui, 'a>(
    ui: &'ui mut Ui<'a>,
    id: &'ui str,
    title: &'ui str,
    enable: &'ui mut bool,
) -> rjw_krusie::ui::WindowBuilder<'ui, 'a> {
    ui.window(id)
        .title(title)
        .close_button(enable)
        .collapsible(true, None)
        // .resize Horizontal 不在设置 width 的情况下无法缩放；Vertical 缩放幽灵控件问题；
        // 可显式引入是否允许裁切（引入 .vscroll(enum) 和 .hscroll(enum)，enum { NoClip, ClipOnly, Scroll }，就像 egui 那样，NoClip 下的大小必须能够呈现所有内容）；
        // BUG：row 容器似乎不会“撑开”宽度

        // ColorPicker 位置有时出现问题
}