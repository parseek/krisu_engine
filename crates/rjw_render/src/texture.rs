//! 纹理包装：纹理 + 采样器 + 绑定组，并带全局唯一 id 用于合批排序。
//!
//! 从 `rjw_2d_render` 移入（texture 为通用 GPU 资源，不依赖 2D 批处理逻辑）。

use std::sync::Arc;

static NEXT_TEXTURE_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 纹理包装：纹理 + 采样器 + 绑定组，并带全局唯一 id 用于合批排序
///
/// `texture`/`view`/`sampler` 为后续版本预留（例如纹理重建/删除时直接访问）。
#[allow(dead_code)]
pub struct TextureWrapped {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    /// group(1) 的绑定组（纹理 + 采样器）；由 2D 渲染器在绘制时绑定。
    pub bind_group: wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
    /// 全局唯一 id，用于合批排序。
    pub uid: u64,
}

pub type ArcTextureWrapped = Arc<TextureWrapped>;

impl TextureWrapped {
    /// 从 RGBA8 字节数据创建纹理（宽高 1 像素 = 4 字节）。
    ///
    /// `tex_layout`: group(1) 绑定组布局（binding 0 = 纹理采样，binding 1 = 过滤器采样器）。
    ///
    /// # Panics
    ///
    /// 当 `data.len() != width * height * 4` 时 panic（用 usize 运算避免
    /// u32 乘法在超大尺寸下溢出回绕导致长度校验失效）。
    pub fn from_rgba8(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tex_layout: &wgpu::BindGroupLayout,
        label: &str,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Self {
        debug_assert_eq!(data.len() as u32, width * height * 4, "RGBA8 data length mismatch");

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(&format!("{label} sampler")),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("{label} bind group")),
            layout: tex_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        Self {
            texture,
            view,
            sampler,
            bind_group,
            width,
            height,
            uid: NEXT_TEXTURE_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// 访问纹理视图（供自定义绑定组 / 采样器组合使用）。
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}