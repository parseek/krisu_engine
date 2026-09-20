use std::ops::Not;

use rjw_krusie::{prelude::*, ui::{TextEditor, widgets}};

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
            }); // row 被限制在单行，使得多行 `TextEditor` 被限制
            ui.label("多行输入框：");
            ui.add(TextEditor::new("text_ml", input_multi_line).multiline().resize(Resize::Both)); // TextEditor 缩放没有默认最小宽高，且下面的控件不会跟着下去，且不在 prelude 里；默认缩放柄为斜线
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