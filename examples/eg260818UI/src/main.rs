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
//! - [`overlap`]：两个**故意重叠**的控件探针 —— 演示 / 自证**控件级遮挡**
//!   （重叠处只有画在上面的那个被触发；`--sim-overlap` 脚本化点击验证）。
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
use rjw_krusie::ui::{
    ColorPicker, CornerRadius, DEFAULT_LINE_SPACING, Density, FontModal, GRIP_W, GripShape,
    GripStyle, IdAbsolute, Label, Palette, Position, ShadowStyle, Weight, weight_label,
};

/// 「重叠控件」演示模块（控件级遮挡：重叠处只有最上层被触发 + `--sim-overlap` 自证）。
mod overlap;
use overlap::OverlapDemo;

/// **文件导入**（系统文件选择器 → 字节 → 引擎资源）：图片当背景纹理、字体进运行时字体库。
mod filedialog;
use filedialog::ImportKind;

/// 「上层窗口没挡住背后窗口的控件」脚本化复现（`--sim-cover`）。
mod cover;
use cover::CoverDemo;

/// 顶部状态栏模块：FPS / 点击次数标签 + 字体按钮（打开 Modal）+ 玩家名输入框 + 字体 Modal。
struct TopBar {
    /// 玩家名输入框内容（跨帧持久）。
    player_name: String,
    /// 当前应用的字体族（空 = 系统默认；FontModal 确定后写入，下一帧主题按它重建）。
    font_name: String,
    /// 当前**全局字重**（FontModal 的字重下拉；默认 400）。引擎侧是主题令牌
    /// `Theme::font_weight`——改它 = 全 UI 一起换字形（不只是"看着粗一点"，
    /// 步进宽度也会变 ⇒ 布局随之变）。
    font_weight: Weight,
    /// 字体 Modal 输入框内容（跨帧持久）。
    font_input: String,
    /// 字体 Modal 开关。
    font_modal_open: bool,
    /// **固定位置**的取色器颜色（物理定位：`--sim-picker` 的脚本化点击要能算到坐标）。
    demo_color: Color,
    /// **待处理的导入请求**（点「导入图片…」/「导入字体…」只记请求：系统选择器是
    /// **阻塞**调用，录制期不能弹——见 `filedialog` 模块文档）。
    import_request: Option<ImportKind>,
    /// 导入结果 / 失败原因（顶栏状态标签显示）。
    import_status: String,
}

impl TopBar {
    fn new() -> Self {
        Self {
            player_name: "Krisu".to_owned(),
            font_name: String::new(),
            font_weight: Weight::NORMAL,
            font_input: String::new(),
            font_modal_open: false,
            demo_color: Color::rgba_u8(255, 128, 40, 255),
            import_request: None,
            import_status: String::new(),
        }
    }

    /// 当前字体族（空 = 系统默认；供主题构建读取）。
    fn font_name(&self) -> &str {
        &self.font_name
    }

    /// 当前全局字重（供主题构建读取）。
    fn font_weight(&self) -> Weight {
        self.font_weight
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
                // 按钮文字带出当前**字体族 + 字重**（改完不用重新打开弹窗就知道现在是哪档）。
                let fam = if self.font_name.is_empty() { "默认" } else { &self.font_name };
                let label = format!("字体… {fam} / {}", weight_label(self.font_weight));
                if r.button("font_btn", &label).clicked() {
                    self.font_modal_open = true;
                }
                if r.button("theme_btn", "主题调节…").clicked() {
                    tuner.open = !tuner.open;
                }
                // **文件导入**（系统文件选择器）：点击只**记待办**——`rfd` 的选择器是阻塞
                // 调用，而这里还在录制帧（`f` 借着 `ctx`）⇒ 帧外再弹（见 `update`）。
                // 三条来源（两个按钮 / `--font-file` / `--sim-import`）共用 `apply_import`。
                if r.button("import_img_btn", "导入图片…").clicked() {
                    self.import_request = Some(ImportKind::Image);
                }
                if r.button("import_font_btn", "导入字体…").clicked() {
                    self.import_request = Some(ImportKind::Font);
                }
                // 导入结果 / 失败原因（空 = 不占位）。
                if !self.import_status.is_empty() {
                    r.add(Label::new(&self.import_status).ellipsis());
                }
            });
        });
        // 玩家名可拖动面板（右移：上面那行按钮随字体名变长，别压到它）。
        ui.drag_panel_at("name_panel", Vec2::new(430.0, 12.0), |p| {
            p.label("玩家名（可拖动）");
            p.text_input("name", &mut self.player_name);
        });
        // **取色器演示**（固定物理位置：`--sim-picker` 的脚本化点击按这个坐标算）。
        // 不传 `&mut String` —— 面板的文本框用全局跨帧缓冲（`ColorPickerState::text`）。
        ui.add_at(
            Position::Physical(Vec2::new(240.0, 250.0)),
            ColorPicker::new("picker_demo", &mut self.demo_color).alpha(true),
        );
    }

    /// 字体 Modal（**帧末调用**：modal 的 z 每帧重写为当前最大，最后录制才能保证
    /// 不被本帧后录的窗口盖住——见 `modal_at` 文档）。
    fn show_font_modal(&mut self, ui: &mut Ui) {
        if self.font_modal_open {
            FontModal {
                input: &mut self.font_input,
                weight: &mut self.font_weight,
                // 字重由**弹窗直接写回**（确定时按下拉选中项赋值）⇒ 回调里不再碰
                // `self.font_weight`（否则与上面那行 `&mut` 借用冲突）。
                apply: &mut |name: &str, _w: Weight| self.font_name = name.to_owned(),
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
    /// chishi（旋转 + 染色窗口）数值。
    cshi_num: f32,
    /// chishi 的整窗染色（由窗口内的 `ColorPicker` 调）——演示"窗口内取色器"。
    cshi_tint: Color,
    /// --auto-drag：每帧递增的帧序号（强制窗口内容每帧变化 → 缓存 miss 重建）。
    auto_tick: u64,
    /// **背景图**（`init` 里建的棋盘纹理）——四个窗口分别演示四种铺排（`ImageFit`）。
    bg_image: Option<ImageBg>,
    /// 窗口 A 是否显示（`close_button` 绑定的开关；× 点击 ⇒ `false`，本窗口**整窗短路**）。
    win_a_open: bool,
    /// 窗口 A 是否**收起**（`shrink` 绑定的状态；`true` = 只留标题栏）。
    win_a_collapsed: bool,
    /// --sim-chrome：上一次窗口 A 的结算尺寸（只在**变化**时打印，作为标题栏 / 收起 /
    /// 关闭三条路径的机器可读证据）。
    last_win_a_size: Vec2,
    /// --sim-chrome：窗口 A 出现过的 `(open, collapsed)` 组合（去重，帧末判定用）。
    chrome_states: Vec<(bool, bool)>,
    /// --sim-chrome：是否打印上面的证据。
    sim_chrome: bool,
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
            cshi_tint: Color::WHITE,
            auto_tick: 0,
            bg_image: None,
            win_a_open: true,
            win_a_collapsed: false,
            last_win_a_size: Vec2::ZERO,
            chrome_states: Vec::new(),
            sim_chrome: false,
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

    /// 全部窗口 + 整窗 FX + 可调大小文本输入框。
    ///
    /// 不再收帧时间：`win_b` 的浮动 / 淡入淡出已去掉，剩下唯一的 FX（赤石旋转 + 染色）
    /// 由用户拖滑块驱动；自动摆动在 `update` 里经 `tick_auto` 单独驱动。
    fn ui(&mut self, ui: &mut Ui, clicks: &mut u32) {
        // 窗口 A：固定宽（右下角缩放柄）+ 逐窗口样式 + 位置 clamp。
        //
        // 逐窗口覆盖**从当前主题派生**（`ui.theme().panel`）再改背景与圆角——
        // 只覆盖想改的字段，边框 / 内边距等仍跟主题走。若写死 `PanelStyle::default()`
        // 就是拿**浅色**默认当基底，切到深色主题后这个窗口会与其它窗口不一致。
        //
        // 圆角用 [`CornerRadius`]：**只圆上面两个角**（标签页 / 附着在工具栏下方的面板
        // 就是这么做的），下面两个角贴齐直角。
        let panel_a = ui.theme().panel.clone();
        let a_size = ui
            .window("win_a")
            .pos(self.win_a_pos)
            .width(220.0)
            .style(
                panel_a
                    .with_bg(Color::rgba_u8(40, 44, 62, 255))
                    .with_radius(CornerRadius { tl: 12.0, tr: 12.0, br: 0.0, bl: 0.0 }),
            )
            .clamp(WindowClamp::Screen)
            // **窗口外框**（责任链 builder）：标题栏 + 关闭 × + 收起 ⌃。
            // - `title("窗口 A")`：标题栏 = 内容**第一行**（窗口高度自然包含它）；空白处
            //   仍可拖动窗口，只有按钮上的按下不算"拖窗口"。
            // - `close_button(&mut self.win_a_open)`：点 × ⇒ `*open = false`；为 `false`
            //   时**整窗短路**（不录制、不占命中 / 遮挡矩形），重新打开由应用决定
            //   （下面 win_b 里的"窗口 A 显示"勾选框就是那条重开路径）。
            // - `shrink(true, &mut self.win_a_collapsed)`：点 ⌃ ⇒ 收起（只留标题栏、
            //   跳过内容闭包）。第一个参数是"要不要画按钮"——`false` 时仍能由代码 /
            //   菜单翻转状态（win_b 里的"窗口 A 收起"勾选框演示这条）。
            .title("窗口 A")
            .close_button(&mut self.win_a_open)
            .shrink(true, &mut self.win_a_collapsed)
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
        // --sim-chrome：结算尺寸**变化**即打印一行（收起 ⇒ 高度掉到一行标题栏；
        // 关闭 ⇒ `show` 返回 `(0,0)`）。这是三条外框路径的机器可读证据。
        if self.sim_chrome && a_size != self.last_win_a_size {
            eprintln!(
                "sim-chrome: win_a open={} collapsed={} pos=({:.0},{:.0}) size=({:.0},{:.0})",
                self.win_a_open,
                self.win_a_collapsed,
                self.win_a_pos.x,
                self.win_a_pos.y,
                a_size.x,
                a_size.y
            );
            self.last_win_a_size = a_size;
        }
        if self.sim_chrome {
            let st = (self.win_a_open, self.win_a_collapsed);
            if self.chrome_states.last() != Some(&st) {
                self.chrome_states.push(st);
            }
        }
        // 窗口 B（覆盖在 A 之上）：输入框 + 多行 TextArea。
        ui.window("win_b").pos(self.win_b_pos).show(|w| {
            w.label("窗口 B（覆盖在 A 之上）");
            // 性能测量：auto_drag 时每帧变化的标签（强制窗口内容每帧变化 → 重建路径）。
            w.label(&format!("帧序号 {}", self.auto_tick % 1000));
            if w.button("win_b_btn", "B 按钮").clicked() {
                *clicks += 1;
            }
            // **窗口 A 的开关**（`close_button` / `shrink` 的宿主状态）：× 关掉 A 之后
            // 只有这里能把它勾回来（引擎不替应用决定"重开"）。
            w.checkbox_mut(Some("win_a_show"), "窗口 A 显示", &mut self.win_a_open);
            w.checkbox_mut(Some("win_a_fold"), "窗口 A 收起", &mut self.win_a_collapsed);
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
        // 矢量图标演示：内置 [`Icon`] 全是**画出来的几何**（单位方框内的凸分片 +
        // 边缘羽化），与字体完全无关——换字体 / 字体缺字形都不会变形或缺字。
        // `icon` 占光标，`row` 内连续调用即得一条工具栏。
        let icon_fg = ui.theme().label.color;
        ui.window("icons").pos(Vec2::new(560.0, 300.0)).show(|w| {
            w.label("矢量图标（画出来的几何，与字体无关）");
            w.row(|r| {
                for ic in [
                    Icon::Check,
                    Icon::ChevronUp,
                    Icon::ChevronDown,
                    Icon::ChevronLeft,
                    Icon::ChevronRight,
                    Icon::Grip,
                ] {
                    r.icon(Vec2::new(18.0, 18.0), ic, icon_fg);
                }
            });
            w.label("绝对定位版：ui.icon_at(pos, size, icon, color)");
        });
        // ── 背景图（`ImageBg`）：四种铺排收进**一个窗口** ───────────────
        // 棋盘纹理在 `init` 里建（`Gfx::texture`），这里只拿 uid + 纹素尺寸。
        // 窗口背景图经 `PanelStyle::with_bg_image`（画在背景刷之上、内容之下，
        // 圆角遮罩**恒用面板 radius**）。
        //
        // **为什么是一个窗口、且在这个位置**：旧实现是四个独立演示硬编码在左侧
        // `(16,300)/(16,400)/(16,470)/(16,536)`，而左侧主菜单 pack 是 **win=0 同层**
        // 内容、已长到 y≈560 —— 于是 `image_at` 那两块棋盘**压在菜单文字上**
        // （"数字条 ×2 + 按钮"那行），两个图片窗口又盖住菜单中段。现在：
        // ① 四种铺排全在窗口内（窗口画在 win=0 之上，层次天然正确）；
        // ② 位置挪到右侧空白带（x 1120..1278 / y 545..~690：`flex_at` 之右、
        //    `strict_win` 之下、底部说明之上），与菜单 / 列表 / flex 全不重叠。
        //
        // 窗口本体 = **Tile 1:1 + 直角**：这正是"直角面板丢背景图"那个历史 bug 的现场
        // （`push_panel_like_img` 曾把背景图写在圆角分支里），修好后应能看见满窗棋盘。
        if let Some(bg) = self.bg_image {
            let base = ui.theme().panel.clone();
            let tile = bg.fit(ImageFit::Tile);
            ui.window("img_box")
                .pos(Vec2::new(1120.0, 548.0))
                .width(140.0)
                .style(base.with_radius(0.0).with_bg_image(tile))
                .show(|w| {
                    // 三行内嵌图片演示其余三种铺排（等比覆盖 / 拉伸 / 居中半透明）。
                    // ⚠ 行内文本 + 图片的**总宽必须 < 固定宽 − 内边距**：窗口会比
                    // 内容包围盒只大不小（`settle_size` 取并集），内容一宽就会被
                    // `WindowClamp::Screen` 顶到屏幕边、压到隔壁的 flex 演示上。
                    w.row(|r| {
                        r.label("Fill");
                        r.image(Vec2::new(56.0, 26.0), bg.fit(ImageFit::Fill).radius(4.0));
                    });
                    w.row(|r| {
                        r.label("Stretch");
                        r.image(Vec2::new(56.0, 26.0), bg.fit(ImageFit::Stretch).radius(4.0));
                    });
                    w.row(|r| {
                        r.label("Center");
                        r.image(
                            Vec2::new(56.0, 26.0),
                            bg.fit(ImageFit::Center).tint(Color::rgba(1.0, 1.0, 1.0, 0.6)),
                        );
                    });
                    w.label("本体 Tile 1:1");
                });
        }
        // 赤石窗口：整窗旋转（角度 = cshi_num）+ 染色（**用 ColorPicker 调**：
        // 一个控件顶掉原来那 4 条 RGBA 滑条，alpha 也由面板的 A 行负责）。
        //
        // ⚠ 位置从 `(155, 32)` 挪到 `(175, 170)`：原来它**压着顶部按钮行**（字体… /
        //    主题调节… 那一行在 y 56..82、x 155..314 正落在它下面）——窗口遮挡让那几个
        //    按钮**点不动**（点下去命中 `chishi/chisN1`）。现在让开顶部整条。
        ui.window("chishi").pos(vec2(175., 170.)).show(|w| {
            w.label("赤石");
            w.add(NumberInput::new("chisN1", &mut self.cshi_num).step(0.1));
            self.cshi_num = w.slider("sb", 0.0..=360., self.cshi_num);
            w.label("整窗染色");
            w.add(ColorPicker::new("cshi_tint", &mut self.cshi_tint).alpha(true));
        });
        // 赤石整窗 FX：旋转绕窗口中心；顶点缓存不变，仅提交时应用 tint/transform。
        ui.window_fx("chishi", WindowFx {
            tint: self.cshi_tint,
            transform: Some(Transform2D::IDENTITY.with_rot(self.cshi_num.to_radians())),
            anchor: Vec2::new(0.5, 0.5),
        });
        // 窗口级 FX 只演示「赤石」这一个窗口（整窗旋转 + RGBA 染色）。
        //
        // win_b 原本还有"整窗淡入淡出 + 轻微上浮"的 FX —— **已去掉**：持续运动会让人
        // 以为窗口在抖，干扰对布局 / 圆角 / 边框的观察；而"旋转 + 染色绕锚点"这一条
        // 已经完整覆盖了 `WindowFx` 的能力（顶点缓存不变、仅提交时应用）。
        // 想看淡入淡出，把这里的 tint alpha 换成随时间变化的量即可。
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

    /// 右侧区 UI。`prev_press` / `prev_blocked` / `prev_widget_blocked` 为上一帧诊断
    /// 数据（须在**本帧 UI 录制之前**从 `ui.state()` 读取——值由上一帧 `Ui::finish`
    /// 写入，本帧 `finish` 才覆盖）。
    fn ui(
        &mut self,
        ui: &mut Ui,
        clicks: &mut u32,
        prev_press: &str,
        prev_blocked: u32,
        prev_widget_blocked: u32,
    ) {
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
                "窗口 z 序: {}\n鼠标下最上层: {}\n上次按下接收: {}（上帧）\n窗口遮挡拦截: {}（上帧）\n控件遮挡拦截: {}（上帧）",
                if order.is_empty() { "无" } else { &order },
                under,
                prev_press,
                prev_blocked,
                prev_widget_blocked,
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
/// **主题调节窗口**：实时改调色板令牌 / 圆角 / 羽化 / 微渐变 / 边框。
///
/// 主题每帧由 [`ThemeTuner::theme`] 重新组装（`Theme::themed(&Palette)`），所以拖动
/// 滑块**当帧**就能看到整屏变化——顺带演示了「换肤 = 换一份 [`Palette`]」这条设计：
/// 主题不是 11 个子样式的字面量，而是一份按**层次**命名的调色板 + 几个全局标量。
///
/// 用法：
/// - 「圆角 / 羽化」看窗口与按钮的圆角与边缘软硬；
/// - 「微渐变」看背景刷（`Brush::Vertical`）两端色的差值——**颜色由顶点色 lerp**，
///   圆角弧上的顶点也按位置取色，所以整块渐变与矩形渐变一致；
/// - 「背景 R/G/B」直接设表面基色（整条层次阶梯按通道比一起缩，保持相对关系）；
/// - 「边框 R/G/B / 宽」改描边色与宽度（宽度级联到 panel / button / input / checkbox）；
/// - 「强调 R/G/B」看焦点描边、滑轨填充、勾选填充、下拉菜单选中项一起变色。
struct ThemeTuner {
    /// 预设：0 = dark，1 = light，2 = legacy dark（改造前的旧配色，用于对照）。
    preset: u8,
    /// 全局圆角（逻辑像素；级联到 panel / button / input / checkbox / combo）。
    radius: f32,
    /// 边缘羽化宽（逻辑像素；0 = 硬边）。
    feather: f32,
    /// 表面微渐变强度（`Palette.bevel`）。
    bevel: f32,
    /// 全局边框宽（逻辑像素；0 = 无边框）。
    border_w: f32,
    /// **布局密度档**（[`Density`]）：一键铺开"间距 / 字号 / 行距"三个倍率。
    density: Density,
    /// 字号倍率（在密度档基础上叠乘；1.0 = 不额外缩放）。
    font_scale: f32,
    /// 间距类令牌倍率（间距 / 行高 / 内边距 / 控件尺寸）。
    spacing_scale: f32,
    /// **多行行距倍率**（行高 = 字号 × 该值；作用于自动换行标签与多行输入框）。
    line_spacing: f32,
    /// **投影模糊宽**（逻辑像素；0 = 不画投影——立面 / 扁平风格）。
    shadow_blur: f32,
    /// **投影颜色**（含 alpha；`PanelStyle::shadow.color`，预设已带"深色 alpha 120 /
    /// 浅色 48"）。alpha 拖到 0 = 投影看不见（但 `blur > 0` 仍会镶嵌几何）。
    shadow_color: Color,
    /// **右下角缩放柄形状**（[`GripShape`]；只对固定宽窗口生效）。
    grip_shape: GripShape,
    /// 缩放柄颜色（默认跟预设的描边色）。
    grip_color: Color,
    /// 表面基色（整条 `surface*` 阶梯按**逐通道比**一起缩）。
    bg: Color,
    /// 描边色；`border_strong` 由它派生。
    border: Color,
    /// 强调色；hover / active 由它派生。
    accent: Color,
    /// 是否显示本窗口。
    open: bool,
}

/// 预设调色板。
fn preset_palette(preset: u8) -> Palette {
    match preset {
        1 => Palette::light(),
        2 => Palette::legacy_dark(),
        _ => Palette::dark(),
    }
}

/// 取颜色的 RGB（sRGB 0..1）——用于把预设颜色灌进调色旋钮。
fn rgb_of(c: rjw_krusie::color::Color) -> [f32; 3] {
    let a: [f32; 4] = c.into();
    [a[0], a[1], a[2]]
}

impl ThemeTuner {
    fn new() -> Self {
        let p = preset_palette(0);
        Self {
            preset: 0,
            radius: 8.0,
            feather: 1.0,
            bevel: p.bevel,
            border_w: 1.0,
            density: Density::default(),
            font_scale: 1.0,
            spacing_scale: 1.0,
            line_spacing: DEFAULT_LINE_SPACING,
            shadow_blur: ShadowStyle::default().blur,
            // 投影颜色取**预设调色板**的 `shadow`（深色 alpha 120 / 浅色 48）——
            // 那正是 `Theme::themed` 灌进 `PanelStyle::shadow.color` 的值 ⇒ 不动色块
            // 时与扩展前逐像素一致。
            shadow_color: p.shadow,
            grip_shape: GripShape::default(),
            grip_color: p.border,
            bg: p.surface,
            border: p.border,
            accent: p.accent,
            // 默认打开：这是个"可调的窗口"，开着才能看见效果。
            open: true,
        }
    }

    /// 切到某个预设并把旋钮复位到该预设的令牌（预设按钮用）。
    fn set_preset(&mut self, preset: u8) {
        let p = preset_palette(preset);
        self.preset = preset;
        self.bevel = p.bevel;
        self.bg = p.surface;
        self.border = p.border;
        self.accent = p.accent;
        // 柄色默认跟描边色（形状是造型选择，切预设不动它）。
        self.grip_color = p.border;
        // 投影色跟预设的 `Palette::shadow`（投影是预设的一部分）。
        self.shadow_color = p.shadow;
    }

    /// **切密度档并把三个倍率滑杆对齐到该档的规范值**（与 [`Self::set_preset`] 同思路：
    /// 一键到"标准答案"，之后滑杆仍可自由微调）。
    fn set_density(&mut self, d: Density) {
        let (spacing, font, line) = d.scales();
        self.density = d;
        self.spacing_scale = spacing;
        self.font_scale = font;
        self.line_spacing = line;
    }

    /// 按当前旋钮组装主题（`frame.ui(..)` 之前调用——闭包借用 `self`，闭包内不能构造）。
    fn theme(&self, font: &str, weight: Weight) -> Theme {
        let mut p = preset_palette(self.preset);
        // 表面基色：按**逐通道比**缩放整条 `surface*` 阶梯 —— 既改亮度也改色相，
        // 同时保持"凹陷 / 面板 / 抬升 / 浮层 / 悬停 / 激活"之间的相对关系不塌。
        let base = rgb_of(p.surface);
        let bg = rgb_of(self.bg);
        let f = [
            bg[0] / base[0].max(0.02),
            bg[1] / base[1].max(0.02),
            bg[2] / base[2].max(0.02),
        ];
        let sc = |c: rjw_krusie::color::Color| scale_rgb(c, f);
        p.surface_dim = sc(p.surface_dim);
        p.surface_sunken = sc(p.surface_sunken);
        p.surface_raised = sc(p.surface_raised);
        p.surface_overlay = sc(p.surface_overlay);
        p.surface_hover = sc(p.surface_hover);
        p.surface_active = sc(p.surface_active);
        p.surface = self.bg;
        // 描边：常规 + 强描边（后者更亮，用于按钮 / 输入框 / 勾选框）。
        p.border = self.border;
        p.border_strong = scale_luma(self.border, 1.25);
        p.bevel = self.bevel;
        p.accent = self.accent;
        p.accent_hover = scale_luma(self.accent, 1.25);
        p.accent_active = scale_luma(self.accent, 0.80);
        let t = Theme::themed(&p)
            .with_radius(self.radius)
            .with_feather(self.feather)
            .with_border_w(self.border_w)
            // **布局密度**：先铺密度档，再叠三个手动倍率（倍率是"在现值上叠乘"，
            // 故顺序无关；密度档只负责给出起点）。
            .density(self.density)
            .with_font_scale(self.font_scale)
            .with_spacing_scale(self.spacing_scale)
            .with_line_spacing(self.line_spacing)
            // **全局字重**（`字体…` 弹窗里选；默认 400 = 与扩展前逐像素一致）。
            .with_font_weight(weight);
        // **投影**：模糊宽 + 颜色（偏移沿用预设令牌）；模糊宽拖到 0 = 平面风格。
        // `with_shadow` 吃 `self`，故先拷出现值再整体替换（`ShadowStyle` 是 `Copy`）。
        let shadow = t.panel.shadow;
        let mut t = if self.shadow_blur > 0.0 {
            t.with_shadow(ShadowStyle {
                blur: self.shadow_blur,
                color: self.shadow_color,
                ..shadow
            })
        } else {
            t.without_shadow()
        };
        if !font.is_empty() {
            t = t.with_font_family(font);
        }
        // **右下角缩放柄**：形状 + 颜色（尺寸/个数沿用令牌默认）。只对固定宽窗口生效
        // （`win_a` / `img_box`）；`GripShape::Hidden` 时图案不画但**仍可拖动缩放**。
        t.panel.grip = GripStyle {
            shape: self.grip_shape,
            color: self.grip_color,
            ..t.panel.grip
        };
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
                        self.set_preset(0);
                    }
                    if w.button("th_light", "light").clicked() {
                        self.set_preset(1);
                    }
                    if w.button("th_legacy", "legacy").clicked() {
                        self.set_preset(2);
                    }
                });
                w.row(|w| {
                    w.label("圆角");
                    self.radius = w.slider("th_radius", 0.0..=24.0, self.radius);
                    // **滑杆后跟数字条**（`NumberInput`）：同一根滑杆的"精确输入"入口——
                    // 拖滑杆粗调、在数字条上拖动 / 点击输入精确值（两者绑的是**同一个
                    // `&mut f32`** ⇒ 天然同步；`step` 同时决定拖动吸附台阶与显示小数位）。
                    w.add(NumberInput::new("th_radius_n", &mut self.radius).range(0.0, 24.0).step(0.5));
                });
                w.row(|w| {
                    w.label("羽化");
                    self.feather = w.slider("th_feather", 0.0..=5.0, self.feather);
                    w.add(NumberInput::new("th_feather_n", &mut self.feather).range(0.0, 5.0).step(0.1));
                });
                w.row(|w| {
                    w.label("微渐变");
                    self.bevel = w.slider("th_bevel", 0.0..=0.35, self.bevel);
                    w.add(NumberInput::new("th_bevel_n", &mut self.bevel).range(0.0, 0.35).step(0.01));
                });
                // 三组颜色改用 [`ColorPicker`]：内联色块（内含 `#RRGGBB`）→ 点开取色面板
                // （u8/HEX/F 呈现 + HSV 平面 + 通道行）。强调色额外开 Alpha 行
                //（演示 `.alpha(true)`）。**不必自己持有 String**：面板的文本缓冲
                // 来自全局跨帧状态（`ColorPickerState::text`）。
                // 并排放一行，省纵向空间。
                w.row(|w| {
                    w.add(ColorPicker::new("th_bg", &mut self.bg));
                    w.add(ColorPicker::new("th_border", &mut self.border));
                    w.add(ColorPicker::new("th_accent", &mut self.accent).alpha(true));
                });
                w.row(|w| {
                    w.label("边框宽");
                    self.border_w = w.slider("th_bdw", 0.0..=5.0, self.border_w);
                    w.add(NumberInput::new("th_bdw_n", &mut self.border_w).range(0.0, 5.0).step(0.5));
                });
                // 右下角**缩放柄**：形状三档 + 颜色。只对固定宽窗口生效（`win_a` / `img_box`）；
                // "不画"只是没有图案，**拖动缩放照旧**（命中区单独存在，见 `GripStyle`）。
                w.row(|w| {
                    w.label("拖拽柄");
                    if w.button("th_grip_sq", "方块").clicked() {
                        self.grip_shape = GripShape::Squares;
                    }
                    if w.button("th_grip_bar", "三横").clicked() {
                        self.grip_shape = GripShape::Bars;
                    }
                    if w.button("th_grip_off", "不画").clicked() {
                        self.grip_shape = GripShape::Hidden;
                    }
                    w.add(ColorPicker::new("th_grip_color", &mut self.grip_color));
                });
                // ── 布局密度（主题扩展：紧凑 / 标准 / 宽松）──────────────────
                // 三个按钮一键铺开"间距 / 字号 / 行距"，三根滑杆随后可自由微调
                // （点档位 = 把滑杆对齐到该档的规范值，与"预设"按钮同思路）。
                w.row(|w| {
                    w.label("密度:");
                    if w.button("th_cmp", "紧凑").clicked() {
                        self.set_density(Density::Compact);
                    }
                    if w.button("th_coz", "标准").clicked() {
                        self.set_density(Density::Cozy);
                    }
                    if w.button("th_spa", "宽松").clicked() {
                        self.set_density(Density::Spacious);
                    }
                });
                w.row(|w| {
                    w.label("字号");
                    self.font_scale = w.slider("th_fsc", 0.70..=1.50, self.font_scale);
                    w.add(NumberInput::new("th_fsc_n", &mut self.font_scale).range(0.70, 1.50).step(0.01));
                });
                // 间距与行距并排（省一行纵向空间；两者都是"排布疏密"、常一起调）。
                w.row(|w| {
                    w.label("间距");
                    self.spacing_scale = w.slider("th_ssc", 0.70..=1.50, self.spacing_scale);
                    w.add(NumberInput::new("th_ssc_n", &mut self.spacing_scale).range(0.70, 1.50).step(0.01));
                    w.label("行距");
                    self.line_spacing = w.slider("th_lsp", 0.80..=2.00, self.line_spacing);
                    w.add(NumberInput::new("th_lsp_n", &mut self.line_spacing).range(0.80, 2.00).step(0.05));
                });
                // 投影模糊宽：0 = 不画（平面风格）；拖大 = 窗口"浮"得更高。
                // 后面紧跟**投影颜色**（色令牌 `Palette::shadow`；alpha 也归它管 ⇒
                // 拖 alpha 可以做出"很淡的浮起"或"很重的压深"）。
                w.row(|w| {
                    w.label("投影");
                    self.shadow_blur = w.slider("th_shd", 0.0..=32.0, self.shadow_blur);
                    w.add(NumberInput::new("th_shd_n", &mut self.shadow_blur).range(0.0, 32.0).step(1.0));
                    w.add(ColorPicker::new("th_shadow_color", &mut self.shadow_color).alpha(true));
                });
                w.label(&format!(
                    "{} · 圆角 {:.0} · 羽化 {:.1} · 微渐变 {:.2} · 边框宽 {:.1} · 投影 {:.0}/a{:.0} · 柄 {}",
                    ["dark", "light", "legacy"][self.preset.min(2) as usize],
                    self.radius,
                    self.feather,
                    self.bevel,
                    self.border_w,
                    self.shadow_blur,
                    // 投影色的 alpha（0..1 → 0..255 显示，与色块里的读数一致）。
                    self.shadow_color.a * 255.0,
                    match self.grip_shape {
                        GripShape::Squares => "方块",
                        GripShape::Bars => "三横",
                        GripShape::Hidden => "无",
                    },
                ));
                w.label(&format!(
                    "密度 {} · 字号 ×{:.2} · 间距 ×{:.2} · 行距 ×{:.2}",
                    ["紧凑", "标准", "宽松"][self.density as usize],
                    self.font_scale,
                    self.spacing_scale,
                    self.line_spacing,
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

/// 逐通道缩放（保持色相 / 相对层次；alpha 不变）。
fn scale_rgb(c: rjw_krusie::color::Color, f: [f32; 3]) -> rjw_krusie::color::Color {
    let a: [f32; 4] = c.into();
    Color::from([
        (a[0] * f[0]).clamp(0.0, 1.0),
        (a[1] * f[1]).clamp(0.0, 1.0),
        (a[2] * f[2]).clamp(0.0, 1.0),
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
    /// --sim-picker：**脚本化鼠标**打开取色面板并在其中拖动（面板路径只有点击才录制）。
    sim_picker: bool,
    /// --sim-overlap：**脚本化鼠标**点击两个重叠控件的交集——验证"只有上层被触发"。
    sim_overlap: bool,
    /// --sim-click X,Y：**脚本化鼠标**在指定屏幕物理点按下 + 释放（配合 `RJ_HIT_TRACE=1`
    /// 排查"这一像素到底命中了谁"：重叠 / 相邻控件边界、跨窗口遮挡、滚动条条带）。
    sim_click: Option<Vec2>,
    /// 「重叠控件」演示（两个探针的点击计数）。
    overlap: OverlapDemo,
    /// --sim-cover：**脚本化复现**"被上层窗口盖住的控件仍收到按下"（见 `cover` 模块）。
    sim_cover: bool,
    /// --sim-chrome：**脚本化驱动窗口外框**（标题栏 / 关闭 × / 收起）——在固定帧翻转
    /// `win_a_open` / `win_a_collapsed`，并由 `Windows` 打印每次结算尺寸变化
    /// （收起 ⇒ 高度塌到一行标题栏；关闭 ⇒ `(0,0)` 整窗短路）。见 docs/DEBUGGING.md。
    sim_chrome: bool,
    /// --sim-weight：第 20 帧（字重 400）量到的文本宽；第 40 帧（字重 700）对比用
    /// （**硬断言**：字重必须真的进排版输入 ⇒ 宽度必须变）。
    weight_probe: Option<f32>,
    /// --sim-weight：**脚本化切字重**（第 30 帧 NORMAL → BOLD），前后量同一串文本。
    sim_weight: bool,
    /// --sim-shadow：**脚本化改投影颜色**（第 30 帧换成红色），打印主题里的
    /// `panel.shadow.color` 证明"色块 → ShadowStyle → 主题"这条线是通的。
    sim_shadow: bool,
    /// --sim-shadow：第 20 帧的主题投影色（第 40 帧对比用）。
    shadow_probe: Option<Color>,
    /// --sim-tuner：脚本化鼠标的两个目标点（**录制时运行时解算**）：
    /// `(圆角数字条中心, 圆角滑杆中心)`。注入只能经 `Frame`（录制中 `f` 被借着）
    /// ⇒ 段末（`ui.finish()` 之后）再按帧注入——注入本来就下一帧才生效。
    sim_tuner_pts: Option<(Vec2, Vec2)>,
    /// --sim-tuner：拖数字条后的圆角值（第二阶段判定"数字条真的改了值"）。
    tuner_probe: Option<f32>,
    /// --sim-tuner：**实操主题调节窗口里的"滑杆 + 数字条"**（坐标运行时解算，不写死像素）。
    sim_tuner: bool,
    /// --sim-import <路径>：脚本化导入（**不弹对话框**，走同一条应用通路）——验证
    /// "字节 → 纹理 / 字体"这条线（真人点选择器那步无法在无头环境里跑）。
    sim_import: Option<String>,
    /// 文件导入：**已选好、等帧内应用**的图片路径（要 `f.draw().gpu()`；`Ctx` 在帧外
    /// 拿不到 `Gpu`）。请求与状态在 [`TopBar`]（那是显示它们的模块）。
    import_image_path: Option<std::path::PathBuf>,
    /// 「被遮挡控件仍被触发」复现器。
    cover: CoverDemo,
    /// `--sim-cover` 段 A 结束时的认领次数（段 B 不许再涨）。
    cover_starts_after_a: u32,
    /// --image <路径>：**加载用户图片文件**当背景图（`ImageBg`；PNG / JPEG / BMP / GIF），
    /// 替代内建棋盘纹理——四个窗口仍分别演示四种铺排（`ImageFit`）。
    image_file: Option<String>,
    /// --font-file <路径>：**加载用户字体文件**（ttf / otf / ttc）到运行时文本子系统
    /// （`Ctx::text_mut().load_font_data`）——之后 `字体…` 弹窗里输入该字体的**族名**
    /// 即可全局换字（下一帧按族名重建主题）。
    font_file: Option<String>,
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
    /// **应用一个导入路径**（「导入图片…」/「导入字体…」选择器、`--font-file`、
    /// `--sim-import` **共用**的那条线）：
    ///
    /// - **字体**：当场加载进**运行时**文本子系统（`Ctx::text_mut()`；应用自建的
    ///   `Gfx::text()` 是另一套图集，UI 不会用），成功则**自动切到新族名**（导入即刻生效，
    ///   「字体…」弹窗仍可换回去）。`ttc` 会一次进多个族，取第一个。
    /// - **图片**：只记待办——`Gpu` 只能在帧内从 `f.draw()` 拿到（见 `update` 里的
    ///   "图片导入要帧内应用"那段）。
    /// - **失败不致命**：原因写进顶栏状态（`TopBar::import_status`），画面照旧。
    ///
    /// `verbose`：`--font-file` 这类 CLI 开关额外打一行 stderr（脚本 / 日志里可见）。
    fn apply_import(&mut self, ctx: &mut Ctx, path: std::path::PathBuf, verbose: bool) {
        let label = filedialog::file_label(&path);
        match ImportKind::from_path(&path) {
            Some(ImportKind::Font) => {
                let Some(text) = ctx.text_mut() else {
                    self.top.import_status = "字体导入失败：文本子系统不可用".to_owned();
                    return;
                };
                match filedialog::apply_font(text, &path) {
                    Ok(fams) if fams.is_empty() => {
                        // 已加载但族已存在（系统字体默认已在库里）——不是失败。
                        self.top.import_status = format!("字体：{label}（该族已在库里）");
                        if verbose {
                            eprintln!("--font-file: 已加载 {label}（族已在库里，无新增）");
                        }
                    }
                    Ok(fams) => {
                        let first = fams[0].clone();
                        self.top.font_name = first.clone();
                        self.top.import_status = format!("字体：{label} → {first}");
                        if verbose {
                            eprintln!(
                                "--font-file: 已加载 {label} → 族名 {first}（其余新增：{:?}）",
                                &fams[1..]
                            );
                        }
                    }
                    Err(e) => self.top.import_status = format!("字体导入失败：{e}"),
                }
            }
            Some(ImportKind::Image) => self.import_image_path = Some(path),
            None => {
                self.top.import_status = format!("不认得的文件类型：{label}（要图片或 ttf/otf/ttc）");
                if verbose {
                    eprintln!("--font-file: 不认得的文件类型 {label}");
                }
            }
        }
    }

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
            sim_picker: false,
            sim_overlap: false,
            sim_click: None,
            overlap: OverlapDemo::default(),
            sim_cover: false,
            sim_chrome: false,
            sim_weight: false,
            sim_shadow: false,
            shadow_probe: None,
            sim_tuner_pts: None,
            tuner_probe: None,
            sim_tuner: false,
            sim_import: None,
            import_image_path: None,
            weight_probe: None,
            cover: CoverDemo::default(),
            cover_starts_after_a: 0,
            image_file: None,
            font_file: None,
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
    /// 各段开场（引擎：懒开场 / 冻结输入 / 装载帧级事实 / 建根容器）累计。
    prologue_us: f64,
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
            prologue_us: 0.0,
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
        self.prologue_us += s.prologue_us;
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
        // `record` = 应用侧录制（控件代码 + 主题构造）= 整帧 − 引擎开场 − 引擎收尾。
        // 三分解是"UI 时间翻倍"排查的第一把尺子：引擎 bug 还是应用内容增长，一眼可分。
        let ui_ms = self.ui_us / n / 1000.0;
        let prologue_ms = self.prologue_us / n / 1000.0;
        let finish_ms = self.finish_us / n / 1000.0;
        let record_ms = (ui_ms - prologue_ms - finish_ms).max(0.0);
        println!(
            "[perf] fps={fps:.0} frame={:.2}ms ui={ui_ms:.2}ms (prologue={prologue_ms:.2} \
             record={record_ms:.2} finish={finish_ms:.2}) \
             | ui: sort={:.1}us sig={:.1}us collect={:.1}us clone={:.1}us submit={:.1}us \
             | render: total={:.2}ms begin={:.1}us encode={:.1}us submit={:.1}us present={:.1}us \
             | cmds={:.0} wins={:.0} cache_hit={:.0} cache_miss={:.0}",
            self.frame_us / n / 1000.0,
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
        // **背景图演示用的棋盘纹理**：8×8 格、每格 4px ⇒ 32×32。
        // `Gfx::texture` 会把它注册进纹理表 → UI 侧只持 **uid**（`ImageBg::tex`），
        // 后端按 uid 解析（`UiBackend::texture`）。对角线染色便于看清平铺接缝。
        const TEX: u32 = 32;
        let mut px = vec![0u8; (TEX * TEX * 4) as usize];
        for y in 0..TEX {
            for x in 0..TEX {
                let dark = ((x / 4) + (y / 4)) % 2 == 0;
                let i = ((y * TEX + x) * 4) as usize;
                let c: [u8; 4] = if dark {
                    [70, 110, 170, 255]
                } else {
                    [40, 60, 100, 255]
                };
                px[i..i + 4].copy_from_slice(&c);
            }
        }
        let mut tex = gfx.texture("eg260818UI.pattern", Rgba8::new(&px, (TEX, TEX)));
        let mut size = Vec2::new(TEX as f32, TEX as f32);
        // ── `--image <路径>`：用**用户自己的图片文件**替换棋盘纹理 ────────
        // 解码由 `image` crate 完成（应用侧依赖，引擎不背），产出 RGBA8 →
        // `Gfx::texture` → 同一套 `ImageBg` 通路（Fill / Tile / 九宫…全靠它）。
        // 失败**不致命**（退回棋盘）并打印原因：示例要能离线裸跑。
        if let Some(path) = &self.image_file {
            match image::open(path) {
                Ok(img) => {
                    let img = img.to_rgba8();
                    let (w, h) = img.dimensions();
                    tex = gfx.texture("eg260818UI.user_image", Rgba8::new(img.as_raw(), (w, h)));
                    size = Vec2::new(w as f32, h as f32);
                    eprintln!("--image: 已加载 {w}×{h} 图片 ← {path}");
                }
                Err(e) => {
                    eprintln!("--image: 加载失败（退回内建棋盘纹理）{path}: {e}");
                }
            }
        }
        self.windows.bg_image = Some(ImageBg::new(tex.uid, size));
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
        // `--sim-picker` 需要 DPI（`Theme` 在 `Ui` 内才被 `scaled`——主题 builder 返回的是
        // 未缩放值），而 `f` 借走 `ctx` 后不能再读，故先取。
        let scale = ctx.scale();
        // ── 文件导入：**对话框在帧外弹**（`rfd` 阻塞，且 `f` 会借走 `ctx`）──────
        // 三条来源共用 `apply_import`：
        //   ① 「导入图片…」/「导入字体…」按钮 → 弹系统文件选择器；
        //   ② `--font-file <路径>` → 启动期直接加载（老开关，走同一条通路）；
        //   ③ `--sim-import <路径>` → 脚本化导入（**不弹对话框**，验证字节→资源这条线）。
        if let Some(kind) = self.top.import_request.take() {
            match filedialog::pick(kind) {
                Some(path) => self.apply_import(ctx, path, false),
                None => self.top.import_status = "导入已取消".to_owned(),
            }
        }
        if let Some(path) = self.font_file.take() {
            self.apply_import(ctx, std::path::PathBuf::from(path), true);
        }
        if let Some(p) = self.sim_import.clone()
            && ctx.frames() == 20
        {
            self.apply_import(ctx, std::path::PathBuf::from(p), false);
        }
        let Some(mut f) = ctx.frame() else {
            return;
        };
        // ── 图片导入要**帧内**应用：`Gpu` 只能从 `f.draw()` 拿到（`Ctx` 没有 `Gpu`）──
        // 放在这里（任何录制之前）⇒ 本帧 `self.windows.ui(..)` 就用上新纹理，不等下一帧。
        if let Some(path) = self.import_image_path.take() {
            match filedialog::import_image(f.draw().gpu(), "eg260818UI.imported_image", &path) {
                Ok(bg) => {
                    self.top.import_status = format!(
                        "图片：{} {:.0}×{:.0}",
                        filedialog::file_label(&path),
                        bg.texel.x,
                        bg.texel.y
                    );
                    self.windows.bg_image = Some(bg);
                }
                Err(e) => self.top.import_status = format!("图片导入失败：{e}"),
            }
        }
        // ── 调试：脚本化鼠标（`--sim-picker`）──────────────────────
        // 复现"打开取色面板 → 在面板里拖/点"：面板路径（SV 平面 / 色相条 / 通道滑块 /
        // 文本框 / **警告按钮恢复** / 模式切换）只有交互才会录制，普通冒烟跑不到——
        // 本开关守住"面板真能开、拖动真改色、不 panic、无 wgpu 校验错误"。
        //
        // 坐标**由主题解算**（`Theme` 在 `Ui` 内才按 DPI 预乘，故这里手动乘 `scale`；
        // 布局常量与 `widgets/colorpicker/panel.rs` 对齐）——写死像素在非 1.0 DPI 下会点空。
        if self.sim_picker {
            let n = f.frames();
            let theme = self
                .theme_tuner
                .theme(self.top.font_name(), self.top.font_weight());
            // 取色面板内部的固定常量（与 `panel.rs` 一致）+ 主题尺寸 × DPI。
            let (pad, gap, hue_w, label_w, slider_min) =
                (6.0f32, 6.0f32, 14.0f32, 14.0f32, 90.0f32);
            let (row, input_h, field_w) = (
                theme.row_h * scale,
                theme.input.height * scale,
                theme.input.min_w * scale,
            );
            let anchor = Vec2::new(240.0, 250.0); // `TopBar` 里 picker_demo 的物理定位
            let swatch = Vec2::new(anchor.x + field_w * 0.5, anchor.y + 11.0);
            let pw = (field_w * 1.9).max(pad * 2.0 + label_w + gap + slider_min + gap + field_w);
            let body_w = pw - pad * 2.0;
            let origin = Vec2::new(anchor.x, anchor.y + 24.0); // 内联高 22 + 2px 间隙
            let mode_w = (body_w - gap * 2.0) / 3.0;
            let text_y = origin.y + pad + row + gap;
            let sv_y = text_y + input_h + gap;
            // SV 平面是**正方形**（边长 = 内容宽去掉色相条与间隙），与 `panel.rs` 一致。
            let sv_w = body_w - hue_w - gap;
            let sv_h = sv_w;
            let mode = |i: usize| {
                Vec2::new(
                    origin.x + pad + (mode_w + gap) * i as f32 + mode_w * 0.5,
                    origin.y + pad + row * 0.5,
                )
            };
            let sv = |s: f32, v: f32| Vec2::new(origin.x + pad + s * sv_w, sv_y + v * sv_h);
            let hue = |t: f32| Vec2::new(origin.x + pad + sv_w + gap + hue_w * 0.5, sv_y + t * sv_h);
            let text_box = Vec2::new(origin.x + pad + 24.0, text_y + input_h * 0.5);
            let warn = Vec2::new(origin.x + pad + body_w - input_h * 0.5, text_y + input_h * 0.5);
            let ch_row = |ch: usize| sv_y + sv_h + gap + row * (ch as f32 + 0.5);
            let slider = |ch: usize, t: f32| {
                let sw = body_w - label_w - gap - field_w - gap;
                Vec2::new(origin.x + pad + label_w + gap + sw * t, ch_row(ch))
            };
            match n {
                20 => f.debug_inject_mouse(swatch, true), // 内联色块
                21 => f.debug_inject_mouse(swatch, false), // → 点开面板
                24 => f.debug_inject_mouse(sv(0.5, 0.5), true), // SV 平面按下
                25..=34 => {
                    let k = (n - 24) as f32 / 10.0;
                    f.debug_inject_mouse(sv(0.2 + k * 0.7, 0.2 + k * 0.6), true)
                }
                35 => f.debug_inject_mouse(sv(0.9, 0.8), false),
                38 => f.debug_inject_mouse(hue(0.0), true), // 色相条按下
                39..=46 => {
                    let k = (n - 38) as f32 / 8.0;
                    f.debug_inject_mouse(hue(k), true)
                }
                47 => f.debug_inject_mouse(hue(0.95), false),
                50 => f.debug_inject_mouse(slider(0, 0.2), true), // R 通道滑块
                51..=54 => {
                    let k = (n - 50) as f32 / 4.0;
                    f.debug_inject_mouse(slider(0, 0.2 + k * 0.8), true)
                }
                55 => f.debug_inject_mouse(slider(0, 1.0), false),
                58 => f.debug_inject_mouse(slider(3, 0.8), true), // A 通道滑块
                59..=62 => f.debug_inject_mouse(slider(3, 0.8 - (n - 58) as f32 * 0.2), true),
                63 => f.debug_inject_mouse(slider(3, 0.0), false),
                66 => f.debug_inject_mouse(text_box, true), // 文本框（聚焦）
                67 => f.debug_inject_mouse(text_box, false),
                74 => f.debug_inject_mouse(warn, true), // 警告按钮（恢复成有效值）
                75 => f.debug_inject_mouse(warn, false),
                82 => f.debug_inject_mouse(mode(2), true), // 切到 F 呈现
                83 => f.debug_inject_mouse(mode(2), false),
                96 => f.debug_inject_mouse(Vec2::new(900.0, 100.0), true), // 点面板外 → 收起
                97 => f.debug_inject_mouse(Vec2::new(900.0, 100.0), false),
                _ => {}
            }
        }

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

        // ── 调试：脚本化鼠标（`--sim-overlap`）────────────────────
        // 点在两个**故意重叠**控件的**交集中心**：期望只有画在上面的那个被触发
        // （修复前两者一起触发 —— "重叠控件被一起触发"）。坐标由 `overlap` 模块解算，
        // 与绘制同源（不写死像素）。
        if self.sim_overlap {
            let p = OverlapDemo::overlap_point();
            // **每帧重注入**（注入只对"下一帧"生效；真实鼠标一动就会把它顶掉 ——
            // 只注一次会让断言随真人手抖而随机失败，`--sim-drag` 同理每帧注入）。
            match f.frames() {
                20..=21 => f.debug_inject_mouse(p, true), // 按下并按住
                22..=58 => f.debug_inject_mouse(p, false), // 释放后仍停在重叠区（到断言帧）
                _ => {}
            }
        }

        // ── 调试：脚本化鼠标（`--sim-cover`）───────────────────────
        // 三段脚本（详见 `cover` 模块文档）：
        //   段 0（帧 5..）**正对照**：移动窗让开 ⇒ 探针没被盖住，按下并按住**必须拖得动**
        //     （曾经被"帧末复核拿旧 z 比"误撤，窗口内控件全拖不动）；
        //   段 A（帧 30）**同帧移动**：移动窗本帧才移到探针上，按下也在本帧；
        //   段 B（帧 60）**应用改 z**：录制前把移动窗 z 抬到最前（不占鼠标键），本帧再按下。
        // 命中点由 `cover` 模块按本帧结算尺寸解算（`overlap` 模块同一套"不写死像素"做法）。
        // z 的改动在段内、`cover.ui` **之前**做（见下方录制处）——那正是"应用置顶"的时机。
        let mut cover_over = false;
        if self.sim_cover {
            let n = f.frames();
            let p = self.cover.probe_point;
            cover_over = n >= 30;
            // 注入**只对下一帧生效**（引擎把边沿合成到下一帧快照）。于是要让"按下那一帧"
            // 正好是"几何变化那一帧"（缺陷窗口），注入必须比几何变化**早一帧**：
            // 段 A 几何在 `n = 30` 变（按下注入在 n=30 ⇒ 落在同一帧）；
            // 段 B z 在 `n = 59` 变（注入同样在 n=59）。
            match n {
                1..=4 => f.debug_inject_mouse(Vec2::new(1800.0, 1050.0), false),
                5..=18 => f.debug_inject_mouse(p, true), // 段 0：正对照——应当拖得动
                19..=29 => f.debug_inject_mouse(Vec2::new(1800.0, 1050.0), false),
                30..=33 => f.debug_inject_mouse(p, true), // 段 A：同帧移动 + 按下
                34..=58 => f.debug_inject_mouse(Vec2::new(1800.0, 1050.0), false),
                59..=62 => f.debug_inject_mouse(p, true), // 段 B：同帧改 z + 按下
                63..=75 => f.debug_inject_mouse(Vec2::new(1800.0, 1050.0), false),
                _ => {}
            }
        }

        // ── 调试：脚本化鼠标（`--sim-chrome`）────────────────────
        // **真的去点**标题栏那两个按钮（不是直接翻 flag）：命中 → 按下认领 → 释放结算
        // 这条完整路径才被验证。坐标由**主题尺寸解算**（不写死像素）：按钮在内容行右端，
        // 从右往左依次是 × 与 ⌃，边长 `row_h - 2`（同 `TitleIconButton::size`）。
        //
        // 调度（注入只对**下一帧**生效 ⇒ 按下/抬起各留两帧）：
        //   12..13 ↓⌃ / 14..15 ↑⌃（点收起）→ 60 帧由应用重开（等价菜单勾选）
        //   40..41 ↓× / 42..43 ↑×（点关闭）→ 80 帧由应用展开
        if self.sim_chrome {
            // 尺寸取自**本帧主题**（与下面 `let theme` 同一套输入 ⇒ 值一致）；
            // `Theme` 在 `Ui` 内才按 DPI 预乘 ⇒ 这里手动乘 `scale`（同 `--sim-picker`）。
            let th = self
                .theme_tuner
                .theme(self.top.font_name(), self.top.font_weight());
            let (pad, row, gap) =
                ((th.panel.padding + th.panel.border_w) * scale, th.row_h * scale, th.gap * scale);
            // 按钮边长 = `row_h - 2`，**减号作用在已缩放的 row_h 上**（同 `TitleIconButton::size`）。
            let btn = (row - 2.0).max(12.0);
            let origin = (self.windows.win_a_pos * scale).round();
            let right = origin.x + 220.0 * scale + pad; // 行右缘 = 内容右缘
            let cy = origin.y + pad + row * 0.5;
            let close_p = Vec2::new(right - btn * 0.5, cy);
            let fold_p = Vec2::new(right - btn * 1.5 - gap, cy);
            let away = Vec2::new(1800.0, 1050.0);
            if f.frames() == 12 {
                eprintln!("sim-chrome: scale={scale} ⌃={fold_p:?} ×={close_p:?}");
            }
            match f.frames() {
                // 先把 win_a 挪到**没有别的窗口压着**的空位（默认布局里 win_b / chishi 正盖着
                // 它的右上角——那正是"点击置顶"演示）：× 被压住时点不到，这是引擎**正确**的
                // 遮挡行为；脚本要的只是让按钮露出来（同 `--sim-cover` 调 z）。
                // 空位 `(40,470)`（逻辑）= 物理 `(60,705)`：`theme_tuner` 从 x=420 起、
                // `strict_win` 从 y=690 起，两者都够不着这块。
                8 => self.windows.win_a_pos = Vec2::new(40.0, 470.0),
                12..=13 => f.debug_inject_mouse(fold_p, true),
                14..=15 => f.debug_inject_mouse(fold_p, false),
                40..=41 => f.debug_inject_mouse(close_p, true),
                42..=43 => f.debug_inject_mouse(close_p, false),
                _ => {
                    f.debug_inject_mouse(away, false);
                    // 应用侧重开 / 展开（引擎不替应用决定"何时重开"）。
                    match f.frames() {
                        60 => self.windows.win_a_open = true,
                        80 => self.windows.win_a_collapsed = false,
                        _ => {}
                    }
                }
            }
        }
        // ── 调试：脚本化鼠标（`--sim-tuner`）──────────────────────
        // ⚠ **必须在这里注入**（各段 `f.ui(..)` 之前）：`MouseInput::end_frame` 每帧末
        // 清掉边沿位（`off_edge`）⇒ 在段末注入的 `down_edge` 会被同一帧的收尾吃掉，
        // 下一帧只剩 `pressed`（症状：命中正常、`update_drag` 却永远不开始拖）。
        // 坐标由**上一帧录制时**解算（`sim_tuner_pts`，见段 2 里的 `--sim-tuner` 块）。
        //
        // 行程：10..11 移到数字条手柄上（悬停）→ 12..13 按下 → 14..17 按住右移 20px
        // （数字条：每像素 = step × 灵敏度 ⇒ step 0.5 × 20px = +10）→ 18..19 抬起；
        // 30..31 按下滑杆 → 32..35 拖到轨道最左（圆角归 0）→ 36..43 抬起。
        if self.sim_tuner {
            let (num_grip, slider_c) = self.sim_tuner_pts.unwrap_or((Vec2::ZERO, Vec2::ZERO));
            let right = Vec2::new(num_grip.x + 20.0, num_grip.y);
            let left = Vec2::new(slider_c.x - 200.0, slider_c.y);
            match f.frames() {
                10..=11 => f.debug_inject_mouse(num_grip, false),
                12..=13 => f.debug_inject_mouse(num_grip, true),
                14..=17 => f.debug_inject_mouse(right, true),
                18..=19 => f.debug_inject_mouse(right, false),
                30..=31 => f.debug_inject_mouse(slider_c, true),
                32..=35 => f.debug_inject_mouse(left, true),
                36..=43 => f.debug_inject_mouse(left, false),
                _ => {}
            }
        }
        // ── 调试：脚本化鼠标（`--sim-click X,Y`）──────────────────
        // 在**指定屏幕物理点**按下 + 释放（第 20/21 帧，之后停在原地到第 40 帧）——
        // 配合 `RJ_HIT_TRACE=1`（引擎打印每次命中归属）就能回答"这一像素到底是谁的"：
        // 重叠 / 相邻控件的边界、跨窗口遮挡、滚动条条带都能一眼定位。
        // 第 22 帧起鼠标一直停在原地（**到断言帧之后**）⇒ 读数不被真实鼠标移动顶掉。
        if let Some(p) = self.sim_click {
            match f.frames() {
                20..=21 => f.debug_inject_mouse(p, true),
                22..=48 => f.debug_inject_mouse(p, false),
                _ => {}
            }
        }

        // 主题由 [`ThemeTuner`] 每帧组装（预设调色板 + 圆角 / 羽化 / 微渐变 / 强调色，
        // 以及 FontModal 选定的字体族）。两段 UI 各传一份（`Theme` 可克隆）。
        let theme = self
            .theme_tuner
            .theme(self.top.font_name(), self.top.font_weight());
        // 段 2 会**吃掉** `theme`（`f.ui(theme)`）；本帧主题里的圆角先拷出来，
        // 供 `--sim-tuner` 的段末判定读（`CornerRadius` 是 `Copy`）。
        let panel_radius = theme.panel.radius;

        // 性能统计（UI 段 2 里读取"上一帧收尾"写入的值；延迟初始化避免多余默认值）。
        let ui_stats: UiStats;
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
        // --sim-picker：文本框聚焦后**在固定几帧内**注入不可识别文本（非法文本只在聚焦时
        // 得以保留——非聚焦会被每帧重写）⇒ 守护"警告按钮 + 按下恢复"这条路径。
        // ⚠ 必须限定帧窗口：本闭包每帧新建，用 `bool` 会每帧都注入（覆盖恢复结果）。
        let sim_bad_text_frames = 66..=72;
        let sim_frame = f.frames();
        let sim_picker = self.sim_picker;
        let sim_overlap = self.sim_overlap;
        let sim_click = self.sim_click;
        // `--sim-cover`：本帧移动窗是否移到探针上（脚本解算，与录制同源）。
        let sim_cover = self.sim_cover;
        // `--sim-overlap`：本帧被**控件级遮挡**拦下的命中次数（重叠区里下层探针的那次）。
        let mut widget_blocked = 0u32;
        // 本帧被**窗口遮挡**拦下的命中次数（`--sim-click` 诊断用）。
        let mut window_blocked = 0u32;

        // ── UI 层（段 1）：状态栏 / 菜单 / 背包 / 窗口 / 重叠探针 ───────────────────
        // 「ui anywhere」：`f.ui(theme)` 开**一段**录制；一帧可开任意多段、位置随意，
        // 段之间可以穿插世界层绘制 / 世界文本（段存续期间 `f` 被借用 ⇒ 不能 draw/submit）。
        // 帧级账（帧号 / 命中区翻页 / 输入快照）在第一段**懒开场**，后续段复用。
        let t_ui = Instant::now();
        {
            let mut ui = f.ui(theme.clone());
            if sim_picker
                && sim_bad_text_frames.contains(&sim_frame)
                && ui
                    .state()
                    .focused
                    .as_ref()
                    .is_some_and(|f| f.as_str().contains("picker_demo"))
            {
                ui.state_mut().color_picker.text = "zzz".into();
            }
            // 警告按钮按下（第 74 帧注入，位置/边沿下一帧生效）之后文本框应是**有效文本**
            // （不是注入的 "zzz"）；第 84 帧再看一眼：模式已切到 F ⇒ 呈现应是 `0.00, ...`。
            if sim_picker && (sim_frame == 80 || sim_frame == 86) {
                let st = ui.state();
                eprintln!(
                    "sim-picker: frame={sim_frame} mode={:?} text={:?}",
                    st.color_picker.mode,
                    st.color_picker.text
                );
            }
            // ── `--sim-import`：**导入的字体真的进了排版**吗 ────────────────────
            // 第 20 帧应用导入（帧外），这里在第 30 帧用**导入的族名**与"不指定族名"
            // （系统默认）各量一次同一串文本：宽度必须不同 ⇒ 新字体真的参与了整形
            // （族名写错 / 没进运行时字体库时，cosmic-text 会回落到默认族 ⇒ 两者相等）。
            if self.sim_import.is_some() && sim_frame == 30 {
                let size = ui.theme().label.font_size;
                let fam = self.top.font_name.clone();
                // 样本刻意混排（汉字 + 拉丁 + 数字）：导入字体与回落字体在任一类字形上
                // 步进不同都会体现出来，比"只量两个汉字"更不容易撞上等宽巧合。
                const SAMPLE: &str = "字体导入测试 ABCDEFG 0123456789";
                let w_default = ui.text_size(SAMPLE, size, None).x;
                let w_imported = ui.text_size(SAMPLE, size, Some(&fam)).x;
                let ok = !fam.is_empty() && (w_default - w_imported).abs() > 0.5;
                eprintln!(
                    "sim-import: 排版实测「{SAMPLE}」默认族 {w_default:.1} vs 导入族 {fam:?} {w_imported:.1} {}",
                    if ok {
                        "[OK] 导入的字体真的参与了整形"
                    } else {
                        "[FAIL] 导入的族名没生效（回落默认族）"
                    }
                );
            }
            // ── `--sim-shadow`：投影颜色的**主题通路**（色块 → `ShadowStyle::color`
            //    → 主题 → 镶嵌）。第 30 帧把投影换成半透明红，前后各打印一次主题值：
            //    主题值必须真的变（改色块没反应 = 主题没吃这个令牌）。
            //    ⚠ 引擎侧的"任意色都原样带出顶点"由单测
            //    `tess::tests::shadow_keeps_the_callers_rgb_and_alpha` 守着。
            if self.sim_shadow {
                if sim_frame == 30 {
                    self.theme_tuner.shadow_color = Color::rgba(0.9, 0.15, 0.1, 0.55);
                }
                if sim_frame == 20 || sim_frame == 40 {
                    let c = ui.theme().panel.shadow.color;
                    eprintln!(
                        "sim-shadow: frame={sim_frame} panel.shadow.color = ({:.2},{:.2},{:.2},a{:.2})",
                        c.r, c.g, c.b, c.a
                    );
                    match self.shadow_probe {
                        None => self.shadow_probe = Some(c),
                        Some(c0) => {
                            // **判定**：主题里的投影色必须真的跟着色块变（RGB 与 alpha 都比）。
                            let ok = (c.r - c0.r).abs() > 0.1 || (c.a - c0.a).abs() > 0.05;
                            eprintln!(
                                "sim-shadow: 主题投影色 {} {}",
                                if ok { "已跟随色块变化" } else { "没跟着变" },
                                if ok {
                                    "[OK] 阴影颜色进了主题"
                                } else {
                                    "[FAIL] 阴影颜色没进主题（色块白调）"
                                }
                            );
                        }
                    }
                }
            }
            // ── `--sim-weight`：字重是**排版输入**（改字形 + 步进宽度）──
            // 第 30 帧 NORMAL → BOLD，并在 20 / 40 帧量同一串文本：宽度应变化
            // （字体没有该字面时 cosmic-text 回落最接近的字面，宽度可能不变 ⇒ 打印是
            // **观察证据**；硬断言在单测：字重进缓冲缓存键 + 进窗口几何签名前缀）。
            if self.sim_weight {
                if sim_frame == 30 {
                    self.top.font_weight = Weight::BOLD;
                }
                if sim_frame == 20 || sim_frame == 40 {
                    // 主题是 `&`（`ui.theme()`），先拷出需要的值再 `text_size(&mut self)`。
                    let (size, fam, weight) = {
                        let t = ui.theme();
                        (t.label.font_size, t.label.font_family.clone(), t.font_weight)
                    };
                    let w = ui.text_size("字重 Aa 123", size, fam.as_deref()).x;
                    eprintln!(
                        "sim-weight: frame={sim_frame} weight={} '字重 Aa 123' width={w:.1} (label size {size})",
                        weight.0
                    );
                    match self.weight_probe {
                        None => self.weight_probe = Some(w),
                        Some(w0) => {
                            // **硬断言**：400 → 700 后同一串文本的实测宽必须变化
                            // ——字重真的进了排版输入，而不是只记在主题里没被用上。
                            let ok = (w - w0).abs() > 0.5;
                            eprintln!(
                                "sim-weight: weight 400 → {} : width {w0:.1} → {w:.1} {}",
                                weight.0,
                                if ok {
                                    "[OK] 字重真的改变了字形 / 步进宽度"
                                } else {
                                    "[FAIL] 字重没进排版输入"
                                }
                            );
                        }
                    }
                }
            }
            // ── 应用快捷键：**文本输入框聚焦时屏蔽**（`UiState::text_focus()`）——
            //    输入 `R` / `Esc` 不会被当作重置 / 退出。
            //    与旧 `capturing_text()`（任何控件持焦点都为真）不同：只有**文本控件**
            //    持焦点才屏蔽快捷键（按钮/滑块 Tab 焦点不吞应用按键）。
            if ui.state().text_focus().is_none() {
                if ui.key_down_edge(KeyCode::Escape) {
                    exit_requested = true;
                }
                if ui.key_down_edge(KeyCode::KeyR) {
                    reset_ui_state(ui.state_mut());
                }
            }
            // ── 位置责任链演示（--script-pos）：脚本让窗口 A 沿正弦摆动 ──
            // 处理器优先级 -10（< 0）：**用户拖拽优先**——拖住 A 时脚本让位、窗口跟手，
            // 松开后停在放置处；不拖时脚本每帧驱动位置（脚本"动画"，拖动"覆盖"）。
            // ⚠ 责任链是**帧级暂存**（跨段共享）：段 1 注册的处理器对段 2 录制的窗口
            //    同样生效。
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
            self.top.ui(&mut ui, fps, clicks, &mut self.theme_tuner);
            self.menu.ui(&mut ui, &mut clicks);
            self.inventory.ui(&mut ui);
            self.windows.ui(&mut ui, &mut clicks);
            // 重叠控件探针（顶层 win=0；位置在全屏所有窗口下方 ⇒ 不会被窗口遮挡）。
            self.overlap.ui(&mut ui);
            // 「被遮挡控件仍被触发」复现（`--sim-cover`）：z 改动在**录制前**做——那正是
            // "应用把某窗口置顶"的时机（本帧它按新 z 绘制，而遮挡表里还是上一帧的旧 z）。
            if sim_cover {
                if sim_frame == 25 {
                    // 段 0 的按下把探针窗置顶了 ⇒ 复位两窗 z，恢复段 A 的前置条件
                    // （"移动窗画在探针窗之上"）。
                    CoverDemo::reseed_z(&mut ui);
                } else if sim_frame == 50 {
                    // 段 B 前置：把移动窗压到探针窗**之下**（此时它盖住探针也不算"该挡住"）。
                    CoverDemo::lower_mover_z(&mut ui);
                } else if sim_frame == 59 {
                    // 段 B：本帧把它抬到最前——本帧它画在探针之上，而遮挡表里还是旧 z。
                    CoverDemo::lift_mover(&mut ui);
                }
                self.cover.ui(&mut ui, cover_over);
            }
            // `--ui-dump`：段 1 也打印一份——两段应打印**同一个 `frame=`**（帧号每帧只 +1），
            // 且段 2 那份还应包含段 1 录的窗口（帧级暂存跨段共享）。
            if ui_dump {
                eprintln!("[段 1] {}", ui.debug_dump());
            }
            // 段收尾：提交本段到 UI 层自己的 `Render2D`；`f` 的借用到此结束。
            ui.finish();
        }
        let ui_seg1_us = t_ui.elapsed().as_secs_f64() * 1e6;

        // ── 段之间：世界层绘制（在 UI 之下；证明 UI 段与世界绘制可任意交错）─────
        // 阶段计时（`begin` = 世界层录制，`encode` = UI 两段之和，`submit` = `f.submit`，
        // `present` = `f.present`）。
        let t_world = Instant::now();
        render_world(f.draw());
        let begin_us = t_world.elapsed().as_secs_f64() * 1e6;

        // ── UI 层（段 2）：右侧诊断 / 主题调音台 / 字体 Modal ─────────────────────
        let t_ui2 = Instant::now();
        {
            let mut ui = f.ui(theme);
            // 窗口诊断（调试机制）：值由**上一帧** UI 收尾写入、本帧收尾覆盖
            // （`last_press_window` / `occluded_hits` 跨帧保留）——须在「本段 UI 模块录制
            // 之前」从 `ui.state()` 读取。段内 `Ui` 视图与 `UiState` 一一对应，
            // 故任一段都能读到同一份跨帧状态。
            // 故由旧版「`Ui::begin` 之前读」改为「段开头读」；显示内容与旧版一致。
            let prev_press = ui
                .state()
                .last_press_window()
                .map(|(id, z)| format!("{id} (z{z})"))
                .unwrap_or_else(|| "无".to_owned());
            let prev_blocked = ui.state().occluded_hits();
            let prev_widget_blocked = ui.state().widget_occluded_hits();

            self.right
                .ui(&mut ui, &mut clicks, &prev_press, prev_blocked, prev_widget_blocked);
            self.theme_tuner.ui(&mut ui);

            // 字体 Modal（**帧末录制**：modal 的 z 每帧重写为当前最大，最后录制才能保证
            // 不被本帧后录的窗口盖住——见 modal_at 文档）。
            self.top.show_font_modal(&mut ui);

            // `--sim-overlap` / `--sim-click`：读**本帧**两类遮挡拦截计数——必须在
            // 全部模块录制之后（早读只会看到前半帧）。
            if (sim_overlap || sim_click.is_some()) && sim_frame >= 26 {
                widget_blocked = ui.state().widget_occluded_hits();
                window_blocked = ui.state().occluded_hits();
            }

            // ── `--sim-tuner`：**主题调节窗口**里"滑杆 + 数字条"两件套的坐标解算 ──
            // 全部**运行时**算（不写死像素）：窗口原点取自 `ui.debug_dump()`（引擎本帧
            // 真正提交的原点，已含 Screen 限位），行内 x 由主题尺寸 + 实测标签宽推出
            // （`pad / gap / row_h / slider.min_w / input.min_w / 字号`）⇒ 换 DPI、
            // 换字体、换密度档都不会点空。
            if self.sim_tuner {
                let dump = ui.debug_dump();
                if let Some(tw) = dump.windows.iter().find(|w| w.id == "theme_tuner") {
                    let (pad, row, gap, font, slider_w, num_w) = {
                        let t = ui.theme();
                        (
                            t.panel.padding + t.panel.border_w,
                            t.row_h,
                            t.gap,
                            t.label.font_size,
                            t.slider.min_w.max(40.0),
                            t.input.min_w,
                        )
                    };
                    // 内容行自上而下：① 标题 label ② 预设 row ③ 圆角 row（本脚本的目标）。
                    // ⚠ 数字条的**可拖动区 = 最右 `GRIP_W` 宽那一条**（点在文本框上是
                    //   进入编辑、不调值）⇒ 目标点取手柄中心，不是控件中心。
                    let label_w = ui.text_size("圆角", font, None).x;
                    let row_y = tw.origin.y + pad + font + gap + row + gap + row * 0.5;
                    let slider_x = tw.origin.x + pad + label_w + gap;
                    let slider_c = Vec2::new(slider_x + slider_w * 0.5, row_y);
                    let num_x = slider_x + slider_w + gap;
                    let num_grip = Vec2::new(num_x + num_w - GRIP_W * 0.5, row_y);
                    self.sim_tuner_pts = Some((num_grip, slider_c));
                    if sim_frame == 10 {
                        eprintln!(
                            "sim-tuner: tuner origin={:?} size={:?} num_grip={num_grip:?} slider_c={slider_c:?} input.min_w={num_w} slider.min_w={slider_w} label_w={label_w}",
                            tw.origin, tw.size
                        );
                    }
                }
            }

            // 性能统计（读到的正是**上一帧**收尾写入的 UI 各阶段耗时）。
            ui_stats = ui.state().stats.clone();

            // ── 引擎状态诊断（`--ui-dump`）：Rust 侧调试用 —— 打印每个窗口的
            //    id / z / **本帧提交原点** / 尺寸 / 拖拽状态 / 持久位置 + 鼠标 / 焦点。
            //    排查"位置 / 层级 / 拖拽"问题时**先看这份状态**（见 docs/DEBUGGING.md）。
            //    ⚠ 帧级暂存跨段共享 ⇒ 段 2 的 dump 能看到**段 1 录的窗口**（且两段打印的
            //    `frame=` 相同：帧号每帧只 +1）。
            if ui_dump {
                eprintln!("[段 2] {}", ui.debug_dump());
            }

            // 重置请求（按钮点击 / `R` 键）：须在**全部录制之后**执行——
            // `UiState::reset` 清空控件状态与窗口缓存，此时本帧命令已录好、下次
            // 开场前无读取者。
            if self.menu.reset_requested {
                self.menu.reset_requested = false;
                reset_ui_state(ui.state_mut());
            }
            ui.finish();
        }
        let encode_us = ui_seg1_us + t_ui2.elapsed().as_secs_f64() * 1e6;
        self.clicks = clicks;
        // --sim-tuner：两阶段判定（打印的是**主题里**的圆角 ⇒ 滑杆 / 数字条 → 主题
        // 这条线才是最终目的）。
        if self.sim_tuner && f.frames() == 24 {
            let r = self.theme_tuner.radius;
            self.tuner_probe = Some(r);
            let ok = (r - 18.0).abs() <= 1.0 && (panel_radius.tl - r).abs() < 1.0;
            eprintln!(
                "sim-tuner: 拖数字条后 radius={r:.1}（期望 ~18 = 8 + 20px × step 0.5）· 主题圆角 tl={:.1} {}",
                panel_radius.tl,
                if ok {
                    "[OK] 数字条能改值，且进了主题"
                } else {
                    "[FAIL] 数字条没改值（点空 / 手柄不响应）或没进主题"
                }
            );
        }
        if self.sim_tuner && f.frames() == 44 {
            let r0 = self.tuner_probe.unwrap_or(-1.0);
            let r1 = self.theme_tuner.radius;
            // 滑杆拖到轨道最左 ⇒ 值到下限 0；同时要求第一阶段的数字条确实改过值。
            let ok = r0 > 0.0 && r1 <= 0.5 && panel_radius.tl <= 0.5;
            eprintln!(
                "sim-tuner: 数字条 {r0:.1} → 拖滑杆到最左 {r1:.1} · 主题圆角 tl={:.1} {}",
                panel_radius.tl,
                if ok {
                    "[OK] 滑杆与数字条绑同一个值"
                } else {
                    "[FAIL] 滑杆没改值 / 没进主题"
                }
            );
        }
        // --sim-import：打印导入结果 + 应用侧真的拿到了什么（字体族 / 背景纹理尺寸）——
        // 覆盖"字节 → 纹理 / 字体"这条线（**不含**真人点系统选择器那一步：阻塞对话框在
        // 无头环境里没法跑，而且那一步没有引擎逻辑）。
        if self.sim_import.is_some() && f.frames() == 30 {
            let bg = self
                .windows
                .bg_image
                .map(|b| format!("{:.0}×{:.0}", b.texel.x, b.texel.y))
                .unwrap_or_else(|| "无".to_owned());
            let ok = !self.top.import_status.contains("失败")
                && !self.top.import_status.contains("不认得")
                && !self.top.import_status.is_empty();
            eprintln!(
                "sim-import: status={:?} · 字体族={:?} · 背景纹理={bg} {}",
                self.top.import_status,
                self.top.font_name,
                if ok { "[OK] 导入通路走通" } else { "[FAIL] 导入没成功" }
            );
        }
        // --sim-picker：打印脚本化拖动后演示取色器的颜色（守护"面板确实改了值"：
        // 只有点击命中色块 → 面板打开 → SV 平面/色相条/滑块被拖到，颜色才会变）。
        if self.sim_picker && f.frames() == 90 {
            eprintln!("sim-picker: demo_color = {:?}", self.top.demo_color);
        }
        // --sim-click：打印"这一像素的归属"证据（点中了几个控件 + 谁被遮挡拦下）。
        // 背包格子（`inventory`）相邻格的**共享边**是最典型的用例：修复前点在边上会
        // **两个格子一起切换**，修复后只有画在后面的那个生效。
        if sim_click.is_some() && f.frames() == 45 {
            let owned = self.inventory.inventory.iter().filter(|x| **x).count();
            eprintln!(
                "sim-click: 背包已选中 {owned} 个 / 控件遮挡拦截 {widget_blocked} / \
                 窗口遮挡拦截 {window_blocked} {}",
                if owned <= 1 {
                    "[OK] 一次点击最多切换一个控件"
                } else {
                    "[FAIL] 一次点击切换了多个控件（重叠处被一起触发）"
                }
            );
        }
        // --sim-overlap：重叠处点击的**判定**（自证控件级遮挡生效）。
        // 期望：下层 = 0（灰：不会被触发）、上层 = 1（蓝：会被触发）、拦截计数 > 0。
        if self.sim_overlap && f.frames() == 58 {
            let (b, a) = (self.overlap.below_clicks, self.overlap.above_clicks);
            let ok = b == 0 && a == 1 && widget_blocked > 0;
            eprintln!(
                "sim-overlap: below={b} above={a} widget_occluded_hits={widget_blocked} -> {}",
                if ok {
                    "[OK] 重叠处只有最上层控件被触发"
                } else {
                    "[FAIL] 重叠处触发了多个控件 / 遮挡未生效"
                }
            );
        }

        // --sim-chrome：**外框四态**的判定（收起 / 关闭 / 重开 / 展开各走通 + 按钮按下
        // 没有变成窗口拖拽）。`Windows` 每帧把当前 `(open, collapsed)` 压进
        // `chrome_states`（去重），这里只看**最终态**与"过程中是否出现过关闭态"：
        // - 出现过 `open = false` ⇒ × 真的把窗口关掉了（整窗短路）；
        // - 出现过 `collapsed = true` 且尺寸仍是标题栏高 ⇒ ⌃ 真的收起了内容；
        // - 结束态是 `open = true && collapsed = false` ⇒ 应用侧重开 / 展开都生效；
        // - 全程窗口位置不变 ⇒ 按钮上的按下没被当成窗口拖拽（`claim_press`）。
        if self.sim_chrome && f.frames() == 99 {
            let st = &self.windows.chrome_states;
            let closed = st.iter().any(|(o, _)| !*o);
            let folded = st.iter().any(|(o, c)| *o && *c);
            let back = self.windows.win_a_open && !self.windows.win_a_collapsed;
            let moved = (self.windows.win_a_pos - Vec2::new(40.0, 470.0)).length() > 0.5;
            let ok = closed && folded && back && !moved;
            eprintln!(
                "sim-chrome[四态]: 关闭={closed} / 收起={folded} / 重开+展开={back} / 窗口没被拖动={} {}",
                !moved,
                if ok { "[OK] 标题栏按钮三态都走通" } else { "[FAIL] 外框按钮路径不完整" }
            );
        }

        // --sim-cover：**被上层窗口盖住的控件不该收到按下**。
        // 判定口径（两段各管一处修复，见 `cover` 模块文档）：
        // - `covered_drags`：被盖住却还带着拖拽状态进来（错误认领的按下会一直拖到释放）
        //   —— 帧末复核（`Ui::resolve_widget_press`）必须把它压成 0；
        // - 段 B 的 `starts` **不许再涨**：遮挡表按窗口 ID 跨帧存活后，"应用改 z"那一帧
        //   就不该再让被盖住的控件认领按下。
        // 段 A（同帧移动）的 `starts` 允许为 1：命中发生在几何变化那一帧，那一刻**本帧
        // 几何还没录完**，任何帧内判定都拿不到新位置——这条只能靠帧末复核兜住后果。
        if sim_cover && (f.frames() == 24 || f.frames() == 42 || f.frames() == 72) {
            let (s, d, c) = (self.cover.starts, self.cover.drag_frames, self.cover.covered_drags);
            if f.frames() == 24 {
                // 段 0（正对照）：没被盖住 ⇒ 拖拽必须**持续活着**（脚本按住约 14 帧）。
                // 若按下被误撤，拖拽状态在第 1 帧就被清掉 ⇒ 这里只有 1 帧 ⇒ FAIL。
                let ok = d >= 5;
                eprintln!(
                    "sim-cover[0 正对照]: 拖拽活着={d} 帧（探针未被盖住，按住约 14 帧） {}",
                    if ok { "[OK] 窗口内控件拖得动" } else { "[FAIL] 窗口内控件拖不动（按下被误撤）" }
                );
            } else if f.frames() == 42 {
                self.cover_starts_after_a = s;
                let ok = c == 0 && self.cover.covers;
                eprintln!(
                    "sim-cover[A 同帧移动]: 认领按下={s} / 被盖住却还在拖={c} 帧 / 移动窗确实盖住探针={} {}",
                    self.cover.covers,
                    if ok { "[OK] 被盖住的控件没留下按下状态" } else { "[FAIL] 被盖住的控件带着按下状态继续拖" }
                );
            } else {
                let no_new = s == self.cover_starts_after_a;
                let ok = c == 0 && no_new && self.cover.covers;
                eprintln!(
                    "sim-cover[B 应用改 z]: 段 B 新增认领={} / 累计被盖住却还在拖={c} 帧 / 移动窗确实盖住探针={} {}",
                    s - self.cover_starts_after_a,
                    self.cover.covers,
                    if ok { "[OK] 抬高 z 后背后的控件不再被触发" } else { "[FAIL] 抬高 z 后背后的控件仍被触发" }
                );
            }
        }

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

/// 解析 `--key 值` 形式的**字符串**参数（缺值 = `None`；`--image` / `--font-file` 用）。
fn parse_str_arg(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .filter(|v| !v.starts_with("--"))
        .cloned()
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
    app.sim_picker = args.iter().any(|a| a == "--sim-picker");
    app.sim_overlap = args.iter().any(|a| a == "--sim-overlap");
    app.sim_cover = args.iter().any(|a| a == "--sim-cover");
    app.sim_chrome = args.iter().any(|a| a == "--sim-chrome");
    app.sim_weight = args.iter().any(|a| a == "--sim-weight");
    app.sim_shadow = args.iter().any(|a| a == "--sim-shadow");
    app.sim_tuner = args.iter().any(|a| a == "--sim-tuner");
    app.sim_import = parse_str_arg(&args, "--sim-import");
    app.windows.sim_chrome = app.sim_chrome;
    app.sim_click = args
        .iter()
        .any(|a| a == "--sim-click")
        .then(|| parse_pos_arg(&args, "--sim-click", Vec2::ZERO));
    app.image_file = parse_str_arg(&args, "--image");
    app.font_file = parse_str_arg(&args, "--font-file");
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