//! 比 eg260818UI 更加简洁的实例
//! 目的在于简洁
//!
//! # 诊断开关（P0 安全网，见 `docs/DEBUGGING.md` §0）
//!
//! - `--demo <子串>`：只把名字含子串的那个 demo 置为可见（子串打错 ⇒ 打清单 + 非 0 退出）。
//!   没有它时**所有 demo 默认关闭**，`--frames N` 冒烟跑的是空屏（等于没验证）。
//! - `--ui-dump`：每帧把引擎状态（`Ui::debug_dump`）打到 stderr。
//! - `--frames N` 由运行时自己解析（不经过这里）。

pub mod app;

fn main() -> Result<(), app::RunError> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut app = app::RJWApp::default();
    app.register_demo(app::hellowindow::HelloWindow);
    app.register_demo(app::base_information::BaseInformation);
    app.register_demo(app::color_picker::ColorPicker::default());
    app.register_demo(app::gallery::Gallery::default());

    app.ui_dump = args.iter().any(|a| a == "--ui-dump");
    if let Some(needle) = flag_value(&args, "--demo") {
        match app.show_demo(&needle) {
            // 回显命中的类型名 = 窗口 id ⇒ 直接对着 `--ui-dump` 的那行看。
            Some((id, type_name)) => eprintln!("[demo] {id}（窗口 id {type_name}）已可见"),
            None => {
                eprintln!(
                    "[demo] 没有匹配 {needle:?} 的 demo；可选：{}",
                    app.demo_names().join(" / ")
                );
                std::process::exit(1);
            }
        }
    }

    app::run(app)
}

/// 取 `--flag value` 形式的值（`--flag` 在末尾没有值 ⇒ `None`）。
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).cloned()
}
