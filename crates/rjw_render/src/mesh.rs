//! 静态网格数据：`MeshData` + 全局注册表 `MESHES` + 类型化句柄 [`MeshId`]。
//!
//! `MeshData` 包装已上传到 GPU 的顶点/索引缓冲，供 2D 渲染器静态实例化合并绘制。
//!
//! 用户路径（happy path）：[`crate::Gpu::mesh`] → [`MeshId`] → `Render2D::static_mesh(id, &tex)`。
//! 低层路径：手动建缓冲后 `MeshData::from_buffers` + `MESHES.register`。

use std::sync::LazyLock;

use crate::registry::{HasUid, TypedRegistry};

static NEXT_MESH_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 静态网格的**类型化句柄**（取代裸 `u64`：不再能和非网格 id 混用）。
///
/// 句柄本身不持有资源；资源在全局 [`MESHES`] 注册表里，uid 与 [`MeshData::uid`] 对应。
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
pub type MeshRegistry = TypedRegistry<MeshData>;

/// 全局静态网格注册表。
pub static MESHES: LazyLock<MeshRegistry> = LazyLock::new(TypedRegistry::default);

#[cfg(test)]
mod tests {
    use super::*;

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
}
