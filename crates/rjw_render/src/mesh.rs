//! 静态网格数据：`MeshData` + 全局注册表 `MESHES`。
//!
//! `MeshData` 包装已上传到 GPU 的顶点/索引缓冲，供 2D 渲染器静态实例化合并绘制。

use std::sync::LazyLock;

use crate::registry::{HasUid, TypedRegistry};

static NEXT_MESH_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 静态网格：GPU 顶点/索引缓冲 + 全局唯一 uid。
///
/// 用户通过 [`MeshData::from_buffers`]（或便捷方法 [`MeshData::from_pod`]）创建后
/// 用 `MESHES.register` / `Render2D::register_mesh` 注册，获得可复用的 `mesh_id`。
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
    /// 直接包装已创建的 GPU 缓冲。
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

    /// 便捷构造：从 CPU 数据创建顶点/索引缓冲。
    ///
    /// `T` 需为 `bytemuck::Pod`（例如 `[f32; 3]` + UV + 颜色的组合顶点）。
    /// 索引使用 `u16`。
    pub fn from_pod<T: bytemuck::Pod>(
        device: &wgpu::Device,
        vertices: &[T],
        indices: &[u16],
        label: &str,
    ) -> Self {
        use wgpu::util::DeviceExt;
        let label_prefix = if cfg!(debug_assertions) {
            format!("MeshData #{:0>4} ", NEXT_MESH_UID.load(std::sync::atomic::Ordering::Relaxed))
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
        Self::from_buffers(vertex_buffer, index_buffer, indices.len() as u32)
    }
}

/// 全局静态网格注册表。
pub static MESHES: LazyLock<TypedRegistry<MeshData>> = LazyLock::new(TypedRegistry::default);

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
        // 不构造真实 GPU 缓冲（无 device），仅验证注册表与 MeshData 类型组合。
        let r = TypedRegistry::<Dummy>::default();
        assert!(!r.contains_uid(1));
        assert!(!r.contains_name("nope"));
        assert_eq!(r.remove_name_mapping("nope"), None);
    }

    #[test]
    fn mesh_uid_is_monotonic() {
        // 无法构造 MeshData（需 GPU buffer），改测 uid 计数器递增即可。
        let a = NEXT_MESH_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let b = NEXT_MESH_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert!(b > a);
    }
}