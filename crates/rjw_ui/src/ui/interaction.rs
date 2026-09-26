//! 命中 / 申请 / 交互：`hit_abs` 家族、`allocate*` 家族、`interact`、`view_at`、
//! 通用容器 `container` / `ornament_at`、`avail_w`、`divider_at`。
//!
//! 维护者笔记：这里决定「一条命令算谁的、能不能点」——改动会影响遮挡 / 拖拽 / 焦点
//! 三条语义，务必同时看 `docs/DEBUGGING.md` 的命中与闪烁章节。

use super::*;
use super::window::content_max_w;

use glam::Vec2;
use rjw_keystate::KeyState;
use rjw_mouse::MouseButton;
use rjw_transform::Rect;

use crate::draw::{
    Position, Size,
};

/// **"待定满宽"命令回填后的宽度**（纯函数，可单测）：从命令的 `x` 一直到 `content_w`。
///
/// **只增不减**：容器比线还窄时保持原宽 —— 宁可留一条按老规则算的线，也不把分割线
/// 拉成 0 宽（"控件凭空消失"比"线短一截"更难排查，与 `layout::ADAPT_MIN_W` 同一条纪律）。
#[inline]
pub(crate) fn full_w_target(x: f32, w: f32, content_w: f32) -> f32 {
    w.max((content_w - x).max(0.0))
}

/// **装饰容器里"内容比盒矮"时的垂直居中偏移**（纯函数，可单测）：`(盒高 − 内容高) / 2`，
/// 内容更高时 `0`（不居中、由内容撑开）。
///
/// 用在 [`Ui::ornament_impl`]：让 `ui.foldable_custom(id, |t| { t.label("标题"); })` 与
/// `ui.foldable(id, "标题")` **长得一样** —— 文本标题的文字是 `TextVAlign::Center`
/// 居中于标题行，而自定义标题里的控件默认从容器顶排（实测差 `(39−21)/2 = 9` 物理像素）。
#[inline]
pub(crate) fn center_offset(box_h: f32, content_h: f32) -> f32 {
    ((box_h - content_h) * 0.5).max(0.0)
}
// 顶点收集 / 合批机制（原在 `ui.rs`，见 `gpu_batch` 模块文档）。
use crate::gpu_batch::fully_outside;
use crate::hit::{
    HitRegion, hit_test, id_hash, update_drag, update_interact,
    widget_occluded, window_occluded,
};
use crate::id::IdAbsolute;
use crate::layout::{Child, Frame, PackSide};
use crate::view::{clip_for_view, ViewCtx, ViewMode};

impl<'a> Ui<'a> {
    /// 鼠标左键状态（含本帧边沿；控件作者交互判断用）。
    #[inline]
    pub fn mouse_left(&self) -> KeyState {
        self.mouse.button(MouseButton::Left)
    }

    /// **记入当前容器**：绝对放置的控件矩形也要算进容器尺寸（`Frame::content_bounds`）。
    ///
    /// 所有"显式 rect"的公开控件入口（`button_at_styled` / `slider_at_drag` /
    /// `checkbox_at_styled` / `radio_at` / `text_input_at` / `text_area_impl` /
    /// `label_at`）都调它——这样"把控件放在容器外"这件事**要么让容器长大、要么
    /// 被 `Clip` 裁掉**，不会出现"看得见点得着却不在容器矩形里"的中间态。
    /// 与鼠标无关（布局期调用），故布局不会随鼠标漂移。
    ///
    /// **公开理由**（控件作者公开面）：控件核心正在往 `widgets/<name>.rs` 搬，而
    /// `widgets` 是 `ui` 的**兄弟模块** ⇒ 私有方法对它们不可见。与其开 `pub(crate)`
    /// 后门，不如把它作为"显式 rect 的控件都该调"的公开原语（自定义控件同理）。
    #[inline]
    pub fn note_placed(&mut self, rect: Rect) {
        if let Some(frame) = self.frames.last_mut() {
            frame.note_content(rect);
        }
    }

    /// 鼠标绝对坐标 → 当前容器局部坐标（逻辑像素，字段运算避免方法借用）。
    ///
    /// **控件作者公开面**：自定义拖拽区（如取色器的 SV 平面 / 色相条）拿它做
    /// "点哪取哪"的绝对映射；[`Self::mouse_screen`] 是屏幕坐标，只有配合容器原点才有意义。
    #[inline]
    pub fn mouse_local(&self) -> Vec2 {
        self.mouse_logical - self.abs_base
    }

    /// 鼠标局部坐标的 x（内部热路径用；语义同 [`Self::mouse_local`]）。
    #[inline]
    pub(super) fn mouse_local_x(&self) -> f32 {
        self.mouse_local().x
    }

    /// 局部矩形（逻辑）→ 命中测试（与逻辑鼠标坐标比较；含窗口外判定、窗口遮挡与
    /// **控件级遮挡**）。
    ///
    /// `owner` = 本控件的**绝对 ID**：用于控件级遮挡的身份判定（同一控件的多个区域
    /// 互不遮挡）。控件作者交互判断用（与 [`Self::mouse_left`] /
    /// [`hit::update_interact`](crate::hit::update_interact) 组合）。
    ///
    /// # 遮挡（"重叠控件被一起触发"修复）
    ///
    /// 同一窗口 / 面板内，**后录制 = 画在上面**的控件优先：鼠标下若有别的控件
    /// 记录得比我晚且覆盖此处，本控件**不响应**（点击 / 悬停 / 拖拽都不响应），
    /// 重叠区域只有最上层那一个控件被触发。判定用**上一帧**登记的区域
    /// （[`crate::hit::widget_occluded`]）——本帧后面的控件还没录制，无法参与判定。
    ///
    /// ⚠ **自定义控件务必传自己的绝对 ID**（不是容器的 id）：传容器 id 会让同容器内
    /// 的所有控件"互不遮挡"，传别人的 id 会让自己永远被那个控件挡住。
    ///
    /// # 其余拦截
    ///
    /// - **窗口遮挡**：鼠标下若有更高 z 的窗口盖住本控件所在窗口 → 不响应，
    ///   累加 [`UiState::occluded_hits`](crate::UiState::occluded_hits)（诊断）；
    /// - **强制裁剪层**：鼠标在 [`Self::clip`]（ScrollView 可视区 / Clip 沙箱）
    ///   之外时不命中——修复"滚出可视区的控件边缘仍可交互"缺口。
    #[inline]
    pub fn hit_abs(&mut self, owner: &IdAbsolute<'_>, local: &Rect) -> bool {
        self.hit_impl(Some(owner), local)
    }

    /// **"本体"命中**（窗口 / 面板 / 浮层的**整块区域**，不是控件）：与 [`Self::hit_abs`]
    /// 的区别是**不参与控件级遮挡**——本体**包含**它内部的子控件，若也走控件级遮挡，
    /// 鼠标停在子控件上时本体就会被自己的子控件判成"被挡住"（浮层会误判"点在面板外"
    /// 而收起）。窗口 / 面板本体的层级由**窗口遮挡**（z-order）负责，与控件级遮挡正交。
    ///
    /// 也不登记为遮挡区域：本体不需要挡住别的控件（那是窗口遮挡的事）。
    #[inline]
    pub fn hit_body_abs(&mut self, local: &Rect) -> bool {
        self.hit_impl(None, local)
    }

    /// [`Self::hit_abs`] / [`Self::hit_body_abs`] 的公共实现
    /// （`owner = None` ⇒ 本体：不做控件级遮挡、也不登记）。
    fn hit_impl(&mut self, owner: Option<&IdAbsolute<'_>>, local: &Rect) -> bool {
        if !self.mouse_in_window {
            return false;
        }
        // 面板/窗口**真正拖拽中**（按下后位移 ≥ DRAG_ACTIVATE_PX）抑制子控件交互
        // （防止拖动中误触按钮等）；纯点击不进入拖拽，子控件正常响应。
        if self.drag_panel.is_some() {
            return false;
        }
        let abs = Rect::new(
            self.abs_base.x + local.x,
            self.abs_base.y + local.y,
            local.w,
            local.h,
        );
        if !hit_test(&abs, self.mouse_logical) {
            return false;
        }
        // 强制裁剪层（Clip 沙箱 / ScrollView 可视区）：层外命中失效。
        if let Some(c) = self.painter.q.clip
            && !c.contains_point(self.mouse_logical) {
                return false;
            }
        // **窗口自身的可命中范围**（内容被裁切时才有）：裁掉的部分不得命中 ——
        // 否则"看不见的控件"还能点到（幽灵控件）。放在**登记命中区之前**：越界的命中
        // 既不响应，也不会把窗口的遮挡矩形撑到窗口外面去。
        if let Some(limit) = self.cur_win_hit_limit
            && !limit.contains_point(self.mouse_logical)
        {
            self.state.occluded_hits += 1;
            return false;
        }
        // **窗口遮挡**（点击穿透修复）：鼠标下若有更高 z 的窗口（`win=0` 内容被任意
        // 窗口）覆盖本控件所在窗口 → 本窗口不得响应——重叠区域只让最上层窗口交互，
        // 背后窗口的控件不会误触发。窗口矩形来自 [`UiState::window_rects`]（跨帧缓存）。
        if window_occluded(self.painter.q.cur_win, self.mouse_logical, self.window_rects_iter()) {
            // 命中但被遮挡 → 记录诊断计数（未响应）。
            self.state.occluded_hits += 1;
            return false;
        }
        let Some(owner) = owner else {
            return true;
        };
        // 当前窗口的**可见交互范围**并集（`window_rects[z]` 用它扩展，见字段文档）。
        // ⚠ 这里只做"遮挡范围"的记录，**不**参与容器尺寸（容器尺寸由
        // `Frame::note_content` 在布局期记，与鼠标无关——否则布局会随鼠标漂移）。
        self.win_hit_bounds = Some(match self.win_hit_bounds {
            Some(b) => b.union(&abs),
            None => abs,
        });
        // **控件级遮挡**：登记自己（供下一帧判定"谁盖住谁"），并检查上一帧里是否有
        // 更上层的别的控件覆盖此处。登记只在"几何命中"之后发生——遮挡判定只关心
        // 鼠标下那一处，鼠标不在自己矩形内时无需登记。
        let me = id_hash(owner);
        let key = self.painter.q.seq;
        let blocked = widget_occluded(
            me,
            key,
            self.mouse_logical,
            self.state.prev_hit_regions.iter().copied(),
        );
        self.state.hit_regions.push(HitRegion {
            owner: me,
            key,
            rect: abs,
            clip: self.painter.q.clip,
        });
        let traced = std::env::var_os("RJ_HIT_TRACE").is_some();
        if blocked {
            self.state.widget_occluded_hits += 1;
            if traced {
                eprintln!(
                    "hit[frame {}] {} BLOCKED-by-widget rect=({},{},{},{}) mouse=({},{})",
                    self.state.frame, owner.as_str(), abs.x, abs.y, abs.w, abs.h,
                    self.mouse_logical.x, self.mouse_logical.y
                );
            }
            return false;
        }
        if traced {
            eprintln!(
                "hit[frame {}] {} OK rect=({},{},{},{}) mouse=({},{})",
                self.state.frame, owner.as_str(), abs.x, abs.y, abs.w, abs.h,
                self.mouse_logical.x, self.mouse_logical.y
            );
        }
        // **记下"本帧由谁认领了按下"**（帧末复核用，见 `resolve_widget_press`）：
        // 连同**所在窗口的绝对 ID**一起记（复核时要解它的**当前** z——不能在复核时拿认领
        // 时的旧 z 比，那样窗口会把自己判成"被别人盖住"，把窗口内所有拖拽都撤掉）。
        // 只记第一个认领者——正常情况下（控件级遮挡生效）也只有一个。
        if self.mouse_left().down_edge() && self.state.frame_state.press_widget.is_none() {
            self.state.frame_state.press_widget =
                Some((owner.to_static(), self.cur_win_id.clone()));
        }
        true
    }

}

impl<'a> Ui<'a> {
    /// 当前容器为子项分配局部矩形（`finish` 前任何位置可用：顶层由**根容器**
    /// （[`Ui::begin`] 内建，可用宽 = 视口物理宽）流式堆叠，容器 / 窗口内
    /// 用各自的帧）。控件作者做"占光标"式自定义容器时用（相对当前容器内容原点）。
    ///
    /// `child`：[`Child::Expand`]（默认，撑大父级）/ [`Child::Fit`]（不撑大父级，
    /// 对应 [`crate::widgets::Expansion::DisableAutoExpansion`]）/ [`Child::Fill`]
    /// （**宽度铺满、但宽度不计入父级**，高度照常 —— 整格装饰用，见 `Child` 文档）。
    pub fn child_rect(&mut self, w: f32, h: f32, child: Child) -> Rect {
        self.frames
            .last_mut()
            .expect("root frame")
            .child_rect_exp(w, h, child)
    }

    /// **放置控件**（[`crate::widgets::Widget`] trait）：容器内**占光标**；尺寸由控件
    /// 自己在 `ui()` 里就地申请（[`Self::allocate`] 一族），因此这里只是把 `Ui` 交给它。
    ///
    /// 属性化 builder 示例：`ui.add(Button::new("ok", "确定").color(Color::WHITE))`。
    /// 容器包装（`Panel` / `Pack` / `Grid` / `Window` / `Scroll` / `FlexCtx`）经
    /// [`UiAdd`] 提供同样的 `add` / `add_at` 与全部便捷方法（`p.button` / `p.label` 等）。
    pub fn add(&mut self, w: impl crate::widgets::Widget) -> crate::widgets::Response {
        // 尺寸类（`Widget::size_class`）：在 `ui()` 之前交给当前 frame —— 水平行据此
        // 决定"单行钉标准高 / 多行可撑高"（见 `crate::widgets::SizeClass`）。
        if let Some(f) = self.frames.last_mut() {
            f.set_next_class(w.size_class());
        }
        let resp = w.ui(self);
        // 兜底清掉：控件可能一次 `child_rect` 都没走（`add_at` 的绝对定位 / 自绘控件），
        // 留着会让**下一个**子项拿到错的尺寸类。
        if let Some(f) = self.frames.last_mut() {
            f.set_next_class(crate::widgets::SizeClass::SingleLine);
        }
        resp
    }

    /// **绝对定位放置控件**（`pos` 相对当前容器内容原点；**不占光标**）。
    ///
    /// 实现 = 给 `Ui` 打一个**一次性放置覆盖**，控件的第一次申请（[`Self::allocate`] /
    /// [`Self::allocate_sense`] / …）消费它 ⇒ 控件自身的 `ui()` 不必关心"我是被 `add`
    /// 还是 `add_at` 放的"。
    ///
    /// ⚠ 只作用于**第一次**申请：控件要摆多个矩形时用 [`Self::allocate_at`]。
    pub fn add_at(
        &mut self,
        pos: impl Into<Position>,
        w: impl crate::widgets::Widget,
    ) -> crate::widgets::Response {
        self.place_once = Some(pos.into().to_physical(self.scale));
        if let Some(f) = self.frames.last_mut() {
            f.set_next_class(w.size_class());
        }
        let resp = w.ui(self);
        // 兜底清掉（控件可能一次申请都没做）。
        self.place_once = None;
        if let Some(f) = self.frames.last_mut() {
            f.set_next_class(crate::widgets::SizeClass::SingleLine);
        }
        resp
    }

    // ── 申请布局（控件作者用：参考 egui 的 allocate_*） ──────────
    //
    // ⚠ **尺寸一律是物理像素 `Vec2`**（与"`Theme` 已预乘、内部全物理像素"的约定一致）；
    // 要写逻辑单位就在自己的 API 边界换算（`Size::Logical(x).to_physical(ui.scale())`）。
    // 这里**刻意不收** `impl Into<Size<Vec2>>`：那会把已经是物理像素的测量结果
    // （`text_size` / 主题常量）再乘一次 DPI —— 实测 bug：scale = 1.5 时所有控件
    // 长到 1.5 倍，固定宽窗口的尺寸也跟着变。

    /// **占光标申请矩形**（撑大父级；**物理像素**）。
    ///
    /// 控件 `ui()` 的第一步。⚠ **先量后申请**：`avail_w()` / `text_size()` 要在本调用
    /// **之前**取值——申请会推进容器光标，之后的 `avail_w()` 是"下一项"的约束。
    #[inline]
    pub fn allocate(&mut self, size: Vec2) -> Rect {
        self.allocate_mode(size, crate::widgets::Expansion::UnlimitedExpansion)
    }

    /// **占光标申请矩形**，并指定**膨胀模式**（取代旧 `Widget::expansion()`）：
    /// - [`Expansion::UnlimitedExpansion`]：撑大父级（默认）；
    /// - [`Expansion::DisableAutoExpansion`]：不撑大父级（装饰件 / 分隔线）；
    /// - [`Expansion::LimitedInParent`]：宽度压到父级可用宽（`avail_w`）。
    pub fn allocate_mode(&mut self, size: Vec2, mode: crate::widgets::Expansion) -> Rect {
        self.allocate_rect(size, mode)
    }

    /// **绝对定位申请矩形**（不占光标；`pos` 相对当前容器内容原点，`size` 物理像素）。
    ///
    /// 与 [`Self::add_at`] 的区别：这是控件**自己在 `ui()` 里**决定位置（比如一个控件
    /// 要摆多块），`add_at` 是调用方决定控件的落点。
    pub fn allocate_at(&mut self, pos: impl Into<Position>, size: Vec2) -> Rect {
        let pos = pos.into().to_physical(self.scale);
        let rect = Rect::new(pos.x, pos.y, size.x, size.y);
        self.note_placed(rect);
        rect
    }

    /// **申请 + 一次性收交互**（egui `allocate_exact_size` 的对应物；`size` 物理像素）：
    /// 命中测试 / 焦点链 / 按下认领（`sense.drag`）/ 跨帧状态一次做完。
    #[inline]
    pub fn allocate_sense(
        &mut self,
        id: &str,
        size: Vec2,
        sense: crate::widgets::Sense,
    ) -> (Rect, crate::widgets::Response) {
        self.allocate_sense_mode(id, size, crate::widgets::Expansion::UnlimitedExpansion, sense)
    }

    /// [`Self::allocate_sense`] + 膨胀模式（见 [`Self::allocate_mode`]）。
    pub fn allocate_sense_mode(
        &mut self,
        id: &str,
        size: Vec2,
        mode: crate::widgets::Expansion,
        sense: crate::widgets::Sense,
    ) -> (Rect, crate::widgets::Response) {
        let rect = self.allocate_mode(size, mode);
        let abs = self.id_for(id);
        let resp = self.interact(&abs, rect, sense);
        (rect, resp)
    }

    /// **绝对定位申请 + 收交互**（不占光标）。
    pub fn allocate_sense_at(
        &mut self,
        pos: impl Into<Position>,
        id: &str,
        size: Vec2,
        sense: crate::widgets::Sense,
    ) -> (Rect, crate::widgets::Response) {
        let rect = self.allocate_at(pos, size);
        let abs = self.id_for(id);
        let resp = self.interact(&abs, rect, sense);
        (rect, resp)
    }

    /// 申请的核心（已换算物理像素）：按模式调整 → 消费一次性放置覆盖 → 占光标。
    fn allocate_rect(&mut self, mut size: Vec2, mode: crate::widgets::Expansion) -> Rect {
        use crate::widgets::Expansion;
        // `LimitedInParent`：宽度压到父级可用宽（`avail_w`）；无可用宽 = 自然尺寸。
        if mode == Expansion::LimitedInParent
            && let Some(avail) = self.avail_w()
            && avail < size.x
        {
            size.x = avail;
        }
        // `add_at` 的一次性覆盖：不占光标，直接落在指定坐标。
        if let Some(pos) = self.place_once.take() {
            let rect = Rect::new(pos.x, pos.y, size.x, size.y);
            self.note_placed(rect);
            return rect;
        }
        let child = if mode == Expansion::DisableAutoExpansion {
            Child::Fit
        } else {
            Child::Expand
        };
        self.child_rect(size.x, size.y, child)
    }

    /// **本矩形是否被裁剪层完全剔除**（分配 → 判一次 → 直接 return 的判据）。
    ///
    /// 语义 = "在当前**强制裁剪层**（Clip 沙箱 / ScrollView 可视区 / 文本框盒；不含
    /// 窗口结算后才知道的严格裁剪）里，这个矩形**一个像素都看不见**"。
    /// `rect` 传**当前容器局部**矩形（就是 `allocate*` 给的那个）。
    ///
    /// ⚠ 与 batch scissor 分工：scissor 裁**像素**（部分重叠时保住圆角/投影原形），
    /// 本判据裁**整条命令**（省镶嵌 + 顶点 + 段）。两者互补，都要有。
    #[inline]
    pub fn culled(&self, rect: Rect) -> bool {
        // 命令是先按容器局部录制、弹出时统一平移成绝对坐标；裁剪层是**绝对**坐标 ⇒
        // 比较前先把矩形抬到绝对空间（`abs_base` 就是当前容器的绝对原点）。
        let abs = Rect::new(
            rect.x + self.abs_base.x,
            rect.y + self.abs_base.y,
            rect.w,
            rect.h,
        );
        fully_outside(abs, self.painter.q.clip)
    }

    /// **收交互**（控件作者用；[`Self::allocate_sense`] 的底层）：按 [`crate::widgets::Sense`]
    /// 把"命中 → 焦点 → 按下认领 → 跨帧状态机"一次做完，返回本帧 [`crate::widgets::Response`]。
    ///
    /// 与既有内置控件逐条对齐（`button_at_styled` / `checkbox_at_styled` 等就是这套组合）：
    /// 1. `hit = hit_abs(id, rect)`（含窗口遮挡 / 控件级遮挡 / 强制裁剪层）；
    /// 2. `sense.focus` ⇒ `register_focus(id, rect, kind)`（Tab 可到 + 焦点描边）；
    /// 3. `sense.focus` ⇒ `key_click(id, kind)`（Enter/Space 合成点击）；
    /// 4. `sense.drag && 按下边沿 && hit` ⇒ `claim_press()`（外层窗口/面板不再把这次按下
    ///    当作拖动基准）；
    /// 5. `update_interact`（hover/pressed/clicked/released 跨帧状态）+ 键盘点击合成；
    /// 6. `pressed` ⇒ `note_press_handled()`（帧末"点空白清焦点"要区分按下与空白）；
    /// 7. `sense.drag` ⇒ `update_drag`（拖拽态；基准用 `WidgetState::{press_mouse,press_panel}`）。
    ///
    /// ⚠ 有拖拽语义的控件**仍要自己**维护拖拽基准（按下时写 `press_mouse` / `press_panel`）
    /// 与数值映射——本方法只负责状态机与认领。
    pub fn interact(
        &mut self,
        id: &IdAbsolute<'_>,
        rect: Rect,
        sense: crate::widgets::Sense,
    ) -> crate::widgets::Response {
        use crate::widgets::Response;
        let hit = sense.needs_hit() && self.hit_abs(id, &rect);
        if let Some(kind) = sense.focus {
            self.register_focus(id, rect, kind);
        }
        let btn = self.mouse_left();
        let key_click = sense.focus.is_some_and(|kind| self.key_click(id, kind));
        // 拖拽语义：按下边沿 + 命中 ⇒ 认领本次按压（阻止窗口/面板的拖动基准）。
        if sense.drag && hit && btn.down_edge() {
            self.claim_press();
        }
        let mut ev = {
            let abs_rect = Rect::new(
                self.abs_base.x + rect.x,
                self.abs_base.y + rect.y,
                rect.w,
                rect.h,
            );
            let ws = self.state_mut().widget(id);
            let ev = update_interact(ws, hit, btn);
            if key_click {
                // 键盘激活：合成"按下"，与鼠标路径同形（见各内置控件的 key_click 处理）。
                ws.pressed = true;
            }
            // **记下这次交互的矩形**（**绝对逻辑屏幕坐标**）——诊断 / `--sim-*` 按名字取坐标用，
            // 免得把手算的位置写死（主题 / 布局一改就废）。⚠ 必须是**绝对**：`rect` 是当前
            // 容器的局部坐标，直接拿去注入鼠标会落在窗口内容原点之外（实测偏一整个标题栏 + 内边距）。
            ws.last_interact_rect = Some(abs_rect);
            ev
        };
        if key_click {
            ev.clicked = true;
        }
        if ev.pressed || key_click {
            self.note_press_handled();
        }
        if sense.drag {
            let ws = self.state_mut().widget(id);
            update_drag(ws, hit, btn);
        }
        // `pressed` = **持续按住**（与 `ButtonState::pressed` 同义），不是"本帧按下边沿"：
        // 三态配色（常态/悬停/按下）靠它，读边沿会让按住期间退回悬停色。
        let held = self.state().widgets.get(id.as_str()).is_some_and(|ws| ws.pressed);
        Response {
            rect,
            // 分配处剔除：完全看不见的控件直接把信号交给控件（它应立刻 return）。
            culled: self.culled(rect),
            hovered: hit,
            pressed: held,
            clicked: ev.clicked,
            released: ev.released,
            toggled: false,
        }
    }

    /// 通用容器：push 帧 → 闭包 → 结算（返回尺寸与最大子尺寸）→ 平移子命令 → pop。
    pub(super) fn container<F>(&mut self, pos: Vec2, frame: Frame, f: F) -> (Vec2, Vec2)
    where
        F: FnOnce(&mut ContainerCtx<'_, '_>),
    {
        let start = self.painter.q.queue.len();
        self.begin_top_placement();
        let saved_base = self.abs_base;
        self.abs_base = saved_base + pos;
        // **可用宽下传**（嵌套容器不许把内容排到固定宽父级外面）：
        // 子容器自己没有固定宽时，继承父级的可用宽（扣除子容器自己的内边距）。
        // 见 `Frame::max_w` 与 [`Self::avail_w`]。
        let mut frame = frame;
        if !frame.has_fixed_w() {
            let parent_avail = self.avail_w();
            frame.set_max_w(content_max_w(parent_avail, frame.pad_total));
        }
        self.frames.push(frame);
        self.painter.q.depth += 1;
        f(&mut ContainerCtx { ui: self });
        let frame = self.frames.pop().expect("container frame");
        let size = frame.settle_size();
        let max_child = frame.max_child;
        let inner_bounds = frame.content_bounds();
        // **分割线满宽**：本容器内容宽 = 结算宽 − 2×内边距（此刻才知道 ⇒ 回填线段宽度）。
        let content_w = (size.x - frame.pad_total() * 2.0).max(0.0);
        self.painter.q.depth -= 1;
        self.expand_pending_full_w(start, content_w);
        self.abs_base = saved_base;
        for d in &mut self.painter.q.queue[start..] {
            d.translate(pos);
        }
        // 绝对放置的容器整体也要算进**父级**尺寸（否则父容器/窗口仍会"只有标题那么高"）；
        // 连同容器**内部**的内容包围盒一起平移上报（自然尺寸可能低估子控件范围）。
        if let Some(parent) = self.frames.last_mut() {
            parent.note_content(Rect::new(pos.x, pos.y, size.x, size.y));
            if let Some(ib) = inner_bounds {
                parent.note_content(Rect::new(ib.x + pos.x, ib.y + pos.y, ib.w, ib.h));
            }
        }
        (size, max_child)
    }

    /// **长宽已确定的装饰容器**（内部原语）：在 `rect`（**当前容器局部**坐标、尺寸固定）里跑一个
    /// `Pack`，命令留在当前容器的局部空间（随后由调用方统一平移），**不参与父级尺寸结算**。
    ///
    /// 与 [`Self::container`] 的区别只有一条，但很关键：**不 `note_content` 到父 frame**
    /// —— 装饰（标题栏 / 状态条）是在**尺寸已经结算之后**才跑的，它的内容**不得**反过来
    /// 影响任何容器尺寸（否则"结算后再画"就自相矛盾）。
    ///
    /// 用途：窗口标题栏（[`Self::window_impl`] 在 `settle_size` 之后、**按下裁决之前**
    /// 用最终外框宽跑它 ⇒ 标题可居中、按钮可贴外缘）。用户侧想要同一形状可以用公开的
    /// [`Self::view_at`]（`ViewMode::Expand` 的沙箱：提供 `avail_w`、不裁剪、内容可溢出）。
    pub(super) fn ornament_at(&mut self, rect: Rect, f: impl FnOnce(&mut Pack<'_, '_>)) {
        self.ornament_impl(rect, true, |ui| {
            let mut p = Pack { ui };
            f(&mut p);
        });
    }

    /// **装饰容器（自然高）**：同 [`Self::ornament_at`] 的"不 `note_content`、命令平移到
    /// `rect` 原点"语义，但**只固定宽**、高度交给内容，并返回内容**结算高**。
    ///
    /// 用户是 `Foldable` 的自定义标题：闭包拿到 [`PackEntry`]（与正文同一个上下文类型），
    /// 里面可以放 Row / 多行内容 ⇒ 标题块自己撑开（"标题可以视作一个标准容器"），
    /// 调用方拿返回高度把标题行矩形加高（见 `Foldable::show`）。
    pub(super) fn ornament_entry_natural_h(
        &mut self,
        rect: Rect,
        f: impl FnOnce(&mut PackEntry<'_, '_>),
    ) -> f32 {
        self.ornament_impl(rect, false, |ui| {
            let mut e = PackEntry::new(ui);
            f(&mut e);
        })
    }

    /// 装饰容器的公共实现（`fixed_h = false` ⇒ 高度自然，返回值才有意义）：
    /// 布局 / 深度 / 平移语义必须逐字一致。
    fn ornament_impl(
        &mut self,
        rect: Rect,
        fixed_h: bool,
        f: impl FnOnce(&mut Ui<'a>),
    ) -> f32 {
        let start = self.painter.q.queue.len();
        // 本帧新增的命中区起点（居中时要一起平移，见下方 `dy`）。
        let hit_start = self.state.hit_regions.len();
        let saved_base = self.abs_base;
        self.abs_base = saved_base + Vec2::new(rect.x, rect.y);
        let mut frame = Frame::new_stack(PackSide::Top, self.theme.gap, 0.0);
        // 固定宽：子项因此拿到确定的 `avail_w`（省略号 / `LimitedInParent` 用得上）。
        frame.set_fixed_w(rect.w);
        if fixed_h {
            frame.set_fixed_h(rect.h);
        }
        self.frames.push(frame);
        // 深度与"同一窗口里的内容"持平 ⇒ `elem` 语义与旧实现（标题/按钮录在窗口 frame 里）
        // 完全一致：晚入队 ⇒ 排在面板底色（`elem = 0`，在窗口外一层画）之上、内容同级之上。
        self.painter.q.depth += 1;
        f(&mut *self);
        // 结算尺寸只由调用方关心（装饰**不** `note_content` 到父级）。
        let frame = self.frames.pop().expect("ornament frame");
        let content_h = frame.settle_size().y;
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        // **垂直居中**（只对"高度自然"的那种，即 `fixed_h == false`）：内容比 `rect` 矮时，
        // 把整段命令与命中区一起下移 `(rect.h - content_h) / 2`。
        //
        // 为什么必须居中：`Foldable` 的**文本标题**走 `push_text_rect(area, .., VAlign::Center)`
        // ⇒ 文字**居中于标题行**；而**自定义标题**的闭包跑在这里、里面的 `Label` 是普通控件
        // ⇒ 从容器**顶**开始排。不补这一步，`ui.foldable(id, "标题")` 与
        // `ui.foldable_custom(id, |t| { t.label("标题"); })` 会长得**不一样**（实测差
        // `(row_h 39 − 行高 21) / 2 = 9` 物理像素）——而用户合理地期望二者等效。
        //
        // ⚠ 事后平移的代价（与 `Foldable` 正文缩进同一类）：**本帧**的命中判定已在录制时
        // 用未平移的矩形算过 ⇒ 本帧的点击会偏 `dy`（下一帧起一致）。所以这里同步平移
        // `hit_regions`（下一帧的控件级遮挡正确）；`WidgetState::last_interact_rect`（诊断 /
        // 脚本化注入用）仍记录未含 `dy` 的矩形。
        let dy = if fixed_h {
            0.0
        } else {
            center_offset(rect.h, content_h)
        };
        if rect.x != 0.0 || rect.y != 0.0 || dy != 0.0 {
            for d in &mut self.painter.q.queue[start..] {
                d.translate(Vec2::new(rect.x, rect.y + dy));
            }
        }
        if dy != 0.0 {
            for r in &mut self.state.hit_regions[hit_start..] {
                r.rect.y += dy;
            }
        }
        content_h
    }

    /// **View 沙箱**（闭包作用域，见 [`crate::view`]）：进入沙箱后——
    ///
    /// - [`ViewMode::Clip`]：内容超出沙箱**强制裁剪**（外层裁剪 ∩ 沙箱可视区），
    ///   沙箱外的鼠标**命中失效**（`hit_abs` 带沙箱判定）；
    /// - [`ViewMode::Expand`]：不裁剪，内容自然尺寸可溢出沙箱并撑大外层容器；
    ///   沙箱提供"可用宽度"（[`Self::avail_w`]），供 `LimitedInParent` 控件自洽
    ///   （自动换行 / "…"省略）。
    ///
    /// 沙箱内录制的命令随弹出统一平移 `pos`（相对当前容器内容原点，不占父光标）。
    /// 返回内容结算尺寸（`Expand` 下可大于 `size`）。**ScrollView**（[`Self::scroll_at`]、
    /// 文本编辑框）与严格窗口（[`Self::window`] + `Placement::Clip`）的公共底座。
    pub fn view_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        mode: ViewMode,
        f: impl FnOnce(&mut ViewCtx<'_, '_>),
    ) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        let size = size.into().to_physical(self.scale);
        let saved_clip = self.painter.q.clip;
        let saved_base = self.abs_base;
        let view_rel = Rect::new(pos.x, pos.y, size.x.max(0.0), size.y.max(0.0));
        let view_abs = Rect::new(
            saved_base.x + view_rel.x,
            saved_base.y + view_rel.y,
            view_rel.w,
            view_rel.h,
        );
        // 强制裁剪层（Clip 模式：外层 ∩ 可视区；Expand：原样传递）。
        self.painter.q.clip = clip_for_view(saved_clip, view_abs, mode);
        // 可用宽度栈：沙箱内 avail_w() = 沙箱宽。
        self.avail_stack.push(Some(view_rel.w));
        let start = self.painter.q.queue.len();
        self.begin_top_placement();
        self.abs_base = saved_base + pos;
        self.frames.push(Frame::new_stack(PackSide::Top, self.theme.gap, 0.0));
        self.painter.q.depth += 1;
        f(&mut ViewCtx { ui: self });
        let frame = self.frames.pop().expect("view frame");
        let content = frame.settle_size();
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        self.avail_stack.pop();
        self.painter.q.clip = saved_clip;
        for d in &mut self.painter.q.queue[start..] {
            d.translate(pos);
        }
        content
    }

    /// 当前可用的**内容宽度**（物理像素）：沙箱宽 → **由内向外**第一个给出宽度约束的
    /// 容器 frame（固定宽窗口 / 从父级继承的内容最大宽）→ 下一子项 max 约束，取最小；
    /// 无任何约束 = `None`（内容自然宽度）。供 `LimitedInParent` 控件（如
    /// [`crate::widgets::Label`]）自洽（自动换行 / 省略号）。
    ///
    /// ⚠ **必须由内向外扫整个 frame 栈**（不是只看 `frames.last()`）：`row` 是独立
    /// frame、自己不设固定宽，只看最内层会让 `row` 里的 `Label` 拿不到外层窗口的可用宽
    /// ⇒ 按自然宽把整行排到**固定宽窗口外面**（用户实测："指定 width 里控件会突出去，
    /// 直到你去拖拽缩放"）。
    #[inline]
    pub fn avail_w(&self) -> Option<f32> {
        let base = self
            .avail_stack
            .last()
            .copied()
            .flatten()
            .or_else(|| crate::layout::stack_avail_w(&self.frames));
        // **下一子项**的两条约束：显式 `max_size`，以及水平堆叠的**剩余宽**
        // （一行里后面的控件拿到的可用宽必须扣掉前面已经用掉的）。
        let nm = self.frames.last().map(|f| f.next_max_w()).unwrap_or(0.0);
        let rem = self.frames.last().and_then(|f| f.remaining_w());
        let mut out = base;
        if nm > 0.0 {
            out = Some(out.map_or(nm, |b| b.min(nm)));
        }
        if let Some(r) = rem {
            out = Some(out.map_or(r, |b| b.min(r)));
        }
        out
    }

    /// **分割线**（绝对定位水平线）：`pos` 相对当前容器内容原点，宽 `w`（逻辑像素）。
    /// 线画在 `pos.y + margin`（上下留白由调用方行高体现）。样式取
    /// [`Theme::divider`](crate::style::Theme::divider)。
    ///
    /// ⚠ **容器内的分割线请用 [`Self::divider_full_w`]**（`ui.divider()` / 水平 `Divider`
    /// 都走它）：本方法画的是"调用方给定宽度"的线，而容器内的分割线要的是**内容宽** ——
    /// 那要等容器结算尺寸才知道（自动宽窗口是布局根、`avail_w()` 恒 `None`）。
    pub fn divider_at(&mut self, pos: impl Into<Position>, w: impl Into<Size<f32>>) {
        let pos = pos.into().to_physical(self.scale);
        let w = w.into().to_physical(self.scale);
        let st = self.theme.divider.clone();
        if w > 0.0 && st.thickness > 0.0 {
            self.push_solid_rect(Rect::new(pos.x, pos.y + st.margin, w, st.thickness), st.color);
        }
    }

    /// **分割线（容器内 → 满宽）**：与 [`Self::divider_at`] 同形，但把这条命令标成
    /// **待定满宽**（[`UiDraw::full_w`]）—— 它所属容器结算尺寸后由
    /// [`Self::expand_pending_full_w`] 回填为"从自己的 x 一直到**内容盒右缘**"。
    ///
    /// `w` 是录制期的**保守宽度**（`avail_w()` / 最宽子项 / 120 兜底）：它既决定本帧占位，
    /// 也是**下限** —— 回填只增不减，容器比线还窄时不会把线拉没。
    ///
    /// ⚠ **深度 0（`win=0` 顶层）不标记**：那里没有"容器内容宽"这回事，拿根 frame 的固定宽
    /// （= 视口宽）会把线拉成整屏 —— 历史上正是这条把自动宽窗口撑成屏幕宽
    /// （见 `layout::stack_avail_w` 的文档）。
    pub fn divider_full_w(&mut self, pos: impl Into<Position>, w: impl Into<Size<f32>>) {
        let pos = pos.into().to_physical(self.scale);
        let w = w.into().to_physical(self.scale);
        let st = self.theme.divider.clone();
        if w > 0.0 && st.thickness > 0.0 {
            self.painter
                .solid(Rect::new(pos.x, pos.y + st.margin, w, st.thickness), st.color);
            // `Painter::solid` 只录这一条 ⇒ 刚录的就是它（按 `seq` 命中，不用猜下标）。
            self.mark_last_full_w();
        }
    }

    /// **回填"待定满宽"命令**（[`UiDraw::full_w`]）：`content_w` = 本容器**内容盒宽**
    /// （`settle_size().x - 2 × pad_total`）。
    ///
    /// 对 `[start, end)` 内每条已标记的 `Solid`：
    /// - `rect.w = max(rect.w, 内容盒右缘 − rect.x)`（**只增不减**：容器比线窄时保持原样）；
    /// - 无论是否回填都**清零标记** —— 于是走进 `finish` / 顶点缓存的命令里 `full_w` 恒
    ///   `false`（不进内容签名、不会造成缓存失效），外层容器也不会重复拉伸内层已定宽的线。
    ///
    /// `content_w <= 0`（退化容器 / 没量到）时**只清零**。
    pub(super) fn expand_pending_full_w(&mut self, start: usize, content_w: f32) {
        let traced = std::env::var_os("RJ_DIVIDER_TRACE").is_some();
        for d in &mut self.painter.q.queue[start..] {
            if !d.full_w {
                continue;
            }
            d.full_w = false;
            if content_w > 0.0 && matches!(d.kind, crate::draw::DrawKind::Solid(_)) {
                d.rect.w = full_w_target(d.rect.x, d.rect.w, content_w);
                if traced {
                    eprintln!(
                        "divider[full_w]: x={:.1} w={:.1} 内容宽={content_w:.1}",
                        d.rect.x, d.rect.w
                    );
                }
            }
        }
    }

}

