//! 遗留 `*_at` 控件实现：滑块 / 勾选 / 单选（含公共绘制）、缩放柄、可调整大小的
//! 文本输入框包装，以及 `text_input_at` / `text_area_at` 薄封装。
//!
//! 维护者笔记：文本编辑**核心**在 `ui::textedit`；本模块只做「解析样式 → 交给核心」。
//! 这些 `*_at` 是公开面的一部分（`Ui::slider_at` 等），搬去 `widgets/` 属于另一轮
//! 结构欠账（`docs/UI_ARCHITECTURE.md` §5.1），不要在本模块顺手改行为。

use super::*;
use std::ops::RangeInclusive;
use std::sync::Arc;

use glam::Vec2;
use rjw_color::Color;
use rjw_keyboard::KeyCode;
use rjw_transform::Rect;

use crate::draw::{text_cmd, DrawKind, TextAlign, TextVAlign, UiDraw};
use crate::focus::FocusKind;
use crate::hit::{normalize_x, update_drag, update_interact};
use crate::state::CheckboxState;
use crate::style::{CheckboxStyle, GripStyle};

impl Ui<'_> {
    /// 滑块（显式 rect；拖拽灵敏度 = 1，值随鼠标 1:1）。
    pub fn slider_at(
        &mut self,
        id: &str,
        rect: Rect,
        range: RangeInclusive<f32>,
        value: f32,
    ) -> f32 {
        self.slider_at_drag(id, rect, range, value, 1.0)
    }

    /// 滑块（显式 rect；`sens` = 拖拽灵敏度，每像素数值 = 轨道全值 / 宽 × `sens`）。
    ///
    /// - `sens = 1.0`：值随鼠标 1:1（点击轨道即定位，拖拽从按下位置值增量）；
    /// - `sens > 1` 更快、`< 1` 更慢（`widgets::Slider` 的 `drag_sensitivity`）；
    /// - Shift/Ctrl 速度倍率由控件作者并入 `sens`（如 `widgets::Slider` 的
    ///   `shift_speed` / `ctrl_speed`）。
    ///
    /// 非公开：灵敏度只由 widget builder 决定，避免"同一种控件两条入口"。
    pub(crate) fn slider_at_drag(
        &mut self,
        id: &str,
        rect: Rect,
        range: RangeInclusive<f32>,
        value: f32,
        sens: f32,
    ) -> f32 {
        let style = self.theme.slider.clone();
        self.slider_at_styled(id, rect, range, value, sens, &style)
    }

    /// 滑块（显式 rect + **样式可覆盖** + 灵敏度）——widget 层 / 组合控件经此合并主题
    /// 与逐控件属性（[`Self::slider_at_drag`] 委托本方法）。
    ///
    /// 非公开：样式必须来自 [`crate::style::Theme`] 或控件 builder，避免"同一种控件两条入口"。
    /// 取色器的**通道颜色滑块**靠它给出"该通道 0→最大"的水平渐变轨道
    /// （[`crate::Brush::Horizontal`]）——渐变轨道是刷，不是单色，故必须能逐次覆盖样式。
    pub(crate) fn slider_at_styled(
        &mut self,
        id: &str,
        rect: Rect,
        range: RangeInclusive<f32>,
        value: f32,
        sens: f32,
        style: &crate::style::SliderStyle,
    ) -> f32 {
        let id_for = self.id_for(id);

        self.note_placed(rect);
        let hit = self.hit_abs(&id_for, &rect);
        let btn = self.mouse_left();
        // 登记焦点链（键盘导航：Tab 可到；焦点下左右方向键调值 —— 见下方键盘分支）。
        self.register_focus(&id_for, rect, FocusKind::Slider);
        // 滑块自身有拖拽语义：按下即置位 press_claimed——阻止外层窗口/面板把本次
        // 按下当作拖拽基准（窗口内拖滑块不再连窗口一起动）。
        if btn.down_edge() && hit {
            self.press_claimed = true;
        }
        let (lo, hi) = (*range.start(), *range.end());
        let span = hi - lo;
        // 按下位置的轨道比例（点击即定位；拖拽基准用）。
        let t0 = if span.abs() > f32::EPSILON {
            normalize_x(&rect, self.mouse_local_x()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut new_value = value;
        // 按下基准值先取出（&self 读取，避免与下方 self.state 可变借用冲突）。
        let press_mx = self.mouse_local_x();
        let active = {
            let ws = self.state.widgets.entry(id_for.to_static()).or_default();
            let dragging = update_drag(ws, hit, btn);
            if btn.down_edge() && hit {
                // 拖拽基准：按下鼠标**局部 x**（与增量计算 `mouse_local_x` 同单位——
                // 旧实现混用屏幕坐标导致按下瞬间 dx = -abs_base → 值跳 0）
                // + 按下位置对应值（点击轨道即定位）。
                ws.press_mouse = Some(Vec2::new(press_mx, 0.0));
                ws.press_panel = Some(Vec2::new(press_mx, lo + t0 * span));
            } else if ws.drag_sens != 0.0 && ws.drag_sens != sens {
                // 灵敏度（Shift/Ctrl）变化 → 重设拖拽基准：从**当前值**继续增量
                // （否则 `Δx × 新 sens` 使值瞬间跳变）。
                ws.press_mouse = Some(Vec2::new(press_mx, 0.0));
                ws.press_panel = Some(Vec2::new(press_mx, new_value));
            }
            ws.drag_sens = sens;
            dragging
        };
        // 光标：滑块悬停 / 拖拽 → **左右箭头**（↔，EwResize）——水平调值语义。
        // 用 `cursor_custom`（而非 `cursor_grab/grabbing`）以保证拖拽中也显示 ↔
        // （finish 里 `cursor_grabbing` 优先于 `cursor_custom`）。
        if active || hit {
            self.cursor_custom = Some(UiCursor::EwResize.to_winit());
        }
        if active {
            self.any_pressed = true;
            if span.abs() > f32::EPSILON {
                // **增量拖拽**：从按下位置对应值开始，每像素数值 = 全值/宽 × `sens`。
                // （旧实现"绝对位置"无法表达灵敏度，且拖拽必须落点在轨道内。）
                let (px, pv) = {
                    let ws = self.state.widgets.get(id_for.as_str());
                    let pm = ws.and_then(|w| w.press_mouse).unwrap_or(self.mouse_screen);
                    let pp = ws
                        .and_then(|w| w.press_panel)
                        .unwrap_or(Vec2::new(self.mouse_local_x(), new_value));
                    (pm.x, pp.y)
                };
                let dx = self.mouse_local_x() - px;
                // 增量拖拽：从按下位置对应值开始；**clamp 到 [lo, hi]**（防越界）。
                new_value = (pv + dx * (span / rect.w.max(1.0)) * sens).clamp(lo, hi);
            }
        }
        // 键盘：焦点下滑块用左右方向键调值（步进 = 范围的 5%，即时生效）。
        if self.focused_is(&id_for) && span.abs() > f32::EPSILON {
            let step = span * 0.05;
            if self.keyboard.key(KeyCode::ArrowLeft).down_edge() {
                new_value = (new_value - step).clamp(lo, hi);
            }
            if self.keyboard.key(KeyCode::ArrowRight).down_edge() {
                new_value = (new_value + step).clamp(lo, hi);
            }
        }
        let t = if span.abs() > f32::EPSILON {
            ((new_value - lo) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let elem = self.painter.q.seq + 1;
        let track_rect = Rect::new(
            rect.x,
            rect.y + (rect.h - style.track_h) * 0.5,
            rect.w,
            style.track_h,
        );
        // 手柄**中心**夹在轨道两端之内（t=0/1 时手柄不伸出轨道/控件外）；
        // 填充画到手柄**左缘**（与手柄无缝衔接，而非只到手柄中心——消除"错位"）。
        let handle_cx = rect.x + style.handle_w * 0.5 + (rect.w - style.handle_w) * t;
        let fill_w = (handle_cx - style.handle_w * 0.5 - rect.x).max(0.0);
        let fill_rect = Rect::new(rect.x, track_rect.y, fill_w, style.track_h);
        let handle_rect = Rect::new(
            handle_cx - style.handle_w * 0.5,
            rect.y + (rect.h - style.handle_w) * 0.5,
            style.handle_w,
            style.handle_w,
        );
        // 轨道 / 填充 / 手柄都是**圆角**矩形（`SliderStyle::radius`，默认胶囊）。
        // 填充画在手柄**左缘**且与手柄同高——两者都是胶囊时左右端自然接成一条。
        //
        // 轨道是**刷**（可能是"该通道 0→最大"的水平渐变）：无边框（`border_w = 0`，
        // 与旧行为一致，边框色不参与绘制）。填充刷**全透明时不画**——取色器的通道行
        // 靠轨道本身表达色彩，纯色填充会盖掉斜坡，而且省一段几何。
        self.push_panel_like(
            track_rect,
            style.track,
            Color::TRANSPARENT,
            0.0,
            style.radius,
            elem,
        );
        let fill_alpha = style.fill.as_solid().map(|c| {
            let a: [f32; 4] = c.into();
            a[3]
        });
        if fill_alpha != Some(0.0) {
            self.push_panel_like(
                fill_rect,
                style.fill,
                Color::TRANSPARENT,
                0.0,
                style.radius,
                elem,
            );
        }
        self.push_panel_like(
            handle_rect,
            style.handle,
            style.handle_border,
            if style.handle_w > 2.0 { 1.0 } else { 0.0 },
            style.radius,
            elem,
        );
        new_value
    }

    /// 勾选框（显式 rect；样式取全局 `Theme::checkbox`）。
    pub fn checkbox_at(
        &mut self,
        id: &str,
        rect: Rect,
        label: &str,
        checked: bool,
    ) -> CheckboxState {
        let style = self.theme.checkbox.clone();
        self.checkbox_at_styled(id, rect, label, checked, &style)
    }

    /// 勾选框（显式 rect + **样式可覆盖**——widget 层 [`crate::widgets::Checkbox`] 经此
    /// 合并主题与逐控件属性；[`Self::checkbox_at`] 委托本方法）。
    ///
    /// 非公开：样式必须来自 [`Theme`] 或 widget builder，避免"同一种控件两条入口"。
    pub(crate) fn checkbox_at_styled(
        &mut self,
        id: &str,
        rect: Rect,
        label: &str,
        checked: bool,
        style: &CheckboxStyle,
    ) -> CheckboxState {
        let abs = self.id_for(id);
        self.note_placed(rect);
        // 命中 / 焦点链 / 键盘激活 / 跨帧状态机：一句话（勾选框没有拖拽语义）。
        let resp = self.interact(
            &abs,
            rect,
            crate::widgets::Sense::CLICK.focus(FocusKind::Checkbox),
        );
        self.draw_check_common(rect, label, checked, resp.hovered, style);
        CheckboxState {
            hovered: resp.hovered,
            pressed: resp.pressed,
            checked,
            toggled: resp.clicked,
            clicked: resp.clicked,
        }
    }

    /// 单选（显式 rect）。
    pub fn radio_at(&mut self, id: &str, group: &str, rect: Rect, label: &str) -> CheckboxState {
        // 单选 id 也参与命名空间（组名 `group` 不前缀——跨窗口复用组语义保留）。
        let abs = self.id_for(id);
        self.note_placed(rect);
        let hit = self.hit_abs(&abs, &rect);
        let btn = self.mouse_left();
        // 登记焦点链（键盘导航：Tab 可到；Enter/Space 选中）。
        self.register_focus(&abs, rect, FocusKind::Radio);
        let key_click = self.key_click(&abs, FocusKind::Radio);
        if key_click {
            self.any_pressed = true;
        }
        let mut ev = {
            let ws = self.state.widgets.entry(abs.to_static()).or_default();
            let ev = update_interact(ws, hit, btn);
            if key_click {
                ws.pressed = true;
            }
            ev
        };
        if key_click {
            ev.clicked = true;
        }
        if ev.pressed {
            self.any_pressed = true;
        }
        let was_checked = self
            .state
            .radio_groups
            .get(group)
            .is_some_and(|s| s.as_str() == abs.as_str());
        if ev.clicked {
            self.state
                .radio_groups
                .insert(group.to_owned(), abs.to_static());
        }
        let checked = self
            .state
            .radio_groups
            .get(group)
            .is_some_and(|s| s.as_str() == abs.as_str());
        let style = self.theme.checkbox.clone();
        let (hovered, pressed) = {
            let ws = self.state.widgets.get(abs.as_str()).expect("radio ws");
            (ws.hovered, ws.pressed)
        };
        self.draw_check_common(rect, label, checked, hovered, &style);
        CheckboxState {
            hovered,
            pressed,
            checked,
            toggled: ev.clicked && !was_checked,
            clicked: ev.clicked,
        }
    }

    /// 勾选框 / 单选公共绘制：方框 +（选中时）填充 + 标签文本（样式可覆盖）。
    fn draw_check_common(
        &mut self,
        rect: Rect,
        label: &str,
        checked: bool,
        hovered: bool,
        style: &CheckboxStyle,
    ) {
        let depth = self.painter.q.depth;
        let win = self.painter.q.cur_win;
        let elem = self.painter.q.seq + 1;
        let box_rect = Rect::new(
            rect.x,
            rect.y + (rect.h - style.box_size) * 0.5,
            style.box_size,
            style.box_size,
        );
        let seq = self.next_seq();
        // **边框宽 = 0 时的兜底底色**：未勾选的方框本来只画一圈描边——主题把
        // `border_w` 拖到 0（"平面风格"）后它**整个消失**，标签看起来像"没有控件"
        // （用户实测："控件严重错位"：两个勾选框一个有一个没有）。
        // 这里退化成**实心底**（下沉色 / 悬停色），保证任何主题下都看得见方框。
        let flat = style.border_w <= 0.0;
        if !checked && flat {
            let bg = if hovered {
                self.theme.palette.surface_hover
            } else {
                self.theme.palette.surface_sunken
            };
            let seq = self.next_seq();
            self.painter.q.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: box_rect,
                clip: self.painter.q.clip,
                full_w: false,
                kind: DrawKind::RoundedRect {
                    corners: [bg; 4],
                    radius: style.radius,
                },
            });
        } else {
            self.painter.q.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: box_rect,
                clip: self.painter.q.clip,
                full_w: false,
                kind: DrawKind::Border {
                    // 悬停时方框描边转向强调色——与按钮 / 下拉框的悬停反馈一致
                    // （此前勾选框 hover 毫无变化，鼠标移上去看不出"可以点"）。
                    color: if hovered {
                        self.theme.focus.color
                    } else {
                        style.box_border
                    },
                    width: style.border_w,
                    radius: style.radius,
                },
            });
        }
        if checked {
            // 中心填充 = 外框 **内缩**（减法，非写死偏移）：
            // inset（物理像素）= floor(border_w) + floor(CHECKBOX_INNER)，
            // 内缩量与边框一致，任意缩放不溢出。
            let inset_px = style.border_w.floor() + CHECKBOX_INNER.floor();
            let inner = box_rect.shrink(inset_px);
            if inner.w > 0.0 && inner.h > 0.0 {
                let seq = self.next_seq();
                // 填充与外框同心的内圆角（`radius - inset`，clamp 到 0）——
                // 与外框环带的内侧半径取同一套规则，两者贴合不留缝。
                let fill_radius = style.radius.map(|r| (r - inset_px).max(0.0));
                self.painter.q.queue.push(UiDraw {
                    depth,
                    seq,
                    win,
                    elem,
                    rect: inner,
                    clip: self.painter.q.clip,
                full_w: false,
                    kind: DrawKind::RoundedRect {
                        corners: [style.checked_fill; 4],
                        radius: fill_radius,
                    },
                });
            }
        }
        let text_rect = Rect::new(
            box_rect.x + style.box_size + style.gap,
            rect.y,
            (rect.w - style.box_size - style.gap).max(0.0),
            rect.h,
        );
        // 标签文本自动省略（缩窄 / max 约束下不溢出，内容自洽）。
        let label_owned = self.ellipsized(
            label,
            style.font_size,
            style.font_family.as_deref(),
            text_rect.w,
        );
        let draw_label: &str = label_owned.as_deref().unwrap_or(label);
        let seq = self.next_seq();
        self.painter.q.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            text_rect,
            Arc::from(draw_label),
            style.font_size,
            style.fg,
            TextAlign::Left,
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.painter.q.clip,
            None,
        ));
    }

    /// 绘制**右下角缩放柄**（拖动框；形状 / 颜色 / 尺寸全部来自
    /// [`InputStyle::grip`](crate::style::InputStyle::grip)，默认三条横线）。
    /// `rect` 为文本框本体矩形（当前容器局部坐标）；交互由 [`Self::resize_handle`] 处理。
    fn draw_resize_grip(&mut self, rect: Rect, grip: &GripStyle) {
        self.push_resize_grip_at(rect, grip);
    }

    /// **可调整宽度的文本输入框**（单行）：右下角拖拽改宽度（高度固定），尺寸跨帧
    /// 持久于 [`UiState::sizes`]，也可由 [`Self::size_handler`]（尺寸责任链）指定/覆盖。
    ///
    /// - `rect`：**初始**矩形（`x/y` 为位置，`w` 为初始宽；高度取 `rect.h`）；
    ///   `min_w`：最小宽；`resize`：[`Resize`]（单行输入框只支持 [`Resize::None`] /
    ///   [`Resize::Horizontal`]，`Both` 等价 `Horizontal`——高度由行高固定）。
    /// - 拖拽结果当帧生效（下一帧起按新宽布局，同 `window(width)` 的 1 帧滞后）。
    pub fn resizable_text_input_at(
        &mut self,
        id: &str,
        rect: Rect,
        value: &mut String,
        min_w: f32,
        resize: Resize,
    ) {
        let id_for = self.id_for(id);
        // 尺寸责任链解析宽度（脚本/布局/用户拖拽覆盖），高度固定 = 传入 rect 高。
        let w = self
            .resolve_size(&id_for, Vec2::new(rect.w, rect.h))
            .x
            .max(min_w);
        let input_rect = Rect::new(rect.x, rect.y, w, rect.h);
        self.text_input_core(id, input_rect, value);
        if resize != Resize::None {
            let style = self.theme.input.clone();
            let hw = 14.0_f32;
            let handle = Rect::new(
                input_rect.x + input_rect.w - hw,
                input_rect.y + input_rect.h - hw,
                hw,
                hw,
            );
            let h_id = format!("{id}::resize");
            if let Some(new) = self.resize_handle(
                &h_id,
                handle,
                Vec2::new(w, input_rect.h),
                Vec2::new(min_w, input_rect.h),
                crate::UiCursor::EwResize,
            ) {
                self.state
                    .sizes
                    .insert(id_for.to_static(), Vec2::new(new.x, input_rect.h));
            }
            self.draw_resize_grip(input_rect, &style.grip);
        }
    }

    /// **可调整大小的文本输入框（多行 TextArea）**：右下角拖拽改尺寸，尺寸跨帧
    /// 持久于 [`UiState::sizes`]，也可由 [`Self::size_handler`]（尺寸责任链）指定/覆盖。
    ///
    /// - `rect`：**初始**矩形；`min`：最小尺寸（`(min_w, min_h)`）；`resize`：[`Resize`]
    ///   （[`Resize::None`] 不显示缩放柄 / [`Resize::Horizontal`] 只调宽 /
    ///   [`Resize::Both`] 宽高同调）。
    /// - 自动换行（`wrap=true`，同 [`Self::text_area_at`]）；宽度变化会触发重新换行。
    pub fn resizable_text_area_at(
        &mut self,
        id: &str,
        rect: Rect,
        value: &mut String,
        min: Vec2,
        resize: Resize,
    ) {
        let id_for = self.id_for(id);
        let resolved = self.resolve_size(&id_for, Vec2::new(rect.w, rect.h));
        let area_rect = Rect::new(rect.x, rect.y, resolved.x.max(min.x), resolved.y.max(min.y));
        self.text_area_impl(id, area_rect, value, true);
        if resize != Resize::None {
            let style = self.theme.input.clone();
            let hw = 14.0_f32;
            let handle = Rect::new(
                area_rect.x + area_rect.w - hw,
                area_rect.y + area_rect.h - hw,
                hw,
                hw,
            );
            // 只调宽：高度锁死为当前高（`min.y` 也取当前高）。
            let (min_size, cursor) = match resize {
                Resize::Horizontal => (Vec2::new(min.x, area_rect.h), crate::UiCursor::EwResize),
                _ => (min, crate::UiCursor::NwseResize),
            };
            let h_id = format!("{id}::resize");
            if let Some(new) = self.resize_handle(
                &h_id,
                handle,
                Vec2::new(area_rect.w, area_rect.h),
                min_size,
                cursor,
            ) {
                self.state.sizes.insert(id_for.to_static(), new);
            }
            self.draw_resize_grip(area_rect, &style.grip);
        }
    }

    /// **单行文本输入框**（显式 `rect`）——[`crate::widgets::TextEditor`] 的薄封装。
    ///
    /// 需要宽 / 高 / 缩放柄 / 字号等属性时直接用责任链
    /// [`crate::widgets::TextEditor`]（本方法等价 `TextEditor::new(id, value).at(rect)`）。
    /// 编辑能力见 [`Self::text_input_core`]。
    pub fn text_input_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        crate::widgets::Widget::ui(crate::widgets::TextEditor::new(id, value).at(rect), self);
    }
}

impl Ui<'_> {
    /// **多行文本输入框（自动换行，显式 `rect`）**——[`crate::widgets::TextEditor`] 的薄封装。
    ///
    /// 等价 `TextEditor::new(id, value).at(rect).multiline()`；宽 / 高 / 缩放柄 / 字号等
    /// 属性请用责任链。编辑能力见 [`Self::text_area_impl`]。
    pub fn text_area_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        crate::widgets::Widget::ui(
            crate::widgets::TextEditor::new(id, value)
                .at(rect)
                .multiline(),
            self,
        );
    }

    /// **多行文本输入框（不自动换行，显式 `rect`）**——等价
    /// `TextEditor::new(id, value).at(rect).multiline().no_wrap()`。
    pub fn text_area_at_nw(&mut self, id: &str, rect: Rect, value: &mut String) {
        crate::widgets::Widget::ui(
            crate::widgets::TextEditor::new(id, value)
                .at(rect)
                .multiline()
                .no_wrap(),
            self,
        );
    }
}
