//! 统一绘制 Builder：[`Draw2D<K>`] 以 4 个 kind 取代旧的
//! `Sprite2DBuilder` / `MeshBuilder` / `StaticMeshBuilder` / `CustomBuilder`。
//!
//! 设计要点：
//! - **一份责任链**：`layer / color / transform / model / texture / states` 等修饰只有一处实现，
//!   4 个 Builder 不再各写一遍（旧实现 ~84 个转发方法）；
//! - **一份 `Drop`**：`Drop` 时按 kind 生成 [`DrawCommand`] 并 push 进队列，链式与不链式行为一致；
//! - **零堆分配**：`Draw2D` 是栈上 struct（与原 Builder 相同的性能约定）；
//! - 入口方法（`sprite` / `mesh` / `polygon` / `quads` / `static_mesh` / `custom`）在
//!   [`Render2D`](crate::Render2D) 上，构造 `Draw2D` 后返回给调用者继续链式修饰。
//!
//! ```no_run
//! # use rjw_2d_render::Render2D;
//! # let (mut r2d, tex, rect): (Render2D, rjw_2d_render::ArcTextureWrapped, rjw_2d_render::SpriteRect) = unimplemented!();
//! use rjw_2d_render::{BlendMode, Color};
//! r2d.sprite(rect, &tex)
//!     .tint(Color::WHITE)
//!     .layer(10.0)
//!     .blend(BlendMode::Additive);
//! ```

use std::marker::PhantomData;
use std::ops::Range;

use glam::Vec2;
use rjw_color::Color;
use rjw_render::ArcTextureWrapped;
use rjw_transform::Transform2D;

use crate::command::{DrawCommand, DrawCommandQueue, Layer, States};
use crate::data::{Index, MeshSink, MeshStorage, SpriteRect, TriIndices, VertexP3U2C4};
use crate::rstates::{
    AddressMode, BlendDesc, BlendMode, CullMode, DepthState, FilterMode, RStates, RasterState,
    SamplerDesc, StencilState,
};

// ─── 外部绘制 trait ───────────────────────────────────────────

/// 外部绘制 trait：实现此 trait 的结构体/闭包可通过
/// [`Render2D::custom`](crate::Render2D::custom) 注入绘制队列。
/// 渲染器持有 `Arc<dyn CustomDraw>`，`draw()` 中可安全共享引用。
pub trait CustomDraw: Send + Sync {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>);
}

/// 闭包的 blanket impl——直接传 `|pass| { ... }` 即可。
impl<F: Fn(&mut wgpu::RenderPass<'_>) + Send + Sync> CustomDraw for F {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self(pass);
    }
}

// ─── Kind 标记 ────────────────────────────────────────────────

/// Sprite kind（贴纹理 / 纯色四边形，实例化合批）。
#[doc(hidden)]
pub enum Sprite {}
/// Mesh kind（动态顶点段：`mesh` / `mesh_with` / `polygon` / `quads`）。
#[doc(hidden)]
pub enum Mesh {}
/// StaticMesh kind（`MESHES` 注册表网格 + 实例化合批）。
#[doc(hidden)]
pub enum StaticMesh {}
/// Custom kind（原生 wgpu 绘制调用注入）。
#[doc(hidden)]
pub enum Custom {}

/// `color()` 的作用位置（每个入口决定，调用者无需关心）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorMode {
    /// 写入**逐顶点颜色**（入口自行生成顶点时：`mesh` / `mesh_with` / `polygon`）。
    Vertex,
    /// 作为**整段实例色**（顶点由调用者提供时：`quads`，等价于旧的 `add_quads_styled`）。
    Instance,
    /// 写入命令自带颜色（实例化路径：`sprite` / `solid` / `static_mesh`）。
    Command,
}

/// Kind 行为：如何把当前 Builder 状态落成一条 [`DrawCommand`]。
///
/// 引擎内部机制（类型级标记，用户通过 `Render2D` 的入口方法间接使用）；
/// 自定义绘制请用 [`Render2D::custom`](crate::Render2D::custom) + [`CustomDraw`]。
#[doc(hidden)]
pub trait DrawKind: Sized + 'static {
    #[doc(hidden)]
    fn commit(b: &mut Draw2D<'_, Self>);
}

// ─── 统一 Builder ─────────────────────────────────────────────

/// 统一绘制 Builder（由 `Render2D` 的入口方法构造）。
///
/// **Drop 即提交**：不链式调用也会在语句结束时提交一次；
/// 想显式表达"到此为止"可调用 [`Draw2D::done`]。
pub struct Draw2D<'a, K: DrawKind> {
    queue: &'a mut DrawCommandQueue,
    mesh: &'a mut MeshStorage,

    // ── kind 负载（按 kind 使用，其余保持默认）──
    rect: SpriteRect,
    mesh_id: u64,
    vert: Range<usize>,
    tri: Range<usize>,
    custom_idx: usize,

    // ── 通用修饰 ──
    tex: Option<u64>,
    color: Color,
    color_mode: ColorMode,
    /// 整段实例色（`ColorMode::Instance` 时生效）→ `DrawCommand::MeshStyled`。
    tint: Option<Color>,
    transform: Transform2D,
    /// 显式模型矩阵（`.matrix(..)`）优先于 `transform`。
    model: Option<glam::Mat4>,
    /// `None` = 继承渲染器全局默认状态（`Render2D::states()`）。
    states: Option<RStates>,
    layer: Layer,

    _k: PhantomData<K>,
}

impl<'a, K: DrawKind> Draw2D<'a, K> {
    /// 统一构造：kind 负载随后由入口方法填充。
    fn base(
        queue: &'a mut DrawCommandQueue,
        mesh: &'a mut MeshStorage,
        color_mode: ColorMode,
        tex: Option<u64>,
    ) -> Self {
        Self {
            queue,
            mesh,
            rect: SpriteRect::default(),
            mesh_id: 0,
            vert: 0..0,
            tri: 0..0,
            custom_idx: 0,
            tex,
            color: Color::WHITE,
            color_mode,
            tint: None,
            transform: Transform2D::IDENTITY,
            model: None,
            states: None,
            layer: Layer::default(),
            _k: PhantomData,
        }
    }

    /// 提交一条命令（kind 的 `commit` 调用）。
    pub(crate) fn commit_command(&mut self, cmd: DrawCommand) {
        self.queue.push(
            cmd,
            self.layer,
            States {
                rstates: self.states,
                texture_uid: self.tex,
            },
        );
    }

    /// 显式提交（等价于让 Builder 离开作用域被 Drop）。
    #[inline]
    pub fn done(self) {}
}

// ─── 责任链（4 个 kind 共用同一份实现） ───────────────────────

impl<K: DrawKind> Draw2D<'_, K> {
    /// 绘制层级（数值小先绘制；默认 `0.0`）。
    #[inline]
    pub fn layer(mut self, layer: impl Into<Layer>) -> Self {
        self.layer = layer.into();
        self
    }

    /// 着色。作用位置由入口决定（**对调用者透明**）：
    /// - `sprite` / `solid` / `static_mesh`：实例色（默认 `WHITE`）；
    /// - `mesh` / `mesh_with` / `polygon`：**逐顶点色**（默认 `WHITE`，保持动态段可合批）；
    /// - `quads` / `quads_with`：**整段实例色**（顶点自带色 × 该色）。
    ///
    /// 名字统一为 `tint`（旧名 `color` 语义随 kind 变，已按 `docs/API_DESIGN.md` R6 收敛）。
    #[inline]
    pub fn tint(mut self, color: Color) -> Self {
        self.color = color;
        match self.color_mode {
            ColorMode::Vertex => {
                let ca: [f32; 4] = color.into();
                for v in &mut self.mesh.vertices[self.vert.clone()] {
                    v.color = ca;
                }
            }
            ColorMode::Instance => self.tint = Some(color),
            ColorMode::Command => {}
        }
        self
    }

    /// 局部 → 世界变换（默认 `IDENTITY`：顶点即世界坐标）。
    #[inline]
    pub fn transform(mut self, t: Transform2D) -> Self {
        self.transform = t;
        self.model = None;
        self
    }

    /// 平移（`transform` 便捷糖；接受 `Vec2` 或 `(x, y)`）。
    ///
    /// 名字为 `at`（旧名 `pos` 与 `Transform2D` 的字段/构建器同义异名，已收敛）。
    #[inline]
    pub fn at(mut self, pos: impl Into<Vec2>) -> Self {
        self.transform.pos = pos.into();
        self.model = None;
        self
    }

    /// 旋转（弧度；`transform` 便捷糖）。
    #[inline]
    pub fn rot(mut self, rotation: f32) -> Self {
        self.transform.rotation = rotation;
        self.model = None;
        self
    }

    /// 缩放（`transform` 便捷糖；接受 `Vec2` 或 `(x, y)`）。
    #[inline]
    pub fn scale(mut self, scale: impl Into<Vec2>) -> Self {
        self.transform.scale = scale.into();
        self.model = None;
        self
    }

    /// 直接给出**列主序模型矩阵**（跳过 `Transform2D` 推导；覆盖 `.transform/.at/.rot/.scale`）。
    ///
    /// 名字统一为 `matrix`（旧名 `model` 语义与字段重名，已按 `docs/API_DESIGN.md` §8.3 收敛）。
    #[inline]
    pub fn matrix(mut self, model: glam::Mat4) -> Self {
        self.model = Some(model);
        self
    }

    /// 覆盖采样纹理（Mesh 系入口默认白纹理，其他入口为入口参数）。
    #[inline]
    pub fn texture(mut self, texture: &ArcTextureWrapped) -> Self {
        self.tex = Some(texture.uid);
        self
    }

    /// 完整渲染状态（**唯一状态语言**；不调用则继承 `Render2D::states()`）。
    #[inline]
    pub fn states(mut self, states: RStates) -> Self {
        self.states = Some(states);
        self
    }

    /// 混合模式（`states` 便捷糖）。
    #[inline]
    pub fn blend(self, mode: BlendMode) -> Self {
        self.with_states(|s| s.blend(mode))
    }

    /// 采样器过滤 + 寻址（`states` 便捷糖）。
    #[inline]
    pub fn samp(self, filter: FilterMode, addr: AddressMode) -> Self {
        self.with_states(|s| s.samp_filter_all(filter).samp_addr_all(addr))
    }

    /// 剔除面（`states` 便捷糖）。
    #[inline]
    pub fn cull(self, mode: CullMode) -> Self {
        self.with_states(|s| s.cull(mode))
    }

    /// 深度状态（`states` 便捷糖）：接受 [`DepthState`]（或 `bool` 便捷：`true` = 只测试）。
    #[inline]
    pub fn depth(self, state: impl Into<DepthState>) -> Self {
        let d = state.into();
        self.with_states(|s| s.depth_test(d.test).depth_write(d.write).depth_compare(d.compare))
    }

    /// 模板状态（`states` 便捷糖）：接受 [`StencilState`]（或 `bool` 便捷：`true` = 只测试）。
    #[inline]
    pub fn stencil(self, state: impl Into<StencilState>) -> Self {
        let d = state.into();
        self.with_states(|s| s.stencil_test(d.test).stencil_write(d.write).stencil_compare(d.compare))
    }

    /// 批量设置 Blit 描述符（`states` 便捷糖）。
    #[inline]
    pub fn blend_state(self, d: BlendDesc) -> Self {
        self.with_states(|s| s.blend_state(d))
    }

    /// 批量设置采样器描述符（`states` 便捷糖）。
    #[inline]
    pub fn samp_state(self, d: SamplerDesc) -> Self {
        self.with_states(|s| s.samp_state(d))
    }

    /// 批量设置光栅化状态（`states` 便捷糖）。
    #[inline]
    pub fn raster_state(self, s: RasterState) -> Self {
        self.with_states(|st| st.raster_state(s))
    }

    /// 组合：把状态从 `None`（继承）落到 `Some(..)` 上。
    #[inline]
    fn with_states(mut self, f: impl FnOnce(RStates) -> RStates) -> Self {
        self.states = Some(f(self.states.unwrap_or_default()));
        self
    }
}

// ─── Kind 落地 + 统一 Drop ────────────────────────────────────

impl DrawKind for Sprite {
    fn commit(b: &mut Draw2D<'_, Self>) {
        let (rect, color, transform) = (b.rect, b.color, b.transform);
        let model = b.model;
        let cmd = match model {
            Some(m) => {
                let mat_idx = b.queue.matrices.len();
                b.queue.matrices.push(m);
                DrawCommand::Sprite2DMatrix {
                    rect,
                    color,
                    mat_idx,
                }
            }
            None => DrawCommand::Sprite2D {
                rect,
                color,
                transform,
            },
        };
        b.commit_command(cmd);
    }
}

impl DrawKind for StaticMesh {
    fn commit(b: &mut Draw2D<'_, Self>) {
        let (mesh_id, color, transform) = (b.mesh_id, b.color, b.transform);
        let model = b.model;
        let cmd = match model {
            Some(m) => {
                let mat_idx = b.queue.matrices.len();
                b.queue.matrices.push(m);
                DrawCommand::StaticMeshMatrix {
                    mesh_id,
                    color,
                    mat_idx,
                }
            }
            None => DrawCommand::StaticMesh {
                mesh_id,
                color,
                transform,
            },
        };
        b.commit_command(cmd);
    }
}

impl DrawKind for Mesh {
    fn commit(b: &mut Draw2D<'_, Self>) {
        let (vert, tri_index, tint) = (b.vert.clone(), b.tri.clone(), b.tint);
        // **变换必须落地**：Mesh 命令只有 `mat_idx`（没有 transform 字段），所以这里
        // 无论如何都要写一条矩阵——`.matrix(m)` 优先，否则由 `.transform(tf)` 推导。
        //
        // ⚠ 旧实现只取 `b.model`（`None` ⇒ `InstanceData::identity()`）⇒ **`.transform(..)`
        // 对 mesh / polygon / quads 被静默忽略**：世界层里"顶点已是世界坐标"的用法看不出
        // 问题，但 `rjw_ui` 的窗口四边形是**窗口局部顶点 + 屏幕固定变换**——
        // 变换一丢，所有窗口都画在 UI 空间原点（= 屏幕左上角，"位置恒为 (0,0)"）。
        let model = b
            .model
            .unwrap_or_else(|| crate::cull::transform2d_model(&b.transform));
        let mat_idx = {
            let i = b.queue.matrices.len();
            b.queue.matrices.push(model);
            Some(i)
        };
        // 整段实例色（`quads` 的 tint）→ 已提前合批段，自成一整段不参与跨段合批；
        // 逐顶点色（`mesh` / `polygon`）→ 普通动态段，可与其他段合批。
        let cmd = match tint {
            Some(t) => DrawCommand::MeshStyled {
                vert,
                tri_index,
                mat_idx,
                color: t.into(),
            },
            None => DrawCommand::Mesh {
                vert,
                tri_index,
                mat_idx,
            },
        };
        b.commit_command(cmd);
    }
}

impl DrawKind for Custom {
    fn commit(b: &mut Draw2D<'_, Self>) {
        let idx = b.custom_idx;
        b.commit_command(DrawCommand::Custom { idx });
    }
}

impl<K: DrawKind> Drop for Draw2D<'_, K> {
    fn drop(&mut self) {
        K::commit(self);
    }
}

// ─── 流式构造 sink ───────────────────────────────────────────

/// 多边形流式构造 sink（`Render2D::polygon_with`）。
///
/// **自动三角化（fan）**：闭包结束时会以**第一个顶点为中心**生成 `n-2` 个三角形，
/// 与旧的 `add_polygon_fan` 语义完全一致。需要自定义三角化请改用
/// `mesh_with` + [`MeshSink::push_tri`]。
///
/// 顶点坐标为 **`Draw2D::transform` 的局部坐标**（默认即世界坐标）。
pub struct PolygonSink<'a> {
    verts: &'a mut Vec<VertexP3U2C4>,
    tris: &'a mut Vec<TriIndices>,
    /// 本段起始全局顶点号（三角化 / 索引重定位用）。
    base: u32,
    color: [f32; 4],
    count: u16,
}

impl PolygonSink<'_> {
    /// 追加一个顶点（UV 为 0；逐顶点色取入口默认色，可由 `.tint()` 整段覆盖）。
    #[inline]
    pub fn vertex(&mut self, pos: impl Into<Vec2>) -> u16 {
        let c = self.color;
        self.push(pos.into(), Vec2::ZERO, c)
    }

    /// 追加一个带 UV 的顶点。
    #[inline]
    pub fn vertex_uv(&mut self, pos: impl Into<Vec2>, uv: impl Into<Vec2>) -> u16 {
        let c = self.color;
        self.push(pos.into(), uv.into(), c)
    }

    /// 追加一个带 UV + 自定义逐顶点颜色的顶点（逐顶点渐变用）。
    #[inline]
    pub fn vertex_uv_color(
        &mut self,
        pos: impl Into<Vec2>,
        uv: impl Into<Vec2>,
        color: [f32; 4],
    ) -> u16 {
        self.push(pos.into(), uv.into(), color)
    }

    /// 已追加的顶点数。
    #[inline]
    pub fn len(&self) -> usize {
        self.count as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// 追加顶点，返回**局部**索引（0 起）。
    #[inline]
    fn push(&mut self, pos: Vec2, uv: Vec2, color: [f32; 4]) -> u16 {
        let local = self.verts.len() as u32 - self.base;
        debug_assert!(local < u16::MAX as u32, "polygon vertex count exceeds u16");
        self.verts.push(VertexP3U2C4 {
            pos: [pos.x, pos.y, 0.0],
            uv: [uv.x, uv.y],
            color,
        });
        self.count += 1;
        local as u16
    }

    /// 收尾：以首个顶点为中心 fan 三角化（`< 3` 顶点时不产生三角形）。
    pub(crate) fn finish(&mut self) {
        let n = self.count as usize;
        if n < 3 {
            return;
        }
        let b = self.base;
        for i in 0..(n - 2) {
            self.tris.push(TriIndices(
                Index(b as u16),
                Index((b + i as u32 + 1) as u16),
                Index((b + i as u32 + 2) as u16),
            ));
        }
    }
}

/// 四边形流式构造 sink（`Render2D::quads_with`）。
///
/// 每组四边形按 **TL, TR, BL, BR** 顺序写入，索引固定 `(0,1,3) + (3,2,0)`
/// （与 [`QUAD_TRI_INDICIES`](crate::QUAD_TRI_INDICIES) 一致）。
/// 顶点坐标为 **`Draw2D::transform` 的局部坐标**。
pub struct QuadSink<'a> {
    verts: &'a mut Vec<VertexP3U2C4>,
    tris: &'a mut Vec<TriIndices>,
    color: [f32; 4],
    quads: u32,
}

impl QuadSink<'_> {
    /// 追加一个四边形（顶点色取入口默认色；UV 为整张纹理 `[0,1]`），返回该组的**局部**组号。
    #[inline]
    pub fn quad(
        &mut self,
        tl: impl Into<Vec2>,
        tr: impl Into<Vec2>,
        bl: impl Into<Vec2>,
        br: impl Into<Vec2>,
    ) -> u16 {
        let (tl, tr, bl, br) = (tl.into(), tr.into(), bl.into(), br.into());
        let c = self.color;
        self.quad_raw([
            VertexP3U2C4 { pos: [tl.x, tl.y, 0.0], uv: [0.0, 0.0], color: c },
            VertexP3U2C4 { pos: [tr.x, tr.y, 0.0], uv: [1.0, 0.0], color: c },
            VertexP3U2C4 { pos: [bl.x, bl.y, 0.0], uv: [0.0, 1.0], color: c },
            VertexP3U2C4 { pos: [br.x, br.y, 0.0], uv: [1.0, 1.0], color: c },
        ])
    }

    /// 追加一个四边形并给出对角 UV（`uv_tl` ↔ TL，`uv_br` ↔ BR；TR/BL 线性插值）。
    #[inline]
    pub fn quad_uv(
        &mut self,
        tl: impl Into<Vec2>,
        tr: impl Into<Vec2>,
        bl: impl Into<Vec2>,
        br: impl Into<Vec2>,
        uv_tl: impl Into<Vec2>,
        uv_br: impl Into<Vec2>,
    ) -> u16 {
        let (tl, tr, bl, br) = (tl.into(), tr.into(), bl.into(), br.into());
        let (uv_tl, uv_br) = (uv_tl.into(), uv_br.into());
        let c = self.color;
        self.quad_raw([
            VertexP3U2C4 { pos: [tl.x, tl.y, 0.0], uv: uv_tl.to_array(), color: c },
            VertexP3U2C4 {
                pos: [tr.x, tr.y, 0.0],
                uv: [uv_br.x, uv_tl.y],
                color: c,
            },
            VertexP3U2C4 {
                pos: [bl.x, bl.y, 0.0],
                uv: [uv_tl.x, uv_br.y],
                color: c,
            },
            VertexP3U2C4 { pos: [br.x, br.y, 0.0], uv: uv_br.to_array(), color: c },
        ])
    }

    /// 追加一组**已构造好**的四边形顶点（顺序必须为 TL, TR, BL, BR）。
    #[inline]
    pub fn quad_raw(&mut self, q: [VertexP3U2C4; 4]) -> u16 {
        let g = self.verts.len() as u32;
        debug_assert!(g + 4 <= u16::MAX as u32, "quad vertex count exceeds u16");
        let local = (self.quads) as u16;
        self.verts.extend_from_slice(&q);
        self.tris.push(TriIndices(
            Index(g as u16),
            Index((g + 1) as u16),
            Index((g + 3) as u16),
        ));
        self.tris.push(TriIndices(
            Index((g + 3) as u16),
            Index((g + 2) as u16),
            Index(g as u16),
        ));
        self.quads += 1;
        local
    }

    /// 已追加的四边形数量。
    #[inline]
    pub fn len(&self) -> usize {
        self.quads as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.quads == 0
    }
}

// ─── 入口构造（由 `Render2D` 的入口方法调用） ─────────────────

/// 顶点默认色（未调用 `.tint()` 时的逐顶点色）。
#[inline]
fn white() -> [f32; 4] {
    Color::WHITE.into()
}

impl<'a> Draw2D<'a, Sprite> {
    /// 贴纹理精灵（`Render2D::sprite`）。
    pub(crate) fn sprite(
        queue: &'a mut DrawCommandQueue,
        mesh: &'a mut MeshStorage,
        rect: SpriteRect,
        tex_uid: u64,
    ) -> Self {
        let mut b = Self::base(queue, mesh, ColorMode::Command, Some(tex_uid));
        b.rect = rect;
        b
    }
}

impl<'a> Draw2D<'a, StaticMesh> {
    /// 静态网格实例（`Render2D::static_mesh`）。
    pub(crate) fn static_mesh(
        queue: &'a mut DrawCommandQueue,
        mesh: &'a mut MeshStorage,
        mesh_id: u64,
        tex_uid: u64,
    ) -> Self {
        let mut b = Self::base(queue, mesh, ColorMode::Command, Some(tex_uid));
        b.mesh_id = mesh_id;
        b
    }
}

impl<'a> Draw2D<'a, Custom> {
    /// 外部绘制（`Render2D::custom`）。
    pub(crate) fn custom(
        queue: &'a mut DrawCommandQueue,
        mesh: &'a mut MeshStorage,
        idx: usize,
    ) -> Self {
        let mut b = Self::base(queue, mesh, ColorMode::Command, None);
        b.custom_idx = idx;
        b
    }
}

impl<'a> Draw2D<'a, Mesh> {
    /// 收尾：从顶点/索引范围构造 Mesh Builder。
    fn mesh_from(

        queue: &'a mut DrawCommandQueue,
        mesh: &'a mut MeshStorage,
        vert: Range<usize>,
        tri: Range<usize>,
        color_mode: ColorMode,
        tex_uid: Option<u64>,
    ) -> Self {
        let mut b = Self::base(queue, mesh, color_mode, tex_uid);
        b.vert = vert;
        b.tri = tri;
        b
    }
    /// 显式顶点 + 三角形索引（`Render2D::mesh`）。
    pub(crate) fn mesh(
        queue: &'a mut DrawCommandQueue,
        mesh: &'a mut MeshStorage,
        vertices: &[Vec2],
        tri_indices: &[u16],
        tex_uid: Option<u64>,
    ) -> Self {
        assert!(
            !vertices.is_empty()
                && tri_indices.len().is_multiple_of(3)
                && tri_indices.iter().all(|&i| (i as usize) < vertices.len()),
            "mesh: vertices must be non-empty and tri_indices a valid multiple of 3"
        );
        let vs = mesh.vertices.len();
        let ts = mesh.tri_indices.len();
        let ca = white();
        for p in vertices {
            mesh.vertices.push(VertexP3U2C4 {
                pos: [p.x, p.y, 0.0],
                uv: [0.0, 0.0],
                color: ca,
            });
        }
        for c in tri_indices.chunks_exact(3) {
            mesh.tri_indices.push(TriIndices(
                Index((c[0] as u32 + vs as u32) as u16),
                Index((c[1] as u32 + vs as u32) as u16),
                Index((c[2] as u32 + vs as u32) as u16),
            ));
        }
        let (ve, te) = (mesh.vertices.len(), mesh.tri_indices.len());
        Self::mesh_from(queue, mesh, vs..ve, ts..te, ColorMode::Vertex, tex_uid)
    }

    /// 流式构造网格（`Render2D::mesh_with`）。
    pub(crate) fn mesh_with<F>(
        queue: &'a mut DrawCommandQueue,
        mesh: &'a mut MeshStorage,
        f: F,
        tex_uid: Option<u64>,
    ) -> Self
    where
        F: FnOnce(&mut MeshSink<'_>),
    {
        let vs = mesh.vertices.len();
        let ts = mesh.tri_indices.len();
        {
            let mut sink = MeshSink {
                base: vs as u32,
                verts: &mut mesh.vertices,
                tris: &mut mesh.tri_indices,
                color_arr: white(),
            };
            f(&mut sink);
        }
        let (ve, te) = (mesh.vertices.len(), mesh.tri_indices.len());
        Self::mesh_from(queue, mesh, vs..ve, ts..te, ColorMode::Vertex, tex_uid)
    }
    /// 多边形（fan 三角化：首个顶点为中心；`Render2D::polygon` / `polygon_uv`）。
    pub(crate) fn polygon(
        queue: &'a mut DrawCommandQueue,
        mesh: &'a mut MeshStorage,
        vertices: &[Vec2],
        uvs: Option<&[Vec2]>,
        tex_uid: Option<u64>,
    ) -> Self {
        debug_assert!(vertices.len() >= 3, "polygon needs at least 3 vertices");
        if let Some(uvs) = uvs {
            debug_assert_eq!(vertices.len(), uvs.len(), "polygon: uvs must match vertices");
        }
        let n = vertices.len();
        let vs = mesh.vertices.len();
        let ts = mesh.tri_indices.len();
        let ca = white();
        for (i, p) in vertices.iter().enumerate() {
            let uv = uvs.map(|u| u[i].to_array()).unwrap_or([0.0, 0.0]);
            mesh.vertices.push(VertexP3U2C4 {
                pos: [p.x, p.y, 0.0],
                uv,
                color: ca,
            });
        }
        for i in 0..n.saturating_sub(2) {
            mesh.tri_indices.push(TriIndices(
                Index(vs as u16),
                Index((vs + i + 1) as u16),
                Index((vs + i + 2) as u16),
            ));
        }
        let (ve, te) = (mesh.vertices.len(), mesh.tri_indices.len());
        Self::mesh_from(queue, mesh, vs..ve, ts..te, ColorMode::Vertex, tex_uid)
    }

    /// 流式构造多边形（闭包结束时自动 fan 三角化；`Render2D::polygon_with`）。
    pub(crate) fn polygon_with<F>(
        queue: &'a mut DrawCommandQueue,
        mesh: &'a mut MeshStorage,
        f: F,
        tex_uid: Option<u64>,
    ) -> Self
    where
        F: FnOnce(&mut PolygonSink<'_>),
    {
        let vs = mesh.vertices.len();
        let ts = mesh.tri_indices.len();
        {
            let mut sink = PolygonSink {
                verts: &mut mesh.vertices,
                tris: &mut mesh.tri_indices,
                base: vs as u32,
                color: white(),
                count: 0,
            };
            f(&mut sink);
            sink.finish();
        }
        let (ve, te) = (mesh.vertices.len(), mesh.tri_indices.len());
        Self::mesh_from(queue, mesh, vs..ve, ts..te, ColorMode::Vertex, tex_uid)
    }
    pub(crate) fn quads(
        queue: &'a mut DrawCommandQueue,
        mesh: &'a mut MeshStorage,
        vertices: &[VertexP3U2C4],
        tex_uid: u64,
    ) -> Self {
        assert!(
            vertices.len().is_multiple_of(4),
            "quads: vertex count must be a multiple of 4 (one quad = 4 vertices)"
        );
        let vs = mesh.vertices.len();
        let ts = mesh.tri_indices.len();
        mesh.vertices.extend_from_slice(vertices);
        for i in (0..vertices.len()).step_by(4) {
            let b = (vs + i) as u32;
            // Quad 标准索引：TL,TR,BL,BR → 三角形 (0,1,3) + (3,2,0)
            mesh.tri_indices.push(TriIndices(
                Index(b as u16),
                Index((b + 1) as u16),
                Index((b + 3) as u16),
            ));
            mesh.tri_indices.push(TriIndices(
                Index((b + 3) as u16),
                Index((b + 2) as u16),
                Index(b as u16),
            ));
        }
        let (ve, te) = (mesh.vertices.len(), mesh.tri_indices.len());
        Self::mesh_from(queue, mesh, vs..ve, ts..te, ColorMode::Instance, Some(tex_uid))
    }

    /// 四边形段（流式构造；`Render2D::quads_with`）。
    pub(crate) fn quads_with<F>(
        queue: &'a mut DrawCommandQueue,
        mesh: &'a mut MeshStorage,
        f: F,
        tex_uid: u64,
    ) -> Self
    where
        F: FnOnce(&mut QuadSink<'_>),
    {
        let vs = mesh.vertices.len();
        let ts = mesh.tri_indices.len();
        {
            let mut sink = QuadSink {
                verts: &mut mesh.vertices,
                tris: &mut mesh.tri_indices,
                color: white(),
                quads: 0,
            };
            f(&mut sink);
        }
        let (ve, te) = (mesh.vertices.len(), mesh.tri_indices.len());
        Self::mesh_from(queue, mesh, vs..ve, ts..te, ColorMode::Instance, Some(tex_uid))
    }
}

// ─── 单元测试（无 GPU：Builder → 命令队列） ───────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{DrawCommandQueue, Layer};
    use crate::rstates::BlendMode;
    use glam::Vec2;

    /// 建一个空队列 + 网格存储（无需 GPU）。
    fn scratch() -> (DrawCommandQueue, MeshStorage) {
        (DrawCommandQueue::default(), MeshStorage::default())
    }

    /// 取出唯一命令（含 layer / states），用于断言 Builder 的落地结果。
    fn only(q: &DrawCommandQueue) -> (&DrawCommand, Layer, Option<RStates>, Option<u64>) {
        let mut it = q.iter();
        let (cmd, layer, states) = it.next().expect("one command expected");
        assert!(it.next().is_none(), "exactly one command expected");
        (cmd, layer, states.rstates, states.texture_uid)
    }

    /// **回归**：`.transform(tf)` 必须对 **mesh / polygon / quads** 生效。
    ///
    /// 旧实现只在 `.matrix(m)` 时写 `mat_idx`，`.transform(tf)` 对 Mesh 系被**静默忽略**
    /// ⇒ 顶点按"已是世界坐标"处理。世界层用法看不出问题（顶点本来就是世界坐标），
    /// 但 `rjw_ui` 的窗口四边形是**窗口局部顶点 + 屏幕固定变换**：变换一丢，
    /// 所有窗口都画在 UI 空间原点（屏幕左上角）——"引擎里位置对、视觉恒为 (0,0)"。
    #[test]
    fn mesh_transform_is_baked_into_matrix() {
        let (mut q, mut m) = scratch();
        let quad = [VertexP3U2C4 { color: [1.0; 4], ..Default::default() }; 4];

        // quads + `.transform(..)`：命令必须带矩阵，且矩阵 == transform2d_model(tf)
        let tf = Transform2D::IDENTITY.with_pos(Vec2::new(30.0, 40.0));
        Draw2D::quads(&mut q, &mut m, &quad, 3).transform(tf);
        let (cmd, ..) = only(&q);
        let mat_idx = match cmd {
            DrawCommand::Mesh { mat_idx, .. } => mat_idx.expect("`.transform(..)` 必须落到 mat_idx"),
            other => panic!("应为 Mesh，实际 {other:?}"),
        };
        assert_eq!(
            q.matrices[mat_idx],
            crate::cull::transform2d_model(&tf),
            "矩阵应等于 transform2d_model(tf)（含平移）"
        );

        // 不设变换 ⇒ 单位矩阵（与旧行为一致：顶点即世界坐标）
        let (mut q2, mut m2) = scratch();
        Draw2D::quads(&mut q2, &mut m2, &quad, 3);
        let (cmd2, ..) = only(&q2);
        let mi2 = match cmd2 {
            DrawCommand::Mesh { mat_idx, .. } => mat_idx.expect("也应带单位矩阵"),
            other => panic!("应为 Mesh，实际 {other:?}"),
        };
        assert_eq!(q2.matrices[mi2], glam::Mat4::IDENTITY);

        // `mesh` / `polygon` 同理；`.matrix(m)` 优先于 `.transform(tf)`
        let (mut q3, mut m3) = scratch();
        let verts = [Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)];
        let manual = glam::Mat4::from_translation(glam::Vec3::new(5.0, 6.0, 0.0));
        Draw2D::mesh(&mut q3, &mut m3, &verts, &[0, 1, 2], None)
            .transform(Transform2D::IDENTITY.with_pos(Vec2::new(99.0, 99.0)))
            .matrix(manual);
        let (cmd3, ..) = only(&q3);
        let mi3 = match cmd3 {
            DrawCommand::Mesh { mat_idx, .. } => mat_idx.expect("mat_idx"),
            other => panic!("应为 Mesh，实际 {other:?}"),
        };
        assert_eq!(q3.matrices[mi3], manual, "`.matrix(m)` 优先于 `.transform(tf)`");
    }

    /// `mesh` + `.tint()` → 写**逐顶点色**（保持普通 `Mesh` 命令，可参与动态段合批）。
    #[test]
    fn mesh_color_writes_vertex_colors() {
        let (mut q, mut m) = scratch();
        let verts = [Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)];
        Draw2D::mesh(&mut q, &mut m, &verts, &[0, 1, 2], Some(9))
            .tint(Color::RED)
            .layer(2.0);

        let red: [f32; 4] = Color::RED.into();
        assert_eq!(m.vertices.len(), 3);
        assert_eq!(m.vertices[0].color, red, "顶点色应被 .tint() 写入");
        assert_eq!(m.tri_indices.len(), 1);

        let (cmd, layer, states, tex) = only(&q);
        assert!(matches!(cmd, DrawCommand::Mesh { .. }), "应为普通 Mesh（非 MeshStyled）");
        assert_eq!(layer, Layer::from(2.0));
        assert_eq!(tex, Some(9), "入口纹理 uid 应透传");
        assert_eq!(states, None, "未链式设置状态 → None（继承全局默认）");
    }

    /// `quads` + `.tint()` → **整段实例色**（`MeshStyled`），顶点自带色不被改写。
    #[test]
    fn quads_color_becomes_instance_tint() {
        let (mut q, mut m) = scratch();
        let white: [f32; 4] = Color::WHITE.into();
        let quad = [VertexP3U2C4 { color: white, ..Default::default() }; 4];
        Draw2D::quads(&mut q, &mut m, &quad, 3).tint(Color::CYAN);

        let cyan: [f32; 4] = Color::CYAN.into();
        assert_eq!(m.vertices[0].color, white, "调用者顶点色不被 .tint() 改写");
        assert_eq!(m.tri_indices.len(), 2, "一个四边形 = 2 个三角形");

        let (cmd, _, _, tex) = only(&q);
        match cmd {
            DrawCommand::MeshStyled { color, .. } => assert_eq!(*color, cyan),
            other => panic!("应转为 MeshStyled（整段 tint），实际 {other:?}"),
        }
        assert_eq!(tex, Some(3));
    }

    /// `polygon_with` → 闭包结束自动 fan 三角化（`(0,i+1,i+2)`），索引全局重定位正确。
    #[test]
    fn polygon_with_auto_fans() {
        let (mut q, mut m) = scratch();
        Draw2D::polygon_with(&mut q, &mut m, |p| {
            p.vertex(Vec2::new(0.0, 0.0));
            p.vertex(Vec2::new(10.0, 0.0));
            p.vertex(Vec2::new(10.0, 10.0));
            p.vertex(Vec2::new(0.0, 10.0));
        }, None);

        assert_eq!(m.vertices.len(), 4);
        assert_eq!(m.tri_indices.len(), 2, "4 顶点 → 2 个 fan 三角形");
        let t0 = m.tri_indices[0];
        let t1 = m.tri_indices[1];
        assert_eq!((t0.0.0, t0.1.0, t0.2.0), (0, 1, 2));
        assert_eq!((t1.0.0, t1.1.0, t1.2.0), (0, 2, 3));
    }

    /// `.matrix(mat)` → 走 `*Matrix` 变体（并把矩阵写入队列的 matrices 池）。
    #[test]
    fn matrix_selects_matrix_variant() {
        let (mut q, mut m) = scratch();
        Draw2D::sprite(&mut q, &mut m, SpriteRect::default(), 1)
            .matrix(glam::Mat4::from_translation(glam::Vec3::new(5.0, 6.0, 0.0)));
        let (cmd, _, _, _) = only(&q);
        assert!(matches!(cmd, DrawCommand::Sprite2DMatrix { mat_idx: 0, .. }));
        assert_eq!(q.matrices.len(), 1);
    }

    /// `custom` + 链式 layer / states。
    #[test]
    fn custom_records_layer_and_states() {
        let (mut q, mut m) = scratch();
        Draw2D::custom(&mut q, &mut m, 7)
            .layer(4.0)
            .blend(BlendMode::Additive)
            .depth(true);
        let (cmd, layer, states, tex) = only(&q);
        assert!(matches!(cmd, DrawCommand::Custom { idx: 7 }));
        assert_eq!(layer, Layer::from(4.0));
        assert_eq!(tex, None);
        let r = states.expect("状态应被显式设置");
        assert!(r.to_blend().is_some(), "Additive 混合应生效");
        assert!(r.to_depth_stencil().is_some(), "深度测试应生效");
    }

    /// `solid` 走白纹理；`static_mesh` 的纹理随入口参数。
    #[test]
    fn entry_texture_routing() {
        let (mut q, mut m) = scratch();
        Draw2D::sprite(&mut q, &mut m, SpriteRect::default(), 42);
        let (_, _, _, tex) = only(&q);
        assert_eq!(tex, Some(42));
    }
}

