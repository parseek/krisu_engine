//! `Frame`：**本帧渲染面**（画面矩形 / 绘制 / 提交 / 呈现）。
//!
//! 由 [`Ctx::frame`](crate::runtime::Ctx::frame) 给出：拿不到表面时为 `None`，应用据此守卫
//! （`let Some(mut f) = ctx.frame() else { return };`）——**无帧不执行渲染代码**。
//!
//! `Frame` 经 [`Deref`] 暴露 `Ctx` 的只读面（`f.key(..)` / `f.dt()` / `f.mouse()`）；
//! 取到帧之后请一律经 `f` 访问，不要再用 `ctx`（借用会冲突）。

use std::ops::Deref;

use rjw_2d_render::Render2D;
use rjw_render::wgpu;
use rjw_render::Clear;
use rjw_transform::{Camera2D, Rect};

use crate::runtime::ctx::Ctx;

/// 本帧渲染面。
pub struct Frame<'a> {
    ctx: &'a mut Ctx,
    /// **是否已显式 present**（[`Frame::present`]）。`submit` **不**置位：提交只负责把队列
    /// 落成一个 pass，呈现统一由析构收尾（`Ctx::present` 幂等；一帧多画面只能 present 一次）。
    presented: bool,
}

impl<'a> Frame<'a> {
    pub(crate) fn new(ctx: &'a mut Ctx) -> Self {
        Self { ctx, presented: false }
    }

    /// 本帧的 raw `RenderFrame`（逃生口；正常路径不需要）。
    #[inline]
    pub fn raw(&mut self) -> Option<&mut rjw_render::RenderFrame> {
        self.ctx.surface.as_mut()
    }

    /// 表面尺寸（物理像素）。
    #[inline]
    pub fn size(&self) -> (u32, u32) {
        self.ctx.size
    }

    /// 当前画面矩形（默认 = 全窗口；随窗口缩放自动更新）。
    #[inline]
    pub fn region(&self) -> Rect {
        self.ctx.region()
    }

    /// 设定本画面的屏幕矩形（分屏 / 画中画）。
    #[inline]
    pub fn set_region(&mut self, region: Rect) {
        self.ctx.region = Some(region);
    }

    /// 世界层渲染器（录制世界坐标命令）。
    #[inline]
    pub fn draw(&mut self) -> &mut Render2D {
        self.ctx.world_mut()
    }

    /// UI 层渲染器（录制屏幕坐标命令；排序已关闭，UI 自行管理顺序）。
    #[inline]
    pub fn draw_ui(&mut self) -> &mut Render2D {
        self.ctx.ui_mut()
    }

    /// 提交当前队列为**一个画面**（= 一个 pass）：写入画面矩形 → 取 VP → 开 pass →
    /// 提交世界层与 UI 层 → 清空队列。
    ///
    /// - `cam`：本画面的相机；`cam.region` 会被写成当前画面矩形（相机自己存视口）；
    /// - 一帧内可多次调用 = 多个画面，每个画面一个独立 VP 槽（互不干扰）；
    /// - **不**在此 present：呈现由 `Frame` 析构统一收尾（`Ctx::present`，幂等）——
    ///   多次 `submit` 只 present 一次。`Ctx::submit_with` 内部已标记"本帧由应用接管"，
    ///   帧尾**不会**再补一次配置清屏的 pass。
    pub fn submit(&mut self, cam: &mut Camera2D, clear: impl Into<Clear>) {
        let region = self.ctx.region();
        cam.set_region(region);
        let cam = *cam;
        self.ctx.submit_with(clear.into(), Some(&cam));
    }

    /// 再提交一次（保留颜色与深度）：用于「UI 与上一个画面同帧但独立 pass」。
    ///
    /// 若本帧还没有任何画面，等价于用 `Clear::Keep` 提交一次。同样不在此 present。
    pub fn submit_keep(&mut self) {
        self.ctx.submit_with(Clear::Keep, None);
    }

    /// 呈现本帧（提交 encoder + present）。可省略：`Frame` 析构时自动呈现。
    ///
    /// 呈现**之前**会先画「画面边框」overlay（若 `AppConfig::viewport_borders` 开启）——
    /// 它必须是本帧的**最后一个 pass**（视口 = 整屏），所以挂在帧末收尾而不是画面里。
    pub fn present(&mut self) {
        self.ctx.submit_viewport_borders();
        self.ctx.present();
        self.presented = true;
    }

    /// 请求本帧结束后退出（等价 `Ctx::exit`；持帧期间也能调用，无需先 drop 帧）。
    #[inline]
    pub fn exit(&mut self) {
        self.ctx.exit();
    }

    /// **调试注入鼠标**（同 [`Ctx::debug_inject_mouse`]；持帧期间用这个）。
    ///
    /// 脚本化复现交互（拖动 / 点击 / 命中穿透）——不需要真实鼠标，见 `docs/DEBUGGING.md`。
    #[inline]
    pub fn debug_inject_mouse(&mut self, pos_px: impl Into<rjw_transform::Vec2>, down: bool) {
        self.ctx.debug_inject_mouse(pos_px, down);
    }

    /// 逃生口：直接写帧的 `CommandEncoder`（自定义编码 / 后处理）。
    ///
    /// 返回 `None` 表示当前无帧。
    pub fn escape_encoder(&mut self) -> Option<&mut wgpu::CommandEncoder> {
        self.ctx.surface.as_mut().map(|f| f.escape_encoder())
    }

    /// 逃生口：拿到设备 / 队列（自定义管线创建用）。
    pub fn escape_device_queue(&mut self) -> Option<(&wgpu::Device, &wgpu::Queue)> {
        self.ctx.surface.as_mut().map(|f| (f.device(), f.queue()))
    }

    /// UI（feature = `ui`）：自动完成 `Ui::begin` / 输入快照 / 主题 / DPI / `finish`。
    ///
    /// ```ignore
    /// f.ui(Theme::dark(), |ui| {
    ///     ui.window("inv").pos(Vec2::new(300.0, 90.0)).show(|w| { w.label("背包"); });
    /// });
    /// ```
    #[cfg(feature = "ui")]
    pub fn ui(&mut self, theme: rjw_ui::Theme, f: impl FnOnce(&mut rjw_ui::Ui<'_>)) {
        self.ctx.run_ui(theme, f);
    }

    /// 文本（feature = `text`）：闭包拿到运行时文本上下文 [`TextCtx`]
    /// （同时持有 `&mut Text` 与 `&mut Render2D`，两者需同时可变借用）。
    ///
    /// **世界层**文本（`TextCtx::label(..)` 返回的 [`Label`](rjw_text::Label) 已绑定世界层
    /// `Render2D`，因此终点 `draw(layer)` **只收 1 个参数**）：
    ///
    /// ```ignore
    /// f.text(|t| {
    ///     t.label("HP 100").size(16.0).at(world_pos).draw(10.0);
    /// });
    /// ```
    ///
    /// 屏幕固定 UI 文本请用 [`Self::text_ui`]（绑定 UI 层渲染器）。
    /// 返回 `None` 表示当前无文本子系统（未 `attach`）或无世界层渲染器。
    #[cfg(feature = "text")]
    pub fn text<R>(&mut self, f: impl FnOnce(&mut rjw_text::TextCtx<'_>) -> R) -> Option<R> {
        let ctx = &mut *self.ctx;
        let text = ctx.text.as_mut()?;
        let r2d = ctx.world.as_mut()?;
        let mut tc = rjw_text::TextCtx::new(text, r2d);
        Some(f(&mut tc))
    }

    /// 文本（feature = `text`）：同 [`Self::text`]，但绑定 **UI 层**渲染器
    /// （`Frame::draw_ui()` 的那个）——**屏幕固定 UI 文字**用。
    ///
    /// 坐标系与 [`Self::draw_ui`] 一致：**物理像素、左上原点**（UI 层用 identity 相机提交）。
    /// 与 `f.ui(..)`（rjw_ui 窗口）同层，且在**世界层之后**提交 ⇒ UI 文字不会被世界内容遮挡。
    ///
    /// ```ignore
    /// f.draw_ui().solid(SpriteRect::new((16.0, 16.0), (200.0, 24.0)));   // 物理像素
    /// f.text_ui(|t| { t.label("HP 100").size(16.0 * scale).at((16.0, 16.0)).draw(0.0); });
    /// ```
    #[cfg(feature = "text")]
    pub fn text_ui<R>(&mut self, f: impl FnOnce(&mut rjw_text::TextCtx<'_>) -> R) -> Option<R> {
        let ctx = &mut *self.ctx;
        let text = ctx.text.as_mut()?;
        let r2d = ctx.ui_2d.as_mut()?;
        let mut tc = rjw_text::TextCtx::new(text, r2d);
        Some(f(&mut tc))
    }
}

impl Deref for Frame<'_> {
    type Target = Ctx;
    #[inline]
    fn deref(&self) -> &Ctx {
        self.ctx
    }
}

impl Drop for Frame<'_> {
    fn drop(&mut self) {
        if !self.presented {
            // 未显式 present：帧尾收尾（画面边框 overlay → 自动 clear → present）。
            self.ctx.submit_viewport_borders();
            self.ctx.present();
        }
    }
}
