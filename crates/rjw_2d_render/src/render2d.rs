//! `Render2D`：Batch2D 渲染器主体 + Clear 配置。

use std::{ops::Range, sync::Arc};

use rjw_color::Color;
use rjw_render::{ArcTextureWrapped, TextureWrapped};
use rjw_transform::Transform2D;

use crate::command::{DrawCommand, DrawCommandQueue, Layer, States};
use crate::data::{
    Index, MeshSink, MeshStorage, SpriteRect, TriIndicies, VertexP3U2C4, QUAD_TRI_INDICIES,
};
use crate::draw_page::{
    DrawOp, DrawPage, InstanceData, MAX_INSTANCES_PER_DRAW, MAX_MESH_VERTS, DEPTH_FORMAT,
};

// ─── Clear 配置 ───────────────────────────────────────────────

/// 每帧 Clear/Load 配置（`Render2D::render` 使用）。
///
/// - `color`:  `Some(c)` = 用 `c` 清屏；`None` = 保留旧内容（Load）。
/// - `depth`:  `Some(d)` = 清深度为 `d`；`None` = 不碰深度。需要时自动创建深度纹理。
/// - `stencil`: `Some(s)` = 清模板为 `s`；`None` = 不碰模板。需要时自动创建深度纹理。
#[derive(Debug, Clone, Copy)]
pub struct ClearConfig {
    pub color: Option<wgpu::Color>,
    pub depth: Option<f32>,
    pub stencil: Option<u32>,
}

impl Default for ClearConfig {
    fn default() -> Self {
        Self {
            color: Some(wgpu::Color::TRANSPARENT),
            depth: None,
            stencil: None,
        }
    }
}

// ─── Render2D ─────────────────────────────────────────────────

/// Batch2D 渲染器：命令录制 → 排序合批 → 统一提交
///
/// 从 `rjw_render::RenderContext` 构建（clone device/queue、借用 surface）。
/// - `render(&ClearConfig)`：自动创建 RenderPass（可选 Clear color/depth/stencil），提交并呈现。
/// - `flush(&mut pass)`：用户自行创建 RenderPass 后调用，仅录制绘制命令。
/// - `set_mvp(Mat4)`：直接透传外部 View-Projection（例如 `Camera2D::vp_matrix()`，列主序）。
///
/// 性能：所有排序/聚合缓冲（`buf_*`）为常驻字段，每帧 `clear()` 复用容量，
/// 避免运行时堆分配。
pub struct Render2D {
    surface: &'static wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,

    /// Sprite 实例化管线（slot0 四边形 + slot1 实例数据）
    sprite_pipeline: wgpu::RenderPipeline,
    /// Mesh/Polygon 非实例化管线（仅 slot0，`vs_mesh` 世界坐标直通 VP）
    mesh_pipeline: wgpu::RenderPipeline,
    tex_bind_group_layout: wgpu::BindGroupLayout,
    white_texture: ArcTextureWrapped,
    textures: Vec<ArcTextureWrapped>,

    mesh_storage: MeshStorage,
    command_queue: DrawCommandQueue,
    draw_page: DrawPage,

    /// 懒创建深度/模板纹理及其视图
    depth_view: Option<wgpu::TextureView>,
    depth_size: (u32, u32),

    /// 当前 VP（列主序，由 `set_mvp` 透传）。
    #[allow(unused)]
    mvp: glam::Mat4,

    // ── 每帧复用缓冲池（避免堆分配） ──
    /// 排序后的 Mesh 命令（CPU 顶点/索引范围，顺序与 `buf_ops` 中 Mesh 项一致）
    buf_mesh_cmds: Vec<(Range<usize>, Range<usize>)>,
    /// 聚合后的实例数据
    buf_instances: Vec<InstanceData>,
    /// **统一绘制操作序列**（Sprite 批次与 Mesh 交错，按 (layer, states) 排序）。
    /// 这是唯一决定 `draw()` 绘制顺序的数据结构。
    buf_ops: Vec<DrawOp>,
    /// 组装后的 Mesh 顶点（排序顺序）
    buf_all_verts: Vec<VertexP3U2C4>,
    /// 组装后的 Mesh 重定位索引
    buf_all_tris: Vec<TriIndicies>,
    /// 索引字节缓冲（含对齐补 0）
    buf_padded: Vec<u8>,
}

impl Render2D {
    /// 创建渲染器，基于 `RenderContext`。
    ///
    /// # Safety
    ///
    /// `render` 必须比返回的 `Render2D` 存活更久（Render2D 持有 `'static` 表面引用）。
    /// 在使用 `rjw_render` 框架时，RenderContext 存活至事件循环结束，天然满足。
    pub fn new(render: &rjw_render::RenderContext) -> Self {
        let device = render.device().clone();
        let queue = render.queue().clone();
        let surface_format = render.format();

        // ⚠️ surface 引用提升为 'static：由 RenderContext 的所有者保证存活期。
        // 参考 rjw_render::RenderContext::new 中相同的 transmute 约定。
        let surface: &'static wgpu::Surface<'static> =
            unsafe { std::mem::transmute(render.surface()) };

        let vp_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vp bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let tex_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // wgpu 30: bind_group_layouts 为 Option 数组，push_constant_ranges 被 immediate_size 取代。
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite pipeline layout"),
            bind_group_layouts: &[Some(&vp_bind_group_layout), Some(&tex_bind_group_layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
        });

        // 顶点缓冲布局：
        //   slot 0: 单位四边形（逐顶点）pos/uv/color（也用于 Mesh 顶点）
        //   slot 1: 实例数据 mesh_tl/mesh_wh/uv_tl/uv_wh/color/model
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

        let sprite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout_quad.clone()), Some(vertex_layout_instance)],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // Mesh/Polygon 非实例化管线：仅 slot0（pos/uv/color），entry=`vs_mesh`（世界坐标直通 VP）。
        // ⚠️ 不能复用 sprite_pipeline：其 vs_main 用 mesh_tl/mesh_wh 需要 slot1 实例数据。
        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_mesh"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout_quad)],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // 1×1 白色默认纹理（纯色 sprite / mesh 使用）。
        let white_texture = Arc::new(TextureWrapped::from_rgba8(
            &device,
            &queue,
            &tex_bind_group_layout,
            "white",
            &[255, 255, 255, 255],
            1,
            1,
        ));

        let draw_page =
            DrawPage::new(&device, &vp_bind_group_layout, MAX_INSTANCES_PER_DRAW, glam::Mat4::IDENTITY);

        Self {
            surface,
            device,
            queue,
            sprite_pipeline,
            mesh_pipeline,
            tex_bind_group_layout,
            white_texture,
            textures: Vec::new(),
            mesh_storage: MeshStorage::default(),
            command_queue: DrawCommandQueue::default(),
            draw_page,
            depth_view: None,
            depth_size: (0, 0),
            mvp: glam::Mat4::IDENTITY,
            buf_mesh_cmds: Vec::new(),
            buf_instances: Vec::new(),
            buf_ops: Vec::new(),
            buf_all_verts: Vec::new(),
            buf_all_tris: Vec::new(),
            buf_padded: Vec::new(),
        }
    }

    /// 设置 View-Projection 矩阵（列主序），由外部相机提供（例如 `Camera2D::vp_matrix()`）。
    ///
    /// 直接透传：坐标系由相机保证（原点中心、X+ 右、Y+ 下），渲染器不再做任何翻转。
    pub fn set_mvp(&mut self, vp: glam::Mat4) {
        self.mvp = vp;
        self.draw_page.update_vp(&self.queue, vp);
    }

    /// 从 RGBA8 字节数据创建 Render2D 使用的纹理（宽高 1 像素 = 4 字节）
    ///
    /// # Panics
    ///
    /// 当 `data.len() != width * height * 4` 时 panic（用 usize 运算避免
    /// u32 乘法在超大尺寸下溢出回绕导致长度校验失效）。
    pub fn create_texture(&mut self, label: &str, data: &[u8], width: u32, height: u32) -> ArcTextureWrapped {
        let expected = (width as usize) * (height as usize) * 4;
        assert_eq!(
            data.len(),
            expected,
            "RGBA8 data length mismatch: expected {expected} (w*h*4), got {} (label={label})",
            data.len()
        );
        let tex = Arc::new(TextureWrapped::from_rgba8(
            &self.device,
            &self.queue,
            &self.tex_bind_group_layout,
            label,
            data,
            width,
            height,
        ));
        self.textures.push(tex.clone());
        tex
    }

    // ── 绘制命令录制 ──

    /// 录制一个带纹理的 Sprite（实例化渲染、可合批）
    pub fn add_sprite2d(
        &mut self,
        rect: SpriteRect,
        color: Color,
        transform: Transform2D,
        layer: impl Into<Layer>,
        texture: &ArcTextureWrapped,
    ) {
        let uid = texture.uid;
        self.command_queue.push(
            DrawCommand::Sprite2D { rect, color, transform },
            layer.into(),
            Some(States { texture_uid: Some(uid) }),
        );
    }

    /// 录制一个纯色 Sprite（`add_sprite2d` 的包装：使用默认白色纹理）
    pub fn add_sprite2d_solid(
        &mut self,
        rect: SpriteRect,
        color: Color,
        transform: Transform2D,
        layer: impl Into<Layer>,
    ) {
        let white = self.white_texture.clone();
        self.add_sprite2d(rect, color, transform, layer, &white);
    }

    /// 录制一个通用网格（Mesh，非实例化路径）。
    ///
    /// - `vertices`：世界坐标顶点
    /// - `tri_indices`：三角形索引（**局部索引**，相对本 mesh 从 0 起；长度 = 三角形数 × 3）
    ///
    /// 顶点/索引暂存于 CPU（`MeshStorage`），`prepare()` 按排序顺序组装重定位后
    /// 一次性拷入 `DrawPage::mesh_vb`/`mesh_ib`。
    pub fn add_mesh(
        &mut self,
        vertices: &[glam::Vec2],
        tri_indices: &[u16],
        color: Color,
        layer: impl Into<Layer>,
    ) {
        // ⚠️ 使用 `assert`（而非 `debug_assert`）：
        //    索引越界会在后续 `as u16` 截断 / slice 索引阶段产生不确定行为
        //    （release 下 `as u16` 静默回绕、slice 越界 panic），显式失败更安全。
        assert!(vertices.len() > 0, "mesh requires at least 1 vertex");
        assert!(tri_indices.len() % 3 == 0, "tri_indices length must be a multiple of 3");
        assert!(
            tri_indices.iter().all(|&i| (i as usize) < vertices.len()),
            "tri_indices out of bounds for vertices"
        );
        assert!(
            vertices.len() as u64 <= u16::MAX as u64 + 1,
            "single mesh has too many vertices for u16 indices: {} (max {})",
            vertices.len(),
            u16::MAX as u64 + 1
        );

        let vert_start = self.mesh_storage.vertices.len();
        let tri_start = self.mesh_storage.tri_indices.len();
        let color_arr: [f32; 4] = color.into();

        // 顶点 → 全局（含颜色）。
        for p in vertices {
            self.mesh_storage.vertices.push(VertexP3U2C4 {
                pos: [p.x, p.y, 0.0],
                uv: [0.0, 0.0],
                color: color_arr,
            });
        }
        // 局部索引 → 全局索引（+base）。
        // ⚠️ 用 u32 加法再截断，避免 debug 模式 u16 overflow panic。
        for chunk in tri_indices.chunks_exact(3) {
            self.mesh_storage.tri_indices.push(TriIndicies(
                Index((chunk[0] as u32 + vert_start as u32) as u16),
                Index((chunk[1] as u32 + vert_start as u32) as u16),
                Index((chunk[2] as u32 + vert_start as u32) as u16),
            ));
        }

        let vert_range = vert_start..self.mesh_storage.vertices.len();
        let tri_range = tri_start..self.mesh_storage.tri_indices.len();
        self.command_queue.push(
            DrawCommand::Mesh { vert: vert_range, tri_index: tri_range },
            layer.into(),
            Some(States { texture_uid: None }),
        );
    }

    /// 录制一个通用网格（Mesh，非实例化路径），以**预分配可变切片**填充，零帧内堆分配。
    ///
    /// - `max_vertices`: 本 mesh 所需的顶点数上限
    /// - `max_triangles`: 本 mesh 所需的三角形数上限
    /// - `f`: 闭包，接收 **Storage 中预先分配好的** `&mut [VertexP3U2C4]`（长度 =
    ///   `max_vertices`）与 `&mut [TriIndicies]`（长度 = `max_triangles`）两个可变切片。
    ///   闭包直接写入这两块**复用缓冲**中的内存（不产生分配），并返回实际填充的
    ///   `(顶点数, 三角形数)`。
    ///
    /// 约定：
    /// - 顶点切片写入的顶点**位置为世界坐标**，颜色统一取 `color`。
    /// - 三角形切片写入的索引为**局部索引**（相对本 mesh 从 0 起），
    ///   方法内部会自动把实际使用的 `[0, used_tris)` 重定位为全局索引。
    ///
    /// 与 `add_mesh_fn` 的关系：本方法适合**已知顶点/三角形数量**的网格（例如圆环、
    /// 格子等规则网格），闭包直接写内存避免 push 调用开销；Storage 容量不足时
    /// 仅在该帧一次性扩容（`resize`），后续帧复用容量、零分配。
    pub fn add_mesh_fn_prealloc<F>(
        &mut self,
        max_vertices: usize,
        max_triangles: usize,
        color: Color,
        layer: impl Into<Layer>,
        f: F,
    ) where
        F: FnOnce(&mut [VertexP3U2C4], &mut [TriIndicies]) -> (usize, usize),
    {
        assert!(max_vertices > 0, "add_mesh_fn_prealloc requires at least 1 vertex");
        assert!(
            max_vertices <= MAX_MESH_VERTS,
            "mesh vertex count exceeds u16 limit: {max_vertices} > {MAX_MESH_VERTS}"
        );
        debug_assert!(
            max_vertices as u64 <= u16::MAX as u64 + 1,
            "single mesh has too many vertices for u16 indices: {max_vertices} (max {})",
            u16::MAX as u64 + 1
        );

        let v_off = self.mesh_storage.vertices.len();
        let i_off = self.mesh_storage.tri_indices.len();
        let color_arr: [f32; 4] = color.into();

        // 在常驻 Storage 中预留本 mesh 的容量（保留 capacity，容量充足时 resize 不分配）。
        self.mesh_storage.vertices.resize(v_off + max_vertices, VertexP3U2C4::default());
        self.mesh_storage.tri_indices.resize(i_off + max_triangles, TriIndicies::default());

        let (used_verts, used_tris) = {
            // 同时可变借用不同字段（合法）；两块切片为 Storage 内存的直接视图。
            let v_slice: &mut [VertexP3U2C4] =
                &mut self.mesh_storage.vertices[v_off..v_off + max_vertices];
            let i_slice: &mut [TriIndicies] =
                &mut self.mesh_storage.tri_indices[i_off..i_off + max_triangles];
            f(v_slice, i_slice)
        };
        debug_assert!(used_verts <= max_vertices, "closure filled more vertices than declared");
        debug_assert!(used_tris <= max_triangles, "closure filled more triangles than declared");

        // 截断到实际使用长度（未使用部分丢弃）。
        self.mesh_storage.vertices.truncate(v_off + used_verts);
        self.mesh_storage.tri_indices.truncate(i_off + used_tris);

        // 统一填充颜色（与 add_mesh / add_mesh_fn 一致：颜色取录制传入的 `color`；
        // 闭包只需写 `pos`/`uv`，颜色由内部批量写入）。
        for v in &mut self.mesh_storage.vertices[v_off..v_off + used_verts] {
            v.color = color_arr;
        }

        // 重定位：局部索引 → 全局索引（+v_off）。
        // ⚠️ 用 u32 加法再截断，避免 debug 模式 u16 overflow panic。
        if used_tris != 0 {
            let base = v_off as u32;
            for t in &mut self.mesh_storage.tri_indices[i_off..i_off + used_tris] {
                *t = TriIndicies(
                    Index((t.0 .0 as u32 + base) as u16),
                    Index((t.1 .0 as u32 + base) as u16),
                    Index((t.2 .0 as u32 + base) as u16),
                );
            }
        }

        self.command_queue.push(
            DrawCommand::Mesh {
                vert: v_off..v_off + used_verts,
                tri_index: i_off..i_off + used_tris,
            },
            layer.into(),
            Some(States { texture_uid: None }),
        );
    }

    /// 录制一个通用网格（Mesh，非实例化路径），闭包通过**安全 push 封装** `MeshSink` 写入。
    ///
    /// - 闭包接收 `&mut MeshSink`，调用 `push_vertex(pos)`（返回局部顶点索引）与
    ///   `push_tri(a, b, c)`（自动重定位为全局索引）即可构建任意网格。
    /// - 所有写入直接 push 到常驻 `MeshStorage`（复用容量），不产生每帧临时堆分配；
    ///   Storage 容量不足时按需扩容（保留 capacity，后续帧复用）。
    ///
    /// 与 `add_mesh_fn_prealloc` 的关系：本方法**不需要**预先知道顶点/三角形数量，
    /// 适合流程式构建（例如多边形扇/条带），以轻量 push 调用换取灵活性。
    pub fn add_mesh_fn<F>(
        &mut self,
        color: Color,
        layer: impl Into<Layer>,
        f: F,
    ) where
        F: FnOnce(&mut MeshSink<'_>),
    {
        let vert_start = self.mesh_storage.vertices.len();
        let tri_start = self.mesh_storage.tri_indices.len();
        let color_arr: [f32; 4] = color.into();

        {
            let mut sink = MeshSink {
                base: vert_start as u32,
                verts: &mut self.mesh_storage.vertices,
                tris: &mut self.mesh_storage.tri_indices,
                color_arr,
            };
            f(&mut sink);
        }

        let vert_count = self.mesh_storage.vertices.len() - vert_start;
        assert!(
            vert_count <= u16::MAX as usize + 1,
            "single mesh has too many vertices for u16 indices: {vert_count} (max {})",
            u16::MAX as usize + 1
        );

        let vert_range = vert_start..self.mesh_storage.vertices.len();
        let tri_range = tri_start..self.mesh_storage.tri_indices.len();
        self.command_queue.push(
            DrawCommand::Mesh { vert: vert_range, tri_index: tri_range },
            layer.into(),
            Some(States { texture_uid: None }),
        );
    }

    /// 录制一个多边形（`add_mesh_fn_prealloc` 的便捷包装）。
    ///
    /// 只要顶点（世界坐标、多边形、按逆时针/顺时针环绕），
    /// 内部按三角形扇（fan）自动生成索引后走 Mesh 路径。要求 `vertices.len() >= 3`。
    ///
    /// 无每帧临时堆分配：索引直接写入常驻 `MeshStorage` 预分配切片（复用容量）。
    pub fn add_polygon_fan(
        &mut self,
        vertices: &[glam::Vec2],
        color: Color,
        layer: impl Into<Layer>,
    ) {
        debug_assert!(vertices.len() >= 3, "polygon requires at least 3 vertices");
        let n = vertices.len();
        // 顶点数 = n；三角形扇三角形数 = n - 2。闭包直接写预分配切片（零 push 开销）。
        self.add_mesh_fn_prealloc(n, n - 2, color, layer, |v_slice, t_slice| {
            for (dst, src) in v_slice.iter_mut().zip(vertices.iter()) {
                dst.pos = [src.x, src.y, 0.0];
            }
            // 三角形扇（局部索引）：(0, i, i+1)
            for (i, t) in t_slice.iter_mut().enumerate() {
                *t = TriIndicies::new(0, (i + 1) as u16, (i + 2) as u16);
            }
            (n, n - 2)
        });
    }

    /// 录制一个多边形（`add_mesh_fn_prealloc` 的便捷包装）。
    ///
    /// 只要顶点（世界坐标、多边形、按逆时针/顺时针环绕），
    /// 内部按三角形扇（strip）自动生成索引后走 Mesh 路径。要求 `vertices.len() >= 3`。
    ///
    /// 无每帧临时堆分配：索引直接写入常驻 `MeshStorage` 预分配切片（复用容量）。
    pub fn add_polygon_strip(
        &mut self,
        vertices: &[glam::Vec2],
        color: Color,
        layer: impl Into<Layer>,
    ) {
        debug_assert!(vertices.len() >= 3, "polygon requires at least 3 vertices");
        let n = vertices.len();
        // 兼容既有行为：原实现与 fan 相同（(0, i, i+1) 三角形扇）。
        self.add_mesh_fn_prealloc(n, n - 2, color, layer, |v_slice, t_slice| {
            for (dst, src) in v_slice.iter_mut().zip(vertices.iter()) {
                dst.pos = [src.x, src.y, 0.0];
            }
            for (i, t) in t_slice.iter_mut().enumerate() {
                *t = TriIndicies::new(0, (i + 1) as u16, (i + 2) as u16);
            }
            (n, n - 2)
        });
    }

    /// 采样 1×1 白色纹理（纯色绘制用）。
    #[allow(unused)]
    pub fn white_texture(&self) -> &ArcTextureWrapped {
        &self.white_texture
    }

    // ── 提交 ──

    /// 全流程渲染：`begin_frame` → 创建 RenderPass（按 `clear` 决定 Clear/Load）→ `flush` → 提交并呈现。
    ///
    /// 清理配置：
    /// - `clear.color == Some(c)` → LoadOp::Clear(c)；`None` → Load
    /// - `clear.depth / stencil` 为 `Some` 时自动创建（并按需重建）`Depth24PlusStencil8` 深度纹理
    pub fn render(&mut self, clear: &ClearConfig) {
        let Some((surface_texture, view)) = self.begin_frame() else {
            // 无可呈现表面时依然清空命令队列与 Mesh 暂存，避免脏数据残留。
            self.command_queue.clear();
            self.mesh_storage.clear();
            return;
        };

        // 1. 排序 + 聚合（写实例/Mesh 缓冲，复用常驻池）。
        self.prepare();

        // 2. 创建 encoder 与 RenderPass（深度纹理按需懒建）。
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render2d encoder"),
        });

        let needs_depth = clear.depth.is_some() || clear.stencil.is_some();
        let size = self
            .surface
            .get_configuration()
            .map(|c| (c.width, c.height))
            .unwrap_or((1, 1));
        if needs_depth {
            self.ensure_depth(size.0, size.1);
        }
        let depth_view = if needs_depth { self.depth_view.as_ref() } else { None };

        {
            let color_ops = match clear.color {
                Some(c) => wgpu::Operations { load: wgpu::LoadOp::Clear(c), store: wgpu::StoreOp::Store },
                None => wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            };

            let depth_stencil_attachment = match depth_view {
                Some(dv) => Some(wgpu::RenderPassDepthStencilAttachment {
                    view: dv,
                    depth_ops: clear.depth.map(|d| wgpu::Operations {
                        load: wgpu::LoadOp::Clear(d),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: clear.stencil.map(|s| wgpu::Operations {
                        load: wgpu::LoadOp::Clear(s),
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                None => None,
            };

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render2d pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: color_ops,
                })],
                depth_stencil_attachment,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });

            self.draw(&mut pass);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        self.queue.present(surface_texture);
        // 帧结束：清理命令队列与 Mesh 暂存（clear 复用容量，仅重置长度），等待下一帧录制。
        self.command_queue.clear();
        self.mesh_storage.clear();
    }

    /// 由用户自行创建 RenderPass 后调用：排序 + 合批 + 录制绘制命令。
    ///
    /// 不负责提交/呈现（由调用方管理 encoder 与 present）。
    /// 建议先调用 `begin_frame()` 获取视图，自行构建 pass：
    /// ```ignore
    /// let (tex, view) = render2d.begin_frame()?;
    /// let mut encoder = ...;
    /// let mut pass = encoder.begin_render_pass(&desc_containing_view);
    /// render2d.flush(&mut pass);
    /// queue.submit(...); queue.present(tex);
    /// ```
    pub fn flush(&mut self, pass: &mut wgpu::RenderPass<'_>) {
        self.prepare();
        self.draw(pass);
        // 由调用方提交/呈现；此处只清理队列与 Mesh 暂存（clear 复用容量，仅重置长度）。
        self.command_queue.clear();
        self.mesh_storage.clear();
    }

    /// 获取当前帧表面纹理 + 视图。若表面不可呈现返回 `None`。
    pub fn begin_frame(&mut self) -> Option<(wgpu::SurfaceTexture, wgpu::TextureView)> {
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            // 其他情况（Outdated/Lost/Timeout/Occluded/Validation）跳过本帧。
            _ => return None,
        };
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        Some((texture, view))
    }

    // ── 内部：排序 / 聚合 / 绘制（全部复用常驻 Vec，避免运行时堆分配） ──

    /// 排序命令并聚合实例/Mesh 数据到常驻缓冲池。
    ///
    /// Mesh 流程：按排序后的顺序，将每个 Mesh 的顶点从 CPU 暂存**按序组装**到
    /// `buf_all_verts`/`buf_all_tris`（常驻，`clear()` 复用容量），随后整批写入
    /// `DrawPage::mesh_vb`/`mesh_ib`（索引重定位到排序后偏移；字节数补 0 垫整以满足
    /// COPY_BUFFER_ALIGNMENT）。总容量不足时先统计总量一次性扩容。
    fn prepare(&mut self) {
        self.command_queue.sort_layer_then_states();

        // 清空复用池（保留 capacity）。
        self.buf_mesh_cmds.clear();
        self.buf_instances.clear();
        self.buf_ops.clear();
        self.buf_all_verts.clear();
        self.buf_all_tris.clear();

        // ── 第一遍：按排序顺序生成统一绘制操作序列（DrawOp） ──
        // Sprite 连续同纹理合并为一个 DrawOp::Sprite 批次；
        // Mesh 在序列中插入占位符（稍后替换为 DrawOp::Mesh）——
        // 因此 buf_ops 中 Sprite/Mesh 严格按 (layer, states) 排序交错，
        // 修复「先画全部 Sprite 再画全部 Mesh → Mesh 恒盖 Sprite」的层级 bug。
        // 实例按顺序**分页摊入**：每页容量固定 = MAX_INSTANCES_PER_DRAW。
        // `page` = 当前批次所在页；`page_start` = 该页起始的全局实例序号；
        // `run_start` = 当前同纹理 run 起始的全局实例序号（≤ page 内容量）。
        // DrawOp::Sprite 记录 **页号 + 页内范围**（相对该页缓冲，单批 ≤ 一页）。
        // 跨页切点仅在「当前页已满」时发生——页与页之间数据独立、每页只写一次，
        // 规避 `Queue::write_buffer` 在 submit 前全部执行引起的同缓冲覆盖问题，
        // 同时天然支持单帧总实例数 > 4096。
        let mut current_tex: Option<u64> = None;
        let mut run_start: usize = 0;
        let mut page: u32 = 0;
        let mut page_start: usize = 0;

        /// 结批辅助：把 `[run_start, buf_instances.len())` 生成 DrawOp::Sprite（页内相对范围）。
        macro_rules! close_run {
            ($tex:expr) => {{
                if self.buf_instances.len() > run_start {
                    self.buf_ops.push(DrawOp::Sprite {
                        page,
                        tex_uid: $tex,
                        range: ((run_start - page_start) as u32)..((self.buf_instances.len() - page_start) as u32),
                    });
                }
            }};
        }

        for (cmd, _layer, states) in self.command_queue.iter() {
            let tex_uid = states.and_then(|s| s.texture_uid);
            match cmd {
                DrawCommand::Sprite2D { rect, color, transform } => {
                    // 状态变化 或 当前页已满 → 先结批。
                    let page_full = self.buf_instances.len() - page_start >= MAX_INSTANCES_PER_DRAW;
                    if tex_uid != current_tex || page_full {
                        close_run!(current_tex);
                        current_tex = tex_uid;
                        run_start = self.buf_instances.len();
                        if page_full {
                            page += 1;
                            page_start = self.buf_instances.len();
                        }
                    }
                    let data = InstanceData::from_sprite(rect, *color, *transform);
                    self.buf_instances.push(data);
                }
                DrawCommand::Mesh { vert, tri_index } => {
                    // Mesh 非实例化：先结批实例，再登记 Mesh 命令 + 占位符。
                    close_run!(current_tex);
                    current_tex = None;
                    run_start = self.buf_instances.len();
                    self.buf_ops.push(DrawOp::MeshPlaceholder);
                    self.buf_mesh_cmds.push((vert.clone(), tri_index.clone()));
                }
            }
        }
        // 收尾最后一批。
        close_run!(current_tex);

        // 按已使用的页数确保页池容量（帧初一次性增长、永久复用；不足时创建新页）。
        let page_count = page as usize + 1;
        self.draw_page.ensure_instance_pages(&self.device, page_count);

        // **每页整页只写入一次**（offset 0）：按页切分 buf_instances 写入对应页缓冲。
        // 页与页相互独立，规避 `Queue::write_buffer` 在 submit 前全部执行
        // 导致「同一缓冲最后一次写入覆盖先前批次」的问题（此前曾使所有精灵
        // 位置全部变成与最后一屏 UI 相同的 NDC 位置）。页池缓冲跨帧复用。
        {
            let mut page_idx = 0usize;
            let mut start = 0usize;
            while start < self.buf_instances.len() {
                let end = (start + MAX_INSTANCES_PER_DRAW).min(self.buf_instances.len());
                self.draw_page.update_instances_page(
                    &self.queue,
                    page_idx,
                    &self.buf_instances[start..end],
                );
                page_idx += 1;
                start = end;
            }
        }

        // ── Mesh：按排序顺序将顶点/索引组装后整批写入 DrawPage 动态缓冲 ──
        if !self.buf_mesh_cmds.is_empty() {
            // 1) 统计总量 → 一次性扩容（不足 2× 增长；u16 索引上限）。
            //
            // ⚠️ 必须用 `assert`（而非 `debug_assert`）：
            //    release 构建下如果总顶点 > u16::MAX，`as u16` 截断会导致
            //    顶点引用错误（乱画），因此运行时显式失败更安全。
            let total_verts: usize = self.buf_mesh_cmds.iter().map(|(v, _)| v.end - v.start).sum();
            let total_tris: usize = self.buf_mesh_cmds.iter().map(|(_, t)| t.end - t.start).sum();
            assert!(
                total_verts <= MAX_MESH_VERTS,
                "mesh vertices exceed u16 index limit: {total_verts} > {MAX_MESH_VERTS}"
            );
            // 索引缓冲预留 1 个 TriIndicies（6 字节）余量，容纳对齐补 0。
            self.draw_page
                .ensure_mesh_capacity(&self.device, total_verts, total_tris + 1);

            // 2) 按排序顺序组装整批顶点 + 重定位索引（复用 buf_all_*），
            //    同时把 `buf_ops` 中的 MeshPlaceholder **按序**替换为实际 DrawOp::Mesh。
            let mut v_cursor: usize = 0;
            let mut i_cursor: usize = 0;
            let mut op_plh = self.buf_ops.iter_mut().filter_map(|op| match op {
                DrawOp::MeshPlaceholder => Some(op),
                _ => None,
            });

            for (vert, tri_index) in &self.buf_mesh_cmds {
                let vcount = vert.end - vert.start;
                let icount = tri_index.end - tri_index.start;

                // 顶点：直接追加（顺序 = 排序顺序；空 mesh 无顶点可拷）。
                if vcount != 0 {
                    self.buf_all_verts.extend_from_slice(&self.mesh_storage.vertices[vert.clone()]);
                }
                // 索引：旧全局索引 − 记录起点 + 当前顶点游标 → 重定位后追加。
                if vcount != 0 && icount != 0 {
                    let reloc_base = (v_cursor as i64) - (vert.start as i64);
                    for t in &self.mesh_storage.tri_indices[tri_index.clone()] {
                        self.buf_all_tris.push(TriIndicies(
                            Index((t.0 .0 as i64 + reloc_base) as u16),
                            Index((t.1 .0 as i64 + reloc_base) as u16),
                            Index((t.2 .0 as i64 + reloc_base) as u16),
                        ));
                    }
                }

                let item = crate::draw_page::MeshDrawItem {
                    first_vertex: v_cursor as u32,
                    vertex_count: vcount as u16,
                    tri_index_start: i_cursor as u32,
                    tri_index_count: icount as u16,
                };
                // 替换本 mesh 对应的占位符（占位符数量 == buf_mesh_cmds 数量，顺序一致）。
                if let Some(op) = op_plh.next() {
                    *op = DrawOp::Mesh { item };
                }

                v_cursor += vcount;
                i_cursor += icount;
            }

            // 3) 整批写入（offset 0；size 对齐 4）。
            if !self.buf_all_verts.is_empty() {
                self.queue.write_buffer(
                    &self.draw_page.mesh_vb,
                    0,
                    bytemuck::cast_slice(&self.buf_all_verts),
                );
            }
            if !self.buf_all_tris.is_empty() {
                let bytes = bytemuck::cast_slice(&self.buf_all_tris); // 6*N 字节
                let padded_len = (bytes.len() + 3) & !3; // 垫整到 4 的倍数
                self.buf_padded.clear();
                self.buf_padded.extend_from_slice(bytes);
                self.buf_padded.resize(padded_len, 0u8);
                self.queue.write_buffer(&self.draw_page.mesh_ib, 0, &self.buf_padded);
            }
        }
    }

    /// 录制全部绘制命令到 `pass`（供 `render` 与 `flush` 共用；从常驻池读取）。
    ///
    /// 严格按 `buf_ops`（由 `prepare()` 按 (layer, states) 排序生成）逐个 op 绘制：
    /// Sprite 批次与 Mesh **交错**、连续相同纹理跨 layer 合批，
    /// 保证跨类别（Sprite ↔ Mesh）与跨层级的最终绘制顺序正确。
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.buf_ops.is_empty() {
            return;
        }

        // 实例缓冲**页池**：prepare() 已把实例按 MAX_INSTANCES_PER_DRAW 分页，
        // 每个 DrawOp::Sprite 记录 页号 + 页内范围。**每页只写入一次**（offset 0），
        // 绘制时绑定该页缓冲 → 页与页之间数据独立，规避 `Queue::write_buffer`
        // 在 submit 前全部执行导致「最后一次写入覆盖先前批次」的图形错误
        //（此前曾出现：所有精灵位置全变成与最后一屏 UI 相同的 NDC 位置）。

        // 连续相邻的 DrawOp::Mesh（中间无 Sprite 打断）合并为**一次** draw_indexed：
        // prepare() 已把 mesh 顶点/索引按排序顺序**连续**写入 mesh_vb/mesh_ib，
        // 且它们共用同一 mesh_pipeline + 白色纹理 —— 因此即使 layer 不同（如示例中
        // 同为 96 的三角形与四边形）也完全满足合批条件（数据连续 + 管线/绑定组相同）。
        let mut i = 0usize;
        while i < self.buf_ops.len() {
            match &self.buf_ops[i] {
                DrawOp::Sprite { page, tex_uid, range } => {
                    let count = range.end - range.start;
                    if count != 0 {
                        pass.set_pipeline(&self.sprite_pipeline);
                        pass.set_bind_group(0, &self.draw_page.vp_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.draw_page.quad_vb.slice(..));
                        // 绑定本批次所在实例页；实例范围 = 页内相对偏移
                        // （prepare() 已把每页实例整页写入对应页缓冲，offset 0）。
                        pass.set_vertex_buffer(
                            1,
                            self.draw_page
                                .instance_page_buffer(*page as usize)
                                .slice(..),
                        );
                        pass.set_index_buffer(
                            self.draw_page.quad_ib.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );

                        let bg = match tex_uid {
                            // 白色默认纹理在独立字段，不在纹理池中。
                            Some(uid) if *uid == self.white_texture.uid => &self.white_texture.bind_group,
                            Some(uid) => {
                                let tex = self
                                    .textures
                                    .iter()
                                    .find(|t| t.uid == *uid)
                                    .expect("texture uid not found in pool");
                                &tex.bind_group
                            }
                            None => &self.white_texture.bind_group,
                        };
                        pass.set_bind_group(1, bg, &[]);
                        pass.draw_indexed(
                            0..QUAD_TRI_INDICIES.len() as u32,
                            0,
                            range.clone(),
                        );
                    }
                    i += 1;
                }
                DrawOp::Mesh { .. } => {
                    // 向后收集**连续**的所有 Mesh（空 mesh 计入段内但不产生几何，
                    // 不打断合批连续性；遇到 Sprite/结尾则结束本段）。
                    let mut seg_start: Option<u32> = None;
                    let mut seg_count: u32 = 0;
                    while i < self.buf_ops.len() {
                        match &self.buf_ops[i] {
                            DrawOp::Mesh { item } => {
                                // ⚠️ mesh_ib 存的是 TriIndicies（每个含 3 个 u16 索引），
                                //    draw_indexed 的 Range 单位是「索引」= 三角形数 × 3。
                                if item.tri_index_count != 0 {
                                    if seg_start.is_none() {
                                        seg_start = Some(item.tri_index_start * 3);
                                    }
                                    seg_count += item.tri_index_count as u32 * 3;
                                }
                                i += 1;
                            }
                            _ => break,
                        }
                    }
                    if let Some(start) = seg_start {
                        pass.set_pipeline(&self.mesh_pipeline);
                        pass.set_bind_group(0, &self.draw_page.vp_bind_group, &[]);
                        pass.set_bind_group(1, &self.white_texture.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.draw_page.mesh_vb.slice(..));
                        pass.set_index_buffer(
                            self.draw_page.mesh_ib.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(
                            start..start + seg_count,
                            0,
                            0..1,
                        );
                    }
                }
                DrawOp::MeshPlaceholder => {
                    // prepare() 应已将所有占位符替换为 DrawOp::Mesh；残留说明内部逻辑 bug。
                    debug_assert!(false, "unreplaced MeshPlaceholder in buf_ops");
                    i += 1;
                }
            }
        }
    }

    /// 确保深度/模板纹理存在且尺寸匹配（否则重建）。
    fn ensure_depth(&mut self, width: u32, height: u32) {
        let inited = self
            .depth_view
            .as_ref()
            .is_some_and(|_| self.depth_size == (width.max(1), height.max(1)));
        if inited {
            return;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render2d depth-stencil"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // 旧纹理随 field 替换被释放。
        self.depth_view = Some(view);
        self.depth_size = (width.max(1), height.max(1));
    }

    /// 访问 device（保留给高级 API）。
    #[allow(unused)]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// 访问 queue（保留给高级 API）。
    #[allow(unused)]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}