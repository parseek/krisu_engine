//! **ID 命名空间区块**（[`Ui::namespace`](crate::Ui::namespace) / [`Namespace`]）：把一段
//! 内容录在当前容器的光标处，只给它加一层 **ID 前缀**。
//!
//! # 维护者笔记（改这个文件前先读）
//!
//! 1. **它不画任何东西、不做任何布局**：没有背景、没有内边距、不裁切、不另开 `pack`。
//!    唯一的效果是 `IdStack::push(前缀)` —— 内部控件的**绝对 ID** 变成 `前缀/相对名`。
//!    于是两处同名控件（同名窗口里的两个 `"ok"`、同一列表里的同名输入框）不再互相踩状态。
//! 2. **为什么需要它（而不是让用户自己写 `with_id`）**：`with_id` 是 `pub(crate)`
//!    （引擎内部容器用），第三方 / 应用写不出"只加命名空间、不要容器"的东西 —— 于是
//!    `gallery.rs` 里长期躺着一句 "推出 `ui.namespace(..)`" 的 TODO。
//! 3. **正文录在父 frame 里（不另开 frame）**：内容与"不用 namespace 时"逐像素相同，
//!    父 frame 的 `gap` 与 `avail_w` 照旧生效。返回的是"正文推进了父光标多少"。
//! 4. **返回尺寸是给"占光标"用的**：正文的每一项自己就推进了父光标，所以这里**不再**
//!    `place_external`（那会推进两次）。返回值只是给调用方做诊断 / 后续排版决策。
//! 5. 纯几何抽成自由函数 [`namespace_size`]（本仓库单测不构造 `Ui`）。

use super::*;

use glam::Vec2;

/// **命名空间区块 builder**（[`Ui::namespace`] / [`UiAdd::namespace`] 返回）。
///
/// ```no_run
/// # use rjw_ui::{Ui, UiAdd};
/// # fn demo(ui: &mut Ui, a: &mut String, b: &mut String) {
/// ui.namespace("left", |ui| {
///     ui.text_input("kw", a);          // 状态键 = "left/kw"
/// });
/// ui.namespace("right", |ui| {
///     ui.text_input("kw", b);          // 状态键 = "right/kw"（与上面互不干扰）
/// });
/// # }
/// ```
///
/// 嵌套会**顺序拼接**：`namespace("a", ..)` 里的 `namespace("b", ..)` 里的 `"kw"` ⇒ `"a/b/kw"`。
pub struct Namespace<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
    /// 命名空间段（相对名）。
    id: String,
}

impl<'ui, 'a> Namespace<'ui, 'a> {
    /// 新建（[`Ui::namespace`] 用；`id` = 命名空间段，也是段内所有控件的**绝对 ID 前缀**）。
    pub fn new(ui: &'ui mut Ui<'a>, id: impl Into<String>) -> Self {
        Self { ui, id: id.into() }
    }

    /// 终结：在 `id` 命名空间内执行 `body`，返回正文**结算尺寸**（物理像素；空内容 = `(0,0)`）。
    ///
    /// `body` 拿到 [`PackEntry`]（实现了 [`UiAdd`]）—— 与 [`Foldable::show`](crate::Foldable::show)
    /// 同一个上下文类型（正文里 `label` / `row` / 嵌套 `namespace` 都能用）。
    pub fn show(self, body: impl FnOnce(&mut PackEntry<'_, '_>)) -> Vec2 {
        let Self { ui, id } = self;
        let before = ui.cursor_pos();
        // **区块级作用域**（[`Ui::container_scope`]）+ ID 命名空间（`with_id`）：正文 = 一个
        // 逻辑区块 ⇒ **不新开顶层放置**（空命名空间因此不产生空放置；有内容时也不会让正文
        // 首项多带一个父级 `gap`）。两者的闭包作用域都保证成对。
        ui.container_scope(|ui| ui.with_id(id.as_str(), |ui| body(&mut PackEntry::new(ui))));
        let after = ui.cursor_pos();
        namespace_size(before, after, ui.theme().gap)
    }
}

impl<'a> Ui<'a> {
    /// **ID 命名空间区块**：在 `id` 命名空间内执行 `body`，返回正文**结算尺寸**（物理像素）。
    ///
    /// 用途：让**同名控件**互不干扰 —— 两处 UI（两个子窗口 / 两组表单 / 动态列表的每一行）
    /// 都可能出现 `"ok"` / `"kw"` 这类相对名，没有命名空间时它们的跨帧状态（输入内容 /
    /// 焦点 / 勾选）会互相覆盖。窗口 / 面板 / 滚动容器 / grid / 区块**自己**就是命名空间
    /// 边界；本方法是"只想要命名空间、不要任何容器"的那个入口。
    ///
    /// ```no_run
    /// # use rjw_ui::{Ui, UiAdd};
    /// # fn demo(ui: &mut Ui, a: &mut String, b: &mut String) {
    /// ui.namespace("left", |ui| {
    ///     ui.text_input("kw", a);      // 状态键 = "left/kw"
    /// });
    /// ui.namespace("right", |ui| {
    ///     ui.text_input("kw", b);      // 状态键 = "right/kw"（与上面互不干扰）
    /// });
    /// # }
    /// ```
    ///
    /// - **不做任何布局 / 绘制**：没有背景、内边距、裁剪，也不另开容器 —— 正文与"不用它"
    ///   时**逐像素相同**，只是 ID 多了一层前缀；
    /// - 正文是**当前容器的子项**：录在光标处，`avail_w` 与 `gap` 照旧生效；父容器光标由
    ///   正文各项自己推进（本方法**不**额外占位）；
    /// - 嵌套顺序拼接：`"a"` 里的 `"b"` 里的 `"kw"` ⇒ `"a/b/kw"`（`/` 是段分隔符，
    ///   见 [`crate::id`]）；
    /// - 返回正文结算尺寸（空内容 / 只有绝对定位时 = `(0,0)`）——供调用方做后续排版决策。
    pub fn namespace(&mut self, id: &str, body: impl FnOnce(&mut PackEntry<'_, '_>)) -> Vec2 {
        Namespace::new(self, id.to_owned()).show(body)
    }
}

/// **命名空间区块的结算尺寸**（纯函数，可单测）：光标在 `body` 前后各采一次
/// （`child_rect` 每次推进 `高 + gap`）⇒ 高度要扣掉末尾那一项 `gap`。
///
/// 只在"内容非空且确实推进过"时给出正高度：空内容 / 只有绝对定位（`*_at` 不占光标）
/// 时返回 `(0,0)`。
#[inline]
pub fn namespace_size(before: Vec2, after: Vec2, gap: f32) -> Vec2 {
    let w = (after.x - before.x).max(0.0);
    let h = (after.y - before.y - gap).max(0.0);
    Vec2::new(w, h)
}

/// **`namespace` 不新增布局开销**（钉住"只加 ID 前缀"这条语义的机器判据）。
///
/// 判据 = 光标推进量只由**子项数量**决定（`child_rect` 每次推进 `高 + gap`），与
/// "开不开命名空间"无关：所以"同一串子项，包一层 namespace"与"不包"的父光标必须一致。
///
/// 单测不构造 `Ui`（没有窗口），故这里只断言纯函数看得到的那一半：**同一份
/// `(before, after)` 下 `namespace_size` 只做算术**（不额外加内边距 / 不累计容器高）。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_size_is_pure_arithmetic_on_the_cursor() {
        // 三项子项（各 26 高、gap 6）推进 = 26+6 + 26+6 + 26+6 = 96；结算高必须扣掉**一项** gap。
        let before = Vec2::new(8.0, 8.0);
        let after = before + Vec2::new(120.0, 96.0);
        assert_eq!(namespace_size(before, after, 6.0), Vec2::new(120.0, 90.0));
        // 不推进（空内容 / 只有绝对定位）⇒ 永远 (0,0)：`namespace` 不"凭空占位"。
        assert_eq!(namespace_size(before, before, 6.0), Vec2::ZERO);
        // 纯横向推进（`PackSide::Left` 的父容器）不产生高度。
        assert_eq!(
            namespace_size(before, before + Vec2::new(50.0, 0.0), 6.0),
            Vec2::new(50.0, 0.0)
        );
    }
}
