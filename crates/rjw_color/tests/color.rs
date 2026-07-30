use rjw_color::Color;

#[test]
fn color_def() {
    let black = Color::BLACK;
    assert_eq!(black, Color::rgb(0.0, 0.0, 0.0));
    assert_eq!(black, Color::rgba(0.0, 0.0, 0.0, 1.0));
    assert_eq!(black, Color::rgb_u8(0, 0, 0));
    let white = Color::WHITE;
    assert_eq!(white, Color::rgb(1.0, 1.0, 1.0));
    assert_eq!(black, Color::rgba(0.0, 0.0, 0.0, 1.0));
    assert_eq!(white, Color::rgb_u8(255, 255, 255));
}

// 一边写 test 一边完善功能的 workflow です

#[test]
#[cfg(feature = "glam")]
fn color_calc() {
    let (color, alpha) = Color::CYAN.into();
    let color = color * 0.5;
    let color = Color::from((color, alpha));
    assert_eq!(color, Color::rgba(0.0, 0.5, 0.5, 1.0))
}

#[test]
fn color_tuple() {
    let (r, g, b, a) = Color::CYAN.into();
    assert_eq!(Color::rgba(b, g, r, a), Color::rgba(1.0, 1.0, 0.0, 1.0))
}