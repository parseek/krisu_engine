//! 绘制命令 / 排序：命令枚举、层级、状态与命令队列。

use std::ops::Range;

use rjw_transform::Transform2D;

use crate::{data::SpriteRect, rstates::RStates};

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

impl Layer {
    /// 获取层级数值（f64）。
    #[inline]
    pub fn as_f64(&self) -> f64 {
        self.0.into_inner()
    }
}

/// 渲染状态（Pipeline + 绑定组），不拥有所有权。
/// 实现排序 trait，相邻相同状态可合批。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct States {
    pub(crate) rstates: Option<RStates>,
    pub(crate) texture_uid: Option<u64>,
}

/// 绘制命令队列：命令 + 层级 + 状态，支持排序合批
#[derive(Debug, Default)]
pub(crate) struct DrawCommandQueue {
    commands: Vec<DrawCommand>,
    layers: Vec<Layer>,
    states: Vec<States>,
    cmd_indicies: Vec<usize>,

    /// 高级 Sprite2D 的模型矩阵池（`DrawCommand::Sprite2DMatrix.mat_idx` 指向此处）。
    pub(crate) matrices: Vec<glam::Mat4>,

    dirty: bool,
}

impl DrawCommandQueue {
    #[inline]
    fn check_vaild(&self) {
        debug_assert_eq!(self.commands.len(), self.layers.len());
        debug_assert_eq!(self.states.len(), self.layers.len());
        debug_assert_eq!(self.states.len(), self.cmd_indicies.len());
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.check_vaild();
        self.cmd_indicies.len()
    }

    pub(crate) fn push(&mut self, command: DrawCommand, layer: Layer, states: States) {
        self.cmd_indicies.push(self.len());
        self.commands.push(command);
        self.layers.push(layer);
        self.states.push(states);
        self.dirty = true;
        self.check_vaild();
    }

    pub(crate) fn clear(&mut self) {
        self.cmd_indicies.clear();
        self.commands.clear();
        self.layers.clear();
        self.states.clear();
        self.matrices.clear();
        self.dirty = false;
    }

    #[allow(unused)]
    pub(crate) fn sort_layer(&mut self) {
        if !self.dirty {
            return;
        }
        self.cmd_indicies.sort_by_key(|&i| self.layers[i]);
        self.dirty = false;
    }

    pub(crate) fn sort_layer_then_states(&mut self) {
        if !self.dirty {
            return;
        }
        self.cmd_indicies.sort_by(|&a, &b| {
            self.layers[a]
                .cmp(&self.layers[b])
                .then(self.states[a].cmp(&self.states[b]))
        });
        self.dirty = false;
    }

    #[inline]
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&DrawCommand, Layer, &States)> {
        self.cmd_indicies
            .iter()
            .map(|&i| (&self.commands[i], self.layers[i], &self.states[i]))
    }
}
