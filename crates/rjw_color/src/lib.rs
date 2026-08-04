#[cfg(feature = "serde")]
fn default_alpha() -> f32 { 1.0 }

#[derive(Debug, PartialEq, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    #[cfg_attr(feature = "serde", serde(default="default_alpha"))]
    pub a: f32,
}

impl Color {
    const INV_U8MAX: f32 = 1. / 255.;

    #[inline]
    pub const fn rgba_u8(r: u8, g: u8, b: u8, a: u8) -> Self {
        let r = r as f32 * Self::INV_U8MAX;
        let g = g as f32 * Self::INV_U8MAX;
        let b = b as f32 * Self::INV_U8MAX;
        let a = a as f32 * Self::INV_U8MAX;
        Self { r, g, b, a }
    }
    #[inline]
    pub const fn rgb_u8(r: u8, g: u8, b: u8) -> Self {
        Self::rgba_u8(r, g, b, 255)
    }

    #[inline]
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
    #[inline]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::rgba(r, g, b, 1.0)
    }

    #[inline]
    pub const fn rgba_one(value: f32) -> Self {
        Self { r: value, g: value, b: value, a: value }
    }
    #[inline]
    pub const fn rgb_one(value: f32) -> Self {
        Self::rgba(value, value, value, 1.0)
    }
}

/// f64-precision color. Convenient for working with `wgpu::Color` and DVec3.
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct ColorF64 {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl ColorF64 {
    #[inline]
    pub const fn rgba(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }
    #[inline]
    pub const fn rgb(r: f64, g: f64, b: f64) -> Self {
        Self::rgba(r, g, b, 1.0)
    }

    #[inline]
    pub const fn rgba_one(value: f64) -> Self {
        Self { r: value, g: value, b: value, a: value }
    }
    #[inline]
    pub const fn rgb_one(value: f64) -> Self {
        Self::rgba(value, value, value, 1.0)
    }
}

impl Default for ColorF64 {
    #[inline]
    fn default() -> Self {
        Self {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }
    }
}

pub mod from_hex;
pub mod casts;
pub mod consts;

impl Default for Color {
    #[inline]
    fn default() -> Self {
        Self::BLACK
    }
}