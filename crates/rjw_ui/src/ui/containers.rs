//! 容器控件 API：`UiAdd` trait（全部便捷方法的默认实现）、容器类型
//! （`Panel` / `Pack` / `Grid` / `Window` / `Scroll` / `FlexCtx`）、诊断快照类型，
//! 以及 `Ui` 的容器入口（`pack_at` / `flex_at` / `grid_at` / `min_size` / `max_size`）。
//!
//! 维护者笔记：唯一必需方法是 `UiAdd::ui_mut`；新控件便捷方法 = 在 trait 加一个默认
//! 实现（所有容器自动获得，无需宏）。trait 内方法**不能**加可见性修饰。

use super::*;
use std::ops::RangeInclusive;

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::draw::{Icon, ImageBg, Position, Size, TextAlign, TextVAlign};
use crate::layout::{Child, Frame, PackSide};
use crate::state::{ButtonState, CheckboxState};

impl<'a> Ui<'a> {
    /// 建**窗口**（可重叠 + 焦点置顶 + 可拖拽）：返回 [`WindowBuilder`]，选项链式设置后
    /// 以 `.show(f)` 执行（返回窗口尺寸）。
    ///
    /// **唯一窗口入口**（旧 `window_at` / `window_at_w` / `window_at_strict` /
    /// `window_at_strict_w` 四个变体已删除——选项改由枚举表达，无裸布尔）：
    ///
    /// ```no_run
    /// # use rjw_ui::{Level, Placement, Ui, UiAdd};
    /// # let mut ui: rjw_ui::Ui = todo!();
    /// ui.window("hud")
    ///     .pos(glam::Vec2::new(400.0, 40.0))
    ///     .width(280.0)                       // 可选固定宽（右下角可缩放，跨帧持久）
    ///     .placement(Placement::Clip)         // 可选严格裁剪（默认 Expand = 不裁剪）
    ///     .level(Level::Normal)               // 可选：点击不置顶（默认 Topmost）
    ///     .style(rjw_ui::PanelStyle::default().with_radius(8.0)) // 可选逐窗口样式
    ///     .show(|w| { w.label("HUD"); });
    /// ```
    pub fn window<'s>(&'s mut self, id: &'s str) -> WindowBuilder<'s, 'a> {
        WindowBuilder {
            ui: self,
            id,
            o: WindowOptions::default(),
            title: None,
            close: None,
            collapsible: None,
        }
    }

    /// 建**面板**（背景 + 边框 + 内容垂直堆叠）：返回 [`PanelBuilder`]，链式设置后
    /// 以 `.show(f)` 执行。统一 [`Self::panel_at`] / [`Self::drag_panel_at`]。
    pub fn panel<'s>(&'s mut self) -> PanelBuilder<'s, 'a> {
        PanelBuilder {
            ui: self,
            pos: Position::Logical(Vec2::ZERO),
            drag: None,
            style: None,
        }
    }

    /// 建**滚动容器**（ScrollView）：返回 [`crate::ScrollArea`] builder，链式设置视口位置 /
    /// 尺寸 / 两轴策略后以 `.show(f)` 执行（等价于 [`Self::scroll_axes_at`]，但可读性更好，
    /// 并返回 [`ScrollOutcome`]）。
    ///
    /// ```ignore
    /// ui.scroll_area("log", Size::Logical(Vec2::new(240.0, 320.0)))
    ///     .vscroll(true)      // 竖向滚动条
    ///     .hscroll(true)      // 横向滚动条（长行不折行）
    ///     .show(|s| { s.label("一行很长的内容 …"); });
    /// ```
    pub fn scroll_area<'s>(
        &'s mut self,
        id: &'s str,
        size: impl Into<Size<Vec2>>,
    ) -> crate::widgets::ScrollArea<'s, 'a> {
        crate::widgets::ScrollArea::new(self, id, size.into())
    }

    /// 建**模态对话框**（全屏半透明遮罩 + 对话框）：返回 [`ModalBuilder`]，链式设置后
    /// 以 `.show(f)` 执行。统一 [`Self::modal_at`] / [`Self::modal_at_w`]。
    pub fn modal<'s>(&'s mut self, id: &'s str) -> ModalBuilder<'s, 'a> {
        ModalBuilder {
            ui: self,
            id,
            pos: Position::Logical(Vec2::ZERO),
            width: None,
        }
    }

    /// pack 容器：按 `side` 堆叠，尺寸自动。
    pub fn pack_at(
        &mut self,
        pos: impl Into<Position>,
        side: PackSide,
        f: impl FnOnce(&mut Pack<'_, '_>),
    ) -> Vec2 {        let pos = pos.into().to_physical(self.scale);
        let gap = self.theme.gap;
        self.container(pos, Frame::new_stack(side, gap, 0.0), |ctx| {
            let mut p = Pack { ui: ctx.ui };
            f(&mut p);
        })
        .0
    }

    /// 当前容器**下一子项**的最小尺寸约束（`0` = 该轴不约束；一次性）。
    /// 容器内便捷方法：`p.min_size(120.0, 0.0)`（见 [`crate::Ui`] 文档 / 示例）。
    fn set_next_min(&mut self, min: impl Into<Vec2>) {
        self.frames
            .last_mut()
            .expect("min_size 需在容器内调用（顶层请用 *_at 定位）")
            .set_next_min(min.into());
    }

    /// 当前容器**下一子项**的最大尺寸约束（`0` = 该轴不约束；一次性）。
    fn set_next_max(&mut self, max: impl Into<Vec2>) {
        self.frames
            .last_mut()
            .expect("max_size 需在容器内调用（顶层请用 *_at 定位）")
            .set_next_max(max.into());
    }

    /// **flex 容器**：固定总高 `total_h`（逻辑像素），子项按 `weights` 权重**等分高度**
    /// （扣掉子项间距后按权重分配；权重全 0 时子项高为 0），回调按索引布局——
    /// 同帧精确分配，无需跨帧缓存；返回 `(最大子项宽, total_h)`。
    ///
    /// 子项内可放任意控件（`f.label` / `f.button` 等占光标，高度被强制为分配值）；
    /// 内容超高时**溢出可见**（需要滚动时在子项内嵌 [`Self::scroll_at`]）。
    /// `pos` 相对当前容器内容原点（顶层即屏幕原点），不占父容器光标。
    pub fn flex_at<F>(
        &mut self,
        pos: impl Into<Position>,
        total_h: impl Into<Size<f32>>,
        weights: &[u32],
        mut f: F,
    ) -> Vec2
    where
        F: FnMut(&mut FlexCtx<'_, '_>, usize),
    {
        let pos = pos.into().to_physical(self.scale);
        let total_h = total_h.into().to_physical(self.scale);
        let gap = self.theme.gap;
        let start = self.painter.q.queue.len();
        self.begin_top_placement();
        let saved_base = self.abs_base;
        self.abs_base = saved_base + pos;
        let mut frame = Frame::new_stack(PackSide::Top, gap, 0.0);
        frame.set_fixed_h(total_h);
        self.frames.push(frame);
        self.painter.q.depth += 1;
        let sum: u32 = weights.iter().sum();
        let gaps = gap * weights.len().saturating_sub(1) as f32;
        let usable = (total_h - gaps).max(0.0);
        {
            let mut fc = FlexCtx { ui: self };
            for (i, &w) in weights.iter().enumerate() {
                let h = if sum > 0 {
                    usable * w as f32 / sum as f32
                } else {
                    0.0
                };
                fc.ui.frames.last_mut().expect("flex frame").force_next_h(h);
                f(&mut fc, i);
            }
        }
        let frame = self.frames.pop().expect("flex frame");
        let size = frame.settle_size();
        let inner_bounds = frame.content_bounds();
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        for d in &mut self.painter.q.queue[start..] {
            d.translate(pos);
        }
        if let Some(parent) = self.frames.last_mut() {
            parent.note_content(Rect::new(pos.x, pos.y, size.x, size.y));
            if let Some(ib) = inner_bounds {
                parent.note_content(Rect::new(ib.x + pos.x, ib.y + pos.y, ib.w, ib.h));
            }
        }
        size
    }

    /// **自动换行的水平行**（占光标；**裸 `Ui`** 上的入口 —— 容器闭包里用
    /// [`UiAdd::row_wrap`]）：`max_w` = **行宽上限**，塞不下就收行。
    ///
    /// ```no_run
    /// # use rjw_ui::{Ui, UiAdd};
    /// # fn demo(ui: &mut Ui) {
    /// ui.row_wrap(240.0, |r| {
    ///     for t in ["标签一", "标签二", "标签三"] {
    ///         r.button(t, t);
    ///     }
    /// });
    /// # }
    /// ```
    pub fn row_wrap(
        &mut self,
        max_w: impl Into<Size<f32>>,
        f: impl FnOnce(&mut Pack<'_, '_>),
    ) -> Vec2 {
        RowBuilder {
            ui: self,
            min_h: None,
            max_h: None,
            gap: None,
            pad: 0.0,
            wrap: true,
            wrap_w: Some(max_w.into()),
            line_gap: None,
        }
        .show(f)
    }

    /// grid 容器：`cols` 列均匀网格，单元格尺寸跨帧缓存（`id` 须稳定）。
    pub fn grid_at(
        &mut self,
        pos: impl Into<Position>,
        cols: usize,
        id: &str,
        f: impl FnOnce(&mut Grid<'_, '_>),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        assert!(cols > 0, "grid cols must be > 0");
        // grid 容器是命名空间边界：内部子控件（如背包格子按钮）ID 自动带前缀。
        let abs = self.id_for(id);
        let cell = self
            .state
            .grid_cells
            .get(abs.as_str())
            .copied()
            .unwrap_or(Vec2::ZERO);
        let mut result = (Vec2::ZERO, Vec2::ZERO);
        self.with_id(id, |ui| {
            result = ui.container(pos, Frame::new_grid(cols, cell, 0.0), |ctx| {
                let mut g = Grid { ui: ctx.ui };
                f(&mut g);
            });
        });
        let (size, max_child) = result;
        // 回写单元格缓存：**内容变化时同时允许扩大与缩小**（未达到 max 的控件按
        // 内容自动改大小——如背包格子文字变长/变短；内容不变时布局依旧跨帧稳定）。
        self.state.grid_cells.insert(abs.to_static(), max_child);
        size
    }

    // ── 提交 ─────────────────────────────────────────────────
}

/// **容器控件 API**：全部容器包装（[`Panel`] / [`Pack`] / [`Grid`] / [`Window`] /
/// [`Scroll`] / [`FlexCtx`]）共享的便捷方法，**替代旧的 `widget_api!` 宏**。
///
/// - 唯一必需方法 [`UiAdd::ui_mut`]（返回容器持有的 `Ui`）——**新容器只需一行 impl**
///   即可获得全部方法；
/// - 全部便捷方法都是**默认方法**（占光标、内容自动尺寸；`*_at` 变体为显式尺寸
///   逃生舱）——**新增控件便捷方法 = 在本 trait 加一个默认方法**，所有容器自动获得，
///   无需改宏；
/// - 顶层（无容器）请用 `Ui` 的 `*_at` 绝对定位方法（如 [`Ui::add_at`] /
///   [`Ui::label_at`]）。
pub trait UiAdd<'a> {
    /// 容器持有的 `Ui`（包装字段，仅本 crate 内实现）。
    fn ui_mut(&mut self) -> &mut Ui<'a>;

    /// 在容器内**占光标**放置 [`crate::widgets::Widget`] 控件（尺寸由控件自己在
    /// `ui()` 里申请，见 [`Ui::allocate`]）。
    fn add(&mut self, w: impl crate::widgets::Widget) -> crate::widgets::Response {
        self.ui_mut().add(w)
    }

    /// **绝对定位**放置 [`crate::widgets::Widget`] 控件（`pos` 相对当前容器内容原点；
    /// 不占光标）。
    fn add_at(
        &mut self,
        pos: impl Into<Position>,
        w: impl crate::widgets::Widget,
    ) -> crate::widgets::Response {
        self.ui_mut().add_at(pos, w)
    }

    /// 在容器内建**窗口**（可重叠 + 置顶 + 可拖拽；[`WindowBuilder`] 链，`.show(f)` 执行）。
    fn window<'s>(&'s mut self, id: &'s str) -> WindowBuilder<'s, 'a> {
        self.ui_mut().window(id)
    }

    /// 在容器内建**面板**（背景 + 边框；[`PanelBuilder`] 链，`.show(f)` 执行）。
    fn panel<'s>(&'s mut self) -> PanelBuilder<'s, 'a> {
        self.ui_mut().panel()
    }

    /// 在容器内建**滚动容器**（[`crate::ScrollArea`] 链，`.show(f)` 执行）。
    fn scroll_area<'s>(
        &'s mut self,
        id: &'s str,
        size: impl Into<Size<Vec2>>,
    ) -> crate::widgets::ScrollArea<'s, 'a> {
        self.ui_mut().scroll_area(id, size)
    }

    /// 标签（占光标，内容自然尺寸；默认 `LimitedInParent`——在父级可用宽内
    /// **自动换行**，Resizable 窗口缩窄后不溢出）。
    fn label(&mut self, text: &str) -> Vec2 {
        let ui = self.ui_mut();
        // 尺寸由控件申请；`Response::rect` 就是它最终占的矩形（见 `Widget::ui`）。
        let resp = crate::widgets::Widget::ui(crate::widgets::Label::new(text), ui);
        resp.rect.size()
    }

    /// 绝对定位标签（`pos` 相对当前容器内容原点）。
    fn label_at(&mut self, pos: impl Into<Position>, text: &str) -> Vec2 {
        self.ui_mut().label_at(pos, text)
    }

    /// **矢量图标**（占光标）：容器内按当前游标放置，方块 `size`（等比缩放笔画）。
    /// 与 [`crate::Ui::icon`] 同语义——`row` 内连续调用即得一条工具栏。
    fn icon(&mut self, size: impl Into<Size<Vec2>>, icon: Icon, color: Color) {
        self.ui_mut().icon(size, icon, color)
    }

    /// **矢量图标**（绝对定位；`pos` 相对当前容器内容原点）。
    fn icon_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        icon: Icon,
        color: Color,
    ) {
        self.ui_mut().icon_at(pos, size, icon, color)
    }

    /// **背景图**（占光标）：容器内按当前游标放置，尺寸 `size`。
    fn image(&mut self, size: impl Into<Size<Vec2>>, bg: ImageBg) {
        self.ui_mut().image(size, bg)
    }

    /// **背景图**（绝对定位；`pos` 相对当前容器内容原点）。
    fn image_at(&mut self, pos: impl Into<Position>, size: impl Into<Size<Vec2>>, bg: ImageBg) {
        self.ui_mut().image_at(pos, size, bg)
    }

    /// **自动换行标签**（占光标）：`max_w` 逻辑像素内按词/字换行；
    /// 返回自然尺寸（宽 = min(自然宽, max_w)，高 = 行数 × 行高）。
    /// `max_w <= 0` = 不换行（同 `label`）。
    fn label_wrap(&mut self, max_w: f32, text: &str) -> Vec2 {
        let ui = self.ui_mut();
        let style = ui.theme.label.clone();
        let size = ui.text_size_wrap(text, style.font_size, style.font_family.as_deref(), max_w);
        let rect = ui.child_rect(size.x, size.y, Child::Expand);
        ui.painter().text(
            rect,
            text,
            style.font_size,
            style.color,
            style.font_family.clone(),
            TextAlign::from(style.align),
            TextVAlign::Center,
            None,
            None,
        );
        size
    }

    /// **水平行容器**（占光标）：子项按 [`PackSide::Left`] 水平堆叠
    /// （`{Label} {Input} {Button}` 排列），整体在父容器（垂直 pack 等）中**占一行**：
    /// 宽 = 子项结算、撑大父级。
    ///
    /// **行高**（= 默认形态的 `row_builder`）：
    /// - **单行子项**（[`crate::widgets::SizeClass::SingleLine`]，默认）被**钉到标准行高**
    ///   [`Theme::row_h`]，各自内容垂直居中 → 文字中心线对齐（Label 不偏上）；
    /// - **多行子项**（`TextEditor::multiline()`）以标准行高为**下限**，可以**撑高整行**；
    /// - 子项**左上角对齐、沿 X 推进**；行高随最高的子项长。
    ///
    /// 要自定义行高上下限 / 间距 / 内边距用 [`Self::row_builder`]。
    fn row(&mut self, f: impl FnOnce(&mut Pack<'_, '_>)) -> Vec2 {
        self.row_builder().show(f)
    }

    /// **水平行 Builder**（占光标）：`row(f)` 的可配置形态——行高下限 / 上限 / 子项间距 /
    /// 行内边距，最后 `.show(|r| ..)` 执行并返回结算尺寸（见 [`crate::RowBuilder`]）。
    ///
    /// ```ignore
    /// // ⚠ `ignore`：`rjw_ui` 的 doctest 里拿不到"容器"——应用侧是
    /// // `Frame::ui()` 给的 `UiSession`（它实现本 trait），`Ui` 自身不实现 `UiAdd`
    /// // （所以这里不能用 `&mut Ui` 调 trait 方法）。行为验收见 `--sim-ta-resize`。
    /// ui.row_builder().height(60.0).show(|r| { r.label("高一行"); });
    /// ui.row_builder().min_h(60.0).max_h(200.0).pad(6.0).gap(12.0).show(|r| { r.label("…"); });
    /// ```
    fn row_builder(&mut self) -> RowBuilder<'_, 'a> {
        RowBuilder {
            ui: self.ui_mut(),
            min_h: None,
            max_h: None,
            gap: None,
            pad: 0.0,
            wrap: false,
            wrap_w: None,
            line_gap: None,
        }
    }

    /// **自动换行的水平行**（占光标）：`max_w` = **行宽上限**，塞不下就收行（行内左上角对齐、
    /// 行间距默认 = `gap`）。等价 `row_builder().wrap_w(max_w).show(f)`。
    ///
    /// 与 [`UiAdd::row`] 的区别只有"有宽度上限"这一条：**只换行、不压缩**（开启后不再报
    /// 行内剩余宽 ⇒ `Label` 这类子项按整行宽排版、塞不下换行，而不是被压扁）。
    ///
    /// ```ignore
    /// // ⚠ `ignore`：同 `row_builder`（容器闭包里才拿得到 `UiAdd`）。
    /// ui.row_wrap(240.0, |r| { for t in tags { r.button(t, t); } });
    /// ```
    fn row_wrap(
        &mut self,
        max_w: impl Into<Size<f32>>,
        f: impl FnOnce(&mut Pack<'_, '_>),
    ) -> Vec2 {
        self.row_builder().wrap_w(max_w).show(f)
    }

    /// **ID 命名空间区块**（占光标；[`Ui::namespace`] 的转发）：正文录在当前光标处、
    /// 只给内部控件加一层 ID 前缀；返回正文结算尺寸。
    ///
    /// ```ignore
    /// // ⚠ `ignore`：同 `row_builder`（容器闭包里才拿得到 `UiAdd`）。
    /// p.namespace("left", |ui| { ui.text_input("kw", &mut kw); });   // 状态键 = "left/kw"
    /// ```
    fn namespace(&mut self, id: &str, body: impl FnOnce(&mut PackEntry<'_, '_>)) -> glam::Vec2 {
        self.ui_mut().namespace(id, body)
    }

    /// **可收缩区块**（占光标；[`Ui::foldable`] 的转发）：一行标题 + 可折叠正文。
    ///
    /// ```ignore
    /// // ⚠ `ignore`：同 `row_builder`（容器闭包里才拿得到 `UiAdd`）。
    /// p.foldable("perf", "性能统计").show(|ui| { ui.label("FPS 60"); });
    /// p.foldable("advanced", "高级").open(true).show(|ui| { /* 首次就展开 */ });
    /// ```
    fn foldable<'s>(&'s mut self, id: &str, label: &str) -> crate::Foldable<'s, 'a> {
        crate::Foldable::new(self.ui_mut(), id, label)
    }

    /// **可收缩区块（自定义标题）**（占光标；[`Ui::foldable_custom`] 的转发）：标题里可放
    /// 任意控件（它们自己认领按下 ⇒ 点它们不会连带折叠标题）。
    ///
    /// ```ignore
    /// // ⚠ `ignore`：同 `row_builder`（容器闭包里才拿得到 `UiAdd`）。
    /// p.foldable_custom("filters", |t| {
    ///     t.label("过滤");
    ///     t.checkbox_mut("all", "全选", &mut all);
    /// }).show(|ui| { ui.text_input("kw", &mut kw); });
    /// ```
    fn foldable_custom<'s>(
        &'s mut self,
        id: &str,
        title: impl FnOnce(&mut PackEntry<'_, '_>) + 's,
    ) -> crate::Foldable<'s, 'a> {
        crate::Foldable::custom(self.ui_mut(), id, title)
    }

    /// **扩展标签 builder**（占光标；`Ui::label_ex` 的转发）：需要字重 / 斜体 / 字距 /
    /// 行高 / **渐变**这类逐标签样式时用它。
    ///
    /// ```ignore
    /// // ⚠ `ignore`：同 `row_builder`（容器闭包里才拿得到 `UiAdd`）。
    /// ui.label_ex("标题").weight(Weight::BOLD).gradient(Color::RED, Color::YELLOW).show(ui);
    /// ```
    fn label_ex<'s>(&mut self, text: &'s str) -> crate::widgets::LabelEx<'s> {
        crate::widgets::LabelEx::new(text)
    }

    /// **彩色标签**（占光标；一步到位）：`ui.label_ex(text).tint(color).show(ui)` 的糖。
    ///
    /// ```ignore
    /// // ⚠ `ignore`：同 `row_builder`（容器闭包里才拿得到 `UiAdd`）。
    /// ui.colored_label("HP 100", Color::GREEN);
    /// ```
    fn colored_label(&mut self, text: &str, color: Color) -> crate::widgets::Response {
        self.ui_mut().colored_label(text, color)
    }

    /// **分割线**（占光标）：容器内占一行（高 = 线厚 + 上下留白），水平线宽 =
    /// **容器内容宽**（= "分割整个容器"：线段从内容盒左缘到右缘，两端各内缩一个内边距）。
    ///
    /// 宽度**不是**录制那一刻能算出来的：自动宽窗口是布局根（`avail_w()` 恒 `None`）。
    /// 所以这里先用保守宽度占位（`avail_w` → 当前最宽子项 → 120），并把线段标成
    /// **待定满宽**（[`Ui::divider_full_w`]）—— 所属容器（窗口 / panel / pack…）结算尺寸后
    /// 回填成真实内容宽。**深度 0（`win=0` 顶层）不标记**：那里没有"容器内容宽"这回事，
    /// 拿根 frame 的固定宽（视口宽）会把线拉成整屏（历史 BUG）。
    fn divider(&mut self) {
        let ui = self.ui_mut();
        let st = ui.theme.divider.clone();
        // **保守宽 = 同容器"已排布内容"的最宽宽**（"其他内容有多宽，线就多宽"），本容器还
        // 没内容时才回落到可用宽（"只有一条线的容器"维持旧观感：铺满可用宽）。
        //
        // ⚠ **不能一律用 `avail_w()`**：带 `vscroll` 的窗口里 divider 跑在**滚动视口**内，
        // 那里的 `avail_w()` 是**视口宽** —— 而自动宽窗口的视口宽首帧会引导到"屏幕剩余宽"
        // （`window_impl` 里为了不让视口塌成 1px 的引导值）。线一旦按它铺满，就把自己算进了
        // "内容宽" ⇒ 下一帧视口宽仍按内容反推 ⇒ **正反馈锁定**：窗口一打开就被撑到屏幕大小
        // （用户实测："分割线会默认水平撑开到屏幕大小，我们当然不希望一开始就被撑开得这么大，
        // 而是其他内容有多少就该多宽"）。
        let content_w = ui
            .frames
            .last()
            .map(|f| f.max_child_w())
            .filter(|&w| w > 0.0);
        let w = content_w.or_else(|| ui.avail_w()).unwrap_or(120.0);
        let h = st.thickness + st.margin * 2.0;
        // **宽度铺满、但不计入容器宽**（`Child::Fill`）：分割线是**整格装饰**，绝不能成为
        // 容器宽的决定者 —— 让它参与结算会在带 `vscroll` 的自动宽窗口里形成**正反馈锁定**
        // （视口宽首帧取"屏幕剩余宽" ⇒ 线按它铺满 ⇒ 内容宽 = 视口宽 ⇒ 下一帧照旧 ⇒ 窗口一开
        // 就被撑到屏幕大小）。用 `Fill` 后：**其他内容**独自决定容器宽，线跟着铺满即可。
        // 容器结算后 [`Ui::divider_full_w`] 仍会把线段回填到**内容盒右缘**（= 其他内容宽）。
        let rect = ui.child_rect(w, h, Child::Fill);
        let pos = Position::Physical(Vec2::new(rect.x, rect.y));
        if ui.painter.q.depth > 0 {
            ui.divider_full_w(pos, Size::Physical(w));
        } else {
            ui.divider_at(pos, Size::Physical(w));
        }
    }

    /// **多行文本输入框**（占光标；默认约 200×90，可 `text_area_at` 显式尺寸）。
    /// Enter 换行、↑/↓ 跨行、Home/End 行首尾；自动换行 + 垂直滚动；选择/复制/
    /// 粘贴/剪切（Ctrl+C/V/X）；IME 支持。返回 `()`（内容写回 `value`）。
    ///
    /// 属性（尺寸 / 缩放柄 / 样式 / 不自动换行）请用责任链
    /// [`crate::widgets::TextEditor`]（本方法 = `ui.add(TextEditor::new(id, value).multiline())`）。
    fn text_area(&mut self, id: &str, value: &mut String) {
        let ui = self.ui_mut();
        crate::widgets::Widget::ui(crate::widgets::TextEditor::new(id, value).multiline(), ui);
    }

    /// **多行文本输入框**（显式 `Rect`）。
    fn text_area_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        self.ui_mut().text_area_at(id, rect, value);
    }

    /// **多行文本输入框（不自动换行）**（占光标；默认约 200×90）：行宽不限
    /// （显式 `\n` 分行），超出内容区**水平滚动**跟随光标；垂直滚动/选择/IME
    /// 与 [`UiAdd::text_area`] 一致。
    fn text_area_nw(&mut self, id: &str, value: &mut String) {
        let ui = self.ui_mut();
        crate::widgets::Widget::ui(
            crate::widgets::TextEditor::new(id, value)
                .multiline()
                .no_wrap(),
            ui,
        );
    }

    /// **多行文本输入框（不自动换行）**（显式 `Rect`）。
    fn text_area_at_nw(&mut self, id: &str, rect: Rect, value: &mut String) {
        self.ui_mut().text_area_at_nw(id, rect, value);
    }

    /// **下一子项的最小尺寸约束**（`0` = 该轴不约束；一次性，作用于紧接着的下一个子项）。
    fn min_size(&mut self, w: f32, h: f32) {
        self.ui_mut().set_next_min(glam::Vec2::new(w, h));
    }

    /// **下一子项的最大尺寸约束**（`0` = 该轴不约束；一次性，作用于紧接着的下一个子项）。
    fn max_size(&mut self, w: f32, h: f32) {
        self.ui_mut().set_next_max(glam::Vec2::new(w, h));
    }

    /// **下拉框**（占光标，自动尺寸）：按钮 + 展开选项浮层；返回本帧新选中索引。
    fn combo(
        &mut self,
        id: &str,
        current: &str,
        options: &[String],
        selected: Option<u32>,
    ) -> Option<u32> {
        let ui = self.ui_mut();
        let style = ui.theme.button.clone();
        let tsize = ui.text_size(current, style.font_size, style.font_family.as_deref());
        let w = (tsize.x + 20.0).max(90.0) + style.padding.x * 2.0;
        let h = style.padding.y * 2.0 + tsize.y;
        let rect = ui.child_rect(w, h, Child::Expand);
        ui.combo_at(id, rect, current, options, selected)
    }

    /// 按钮（文本 + padding 自动尺寸）。
    fn button(&mut self, id: &str, label: &str) -> ButtonState {
        let ui = self.ui_mut();
        let style = ui.theme.button.clone();
        let tsize = ui.text_size(label, style.font_size, style.font_family.as_deref());
        let size = Vec2::new(
            tsize.x + style.padding.x * 2.0,
            tsize.y + style.padding.y * 2.0,
        );
        let rect = ui.child_rect(size.x, size.y, Child::Expand);
        // 直接走 styled 变体：避免 button_at 内再 clone 一次样式。
        ui.button_at_styled(id, rect, label, &style)
    }

    /// 显式尺寸按钮（逃生舱）。
    fn button_at(&mut self, id: &str, rect: Rect, label: &str) -> ButtonState {
        self.ui_mut().button_at(id, rect, label)
    }

    /// 滑块（自动尺寸：高度固定，宽度取样式最小宽）。
    fn slider(&mut self, id: &str, range: RangeInclusive<f32>, value: f32) -> f32 {
        let ui = self.ui_mut();
        let style = ui.theme.slider.clone();
        let size = Vec2::new(style.min_w.max(40.0), style.height);
        let rect = ui.child_rect(size.x, size.y, Child::Expand);
        ui.slider_at(id, rect, range, value)
    }

    /// 显式尺寸滑块（逃生舱）。
    fn slider_at(&mut self, id: &str, rect: Rect, range: RangeInclusive<f32>, value: f32) -> f32 {
        self.ui_mut().slider_at(id, rect, range, value)
    }

    /// 勾选框（勾选值由用户维护，返回含 `toggled` 的状态）。
    fn checkbox(&mut self, id: &str, label: &str, checked: bool) -> CheckboxState {
        let ui = self.ui_mut();
        let style = ui.theme.checkbox.clone();
        let tsize = ui.text_size(label, style.font_size, style.font_family.as_deref());
        let size = Vec2::new(
            style.box_size + style.gap + tsize.x,
            style.box_size.max(tsize.y),
        );
        let rect = ui.child_rect(size.x, size.y, Child::Expand);
        ui.checkbox_at(id, rect, label, checked)
    }

    /// 显式尺寸勾选框（逃生舱）。
    fn checkbox_at(&mut self, id: &str, rect: Rect, label: &str, checked: bool) -> CheckboxState {
        self.ui_mut().checkbox_at(id, rect, label, checked)
    }

    /// 勾选框（**状态自持**）：`checked` 由调用方持有，点击时本方法**直接翻转**，
    /// 无需手动 `toggled()` 维护。
    ///
    /// `id` 灵活指定（[`crate::widgets::WidgetId`]，经 [`From`] 转换）：
    /// - `None` → 以 `label` 文本为 ID（同容器内标签唯一时最简）；
    /// - `Some("id")` / `"id"` → 显式字符串 ID；
    /// - `42u64` → 数字 ID（如列表行索引 `i as u64`）。
    ///
    /// 用法：`w.checkbox_mut(None, "窗口 A 选项", &mut self.win_a_checked);`
    fn checkbox_mut<'x>(
        &mut self,
        id: impl Into<crate::widgets::WidgetId<'x>>,
        label: &str,
        checked: &mut bool,
    ) -> CheckboxState {
        let id = id.into().resolve(label);
        let st = self.checkbox(&id, label, *checked);
        if st.toggled() {
            *checked = !*checked;
        }
        st
    }

    /// 单选（同组 ID 互斥；返回 `checked` / `toggled`）。
    fn radio(&mut self, id: &str, group: &str, label: &str) -> CheckboxState {
        let ui = self.ui_mut();
        let style = ui.theme.checkbox.clone();
        let tsize = ui.text_size(label, style.font_size, style.font_family.as_deref());
        let size = Vec2::new(
            style.box_size + style.gap + tsize.x,
            style.box_size.max(tsize.y),
        );
        let rect = ui.child_rect(size.x, size.y, Child::Expand);
        ui.radio_at(id, group, rect, label)
    }

    /// 显式尺寸单选（逃生舱）。
    fn radio_at(&mut self, id: &str, group: &str, rect: Rect, label: &str) -> CheckboxState {
        self.ui_mut().radio_at(id, group, rect, label)
    }

    /// 文本输入框（内容写入 `value`；自动尺寸：高度固定，宽度取样式最小宽）。
    ///
    /// 等价 `ui.add(TextEditor::new(id, value))`——宽 / 高 / 缩放柄 / 样式请用
    /// [`crate::widgets::TextEditor`] 责任链。
    fn text_input(&mut self, id: &str, value: &mut String) {
        let ui = self.ui_mut();
        crate::widgets::Widget::ui(crate::widgets::TextEditor::new(id, value), ui);
    }

    /// 显式尺寸文本输入框（逃生舱）。
    fn text_input_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        self.ui_mut().text_input_at(id, rect, value);
    }

    /// 嵌套面板（`pos` 相对当前容器内容原点；不占光标）。
    fn panel_at(&mut self, pos: impl Into<Position>, f: impl FnOnce(&mut Panel<'_, '_>)) -> Vec2 {
        self.ui_mut().panel_at(pos, f)
    }

    /// 嵌套**可拖拽**面板（位置持久于 `UiState.panel_pos`）。
    fn drag_panel_at(
        &mut self,
        id: &str,
        pos: impl Into<Position>,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().drag_panel_at(id, pos, f)
    }

    /// **嵌套模态对话框**（全屏遮罩 + 对话框，背后交互被阻断）——责任链 builder。
    fn modal<'s>(&'s mut self, id: &'s str) -> ModalBuilder<'s, 'a> {
        self.ui_mut().modal(id)
    }

    /// 嵌套 pack（`pos` 相对当前容器内容原点；不占光标）。
    fn pack_at(
        &mut self,
        pos: impl Into<Position>,
        side: PackSide,
        f: impl FnOnce(&mut Pack<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().pack_at(pos, side, f)
    }

    /// 嵌套 grid（`pos` 相对当前容器内容原点；不占光标）。
    fn grid_at(
        &mut self,
        pos: impl Into<Position>,
        cols: usize,
        id: &str,
        f: impl FnOnce(&mut Grid<'_, '_>),
    ) -> Vec2 {
        self.ui_mut().grid_at(pos, cols, id, f)
    }
}

/// 容器闭包上下文（内部类型）。
pub(crate) struct ContainerCtx<'ui, 'a> {
    pub(crate) ui: &'ui mut Ui<'a>,
}

/// 面板容器（背景 + 边框 + 垂直堆叠内容）。
pub struct Panel<'ui, 'a> {
    pub(super) ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> UiAdd<'a> for Panel<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

/// pack 容器（无背景，纯布局）。
pub struct Pack<'ui, 'a> {
    pub(super) ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> Pack<'ui, 'a> {
    /// **在容器闭包里构造**（组合控件 / 骨架用；应用侧要 pack 走 [`UiAdd::row`]）。
    ///
    /// 唯一用户是 [`Ui::menu_bar`]：它把"一行 + 全宽背景"的栏做成
    /// `ui.container(pos, Frame::new_stack(PackSide::Left, gap, 0.0), ..)` 里的这个 `Pack`，
    /// 再交给 [`crate::widgets::MenuBar`]（后者 `Deref` 到 `Pack`，于是 `bar.add(..)` /
    /// `bar.button(..)` / `bar.label(..)` / `bar.text_input(..)` 全部直接可用）。
    pub(crate) fn new(ui: &'ui mut Ui<'a>) -> Self {
        Self { ui }
    }
}
impl<'ui, 'a> UiAdd<'a> for Pack<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

/// grid 容器（无背景，均匀网格）。
pub struct Grid<'ui, 'a> {
    pub(super) ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> UiAdd<'a> for Grid<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

/// 窗口容器（可重叠 + 焦点置顶 + 可拖拽；见 [`Ui::window`]）。
pub struct Window<'ui, 'a> {
    pub(super) ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> UiAdd<'a> for Window<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

/// 滚动容器（内容在可视区内堆叠 + 滚动；见 [`Ui::scroll_at`] / [`crate::ScrollArea`]）。
///
/// ⚠ 字段私有、且**只由 [`Ui::scroll_axes_at`] 构造**（`widgets::ScrollArea` 只是把闭包
/// 透传过去）——外部没有"自己拼一个 `Scroll`"的口子，于是沙箱的进入/退出永远成对。
pub struct Scroll<'ui, 'a> {
    pub(super) ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> UiAdd<'a> for Scroll<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

/// **flex 容器上下文**（[`Ui::flex_at`]）：子项高度已按权重分配（强制），
/// 内部可调用任意控件方法占光标（`f.label` / `f.button` 等）。
pub struct FlexCtx<'ui, 'a> {
    pub(super) ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> UiAdd<'a> for FlexCtx<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}

/// **区块正文上下文**（[`Foldable::show`](crate::Foldable::show) /
/// [`Ui::namespace`] 的闭包参数）。
///
/// **为什么是一个包装体而不是裸 `&mut Ui`**：正文里的代码要能用 [`UiAdd`] 的整套方法
/// （`label` / `row` / `button` / 嵌套 `foldable` …）——`Ui` 自身**不实现** `UiAdd`
/// （见 trait 文档），所以用户闭包必须拿到实现了它的类型。这与窗口 / 面板 / 滚动容器 /
/// `row` 的闭包形状一致（都是"容器包装 + `UiAdd`"）。
///
/// 它**不是布局容器**：不推帧、不加内边距、不占额外光标（[`Ui::namespace`] 的语义就是
/// "只加 ID 前缀"；[`Foldable`](crate::Foldable) 的正文与标题行同级）。需要 `Ui` 的固有
/// 方法（`label_at` / `add_at` / `state` / `avail_w` …）时经
/// [`Deref`](std::ops::Deref) / [`DerefMut`](std::ops::DerefMut) 直接透出。
pub struct PackEntry<'ui, 'a> {
    pub(super) ui: &'ui mut Ui<'a>,
}
impl<'ui, 'a> PackEntry<'ui, 'a> {
    /// 构造（[`Foldable::show`](crate::Foldable::show) / [`Ui::namespace`] 内部用）。
    pub(crate) fn new(ui: &'ui mut Ui<'a>) -> Self {
        Self { ui }
    }
}
impl<'ui, 'a> UiAdd<'a> for PackEntry<'ui, 'a> {
    fn ui_mut(&mut self) -> &mut Ui<'a> {
        self.ui
    }
}
/// 透出 `Ui` 的固有方法（`label_at` / `add_at` / `state` / `avail_w` …）——`r.label(..)`
/// 走 [`UiAdd`]，`r.label_at(..)` / `r.avail_w()` 走这里（与 `MenuBar` 的 `Deref` 同一理由）。
impl<'ui, 'a> std::ops::Deref for PackEntry<'ui, 'a> {
    type Target = Ui<'a>;
    #[inline]
    fn deref(&self) -> &Ui<'a> {
        self.ui
    }
}
impl std::ops::DerefMut for PackEntry<'_, '_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ui
    }
}

#[derive(Clone, Debug)]
pub struct UiWindowInfo {
    /// 窗口 **绝对 ID**。
    pub id: String,
    /// z 序（越大越上）。
    pub z: u32,
    /// **本帧提交原点**（物理像素）—— 即 `win_origins[z]`。
    ///
    /// ⚠ 它是**相对直接容器**的原点（顶点管线"采集时减去本窗口 origin、提交时再加回来"，
    /// 所以两者一致 ⇒ 渲染正确）：
    /// - **顶层窗口**（`abs_base = 0` 处录制）= **屏幕坐标**，直接可用；
    /// - **嵌套窗口**（如对话框里的下拉浮层）**不是**屏幕坐标 —— 想用它算屏幕位置，
    ///   要把每层外层窗口的 `origin` 累加（实测：对话框内下拉 dump 里是 `(14,162)`，
    ///   屏幕上在 `(704,492)` = 对话框 `(690,330)` + 它）。
    pub origin: Vec2,
    /// 结算尺寸（物理像素）。
    pub size: Vec2,
    /// 本帧是否处于拖拽激活态。
    pub dragging: bool,
    /// 拖拽按下时的窗口左上角（`None` = 本帧无拖拽基准）。
    pub press_panel: Option<Vec2>,
    /// **跨帧持久位置**（`UiState::panel_pos`；`None` = 从未拖过，用传入 `pos`）。
    pub stored_pos: Option<Vec2>,
    /// **本帧实际提交用的平移量**（`flush_seg` 记录）。
    ///
    /// 与 `origin` 不一致 ⇒ "引擎状态 vs 视觉"不一致（渲染/变换路径问题；
    /// 历史 bug：Mesh/quads 命令忽略 `.transform(..)` ⇒ 提交平移恒为 0，窗口全在左上角）。
    pub submit_pos: Option<Vec2>,
    /// **本窗最近一次提交的批次 scissor**（屏幕物理像素；`None` = 该窗内容不裁剪）。
    ///
    /// 引擎把环境裁剪层（严格窗口 / 滚动可视区 / Clip 沙箱 / 文本框盒）作为 **batch
    /// scissor** 交给 GPU（不再切割几何）——这个字段就是"到底裁到哪"的直接证据
    /// （见 [`crate::UiBatch::clip`] 与 `docs/ENGINE_GUIDE.md` §18.20）。
    pub clip: Option<Rect>,
}

/// **UI 引擎状态快照**（[`Ui::debug_dump`]；`Display` 为单行可 grep 格式）。
#[derive(Clone, Debug)]
pub struct UiDebugDump {
    pub frame: u64,
    /// DPI scale（物理 / 逻辑）。
    pub scale: f32,
    /// 视口物理尺寸。
    pub viewport: Vec2,
    /// 鼠标物理屏幕坐标。
    pub mouse_px: Vec2,
    pub mouse_in_window: bool,
    /// 键盘焦点（绝对 ID）。
    pub focused: Option<String>,
    /// **文本焦点**（只有文本控件持焦点才非 `None`）。
    pub text_focus: Option<String>,
    pub windows: Vec<UiWindowInfo>,
}

impl std::fmt::Display for UiDebugDump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ui[frame={} scale={:.2} viewport=({:.0},{:.0}) mouse=({:.0},{:.0}) in_win={} focus={:?} text_focus={:?}]",
            self.frame,
            self.scale,
            self.viewport.x,
            self.viewport.y,
            self.mouse_px.x,
            self.mouse_px.y,
            self.mouse_in_window,
            self.focused,
            self.text_focus,
        )?;
        for w in &self.windows {
            write!(
                f,
                " | {} z={} origin=({:.0},{:.0}) submit={:?} clip={:?} size=({:.0},{:.0}) drag={} press={:?} stored={:?}",
                w.id, w.z, w.origin.x, w.origin.y, w.submit_pos, w.clip, w.size.x, w.size.y, w.dragging, w.press_panel, w.stored_pos
            )?;
        }
        Ok(())
    }
}
