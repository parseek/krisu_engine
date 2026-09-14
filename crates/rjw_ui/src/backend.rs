//! UI 绘制后端抽象：`rjw_ui` **只输出批次数据**，不再直接调用具体渲染器的方法。
//!
//! # 为什么需要这一层
//!
//! 旧实现里 `Ui::finish(&mut Render2D)` 直接调用 `Render2D::quads(..)` /
//! `Render2D::textures()` / `Render2D::gpu()`——UI 模块与 2D 批渲染器**紧耦合**，
//! 于是 UI 无法换后端（离线渲染 / 测试 / 自定义后端），也无法在没有 GPU 的情况下
//! 验证「一次交互产生几次 draw call」。
//!
//! 现在 `rjw_ui` 只认识本文件的 trait；所有绘制改为产出 [`UiBatch`]，
//! 由后端决定如何提交。桥接实现放在同时依赖两者的上层
//! （`rjw_krusie::runtime::layers::ui_backend`）。
//!
//! # 实例粒度与 DrawCall（本设计的核心取舍）
//!
//! 一个 [`UiBatch`] = **一个实例** = 后端的一次合批提交（通常 = 一次 draw call）。
//! 实例的划分粒度由 `rjw_ui` 的顶点收集阶段决定，当前策略是
//! **（窗口 × 纹理）**，并对超长顶点段切分：
//!
//! - **不按控件 / 容器切**：同一窗口内的所有控件、容器背景、文字共享顶点缓冲，
//!   合成一个批次 ⇒ 「一个窗口 ≈ 1~2 次 draw call」（字形图集页与程序化图集页各一次）。
//!   这是「尽量减少 DrawCall」的主要来源——**实例内容数量范围 = 整个窗口**。
//! - **按窗口切是必需的**：批次携带**窗口级** `transform`（屏幕固定变换 + 窗口 FX
//!   绕锚点旋转）与**窗口级** `tint`。若把全部窗口合成一批，就必须把这两者烘进顶点，
//!   于是窗口 FX 动画（位移 / 缩放 / 旋转 / 染色）每帧都要重建整窗顶点，
//!   摧毁现有的窗口顶点缓存（`UiState::window_quads`）。
//! - **按纹理切是必需的**：一次 draw call 只能绑定一个纹理（bind group）。
//! - **超长切分**：单段顶点数超过 `MAX_UI_SEG_VERTS` 时切段，避免越 u16 索引上限。
//!
//! 因此 [`UiBatchSource::elements`] 记录「本批次覆盖了多少个 UI 元素（控件）」——
//! 它是上述取舍的**可观测指标**，也是回归测试的断言对象。
//!
//! # 已知边界
//!
//! 顶点格式 [`VertexP3U2C4`] 与 `rjw_atlas::AtlasRegion` 仍来自
//! `rjw_2d_render` / `rjw_atlas`，因此 `rjw_ui` 目前仍依赖这两个 crate
//! （`rjw_text` 也在其公开面上重导出 `Layer` / `SpriteRect`）。
//! 本次解耦消除的是**绘制调用**层的耦合；把顶点格式下沉到 `rjw_render`
//! 以彻底移除该依赖，属于后续独立改动。

use std::sync::Arc;

use rjw_color::Color;
use rjw_render::TextureWrapped;
use rjw_transform::Transform2D;

pub use rjw_2d_render::VertexP3U2C4;

/// 一个三角形的三个顶点索引（相对**同一批次**的 [`UiBatch::vertices`]）。
pub type Tri = [u16; 3];

/// 一个绘制批次 = 一个实例 = 后端的一次合批提交。
///
/// 顶点已是**最终屏幕物理像素坐标**；`transform` / `tint` 保留为**实例级**数据，
/// 使窗口 FX 动画不需要重建顶点缓冲（见模块文档）。
///
/// 注：未派生 `Debug`（`TextureWrapped` / `wgpu::Texture` 不实现 `Debug`）；
/// 调试请读 `vertices.len()` / `layer` / `source` 等字段。
#[derive(Clone)]
pub struct UiBatch {
    /// 所在页纹理（`Arc` 共享；UI 不关心它来自字形图集还是程序化图集）。
    pub texture: Arc<TextureWrapped>,
    /// 顶点数据（屏幕物理像素）。
    pub vertices: Vec<VertexP3U2C4>,
    /// 三角形索引（相对 `vertices`；`u16` ⇒ 单段顶点数必须 ≤ 65535）。
    ///
    /// **非空**：UI 全程直出三角形（圆角 + 羽化由 CPU 镶嵌产生，见
    /// `crate::tess`），四边形也只是"4 顶点 + 2 三角形"的退化情形。
    ///
    /// 兼容：允许为空——此时后端应把顶点按「每 4 个一组、顺序 TL,TR,BL,BR」的
    /// 旧四边形约定补出索引（`rjw_krusie` 的桥接后端即如此回退），使外部
    /// `UiBackend` 实现者仍可只产出顶点。
    pub indices: Vec<Tri>,
    /// 实例变换（屏幕固定 + 窗口 FX 组合后的结果）。
    pub transform: Transform2D,
    /// 实例颜色（窗口 FX tint；批内顶点色已含控件自身 tint，这里是**整段**染色）。
    pub tint: Color,
    /// 排序层级（由 UI 决定，后端**不得**重排批次顺序）。
    pub layer: f64,
    /// 批次来源（实例用户数据）。
    pub source: UiBatchSource,
}

/// 批次来源：本实例覆盖了哪些 UI 实例（窗口 / 元素）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiBatchSource {
    /// 所属窗口 id（非窗口内容 = 0）。
    pub window: u32,
    /// 本批次覆盖的**元素（控件）个数** —— 实例内容数量范围的度量。
    pub elements: u32,
    /// 是否为调试图元批次（恒最后提交）。
    pub debug: bool,
}

/// UI 绘制后端。
///
/// `rjw_ui` 只经此接口与渲染器交互。实现者负责：按 uid 解析纹理、把批次写进自己的
/// 命令队列。
///
/// **只有两个方法**：UI 的绘制输出就是「纹理 + 顶点」，不需要后端参与纹理生成——
/// 矩形渐变已改为四角顶点色（见 [`crate::Gradient`]），不再有程序化渐变纹理。
pub trait UiBackend {
    /// 按 uid 解析纹理（UI 内部只持 uid，不持纹理句柄）。
    fn texture(&self, uid: u64) -> Option<Arc<TextureWrapped>>;

    /// 提交一个批次。**调用顺序即绘制顺序**，后端不得重排。
    fn submit(&mut self, batch: UiBatch);
}

/// 记录型后端（**纯 CPU，无需 GPU**）：把批次收集进 `Vec`，供测试断言
/// 「一次交互产生几次 draw call」「实例覆盖了多少控件」。
///
/// 这是把「尽量减少 DrawCall」从口头承诺变成**可回归断言**的机制。
#[derive(Default)]
pub struct RecordingBackend {
    /// 已提交的批次（顺序 = 绘制顺序）。
    pub batches: Vec<UiBatch>,
    /// 纹理 uid → 纹理 的预置映射（测试注入；无 GPU 时可为空）。
    pub textures: std::collections::HashMap<u64, Arc<TextureWrapped>>,
}

impl RecordingBackend {
    /// 批次数量 = 本帧的 draw call 数（每个批次一次合批提交）。
    #[inline]
    pub fn draw_calls(&self) -> usize {
        self.batches.len()
    }

    /// 本帧全部批次合计覆盖的元素（控件）数。
    #[inline]
    pub fn total_elements(&self) -> u32 {
        self.batches.iter().map(|b| b.source.elements).sum()
    }

    /// 本帧全部批次的顶点总数。
    #[inline]
    pub fn total_vertices(&self) -> usize {
        self.batches.iter().map(|b| b.vertices.len()).sum()
    }

    /// 本帧全部批次的三角形总数。
    #[inline]
    pub fn total_triangles(&self) -> usize {
        self.batches.iter().map(|b| b.indices.len()).sum()
    }
}

impl UiBackend for RecordingBackend {
    fn texture(&self, uid: u64) -> Option<Arc<TextureWrapped>> {
        self.textures.get(&uid).cloned()
    }

    fn submit(&mut self, batch: UiBatch) {
        self.batches.push(batch);
    }
}
