# 矩形瓦片地图（GameMaker 风格）可行性研究报告

> 状态：**调研完成（未实现）**。评估为 krisu_engine（工作区 `krusie`）实现 GameMaker 风格矩形 tilemap
> 系统的可行性。生态调研基于 web（来源见文末链接），引擎能力部分基于本仓库代码核实。
> 结论先行：**可行，且非常契合现有架构**——缺失的部分全部在 CPU/数据侧，无算法/GPU 风险。

---

## 0. TL;DR

- krisu_engine 的 `Render2D` 已是"实例化四边形 + 图集 UV 子矩形"的批渲染管线（`SpriteRect` + 每实例
  `model` 矩阵 + 8192 实例/批自动分页），这正是"每 tile = 一个四边形 + tileset 图集 UV"所需的全部 GPU 侧能力。
- 缺失项全部在 CPU/数据侧：tile 数据模型（`TileSet` / `TileMap` / 层 / chunk 存储）、相机可见 tile 区间计算、
  tile→实例生成层（可加 chunk 级脏标记增量更新）。
- 建议：新增 `rjw_tilemap` crate，数据层与渲染层分离；存储用 **16×16 chunk 的 `HashMap<ChunkPos, [TileId; 256]>`**；
  `TileId` 用 **u32 打包（低 24 位 tile index + 翻转/旋转标志位）**，与 Tiled 的 GID 布局对齐；
  渲染先复用 `add_sprite2d` 做 MVP，再演进为 chunk 级实例缓冲 + 脏标记；交换格式采用 **TMX/TSX**
  （用 [`tiled`](https://docs.rs/tiled/latest/tiled/) crate 做 loader）。
- 工作量：MVP ≈ 1 个新 crate + 集成示例，纯 CPU 逻辑；完整版（chunk 增量实例、动画、翻转、TMX 导入、流式加载）为中等复杂度。

## 1. 现有 Rust 生态调研

### 1.1 bevy_ecs_tilemap（Bevy 生态事实标准）

[bevy_ecs_tilemap](https://docs.rs/bevy_ecs_tilemap/latest/bevy_ecs_tilemap/)（[GitHub](https://github.com/StarArawn/bevy_ecs_tilemap)）架构参考价值最大：

- **ECS 化**：每层一个 Entity，每个 **chunk 一个 Entity**；tile 数据经 `MapQuery` 读写。
- **存储**：`LayerSettings` 定义 `ChunkSize`；支持稀疏瓦片（[sparse_tiles 示例](https://github.com/Ygg01/bevy_ecs_tilemap/blob/main/examples/sparse_tiles.rs)）。
- **渲染**：每个 chunk 构建一个 Mesh（四边形 + 图集 UV），**tile 变更时重建该 chunk 的 mesh**；tileset 用 Bevy `TextureAtlas`。
- **配套**：[bevy_ecs_tiled](https://github.com/adrien-bon/bevy_ecs_tiled) 负责 TMX → bevy_ecs_tilemap。

启示：**"tiled crate 解析 + 引擎自带渲染"是主流组合**；bevy_ecs_tilemap 的"变更时重建 mesh"与 krisu_engine
实例化路径的"静态 + 每帧收集可见实例"是两条等价路线。

### 1.2 manytiles（macroquad 生态）

[manytiles](https://github.com/strawstack/ManyTiles)（manytiles.org）：分块存储 + **只渲染可见 chunk** 的剔除策略，
多 layer、tileset（单图/多图集合）；绑定 macroquad API，无法直接复用，但思路与 krisu_engine 要做的事同构。

### 1.3 tiled（rs-tiled，TMX 解析）

[`tiled`](https://docs.rs/tiled/latest/tiled/) 是 Rust 端解析 Tiled 格式的标准库：TMX（XML）/ TSX / TBIN（JSON），
完整数据模型（`Map`/`Layer`/`Tileset`/`Tile`），支持 **infinite map（chunk 层数据）**、动画 tile、Wang 集；
**纯数据层不渲染**，天然适合做导入器。备选：`tmx-rs`、`macroquad-tiled`。

### 1.4 wgpu 原生方案（非 Bevy）

[wgpu-tilemap](https://github.com/aweinstock314/wgpu-tilemap) 验证了"wgpu 直连 + chunked tilemap"可行但活跃度低；
[wgpu-pixel-renderer](https://github.com/erathe/wgpu-pixel-renderer)、[kelp-2d](https://github.com/emmyleaf/kelp-2d) 可作参考。
判断：**没有 wgpu 原生 tilemap 事实标准**——大多数 wgpu 引擎自实现瓦片层，复用自家 sprite 批处理路径。

### 1.5 GameMaker 的瓦片地图（参考对象）

依据 [GameMaker 手册](https://manual.gamemaker.io/lts/en/GameMaker_Language/GML_Reference/Asset_Management/Rooms/Tile_Map_Layers/Tile_Map_Layers.htm)：

- tile layer 是房间内一层网格，用画笔放置 tileset 瓦片；网格/瓦片尺寸由 **tileset 资产**定义（Tile Set Editor 可配 16/32/64px）。
- 数据模型：每层一个 tilemap 元素；瓦片以 **tile ID + 标志位打包成 bitmask**（`tilemap_get` 返回打包值），翻转/镜像由高位标志控制。
- 运行时 `layer_tilemap_create` + `tilemap_set` 编辑。
- **自动瓦片（Auto Tiles）**：基于邻居瓦片选变体（社区参考 [iAmMortos/autotile](https://github.com/iAmMortos/autotile)）。
- **大图性能**：GameMaker tile layer 是房间级固定网格，**无原生 chunk 流式**；官方论坛建议"关闭视野外 tile 绘制"
  （[1](https://forum.gamemaker.io/index.php?threads/large-amount-of-tiles-need-to-disable-tiles-outside-of-view.57000/)、[2](https://forum.gamemaker.io/index.php?threads/how-to-implement-large-maps.37981/)）——引擎侧自己做 chunk 剔除正是 Rust 实现优势。

### 1.6 TMX / Tiled 作为交换格式

- Tiled（mapeditor.org）跨平台开源地图编辑器，TMX（XML）+ TSX（tileset）是事实标准（[格式规范](https://github.com/mapeditor/tiled/blob/master/docs/reference/tmx-map-format.rst)）。
- **GID 翻转标志**：tile ID 与翻转标志打包进 32 位 GID——`0x80000000` 水平翻转、`0x40000000` 垂直翻转、`0x20000000` 对角翻转（[Global Tile IDs](https://doc.mapeditor.org/en/latest/reference/global-tile-ids/)）。
- **Infinite map**：层数据按 `<chunk>` 存储，默认 16×16。
- **自动瓦片**：Tiled 用 **Wang 集**描述邻居规则（[Using Wang Tiles](https://github.com/mapeditor/tiled/blob/ff0c3fbf0e7ba934605cae308540fa653d75a182/docs/manual/using-wang-tiles.rst)）。

## 2. 关键设计问题

### 2.1 数据模型

**TileSet（瓦片集）= 源图集 + 瓦片矩形网格 + 元数据**
- 复用 `rjw_atlas::AtlasRegion { tl_px, wh_px, origin_px, page_uid }`：每个 tile index → 图集区域 → `SpriteRect` 归一化 UV。
- 附加：动画帧表（index → `Vec<(帧区域, 帧时长_ms)>`）、可选 Wang/自动瓦片规则、碰撞/属性（后置）。

**TileMap（瓦片地图）= 层 × 分块二维存储**，三方案：

| 方案 | 结构 | 优点 | 缺点 |
|---|---|---|---|
| A. 完整二维数组 | `Vec<TileId>`，宽×高 | 简单、缓存友好 | 大图/无限图内存浪费 |
| B. 稀疏哈希 | `HashMap<(i32,i32), TileId>` | 编辑器友好、内存省 | 迭代/定位略慢 |
| C. **分块（推荐）** | `HashMap<ChunkPos, [TileId; 16×16]>` | 定位 O(1)、可流式、chunk 级脏标记、与 Tiled infinite chunk 对齐 | 多一层间接 |

- **TileId 打包**：仿 GameMaker/Tiled 用单个 `u32`——低 24 位 tile index + 高位 flip/旋转标志；TMX 导入时 GID→TileId 为常量时间掩码运算。
- **层**：复用 `rjw_2d_render::Layer(f64)`——每个 tilemap layer 对应一个 Layer 值，天然进入现有按 (layer, states) 排序的绘制队列。
- **动画 tile**：渲染时 `frame = (global_time / duration) % frames.len()`，只影响生成的 UV，不改变存储。

### 2.2 渲染（现有实例化路径直配）

现有能力（源码已核实）：
- `SpriteRect { mesh_tl, mesh_wh, uv_tl, uv_wh }`，UV 已归一化 [0,1]——正是"每 tile 四边形 + tileset 图集 UV 子矩形"。
- `InstanceData` 每实例携带 `mesh_tl/mesh_wh/uv_tl/uv_wh/color/model(Mat4)`——**翻转 = UV 翻折或 model 负 scale，旋转 = model 矩阵，零新增 GPU 代码**。
- 单批上限 `MAX_INSTANCES_PER_DRAW = 8192`，实例缓冲页池自动分页，单帧可远超 8192 实例。

三种发射策略：
1. **方案 A（MVP，先做）**：每帧遍历可见 tile 区间，逐个 `add_sprite2d(...)`。实现量最小；1080p + 64px tile 下可见约 30×17 ≈ 500 实例，性能无虞。
2. **方案 B（终态）**：按 chunk 收集实例（跳过空 tile），chunk 加**脏标记**，静态 chunk 缓存其 SpriteRect+Transform 列表，仅变更 chunk 重建。tileset 单纹理 → 所有 tile 同 (layer, states, texture)，合并为极少数 draw call。
3. **方案 C（进阶）**：GPU 端分块 index buffer / 每 chunk 独立实例缓冲段，静态时 CPU 零成本；本引擎规模下非必需。

**可见区间剔除**：`Camera2D` 已有 `viewport_size` + `screen_to_world`/`world_to_screen`，可计算视口世界矩形 →
tile 行列区间（旋转相机取包围盒近似）→ 区间内按 chunk 迭代。**需新增的小工具**：相机世界 AABB
（当前无直接 API，已确认无 cull/frustum 方法）。

### 2.3 内存与流式

- 16×16 或 32×32 chunk；`HashMap<ChunkPos, Box<[TileId; N]>>`，未加载 chunk 视为空。
- 每帧迭代 = 视口世界矩形 → chunk 坐标范围 → 仅访问这些 chunk（可见范围外不触碰，天然"chunk streaming"）。
- 纹理侧：tileset 合并进 `StaticAtlas` 单页（多页时 `page_uid` 区分，`rjw_atlas` 已支持），避免每 tile 一张纹理切换。

### 2.4 编辑器

- **TMX/TSX 是正确的交换格式**：用 `tiled` crate 做 loader，内部模型独立（loader 只做一次转换）；先支持 CSV 层数据，再支持 infinite chunk。
- **内置编辑器最小集**（后置）：放置/擦除、选区、层管理、翻转；自动瓦片可导入 Tiled Wang 集规则，引擎内用"8 邻域 bitmask → tile 变体"查表实现。

## 3. 针对 krisu_engine 的可行性结论

### 3.1 已有能力直接映射

| krisu_engine 已有 | tilemap 需求 |
|---|---|
| `rjw_atlas::AtlasRegion` / `StaticAtlas`（TOML） | tileset 瓦片区域（tile index → UV） |
| `SpriteRect`（mesh + 归一化 UV） | 每 tile 四边形 + 图集 UV 子矩形 |
| `InstanceData.model`（Mat4） | 翻转 / 旋转 / 缩放 |
| 实例化批渲染 + 8192 页池分页 | 每帧任意数量 tile 实例 |
| `Layer(f64)` 排序 | 多 tilemap 层绘制顺序 |
| `Camera2D`（viewport_size + 坐标互转） | 可见 tile 区间计算（需补一个小 AABB 工具） |

### 3.2 缺失项（全部 CPU 侧）

1. 瓦片数据模型：`TileSet` / `TileMap` / `ChunkMap` / 层 / 打包 TileId（全新代码，无 GPU 改动）。
2. 可见 tile 区间 + chunk 迭代：相机世界 AABB 小工具 + 区间→chunk 映射。
3. tile→实例生成层：MVP 走 `add_sprite2d`；终态走 chunk 实例缓存 + 脏标记。
4. 动画 tile 帧选择；翻转/旋转标志→UV/model 处理。
5. TMX 导入（可选依赖 `tiled` crate）。

### 3.3 工作量 / 复杂度

| 里程碑 | 内容 | 复杂度 | 预估量级 |
|---|---|---|---|
| MVP | 新 crate `rjw_tilemap`：TileSet/TileMap（2D 数组或 chunk）+ 可见区间剔除 + `add_sprite2d` 发射 + demo | 低 | ~600–1000 行 + 1 个 example |
| 标准版 | chunk 存储 + 脏标记实例缓存 + 翻转/动画 + 多 layer | 中 | 增量 ~500–800 行 |
| 完整版 | TMX/TSX 导入（tiled crate）、流式加载、Wang 自动瓦片 | 中 | 增量 ~800–1500 行 |

风险点：无算法/GPU 风险；注意点——静态大图避免每帧重复生成命令（方案 B 兜底）、旋转相机剔除用包围盒近似、
实例缓冲页池与大量静态 tile 的交互。

### 3.4 具体建议（5 条）

1. **新建 `rjw_tilemap` crate**：依赖 `rjw_2d_render` / `rjw_atlas` / `rjw_transform`，数据层（TileSet/TileMap/ChunkMap）
   与渲染层（Emitter）分离；MVP 直接复用 `add_sprite2d`。
2. **存储用 16×16 chunk**（`HashMap<ChunkPos, [TileId; 256]>`），TileId u32 位打包（低 24 位 index + 高位 flip/旋转标志），
   **与 Tiled GID 布局对齐**——未来 TMX 导入零转换成本，且天然支持流式。
3. **渲染终态为 chunk 级实例收集 + 脏标记增量更新**：tileset 单纹理时所有 tile 同批，draw call 数 ≈ 可见 chunk 数；
   保持"每帧实例数 = 可见 tile 数"。
4. **采纳 TMX/TSX 交换格式**：`tiled` crate 做 loader、内部模型独立；先 CSV 层，再 infinite chunk；
   编辑器能力（放置/擦除/翻转/层）后续迭代，与运行时解耦。
5. **动画与自动瓦片延后**：先静态矩形 tilemap + 剔除跑通；动画用"时间驱动帧表"轻量加入；自动瓦片按 Wang 集规则做可选层。

## 参考链接

**Rust 生态**
- [bevy_ecs_tilemap — docs.rs](https://docs.rs/bevy_ecs_tilemap/latest/bevy_ecs_tilemap/) · [GitHub](https://github.com/StarArawn/bevy_ecs_tilemap) · [sparse_tiles 示例](https://github.com/Ygg01/bevy_ecs_tilemap/blob/main/examples/sparse_tiles.rs)
- [bevy_ecs_tiled — GitHub](https://github.com/adrien-bon/bevy_ecs_tiled) · [docs.rs](https://docs.rs/bevy_ecs_tiled/latest/bevy_ecs_tiled/)
- [manytiles — README](https://github.com/strawstack/ManyTiles/blob/master/README.md) · [crates.io](https://crates.io/crates/manytiles) · [manytiles.org](https://manytiles.org)
- [tiled crate — docs.rs](https://docs.rs/tiled/latest/tiled/) · [README](https://docs.rs/crate/tiled/latest/source/README.md)
- [wgpu-tilemap — GitHub](https://github.com/aweinstock314/wgpu-tilemap) · [wgpu-pixel-renderer](https://github.com/erathe/wgpu-pixel-renderer) · [kelp-2d](https://github.com/emmyleaf/kelp-2d)
- [krisu_engine 仓库](https://github.com/parseek/krisu_engine)

**GameMaker 手册**
- [Tile Map Elements（含 bitmask）](https://manual.gamemaker.io/lts/en/GameMaker_Language/GML_Reference/Asset_Management/Rooms/Tile_Map_Layers/Tile_Map_Layers.htm) · [tilemap_get](https://manual.gamemaker.io/lts/en/GameMaker_Language/GML_Reference/Asset_Management/Rooms/Tile_Map_Layers/tilemap_get.htm) · [layer_tilemap_create](https://manual.gamemaker.io/lts/en/GameMaker_Language/GML_Reference/Asset_Management/Rooms/Tile_Map_Layers/layer_tilemap_create.htm)
- [Tile Set Editor](https://manual.gamemaker.io/monthly/en/The_Asset_Editors/Tile_Sets.htm) · [Auto Tiles](https://manual.gamemaker.io/lts/en/The_Asset_Editors/Tile_Set_Editors/Auto_Tiles.htm)
- 大图性能：[关闭视野外 tile 绘制](https://forum.gamemaker.io/index.php?threads/large-amount-of-tiles-need-to-disable-tiles-outside-of-view.57000/) · [How to implement large maps](https://forum.gamemaker.io/index.php?threads/how-to-implement-large-maps.37981/)
- 社区 autotile：[iAmMortos/autotile](https://github.com/iAmMortos/autotile)

**Tiled / TMX**
- [TMX Map Format 规范](https://github.com/mapeditor/tiled/blob/master/docs/reference/tmx-map-format.rst)
- [Global Tile IDs（GID 翻转标志）](https://doc.mapeditor.org/en/latest/reference/global-tile-ids/)
- [Using Wang Tiles（自动瓦片规则）](https://github.com/mapeditor/tiled/blob/ff0c3fbf0e7ba934605cae308540fa653d75a182/docs/manual/using-wang-tiles.rst)
