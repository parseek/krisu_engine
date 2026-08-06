//! 泛型线程安全注册表：`TypedRegistry<T>` + `HasUid`。
//!
//! - 多线程读取（`Arc::clone` / `get_ref` 零拷贝）
//! - 多线程增添（`register` / `register_named`）
//! - 线程安全删除（`remove` / `remove_name_mapping` / `rename`）

use std::sync::Arc;

use dashmap::DashMap;
use dashmap::mapref::one::Ref;

/// 类型的全局唯一 id 标识（用于注册表 key）。
pub trait HasUid {
    fn uid(&self) -> u64;
}

/// 泛型注册表：按 uid / name 查找 `Arc<T>`。
///
/// `T` 必须实现 [`HasUid`]（实例自带 uid，注册时无需额外分配）。
///
/// 线程安全：
/// - 读：`get`（克隆 Arc）、`get_ref`（零拷贝引用）；
/// - 写：`register*` / `remove*` / `rename` 均为原子操作，可跨线程并发。
pub struct TypedRegistry<T: HasUid> {
    by_uid: DashMap<u64, Arc<T>>,
    by_name: DashMap<String, u64>,
}

impl<T: HasUid> Default for TypedRegistry<T> {
    fn default() -> Self {
        Self {
            by_uid: DashMap::with_capacity(64),
            by_name: DashMap::with_capacity(64),
        }
    }
}

impl<T: HasUid> TypedRegistry<T> {
    /// 注册条目，key 取 `item.uid()`。返回该 uid。
    pub fn register(&self, item: Arc<T>) -> u64 {
        let uid = item.uid();
        self.by_uid.insert(uid, item);
        uid
    }

    /// 注册条目并同时建立名称映射。返回该 uid。
    pub fn register_named(&self, name: &str, item: Arc<T>) -> u64 {
        let uid = item.uid();
        self.by_uid.insert(uid, item);
        self.by_name.insert(name.to_string(), uid);
        uid
    }

    /// 按 uid 获取条目（`Arc::clone`，可持有引用）。
    pub fn get(&self, uid: u64) -> Option<Arc<T>> {
        self.by_uid.get(&uid).map(|r| r.clone())
    }

    /// 按 uid 获取条目（`DashMap::Ref` 零拷贝；借用期间不可写同 key）。
    pub fn get_ref(&self, uid: u64) -> Option<Ref<'_, u64, Arc<T>>> {
        self.by_uid.get(&uid)
    }

    /// 按名称获取条目（名称 → uid → 条目）。
    pub fn get_by_name(&self, name: &str) -> Option<Arc<T>> {
        self.by_name.get(name).and_then(|uid| self.get(*uid))
    }

    /// 按名称获取 uid。
    pub fn uid_by_name(&self, name: &str) -> Option<u64> {
        self.by_name.get(name).map(|r| *r)
    }

    /// 按 uid 获取只读引用（零拷贝），无条目时返回 `None`。
    pub fn get_ref_by_name(&self, name: &str) -> Option<Ref<'_, u64, Arc<T>>> {
        let uid = self.uid_by_name(name)?;
        self.by_uid.get(&uid)
    }

    /// 完全删除：移除 uid 映射及所有指向该 uid 的名称映射。
    pub fn remove(&self, uid: u64) {
        self.by_uid.remove(&uid);
        self.by_name.retain(|_, v| *v != uid);
    }

    /// 仅删除名称映射（条目本身保留）。返回被删除名称对应的 uid。
    pub fn remove_name_mapping(&self, name: &str) -> Option<u64> {
        self.by_name.remove(name).map(|(_, uid)| uid)
    }

    /// 替换名称映射：`old` → `new`。返回是否成功（`old` 存在时）。
    pub fn rename(&self, old: &str, new: &str) -> bool {
        if old == new {
            return true;
        }
        let Some(uid) = self.remove_name_mapping(old) else {
            return false;
        };
        self.by_name.insert(new.to_string(), uid);
        true
    }

    /// 是否包含指定 uid。
    pub fn contains_uid(&self, uid: u64) -> bool {
        self.by_uid.contains_key(&uid)
    }

    /// 是否包含指定名称映射。
    pub fn contains_name(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Item {
        id: u64,
    }
    impl HasUid for Item {
        fn uid(&self) -> u64 {
            self.id
        }
    }

    #[test]
    fn register_and_get() {
        let r = TypedRegistry::<Item>::default();
        let item = Arc::new(Item { id: 7 });
        let uid = r.register(item.clone());
        assert_eq!(uid, 7);
        assert!(r.contains_uid(7));
        assert!(!r.contains_uid(8));
        assert!(r.get(7).is_some_and(|v| Arc::ptr_eq(&v, &item)));
        assert!(r.get_ref(7).is_some());
        assert!(r.get_ref(8).is_none());
    }

    #[test]
    fn register_named_and_lookup() {
        let r = TypedRegistry::<Item>::default();
        let item = Arc::new(Item { id: 3 });
        let uid = r.register_named("foo", item.clone());
        assert_eq!(uid, 3);
        assert!(r.contains_name("foo"));
        assert_eq!(r.uid_by_name("foo"), Some(3));
        assert!(r.get_by_name("foo").is_some_and(|v| Arc::ptr_eq(&v, &item)));
        assert!(r.get_ref_by_name("foo").is_some());
        assert!(r.get_by_name("bar").is_none());
    }

    #[test]
    fn remove_fully_cleans_both_maps() {
        let r = TypedRegistry::<Item>::default();
        r.register_named("a", Arc::new(Item { id: 1 }));
        r.register_named("b", Arc::new(Item { id: 1 })); // 同 uid 两个名称
        r.remove(1);
        assert!(!r.contains_uid(1));
        assert!(!r.contains_name("a"));
        assert!(!r.contains_name("b"));
        assert!(r.get(1).is_none());
    }

    #[test]
    fn remove_name_mapping_keeps_entry() {
        let r = TypedRegistry::<Item>::default();
        let item = Arc::new(Item { id: 5 });
        r.register_named("x", item.clone());
        assert_eq!(r.remove_name_mapping("x"), Some(5));
        assert!(!r.contains_name("x"));
        assert!(r.contains_uid(5)); // 条目仍保留
        assert!(r.get(5).is_some());
        assert_eq!(r.remove_name_mapping("x"), None); // 不存在
    }

    #[test]
    fn rename_maps_to_same_uid() {
        let r = TypedRegistry::<Item>::default();
        let item = Arc::new(Item { id: 9 });
        r.register_named("old", item.clone());
        assert!(r.rename("old", "new"));
        assert!(!r.contains_name("old"));
        assert!(r.contains_name("new"));
        assert_eq!(r.uid_by_name("new"), Some(9));
        assert!(r.get_by_name("new").is_some_and(|v| Arc::ptr_eq(&v, &item)));
        // 重命名不存在的映射
        assert!(!r.rename("nope", "newer"));
        // 重命名到自己
        assert!(r.rename("new", "new"));
    }
}