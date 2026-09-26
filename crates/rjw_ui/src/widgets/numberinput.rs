//! **数字输入框**（组合控件，只依赖公开 API）。
//!
//! - **拖拽调值**：按住右侧手柄**水平拖动**（**向右拖 = 增加**），拖到窗口边缘自动
//!   **warp**（光标跳到对侧继续拖），松开结束；手柄悬停/拖拽显示
//!   [`crate::UiCursor::EwResize`]（↔）；**窗口最大化时同样生效**（鼠标无法越出窗口，
//!   用"边缘检测"触发 warp）；
//! - **输入模式**：点击文本框 → 禁用拖拽 + **全选** + I 型光标键盘输入；直到失去
//!   焦点 → 恢复拖拽模式；
//! - **显示文本内部管理**：[`NumberInput::new`] 只需数值引用——失焦显示由 `value`
//!   派生，聚焦编辑缓冲跨帧持久于 `WidgetState`（无需调用方持有 `String`）；也可
//!   [`NumberInput::with_text`] 绑定外部 `&mut String`（旧形态）；
//! - 非数字输入被屏蔽（只留数字 / 负号 / 小数点 / 空格；整数类型连小数点也屏蔽）；
//!   拖拽基准用独立状态 ID `{id}::grip`（不与 `text_input_at` 共用 `WidgetState`，
//!   否则 `press_mouse` 互相覆盖）。
//!
//! # 数值类型（与 [`Slider`](crate::Slider) 同一套泛型）
//!
//! `T: SliderValue` = **`f32`（默认）/ `f64` / 全部有符号与无符号整数**（见
//! [`crate::SliderValue`]；自定义类型实现两个转换方法即可）。类型由 `&mut T` 推断：
//!
//! ```no_run
//! # use rjw_ui::{NumberInput, Ui, UiAdd};
//! # fn f(ui: &mut Ui, mut gain: f32, mut hp: i32, mut acc: f64) {
//! ui.add(NumberInput::new("gain", &mut gain));                    // f32（默认）
//! ui.add(NumberInput::new("hp", &mut hp).range(0, 100));           // i32：默认步进 1、显示无小数
//! ui.add(NumberInput::new("acc", &mut acc).step(1e-6));           // f64：高精度不丢位
//! # }
//! ```
//!
//! 整数类型的差异：**默认步进 = 1**、**显示无小数**、**输入只收整数文本**（小数点被过滤）、
//! 拖拽值在 `T` 边界处**饱和**（`u8` 拖到 300 → 255）。
//!
//! # 精度（四条明确的口径）
//!
//! 1. **内部数学用 `f64`**（步进 / 位置增量 / 吸附都在 `f64` 里算，进出一共各换算一次）：
//!    `f32` 在"大数值 + 小步进"（`1000` 配 `step = 0.01` ⇒ `raw / step` 已到 `1e5`，
//!    而 `f32` 只有 24 位有效位）下吸附会**跳格 / 不动**；
//! 2. **吸附后按 `step` 的十进制位数取整**（`0.1` → 1 位、`0.25` → 2 位）：写回的是
//!    "用户以为的那个十进制数"的**最邻近 double** ⇒ 应用侧 `v == 0.1` 这类比较能成立
//!    （旧实现存 `0.30000001…` 之类的原始值，`==` 永远 false）；
//! 3. **先吸附、再 clamp**：旧顺序（先 clamp 再吸附）会让结果**顶出 `max`**
//!    （`max = 1.0`、`step = 0.4` ⇒ `round(1 / 0.4) × 0.4 = 1.2`）。代价是"范围端点不在
//!    格点上"时结果是端点本身（不在格上）——这比越界好；
//! 4. **拖动会吸附、打字不会**：`step` 是**拖动精度**；手打的 `0.37`（`step = 0.1`）原样保留
//!    （只 clamp 到 `range`），显示上也**如实**显示到"能表示它"的小数位（`0.37`，而不是被
//!    `step` 的四舍五入糊成 `0.4`）。
//!
//! # 绕窗（warp）拖拽：**请求**与**补偿**分开
//!
//! 拖到窗口左右边缘时，控件请求把光标挪到对侧内侧（[`Ui::set_cursor_position`]）——
//! 这样"横向拖很短的距离"也能穿过整个取值范围（窗口最大化时鼠标本来也出不去窗口）。
//!
//! ⚠ 关键：`set_cursor_position` 只是**请求**，真正挪动要等 OS 事件（可能晚一两帧，
//! 也可能不生效）。所以拖拽基准**不能**在请求当帧就平移——那会在"光标还没挪"的几帧里
//! **每帧再平移一次**，值以"一个窗宽 / 帧"飞走（历史 BUG）。现在的口径：
//!
//! 1. **请求**：`mx` 到边缘 ⇒ 请求挪到对侧内侧 `3px`（`mx` 用**客户区物理像素**，与
//!    [`Ui::mouse_screen`] / winit 的 `set_cursor_position` 同一坐标系）；
//! 2. **补偿**：下一帧**真的看到**跨窗跳变（`|mx − 请求点| > 半个窗宽`）才把拖拽基准
//!    平移**同样的量** ⇒ 值严格连续、且只补一次；光标没挪 ⇒ 基准不动 ⇒ 值停在边缘；
//! 3. 只有"请求过"才会做跳变判定 ⇒ **快速甩鼠标**（单帧大位移）不会被误判成 warp。
//!
//! 这一整套状态机是纯函数 [`warp_step`]（4 个单测：请求不动基准 / 光标没挪时冻结 /
//! 补偿恰好一次 / 端到端序列严格连续）。

use glam::Vec2;
use rjw_transform::Rect;

use crate::draw::{CornerRadius, Icon, Position, Size};
use crate::hit::update_drag;
use crate::id::IdAbsolute;
use crate::{FocusKind, Response, SliderValue, TextEditor, Ui, UiCursor, Widget};

/// **右侧拖拽调值手柄的宽度**（物理像素）。
///
/// 手柄就是数字条最右边这一条（`rect` 的右 `GRIP_W` 宽），**不是整个控件**：
/// 点在文本框上是"进入编辑"，只有这一条能拖动调值。公开出来是给**脚本化测试**用的
/// （`--sim-tuner` 要算出"手柄在哪一格"）——写死这个数字，改宽度后会静默点空，
/// 而点空的症状是"控件没反应"，非常难查。
pub const GRIP_W: f32 = 20.0;

/// 数字输入框（文本框 + 右侧拖拽调值手柄）；数值类型 `T` 泛型。
pub struct NumberInput<'a, T: SliderValue = f32> {
    id: &'a str,
    value: &'a mut T,
    /// 外部绑定的显示文本（`None` = 内部跨帧持久管理，无需调用方持有）。
    text: Option<&'a mut String>,
    /// 下界（`None` = 不限制；整数类型恒受自身取值范围限制——回写是**饱和转换**）。
    pub min: Option<T>,
    /// 上界（`None` = 不限制）。
    pub max: Option<T>,
    /// 拖拽**精度**：每物理像素数值（默认：浮点 `0.01` = 每像素 ±0.01、整数 `1`；`≤ 0` = 不吸附）。
    pub step: T,
    /// 默认速度倍率（默认 10：细调）。
    pub speed: f32,
    /// 按住 **Shift** 拖拽的速度倍率（默认 10：细调）。
    pub shift_speed: f32,
    /// 按住 **Ctrl** 拖拽的速度倍率（默认 0.1：精调）。
    pub ctrl_speed: f32,
}

impl<'a, T: SliderValue + PartialOrd> NumberInput<'a, T> {
    /// 主构造：只需数值引用——显示文本由内部跨帧持久管理（失焦显示由 `value` 派生，
    /// 聚焦时编辑缓冲存于 `WidgetState`），**无需调用方持有 `&mut String`**。
    pub fn new(id: &'a str, value: &'a mut T) -> Self {
        Self {
            id,
            value,
            text: None,
            min: None,
            max: None,
            step: T::default_step(),
            speed: 0.1,
            shift_speed: 10.0,
            ctrl_speed: 0.1,
        }
    }

    /// 可选：绑定外部显示文本（旧形态，显式持有 / 需要从外部读显示值的场景）。
    pub fn with_text(mut self, text: &'a mut String) -> Self {
        self.text = Some(text);
        self
    }

    /// 取值范围（两端都限制；不调 = 不限制，但整数类型仍受自身类型范围限制）。
    pub fn range(mut self, min: T, max: T) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }

    /// 只设下界。
    pub fn min(mut self, min: T) -> Self {
        self.min = Some(min);
        self
    }

    /// 只设上界。
    pub fn max(mut self, max: T) -> Self {
        self.max = Some(max);
        self
    }

    /// 拖拽**精度**：每物理像素数值（默认：浮点 `0.01` = 每像素 ±0.01、整数 `1`；`≤ 0` = 不吸附）。
    /// 它也决定**显示小数位**（`0.01` ⇒ 2 位、`0.001` ⇒ 3 位——颜色通道那种更细的场景显式覆盖）。
    pub fn step(mut self, step: T) -> Self {
        self.step = step;
        self
    }

    /// 默认速度倍率（默认 10：细调）。
    pub fn speed(mut self, s: f32) -> Self {
        self.speed = s;
        self
    }

    /// 按住 **Shift** 拖拽的速度倍率（默认 10）。
    pub fn shift_speed(mut self, s: f32) -> Self {
        self.shift_speed = s;
        self
    }

    /// 按住 **Ctrl** 拖拽的速度倍率（默认 0.1）。
    pub fn ctrl_speed(mut self, s: f32) -> Self {
        self.ctrl_speed = s;
        self
    }
}

/// 按 `lo` / `hi` 夹住（`T` 内比较，不走 `f64` 中转 ⇒ 大整数端点也精确）。
fn clamp_to<T: SliderValue + PartialOrd>(v: T, lo: Option<T>, hi: Option<T>) -> T {
    let mut out = v;
    if let Some(lo) = lo
        && out < lo
    {
        out = lo;
    }
    if let Some(hi) = hi
        && out > hi
    {
        out = hi;
    }
    out
}

/// **绕窗（warp）到边缘的判定宽度**（客户区物理像素；`mx >= win_w − EDGE` 即"到右边"）。
const WARP_EDGE: f32 = 1.0;
/// **绕窗落点内缩**（客户区物理像素）：跳到对侧边缘**内侧** `3px`——跳完就不在边缘了
/// （不会连着触发），又足够近（继续拖时值连续）。
const WARP_INSET: f32 = 3.0;

/// [`warp_step`] 的结果。
#[derive(Clone, Copy, Debug, PartialEq)]
struct WarpStep {
    /// 本帧的拖拽基准（**已把"观察到的跨窗跳变"补偿掉**）。
    pm: f64,
    /// 本帧要**请求**的 warp 目标 x（`None` = 不在边缘 / 不请求）。
    target: Option<f32>,
    /// 记给下一帧的"请求点"（= 请求时的 `mx`；`None` = 清空）。
    from: Option<f32>,
}

/// **绕窗一步**（纯函数，可单测）：绕窗拖拽的**全部状态机**都在这里。
///
/// 语义（两条，缺一不可）：
/// 1. **请求**：鼠标到窗口左 / 右边缘就请求把光标挪到对侧内侧（`target`）——这只是**请求**，
///    `set_cursor_position` 要等 OS 事件才生效，甚至可能不生效；
/// 2. **补偿**：只有在**真的观察到**跨窗跳变（`|mx − from| > 半个窗宽`）时，才把拖拽基准
///    平移**同样的量** ⇒ 值**严格连续**、**不重复补偿**。
///
/// ⚠ 为什么不能"请求时顺便补偿"（旧实现）：光标还没挪的那几帧会**每帧再补偿一次** ⇒ 值以
/// "一个窗宽 / 帧"的速度飞走（实机与仿真都能复现）。另外：只有 `from` 存在时才做跳变判定，
/// 所以"快速甩鼠标"这种单帧大位移**不会**被误判成 warp。
fn warp_step(pm: f64, prev_from: Option<f32>, mx: f32, win_w: f32) -> WarpStep {
    let mut pm = pm;
    // ① 观察补偿：上一帧请求过 warp，本帧看到"跳到对侧" ⇒ 基准平移同样的量（值不变）。
    if let Some(from) = prev_from {
        let jump = mx - from;
        if jump.abs() > win_w * 0.5 {
            pm += f64::from(jump);
        }
    }
    // ② 请求：到边缘就把光标挪到对侧**内侧**。只请求、不动基准（挪动了下一帧才补偿）。
    let target = if mx >= win_w - WARP_EDGE {
        Some(WARP_INSET)
    } else if mx <= WARP_EDGE {
        Some(win_w - WARP_INSET)
    } else {
        None
    };
    WarpStep { pm, target, from: target.map(|_| mx) }
}

/// `step` 的**十进制小数位数**（`0.1` → 1、`0.25` → 2、`0.05` → 2、`≥ 1` / `≤ 0` → 0）。
///
/// 与显示 / 回写共用同一口径：`step` 决定了"这个控件认为自己有几位精度"。
fn step_decimals(step: f64) -> usize {
    if !(step > 0.0 && step < 1.0) {
        return 0;
    }
    // 从 0 位往上找**第一个**能把 `step` 表示成"整十进制"的位数：`0.1` → 1、`0.25` → 2、
    // `0.01` → 2、`0.05` → 2、`1e-6` → 6（上限）。
    // ⚠ 容差必须**相对**且宽松到 ~1e-6：`step` 常来自 `f32` 字面量（`0.01f32` 的精确值是
    // `0.009999999776…`），用绝对 `1e-9` 会一路找不到、退化成 6 位小数（实测：`0.01`
    // 显示成 `0.100000`）。
    for dec in 1..=6usize {
        let mag = step * 10f64.powi(dec as i32);
        let tol = mag.abs().max(1.0) * 1e-6;
        if (mag - mag.round()).abs() <= tol {
            return dec;
        }
    }
    6
}

/// 按 `dec` 位十进制**取整**（返回该十进制数的**最邻近 double**）。
///
/// 非有限 / 量级已超出十进制有效位（`|v| ≥ 1e15`）时原样返回——再乘 `10^dec` 只会溢出。
fn round_decimals(v: f64, dec: usize) -> f64 {
    if !v.is_finite() || v == 0.0 || v.abs() >= 1e15 {
        return v;
    }
    let p = 10f64.powi(dec as i32);
    if !p.is_finite() {
        return v;
    }
    (v * p).round() / p
}

/// **吸附到 `step` 格点，再夹到 `[lo, hi]`**（纯函数，可单测）。
///
/// 吸附 = `round(v / step) × step`（格点锚在 **0**：`8.0` / `8.5` …）→ 再按 `step` 的十进制
/// 位数取整（见 [`round_decimals`]）。`step ≤ 0` = 不吸附（连续）。
/// ⚠ 顺序是**先吸附再 clamp**：反过来的话吸附能把值顶出 `max`（`max = 1.0`、`step = 0.4`
/// ⇒ `1.2`）。
fn snap_clamp(v: f64, step: f64, lo: Option<f64>, hi: Option<f64>) -> f64 {
    let snapped = if step > 0.0 {
        round_decimals((v / step).round() * step, step_decimals(step))
    } else {
        v
    };
    let mut out = snapped;
    if let Some(lo) = lo {
        out = out.max(lo);
    }
    if let Some(hi) = hi {
        out = out.min(hi);
    }
    out
}

/// **显示文本**（纯函数，可单测）。
///
/// - 整数类型：无小数（`fmt_text` 自己忽略位数）；
/// - 浮点：值**在 `step` 格点上** ⇒ 按 `step` 的位数显示（`step = 0.5` 的 `8.0` → `"8.0"`）；
///   值**不在格上**（手打进来的 `0.37`）⇒ 如实显示到"能表示它"的最少位数（上限 6 位），
///   绝不糊成 `step` 的位数（那会让输入框**显示的值与实际值不一致**）。
fn display_text<T: SliderValue>(v: T, step: T) -> String {
    if T::is_integral() {
        return v.fmt_text(0);
    }
    let dec = step_decimals(step.to_f64());
    let vf = v.to_f64();
    // "在 `dec` 位小数上" = **按 `T` 自身的精度**看，把值四舍五入到 `dec` 位再装回 `T`
    // 是否还是原值。⚠ 别拿 `f64` 的严格相等去判：`f32` 只有 24 位有效位，
    // `0.1f32`（= 0.10000000149011612）在 `f64` 网格上永远"不在格上"。
    let round_trip = |d: usize| T::from_f64(round_decimals(vf, d)).to_f64();
    if dec > 0 && round_trip(dec) == vf {
        return v.fmt_text(dec);
    }
    if dec == 0 && vf == vf.trunc() {
        return v.fmt_text(0);
    }
    let mut d = dec;
    while d < 6 && round_trip(d) != vf {
        d += 1;
    }
    v.fmt_text(d)
}

impl<T: SliderValue + PartialOrd> Widget for NumberInput<'_, T> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        // ① 申请（尺寸 = 主题固定值；手柄与文本框的切分见下）
        let rect = ui.allocate(Vec2::new(ui.theme().input.min_w, ui.theme().input.height));
        // 被裁剪层完全剔除 ⇒ 直接 return（不镶嵌、不入段：scissor 只省片元）。
        if ui.culled(rect) {
            return Response { rect, culled: true, ..Default::default() };
        }
        let id_for = ui.id_for(self.id);
        let focused = ui
            .state()
            .focused
            .as_ref()
            .is_some_and(|f| f.as_str() == id_for.as_str());

        // 手柄宽度：16px 太窄（`≡` 几乎贴边、并与文本框的圆角打架），20px 更从容。
        // ⚠ 公开常量：脚本化测试（`--sim-tuner`）要能算出"手柄在哪一格"——
        // 写死 20 会在改宽度后点空，且点空**看起来像"控件不响应"**，很难查。
        let grip = Rect::new(rect.x + rect.w - GRIP_W, rect.y, GRIP_W, rect.h);
        // `RJ_NUM_TRACE=1`：打印**矩形切分**与拖拽状态机（见 `docs/DEBUGGING.md`）。
        // 为什么专门要这一条：数字条"点上去没反应"有两种完全不同的成因——
        // ① 点在**文本框**上（那是"进入编辑"，本来就不调值）；② `down_edge` 没到
        // （`update_drag` 不开始拖）。这两个数（`手柄 x 区间` / `down_edge`）一眼分开。
        // 坐标是**当前容器局部**（通常 = 行内坐标）；绝对矩形看 `RJ_HIT_TRACE=1`。
        if std::env::var_os("RJ_NUM_TRACE").is_some() {
            eprintln!(
                "num[{}] 局部 rect=({:.0},{:.0},{:.0},{:.0}) 手柄 x={:.0}..{:.0}（宽 {GRIP_W:.0}）",
                id_for.as_str(),
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                grip.x,
                grip.x + grip.w
            );
        }
        let text_rect = Rect::new(rect.x, rect.y, (rect.w - GRIP_W).max(0.0), rect.h);
        let drag_id = IdAbsolute::owned(format!("{}::grip", id_for.as_str()));
        // 取值范围**先拷出来**（`T: Copy`）：下面 `edit_text` 会可变借用 `self.text`，
        // 那时就不能再整块借 `self` 了（字段级分离借用照旧可用）。
        let (lo_v, hi_v) = (self.min, self.max);
        // 手柄是**独立可交互区**（不与文本框重叠）→ 传自己的 id 参与控件级遮挡判定。
        let grip_hit = ui.hit_abs(&drag_id, &grip);
        let btn = ui.mouse_left();
        ui.register_focus(&id_for, rect, FocusKind::TextInput);

        // 显示 / 编辑文本来源：
        // - 外部绑定（`with_text`）→ 直接使用（旧形态）；
        // - 内部管理 → 聚焦时从 `WidgetState::input_text` 取出（首次用 `value` 派生
        //   初始化）；失焦时用 `value` 派生文本（无需存储）。
        let mut owned: Option<String> = None;
        if self.text.is_none() {
            owned = Some(if focused {
                let ws = ui.state_mut().widget(&id_for);
                ws.input_text
                    .take()
                    .unwrap_or_else(|| display_text(*self.value, self.step))
            } else {
                display_text(*self.value, self.step)
            });
        }
        // 统一编辑缓冲：外部绑定或内部 owned（字段分离借用：text 字段 vs value 字段）。
        let edit_text: &mut String = match &mut self.text {
            Some(ext) => ext,
            None => owned.as_mut().expect("internal text initialized"),
        };

        // 输入模式（聚焦）也可拖动手柄调值：手柄在右侧 grip 区（text_rect 之外），
        // 聚焦时点手柄拖动 → 调值并更新编辑缓冲；点文本框 → 打字。两者区域分离。
        let drag_enabled = true;
        if btn.down_edge() && grip_hit {
            ui.claim_press();
        }
        let mut dragging = false;
        // 先拷出鼠标/窗口尺寸（ws 借用期间不能再借 ui）
        let mx = ui.mouse_screen().x;
        let my = ui.mouse_screen().y;
        let win_w = ui.window_physical_size().0 as f32;
        // warp 待办（ws 释放后执行 set_cursor_position）
        let mut warp_to: Option<(f32, f32)> = None;
        // 拖拽数学一律 `f64`（见模块文档"精度"）：`step` / 位置增量 / 吸附都在这里换算。
        let step_f = self.step.to_f64();
        let lo_f = lo_v.map(SliderValue::to_f64);
        let hi_f = hi_v.map(SliderValue::to_f64);
        if drag_enabled {
            // 速度倍率：Shift = 细调（默认 ×10），Ctrl = 精调（默认 ×0.1）。
            let speed = if ui.key_down(winit::keyboard::KeyCode::ShiftLeft)
                || ui.key_down(winit::keyboard::KeyCode::ShiftRight)
            {
                self.shift_speed
            } else if ui.key_down(winit::keyboard::KeyCode::ControlLeft)
                || ui.key_down(winit::keyboard::KeyCode::ControlRight)
            {
                self.ctrl_speed
            } else {
                1.0
            } * self.speed;
            let speed_f = f64::from(speed);
            let ws = ui.state_mut().widget(&drag_id);
            dragging = update_drag(ws, grip_hit, btn);
            if std::env::var_os("RJ_NUM_TRACE").is_some() {
                eprintln!(
                    "num-grip[{}] 命中={grip_hit} down_edge={} 按住={} 拖拽中={dragging} 屏幕x={mx:.0} 值={}",
                    id_for.as_str(),
                    btn.down_edge(),
                    btn.pressed(),
                    display_text(*self.value, self.step)
                );
            }
            if btn.down_edge() && grip_hit {
                // 拖拽基准：物理 x + 起始值（`f32` 容器只借用，值本身在 `f64` 里算）。
                ws.press_mouse = Some(Vec2::new(mx, 0.0));
                ws.press_panel = Some(Vec2::new(0.0, self.value.to_f64() as f32));
            }
            if dragging {
                let mut pm = f64::from(ws.press_mouse.unwrap_or(Vec2::new(mx, 0.0)).x);
                let mut base = f64::from(ws.press_panel.unwrap_or(Vec2::ZERO).y);
                // 速度（Shift/Ctrl）变化 → 重设拖拽基准：从**当前值**继续增量，
                // 避免 `Δx × 新 speed` 使值瞬间跳变。
                let prev_sens = ws.drag_sens;
                if prev_sens != 0.0 && prev_sens != speed {
                    pm = f64::from(mx);
                    base = self.value.to_f64();
                    ws.press_mouse = Some(Vec2::new(mx, 0.0));
                    ws.press_panel = Some(Vec2::new(0.0, base as f32));
                }
                ws.drag_sens = speed;
                // **绕窗（warp）**：鼠标到达窗口左右**边缘**就把光标挪到对侧内侧继续拖
                // （窗口**最大化**时鼠标无法越出窗口，靠"边缘检测"而不是"越出检测"）。
                // 全部状态机在 [`warp_step`] 里（请求 + **观察补偿**）：
                // - **只在真正观察到跨窗跳变时**才平移拖拽基准 ⇒ 值严格连续；
                // - 光标还没挪（`set_cursor_position` 是请求，OS 事件可能晚到甚至不生效）时
                //   基准不动 ⇒ 值停在边缘。旧实现在请求当帧就平移基准 ⇒ 那几帧每帧再平移
                //   一次 ⇒ 值以"一个窗宽 / 帧"飞走（实机与 `--sim-*` 都能复现）。
                let step_res = warp_step(pm, ws.warp_from, mx, win_w);
                if step_res.pm != pm {
                    pm = step_res.pm;
                    ws.press_mouse = Some(Vec2::new(pm as f32, 0.0));
                }
                ws.warp_from = step_res.from;
                if let Some(nx) = step_res.target {
                    warp_to = Some((nx, my));
                }
                let raw = base + (f64::from(mx) - pm) * step_f * speed_f;
                let v = snap_clamp(raw, step_f, lo_f, hi_f);
                *self.value = clamp_to(T::from_f64(v), lo_v, hi_v);
                *edit_text = display_text(*self.value, self.step);
            } else {
                // 松手 / 未拖拽：清掉绕窗的待补偿点（下次拖拽重新开始）。
                ws.warp_from = None;
            }
        }
        if let Some((nx, ny)) = warp_to {
            ui.set_cursor_position(nx, ny);
        }
        // 光标：手柄悬停/拖拽 → ↔（EwResize）；文本框 → 内置 I 型
        if grip_hit || dragging {
            ui.set_cursor(UiCursor::EwResize);
        }
        // 文本框：打字写入 edit_text，随后屏蔽非数字输入并解析回数值。
        // 文本框**只圆左侧两角**（右侧与手柄拼成一条直边，否则文本框自己的圆角会在
        // 手柄左缘留下缺口）：经 [`TextEditor::radius`] 逐控件覆盖圆角——
        // `Size::Physical`（主题圆角在 `Theme::build` 已预乘 scale，不能再乘一次）。
        let in_radius = ui.theme().input.radius;
        let panel_radius = CornerRadius {
            tl: in_radius.tl,
            br: 0.0,
            tr: 0.0,
            bl: in_radius.bl,
        };
        ui.add(
            TextEditor::new(self.id /* 内部处理 id_for */, edit_text)
                .at(text_rect)
                .radius(Size::Physical(panel_radius)),
        );
        // 输入过滤：整数类型连小数点与正号一起屏蔽（打了也解析不了，只会让光标"卡住"）。
        if T::is_integral() {
            edit_text.retain(|c| c.is_ascii_digit() || c == '-' || c == ' ');
        } else {
            edit_text.retain(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == '+' || c == ' ');
        }
        // 输入模式：**仅首次聚焦时全选**（之后可正常用鼠标部分选择文本；
        // 失焦后 focused_prev 复位，下次聚焦再全选）。
        let now_focused = ui
            .state()
            .focused
            .as_ref()
            .is_some_and(|f| f.as_str() == id_for.as_str());
        {
            let ws = ui.state_mut().widget(&id_for);
            let just_focused = now_focused && !ws.focused_prev;
            ws.focused_prev = now_focused;
            if just_focused {
                ws.sel_anchor = Some(0);
                ws.caret = edit_text.chars().count();
            }
        }
        // ⚠ 手打的输入**只 clamp、不吸附**：`step` 是拖动精度（见模块文档"精度"第 4 条）。
        if let Some(v) = T::parse_text(edit_text) {
            *self.value = clamp_to(v, lo_v, hi_v);
        }
        // 内部管理：编辑缓冲写回持久（聚焦时）；失焦清空（显示由 value 派生）。
        if self.text.is_none() {
            let ws = ui.state_mut().widget(&id_for);
            ws.input_text = if now_focused {
                Some(owned.take().unwrap_or_default())
            } else {
                None
            };
        }
        // 拖拽手柄：与文本框**拼成一个控件**——
        // - 只圆**右侧**两角，且半径与输入框同源 ⇒ 与文本框的圆角严丝合缝
        //   （`CornerRadius` 的具名字段在这里正好用上）；
        // - 底色用按钮刷（略高于输入框的"可按"暗示），边框色只作**左缘分隔线**，
        //   不再整块刷成边框色（旧样子像"两个独立的深色方块"）。
        let (grip_bg, sep, glyph, radius) = {
            let st = &ui.theme().input;
            (
                ui.theme().button.bg,
                st.border,
                st.fg,
                CornerRadius { tl: 0.0, tr: st.radius.tr, br: st.radius.br, bl: 0.0 },
            )
        };
        // ⚠ 元素序必须取**当前录制位置**（`elem_hint`）：手柄与分隔线画在文本框**之上**，
        // 写死 `1` 会被文本框（`elem = seq + 1`，更大）整块盖住——历史 bug：
        // 手柄底色 / 分隔线 / `≡` 图标全部看不见，只剩一个普通输入框。
        let elem = ui.elem_hint();
        ui.push_panel_like(grip, grip_bg, sep, 0.0, radius, elem);
        ui.push_panel_like(
            Rect::new(grip.x, grip.y, 1.0, grip.h),
            sep,
            sep,
            0.0,
            CornerRadius::default(),
            elem,
        );
        // 手柄图标用**矢量三横**（`≡` 字形会随字体变宽变高，甚至缺字形）。
        ui.painter().icon_at(
            Position::Physical(Vec2::new(grip.x + (grip.w - 12.0) * 0.5, grip.y + (grip.h - 14.0) * 0.5)),
            Size::Physical(Vec2::new(12.0, 14.0)),
            Icon::Grip,
            glyph,
        );
        Response { rect, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_precision_follows_the_step() {
        // **显示精度必须跟着 step**：旧实现写死 `{:.0}`，0.25 步进显示成整数
        // （观感"step 没生效"）。这张表就是"主题调节"窗口里那几根滑杆 + 数字条的
        // `range/step` 组合——它们现在都带一个数字条，显示错位会立刻被看见。
        assert_eq!(display_text(8.0f32, 0.5), "8.0", "step 0.5 → 1 位小数");
        assert_eq!(display_text(0.1f32, 0.01), "0.10", "step 0.01 → 2 位");
        assert_eq!(display_text(0.35f32, 0.01), "0.35", "细粒度小范围不得被取整成 0");
        assert_eq!(display_text(0.25f32, 0.25), "0.25", "step 不是 10 的整幂 → 多留一位");
        assert_eq!(display_text(1.5f32, 0.05), "1.50", "step 0.05 → 2 位");
        assert_eq!(display_text(0.8f32, 0.05), "0.80");
        // step ≥ 1 → 取整（投影模糊宽这类"整像素"量）。
        assert_eq!(display_text(14.0f32, 1.0), "14");
        assert_eq!(display_text(-0.4f32, 0.5), "-0.4", "负值/接近 0 也按同一精度显示");
        // ⚠ 不在 `step` 位上的值**如实显示**（旧实现会四舍五入成 `step` 的位数，
        // 于是"显示的值 ≠ 实际值"）：吸附路径不会产生 `0.126`（它会变成 `0.13`），
        // 但应用可以自己写进来，或用户手打进来。
        assert_eq!(display_text(0.126f32, 0.01), "0.126", "宁可多显示，也不显示错");
    }

    /// **不在 `step` 格点上的值要如实显示**（手打进来的 `0.37` 不许被糊成 `0.4`）：
    /// 显示与实际值不一致是输入框最招人恨的那种 bug。
    #[test]
    fn off_grid_values_display_honestly() {
        assert_eq!(display_text(0.37f32, 0.1), "0.37", "手打值不被 step 位数糊掉");
        assert_eq!(display_text(0.371f32, 0.1), "0.371");
        // 在格点上的仍然按 step 位数（保持老观感）。
        assert_eq!(display_text(0.4f32, 0.1), "0.4");
        assert_eq!(display_text(0.3f64, 0.1), "0.3");
        // 连续模式（step ≤ 0）：不吸附、也不假装有精度。
        assert_eq!(display_text(13.6f32, 0.0), "13.6", "step ≤ 0 = 连续：如实显示");
    }

    /// 整数类型：显示无小数；**大整数不经过 `f32`**（走 `f64`）。
    #[test]
    fn integer_values_display_without_decimals() {
        assert_eq!(display_text(42i32, 1), "42");
        assert_eq!(display_text(-7i32, 5), "-7", "整数忽略 step 的小数位");
        assert_eq!(display_text(1_000_000_000_000_000i64, 1), "1000000000000000");
        assert_eq!(display_text(255u8, 1), "255");
    }

    /// `step` 的十进制位数口径（显示与回写共用）。
    #[test]
    fn step_decimals_cover_common_steps() {
        assert_eq!(step_decimals(0.1), 1);
        assert_eq!(step_decimals(0.25), 2);
        assert_eq!(step_decimals(0.05), 2);
        assert_eq!(step_decimals(0.01), 2);
        assert_eq!(step_decimals(1.0), 0);
        assert_eq!(step_decimals(5.0), 0);
        assert_eq!(step_decimals(0.0), 0, "连续模式 ⇒ 0 位（不当成'有精度'）");
    }

    /// **吸附 = 格点 + 十进制取整**：写回的是"用户以为的那个十进制数"的最邻近 double,
    /// 于是应用侧 `v == 0.1` 这类比较能成立（旧实现存 `0.30000001…`，`==` 恒 false）。
    #[test]
    fn snapping_lands_on_the_decimal_grid() {
        assert_eq!(snap_clamp(0.30000001, 0.1, None, None), 0.3f64);
        assert_eq!(snap_clamp(0.29999999, 0.1, None, None), 0.3f64);
        assert_eq!(snap_clamp(0.3, 0.1, None, None), 0.3f64);
        assert_eq!(snap_clamp(17.999998, 0.1, None, None), 18.0f64);
        assert_eq!(snap_clamp(8.24, 0.25, None, None), 8.25f64);
        assert_eq!(snap_clamp(8.1, 0.5, None, None), 8.0f64);
        // 格点锚在 0：`0.3..=0.9` 配 `step = 0.5` ⇒ 0.5（不是 0.3 / 0.8）。
        assert_eq!(snap_clamp(0.6, 0.5, Some(0.3), Some(0.9)), 0.5f64);
        // `step ≤ 0` = 不吸附。
        assert_eq!(snap_clamp(0.37, 0.0, None, None), 0.37f64);
    }

    /// **先吸附、再 clamp**（旧顺序会把值顶出 `max`）：`max = 1.0`、`step = 0.4`。
    #[test]
    fn snapping_never_escapes_the_range() {
        // 先 clamp 再吸附的老实现：round(1.0 / 0.4) × 0.4 = 3 × 0.4 = 1.2 > max。
        assert_eq!(snap_clamp(1.0, 0.4, Some(0.0), Some(1.0)), 1.0f64);
        assert_eq!(snap_clamp(9.9, 0.4, Some(0.0), Some(1.0)), 1.0f64);
        assert_eq!(snap_clamp(-3.0, 0.4, Some(0.0), Some(1.0)), 0.0f64);
        // 端点不在格上 ⇒ 结果是端点本身（不在格上，但**不越界**）。
        assert_eq!(snap_clamp(0.9, 0.25, Some(0.0), Some(0.9)), 0.9f64);
    }

    /// **大数值 + 小步进**：`f32` 会丢位（`raw / step` 到 1e5 后就只剩 ~1 位小数分辨率），
    /// `f64` 照常吸附。这条是"内部数学用 f64"的存在理由。
    #[test]
    fn large_values_with_small_steps_keep_snapping() {
        // 1000.0 + 0.01 的格点：f32 下 1000.01 已经没有可分辩的格点（间距 6e-5 > 0.01? 不，
        // f32 在 1000 附近的间距 ≈ 6.1e-5 —— 0.01 级格点**勉强**可用；到 1e5 就彻底不行）。
        assert_eq!(snap_clamp(100_000.004, 0.01, None, None), 100_000.0f64);
        assert_eq!(snap_clamp(100_000.006, 0.01, None, None), 100_000.01f64);
        // f32 版本：1e5 附近间距 = 0.0078，`(v / 0.01).round() * 0.01` 已开始跳格。
        let f32_grid = (((1.0e5f32 + 0.004) / 0.01f32).round() * 0.01) as f64;
        assert_ne!(f32_grid, 100_000.0f64, "f32 在这量级已经吸不准（f64 才对）");
    }

    /// 整数类型：默认步进 1、饱和到类型边界、显示无小数。
    #[test]
    fn integer_number_input_semantics() {
        assert_eq!(<i32 as SliderValue>::default_step(), 1);
        assert_eq!(snap_clamp(3.6, 1.0, None, None), 4.0f64, "整数默认一格一格");
        assert_eq!(<u8 as SliderValue>::from_f64(snap_clamp(300.0, 1.0, None, None)), 255u8);
        assert_eq!(<i32 as SliderValue>::from_f64(snap_clamp(-2.5, 1.0, None, None)), -3i32);
    }

    // ─── 绕窗（warp）拖拽 ────────────────────────────────────────
    //
    // 曾经的 BUG：请求 warp 的**当帧**就平移拖拽基准 ⇒ 光标还没被挪到对侧的那几帧
    // **每帧再平移一次** ⇒ 值以"一个窗宽 / 帧"飞走（`--sim-*` 与实机都能复现）。
    // 现在：只**请求**，等**观察到**跨窗跳变才补偿（补偿量与跳变量完全相同）。

    /// 请求 warp：到边缘只**请求**、不动基准；记下请求点给下一帧。
    #[test]
    fn warp_requests_at_the_edge_without_moving_the_base() {
        let (pm, win_w) = (100.0f64, 1920.0f32);
        // 右边缘：请求跳到左侧内侧 3px。
        let s = warp_step(pm, None, 1919.0, win_w);
        assert_eq!(s.pm, pm, "请求阶段不许动基准（这正是旧 BUG）");
        assert_eq!(s.target, Some(WARP_INSET));
        assert_eq!(s.from, Some(1919.0), "记下请求点");
        // 左边缘：请求跳到右侧内侧。
        let s = warp_step(pm, None, 0.0, win_w);
        assert_eq!(s.pm, pm);
        assert_eq!(s.target, Some(win_w - WARP_INSET));
        assert_eq!(s.from, Some(0.0));
        // 中间：不请求、也清空待补偿点。
        let s = warp_step(pm, Some(1919.0), 1000.0, win_w);
        assert_eq!(s.pm, pm, "没看到跨窗跳变 ⇒ 不补偿");
        assert_eq!(s.target, None);
        assert_eq!(s.from, None);
    }

    /// **光标还没挪**（同一 `mx` 连续几帧）：基准**不动** ⇒ 值停在边缘，不飞走。
    #[test]
    fn warp_holds_the_value_while_the_cursor_has_not_moved() {
        let (pm, win_w) = (100.0f64, 1920.0f32);
        let mut pm = pm;
        let mut from = None;
        // 第 1 帧：到边缘 → 请求。
        let s = warp_step(pm, from, 1919.0, win_w);
        (pm, from) = (s.pm, s.from);
        assert_eq!((pm, s.target), (100.0, Some(WARP_INSET)));
        // 之后 10 帧光标仍停在 1919（OS 事件还没到 / 不生效）⇒ 基准与值都不变。
        for i in 0..10 {
            let s = warp_step(pm, from, 1919.0, win_w);
            assert_eq!(s.pm, pm, "第 {i} 帧不许再补偿（旧 BUG：这里每帧累加一个窗宽）");
            (pm, from) = (s.pm, s.from);
        }
        assert_eq!(pm, 100.0);
    }

    /// **真的跳了才补偿**：补偿量 = 跳变量 ⇒ 值**严格连续**（不多不少），且只补一次。
    #[test]
    fn warp_compensates_the_observed_jump_exactly_once() {
        let (pm, win_w) = (100.0f64, 1920.0f32);
        // 请求点在 1919，下一帧观察到跳到 3 ⇒ `pm += (3 - 1919)`。
        let s = warp_step(pm, Some(1919.0), 3.0, win_w);
        assert_eq!(s.pm, pm + 3.0 - 1919.0);
        assert_eq!(s.target, None, "3 不在边缘 ⇒ 不重复请求");
        assert_eq!(s.from, None, "补偿过就清空");
        // 再调一次（已无 from）⇒ 不再补偿。
        let s2 = warp_step(s.pm, s.from, 4.0, win_w);
        assert_eq!(s2.pm, s.pm);
        // 快速甩鼠标（没有待补偿点）⇒ 单帧大位移**不**被当成 warp。
        let s3 = warp_step(pm, None, 1500.0, win_w);
        assert_eq!(s3.pm, pm);
    }

    /// **端到端序列**（用与 `ui()` 相同的公式）：拖到右边缘 → 光标没挪（停 5 帧）→
    /// 观察到跳到对侧 → 继续右拖。断言：值**单调不减**、边缘处**冻结**、
    /// 跳变前后**严格相等**、跳完继续涨。
    #[test]
    fn warp_drag_sequence_is_monotone_and_continuous() {
        let win_w = 1920.0f32;
        let (step, speed, base) = (0.5f64, 1.0f64, 0.0f64);
        let mut pm = 0.0f64; // 按下时的鼠标 x（客户区）
        let mut from: Option<f32> = None;
        let mut last = base;
        let mut edge_value: Option<f64> = None;
        let mut after_jump: Option<f64> = None;
        // 帧序列：走到 1919（边缘）→ 原地停 5 帧 → 跳到 3 → 继续到 20。
        // 索引：0=100、1=900、2=1919（首碰边缘）、3..=7=1919（光标没挪）、8=3（观察到跳变）、9=5、10=20。
        let positions: Vec<f32> = [100.0f32, 900.0, 1919.0]
            .into_iter()
            .chain(std::iter::repeat_n(1919.0, 5))
            .chain([3.0f32, 5.0, 20.0])
            .collect();
        for (i, mx) in positions.iter().copied().enumerate() {
            let s = warp_step(pm, from, mx, win_w);
            (pm, from) = (s.pm, s.from);
            // 与 `ui()` 里同一套：raw → 吸附（不设 range）。
            let v = snap_clamp(base + (f64::from(mx) - pm) * step * speed, step, None, None);
            assert!(v >= last, "第 {i} 帧值回退：{last} → {v}（mx={mx}）");
            last = v;
            match i {
                2 => edge_value = Some(v), // 刚碰到边缘那一帧
                8 => after_jump = Some(v), // 观察到跳变、补偿后的那一帧
                9..=10 => {
                    assert!(
                        v > after_jump.expect("第 8 帧已记录"),
                        "跳完必须继续涨（第 {i} 帧 {v}）"
                    );
                }
                _ => {}
            }
        }
        // 边缘值 = 按下点 0 → 1919，每像素 0.5 ⇒ 959.5（吸附到 0.5 的格点）。
        assert_eq!(edge_value, Some(959.5));
        // 跳变前后**严格相等**（"补偿量 = 跳变量"的直接结果；旧实现要么飞走、要么差一个边缘宽度）。
        assert_eq!(after_jump, edge_value, "绕窗后值必须接得上");
    }

    /// **拖拽序列**：`step = 0.1` 每像素 +0.1，走 30 步——
    /// ① 每一步都必须**正好**是该十进制的"最邻近 double"（`k / 10` 的最近舍入），
    ///    于是应用侧 `v == 0.3` 这类比较成立、`v * 10` 是整数；
    /// ② 序列**不累积漂移**（每步都是 `base + 总位移 × step` 的绝对量，不是逐帧累加）。
    #[test]
    fn drag_sequence_with_step_0_1_stays_on_the_decimal_grid() {
        let step = 0.1f64;
        let (pm, base) = (0.0f64, 0.0f64); // 按下时的鼠标 x / 起始值
        for i in 1..=30u32 {
            let mx = f64::from(i); // 第 i 个物理像素
            let v = snap_clamp(base + (mx - pm) * step, step, None, None);
            let want = f64::from(i) / 10.0;
            assert_eq!(v, want, "第 {i} 步应正好是 {want}");
            assert_eq!(v * 10.0, f64::from(i), "乘 10 必须是整数（无尘埃位）");
            // 存进 `f32` 也一样（`3.0 / 10.0` 与字面量 `0.3` 都是"最近舍入"⇒ 相等）。
            let stored = <f32 as SliderValue>::from_f64(v);
            assert_eq!(stored, i as f32 / 10.0);
            // 反例（旧实现的路）：`round(raw / step) * step` 会留下尘埃位 ⇒ 存的是
            // `0.30000000000000004`，应用侧 `v == 0.3` 恒 false。十进制取整后才是 `0.3`。
            if i == 3 {
                let old = (v / step).round() * step;
                assert_ne!(old, 0.3f64, "旧路：0.1 的三步 = 0.30000000000000004");
                assert_ne!(old, v, "新旧必须不同（否则这条测试没守住任何东西）");
                assert_eq!(v, 0.3f64);
            }
        }
    }
}
