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

use glam::Vec2;
use rjw_2d_render::{ClearConfig, Render2D, SpriteRect};
use rjw_color::Color;
use rjw_main::*;
use rjw_render::{wgpu, RenderConfig, RenderContext};
use rjw_text::Text;
use rjw_transform::{Camera2D, Transform2D};
use rjw_ui::{PackSide, Theme, Ui, UiState};

const LAYER_UI: f64 = 10_000_000.0;

struct UiApp {
    render: Option<RenderContext>,
    render2d: Option<Render2D>,
    render2d_ui: Option<Render2D>,
    font: Option<Text>,
    cam: Camera2D,
    ui_state: UiState,
    // 窗口位置（支持命令行参数覆盖，便于 RenderDoc 验证重叠次序）
    win_a_pos: Vec2,
    win_b_pos: Vec2,
    // 演示状态（由 UI 控件驱动）
    clicks: u32,
    volume: f32,
    fullscreen: bool,
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
}

impl UiApp {
    fn new() -> Self {
        let mut ui_state = UiState::new();
        // 默认选中"普通"难度
        ui_state.radio_groups.insert("diff".to_owned(), "diff_normal".to_owned());
        Self {
            render: None,
            render2d: None,
            render2d_ui: None,
            font: None,
            cam: Camera2D::default(),
            ui_state,
            win_a_pos: Vec2::new(560.0, 240.0),
            win_b_pos: Vec2::new(640.0, 330.0),
            clicks: 0,
            volume: 0.6,
            fullscreen: false,
            difficulty: "普通".to_owned(),
            diff_idx: Some(1),
            list_sel: None,
            player_name: "Krisu".to_owned(),
            win_a_checked: false,
            win_b_note: String::new(),
            win_b_note_area: "多行备注：\nEnter 换行，↑↓ 跨行，Home/End 行首尾，\n拖选文本后 Ctrl+C/V/X 复制/粘贴/剪切。".to_owned(),
            inventory: [false; 9],
        }
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
        let mut cam = Camera2D::new(Vec2::new(w as f32, h as f32));
        cam.set_vp(Vec2::new(w as f32, h as f32), Vec2::ZERO);
        self.render2d = Some(render2d);
        self.render2d_ui = Some(render2d_ui);
        self.font = Some(font);
        self.cam = cam;
    }

    fn on_resized(&mut self, _ctx: &mut MainContext, width: u32, height: u32) {
        if let Some(r) = &mut self.render {
            r.resize(width, height);
        }
        self.cam.set_vp(Vec2::new(width as f32, height as f32), Vec2::ZERO);
    }

    fn about_to_wait(&mut self, ctx: &mut MainContext) {
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

        let Some(r2d) = &mut self.render2d else {
            return;
        };
        r2d.set_mvp(self.cam.vp_matrix());
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
        r2d_ui.set_mvp(self.cam.vp_matrix());
        let window = ctx.primary_window().expect("window");
        // 窗口诊断（调试机制）：`UiState` 的跨帧诊断数据须在 `Ui::begin` **之前**读取
        // （begin 会借用 ui_state）——上一帧 finish 写入的值本帧显示。
        let prev_press = self
            .ui_state
            .last_press_window()
            .map(|(id, z)| format!("{id} (z{z})"))
            .unwrap_or_else(|| "无".to_owned());
        let prev_blocked = self.ui_state.occluded_hits();
        // 渲染增强演示：渐变状态栏背景（圆角样式已弃用——默认 radius=0 方形路径，
        // 不再生成程序化圆角纹理）。
        let theme = Theme::dark();
        let mut ui = Ui::begin(window, &self.cam, &ctx.mouse, &ctx.keyboard, font, r2d_ui, &mut self.ui_state)
            .theme(theme)
            .base_layer(LAYER_UI)
            // DPI 缩放：控件坐标/字号按逻辑像素，内部换算物理像素
            .scale_factor(ctx.scale_factor().unwrap_or(1.0))
            .build();

        // ── place：顶部状态栏（渐变背景 + 圆角原语演示） ────────
        ui.gradient_rect_at(
            Vec2::new(0.0, 0.0),
            Vec2::new(1280.0, 56.0),
            rjw_ui::GradientAxis::Horizontal,
            vec![
                (0.0, Color::rgba_u8(38, 52, 90, 255)),
                (1.0, Color::rgba_u8(26, 34, 60, 255)),
            ],
        );
        ui.label_at(Vec2::new(16.0, 12.0), &format!("FPS: {:.0}", ctx.timer.get_fps()));
        ui.label_at(Vec2::new(16.0, 34.0), &format!("点击次数: {}", self.clicks));
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
            if p.button("btn_start", "开始游戏").clicked() {
                self.clicks += 1;
            }
            if p.button("btn_reset", "重置 UI 状态 (R)").clicked() {
                reset_requested = true;
            }
            // 滑块：返回更新后的值，写入应用状态
            self.volume = p.slider("vol", 0.0..=1.0, self.volume);
            p.label(&format!("音量: {:.0}%", self.volume * 100.0));
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
        });

        // ── 可拖拽面板 + grid：右侧背包 ───────────────────────
        ui.window_at("inv_panel", Vec2::new(300.0, 90.0), |p| {
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
        ui.window_at("win_a", self.win_a_pos, |w| {
            w.label("窗口 A（点击置顶 · 拖动移动）");
            if w.button("win_a_btn", "A 按钮").clicked() {
                self.clicks += 1;
            }
            // 勾选状态由应用持有（跨帧持久），点击切换 ✓
            if w.checkbox("win_a_cb", "窗口 A 选项", self.win_a_checked).toggled() {
                self.win_a_checked = !self.win_a_checked;
            }
        });
        ui.window_at("win_b", self.win_b_pos, |w| {
            w.label("窗口 B（覆盖在 A 之上）");
            if w.button("win_b_btn", "B 按钮").clicked() {
                self.clicks += 1;
            }
            // 输入内容写入应用状态（跨帧持久），聚焦后打字生效 ✓
            w.text_input("win_b_input", &mut self.win_b_note);
            // 多行 TextArea：Enter 换行、自动换行 + 垂直滚动、拖选 + Ctrl+C/V/X
            w.label("多行备注（Enter 换行 · 拖选复制粘贴）");
            w.text_area("win_b_note_area", &mut self.win_b_note_area);
        });

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
        ui.flex_at(Vec2::new(880.0, 450.0), 150.0, &[1, 2, 1], |f, i| {
            if f.button(&format!("flex_row_{i}"), &format!("行 {i} · 权重 {}", [1, 2, 1][i]))
                .clicked()
            {
                self.clicks += 1;
            }
        });

        // ── place：底部说明 ───────────────────────────────────
        ui.label_at(
            Vec2::new(16.0, 690.0),
            "Tab/方向键 遍历焦点 · 输入框拖选文本 + Ctrl+C/V/X · Enter 换行（多行） · Esc 收起/失焦（再按退出） · R 重置",
        );

        ui.finish();

        // 重置请求（ui 借用已随 finish 结束，可安全触碰 ui_state）
        if reset_requested {
            reset_ui_state(&mut self.ui_state);
        }

        // ── 合并提交：世界 → UI → 一次 present ────────────────
        // 世界用 render_command_buffer（不 present），UI 叠加在同一 surface 视图，
        // 两个 command buffer 一次 submit + 一次 present——UI 覆盖世界且无额外延迟。
        let Some((surface_tex, view)) = r2d.begin_frame() else {
            return;
        };
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
        r2d.queue().submit([cb_world, cb_ui]);
        r2d.queue().present(surface_tex);
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
    run_app(app)
}

/// 重置 UI 状态并恢复默认选中"普通"难度。
fn reset_ui_state(state: &mut UiState) {
    state.reset();
    state
        .radio_groups
        .insert("diff".to_owned(), "diff_normal".to_owned());
}
