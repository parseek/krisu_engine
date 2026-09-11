
#[allow(unused)]
pub mod transform2d;
pub use transform2d::Transform2D;

#[allow(unused)]
pub mod camera2d;
pub use camera2d::Camera2D;

#[allow(unused)]
pub mod rect;
pub use rect::Rect;

/// DPI 类型（逻辑 / 物理尺寸与位置，语义同 `winit::dpi`；见 [`dpi`]）。
///
/// 这是引擎**唯一**的 dpi 类型族（happy path 用它）；winit 的同名类型只出现在 `rjw_main` 低层适配里。
#[allow(unused)]
pub mod dpi;
pub use dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};

pub use glam;
pub use glam::{
    Vec2,
    Vec3,
    Vec4,
    Mat4,
    vec2,
    vec3,
    vec3a,
    vec4,
    mat4,
};
