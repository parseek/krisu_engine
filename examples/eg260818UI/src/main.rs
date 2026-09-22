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
    ColorPicker, CornerRadius, DEFAULT_LINE_SPACING, Density, Dropdown, FONT_WEIGHT_CHOICES,
    FontModal, GRIP_W, GripShape, GripStyle, IdAbsolute, Item, Label, MenuClick, Palette, PopupSide,
    Position, ScrollMode, Segmented, ShadowStyle, Size, Weight, item_h, popup_gap, popup_origin,
    popup_padding, weight_label,
};

/// 「重叠控件」演示模块（控件级遮挡：重叠处只有最上层被触发 + `--sim-overlap` 自证）。
mod overlap;
use overlap::OverlapDemo;


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
    app.windows.sim_row_overflow = args.iter().any(|a| a == "--sim-row-overflow");
    app.sim_weight = args.iter().any(|a| a == "--sim-weight");
    app.sim_shadow = args.iter().any(|a| a == "--sim-shadow");
    app.sim_clip = args.iter().any(|a| a == "--sim-clip");
    app.sim_zorder = args.iter().any(|a| a == "--sim-zorder");
    app.sim_text_cull = args.iter().any(|a| a == "--sim-text-cull");
    app.sim_tuner = args.iter().any(|a| a == "--sim-tuner");
    app.sim_import = parse_str_arg(&args, "--sim-import");
    app.theme_file = parse_str_arg(&args, "--theme");
    // `--theme`（启动载入主题 = `override_theme`）与 **"调参旋钮 → 主题"** 类 sim **不兼容**：
    // override 生效期间旋钮**故意不生效**（窗口里有「恢复调节」，见 `ThemeTuner::theme`），
    // 而 `--sim-tuner` / `--sim-shadow` 断言的就是"拧旋钮 ⇒ 引擎主题变" ⇒ 必然 `[FAIL]`。
    // 与其让人误以为回归了，不如**不开这两个 sim 并说明**（其余 sim 照常跑；`--sim-picker`
    // 这类不依赖旋钮的正是主题扫掠要用到的）。见 docs/DEBUGGING.md 的主题扫掠一节。
    let theme_override = app.theme_file.is_some();
    if theme_override && (app.sim_tuner || app.sim_shadow) {
        eprintln!(
            "theme: --theme 生效 ⇒ 跳过依赖调参旋钮的 sim（--sim-tuner / --sim-shadow）：\
             override 期间旋钮故意不生效"
        );
        app.sim_tuner = false;
        app.sim_shadow = false;
    }
    app.sim_theme = parse_str_arg(&args, "--sim-theme");
    app.sim_menu = args.iter().any(|a| a == "--sim-menu");
    app.sim_dropdown = args.iter().any(|a| a == "--sim-dropdown");
    app.sim_weight_modal = args.iter().any(|a| a == "--sim-weight-modal");
    app.sim_resize = args.iter().any(|a| a == "--sim-resize");
    // `--sim-pick-save`：**走真实的导出通路**（`export_request` → `Policy::save` →
    // `save_theme`）。选择器怎么跳过由 `filedialog::Policy` 决定（**显式 CLI**）：
    // `--no-file-dialog` ⇒ **完全不碰 rfd**；`--pick theme-save=<路径|none>` ⇒ 预置结果。
    // 本开关只负责"按下导出请求"，不再自己解析路径（单一机制，避免两套旁路）。
    app.sim_pick_save = args.iter().any(|a| a == "--sim-pick-save");
    // **选择器策略**：只由**显式命令行**构造（`--no-file-dialog` / `--pick <目标>=<路径|none>`）。
    // 用法错误 ⇒ 打清单 + 非 0 退出（静默忽略会让"以为设了路径"的测试拿不到结果还查不出原因）。
    app.dialogs = match filedialog::Policy::from_args(&args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[args] {e}");
            std::process::exit(1);
        }
    };
    app.windows.show_grip_win = app.sim_resize;
    // `--sim-scroll-mode`：按轴溢出策略（`.vscroll` / `.hscroll`）的验收现场。
    app.windows.show_scroll_wins = args.iter().any(|a| a == "--sim-scroll-mode");
    app.sim_scroll_mode = app.windows.show_scroll_wins;
    app.sim_ta_resize = args.iter().any(|a| a == "--sim-ta-resize");
    app.windows.sim_chrome = app.sim_chrome;
    app.sim_click = args
        .iter()
        .any(|a| a == "--sim-click")
        .then(|| parse_pos_arg(&args, "--sim-click", Vec2::ZERO));
    app.image_file = parse_str_arg(&args, "--image");
    app.font_file = parse_str_arg(&args, "--font-file");
    run(app)
}


/// **菜单栏位置**（逻辑像素，左上角）：录制与 `--sim-menu` 的坐标解算共用同一常量
/// —— 挪栏不用改脚本（写死像素的脚本一挪就点空）。
const MENUBAR_POS: Vec2 = Vec2::new(90.0, 12.0);

/// **引擎托管收起**的那个窗口（`.collapsible(true, None)`）：`--sim-chrome` 阶段 4 点它的 ⌃
/// 两次（收起 → 展开）来验证"状态在 `UiState` 里、应用侧没有 bool"。
const ENG_COLLAPSE_WIN: &str = "img_box_fill";

/// **某个窗口的 caption 按钮中心**（**物理像素**：`(收起 ⌃, 关闭 ✕)`）。
///
/// 口径与引擎的 `title_bar_layout` **同源**：按钮右缘贴**外框右缘**（`inset = 0`）
/// ⇒ `✕ = right − btn×0.5`；`has_close` 时 `⌃` 在它左边一个按钮 + 一个 gap
/// （`right − btn×1.5 − gap`），**没有 ✕ 时 ⌃ 自己占最右那格**（`right − btn×0.5`）。
/// 其中 `right = origin.x + size.x`、`btn = (row_h − 2).max(12)`（与 `TitleIconButton` 一致）。
///
/// ⚠ 点按钮的**上半部**（`row_h × 0.25`）而不是行中心：窗口收起时只有一行高，右下角的
/// **拖拽缩放柄**会盖到按钮行上，而柄在 `window_impl` 里**先于**按钮判定 ⇒ 点行中心会被
/// 柄抢走。`RJ_HIT_TRACE=1` 实测（150% DPI，收起态）：
/// `hit[frame 41] win_a::resize OK rect=(383,723,35,35) mouse=(399.5,724.5)`，而 ✕ 的矩形是
/// `(381,705,37,39)`（y 到 744）—— 柄（y 从 723 起）压住按钮下半，抢走按下并把窗口宽度改了
/// 1px（`RJ_CHROME_TRACE`：`bar_w` 358 → 357）。那是既有缺陷（柄 / 按钮重叠，另案处理）。
fn caption_pts(
    dump: &rjw_krusie::ui::UiDebugDump,
    id: &str,
    row_h: f32,
    gap: f32,
    has_close: bool,
) -> Option<(Vec2, Vec2)> {
    let w = dump.windows.iter().find(|p| p.id == id)?;
    let btn = (row_h - 2.0).max(12.0);
    let right = w.origin.x + w.size.x;
    let cy = w.origin.y + row_h * 0.25;
    let close_x = right - btn * 0.5;
    let fold_x = if has_close { right - btn * 1.5 - gap } else { close_x };
    Some((Vec2::new(fold_x, cy), Vec2::new(close_x, cy)))
}

/// **取色器演示**的锚点（物理像素，左上角）：录制（`TopBar::ui`）与 `--sim-picker`
/// 的坐标解算共用同一常量（脚本不写死像素）。
///
/// ⚠ 这里是 `win=0` 内容，位置必须**既不被窗口盖住、也不压在别的控件上**：
/// - 被窗口盖住 ⇒ 按下被 `window_occluded` 吞掉，而 `RJ_HIT_TRACE` **不打印**这种拦截
///   （只进 `hits().occluded_hits`）⇒ **静默失败**：脚本的"点色块"会变成"拖那个窗口"，
///   面板永远开不了（历史 bug：本 sim 早期只打印不断言，所以没人发现）。且**挪窗救不了**：
///   演示位置 `(240,250)` 被 `chishi` 盖住，而那个窗口每帧被应用重新 `.pos()` 钉住。
/// - 压在别的控件上 ⇒ 控件级遮挡 / 被对方的点击处理抢走（实测 `(24,250)` 落在「重置」上，
///   按下被 reset 抢走并**清空 UI 状态**；`(24,220)` 在紧凑主题下正好撞上重置按钮的新行位）。
///
/// 现在取 `chishi`（263,255 起）左边、重置按钮那一列**上方**的空白带：x 24..214 没有
/// 窗口，y 150..172 在控件列起点之上（换主题行高变了也不会掉进来）。
const PICKER_DEMO_POS: Vec2 = Vec2::new(24.0, 150.0);

/// `--sim-picker` 在第 88 帧把**入口色块**改成的尺寸（物理像素，故意远宽于默认）。
/// 面板此时仍开着 ⇒ 用来钉住"**入口尺寸不影响对话框尺寸**"。
const PICKER_BIG_TRIGGER: Vec2 = Vec2::new(200.0, 40.0);

/// `--sim-row-overflow` 的窄窗口左上角（物理像素）与该窗口**外面**那个控件的偏移
/// （内容坐标）。窗口宽固定为逻辑 150 ⇒ 内容宽 = 150 × scale，输入框默认宽远超它。
const ROW_WIN_POS: Vec2 = Vec2::new(200.0, 620.0);

/// `--sim-resize` 阶段 2 的窗口（**故意不给 `.width()`**）：位置取空白带 —— 它的**右下角**
/// （缩放柄所在处）不能被任何更高 z 的窗口压住（`icons` y ≤ 575 / `strict_win` y ≥ 690 /
/// `chishi` x ≤ 499 都要避开），否则 `window_occluded` 会让柄判不中（实测踩过两次）。
const GRIP_WIN_POS: Vec2 = Vec2::new(500.0, 590.0);

/// `--sim-scroll-mode` 的两扇窗（都避开 `chishi`(x ≤ 499) / `inv_panel`(y ≤ 307) /
/// `icons`(x ≥ 840) / `strict_win`(y ≥ 690)）。
const SCROLL_NOCLIP_POS: Vec2 = Vec2::new(520.0, 330.0);
const SCROLL_CLIP_POS: Vec2 = Vec2::new(520.0, 470.0);
/// `--sim-scroll-mode` 第三扇窗（`vscroll(Scroll)`）：放在**屏幕下缘附近**，
/// 于是"屏幕剩下的高"明显小于内容高 ⇒ 视口 + 滚动条。
const SCROLL_V_POS: Vec2 = Vec2::new(830.0, 700.0);
/// 窗口**外面**那个控件（"幽灵控件"）在窗口内容坐标里的位置：x 明显超过内容宽。
const ROW_GHOST_OFFSET: Vec2 = Vec2::new(260.0, 12.0);
/// 窗口**里面**那个控件（正对照）在窗口内容坐标里的位置与宽（物理像素）。
const ROW_INSIDE_OFFSET: Vec2 = Vec2::new(8.0, 60.0);
const ROW_INSIDE_W: f32 = 120.0;
const ROW_TE_H: f32 = 39.0;

/// 取色面板里的**固定**几何常量（物理像素；与 `colorpicker/panel.rs` 同源）。
const PICKER_PAD: f32 = 6.0;
const PICKER_GAP: f32 = 6.0;
const PICKER_LABEL_W: f32 = 14.0;
const PICKER_SLIDER_MIN: f32 = 90.0;

/// 取色面板的**默认外宽**（物理像素）：`主题默认入口宽 × 1.9` 与"一行通道最小宽"取大。
///
/// ⚠ 取的是**主题**口径（`Theme::input.min_w`），**不是入口色块的实测宽** —— 那正是
/// `ColorPicker::size(..)` 不该影响对话框尺寸的地方（`--sim-picker` 第 88/92 帧钉住）。
fn picker_panel_w(field_w: f32) -> f32 {
    (field_w * 1.9).max(
        PICKER_PAD * 2.0 + PICKER_LABEL_W + PICKER_GAP + PICKER_SLIDER_MIN + PICKER_GAP + field_w,
    )
}

/// **「玩家名」可拖动面板的初始位置**（逻辑像素，左上角）：录制与 `--sim-zorder`
/// 的坐标解算共用同一常量（同理：脚本不写死像素）。
const NAME_PANEL_POS: Vec2 = Vec2::new(430.0, 12.0);

/// `--sim-zorder` 把 `name_panel` 拖到的**目标左上角**（物理像素）：压在顶部
/// "FPS / 点击次数"两个标签上——那正是"面板底色被更早录制的 win=0 内容穿透"的现场。
/// 值按**物理像素**给（注入与面板命中都在物理空间），脚本不做二次 DPI 换算。
const ZORDER_PANEL_DST: Vec2 = Vec2::new(30.0, 18.0);

/// **滚动列表的几何**（逻辑像素，左上角 + 尺寸）：录制（`list_at`）与 `--sim-zorder`
/// 的坐标解算共用同一常量——列表挪位置 / 改尺寸时脚本自动跟随。
const LIST_POS: Vec2 = Vec2::new(880.0, 130.0);
const LIST_SIZE: Vec2 = Vec2::new(240.0, 300.0);

/// `--sim-text-cull` 用的长文本（重复若干次 ⇒ 全文宽远大于输入框内宽）。
const TEXT_CULL_LONG: &str = "滚过头就不见了？这段刻意写得很长，用来复现单行输入框的水平滚动。";
/// `--sim-text-cull` 把 `text_scroll` 推到的位置（物理像素）：**远大于框内宽**
/// （`res_input` 内容宽 ≈ 336、`win_b_input` ≈ 510）而**远小于全文宽**（≈ 30 字 × 8 遍），
/// 即用户按 End / 把视图拖到中段时合法会到达的状态。
const TEXT_CULL_SCROLL: f32 = 1000.0;

/// **统一后的「按钮下拉菜单」演示位置**（逻辑像素，左上角；与菜单栏同一行、在其右侧）：
/// 录制与 `--sim-dropdown` 的坐标解算**共用同一常量**（同理：脚本不写死像素）。
const DROPDOWN_OPT_POS: Vec2 = Vec2::new(660.0, 12.0);
/// 演示 ②（富内容模式：菜单里放文本输入 / 分割线 / 菜单项 / 子下拉）的位置。
const DROPDOWN_FILE_POS: Vec2 = Vec2::new(860.0, 12.0);
/// 两个演示下拉的**固定触发器宽**（逻辑像素）：仿真按它算点击点，不必复刻
/// "按文字自动宽"的内部公式（`Dropdown::width`）。
const DROPDOWN_W: f32 = 160.0;
/// 演示 ①（选项列表模式）的选项。
const DIFF_TOP: [&str; 3] = ["简单", "普通", "困难"];
/// 演示 ②（富内容模式里嵌的子下拉）的选项。
const ENCODINGS: [&str; 3] = ["ASCII", "UTF-8", "GBK"];

/// **文件导入**（系统文件选择器 → 字节 → 引擎资源）：图片当背景纹理、字体进运行时字体库、
/// 主题走 TOML 序列化（导入 = 在当前主题上合并覆盖，导出 = 全量落盘）。
mod filedialog;
use filedialog::{ExportKind, ImportKind};

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
    ///
    /// 这是**已应用**值（主题每帧按它重建）。字体重叠对话框用的是**草稿**
    /// `font_weight_draft`：对话框里选档位立刻写草稿（下拉是"点一次就生效"的控件），
    /// 点「确定」才 `apply` 到这里，点「取消」丢弃草稿。
    font_weight: Weight,
    /// 字体对话框里的**字重草稿**（每次打开时从 `font_weight` 拷入）。
    font_weight_draft: Weight,
    /// 字体 Modal 输入框内容（跨帧持久）。
    font_input: String,
    /// 字体 Modal 开关。
    font_modal_open: bool,
    /// **固定位置**的取色器颜色（物理定位：`--sim-picker` 的脚本化点击要能算到坐标）。
    demo_color: Color,
    /// 取色器**入口色块**的显式尺寸（物理像素；`None` = 主题默认）。
    /// `--sim-picker` 第 88 帧把它设成 [`PICKER_BIG_TRIGGER`] ⇒ 面板仍开着，用来验证
    /// "入口尺寸改了，**对话框宽度不变**"。
    demo_picker_size: Option<Vec2>,
    /// **待处理的导入请求**（点「导入图片…」/「导入字体…」只记请求：系统选择器是
    /// **阻塞**调用，录制期不能弹——见 `filedialog` 模块文档）。
    import_request: Option<ImportKind>,
    /// **待处理的导出请求**（「导出主题…」→ TOML；同样帧外弹"另存为"）。
    export_request: Option<ExportKind>,
    /// 导入结果 / 失败原因（顶栏状态标签显示）。
    import_status: String,
    /// 本帧是否已经录过字体 Modal（**一帧只允许录一次**：重复调用会把面板与文本画两遍，
    /// 观感就是"文本输入重复"——历史 bug，见 `show_font_modal`）。
    modal_recorded: bool,
}

impl TopBar {
    fn new() -> Self {
        Self {
            player_name: "Krisu".to_owned(),
            font_name: String::new(),
            font_weight: Weight::NORMAL,
            font_weight_draft: Weight::NORMAL,
            font_input: String::new(),
            font_modal_open: false,
            demo_color: Color::rgba_u8(255, 128, 40, 255),
            demo_picker_size: None,
            import_request: None,
            export_request: None,
            import_status: String::new(),
            modal_recorded: false,
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
                    // 打开对话框 = **重置字重草稿**（上一轮取消掉的草稿不该漏进来）。
                    self.font_weight_draft = self.font_weight;
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
                // **主题序列化（TOML）**：导出 = 当前主题全量落盘；导入 = 在**当前主题上
                // 合并覆盖**（手写小文件只写要改的几行也能用）。同样只记待办（`rfd` 阻塞）。
                if r.button("theme_export_btn", "导出主题…").clicked() {
                    self.export_request = Some(ExportKind::Theme);
                }
                if r.button("theme_import_btn", "导入主题…").clicked() {
                    self.import_request = Some(ImportKind::Theme);
                }
                // 导入结果 / 失败原因（空 = 不占位）。
                if !self.import_status.is_empty() {
                    r.add(Label::new(&self.import_status).ellipsis());
                }
            });
        });
        // 玩家名可拖动面板（右移：上面那行按钮随字体名变长，别压到它）。
        ui.drag_panel_at("name_panel", NAME_PANEL_POS, |p| {
            p.label("玩家名（可拖动）");
            p.text_input("name", &mut self.player_name);
        });
        // **取色器演示**（固定物理位置：`--sim-picker` 的脚本化点击按这个坐标算）。
        // 不传 `&mut String` —— 面板的文本框用全局跨帧缓冲（`ColorPickerState::text`）。
        // 入口色块可 `.size(..)`（`--sim-picker` 用它验证"入口尺寸 ≠ 对话框尺寸"）。
        //
        // ⚠ **不要把它塞进某扇窗口里**（试过）：面板会变成**嵌套浮层**
        // （`picker_win/picker_demo::popup`），`debug_dump` 里那个 origin 是**父窗口局部**
        // 坐标 ⇒ 脚本按它算出的面板坐标全落在父窗口坐标系里（点空）。`win=0` 摆放时
        // 面板是**顶层**浮层，origin 是屏幕绝对坐标。
        let picker = ColorPicker::new("picker_demo", &mut self.demo_color).alpha(true);
        let picker = match self.demo_picker_size {
            Some(s) => picker.size(Size::Physical(s)),
            None => picker,
        };
        ui.add_at(Position::Physical(PICKER_DEMO_POS), picker);
    }

    /// 字体 Modal（**帧末调用**：modal 的 z 每帧重写为当前最大，最后录制才能保证
    /// 不被本帧后录的窗口盖住——见 `modal_at` 文档）。
    fn show_font_modal(&mut self, ui: &mut Ui) {
        if self.font_modal_open {
            // **一帧只允许录一次**：调用点写重复（每帧两次 `show_font_modal`）会把整个
            // 面板 —— 输入框、预览框、所有文本 —— 画两遍，观感就是"文本输入重复"。
            // 这个断言把"多调用一次"这种无声 bug 变成冒烟测试里的 panic。
            debug_assert!(
                !self.modal_recorded,
                "FontModal 一帧只能录一次（检查调用点是否重复）"
            );
            self.modal_recorded = true;
            FontModal {
                input: &mut self.font_input,
                // 对话框写的是**草稿**；「确定」时经 `apply` 落到已应用值。
                weight: &mut self.font_weight_draft,
                // 字重由**弹窗直接写回**（草稿，选中即写）⇒ 这里只负责把草稿 + 字体名
                // 一起提交到应用状态（`apply` 的两个参数都要用上）。
                apply: &mut |name: &str, w: Weight| {
                    self.font_name = name.to_owned();
                    self.font_weight = w;
                },
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
    /// `Dropdown::options` 的选中索引（难度下拉；统一后的下拉菜单**选项列表模式**）。
    diff_idx: u32,
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
            diff_idx: 1,
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
            // **统一后的下拉菜单**（`Dropdown` 是普通 `Widget` ⇒ `p.add(..)` 放进任何容器）。
            // 这里是"选项列表模式"：菜单项由引擎排（选中行打勾 + 整行高亮），
            // 点击写回 `&mut u32` 并自动收起；键盘 ↑/↓ 也能切。
            // ⚠ 旧入口 `p.combo(..)` 仍然可用（= 本控件 + 固定布局宽的糖）——
            // `FontModal` 的字重下拉就还在用它（老代码不必改）。
            const DIFFS: [&str; 3] = ["简单", "普通", "困难"];
            p.add(Dropdown::options(
                "diff_dd",
                DIFFS[self.diff_idx.min(2) as usize],
                &mut self.diff_idx,
                &DIFFS,
            ));
            self.difficulty = DIFFS[self.diff_idx.min(2) as usize].to_owned();
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
    /// --sim-chrome：`(收起, 关闭)` 按钮中心（**物理像素**；每帧从 `debug_dump` 的
    /// `win_a` **外框矩形**推导 —— 不写死宽度 / 内容内边距，才能验证"按钮贴外框右缘"）。
    chrome_pts: Option<(Vec2, Vec2)>,
    /// --sim-chrome：**引擎托管收起**那个窗口（`collapsible(true, None)`）的 ⌃ 中心。
    eng_pts: Option<Vec2>,
    /// --sim-chrome：该窗口出现过的 `(引擎收起状态, dump 尺寸)`（去重）——状态与几何必须同步。
    eng_states: Vec<(bool, Vec2)>,
    /// --sim-chrome：是否打印上面的证据。
    sim_chrome: bool,
    /// --sim-resize 阶段 2：`grip_win`（**没有 `.width()`** 的那个窗口）的柄中心、
    /// 拖之前 / 之后的结算尺寸。
    grip_pt: Option<Vec2>,
    grip_before: Option<Vec2>,
    grip_after: Option<Vec2>,
    /// --sim-resize 阶段 2：**冻结的拖拽目标点**（帧 60 记一次）。
    ///
    /// ⚠ 必须冻结：柄会随窗口长大而移动，每帧重算目标 = 鼠标"追着柄跑" ⇒ 位移逐帧累加
    /// （阶段 1 第一版就踩过这个坑）。收缩方向更明显：柄朝鼠标方向移动 ⇒ 高度一档一档
    /// 掉到下限，看起来像"收缩高度不是恒定值"。
    grip_to: Option<Vec2>,
    /// --sim-resize 阶段 3：**冻结的收缩目标点**（帧 90 记一次；同上）。
    resize_up: Option<Vec2>,
    /// --sim-resize 阶段 3：**收缩高度**现场（用户实测："窗口收缩高度应当为恒定值"）：
    /// `(松手后一帧的高度, 再等若干帧之后的高度)` —— 两者必须相等（不许回弹 / 抖动），
    /// 且必须 ≥ 引擎的恒定下限（`row_h + 2×pad`）。
    shrink_after_free: Option<f32>,
    shrink_later: Option<f32>,
    /// --sim-resize 阶段 2：是否录 `grip_win` 那扇窗口（由 App 侧按 `--sim-resize` 置位）。
    show_grip_win: bool,
    /// --sim-scroll-mode：是否录那两扇"按轴溢出策略"窗口。
    show_scroll_wins: bool,
    /// --sim-scroll-mode 两扇窗的文本（各自独立，避免共用缓冲互相覆写）。
    scroll_text_a: String,
    scroll_text_b: String,
    /// --sim-chrome 阶段 5：⌃ 按钮的**行中心**（`caption_pts` 给的是"上半"——那是为了
    /// 躲开收起态右下角的缩放柄；本轮让柄给内容让位之后，**中心也该能点**）。
    eng_center_pt: Option<Vec2>,
    /// --sim-chrome 阶段 5：点中心之前 / 之后的引擎收起状态（必须不同 = 真的翻转过）。
    eng_center_before: Option<bool>,
    eng_center_after: Option<bool>,
    /// --sim-chrome 阶段 6：`img_box_fill` 的 `(原点, 尺寸)`（缩放柄坐标从它解算）。
    eng_rect: Option<(Vec2, Vec2)>,
    /// --sim-chrome 阶段 6：拖大之后 / 点 ⌃ 收起之后的高度（断言收起 ≈ 一行标题栏）。
    eng_dragged_h: Option<f32>,
    eng_collapsed_h: Option<f32>,
    /// --sim-chrome 阶段 6：**冻结**的拖拽目标（柄随尺寸移动 ⇒ 每帧重算会追着拖）。
    eng_drag_to: Option<Vec2>,
    /// --sim-row-overflow：本窗口是否录制（固定窄宽 + `Placement::Clip` 的验收现场）。
    sim_row_overflow: bool,
    /// --sim-row-overflow：窄窗口里那一行的**结算尺寸**（`row(..)` 的返回值）——
    /// 判定"整行落在窗口可用宽内"（修前 = 标签 + 输入框的自然宽之和，会超出）。
    row_size: Option<Vec2>,
    /// --sim-row-overflow：窗口内容的可用宽（物理像素；= `.width()` 逻辑值 × scale）。
    row_avail_w: f32,
    /// --sim-row-overflow：`Label + ColorPicker` 那一行的结算尺寸（同上，判定不溢出）。
    row_size_picker: Option<Vec2>,
    /// --sim-row-overflow：取色器的颜色（演示行用）。
    row_color: Color,
    /// --sim-row-overflow 两个控件的文本（控件本身只用来产生可观测的点击）。
    ghost_text: String,
    inside_text: String,
    /// --sim-row-overflow：窗口里 / 窗口外两个点击点（**物理像素**，从 dump 的窗口原点 +
    /// 主题内边距解算 ⇒ 换主题 / 换 DPI 都对）。
    row_inside_pt: Option<Vec2>,
    row_ghost_pt: Option<Vec2>,
    /// --sim-row-overflow：点窗口**内**那个控件后，焦点是否落在它身上（正对照）。
    row_inside_focus: bool,
    /// --sim-row-overflow：点窗口**外**那个溢出控件后，焦点是否落在它身上（必须 false）。
    row_ghost_focus: bool,
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
            win_a_open: false,
            win_a_collapsed: false,
            last_win_a_size: Vec2::ZERO,
            chrome_states: Vec::new(),
            chrome_pts: None,
            eng_pts: None,
            eng_states: Vec::new(),
            sim_chrome: false,
            grip_pt: None,
            grip_before: None,
            grip_after: None,
            grip_to: None,
            resize_up: None,
            show_grip_win: false,
            show_scroll_wins: false,
            scroll_text_a: String::new(),
            scroll_text_b: String::new(),
            eng_center_pt: None,
            eng_center_before: None,
            eng_center_after: None,
            eng_rect: None,
            eng_dragged_h: None,
            eng_collapsed_h: None,
            eng_drag_to: None,
            shrink_after_free: None,
            shrink_later: None,
            sim_row_overflow: false,
            row_size: None,
            row_size_picker: None,
            row_color: Color::rgba_u8(110, 168, 255, 255),
            row_avail_w: 0.0,
            ghost_text: String::new(),
            inside_text: String::new(),
            row_inside_pt: None,
            row_ghost_pt: None,
            row_inside_focus: false,
            row_ghost_focus: false,
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
            // **应用持有**收起状态（`Some(&mut bool)`）：可持久化 / 可与别的状态联动 ——
            // 菜单里的「窗口 A 收起」勾选项与标题栏的 ⌃ 是**同一个** `bool`。
            .collapsible(true, Some(&mut self.win_a_collapsed))
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
        // ── `--sim-resize` 阶段 2：**没有 `.width()` 也要能拖宽** ────────────────
        // 用户实测："resize Horizontal 不在设置 width 的情况下无法缩放"——根因是持久宽
        // （`window_widths`）只在 `width.map(..)` 里被读 ⇒ 拖了也被丢掉。这里故意**不写**
        // `.width()`，只开 `Resize::Horizontal`：拖右 +50 之后宽必须变大、**高不变**。
        if self.show_grip_win {
            ui.window("grip_win")
                .pos(Position::Physical(GRIP_WIN_POS))
                .resize(true, Resize::Horizontal)
                .title("没有 width，也能拖宽")
                .show(|w| {
                    w.label("只有宽度轴可拖（Resize::Horizontal）");
                });
        }
        // ── `--sim-scroll-mode`：**按轴**的溢出策略验收现场 ──────────────────────
        // 两扇窗**同宽同内容**，只有水平轴的策略不同：
        //  · `hscroll(NoClip)`：该轴不裁 ⇒ `.width(150)` 只是**下限**，内容（宽 400 的编辑器）
        //    把窗口撑到 ~426；
        //  · `hscroll(ClipOnly)`：该轴是视口 ⇒ 窗口固定 150×scale（内容溢出被裁）。
        // 两扇窗的**高**必须相同（纵向都是默认的 `NoClip` ⇒ 内容撑高）——这条一起证明
        // "两条轴各自独立"（旧的 `Placement::Clip` 会把两条轴一起处理）。
        if self.show_scroll_wins {
            // `NoClip` 那扇**故意不给 `.width(..)`**：该轴"不裁"的前提是**大小装得下内容**
            // ⇒ 让它自动宽（内容 400 逻辑像素 = 600 物理 ⇒ 窗口 ~626）。给了 `.width(..)`
            // 就是"我要这个宽"（固定），内容超出只会**溢出可见**而不是把窗口撑大。
            ui.window("scroll_noclip_win")
                .pos(Position::Physical(SCROLL_NOCLIP_POS))
                .hscroll(ScrollMode::NoClip)
                .title("hscroll(NoClip)")
                .show(|w| {
                    // 与 ClipOnly 那扇窗**同一个**普通子项（占光标）⇒ 两窗自然高一致，
                    // 才谈得上"纵向没被牵连"。
                    w.label("横向溢出：");
                    w.add(TextEditor::new("scroll_noclip_te", &mut self.scroll_text_a).width(400.0));
                });
            ui.window("scroll_clip_win")
                .pos(Position::Physical(SCROLL_CLIP_POS))
                .width(150.0)
                .hscroll(ScrollMode::ClipOnly)
                .title("hscroll(ClipOnly)")
                .show(|w| {
                    // 一行普通内容（占光标）⇒ 与 NoClip 那扇窗的"内容撑高"口径一致，
                    // 两窗高度才对得上（`add_at` 不占光标、不进自然高）。
                    w.label("横向溢出：");
                    // 绝对定位：不进可用宽 clamp ⇒ 内容真的比视口宽（这才是"要裁"的现场）。
                    w.add_at(
                        Position::Physical(Vec2::ZERO),
                        TextEditor::new("scroll_clip_te", &mut self.scroll_text_b).width(400.0),
                    );
                });
            // ③ `vscroll(Scroll)`：**真的滚动视口**（窗口内滚动条）——内容很高、窗口只占
            //    屏幕剩下的那点高 ⇒ 出现滚动条；拖 thumb 能改偏移（`ScrollState.offset`）。
            ui.window("scroll_v_win")
                .pos(Position::Physical(SCROLL_V_POS))
                .width(140.0)
                .vscroll(ScrollMode::Scroll)
                // 关掉窗口自己的缩放柄：右下角要留给**滚动条**（否则点下去是在拖窗口尺寸）。
                .resize(false, Resize::None)
                .title("vscroll(Scroll)")
                .show(|w| {
                    for i in 0..12 {
                        w.label(&format!("第 {i} 行（内容远高于视口）"));
                    }
                });
        }
        // ── `--sim-row-overflow`：**窄的固定宽窗口**里的验收现场 ──────────────────
        //
        // 用户实测的两条 BUG：
        // ① `.width()` 较小时，`row` 里的控件**整排突到窗口外面**（每个子项各自都没
        //    超限，但整行超过了可用宽）——判定用 `row(..)` 的**结算宽** ≤ 窗口内容宽；
        // ② 溢出到面板外的控件（这里是 `add_at` 放到窗口右边的编辑器）**看不见却点得着**
        //    （`Placement::Clip` 裁掉了绘制命令，命中区还在）——判定"窗口外那个点
        //    点不到、窗口内那个点点得到"（正反对照，避免把守卫写成"整窗不可点"）。
        //
        // 位置 / 尺寸全部写成常量：脚本按同一组常量算点击点（不写死"猜"出来的像素）。
        if self.sim_row_overflow {
            let scale = ui.scale();
            // `.width(150.0)` = **逻辑**宽 ⇒ 内容宽 = 150 × scale（物理）。
            self.row_avail_w = 150.0 * scale;
            ui.window("row_win")
                .pos(Position::Physical(ROW_WIN_POS))
                .width(150.0)
                .placement(Placement::Clip)
                .resize(false, Resize::None)
                .title("窄窗口（row 不许突出去）")
                .show(|w| {
                    // ① 一行：标签 + 输入框（输入框默认宽 = 主题 `input.min_w`，远超可用宽）
                    let size = w.row(|r| {
                        r.label("标签：");
                        r.add(TextEditor::new("row_te", &mut self.inside_text));
                    });
                    self.row_size = Some(size);
                    // ①b 同一扇窗里再放一行 **`Label + ColorPicker`**（用户截图里的调色板
                    //    形态）：取色器的自然宽取自主题 `input.min_w`，如果它**不看
                    //    `avail_w`**，这一行同样会突到窗口外面（与 `TextEditor` 不对称）。
                    let size2 = w.row(|r| {
                        r.label("颜色：");
                        r.add(ColorPicker::new("row_cp", &mut self.row_color));
                    });
                    self.row_size_picker = Some(size2);
                    // ② 绝对定位到窗口**外面**的控件（不受可用宽 clamp 约束）：
                    //    它就是"内容溢出时那个看不见的幽灵控件"。
                    w.add_at(
                        Position::Physical(ROW_GHOST_OFFSET),
                        TextEditor::new("ghost_te", &mut self.ghost_text).width(120.0),
                    );
                    // ③ 窗口**里面**的正对照控件（已知矩形 ⇒ 脚本能算到点击点）。
                    w.add_at(
                        Position::Physical(ROW_INSIDE_OFFSET),
                        TextEditor::new("inside_te", &mut self.inside_text).width(ROW_INSIDE_W),
                    );
                });
        }
        // 严格裁剪窗口（Placement::Clip）：内容超出窗口被强制裁剪（Clip 沙箱）。
        ui.window("strict_win")
            .pos(Vec2::new(560.0, 460.0))
            .placement(Placement::Clip)
            .resize(true, Resize::Both)
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
                .style(base.clone().with_radius(0.0).with_bg_image(tile))
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
            // **宽高同调**（`resize(true, Resize::Both)`）：右下角柄拖宽拖高，高度跨帧
            // 持久于 `UiState::window_heights`（高度一旦被拖过就由用户接管，内容不再撑高）。
            // 这也是"斜线缩放柄"的主要展示窗口（柄形状由主题调节窗口的「拖拽柄」选）。
            ui.window("img_box_fill")
                .pos(Vec2::new(16.0, 560.0))
                .width(200.0)
                .resize(true, Resize::Both)
                .title("TTT（可拖宽拖高）")
                // **引擎托管的收起**（`None`）：应用**不必**自己持有一个 `bool`，点 ⌃ 由引擎
                // 按窗口绝对 ID 翻转（存 `UiState::collapsed`）；应用要读 / 代码收起时用
                // `ui.state().is_collapsed("img_box_fill")` / `ui.state_mut().set_collapsed(..)`。
                // `--sim-chrome` 阶段 4 点它两次（收起 → 展开）实测状态与几何同步。
                .collapsible(true, None)
                .style(base.with_bg_image(bg.fit(ImageFit::Fill).tint(Color::WHITE.with_a(0.5))))
                .show(|w| {
                    w.label("另一个窗口");
                    w.button("btn00", "Awa");
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
            LIST_POS,
            LIST_SIZE,
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
    /// **导入 / `--theme` 载入的主题**（TOML 序列化格式）。
    ///
    /// `Some` 时它当**基底**：旋钮（圆角 / 密度 / 投影 / 缩放柄 / 调色）**暂不生效**，
    /// 但应用侧**运行时令牌**（字体族 / 字重）仍然照常覆盖——这样「字体…」弹窗与
    /// `--font-file` 不会因为导入主题而失效。窗口里给一行提示 + 「恢复调节」按钮清除它。
    override_theme: Option<Theme>,
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
            grip_shape: GripShape::Diagonal,
            grip_color: p.border,
            bg: p.surface,
            border: p.border,
            accent: p.accent,
            override_theme: None,
            // 默认打开：这是个"可调的窗口"，开着才能看见效果。
            open: false,
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
        // **导入 / `--theme` 的主题当基底**：旋钮暂不生效（窗口里有提示 + 恢复按钮），
        // 但应用侧运行时令牌（字体族 / 字重）仍然照常覆盖。
        if let Some(base) = &self.override_theme {
            let mut t = base.clone();
            if !font.is_empty() {
                t = t.with_font_family(font);
            }
            return t.with_font_weight(weight);
        }
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
                // **导入 / `--theme` 的主题**：作为基底生效时，下面的旋钮暂不生效
                // （避免"拖了没反应"的困惑）——给一行提示 + 「恢复调节」。
                if self.override_theme.is_some() {
                    w.row(|w| {
                        w.label("已导入主题文件（旋钮暂不生效）");
                        if w.button("th_clear_override", "恢复调节").clicked() {
                            self.override_theme = None;
                        }
                    });
                }
                w.row(|w| {
                    w.label("预设:");
                    // **分段按钮组**（`Segmented`）：互斥选项拼成一个整体——相邻段共享边、
                    // 只有整组外侧角是圆的、选中段高亮（"按钮样式：拼在一起"）。
                    let mut preset = self.preset.min(2) as usize;
                    w.add(Segmented::new("th_preset", &["dark", "light", "legacy"], &mut preset));
                    if preset != self.preset as usize {
                        self.set_preset(preset as u8);
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
                    w.add(ColorPicker::new("th_bg", &mut self.bg).alpha(true));
                    w.add(ColorPicker::new("th_border", &mut self.border).alpha(true));
                    w.add(ColorPicker::new("th_accent", &mut self.accent).alpha(true));
                });
                w.row(|w| {
                    w.label("边框宽");
                    self.border_w = w.slider("th_bdw", 0.0..=5.0, self.border_w);
                    w.add(NumberInput::new("th_bdw_n", &mut self.border_w).range(0.0, 5.0).step(0.5));
                });
                // 右下角**缩放柄**：形状四档 + 颜色。只对"允许拖拽缩放"的窗口生效；
                // "不画"只是没有图案，**拖动缩放照旧**（命中区单独存在，见 `GripStyle`）。
                w.row(|w| {
                    w.label("拖拽柄");
                    let mut gi = match self.grip_shape {
                        GripShape::Squares => 0usize,
                        GripShape::Bars => 1,
                        GripShape::Diagonal => 2,
                        GripShape::Hidden => 3,
                    };
                    w.add(Segmented::new(
                        "th_grip",
                        &["方块", "横线", "斜线", "不画"],
                        &mut gi,
                    ));
                    self.grip_shape = [
                        GripShape::Squares,
                        GripShape::Bars,
                        GripShape::Diagonal,
                        GripShape::Hidden,
                    ][gi.min(3)];
                    w.add(ColorPicker::new("th_grip_color", &mut self.grip_color));
                });
                // ── 布局密度（主题扩展：紧凑 / 标准 / 宽松）──────────────────
                // 一键铺开"间距 / 字号 / 行距"，三根滑杆随后可自由微调
                // （点档位 = 把滑杆对齐到该档的规范值，与"预设"同思路）。
                w.row(|w| {
                    w.label("密度:");
                    let mut di = self.density as usize;
                    w.add(Segmented::new("th_density", &["紧凑", "标准", "宽松"], &mut di));
                    if di != self.density as usize {
                        self.set_density([Density::Compact, Density::Cozy, Density::Spacious]
                            [di.min(2)]);
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
                // 尾部**不再放"当前值一览"标签**：每行都有滑杆 + 数字条，值就近看得见
                // （用户："可以去掉了"）。
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
    /// --sim-picker：**面板窗口的实测原点**（从 `debug_dump` 读到就存下来）。
    ///
    /// 面板的绝对位置**不能**按"锚点 + 24"硬算：面板比屏幕下半部还高时会被
    /// `WindowClamp` **翻到上方**（实测 `krusie-dark` 主题：面板高 676 ⇒ 被翻），
    /// 于是按常数算出的模式行 / SV 平面 / 滑杆坐标全部落空——点到"面板外"还会把面板
    /// 关掉，看起来却像"面板压根没开"。与 `--sim-dropdown` / `--sim-chrome` 同路子：
    /// **从引擎状态里读**，别复刻布局公式。
    sim_picker_popup: Option<Vec2>,
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
    /// --sim-clip：**环境裁剪走 batch scissor**（不是逐命令切割几何）。
    /// 第 30 / 90 帧打印 `clip_batches` + 严格窗口 / 普通窗口的 `clip`（`--ui-dump` 同源）；
    /// 期间脚本拖动该窗口 ⇒ 顺带回归"scissor 与内容一起移动"。
    sim_clip: bool,
    /// --sim-clip：第 30 帧记录的严格窗口原点（第 90 帧比较"确实被拖动了"）。
    clip_probe: Option<Vec2>,
    /// --sim-zorder：**脚本化验证 win=0 的绘制序**（不是命中序）。
    ///
    /// 两段：
    /// 1. 把 `name_panel` 拖到顶部 FPS / 点击次数标签**上面**（按下点在面板**内边距**里，
    ///    避开内部 label / 输入框 ⇒ 不会被 `press_claimed` 撤销拖拽基准）。拖动本身与
    ///    绘制序无关，只是把"重叠现场"摆出来；
    /// 2. 点 `list_demo` 的滚动条条带**翻页**，再点列表里的同一屏幕点 ⇒ 选中项号必须变大
    ///    （证明滚动条命中通路真的被脚本摸到了，现场画面确实含"滑块 + 列表项"重叠）。
    ///
    /// 绘制序本身由 `RJ_ORDER_TRACE=1` 的 `order[...]` 行核对（引擎侧证据，
    /// 应用侧读不到"谁画在谁上面"）。
    sim_zorder: bool,
    /// --sim-text-cull：**复现「单行输入框滚过头文字消失」**（用户报的 BUG）。
    ///
    /// 现场：把 `res_input` / `win_b_input` 绑的 `win_b_note` 换成长串，再把该控件的
    /// `text_scroll` 直接推到远超框内宽、但仍远小于全文宽的位置（= 用户按 End / 拖到中段时
    /// **合法会到达**的状态）⇒ 修前整条文本命令被兜底剔除（`culled_text` 涨、`verts` 掉一截），
    /// 修后两者都不变。见 `docs/DEBUGGING.md` §8.4。
    sim_text_cull: bool,
    /// --sim-text-cull：**未滚动**那一帧的 `verts`（帧 N 读到的是 N-1 的统计）。
    cull_verts_before: Option<u32>,
    /// --sim-text-cull：**滚到中段**那一帧的 `verts`。
    cull_verts_after: Option<u32>,
    /// --sim-text-cull：同两帧的 `culled_text`（被兜底剔除的文本命令条数）——
    /// 比 `verts` 更直接：修前"滚到中段"那一帧应 ≥ 1，修后两帧都为 0/不变。
    cull_text_before: Option<u32>,
    cull_text_after: Option<u32>,
    /// --sim-zorder：第 100 帧读到的 `name_panel` 位置（判定"真的拖动了"）。
    zorder_panel: Option<Vec2>,
    /// --sim-zorder：翻页**之前**点列表得到的选中项号（判定"同一屏幕点选了更后面的条目"）。
    zorder_sel_before: Option<u32>,
    /// --sim-shadow：第 20 帧的主题投影色（第 40 帧对比用）。
    shadow_probe: Option<Color>,
    /// --sim-tuner：脚本化鼠标的两个目标点（**录制时运行时解算**）：
    /// `(圆角数字条中心, 圆角滑杆中心)`。注入只能经 `Frame`（录制中 `f` 被借着）
    /// ⇒ 段末（`ui.finish()` 之后）再按帧注入——注入本来就下一帧才生效。
    sim_tuner_pts: Option<(Vec2, Vec2)>,
    /// --sim-tuner：拖数字条后的圆角值（第二阶段判定"数字条真的改了值"）。
    tuner_probe: Option<f32>,
    /// --sim-tuner：分段按钮组（预设行第 3 段）中心——第三阶段判定"分段能点"。
    sim_seg_pt: Option<Vec2>,
    /// --sim-tuner：**实操主题调节窗口里的"滑杆 + 数字条"**（坐标运行时解算，不写死像素）。
    sim_tuner: bool,
    /// --sim-import <路径>：脚本化导入（**不弹对话框**，走同一条应用通路）——验证
    /// "字节 → 纹理 / 字体"这条线（真人点选择器那步无法在无头环境里跑）。
    sim_import: Option<String>,
    /// 文件导入：**已选好、等帧内应用**的图片路径（要 `f.draw().gpu()`；`Ctx` 在帧外
    /// 拿不到 `Gpu`）。请求与状态在 [`TopBar`]（那是显示它们的模块）。
    import_image_path: Option<std::path::PathBuf>,
    /// **菜单栏**里的"文件名过滤"输入框内容（演示"菜单里也能放文本输入"）。
    ///
    /// ⚠ 富内容下拉菜单里的文本输入**共用同一个缓冲**（同一份内容，两个地方都能改：
    /// "菜单里能放文本输入"这件事在菜单栏与下拉菜单上是同一套能力）。
    menu_filter: String,
    /// **统一后的下拉菜单**演示状态 ①：选项列表模式（`Dropdown::options`）的选中索引。
    dd_opt_idx: u32,
    /// 演示状态 ②：富内容菜单里**子菜单**（`Item::submenu`，Submenu）里点了第几项。
    dd_sub_idx: Option<usize>,
    /// 演示状态 ③：富内容菜单里点了菜单项的次数。
    ///
    /// 为什么不用 `top.import_request` 判定：它在**下一帧开场**就被 `take` 走（真正的
    /// 导入通路），跨帧断言只会读到 `None`；计数器才跨帧稳定（`--sim-dropdown` 用它）。
    dd_item_clicks: u32,
    /// 演示状态 ④：点了「保持打开」（`MenuClick::Keep`）的次数 ——
    /// 点它之后 **popup 必须仍然开着**（`--sim-dropdown` 用它做硬断言）。
    dd_keep_clicks: u32,
    /// 菜单栏「视图」里「不收起」项（`MenuClick::Keep`）的点击次数（`--sim-menu` 断言用）。
    mb_keep_clicks: u32,
    /// --sim-dropdown：**实操统一后的下拉菜单**（选项列表 + 富内容两段；坐标运行时解算）。
    sim_dropdown: bool,
    /// --sim-dropdown：两个触发器的中心（每帧按常量 + 主题尺寸解算）。
    dd_opt_pt: Option<Vec2>,
    dd_file_pt: Option<Vec2>,
    /// --sim-dropdown：选项面板里第 1 行的中心（面板出现后才知道它在哪）。
    dd_opt_row0_pt: Option<Vec2>,
    /// --sim-dropdown：富内容面板里**文本输入**的中心（菜单第 1 行）。
    dd_input_pt: Option<Vec2>,
    /// --sim-dropdown：富内容面板里**第 1 个菜单项**（"导入图片…"）的中心。
    dd_item_pt: Option<Vec2>,
    /// --sim-dropdown：富内容面板里**子菜单行「编码」**的中心（Hover 目标）。
    dd_sub_row_pt: Option<Vec2>,
    /// --sim-dropdown：富内容面板里「保持打开」（`MenuClick::Keep`）行的中心。
    dd_keep_row_pt: Option<Vec2>,
    /// --sim-dropdown：子面板里第 2 项（"UTF-8"）的中心（点击目标）。
    dd_sub_item1_pt: Option<Vec2>,
    /// --sim-dropdown：子面板的**屏幕**原点（嵌套窗口 origin 已叠加父面板）。
    dd_sub_panel: Option<Vec2>,
    /// --sim-dropdown：子面板**应该**在的原点（= `popup_origin(子菜单行, Right)`）。
    dd_sub_want: Option<Vec2>,
    /// --sim-dropdown：解算出的两个面板原点（判定"面板该在触发器正下方"）。
    dd_opt_panel: Option<Vec2>,
    dd_file_panel: Option<Vec2>,
    /// --sim-dropdown：两个面板**应该**在的原点（= 触发器下方 + 2px；
    /// 由**公开**助手 `popup_origin` 算出 ⇒ 顺带守住"公开助手与引擎几何同源"）。
    dd_opt_want: Option<Vec2>,
    dd_file_want: Option<Vec2>,
    /// --sim-weight-modal：**实操字体对话框里的字重下拉** —— 守护"真的能选其他档位"
    /// （曾经失效：选中的索引只进局部变量、等"确定"才写回 ⇒ 每帧被重置，
    /// 勾/触发文字永远回到旧档位；`--sim-dropdown` 走的是 `Sel::One`，覆盖不到
    /// `combo_at` 这条 `Sel::Opt` 老入口）。
    sim_weight_modal: bool,
    /// --sim-weight-modal：下拉触发器 / 下拉第 1 行 / 对话框「确定」的点击点（录制时解算）。
    wm_trigger_pt: Option<Vec2>,
    wm_row0_pt: Option<Vec2>,
    wm_ok_pt: Option<Vec2>,
    /// --sim-menu：**实操菜单栏**（坐标运行时解算，不写死像素）。
    sim_menu: bool,
    /// --sim-resize：脚本化拖拽**右下角缩放柄**（验"宽高同调"与"不允许拖拽就不出柄"）。
    sim_resize: bool,
    /// --sim-resize：`img_box_fill` 的把手点（窗口右下角柄中心）与该窗口上一帧尺寸。
    sim_resize_pt: Option<Vec2>,
    sim_resize_size: Option<Vec2>,
    /// --sim-resize：每帧刷新的"当前尺寸"（帧 30 与 `sim_resize_size` 比）。
    sim_resize_after: Option<Vec2>,
    /// --sim-resize：**冻结**的拖拽目标点（柄会随窗口长大而移动；目标点若每帧重算，
    /// 鼠标就被"追着拖"，位移每帧累加——脚本自己会变成 bug 源）。
    sim_resize_to: Option<Vec2>,
    /// --sim-ta-resize：脚本化拖拽**行内多行 TextEditor** 的右下角缩放柄
    /// （验"申请尺寸 = 绘制尺寸 ⇒ 窗口/下面的控件跟着长" + "单行子项在行里被钉住"）。
    sim_ta_resize: bool,
    /// --sim-pick-save：走**真实的导出通路**（`export_request` → `Policy::save` →
    /// `save_theme`）；选择器是否真的弹、以及预设结果，全部由 [`filedialog::Policy`] 决定
    /// （`--no-file-dialog` 时**完全不碰 rfd**）。
    sim_pick_save: bool,
    /// --sim-scroll-mode：按轴溢出策略的验收现场（含 `vscroll(Scroll)` 的滚动条拖拽）。
    sim_scroll_mode: bool,
    /// --sim-scroll-mode：`vscroll(Scroll)` 那扇窗**右侧滚动条**上的抓取点（从 dump 的窗口
    /// 矩形 + 主题内边距解算 ⇒ 不写死像素）。
    scroll_thumb_pt: Option<Vec2>,
    /// **文件选择器策略**（导入 / 导出共用）：真人用法 = 弹系统选择器；测试 = 由显式 CLI
    /// （`--no-file-dialog` / `--pick <目标>=<路径|none>`）决定，可**完全不碰 `rfd`**。
    /// 见 [`filedialog::Policy`]（那里说明了为什么**不用**环境变量做旁路）。
    dialogs: filedialog::Policy,
    /// --sim-ta-resize：两个输入框的内容（多行那个要缩放；单行那个在 `min_h(60)` 行里）。
    sim_ta_text: String,
    sim_ta_single: String,
    /// --sim-ta-resize：编辑器**窗口局部**矩形（`Response::rect`）与 `row(..)` 返回的行高。
    sim_ta_rect: Option<Rect>,
    sim_ta_row_h: f32,
    /// --sim-ta-resize：`row_builder().min_h(60)` 那一行的返回尺寸
    /// （单行子项自己被钉到 60 ⇒ 行高恰为 60 逻辑像素 = 90 物理像素）。
    sim_ta_pin_row: Vec2,
    /// --sim-ta-resize：grip 中心（屏幕物理，每帧由几何解算）。
    sim_ta_grip: Option<Vec2>,
    /// --sim-ta-resize：**冻结**的两段拖拽目标（柄随编辑器长大而移动 ⇒ 目标必须冻结）。
    sim_ta_to: Option<Vec2>,
    sim_ta_to2: Option<Vec2>,
    /// --sim-ta-resize：拖动前 / 后 / 过拖后的 `(编辑器尺寸, 窗口高)`。
    sim_ta_before: Option<(Vec2, f32)>,
    sim_ta_after: Option<(Vec2, f32)>,
    sim_ta_min: Option<(Vec2, f32)>,
    /// --sim-ta-resize：每帧刷新的 `(编辑器尺寸, 窗口高)` 与默认下限
    /// （判定在录制段**外**做 ⇒ 只能从这里读，段内 `ui` 已不可见）。
    sim_ta_now: Option<(Vec2, f32)>,
    sim_ta_min_want: Option<Vec2>,
    /// --sim-ta-resize：DPI scale（判定在段外做 ⇒ 一并记下）。
    sim_ta_scale: f32,
    /// --sim-ta-resize：标准行高 `Theme::row_h`（**物理像素**，主题已预乘）——③ 的对照值。
    sim_ta_std_row: f32,
    /// --sim-zorder：列表里"点得到"的两个屏幕点 `(行, 滚动条条带)`（每帧从几何解算：
    /// `win_b` 下缘 + 8，并夹进列表可视区——写死会被长高后的 `win_b` 盖住 ⇒ 点空）。
    sim_zorder_pts: Option<(Vec2, Vec2)>,
    /// --sim-menu：解算出的「视图」触发器中心（**每帧都算**：栏位置只跟主题有关）。
    menu_trigger_pt: Option<Vec2>,
    /// --sim-menu：下拉里第一个菜单项中心（**菜单展开后**才知道下拉窗口在哪）。
    menu_item_pt: Option<Vec2>,
    /// --sim-menu：下拉面板的 `(原点, 尺寸)`（脚本拖动用；每帧从 `debug_dump` 读）。
    menu_panel: Option<(Vec2, Vec2)>,
    /// --sim-menu：下拉面板**应该**在的原点（= 触发器左下 + 2px；由主题尺寸算出）。
    menu_want_origin: Option<Vec2>,
    /// --sim-menu：**栏内空白**处的点（栏覆盖整屏 ⇒ 该点一定在栏矩形内、且不在任何
    /// 触发器 / 控件上）——用来验证"点栏内空白**不**收起菜单"（`on_bar` 语义）。
    menu_bar_blank_pt: Option<Vec2>,
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
    /// --theme <路径>：**启动时指定主题**（TOML 序列化格式；见 `filedialog::load_theme_onto`）
    /// ——文件里出现的字段覆盖在"旋钮组装出来的主题"上，于是两行的手写文件也能当启动皮肤。
    theme_file: Option<String>,
    /// --sim-theme <路径>：**脚本化**验证"序列化 → 文件 → 反序列化 → 进引擎"（不弹对话框）。
    sim_theme: Option<String>,
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
            Some(ImportKind::Theme) => {
                // **主题导入 = 在当前主题上合并覆盖**（TOML）⇒ 立即当基底生效；
                // 旋钮（圆角 / 密度 / 调色…）暂不生效直到「恢复调节」（窗口里有提示）。
                let base = self
                    .theme_tuner
                    .theme(self.top.font_name(), self.top.font_weight());
                match filedialog::load_theme_onto(&path, &base) {
                    Ok(t) => {
                        self.top.import_status = format!("主题：{label}");
                        self.theme_tuner.override_theme = Some(t);
                    }
                    Err(e) => self.top.import_status = format!("主题导入失败：{e}"),
                }
            }
            None => {
                self.top.import_status =
                    format!("不认得的文件类型：{label}（要图片 / ttf·otf·ttc / toml）");
                if verbose {
                    eprintln!("--font-file: 不认得的文件类型 {label}");
                }
            }
        }
    }

    /// **`--sim-theme <路径>`**：脚本化验证"**序列化 → 文件 → 反序列化 → 进引擎**"整条线
    /// （`rfd` 对话框本身无法无头跑，但文件这条通路是真实的）。
    ///
    /// 两阶段（各打印一行 `[OK]`/`[FAIL]`）：
    /// 1. **文件第 30 帧**：把**当前**主题 `to_toml()` 写到 `<路径>`，再读回来
    ///    `Theme::from_toml` ⇒ 两边**再导出一次文本必须逐字相同**（字段级往返一致）；
    /// 2. **第 60 帧**：把文件里 `[theme]` 的 `row_h` 改成一个**明显不同的值**写回，
    ///    再走应用的真通路（`load_theme_onto` → `override_theme`）⇒
    ///    `theme_tuner.theme(..)`（引擎真正吃的那一份）的 `row_h` 必须变成新值。
    fn sim_theme_io(&mut self, ctx: &mut Ctx, path: std::path::PathBuf) {
        let frame = ctx.frames();
        match frame {
            30 => {
                let mut theme = self
                    .theme_tuner
                    .theme(self.top.font_name(), self.top.font_weight());
                // **故意塞一个渐变刷**：这就是用户踩的那个坑——serde 默认的"外部标签枚举"
                // 在 TOML 里变成数组表，读回来报 "wanted exactly 1 element, more than 1
                // element in `button.bg`"。刷子现在有显式表示 `{kind, colors}`，必须往返。
                theme.button.bg = rjw_krusie::ui::Brush::Vertical(
                    rjw_krusie::color::Color::rgba_u8(20, 40, 60, 255),
                    rjw_krusie::color::Color::rgba_u8(200, 210, 220, 255),
                );
                let text = match theme.to_toml() {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("sim-theme: ① 序列化失败：{e} [FAIL]");
                        return;
                    }
                };
                if let Err(e) = std::fs::write(&path, &text) {
                    eprintln!("sim-theme: ① 写文件失败：{e} [FAIL]");
                    return;
                }
                let back = match std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|s| Theme::from_toml(&s))
                {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("sim-theme: ① 读回失败：{e} [FAIL]");
                        return;
                    }
                };
                let again = back.to_toml().unwrap_or_default();
                let ok = again == text
                    && back.button.bg == theme.button.bg
                    && back.button.bg != rjw_krusie::ui::Brush::Solid(rjw_krusie::color::Color::WHITE);
                eprintln!(
                    "sim-theme: ① {} 字节 · row_h={} gap={} 字重={} · 渐变刷往返={:?} 再导出逐字相同={} {}",
                    text.len(),
                    theme.row_h,
                    theme.gap,
                    theme.font_weight.0,
                    back.button.bg,
                    again == text,
                    if ok {
                        "[OK] 主题导出 → 文件 → 导入：字段级往返一致（含渐变刷）"
                    } else {
                        "[FAIL] 往返后字段变了（渐变刷丢了？）"
                    }
                );
            }
            60 => {
                // 真实用户流程：**导出全量文件 → 手改一行 → 导入**。
                let Ok(text) = std::fs::read_to_string(&path) else {
                    eprintln!("sim-theme: ② 文件读不到 [FAIL]");
                    return;
                };
                let patched: String = text
                    .lines()
                    .map(|l| if l.starts_with("row_h =") { "row_h = 27.0" } else { l })
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Err(e) = std::fs::write(&path, &patched) {
                    eprintln!("sim-theme: ② 写文件失败：{e} [FAIL]");
                    return;
                }
                let base = self
                    .theme_tuner
                    .theme(self.top.font_name(), self.top.font_weight());
                let before = base.row_h;
                match filedialog::load_theme_onto(&path, &base) {
                    Ok(t) => {
                        self.theme_tuner.override_theme = Some(t);
                        let now = self
                            .theme_tuner
                            .theme(self.top.font_name(), self.top.font_weight())
                            .row_h;
                        let ok = (now - 27.0).abs() < 0.01 && (before - 27.0).abs() >= 0.01;
                        eprintln!(
                            "sim-theme: ② 导入前 row_h={before} → 引擎侧 row_h={now}（期望 27）{}",
                            if ok {
                                "[OK] 导入的主题真的进了引擎（全量文件改一行 ⇒ 只那一项变）"
                            } else {
                                "[FAIL] 导入的主题没进引擎 / 覆盖语义不对"
                            }
                        );
                    }
                    Err(e) => eprintln!("sim-theme: ② 导入失败：{e} [FAIL]"),
                }
            }
            _ => {}
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
            sim_picker_popup: None,
            sim_overlap: false,
            sim_click: None,
            overlap: OverlapDemo::default(),
            sim_cover: false,
            sim_chrome: false,
            sim_weight: false,
            sim_shadow: false,
            sim_clip: false,
            clip_probe: None,
            shadow_probe: None,
            sim_tuner_pts: None,
            tuner_probe: None,
            sim_seg_pt: None,
            sim_tuner: false,
            sim_import: None,
            import_image_path: None,
            menu_filter: String::new(),
            dd_opt_idx: 1,
            dd_sub_idx: None,
            dd_item_clicks: 0,
            dd_keep_clicks: 0,
            mb_keep_clicks: 0,
            sim_dropdown: false,
            dd_opt_pt: None,
            dd_file_pt: None,
            dd_opt_row0_pt: None,
            dd_input_pt: None,
            dd_item_pt: None,
            dd_sub_row_pt: None,
            dd_keep_row_pt: None,
            dd_sub_item1_pt: None,
            dd_sub_panel: None,
            dd_sub_want: None,
            dd_opt_panel: None,
            dd_file_panel: None,
            dd_opt_want: None,
            dd_file_want: None,
            sim_weight_modal: false,
            wm_trigger_pt: None,
            wm_row0_pt: None,
            wm_ok_pt: None,
            sim_menu: false,
            sim_resize: false,
            sim_resize_pt: None,
            sim_resize_size: None,
            sim_resize_after: None,
            sim_resize_to: None,
            sim_ta_resize: false,
            sim_pick_save: false,
            sim_scroll_mode: false,
            scroll_thumb_pt: None,
            dialogs: filedialog::Policy::interactive(),
            sim_ta_text: String::new(),
            sim_ta_single: String::new(),
            sim_ta_rect: None,
            sim_ta_row_h: 0.0,
            sim_ta_pin_row: Vec2::ZERO,
            sim_ta_grip: None,
            sim_ta_to: None,
            sim_ta_to2: None,
            sim_ta_before: None,
            sim_ta_after: None,
            sim_ta_min: None,
            sim_ta_now: None,
            sim_ta_min_want: None,
            sim_ta_scale: 1.0,
            sim_ta_std_row: 0.0,
            sim_zorder_pts: None,
            menu_trigger_pt: None,
            menu_item_pt: None,
            menu_panel: None,
            menu_want_origin: None,
            menu_bar_blank_pt: None,
            weight_probe: None,
            cover: CoverDemo::default(),
            cover_starts_after_a: 0,
            image_file: None,
            font_file: None,
            theme_file: None,
            sim_theme: None,
            sim_zorder: false,
            sim_text_cull: false,
            cull_verts_before: None,
            cull_verts_after: None,
            cull_text_before: None,
            cull_text_after: None,
            zorder_panel: None,
            zorder_sel_before: None,
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
    /// `submit` 的**两笔账**：`asm` = UI 自己装配（组装/排序/切段/段内拼接顶点），
    /// `flush` = 交后端（`flush_seg` → `UiBackend::submit`，真实后端在这里把顶点拷进
    /// `rjw_2d_render` 暂存）。`asm + flush = submit`；两者之比回答"这 0.3ms 该算谁"。
    submit_asm_us: f64,
    submit_flush_us: f64,
    cmds: u64,
    wins: u64,
    hits: u64,
    misses: u64,
    /// 本帧带 scissor 的批次数（环境裁剪：严格窗口 / 滚动可视区 / Clip 沙箱 / 文本框盒）。
    /// 诊断用：它 = 0 而界面里明明有 Clip 沙箱 ⇒ 裁剪没接上；远大于窗口数 ⇒ 裁剪区太碎
    /// （每个不同 scissor 单独一次 draw，见 `docs/ENGINE_GUIDE.md` §18.20）。
    clip_batches: u64,
    /// 本帧提交段数（= draw call 候选）/ 顶点 / 三角（`[perf] segs=/verts=/tris=`）。
    /// `segs` 是"scissor 让 draw 变多"的直接度量；`verts/tris` 是"这一帧镶嵌了多少"。
    segs: u64,
    verts: u64,
    tris: u64,
    /// 本帧**真实 draw op 数**（`Render2D::draw_op_count()`，`prepare` 之后可查）。
    /// 与 `segs`（`UiBatch` 数 = 批次候选数）配对照：阶段 9 起提交计划按单元缓存，
    /// `segs` 会略涨，但相邻同状态批次在 `Render2D` 里合成一个动态段 ⇒ `draw_ops` 不涨。
    draw_ops: u64,
    /// 本帧被**兜底剔除**的**文本命令**条数（`UiStats::culled_text`）。**含合法剔除**
    /// （滚出可视区的列表条目文字），所以不是"必须 0"；`--sim-text-cull` 用它 + `verts`
    /// 一起判"输入框滚到末尾后文字是不是整块没了"。
    culled_text: u64,
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
            submit_asm_us: 0.0,
            submit_flush_us: 0.0,
            cmds: 0,
            wins: 0,
            hits: 0,
            misses: 0,
            clip_batches: 0,
            segs: 0,
            verts: 0,
            tris: 0,
            draw_ops: 0,
            culled_text: 0,
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
        self.submit_asm_us += s.submit_asm_us;
        self.submit_flush_us += s.submit_flush_us;
        self.cmds += s.cmd_count as u64;
        self.wins += s.win_count as u64;
        self.hits += s.cache_hits as u64;
        self.misses += s.cache_misses as u64;
        self.clip_batches += s.clip_batches as u64;
        self.segs += s.seg_count as u64;
        self.verts += s.vert_count as u64;
        self.tris += s.tri_count as u64;
        self.culled_text += s.culled_text as u64;
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
             | ui: sort={:.1}us sig={:.1}us collect={:.1}us clone={:.1}us \
               submit={:.1}us(asm={:.1} flush={:.1}) \
             | render: total={:.2}ms begin={:.1}us encode={:.1}us submit={:.1}us present={:.1}us \
             | cmds={:.0} wins={:.0} cache_hit={:.1} cache_miss={:.1} clip_batches={:.0} \
             segs={:.0} draw_ops={:.0} culled_text={:.0} verts={:.0} tris={:.0}",
            self.frame_us / n / 1000.0,
            self.sort_us / n,
            self.sig_us / n,
            self.collect_us / n,
            self.clone_us / n,
            self.submit_ui_us / n,
            self.submit_asm_us / n,
            self.submit_flush_us / n,
            self.render_us / n / 1000.0,
            self.begin_us / n,
            self.encode_us / n,
            self.submit_us / n,
            self.present_us / n,
            self.cmds as f64 / n,
            self.wins as f64 / n,
            self.hits as f64 / n,
            self.misses as f64 / n,
            self.clip_batches as f64 / n,
            self.segs as f64 / n,
            self.draw_ops as f64 / n,
            self.culled_text as f64 / n,
            self.verts as f64 / n,
            self.tris as f64 / n,
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
            match self.dialogs.open(kind) {
                Some(path) => self.apply_import(ctx, path, false),
                None => self.top.import_status = "导入已取消".to_owned(),
            }
        }
        // ── 主题**导出**（TOML）：帧外弹"另存为"，写的是**当前**主题 ─────────────
        // `theme_tuner.theme(..)` 与渲染用的那一份**同源**（`menu/窗口/控件` 都吃它）
        // ⇒ 导出的文件导入回来就是"现在这个样子"。
        if let Some(kind) = self.top.export_request.take() {
            // 选择器只有一条**显式**通路：`filedialog::Policy`（`--no-file-dialog` ⇒
            // **完全不碰 rfd**；`--pick theme-save=<路径|none>` ⇒ 预置这一次的结果）。
            // ⚠ 刻意**不读环境变量**：那会让"导出主题"能被外部悄悄改写到任意路径
            // （隐式、界面无异常 = 攻击入口）。见 `filedialog::Policy` 的文档。
            let picked = self.dialogs.save(kind);
            match kind {
                ExportKind::Theme => match picked {
                    Some(path) => {
                        let theme = self
                            .theme_tuner
                            .theme(self.top.font_name(), self.top.font_weight());
                        match filedialog::save_theme(&path, &theme) {
                            Ok(()) => {
                                self.top.import_status =
                                    format!("主题已导出：{}", filedialog::file_label(&path));
                            }
                            Err(e) => self.top.import_status = format!("主题导出失败：{e}"),
                        }
                    }
                    None => self.top.import_status = "导出已取消".to_owned(),
                },
            }
        }
        // ── `--theme <路径|builtin:名字>`：**启动时指定主题**（TOML 序列化格式）──────
        // 与「导入主题…」同一条通路（`filedialog::load_theme_onto`）：文件里出现的字段
        // 覆盖在**当前**（旋钮组装出来的）主题上 ⇒ 一个只写两行的文件也能当"启动皮肤"。
        // 放在帧外（不需要 `Ctx` / `Gpu`），失败只写状态行，不挡启动。
        //
        // `builtin:<名字>` 走**仓库内置主题**（`include_str!` 编译期导入，
        // `crates/rjw_ui/themes/*.toml`）——自动化测试用它：不依赖磁盘路径、不带外部
        // 文件，跑的就是提交进仓库的那份文本。`--theme builtin:` 后跟不存在名字时把
        // 可用名字列出来（`apply_builtin_theme` 的错误消息）。
        if let Some(spec) = self.theme_file.take() {
            let base = self
                .theme_tuner
                .theme(self.top.font_name(), self.top.font_weight());
            let loaded = match spec.strip_prefix("builtin:") {
                Some(name) => {
                    let mut t = base;
                    match rjw_krusie::ui::theme_toml::apply_builtin_theme(&mut t, name) {
                        Ok(()) => Ok(t),
                        Err(e) => Err(e),
                    }
                }
                None => {
                    let path = std::path::PathBuf::from(&spec);
                    filedialog::load_theme_onto(&path, &base)
                }
            };
            match loaded {
                Ok(t) => {
                    self.top.import_status = format!("主题已载入：{spec}");
                    self.theme_tuner.override_theme = Some(t);
                    eprintln!("theme: 启动载入 {spec}");
                }
                Err(e) => {
                    self.top.import_status = format!("主题载入失败：{e}");
                    eprintln!("theme: 启动载入失败 {spec}：{e}");
                }
            }
        }
        // ── `--sim-theme <路径>`：**脚本化验证 序列化 → 文件 → 反序列化 → 进引擎** ──
        // 与 `--sim-import` 同一套思路：`rfd` 对话框本身无法在无头环境里跑，所以脚本直接
        // 走"文件"这条真实通路（导出 → 读回 → 改一个字段 → 再导入 → 断言引擎侧主题变了）。
        if let Some(p) = self.sim_theme.clone() {
            self.sim_theme_io(ctx, std::path::PathBuf::from(p));
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
        // ── 仿真前置：**脚本自己把需要的面板 / 窗口打开** ────────────────────────
        // 演示的默认开关（`theme_tuner.open` / `win_a_open`）是**应用自己的选择**，
        // 脚本不该依赖它——否则"把面板默认关掉"就会让一堆仿真莫名其妙地失败。
        // 这里在开头几帧无条件置位（幂等）。
        let sim_lead = f.frames() <= 4;
        if sim_lead {
            if self.sim_tuner || self.sim_menu {
                self.theme_tuner.open = true;
            }
            if self.sim_chrome {
                self.windows.win_a_open = true;
            }
            // `--sim-dropdown`：两个演示下拉在顶栏右侧的固定位置，**别被别的窗口盖住**
            // （窗口 z 高于 win=0 内容）⇒ 脚本自己把它们收起来。
            if self.sim_dropdown {
                self.theme_tuner.open = false;
                self.windows.win_a_open = false;
            }
            // `--sim-weight-modal`：脚本自己打开字体对话框（并重置字重草稿，
            // 与「字体…」按钮按下时同一条路）。
            if self.sim_weight_modal {
                self.top.font_weight_draft = self.top.font_weight;
                self.top.font_modal_open = true;
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
            // 取色面板内部的固定常量（与 `panel.rs` 一致；`PICKER_*` 与判定段同源）+
            // 主题尺寸 × DPI。`hue_w` 只有面板坐标解算用，不在 `picker_panel_w` 里。
            let (pad, gap, hue_w, label_w) = (PICKER_PAD, PICKER_GAP, 14.0f32, PICKER_LABEL_W);
            let (row, input_h, field_w) = (
                theme.row_h * scale,
                theme.input.height * scale,
                theme.input.min_w * scale,
            );
            // 取色器**在 `win=0`**（见 `TopBar::ui` 的注释）：脚本坐标 = 锚点 + 常数
            // （面板是顶层浮层，位置见下），锚点就是演示位置本身。
            let anchor = PICKER_DEMO_POS;
            let swatch = Vec2::new(anchor.x + field_w * 0.5, anchor.y + 11.0);
            let pw = picker_panel_w(field_w);
            let body_w = pw - pad * 2.0;
            // 面板**实测**原点（见 `sim_picker_popup`）；还没读到就先按锚点 + 24 估一个
            // （仅第 20~21 帧用得到，那两帧只点色块，不碰面板内部坐标）。
            let origin = self.sim_picker_popup.map_or(
                Vec2::new(anchor.x, anchor.y + 24.0),
                |o| o + Vec2::splat(1.0),
            );
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
                // 面板仍开着，把**入口色块**换成远宽于默认的尺寸（下一帧生效）：
                // 第 92 帧的判定要看到"对话框宽度**没变**"（宽取主题口径，与入口无关）。
                88 => self.top.demo_picker_size = Some(PICKER_BIG_TRIGGER),
                96 => f.debug_inject_mouse(Vec2::new(900.0, 100.0), true), // 点面板外 → 收起
                97 => f.debug_inject_mouse(Vec2::new(900.0, 100.0), false),
                _ => {}
            }
        }

        // ── 调试：脚本化鼠标（`--sim-clip`）──────────────────────────
        // **用户报告的 bug**：拖动"被裁窗口"时，**scissor 不跟着内容走**（缓存的裁剪层
        // 一度存了绝对坐标 ⇒ 拖动中缓存命中后拿到过期矩形，裁到旧位置）。这里在
        // `strict_win` 标题栏上按住右移——第 30 / 90 帧的断言会同时检查"窗口确实移动了"
        // 与"scissor 仍等于窗口内容区（即跟着一起走）"。
        if self.sim_clip {
            let n = f.frames();
            let start = Vec2::new(760.0, 700.0); // strict_win 标题栏（origin ≈ 711,690）
            match n {
                35 => f.debug_inject_mouse(start, true),
                36..=80 => {
                    // ⚠ **往左拖**：该窗口被 `WindowClamp::Screen` 夹在屏幕右缘
                    // （560 → 474 逻辑），往右拖不动 —— 那样这条断言会"看起来通过"
                    // 却什么都没验证（实测踩过）。
                    let dx = (n - 35) as f32 * 5.0;
                    f.debug_inject_mouse(Vec2::new(start.x - dx, start.y), true)
                }
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

        // ── 调试：脚本化鼠标（`--sim-zorder`）─────────────────────
        // 摆出两个"必须靠绘制序判断"的重叠现场（**不看命中**：命中由 `hit_regions` 管，
        // 与画在谁上面无关）：
        //   ① 把 `name_panel` 拖到顶部「FPS / 点击次数」标签上（面板底色原本被更早录制的
        //      win=0 内容穿透 ⇒ 标签从面板里透出来）；
        //   ② 点 `list_demo` 的滚动条**条带**翻页（滑块原本画在列表项之下 ⇒ 只在条目
        //      间隙里闪），再点同一屏幕点 ⇒ 选中项号必须变大。
        // 按下点取在面板**内边距**里（`pad * 0.5`，避开内部 label / 输入框），否则输入框会
        // `press_claimed` 撤销拖拽基准 ⇒ 拖不动（见 `panel_impl` 的按下裁决）。
        if self.sim_zorder {
            let n = f.frames();
            // 面板原点（物理）+ 内边距的一半：内边距 = (panel.padding + border_w) × scale。
            // `Theme` 在 `Ui` 内才按 DPI 预乘 ⇒ 这里手动乘 `scale`（同 `--sim-chrome`）。
            let th = self.theme_tuner.theme(self.top.font_name(), self.top.font_weight());
            let pad = (th.panel.padding + th.panel.border_w) * scale;
            let from = NAME_PANEL_POS * scale + Vec2::splat(pad * 0.5);
            let to = ZORDER_PANEL_DST + Vec2::splat(pad * 0.5);
            // 列表里的两个固定屏幕点（**同一物理点**，翻页前后各点一次）：
            // 行 = 列表内左缘 + 10、条带 = 列表右缘 − 7（`SCROLLBAR_W` 的中心）。
            // ⚠ **都由几何解算**（见段 2 的 `--sim-zorder` 解算块）：写死的 y 会被
            // "内容变高后"的 `win_b` 盖住 —— 那时点下去命中的是 `win_b` 的文本框，
            // 行点击选不中、条带点击翻不了页（`TextEditor` 默认尺寸按 DPI 修正后踩过）。
            let (row, strip) = self.sim_zorder_pts.unwrap_or((
                Vec2::new((LIST_POS.x + 10.0) * scale, 620.0),
                Vec2::new((LIST_POS.x + LIST_SIZE.x) * scale - 7.0, 620.0),
            ));
            match n {
                // ① 拖动玩家名面板（按下帧先无条件建立基准，位移 ≥ 3px 才激活）。
                20 => f.debug_inject_mouse(from, true),
                21..=70 => {
                    let k = (n - 20) as f32 / 50.0;
                    f.debug_inject_mouse(from + (to - from) * k, true)
                }
                71..=120 => f.debug_inject_mouse(to, true),
                121 => f.debug_inject_mouse(to, false),
                // ② 翻页前先点一次列表里的固定点（作为"翻页后必须更大"的基线）。
                126 => f.debug_inject_mouse(row, true),
                127 => f.debug_inject_mouse(row, false),
                // ③ 点条带（滑块在下 ⇒ 走翻页路径），内容下移一格可视区。
                134 => f.debug_inject_mouse(strip, true),
                135 => f.debug_inject_mouse(strip, false),
                // ④ 再点**同一个屏幕点**：内容已滚过 ⇒ 命中的条目号必须更大。
                142 => f.debug_inject_mouse(row, true),
                143 => f.debug_inject_mouse(row, false),
                _ => {}
            }
        }

        // ── 调试：脚本化鼠标（`--sim-ta-resize`）────────────────────
        // ① 拖 `ta_edit`（行内多行 TextEditor）的右下角缩放柄 +120/+80 物理像素；
        // ② 再往左上猛拖 300 → 期望被**默认下限**挡住（一行文字高 / `min_w`，拖不到 0）。
        // 目标点**冻结**（柄随编辑器长大而移动；每帧重算就是"追着拖"）。
        if self.sim_ta_resize {
            let n = f.frames();
            let grip = self.sim_ta_grip.unwrap_or(Vec2::ZERO);
            match n {
                34 => {
                    let to = grip + Vec2::new(120.0, 80.0);
                    self.sim_ta_to = Some(to);
                    f.debug_inject_mouse(grip, true);
                }
                35..=50 => {
                    let to = self.sim_ta_to.unwrap_or(grip);
                    let k = (n - 34) as f32 / 16.0;
                    f.debug_inject_mouse(grip + (to - grip) * k, true);
                }
                51 => f.debug_inject_mouse(self.sim_ta_to.unwrap_or(grip), false),
                // ② 过拖：起点 = 现在的柄（已长大），终点冻结为左上方向 300。
                64 => {
                    let to = grip - Vec2::new(300.0, 300.0);
                    self.sim_ta_to2 = Some(to);
                    f.debug_inject_mouse(grip, true);
                }
                65..=80 => {
                    let from = grip;
                    let to = self.sim_ta_to2.unwrap_or(from);
                    let k = (n - 64) as f32 / 16.0;
                    f.debug_inject_mouse(from + (to - from) * k, true);
                }
                81 => f.debug_inject_mouse(self.sim_ta_to2.unwrap_or(grip), false),
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
        // **真的去点**标题栏那几个按钮（不是直接翻 flag）：命中 → 按下认领 → 释放结算
        // 这条完整路径才被验证。坐标**从上一帧 `debug_dump` 的窗口矩形推导**
        // （见段 2 里 `chrome_pts` / `eng_pts` 的解算）：按钮右缘 = 外框右缘 ⇒ 点最外缘那格。
        //
        // 调度（注入只对**下一帧**生效 ⇒ 按下/抬起各留两帧）：
        //   12..13 ↓⌃ / 14..15 ↑⌃（点收起；应用持有 bool）→ 60 帧由应用重开
        //   40..41 ↓× / 42..43 ↑×（点关闭）→ 80 帧由应用展开
        //   104..115 ↓/↑ ⌃ ×2（**引擎托管**那个窗口：收起 → 展开，应用侧没有 bool）
        if self.sim_chrome {
            // 外框矩形口径：`right = origin.x + size.x`、`btn = row_h - 2`（同引擎）。
            // ⚠ 点的是**最外缘**：这条就是"✕ 贴窗口右缘（Windows 风格）"的脚本级回归
            // —— 按钮若缩回内容右缘（旧行为，差 `pad + 4`），这两下就落空了。
            let (fold_p, close_p) = self.windows.chrome_pts.unwrap_or((Vec2::ZERO, Vec2::ZERO));
            let eng_p = self.windows.eng_pts.unwrap_or(Vec2::ZERO);
            let eng_c = self.windows.eng_center_pt.unwrap_or(eng_p);
            // 阶段 6：`img_box_fill` 右下角的**缩放柄**中心（`Resize::Both`）。
            let eng_grip = self
                .windows
                .eng_rect
                .map(|(o, s)| Vec2::new(o.x + s.x - 8.0, o.y + s.y - 8.0))
                .unwrap_or(eng_p);
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
                // 阶段 4：**引擎托管的收起**（`img_box_fill` 用的是 `collapsible(true, None)`）
                // —— 点 ⌃ 收起、再点 ⌃ 展开，**应用侧一个 bool 都没有**（状态在 `UiState`）。
                // ⚠ 先把 win_a 挪回右上角：它在阶段 1 被挪到 `(40,470)`（逻辑），矩形
                // `(60,705) 358×306` 正好**盖住** `img_box_fill` 的 ⌃ 像素（`--ui-dump` 可见；
                // 被更高 z 的窗口盖住 ⇒ `window_occluded` 让命中失效，症状就是"点不动"）。
                100 => self.windows.win_a_pos = Vec2::new(560.0, 240.0),
                104..=105 => f.debug_inject_mouse(eng_p, true),
                106..=107 => f.debug_inject_mouse(eng_p, false),
                112..=113 => f.debug_inject_mouse(eng_p, true),
                114..=115 => f.debug_inject_mouse(eng_p, false),
                // 阶段 5：点同一个 ⌃ 的**行中心**——收起态里窗口右下角的缩放柄正压在这里，
                // 旧实现（柄先跑）会把这次按下抢走 ⇒ 状态不翻转。本轮"柄让位给内容/按钮"
                // 之后必须照常翻转（`!press_claimed` 才应用窗口柄）。
                140..=141 => f.debug_inject_mouse(eng_c, true),
                142..=143 => f.debug_inject_mouse(eng_c, false),
                // 阶段 6：**先确定地展开 → 把高度拖大 → 再点 ⌃** ——
                // 持久高不能让收起后的窗口"空着不倒"（用户实测："点击收起后不会收起"）。
                // 展开走**公开状态 API**（`set_collapsed(.., false)`）而不是再点一次 ⌃：
                // 收起态里窗口柄正压在 ⌃ 上，靠点击会随"谁赢"而抖（见 ui 段的调用）。
                190..=191 => f.debug_inject_mouse(eng_grip, false),
                192..=193 => {
                    self.windows.eng_drag_to = Some(Vec2::new(eng_grip.x, eng_grip.y + 40.0));
                    f.debug_inject_mouse(eng_grip, true)
                }
                194..=199 => {
                    let to = self.windows.eng_drag_to.unwrap_or(eng_grip);
                    f.debug_inject_mouse(to, true)
                }
                200..=204 => {
                    let to = self.windows.eng_drag_to.unwrap_or(eng_grip);
                    f.debug_inject_mouse(to, false)
                }
                210..=211 => f.debug_inject_mouse(eng_p, true),
                212..=213 => f.debug_inject_mouse(eng_p, false),
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
            let seg = self.sim_seg_pt.unwrap_or(Vec2::ZERO);
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
                // 阶段 3：点预设行的第 3 段（"legacy"）——分段按钮组可交互。
                50..=51 => f.debug_inject_mouse(seg, false),
                52..=53 => f.debug_inject_mouse(seg, true),
                54..=59 => f.debug_inject_mouse(seg, false),
                _ => {}
            }
        }
        // ── 调试：脚本化鼠标（`--sim-resize`）──────────────────────
        // 拖 `img_box_fill` 的右下角柄（**斜向**拖 +60/+40）⇒ 宽高都该变大
        // （`.resize(true, Resize::Both)`）。坐标 = 窗口原点 + 结算尺寸 − 半个柄。
        if self.sim_resize {
            let p = self.sim_resize_pt.unwrap_or(Vec2::ZERO);
            // **冻结目标点**（帧 8 记录）：柄随窗口长大而移动，每帧重算目标 = 鼠标被
            // "追着拖"，位移逐帧累加（第一版就这么错：拖 +60 结果宽了 +180）。
            let to = self.sim_resize_to.unwrap_or(Vec2::new(p.x + 60.0, p.y + 40.0));
            // 阶段 2 的点（没有 `.width()` 的那个窗口的柄）；冻结成"右移 50"。
            let gp = self.windows.grip_pt.unwrap_or(Vec2::ZERO);
            let gto = self
                .windows
                .grip_to
                .unwrap_or(Vec2::new(gp.x + 50.0, gp.y));
            // 阶段 3 的目标：同一个柄**只往上**（收缩高度）；已冻结。
            let up = self
                .windows
                .resize_up
                .unwrap_or(Vec2::new(p.x, p.y - 40.0));
            match f.frames() {
                10..=11 => f.debug_inject_mouse(p, false),
                12..=13 => f.debug_inject_mouse(p, true),
                14..=19 => f.debug_inject_mouse(to, true),
                20..=24 => f.debug_inject_mouse(to, false),
                // 阶段 2：**横向**拖 `grip_win` 的柄（只右移，不碰 y）⇒ 只该宽变。
                62..=63 => f.debug_inject_mouse(gp, false),
                64..=65 => f.debug_inject_mouse(gp, true),
                66..=71 => f.debug_inject_mouse(gto, true),
                72..=76 => f.debug_inject_mouse(gto, false),
                // 阶段 3：**向上收缩** `img_box_fill` 的高度（用户实测："窗口收缩高度
                // 应当为恒定值"）：只动 y（−40），松手后高度必须停在拖出来的值上。
                90..=91 => f.debug_inject_mouse(p, false),
                92..=93 => f.debug_inject_mouse(p, true),
                94..=99 => f.debug_inject_mouse(up, true),
                100..=104 => f.debug_inject_mouse(up, false),
                // 阶段 4：**再点一下柄（不拖动）** —— 用户实测："缩放柄拖到别处后，第二次
                // 点击会瞬移到原位"。纯点击不该改尺寸（更不该跳回上一次拖之前的尺寸）。
                170..=171 => f.debug_inject_mouse(p, false),
                172..=173 => f.debug_inject_mouse(p, true),
                174..=176 => f.debug_inject_mouse(p, false),
                _ => {}
            }
        }
        // ── 调试：脚本化鼠标（`--sim-row-overflow`）──────────────
        // 两个点由 ui 段从 `debug_dump` 的窗口原点 + 主题内边距解算（不写死像素）：
        // ① 窗口**里**的正确控件（正对照，必须点得到 ⇒ 证明守卫没有把整窗点废）；
        // ② 窗口**外**的溢出控件（裁掉了 ⇒ 必须点不到；点到了就是"幽灵控件"回来了）。
        if self.windows.sim_row_overflow {
            let inside = self.windows.row_inside_pt.unwrap_or(Vec2::ZERO);
            let ghost = self.windows.row_ghost_pt.unwrap_or(Vec2::ZERO);
            match f.frames() {
                20..=21 => f.debug_inject_mouse(inside, false),
                22..=23 => f.debug_inject_mouse(inside, true),
                24..=25 => f.debug_inject_mouse(inside, false),
                32..=33 => f.debug_inject_mouse(ghost, false),
                34..=35 => f.debug_inject_mouse(ghost, true),
                36..=37 => f.debug_inject_mouse(ghost, false),
                _ => {}
            }
        }
        // ── 调试：脚本化鼠标（`--sim-pick-save`）──────────────────
        // 第 20 帧按下「导出主题…」那条路（只记请求）：帧外处理器会走
        // `--sim-pick-save <路径|none>`（显式 CLI ⇒ 不弹阻塞的另存为），于是"导出主题"
        // 整条通路可无人值守跑完。缺值就什么都不做（只印用法）。
        if self.sim_pick_save && f.frames() == 20 {
            self.top.export_request = Some(ExportKind::Theme);
        }
        // ── 调试：脚本化鼠标（`--sim-scroll-mode`）────────────────
        // 拖 `vscroll(Scroll)` 那扇窗的**滚动条 thumb**（往下 40px）：`ScrollState.offset`
        // 必须真的变大（滚轮没法注入 ⇒ 拖 thumb 是唯一能脚本化的滚动入口）。
        if self.sim_scroll_mode {
            let p = self.scroll_thumb_pt.unwrap_or(Vec2::ZERO);
            // 单击（不拖）：滚动条轨道 ⇒ 翻一页。
            match f.frames() {
                20..=21 => f.debug_inject_mouse(p, false),
                22..=23 => f.debug_inject_mouse(p, true),
                24..=26 => f.debug_inject_mouse(p, false),
                _ => {}
            }
        }
        // ── 调试：脚本化鼠标（`--sim-menu`）──────────────────────
        // 行程：10..11 移到「视图」触发器 → 12..13 按下 → 14..15 抬起（菜单打开）
        //      → 22..23 按下第一个菜单项（"主题调节窗口"）→ 24..25 抬起
        // （坐标由上一帧录制时解算：触发器按主题尺寸算，菜单项按**下拉窗口原点**算。）
        if self.sim_menu {
            let trigger = self.menu_trigger_pt.unwrap_or(Vec2::ZERO);
            let item = self.menu_item_pt.unwrap_or(trigger);
            let bar_blank = self.menu_bar_blank_pt.unwrap_or(trigger);
            // 阶段 2 用：**面板里"没有控件"的地方**（标题行左侧的空白——菜单项只占上半，
            // 密度标题只从勾选列起排）⇒ 在那里按下会落到"窗口本体"，正是"拖菜单"的入口。
            // 拖到面板**右侧外面**松手（不会点到任何菜单项）。
            let grab = self
                .menu_panel
                .map(|(o, s)| Vec2::new(o.x + 20.0, o.y + s.y - 60.0))
                .unwrap_or(trigger);
            let aside = self
                .menu_panel
                .map(|(o, s)| Vec2::new(o.x + s.x + 30.0, grab.y))
                .unwrap_or(trigger);
            match f.frames() {
                10..=11 => f.debug_inject_mouse(trigger, false),
                12..=13 => f.debug_inject_mouse(trigger, true),
                14..=15 => f.debug_inject_mouse(trigger, false),
                22..=23 => f.debug_inject_mouse(item, true),
                24..=25 => f.debug_inject_mouse(item, false),
                // 阶段 2：重开菜单 → 在**面板空白处**按住 → 拖到面板外 → 松手。
                // 面板必须**原地不动**（`WindowClamp::Locked`：下拉浮层不可拖动）。
                40..=41 => f.debug_inject_mouse(trigger, false),
                42..=43 => f.debug_inject_mouse(trigger, true),
                44..=45 => f.debug_inject_mouse(trigger, false),
                46..=47 => f.debug_inject_mouse(grab, true),
                48..=52 => f.debug_inject_mouse(aside, true),
                53..=54 => f.debug_inject_mouse(aside, false),
                // 阶段 3：菜单**还开着**（阶段 2 没有关它）⇒ 直接**点栏内空白**
                // （不是触发器、也不是下拉面板）：菜单必须**仍开着**（`on_bar` 语义；
                // 旧实现只认"落在某个触发器上"，点栏里空白会误关）。
                64..=65 => f.debug_inject_mouse(bar_blank, true),
                66..=67 => f.debug_inject_mouse(bar_blank, false),
                _ => {}
            }
        }
        // ── 调试：脚本化鼠标（`--sim-dropdown`）────────────────────
        // 行程（坐标由上一帧录制时解算：触发器按常量 + 主题尺寸，面板行按**面板窗口原点**
        // + `popup_padding` / `popup_gap` / `item_h` —— 与引擎同源，不写死像素）：
        //   ① 10..15 点选项下拉触发器 → 菜单打开
        //   ② 22..25 点第 1 个选项（"简单"）→ 选中 + 自动收起
        //   ③ 40..45 点富内容下拉触发器 → 菜单打开
        //   ④ 52..55 点菜单里**第 1 行的文本输入** → 聚焦（"菜单内又是 UiAdd"的实证）
        //   ⑤ 60..70 **只悬停**子菜单行「编码」→ 子面板在**行右侧**展开、父 popup 保留
        //   ⑥ 74..77 点子面板里的"UTF-8" → **整条链**（子 + 父）收起
        //   ⑦ 88..93 重开富内容下拉 → 100..103 点「保持打开」项 → **popup 保留**
        if self.sim_dropdown {
            let opt = self.dd_opt_pt.unwrap_or(Vec2::ZERO);
            let file = self.dd_file_pt.unwrap_or(Vec2::ZERO);
            let row0 = self.dd_opt_row0_pt.unwrap_or(opt);
            let input = self.dd_input_pt.unwrap_or(file);
            let sub_row = self.dd_sub_row_pt.unwrap_or(file);
            let sub_item1 = self.dd_sub_item1_pt.unwrap_or(sub_row);
            let keep_row = self.dd_keep_row_pt.unwrap_or(file);
            match f.frames() {
                10..=11 => f.debug_inject_mouse(opt, false),
                12..=13 => f.debug_inject_mouse(opt, true),
                14..=15 => f.debug_inject_mouse(opt, false), // ① 开
                22..=23 => f.debug_inject_mouse(row0, true), // ② 选第一个选项
                24..=25 => f.debug_inject_mouse(row0, false),
                40..=41 => f.debug_inject_mouse(file, false),
                42..=43 => f.debug_inject_mouse(file, true),
                44..=45 => f.debug_inject_mouse(file, false), // ③ 开
                52..=53 => f.debug_inject_mouse(input, true), // ④ 聚焦菜单里的文本输入
                54..=55 => f.debug_inject_mouse(input, false),
                // ⑤ **只移动、不按下**：Hover 就应当展开子菜单（每帧注入 ⇒ 悬停保持）
                60..=70 => f.debug_inject_mouse(sub_row, false),
                72..=73 => f.debug_inject_mouse(sub_item1, false),
                74..=75 => f.debug_inject_mouse(sub_item1, true), // ⑥ 点子菜单里的项
                76..=77 => f.debug_inject_mouse(sub_item1, false),
                88..=89 => f.debug_inject_mouse(file, false),
                90..=91 => f.debug_inject_mouse(file, true), // ⑦ 重开
                92..=93 => f.debug_inject_mouse(file, false),
                100..=101 => f.debug_inject_mouse(keep_row, true), // ⑧ 点「保持打开」
                102..=103 => f.debug_inject_mouse(keep_row, false),
                _ => {}
            }
        }
        // ── 调试：脚本化鼠标（`--sim-weight-modal`）────────────────
        // 行程：① 点开字重下拉 → ② 点第 1 档（"细 300"）→ ③ 点「确定」提交。
        // 坐标由上一帧录制时解算（对话框原点/尺寸来自 `debug_dump`，行偏移用主题尺寸 +
        // **公开**助手 `popup_origin` / `popup_padding` / `item_h`）。
        if self.sim_weight_modal {
            let trigger = self.wm_trigger_pt.unwrap_or(Vec2::ZERO);
            let row0 = self.wm_row0_pt.unwrap_or(trigger);
            let ok = self.wm_ok_pt.unwrap_or(trigger);
            match f.frames() {
                10..=11 => f.debug_inject_mouse(trigger, false),
                12..=13 => f.debug_inject_mouse(trigger, true),
                14..=15 => f.debug_inject_mouse(trigger, false), // ① 开下拉
                22..=23 => f.debug_inject_mouse(row0, true),     // ② 选"细 300"
                24..=25 => f.debug_inject_mouse(row0, false),
                40..=41 => f.debug_inject_mouse(ok, true), // ③ 确定（提交草稿）
                42..=43 => f.debug_inject_mouse(ok, false),
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
                // `Esc` 先让给**菜单栏**（菜单开着时那一帧不退出）：引擎在菜单栏里消费了
                // `Esc`（收起菜单），这里看**上一帧**的展开状态就够（菜单栏录在本帧后段）。
                if ui.key_down_edge(KeyCode::Escape) && ui.state().menu_open().is_none() {
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
            //
            // ── `--sim-picker`：记下面板窗口的**实测原点**（供下一帧的脚本坐标解算）──
            // 面板位置受 `WindowClamp` 影响（太高就被翻到上方），按"锚点 + 24"硬算会
            // 全盘落空 ⇒ 从引擎状态里读（`--sim-dropdown` / `--sim-chrome` 同路子）。
            if sim_picker && self.sim_picker_popup.is_none() {
                self.sim_picker_popup = ui
                    .debug_dump()
                    .windows
                    .iter()
                    .find(|w| w.id.ends_with("picker_demo::popup"))
                    .map(|w| w.origin);
            }
            if ui_dump {
                eprintln!("[段 1] {}", ui.debug_dump());
            }
            // ── `--sim-picker` 判定（第 92 帧）：**入口色块尺寸 ≠ 对话框尺寸** ──
            // 脚本第 88 帧把入口换成 `PICKER_BIG_TRIGGER`（200×40，远宽于默认），
            // 面板此时仍开着 ⇒ 面板外宽必须**仍是** `picker_panel_w(主题默认入口宽)`；
            // 旧实现按**入口实测宽 × 1.9** 算 ⇒ 这里会读到 ~380（立刻 FAIL）。
            // `+2` = 面板边框（`panel_style.border_w = 1`，两侧共 2 物理像素）。
            if sim_picker && sim_frame == 92 {
                let field_w = ui.theme().input.min_w;
                let want = picker_panel_w(field_w) + 2.0;
                let got = ui
                    .debug_dump()
                    .windows
                    .iter()
                    .find(|w| w.id.ends_with("picker_demo::popup"))
                    .map(|w| w.size.x);
                // 判据能失败的自证：入口宽 × 1.9 必须**明显不同于**期望值（否则等式
                // 两边恰好撞上，断言等于没写）。
                let bug = PICKER_BIG_TRIGGER.x * 1.9 + 2.0;
                let discriminates = (bug - want).abs() > 8.0;
                let ok = got.is_some_and(|g| (g - want).abs() <= 2.0) && discriminates;
                eprintln!(
                    "sim-picker: 入口={:?} 面板宽 期望={want:.0} 实测={:?}（按入口算会是 {bug:.0}） {}",
                    PICKER_BIG_TRIGGER,
                    got.map(|g| g.round()),
                    if ok {
                        "[OK] 入口尺寸改了、对话框宽度不变（宽取主题口径）"
                    } else {
                        "[FAIL] 面板宽跟着入口走了 / 面板没开"
                    }
                );
            }
            // ── `--sim-clip`（放在**窗口都录完**之后：`debug_dump` 只看本帧已录窗口）──
            // 环境裁剪走 batch scissor（严格窗口 / Clip 沙箱），而不是逐命令切割几何。
            // 判定口径（第 30 / 90 帧）：
            // ① `stats.clip_batches > 0`：本帧确实提交了带 scissor 的批次；
            // ② `strict_win` 的 `clip` 有值，且 = 该窗口内容区（原点 + 结算尺寸）——
            //    矩形错了 scissor 就裁错地方。
            // ⚠ 不做"普通窗必须无裁剪"的断言：任何窗口里的**文本框盒裁剪**都是一个
            // legit 的 scissor（`win_b` 实测就有一个 210×90 的输入框裁剪），
            // `debug_clip` 记的是该窗**最后一批**的 scissor。
            if self.sim_clip && (sim_frame == 30 || sim_frame == 90) {
                let dump = ui.debug_dump();
                let strict = dump.windows.iter().find(|w| w.id.ends_with("strict_win"));
                let strict_clip = strict.and_then(|w| w.clip);
                let clip_batches = ui.state().stats.clip_batches;
                // ① scissor == 该窗内容区（原点 + 结算尺寸）；
                let ok_rect = strict.map(|w| (w.origin, w.size)).is_some_and(|(o, s)| {
                    strict_clip.is_some_and(|c| {
                        (c.x - o.x).abs() <= 1.0
                            && (c.y - o.y).abs() <= 1.0
                            && (c.w - s.x).abs() <= 2.0
                    })
                });
                // ② **跟着窗口一起走**：第 90 帧时窗口已被脚本拖动过（origin 变了），
                //    scissor 必须仍然贴着它（上面那条已含此意，这里额外打印位移量）。
                if sim_frame == 30 {
                    self.clip_probe = strict.map(|w| w.origin);
                }
                let moved = match self.clip_probe {
                    Some(o0) => strict.map(|w| w.origin).is_some_and(|o| (o - o0).length() > 5.0),
                    None => false,
                };
                // 第 30 帧尚未拖动 ⇒ 不要求 moved；第 90 帧必须已移动且 scissor 仍贴合。
                let ok = clip_batches > 0 && ok_rect && (sim_frame == 30 || moved);
                eprintln!(
                    "sim-clip: 帧={sim_frame} clip_batches={clip_batches} 严格窗 clip={strict_clip:?}（内容区={:?}）移动过={moved} {}",
                    strict.map(|w| (w.origin, w.size)),
                    if ok {
                        "[OK] 环境裁剪进了批次 scissor，且拖动时 scissor 与内容一起移动"
                    } else {
                        "[FAIL] 裁剪没进 scissor / 矩形不对 / 拖动时 scissor 没跟着走"
                    }
                );
            }
            // `--sim-zorder`：记下 `name_panel` **实际结算位置**（判定"脚本真的拖动了
            // 面板"——否则这条现场是空跑的，同 `--sim-clip` 拖动段的教训）。
            // **绘制序本身应用侧读不到**：谁画在谁上面只能靠 `RJ_ORDER_TRACE=1`
            // 的 `order[...]` 行核对（引擎侧证据）。
            if self.sim_zorder && sim_frame == 100 {
                self.zorder_panel = ui.state().panel_pos.get("name_panel").copied();
            }
            // ── `--sim-text-cull`：**单行输入框滚过头文字消失**（用户报的 BUG）──
            // 摆现场（第 20 帧起换成 8 遍长文本；30 字 × 8 ≈ 全文宽远大于框内宽）：
            //   ① 第 22 帧读**未滚动**那一帧的 `verts`（帧 N 读到的是 N-1 的统计）；
            //   ② 第 24 帧把该控件的 `text_scroll` 推到 `TEXT_CULL_SCROLL`
            //      （引擎只在"滚轮 / 光标跟随"时改写它，脚本直接置位 = 等价于把视图
            //      拖到中段；取值**远大于框内宽、远小于全文宽**，是合法可达状态）；
            //   ③ 第 26 帧读**滚到中段**那一帧的 `verts`。
            // 修前：整条文本命令被 `collect_cmds` 的兜底剔除丢掉 ⇒ `verts` 掉一截
            // （`culled_text` 也会涨）；修后两者都不变。判定见下面的 `sim-text-cull:` 行。
            if self.sim_text_cull {
                if sim_frame == 20 {
                    self.windows.win_b_note = TEXT_CULL_LONG.repeat(8);
                }
                if sim_frame == 24 {
                    // 绝对 id 前缀由窗口决定（`win_b/res_input`）⇒ 按后缀找，不写死前缀。
                    if let Some((_, ws)) = ui
                        .state_mut()
                        .widgets
                        .iter_mut()
                        .find(|(id, _)| id.as_str().ends_with("res_input"))
                    {
                        ws.text_scroll = TEXT_CULL_SCROLL;
                    }
                }
                if sim_frame == 22 {
                    self.cull_verts_before = Some(ui.state().stats.vert_count);
                    self.cull_text_before = Some(ui.state().stats.culled_text);
                }
                if sim_frame == 26 {
                    self.cull_verts_after = Some(ui.state().stats.vert_count);
                    self.cull_text_after = Some(ui.state().stats.culled_text);
                }
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

            // ── **行 + 多行 TextEditor**（widget 路径；`--sim-ta-resize` 的现场）───
            //
            // 三件事一起演示 / 自证（都被 `--sim-ta-resize` 断言）：
            // ① **行 + 多行编辑器**：多行子项把行撑高（`SizeClass::Multiline` ⇒ 行高 = 子项高），
            //    编辑器右下角有缩放柄；
            // ② 拖大编辑器 ⇒ **窗口与下面的标签跟着长**（自动申请先问尺寸责任链
            //    [`Ui::resolved_size`]，所以"申请尺寸 = 绘制尺寸"）；
            // ③ 再放一行 `min_h(60)` 的**单行**控件：子项自己要 90 逻辑像素高，也会被
            //    行的 `min_h` **钉到 60**（单行控件的标准高）。
            // ⚠ 只在本仿真里出现：它的位置会盖住 `img_box_fill` 的 ⌃（`--sim-chrome` 要点的
            // 那个像素），常态演示留在 egUI 的 Gallery/后续模块里（P3 会把内容整体搬过去）。
            if self.sim_ta_resize {
                ui.window("ta_win").pos(vec2(20.0, 320.0)).show(|w| {
                    self.sim_ta_row_h = w
                        .row(|r| {
                            let resp = r.add(
                                TextEditor::new("ta_edit", &mut self.sim_ta_text)
                                    .multiline()
                                    .resize(Resize::Both),
                            );
                            self.sim_ta_rect = Some(resp.rect);
                        })
                        .y;
                    w.label("FOLLOW（拖大上面那行之后，我应该往下走）");
                    self.sim_ta_pin_row = w
                        .row_builder()
                        .min_h(60.0)
                        .show(|r| {
                            r.add(
                                TextEditor::new("ta_single", &mut self.sim_ta_single)
                                    .height(90.0),
                            );
                        });
                });
            }

            // ── **菜单栏**（横向；`Ui::menu_bar`）────────────────────────────────
            // 录在各窗口**之后**：下拉面板是浮层窗口，窗口 z 在**首次录制**时按 `max+1`
            // 分配 ⇒ 放最后才能保证它盖住别的窗口（见 `widgets/menubar.rs` 模块文档）。
            //
            // 三个菜单覆盖不同内容形态，把"闭包上下文"的能力演示全：
            // - 「文件」：**文本输入**（文件名过滤，实时回显）+ 分割线 + 菜单项（导入 / 退出）；
            // - 「视图」：**带勾选的菜单项**（窗口显隐 / 收起，直接绑应用自己的 `&mut bool`）
            //   + 标题行 + **横向排版**（密度三档按钮 —— `MenuCtx` 解引用到 `Window`）；
            // - 「帮助」：纯文本行（操作提示）。
            //
            // **栏 = 一行 + 全宽背景**：`bar.width(..)` 把背景铺满整个屏幕；菜单之间用
            // **竖向分割线**（`bar.separator_v()`）分组；菜单之后还能塞**任何控件**
            // （这里放一个状态标签）—— `MenuBar` 解引用到 `Pack` ⇒ `UiAdd` 的方法都能用。
            let screen_w = ui.window_physical_size().0 as f32;
            let menu_size = ui.menu_bar("menubar", MENUBAR_POS, |bar| {
                bar.width(Size::Physical(Vec2::new(screen_w, 0.0)));
                bar.menu("文件", |m| {
                    m.caption("文件名过滤（菜单里也能放文本输入）");
                    m.text_input("menu_filter", &mut self.menu_filter);
                    if !self.menu_filter.is_empty() {
                        m.caption(&format!("当前：{}", self.menu_filter));
                    }
                    m.separator();
                    if m.item("导入图片…") {
                        self.top.import_request = Some(ImportKind::Image);
                    }
                    if m.item("导入字体…") {
                        self.top.import_request = Some(ImportKind::Font);
                    }
                    m.separator();
                    if m.item("退出（Esc）") {
                        exit_requested = true;
                    }
                });
                bar.menu("视图", |m| {
                    m.item_checked("主题调节窗口", &mut self.theme_tuner.open);
                    // **责任链写法**（与上面 `item_checked` 等价，两种都留着当对照）：
                    // `Item::new(..).checked(..)` = 勾选项、点击即收起。
                    m.item(Item::new("窗口 A 显示").checked(&mut self.windows.win_a_open));
                    // 收起状态是**应用自己的 bool**：菜单能收起窗口，标题栏按钮也能
                    // （`shrink(show, &mut bool)` 里 `show = false` 正是给这种用法留的）。
                    m.item_checked("窗口 A 收起", &mut self.windows.win_a_collapsed);
                    // **点击行为 flag**：这一项点完**保留 popup**（连续点几次都行）。
                    if m.item(
                        Item::new(&format!("不收起（已点 {} 次）", self.mb_keep_clicks))
                            .click_behavior(MenuClick::Keep),
                    ) {
                        self.mb_keep_clicks += 1;
                    }
                    m.separator();
                    m.caption("密度");
                    m.row(|r| {
                        let mut di = self.theme_tuner.density as usize;
                        r.add(Segmented::new("mb_density", &["紧凑", "标准", "宽松"], &mut di));
                        if di != self.theme_tuner.density as usize {
                            self.theme_tuner.set_density(
                                [Density::Compact, Density::Cozy, Density::Spacious][di.min(2)],
                            );
                        }
                    });
                });
                bar.menu("帮助", |m| {
                    m.caption("操作提示");
                    m.separator();
                    m.label("拖动 = 移动窗口 · 点击 = 置顶");
                    m.label("Tab 遍历焦点 · Enter / Space 激活");
                    m.label("Esc 先关菜单，再按才退出");
                });
                // **竖分割线 + 非菜单控件**：栏本质就是一行 —— 菜单组与"状态"之间用竖线分开，
                // 后面再放一个标签。样式走 `Theme::menubar` 的分割线令牌（更短更淡），
                // 点它们**不会**关菜单（见 `menu_bar_should_close` 的 `on_bar`）。
                bar.separator_v();
                bar.label("状态：就绪");
            });
            debug_assert!(menu_size.x > 0.0, "菜单栏至少有宽度");

            // ── **按钮下拉菜单**（`UiAdd::add(Dropdown::…)`：与菜单栏**同一套浮层实现**）──
            // 位置固定（常量 `DROPDOWN_*_POS`）⇒ `--sim-dropdown` 能按同一常量 + 主题尺寸
            // 解算点击点。① = 选项列表模式（图一的形态）；② = 富内容模式（图二 + 子菜单）。
            ui.add_at(
                DROPDOWN_OPT_POS,
                Dropdown::options(
                    "dd_opt",
                    DIFF_TOP[self.dd_opt_idx.min(2) as usize],
                    &mut self.dd_opt_idx,
                    &DIFF_TOP,
                )
                .width(DROPDOWN_W),
            );
            ui.add_at(
                DROPDOWN_FILE_POS,
                Dropdown::new("dd_file", "文件名过滤")
                    .width(DROPDOWN_W)
                    .menu(|m| {
                        // ↓ 这几行就是「**菜单内又可以 `UiAdd::add`**」：
                        //   文本输入（普通控件）/ **子菜单** / 责任链菜单项 / 分割线。
                        m.text_input("dd_filter", &mut self.menu_filter);
                        // **Submenu**（新菜单项控件）：普通项样式 + 右侧 ▸，
                        // **Hover** 时在原 popup **右侧**展开；点击它**不收起**父 popup。
                        m.item(Item::new("编码").submenu(|s| {
                            for (i, enc) in ENCODINGS.iter().enumerate() {
                                let mut checked = i == self.dd_sub_idx.unwrap_or(0);
                                if s.item(Item::new(enc).checked(&mut checked)) {
                                    self.dd_sub_idx = Some(i);
                                }
                            }
                        }));
                        // 点击行为 flag：点完**保留** popup（"点了还要继续操作"的项）。
                        if m.item(
                            Item::new(&format!("保持打开（已点 {} 次）", self.dd_keep_clicks))
                                .click_behavior(MenuClick::Keep),
                        ) {
                            self.dd_keep_clicks += 1;
                        }
                        m.separator();
                        if m.item("导入图片…") {
                            self.dd_item_clicks += 1;
                            self.top.import_request = Some(ImportKind::Image);
                        }
                        if m.item("导入字体…") {
                            self.dd_item_clicks += 1;
                            self.top.import_request = Some(ImportKind::Font);
                        }
                    }),
            );
            // `--sim-dropdown`：坐标解算（**本帧录制后**已知面板在哪；注入只能经 `Frame`
            // 且在段之前 ⇒ 这里只算、下一帧注，与 `--sim-menu` 同一套做法）。
            if self.sim_dropdown {
                let dump = ui.debug_dump();
                let t = ui.theme().clone();
                let scale = ui.scale();
                let (fs, pad, fam) = (t.button.font_size, t.button.padding, t.button.font_family.clone());
                // 触发器 = 常量位置 + `DROPDOWN_W`（`.width(..)` 是逻辑单位）× DPI，
                // 高 = 上下内边距 + 一行文字（与 `Dropdown::trigger_size` 同源）。
                let h = pad.y * 2.0 + ui.text_size("文件名过滤", fs, fam.as_deref()).y;
                let w = DROPDOWN_W * scale;
                let trigger = |p: Vec2| Rect::new(p.x * scale, p.y * scale, w, h);
                let (opt_t, file_t) = (trigger(DROPDOWN_OPT_POS), trigger(DROPDOWN_FILE_POS));
                self.dd_opt_pt = Some(Vec2::new(opt_t.x + w * 0.5, opt_t.y + h * 0.5));
                self.dd_file_pt = Some(Vec2::new(file_t.x + w * 0.5, file_t.y + h * 0.5));
                // 面板**应该**在的原点：公开助手 `popup_origin`（与引擎同一函数）——
                // 判定时拿它和 `debug_dump` 里的真实原点比，顺带守住"同源"。
                self.dd_opt_want = Some(popup_origin(opt_t, PopupSide::Below));
                self.dd_file_want = Some(popup_origin(file_t, PopupSide::Below));
                // 面板内第一行的原点 = 面板原点 + 内边距 + 边框（公开助手，与引擎同源）。
                let inset = popup_padding(&t) + t.panel.border_w;
                let ih = item_h(fs);
                let menu_gap = popup_gap(scale);
                if let Some(p) = dump.windows.iter().find(|p| p.id == "dd_opt::popup") {
                    self.dd_opt_panel = Some(p.origin);
                    self.dd_opt_row0_pt = Some(Vec2::new(
                        p.origin.x + inset + 20.0,
                        p.origin.y + inset + ih * 0.5,
                    ));
                }
                if let Some(p) = dump.windows.iter().find(|p| p.id == "dd_file::popup") {
                    self.dd_file_panel = Some(p.origin);
                    // 富内容菜单的**行序与示例录制顺序同源**：
                    // ① 文本输入 ② **子菜单「编码」** ③ 「保持打开」(Keep) ④ 分割线 ⑤⑥ 两个导入项
                    let x = p.origin.x + inset + 20.0;
                    let y0 = p.origin.y + inset;
                    self.dd_input_pt = Some(Vec2::new(x, y0 + t.input.height * 0.5));
                    // ② 子菜单行（Hover 展开的目标）
                    let y_sub = y0 + t.input.height + menu_gap;
                    self.dd_sub_row_pt = Some(Vec2::new(x, y_sub + ih * 0.5));
                    // 子面板**应该**在的原点 = `popup_origin(行矩形, Right)`（公开助手）。
                    let row_rect = Rect::new(
                        p.origin.x + inset,
                        y_sub,
                        (p.size.x - inset * 2.0).max(0.0),
                        ih,
                    );
                    self.dd_sub_want = Some(popup_origin(row_rect, PopupSide::Right));
                    // ③ Keep 项
                    self.dd_keep_row_pt =
                        Some(Vec2::new(x, y_sub + ih + menu_gap + ih * 0.5));
                    // ④ 分割线 → ⑤ 第 1 个导入项（行距 = `popup_gap`，与引擎同源）
                    let sep_h = t.divider.thickness + t.divider.margin * 2.0;
                    self.dd_item_pt = Some(Vec2::new(
                        x,
                        y_sub + ih + menu_gap + ih + menu_gap + sep_h + menu_gap + ih * 0.5,
                    ));
                    // 子菜单面板（**嵌套窗口**：`origin` 相对**父窗口** ⇒ 必须叠加父面板原点）；
                    // 它的第 2 行（"UTF-8"）是仿真点击目标。
                    if let Some(q) = dump
                        .windows
                        .iter()
                        .find(|q| q.id.ends_with("item::编码::sub"))
                    {
                        let abs = p.origin + q.origin;
                        let m_inset = popup_padding(&t) + t.panel.border_w;
                        self.dd_sub_panel = Some(abs);
                        self.dd_sub_item1_pt =
                            Some(Vec2::new(abs.x + m_inset + 20.0, abs.y + m_inset + ih + menu_gap + ih * 0.5));
                    }
                }
            }
            // `--sim-ta-resize`：grip 点解算 —— 窗口原点 + 编辑器**窗口局部**矩形的右下角
            // − 半个柄（柄命中区 = 14×14 贴右下角 ⇒ 中心 = 右下角 − 7）。全部由几何得来，
            // 编辑器自己长大 / 行高变化都不用改脚本。
            if self.sim_ta_resize {
                let dump = ui.debug_dump();
                let win = dump.windows.iter().find(|p| p.id == "ta_win");
                if let (Some(win), Some(local)) = (win, self.sim_ta_rect) {
                    // ⚠ `Response::rect` 是**行局部**坐标（控件录在它所在的容器里），而窗口内容
                    // 原点又比窗口外框多一个 `pad`（= `panel.padding + border_w`，主题已预乘
                    // DPI ⇒ 这里只 × scale，不再叠加任何逻辑值）。
                    let pad = (ui.theme().panel.padding + ui.theme().panel.border_w) * ui.scale();
                    self.sim_ta_grip = Some(
                        Vec2::new(win.origin.x, win.origin.y) + Vec2::splat(pad) + local.max()
                            - Vec2::splat(7.0),
                    );
                    let editor = ui
                        .state()
                        .sizes
                        .get("ta_edit")
                        .copied()
                        .unwrap_or_else(|| local.size());
                    if self.sim_ta_before.is_none() {
                        self.sim_ta_before = Some((editor, win.size.y));
                    }
                    self.sim_ta_now = Some((editor, win.size.y));
                    if self.sim_ta_min_want.is_none() {
                        // 默认下限 = `(min_w, height)` —— 主题里的值**已经是物理像素**，
                        // 不能再 × scale（那正是单位纪律第 2 条要防的错）。
                        self.sim_ta_min_want =
                            Some(Vec2::new(ui.theme().input.min_w, ui.theme().input.height));
                        self.sim_ta_scale = ui.scale();
                        self.sim_ta_std_row = ui.theme().row_h;
                    }
                }
            }
            // `--sim-resize`：坐标解算（窗口原点 + 结算尺寸 = 右下角；柄是那个角上的方块）。
            if self.sim_resize {
                let dump = ui.debug_dump();
                let w = dump.windows.iter().find(|p| p.id == "img_box_fill");
                if let Some(w) = w {
                    self.sim_resize_pt =
                        Some(Vec2::new(w.origin.x + w.size.x - 8.0, w.origin.y + w.size.y - 8.0));
                    self.sim_resize_after = Some(w.size);
                    if sim_frame == 8 {
                        self.sim_resize_size = Some(w.size);
                        // 目标点也在这里冻结（+60/+40 ⇒ 期望尺寸变化同样只该是这个量级）。
                        self.sim_resize_to = Some(Vec2::new(
                            w.origin.x + w.size.x - 8.0 + 60.0,
                            w.origin.y + w.size.y - 8.0 + 40.0,
                        ));
                    }
                }
            }
            // `--sim-chrome`：按钮中心解算（**本帧录制后**从 `win_a` 的**外框矩形**推导）。
            // 引擎侧按钮右缘 = **外框右缘 − `TITLE_BUTTON_INSET`**（0）⇒ "点最外缘那个按钮的
            // 中心必须命中 ✕ / ⌃"；点不到就说明按钮没贴右缘（这条脚本 = "贴外缘"的回归守卫）。
            // ⚠ 旧版这里用 `220*scale + pad`（**内容**右缘）算 —— 那套坐标在按钮右移到外框
            // 右缘之后就点不准了，所以脚本必须跟着"外框"口径走，而不是继续写死数字。
            if self.sim_chrome {
                // 阶段 6 的前置：**确定地展开**（见注入段注释）。
                if sim_frame == 184 {
                    ui.state_mut().set_collapsed(ENG_COLLAPSE_WIN, false);
                }
                let dump = ui.debug_dump();
                let (row_h, gap) = (ui.theme().row_h, ui.theme().gap);
                // 两个窗口的按钮中心都用**同一个口径**（引擎的 `title_bar_layout`）：
                // 右缘 = 外框右缘、`btn = row_h − 2`、顺序 `[⌃][✕]`、点按钮**上半部**
                // （下半会被收起态右下角的 resize 柄抢走，见 `docs/DEBUGGING.md`）。
                // ⚠ `has_close`：win_a 有 ×（⌃ 在它左边），`img_box_fill` 只有 ⌃（自己占最右格）。
                self.windows.chrome_pts = caption_pts(&dump, "win_a", row_h, gap, true);
                self.windows.eng_pts = caption_pts(&dump, ENG_COLLAPSE_WIN, row_h, gap, false)
                    .map(|(fold, _)| fold);
                // 阶段 5 的点 = 同一个 ⌃ 的**行中心**（`caption_pts` 给的是 `row_h*0.25`，
                // 即行上半；中心还要再往下 `row_h*0.25`）。
                self.windows.eng_center_pt = self
                    .windows
                    .eng_pts
                    .map(|p| Vec2::new(p.x, p.y + row_h * 0.25));
                // **引擎托管收起**的证据：每帧记 `(UiState 里的收起状态, dump 尺寸)`
                // —— 状态与几何必须**同步**变（只记变化点，避免几百条重复）。
                if let Some(w) = dump.windows.iter().find(|p| p.id == ENG_COLLAPSE_WIN) {
                    self.windows.eng_rect = Some((w.origin, w.size));                    let now = (ui.state().is_collapsed(ENG_COLLAPSE_WIN), w.size);
                    if self.windows.eng_states.last() != Some(&now) {
                        self.windows.eng_states.push(now);
                    }
                    if sim_frame == 207 {
                        self.windows.eng_dragged_h = Some(w.size.y);
                    }
                    if sim_frame == 220 {
                        self.windows.eng_collapsed_h = Some(w.size.y);
                    }
                }
            }
            // `--sim-resize` 阶段 2：`grip_win`（没有 `.width()`）的柄中心 + 拖前后尺寸。
            // 位置从 dump 解算（窗口首帧还没被 clamp ⇒ 只信 frame ≥ 2 的值）。
            if self.sim_resize && sim_frame >= 2 {
                let dump = ui.debug_dump();
                if let Some(w) = dump.windows.iter().find(|p| p.id == "grip_win") {
                    self.windows.grip_pt =
                        Some(Vec2::new(w.origin.x + w.size.x - 8.0, w.origin.y + w.size.y - 8.0));
                    if sim_frame == 60 {
                        self.windows.grip_before = Some(w.size);
                        self.windows.grip_after = Some(w.size);
                        // 目标点**冻结**（见 `grip_to` 的说明）。
                        if let Some(p) = self.windows.grip_pt {
                            self.windows.grip_to = Some(Vec2::new(p.x + 50.0, p.y));
                        }
                    } else if sim_frame > 60 {
                        self.windows.grip_after = Some(w.size);
                    }
                }
            }
            // `--sim-resize` 阶段 3：收缩高度之后，高度必须是**恒定值**（松手当帧与
            // 若干帧之后相同）——"窗口收缩高度应当为恒定值"（用户实测）。
            if self.sim_resize && sim_frame >= 86 {
                let dump = ui.debug_dump();
                if let Some(w) = dump.windows.iter().find(|p| p.id == "img_box_fill") {
                    if sim_frame == 90
                        && let Some(p) = self.sim_resize_pt
                    {
                        // 阶段 3 的**冻结**收缩目标（只上移 40）：柄会随高度一起上移，
                        // 每帧重算目标 = 鼠标追着柄跑 ⇒ 高度一档一档掉到下限。
                        self.windows.resize_up = Some(Vec2::new(p.x, p.y - 40.0));
                    }
                    if sim_frame == 106 {
                        self.windows.shrink_after_free = Some(w.size.y);
                    } else if sim_frame >= 140 {
                        self.windows.shrink_later = Some(w.size.y);
                    }
                }
            }
            // `--sim-scroll-mode`：`vscroll(Scroll)` 那扇窗的**滚动条翻页点**（每帧重算：
            // 首帧窗口还没被 clamp 住，只信 frame ≥ 2）。点条带**靠下**那一截（thumb 贴顶
            // ⇒ 那里是轨道）⇒ 点一下翻一页，`offset` 必然变大。
            if self.sim_scroll_mode && sim_frame >= 2 && self.scroll_thumb_pt.is_none() {
                let dump = ui.debug_dump();
                if let Some(w) = dump.windows.iter().find(|p| p.id == "scroll_v_win") {
                    let pad = ui.theme().panel.padding + ui.theme().panel.border_w;
                    self.scroll_thumb_pt = Some(Vec2::new(
                        // 条带 = 可视区右缘约 12px 宽 ⇒ 取右缘 −6。
                        w.origin.x + w.size.x - pad - 6.0,
                        // 靠下 10px：必定落在轨道（而不是顶部的 thumb）。
                        w.origin.y + w.size.y - pad - 10.0,
                    ));
                }
            }
            // `--sim-scroll-mode`：读两扇窗的**实测尺寸 + 裁剪层**（判定：按轴策略各自生效）。
            if self.windows.show_scroll_wins && sim_frame == 40 {                let dump = ui.debug_dump();
                let find = |id: &str| dump.windows.iter().find(|p| p.id == id);
                let (a, b) = (find("scroll_noclip_win"), find("scroll_clip_win"));
                let pad = (ui.theme().panel.padding + ui.theme().panel.border_w) * 2.0;
                let want_clip = 150.0 * ui.scale() + pad;
                let screen_h = ui.window_physical_size().1 as f32;
                let (sa, sb) = (a.map(|w| w.size), b.map(|w| w.size));
                let (ca, cb) = (a.and_then(|w| w.clip), b.and_then(|w| w.clip));
                // ① NoClip 的水平轴：`.width(150)` 只是下限 ⇒ 内容（400 逻辑 = 600 物理）
                //    把窗口撑到 ~626；
                let noclip_ok = sa.is_some_and(|s| s.x >= 400.0);
                // ② ClipOnly：固定视口宽 = 150×scale + 内边距；
                let clip_ok = sb.is_some_and(|s| (s.x - want_clip).abs() <= 3.0);
                // ③ **两条轴各自独立**（这条是旧的 `Placement::Clip` 做不到的）：NoClip 窗
                //    **完全没有裁剪层**；ClipOnly 窗的裁剪层 x = 窗口宽（横向被裁）、
                //    y 仍铺满屏幕（**纵向没被裁**）。
                let no_clip_layer = a.is_some_and(|w| w.clip.is_none());
                let clip_x_only = cb
                    .zip(sb)
                    .is_some_and(|(c, s)| (c.w - s.x).abs() <= 1.0 && c.h >= screen_h - 1.0);
                // ⚠ 不能用"NoClip 窗的 `clip` 是 `None`"当判据：那个字段是**该窗最后一批的
                // scissor**，而窗里的 `TextEditor` 自带**文本框盒裁剪** ⇒ 恒为 `Some`。
                // 所以 NoClip 那条轴用"窗口宽被内容撑过 `.width()`"来证（626 > 225），
                // ClipOnly 那条轴用"scissor 宽 = 窗口宽 且 纵向铺满屏幕"来证（只裁横向）。
                // ④ `vscroll(Scroll)` 那扇窗：视口高 = 屏幕剩下的高（远小于内容）⇒
                //    `ScrollState.content_h > 视口高`；拖一下右侧滚动条 ⇒ `offset > 0`。
                let sv = ui.state().scrolls().get("scroll_v_win/scroll").copied();
                let view_h = find("scroll_v_win")
                    .map(|w| w.size.y)
                    .unwrap_or(0.0);
                // 视口建成（内容高于视口）+ 点条带**真的翻了一页**（`offset` 变大）。
                let ok = sv.is_some_and(|s| s.content_h > view_h + 1.0 && s.offset > 1.0);
                eprintln!(
                    "sim-scroll-mode[vscroll(Scroll)]: 视口高={view_h:.0} · ScrollState={sv:?} {}",
                    if ok {
                        "[OK] 窗口内的滚动视口建成 + 点条带翻页改了 offset"
                    } else {
                        "[FAIL] 视口没建 / 条带没点到 / offset 没写回"
                    }
                );
                let _ = no_clip_layer;
                let _ = ca;
                eprintln!(
                    "sim-scroll-mode: NoClip 窗={sa:?}（期望宽 ≥ 400）· ClipOnly 窗={sb:?}（期望宽 {want_clip:.0}）\
                     · NoClip scissor={ca:?} · ClipOnly scissor={cb:?}（期望宽=窗口宽、高≥{screen_h:.0}）{}",
                    if noclip_ok && clip_ok && clip_x_only {
                        "[OK] 按轴策略各自生效：NoClip 让内容定宽、ClipOnly 固定视口宽且只裁横向（纵向未被牵连）"
                    } else {
                        "[FAIL] 某条轴没按策略走（NoClip 被压窄 · ClipOnly 被撑开 · 纵向被牵连）"
                    }
                );
            }
            // `--sim-row-overflow`：① 两个点击点（窗口内 / 窗口外）从 `row_win` 的**实测
            // 原点** + 主题内边距解算——不写死像素；② 两次点击后各看一眼**焦点**落点。
            if self.windows.sim_row_overflow {
                // ⚠ **每帧重算**（只在脚本还会用到点击点的那几帧）：首帧窗口还没被
                // `WindowClamp::Screen` 夹住（引擎要上一帧尺寸才能 clamp）⇒ 只算一次会
                // 把点定在窗口"将要被夹走"之前的位置（实测踩过：窗外那个点偏了 44px）。
                if sim_frame < 38 {
                    let dump = ui.debug_dump();
                    if let Some(w) = dump.windows.iter().find(|p| p.id == "row_win")
                        && w.size.x > 0.0
                    {
                        let pad = ui.theme().panel.padding + ui.theme().panel.border_w;
                        let origin = w.origin + Vec2::splat(pad);
                        self.windows.row_inside_pt = Some(
                            origin
                                + ROW_INSIDE_OFFSET
                                + Vec2::new(ROW_INSIDE_W * 0.5, ROW_TE_H * 0.5),
                        );
                        // 幽灵控件宽固定 120（见 `TextEditor::width(120.0)`）。
                        self.windows.row_ghost_pt =
                            Some(origin + ROW_GHOST_OFFSET + Vec2::new(60.0, ROW_TE_H * 0.5));
                    }
                }
                let focused = ui
                    .state()
                    .focused
                    .as_ref()
                    .map(|f| f.as_str().to_owned());
                if sim_frame == 28 {
                    self.windows.row_inside_focus =
                        focused.as_ref().is_some_and(|f| f.contains("inside_te"));
                }
                if sim_frame == 40 {
                    self.windows.row_ghost_focus =
                        focused.as_ref().is_some_and(|f| f.contains("ghost_te"));
                    eprintln!(
                        "sim-row-overflow: 点窗外后 focus={focused:?} occluded_hits={} 行宽={:?} 可用宽={:.0}",
                        ui.state().hits().occluded_hits(),
                        self.windows.row_size,
                        self.windows.row_avail_w,
                    );
                }
            }
            // `--sim-zorder`：列表里"点得到"的点必须**从几何解算**——
            // 列表下方邻接的 `win_b` 会随内容长高（`TextEditor` 默认尺寸按 DPI 修正后
            // 就长高了一截），写死的 y 会被它盖住 ⇒ 点下去命中 `win_b` 的文本框：
            // 行点击选不中、条带点击翻不了页。取"`win_b` 下缘 + 8"夹进列表可视区；
            // x 分别取列表内左缘 + 10（行）与右缘 − 7（滚动条条带中心）。
            if self.sim_zorder {
                let dump = ui.debug_dump();
                let scale = ui.scale();
                let top = LIST_POS.y * scale;
                let bottom = (LIST_POS.y + LIST_SIZE.y) * scale;
                let win_b_bottom = dump
                    .windows
                    .iter()
                    .find(|w| w.id == "win_b")
                    .map(|w| w.origin.y + w.size.y);
                let y = win_b_bottom
                    .map_or(top + 8.0, |b| (b + 8.0).max(top + 8.0))
                    .min(bottom - 8.0);
                self.sim_zorder_pts = Some((
                    Vec2::new((LIST_POS.x + 10.0) * scale, y),
                    Vec2::new((LIST_POS.x + LIST_SIZE.x) * scale - 7.0, y),
                ));
            }
            // `--sim-menu`：坐标解算（**本帧录制后**已知栏在哪、下拉面板在哪）——
            // 注入只能经 `Frame` 且在段之前，所以这里只算、段外下一帧注（同 `--sim-tuner`）。
            if self.sim_menu {
                let dump = ui.debug_dump();
                // 触发器几何 = `Theme::menubar` 的令牌（字号 / 左右内边距 / 行列间距 / 栏内边距）
                // —— 与引擎同源，改主题不用改脚本。
                let (fs, pad_x, row_h, gap, mb_pad) = {
                    let m = &ui.theme().menubar;
                    (m.font_size, m.trigger_pad_x, ui.theme().row_h, m.gap, m.padding)
                };
                // 触发器：「视图」是第 2 个（三个都是两个字 ⇒ 等宽）；栏位置取常量
                // `MENUBAR_POS`（逻辑 → 物理，与录制同源）——挪栏不用改脚本。
                // 栏内容 = `[内边距][文件][间距][视图]…` ⇒ 视图 x = 栏 + 内边距 + 触-发器宽 + 间距。
                let bar = (MENUBAR_POS * scale).round();
                let tw = ui.text_size("视图", fs, None).x;
                let w = tw + pad_x * 2.0;
                let trigger_rect = Rect::new(bar.x + mb_pad + w + gap, bar.y + mb_pad, w, row_h);
                self.menu_trigger_pt =
                    Some(Vec2::new(trigger_rect.x + w * 0.5, trigger_rect.y + row_h * 0.5));
                // 下拉原点（引擎里的 `pos = (t.x, t.y + h + 2)`，见 `MenuBar::popup`）。
                self.menu_want_origin =
                    Some(Vec2::new(trigger_rect.x, trigger_rect.y + row_h + 2.0));
                // 阶段 3 用：**栏内空白**（栏覆盖整屏 ⇒ 右侧远处必在栏矩形内，且那里
                // 没有任何触发器 / 控件）——验"点栏内空白不收起菜单"。
                self.menu_bar_blank_pt = Some(Vec2::new(
                    bar.x + ui.window_physical_size().0 as f32 * 0.6,
                    bar.y + mb_pad + row_h * 0.5,
                ));
                // 下拉里**第一个菜单项**：注意面板左内边距 = `item_pad_x`（勾选方框画在
                // 菜单项**内容里**，不占内边距）+ 边框 ⇒ 第一项从内容原点起，不是面板顶边。
                let item_h = (fs * 1.3).round() + 2.0;
                let top_pad = ui.theme().combo.item_pad_x + ui.theme().panel.border_w;
                if let Some(p) = dump.windows.iter().find(|p| p.id == "menubar::视图") {
                    // ⚠ **下拉面板是嵌套窗口**（菜单栏现在是"一行容器"，面板录在栏里）：
                    // dump 的 `origin` 相对**直接容器** ⇒ 必须叠加栏原点（`MENUBAR_POS`）
                    // 才是屏幕坐标。不叠加就会点到栏外 ⇒ 菜单当场收起（实测症状：
                    // "菜单没开 / 菜单项没执行"）。
                    let abs = bar + p.origin;
                    self.menu_item_pt = Some(Vec2::new(
                        abs.x + top_pad + 20.0,
                        abs.y + top_pad + item_h * 0.5,
                    ));
                    self.menu_panel = Some((abs, p.size));
                } else {
                    self.menu_panel = None;
                }
            }
            // `--sim-menu` 判定（在菜单栏录制**之后**读状态：本帧的展开 / 收起已定）。
            // ① 点「视图」触发器 → 菜单打开（`UiState::menu_open` = 该触发器绝对 ID）；
            // ② 点下拉第一个菜单项（"主题调节窗口"）→ 勾选翻转（窗口关掉）+ 菜单自动收起。
            if self.sim_menu && sim_frame == 30 {
                let open = ui.state().menu_open().map(|s| s.to_owned());
                let tuner_open = self.theme_tuner.open;
                let ok = open.is_none() && !tuner_open;
                eprintln!(
                    "sim-menu: menu_open={open:?} · 主题调节窗口={} {}",
                    if tuner_open { "开" } else { "关" },
                    if ok {
                        "[OK] 点触发器开菜单 + 点菜单项执行并自动收起"
                    } else {
                        "[FAIL] 菜单没开 / 菜单项没执行 / 没自动收起"
                    }
                );
            }
            // 阶段 2 判定：**菜单面板不可拖动**（拖面板 = 拖窗口会把它从触发器上拖走，
            // 命中判定按窗口走、视觉却跑别处 ⇒"控件严重错位"）。
            // 期望原点由**主题尺寸**算出（触发器左下 + 2px），与 `MenuBar::popup` 同源。
            if self.sim_menu && sim_frame == 58 {
                let open = ui.state().menu_open().is_some();
                let dump = ui.debug_dump();
                // ⚠ 面板是**嵌套窗口**（录在"一行容器"里）⇒ dump 的 `origin` 相对栏原点，
                // 屏幕坐标要叠加 `MENUBAR_POS`（与 `menu_want_origin` 同一坐标系才可比）。
                let bar = (MENUBAR_POS * scale).round();
                let got = dump
                    .windows
                    .iter()
                    .find(|p| p.id == "menubar::视图")
                    .map(|p| bar + p.origin);
                let stayed = match (got, self.menu_want_origin) {
                    (Some(g), Some(w)) => (g.x - w.x).abs() < 2.0 && (g.y - w.y).abs() < 2.0,
                    _ => false,
                };
                eprintln!(
                    "sim-menu: 面板原点={got:?} · 期望={:?} · 菜单仍开={open} · 拖拽后没跑位={stayed} {}",
                    self.menu_want_origin,
                    if open && stayed {
                        "[OK] 菜单面板不会被拖动"
                    } else {
                        "[FAIL] 面板被拖走了 / 菜单意外关闭"
                    }
                );
            }

            // 阶段 3 判定：**点栏内空白不收起菜单**（"栏 = 一行容器"的 `on_bar` 语义；
            // 旧实现只认"按下落在某个触发器上" ⇒ 点栏里的空白 / 竖分割线 / 别的控件都会误关）。
            if self.sim_menu && sim_frame == 72 {
                let open = ui.state().menu_open().is_some();
                eprintln!(
                    "sim-menu: 点栏内空白后 menu_open={open} {}",
                    if open {
                        "[OK] 点栏内空白不收起菜单"
                    } else {
                        "[FAIL] 点栏内空白把菜单关了"
                    }
                );
            }

            // `--sim-dropdown` 判定（**下拉录制之后**读状态：本帧的展开 / 收起已定）。
            // ① 点触发器 ⇒ 下拉打开（`combo_open` = 面板 id；面板真在 dump 里、且**正好在
            //    触发器下方 2px** —— 用**公开**助手 `popup_origin` 算期望值，顺带守住"同源"）；
            // ② 点选项 ⇒ 选中索引变 + 自动收起 + 面板消失；
            // ③ 富内容下拉同样开得起来（**同一个控件、同一套浮层实现**）；
            // ④ 菜单里的**文本输入**真的可聚焦（"菜单内又是 `UiAdd`"的实证）；
            // ⑤ 菜单项点了 ⇒ 执行 + 自动收起。
            if self.sim_dropdown {
                let open = ui.state().combo_open().map(|s| s.to_owned());
                let dump = ui.debug_dump();
                let at = |id: &str| dump.windows.iter().find(|p| p.id == id).map(|p| p.origin);
                // 嵌套浮层的 id 带父窗口前缀（`<父面板>/item::编码::sub`）⇒ 按**后缀**匹配。
                let seen_suffix =
                    |suffix: &str| dump.windows.iter().any(|p| p.id.ends_with(suffix));
                let z_of = |id: &str| dump.windows.iter().find(|p| p.id == id).map(|p| p.z);
                let z_suffix =
                    |suffix: &str| dump.windows.iter().find(|p| p.id.ends_with(suffix)).map(|p| p.z);
                let near = |got: Option<Vec2>, want: Option<Vec2>| match (got, want) {
                    (Some(g), Some(w)) => (g.x - w.x).abs() < 2.0 && (g.y - w.y).abs() < 2.0,
                    _ => false,
                };
                match sim_frame {
                    20 => {
                        let (got, want) = (at("dd_opt::popup"), self.dd_opt_want);
                        // ⚠ `combo_open` 记的是**控件（触发器）的绝对 ID**（与 `menu_open`
                        // 记触发器一致）；面板窗口 id 是它加 `::popup` 后缀。
                        let ok = open.as_deref() == Some("dd_opt") && near(got, want);
                        eprintln!(
                            "sim-dropdown: ① combo_open={open:?}（期望 dd_opt）面板原点={got:?} 期望={want:?} {}",
                            if ok {
                                "[OK] 点触发器开下拉（面板在触发器正下方）"
                            } else {
                                "[FAIL] 下拉没开 / 面板跑位"
                            }
                        );
                    }
                    32 => {
                        let idx = self.dd_opt_idx;
                        let gone = at("dd_opt::popup").is_none();
                        let ok = idx == 0 && open.is_none() && gone;
                        eprintln!(
                            "sim-dropdown: ② 选中索引={idx}（期望 0）combo_open={open:?} 面板消失={gone} {}",
                            if ok {
                                "[OK] 点选项 ⇒ 选中 + 自动收起"
                            } else {
                                "[FAIL] 选项没选中 / 没收起"
                            }
                        );
                    }
                    50 => {
                        let (got, want) = (at("dd_file::popup"), self.dd_file_want);
                        let ok = open.as_deref() == Some("dd_file") && near(got, want);
                        eprintln!(
                            "sim-dropdown: ③ 富内容下拉 combo_open={open:?}（期望 dd_file）面板原点={got:?} 期望={want:?} {}",
                            if ok {
                                "[OK] 同一个控件也能开富内容菜单"
                            } else {
                                "[FAIL] 富内容下拉没开 / 面板跑位"
                            }
                        );
                    }
                    58 => {
                        let tf = ui.state().text_focus().map(|f| f.id.as_str().to_owned());
                        let ok = tf
                            .as_deref()
                            .is_some_and(|f| f.contains("dd_file::popup"));
                        eprintln!(
                            "sim-dropdown: ④ text_focus={tf:?} {}",
                            if ok {
                                "[OK] 菜单里的文本输入真的可聚焦（菜单内又是 UiAdd）"
                            } else {
                                "[FAIL] 菜单里的文本输入拿不到焦点"
                            }
                        );
                    }
                    72 => {
                        // ⑤ **只悬停**子菜单行 ⇒ 子面板在**行右侧**展开，且**父 popup 仍在**
                        // （曾经的做法——菜单里嵌 `Dropdown`——会把 `combo_open` 覆盖掉，
                        // 症状是"点一下整条 popup 消失"）。
                        let (got, want) = (self.dd_sub_panel, self.dd_sub_want);
                        let sub_seen = seen_suffix("item::编码::sub");
                        // **分层 z**：子面板 z 必须 **大于** 父面板（浮层 z = 基址 + 嵌套层数）
                        // —— 同 z 会让两层命令落进同一个 `(win, elem)` 分组排序，子层的**阴影**
                        // （elem 0）被父层控件（elem ≥ 1）盖住（用户实测："下级 popup 阴影被绘制
                        // 在了上级控件后面"）。
                        let (zp, zs) = (z_of("dd_file::popup"), z_suffix("item::编码::sub"));
                        let layered = matches!((zp, zs), (Some(a), Some(b)) if b > a);
                        let ok = sub_seen
                            && near(got, want)
                            && layered
                            && open.as_deref() == Some("dd_file");
                        eprintln!(
                            "sim-dropdown: ⑤ 子面板原点={got:?} 期望={want:?} 面板在 dump={sub_seen} z(父/子)={zp:?}/{zs:?} 子层z更大={layered} combo_open={open:?} {}",
                            if ok {
                                "[OK] Hover 在 item 右边展开子菜单（分层 z：阴影不再被父层控件盖住），且父 popup 不消失"
                            } else {
                                "[FAIL] 子菜单没展开 / 位置不对 / z 没分层 / 父 popup 被关掉"
                            }
                        );
                    }
                    84 => {
                        // ⑥ 点子菜单里的项 ⇒ 应用状态变了 + **整条链**（子 + 父）收起。
                        let idx = self.dd_sub_idx;
                        let sub_gone = !seen_suffix("item::编码::sub");
                        let file_gone = at("dd_file::popup").is_none();
                        let ok = idx == Some(1) && open.is_none() && sub_gone && file_gone;
                        eprintln!(
                            "sim-dropdown: ⑥ 子菜单选中={idx:?}（期望 Some(1)=UTF-8）combo_open={open:?} 子面板消失={sub_gone} 父面板消失={file_gone} {}",
                            if ok {
                                "[OK] 点子菜单项 ⇒ 执行 + 整条链一起收起"
                            } else {
                                "[FAIL] 子菜单项没执行 / 没收起整条链"
                            }
                        );
                    }
                    112 => {
                        // ⑦ 点「保持打开」（`MenuClick::Keep`）⇒ 应用计数 + **popup 保留**。
                        let n = self.dd_keep_clicks;
                        let still = at("dd_file::popup").is_some();
                        let ok = n == 1 && open.as_deref() == Some("dd_file") && still;
                        eprintln!(
                            "sim-dropdown: ⑦ Keep 项点击次数={n}（期望 1）combo_open={open:?} 面板还在={still} {}",
                            if ok {
                                "[OK] 点\"保留 popup\"的项 ⇒ 执行 + 不收起"
                            } else {
                                "[FAIL] Keep 项没执行 / 被收起了"
                            }
                        );
                    }
                    _ => {}
                }
            }

            // 字体 Modal（**帧末录制**：modal 的 z 每帧重写为当前最大，最后录制才能保证
            // 不被本帧后录的窗口盖住——见 `modal_at` 文档）。
            self.top.show_font_modal(&mut ui);

            // `--sim-weight-modal`：坐标解算（**modal 录完之后**，窗口原点/尺寸已在 dump 里）。
            // 对话框内容行（垂直栈 + `Theme::gap`）：① 说明 label ② 字体名输入框
            // ③ **字重下拉触发器**；底部是「确定 / 取消」行（取消最右、确定在它左边一个 gap）。
            if self.sim_weight_modal {
                let dump = ui.debug_dump();
                let t = ui.theme().clone();
                let (fs, pad, fam) = (t.button.font_size, t.button.padding, t.button.font_family.clone());
                if let Some(p) = dump.windows.iter().find(|w| w.id == "font_modal") {
                    let inset = t.panel.padding + t.panel.border_w;
                    // ⚠ 说明 label 会在内容宽内**折行**（这里是两行）——必须按 `text_size_wrap`
                    // 量（与引擎同一套折行测量），否则少算一行、点到的就是下一行的输入框。
                    // 折行宽 = `Frame::fixed_avail_w` = 固定宽 − 2×pad_total；而窗口结算宽
                    // `p.size.x` = 固定宽 + 2×pad_total ⇒ 折行宽 = `p.size.x − 4×inset`。
                    let label_h = ui
                        .text_size_wrap(
                            "字体切换：输入字体名预览，确定生效（空 = 默认）",
                            t.label.font_size,
                            t.label.font_family.as_deref(),
                            (p.size.x - inset * 4.0).max(0.0),
                        )
                        .y;
                    // ③ 触发器：内容第 3 行（+5px 落在行内——触发器高约 2×padding.y+字号）。
                    self.wm_trigger_pt = Some(Vec2::new(
                        p.origin.x + inset + 10.0,
                        p.origin.y + inset + label_h + t.gap + t.input.height + t.gap + 5.0,
                    ));
                    // 底部按钮行：「取消」最右，「确定」在它左边一个 gap（宽度按文字实测）。
                    let ok_w = ui.text_size("确定", fs, fam.as_deref()).x + pad.x * 2.0;
                    let cancel_w = ui.text_size("取消", fs, fam.as_deref()).x + pad.x * 2.0;
                    let btn_h = pad.y * 2.0 + ui.text_size("确定", fs, fam.as_deref()).y;
                    self.wm_ok_pt = Some(Vec2::new(
                        p.origin.x + p.size.x - inset - cancel_w - t.gap - ok_w * 0.5,
                        p.origin.y + p.size.y - inset - btn_h * 0.5,
                    ));
                }
                // 下拉第 1 行（面板出现后才知道它在哪）：面板原点来自 dump，
                // 行内偏移用**公开**助手 `popup_padding` / `item_h`。
                if let Some(q) = dump
                    .windows
                    .iter()
                    .find(|w| w.id.ends_with("font_modal_weight::popup"))
                {
                    let m_inset = popup_padding(&t) + t.panel.border_w;
                    let ih = item_h(fs);
                    // ⚠ `UiWindowInfo::origin` 对**嵌套浮层**是**相对其直接容器**的原点
                    // （顶点管线"减去本窗口 origin、提交时再加回来"，所以那时才成立）——
                    // 对话框里的下拉必须**加上对话框原点**才是屏幕坐标（实测：dump 里
                    // 是 (14,162)，屏幕上是 (704,492)）。顶层窗口两者相同。
                    let modal_origin = dump
                        .windows
                        .iter()
                        .find(|w| w.id == "font_modal")
                        .map(|w| w.origin)
                        .unwrap_or(Vec2::ZERO);
                    let abs = modal_origin + q.origin;
                    self.wm_row0_pt =
                        Some(Vec2::new(abs.x + m_inset + 10.0, abs.y + m_inset + ih * 0.5));
                }
            }

            // `--sim-weight-modal` 判定（在 modal 录制**之后**读状态）。
            // ① 点下拉第 1 档（"细 300"）⇒ **草稿**必须已经变了（曾经失效 bug），
            //    而**已应用**字重不动（草稿语义：确定才提交）；
            // ② 点「确定」⇒ 草稿提交到已应用值，对话框关闭。
            if self.sim_weight_modal {
                match sim_frame {
                    30 => {
                        let draft = self.top.font_weight_draft;
                        let applied = self.top.font_weight;
                        let open = ui.state().combo_open().map(|s| s.to_owned());
                        // ⚠ 断言**第 1 档**（`FONT_WEIGHT_CHOICES[0]`）而不是写死 300：
                        // 档位表是应用可见的公共常量（现在九档 100…900），写死数值会让
                        // "加一档"这种改动把脚本变成假失败。
                        let first = FONT_WEIGHT_CHOICES[0];
                        let ok = draft == first && draft != applied && open.is_none();
                        eprintln!(
                            "sim-weight-modal: ① 草稿={}（期望 {} = 第 1 档）已应用={}（期望 {}）combo_open={open:?} {}",
                            draft.0,
                            first.0,
                            applied.0,
                            Weight::NORMAL.0,
                            if ok {
                                "[OK] 字重下拉真的选中了其他档位（草稿已改、尚未提交）"
                            } else {
                                "[FAIL] 下拉选了没生效（草稿没变 / 提前提交 / 没收起）"
                            }
                        );
                    }
                    50 => {
                        let applied = self.top.font_weight;
                        let still_open = self.top.font_modal_open;
                        let first = FONT_WEIGHT_CHOICES[0];
                        let ok = applied == first && !still_open;
                        eprintln!(
                            "sim-weight-modal: ② 已应用字重={}（期望 {}）对话框还开着={still_open} {}",
                            applied.0,
                            first.0,
                            if ok {
                                "[OK] 确定后字重提交到应用状态（主题随之重建）"
                            } else {
                                "[FAIL] 字重没提交 / 对话框没关"
                            }
                        );
                    }
                    _ => {}
                }
            }

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
                    // **分段按钮组**（`Segmented`）的目标：预设行的第 3 段（"legacy"）。
                    // 段宽 = 文字实测 + `button.padding.x × 2`（与 `Segmented::size` 同口径）。
                    let (btn_fs, pad_x) = {
                        let t = ui.theme();
                        (t.button.font_size, t.button.padding.x)
                    };
                    let (w_dark, w_light, w_legacy, label_preset) = (
                        ui.text_size("dark", btn_fs, None).x + pad_x * 2.0,
                        ui.text_size("light", btn_fs, None).x + pad_x * 2.0,
                        ui.text_size("legacy", btn_fs, None).x + pad_x * 2.0,
                        ui.text_size("预设:", font, None).x,
                    );
                    let preset_y = tw.origin.y + pad + font + gap + row * 0.5;
                    let preset_x = tw.origin.x + pad + label_preset + gap;
                    self.sim_seg_pt = Some(Vec2::new(
                        preset_x + w_dark + w_light + w_legacy * 0.5,
                        preset_y,
                    ));
                    if sim_frame == 10 {
                        eprintln!(
                            "sim-tuner: tuner origin={:?} size={:?} num_grip={num_grip:?} slider_c={slider_c:?} seg={:?}",
                            tw.origin, tw.size, self.sim_seg_pt
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
        // 帧内"只录一次"的守卫复位（本帧的 UI 段已全部录完）。
        self.top.modal_recorded = false;
        // --sim-tuner：两阶段判定（打印的是**主题里**的圆角 ⇒ 滑杆 / 数字条 → 主题
        // 这条线才是最终目的）。
        if self.sim_tuner && f.frames() == 24 {
            let r = self.theme_tuner.radius;
            self.tuner_probe = Some(r);
            // **精度断言用严格相等**：数字条内部按 `step` 的十进制位数回写（`step = 0.5`
            // ⇒ 1 位小数），所以 `8.0 + 20px × 0.5` 必须**正好**是 `18.0`——不是"约 18"。
            // 这条把"吸附 / 十进制取整"钉死：若把 `round_decimals` 去掉（只 `(v/step).round()*step`），
            // `0.5` 在二进制里可精确表示 ⇒ 仍然相等；若把 `f64` 数学退回 `f32`，
            // 在这个量级还看不出来（`step = 0.1` 才会）——所以另有 `numberinput` 单测守 f32 失准。
            let ok = r == 18.0 && (panel_radius.tl - r).abs() < f32::EPSILON;
            eprintln!(
                "sim-tuner: 拖数字条后 radius={r:?}（期望**精确** 18.0 = 8 + 20px × step 0.5）· 主题圆角 tl={:.1} {}",
                panel_radius.tl,
                if ok {
                    "[OK] 数字条能改值（且精确落在 step 格点上），并进了主题"
                } else {
                    "[FAIL] 数字条没改值（点空 / 手柄不响应）/ 没进主题 / 值没吸到格点"
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
        // --sim-tuner 阶段 3：**分段按钮组**（`Segmented`）——点第 3 段（"legacy"）应把
        // 预设切到 2（`ThemeTuner::preset`）。
        if self.sim_tuner && f.frames() == 66 {
            let p = self.theme_tuner.preset;
            eprintln!(
                "sim-tuner: 点预设第 3 段后 preset={p} {}",
                if p == 2 {
                    "[OK] 分段按钮组可点（拼在一起的那组）"
                } else {
                    "[FAIL] 分段没被点到 / 没写回选中"
                }
            );
        }
        // --sim-ta-resize 判定（读段内记下的现场：编辑器持久尺寸 / 窗口高 / 行高）。
        if self.sim_ta_resize {
            let n = f.frames();
            let now = self.sim_ta_now.unwrap_or((Vec2::ZERO, 0.0));
            if n == 60 {
                self.sim_ta_after = Some(now);
                let ok = self.sim_ta_before.is_some_and(|(e0, w0)| {
                    (now.0 - e0 - Vec2::new(120.0, 80.0)).length() <= 3.0
                        && ((now.1 - w0) - 80.0).abs() <= 3.0
                });
                eprintln!(
                    "sim-ta-resize: ① 拖柄后 编辑器 {:?} → {:?} · 窗口高 {:?} → {:?} {}",
                    self.sim_ta_before.map(|b| b.0),
                    now.0,
                    self.sim_ta_before.map(|b| b.1),
                    now.1,
                    if ok {
                        "[OK] 申请尺寸 = 绘制尺寸：编辑器 +120/+80，窗口高同步 +80（下面的控件跟着下去）"
                    } else {
                        "[FAIL] 拖大后窗口/后续控件没跟着长（申请尺寸没走尺寸责任链）"
                    }
                );
                // ③ 在**编辑器被拖高**的这一帧断言：行高必须 = 子项高（远大于标准行高）。
                //    旧语义（`force_h_all` 覆盖一切）会让行高恒等于 `Theme::row_h`（39 物理）
                //    ⇒ 这条才是有区分度的现场。
                let std_row = self.sim_ta_std_row;
                let ok_row = (self.sim_ta_row_h - now.0.y).abs() <= 1.5 && self.sim_ta_row_h > std_row;
                eprintln!(
                    "sim-ta-resize: ③ 行高={} · 编辑器高={}（标准行高 ≈{std_row}）{}",
                    self.sim_ta_row_h,
                    now.0.y,
                    if ok_row {
                        "[OK] 多行子项把行撑高了（行高 = 子项高，没被压成一行高）"
                    } else {
                        "[FAIL] 行高与多行子项高不一致（多行被压成一行高）"
                    }
                );
                let _ = &self.sim_ta_min;
            } else if n == 90 {
                self.sim_ta_min = Some(now);
                let min = self.sim_ta_min_want.unwrap_or(Vec2::ZERO);
                let ok_min = now.0.x >= min.x - 1.0 && now.0.y >= min.y - 1.0;
                eprintln!(
                    "sim-ta-resize: ② 往左上过拖后 编辑器 {:?}（默认下限 {min:?}）{}",
                    now.0,
                    if ok_min {
                        "[OK] 默认最小尺寸生效（拖不到 0）"
                    } else {
                        "[FAIL] 能被拖到默认下限以下 / 拖到 0"
                    }
                );
                let ok_row = (self.sim_ta_row_h - now.0.y).abs() <= 1.5;
                eprintln!(
                    "sim-ta-resize: ③b 行高={} · 编辑器高={}（缩到下限后仍同步）{}",
                    self.sim_ta_row_h,
                    now.0.y,
                    if ok_row {
                        "[OK] 行高仍 = 子项高"
                    } else {
                        "[FAIL] 行高与子项高不一致"
                    }
                );
                // ④ 单行子项被**钉到行的 `min_h`**：子项自己要 90 逻辑像素高（= 135 物理），
                //    行 `min_h(60)` ⇒ 行高必须恰为 60 逻辑像素（90 物理），不是 135。
                let want = 60.0 * self.sim_ta_scale;
                let ok_pin = (self.sim_ta_pin_row.y - want).abs() <= 1.5;
                eprintln!(
                    "sim-ta-resize: ④ 单行行的行高={}（期望 {want}）{}",
                    self.sim_ta_pin_row.y,
                    if ok_pin {
                        "[OK] 单行子项被钉到行的 min_h（哪怕它自己 .height(90)）"
                    } else {
                        "[FAIL] 单行子项没被钉到行高（或行高被子项撑开）"
                    }
                );
            }
        }
        // --sim-resize：**宽高同调**判定 —— 拖完后两个轴都必须变大（只变大一个 = 轴没接上）。
        if self.sim_resize && f.frames() == 30 {
            let before = self.sim_resize_size;
            let after = self.sim_resize_after;
            let (b, a) = (before.unwrap_or(Vec2::ZERO), after.unwrap_or(Vec2::ZERO));
            let ok = a.x > b.x + 1.0 && a.y > b.y + 1.0;
            eprintln!(
                "sim-resize: 拖柄前 {:.0}×{:.0} → 后 {:.0}×{:.0} {}",
                b.x,
                b.y,
                a.x,
                a.y,
                if ok {
                    "[OK] 宽高同调（两个轴都被拖大了）"
                } else {
                    "[FAIL] 只有一条轴生效 / 柄没点到"
                }
            );
        }
        // --sim-resize 阶段 2：**没有 `.width()` 的窗口也能拖宽**，且**只**宽变（高不变）。
        if self.sim_resize && f.frames() == 80 {
            let (b, a) = (
                self.windows.grip_before.unwrap_or(Vec2::ZERO),
                self.windows.grip_after.unwrap_or(Vec2::ZERO),
            );
            let wider = a.x > b.x + 30.0;
            let same_h = (a.y - b.y).abs() <= 1.0;
            eprintln!(
                "sim-resize: 无 width 窗口 拖前 {:.0}×{:.0} → 拖后 {:.0}×{:.0} {}",
                b.x,
                b.y,
                a.x,
                a.y,
                if wider && same_h {
                    "[OK] 没给 `.width()` 也能横向拖宽（且高度轴没被带上 —— Resize::Horizontal）"
                } else {
                    "[FAIL] 无 width 的窗口拖不动 / 横向拖动把高度也改了"
                }
            );
        }
        // --sim-resize 阶段 3：**收缩高度 = 恒定值**（松手当帧 vs 40 帧后必须相同）。
        if self.sim_resize && f.frames() == 150 {
            let free = self.windows.shrink_after_free;
            let later = self.windows.shrink_later;
            let ok = free.zip(later).is_some_and(|(a, b)| (a - b).abs() <= 0.5);
            eprintln!(
                "sim-resize: 收缩高度 松手后={free:?} 40 帧后={later:?} {}",
                if ok {
                    "[OK] 收缩后的窗口高度是恒定值（不回弹 / 不抖动）"
                } else {
                    "[FAIL] 收缩后的高度变了（回弹 / 抖动 ⇒ 高度不是恒定值）"
                }
            );
        }
        // --sim-pick-save：**导出主题的端到端判定**（含"跳过阻塞对话框"）。
        // 真值看状态行：载入 / 导出成功、或"导出已取消"（`RJ_PICK_SAVE=none`）。
        if self.sim_pick_save && f.frames() == 40 {
            let st = &self.top.import_status;
            let ok = st.contains("主题已导出") || st.contains("导出已取消");
            eprintln!(
                "sim-pick-save: status={st:?} {}",
                if ok {
                    "[OK] 导出通路走通（`--no-file-dialog` ⇒ 完全不碰 rfd；结果由 `--pick theme-save=..` 预置）"
                } else {
                    "[FAIL] 没走到导出（缺 `--no-file-dialog` ⇒ 弹了阻塞的对话框 / 卡住）"
                }
            );
        }
        // --sim-resize 阶段 4：**纯点击柄不该改尺寸**（"第二次点击瞬移回原位"的回归守卫）。
        if self.sim_resize && f.frames() == 185 {
            let before = self.windows.shrink_later;
            let now = self.sim_resize_after.map(|s| s.y);
            let ok = before.zip(now).is_some_and(|(a, b)| (a - b).abs() <= 0.5);
            eprintln!(
                "sim-resize: 再点一次柄（不拖）：前={before:?} 后={now:?} {}",
                if ok {
                    "[OK] 纯点击不改尺寸（没有跳回上一次拖之前的值）"
                } else {
                    "[FAIL] 第二次点击把尺寸改了（瞬移）"
                }
            );
        }
        // --sim-chrome 阶段 6：**拖高过之后点 ⌃，必须真的收起**（= 一行标题栏，
        // 而不是"空着的高窗"）。用户实测："点击收起后不会收起"。
        if self.sim_chrome && f.frames() == 230 {
            let (dragged, collapsed) = (self.windows.eng_dragged_h, self.windows.eng_collapsed_h);
            let ok = dragged
                .zip(collapsed)
                .is_some_and(|(d, c)| d > 100.0 && (40.0..=70.0).contains(&c) && c < d - 30.0);
            eprintln!(
                "sim-chrome[收起忽略持久高]: 拖大后高={dragged:?} → 收起后高={collapsed:?} {}",
                if ok {
                    "[OK] 收起后真的只剩一行标题栏（持久高不参与收起态）"
                } else {
                    "[FAIL] 收起后仍是原来的高 / 空着不倒（持久高把收起态撑住了）"
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
        // --sim-row-overflow：三条判据（窄窗口里"不许突出去" + "幽灵控件点不到"）。
        // 1. `row(..)` 的**结算宽** ≤ 窗口可用宽 —— 修前整行 = 标签 + 输入框自然宽之和，
        //    会超出可用宽（控件排到窗口外面）；
        // 2. 点窗口**内**那个控件 ⇒ 焦点必须落在它身上（正对照：守卫不能把整窗点废）；
        // 3. 点窗口**外**那个溢出控件 ⇒ 焦点**不得**落在它身上（裁掉的部分不可命中）。
        if self.windows.sim_row_overflow && f.frames() == 44 {
            let size = self.windows.row_size;
            let avail = self.windows.row_avail_w;
            let row_ok = size.is_some_and(|s| s.x <= avail + 0.5);
            // ①b `Label + ColorPicker` 那一行同样不许溢出（取色器也必须尊重可用宽）。
            let picker_ok = self
                .windows
                .row_size_picker
                .is_some_and(|s| s.x <= avail + 0.5);
            let inside_ok = self.windows.row_inside_focus;
            let ghost_ok = !self.windows.row_ghost_focus;
            eprintln!(
                "sim-row-overflow: 行宽={:?} 取色器行宽={:?} 可用宽={avail:.0} 窗内点得到={inside_ok} 窗外点得到={} {}",
                size,
                self.windows.row_size_picker,
                self.windows.row_ghost_focus,
                if row_ok && picker_ok && inside_ok && ghost_ok {
                    "[OK] 窄窗口里两行都落在可用宽内（含取色器）+ 溢出的幽灵控件点不到（窗内的仍点得到）"
                } else {
                    "[FAIL] 行仍然突出去（含取色器行）/ 幽灵控件还能点 / 守卫把窗内也挡住了"
                }
            );
        }
        // --sim-zorder：先记"翻页前同一屏幕点选中的条目号"（翻页后必须更大）。
        if self.sim_zorder && f.frames() == 132 {
            self.zorder_sel_before = self.right.list_sel;
        }
        // --sim-zorder 判定：两段现场都必须**真的发生**，否则 `RJ_ORDER_TRACE` 的那几行
        // `order[...]` 只是纸上谈兵（同 `--sim-clip` 的教训：断言空跑比没有断言更糟）。
        if self.sim_zorder && f.frames() == 150 {
            let moved = self
                .zorder_panel
                .is_some_and(|p| (p - ZORDER_PANEL_DST).length() <= 4.0);
            let before = self.zorder_sel_before;
            let after = self.right.list_sel;
            let scrolled = before.zip(after).is_some_and(|(b, a)| a > b);
            eprintln!(
                "sim-zorder: 面板拖动后={:?}（目标={ZORDER_PANEL_DST:?}）列表点={:?} 选中 {before:?} → {after:?} {}",
                self.zorder_panel,
                self.sim_zorder_pts,
                if moved && scrolled {
                    "[OK] 面板压到标签上 + 滚动条翻页后同一屏幕点选到更后面的条目"
                } else {
                    "[FAIL] 面板没拖到位 / 滚动条翻页没生效（脚本坐标打空）"
                }
            );
        }
        // --sim-text-cull 判定：滚到中段后**文字仍在提交**。
        // 判据 = 两帧的 `verts` 基本不变（修前会掉掉整条文本的顶点）；容差 24 覆盖
        // "光标随视图滚出框而被合法剔除"（`Caret` 是图形命令，本来就不该计入文本剔除）。
        if self.sim_text_cull && f.frames() == 30 {
            let (b, a) = (
                self.cull_verts_before.unwrap_or(0),
                self.cull_verts_after.unwrap_or(0),
            );
            let (cb, ca) = (
                self.cull_text_before.unwrap_or(0),
                self.cull_text_after.unwrap_or(0),
            );
            eprintln!(
                "sim-text-cull: 未滚动 verts={b} culled_text={cb} → 滚到 {TEXT_CULL_SCROLL:.0}px 后 \
                 verts={a} culled_text={ca}（verts 差 {}）",
                b as i64 - a as i64
            );
            eprintln!(
                "sim-text-cull: {}",
                if a + 24 >= b && ca <= cb {
                    "[OK] 滚过头后整条文本仍在提交（未被兜底剔除）"
                } else {
                    "[FAIL] 滚到中段后文本被整条剔除（顶点数掉了一截 / culled_text 涨了）"
                }
            );
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

        // --sim-chrome：**引擎托管收起**的判定（阶段 4）：`img_box_fill` 用的是
        // `.collapsible(true, None)`（应用侧**没有** bool）⇒ 点 ⌃ 收起 / 再点展开，必须看到
        // `UiState::is_collapsed` 与 dump 尺寸**同步**变化（只改状态不改几何 = 只画了按钮；
        // 只改几何不改状态 = 状态没托管住）。`eng_states` 每帧记 `(状态, 尺寸)`（去重）。
        if self.sim_chrome && f.frames() == 122 {
            let st = &self.windows.eng_states;
            let collapsed_seen = st.iter().any(|(c, _)| *c);
            let expanded_back = st.last().is_some_and(|(c, _)| !*c);
            let (lo, hi) = st.iter().fold((f32::MAX, 0.0f32), |(lo, hi), (_, s)| {
                (lo.min(s.y), hi.max(s.y))
            });
            // 收起 ⇒ 只剩一行标题栏 ⇒ 高度明显变矮（不是"差一两个像素"）。
            let geometry_moved = hi - lo > 10.0;
            let ok = collapsed_seen && expanded_back && geometry_moved;
            eprintln!(
                "sim-chrome[引擎托管收起]: 出现过收起={collapsed_seen} / 又展开={expanded_back} / \
                 几何同步变={geometry_moved}（高 {lo:.0}↔{hi:.0}） {}",
                if ok {
                    "[OK] collapsible(.., None)：收起状态由 UiState 托管，点 ⌃ 收起 / 展开都生效"
                } else {
                    "[FAIL] 引擎托管的收起没走通（状态或几何没跟着变）"
                }
            );
        }

        // --sim-chrome 阶段 5：点 ⌃ 的**行中心**也必须能收起。中心点落在收起态窗口右下角
        // 的**缩放柄**上（`caption_pts` 之所以只点"上半"，就是为了躲它）——本轮把窗口柄的
        // **应用**推迟到内容 + 标题栏之后（且只在 `!press_claimed` 时），语义上"控件/按钮
        // 优先"。这条是**回归守卫**：中心照旧可点（不断言"修前必挂"——修前靠控件级遮挡
        // 也能赢，但那是两套机制碰巧一致）。
        if self.sim_chrome && (f.frames() == 138 || f.frames() == 160) {
            let now = self.windows.eng_states.last().map(|(c, _)| *c).unwrap_or(false);
            if f.frames() == 138 {
                self.windows.eng_center_before = Some(now);
            } else {
                self.windows.eng_center_after = Some(now);
                let ok = self
                    .windows
                    .eng_center_before
                    .zip(self.windows.eng_center_after)
                    .is_some_and(|(a, b)| a != b);
                eprintln!(
                    "sim-chrome[柄让位]: 点 ⌃ 行中心 前={:?} 后={:?} {}",
                    self.windows.eng_center_before,
                    self.windows.eng_center_after,
                    if ok {
                        "[OK] 收起态里点 ⌃ 的**行中心**也能翻转（缩放柄与按钮重叠时按钮照常生效）"
                    } else {
                        "[FAIL] 行中心点不动（按下被缩放柄抢走 / 按钮没响应）"
                    }
                );
            }
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
        // **UI 层的真实 draw op 数**（`Render2D::prepare` 之后可查；UI 有自己的
        // `Render2D`，见 `Frame::draw_ui`）：与 UI 自己的 `[perf] segs=`（**批次候选数**
        // = `UiBatch` 数）是**两个口径**——阶段 9 之后提交计划按单元缓存，`segs` 会略涨，
        // 但相邻同状态批次会被 `Render2D` 合成一个动态段，所以 `draw_ops` 才是
        // "UI 这一帧真的发了几次 draw"。⚠ 别读 `f.draw()`（那是**世界层**）。
        let draw_ops = f.draw_ui().draw_op_count() as u64;
        let t_present = Instant::now();
        f.present();
        let present_us = t_present.elapsed().as_secs_f64() * 1e6;

        // 性能统计：整帧 / 渲染（细分）/ UI 各阶段（每 PERF_PRINT_EVERY 帧打印一次）
        let render_us = begin_us + encode_us + submit_us + present_us;
        let frame_us = t_frame.elapsed().as_secs_f64() * 1e6;
        self.perf.add(
            &ui_stats,
            frame_us,
            render_us,
            begin_us,
            encode_us,
            submit_us,
            present_us,
        );
        self.perf.draw_ops += draw_ops;
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
