//! 窗口身份：`Ctx` 是**窗口作用域**的（多窗口就绪，本期只有主窗口）。

/// 窗口身份。
///
/// 本期只有 [`WindowId::PRIMARY`]；多窗口落地时每个窗口一个 `WindowId`，
/// 引擎按窗口各调一次 `App::update`（`Ctx::frame_of`），`App` trait 形状不变。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(u64);

impl WindowId {
    /// 主窗口（本期唯一窗口）。
    pub const PRIMARY: WindowId = WindowId(0);

    /// 由裸 id 构造（引擎内部 / 多窗口扩展用）。
    #[inline]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// 裸 id。
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl Default for WindowId {
    fn default() -> Self {
        Self::PRIMARY
    }
}
