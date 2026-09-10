//! 排序：对**命令索引数组**重排的策略。
//!
//! 设计要点（职责分离）：
//! - 排序**只操作索引数组** `&mut [usize]`（元素 = 命令在队列中的下标）——命令数据本身不动；
//! - 本模块**不依赖 GPU**，纯函数可单测；
//! - [`Render2D`](crate::Render2D) 只持有策略（[`SortMode`] 或自定义 [`SortPolicy`]）并调用它，
//!   自身不含任何排序逻辑。
//!
//! ```no_run
//! use rjw_2d_render::{SortMode, SortPolicy, SortKey};
//! # let (mut order, keys): (Vec<usize>, Vec<SortKey>) = (vec![0, 1], vec![]);
//! SortMode::LayerAndStates.sort(&mut order, &keys);
//! ```

use crate::command::Layer;
use crate::rstates::RStates;

// ─── 排序键 ───────────────────────────────────────────────────

/// 与命令一一对应的排序键（`DrawCommandQueue` 的**公开投影**）。
///
/// 与内部 `States` 字段一一对应，用于让自定义 [`SortPolicy`] 在不接触内部类型的前提下
/// 复现引擎的排序语义（`layer` 为主，其次 `rstates`，再次 `texture_uid`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortKey {
    /// 绘制层级（数值小先绘制）。
    pub layer: Layer,
    /// 该命令显式设置的渲染状态（`None` = 继承渲染器全局默认）。
    pub rstates: Option<RStates>,
    /// 该命令采样的纹理 uid（`None` = 白纹理）。
    pub texture_uid: Option<u64>,
}

// ─── 排序模式 ─────────────────────────────────────────────────

/// 命令排序模式（引擎内置策略，[`Render2D::set_sort_mode`](crate::Render2D::set_sort_mode)）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    /// 按 `(layer, states)` 排序后合批绘制（引擎默认）。
    #[default]
    LayerAndStates,
    /// **仅按 layer 稳定排序**：同 layer 内保持录制顺序（`states` 只参与相邻合批）。
    /// 适合"每图层一个 layer、图层内部顺序由录制序保证"的场景（如 UI 窗口）。
    LayerOnly,
    /// 不排序：按录制顺序绘制（相邻同状态仍合批）。
    None,
}

// ─── 策略 trait ───────────────────────────────────────────────

/// 排序策略：对索引数组就地排序。
///
/// 引擎默认实现是 [`SortMode`]；需要自定义排序键（如 UI 的
/// `(窗口, 深度, 元素序, 图形/文字组, 录制序)`）时实现此 trait 并交给
/// [`Render2D::set_sorter`](crate::Render2D::set_sorter)。
pub trait SortPolicy: Send + Sync {
    /// `order`：命令索引数组（元素为命令下标）；`keys`：与**命令下标**对齐的排序键
    /// （即 `keys[order[i]]` 才是第 `i` 条命令的键）。
    fn sort(&self, order: &mut [usize], keys: &[SortKey]);
}

impl SortPolicy for SortMode {
    fn sort(&self, order: &mut [usize], keys: &[SortKey]) {
        match self {
            // `sort_by` 为稳定排序：键相同时保持录制顺序（与原实现一致）。
            SortMode::LayerAndStates => order.sort_by(|&a, &b| {
                keys[a]
                    .layer
                    .cmp(&keys[b].layer)
                    .then(keys[a].rstates.cmp(&keys[b].rstates))
                    .then(keys[a].texture_uid.cmp(&keys[b].texture_uid))
            }),
            SortMode::LayerOnly => order.sort_by(|&a, &b| keys[a].layer.cmp(&keys[b].layer)),
            SortMode::None => {}
        }
    }
}

// ─── 单元测试 ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn key(layer: f64, rstates: Option<RStates>, tex: Option<u64>) -> SortKey {
        SortKey {
            layer: layer.into(),
            rstates,
            texture_uid: tex,
        }
    }

    /// `LayerAndStates`：先 layer，后 rstates，再 texture_uid；同键保持录制顺序（稳定）。
    #[test]
    fn layer_and_states_orders_by_layer_then_states() {
        let keys = vec![
            key(1.0, Some(RStates::new().blend(crate::BlendMode::Multiply)), None), // 0
            key(0.0, Some(RStates::new()), Some(7)),                                // 1
            key(0.0, Some(RStates::new()), Some(3)),                                // 2
            key(0.0, None, None),                                                   // 3
        ];
        let mut order = vec![0, 1, 2, 3];
        SortMode::LayerAndStates.sort(&mut order, &keys);
        // layer 0 组：rstates None(3) < Some(1/2)；Some 内部按 texture_uid 3 < 7。
        assert_eq!(order, vec![3, 2, 1, 0]);
    }

    /// `LayerOnly`：仅 layer 有序，同 layer 内保持录制顺序。
    #[test]
    fn layer_only_is_stable_within_layer() {
        let keys = vec![
            key(2.0, None, Some(9)),
            key(1.0, None, Some(5)),
            key(1.0, None, Some(1)),
        ];
        let mut order = vec![0, 1, 2];
        SortMode::LayerOnly.sort(&mut order, &keys);
        assert_eq!(order, vec![1, 2, 0], "同 layer(1.0) 内应保持 1→2 的录制顺序");
    }

    /// `None`：完全不动（按录制顺序）。
    #[test]
    fn none_keeps_recording_order() {
        let keys = vec![key(5.0, None, None), key(0.0, None, None)];
        let mut order = vec![0, 1];
        SortMode::None.sort(&mut order, &keys);
        assert_eq!(order, vec![0, 1]);
    }
}
