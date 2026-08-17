# rjw_text 文本可见性剔除 — 理论分析与设计草案

> 状态：**A1（收集期剔除）、B（整块 + 逐字形剔除）、CachePolicy 已实现**（见 `rjw_text` 的
> `TextLayout::clip/cull/cache`、`TextRender::clip/clip_world/cull`、`CachePolicy`，及 `rjw_2d_render`
> 的 `Render2D::set_culling`）。A2（cosmic-text scroll/prune 整形期裁剪）与双图集仍为**后续项**。
> 本文保留原始分析供实现评审。所有代码事实均基于当前仓库（`crates/rjw_text`）与 cosmic-text 0.19 本机源码。
> 背景：与 glyphon 对比后确认 rjw_text 缺少可见性剔除；本文同时记录双图集方案为何暂缓。

---

## 1. 目标

在不改变 API 语义的前提下，为 rjw_text 增加两种剔除：

- **排版阶段剔除（轨道 A）**：普通 GUI 文本（**无变换**，轴对齐）在文本局部坐标系裁剪，跳过不可见行/字形。
- **渲染阶段剔除（轨道 B）**：带变换（旋转/缩放）的文本，在世界坐标系对逐字形保守剔除，先做整块剔除。

双图集（R8 mask + RGBA8 color）因 `Render2D` 尚不支持自定义 Shader 而**暂缓**，仅在第 8 节做理论分析。

## 2. 现状链路与剔除插入点

当前数据流（每帧）：

```
TextLayout（text/size/align/attrs…）
  └─ into_render() / into_render_with() / render_from()
       ├─ shape_and_rasterize()         ① 排版（create_buffer → cosmic-text 整形 + 光栅化入图集）
       └─ collect_glyphs()              ② 遍历 layout_runs() 填充 Vec<GlyphData> / Vec<LineMeasureInfo>
            └─ TextRender
                 ├─ draw_sprite2d()     ③ 逐字形 add_sprite2d（Render2D 批处理）
                 ├─ draw_2d_gradient()  ④ 逐字形动态 mesh（逐顶点色）
                 └─ draw_with()         ⑤ 逐字形回调（渲染器无关）
```

旁路：`draw_label_with` / `draw_text` → `visit_glyphs()`（lib.rs）回调遍历；`measure` / `measure_buffer` 只排版不收集。

**可插入剔除的位置**（按"越早越省"排序）：

| 位置 | 阶段 | 省下的开销 | 备注 |
|---|---|---|---|
| ① 整形前/中 | 排版 | 整形 CPU（Debug 下主要开销） | 只能靠 cosmic-text scroll/prune（见 A2），与共享 `Arc<Buffer>` 缓存冲突 |
| ② 收集期 | 收集 | 收集 + 后续提交；不可见字形不产生 `GlyphData` | 文本局部坐标裁剪；对测量/坐标零影响（见 A1，**推荐默认**） |
| ③④⑤ 提交期 | 渲染 | 提交/实例/绘制 | 世界坐标；必须先整块剔除再逐字形（见 B） |

光栅化（`rasterize_and_pack`）本身有图集缓存：不可见字形若曾可见不会重复光栅化；从未可见的字形第一次收集时才会入图集——收集期剔除顺带省掉这部分图集占用。

## 3. 两条轨道总览

| | 轨道 A：排版/收集期剔除 | 轨道 B：渲染期剔除 |
|---|---|---|
| 适用 | GUI 文本，**无变换**（轴对齐） | 变换文本（旋转/缩放），及任意文本的兜底 |
| 坐标系 | 文本局部（clip 相对视觉原点） | 世界（clip 相对场景） |
| 粒度 | 行（垂直）→ 字形（水平） | 整块 → 字形 |
| 对测量影响 | A1 无；A2 有（见 4.2） | 无（测量在剔除前完成） |
| 对缓存影响 | A1 无（clip 是每次绘制的参数，不进缓存键）；A2 与共享 Arc 缓存冲突 | 无 |
| 成本 | 每帧一次矩形比较 | 每字形 1~4 次 transform_point |

两条轨道互斥使用：**有 transform 走 B，无 transform 走 A**（内部按 `TextRender::transform` 是否 `Some` 自动选择；`map()` 逐字形 transform 视为 B）。

## 4. 轨道 A：排版/收集期剔除（GUI 文本）

### 4.1 A1：收集期剔除（默认推荐）

**算法**：`collect_glyphs` 与 `visit_glyphs` 增加可选 `clip: Option<Rect>`（文本局部坐标，相对文本视觉原点）。

- **行级（垂直）**：`run.line_top`（含 `run.line_height`）与 `clip.y..clip.y+clip.h` 无交集 → 整行跳过。
- **字形级（水平）**：`physical.x + loc.left`（字形左上）与 `physical.x + loc.left + glyph_w` 与 `clip.x..clip.x+clip.w` 无交集 → 跳过。
- 收集到的字形坐标/尺寸**不做任何修改**（剔除只是"不收集"，不影响位置语义）。

**为何无副作用**：
- 坐标仍是文本局部绝对值（未用 scroll，位置不受影响）。
- `measure` 来自未裁剪的布局（A1 不改 cosmic-text 配置：`set_size(Some(wrap), None)` 保持）→ 测量恒为全文。
- 与共享 `Arc<Buffer>` 排版缓存完全兼容：clip 是**每次绘制的参数**，不属于缓存键；同一 Buffer 可被不同 clip 复用。

**代价**：`collect_glyphs` 每帧仍遍历全部 `layout_runs()`（O(行)），只是跳过裁剪区外的收集。对超大文本（几千行）行遍历本身也可用 `line_top` 二分定位起始行，但常见 GUI 文本行数少，先不做。

**注意**：cosmic-text 的 `LayoutRunIter` 在设了 `height_opt` 时会**硬截断**迭代（buffer.rs:261-265 `if line_y - max_ascent > height { return None }`），因此 A1 必须保持 `height = None`，裁剪完全由我们自己做。

### 4.2 A2：整形期裁剪（cosmic-text scroll/prune，大文本进阶）

cosmic-text 0.19 原生支持"只整形可见区"（源码证据，`buffer.rs`）：

- `Buffer::set_scroll(Scroll{line, vertical})`（buffer.rs:854）+ `set_size(width, height)` 后，`shape_until_scroll(font_system, prune=true)`（buffer.rs:571）**只对可见行整形**，`prune=true` 还释放区外行（`reset_shaping`，buffer.rs:612-615）。
- `layout_runs()` 只迭代 scroll 窗口内的 runs（`LayoutRunIter`，buffer.rs:244-288；`line_top = self.line_top - scroll`，坐标变为 **scroll 相对**）。

**收益**：这是唯一能省**整形 CPU** 的路径。rjw_text 现状：Release 下 >512B 文本不缓存、每帧 `create_buffer` 整形（`lib.rs` `should_use_layout_cache`），此时 scroll 裁剪直接省整形。

**代价 / 语义变化**（必须如实记录）：
1. **坐标相对化**：`layout_runs()` 返回的 `line_y/line_top` 减去 scroll → 收集/绘制前必须回加 scroll 偏移，否则位置错位。
2. **测量变化**：`measure_buffer` 遍历 `layout_runs()` → 裁剪后只测得可见区；若需要全文测量需另做一次未裁剪整形（或接受"测量=可见区"语义，向调用方文档化）。
3. **缓存冲突**：scroll 是 Buffer 的**可变状态**；不同 clip 需要不同 scroll → 不能共用同一个共享 `Arc<Buffer>`。方案：scroll 路径**不入 LRU 排版缓存**（大文本本来也不缓存），或缓存键加入 scroll（收益低，不推荐）。
4. 行级可见性：部分可见的行仍整体整形（cosmic-text 行是整形最小粒度）——符合预期，无需处理。

**结论**：A2 只在"大文本 + 每帧整形 + 轴对齐"场景有净收益，作为后续可选优化；先做 A1（无副作用）与 B。

### 4.3 边界情况（A1/A2 通用）

- `clip` 为空矩形 / 全不可见 → 直接跳过收集（`MeasureInfo.glyph_count = 0` 语义保留，调用方按现状处理空文本）。
- 行高为负 / 异常度量（字体回退导致 `line_height` 波动）→ 用"区间相交"而非"上下界顺序"，避免负高度误杀。
- RTL / bidi：cosmic-text 已把字形排到物理位置，clip 只看物理坐标，无方向性假设。
- 部分可见行：整行收集（行内字形再水平裁剪），保证 bearing/基线度量完整。
- `origin` / `offset`：clip 是文本局部坐标，**不含** `render_delta`（origin/offset）；绘制时 delta 照常叠加。

## 5. 轨道 B：渲染期剔除（变换文本）

### 5.1 算法

`TextRender` 增加 `clip_world: Option<Rect>`（**世界坐标**）。绘制路径（③④⑤ 共用一条过滤）按序：

1. **整块剔除（最便宜）**：文本块世界包围盒 = `transform`（若有）作用于 `content_size` 的 4 角 AABB，再平移 `render_delta`；与 clip 无交集 → 整块跳过（三个绘制路径都直接 return）。
2. **逐字形剔除**：对每个字形，取局部 quad（`top_left` + `size`）四角，依次应用 `map()` 后的 `GlyphData::transform`（若有）与 `TextRender::transform`（若有）→ `transform_point` 得世界四角 → 取 min/max AABB → 与 clip 相交测试。旋转时 AABB 是**保守包围盒**（不会误杀可见字形）。
3. 剔除仅跳过**提交/回调**；`MeasureInfo` 与 `glyphs()` 切片不变（剔除不影响数据，只影响绘制）——与 A 的"不收集"不同，B 是"不提交"。

### 5.2 与现有 API 的相互作用

- `map()` 逐字形改 `top_left` / `transform`：B 的逐字形剔除**必须**在 `map()` 之后（当前 `draw_sprite2d` 已按 `g.transform` 组合变换，过滤插在同一循环前即可）。
- `draw_with` 回调语义变化：被剔除字形**不回调**。文档化："回调只收到可见字形"。
- `draw_2d_gradient`：渐变域（Glyph/Line/Frame）基于全部字形计算——**剔除不应改变渐变域**。因此渐变路径的剔除放在"渐变域已算完、逐字形提交 mesh"之前，且 Frame/Line 域仍遍历全部字形（仅提交跳过）。这是 B 在渐变路径上的一个实现注意点。
- clip 默认 `None` = 不剔除（保持现状行为）。

### 5.3 边界情况

- `transform` 为旋转 90°：AABB 退化为正方形包围，保守但正确。
- 缩放为 0 / 负：AABB 退化（宽度 0）→ 按"与 clip 无交集"处理；负缩放需取 abs（设计上约定 clip 剔除用 |scale|）。
- 层级（layer）：剔除只看空间，与层级无关。

## 6. API 草案

```rust
// rjw_transform 或 rjw_text 提供：
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
impl Rect {
    pub fn intersects(&self, other: &Rect) -> bool;   // 区间相交（容忍负宽高的保守实现）
}

// 轨道 A（文本局部坐标）：
impl TextLayout<'a> {
    pub fn clip(self, clip: impl Into<Option<Rect>>) -> Self;      // 进入收集期剔除
}
impl TextRender<'_> {
    // 轨道 B（世界坐标，自动选择：有 transform → B，无 → A）
    pub fn clip_world(&mut self, clip: impl Into<Option<Rect>>) -> &mut Self;
    pub fn clip(&mut self, clip: impl Into<Option<Rect>>) -> &mut Self; // 无 transform 时等价 A；内部按 transform 有无分发
}

// draw_label* 便捷方法透传：
impl Text {
    pub fn draw_label_ex_clipped(/*…, clip: Option<Rect>*/) -> Vec2; // 或 draw_label_ex 增加可选参数
}
```

坐标约定：
- `clip`（A）相对文本**视觉原点**（与 `GlyphData::top_left` 同基准，不含 origin/offset）。
- `clip_world`（B）为世界坐标（与最终 sprite 位置同基准）。

## 7. 理论成本-收益分析

| 粒度 | 单帧成本 | 命中场景 | 备注 |
|---|---|---|---|
| 整块（B1） | O(1)（4 次 transform_point） | 文本完全在视口外（滚动/移出屏幕的 HUD 块、Tab 页切换） | **性价比最高**，先行 |
| 行（A1/B） | O(可见行) | 长列表/日志/聊天，视口只露几行 | 收集期跳行；A1 零副作用 |
| 字形（A1/B） | O(可见字形) | 宽文本水平滚动、居中截断 | 水平裁剪仅在收集/提交循环内 |

与现有机制的关系：

- **LRU 排版缓存**：A1 不触碰缓存键（clip 是绘制参数）→ 缓存命中率不变；被剔除的块不再白收集，但排版本身已被缓存复用，无浪费。
- **`render_from`（用户持 `Arc<Buffer>`）**：A1 同样适用（clip 参数化）；A2 要求用户改用可变的裁剪缓冲，暂不提供。
- **`no_image`**：收集期剔除使从未可见的字形不触发首次光栅化 → 图集占用/打包更少；已入图集的字形不受影响。
- **图集去碎片重排**：与剔除无关（区域同步在 `sync_atlas_regions` 完成）。

量化建议（实现阶段）：扩展 `eg260810TextChain` 加"万行日志 + 视口"场景，用 `MeasureInfo.glyph_count` 与帧时间对比剔除前后；Debug 下预期收集/提交是主要开销（整形被缓存），Release 下大文本整形才是大头（对应 A2 的收益区）。

## 8. 双图集理论分析（暂缓）

**现状**：`rasterize_and_pack` 把所有字形（灰度 mask 与彩色 emoji）统一输出为 **RGBA**（mask → 白+alpha，lib.rs `swash_render_image` → `rgba`），存入单一 `DynamicAtlas`。优点：单一纹理、单一采样路径、mask 可染色（draw_sprite2d 按 `GlyphType` 乘色）。代价：mask 字形占 4B/px（R8 的 4 倍），且 emoji 颜色在 sRGB 上不做精确处理。

**glyphon 的做法**（对比依据）：双图集 `mask_atlas`(R8Unorm) + `color_atlas`(Rgba8UnormSrgb / Rgba8Unorm)，shader 里按每实例 `content_type` 分支：Color → 直接取色，Mask → `color.rgb, color.a * mask.a`；`ColorMode::Accurate` 时顶点色做 sRGB→linear。

**rjw_text 采用双图集的前提**：需要
1. 两个纹理绑定（color + mask）+ 每实例 `content_type`（或每字形 UV 指向的图集区分）；
2. 一个能分支的 fragment shader；
3. mask 字形 1B/px 打包（`Format::Mask` 输出 + R8 纹理）。

**障碍**：`Render2D`（`crates/rjw_2d_render`）的 `sprite.wgsl` 是**单一纹理采样**，`add_sprite2d` 不接受自定义 Shader / 额外 bind group，`Render2D` 无 shader 注册机制。

**三条路径评估**：
- a) **Render2D 增加自定义 Shader/专用文本管线**（shader 注册 + bind group 扩展）：改动面大，跨 crate；收益=内存 4x 节省 + emoji sRGB 精确。
- b) **保持单图集，仅增加每字形 sRGB 标记**（仍需 shader 改动，只解决色彩精度，不省内存）。
- c) **维持现状**：内存/精度都可接受时不动。

**建议**：优先完成剔除（A1+B，第 4/5 节）；双图集作为"Render2D 支持自定义 Shader"之后的独立后续项（届时选 a），本文记录前置条件即可。

## 9. 实施顺序建议（供下一计划使用）

1. **B1 整块剔除**（最小改动：`draw_sprite2d`/`draw_2d_gradient`/`draw_with` 开头一次 AABB 判断）——性价比最高。
2. **A1 收集期剔除**（`collect_glyphs`/`visit_glyphs` 加 clip 参数，`TextLayout::clip`）。
3. **B2 逐字形世界剔除**（`transform_point` 四角，渐变路径注意域不变）。
4. **A2 scroll/prune**（可选，先基准验证 Release 大文本收益）。
5. **双图集**（依赖 Render2D shader 支持，另行计划）。

每步都补测试：clip 语义、RTL、旋转 AABB 保守性、渐变域不变性、`measure` 不受 A1 影响。
