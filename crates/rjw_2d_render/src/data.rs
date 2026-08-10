//! 几何 / 数据类型：精灵矩形（归一化 / 像素 UV）、顶点、索引、Mesh CPU 暂存与安全写入封装。

use crate::ArcTextureWrapped;

// ─── 常量 ─────────────────────────────────────────────────────

/// 单位四边形顶点数
pub const QUAD_VERT_COUNT: usize = 4;
/// 单位四边形三角形索引（u16）
pub const QUAD_TRI_INDICIES: [u16; 6] = [0, 1, 3, 3, 2, 0];

// ─── 几何 / 数据类型 ──────────────────────────────────────────

/// 精灵矩形：网格范围（世界坐标）+ 纹理 UV（已归一化到 `[0,1]`）
#[derive(Debug, Default, Clone, Copy)]
pub struct SpriteRect {
    pub mesh_tl: glam::Vec2,
    pub mesh_wh: glam::Vec2,
    pub uv_tl: glam::Vec2,
    pub uv_wh: glam::Vec2,
}

impl SpriteRect {
    /// 手动指定（UV 为 `[0,1]` 归一化坐标）
    #[inline]
    pub const fn new(
        mesh_tl: glam::Vec2,
        mesh_wh: glam::Vec2,
        uv_tl: glam::Vec2,
        uv_wh: glam::Vec2,
    ) -> Self {
        Self {
            mesh_tl,
            mesh_wh,
            uv_tl,
            uv_wh,
        }
    }

    /// 整张纹理平铺
    #[inline]
    pub fn from_texture(mesh_tl: glam::Vec2, mesh_wh: glam::Vec2) -> Self {
        Self {
            mesh_tl,
            mesh_wh,
            uv_tl: glam::Vec2::ZERO,
            uv_wh: glam::Vec2::ONE,
        }
    }

    /// 以**像素**指定纹理子区域，自动归一化到 `[0,1]`
    #[inline]
    pub fn from_texture_px(
        mesh_tl: glam::Vec2,
        mesh_wh: glam::Vec2,
        uv_tl_px: glam::Vec2,
        uv_wh_px: glam::Vec2,
        inv_tex_wh: glam::Vec2,
    ) -> Self {
        Self {
            mesh_tl,
            mesh_wh,
            uv_tl: glam::Vec2::new(uv_tl_px.x, uv_tl_px.y) * inv_tex_wh,
            uv_wh: glam::Vec2::new(uv_wh_px.x, uv_wh_px.y) * inv_tex_wh,
        }
    }
}

impl SpriteRect {
    #[inline]
    pub fn shrink_mesh_x(self, sh: f32) -> Self {
        let mut mesh_wh = self.mesh_wh;
        let mut mesh_tl = self.mesh_tl;
        mesh_wh.x -= sh*2.0;
        mesh_tl.x += sh;
        Self { mesh_tl, mesh_wh, ..self }
    }
    #[inline]
    pub fn shrink_mesh_y(self, sh: f32) -> Self {
        let mut mesh_wh = self.mesh_wh;
        let mut mesh_tl = self.mesh_tl;
        mesh_wh.y -= sh*2.0;
        mesh_tl.y += sh;
        Self { mesh_tl, mesh_wh, ..self }
    }
    #[inline]
    pub fn shrink_mesh(self, x: f32, y: f32) -> Self {
        self.shrink_mesh_x(x).shrink_mesh_y(y)
    }
    #[inline]
    pub fn shrink_uv_x(self, sh: f32) -> Self {
        let mut uv_wh = self.uv_wh;
        let mut uv_tl = self.uv_tl;
        uv_wh.x -= sh*2.0;
        uv_tl.x += sh;
        Self { uv_tl, uv_wh, ..self }
    }
    #[inline]
    pub fn shrink_uv_y(self, sh: f32) -> Self {
        let mut uv_wh = self.uv_wh;
        let mut uv_tl = self.uv_tl;
        uv_wh.y -= sh*2.0;
        uv_tl.y += sh;
        Self { uv_tl, uv_wh, ..self }
    }
    #[inline]
    pub fn shrink_uv(self, u: f32, v: f32) -> Self {
        self.shrink_uv_x(u).shrink_uv_y(v)
    }
}

/// 精灵矩形（像素 UV 版）：`uv_tl` / `uv_wh` 以**像素**为单位（而非归一化坐标），
/// 便于实现裁剪类特效（shrink、expand_left、expand_down 等）。
///
/// 持有纹理像素尺寸 [`SpriteRectPx::tex_wh`]，通过 [`SpriteRectPx::to_sprite_rect`] /
/// `From` 可无损转为归一化 UV 的 [`SpriteRect`]。
/// 引擎主要使用 [`ArcTextureWrapped`]（内置 `width`/`height`），可直接用
/// [`SpriteRectPx::from_tex`] / [`SpriteRectPx::from_tex_px`] 构造。
#[derive(Debug, Default, Clone, Copy)]
pub struct SpriteRectPx {
    pub mesh_tl: glam::Vec2, // 世界坐标左上角
    pub mesh_wh: glam::Vec2, // 世界尺寸
    pub uv_tl: glam::Vec2,   // 纹理子区左上角（像素）
    pub uv_wh: glam::Vec2,   // 纹理子区尺寸（像素）
    pub tex_wh: glam::Vec2,  // 纹理尺寸（像素）
}

impl SpriteRectPx {
    /// 手动指定（UV 为**像素**坐标）
    #[inline]
    pub const fn new(
        mesh_tl: glam::Vec2,
        mesh_wh: glam::Vec2,
        uv_tl: glam::Vec2,
        uv_wh: glam::Vec2,
        tex_wh: glam::Vec2,
    ) -> Self {
        Self {
            mesh_tl,
            mesh_wh,
            uv_tl,
            uv_wh,
            tex_wh,
        }
    }

    /// 整张纹理平铺
    #[inline]
    pub fn from_texture(mesh_tl: glam::Vec2, mesh_wh: glam::Vec2, tex_wh: glam::Vec2) -> Self {
        Self {
            mesh_tl,
            mesh_wh,
            uv_tl: glam::Vec2::ZERO,
            uv_wh: tex_wh,
            tex_wh,
        }
    }

    /// 整张纹理平铺（纹理尺寸取自 `ArcTextureWrapped` 内置的 `width` / `height`）
    #[inline]
    pub fn from_tex(mesh_tl: glam::Vec2, mesh_wh: glam::Vec2, tex: &ArcTextureWrapped) -> Self {
        Self::from_texture(
            mesh_tl,
            mesh_wh,
            glam::Vec2::new(tex.width as f32, tex.height as f32),
        )
    }

    /// 以**像素**指定纹理子区域（纹理尺寸取自 `ArcTextureWrapped` 内置的 `width` / `height`）
    #[inline]
    pub fn from_tex_px(
        mesh_tl: glam::Vec2,
        mesh_wh: glam::Vec2,
        uv_tl: glam::Vec2,
        uv_wh: glam::Vec2,
        tex: &ArcTextureWrapped,
    ) -> Self {
        Self::new(
            mesh_tl,
            mesh_wh,
            uv_tl,
            uv_wh,
            glam::Vec2::new(tex.width as f32, tex.height as f32),
        )
    }

    /// 以**像素**指定纹理子区域（纹理尺寸取自 `ArcTextureWrapped` 内置的 `width` / `height`）
    #[inline]
    pub fn from_tex_wh_px(
        mesh_tl: glam::Vec2,
        mesh_wh: glam::Vec2,
        uv_tl: glam::Vec2,
        uv_wh: glam::Vec2,
        tex_wh: glam::Vec2,
    ) -> Self {
        Self::new(
            mesh_tl,
            mesh_wh,
            uv_tl,
            uv_wh,
            tex_wh,
        )
    }

    /// 转为归一化 UV 的 [`SpriteRect`]（`tex_wh` 各轴按 `max(1.0)` 防除零）
    #[inline]
    pub fn to_sprite_rect(&self) -> SpriteRect {
        let safe = 1.0 / self.tex_wh.max(glam::Vec2::splat(1.0));
        SpriteRect::new(self.mesh_tl, self.mesh_wh, self.uv_tl * safe, self.uv_wh * safe)
    }
}

impl From<SpriteRectPx> for SpriteRect {
    #[inline]
    fn from(v: SpriteRectPx) -> Self {
        v.to_sprite_rect()
    }
}

impl SpriteRectPx {
    /// 左右两侧各收窄 `sh`（世界坐标，同 [`SpriteRect::shrink_mesh_x`]）
    #[inline]
    pub fn shrink_mesh_x(self, sh: f32) -> Self {
        let mut mesh_wh = self.mesh_wh;
        let mut mesh_tl = self.mesh_tl;
        mesh_wh.x -= sh * 2.0;
        mesh_tl.x += sh;
        Self { mesh_tl, mesh_wh, ..self }
    }

    /// 上下两侧各收窄 `sh`（世界坐标，同 [`SpriteRect::shrink_mesh_y`]）
    #[inline]
    pub fn shrink_mesh_y(self, sh: f32) -> Self {
        let mut mesh_wh = self.mesh_wh;
        let mut mesh_tl = self.mesh_tl;
        mesh_wh.y -= sh * 2.0;
        mesh_tl.y += sh;
        Self { mesh_tl, mesh_wh, ..self }
    }

    /// 四周各收窄（世界坐标）
    #[inline]
    pub fn shrink_mesh(self, x: f32, y: f32) -> Self {
        self.shrink_mesh_x(x).shrink_mesh_y(y)
    }

    /// UV 水平双侧各收窄 `px` 像素（居中收缩；`px` 过大时 clamp 到 0，不翻转）
    #[inline]
    pub fn shrink_uv_x(self, px: f32) -> Self {
        let w = self.uv_wh.x;
        let nw = (w - px * 2.0).max(0.0);
        let d = (w - nw) * 0.5;
        Self {
            uv_tl: self.uv_tl + glam::Vec2::new(d, 0.0),
            uv_wh: glam::Vec2::new(nw, self.uv_wh.y),
            ..self
        }
    }

    /// UV 垂直双侧各收窄 `px` 像素（居中收缩；`px` 过大时 clamp 到 0，不翻转）
    #[inline]
    pub fn shrink_uv_y(self, px: f32) -> Self {
        let h = self.uv_wh.y;
        let nh = (h - px * 2.0).max(0.0);
        let d = (h - nh) * 0.5;
        Self {
            uv_tl: self.uv_tl + glam::Vec2::new(0.0, d),
            uv_wh: glam::Vec2::new(self.uv_wh.x, nh),
            ..self
        }
    }

    /// UV 四周各收窄（像素）
    #[inline]
    pub fn shrink_uv(self, x: f32, y: f32) -> Self {
        self.shrink_uv_x(x).shrink_uv_y(y)
    }

    /// UV 左侧收窄 `px` 像素（clamp 到宽度为 0）
    #[inline]
    pub fn shrink_left(self, px: f32) -> Self {
        let a = px.min(self.uv_wh.x.max(0.0));
        Self {
            uv_tl: self.uv_tl + glam::Vec2::new(a, 0.0),
            uv_wh: glam::Vec2::new(self.uv_wh.x - a, self.uv_wh.y),
            ..self
        }
    }

    /// UV 右侧收窄 `px` 像素（clamp 到宽度为 0）
    #[inline]
    pub fn shrink_right(self, px: f32) -> Self {
        let a = px.min(self.uv_wh.x.max(0.0));
        Self {
            uv_wh: glam::Vec2::new(self.uv_wh.x - a, self.uv_wh.y),
            ..self
        }
    }

    /// UV 上侧收窄 `px` 像素（clamp 到高度为 0）
    #[inline]
    pub fn shrink_up(self, px: f32) -> Self {
        let a = px.min(self.uv_wh.y.max(0.0));
        Self {
            uv_tl: self.uv_tl + glam::Vec2::new(0.0, a),
            uv_wh: glam::Vec2::new(self.uv_wh.x, self.uv_wh.y - a),
            ..self
        }
    }

    /// UV 下侧收窄 `px` 像素（clamp 到高度为 0）
    #[inline]
    pub fn shrink_down(self, px: f32) -> Self {
        let a = px.min(self.uv_wh.y.max(0.0));
        Self {
            uv_wh: glam::Vec2::new(self.uv_wh.x, self.uv_wh.y - a),
            ..self
        }
    }
}

impl SpriteRectPx {
    /// UV 左侧展开 `px` 像素（clamp 到纹理左边界，不越界）
    #[inline]
    pub fn expand_left(self, px: f32) -> Self {
        let a = px.min(self.uv_tl.x.max(0.0));
        Self {
            uv_tl: self.uv_tl - glam::Vec2::new(a, 0.0),
            uv_wh: glam::Vec2::new(self.uv_wh.x + a, self.uv_wh.y),
            ..self
        }
    }

    /// UV 右侧展开 `px` 像素（clamp 到纹理右边界，不越界）
    #[inline]
    pub fn expand_right(self, px: f32) -> Self {
        let a = px.min((self.tex_wh.x - (self.uv_tl.x + self.uv_wh.x)).max(0.0));
        Self {
            uv_wh: glam::Vec2::new(self.uv_wh.x + a, self.uv_wh.y),
            ..self
        }
    }

    /// UV 上侧展开 `px` 像素（clamp 到纹理上边界，不越界）
    #[inline]
    pub fn expand_up(self, px: f32) -> Self {
        let a = px.min(self.uv_tl.y.max(0.0));
        Self {
            uv_tl: self.uv_tl - glam::Vec2::new(0.0, a),
            uv_wh: glam::Vec2::new(self.uv_wh.x, self.uv_wh.y + a),
            ..self
        }
    }

    /// UV 下侧展开 `px` 像素（clamp 到纹理下边界，不越界）
    #[inline]
    pub fn expand_down(self, px: f32) -> Self {
        let a = px.min((self.tex_wh.y - (self.uv_tl.y + self.uv_wh.y)).max(0.0));
        Self {
            uv_wh: glam::Vec2::new(self.uv_wh.x, self.uv_wh.y + a),
            ..self
        }
    }

    /// UV 四周各展开 `px` 像素（clamp 到纹理边界）
    #[inline]
    pub fn expand(self, px: f32) -> Self {
        self.expand_left(px)
            .expand_right(px)
            .expand_up(px)
            .expand_down(px)
    }

    /// UV 左侧展开 `px` 像素（**不 Clamp**：允许 `uv_tl` 越过 0）
    #[inline]
    pub fn exceed_left(self, px: f32) -> Self {
        Self {
            uv_tl: self.uv_tl - glam::Vec2::new(px, 0.0),
            uv_wh: glam::Vec2::new(self.uv_wh.x + px, self.uv_wh.y),
            ..self
        }
    }

    /// UV 右侧展开 `px` 像素（**不 Clamp**：允许 UV 越过纹理右边界）
    #[inline]
    pub fn exceed_right(self, px: f32) -> Self {
        Self {
            uv_wh: glam::Vec2::new(self.uv_wh.x + px, self.uv_wh.y),
            ..self
        }
    }

    /// UV 上侧展开 `px` 像素（**不 Clamp**：允许 `uv_tl` 越过 0）
    #[inline]
    pub fn exceed_up(self, px: f32) -> Self {
        Self {
            uv_tl: self.uv_tl - glam::Vec2::new(0.0, px),
            uv_wh: glam::Vec2::new(self.uv_wh.x, self.uv_wh.y + px),
            ..self
        }
    }

    /// UV 下侧展开 `px` 像素（**不 Clamp**：允许 UV 越过纹理下边界）
    #[inline]
    pub fn exceed_down(self, px: f32) -> Self {
        Self {
            uv_wh: glam::Vec2::new(self.uv_wh.x, self.uv_wh.y + px),
            ..self
        }
    }
}

/// 顶点：位置 (3) + UV (2) + 颜色 (4)
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct VertexP3U2C4 {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// 单位四边形顶点（x: 0→1, y: 0→1），用于实例化渲染
pub const QUAD_VERTS: [VertexP3U2C4; QUAD_VERT_COUNT] = [
    VertexP3U2C4 {
        pos: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        color: [1.0; 4],
    },
    VertexP3U2C4 {
        pos: [1.0, 0.0, 0.0],
        uv: [1.0, 0.0],
        color: [1.0; 4],
    },
    VertexP3U2C4 {
        pos: [0.0, 1.0, 0.0],
        uv: [0.0, 1.0],
        color: [1.0; 4],
    },
    VertexP3U2C4 {
        pos: [1.0, 1.0, 0.0],
        uv: [1.0, 1.0],
        color: [1.0; 4],
    },
];

pub type Vertex = VertexP3U2C4;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct Index(pub(crate) u16);
impl Index {
    pub const FORMAT: wgpu::IndexFormat = wgpu::IndexFormat::Uint16;

    #[inline]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct TriIndicies(pub(crate) Index, pub(crate) Index, pub(crate) Index);
impl TriIndicies {
    /// 以**局部**索引构造一个三角形（`add_mesh_fn_prealloc` 闭包内使用；
    /// 内部会自动重定位为全局索引）。
    #[inline]
    pub const fn new(a: u16, b: u16, c: u16) -> Self {
        TriIndicies(Index::new(a), Index::new(b), Index::new(c))
    }
}

/// Mesh CPU 侧暂存（非实例化路径；录制顺序，prepare 时按排序重排拷入 DrawPage）
#[derive(Debug, Default)]
pub struct MeshStorage {
    pub vertices: Vec<VertexP3U2C4>,
    pub tri_indices: Vec<TriIndicies>,
}

impl MeshStorage {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.tri_indices.clear();
    }
}

/// `add_mesh_fn` 闭包内使用的安全网格写入封装。
///
/// 内部持有 `Render2D` 的 `MeshStorage` 的**可变借用**；闭包执行完毕后借用自动释放。
/// `push_vertex(pos)` 返回**局部**顶点索引（相对本 mesh 从 0 起），
/// `push_tri(a, b, c)` 接收局部索引并自动重定位为**全局**索引写入 Storage。
/// 所有 push 均带边界检查（debug 断言），保证不会越界。
pub struct MeshSink<'a> {
    /// 全局顶点基址（本 mesh 起始全局顶点号；`push_tri` 重定位用）
    pub(crate) base: u32,
    pub(crate) verts: &'a mut Vec<VertexP3U2C4>,
    pub(crate) tris: &'a mut Vec<TriIndicies>,
    pub(crate) color_arr: [f32; 4],
}

impl<'a> MeshSink<'a> {
    /// push 一个顶点（位置 → 世界坐标；UV 置 0；颜色取录制时传入的 `color`）。
    ///
    /// 返回该顶点的**局部索引**，可直接传给 `push_tri`。
    #[inline]
    pub fn push_vertex(&mut self, pos: glam::Vec2) -> u16 {
        let idx = self.verts.len() as u32 - self.base;
        debug_assert!(
            idx <= u16::MAX as u32,
            "too many vertices for u16 indices in one mesh"
        );
        self.verts.push(VertexP3U2C4 {
            pos: [pos.x, pos.y, 0.0],
            uv: [0.0, 0.0],
            color: self.color_arr,
        });
        idx as u16
    }

    /// push 一个顶点（位置 → 世界坐标；UV 手动指定；颜色取录制时传入的 `color`）。
    ///
    /// 返回该顶点的**局部索引**，可直接传给 `push_tri`。
    #[inline]
    pub fn push_vertex_uv(&mut self, pos: glam::Vec2, uv: glam::Vec2) -> u16 {
        let idx = self.verts.len() as u32 - self.base;
        debug_assert!(
            idx <= u16::MAX as u32,
            "too many vertices for u16 indices in one mesh"
        );
        self.verts.push(VertexP3U2C4 {
            pos: [pos.x, pos.y, 0.0],
            uv: [uv.x, uv.y],
            color: self.color_arr,
        });
        idx as u16
    }

    /// push 一个顶点（位置 + UV + **自定义逐顶点颜色**；不取录制时的 `color`）。
    ///
    /// 用于逐顶点渐变等场景（如文本渐变）。返回该顶点的**局部索引**。
    #[inline]
    pub fn push_vertex_uv_color(&mut self, pos: glam::Vec2, uv: glam::Vec2, color: [f32; 4]) -> u16 {
        let idx = self.verts.len() as u32 - self.base;
        debug_assert!(
            idx <= u16::MAX as u32,
            "too many vertices for u16 indices in one mesh"
        );
        self.verts.push(VertexP3U2C4 {
            pos: [pos.x, pos.y, 0.0],
            uv: [uv.x, uv.y],
            color,
        });
        idx as u16
    }

    /// push 一个三角形（`a`/`b`/`c` 为**局部**索引，需已在前面 push 过对应顶点）。
    ///
    /// 内部自动把局部索引 + 本 mesh 的全局基址，重定位为全局索引写入 Storage。
    #[inline]
    pub fn push_tri(&mut self, a: u16, b: u16, c: u16) {
        let n = self.verts.len() as u32 - self.base;
        debug_assert!(
            (a as u32) < n && (b as u32) < n && (c as u32) < n,
            "push_tri index out of bounds: local vertex count = {n}, got ({a}, {b}, {c})"
        );
        let base = self.base;
        self.tris.push(TriIndicies(
            Index((base + a as u32) as u16),
            Index((base + b as u32) as u16),
            Index((base + c as u32) as u16),
        ));
    }
}
