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
}

impl DrawQueue {
    /// 推进并返回全局序号（同深度内排序键）。
    #[inline]
    pub(crate) fn next_seq(&mut self) -> u32 {
        self.seq += 1;
        self.seq
    }

    /// **元素序提示**：当前录制位置的下一个元素序（`seq + 1`）。
    ///
    /// 控件作者用它把"画在自家背景之上"的装饰排到本控件元素之后（见
    /// [`Painter`](crate::Painter) 的 elem 约定）。
    #[inline]
    pub(crate) fn elem_hint(&self) -> u32 {
        self.seq + 1
    }

    /// 录制一条绘制命令（`elem` 由调用方给：`0` = 容器装饰层，画在本容器元素之下）。
    #[inline]
    pub(crate) fn push(&mut self, kind: DrawKind, rect: Rect, elem: u32) {
        let seq = self.next_seq();
        let (depth, win, clip) = (self.depth, self.cur_win, self.clip);
        self.queue.push(UiDraw { depth, seq, win, elem, rect, clip, kind });
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
}
