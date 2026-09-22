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

use crate::widgets::SizeClass;

/// **由内向外**找第一个给出内容可用宽的 frame（纯函数，可单测）。
///
/// `Ui::avail_w` 用它：`row` 自己不是固定宽容器，只看最内层会让 row 里的控件拿不到
/// 外层窗口的可用宽（见 [`Frame::avail_w`] 的说明）。
pub(crate) fn stack_avail_w(frames: &[Frame]) -> Option<f32> {
    frames.iter().rev().find_map(|f| f.avail_w())
}

/// **自适应收窄的下限**（物理像素）：水平一行里给"下一个子项"留的余量小于它时
/// **不再压窄**（宁可让内容溢出可见，也不把控件压成一条线 / 0 宽）。
///
/// 24 物理像素 ≈ 150% DPI 下 16 逻辑像素——比这更窄的控件已经点不中、看不清，
/// 压它只会把"内容溢出"换成"控件消失"，后者更难排查。
const ADAPT_MIN_W: f32 = 24.0;

/// pack 堆叠方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackSide {
    /// 垂直堆叠（自上而下），子项左对齐；宽度 = 最大子项宽。
    Top,
    /// 水平堆叠（自左而右），子项顶对齐；高度 = 最大子项高。
    Left,
    /// 垂直堆叠（**自下而上**，内容向上生长），子项左对齐；宽度 = 最大子项宽。
    /// `pack_at` 的 `pos` 对准 pack **下边缘**（如页脚：`pos.y` = 底部，向上长）。
    Bottom,
    /// 水平堆叠（**自右而左**，内容向左生长），子项顶对齐；高度 = 最大子项高。
    /// `pack_at` 的 `pos` 对准 pack **右边缘**（如右对齐菜单栏：`pos.x` = 右侧，向左长）。
    Right,
}

/// 容器布局种类。
#[derive(Clone, Copy, Debug)]
pub(crate) enum FrameKind {
    /// pack / panel 堆叠布局。
    Stack { side: PackSide, gap: f32 },
    /// grid 网格布局：`cols` 列，单元格尺寸 `cell`（跨帧缓存，见 `UiState::grid_cells`）。
    Grid { cols: usize, cell: Vec2 },
}

/// **子项对父级尺寸的贡献方式**（取代旧的 `expands: bool` 裸布尔开关）。
///
/// 用于 [`Ui::child_rect`](crate::Ui::child_rect) / [`UiAdd::add`](crate::UiAdd::add)：
/// - [`Child::Expand`]（默认）：子项尺寸计入父级 `max_child`（Stack 撑大父级 / Grid 扩格）；
/// - [`Child::Fit`]：**不撑大父级**——按自身尺寸放置，溢出由控件自洽
///   （对应 [`crate::widgets::Expansion::DisableAutoExpansion`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Child {
    /// 撑大父级（默认）。
    #[default]
    Expand,
    /// 不撑大父级（限制在父级可用空间内，内容自洽）。
    Fit,
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
    /// **本 frame 全部子项的强制高度**（水平行 `row` 等高；`None` = 自然测量）。
    /// 持续作用于本 frame 所有子项（区别于一次性 `next_fixed_h`）。
    ///
    /// ⚠ 语义 = **标准行高**：单行子项被**钉到**它，多行子项以它为**下限**（见
    /// [`Self::next_class`] 与 [`crate::widgets::SizeClass`]）。
    force_h_all: Option<f32>,
    /// **下一个子项的尺寸类**（一次性，`child_rect` 消费；默认
    /// [`SizeClass::SingleLine`] = 旧行为）。由 `Ui::add` 在调 `Widget::ui` 之前写入。
    next_class: SizeClass,
    /// **行级高度下限 / 上限**（只由 `Ui::row_builder` 的 `RowBuilder` 设置；`None` = 不限）。
    /// 在 [`Self::settle_size`] 末尾按"先压 max 再抬 min"夹取（min 胜）。
    row_min_h: Option<f32>,
    row_max_h: Option<f32>,
    /// **容器固定高度**（flex_at 等；覆盖 `settle_size` 的自然高度）。
    fixed_h: Option<f32>,
    /// **容器固定宽度**（`window_at_w` 等；覆盖 `settle_size` 的自然宽度，
    /// 子项宽度 clamp 到该值——内容按固定宽排布、高度自然，如同 egui）。
    fixed_w: Option<f32>,
    /// **内容最大宽**（`None` = 不限）：子项宽度 clamp 到它，但不改写 `settle_size`
    /// 的自然宽度（容器仍按内容结算，只是不会长过这个上限）。
    ///
    /// 由 [`Ui::container`](crate::Ui::container) 从**父级可用宽**继承而来——这一条是
    /// "嵌套容器（`row` / `panel` / `view`）不会把内容排到固定宽窗口外面"的机器保证：
    /// 子项被 clamp、`Label` 这类 `LimitedInParent` 控件经 [`Self::avail_w`] 拿到该宽
    /// 后自动换行（实测过的 bug：窄的 `.width()` 窗口里 `row` 里的控件整排突出去，
    /// 直到把窗口拖大才看得回去）。
    max_w: Option<f32>,
    /// **容器最小宽度**（见 [`Self::set_min_w`]；`None` = 不限）：只抬 `settle_size` 的宽。
    min_w: Option<f32>,
    /// **绝对放置内容的包围盒**（`*_at` / `add_at` / 控件命中区 的矩形并集；
    /// **相对容器 origin**，与 `child_rect` 同一空间）。
    ///
    /// [`Self::settle_size`] 把它并入自然尺寸 ⇒ **容器尺寸包住子控件**。
    ///
    /// 没有它时，`grid_at` / `add_at` 这类**不占光标**的绝对放置不进容器的自然尺寸：
    /// 内容画得出来，却在窗口矩形之外 ⇒ **不在窗口 z-order / 遮挡 / 拖拽判定里**
    /// （示例"背包按钮超出窗口"就是这么来的：窗口只有标题栏那么高，格子全在框外，
    /// 于是重叠的另一个窗口反而盖住了它们、拖窗口也拖不动它们）。
    content_bounds: Option<Rect>,
}

impl Frame {
    pub(crate) fn new_stack(side: PackSide, gap: f32, pad_total: f32) -> Self {
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
            force_h_all: None,
            next_class: SizeClass::SingleLine,
            row_min_h: None,
            row_max_h: None,
            fixed_h: None,
            fixed_w: None,
            max_w: None,
            min_w: None,
            content_bounds: None,
        }
    }

    pub(crate) fn new_grid(cols: usize, cell: Vec2, pad_total: f32) -> Self {
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
            force_h_all: None,
            next_class: SizeClass::SingleLine,
            row_min_h: None,
            row_max_h: None,
            fixed_h: None,
            fixed_w: None,
            max_w: None,
            min_w: None,
            content_bounds: None,
        }
    }

    /// 设置**下一子项**的最小尺寸约束（`0` = 该轴不约束）。一次性，`child_rect` 消耗。
    /// 多次调用取各轴最大值。
    pub(crate) fn set_next_min(&mut self, min: impl Into<Vec2>) {
        let min = min.into();
        self.next_min = Vec2::new(self.next_min.x.max(min.x), self.next_min.y.max(min.y));
    }

    /// 设置**下一子项**的最大尺寸约束（`0` = 该轴不约束）。一次性，`child_rect` 消耗。
    /// 多次调用取各轴最小值（0 表示不约束，取非零较小值）。
    pub(crate) fn set_next_max(&mut self, max: impl Into<Vec2>) {
        let max = max.into();
        let merge = |cur: f32, v: f32| {
            if cur <= 0.0 { v } else if v <= 0.0 { cur } else { cur.min(v) }
        };
        self.next_max = Vec2::new(merge(self.next_max.x, max.x), merge(self.next_max.y, max.y));
    }

    /// 强制**下一子项**高度（flex 权重分配）。一次性，`child_rect` 消耗。
    pub(crate) fn force_next_h(&mut self, h: f32) {
        self.next_fixed_h = Some(h.max(0.0));
    }

    /// **强制本 frame 全部子项等高**（水平行 `row` 用）：`child_rect` 时高度 =
    /// `force_h_all`（覆盖自然高 / 一次性 next_fixed_h），持续到 frame 结束。
    ///
    /// ⚠ 对**多行**子项（[`SizeClass::Multiline`]）它是**下限**而不是上限——多行控件
    /// 可以把整行撑高（`row` 的 `min_h` 语义）；单行子项照旧被钉到该高度。
    pub(crate) fn set_force_h_all(&mut self, h: f32) {
        self.force_h_all = Some(h.max(0.0));
    }

    /// 设置**下一子项**的尺寸类（一次性，`child_rect` 消费）。`Ui::add` 在调用
    /// `Widget::ui` 之前按 [`crate::widgets::Widget::size_class`] 写入。
    pub(crate) fn set_next_class(&mut self, class: SizeClass) {
        self.next_class = class;
    }

    /// **行级高度约束**（`row_builder` 的 `min_h` / `max_h`）：容器结算尺寸的高度被
    /// 夹到 `[min, max]`（`min > max` 时 min 胜）。`None` = 该端不限。
    ///
    /// 只影响**容器自身**的结算高度（⇒ 父级光标推进 / 后续控件位置）；子项各自
    /// 按自己的高度录制，`max` 比子项矮时内容溢出（同 `fixed_h` 的语义）。
    pub(crate) fn set_row_bounds(&mut self, min: Option<f32>, max: Option<f32>) {
        self.row_min_h = min.map(|v| v.max(0.0));
        self.row_max_h = max.map(|v| v.max(0.0));
    }

    /// 固定容器结算高度（`settle_size` 覆盖自然高度）。
    pub(crate) fn set_fixed_h(&mut self, h: f32) {
        self.fixed_h = Some(h.max(0.0));
    }

    /// 固定容器结算宽度（`settle_size` 覆盖自然宽度；子项宽度 clamp 到该值）。
    pub(crate) fn set_fixed_w(&mut self, w: f32) {
        self.fixed_w = Some(w.max(0.0));
    }

    /// 固定宽容器（`fixed_w`）扣除内边距后的**内容可用宽度**（`None` = 未设固定宽）。
    /// 供 `Ui::avail_w`（沙箱可用宽度）兜底。
    pub(crate) fn fixed_avail_w(&self) -> Option<f32> {
        match self.fixed_w {
            Some(w) if w > 0.0 => Some((w - self.pad_total * 2.0).max(0.0)),
            _ => None,
        }
    }

    /// **本容器给出的内容可用宽**（`None` = 本帧不限制宽度）：
    /// 固定宽容器 ⇒ `fixed_avail_w`；否则继承来的 [`Self::max_w`]。
    ///
    /// `Ui::avail_w` 由内向外找**第一个**有值的 frame —— 于是 `row` 里的 `Label`
    /// 也能看到外层窗口的固定宽（否则它会按自然宽把整行排到窗口外面）。
    pub(crate) fn avail_w(&self) -> Option<f32> {
        self.fixed_avail_w().or(match self.max_w {
            Some(w) if w > 0.0 => Some(w),
            _ => None,
        })
    }

    /// **容器最小宽度**（`None` = 不限）：`settle_size` 的宽度被**抬到**它
    /// （内容比它宽 ⇒ 内容胜 —— 这正是 `ScrollMode::NoClip` 的"拖出来的是下限"语义：
    /// 窗口不许裁掉内容，所以拖小只会被内容顶回去）。
    ///
    /// 与 [`Self::fixed_w`] 的区别：固定宽**覆盖**自然宽（内容超出被裁）；最小宽只是下限。
    pub(crate) fn set_min_w(&mut self, w: f32) {
        self.min_w = (w > 0.0).then_some(w);
    }

    /// 设置**内容最大宽**（见 [`Self::max_w`]；`None` / `<= 0` = 不限）。
    pub(crate) fn set_max_w(&mut self, w: Option<f32>) {
        self.max_w = w.filter(|w| *w > 0.0);
    }

    /// **本容器还能给下一个子项多少宽**（`None` = 不限 / 不是水平堆叠）。
    ///
    /// 内容盒（局部坐标）= `[pad_total, pad_total + max_w]`：
    /// - [`PackSide::Left`]：子项向右推进 ⇒ 余量 = `pad_total + max_w - cursor.x`；
    /// - [`PackSide::Right`]：子项向左推进 ⇒ 余量 = `cursor.x - pad_total`；
    /// - 垂直堆叠 / 没有 `max_w` / grid ⇒ `None`。
    ///
    /// 这是"一行里的**最后一个**控件也得缩"的依据——只按单子项上限 clamp 不够：
    /// 一行 `标签 + 输入框`（97 + 9 + 210 = 316）在 276 的可用宽里**每个子项都没超限**，
    /// 但整行仍然排到窗口外面（用户实测的 BUG）。给输入框的余量 = 276 − 106 = 170
    /// ⇒ 它缩到 170，整行正好落在可用宽内。
    pub(crate) fn remaining_w(&self) -> Option<f32> {
        let mw = self.max_w?;
        let FrameKind::Stack { side, .. } = &self.kind else {
            return None;
        };
        match side {
            PackSide::Left => Some((self.pad_total + mw - self.cursor.x).max(0.0)),
            PackSide::Right => Some((self.cursor.x - self.pad_total).max(0.0)),
            PackSide::Top | PackSide::Bottom => None,
        }
    }

    /// 本帧是否已**固定宽**（固定宽容器由调用方定死，嵌套容器不必再继承上限）。
    pub(crate) fn has_fixed_w(&self) -> bool {
        self.fixed_w.is_some()
    }

    /// 下一子项的 max **宽度**约束（`0` = 不约束）。
    pub(crate) fn next_max_w(&self) -> f32 {
        self.next_max.x
    }

    /// 已放置子项的最大宽度（`0` = 尚无子项）。分割线"取当前容器宽度"用。
    pub(crate) fn max_child_w(&self) -> f32 {
        self.max_child.x
    }

    /// **记入一处内容矩形**（相对本容器 origin，与 `child_rect` 同空间）：
    /// 容器结算尺寸**至少包住它**（见 [`Self::content_bounds`]）。
    ///
    /// 调用方：`Ui::container` / `Ui::flex_at`（绝对容器整体）、`Ui::add_at`
    /// （绝对放置控件）、`Ui::hit_abs`（任何可交互控件的命中区——**交互范围必须落在
    /// 容器内**，否则"看得见点得着却不在窗口矩形里"）。
    pub(crate) fn note_content(&mut self, rect: Rect) {
        self.content_bounds = Some(match self.content_bounds {
            Some(b) => b.union(&rect),
            None => rect,
        });
    }

    /// 本容器已记入的内容包围盒（**相对本容器 origin**；`None` = 没记过）。
    ///
    /// 容器弹出时把它平移到父级空间并 `note_content` 上去——**必须**这样做：
    /// grid / pack 的自然尺寸只统计"单元格 × 列数"，子控件若比单元格宽（内容变了、
    /// 单元格缓存还是上一帧的值），自然尺寸会**低估**子控件的实际范围。
    pub(crate) fn content_bounds(&self) -> Option<Rect> {
        self.content_bounds
    }

    /// 为尺寸 `(w, h)` 的子项分配一个局部矩形，并推进光标 / 更新统计
    /// （**撑大父级**；等价 `child_rect_exp(w, h, true)`）。
    ///
    /// 生产路径统一走 [`Self::child_rect_exp`]（`Child::Fit/Expand` 由 `Ui` 决定）；
    /// 本便捷入口只给本模块单测用。
    #[cfg(test)]
    pub(crate) fn child_rect(&mut self, w: f32, h: f32) -> Rect {
        self.child_rect_inner(w, h, true)
    }

    /// 同 [`Self::child_rect`]，但 `expands = false` 时该子项**不撑大父级**
    /// （`max_child` 不更新、grid 不扩格）——`DisableAutoExpansion` 控件语义：
    /// 内容按自身尺寸放置，溢出由控件自身自洽（noclip / 省略）。
    pub(crate) fn child_rect_exp(&mut self, w: f32, h: f32, expands: bool) -> Rect {
        self.child_rect_inner(w, h, expands)
    }

    /// **外部放置**（嵌套容器结算后补记）：按 Stack 语义推进光标 + 更新 `max_child`
    /// + 计数——供"占光标"式嵌套容器（如 [`crate::ui::UiAdd::row`]）使用：
    ///   子容器已按自身 Frame 放置内容，结算尺寸 `size` 后由父 Frame 补记占位。
    pub(crate) fn place_external(&mut self, size: Vec2) {
        match &mut self.kind {
            FrameKind::Stack { side, gap } => {
                match side {
                    PackSide::Top => self.cursor.y += size.y + *gap,
                    PackSide::Left => self.cursor.x += size.x + *gap,
                    PackSide::Bottom => self.cursor.y -= size.y + *gap,
                    PackSide::Right => self.cursor.x -= size.x + *gap,
                }
                self.max_child.x = self.max_child.x.max(size.x);
                self.max_child.y = self.max_child.y.max(size.y);
                self.count += 1;
            }
            FrameKind::Grid { .. } => {
                // grid 单元格尺寸由 grid 内部管理（外部放置不适用）。
            }
        }
    }

    /// `child_rect` 公共实现（`track_max`：是否更新 `max_child` / 扩展 grid 单元格）。
    fn child_rect_inner(&mut self, w: f32, h: f32, track_max: bool) -> Rect {
        let w = w.max(0.0);
        let h = h.max(0.0);
        let w = if self.next_max.x > 0.0 { w.min(self.next_max.x).max(self.next_min.x) } else { w.max(self.next_min.x) };
        let h = if self.next_max.y > 0.0 { h.min(self.next_max.y).max(self.next_min.y) } else { h.max(self.next_min.y) };
        let h = self.next_fixed_h.take().unwrap_or(h);
        // 行标准高（`row`）：**单行子项钉到它**（旧行为，文字中心线对齐）、
        // **多行子项以它为下限**（可撑高整行——`TextEditor::multiline()` 这类）。
        let h = match self.force_h_all {
            None => h,
            Some(std_h) => match self.next_class {
                SizeClass::Multiline => h.max(std_h),
                SizeClass::SingleLine => std_h,
            },
        };
        self.next_min = Vec2::ZERO;
        self.next_max = Vec2::ZERO;
        self.next_class = SizeClass::SingleLine;
        // 容器固定宽：子项宽度 clamp（内容按固定宽排布，高度自然）
        let w = match self.fixed_w {
            Some(fw) if fw > 0.0 => w.min(fw),
            _ => w,
        };
        // 内容最大宽（从父级继承）：同样 clamp 子项宽度，但**不改写**结算宽 ——
        // 嵌套容器因此不会把内容排到固定宽窗口外面（宽度收窄，`LimitedInParent`
        // 控件经 `avail_w` 拿到该宽后自动换行 / 压窄）。
        let w = match self.max_w {
            Some(mw) => w.min(mw),
            None => w,
        };
        // **水平堆叠的余量**：一行里"最后一个控件"也必须缩，否则整行会排到可用宽
        // 外面（单子项各自都没超限）。只在余量还够一个像样的控件时才压——余量太小
        // 就宁可让它溢出，也不能把控件压成 0 宽（那等于凭空消失）。
        let w = match self.remaining_w() {
            Some(rem) if rem < w && rem >= ADAPT_MIN_W => rem,
            _ => w,
        };
        let placed = match &mut self.kind {
            FrameKind::Stack { side, gap } => {
                let local = self.cursor;
                match side {
                    PackSide::Top => {
                        self.cursor.y += h + *gap;
                        if track_max {
                            self.max_child.x = self.max_child.x.max(w);
                            self.max_child.y = self.max_child.y.max(h);
                        }
                    }
                    PackSide::Left => {
                        self.cursor.x += w + *gap;
                        if track_max {
                            self.max_child.x = self.max_child.x.max(w);
                            self.max_child.y = self.max_child.y.max(h);
                        }
                    }
                    // Bottom/Right：**负向推进**——子项落在 y/x ≤ 0 的负区，
                    // 故 pack 的 0 边（`pos` 锚定边）即下/右边；首个子项贴该边。
                    PackSide::Bottom => {
                        self.cursor.y -= h + *gap;
                        if track_max {
                            self.max_child.x = self.max_child.x.max(w);
                            self.max_child.y = self.max_child.y.max(h);
                        }
                    }
                    PackSide::Right => {
                        self.cursor.x -= w + *gap;
                        if track_max {
                            self.max_child.x = self.max_child.x.max(w);
                            self.max_child.y = self.max_child.y.max(h);
                        }
                    }
                }
                self.count += 1;
                Rect::new(local.x, local.y, w, h)
            }
            FrameKind::Grid { cols, cell } => {
                // 渐进扩展 cell：容纳当前子项（缓存值不足时本帧就地扩大，位置即时一致）。
                if track_max {
                    if w > cell.x {
                        cell.x = w;
                    }
                    if h > cell.y {
                        cell.y = h;
                    }
                }
                let col = self.count % *cols;
                let row = self.count / *cols;
                self.count += 1;
                if track_max {
                    self.max_child.x = self.max_child.x.max(w);
                    self.max_child.y = self.max_child.y.max(h);
                }
                Rect::new(
                    self.pad_total + col as f32 * cell.x,
                    self.pad_total + row as f32 * cell.y,
                    w,
                    h,
                )
            }
        };
        // **每处子项矩形都进内容包围盒**（仅 `Child::Expand`——`Fit` /
        // `DisableAutoExpansion` 的语义就是"**不**撑大父级"）。
        // grid 的自然尺寸是"列数 × 单元格缓存"，单元格比子项窄时（内容刚变宽、
        // 缓存还是上一帧的值）会**低估**子项范围 ⇒ 子项"长到容器外"，交互随之失真。
        // ⚠ 必须在此处（布局期、与鼠标无关）记，不能在命中测试里记——否则容器尺寸
        // 会随鼠标位置变化。
        if track_max {
            self.note_content(placed);
        }
        placed
    }

    /// 结算容器**总尺寸**（含 pad_total 外扩；相对容器 origin）：
    /// 自然尺寸 ∪ 绝对放置内容的包围盒（[`Self::content_bounds`]），最后按
    /// [`Self::set_row_bounds`] 夹取高度（行级 min/max）。
    pub(crate) fn settle_size(&self) -> Vec2 {
        let size = self.settle_size_inner();
        // 行级高度：先压 max 再抬 min（**min 胜**，与 `apply_constraints` 同口径）。
        let h = match self.row_max_h {
            Some(m) => size.y.min(m),
            None => size.y,
        };
        let h = match self.row_min_h {
            Some(m) => h.max(m),
            None => h,
        };
        // 容器**最小宽度**（`set_min_w`）：只抬宽，不改写子项布局 —— `NoClip` 轴的
        // "拖出来的是下限"用它表达（内容比拖出来的大 ⇒ 内容胜）。
        let w = match self.min_w {
            Some(m) => size.x.max(m),
            None => size.x,
        };
        Vec2::new(w, h)
    }

    fn settle_size_inner(&self) -> Vec2 {
        let natural = self.natural_size();
        let Some(b) = self.content_bounds else {
            return natural;
        };
        // 内容包围盒的**右下角** + 另一侧 padding（左上溢出无法让容器"向左上长"，
        // 那部分仍旧溢出——但**往右/往下**放的内容一律被包住）。
        //
        // ⚠ **固定轴不参与**：`fixed_w` / `fixed_h`（窗口固定宽 / flex 定高）表示
        // "这一轴由调用方定死"，内容按它排布、超出由 `Clip` 语义处理；若这里也并进
        // 包围盒，固定宽窗口会被宽内容硬撑开（`modal` 的按钮行 spacer 就是这种内容）。
        let far = b.max() + Vec2::splat(self.pad_total);
        Vec2::new(
            if self.fixed_w.is_some() {
                natural.x
            } else {
                natural.x.max(far.x)
            },
            if self.fixed_h.is_some() {
                natural.y
            } else {
                natural.y.max(far.y)
            },
        )
    }

    fn natural_size(&self) -> Vec2 {
        match &self.kind {
            FrameKind::Stack { side, gap } => {
                if self.count == 0 {
                    let p = self.pad_total * 2.0;
                    return Vec2::new(p, p);
                }
                match side {
                    PackSide::Top => {
                        let w = match self.fixed_w {
                            Some(fw) if fw > 0.0 => fw + self.pad_total * 2.0,
                            _ => self.max_child.x + self.pad_total * 2.0,
                        };
                        let h = self.fixed_h.unwrap_or((self.cursor.y - gap).max(0.0) + self.pad_total);
                        Vec2::new(w, h)
                    }
                    PackSide::Left => Vec2::new(
                        (self.cursor.x - gap).max(0.0) + self.pad_total,
                        self.max_child.y + self.pad_total * 2.0,
                    ),
                    // Bottom/Right：光标为负，取绝对值结算（0 边 = pos 锚定边）。
                    PackSide::Bottom => {
                        let w = match self.fixed_w {
                            Some(fw) if fw > 0.0 => fw + self.pad_total * 2.0,
                            _ => self.max_child.x + self.pad_total * 2.0,
                        };
                        let h = self.fixed_h.unwrap_or((-self.cursor.y - gap).max(0.0) + self.pad_total);
                        Vec2::new(w, h)
                    }
                    PackSide::Right => Vec2::new(
                        (-self.cursor.x - gap).max(0.0) + self.pad_total,
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
    fn stack_bottom_advances_y_negative_and_settles() {
        // Bottom：首个子项在锚定边（下边 y=0），后续向上（负 y），0 边即 pos 锚定边。
        let mut f = Frame::new_stack(PackSide::Bottom, 6.0, 0.0);
        let a = f.child_rect(100.0, 20.0);
        assert_eq!(a, Rect::new(0.0, 0.0, 100.0, 20.0), "首个子项在底部（y=0）");
        let b = f.child_rect(80.0, 30.0);
        assert_eq!(b, Rect::new(0.0, -26.0, 80.0, 30.0), "第二个在第一个上方（-20-gap6）");
        assert_eq!(f.settle_size(), Vec2::new(100.0, 56.0), "宽=max(100,80)，高=20+6+30");
    }

    #[test]
    fn stack_right_advances_x_negative_and_settles() {
        // Right：首个子项在锚定边（右边 x=0），后续向左（负 x），0 边即 pos 锚定边。
        let mut f = Frame::new_stack(PackSide::Right, 4.0, 0.0);
        let a = f.child_rect(30.0, 50.0);
        assert_eq!(a, Rect::new(0.0, 0.0, 30.0, 50.0), "首个子项在右侧（x=0）");
        let b = f.child_rect(40.0, 60.0);
        assert_eq!(b, Rect::new(-34.0, 0.0, 40.0, 60.0), "第二个在第一个左侧（-30-gap4）");
        assert_eq!(f.settle_size(), Vec2::new(74.0, 60.0), "宽=30+4+40，高=max(50,60)");
    }

    #[test]
    fn stack_bottom_min_max_and_fixed_h() {
        // Bottom 与 min/max 约束、fixed_h 叠加（负向推进下 clamp 行为不变）。
        let mut f = Frame::new_stack(PackSide::Bottom, 6.0, 0.0);
        f.set_next_min(Vec2::new(100.0, 0.0));
        assert_eq!(f.child_rect(40.0, 20.0), Rect::new(0.0, 0.0, 100.0, 20.0), "min 抬宽");
        f.set_next_max(Vec2::new(80.0, 0.0));
        assert_eq!(f.child_rect(120.0, 10.0), Rect::new(0.0, -26.0, 80.0, 10.0), "max 压宽");
        assert_eq!(f.settle_size().y, 36.0, "高=20+6+10");
        f.set_fixed_h(200.0);
        assert_eq!(f.settle_size().y, 200.0, "fixed_h 覆盖结算高");
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
    fn content_bounds_make_container_cover_absolute_placements() {
        // 绝对放置（`*_at` / `add_at`）不占光标：若不记内容包围盒，容器**只有标题那么高**
        // ⇒ 控件"长到窗口外"（画得出来但不在窗口矩形 / 遮挡判定里）。
        let mut f = Frame::new_stack(PackSide::Top, 6.0, 0.0);
        assert_eq!(f.child_rect(100.0, 20.0), Rect::new(0.0, 0.0, 100.0, 20.0));
        assert_eq!(f.settle_size(), Vec2::new(100.0, 20.0), "只有流内子项时尺寸不变");
        // 绝对放置在 (0, 28) 处、尺寸 200×120
        f.note_content(Rect::new(0.0, 28.0, 200.0, 120.0));
        assert_eq!(f.settle_size(), Vec2::new(200.0, 148.0), "尺寸撑到包住它");
        // 再记一个往右下更远的矩形：取并集右下角
        f.note_content(Rect::new(40.0, 60.0, 260.0, 40.0));
        assert_eq!(f.settle_size(), Vec2::new(300.0, 148.0));
        // 往左上溢出的部分无法让容器"向左上长"（只取右下）
        f.note_content(Rect::new(-20.0, -10.0, 30.0, 30.0));
        assert_eq!(f.settle_size(), Vec2::new(300.0, 148.0));
    }

    #[test]
    fn grid_child_wider_than_cached_cell_is_covered() {
        // grid 的自然尺寸 = 列数 × **单元格缓存**；子控件比缓存宽时（内容刚变宽）会低估
        // 范围 ⇒ 必须靠"每处子项矩形都进包围盒"兜住。
        let mut f = Frame::new_grid(3, Vec2::new(50.0, 20.0), 0.0);
        // 三个子项都请求 80 宽（> 缓存 50）⇒ 就地扩格到 80
        for _ in 0..3 {
            f.child_rect(80.0, 20.0);
        }
        let size = f.settle_size();
        assert_eq!(size.x, 240.0, "三列 × 80（就地扩格后的单元格）");
        // 单元格缓存比子项宽的**反向**情况（子项 30 宽、缓存 50）：
        let mut g = Frame::new_grid(2, Vec2::new(50.0, 20.0), 0.0);
        g.child_rect(30.0, 20.0);
        g.child_rect(30.0, 20.0);
        // 单元格仍是 50（不缩）⇒ 自然尺寸 100 已覆盖子项；内容包围盒不小于它
        assert_eq!(g.settle_size().x, 100.0);
        assert!(g.content_bounds().is_some_and(|b| b.max().x <= 100.0));
    }

    #[test]
    fn note_content_survives_settle_and_exposes_bounds() {
        let mut f = Frame::new_stack(PackSide::Top, 0.0, 10.0);
        f.note_content(Rect::new(0.0, 0.0, 60.0, 40.0));
        assert_eq!(f.content_bounds(), Some(Rect::new(0.0, 0.0, 60.0, 40.0)));
        // pad_total 在另一侧外扩（右下角 + pad）
        assert_eq!(f.settle_size(), Vec2::new(70.0, 50.0));
        // **固定轴不参与**：固定宽容器不能被宽内容撑开（固定宽 = 按该宽排布，
        // 超出交给 Clip 语义；`modal` 的按钮行 spacer 就是这种"宽内容"）。
        let mut g = Frame::new_stack(PackSide::Top, 0.0, 0.0);
        g.set_fixed_w(200.0);
        g.child_rect(200.0, 20.0);
        g.note_content(Rect::new(0.0, 30.0, 600.0, 20.0));
        assert_eq!(g.settle_size(), Vec2::new(200.0, 50.0), "宽锁在 200，高仍被内容撑开");
    }

    #[test]
    fn fit_child_does_not_grow_container() {
        // `Child::Fit`（`DisableAutoExpansion`）语义 = **不撑大父级** ⇒ 不进内容包围盒。
        // （Stack 的光标照常前进 → 高度仍会长；这里是**宽**不被撑开。）
        let mut f = Frame::new_stack(PackSide::Top, 0.0, 0.0);
        f.child_rect(50.0, 20.0);
        f.child_rect_exp(400.0, 20.0, false); // Fit：故意超宽
        assert_eq!(f.settle_size().x, 50.0, "Fit 子项不撑宽容器");
        assert!(f.content_bounds().is_none_or(|b| b.max().x <= 50.0));
        // 对照组：Expand 的超宽子项**必须**撑宽容器（否则内容长到外面）
        let mut g = Frame::new_stack(PackSide::Top, 0.0, 0.0);
        g.child_rect(50.0, 20.0);
        g.child_rect(400.0, 20.0);
        assert_eq!(g.settle_size().x, 400.0);
    }

    #[test]
    fn force_next_h_overrides_measured_height() {        let mut f = Frame::new_stack(PackSide::Top, 6.0, 0.0);
        f.force_next_h(60.0);
        assert_eq!(f.child_rect(50.0, 20.0), Rect::new(0.0, 0.0, 50.0, 60.0), "高度被强制为 60");
        assert_eq!(f.child_rect(50.0, 20.0), Rect::new(0.0, 66.0, 50.0, 20.0), "一次性，后续恢复自然");
        assert_eq!(f.settle_size(), Vec2::new(50.0, 86.0));
    }

    #[test]
    fn force_h_all_makes_every_child_equal_height() {
        // 水平行等高（row）：全部子项高度 = row_h（持续作用于本 frame）
        let mut f = Frame::new_stack(PackSide::Left, 6.0, 0.0);
        f.set_force_h_all(26.0);
        assert_eq!(f.child_rect(30.0, 16.0), Rect::new(0.0, 0.0, 30.0, 26.0), "Label 被抬高到 row_h");
        assert_eq!(f.child_rect(60.0, 26.0), Rect::new(36.0, 0.0, 60.0, 26.0), "Input 保持 row_h");
        assert_eq!(f.child_rect(40.0, 20.0), Rect::new(102.0, 0.0, 40.0, 26.0), "继续等高");
        // 结算：高 = row_h（max_child.y），宽 = 子项宽和 + 间距（30+6+60+6+40 = 142）
        assert_eq!(f.settle_size(), Vec2::new(142.0, 26.0));
        // force_h_all 与一次性 force_next_h 并存：force_h_all 覆盖（row 语义）
        let mut g = Frame::new_stack(PackSide::Left, 6.0, 0.0);
        g.set_force_h_all(26.0);
        g.force_next_h(60.0);
        assert_eq!(g.child_rect(30.0, 16.0).h, 26.0, "行等高优先于一次性 flex 高");
    }

    #[test]
    fn row_multiline_child_grows_the_row() {
        // 多行子项（`SizeClass::Multiline`）：标准行高只是**下限**，可以撑高整行。
        let mut f = Frame::new_stack(PackSide::Left, 6.0, 0.0);
        f.set_force_h_all(26.0);
        f.set_next_class(SizeClass::Multiline);
        assert_eq!(
            f.child_rect(200.0, 90.0),
            Rect::new(0.0, 0.0, 200.0, 90.0),
            "多行子项按自身高度（不被压成一行）"
        );
        // 同 frame 内混排：后面的单行子项仍被钉到 26（文字中心线对齐不受影响）
        assert_eq!(f.child_rect(30.0, 16.0), Rect::new(206.0, 0.0, 30.0, 26.0));
        assert_eq!(f.settle_size(), Vec2::new(236.0, 90.0), "行高 = 最高子项（宽 = 200+6+30）");
        // 比标准行高**矮**的多行子项也要吃下限
        let mut g = Frame::new_stack(PackSide::Left, 0.0, 0.0);
        g.set_force_h_all(26.0);
        g.set_next_class(SizeClass::Multiline);
        assert_eq!(g.child_rect(100.0, 10.0).h, 26.0, "多行子项同样有标准高下限");
        // 类标记是**一次性**的：用一个"自然高 > 行高"的后续子项来证明它已复位
        // （单行 ⇒ 被压回 26；若标记泄漏 ⇒ 会保持 90）。
        let mut h = Frame::new_stack(PackSide::Left, 0.0, 0.0);
        h.set_force_h_all(26.0);
        h.set_next_class(SizeClass::Multiline);
        h.child_rect(10.0, 90.0);
        assert_eq!(h.child_rect(10.0, 90.0).h, 26.0, "标记只作用于紧接着的那个子项");
    }

    #[test]
    fn row_bounds_clamp_settled_height() {
        // 行级 min/max：先压 max 再抬 min（min 胜，与 `apply_constraints` 同口径）。
        let mut f = Frame::new_stack(PackSide::Left, 0.0, 0.0);
        f.child_rect(50.0, 20.0);
        f.set_row_bounds(Some(40.0), None);
        assert_eq!(f.settle_size().y, 40.0, "低于下限 ⇒ 抬到下限");
        let mut g = Frame::new_stack(PackSide::Left, 0.0, 0.0);
        g.child_rect(50.0, 200.0);
        g.set_row_bounds(Some(40.0), Some(120.0));
        assert_eq!(g.settle_size().y, 120.0, "高于上限 ⇒ 压到上限");
        let mut h = Frame::new_stack(PackSide::Left, 0.0, 0.0);
        h.child_rect(50.0, 60.0);
        h.set_row_bounds(Some(90.0), Some(30.0));
        assert_eq!(h.settle_size(), Vec2::new(50.0, 90.0), "min > max 时 min 胜；宽度不受影响");
    }

    #[test]
    fn fixed_h_overrides_settle_height() {
        let mut f = Frame::new_stack(PackSide::Top, 6.0, 0.0);
        f.child_rect(50.0, 20.0);
        f.child_rect(40.0, 10.0);
        f.set_fixed_h(200.0);
        assert_eq!(f.settle_size(), Vec2::new(50.0, 200.0), "固定高覆盖自然结算");
    }

    #[test]
    fn fixed_w_clamps_children_and_settles_width() {
        // 窗口固定宽：子项宽度 clamp 到 fixed_w，高度自然（egui 风格）
        let mut f = Frame::new_stack(PackSide::Top, 6.0, 4.0);
        f.set_fixed_w(120.0);
        assert_eq!(f.child_rect(300.0, 20.0), Rect::new(4.0, 4.0, 120.0, 20.0), "宽被 clamp 到 120");
        assert_eq!(f.child_rect(50.0, 10.0), Rect::new(4.0, 30.0, 50.0, 10.0), "窄子项保持自然");
        // 结算：宽 = fixed_w + 2*pad；高 = 内容自然高
        assert_eq!(f.settle_size(), Vec2::new(120.0 + 8.0, 4.0 + 20.0 + 6.0 + 10.0 + 4.0));
        // 未设 fixed_w 行为不变
        let mut g = Frame::new_stack(PackSide::Top, 6.0, 4.0);
        g.child_rect(300.0, 20.0);
        assert_eq!(g.settle_size(), Vec2::new(308.0, 28.0));
    }

    #[test]
    fn max_w_clamps_children_without_changing_settled_width() {
        // `max_w`（从父级继承的内容最大宽）：子项被 clamp，但**不改写**自然结算宽
        // （容器仍按内容结算 ⇒ 宽度收窄而不是被撑成整宽）。
        let mut f = Frame::new_stack(PackSide::Top, 6.0, 0.0);
        f.set_max_w(Some(100.0));
        assert_eq!(f.child_rect(180.0, 20.0).w, 100.0, "超宽子项被压到 max_w");
        assert_eq!(f.child_rect(60.0, 20.0).w, 60.0, "窄子项保持自然");
        assert_eq!(f.settle_size().x, 100.0, "结算宽 = 最宽子项（= max_w）");
        // 无上限 ⇒ 行为不变
        let mut g = Frame::new_stack(PackSide::Top, 6.0, 0.0);
        assert_eq!(g.child_rect(180.0, 20.0).w, 180.0);
    }

    #[test]
    fn horizontal_row_shrinks_the_last_child_to_the_remaining_width() {
        // **用户实测的 BUG**：`.width()` 较小的窗口里 `row(标签 + 输入框)` 整排突出去
        // ——每个子项各自都没超上限，但**整行**超出了可用宽。给后面的子项的余量必须
        // 扣掉前面已经用掉的（窗口内容宽 276；标签 126 + 间距 9 ⇒ 输入框只该有 141）。
        let mut row = Frame::new_stack(PackSide::Left, 9.0, 0.0);
        row.set_max_w(Some(276.0));
        assert_eq!(row.child_rect(126.0, 30.0).w, 126.0, "第一个子项拿自然宽（余量够）");
        assert_eq!(row.remaining_w(), Some(141.0), "余量 = 276 − (126 + 9)");
        assert_eq!(row.child_rect(300.0, 30.0).w, 141.0, "第二个子项缩到余量");
        assert!(row.settle_size().x <= 276.0, "整行落在可用宽内：{}", row.settle_size().x);
    }

    #[test]
    fn tiny_remainder_keeps_the_child_visible() {
        // 余量太小（< `ADAPT_MIN_W`）⇒ **不压**：宁可溢出可见，也不把控件压成一条线。
        // （`max_w` 的"单个子项不许超内容盒"仍然独立生效——那是另一条规则。）
        let mut row = Frame::new_stack(PackSide::Left, 9.0, 0.0);
        row.set_max_w(Some(276.0));
        row.child_rect(260.0, 20.0);
        assert!(row.remaining_w().unwrap() < 24.0, "前置条件：余量已很小");
        assert_eq!(row.child_rect(200.0, 20.0).w, 200.0, "余量太小 ⇒ 保持自然宽（不消失）");
    }

    #[test]
    fn remaining_w_follows_side_and_padding() {
        // 左推：内容盒 = [pad, pad + max_w]；右推（自右向左长）：余量在光标左侧。
        let mut left = Frame::new_stack(PackSide::Left, 0.0, 4.0);
        left.set_max_w(Some(100.0));
        assert_eq!(left.remaining_w(), Some(100.0), "起点：pad + max_w − pad = max_w");
        left.child_rect(40.0, 10.0);
        assert_eq!(left.remaining_w(), Some(60.0));
        let mut right = Frame::new_stack(PackSide::Right, 0.0, 4.0);
        right.set_max_w(Some(100.0));
        assert_eq!(right.remaining_w(), Some(0.0), "右推起点在内容盒右缘 ⇒ 余量 0");
        // 垂直堆叠没有"同行余量"概念
        let mut top = Frame::new_stack(PackSide::Top, 0.0, 0.0);
        top.set_max_w(Some(100.0));
        assert_eq!(top.remaining_w(), None);
    }

    #[test]
    fn stack_avail_w_takes_the_innermost_constraint() {
        // `Ui::avail_w` 的基础：由内向外找**第一个**给出宽度的 frame —— `row`（无约束）
        // 里面也能看到外层固定宽窗口的内容宽。
        let window = {
            let mut f = Frame::new_stack(PackSide::Top, 6.0, 4.0);
            f.set_fixed_w(120.0); // 内容宽 = 112
            f
        };
        let row = Frame::new_stack(PackSide::Left, 6.0, 0.0); // 自己不设约束
        let frames = [window, row];
        assert_eq!(stack_avail_w(&frames), Some(112.0));
        // 沙箱 / 固定宽都有时，最内层的那个胜（这里是 row 继承来的 max_w）
        let mut row2 = Frame::new_stack(PackSide::Left, 6.0, 0.0);
        row2.set_max_w(Some(50.0));
        assert_eq!(stack_avail_w(&[window, row2]), Some(50.0));
        // 谁都没约束 ⇒ None（自动宽窗口的自然排版）
        assert_eq!(stack_avail_w(&[Frame::new_stack(PackSide::Top, 0.0, 0.0)]), None);
    }
}
