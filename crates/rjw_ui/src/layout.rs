//! 容器布局：`Frame`（pack / grid 几何管理器）+ 子项放置与尺寸结算（纯函数，可单测）。
//!
//! 模型（DOM 风格自动尺寸）：
//! - 叶子控件由内容测量自然撑开，调用 `child_rect` 占据容器内一个位置并推进光标；
//! - 容器 `settle_size` 按已放置子项结算自身尺寸（pack 取最大宽，grid 取单元格）；
//! - 控件绘制坐标一律为**相对容器 origin 的局部坐标**，容器弹出时由 `Ui` 统一平移。
//!
//! 坐标系：左上角原点，Y+ 向下（与屏幕/相机一致）。

use glam::Vec2;
use rjw_transform::Rect;

/// pack 堆叠方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackSide {
    /// 垂直堆叠（自上而下），子项左对齐；宽度 = 最大子项宽。
    Top,
    /// 水平堆叠（自左而右），子项顶对齐；高度 = 最大子项高。
    Left,
}

/// 容器布局种类。
#[derive(Clone, Copy, Debug)]
pub(crate) enum FrameKind {
    /// pack / panel 堆叠布局。
    Stack { side: PackSide, gap: f32 },
    /// grid 网格布局：`cols` 列，单元格尺寸 `cell`（跨帧缓存，见 `UiState::grid_cells`）。
    Grid { cols: usize, cell: Vec2 },
}

/// 容器布局帧（`Ui` 内部维护一个栈）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    pub kind: FrameKind,
    /// 内容区内边距 + 边框合计（panel；pack/grid 为 0）。
    pub pad_total: f32,
    /// 局部光标（Stack：下一个子项左上角；相对容器 origin）。
    pub cursor: Vec2,
    /// 已放置子项的最大尺寸（Stack：max 宽/高；Grid：max 子尺寸）。
    pub max_child: Vec2,
    /// 已放置子项数量（Grid 用）。
    pub count: usize,
    /// **下一子项的尺寸约束**（min / max；0 = 该轴不约束）。
    /// 一次性：`child_rect` 消耗并清零（[`Self::set_next_constraint`] 设置）。
    next_min: Vec2,
    next_max: Vec2,
    /// **下一子项的强制高度**（flex 权重分配；`None` = 自然测量）。
    /// 一次性：`child_rect` 消耗（[`Self::force_next_h`] 设置）。
    next_fixed_h: Option<f32>,
    /// **容器固定高度**（flex_at 等；覆盖 `settle_size` 的自然高度）。
    fixed_h: Option<f32>,
}

impl Frame {
    pub fn new_stack(side: PackSide, gap: f32, pad_total: f32) -> Self {
        let p = pad_total;
        Self {
            kind: FrameKind::Stack { side, gap },
            pad_total,
            cursor: Vec2::new(p, p),
            max_child: Vec2::ZERO,
            count: 0,
            next_min: Vec2::ZERO,
            next_max: Vec2::ZERO,
            next_fixed_h: None,
            fixed_h: None,
        }
    }

    pub fn new_grid(cols: usize, cell: Vec2, pad_total: f32) -> Self {
        let p = pad_total;
        Self {
            kind: FrameKind::Grid { cols, cell },
            pad_total,
            cursor: Vec2::new(p, p),
            max_child: Vec2::ZERO,
            count: 0,
            next_min: Vec2::ZERO,
            next_max: Vec2::ZERO,
            next_fixed_h: None,
            fixed_h: None,
        }
    }

    /// 设置**下一子项**的最小尺寸约束（`0` = 该轴不约束）。一次性，`child_rect` 消耗。
    /// 多次调用取各轴最大值。
    pub fn set_next_min(&mut self, min: Vec2) {
        self.next_min = Vec2::new(self.next_min.x.max(min.x), self.next_min.y.max(min.y));
    }

    /// 设置**下一子项**的最大尺寸约束（`0` = 该轴不约束）。一次性，`child_rect` 消耗。
    /// 多次调用取各轴最小值（0 表示不约束，取非零较小值）。
    pub fn set_next_max(&mut self, max: Vec2) {
        let merge = |cur: f32, v: f32| {
            if cur <= 0.0 { v } else if v <= 0.0 { cur } else { cur.min(v) }
        };
        self.next_max = Vec2::new(merge(self.next_max.x, max.x), merge(self.next_max.y, max.y));
    }

    /// 强制**下一子项**高度（flex 权重分配）。一次性，`child_rect` 消耗。
    pub fn force_next_h(&mut self, h: f32) {
        self.next_fixed_h = Some(h.max(0.0));
    }

    /// 固定容器结算高度（`settle_size` 覆盖自然高度）。
    pub fn set_fixed_h(&mut self, h: f32) {
        self.fixed_h = Some(h.max(0.0));
    }

    /// 为尺寸 `(w, h)` 的子项分配一个局部矩形，并推进光标 / 更新统计。
    ///
    /// 应用顺序：**min/max 约束** → **flex 强制高度**（覆盖测量值）。
    pub fn child_rect(&mut self, w: f32, h: f32) -> Rect {
        let w = w.max(0.0);
        let h = h.max(0.0);
        let w = if self.next_max.x > 0.0 { w.min(self.next_max.x).max(self.next_min.x) } else { w.max(self.next_min.x) };
        let h = if self.next_max.y > 0.0 { h.min(self.next_max.y).max(self.next_min.y) } else { h.max(self.next_min.y) };
        let h = self.next_fixed_h.take().unwrap_or(h);
        self.next_min = Vec2::ZERO;
        self.next_max = Vec2::ZERO;
        match &mut self.kind {
            FrameKind::Stack { side, gap } => {
                let local = self.cursor;
                match side {
                    PackSide::Top => {
                        self.cursor.y += h + *gap;
                        self.max_child.x = self.max_child.x.max(w);
                        self.max_child.y = self.max_child.y.max(h);
                    }
                    PackSide::Left => {
                        self.cursor.x += w + *gap;
                        self.max_child.x = self.max_child.x.max(w);
                        self.max_child.y = self.max_child.y.max(h);
                    }
                }
                self.count += 1;
                Rect::new(local.x, local.y, w, h)
            }
            FrameKind::Grid { cols, cell } => {
                // 渐进扩展 cell：容纳当前子项（缓存值不足时本帧就地扩大，位置即时一致）。
                if w > cell.x {
                    cell.x = w;
                }
                if h > cell.y {
                    cell.y = h;
                }
                let col = self.count % *cols;
                let row = self.count / *cols;
                self.count += 1;
                self.max_child.x = self.max_child.x.max(w);
                self.max_child.y = self.max_child.y.max(h);
                Rect::new(
                    self.pad_total + col as f32 * cell.x,
                    self.pad_total + row as f32 * cell.y,
                    w,
                    h,
                )
            }
        }
    }

    /// 结算容器自然尺寸（含 pad_total 外扩；相对容器 origin）。
    pub fn settle_size(&self) -> Vec2 {
        match &self.kind {
            FrameKind::Stack { side, gap } => {
                if self.count == 0 {
                    let p = self.pad_total * 2.0;
                    return Vec2::new(p, p);
                }
                match side {
                    PackSide::Top => {
                        let w = self.max_child.x + self.pad_total * 2.0;
                        let h = self.fixed_h.unwrap_or((self.cursor.y - gap).max(0.0) + self.pad_total);
                        Vec2::new(w, h)
                    }
                    PackSide::Left => Vec2::new(
                        (self.cursor.x - gap).max(0.0) + self.pad_total,
                        self.max_child.y + self.pad_total * 2.0,
                    ),
                }
            }
            FrameKind::Grid { cols, cell } => {
                if self.count == 0 {
                    let p = self.pad_total * 2.0;
                    return Vec2::new(p, p);
                }
                let rows = self.count.div_ceil(*cols);
                Vec2::new(
                    (*cols).max(1) as f32 * cell.x + self.pad_total * 2.0,
                    rows as f32 * cell.y + self.pad_total * 2.0,
                )
            }
        }
    }
}

/// 文本内容自然尺寸：宽 = max(行宽)，高 = 行高之和（无字形时 `ZERO`）。
/// 纯辅助：仅做布局，不依赖渲染器。
pub fn text_natural(w: f32, h: f32, padding: Vec2) -> Vec2 {
    Vec2::new(w + padding.x * 2.0, h + padding.y * 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_top_advances_y_and_tracks_max_w() {
        let mut f = Frame::new_stack(PackSide::Top, 6.0, 0.0);
        let a = f.child_rect(100.0, 20.0);
        assert_eq!(a, Rect::new(0.0, 0.0, 100.0, 20.0));
        let b = f.child_rect(80.0, 30.0);
        assert_eq!(b, Rect::new(0.0, 26.0, 80.0, 30.0), "y 应 +20+gap6");
        assert_eq!(f.settle_size(), Vec2::new(100.0, 56.0), "宽=max(100,80)，高=26+30");
    }

    #[test]
    fn stack_left_advances_x_and_tracks_max_h() {
        let mut f = Frame::new_stack(PackSide::Left, 4.0, 0.0);
        let a = f.child_rect(30.0, 50.0);
        assert_eq!(a, Rect::new(0.0, 0.0, 30.0, 50.0));
        let b = f.child_rect(40.0, 60.0);
        assert_eq!(b, Rect::new(34.0, 0.0, 40.0, 60.0), "x 应 +30+gap4");
        assert_eq!(f.settle_size(), Vec2::new(74.0, 60.0), "宽=34+40，高=max(50,60)");
    }

    #[test]
    fn panel_pad_total_expands_size() {
        // panel：pad_total=4（border+padding），子项从 (4,4) 开始
        let mut f = Frame::new_stack(PackSide::Top, 6.0, 4.0);
        f.child_rect(50.0, 10.0);
        f.child_rect(40.0, 20.0);
        let size = f.settle_size();
        assert_eq!(size, Vec2::new(50.0 + 8.0, 4.0 + 10.0 + 6.0 + 20.0 + 4.0), "宽=内容+2*pad，高=pad+子高和+gap+pad");
        let _ = size;
    }

    #[test]
    fn grid_places_by_cell_and_settles() {
        let mut f = Frame::new_grid(2, Vec2::new(30.0, 20.0), 0.0);
        assert_eq!(f.child_rect(25.0, 18.0), Rect::new(0.0, 0.0, 25.0, 18.0));
        assert_eq!(f.child_rect(28.0, 15.0), Rect::new(30.0, 0.0, 28.0, 15.0));
        assert_eq!(f.child_rect(20.0, 16.0), Rect::new(0.0, 20.0, 20.0, 16.0), "第二行");
        // cell 由调用方在闭包结束后用 max_child 更新；此处验证结算用当前 cell
        assert_eq!(f.settle_size(), Vec2::new(60.0, 40.0), "2列×30 × 2行×20");
        // max_child 记录了最大子尺寸（供调用方回写缓存）
        assert_eq!(f.max_child, Vec2::new(28.0, 18.0));
    }

    #[test]
    fn grid_pad_total_offsets_cells() {
        let mut f = Frame::new_grid(2, Vec2::new(30.0, 20.0), 5.0);
        assert_eq!(f.child_rect(10.0, 10.0), Rect::new(5.0, 5.0, 10.0, 10.0));
        assert_eq!(f.child_rect(10.0, 10.0), Rect::new(35.0, 5.0, 10.0, 10.0));
        assert_eq!(f.settle_size(), Vec2::new(60.0 + 10.0, 20.0 + 10.0));
    }

    #[test]
    fn empty_containers_settle_to_padding() {
        let s = Frame::new_stack(PackSide::Top, 6.0, 0.0).settle_size();
        assert_eq!(s, Vec2::ZERO);
        let p = Frame::new_stack(PackSide::Top, 6.0, 4.0).settle_size();
        assert_eq!(p, Vec2::new(8.0, 8.0));
        let g = Frame::new_grid(3, Vec2::splat(10.0), 0.0).settle_size();
        assert_eq!(g, Vec2::ZERO);
    }

    #[test]
    fn text_natural_adds_padding() {
        assert_eq!(text_natural(100.0, 20.0, Vec2::new(10.0, 5.0)), Vec2::new(120.0, 30.0));
    }

    #[test]
    fn min_max_constraint_clamps_child() {
        let mut f = Frame::new_stack(PackSide::Top, 6.0, 0.0);
        // 无约束：自然尺寸
        assert_eq!(f.child_rect(40.0, 20.0), Rect::new(0.0, 0.0, 40.0, 20.0));
        // min 约束：宽 < 100 → 抬到 100；高 < 30 → 抬到 30
        f.set_next_min(Vec2::new(100.0, 30.0));
        assert_eq!(f.child_rect(40.0, 20.0), Rect::new(0.0, 26.0, 100.0, 30.0));
        // max 约束：宽 > 80 → 压到 80；高 0 表示不约束
        f.set_next_max(Vec2::new(80.0, 0.0));
        assert_eq!(f.child_rect(120.0, 50.0), Rect::new(0.0, 62.0, 80.0, 50.0));
        // 约束一次性消耗：下一个子项恢复自然
        assert_eq!(f.child_rect(30.0, 10.0), Rect::new(0.0, 118.0, 30.0, 10.0));
    }

    #[test]
    fn force_next_h_overrides_measured_height() {
        let mut f = Frame::new_stack(PackSide::Top, 6.0, 0.0);
        f.force_next_h(60.0);
        assert_eq!(f.child_rect(50.0, 20.0), Rect::new(0.0, 0.0, 50.0, 60.0), "高度被强制为 60");
        assert_eq!(f.child_rect(50.0, 20.0), Rect::new(0.0, 66.0, 50.0, 20.0), "一次性，后续恢复自然");
        assert_eq!(f.settle_size(), Vec2::new(50.0, 86.0));
    }

    #[test]
    fn fixed_h_overrides_settle_height() {
        let mut f = Frame::new_stack(PackSide::Top, 6.0, 0.0);
        f.child_rect(50.0, 20.0);
        f.child_rect(40.0, 10.0);
        f.set_fixed_h(200.0);
        assert_eq!(f.settle_size(), Vec2::new(50.0, 200.0), "固定高覆盖自然结算");
    }
}
