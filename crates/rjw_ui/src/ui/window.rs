//! 窗口 / 模态 / 菜单栏容器实现，以及窗口几何纯函数（溢出策略 / 裁剪 / 限位）。
//!
//! 维护者笔记：`window_impl` 是 `ui.rs` 里最大的单个函数（一屏内完成位置求解 →
//! 录制内容 → 缩放柄 → 按下裁决），改动前先读 `docs/UI_ARCHITECTURE.md` §5.6 与
//! `docs/ENGINE_GUIDE.md` 的窗口尺寸判定表。标题栏在 `ui::chrome`。

use super::*;
use super::chrome::{TITLE_BUTTON_INSET, title_bar_h, title_bar_layout, window_title_bar};
use super::cmds::{clear_drag_base, resolve_drag};

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::Rect;

use crate::draw::{
    CornerRadius, DrawKind, Position, Size,
    UiDraw,
};
use crate::hit::{
    hit_test, window_occluded,
};
use crate::id::IdAbsolute;
use crate::layout::{Child, Frame, PackSide};
use crate::style::PanelStyle;

impl<'a> Ui<'a> {
    /// **窗口**容器实现（**非公开**：公开入口是 [`Self::window`] 责任链 builder）。
    ///
    /// - **可重叠**：多个窗口按 **z-order** 排列（`UiState.window_z`），
    ///   点击窗口即**置顶**（焦点，`topmost = true`）；z 越大越靠上。
    /// - **遮挡隔离**（点击穿透修复）：重叠区域只让**鼠标下最上层**的窗口响应
    ///   （见 [`crate::hit::window_occluded`]）。
    /// - **可拖拽**：按住窗口任意处移动 ≥ 3 物理像素进入拖拽（位置持久于
    ///   `UiState.panel_pos`）；纯点击不拖拽，窗口内子控件正常响应。
    /// - 绘制顺序由 [`Ui::finish`] 保证：**背景/图形严格先于文字**。
    ///
    /// 参数：`width = Some(w)` 固定宽（高度自然、右下角可缩放、跨帧持久）；
    /// `topmost` 点击是否置顶（modal 用 `false`）；`strict` 内容强制裁剪到窗口矩形
    /// （`Placement`）；`style` 逐窗口覆盖（`None` = 全局 [`Theme::panel`]）；
    /// `clamp` 位置约束模式（见 [`WindowClamp`]）。
    pub(super) fn window_impl(
        &mut self,
        id: &str,
        pos: Vec2,
        width: Option<f32>,
        // **固定高**（物理；`None` = 由内容决定）——`.height(..)` 的物理值。
        height: Option<f32>,
        // 内容子项间距（物理；`None` = `Theme::gap`）。下拉 / 菜单浮层传更紧的值。
        content_gap: Option<f32>,
        topmost: bool,
        strict: bool,
        style: Option<&PanelStyle>,
        clamp: WindowClamp,
        // 两条轴的溢出策略（显式设置；`None` = 按 `strict` / "该轴是否被拖过"解算）。
        scroll: (Option<ScrollMode>, Option<ScrollMode>),
        chrome: &mut WindowChrome<'_>,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        let id_for = self.id_for(id);
        // **本帧生效的尺寸** = 持久值（缩放柄拖出来的，跨帧保持）**优先**于传入的
        // `.width(..)` / `.height(..)`（那只是**初始值**）。统一在此读取——`window_at_w` /
        // `modal_at_w` / `WindowBuilder::{width,height}` 都不必各自处理。
        //
        // ⚠ **必须"持久优先"**，否则第二次拖柄会从**旧值**重新起算 ⇒ 那条轴弹回原位
        // （用户实测："拖拽缩放柄到别的地方，然后下一次点击 x 坐标弹回"——`eg260818UI`
        // 里只有**唯一没有 `.width()`** 的 `strict_win` 不弹，正是这条的反证）。
        let explicit_w = width;
        let persisted_w = self.state.window_widths.get(id_for.as_str()).copied();
        // 用于 clamp / 缩放柄 / 持久化判断的"有效宽"。
        let width = persisted_w.or(explicit_w);
        // **高度**：`.height(..)`（固定高）与持久高（拖过）同口径（持久优先）；
        // **收起态例外**（收起 = 只剩一行标题栏 ⇒ 忽略固定高 / 持久高）。这里**必须在
        // 缩放柄之前**算好：柄的基准 / 当前尺寸要用它。
        let explicit_h = height;
        let persisted_h = self.state.window_heights.get(id_for.as_str()).copied();
        let eff_h = persisted_h.or(explicit_h);
        let collapsed_now = chrome.collapsed(self.state, id_for.as_str());
        let fixed_h = if collapsed_now { None } else { eff_h };
        // z-order：首次分配 max+1；点击置顶在拖拽判定处处理
        let z = {
            // z-order：首次分配 max+1；点击置顶在拖拽判定处处理。
            // ⚠ 排除置顶哨兵（WIN_TOPMOST）——浮层不参与普通窗口的 z 递增。
            let max_z = self
                .state
                .window_z
                .values()
                .copied()
                .filter(|&z| z < WIN_TOPMOST)
                .max()
                .unwrap_or(0);
            *self.state.window_z.entry(id_for.to_static()).or_insert(max_z + 1)
        };
        let saved_win = std::mem::replace(&mut self.painter.q.cur_win, z);
        // 当前窗口 ID（嵌套窗口 = 下拉浮层进出时保存/恢复）：`hit_impl` 记按下归属要用它，
        // 因为 z 会在帧末被"点击置顶"改，而复核要用**当前** z（见 `resolve_widget_press`）。
        let saved_win_id = self.cur_win_id.replace(id_for.to_static());
        // 当前窗口的"可交互内容范围"并集：进入时从零开始，退出时并进遮挡矩形
        // （浮层是嵌套窗口 ⇒ 保存/恢复，浮层的内容不该算进外层窗口）。
        let saved_hit_bounds = self.win_hit_bounds.take();
        let saved_clip = self.painter.q.clip;
        // **窗口不继承外层的裁剪层**（与"窗口是布局根"同一条边界规则）：窗口是**浮层** ——
        // 它的内容（以及它内部开的浮层：下拉 / 取色面板 / 子菜单）不该被外层沙箱裁掉。
        // 历史 bug：`vscroll(Scroll)` 窗口里开一个取色面板，面板的绘制命令带着**外层视口**
        // 的 scissor ⇒ 面板被"不经意地"裁掉（用户截图批注："子窗口被不经意地裁掉"）。
        // 本函数末尾恢复：外层沙箱后续的内容仍要按它的 scissor 走。
        self.painter.q.clip = None;
        // 位置经**责任链**解析（脚本处理器 → 用户拖拽状态 → 传入 pos，见 pos_handler）
        let origin = self.resolve_pos(&id_for, pos);
        let start = self.painter.q.queue.len();
        let style = style
            .cloned()
            .unwrap_or_else(|| self.theme.panel.clone());
        let (pad_total, gap) = (
            style.padding + style.border_w,
            content_gap.unwrap_or(self.theme.gap),
        );
        let saved_base = self.abs_base;
        let (sw, sh) = (
            self.window.inner_size().width as f32,
            self.window.inner_size().height as f32,
        );
        // 窗口尺寸（clamp 用）：**按 id 跨帧持久**（`window_sizes`）——点击置顶
        // z+1 后尺寸不丢 → clamp 边界稳定（消除"按下即跳变"）；`window_rects[z]`
        // 仅兜底；首帧（均无记录）= `None`。
        let prev_size = self
            .state
            .window_sizes
            .get(id_for.as_str())
            .copied()
            .or_else(|| {
                self.state
                    .window_rects
                    .get(id_for.as_str())
                    .map(|r| Vec2::new(r.w, r.h))
            });
        // ─── ① 位置与交互**先于内容录制**求解 ────────────────────────────
        // 鼠标事件是针对**屏幕上已有的几何**（上一帧结算的 `prev_size`）产生的，
        // 故命中 / 拖拽 / clamp 全用 `prev_size`；由此 `display_pos` 在录制前已知，
        // `abs_base` 与随后 `translate(display_pos)` 的几何**当帧一致**。
        //
        // ⚠ 旧实现 `abs_base` 取自上一帧位置（`base_pos`）、几何用本帧位置
        // （`display_pos`），于是**拖拽期间**每个走 `abs_base` 的绝对空间量都落后
        // 一帧：文本/多行框的 `box_clip`（文字被裁）、IME 光标定位、滑块拖拽基准、
        // 下拉浮层位置。位移越大错得越多 → 快速拖动时"文字/点击瞬间偏移"。
        //
        // 非首帧用持久尺寸 clamp（主窗口缩小 / 内容变化后窗口被拉回屏幕内 → 照常
        // 可点可拖）；**首帧（尺寸未知）不 clamp**——窗口出现在应用指定位置，当帧
        // 显示已 clamp，次帧收敛一致（消除首帧跳变）。
        let base_pos = match (clamp, prev_size) {
            (WindowClamp::Screen, Some(ps)) => {
                clamp_window_pos(saved_base + origin, ps, sw, sh) - saved_base
            }
            _ => origin,
        };
        // 命中矩形 = 屏幕上那个矩形（首帧无 `prev_size` ⇒ 本帧不参与交互）。
        let panel_rect = prev_size.map(|ps| Rect::new(base_pos.x, base_pos.y, ps.x, ps.y));
        // **拖拽缩放**（右下角柄）：宽度跨帧持久于 `UiState::window_widths`、高度持久于
        // `UiState::window_heights`。⚠ 交互须在窗口拖拽判定**之前**（`claim_press`
        // 阻止按下缩放柄时同时建立窗口拖拽基准）。基于通用 [`Self::resize_handle`]。
        // handle 为**外层容器局部坐标**（此处 `abs_base` 仍为外层原点）；用 clamp 后
        // 位置 `base_pos`（而非 origin）——与显示一致，贴边窗口缩放柄可命中。
        //
        // `(allow, axes)`：`None` = 旧行为（**有 `.width(..)` 就能横向拖**）；
        // `allow = false` ⇒ 不画柄也不响应拖拽（`.width(..)` 仍作布局固定宽）；
        // `axes`：`Horizontal` 只调宽 / `Vertical` 只调高 / `Both` 宽高同调（光标随之）。
        let (allow_resize, resize_axes) = resolve_window_resize(chrome.resize, width.is_some());
        // ⚠ **收起态没有尺寸可调**：收起 = 一行标题栏（固定高都被忽略），此时——
        // ① 柄的命中区（右下角 14~35px 方块）**压住标题栏最右那两个按钮**：短窗口（收起后
        //    只有一行高）里 ⌃/✕ 会被柄抢走 ⇒ 用户实测"resizable 的窗口在点击收起按钮时
        //    仍然不会收起"（点下去是在拖尺寸）；
        // ② 柄的**按下种子**会把"收起后的那一行高"写进 `window_heights`（`or_insert` 是
        //    永久的）⇒ 展开回来时窗口变成一条缝，看起来就是"点一下柄就弹回/跳回去了"。
        // 故收起态整条缩放链路关掉（不画、不命中、不种子、不应用）。
        let resize_on = allow_resize && resize_axes != Resize::None && !collapsed_now;
        // **窗口柄：命中在这里（内容之前），应用推迟到内容 + 标题栏之后** ——
        // 面板内最后一个控件的缩放柄（如 `TextEditor::resize(Both)`）常常和窗口柄叠在
        // 同一个右下角：内容后登记 ⇒ 控件级遮挡上它在上；应用再等 `!press_claimed` ⇒
        // "点控件的柄"不会连带把窗口也缩了（用户实测："在文本编辑器控件上缩放柄无法使用"）。
        // 同一机制修掉收起态里"窗口柄抢 ⌃/✕ 按钮"的既有缺陷（按钮在装饰段里 claim）。
        let mut grip = None;
        if resize_on
            && let Some(ps) = prev_size
        {
            // **命中区跟随柄的图案尺寸**（`GripStyle::extent`），下限 14px（太小的柄点不中）；
            // `GripShape::Hidden` 时退回下限 —— 图案可以不画，但**缩放能力保留**。
            let hw = style.grip.extent().max(14.0);
            let handle = Rect::new(base_pos.x + ps.x - hw, base_pos.y + ps.y - hw, hw, hw);
            let h_id = format!("{id}::resize");
            let cursor = match resize_axes {
                Resize::Both => crate::UiCursor::NwseResize,
                Resize::Vertical => crate::UiCursor::NsResize,
                _ => crate::UiCursor::EwResize,
            };
            let hit = self.resize_handle_hit(&h_id, &handle, cursor);
            // **按下即落持久值**（种子 = 当前尺寸）：`fixed_w/h` 与 `axis_dragged` 都在
            // **本函数前面**读过，不种这一下，第一次拖拽本帧不生效（要等下一帧）——
            // 症状正是"点击缩小窗口后不会缩小"（尤其内容比它高时，本帧仍按内容撑开）。
            if hit && self.mouse_left().down_edge() {
                // ⚠ 两个槽的**单位不一样**、必须按各自口径写：
                // - `window_widths` = **内容宽**（`set_fixed_w` 的语义，外框 = 内容 + 2×pad）；
                // - `window_heights` = **外框高**（`Frame::fixed_h` 就是结算高本身）。
                // 早先把外框宽写进内容宽槽 ⇒ 每次按下/拖拽都白涨 2×pad，**越拖越宽**
                // （用户实测："gallery 咋变这么宽"）。
                if resize_fixes_width(resize_axes) && explicit_w.is_none() {
                    self.state
                        .window_widths
                        .entry(id_for.to_static())
                        .or_insert((ps.x - pad_total * 2.0).max(1.0));
                }
                if resize_fixes_height(resize_axes) {
                    self.state
                        .window_heights
                        .entry(id_for.to_static())
                        .or_insert(ps.y);
                }
            }
            // 当前尺寸 = 屏幕上那个 = **本帧生效的尺寸**（`width` / `eff_h` 已是"持久优先"；
            // 两者都没有 = 自动尺寸，由上一帧外框换算）。**宽是内容宽、高是外框高** ——
            // 单位不同，混用会让每拖一次涨 2×pad；用**显式值**当基准则会让第二次拖拽
            // 从旧值起算（那条轴"弹回原位"）。
            let cur_w = width.unwrap_or((ps.x - pad_total * 2.0).max(1.0));
            let cur_h = eff_h.unwrap_or(ps.y);
            grip = Some((
                h_id,
                Vec2::new(cur_w, cur_h),
                cursor,
                hit,
                // 最小尺寸：宽 120；高至少装得下一行 + 上下内边距（拖到 0 高的窗口
                // 会变成"一条线"，既点不中柄也看不出是什么）。
                Vec2::new(120.0, (self.theme.row_h + pad_total * 2.0).max(40.0)),
            ));
        }
        let btn = self.mouse_left();
        // 窗口遮挡：被更高 z 的窗口覆盖的区域，本窗口不响应拖拽 / 置顶 /
        // 子控件交互（点击穿透修复——重叠区域只让最上层窗口可交互）。
        let hit = panel_rect.is_some_and(|r| hit_test(&r, self.mouse_logical))
            && self.mouse_in_window
            && !window_occluded(z, self.mouse_logical, self.window_rects_iter());
        let press_here = btn.down_edge() && hit;
        // 拖拽基准：按下帧**先无条件**建立（基准 = 屏幕上那个矩形）。窗口内子控件
        // （输入框选择 / 滑块 / 滚动条）随后声明本次按下（`press_claimed`）时，
        // 在内容录制后清除基准（见 ②）——判定顺序与旧版一致。
        let (active, new_pos, drag_clamp_size) = if clamp == WindowClamp::Locked {
            // 锁定：位置固定（不建立拖拽基准、不激活拖拽；点击置顶 / 子控件仍有效）。
            (false, origin, prev_size)
        } else {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            let (active, pos) = resolve_drag(ws, hit, btn, self.mouse_screen, base_pos);
            // 拖拽中 clamp 边界 = 按下帧尺寸（固定）→ 位置纯跟手、不因内容尺寸
            // 变化被推回（消除"拖动单帧跳变"）；非拖拽帧用持久尺寸（命中基准一致）。
            let clamp_size = if active { ws.press_size.or(prev_size) } else { prev_size };
            (active, pos, clamp_size)
        };
        // **Screen 限位**：窗口 clamp 到画面（窗口客户区）内——拖拽 / 脚本定位后
        // 的位置都被限制（绝对坐标 clamp 后回容器局部）；`Free` / `Locked` 不 clamp
        // （Locked 本身位置固定）。**clamp 尺寸**：拖拽中 = 按下帧尺寸（固定，
        // 边界稳定 → 无单帧跳变）；非拖拽 = 持久尺寸（与命中基准一致，
        // 主窗口缩小 / 窗口比画面大也不会"看得见拖不动"）；首帧无记录 = 本帧
        // 显示不 clamp（与旧版一致：`prev_size` 为 `None` ⇒ 用 `origin`）。
        let display_pos = match (clamp, drag_clamp_size) {
            (WindowClamp::Screen, Some(cs)) => {
                clamp_window_pos(saved_base + new_pos, cs, sw, sh) - saved_base
            }
            _ => new_pos,
        };
        // 内容基准 = **本帧显示基准**（`display_pos`）——录制期的绝对空间量与几何一致。
        self.abs_base = saved_base + display_pos;
        let saved_hit_limit = self.cur_win_hit_limit.take();
        // ── 按轴解算溢出策略（显式 `.vscroll/.hscroll` > `Placement::Clip` > "该轴被拖过"）
        // `axis_dragged` = 该轴**已有持久尺寸**（用户拖过 ⇒ 老语义里"由用户接管"）。
        let v_dragged = self.state.window_heights.contains_key(id_for.as_str());
        let h_dragged = self.state.window_widths.contains_key(id_for.as_str());
        let vs = resolve_scroll_mode(scroll.0, strict, v_dragged);
        let hs = resolve_scroll_mode(scroll.1, strict, h_dragged);
        // 两条轴都是**真的**：`vscroll(Scroll)` / `hscroll(Scroll)` 都会把窗口内容包进
        // 一个滚动视口（见下面的 `viewport`），滚动条与偏移各由 `ScrollState::offset` /
        // `offset_x` 承载（`scroll_at_axes` 里按轴画条、按轴吃滚轮）。
        let mut frame = Frame::new_stack(PackSide::Top, gap, pad_total);
        // ── 宽度：**一律固定**（不再有"拖出来的是下限"这条）────────────────────────
        //
        // 老语义里 `NoClip` + 持久宽 = **下限**（内容可以把窗口撑得更宽）。它有两个
        // 后果，用户都实测过：拖过一次之后窗口被内容顶得**越拖越宽**（"gallery 咋变
        // 这么宽"）；而"压缩内容"根本没发生。egui 语义（用户给的判定表）是：
        // 水平轴要么**压缩**内容（`NoClip`）、要么**裁切**内容（`ClipOnly`/`Scroll`），
        // 两条都用**给定的宽**（`.width(..)` 或用户拖出来的持久宽）。
        // **有宽就固定宽**（`.width(..)` 的初始值 / 用户拖出来的持久值都已含在 `width` 里）；
        // 没宽 = 自动宽（由内容结算）。
        if let Some(w) = width {
            frame.set_fixed_w(w);
        }
        // **该轴是视口** ⇒ 子项按自然宽排布、不压缩（超出由下面的 clip 层裁掉）。
        // 这就是"**裁切内容**"与"**压缩内容**"的唯一开关（见 `Frame::set_clip_w`）。
        frame.set_clip_w(hs != ScrollMode::NoClip);
        // **窗口 = 布局根**：内容宽只能由本窗口给（`fixed_w` / 内容自然宽），**不许**
        // 向窗口外的容器借宽（根 frame 的固定宽是"视口宽"，那是给顶层 `win=0` 内容用的）。
        // 不设它 ⇒ 自动宽窗口里的 `divider()` / `Label` 拿到视口宽 ⇒ 整窗被撑成屏幕宽。
        //
        // **但要把"窗口自己已知的宽"报出去**（`Frame::set_pass_through_w`）：自动宽窗口在录制前
        // 就已经知道自己的内容宽（`.width()` 在 `width` 里 / 上一帧结算宽 `prev_size`）——
        // `ui.foldable(..)` 的标题行、`ui.divider()` 这类**整格容器**因此**首帧就铺满**，
        // 而不是"先按自然宽录一帧、下一帧才变宽"（用户实测："标题行缩在半截 / 线短一截"；
        // 这也正是 `divider()` 要"结算后回填宽度"那条补丁的**更根本**的修法）。
        frame.set_layout_root(true);
        frame.set_pass_through_w(
            width.or_else(|| prev_size.map(|s| s.x)).map(|w| (w - pad_total * 2.0).max(1.0)),
        );
        // `vscroll(Scroll)` 的**视口高**在下面算（要等标题栏占位之后才拿得到内容原点）；
        // 这里先记下"该轴是滚动视口"，`set_fixed_h` 在那里做。
        // `fixed_h` / `eff_h` / `collapsed_now` 已在缩放柄之前算好（柄要用它）。
        match (vs, fixed_h) {
            // 视口轴（ClipOnly）：固定高，内容不撑高它。
            (ScrollMode::ClipOnly, Some(h)) => frame.set_fixed_h(h),
            // `NoClip` 轴：拖出来的高度是**下限**（内容仍可撑高）⇒ "大小必须能呈现所有内容"。
            (ScrollMode::NoClip, Some(h)) => frame.set_row_bounds(Some(h), None),
            _ => {}
        }
        // **可命中限制**：内容会被裁切的那条轴把命中范围钉在窗口矩形内 ⇒ 溢出到面板外的
        // **幽灵控件**既看不见也点不到（`NoClip` 的轴不限制：溢出可见就该可点）。
        // 用 `prev_size`（上一帧结算尺寸）：尺寸要等录完才知道，而鼠标事件本来就是针对
        // "屏幕上已有的几何"产生的（与命中 / 拖拽 / clamp 同一口径）。
        if (vs != ScrollMode::NoClip || hs != ScrollMode::NoClip)
            && let Some(ps) = prev_size
            && ps.x > 0.0
            && ps.y > 0.0
        {
            let w = saved_base.x + display_pos.x;
            let h = saved_base.y + display_pos.y;
            // 与外层窗口的限制求交（浮层是嵌套窗口）；"不裁"那条轴的兜底 = **屏幕**
            // （不裁 ≠ 无限，只是别用窗口边界去裁）。
            self.cur_win_hit_limit = clip_for_axes(
                saved_hit_limit,
                Rect::new(w, h, ps.x, ps.y),
                Rect::new(0.0, 0.0, sw, sh),
                vs,
                hs,
            );
        }
        self.frames.push(frame);
        self.painter.q.depth += 1;

        // ID 命名空间：窗口进入压栈、退出弹栈（闭包作用域保证配对——取代手动
        // push_id/pop_id，杜绝漏配对/多弹出）。窗口内子控件 ID 自动带窗口前缀。
        //
        // **标题栏 = "长宽已确定的容器"，在窗口结算之后再跑**（见 [`Self::ornament_at`]）。
        // 第一遍只**占位**：留出条高，让用户内容从条下开始（与旧实现同一个 y）。
        // 通条高度 = **一行**（条贴窗口顶边，起点 0）：内容上抬 `pad_total`、
        // 条高不含上内边距，下面的内容与窗口高度随之各少一个 `pad_total`。
        // ⚠ 条只是**背景装饰、不裁剪内容**：标题 / ▲ / ✕ 可以比条高再高一点（用户要求）。
        let bar_h = if chrome.bar_on() { title_bar_h(self.theme.row_h) } else { 0.0 };
        let collapsed = chrome.collapsed(self.state, id_for.as_str());
        // **占位宽度**：展开态由**内容**决定（标题不再撑宽窗口 —— 标题改为在最终宽度里
        // 居中 / 省略）；**没有内容**时（收起态、且未给 `.width()`）用"标题 + 按钮簇 +
        // 内边距"兜底，否则窗口会塌成 `2×pad`、标题被省略成空。
        // ⚠ 固定宽窗口（`.width(..)`）由 `fixed_w` 决定宽度 ⇒ 这里不需要占位宽。
        // （展开态但内容为空的自动宽窗口仍是 `2×pad`：那种窗口没有内容可依据，属已知取舍。）
        let reserve_w = if bar_h > 0.0 && collapsed && width.is_none() {
            let (row_h, gap) = (self.theme.row_h, self.theme.gap);
            // 先拷 held 值再调 `&mut self` 的方法（`font_family` 借用不能跨调用）。
            let (font_size, family) =
                (self.theme.label.font_size, self.theme.label.font_family.clone());
            let title_w = self
                .text_size(chrome.title.unwrap_or(""), font_size, family.as_deref())
                .x;
            // `None` = "没有外框宽"那条解算 ⇒ 返回"标题 + 按钮簇 + 内边距"的最低宽度。
            title_bar_layout(
                None,
                pad_total,
                row_h,
                gap,
                chrome.show_collapse(),
                chrome.close.is_some(),
                TITLE_BUTTON_INSET,
                title_w,
            )
            .bar_w
        } else {
            0.0
        };
        if bar_h > 0.0 {
            // **占位记录在窗口顶边**（`y = 0`，x 保持内容左缘）——与旧实现"把内容光标临时
            // 抬到 `y = 0` 再录标题行"同一口径：内容因此从 `bar_h + gap` 开始（**不是**
            // `pad_total + bar_h + gap`），窗口高度不会多出一个 `pad_total`。
            if let Some(fr) = self.frames.last_mut() {
                fr.cursor.y = 0.0;
            }
            self.child_rect(reserve_w, bar_h, Child::Expand);
        }
        // **结算 + 标题栏容器都在窗口 ID 命名空间内**（`with_id` 作用域里）：
        // 装饰控件的 id 必须带窗口前缀，否则帧末"被更高窗口遮挡"的复核会把标题栏按钮
        // 当成 **win=0 内容**而撤销它的按下（实测：`hit[..] ::collapse OK` 但点不动、
        // 收起状态永远不翻转）。
        let mut size = Vec2::ZERO;
        // **内容命令区间的右端**（见下面 `content_end = ..` 的赋值）：裁切层只盖这段。
        let mut content_end = start;
        // **内容原点**（窗口局部坐标）= 标题栏占位之后的游标。窗口内容盒的左上角就是它；
        // 裁切用的 scissor 必须**只有内容**（不含标题栏 / 内边距 —— 用户要求：
        // "裁切内容的绘制用 Scissor 矩形范围应该只有内容，没有标题栏"）。
        let content_origin = self
            .frames
            .last()
            .map(|f| f.cursor)
            .unwrap_or(Vec2::splat(pad_total));
        // ─── `vscroll(Scroll)`：把内容录进一个**滚动视口**（窗口内滚动条）───────────
        // 视口 = 窗口内容盒（标题栏之下），高 = 固定高（`.height(..)` / 拖过）**或**
        // "窗口顶到屏幕底还剩多少"（`Scroll` 的自然含义：不许跑出屏幕，多出来的滚）。
        //
        // ⚠ **收起态不建视口**（`!collapsed`）：收起就是"只剩一行标题栏"，而视口会
        // `set_fixed_h(视口高 + pad)` 把窗口重新顶高 ⇒ 用户实测的"resizable 窗口点 ⌃
        // 仍然不收起"（状态翻转了、内容也没了，但那个空面板还是原来那么高）。
        let scroll_axis_on = !collapsed && (vs == ScrollMode::Scroll || hs == ScrollMode::Scroll);
        // 该滚动容器的**跨帧状态**（上一帧的内容尺寸）：视口宽 / 内容定高 / 自动宽窗口的
        // 结算都用它（`ScrollState` 是 `Copy` 的 ⇒ 拷出来用，不与后面的 `&mut self` 打架）。
        let prev_scroll = self
            .state
            .scrolls
            .get(format!("{}/scroll", id_for.as_str()).as_str())
            .copied();
        let viewport = if scroll_axis_on {
            let top_left = content_origin;
            // ⚠ 用 `fixed_h`（**已按收起态清零**）而不是 `eff_h`：同一条"收起忽略固定高"
            // 的规则必须在这里也生效，否则视口会把忽略掉的那个高又拿回来。
            let avail_h = if let Some(h) = fixed_h {
                // 固定高（`.height(..)` / 拖过）：外框高 ⇒ 视口 = 它扣掉标题栏与内边距。
                h
            } else {
                // **没给固定高也没拖过 ⇒ 按内容定高**：视口 = min(内容需要的总高, 屏幕剩余)。
                // 只用"屏幕剩余"会把**内容很矮**的窗口也撑到屏幕底（用户实测："最开始打开
                // 调色板编辑器时仍然会把高度撑到窗口底端"的同一根因）。
                // 内容高从**上一帧**的滚动状态读（`ScrollState.content_h`；它是滚动容器结算
                // 出来的内容高，与视口无关）——首次会话未知 ⇒ 先按屏幕剩余，次帧收敛。
                let top_abs = saved_base.y + display_pos.y;
                let screen_avail = (sh - top_abs).max(0.0);
                match prev_scroll.map(|s| s.content_h) {
                    Some(ch) if ch > 0.0 => screen_avail.min(ch + top_left.y + pad_total),
                    _ => screen_avail,
                }
            };
            let h = avail_h - top_left.y - pad_total;
            // ⚠ 视口**横跨窗口内容盒**：`width` 已经是**内容宽**（`set_fixed_w` 的语义，
            // 外框宽 = `width + 2×pad_total`），再减一次内边距会让视口窄 2×pad（实测：
            // 条带跟着左移 26px ⇒ 脚本按"窗口右缘 − pad − 7"点的条带落空、`offset` 恒 0）。
            //
            // **自动宽窗口**（`width == None`）的视口宽：首选取**内容自己的自然宽**
            // （上一帧 `ScrollState.content_w`），其次上一帧结算宽反推，首帧用**屏幕剩余宽**
            // 当引导值 —— **绝不能是 1px**：视口 1px 宽 ⇒ 内容按 1px 折行（`Label` 一个字
            // 一行、`row` 里的控件被压到下限）⇒ 窗口塌成 `2×pad` 的一条缝，下一帧照它再算
            // ⇒ **永远一条缝**（用户实测："调色板编辑器在不指定 width 的情况下无法使用"）。
            let w = width.unwrap_or_else(|| {
                prev_scroll
                    .map(|s| s.content_w)
                    .filter(|w| *w > 0.0)
                    .or_else(|| prev_size.map(|s| (s.x - pad_total * 2.0).max(1.0)))
                    .unwrap_or_else(|| (sw - (saved_base.x + display_pos.x)).max(1.0))
            });
            Some(Rect::new(
                top_left.x,
                top_left.y,
                w.max(1.0),
                h.max(1.0),
            ))
        } else {
            None
        };
        self.with_id(id, |ui| {
            {
                // 显式重借用：`Window` 拿走 `&mut Ui`，块结束后 `ui` 仍可用。
                if let Some(vp) = viewport {
                    // 视口高固定 ⇒ 窗口尺寸 = 视口 + 内边距（内容再高也不撑大它）。
                    if let Some(fr) = ui.frames.last_mut() {
                        fr.set_fixed_h(vp.y + vp.h + pad_total);
                    }
                    // ⚠ 滚动容器的 id 用**相对名**（这里已经在 `with_id(id)` 的窗口命名空间里；
                    // 写 `{id}::scroll` 会变成 `win/win::scroll` —— 双重前缀）。
                    ui.scroll_at_axes(
                        Position::Physical(Vec2::new(vp.x, vp.y)),
                        Size::Physical(Vec2::new(vp.w, vp.h)),
                        "scroll",
                        vs,
                        hs,
                        |ui| {
                            let mut w = Window { ui };
                            if !collapsed {
                                f(&mut w);
                            }
                        },
                    );
                    // **自动宽窗口必须"看见"内容**（设计理念：**无顾虑地使用** —— 不写
                    // `.width(..)` 也要能用）：`scroll_at_axes` 是沙箱、**自己不 note**
                    // （它不知道调用方要不要这块占位），于是窗口帧的 `max_child.x` 恒 0 ⇒
                    // 窗口宽 = `2×pad` 的一条缝、内容按它折行。用刚写回的 `content_w/h`
                    // 补一条内容矩形 ⇒ 窗口撑到**内容的自然尺寸**（高那条会被上面的
                    // `set_fixed_h`/视口覆盖，不影响定高视口）。
                    if width.is_none()
                        && let Some(fr) = ui.frames.last_mut()
                        && let Some(st) = ui
                            .state
                            .scrolls
                            .get(format!("{}/scroll", id_for.as_str()).as_str())
                            .copied()
                    {
                        fr.note_content(Rect::new(vp.x, vp.y, st.content_w, st.content_h));
                    }
                } else {
                    let mut w = Window { ui: &mut *ui };
                    if !collapsed {
                        f(&mut w);
                    }
                }
            }
            // 结算尺寸（装饰之前）：标题/按钮不得反过来影响窗口尺寸。
            size = ui.frames.last().expect("window frame").settle_size();
            // **分割线满宽**：窗口内容宽 = 结算宽 − 2×内边距（此刻才知道 ⇒ 回填线段宽度）。
            // 必须在 `content_end` **之前**：回填只改 `rect.w`（不改条数）⇒ 命令区间不受影响，
            // 但把线宽定在"内容宽"上要趁 `patch_window_content_clip` 用 `size` 之前做完。
            let content_w = (size.x - pad_total * 2.0).max(0.0);
            ui.expand_pending_full_w(start, content_w);
            // **内容绘制的命令区间**（`[start, content_end)`）：裁切层只该盖**内容** ——
            // 标题栏（`ornament_at`）与面板底色 / 边框 / 缩放柄都在这之后录，**不参与**裁切。
            content_end = ui.painter.q.queue.len();
            // **标题栏容器**：宽 = 最终外框宽 ⇒ 按钮贴外缘、标题可按最终宽度居中/省略。
            // 插在**按下裁决之前**（裁决在 `with_id` 之后）：按钮的 `claim_press()` 必须
            // 早于窗口自己的裁决，否则"按按钮把窗口拖走"复现。
            if bar_h > 0.0 && size.x > 0.0 {
                let (chrome, id_for) = (&mut *chrome, &id_for);
                ui.ornament_at(Rect::new(0.0, 0.0, size.x, bar_h), |bar| {
                    // `style.radius`：最右按钮的右上角取**面板圆角**（贴外缘才嵌得进圆角）；
                    // `id_for`（绝对 ID）：引擎托管的收起状态（`collapsible: None`）按它存取。
                    window_title_bar(bar, chrome, id_for, collapsed, size.x, pad_total, style.radius);
                });
            }
        });

        // 弹出窗口 frame。**沿用上面已经算好的 `size`**：`ornament_at` 不 note_content
        // ⇒ 装饰不可能改变尺寸（语义保证：装饰绝不参与尺寸结算）。
        self.frames.pop().expect("window frame");
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        // 记录窗口尺寸（按 id 持久；点击置顶 z 变化后下帧 prev_size 仍可取）。
        self.state.window_sizes.insert(id_for.to_static(), size);
        // ─── ①b 窗口缩放柄的**应用**（内容 + 标题栏之后）────────────────────
        // 命中早在内容之前就算过（`grip.3`，为的是把命中区登记顺序排在内容之前 ⇒
        // 内容里的控件在控件级遮挡上获胜）；这里只在**没人认领这次按下**时把拖拽落到
        // 尺寸上：`TextEditor` 自己的缩放柄认领了就轮不到窗口。
        if let Some((h_id, cur, cursor, grip_hit, min)) = grip
            && !self.press_claimed
            && let Some(new_size) =
                self.resize_handle_apply(&h_id, cur, min, cursor, grip_hit)
        {
            // 新尺寸下帧生效（`width` / 高度于本函数开头读取）——与旧版一致，避免
            // 同帧内布局尺寸与 clamp 尺寸互相矛盾。
            if resize_fixes_width(resize_axes) {
                self.state.window_widths.insert(id_for.to_static(), new_size.x);
            }
            if resize_fixes_height(resize_axes) {
                self.state
                    .window_heights
                    .insert(id_for.to_static(), new_size.y);
            }
        }
        // ─── ② 内容录完后：按下裁决 ──────────────────────────────────────
        // 窗口内子控件（文本框选择 / 滑块 / 滚动条）在录制期可能已声明本次按下
        // （`press_claimed`）——此时**清除**拖拽基准，否则 `update_drag` 已置
        // dragging=true，残留的 press_mouse 会被 drag_moved 当作基准算出巨大位移
        // → 窗口"瞬移"（从输入框上拖拽 = 选择文本；窗口改从空白/标题区拖动）。
        if press_here {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            if self.press_claimed {
                // 输入框等文本控件按下（选择拖拽优先）：**清除拖拽基准**——
                // 否则 `update_drag` 已置 dragging=true，残留的 press_mouse 会被
                // drag_moved 当作基准，算出巨大位移 → 窗口"瞬移"。
                clear_drag_base(ws);
            } else {
                // 按下帧窗口尺寸：拖拽中 clamp 边界**固定**——内容尺寸变化不推窗。
                ws.press_size = Some(size);
            }
        }
        // 点击置顶（modal 对话框**不主动置顶**——它已最上，且避免 z 漂移/与浮层冲突）。
        // 重叠区域点击按下时记录"本帧按下命中的**最上层**窗口"（win_press_top），
        // `finish::resolve_win_press` 只保留它的拖拽与置顶——避免同时拖动多个窗口。
        if topmost
            && press_here
            && self
                .win_press_top
                .as_ref()
                .is_none_or(|(_, top_z)| self.painter.q.cur_win > *top_z)
        {
            self.win_press_top = Some((id_for.to_static(), self.painter.q.cur_win));
        }
        if active {
            self.drag_panel = Some(id_for.to_static());
            // 持久化 **clamp 后**的位置（下帧 origin 已限位，视觉与状态一致）。
            if self.state.panel_pos.get(id_for.as_str()) != Some(&display_pos) {
                self.state.panel_pos.insert(id_for.to_static(), display_pos);
            }
        } else if self
            .drag_panel
            .as_ref()
            .is_some_and(|d| d.as_str() == id_for.as_str())
        {
            self.drag_panel = None;
        }
        if press_here || active {
            // 按下窗口（或拖拽中）都算"已响应按下"——避免空白点击清焦点
            self.any_pressed = true;
        }
        // 窗口拖动激活 → 强制普通 Arrow（UI_NEEDS：移动窗口时无需 <->，是 BUG）。
        if active {
            self.cursor_window_drag = true;
        }
        // 记录窗口原点（顶点局部化基准；win=0 非窗口默认 (0,0)）与窗口 id（缓存 key）
        self.win_origins.insert(z, display_pos);
        // **按 id 再记一份**：同一帧可以有多个 z 相同的 `WIN_TOPMOST` 浮层（下拉里开子菜单），
        // z 键表只剩最后一个 ⇒ `debug_dump` 会漏掉嵌套浮层（渲染不受影响：采集减、提交加
        // 用的是同一个 z 键值）。id 键表让每个浮层都能被诊断到。
        self.state
            .window_origins
            .insert(id_for.to_static(), display_pos);
        self.win_ids.insert(z, id_for.to_static());
        // 窗口矩形入遮挡判定缓存（跨帧；finish 末尾只保留本帧录制的窗口）。
        // ⚠ 存**绝对**坐标：嵌套窗口 / 下拉浮层在容器内时 `display_pos` 是容器
        // 局部坐标，须加容器绝对原点（`saved_base`）——否则遮挡判定用绝对鼠标
        // 比局部矩形恒不命中，浮层背后的控件仍响应 hover/click（"下拉菜单选项
        // 悬停时背后按钮一起 Hover"）。
        //
        // **遮挡矩形 = 窗口盒子 ∪ 本帧子控件的命中区**（`win_hit_bounds`）：
        // 容器尺寸已保证包住子控件（见 `Frame::content_bounds`），这里是**兜底**——
        // 固定尺寸容器 / 有意溢出的装饰 / 未来新增的绝对放置 API 都还能保住
        // "看得见就能点"：遮挡判定按内容的实际范围走，而不是按边框盒子。
        let win_abs = Rect::new(
            saved_base.x + display_pos.x,
            saved_base.y + display_pos.y,
            size.x,
            size.y,
        );
        let occl = match self.win_hit_bounds {
            Some(b) => win_abs.union(&b),
            None => win_abs,
        };
        // **键 = 窗口绝对 ID**（不是 z）：z 会在帧末被"点击置顶"改，而矩形是这一刻录下的；
        // 按 ID 存 + 查询时解当前 z，才能让"刚被抬高的窗口"立刻按**新 z**参与遮挡判定。
        let wid = id_for.to_static();
        self.state.window_rects.insert(wid.clone(), occl);
        // 记入**帧级**"本帧录过的窗口"清单：帧末视图的 `win_ids` 会被 `save_frame_state`
        // 换空，只有这里记下的清单能在**下一帧开场**用来清陈旧（见 `window_ids_seen`）。
        if !self.state.frame_state.window_ids_seen.contains(&wid) {
            self.state.frame_state.window_ids_seen.push(wid);
        }
        // 严格裁剪（`window_at_strict`）：窗口内容**强制裁剪**到窗口矩形——结算后
        // 统一改写本窗口命令的裁剪层（录制期窗口尺寸未知，背景/子控件命令都覆盖；
        // 命中裁剪由窗口遮挡机制负责）。默认窗口为 Expand 语义（不裁剪）。
        // **按轴**强制裁剪：结算后统一改写本窗口命令的裁剪层（录制期窗口尺寸未知，
        // 背景 / 子控件命令都覆盖；命中侧由 `cur_win_hit_limit` 负责）。
        //
        // 每条轴独立：`NoClip` 的轴**不裁**（内容溢出可见，也能点），另一条轴照裁 ——
        // 这正是"只要纵向滚动条、横向不裁"能成立的原因。触发条件：
        // ① `.vscroll/.hscroll(ClipOnly|Scroll)`（显式）；
        // ② `.placement(Placement::Clip)`（老的一体化开关 ⇒ 两条轴都裁）；
        // ③ **该轴被用户拖过尺寸**（固定尺寸视口：内容再撑不开它，不裁就会画到面板外
        //    —— 用户实测的 TTT 窗口 bug）。
        if vs != ScrollMode::NoClip || hs != ScrollMode::NoClip {
            // **裁切矩形 = 窗口的"内容盒"**（不含标题栏、不含内边距）——用户要求：
            // "裁切内容的绘制用 Scissor 矩形范围应该只有内容，没有标题栏"。
            // 局部坐标 = `[content_origin, size − pad_total]`（`content_origin` 已在标题栏
            // 占位之后取过；无标题栏时它就是 `(pad, pad)`）。
            let content_box = Rect::new(
                saved_base.x + display_pos.x + content_origin.x,
                saved_base.y + display_pos.y + content_origin.y,
                (size.x - content_origin.x - pad_total).max(1.0),
                (size.y - content_origin.y - pad_total).max(1.0),
            );
            let clip = clip_for_axes(saved_clip, content_box, Rect::new(0.0, 0.0, sw, sh), vs, hs);
            // 诊断（`RJ_WINCLIP_TRACE=1`）：内容盒 scissor 的**输入与输出**——
            // "内容 scissor 不含标题栏"这类几何争议直接看这三行，不必猜。
            if std::env::var_os("RJ_WINCLIP_TRACE").is_some() {
                eprintln!(
                    "winclip[{id}] win=({:.0},{:.0},{:.0},{:.0}) content_origin=({:.0},{:.0}) box=({:.0},{:.0},{:.0},{:.0}) v={vs:?} h={hs:?} -> clip={clip:?}",
                    saved_base.x + display_pos.x,
                    saved_base.y + display_pos.y,
                    size.x,
                    size.y,
                    content_origin.x,
                    content_origin.y,
                    content_box.x,
                    content_box.y,
                    content_box.w,
                    content_box.h,
                );
            }
            // ⚠ **只盖内容自己的绘制，且只盖本窗口**：
            // - 区间 `[start, content_end)`：标题栏（`ornament_at`）与面板底色 / 边框 /
            //   缩放柄都在那之后录 ⇒ 标题栏**不会**被内容 scissor 裁掉；
            // - `d.win == z`：本窗范围内录的**嵌套浮层**（下拉 / 取色面板 / 子菜单的 z 是
            //   `WIN_TOPMOST`）各有自己的层级与裁剪，被父窗口的视口顺带裁掉就是
            //   "子窗口被不经意地裁掉"；
            // - **与已有的内层裁剪求交**（`clip_and`）：内容里的文本框盒 / 内层滚动视口
            //   必须保留自己更窄的 scissor（旧实现直接覆盖 ⇒ 内层裁剪被吃掉）。
            for d in &mut self.painter.q.queue[start..content_end] {
                if d.win == z {
                    d.clip = clip_and(d.clip, clip);
                }
            }
        }
        // 背景 + 边框（win = z，画在窗口子控件之下；radius > 0 走圆角双层矩形）
        let bg_rect = Rect::new(0.0, 0.0, size.x, size.y);
        self.push_panel_shadow(bg_rect, &style.shadow, style.radius);
        self.push_panel_like_img(bg_rect, style.bg, style.bg_image, style.border, style.border_w, style.radius, 0);
        // **拖拽缩放柄图案**：只在"允许拖拽 + 有轴"时画（`GripShape::Hidden` 时
        // `push_resize_grip` 自己短路）——菜单 / 下拉浮层用 `resize(false, Resize::None)`
        // 拿到"固定宽但不画柄、不可拖"的效果。
        if resize_on {
            self.push_resize_grip(size, &style.grip);
        }
        // **标题栏通条**（整窗宽、含面板内边距 ⇒ 通条观感）：在**这里**画（`size` 已知）、
        // `elem = 0` 且晚于面板背景入队 ⇒ 按 `(elem, seq)` 排在面板背景**之上**、
        // 所有控件（`elem ≥ 1`）**之下**。
        //
        // 一次 `push_panel_like` 就够：底色 `surface_raised` + 面板同色同宽边框 ⇒
        // 上/左/右三段边框与面板边框**连续**（不会"标题栏把上边框啃掉"），底边那条
        // 就是 1px 分隔线。圆角取面板的**上面两角**（只有下面两角贴直角的面板，
        // 通条才不会在圆角处出框）。
        if bar_h > 0.0 && size.x > 0.0 {
            let bar = Rect::new(0.0, 0.0, size.x, bar_h);
            let top_radius = CornerRadius {
                tl: style.radius.tl,
                tr: style.radius.tr,
                br: 0.0,
                bl: 0.0,
            };
            self.push_panel_like(
                bar,
                self.theme.palette.surface_raised,
                style.border,
                style.border_w,
                top_radius,
                0,
            );
        }
        for d in &mut self.painter.q.queue[start..] {
            d.translate(display_pos);
        }
        // 恢复外层的**裁剪层**（见入口处的说明：窗口不继承外层的裁剪）。
        self.painter.q.clip = saved_clip;
        self.painter.q.cur_win = saved_win;
        self.cur_win_id = saved_win_id;
        // 恢复外层窗口的"可交互内容范围"（本窗口已并进自己的遮挡矩形）。
        self.win_hit_bounds = saved_hit_bounds;
        // 恢复外层窗口的可命中限制（浮层 = 嵌套窗口）。
        self.cur_win_hit_limit = saved_hit_limit;
        size
    }

    /// **菜单栏**（横向；见 [`crate::widgets::MenuBar`] 的模块文档与用法示例）。
    ///
    /// **本质 = 一行（`row`）+ 一条覆盖整栏宽度的背景**：
    /// `f` 里 `bar.menu(label, |m| ..)` 加菜单（下拉面板内容用闭包写：`item` /
    /// `item_checked` / `caption` / `separator`，并 `Deref` 到 [`Window`] ⇒ 文本输入 /
    /// 分割线 / 按钮 / 横向排版都能放）；[`MenuBar`](crate::widgets::MenuBar) 又 `Deref` 到
    /// [`Pack`] ⇒ `bar.add(..)` / `bar.button(..)` / `bar.label(..)` / `bar.text_input(..)`
    /// / `bar.separator_v()`（**竖向分割线**）都能直接放进栏里。返回栏尺寸。
    ///
    /// - `pos` 是栏左上角（带单位，顶层放置时即屏幕坐标）；栏**不占父容器光标**（浮在顶层）；
    /// - `bar.width(..)` = 背景铺多宽（不调 = 子项撑多大就多大，如整条屏幕宽）；
    /// - 展开状态跨帧持久于 [`UiState::menu_open`]（触发器的**绝对 ID**）；
    /// - **同一时刻只有一个菜单开着**；点菜单项 / 点**栏外** / Esc 都会收起——
    ///   点栏内空白 / 栏里的竖分割线或别的控件**不会**收起（纯函数
    ///   `widgets::menubar::menu_bar_should_close`，逐组合单测）；
    /// - 下拉面板是 [`Level::Normal`] 浮层窗口 —— 想让它盖住别的窗口就把菜单栏录在
    ///   **各窗口之后**（窗口 z 在首次录制时按 `max+1` 分配）。背景同理只压同 `win` 的底层。
    pub fn menu_bar(
        &mut self,
        id: &str,
        pos: impl Into<Position>,
        f: impl FnOnce(&mut crate::widgets::MenuBar<'_, '_>),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        let open = self.state.menu_open.as_ref().map(|s| s.as_str().to_owned());
        // **栏 = 一行**：`PackSide::Left` + `Theme::menubar` 的内边距 / 间距（默认 4 / 2）、
        // `force_h_all(row_h)` ⇒ 触发器 / 竖分割线 / 塞进来的控件同一个行高。
        // 内边距为 0 时第一个触发器正好从 `pos` 起（`--sim-menu` 的坐标解算按主题取值）。
        let (mb_gap, mb_pad) = (self.theme.menubar.gap, self.theme.menubar.padding);
        let mut facts = None;
        let (size, _) = self.container(
            pos,
            Frame::new_stack(PackSide::Left, mb_gap, mb_pad),
            |ctx| {
                // 重借用：`Pack` 要**拿走**一个 `&mut Ui`（`MenuBar` 靠 `Deref` 到它拿 `UiAdd`），
                // 而 `ctx` 只是 `&mut ContainerCtx` ⇒ 借用 `*ctx.ui`（生命周期到闭包结束，
                // `bar.finish()` 就地消费，不逃出闭包）。
                let ui: &mut Ui<'_> = &mut *ctx.ui;
                let row_h = ui.theme.row_h;
                if let Some(fr) = ui.frames.last_mut() {
                    fr.set_force_h_all(row_h);
                }
                let mut bar = crate::widgets::MenuBar::new(Pack::new(ui), id, open);
                f(&mut bar);
                facts = Some(bar.finish());
            },
        );
        let facts = facts.expect("menu_bar: 闭包总是执行");
        // 栏矩形（**父容器局部**，与 `mouse_local()` 同一坐标系）：宽 = `bar.width(..)` 或内容宽。
        let bar_rect = Rect::new(pos.x, pos.y, facts.width.unwrap_or(size.x), size.y);
        let on_bar = hit_test(&bar_rect, self.mouse_local());
        let close = crate::widgets::menubar::menu_bar_should_close(
            facts.item_clicked,
            facts.down_outside,
            facts.on_trigger,
            on_bar,
            facts.esc,
        );
        if std::env::var_os("RJ_MENU_TRACE").is_some() {
            eprintln!(
                "menu[bar {id}] content=({:.0},{:.0}) bar=({:.0},{:.0} {:.0}x{:.0}) on_trigger={} \
                 on_bar={on_bar} down_outside={} item_clicked={} esc={} popup={:?} close={close}",
                facts.content.x,
                facts.content.y,
                bar_rect.x,
                bar_rect.y,
                bar_rect.w,
                bar_rect.h,
                facts.on_trigger,
                facts.down_outside,
                facts.item_clicked,
                facts.esc,
                facts.popup,
            );
        }
        // 展开 / 收起：`action`（本帧点了触发器 = 切换）优先；否则按收起规则。
        self.state.menu_open = match (facts.action, close) {
            (Some(a), _) => a.map(IdAbsolute::owned),
            (None, true) => None,
            (None, false) => self.state.menu_open.clone(),
        };
        // **全宽背景**：在子项**之后**录（`elem = 0`，但 seq 更大）⇒ 压在同深度底层绘制之上、
        // 所有控件之下（子项在 `container` 里 depth + 1 ⇒ 天然画在它之上）。
        // ⚠ 底边线**单独画**（不是四边环）：通栏条只要一条"下沿"，画环会在屏幕边缘多出两条
        // 竖线；线画在栏内下沿。`bar.border_w(0)` / 主题 `border_w = 0` ⇒ 不画。
        if bar_rect.w > 0.0 && bar_rect.h > 0.0 {
            let bg = facts.bg;
            self.push_panel_like(bar_rect, bg.bg, Color::TRANSPARENT, 0.0, bg.radius, 0);
            if bg.border_w > 0.0 {
                self.push_solid_rect(
                    Rect::new(
                        bar_rect.x,
                        bar_rect.y + bar_rect.h - bg.border_w,
                        bar_rect.w,
                        bg.border_w,
                    ),
                    bg.border,
                );
            }
        }
        size
    }

    /// **模态对话框**：全屏半透明遮罩（[`Theme::modal`](crate::style::Theme::modal)
    /// 的颜色/尺寸，默认全屏半透明黑）置于最上层，背后一切交互被遮挡（遮罩矩形
    /// 经窗口遮挡判定阻断，含顶层 win=0 内容）；对话框（可拖拽）浮于遮罩之上。
    /// `pos` 为对话框左上角（逻辑，相对当前容器原点；按顶层使用）。`Esc` 关闭由
    /// 调用方处理（见 [`crate::widgets::FontModal`]）。
    ///
    /// ⚠ **应在帧末（其它窗口之后）调用**：遮罩/对话框 z 每帧重写为"当前最大+1/+2"，
    /// 但本帧**之后**录制的窗口会分到更高 z 并绘制在其上——先录制窗口、最后录制
    /// modal，才能保证 Modal 恒在最上。
    ///
    /// 公开入口是 [`Self::modal`]（责任链 builder）。
    pub(super) fn modal_impl(
        &mut self,
        id: &str,
        pos: Vec2,
        width: Option<f32>,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        // 遮罩 z = 当前最大 + 1（普通窗口之上）；对话框 z 再 +1（window_impl 自动
        // 分配）。**每帧强制重写**（不是 or_insert）——Modal 打开期间恒在最上，
        // 不会被其它后置顶的窗口盖住；点击对话框/背景不触发额外 z 提升。
        let max_z = self
            .state
            .window_z
            .values()
            .copied()
            .filter(|&z| z < WIN_TOPMOST)
            .max()
            .unwrap_or(0);
        let dim_id = format!("{id}::dim");
        let dim_abs = self.id_for(dim_id.as_str());
        let z_dim = max_z + 1;
        // 遮罩与对话框 z **每帧强制重写**（不是 or_insert）——Modal 打开期间恒在最上：
        // ① 遮罩不被后置顶的窗口盖住；② 对话框不被自家遮罩盖住（or_insert 会保留
        // 旧 z，其它窗口置顶后遮罩 max+1 反超对话框旧 z → 字体窗口跑到遮罩后面）。
        self.state.window_z.insert(dim_abs.to_static(), z_dim);
        let dlg_abs = self.id_for(id);
        self.state.window_z.insert(dlg_abs.to_static(), z_dim + 1);
        // 遮罩矩形（**绝对物理坐标**；默认全屏 = 窗口客户区物理尺寸，
        // 可被 [`Theme::modal`] 的 `size` 覆盖）。
        let (mw, mh) = match self.theme.modal.size {
            Some(s) => (s.x, s.y),
            None => {
                let s = self.window.inner_size();
                (s.width as f32, s.height as f32)
            }
        };
        let dim_rect = Rect::new(0.0, 0.0, mw, mh);
        // 遮罩录制（win = z_dim；按顶层使用，局部 == 绝对）。
        let saved_win = std::mem::replace(&mut self.painter.q.cur_win, z_dim);
        let seq = self.next_seq();
        let depth = self.painter.q.depth;
        self.painter.q.queue.push(UiDraw {
            depth,
            seq,
            win: z_dim,
            elem: 0,
            rect: dim_rect,
            clip: self.painter.q.clip,
            full_w: false,
            kind: DrawKind::Solid(self.theme.modal.dim),
        });
        // 遮罩窗口矩形（遮挡判定用；绝对）。键按**遮罩的绝对 ID**（同 `window_impl`）。
        let dim_wid = dim_abs.to_static();
        self.state.window_rects.insert(dim_wid.clone(), dim_rect);
        if !self.state.frame_state.window_ids_seen.contains(&dim_wid) {
            self.state.frame_state.window_ids_seen.push(dim_wid);
        }
        self.win_ids.insert(z_dim, dim_abs.to_static());
        self.win_origins.insert(z_dim, Vec2::ZERO);
        self.painter.q.cur_win = saved_win;
        // 对话框窗口（window_impl 按 max+1 分配 → z = z_dim + 1，浮于遮罩之上；
        // **不主动置顶**——点击对话框/背景不触发 z 提升）。
        self.window_impl(
            id,
            pos,
            width,
            // modal 不设固定高（高度由内容定）。
            None,
            None,
            false,
            false,
            None,
            WindowClamp::Screen,
            // modal 不做按轴滚动 / 裁切（要滚动就在对话框内容里嵌 `scroll_at`）。
            (None, None),
            &mut WindowChrome::none(),
            f,
        )
    }

    // ── 容器责任链 builder 入口（window / panel / modal） ──────────

}

/// **窗口拖拽缩放的有效开关**（纯函数，可单测）：`opt` = `WindowBuilder::resize` 的显式设置
/// （已由 `show()` 按判定表把 `bool` 展开成 `(allow, axes)`）。
///
/// - `Some((allow, axes))` ⇒ 用调用方的（`allow = false` ⇒ 不画柄、不响应拖拽）；
/// - `None` ⇒ **旧行为**：有 `.width(..)` 就能横向拖（`Resize::Horizontal`），没有就不拖。
///
/// 抽成纯函数是为了把"允许 / 轴 / 没设置"三种情况的判定钉在测试里——
/// 这类"开关没接上"的 bug 在 GUI 里很难肉眼发现（柄画了但不响应、或没画却响应）。
pub(super) fn resolve_window_resize(opt: Option<(bool, Resize)>, has_width: bool) -> (bool, Resize) {
    match opt {
        Some((allow, axes)) => (allow, axes),
        None => (has_width, Resize::Horizontal),
    }
}

/// **该轴向是否让"宽度"由用户接管**（拖过即固定宽；纯函数，可单测）。
///
/// `Horizontal` / `Both` ⇒ `true`；`Vertical` / `None` ⇒ `false`（宽度仍由内容决定）。
#[inline]
pub(super) fn resize_fixes_width(axes: Resize) -> bool {
    matches!(axes, Resize::Horizontal | Resize::Both)
}

/// **该轴向是否让"高度"由用户接管**（拖过即固定高；纯函数，可单测）。
///
/// `Vertical` / `Both` ⇒ `true`（高度跨帧持久于 `UiState::window_heights`，窗口成为
/// 固定尺寸视口 ⇒ 内容被裁切 / 可配滚动）；`Horizontal` / `None` ⇒ `false`。
#[inline]
pub(super) fn resize_fixes_height(axes: Resize) -> bool {
    matches!(axes, Resize::Vertical | Resize::Both)
}

/// **单轴的内容溢出策略解算**（纯函数，可单测）。
///
/// 输入：应用的显式选择（`.vscroll(..)` / `.hscroll(..)`，`None` = 没给）、老的
/// [`Placement::Clip`] 一体化开关、以及**这条轴是否被用户拖过尺寸**。输出：该轴怎么处理。
///
/// 规则（顺序即优先级）：
/// 1. **显式设置胜**（新 API 覆盖 `Placement`）；
/// 2. 否则 `Placement::Clip` ⇒ `ClipOnly`（两条轴都裁，老行为）；
/// 3. 否则**被拖过的轴** ⇒ `ClipOnly` —— 这是既有语义："高度一旦被拖过，窗口就是固定
///    尺寸视口，内容不再撑高它"（用户实测的 TTT 窗口 bug 就是这么修的）；
/// 4. 其余 ⇒ `NoClip`（内容撑大窗口，与不加本 API 之前逐像素一致）。
pub(super) fn resolve_scroll_mode(explicit: Option<ScrollMode>, placement_clip: bool, axis_dragged: bool) -> ScrollMode {
    match explicit {
        Some(m) => m,
        None if placement_clip => ScrollMode::ClipOnly,
        None if axis_dragged => ScrollMode::ClipOnly,
        None => ScrollMode::NoClip,
    }
}

/// **垂直轴是不是"视口"**（纯函数，可单测）：显式 `.vscroll(非 NoClip)`，或**另有**垂直视口
/// 来源（`has_fixed_v`：调用方把 `.height(..)` / `.placement(Clip)` 并进来）。
///
/// `NoClip`（默认）= 高度由内容决定 —— 此时"拖高"没有意义（内容当帧就把它顶回去）。
pub(super) fn v_axis_is_viewport(vscroll: Option<ScrollMode>, has_fixed_v: bool) -> bool {
    has_fixed_v || vscroll.is_some_and(|m| m != ScrollMode::NoClip)
}

/// **`.resize(bool)` 的判定表**（纯函数，可单测；egui 风）：
/// 垂直轴是视口 ⇒ **垂直 + 水平**都能拖；否则**只有水平**（高度由内容定，拖它没意义）。
///
/// `allow = false` ⇒ [`Resize::None`]（不画柄也不响应）。
pub(super) fn resolve_resizable_axes(allow: bool, v_is_viewport: bool) -> Resize {
    match (allow, v_is_viewport) {
        (false, _) => Resize::None,
        (true, true) => Resize::Both,
        (true, false) => Resize::Horizontal,
    }
}

/// **按轴求交裁剪层**（纯函数，可单测）：`ScrollMode::NoClip` 的那条轴**不裁**
/// （另一条轴照裁）——于是"只裁纵向"的窗口内容仍能横向溢出（反之亦然）。
///
/// - 两条轴都 `NoClip` 且无外层裁剪 ⇒ 返回 `None`（**与不加本 API 之前逐字节一致**）；
/// - 只有一条轴裁 ⇒ 另一条轴沿用外层裁剪；没有外层裁剪时用 `fallback`
///   （调用方传**屏幕矩形**：不裁 ≠ 无限，只是"别用窗口边界去裁"）；
/// - 函数名里的"按轴"是重点：旧的 `clip_for_view(.., Clip)` 会把两条轴一起裁，
///   那正是"只想要纵向滚动条，却被横向也裁掉"的原因。
pub(super) fn clip_for_axes(
    saved: Option<Rect>,
    win: Rect,
    fallback: Rect,
    v: ScrollMode,
    h: ScrollMode,
) -> Option<Rect> {
    if v == ScrollMode::NoClip && h == ScrollMode::NoClip {
        return saved;
    }
    let mut out = saved.unwrap_or(fallback);
    if h != ScrollMode::NoClip {
        // 横向收窄到窗口的 x 范围，纵向保持外层的。
        out = Rect::new(win.x, out.y, win.w, out.h);
    }
    if v != ScrollMode::NoClip {
        out = Rect::new(out.x, win.y, out.w, win.h);
    }
    if let Some(s) = saved {
        let x0 = out.x.max(s.x);
        let y0 = out.y.max(s.y);
        let x1 = (out.x + out.w).min(s.x + s.w);
        let y1 = (out.y + out.h).min(s.y + s.h);
        if x1 <= x0 || y1 <= y0 {
            return Some(Rect::new(x0, y0, 0.0, 0.0));
        }
        out = Rect::new(x0, y0, x1 - x0, y1 - y0);
    }
    Some(out)
}

/// **两个裁剪层求交**（纯函数，可单测）：窗口的"内容盒 scissor"与命令**已经带着**的
/// 内层裁剪（文本框盒 / 内层滚动视口）必须**求交**而不是覆盖 —— 覆盖会把内层更窄的
/// scissor 吃掉（文字画到框外）。任一为 `None` ⇒ 取另一个。
fn clip_and(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) => {
            let x0 = a.x.max(b.x);
            let y0 = a.y.max(b.y);
            let x1 = (a.x + a.w).min(b.x + b.w);
            let y1 = (a.y + a.h).min(b.y + b.h);
            if x1 <= x0 || y1 <= y0 {
                return Some(Rect::new(x0, y0, 0.0, 0.0));
            }
            Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
        }
    }
}

/// **嵌套容器的内容最大宽**（纯函数，可单测）：父级可用宽扣掉本容器内边距。
///
/// `None`（父级不限宽）/ 结果为 `<= 0` ⇒ `None`（不限）——"父级可用宽 0"不该把子项
/// 压成 0 宽（那会让内容彻底看不见），交回原来的自然排版。
///
/// 这一条是"**指定 width 的窗口里，嵌套容器（`row` / `panel` / `view`）不会把内容排到
/// 窗口外面**"的关键：子容器继承该上限后，子项被 clamp、`LimitedInParent` 控件拿到
/// [`Ui::avail_w`] 后自动换行（用户实测："width 较小，控件会突出去，直到你去拖拽缩放"）。
pub(super) fn content_max_w(parent_avail: Option<f32>, pad_total: f32) -> Option<f32> {
    parent_avail.and_then(|w| {
        let inner = w - pad_total * 2.0;
        (inner > 0.0).then_some(inner)
    })
}

/// **窗口内容是否强制裁剪**（纯函数，可单测；**旧的"一体化"判据**）。
///
/// - `strict`（`.placement(Placement::Clip)`）：应用的显式选择；
/// - `fixed_h = Some(..)`：**高度被用户拖过**（`Resize::Both` 的柄）⇒ 窗口成了"固定
///   尺寸视口"，内容撑不高它；不裁剪就会画到窗口外面（用户实测的 TTT 窗口 bug）。
///   `.width(..)` 不触发本项：固定宽但高度自然时，内容在垂直方向不会溢出。
///
/// ⚠ `window_impl` 现在走**按轴**的 [`resolve_scroll_mode`] + [`clip_for_axes`]；本函数
/// **只剩测试用途**（等价性基准：两条轴都按老判据解算时必须与它一致）⇒ `#[cfg(test)]`，
/// 免得"没人用的旧判据"留在生产代码里被误用。
#[cfg(test)]
pub(super) fn window_content_clipped(strict: bool, fixed_h: Option<f32>) -> bool {
    strict || fixed_h.is_some()
}

/// 窗口限位（`WindowClamp::Screen` 模式）：把绝对位置 clamp 到窗口客户区内。
/// - 窗口 ≤ 屏幕：整体在屏幕内，左上角 ∈ `[0, sw-size]`（贴边）；
/// - 窗口 > 屏幕：允许左上角 ∈ `[sw-size, 0]`（窗口**仍覆盖屏幕**，可拖动）——
///   否则窗口比画面大时被钉死在左上角、永远拖不走。
pub(super) fn clamp_window_pos(abs: Vec2, size: Vec2, sw: f32, sh: f32) -> Vec2 {
    let min_x = (sw - size.x).min(0.0);
    let max_x = (sw - size.x).max(0.0);
    let min_y = (sh - size.y).min(0.0);
    let max_y = (sh - size.y).max(0.0);
    Vec2::new(abs.x.clamp(min_x, max_x), abs.y.clamp(min_y, max_y))
}

