//! DPI 类型：`LogicalSize` / `PhysicalSize` / `LogicalPosition` / `PhysicalPosition`。
//!
//! 语义与 [`winit::dpi`](https://docs.rs/winit/latest/winit/dpi/index.html) 一致：
//!
//! - **逻辑**（DIP，设备无关像素）：应用层坐标 / 字号使用，与屏幕分辨率无关；
//! - **物理**（真实像素）：GPU 纹理 / 窗口尺寸 / 鼠标原始坐标使用；
//! - 换算：`physical = logical × scale_factor`，`scale_factor` 如 `1.0 / 1.5 / 2.0`
//!   （`rjw_ui` 的 `UiInit::scale_factor` 输入来源之一，见 `rjw_ui::ui::UiInit`）。
//!
//! 本模块提供 winit 风格的类型化互转（避免 f32 逻辑坐标与物理坐标混用），
//! 以及到 `glam::Vec2` / 元组 / 数组的互转（引擎内部坐标载体）。
//!
//! **用户侧用法**：传入 `Logical`，转 `Physical` 时**floor 取整**到整数像素——
//! 像素坐标必须为整数，floor 保证窗口/贴图尺寸不越界、相邻元素不重叠、位置无负值
//! 饱和歧义（位置可用 `to_physical_i32`，负坐标也正确）：
//!
//! ```no_run
//! use rjw_transform::{LogicalPosition, LogicalSize};
//! // 尺寸：floor 到 u32（窗口 / 纹理物理尺寸）
//! let logical = LogicalSize::new(1280.0f32, 720.0);
//! let physical = logical.to_physical_u32(2.0); // PhysicalSize<u32> = (2560, 1440)
//! // 位置：floor 到 i32（负坐标也正确：-0.5 × 2 = -1.0 → floor = -1）
//! let pos = LogicalPosition::new(0.5f32, -0.5).to_physical_i32(2.0); // (1, -1)
//! ```

use glam::Vec2;

/// 逻辑尺寸（DIP，设备无关像素）。`T` 通常为 `f32` / `f64` / `u32`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LogicalSize<T> {
    pub width: T,
    pub height: T,
}

/// 物理尺寸（真实像素）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PhysicalSize<T> {
    pub width: T,
    pub height: T,
}

/// 逻辑位置（DIP，设备无关像素；左上角为原点，Y+ 向下与屏幕坐标一致）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LogicalPosition<T> {
    pub x: T,
    pub y: T,
}

/// 物理位置（真实像素）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PhysicalPosition<T> {
    pub x: T,
    pub y: T,
}

// ─── 构造 / 类型转换（cast） ────────────────────────────────────

impl<T: Copy> LogicalSize<T> {
    #[inline]
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }

    /// 把内部数值类型换成 `X`（无损，如 `f32 → f64`；整数→浮点也可）。
    #[inline]
    pub fn cast<X: From<T>>(self) -> LogicalSize<X> {
        LogicalSize {
            width: self.width.into(),
            height: self.height.into(),
        }
    }
}

impl<T: Copy> PhysicalSize<T> {
    #[inline]
    pub const fn new(width: T, height: T) -> Self {
        Self { width, height }
    }

    #[inline]
    pub fn cast<X: From<T>>(self) -> PhysicalSize<X> {
        PhysicalSize {
            width: self.width.into(),
            height: self.height.into(),
        }
    }
}

impl<T: Copy> LogicalPosition<T> {
    #[inline]
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn cast<X: From<T>>(self) -> LogicalPosition<X> {
        LogicalPosition {
            x: self.x.into(),
            y: self.y.into(),
        }
    }
}

impl<T: Copy> PhysicalPosition<T> {
    #[inline]
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn cast<X: From<T>>(self) -> PhysicalPosition<X> {
        PhysicalPosition {
            x: self.x.into(),
            y: self.y.into(),
        }
    }
}

// ─── DPI 换算（physical = logical × scale_factor） ───────────────

impl<T: Copy + Into<f64>> LogicalSize<T> {
    /// → 物理尺寸（`f64` 精度）。
    #[inline]
    pub fn to_physical(&self, scale_factor: f64) -> PhysicalSize<f64> {
        PhysicalSize::new(self.width.into() * scale_factor, self.height.into() * scale_factor)
    }

    /// → 物理尺寸（`f32`，引擎常用；`scale_factor` 与 `rjw_ui` 的 `Ui::scale` 一致）。
    #[inline]
    pub fn to_physical_f32(&self, scale_factor: f32) -> PhysicalSize<f32> {
        let s = scale_factor as f64;
        PhysicalSize::new((self.width.into() * s) as f32, (self.height.into() * s) as f32)
    }

    /// → 物理尺寸（`u32`，**floor 取整**到整数像素：像素坐标必须为整数，
    /// 向下取整保证窗口/纹理尺寸不越界、相邻元素不重叠）。
    #[inline]
    pub fn to_physical_u32(&self, scale_factor: f64) -> PhysicalSize<u32> {
        PhysicalSize::new(
            (self.width.into() * scale_factor).floor() as u32,
            (self.height.into() * scale_factor).floor() as u32,
        )
    }

    /// 由物理尺寸反算逻辑尺寸（`f64` 精度）。
    #[inline]
    pub fn from_physical(physical: PhysicalSize<T>, scale_factor: f64) -> LogicalSize<f64> {
        LogicalSize::new(
            physical.width.into() / scale_factor,
            physical.height.into() / scale_factor,
        )
    }
}

impl<T: Copy + Into<f64>> PhysicalSize<T> {
    /// → 逻辑尺寸（`f64` 精度）。
    #[inline]
    pub fn to_logical(&self, scale_factor: f64) -> LogicalSize<f64> {
        LogicalSize::new(self.width.into() / scale_factor, self.height.into() / scale_factor)
    }

    /// → 逻辑尺寸（`f32`）。
    #[inline]
    pub fn to_logical_f32(&self, scale_factor: f32) -> LogicalSize<f32> {
        let s = scale_factor as f64;
        LogicalSize::new((self.width.into() / s) as f32, (self.height.into() / s) as f32)
    }

    /// 由逻辑尺寸换算物理尺寸（`f64` 精度）。
    #[inline]
    pub fn from_logical(logical: LogicalSize<T>, scale_factor: f64) -> PhysicalSize<f64> {
        PhysicalSize::new(
            logical.width.into() * scale_factor,
            logical.height.into() * scale_factor,
        )
    }
}

impl<T: Copy + Into<f64>> LogicalPosition<T> {
    /// → 物理位置（`f64` 精度）。
    #[inline]
    pub fn to_physical(&self, scale_factor: f64) -> PhysicalPosition<f64> {
        PhysicalPosition::new(self.x.into() * scale_factor, self.y.into() * scale_factor)
    }

    /// → 物理位置（`f32`）。
    #[inline]
    pub fn to_physical_f32(&self, scale_factor: f32) -> PhysicalPosition<f32> {
        let s = scale_factor as f64;
        PhysicalPosition::new((self.x.into() * s) as f32, (self.y.into() * s) as f32)
    }

    /// → 物理位置（`u32`，**floor 取整**；非负坐标用，负值会饱和到 0）。
    #[inline]
    pub fn to_physical_u32(&self, scale_factor: f64) -> PhysicalPosition<u32> {
        PhysicalPosition::new(
            (self.x.into() * scale_factor).floor() as u32,
            (self.y.into() * scale_factor).floor() as u32,
        )
    }

    /// → 物理位置（`i32`，**floor 取整**；**负坐标也正确**，如窗口负边距 / 屏幕外锚点）。
    #[inline]
    pub fn to_physical_i32(&self, scale_factor: f64) -> PhysicalPosition<i32> {
        PhysicalPosition::new(
            (self.x.into() * scale_factor).floor() as i32,
            (self.y.into() * scale_factor).floor() as i32,
        )
    }

    /// 由物理位置反算逻辑位置（`f64` 精度）。
    #[inline]
    pub fn from_physical(physical: PhysicalPosition<T>, scale_factor: f64) -> LogicalPosition<f64> {
        LogicalPosition::new(
            physical.x.into() / scale_factor,
            physical.y.into() / scale_factor,
        )
    }
}

impl<T: Copy + Into<f64>> PhysicalPosition<T> {
    /// → 逻辑位置（`f64` 精度）。
    #[inline]
    pub fn to_logical(&self, scale_factor: f64) -> LogicalPosition<f64> {
        LogicalPosition::new(self.x.into() / scale_factor, self.y.into() / scale_factor)
    }

    /// → 逻辑位置（`f32`）。
    #[inline]
    pub fn to_logical_f32(&self, scale_factor: f32) -> LogicalPosition<f32> {
        let s = scale_factor as f64;
        LogicalPosition::new((self.x.into() / s) as f32, (self.y.into() / s) as f32)
    }

    /// 由逻辑位置换算物理位置（`f64` 精度）。
    #[inline]
    pub fn from_logical(logical: LogicalPosition<T>, scale_factor: f64) -> PhysicalPosition<f64> {
        PhysicalPosition::new(
            logical.x.into() * scale_factor,
            logical.y.into() * scale_factor,
        )
    }
}

// ─── 与 Vec2 / 元组 / 数组互转 ──────────────────────────────────

// 尺寸类型用 width/height，位置类型用 x/y —— 分开实现。
impl<T: Copy> From<(T, T)> for LogicalSize<T> {
    #[inline]
    fn from((w, h): (T, T)) -> Self {
        Self::new(w, h)
    }
}
impl<T: Copy> From<[T; 2]> for LogicalSize<T> {
    #[inline]
    fn from([w, h]: [T; 2]) -> Self {
        Self::new(w, h)
    }
}
impl<T: Copy> From<LogicalSize<T>> for (T, T) {
    #[inline]
    fn from(v: LogicalSize<T>) -> Self {
        (v.width, v.height)
    }
}
impl<T: Copy> From<(T, T)> for PhysicalSize<T> {
    #[inline]
    fn from((w, h): (T, T)) -> Self {
        Self::new(w, h)
    }
}
impl<T: Copy> From<[T; 2]> for PhysicalSize<T> {
    #[inline]
    fn from([w, h]: [T; 2]) -> Self {
        Self::new(w, h)
    }
}
impl<T: Copy> From<PhysicalSize<T>> for (T, T) {
    #[inline]
    fn from(v: PhysicalSize<T>) -> Self {
        (v.width, v.height)
    }
}

impl<T: Copy> From<(T, T)> for LogicalPosition<T> {
    #[inline]
    fn from((x, y): (T, T)) -> Self {
        Self::new(x, y)
    }
}
impl<T: Copy> From<[T; 2]> for LogicalPosition<T> {
    #[inline]
    fn from([x, y]: [T; 2]) -> Self {
        Self::new(x, y)
    }
}
impl<T: Copy> From<LogicalPosition<T>> for (T, T) {
    #[inline]
    fn from(v: LogicalPosition<T>) -> Self {
        (v.x, v.y)
    }
}
impl<T: Copy> From<(T, T)> for PhysicalPosition<T> {
    #[inline]
    fn from((x, y): (T, T)) -> Self {
        Self::new(x, y)
    }
}
impl<T: Copy> From<[T; 2]> for PhysicalPosition<T> {
    #[inline]
    fn from([x, y]: [T; 2]) -> Self {
        Self::new(x, y)
    }
}
impl<T: Copy> From<PhysicalPosition<T>> for (T, T) {
    #[inline]
    fn from(v: PhysicalPosition<T>) -> Self {
        (v.x, v.y)
    }
}

// Vec2（f32 逻辑坐标，引擎内部载体）互转：逻辑类型 ↔ Vec2。
impl From<Vec2> for LogicalSize<f32> {
    #[inline]
    fn from(v: Vec2) -> Self {
        Self::new(v.x, v.y)
    }
}
impl From<LogicalSize<f32>> for Vec2 {
    #[inline]
    fn from(v: LogicalSize<f32>) -> Self {
        Vec2::new(v.width, v.height)
    }
}
impl From<Vec2> for LogicalPosition<f32> {
    #[inline]
    fn from(v: Vec2) -> Self {
        Self::new(v.x, v.y)
    }
}
impl From<LogicalPosition<f32>> for Vec2 {
    #[inline]
    fn from(v: LogicalPosition<f32>) -> Self {
        Vec2::new(v.x, v.y)
    }
}
// 物理（f32）类型 ↔ Vec2（物理像素的浮点载体）。
impl From<Vec2> for PhysicalSize<f32> {
    #[inline]
    fn from(v: Vec2) -> Self {
        Self::new(v.x, v.y)
    }
}
impl From<PhysicalSize<f32>> for Vec2 {
    #[inline]
    fn from(v: PhysicalSize<f32>) -> Self {
        Vec2::new(v.width, v.height)
    }
}
impl From<Vec2> for PhysicalPosition<f32> {
    #[inline]
    fn from(v: Vec2) -> Self {
        Self::new(v.x, v.y)
    }
}
impl From<PhysicalPosition<f32>> for Vec2 {
    #[inline]
    fn from(v: PhysicalPosition<f32>) -> Self {
        Vec2::new(v.x, v.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_to_physical_scale() {
        let l = LogicalSize::new(1280.0f32, 720.0);
        assert_eq!(l.to_physical(2.0), PhysicalSize::new(2560.0, 1440.0));
        assert_eq!(l.to_physical_f32(1.5), PhysicalSize::new(1920.0, 1080.0));
        assert_eq!(l.to_physical_u32(2.0), PhysicalSize::new(2560, 1440));
    }

    #[test]
    fn physical_to_logical_roundtrip() {
        let p = PhysicalSize::new(2560u32, 1440);
        let l = p.to_logical(2.0);
        assert!((l.width - 1280.0).abs() < 1e-9 && (l.height - 720.0).abs() < 1e-9);
        // 反方向（associated）也一致
        let l2 = LogicalSize::from_physical(p, 2.0);
        assert_eq!(l, l2);
    }

    #[test]
    fn position_conversions() {
        let lp = LogicalPosition::new(100.0f32, 50.0);
        assert_eq!(lp.to_physical(2.0), PhysicalPosition::new(200.0, 100.0));
        let pp = PhysicalPosition::new(300.0f32, 150.0);
        assert_eq!(pp.to_logical_f32(2.0), LogicalPosition::new(150.0, 75.0));
        assert_eq!(
            LogicalPosition::from_physical(PhysicalPosition::new(400u32, 200), 2.0),
            LogicalPosition::new(200.0, 100.0)
        );
    }

    #[test]
    fn integer_conversions_floor_to_pixels() {
        // floor 取整：1.0 逻辑 × 1.5 = 1.5 → 1（不四舍五入到 2）
        assert_eq!(
            LogicalSize::new(1.0f32, 1.0).to_physical_u32(1.5),
            PhysicalSize::new(1, 1)
        );
        // 负坐标：floor(-0.5 × 2 = -1.0) = -1（截断会得到 0）——i32 变体正确
        assert_eq!(
            LogicalPosition::new(0.5f32, -0.5).to_physical_i32(2.0),
            PhysicalPosition::new(1, -1)
        );
        // u32 位置负值饱和到 0
        assert_eq!(
            LogicalPosition::new(-0.5f32, 1.0).to_physical_u32(2.0),
            PhysicalPosition::new(0, 2)
        );
    }

    #[test]
    fn cast_changes_scalar_type() {
        // cast 只做无损转换（From）：u32→f64、f32→f64；f64→f32 请用 to_*_f32。
        let l = LogicalSize::new(100u32, 200);
        let lf: LogicalSize<f64> = l.cast();
        assert_eq!(lf, LogicalSize::new(100.0, 200.0));
        let p = PhysicalPosition::new(10.0f32, 20.0);
        assert_eq!(p.cast::<f64>(), PhysicalPosition::new(10.0, 20.0));
    }

    #[test]
    fn vec2_and_tuple_interop() {
        let v = Vec2::new(640.0, 360.0);
        assert_eq!(LogicalSize::from(v), LogicalSize::new(640.0, 360.0));
        assert_eq!(Vec2::from(LogicalSize::new(640.0, 360.0)), v);
        assert_eq!(LogicalPosition::from((10.0, 20.0)), LogicalPosition::new(10.0, 20.0));
        let (x, y): (f32, f32) = PhysicalPosition::new(1.0, 2.0).into();
        assert_eq!((x, y), (1.0, 2.0));
        assert_eq!(PhysicalSize::from([800u32, 600]), PhysicalSize::new(800, 600));
    }
}
