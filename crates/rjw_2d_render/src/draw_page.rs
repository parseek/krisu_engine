//! `DrawPage`：设备缓冲管理（顶点/索引）+ VP BindGroup + 管线缓存 + 统一绘制。

use std::{collections::HashMap, ops::Range};

use rjw_color::Color;
use rjw_transform::Transform2D;
use wgpu::util::DeviceExt;

use crate::data::{QUAD_TRI_INDICIES, QUAD_VERTS, SpriteRect, TriIndicies, VertexP3U2C4};
use crate::rstates::RStates;

// ─── 常量 ─────────────────────────────────────────────────────

/// 实例化上限：单次 `draw_indexed` 的实例数量
pub const MAX_INSTANCES_PER_DRAW: usize = 8192;

/// 深度/模板纹理格式
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

/// Mesh 顶点数上限（u16 索引）
pub(crate) const MAX_MESH_VERTS: usize = u16::MAX as usize;

/// 身份实例常量（静态网格 / 动态 Mesh 绘制时用作 slot1 占位，使 world pos 直通 VP）。
///
/// 注意：`uv_tl = [0,0]`、`uv_wh = [1,1]` —— 顶点自带 UV **直通**，不缩放不清零。
/// 这是动态 Mesh 正确采样纹理的关键（旧实现 `uv_wh = [0,0]` 会把 UV 清零导致无法贴图）。
const IDENTITY_INSTANCE: InstanceData = InstanceData {
    mesh_tl: [0.0, 0.0],
    mesh_wh: [1.0, 1.0],
    uv_tl: [0.0, 0.0],
    uv_wh: [1.0, 1.0],
    color: [1.0, 1.0, 1.0, 1.0],
    model: [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ],
};

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

    /// 身份实例：顶点即世界坐标、UV 直通、单位模型。
    /// 供动态 Mesh（`add_mesh*` 系列）与静态 Mesh 单实例占位使用。
    #[inline]
    pub(crate) fn identity() -> Self {
        IDENTITY_INSTANCE
    }

    /// 带 model 变换的 identity 实例（顶点为**局部坐标**，经 `model` 到世界）。
    /// 供动态 Mesh 带变换（`add_quads` / `add_mesh_transform`）使用——
    /// 移动窗口/物体只需改变换矩阵，顶点不变（可缓存）。
    #[inline]
    pub(crate) fn from_model(model: glam::Mat4) -> Self {
        let mut id = IDENTITY_INSTANCE;
        id.model = model.to_cols_array_2d();
        id
    }

    pub(crate) fn from_sprite(rect: &SpriteRect, color: Color, transform: Transform2D) -> Self {
        let (sin, cos) = transform.rotation.sin_cos();
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

    /// 高级 Sprite：直接传入列主序 Mat4，跳过 Transform2D → Mat4 推导。
    pub(crate) fn from_sprite_matrix(rect: &SpriteRect, color: Color, model: glam::Mat4) -> Self {
        Self {
            mesh_tl: rect.mesh_tl.to_array(),
            mesh_wh: rect.mesh_wh.to_array(),
            uv_tl: rect.uv_tl.to_array(),
            uv_wh: rect.uv_wh.to_array(),
            color: color.into(),
            model: model.to_cols_array_2d(),
        }
    }

    /// 静态网格（Transform2D）：顶点即世界坐标、UV 直通、应用变换矩阵。
    pub(crate) fn from_static_transform(color: Color, transform: Transform2D) -> Self {
        let (sin, cos) = transform.rotation.sin_cos();
        let model = glam::Mat4::from_cols_array_2d(&[
            [cos * transform.scale.x, sin * transform.scale.x, 0.0, 0.0],
            [-sin * transform.scale.y, cos * transform.scale.y, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [transform.pos.x, transform.pos.y, 0.0, 1.0],
        ]);
        Self {
            mesh_tl: [0.0, 0.0],
            mesh_wh: [1.0, 1.0],
            uv_tl: [0.0, 0.0],
            uv_wh: [1.0, 1.0],
            color: color.into(),
            model: model.to_cols_array_2d(),
        }
    }

    /// 静态网格（直接 Mat4）：顶点即世界坐标、UV 直通、应用列主序模型矩阵。
    pub(crate) fn from_static(color: Color, model: glam::Mat4) -> Self {
        Self {
            mesh_tl: [0.0, 0.0],
            mesh_wh: [1.0, 1.0],
            uv_tl: [0.0, 0.0],
            uv_wh: [1.0, 1.0],
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

/// 统一绘制操作：`prepare()` 阶段已 resolve `rstates` 为 `RStates::raw()`，
/// `draw()` 直接用于管线缓存查找。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DrawOp {
    /// 注册表网格（Sprite / StaticMesh）：`mesh_id` → `MESHES.get` 取顶点/索引缓冲。
    /// `index_range` 为静态网格恒定的 `0..index_count`。
    InstancedMesh {
        mesh_id: u64,
        /// 实例所在页
        page: u32,
        /// 页内实例范围
        instance_range: Range<u32>,
        /// 索引范围（静态网格 = 0..index_count）
        index_range: Range<u32>,
        rstates: u64,
        tex_uid: Option<u64>,
    },
    /// 动态缓冲段（`add_mesh*` 系列）：使用 `draw_page.mesh_vb / mesh_ib`。
    /// `index_range` 为动态段的三倍三角形范围。
    DynamicMesh {
        /// 实例所在页
        page: u32,
        /// 页内实例范围
        instance_range: Range<u32>,
        /// 索引范围（动态索引缓冲内）
        index_range: Range<u32>,
        rstates: u64,
        tex_uid: Option<u64>,
    },
    /// 外部自定义绘制调用（`draw()` 中执行）。
    /// `idx` 指向 `Render2D::buf_custom_draws`。
    Custom {
        idx: usize,
    },
}

/// GPU 缓冲页 + 管线缓存 + 身份实例缓冲
pub(crate) struct DrawPage {
    pub(crate) quad_vb: wgpu::Buffer,
    pub(crate) quad_ib: wgpu::Buffer,
    pub(crate) instance_pages: Vec<wgpu::Buffer>,
    pub(crate) vp_buffer: wgpu::Buffer,
    pub(crate) vp_bind_group: wgpu::BindGroup,

    // ── Mesh 动态缓冲 ──
    pub(crate) mesh_vb: wgpu::Buffer,
    pub(crate) mesh_ib: wgpu::Buffer,
    pub(crate) mesh_capacity_verts: usize,
    pub(crate) mesh_capacity_indices: usize,

    // ── 统一管线缓存（key = RStates::raw()） ──
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
    pipeline_cache: HashMap<u64, wgpu::RenderPipeline>,
}

impl DrawPage {
    pub(crate) fn new(
        device: &wgpu::Device,
        vp_bind_group_layout: &wgpu::BindGroupLayout,
        tex_bind_group_layout: &wgpu::BindGroupLayout,
        shader: wgpu::ShaderModule,
        surface_format: wgpu::TextureFormat,
        max_instances: usize,
        vp: glam::Mat4,
    ) -> Self {
        let quad_vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Render2D: Quad vb (Capacity {})", QUAD_VERTS.len())),
            contents: bytemuck::cast_slice(&QUAD_VERTS),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let quad_ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Render2D: Quad ib (Capacity {})", QUAD_TRI_INDICIES.len())),
            contents: bytemuck::cast_slice(&QUAD_TRI_INDICIES),
            usage: wgpu::BufferUsages::INDEX,
        });
        let instance_pages = vec![device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("Render2D: Instance page 0 (Capacity {})", max_instances)),
            size: (InstanceData::SIZE * max_instances) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })];
        let vp_data = VPBuffer {
            vp: vp.to_cols_array_2d(),
        };
        let vp_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Render2D: VP buffer"),
            contents: bytemuck::bytes_of(&vp_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let vp_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Render2D: VP bind group"),
            layout: vp_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: vp_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render2D: Unified pipeline layout"),
            bind_group_layouts: &[Some(vp_bind_group_layout), Some(tex_bind_group_layout)],
            immediate_size: 0,
        });

        let default_pipeline = Self::create_pipeline(
            device,
            &pipeline_layout,
            &shader,
            surface_format,
            RStates::default(),
        );

        let mesh_vb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Render2D: Mesh_vb"),
            size: 4,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mesh_ib = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Render2D: Mesh_ib"),
            size: 4,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut pipeline_cache = HashMap::with_capacity(8);
        pipeline_cache.insert(RStates::default().raw(), default_pipeline.clone());

        Self {
            quad_vb,
            quad_ib,
            instance_pages,
            vp_buffer,
            vp_bind_group,
            mesh_vb,
            mesh_ib,
            mesh_capacity_verts: 0,
            mesh_capacity_indices: 0,
            pipeline_layout,
            shader,
            surface_format,
            pipeline_cache,
        }
    }

    /// 获取或创建管线（按 RStates::raw() 缓存）。
    pub(crate) fn get_or_create_pipeline(
        &mut self,
        device: &wgpu::Device,
        raw: u64,
    ) -> &wgpu::RenderPipeline {
        use std::collections::hash_map::Entry;
        match self.pipeline_cache.entry(raw) {
            Entry::Occupied(e) => e.into_mut(),
            Entry::Vacant(e) => {
                let rst = RStates::from_raw(raw);
                let pipeline = Self::create_pipeline(
                    device,
                    &self.pipeline_layout,
                    &self.shader,
                    self.surface_format,
                    rst,
                );
                e.insert(pipeline)
            }
        }
    }

    fn create_pipeline(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        surface_format: wgpu::TextureFormat,
        rst: RStates,
    ) -> wgpu::RenderPipeline {
        let vertex_layout_quad = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<VertexP3U2C4>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x3,
                1 => Float32x2,
                2 => Float32x4,
            ],
        };
        let instance_attr_array = wgpu::vertex_attr_array![
            3 => Float32x2,
            4 => Float32x2,
            5 => Float32x2,
            6 => Float32x2,
            7 => Float32x4,
            8 => Float32x4,
            9 => Float32x4,
            10 => Float32x4,
            11 => Float32x4,
        ];
        let vertex_layout_instance = wgpu::VertexBufferLayout {
            array_stride: InstanceData::SIZE as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &instance_attr_array,
        };

        let blend = rst.to_blend();
        let depth_stencil = rst.to_depth_stencil();

        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render2D: Unified pipeline"),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout_quad), Some(vertex_layout_instance)],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: rst.to_front_face(),
                cull_mode: rst.to_cull(),
                unclipped_depth: false,
                polygon_mode: rst.to_polygon(),
                conservative: rst.to_conservative(),
            },
            depth_stencil,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    /// 更新 VP 缓冲（写整个矩阵）。
    pub(crate) fn update_vp(&self, queue: &wgpu::Queue, vp: glam::Mat4) {
        let vp_data = VPBuffer {
            vp: vp.to_cols_array_2d(),
        };
        queue.write_buffer(&self.vp_buffer, 0, bytemuck::bytes_of(&vp_data));
    }

    pub(crate) fn ensure_instance_pages(&mut self, device: &wgpu::Device, count: usize) {
        let existing = self.instance_pages.len();
        if count <= existing {
            return;
        }
        let size = (InstanceData::SIZE * MAX_INSTANCES_PER_DRAW) as u64;
        self.instance_pages.reserve(count - existing);
        for i in existing..count {
            self.instance_pages
                .push(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("Render2D: Instance page {i}")),
                    size,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
        }
    }

    pub(crate) fn update_instances_page(
        &self,
        queue: &wgpu::Queue,
        page: usize,
        instances: &[InstanceData],
    ) {
        if instances.is_empty() {
            return;
        }
        debug_assert!(
            page < self.instance_pages.len(),
            "instance page {page} out of range"
        );
        queue.write_buffer(
            &self.instance_pages[page],
            0,
            bytemuck::cast_slice(instances),
        );
    }

    pub(crate) fn instance_page_buffer(&self, page: usize) -> &wgpu::Buffer {
        &self.instance_pages[page]
    }

    pub(crate) fn ensure_mesh_capacity(
        &mut self,
        device: &wgpu::Device,
        verts: usize,
        tris: usize,
    ) {
        if verts > self.mesh_capacity_verts {
            let new_cap = (self.mesh_capacity_verts.max(1) * 2).max(verts);
            let size = (std::mem::size_of::<VertexP3U2C4>() * new_cap) as u64;
            self.mesh_vb = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("Render2D: Mesh_vb (Capacity {new_cap})")),
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
                label: Some(&format!("Render2D: Mesh_ib (Capacity {new_cap})")),
                size: size.max(4),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.mesh_capacity_indices = new_cap;
        }
    }
}