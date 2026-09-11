//! `Gfx`：长期 GPU 能力对象（`App::init` 拿到它建资源）。
//!
//! 把 `device` + `queue` + 纹理 bind group layout 收成**一个参数**（`docs/API_DESIGN.md` R5），
//! 并提供资源工厂。与 [`crate::runtime::Ctx`]（每帧）分工：`Gfx` 只在初始化时出现，
//! 无帧概念；`Ctx` 每帧都有。

use rjw_render::{ArcTextureWrapped, Gpu, MeshId, MeshSpec, Rgba8};

/// 长期 GPU 能力对象。
///
/// 经 [`Deref`] 直接暴露底层 [`Gpu`]（`Gfx` → `&Gpu` 的 deref coercion 让
/// 「收 `&Gpu`」的构造器可以直接传 `gfx`，例如 `DynamicAtlas::new(gfx, cfg)`）。
pub struct Gfx<'a> {
    gpu: &'a Gpu,
    format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    size: (u32, u32),
}

impl std::ops::Deref for Gfx<'_> {
    type Target = Gpu;
    #[inline]
    fn deref(&self) -> &Gpu {
        self.gpu
    }
}

impl<'a> Gfx<'a> {
    pub(crate) fn new(
        gpu: &'a Gpu,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> Self {
        Self { gpu, format, depth_format, size }
    }

    /// 底层设备（逃生口：自定义管线 / 后处理）。
    #[inline]
    pub fn device(&self) -> &wgpu::Device {
        self.gpu.device()
    }

    /// 底层队列（逃生口）。
    #[inline]
    pub fn queue(&self) -> &wgpu::Queue {
        self.gpu.queue()
    }

    /// 引擎规范的纹理 bind group layout（自建采样管线用）。
    #[inline]
    pub fn texture_layout(&self) -> &wgpu::BindGroupLayout {
        self.gpu.texture_layout()
    }

    /// 颜色附件格式（= surface 格式）：自建管线 / 自定义绘制需要它，且只在 `init` 期可知。
    #[inline]
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// 深度 / 模板附件格式（帧级常量）。
    #[inline]
    pub fn depth_format(&self) -> wgpu::TextureFormat {
        self.depth_format
    }

    /// 初始表面尺寸（物理像素）——`init` 期需要它来初始化相机 / 依赖尺寸的资源。
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// 创建 RGBA8 纹理（自动注册进全局纹理表，供渲染器按 uid 取样）。
    #[inline]
    pub fn texture(&self, label: &str, px: Rgba8<'_>) -> ArcTextureWrapped {
        self.gpu.texture(label, px)
    }

    /// 创建静态网格（POD 顶点 + u16 索引），返回可复用句柄。
    #[inline]
    pub fn mesh<T: bytemuck::Pod>(&self, spec: MeshSpec<'_, T>) -> MeshId {
        self.gpu.mesh(spec)
    }

    /// 新建文本子系统（feature = `text`）。
    #[cfg(feature = "text")]
    #[inline]
    pub fn text(&self) -> rjw_text::Text {
        rjw_text::Text::new(self.gpu)
    }

    /// 底层 `Gpu`（低层逃生口）。
    #[inline]
    pub fn gpu(&self) -> &Gpu {
        self.gpu
    }
}

// `wgpu` 需要在签名里出现（device/queue），从 rjw_render 重导出的是同一个版本。
pub use rjw_render::wgpu;
