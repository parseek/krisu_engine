//! GPU 能力对象 [`Gpu`]：把 `device` + `queue` + 纹理 bind group layout **合成一个参数**。
//!
//! 动机（`docs/API_DESIGN.md` R5 / 简并表）：此前每个子系统的构造器都要调用方手动穿线
//! `device` / `queue` / `texture_layout` 三件套，例如
//! `Text::new(r2d.device(), r2d.queue(), r2d.texture_layout())`。现在统一收 `&Gpu`。
//!
//! `Gpu` 还是唯一的**资源工厂**：纹理、静态网格的创建与注册都从这里走，调用方不再碰
//! 全局注册表或 wgpu 类型（需要时经 [`Gpu::device`] / [`Gpu::queue`] / [`Gpu::texture_layout`]
//! 逃生口）。

use std::sync::Arc;

use crate::mesh::{MeshData, MeshId, MeshSpec};
use crate::texture::{ArcTextureWrapped, TextureWrapped, TEXTURES};
use crate::MESHES;

/// RGBA8 像素数据 + 尺寸。
///
/// 存在的意义：让「创建纹理」这类入口保持 ≤2 参（`Gpu::texture(label, px)`），
/// 而不是 `(device, queue, label, data, w, h)` 式的 6 参长表。
#[derive(Debug, Clone, Copy)]
pub struct Rgba8<'a> {
    /// RGBA8 字节（长度必须等于 `size.0 * size.1 * 4`）。
    pub data: &'a [u8],
    /// `(width, height)`（像素）。
    pub size: (u32, u32),
}

impl<'a> Rgba8<'a> {
    /// 构造（`data` 长度与 `size` 不匹配时在 `Gpu::texture` 里 debug 断言）。
    #[inline]
    pub fn new(data: &'a [u8], size: (u32, u32)) -> Self {
        Self { data, size }
    }

    #[inline]
    pub fn width(&self) -> u32 {
        self.size.0
    }

    #[inline]
    pub fn height(&self) -> u32 {
        self.size.1
    }
}

/// GPU 能力对象：设备 + 队列 + 纹理 bind group layout + 资源工厂。
///
/// 由 [`crate::RenderContext`] 创建并持有（`RenderContext::gpu()`）。
pub struct Gpu {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    texture_layout: wgpu::BindGroupLayout,
    vp_layout: wgpu::BindGroupLayout,
}

impl Gpu {
    /// 由设备 / 队列构造，并建立引擎规范的纹理 / VP bind group layout。
    ///
    /// `pub(crate)`：正常路径由 [`crate::RenderContext`] 创建，不暴露给用户（R5）。
    pub(crate) fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        let texture_layout = device.create_bind_group_layout(&TEXTURE_BIND_GROUP_LAYOUT);
        let vp_layout = device.create_bind_group_layout(&vp_bind_group_layout_desc());
        Self { device, queue, texture_layout, vp_layout }
    }

    /// 底层设备（逃生口：自定义管线 / 后处理）。
    #[inline]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// 底层队列（逃生口）。
    #[inline]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// 共享设备句柄（帧 / 内部使用）。
    #[inline]
    pub(crate) fn device_arc(&self) -> Arc<wgpu::Device> {
        self.device.clone()
    }

    /// 共享队列句柄（帧 / 内部使用）。
    #[inline]
    pub(crate) fn queue_arc(&self) -> Arc<wgpu::Queue> {
        self.queue.clone()
    }

    /// 设备身份：深度附件池 key 的一部分（多设备不得共享附件）。
    #[inline]
    pub(crate) fn device_key(&self) -> usize {
        Arc::as_ptr(&self.device) as usize
    }

    /// 引擎规范的纹理 bind group layout（`texture2d` + `sampler`，片元可见）。
    ///
    /// `Render2D` / `rjw_text` / `rjw_atlas` / `rjw_ui` 的全部纹理采样管线共用它，
    /// 因此子系统之间可以互相复用 bind group。
    #[inline]
    pub fn texture_layout(&self) -> &wgpu::BindGroupLayout {
        &self.texture_layout
    }

    /// 引擎规范的 **VP bind group layout**（动态偏移 uniform，绑定 0）。
    ///
    /// 渲染器用它建管线；每个 pass 的 VP 数据由**帧级 VP 槽环**
    /// （`RenderFrame` 内部）写入并通过动态偏移绑定——因此一帧内的多个画面
    /// （多个相机）互不干扰。
    #[inline]
    pub fn vp_layout(&self) -> &wgpu::BindGroupLayout {
        &self.vp_layout
    }

    /// 创建 RGBA8 纹理并注册进全局 [`crate::TEXTURES`]。
    ///
    /// 这是**唯一**的纹理创建入口（happy path）。
    pub fn texture(&self, label: &str, px: Rgba8<'_>) -> ArcTextureWrapped {
        let (w, h) = px.size;
        assert_eq!(
            px.data.len() as u32,
            w * h * 4,
            "Gpu::texture: RGBA8 数据长度 {} 与 {}x{} 不匹配",
            px.data.len(),
            w,
            h
        );
        let tex = Arc::new(TextureWrapped::from_rgba8(&self.device, &self.queue, label, px.data, w, h));
        TEXTURES.register(tex.clone());
        tex
    }

    /// 创建静态网格（POD 顶点 + u16 索引）并注册进全局 [`crate::MESHES`]，返回可复用句柄。
    pub fn mesh<T: bytemuck::Pod>(&self, spec: MeshSpec<'_, T>) -> MeshId {
        let mesh = MeshData::from_pod(&self.device, spec.vertices, spec.indices, spec.label);
        MeshId::new(MESHES.register(Arc::new(mesh)))
    }

    /// 全局纹理注册表（按 uid / name 查找）。
    #[inline]
    pub fn textures(&self) -> &'static crate::TextureRegistry {
        &TEXTURES
    }

    /// 全局静态网格注册表。
    #[inline]
    pub fn meshes(&self) -> &'static crate::MeshRegistry {
        &MESHES
    }
}

/// 引擎规范的纹理 bind group layout 描述。
const TEXTURE_BIND_GROUP_LAYOUT: wgpu::BindGroupLayoutDescriptor<'static> = wgpu::BindGroupLayoutDescriptor {
    label: Some("krusie: texture bind group layout"),
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
};

/// VP uniform 的字节数（列主序 4×4 f32 = 64 B）。
const VP_UNIFORM_SIZE: u64 = 64;
/// 编译期保证它与 `Matrix4x4` 一致。
const _: () = assert!(std::mem::size_of::<crate::frame::Matrix4x4>() as u64 == VP_UNIFORM_SIZE);

/// VP bind group layout 的条目（静态，供 `BindGroupLayoutDescriptor` 借用）。
static VP_ENTRIES: [wgpu::BindGroupLayoutEntry; 1] = [wgpu::BindGroupLayoutEntry {
    binding: 0,
    visibility: wgpu::ShaderStages::VERTEX,
    ty: wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: true,
        // `Option::unwrap` 在 const 上下文可用（Rust ≥ 1.83）⇒ 不需要 unsafe。
        min_binding_size: std::num::NonZeroU64::new(VP_UNIFORM_SIZE),
    },
    count: None,
}];

/// VP bind group layout 描述（动态偏移的 uniform）。
fn vp_bind_group_layout_desc() -> wgpu::BindGroupLayoutDescriptor<'static> {
    wgpu::BindGroupLayoutDescriptor {
        label: Some("krusie: VP bind group layout"),
        entries: &VP_ENTRIES,
    }
}
