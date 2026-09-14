//! eg260818UI —— `rjw_ui` 示例：DOM 风格自动布局 + Tkinter 几何管理器（pack / grid / place）。
//!
//! 展示：
//! - **独立 UI 层**：`rjw_krusie::runtime` 各持一个世界层与一个 UI 层渲染器（UI 层排序由
//!   运行时关闭——`SortMode::None`，UI 自行管理绘制顺序：窗口按 z 提交、窗口内
//!   "背景/图形 → 文字"），两层的录制进**同一个 pass**（一次 `f.submit(..)` 提交 + present）
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
//! - **可调整大小 TextArea / 宽度 TextInput**：右下角缩放柄拖拽，尺寸责任链可覆盖
//!
//! 操作：鼠标点击 / 拖拽 · 键盘 Tab/方向键/Enter/Space/Esc · 输入框打字（IME 已支持，
//! Enter / Esc 失焦） · `R` 重置 UI 状态 · `Esc`（先失焦）退出
//!
//! # 代码组织
//!
//! 为避免 `UiApp` 结构体 / `new()` / `update()` 过于杂乱，把演示状态与各区域
//! 绘制按**模块拆成独立 struct**，每个 struct 自持状态并提供一个 `ui(&mut self, &mut Ui, …)`
//! 方法（`update()` 内的 `f.ui(..)` 闭包里依次调用）：
//! - [`TopBar`]：顶部状态栏（FPS / 点击次数标签 + 字体按钮 + 玩家名输入框 + 字体 Modal）；
//! - [`Menu`]：左侧主菜单（pack：按钮 / 滑块 / 数字条 / 勾选框 / 下拉 / 布局约束 / 水平行）；
//! - [`Inventory`]：可拖拽面板 + 3 列背包 grid；
//! - [`Windows`]：可重叠 / 置顶 / 拖拽窗口 + 整窗 FX + 可调整大小文本输入框；
//! - [`RightPanel`]：右侧窗口诊断 + 滚动列表 + flex 权重 + 底部说明。
//!
//! 运行时（`rjw_krusie::runtime`）接管驱动/生命周期：窗口、渲染上下文、世界层与 UI 层
//! 渲染器、每帧 `Ui::begin` / 输入快照 / 主题 / DPI / `Ui::finish`。应用侧只剩
//! `config` / `init` / `update`（+ 可选的 `resized`）。
//!
//! 跨帧共享的全局状态（点击计数、相机、性能测量、`--auto-drag` / `--script-pos`
//! 开关）仍留在 [`UiApp`]，需要时以 `&mut` 参数传给各模块的 `ui()`。

use std::time::Instant;

use rjw_krusie::prelude::*;
// prelude 未含的 UI 类型（`rjw_ui` 公共导出；prelude 的 UI 子集见 `rjw_krusie::prelude`）。
use rjw_krusie::ui::{FontModal, IdAbsolute, Label, Palette};

/// 顶部状态栏模块：FPS / 点击次数标签 + 字体按钮（打开 Modal）+ 玩家名输入框 + 字体 Modal。
struct TopBar {
    /// 玩家名输入框内容（跨帧持久）。
    player_name: String,
    /// 当前应用的字体族（空 = 系统默认；FontModal 确定后写入，下一帧主题按它重建）。
    font_name: String,
    /// 字体 Modal 输入框内容（跨帧持久）。
    font_input: String,
    /// 字体 Modal 开关。
    font_modal_open: bool,
}

impl TopBar {
    fn new() -> Self {
        Self {
            player_name: "Krisu".to_owned(),
            font_name: String::new(),
            font_input: String::new(),
            font_modal_open: false,
        }
    }

    /// 当前字体族（空 = 系统默认；供主题构建读取）。
    fn font_name(&self) -> &str {
        &self.font_name
    }

    /// 顶部 place 区：状态标签 + 字体按钮 + 玩家名可拖动面板。
    fn ui(&mut self, ui: &mut Ui, fps: f64, clicks: u32, tuner: &mut ThemeTuner) {
        ui.label_at(Vec2::new(16.0, 12.0), &format!("FPS: {fps:.0}"));
        ui.label_at(Vec2::new(16.0, 34.0), &format!("点击次数: {clicks}"));
        // 字体按钮 +「主题调节…」开关。**放在同一 row 里**（而不是 pack 两行）：
        // 主菜单（`Menu::ui`）从 `(16, 90)` 往下堆，这里占满 `y = 56..82` 一行正好
        // 留出 8px 间隙；叠成两行会压到菜单上。
        ui.pack_at(Vec2::new(16.0, 56.0), PackSide::Top, |p| {
            p.row(|r| {
                if r.button("font_btn", &format!("字体… {}", self.font_name)).clicked() {
                    self.font_modal_open = true;
                }
                if r.button("theme_btn", "主题调节…").clicked() {
                    tuner.open = !tuner.open;
                }
            });
        });
        // 玩家名可拖动面板（右移：上面那行按钮随字体名变长，别压到它）。
        ui.drag_panel_at("name_panel", Vec2::new(430.0, 12.0), |p| {
            p.label("玩家名（可拖动）");
            p.text_input("name", &mut self.player_name);
        });
    }

    /// 字体 Modal（**帧末调用**：modal 的 z 每帧重写为当前最大，最后录制才能保证
    /// 不被本帧后录的窗口盖住——见 `modal_at` 文档）。
    fn show_font_modal(&mut self, ui: &mut Ui) {
        if self.font_modal_open {
            FontModal {
                input: &mut self.font_input,
                apply: &mut |name: &str| self.font_name = name.to_owned(),
            }
            .show(ui, &mut self.font_modal_open);
        }
    }
}

/// 左侧主菜单模块（pack）：按钮 / 滑块 / 数字条 / 勾选框 / 单选下拉 / 布局约束 / 水平行。
struct Menu {
    volume: f32,
    /// 数字输入（数字条）值。
    hp: f32,
    /// 水平行（row）演示的第二/三个数字条状态。
    hp2: f32,
    hp3: f32,
    /// checkbox_mut（WidgetId）演示状态。
    show_hud: bool,
    opt7: bool,
    fullscreen: bool,
    difficulty: String,
    /// combo 选中索引（难度下拉框）。
    diff_idx: Option<u32>,
    /// 本帧是否请求重置 UI 状态（`Frame::ui` 闭包末尾由 `update` 统一处理）。
    reset_requested: bool,
}

impl Menu {
    fn new() -> Self {
        Self {
            volume: 0.6,
            hp: 66.0,
            hp2: 40.0,
            hp3: 60.0,
            show_hud: true,
            opt7: false,
            fullscreen: false,
            difficulty: "普通".to_owned(),
            diff_idx: Some(1),
            reset_requested: false,
        }
    }

    /// 左侧主菜单。`clicks` 为共享的点击计数（`&mut` 传入，各模块累加）。
    /// 注意：闭包内不可触碰 `UiState`（已被 `ui` 借用），重置请求记录到
    /// `self.reset_requested`，由 `update` 在 `Frame::ui` 闭包末尾统一处理。
    fn ui(&mut self, ui: &mut Ui, clicks: &mut u32) {
        ui.pack_at(Vec2::new(16.0, 90.0), PackSide::Top, |p| {
            p.label("主菜单");
            // 新控件 API（Widget trait + 属性化 builder，见 rjw_krusie::ui::widgets）：
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
                *clicks += 1;
            }
            p.add(
                Label::new("样式标签：蓝色 16px")
                    .color(Color::rgba_u8(96, 160, 235, 255))
                    .font_size(16.0),
            );
            if p.button("btn_reset", "重置 UI 状态 (R)").clicked() {
                self.reset_requested = true;
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
            // 新功能演示：数字条（widgets 组合控件） + WidgetId 数字 ID。
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
            // combo 下拉框（难度选择）：展开浮层选一项，点击外部收起。
            const DIFFS: [&str; 3] = ["简单", "普通", "困难"];
            let diff_opts: Vec<String> = DIFFS.iter().map(|s| s.to_string()).collect();
            if let Some(i) = p.combo("diff_combo", &self.difficulty, &diff_opts, self.diff_idx) {
                self.diff_idx = Some(i);
                self.difficulty = DIFFS[i as usize].to_owned();
            }
            p.label(&format!("难度: {}", self.difficulty));
            p.label(&format!("全屏: {}", if self.fullscreen { "开" } else { "关" }));
            // 布局增强演示：换行 + min/max 尺寸约束。
            p.label("尺寸约束（min 160 / max 120）");
            p.min_size(160.0, 0.0);
            if p.button("btn_min", "min 宽").clicked() {
                *clicks += 1;
            }
            p.max_size(120.0, 0.0);
            if p.button("btn_max", "max 宽").clicked() {
                *clicks += 1;
            }
            p.label_wrap(180.0, "自动换行标签：pack 内 180 宽自动换行成多行，适合说明文字。");
            // 水平行（row）：{Label} {NumberInput} {NumberInput} {Button} 占一行。
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
            // 分割线（占光标；宽 = 容器可用宽 / 当前最宽子项）。
            p.divider();
            p.label("分割线下方的段落……");
        });
    }
}

/// 可拖拽面板 + 3 列背包 grid 模块。
struct Inventory {
    inventory: [bool; 9],
}

impl Inventory {
    fn new() -> Self {
        Self { inventory: [false; 9] }
    }

    /// 右侧背包窗口（可拖拽；3 列均匀网格，点击格子切换）。
    fn ui(&mut self, ui: &mut Ui) {
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
    }
}

/// 可重叠 / 置顶 / 拖拽窗口 + 整窗 FX + 可调整大小文本输入框模块。
struct Windows {
    win_a_pos: Vec2,
    win_b_pos: Vec2,
    /// 窗口 A 勾选状态（跨帧持久）。
    win_a_checked: bool,
    /// 窗口 B 单行输入内容。
    win_b_note: String,
    /// 窗口 B 多行备注（TextArea 演示；同时被"可调大小 TextArea"复用）。
    win_b_note_area: String,
    /// 多行文本域是否自动换行（false = 不换行 + 水平滚动）。
    ta_wrap: bool,
    /// chishi（旋转 + RGBA 染色窗口）数值。
    cshi_num: f32,
    /// --auto-drag：每帧递增的帧序号（强制窗口内容每帧变化 → 缓存 miss 重建）。
    auto_tick: u64,
}

impl Windows {
    fn new() -> Self {
        Self {
            win_a_pos: Vec2::new(560.0, 240.0),
            // win_b 默认避开 win_a 右下角（缩放柄可达；仍与 win_a 右上角重叠演示置顶）
            win_b_pos: Vec2::new(760.0, 120.0),
            win_a_checked: false,
            win_b_note: String::new(),
            win_b_note_area: "多行备注：\nEnter 换行，↑↓ 跨行，Home/End 行首尾，\n拖选文本后 Ctrl+C/V/X 复制/粘贴/剪切。".to_owned(),
            ta_wrap: true,
            cshi_num: 0.,
            auto_tick: 0,
        }
    }

    /// --auto-drag：自动拖动 win_b（圆周轨迹）+ 每帧改内容（等价"拖动中 hover/光标
    /// 闪烁/滚动"的逐帧内容变化 → 走缓存未命中重建路径）。
    fn tick_auto(&mut self, t: f64) {
        self.win_b_pos = Vec2::new(
            640.0 + 220.0 * (t * 0.7).sin() as f32,
            330.0 + 140.0 * (t * 1.1).cos() as f32,
        );
        self.auto_tick = self.auto_tick.wrapping_add(1);
    }

    /// 全部窗口 + 整窗 FX + 可调大小文本输入框。`t` 为帧时间基准（`drag_t0` 起），
    /// 供 FX 动画与 auto_drag 用。
    fn ui(&mut self, ui: &mut Ui, clicks: &mut u32, t: f64) {
        // 窗口 A：固定宽（右下角缩放柄）+ 逐窗口样式 + 位置 clamp。
        //
        // 逐窗口覆盖**从当前主题派生**（`ui.theme().panel`）再改背景与圆角——
        // 只覆盖想改的字段，边框 / 内边距等仍跟主题走。若写死 `PanelStyle::default()`
        // 就是拿**浅色**默认当基底，切到深色主题后这个窗口会与其它窗口不一致。
        let panel_a = ui.theme().panel.clone();
        ui.window("win_a")
            .pos(self.win_a_pos)
            .width(220.0)
            .style(
                panel_a
                    .with_bg(Color::rgba_u8(40, 44, 62, 255))
                    .with_radius(8.0),
            )
            .clamp(WindowClamp::Screen)
            .show(|w| {
                w.label("窗口 A（点击置顶 · 拖动移动）");
                if w.button("win_a_btn", "A 按钮").clicked() {
                    *clicks += 1;
                }
                // 勾选状态由应用持有（跨帧持久）：checkbox_mut 点击直接翻转 `&mut bool`。
                w.checkbox_mut(None, "窗口 A 选项", &mut self.win_a_checked);
                // Label 溢出演示（Resizable 窗口缩窄）：默认自动换行；`.ellipsis()` 省略。
                w.add(Label::new("自动换行标签：窗口缩窄后自动换行，不再溢出画到窗口外。"));
                w.add(Label::new("省略标签：窗口缩窄后显示为省略号……").ellipsis());
                w.add(Divider::new());
                w.label("分割线下方");
            });
        // 窗口 B（覆盖在 A 之上）：输入框 + 多行 TextArea。
        ui.window("win_b").pos(self.win_b_pos).show(|w| {
            w.label("窗口 B（覆盖在 A 之上）");
            // 性能测量：auto_drag 时每帧变化的标签（强制窗口内容每帧变化 → 重建路径）。
            w.label(&format!("帧序号 {}", self.auto_tick % 1000));
            if w.button("win_b_btn", "B 按钮").clicked() {
                *clicks += 1;
            }
            w.text_input("win_b_input", &mut self.win_b_note);
            w.checkbox_mut(Some("ta_wrap"), "自动换行", &mut self.ta_wrap);
            w.label("多行备注（Enter 换行 · 双击按词选择 · 拖选复制粘贴）");
            if self.ta_wrap {
                w.text_area("win_b_note_area", &mut self.win_b_note_area);
            } else {
                w.text_area_nw("win_b_note_area", &mut self.win_b_note_area);
            }
        });
        // 严格裁剪窗口（Placement::Clip）：内容超出窗口被强制裁剪（Clip 沙箱）。
        ui.window("strict_win")
            .pos(Vec2::new(560.0, 460.0))
            .placement(Placement::Clip)
            .show(|w| {
            w.label("严格裁剪窗口（内容超出被裁）");
            w.add(Label::new(
                "这一段文字足够长，会超出严格窗口的可视区——超出部分被强制裁剪，\
                 不再撑大窗口；滚动容器 / 文本编辑框同为 Clip 语义。",
            ));
            if w.button("strict_btn", "被裁窗口按钮").clicked() {
                *clicks += 1;
            }
        });
        // 赤石窗口：整窗旋转（角度 = cshi_num）+ RGBA 染色（4 个 slider 调）。
        let mut r = 1.0;
        let mut g = 1.0;
        let mut b = 1.0;
        let mut a = 1.0;
        ui.window("chishi").pos(vec2(155., 32.)).show(|w| {
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
        });
        // 赤石整窗 FX：旋转绕窗口中心；顶点缓存不变，仅提交时应用 tint/transform。
        ui.window_fx("chishi", WindowFx {
            tint: Color::rgba(r, g, b, a),
            transform: Some(Transform2D::IDENTITY.with_rot(self.cshi_num.to_radians())),
            anchor: Vec2::new(0.5, 0.5),
        });
        // 窗口级 FX（window_fx）：win_b 整窗淡入淡出 + 轻微上浮动画。
        let fx_alpha = 0.75 + 0.25 * (t * 1.5).sin() as f32;
        ui.window_fx(
            "win_b",
            WindowFx {
                tint: Color::rgba_u8(255, 255, 255, (fx_alpha * 255.0) as u8),
                transform: Some(
                    Transform2D::IDENTITY.with_pos(Vec2::new(0.0, 5.0 * (t * 1.2).sin() as f32)),
                ),
                anchor: Vec2::new(0.5, 0.5), // 旋转/缩放绕窗口中心
            },
        );
        // 可调整大小 TextArea / 宽度 TextInput（右下角缩放柄拖拽；尺寸责任链可覆盖）。
        // 复用 win_b 的备注字符串（同内容两处编辑演示）。
        ui.label_at(Vec2::new(880.0, 618.0), "可调大小 TextArea / 宽度 TextInput（右下角拖拽）");
        ui.resizable_text_area_at(
            "res_area",
            Rect::new(880.0, 638.0, 240.0, 50.0),
            &mut self.win_b_note_area,
            Vec2::new(140.0, 32.0),
            Resize::Both,
        );
        ui.resizable_text_input_at(
            "res_input",
            Rect::new(880.0, 696.0, 240.0, 26.0),
            &mut self.win_b_note,
            100.0,
            Resize::Horizontal,
        );
    }
}

/// 右侧区模块：窗口诊断面板 + 滚动列表 + flex 权重 + 底部说明。
struct RightPanel {
    /// list 选中索引（滚动列表）。
    list_sel: Option<u32>,
}

impl RightPanel {
    fn new() -> Self {
        Self { list_sel: None }
    }

    /// 右侧区 UI。`prev_press` / `prev_blocked` 为上一帧窗口诊断数据（须在
    /// **本帧 UI 录制之前**从 `ui.state()` 读取——值由上一帧 `Ui::finish` 写入，
    /// 本帧 `finish` 才覆盖）。
    fn ui(&mut self, ui: &mut Ui, clicks: &mut u32, prev_press: &str, prev_blocked: u32) {
        // 窗口诊断面板：实时显示窗口叠放与点击解析。
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
        // 滚动容器演示：可滚动选择列表（list_at：滚轮 / 滚动条 + 选中态）。
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
            *clicks += 1;
        }
        // 布局增强演示：flex 权重（固定高 150，[1:2:1] 等分）。
        ui.flex_at(vec2(880.0, 450.0), 150.0, &[1, 2, 1], |f, i| {
            if f.button(&format!("flex_row_{i}"), &format!("行 {i} · 权重 {}", [1, 2, 1][i]))
                .clicked()
            {
                *clicks += 1;
            }
        });
        // 底部说明（锚定视口左下角——不再被窗口遮挡）。
        let hint = "Tab/方向键 遍历焦点 · 输入框拖选文本 + Ctrl+C/V/X · 双击按词选择 · Enter 换行（多行） · 滚轮滚动（指针在框内） · Esc 收起/失焦（再按退出） · R 重置";
        let (hint_fs, hint_ff) = (ui.theme().label.font_size, ui.theme().label.font_family.clone());
        let hint_size = ui.text_size(hint, hint_fs, hint_ff.as_deref());
        let hint_pos = ui.anchor_pos(Anchor::BottomLeft, hint_size, Vec2::new(16.0, 16.0));
        ui.label_at(hint_pos, hint);
    }
}

/// 应用主体：相机 + 跨模块全局状态 + 各 UI 模块（见文件顶部「代码组织」）。
///
/// 旧的 `render` / `render2d` / `render2d_ui` / `font` / `viewport` 字段已删除：
/// 渲染上下文、世界层与 UI 层渲染器、文本子系统、画面矩形全部由运行时持有
/// （`Ctx` / `Frame` / `Gfx`），应用不再直接管理它们。
/// **主题调节窗口**：实时改调色板令牌 / 圆角 / 羽化 / 微渐变强度。
///
/// 主题每帧由 [`ThemeTuner::theme`] 重新组装（`Theme::themed(&Palette)`），所以拖动
/// 滑块**当帧**就能看到整屏变化——顺带演示了「换肤 = 换一份 [`Palette`]」这条设计：
/// 主题不是 11 个子样式的字面量，而是一份按**层次**命名的调色板 + 几个全局标量。
///
/// 用法：拖「圆角 / 羽化」看窗口与按钮的圆角与边缘软硬；拖「表面明度」找你要的暗度；
/// 拖「强调 R/G/B」看焦点描边、滑轨填充、勾选填充、下拉菜单选中项一起变色。
struct ThemeTuner {
    /// 预设：0 = dark，1 = light，2 = legacy dark（改造前的旧配色，用于对照）。
    preset: u8,
    /// 全局圆角（逻辑像素；级联到 panel / button / input / checkbox / combo）。
    radius: f32,
    /// 边缘羽化宽（逻辑像素；0 = 硬边）。
    feather: f32,
    /// 表面微渐变强度（`Palette.bevel`）。
    bevel: f32,
    /// 表面基色明度增益（乘到全部 `surface*` 令牌上；< 1 更暗、> 1 更亮）。
    surface_gain: f32,
    /// 强调色（sRGB 0..1 的 RGB；hover / active 由它派生）。
    accent: [f32; 3],
    /// 是否显示本窗口。
    open: bool,
}

impl ThemeTuner {
    fn new() -> Self {
        Self {
            preset: 0,
            radius: 8.0,
            feather: 1.0,
            bevel: 0.10,
            surface_gain: 1.0,
            accent: [0.43, 0.66, 1.0],
            open: false,
        }
    }

    /// 按当前旋钮组装主题（`frame.ui(..)` 之前调用——闭包借用 `self`，闭包内不能构造）。
    fn theme(&self, font: &str) -> Theme {
        let mut p = match self.preset {
            1 => Palette::light(),
            2 => Palette::legacy_dark(),
            _ => Palette::dark(),
        };
        // 表面明度整体增益：一次改完一整条层次阶梯，不用逐个令牌调。
        let g = self.surface_gain;
        if (g - 1.0).abs() > 1e-4 {
            let sc = |c: rjw_krusie::color::Color| scale_luma(c, g);
            p.surface_dim = sc(p.surface_dim);
            p.surface_sunken = sc(p.surface_sunken);
            p.surface = sc(p.surface);
            p.surface_raised = sc(p.surface_raised);
            p.surface_overlay = sc(p.surface_overlay);
            p.surface_hover = sc(p.surface_hover);
            p.surface_active = sc(p.surface_active);
        }
        p.bevel = self.bevel;
        p.accent = Color::rgba(self.accent[0], self.accent[1], self.accent[2], 1.0);
        p.accent_hover = scale_luma(p.accent, 1.25);
        p.accent_active = scale_luma(p.accent, 0.80);
        let mut t = Theme::themed(&p)
            .with_radius(self.radius)
            .with_feather(self.feather);
        if !font.is_empty() {
            t = t.with_font_family(font);
        }
        t
    }

    fn ui(&mut self, ui: &mut Ui) {
        if !self.open {
            return;
        }
        // ⚠ **不设 `.style(..)`**：窗口继承当前主题的 `PanelStyle`，与其它窗口**完全一致**
        // （背景刷 / 边框 / 圆角 / 内边距都跟着滑块实时变）。
        // 早先这里写死 `PanelStyle::default()`——那是**浅色**基底，于是调节窗口自己是
        // 一块浅灰板，跟满屏深色格不入，看着像"另一个主题的窗口"。
        ui.window("theme_tuner")
            .pos(vec2(280.0, 420.0))
            .show(|w| {
                w.label("主题调节（实时）");
                w.row(|w| {
                    w.label("预设:");
                    if w.button("th_dark", "dark").clicked() {
                        self.preset = 0;
                        self.surface_gain = 1.0;
                        self.bevel = 0.10;
                        self.accent = [0.43, 0.66, 1.0];
                    }
                    if w.button("th_light", "light").clicked() {
                        self.preset = 1;
                        self.surface_gain = 1.0;
                        self.bevel = 0.02;
                        self.accent = [0.31, 0.55, 0.86];
                    }
                    if w.button("th_legacy", "legacy").clicked() {
                        self.preset = 2;
                        self.surface_gain = 1.0;
                        self.bevel = 0.0;
                        self.accent = [0.38, 0.59, 0.86];
                    }
                });
                w.row(|w| {
                    w.label("圆角");
                    self.radius = w.slider("th_radius", 0.0..=24.0, self.radius);
                });
                w.row(|w| {
                    w.label("羽化");
                    self.feather = w.slider("th_feather", 0.0..=5.0, self.feather);
                });
                w.row(|w| {
                    w.label("微渐变");
                    self.bevel = w.slider("th_bevel", 0.0..=0.35, self.bevel);
                });
                w.row(|w| {
                    w.label("表面");
                    self.surface_gain = w.slider("th_gain", 0.4..=1.6, self.surface_gain);
                });
                w.row(|w| {
                    w.label("强调");
                    self.accent[0] = w.slider("th_ar", 0.0..=1.0, self.accent[0]);
                    self.accent[1] = w.slider("th_ag", 0.0..=1.0, self.accent[1]);
                    self.accent[2] = w.slider("th_ab", 0.0..=1.0, self.accent[2]);
                });
                w.label(&format!(
                    "预设 {} · 圆角 {:.0} · 羽化 {:.1} · 微渐变 {:.2} · 表面 ×{:.2}",
                    ["dark", "light", "legacy"][self.preset.min(2) as usize],
                    self.radius,
                    self.feather,
                    self.bevel,
                    self.surface_gain,
                ));
            });
    }
}

/// 明度缩放（`k > 1` 变亮；alpha 不变，分量 clamp 到 [0,1]）。
fn scale_luma(c: rjw_krusie::color::Color, k: f32) -> rjw_krusie::color::Color {
    let a: [f32; 4] = c.into();
    Color::from([
        (a[0] * k).clamp(0.0, 1.0),
        (a[1] * k).clamp(0.0, 1.0),
        (a[2] * k).clamp(0.0, 1.0),
        a[3],
    ])
}

struct UiApp {
    /// 世界层相机（`f.submit` 会写入当前画面矩形；identity 位姿 = 世界原点居中）。
    cam: Camera2D,
    /// 文本子系统（`init` 里用 `Gfx::text()` 建；UI 由运行时驱动，应用侧不再用它绘制）。
    font: Option<Text>,
    // 跨模块共享 / 全局状态
    /// 点击计数（各 UI 模块以 `&mut u32` 累加，顶部状态栏显示）。
    clicks: u32,
    drag_t0: Instant,
    /// --auto-drag：自动拖动 win_b 并每帧改内容（最坏重建路径）。
    auto_drag: bool,
    /// --script-pos：位置责任链演示——脚本驱动 win_a 摆动（优先级 -10，拖拽优先）。
    script_pos: bool,
    /// --ui-dump：每帧打印 UI 引擎状态（`Ui::debug_dump`，Rust 侧调试）。
    ui_dump: bool,
    /// --sim-drag：**脚本化鼠标**拖动 win_b（`Frame::debug_inject_mouse`）——
    /// 无鼠标环境复现「窗口拖动」（见 docs/DEBUGGING.md）。
    sim_drag: bool,
    /// 帧统计聚合（每 `PERF_PRINT_EVERY` 帧打印一次）。
    perf: PerfAgg,
    // 各 UI 模块
    top: TopBar,
    menu: Menu,
    inventory: Inventory,
    windows: Windows,
    right: RightPanel,
    theme_tuner: ThemeTuner,
}

impl UiApp {
    fn new() -> Self {
        Self {
            cam: Camera2D::default(),
            font: None,
            clicks: 0,
            drag_t0: Instant::now(),
            auto_drag: false,
            script_pos: false,
            ui_dump: false,
            sim_drag: false,
            perf: PerfAgg::new(),
            top: TopBar::new(),
            menu: Menu::new(),
            inventory: Inventory::new(),
            windows: Windows::new(),
            right: RightPanel::new(),
            theme_tuner: ThemeTuner::new(),
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

    /// 累计一帧（`s` = 本帧 `Ui::finish` 写入的 UI 阶段统计；须在 `f.ui(..)` 闭包内取）。
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

/// 世界层：几个背景方块（在 UI 之下）。与 `UiApp` 无关，故为自由函数（避免 `&mut self`
/// 与 `f.draw()` 的字段借用冲突）。
fn render_world(r2d: &mut Render2D) {
    let world_tf = Transform2D::default();
    r2d.solid(SpriteRect::new((-640.0, -360.0), (1280.0, 720.0)))
    .tint(Color::rgba_u8(30, 36, 48, 255))
    .transform(world_tf)
    .layer(0.0);
    for i in 0..6 {
        let x = -560.0 + i as f32 * 220.0;
        r2d.solid(SpriteRect::new(
            (x, -280.0 + (i % 2) as f32 * 160.0),
            (160.0, 90.0),
        ))
        .tint(Color::rgba_u8(40 + i * 20, 60, 90, 255))
        .transform(world_tf)
        .layer(1.0);
    }
}

impl App for UiApp {
    fn config(&self) -> AppConfig {
        AppConfig::new("eg260818UI — rjw_ui 示例").size(1280.0, 720.0)
    }

    fn init(&mut self, gfx: &Gfx) -> Result<(), AppInitError> {
        // 文本子系统（长期资源）由 `Gfx` 建：取代旧的
        // `Text::new(r2d.device(), r2d.queue(), r2d.texture_layout())`。
        // 其余长期资源（RenderContext / 世界层与 UI 层 Render2D / 画面矩形）由运行时持有，
        // 应用不再自建（UI 层排序已由运行时关闭，无需 `set_sort_mode(SortMode::None)`）。
        self.font = Some(gfx.text());
        Ok(())
    }

    fn update(&mut self, ctx: &mut Ctx) {
        let t_frame = Instant::now();

        // 帧时间基准（FX 动画 / --auto-drag 用）。
        let t = self.drag_t0.elapsed().as_secs_f64();
        // 性能测量：--auto-drag 自动拖动 win_b（圆周轨迹）+ 每帧改内容。
        if self.auto_drag {
            self.windows.tick_auto(t);
        }

        // 无帧不执行渲染代码（本帧也不录制 UI）。
        let Some(mut f) = ctx.frame() else {
            return;
        };

        // ── 调试：脚本化鼠标（`--sim-drag`）────────────────────────
        // 复现"窗口拖动"：第 20 帧在 win_b 标题栏按下，随后每帧右移 6px（物理像素），
        // 第 80 帧释放。**不需要真实鼠标**——引擎合成的边沿与真实设备一致。
        if self.sim_drag {
            let n = f.frames();
            let start = Vec2::new(1180.0, 200.0); // win_b 标题栏附近（origin ≈ 1140,180）
            match n {
                20 => f.debug_inject_mouse(start, true),
                21..=79 => {
                    let dx = (n - 20) as f32 * 6.0;
                    f.debug_inject_mouse(Vec2::new(start.x + dx, start.y + dx * 0.5), true)
                }
                _ => {}
            }
        }

        // ── 世界层：几个背景方块（在 UI 之下）─────────────────
        // 阶段计时（沿用旧 `[perf]` 的细分口径，因新驱动不再暴露 Frame 取用/pass 边界，
        // 以可达的边界重新划分）：`begin` = 世界层录制，`encode` = UI 帧
        // （`Ui::begin` → `Ui::finish` 的录制/布局/提交队列），`submit` = `f.submit`，
        // `present` = `f.present`；四段不重叠，合计 = `render` 总耗时。
        let t_world = Instant::now();
        render_world(f.draw());
        let begin_us = t_world.elapsed().as_secs_f64() * 1e6;

        // 主题由 [`ThemeTuner`] 每帧组装（预设调色板 + 圆角 / 羽化 / 微渐变 / 强调色，
        // 以及 FontModal 选定的字体族）。⚠ 须在 `f.ui(..)` **之前**构建：
        // 闭包借用 `self`，闭包内不能构造它。
        let theme = self.theme_tuner.theme(self.top.font_name());

        // 性能统计（`f.ui` 前复制的上一帧值；闭包内每帧覆盖）。
        let mut ui_stats = UiStats::default();
        // 本帧点击计数（顶部状态栏显示）；UI 模块内累加，帧末写回 `self.clicks`。
        let mut clicks = self.clicks;
        // --script-pos：`Instant` 为 `Copy`——先复制出时间基准，闭包只捕获它（不借 `self`）。
        let script_pos = self.script_pos;
        // --ui-dump：每帧打印 UI 引擎状态（Rust 侧诊断，见 docs/DEBUGGING.md）。
        let ui_dump = self.ui_dump;
        let t0 = self.drag_t0;
        let fps = f.fps();
        // `Esc` 退出请求：`Frame` 借用了 `ctx`，闭包内不能再借 `ctx`（`f.ui` 与
        // `ctx.exit()` 的借用冲突）——先记标记，闭包结束后再请求退出。
        let mut exit_requested = false;

        // ── UI 层：录制 + 提交由运行时接管（`Ui::begin` / 输入快照 / 主题 / DPI /
        //    `Ui::finish(&region, r2d_ui)`）；UI 渲染器排序已关闭。 ────────────
        let t_ui = Instant::now();
        f.ui(theme, |ui| {
            // ── 应用快捷键：**文本输入框聚焦时屏蔽**（`UiState::text_focus()`）——
            //    输入 `R` / `Esc` 不会被当作重置 / 退出。
            //    与旧 `capturing_text()`（任何控件持焦点都为真）不同：只有**文本控件**
            //    持焦点才屏蔽快捷键（按钮/滑块 Tab 焦点不吞应用按键）。
            //    新驱动下没有「`Ui::begin` 之前」的 `UiState` 取用口，故快捷键判定放在
            //    闭包内、经 `Ui::state()` 读取（输入快照在 `Ui::begin` 时已捕获，
            //    `down_edge` 语义与旧版一致）。
            if ui.state().text_focus().is_none() {
                if ui.key_down_edge(KeyCode::Escape) {
                    exit_requested = true;
                }
                if ui.key_down_edge(KeyCode::KeyR) {
                    reset_ui_state(ui.state_mut());
                }
            }
            // 窗口诊断（调试机制）：值由**上一帧** `Ui::finish` 写入、本帧 `finish` 覆盖
            // （`last_press_window` / `occluded_hits` 跨帧保留）——须在「本帧 UI 模块录制
            // 之前」从 `ui.state()` 读取。`f.ui(..)` 的闭包是唯一能拿到 `UiState` 的地方，
            // 故由旧版「`Ui::begin` 之前读」改为「闭包开头读」；显示内容与旧版一致。
            let prev_press = ui
                .state()
                .last_press_window()
                .map(|(id, z)| format!("{id} (z{z})"))
                .unwrap_or_else(|| "无".to_owned());
            let prev_blocked = ui.state().occluded_hits();

            // ── 位置责任链演示（--script-pos）：脚本让窗口 A 沿正弦摆动 ──
            // 处理器优先级 -10（< 0）：**用户拖拽优先**——拖住 A 时脚本让位、窗口跟手，
            // 松开后停在放置处；不拖时脚本每帧驱动位置（脚本"动画"，拖动"覆盖"）。
            if script_pos {
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

            // ── 各 UI 模块依次录制（互不重叠字段借用，顺序与屏幕布局无关） ──
            self.top.ui(ui, fps, clicks, &mut self.theme_tuner);
            self.menu.ui(ui, &mut clicks);
            self.inventory.ui(ui);
            self.windows.ui(ui, &mut clicks, t);
            self.right.ui(ui, &mut clicks, &prev_press, prev_blocked);
            self.theme_tuner.ui(ui);

            // 字体 Modal（**帧末录制**：modal 的 z 每帧重写为当前最大，最后录制才能保证
            // 不被本帧后录的窗口盖住——见 modal_at 文档）。
            self.top.show_font_modal(ui);

            // 性能统计（**闭包末尾、本帧 `Ui::finish` 之前**读到的正是上一帧 finish 写入
            // 的 UI 各阶段耗时——与旧版「`ui.finish()` 之后读 `ui_state.stats`」等价：
            // 那时读到的同样是上一帧的统计，本次 `finish` 才会覆盖它）。
            ui_stats = ui.state().stats.clone();

            // ── 引擎状态诊断（`--ui-dump`）：Rust 侧调试用 —— 打印每个窗口的
            //    id / z / **本帧提交原点** / 尺寸 / 拖拽状态 / 持久位置 + 鼠标 / 焦点。
            //    排查"位置 / 层级 / 拖拽"问题时**先看这份状态**（见 docs/DEBUGGING.md）。
            if ui_dump {
                eprintln!("{}", ui.debug_dump());
            }

            // 重置请求（按钮点击 / `R` 键）：须在**全部录制之后**执行——
            // `UiState::reset` 清空控件状态与窗口缓存，此时本帧命令已录好、下次
            // `begin_frame` 前无读取者（与旧版「`ui.finish()` 后重置」等价）。
            if self.menu.reset_requested {
                self.menu.reset_requested = false;
                reset_ui_state(ui.state_mut());
            }
        });
        let encode_us = t_ui.elapsed().as_secs_f64() * 1e6;
        self.clicks = clicks;

        // ── 提交：世界层与 UI 层进同一个 pass（清色 + 一次 present）──────
        // `f.submit` 负责写入画面矩形 → 取 VP → 开 pass → 提交世界与 UI → 编码提交；
        // `f.present()` 呈现（可省略：`Frame` 析构自动呈现）。
        let t_sub = Instant::now();
        f.submit(
            &mut self.cam,
            Clear::color(Color::rgb(0.09, 0.11, 0.16)),
        );
        let submit_us = t_sub.elapsed().as_secs_f64() * 1e6;
        let t_present = Instant::now();
        f.present();
        let present_us = t_present.elapsed().as_secs_f64() * 1e6;

        // 性能统计：整帧 / 渲染（细分）/ UI 各阶段（每 PERF_PRINT_EVERY 帧打印一次）
        let render_us = begin_us + encode_us + submit_us + present_us;
        let frame_us = t_frame.elapsed().as_secs_f64() * 1e6;
        self.perf
            .add(&ui_stats, frame_us, render_us, begin_us, encode_us, submit_us, present_us);
        if self.perf.frames >= PERF_PRINT_EVERY {
            self.perf.flush(fps);
        }

        // 退出请求（`Esc`；输入框聚焦时已在闭包内屏蔽）：本帧照常收尾，帧末退出循环。
        // ⚠ 须在 `f` 最后一次使用之后——`Frame` 借用了 `ctx`（且有 `Drop`，借用活到
        //    其作用域末尾），`f.drop()` 之后才能再取 `ctx`。
        drop(f);
        if exit_requested {
            ctx.exit();
        }
    }
}

/// 解析 `--win-a X,Y --win-b X,Y` 命令行参数（RenderDoc 重叠次序验证用）。
fn parse_pos_arg(args: &[String], key: &str, default: Vec2) -> Vec2 {
    let mut out = default;
    let mut i = 0;
    while i < args.len() {
        if args[i] == key && i + 1 < args.len()
            && let Some((x, y)) = args[i + 1].split_once(',')
                && let (Ok(x), Ok(y)) = (x.trim().parse(), y.trim().parse()) {
                    out = Vec2::new(x, y);
                }
        i += 1;
    }
    out
}

fn main() -> Result<(), RunError> {
    let args: Vec<String> = std::env::args().collect();
    let mut app = UiApp::new();
    app.windows.win_a_pos = parse_pos_arg(&args, "--win-a", app.windows.win_a_pos);
    app.windows.win_b_pos = parse_pos_arg(&args, "--win-b", app.windows.win_b_pos);
    app.auto_drag = args.iter().any(|a| a == "--auto-drag");
    app.script_pos = args.iter().any(|a| a == "--script-pos");
    app.ui_dump = args.iter().any(|a| a == "--ui-dump");
    app.sim_drag = args.iter().any(|a| a == "--sim-drag");
    run(app)
}

/// 重置 UI 状态并恢复默认选中"普通"难度。
///
/// UI 状态现由运行时持有（`Ctx::ui_layer`），应用经 `Ui::state_mut()` 在
/// `Frame::ui` 闭包内取用——本函数改为就地传入 `&mut UiState`。
fn reset_ui_state(state: &mut UiState) {
    state.reset();
    // 默认选中"普通"难度（单选组值 = 控件**绝对 ID**；顶层无前缀 = 原样）
    state
        .radio_groups
        .insert("diff".to_owned(), IdAbsolute::from("diff_normal"));
}