// `TextEditor` 现在在 prelude 里（与 `Segmented` 不同——后者仍要显式引 `widgets`）。
// `foldable` 的容器入口在 `UiAdd` 上（`ui.foldable(id, label)` / `ui.foldable_custom(id, ..)`）；
// **顶层**的裸 `Ui` 想自定义标题就显式引 `Foldable` 并传 `ui` 作首参。
use rjw_krusie::{prelude::*, ui::widgets};
// **顶层**（不在容器闭包里）的自定义标题区块要显式引 `Foldable` 并传裸 `ui` 作首参。
// ⚠ 扩展标签的 `Label::ex` 要用 **UI 的 `Label`**——prelude 里的 `Label` 是
// `rjw_text::Label`（同名不同物，它是文本链的 builder），显式引会遮蔽它。
use rjw_krusie::ui::{Foldable, Label};

use crate::app;

#[derive(Default)]
pub struct Gallery {
    input_single_line: String,
    input_multi_line: String,
    integer: i32,
    boolean: bool,
    /// **可收缩区块**演示：标题里的勾选框（`Foldable::custom` 的逃生舱）。
    fold_all: bool,
    /// **顶层 `Foldable::custom(ui, ..)` 现场**是否录制（`--sim-fold` 打开；见 `demo` 里
    /// "顶层区块"那一段）。
    sim_fold: bool,
}

impl Gallery {
    /// `--sim-fold` 用：默认 demo（所有现场照常录制）。
    pub fn new_sim_fold() -> Self {
        Self { sim_fold: true, ..Self::default() }
    }
}

const TITLE: &str = "Gallery 窗口控件展示";

/// **`--sim-fold`：控件绝对 ID 的候选列表**（可机器断言的判据来源）。
///
/// 三条事实在这里钉住：
/// 1. **窗口内**控件录在窗口的 ID 命名空间里（窗口名 = `type_name::<Self>()`）；
/// 2. `Foldable` 的**正文**录在区块自己的命名空间里 ⇒ 相对名 `perf_number` 的绝对 ID
///    带上 `gallery_perf/` 前缀（这就是"折叠 = 正文完全不录制"能被机器查到的原因：
///    折叠时 `UiState::widgets` 里根本没有这个键）；
/// 3. **顶层**（不在任何容器里）调用时前缀为空 —— 所以顶层区块的键是裸 `gallery_top`。
///
/// ⚠ **本窗口开了 `.vscroll(true)`** ⇒ 窗口内容被包进**滚动视口** ⇒ 窗口内控件的绝对 ID
/// 多一层 `scroll/`（`{窗口 id}/scroll/{相对名}`，见 `window_impl` 的滚动分支）。
/// 所以这里给**两个候选**（带 / 不带 `scroll/`）：开关 `.vscroll(..)` 都不会让仿真失真。
fn sim_id_candidates(rel: &str) -> [String; 2] {
    let w = std::any::type_name::<Gallery>();
    [format!("{w}/scroll/{rel}"), format!("{w}/{rel}")]
}

/// **`--sim-fold` 解算出的区块标题行屏幕中心**（录制端写、注入端读，见 `RJWApp::update`）。
/// f32 位模式打包（x,y × 3 个区块）；全 `0` = 还没解算。
static SIM_FOLD_PTS: [std::sync::atomic::AtomicU64; 6] = [
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
    std::sync::atomic::AtomicU64::new(0),
];
/// 顶层区块的键（无命名空间前缀；见 [`sim_id_in_window`] 第 3 条）。
fn sim_id_at_root(rel: &str) -> String {
    rel.to_owned()
}

/// **`--sim-fold` 自校准**：每一轮"点之前"的折叠态（跨帧存，见 `demo` 里的断言）。
/// `u8` 三态：`0` = 未记录、`1` = 展开、`2` = 折叠。
static SIM_FOLD_BEFORE: [std::sync::atomic::AtomicU8; 3] = [
    std::sync::atomic::AtomicU8::new(0),
    std::sync::atomic::AtomicU8::new(0),
    std::sync::atomic::AtomicU8::new(0),
];

fn sim_store_before(i: usize, folded: bool) {
    SIM_FOLD_BEFORE[i].store(if folded { 2 } else { 1 }, std::sync::atomic::Ordering::Relaxed);
}

/// 取出并清空第 `i` 轮的"点之前"折叠态（`None` = 没记过）。
fn sim_take_before(i: usize) -> Option<bool> {
    match SIM_FOLD_BEFORE[i].swap(0, std::sync::atomic::Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

/// 写入第 `i` 个区块的屏幕注入点（见 [`SIM_FOLD_PTS`]）。
fn sim_store_point(i: usize, p: Vec2) {
    use std::sync::atomic::Ordering as O;
    SIM_FOLD_PTS[i * 2].store(p.x.to_bits() as u64, O::Relaxed);
    SIM_FOLD_PTS[i * 2 + 1].store(p.y.to_bits() as u64, O::Relaxed);
}

/// 读出第 `i` 个区块的屏幕注入点（`None` = 还没解算）。
pub fn sim_fold_point(i: usize) -> Option<Vec2> {
    use std::sync::atomic::Ordering as O;
    let x = SIM_FOLD_PTS[i * 2].load(O::Relaxed);
    let y = SIM_FOLD_PTS[i * 2 + 1].load(O::Relaxed);
    if x == 0 && y == 0 {
        return None;
    }
    Some(Vec2::new(f32::from_bits(x as u32), f32::from_bits(y as u32)))
}


impl app::Demo for Gallery {
    fn id(&self) -> &'static str {
        TITLE
    }

    fn demo(&mut self, ui: &mut app::Ui, enable: &mut bool, _global: &mut app::global::GlobalData) {
        let Self {
            input_single_line,
            input_multi_line,
            integer,
            boolean,
            fold_all,
            sim_fold,
        } = self;

        // ── `--sim-fold`（脚本化自证）────────────────────────────────────────────
        // 帧号以**引擎侧**为准（`UiState::frame`）—— `demo` 一帧可能被调用多次（仿真里还会
        // 另开一段 UI 录顶层区块），自数帧会漂。
        // 判据用**引擎状态**（正文里的控件有没有被登记）而不是像素："折叠 = 正文完全不录制"
        // 正是本特性最容易悄悄坏掉的地方，`UiState::widgets` 里没有那个键就是硬证据。
        let frame = ui.state().frame().frame();
        if *sim_fold {
            // ① 每帧解算三个区块标题行的**屏幕矩形**（引擎记下的交互矩形，绝对坐标）
            //    ⇒ 注入点零手算，主题 / 布局怎么改都跟得上；窗口被拖过也不会点空。
            for (i, rel) in ["gallery_perf", "gallery_hint", "gallery_filters"].iter().enumerate() {
                if let Some(r) = sim_id_candidates(rel)
                    .iter()
                    .find_map(|k| ui.state().widgets().rect(k))
                {
                    sim_store_point(i, Vec2::new(r.x + r.w * 0.5, r.y + r.h * 0.5));
                }
            }
            if frame == 20 {
                // ② **标题行宽度**（本轮修的显示 BUG）：自动宽窗口里 `avail_w` 恒 `None`，
                //    标题行一度退化成"图标 + 文字实测宽"（看起来缩在半截、与上下几行不对齐），
                //    而**自定义标题**更糟：退化成 `pad*2 + icon ≈ 39px` ⇒ 闭包里的控件整排溢出。
                //    现在两条路都取"同容器已排布内容的最宽宽"⇒ 三个标题行都必须**远宽于一行高**。
                let mut bad = Vec::new();
                for rel in ["gallery_perf", "gallery_hint", "gallery_filters"] {
                    let w = sim_id_candidates(rel)
                        .iter()
                        .find_map(|k| ui.state().widgets().rect(k))
                        .map(|r| r.w);
                    if !w.is_some_and(|w| w > 200.0) {
                        bad.push(format!("{rel}:{w:?}"));
                    }
                }
                eprintln!(
                    "sim-fold: 三个标题行宽度都 > 200（装得下文字 / 自定义标题不退化）{} {:?}",
                    if bad.is_empty() { "[OK]" } else { "[FAIL]" },
                    bad
                );
                // ③ 折叠语义：默认折叠的区块**正文完全不录制**；`.open(true)` 的区块标题在录。
                let has = |rel: &str| {
                    sim_id_candidates(rel)
                        .iter()
                        .any(|k| ui.state().widgets().get(k).is_some())
                };
                let perf_body = has("gallery_perf/perf_number"); // 默认**折叠** ⇒ 不该有
                let hint_title = has("gallery_hint"); // `.open(true)` ⇒ 展开（标题行自己有键）
                let filters_body = has("gallery_filters/kw"); // 默认折叠 ⇒ 不该有
                let ok = !perf_body && hint_title && !filters_body;
                eprintln!(
                    "sim-fold: 折叠区块正文={perf_body}(期望 false) · 展开区块标题={hint_title}(期望 true) · \
                     过滤区块正文={filters_body}(期望 false) {}",
                    if ok { "[OK]" } else { "[FAIL]" }
                );
            }
            // ④ **脚本化点击逐个区块**（回归"有些 foldable 无法展开"）：每一轮 = 点标题行，
            //    隔几帧读**引擎状态**断言"折叠态**翻了**"（自校准：点前先读一次 ⇒ 不写死期望值）。
            //    轮 3 回到轮 0 的区块 ⇒ 证明**可来回翻**而不是"只能开一次"。
            let rounds: &[(usize, u64, &str)] = &[
                (0, 50, "gallery_perf"),
                (1, 60, "gallery_hint"),
                (2, 70, "gallery_filters"),
                (0, 80, "gallery_perf"),
            ];
            // 折叠态也走**候选键**（区块 id 同样多一层 `scroll/`）。
            let folded_of = |rel: &str| {
                sim_id_candidates(rel).iter().any(|k| ui.state().is_folded(k))
            };
            for &(i, t, fold_id) in rounds {
                if frame == t - 2 {
                    sim_store_before(i, folded_of(fold_id));
                }
                if frame == t + 4 {
                    let folded = folded_of(fold_id);
                    let before = sim_take_before(i);
                    let ok = before.is_some_and(|b| folded != b);
                    eprintln!(
                        "sim-fold: 轮(i={i}, frame={t}) 点标题行 ⇒ folded={folded}（点前 {before:?}）{}",
                        if ok { "[OK]" } else { "[FAIL]" }
                    );
                }
            }
            if frame == 90 {
                // ⑤ 顶层 `Foldable::custom(ui, ..)`（**裸 `Ui`** 入口）也在录：标题行 + 它
                //    `.open(true)` 的正文都该有键（第 44 帧起才录）；顶层调用**无前缀**。
                let title = ui.state().widgets().get(&sim_id_at_root("gallery_top")).is_some();
                let body = ui.state().widgets().get(&sim_id_at_root("gallery_top/top_number")).is_some();
                eprintln!(
                    "sim-fold: 顶层自定义标题={title} 其正文={body}(期望 true/true) {}",
                    if title && body { "[OK]" } else { "[FAIL]" }
                );
            }
        }
        

        super::demo_window(ui, std::any::type_name::<Self>(), TITLE, enable)
        .resize(true)
        .vscroll(true)
        .show(|ui| {
            ui.row( |ui| {
                ui.label("这是一个Label");
                ui.colored_label("这是彩色的", Color::CSS_AQUAMARINE);
            });
            ui.divider();
            ui.row(|ui| {
                ui.label("按钮：");
                if ui.button("btn1", "点我 + 1").clicked() {
                    *integer = integer.overflowing_add(1).0;
                }
                if ui.button("btn2", "点我 + 10").clicked() {
                    *integer = integer.overflowing_add(10).0;
                }
            });
            ui.row(|ui| {
                ui.label("数字输入：");
                ui.add(NumberInput::new("number_input", integer).speed(0.01));
            });
            ui.row(|ui| {
                ui.label("单行输入框：");
                ui.text_input("text_sl", input_single_line);
            }); // ⚠ row 把子项钉到"一行标准高"（`Theme::row_h`）——多行控件想撑高整行见 row Builder
            ui.row(|ui| {
                ui.label("多行输入框：");
                // 缩放：默认下限 = 一行文字高（拖不到 0）；自动申请会先问尺寸责任链
                // ⇒ 拖大后窗口与**下面的控件**跟着长；缩放柄形状取 `Theme::input.grip`（默认三条横线）。
                ui.add(TextEditor::new("text_ml", input_multi_line).multiline().resize(Resize::Both));
            });
            ui.row(|ui| {
                ui.label("复选框：");
                ui.checkbox("checkbox", "", *boolean).toggled().then(|| {*boolean = !*boolean});
            });
            ui.row(|ui| {
                ui.label("分段按钮：");
                let mut idx = !*boolean as usize;
                ui.add(widgets::Segmented::new("seg", &["是", "否"], &mut idx));
                *boolean = idx == 0;
            });

            ui.divider();
            
            // ① 默认折叠 + 正文里放控件（折叠时 `perf_number` **不存在**于 `UiState`）——
            // `--sim-fold` 的 ① 与轮 0/3 都指着它（整段被删过一次，整批仿真当场失真）。
            ui.foldable("gallery_perf", "性能统计（默认折叠）").show(|ui| {
                ui.row(|ui| {
                    ui.label("FPS：");
                    ui.add(NumberInput::new("perf_number", integer));
                });
            });
            ui.foldable("gallery_hint", "折叠区块（.open(true) 默认展开）")
                .open(true)
                .show(|ui| {
                    ui.label("展开态由 `.open(bool)` 决定 —— 只影响**从未被点过**的区块。");
                    ui.label("点标题行任意空白处即可翻转；标题行里的控件自己认领按下。");
                });
            ui.foldable_custom("gallery_filters", |t| {
                t.label("过滤");
                t.checkbox_mut("all", "全选", fold_all);
            })
            .show(|ui| {
                ui.row(|ui| {
                    ui.label("关键字：");
                    ui.text_input("kw", input_single_line);
                });
            });

            ui.divider();
            // **区块标题里也能放 `LabelEx`**（标题 = 标准容器）。⚠ `.open(true)` 是必需的：
            // 区块默认折叠 ⇒ 正文**完全不录制** ⇒ 下面这组演示会等于不存在（曾经就踩过）。
            ui.foldable_custom("title_fold", |ui| {
                Label::ex("彩色渐变实例（标题里放 LabelEx）")
                    .gradient(Color::rgba_u8(255, 96, 96, 255), Color::rgba_u8(120, 180, 255, 255))
                    .valign(rjw_krusie::ui::text::TextVAlign::Center)
                    .show_in(ui);
            })
            .show(|ui| {
                // ── 扩展标签（`Label::ex` / `ui.label_ex` / `ui.colored_label`）──────────
                // 全部**与默认 `Label` 同字号**（不设 `.font_size`）⇒ 观感与上面一致，
                // 差别只在**显式指定**的那些属性（颜色 / 字重 / 渐变）上。
                // ① **横向渐变**（首末两色；容器闭包内 ⇒ `show_in`）。
                Label::ex("① 横向渐变（Label::ex + gradient，字号同默认 Label）")
                    .gradient(Color::rgba_u8(255, 96, 96, 255), Color::rgba_u8(120, 180, 255, 255))
                    .show_in(ui);
                // ② **整组 `TextStyle` + 字段级覆盖**（容器闭包内走 `show_in`）：`.style(..)`
                //    给基准，`.tint` / `.letter_spacing` 再压上去（字段级恒胜，与调用顺序无关）。
                ui.label_ex("整组 TextStyle + 字段级覆盖")
                    .style(TextStyle::new().size(15.0).italic(true))
                    .tint(Color::rgba_u8(170, 235, 190, 255))
                    .letter_spacing(0.05)
                    .show_in(ui);
                // ③ **一步到位的语法糖**（颜色随数值变化 ⇒ 每帧重建顶点，验证签名含颜色）。
                ui.colored_label(
                    &format!("整数（colored_label 快照）：{integer}"),
                    Color::rgba_u8(255, 210, 120, 255),
                );
                // ④ **纵向渐变**（`gradient_v`）：整块自上而下过渡。
                ui.label_ex("纵向渐变（gradient_v）：上白下蓝")
                    .gradient_v(Color::WHITE, Color::rgba_u8(120, 160, 255, 255))
                    .show_in(ui);
                // ⑤ **渐变域对照**（`Glyph` / `Line` / `Text`=整块）：**多行**才看得出差别 ——
                //    逐字形每字一条完整渐变、逐行每行一条、整块跨行连续（短行只吃一段）。
                //    单行文本时 Line 与 Text 逐像素相同。
                let g_from = Color::rgba_u8(255, 96, 96, 255);
                let g_to = Color::rgba_u8(120, 180, 255, 255);
                ui.label_ex("Glyph 域：每个字一条完整渐变\n第二行明显更长一些")
                    .gradient_glyph(g_from, g_to)
                    .show_in(ui);
                ui.label_ex("Line 域：每行一条完整渐变（默认）\n第二行明显更长一些")
                    .gradient_line(g_from, g_to)
                    .show_in(ui);
                ui.label_ex("Text 域（Frame）：跨行连续一条渐变\n第二行明显更长一些")
                    .gradient_text(g_from, g_to)
                    .show_in(ui);
            });

            ui.divider();
            // ── `row` 自动换行（`row_wrap` / `row_builder().wrap_w(..)`）──────────
            // 行宽上限 260：塞不下就**收行**（行间距默认 = `gap`），行内左上角对齐。
            // "只换行、不压缩"——里面的子项按自然宽排版，不会被压扁。
            ui.label("row 自动换行（上限 260，7 个按钮 ⇒ 应当折成多行）：");
            let wrapped = ui.row_builder().wrap_w(260.0).show(|r| {
                for i in 1..=7 {
                    r.button(&format!("btn_wrap{i}"), &format!("按钮{i}"));
                }
            });
            ui.label(&format!(
                "上面这行的结算尺寸 = {:.0}×{:.0}（高 > 一行标准高 ⇒ 确实折行了）",
                wrapped.x, wrapped.y
            ));
        });

        // ── 顶层 `Foldable::custom(ui, ..)`（**裸 `Ui`** 入口）────────────────────────
        // 只有 `--sim-fold` 录它，且从第 44 帧起（前 44 帧留给"点窗口内区块"的注入，
        // 免得顶层区块与窗口抢命中）。顶层调用**无命名空间前缀**：`gallery_top` 是裸键。
        // ⚠ 它在**窗口之外**录 ⇒ 必须显式引 `Foldable` 并传 `ui` 作首参（容器闭包里才用
        // `UiAdd::foldable_custom`）。
        if *sim_fold && frame >= 44 {
            // 先占一格，把区块让到菜单栏下方（`foldable` 是"占光标的一整格"，没有 pos 参数）。
            ui.child_rect(1.0, 44.0, Child::Fit);
            Foldable::custom(ui, "gallery_top", |t| {
                t.label("顶层区块（裸 Ui + 自定义标题）");
            })
            .open(true)
            .show(|ui| {
                ui.row(|ui| {
                    ui.label("整数：");
                    ui.add(NumberInput::new("top_number", integer));
                });
            });
            // **裸 `Ui` 的 `Label::ex(..).show(ui)`**（窗口闭包外唯一能写 `show` 的地方；
            // 容器里是 `show_in`）。同样只在 `--sim-fold` 录 —— 免得顶层内容与窗口抢位置。
            Label::ex("裸 Ui 的 Label::ex(..).show(ui)（sim-only）")
                .tint(Color::rgba_u8(150, 220, 255, 255))
                .show(ui);
        }
    }
}