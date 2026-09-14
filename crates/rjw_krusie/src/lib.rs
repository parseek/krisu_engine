//! `rjw_krusie` —— krusie 引擎的**统一入口**（聚合 + 模块化运行时）。
//!
//! # 一行起步
//!
//! ```no_run
//! use rjw_krusie::prelude::*;
//!
//! #[derive(Default)]
//! struct Game;
//!
//! impl App for Game {
//!     fn config(&self) -> AppConfig { AppConfig::new("my game").size(1280.0, 720.0) }
//!     fn update(&mut self, ctx: &mut Ctx) {
//!         if ctx.key(KeyCode::Escape).down_edge() { ctx.exit(); }
//!         let Some(mut f) = ctx.frame() else { return };   // 无帧不执行渲染代码
//!         f.draw().solid(SpriteRect::new((-50.0, -50.0), (100.0, 100.0))).tint(Color::GREEN).layer(0.0);
//!         // 呈现由 `Frame` 析构统一收尾（`submit` 只落 pass，不 present）
//!     }
//! }
//!
//! fn main() -> Result<(), RunError> { run(Game) }
//! ```
//!
//! # 三层结构
//!
//! | 层 | 内容 | 用法 |
//! |---|---|---|
//! | [`runtime`] | 应用骨架（`App` / `Ctx` / `Frame` / `Gfx` / `AppConfig`） | `run(Game)` |
//! | [`prelude`] | happy path 一行导入（**零命名冲突**） | `use rjw_krusie::prelude::*;` |
//! | 命名空间 | 低层类型、自由函数、约定名（各对应原 crate） | `rjw_krusie::render2d::Draw2D` |
//!
//! 命名空间同时保留**原 crate 名**与**短别名**，两种写法等价：
//! `rjw_krusie::rjw_ui::Ui` == `rjw_krusie::ui::Ui`。
//!
//! 设计契约（分层 / 规则 / 责任表 / 简并表 / 旧→新映射）见仓库 `docs/API_DESIGN.md`。
//!
//! # 裁剪依赖
//!
//! 只做 2D 绘制（不要文本 / UI / 图集 / 瓦片 / 碰撞）时可关闭默认 feature：
//!
//! ```toml
//! rjw_krusie = { version = "0.3", default-features = false }
//! ```

// ─── 命名空间（原 crate 名） ──────────────────────────────────

pub use rjw_2d_render;
#[cfg(feature = "atlas")]
pub use rjw_atlas;
#[cfg(feature = "collision")]
pub use rjw_collision;
pub use rjw_color;
pub use rjw_main;
pub use rjw_render;
#[cfg(feature = "text")]
pub use rjw_text;
#[cfg(feature = "tilemap")]
pub use rjw_tilemap;
pub use rjw_transform;
#[cfg(feature = "ui")]
pub use rjw_ui;

// ─── 命名空间（短别名，等价写法） ─────────────────────────────

/// 应用骨架（`App` / `run` / `Ctx` / `Frame` / `Gfx` / `AppConfig`）。
pub use runtime as app;
/// 底层 GPU：`RenderContext` / `Gpu` / 纹理 / 网格 / `wgpu` 重导出。
pub use rjw_render as gpu;
/// 2D 批渲染器：`Render2D` / `SpriteRect` / `Draw2D` / 排序 / 剔除。
pub use rjw_2d_render as render2d;
/// 变换与相机：`Camera2D` / `Transform2D` / `Rect` / `glam` 重导出。
pub use rjw_transform as transform;
/// 颜色：`Color` / `ColorF64`。
pub use rjw_color as color;
/// winit 适配（低层）；happy path 用 [`runtime`]。
pub use rjw_main as main;
#[cfg(feature = "atlas")]
pub use rjw_atlas as atlas;
#[cfg(feature = "text")]
pub use rjw_text as text;
#[cfg(feature = "ui")]
pub use rjw_ui as ui;
#[cfg(feature = "tilemap")]
pub use rjw_tilemap as tilemap;
#[cfg(feature = "collision")]
pub use rjw_collision as collision;

// ─── 运行时（门面顶层） ───────────────────────────────────────

pub mod runtime;

/// 应用入口 trait（同 [`app::App`]）。
pub use runtime::App;

// ─── Prelude ──────────────────────────────────────────────────

/// 整套库的 happy path：一行导入即可写完整应用，**零命名冲突**（见 `docs/API_DESIGN.md` §6）。
///
/// 低层机制（`Draw2D` / `SortKey` / `VertexP3U2C4` / 注册表 / winit dpi 类型）
/// 请走命名空间（`rjw_krusie::render2d::…` / `rjw_krusie::main::…`）。
pub mod prelude {
    // ── 运行时（应用骨架）──
    pub use crate::runtime::{
        run, run_with, App, AppConfig, AppInitError, Background, Clear, Ctx, Escape, Frame,
        FrameSource, Gfx, Never, RenderConfig, RunError, ViewportBorders, Vsync, WindowId,
    };

    // ── 2D 绘制 ──
    pub use crate::render2d::{
        AddressMode, BlendMode, CompareFunc, Cull, CullMode, DepthState, Edges, FilterMode,
        FrontFaceWinding, Layer, PolygonMode, RasterState, Render2D, SamplerDesc, SortMode,
        SpriteRect, StencilState,
    };
    // 全局默认状态（唯一状态语言）
    pub use crate::render2d::RStates;

    // ── 资源句柄 / 参数对象 ──
    pub use crate::gpu::{ArcTextureWrapped, MeshId, MeshSpec, Rgba8};

    // ── 颜色 ──
    pub use crate::color::{Color, ColorF64};

    // ── 变换 / 相机 / 几何（引擎自有 dpi 类型也在此）──
    pub use crate::transform::{
        glam, mat4, vec2, vec3, Camera2D, LogicalPosition, LogicalSize, Mat4, PhysicalPosition,
        PhysicalSize, Rect, Transform2D, Vec2, Vec3, Vec4,
    };

    // ── 输入 ──
    pub use crate::main::{EventLoopError, KeyCode, KeyState, KeyboardInput, MouseButton};
    pub use crate::main::{MouseInput, ScrollDelta};

    // ── 图集（feature = atlas）──
    #[cfg(feature = "atlas")]
    pub use crate::atlas::{
        AtlasConfig, AtlasRegion, AtlasSprite, AtlasStats, DynamicAtlas, InsertOpts, RegionRef,
    };

    // ── 文本（feature = text）──
    #[cfg(feature = "text")]
    pub use crate::text::{
        Align, CachePolicy, Gradient, GradientAxis, GradientMode, Label, LineSpace, Text, TextBuffer,
        TextCtx, TextStyle,
    };

    // ── UI（feature = ui）──
    #[cfg(feature = "ui")]
    pub use crate::ui::{
        Anchor, Button, Checkbox, Child, ColorPicker, Divider, Level, NumberInput, PackSide,
        Placement, Resize,
        Slider, Theme, Ui, UiAdd, UiState, UiStats, WindowClamp, WindowFx,
    };

    // ── 瓦片地图（feature = tilemap）──
    #[cfg(feature = "tilemap")]
    pub use crate::tilemap::{Tile, TileMap};
}
