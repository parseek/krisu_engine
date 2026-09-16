# rjw_ui

krusie 引擎的 UI 模块：**hybrid 模式**（立即外观 + ID 持久状态）+ **DOM 风格自动布局** + **Tkinter 风格几何管理器**（`place` / `pack` / `grid`）。

> 整套库入口：`use rjw_krusie::prelude::*;` 可直接取到 `Ui` / `UiAdd` / `UiState` / `Theme` / 常用控件；
> 低层与冲突名（如 UI 容器 `Window`）走 `rjw_krusie::ui::*`。

## 设计要点

- **立即外观**：每帧 `Ui::begin(...)` → 录制控件 → `finish(&mut backend)` 深度排序后**产出 `UiBatch` 批次**交给绘制后端（`rjw_ui` 不直接调用任何渲染器）。
- **绘制后端解耦**（v0.3）：`rjw_ui` 只认识 [`UiBackend`](src/backend.rs) trait —
  **只有 `texture(uid)` 与 `submit(UiBatch)` 两个方法**（矩形渐变改为四角顶点色后，
  不再有「后端替我建渐变纹理」这一职责）。一个 `UiBatch` = 一个实例
  =（纹理 + 顶点 + **三角形索引** + 实例变换 + 实例 tint + 实例数据 `UiBatchSource`）。
  - **几何全程直出三角形**：`indices: Vec<Tri>`（`Tri = [u16; 3]`）相对 `vertices`；
    索引与顶点同源同段存放（`Geom`），拼接时由 `Geom::append` 自动平移——分开算一旦
    不一致会产生错乱三角形且**不会 panic**。为空索引时后端按「每 4 顶点一组、
    `TL,TR,BL,BR`」的旧约定回退，外部后端可继续只产出顶点。
  - 实例粒度 = **（窗口 × 纹理）**：同一窗口内**所有控件 / 容器**合成一批
    ⇒ 「一个窗口 ≈ 1~2 次 draw call」（`source.elements` 记录本批覆盖的控件数）。
  - **不按控件切**是「尽量减少 DrawCall」的关键；**按窗口切**是因为批次携带窗口级
    transform/tint（烘进顶点会让窗口 FX 动画每帧重建整窗顶点，摧毁窗口顶点缓存）。
    窗口 FX tint 非白时该段自带实例色（`MeshStyled`）会自成一整段不参与跨段合批；
    其余批次走普通 `Mesh`，可继续与相邻同纹理同变换的段合并。
  - 切段规则由纯函数 `segment_runs` 裁决，契约由 `ui::batch_contract_tests` 断言（无 GPU）。
  - 真实后端：`rjw_krusie::runtime::layers::ui_backend::Render2dUiBackend`；
    测试后端：`rjw_ui::RecordingBackend`（收集批次，可断言 draw call 数）。
- **圆角不用纹理、不改着色器**（v0.4）：`tess` 模块把圆角矩形**CPU 镶嵌成三角形**——
  硬体轮廓 `alpha = 1`、同心的外圈轮廓 `alpha = 0`，两者沿**整圈**配成带状三角形（含四条
  直边）由光栅化器插值出羽化边缘（这就是抗锯齿）；梯度以几何边缘为中心（硬体内缩
  `f/2`、外环外扩 `f/2`）⇒ 视觉尺寸不变。羽化宽 = `Theme::feather`（逻辑像素，默认 1.0
  ≈ 标准 1px 抗锯齿，`with_feather(px)` 可调，0 = 硬边）。「单位四分之一圆弧表 + 步长抽样」用**一张**
  32 点表服务所有半径（半径 → 目标弧距 2px → `stride`（2 的幂）→ `segs = 32 / stride`）。
  圆角**边框**是外/内轮廓之间的**环带**（内半径 `max(0, r - width)`，与 CSS 同规则），
  只画一次边界。半径**不取整**（任意小数半径，超出半高 clamp 成胶囊）。
  旧的 `rjw_ui::proc` 32×32 九宫格圆角纹理**已删除**。
- **背景刷与配色令牌**（v0.4）：`PanelStyle::bg` / `ButtonStyle::{bg,bg_hover,bg_pressed}` /
  `InputStyle::bg` 的类型是 [`Brush`](src/style.rs)（`Solid` / `Vertical` / `Horizontal`；
  `Color: Into<Brush>` ⇒ 既有 `with_bg(Color::RED)` 不用改）。四角色进顶点色 ⇒
  **圆角 + 渐变天然共存**，零纹理。整套配色来自 `Palette`（按**层次**命名的表面阶梯 +
  描边 / 前景 / 强调）：`Theme::themed(&Palette)` 组装，`Theme::{light,dark,dark_legacy}`
  是预设；`Palette.bevel` + `bevel_raised/sunken` 从**一个**表面色派生微渐变。
  `Theme::with_radius` 级联到全部有圆角的子样式（panel / button / input /
  checkbox`r/2` / combo.menu_radius`min(r,6)`）；`Theme::with_border_w` 级联边框宽
  （0 = 不画），边框**颜色**是 `Palette::{border, border_strong}` 令牌。
  背景颜色按顶点位置双线性 lerp（圆角弧上的顶点也按位置取色 ⇒ 渐变不被"压进中间"）。
- **窗口投影也是顶点色**（v0.4）：`PanelStyle::shadow`（`ShadowStyle { blur, offset, color }`，
  色令牌 `Palette::shadow`）向外铺 4 圈同心圆角带、alpha 按 `a·(1−t)²` 衰减，**偏移逐环分摊**
  （只平移整体会在本体下方留一条等浓度暗带）；`blur = 0` = 不画
  （`Theme::without_shadow()`）。零纹理、零着色器改动、不增 draw call，进窗口顶点缓存。
- **布局密度可调**（v0.4）：`Theme::density(Density::{Compact,Cozy,Spacious})` 一趟缩放
  间距 / 字号 / 行距；单维微调 `with_font_scale` / `with_spacing_scale` / `with_line_spacing`。
  行距 `Theme::line_spacing`（行高 = 字号 × 该值，默认 `DEFAULT_LINE_SPACING` = 1.2）只作用于
  **可能换行**的文本；它同时进排版缓冲缓存键与窗口几何签名（`Ui::hash_cmds`），
  所以调它不会留下陈旧几何。默认档与扩展前逐像素一致。
- **状态持久**：交互控件（按钮/滑块/勾选/输入框）通过 **ID**（`&str`）把 hover / 按下 / 焦点 / 输入内容 / 拖拽标记持久化在 `UiState` 中（应用持有，跨帧复用）。
- **自动尺寸**（DOM 风格）：叶子控件由内容测量（`rjw_text::Text::measure` + padding）自然撑开，容器（panel / pack / grid）在闭包结束时按子控件结算自身尺寸——**默认无需手写宽高**；任何控件可显式 `.size(w, h)` 或传 `Rect` 覆盖。
- **屏幕空间**：控件坐标一律为屏幕像素（左上角原点、Y+ 向下），内部经相机屏幕固定变换绘制，命中测试直接在屏幕像素进行（旋转/缩放相机依然准确）。

> ⚠ **下方"快速上手"是 v0.2 的旧签名**（`Ui::begin` 收 6 参、`finish()` 不收后端），
> 与 v0.3 不符。现行写法见 [`docs/API_REFERENCE.md`](../../docs/API_REFERENCE.md) §11
> 与 `examples/eg260818UI`。

## 快速上手

```rust
let mut ui = Ui::begin(&cam, &ctx.mouse, &ctx.keyboard, &mut text, &mut r2d, &mut self.ui_state)
    .theme(Theme::dark())
    .base_layer(LAYER_UI)
    .build();

ui.label_at(Vec2::new(500.0, 20.0), "FPS: 60");       // 内容自动撑开
ui.pack_at(Vec2::new(24.0, 24.0), |p| {
    if p.button("开始游戏").clicked() { /* ... */ }   // 文本 + padding 自动宽高
    p.slider("volume", 0.0..=1.0, vol);               // 宽度 = 容器内宽
    p.checkbox("fs", "全屏", fs);
    p.text_input("name", &mut player_name);           // 自动高度
});
ui.grid_at(Vec2::new(320.0, 24.0), 3, |g| {           // 3 列自动均匀网格
    g.button("A"); g.button("B"); g.button("C");
});
ui.finish();
```

## 控件

| 控件 | 交互状态（ID 持久） | 返回值 |
|---|---|---|
| `panel` | 无（纯背景 + 边框） | `()` |
| `label` | 无（纯文本） | `()` |
| `button` | hover / 按下 | `ButtonState`（`.clicked()` 等） |
| `slider` | 拖拽标记 + 值 | `f32`（更新后的值） |
| `checkbox` | 勾选值 | `CheckboxState`（`.checked()` / `.toggled()`） |
| `radio` | 组内互斥（同组 ID 前缀） | `CheckboxState` |
| `text_input` | 焦点 / 光标 / 内容 | `()`（写入 `&mut String`） |
| `number_input` | 拖动调值 + 输入（`NumberInput`） | `()`（写入 `&mut f32`） |
| `color_picker` | 内联色块 → 弹出取色面板（`ColorPicker`） | `()`（写入 `&mut Color`） |

## 取色器（`ColorPicker`）

内联只占一行（色块 + `#RRGGBB` + 展开箭头），点开是**独立置顶面板**：

```rust
ui.add(ColorPicker::new("tint", &mut color).alpha(true)); // 面板里多一行 A
```

- **呈现模式** `u8` / `HEX` / `F` + 文本输入框：按模式呈现，**输入自动识别格式**
  （`255, 0, 0` / `#FF00AA` / `1.00, 0.00, 0.00` 都能直接粘贴）；
- 文本无法识别时输入框右侧出现**警告按钮**（`Icon::Warning`），按下恢复成当前颜色的有效值；
  打字中途**不会**改动颜色；
- **HSV 区**：SV 平面（四角顶点色的圆角矩形 = 精确 HSV 公式）+ 6 段色相条；
- **通道行**：颜色滑块（该通道 0→最大的渐变轨）+ `NumberInput`；
- `A` 行可选（`.alpha(true)`）；圆角 / 颜色 / 字体全跟 `Theme`；
- **跨帧数据全局唯一**：呈现模式、替补文本缓冲、展开的面板、HSV 缓存都在
  `ColorPickerState`（`UiState::color_picker`，定义在控件自己的模块里）——
  调用方**不需要**自己持有 `String`（需要外部读取时用 `.with_hex(&mut buf)`）；
- 同一时刻**只开一个**面板（共享文本缓冲必须只有一个所有者）。

纯函数 `format_color` / `parse_color`（+ `color_hex` / `parse_hex` / `ink_on` / `luma`）可单独用。

## 布局

> 布局模型的完整说明见 [`LAYOUT.md`](./LAYOUT.md)（含 pack 逐步推演、grid / flex / place / window / scroll / row）。

- `*_at(pos)`：绝对定位 + 内容自然尺寸（place）
- `pack_at(pos, side, |p| ...)`：按 `side`（`Top`/`Left`/`Bottom`/`Right`）堆叠，宽度 = 最大子控件自然宽；`Bottom`/`Right` 的 `pos` 锚定下/右边缘（页脚 / 右栏）
- `grid_at(pos, cols, |g| ...)`：均匀单元格网格，尺寸 = 最大子控件自然尺寸
- `flex_at(pos, total_h, weights, |f, i| ...)`：固定总高按权重等分
- `row(|r| ...)`：水平等高管线，占一行

## 绘制原语（圆角 / 渐变 / 矢量图标）

控件之外，`Ui` 直接暴露一组**绘制原语**（绝对定位 + 占光标两种写法）：

```rust
ui.rounded_rect_at(pos, size, 8.0, Color::RED);                       // 圆角（radius: f32 或 CornerRadius）
ui.rounded_rect_at(pos, size, CornerRadius { tl: 10.0, tr: 10.0, br: 0.0, bl: 0.0 }, Color::RED);
ui.gradient_rect_at(pos, size, Gradient::vertical(Color::RED, Color::BLUE)); // 渐变（无纹理，顶点色插值）
ui.icon_at(pos, Vec2::splat(16.0), Icon::ChevronDown, Color::WHITE);  // 矢量图标（不用字体字形）
ui.row(|r| r.icon(Vec2::splat(18.0), Icon::Check, Color::WHITE));     // 占光标 → 工具栏

// 背景图（tex = TextureWrapped::uid，texel = 纹理纹素尺寸）
let bg = ImageBg::new(tex, Vec2::new(64.0, 64.0));
ui.image_at(pos, size, bg.fit(ImageFit::Fill).radius(8.0));   // 等比覆盖 + 圆角遮罩
ui.image_at(pos, size, bg.fit(ImageFit::Tile));               // 1:1 平铺
ui.window("w").style(ui.theme().panel.clone().with_bg_image(bg)); // 窗口/面板底图
```

- 圆角 / 羽化 / 渐变 / **背景图**全部在 **CPU 镶嵌**（[`crate::tess`]）成三角形，
  无额外纹理、无额外 draw call；
- [`Icon`] 是**画出来的几何**（`Check` / `ChevronUp` / `ChevronDown` / `ChevronLeft` /
  `ChevronRight` / `Grip`），与字体无关，缺字形也不会变形；
- [`ImageBg`] 的 `Stretch` / `Fill` / `Center` 走**仿射 UV 映射**，因此能与圆角遮罩
  共存；`Tile` 用 1:1 图块四边形（不支持圆角，见 `ImageFit::Tile` 文档）。

## 依赖

`rjw_2d_render`（绘制）/ `rjw_text`（测量与渲染）/ `rjw_transform`（屏幕固定变换）/ `rjw_color` / `rjw_mouse`（鼠标）/ `rjw_keyboard`（字符输入）/ `glam`

## 许可

MIT
