//! ID 命名空间：`IdRelative`（应用层名字）与 `IdAbsolute`（完整状态键）。
//!
//! # 为什么强类型
//!
//! - [`IdRelative`]：控件作者 / 应用给的**原始名字**（如 `"btn"`、`"chishi"`），未加任何前缀；
//! - [`IdAbsolute`]：**完整命名空间键**（如 `"chishi/btn"`、`"outer/c::popup/opt_0"`），
//!   可直接作 `UiState` 各 HashMap 的键 / 焦点 id / 窗口 id。
//!
//! 类型层面杜绝两类 bug：
//! 1. **双重前缀**：[`IdStack::push`] / [`IdStack::id_for`] 只收 `IdRelative`——把已解析的
//!    绝对 id 再塞进前缀系统（拼出 `outer/outer/btn`）编译期报错；
//! 2. **相对 / 绝对混用**：状态读写（`widgets` / `focused` / `combo_open` / `panel_pos` …）
//!    只接受 `IdAbsolute`——拿原始名字查跨帧状态（漏前缀失配）编译期报错。
//!
//! # 零拷贝路径
//!
//! - [`IdStack::id_for`] 在栈空（顶层）时返回 `Cow::Borrowed(label)`——**零分配**；
//! - 读路径 `HashMap::get(&str)`（经 [`Borrow<str>`]）同样**零分配**；
//! - 仅**写库**（`entry` / `insert`，键需 owned）与嵌套拼接各一次分配。

use std::borrow::Cow;

/// 相对 ID：应用 / 控件作者提供的**原始名字**（未加命名空间前缀）。
///
/// 由 `From<&str>` 构造（`IdRelative::from("btn")` / `"btn".into()`）；控件公开 API
/// 保留 `&str` 参数，入口统一转换。
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IdRelative<'a>(&'a str);

impl<'a> From<&'a str> for IdRelative<'a> {
    #[inline]
    fn from(id: &'a str) -> Self {
        Self(id)
    }
}

impl IdRelative<'_> {
    #[inline]
    pub fn as_str(&self) -> &str {
        self.0
    }
}

/// 绝对 ID：**完整命名空间状态键**（如 `"chishi/btn"`）。
///
/// - **帧内借用**：[`IdStack::id_for`] 返回 `IdAbsolute<'_>`，栈空时 `Cow::Borrowed`
///   （零拷贝零分配），嵌套时 `Cow::Owned`（一次拼接）；
/// - **跨帧存储**：[`UiState`](crate::state::UiState) 各 HashMap 的键为
///   `IdAbsolute<'static>`（owned），经 [`Self::to_static`] 落库（Borrowed 才分配）。
///
/// 读路径（`HashMap::get` / `remove`）可传 `&str`（[`Borrow<str>`] 实现），零分配。
#[derive(Clone, Debug, Hash)]
pub struct IdAbsolute<'a>(Cow<'a, str>);

impl<'a, 'b> PartialEq<IdAbsolute<'b>> for IdAbsolute<'a> {
    #[inline]
    fn eq(&self, other: &IdAbsolute<'b>) -> bool {
        self.as_str() == other.as_str()
    }
}
impl Eq for IdAbsolute<'_> {}

impl IdAbsolute<'_> {
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 构造 owned 绝对 id（子键如 `grip` / `vbar` / `popup`、强制 z 的浮层 id 用）。
    pub fn owned(s: String) -> IdAbsolute<'static> {
        IdAbsolute(Cow::Owned(s))
    }

    /// 转成可跨帧存储的 `'static` 键（始终 owned；写库路径本就需一次分配）。
    pub fn to_static(&self) -> IdAbsolute<'static> {
        IdAbsolute(Cow::Owned(self.as_str().to_owned()))
    }
}

impl<'a> From<&'a str> for IdAbsolute<'a> {
    /// 字面量 / 长命 `&str` → borrowed 绝对 id（应用初始化 `radio_groups` 等用）。
    #[inline]
    fn from(id: &'a str) -> Self {
        Self(Cow::Borrowed(id))
    }
}

/// 读路径零拷贝：`map.get(abs.as_str())` / `map.remove(abs.as_str())`。
impl<'a> std::borrow::Borrow<str> for IdAbsolute<'a> {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

/// ID 命名空间栈：连续前缀缓冲 + 段偏移，push / pop **就地**维护（无 join 重建）。
#[derive(Debug, Default)]
pub struct IdStack {
    /// 连续前缀，如 `"outer/inner"`。
    buf: String,
    /// 每段在 `buf` 中的起点偏移（push 记起点、pop 按起点截断）。
    segs: Vec<usize>,
}

impl IdStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// 压入一个命名空间段（容器进入时）。
    pub fn push(&mut self, id_relative: IdRelative<'_>) {
        if !self.buf.is_empty() {
            self.buf.push('/');
        }
        let start = self.buf.len();
        self.buf.push_str(id_relative.as_str());
        self.segs.push(start);
    }

    /// 弹出最后一个命名空间段（容器退出时）。
    pub fn pop(&mut self) {
        if let Some(start) = self.segs.pop() {
            // start 是该段内容起点；若前面有 '/', 截断到 start-1 保留前段。
            self.buf.truncate(start.saturating_sub(1));
        }
    }

    /// 根据当前栈与相对 id 生成**绝对 id**。
    ///
    /// **生命周期绑定传入的相对名字（label），而非 `&mut self`**——`Cow::Borrowed`
    /// 借用的是 `id_relative` 内部 `&str`（`'l`），`Owned` 无借用；因此调用后 `self` 的可变
    /// 借用立即释放，生成的 `IdAbsolute` 可继续用于后续 `&mut self` 操作
    /// （`register_focus` / 状态读写），读路径零分配。
    ///
    /// - 栈空（顶层）：`Borrowed(label)`——**零拷贝零分配**；
    /// - 嵌套：拼接一次 `format!("{prefix}/{label}")`。
    pub fn id_for<'s, 'l>(&'s mut self, id_relative: IdRelative<'l>) -> IdAbsolute<'l> {
        if self.segs.is_empty() {
            // 直接取字段：`id_relative.0` 是 `&'l str`（Copy），保留 label 生命周期。
            IdAbsolute(Cow::Borrowed(id_relative.0))
        } else {
            IdAbsolute(Cow::Owned(format!("{}/{}", self.buf, id_relative.0)))
        }
    }

    /// 当前前缀（不含最终斜杠）。
    #[cfg(test)]
    fn prefix(&self) -> &str {
        &self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn empty_stack_borrows_label() {
        let mut st = IdStack::new();
        let abs = st.id_for("btn".into());
        assert_eq!(abs.as_str(), "btn");
        // 栈空必须 Borrowed（零拷贝零分配）。
        assert!(matches!(abs.0, Cow::Borrowed("btn")));
    }

    #[test]
    fn nested_joins_and_pop_restores() {
        let mut st = IdStack::new();
        st.push("win_a".into());
        assert_eq!(st.prefix(), "win_a");
        assert_eq!(st.id_for("btn".into()).as_str(), "win_a/btn");
        st.push("c::popup".into());
        assert_eq!(st.id_for("opt_0".into()).as_str(), "win_a/c::popup/opt_0");
        st.pop();
        assert_eq!(st.prefix(), "win_a");
        assert_eq!(st.id_for("ok".into()).as_str(), "win_a/ok");
        st.pop();
        assert_eq!(st.prefix(), "");
        // 空栈恢复 Borrowed。
        assert!(matches!(st.id_for("x".into()).0, Cow::Borrowed("x")));
    }

    #[test]
    fn to_static_matches_content() {
        let mut st = IdStack::new();
        st.push("w".into());
        let abs = st.id_for("b".into()); // Owned
        let s = abs.to_static();
        assert_eq!(s.as_str(), "w/b");
        st.pop();
        let b = st.id_for("t".into()); // Borrowed
        assert_eq!(b.to_static().as_str(), "t");
    }

    #[test]
    fn borrow_str_allows_get_by_str() {
        let mut m: HashMap<IdAbsolute<'static>, u32> = HashMap::new();
        m.insert(IdAbsolute::from("a/b"), 1u32);
        m.insert(IdAbsolute::owned("w/c".into()), 2u32);
        assert_eq!(m.get("a/b"), Some(&1)); // Borrow<str> 零拷贝读
        assert_eq!(m.get("w/c"), Some(&2));
        assert_eq!(m.get("nope"), None);
        // 跨生命周期比较：内容相等即相等。
        let mut st = IdStack::new();
        st.push("a".into());
        assert!(IdAbsolute::owned("a/b".into()) == st.id_for("b".into()));
    }
}
