//! 一次 pass 的附件清理配置。
//!
//! 取代旧的 `ClearConfig { color: Option<..>, depth: Option<..>, stencil: Option<..> }`
//! ——三个 `Option` 同时是「清什么」和「是否清」的开关（违反 `docs/API_DESIGN.md` R3），
//! 且把 `wgpu::Color` 这种低层类型带上 happy path（违反 R5）。
//!
//! 现在是一个**枚举**：每个变体就是一种明确的清理意图，颜色用引擎的 [`ColorF64`]。

use rjw_color::{Color, ColorF64};

/// 一次 pass 的清理意图。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Clear {
    /// 全部保留（不清理任何附件；叠加画面 / 多 pass 组合用）。
    Keep,
    /// 清颜色。
    Color(ColorF64),
    /// 清颜色 + 深度。
    ColorDepth(ColorF64, f32),
    /// 只清深度（保留颜色；重叠画面的独立 pass 用）。
    Depth(f32),
    /// 只清模板。
    Stencil(u32),
}

impl Default for Clear {
    /// 默认清黑（等价旧 `ClearConfig::default()`）。
    fn default() -> Self {
        Clear::Color(ColorF64::BLACK)
    }
}

impl Clear {
    /// 清颜色（接受 `Color` 或 `ColorF64`）。
    #[inline]
    pub fn color(c: impl Into<ColorF64>) -> Self {
        Clear::Color(c.into())
    }

    /// 清颜色 + 深度。
    #[inline]
    pub fn color_depth(c: impl Into<ColorF64>, depth: f32) -> Self {
        Clear::ColorDepth(c.into(), depth)
    }

    /// 只清深度。
    #[inline]
    pub fn depth(depth: f32) -> Self {
        Clear::Depth(depth)
    }

    /// 只清模板。
    #[inline]
    pub fn stencil(value: u32) -> Self {
        Clear::Stencil(value)
    }

    /// 是否需要深度清理值。
    #[inline]
    pub fn depth_value(&self) -> Option<f32> {
        match *self {
            Clear::ColorDepth(_, d) | Clear::Depth(d) => Some(d),
            _ => None,
        }
    }

    /// 是否需要模板清理值。
    #[inline]
    pub fn stencil_value(&self) -> Option<u32> {
        match *self {
            Clear::Stencil(s) => Some(s),
            _ => None,
        }
    }

    /// 颜色清理值。
    #[inline]
    pub fn color_value(&self) -> Option<ColorF64> {
        match *self {
            Clear::Color(c) | Clear::ColorDepth(c, _) => Some(c),
            _ => None,
        }
    }

    /// 是否需要绑定深度附件。
    #[inline]
    pub fn uses_depth(&self) -> bool {
        self.depth_value().is_some()
    }

    /// 是否需要绑定模板附件。
    #[inline]
    pub fn uses_stencil(&self) -> bool {
        self.stencil_value().is_some()
    }

    /// 是否需要深度 / 模板附件（`PassBuilder` 推导 `need` 用）。
    #[inline]
    pub fn uses_depth_stencil(&self) -> bool {
        self.uses_depth() || self.uses_stencil()
    }

    /// **去掉颜色清屏**，保留深度 / 模板意图（`Keep` 若本来就没有）。
    ///
    /// 多画面（分屏 / 画中画）用：wgpu 的 load-op 颜色清屏作用于**整张附件**——
    /// 第二个画面的「清成某色」会把第一个画面一起抹掉。运行时据此把非首个画面的
    /// 颜色清屏改成「保留 + 在画面矩形内画一块实心底」，语义不变且**只影响本画面**。
    #[inline]
    pub fn without_color(&self) -> Clear {
        match *self {
            Clear::Color(_) => Clear::Keep,
            Clear::ColorDepth(_, d) => Clear::Depth(d),
            other => other,
        }
    }
}

impl From<Color> for Clear {
    #[inline]
    fn from(value: Color) -> Self {
        Clear::Color(value.into())
    }
}

impl From<ColorF64> for Clear {
    #[inline]
    fn from(value: ColorF64) -> Self {
        Clear::Color(value)
    }
}

impl From<&ColorF64> for Clear {
    #[inline]
    fn from(value: &ColorF64) -> Self {
        Clear::Color(*value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_black_color_and_needs_no_depth() {
        let c = Clear::default();
        assert_eq!(c.color_value(), Some(ColorF64::BLACK));
        assert!(!c.uses_depth_stencil());
    }

    #[test]
    fn keep_clears_nothing() {
        let c = Clear::Keep;
        assert_eq!(c.color_value(), None);
        assert!(!c.uses_depth_stencil());
    }

    #[test]
    fn color_depth_sets_both_and_needs_attachment() {
        // 用二进制可精确表示的值，避免 f32→f64 转换误差影响断言。
        let c = Clear::color_depth(Color::rgba(0.5, 0.25, 0.125, 1.0), 1.0);
        assert_eq!(c.color_value(), Some(ColorF64::rgba(0.5, 0.25, 0.125, 1.0)));
        assert_eq!(c.depth_value(), Some(1.0));
        assert!(c.uses_depth());
        assert!(!c.uses_stencil());
        assert!(c.uses_depth_stencil());
    }

    #[test]
    fn from_color_and_colorf64() {
        let a: Clear = Color::RED.into();
        let b: Clear = ColorF64::GREEN.into();
        assert_eq!(a.color_value(), Some(Color::RED.into()));
        assert_eq!(b.color_value(), Some(ColorF64::GREEN));
    }

    #[test]
    fn stencil_only() {
        let c = Clear::stencil(0);
        assert_eq!(c.stencil_value(), Some(0));
        assert_eq!(c.color_value(), None);
        assert!(c.uses_stencil());
        assert!(!c.uses_depth());
    }

    /// **多画面回归**：`without_color` 只去掉颜色清屏，深度 / 模板 / 保留语义不变。
    ///
    /// 运行时用它把「非首个画面的颜色清屏」降级为「保留 + 画一块画面底色」——
    /// 否则 load-op 会清**整张附件**，把先前的分屏一起抹掉（"左分屏完全未显示"）。
    #[test]
    fn without_color_keeps_depth_and_stencil() {
        assert_eq!(Clear::color(Color::RED).without_color(), Clear::Keep);
        assert_eq!(Clear::color_depth(Color::RED, 0.5).without_color(), Clear::Depth(0.5));
        assert_eq!(Clear::depth(1.0).without_color(), Clear::Depth(1.0));
        assert_eq!(Clear::stencil(7).without_color(), Clear::Stencil(7));
        assert_eq!(Clear::Keep.without_color(), Clear::Keep);
        // 颜色确实被去掉（这是关键：调用方据 color_value() 决定是否补底色矩形）。
        assert_eq!(Clear::color_depth(Color::RED, 0.5).without_color().color_value(), None);
        assert_eq!(Clear::color(Color::RED).without_color().color_value(), None);
    }
}
