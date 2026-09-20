//! 绘制原语：屏幕固定变换 + 实心矩形 / 边框（记录式，实际提交在 `Ui::finish`）。
//!
//! 屏幕固定变换数学：`{ pos: anchor_px, rotation: 0, scale: 1 }`——**UI 层渲染器的坐标空间
//! 就是"物理像素、左上原点"**（运行时给它一个平移了 `region.center()` 的 identity 相机，
//! 见 `Ctx::submit_with`），所以局部像素点经本变换即 1:1 落在屏幕上
//! （不随世界相机旋转/缩放而变形）。
//!
//! 与"世界层"的区别：世界层相机由应用持有（可平移/旋转/缩放），UI 层相机由运行时固定。

use glam::Vec2;
use rjw_2d_render::SpriteRect;
use rjw_color::Color;
use rjw_text::Buffer;
use rjw_transform::{Rect, Transform2D};
use std::sync::Arc;

/// 屏幕固定变换：把屏幕像素锚点映射为 **UI 层**中的 `Transform2D`。
///
/// UI 层坐标空间 = 物理像素、左上原点 ⇒ 本变换就是"平移到 `anchor_px`"。
#[inline]
pub fn screen_fixed_tf(anchor_px: Vec2) -> Transform2D {
    Transform2D::IDENTITY.with_pos(anchor_px)
}

/// 屏幕矩形 → 精灵矩形（mesh 局部坐标从 (0,0) 起，尺寸 = 矩形宽高）。
#[inline]
pub fn rect_sprite(rect: &Rect) -> SpriteRect {
    SpriteRect::new(Vec2::ZERO, (rect.w, rect.h))
}

/// **屏幕像素取整**（pixel snapping）：左上角与右下角分别四舍五入到整数像素，
/// 保证取整后矩形不缩水（面积 ≥ 原矩形）。在提交绘制前对**物理像素**矩形调用，
/// 避免高 DPI / 非整数布局下出现半像素采样导致的边缘模糊与闪烁。
#[inline]
pub fn snap_rect(r: &Rect) -> Rect {
    let x0 = r.x.round();
    let y0 = r.y.round();
    let x1 = (r.x + r.w).round();
    let y1 = (r.y + r.h).round();
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// **居中正方形**（边长 = `min(w, h)`）：矢量图标（[`Icon`]）的定义域是**单位方框**，
/// 直接映射到非方形框会把笔画拉扁（`row` 的等高约束就会给出 18×26 这类框）。
#[inline]
pub(crate) fn centered_square(r: Rect) -> Rect {
    let side = r.w.min(r.h);
    Rect::new(r.x + (r.w - side) * 0.5, r.y + (r.h - side) * 0.5, side, side)
}

/// **物理 / 逻辑单位包装**（DPI 边界类型）。
///
/// 内部计算一律使用**物理像素**（渲染取整 / 命中 / 滚动都在物理侧），逻辑单位只在
/// 公开 API 边界换算一次。对外参数若允许用户二选一，用本类型接收：
///
/// ```no_run
/// # use rjw_ui::draw::Metric;
/// let m: Metric<f32> = Metric::Logical(40.0);
/// let px = m.to_physical(1.5); // 60.0 物理像素
/// # let _ = px;
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Metric<T> {
    /// 已按 DPI 换算的物理像素值。
    Physical(T),
    /// 逻辑像素值（内部 × scale 换算为物理）。
    Logical(T),
}

impl Metric<f32> {
    /// 换算为**物理像素**（`Logical` 值 × `scale`；`Physical` 原样）。
    #[inline]
    pub fn to_physical(self, scale: f32) -> f32 {
        match self {
            Metric::Physical(v) => v,
            Metric::Logical(v) => v * scale,
        }
    }
}

// ─── Size / Position：公开 API 的带单位参数（逻辑 / 物理） ────────

/// **带单位的尺寸**（公开 API 参数）：`Size<f32>` 标量（宽 / 高 / 半径 / 字号…）、
/// `Size<Vec2>` 向量（尺寸对）。默认 `From` 为 [`Size::Logical`]——现有调用
/// `width(220.0)` / `view_size(vec2(…))` 源码兼容；需物理像素时显式 `Size::Physical(..)`。
///
/// 换算在 **API 边界**进行一次（[`Size::to_physical`]），Ui 内部以物理像素为单位
/// （布局 / 命中 / 绘制零 scale 换算）。`Logical` 换算会**取整**（`(v×scale).round()`，
/// 保布局整数不变量）。
///
/// # 单位纪律（API 实现者必读；**内置控件与用户自定义控件同一套规矩**）
///
/// `Logical` / `Physical` 是**调用点的选择**；写进实现体后必须**显式**对待它。
/// 违反本纪律**不会编译失败**——只会静默错位（漏乘 / 多乘 `scale`），所以它是纪律而不是类型。
/// 三条：
///
/// 1. **解释必须显式**：把带单位值变成数值（布局 / 命中 / 绘制）时，只能用
///    [`to_physical(scale)`](Size::to_physical) 或 `match`，而且要**紧邻**参数解包：
///    `let w = w.into().to_physical(scale);`。禁止 `let w = w.into(); … w.0`
///    ——读 `.0` 的那一刻单位就丢了（默认 `Logical` 被当成物理像素用）。
/// 2. **构造必须指名单位**：函数体内造值一律写 `Size::Logical(..)` / `Size::Physical(..)`，
///    不得用隐式糖（`220.0.into()`）。**主题值 / 跨帧持久化值 / 已经乘过 DPI 的值一律
///    `Physical`**：`Theme` 在 `Ui` 内部就已被 DPI 预乘，控件复用主题值时必须显式声明
///    "这是物理像素"（见 `Button::resolve` 里 `.map(|s| s.to_physical(scale))` 与
///    `unwrap_or(base.font_size)` 的对照）。
/// 3. **转发是唯一的例外**：签名收 `impl Into<Size<..>>` / 存 `Option<Size<..>>` 的 setter
///    可以 `self.font_size = Some(s.into());` 把**调用者的选择**原样存起来——这里不做单位
///    解释，解释推迟到第 1 条的边界（`Button::font_size` 存 `Option<Size<f32>>`，
///    `resolve()` 处才 `to_physical`）。除此之外实现体内不出现隐式糖。
///
/// 明确的物理像素值也可以在**调用点**写 `Size::Physical(..)`（如 `Size::Physical(16.0)`
/// 的字号）；反过来，`From<f32>` / `From<Vec2>`（⇒ `Logical`）只是给调用点的源码兼容
/// **糖**（`width(220.0)`），不是实现者的工具——它成立的前提正是上面三条被遵守。
///
/// ```
/// # use rjw_ui::draw::Size;
/// // ✅ 边界显式换算（Ui 内部一律物理像素）
/// fn width(v: impl Into<Size<f32>>, scale: f32) -> f32 {
///     let v = v.into().to_physical(scale);
///     v.max(0.0)
/// }
/// // ✅ 显式 match：逻辑 / 物理各自处理（栏宽那种要立刻拿到数值的场景）
/// fn exact(v: impl Into<Size<f32>>, scale: f32) -> f32 {
///     match v.into() {
///         Size::Logical(x) => x * scale,
///         Size::Physical(x) => x,
///     }
/// }
/// # let _ = (width(220.0, 1.5), exact(Size::Physical(8.0), 1.5));
/// ```
///
/// ```compile_fail
/// # use rjw_ui::draw::Size;
/// // ⛔ 只收显式单位的方法**不收裸数字**：这正是"单位必须在实现体可见"的机器可查锚点。
/// //    （裸数字只对 `impl Into<Size<f32>>` 这类调用点糖生效；若哪天有人把签名放宽成
/// //    `impl Into<..>`，本 doctest 会失效并提醒同步代码与文档。）
/// fn only_explicit(w: Size<f32>) -> f32 {
///     w.to_physical(1.5)
/// }
/// only_explicit(220.0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size<T> {
    /// 逻辑像素（× scale 换算为物理，取整）。
    Logical(T),
    /// 物理像素（原样）。
    Physical(T),
}

impl Size<f32> {
    #[inline]
    pub fn to_physical(self, scale: f32) -> f32 {
        match self {
            Size::Physical(v) => v,
            Size::Logical(v) => (v * scale).round(),
        }
    }
}

impl Size<CornerRadius> {
    /// 换算为**物理像素**的圆角（`Logical` 四角 × scale 取整；`Physical` 原样）。
    #[inline]
    pub fn to_physical(self, scale: f32) -> CornerRadius {
        match self {
            Size::Physical(v) => v,
            Size::Logical(v) => v.scaled_rounded(scale),
        }
    }
}

// ─── 圆角半径（四角可各自独立） ───────────────────────────────

/// **四角各自独立的圆角半径**（逻辑像素；0 = 直角）。
///
/// 常见用法是"只圆上面两个角"（标签页 / 附着在工具栏下方的面板 / 气泡尖角）或
/// "只圆外上角"（折叠面板首项）：
///
/// ```no_run
/// # use rjw_ui::draw::CornerRadius;
/// # use rjw_ui::{Theme, PanelStyle};
/// let tab = CornerRadius { tl: 8.0, tr: 8.0, br: 0.0, bl: 0.0 };
/// let style = PanelStyle::default().with_radius(tab);
/// let theme = Theme::dark().with_radius(CornerRadius::all(6.0));
/// # let _ = (style, theme);
/// ```
///
/// `From<f32>` ⇒ **四角相同**，所以 `with_radius(8.0)` 这类既有调用点不用改。
///
/// ⚠ 与 [`DrawKind::RoundedRect`] 的 `corners: [Color; 4]` **顺序不同**：颜色数组是
/// `[TL, TR, BL, BR]`（历史约定，与四边形顶点一致），本类型是**具名字段**故无歧义。
/// 镶嵌器内部按屏幕顺时针 TL → TR → BR → BL 遍历。
/// **圆角半径的可反序列化表示**（手写主题文件两种写法都要能收）：
/// `radius = 3.0`（四角同值）或 `radius = { tl = 3.0, tr = 0.0, … }`（缺角回落 `0`）。
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum CornerRadiusRepr {
    Scalar(f32),
    Corners {
        #[serde(default)]
        tl: f32,
        #[serde(default)]
        tr: f32,
        #[serde(default)]
        br: f32,
        #[serde(default)]
        bl: f32,
    },
}

#[cfg(feature = "serde")]
impl From<CornerRadiusRepr> for CornerRadius {
    fn from(r: CornerRadiusRepr) -> Self {
        match r {
            CornerRadiusRepr::Scalar(v) => CornerRadius::all(v),
            CornerRadiusRepr::Corners { tl, tr, br, bl } => CornerRadius { tl, tr, br, bl },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default, from = "CornerRadiusRepr")
)]
pub struct CornerRadius {
    pub tl: f32,
    pub tr: f32,
    pub br: f32,
    pub bl: f32,
}

impl Default for CornerRadius {
    #[inline]
    fn default() -> Self {
        Self::all(0.0)
    }
}

impl From<f32> for CornerRadius {
    #[inline]
    fn from(r: f32) -> Self {
        Self::all(r)
    }
}

/// 与标量比较（**四角相同**才算相等）——让 `assert_eq!(style.radius, 8.0)` 这类
/// 断言与用户代码可直接写；四角不同的值不等于任何标量（更不会误判为"没设圆角"）。
impl PartialEq<f32> for CornerRadius {
    #[inline]
    fn eq(&self, other: &f32) -> bool {
        self.uniform() == Some(*other)
    }
}

impl From<(f32, f32, f32, f32)> for CornerRadius {
    /// `(tl, tr, br, bl)`——与字段顺序一致（**不是**颜色数组的 `[TL,TR,BL,BR]`）。
    #[inline]
    fn from((tl, tr, br, bl): (f32, f32, f32, f32)) -> Self {
        Self { tl, tr, br, bl }
    }
}

impl CornerRadius {
    /// 四角相同。
    #[inline]
    pub const fn all(r: f32) -> Self {
        Self { tl: r, tr: r, br: r, bl: r }
    }    /// 四角相同则返回该值（否则 `None`）——用于"能否走更省的路径"之类的判断。
    #[inline]
    pub fn uniform(self) -> Option<f32> {
        (self.tl == self.tr && self.tl == self.br && self.tl == self.bl).then_some(self.tl)
    }
    /// 四角全为 0（纯直角）。
    #[inline]
    pub fn is_zero(self) -> bool {
        self.tl <= 0.0 && self.tr <= 0.0 && self.br <= 0.0 && self.bl <= 0.0
    }
    /// 最大 / 最小角半径（镶嵌时用最大角定弧的段数，取最细）。
    #[inline]
    pub fn max(self) -> f32 {
        self.tl.max(self.tr).max(self.br).max(self.bl)
    }
    /// 最小角半径。
    #[inline]
    pub fn min(self) -> f32 {
        self.tl.min(self.tr).min(self.br).min(self.bl)
    }
    /// 逐角变换。
    #[inline]
    pub fn map(self, f: impl Fn(f32) -> f32) -> Self {
        let (tl, tr, br, bl) = (self.tl, self.tr, self.br, self.bl);
        Self { tl: f(tl), tr: f(tr), br: f(br), bl: f(bl) }
    }
    /// 逐角缩放（不取整）——DPI 物理化用。
    #[inline]
    pub fn scaled(self, s: f32) -> Self {
        self.map(|r| r * s)
    }
    /// 逐角缩放并**取整**（`Size::Logical` 的 DPI 换算，与其它尺寸字段一致）。
    #[inline]
    pub fn scaled_rounded(self, s: f32) -> Self {
        self.map(|r| (r * s).round())
    }
    /// 逐角 clamp 到 `[0, limit]`。
    #[inline]
    pub fn clamped(self, limit: f32) -> Self {
        self.map(|r| r.clamp(0.0, limit.max(0.0)))
    }
    /// **按 CSS `border-radius` 的规则收缩**，使四角互不重叠地放进 `(w, h)` 的盒子。
    ///
    /// 两条边上的半径之和不得超过该边长：`tl + tr ≤ w`、`bl + br ≤ w`、
    /// `tl + bl ≤ h`、`tr + br ≤ h`。若违反，**四角按同一比例缩小**（保持相对比例），
    /// 而不是各自独立 clamp（后者会让"大圆角"变成"圆角被削平"，形状会突变）。
    #[inline]
    pub fn fit(self, w: f32, h: f32) -> Self {
        let f = self.fit_factor(w, h);
        if f >= 1.0 {
            return self;
        }
        self.scaled(f)
    }

    /// [`Self::fit`] 用的收缩比例（`≥ 1` 表示无需收缩）。
    #[inline]
    pub fn fit_factor(self, w: f32, h: f32) -> f32 {
        let mut f = 1.0f32;
        let mut edge = |a: f32, b: f32, len: f32| {
            let s = a + b;
            if s > 0.0 && len > 0.0 {
                f = f.min(len / s);
            }
        };
        edge(self.tl, self.tr, w);
        edge(self.bl, self.br, w);
        edge(self.tl, self.bl, h);
        edge(self.tr, self.br, h);
        f
    }
}

impl Size<Vec2> {
    #[inline]
    pub fn to_physical(self, scale: f32) -> Vec2 {
        match self {
            Size::Physical(v) => v,
            Size::Logical(v) => (v * scale).round(),
        }
    }
}

impl Default for Size<f32> {
    fn default() -> Self {
        Size::Logical(0.0)
    }
}

impl Default for Size<Vec2> {
    fn default() -> Self {
        Size::Logical(Vec2::ZERO)
    }
}

/// 调用点糖（裸数字 ⇒ **逻辑像素**）。**实现体内部禁止使用**：见 [`Size`] 的「单位纪律」。
impl From<f32> for Size<f32> {
    #[inline]
    fn from(v: f32) -> Self {
        Size::Logical(v)
    }
}

/// 调用点糖（裸 `Vec2` ⇒ **逻辑像素**）。**实现体内部禁止使用**：见 [`Size`] 的「单位纪律」。
impl From<Vec2> for Size<Vec2> {
    #[inline]
    fn from(v: Vec2) -> Self {
        Size::Logical(v)
    }
}

/// `Size<CornerRadius>` 的构造糖：`Button::radius(6.0)` / `radius(CornerRadius { .. })`
/// 都按**逻辑像素**处理（需物理像素时显式 `Size::Physical(..)`）。
/// **实现体内部禁止使用**：见 [`Size`] 的「单位纪律」。
impl From<CornerRadius> for Size<CornerRadius> {
    #[inline]
    fn from(v: CornerRadius) -> Self {
        Size::Logical(v)
    }
}

/// 调用点糖（裸数字 ⇒ 逻辑像素的**四角同值**圆角）。**实现体内部禁止使用**：
/// 见 [`Size`] 的「单位纪律」。
impl From<f32> for Size<CornerRadius> {
    #[inline]
    fn from(v: f32) -> Self {
        Size::Logical(CornerRadius::all(v))
    }
}

/// **带单位的位置**（公开 API 参数，默认 `Vec2`）：`Position` / `Position<Vec2>`。
/// 语义与换算同 [`Size`]（[`Position::to_physical`]）；`From<Vec2>` 默认 [`Position::Logical`]。
///
/// 单位纪律与 [`Size`] 的「单位纪律」**同一套**（解释显式 / 构造指名 / 只允许转发这一种隐式）：
/// API 实现体内把位置变成数值时用 `to_physical(scale)` 或 `match`，禁止靠默认 `Logical` 猜。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Position<T = Vec2> {
    Logical(T),
    Physical(T),
}

impl Position<Vec2> {
    #[inline]
    pub fn to_physical(self, scale: f32) -> Vec2 {
        match self {
            Position::Physical(v) => v,
            Position::Logical(v) => (v * scale).round(),
        }
    }
}

impl Position<f32> {
    #[inline]
    pub fn to_physical(self, scale: f32) -> f32 {
        match self {
            Position::Physical(v) => v,
            Position::Logical(v) => (v * scale).round(),
        }
    }
}

impl Default for Position {
    fn default() -> Self {
        Position::Logical(Vec2::ZERO)
    }
}

/// 调用点糖（裸 `Vec2` ⇒ **逻辑像素**）。**实现体内部禁止使用**：见 [`Position`] 的单位纪律。
impl From<Vec2> for Position {
    #[inline]
    fn from(v: Vec2) -> Self {
        Position::Logical(v)
    }
}

/// 调用点糖（裸数字 ⇒ **逻辑像素**）。**实现体内部禁止使用**：见 [`Position`] 的单位纪律。
impl From<f32> for Position<f32> {
    #[inline]
    fn from(v: f32) -> Self {
        Position::Logical(v)
    }
}


/// 屏幕像素点取整（锚点 / 文本位置用）。
#[inline]
pub fn snap_point(p: Vec2) -> Vec2 {
    Vec2::new(p.x.round(), p.y.round())
}

/// **文本块内容对齐偏移**（屏幕像素，UI"浮点整数"不变量的一部分）。
///
/// 水平：左 `0`、中 `-round(content_w/2)`、右 `-content_w`；
/// 垂直：`Top` = `-first_line_top`（行盒顶对齐矩形顶），
///       `Center` = `-first_line_top - round(content_h/2)`（行盒垂直居中）。
///
/// 前置条件（调用方保证）：`content`（排版内容**物理**尺寸，`measure_buffer` 已取整）
/// 与 `first_line_top`（行盒顶相对视觉原点，`rjw_text` 收集期已取整）均为**整数像素**。
/// 因此本函数内部全部加/减法操作数都是整数 —— 与整数锚点相加
/// （`block_tl = anchor + off`）时两侧均为整数，结果恒为整数：
/// 小数（如 0.5px 的居中奇数宽 / 行盒偏移）只在 `round` 边界被消化，
/// 不会流入加法链 → 无误差累加、无亚像素摆放。
#[inline]
pub fn text_block_offset(align: TextAlign, valign: TextVAlign, content: Vec2, first_line_top: f32) -> Vec2 {
    let off_x = match align {
        TextAlign::Left => 0.0,
        TextAlign::Center => -(content.x * 0.5).round(),
        TextAlign::Right => -content.x,
    };
    let off_y = match valign {
        TextVAlign::Top => -first_line_top,
        TextVAlign::Center => -first_line_top - (content.y * 0.5).round(),
    };
    Vec2::new(off_x, off_y)
}

/// 两矩形求交（裁剪用；无交集返回 `None`）。纯函数（可单测）。
#[inline]
pub fn intersect_rect(a: &Rect, b: &Rect) -> Option<Rect> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    if x1 > x0 && y1 > y0 {
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    } else {
        None
    }
}

// ── 注意：`clipped`（逐命令几何求交）已删除 ──
//
// 环境裁剪层不再切割几何，而是作为 batch scissor 交给 GPU（`UiBatch::clip` +
// `rjw_2d_render::Draw2D::scissor`）：圆角 / 环带 / 投影保持原形，只被裁掉越界像素。
// 保留的"矩形求交"工具是 [`intersect_rect`]（布局 / 命中 / 沙箱裁剪层仍在用）。

/// 边框四边（画在矩形内边缘；宽度 <= 0 或 宽度 >= 半尺寸时退化）。
pub fn border_rects(rect: &Rect, width: f32) -> [Rect; 4] {
    let w = width.max(0.0);
    let hw = rect.w * 0.5;
    let hh = rect.h * 0.5;
    let w = w.min(hw).min(hh);
    [
        Rect::new(rect.x, rect.y, rect.w, w),                    // 上
        Rect::new(rect.x, rect.y + rect.h - w, rect.w, w),       // 下
        Rect::new(rect.x, rect.y, w, rect.h),                    // 左
        Rect::new(rect.x + rect.w - w, rect.y, w, rect.h),       // 右
    ]
}

/// 调试形状（物理像素）→ 物理像素线段列表：每条 = `([起点, 终点], 线宽)`。
///
/// 纯几何（可单测）；线段随后由 `QuadCollector` 经 `thick_line_quad` 生成带厚度
/// 四边形。Grid 每方向最多 512 条（防病态输入）。
pub(crate) fn debug_shape_segments(shape: &DebugShape) -> Vec<([Vec2; 2], f32)> {
    use std::f32::consts::TAU;
    let mut segs: Vec<([Vec2; 2], f32)> = Vec::new();
    match shape {
        DebugShape::Line { a, b, width } => {
            segs.push(([*a, *b], *width));
        }
        DebugShape::RectOutline { rect, width } => {
            let tl = Vec2::new(rect.x, rect.y);
            let tr = Vec2::new(rect.x + rect.w, rect.y);
            let br = Vec2::new(rect.x + rect.w, rect.y + rect.h);
            let bl = Vec2::new(rect.x, rect.y + rect.h);
            for (a, b) in [(tl, tr), (tr, br), (br, bl), (bl, tl)] {
                segs.push(([a, b], *width));
            }
        }
        DebugShape::CircleOutline {
            center,
            radius,
            segments,
            width,
        } => {
            let seg = (*segments).max(3);
            let c = *center;
            let r = *radius;
            for i in 0..seg {
                let a0 = i as f32 / seg as f32 * TAU;
                let a1 = (i + 1) as f32 / seg as f32 * TAU;
                let p0 = c + Vec2::new(a0.cos(), a0.sin()) * r;
                let p1 = c + Vec2::new(a1.cos(), a1.sin()) * r;
                segs.push(([p0, p1], *width));
            }
        }
        DebugShape::Cross { center, half, width } => {
            let c = *center;
            let h = *half;
            segs.push(([c - Vec2::new(h, 0.0), c + Vec2::new(h, 0.0)], *width));
            segs.push(([c - Vec2::new(0.0, h), c + Vec2::new(0.0, h)], *width));
        }
        DebugShape::Grid { rect, spacing, width } => {
            let w = *width;
            let sp = (*spacing).max(f32::EPSILON);
            let x0 = rect.x;
            let y0 = rect.y;
            let x1 = rect.x + rect.w;
            let y1 = rect.y + rect.h;
            let mut x = x0;
            let mut n = 0;
            while x <= x1 && n < 512 {
                segs.push(([Vec2::new(x, y0), Vec2::new(x, y1)], w));
                x += sp;
                n += 1;
            }
            let mut y = y0;
            n = 0;
            while y <= y1 && n < 512 {
                segs.push(([Vec2::new(x0, y), Vec2::new(x1, y)], w));
                y += sp;
                n += 1;
            }
        }
    }
    segs
}

/// 屏幕空间调试图元（`rjw_ui` 的 DebugDraw；坐标 = **逻辑屏幕像素**，左上角原点、
/// Y+ 向下，与 UI 控件坐标一致）。经 [`Ui::debug_line`] 等录制，`finish` 时以
/// **白色纹理四边形**在 UI 内容**之后**提交（覆盖在一切 UI 之上）。
///
/// 世界坐标（游戏场景内）的调试图元见 [`rjw_2d_render::debug_draw`]。
#[derive(Clone, Copy, Debug)]
pub enum DebugShape {
    /// 线段（`a` → `b`；`width` 为逻辑像素）。
    Line { a: Vec2, b: Vec2, width: f32 },
    /// 矩形边框。
    RectOutline { rect: Rect, width: f32 },
    /// 圆环（`segments` 段折线近似）。
    CircleOutline { center: Vec2, radius: f32, segments: usize, width: f32 },
    /// 十字标记（点 / 采样位置）。
    Cross { center: Vec2, half: f32, width: f32 },
    /// 网格线（`rect` 范围内按 `spacing` 竖线 + 横线；每方向最多 512 条）。
    Grid { rect: Rect, spacing: f32, width: f32 },
}

/// **矩形渐变**（v0.3）：四角颜色 + 光栅化器双线性插值——**不需要纹理**。
///
/// # 为什么不用纹理
///
/// 旧实现把渐变烘成一张 1×64 / 64×1 的纹理塞进动态图集，再拉伸采样。那带来一串代价：
/// 每帧一次 `String` 建 key（且 3 位小数精度会静默撞键）、`permanent` 条目让图集
/// **永久无法 `repack_all`**、每次纹理切换多一次 draw call、而且**单轴纹理表达不了
/// 四角各异的颜色**。
///
/// 顶点格式 [`VertexP3U2C4`](rjw_2d_render::VertexP3U2C4) 自带 4 分量顶点色，
/// 光栅化器本就对顶点色做重心插值 ⇒ 一个 quad + 白纹理即可，**管线零改动**。
///
/// # 用法
///
/// ```ignore
/// // 纯色（等价于 solid 的另一种写法）
/// ui.gradient_rect_at(pos, size, Color::RED);
/// // 上下 / 左右双色
/// ui.gradient_rect_at(pos, size, Gradient::vertical(Color::RED, Color::BLUE));
/// ui.gradient_rect_at(pos, size, Gradient::horizontal(Color::RED, Color::BLUE));
/// // 任意角度（0° = 下→上，90° = 左→右；逆时针）
/// ui.gradient_rect_at(pos, size, Gradient::rotated(Color::RED, Color::BLUE, 30.0f32.to_radians()));
/// // 四角各异（1D 纹理做不到）
/// ui.gradient_rect_at(pos, size, Gradient::corners(Color::RED, Color::YELLOW, Color::BLUE, Color::GREEN));
/// ```
///
/// **不支持多段 stops**（3+ 颜色停靠点）：四角顶点色是双线性的，无法精确表达多段。
/// 多段渐变请用 `rjw_text::Gradient`（作用于文字，本就支持多段）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    /// 左上角色。
    pub tl: Color,
    /// 右上角色。
    pub tr: Color,
    /// 左下角色。
    pub bl: Color,
    /// 右下角色。
    pub br: Color,
}

impl From<Color> for Gradient {
    /// 纯色（四角同色）——让 `gradient_rect_at(..)` 也能直接收 `Color`。
    #[inline]
    fn from(c: Color) -> Self {
        Self::pure(c)
    }
}

impl Gradient {
    /// 纯色（四角同色；等价于实心填充）。
    #[inline]
    pub const fn pure(c: Color) -> Self {
        Self { tl: c, tr: c, bl: c, br: c }
    }

    /// 上下双色：`top` 在上、`bottom` 在下。
    #[inline]
    pub const fn vertical(top: Color, bottom: Color) -> Self {
        Self { tl: top, tr: top, bl: bottom, br: bottom }
    }

    /// 左右双色：`left` 在左、`right` 在右。
    #[inline]
    pub const fn horizontal(left: Color, right: Color) -> Self {
        Self { tl: left, tr: right, bl: left, br: right }
    }

    /// 四角显式指定（双线性插值）。
    ///
    /// 顺序为**左上、右上、左下、右下**——与 `VertexP3U2C4` 的
    /// `[TL, TR, BL, BR]` 顶点顺序一致。
    #[inline]
    pub const fn corners(tl: Color, tr: Color, bl: Color, br: Color) -> Self {
        Self { tl, tr, bl, br }
    }

    /// 任意角度双色渐变：颜色沿 `angle` 方向从 `from` 过渡到 `to`。
    ///
    /// - **`0` = 下→上**（等价 [`Self::vertical`]，`from` 在下）；
    /// - **`PI/2` = 左→右**（等价 [`Self::horizontal`]，`from` 在左）；
    /// - 角度**逆时针**增大；
    /// - 渐变轴过矩形中心，把四角投影到该轴上取 `t ∈ [0,1]`，因此**任意角度下
    ///   两端的颜色都恰好落在矩形的两个极角上**（不会出现「只渐变了一半」）。
    ///
    /// `angle` 为 0 或非有限值时退化为 [`Self::vertical`]。
    pub fn rotated(from: Color, to: Color, angle: f32) -> Self {
        if !angle.is_finite() || angle == 0.0 {
            return Self::vertical(from, to);
        }
        // 方向向量：0° → (0,-1)（屏幕坐标 Y+ 向下 ⇒ 指向「上」）；逆时针为正。
        let (sin, cos) = angle.sin_cos();
        let dir = [sin, -cos]; // (dx, dy)
        // 四角在 direction 上的投影（矩形局部 0/1 坐标下等价于符号组合）。
        let proj = |x: f32, y: f32| x * dir[0] + y * dir[1];
        let corners = [
            (0.0f32, 0.0f32), // TL
            (1.0, 0.0),       // TR
            (0.0, 1.0),       // BL
            (1.0, 1.0),       // BR
        ];
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for (x, y) in corners {
            let p = proj(x, y);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let span = hi - lo;
        let sample = |x: f32, y: f32| -> Color {
            if span <= f32::EPSILON {
                return from;
            }
            lerp_color(from, to, (proj(x, y) - lo) / span)
        };
        Self {
            tl: sample(0.0, 0.0),
            tr: sample(1.0, 0.0),
            bl: sample(0.0, 1.0),
            br: sample(1.0, 1.0),
        }
    }
}

/// 颜色线性插值（`k` 不 clamp——调用方负责；`Gradient` 的构造器与四角采样用）。
#[inline]
pub fn lerp_color(a: Color, b: Color, k: f32) -> Color {
    let af: [f32; 4] = a.into();
    let bf: [f32; 4] = b.into();
    let mut o = [0f32; 4];
    for i in 0..4 {
        o[i] = af[i] + (bf[i] - af[i]) * k;
    }
    Color::from(o)
}

// ─── 背景图（ImageBg） ────────────────────────────────────────

/// 背景图**铺排方式**（[`ImageBg::fit`]）。
///
/// 前三种（`Stretch` / `Fill` / `Center`）的 UV 映射是**仿射**的：扇形三角化下的
/// 重心插值**精确**，因此能和圆角遮罩（`ImageBg::radius`）共存、且只花一个四边形的
/// 几何（`radius > 0` 时是圆角硬体 + 羽化带，与实心背景同一条路径）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ImageFit {
    /// **拉伸填满**（各向异性缩放；会变形）。
    #[default]
    Stretch,
    /// **等比放大到覆盖整块区域**（保持宽高比；超出的部分按**居中**裁剪，不变形）。
    Fill,
    /// **原始尺寸居中**（纹素 = 逻辑像素，1:1；放不下时居中裁剪，最远端先丢）。
    Center,
    /// **1:1 平铺**（纹素 = 逻辑像素，不平滑缩放；最后一行/列是**部分块**，UV 按比例截断）。
    ///
    /// ⚠ 两个已知边界：
    /// 1. **不支持圆角遮罩**（`radius` 被忽略）：圆角靠"逐顶点 UV 的仿射映射 + 扇形
    ///    三角化"实现，而平铺需要 UV **环绕**（`u > 1`）——除非给批次换成 `Repeat`
    ///    采样器（需 `UiBatch` 携带 `RStates`，属引擎级改动）。需要圆角用 `Fill`。
    /// 2. 平铺块数有上限（[`MAX_IMAGE_TILES`]），超限**退化为 [`ImageFit::Stretch`]**
    ///    （避免 1px 图块在大面板上生成上万顶点）。
    Tile,
}

/// 平铺块数上限（`Tile` 单块四边形 = 4 顶点；上限 × 4 顶点要远小于 `u16` 索引域）。
pub const MAX_IMAGE_TILES: u32 = 2048;

/// **背景图**：纹理 uid + 纹素尺寸 + 铺排 + 染色 + 圆角遮罩。
///
/// 纹理按 **uid** 引用（[`rjw_render::TextureWrapped::uid`]；后端按 uid 解析），
/// `texel` = 纹理的**纹素**尺寸——`Center` / `Tile` 恒按 1:1 铺排，必须靠它换算。
///
/// ```no_run
/// # use rjw_ui::{ImageBg, ImageFit};
/// # let tex_uid: u64 = 1;
/// let bg = ImageBg::new(tex_uid, glam::Vec2::new(64.0, 64.0))
///     .fit(ImageFit::Fill)
///     .radius(8.0)
///     .tint(rjw_color::Color::WHITE);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageBg {
    /// 纹理 uid（`TextureWrapped::uid`）。
    pub tex: u64,
    /// 纹理**纹素**尺寸（物理像素；`Center` / `Tile` 的 1:1 基准）。
    pub texel: Vec2,
    /// 铺排方式。
    pub fit: ImageFit,
    /// 顶点色（乘在纹理上；alpha < 1 = 整块半透明）。
    pub tint: Color,
    /// **圆角遮罩**半径（物理像素；0 = 直角，不裁）。`Tile` 忽略它（见 [`ImageFit::Tile`]）。
    pub radius: CornerRadius,
}

/// 铺排解算结果（纯几何；[`ImageBg::layout`]）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageLayout {
    /// 图片实际绘制矩形（`Center` 可能小于输入 `rect`，`Fill` 恒等于输入 `rect`）。
    pub rect: Rect,
    /// `rect` 左上角 / 右下角对应的 **UV**。
    pub uv0: Vec2,
    pub uv1: Vec2,
    /// `Some(tile)` = 平铺：按 `tile`（= 纹素尺寸）切网格，每块 UV 恒 `0..1`。
    pub tile: Option<Vec2>,
}

impl ImageBg {
    /// 拉伸填满（默认）。
    #[inline]
    pub fn new(tex: u64, texel: Vec2) -> Self {
        Self { tex, texel, fit: ImageFit::Stretch, tint: Color::WHITE, radius: CornerRadius::default() }
    }

    /// 设置铺排方式。
    #[inline]
    pub fn fit(mut self, fit: ImageFit) -> Self {
        self.fit = fit;
        self
    }

    /// 设置圆角遮罩（`f32` = 四角同半径；`CornerRadius` = 逐角）。
    #[inline]
    pub fn radius(mut self, radius: impl Into<CornerRadius>) -> Self {
        self.radius = radius.into();
        self
    }

    /// 设置顶点色（乘在纹理上）。
    #[inline]
    pub fn tint(mut self, tint: Color) -> Self {
        self.tint = tint;
        self
    }

    /// **铺排解算**（纯函数）：输入目标矩形，输出"画在哪、取哪块 UV"。
    ///
    /// `None` = 没有可画的东西（尺寸退化 / 纹素尺寸为 0）。
    pub fn layout(&self, rect: Rect) -> Option<ImageLayout> {
        if rect.w <= 0.0 || rect.h <= 0.0 || self.texel.x <= 0.0 || self.texel.y <= 0.0 {
            return None;
        }
        match self.fit {
            ImageFit::Stretch => Some(ImageLayout {
                rect,
                uv0: Vec2::ZERO,
                uv1: Vec2::ONE,
                tile: None,
            }),
            ImageFit::Fill => {
                // 等比放大到覆盖：`scale = max(覆盖所需的两轴比例)`；缩放后按居中裁剪，
                // 于是**可见矩形 = 输入 rect**，UV 只取中间那块。
                let s = (rect.w / self.texel.x).max(rect.h / self.texel.y);
                let shown = Vec2::new(rect.w / (self.texel.x * s), rect.h / (self.texel.y * s));
                let off = (Vec2::ONE - shown) * 0.5;
                Some(ImageLayout {
                    rect,
                    uv0: off,
                    uv1: off + shown,
                    tile: None,
                })
            }
            ImageFit::Center | ImageFit::Tile => {
                // 1:1 居中：可见尺寸 = min(纹素, 可用区)。
                let shown_px = Vec2::new(rect.w.min(self.texel.x), rect.h.min(self.texel.y));
                let img = Rect::new(
                    rect.x + (rect.w - shown_px.x) * 0.5,
                    rect.y + (rect.h - shown_px.y) * 0.5,
                    shown_px.x,
                    shown_px.y,
                );
                // 放不下时居中裁剪：只取中央那块 UV。
                let scale = Vec2::new(shown_px.x / self.texel.x, shown_px.y / self.texel.y);
                let off = (Vec2::ONE - scale) * 0.5;
                Some(ImageLayout {
                    rect: img,
                    uv0: off,
                    uv1: off + scale,
                    tile: matches!(self.fit, ImageFit::Tile).then_some(self.texel),
                })
            }
        }
    }
}

/// **平铺网格**（纯函数）：把 `rect` 按 `tile` 切成 1:1 图块，返回每块的
/// `(矩形, UV 终点)`——UV 起点恒 `(0,0)`，终点 < `(1,1)` 表示**边缘的部分块**
/// （按比例截断，不会把整块图压进去）。
///
/// 超过 [`MAX_IMAGE_TILES`] 时返回 `None`（调用方退化为拉伸）。
pub fn tile_grid(rect: Rect, tile: Vec2) -> Option<impl Iterator<Item = (Rect, Vec2)> + use<>> {
    if tile.x <= 0.0 || tile.y <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return None;
    }
    let cols = (rect.w / tile.x).ceil().max(1.0) as u32;
    let rows = (rect.h / tile.y).ceil().max(1.0) as u32;
    if cols.saturating_mul(rows) > MAX_IMAGE_TILES {
        return None;
    }
    Some((0..rows).flat_map(move |r| {
        (0..cols).map(move |c| {
            let x = rect.x + c as f32 * tile.x;
            let y = rect.y + r as f32 * tile.y;
            let w = tile.x.min(rect.x + rect.w - x);
            let h = tile.y.min(rect.y + rect.h - y);
            let uv1 = Vec2::new(w / tile.x, h / tile.y);
            (Rect::new(x, y, w, h), uv1)
        })
    }))
}

/// 绘制命令种类（记录式；`Ui::finish` 逐条提交到 `Render2D`）。
#[derive(Clone, Debug)]
pub enum DrawKind {
    /// 实心矩形。
    Solid(Color),
    /// **圆角矩形**（背景填充；`radius` 物理像素，CPU 镶嵌成三角形，颜色走顶点色）。
    ///
    /// `corners` = `[TL, TR, BL, BR]`（⚠ **不是** [`CornerRadius`] 的 `tl/tr/br/bl` 顺序）：
    /// 纯色时四者相同；两端色渐变时各异 ⇒「圆角 + 渐变」不需要任何专门着色器或渐变纹理。
    RoundedRect { corners: [Color; 4], radius: CornerRadius },
    /// **矩形渐变**（四角颜色；顶点色插值，**无纹理**）。
    Rect(Gradient),
    /// 矩形边框（画在 `rect` 内缘）。
    ///
    /// `radius` 非零时是**圆角环带**（外轮廓半径 `radius`、内轮廓半径
    /// `max(0, radius - width)` 逐角计算；见 `crate::tess::push_rounded_ring`），
    /// 与 [`Self::RoundedRect`] 的圆角语义一致（四角可各自独立）。
    Border { color: Color, width: f32, radius: CornerRadius },
    /// **窗口 / 面板投影**（**顶点色**软阴影：无纹理、无着色器、不增 draw call）。
    ///
    /// `rect` = **本体矩形**（阴影内轮廓**恒在本体边缘**：浓度从本体边向外单调衰减），
    /// `blur` = 向外渐隐宽度，`offset` = 最外圈相对本体的偏移（光源反向；**按圈数线性
    /// 分摊** ⇒ 投影整体偏向光源反侧，且没有"等浓度暗带"）——见
    /// `crate::tess::push_rounded_shadow`。
    ///
    /// 遮蔽与元素序：画在窗口背景**之下**（`elem = 0`，且先于背景入队）——于是它
    /// 覆盖在**更低 z 的窗口**（"投影落在下面的窗口上"）与窗口自身内容之下。
    Shadow { color: Color, blur: f32, offset: Vec2, radius: CornerRadius },
    /// **矢量图标**（画出来的几何，与字体无关）：`rect` 是图标方框，几何取
    /// [`Icon::parts`] 的单位坐标映射进去，并按 `Theme::feather` 做边缘羽化。
    Icon { icon: Icon, color: Color },
    /// **背景图**（[`ImageBg`]）：`rect` 是目标区域，铺排 / 染色 / 圆角遮罩由 `bg` 决定。
    ///
    /// 与实心背景同一条镶嵌路径（CPU 直出三角形 + 羽化），因此**不额外增加 draw call**：
    /// 图片落在自己纹理的批次里（按纹理切段，与字形/白纹理各一段）。
    Image(ImageBg),
    /// 文本（绘制时经 `rjw_text` 责任链渲染）。
    Text {
        /// 文本内容（`Arc<str>`：命令间共享，避免每命令 String 克隆）。
        text: Arc<str>,
        size: f32,
        color: Color,
        align: TextAlign,
        valign: TextVAlign,
        family: Option<Arc<str>>,
        /// 文本局部裁剪（相对内容起点；`None` = 不裁剪）。
        clip: Option<Rect>,
        /// **预排版缓冲**（控件自持，输入框/TextArea 用；`None` = 绘制期按需缓存）。
        /// 不进内容签名（排版结果由 `text`/`size`/`family` 决定；窗口顶点缓存已固化字形）。
        buf: Option<Arc<Buffer>>,
    },
    /// 文本输入框光标（竖条）。
    Caret { color: Color, width: f32 },
    /// 屏幕空间调试图元（DebugDraw；坐标 = 逻辑屏幕像素，覆盖在 UI 内容之上）。
    Debug { color: Color, shape: DebugShape },
}

/// 文本水平对齐。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// 文本**垂直**对齐（锚点在矩形内的位置）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextVAlign {
    /// 顶对齐（行盒顶 = 矩形顶；TextArea 多行编辑用——光标按逻辑行 TopLeft 定位）。
    Top,
    /// 垂直居中（默认；单行控件）。
    Center,
}

// ─── 矢量图标（画出来的几何，与字体无关） ──────────────────────

/// 图标笔画在**单位方框** `[0,1]²` 内的顶点表（Y 向下）。
///
/// 每个分片都必须是**凸**多边形且按**屏幕顺时针**给出（与 `Quad` 的 `(TL,TR,BR)`
/// 同向）——镶嵌器据此做扇形三角化 + 边缘羽化。多分片表示"一笔一个凸多边形"
/// （勾选的"✓"是两笔，`Grip` 是三横）。
type IconPart = &'static [Vec2];

/// 勾选"✓"的左笔（短促的下行）。
const CHECK_L: IconPart = &[
    Vec2::new(0.177, 0.463),
    Vec2::new(0.417, 0.703),
    Vec2::new(0.303, 0.817),
    Vec2::new(0.063, 0.577),
];
/// 勾选"✓"的右笔（长上行）。
const CHECK_R: IconPart = &[
    Vec2::new(0.268, 0.599),
    Vec2::new(0.828, 0.119),
    Vec2::new(0.932, 0.241),
    Vec2::new(0.372, 0.721),
];
/// 关闭"✕"的第一笔（左上 → 右下；凸四边形，笔画宽 ~0.18）。
///
/// 与勾选同规格的**对角笔画**：常按 12~16px 画，0.18 单位 ≈ 2~3px 实心，
/// 加羽化边缘后两笔交叠处读得出来（更细会被 AA 糊成一片）。
const CLOSE_A: IconPart = &[
    Vec2::new(0.13, 0.31),
    Vec2::new(0.31, 0.13),
    Vec2::new(0.87, 0.69),
    Vec2::new(0.69, 0.87),
];
/// 关闭"✕"的第二笔（右上 → 左下）。
const CLOSE_B: IconPart = &[
    Vec2::new(0.13, 0.69),
    Vec2::new(0.69, 0.13),
    Vec2::new(0.87, 0.31),
    Vec2::new(0.31, 0.87),
];
/// 拖拽手柄：三横（每横一个凸四边形）。
///
/// ⚠ 每横的高度别低于 ~0.14：图标常按 12~14px 画，1 个"单位"才 1.2~1.4px——再薄就只剩
/// 亚像素，配合 AA 会被糊成一片（"图标像是近视一样"）。0.16 高 + 0.06 间隙在 12px 下
/// 是 ~1.9px 实心 + ~0.7px 间隙，三横读得出来。
const GRIP_1: IconPart = &[
    Vec2::new(0.25, 0.20),
    Vec2::new(0.75, 0.20),
    Vec2::new(0.75, 0.36),
    Vec2::new(0.25, 0.36),
];
const GRIP_2: IconPart = &[
    Vec2::new(0.25, 0.42),
    Vec2::new(0.75, 0.42),
    Vec2::new(0.75, 0.58),
    Vec2::new(0.25, 0.58),
];
const GRIP_3: IconPart = &[
    Vec2::new(0.25, 0.64),
    Vec2::new(0.75, 0.64),
    Vec2::new(0.75, 0.80),
    Vec2::new(0.25, 0.80),
];
/// 拖拽手柄：**三斜线**（45°，从左下到右上；每笔一个凸四边形）。
///
/// 几何契约（用户给的图）：三条线的**首端点在一条水平线上等距**、**末端点在一条竖直线上
/// 等距**——即第 `i` 条从 `(s, 0.92)` 到 `(0.92, s)`（`s = 0.20 / 0.40 / 0.60`），
/// 于是三条都是 45°、互相平行、垂直间距相等。笔宽沿 `(1,1)` 偏 `0.055`（垂直厚度
/// ≈ 0.078；27px 方框里 ≈ 2.1px 实心）——别更细，羽化会把三条糊在一起。
const GRIP_D1: IconPart = &[
    Vec2::new(0.20, 0.92),
    Vec2::new(0.92, 0.20),
    Vec2::new(0.975, 0.255),
    Vec2::new(0.255, 0.975),
];
const GRIP_D2: IconPart = &[
    Vec2::new(0.40, 0.92),
    Vec2::new(0.92, 0.40),
    Vec2::new(0.975, 0.455),
    Vec2::new(0.455, 0.975),
];
const GRIP_D3: IconPart = &[
    Vec2::new(0.60, 0.92),
    Vec2::new(0.92, 0.60),
    Vec2::new(0.975, 0.655),
    Vec2::new(0.655, 0.975),
];
/// 箭头（下 / 上 / 左 / 右）——等腰三角形。
const TRI_DOWN: IconPart = &[
    Vec2::new(0.15, 0.32),
    Vec2::new(0.85, 0.32),
    Vec2::new(0.50, 0.72),
];
const TRI_UP: IconPart = &[
    Vec2::new(0.15, 0.68),
    Vec2::new(0.50, 0.28),
    Vec2::new(0.85, 0.68),
];
const TRI_RIGHT: IconPart = &[
    Vec2::new(0.32, 0.15),
    Vec2::new(0.72, 0.50),
    Vec2::new(0.32, 0.85),
];
const TRI_LEFT: IconPart = &[
    Vec2::new(0.68, 0.15),
    Vec2::new(0.68, 0.85),
    Vec2::new(0.28, 0.50),
];
/// 警告三角（等边感、尖朝上）：外轮廓本身就是**凸**的，可直接作一个分片。
const WARN_TRI: IconPart = &[
    Vec2::new(0.50, 0.10),
    Vec2::new(0.96, 0.88),
    Vec2::new(0.04, 0.88),
];
/// 感叹号竖条（警告三角内部，凸四边形）。
const WARN_BAR: IconPart = &[
    Vec2::new(0.44, 0.38),
    Vec2::new(0.56, 0.38),
    Vec2::new(0.56, 0.64),
    Vec2::new(0.44, 0.64),
];
/// 感叹号圆点（近似方点即可——12px 图标下看不出差别，且保持"凸分片"前提）。
const WARN_DOT: IconPart = &[
    Vec2::new(0.44, 0.70),
    Vec2::new(0.56, 0.70),
    Vec2::new(0.56, 0.82),
    Vec2::new(0.44, 0.82),
];

/// **矢量图标**：内置图元一律用**画出来的几何**，不用字体字形。
///
/// # 为什么不用字形
///
/// `▾` / `▼` / `✓` / `≡` 这类字符的可用性、宽度、基线**全由字体决定**——字体缺字形
/// 就走 fallback（豆腐块 / 尺寸不对），同一套 UI 换个字体图标就跑偏。图标是**几何**，
/// 应该与字体无关。
///
/// # 为什么用枚举而不是任意点表
///
/// 图标是**有限且固定**的一组，用 `Copy` 枚举：
/// - 内容签名（`cmd_sig_hash`）只需哈希一个小整数，不必逐点哈希；
/// - 几何是 `const` 表（零分配、可单测）；
/// - 调用点写 `Icon::Check` 而不是一串魔数坐标。
///
/// 需要任意多边形/曲线时用 [`Ui::polygon_at`](crate::ui::Ui::polygon_at)（凸多边形）——
/// 图标走枚举是为了"常用形状便宜且统一"。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// 向下箭头（下拉框 / 展开的取色器）。
    ChevronDown,
    /// 向上箭头（已展开）。
    ChevronUp,
    /// 向左箭头。
    ChevronLeft,
    /// 向右箭头。
    ChevronRight,
    /// 勾选（下拉菜单选中标记）。
    Check,
    /// 拖拽手柄（三横）。
    Grip,
    /// **拖拽手柄（三斜线）**：三条平行的 45° 笔画（从左下到右上，越靠右下角越长）——
    /// 经典"缩放角"观感（见 [`Icon::Grip`] 的横线版本与 [`crate::GripShape::Diagonal`]）。
    GripDiagonal,
    /// 警告 / 非法输入（三角 + 感叹号）——错误提示、取色器"文本无法识别"按钮用。
    Warning,
    /// 关闭"✕"（两条对角笔画）——窗口标题栏的关闭按钮用。
    Close,
}

impl Icon {
    /// 单位方框内的**凸**分片（顺时针，Y 向下）——镶嵌器的输入。
    #[inline]
    pub fn parts(self) -> &'static [IconPart] {
        match self {
            Icon::ChevronDown => &[TRI_DOWN],
            Icon::ChevronUp => &[TRI_UP],
            Icon::ChevronLeft => &[TRI_LEFT],
            Icon::ChevronRight => &[TRI_RIGHT],
            Icon::Check => &[CHECK_L, CHECK_R],
            Icon::Grip => &[GRIP_1, GRIP_2, GRIP_3],
            Icon::GripDiagonal => &[GRIP_D1, GRIP_D2, GRIP_D3],
            Icon::Warning => &[WARN_TRI, WARN_BAR, WARN_DOT],
            Icon::Close => &[CLOSE_A, CLOSE_B],
        }
    }
}

impl DrawKind {
    /// 类别分组（同一 layer 内"**背景/图形 → 文字**"排序用）：
    /// - `0`：背景 / 图形（Solid / Border / Caret / Icon）——先画；
    /// - `1`：文字（Text）——后画（覆盖在图形之上）。
    /// - `2`：调试图元（Debug）——内容排序时不会出现（走独立调试队列，恒最后提交）。
    ///
    /// 窗口内元素**不处理互相重叠**：仅保证类别顺序，同类间保持录制顺序。
    #[inline]
    pub fn group(&self) -> u8 {
        match self {
            DrawKind::Text { .. } => 1,
            DrawKind::Debug { .. } => 2,
            _ => 0,
        }
    }
}

/// 一条绘制命令（坐标 = 相对当前容器 origin 的局部坐标，容器弹出时统一平移）。
#[derive(Clone, Debug)]
pub struct UiDraw {
    pub depth: u32,
    pub seq: u32,
    /// 所属窗口的 z 序（[`crate::Ui::window`]；非窗口内容 = 0）。
    /// 窗口间按 z 升序绘制（焦点窗口 z 最大 → 最后画 → 最上层）。
    pub win: u32,
    /// **元素序**：所属控件（元素）开始录制时的序号。
    ///
    /// 排序键 `(win, depth, elem, group, seq)`：**元素间按录制顺序**（后录元素
    /// 覆盖先录元素，重叠层级正确），**元素内**再按"背景/图形 → 文字"（`group`）。
    /// 容器背景 / 边框等"容器装饰"用 `elem = 0`（恒画在本容器元素之下）。
    pub elem: u32,
    pub rect: Rect,
    /// **裁剪区**（**绝对逻辑屏幕坐标**；滚动容器等设置，`None` = 不裁剪）。
    /// 与 `rect` 不同：**不随容器弹出平移**（`translate` 只移动 `rect`）——因为
    /// `rect` 是相对当前容器 origin 的局部坐标（逐层平移成绝对），而 `clip` 记录时
    /// 已是绝对坐标，再平移会双重偏移（滚动容器内容被错误裁掉）。
    pub clip: Option<Rect>,
    pub kind: DrawKind,
}

impl UiDraw {
    #[inline]
    pub fn translate(&mut self, by: Vec2) {
        self.rect.x += by.x;
        self.rect.y += by.y;
        // 只平移 rect：clip 已是绝对坐标（见字段文档），平移会造成双重偏移——
        // 滚动容器（scroll_at）里录制的命令 clip = 可视区绝对矩形，若再加一次
        // pos 偏移会被裁到屏幕外（list_at 内容"不显示"的根因）。
    }
}

/// 一条文本命令的便捷构造。
#[allow(clippy::too_many_arguments)]
pub fn text_cmd(
    depth: u32,
    seq: u32,
    win: u32,
    elem: u32,
    rect: Rect,
    text: Arc<str>,
    size: f32,
    color: Color,
    align: TextAlign,
    valign: TextVAlign,
    family: Option<Arc<str>>,
    clip: Option<Rect>,
    clip_outer: Option<Rect>,
    buf: Option<Arc<Buffer>>,
) -> UiDraw {
    UiDraw {
        depth,
        seq,
        win,
        elem,
        rect,
        clip: clip_outer,
        kind: DrawKind::Text {
            text,
            size,
            color,
            align,
            valign,
            family,
            clip,
            buf,
        },
    }
}

#[cfg(test)]
mod corner_radius_tests {
    use super::*;

    #[test]
    fn grip_diagonal_matches_the_spec() {
        // 用户给的几何契约：三条 45° 斜线的**首端点**在一条水平线上（同 y）**等距**，
        // **末端点**在一条竖直线上（同 x）**等距**；每条都是 45°（dx == -dy）。
        let parts = Icon::GripDiagonal.parts();
        assert_eq!(parts.len(), 3, "三条斜线");
        let starts: Vec<Vec2> = parts.iter().map(|p| p[0]).collect();
        let ends: Vec<Vec2> = parts.iter().map(|p| p[1]).collect();
        // 首端点：同 y、x 等距。
        assert!(
            starts.windows(2).all(|w| (w[0].y - w[1].y).abs() < 1e-6),
            "首端点必须在同一条水平线上：{starts:?}"
        );
        let d1 = starts[1].x - starts[0].x;
        let d2 = starts[2].x - starts[1].x;
        assert!((d1 - d2).abs() < 1e-6 && d1 > 0.0, "首端点必须等距：{d1} vs {d2}");
        // 末端点：同 x、y 等距（间距与首端一致）。
        assert!(
            ends.windows(2).all(|w| (w[0].x - w[1].x).abs() < 1e-6),
            "末端点必须在同一条竖直线上：{ends:?}"
        );
        let e1 = ends[1].y - ends[0].y;
        let e2 = ends[2].y - ends[1].y;
        assert!((e1 - e2).abs() < 1e-6 && e1 > 0.0, "末端点必须等距：{e1} vs {e2}");
        assert!((e1 - d1).abs() < 1e-6, "两端间距相等 ⇒ 三条平行 45°：{d1} vs {e1}");
        // 每条 45°（屏幕 y 向下 ⇒ 向右同时向上）。
        for p in parts {
            let (dx, dy) = (p[1].x - p[0].x, p[1].y - p[0].y);
            assert!((dx + dy).abs() < 1e-6, "必须是 45°：{p:?}（dx={dx} dy={dy}）");
        }
        // 笔宽：第三、四点相对第一、二点沿 (1,1) 等量偏移（同一方向、同一厚度）。
        for p in parts {
            let (ox, oy) = (p[3].x - p[0].x, p[3].y - p[0].y);
            let (ox2, oy2) = (p[2].x - p[1].x, p[2].y - p[1].y);
            assert!((ox - ox2).abs() < 1e-6 && (oy - oy2).abs() < 1e-6, "同厚度：{p:?}");
            assert!(ox > 0.0 && oy > 0.0, "沿 (1,1) 偏移（右下方向）：{ox},{oy}");
        }
    }

    #[test]
    fn from_scalar_is_uniform_and_compares_against_scalars() {
        let r: CornerRadius = 8.0.into();
        assert_eq!(r, CornerRadius::all(8.0));
        assert_eq!(r.uniform(), Some(8.0));
        assert_eq!(r, 8.0, "四角相同才等于该标量");
        // 四角不同 ⇒ 不等于任何标量（避免把"只圆上面两角"误判成"没设圆角"）
        let tab = CornerRadius { tl: 8.0, tr: 8.0, br: 0.0, bl: 0.0 };
        assert_eq!(tab.uniform(), None);
        assert_ne!(tab, 8.0);
        assert_ne!(tab, 0.0);
        assert!(CornerRadius::all(0.0).is_zero());
        assert!(!tab.is_zero());
    }

    #[test]
    fn tuple_constructor_is_tl_tr_br_bl() {
        // `(tl, tr, br, bl)` —— 与字段顺序一致，**不是**颜色数组的 `[TL,TR,BL,BR]`。
        let r: CornerRadius = (1.0, 2.0, 3.0, 4.0).into();
        assert_eq!((r.tl, r.tr, r.br, r.bl), (1.0, 2.0, 3.0, 4.0));
    }

    #[test]
    fn map_scaled_and_clamped_are_per_corner() {
        let r = CornerRadius { tl: 4.0, tr: 8.0, br: 12.0, bl: 16.0 };
        assert_eq!(r.scaled(0.5), CornerRadius { tl: 2.0, tr: 4.0, br: 6.0, bl: 8.0 });
        assert_eq!(r.scaled_rounded(0.5), r.scaled(0.5));
        assert_eq!(r.max(), 16.0);
        assert_eq!(r.min(), 4.0);
        assert_eq!(r.clamped(10.0), CornerRadius { tl: 4.0, tr: 8.0, br: 10.0, bl: 10.0 });
    }

    #[test]
    fn fit_shrinks_all_corners_by_the_same_factor() {
        // 宽 40、上下各 30 ⇒ tl + tr = 60 > 40 ⇒ 比例 40/60 = 2/3。
        let r = CornerRadius { tl: 30.0, tr: 30.0, br: 0.0, bl: 0.0 };
        let f = r.fit(40.0, 100.0);
        assert!((f.tl - 20.0).abs() < 1e-4 && (f.tr - 20.0).abs() < 1e-4);
        assert_eq!((f.br, f.bl), (0.0, 0.0), "0 角缩放后仍是 0");
        // **等比**而不是各自 clamp：两个 30 变成两个 20（不是 20 + 30 那种削平）。
        assert!((f.tl - f.tr).abs() < 1e-4);
        // 受高度限制：h = 10、tl + bl = 30 + 0 = 30 > 10 ⇒ 比例 1/3 ⇒ 30 → 10。
        assert_eq!(
            r.fit(100.0, 10.0),
            CornerRadius { tl: 10.0, tr: 10.0, br: 0.0, bl: 0.0 }
        );
        assert_eq!(r.fit(1000.0, 1000.0), r, "放得下就原样");
    }

    #[test]
    fn fit_uses_the_most_restrictive_edge() {
        // 四条边约束里最紧的那条决定比例：w/2 与 h/4 取小。
        let r = CornerRadius::all(10.0);
        // w = 40 ⇒ tl+tr = 20 ⇒ 2.0；h = 10 ⇒ tl+bl = 20 ⇒ 0.5 ⇒ 取 0.5
        let f = r.fit(40.0, 10.0);
        assert!((f.tl - 5.0).abs() < 1e-4, "应被高度约束收到 5，实际 {}", f.tl);
        assert_eq!(f.uniform(), Some(f.tl));
    }

    #[test]
    fn fit_is_noop_on_degenerate_boxes() {
        // 0 尺寸 / 无圆角都不得 panic 也不得把半径变成 NaN。
        let r = CornerRadius::all(8.0);
        assert_eq!(r.fit(0.0, 0.0), r, "退化盒子不收缩（由上游的 w/h <= 0 早退兜底）");
        assert!(CornerRadius::default().fit(10.0, 10.0).is_zero());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_rect_math() {
        // 相交：取交集
        let i = intersect_rect(&Rect::new(0.0, 0.0, 100.0, 50.0), &Rect::new(50.0, 25.0, 100.0, 50.0)).unwrap();
        assert_eq!(i, Rect::new(50.0, 25.0, 50.0, 25.0));
        // 完全包含：取较小
        let i = intersect_rect(&Rect::new(0.0, 0.0, 100.0, 50.0), &Rect::new(10.0, 10.0, 20.0, 20.0)).unwrap();
        assert_eq!(i, Rect::new(10.0, 10.0, 20.0, 20.0));
        // 不相交：None（滚动裁剪 → 全裁）
        assert!(intersect_rect(&Rect::new(0.0, 0.0, 10.0, 10.0), &Rect::new(20.0, 20.0, 10.0, 10.0)).is_none());
        // 仅边接触（半开区间）：None
        assert!(intersect_rect(&Rect::new(0.0, 0.0, 10.0, 10.0), &Rect::new(10.0, 0.0, 10.0, 10.0)).is_none());
    }

    #[test]
    fn centered_square_keeps_icons_undistorted() {
        // 已是方形：原样（浮点精确相等在这里成立：min 取其一，偏移恰为 0）。
        let sq = Rect::new(3.0, 4.0, 16.0, 16.0);
        assert_eq!(centered_square(sq), sq);
        // 非方形（row 等高约束给的 18×26）：取 min = 18 的居中方块 —— 笔画不形变。
        let tall = centered_square(Rect::new(10.0, 20.0, 18.0, 26.0));
        assert_eq!(tall, Rect::new(10.0, 24.0, 18.0, 18.0), "高框 → 居中正方形");
        let wide = centered_square(Rect::new(10.0, 20.0, 26.0, 18.0));
        assert_eq!(wide, Rect::new(14.0, 20.0, 18.0, 18.0), "宽框 → 居中正方形");
        // 退化：0 尺寸不 panic、不产生负宽高。
        let zero = centered_square(Rect::new(5.0, 5.0, 0.0, 12.0));
        assert_eq!(zero.w, 0.0);
        assert!(zero.h >= 0.0 && zero.x.is_finite());
    }

    #[test]
    fn snap_rect_rounds_to_integer_pixels() {
        let r = Rect::new(10.4, 20.6, 30.2, 15.7);
        let s = snap_rect(&r);
        assert_eq!(s.x, 10.0);
        assert_eq!(s.y, 21.0);
        // 左上角 + 右下角分别四舍五入：x1 = round(40.6) = 41, y1 = round(36.3) = 36
        assert_eq!(s.w, 31.0);
        assert_eq!(s.h, 15.0);
        // 全部整数像素
        for v in [s.x, s.y, s.w, s.h] {
            assert_eq!(v.fract(), 0.0, "取整后应为整数像素，实际 {v}");
        }
        // 尺寸偏差 ≤ 1px（round 允许 ≤0.5px 偏移换取像素对齐）
        assert!((s.w - r.w).abs() <= 1.0 && (s.h - r.h).abs() <= 1.0);
    }

    #[test]
    fn snap_rect_clamps_negative_shrink() {
        // 极端：宽高取整可能为负（如 0.4 → round 0.4=0, round(0.4+0.2)=1 → w=1）
        // 构造 w 不足 0.5 且四舍五入抵消的情形：clamp 到 ≥ 0
        let r = Rect::new(0.2, 0.2, 0.1, 0.1);
        let s = snap_rect(&r);
        assert!(s.w >= 0.0 && s.h >= 0.0, "宽高不得为负，实际 {s:?}");
        let r2 = Rect::new(0.5, 0.5, 0.4, 0.4);
        let s2 = snap_rect(&r2);
        assert!(s2.w >= 0.0 && s2.h >= 0.0, "宽高不得为负，实际 {s2:?}");
    }

    #[test]
    fn size_and_position_units_convert() {
        // From 默认 Logical（现有 f32/Vec2 调用兼容）。
        let s: Size<f32> = 220.0.into();
        assert_eq!(s, Size::Logical(220.0));
        let p: Position = Vec2::new(10.0, 20.0).into();
        assert_eq!(p, Position::Logical(Vec2::new(10.0, 20.0)));
        let v: Size<Vec2> = Vec2::new(1.0, 2.0).into();
        assert_eq!(v, Size::Logical(Vec2::new(1.0, 2.0)));
        // Logical → Physical：× scale 并**取整**（布局整数不变量）。
        assert_eq!(Size::Logical(220.0).to_physical(1.25), 275.0);
        assert_eq!(Size::Logical(6.0).to_physical(1.25), 8.0, "6×1.25=7.5 → round 8");
        assert_eq!(
            Position::Logical(Vec2::new(155.0, 32.0)).to_physical(1.25),
            Vec2::new(194.0, 40.0)
        );
        // Physical 原样（不取整）。
        assert_eq!(Size::Physical(220.0).to_physical(1.25), 220.0);
        assert_eq!(Position::Physical(Vec2::new(7.5, 8.5)).to_physical(2.0), Vec2::new(7.5, 8.5));
        // Default。
        assert_eq!(Size::<f32>::default(), Size::Logical(0.0));
        assert_eq!(Position::default(), Position::Logical(Vec2::ZERO));
    }

    #[test]
    fn snap_point_rounds() {
        assert_eq!(snap_point(Vec2::new(1.2, -3.6)), Vec2::new(1.0, -4.0));
    }

    #[test]
    fn debug_shape_segments_counts_and_geometry() {
        // 线段：1 条（坐标 / 线宽为物理像素，原样输出）
        let segs = debug_shape_segments(&DebugShape::Line {
            a: Vec2::new(10.0, 20.0),
            b: Vec2::new(30.0, 40.0),
            width: 2.0,
        });
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].0, [Vec2::new(10.0, 20.0), Vec2::new(30.0, 40.0)]);
        assert_eq!(segs[0].1, 2.0);
        // 矩形框：4 条边，角点落在矩形角上
        let segs = debug_shape_segments(&DebugShape::RectOutline {
            rect: Rect::new(0.0, 0.0, 100.0, 50.0),
            width: 1.0,
        });
        assert_eq!(segs.len(), 4);
        let corners: Vec<Vec2> = segs.iter().flat_map(|([a, b], _)| [*a, *b]).collect();
        for c in [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0), Vec2::new(100.0, 50.0), Vec2::new(0.0, 50.0)] {
            assert!(corners.contains(&c), "角点 {c:?} 应出现在矩形框线段中");
        }
        // 圆环：segments 条线段，半径不缩放
        let segs = debug_shape_segments(&DebugShape::CircleOutline {
            center: Vec2::ZERO,
            radius: 10.0,
            segments: 32,
            width: 1.0,
        });
        assert_eq!(segs.len(), 32);
        for ([a, b], w) in &segs {
            assert!((a.length() - 10.0).abs() < 0.2 && (b.length() - 10.0).abs() < 0.2, "环上点半径≈10");
            assert_eq!(*w, 1.0);
        }
        // 十字：2 条线段（横 + 竖）
        let segs = debug_shape_segments(&DebugShape::Cross {
            center: Vec2::new(5.0, 5.0),
            half: 8.0,
            width: 2.0,
        });
        assert_eq!(segs.len(), 2);
        // 网格：40×20 每 10px → 竖线 0,10,20,30,40 = 5 条；横线 0,10,20 = 3 条
        let segs = debug_shape_segments(&DebugShape::Grid {
            rect: Rect::new(0.0, 0.0, 40.0, 20.0),
            spacing: 10.0,
            width: 1.0,
        });
        assert_eq!(segs.len(), 5 + 3, "40px 宽每 10px 一条（含两端）→ 5 条；20px 高 → 3 条");
        // segments 下限：segments=0 → 按 3 处理
        let segs = debug_shape_segments(&DebugShape::CircleOutline {
            center: Vec2::ZERO,
            radius: 5.0,
            segments: 0,
            width: 1.0,
        });
        assert_eq!(segs.len(), 3);
    }

    #[test]
    fn debug_kind_group_is_last() {
        // 调试图元分组 2（恒排在图形 0 / 文字 1 之后）
        assert_eq!(
            DrawKind::Debug {
                color: Color::WHITE,
                shape: DebugShape::Line { a: Vec2::ZERO, b: Vec2::ONE, width: 1.0 },
            }
            .group(),
            2
        );
        assert_eq!(DrawKind::Solid(Color::WHITE).group(), 0);
    }

    #[test]
    fn text_block_offset_uses_integer_operands_only() {
        // 不变量：UI 文本定位的所有加/减法操作数必须为整数（防止误差累加 / 亚像素摆放）。
        // content（物理尺寸）与 first_line_top（行盒顶）均为整数输入。
        // 居中 + 奇数内容宽 21：off_x = -round(10.5) = -11（整数）；
        // 垂直：-(-7) - round(17/2) = 7 - 9 = -2（整数）。
        let off = text_block_offset(TextAlign::Center, TextVAlign::Center, Vec2::new(21.0, 17.0), -7.0);
        assert_eq!(off, Vec2::new(-11.0, -2.0), "居中奇数宽 + 行盒偏移应为整数");
        assert_eq!(off.x.fract(), 0.0, "off.x 必须为整数像素，实际 {}", off.x);
        assert_eq!(off.y.fract(), 0.0, "off.y 必须为整数像素，实际 {}", off.y);
        // 左对齐：水平偏移恒为 0
        let off_l = text_block_offset(TextAlign::Left, TextVAlign::Center, Vec2::new(21.0, 17.0), -7.0);
        assert_eq!(off_l.x, 0.0);
        assert_eq!(off_l.y.fract(), 0.0);
        // 右对齐：-content_w
        let off_r = text_block_offset(TextAlign::Right, TextVAlign::Center, Vec2::new(30.0, 17.0), -7.0);
        assert_eq!(off_r.x, -30.0);
        // 偶数宽/高：取整无偏差（round(20/2)=10、round(16/2)=8）
        let off_e = text_block_offset(TextAlign::Center, TextVAlign::Center, Vec2::new(20.0, 16.0), -6.0);
        assert_eq!(off_e, Vec2::new(-10.0, -2.0));
        // 空文本（content = 0）：偏移为 0
        assert_eq!(
            text_block_offset(TextAlign::Center, TextVAlign::Center, Vec2::ZERO, 0.0),
            Vec2::ZERO
        );
        // 锚点 + 偏移 = 整数 + 整数：任意组合结果恒为整数
        for anchor in [Vec2::new(100.0, 200.0), Vec2::new(0.5, -3.5).round()] {
            let block = anchor + text_block_offset(TextAlign::Center, TextVAlign::Center, Vec2::new(21.0, 17.0), -7.0);
            assert_eq!(block.x.fract(), 0.0, "block.x 必须为整数，实际 {}", block.x);
            assert_eq!(block.y.fract(), 0.0, "block.y 必须为整数，实际 {}", block.y);
        }
    }

    #[test]
    fn text_block_offset_top_aligns_to_rect_top() {
        // TextVAlign::Top：垂直偏移 = -first_line_top（行盒顶对齐矩形顶，整数不变量）。
        let off = text_block_offset(TextAlign::Left, TextVAlign::Top, Vec2::new(100.0, 60.0), -7.0);
        assert_eq!(off, Vec2::new(0.0, 7.0));
        assert_eq!(off.y.fract(), 0.0);
        // 多行内容（行盒高 60）：Top 与 Center 的差异 = round(60/2) = 30。
        let top = text_block_offset(TextAlign::Left, TextVAlign::Top, Vec2::new(100.0, 60.0), -7.0);
        let ctr = text_block_offset(TextAlign::Left, TextVAlign::Center, Vec2::new(100.0, 60.0), -7.0);
        assert_eq!(ctr.y - top.y, -30.0, "Center 比 Top 上移半个内容高");
    }

    #[test]
    fn translate_moves_rect_but_not_absolute_clip() {
        // 回归：滚动容器内容被错误裁掉（list_at 不显示）的根因——
        // `clip` 是绝对逻辑坐标，`translate` 只应平移局部坐标的 `rect`。
        let mut d = UiDraw {
            depth: 1,
            seq: 0,
            win: 0,
            elem: 1,
            rect: Rect::new(0.0, 0.0, 20.0, 10.0),
            clip: Some(Rect::new(880.0, 130.0, 240.0, 300.0)),
            kind: DrawKind::Solid(Color::WHITE),
        };
        d.translate(Vec2::new(880.0, 120.0));
        // rect：局部 → 绝对（随容器平移）
        assert_eq!(d.rect, Rect::new(880.0, 120.0, 20.0, 10.0));
        // clip：绝对坐标，**不得**再加一次偏移
        assert_eq!(d.clip, Some(Rect::new(880.0, 130.0, 240.0, 300.0)));
    }
}

/// **`Gradient` 顶点色语义**（无纹理路径的契约）。
///
/// 这些测试是「渐变不依赖纹理」这一决定的守卫：四角颜色就是**全部**渲染输入，
/// 因此每个构造器都必须能精确预测四角色。同时锁定 `rotated` 的角度约定
/// （`0` = 下→上、`PI/2` = 左→右），它是最容易写反的地方。
#[cfg(test)]
mod gradient_tests {
    use super::{Gradient, lerp_color};
    use rjw_color::Color;

    /// 比较颜色（逐分量，容差 `eps`）。
    fn approx(a: Color, b: Color, eps: f32) -> bool {
        let (af, bf): ([f32; 4], [f32; 4]) = (a.into(), b.into());
        af.iter().zip(bf.iter()).all(|(x, y)| (x - y).abs() <= eps)
    }

    #[test]
    fn pure_is_uniform() {
        let g = Gradient::pure(Color::RED);
        assert!(approx(g.tl, Color::RED, 0.0));
        assert!(approx(g.tr, Color::RED, 0.0));
        assert!(approx(g.bl, Color::RED, 0.0));
        assert!(approx(g.br, Color::RED, 0.0));
    }

    /// `vertical(top, bottom)`：上两点 = top，下两点 = bottom。
    #[test]
    fn vertical_pairs_top_and_bottom() {
        let g = Gradient::vertical(Color::RED, Color::BLUE);
        assert!(approx(g.tl, Color::RED, 0.0), "左上 = top");
        assert!(approx(g.tr, Color::RED, 0.0), "右上 = top");
        assert!(approx(g.bl, Color::BLUE, 0.0), "左下 = bottom");
        assert!(approx(g.br, Color::BLUE, 0.0), "右下 = bottom");
    }

    /// `horizontal(left, right)`：左两点 = left，右两点 = right。
    #[test]
    fn horizontal_pairs_left_and_right() {
        let g = Gradient::horizontal(Color::RED, Color::BLUE);
        assert!(approx(g.tl, Color::RED, 0.0), "左上 = left");
        assert!(approx(g.bl, Color::RED, 0.0), "左下 = left");
        assert!(approx(g.tr, Color::BLUE, 0.0), "右上 = right");
        assert!(approx(g.br, Color::BLUE, 0.0), "右下 = right");
    }

    /// `corners(..)`：四角**原样**（含双线性不一致的情形——1D 纹理做不到）。
    #[test]
    fn corners_are_verbatim() {
        let g = Gradient::corners(Color::RED, Color::GREEN, Color::BLUE, Color::YELLOW);
        assert!(approx(g.tl, Color::RED, 0.0));
        assert!(approx(g.tr, Color::GREEN, 0.0));
        assert!(approx(g.bl, Color::BLUE, 0.0));
        assert!(approx(g.br, Color::YELLOW, 0.0));
    }

    /// **角度约定**：`rotated(.., 0)` == `vertical`，`rotated(.., PI/2)` == `horizontal`。
    /// 这条锁死「0° 是下→上而不是左→右」。
    #[test]
    fn rotation_cardinals_match_vertical_and_horizontal() {
        let (a, b) = (Color::RED, Color::BLUE);
        let v = Gradient::vertical(a, b);
        let r0 = Gradient::rotated(a, b, 0.0);
        assert!(approx(r0.tl, v.tl, 1e-6) && approx(r0.br, v.br, 1e-6), "0° 应等于 vertical");

        let h = Gradient::horizontal(a, b);
        let r90 = Gradient::rotated(a, b, std::f32::consts::FRAC_PI_2);
        assert!(approx(r90.tl, h.tl, 1e-5), "90° 左上应 = left");
        assert!(approx(r90.tr, h.tr, 1e-5), "90° 右上应 = right");
    }

    /// `rotated` 的两端颜色必须**恰好落在矩形的两个极角上**（不会「只渐变一半」）：
    /// 存在一对角分别是纯 `from` 与纯 `to`。
    #[test]
    fn rotated_endpoints_reach_both_extremes() {
        for angle in [0.3f32, 1.0, 2.0, 4.0, -0.7] {
            let (from, to) = (Color::RED, Color::BLUE);
            let g = Gradient::rotated(from, to, angle);
            let corners = [g.tl, g.tr, g.bl, g.br];
            let has_from = corners.iter().any(|c| approx(*c, from, 1e-4));
            let has_to = corners.iter().any(|c| approx(*c, to, 1e-4));
            assert!(has_from, "angle={angle}: 应有一角为纯 from");
            assert!(has_to, "angle={angle}: 应有一角为纯 to");
        }
    }

    /// 非有限角度 → 退化为 `vertical`（不产生 NaN 颜色）。
    #[test]
    fn rotated_non_finite_falls_back() {
        let v = Gradient::vertical(Color::RED, Color::BLUE);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let g = Gradient::rotated(Color::RED, Color::BLUE, bad);
            assert!(approx(g.tl, v.tl, 0.0) && approx(g.br, v.br, 0.0), "{bad} 应退化 vertical");
        }
    }

    /// `Color: Into<Gradient>`（纯色）——让 `gradient_rect_at` 直接收 `Color`。
    #[test]
    fn color_converts_to_pure_gradient() {
        let g: Gradient = Color::GREEN.into();
        assert!(approx(g.tl, Color::GREEN, 0.0) && approx(g.br, Color::GREEN, 0.0));
    }

    /// `lerp_color` 端点与中点（`Gradient::rotated` 的采样基础）。
    ///
    /// 注意：4 个分量**都**参与插值——`BLACK`→`WHITE` 的 alpha 是 `1.0 → 1.0 = 1.0`，
    /// 所以中点只有 RGB 是 0.5（这条测试曾因为误断言 alpha 也变 0.5 而失败）。
    #[test]
    fn lerp_color_endpoints_and_midpoint() {
        assert!(approx(lerp_color(Color::BLACK, Color::WHITE, 0.0), Color::BLACK, 0.0));
        assert!(approx(lerp_color(Color::BLACK, Color::WHITE, 1.0), Color::WHITE, 0.0));
        let mid: [f32; 4] = lerp_color(Color::BLACK, Color::WHITE, 0.5).into();
        for (i, v) in mid[..3].iter().enumerate() {
            assert!((v - 0.5).abs() <= 1e-6, "RGB 分量 {i} 中点应为 0.5，实际 {v}");
        }
        assert!((mid[3] - 1.0).abs() <= 1e-6, "alpha 1→1 应保持 1.0，实际 {}", mid[3]);
    }
}

// ─── 面板命令（纯函数：绘制器 `panel*` 与单测共用） ──────────────

/// 一条面板命令的公共字段（[`push_panel_img_cmds`] 的入参；`seq` = 背景刷那条命令的序号）。
pub(crate) struct PanelCmdCtx {
    pub depth: u32,
    pub win: u32,
    pub elem: u32,
    pub rect: Rect,
    pub clip: Option<Rect>,
    pub seq: u32,
}

/// **面板的三层命令**（背景刷 → 背景图 → 边框）——从面板绘制原语里提出来的**纯函数**：
/// 不碰 `Ui` / 绘制器，因此可以直接单测"直角面板到底推了哪几条命令"。
///
/// 为什么值得单独成函数：**背景图曾在 `radius == 0`（直角）分支里被静默丢掉**
/// （"Tile（1:1 平铺，直角）"那个窗口就是因为这条而空白）。当时的方法体把"背景刷
/// 形状"和"要不要画图 / 边框"混在同一个 `if radius` 里，看代码很难一眼发现。现在：
/// 背景刷按 `(是否直角, 是否纯色)` 一次 `match` 定形，**图与边框移出分支、无条件执行**。
pub(crate) fn push_panel_img_cmds(
    out: &mut Vec<UiDraw>,
    ctx: PanelCmdCtx,
    bg: &crate::style::Brush,
    img: Option<ImageBg>,
    border: Color,
    border_w: f32,
    radius: CornerRadius,
) {
    let PanelCmdCtx { depth, win, elem, rect, clip, seq } = ctx;
    // 背景图的 seq 夹在"背景刷"与"边框"之间（同 elem 内按 seq 排序 ⇒ 层次正确）。
    let img_seq = seq + 1;
    let border_seq = if img.is_some() { seq + 2 } else { seq + 1 };
    // 渐变锚定在 `rect` 上（`resample_gradient_local` 保证裁剪不改变颜色锚定）。
    let grad = Gradient::corners(
        bg.corners()[0],
        bg.corners()[1],
        bg.corners()[2],
        bg.corners()[3],
    );
    // **背景刷**：圆角 = 一整块 `RoundedRect`（渐变四角色直接给它，无需内缩重采样）；
    // 直角 = `Solid` / `Rect`（少一次镶嵌）。
    //
    // 圆角分支的历史：旧实现是"外圈 border 色实心圆角 + 内圈 bg 色实心圆角"，两块
    // 的抗锯齿边缘会在圆角处各混合一次（看起来发灰、边缘偏粗）；现在边框是**环带**，
    // 只画一次边界。
    let kind = match (radius.is_zero(), bg.as_solid()) {
        (false, Some(c)) => DrawKind::RoundedRect { corners: [c; 4], radius },
        (false, None) => DrawKind::RoundedRect {
            corners: [grad.tl, grad.tr, grad.bl, grad.br],
            radius,
        },
        (true, Some(c)) => DrawKind::Solid(c),
        (true, None) => DrawKind::Rect(grad),
    };
    out.push(UiDraw { depth, seq, win, elem, rect, clip, kind });
    // **背景图**（背景刷之上、边框之下）：圆角遮罩用面板 radius（直角时半径 0 ⇒ 不裁）。
    // ⚠ 与半径**无关**：直角面板同样要画图（历史 bug 就在这里）。
    if let Some(mut img) = img {
        img.radius = radius;
        out.push(UiDraw {
            depth,
            seq: img_seq,
            win,
            elem,
            rect,
            clip,
            kind: DrawKind::Image(img),
        });
    }
    // **边框**（最上层）：直角时 `radius` 本身就是 0，无需另写 `CornerRadius::default()`。
    if border_w > 0.0 {
        out.push(UiDraw {
            depth,
            seq: border_seq,
            win,
            elem,
            rect,
            clip,
            kind: DrawKind::Border { color: border, width: border_w, radius },
        });
    }
}
