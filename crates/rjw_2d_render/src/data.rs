//! 几何 / 数据类型：精灵矩形（归一化 / 像素 UV）、顶点、索引、Mesh CPU 暂存与安全写入封装。

use crate::ArcTextureWrapped;

// ─── 常量 ─────────────────────────────────────────────────────

/// 单位四边形顶点数
pub const QUAD_VERT_COUNT: usize = 4;
/// 单位四边形三角形索引（u16）
pub const QUAD_TRI_INDICIES: [u16; 6] = [0, 1, 3, 3, 2, 0];

// ─── 几何 / 数据类型 ──────────────────────────────────────────

/// 精灵矩形：网格范围（世界坐标）+ 纹理 UV（已归一化到 `[0,1]`）。
///
/// 位置 / 尺寸 / UV 参数接受 `Vec2` 或 `(x, y)`（正方形写 `(x, x)`）。
///
/// ```ignore
/// r2d.sprite(SpriteRect::new((10.0, 20.0), (64.0, 64.0)), &tex);   // 整张纹理
/// r2d.sprite(SpriteRect::centered(pos, (32.0, 48.0)), &tex);       // 以 pos 为中心
/// r2d.sprite(SpriteRect::with_uv_tex(tl, wh, (8, 8), (16, 16), &tex), &tex);
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct SpriteRect {
    pub mesh_tl: glam::Vec2,
    pub mesh_wh: glam::Vec2,
    pub uv_tl: glam::Vec2,
    pub uv_wh: glam::Vec2,
}

impl SpriteRect {
    // ── 构造 ──

    /// **最常用**：世界矩形（左上角 + 尺寸）+ **整张纹理** UV。
    #[inline]
    pub fn new(tl: impl Into<glam::Vec2>, wh: impl Into<glam::Vec2>) -> Self {
        Self {
            mesh_tl: tl.into(),
            mesh_wh: wh.into(),
            uv_tl: glam::Vec2::ZERO,
            uv_wh: glam::Vec2::ONE,
        }
    }

    /// 以**中心点** + 尺寸构造（旋转 / 居中绘制更自然）。
    #[inline]
    pub fn centered(center: impl Into<glam::Vec2>, wh: impl Into<glam::Vec2>) -> Self {
        let wh = wh.into();
        Self::new(center.into() - wh * 0.5, wh)
    }

    /// 完整指定（UV 为**归一化** `[0,1]` 坐标）。
    #[inline]
    pub fn with_uv(
        tl: impl Into<glam::Vec2>,
        wh: impl Into<glam::Vec2>,
        uv_tl: impl Into<glam::Vec2>,
        uv_wh: impl Into<glam::Vec2>,
    ) -> Self {
        Self {
            mesh_tl: tl.into(),
            mesh_wh: wh.into(),
            uv_tl: uv_tl.into(),
            uv_wh: uv_wh.into(),
        }
    }

    /// 以**像素**指定纹理子区域：给纹理像素尺寸 `tex_wh`（**无需手算倒数**，内部归一化）。
    #[inline]
    pub fn with_uv_px(
        tl: impl Into<glam::Vec2>,
        wh: impl Into<glam::Vec2>,
        uv_tl_px: impl Into<glam::Vec2>,
        uv_wh_px: impl Into<glam::Vec2>,
        tex_wh: impl Into<glam::Vec2>,
    ) -> Self {
        let inv = 1.0 / tex_wh.into().max(glam::Vec2::ONE);
        Self {
            mesh_tl: tl.into(),
            mesh_wh: wh.into(),
            uv_tl: uv_tl_px.into() * inv,
            uv_wh: uv_wh_px.into() * inv,
        }
    }

    /// 以**像素**指定纹理子区域，纹理像素尺寸直接取自 [`ArcTextureWrapped`]。
    #[inline]
    pub fn with_uv_tex(
        tl: impl Into<glam::Vec2>,
        wh: impl Into<glam::Vec2>,
        uv_tl_px: impl Into<glam::Vec2>,
        uv_wh_px: impl Into<glam::Vec2>,
        tex: &ArcTextureWrapped,
    ) -> Self {
        Self::with_uv_px(tl, wh, uv_tl_px, uv_wh_px, tex_size(tex))
    }

    // ── 链式调整（`Copy`，返回新值） ──

    /// 移动（保持尺寸与 UV）。
    #[inline]
    pub fn at(self, tl: impl Into<glam::Vec2>) -> Self {
        Self {
            mesh_tl: tl.into(),
            ..self
        }
    }

    /// 相对移动。
    #[inline]
    pub fn move_by(self, delta: impl Into<glam::Vec2>) -> Self {
        self.at(self.mesh_tl + delta.into())
    }

    /// 改世界尺寸（保持左上角与 UV）。
    #[inline]
    pub fn size(self, wh: impl Into<glam::Vec2>) -> Self {
        Self {
            mesh_wh: wh.into(),
            ..self
        }
    }

    /// 改**归一化** UV。
    #[inline]
    pub fn uv(self, uv_tl: impl Into<glam::Vec2>, uv_wh: impl Into<glam::Vec2>) -> Self {
        Self {
            uv_tl: uv_tl.into(),
            uv_wh: uv_wh.into(),
            ..self
        }
    }

    /// 改**像素** UV（`tex_wh` = 纹理像素尺寸）。
    #[inline]
    pub fn uv_px(
        self,
        uv_tl_px: impl Into<glam::Vec2>,
        uv_wh_px: impl Into<glam::Vec2>,
        tex_wh: impl Into<glam::Vec2>,
    ) -> Self {
        let inv = 1.0 / tex_wh.into().max(glam::Vec2::ONE);
        self.uv(uv_tl_px.into() * inv, uv_wh_px.into() * inv)
    }

    /// 世界矩形**各边**收窄（`f32` = 四边同值 / `(x, y)` = 左右、上下 / [`Edges`] 逐边；负值即外扩）。
    #[inline]
    pub fn shrink(self, edges: impl Into<Edges>) -> Self {
        let e = edges.into();
        Self {
            mesh_tl: self.mesh_tl + glam::Vec2::new(e.left, e.top),
            mesh_wh: self.mesh_wh - glam::Vec2::new(e.left + e.right, e.top + e.bottom),
            ..self
        }
    }

    /// 归一化 UV **各边**收窄（参数同 [`Self::shrink`]）。
    #[inline]
    pub fn shrink_uv(self, edges: impl Into<Edges>) -> Self {
        let e = edges.into();
        Self {
            uv_tl: self.uv_tl + glam::Vec2::new(e.left, e.top),
            uv_wh: self.uv_wh - glam::Vec2::new(e.left + e.right, e.top + e.bottom),
            ..self
        }
    }
}

/// 纹理像素尺寸（`Vec2`；1×1 下限防除零）。
#[inline]
fn tex_size(tex: &ArcTextureWrapped) -> glam::Vec2 {
    glam::Vec2::new(tex.width as f32, tex.height as f32).max(glam::Vec2::ONE)
}

// ─── 四边量（裁剪类特效） ─────────────────────────────────────

/// 四边量（左 / 右 / 上 / 下）：一律表示**每边各自**的量（不是总量）。
///
/// 用于 [`SpriteRectPx`] 的收缩 / 展开 / 越界扩展。构造方式：
/// - `Edges::all(v)`：四边同值（等价直接传 `f32`）
/// - `Edges::xy(x, y)`：左右 `x`、上下 `y`（等价直接传 `(x, y)` / `Vec2`）
/// - `Edges::lrtb(l, r, t, b)`：逐边指定
/// - `Edges::new().left(8.0)`：链式只改某一边
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Edges {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl Edges {
    pub const ZERO: Self = Self {
        left: 0.0,
        right: 0.0,
        top: 0.0,
        bottom: 0.0,
    };

    #[inline]
    pub const fn new() -> Self {
        Self::ZERO
    }

    /// 四边同值。
    #[inline]
    pub const fn all(v: f32) -> Self {
        Self {
            left: v,
            right: v,
            top: v,
            bottom: v,
        }
    }

    /// 左右 = `x`，上下 = `y`。
    #[inline]
    pub const fn xy(x: f32, y: f32) -> Self {
        Self {
            left: x,
            right: x,
            top: y,
            bottom: y,
        }
    }

    /// 逐边指定（左 / 右 / 上 / 下）。
    #[inline]
    pub const fn lrtb(left: f32, right: f32, top: f32, bottom: f32) -> Self {
        Self {
            left,
            right,
            top,
            bottom,
        }
    }

    #[inline]
    pub fn left(mut self, v: f32) -> Self {
        self.left = v;
        self
    }

    #[inline]
    pub fn right(mut self, v: f32) -> Self {
        self.right = v;
        self
    }

    #[inline]
    pub fn top(mut self, v: f32) -> Self {
        self.top = v;
        self
    }

    #[inline]
    pub fn bottom(mut self, v: f32) -> Self {
        self.bottom = v;
        self
    }
}

/// `f32` → 四边同值。
impl From<f32> for Edges {
    #[inline]
    fn from(v: f32) -> Self {
        Self::all(v)
    }
}

/// `(x, y)` → 左右 `x`、上下 `y`。
impl From<(f32, f32)> for Edges {
    #[inline]
    fn from(v: (f32, f32)) -> Self {
        Self::xy(v.0, v.1)
    }
}

impl From<[f32; 2]> for Edges {
    #[inline]
    fn from(v: [f32; 2]) -> Self {
        Self::xy(v[0], v[1])
    }
}

impl From<glam::Vec2> for Edges {
    #[inline]
    fn from(v: glam::Vec2) -> Self {
        Self::xy(v.x, v.y)
    }
}

// ─── 精灵矩形（像素 UV） ──────────────────────────────────────

/// 精灵矩形（**像素 UV** 版）：`uv_tl` / `uv_wh` 以**像素**为单位，便于做裁剪类特效。
///
/// 持有纹理像素尺寸 [`SpriteRectPx::tex_wh`]，用 [`SpriteRectPx::to_sprite_rect`] /
/// `From` 可无损转为归一化 UV 的 [`SpriteRect`]。
///
/// ```ignore
/// let r = SpriteRectPx::of_tex(pos, size, &tex);                    // 整张纹理
/// let r = SpriteRectPx::with_uv_px(pos, size, (8, 8), (32, 32), (256, 128));
/// let r = r.shrink(4.0).expand(Edges::new().left(8.0));            // 裁剪特效
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct SpriteRectPx {
    pub mesh_tl: glam::Vec2, // 世界坐标左上角
    pub mesh_wh: glam::Vec2, // 世界尺寸
    pub uv_tl: glam::Vec2,   // 纹理子区左上角（像素）
    pub uv_wh: glam::Vec2,   // 纹理子区尺寸（像素）
    pub tex_wh: glam::Vec2,  // 纹理尺寸（像素）
}

impl SpriteRectPx {
    // ── 构造 ──

    /// **最常用**：世界矩形 + **整张纹理**（UV 覆盖全图）。
    #[inline]
    pub fn new(
        tl: impl Into<glam::Vec2>,
        wh: impl Into<glam::Vec2>,
        tex_wh: impl Into<glam::Vec2>,
    ) -> Self {
        let tex_wh = tex_wh.into().max(glam::Vec2::ONE);
        Self {
            mesh_tl: tl.into(),
            mesh_wh: wh.into(),
            uv_tl: glam::Vec2::ZERO,
            uv_wh: tex_wh,
            tex_wh,
        }
    }

    /// 整张纹理（像素尺寸取自 [`ArcTextureWrapped`]）。
    #[inline]
    pub fn of_tex(
        tl: impl Into<glam::Vec2>,
        wh: impl Into<glam::Vec2>,
        tex: &ArcTextureWrapped,
    ) -> Self {
        Self::new(tl, wh, tex_size(tex))
    }

    /// 以**中心点** + 尺寸构造。
    #[inline]
    pub fn centered(
        center: impl Into<glam::Vec2>,
        wh: impl Into<glam::Vec2>,
        tex_wh: impl Into<glam::Vec2>,
    ) -> Self {
        let wh = wh.into();
        Self::new(center.into() - wh * 0.5, wh, tex_wh)
    }

    /// 以**像素**指定纹理子区域。
    #[inline]
    pub fn with_uv_px(
        tl: impl Into<glam::Vec2>,
        wh: impl Into<glam::Vec2>,
        uv_tl_px: impl Into<glam::Vec2>,
        uv_wh_px: impl Into<glam::Vec2>,
        tex_wh: impl Into<glam::Vec2>,
    ) -> Self {
        Self {
            mesh_tl: tl.into(),
            mesh_wh: wh.into(),
            uv_tl: uv_tl_px.into(),
            uv_wh: uv_wh_px.into(),
            tex_wh: tex_wh.into().max(glam::Vec2::ONE),
        }
    }

    /// 以**像素**指定纹理子区域（像素尺寸取自 [`ArcTextureWrapped`]）。
    #[inline]
    pub fn with_uv_tex(
        tl: impl Into<glam::Vec2>,
        wh: impl Into<glam::Vec2>,
        uv_tl_px: impl Into<glam::Vec2>,
        uv_wh_px: impl Into<glam::Vec2>,
        tex: &ArcTextureWrapped,
    ) -> Self {
        Self::with_uv_px(tl, wh, uv_tl_px, uv_wh_px, tex_size(tex))
    }

    // ── 链式调整（`Copy`，返回新值） ──

    /// 移动（保持尺寸与 UV）。
    #[inline]
    pub fn at(self, tl: impl Into<glam::Vec2>) -> Self {
        Self {
            mesh_tl: tl.into(),
            ..self
        }
    }

    /// 相对移动。
    #[inline]
    pub fn move_by(self, delta: impl Into<glam::Vec2>) -> Self {
        self.at(self.mesh_tl + delta.into())
    }

    /// 改世界尺寸（保持左上角与 UV）。
    #[inline]
    pub fn size(self, wh: impl Into<glam::Vec2>) -> Self {
        Self {
            mesh_wh: wh.into(),
            ..self
        }
    }

    /// 世界矩形**各边**收窄（`f32` = 四边同值 / `(x, y)` = 左右、上下 / [`Edges`] 逐边）。
    #[inline]
    pub fn shrink_mesh(self, edges: impl Into<Edges>) -> Self {
        let e = edges.into();
        Self {
            mesh_tl: self.mesh_tl + glam::Vec2::new(e.left, e.top),
            mesh_wh: self.mesh_wh - glam::Vec2::new(e.left + e.right, e.top + e.bottom),
            ..self
        }
    }

    // ── 裁剪特效（UV，单位为像素；每边量见 [`Edges`]） ──

    /// 各边收窄（像素）：超出按 `左 → 上 → 右 → 下` 顺序 clamp，尺寸不小于 0。
    #[inline]
    pub fn shrink(self, edges: impl Into<Edges>) -> Self {
        let e = edges.into();
        let w = self.uv_wh.x.max(0.0);
        let h = self.uv_wh.y.max(0.0);
        let l = e.left.max(0.0).min(w);
        let t = e.top.max(0.0).min(h);
        let r = e.right.max(0.0).min(w - l);
        let b = e.bottom.max(0.0).min(h - t);
        Self {
            uv_tl: self.uv_tl + glam::Vec2::new(l, t),
            uv_wh: glam::Vec2::new(w - l - r, h - t - b),
            ..self
        }
    }

    /// 各边外扩（像素）：clamp 在纹理边界内，不越界。
    #[inline]
    pub fn expand(self, edges: impl Into<Edges>) -> Self {
        let e = edges.into();
        let l = e.left.max(0.0).min(self.uv_tl.x.max(0.0));
        let t = e.top.max(0.0).min(self.uv_tl.y.max(0.0));
        let room_r = (self.tex_wh.x - (self.uv_tl.x + self.uv_wh.x)).max(0.0);
        let room_b = (self.tex_wh.y - (self.uv_tl.y + self.uv_wh.y)).max(0.0);
        let r = e.right.max(0.0).min(room_r);
        let b = e.bottom.max(0.0).min(room_b);
        Self {
            uv_tl: self.uv_tl - glam::Vec2::new(l, t),
            uv_wh: glam::Vec2::new(self.uv_wh.x + l + r, self.uv_wh.y + t + b),
            ..self
        }
    }

    /// 各边外扩（像素）：**不 clamp**，允许越过纹理边界（采样依赖寻址模式）。
    #[inline]
    pub fn exceed(self, edges: impl Into<Edges>) -> Self {
        let e = edges.into();
        Self {
            uv_tl: self.uv_tl - glam::Vec2::new(e.left, e.top),
            uv_wh: glam::Vec2::new(
                self.uv_wh.x + e.left + e.right,
                self.uv_wh.y + e.top + e.bottom,
            ),
            ..self
        }
    }

    /// 转为归一化 UV 的 [`SpriteRect`]（`tex_wh` 各轴按 `max(1.0)` 防除零）。
    #[inline]
    pub fn to_sprite_rect(&self) -> SpriteRect {
        let inv = 1.0 / self.tex_wh.max(glam::Vec2::ONE);
        SpriteRect::with_uv(self.mesh_tl, self.mesh_wh, self.uv_tl * inv, self.uv_wh * inv)
    }
}

impl From<SpriteRectPx> for SpriteRect {
    #[inline]
    fn from(v: SpriteRectPx) -> Self {
        v.to_sprite_rect()
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
    /// 以**局部**索引构造一个三角形（`mesh_with_cap` 闭包内使用；
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

/// `mesh_with` 闭包内使用的安全网格写入封装。
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
    pub fn push_vertex(&mut self, pos: impl Into<glam::Vec2>) -> u16 {
        let pos = pos.into();
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
    pub fn push_vertex_uv(&mut self, pos: impl Into<glam::Vec2>, uv: impl Into<glam::Vec2>) -> u16 {
        let (pos, uv) = (pos.into(), uv.into());
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
    pub fn push_vertex_uv_color(
        &mut self,
        pos: impl Into<glam::Vec2>,
        uv: impl Into<glam::Vec2>,
        color: [f32; 4],
    ) -> u16 {
        let (pos, uv) = (pos.into(), uv.into());
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
