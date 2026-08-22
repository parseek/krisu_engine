//! eg260818UI —— `rjw_ui` 示例：DOM 风格自动布局 + Tkinter 几何管理器（pack / grid / place）。
//!
//! 展示：
//! - **独立 UI 渲染**：UI 录制到**单独 Render2D**（`set_sorting(false)`：关闭 Render2D
//!   排序，UI 自行管理绘制顺序——窗口按 z 提交、窗口内"背景/图形 → 文字"），与世界合并
//!   提交（世界 `render_command_buffer` → UI `render_command_buffer` → 一次 present）
//! - **Window 容器**：可重叠 + 点击置顶（焦点 z-order）+ 可拖拽；同一 layer 内
//!   "背景/图形 → 文字"顺序绘制（不做元素重叠处理）
//! - **pack**：左侧主菜单（标题 / 按钮 / 滑块 / 勾选框 / 单选组）垂直堆叠
//! - **grid**：背包 3 列均匀网格（单元格尺寸跨帧缓存），点击格子切换
//! - **place**：顶部状态栏（渐变背景 + 玩家名输入框）与底部说明绝对定位
//! - **键盘导航**：Tab / Shift+Tab / 方向键遍历焦点（青色描边），Enter / Space 激活、
//!   左右键调滑块、下拉框展开时方向键切选项、Esc 收起/失焦
//! - **输入屏蔽**：文本输入框聚焦时（`UiState::capturing_text()`）屏蔽应用快捷键
//!   （输入 `R` / `Esc` 不再触发重置 / 退出）
//! - **IME**：中文输入支持（上屏 + 组合候选 + 候选框定位到输入框光标）
//!
//! 操作：鼠标点击 / 拖拽 · 键盘 Tab/方向键/Enter/Space/Esc · 输入框打字（IME 已支持，
//! Enter / Esc 失焦） · `R` 重置 UI 状态 · `Esc`（先失焦）退出

use std::time::Instant;

use glam::{Vec2, vec2};
use rjw_2d_render::{ClearConfig, Render2D, SpriteRect};
use rjw_color::Color;
use rjw_main::*;
use rjw_render::{wgpu, RenderConfig, RenderContext};
use rjw_text::Text;
use rjw_transform::{Transform2D, Viewport};
use rjw_ui::{Anchor, Button, Divider, FontModal, IdAbsolute, Label, NumberInput, PackSide, PanelStyle, Slider, Theme, Ui, UiAdd, UiState, UiStats, WindowClamp, WindowFx};

const LAYER_UI: f64 = 10_000_000.0;

/// 数字输入 / 字体 Modal 演示状态。
struct UiApp {
    render: Option<RenderContext>,
    render2d: Option<Render2D>,
    render2d_ui: Option<Render2D>,
    font: Option<Text>,
    viewport: Viewport,
    ui_state: UiState,
    // 窗口位置（支持命令行参数覆盖，便于 RenderDoc 验证重叠次序）
    win_a_pos: Vec2,
    win_b_pos: Vec2,
    // 演示状态（由 UI 控件驱动）
    clicks: u32,
    volume: f32,
    fullscreen: bool,
    /// 数字输入（数字条）值。
    hp: f32,
    /// 水平行（row）演示的第二/三个数字条状态。
    hp2: f32,
    hp3: f32,
    /// checkbox_mut（WidgetId）演示状态。
    show_hud: bool,
    opt7: bool,
    /// 当前应用的字体族（空 = 系统默认；FontModal 确定后写入）。
    font_name: String,
    /// 字体 Modal 输入框内容（跨帧持久）。
    font_input: String,
    /// 字体 Modal 开关。
    font_modal_open: bool,
    /// 多行文本域是否自动换行（false = 不换行 + 水平滚动）。
    ta_wrap: bool,
    difficulty: String,
    /// combo 选中索引（难度下拉框）。
    diff_idx: Option<u32>,
    /// list 选中索引（滚动列表）。
    list_sel: Option<u32>,
    player_name: String,
    win_a_checked: bool,
    win_b_note: String,
    /// 多行备注（TextArea 演示）。
    win_b_note_area: String,
    inventory: [bool; 9],
    // chishi（旋转 + RGBA 染色窗口；显示文本由 NumberInput 内部管理）
    cshi_num: f32,
    // —— 性能测量（--auto-drag：自动拖动 win_b 并每帧改内容，模拟"拖动中内容逐帧变化"的最坏路径） ——
    perf: PerfAgg,
    auto_drag: bool,
    /// --script-pos：位置责任链演示——脚本驱动 win_a 摆动（优先级 -10，拖拽优先）。
    script_pos: bool,
    drag_t0: Instant,
    auto_tick: u64,
}

impl UiApp {
    fn new() -> Self {
        let mut ui_state = UiState::new();
        // 默认选中"普通"难度（单选组值 = 控件**绝对 ID**；顶层无前缀 = 原样）
        ui_state
            .radio_groups
            .insert("diff".to_owned(), IdAbsolute::from("diff_normal"));
        Self {
            render: None,
            render2d: None,
            render2d_ui: None,
            font: None,
            viewport: Viewport::new(Vec2::new(1280.0, 720.0), Vec2::ZERO),
            ui_state,
            win_a_pos: Vec2::new(560.0, 240.0),
            // win_b 默认避开 win_a 右下角（缩放柄可达；仍与 win_a 右上角重叠演示置顶）
            win_b_pos: Vec2::new(760.0, 120.0),
            clicks: 0,
            volume: 0.6,
            hp: 66.0,
            hp2: 40.0,
            hp3: 60.0,
            show_hud: true,
            opt7: false,
            font_name: String::new(),
            font_input: String::new(),
            font_modal_open: false,
            ta_wrap: true,
            fullscreen: false,
            difficulty: "普通".to_owned(),
            diff_idx: Some(1),
            list_sel: None,
            player_name: "Krisu".to_owned(),
            win_a_checked: false,
            win_b_note: String::new(),
            win_b_note_area: "多行备注：\nEnter 换行，↑↓ 跨行，Home/End 行首尾，\n拖选文本后 Ctrl+C/V/X 复制/粘贴/剪切。".to_owned(),
            inventory: [false; 9],
            perf: PerfAgg::new(),
            auto_drag: false,
            script_pos: false,
            drag_t0: Instant::now(),
            auto_tick: 0,
            cshi_num: 0.,
        }
    }
}

/// 帧统计聚合：累加 `PERF_PRINT_EVERY` 帧后打印平均值（stdout），随后清零。
struct PerfAgg {
    frames: u32,
    frame_us: f64,
    ui_us: f64,
    finish_us: f64,
    render_us: f64,
    begin_us: f64,
    encode_us: f64,
    submit_us: f64,
    present_us: f64,
    sort_us: f64,
    sig_us: f64,
    collect_us: f64,
    clone_us: f64,
    submit_ui_us: f64,
    cmds: u64,
    wins: u64,
    hits: u64,
    misses: u64,
}

/// 每多少帧打印一次 [perf] 统计（165Hz 下约 0.7 秒一次）。
const PERF_PRINT_EVERY: u32 = 120;

impl PerfAgg {
    fn new() -> Self {
        Self {
            frames: 0,
            frame_us: 0.0,
            ui_us: 0.0,
            finish_us: 0.0,
            render_us: 0.0,
            begin_us: 0.0,
            encode_us: 0.0,
            submit_us: 0.0,
            present_us: 0.0,
            sort_us: 0.0,
            sig_us: 0.0,
            collect_us: 0.0,
            clone_us: 0.0,
            submit_ui_us: 0.0,
            cmds: 0,
            wins: 0,
            hits: 0,
            misses: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        s: &UiStats,
        frame_us: f64,
        render_us: f64,
        begin_us: f64,
        encode_us: f64,
        submit_us: f64,
        present_us: f64,
    ) {
        self.frames += 1;
        self.frame_us += frame_us;
        self.ui_us += s.ui_frame_us;
        self.finish_us += s.finish_us;
        self.render_us += render_us;
        self.begin_us += begin_us;
        self.encode_us += encode_us;
        self.submit_us += submit_us;
        self.present_us += present_us;
        self.sort_us += s.sort_us;
        self.sig_us += s.sig_us;
        self.collect_us += s.collect_us;
        self.clone_us += s.clone_us;
        self.submit_ui_us += s.submit_us;
        self.cmds += s.cmd_count as u64;
        self.wins += s.win_count as u64;
        self.hits += s.cache_hits as u64;
        self.misses += s.cache_misses as u64;
    }

    /// 打印近 N 帧均值（ms / µs）后清零。
    fn flush(&mut self, fps: f64) {
        let n = self.frames.max(1) as f64;
        println!(
            "[perf] fps={fps:.0} frame={:.2}ms ui={:.2}ms finish={:.2}ms \
             | ui: sort={:.1}us sig={:.1}us collect={:.1}us clone={:.1}us submit={:.1}us \
             | render: total={:.2}ms begin={:.1}us encode={:.1}us submit={:.1}us present={:.1}us \
             | cmds={:.0} wins={:.0} cache_hit={:.0} cache_miss={:.0}",
            self.frame_us / n / 1000.0,
            self.ui_us / n / 1000.0,
            self.finish_us / n / 1000.0,
            self.sort_us / n,
            self.sig_us / n,
            self.collect_us / n,
            self.clone_us / n,
            self.submit_ui_us / n,
            self.render_us / n / 1000.0,
            self.begin_us / n,
            self.encode_us / n,
            self.submit_us / n,
            self.present_us / n,
            self.cmds as f64 / n,
            self.wins as f64 / n,
            self.hits as f64 / n,
            self.misses as f64 / n,
        );
        *self = Self::new();
    }
}

impl App for UiApp {
    fn primary_window_attrib(&self) -> WindowAttributes {
        WindowAttributes::default()
            .with_title("eg260818UI — rjw_ui 示例")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
    }

    fn on_init(&mut self, ctx: &mut MainContext) {
        let window = ctx.primary_window().expect("window");
        self.render = Some(RenderContext::new(window, &RenderConfig::default()));
        let render = self.render.as_ref().unwrap();
        let render2d = Render2D::new(render);
        // 独立 UI 渲染器：**必须关闭 Render2D 排序**（set_sorting(false)）——
        // UI 自行管理绘制顺序：`finish` 按（窗口 z 升序 → 窗口内图形组 → 字形文字组）提交，
        // 每窗口 `layer = base + z*1.0` 仅作兜底；Render2D 按提交顺序原样绘制。
        // ⚠ 不要用 set_sorting(true)（LayerAndStates）：它会在同一 layer 内**按纹理 uid
        // 重排**，字形图集页先于程序化纹理页（圆角/渐变）注册 → 圆角/渐变会盖住文字。
        let mut render2d_ui = Render2D::new(render);
        render2d_ui.set_sorting(false);
        let font = Text::new(render2d.device(), render2d.queue(), render2d.tex_bind_group_layout());
        let (w, h) = render.size();
        let viewport = Viewport::new(Vec2::new(w as f32, h as f32), Vec2::ZERO);
        self.render2d = Some(render2d);
        self.render2d_ui = Some(render2d_ui);
        self.font = Some(font);
        self.viewport = viewport;
    }

    fn on_resized(&mut self, _ctx: &mut MainContext, width: u32, height: u32) {
        if let Some(r) = &mut self.render {
            r.resize(width, height);
        }
        self.viewport = Viewport::new(Vec2::new(width as f32, height as f32), Vec2::ZERO);
    }

    fn about_to_wait(&mut self, ctx: &mut MainContext) {
        let t_frame = Instant::now();
        // ── 应用快捷键：**输入框聚焦时屏蔽**（capturing_text）——
        //    输入 `R` / `Esc` 不会被当作重置 / 退出。
        if !self.ui_state.capturing_text() {
            if ctx.keyboard.get(KeyCode::Escape).down_edge() {
                ctx.request_exit();
            }
            if ctx.keyboard.get(KeyCode::KeyR).down_edge() {
                reset_ui_state(&mut self.ui_state);
            }
        }

        // 性能测量：--auto-drag 自动拖动 win_b（圆周轨迹）+ 每帧改内容
        // （等价"拖动中 hover/光标闪烁/滚动"的逐帧内容变化 → 走缓存未命中重建路径）。
        if self.auto_drag {
            let t = self.drag_t0.elapsed().as_secs_f64();
            self.win_b_pos = Vec2::new(
                640.0 + 220.0 * (t * 0.7).sin() as f32,
                330.0 + 140.0 * (t * 1.1).cos() as f32,
            );
            self.auto_tick = self.auto_tick.wrapping_add(1);
        }

        let Some(r2d) = &mut self.render2d else {
            return;
        };
        r2d.set_mvp(self.viewport.vp_matrix());
        let font = self.font.as_mut().unwrap();

        // ── 世界层：几个背景方块（在 UI 之下） ─────────────────
        let world_tf = Transform2D::default();
        r2d.add_sprite2d_solid(
            SpriteRect::from_texture(Vec2::new(-640.0, -360.0), Vec2::new(1280.0, 720.0)),
            Color::rgba_u8(30, 36, 48, 255),
            world_tf,
            0.0,
        );
        for i in 0..6 {
            let x = -560.0 + i as f32 * 220.0;
            r2d.add_sprite2d_solid(
                SpriteRect::from_texture(Vec2::new(x, -280.0 + (i % 2) as f32 * 160.0), Vec2::new(160.0, 90.0)),
                Color::rgba_u8(40 + i * 20, 60, 90, 255),
                world_tf,
                1.0,
            );
        }

        // ── UI 层：录制到独立 Render2D（关闭排序） ─────────────
        let r2d_ui = self.render2d_ui.as_mut().unwrap();
        r2d_ui.set_mvp(self.viewport.vp_matrix());
        let window = ctx.primary_window().expect("window");
        // 窗口诊断（调试机制）：`UiState` 的跨帧诊断数据须在 `Ui::begin` **之前**读取
        // （begin 会借用 ui_state）——上一帧 finish 写入的值本帧显示。
        let prev_press = self
            .ui_state
            .last_press_window()
            .map(|(id, z)| format!("{id} (z{z})"))
            .unwrap_or_else(|| "无".to_owned());
        let prev_blocked = self.ui_state.occluded_hits();
        // 渲染增强演示：圆角已修复（见 proc::rounded_rect_rgba 非整数半径回归测试）——
        // 主题责任链 `with_radius` 级联到 panel / button / input，高 DPI 下也不再破损。
        // 主题按所选字体构建（FontModal 确定后写入 font_name；空 = 系统默认）。
        // with 责任链：全局字体族级联到全部文本子样式 + 全局圆角。
        let theme = if self.font_name.is_empty() {
            Theme::dark()
        } else {
            Theme::dark().with_font_family(&self.font_name)
        }
        .with_radius(8.0);
        let mut ui = Ui::begin(window, font, &mut self.ui_state)
            .capture(&ctx.mouse, &ctx.keyboard)
            .theme(theme)
            .base_layer(LAYER_UI)
            // DPI 缩放：控件坐标/字号按逻辑像素，内部换算物理像素
            .scale_factor(ctx.scale_factor().unwrap_or(1.0))
            .build();

        // ── 位置责任链演示（--script-pos）：脚本让窗口 A 沿正弦摆动 ──
        // 处理器优先级 -10（< 0）：**用户拖拽优先**——拖住 A 时脚本让位、窗口跟手，
        // 松开后停在放置处；不拖时脚本每帧驱动位置（脚本"动画"，拖动"覆盖"）。
        if self.script_pos {
            let t0 = self.drag_t0; // Instant: Copy，闭包只捕获时间基准（不借 self）
            ui.pos_handler(-10, move |id| {
                if id == "win_a" {
                    let t = t0.elapsed().as_secs_f64();
                    Some(Vec2::new(
                        560.0 + 260.0 * (t * 0.5).sin() as f32,
                        240.0 + 120.0 * (t * 0.9).cos() as f32,
                    ))
                } else {
                    None
                }
            });
        }

        // ── place：顶部状态栏（渐变背景 + 圆角原语演示） ────────
        // ui.gradient_rect_at(
        //     Vec2::new(0.0, 0.0),
        //     Vec2::new(1280.0, 56.0),
        //     rjw_ui::GradientAxis::Horizontal,
        //     vec![
        //         (0.0, Color::rgba_u8(38, 52, 90, 255)),
        //         (1.0, Color::rgba_u8(26, 34, 60, 255)),
        //     ],
        // );
        ui.label_at(Vec2::new(16.0, 12.0), &format!("FPS: {:.0}", ctx.timer.get_fps()));
        ui.label_at(Vec2::new(16.0, 34.0), &format!("点击次数: {}", self.clicks));
        // ── 字体切换（Modal 窗口：Input + PreviewInput + 确定/取消右对齐）──
        // "字体…"按钮（**pack 内自动尺寸**：随字体名变长自动变宽）打开 Modal；
        // 确定后写入 font_name，下一帧主题按它重建。
        ui.pack_at(Vec2::new(16.0, 56.0), PackSide::Top, |p| {
            if p.button("font_btn", &format!("字体… {}", self.font_name)).clicked() {
                self.font_modal_open = true;
            }
        });
        ui.drag_panel_at("name_panel", Vec2::new(200.0, 12.0), |p| {
            p.label("玩家名（可拖动）");
            p.text_input("name", &mut self.player_name);
        });

        // ── pack：左侧主菜单 ──────────────────────────────────
        // 注意：闭包内不可触碰 `self.ui_state`（已被 `ui` 借用），
        // 重置请求先记录到局部标记，`ui.finish()` 后统一处理。
        let mut reset_requested = false;
        ui.pack_at(Vec2::new(16.0, 90.0), PackSide::Top, |p| {
            p.label("主菜单");
            // 新控件 API（Widget trait + 属性化 builder，见 rjw_ui::widget）：
            // `p.add(…)` 占光标，属性逐控件覆盖主题（文本色/背景/圆角等），
            // 旧 `p.button(…)` API 仍可用。
            if p
                .add(
                    Button::new("btn_start", "开始游戏")
                        .color(Color::WHITE)
                        .bg(Color::rgba_u8(52, 120, 200, 255))
                        .bg_hover(Color::rgba_u8(70, 140, 220, 255))
                        .bg_pressed(Color::rgba_u8(40, 95, 165, 255))
                        .radius(6.0),
                )
                .clicked()
            {
                self.clicks += 1;
            }
            p.add(
                Label::new("样式标签：蓝色 16px")
                    .color(Color::rgba_u8(96, 160, 235, 255))
                    .font_size(16.0),
            );
            if p.button("btn_reset", "重置 UI 状态 (R)").clicked() {
                reset_requested = true;
            }
            // 滑块 builder（链式拖拽精度 + Shift/Ctrl 速度；占光标 add）：
            // 拖拽精度 = 每像素数值倍率；Shift 按住 ×10、Ctrl 按住 ×0.1。
            p.add(
                Slider::new("vol", 0.0..=1.0, &mut self.volume)
                    .drag_sensitivity(1.0)
                    .shift_speed(10.0)
                    .ctrl_speed(0.1),
            );
            p.label(&format!("音量: {:.0}%", self.volume * 100.0));
            // ── 新功能演示：数字条（builtin 组合控件） + WidgetId 数字 ID ──
            p.label("生命值（数字条：拖动手柄左右调值 / 点击输入）");
            p.add(NumberInput::new("hp_bar", &mut self.hp).range(0.0, 100.0).step(0.25));
            p.label(&format!("HP: {:.2}", self.hp));
            if p.checkbox_mut(Some("cb_hud"), "显示 HUD", &mut self.show_hud).toggled() {
                // checkbox_mut 点击已直接翻转 `&mut bool`；此处演示返回状态仍可判断
            }
            p.checkbox_mut(7u64, "选项 7（数字 ID）", &mut self.opt7); // id = WidgetId::Int(7)
            if p.checkbox("fs", "全屏", self.fullscreen).toggled() {
                self.fullscreen = !self.fullscreen;
            }
            p.label("难度");
            // combo 下拉框（难度选择）：展开浮层选一项，点击外部收起
            const DIFFS: [&str; 3] = ["简单", "普通", "困难"];
            let diff_opts: Vec<String> = DIFFS.iter().map(|s| s.to_string()).collect();
            if let Some(i) = p.combo("diff_combo", &self.difficulty, &diff_opts, self.diff_idx) {
                self.diff_idx = Some(i);
                self.difficulty = DIFFS[i as usize].to_owned();
            }
            p.label(&format!("难度: {}", self.difficulty));
            p.label(&format!(
                "全屏: {}",
                if self.fullscreen { "开" } else { "关" }
            ));
            // ── 布局增强演示：换行 + min/max 尺寸约束 ──────────
            p.label("尺寸约束（min 160 / max 120）");
            p.min_size(160.0, 0.0);
            if p.button("btn_min", "min 宽").clicked() {
                self.clicks += 1;
            }
            p.max_size(120.0, 0.0);
            if p.button("btn_max", "max 宽").clicked() {
                self.clicks += 1;
            }
            p.label_wrap(180.0, "自动换行标签：pack 内 180 宽自动换行成多行，适合说明文字。");
            // ── 水平行（row）：{Label} {NumberInput} {NumberInput} {Button} 占一行 ──
            p.divider();
            p.label("水平行（row）：数字条 ×2 + 按钮");
            p.row(|r| {
                r.label("HP:");
                // NumberInput 新 API：只需数值引用（显示文本内部跨帧持久管理）。
                r.add(NumberInput::new("hp_row_a", &mut self.hp2).range(0.0, 100.0).step(0.25));
                r.add(NumberInput::new("hp_row_b", &mut self.hp3).range(0.0, 100.0).step(0.25));
                if r.button("hp_row_btn", "同步").clicked() {
                    // 同步只写值即可——NumberInput 失焦显示由 value 派生，自动跟随。
                    self.hp3 = self.hp2;
                }
            });
            // ── 分割线（占光标；宽 = 容器可用宽 / 当前最宽子项） ──
            p.divider();
            p.label("分割线下方的段落……");
        });

        // ── 可拖拽面板 + grid：右侧背包 ───────────────────────
        // 容器责任链 builder：`ui.window(id)` → 选项链 → `.show(f)`（等价旧 window_at）。
        ui.window("inv_panel")
            .pos(Vec2::new(300.0, 90.0))
            .show(|p| {
                p.label("背包（按住拖动 · 点击切换物品）");
                p.grid_at(Vec2::new(0.0, 28.0), 3, "inv", |g| {
                    for i in 0..9 {
                        let owned = self.inventory[i];
                        let label = if owned { format!("物品 {i} ★") } else { format!("物品 {i}") };
                        if g.button(&format!("slot_{i}"), &label).clicked() {
                            self.inventory[i] = !self.inventory[i];
                        }
                    }
                });
            });

        // ── Window 容器：可重叠 + 点击置顶 + 可拖拽 ───────────
        // 两个窗口互相重叠；点击任一窗口即置顶（焦点 z-order）。
        // 窗口 A 用**固定宽**（builder `.width()`，等价旧 `window_at_w`：右下角缩放柄，
        // 鼠标拖动改宽度、高度自动）+ **逐窗口样式**（`.style()`：圆角深色面板，
        // 默认回落全局 `Theme::panel`）。
        // 缩窄窗口 A：Label 自动换行 / `.ellipsis()` 省略 / 按钮文本省略。
        ui.window("win_a")
            .pos(self.win_a_pos)
            .width(220.0)
            .style(
                PanelStyle::default()
                    .with_bg(Color::rgba_u8(40, 44, 62, 255))
                    .with_radius(8.0),
            )
            // 位置约束责任链：默认 Screen = 窗口整体不跑出屏幕；Free = 可拖出；
            // Locked = 位置固定（拖不动，脚本/传入位置仍生效）。
            .clamp(WindowClamp::Screen)
            .show(|w| {
                w.label("窗口 A（点击置顶 · 拖动移动）");
                if w.button("win_a_btn", "A 按钮").clicked() {
                    self.clicks += 1;
                }
                // 勾选状态由应用持有（跨帧持久）：checkbox_mut 点击**直接翻转** `&mut bool`，
                // 无需手动 toggled() 维护；ID = None → 以 label 文本为 ID。
                w.checkbox_mut(None, "窗口 A 选项", &mut self.win_a_checked);
                // Label 溢出演示（Resizable 窗口缩窄）：
                // - 默认（无显式 wrap）：在窗口固定宽内**自动换行**；
                // - `.ellipsis()`：超出宽度以 "…" 省略为单行。
                w.add(Label::new("自动换行标签：窗口缩窄后自动换行，不再溢出画到窗口外。"));
                w.add(Label::new("省略标签：窗口缩窄后显示为省略号……").ellipsis());
                w.add(Divider::new());
                w.label("分割线下方");
            });
        ui.window_at("win_b", self.win_b_pos, |w| {
            w.label("窗口 B（覆盖在 A 之上）");
            // 性能测量：auto_drag 时每帧变化的标签（强制窗口内容每帧变化 → 重建路径）
            w.label(&format!("帧序号 {}", self.auto_tick % 1000));
            if w.button("win_b_btn", "B 按钮").clicked() {
                self.clicks += 1;
            }
            // 输入内容写入应用状态（跨帧持久），聚焦后打字生效 ✓
            w.text_input("win_b_input", &mut self.win_b_note);
            // 多行 TextArea：Enter 换行、拖选 + Ctrl+C/V/X。两种模式：
            // 自动换行（按内容区宽换行）/ 不自动换行（行宽不限 + 水平滚动）。
            w.checkbox_mut(Some("ta_wrap"), "自动换行", &mut self.ta_wrap);
            w.label("多行备注（Enter 换行 · 双击按词选择 · 拖选复制粘贴）");
            if self.ta_wrap {
                w.text_area("win_b_note_area", &mut self.win_b_note_area);
            } else {
                w.text_area_nw("win_b_note_area", &mut self.win_b_note_area);
            }
        });

        // ── 严格裁剪窗口（window_at_strict）：内容超出窗口被强制裁剪（Clip 沙箱）──
        // 对照 win_a 的 Expand 语义（内容自动换行/撑高窗口）。
        ui.window_at_strict("strict_win", Vec2::new(560.0, 460.0), |w| {
            w.label("严格裁剪窗口（内容超出被裁）");
            w.add(Label::new(
                "这一段文字足够长，会超出严格窗口的可视区——超出部分被强制裁剪，\
                 不再撑大窗口；滚动容器 / 文本编辑框同为 Clip 语义。",
            ));
            if w.button("strict_btn", "被裁窗口按钮").clicked() {
                self.clicks += 1;
            }
        });
        
        let mut r = 1.0;
        let mut g = 1.0;
        let mut b = 1.0;
        let mut a = 1.0;
        ui.window_at("chishi", vec2(155., 32.), |w| {
            w.label("赤石");
            w.add(NumberInput::new("chisN1", &mut self.cshi_num).step(0.1));
            self.cshi_num = w.slider("sb", 0.0..=360., self.cshi_num);
            w.row(|w| {
                w.label("HP:");
                r = w.slider("CSHI_r", 0.0..=1.0, r);
                g = w.slider("CSHI_g", 0.0..=1.0, g);
                b = w.slider("CSHI_b", 0.0..=1.0, b);
                a = w.slider("CSHI_a", 0.0..=1.0, a);
            });
        } );
        // 赤石窗口：整窗旋转（角度 = cshi_num）+ RGBA 染色（4 个 slider 调）
        // —— window_fx 演示：顶点缓存不变，仅提交时应用 tint/transform。
        // `anchor = (0.5, 0.5)`：旋转/缩放绕**窗口中心**（transform = IDENTITY 时
        // 无论锚点何值，位置恒为窗口原位置）。
        ui.window_fx("chishi", WindowFx {
            tint: Color::rgba(r, g, b, a),
            transform: Some(Transform2D::IDENTITY.with_rot(self.cshi_num.to_radians())),
            anchor: Vec2::new(0.5, 0.5),
        });

        // ── 窗口级 FX（window_fx）：win_b 整窗淡入淡出 + 轻微上浮动画 ──
        // tint（混合色，顶点色×实例色）与 transform override（叠加在窗口原点上）——
        // 顶点缓存不变，仅提交时应用，支撑整窗口动画/特效。
        let fx_t = self.drag_t0.elapsed().as_secs_f64();
        let fx_alpha = 0.75 + 0.25 * (fx_t * 1.5).sin() as f32;
        ui.window_fx(
            "win_b",
            WindowFx {
                tint: Color::rgba_u8(255, 255, 255, (fx_alpha * 255.0) as u8),
                transform: Some(
                    Transform2D::IDENTITY.with_pos(Vec2::new(0.0, 5.0 * (fx_t * 1.2).sin() as f32)),
                ),
                anchor: Vec2::new(0.5, 0.5), // 旋转/缩放绕窗口中心
            },
        );

        // ── 窗口诊断面板（调试机制：实时告诉你窗口叠放与点击解析）──
        // 把窗口 A/B/背包叠在同一处点击：面板会显示鼠标下**最上层**窗口是哪个、
        // 上次按下由哪个窗口接收、以及有多少次控件命中被遮挡拦截——
        // 重叠区域只有最上层窗口的控件会响应（点击穿透已修复）。
        let order: String = ui
            .window_order()
            .into_iter()
            .map(|(id, z)| format!("{id}(z{z})"))
            .collect::<Vec<_>>()
            .join(" ");
        let under = ui
            .window_under_mouse()
            .map(|(id, z)| format!("{id} (z{z})"))
            .unwrap_or_else(|| "无".to_owned());
        ui.label_at(
            Vec2::new(880.0, 12.0),
            &format!(
                "窗口 z 序: {}\n鼠标下最上层: {}\n上次按下接收: {}（上帧）\n被遮挡拦截: {}（上帧）",
                if order.is_empty() { "无" } else { &order },
                under,
                prev_press,
                prev_blocked,
            ),
        );

        // ── 滚动容器演示：可滚动选择列表（list_at：滚轮 / 滚动条 + 选中态）──
        let sel = ui.list_at(
            Vec2::new(880.0, 130.0),
            Vec2::new(240.0, 300.0),
            "list_demo",
            40,
            self.list_sel,
            |s, i, is_sel| {
                let label = format!("{}条目 {i}", if is_sel { "✓ " } else { "" });
                s.button(&format!("log_{i}"), &label).clicked()
            },
        );
        if let Some(i) = sel {
            self.list_sel = Some(i);
            self.clicks += 1;
        }

        // ── 布局增强演示：flex 权重（固定高 150，[1:2:1] 等分） ──
        ui.flex_at(vec2(880.0, 450.0), 150.0, &[1, 2, 1], |f, i| {
            if f.button(&format!("flex_row_{i}"), &format!("行 {i} · 权重 {}", [1, 2, 1][i]))
                .clicked()
            {
                self.clicks += 1;
            }
        });

        // ── place：底部说明（**锚定视口左下角**——不再被窗口遮挡） ──
        let hint = "Tab/方向键 遍历焦点 · 输入框拖选文本 + Ctrl+C/V/X · 双击按词选择 · Enter 换行（多行） · 滚轮滚动（指针在框内） · Esc 收起/失焦（再按退出） · R 重置";
        let (hint_fs, hint_ff) = (ui.theme.label.font_size, ui.theme.label.font_family.clone());
        let hint_size = ui.text_size(hint, hint_fs, hint_ff.as_deref());
        let hint_pos = ui.anchor_pos(Anchor::BottomLeft, hint_size, Vec2::new(16.0, 16.0));
        ui.label_at(hint_pos, hint);

        // ── 字体 Modal（**帧末录制**：modal 的 z 每帧重写为当前最大，最后录制
        //    才能保证不被本帧后录的窗口盖住——见 modal_at 文档）──
        if self.font_modal_open {
            FontModal {
                input: &mut self.font_input,
                apply: &mut |name: &str| {
                    self.font_name = name.to_owned();
                },
            }
            .show(&mut ui, &mut self.font_modal_open);
        }

        ui.finish(&self.viewport, r2d_ui);

        // 重置请求（ui 借用已随 finish 结束，可安全触碰 ui_state）
        if reset_requested {
            reset_ui_state(&mut self.ui_state);
        }
        // 性能统计（上一帧 finish 写入的 UI 各阶段耗时）
        let ui_stats = self.ui_state.stats.clone();

        // ── 合并提交：世界 → UI → 一次 present ────────────────
        // 世界用 render_command_buffer（不 present），UI 叠加在同一 surface 视图，
        // 两个 command buffer 一次 submit + 一次 present——UI 覆盖世界且无额外延迟。
        let t_render = Instant::now();
        let Some((surface_tex, view)) = r2d.begin_frame() else {
            return;
        };
        let begin_us = t_render.elapsed().as_secs_f64() * 1e6;
        let t_enc = Instant::now();
        let cb_world = r2d.render_command_buffer(
            &ClearConfig {
                color: Some(wgpu::Color { r: 0.09, g: 0.11, b: 0.16, a: 1.0 }),
                depth: None,
                stencil: None,
            },
            &view,
            None,
        );
        let cb_ui = r2d_ui.render_command_buffer(
            &ClearConfig { color: None, depth: None, stencil: None },
            &view,
            None,
        );
        let encode_us = t_enc.elapsed().as_secs_f64() * 1e6;
        let t_sub = Instant::now();
        r2d.queue().submit([cb_world, cb_ui]);
        let submit_us = t_sub.elapsed().as_secs_f64() * 1e6;
        let t_pr = Instant::now();
        r2d.queue().present(surface_tex);
        let present_us = t_pr.elapsed().as_secs_f64() * 1e6;
        let render_us = begin_us + encode_us + submit_us + present_us;
        // 性能统计：整帧 / 渲染（细分）/ UI 各阶段（每 PERF_PRINT_EVERY 帧打印一次）
        let frame_us = t_frame.elapsed().as_secs_f64() * 1e6;
        self.perf
            .add(&ui_stats, frame_us, render_us, begin_us, encode_us, submit_us, present_us);
        if self.perf.frames >= PERF_PRINT_EVERY {
            self.perf.flush(ctx.timer.get_fps());
        }
    }
}

/// 解析 `--win-a X,Y --win-b X,Y` 命令行参数（RenderDoc 重叠次序验证用）。
fn parse_pos_arg(args: &[String], key: &str, default: Vec2) -> Vec2 {
    let mut out = default;
    let mut i = 0;
    while i < args.len() {
        if args[i] == key && i + 1 < args.len() {
            if let Some((x, y)) = args[i + 1].split_once(',') {
                if let (Ok(x), Ok(y)) = (x.trim().parse(), y.trim().parse()) {
                    out = Vec2::new(x, y);
                }
            }
        }
        i += 1;
    }
    out
}

fn main() -> Result<(), EventLoopError> {
    let args: Vec<String> = std::env::args().collect();
    let mut app = UiApp::new();
    app.win_a_pos = parse_pos_arg(&args, "--win-a", app.win_a_pos);
    app.win_b_pos = parse_pos_arg(&args, "--win-b", app.win_b_pos);
    app.auto_drag = args.iter().any(|a| a == "--auto-drag");
    app.script_pos = args.iter().any(|a| a == "--script-pos");
    run_app(app)
}

/// 重置 UI 状态并恢复默认选中"普通"难度。
fn reset_ui_state(state: &mut UiState) {
    state.reset();
    state
        .radio_groups
        .insert("diff".to_owned(), IdAbsolute::from("diff_normal"));
}
