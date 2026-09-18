//! **UI 后端桥接**：把 `rjw_ui::UiBackend` 接到 `rjw_2d_render::Render2D`。
//!
//! `rjw_ui` 只输出 [`UiBatch`]（纹理 + 顶点 + 索引 + 实例变换 + 实例数据 + **批次
//! scissor**），本模块是**唯一**知道「批次要变成 `Render2D::mesh_indexed(..)` 调用」的
//! 地方。放在 `rjw_krusie` 是因为它同时依赖二者——`rjw_ui` 不反向依赖渲染器。

use std::sync::Arc;

use rjw_2d_render::{Layer, Render2D};
use rjw_color::Color;
use rjw_render::TextureWrapped;
use rjw_ui::{Tri, UiBackend, UiBatch};

/// 把 `Render2D` 适配成 UI 绘制后端。
pub struct Render2dUiBackend<'a> {
    r2d: &'a mut Render2D,
}

impl<'a> Render2dUiBackend<'a> {
    /// 借用本帧的 UI 层渲染器。
    #[inline]
    pub fn new(r2d: &'a mut Render2D) -> Self {
        Self { r2d }
    }
}

/// 旧四边形约定：顶点按 `[TL, TR, BL, BR]` 每 4 个一组，补出 2 个三角形。
///
/// 仅用于**索引为空**的批次——`rjw_ui` 自己的产出恒带索引（全程直出三角形）。
/// 保留这条回退是为让外部 `UiBackend` 实现者 / 旧录制数据仍能提交。
fn quad_indices(n_verts: usize) -> Vec<Tri> {
    let mut out = Vec::with_capacity(n_verts / 4 * 2);
    for b in (0..n_verts).step_by(4) {
        let b = b as u16;
        out.push([b, b + 1, b + 3]);
        out.push([b + 3, b + 2, b]);
    }
    out
}

impl UiBackend for Render2dUiBackend<'_> {
    fn texture(&self, uid: u64) -> Option<Arc<TextureWrapped>> {
        self.r2d.textures().get(uid)
    }

    fn submit(&mut self, batch: UiBatch<'_>) {
        // 一个批次 = 一个实例 = 一次 `mesh_indexed(..)` 提交（`Render2D` 内按
        // (layer, rstates, tex, transform, scissor) 继续合批；UI 层已关闭排序，
        // 保持提交顺序）。
        //
        // ⚠ `UiBatch` 的顶点/索引是**借用**的（阶段 9 起）——`mesh_indexed` 会把它们
        // 拷进 `MeshStorage`（渲染器的账，见 `docs/UI_ARCHITECTURE.md` 的拷贝链 C3）。
        let fallback: Vec<Tri>;
        let indices: &[Tri] = if batch.indices.is_empty() {
            fallback = quad_indices(batch.vertices.len());
            &fallback
        } else {
            batch.indices
        };
        if indices.is_empty() {
            return;
        }
        let mut b = self
            .r2d
            .mesh_indexed(batch.vertices, indices, &batch.texture)
            .transform(batch.transform)
            .layer(Layer::from(batch.layer));
        // **批次 scissor**：UI 的窗口内容 / 滚动可视区 / Clip 沙箱裁剪。
        //
        // 语义 = "这一批只能画在这个屏幕矩形内"（渲染器负责钳到目标；空矩形 ⇒ 丢 draw）。
        // 这是"环境裁剪不再切割几何"的落点：顶点保持原形（圆角、环带、投影都不被切平），
        // 越界像素由 scissor 裁掉。`Render2D` 自带画面级 scissor，两者**叠加**
        // （最终 = 命令级 ∩ 画面级 ∩ 目标矩形）。
        if let Some(clip) = batch.clip {
            b = b.scissor(clip);
        }
        // **仅在窗口 FX tint 生效时**才把 tint 抬到实例上。
        //
        // `mesh_indexed` 是 `ColorMode::Instance` ⇒ `tint` = 整段实例色（顶点色 × tint）。
        // 但 `Render2D` 里带实例色的网格段（`MeshStyled`）**自成一整段一次 draw**
        // （前后都 flush，不参与跨段合批）。不给 tint 时是普通 `Mesh`，可与同窗口、
        // 同纹理、同变换的相邻段合批——这正是「尽量减少 DrawCall」的收益点。
        // 未设 FX（绝大多数帧）时结果是 `WHITE`，跳过 `.tint()` 视觉上完全等价。
        if batch.tint != Color::WHITE {
            b = b.tint(batch.tint);
        }
        // 显式提交（等价于 Drop，但让"这个 builder 必须活到这里"成为读者的可见意图）。
        b.done();
    }
}
