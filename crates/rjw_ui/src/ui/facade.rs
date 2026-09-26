//! `Ui` 门面：状态 / 主题访问、帧级事实搬运、光标意图、焦点注册与诊断。
//!
//! 维护者笔记：本模块只做「把 `Ui` 的私有字段按语义读写成公开面」，不产生绘制命令、
//! 不推进布局。拿不准某方法该不该在这里时问：它是否只读 / 只写 `Ui` 自身的状态字段？
//! 是 ⇒ 这里；牵扯录制序（`next_seq`）或布局帧 ⇒ `interaction` / `panel`。

use super::*;

use glam::Vec2;
use rjw_keyboard::KeyCode;
use rjw_transform::Rect;
use winit::dpi::PhysicalPosition;

use crate::draw::Position;
use crate::focus::{FocusEntry, FocusKind};
use crate::hit::update_drag;
use crate::id::IdAbsolute;
use crate::state::UiState;
use crate::style::Theme;

impl<'a> Ui<'a> {
    /// 跨帧 UI 状态（只读；交互控件状态持久于此）。
    #[inline]
    pub fn state(&self) -> &UiState {
        self.state
    }
    /// 跨帧 UI 状态（可变；控件作者用 [`UiState::widget`] 读写指定 ID 的状态）。
    #[inline]
    pub fn state_mut(&mut self) -> &mut UiState {
        self.state
    }

    /// 主题（只读；每帧由 [`UiInit::theme`] / `Frame::ui` 传入）。
    ///
    /// 需要逐控件改样式时优先用 widget builder（`.color(..)` 等）；确实要改全局主题
    /// 用 [`Self::theme_mut`]（只影响**本帧后续**录制——每帧 `Ui::begin` 会重新传入）。
    #[inline]
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// 主题（可变；**只影响本帧后续录制**——每帧 [`Ui::begin`] 会重新传入主题）。
    #[inline]
    pub fn theme_mut(&mut self) -> &mut Theme {
        &mut self.theme
    }

    /// DPI scale factor（物理像素 / 逻辑像素，如 1.0 / 1.5 / 2.0）。
    ///
    /// **仅公开 API 边界** [`Size`](crate::draw::Size) / [`Position`](crate::draw::Position)
    /// 的 Logical→Physical 换算用——内部布局 / 命中 / 绘制一律物理像素（Theme 已预乘，
    /// 见 [`Theme::scaled`](crate::style::Theme::scaled)）。
    #[inline]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// 鼠标**物理**屏幕坐标（warp 边缘判定 / 物理像素增量拖拽用）。
    #[inline]
    pub fn mouse_screen(&self) -> Vec2 {
        self.mouse_screen
    }

    /// 按键本帧按下边沿（控件作者键盘交互用，如 Esc 关闭模态对话框）。
    #[inline]
    pub fn key_down_edge(&self, key: winit::keyboard::KeyCode) -> bool {
        self.keyboard.key(key).down_edge()
    }

    /// 按键**当前是否按住**（控件作者键盘交互用，如 Shift/Ctrl 修饰拖拽速度）。
    #[inline]
    pub fn key_down(&self, key: winit::keyboard::KeyCode) -> bool {
        self.keyboard.key(key).pressed()
    }

    /// 当前容器光标位置（局部坐标；自写"占光标"式组合布局时用于放置子容器）。
    #[inline]
    pub fn cursor_pos(&self) -> Vec2 {
        self.frames.last().map(|f| f.cursor).unwrap_or(Vec2::ZERO)
    }

    /// **当前容器内容原点的绝对坐标**（物理像素；`abs_base`）——**组合控件作者公开面**。
    ///
    /// 自绘浮层（取色器面板 / 第三方下拉）要把"当前容器局部"的锚点换算成**屏幕绝对**
    /// 坐标才能判断"放不放得下"；`Ui::abs_rect` 是它的常用封装。
    #[inline]
    pub fn content_origin(&self) -> Vec2 {
        self.abs_base
    }

    /// **局部矩形 → 绝对矩形**（物理像素；与 [`Self::content_origin`] 配套）。
    ///
    /// 用途：组合控件在"当前容器局部"里拿到自己的矩形后，需要绝对坐标去做**屏幕级**
    /// 决策（浮层该朝哪边翻、会不会出屏幕）。引擎侧同名的绝对量是 `abs_base`。
    #[inline]
    pub fn abs_rect(&self, local: Rect) -> Rect {
        Rect::new(
            self.abs_base.x + local.x,
            self.abs_base.y + local.y,
            local.w,
            local.h,
        )
    }

    /// **声明本次按下归本控件**：自定义交互控件（自身有**拖拽语义**，如数字输入的
    /// 拖动手柄）在 `down_edge && hit` 时调用——阻止外层窗口/面板把本次按下当作
    /// **窗口拖拽基准**（否则窗口内拖滑块/手柄会连窗口一起动）。内置滑块 / 滚动条
    /// / 文本框已自行调用。
    #[inline]
    pub fn claim_press(&mut self) {
        self.press_claimed = true;
    }

    /// **声明本次按下被某个控件响应**（控件作者用）：帧末"点空白处清焦点"的判定要
    /// 区分"按在了控件上"与"按在了空白处"——自定义控件（如 [`crate::Dropdown`] 的
    /// 触发器）按下时调它。
    #[inline]
    pub(crate) fn note_press_handled(&mut self) {
        self.any_pressed = true;
    }

    /// **进入一层浮层**（控件作者用：下拉 / 菜单 / 取色面板…）：返回本层该用的 z
    /// （[`overlay_z`]`(嵌套层数)`），并把层数 +1。配套 [`Self::pop_overlay`]。
    ///
    /// 用法：`let z = ui.push_overlay_z(); state.window_z.insert(id, z); …录浮层窗口… ;
    /// ui.pop_overlay();`
    ///
    /// ⚠ 浮层里再开浮层（子菜单 / 菜单里的取色器）**必须**走这一对方法：同 z 会让两层的
    /// 绘制命令落进同一个 `(win, elem)` 分组，子层的阴影 / 背景（`elem = 0`）被父层控件
    /// （`elem ≥ 1`）盖住（用户实测："下级 popup 阴影被绘制在了上级控件后面"）。
    #[inline]
    pub(crate) fn push_overlay_z(&mut self) -> u32 {
        let z = overlay_z(self.overlay_depth);
        self.overlay_depth += 1;
        z
    }

    /// **退出一层浮层**（与 [`Self::push_overlay_z`] 配对）。
    #[inline]
    pub(crate) fn pop_overlay(&mut self) {
        self.overlay_depth = self.overlay_depth.saturating_sub(1);
    }

    /// **通用拖拽缩放柄**（控件作者原语）：`handle` 为**当前容器局部坐标**的柄矩形
    /// （通常右下角）。按住拖拽把 `current` 改为新尺寸（返回 `Some(new)`；`None` =
    /// 本帧无变化）；范围 clamp 到 `min`。拖动中置位 `press_claimed`（阻止外层
    /// 窗口/面板把本次按下当拖拽基准），悬停/拖拽显示 `cursor`（如 ↔ / ↖↘）。
    ///
    /// 持久尺寸由调用方写入（推荐 [`UiState::sizes`]）；可缩放 widget 的 `size()`
    /// 优先读持久值（见 [`Self::resolved_size`]），用 [`crate::Resize`] 声明轴向
    /// （如 [`crate::TextEditor::resize`]）。
    /// `window(width)` / `Placement::Clip` 的宽度缩放即基于本原语。
    ///
    /// = [`Self::resize_handle_hit`]（命中 + 光标，**应在内容之前**）+ 本体的拖拽应用。
    /// 需要"让内容里的控件优先"的调用方（窗口右下角柄 vs 面板内最后一个控件的柄重叠）
    /// 要把这两步**拆开**：命中登记早、应用晚（见 [`Self::window_impl`]）。
    pub fn resize_handle(
        &mut self,
        id: &str,
        handle: Rect,
        current: Vec2,
        min: Vec2,
        cursor: crate::UiCursor,
    ) -> Option<Vec2> {
        let hit = self.resize_handle_hit(id, &handle, cursor);
        self.resize_handle_apply(id, current, min, cursor, hit)
    }

    /// [`Self::resize_handle`] 的**上半**：命中判定 + 光标（登记命中区）。
    ///
    /// ⚠ **登记顺序就是优先级**：控件级遮挡按"谁后登记谁在上"判定 ⇒ 想让内容里的控件
    /// 压过窗口自己的柄，窗口必须在**内容之前**调本方法（后登记的内容控件因此获胜）。
    /// ⚠ **本方法不置位 `press_claimed`**（认领在 [`Self::resize_handle_apply`]）：
    /// 窗口路径要在内容之后凭 `!press_claimed` 判断"内容有没有抢走这次按下"，若这里就
    /// 认领，窗口会把自己挡住（实测：`--sim-resize` 变成"柄点不动"）。
    pub(crate) fn resize_handle_hit(
        &mut self,
        id: &str,
        handle: &Rect,
        cursor: crate::UiCursor,
    ) -> bool {
        let abs = self.id_for(id);
        let hhit = self.hit_abs(&abs, handle);
        if hhit {
            self.set_cursor(cursor);
        }
        hhit
    }

    /// [`Self::resize_handle`] 的**下半**：把拖拽落到尺寸上（`hit` 来自上半）。
    ///
    /// 调用方通常**只看 `!press_claimed` 再调**：内容里的控件已经认领这次按下时，
    /// 外层（窗口）的柄必须让位——否则"点面板内最后一个控件的缩放柄"会连带把窗口也缩了。
    pub(crate) fn resize_handle_apply(
        &mut self,
        id: &str,
        current: Vec2,
        min: Vec2,
        cursor: crate::UiCursor,
        hit: bool,
    ) -> Option<Vec2> {
        let abs = self.id_for(id);
        let hbtn = self.mouse_left();
        if hbtn.down_edge() && hit {
            // 缩放柄自身有拖拽语义：阻止外层窗口/面板把本次按下当作拖拽基准。
            self.press_claimed = true;
        }
        let mut new = current;
        let active = {
            let ws = self.state.widgets.entry(abs.to_static()).or_default();
            // **松手即清基准**：基准属于"某一次按住"，跨次复用会让下一次拖拽以上一次的
            // 基准结算 ⇒ 尺寸每帧按位移**连乘**下去（用户实测："窗口收缩高度应当为恒定值"：
            // 往上收缩时高度一档一档掉到下限，明明鼠标已经停了）。
            if !hbtn.pressed() {
                ws.press_mouse = None;
                ws.press_panel = None;
            }
            let was_dragging = ws.dragging;
            let a = update_drag(ws, hit, hbtn);
            // 基准**只在按下边沿**捕获（`hbtn.pressed()` 也要为真）。
            //
            // ⚠ 曾经这里还有 `|| !was_dragging` 的"兜底补捕获"：一旦按键状态在某帧读到
            // "未按下"（真实鼠标在窗口外、注入式输入、丢帧），`dragging` 被清掉 ⇒ 下一帧
            // 又满足 `!was_dragging` ⇒ **拿当前尺寸 + 当前鼠标重新立基准**，于是每帧再加
            // 一次位移 ⇒ 尺寸**无限增长**（用户实测："加了 resize Both 则无限增高"）。
            // 现在：没有基准的那一帧就**不生效**（少走一帧），而不是重新立基准。
            let _ = was_dragging;
            if hbtn.down_edge() && hbtn.pressed() && hit {
                ws.press_mouse = Some(self.mouse_screen.round());
                ws.press_panel = Some(current);
            }
            if a {
                let pm = ws.press_mouse.unwrap_or(self.mouse_screen);
                let base = ws.press_panel.unwrap_or(current);
                let d = (self.mouse_screen - pm).round();
                new = Vec2::new((base.x + d.x).max(min.x), (base.y + d.y).max(min.y));
            }
            if std::env::var_os("RJ_GRIP_TRACE").is_some() {
                eprintln!(
                    "grip[{id}] hit={hit} edge={} drag={a} cur={current:?} press_m={:?} press_p={:?} mouse={:?} -> new={new:?}",
                    hbtn.down_edge(),
                    ws.press_mouse,
                    ws.press_panel,
                    self.mouse_screen
                );
            }
            a
        };
        if active {
            self.set_cursor(cursor);
        }
        active.then_some(new)
    }

    /// **设置本帧系统光标**（控件作者用）：如数字输入拖动手柄悬停/拖拽时
    /// [`UiCursor::EwResize`]（↔），点击文本框时由内置逻辑显示 I 型。优先级低于
    /// 内置拖拽（滑块/滚动条抓握）、高于 I 型文本光标；窗体悬停/拖动保持默认箭头。
    #[inline]
    pub fn set_cursor(&mut self, icon: UiCursor) {
        self.cursor_custom = Some(icon.to_winit());
    }

    /// 窗口客户区**物理尺寸**（`(w, h)` 像素；拖拽调值的 warp 边缘判定用）。
    #[inline]
    pub fn window_physical_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width, s.height)
    }

    /// **窗口整窗口特效**：`tint`（混合色，淡入淡出/整窗染色）+ `transform`
    /// override（位移/缩放/旋转动画）。**每帧可改**；窗口顶点缓存不变，仅提交时
    /// 应用到窗口段实例（矩阵/颜色）。默认无 fx（`WindowFx::default()`）。
    pub fn window_fx(&mut self, id: &str, fx: WindowFx) {
        let abs = self.id_for(id);
        self.state.window_fx.insert(abs.to_static(), fx);
    }

    /// 视口**物理**尺寸（窗口客户区物理像素；锚定布局 / 全屏遮罩用）。
    #[inline]
    pub fn viewport_size(&self) -> Vec2 {
        let s = self.window.inner_size();
        Vec2::new(s.width as f32, s.height as f32)
    }

    /// **调试快照**（Rust 侧诊断）：把本帧"引擎眼里的世界"整成一个可打印的结构——
    /// 每个窗口的 **id / z / 本帧提交原点（`win_origins`）/ 尺寸 / 拖拽状态 / 持久位置**，
    /// 加焦点、文本焦点、鼠标、DPI、视口、帧号。
    ///
    /// 用途（`docs/DEBUGGING.md`）：
    /// - 排查"位置 / 层级 / 拖拽看着不对"时，**先看引擎状态**再怀疑渲染：
    ///   `log::info!("{}", ui.debug_dump())`（`RUST_LOG=rjw_ui=info`）或 `eprintln!`；
    /// - 也可交给外部工具（MCP / DAP 断点里 `println!("{}", ui.debug_dump())`）。
    ///
    /// 必须在**录制期**调用（`win_origins` 是帧内状态，`finish` 后清空）；
    /// 在 `f.ui(|ui| { ...; ui.debug_dump() })` 闭包末尾调用可拿到全部窗口。
    pub fn debug_dump(&self) -> UiDebugDump {
        let mut windows: Vec<UiWindowInfo> = Vec::new();
        // ⚠ 遍历**本帧录制过的窗口 id**（帧级、各段累加），不是 `Ui` 帧内的 `win_ids`
        // （**按 z 键** ⇒ 同一帧的多个 `WIN_TOPMOST` 浮层只会留下最后一个，嵌套子菜单
        // 从 dump 里消失）。原点取 `state.window_origins`（**按 id**）。
        for id in &self.state.frame_state.window_ids_seen {
            let Some(z) = self.state.window_z.get(id.as_str()).copied() else {
                continue;
            };
            let origin = self
                .state
                .window_origins
                .get(id.as_str())
                .copied()
                .unwrap_or(Vec2::ZERO);
            let size = self
                .state
                .window_sizes
                .get(id.as_str())
                .copied()
                .or_else(|| {
                    self.state
                        .window_rects
                        .get(id.as_str())
                        .map(|r| Vec2::new(r.w, r.h))
                })
                .unwrap_or(Vec2::ZERO);
            let ws = self.state.widgets.get(id.as_str());
            windows.push(UiWindowInfo {
                id: id.as_str().to_owned(),
                z,
                origin,
                size,
                dragging: ws.is_some_and(|w| w.dragging),
                press_panel: ws.and_then(|w| w.press_panel),
                stored_pos: self.state.panel_pos.get(id.as_str()).copied(),
                submit_pos: self.state.debug_submit.get(&z).copied(),
                clip: self.state.debug_clip.get(&z).copied(),
            });
        }
        windows.sort_by_key(|w| w.z);
        UiDebugDump {
            frame: self.state.frame,
            scale: self.scale,
            viewport: self.viewport_size(),
            mouse_px: self.mouse_screen,
            mouse_in_window: self.mouse_in_window,
            focused: self.state.focused.as_ref().map(|f| f.as_str().to_owned()),
            text_focus: self.state.text_focus().map(|f| f.id.as_str().to_owned()),
            windows,
        }
    }

    /// 按锚点计算**绝对物理 pos**（顶层容器用）：内容尺寸 `size` 在视口内按
    /// `anchor` 停靠、距视口边 `margin`（均为**物理像素**；内容超视口时 clamp 到
    /// 视口内不溢出）。返回 [`Position::Physical`]——直接传给 `label_at` / `add_at`
    /// 等（不参与 Logical→Physical 二次换算）。纯几何见 [`Self::anchor_pos_in`]。
    #[inline]
    pub fn anchor_pos(&self, a: Anchor, size: Vec2, margin: Vec2) -> Position {
        Position::Physical(Self::anchor_pos_in(self.viewport_size(), a, size, margin))
    }

    /// 锚定位置纯计算：`vp` 视口内按 `anchor` 停靠（可单测）。
    pub fn anchor_pos_in(vp: Vec2, a: Anchor, size: Vec2, margin: Vec2) -> Vec2 {
        let m = margin.max(Vec2::ZERO);
        let sx = (vp.x - m.x * 2.0).max(0.0);
        let sy = (vp.y - m.y * 2.0).max(0.0);
        let x = match a {
            Anchor::TopLeft | Anchor::CenterLeft | Anchor::BottomLeft => m.x,
            Anchor::TopCenter | Anchor::Center | Anchor::BottomCenter => m.x + (sx - size.x) * 0.5,
            Anchor::TopRight | Anchor::CenterRight | Anchor::BottomRight => {
                (vp.x - m.x - size.x).max(m.x)
            }
        };
        let y = match a {
            Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => m.y,
            Anchor::CenterLeft | Anchor::Center | Anchor::CenterRight => m.y + (sy - size.y) * 0.5,
            Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => {
                (vp.y - m.y - size.y).max(m.y)
            }
        };
        Vec2::new(x, y)
    }

    /// 设置鼠标光标**物理屏幕位置**（warp 用：拖到窗口边缘跳到对侧继续拖；
    /// 下一帧输入快照生效，配合拖拽基准偏移保持增量连续）。
    #[inline]
    pub fn set_cursor_position(&mut self, x: f32, y: f32) {
        let _ = self
            .window
            .set_cursor_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    }

    // （`phys_rect` / `phys_f` 已删除：内部坐标一律物理像素，`self.scale` 仅用于
    // API 边界 `Size` / `Position` 的 Logical→Physical 换算，不参与布局/命中/绘制。）

    // ── 键盘导航（焦点链） ────────────────────────────────────

    /// 把控件登记进本帧焦点链（键盘导航用）。`rect` 为**相对当前容器**的局部矩形，
    /// 内部转成**绝对逻辑坐标**（焦点描边绘制 / 排序用）。**交互控件必须调用**。
    /// `id` 为**绝对 ID**（控件内 `self.id_for(..)` 所得——状态键 / 焦点 id 必须一致）。
    pub fn register_focus(&mut self, id: &IdAbsolute<'_>, rect: Rect, kind: FocusKind) {
        let abs = Rect::new(
            self.abs_base.x + rect.x,
            self.abs_base.y + rect.y,
            rect.w,
            rect.h,
        );
        // 记录焦点控件的**类型**（`UiState::text_focus` 据此区分"文本焦点"与
        // "按钮/滑块焦点"：后者不应阻断应用快捷键）。Tab 改焦点时在 `finish` 同步。
        if self.focused_is(id) {
            self.state.focused_kind = Some(kind);
        }
        self.focusables.push(FocusEntry {
            id: id.to_static(),
            win: self.painter.q.cur_win,
            kind,
            depth: self.painter.q.depth,
            rect: abs,
            clip: self.painter.q.clip,
        });
    }

    /// 本控件是否持有键盘焦点（`UiState.focused == id`；`id` 为**绝对 ID**）。
    #[inline]
    pub(crate) fn focused_is(&self, id: &IdAbsolute<'_>) -> bool {
        self.state.focused.as_ref().is_some_and(|f| f.as_str() == id.as_str())
    }

    /// **键盘激活**：Enter / Space 在本帧按下、本控件持有焦点且不在 IME 组合中
    /// → 视为一次点击（按钮 / 勾选 / 单选 / 下拉框用）。文本输入框与滑块不参与
    /// （前者走打字路径，后者用方向键调值）。`id` 为**绝对 ID**。
    pub fn key_click(&self, id: &IdAbsolute<'_>, kind: FocusKind) -> bool {
        if !self.focused_is(id) || kind == FocusKind::TextInput || kind == FocusKind::Slider {
            return false;
        }
        let composing = self
            .keyboard
            .ime_preedit()
            .is_some_and(|p| !p.is_empty());
        if composing {
            return false;
        }
        self.keyboard.key(KeyCode::Enter).down_edge()
            || self.keyboard.key(KeyCode::Space).down_edge()
    }

    // ── Debug UI / DebugDraw（调试 rjw_ui 自身 + 屏幕空间调试图元） ──

    /// 调试 UI 布局**开启**（运行时切换；等价于 [`UiInit::debug_layout`]）。
    ///
    /// 开启后 `finish` 为**每一个录制命令的矩形**画青色描边（覆盖在 UI 内容之上）——
    /// 可视化每个控件 / 容器的布局矩形与命中区域。
    #[inline]
    pub fn debug_layout(&mut self) -> &mut Self {
        self.debug_layout = true;
        self
    }

    /// 调试 UI 布局**关闭**（默认）。
    #[inline]
    pub fn without_debug_layout(&mut self) -> &mut Self {
        self.debug_layout = false;
        self
    }

}

impl<'a> Ui<'a> {
    /// **窗口遮挡判定用的窗口矩形迭代器**（`(z, rect)`；逻辑像素）。
    ///
    /// ⚠ 表按**窗口绝对 ID** 键（[`UiState::window_rects`]），这里把每条的 z **解成"当前
    /// z"**：矩形是上一帧录下的，而 z 可能在帧末被"点击置顶"改过——若按矩形写入时的旧 z
    /// 参与比较，被抬高的那个窗口就会**在它下面的窗口面前"消失"一帧**（它画在上面却挡不住
    /// 别人）。按 ID 查当前 z 从根上消除这处错位。
    #[inline]
    pub(super) fn window_rects_iter(&self) -> impl Iterator<Item = (u32, Rect)> + '_ {
        self.state.window_rects.iter().map(|(id, &r)| {
            (
                self.state.window_z.get(id.as_str()).copied().unwrap_or(0),
                r,
            )
        })
    }

    /// **诊断**：当前窗口 z-order（按 z 升序）：`(id, z)`。
    pub fn window_order(&self) -> Vec<(String, u32)> {
        let mut v: Vec<(String, u32)> = self
            .state
            .window_z
            .iter()
            .map(|(id, &z)| (id.as_str().to_owned(), z))
            .collect();
        v.sort_by_key(|&(_, z)| z);
        v
    }

    /// **诊断**：鼠标下**最上层**的窗口（`id, z`）——重叠点击时唯一可交互的窗口；
    /// 鼠标不在任何窗口上时返回 `None`。窗口矩形来自跨帧缓存（含本帧已录制的窗口）。
    pub fn window_under_mouse(&self) -> Option<(String, u32)> {
        let mut best: Option<(String, u32)> = None;
        for (id, &z) in &self.state.window_z {
            if let Some(r) = self.state.window_rects.get(id.as_str())
                && r.contains_point(self.mouse_logical) {
                    match &best {
                        Some((_, bz)) if *bz >= z => {}
                        _ => best = Some((id.as_str().to_owned(), z)),
                    }
                }
        }
        best
    }

}

