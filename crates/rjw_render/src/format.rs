//! wgpu 纹理格式切面查询（低层工具）。
//!
//! 放在独立模块而不是 `pub(crate)`：`rjw_2d_render::rstates` 需要用它们把
//! `RStates` 的深度 / 模板位域翻译成 `wgpu` 附件操作（跨 crate 无法 `pub(crate)`）。
//! 属于 L3 低层工具，不进 prelude。

/// 该格式是否含**深度**切面（`depth_ops` 可用）。
pub const fn has_depth_aspect(f: wgpu::TextureFormat) -> bool {
    use wgpu::TextureFormat as F;
    matches!(
        f,
        F::Depth16Unorm
            | F::Depth24Plus
            | F::Depth24PlusStencil8
            | F::Depth32Float
            | F::Depth32FloatStencil8
    )
}

/// 该格式是否含**模板**切面（`stencil_ops` 可用）。
pub const fn has_stencil_aspect(f: wgpu::TextureFormat) -> bool {
    use wgpu::TextureFormat as F;
    matches!(f, F::Stencil8 | F::Depth24PlusStencil8 | F::Depth32FloatStencil8)
}
