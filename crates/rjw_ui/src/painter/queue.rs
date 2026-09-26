//! **绘制队列**：录制状态（命令队列 + 播放头）。
//!
//! 本类型**不依赖 [`Ui`](crate::Ui)、不需要字体图集 / GPU**——可以独立构造，因此
//! 「画出来的到底是哪几条命令」可以在**普通单测**里断言（见本模块 `tests`）。

use rjw_transform::Rect;

use crate::draw::{DrawKind, UiDraw};

/// **录制状态**：一条内容命令队列 + 一条调试命令队列 + 播放头（序号 / 深度 / 窗口 / 裁剪）。
///
/// 由 [`Painter`](crate::Painter) **按值持有**；[`Ui`](crate::Ui) 持有那个 `Painter`。
/// 字段是 crate 内可见：`finish` 的缓存 / 合批阶段要直接读队列与 `seq`。
#[derive(Debug, Default)]
pub struct DrawQueue {
    /// 内容命令（坐标 = 相对当前容器 origin 的局部坐标；容器弹出时统一平移）。
    pub(crate) queue: Vec<UiDraw>,
    /// **调试命令队列**（绝对逻辑屏幕像素；不进窗口缓存、恒在内容之后提交）。
    pub(crate) debug_queue: Vec<UiDraw>,
    /// 全局递增序号（同深度内排序）。
    pub(crate) seq: u32,
    /// 当前录制深度（容器嵌套层数）。
    pub(crate) depth: u32,
    /// 当前窗口 z 序（[`Ui::window`](crate::Ui::window)；非窗口内容 = 0）。
    pub(crate) cur_win: u32,
    /// **当前裁剪区**（绝对逻辑屏幕坐标；滚动容器 / Clip 沙箱设置）。
    pub(crate) clip: Option<Rect>,
    /// **区块级容器的嵌套层数**（[`DrawQueue::placement_locked`] 用）。
    pub(crate) placement_locks: u32,
}

impl DrawQueue {
    /// 推进并返回全局序号（同深度内排序键）。
    #[inline]
    pub(crate) fn next_seq(&mut self) -> u32 {
        self.seq += 1;
        self.seq
    }

    /// **进入一个"区块级容器"作用域**（[`crate::Ui::container_scope`]）：作用域内
    /// **顶层放置序不再新开**（见 [`Self::placement_locked`]）。
    #[inline]
    pub(crate) fn placement_push(&mut self) {
        self.placement_locks += 1;
    }

    /// 退出一层区块级作用域（与 [`Self::placement_push`] 配对）。
    #[inline]
    pub(crate) fn placement_pop(&mut self) {
        self.placement_locks = self.placement_locks.saturating_sub(1);
    }

    /// 本帧**是否处于区块级容器作用域内**（`ui.foldable(..)` 的正文 / `ui.namespace(..)`）。
    ///
    /// 命中时 [`crate::Ui::begin_top_placement`] **不新开**放置：一个逻辑区块 = **一个**
    /// 顶层放置（`place` 空间）——
    /// - 少了它，区块的正文会另起一个 `place`，而每次 `child_rect` 推进都会带上父 frame 的
    ///   `gap` ⇒ **标题与正文之间凭空多出一个 `gap`**（用户可见："展开的内容看起来悬空"），
    ///   `Foldable` / `Namespace` 的尺寸结算也跟着偏大 / 偏小；
    /// - 多一个空放置还会让 win=0 的缓存槽多出一格（空槽没有任何命令）。
    #[inline]
    pub(crate) fn placement_locked(&self) -> bool {
        self.placement_locks > 0
    }

    /// **元素序提示**：当前录制位置的下一个元素序（`seq + 1`）。
    ///
    /// 控件作者用它把"画在自家背景之上"的装饰排到本控件元素之后（见
    /// [`Painter`](crate::Painter) 的 elem 约定）。
    #[inline]
    pub(crate) fn elem_hint(&self) -> u32 {
        self.seq + 1
    }

    /// **把播放头抬到 `n`**（只增不减）。
    ///
    /// 给"一次调用里推多条命令、序号是 `seq + k`"的入口用（[`Painter::panel_img_elem`]
    /// 就是：背景刷 / 背景图 / 边框用了 `seq` / `seq + 1` / `seq + 2`，而 `next_seq()`
    /// 只推进了 1）。不补的后果是**下一条命令拿到重复序号**：`place` 归一化按 `seq` 切分
    /// ⇒ 同一个容器的命令被拆进两个排序空间（闪烁 / 序不稳），而且
    /// [`Ui::begin_top_placement`](crate::Ui) 的"播放头 = 已分配的最大序号"不变量会被打破
    /// （它是显式 `debug_assert`，实测由 `menu_bar` 的容器入口先踩到）。
    #[inline]
    pub(crate) fn advance_seq_to(&mut self, n: u32) {
        self.seq = self.seq.max(n);
    }

    /// 录制一条绘制命令（`elem` 由调用方给：`0` = 容器装饰层，画在本容器元素之下）。
    #[inline]
    pub(crate) fn push(&mut self, kind: DrawKind, rect: Rect, elem: u32) {
        let seq = self.next_seq();
        let (depth, win, clip) = (self.depth, self.cur_win, self.clip);
        self.queue.push(UiDraw { depth, seq, win, elem, rect, clip, full_w: false, kind });
    }

    /// **标记"待定满宽"**（[`UiDraw::full_w`]）：把**刚录的那条命令**标上，等它所属容器
    /// 结算尺寸后由 [`crate::Ui::expand_pending_full_w`] 回填宽度。
    ///
    /// 按**序号**找而不是"最后一条"：`panel_img` 这类入口一次推多条命令（背景 / 图 / 边框），
    /// 用"最后一条"会标错对象。找不到（命令已被取走）⇒ 静默返回。
    #[inline]
    pub(crate) fn mark_full_w(&mut self, seq: u32) {
        if let Some(d) = self.queue.iter_mut().rev().find(|d| d.seq == seq) {
            d.full_w = true;
        }
    }

    /// 本帧已录制的**内容命令**（只读；测试 / 离屏工具用）。
    #[inline]
    pub fn commands(&self) -> &[UiDraw] {
        &self.queue
    }

    /// 本帧已录制的**调试命令**（只读）。
    #[inline]
    pub fn debug_commands(&self) -> &[UiDraw] {
        &self.debug_queue
    }

    /// 取走内容命令（清空本队列；`finish` 的分桶入口用）。
    #[inline]
    pub fn take_commands(&mut self) -> Vec<UiDraw> {
        std::mem::take(&mut self.queue)
    }

    /// 内容命令条数。
    #[inline]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// 内容命令是否为空。
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rjw_color::Color;

    /// 序号逐条递增、`elem_hint` = 下一个元素序、命令字段来自当前播放头。
    #[test]
    fn push_stamps_playhead_into_the_command() {
        let mut q = DrawQueue {
            depth: 2,
            cur_win: 7,
            clip: Some(Rect::new(1.0, 2.0, 3.0, 4.0)),
            ..Default::default()
        };
        assert_eq!(q.elem_hint(), 1, "空队列的元素序提示 = 1");
        q.push(DrawKind::Solid(Color::WHITE), Rect::new(0.0, 0.0, 5.0, 5.0), 0);
        assert_eq!(q.elem_hint(), 2, "录一条之后 = 2");
        let d = &q.commands()[0];
        assert_eq!((d.depth, d.win, d.elem, d.seq), (2, 7, 0, 1));
        assert_eq!(d.clip, Some(Rect::new(1.0, 2.0, 3.0, 4.0)), "裁剪区随命令落盘");
        assert_eq!(d.rect, Rect::new(0.0, 0.0, 5.0, 5.0));
    }

    /// `take_commands` 清空队列但**不动**播放头（`seq` 单调，跨段/分桶不重号）。
    #[test]
    fn take_commands_keeps_the_playhead() {
        let mut q = DrawQueue::default();
        q.push(DrawKind::Solid(Color::WHITE), Rect::new(0.0, 0.0, 1.0, 1.0), 0);
        q.push(DrawKind::Solid(Color::WHITE), Rect::new(0.0, 0.0, 1.0, 1.0), 0);
        let taken = q.take_commands();
        assert_eq!(taken.len(), 2);
        assert!(q.is_empty(), "取走后队列为空");
        assert_eq!(q.elem_hint(), 3, "播放头保留 ⇒ 同深度后续命令 seq 不重号");
    }

    /// `advance_seq_to` **只增不减**（补号用；回退会让后续命令重号）。
    #[test]
    fn advance_seq_to_never_rewinds() {
        let mut q = DrawQueue::default();
        q.advance_seq_to(5);
        assert_eq!(q.elem_hint(), 6);
        q.advance_seq_to(3);
        assert_eq!(q.elem_hint(), 6, "不能回退");
        q.advance_seq_to(9);
        assert_eq!(q.elem_hint(), 10);
    }
}
