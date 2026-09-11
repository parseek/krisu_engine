//! 应用配置：窗口 + 渲染 + 清屏 + 后台策略 + 退出策略。

pub use rjw_render::{Clear, RenderConfig, Vsync};

/// 取不到表面（最小化 / 遮挡 / 超时 / 丢失）时的迭代策略。
///
/// 动机：`App::update` 在无帧时**照常执行**（允许后台模拟），但 `ControlFlow::Poll`
/// 会让事件循环空转烧 CPU。默认 `Throttle(50)`：仅在**连续无帧**时退避；一旦恢复出帧
/// 立刻回到 `Poll`（满帧率）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    /// 始终忙等（`ControlFlow::Poll`）——不推荐，仅在需要「无帧也跑满速」时用。
    Poll,
    /// 连续无帧时退避 `ms` 毫秒（默认 50）。
    Throttle(u32),
    /// 连续无帧时进入 `Wait`（由事件唤醒）。
    Wait,
}

impl Default for Background {
    fn default() -> Self {
        Background::Throttle(50)
    }
}

impl Background {
    /// 退避毫秒数（`Poll` / `Wait` 返回 `None`）。
    #[inline]
    pub const fn throttle_ms(self) -> Option<u32> {
        match self {
            Background::Throttle(ms) => Some(ms),
            _ => None,
        }
    }
}

/// **画面边框**（调试视觉辅助）：多画面（分屏 / 画中画）时给每个画面矩形画一圈
/// 描边 + 左上角标注 `#序号 宽×高`，用来**区分各个分屏**分别画了什么。
///
/// 关闭时零开销（不产生任何绘制命令）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewportBorders {
    /// 关闭（默认）。
    #[default]
    Off,
    /// 开启：按画面顺序自动配色（红 / 绿 / 蓝 / 黄循环）。
    On,
}

impl ViewportBorders {
    /// 是否开启。
    #[inline]
    pub const fn is_on(self) -> bool {
        matches!(self, ViewportBorders::On)
    }
}

/// 应用配置（`App::config` 返回）。
///
/// 尺寸用**逻辑像素 f32**（不暴露 winit dpi 类型；DPI 换算见 `Ctx::scale`）。
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// 窗口标题。
    pub title: String,
    /// 初始窗口尺寸（逻辑像素，`(宽, 高)`）。
    pub size: (f32, f32),
    /// 渲染上下文配置（后端 / vsync / 深度格式）。
    pub render: RenderConfig,
    /// 自动提交（应用从不 `submit`）时使用的清屏意图。
    pub clear: Clear,
    /// 无帧时的迭代策略。
    pub background: Background,
    /// **画面边框**（调试：区分多画面 / 分屏；默认关闭）。
    pub viewport_borders: ViewportBorders,
    /// 自动退出：跑满 N 帧迭代后退出（冒烟测试 / 自动演示用；`None` = 不自动退出）。
    pub exit_after_frames: Option<u64>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            title: super::default_title(),
            size: (1280.0, 720.0),
            render: RenderConfig::default(),
            clear: Clear::default(),
            background: Background::default(),
            viewport_borders: ViewportBorders::default(),
            exit_after_frames: None,
        }
    }
}

impl AppConfig {
    /// 以标题构造（其余为默认值）。
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), ..Self::default() }
    }

    /// 初始窗口尺寸（逻辑像素）。
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = (width, height);
        self
    }

    /// 垂直同步策略。
    pub fn vsync(mut self, vsync: Vsync) -> Self {
        self.render.vsync = vsync;
        self
    }

    /// 自动提交时的清屏意图（接受 `Clear` / `Color` / `ColorF64`）。
    pub fn clear(mut self, clear: impl Into<Clear>) -> Self {
        self.clear = clear.into();
        self
    }

    /// 无帧时的迭代策略。
    pub fn background(mut self, background: Background) -> Self {
        self.background = background;
        self
    }

    /// **画面边框**（调试辅助）：多画面 / 分屏时给每个画面画一圈描边 + 标注序号与尺寸。
    pub fn viewport_borders(mut self, borders: ViewportBorders) -> Self {
        self.viewport_borders = borders;
        self
    }

    /// 跑满 `frames` 次迭代后自动退出（冒烟测试用）。
    pub fn exit_after_frames(mut self, frames: u64) -> Self {
        self.exit_after_frames = Some(frames);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rjw_color::Color;

    #[test]
    fn builder_sets_fields() {
        let c = AppConfig::new("t").size(800.0, 600.0).vsync(Vsync::Off).clear(Color::RED).exit_after_frames(3);
        assert_eq!(c.title, "t");
        assert_eq!(c.size, (800.0, 600.0));
        assert_eq!(c.render.vsync, Vsync::Off);
        assert_eq!(c.clear, Clear::color(Color::RED));
        assert_eq!(c.exit_after_frames, Some(3));
    }

    #[test]
    fn background_default_is_throttled() {
        assert_eq!(Background::default(), Background::Throttle(50));
        assert_eq!(Background::default().throttle_ms(), Some(50));
        assert_eq!(Background::Poll.throttle_ms(), None);
        assert_eq!(Background::Wait.throttle_ms(), None);
    }
}
