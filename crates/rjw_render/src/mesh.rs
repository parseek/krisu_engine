//! 静态网格数据：`MeshData` + 注册表 `MeshRegistry` + 类型化句柄 [`MeshId`]。
//!
//! `MeshData` 包装已上传到 GPU 的顶点/索引缓冲，供 2D 渲染器静态实例化合并绘制。
//!
//! 用户路径（happy path）：[`crate::Gpu::mesh`] → [`MeshId`] → `Render2D::static_mesh(id, &tex)`。
//! 低层路径：手动建缓冲后 `MeshData::from_buffers` + `Gpu::mesh_registry().register(..)`。
//!
//! **注册表归 [`Gpu`](crate::Gpu) 所有**（每 `RenderContext` 一份，不再是进程级
//! `static`）：绘制期经 `Render2D::meshes()` 解析，构造期经 `Gpu::mesh_registry()`。

use crate::registry::{HasUid, TypedRegistry};

static NEXT_MESH_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 静态网格的**类型化句柄**（取代裸 `u64`：不再能和非网格 id 混用）。
///
/// 句柄本身不持有资源；资源在所属 [`Gpu`](crate::Gpu) 的网格注册表里，
/// uid 与 [`MeshData::uid`] 对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshId(u64);

impl MeshId {
    #[inline]
    pub const fn new(uid: u64) -> Self {
        Self(uid)
    }

    #[inline]
    pub const fn uid(self) -> u64 {
        self.0
    }
}

impl HasUid for MeshId {
    #[inline]
    fn uid(&self) -> u64 {
        self.0
    }
}

impl From<MeshId> for u64 {
    #[inline]
    fn from(id: MeshId) -> Self {
        id.0
    }
}

/// 从 CPU 数据创建静态网格的参数对象（让 [`crate::Gpu::mesh`] 保持 1 参入口）。
#[derive(Debug, Clone, Copy)]
pub struct MeshSpec<'a, T> {
    /// 调试标签（同时进 GPU 缓冲 label）。
    pub label: &'a str,
    /// 顶点数据（须为 `bytemuck::Pod`，布局与 2D 渲染管线 `VertexP3U2C4` 兼容）。
    pub vertices: &'a [T],
    /// u16 索引。
    pub indices: &'a [u16],
}

impl<'a, T> MeshSpec<'a, T> {
    /// 构造 POD 网格参数。
    #[inline]
    pub fn pod(label: &'a str, vertices: &'a [T], indices: &'a [u16]) -> Self {
        Self { label, vertices, indices }
    }
}

/// 静态网格：GPU 顶点/索引缓冲 + 全局唯一 uid。
pub struct MeshData {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    /// 索引数量（三角形个数 × 3），`draw_indexed` 使用。
    pub index_count: u32,
    /// 全局唯一 id。
    pub uid: u64,
}

impl HasUid for MeshData {
    fn uid(&self) -> u64 {
        self.uid
    }
}

impl MeshData {
    /// 直接包装已创建的 GPU 缓冲（低层入口）。
    ///
    /// - `vertex_buffer`：顶点缓冲（与 2D 渲染管线 `VertexP3U2C4` 布局兼容）
    /// - `index_buffer`：u16 索引缓冲
    /// - `index_count`：索引数量（每三角形 3 个）
    ///
    /// 自动分配全局唯一 uid。
    pub fn from_buffers(
        vertex_buffer: wgpu::Buffer,
        index_buffer: wgpu::Buffer,
        index_count: u32,
    ) -> Self {
        Self {
            vertex_buffer,
            index_buffer,
            index_count,
            uid: NEXT_MESH_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// 便捷构造：从 CPU 数据创建顶点/索引缓冲（低层入口；happy path 用 [`crate::Gpu::mesh`]）。
    pub fn from_pod<T: bytemuck::Pod>(
        device: &wgpu::Device,
        vertices: &[T],
        indices: &[u16],
        label: &str,
    ) -> Self {
        use wgpu::util::DeviceExt;
        // 先取本网格的 uid，再用于 label——保证「label 里的编号 == 返回的 uid」（旧实现用
        // `NEXT_MESH_UID.load` 未 fetch_add，label 与真实 uid 差 1）。
        let uid = NEXT_MESH_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let label_prefix = if cfg!(debug_assertions) {
            format!("MeshData #{uid:0>4} ")
        } else {
            "MeshData ".to_string()
        };
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label_prefix}{label}: Mesh vertex buffer")),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label_prefix}{label}: Mesh index buffer")),
            contents: bytemuck::cast_slice(indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            uid,
        }
    }
}

/// 静态网格注册表类型。
///
/// 每个 `RenderContext`（经 [`Gpu`](crate::Gpu)）各持一份，因此**跨 `RenderContext`
/// 互不可见**。`MeshData::uid` 仍由进程级计数器保证全局单调不复用，但
/// **uid 相等不再蕴含「同一个网格」**。
pub type MeshRegistry = TypedRegistry<MeshData>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Dummy;
    impl HasUid for Dummy {
        fn uid(&self) -> u64 {
            unreachable!()
        }
    }

    #[test]
    fn typed_registry_works_for_mesh() {
        let r = TypedRegistry::<Dummy>::default();
        assert!(!r.contains_uid(1));
        assert!(!r.contains_name("nope"));
        assert_eq!(r.remove_name_mapping("nope"), None);
    }

    #[test]
    fn mesh_uid_is_monotonic() {
        let a = NEXT_MESH_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let b = NEXT_MESH_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert!(b > a);
    }

    #[test]
    fn mesh_id_newtype_roundtrip() {
        let id = MeshId::new(42);
        assert_eq!(id.uid(), 42);
        assert_eq!(u64::from(id), 42);
        assert_eq!(id, MeshId::new(42));
    }

    /// **注册表实例化** 的回归：两个 `MeshRegistry` 必须互不可见。
    ///
    /// 旧实现是进程级 `static MESHES` ⇒ 所有 `RenderContext` 共用一张表，
    /// `Render2D::new` 每建一个渲染器就注册一个新的四边形网格与白纹理，
    /// 后建的会**覆盖**先建的条目，使先建渲染器的 uid 指向别人的资源。
    ///
    /// 这里用 `Dummy`（无需 GPU）验证隔离语义：`Dummy` 的 uid 是调用方指定的，
    /// 因此可以精确构造「同一 uid、不同实例」的局面。
    #[test]
    fn registries_are_isolated_and_same_uid_does_not_alias() {
        struct Item(u64);
        impl HasUid for Item {
            fn uid(&self) -> u64 {
                self.0
            }
        }

        let a: TypedRegistry<Item> = TypedRegistry::default();
        let b: TypedRegistry<Item> = TypedRegistry::default();

        // 两个注册表各注册一个 **uid 相同** 但实例不同的条目。
        let ia = Arc::new(Item(7));
        let ib = Arc::new(Item(7));
        assert_eq!(a.register(ia.clone()), 7);
        assert_eq!(b.register(ib.clone()), 7);

        // 各自解析到**自己的**那个实例（不是对方的）。
        assert!(Arc::ptr_eq(&a.get(7).expect("a 有 uid 7"), &ia));
        assert!(Arc::ptr_eq(&b.get(7).expect("b 有 uid 7"), &ib));

        // 一边移除，另一边不受影响 —— 这正是实例化要保证的隔离性。
        a.remove(7);
        assert!(!a.contains_uid(7), "a 已移除");
        assert!(b.contains_uid(7), "b 必须保留（旧全局单例下这里会一起消失）");
        assert!(Arc::ptr_eq(&b.get(7).expect("b 仍有 uid 7"), &ib));
    }
}
