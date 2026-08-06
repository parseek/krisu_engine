//! 几何 / 数据类型：精灵矩形、顶点、索引、Mesh CPU 暂存与安全写入封装。

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
