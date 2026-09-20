use std::ops::Not;

// `TextEditor` 现在在 prelude 里（与 `Segmented` 不同——后者仍要显式引 `widgets`）。
use rjw_krusie::{prelude::*, ui::widgets};

use crate::app;

#[derive(Default)]
pub struct Gallery {
    input_single_line: String,
    input_multi_line: String,
    integer: i32,
    boolean: bool,
}

const TITLE: &str = "Gallery 窗口控件展示";

impl app::Demo for Gallery {
    fn id(&self) -> &'static str {
        TITLE
    }

    fn demo(&mut self, ui: &mut app::Ui, enable: &mut bool, _global: &mut app::global::GlobalData) {
        let Self {
            input_single_line,
            input_multi_line,
            integer,
            boolean,
        } = self;

        ui.window(std::any::type_name::<Self>())
        .title(TITLE)
        .close_button(enable)
        .collapsible(true, None)
        .show(|ui| {
            ui.label("这是一个Label");
            ui.divider();
            ui.row(|ui| {
                ui.label("按钮：");
                if ui.button("btn1", "点我 + 1").clicked() {
                    *integer = integer.overflowing_add(1).0;
                }
                if ui.button("btn2", "点我 + 10").clicked() {
                    *integer = integer.overflowing_add(10).0;
                }
            });
            ui.row(|ui| {
                ui.label("数字输入：");
                ui.add(NumberInput::new("number_input", integer));
            });
            ui.row(|ui| {
                ui.label("单行输入框：");
                ui.text_input("text_sl", input_single_line);
            }); // ⚠ row 把子项钉到"一行标准高"（`Theme::row_h`）——多行控件想撑高整行见 row Builder
            ui.label("多行输入框：");
            // 缩放：默认下限 = 一行文字高（拖不到 0）；自动申请会先问尺寸责任链
            // ⇒ 拖大后窗口与**下面的控件**跟着长；缩放柄形状取 `Theme::input.grip`（默认三条横线）。
            ui.add(TextEditor::new("text_ml", input_multi_line).multiline().resize(Resize::Both));
            ui.row(|ui| {
                ui.label("复选框：");
                ui.checkbox("checkbox", "", *boolean).toggled().then(|| {*boolean = boolean.not()});
            });
            ui.row(|ui| {
                ui.label("分段按钮：");
                let mut idx = !*boolean as usize;
                ui.add(widgets::Segmented::new("seg", &["是", "否"], &mut idx));
                *boolean = idx == 0;
            });
        });
    }
}