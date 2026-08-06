//! 纹理包装：纹理 + 视图 + 全局唯一 id 用于合批排序。
//! 同时提供全局、线程安全的 `TextureRegistry`（基于 `TypedRegistry`）。
//!
//! **与采样器解耦**：`TextureWrapped` 不再持有 sampler / bind group。
//! 采样器由 `rjw_2d_render::rstates::RStates`（bits 8..24 位域）驱动，
//! bind group 由渲染器按 `(tex_uid, samp_key)` 缓存创建。

use std::sync::{Arc, LazyLock};

use crate::registry::{HasUid, TypedRegistry};

static NEXT_TEXTURE_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 纹理包装：纹理 + 视图 + 全局唯一 id。
///
/// `texture`/`view` 为渲染与 atlas 写子区域预留。
#[allow(dead_code)]
pub struct TextureWrapped {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    /// 全局唯一 id，用于合批排序。
    pub uid: u64,
}

impl HasUid for TextureWrapped {
    fn uid(&self) -> u64 {
        self.uid
    }
}

pub type ArcTextureWrapped = Arc<TextureWrapped>;

impl TextureWrapped {
    /// 从 RGBA8 字节数据创建纹理（宽高 1 像素 = 4 字节）。
    ///
    /// 不创建采样器 / bind group（与采样器解耦）；`view()` 用于取样。
    pub fn from_rgba8(
        device: &wgpu::Device, queue: &wgpu::Queue,
        label: &str, data: &[u8], width: u32, height: u32,
    ) -> Self {
        debug_assert_eq!(data.len() as u32, width * height * 4, "RGBA8 data length mismatch");

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            texture, view, width, height,
            uid: NEXT_TEXTURE_UID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    pub fn view(&self) -> &wgpu::TextureView { &self.view }

    /// 暴露底层 `Texture`（供 atlas write_subregion 等操作）。
    pub fn raw_texture(&self) -> &wgpu::Texture { &self.texture }
}

// ─── 全局纹理注册表 ──────────────────────────────────────────

/// 线程安全纹理注册表：按 uid / name 查找 `ArcTextureWrapped`。
///
/// 泛型 `TypedRegistry` 的别名；提供 `register`/`register_named`/`get`/`get_ref`/
/// `remove`/`remove_name_mapping`/`rename`/`contains_*` 等完整能力。
pub type TextureRegistry = TypedRegistry<TextureWrapped>;

/// 全局纹理注册表单例。
pub static TEXTURES: LazyLock<TextureRegistry> = LazyLock::new(TextureRegistry::default);