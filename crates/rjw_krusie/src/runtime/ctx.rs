//! `Ctx`：**本帧宿主事实**（dt / fps / 输入 / 窗口 / DPI / 退出）+ 取帧 + 渲染面入口。
//!
//! 分工（见 `docs/API_DESIGN.md` 责任表）：
//!
//! - [`Ctx`]：每帧都有（无帧也有），只描述"宿主发生了什么"；
//! - [`Frame`](crate::runtime::Frame)：只在**取到表面**时存在，承载渲染面；
//! - `App::update` 每帧必调；应用用 `let Some(mut f) = ctx.frame() else { return };` 守卫渲染代码。
//!
//! 多窗口就绪：`Ctx` 属于一个 [`WindowId`]；`frame_of` 取指定窗口的帧
//! （本期只有主窗口，等价于 [`Ctx::frame`]）。

use std::sync::Arc;

use rjw_2d_render::{Layer, Render2D, SpriteRect};
use rjw_main::{DeltaTimer, KeyboardInput, MouseInput};
use rjw_render::wgpu;
use rjw_render::{Clear, RenderContext, RenderFrame};
use rjw_transform::{Camera2D, Rect, Transform2D};
use rjw_color::Color;
use rjw_transform::Vec2;

use crate::runtime::frame::Frame;
use crate::runtime::window::WindowId;
use crate::runtime::{AppConfig, ViewportBorders};

/// winit 窗口类型（经 `rjw_main` 重导出的同一版本）。
type WinitWindow = rjw_main::Window;

/// 每帧上下文。
pub struct Ctx {
    // ── 窗口 / 宿主 ──
    pub(crate) window: Option<Arc<WinitWindow>>,
    pub(crate) window_id: WindowId,
    /// 表面尺寸（物理像素）。
    pub(crate) size: (u32, u32),
    /// DPI 缩放（物理 / 逻辑）。
    pub(crate) scale: f32,

    // ── 时间 ──
    pub(crate) timer: DeltaTimer,
    pub(crate) dt: f32,
    pub(crate) fps: f64,
    pub(crate) elapsed: f64,

    // ── 输入 ──
    pub(crate) keyboard: KeyboardInput,
    pub(crate) mouse: MouseInput,

    // ── 帧 ──
    pub(crate) surface: Option<RenderFrame>,
    pub(crate) world: Option<Render2D>,
    pub(crate) region: Option<Rect>,
    /// **画面边框**（调试：区分多画面 / 分屏；来自 AppConfig）。
    pub(crate) viewport_borders: ViewportBorders,
    /// 本帧已提交的画面序号（画面边框配色 / 标注用；每帧复位）。
    pub(crate) frame_viewports: u32,
    /// 本帧各画面的 (矩形, 序号)（画面边框 overlay 用；每帧复位）。
    pub(crate) frame_regions: Vec<(Rect, u32)>,
    /// 本帧是否已经开过 pass（多画面：只有首个画面能用 load-op 颜色清屏）。
    pub(crate) submitted: bool,
    pub(crate) presented: bool,
    pub(crate) clear: Clear,

    // ── 可选层 ──
    #[cfg(feature = "text")]
    pub(crate) text: Option<rjw_text::Text>,
    #[cfg(feature = "ui")]
    pub(crate) ui_layer: Option<crate::runtime::layers::ui::UiLayer>,

    // ── 宿主状态 ──
    pub(crate) exit_requested: bool,
    pub(crate) frames: u64,
    /// 调试注入的鼠标左键是否处于按住状态（[`Ctx::debug_inject_mouse`] 的边沿合成用）。
    pub(crate) debug_mouse_down: bool,
    /// **已呈现**的帧数（[`Ctx::present`] 真正提交 encoder + present 时 +1）。
    ///
    /// 冒烟 harness 用它抓「跑满 N 帧却一帧都没 present」这类**画面空白**回归：
    /// `frames` 只是迭代数（无帧 / 提交了但未呈现都会增长），`presented_frames`
    /// 才是"画面上真的换了帧"的证据。
    pub(crate) presented_frames: u64,
    pub(crate) no_frame_streak: u64,
    /// 是否接入了渲染上下文（headless 测试时为 `false`）。
    pub(crate) attached: bool,
}

impl Ctx {
    /// 空上下文（引擎在 `resumed` 里 `attach`；测试用 [`Ctx::headless`]）。
    pub(crate) fn new(config: &AppConfig) -> Self {
        Self {
            window: None,
            window_id: WindowId::PRIMARY,
            size: (1, 1),
            scale: 1.0,
            timer: DeltaTimer::default(),
            dt: 0.0,
            fps: 0.0,
            elapsed: 0.0,
            keyboard: KeyboardInput::default(),
            mouse: MouseInput::default(),
            surface: None,
            world: None,
            region: None,
            viewport_borders: config.viewport_borders,
            frame_viewports: 0,
            frame_regions: Vec::new(),
            submitted: false,
            presented: false,
            clear: config.clear,
            #[cfg(feature = "text")]
            text: None,
            #[cfg(feature = "ui")]
            ui_layer: None,
            exit_requested: false,
            frames: 0,
            debug_mouse_down: false,
            presented_frames: 0,
            no_frame_streak: 0,
            attached: false,
        }
    }

    /// **headless 上下文**（无窗口 / 无 GPU）：只跑宿主事实与取帧守卫，供单元测试
    /// 验证「无帧也执行 `update`」「退避」等逻辑。
    pub fn headless(config: &AppConfig) -> Self {
        Self::new(config)
    }

    /// 接入窗口与渲染上下文（引擎内部；建两个渲染器 + 可选子系统）。
    pub(crate) fn attach(&mut self, window: Arc<WinitWindow>, render: &RenderContext) {
        self.size = render.size();
        self.scale = window.scale_factor() as f32;
        self.window = Some(window);

        let mut world = Render2D::new(render);
        world.reset();
        self.world = Some(world);

        #[cfg(feature = "text")]
        {
            self.text = Some(rjw_text::Text::new(render.gpu()));
        }
        #[cfg(feature = "ui")]
        {
            // UI 层自带专用渲染器（SortMode::None）：状态 + 主题 + 渲染器都在层里。
            self.ui_layer = Some(crate::runtime::layers::ui::UiLayer::new(render));
        }
        self.attached = true;
    }

    // ── 宿主事实 ───────────────────────────────────────────

    /// 本上下文所属窗口。
    #[inline]
    pub fn window(&self) -> WindowId {
        self.window_id
    }

    /// 帧间隔（秒）。
    #[inline]
    pub fn dt(&self) -> f32 {
        self.dt
    }

    /// 帧间隔（秒，f64）。
    #[inline]
    pub fn dt_f64(&self) -> f64 {
        self.dt as f64
    }

    /// 瞬时 FPS（`DeltaTimer` 平滑值）。
    #[inline]
    pub fn fps(&self) -> f64 {
        self.fps
    }

    /// 累计经过时间（秒，自 `resumed` 起）。
    #[inline]
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// DPI 缩放（物理 / 逻辑）。
    #[inline]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// 表面尺寸（物理像素）。
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// 键盘状态查询。
    #[inline]
    pub fn key(&self, code: rjw_main::KeyCode) -> rjw_main::KeyState {
        self.keyboard.key(code)
    }

    /// 鼠标按键状态查询。
    #[inline]
    pub fn button(&self, button: rjw_main::MouseButton) -> rjw_main::KeyState {
        self.mouse.button(button)
    }

    /// 键盘设备（`chars()` / IME / 遍历）。
    #[inline]
    pub fn keys(&self) -> &KeyboardInput {
        &self.keyboard
    }

    /// 鼠标设备（位置 / 位移 / 滚轮）。
    #[inline]
    pub fn mouse(&self) -> &MouseInput {
        &self.mouse
    }

    /// 请求在本帧结束后退出事件循环。
    #[inline]
    pub fn exit(&mut self) {
        self.exit_requested = true;
    }

    /// **调试注入鼠标**（脚本化复现交互；见 `docs/DEBUGGING.md`）：
    /// 直接指定本帧的**光标物理像素位置**与**左键是否按住**，不需要真实鼠标
    /// （无鼠标环境 / CI / 自动回归）。
    ///
    /// 边沿由引擎合成：`down` 由 `false→true` 的那帧给 `down_edge`，按住期间给
    /// `pressed`，`true→false` 的那帧给 `up_edge`——与真实设备的状态机语义一致，
    /// 因此 UI 的命中 / 拖拽 / 点击逻辑无差别地生效。
    ///
    /// 用法（应用在 `update` 里、取帧之后、`f.ui(..)` 之前调用）：
    /// ```ignore
    /// let Some(mut f) = ctx.frame() else { return };
    /// // 第 10 帧起按住并向右拖：复现"窗口拖动"
    /// let n = f.frames();
    /// if (10..60).contains(&n) {
    ///     f.debug_inject_mouse(Vec2::new(100.0 + (n as f32 - 10.0) * 4.0, 100.0), true);
    /// }
    /// ```
    pub fn debug_inject_mouse(&mut self, pos_px: impl Into<Vec2>, down: bool) {
        let pos = pos_px.into();
        let was_down = self.debug_mouse_down;
        self.debug_mouse_down = down;
        // 边沿合成在 `rjw_mouse` 内（与真实设备同一套 `KeyState` 语义）。
        self.mouse.debug_inject_press((pos.x as f64, pos.y as f64), down, was_down);
    }

    /// 是否已请求退出（引擎读取后清位）。
    #[inline]
    pub fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    /// 本帧是否有可呈现的表面。
    #[inline]
    pub fn has_frame(&self) -> bool {
        self.surface.is_some()
    }

    /// 已完成的帧迭代数（含无帧帧）。
    #[inline]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// **已呈现**的帧数（真正提交 encoder + present）。
    ///
    /// 与 [`Self::frames`] 的区别：`frames` 是迭代数（无帧 / 提交了但没呈现都会增长），
    /// 本值才是"画面上真的换了帧"的证据（冒烟 harness 据此判定示例确实有画面）。
    #[inline]
    pub fn presented_frames(&self) -> u64 {
        self.presented_frames
    }

    /// 连续无帧迭代数（退避策略据此工作）。
    #[inline]
    pub fn no_frame_streak(&self) -> u64 {
        self.no_frame_streak
    }

    /// 当前画面矩形（默认 = 全窗口）。
    #[inline]
    pub fn region(&self) -> Rect {
        self.region.unwrap_or_else(|| self.full_region())
    }

    /// 设定本画面矩形（分屏 / 画中画；一般用 `Frame::set_region`）。
    #[inline]
    pub fn set_region(&mut self, region: Rect) {
        self.region = Some(region);
    }

    /// 全窗口矩形。
    #[inline]
    pub fn full_region(&self) -> Rect {
        Rect::new(0.0, 0.0, self.size.0 as f32, self.size.1 as f32)
    }

    // ── 渲染面 ─────────────────────────────────────────────

    /// 本窗口本帧的渲染面；**取不到表面时返回 `None`**（应用自行守卫）。
    ///
    /// ```ignore
    /// let Some(mut f) = ctx.frame() else { return };   // ← 无帧不执行渲染代码
    /// ```
    #[inline]
    pub fn frame(&mut self) -> Option<Frame<'_>> {
        if self.surface.is_some() {
            Some(Frame::new(self))
        } else {
            None
        }
    }

    /// 指定窗口的渲染面（多窗口就绪；本期只有主窗口，等价于 [`Self::frame`]）。
    #[inline]
    pub fn frame_of(&mut self, window: WindowId) -> Option<Frame<'_>> {
        if window == self.window_id {
            self.frame()
        } else {
            None
        }
    }

    /// 窗口句柄（低层逃生口：标题 / 光标 / IME 等 winit 操作）。
    #[inline]
    pub fn window_handle(&self) -> Option<&WinitWindow> {
        self.window.as_deref()
    }

    /// 当前窗口列表（多窗口就绪；本期只有主窗口）。
    #[inline]
    pub fn windows(&self) -> impl Iterator<Item = WindowId> {
        std::iter::once(self.window_id)
    }

    /// 逃生口：原生帧 / encoder / device / queue（自定义编码 / 后处理）。
    #[inline]
    pub fn escape(&mut self) -> Escape<'_> {
        Escape { frame: self.surface.as_mut() }
    }

    // ── 引擎内部 ───────────────────────────────────────────

    /// 世界层渲染器（`Frame::draw` 用）。
    #[inline]
    pub(crate) fn world_mut(&mut self) -> &mut Render2D {
        self.world.as_mut().expect("Ctx 未接入渲染上下文（headless）：无法绘制")
    }

    /// UI 层渲染器（`Frame::draw_ui` / `Frame::text_ui` 用）。
    #[inline]
    pub(crate) fn ui_mut(&mut self) -> &mut Render2D {
        self.ui_layer
            .as_mut()
            .and_then(|l| l.r2d.as_mut())
            .expect("Ctx 未接入渲染上下文（headless）：无法绘制")
    }

    /// 每帧开始：刷新时间，复位画面状态，统计无帧连续次数。
    pub(crate) fn begin_update(&mut self) {
        self.timer.per_frame();
        self.dt = self.timer.dt().get_f32();
        self.fps = self.timer.get_fps();
        self.elapsed += self.dt as f64;
        self.region = None;
        self.submitted = false;
        self.frame_viewports = 0;
        self.frame_regions.clear();
        self.presented = false;
        self.no_frame_streak = if self.surface.is_some() { 0 } else { self.no_frame_streak + 1 };
    }

    /// 每帧结束：未提交则用配置的清屏自动提交并呈现；清空残留录制；复位一帧状态。
    pub(crate) fn end_update(&mut self) {
        // 本迭代是否取到了表面（渲染帧）。
        let rendered = self.surface.is_some();
        // UI 帧收尾（幂等）：焦点导航 + 描边 / 光标 / 统计写回 / 帧级暂存关场。
        // 必须在**提交之前**（描边命令要进本次提交的 UI 队列）。
        self.ui_end_frame();
        if rendered && !self.presented {
            // 应用没有显式 submit：用配置的清屏 + identity 相机自动提交世界 / UI
            // （日志便于排查「画面被清掉」类问题：本行**不应**在应用已 submit 的帧出现）。
            log::trace!("krusie: 应用未提交本帧，使用 AppConfig::clear 自动清屏并呈现");
            self.submit_with(self.clear, None);
            // 画面边框 overlay（若开启）：也走"帧末最后一个 pass"。
            self.submit_viewport_borders();
            self.present();
        }
        // 防御：未提交的录制不得跨帧累积。
        if let Some(w) = self.world.as_mut() {
            w.discard();
        }
        #[cfg(feature = "ui")]
        if let Some(u) = self.ui_layer.as_mut().and_then(|l| l.r2d.as_mut()) {
            u.discard();
        }
        // 字形图集寿命推进（**只在渲染帧**推进：无帧时不应让缓存老化）。
        #[cfg(feature = "text")]
        if rendered
            && let Some(text) = self.text.as_mut() {
                text.tick();
            }
        self.surface = None;
        self.region = None;
        self.submitted = false;
        self.presented = false;
        self.frames += 1;
    }

    /// **本帧是否已被应用接管**（调用过 `Frame::submit` / `submit_keep`）。
    ///
    /// 与 [`Self::presented`]（真正 present 过）分开：帧末只对"应用没接管"的帧补一次
    /// 自动提交（`AppConfig::clear` 清屏 + 世界 / UI 层）。
    #[inline]
    pub(crate) fn app_submitted(&self) -> bool {
        self.submitted
    }

    /// 帧末自动提交：用 `AppConfig::clear` + identity 相机把世界层 + UI 层落成一个 pass。
    #[inline]
    pub(crate) fn submit_default_clear(&mut self) {
        let clear = self.clear;
        self.submit_with(clear, None);
    }

    /// 提交当前队列为一个 pass（世界层 + UI 层）。
    ///
    /// `cam = None` ⇒ 用 identity 相机（自动提交路径 / 仅 UI 路径）。
    pub(crate) fn submit_with(&mut self, clear: Clear, cam: Option<&Camera2D>) {
        let region = self.region();
        let fallback;
        let cam = match cam {
            Some(c) => c,
            None => {
                fallback = Camera2D::new(region);
                &fallback
            }
        };
        // **多画面：颜色清屏必须限制在画面矩形内**。
        // wgpu 的 load-op 颜色清屏作用于**整张附件**：第二个画面「清成某色」会把第一个
        // 画面一起抹掉（左分屏变纯色 = "左分屏完全未显示"）。所以非首个画面把颜色清屏
        // 降级为「保留 + 在世界层先画一块覆盖画面矩形的实心底」——语义不变、只影响本画面。
        let first_pass = !self.submitted;
        self.submitted = true;
        self.frame_viewports += 1;
        let fill_color = if first_pass { None } else { clear.color_value() };
        let pass_clear = if fill_color.is_some() {
            clear.without_color()
        } else {
            clear
        };

        // **UI 帧收尾**（幂等）：应用可能录了多段 UI 且本帧只 submit 一次；焦点描边 /
        // 光标 / 统计属于"每帧一次"，必须在这里补齐——且必须在取 UI 队列（下面）之前。
        self.ui_end_frame();
        let Self { surface, world, ui_layer, .. } = self;
        let ui_2d = ui_layer.as_mut().and_then(|l| l.r2d.as_mut());
        let Some(surface) = surface.as_mut() else {
            return;
        };
        // 诊断：每个画面的矩形（`RUST_LOG=rjw_krusie=debug`）——多画面"画到哪去了"先看这行。
        log::debug!(
            "krusie: 画面 #{idx} region=({x:.0},{y:.0},{w:.0},{h:.0}) clear={clear:?}",
            idx = self.frame_viewports,
            x = region.x,
            y = region.y,
            w = region.w,
            h = region.h,
            clear = pass_clear,
        );
        // 屏幕空间 → 世界：`f(px) = cam.transform(px − region.center())`
        // （把"屏幕像素矩形"表达成该画面相机下的世界矩形；相机平移/旋转/缩放都对）。
        let screen_tf = screen_space_transform(cam, region);
        // 画面底色（世界层第一条命令；屏幕空间矩形 → 世界变换，随相机旋转/缩放也对）。
        if let (Some(color), Some(world)) = (fill_color, world.as_mut()) {
            world
                .solid(SpriteRect::new(region.min(), region.size()))
                .tint(Color::from(color))
                .transform(screen_tf)
                .layer(Layer::from(-1.0e18));
        }
        // **画面边框（调试）**：本画面矩形描边 + 左上角标注 `#序号 宽×高`
        // （`AppConfig::viewport_borders`）——统一在帧末的**全屏 overlay pass** 里画
        // （见 [`Self::submit_viewport_borders`]）：本画面 pass 的视口会把别的画面裁掉。
        self.frame_regions.push((region, self.frame_viewports));

        let mut pass = surface.pass(pass_clear);
        if let Some(world) = world.as_mut() {
            world.submit(&mut pass, cam);
        }
        if let Some(ui) = ui_2d {
            // UI 层坐标 = **物理像素、左上原点**（与 `rjw_ui` 的公开 API 一致）：
            // 把 UI 相机整体平移 `region.center()` ⇒ `screen_to_world(px) == px`。
            // 这样 `f.draw_ui()` / `f.text_ui()` 里的 (0,0) 就是窗口左上角，
            // 不需要调用方自己减半屏（旧实现是居中坐标，极易把 UI 画到屏幕中央）。
            let mut ui_cam = Camera2D::new(region);
            ui_cam.transform.pos = region.center();
            ui.submit(&mut pass, &ui_cam);
        }
        // 标记本帧已被应用接管：帧尾不再补一次「配置清屏」的 pass
        // （否则那次 pass 会把刚提交的画面清掉）。
        drop(pass);
        self.presented = true;
    }

    /// **画面边框 overlay pass**（`AppConfig::viewport_borders` 开启时，帧末 present 之前
    /// 由 `Frame` 收尾调用）：
    ///
    /// - 打开一个 **视口 = 整屏** 的 pass（`Clear::Keep`：不动已画好的颜色 / 深度）；
    /// - 在 **UI 层（最上层覆盖）** 给本帧每个画面矩形描一圈（按序号配色）+ 左上角标注
    ///   `#序号 宽×高`；
    /// - 只描边不填充 ⇒ 不挡内容，用于多画面 / 分屏时区分"哪块是哪块"。
    ///
    /// 返回是否真的画了（未开启 / 本帧无画面 ⇒ `false`）。
    pub(crate) fn submit_viewport_borders(&mut self) -> bool {
        if !self.viewport_borders.is_on() || self.frame_regions.is_empty() {
            return false;
        }
        let full = self.full_region();
        // 记录区已经收集完，先取出（提交过程不再往 frame_regions 里追加）。
        let regions = std::mem::take(&mut self.frame_regions);
        self.region = Some(full);
        log::debug!("krusie: 画面边框 overlay：{} 个画面", regions.len());

        let Self { surface, ui_layer, text, .. } = self;
        let Some(surface) = surface.as_mut() else {
            return false;
        };
        let Some(ui) = ui_layer.as_mut().and_then(|l| l.r2d.as_mut()) else {
            return false;
        };
        for (region, idx) in &regions {
            draw_viewport_border(ui, *region, *idx);
        }
        #[cfg(feature = "text")]
        if let Some(text) = text.as_mut() {
            let mut tc = rjw_text::TextCtx::new(text, ui);
            for (region, idx) in &regions {
                let cap = format!("#{} {:.0}×{:.0}", idx, region.size().x, region.size().y);
                tc.label(cap)
                    .size(16.0)
                    .at(region.min() + Vec2::new(8.0, 6.0))
                    .color(viewport_border_color(*idx))
                    .draw(1.0e18);
            }
        }
        // 视口 = 整屏（UI 层坐标 = 物理像素、左上原点）。
        let mut ui_cam = Camera2D::new(full);
        ui_cam.transform.pos = full.center();
        let mut pass = surface.pass(Clear::Keep);
        ui.submit(&mut pass, &ui_cam);
        drop(pass);
        self.presented = true;
        true
    }

    /// 提交并呈现（encoder submit + present）。重复调用安全（第二次为空操作）。
    pub(crate) fn present(&mut self) {
        if let Some(frame) = self.surface.take() {
            frame.present();
            self.presented_frames += 1;
        }
        self.presented = true;
    }

    /// **开一段 UI 录制**（feature = `ui`；`Frame::ui(theme)` 的实现）。
    ///
    /// 一帧可调用**任意多次**、位置随意（世界绘制之前 / 之间 / 之后）：本帧第一段
    /// **懒开场**（帧号 +1 / 命中区翻页 / 输入快照冻结 / 责任链种入），后续段复用帧级
    /// 暂存；每段析构时 `Ui::finish` 提交到 UI 层自己的 `Render2D`。
    ///
    /// 返回 [`UiSession`](crate::runtime::layers::ui::UiSession)：`Deref` 到 [`rjw_ui::Ui`]，
    /// 可直接透传给任意函数；段存活期间 `Ctx` / `Frame` 被借用（不能 `draw` / `text` /
    /// `submit`，编译期保证）。
    ///
    /// `panic`：feature `ui` 已编译且取到帧 ⇒ `window` / `text` / `ui_layer`（含渲染器）
    /// **必定**已接入（`attach` 无条件设置）——与 `Ctx::world_mut()` 同一不变式。
    #[cfg(feature = "ui")]
    pub(crate) fn ui_view(
        &mut self,
        theme: rjw_ui::Theme,
        base_layer: f64,
        debug_layout: bool,
    ) -> crate::runtime::layers::ui::UiSession<'_> {
        use rjw_ui::Ui;
        let scale_f = self.scale as f64;
        let scale = self.scale;
        let Self { window, keyboard, mouse, text, ui_layer, .. } = self;
        let window = window.as_ref().expect("Ctx 未接入窗口：无法录制 UI");
        let text = text.as_mut().expect("Ctx 未接入文本子系统：无法录制 UI");
        let layer = ui_layer.as_mut().expect("Ctx 未接入 UI 层：无法录制 UI");
        // 记录本段配置（帧收尾视图复用最近一次）——必须在取 `r2d`（会借用 `layer.r2d`）之前。
        layer.remember_view_config(theme.clone(), base_layer, scale, debug_layout);
        let r2d = layer.r2d.as_mut().expect("Ctx 未接入渲染上下文（headless）：无法录制 UI");
        let mut init = Ui::begin(window, text, &mut layer.state)
            .capture(mouse, keyboard)
            .theme(theme)
            .scale_factor(scale_f)
            .base_layer(base_layer);
        if debug_layout {
            init = init.debug_layout();
        }
        crate::runtime::layers::ui::UiSession::new(init.build(), r2d)
    }

    /// **UI 帧收尾**（feature = `ui`）：每帧一次、**幂等**——运行时在 UI 队列被提交之前
    /// （`Frame::submit` / `present` / `drop` / `Ctx::end_update`）调用。
    ///
    /// 做的事（把"一帧一份"的账补齐）：开一个**空段**（本帧已开场 ⇒ 不再开场）→
    /// `Ui::end_frame`（输入结算 / 焦点导航 + 描边 / 光标定夺 / 统计写回 / 帧级复位）→
    /// 提交描边命令到 UI 层渲染器。本帧没有任何 UI 段时是**空操作**（零开销）。
    #[cfg(feature = "ui")]
    pub(crate) fn ui_end_frame(&mut self) {
        use rjw_ui::Ui;
        let scale_f = self.scale as f64;
        let Self { window, keyboard, mouse, text, ui_layer, .. } = self;
        let (Some(window), Some(text), Some(layer)) = (window.as_ref(), text.as_mut(), ui_layer.as_mut())
        else {
            return;
        };
        if !layer.state.frame_open() {
            return;
        }
        let Some(r2d) = layer.r2d.as_mut() else {
            return;
        };
        let (theme, base_layer, debug_layout) =
            (layer.theme.clone(), layer.base_layer, layer.debug_layout);
        let mut init = Ui::begin(window, text, &mut layer.state)
            .capture(mouse, keyboard)
            .theme(theme)
            .scale_factor(scale_f)
            .base_layer(base_layer);
        if debug_layout {
            init = init.debug_layout();
        }
        let mut ui = init.build();
        let mut backend = crate::runtime::layers::ui_backend::Render2dUiBackend::new(r2d);
        ui.end_frame(&mut backend);
    }

    /// 文本子系统（feature = `text`；`Frame` 上的便捷入口在 P2 提供 `label` 链）。
    #[cfg(feature = "text")]
    #[inline]
    pub fn text_mut(&mut self) -> Option<&mut rjw_text::Text> {
        self.text.as_mut()
    }

    /// UI 跨帧状态（feature = `ui`，只读）。
    ///
    /// 用途：**取帧之前**读取上一帧诊断（`UiState::text_focus()` / `last_press_window()` /
    /// `occluded_hits()` / `stats`）——这些值必须在 `Ui::begin` 之前读（`begin` 会借用它）。
    #[cfg(feature = "ui")]
    #[inline]
    pub fn ui_state(&self) -> Option<&rjw_ui::UiState> {
        self.ui_layer.as_ref().map(|l| &l.state)
    }

    /// UI 跨帧状态（feature = `ui`，可变）。
    #[cfg(feature = "ui")]
    #[inline]
    pub fn ui_state_mut(&mut self) -> Option<&mut rjw_ui::UiState> {
        self.ui_layer.as_mut().map(|l| &mut l.state)
    }

    /// UI 主题（feature = `ui`）。
    #[cfg(feature = "ui")]
    #[inline]
    pub fn ui_theme(&self) -> Option<&rjw_ui::Theme> {
        self.ui_layer.as_ref().map(|l| &l.theme)
    }
}

/// 逃生口：原生帧资源。
pub struct Escape<'a> {
    frame: Option<&'a mut RenderFrame>,
}

impl Escape<'_> {
    /// 帧的 `CommandEncoder`（手动编码 / 后处理）。
    #[inline]
    pub fn encoder(&mut self) -> Option<&mut wgpu::CommandEncoder> {
        self.frame.as_mut().map(|f| f.escape_encoder())
    }

    /// 设备。
    #[inline]
    pub fn device(&self) -> Option<&wgpu::Device> {
        self.frame.as_deref().map(|f| f.device())
    }

    /// 队列。
    #[inline]
    pub fn queue(&self) -> Option<&wgpu::Queue> {
        self.frame.as_deref().map(|f| f.queue())
    }

    /// 以给定清屏开一个 pass 并执行自定义绘制（`hook` 在附件配置完成后调用）。
    pub fn pass(&mut self, clear: impl Into<Clear>, hook: impl FnOnce(&mut wgpu::RenderPass<'_>)) {
        if let Some(frame) = self.frame.as_deref_mut() {
            frame.pass(clear).escape(hook);
        }
    }
}

/// 便捷：把逻辑尺寸换算成 `Rect`（分屏常用）。
#[inline]
pub fn rect_from_size(size: impl Into<Vec2>) -> Rect {
    let s = size.into();
    Rect::new(0.0, 0.0, s.x, s.y)
}

/// 屏幕像素空间 → 某画面相机下的世界空间：`f(px) = cam.transform(px − region.center())`
/// （即 [`Camera2D::screen_to_world`]，取"每个画面各自的 `region.center()`"）。
///
/// 用于把**屏幕空间矩形**（画面底色 / 画面边框）表达成该相机下的世界矩形——
/// 相机平移 / 旋转 / 缩放时自动跟随。
///
/// ⚠ 组合顺序：`Transform2D::compose(&parent) == parent * self`，所以
/// `T(−c).compose(&cam.transform) == cam.transform * T(−c)`（先减中心、再套相机变换）。
pub(crate) fn screen_space_transform(cam: &Camera2D, region: Rect) -> Transform2D {
    Transform2D::IDENTITY
        .with_pos(-region.center())
        .compose(&cam.transform)
}

/// 画面边框配色（按画面序号循环；`AppConfig::viewport_borders` 开启时用）。
#[inline]
pub(crate) fn viewport_border_color(index: u32) -> Color {
    const PALETTE: [Color; 4] = [
        Color::rgb(1.0, 0.25, 0.25), // 红
        Color::rgb(0.25, 1.0, 0.35), // 绿
        Color::rgb(0.35, 0.6, 1.0),  // 蓝
        Color::rgb(1.0, 0.85, 0.25), // 黄
    ];
    PALETTE[(index as usize) % PALETTE.len()]
}

/// 画一圈画面描边（4 条 2px 实心边）到 **UI 层（上层）**：UI 层坐标 = 物理像素、左上原点，
/// 因此直接用画面矩形即可；只画边、不填充，不挡内容（层次用极大值压在最上）。
#[cfg(feature = "ui")]
fn draw_viewport_border(ui: &mut Render2D, region: Rect, index: u32) {
    let c = viewport_border_color(index);
    let w = 2.0_f32;
    let (x, y, rw, rh) = (region.x, region.y, region.w, region.h);
    let layers = 1.0e18_f64;
    let mut edge = |rect: SpriteRect| {
        ui.solid(rect).tint(c).layer(Layer::from(layers));
    };
    // 上 / 下 / 左 / 右
    edge(SpriteRect::new((x, y), (rw, w)));
    edge(SpriteRect::new((x, y + rh - w), (rw, w)));
    edge(SpriteRect::new((x, y), (w, rh)));
    edge(SpriteRect::new((x + rw - w, y), (w, rh)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **屏幕空间 → 世界**的组合顺序回归：`f(px)` 必须等于 `cam.screen_to_world(px)`
    /// （`compose(&parent) == parent * self`，写反了边框/底色就会跑到屏幕外）。
    #[test]
    fn screen_space_transform_matches_screen_to_world() {
        let region = Rect::new(960.0, 0.0, 960.0, 1080.0);
        let cam = Camera2D {
            region,
            transform: Transform2D::IDENTITY
                .with_pos(Vec2::new(120.0, 60.0))
                .with_rot(0.3)
                .with_scale(Vec2::splat(1.6)),
        };
        let tf = screen_space_transform(&cam, region);
        for px in [
            region.min(),
            region.center(),
            Vec2::new(region.x + region.w, region.y + region.h),
            Vec2::new(1200.0, 300.0),
        ] {
            let want = cam.screen_to_world(px);
            let got = tf.transform_point(px);
            assert!(
                (got - want).length() < 1e-3,
                "px={px:?} 期望 {want:?}，实际 {got:?}"
            );
        }
    }

    /// 画面边框配色按序号循环（4 色一循环）。
    #[test]
    fn viewport_border_color_cycles() {
        assert_eq!(viewport_border_color(1), viewport_border_color(5));
        assert_ne!(viewport_border_color(1), viewport_border_color(2));
    }
}
