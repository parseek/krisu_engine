//! RStates：2D 渲染状态 bitfield（u64 压缩），独立于纹理。
//!
//! 控制域：
//! - Blend (8 bits)
//! - Sampler (16 bits)
//! - Cull + Raster (8 bits)
//! - Depth (8 bits)
//! - Stencil (8 bits)
//! - Reserved (16 bits)
//!
//! `RStates(0)` = 全默认：alpha blend + linear filter + clamp + no cull + fill + ccw + no depth/stencil。

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RStates(u64);

impl Default for RStates {
    #[inline]
    fn default() -> Self {
        Self(0)
    }
}

impl RStates {
    #[inline]
    pub const fn new() -> Self {
        Self(0)
    }

    /// 返回原始 u64（供排序/比较等内部使用）。
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// 从 u64 重建。
    #[inline]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    // ─── Blend 域 (bits 0..8) ───

    #[inline]
    pub fn blend(mut self, mode: BlendMode) -> Self {
        self.0 = (self.0 & !0xFF) | ((mode.to_u32() as u64) & 0xFF);
        self
    }

    #[inline]
    pub fn blend_state(self, d: BlendDesc) -> Self {
        self.blend(d.blend_mode)
    }

    #[inline]
    pub fn to_blend(self) -> Option<wgpu::BlendState> {
        let mode = BlendMode::from_u32((self.0 & 0xFF) as u32);
        match mode {
            BlendMode::Alpha => Some(wgpu::BlendState::ALPHA_BLENDING),
            BlendMode::Additive => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            BlendMode::Multiply => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Dst,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Dst,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            BlendMode::Premultiplied => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            BlendMode::Inverse => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::OneMinusDst,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Zero,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
            BlendMode::Subtract => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::ReverseSubtract,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::ReverseSubtract,
                },
            }),
            BlendMode::Min => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Min,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Min,
                },
            }),
            BlendMode::Max => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Max,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Max,
                },
            }),
            BlendMode::Disabled => None,
        }
    }

    // ─── Sampler 域 (bits 8..24) ───

    fn samp_field(self, shift: u32, mask: u64) -> u32 {
        ((self.0 >> shift) & mask) as u32
    }

    fn set_samp_field(mut self, shift: u32, mask: u64, val: u32) -> Self {
        self.0 = (self.0 & !(mask << shift)) | (((val as u64) & mask) << shift);
        self
    }

    #[inline]
    pub fn samp_mag(self, f: FilterMode) -> Self {
        self.set_samp_field(8, 0x3, f.to_u32())
    }
    #[inline]
    pub fn samp_min(self, f: FilterMode) -> Self {
        self.set_samp_field(10, 0x3, f.to_u32())
    }
    #[inline]
    pub fn samp_mip(self, f: FilterMode) -> Self {
        self.set_samp_field(12, 0x3, f.to_u32())
    }
    #[inline]
    pub fn samp_addr_u(self, a: AddressMode) -> Self {
        self.set_samp_field(14, 0x3, a.to_u32())
    }
    #[inline]
    pub fn samp_addr_v(self, a: AddressMode) -> Self {
        self.set_samp_field(16, 0x3, a.to_u32())
    }
    #[inline]
    pub fn samp_addr_w(self, a: AddressMode) -> Self {
        self.set_samp_field(18, 0x3, a.to_u32())
    }

    pub fn samp_state(mut self, d: SamplerDesc) -> Self {
        self = self.samp_mag(d.mag).samp_min(d.min).samp_mip(d.mip);
        self = self.samp_addr_u(d.addr_u).samp_addr_v(d.addr_v).samp_addr_w(d.addr_w);
        self
    }

    pub fn to_sampler_desc(self) -> wgpu::SamplerDescriptor<'static> {
        let mag = FilterMode::from_u32(self.samp_field(8, 0x3));
        let min = FilterMode::from_u32(self.samp_field(10, 0x3));
        let mip = FilterMode::from_u32(self.samp_field(12, 0x3));
        let au = AddressMode::from_u32(self.samp_field(14, 0x3));
        let av = AddressMode::from_u32(self.samp_field(16, 0x3));
        let aw = AddressMode::from_u32(self.samp_field(18, 0x3));
        wgpu::SamplerDescriptor {
            label: None,
            address_mode_u: au.to_wgpu(),
            address_mode_v: av.to_wgpu(),
            address_mode_w: aw.to_wgpu(),
            mag_filter: mag.to_wgpu(),
            min_filter: min.to_wgpu(),
            mipmap_filter: mip.to_wgpu_mip(),
            ..Default::default()
        }
    }

    // ─── Cull + Raster 域 (bits 24..32) ───

    #[inline]
    pub fn cull(mut self, c: CullMode) -> Self {
        self.0 = (self.0 & !(0x3 << 24)) | (((c.to_u32() as u64) & 0x3) << 24);
        self
    }

    #[inline]
    pub fn polygon(mut self, p: PolygonMode) -> Self {
        self.0 = (self.0 & !(0x3 << 26)) | (((p.to_u32() as u64) & 0x3) << 26);
        self
    }

    #[inline]
    pub fn front_face(mut self, f: FrontFaceWinding) -> Self {
        self.0 = (self.0 & !(1 << 28)) | (((f.to_u32() as u64) & 1) << 28);
        self
    }

    #[inline]
    pub fn conservative_raster(mut self, b: bool) -> Self {
        if b {
            self.0 |= 1 << 29;
        } else {
            self.0 &= !(1 << 29);
        }
        self
    }

    pub fn raster_state(mut self, s: RasterState) -> Self {
        self = self
            .cull(s.cull)
            .polygon(s.polygon)
            .front_face(s.front_face)
            .conservative_raster(s.conservative);
        self
    }

    pub fn to_cull(self) -> Option<wgpu::Face> {
        if self.0 & 2 != 0 {
            // Back
            Some(wgpu::Face::Back)
        } else if self.0 & 1 != 0 {
            // Front
            Some(wgpu::Face::Front)
        } else {
            None
        }
    }

    pub fn to_polygon(self) -> wgpu::PolygonMode {
        PolygonMode::from_u32(((self.0 >> 26) & 0x3) as u32).to_wgpu()
    }

    pub fn to_front_face(self) -> wgpu::FrontFace {
        if (self.0 >> 28) & 1 != 0 {
            wgpu::FrontFace::Cw
        } else {
            wgpu::FrontFace::Ccw
        }
    }

    pub fn to_conservative(self) -> bool {
        (self.0 >> 29) & 1 != 0
    }

    // ─── Depth 域 (bits 32..40) ───

    #[inline]
    pub fn depth_test(mut self, b: bool) -> Self {
        if b {
            self.0 |= 1 << 32;
        } else {
            self.0 &= !(1 << 32);
        }
        self
    }

    #[inline]
    pub fn depth_write(mut self, b: bool) -> Self {
        if b {
            self.0 |= 1 << 33;
        } else {
            self.0 &= !(1 << 33);
        }
        self
    }

    #[inline]
    pub fn depth_compare(mut self, f: CompareFunc) -> Self {
        self.0 = (self.0 & !(0x7 << 34)) | (((f.to_u32() as u64) & 0x7) << 34);
        self
    }

    pub fn depth_state(mut self, s: DepthState) -> Self {
        self = self
            .depth_test(s.test)
            .depth_write(s.write)
            .depth_compare(s.compare);
        self
    }

    pub fn to_depth_stencil(self) -> Option<wgpu::DepthStencilState> {
        let depth_test = (self.0 >> 32) & 1 != 0;
        let depth_write = (self.0 >> 33) & 1 != 0;
        let depth_compare = ((self.0 >> 34) & 0x7) as u32;

        let stencil_test = (self.0 >> 40) & 1 != 0;
        let stencil_compare = ((self.0 >> 42) & 0x7) as u32;

        if !depth_test && !stencil_test {
            return None;
        }
        Some(wgpu::DepthStencilState {
            format: crate::draw_page::DEPTH_FORMAT,
            depth_write_enabled: Some(depth_test && depth_write),
            depth_compare: if depth_test {
                Some(CompareFunc::from_u32(depth_compare).to_wgpu())
            } else {
                Some(wgpu::CompareFunction::Always)
            },
            stencil: if stencil_test {
                wgpu::StencilState {
                    front: wgpu::StencilFaceState {
                        compare: CompareFunc::from_u32(stencil_compare).to_wgpu(),
                        fail_op: wgpu::StencilOperation::Keep,
                        depth_fail_op: wgpu::StencilOperation::Keep,
                        pass_op: wgpu::StencilOperation::Keep,
                    },
                    back: wgpu::StencilFaceState {
                        compare: CompareFunc::from_u32(stencil_compare).to_wgpu(),
                        fail_op: wgpu::StencilOperation::Keep,
                        depth_fail_op: wgpu::StencilOperation::Keep,
                        pass_op: wgpu::StencilOperation::Keep,
                    },
                    read_mask: 0xFF,
                    write_mask: 0xFF,
                }
            } else {
                wgpu::StencilState::default()
            },
            bias: wgpu::DepthBiasState::default(),
        })
    }

    // ─── Stencil 域 (bits 40..48) ───

    #[inline]
    pub fn stencil_test(mut self, b: bool) -> Self {
        if b {
            self.0 |= 1 << 40;
        } else {
            self.0 &= !(1 << 40);
        }
        self
    }

    #[inline]
    pub fn stencil_write(mut self, b: bool) -> Self {
        if b {
            self.0 |= 1 << 41;
        } else {
            self.0 &= !(1 << 41);
        }
        self
    }

    #[inline]
    pub fn stencil_compare(mut self, f: CompareFunc) -> Self {
        self.0 = (self.0 & !(0x7 << 42)) | (((f.to_u32() as u64) & 0x7) << 42);
        self
    }

    pub fn stencil_state(mut self, s: StencilState) -> Self {
        self = self
            .stencil_test(s.test)
            .stencil_write(s.write)
            .stencil_compare(s.compare);
        self
    }
}

// ─── 子类型 / 描述符（Copy，零开销抽象） ───

/// 混合模式预设。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlendMode {
    Alpha,
    Additive,
    Multiply,
    Premultiplied,
    Inverse,
    Subtract,
    Min,
    Max,
    Disabled,
}

impl BlendMode {
    #[inline]
    pub fn to_u32(self) -> u32 {
        match self {
            Self::Alpha => 0,
            Self::Additive => 1,
            Self::Multiply => 2,
            Self::Premultiplied => 3,
            Self::Inverse => 4,
            Self::Subtract => 5,
            Self::Min => 6,
            Self::Max => 7,
            Self::Disabled => 8,
        }
    }

    #[inline]
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Additive,
            2 => Self::Multiply,
            3 => Self::Premultiplied,
            4 => Self::Inverse,
            5 => Self::Subtract,
            6 => Self::Min,
            7 => Self::Max,
            8 => Self::Disabled,
            _ => Self::Alpha,
        }
    }
}

/// 混合描述符（合并为 blend 域）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlendDesc {
    pub blend_mode: BlendMode,
}

impl Default for BlendDesc {
    fn default() -> Self {
        Self {
            blend_mode: BlendMode::Alpha,
        }
    }
}

/// 纹理过滤模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FilterMode {
    Linear,
    Nearest,
}

impl FilterMode {
    #[inline]
    pub fn to_u32(self) -> u32 {
        match self {
            Self::Linear => 0,
            Self::Nearest => 1,
        }
    }
    #[inline]
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Nearest,
            _ => Self::Linear,
        }
    }
    pub fn to_wgpu(self) -> wgpu::FilterMode {
        match self {
            Self::Linear => wgpu::FilterMode::Linear,
            Self::Nearest => wgpu::FilterMode::Nearest,
        }
    }
    pub fn to_wgpu_mip(self) -> wgpu::MipmapFilterMode {
        match self {
            Self::Linear => wgpu::MipmapFilterMode::Linear,
            Self::Nearest => wgpu::MipmapFilterMode::Nearest,
        }
    }
}

/// 寻址模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AddressMode {
    ClampToEdge,
    Repeat,
    MirrorRepeat,
}

impl AddressMode {
    #[inline]
    pub fn to_u32(self) -> u32 {
        match self {
            Self::ClampToEdge => 0,
            Self::Repeat => 1,
            Self::MirrorRepeat => 2,
        }
    }
    #[inline]
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Repeat,
            2 => Self::MirrorRepeat,
            _ => Self::ClampToEdge,
        }
    }
    pub fn to_wgpu(self) -> wgpu::AddressMode {
        match self {
            Self::ClampToEdge => wgpu::AddressMode::ClampToEdge,
            Self::Repeat => wgpu::AddressMode::Repeat,
            Self::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
        }
    }
}

/// 采样器描述符（合并为 sampler 域）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerDesc {
    pub mag: FilterMode,
    pub min: FilterMode,
    pub mip: FilterMode,
    pub addr_u: AddressMode,
    pub addr_v: AddressMode,
    pub addr_w: AddressMode,
}

impl Default for SamplerDesc {
    fn default() -> Self {
        Self {
            mag: FilterMode::Linear,
            min: FilterMode::Linear,
            mip: FilterMode::Linear,
            addr_u: AddressMode::ClampToEdge,
            addr_v: AddressMode::ClampToEdge,
            addr_w: AddressMode::ClampToEdge,
        }
    }
}

/// 剔除模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CullMode {
    None,
    Front,
    Back,
}

impl CullMode {
    #[inline]
    pub fn to_u32(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Front => 1,
            Self::Back => 2,
        }
    }
    #[inline]
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Front,
            2 => Self::Back,
            _ => Self::None,
        }
    }
}

/// 多边形绘制模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PolygonMode {
    Fill,
    Line,
    Point,
}

impl PolygonMode {
    #[inline]
    pub fn to_u32(self) -> u32 {
        match self {
            Self::Fill => 0,
            Self::Line => 1,
            Self::Point => 2,
        }
    }
    #[inline]
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Line,
            2 => Self::Point,
            _ => Self::Fill,
        }
    }
    pub fn to_wgpu(self) -> wgpu::PolygonMode {
        match self {
            Self::Fill => wgpu::PolygonMode::Fill,
            Self::Line => wgpu::PolygonMode::Line,
            Self::Point => wgpu::PolygonMode::Point,
        }
    }
}

/// 正面绕序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FrontFaceWinding {
    Ccw,
    Cw,
}

impl FrontFaceWinding {
    #[inline]
    pub fn to_u32(self) -> u32 {
        match self {
            Self::Ccw => 0,
            Self::Cw => 1,
        }
    }
    #[inline]
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Cw,
            _ => Self::Ccw,
        }
    }
}

/// 光栅化描述符（合并为 cull+raster 域）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RasterState {
    pub cull: CullMode,
    pub polygon: PolygonMode,
    pub front_face: FrontFaceWinding,
    pub conservative: bool,
}

impl Default for RasterState {
    fn default() -> Self {
        Self {
            cull: CullMode::None,
            polygon: PolygonMode::Fill,
            front_face: FrontFaceWinding::Ccw,
            conservative: false,
        }
    }
}

/// 深度比较函数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompareFunc {
    Never,
    Less,
    Equal,
    LessEq,
    Greater,
    NotEq,
    GreaterEq,
    Always,
}

impl CompareFunc {
    #[inline]
    pub fn to_u32(self) -> u32 {
        match self {
            Self::Never => 0,
            Self::Less => 1,
            Self::Equal => 2,
            Self::LessEq => 3,
            Self::Greater => 4,
            Self::NotEq => 5,
            Self::GreaterEq => 6,
            Self::Always => 7,
        }
    }
    #[inline]
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Less,
            2 => Self::Equal,
            3 => Self::LessEq,
            4 => Self::Greater,
            5 => Self::NotEq,
            6 => Self::GreaterEq,
            7 => Self::Always,
            _ => Self::Never,
        }
    }
    pub fn to_wgpu(self) -> wgpu::CompareFunction {
        match self {
            Self::Never => wgpu::CompareFunction::Never,
            Self::Less => wgpu::CompareFunction::Less,
            Self::Equal => wgpu::CompareFunction::Equal,
            Self::LessEq => wgpu::CompareFunction::LessEqual,
            Self::Greater => wgpu::CompareFunction::Greater,
            Self::NotEq => wgpu::CompareFunction::NotEqual,
            Self::GreaterEq => wgpu::CompareFunction::GreaterEqual,
            Self::Always => wgpu::CompareFunction::Always,
        }
    }
}

/// 深度状态描述符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepthState {
    pub test: bool,
    pub write: bool,
    pub compare: CompareFunc,
}

impl Default for DepthState {
    fn default() -> Self {
        Self {
            test: false,
            write: false,
            compare: CompareFunc::Less,
        }
    }
}

/// 模板状态描述符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StencilState {
    pub test: bool,
    pub write: bool,
    pub compare: CompareFunc,
}

impl Default for StencilState {
    fn default() -> Self {
        Self {
            test: false,
            write: false,
            compare: CompareFunc::Always,
        }
    }
}