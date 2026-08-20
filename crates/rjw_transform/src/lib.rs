
#[allow(unused)]
pub mod transform2d;
pub use transform2d::Transform2D;

#[allow(unused)]
pub mod camera2d;
pub use camera2d::{Camera2D, Viewport};

#[allow(unused)]
pub mod rect;
pub use rect::Rect;

#[allow(unused)]
pub mod view_cull;
pub use view_cull::ViewCull;

/// DPI 类型（逻辑 / 物理尺寸与位置，语义同 `winit::dpi`；见 [`dpi`]）。
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