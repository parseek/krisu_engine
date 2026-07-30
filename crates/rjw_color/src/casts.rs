use super::{Color, ColorF64};

// ── Color ↔ (f32, f32, f32, f32) ──────────────────────────────

impl From<Color> for (f32, f32, f32, f32) {
    #[inline]
    fn from(value: Color) -> Self {
        (value.r, value.g, value.b, value.a)
    }
}
impl From<Color> for [f32; 4] {
    #[inline]
    fn from(value: Color) -> Self {
        [value.r, value.g, value.b, value.a]
    }
}

impl From<(f32, f32, f32, f32)> for Color {
    #[inline]
    fn from(value: (f32, f32, f32, f32)) -> Self {
        Self::rgba(value.0, value.1, value.2, value.3)
    }
}
impl From<[f32; 4]> for Color {
    #[inline]
    fn from(value: [f32; 4]) -> Self {
        Self::rgba(value[0], value[1], value[2], value[3])
    }
}

impl From<(f32, f32, f32)> for Color {
    #[inline]
    fn from(value: (f32, f32, f32)) -> Self {
        Self::rgba(value.0, value.1, value.2, 1.0)
    }
}

// ── ColorF64 ↔ (f64, f64, f64, f64) ──────────────────────────

impl From<ColorF64> for (f64, f64, f64, f64) {
    #[inline]
    fn from(value: ColorF64) -> Self {
        (value.r, value.g, value.b, value.a)
    }
}
impl From<ColorF64> for [f64; 4] {
    #[inline]
    fn from(value: ColorF64) -> Self {
        [value.r, value.g, value.b, value.a]
    }
}
impl From<ColorF64> for (f64, f64, f64) {
    #[inline]
    fn from(value: ColorF64) -> Self {
        (value.r, value.g, value.b)
    }
}

impl From<(f64, f64, f64, f64)> for ColorF64 {
    #[inline]
    fn from(value: (f64, f64, f64, f64)) -> Self {
        Self::rgba(value.0, value.1, value.2, value.3)
    }
}
impl From<[f64; 4]> for ColorF64 {
    #[inline]
    fn from(value: [f64; 4]) -> Self {
        Self::rgba(value[0], value[1], value[2], value[3])
    }
}
impl From<(f64, f64, f64)> for ColorF64 {
    #[inline]
    fn from(value: (f64, f64, f64)) -> Self {
        Self::rgba(value.0, value.1, value.2, 1.0)
    }
}

// ── Color ↔ ColorF64 ──────────────────────────────────────────

impl From<Color> for ColorF64 {
    #[inline]
    fn from(value: Color) -> Self {
        Self {
            r: value.r as f64,
            g: value.g as f64,
            b: value.b as f64,
            a: value.a as f64,
        }
    }
}
impl From<ColorF64> for Color {
    #[inline]
    fn from(value: ColorF64) -> Self {
        Self {
            r: value.r as f32,
            g: value.g as f32,
            b: value.b as f32,
            a: value.a as f32,
        }
    }
}

// ── ColorF64 ↔ wgpu::Color (feature = "wgpu") ────────────────

#[cfg(feature = "wgpu")]
impl From<ColorF64> for wgpu::Color {
    #[inline]
    fn from(value: ColorF64) -> Self {
        wgpu::Color {
            r: value.r,
            g: value.g,
            b: value.b,
            a: value.a,
        }
    }
}
#[cfg(feature = "wgpu")]
impl From<wgpu::Color> for ColorF64 {
    #[inline]
    fn from(value: wgpu::Color) -> Self {
        Self {
            r: value.r,
            g: value.g,
            b: value.b,
            a: value.a,
        }
    }
}

// ── Color ↔ wgpu::Color (feature = "wgpu") ───────────────────

#[cfg(feature = "wgpu")]
impl From<Color> for wgpu::Color {
    #[inline]
    fn from(value: Color) -> Self {
        wgpu::Color {
            r: value.r as f64,
            g: value.g as f64,
            b: value.b as f64,
            a: value.a as f64,
        }
    }
}

// ── ColorF64 ↔ glam::DVec3 / (DVec3, f64) (feature = "glam") ──

#[cfg(feature = "glam")]
pub mod glam {
    use super::{Color, ColorF64};
    use glam::{DVec3, DVec4, Vec3, Vec4};

    // ── Color (f32) ──────────────────────────────────────────

    impl From<Vec4> for Color {
        #[inline]
        fn from(v: Vec4) -> Self {
            Self::rgba(v.x, v.y, v.z, v.w)
        }
    }
    impl From<Color> for Vec4 {
        #[inline]
        fn from(c: Color) -> Self {
            Self::new(c.r, c.g, c.b, c.a)
        }
    }
    impl From<Vec3> for Color {
        #[inline]
        fn from(v: Vec3) -> Self {
            Self::rgba(v.x, v.y, v.z, 1.0)
        }
    }
    impl From<(Vec3, f32)> for Color {
        #[inline]
        fn from(v: (Vec3, f32)) -> Self {
            Self::rgba(v.0.x, v.0.y, v.0.z, v.1)
        }
    }
    impl From<Color> for (Vec3, f32) {
        #[inline]
        fn from(c: Color) -> Self {
            (Vec3::new(c.r, c.g, c.b), c.a)
        }
    }

    // ── ColorF64 (f64) ──────────────────────────────────────

    impl From<DVec4> for ColorF64 {
        #[inline]
        fn from(v: DVec4) -> Self {
            Self::rgba(v.x, v.y, v.z, v.w)
        }
    }
    impl From<ColorF64> for DVec4 {
        #[inline]
        fn from(c: ColorF64) -> Self {
            Self::new(c.r, c.g, c.b, c.a)
        }
    }
    impl From<DVec3> for ColorF64 {
        #[inline]
        fn from(v: DVec3) -> Self {
            Self::rgba(v.x, v.y, v.z, 1.0)
        }
    }
    impl From<(DVec3, f64)> for ColorF64 {
        #[inline]
        fn from(v: (DVec3, f64)) -> Self {
            Self::rgba(v.0.x, v.0.y, v.0.z, v.1)
        }
    }
    impl From<ColorF64> for (DVec3, f64) {
        #[inline]
        fn from(c: ColorF64) -> Self {
            (DVec3::new(c.r, c.g, c.b), c.a)
        }
    }
}