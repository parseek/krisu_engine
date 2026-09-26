//! 面板与「位置 / 尺寸责任链」：`panel_at` / `drag_panel_at` / `panel_impl`、
//! `pos_handler` / `size_handler` 与其解析、`with_id` / `id_for`。
//!
//! 维护者笔记：责任链优先级降序、第一个 `Some` 生效；内置用户拖拽结果恒为优先级 0。
//! 新增来源请走 `*_handler` 而不是在解析点写 if 分支。

use super::cmds::{clear_drag_base, resolve_drag};
use super::*;

use glam::Vec2;
use rjw_transform::Rect;

use crate::draw::Position;
use crate::hit::{hit_test, window_occluded};
use crate::id::{IdAbsolute, IdRelative};
use crate::layout::{Frame, PackSide};
use crate::style::PanelStyle;

impl<'a> Ui<'a> {
    /// **窗口/面板位置责任链**：注册一个位置解析器（脚本 / 动画 / 自动布局提供者）。
    ///
    /// 解析顺序（**优先级降序**，第一个返回 `Some` 的生效）：
    /// 1. 应用注册的处理器（`priority` 越大越先问）；
    /// 2. 内置**用户拖拽状态**（[`UiState::panel_pos`]，固定优先级 `0`）——用户拖过
    ///    就永远赢过负优先级脚本，松开后停在用户放置处；
    /// 3. 调用者传入的 `pos`（终端兜底，恒最后）。
    ///
    /// 优先级选择：
    /// - `priority < 0`（如 `-10`）：动画 / 自动布局——**用户拖拽优先**（拖拽中
    ///   `panel_pos` 先于脚本被询问，窗口跟手；脚本不阻塞拖动）；
    /// - `priority > 0`（如 `+10`）：**脚本锁定位置**——程序控制优先，拖拽被覆盖
    ///   （切场景锁窗口 / 剧情镜头等）；脚本返回 `None` 即交还控制权。
    ///
    /// **闭包须 `'static`**：可捕获拥有值 / `Copy` 值（如 [`std::time::Instant`] 时间
    /// 基准）/ `Arc`；需要与主循环共享可变状态时用 `Arc<Mutex<_>>`。这保证处理器
    /// 不借用 `self`——`ui.finish()` 之后应用仍可正常访问自己的状态。
    ///
    /// 示例（HUD 自动左右摆动，但用户仍可拖动——`-10 < 0` 拖拽优先）：
    /// ```no_run
    /// # let viewport = todo!(); let mouse = todo!(); let keyboard = todo!();
    /// # let text = todo!(); let mut backend = rjw_ui::RecordingBackend::default(); let state = todo!(); let window = todo!();
    /// use rjw_ui::{Theme, Ui, UiAdd};
    /// let mut ui = Ui::begin(&window, &mut text, &mut state)
    ///     .capture(&mouse, &keyboard)
    ///     .theme(Theme::dark())
    ///     .build();
    /// let t0 = std::time::Instant::now();
    /// ui.pos_handler(-10, move |id| {
    ///     if id == "hud" {
    ///         let t = t0.elapsed().as_secs_f64();
    ///         Some(glam::Vec2::new(400.0 + 120.0 * (t * 2.0).sin() as f32, 40.0))
    ///     } else {
    ///         None
    ///     }
    /// });
    /// ui.window("hud").pos(glam::Vec2::new(400.0, 40.0)).show(|w| { w.label("HUD"); });
    /// ui.finish(&mut backend);
    /// ```
    pub fn pos_handler(&mut self, priority: i32, f: impl Fn(&str) -> Option<Vec2> + 'static) {
        self.pos_chain
            .push((priority, PosLink::Script(Box::new(f))));
        // 优先级降序（稳定排序：同优先级保持注册顺序）
        self.pos_chain.sort_by_key(|e| std::cmp::Reverse(e.0));
    }

    /// 责任链解析窗口/面板位置（见 [`Self::pos_handler`]）。
    #[inline]
    pub(super) fn resolve_pos(&self, id: &IdAbsolute<'_>, pos: Vec2) -> Vec2 {
        resolve_pos_link(&self.pos_chain, &self.state.panel_pos, id, pos)
    }

    /// **自动窗口位置**（Win32 `CW_USEDEFAULT` 语义）：没写 `.pos()` 的窗口由引擎
    /// 按**首次出现顺序**级联分配（见 [`auto_pos_slot`]），并记进
    /// [`UiState::auto_pos`] 跨帧稳定。
    ///
    /// 优先级：`pos_handler` 脚本 > 用户拖拽（[`UiState::panel_pos`]）> **本自动位置**
    /// ——自动位置只是"初值"，拖过之后停在用户放置处（`reset()` 会连同记录一起清空）。
    pub(super) fn auto_window_pos(
        &mut self,
        id: &IdAbsolute<'_>,
        viewport: Vec2,
        scale: f32,
    ) -> Vec2 {
        auto_pos_take(
            &mut self.state.auto_pos,
            &mut self.state.auto_next,
            id,
            viewport,
            scale,
        )
    }

    /// **尺寸责任链**：注册可调尺寸控件（[`Self::resizable_text_area_at`] /
    /// [`Self::resizable_text_input_at`]）的**尺寸**处理器（如脚本/动画/外部布局约束）。
    ///
    /// `priority` 高的优先；返回 `Some(size)` 即生效，`None` 落到低优先级 /
    /// 用户拖拽缩放（[`UiState::sizes`]）/ 传入 rect 尺寸兜底。语义同 [`Self::pos_handler`]：
    /// `'static` 闭包，不借用 `self`（共享可变状态用 `Arc<Mutex<_>>`）。
    pub fn size_handler(&mut self, priority: i32, f: impl Fn(&str) -> Option<Vec2> + 'static) {
        self.size_chain
            .push((priority, SizeLink::Script(Box::new(f))));
        self.size_chain.sort_by_key(|e| std::cmp::Reverse(e.0));
    }

    /// 责任链解析可调尺寸控件尺寸（见 [`Self::size_handler`]）。
    #[inline]
    pub(super) fn resolve_size(&self, id: &IdAbsolute<'_>, fallback: Vec2) -> Vec2 {
        resolve_size_link(&self.size_chain, &self.state.sizes, id, fallback)
    }

    /// **解析可调尺寸控件的**当前**尺寸**（尺寸责任链 → 用户拖拽持久值
    /// [`UiState::sizes`] → `fallback`）——**控件作者公开面**。
    ///
    /// 与 [`Self::resize_handle`] 配对使用：`resize_handle` 负责**改**尺寸（拖拽 +
    /// 写持久值），本方法负责**读**它。自定义"可拖拽缩放"控件必须在 `ui()` 里
    /// **申请之前**调用它，把上一帧的持久尺寸并进本帧的申请尺寸——否则会出现
    /// "画的是拖大的框、申请的却是默认尺寸"：容器不跟着长、后面的控件不动，
    /// 而框本身溢出父级（`TextEditor` 就踩过这条）。
    ///
    /// ```no_run
    /// # use glam::Vec2;
    /// # use rjw_ui::{Ui, Resize};
    /// # fn demo(ui: &mut Ui, id: &str, value: &mut String) {
    /// // ① 想好默认尺寸（物理像素）
    /// let want = Vec2::new(240.0, 60.0);
    /// // ② 责任链 / 用户拖拽值优先
    /// let size = ui.resolved_size(id, want);
    /// // ③ 用解析后的尺寸申请（容器尺寸随它走）
    /// let rect = ui.allocate(size);
    /// // ④ 交给 `resize_handle` 拖拽（`current` 传解析后的尺寸）
    /// # let _ = (rect, Resize::Both);
    /// # }
    /// ```
    pub fn resolved_size(&mut self, id: &str, fallback: Vec2) -> Vec2 {
        let abs = self.id_for(id);
        self.resolve_size(&abs, fallback)
    }

    /// 面板：背景 + 边框 + 内容垂直堆叠（pack Top）；尺寸自动包裹内容。
    pub fn panel_at(
        &mut self,
        pos: impl Into<Position>,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        self.panel_impl(pos, None, None, f)
    }

    /// **可拖拽**面板：同 [`Self::panel_at`]，且按住面板任意处**移动 ≥ 3 物理像素**
    /// 可拖动（纯点击不拖拽，面板内子控件正常响应）。
    ///
    /// - 位置持久化于 `UiState.panel_pos`（`id` 须稳定），跨帧跟随鼠标；
    ///   也可经**位置责任链**（[`Self::pos_handler`]）由脚本/动画提供——用户拖拽
    ///   始终优先于负优先级脚本；
    /// - 真正拖动期间**抑制面板内子控件交互**（不会误触发按钮点击）；
    /// - `pos` 为初始位置（首次）；`UiState::reset()` 可复位。
    pub fn drag_panel_at(
        &mut self,
        id: &str,
        pos: impl Into<Position>,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        self.panel_impl(pos, Some(id), None, f)
    }

    /// 面板公共实现：`drag = Some(id)` 时启用拖拽；`style` 逐面板覆盖（`None` = 全局
    /// [`Theme::panel`]）。
    pub(super) fn panel_impl(
        &mut self,
        pos: Vec2,
        drag: Option<&str>,
        style: Option<&PanelStyle>,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        // 拖拽面板的位置从**责任链**读取（脚本处理器 → 用户拖拽状态 → 传入 pos，
        // 见 pos_handler）：首次 / 从未拖过时用传入 pos
        // 面板自身也是命名空间边界（可拖拽面板有稳定 id）——先解析绝对 id 供状态键用。
        let abs = drag.map(|id| self.id_for(id));
        let origin = match abs.as_ref() {
            Some(a) => self.resolve_pos(a, pos),
            None => pos,
        };
        let start = self.painter.q.queue.len();
        self.begin_top_placement();
        let style = style.cloned().unwrap_or_else(|| self.theme.panel.clone());
        let (pad_total, gap) = (style.padding + style.border_w, self.theme.gap);
        let saved_base = self.abs_base;
        // ─── ① 位置与交互**先于内容录制**求解（同 `window_impl`）──────────
        // 鼠标事件是针对**屏幕上已有的几何**（上一帧结算的 `panel_sizes`）产生的，
        // 故命中 / 拖拽基准用上一帧矩形；由此 `display_pos` 在录制前已知，
        // `abs_base` 与随后 `translate(display_pos)` 的几何**当帧一致**。
        // 旧实现 `abs_base` 用上一帧位置、几何用本帧位置：拖动面板时面板内文本框
        // 的 `box_clip` / 光标 / 滑块基准落后一帧（快速拖动时文字被裁、点击错位）。
        let prev_size = abs
            .as_ref()
            .and_then(|a| self.state.panel_sizes.get(a.as_str()).copied());
        let panel_rect = prev_size.map(|ps| Rect::new(origin.x, origin.y, ps.x, ps.y));
        let btn = self.mouse_left();
        // 窗口遮挡：面板是 win=0 内容（绘制在所有窗口之下），被任意窗口覆盖时不可拖拽。
        // 首帧无 `prev_size` ⇒ 本帧不参与交互（面板尚未被看到）。
        let hit = panel_rect.is_some_and(|r| {
            hit_test(&r, self.mouse_logical)
                && self.mouse_in_window
                && !window_occluded(0, self.mouse_logical, self.window_rects_iter())
        });
        let press_here = btn.down_edge() && hit;
        // 拖拽交互：**物理像素粒度**拖动基准（见下方说明）。按下帧先无条件建立
        // 基准；面板内子控件随后声明本次按下（`press_claimed`）时在 ② 清除。
        let (active, display_pos) = match abs.as_ref() {
            Some(a) => {
                let ws = self.state.widgets.entry(a.to_static()).or_default();
                resolve_drag(ws, hit, btn, self.mouse_screen, origin)
            }
            None => (false, origin),
        };
        // 内容基准 = **本帧显示基准**（`display_pos`）——录制期的绝对空间量与几何一致。
        self.abs_base = saved_base + display_pos;
        self.frames
            .push(Frame::new_stack(PackSide::Top, gap, pad_total));
        self.painter.q.depth += 1;
        let mut panel = Panel { ui: self };
        f(&mut panel);
        let frame = self.frames.pop().expect("panel frame");
        let size = frame.settle_size();
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        // ─── ② 内容录完后：按下裁决 + 位置持久 ──────────────────────────
        if let Some(a) = abs.as_ref() {
            if press_here {
                let ws = self.state.widgets.entry(a.to_static()).or_default();
                if self.press_claimed {
                    // 文本框等子控件按下（选择拖拽优先）：清除基准（见 `window_impl`）。
                    clear_drag_base(ws);
                }
            }
            // 结算尺寸跨帧持久：下帧命中 / 拖拽基准 = 屏幕上那个矩形。
            self.state.panel_sizes.insert(a.to_static(), size);
            if active {
                self.drag_panel = Some(a.to_static());
                // 仅位置变化时写入（滞回：同一位置不重写）
                if self.state.panel_pos.get(a.as_str()) != Some(&display_pos) {
                    self.state.panel_pos.insert(a.to_static(), display_pos);
                }
            } else if self
                .drag_panel
                .as_ref()
                .is_some_and(|d| d.as_str() == a.as_str())
            {
                self.drag_panel = None;
            }
            if press_here || active {
                // 按下面板（或拖拽中）都算"已响应按下"——避免空白点击清焦点
                self.any_pressed = true;
            }
            // 面板拖动激活 → 强制普通 Arrow（UI_NEEDS：窗体拖动无需 <->）。
            if active {
                self.cursor_window_drag = true;
            }
        }
        // 背景 + 边框（depth = 进入前深度，画在子控件之下；radius > 0 走圆角双层矩形）
        let bg_rect = Rect::new(0.0, 0.0, size.x, size.y);
        self.push_panel_shadow(bg_rect, &style.shadow, style.radius);
        self.push_panel_like_img(
            bg_rect,
            style.bg,
            style.bg_image,
            style.border,
            style.border_w,
            style.radius,
            0,
        );
        // 平移全部（子命令 + 背景/边框）：
        // 用 `display_pos`（拖拽中 = 本帧新位置）→ 文字/矩形**当帧生效**。
        for d in &mut self.painter.q.queue[start..] {
            d.translate(display_pos);
        }
        size
    }

    /// **区块级容器作用域**：作用域内**不新开顶层放置**（`place`），退出时恢复。
    ///
    /// 「区块级容器」= 一个逻辑区块由**多条命令 / 多个子项**组成，但对外只有**一个**
    /// 排序与缓存单元 —— 目前只有两个用户：
    /// - [`Self::foldable`](crate::Ui::foldable)：标题行 + 正文（正文不另开 frame，但**要**避免
    ///   父 frame 的 `gap` 在"标题 → 正文"之间被多推进一次）；
    /// - [`Self::namespace`](crate::Ui::namespace)：正文（空命名空间因此**不产生空放置**）。
    ///
    /// `placement_push` / `placement_pop` 用**闭包作用域**保证成对（`?` / panic 也安全，
    /// 与 [`Self::with_id`] 同一套理由）。
    pub(crate) fn container_scope<R>(&mut self, f: impl FnOnce(&mut Ui<'a>) -> R) -> R {
        self.painter.q.placement_push();
        let out = f(self);
        self.painter.q.placement_pop();
        out
    }

    /// 在 `id_relative` 命名空间内执行 `f`：进入压栈、退出弹栈。
    /// **闭包作用域保证配对**（借用检查器 + 栈帧语义，`?`/panic 也安全）——
    /// 消除手动 `push_id`/`pop_id` 的漏配对/多弹出风险。容器实现
    /// （window / scroll / grid）用。`id_relative` 为相对名字（容器的命名空间段）。
    pub(crate) fn with_id<'s>(
        &mut self,
        id_relative: impl Into<IdRelative<'s>>,
        f: impl FnOnce(&mut Ui<'_>),
    ) {
        self.ids.push(id_relative.into());
        f(self);
        self.ids.pop();
    }

    /// 根据当前命名空间栈与相对 id 生成**绝对 id**（状态键 / 焦点 id 用）。
    ///
    /// - 顶层（栈空）返回 `Borrowed`——**零拷贝零分配**；
    /// - 嵌套返回 `Owned`（一次拼接）。
    ///
    /// 类型安全：`id_relative` 只接受相对名字（[`IdRelative`] / `&str`），已解析的
    /// 绝对 id **无法**再传进来（双重前缀编译期报错）。
    ///
    /// 生命周期 `'l` 绑定**传入的名字**（而非 `&mut self` 的 `'s`）：调用后 `self` 借用
    /// 释放，返回值可继续用于后续 `&mut self` 操作（`register_focus` / 状态读写）。
    pub fn id_for<'s, 'l>(&'s mut self, id_relative: impl Into<IdRelative<'l>>) -> IdAbsolute<'l> {
        self.ids.id_for(id_relative.into())
    }
}
