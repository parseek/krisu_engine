//! 绘制命令 / 排序：命令枚举、层级、状态与命令队列。

use std::ops::Range;

use rjw_transform::{Rect, Transform2D};

use crate::{cull, data::SpriteRect, rstates::RStates, sort::SortKey};

// ─── 绘制命令 / 排序 ──────────────────────────────────────────

#[derive(Debug)]
pub(crate) enum DrawCommand {
    Sprite2D {
        rect: SpriteRect,
        color: rjw_color::Color,
        transform: Transform2D,
    },
    /// 高级 Sprite：跳过 Transform2D → Mat4 的自动推导，直接传入列主序模型矩阵。
    /// `mat_idx` 指向 `DrawCommandQueue.matrices` 中的条目。
    Sprite2DMatrix {
        rect: SpriteRect,
        color: rjw_color::Color,
        mat_idx: usize,
    },
    Mesh {
        /// 该命令的顶点在 `MeshStorage.vertices` 中的范围（录制时）
        vert: Range<usize>,
        /// 该命令的三角形索引在 `MeshStorage.tri_indices` 中的范围（全局索引）
        tri_index: Range<usize>,
        /// 可选变换（`DrawCommandQueue.matrices` 索引）：顶点为**局部坐标**，
        /// 经 model 变换到世界（`None` = 顶点即世界坐标，原语义）。
        mat_idx: Option<usize>,
    },
    /// **已提前合批的四边形段**（QuadVerticesCommand）：一整段 QuadVertices +
    /// 单一变换矩阵 + 单一**混合颜色**（实例 color，shader 里 顶点色×实例色）。
    /// 语义 = 自成一整段一次 `draw_indexed`，**不参与**通用 Mesh 的跨段合批比较
    /// （避免 color 参与分组）。供 UI 窗口整段提交（整窗口动画/特效）。
    MeshStyled {
        vert: Range<usize>,
        tri_index: Range<usize>,
        mat_idx: Option<usize>,
        /// 整段混合色（实例 color；`[1,1,1,1]` = 不染色）。
        color: [f32; 4],
    },
    /// 静态网格（注册表）：`mesh_id` → `MESHES` 中的 `Arc<MeshData>`，实例化合并绘制。
    /// 顶点自带 UV，通过 `States.texture_uid` 采样纹理。
    StaticMesh {
        mesh_id: u64,
        color: rjw_color::Color,
        transform: Transform2D,
    },
    /// 高级静态网格：直接传入列主序模型矩阵（`mat_idx` 指向 `DrawCommandQueue.matrices`）。
    StaticMeshMatrix {
        mesh_id: u64,
        color: rjw_color::Color,
        mat_idx: usize,
    },
    /// 外部绘制调用标记（不含数据，实际闭包由 `Render2D::buf_custom_draws` 管理）。
    /// `idx` 指向 `Render2D::buf_custom_draws` 中的条目（与 `Sprite2DMatrix.mat_idx` 同理，
    /// 随命令参与排序，保证排序后仍能正确关联到对应闭包）。
    Custom { idx: usize },
}

/// **绘制命令的整数像素裁剪键**（比较 / 排序用；`Rect` 是 f32，不能直接做 `Ord` 键）。
///
/// 取整规则：**四舍五入**到最近像素（最终 `set_scissor_rect` 也按像素取整；
/// 顶点本身已是物理像素整数，所以不存在"少裁一条边"的问题）。
#[inline]
pub(crate) fn clip_px(rect: Rect) -> (i32, i32, i32, i32) {
    let r = rect.normalized();
    (
        r.x.round() as i32,
        r.y.round() as i32,
        r.w.round() as i32,
        r.h.round() as i32,
    )
}

/// 裁剪矩形是否为空（`w/h <= 0`，或含 NaN/∞）——空 ⇒ 该 draw **整条跳过**。
#[inline]
pub(crate) fn clip_is_empty(rect: Rect) -> bool {
    let r = rect.normalized();
    let has_area = r.w > 0.0 && r.h > 0.0;
    let finite = r.x.is_finite() && r.y.is_finite() && r.w.is_finite() && r.h.is_finite();
    !(has_area && finite)
}

/// 层级：数值越小越先绘制（越靠后）
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Layer(ordered_float::OrderedFloat<f64>);

impl From<f64> for Layer {
    fn from(value: f64) -> Self {
        Self(value.into())
    }
}

impl From<f32> for Layer {
    fn from(value: f32) -> Self {
        Self((value as f64).into())
    }
}

impl From<i32> for Layer {
    fn from(value: i32) -> Self {
        Self((value as f64).into())
    }
}

impl From<u32> for Layer {
    fn from(value: u32) -> Self {
        Self((value as f64).into())
    }
}

impl From<i64> for Layer {
    fn from(value: i64) -> Self {
        Self((value as f64).into())
    }
}

impl Layer {
    /// 获取层级数值（f64）。
    #[inline]
    pub fn as_f64(&self) -> f64 {
        self.0.into_inner()
    }
}

/// 渲染状态（Pipeline + 绑定组 + 命令级 scissor），不拥有所有权。
/// 相邻相同状态（含 scissor）才合批。
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(crate) struct States {
    pub(crate) rstates: Option<RStates>,
    pub(crate) texture_uid: Option<u64>,
    /// **命令级 scissor**（屏幕 / 目标像素、左上原点；[`crate::Draw2D::scissor`]）；
    /// `None` = 继承画面级（[`Render2D::scissor`](crate::Render2D::scissor)）。
    ///
    /// 只参与"相邻同状态才合批"的比较（**不参与排序**：公开的
    /// [`SortKey`](crate::SortKey) 里没有它，加字段会破坏外部结构体字面量）。
    pub(crate) scissor: Option<Rect>,
}

/// 绘制命令队列：命令 + 层级 + 状态（三条并行数组 + 索引数组）。
///
/// **职责边界**：本类型只负责"存取"，不含排序 / 剔除策略——
/// 排序见 [`crate::sort`]、剔除见 [`crate::cull`]，二者都只操作
/// [`Self::take_order`] 取出的索引数组。
#[derive(Debug, Default)]
pub(crate) struct DrawCommandQueue {
    commands: Vec<DrawCommand>,
    layers: Vec<Layer>,
    states: Vec<States>,
    /// 绘制顺序（索引数组）：元素为 `commands` 的下标。排序 / 剔除都作用于它。
    order: Vec<usize>,

    /// 高级 Sprite2D 的模型矩阵池（`DrawCommand::Sprite2DMatrix.mat_idx` 指向此处）。
    pub(crate) matrices: Vec<glam::Mat4>,
}

impl DrawCommandQueue {
    #[inline]
    fn check_vaild(&self) {
        debug_assert_eq!(self.commands.len(), self.layers.len());
        debug_assert_eq!(self.states.len(), self.layers.len());
        debug_assert_eq!(self.states.len(), self.order.len());
    }

    pub(crate) fn push(&mut self, command: DrawCommand, layer: Layer, states: States) {
        self.order.push(self.commands.len());
        self.commands.push(command);
        self.layers.push(layer);
        self.states.push(states);
        self.check_vaild();
    }

    pub(crate) fn clear(&mut self) {
        self.order.clear();
        self.commands.clear();
        self.layers.clear();
        self.states.clear();
        self.matrices.clear();
    }

    // ── 索引数组存取（排序 / 剔除的输入输出） ──────────────────

    /// 索引数组是否为空（未被录制任何命令）。
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// 本批命令是否需要深度 / 模板附件（任一命令声明 depth / stencil 状态）。
    ///
    /// 附件是 **pass 级**的：渲染器在开 pass 之前用它决定 `need_depth_stencil`。
    pub(crate) fn requires_depth_stencil(&self) -> bool {
        self.states
            .iter()
            .any(|s| s.rstates.is_some_and(|r| r.uses_depth_stencil()))
    }

    /// 取出索引数组（所有权转移，便于在此期间继续借用队列自身）。
    #[inline]
    pub(crate) fn take_order(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.order)
    }

    /// 回填索引数组。
    #[inline]
    pub(crate) fn set_order(&mut self, order: Vec<usize>) {
        self.order = order;
    }

    /// 生成与**命令下标**对齐的排序键（写进常驻缓冲，零堆分配）。
    pub(crate) fn fill_sort_keys(&self, out: &mut Vec<SortKey>) {
        self.check_vaild();
        out.clear();
        out.reserve(self.order.len());
        for i in 0..self.commands.len() {
            let s = &self.states[i];
            out.push(SortKey {
                layer: self.layers[i],
                rstates: s.rstates,
                texture_uid: s.texture_uid,
            });
        }
    }

    /// 命令 `i` 的世界 AABB（供剔除）；`None` = 该命令不参与剔除。
    ///
    /// 只有 Sprite 命令有确定的 AABB（动态 Mesh / StaticMesh / Custom 恒保留），
    /// 与原"仅 Sprite 被剔除"的行为一致。
    pub(crate) fn cull_aabb(&self, i: usize) -> Option<Rect> {
        match &self.commands[i] {
            DrawCommand::Sprite2D { rect, transform, .. } => Some(cull::sprite_world_aabb(
                rect,
                &cull::transform2d_model(transform),
            )),
            DrawCommand::Sprite2DMatrix { rect, mat_idx, .. } => {
                Some(cull::sprite_world_aabb(rect, &self.matrices[*mat_idx]))
            }
            _ => None,
        }
    }

    /// 按绘制顺序迭代命令（`iter` 前应已应用排序 / 剔除结果）。
    #[inline]
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&DrawCommand, Layer, &States)> {
        self.order
            .iter()
            .map(|&i| (&self.commands[i], self.layers[i], &self.states[i]))
    }
}

// ─── 单元测试（无 GPU 依赖） ──────────────────────────────────

#[cfg(test)]
mod queue_tests {
    use super::*;

    fn push_sprite(q: &mut DrawCommandQueue, rstates: Option<RStates>) {
        q.push(
            DrawCommand::Sprite2D {
                rect: SpriteRect::new((0.0, 0.0), (1.0, 1.0)),
                color: rjw_color::Color::WHITE,
                transform: Transform2D::IDENTITY,
            },
            Layer::from(0.0),
            States { rstates, texture_uid: None, scissor: None },
        );
    }

    /// **命令级 scissor：整数像素键**（比较用；`Rect` 是 f32 不能直接做排序键）。
    ///
    /// 空矩形（`w/h <= 0`，含 NaN）⇒ 调用方据此**跳过该 draw**（而不是退化成"不裁剪"）。
    #[test]
    fn scissor_round_out_expands() {
        assert_eq!(clip_px(Rect::new(1.2, 2.7, 10.4, 5.1)), (1, 3, 10, 5), "四舍五入到像素");
        // 负宽高先归一化
        assert_eq!(clip_px(Rect::new(10.0, 10.0, -4.0, -3.0)), (6, 7, 4, 3));
        // 空 / 非有限 ⇒ 空（该 draw 整条跳过）
        assert!(clip_is_empty(Rect::new(0.0, 0.0, 0.0, 10.0)));
        assert!(clip_is_empty(Rect::new(f32::NAN, 0.0, 10.0, 10.0)));
    }

    /// 附件需求由**命令状态**驱动：任一命令声明 depth / stencil ⇒ 需要深度附件。
    #[test]
    fn requires_depth_stencil_is_state_driven() {
        let mut q = DrawCommandQueue::default();
        assert!(!q.requires_depth_stencil(), "空队列不需要附件");

        push_sprite(&mut q, None);
        assert!(!q.requires_depth_stencil(), "无状态（继承全局默认）不需要");

        push_sprite(&mut q, Some(RStates::default()));
        assert!(!q.requires_depth_stencil(), "默认状态不声明深度");

        push_sprite(&mut q, Some(RStates::new().depth_test(true)));
        assert!(q.requires_depth_stencil(), "出现 depth_test ⇒ 需要附件");
    }

    #[test]
    fn stencil_state_also_requires_attachment() {
        let mut q = DrawCommandQueue::default();
        push_sprite(&mut q, Some(RStates::new().stencil_test(true)));
        assert!(q.requires_depth_stencil());
    }
}
