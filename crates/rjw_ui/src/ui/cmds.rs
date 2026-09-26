//! 命令 → 几何：`collect_cmds` 镶嵌、`cmd_sig` 内容签名、`draw_text_quads` 字形四边形、
//! 拖拽激活 / 位置求解、以及键盘导航的帧末处理。
//!
//! 维护者笔记：`cmd_sig` 必须覆盖一切渲染相关字段（尤其颜色 / 文本 / 字重），
//! 漏一个就表现为「交互变色不刷新」——`ui/tests.rs` 有回归防线。

use super::*;
use super::commit::local_of;
use std::sync::Arc;

use glam::Vec2;
use rjw_color::Color;
use rjw_keyboard::KeyCode;
use rjw_keystate::KeyState;
use rjw_text::Buffer;
use rjw_transform::Rect;

use crate::draw::{
    CornerRadius, DrawKind, Gradient, TextAlign, TextRamp,
    TextVAlign,
    UiDraw, border_rects, centered_square, debug_shape_segments, intersect_rect,
    screen_fixed_tf, snap_rect, text_block_offset,
};
// 顶点收集 / 合批机制（原在 `ui.rs`，见 `gpu_batch` 模块文档）。
use crate::gpu_batch::{
    QuadCollector,
    cmd_sig_hash, debug_layout_outline, fully_outside,
    resample_gradient_local, text_visible_rect, vertex_p3u2c4,
};
use crate::focus::{focus_step, FocusEntry, FocusKind};
use crate::hit::update_drag;
use crate::state::WidgetState;

impl<'a> Ui<'a> {
    /// 把一组命令收集为四边形顶点（**相对窗口原点的局部物理像素**；
    /// `win` 决定局部化基准，非窗口 win=0 基准 (0,0)）。
    ///
    /// `debug_layout` 开启时，每个命令的矩形同时向 `quads.debug` 追加**青色描边**
    /// （调试 rjw_ui 自身的布局 / 命中区域）；`DrawKind::Debug` 命令（[`Self::debug_line`]
    /// 等屏幕空间调试图元）则只写入 `quads.debug`（覆盖在 UI 内容之上）。
    pub(super) fn collect_cmds(
        &mut self,
        quads: &mut QuadCollector,
        win: u32,
        cmds: &[Vec<UiDraw>],
    ) {
        let anchor_px = self
            .win_origins
            .get(&win)
            .copied()
            .unwrap_or(Vec2::ZERO);
        let dbg = if self.debug_layout {
            // debug_layout 描边样式：读 Theme::debug（Copy 值先取出，
            // 避免与循环内 `&mut self` 调用（draw_text_quads）的借用冲突）。
            Some((self.theme.debug.layout_outline, self.theme.debug.layout_outline_width))
        } else {
            None
        };
        // depth 桶展平（桶序 = depth 升序，桶内录制序）：免排序下仍满足提交序。
        for d in cmds.iter().flatten() {
            // **当前放置序**：win=0 的排序空间（窗口内恒 0——窗口本身就是一个排序空间）。
            // 见 `Self::z0_ranges`：没有它，`elem = 0` 的容器装饰会被别的 win=0 放置穿透。
            quads.cur_place = if win == 0 { self.z0_place_for_seq(d.seq) } else { 0 };
            // 当前元素序：push 方法按其分组（控件级提交顺序——见 QuadCollector）。
            quads.cur_elem = d.elem;
            // **当前裁剪层的绝对矩形**（`d.clip` 已是绝对坐标）：**剔除**用它。
            let clip_abs = d.clip.map(|c| snap_rect(&c));
            // **环境裁剪层**（**窗口局部**；内容已随容器平移成绝对坐标后减本窗原点）。
            //
            // ⚠ 这里**不再切割几何**（旧实现逐命令 `clipped(..)`）：环境裁剪改由 batch
            // scissor 在 GPU 侧执行（见 `UiBatch::clip` / `view::batch_scissor`）。收益：
            // ① 圆角 / 环带不再被切平；② 省掉每命令的矩形求交与渐变重采样；
            // ③ 投影不再需要"部分可见就整块跳过"的特例（像素级裁边更干净）。
            //
            // ⚠ **必须存局部坐标**：它与缓存里的顶点同空间 ⇒ 窗口移动时缓存键与裁剪
            // 一起"跟着走"（存绝对值会让"窗口移动"污染分组键，并在缓存命中时拿到
            // 过期矩形——实测拖动窗口后 scissor 偏了一个位移量）。
            quads.cur_clip = clip_abs.map(|c| local_of(c, anchor_px));
            // **兜底剔除**：完全在裁剪层之外的命令**不镶嵌、不入段**。
            //
            // 主剔除在**分配处**（`Ui::culled` / `Response::culled`：控件自己直接 return
            // ——省的是它内部的全部命令）；这里兜住两类"分配处看不见的情况"：
            // ① 自绘装饰溢出到裁剪层外（控件本体可见、装饰不可见）；
            // ② 没检查 `culled` 的控件（含第三方）。
            // ⚠ **只剔"全外"**：**部分**重叠照整条画，越界像素交给 batch scissor。
            // ⚠ **空间**：`d.rect` 此刻已是**绝对**坐标（容器弹出时平移过），所以要与
            // **绝对**的 `clip_abs` 比——`quads.cur_clip` 是窗口**局部**的（缓存键用），
            // 拿它比会把内容整块误剔（实测：严格窗口内容全没了、子菜单面板不见了）。
            let cull_rect = match &d.kind {
                // 投影向外溢出本体：按**外沿**判可见性（否则贴边窗口的投影被误剔）。
                DrawKind::Shadow { blur, offset, .. } => {
                    let p = snap_rect(&d.rect);
                    Rect::new(
                        p.x - *blur + offset.x,
                        p.y - *blur + offset.y,
                        p.w + (*blur + offset.x.abs()) * 2.0,
                        p.h + (*blur + offset.y.abs()) * 2.0,
                    )
                }
                // ⚠ 文本的 `rect` 是**排版锚点矩形**（宽/高 = 容器盒），内容却随滚动平移
                // ⇒ 必须用**可见窗口**（软裁剪层，锚在盒子上）判，见 `text_visible_rect`。
                // 用 `rect` 会把"滚过头但明明可见"的整条文本剔掉（用户报的 BUG）。
                DrawKind::Text { clip, .. } => {
                    text_visible_rect(snap_rect(&d.rect), clip.map(|c| snap_rect(&c)))
                }
                _ => snap_rect(&d.rect),
            };
            if fully_outside(cull_rect, clip_abs) {
                if matches!(d.kind, DrawKind::Text { .. }) {
                    let acc = &mut self.state.frame_state.stats;
                    acc.culled_text = acc.culled_text.saturating_add(1);
                }
                continue;
            }
            match &d.kind {
                DrawKind::Solid(color) => {
                    if d.rect.w > 0.0 && d.rect.h > 0.0 {
                        let pr = snap_rect(&d.rect);
                        if pr.w > 0.0 && pr.h > 0.0 {
                            quads.push_white(win, local_of(pr, anchor_px), *color);
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                    }
                }
                DrawKind::RoundedRect { corners, radius } => {
                    let pr = snap_rect(&d.rect);
                    let local = local_of(pr, anchor_px);
                    if local.w > 0.0 && local.h > 0.0 {
                            // **无纹理、无着色器改动**：CPU 把圆角矩形镶嵌成三角形
                            // （硬体 + 1 物理像素羽化带，见 `crate::tess`）。
                            // 半径不做取整 / 9-patch clamp——镶嵌器接受任意半径并把
                            // 超出半高的半径 clamp 成胶囊；羽化带随控件尺寸自动收紧。
                            //
                            // 四角可各异 ⇒「圆角 + 渐变」自然成立。裁剪时按四角在
                            // **原矩形**中的相对位置重采样，保证渐变锚定不被裁剪平移。
                            let grad = Gradient::corners(
                                corners[0], corners[1], corners[2], corners[3],
                            );
                            let c = resample_gradient_local(grad, local, pr, anchor_px);
                            let table = self.state.tess.table();
                            // 采样 UV 由 `push_rounded` 填成白纹理 region 中心
                            // （写错 `(0,0)` 会静默采到字形页左上角的字形像素）。
                            quads.push_rounded(win, &table, local, *radius, self.theme.feather, c);
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                }
                DrawKind::Shadow { color, blur, offset, radius } => {
                    // **顶点色软阴影**（无纹理 / 无着色器 / 不增 draw call）。
                    // `rect` = 本体矩形（内轮廓恒在本体边缘，无"等浓度平台"）。
                    let pr = snap_rect(&d.rect);
                    // ⚠ **不再需要"部分可见就整块跳过"**：环境裁剪由 scissor 在像素级执行，
                    // 几何不会被形变（旧实现切割矩形会让投影的内轮廓错位成一条暗带，
                    // 于是只能在"被裁掉一部分"时整块放弃）。被裁的部分由 scissor 裁掉即可。
                    let local = local_of(pr, anchor_px);
                    if local.w > 0.0 && local.h > 0.0 && *blur > 0.0 {
                        let table = self.state.tess.table();
                        quads.push_rounded_shadow(
                            win, &table, local, *radius, *blur, *offset, *color,
                        );
                    }
                }
                DrawKind::Rect(gradient) => {
                    let pr = snap_rect(&d.rect);
                    let local = local_of(pr, anchor_px);
                    if local.w > 0.0 && local.h > 0.0 {
                            // **无纹理**：四角颜色直接进顶点色（光栅化器双线性插值）。
                            // 四角色按它在**原矩形**中的相对位置采样，保证渐变锚定不变。
                            let c = resample_gradient_local(*gradient, local, pr, anchor_px);
                            quads.push_white_quad(win, local, c);
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                }
                DrawKind::Border { color, width, radius } => {
                    let pr = snap_rect(&d.rect);
                    if pr.w > 0.0 && pr.h > 0.0 {
                        let local = local_of(pr, anchor_px);
                        if !radius.is_zero() {
                            // 圆角环带：只画一次边界，圆角处不会像"外圈实心 + 内圈实心"
                            // 那样把抗锯齿边缘混合两次。
                            //
                            // ⚠ **不要**因为"矩形被裁剪过"就退回直角四边条：窗口被拖到
                            // 视口边缘（或父裁剪区内侧）时被裁掉一部分，退回直角会让
                            // **整个窗口的边框瞬间变方**（"拖动变方"）。裁剪后的矩形交给
                            // 环带自己处理即可——`push_rounded_ring` 内部会
                            // `CornerRadius::fit(w, h)` 把半径夹到放得下，内轮廓塌缩时
                            // 也会退化成一块实心圆角矩形。
                            let table = self.state.tess.table();
                            quads.push_rounded_ring(
                                win,
                                &table,
                                local,
                                *radius,
                                *width,
                                self.theme.feather,
                                *color,
                            );
                        } else {
                            for br in border_rects(&local, (*width).round()) {
                                if br.w > 0.0 && br.h > 0.0 {
                                    quads.push_white(win, br, *color);
                                }
                            }
                        }
                        debug_layout_outline(quads, win, anchor_px, pr, dbg);
                    }
                }
                DrawKind::Icon { icon, color } => {
                    // 矢量图标：单位方框内的凸分片映射到 `rect`（窗口局部），
                    // 并按 `Theme::feather` 做边缘羽化——与圆角矩形同一套 AA 机制。
                    let pr = snap_rect(&d.rect);
                    let local = local_of(pr, anchor_px);
                    if local.w > 0.0 && local.h > 0.0 {
                            // **等比**：图标分片画在 `[0,1]²` 的方形域里，把 `rect` 直接映射过去
                            // 会在非方形框里被拉扁（`row` 内 `force_h_all` 就会把 18×26 的框
                            // 交给这里）。故取 `min(w,h)` 的**居中方块**——图标永不形变。
                            quads.push_icon(
                                win,
                                centered_square(local),
                                *icon,
                                *color,
                                self.theme.feather,
                            );
                        }
                }
                DrawKind::Text {
                    text,
                    size,
                    color,
                    align,
                    valign,
                    family,
                    clip,
                    buf,
                    ramp,
                } => {
                    // 外层裁剪（绝对逻辑）→ 相对文本块左上角（与 DrawKind::Text::clip 同空间），
                    // 与命令自带裁剪求交后传给 draw_text_quads。
                    let merged = match d.clip.map(|c| {
                        Rect::new(c.x - d.rect.x, c.y - d.rect.y, c.w, c.h)
                    }) {
                        Some(outer) => match clip {
                            Some(inner) => intersect_rect(&outer, inner),
                            None => Some(outer),
                        },
                        None => *clip,
                    };
                    self.draw_text_quads(
                        quads,
                        win,
                        anchor_px,
                        &d.rect,
                        text,
                        *size,
                        *color,
                        *align,
                        *valign,
                        family.as_deref(),
                        merged,
                        buf.as_ref().map(Arc::clone),
                        *ramp,
                    );
                    let pr = snap_rect(&d.rect);
                    debug_layout_outline(quads, win, anchor_px, pr, dbg);
                }
                DrawKind::Image(bg) => {
                    // 背景图：与实心背景同一条镶嵌路径（CPU 直出三角形 + 羽化），
                    // 只多一个"逐顶点 UV 的仿射映射"（见 `QuadCollector::push_image`）。
                    let pr = snap_rect(&d.rect);
                    let local = local_of(pr, anchor_px);
                    if local.w > 0.0 && local.h > 0.0 {
                            let table = self.state.tess.table();
                            quads.push_image(win, &table, local, *bg, self.theme.feather);
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                }
                DrawKind::Caret { color, width } => {
                    let r = Rect::new(d.rect.x, d.rect.y, *width, d.rect.h);
                    let pr = snap_rect(&r);
                    if pr.w > 0.0 && pr.h > 0.0 {
                            quads.push_white(win, local_of(pr, anchor_px), *color);
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                }
                DrawKind::Debug { color, shape } => {
                    // 屏幕空间调试图元：逻辑像素 → 物理像素线段，转窗口局部写入 debug 叠加。
                    for ([a, b], w) in debug_shape_segments(shape) {
                        quads.push_debug_line(win, a - anchor_px, b - anchor_px, w, *color);
                    }
                }
            }
        }
    }

    /// 窗口内容签名：命令的 `(kind, rect, color, 文本…)` 哈希（窗口顶点缓存 key 用）。
    /// 忽略 `win/seq`（窗口内固定）；任何影响渲染的内容变化都会改变签名。
    /// 
    /// **包含文本缓存版本号**：`TEXT_LINE_HEIGHT_VERSION` 变化时，窗口缓存自动失效，
    /// 避免新旧行高混用导致布局错乱。
    ///
    /// ⚠ **必须覆盖一切渲染相关字段**（颜色 / 边框宽 / 圆角 / 对齐 / 光标 / 选择 /
    /// 文本内容）——曾用"轻量摘要"跳过它，漏掉颜色位导致 hover/click 变色时
    /// 缓存不失效、窗口内交互效果不刷新（见 [`crate::state::UiState::window_quads`] 文档）。
    pub(super) fn cmd_sig(&self, h: &mut std::collections::hash_map::DefaultHasher, d: &UiDraw, anchor: Vec2) {
        cmd_sig_hash(h, d, anchor);
    }

    /// 窗口按下裁决：本帧若有窗口被按下（重叠区域点击），**只保留最上层窗口**
    /// 的拖拽（其余取消，修复"重叠时同时拖动两个窗口"），且仅最上层窗口置顶。
    pub(super) fn resolve_win_press(&mut self) {
        let Some((top_id, old_z)) = self.win_press_top.clone() else {
            return;
        };
        // 只保留最高 z 命中窗口的拖拽：按下新窗口时停止**其它窗口**（含本帧未按下
        // 的旧窗口）的拖拽。⚠ 只清**窗口 id**（`win_ids`）的拖拽状态——不能碰控件
        // 自身的拖拽（滑块 / 滚动条 / 窗口缩放柄等），否则窗口内控件的拖拽会被
        // finish 误清（Resize 手柄按下后下一帧即失效）。
        for (wid, ws) in self.state.widgets.iter_mut() {
            if wid != &top_id
                && self.win_ids.values().any(|i| i == wid)
                && ws.dragging
                && !ws.pressed
            {
                ws.dragging = false;
            }
        }
        // 仅最上层命中窗口置顶（z+1；本帧命令仍按旧 z，下一帧生效）。
        // ⚠ 排除置顶哨兵（WIN_TOPMOST）并 saturating：浮层恒顶，真实窗口 z 不会
        // 递增碰撞到哨兵。
        let max_z = self
            .state
            .window_z
            .values()
            .copied()
            .filter(|&z| z < WIN_TOPMOST)
            .max()
            .unwrap_or(0);
        let new_z = max_z.saturating_add(1);
        self.state.window_z.insert(top_id.clone(), new_z);
        // **焦点归属清理**：焦点控件若在**其他窗口**（本次置顶的窗口之外）——
        // 清除焦点。否则旧输入框在窗口被盖住后仍持焦点（点击被遮挡无法再聚焦、
        // 打字落入不可见输入框），表现为"使用其他窗口后文本框失效"。
        // ⚠ 与**置顶前**的 z（`old_z`，win_press_top 记录点击时的窗口 z）比较——
        // 焦点条目本帧以旧 z 录制；拿置顶后的 `new_z` 比会恒不相等 → 点击输入框
        // （其所在窗口同时置顶）焦点被立即清除，窗口内文本框"无法使用"。
        if let Some(fid) = &self.state.focused {
            let fwin = self.focusables.iter().find(|e| e.id == *fid).map(|e| e.win);
            if fwin.is_some_and(|w| w != 0 && w != old_z) {
                self.state.focused = None;
            }
        }
        // 诊断：记录本次按下由哪个窗口接收（重叠点击时"赢家"）。
        self.state.last_press_window = Some((top_id, new_z));
    }

    /// **键盘导航**（`finish` 末尾调用）：
    ///
    /// - **Tab / Shift+Tab / 方向键**：按 `(win, 注册序)` 排序的焦点链遍历
    ///   （[`focus_step`]），更新 `UiState.focused`；焦点控件本帧未录制时自动清除；
    /// - **Esc**：优先收起展开的下拉框，否则取消焦点；
    /// - **焦点描边**：对当前焦点控件画一圈描边（[`crate::style::FocusStyle`]，
    ///   `Theme::focus`；elem 取全局最大 → 画在窗口内容之上，裁剪沿用控件自身）。
    pub(super) fn handle_focus_keys(&mut self) {
        // 链排序：按 (win, 注册序) 稳定排序（非窗口 0 在前，窗口按 z 从下到上）。
        let mut chain: Vec<&FocusEntry> = self.focusables.iter().collect();
        chain.sort_by_key(|e| e.win);
        // 焦点控件本帧未录制（所在窗口关闭 / 控件移除）→ 清除焦点。
        if let Some(fid) = &self.state.focused
            && !chain.iter().any(|e| e.id == *fid) {
                self.state.focused = None;
            }
        // 移动：Tab（+1）/ Shift+Tab（-1）/ Down（+1）/ Up（-1）。
        // ⚠ IME 组合中（preedit 非空或上帧在组合）**禁止方向键/Tab 移动焦点**——
        // 中文输入法用 ↑/↓ 切换候选、Enter 上屏，焦点被移走会立刻打断输入（文本框"失效"）。
        let composing = self
            .keyboard
            .ime_preedit()
            .is_some_and(|p| !p.is_empty())
            || self.state.ime_composing;
        let shift = self.keyboard.key(KeyCode::ShiftLeft).pressed()
            || self.keyboard.key(KeyCode::ShiftRight).pressed();
        // **文本输入框持有焦点时，↑/↓ 由输入框自身处理**（多行跨视觉行移动光标、
        // 单行无操作）——全局焦点遍历只接管 Tab / Shift+Tab；否则按 ↑/↓ 会把焦点
        // 跳到别的控件（多行文本框内"上下键跳走"的 bug）。
        let focus_is_text = chain
            .iter()
            .find(|e| {
                self.state
                    .focused
                    .as_ref()
                    .is_some_and(|f| f.as_str() == e.id.as_str())
            })
            .is_some_and(|e| e.kind == FocusKind::TextInput);
        let dir: i32 = if composing {
            0
        } else if self.keyboard.key(KeyCode::Tab).down_edge() {
            if shift { -1 } else { 1 }
        } else if self.keyboard.key(KeyCode::ArrowDown).down_edge() && !focus_is_text {
            1
        } else if self.keyboard.key(KeyCode::ArrowUp).down_edge() && !focus_is_text {
            -1
        } else {
            0
        };
        if dir != 0 {
            let next = focus_step(&chain, self.state.focused.as_ref(), dir);
            // 同步焦点控件的**类型**（文本焦点判定用）：查链拿到该控件的 `FocusKind`。
            self.state.focused_kind = next
                .as_ref()
                .and_then(|id| chain.iter().find(|e| e.id == *id).map(|e| e.kind));
            self.state.focused = next;
        }
        // Esc：优先收起下拉框，否则取消焦点。
        if self.keyboard.key(KeyCode::Escape).down_edge() {
            if self.state.combo_open.is_some() {
                self.state.combo_open = None;
            } else if self.state.focused.is_some() {
                self.state.focused = None;
                self.state.focused_kind = None;
            }
        }
        // 焦点描边：对当前焦点控件画一圈 Border。
        // ⚠ 走 **`debug_queue`**（`submit_debug` 路径：不进窗口顶点缓存、恒覆盖在最上）：
        // 一帧多段后帧收尾视图的 `seq` 从 0 重开，若沿用"`elem = seq + 1` 最大 ⇒ 画在
        // 窗口内容之上"的老写法，描边会被压到窗口内容下面（`elem` 比各段的都小）。
        if let Some(fid) = &self.state.focused {
            let Some(entry) = chain.iter().find(|e| e.id == *fid) else {
                return;
            };
            // 先拷贝字段，结束对 chain 的借用（随后需要 &mut self）。
            let (win, depth, rect, clip) = (entry.win, entry.depth, entry.rect, entry.clip);
            let focus = self.theme.focus.clone();
            let elem = self.painter.q.seq + 1;
            let seq = self.next_seq();
            self.painter.q.debug_queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect,
                clip,
                full_w: false,
                kind: DrawKind::Border {
                    color: focus.color,
                    width: focus.width,
                    radius: CornerRadius::default(),
                },
            });
        }
    }

    /// 文本 → 字形四边形（收集到 `quads`；按字形图集页纹理分组）。
    ///
    /// **精确裁切**（"半消失"）：字形与裁剪区求交，相交部分生成裁剪后的四边形
    /// （UV 按比例同步缩放）——字形在裁剪线处被**部分绘制**，而非整字形保留/消失
    /// （`rjw_text` 的 `cull` 只做整字形剔除，像素级裁剪在此完成）。
    #[allow(clippy::too_many_arguments)]
    fn draw_text_quads(
        &mut self,
        quads: &mut QuadCollector,
        win: u32,
        anchor_px: Vec2,
        rect: &Rect,
        text: &str,
        size: f32,
        color: Color,
        align: TextAlign,
        valign: TextVAlign,
        family: Option<&str>,
        clip: Option<Rect>,
        buf: Option<Arc<Buffer>>,
        ramp: Option<TextRamp>,
    ) {
        if rect.w <= 0.0 || rect.h <= 0.0 || text.is_empty() {
            return;
        }
        // 全部换算物理像素（锚点 / 裁剪区），与屏幕固定变换 1:1 匹配；
        // 排版缓冲：控件自持（输入框）或按需缓存（静态标签）。
        let pr = snap_rect(rect);
        // 矩形锚点（**整数像素**，逐项取整，无小数参与加法）：
        // 水平：左 = 左缘、中 = 左缘 + 半宽取整、右 = 右缘；
        // 垂直：Center = 上缘 + 半高取整，Top = 上缘（TextArea 多行顶对齐）。
        let anchor = Vec2::new(
            match align {
                TextAlign::Left => pr.x,
                TextAlign::Center => pr.x + (pr.w * 0.5).round(),
                TextAlign::Right => pr.x + pr.w,
            },
            match valign {
                TextVAlign::Top => pr.y,
                TextVAlign::Center => pr.y + (pr.h * 0.5).round(),
            },
        );
        // 局部化基准 = **窗口原点**（anchor_px），不是文字锚点：
        // 字形世界坐标 - 窗口原点世界 = 相对窗口原点的局部顶点，
        // 提交时经窗口 transform（screen_fixed_tf(窗口原点)）映射回世界。
        let win_anchor_world = anchor_px;
        let buf = match buf {
            Some(b) => b,
            None => self.cache_buffer(text, size, family),
        };
        // 排版几何（UI 稳定集成面）：内容尺寸 / 首行行盒顶 / 图集页尺寸。
        // 取代旧 `Text::render_from(&buf)` + `tr.content_size()/lines()/page_size()`。
        let geo = self.text.geometry(&buf);
        // 垂直定位按**行盒**（行高），而非字形墨迹内容：
        // 以首行行顶（相对文本视觉原点）为参考，使行盒中心对准锚点（矩形垂直中心）。
        // 字形在行盒内按基线排布——矮小写字母（如 "a"，无 descender）落在基线上，
        // 不再因"以墨迹顶为参考"（旧实现把 `[视觉原点, 视觉原点+行高]` 当块居中，
        // 行盒整体上移约一个 top-bearing）而浮在行盒上部偏上显示。
        let content = geo.content_size;
        // `first_line_top` 为**整数**（rjw_text 收集期已对行顶取整）。
        let first_line_top = geo.first_line_top;
        // **整数加法不变量**：`block_tl = anchor + off` 的两侧均为整数——
        // 锚点（上方逐项取整）、`text_block_offset`（content / first_line_top 均为
        // 整数，内部只有整数加减与边界 `round`）。0.5px 小数（居中奇数宽 / 行盒偏移）
        // 在 `round` 边界被一次性消化，不会流入加法链 → 无误差累加；
        // 字形四边形角点 = block_tl + 整数字形偏移 = 整数屏幕像素 → 采样精确落在
        // 纹素中心，消除 1:1 图集双线性采样的亚像素模糊。
        let block_tl = anchor + text_block_offset(align, valign, content, first_line_top);
        let tf = screen_fixed_tf(block_tl);
        // 裁剪区：相对命令矩形（`merged`，逻辑）→ **窗口局部**。
        // ⚠ 相对基准是命令矩形物理 `pr`，**不是**文本块原点 `block_tl`：
        // `merged` 相对 `d.rect`，而字形窗口局部坐标以 `anchor_px` 为原点——
        // clip 窗口局部 = merged×scale + (pr - anchor_px)。用 block_tl 会整体错位
        // (block_tl - pr)（Top 对齐 / 多行时垂直偏差数像素）→ 滚动后文本不消失/错位。
        let clip_local: Option<Rect> = clip.map(|c| {
            let pc = snap_rect(&c);
            Rect::new(
                pc.x + pr.x - anchor_px.x,
                pc.y + pr.y - anchor_px.y,
                pc.w,
                pc.h,
            )
        });
        // 像素级裁剪完全由下方逐字形求交完成（完全在外 → 剔除；部分相交 → "半消失"）。
        // 不再使用 rjw_text 的 clip/cull（其坐标系相对字形 top_left，语义易错位）。
        let page_size = geo.page_size;
        let inv_page = 1.0 / page_size;
        let ca: [f32; 4] = color.into();
        // ── 首末两色渐变（[`TextRamp`]）的采样域 ─────────────────────────────
        // 三条口径，改这段前先读（每一条都对应一个真实缺陷）：
        //
        // ① **坐标系 = 文本视觉原点系**（与 `rjw_text` 的字形 `top_left` 同系，也即
        //    `geo.first_line_top` / `content` 所在的系）。**不要**用窗口局部或绝对坐标：
        //    曾经的实现拿 `block_tl + pr - anchor_px` 当原点，而 `block_tl` **本身就是绝对
        //    坐标**（命令在容器弹出时已 `translate` 成绝对）⇒ 多加了一个 `pr` ⇒ 渐变随控件
        //    在窗口里的位置整体平移；标签靠右 / 靠下时偏移可达数百像素、被 `clamp` 成纯
        //    `from` 色（现象："渐变不是一成不变的，某些情况下会变化"——纵向最明显）。
        // ② **采样点用未裁剪的字形几何**：取"顶点在字形内的归一化位置"（`(qx-gx)/gw`），再
        //    还原成局部坐标 ⇒ **裁剪不改变颜色**（只少画一部分顶点）。用裁剪后的角点直接
        //    取色会让同一字形在被裁前后颜色不同（滚动 / 裁切时整条渐变跟着变）。
        // ③ **域由 `mode` 决定**：Glyph = 该字形自身、Line = 该行、Frame = 整块（"Text 域"）。
        //    Line 需要每行的沿轴范围；Frame 用 `content`（首行顶 → 内容底）。
        let frame_lo = match ramp.map(|r| r.axis) {
            Some(rjw_text::GradientAxis::Vertical) => geo.first_line_top,
            _ => 0.0,
        };
        let frame_span = match ramp.map(|r| r.axis) {
            Some(rjw_text::GradientAxis::Vertical) => content.y - geo.first_line_top,
            _ => content.x,
        };
        let line_bounds = ramp_line_bounds(ramp, &buf, content.y);
        // 逐顶点取色：`None` = 整段单色（与旧行为逐位相同）。
        // `local` = 该顶点沿渐变轴在**文本视觉原点系**里的坐标。
        let tint_at = |local: f32, glyph: (f32, f32), line: usize| -> [f32; 4] {
            let Some(r) = ramp else { return ca };
            let (lo, hi) = match r.mode {
                rjw_text::GradientMode::Glyph => glyph,
                rjw_text::GradientMode::Line => {
                    line_bounds.get(line).copied().unwrap_or(glyph)
                }
                rjw_text::GradientMode::Frame => (frame_lo, frame_lo + frame_span),
            };
            r.color_at(r.t_in(lo, hi, local), color).into()
        };
        ramp_trace(ramp, frame_lo, frame_span, line_bounds.len());
        // 唯一文本链：`label_from`（复用已排版缓冲，不重新整形）→ `transform(tf)` → 逐字形回调。
        // 旧写法 `render_from(&buf).origin(ZERO).transform(tf).color(color).draw_with(..)`
        // 在此机械适配（`origin(ZERO)` = 默认定位，`color` 由下方顶点色统一施加）。
        self.text.label_from(&buf).transform(tf).draw_with(|g| {
            let region = g.region();
            let transform = g.transform();
            // 字形精灵（轴对齐四边形）：四角经 transform 到世界坐标，再转窗口局部 + 图集 UV
            let w = region.wh_px.0 as f32;
            let h = region.wh_px.1 as f32;
            let tl = transform.transform_point(Vec2::new(0.0, 0.0)) - win_anchor_world;
            let tr_p = transform.transform_point(Vec2::new(w, 0.0)) - win_anchor_world;
            let bl = transform.transform_point(Vec2::new(0.0, h)) - win_anchor_world;
            let uv_tl = Vec2::new(
                region.tl_px.0 as f32 * inv_page,
                region.tl_px.1 as f32 * inv_page,
            );
            let uv_wh = Vec2::new(w * inv_page, h * inv_page);
            // 字形窗口局部 AABB（轴对齐；屏幕固定变换下无旋转）。
            let gx = tl.x;
            let gy = tl.y;
            let gw = tr_p.x - tl.x;
            let gh = bl.y - tl.y;
            let (qx0, qy0, qx1, qy1) = match &clip_local {
                Some(c) => {
                    // 与裁剪区求交：无交集 → 整字形剔除（"完全消失"）；
                    // 部分相交 → 生成裁剪后四边形（"半消失"，UV 按比例缩放）。
                    let ix0 = gx.max(c.x);
                    let iy0 = gy.max(c.y);
                    let ix1 = (gx + gw).min(c.x + c.w);
                    let iy1 = (gy + gh).min(c.y + c.h);
                    if ix1 <= ix0 || iy1 <= iy0 {
                        return;
                    }
                    (ix0, iy0, ix1, iy1)
                }
                None => (gx, gy, gx + gw, gy + gh),
            };
            let nw = qx1 - qx0;
            let nh = qy1 - qy0;
            let u0 = uv_tl.x + (qx0 - gx) / gw * uv_wh.x;
            let v0 = uv_tl.y + (qy0 - gy) / gh * uv_wh.y;
            let u1 = u0 + nw / gw * uv_wh.x;
            let v1 = v0 + nh / gh * uv_wh.y;
            // 顶点在**字形内**的归一化位置（0..1；由未裁剪的 `gx/gw` 与裁剪后角点给出）——
            // 裁剪只是取这段区间的子集，**不改变**取色（口径 ②）。
            let safe = |d: f32| d.abs() > f32::EPSILON;
            let fx0 = if safe(gw) { (qx0 - gx) / gw } else { 0.0 };
            let fx1 = if safe(gw) { (qx1 - gx) / gw } else { 1.0 };
            let fy0 = if safe(gh) { (qy0 - gy) / gh } else { 0.0 };
            let fy1 = if safe(gh) { (qy1 - gy) / gh } else { 1.0 };
            // 字形在**文本视觉原点系**里的沿轴范围（口径 ①③）+ 顶点在该系里的沿轴坐标。
            let (g_tl, g_sz) = (g.top_left(), g.size());
            let line_idx = g.line_index();
            let vertical = ramp.map(|r| r.axis) == Some(rjw_text::GradientAxis::Vertical);
            let (glyph_lo, glyph_hi) = if vertical {
                (g_tl.y, g_tl.y + g_sz.y)
            } else {
                (g_tl.x, g_tl.x + g_sz.x)
            };
            let local_at = |fx: f32, fy: f32| if vertical { g_tl.y + fy * g_sz.y } else { g_tl.x + fx * g_sz.x };
            // 四角**各自**取渐变色（字形内部也有梯度）。
            let quad = [
                vertex_p3u2c4(
                    Vec2::new(qx0, qy0),
                    [u0, v0],
                    tint_at(local_at(fx0, fy0), (glyph_lo, glyph_hi), line_idx),
                ),
                vertex_p3u2c4(
                    Vec2::new(qx1, qy0),
                    [u1, v0],
                    tint_at(local_at(fx1, fy0), (glyph_lo, glyph_hi), line_idx),
                ),
                vertex_p3u2c4(
                    Vec2::new(qx0, qy1),
                    [u0, v1],
                    tint_at(local_at(fx0, fy1), (glyph_lo, glyph_hi), line_idx),
                ),
                vertex_p3u2c4(
                    Vec2::new(qx1, qy1),
                    [u1, v1],
                    tint_at(local_at(fx1, fy1), (glyph_lo, glyph_hi), line_idx),
                ),
            ];
            quads.push_tex_quad(win, region.page_uid, quad);
        });
    }
}


/// **逐行渐变（`GradientMode::Line`）的行范围**（沿渐变轴，**文本视觉原点系**）。
///
/// `mode != Line` ⇒ 空（调用方回落到字形 / 整块域）。用 [`rjw_text::Text::lines`] 的
/// **排版元数据**（不遍历字形）：行顶取 `top`、行底取**下一行的顶**、末行用内容底补齐；
/// 横向取 `0..行宽`（UI 恒左对齐 ⇒ 行左缘 = 0）。
#[inline]
fn ramp_line_bounds(ramp: Option<TextRamp>, buf: &Buffer, content_h: f32) -> Vec<(f32, f32)> {
    let Some(r) = ramp else { return Vec::new() };
    if r.mode != rjw_text::GradientMode::Line {
        return Vec::new();
    }
    let lines = rjw_text::Text::lines(buf);
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(lines.len());
    for (i, l) in lines.iter().enumerate() {
        out.push(match r.axis {
            rjw_text::GradientAxis::Horizontal => (0.0, l.width),
            rjw_text::GradientAxis::Vertical => {
                let bottom = lines.get(i + 1).map(|n| n.top).unwrap_or(content_h);
                (l.top, bottom)
            }
        });
    }
    out
}

/// `RJ_RAMP_TRACE=1`：打印文本渐变的**域**（口径 ①③ 的回归证据）。
///
/// 为什么留：坐标系错一次就是"渐变随控件位置漂移 / 被 clamp 成单色"（用户报过的现象），
/// 域的数值（`frame` 与 `content` **同系**、`line` 数与内容底）能一眼看出是否自洽。
fn ramp_trace(ramp: Option<TextRamp>, frame_lo: f32, frame_span: f32, lines: usize) {
    let Some(r) = ramp else { return };
    if std::env::var_os("RJ_RAMP_TRACE").is_none() {
        return;
    }
    eprintln!(
        "ramp_trace: mode={:?} axis={:?} frame={frame_lo:.1}..{:.1} lines={lines}",
        r.mode,
        r.axis,
        frame_lo + frame_span
    );
}

/// 拖拽激活所需的最小**物理像素**位移：按下后鼠标位移 ≥ 此值才视为“拖拽”。///
/// 纯点击（无位移）不激活拖拽 → 窗口 / 可拖拽面板内的子控件（按钮 / 勾选框 /
/// 输入框等）**正常响应点击**；真正拖动中才抑制子控件交互（防止误触）。
const DRAG_ACTIVATE_PX: f32 = 3.0;

/// **几何缓存签名** = 命令内容哈希 ⊕ 字形图集区域失效世代号
/// （[`rjw_text::Text::atlas_revision`](rjw_text::Text::atlas_revision)）。
///
/// 窗口 / win=0 子槽的顶点缓存烘的是**最终 UV**（字形 + WHITE 基础纹理都取自字形
/// 图集）。图集一旦**重排**（搬动区域）或**复用已逐出条目的槽位**，旧 UV 就指向
/// 别的像素（"陈旧文字" / "背景消失"），而命令内容没变 ⇒ 只靠命令哈希**永远不失效**。
/// 故把世代号并入缓存键：图集一变，全部几何缓存自动重建（重建期会重新解析字形区域）。
#[inline]
pub(super) fn geom_cache_sig(cmd_hash: u64, atlas_revision: u64) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    h.write_u64(cmd_hash);
    h.write_u64(atlas_revision);
    h.finish()
}

/// 是否已产生足以激活拖拽的位移（`current_px` / `press_px` 均为**物理像素**，
/// 已取整；`None` = 无按下基准，未激活）。
#[inline]
pub(super) fn drag_moved(current_px: Vec2, press_px: Option<Vec2>) -> bool {
    match press_px {
        Some(p) => {
            (current_px - p).length_squared() >= DRAG_ACTIVATE_PX * DRAG_ACTIVATE_PX
        }
        None => false,
    }
}

/// **窗口 / 可拖拽面板的位置求解**（纯函数，`window_impl` / `panel_impl` 共用）。
///
/// - `origin`：责任链（脚本 → 拖拽状态 → 传入 pos）解析出的本帧基准位置；
/// - `mouse_screen`：本帧鼠标物理坐标；
/// - `hit`：鼠标是否在本体（由**上一帧结算尺寸**构造的矩形判定，见调用方）。
///
/// 语义：
/// 1. 按下帧（`down_edge && hit`）**先无条件**建立拖拽基准
///    `(press_panel = origin, press_mouse = 本帧鼠标)`；内容录制后若发现子控件
///    声明了本次按下（`press_claimed`），调用方须 [`clear_drag_base`] 清除
///    ——这样"从输入框上拖拽 = 选择文本"与"窗口从空白处拖动"两者都对，且
///    基准判定不依赖内容录制（`display_pos` 得以在录制**前**求出）。
/// 2. `active` = 拖动标记（[`crate::hit::update_drag`]）+ **有基准** +
///    位移 ≥ [`DRAG_ACTIVATE_PX`]（纯点击不拖拽）。
/// 3. 位置**只由按下帧基准 + 当前鼠标位移决定**，与上一帧位置无关——这是
///    `abs_base` 能与几何（`translate(pos)`）当帧一致、拖拽不落后一帧的前提。
pub(super) fn resolve_drag(
    ws: &mut WidgetState,
    hit: bool,
    btn: KeyState,
    mouse_screen: Vec2,
    origin: Vec2,
) -> (bool, Vec2) {
    let dragging = update_drag(ws, hit, btn);
    if btn.down_edge() && hit {
        // 物理像素粒度基准（取整消除鼠标静止噪声；DPI 1.5 下也不会"移动 1.5px 才动"）。
        ws.press_panel = Some(origin);
        ws.press_mouse = Some(mouse_screen.round());
    }
    let active =
        dragging && ws.press_mouse.is_some() && drag_moved(mouse_screen.round(), ws.press_mouse);
    let pos = if active {
        let pp = ws.press_panel.unwrap_or(origin);
        let pm = ws.press_mouse.unwrap_or(mouse_screen);
        // 物理像素增量（round：对噪声滞回，静止时不变）→ 物理位移
        pp + (mouse_screen - pm).round()
    } else {
        origin
    };
    (active, pos)
}

/// **清除拖拽基准**：本帧按下被窗口 / 面板内的子控件声明（`press_claimed`——
/// 输入框选择、滑块调值、滚动条拖拽），窗口 / 面板**不得**跟随移动。
pub(super) fn clear_drag_base(ws: &mut WidgetState) {
    ws.press_panel = None;
    ws.press_mouse = None;
    ws.press_size = None;
}

