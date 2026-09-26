//! 滚动容器（ScrollView）与滚动条：`scroll_at` / `scroll_axes_at` / `list_at` /
//! `scrollbar(_axis)`，以及全部滚动几何纯函数（含单测用到的 `scrollbar_rects` 等）。
//!
//! 维护者笔记：滚动偏移一律**物理像素**并持久于 `UiState::scrolls`；对外参数才经
//! `Size` / `Position` 换算（单位纪律见 `docs/UI_ARCHITECTURE.md` §5.0）。

use super::window::clip_for_axes;
use super::*;

use glam::Vec2;
use rjw_transform::Rect;

use crate::draw::{CornerRadius, DrawKind, Gradient, Position, Size, UiDraw};
use crate::hit::{hit_test, id_hash, update_drag, widget_occluded, window_occluded, HitRegion};
use crate::id::IdAbsolute;
use crate::layout::{Frame, PackSide};

impl<'a> Ui<'a> {
    /// **滚动容器（ScrollView）**：内容在 `view_size` 可视区内垂直堆叠（pack Top），
    /// 超出部分滚动查看——**滚轮**滚动 + 右侧**滚动条**（拖 thumb / 点轨道翻页）。
    ///
    /// - `id`：滚动偏移状态键（[`UiState::scrolls`]，跨帧持久）；
    /// - 内容子项照常录制（`s.label` / `s.button` 等，占光标堆叠）；
    /// - 可视区之外的图形/文字**强制裁剪**（Clip 沙箱：`UiDraw.clip` 绝对逻辑矩形，
    ///   收集期求交，**含 noclip 绘制**）；
    /// - 沙箱内 `avail_w()` = 可视区宽（`LimitedInParent` 控件自洽）；
    /// - 返回 `view_size`（内容尺寸超出时可经 [`UiState::scrolls`] 读取）。
    ///
    /// **默认两条轴** = 纵向[`ScrollMode::Scroll`]（滚动条）/ 横向[`ScrollMode::ClipOnly`]
    /// （只裁不滚）—— 与加横向滚动之前的观感**逐像素一致**（今天横向就没有滚动条，内容
    /// 又被压到视口宽）：既有列表不会突然长出横向条、长 `Label` 照旧按视口宽折行。
    /// 想要**真横向滚动**（内容保持自然宽、超出横向滚动）请用 [`Self::scroll_axes_at`]
    /// 或 [`crate::ScrollArea`]（`hscroll(true)` ⇒ 该轴不折行）。
    ///
    /// **内部计算一律物理像素**（DPI 只在该换算处出现一次）：滚动偏移
    /// [`ScrollState::offset`] / [`ScrollState::offset_x`] 为**物理像素**整数步进
    /// （滚轮 / 拖 thumb 均取整），内容按偏移逻辑平移后 ×scale 回到整物理像素——
    /// **整体刚性移动**，非整数 DPI（125%/150%）下相邻元素取整相位不抖。
    pub fn scroll_at(
        &mut self,
        pos: impl Into<Position>,
        view_size: impl Into<Size<Vec2>>,
        id: &str,
        f: impl FnOnce(&mut Scroll<'_, '_>),
    ) -> Vec2 {
        // 纵向滚动 + 横向裁切（视觉与加横向滚动之前一致；真两轴见 `scroll_axes_at`）。
        self.scroll_axes_at(
            pos,
            view_size,
            id,
            ScrollMode::Scroll,
            ScrollMode::ClipOnly,
            f,
        )
        .view
    }

    /// **按轴的滚动容器**（[`Self::scroll_at`] 的完整版）：`v` / `h` 各自指定该轴的
    /// 溢出策略 —— 传 `bool` 或 [`ScrollMode`]（见 [`ScrollParam`]）。
    ///
    /// - `NoClip`：那条轴**不裁**（内容溢出可见；也不吃滚轮 / 不画滚动条）；
    /// - `ClipOnly`：那条轴是视口（裁切，无滚动条、不吃滚轮）；
    /// - `Scroll`：视口 + **滚动条**（滚轮 / 拖 thumb / 点轨道都能滚）。
    ///
    /// ⚠ **`h == Scroll` 时该轴不折行**（沙箱不再上报可用宽）：横向滚动的前提是内容
    /// 保持**自然宽**；要"按窗口宽折行"就用 `hscroll(false)`（`NoClip`）。
    ///
    /// 返回 [`ScrollOutcome`]（视口 + 内容结算尺寸）；滚动偏移跨帧持久于
    /// [`UiState::scrolls`]。
    pub fn scroll_axes_at(
        &mut self,
        pos: impl Into<Position>,
        view_size: impl Into<Size<Vec2>>,
        id: &str,
        v: impl ScrollParam,
        h: impl ScrollParam,
        f: impl FnOnce(&mut Scroll<'_, '_>),
    ) -> ScrollOutcome {
        let (v, h) = (v.scroll_mode(), h.scroll_mode());
        self.scroll_at_axes(pos, view_size, id, v, h, |ui| f(&mut Scroll { ui }))
    }

    /// [`Self::scroll_axes_at`] 的**核心**（已归一成 [`ScrollMode`] 的按轴版本）：
    /// 把 `f` 录进一个滚动视口，返回视口 + 内容结算尺寸。
    ///
    /// 抽出来是为了让**窗口**（`.vscroll(..)` / `.hscroll(..)`）与公开的 `scroll_at` /
    /// `scroll_axes_at` 共用同一实现：窗口把用户闭包包进来即可（`f` 收 `&mut Ui`，
    /// 两边各自包成 `Scroll` / `Window`）。
    pub(crate) fn scroll_at_axes(
        &mut self,
        pos: impl Into<Position>,
        view_size: impl Into<Size<Vec2>>,
        id: &str,
        v: ScrollMode,
        h: ScrollMode,
        f: impl FnOnce(&mut Ui),
    ) -> ScrollOutcome {
        let pos = pos.into().to_physical(self.scale);
        let view_size = view_size.into().to_physical(self.scale);
        // 滚动容器自身也是命名空间边界：内部子控件 ID 自动带 `id` 前缀。
        let abs = self.id_for(id);
        let saved_clip = self.painter.q.clip;
        let saved_base = self.abs_base;
        let (sw, sh) = (
            self.window.inner_size().width as f32,
            self.window.inner_size().height as f32,
        );
        // 可视区（**相对**当前容器 origin：内容 / 滚动条命令都录在容器局部坐标，
        // 随外层容器弹出统一平移成绝对坐标）。
        let view_rel = Rect::new(pos.x, pos.y, view_size.x.max(0.0), view_size.y.max(0.0));
        // 可视区（**绝对**逻辑屏幕坐标：裁剪 / 滚轮命中用）。
        let view_abs = Rect::new(
            saved_base.x + pos.x,
            saved_base.y + pos.y,
            view_size.x.max(0.0),
            view_size.y.max(0.0),
        );
        // 强制裁剪层 = 外层裁剪 ∩ 本可视区 —— **按轴**（`NoClip` 那条轴不动）。
        self.painter.q.clip =
            clip_for_axes(saved_clip, view_abs, Rect::new(0.0, 0.0, sw, sh), v, h);
        // 滚动偏移（**物理像素**，跨帧状态；先 Copy 读出，`f` 结束再写回——避免
        // 借用冲突）。以整物理像素步进（滚轮 / 拖 thumb 均取整）。
        let (mut offset_px, mut offset_x_px) = self
            .state
            .scrolls
            .get(abs.as_str())
            .map(|s| (s.offset, s.offset_x))
            .unwrap_or((0.0, 0.0));
        // 可用宽度栈：**只有纵向是滚动轴时**才上报可视区宽 —— `h == Scroll` 表示
        // "内容保持自然宽、超出横向滚"，此时一旦上报可用宽，`LimitedInParent` 控件
        // （`Label` 折行 / 省略）就会把自己压进视口 ⇒ **永远没有横向溢出**、
        // 横向滚动条永远不出现（见 `Frame::set_clip_w` 的同一条规则）。
        self.avail_stack.push(scroll_axes_avail_w(h, view_rel.w));
        // 内容 pack 堆叠（手动管理帧栈：平移 = pos - offset/scale，而非 container 的 pos）。
        let start = self.painter.q.queue.len();
        self.begin_top_placement();
        // abs_base = 内容**渲染**原点（已含 -offset 滚动偏移）——`hit_abs`（点击
        // 命中）/ `register_focus`（焦点描边）/ IME 光标定位都经 abs_base 换算，
        // 必须与平移后的绘制位置一致，否则点击位置跟不上滚动视图（offset ≠ 0 时
        // 命中落在未滚动坐标上）。
        self.abs_base = saved_base + pos - Vec2::new(offset_x_px, offset_px);
        self.frames
            .push(Frame::new_stack(PackSide::Top, self.theme.gap, 0.0));
        self.painter.q.depth += 1;
        // ID 命名空间：滚动容器进入压栈、退出弹栈（闭包作用域保证配对）。
        self.with_id(id, |ui| f(ui));
        let frame = self.frames.pop().expect("scroll frame");
        let content_size = frame.settle_size();
        self.painter.q.depth -= 1;
        self.abs_base = saved_base;
        self.avail_stack.pop();
        let max_off_px = (content_size.y - view_size.y).max(0.0).round();
        let max_off_x_px = (content_size.x - view_size.x).max(0.0).round();
        offset_px = offset_px.clamp(0.0, max_off_px);
        offset_x_px = offset_x_px.clamp(0.0, max_off_x_px);
        // 滚轮（鼠标在可视区内且未被窗口遮挡；wheel y 向上为正 → offset 减小）。
        // 每格 40 物理像素取整（trackpad 连续增量同样按格取整步进）。
        // 横向增量取 wheel.x；**Shift + 滚轮**把它当横向（见 `wheel_axes`）。
        let hit = hit_test(&view_abs, self.mouse_logical)
            && self.mouse_in_window
            && !window_occluded(
                self.painter.q.cur_win,
                self.mouse_logical,
                self.window_rects_iter(),
            );
        if hit {
            let (wx, wy) = self.mouse.wheel();
            if wx != 0.0 || wy != 0.0 {
                let shift = self.key_down(winit::keyboard::KeyCode::ShiftLeft)
                    || self.key_down(winit::keyboard::KeyCode::ShiftRight);
                let (dx, dy) = wheel_axes(wx, wy, shift, v, h);
                if dy != 0.0 {
                    offset_px = (offset_px + dy).clamp(0.0, max_off_px);
                }
                if dx != 0.0 {
                    offset_x_px = (offset_x_px + dx).clamp(0.0, max_off_x_px);
                }
            }
        }
        // 平移内容子命令：局部坐标 → 绝对（`UiDraw::clip` 已是绝对，不随平移——见其
        // 文档）。offset 为物理像素 → 刚性平移。
        for d in &mut self.painter.q.queue[start..] {
            d.translate(pos - Vec2::new(offset_x_px, offset_px));
        }
        // 滚动条（内容超出可视区时显示；拖 thumb / 点轨道翻页）——在**当前容器局部
        // 坐标**绘制，**不参与**上面的内容平移；随外层容器弹出统一平移成绝对坐标。
        //
        // ⚠ 只有 `Scroll` 那条轴画条 / 吃滚动条交互（`ClipOnly` 是"只裁不滚"）；
        // 两条轴同时溢出时**拐角互让**：纵向条带矮 `SCROLLBAR_W`、横向条带窄
        // `SCROLLBAR_W`（两条带不重叠，见 `scrollbar_rects*` 的长度参数）。
        let v_bar =
            v == ScrollMode::Scroll && content_size.y > view_size.y + 1.0 && view_size.y > 0.0;
        let h_bar =
            h == ScrollMode::Scroll && content_size.x > view_size.x + 1.0 && view_size.x > 0.0;
        let (v_gap, h_gap) = (
            if h_bar { SCROLLBAR_W } else { 0.0 },
            if v_bar { SCROLLBAR_W } else { 0.0 },
        );
        // `elem` = **本放置的下一个元素序**（`elem_hint()`）：滚动条在**内容之后**录制，
        // 于是它排在本容器子内容**之上**——与 `scrollbar_axis` 的文档 / 它注册的控件级
        // 遮挡（"点滚动条不该连带触发被压住的列表项"）一致。
        //
        // ⚠ 这里**曾经传 `0`**（当成"容器装饰"）。但 `elem = 0` 的语义是"画在本容器
        // 元素**之下**"，于是列表项（`elem ≥ 1`）把滑块整块盖住 —— 用户报的
        // "ScrollBar 闪烁"（滑块只在条目间隙里露一条，滚动时忽隐忽现）。
        // 容器**底色 / 投影**用 `elem = 0` 是对的（它们确实该在最底），滚动条不是。
        if v_bar {
            offset_px = self.scrollbar_axis(
                &IdAbsolute::owned(format!("{}::bar", abs.as_str())),
                &view_rel,
                view_size.y - v_gap,
                content_size.y,
                offset_px,
                max_off_px,
                saved_clip,
                self.painter.q.seq + 1,
                false,
            );
        }
        if h_bar {
            offset_x_px = self.scrollbar_axis(
                &IdAbsolute::owned(format!("{}::hbar", abs.as_str())),
                &view_rel,
                view_size.x - h_gap,
                content_size.x,
                offset_x_px,
                max_off_x_px,
                saved_clip,
                self.painter.q.seq + 1,
                true,
            );
        }
        // 写回滚动状态（`f` 借用已结束；offset 为物理像素）。
        let st = self.state.scrolls.entry(abs.to_static()).or_default();
        st.offset = offset_px;
        st.offset_x = offset_x_px;
        st.content_h = content_size.y;
        st.content_w = content_size.x;
        self.painter.q.clip = saved_clip;
        ScrollOutcome {
            view: view_size,
            content: content_size,
        }
    }

    /// **选择列表**：`scroll_at` + 逐项回调（选中态由调用方维护）。
    ///
    /// `item` 回调 `(容器, 索引, 是否选中) -> bool`：返回 `true` 表示该项被点击。
    /// 返回本帧被点击的索引（`None` = 无）。
    pub fn list_at<F>(
        &mut self,
        pos: impl Into<Position>,
        view_size: impl Into<Size<Vec2>>,
        id: &str,
        count: usize,
        selected: Option<u32>,
        mut item: F,
    ) -> Option<u32>
    where
        F: FnMut(&mut Scroll<'_, '_>, u32, bool) -> bool,
    {
        let pos = pos.into().to_physical(self.scale);
        let view_size = view_size.into().to_physical(self.scale);
        let mut clicked = None;
        // 内部已是物理：显式 `Physical`（避免默认 Logical 二次换算）。
        self.scroll_at(
            Position::Physical(pos),
            Size::Physical(view_size),
            id,
            |s| {
                for i in 0..count as u32 {
                    if item(s, i, selected == Some(i)) && clicked.is_none() {
                        clicked = Some(i);
                    }
                }
            },
        );
        clicked
    }

    /// 滚动条：右侧竖条（轨道 + 胶囊滑块）。返回更新后的滚动偏移（**物理像素**）。
    ///
    /// 观感（本轮起）：**常驻**、比旧版更粗的**胶囊**滑块，居中于 [`SCROLLBAR_W`]
    /// 条带内 ⇒ **两侧留白**；配色取调色板的弱色（`text_dim`，悬停 / 拖拽转
    /// `text_muted`）而不再用近白的 `slider.handle`——深 / 浅两色都不刺眼。
    /// 条带（含留白）即命中 / 翻页热区，比可见滑块宽 ⇒ 抓取更容易。
    ///
    /// `view` 为**当前容器局部坐标**的可视区（与内容同空间，**不随内容滚动**；
    /// 由外层容器弹出统一平移成绝对坐标）；命中用局部坐标鼠标（`mouse_logical −
    /// abs_base`），遮挡判定仍用绝对鼠标。滑块几何在物理像素里取整（
    /// [`scroll_thumb`]），拖拽按 **整物理像素 1:1** 步进——内容与滑块刚性移动
    /// （非整数 DPI 不抖）。
    /// `elem`：所属元素序（`scroll_at` 传 `elem_hint()` ⇒ 覆盖在本容器内容之上；
    /// 文本编辑框传自己的 `elem`，同样在文本之上——**不要传 `0`**：那是"画在内容之下"，
    /// 会被自家内容盖住，见 `scroll_at` 里的说明）。
    ///
    /// 横向条请用 [`Self::scrollbar_axis`]（`horizontal = true`）。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn scrollbar(
        &mut self,
        id: &IdAbsolute<'static>,
        view: &Rect,
        view_h: f32,
        content_h: f32,
        offset_px: f32,
        max_off_px: f32,
        outer_clip: Option<Rect>,
        elem: u32,
    ) -> f32 {
        self.scrollbar_axis(
            id, view, view_h, content_h, offset_px, max_off_px, outer_clip, elem, false,
        )
    }

    /// **滚动条的轴无关实现**（[`Self::scrollbar`] 是纵向的薄封装）。
    ///
    /// `horizontal = false` = 右侧竖条（`scrollbar_rects`）；`true` = 底缘横条
    /// （[`scrollbar_rects_h`]）。滑块几何共用**轴无关**的 [`scroll_thumb`]——
    /// 只把"长度 / 行程 / 偏移"沿哪条轴从 `w/h`、`x/y` 里取出来。
    ///
    /// ⚠ `id` 已经是**完整**状态键（纵向 `…::bar` / 横向 `…::hbar` / 文本框 `…::vbar`）：
    /// 两条轴的状态必须分开，否则同一容器拖横条会带动竖条。
    #[allow(clippy::too_many_arguments)]
    fn scrollbar_axis(
        &mut self,
        id: &IdAbsolute<'static>,
        view: &Rect,
        view_len: f32,
        content_len: f32,
        offset_px: f32,
        max_off_px: f32,
        outer_clip: Option<Rect>,
        elem: u32,
        horizontal: bool,
    ) -> f32 {
        let mut offset_px = offset_px;
        // 条带（占位 + 命中 / 翻页热区）与**可见**轨道（居中、两端留白 ⇒ 胶囊不贴边）。
        let (strip, track) = if horizontal {
            scrollbar_rects_h(view, view_len)
        } else {
            scrollbar_rects(view, view_len)
        };
        let track_len = if horizontal { track.w } else { track.h };
        // 滑块几何：**物理像素**计算（整像素步进 → 刚性；纯函数可单测）。
        let (thumb_len_px, travel_px, thumb_off_px) =
            scroll_thumb(track_len, view_len, content_len, offset_px, max_off_px);
        // 滑块起点 = 轨道起点（局部坐标）+ 轨道内偏移（物理像素）。
        let thumb = if horizontal {
            Rect::new(track.x + thumb_off_px, track.y, thumb_len_px, track.h)
        } else {
            Rect::new(track.x, track.y + thumb_off_px, track.w, thumb_len_px)
        };
        // 交互判定必须在**绘制前**求出（滑块颜色取决于悬停 / 拖拽状态）。
        // 局部坐标鼠标 = 绝对鼠标 − 当前容器绝对原点（abs_base 已恢复为外层值）。
        let depth = self.painter.q.depth;
        let win = self.painter.q.cur_win;
        let mouse_rel = self.mouse_logical - self.abs_base;
        // **沿本轴**的鼠标分量（条带内的位置判翻页方向用）。
        let mouse_axis = if horizontal { mouse_rel.x } else { mouse_rel.y };
        let on_top = self.mouse_in_window
            && !window_occluded(win, self.mouse_logical, self.window_rects_iter());
        // **控件级遮挡**（与 `hit_abs` 同一套）：滚动条画在内容**之上**，
        // - 它自己参与遮挡链（鼠标在条带内时，条带下方的控件不得响应——否则
        //   "点滚动条"会连带触发被压住的列表项 / 文本插入符）；
        // - 同时也要能被**更晚录制**的控件挡住（对称处理，不搞特例）。
        // 登记用**条带**（滑块 + 两侧留白 + 上下留白）：热区即占位区。
        let me = id_hash(id);
        let key = self.painter.q.seq;
        let strip_abs = Rect::new(
            self.abs_base.x + strip.x,
            self.abs_base.y + strip.y,
            strip.w,
            strip.h,
        );
        if on_top && hit_test(&strip_abs, self.mouse_logical) {
            self.state.hit_regions.push(HitRegion {
                owner: me,
                key,
                rect: strip_abs,
                clip: self.painter.q.clip,
            });
        }
        let on_top = on_top
            && !widget_occluded(
                me,
                key,
                self.mouse_logical,
                self.state.prev_hit_regions.iter().copied(),
            );
        let bar_hit = on_top && hit_test(&thumb, mouse_rel);
        let strip_hit = on_top && hit_test(&strip, mouse_rel);
        let btn = self.mouse_left();
        // 滚动条自身有拖拽语义：按下（滑块 / 条带）置位 press_claimed，
        // 阻止外层窗口把本次按下当作窗口拖拽基准（窗口内拖滚动条不连窗口一起动）。
        if btn.down_edge() && strip_hit {
            self.press_claimed = true;
        }
        let grab = {
            let ws = self.state.widgets.entry(id.clone()).or_default();
            let dragging = update_drag(ws, bar_hit, btn);
            if btn.down_edge() && bar_hit {
                ws.press_mouse = Some(self.mouse_screen.round());
                ws.press_panel = Some(Vec2::new(thumb_off_px, offset_px));
            }
            (dragging, ws.press_panel.unwrap_or(Vec2::ZERO))
        };
        // 绘制：轨道 + 滑块（白纹理图形，`elem` 所属元素）。胶囊 = 半径取半宽。
        //
        // ⚠ **每条命令各取一次 `next_seq()`**：这里是少数"直接写队列"的入口之一，
        // 队列播放头 `DrawQueue::seq` 必须恒等于已分配的最大序号——否则下一条命令会
        // 拿到**重复序号**，而"放置序"（`z0_place_for_seq`）是按 `seq` 归一化的
        // ⇒ 重复序号跨越放置边界时会把同一容器的命令拆进两个排序空间（实测：滚动条的
        // 轨道与滑块被拆成 place=1 / place=2，滑块被列表项盖住 = 又变成闪烁）。
        // （旧写法 `let seq = next_seq(); push(seq); push(seq + 1)` 就是这样漏掉一次的。）
        let track_seq = self.next_seq();
        let thumb_seq = self.next_seq();
        let radius = CornerRadius::all(SCROLLBAR_BAR_W * 0.5);
        let pal = self.theme.palette;
        let thumb_col = if bar_hit || grab.0 {
            pal.text_muted
        } else {
            pal.text_dim
        };
        self.painter.q.queue.push(UiDraw {
            depth,
            seq: track_seq,
            win,
            elem,
            rect: track,
            clip: outer_clip,
            full_w: false,
            // 滚动条轨道用主题滑块轨道刷（可能是渐变：纯色 → **胶囊**圆角矩形；
            // 渐变无法圆角 → 退化成四角顶点色的 `Rect`。两者都是一条命令、无纹理）。
            kind: match self.theme.slider.track.as_solid() {
                Some(c) => DrawKind::RoundedRect {
                    corners: [c; 4],
                    radius,
                },
                None => DrawKind::Rect(Gradient::corners(
                    self.theme.slider.track.corners()[0],
                    self.theme.slider.track.corners()[1],
                    self.theme.slider.track.corners()[2],
                    self.theme.slider.track.corners()[3],
                )),
            },
        });
        self.painter.q.queue.push(UiDraw {
            depth,
            seq: thumb_seq,
            win,
            elem,
            rect: thumb,
            clip: outer_clip,
            full_w: false,
            kind: DrawKind::RoundedRect {
                corners: [thumb_col; 4],
                radius,
            },
        });
        if grab.0 {
            let pm = self
                .state
                .widgets
                .get(id.as_str())
                .and_then(|w| w.press_mouse)
                .unwrap_or(self.mouse_screen);
            // 滑块**跟随鼠标 1:1**（保持按下时的抓取点偏移），滚动偏移由滑块
            // 位置反推——否则滑块按比例慢于鼠标（内容越长越明显，"不同步"）。
            let d_px = if horizontal {
                (self.mouse_screen.x - pm.x).round()
            } else {
                (self.mouse_screen.y - pm.y).round()
            };
            let thumb_off_px_new = grab.1.x + d_px; // grab.1.x = 按下时 thumb_off_px
            offset_px = scroll_offset_for_thumb(thumb_off_px_new, travel_px, max_off_px);
        }
        // 光标：视口滑条（滑块 / 条带）保持普通 Arrow（UI_NEEDS：滑条不用 <->）。
        // 条带点击（滑块外）→ 翻页（整物理像素步长）。
        let page_px = view_len.round();
        let (thumb_lo, thumb_hi) = if horizontal {
            (thumb.x, thumb.x + thumb.w)
        } else {
            (thumb.y, thumb.y + thumb.h)
        };
        if btn.down_edge() && strip_hit && !bar_hit {
            if mouse_axis < thumb_lo {
                offset_px = (offset_px - page_px).max(0.0);
            } else if mouse_axis > thumb_hi {
                offset_px = (offset_px + page_px).min(max_off_px);
            }
        }
        offset_px
    }

    // ── 顶层入口（*_at：位置显式，尺寸自动） ─────────────────
}

/// **滚动条几何**（纯函数，物理像素）：返回 `(条带, 可见轨道)`。
///
/// - 条带 = 右缘 [`SCROLLBAR_W`] 宽的全高矩形：**占位 + 命中 / 翻页热区**（比
///   可见滑块宽 ⇒ 抓取更容易，也避免了"滑块太细点不中"）；
/// - 可见轨道 = 条带**居中**的 [`SCROLLBAR_BAR_W`] 宽胶囊（两侧各留
///   `(SCROLLBAR_W − SCROLLBAR_BAR_W) / 2` 的空白），上下各留 [`SCROLLBAR_MARGIN`]
///   （胶囊两端不贴可视区边缘）。
pub(crate) fn scrollbar_rects(view: &Rect, view_h: f32) -> (Rect, Rect) {
    let strip = Rect::new(view.x + view.w - SCROLLBAR_W, view.y, SCROLLBAR_W, view_h);
    let track = Rect::new(
        strip.x + (SCROLLBAR_W - SCROLLBAR_BAR_W) * 0.5,
        strip.y + SCROLLBAR_MARGIN,
        SCROLLBAR_BAR_W,
        (view_h - SCROLLBAR_MARGIN * 2.0).max(1.0),
    );
    (strip, track)
}

/// **横向滚动条几何**（纯函数，物理像素；[`scrollbar_rects`] 的镜像）：返回 `(条带, 可见轨道)`。
///
/// - 条带 = **底缘** [`SCROLLBAR_W`] 高的全宽矩形（占位 + 命中 / 翻页热区）；
/// - 可见轨道 = 条带**居中**的 [`SCROLLBAR_BAR_W`] 高胶囊（上下各留
///   `(SCROLLBAR_W − SCROLLBAR_BAR_W) / 2` 的空白），左右各留 [`SCROLLBAR_MARGIN`]
///   （胶囊两端不贴可视区边缘）。
///
/// `view_w` 由调用方给（两条轴都溢出时传 `视口宽 − SCROLLBAR_W`：**拐角互让**给竖条，
/// 与 [`scrollbar_rects`] 收 `view_h − SCROLLBAR_W` 对称）。
pub(crate) fn scrollbar_rects_h(view: &Rect, view_w: f32) -> (Rect, Rect) {
    let strip = Rect::new(view.x, view.y + view.h - SCROLLBAR_W, view_w, SCROLLBAR_W);
    let track = Rect::new(
        strip.x + SCROLLBAR_MARGIN,
        strip.y + (SCROLLBAR_W - SCROLLBAR_BAR_W) * 0.5,
        (view_w - SCROLLBAR_MARGIN * 2.0).max(1.0),
        SCROLLBAR_BAR_W,
    );
    (strip, track)
}

/// **滚动容器内是否上报"可用宽"**（纯函数，可单测）。
///
/// `h == Scroll` 意味着**这条轴要横着滚** ⇒ 内容必须保持**自然宽**（不上报可用宽，
/// `LimitedInParent` 控件不折行 / 不省略），否则内容永远被压进视口、横向滚动条永远
/// 不出现。其余情况（`NoClip` 压缩 / `ClipOnly` 只裁）保持上报视口宽 —— 既有滚动列表
/// 的折行行为因此逐像素不变。
#[inline]
pub(super) fn scroll_axes_avail_w(h: ScrollMode, view_w: f32) -> Option<f32> {
    if h == ScrollMode::Scroll {
        None
    } else {
        Some(view_w)
    }
}

/// **滚轮 → 两条轴的滚动偏移增量**（纯函数，可单测；物理像素，正 = 偏移增大）。
///
/// - 每格 40 物理像素、取整（trackpad 连续增量同样按格步进）；
/// - **Shift + 滚轮**改成横向（`-wy`：Shift + 向下滚 ⇒ 往右滚）；
/// - **只有一条轴可滚**（`Scroll`）时，把任何轮子增量都交给它（普通鼠标 / 触控板
///   最常见的 UX：竖轮子滚横向列表）；
/// - 不可滚（`NoClip` / `ClipOnly`）那条轴的增量恒为 0。
pub(super) fn wheel_axes(
    wx: f64,
    wy: f64,
    shift: bool,
    v: ScrollMode,
    h: ScrollMode,
) -> (f32, f32) {
    /// 滚轮每格步进（物理像素）——与纵向滚动条的历史口径一致。
    const WHEEL_STEP: f32 = 40.0;
    let (v_on, h_on) = (v == ScrollMode::Scroll, h == ScrollMode::Scroll);
    // 统一到"意图"空间：`ix` 正 = 想往右滚，`iy` 正 = 想往下滚（= offset 增大）。
    let (mut ix, mut iy) = if shift { (-wy, 0.0) } else { (wx, -wy) };
    if h_on && !v_on && ix == 0.0 {
        ix = iy;
        iy = 0.0;
    } else if v_on && !h_on && iy == 0.0 {
        iy = ix;
        ix = 0.0;
    }
    if !h_on {
        ix = 0.0;
    }
    if !v_on {
        iy = 0.0;
    }
    (
        (ix as f32 * WHEEL_STEP).round(),
        (iy as f32 * WHEEL_STEP).round(),
    )
}

/// **滚动条滑块几何**（纯函数，物理像素，全部取整 ⇒ 整像素步进、不抖）。
///
/// 输入：`track_h` 轨道高、`view_h` / `content_h` 可视高 / 内容高、`offset_px` /
/// `max_off_px` 当前 / 最大滚动偏移。返回 `(thumb_h, travel, thumb_y)`：
/// 滑块长、滑块行程（= 轨道高 − 滑块长）、滑块顶端相对**轨道顶端**的偏移。
///
/// 不变量：内容装得下（`ratio ≥ 1`）⇒ 滑块铺满轨道、行程 0；`offset_px ==
/// max_off_px` ⇒ `thumb_y == travel`（滑块底端与轨道底端对齐）——滑块与内容
/// 刚性对应，这正是"滚到底"时视觉上真的贴底的原因。
pub(crate) fn scroll_thumb(
    track_h: f32,
    view_h: f32,
    content_h: f32,
    offset_px: f32,
    max_off_px: f32,
) -> (f32, f32, f32) {
    let track_h = track_h.round().max(1.0);
    let ratio = if content_h > 0.0 {
        (view_h / content_h).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let thumb_h = (track_h * ratio)
        .max(SCROLLBAR_MIN_THUMB.min(track_h))
        .min(track_h)
        .round();
    let travel = (track_h - thumb_h).max(0.0);
    let thumb_y = if max_off_px > 1e-6 && travel > 1.0 {
        (offset_px / max_off_px * travel).round().clamp(0.0, travel)
    } else {
        0.0
    };
    (thumb_h, travel, thumb_y)
}

/// **滚动条滑块位置 → 滚动偏移**（[`scroll_thumb`] 的逆，拖拽用；物理像素取整）。
#[inline]
pub(crate) fn scroll_offset_for_thumb(thumb_y: f32, travel: f32, max_off_px: f32) -> f32 {
    if travel > 1.0 {
        (thumb_y.clamp(0.0, travel) / travel * max_off_px)
            .round()
            .clamp(0.0, max_off_px)
    } else {
        0.0
    }
}
