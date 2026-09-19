//! 比 eg260818UI 更加简洁的实例
//! 目的在于简洁

pub mod app;

fn main() -> Result<(), app::RunError> {
    let mut app = app::RJWApp::default();
    app.register_demo(app::hellowindow::HelloWindow);
    app.register_demo(app::base_information::BaseInformation);
    app.register_demo(app::color_picker::ColorPicker::default());
    app::run(app)
}