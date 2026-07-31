//! GPU 实例数据 / 缓冲页：实例化数据、VP、Mesh 动态缓冲与统一绘制操作。

use std::ops::Range;

use rjw_color::Color;
use rjw_transform::Transform2D;
use wgpu::util::DeviceExt;

use crate::data::{SpriteRect, TriIndicies, VertexP3U2C4, QUAD_TRI_INDICIES, QUAD_VERTS};

// ─── 常量 ─────────────────────────────────────────────────────

/// 实例化上限：单次 `draw_indexed` 的实例数量
pub const MAX_INSTANCES_PER_DRAW: usize = 4096;

/// 深度/模板纹理格式
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

/// Mesh 顶点数上限（u16 索引）
pub(crate) const MAX_MESH_VERTS: usize = u16::MAX as usize;

// ─── GPU 实例数据 / 缓冲页 ────────────────────────────────────

/// 实例数据（对应 shader 中 @location(3..11)）
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub(crate) struct InstanceData {
    /// 网格左上角（世界坐标）
    pub(crate) mesh_tl: [f32; 2],
    /// 网格尺寸（世界坐标）
    pub(crate) mesh_wh: [f32; 2],
    /// 已归一化 UV 左上角
    pub(crate) uv_tl: [f32; 2],
    /// 已归一化 UV 尺寸
    pub(crate) uv_wh: [f32; 2],
    pub(crate) color: [f32; 4],
    /// model 变换（列主序）
    pub(crate) model: [[f32; 4]; 4],
}

impl InstanceData {
    pub(crate) const SIZE: usize = std::mem::size_of::<Self>();

    pub(crate) fn from_sprite(rect: &SpriteRect, color: Color, transform: Transform2D) -> Self {
        let (sin, cos) = transform.rotation.sin_cos();
        // 2D 变换矩阵（列主序）：
        //   [cos*sx, sin*sx]  [-sin*sy, cos*sy]  [pos.x, pos.y]
        let model = glam::Mat4::from_cols_array_2d(&[
            [cos * transform.scale.x, sin * transform.scale.x, 0.0, 0.0],
            [-sin * transform.scale.y, cos * transform.scale.y, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [transform.pos.x, transform.pos.y, 0.0, 1.0],
        ]);
        Self {
            mesh_tl: rect.mesh_tl.to_array(),
            mesh_wh: rect.mesh_wh.to_array(),
            uv_tl: rect.uv_tl.to_array(),
            uv_wh: rect.uv_wh.to_array(),
            color: color.into(),
            model: model.to_cols_array_2d(),
        }
    }
}

/// 全局 VP（视图投影）矩阵
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub(crate) struct VPBuffer {
    vp: [[f32; 4]; 4],
}

/// 一帧中单个 Mesh 在 DrawPage 动态缓冲中的定位（由 `prepare` 生成）
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MeshDrawItem {
    /// 在 `DrawPage::mesh_vb` 中的首顶点偏移（元素数；预留：无索引时用 `draw` 定位）
    #[allow(dead_code)]
    pub(crate) first_vertex: u32,
    /// 顶点数量（肯定不会超过 65535；预留）
    #[allow(dead_code)]
    pub(crate) vertex_count: u16,
    /// 在 `DrawPage::mesh_ib` 中的起始**三角形**游标（TriIndicies 元素数；×3 得 draw_indexed 索引起始）
    pub(crate) tri_index_start: u32,
    /// 三角形数量（TriIndicies 元素数；×3 得 draw_indexed 索引数）
    pub(crate) tri_index_count: u16,
}

/// 统一绘制操作：由 `prepare()` 按 (layer, states) 排序后生成，
/// Sprite 批次与 Mesh **交错**排列，`draw()` 严格按此顺序切换管线 /
/// 绑定组 / 顶点缓冲，保证跨类别（Sprite ↔ Mesh）的层级（layer）正确。
///（`DrawOp::Sprite.range` 为 `Range<u32>`，不 `Copy`，故这里用 `Clone`。）
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DrawOp {
    /// 实例化 sprite 批次：纹理 + 实例范围（`buf_instances` 下标；实例缓冲已按序全量写入）
    Sprite {
        tex_uid: Option<u64>,
        range: Range<u32>,
    },
    /// 非实例化 mesh：指向 `mesh_vb`/`mesh_ib` 中重定位后的数据
    Mesh {
        item: MeshDrawItem,
    },
    /// 占位标记：在 mesh 组装阶段被替换为 `DrawOp::Mesh`（保持 op 顺序与排序一致）
    MeshPlaceholder,
}

/// GPU 缓冲页：四边形 + 实例化 + VP + Mesh 动态缓冲
pub(crate) struct DrawPage {
    /// 不可变单位四边形顶点
    pub(crate) quad_vb: wgpu::Buffer,
    /// 不可变四边形索引
    pub(crate) quad_ib: wgpu::Buffer,
    /// 动态实例缓冲（实例化渲染）
    pub(crate) instance_buffer: wgpu::Buffer,
    /// 动态 VP 矩阵缓冲
    pub(crate) vp_buffer: wgpu::Buffer,
    /// VP 绑定组
    pub(crate) vp_bind_group: wgpu::BindGroup,

    // ── Mesh 动态缓冲（按排序顺序拷贝/重定位） ──
    /// 动态 Mesh 顶点缓冲
    pub(crate) mesh_vb: wgpu::Buffer,
    /// 动态 Mesh 索引缓冲
    pub(crate) mesh_ib: wgpu::Buffer,
    /// 当前容量（顶点元素数）
    pub(crate) mesh_capacity_verts: usize,
    /// 当前容量（索引元素数）
    pub(crate) mesh_capacity_indices: usize,
}

impl DrawPage {
    /// 创建 GPU 缓冲页。
    ///
    /// `vp_bind_group_layout`: group(0) 的绑定组布局（仅 uniform VP）。
    pub(crate) fn new(
        device: &wgpu::Device,
        vp_bind_group_layout: &wgpu::BindGroupLayout,
        max_instances: usize,
        vp: glam::Mat4,
    ) -> Self {
        let quad_vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad vb"),
            contents: bytemuck::cast_slice(&QUAD_VERTS),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let quad_ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad ib"),
            contents: bytemuck::cast_slice(&QUAD_TRI_INDICIES),
            usage: wgpu::BufferUsages::INDEX,
        });
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance buffer"),
            size: (InstanceData::SIZE * max_instances) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vp_data = VPBuffer { vp: vp.to_cols_array_2d() };
        let vp_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vp buffer"),
            contents: bytemuck::bytes_of(&vp_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let vp_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vp bind group"),
            layout: vp_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: vp_buffer.as_entire_binding(),
            }],
        });
        // Mesh 动态缓冲初始容量 0（首次使用前由 ensure_mesh_capacity 创建）。
        let mesh_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh vb"),
            size: 4,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mesh_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh ib"),
            size: 4,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            quad_vb,
            quad_ib,
            instance_buffer,
            vp_buffer,
            vp_bind_group,
            mesh_vb,
            mesh_ib,
            mesh_capacity_verts: 0,
            mesh_capacity_indices: 0,
        }
    }

    /// 更新 VP 缓冲（写整个矩阵）。
    pub(crate) fn update_vp(&self, queue: &wgpu::Queue, vp: glam::Mat4) {
        let vp_data = VPBuffer { vp: vp.to_cols_array_2d() };
        queue.write_buffer(&self.vp_buffer, 0, bytemuck::bytes_of(&vp_data));
    }

    /// 更新实例数据（`instances.len()` 必须 ≤ 缓冲容量）。
    pub(crate) fn update_instances(&self, queue: &wgpu::Queue, instances: &[InstanceData]) {
        if instances.is_empty() {
            return;
        }
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(instances));
    }

    /// 确保 Mesh 动态缓冲容量可容纳 `verts` 个顶点与 `tris` 个三角形（按 2× 扩容重建）。
    pub(crate) fn ensure_mesh_capacity(&mut self, device: &wgpu::Device, verts: usize, tris: usize) {
        if verts > self.mesh_capacity_verts {
            let new_cap = (self.mesh_capacity_verts.max(1) * 2).max(verts);
            let size = (std::mem::size_of::<VertexP3U2C4>() * new_cap) as u64;
            self.mesh_vb = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh vb"),
                size: size.max(4),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.mesh_capacity_verts = new_cap;
        }
        if tris > self.mesh_capacity_indices {
            let new_cap = (self.mesh_capacity_indices.max(1) * 2).max(tris);
            let size = (std::mem::size_of::<TriIndicies>() * new_cap) as u64;
            self.mesh_ib = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mesh ib"),
                size: size.max(4),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.mesh_capacity_indices = new_cap;
        }
    }
}