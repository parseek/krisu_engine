//! 绘制原语与文本缓冲：调试图元、圆角 / 渐变 / 图标 / 背景图、面板与配件的 push、
//! 文本度量与共享排版缓冲缓存、`label_at` / `label_wrap_at`。
//!
//! 维护者笔记：本模块是「引擎侧原语」——可以被控件层调用，但**不得**反向依赖
//! `crate::widgets`（依赖方向见 `docs/UI_ARCHITECTURE.md` §2.5.1）。新增原语请优先
//! 补在 `Painter`（`crate::painter`）；只有需要 `Ui` 的 ID / 主题 / 帧状态时才落在这里。

use super::*;
use std::sync::Arc;

use glam::Vec2;
use rjw_color::Color;
use rjw_text::{Align, Buffer, CachePolicy, Text, TextStyle};
use rjw_transform::Rect;

use crate::draw::{
    CornerRadius, DebugShape, DrawKind, Gradient, Icon, ImageBg, Position, Size, TextAlign,
    TextRamp, TextVAlign, text_cmd,
};
use crate::edit::caret_index_by_width;
use crate::layout::Child;
use crate::painter::Painter;
use crate::state::TEXT_BUFFER_CACHE_CAP;
use crate::style::{GripShape, GripStyle};

impl<'a> Ui<'a> {
    /// 屏幕空间线段（**绝对逻辑屏幕像素**，Y+ 向下；覆盖在 UI 内容之上；接受 `Vec2` 或 `(x, y)`）。
    pub fn debug_line(&mut self, a: impl Into<Vec2>, b: impl Into<Vec2>, width: f32, color: Color) {
        self.push_debug(
            DebugShape::Line {
                a: a.into(),
                b: b.into(),
                width,
            },
            color,
        );
    }

    /// 屏幕空间矩形边框（逻辑像素）。
    pub fn debug_rect_outline(&mut self, rect: Rect, width: f32, color: Color) {
        self.push_debug(DebugShape::RectOutline { rect, width }, color);
    }

    /// 屏幕空间圆环（`segments` 段折线近似；逻辑像素；接受 `Vec2` 或 `(x, y)`）。
    pub fn debug_circle_outline(
        &mut self,
        center: impl Into<Vec2>,
        radius: f32,
        segments: usize,
        width: f32,
        color: Color,
    ) {
        self.push_debug(
            DebugShape::CircleOutline {
                center: center.into(),
                radius,
                segments,
                width,
            },
            color,
        );
    }

    /// 屏幕空间十字标记（逻辑像素；接受 `Vec2` 或 `(x, y)`）。
    pub fn debug_cross(&mut self, center: impl Into<Vec2>, half: f32, width: f32, color: Color) {
        self.push_debug(
            DebugShape::Cross {
                center: center.into(),
                half,
                width,
            },
            color,
        );
    }

    /// 屏幕空间网格（`rect` 范围内按 `spacing` 竖线 + 横线；每方向最多 512 条）。
    pub fn debug_grid(&mut self, rect: Rect, spacing: f32, width: f32, color: Color) {
        self.push_debug(DebugShape::Grid { rect, spacing, width }, color);
    }

    /// 录制一条屏幕空间调试图元（进 `debug_queue`，坐标 = 绝对逻辑像素）。
    fn push_debug(&mut self, shape: DebugShape, color: Color) {
        self.painter.debug(shape, color);
    }

    /// **圆角矩形**（背景填充原语；绝对定位，`radius` 带单位）。
    ///
    /// **无纹理、无着色器改动**：CPU 把矩形镶嵌成三角形（硬体 + 边缘羽化带，
    /// 见 `crate::tess`）。半径接受任意值（含小数）；四角之和超过边长时按 CSS 规则
    /// **等比收缩**（[`CornerRadius::fit`]）；四角**全**为 0 时退化成普通四边形。
    ///
    /// `radius` 接受 `f32`（四角相同）或 [`CornerRadius`]（**只圆某些角**）：
    ///
    /// ```no_run
    /// # use rjw_ui::{CornerRadius, Position, Size, Ui};
    /// # fn demo(ui: &mut Ui, pos: Position, size: Size<glam::Vec2>) {
    /// // 四角相同
    /// ui.rounded_rect_at(pos, size, 8.0, rjw_color::Color::RED);
    /// // 只圆上面两个角（标签页 / 附着在工具栏下方的面板）
    /// ui.rounded_rect_at(
    ///     pos,
    ///     size,
    ///     CornerRadius { tl: 10.0, tr: 10.0, br: 0.0, bl: 0.0 },
    ///     rjw_color::Color::RED,
    /// );
    /// # }
    /// ```
    pub fn rounded_rect_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        radius: impl Into<Size<CornerRadius>>,
        color: Color,
    ) {
        self.painter.rounded_at(pos, size, radius, color);
    }

    /// **矢量图标**（绝对定位；`size` 为图标方框）。
    ///
    /// 图标是**画出来的几何**（[`Icon`]，单位方框内的凸多边形分片），与字体无关——
    /// `▾` / `✓` / `≡` 这类字符的可用性与宽度全由字体决定，字体缺字形就走 fallback。
    /// 几何按 [`Theme::feather`] 做边缘羽化（与圆角矩形同一套顶点 alpha 插值）。
    ///
    /// 框非方形时按 `min(w, h)` **居中等比**（图标永不形变）。
    ///
    /// ```no_run
    /// # use rjw_ui::{Icon, Position, Size, Ui};
    /// # fn demo(ui: &mut Ui, pos: Position, size: Size<glam::Vec2>) {
    /// ui.icon_at(pos, size, Icon::ChevronDown, rjw_color::Color::WHITE);
    /// # }
    /// ```
    pub fn icon_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        icon: Icon,
        color: Color,
    ) {
        self.painter.icon_at(pos, size, icon, color);
    }

    /// **矢量图标**（随布局流排布；与 [`Self::icon_at`] 同语义，位置来自当前容器游标）。
    ///
    /// 在 [`crate::UiAdd::row`] 容器内连续调用即得一条工具栏；`size` 决定图标方框
    /// （非方形时按 `min(w, h)` 居中等比，笔画不形变）。
    ///
    /// ```no_run
    /// # use rjw_ui::{Icon, Ui};
    /// # fn demo(ui: &mut Ui, c: rjw_color::Color) {
    /// ui.icon(glam::Vec2::new(16.0, 16.0), Icon::Check, c);
    /// # }
    /// ```
    pub fn icon(&mut self, size: impl Into<Size<Vec2>>, icon: Icon, color: Color) {
        let size = size.into().to_physical(self.scale);
        let pos = self.child_rect(size.x, size.y, Child::Expand).min();
        self.painter.icon_at(Position::Physical(pos), Size::Physical(size), icon, color);
    }

    /// **背景图**（绝对定位；`ImageBg` 决定铺排 / 染色 / 圆角遮罩）。
    ///
    /// 与 [`Self::rounded_rect_at`] 同一条 CPU 镶嵌路径：`Stretch` / `Fill` / `Center`
    /// 的 UV 是仿射映射 ⇒ **贴图与圆角遮罩共存**，且不额外产生 draw call（图片按纹理
    /// 切段，与字形 / 白纹理各一段）。
    ///
    /// ```no_run
    /// # use rjw_ui::{ImageBg, ImageFit, Position, Size, Ui};
    /// # fn demo(ui: &mut Ui, pos: Position, size: Size<glam::Vec2>, tex: u64) {
    /// // 等比覆盖 + 圆角遮罩
    /// let bg = ImageBg::new(tex, glam::Vec2::new(64.0, 64.0)).fit(ImageFit::Fill).radius(8.0);
    /// ui.image_at(pos, size, bg);
    /// # }
    /// ```
    pub fn image_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        bg: ImageBg,
    ) {
        self.painter.image_at(pos, size, bg);
    }

    /// **背景图**（随布局流排布；与 [`Self::image_at`] 同语义，位置来自当前容器游标）。
    pub fn image(&mut self, size: impl Into<Size<Vec2>>, bg: ImageBg) {
        let size = size.into().to_physical(self.scale);
        let pos = self.child_rect(size.x, size.y, Child::Expand).min();
        self.painter.image_at(Position::Physical(pos), Size::Physical(size), bg);
    }

    /// **矩形渐变**（绝对定位；背景填充原语）。
    ///
    /// `gradient` 接受 [`Gradient`] 或 `Color`（`Color: Into<Gradient>`，等价纯色）：
    ///
    /// ```ignore
    /// ui.gradient_rect_at(pos, size, Color::RED);                                  // 纯色
    /// ui.gradient_rect_at(pos, size, Gradient::vertical(Color::RED, Color::BLUE)); // 上下
    /// ui.gradient_rect_at(pos, size, Gradient::horizontal(a, b));                  // 左右
    /// ui.gradient_rect_at(pos, size, Gradient::rotated(a, b, 0.5));                // 任意角
    /// ui.gradient_rect_at(pos, size, Gradient::corners(tl, tr, bl, br));           // 四角
    /// ```
    ///
    /// **无纹理**：四角颜色经顶点色由光栅化器双线性插值（详见 [`Gradient`]）。
    pub fn gradient_rect_at(
        &mut self,
        pos: impl Into<Position>,
        size: impl Into<Size<Vec2>>,
        gradient: impl Into<Gradient>,
    ) {
        self.painter.gradient_at(pos, size, gradient);
    }

    /// **矩形渐变**（随布局流排布；与 [`Self::gradient_rect_at`] 同语义，位置来自当前容器游标）。
    pub fn gradient_rect(
        &mut self,
        size: impl Into<Size<Vec2>>,
        gradient: impl Into<Gradient>,
    ) {
        let size = size.into().to_physical(self.scale);
        let pos = self.child_rect(size.x, size.y, Child::Expand).min();
        self.painter.gradient_at(Position::Physical(pos), Size::Physical(size), gradient);
    }

    /// **元素序提示**（控件作者用）：取"当前录制位置"的元素序（`seq + 1`）。
    ///
    /// `elem` 决定**同一窗口内**命令的绘制顺序（排序键 `(win, depth, elem, group, seq)`）：
    /// `push_panel_like` / `push_text_rect` / `slider_at` 等原语各自取"录制时的
    /// `seq + 1`"，而 [`Self::push_draw`] 默认写 `elem = 0`（容器装饰，画在本容器
    /// **所有元素之下**，如窗口背景/边框）。
    ///
    /// **组合控件里"后画的装饰"必须用本方法**：展开箭头、拖拽手柄、分隔线若写死
    /// `0` / `1`，就会被本控件自己的背景 / 文本框盖住（历史 bug：`NumberInput` 的
    /// 拖拽手柄与分隔线、`ColorPicker` 的展开箭头整块看不见——元素序小的先画）。
    ///
    /// 等价于 `ui.painter().elem_hint()`（[`crate::Painter`] 的默认 elem 就是它）。
    #[inline]
    pub fn elem_hint(&self) -> u32 {
        self.painter.elem_hint()
    }

    /// **借出绘制器**（[`crate::Painter`]）：一个**绘制块**取一次，块内所有原语
    /// 不再逐参数传 `elem` / 环境裁剪。
    ///
    /// ```no_run
    /// # use rjw_transform::Rect;
    /// # use rjw_ui::Ui;
    /// # fn demo(ui: &mut Ui, rect: Rect) {
    /// let mut p = ui.painter();
    /// p.panel(rect, rjw_color::Color::BLACK, rjw_color::Color::WHITE, 1.0, 4.0);
    /// # }
    /// ```
    ///
    /// **沙箱内直接可用**：Clip 沙箱 / ScrollView 可视区 / 严格窗口内容裁剪就是
    /// [`Painter::clip`] 上的**当前强制层**（`view_at` / `scroll_at` 进入时写入），
    /// 录出的命令自带它 ⇒ 沙箱里的控件**不需要**任何额外动作。只有"要一层与当前
    /// **不同**的裁剪"时才用 [`Self::painter_clipped`]（更窄，或 `None` 主动不裁）。
    ///
    /// ⚠ **绘制与 `ui.*` 不能交错**：本方法借 `&mut self`，painter 存活期内
    /// `ui.text_size` / `ui.hit_abs` / `ui.allocate*` 都借不到 `ui`。
    /// 顺序是「先量 → 画 → 再量」，不是一个 painter 画到底。
    #[inline]
    pub fn painter(&mut self) -> &mut Painter {
        &mut self.painter
    }

    /// **当前录制深度**（容器嵌套层数；`0` = `win=0` 顶层内容）。
    ///
    /// **控件作者公开面**：少数"顶层与容器内语义不同"的控件要看它 —— 目前唯一用户是
    /// [`crate::widgets::Divider`]（容器内的水平线要**满宽**，`win=0` 顶层则维持
    /// "可用宽 / 默认 120"，见 [`Self::mark_last_full_w`]）。
    #[inline]
    pub fn painter_depth(&self) -> u32 {
        self.painter.q.depth
    }

    /// **把"刚录的那条命令"标成待定满宽**（[`crate::draw::UiDraw::full_w`]；控件作者用）。
    ///
    /// 语义与约束见 [`Self::divider_full_w`]：所属容器结算尺寸后由
    /// [`Self::expand_pending_full_w`] 把 `rect.w` 回填成"容器内容宽"，且**只增不减**。
    /// 调用方必须**紧接着**录完那条命令就调它（按 `seq` 命中最后一条）。
    #[inline]
    pub fn mark_last_full_w(&mut self) {
        let seq = self.painter.q.seq;
        self.painter.q.mark_full_w(seq);
    }

    /// **借出绘制器并覆盖环境裁剪层**（只影响闭包内录的命令，块外自动恢复）。
    ///
    /// 等价于 `ui.painter().clipped(clip, |p| …)`；`clip = None` = 不裁剪。
    /// 用于"本控件内容裁到自己框内、但**不改** `Ui` 的强制裁剪层"——改那一层会连带
    /// 影响子控件 / 兄弟控件（`view_at` / `scroll_at` 就是这么用的）。
    ///
    /// ⚠ **已经在 Clip 沙箱里时不需要它**：沙箱裁剪已经是 painter 的当前层，
    /// 见 [`Self::painter`]。
    #[inline]
    pub fn painter_clipped<R>(
        &mut self,
        clip: Option<Rect>,
        f: impl FnOnce(&mut Painter) -> R,
    ) -> R {
        self.painter.clipped(clip, f)
    }

    /// 录制一条绘制命令（`elem` 由调用方给：`0` = 容器装饰层，画在本容器元素之下）。
    ///
    /// ⚠ 组合控件内"画在自家背景之上"的装饰传 [`Self::elem_hint`]，**不要**写死 `0`。
    pub(crate) fn push_draw(&mut self, kind: DrawKind, rect: Rect, elem: u32) {
        self.painter.draw(kind, rect, elem);
    }

    /// 按样式 push **背景 + 边框**。
    ///
    /// - `radius > 0`：外圈 border 色圆角 + 内圈背景刷圆角（内缩 `border_w`），
    ///   即"圆角边框"。两层都由 CPU 镶嵌（`crate::tess`），因此**圆角与渐变可共存**。
    /// - `radius == 0`：`Solid` + `Border`；背景刷为渐变时走 `Rect(Gradient)`。
    ///
    /// `bg` 接受 [`Color`] 或 [`crate::Brush`]。渐变**锚定在 `rect` 上**——
    /// 内圈即使内缩 `border_w`，颜色按其在 `rect` 中的相对位置重采样，
    /// 不会整体平移（`resample_gradient`）。
    ///
    /// `elem`：元素序（装饰背景传 0；控件背景传 `self.painter.q.seq + 1`）。
    /// **控件作者绘制原语**（逻辑坐标，内部 ×scale 取整到物理像素）。
    #[allow(clippy::too_many_arguments)]
    pub fn push_panel_like(
        &mut self,
        rect: Rect,
        bg: impl Into<crate::style::Brush>,
        border: Color,
        border_w: f32,
        radius: impl Into<CornerRadius>,
        elem: u32,
    ) {
        self.push_panel_like_img(rect, bg, None, border, border_w, radius, elem);
    }

    /// **投影**（窗口 / 面板的**顶点色软阴影**；控件作者绘制原语）。
    ///
    /// 画在 `rect`（本体矩形）**之下**：命令 `elem = 0`（容器装饰层，先于本体背景入队）
    /// ⇒ 本体背景、内容、边框都盖在它上面；而窗口自己的整段提交在更低 z 的窗口之后 ⇒
    /// **投影落在下面的窗口上**（正确的投影观感）。
    ///
    /// `rect` 即本体矩形（**内轮廓恒在本体边缘**，浓度从本体边向外单调衰减，没有
    /// "等浓度暗带"——`shadow.offset` 由镶嵌器按圈数分摊，见 [`crate::tess`]）。
    /// 纯顶点色 + CPU 镶嵌：无纹理、无新 draw call，且**进窗口顶点缓存**（静态窗口零开销）。
    /// `shadow.blur <= 0`（或全透明色）时不产生任何命令。
    pub fn push_panel_shadow(
        &mut self,
        rect: Rect,
        shadow: &crate::style::ShadowStyle,
        radius: impl Into<CornerRadius>,
    ) {
        self.painter.shadow(rect, shadow, radius);
    }

    /// **右下角缩放柄图案**（窗口/面板局部坐标；`size` = 本体尺寸）。
    ///
    /// 只画图案 —— **命中区不在这里**（在 `window_impl` 里由 [`Self::resize_handle`] 建立，
    /// 其大小跟随 `grip.extent()`）。所以 [`GripShape::Hidden`] 时"看不见但仍能拖"。
    ///
    /// 形状 / 颜色 / 尺寸 / 个数全部来自 [`GripStyle`]：`Squares` = 沿右下对角线的
    /// 递减小方块（历史观感）、`Bars` = 内置矢量图标 [`Icon::Grip`]（三条横线，
    /// 与字体无关）、`Hidden` = 不画。
    pub fn push_resize_grip(&mut self, size: Vec2, grip: &GripStyle) {
        self.push_resize_grip_at(Rect::new(0.0, 0.0, size.x, size.y), grip);
    }

    /// 同 [`Self::push_resize_grip`]，但**指定本体矩形**（当前容器局部坐标）：
    /// 图案画在该矩形的**右下角内侧**——文本输入框（`rect` 不为 `(0,0)`）用它。
    pub fn push_resize_grip_at(&mut self, rect: Rect, grip: &GripStyle) {
        if !grip.is_visible() {
            return;
        }
        let (x, y) = (rect.x, rect.y);
        let (w, h) = (rect.w, rect.h);
        match grip.shape {
            GripShape::Hidden => {}
            GripShape::Squares => {
                for k in 0..grip.count {
                    let o = grip.step * (k as f32 + 1.0);
                    self.push_solid_rect(
                        Rect::new(x + w - o, y + h - o, grip.size, grip.size),
                        grip.color,
                    );
                }
            }
            GripShape::Bars => {
                // 三条**实心横杠**（宽 = `size*count`，高 = `size`，间距 = `step`）。
                //
                // ⚠ 不用 `Icon::Grip` 图标：图标走 `push_icon`，每条杠只有 `size` 高
                // （默认 4 逻辑像素），再叠上 `Theme::feather` 的羽化带（默认 1 逻辑像素
                // ⇒ 每侧 0.5）就把三条糊成一坨（用户实测："三横看起来是斜的一坨"）。
                // 实心矩形没有羽化，任意尺寸都读得出三条。
                let bw = grip.size * grip.count as f32;
                let right = x + w - grip.step;
                for k in 0..grip.count {
                    let o = grip.step * (k as f32 + 1.0);
                    self.push_solid_rect(
                        Rect::new(right - bw, y + h - o, bw, grip.size),
                        grip.color,
                    );
                }
            }
            GripShape::Diagonal => {
                // 三条**斜线**（45°，从左下到右上）：一条斜线没法用 `push_solid_rect`
                // （它只有轴对齐矩形）⇒ 走矢量图标 `Icon::GripDiagonal`。
                // 方框取 `size*count*1.5`（比横线版大 1.5×）：三条斜线在 `size*count`
                // 的小方框里间距只有 ~1px，羽化会把它们糊成一片。
                let d = grip.size * grip.count as f32 * 1.5;
                let m = grip.step;
                self.icon_at(
                    Position::Physical(Vec2::new(x + w - d - m, y + h - d - m)),
                    Size::Physical(Vec2::splat(d)),
                    Icon::GripDiagonal,
                    grip.color,
                );
            }
        }
    }

    /// 同 [`Self::push_panel_like`]，另带可选**背景图**（[`ImageBg`]）。
    ///
    /// 绘制层次：**背景刷 → 背景图 → 边框**（图在刷之上，半透明图能透出底色；边框
    /// 恒盖住图的边缘）。圆角遮罩**恒用面板 `radius`**（图片自带的 `radius` 被忽略，
    /// 免得"图与面板圆角不一致"这种要靠肉眼发现的错）。
    ///
    /// ⚠ **背景图与边框在两种半径下都要画**：曾经它们被写在"圆角分支"里，于是
    /// `radius == 0`（直角）的面板**静默丢掉背景图**——演示里"Tile（1:1 平铺，直角）"
    /// 那个窗口就是受害者（平铺为了 UV 环绕特意用直角）。现在"背景刷"按
    /// `(是否直角, 是否纯色)` 一次 `match` 决定，图与边框移出分支。
    #[allow(clippy::too_many_arguments)]
    pub fn push_panel_like_img(
        &mut self,
        rect: Rect,
        bg: impl Into<crate::style::Brush>,
        img: Option<ImageBg>,
        border: Color,
        border_w: f32,
        radius: impl Into<CornerRadius>,
        elem: u32,
    ) {
        self.painter.panel_img_elem(rect, bg, img, border, border_w, radius, elem);
    }

    /// 取（或创建）文本排版缓冲，并测量其自然尺寸（**逻辑像素**，宽 = 内容宽，高 = 内容高）。
    ///
    /// 排版缓冲按**物理字号**（`size × scale` 取整到像素）创建并自持于
    /// [`UiState::text_buffers`]（[`CachePolicy::User`]，不推入 `rjw_text` 内部 LRU）：
    /// 静态标签每帧命中缓存，跳过重复整形；测量结果 ÷ scale 返回逻辑尺寸。
    ///
    /// **整数不变量**：物理尺寸（[`Text::measure_buffer`]，已取整）÷ scale 后**再取整**
    /// （`ceil`）返回——布局光标累加（`child_rect` 的 `cursor += h + gap`）与后续
    /// 加法链的操作数全部为整数（scale = 1.0 时测量结果本就是整数，无任何变化）。
    /// 
    /// **行高版本**：行高 = 字号（而非 1.2 倍），保证字形在文本框内垂直居中位置正确。
    /// 版本号改变时，所有缓存会自动失效，避免新旧行高混用。
    /// 取（或创建）文本排版缓冲，并测量其自然尺寸（**逻辑像素**，宽 = 内容宽，高 = 内容高）。
    ///
    /// 排版缓冲按**物理字号**（`size × scale` 取整到像素）创建并自持于
    /// [`UiState::text_buffers`]（[`CachePolicy::User`]，不推入 `rjw_text` 内部 LRU）：
    /// 静态标签每帧命中缓存，跳过重复整形；测量结果 ÷ scale 返回逻辑尺寸。
    ///
    /// **整数不变量**：物理尺寸（[`Text::measure_buffer`]，已取整）÷ scale 后**再取整**
    /// （`ceil`）返回——布局光标累加（`child_rect` 的 `cursor += h + gap`）与后续
    /// 加法链的操作数全部为整数（scale = 1.0 时测量结果本就是整数，无任何变化）。
    /// 
    /// **行高版本**：行高 = 字号（而非 1.2 倍），保证字形在文本框内垂直居中位置正确。
    /// 版本号改变时，所有缓存会自动失效，避免新旧行高混用。
    /// 测量文本自然尺寸（**物理像素**，ceil 取整；控件作者在 [`Widget::size`] 测量用；
    /// 内部排版缓冲按物理字号缓存，`family = None` = 系统默认字体）。
    pub fn text_size(&mut self, s: &str, size: f32, family: Option<&str>) -> Vec2 {
        let buf = self.cache_buffer(s, size, family);
        Text::measure_buffer(&buf).ceil()
    }

    /// 按**换行宽度**测量文本自然尺寸：`wrap > 0` 时文本在宽度内自动换行
    /// （宽 = min(自然宽, wrap)，高 = 行数 × 行高）；否则同 [`Self::text_size`]。
    pub fn text_size_wrap(&mut self, s: &str, size: f32, family: Option<&str>, wrap: f32) -> Vec2 {
        let buf = self.cache_buffer_wrap(s, size, family, wrap);
        Text::measure_buffer(&buf).ceil()
    }

    // 剪贴板快捷键（Ctrl+C/V/X/A）共用实现已迁至 `crate::edit::clipboard_shortcuts`
    // （纯逻辑，单行 / 多行输入框共用），本处不再保留。

    /// **控件自持排版缓冲**：`WidgetState::text_buf` 命中复用（key 含文本/字号/字体/
    /// 换行宽/**行距**/版本），未命中直接构建——**不写** `UiState::text_buffers` 全局缓存
    /// （文本频繁变化的输入框不污染静态标签缓存）。
    ///
    /// `line_mult`：行高 = 字号 × 行距倍率（1.0 = 无行距；TextArea 多行用
    /// `Theme::line_spacing`，默认 [`crate::DEFAULT_LINE_SPACING`] = 1.2 加行距）。
    /// 用两次独立借用实现（`self.state` 与 `self.text` 不能同时可变借用）。
    pub(super) fn ensure_text_buf(
        &mut self,
        id: &str,
        s: &str,
        size: f32,
        family: Option<&str>,
        wrap_logical: f32,
        line_mult: f32,
    ) -> Arc<Buffer> {
        // 字号 / 换行宽**物理**（内部全物理；取整到像素，防亚像素模糊）。
        let size_px = size.round();
        let wrap_px = wrap_logical.round().max(0.0);
        let mult_bits = line_mult.to_bits();
        let weight = self.theme.font_weight;
        let key = format!(
            "{s}\u{1}{size_px}\u{1}{}\u{1}{wrap_px}\u{1}{mult_bits}\u{1}{}\u{1}{TEXT_LINE_HEIGHT_VERSION}",
            family.unwrap_or(""),
            weight.0
        );
        if let Some((k, b)) = self.state.widgets.get(id).and_then(|w| w.text_buf.as_ref())
            && *k == key {
                return b.clone();
            }
        let lh = (size_px * line_mult.max(1.0)).round();
        // 样式：字号 / 行高（行高 = 字号 × 行距倍率）/ 左对齐 / 可选字体族 / 全局字重。
        // （`TextStyle` 是唯一文本样式类型；此处只做机械适配，排版输入与旧
        //  `Text::create_buffer_wrap` 完全一致。）
        let style = match family {
            Some(f) if !f.is_empty() => TextStyle::new().font_family(f),
            _ => TextStyle::new(),
        }
        .size(size_px)
        .line_height(lh)
        .weight(weight)
        .align(Align::Left);
        let buf = self.text.buffer(s, &style, wrap_px, CachePolicy::User);
        if let Some(ws) = self.state.widgets.get_mut(id) {
            ws.text_buf = Some((key, buf.clone()));
        }
        buf
    }

    /// 按字符**实际宽度**把点击位置（相对内容左缘，逻辑像素）映射为最近的光标 char 索引。
    ///
    /// 用"前缀宽度"二分（宽度随前缀长度单调不减）——混合中英文（字宽不同）时
    /// 比等比估算（`total_w × k / n`）精确；纯中文（等宽）两者一致。
    pub(super) fn caret_index_at_width(
        &mut self,
        value: &str,
        size: f32,
        family: Option<&str>,
        cx: f32,
    ) -> usize {
        let chars: Vec<char> = value.chars().collect();
        let n = chars.len();
        caret_index_by_width(n, cx, |k| {
            let s: String = chars[..k].iter().collect();
            self.text_size(&s, size, family).x
        })
    }

    /// 文本超宽时省略（内容自洽，noclip）：宽度 > `max_w` → 返回 "…" 截断串；
    /// 否则 `None`（原样绘制）。按钮 / 勾选 / 下拉等固定 rect 控件的文本自动省略
    /// （Resizable 窗口缩窄 / max 约束下不溢出）。
    pub(crate) fn ellipsized(
        &mut self,
        s: &str,
        size: f32,
        family: Option<&str>,
        max_w: f32,
    ) -> Option<String> {
        let natural = self.text_size(s, size, family).x;
        if natural > max_w {
            Some(
                crate::edit::ellipsize(s, max_w, |t| self.text_size(t, size, family).x)
                    .into_owned(),
            )
        } else {
            None
        }
    }

    /// 取（或创建）共享排版缓冲（物理字号取整到像素；`CachePolicy::User`：不进 rjw_text LRU）。
    /// 
    /// 行高 = 字号（`size_px`），保证文本框内字形垂直居中位置正确。
    /// 缓存键包含 `TEXT_LINE_HEIGHT_VERSION`，修改行高策略后旧缓存自动失效。
    pub(super) fn cache_buffer(&mut self, s: &str, size: f32, family: Option<&str>) -> Arc<Buffer> {
        self.cache_buffer_wrap(s, size, family, 0.0)
    }

    /// 取（或创建）共享排版缓冲（物理字号取整到像素；`CachePolicy::User`：不进 rjw_text LRU）。
    ///
    /// `wrap <= 0` = 不换行（默认宽裕宽度）；`> 0` = 按该**物理像素**宽度换行
    /// （换行宽度参与缓存键，不同宽度各自缓存）。
    ///
    /// 行高 = 字号（`size_px`）× 主题行距倍率（`wrap <= 0` 的单行文本不受行距影响：
    /// 盒子高度仍是字号，保证文本框内字形垂直居中的位置正确）。
    /// 缓存键包含 `TEXT_LINE_HEIGHT_VERSION` 与行距本身，修改行高策略后旧缓存自动失效。
    fn cache_buffer_wrap(
        &mut self,
        s: &str,
        size: f32,
        family: Option<&str>,
        wrap: f32,
    ) -> Arc<Buffer> {
        let size_px = size.round();
        let wrap_px = wrap.round().max(0.0);
        // 行距：只在**换行文本**上生效（单行文本的行盒 = 字号，见 `TEXT_LINE_HEIGHT_VERSION`）。
        let mult = if wrap_px > 0.0 { self.theme.line_spacing } else { 1.0 };
        let weight = self.theme.font_weight;
        // 归一成一个 [`TextStyle`]，旧元组键的语义**逐字段保留**：
        // 行距倍率折算成**显式行高**（`字号 × 倍率`），字重取全局令牌，对齐恒 `Left`。
        let style = match family {
            Some(f) if !f.is_empty() => TextStyle::new().font_family(f),
            _ => TextStyle::new(),
        }
        .size(size_px)
        .line_height((size_px * mult.max(1.0)).round())
        .weight(weight)
        .align(Align::Left);
        self.cached_buffer_styled(s, style, wrap_px)
    }

    /// **按 [`TextStyle`] 建 / 命中排版缓冲**（`wrap_px` 物理像素；`<= 0` = 不换行）——
    /// **控件作者公开面**：自定义控件要"按某套文本样式排版"时用它，与内置控件走同一套
    /// 缓存键（`TextKey`：字号 / 行高 / 行距 / 字体族 / 字重 / 斜体 / 拉伸 / 字距 / 换行宽）。
    ///
    /// ⚠ **测量与绘制要用同一个 `Arc<Buffer>`**：把它交给
    /// [`Self::push_text_rect_ramp`] / [`Self::push_text_rect`] 的 `buf` 参数——传 `None`
    /// 会让绘制期按主题默认样式重新取缓冲（逐控件样式全丢）。
    /// 只要尺寸时用 [`Self::text_size_styled`]。
    pub fn cache_buffer_styled(&mut self, s: &str, style: &TextStyle, wrap_px: f32) -> Arc<Buffer> {
        self.cached_buffer_styled(s, style.clone(), wrap_px)
    }

    /// **按 [`TextStyle`] 测量**文本自然尺寸（物理像素，ceil）——**控件作者公开面**：
    /// 自定义控件要"按某套文本样式排版并测量"时用它，与内部 `LabelEx` 走同一套缓存键
    /// （`wrap_px <= 0` = 不换行；`family` / 字重 / 斜体 / 字距 / 行高都随样式进键）。
    pub fn text_size_styled(&mut self, s: &str, style: &TextStyle, wrap_px: f32) -> Vec2 {
        let buf = self.cache_buffer_styled(s, style, wrap_px);
        Text::measure_buffer(&buf).ceil()
    }

    /// **排版缓冲的唯一建/查入口**：样式里一切**影响排版**的字段（字号 / 行高 / 行距 /
    /// 字体族 / 字重 / 斜体 / 拉伸 / 字距）都经 [`TextKey`] 进键 ⇒ 改了任一项都会重建
    /// （漏一个就表现为"改了字重文字不刷新"）。
    ///
    /// **水平对齐恒按 `Left` 排版**：UI 的水平对齐由 [`TextAlign`] 在**绘制期**算 anchor
    /// （见 [`Self::push_text_rect`]）——排版期再对齐一次会让多行文本左右位移两次。
    fn cached_buffer_styled(&mut self, s: &str, mut style: TextStyle, wrap_px: f32) -> Arc<Buffer> {
        style.size = style.size.round();
        style.align = Align::Left;
        let key = crate::state::TextKey::new(s, &style, wrap_px);
        if let Some(b) = self.state.text_buffers.get_mut(&key) {
            // 命中：刷新"最后使用帧号"（帧级近似 LRU 驱逐依据）
            b.1 = self.state.frame;
            return b.0.clone();
        }
        let buf = self.text.buffer(s, &style, wrap_px, CachePolicy::User);
        self.evict_text_buffers();
        self.state.text_buffers.insert(key, (buf.clone(), self.state.frame));
        buf
    }

    /// **排版缓冲满容量时驱逐**（帧级近似 LRU）：先清"本帧未使用"的（保留静态标签），
    /// 仍满（本帧全在用）则清最旧一条。**不整表清空**——否则动态文本（FPS 计数、日志等）
    /// 每帧变化会连带全部静态标签每帧重新整形（缓存抖动，debug 下可致录制耗时翻倍）。
    fn evict_text_buffers(&mut self) {
        if self.state.text_buffers.len() < TEXT_BUFFER_CACHE_CAP {
            return;
        }
        let frame = self.state.frame;
        self.state.text_buffers.retain(|_, (_, used)| *used == frame);
        if self.state.text_buffers.len() >= TEXT_BUFFER_CACHE_CAP
            && let Some(oldest) = self
                .state
                .text_buffers
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
        {
            self.state.text_buffers.remove(&oldest);
        }
    }

    /// 取（或创建）**自动换行**排版缓冲（`wrap_logical > 0`；`<= 0` = 不换行同
    /// [`Self::cache_buffer`]）。供 widget 层与绘制路径按"渲染与测量同缓冲"使用。
    /// 取（或创建）**按宽度换行**的排版缓冲（`wrap_logical <= 0` = 不换行）；
    /// 控件作者画多行/换行文本时用（配 [`Self::push_text_rect`] 的 `buf` 参数）。
    pub fn wrap_buffer(
        &mut self,
        s: &str,
        size: f32,
        family: Option<&str>,
        wrap_logical: f32,
    ) -> Arc<Buffer> {
        self.cache_buffer_wrap(s, size, family, wrap_logical)
    }

    /// **控件作者绘制原语**：实心矩形（逻辑坐标；`w/h <= 0` 跳过）。
    ///
    /// 等价于 `ui.painter().solid(rect, color)`；保留为方法是为了不改既有调用点。
    #[inline]
    pub fn push_solid_rect(&mut self, rect: Rect, color: Color) {
        self.painter.solid(rect, color);
    }

    /// **控件作者绘制原语**：矩形边框（逻辑坐标；画在矩形内边缘，宽度取整到物理像素）。
    ///
    /// 等价于 `ui.painter().border(rect, color, width)`。
    #[inline]
    pub fn push_border_rect(&mut self, rect: Rect, color: Color, width: f32) {
        self.painter.border(rect, color, width);
    }

    /// 推送一条文本绘制命令（供 widget 层与 `*_at` 方法共用；`clip` 为文本局部裁剪，
    /// 外层裁剪自动取当前容器 `self.painter.q.clip`；`buf = Some` 时直接用预排版缓冲）。
    /// **控件作者绘制原语**（逻辑坐标；`family` 传 `None` = 系统默认字体）。
    pub fn push_text_rect(
        &mut self,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
        family: Option<Arc<str>>,
        align: TextAlign,
        valign: TextVAlign,
        clip: Option<Rect>,
        buf: Option<Arc<Buffer>>,
    ) {
        self.painter.text(rect, text, size, color, family, align, valign, clip, buf);
    }

    /// **不服从内容裁剪的文本绘制**（控件作者原语）：
    ///
    /// 语义 = [`Self::push_text_rect`] 且**不附加任何软层（内容裁剪）**——调用方
    /// 承诺文本**内容自洽**（自动换行后高 = 自然高、"…"省略后宽 = 分配宽、滚动
    /// 内容受限），无需按控件边界裁剪。**仍服从强制层**（ScrollView 可视区 /
    /// Clip 沙箱，即 `self.painter.q.clip`）：父级如 ScrollView 强制裁切时躲不掉；无 Scroll
    /// 的普通容器本来就没有强制层 → 自洽内容画出界（自洽内容本就不会出界）。
    pub fn push_text_rect_noclip(
        &mut self,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
        family: Option<Arc<str>>,
        align: TextAlign,
        valign: TextVAlign,
        buf: Option<Arc<Buffer>>,
    ) {
        self.push_text_rect(rect, text, size, color, family, align, valign, None, buf)
    }

    /// **带首末两色渐变的文本绘制原语**（[`TextRamp`]）——[`Self::push_text_rect`] 的完整形态。
    ///
    /// `ramp = None` 时与 [`Self::push_text_rect`] 逐字等价（调用方不必分支）。
    #[allow(clippy::too_many_arguments)]
    pub fn push_text_rect_ramp(
        &mut self,
        rect: Rect,
        text: &str,
        size: f32,
        color: Color,
        family: Option<Arc<str>>,
        align: TextAlign,
        valign: TextVAlign,
        clip: Option<Rect>,
        buf: Option<Arc<Buffer>>,
        ramp: Option<TextRamp>,
    ) {
        self.painter
            .text_ramp(rect, text, size, color, family, align, valign, clip, buf, ramp);
    }

}

impl<'a> Ui<'a> {
    /// **扩展标签 builder**（[`crate::LabelEx`]）：裸 `Ui` 上的入口——链式设完
    /// `.show(ui)` 即录制（与容器里的 [`UiAdd::label_ex`](crate::ui::UiAdd::label_ex) 同形）。
    ///
    /// ```no_run
    /// # use rjw_ui::{Ui, UiAdd};
    /// # fn demo(ui: &mut Ui) {
    /// ui.label_ex("HP")
    ///     .tint(rjw_color::Color::GREEN)
    ///     .weight(rjw_ui::Weight::BOLD)
    ///     .show(ui);
    /// # }
    /// ```
    pub fn label_ex<'s>(&mut self, text: &'s str) -> crate::widgets::LabelEx<'s> {
        crate::widgets::LabelEx::new(text)
    }

    /// **彩色标签**（一步到位的语法糖）：等价 `ui.label_ex(text).tint(color).show(ui)`，
    /// 返回占位矩形（[`crate::widgets::Response`]）。
    ///
    /// ```no_run
    /// # use rjw_ui::{Ui, UiAdd};
    /// # fn demo(ui: &mut Ui) {
    /// ui.colored_label("HP 100", rjw_color::Color::GREEN);
    /// # }
    /// ```
    pub fn colored_label(
        &mut self,
        text: &str,
        color: Color,
    ) -> crate::widgets::Response {
        self.add(crate::widgets::LabelEx::new(text).tint(color))
    }

    /// 绝对定位标签（`pos` 相对当前容器内容原点；顶层即屏幕原点）。
    pub fn label_at(&mut self, pos: impl Into<Position>, text: &str) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        let elem = self.painter.q.seq + 1;
        let seq = self.next_seq();
        let style = self.theme.label.clone();
        let size = self.text_size(text, style.font_size, style.font_family.as_deref());
        let rect = Rect::new(pos.x, pos.y, size.x, size.y);
        self.note_placed(rect);
        self.painter.q.queue.push(text_cmd(
            self.painter.q.depth,
            seq,
            self.painter.q.cur_win,
            elem,
            rect,
            Arc::from(text),
            style.font_size,
            style.color,
            TextAlign::from(style.align),
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.painter.q.clip,
        None,
        ));
        size
    }

    /// **自动换行标签**：`max_w`（逻辑像素）内按词/字换行，返回自然尺寸
    /// （宽 = min(自然宽, max_w)，高 = 行数 × 行高）。`max_w <= 0` = 不换行（同 [`Self::label_at`]）。
    ///
    /// 换行宽度参与排版缓存键（不同宽度各自缓存）；多行文本垂直居中于矩形。
    pub fn label_wrap_at(&mut self, pos: impl Into<Position>, max_w: impl Into<Size<f32>>, text: &str) -> Vec2 {
        let pos = pos.into().to_physical(self.scale);
        let max_w = max_w.into().to_physical(self.scale);
        let elem = self.painter.q.seq + 1;
        let seq = self.next_seq();
        let style = self.theme.label.clone();
        let size = self.text_size_wrap(text, style.font_size, style.font_family.as_deref(), max_w);
        let rect = Rect::new(pos.x, pos.y, size.x, size.y);
        self.note_placed(rect);
        // 换行标签：直接传预排版缓冲（渲染与测量同一缓冲）——否则绘制期按不换行
        // 排版，长文本会单行溢出而非自动换行。
        let buf = if max_w > 0.0 {
            Some(self.wrap_buffer(text, style.font_size, style.font_family.as_deref(), max_w))
        } else {
            None
        };
        self.painter.q.queue.push(text_cmd(
            self.painter.q.depth,
            seq,
            self.painter.q.cur_win,
            elem,
            rect,
            Arc::from(text),
            style.font_size,
            style.color,
            TextAlign::from(style.align),
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.painter.q.clip,
            buf,
        ));
        size
    }

}

impl From<Align> for TextAlign {
    fn from(a: Align) -> Self {
        match a {
            Align::Left => TextAlign::Left,
            Align::Right => TextAlign::Right,
            _ => TextAlign::Center,
        }
    }
}

impl From<TextAlign> for Align {
    fn from(a: TextAlign) -> Self {
        match a {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        }
    }
}

