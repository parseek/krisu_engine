//! `rjw_krusie` —— krusie 引擎的**统一入口**（聚合 crate，不含实现）。
//!
//! 目标：整套库「一行起步」——**happy path 走 prelude，低层 / 命名冲突走命名空间**。
//!
//! ```no_run
//! use rjw_krusie::prelude::*;
//! ```
//!
//! # 两层结构
//!
//! | 层 | 内容 | 用法 |
//! |---|---|---|
//! | [`prelude`] | 常用类型（应用骨架 / 绘制 / 相机 / 文本 / UI / 图集 / 瓦片） | `use rjw_krusie::prelude::*;` |
//! | 命名空间 | 低层类型、自由函数、冲突名（各自对应原 crate） | `rjw_krusie::ui::Window`、`rjw_krusie::atlas::RegionRef`、`rjw_krusie::collision::move_and_collide` |
//!
//! 命名空间同时保留**原 crate 名**与**短别名**，两种写法等价：
//! `rjw_krusie::rjw_ui::Ui` == `rjw_krusie::ui::Ui`。
//!
//! # 命名冲突约定
//!
//! `prelude` **不含** `Window` / `Size` / 引擎 DIP 类型（`rjw_transform::LogicalSize` 等）：
//! winit 侧同名类型在应用骨架里更常用，而引擎自家 DIP 类型与 UI 布局枚举属于进阶用法。
//! 需要时写命名空间路径即可——**不做 `as` 改名**（避免同一类型出现两个名字）。
//!
//! | 想要 | 写法 |
//! |---|---|
//! | 应用骨架的 winit `Window` / `Size` | `rjw_krusie::main::Window` / `rjw_krusie::main::Size` |
//! | UI 容器 `Window` / 布局 `Size` | `rjw_krusie::ui::Window` / `rjw_krusie::ui::Size` |
//! | 引擎 DIP 尺寸/位置 | `rjw_krusie::transform::{LogicalSize, PhysicalSize, LogicalPosition, PhysicalPosition}` |
//!
//! # 裁剪依赖
//!
//! 只做 2D 绘制（不要 UI / 文本 / 图集 / 瓦片）时可关闭默认 feature：
//!
//! ```toml
//! rjw_krusie = { version = "0.2", default-features = false }
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

/// 应用入口：`App` / `run_app` / `MainContext` / winit 重导出。
pub use rjw_main as main;
/// 底层 GPU：`RenderContext` / 纹理 / 网格 / `wgpu` 重导出。
pub use rjw_render as gpu;
/// 2D 批渲染器：`Render2D` / `SpriteRect` / `Draw2D` / 排序 / 剔除。
pub use rjw_2d_render as render2d;
/// 变换与相机：`Transform2D` / `Camera2D` / `Rect` / `glam` 重导出。
pub use rjw_transform as transform;
/// 颜色：`Color` / `ColorF64`。
pub use rjw_color as color;
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

// ─── Prelude ──────────────────────────────────────────────────

/// 整套库的 happy path：一行导入即可写完整应用。
///
/// 只放**常用类型**；低层机制（`Draw2D`、`SortKey`、`VertexP3U2C4`…）、自由函数与
/// 冲突名请走命名空间（见 crate 文档的「命名冲突约定」）。
pub mod prelude {
    // ── 应用骨架（winit 事件循环 + 输入 + 计时）──
    pub use crate::main::{
        ActiveEventLoop, App, DeltaTimer, DeviceEvent, EventLoop, EventLoopError, KeyCode,
        KeyboardInput, LogicalPosition, LogicalSize, MainContext, MouseButton, MouseInput,
        PhysicalPosition, PhysicalSize, WindowAttributes, WindowEvent, run_app,
    };

    // ── 底层 GPU / 资源注册表 ──
    pub use crate::gpu::{
        ArcTextureWrapped, MESHES, MeshData, RenderConfig, RenderContext, TEXTURES, wgpu,
    };

    // ── 2D 绘制（入口 / 状态 / 排序 / 剔除）──
    pub use crate::render2d::{
        AddressMode, BlendMode, ClearConfig, CompareFunc, Cull, CullMode, Culler, Edges,
        FilterMode, Layer, RStates, Render2D, SortMode, SpriteRect, SpriteRectPx,
    };

    // ── 颜色 ──
    pub use crate::color::{Color, ColorF64};

    // ── 变换 / 相机 / 几何 ──
    pub use crate::transform::{Camera2D, Mat4, Rect, Transform2D, Vec2, Vec3, Viewport, glam, vec2, vec3};

    // ── 图集（feature = atlas）──
    #[cfg(feature = "atlas")]
    pub use crate::atlas::{AtlasConfig, AtlasRegion, DynamicAtlas};

    // ── 文本（feature = text）──
    #[cfg(feature = "text")]
    pub use crate::text::{
        Align, CachePolicy, LineSpace, Text, TextBuffer, TextLayout, TextRender, TextStyle,
    };

    // ── UI（feature = ui）──
    #[cfg(feature = "ui")]
    pub use crate::ui::{
        Anchor, Button, Checkbox, Divider, FontModal, Grid, IdAbsolute, IdRelative, Label,
        NumberInput, Pack, PackSide, Panel, PanelStyle, Response, Slider, Theme, Ui, UiAdd,
        UiInit, UiState, UiStats, Widget, WindowClamp, WindowFx,
    };

    // ── 瓦片地图（feature = tilemap）──
    #[cfg(feature = "tilemap")]
    pub use crate::tilemap::{Tile, TileMap};
}
