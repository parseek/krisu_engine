//! `Ui` 主体：控件录制 + 深度排序 + 提交绘制。
//!
//! 用法（见 crate 文档与示例）：
//! ```no_run
//! # let cam = todo!(); let mouse = todo!(); let keyboard = todo!();
//! # let text = todo!(); let r2d = todo!(); let state = todo!(); let window = todo!();
//! use rjw_ui::{Theme, Ui};
//! let mut ui = Ui::begin(&window, &cam, &mouse, &keyboard, &mut text, &mut r2d, &mut state)
//!     .theme(Theme::dark())
//!     .base_layer(1e7)
//!     .build();
//! ui.label_at(glam::Vec2::new(20.0, 20.0), "Hello UI");
//! ui.finish();
//! ```
//!
//! 坐标语义：所有位置为**屏幕逻辑像素**（左上角原点，Y+ 向下）；容器内 `*_at` 的 `pos`
//! 相对**当前容器内容原点**（顶层即屏幕原点）；交互命中在逻辑坐标进行（内部经 DPI 换算）。

use std::ops::RangeInclusive;
use std::sync::Arc;

use glam::Vec2;
use rjw_2d_render::{Layer, Render2D, VertexP3U2C4};
use rjw_color::Color;
use rjw_keyboard::{KeyCode, KeyboardInput};
use rjw_keystate::KeyState;
use rjw_mouse::{MouseButton, MouseInput};
use rjw_render::TEXTURES;
use rjw_text::{Align, Attrs, Buffer, CachePolicy, Family, Text};
use rjw_transform::{Camera2D, Rect};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::window::Window as WinitWindow;

use crate::draw::{
    border_rects, clipped, debug_shape_segments, intersect_rect, screen_fixed_tf, snap_rect,
    text_block_offset, text_cmd, DebugShape, DrawKind, GradientAxis, TextAlign, TextVAlign, UiDraw,
};
use crate::edit::{
    byte_to_char, char_to_byte, delete_range, insert_str_at, scroll_follow_caret, sel_range,
    selected_text,
};
use crate::focus::{focus_step, FocusEntry, FocusKind};
use crate::hit::{
    clear_frame_flags, hit_test, normalize_x, update_drag, update_interact, window_occluded,
};
use crate::layout::{Frame, PackSide};
use crate::state::{ButtonState, CheckboxState, TEXT_BUFFER_CACHE_CAP, UiState, WidgetState};
use crate::style::Theme;

// ─── 文本编辑辅助（纯函数，可单测） ─────────────────────────────

/// TextArea **行距倍率**：行高 = 字号 × 该值（1.2 = 略宽松，多行可读性；与
/// `ensure_text_buf` 的 `line_mult` 一致，cosmic 行盒按此递增）。
pub(crate) const TEXT_AREA_LINE_SPACING: f32 = 1.2;

/// 读取系统剪贴板文本（失败返回 `None`：无剪贴板 / 权限受限等）。
fn clipboard_get() -> Option<String> {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut c| c.get_text().ok())
}

/// 写入系统剪贴板文本（失败静默——非致命）。
fn clipboard_set(text: &str) {
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(text.to_owned());
    }
}

/// 在 char 索引 `caret` 处插入字符。
pub(crate) fn insert_char_at(s: &mut String, caret: usize, c: char) {
    let mut chars: Vec<char> = s.chars().collect();
    let idx = caret.min(chars.len());
    chars.insert(idx, c);
    *s = chars.into_iter().collect();
}

/// 删除 `caret` 前一个字符，返回新 caret。
pub(crate) fn remove_before(s: &mut String, caret: usize) -> usize {
    let mut chars: Vec<char> = s.chars().collect();
    if caret > 0 && caret <= chars.len() {
        chars.remove(caret - 1);
    }
    let n = chars.len();
    *s = chars.into_iter().collect();
    caret.saturating_sub(1).min(n)
}

/// 删除 `caret` 处字符。
pub(crate) fn remove_at(s: &mut String, caret: usize) {
    let mut chars: Vec<char> = s.chars().collect();
    if caret < chars.len() {
        chars.remove(caret);
    }
    *s = chars.into_iter().collect();
}

// ─── 文本排版版本号 ─────────────────────────────────────────────

/// 文本缓冲区行高版本号。当行高计算方式变更时递增，使旧缓存失效。
/// 版本 1：行高 = 字号（原为 1.2 倍字号，导致英文字母在文本框内偏上）。
const TEXT_LINE_HEIGHT_VERSION: u8 = 1;

// ─── Ui ─────────────────────────────────────────────────────────

/// `Ui::begin` 返回的构建器：设置主题 / 基层层级 / scale_factor / 调试开关后 `build()`。
pub struct UiInit<'a> {
    window: &'a WinitWindow,
    cam: &'a Camera2D,
    mouse: &'a MouseInput,
    keyboard: &'a KeyboardInput,
    text: &'a mut Text,
    r2d: &'a mut Render2D,
    state: &'a mut UiState,
    theme: Theme,
    base_layer: f64,
    scale: f32,
    debug_layout: bool,
}

impl<'a> UiInit<'a> {
    /// 主题（默认浅色）。
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// 基层层级（默认 `1e7`，与 RPG 示例 UI 层一致）。
    pub fn base_layer(mut self, layer: f64) -> Self {
        self.base_layer = layer;
        self
    }

    /// DPI scale factor（物理像素 / 逻辑像素，如 1.0 / 1.5 / 2.0；默认 1.0）。
    ///
    /// 传入后，**全部控件坐标 / 字号按逻辑像素**使用，内部自动换算物理像素
    /// 绘制与命中；窗口高 DPI 下文字与控件同步缩放。
    /// 取值：`ctx.scale_factor().unwrap_or(1.0)`（`rjw_main::MainContext`）。
    pub fn scale_factor(mut self, scale: f64) -> Self {
        self.scale = scale.max(f64::EPSILON) as f32;
        self
    }

    /// **调试 UI 布局**（默认 **false**）：开启后 `finish` 为**每一个录制命令的矩形**
    /// 画一圈青色描边（覆盖在 UI 内容之上）——可视化每个控件/容器的布局矩形与
    /// 命中区域，用于调试 `rjw_ui` 自身的布局。可在帧内用 [`Ui::debug_layout`] 开关。
    pub fn debug_layout(mut self, on: bool) -> Self {
        self.debug_layout = on;
        self
    }

    pub fn build(self) -> Ui<'a> {
        let UiInit {
            window,
            cam,
            mouse,
            keyboard,
            text,
            r2d,
            state,
            theme,
            base_layer,
            scale,
            debug_layout,
        } = self;
        state.begin_frame();
        let (mx, my) = mouse.get_mouse_position();
        let mouse_screen = Vec2::new(mx as f32, my as f32);
        Ui {
            window,
            cam,
            mouse,
            keyboard,
            text,
            r2d,
            state,
            theme,
            base_layer,
            scale,
            debug_layout,
            frames: Vec::new(),
            queue: Vec::new(),
            debug_queue: Vec::new(),
            clip: None,
            abs_base: Vec2::ZERO,
            depth: 0,
            seq: 0,
            cur_win: 0,
            // 鼠标屏幕坐标：物理（拖拽/IME 基准用）与逻辑（命中测试用）各存一份
            mouse_screen,
            mouse_logical: mouse_screen / scale,
            mouse_in_window: mouse.in_window(),
            any_pressed: false,
            press_claimed: false,
            drag_panel: None,
            win_press_top: None,
            win_origins: std::collections::HashMap::new(),
            win_ids: std::collections::HashMap::new(),
            focusables: Vec::new(),
        }
    }
}

/// UI 录制器（借用窗口 / 相机 / 输入 / 文本 / 渲染器 / 状态，一帧一用）。
pub struct Ui<'a> {
    window: &'a WinitWindow,
    cam: &'a Camera2D,
    mouse: &'a MouseInput,
    keyboard: &'a KeyboardInput,
    text: &'a mut Text,
    r2d: &'a mut Render2D,
    state: &'a mut UiState,
    theme: Theme,
    base_layer: f64,
    /// DPI scale factor（物理 / 逻辑）。
    scale: f32,
    /// 容器帧栈（当前容器在栈顶）。
    frames: Vec<Frame>,
    /// 录制命令（坐标 = 相对当前容器 origin 的局部坐标，**逻辑像素**）。
    queue: Vec<UiDraw>,
    /// **调试命令队列**（[`Self::debug_line`] 等；坐标 = **绝对逻辑屏幕像素**）。
    /// 不进窗口缓存、不参与内容排序——`finish` 时在 UI 内容**之后**提交（恒覆盖在最上）。
    debug_queue: Vec<UiDraw>,
    /// **调试 UI 布局开关**（[`UiInit::debug_layout`] / [`Self::debug_layout`]）：
    /// 开启后每个录制命令的矩形都会画青色描边（布局 / 命中区域可视化）。
    debug_layout: bool,
    /// **当前裁剪区**（**绝对逻辑屏幕坐标**；滚动容器 [`Self::scroll_at`] 等设置）。
    /// 录制命令时存入 `UiDraw.clip`，收集期与内容求交（越界剔除）。
    clip: Option<Rect>,
    /// 当前容器绝对原点（命中测试用，逻辑像素）。
    abs_base: Vec2,
    /// 当前录制深度（容器嵌套层数）。
    depth: u32,
    /// 全局递增序号（同深度内排序）。
    seq: u32,
    /// 当前窗口 z 序（[`Self::window_at`]；非窗口内容 = 0）。
    cur_win: u32,
    /// 鼠标屏幕坐标（逻辑像素 = 物理 ÷ scale，命中测试用）。
    mouse_logical: Vec2,
    /// 鼠标屏幕坐标（**物理像素**，面板拖拽 / IME 基准用）。
    mouse_screen: Vec2,
    mouse_in_window: bool,
    /// 本帧是否有控件被按下（空白点击清焦点用）。
    any_pressed: bool,
    /// **本帧按下是否被文本输入控件占用**（选择拖拽优先于窗口/面板拖拽）：
    /// 输入框/TextArea 在按下响应时置位，`window_at` / `panel_impl` 据此**不建立**
    /// 拖拽基准——从输入框上拖拽 = 选择文本，而不是拖动窗口。
    press_claimed: bool,
    /// 当前拖拽中的面板 / 窗口 ID（拖动期间抑制子控件交互）。
    drag_panel: Option<String>,
    /// 本帧按下命中的**最上层窗口**（重叠点击裁决：只让最高 z 窗口拖拽与置顶）。
    win_press_top: Option<(String, u32)>,
    /// 窗口 z → 窗口左上角（**逻辑**坐标）：QuadVertices 顶点相对此原点存储，
    /// 提交时用 `screen_fixed_tf(原点物理)` 变换到世界（移动窗口只改变换，顶点不变）。
    win_origins: std::collections::HashMap<u32, Vec2>,
    /// 窗口 z → 窗口 id（四边形缓存 key 用；window_at 记录）。
    win_ids: std::collections::HashMap<u32, String>,
    /// **本帧焦点链**（键盘导航）：交互控件录制时注册（[`Self::register_focus`]），
    /// `finish` 按 (win, 注册序) 排序后处理 Tab / 方向键遍历并绘制焦点描边。
    focusables: Vec<FocusEntry>,
}

impl<'a> Ui<'a> {
    /// 一帧一次。`window` 用于 IME 候选框定位（[`winit::window::Window::set_ime_cursor_area`]）。
    pub fn begin(
        window: &'a WinitWindow,
        cam: &'a Camera2D,
        mouse: &'a MouseInput,
        keyboard: &'a KeyboardInput,
        text: &'a mut Text,
        r2d: &'a mut Render2D,
        state: &'a mut UiState,
    ) -> UiInit<'a> {
        UiInit {
            window,
            cam,
            mouse,
            keyboard,
            text,
            r2d,
            state,
            theme: Theme::default(),
            base_layer: 1e7,
            scale: 1.0,
            debug_layout: false,
        }
    }

    // ── 内部工具 ─────────────────────────────────────────────

    #[inline]
    fn next_seq(&mut self) -> u32 {
        self.seq += 1;
        self.seq
    }

    #[inline]
    fn phys_rect(&self, r: &Rect) -> Rect {
        Rect::new(r.x * self.scale, r.y * self.scale, r.w * self.scale, r.h * self.scale)
    }

    #[inline]
    fn phys_f(&self, f: f32) -> f32 {
        f * self.scale
    }

    // ── 键盘导航（焦点链） ────────────────────────────────────

    /// 把控件登记进本帧焦点链（键盘导航用）。`rect` 为**相对当前容器**的局部矩形，
    /// 内部转成**绝对逻辑坐标**（焦点描边绘制 / 排序用）。
    fn register_focus(&mut self, id: &str, rect: Rect, kind: FocusKind) {
        let abs = Rect::new(
            self.abs_base.x + rect.x,
            self.abs_base.y + rect.y,
            rect.w,
            rect.h,
        );
        self.focusables.push(FocusEntry {
            id: id.to_owned(),
            win: self.cur_win,
            kind,
            depth: self.depth,
            rect: abs,
            clip: self.clip,
        });
    }

    /// 本控件是否持有键盘焦点（`UiState.focused == id`）。
    #[inline]
    fn focused_is(&self, id: &str) -> bool {
        self.state.focused.as_deref() == Some(id)
    }

    /// **键盘激活**：Enter / Space 在本帧按下、本控件持有焦点且不在 IME 组合中
    /// → 视为一次点击（按钮 / 勾选 / 单选 / 下拉框用）。文本输入框与滑块不参与
    /// （前者走打字路径，后者用方向键调值）。
    fn key_click(&self, id: &str, kind: FocusKind) -> bool {
        if !self.focused_is(id) || kind == FocusKind::TextInput || kind == FocusKind::Slider {
            return false;
        }
        let composing = self
            .keyboard
            .get_ime_preedit()
            .is_some_and(|p| !p.is_empty());
        if composing {
            return false;
        }
        self.keyboard.get(KeyCode::Enter).down_edge()
            || self.keyboard.get(KeyCode::Space).down_edge()
    }

    // ── Debug UI / DebugDraw（调试 rjw_ui 自身 + 屏幕空间调试图元） ──

    /// 调试 UI 布局开关（运行时切换；等价于 [`UiInit::debug_layout`]）。
    ///
    /// 开启后 `finish` 为**每一个录制命令的矩形**画青色描边（覆盖在 UI 内容之上）——
    /// 可视化每个控件 / 容器的布局矩形与命中区域。
    #[inline]
    pub fn debug_layout(&mut self, on: bool) -> &mut Self {
        self.debug_layout = on;
        self
    }

    /// 屏幕空间线段（**绝对逻辑屏幕像素**，Y+ 向下；覆盖在 UI 内容之上）。
    pub fn debug_line(&mut self, a: Vec2, b: Vec2, width: f32, color: Color) {
        self.push_debug(DebugShape::Line { a, b, width }, color);
    }

    /// 屏幕空间矩形边框（逻辑像素）。
    pub fn debug_rect_outline(&mut self, rect: Rect, width: f32, color: Color) {
        self.push_debug(DebugShape::RectOutline { rect, width }, color);
    }

    /// 屏幕空间圆环（`segments` 段折线近似；逻辑像素）。
    pub fn debug_circle_outline(
        &mut self,
        center: Vec2,
        radius: f32,
        segments: usize,
        width: f32,
        color: Color,
    ) {
        self.push_debug(DebugShape::CircleOutline { center, radius, segments, width }, color);
    }

    /// 屏幕空间十字标记（逻辑像素）。
    pub fn debug_cross(&mut self, center: Vec2, half: f32, width: f32, color: Color) {
        self.push_debug(DebugShape::Cross { center, half, width }, color);
    }

    /// 屏幕空间网格（`rect` 范围内按 `spacing` 竖线 + 横线；每方向最多 512 条）。
    pub fn debug_grid(&mut self, rect: Rect, spacing: f32, width: f32, color: Color) {
        self.push_debug(DebugShape::Grid { rect, spacing, width }, color);
    }

    /// 录制一条屏幕空间调试图元（进 `debug_queue`，坐标 = 绝对逻辑像素）。
    fn push_debug(&mut self, shape: DebugShape, color: Color) {
        let seq = self.next_seq();
        let depth = self.depth;
        let win = self.cur_win;
        self.debug_queue.push(UiDraw {
            depth,
            seq,
            win,
            elem: 0,
            // 调试形状自带几何（DebugShape），rect 字段未用。
            rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            clip: None,
            kind: DrawKind::Debug { color, shape },
        });
    }

    /// **圆角矩形**（背景填充原语；绝对定位，`radius` 逻辑像素，9-patch 绘制——
    /// 程序化纹理进动态 Atlas，颜色顶点色 tint）。
    pub fn rounded_rect_at(&mut self, pos: Vec2, size: Vec2, radius: f32, color: Color) {
        self.push_draw(
            DrawKind::RoundedRect { color, radius },
            Rect::new(pos.x, pos.y, size.x, size.y),
        );
    }

    /// **线性渐变矩形**（背景填充原语；绝对定位，`stops` 沿 `axis`——
    /// 程序化渐变纹理进动态 Atlas，按主轴拉伸采样）。
    pub fn gradient_rect_at(
        &mut self,
        pos: Vec2,
        size: Vec2,
        axis: GradientAxis,
        stops: Vec<(f32, Color)>,
    ) {
        self.push_draw(
            DrawKind::Gradient { axis, stops },
            Rect::new(pos.x, pos.y, size.x, size.y),
        );
    }

    /// 录制一条绘制命令（`elem = 0` 装饰层，画在本窗口元素之下——如背景/边框）。
    fn push_draw(&mut self, kind: DrawKind, rect: Rect) {
        let seq = self.next_seq();
        let depth = self.depth;
        let win = self.cur_win;
        self.queue.push(UiDraw { depth, seq, win, elem: 0, rect, clip: self.clip, kind });
    }

    /// 按样式 push **背景 + 边框**（`radius > 0` 走双层圆角矩形：外圈 border 色、
    /// 内圈 bg 色内缩 `border_w`，近似圆角边框；否则原 Solid + Border 路径）。
    /// `elem`：元素序（装饰背景传 0；控件背景传 `self.seq + 1`）。
    #[allow(clippy::too_many_arguments)]
    fn push_panel_like(
        &mut self,
        rect: Rect,
        bg: Color,
        border: Color,
        border_w: f32,
        radius: f32,
        elem: u32,
    ) {
        let seq = self.next_seq();
        let depth = self.depth;
        let win = self.cur_win;
        let clip = self.clip;
        if radius > 0.0 {
            // 圆角边框 ≈ 外圈 border 色圆角 + 内圈 bg 色圆角（内缩 border_w）。
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect,
                clip,
                kind: DrawKind::RoundedRect { color: border, radius: radius + border_w },
            });
            let bw = border_w.min(rect.w * 0.5).min(rect.h * 0.5);
            let inner = Rect::new(
                rect.x + bw,
                rect.y + bw,
                (rect.w - bw * 2.0).max(0.0),
                (rect.h - bw * 2.0).max(0.0),
            );
            if inner.w > 0.0 && inner.h > 0.0 {
                self.queue.push(UiDraw {
                    depth,
                    seq: seq + 1,
                    win,
                    elem,
                    rect: inner,
                    clip,
                    kind: DrawKind::RoundedRect { color: bg, radius },
                });
            }
        } else {
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect,
                clip,
                kind: DrawKind::Solid(bg),
            });
            if border_w > 0.0 {
                self.queue.push(UiDraw {
                    depth,
                    seq: seq + 1,
                    win,
                    elem,
                    rect,
                    clip,
                    kind: DrawKind::Border { color: border, width: border_w },
                });
            }
        }
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
    fn text_size(&mut self, s: &str, size: f32, family: Option<&str>) -> Vec2 {
        let buf = self.cache_buffer(s, size, family);
        (Text::measure_buffer(&buf) / self.scale).ceil()
    }

    /// 按**换行宽度**测量文本自然尺寸：`wrap_logical > 0` 时文本在宽度内自动换行
    /// （宽 = min(自然宽, wrap)，高 = 行数 × 行高）；否则同 [`Self::text_size`]。
    fn text_size_wrap(&mut self, s: &str, size: f32, family: Option<&str>, wrap_logical: f32) -> Vec2 {
        let buf = self.cache_buffer_wrap(s, size, family, wrap_logical);
        (Text::measure_buffer(&buf) / self.scale).ceil()
    }

    /// **控件自持排版缓冲**：`WidgetState::text_buf` 命中复用（key 含文本/字号/字体/
    /// 换行宽/**行距**/版本），未命中直接构建——**不写** `UiState::text_buffers` 全局缓存
    /// （文本频繁变化的输入框不污染静态标签缓存）。
    ///
    /// `line_mult`：行高 = 字号 × 行距倍率（1.0 = 无行距；TextArea 多行用 1.2 加行距）。
    /// 用两次独立借用实现（`self.state` 与 `self.text` 不能同时可变借用）。
    fn ensure_text_buf(
        &mut self,
        id: &str,
        s: &str,
        size: f32,
        family: Option<&str>,
        wrap_logical: f32,
        line_mult: f32,
    ) -> Arc<Buffer> {
        let size_px = (size * self.scale).round();
        let wrap_px = (wrap_logical * self.scale).round().max(0.0);
        let mult_bits = line_mult.to_bits();
        let key = format!(
            "{s}\u{1}{size_px}\u{1}{}\u{1}{wrap_px}\u{1}{mult_bits}\u{1}{TEXT_LINE_HEIGHT_VERSION}",
            family.unwrap_or("")
        );
        if let Some((k, b)) = self.state.widgets.get(id).and_then(|w| w.text_buf.as_ref()) {
            if *k == key {
                return b.clone();
            }
        }
        let lh = (size_px as f32 * line_mult.max(1.0)).round();
        let attrs = match family {
            Some(f) if !f.is_empty() => Attrs::new().family(Family::Name(f)),
            _ => Attrs::new(),
        };
        let buf = self
            .text
            .create_buffer_wrap(s, attrs, size_px, lh, Align::Left, wrap_px, CachePolicy::User);
        if let Some(ws) = self.state.widgets.get_mut(id) {
            ws.text_buf = Some((key, buf.clone()));
        }
        buf
    }

    /// 按字符**实际宽度**把点击位置（相对内容左缘，逻辑像素）映射为最近的光标 char 索引。
    ///
    /// 用"前缀宽度"二分（宽度随前缀长度单调不减）——混合中英文（字宽不同）时
    /// 比等比估算（`total_w × k / n`）精确；纯中文（等宽）两者一致。
    fn caret_index_at_width(
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

    /// 取（或创建）共享排版缓冲（物理字号取整到像素；`CachePolicy::User`：不进 rjw_text LRU）。
    /// 
    /// 行高 = 字号（`size_px`），保证文本框内字形垂直居中位置正确。
    /// 缓存键包含 `TEXT_LINE_HEIGHT_VERSION`，修改行高策略后旧缓存自动失效。
    fn cache_buffer(&mut self, s: &str, size: f32, family: Option<&str>) -> Arc<Buffer> {
        self.cache_buffer_wrap(s, size, family, 0.0)
    }

    /// 取（或创建）共享排版缓冲（物理字号取整到像素；`CachePolicy::User`：不进 rjw_text LRU）。
    ///
    /// `wrap_logical <= 0` = 不换行（默认宽裕宽度）；`> 0` = 按该**逻辑像素**宽度换行
    /// （物理像素取整后传给 rjw_text；换行宽度参与缓存键，不同宽度各自缓存）。
    ///
    /// 行高 = 字号（`size_px`），保证文本框内字形垂直居中位置正确。
    /// 缓存键包含 `TEXT_LINE_HEIGHT_VERSION`，修改行高策略后旧缓存自动失效。
    fn cache_buffer_wrap(
        &mut self,
        s: &str,
        size: f32,
        family: Option<&str>,
        wrap_logical: f32,
    ) -> Arc<Buffer> {
        // 物理字号取整：字形在像素网格上，避免亚像素渲染模糊；测量/绘制/缓存键一致
        let size_px = (size * self.scale).round();
        let wrap_px = (wrap_logical * self.scale).round().max(0.0);
        let key = (
            s.to_owned(),
            size_px.to_bits(),
            family.map(|f| f.to_owned()),
            wrap_px.to_bits(),
            TEXT_LINE_HEIGHT_VERSION,
        );
        if let Some(b) = self.state.text_buffers.get(&key) {
            return b.clone();
        }
        // 行高 = 字号（精确 1:1），让字形在行盒中垂直居中更准确
        let lh = size_px;
        let attrs = match family {
            Some(f) if !f.is_empty() => Attrs::new().family(Family::Name(f)),
            _ => Attrs::new(),
        };
        let buf = self.text.create_buffer_wrap(
            s,
            attrs,
            size_px,
            lh,
            Align::Left,
            wrap_px,
            CachePolicy::User,
        );
        if self.state.text_buffers.len() >= TEXT_BUFFER_CACHE_CAP {
            self.state.text_buffers.clear();
        }
        self.state.text_buffers.insert(key, buf.clone());
        buf
    }

    #[inline]
    fn mouse_left(&self) -> KeyState {
        self.mouse.get(MouseButton::Left)
    }

    /// 鼠标绝对坐标 → 当前容器局部坐标（逻辑像素，字段运算避免方法借用）。
    #[inline]
    fn mouse_local_x(&self) -> f32 {
        self.mouse_logical.x - self.abs_base.x
    }

    /// 局部矩形（逻辑）→ 命中测试（与逻辑鼠标坐标比较；含窗口外判定与窗口遮挡）。
    ///
    /// 遮挡时若控件矩形本身命中鼠标，累加 [`UiState::occluded_hits`]（诊断机制：
    /// 告诉你"本帧有多少次点击被窗口遮挡拦截"——见示例 `eg260818UI` 的诊断面板）。
    #[inline]
    fn hit_abs(&mut self, local: &Rect) -> bool {
        if !self.mouse_in_window {
            return false;
        }
        // 面板/窗口**真正拖拽中**（按下后位移 ≥ DRAG_ACTIVATE_PX）抑制子控件交互
        // （防止拖动中误触按钮等）；纯点击不进入拖拽，子控件正常响应。
        if self.drag_panel.is_some() {
            return false;
        }
        let abs = Rect::new(
            self.abs_base.x + local.x,
            self.abs_base.y + local.y,
            local.w,
            local.h,
        );
        if !hit_test(&abs, self.mouse_logical) {
            return false;
        }
        // **窗口遮挡**（点击穿透修复）：鼠标下若有更高 z 的窗口（`win=0` 内容被任意
        // 窗口）覆盖本控件所在窗口 → 本窗口不得响应——重叠区域只让最上层窗口交互，
        // 背后窗口的控件不会误触发。窗口矩形来自 [`UiState::window_rects`]（跨帧缓存）。
        if window_occluded(self.cur_win, self.mouse_logical, self.window_rects_iter()) {
            // 命中但被遮挡 → 记录诊断计数（未响应）。
            self.state.occluded_hits += 1;
            return false;
        }
        true
    }

    /// 窗口遮挡判定用的窗口矩形迭代器（`(z, rect)`；逻辑像素）。
    #[inline]
    fn window_rects_iter(&self) -> impl Iterator<Item = (u32, Rect)> + '_ {
        self.state.window_rects.iter().map(|(&z, &r)| (z, r))
    }

    /// **诊断**：当前窗口 z-order（按 z 升序）：`(id, z)`。
    pub fn window_order(&self) -> Vec<(String, u32)> {
        let mut v: Vec<(String, u32)> = self
            .state
            .window_z
            .iter()
            .map(|(id, &z)| (id.clone(), z))
            .collect();
        v.sort_by_key(|&(_, z)| z);
        v
    }

    /// **诊断**：鼠标下**最上层**的窗口（`id, z`）——重叠点击时唯一可交互的窗口；
    /// 鼠标不在任何窗口上时返回 `None`。窗口矩形来自跨帧缓存（含本帧已录制的窗口）。
    pub fn window_under_mouse(&self) -> Option<(String, u32)> {
        let mut best: Option<(String, u32)> = None;
        for (id, &z) in &self.state.window_z {
            if let Some(r) = self.state.window_rects.get(&z) {
                if r.contains_point(self.mouse_logical) {
                    match &best {
                        Some((_, bz)) if *bz >= z => {}
                        _ => best = Some((id.clone(), z)),
                    }
                }
            }
        }
        best
    }

    /// 当前容器为子项分配局部矩形（顶层无容器时 panic，顶层请用 `*_at`）。
    fn child_rect(&mut self, w: f32, h: f32) -> Rect {
        self.frames
            .last_mut()
            .expect("顶层控件请用 *_at(pos, ...) 定位（容器内才可用无 pos 形式）")
            .child_rect(w, h)
    }

    /// 通用容器：push 帧 → 闭包 → 结算（返回尺寸与最大子尺寸）→ 平移子命令 → pop。
    fn container<F>(&mut self, pos: Vec2, frame: Frame, f: F) -> (Vec2, Vec2)
    where
        F: FnOnce(&mut ContainerCtx<'_, '_>),
    {
        let start = self.queue.len();
        let saved_base = self.abs_base;
        self.abs_base = saved_base + pos;
        self.frames.push(frame);
        self.depth += 1;
        f(&mut ContainerCtx { ui: self });
        let frame = self.frames.pop().expect("container frame");
        let size = frame.settle_size();
        let max_child = frame.max_child;
        self.depth -= 1;
        self.abs_base = saved_base;
        for d in &mut self.queue[start..] {
            d.translate(pos);
        }
        (size, max_child)
    }

    /// **滚动容器**：内容在 `view_size` 可视区内垂直堆叠（pack Top），超出部分
    /// 滚动查看——**滚轮**滚动 + 右侧**滚动条**（拖 thumb / 点轨道翻页）。
    ///
    /// - `id`：滚动偏移状态键（[`UiState::scrolls`]，跨帧持久）；
    /// - 内容子项照常录制（`s.label` / `s.button` 等，占光标堆叠）；
    /// - 可视区之外的图形/文字**裁剪**（`UiDraw.clip` 绝对逻辑矩形，收集期求交）；
    /// - 返回 `view_size`（内容尺寸超出时可经 [`UiState::scrolls`] 读取）。
    pub fn scroll_at(
        &mut self,
        pos: Vec2,
        view_size: Vec2,
        id: &str,
        f: impl FnOnce(&mut Scroll<'_, '_>),
    ) -> Vec2 {
        let saved_clip = self.clip;
        let view_abs = Rect::new(pos.x, pos.y, view_size.x.max(0.0), view_size.y.max(0.0));
        // 裁剪区 = 外层裁剪 ∩ 本可视区（绝对逻辑）。
        self.clip = match saved_clip {
            Some(c) => intersect_rect(&c, &view_abs),
            None => Some(view_abs),
        };
        // 滚动偏移（跨帧状态；先 Copy 读出，`f` 结束再写回——避免借用冲突）。
        let mut offset = self
            .state
            .scrolls
            .get(id)
            .map(|s| s.offset)
            .unwrap_or(0.0);
        // 内容 pack 堆叠（手动管理帧栈：平移 = pos - offset，而非 container 的 pos）。
        let start = self.queue.len();
        let saved_base = self.abs_base;
        self.abs_base = saved_base + pos;
        self.frames.push(Frame::new_stack(PackSide::Top, self.theme.gap, 0.0));
        self.depth += 1;
        f(&mut Scroll { ui: self });
        let frame = self.frames.pop().expect("scroll frame");
        let content_size = frame.settle_size();
        self.depth -= 1;
        self.abs_base = saved_base;
        let max_off = (content_size.y - view_size.y).max(0.0);
        offset = offset.clamp(0.0, max_off);
        // 滚轮（鼠标在可视区内且未被窗口遮挡；wheel y 向上为正 → offset 减小）。
        let hit = hit_test(&view_abs, self.mouse_logical)
            && self.mouse_in_window
            && !window_occluded(self.cur_win, self.mouse_logical, self.window_rects_iter());
        if hit {
            let (_, wy) = self.mouse.get_mouse_wheel_delta();
            if wy != 0.0 {
                offset = (offset - wy as f32 * 40.0).clamp(0.0, max_off);
            }
        }
        // 滚动条（内容超出可视区时显示；拖 thumb / 点轨道翻页）。
        if content_size.y > view_size.y + 1.0 && view_size.y > 0.0 {
            offset = self.scrollbar(id, &view_abs, view_size.y, content_size.y, offset, max_off, saved_clip);
        }
        // 写回滚动状态（`f` 借用已结束）。
        let st = self.state.scrolls.entry(id.to_owned()).or_default();
        st.offset = offset;
        st.content_h = content_size.y;
        // 平移子命令：内容上移 offset（clip 随 translate 同步平移）。
        for d in &mut self.queue[start..] {
            d.translate(pos - Vec2::new(0.0, offset));
        }
        self.clip = saved_clip;
        view_size
    }

    /// **选择列表**：`scroll_at` + 逐项回调（选中态由调用方维护）。
    ///
    /// `item` 回调 `(容器, 索引, 是否选中) -> bool`：返回 `true` 表示该项被点击。
    /// 返回本帧被点击的索引（`None` = 无）。
    pub fn list_at<F>(
        &mut self,
        pos: Vec2,
        view_size: Vec2,
        id: &str,
        count: usize,
        selected: Option<u32>,
        mut item: F,
    ) -> Option<u32>
    where
        F: FnMut(&mut Scroll<'_, '_>, u32, bool) -> bool,
    {
        let mut clicked = None;
        self.scroll_at(pos, view_size, id, |s| {
            for i in 0..count as u32 {
                if item(s, i, selected == Some(i)) && clicked.is_none() {
                    clicked = Some(i);
                }
            }
        });
        clicked
    }

    /// 滚动条：右侧竖条（轨道 + thumb）。返回更新后的滚动偏移。
    #[allow(clippy::too_many_arguments)]
    fn scrollbar(
        &mut self,
        id: &str,
        view: &Rect,
        view_h: f32,
        content_h: f32,
        offset: f32,
        max_off: f32,
        outer_clip: Option<Rect>,
    ) -> f32 {
        const SB_W: f32 = 8.0;
        let mut offset = offset;
        let track = Rect::new(view.x + view.w - SB_W, view.y, SB_W, view_h);
        let ratio = (view_h / content_h).clamp(0.0, 1.0);
        let thumb_h = (view_h * ratio).max(16.0);
        let thumb_y = view.y + if max_off > 1e-6 {
            offset / max_off * (view_h - thumb_h)
        } else {
            0.0
        };
        let thumb = Rect::new(track.x, thumb_y, track.w, thumb_h);
        // 绘制：轨道 + thumb（白纹理图形，elem=0 装饰层）。
        let depth = self.depth;
        let win = self.cur_win;
        let seq = self.next_seq();
        self.queue.push(UiDraw {
            depth,
            seq,
            win,
            elem: 0,
            rect: track,
            clip: outer_clip,
            kind: DrawKind::Solid(self.theme.slider.track),
        });
        self.queue.push(UiDraw {
            depth,
            seq: seq + 1,
            win,
            elem: 0,
            rect: thumb,
            clip: outer_clip,
            kind: DrawKind::Solid(self.theme.slider.handle),
        });
        // 交互：thumb 拖拽（复用 WidgetState.press_panel/press_mouse 基准）。
        let bar_id = format!("{id}::bar");
        let bar_hit = hit_test(&thumb, self.mouse_logical)
            && self.mouse_in_window
            && !window_occluded(win, self.mouse_logical, self.window_rects_iter());
        let btn = self.mouse_left();
        let grab = {
            let ws = self.state.widgets.entry(bar_id.clone()).or_default();
            let dragging = update_drag(ws, bar_hit, btn);
            if btn.down_edge() && bar_hit {
                ws.press_mouse = Some(self.mouse_screen.round());
                ws.press_panel = Some(Vec2::new(thumb_y, offset));
            }
            (dragging, ws.press_panel.unwrap_or(Vec2::ZERO))
        };
        if grab.0 {
            let pm = self
                .state
                .widgets
                .get(&bar_id)
                .and_then(|w| w.press_mouse)
                .unwrap_or(self.mouse_screen);
            let dy = (self.mouse_screen.y - pm.y).round() / self.scale;
            offset = (grab.1.y + dy / (view_h - thumb_h).max(1.0) * max_off).clamp(0.0, max_off);
        }
        // 轨道点击（thumb 外）→ 翻页。
        let hit_track = hit_test(&track, self.mouse_logical)
            && self.mouse_in_window
            && !window_occluded(win, self.mouse_logical, self.window_rects_iter());
        if btn.down_edge() && hit_track && !bar_hit {
            if self.mouse_logical.y < thumb_y {
                offset = (offset - view_h).max(0.0);
            } else if self.mouse_logical.y > thumb_y + thumb_h {
                offset = (offset + view_h).min(max_off);
            }
        }
        offset
    }

    // ── 顶层入口（*_at：位置显式，尺寸自动） ─────────────────

    /// 绝对定位标签（`pos` 相对当前容器内容原点；顶层即屏幕原点）。
    pub fn label_at(&mut self, pos: Vec2, text: &str) -> Vec2 {
        let elem = self.seq + 1;
        let seq = self.next_seq();
        let style = self.theme.label.clone();
        let size = self.text_size(text, style.font_size, style.font_family.as_deref());
        let rect = Rect::new(pos.x, pos.y, size.x, size.y);
        self.queue.push(text_cmd(
            self.depth,
            seq,
            self.cur_win,
            elem,
            rect,
            text.to_owned(),
            style.font_size,
            style.color,
            TextAlign::from(style.align),
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.clip,
        None,
        ));
        size
    }

    /// **自动换行标签**：`max_w`（逻辑像素）内按词/字换行，返回自然尺寸
    /// （宽 = min(自然宽, max_w)，高 = 行数 × 行高）。`max_w <= 0` = 不换行（同 [`Self::label_at`]）。
    ///
    /// 换行宽度参与排版缓存键（不同宽度各自缓存）；多行文本垂直居中于矩形。
    pub fn label_wrap_at(&mut self, pos: Vec2, max_w: f32, text: &str) -> Vec2 {
        let elem = self.seq + 1;
        let seq = self.next_seq();
        let style = self.theme.label.clone();
        let size = self.text_size_wrap(text, style.font_size, style.font_family.as_deref(), max_w);
        let rect = Rect::new(pos.x, pos.y, size.x, size.y);
        self.queue.push(text_cmd(
            self.depth,
            seq,
            self.cur_win,
            elem,
            rect,
            text.to_owned(),
            style.font_size,
            style.color,
            TextAlign::from(style.align),
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.clip,
        None,
        ));
        size
    }

    /// 面板：背景 + 边框 + 内容垂直堆叠（pack Top）；尺寸自动包裹内容。
    pub fn panel_at(&mut self, pos: Vec2, f: impl FnOnce(&mut Panel<'_, '_>)) -> Vec2 {
        self.panel_impl(pos, None, f)
    }

    /// **可拖拽**面板：同 [`Self::panel_at`]，且按住面板任意处**移动 ≥ 3 物理像素**
    /// 可拖动（纯点击不拖拽，面板内子控件正常响应）。
    ///
    /// - 位置持久化于 `UiState.panel_pos`（`id` 须稳定），跨帧跟随鼠标；
    /// - 真正拖动期间**抑制面板内子控件交互**（不会误触发按钮点击）；
    /// - `pos` 为初始位置（首次）；`UiState::reset()` 可复位。
    pub fn drag_panel_at(
        &mut self,
        id: &str,
        pos: Vec2,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        self.panel_impl(pos, Some(id), f)
    }

    /// 面板公共实现：`drag = Some(id)` 时启用拖拽。
    fn panel_impl(
        &mut self,
        pos: Vec2,
        drag: Option<&str>,
        f: impl FnOnce(&mut Panel<'_, '_>),
    ) -> Vec2 {
        // 拖拽面板的位置从持久状态读取（首次用传入 pos）
        let origin = match drag {
            Some(id) => self.state.panel_pos.get(id).copied().unwrap_or(pos),
            None => pos,
        };
        let start = self.queue.len();
        let (pad_total, gap) = {
            let p = &self.theme.panel;
            (p.padding + p.border_w, self.theme.gap)
        };
        let saved_base = self.abs_base;
        self.abs_base = saved_base + origin;
        self.frames.push(Frame::new_stack(PackSide::Top, gap, pad_total));
        self.depth += 1;
        let mut panel = Panel { ui: self };
        f(&mut panel);
        let frame = self.frames.pop().expect("panel frame");
        let size = frame.settle_size();
        self.depth -= 1;
        self.abs_base = saved_base;
        // 拖拽交互：**物理像素粒度**拖动基准（见下方说明）。
        // `display_pos`：本帧实际使用的面板位置（拖拽中 = 新位置，当帧生效，无帧延迟）。
        let display_pos = if let Some(id) = drag {
            let panel_rect = Rect::new(origin.x, origin.y, size.x, size.y);
            // 窗口遮挡：面板是 win=0 内容（绘制在所有窗口之下），被任意窗口覆盖时不可拖拽。
            let hit = hit_test(&panel_rect, self.mouse_logical)
                && self.mouse_in_window
                && !window_occluded(0, self.mouse_logical, self.window_rects_iter());
            let btn = self.mouse_left();
            let press_here = btn.down_edge() && hit;
            // 输入框按下（选择拖拽）不建立面板拖拽基准。
            let drag_here = press_here && !self.press_claimed;
            let (active, new_pos) = {
                let ws = self.state.widgets.entry(id.to_owned()).or_default();
                let dragging = update_drag(ws, hit, btn);
                if drag_here {
                    // 拖动基准：面板位置（逻辑）+ 鼠标物理坐标（**取整**）。
                    // 取整消除鼠标静止噪声（滞回）；拖拽中按**物理像素增量**移动：
                    // 粒度 1 物理 px，DPI 1.5 下也不会出现"移动 1.5px 才动"的粘滞感。
                    ws.press_panel = Some(panel_rect.min());
                    ws.press_mouse = Some(self.mouse_screen.round());
                } else if press_here {
                    // 文本框等子控件按下：清除旧拖拽基准（防窗口"瞬移"，见 window_at）。
                    ws.press_panel = None;
                    ws.press_mouse = None;
                }
                // 拖拽需实际位移（≥ DRAG_ACTIVATE_PX）且**有本帧基准**才激活：纯点击不拖拽，
                // 面板内子控件（按钮/勾选/输入框）正常响应（见 hit_abs 抑制条件）。
                let active = dragging
                    && ws.press_mouse.is_some()
                    && drag_moved(self.mouse_screen.round(), ws.press_mouse);
                let np = if active {
                    let pp = ws.press_panel.unwrap_or(origin);
                    let pm = ws.press_mouse.unwrap_or(self.mouse_screen);
                    // 物理像素增量（round：对噪声滞回，静止时不变）→ 逻辑位移
                    let d = (self.mouse_screen - pm).round();
                    pp + d / self.scale
                } else {
                    origin
                };
                (active, np)
            };
            if active {
                self.drag_panel = Some(id.to_owned());
                // 仅位置变化时写入（滞回：同一位置不重写）
                if self.state.panel_pos.get(id) != Some(&new_pos) {
                    self.state.panel_pos.insert(id.to_owned(), new_pos);
                }
            } else if self.drag_panel.as_deref() == Some(id) {
                self.drag_panel = None;
            }
            if press_here || active {
                // 按下面板（或拖拽中）都算"已响应按下"——避免空白点击清焦点
                self.any_pressed = true;
            }
            new_pos
        } else {
            origin
        };
        // 背景 + 边框（depth = 进入前深度，画在子控件之下；radius > 0 走圆角双层矩形）
        let style = self.theme.panel.clone();
        let bg_rect = Rect::new(0.0, 0.0, size.x, size.y);
        self.push_panel_like(bg_rect, style.bg, style.border, style.border_w, style.radius, 0);
        // 平移全部（子命令 + 背景/边框）：
        // 用 `display_pos`（拖拽中 = 本帧新位置）→ 文字/矩形**当帧生效**。
        for d in &mut self.queue[start..] {
            d.translate(display_pos);
        }
        size
    }

    /// **窗口**容器（可重叠 + 焦点置顶 + 可拖拽）。
    ///
    /// - **可重叠**：多个窗口按 **z-order** 排列（`UiState.window_z`），
    ///   点击窗口即**置顶**（焦点）；z 越大越靠上。
    /// - **遮挡隔离**（点击穿透修复）：重叠区域只让**鼠标下最上层**的窗口响应——
    ///   背后窗口的控件在该区域不命中、不可拖拽/置顶（见 [`crate::hit::window_occluded`]）。
    /// - **可拖拽**：按住窗口任意处**移动 ≥ 3 物理像素**进入拖拽（位置持久于
    ///   `UiState.panel_pos`）；**纯点击不拖拽**——窗口内子控件（按钮 / 勾选框 /
    ///   输入框）正常响应；拖动期间抑制窗口内子控件交互。
    /// - 窗口内绘制顺序由 [`Ui::finish`] 保证：**背景/图形严格先于文字**
    ///   （[`crate::draw::DrawKind::group`]；白纹理组先于字形图集组提交），
    ///   不做窗口内元素重叠处理。
    /// - 窗口内容（含背景/边框）带 `win = z` 标记，`finish` 时按
    ///   `(win, depth, kind, seq)` 排序后，按 `(win 升序, 图形 → 文字, 纹理)` 提交。
    pub fn window_at(
        &mut self,
        id: &str,
        pos: Vec2,
        f: impl FnOnce(&mut Window<'_, '_>),
    ) -> Vec2 {
        // z-order：首次分配 max+1；点击置顶在拖拽判定处处理
        let z = {
            let max_z = self.state.window_z.values().copied().max().unwrap_or(0);
            *self.state.window_z.entry(id.to_owned()).or_insert(max_z + 1)
        };
        let saved_win = std::mem::replace(&mut self.cur_win, z);
        let origin = self.state.panel_pos.get(id).copied().unwrap_or(pos);
        let start = self.queue.len();
        let (pad_total, gap) = {
            let p = &self.theme.panel;
            (p.padding + p.border_w, self.theme.gap)
        };
        let saved_base = self.abs_base;
        self.abs_base = saved_base + origin;
        self.frames.push(Frame::new_stack(PackSide::Top, gap, pad_total));
        self.depth += 1;
        let mut w = Window { ui: self };
        f(&mut w);
        let frame = self.frames.pop().expect("window frame");
        let size = frame.settle_size();
        self.depth -= 1;
        self.abs_base = saved_base;
        // 拖拽 + 按下裁决（物理像素粒度拖动基准，同 panel_impl）：
        // 重叠区域点击按下时，记录"本帧按下命中的**最上层**窗口"（win_press_top），
        // `finish::resolve_win_press` 只保留它的拖拽与置顶——避免同时拖动多个窗口。
        let panel_rect = Rect::new(origin.x, origin.y, size.x, size.y);
        // 窗口遮挡：被更高 z 的窗口覆盖的区域，本窗口不响应拖拽 / 置顶 /
        // 子控件交互（点击穿透修复——重叠区域只让最上层窗口可交互）。
        let hit = hit_test(&panel_rect, self.mouse_logical)
            && self.mouse_in_window
            && !window_occluded(z, self.mouse_logical, self.window_rects_iter());
        let btn = self.mouse_left();
        let press_here = btn.down_edge() && hit;
        // 输入框等文本控件按下时置位 press_claimed：**不建立窗口拖拽基准**
        // （从输入框上拖拽 = 选择文本；窗口改从空白/标题区拖动）。
        let drag_here = press_here && !self.press_claimed;
        if press_here {
            if self                .win_press_top
                .as_ref()
                .is_none_or(|(_, top_z)| self.cur_win > *top_z)
            {
                self.win_press_top = Some((id.to_owned(), self.cur_win));
            }
        }
        let (active, new_pos) = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let dragging = update_drag(ws, hit, btn);
            if drag_here {
                ws.press_panel = Some(panel_rect.min());
                ws.press_mouse = Some(self.mouse_screen.round());
            } else if press_here {
                // 文本框等子控件按下（选择拖拽优先）：**清除旧拖拽基准**——
                // 否则 `update_drag` 已置 dragging=true，残留的 press_mouse 会被
                // drag_moved 当作基准，算出巨大位移 → 窗口"瞬移"。
                ws.press_panel = None;
                ws.press_mouse = None;
            }
            // 拖拽需实际位移（≥ DRAG_ACTIVATE_PX）且**有本帧基准**才激活：纯点击不拖拽，
            // 窗口内子控件（按钮/勾选/输入框）正常响应（见 hit_abs 抑制条件）。
            let active = dragging
                && ws.press_mouse.is_some()
                && drag_moved(self.mouse_screen.round(), ws.press_mouse);
            let np = if active {
                let pp = ws.press_panel.unwrap_or(origin);
                let pm = ws.press_mouse.unwrap_or(self.mouse_screen);
                let d = (self.mouse_screen - pm).round();
                pp + d / self.scale
            } else {
                origin
            };
            (active, np)
        };
        if active {
            self.drag_panel = Some(id.to_owned());
            if self.state.panel_pos.get(id) != Some(&new_pos) {
                self.state.panel_pos.insert(id.to_owned(), new_pos);
            }
        } else if self.drag_panel.as_deref() == Some(id) {
            self.drag_panel = None;
        }
        if press_here || active {
            // 按下窗口（或拖拽中）都算"已响应按下"——避免空白点击清焦点
            self.any_pressed = true;
        }
        let display_pos = new_pos;
        // 记录窗口原点（顶点局部化基准；win=0 非窗口默认 (0,0)）与窗口 id（缓存 key）
        self.win_origins.insert(z, display_pos);
        self.win_ids.insert(z, id.to_owned());
        // 窗口矩形入遮挡判定缓存（跨帧；finish 末尾只保留本帧录制的窗口）。
        self.state
            .window_rects
            .insert(z, Rect::new(display_pos.x, display_pos.y, size.x, size.y));
        // 背景 + 边框（win = z，画在窗口子控件之下；radius > 0 走圆角双层矩形）
        let style = self.theme.panel.clone();
        let bg_rect = Rect::new(0.0, 0.0, size.x, size.y);
        self.push_panel_like(bg_rect, style.bg, style.border, style.border_w, style.radius, 0);
        for d in &mut self.queue[start..] {
            d.translate(display_pos);
        }
        self.cur_win = saved_win;
        size
    }

    /// pack 容器：按 `side` 堆叠，尺寸自动。
    pub fn pack_at(
        &mut self,
        pos: Vec2,
        side: PackSide,
        f: impl FnOnce(&mut Pack<'_, '_>),
    ) -> Vec2 {
        let gap = self.theme.gap;
        self.container(pos, Frame::new_stack(side, gap, 0.0), |ctx| {
            let mut p = Pack { ui: ctx.ui };
            f(&mut p);
        })
        .0
    }

    /// 当前容器**下一子项**的最小尺寸约束（`0` = 该轴不约束；一次性）。
    /// 容器内便捷方法：`p.min_size(120.0, 0.0)`（见 [`crate::Ui`] 文档 / 示例）。
    pub(crate) fn set_next_min(&mut self, min: Vec2) {
        self.frames
            .last_mut()
            .expect("min_size 需在容器内调用（顶层请用 *_at 定位）")
            .set_next_min(min);
    }

    /// 当前容器**下一子项**的最大尺寸约束（`0` = 该轴不约束；一次性）。
    pub(crate) fn set_next_max(&mut self, max: Vec2) {
        self.frames
            .last_mut()
            .expect("max_size 需在容器内调用（顶层请用 *_at 定位）")
            .set_next_max(max);
    }

    /// **flex 容器**：固定总高 `total_h`（逻辑像素），子项按 `weights` 权重**等分高度**
    /// （扣掉子项间距后按权重分配；权重全 0 时子项高为 0），回调按索引布局——
    /// 同帧精确分配，无需跨帧缓存；返回 `(最大子项宽, total_h)`。
    ///
    /// 子项内可放任意控件（`f.label` / `f.button` 等占光标，高度被强制为分配值）；
    /// 内容超高时**溢出可见**（需要滚动时在子项内嵌 [`Self::scroll_at`]）。
    /// `pos` 相对当前容器内容原点（顶层即屏幕原点），不占父容器光标。
    pub fn flex_at<F>(
        &mut self,
        pos: Vec2,
        total_h: f32,
        weights: &[u32],
        mut f: F,
    ) -> Vec2
    where
        F: FnMut(&mut FlexCtx<'_, '_>, usize),
    {
        let gap = self.theme.gap;
        let start = self.queue.len();
        let saved_base = self.abs_base;
        self.abs_base = saved_base + pos;
        let mut frame = Frame::new_stack(PackSide::Top, gap, 0.0);
        frame.set_fixed_h(total_h);
        self.frames.push(frame);
        self.depth += 1;
        let sum: u32 = weights.iter().sum();
        let gaps = gap * weights.len().saturating_sub(1) as f32;
        let usable = (total_h - gaps).max(0.0);
        {
            let mut fc = FlexCtx { ui: self };
            for (i, &w) in weights.iter().enumerate() {
                let h = if sum > 0 { usable * w as f32 / sum as f32 } else { 0.0 };
                fc.ui.frames.last_mut().expect("flex frame").force_next_h(h);
                f(&mut fc, i);
            }
        }
        let frame = self.frames.pop().expect("flex frame");
        let size = frame.settle_size();
        self.depth -= 1;
        self.abs_base = saved_base;
        for d in &mut self.queue[start..] {
            d.translate(pos);
        }
        size
    }

    /// grid 容器：`cols` 列均匀网格，单元格尺寸跨帧缓存（`id` 须稳定）。
    pub fn grid_at(
        &mut self,
        pos: Vec2,
        cols: usize,
        id: &str,
        f: impl FnOnce(&mut Grid<'_, '_>),
    ) -> Vec2 {
        assert!(cols > 0, "grid cols must be > 0");
        let cell = self.state.grid_cells.get(id).copied().unwrap_or(Vec2::ZERO);
        let (size, max_child) = self.container(pos, Frame::new_grid(cols, cell, 0.0), |ctx| {
            let mut g = Grid { ui: ctx.ui };
            f(&mut g);
        });
        // 回写单元格缓存（实际最大子尺寸；跨帧稳定布局）
        if max_child.x > 0.0 && max_child.y > 0.0 {
            self.state.grid_cells.insert(id.to_owned(), max_child);
        }
        size
    }

    // ── 提交 ─────────────────────────────────────────────────

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
    /// **提交方式**（不使用 Sprite）：全部图元（背景 / 控件背景 / 文字）转为
    /// **四边形顶点**（[`Render2D::add_quads`]），按 `(窗口, 纹理)` 分组后
    /// **由 UI 自行决定提交顺序**（不依赖 Render2D 排序）：
    /// `(win 升序, 白纹理图形组 → 字形文字组, 纹理 uid)`——
    /// 1. 非窗口内容（`win=0`）最底，窗口按 z 从下到上（`layer = base + z`）；
    /// 2. 同一窗口内**"背景/图形 → 文字"严格成立**（白纹理组先于字形图集组），
    ///    跨帧稳定、与 Render2D 任意排序模式结果一致。
    ///
    /// **UI 的 Render2D 必须关闭排序**（`set_sorting(false)`，完全按提交顺序绘制）：
    /// UI 自行管理绘制顺序，排序键 `(win, depth, elem, group, seq)` 依赖**提交顺序**
    /// 生效（图形组在文字组之前提交）。⚠ `set_sorting(true)`（`SortMode::LayerAndStates`）
    /// 会在同一 layer 内按 `(rstates, texture_uid)` 重排——字形图集页先于程序化纹理页
    /// （圆角/渐变）注册，重排后**圆角/渐变图形会盖住文字**；`set_layer_sort(true)`
    /// （`SortMode::LayerOnly`，稳定按 layer 排序）可接受（同层保持提交顺序）。
    pub fn finish(&mut self) {
        // 空白点击清焦点（本帧按下且无控件响应）
        if self.mouse_left().down_edge() && !self.any_pressed && self.state.focused.is_some() {
            self.state.focused = None;
        }
        // 清除一次性边沿
        for ws in self.state.widgets.values_mut() {
            clear_frame_flags(ws);
        }
        // 窗口按下裁决：重叠点击只让**最上层**窗口获得拖拽与置顶（见 window_at）
        self.resolve_win_press();
        // 键盘导航：Tab / Shift+Tab / 方向键遍历焦点链、Esc 关浮层/失焦、焦点描边。
        self.handle_focus_keys();
        // 排序（窗口 z → 深度 → 元素序 → 元素内图形/文字 → 命令序）：
        // 元素间按录制顺序（后录元素覆盖先录元素，重叠层级正确），
        // 元素内"背景/图形 → 文字"（DrawKind::group）——文字不被自身图形覆盖。
        self.queue
            .sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
        let queue = std::mem::take(&mut self.queue);
        // **WHITE 基础纹理优先取字形图集页**（`Text::white_region`，1×1 clamp_margin）：
        // 实心填充（Solid / 边框 / 光标）与字形**同页同纹理** → 同窗口内"图形组 → 文字组"
        // 相邻且同纹理，Render2D 合批为单个 draw call，省去图形↔文字的纹理状态切换。
        // 兜底：渲染器自带白纹理（整纹理 UV）。
        let (white_uid, white_uv_tl, white_uv_wh) = match self.text.white_region() {
            Some(r) => {
                let inv = match TEXTURES.get(r.page_uid) {
                    Some(t) => 1.0 / t.width as f32,
                    None => 1.0,
                };
                let tl = Vec2::new(r.tl_px.0 as f32, r.tl_px.1 as f32) * inv;
                let wh = Vec2::new(r.wh_px.0 as f32, r.wh_px.1 as f32) * inv;
                (r.page_uid, tl, wh)
            }
            None => {
                let uid = self.r2d.white_texture().uid;
                (uid, Vec2::ZERO, Vec2::ONE)
            }
        };
        // 按窗口分组：非窗口（win=0）每帧重建；窗口按**内容签名**缓存局部顶点，
        // 内容不变时复用（移动窗口只改变换，顶点不重建）。
        let mut groups: std::collections::HashMap<u32, Vec<UiDraw>> =
            std::collections::HashMap::new();
        for d in queue {
            groups.entry(d.win).or_default().push(d);
        }
        let mut wins: Vec<u32> = groups.keys().copied().collect();
        wins.sort_unstable();
        let mut quads = QuadCollector::new(white_uid, white_uv_tl, white_uv_wh); // 非窗口 + 缓存 miss 重建
        let mut cached: Vec<(u32, u8, u64, Vec<VertexP3U2C4>)> = Vec::new(); // 缓存命中
        for win in wins {
            let cmds = groups.remove(&win).expect("group exists");
            // debug_layout：每帧重建（布局描边是调试视图，跳过窗口顶点缓存）。
            if win == 0 || self.debug_layout {
                self.collect_cmds(&mut quads, win, &cmds);
                continue;
            }
            let Some(id) = self.win_ids.get(&win).cloned() else {
                self.collect_cmds(&mut quads, win, &cmds);
                continue;
            };
            // 内容签名：窗口命令的 (kind, rect, color, 文本…) 哈希
            let sig = {
                use std::hash::Hasher;
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for d in &cmds {
                    self.cmd_sig(&mut h, d);
                }
                h.finish()
            };
            // 命中缓存：直接用缓存的局部顶点（分组复制到提交列表），跳过重建
            {
                let entry = self
                    .state
                    .window_quads
                    .entry(id.clone())
                    .or_insert((0, Vec::new()));
                if entry.0 == sig {
                    for (g, tex, verts) in &entry.1 {
                        cached.push((win, *g, *tex, verts.clone()));
                    }
                    continue;
                }
            }
            // 未命中：收集该窗口命令为局部顶点，写入缓存
            let mut q = QuadCollector::new(white_uid, white_uv_tl, white_uv_wh);
            self.collect_cmds(&mut q, win, &cmds);
            let mut grp: Vec<(u8, u64, Vec<VertexP3U2C4>)> = Vec::new();
            for ((_, g, tex), verts) in q.quads {
                grp.push((g, tex, verts.clone()));
                cached.push((win, g, tex, verts));
            }
            // 缓存组顺序与提交顺序一致：图形（白纹理 / 程序化纹理）先于字形文字，
            // 再按纹理 uid——缓存命中的帧沿用该顺序，跨帧稳定。
            grp.sort_by_key(|&(g, tex, _)| (g, tex));
            self.state.window_quads.insert(id, (sig, grp));
        }
        // 提交：**UI 自行管理绘制顺序**，UI 的 Render2D 必须 `set_sorting(false)`
        // （关闭排序，完全按提交顺序绘制）；`set_layer_sort(true)`（LayerOnly，稳定排序）
        // 同层保持提交顺序也可。⚠ 不要用 `set_sorting(true)`（LayerAndStates）：
        // 它按 `(rstates, texture_uid)` 重排，字形图集页 uid < 程序化纹理页 uid →
        // 圆角/渐变会被排在文字之后绘制，盖住文字。
        //
        // 统一排序键 `(win, 图形/文字组, 纹理 uid)`，每 (窗口, 组, 纹理) 一次 add_quads：
        // 1. **win 升序**：非窗口内容（win=0，layer = base）最底，窗口按 z 从下到上
        //    （layer = base + z）——后提交的窗口覆盖先提交的；
        // 2. **窗口内图形组（白纹理 / 圆角 / 渐变 / 边框）先于字形文字组**——保证
        //    "先图形后文字"（含程序化纹理，不随纹理 uid 与白纹理比较而错乱），
        //    任意排序模式下绘制顺序完全相同、跨帧稳定。
        //
        // transform = 屏幕固定变换（窗口原点物理像素）→ 局部顶点映射到世界。
        let layer_base = self.base_layer;
        let mut ordered: Vec<(u32, u8, u64, Vec<VertexP3U2C4>)> =
            Vec::with_capacity(cached.len() + quads.quads.len());
        // mem::take：只移走内容四边形，`quads.debug`（调试叠加）留待最后提交。
        for ((win, g, tex_uid), verts) in std::mem::take(&mut quads.quads) {
            ordered.push((win, g, tex_uid, verts));
        }
        ordered.extend(cached);
        ordered.sort_by_key(|&(win, g, tex_uid, _)| (win, g, tex_uid));
        for (win, _g, tex_uid, verts) in ordered {
            let Some(tex) = TEXTURES.get(tex_uid) else {
                continue;
            };
            let anchor_px = self
                .win_origins
                .get(&win)
                .copied()
                .unwrap_or(Vec2::ZERO)
                * self.scale;
            let tf = screen_fixed_tf(self.cam, anchor_px);
            let layer = Layer::from(layer_base + win as f64 * 1.0);
            self.r2d.add_quads(&verts, tf, layer, &tex);
            // MeshBuilder Drop 即提交 ✓
        }

        // ── Debug 叠加（DebugDraw / debug_layout 描边）────────────
        // 在**全部 UI 内容之后**提交（同 layer 后提交 → 恒覆盖在最上）：
        // 1. 收集 `debug_queue`（[`Self::debug_line`] 等屏幕空间调试图元，
        //    坐标 = 绝对逻辑像素 → 窗口局部物理四边形）；
        // 2. 与 `collect_cmds` 期间产生的布局描边（`quads.debug`）合并；
        // 3. 按 win 分组、白纹理、屏幕固定变换提交（不进窗口缓存）。
        {
            let mut debug_groups: std::collections::HashMap<u32, Vec<UiDraw>> =
                std::collections::HashMap::new();
            for d in self.debug_queue.drain(..) {
                debug_groups.entry(d.win).or_default().push(d);
            }
            let mut dwins: Vec<u32> = debug_groups.keys().copied().collect();
            dwins.sort_unstable();
            for win in dwins {
                let cmds = debug_groups.remove(&win).expect("group exists");
                self.collect_cmds(&mut quads, win, &cmds);
            }
        }
        let mut dwins: Vec<u32> = quads.debug.keys().copied().collect();
        dwins.sort_unstable();
        for win in dwins {
            let verts = quads.debug.remove(&win).expect("debug group exists");
            let Some(tex) = TEXTURES.get(white_uid) else {
                continue;
            };
            let anchor_px = self
                .win_origins
                .get(&win)
                .copied()
                .unwrap_or(Vec2::ZERO)
                * self.scale;
            let tf = screen_fixed_tf(self.cam, anchor_px);
            let layer = Layer::from(layer_base + win as f64 * 1.0);
            self.r2d.add_quads(&verts, tf, layer, &tex);
        }
        // 记录 IME 组合状态（供下一帧退格判定，见 text_input_at）
        self.state.ime_composing =
            self.keyboard.get_ime_preedit().is_some_and(|p| !p.is_empty());
        // 窗口矩形遮挡缓存只保留**本帧录制过**的窗口（z 变化 / 窗口销毁的旧条目随帧清理）。
        self.state.window_rects.retain(|z, _| self.win_origins.contains_key(z));
        self.depth = 0;
        self.seq = 0;
        self.cur_win = 0;
        self.any_pressed = false;
        self.drag_panel = None;
        self.win_press_top = None;
        self.win_origins.clear();
        self.win_ids.clear();
        self.frames.clear();
        self.focusables.clear();
        self.press_claimed = false;
    }

    /// 把一组命令收集为四边形顶点（**相对窗口原点的局部物理像素**；
    /// `win` 决定局部化基准，非窗口 win=0 基准 (0,0)）。
    ///
    /// `debug_layout` 开启时，每个命令的矩形同时向 `quads.debug` 追加**青色描边**
    /// （调试 rjw_ui 自身的布局 / 命中区域）；`DrawKind::Debug` 命令（[`Self::debug_line`]
    /// 等屏幕空间调试图元）则只写入 `quads.debug`（覆盖在 UI 内容之上）。
    fn collect_cmds(&mut self, quads: &mut QuadCollector, win: u32, cmds: &[UiDraw]) {
        let anchor_px = self
            .win_origins
            .get(&win)
            .copied()
            .unwrap_or(Vec2::ZERO)
            * self.scale;
        let dbg = if self.debug_layout {
            // debug_layout 描边样式：读 Theme::debug（Copy 值先取出，
            // 避免与循环内 `&mut self` 调用（draw_text_quads）的借用冲突）。
            Some((self.theme.debug.layout_outline, self.theme.debug.layout_outline_width))
        } else {
            None
        };
        for d in cmds {
            // 裁剪区（绝对物理；内容已随容器平移成绝对逻辑坐标）。
            let clip_abs = d.clip.map(|c| snap_rect(&self.phys_rect(&c)));
            match &d.kind {
                DrawKind::Solid(color) => {
                    if d.rect.w > 0.0 && d.rect.h > 0.0 {
                        let pr = snap_rect(&self.phys_rect(&d.rect));
                        if let Some(r) = clipped(pr, clip_abs) {
                            quads.push_white(
                                win,
                                Rect::new(r.x - anchor_px.x, r.y - anchor_px.y, r.w, r.h),
                                *color,
                            );
                            debug_layout_outline(quads, win, anchor_px, r, dbg);
                        }
                    }
                }
                DrawKind::RoundedRect { color, radius } => {
                    let pr = snap_rect(&self.phys_rect(&d.rect));
                    if let Some(local) = clipped(pr, clip_abs).map(|r| {
                        Rect::new(r.x - anchor_px.x, r.y - anchor_px.y, r.w, r.h)
                    }) {
                        if local.w > 0.0 && local.h > 0.0 {
                            // 半径转物理像素并 clamp（与生成纹理时的 clamp 一致）。
                            let r = (*radius * self.scale)
                                .clamp(0.0, crate::proc::ROUNDED_TEX_SIZE as f32 * 0.5 - 1.0)
                                .max(0.0);
                            if r <= 0.0 {
                                quads.push_white(win, local, *color);
                            } else {
                                let device = self.r2d.device();
                                let queue = self.r2d.queue();
                                let layout = self.r2d.tex_bind_group_layout();
                                if let Some((tex_uid, region)) =
                                    self.state.proc.rounded(device, queue, layout, r)
                                {
                                    if let Some(tex) = TEXTURES.get(tex_uid) {
                                        let inv_page = 1.0 / tex.width as f32;
                                        let base = Vec2::new(
                                            region.tl_px.0 as f32,
                                            region.tl_px.1 as f32,
                                        ) * inv_page;
                                        let tex_wh = Vec2::new(
                                            region.wh_px.0 as f32,
                                            region.wh_px.1 as f32,
                                        ) * inv_page;
                                        // 9-patch：四角原样、四边/中心拉伸（任意尺寸圆弧不畸变）。
                                        for (mr, uvtl, uvwh) in crate::proc::rounded_9patch(
                                            local,
                                            r,
                                            crate::proc::ROUNDED_TEX_SIZE,
                                            r,
                                        ) {
                                            if mr.w <= 0.0 || mr.h <= 0.0 {
                                                continue;
                                            }
                                            quads.push_tex_rect(
                                                win,
                                                tex_uid,
                                                base + uvtl * tex_wh,
                                                uvwh * tex_wh,
                                                mr,
                                                *color,
                                            );
                                        }
                                    }
                                } else {
                                    quads.push_white(win, local, *color);
                                }
                            }
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                    }
                }
                DrawKind::Gradient { axis, stops } => {
                    let pr = snap_rect(&self.phys_rect(&d.rect));
                    if let Some(local) = clipped(pr, clip_abs).map(|r| {
                        Rect::new(r.x - anchor_px.x, r.y - anchor_px.y, r.w, r.h)
                    }) {
                        if local.w > 0.0 && local.h > 0.0 {
                            let vertical = matches!(axis, GradientAxis::Vertical);
                            let device = self.r2d.device();
                            let queue = self.r2d.queue();
                            let layout = self.r2d.tex_bind_group_layout();
                            if let Some((tex_uid, region)) =
                                self.state.proc.gradient(device, queue, layout, vertical, stops)
                            {
                                if let Some(tex) = TEXTURES.get(tex_uid) {
                                    let inv_page = 1.0 / tex.width as f32;
                                    let base = Vec2::new(
                                        region.tl_px.0 as f32,
                                        region.tl_px.1 as f32,
                                    ) * inv_page;
                                    let tex_wh = Vec2::new(
                                        region.wh_px.0 as f32,
                                        region.wh_px.1 as f32,
                                    ) * inv_page;
                                    // 整矩形拉伸采样渐变纹理（主轴 64 级已平滑）。
                                    quads.push_tex_rect(
                                        win,
                                        tex_uid,
                                        base,
                                        tex_wh,
                                        local,
                                        Color::WHITE,
                                    );
                                }
                            }
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                    }
                }
                DrawKind::Border { color, width } => {
                    let pr = snap_rect(&self.phys_rect(&d.rect));
                    if let Some(r) = clipped(pr, clip_abs) {
                        let local = Rect::new(r.x - anchor_px.x, r.y - anchor_px.y, r.w, r.h);
                        for br in border_rects(&local, self.phys_f(*width).round()) {
                            if br.w > 0.0 && br.h > 0.0 {
                                quads.push_white(win, br, *color);
                            }
                        }
                        debug_layout_outline(quads, win, anchor_px, pr, dbg);
                    }
                }
                DrawKind::Text {
                    text,
                    size,
                    color,
                    align,
                    valign,
                    family,
                    clip,
                    buf,
                } => {
                    // 外层裁剪（绝对逻辑）→ 相对文本块左上角（与 DrawKind::Text::clip 同空间），
                    // 与命令自带裁剪求交后传给 draw_text_quads。
                    let merged = match d.clip.map(|c| {
                        Rect::new(c.x - d.rect.x, c.y - d.rect.y, c.w, c.h)
                    }) {
                        Some(outer) => match clip {
                            Some(inner) => intersect_rect(&outer, inner),
                            None => Some(outer),
                        },
                        None => *clip,
                    };
                    self.draw_text_quads(
                        quads,
                        win,
                        anchor_px,
                        &d.rect,
                        text,
                        *size,
                        *color,
                        *align,
                        *valign,
                        family.as_deref(),
                        merged,
                        buf.as_ref().map(Arc::clone),
                    );
                    let pr = snap_rect(&self.phys_rect(&d.rect));
                    debug_layout_outline(quads, win, anchor_px, pr, dbg);
                }
                DrawKind::Caret { color, width } => {
                    let r = Rect::new(d.rect.x, d.rect.y, *width, d.rect.h);
                    let pr = snap_rect(&self.phys_rect(&r));
                    if let Some(rr) = clipped(pr, clip_abs) {
                        if rr.w > 0.0 && rr.h > 0.0 {
                            quads.push_white(
                                win,
                                Rect::new(rr.x - anchor_px.x, rr.y - anchor_px.y, rr.w, rr.h),
                                *color,
                            );
                            debug_layout_outline(quads, win, anchor_px, pr, dbg);
                        }
                    }
                }
                DrawKind::Debug { color, shape } => {
                    // 屏幕空间调试图元：逻辑像素 → 物理像素线段，转窗口局部写入 debug 叠加。
                    for ([a, b], w) in debug_shape_segments(shape, self.scale) {
                        quads.push_debug_line(win, a - anchor_px, b - anchor_px, w, *color);
                    }
                }
            }
        }
    }

    /// 窗口内容签名：命令的 `(kind, rect, color, 文本…)` 哈希（窗口顶点缓存 key 用）。
    /// 忽略 `win/seq`（窗口内固定）；任何影响渲染的内容变化都会改变签名。
    /// 
    /// **包含文本缓存版本号**：`TEXT_LINE_HEIGHT_VERSION` 变化时，窗口缓存自动失效，
    /// 避免新旧行高混用导致布局错乱。
    fn cmd_sig(&self, h: &mut std::collections::hash_map::DefaultHasher, d: &UiDraw) {
        use std::hash::Hash;
        d.depth.hash(h);
        d.elem.hash(h);
        d.rect.x.to_bits().hash(h);
        d.rect.y.to_bits().hash(h);
        d.rect.w.to_bits().hash(h);
        d.rect.h.to_bits().hash(h);
        match &d.kind {
            DrawKind::Solid(c) => {
                0u8.hash(h);
                color_bits(*c).hash(h);
            }
            DrawKind::RoundedRect { color, radius } => {
                5u8.hash(h);
                color_bits(*color).hash(h);
                radius.to_bits().hash(h);
            }
            DrawKind::Gradient { axis, stops } => {
                6u8.hash(h);
                match axis {
                    GradientAxis::Vertical => 0u8.hash(h),
                    GradientAxis::Horizontal => 1u8.hash(h),
                }
                stops.len().hash(h);
                for (t, c) in stops {
                    t.to_bits().hash(h);
                    color_bits(*c).hash(h);
                }
            }
            DrawKind::Border { color, width } => {
                1u8.hash(h);
                color_bits(*color).hash(h);
                width.to_bits().hash(h);
            }
            DrawKind::Text {
                text,
                size,
                color,
                align,
                valign,
                family,
                clip,
                buf: _,
            } => {
                2u8.hash(h);
                text.hash(h);
                size.to_bits().hash(h);
                color_bits(*color).hash(h);
                (*align as u8).hash(h);
                (*valign as u8).hash(h);
                family.hash(h);
                // 文本缓存版本号影响排版结果，必须包含在签名中
                TEXT_LINE_HEIGHT_VERSION.hash(h);
                if let Some(c) = clip {
                    c.x.to_bits().hash(h);
                    c.y.to_bits().hash(h);
                    c.w.to_bits().hash(h);
                    c.h.to_bits().hash(h);
                }
            }
            DrawKind::Caret { color, width } => {
                3u8.hash(h);
                color_bits(*color).hash(h);
                width.to_bits().hash(h);
            }
            DrawKind::Debug { color, shape } => {
                4u8.hash(h);
                color_bits(*color).hash(h);
                // 形状参数逐字段哈希（DebugShape 未实现 Hash）。
                match shape {
                    DebugShape::Line { a, b, width } => {
                        0u8.hash(h);
                        a.x.to_bits().hash(h);
                        a.y.to_bits().hash(h);
                        b.x.to_bits().hash(h);
                        b.y.to_bits().hash(h);
                        width.to_bits().hash(h);
                    }
                    DebugShape::RectOutline { rect, width } => {
                        1u8.hash(h);
                        rect.x.to_bits().hash(h);
                        rect.y.to_bits().hash(h);
                        rect.w.to_bits().hash(h);
                        rect.h.to_bits().hash(h);
                        width.to_bits().hash(h);
                    }
                    DebugShape::CircleOutline {
                        center,
                        radius,
                        segments,
                        width,
                    } => {
                        2u8.hash(h);
                        center.x.to_bits().hash(h);
                        center.y.to_bits().hash(h);
                        radius.to_bits().hash(h);
                        segments.hash(h);
                        width.to_bits().hash(h);
                    }
                    DebugShape::Cross { center, half, width } => {
                        3u8.hash(h);
                        center.x.to_bits().hash(h);
                        center.y.to_bits().hash(h);
                        half.to_bits().hash(h);
                        width.to_bits().hash(h);
                    }
                    DebugShape::Grid { rect, spacing, width } => {
                        4u8.hash(h);
                        rect.x.to_bits().hash(h);
                        rect.y.to_bits().hash(h);
                        rect.w.to_bits().hash(h);
                        rect.h.to_bits().hash(h);
                        spacing.to_bits().hash(h);
                        width.to_bits().hash(h);
                    }
                }
            }
        }
    }

    /// 窗口按下裁决：本帧若有窗口被按下（重叠区域点击），**只保留最上层窗口**
    /// 的拖拽（其余取消，修复"重叠时同时拖动两个窗口"），且仅最上层窗口置顶。
    fn resolve_win_press(&mut self) {
        let Some((top_id, old_z)) = self.win_press_top.clone() else {
            return;
        };
        // 只保留最高 z 命中窗口的拖拽
        for (wid, ws) in self.state.widgets.iter_mut() {
            if wid != &top_id && ws.dragging && !ws.pressed {
                ws.dragging = false;
            }
        }
        // 仅最上层命中窗口置顶（z+1；本帧命令仍按旧 z，下一帧生效）
        let max_z = self.state.window_z.values().copied().max().unwrap_or(0);
        let new_z = max_z + 1;
        self.state.window_z.insert(top_id.clone(), new_z);
        // **焦点归属清理**：焦点控件若在**其他窗口**（本次置顶的窗口之外）——
        // 清除焦点。否则旧输入框在窗口被盖住后仍持焦点（点击被遮挡无法再聚焦、
        // 打字落入不可见输入框），表现为"使用其他窗口后文本框失效"。
        // ⚠ 与**置顶前**的 z（`old_z`，win_press_top 记录点击时的窗口 z）比较——
        // 焦点条目本帧以旧 z 录制；拿置顶后的 `new_z` 比会恒不相等 → 点击输入框
        // （其所在窗口同时置顶）焦点被立即清除，窗口内文本框"无法使用"。
        if let Some(fid) = &self.state.focused {
            let fwin = self.focusables.iter().find(|e| e.id == *fid).map(|e| e.win);
            if fwin.is_some_and(|w| w != 0 && w != old_z) {
                self.state.focused = None;
            }
        }
        // 诊断：记录本次按下由哪个窗口接收（重叠点击时"赢家"）。
        self.state.last_press_window = Some((top_id, new_z));
    }

    /// **键盘导航**（`finish` 末尾调用）：
    ///
    /// - **Tab / Shift+Tab / 方向键**：按 `(win, 注册序)` 排序的焦点链遍历
    ///   （[`focus_step`]），更新 `UiState.focused`；焦点控件本帧未录制时自动清除；
    /// - **Esc**：优先收起展开的下拉框，否则取消焦点；
    /// - **焦点描边**：对当前焦点控件画一圈描边（[`crate::style::FocusStyle`]，
    ///   `Theme::focus`；elem 取全局最大 → 画在窗口内容之上，裁剪沿用控件自身）。
    fn handle_focus_keys(&mut self) {
        // 链排序：按 (win, 注册序) 稳定排序（非窗口 0 在前，窗口按 z 从下到上）。
        let mut chain: Vec<&FocusEntry> = self.focusables.iter().collect();
        chain.sort_by_key(|e| e.win);
        // 焦点控件本帧未录制（所在窗口关闭 / 控件移除）→ 清除焦点。
        if let Some(fid) = &self.state.focused {
            if !chain.iter().any(|e| e.id == *fid) {
                self.state.focused = None;
            }
        }
        // 移动：Tab（+1）/ Shift+Tab（-1）/ Down（+1）/ Up（-1）。
        // ⚠ IME 组合中（preedit 非空或上帧在组合）**禁止方向键/Tab 移动焦点**——
        // 中文输入法用 ↑/↓ 切换候选、Enter 上屏，焦点被移走会立刻打断输入（文本框"失效"）。
        let composing = self
            .keyboard
            .get_ime_preedit()
            .is_some_and(|p| !p.is_empty())
            || self.state.ime_composing;
        let shift = self.keyboard.get(KeyCode::ShiftLeft).pressed()
            || self.keyboard.get(KeyCode::ShiftRight).pressed();
        let dir: i32 = if composing {
            0
        } else if self.keyboard.get(KeyCode::Tab).down_edge() {
            if shift { -1 } else { 1 }
        } else if self.keyboard.get(KeyCode::ArrowDown).down_edge() {
            1
        } else if self.keyboard.get(KeyCode::ArrowUp).down_edge() {
            -1
        } else {
            0
        };
        if dir != 0 {
            let next = focus_step(&chain, self.state.focused.as_deref(), dir);
            self.state.focused = next;
        }
        // Esc：优先收起下拉框，否则取消焦点。
        if self.keyboard.get(KeyCode::Escape).down_edge() {
            if self.state.combo_open.is_some() {
                self.state.combo_open = None;
            } else if self.state.focused.is_some() {
                self.state.focused = None;
            }
        }
        // 焦点描边：对当前焦点控件画一圈 Border（elem 全局最大 → 画在窗口内容之上）。
        if let Some(fid) = &self.state.focused {
            let Some(entry) = chain.iter().find(|e| e.id == *fid) else {
                return;
            };
            // 先拷贝字段，结束对 chain 的借用（随后需要 &mut self）。
            let (win, depth, rect, clip) = (entry.win, entry.depth, entry.rect, entry.clip);
            let focus = self.theme.focus.clone();
            let elem = self.seq + 1;
            let seq = self.next_seq();
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect,
                clip,
                kind: DrawKind::Border { color: focus.color, width: focus.width },
            });
        }
    }

    /// 文本 → 字形四边形（收集到 `quads`；按字形图集页纹理分组）。
    ///
    /// **精确裁切**（"半消失"）：字形与裁剪区求交，相交部分生成裁剪后的四边形
    /// （UV 按比例同步缩放）——字形在裁剪线处被**部分绘制**，而非整字形保留/消失
    /// （`rjw_text` 的 `cull` 只做整字形剔除，像素级裁剪在此完成）。
    #[allow(clippy::too_many_arguments)]
    fn draw_text_quads(
        &mut self,
        quads: &mut QuadCollector,
        win: u32,
        anchor_px: Vec2,
        rect: &Rect,
        text: &str,
        size: f32,
        color: Color,
        align: TextAlign,
        valign: TextVAlign,
        family: Option<&str>,
        clip: Option<Rect>,
        buf: Option<Arc<Buffer>>,
    ) {
        if rect.w <= 0.0 || rect.h <= 0.0 || text.is_empty() {
            return;
        }
        // 全部换算物理像素（锚点 / 裁剪区），与屏幕固定变换 1:1 匹配；
        // 排版缓冲：控件自持（输入框）或按需缓存（静态标签）。
        let pr = snap_rect(&self.phys_rect(rect));
        // 矩形锚点（**整数像素**，逐项取整，无小数参与加法）：
        // 水平：左 = 左缘、中 = 左缘 + 半宽取整、右 = 右缘；
        // 垂直：Center = 上缘 + 半高取整，Top = 上缘（TextArea 多行顶对齐）。
        let anchor = Vec2::new(
            match align {
                TextAlign::Left => pr.x,
                TextAlign::Center => pr.x + (pr.w * 0.5).round(),
                TextAlign::Right => pr.x + pr.w,
            },
            match valign {
                TextVAlign::Top => pr.y,
                TextVAlign::Center => pr.y + (pr.h * 0.5).round(),
            },
        );
        // 局部化基准 = **窗口原点**（anchor_px），不是文字锚点：
        // 字形世界坐标 - 窗口原点世界 = 相对窗口原点的局部顶点，
        // 提交时经窗口 transform（screen_fixed_tf(窗口原点)）映射回世界。
        let win_anchor_world = self.cam.screen_to_world(anchor_px);
        let buf = match buf {
            Some(b) => b,
            None => self.cache_buffer(text, size, family),
        };
        let mut tr = self.text.render_from(&buf);
        // 垂直定位按**行盒**（行高），而非字形墨迹内容：
        // 以首行行顶（相对文本视觉原点）为参考，使行盒中心对准锚点（矩形垂直中心）。
        // 字形在行盒内按基线排布——矮小写字母（如 "a"，无 descender）落在基线上，
        // 不再因"以墨迹顶为参考"（旧实现把 `[视觉原点, 视觉原点+行高]` 当块居中，
        // 行盒整体上移约一个 top-bearing）而浮在行盒上部偏上显示。
        let content = tr.content_size();
        // `l.top_left.y` 为**整数**（rjw_text 收集期已对行顶取整）。
        let first_line_top = match tr.lines().first() {
            Some(l) => l.top_left.y,
            None => 0.0,
        };
        // **整数加法不变量**：`block_tl = anchor + off` 的两侧均为整数——
        // 锚点（上方逐项取整）、`text_block_offset`（content / first_line_top 均为
        // 整数，内部只有整数加减与边界 `round`）。0.5px 小数（居中奇数宽 / 行盒偏移）
        // 在 `round` 边界被一次性消化，不会流入加法链 → 无误差累加；
        // 字形四边形角点 = block_tl + 整数字形偏移 = 整数屏幕像素 → 采样精确落在
        // 纹素中心，消除 1:1 图集双线性采样的亚像素模糊。
        let block_tl = anchor + text_block_offset(align, valign, content, first_line_top);
        let tf = screen_fixed_tf(self.cam, block_tl);
        tr.origin(Vec2::ZERO).transform(tf).color(color);
        // 裁剪区：相对命令矩形（`merged`，逻辑）→ **窗口局部**。
        // ⚠ 相对基准是命令矩形物理 `pr`，**不是**文本块原点 `block_tl`：
        // `merged` 相对 `d.rect`，而字形窗口局部坐标以 `anchor_px` 为原点——
        // clip 窗口局部 = merged×scale + (pr - anchor_px)。用 block_tl 会整体错位
        // (block_tl - pr)（Top 对齐 / 多行时垂直偏差数像素）→ 滚动后文本不消失/错位。
        let clip_local: Option<Rect> = clip.map(|c| {
            let pc = snap_rect(&Rect::new(
                c.x * self.scale,
                c.y * self.scale,
                c.w * self.scale,
                c.h * self.scale,
            ));
            Rect::new(
                pc.x + pr.x - anchor_px.x,
                pc.y + pr.y - anchor_px.y,
                pc.w,
                pc.h,
            )
        });
        // 像素级裁剪完全由下方逐字形求交完成（完全在外 → 剔除；部分相交 → "半消失"）。
        // 不再使用 rjw_text 的 clip/cull（其坐标系相对字形 top_left，语义易错位）。
        let page_size = tr.page_size();
        let inv_page = 1.0 / page_size;
        let ca: [f32; 4] = color.into();
        tr.draw_with(|_m, _line, region, transform| {
            // 字形精灵（轴对齐四边形）：四角经 transform 到世界坐标，再转窗口局部 + 图集 UV
            let w = region.wh_px.0 as f32;
            let h = region.wh_px.1 as f32;
            let tl = transform.transform_point(Vec2::new(0.0, 0.0)) - win_anchor_world;
            let tr_p = transform.transform_point(Vec2::new(w, 0.0)) - win_anchor_world;
            let bl = transform.transform_point(Vec2::new(0.0, h)) - win_anchor_world;
            let uv_tl = Vec2::new(
                region.tl_px.0 as f32 * inv_page,
                region.tl_px.1 as f32 * inv_page,
            );
            let uv_wh = Vec2::new(w * inv_page, h * inv_page);
            // 字形窗口局部 AABB（轴对齐；屏幕固定变换下无旋转）。
            let gx = tl.x;
            let gy = tl.y;
            let gw = tr_p.x - tl.x;
            let gh = bl.y - tl.y;
            let (qx0, qy0, qx1, qy1) = match &clip_local {
                Some(c) => {
                    // 与裁剪区求交：无交集 → 整字形剔除（"完全消失"）；
                    // 部分相交 → 生成裁剪后四边形（"半消失"，UV 按比例缩放）。
                    let ix0 = gx.max(c.x);
                    let iy0 = gy.max(c.y);
                    let ix1 = (gx + gw).min(c.x + c.w);
                    let iy1 = (gy + gh).min(c.y + c.h);
                    if ix1 <= ix0 || iy1 <= iy0 {
                        return;
                    }
                    (ix0, iy0, ix1, iy1)
                }
                None => (gx, gy, gx + gw, gy + gh),
            };
            let nw = qx1 - qx0;
            let nh = qy1 - qy0;
            let u0 = uv_tl.x + (qx0 - gx) / gw * uv_wh.x;
            let v0 = uv_tl.y + (qy0 - gy) / gh * uv_wh.y;
            let u1 = u0 + nw / gw * uv_wh.x;
            let v1 = v0 + nh / gh * uv_wh.y;
            let quad = [
                vertex_p3u2c4(Vec2::new(qx0, qy0), [u0, v0], ca),
                vertex_p3u2c4(Vec2::new(qx1, qy0), [u1, v0], ca),
                vertex_p3u2c4(Vec2::new(qx0, qy1), [u0, v1], ca),
                vertex_p3u2c4(Vec2::new(qx1, qy1), [u1, v1], ca),
            ];
            quads.push_tex_quad(win, region.page_uid, quad);
        });
    }
}

// ─── 四边形收集器（QuadVertices） ──────────────────────────────

/// 分组维度（提交排序用，见 [`submit_group_key`]）：
/// - `0` = 图形（Solid / RoundedRect / Gradient / Border / Caret——白纹理与程序化纹理）；
/// - `1` = 文字（字形图集纹理）。
pub(crate) const GROUP_GRAPHIC: u8 = 0;
pub(crate) const GROUP_TEXT: u8 = 1;

/// 按 `(窗口 z, 图形/文字组, 纹理 uid)` 分组的四边形顶点收集器（finish 提交用）。
///
/// `debug`：**屏幕调试叠加**（DebugDraw 图元 + debug_layout 布局描边）——
/// 按 `win` 分组、恒用白纹理，`finish` 时在全部 UI 内容**之后**提交。
struct QuadCollector {
    quads: std::collections::HashMap<(u32, u8, u64), Vec<VertexP3U2C4>>,
    /// 调试叠加顶点（白纹理；窗口局部物理坐标）。
    debug: std::collections::HashMap<u32, Vec<VertexP3U2C4>>,
    white_uid: u64,
    /// WHITE 纹理区域 UV（字形图集页白纹理 region；兜底为整纹理 [0,1)）。
    white_uv_tl: Vec2,
    white_uv_wh: Vec2,
}

impl QuadCollector {
    fn new(white_uid: u64, white_uv_tl: Vec2, white_uv_wh: Vec2) -> Self {
        Self {
            quads: std::collections::HashMap::new(),
            debug: std::collections::HashMap::new(),
            white_uid,
            white_uv_tl,
            white_uv_wh,
        }
    }

    /// 白色纹理四边形（背景 / 边框 / 光标；WHITE region UV；图形组）。
    fn push_white(&mut self, win: u32, r: Rect, c: Color) {
        self.push_tex_rect(win, self.white_uid, self.white_uv_tl, self.white_uv_wh, r, c);
    }

    /// 带 UV 子区域的矩形四边形（图形组；圆角 9-patch / 渐变 / WHITE region 用）。
    fn push_tex_rect(
        &mut self,
        win: u32,
        tex: u64,
        uv_tl: Vec2,
        uv_wh: Vec2,
        r: Rect,
        c: Color,
    ) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let ca: [f32; 4] = c.into();
        let uv_br = uv_tl + uv_wh;
        let quad = [
            vertex_p3u2c4(Vec2::new(r.x, r.y), [uv_tl.x, uv_tl.y], ca),
            vertex_p3u2c4(Vec2::new(r.x + r.w, r.y), [uv_br.x, uv_tl.y], ca),
            vertex_p3u2c4(Vec2::new(r.x, r.y + r.h), [uv_tl.x, uv_br.y], ca),
            vertex_p3u2c4(Vec2::new(r.x + r.w, r.y + r.h), [uv_br.x, uv_br.y], ca),
        ];
        self.push_tex_quad_group(win, tex, quad, GROUP_GRAPHIC);
    }

    /// 追加一个带 UV 的四边形（字形用；文字组）。
    fn push_tex_quad(&mut self, win: u32, tex: u64, quad: [VertexP3U2C4; 4]) {
        self.push_tex_quad_group(win, tex, quad, GROUP_TEXT);
    }

    fn push_tex_quad_group(
        &mut self,
        win: u32,
        tex: u64,
        quad: [VertexP3U2C4; 4],
        group: u8,
    ) {
        self.quads
            .entry((win, group, tex))
            .or_default()
            .extend_from_slice(&quad);
    }

    /// 调试叠加：一条带厚度线段（白纹理实心色；UV 取 WHITE region 中心）。
    /// 几何复用 [`rjw_2d_render::debug_draw::thick_line_quad`]；退化线段（零长 / 零宽）跳过。
    fn push_debug_line(&mut self, win: u32, a: Vec2, b: Vec2, width: f32, c: Color) {
        let Some([tl, tr, bl, br]) = rjw_2d_render::debug_draw::thick_line_quad(a, b, width) else {
            return;
        };
        let ca: [f32; 4] = c.into();
        // UV 必须落在 WHITE region 内（[0,0] 会采样字形页左上角，可能是字形像素）。
        let uv = self.white_uv_tl + self.white_uv_wh * 0.5;
        let quad = [
            vertex_p3u2c4(tl, [uv.x, uv.y], ca),
            vertex_p3u2c4(tr, [uv.x, uv.y], ca),
            vertex_p3u2c4(bl, [uv.x, uv.y], ca),
            vertex_p3u2c4(br, [uv.x, uv.y], ca),
        ];
        self.debug.entry(win).or_default().extend_from_slice(&quad);
    }

    /// 调试叠加：矩形描边（4 条带厚度线段；`width` 为物理像素）。
    fn push_debug_rect(&mut self, win: u32, r: Rect, width: f32, c: Color) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let tl = Vec2::new(r.x, r.y);
        let tr = Vec2::new(r.x + r.w, r.y);
        let br = Vec2::new(r.x + r.w, r.y + r.h);
        let bl = Vec2::new(r.x, r.y + r.h);
        self.push_debug_line(win, tl, tr, width, c);
        self.push_debug_line(win, tr, br, width, c);
        self.push_debug_line(win, br, bl, width, c);
        self.push_debug_line(win, bl, tl, width, c);
    }
}

/// 构造一个顶点（世界坐标 + UV + 颜色）。
#[inline]
fn vertex_p3u2c4(pos: Vec2, uv: [f32; 2], color: [f32; 4]) -> VertexP3U2C4 {
    VertexP3U2C4 {
        pos: [pos.x, pos.y, 0.0],
        uv,
        color,
    }
}

/// 颜色位模式（签名哈希用）。
#[inline]
fn color_bits(c: Color) -> [u32; 4] {
    let a: [f32; 4] = c.into();
    [
        a[0].to_bits(),
        a[1].to_bits(),
        a[2].to_bits(),
        a[3].to_bits(),
    ]
}

/// debug_layout 模式：给已取整的物理矩形画一圈描边（转窗口局部坐标后写入 debug 叠加）。
/// `dbg = None` 时零开销；`Some((color, width))` 取 [`Theme::debug`] 样式。
#[inline]
fn debug_layout_outline(
    quads: &mut QuadCollector,
    win: u32,
    anchor_px: Vec2,
    pr: Rect,
    dbg: Option<(Color, f32)>,
) {
    let Some((color, width)) = dbg else {
        return;
    };
    let r = Rect::new(pr.x - anchor_px.x, pr.y - anchor_px.y, pr.w, pr.h);
    quads.push_debug_rect(win, r, width, color);
}

/// 拖拽激活所需的最小**物理像素**位移：按下后鼠标位移 ≥ 此值才视为“拖拽”。
///
/// 纯点击（无位移）不激活拖拽 → 窗口 / 可拖拽面板内的子控件（按钮 / 勾选框 /
/// 输入框等）**正常响应点击**；真正拖动中才抑制子控件交互（防止误触）。
const DRAG_ACTIVATE_PX: f32 = 3.0;

/// 是否已产生足以激活拖拽的位移（`current_px` / `press_px` 均为**物理像素**，
/// 已取整；`None` = 无按下基准，未激活）。
#[inline]
fn drag_moved(current_px: Vec2, press_px: Option<Vec2>) -> bool {
    match press_px {
        Some(p) => {
            (current_px - p).length_squared() >= DRAG_ACTIVATE_PX * DRAG_ACTIVATE_PX
        }
        None => false,
    }
}

/// 按前缀宽度把点击 x（相对文本左缘）映射为最近的光标 char 索引。
///
/// `width_of(k)` = 前 `k` 个字符的总宽度（单调不减）。二分找第一个
/// `width_of(k) >= cx` 的 k，再与 `k-1` 比较取更近者（纯函数，可单测）。
fn caret_index_by_width(n: usize, cx: f32, mut width_of: impl FnMut(usize) -> f32) -> usize {
    if n == 0 {
        return 0;
    }
    let mut lo = 1usize;
    let mut hi = n;
    let mut k = n; // 默认：点击在文本末尾之后 → 光标在末尾
    while lo <= hi {
        let mid = (lo + hi) / 2;
        if width_of(mid) >= cx {
            k = mid;
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
    }
    let w_k = width_of(k);
    let w_prev = if k > 0 { width_of(k - 1) } else { 0.0 };
    if (w_k - cx).abs() < (cx - w_prev).abs() {
        k
    } else {
        k - 1
    }
}

// ─── 容器包装（Panel / Pack / Grid / Window 共享同一控件 API） ──

macro_rules! widget_api {
    ($T:ident) => {
        impl<'ui, 'a> $T<'ui, 'a> {
            /// 标签（占光标，内容自然尺寸）。
            pub fn label(&mut self, text: &str) -> Vec2 {
                let elem = self.ui.seq + 1;
                let style = self.ui.theme.label.clone();
                let size =
                    self.ui.text_size(text, style.font_size, style.font_family.as_deref());
                let rect = self.ui.child_rect(size.x, size.y);
                let seq = self.ui.next_seq();
                let depth = self.ui.depth;
                self.ui.queue.push(text_cmd(
                    depth,
                    seq,
                    self.ui.cur_win,
                    elem,
                    rect,
                    text.to_owned(),
                    style.font_size,
                    style.color,
                    TextAlign::from(style.align),
                    TextVAlign::Center,
                    style.font_family.clone(),
                    None,
                    self.ui.clip,
                None,
                ));
                size
            }

            /// 绝对定位标签（`pos` 相对当前容器内容原点）。
            pub fn label_at(&mut self, pos: Vec2, text: &str) -> Vec2 {
                self.ui.label_at(pos, text)
            }

            /// **自动换行标签**（占光标）：`max_w` 逻辑像素内按词/字换行；
            /// 返回自然尺寸（宽 = min(自然宽, max_w)，高 = 行数 × 行高）。
            /// `max_w <= 0` = 不换行（同 `label`）。
            pub fn label_wrap(&mut self, max_w: f32, text: &str) -> Vec2 {
                let style = self.ui.theme.label.clone();
                let size = self
                    .ui
                    .text_size_wrap(text, style.font_size, style.font_family.as_deref(), max_w);
                let rect = self.ui.child_rect(size.x, size.y);
                let elem = self.ui.seq + 1;
                let seq = self.ui.next_seq();
                let depth = self.ui.depth;
                self.ui.queue.push(text_cmd(
                    depth,
                    seq,
                    self.ui.cur_win,
                    elem,
                    rect,
                    text.to_owned(),
                    style.font_size,
                    style.color,
                    TextAlign::from(style.align),
                    TextVAlign::Center,
                    style.font_family.clone(),
                    None,
                    self.ui.clip,
                None,
                ));
                size
            }

            /// **多行文本输入框**（占光标；默认约 200×90，可 `text_area_at` 显式尺寸）。
            /// Enter 换行、↑/↓ 跨行、Home/End 行首尾；自动换行 + 垂直滚动；选择/复制/
            /// 粘贴/剪切（Ctrl+C/V/X）；IME 支持。返回 `()`（内容写回 `value`）。
            pub fn text_area(&mut self, id: &str, value: &mut String) {
                let style = self.ui.theme.input.clone();
                let w = style.min_w.max(200.0);
                let rect = self.ui.child_rect(w, 90.0);
                self.ui.text_area_at(id, rect, value);
            }

            /// **多行文本输入框**（显式 `Rect`）。
            pub fn text_area_at(&mut self, id: &str, rect: Rect, value: &mut String) {
                self.ui.text_area_at(id, rect, value);
            }

            /// **下一子项的最小尺寸约束**（`0` = 该轴不约束；一次性，作用于紧接着的下一个子项）。
            pub fn min_size(&mut self, w: f32, h: f32) {
                self.ui.set_next_min(glam::Vec2::new(w, h));
            }

            /// **下一子项的最大尺寸约束**（`0` = 该轴不约束；一次性，作用于紧接着的下一个子项）。
            pub fn max_size(&mut self, w: f32, h: f32) {
                self.ui.set_next_max(glam::Vec2::new(w, h));
            }

            /// **下拉框**（占光标，自动尺寸）：按钮 + 展开选项浮层；返回本帧新选中索引。
            pub fn combo(
                &mut self,
                id: &str,
                current: &str,
                options: &[String],
                selected: Option<u32>,
            ) -> Option<u32> {
                let style = self.ui.theme.button.clone();
                let tsize =
                    self.ui.text_size(current, style.font_size, style.font_family.as_deref());
                let w = (tsize.x + 20.0).max(90.0) + style.padding.x * 2.0;
                let h = style.padding.y * 2.0 + tsize.y;
                let rect = self.ui.child_rect(w, h);
                self.ui.combo_at(id, rect, current, options, selected)
            }

            /// 按钮（文本 + padding 自动尺寸）。
            pub fn button(&mut self, id: &str, label: &str) -> ButtonState {
                let style = self.ui.theme.button.clone();
                let tsize =
                    self.ui.text_size(label, style.font_size, style.font_family.as_deref());
                let size = Vec2::new(
                    tsize.x + style.padding.x * 2.0,
                    tsize.y + style.padding.y * 2.0,
                );
                let rect = self.ui.child_rect(size.x, size.y);
                self.ui.button_at(id, rect, label)
            }

            /// 显式尺寸按钮（逃生舱）。
            pub fn button_at(&mut self, id: &str, rect: Rect, label: &str) -> ButtonState {
                self.ui.button_at(id, rect, label)
            }

            /// 滑块（自动尺寸：高度固定，宽度取样式最小宽）。
            pub fn slider(&mut self, id: &str, range: RangeInclusive<f32>, value: f32) -> f32 {
                let style = self.ui.theme.slider.clone();
                let size = Vec2::new(style.min_w.max(40.0), style.height);
                let rect = self.ui.child_rect(size.x, size.y);
                self.ui.slider_at(id, rect, range, value)
            }

            /// 显式尺寸滑块（逃生舱）。
            pub fn slider_at(
                &mut self,
                id: &str,
                rect: Rect,
                range: RangeInclusive<f32>,
                value: f32,
            ) -> f32 {
                self.ui.slider_at(id, rect, range, value)
            }

            /// 勾选框（勾选值由用户维护，返回含 `toggled` 的状态）。
            pub fn checkbox(&mut self, id: &str, label: &str, checked: bool) -> CheckboxState {
                let style = self.ui.theme.checkbox.clone();
                let tsize =
                    self.ui.text_size(label, style.font_size, style.font_family.as_deref());
                let size = Vec2::new(
                    style.box_size + style.gap + tsize.x,
                    style.box_size.max(tsize.y),
                );
                let rect = self.ui.child_rect(size.x, size.y);
                self.ui.checkbox_at(id, rect, label, checked)
            }

            /// 显式尺寸勾选框（逃生舱）。
            pub fn checkbox_at(
                &mut self,
                id: &str,
                rect: Rect,
                label: &str,
                checked: bool,
            ) -> CheckboxState {
                self.ui.checkbox_at(id, rect, label, checked)
            }

            /// 单选（同组 ID 互斥；返回 `checked` / `toggled`）。
            pub fn radio(&mut self, id: &str, group: &str, label: &str) -> CheckboxState {
                let style = self.ui.theme.checkbox.clone();
                let tsize =
                    self.ui.text_size(label, style.font_size, style.font_family.as_deref());
                let size = Vec2::new(
                    style.box_size + style.gap + tsize.x,
                    style.box_size.max(tsize.y),
                );
                let rect = self.ui.child_rect(size.x, size.y);
                self.ui.radio_at(id, group, rect, label)
            }

            /// 显式尺寸单选（逃生舱）。
            pub fn radio_at(
                &mut self,
                id: &str,
                group: &str,
                rect: Rect,
                label: &str,
            ) -> CheckboxState {
                self.ui.radio_at(id, group, rect, label)
            }

            /// 文本输入框（内容写入 `value`；自动尺寸：高度固定，宽度取样式最小宽）。
            pub fn text_input(&mut self, id: &str, value: &mut String) {
                let style = self.ui.theme.input.clone();
                let size = Vec2::new(style.min_w, style.height);
                let rect = self.ui.child_rect(size.x, size.y);
                self.ui.text_input_at(id, rect, value);
            }

            /// 显式尺寸文本输入框（逃生舱）。
            pub fn text_input_at(&mut self, id: &str, rect: Rect, value: &mut String) {
                self.ui.text_input_at(id, rect, value);
            }

            /// 嵌套面板（`pos` 相对当前容器内容原点；不占光标）。
            pub fn panel_at(&mut self, pos: Vec2, f: impl FnOnce(&mut Panel<'_, '_>)) -> Vec2 {
                self.ui.panel_at(pos, f)
            }

            /// 嵌套**可拖拽**面板（位置持久于 `UiState.panel_pos`）。
            pub fn drag_panel_at(
                &mut self,
                id: &str,
                pos: Vec2,
                f: impl FnOnce(&mut Panel<'_, '_>),
            ) -> Vec2 {
                self.ui.drag_panel_at(id, pos, f)
            }

            /// 嵌套**窗口**（可重叠 + 焦点置顶 + 可拖拽）。
            pub fn window_at(
                &mut self,
                id: &str,
                pos: Vec2,
                f: impl FnOnce(&mut Window<'_, '_>),
            ) -> Vec2 {
                self.ui.window_at(id, pos, f)
            }

            /// 嵌套 pack（`pos` 相对当前容器内容原点；不占光标）。
            pub fn pack_at(
                &mut self,
                pos: Vec2,
                side: PackSide,
                f: impl FnOnce(&mut Pack<'_, '_>),
            ) -> Vec2 {
                self.ui.pack_at(pos, side, f)
            }

            /// 嵌套 grid（`pos` 相对当前容器内容原点；不占光标）。
            pub fn grid_at(
                &mut self,
                pos: Vec2,
                cols: usize,
                id: &str,
                f: impl FnOnce(&mut Grid<'_, '_>),
            ) -> Vec2 {
                self.ui.grid_at(pos, cols, id, f)
            }
        }
    };
}

/// 容器闭包上下文（内部类型）。
pub(crate) struct ContainerCtx<'ui, 'a> {
    pub(crate) ui: &'ui mut Ui<'a>,
}

/// 面板容器（背景 + 边框 + 垂直堆叠内容）。
pub struct Panel<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
widget_api!(Panel);

/// pack 容器（无背景，纯布局）。
pub struct Pack<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
widget_api!(Pack);

/// grid 容器（无背景，均匀网格）。
pub struct Grid<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
widget_api!(Grid);

/// 窗口容器（可重叠 + 焦点置顶 + 可拖拽；见 [`Ui::window_at`]）。
pub struct Window<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
widget_api!(Window);

/// 滚动容器（内容在可视区内堆叠 + 滚动；见 [`Ui::scroll_at`]）。
pub struct Scroll<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
widget_api!(Scroll);

/// **flex 容器上下文**（[`Ui::flex_at`]）：子项高度已按权重分配（强制），
/// 内部可调用任意控件方法占光标（`f.label` / `f.button` 等）。
pub struct FlexCtx<'ui, 'a> {
    ui: &'ui mut Ui<'a>,
}
widget_api!(FlexCtx);

// ─── 控件实现（Ui 内部方法） ────────────────────────────────────

impl Ui<'_> {
    /// **下拉框**（显式 rect；`rect` 为相对当前容器 origin 的局部坐标）。
    ///
    /// 按钮显示 `current`；点击展开**选项浮层**（临时窗口置顶，自动尺寸包裹选项），
    /// 点击选项选中并收起，点击浮层外收起。`selected` 为当前选中（用于 ✓ 标记）。
    /// 返回本帧新选中的索引（`None` = 无选择/未展开）。
    pub(crate) fn combo_at(
        &mut self,
        id: &str,
        rect: Rect,
        current: &str,
        options: &[String],
        selected: Option<u32>,
    ) -> Option<u32> {
        let mut picked = None;
        let open = self.state.combo_open.as_deref() == Some(id);
        // 登记焦点链（键盘导航：Tab 可到；Enter/Space 展开收起；方向键切换选项）。
        self.register_focus(id, rect, FocusKind::Combo);
        // 按钮交互（点击 toggle）。
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        let key_click = self.key_click(id, FocusKind::Combo);
        let mut ev = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let ev = update_interact(ws, hit, btn);
            if key_click {
                ws.pressed = true;
            }
            ev
        };
        if key_click {
            ev.clicked = true;
        }
        if ev.pressed {
            self.any_pressed = true;
        }
        if ev.clicked {
            self.state.combo_open = if open { None } else { Some(id.to_owned()) };
        }
        // 键盘：焦点下展开时，上下方向键切换选项（选中即关闭浮层）；Esc 收起。
        if open {
            if self.focused_is(id) {
                let n = options.len() as u32;
                if n > 0 {
                    let cur = selected.unwrap_or(0).min(n - 1);
                    if self.keyboard.get(KeyCode::ArrowUp).down_edge() {
                        picked = Some(if cur == 0 { n - 1 } else { cur - 1 });
                    }
                    if self.keyboard.get(KeyCode::ArrowDown).down_edge() {
                        picked = Some(if cur + 1 >= n { 0 } else { cur + 1 });
                    }
                }
            }
            if self.keyboard.get(KeyCode::Escape).down_edge() {
                self.state.combo_open = None;
            }
        }
        // 按钮绘制（展开时用 pressed 态背景）。
        let style = self.theme.button.clone();
        let elem = self.seq + 1;
        let bg = if open { style.bg_pressed } else { style.bg };
        self.push_panel_like(rect, bg, style.border, style.border_w, style.radius, elem);
        let text_rect = Rect::new(rect.x, rect.y, (rect.w - 18.0).max(0.0), rect.h);
        let seq = self.next_seq();
        self.queue.push(text_cmd(
            self.depth,
            seq,
            self.cur_win,
            elem,
            text_rect,
            current.to_owned(),
            style.font_size,
            style.fg,
            TextAlign::Left,
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.clip,
        None,
        ));
        let arrow_rect = Rect::new(rect.x + rect.w - 18.0, rect.y, 18.0, rect.h);
        let seq = self.next_seq();
        self.queue.push(text_cmd(
            self.depth,
            seq,
            self.cur_win,
            elem,
            arrow_rect,
            "▼".to_owned(),
            style.font_size,
            style.fg,
            TextAlign::Center,
            TextVAlign::Center,
            None,
            None,
            self.clip,
        None,
        ));
        // 展开的选项浮层：临时窗口（z 最高 → 覆盖一切），自动尺寸包裹选项。
        if open {
            let popup_pos = Vec2::new(rect.x, rect.y + rect.h + 2.0);
            let popup_id = format!("{id}::popup");
            let popup_size = self.window_at(&popup_id, popup_pos, |w| {
                for (i, opt) in options.iter().enumerate() {
                    let sel = selected == Some(i as u32);
                    let label = if sel { format!("✓ {opt}") } else { opt.clone() };
                    if w.button(&format!("{id}::opt_{i}"), &label).clicked() {
                        picked = Some(i as u32);
                    }
                }
            });
            // 点击浮层外（且不在按钮上）→ 收起。
            // ⚠ popup_pos / rect 是**相对当前容器**的局部坐标，必须转**绝对**再与
            // 绝对鼠标坐标比较——否则容器有偏移时（如 pack_at(16,90)）判定错位，
            // 点选项会被误判为"点外部"导致浮层收起且不选中。
            let popup_abs = Rect::new(
                self.abs_base.x + popup_pos.x,
                self.abs_base.y + popup_pos.y,
                popup_size.x,
                popup_size.y,
            );
            let btn_abs = Rect::new(
                self.abs_base.x + rect.x,
                self.abs_base.y + rect.y,
                rect.w,
                rect.h,
            );
            if btn.down_edge()
                && !hit_test(&popup_abs, self.mouse_logical)
                && !hit_test(&btn_abs, self.mouse_logical)
            {
                self.state.combo_open = None;
            }
        }
        if picked.is_some() {
            self.state.combo_open = None;
        }
        picked
    }

    /// **下拉框**（顶层定位：`pos` 相对当前容器内容原点，绝对定位；尺寸自动）。
    pub fn combo(
        &mut self,
        id: &str,
        pos: Vec2,
        current: &str,
        options: &[String],
        selected: Option<u32>,
    ) -> Option<u32> {
        let style = self.theme.button.clone();
        let tsize = self.text_size(current, style.font_size, style.font_family.as_deref());
        let w = (tsize.x + 20.0).max(90.0) + style.padding.x * 2.0;
        let h = style.padding.y * 2.0 + tsize.y;
        let rect = Rect::new(pos.x, pos.y, w, h);
        self.combo_at(id, rect, current, options, selected)
    }

    /// 按钮（显式 rect；`rect` 为相对当前容器 origin 的局部坐标）。
    pub(crate) fn button_at(&mut self, id: &str, rect: Rect, label: &str) -> ButtonState {
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        // 键盘激活（Enter/Space + 焦点）→ 视为点击；先取出（不借用 self）
        let key_click = self.key_click(id, FocusKind::Button);
        if key_click {
            self.any_pressed = true;
        }
        {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let mut ev = update_interact(ws, hit, btn);
            if key_click {
                // 焦点键盘点击：合成 pressed + clicked（触发本帧回调）。
                ws.pressed = true;
                ev.clicked = true;
            }
            if ev.pressed {
                self.any_pressed = true;
            }
            let (pressed, hovered) = (ws.pressed, ws.hovered);
            // 记录绘制（ws 借用已结束）
            let style = self.theme.button.clone();
            let bg = if pressed {
                style.bg_pressed
            } else if hovered {
                style.bg_hover
            } else {
                style.bg
            };
            let depth = self.depth;
            let win = self.cur_win;
            let elem = self.seq + 1;
            // 背景 + 边框（radius > 0 走圆角双层矩形）。
            self.push_panel_like(rect, bg, style.border, style.border_w, style.radius, elem);
            let text_seq = self.next_seq();
            self.queue.push(text_cmd(
                depth,
                text_seq,
                win,
                elem,
                rect,
                label.to_owned(),
                style.font_size,
                style.fg,
                TextAlign::Center,
                TextVAlign::Center,
                style.font_family.clone(),
                None,
                self.clip,
            None,
            ));
            ButtonState {
                hovered,
                pressed,
                clicked: ev.clicked,
                released: ev.released,
            }
        }
    }

    /// 滑块（显式 rect）。
    pub(crate) fn slider_at(
        &mut self,
        id: &str,
        rect: Rect,
        range: RangeInclusive<f32>,
        value: f32,
    ) -> f32 {
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        let active = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            update_drag(ws, hit, btn)
        };
        if active {
            self.any_pressed = true;
        }
        let (lo, hi) = (*range.start(), *range.end());
        let span = hi - lo;
        let mut new_value = value;
        if active && span.abs() > f32::EPSILON {
            let t = normalize_x(&rect, self.mouse_local_x());
            new_value = lo + t * span;
        }
        // 键盘：焦点下滑块用左右方向键调值（步进 = 范围的 5%，即时生效）。
        if self.focused_is(id) && span.abs() > f32::EPSILON {
            let step = span * 0.05;
            if self.keyboard.get(KeyCode::ArrowLeft).down_edge() {
                new_value = (new_value - step).clamp(lo, hi);
            }
            if self.keyboard.get(KeyCode::ArrowRight).down_edge() {
                new_value = (new_value + step).clamp(lo, hi);
            }
        }
        let t = if span.abs() > f32::EPSILON {
            ((new_value - lo) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let style = self.theme.slider.clone();
        let depth = self.depth;
        let win = self.cur_win;
        let elem = self.seq + 1;
        let track_rect =
            Rect::new(rect.x, rect.y + (rect.h - style.track_h) * 0.5, rect.w, style.track_h);
        let fill_rect = Rect::new(rect.x, track_rect.y, rect.w * t, style.track_h);
        let handle_cx = rect.x + rect.w * t;
        let handle_rect = Rect::new(
            handle_cx - style.handle_w * 0.5,
            rect.y + (rect.h - style.handle_w) * 0.5,
            style.handle_w,
            style.handle_w,
        );
        let push_solid = |ui: &mut Ui<'_>, r: Rect, c: Color| {
            if r.w > 0.0 && r.h > 0.0 {
                let seq = ui.next_seq();
                ui.queue.push(UiDraw {
                    depth,
                    seq,
                    win,
                    elem,
                    rect: r,
                    clip: ui.clip,
                    kind: DrawKind::Solid(c),
                });
            }
        };
        push_solid(self, track_rect, style.track);
        push_solid(self, fill_rect, style.fill);
        push_solid(self, handle_rect, style.handle);
        if style.handle_w > 2.0 {
            let seq = self.next_seq();
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: handle_rect,
                clip: self.clip,
                kind: DrawKind::Border {
                    color: style.handle_border,
                    width: 1.0,
                },
            });
        }
        new_value
    }

    /// 勾选框（显式 rect）。
    pub(crate) fn checkbox_at(
        &mut self,
        id: &str,
        rect: Rect,
        label: &str,
        checked: bool,
    ) -> CheckboxState {
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        let key_click = self.key_click(id, FocusKind::Checkbox);
        if key_click {
            self.any_pressed = true;
        }
        let mut ev = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let ev = update_interact(ws, hit, btn);
            if key_click {
                ws.pressed = true;
            }
            ev
        };
        if key_click {
            ev.clicked = true;
        }
        if ev.pressed {
            self.any_pressed = true;
        }
        self.draw_check_common(rect, label, checked);
        CheckboxState {
            checked,
            toggled: ev.clicked,
            clicked: ev.clicked,
        }
    }

    /// 单选（显式 rect）。
    pub(crate) fn radio_at(
        &mut self,
        id: &str,
        group: &str,
        rect: Rect,
        label: &str,
    ) -> CheckboxState {
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        let key_click = self.key_click(id, FocusKind::Radio);
        if key_click {
            self.any_pressed = true;
        }
        let mut ev = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let ev = update_interact(ws, hit, btn);
            if key_click {
                ws.pressed = true;
            }
            ev
        };
        if key_click {
            ev.clicked = true;
        }
        if ev.pressed {
            self.any_pressed = true;
        }
        let was_checked = self
            .state
            .radio_groups
            .get(group)
            .map(|s| s == id)
            .unwrap_or(false);
        if ev.clicked {
            self.state.radio_groups.insert(group.to_owned(), id.to_owned());
        }
        let checked = self
            .state
            .radio_groups
            .get(group)
            .map(|s| s == id)
            .unwrap_or(false);
        self.draw_check_common(rect, label, checked);
        CheckboxState {
            checked,
            toggled: ev.clicked && !was_checked,
            clicked: ev.clicked,
        }
    }

    /// 勾选框 / 单选公共绘制：方框 +（选中时）填充 + 标签文本。
    fn draw_check_common(&mut self, rect: Rect, label: &str, checked: bool) {
        let style = self.theme.checkbox.clone();
        let depth = self.depth;
        let win = self.cur_win;
        let elem = self.seq + 1;
        let box_rect = Rect::new(
            rect.x,
            rect.y + (rect.h - style.box_size) * 0.5,
            style.box_size,
            style.box_size,
        );
        let seq = self.next_seq();
        self.queue.push(UiDraw {
            depth,
            seq,
            win,
            elem,
            rect: box_rect,
            clip: self.clip,
            kind: DrawKind::Border {
                color: style.box_border,
                width: 1.0,
            },
        });
        if checked {
            let inner = Rect::new(
                box_rect.x + 3.0,
                box_rect.y + 3.0,
                box_rect.w - 6.0,
                box_rect.h - 6.0,
            );
            if inner.w > 0.0 && inner.h > 0.0 {
                let seq = self.next_seq();
                self.queue.push(UiDraw {
                    depth,
                    seq,
                    win,
                    elem,
                    rect: inner,
                    clip: self.clip,
                    kind: DrawKind::Solid(style.checked_fill),
                });
            }
        }
        let text_rect = Rect::new(
            box_rect.x + style.box_size + style.gap,
            rect.y,
            (rect.w - style.box_size - style.gap).max(0.0),
            rect.h,
        );
        let seq = self.next_seq();
        self.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            text_rect,
            label.to_owned(),
            style.font_size,
            style.fg,
            TextAlign::Left,
            TextVAlign::Center,
            style.font_family.clone(),
            None,
            self.clip,
        None,
        ));
    }

    /// 文本输入框（显式 rect，**单行**）。
    ///
    /// 增强能力：
    /// - **超长文本滚动跟随光标**：文本超出内容区时左移，光标始终可见（`WidgetState::text_scroll`）；
    /// - **文本选择**：按住拖拽选择（`WidgetState::sel_anchor`），选择优先于窗口/面板拖拽
    ///   （按下时置位 `press_claimed`）；Ctrl+C/V/X 复制/粘贴/剪切；选择后打字/退格替换选择；
    /// - **IME 组合候选移入浮动提示框**：组合串（preedit）画在输入框下方浮动小框中（不再占行内）。
    pub(crate) fn text_input_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        // 登记焦点链（Tab/方向键可遍历到输入框）。
        self.register_focus(id, rect, FocusKind::TextInput);
        let mouse_local_x = self.mouse_local_x();
        // 提前测量（避免在 ws 借用期间调用 &mut self 方法）
        let input_style = self.theme.input.clone();
        // 鼠标按下时的光标位置（点击定位 / 拖拽选择共用）：按字符**实际宽度**
        // （前缀测量，二分）——混合中英文（字宽不同）时精确落在最近的字符边界。
        // `btn.pressed()` 而非 `hit`：**拖出输入框后仍跟随**（cx 越界 clamp 到两端，
        // 支持"拖到看不见的地方继续选中"——滚动随光标自动跟随）。
        let mouse_caret = if btn.pressed() {
            let cx = (mouse_local_x - rect.x - input_style.padding_x).max(0.0);
            Some(self.caret_index_at_width(
                value,
                input_style.font_size,
                input_style.font_family.as_deref(),
                cx,
            ))
        } else {
            None
        };
        let caret_est = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let ev = update_interact(ws, hit, btn);
            if ev.pressed {
                self.any_pressed = true;
                // 输入框按下占用该次按压：从输入框拖拽 = 选择文本（窗口/面板不建立拖拽基准）
                self.press_claimed = true;
                self.state.focused = Some(id.to_owned());
                if let Some(c) = mouse_caret {
                    ws.caret = c;
                }
                ws.sel_anchor = Some(ws.caret);
            } else if ws.pressed && btn.pressed() {
                // 拖拽选择：按住并移动 → 光标跟随鼠标（**即使拖出输入框**——超出部分
                // clamp 到文本两端；水平滚动随光标自动跟随），选择范围 = [anchor, caret)。
                self.press_claimed = true;
                if let Some(c) = mouse_caret {
                    ws.caret = c;
                }
            }
            if ev.released && ws.sel_anchor == Some(ws.caret) {
                // 纯点击（无位移）：anchor == caret，无实际选择 → 清理，避免残留
                // anchor 在后续无 Shift 方向键移动时"突然变成多选"。
                ws.sel_anchor = None;
            }
            let focused = self.state.focused.as_deref() == Some(id);
            if focused {
                // IME 组合中（preedit 非空）**或刚结束的帧**（上一帧在组合）：
                // 退格/删除/方向键由 **IME 系统**处理（缩短组合串、结束组合、移动
                // 组合光标）——本地处理会误删已有文本（组合结束帧 Preedit("") 先清空
                // 候选、随后退格键到达，只看当前帧会误判为非组合而误删）。
                let in_ime_compose =
                    self.keyboard.get_ime_preedit().is_some_and(|p| !p.is_empty());
                let ime_owns_keys = in_ime_compose || self.state.ime_composing;
                // 剪贴板：Ctrl+C / Ctrl+V / Ctrl+X（选择复制粘贴剪切）。
                let ctrl = self.keyboard.get(KeyCode::ControlLeft).pressed()
                    || self.keyboard.get(KeyCode::ControlRight).pressed();
                if ctrl {
                    let c_down = self.keyboard.get(KeyCode::KeyC).down_edge();
                    let v_down = self.keyboard.get(KeyCode::KeyV).down_edge();
                    let x_down = self.keyboard.get(KeyCode::KeyX).down_edge();
                    if c_down || x_down {
                        let sel = selected_text(value, ws.sel_anchor, ws.caret);
                        if !sel.is_empty() {
                            clipboard_set(&sel);
                            if x_down {
                                if let Some((lo, _hi)) = sel_range(ws.sel_anchor, ws.caret) {
                                    delete_range(value, lo, ws.caret.max(lo));
                                    ws.caret = lo;
                                }
                                ws.sel_anchor = None;
                            }
                        }
                    }
                    if v_down {
                        if let Some(text) = clipboard_get() {
                            if !text.is_empty() {
                                // 粘贴替换选择
                                let lo = match sel_range(ws.sel_anchor, ws.caret) {
                                    Some((lo, hi)) => {
                                        delete_range(value, lo, hi);
                                        lo
                                    }
                                    None => ws.caret,
                                };
                                insert_str_at(value, lo, &text);
                                ws.caret = lo + text.chars().count();
                                ws.sel_anchor = None;
                            }
                        }
                    }
                }
                // 编辑操作（字符 / IME 上屏 / 退格 / 删除）前若存在选择 → 先删除选择
                // ⚠ Ctrl 组合（C/V/X）按下时 `get_chars` 会带出 'c'/'v'/'x'——
                // 剪贴板分支已处理，字符必须过滤（否则 Ctrl+C 留下 'c'、Ctrl+V 多出 'v'）。
                let edit_pending = (!self.keyboard.get_chars().is_empty() && !ctrl)
                    || !self.keyboard.get_ime_commits().is_empty()
                    || (self.keyboard.get(KeyCode::Backspace).down_edge() && !ime_owns_keys)
                    || (self.keyboard.get(KeyCode::Delete).down_edge() && !ime_owns_keys);
                if edit_pending {
                    if let Some((lo, hi)) = sel_range(ws.sel_anchor, ws.caret) {
                        delete_range(value, lo, hi);
                        ws.caret = lo;
                        ws.sel_anchor = None;
                    }
                }
                // IME 上屏文本（中文输入法等）：优先级高于普通字符
                for commit in self.keyboard.get_ime_commits() {
                    insert_str_at(value, ws.caret, commit);
                    ws.caret = (ws.caret + commit.chars().count()).min(value.chars().count());
                }
                // 普通字符输入 / 编辑（Ctrl 组合不产生文本）
                if !ctrl {
                    for ch in self.keyboard.get_chars() {
                        insert_char_at(value, ws.caret, *ch);
                        ws.caret = (ws.caret + 1).min(value.chars().count());
                    }
                }
                if self.keyboard.get(KeyCode::Backspace).down_edge() && !ime_owns_keys {
                    ws.caret = remove_before(value, ws.caret);
                }
                if self.keyboard.get(KeyCode::Delete).down_edge() && !ime_owns_keys {
                    remove_at(value, ws.caret);
                }
                // Shift + 方向键：扩展/收缩选择（anchor 不动；光标越过 anchor 时收缩归零）
                let shift = self.keyboard.get(KeyCode::ShiftLeft).pressed()
                    || self.keyboard.get(KeyCode::ShiftRight).pressed();
                let shift_start = |ws: &mut WidgetState| {
                    if shift && ws.sel_anchor.is_none() {
                        ws.sel_anchor = Some(ws.caret);
                    }
                };
                let shift_shrink = |ws: &mut WidgetState| {
                    if ws.sel_anchor == Some(ws.caret) {
                        ws.sel_anchor = None;
                    }
                };
                if self.keyboard.get(KeyCode::ArrowLeft).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    ws.caret = ws.caret.saturating_sub(1);
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::ArrowRight).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    ws.caret = (ws.caret + 1).min(value.chars().count());
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::Enter).down_edge() {
                    self.state.focused = None;
                }
                // Esc：取消输入焦点（不再把 Esc 传给应用层快捷键）
                if self.keyboard.get(KeyCode::Escape).down_edge() {
                    self.state.focused = None;
                }
            }
            (focused, ws.caret)
        };
        let (focused, caret) = caret_est;
        // 绘制
        let style = self.theme.input.clone();
        let depth = self.depth;
        let win = self.cur_win;
        let elem = self.seq + 1;
        let border = if focused { style.border_focus } else { style.border };
        // 背景 + 边框（radius > 0 走圆角双层矩形）。
        self.push_panel_like(rect, style.bg, border, style.border_w, style.radius, elem);
        let content_w = (rect.w - style.padding_x * 2.0).max(0.0);
        let content_rect = Rect::new(rect.x + style.padding_x, rect.y, content_w, rect.h);
        // 文本自然宽（水平滚动上限）与光标 x（前缀宽度）——都基于未滚动文本。
        let text_w = self.text_size(value, style.font_size, style.font_family.as_deref()).x;
        let caret_x = {
            let prefix: String = value.chars().take(caret).collect();
            self.text_size(&prefix, style.font_size, style.font_family.as_deref()).x
        };
        // 水平滚动跟随光标（跨帧状态；光标右侧保留 8 逻辑像素）。
        let scroll = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            ws.text_scroll = scroll_follow_caret(caret_x, content_w, text_w, 8.0);
            ws.text_scroll
        };
        let text_dx = -scroll;
        // 文本选择高亮（在文本之下绘制：同一 elem 的图形组先于文字组）。
        if let Some((lo, hi)) = sel_range(
            self.state.widgets.get(id).and_then(|w| w.sel_anchor),
            caret,
        ) {
            let lo_x = {
                let p: String = value.chars().take(lo).collect();
                self.text_size(&p, style.font_size, style.font_family.as_deref()).x
            };
            let hi_x = {
                let p: String = value.chars().take(hi).collect();
                self.text_size(&p, style.font_size, style.font_family.as_deref()).x
            };
            let sel_rect = Rect::new(
                content_rect.x + lo_x + text_dx,
                content_rect.y + 1.0,
                (hi_x - lo_x).max(0.0),
                (content_rect.h - 2.0).max(0.0),
            );
            if sel_rect.w > 0.0 && sel_rect.h > 0.0 {
                let seq = self.next_seq();
                self.queue.push(UiDraw {
                    depth,
                    seq,
                    win,
                    elem,
                    rect: sel_rect,
                    // 选择高亮受内容区裁剪（不溢出输入框 / 滚动容器）。
                    clip: Some(content_rect),
                    kind: DrawKind::Solid(style.sel_bg),
                });
            }
        }
        // 文本（左移 scroll；**裁剪窗口固定在内容区**：clip 相对移动后的 rect 起点 = scroll，
        // 绝对位置 = content_rect.x —— 若 clip.x=0 会随 rect 一起左移，始终显示文本开头
        // 且偏离文本框；缓冲控件自持）。
        let clip = Rect::new(scroll, 0.0, content_w, rect.h);
        let buf = self.ensure_text_buf(
            id,
            value,
            style.font_size,
            style.font_family.as_deref(),
            0.0,
            1.0,
        );
        let seq = self.next_seq();
        self.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            Rect::new(content_rect.x + text_dx, content_rect.y, content_w, rect.h),
            value.clone(),
            style.font_size,
            style.fg,
            TextAlign::Left,
            TextVAlign::Center,
            style.font_family.clone(),
            Some(clip),
            self.clip,
            Some(buf),
        ));
        // IME 组合候选 → **浮动提示框**（输入框下方小框，不再占行内）：
        // 面板底色 + 边框 + preedit 文本，自动宽度；组合中实时更新。
        if focused {
            if let Some(preedit) = self.keyboard.get_ime_preedit() {
                if !preedit.is_empty() {
                    let psize =
                        self.text_size(preedit, style.font_size, style.font_family.as_deref());
                    let box_pad = 6.0;
                    let box_w = (psize.x + box_pad * 2.0).max(20.0);
                    let box_h = (psize.y + 4.0).max(rect.h);
                    let box_rect = Rect::new(rect.x, rect.y + rect.h + 4.0, box_w, box_h);
                    let box_elem = self.seq + 1;
                    let bseq = self.next_seq();
                    self.queue.push(UiDraw {
                        depth,
                        seq: bseq,
                        win,
                        elem: box_elem,
                        rect: box_rect,
                        clip: self.clip,
                        kind: DrawKind::Solid(style.bg),
                    });
                    self.queue.push(UiDraw {
                        depth,
                        seq: bseq + 1,
                        win,
                        elem: box_elem,
                        rect: box_rect,
                        clip: self.clip,
                        kind: DrawKind::Border {
                            color: style.border_focus,
                            width: style.border_w,
                        },
                    });
                    let seq = self.next_seq();
                    self.queue.push(text_cmd(
                        depth,
                        seq,
                        win,
                        box_elem,
                        Rect::new(
                            box_rect.x + box_pad,
                            box_rect.y,
                            box_w - box_pad * 2.0,
                            box_h,
                        ),
                        preedit.to_owned(),
                        style.font_size,
                        style.preedit,
                        TextAlign::Left,
                        TextVAlign::Center,
                        style.font_family.clone(),
                        None,
                        self.clip,
                    None,
                    ));
                }
            }
            // **IME 候选框定位**：跟随输入框光标（窗口客户区物理像素；含水平滚动）。
            let ime_x =
                ((self.abs_base.x + content_rect.x + caret_x + text_dx) * self.scale) as i32;
            let ime_y = ((self.abs_base.y + rect.y) * self.scale) as i32;
            let ime_w = (rect.w * self.scale).max(1.0) as u32;
            let ime_h = (rect.h * self.scale).max(1.0) as u32;
            let _ = self.window.set_ime_cursor_area(
                PhysicalPosition::new(ime_x, ime_y),
                PhysicalSize::new(ime_w, ime_h),
            );
        }
        // 光标（跟随水平滚动）
        if focused && self.state.caret_blink_on() {
            let caret_rect = Rect::new(
                content_rect.x + caret_x + text_dx,
                content_rect.y + 2.0,
                1.0,
                (content_rect.h - 4.0).max(1.0),
            );
            let seq = self.next_seq();
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: caret_rect,
                clip: self.clip,
                kind: DrawKind::Caret {
                    color: style.caret,
                    width: 1.0,
                },
            });
        }
    }

    /// 多行文本输入框（显式 rect，**TextArea**）。
    ///
    /// - **编辑**：Enter 换行、↑/↓ 跨**视觉行**（保持列）、Home/End 行首/行尾、
    ///   ←/→ 字符移动、Backspace/Delete、选择替换；Esc 失焦；
    /// - **渲染**：文本按内容区宽度**自动换行**（[`rjw_text::Text::create_buffer_wrap`]）；
    ///   超出高度**垂直滚动**（滚轮 + 光标跟随，`WidgetState::scroll_y`）；
    /// - **光标 / 点击 / 选择按"视觉行"（自动换行后）定位**——与显示完全一致
    ///   （[`rjw_text::Text::visual_lines`]：每个 `LayoutRun` 一行，含字节范围）；
    /// - **选择 / 复制 / 粘贴 / 剪切**（Ctrl+C/V/X）跨视觉行，高亮逐行绘制。
    /// - **IME**：组合候选浮动提示框 + 候选框定位到光标。
    pub(crate) fn text_area_at(&mut self, id: &str, rect: Rect, value: &mut String) {
        let hit = self.hit_abs(&rect);
        let btn = self.mouse_left();
        self.register_focus(id, rect, FocusKind::TextInput);
        let style = self.theme.input.clone();
        // **行距**：多行行高 = 字号 × 1.2（与排版缓冲 `line_mult` 一致；cosmic 行盒
        // 按此递增，光标/高亮按视觉行序号 × 行高对齐）。
        let line_h = (style.font_size * TEXT_AREA_LINE_SPACING).max(1.0);
        let content_w = (rect.w - style.padding_x * 2.0).max(0.0);
        let content_rect = Rect::new(rect.x + style.padding_x, rect.y, content_w, rect.h);
        let mouse_local_y = self.mouse_logical.y - self.abs_base.y;
        // **视觉行**（自动换行后）：光标/点击/选择/Home-End/↑↓ 全部按它定位，与显示一致。
        let vbuf = self.ensure_text_buf(
            id,
            value,
            style.font_size,
            style.font_family.as_deref(),
            content_w,
            TEXT_AREA_LINE_SPACING,
        );
        let vlines = Text::visual_lines(&vbuf);
        let vline_of_byte = |byte: usize| -> usize {
            vlines
                .iter()
                .position(|l| byte >= l.byte_start && byte <= l.byte_end)
                .unwrap_or(vlines.len().saturating_sub(1))
        };
        // 鼠标位置 → 光标（视觉行 + 行内列）。`btn.pressed()` 而非 `hit`：
        // **拖出输入框后仍跟随**（y 越界 clamp 到首/末行；垂直滚动随光标自动跟随）。
        let mouse_caret = if btn.pressed() {
            let row = ((mouse_local_y - rect.y) / line_h).floor().max(0.0) as usize;
            let li = row.min(vlines.len().saturating_sub(1));
            let line = &vlines[li];
            let ltxt = &value[line.byte_start..line.byte_end.min(value.len())];
            let cx = (self.mouse_local_x() - rect.x - style.padding_x).max(0.0);
            let col = self.caret_index_at_width(ltxt, style.font_size, style.font_family.as_deref(), cx);
            let col_byte = char_to_byte(ltxt, col);
            Some(byte_to_char(value, line.byte_start + col_byte))
        } else {
            None
        };
        let caret_est = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            let ev = update_interact(ws, hit, btn);
            if ev.pressed {
                self.any_pressed = true;
                self.press_claimed = true;
                self.state.focused = Some(id.to_owned());
                if let Some(c) = mouse_caret {
                    ws.caret = c;
                }
                ws.sel_anchor = Some(ws.caret);
            } else if ws.pressed && btn.pressed() {
                // 拖拽选择：按住并移动 → 光标跟随鼠标（**即使拖出输入框**——y 越界
                // clamp 到首/末视觉行；垂直滚动随光标自动跟随），范围 = [anchor, caret)。
                self.press_claimed = true;
                if let Some(c) = mouse_caret {
                    ws.caret = c;
                }
            }
            if ev.released && ws.sel_anchor == Some(ws.caret) {
                // 纯点击（无位移）→ 清理 anchor（无实际选择），避免残留 anchor 在
                // 后续无 Shift 方向键移动时"突然变成多选"。
                ws.sel_anchor = None;
            }
            let focused = self.state.focused.as_deref() == Some(id);
            if focused {
                let in_ime_compose =
                    self.keyboard.get_ime_preedit().is_some_and(|p| !p.is_empty());
                let ime_owns_keys = in_ime_compose || self.state.ime_composing;
                // 剪贴板（与单行输入框一致）
                let ctrl = self.keyboard.get(KeyCode::ControlLeft).pressed()
                    || self.keyboard.get(KeyCode::ControlRight).pressed();
                if ctrl {
                    let c_down = self.keyboard.get(KeyCode::KeyC).down_edge();
                    let v_down = self.keyboard.get(KeyCode::KeyV).down_edge();
                    let x_down = self.keyboard.get(KeyCode::KeyX).down_edge();
                    if c_down || x_down {
                        let sel = selected_text(value, ws.sel_anchor, ws.caret);
                        if !sel.is_empty() {
                            clipboard_set(&sel);
                            if x_down {
                                if let Some((lo, _hi)) = sel_range(ws.sel_anchor, ws.caret) {
                                    delete_range(value, lo, ws.caret.max(lo));
                                    ws.caret = lo;
                                }
                                ws.sel_anchor = None;
                            }
                        }
                    }
                    if v_down {
                        if let Some(text) = clipboard_get() {
                            if !text.is_empty() {
                                let lo = match sel_range(ws.sel_anchor, ws.caret) {
                                    Some((lo, hi)) => {
                                        delete_range(value, lo, hi);
                                        lo
                                    }
                                    None => ws.caret,
                                };
                                insert_str_at(value, lo, &text);
                                ws.caret = lo + text.chars().count();
                                ws.sel_anchor = None;
                            }
                        }
                    }
                }
                // 编辑操作前：选择替换（Ctrl 组合字符不触发）
                let edit_pending = (!self.keyboard.get_chars().is_empty() && !ctrl)
                    || !self.keyboard.get_ime_commits().is_empty()
                    || (self.keyboard.get(KeyCode::Backspace).down_edge() && !ime_owns_keys)
                    || (self.keyboard.get(KeyCode::Delete).down_edge() && !ime_owns_keys)
                    || self.keyboard.get(KeyCode::Enter).down_edge();
                if edit_pending {
                    if let Some((lo, hi)) = sel_range(ws.sel_anchor, ws.caret) {
                        delete_range(value, lo, hi);
                        ws.caret = lo;
                        ws.sel_anchor = None;
                    }
                }
                // 换行（Enter；TextArea 语义：插入 '\n'，Esc 失焦）
                if self.keyboard.get(KeyCode::Enter).down_edge() {
                    insert_char_at(value, ws.caret, '\n');
                    ws.caret = (ws.caret + 1).min(value.chars().count());
                }
                for commit in self.keyboard.get_ime_commits() {
                    insert_str_at(value, ws.caret, commit);
                    ws.caret = (ws.caret + commit.chars().count()).min(value.chars().count());
                }
                if !ctrl {
                    for ch in self.keyboard.get_chars() {
                        if *ch == '\n' || *ch == '\r' {
                            continue; // 换行统一由 Enter 处理
                        }
                        insert_char_at(value, ws.caret, *ch);
                        ws.caret = (ws.caret + 1).min(value.chars().count());
                    }
                }
                if self.keyboard.get(KeyCode::Backspace).down_edge() && !ime_owns_keys {
                    ws.caret = remove_before(value, ws.caret);
                }
                if self.keyboard.get(KeyCode::Delete).down_edge() && !ime_owns_keys {
                    remove_at(value, ws.caret);
                }
                // Shift + 方向键/Home/End：扩展选择
                let shift = self.keyboard.get(KeyCode::ShiftLeft).pressed()
                    || self.keyboard.get(KeyCode::ShiftRight).pressed();
                let shift_start = |ws: &mut WidgetState| {
                    if shift && ws.sel_anchor.is_none() {
                        ws.sel_anchor = Some(ws.caret);
                    }
                };
                let shift_shrink = |ws: &mut WidgetState| {
                    if ws.sel_anchor == Some(ws.caret) {
                        ws.sel_anchor = None;
                    }
                };
                if self.keyboard.get(KeyCode::ArrowLeft).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    ws.caret = ws.caret.saturating_sub(1);
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::ArrowRight).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    ws.caret = (ws.caret + 1).min(value.chars().count());
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::ArrowUp).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    // 跨**视觉行**（保持列；列 = 相对行首的 char 数）
                    let cur_byte = char_to_byte(value, ws.caret);
                    let li = vline_of_byte(cur_byte);
                    let col = byte_to_char(value, cur_byte) - byte_to_char(value, vlines[li].byte_start);
                    let tgt = li.saturating_sub(1);
                    let line = &vlines[tgt];
                    let ltxt = &value[line.byte_start..line.byte_end.min(value.len())];
                    let col = col.min(ltxt.chars().count());
                    ws.caret = byte_to_char(value, line.byte_start + char_to_byte(ltxt, col));
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::ArrowDown).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    let cur_byte = char_to_byte(value, ws.caret);
                    let li = vline_of_byte(cur_byte);
                    let col = byte_to_char(value, cur_byte) - byte_to_char(value, vlines[li].byte_start);
                    let tgt = (li + 1).min(vlines.len().saturating_sub(1));
                    let line = &vlines[tgt];
                    let ltxt = &value[line.byte_start..line.byte_end.min(value.len())];
                    let col = col.min(ltxt.chars().count());
                    ws.caret = byte_to_char(value, line.byte_start + char_to_byte(ltxt, col));
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::Home).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    let li = vline_of_byte(char_to_byte(value, ws.caret));
                    ws.caret = byte_to_char(value, vlines[li].byte_start);
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::End).down_edge() && !ime_owns_keys {
                    shift_start(ws);
                    let li = vline_of_byte(char_to_byte(value, ws.caret));
                    ws.caret = byte_to_char(value, vlines[li].byte_end);
                    shift_shrink(ws);
                }
                if self.keyboard.get(KeyCode::Escape).down_edge() {
                    self.state.focused = None;
                }
            }
            (focused, ws.caret)
        };
        let (focused, caret) = caret_est;
        // 光标所在**视觉行** → 光标 x / y（y = 视觉行序号 × 行高）
        let caret_byte = char_to_byte(value, caret);
        let caret_line = vline_of_byte(caret_byte);
        let caret_x = {
            let line = &vlines[caret_line];
            let end = caret_byte.min(line.byte_end).max(line.byte_start);
            let prefix = &value[line.byte_start..end];
            self.text_size(prefix, style.font_size, style.font_family.as_deref()).x
        };
        let caret_y = caret_line as f32 * line_h;
        // 垂直滚动：滚轮 + 光标跟随。内容高用**实际排版缓冲**（含行距 1.2）测量。
        let content_h = Text::measure_buffer(&vbuf).y / self.scale;
        let max_scroll = (content_h - rect.h).max(0.0);
        let scroll = {
            let ws = self.state.widgets.entry(id.to_owned()).or_default();
            if !ws.pressed {
                let (_, wy) = self.mouse.get_mouse_wheel_delta();
                if wy != 0.0 {
                    ws.scroll_y = (ws.scroll_y - wy as f32 * 30.0).clamp(0.0, max_scroll);
                }
            }
            // 光标跟随：光标行滚出可视区时调整
            if caret_y < ws.scroll_y {
                ws.scroll_y = caret_y;
            } else if caret_y + line_h > ws.scroll_y + rect.h {
                ws.scroll_y = (caret_y + line_h - rect.h).min(max_scroll);
            }
            ws.scroll_y = ws.scroll_y.clamp(0.0, max_scroll);
            ws.scroll_y
        };
        // 绘制
        let depth = self.depth;
        let win = self.cur_win;
        let elem = self.seq + 1;
        let border = if focused { style.border_focus } else { style.border };
        self.push_panel_like(rect, style.bg, border, style.border_w, style.radius, elem);
        // 选择高亮（逐**视觉行**；x = 行内前缀宽度，y = 视觉行序号 × 行高——与显示一致）
        if let Some((lo, hi)) = sel_range(
            self.state.widgets.get(id).and_then(|w| w.sel_anchor),
            caret,
        ) {
            let lo_byte = char_to_byte(value, lo);
            let hi_byte = char_to_byte(value, hi);
            let lo_li = vline_of_byte(lo_byte);
            let hi_li = vline_of_byte(hi_byte);
            for li in lo_li..=hi_li {
                let line = &vlines[li];
                let ls = line.byte_start.min(value.len());
                let le = line.byte_end.min(value.len());
                let c0b = if li == lo_li { lo_byte.max(ls).min(le) } else { ls };
                let c1b = if li == hi_li { hi_byte.max(ls).min(le) } else { le };
                if c1b <= c0b {
                    continue;
                }
                let x0 = self
                    .text_size(&value[ls..c0b], style.font_size, style.font_family.as_deref())
                    .x;
                let x1 = self
                    .text_size(&value[ls..c1b], style.font_size, style.font_family.as_deref())
                    .x;
                // y 随垂直滚动上移（-scroll）；clip = 内容区（选择高亮受裁剪，不溢出）。
                let sel_rect = Rect::new(
                    content_rect.x + x0,
                    rect.y + li as f32 * line_h - scroll,
                    (x1 - x0).max(0.0),
                    line_h,
                );
                if sel_rect.w > 0.0 {
                    let seq = self.next_seq();
                    self.queue.push(UiDraw {
                        depth,
                        seq,
                        win,
                        elem,
                        rect: sel_rect,
                        clip: Some(content_rect),
                        kind: DrawKind::Solid(style.sel_bg),
                    });
                }
            }
        }
        // 文本（换行 + 垂直滚动；clip 相对文本块：上缘 = scroll_y，高 = 可视区；
        // 缓冲控件自持——`vbuf` 已按内容区宽度排版，直接复用）。
        let seq = self.next_seq();
        self.queue.push(text_cmd(
            depth,
            seq,
            win,
            elem,
            Rect::new(content_rect.x, content_rect.y - scroll, content_w, rect.h),
            value.clone(),
            style.font_size,
            style.fg,
            TextAlign::Left,
            // 多行编辑：**顶对齐**（行盒顶 = 内容区顶），与光标/点击的 TopLeft 定位一致
            TextVAlign::Top,
            style.font_family.clone(),
            Some(Rect::new(0.0, scroll, content_w, rect.h)),
            self.clip,
            Some(vbuf),
        ));
        // IME 组合候选浮动提示框（输入框下方）
        if focused {
            if let Some(preedit) = self.keyboard.get_ime_preedit() {
                if !preedit.is_empty() {
                    let psize =
                        self.text_size(preedit, style.font_size, style.font_family.as_deref());
                    let box_pad = 6.0;
                    let box_w = (psize.x + box_pad * 2.0).max(20.0);
                    let box_h = (psize.y + 4.0).max(rect.h);
                    let box_rect = Rect::new(rect.x, rect.y + rect.h + 4.0, box_w, box_h);
                    let box_elem = self.seq + 1;
                    let bseq = self.next_seq();
                    self.queue.push(UiDraw {
                        depth,
                        seq: bseq,
                        win,
                        elem: box_elem,
                        rect: box_rect,
                        clip: self.clip,
                        kind: DrawKind::Solid(style.bg),
                    });
                    self.queue.push(UiDraw {
                        depth,
                        seq: bseq + 1,
                        win,
                        elem: box_elem,
                        rect: box_rect,
                        clip: self.clip,
                        kind: DrawKind::Border {
                            color: style.border_focus,
                            width: style.border_w,
                        },
                    });
                    let seq = self.next_seq();
                    self.queue.push(text_cmd(
                        depth,
                        seq,
                        win,
                        box_elem,
                        Rect::new(
                            box_rect.x + box_pad,
                            box_rect.y,
                            box_w - box_pad * 2.0,
                            box_h,
                        ),
                        preedit.to_owned(),
                        style.font_size,
                        style.preedit,
                        TextAlign::Left,
                        TextVAlign::Center,
                        style.font_family.clone(),
                        None,
                        self.clip,
                    None,
                    ));
                }
            }
            let ime_x = ((self.abs_base.x + content_rect.x + caret_x) * self.scale) as i32;
            let ime_y = ((self.abs_base.y + rect.y + caret_y - scroll) * self.scale) as i32;
            let ime_w = (rect.w * self.scale).max(1.0) as u32;
            let ime_h = (line_h * self.scale).max(1.0) as u32;
            let _ = self.window.set_ime_cursor_area(
                PhysicalPosition::new(ime_x, ime_y),
                PhysicalSize::new(ime_w, ime_h),
            );
        }
        // 光标
        if focused && self.state.caret_blink_on() {
            let caret_rect = Rect::new(
                content_rect.x + caret_x,
                rect.y + caret_y - scroll,
                1.0,
                line_h,
            );
            let seq = self.next_seq();
            self.queue.push(UiDraw {
                depth,
                seq,
                win,
                elem,
                rect: caret_rect,
                clip: self.clip,
                kind: DrawKind::Caret {
                    color: style.caret,
                    width: 1.0,
                },
            });
        }
    }
}

// ─── 对齐转换 ───────────────────────────────────────────────────

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

// ─── 单元测试（无 GPU） ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_char_at_handles_caret() {
        let mut s = String::from("abc");
        insert_char_at(&mut s, 1, 'X');
        assert_eq!(s, "aXbc");
        insert_char_at(&mut s, 0, '!');
        assert_eq!(s, "!aXbc");
        insert_char_at(&mut s, 99, 'z');
        assert_eq!(s, "!aXbcz", "caret 越界 clamp 到末尾");
        // 多字节字符按 char 索引
        let mut s = String::from("中文ab");
        insert_char_at(&mut s, 2, '字');
        assert_eq!(s, "中文字ab");
    }

    #[test]
    fn remove_before_and_at() {
        let mut s = String::from("abc");
        assert_eq!(remove_before(&mut s, 2), 1);
        assert_eq!(s, "ac");
        remove_at(&mut s, 0);
        assert_eq!(s, "c");
        // 多字节
        let mut s = String::from("中文");
        assert_eq!(remove_before(&mut s, 2), 1);
        assert_eq!(s, "中", "caret=2 删除前一个字符（'文'）");
        remove_at(&mut s, 0);
        assert_eq!(s, "");
    }

    #[test]
    fn text_align_conversion_roundtrip() {
        assert_eq!(Align::from(TextAlign::Left), Align::Left);
        assert_eq!(Align::from(TextAlign::Center), Align::Center);
        assert_eq!(Align::from(TextAlign::Right), Align::Right);
        assert_eq!(TextAlign::from(Align::Left), TextAlign::Left);
        assert_eq!(TextAlign::from(Align::Center), TextAlign::Center);
        assert_eq!(TextAlign::from(Align::Right), TextAlign::Right);
        // 其他 Align（Justified/End）归入 Center
        assert_eq!(TextAlign::from(Align::Justified), TextAlign::Center);
        assert_eq!(TextAlign::from(Align::End), TextAlign::Center);
    }

    #[test]
    fn draw_kind_group_graphic_before_text() {
        // 同一 layer 内：图形（Solid/Border/Caret）分组 0，文字（Text）分组 1
        assert_eq!(DrawKind::Solid(Color::WHITE).group(), 0);
        assert_eq!(
            DrawKind::Border {
                color: Color::WHITE,
                width: 1.0,
            }
            .group(),
            0
        );
        assert_eq!(DrawKind::Caret { color: Color::WHITE, width: 1.0 }.group(), 0);
        assert_eq!(
            DrawKind::Text {
                text: "x".into(),
                size: 14.0,
                color: Color::WHITE,
                align: TextAlign::Left,
                family: None,
                valign: TextVAlign::Center,
                clip: None,
                buf: None,
            }
            .group(),
            1
        );
        // finish 排序键顺序：win → depth → elem → group → seq
        let mut cmds = vec![
            UiDraw {
                depth: 0,
                seq: 2,
                win: 0,
                elem: 1,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                clip: None,                kind: DrawKind::Text {                    text: "t".into(),
                    size: 14.0,
                    color: Color::WHITE,
                    align: TextAlign::Left,
                    family: None,
                    valign: TextVAlign::Center,
                    clip: None,
                    buf: None,
                },
            },
            UiDraw {
                depth: 0,
                seq: 1,
                win: 0,
                elem: 1,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                kind: DrawKind::Solid(Color::WHITE),
                clip: None,
            },
            UiDraw {
                depth: 0,
                seq: 3,
                win: 1,
                elem: 2,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                kind: DrawKind::Solid(Color::WHITE),
                clip: None,
            },
            UiDraw {
                depth: 0,
                seq: 4,
                win: 1,
                elem: 2,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                clip: None,                kind: DrawKind::Text {                    text: "w".into(),
                    size: 14.0,
                    color: Color::WHITE,
                    align: TextAlign::Left,
                    family: None,
                    valign: TextVAlign::Center,
                    clip: None,
                    buf: None,
                },
            },
        ];
        cmds.sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
        // 期望：win0 图形(elem1) → win0 文字(elem1) → win1 图形(elem2) → win1 文字(elem2)
        let order: Vec<u32> = cmds.iter().map(|d| d.seq).collect();
        assert_eq!(order, vec![1, 2, 3, 4], "窗口 z 升序；元素内图形先、文字后");
    }

    #[test]
    fn submit_order_graphics_before_text_per_window() {
        // 回归：窗口图形/文字绘制顺序抖动——同一窗口内图形组（白纹理 / 圆角 / 渐变）
        // 必须先于字形文字组，且跨帧稳定（不随 HashMap 迭代顺序 / 纹理 uid 分配变化）。
        let g = GROUP_GRAPHIC;
        let t = GROUP_TEXT;
        // 模拟 finish() 的提交列表：win=0 非窗口内容 + 窗口 1/2（各含图形组与文字组）。
        // 故意用乱序 + 反序 uid 输入（如 HashMap 迭代顺序）。
        let mut groups = vec![
            (2u32, t, 3u64),
            (1u32, t, 3u64),
            (0u32, t, 2u64),
            (2u32, g, 2u64),
            (0u32, g, 1u64),
            (1u32, g, 1u64),
            (1u32, g, 2u64),
        ];
        groups.sort_by_key(|&(w, gr, tex)| (w, gr, tex));
        // 期望：win 升序；同一窗口内图形组先于文字组；组内按纹理 uid 稳定。
        assert_eq!(
            groups,
            vec![
                (0, g, 1),
                (0, t, 2),
                (1, g, 1),
                (1, g, 2),
                (1, t, 3),
                (2, g, 2),
                (2, t, 3),
            ],
            "win 升序 + 窗口内图形先于文字（含程序化纹理），跨帧确定"
        );
    }

    #[test]
    fn drag_needs_movement_so_clicks_work() {
        // 回归：窗口/可拖拽面板内 CheckBox / 输入框失效——按下即激活拖拽导致
        // `drag_panel` 在释放帧抑制子控件，点击被吞。修复：位移 ≥ DRAG_ACTIVATE_PX
        // 才视为拖拽；纯点击（无位移 / 微小抖动）不激活。
        let press = Vec2::new(100.0, 200.0); // 按下基准（物理像素，已取整）
        // 静止（纯点击）：未激活
        assert!(!drag_moved(press, Some(press)), "按下无位移不激活拖拽");
        // 微小抖动（< 阈值）：仍视为点击
        assert!(!drag_moved(press + Vec2::new(1.0, 1.0), Some(press)), "1px 抖动不激活");
        assert!(!drag_moved(press + Vec2::new(2.0, 2.0), Some(press)), "~2.8px 位移不激活");
        // 恰好达到阈值
        assert!(
            drag_moved(press + Vec2::new(3.0, 0.0), Some(press)),
            "3px 水平位移激活拖拽"
        );
        // 明显位移：激活
        assert!(
            drag_moved(press + Vec2::new(0.0, -8.0), Some(press)),
            "8px 竖直位移激活拖拽"
        );
        // 无按下基准（如窗口未命中时）：不激活
        assert!(!drag_moved(press, None), "无按下基准不激活");
    }

    #[test]
    fn caret_index_by_width_mixed_cjk_ascii() {
        // 回归：光标位置错误——混合中英文（字宽不同）时用等比估算会把光标落在
        // 错误的字符边界（点击/打字插错位置）。修复：按前缀**实际宽度**二分。
        // 宽度表模拟：你=2.0 好=2.0 a=1.0 b=1.0 c=1.0（prefix 宽度单调不减）
        let widths = [2.0f32, 2.0, 1.0, 1.0, 1.0]; // "你好abc"
        let w = |k: usize| -> f32 { widths[..k].iter().sum() };
        assert_eq!(caret_index_by_width(5, 0.0, w), 0, "点击最左 → 0");
        assert_eq!(caret_index_by_width(5, 1.5, w), 1, "'你' 左半 → 1（等比估算会错）");
        assert_eq!(caret_index_by_width(5, 2.0, w), 1, "'你' 右缘 → 1");
        assert_eq!(caret_index_by_width(5, 3.5, w), 2, "'好' 中 → 2");
        assert_eq!(caret_index_by_width(5, 4.0, w), 2, "'好' 右缘 → 2");
        assert_eq!(caret_index_by_width(5, 4.9, w), 3, "'a' 右缘 → 3");
        assert_eq!(caret_index_by_width(5, 6.6, w), 5, "文本末尾之后 → 5");
        assert_eq!(caret_index_by_width(5, 99.0, w), 5, "远超末尾 → 5");
        assert_eq!(caret_index_by_width(0, 3.0, w), 0, "空文本 → 0");
    }

    #[test]
    fn text_block_placement_is_integer_with_line_box_top() {
        // 回归：UI 文本亚像素模糊——垂直行盒对齐偏移经**整数运算**后，
        // `block_tl = anchor + off` 的两侧操作数均为整数（浮点整数不变量）：
        // 小数（行盒顶 / 奇数宽的一半）只在 `round` 边界被一次性消化，
        // 不流入加法链 → 无误差累加、字形四边形角点恒为整数屏幕像素。
        // 数据来自 14px 排版实测：行高 16.8→content_h 17，"a" 行盒顶 = -7.2
        // （rjw_text 收集期取整为 -7.0）。
        let anchor = Vec2::new(100.0, 200.0);
        let content = Vec2::new(14.0, 17.0);
        let first_line_top = -7.0;
        // 左对齐标签：block = anchor + off，两项均为整数
        let block = anchor + text_block_offset(TextAlign::Left, TextVAlign::Center, content, first_line_top);
        assert_eq!(block.x.fract(), 0.0, "标签块 x 必须为整数像素，实际 {}", block.x);
        assert_eq!(block.y.fract(), 0.0, "标签块 y 必须为整数像素，实际 {}", block.y);
        // 行盒中心对准锚点（±0.5px 量化，奇数 content_h 的固有半像素）：
        // block.y + first_line_top + content_h/2 ≈ anchor.y
        let center = block.y + first_line_top + content.y * 0.5;
        assert!(
            (center - anchor.y).abs() <= 0.5,
            "行盒中心应贴近锚点，偏差 {}",
            center - anchor.y
        );
        // 居中按钮 + 奇数物理宽（21px，如 DPI 1.5 下）：水平对齐亦为整数
        let btn = anchor + text_block_offset(TextAlign::Center, TextVAlign::Center, Vec2::new(21.0, 17.0), first_line_top);
        assert_eq!(btn.x.fract(), 0.0, "按钮块 x 必须为整数像素，实际 {}", btn.x);
        assert_eq!(btn.y.fract(), 0.0, "按钮块 y 必须为整数像素，实际 {}", btn.y);
    }

    #[test]
    fn overlapping_elements_follow_record_order() {
        // 元素重叠层级正确性：后录元素（elem 大）覆盖先录元素（elem 小），
        // **即使后录元素是图形（group 0）、先录元素是文字（group 1）**——
        // 元素序优先于图形/文字分组。
        let mut cmds = vec![
            // 元素 A（先录）：文字
            UiDraw {
                depth: 1,
                seq: 1,
                win: 1,
                elem: 1,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                clip: None,                kind: DrawKind::Text {                    text: "a".into(),
                    size: 14.0,
                    color: Color::WHITE,
                    align: TextAlign::Left,
                    family: None,
                    valign: TextVAlign::Center,
                    clip: None,
                    buf: None,
                },
            },
            // 元素 B（后录）：图形——应覆盖 A 的文字
            UiDraw {
                depth: 1,
                seq: 2,
                win: 1,
                elem: 2,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                kind: DrawKind::Solid(Color::WHITE),
                clip: None,
            },
        ];
        cmds.sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
        let order: Vec<u32> = cmds.iter().map(|d| d.seq).collect();
        assert_eq!(order, vec![1, 2], "后录元素（B 图形）应覆盖先录元素（A 文字）");
        // 元素内：同一 elem 的图形先于文字
        let mut inner = vec![
            UiDraw {
                depth: 1,
                seq: 3,
                win: 1,
                elem: 3,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                clip: None,                kind: DrawKind::Text {                    text: "x".into(),
                    size: 14.0,
                    color: Color::WHITE,
                    align: TextAlign::Left,
                    family: None,
                    valign: TextVAlign::Center,
                    clip: None,
                    buf: None,
                },
            },
            UiDraw {
                depth: 1,
                seq: 2,
                win: 1,
                elem: 3,
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                kind: DrawKind::Solid(Color::WHITE),
                clip: None,
            },
        ];
        inner.sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
        let order: Vec<u32> = inner.iter().map(|d| d.seq).collect();
        assert_eq!(order, vec![2, 3], "元素内：图形(seq2)先画，文字(seq3)后画（文字覆盖图形）");
    }

    #[test]
    fn capturing_text_reflects_focus() {
        let mut st = UiState::new();
        assert!(!st.capturing_text());
        st.focused = Some("name".to_owned());
        assert!(st.capturing_text(), "有输入焦点时应捕获键盘");
        st.focused = None;
        assert!(!st.capturing_text());
    }
}
