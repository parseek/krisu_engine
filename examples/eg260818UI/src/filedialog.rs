//! **文件导入 / 导出**（系统文件选择器 → 字节 → 引擎资源）：图片当背景纹理、字体进运行时
//! 字体库、**主题走 TOML 序列化**（导入 = 在当前主题上合并覆盖，导出 = 全量落盘）。
//!
//! # 为什么要单独一个模块
//!
//! 文件选择器是**应用侧**的事（引擎不背 GUI 依赖）：`rfd` 只出现在示例的 `Cargo.toml`
//! 里。引擎侧提供的只是几条"字节 / 文本 → 资源"的通路，本模块把它们收在一处，并解决三件
//! 容易做错的事：
//!
//! 1. **对话框必须帧外弹**：`rfd` 的选择器是**阻塞**调用，而 UI 录制期 `f` 借着
//!    `ctx`（还占着那一帧）。所以按钮点击只**记一个待办**，帧末 / 下一帧开头再弹
//!    （见 `UiApp::update` 里的 `import_request` / `theme_io`）；
//! 2. **与 CLI 参数复用同一条通路**：`--image` / `--font-file` / `--theme`（启动时加载）
//!    与「导入图片…」/「导入字体…」/「导入主题…」（运行时加载）走**同一个**
//!    `decode_image` / `apply_font` / `load_theme_onto` ⇒ 逻辑只有一份；
//!    `--sim-import` / `--sim-theme` 也能脚本化验证（无需真人点选择器）；
//! 3. **失败不致命**：读不到 / 解不开 / 字体没有可用字面 / TOML 不合法 ⇒ 返回
//!    `Err(String)`，调用方把它显示在状态行上，画面照旧（示例要能离线裸跑）。
//!
//! # 纹理通路
//!
//! 图片 → [`decode_image`]（RGBA8）→ `Render2D::gpu().texture(..)`（注册进**共享纹理表**，
//! 世界层与 UI 层是同一个 `Arc<TextureRegistry>`）→ 拿到 `uid` → `ImageBg::new(uid, size)`
//! → `PanelStyle::with_bg_image(..)`。UI 后端按 `uid` 解析纹理，与启动期加载完全一致。

use std::path::{Path, PathBuf};

use rjw_krusie::prelude::*;
use rjw_krusie::text::Text;
use rjw_krusie::ui::ImageBg;

/// 图片扩展名（选择器过滤器 + `--sim-import` 判类型用）。
pub const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "gif", "webp"];
/// 字体扩展名（`ttf` / `otf` / `ttc`）。
pub const FONT_EXTS: [&str; 3] = ["ttf", "otf", "ttc"];
/// **主题**扩展名（TOML 文本；`txt` 也收 —— 手写的小文件常叫 `.txt`）。
pub const THEME_EXTS: [&str; 2] = ["toml", "txt"];

/// 待处理的导入请求（按钮点击时记录，**帧外**执行）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImportKind {
    /// 图片 → 背景纹理（`ImageBg`）。
    Image,
    /// 字体 → 运行时字体库（成功后可切到它的族名）。
    Font,
    /// **主题** → TOML 文本，在**当前主题上合并覆盖**（见 [`load_theme_onto`]）。
    Theme,
}

/// 待处理的**导出**请求（按钮点击时记录，**帧外**执行——`rfd` 阻塞）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExportKind {
    /// 当前主题 → TOML 文件（[`save_theme`]）。
    Theme,
}

impl ImportKind {
    /// 扩展名过滤表（选择器用）。
    fn exts(self) -> &'static [&'static str] {
        match self {
            ImportKind::Image => &IMAGE_EXTS,
            ImportKind::Font => &FONT_EXTS,
            ImportKind::Theme => &THEME_EXTS,
        }
    }

    /// 按扩展名猜类型（`--sim-import <路径>` 用：脚本只给路径）。
    pub fn from_path(path: &Path) -> Option<ImportKind> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        if FONT_EXTS.contains(&ext.as_str()) {
            Some(ImportKind::Font)
        } else if IMAGE_EXTS.contains(&ext.as_str()) {
            Some(ImportKind::Image)
        } else if THEME_EXTS.contains(&ext.as_str()) {
            Some(ImportKind::Theme)
        } else {
            None
        }
    }
}

/// **弹系统文件选择器**（阻塞；用户取消 ⇒ `None`）。
///
/// `title` / 过滤器按类型给：只列该类型 ⇒ 选错类型的概率大幅下降（仍会校验，
/// 见 [`ImportKind::from_path`] 的调用点）。
pub fn pick(kind: ImportKind) -> Option<PathBuf> {
    let (title, filter) = match kind {
        ImportKind::Image => ("导入图片（当窗口背景纹理）", "图片"),
        ImportKind::Font => ("导入字体（ttf / otf / ttc）", "字体"),
        ImportKind::Theme => ("导入主题（TOML；在当前主题上合并）", "主题 TOML"),
    };
    rfd::FileDialog::new()
        .set_title(title)
        .add_filter(filter, kind.exts())
        .pick_file()
}

/// **弹系统"另存为"选择器**（阻塞；用户取消 ⇒ `None`）。
///
/// 写文件走 [`save_theme`]；与导入共用同一套扩展名与错误显示口径。
pub fn pick_save(kind: ExportKind) -> Option<PathBuf> {
    let (title, filter, exts, default_name) = match kind {
        ExportKind::Theme => (
            "导出主题（TOML）",
            "主题 TOML",
            THEME_EXTS.as_slice(),
            "theme.toml",
        ),
    };
    rfd::FileDialog::new()
        .set_title(title)
        .add_filter(filter, exts)
        .set_file_name(default_name)
        .save_file()
}

/// **主题 → TOML 文件**（[`Theme::to_toml`](rjw_krusie::ui::Theme::to_toml) + 写盘）。
///
/// 失败返回可直接显示的中文原因（状态行显示，画面照旧）。
pub fn save_theme(path: &Path, theme: &Theme) -> Result<(), String> {
    let text = theme.to_toml()?;
    std::fs::write(path, &text).map_err(|e| format!("主题写不进文件：{e}"))?;
    Ok(())
}

/// **TOML 文件 → 主题**：读盘 + 在 `base` 上**合并覆盖**（[`Theme::apply_toml`]）。
///
/// 为什么是"覆盖"而不是"整套替换"：手写的小文件通常只写要改的几行
/// （`gap = 12`），落到**当前主题**上才有意义；导出的**全量**文件同样能吃
/// （它带 `format_version` 头，未列出的字段本就不缺）。
pub fn load_theme_onto(path: &Path, base: &Theme) -> Result<Theme, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("主题文件读不了：{e}"))?;
    let mut t = base.clone();
    t.apply_toml(&text)?;
    Ok(t)
}


/// 解码图片文件为 **RGBA8**（`(像素, 宽, 高)`）。失败返回可直接显示的中文原因。
///
/// 与 `--image` 走同一函数 ⇒ 启动期加载与运行时导入的行为不可能分叉。
pub fn decode_image(path: &Path) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::open(path).map_err(|e| format!("图片打不开：{e}"))?;
    let img = img.to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err("图片尺寸为 0".to_owned());
    }
    Ok((img.into_raw(), w, h))
}

/// **注册图片纹理**并返回可用的 [`ImageBg`]（`name` 是调试标签，重复导入会各自注册一条）。
///
/// ⚠ 纹理表在**世界与 UI 两级渲染器之间共享**（`Arc<TextureRegistry>`）⇒ 从 `f.draw()`
/// 拿到的 `gpu()` 注册进去，UI 侧按 uid 一样解析得到。
pub fn import_image(gpu: &Gpu, name: &str, path: &Path) -> Result<ImageBg, String> {
    let (px, w, h) = decode_image(path)?;
    let tex = gpu.texture(name, Rgba8::new(&px, (w, h)));
    Ok(ImageBg::new(tex.uid, Vec2::new(w as f32, h as f32)))
}

/// **把字体文件加载进运行时字体库**，返回本次新增的**族名**。
///
/// 返回空 = 文件已加载，但这个族**本来就在库里**（`FontSystem::new()` 默认已经索引了
/// 系统字体 ⇒ 导入一个系统字体自己的文件就是这种情况）——这**不是**失败，调用方照常
/// 提示即可。真正的失败（读不到 / 没有可用字面）走 `Err`。
///
/// ⚠ 必须是**运行时**那一套文本子系统（`Ctx::text_mut()`）——UI 排版 / 字形图集都用它；
/// 应用自建的那套（`Gfx::text()`）UI 不会用（历史上"导入了字体但界面没变"就是这个）。
pub fn apply_font(text: &mut Text, path: &Path) -> Result<Vec<String>, String> {
    let data = std::fs::read(path).map_err(|e| format!("字体文件读不了：{e}"))?;
    Ok(text.load_font_data(data))
}

/// 人类可读的文件名（状态行显示用）。
pub fn file_label(path: &Path) -> String {
    path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}
