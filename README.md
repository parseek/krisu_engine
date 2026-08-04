# krusie（krisu_engine）

一个用 **Rust + wgpu** 编写的 2D 游戏/渲染引擎（工作区名 `krusie`），以可运行的 examples 作为视觉验证与教学载体。

本项目继承自[`krjw_rust`](https://github.com/parseek/krjw_rust)，部分 crate 直接使用了该项目的代码，部分实现则借鉴了其思路。

目的为开箱即用，并且可以自由搭配不同的 crate 食用

后续会引入更多功能，但是现在所拥有的已经足够你去实现一个项目了

你以后一定会看见采用了不同 crate 搭配的 example 的（骗你的，现在就已经有了，但……

目前仅实现了基础的 2D 渲染器，计划加入**文本渲染**、**静态瓦片**、**高级特效**等，这些都需要作者（和他用的 Cline 以及里面的 DeepSeek 酱）的辛勤努力  

好在有不少东西都是继承自 [`krjw_rust`](https://github.com/parseek/krjw_rust) 的，不用重复造轮子，yay（

你说音频？直接套 `kira` crate 就够了（

## ✨ 特性

- 🎨 **Batch2D 批渲染**（`rjw_2d_render`）：Sprite/Mesh **统一管线** + RStates 渲染状态 bitfield（u64），按 (layer, states) 排序
- 🔗 **Builder 责任链**：`add_sprite2d(...).blend(Additive).depth_test(true)` 按对象定制渲染状态；不链式 = 全局默认
- 🎛️ **渲染状态 RStates**：Blend（含 Inverse/Subtract/Min/Max/Disabled 9 种模式）/ Sampler / Cull+Raster / Depth / Stencil 6 域 bitfield，三级控制（全局默认 → 单条绘制 → 批量描述符）
- 📦 **实例缓冲页池**：单帧精灵数量可远超单批上限（8192），自动分页绘制，不阻塞帧、无运行时扩张
- 🎥 **2D 正交相机**（`rjw_transform::Camera2D`）：中心原点、Y+ 向下、VP 矩阵直接透传、屏幕↔世界坐标互转
- 📐 **变换系统**（`Transform2D`）：位置/缩放/旋转、父子组合、命中检测
- ⌨️ **输入**：键盘（`KeyState` 边沿：pressed / down_edge / true_edge）+ 鼠标（位置/增量/滚轮/按钮）
- 🎞️ **程序化纹理**：无需外部资源，运行时生成草地/水面/树冠/角色等
- 🕹️ **综合 RPG 示例**（`eg260731RPG`）：波次敌人、多地形大地图、自动 y-sort 纵深感、相机跟踪居中、高 DPI 适配

## 📁 模块地图（crates）

| crate | 职责 |
|---|---|
| `rjw_main` | 入口 `run_app(App)`、事件循环、窗口、`MainContext`（键盘/鼠标/计时） |
| `rjw_render` | 底层 `RenderContext`、纹理 `TextureWrapped`、wgpu 重导出 |
| `rjw_2d_render` | ★ 2D 批渲染器 `Render2D`、`RStates`、Builder 责任链、`SpriteRect`、`Mesh`、分页实例缓冲、**统一管线缓存** |
| `rjw_transform` | `Transform2D` + `Camera2D`（正交投影、坐标转换） |
| `rjw_color` | `Color`(f32) / `ColorF64`(f64) + 常用常量 |
| `rjw_keyboard` / `rjw_keystate` | 键盘输入与边沿状态机 |
| `rjw_mouse` | 鼠标状态 |
| `rjw_time` | `DeltaTimer`（帧间隔 dt / FPS） |

## 🚀 快速开始

```bash
# 运行综合 RPG 示例
cargo run -p eg260731RPG

# 运行 Render2D 精灵/多边形演示
cargo run -p eg260731

# 运行最小清屏示例
cargo run -p eg260729

# 全工作区编译检查（改公共 crate 后必跑）
cargo check --workspace
```

### 综合 RPG 操作

`WASD` / 方向键 移动 · 鼠标 / 空格 / 左键 扇形挥砍 · 清波自动推进下一波 · `R` 重开 · `Esc` 退出

## 📖 文档

- **[docs/API_REFERENCE.md](docs/API_REFERENCE.md)** —— API 参考手册（免读源码版）：`Color` / `Transform2D` / `Camera2D`（含 `walk_xy` 等） / `SpriteRect` / `Render2D` / `ClearConfig` 的函数定义、用法与简单示例
- **[docs/ENGINE_GUIDE.md](docs/ENGINE_GUIDE.md)** —— 引擎「使用 + 维护」指南（人机皆宜）
  - 坐标系（**Camera2D：中心原点、Y+ 向下**）等易混淆概念
  - KeyState 边沿语义（`pressed` vs `down_edge`）
  - Layer 语义与 y-sort 惯用法
  - 物理像素 vs 逻辑像素（高 DPI 适配）
  - 实例缓冲**页池**覆盖问题与维护约定

## 🛠️ 环境要求

- Rust（edition 2024，resolver 3）
- Cargo workspace（见 `Cargo.toml`）
- 支持 wgpu 的图形后端（DirectX12 / Vulkan / Metal / GL）

## 📄 许可证

[MIT](LICENSE) © 2026 parseek(KrisuRJW, Cmd1loica99@163.com)
