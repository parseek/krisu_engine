//! 段 / 帧提交管线：`finish`（段收尾）与 `end_frame`（帧收尾）、提交单元的收集与
//! 计划缓存、批次冲刷、Debug 叠加提交、光标定夺与帧复位、缓存签名辅助。
//!
//! 维护者笔记：提交序 = `(win, place, elem, group, tex, clip)`（`place` 见 §5.6）；
//! 缓存粒度是**提交计划**而非几何碎片（§6.2）。改这里的顺序键会直接表现为闪烁而不是
//! 报错——先用 `RJ_ORDER_TRACE` / `RJ_CACHE_TRACE` 定位。

use super::cmds::geom_cache_sig;
use super::*;
use std::time::Instant;

use glam::Vec2;
use rjw_color::Color;
use rjw_transform::{Rect, Transform2D};

use crate::backend::{UiBackend, UiBatch, UiBatchSource};
use crate::draw::{screen_fixed_tf, UiDraw};
// 顶点收集 / 合批机制（原在 `ui.rs`，见 `gpu_batch` 模块文档）。
use crate::gpu_batch::{
    clip_rect, segment_runs, BatchPlan, CacheSlot, CacheStats, CachedQuad, Geom, QuadCollector,
    SubmitUnit,
};
use crate::hit::{clear_frame_flags, window_occluded};

impl<'a> Ui<'a> {
    /// 排序并提交全部绘制命令；随后清空帧状态（下次 `begin` 复用）。
    ///
    /// **命令排序键**：`(win, depth, elem, kind_group, seq)`
    /// - `win`：窗口 z 序（焦点窗口靠后 → 最上层）；
    /// - `depth`：容器嵌套深度；
    /// - `elem`：**元素序**（控件开始录制时的序号）——元素间按录制顺序，
    ///   重叠时后录元素覆盖先录元素（层级正确）；
    /// - `kind_group`：**元素内**"背景/图形（0）先于文字（1）"——文字不被
    ///   自身图形覆盖（[`crate::draw::DrawKind::group`]）；
    /// - `seq`：同元素内同类命令保持录制顺序。
    ///
    /// **提交方式**（不使用 Sprite）：全部图元（背景 / 圆角 / 渐变 / 控件背景 / 文字）
    /// 转为**顶点 + 三角形索引**（圆角由 `crate::tess` 镶嵌），按 `(窗口, 纹理, 变换)`
    /// 分组后**由 UI 自行决定提交顺序**（不依赖 Render2D 排序）：
    /// `(win 升序, 白纹理图形组 → 字形文字组, 纹理 uid)`——
    /// 1. 非窗口内容（`win=0`）最底，窗口按 z 从下到上（`layer = base + z`）；
    /// 2. 同一窗口内**"背景/图形 → 文字"严格成立**（白纹理组先于字形图集组），
    ///    跨帧稳定、与 Render2D 任意排序模式结果一致。
    ///
    /// **UI 的 Render2D 必须关闭排序**（`sort(SortMode::None)`，完全按提交顺序绘制）：
    /// UI 自行管理绘制顺序，排序键 `(win, depth, elem, group, seq)` 依赖**提交顺序**
    /// 生效（图形组在文字组之前提交）。⚠ `SortMode::LayerAndStates`
    /// 会在同一 layer 内按 `(rstates, texture_uid)` 重排——字形图集页先于程序化纹理页
    /// （圆角/渐变）注册，重排后**圆角/渐变图形会盖住文字**；`SortMode::LayerOnly`
    /// （稳定按 layer 排序）可接受（同层保持提交顺序）。
    /// 提交本帧 UI 到渲染器。**视口与渲染器在此延迟传入**（`begin` 时不需要）——
    /// 录制阶段可完全独立于绘制资源；`viewport`（屏幕矩形，见 [`rjw_transform::Rect`]）
    /// 提供屏幕固定变换，`r2d` 接收四边形。UI 不需要相机（恒为 identity：不旋转/缩放），
    /// 仅需视口矩形。
    /// **段收尾**：把本段录制的命令分桶 → 生成 / 复用顶点 → 提交到 `backend`
    /// （`rjw_ui` 只产出 `UiBatch`；渲染器由调用方适配）。**一帧可调用多次**（多段 UI），
    /// 帧级收尾在 [`Self::end_frame`]。
    ///
    /// 流水线（职责见各自文档注释）：
    /// 1. 命令分桶 + WHITE 纹理解析（本函数内联）；
    /// 2. 按窗口生成可提交顶点 / 顶点缓存（`cache_all_windows` → 子槽 `cache_z0_window`
    ///    / 窗口 `cache_window`；签名 `hash_cmds`）；
    /// 3. 排序 + 连续运行合批提交（`submit_quads` / `flush_seg`）；
    /// 4. Debug 叠加（`submit_debug`：debug_queue / 布局描边 / 焦点描边，恒覆盖在最上）；
    /// 5. 段统计累加进帧级暂存 + 帧级事实回存（`save_frame_state`）。
    pub fn finish(&mut self, backend: &mut dyn UiBackend) {
        let t_finish = Instant::now();
        // 提交序 = (窗口 z → 深度 → 元素序 → 元素内图形/文字 → 命令序)。**免全量排序**：
        // 命令按录制序（seq）生成，同深度内 `(elem, group, seq)` 天然有序（元素随录制
        // 递增、同元素"背景/图形"先于文字录制）；唯一乱序维度是 `depth`（容器嵌套
        // 进出）与 `win`（跨帧 z 缓存使录制序 ≠ z 序）→ 按 win 分桶 + 桶内 depth
        // 分桶、桶内保持录制序，与 `sort_by_key((win, depth, elem, group, seq))`
        // **完全等价**（O(n + 桶数)，免每帧 O(n log n)）。
        let t_sort = Instant::now();
        let cmd_count = self.painter.q.queue.len() as u32;
        let queue = std::mem::take(&mut self.painter.q.queue);
        // **WHITE 基础纹理优先取字形图集页**（`Text::white_region`，1×1 clamp_margin）：
        // 实心填充（Solid / 边框 / 光标）与字形**同页同纹理** → 同窗口内"图形组 → 文字组"
        // 相邻且同纹理，后端的连续段合批合成单次 draw call，省去图形↔文字的纹理状态切换。
        // 兜底：`rjw_text` 不可用时用当前帧第一个命令引用的纹理（整纹理 UV）。
        let (white_uid, white_uv_tl, white_uv_wh) = match self.text.white_region() {
            Some(r) => {
                let inv = match backend.texture(r.page_uid) {
                    Some(t) => 1.0 / t.width as f32,
                    None => 1.0,
                };
                let tl = Vec2::new(r.tl_px.0 as f32, r.tl_px.1 as f32) * inv;
                let wh = Vec2::new(r.wh_px.0 as f32, r.wh_px.1 as f32) * inv;
                (r.page_uid, tl, wh)
            }
            None => (0, Vec2::ZERO, Vec2::ONE),
        };
        // 按窗口分组 + 组内 depth 分桶（桶内保持录制序）：非窗口（win=0）每帧重建；
        // 窗口按**内容签名**缓存局部顶点，内容不变时复用（移动窗口只改变换，顶点不重建）。
        let mut groups = bucket_cmds(queue);
        let mut wins: Vec<u32> = groups.keys().copied().collect();
        wins.sort_unstable();
        let sort_us = t_sort.elapsed().as_secs_f64() * 1e6;
        // —— 计时累加器（本帧各阶段 µs 统计，finish 末尾写入 UiState.stats） ——
        let mut stats = CacheStats::default();
        // 收集器：**未命中**的单元在这里镶嵌（含 `debug_layout` 的布局描边——那条路径
        // 每帧重建，见 `collect_units`）。命中单元不进这里：它们直接在缓存里。
        let mut quads = QuadCollector::new(white_uid, white_uv_tl, white_uv_wh);
        // —— 收集本帧的**提交单元**（窗口 / win=0 顶层放置）——
        //
        // 每个单元 = `(win, place)`：一扇窗，或 win=0 的一个顶层放置。命中 ⇒ 计划从缓存
        // **move 出来**（零顶点拷贝）；未命中 ⇒ 现场镶嵌 + 合并（`build_plans`），本帧
        // 照常提交它，提交完再 move 回缓存（下面的回填循环）⇒ **"重建帧照常绘制、
        // 不消失一帧"由结构保证**（见 `collect_units` 的文档）。
        let t_asm = Instant::now();
        let mut units: Vec<SubmitUnit> = std::mem::take(&mut self.state.scratch_units);
        units.clear();
        self.collect_units(&mut groups, &wins, &mut units, &mut quads, &mut stats);
        // 单元级排序（每帧 ~15 个）：`(win, place)`——非窗口内容（win=0）按**放置序**
        // （后录制者在上），窗口按 z（非窗口恒在窗口之下）。**单元内**的顺序已在计划里
        // 烘好（`BatchPlan`）：`elem`（后录控件覆盖先录）+ `g`（图形先于文字）+ `tex` +
        // `clip`（一次 draw 只能一个 scissor）。
        units.sort_unstable_by_key(|u| (u.win, u.place));
        // 「装配」= 收集 + 合并 + 单元排序，扣掉已单独计时的 `sig` / `collect`
        // （那是"索引"与"镶嵌"的账，不是几何搬运）。
        let submit_asm_us =
            (t_asm.elapsed().as_secs_f64() * 1e6 - stats.sig_us - stats.collect_us).max(0.0);
        // ── 提交：每个计划一次 `flush_seg`。计划是**借来的**（`&BatchPlan`）⇒ 命中路径
        //    全程零拷贝；`UiBatch` 借用切片交给后端，不做"再拷一份"。
        //
        // ⚠ **UI 自行管理绘制顺序**：UI 的 Render2D 必须 `set_sort_mode(SortMode::None)`
        // （关闭排序，完全按提交顺序绘制）；`LayerOnly`（稳定排序）同层保持提交顺序也可。
        // 不要用 `LayerAndStates`：它按 `(rstates, texture_uid)` 重排，字形图集页 uid <
        // 程序化纹理页 uid → 圆角/渐变会被排在文字之后绘制，盖住文字。
        //
        // transform = 屏幕固定变换（窗口原点物理像素）→ 局部顶点映射到世界。
        //
        // **绘制序追踪**（诊断，`RJ_ORDER_TRACE=<帧号>|all`）：按**真实提交顺序**逐条打印
        // 批次计划。为什么需要它：`elem = 0` 的语义是"画在**本容器**元素之下"，只有
        // "每个顶层放置一个排序空间"（`place`）才成立；"某个 win=0 控件被别的 win=0 内容
        // 穿透 / 看错层级"这类现象只能靠这份序核对。
        //
        // 阶段 9 起计划是"单元内已合并好的"，所以打的是**计划级**：`(win, place, tex,
        // clip, verts, 元素区间, 首顶点)`——`v0` 用来认出这是谁（如面板底 `(30,30)` vs
        // FPS 标签 `(25,23)`）。**单元内**（`elem = 0` 的装饰 vs 自家内容）的顺序由
        // `merge_plans` 的排序保证，单测
        // `submit_order_puts_a_placement_above_earlier_content_and_below_its_own_children` 钉住。
        //
        // ⚠ 一帧 ~50 行且 `eprintln!` 无缓冲：`all` 时**必须重定向到真文件**（经
        // PowerShell 管道重定向会顶满缓冲把应用压到 ~1fps——实测踩过，与引擎无关）。
        let trace = match std::env::var("RJ_ORDER_TRACE") {
            Ok(v) if v == "all" => true,
            Ok(v) => v
                .parse::<u64>()
                .map(|f| f == self.state.frame)
                .unwrap_or(false),
            Err(_) => false,
        };
        if trace {
            let mut i = 0usize;
            for u in &units {
                for plan in &u.plans {
                    let v0 = plan
                        .geom
                        .verts
                        .first()
                        .map(|v| (v.pos[0].round(), v.pos[1].round()));
                    eprintln!(
                        "order[frame {}] i={i} win={} place={} tex={} clip={} verts={} elems={}..{} v0={v0:?}",
                        self.state.frame,
                        u.win,
                        u.place,
                        plan.texture,
                        if plan.clip.is_some() { 1 } else { 0 },
                        plan.geom.verts.len(),
                        plan.elements.first().copied().unwrap_or(0),
                        plan.elements.last().copied().unwrap_or(0),
                    );
                    i += 1;
                }
            }
        }
        let t_flush = Instant::now();
        let layer_base = self.base_layer;
        for u in &units {
            for plan in &u.plans {
                if self.flush_seg(backend, layer_base, u.win, plan) {
                    // 段统计（`[perf] segs=/verts=/tris=`）：**段数 = draw call 候选数**，
                    // 顶点/三角数是"这一帧到底提交了多少"的直接度量。
                    let acc = &mut self.state.frame_state.stats;
                    acc.seg_count = acc.seg_count.saturating_add(1);
                    acc.vert_count = acc.vert_count.saturating_add(plan.geom.verts.len() as u32);
                    acc.tri_count = acc.tri_count.saturating_add(plan.geom.tris.len() as u32);
                }
            }
        }
        let submit_flush_us = t_flush.elapsed().as_secs_f64() * 1e6;
        let submit_us = submit_asm_us + submit_flush_us;
        // ── 计划回填缓存（**move**，不是克隆）：命中与未命中都写"本帧的计划 + 本帧的
        //    签名"——未命中时这份就是刚建好的新计划，于是"本帧提交的"与"缓存里的"逐位
        //    一致，下一帧直接命中。
        for u in &mut units {
            match u.slot.take() {
                // ⚠ 同帧同 id 出现两次时以**后者**为准（与旧实现的 `insert` 语义一致）。
                Some(CacheSlot::Window(id)) => {
                    self.state
                        .window_quads
                        .insert(id, (u.sig, std::mem::take(&mut u.plans)));
                }
                Some(CacheSlot::Z0(slot)) => {
                    self.state
                        .z0_quads
                        .insert(slot, (u.sig, std::mem::take(&mut u.plans)));
                }
                None => {}
            }
        }
        units.clear();
        self.state.scratch_units = units;

        // ── Debug 叠加（DebugDraw / debug_layout 描边 / 焦点描边）────────────
        // 在**全部 UI 内容之后**提交（`submit_debug`：合并 debug_queue 与布局描边、
        // 按 win 分组、白纹理、屏幕固定变换提交——不进窗口缓存、恒覆盖在最上）。
        self.submit_debug(backend, &mut quads, white_uid, layer_base);
        // ── 段统计**累加**进帧级暂存 ─────────────────────────────────────
        // （`UiStats` 是每帧聚合值：帧号 / `ui_frame_us` 由 `Ui::end_frame` 写回。）
        let acc = &mut self.state.frame_state.stats;
        acc.cmd_count = acc.cmd_count.saturating_add(cmd_count);
        acc.win_count = acc.win_count.saturating_add(stats.win_count);
        acc.cache_hits = acc.cache_hits.saturating_add(stats.cache_hits);
        acc.cache_misses = acc.cache_misses.saturating_add(stats.cache_misses);
        acc.sort_us += sort_us;
        acc.sig_us += stats.sig_us;
        acc.collect_us += stats.collect_us;
        acc.clone_us += stats.clone_us;
        acc.submit_us += submit_us;
        acc.submit_asm_us += submit_asm_us;
        acc.submit_flush_us += submit_flush_us;
        acc.finish_us += t_finish.elapsed().as_secs_f64() * 1e6;
        // 记录 IME 组合状态（供下一帧退格判定，见 text_input_at）
        self.state.ime_composing = self.keyboard.ime_preedit().is_some_and(|p| !p.is_empty());
        // **段收尾**：本段的帧级事实（按下归属 / 窗口原点 / 焦点链 / 责任链 / 光标意图 /
        // z0 组号）回存暂存，供同帧后续段与帧收尾使用。
        // ⚠ 帧级收尾（输入结算 / 焦点导航与描边 / 光标定夺 / 统计写回 / 复位）**不在这里**：
        // 那是每帧一次的事，搬到 [`Self::end_frame`]。
        self.save_frame_state();
    }

    /// **帧收尾**（**每帧一次**，由运行时在 UI 队列被提交之前调用；低层手动路径自己调）。
    ///
    /// 与 [`Self::finish`]（段收尾：分桶 → 顶点 → 提交）分工明确：
    /// 1. [`Self::finish_pre_input`]：空白点击清焦点 / 清一次性边沿 / 窗口按下裁决
    ///    （`resolve_win_press`）/ 键盘导航（Tab·方向键 / Esc）；
    /// 2. 焦点描边（`handle_focus_keys` 内，走 `debug_queue`：不进窗口缓存、恒覆盖在最上）；
    /// 3. `finish(backend)`：把上面追加的描边命令 flush 出去（本视图自身命令通常为空）；
    /// 4. [`Self::finalize_cursor_and_reset`]：系统光标定夺 / `window_rects` 清理 /
    ///    统计写回（帧号 +1、`ui_frame_us` = 开场 → 收尾）/ 帧级字段复位；
    /// 5. [`UiState::end_frame`]：帧级暂存关场（下次开场重新清零）。
    pub fn end_frame(&mut self, backend: &mut dyn UiBackend) {
        self.finish_pre_input();
        self.finish(backend);
        self.finalize_cursor_and_reset();
        self.state.end_frame();
    }

    /// **帧末输入结算**（`finish` 起始）：空白点击清焦点（本帧按下且无控件响应）、
    /// 清除一次性边沿、窗口按下裁决（重叠点击只让最上层窗口拖拽与置顶）、键盘导航
    /// （Tab/方向键遍历焦点链、Esc 关浮层/失焦、焦点描边）。须在本帧命令取走
    /// （录制队列 `mem::take`）**之前**调用——焦点描边会追加到队列。
    fn finish_pre_input(&mut self) {
        // 空白点击清焦点（本帧按下且无控件响应）
        if self.mouse_left().down_edge() && !self.any_pressed && self.state.focused.is_some() {
            self.state.focused = None;
            self.state.focused_kind = None;
        }
        // 清除一次性边沿
        for ws in self.state.widgets.values_mut() {
            clear_frame_flags(ws);
        }
        // 窗口按下裁决：重叠点击只让**最上层**窗口获得拖拽与置顶（见 window_at）
        self.resolve_win_press();
        // 控件按下裁决：**用帧末完备的遮挡表复核**本帧认领按下的控件（见该方法文档）。
        self.resolve_widget_press();
        // 键盘导航：Tab / Shift+Tab / 方向键遍历焦点链、Esc 关浮层/失焦、焦点描边。
        self.handle_focus_keys();
    }

    /// **控件按下裁决**（帧末复核）：本帧有控件认领了按下，但**帧末**看它所在的窗口并非
    /// 鼠标下最上层（被更高 z 的窗口盖住）⇒ 撤销这次认领。
    ///
    /// # 为什么需要"帧末复核"
    ///
    /// 窗口遮挡判定用的是 `UiState::window_rects`——一张**录制期逐步写入**的表：本帧还没
    /// 录到的窗口，表里是**上一帧**的矩形。于是"盖住我的那个窗口本帧才移过来 / 本帧才被抬高
    /// z"这两种情况下，控件命中时会被判成"没被遮挡"而收下按下。命中的那一刻几何还没录全，
    /// **唯一能拿到完备几何的时机就是帧末**（所有窗口都录完了），所以复核放在这里。
    ///
    /// # ⚠ 比较基准必须是**窗口的当前 z**，不是认领时的旧 z
    ///
    /// `resolve_win_press` 在帧末会把**被点的窗口**抬到 `max+1`；若这里拿"认领时的旧 z"去比，
    /// 那个窗口的**新 z 比自己的旧 z 大**、且它的矩形当然覆盖鼠标（鼠标就在它里面）⇒
    /// **窗口把自己判成"被别人盖住"**，于是**每一次**窗口内控件的按下都被撤销：滑块 /
    /// 滚动条 / 文本选择全都拖不动（真实回归，已由 `--sim-cover` 的正对照守住）。
    /// 用当前 z 后，"自己"与"自己"相等，而 `window_occluded` 是**严格大于**判定 ⇒ 自己永远
    /// 不遮挡自己，同时"确实被别人盖住"依旧成立。
    ///
    /// 代价与边界：一次按下已在命中那一帧被应用侧读到（动作已发生）——本条只保证
    /// **状态不再延续**（`pressed` / `clicked` / `dragging` 全清、`press_claimed` 保持），
    /// 因此不会出现"被盖住的控件一直拖到释放"。同帧内按下 + 抬起（极快点击）不受保护。
    fn resolve_widget_press(&mut self) {
        let Some((id, win_id)) = self.state.frame_state.press_widget.take() else {
            return;
        };
        // 本控件所在窗口的**当前** z（`None` = win=0 非窗口内容 ⇒ z=0）。
        let my_z = win_id
            .as_ref()
            .and_then(|w| self.state.window_z.get(w.as_str()).copied())
            .unwrap_or(0);
        if !window_occluded(my_z, self.mouse_logical, self.window_rects_iter()) {
            return;
        }
        let ws = self.state.widgets.entry(id).or_default();
        ws.pressed = false;
        ws.clicked = false;
        ws.dragging = false;
        self.state.press_cancelled_by_window =
            self.state.press_cancelled_by_window.saturating_add(1);
    }

    /// **收集本帧的提交单元**（`finish` 的核心缓存步骤）：遍历分桶后的窗口，把每个
    /// **提交单元**（一扇窗 / win=0 的一个顶层放置）变成一份 [`BatchPlan`] 列表：
    ///
    /// - **命中 ⇒ `remove` 出计划**（move 一个 `Vec`，**零顶点拷贝**）；
    /// - **未命中 ⇒ 现场镶嵌 + 合并**（[`Self::build_plans`]），本帧照常提交它，提交完
    ///   再把计划 move 回缓存（`finish` 末尾的回填循环）。
    ///
    /// "重建帧照常绘制、不消失一帧"这条**历史不变量由结构本身保证**：数据是当前帧亲手
    /// 从缓存里拿出来的**所有权值**，z→id 映射只解一次，不存在"按旧 z 读新表"的竞态
    /// （那是更早的"零拷贝两阶段读取"版本踩过的坑）。
    ///
    /// `debug_layout` 开启时**每帧重建**（布局描边是调试视图，跳过缓存）。
    /// 各阶段耗时 / 计数累加到 `stats`（`finish` 末尾写入 [`UiStats`]）。
    fn collect_units(
        &mut self,
        groups: &mut std::collections::HashMap<u32, Vec<Vec<UiDraw>>>,
        wins: &[u32],
        units: &mut Vec<SubmitUnit>,
        quads: &mut QuadCollector,
        stats: &mut CacheStats,
    ) {
        for &win in wins {
            let cmds = groups.remove(&win).expect("group exists");
            if win == 0 {
                // 非窗口内容：按**顶层放置**切单元（`place` 既是排序空间也是缓存键）。
                self.collect_z0_units(&cmds, units, quads, stats);
                continue;
            }
            let Some(id) = self.win_ids.get(&win).cloned() else {
                // 没有 id 的窗口（理论上不会有）⇒ 只现场建计划，不进缓存。
                let plans = self.build_plans(win, &cmds, quads, 0, stats);
                units.push(SubmitUnit {
                    win,
                    place: 0,
                    sig: 0,
                    slot: None,
                    plans,
                });
                continue;
            };
            stats.win_count += 1;
            let t_sig = Instant::now();
            // **anchor = 本窗原点**：缓存里存的是窗口局部顶点，签名也必须按局部坐标算，
            // 否则"窗口移动 = 内容变化"⇒ 拖动/滚动每帧整窗重镶嵌。
            let anchor = self.win_origins.get(&win).copied().unwrap_or(Vec2::ZERO);
            let sig = self.hash_cmds(cmds.iter().flatten(), anchor);
            stats.sig_us += t_sig.elapsed().as_secs_f64() * 1e6;
            let hit = !self.debug_layout
                && self
                    .state
                    .window_quads
                    .get(&id)
                    .is_some_and(|(s, _)| *s == sig);
            let plans = if hit {
                stats.cache_hits += 1;
                // **move 出缓存**（不是克隆）：本帧提交完再原样放回去。
                self.state.window_quads.remove(&id).expect("just checked").1
            } else {
                stats.cache_misses += 1;
                self.build_plans(win, &cmds, quads, 0, stats)
            };
            units.push(SubmitUnit {
                win,
                place: 0,
                sig,
                slot: (!self.debug_layout).then_some(CacheSlot::Window(id)),
                plans,
            });
        }
    }

    /// **win=0：按顶层放置收集提交单元**（[`Self::z0_place_for_seq`] 的 `place`）。
    ///
    /// 每个放置一个单元 ⇒ 单元内 `place` 恒定 ⇒ 计划能整体缓存；跨单元的序由
    /// `(win, place)` 在 `finish` 里排（~15 个单元，代价可忽略）。
    fn collect_z0_units(
        &mut self,
        cmds: &[Vec<UiDraw>],
        units: &mut Vec<SubmitUnit>,
        quads: &mut QuadCollector,
        stats: &mut CacheStats,
    ) {
        let mut by_place: Vec<(u32, Vec<&UiDraw>)> = Vec::new();
        for d in cmds.iter().flatten() {
            let place = self.z0_place_for_seq(d.seq);
            match by_place.iter_mut().find(|(pp, _)| *pp == place) {
                Some((_, v)) => v.push(d),
                None => by_place.push((place, vec![d])),
            }
        }
        // **放置槽布局**（诊断，`RJ_ORDER_TRACE=slot`）：逐帧打印每个放置槽的 `seq` 区间。
        // 用来回答"同一个容器是不是被拆进了两个排序空间"。
        if std::env::var("RJ_ORDER_TRACE").is_ok_and(|v| v == "slot") {
            for (place, refs) in &by_place {
                let lo = refs.iter().map(|d| d.seq).min().unwrap_or(0);
                let hi = refs.iter().map(|d| d.seq).max().unwrap_or(0);
                eprintln!(
                    "z0slot[frame {}] seg={} place={place} n={} seq={lo}..{hi}",
                    self.state.frame,
                    self.segment,
                    refs.len()
                );
            }
            eprintln!(
                "z0starts[frame {}] seg={} starts={:?}",
                self.state.frame, self.segment, self.z0_ranges
            );
        }
        for (place, refs) in by_place {
            // **槽 = (段号, 放置序)**：`place` 是段内派生的计数（各段从 0 起），故必须带
            // 段前缀，否则不同段的同号槽共用一个缓存条目、每帧交替覆盖 → 永远 miss。
            // ⚠ `place` **逐帧重算**（不存进 `UiDraw`）：放置增删会让后面的 `place` 整体
            // 平移，键跟着变 ⇒ 直接判 miss 重建，绝不会"命中一个 `place` 已过期的条目"
            // 而把绘制序搞错一帧（那正是阶段 8 修掉的闪烁形态）。
            let slot = (self.segment, place);
            if !self.state.frame_state.z0_seen.contains(&slot) {
                self.state.frame_state.z0_seen.push(slot);
            }
            let t_sig = Instant::now();
            // 非窗口内容的 anchor = 0（其坐标本就是绝对屏幕坐标）。
            let sig = self.hash_cmds(refs.iter().copied(), Vec2::ZERO);
            stats.sig_us += t_sig.elapsed().as_secs_f64() * 1e6;
            let hit = !self.debug_layout
                && self
                    .state
                    .z0_quads
                    .get(&slot)
                    .is_some_and(|(s, _)| *s == sig);
            let plans = if hit {
                stats.cache_hits += 1;
                // **move 出缓存**（不是克隆）：本帧提交完再原样放回去。
                self.state.z0_quads.remove(&slot).expect("just checked").1
            } else {
                stats.cache_misses += 1;
                // 该放置重建（克隆命令为 owned 单桶传入 `collect_cmds`）。
                let owned: Vec<UiDraw> = refs.iter().map(|d| (*d).clone()).collect();
                self.build_plans(0, std::slice::from_ref(&owned), quads, place, stats)
            };
            units.push(SubmitUnit {
                win: 0,
                place,
                sig,
                slot: (!self.debug_layout).then_some(CacheSlot::Z0(slot)),
                plans,
            });
        }
        // 陈旧放置的清理**不在这里**：本函数每段跑一次、只见到本段的槽，按段清会把同帧
        // 其它段刚写好的缓存删掉（那些槽于是每帧 miss）。这里只把本段见到的槽记进帧级
        // 暂存（`z0_seen`），由 `UiState::begin_frame` 在**下一帧开场**按完整集合清一次。
    }

    /// **现场建计划**（缓存未命中）：镶嵌（`collect_cmds`）+ 合并成该单元的
    /// [`BatchPlan`] 列表。`place` 只用于诊断归因（`RJ_CACHE_TRACE`）。
    fn build_plans(
        &mut self,
        win: u32,
        cmds: &[Vec<UiDraw>],
        quads: &mut QuadCollector,
        place: u32,
        stats: &mut CacheStats,
    ) -> Vec<BatchPlan> {
        let t_collect = Instant::now();
        self.collect_cmds(quads, win, cmds);
        stats.collect_us += t_collect.elapsed().as_secs_f64() * 1e6;
        self.trace_cache_miss(
            &format!("win {win} place {place}"),
            cmds.iter().map(|v| v.len()).sum(),
            t_collect,
        );
        self.merge_plans(quads, win, place)
    }

    /// **把收集器里属于 `(win, place)` 的几何合并成提交计划**（`finish` 的装配步骤）。
    ///
    /// 只在**未命中**时调用一次；命中路径直接用缓存里的计划（零拷贝）。步骤：
    /// 取出本单元的条目 → 按 `(elem, group, tex, clip)` 排序 → `segment_runs` 切段 →
    /// 每段几何拼成一段（单条段直接 move，多条段 `append` 一次）→ 附上元素序列表。
    ///
    /// ⚠ 段**不跨单元**合并：两个相邻单元即使 `(win, tex, clip)` 相同也各出一次批次
    /// ⇒ **真实 draw call 会小幅上升**（实测 40 → 44；`Render2D` 的相邻合批对 UI 批次
    /// 不生效，因为每个 `mesh_indexed` 带自己的矩阵下标）。取舍与理由见 [`BatchPlan`]。
    fn merge_plans(&mut self, quads: &mut QuadCollector, win: u32, place: u32) -> Vec<BatchPlan> {
        let mut ordered: Vec<CachedQuad> = std::mem::take(&mut self.state.scratch_ordered);
        ordered.clear();
        // 取出**本单元**的条目（`(win, place)` 唯一定位；其余条目留给别的单元）。
        let keys: Vec<crate::gpu_batch::QuadKey> = quads
            .quads
            .keys()
            .copied()
            .filter(|k| k.0 == win && k.1 == place)
            .collect();
        for k in keys {
            let geom = quads.quads.remove(&k).expect("just listed");
            let elems = quads.elems.remove(&k).unwrap_or_default();
            ordered.push((k.0, k.1, k.2, k.3, k.4, k.5.map(clip_rect), geom, elems));
        }
        ordered.sort_unstable_by_key(crate::gpu_batch::submit_sort_key);
        let runs = segment_runs(
            ordered.iter().map(|q| (q.0, q.4, q.5, q.6.verts.len())),
            MAX_UI_SEG_VERTS,
        );
        let mut plans: Vec<BatchPlan> = Vec::with_capacity(runs.len());
        let mut next = 0usize;
        // 复用同一个 `Geom` scratch（每个单元 1 次分配，而不是每段 1 次）。
        let mut scratch_seg = Geom::default();
        for run in runs {
            let geom = if run.quads == 1 {
                // 单条：直接 move（省一次 `append` 全量拷贝）——`ordered` 是 scratch。
                Geom {
                    verts: std::mem::take(&mut ordered[next].6.verts),
                    tris: std::mem::take(&mut ordered[next].6.tris),
                }
            } else {
                scratch_seg.verts.clear();
                scratch_seg.tris.clear();
                scratch_seg.verts.reserve(run.verts);
                for q in &ordered[next..next + run.quads] {
                    // 索引按已累计顶点数平移（`Geom::append`）——各段索引从 0 起。
                    scratch_seg.append(&q.6);
                }
                Geom {
                    verts: std::mem::take(&mut scratch_seg.verts),
                    tris: std::mem::take(&mut scratch_seg.tris),
                }
            };
            // 本段覆盖的**元素序集合**（去重；`UiBatchSource::elements` 的来源）。
            let mut elements: Vec<u32> = ordered[next..next + run.quads]
                .iter()
                .flat_map(|q| q.7.iter().copied())
                .collect();
            elements.sort_unstable();
            elements.dedup();
            next += run.quads;
            if geom.is_empty() {
                continue;
            }
            plans.push(BatchPlan {
                texture: run.texture,
                clip: run.clip,
                geom,
                elements,
            });
        }
        self.state.scratch_ordered = ordered;
        plans
    }

    /// **缓存未命中的逐窗 / 逐放置归因**（诊断，`RJ_CACHE_TRACE=1`）：窗口 id（或 win=0
    /// 放置序）、命令条数、本次重建耗时。用来回答"稳态下到底是哪几扇窗 / 哪几个 win=0
    /// 放置每帧重镶嵌、各占多少"——顶点缓存 miss 的代价远高于命中（整块重新镶嵌，含
    /// 投影的同心环）。
    ///
    /// 只在 miss 分支调用（命中路径零开销）；env 读取也只发生在未命中时。
    /// 历史战绩：稳态 `cache_miss` 恒等于槽数、`collect` ≈ 0.6ms 时，靠它一眼看出
    /// "miss 全在 win=0 槽、一扇窗都没 miss"，从而定位到槽键跨段冲突（见
    /// [`crate::UiState::z0_quads`]）。
    fn trace_cache_miss(&self, what: &str, cmds: usize, t_collect: Instant) {
        if std::env::var_os("RJ_CACHE_TRACE").is_none() {
            return;
        }
        eprintln!(
            "cache[frame {}] MISS {what} cmds={cmds} collect={:.1}us",
            self.state.frame,
            t_collect.elapsed().as_secs_f64() * 1e6
        );
    }

    /// 对一组命令做**全量内容签名**（`cmd_sig` 哈希）：窗口 / win=0 放置顶点缓存的 key。
    ///
    /// 签名里**并入字形图集的区域失效世代号**（[`rjw_text::Text::atlas_revision`]）：
    /// 本缓存烘的是**最终 UV**（字形 + WHITE 基础纹理都取自字形图集），图集一旦重排
    /// 或复用已逐出字形的槽位，旧 UV 就指向**别的像素**（"陈旧文字"/"背景消失"），
    /// 而命令内容没变 ⇒ 只靠命令哈希永远不失效。世代号只由图集整理（分配失败触发）
    /// 推进，不是每帧变化。
    ///
    /// 签名还**以主题行距 + 字重为前缀**（[`Theme::line_spacing`] / [`Theme::font_weight`]）：
    /// `DrawKind::Text` 的 `buf` 是排版结果、按设计**不参与哈希**（命令内容相同），而换行
    /// 文本的实际行高 / 整体高度取决于行距、字形与步进宽度取决于字重 ⇒ 不并入就会被
    /// "改了行距 / 字重但窗口几何仍命中旧缓存"卡住（固定矩形里的居中文本尤其明显）。
    /// 两者都是主题令牌、只在主题变更时改，代价可忽略。
    ///
    /// `anchor` = 该窗口 / 放置的原点（物理像素）：见 [`cmd_sig_hash`]——按局部坐标入签名，
    /// 于是**窗口移动 / 滚动不会让签名变化**（拖动窗口不再每帧整窗重镶嵌）。
    fn hash_cmds<'c>(&self, cmds: impl IntoIterator<Item = &'c UiDraw>, anchor: Vec2) -> u64 {
        use std::hash::Hasher;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        h.write_u32(self.theme.line_spacing.to_bits());
        h.write_u16(self.theme.font_weight.0);
        for d in cmds {
            self.cmd_sig(&mut h, d, anchor);
        }
        geom_cache_sig(h.finish(), self.text.atlas_revision())
    }

    /// **冲刷一个批次计划**：把 [`BatchPlan`] 作为 [`UiBatch`] 提交（单一窗口的
    /// `screen_fixed_tf` 变换 + 窗口级 FX tint/transform override + **batch scissor**）。
    ///
    /// `plan.clip` = 本段的环境裁剪层（窗口 / 放置**局部**坐标）⇒ 经
    /// [`crate::view::batch_scissor`] 映射成该批次的屏幕像素 scissor。
    ///
    /// ⚠ **几何按引用交给后端**（`UiBatch` 借用切片）：计划是缓存里的东西，提交期不能
    /// 把它的顶点搬走——这正是阶段 9 去掉"每帧克隆一遍几何"的关键。
    ///
    /// **解耦**：不直接调 `Render2D`，只产出数据；后端决定如何提交。
    /// 返回是否真的提交了（纹理缺失 / 空几何 ⇒ `false`，供段统计口径一致）。
    fn flush_seg(
        &mut self,
        backend: &mut dyn UiBackend,
        layer_base: f64,
        win: u32,
        plan: &BatchPlan,
    ) -> bool {
        if plan.geom.is_empty() {
            return false;
        }
        let Some(texture) = backend.texture(plan.texture) else {
            return false;
        };
        let anchor_px = self.win_origins.get(&win).copied().unwrap_or(Vec2::ZERO);
        let base_tf = screen_fixed_tf(anchor_px);
        // 窗口级 FX：tint（淡入淡出/染色）+ transform override（位移/缩放/旋转，
        // 绕**归一化锚点** `fx.anchor`）。`transform = IDENTITY` 时结果恒 = `base_tf`
        // （锚点不影响位置）。
        let fx = self
            .win_ids
            .get(&win)
            .and_then(|id| self.state.window_fx.get(id))
            .copied()
            .unwrap_or_default();
        let tf = match fx.transform {
            Some(t) => {
                // 绕锚点：`base_tf · (T_anc · t · T_anc⁻¹)`——先平移 -锚点（局部），
                // 应用 t，再平移回锚点，最后基础屏幕固定。组合验证：
                // `T_anc⁻¹.with_transform(&t)` = t·T_anc⁻¹；再 `.with_transform(&T_anc)`
                // = T_anc·t·T_anc⁻¹。t = IDENTITY 时 = T_anc·T_anc⁻¹ = IDENTITY ✓。
                // 遮挡表按**窗口 ID** 键，这里只有 z（win）⇒ 先经 `win_ids` 解出 ID。
                let size = self
                    .win_ids
                    .get(&win)
                    .and_then(|id| self.state.window_rects.get(id.as_str()))
                    .map(|r| Vec2::new(r.w, r.h))
                    .unwrap_or(Vec2::ZERO);
                let anchor_local = Vec2::new(fx.anchor.x * size.x, fx.anchor.y * size.y);
                let t_anc = Transform2D::IDENTITY.with_pos(anchor_local);
                let t_anc_inv = Transform2D::IDENTITY.with_pos(-anchor_local);
                let c = t_anc_inv.compose(&t).compose(&t_anc);
                c.compose(&base_tf)
            }
            None => base_tf,
        };
        // 诊断：记录本窗口**实际提交用的平移量**（`debug_dump` 的 `submit` 字段）——
        // 与 `origin` 比对即可判定"引擎状态 vs 视觉"是否一致。
        self.state.debug_submit.insert(win, tf.pos);
        // **batch scissor**：窗口/放置**局部**裁剪层 → 本批次的屏幕像素矩形（见
        // [`crate::view::batch_scissor`]）。`tf` 通常只是"平移到窗口原点"⇒ 结果 = 局部
        // 裁剪 + 窗口原点；窗口 FX（缩放/旋转）下走保守 AABB（宁可多画一点，绝不误裁）。
        // ⚠ 裁剪层必须与**缓存里的顶点同空间**（局部）：否则窗口一动，命中的旧缓存会把
        // scissor 留在旧位置（"scissor 不跟内容一起移动"，用户实测）。
        let clip = plan.clip.map(|c| crate::view::batch_scissor(c, &tf));
        if let Some(c) = clip {
            // 诊断 + 计数：本帧带 scissor 的批次数（`[perf] clip_batches`）。
            self.state.debug_clip.insert(win, c);
            self.state.frame_state.stats.clip_batches =
                self.state.frame_state.stats.clip_batches.saturating_add(1);
        }
        backend.submit(UiBatch {
            texture,
            vertices: &plan.geom.verts,
            indices: &plan.geom.tris,
            transform: tf,
            tint: fx.tint,
            layer: layer_base + win as f64 * 1.0,
            clip,
            source: UiBatchSource {
                window: win,
                elements: plan.elements.len() as u32,
                debug: false,
            },
        });
        true
    }

    /// **提交 Debug 叠加**（`finish` 末尾，全部 UI 内容之后）：合并 `debug_queue`
    /// （屏幕空间调试图元）与 `collect_cmds` 期间产生的布局描边（`quads.debug`），按 win
    /// 分组、白纹理、屏幕固定变换提交——不进窗口缓存、同 layer 后提交 → 恒覆盖在最上。
    fn submit_debug(
        &mut self,
        backend: &mut dyn UiBackend,
        quads: &mut QuadCollector,
        white_uid: u64,
        layer_base: f64,
    ) {
        // 1. 收集 debug_queue（[`Self::debug_line`] 等屏幕空间调试图元），与布局描边合并。
        let mut debug_groups: std::collections::HashMap<u32, Vec<UiDraw>> =
            std::collections::HashMap::new();
        for d in self.painter.q.debug_queue.drain(..) {
            debug_groups.entry(d.win).or_default().push(d);
        }
        let mut dwins: Vec<u32> = debug_groups.keys().copied().collect();
        dwins.sort_unstable();
        for win in dwins {
            let cmds = debug_groups.remove(&win).expect("group exists");
            self.collect_cmds(quads, win, &[cmds]);
        }
        // 2. 提交 quads.debug 顶点（白纹理 + 屏幕固定变换）。
        let mut dwins: Vec<u32> = quads.debug.keys().copied().collect();
        dwins.sort_unstable();
        for win in dwins {
            let geom = quads.debug.remove(&win).expect("debug group exists");
            if geom.is_empty() {
                continue;
            }
            let Some(texture) = backend.texture(white_uid) else {
                continue;
            };
            let anchor_px = self.win_origins.get(&win).copied().unwrap_or(Vec2::ZERO);
            let tf = screen_fixed_tf(anchor_px);
            self.state.debug_submit.insert(win, tf.pos);
            backend.submit(UiBatch {
                texture,
                vertices: &geom.verts,
                indices: &geom.tris,
                transform: tf,
                tint: Color::WHITE,
                layer: layer_base + win as f64 * 1.0,
                // 调试叠加**不受内容裁剪**（诊断图元要看得见）：scissor = None。
                clip: None,
                source: UiBatchSource {
                    window: win,
                    elements: 1,
                    debug: true,
                },
            });
        }
    }

    /// **光标定夺 + 帧复位**（`finish` 末尾）：按优先级选择系统光标（窗口拖拽 Arrow >
    /// 抓握 > 控件作者自定义 > 文本 I 型 > 可拖拽 Grab > 默认），无 UI 光标意图时抑制
    /// （保留应用自定义光标，如游戏准星；上一帧设过则清一次回 Default）；随后清空本帧
    /// 帧级状态（光标位 / depth / seq / cur_win / 窗口映射 / 焦点链等），下一帧从干净起点录制。
    fn finalize_cursor_and_reset(&mut self) {
        // **统计写回**（每帧一次）：各段累加值 + 帧号 + 整帧跨度（开场 → 收尾）。
        let mut stats = std::mem::take(&mut self.state.frame_state.stats);
        stats.frame = self.state.stats.frame.wrapping_add(1);
        stats.ui_frame_us = self.state.frame_state.frame_t0.elapsed().as_secs_f64() * 1e6;
        self.state.stats = stats;
        let intent = self.cursor_text
            || self.cursor_grab
            || self.cursor_grabbing
            || self.cursor_window_drag
            || self.cursor_custom.is_some();
        let icon = if self.cursor_window_drag {
            winit::window::CursorIcon::Default
        } else if self.cursor_grabbing {
            winit::window::CursorIcon::Grabbing
        } else if let Some(icon) = self.cursor_custom {
            icon
        } else if self.cursor_text {
            winit::window::CursorIcon::Text
        } else if self.cursor_grab {
            winit::window::CursorIcon::Grab
        } else {
            winit::window::CursorIcon::Default
        };
        if intent {
            self.window.set_cursor(icon);
            self.state.cursor_was_set = true;
        } else if self.state.cursor_was_set {
            // 无 UI 光标意图但上一帧设过 → 清一次回 Default（避免残留 I 型等）
            self.window.set_cursor(winit::window::CursorIcon::Default);
            self.state.cursor_was_set = false;
        }
        self.cursor_text = false;
        self.cursor_grab = false;
        self.cursor_grabbing = false;
        self.cursor_window_drag = false;
        self.cursor_custom = None;
        // ⚠ `window_rects` 的陈旧清理在**帧末**（这里），但数据源必须是
        // `frame_state.window_ids_seen`——**不能**用本视图的 `win_ids` / `win_origins`：
        // 它们已被上面 `finish()` 末尾的 `save_frame_state` 换走（空表），按它清会把整张
        // 遮挡表清光 ⇒ 遮挡判定退化成"只看本帧已录制的窗口"，上层窗口就挡不住背后窗口的
        // 控件（用户报的"被遮挡的控件仍被触发"）。这是那次事故的根因，别再改回去。
        let seen_wins = &self.state.frame_state.window_ids_seen;
        self.state
            .window_rects
            .retain(|id, _| seen_wins.iter().any(|w| w == id));
        self.painter.q.depth = 0;
        self.painter.q.seq = 0;
        self.painter.q.cur_win = 0;
        self.any_pressed = false;
        self.drag_panel = None;
        self.win_press_top = None;
        self.win_origins.clear();
        self.win_ids.clear();
        // `debug_submit` **故意不清**：`debug_dump()` 通常在本帧**录制期**调用（那时
        // 还没提交），保留上一次 `finish` 的提交平移量才能与 `origin` 对照。
        self.frames.clear();
        self.focusables.clear();
        self.press_claimed = false;
    }
}

/// 把绘制命令按 `win` 分组、组内按 `depth` 分桶（桶内保持**录制序**）。
/// 免全量排序的提交序基础：与 `sort_by_key((win, depth, elem, group, seq))`
/// **完全等价**（命令同深度内 `(elem, group, seq)` 天然有序——元素随录制递增、
/// 同元素"背景/图形"先于文字录制；唯一乱序维度是 depth 与 win）。
pub(super) fn bucket_cmds(queue: Vec<UiDraw>) -> std::collections::HashMap<u32, Vec<Vec<UiDraw>>> {
    let mut groups: std::collections::HashMap<u32, Vec<Vec<UiDraw>>> =
        std::collections::HashMap::new();
    for d in queue {
        let buckets = groups.entry(d.win).or_default();
        while buckets.len() as u32 <= d.depth {
            buckets.push(Vec::new());
        }
        buckets[d.depth as usize].push(d);
    }
    groups
}

/// **绝对矩形 → 窗口局部矩形**（提交 / 镶嵌用的窗口局部物理坐标）。
///
/// 只是平移（减窗口原点）：**环境裁剪不再切割几何**——裁剪改由 batch scissor 在
/// GPU 侧执行（见 [`UiBatch::clip`] / `crate::view::batch_scissor`）。
#[inline]
pub(crate) fn local_of(pr: Rect, anchor_px: Vec2) -> Rect {
    Rect::new(pr.x - anchor_px.x, pr.y - anchor_px.y, pr.w, pr.h)
}

/// **顶层放置序**（纯函数，见 [`Ui`] 的 `z0_ranges` 字段文档）：到 `seq` 这条命令为止
/// 已经开始过的顶层放置个数。
///
/// `starts` = 各顶层放置**第一条命令的 `seq`**（**按录制序**，因此单调不减）；
/// 提前 `break` 依赖这个单调性。散装顶层命令（不属于任何放置）得到"上一个放置"的序
/// —— 它们录制更晚、`elem` 更大，于是仍画在那个放置之上。
///
/// ⚠ 记的是"**第一条命令**的序号"（`begin_top_placement` 传 `queue.seq + 1`），
/// 不是容器入口时的 `seq`——否则紧靠在容器之前录制的那条命令（如 `scroll_at` 的滚动条，
/// 由 `next_seq()` 取号）会被算进新放置，同一容器被拆进两个排序空间。
///
/// 抽成自由函数是为了让这条语义能在**没有 `Ui` 实例**的情况下单测（`Ui` 需要字形图集
/// 与 winit 窗口，构造不出来；见 `ui/tests.rs`）。
pub(crate) fn z0_place_for_seq(starts: &[u32], seq: u32) -> u32 {
    let mut n = 0;
    for &start in starts {
        if start <= seq {
            n += 1;
        } else {
            break;
        }
    }
    n
}
