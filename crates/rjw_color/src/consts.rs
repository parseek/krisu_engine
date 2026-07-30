use super::{Color, ColorF64};

macro_rules! color_consts {
    ($ty:ident) => {
        #[rustfmt::skip]
        impl $ty {
            pub const BLACK: Self           = Self::rgb (        0.0,         0.0,         0.0                );
            pub const WHITE: Self           = Self::rgb (        1.0,         1.0,         1.0                );
            pub const TRANSPARENT: Self     = Self::rgba(        0.0,         0.0,         0.0,            0.0);
            pub const RED: Self             = Self::rgb (        1.0,         0.0,         0.0                );
            pub const GREEN: Self           = Self::rgb (        0.0,         1.0,         0.0                );
            pub const BLUE: Self            = Self::rgb (        0.0,         0.0,         1.0                );
            pub const YELLOW: Self          = Self::rgb (        1.0,         1.0,         0.0                );
            pub const PURPLE: Self          = Self::rgb (        1.0,         0.0,         1.0                );
            pub const CYAN: Self            = Self::rgb (        0.0,         1.0,         1.0                );
        }
    };
}

color_consts!(Color);
color_consts!(ColorF64);
