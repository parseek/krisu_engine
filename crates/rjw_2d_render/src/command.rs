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
    Mesh {
        /// 该命令的顶点在 `MeshStorage.vertices` 中的范围（录制时）
        vert: Range<usize>,
        /// 该命令的三角形索引在 `MeshStorage.tri_indices` 中的范围（全局索引）
        tri_index: Range<usize>,
    },
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
            self.layers[a].cmp(&self.layers[b]).then(self.states[a].cmp(&self.states[b]))
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