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
- **窗口外框可配**：`WindowBuilder::{title, close_button, shrink}` 三个**独立可选**部件
  （都不调 = 逐像素等于旧行为）。标题栏是内容**第一行**（通条底色 + 底边那条就是分隔线）；
  `×` 点击 ⇒ `*open = false` 后**整窗短路**（不录制、不写原点 / 尺寸、不占遮挡矩形），
  **重开由应用负责**；`shrink(show, collapsed)` 的 `show = false` **仍尊重** `*collapsed`
  （菜单 / 代码收起窗口）。按钮是**几何**（`Icon::Close` / `Chevron*`）而非字形，按下即
  `claim_press` ⇒ 点按钮不会顺带拖动窗口。`eg260818UI --sim-chrome` 脚本化守护。
- **按钮下拉菜单**：`Dropdown` 是普通 `Widget` ⇒ `p.add(Dropdown::…)` / `ui.add_at(..)` 加进
  任何容器。① `Dropdown::options(id, label, &mut u32, &[&str])` = 选项列表模式（选中行打勾 +
  整行高亮 + 点击写回 + ↑/↓ 切换）；② `Dropdown::new(id, label).menu(|m| ..)` = 富内容模式，
  `m` 是 `MenuCtx`（`Deref` 到 `Window`）⇒ **菜单内又可以 `UiAdd::add`**（文本输入 / 分割线 /
  横向排版 / **责任链菜单项**）。展开状态 `UiState::combo_open()`（**控件绝对 ID**；面板 = 它
  + `::popup`）。旧的 `combo` / `combo_at` 退化成糖（签名 / 行为不变）。几何助手 `item_h` /
  `popup_padding` / `popup_origin` / `popup_gap` 公开（脚本算坐标与引擎同源）；
  `eg260818UI --sim-dropdown` 脚本化守护（**7 段**）。
- **菜单项 `Item`（责任链）+ `Submenu`**：`m.item(Item::new("…").click_behavior(MenuClick::Keep))`
  / `.checked(&mut bool)` / `.submenu(|s| ..)`（旧写法 `m.item("文本")` 经 `From<&str>` 继续可用）。
  **点击行为 flag** `MenuClick::{Close,Keep}`：点完收起 / **保留 popup**。
  `Submenu`：普通项样式 + 右侧 ▸，**Hover 在行右侧**展开（`PopupSide::Right`）、**点击不收起**、
  鼠标进子面板保持、子面板里点项 ⇒ **整条链**收起；状态**行自持**（`WidgetState::submenu_open`，
  绝不挤 `UiState::combo_open`——那正是"点一下整条 popup 消失"的根因）；层级不限。
- **菜单栏**：`Ui::menu_bar(id, pos, |bar| ..)` —— 横向触发器 + **同一套**下拉面板
  （`MenuCtx` 的 `item` / `item_checked` / `submenu` / `caption` / `separator`）。展开状态
  `UiState::menu_open`（同一时刻只有一个菜单开着；点项 / 点栏外 / Esc 收起；点**另一个触发器**
  = 切换）。下拉的**录制 / 样式 / 宽度 / 行距（`popup_gap` = 逻辑 1px）/ 关闭规则**与 `Dropdown`
  共用 `widgets::menu::popup_show`（`WIN_TOPMOST` 哨兵 ⇒ 菜单栏录在哪里都盖得住别人；
  "点在任意 `WIN_TOPMOST` 浮层上不收起"是给**子菜单**留的）。
- **标题栏贴顶**：标题行录在窗口**顶边**（`y = 0`），条高 = `title_bar_h(row_h)` = **一行**
  （纯函数）；下面的内容与窗口高度各少一个 `pad_total`。条只是**背景装饰、不裁剪内容**
  （标题 / ▲ / ✕ 允许比条高）。`eg260818UI --sim-chrome` 断言按钮命中（其 y 公式同步去掉 `pad`）。
- **主题序列化（TOML）**：`Theme::to_toml`（全量导出 + `format_version` 头）/
  `from_toml`（从 `Default` 加载）/ `apply_toml`（**在当前主题上合并覆盖** ⇒ 手写的
  `gap = 12` 小文件直接可用）。`serde`/`toml` feature（默认开）；`Weight`→`u16`、
  `Align`→小写名、`CornerRadius` 收标量或表、`bg_image` 不入文件（纹理 uid 不可移植）。
  示例侧：顶栏「导出主题…」「导入主题…」+ **`--theme <路径>`**（启动即生效）+
  `--sim-theme <路径>`（脚本化验证往返与"进引擎"）。
- **浮层 z 分层**：浮层 z = `WIN_TOPMOST` 基址 + **嵌套层数**（`Ui::push_overlay_z`）；
  子浮层（子菜单 / 菜单里的取色器）整段（含**阴影**）画在父浮层之后。
  写死同一个哨兵 z 会让阴影被父层控件盖住（`elem 0` vs `elem ≥ 1` 的排序）。
  "是否在浮层上"用区间 `ui::is_overlay_z`。
- **全局字重**：`Theme::font_weight`（`Weight`，默认 `NORMAL` = 400，任意数值可用）；
  它是**排版输入**（改字形与步进宽度）⇒ 与 `line_spacing` 同样进「排版缓冲缓存键 +
  窗口 / 子槽几何签名前缀」两处，改它不会留下陈旧几何。`Density` / `scaled(DPI)` 都不碰它。
- **缩放柄 / 投影颜色令牌**：`PanelStyle::grip: GripStyle`（`GripShape::{Squares, Bars,
  Hidden}` + 颜色 / 尺寸 / 步距 / 个数；**只对固定宽窗口**生效，`Hidden` 只是不画图案、
  拖动缩放照旧）；`ShadowStyle::color` 是**任意色**（顶点 RGB 原样带出、只有 alpha 按圈衰减）。
- **运行时导入字体 / 图片**：`Text::load_font_data(data) -> Vec<String>` **返回新增族名**
  （`label.font_family(name)` 只认族名）；图片走后端**共享的**纹理表
  （`Render2D::gpu().texture(..)` → `ImageBg`）。文件选择器属应用侧（示例用 `rfd`）。
- **状态持久**：交互控件（按钮/滑块/勾选/输入框）通过 **ID**（`&str`）把 hover / 按下 / 焦点 / 输入内容 / 拖拽标记持久化在 `UiState` 中（应用持有，跨帧复用）。- **自动尺寸**（DOM 风格）：叶子控件由内容测量（`rjw_text::Text::measure` + padding）自然撑开，容器（panel / pack / grid）在闭包结束时按子控件结算自身尺寸——**默认无需手写宽高**；任何控件可显式 `.size(w, h)` 或传 `Rect` 覆盖。
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
| `label_ex`（`Label::ex` / `ui.label_ex` / `ui.colored_label`） | 无（纯文本） | `Response`（`rect` = 占用矩形）。**扩展标签**：整组 `TextStyle`（`.style(..)`）+ 字段级糖（`.font_size` / `.font_family` / `.weight` / `.italic` / `.stretch` / `.letter_spacing` / `.line_height` / `.align` / `.valign` / `.wrap` / `.ellipsis` / `.tint`）+ **首末两色渐变**（`.gradient` / `.gradient_v` / `.gradient_axis` + 域 `.gradient_mode(Glyph｜Line｜Frame)` / `.gradient_glyph` / `.gradient_line` / `.gradient_text`；默认 `Line`，单行时与整块逐像素相同）；裸 `Ui` 用 `.show(ui)`、容器里用 `.show_in(ui)` 或 `ui.add(..)`。字段级 > `.style` > `Theme::label` |
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
- `row_wrap(max_w, |r| ...)`：**自动换行的水平行**（等价 `row_builder().wrap_w(max_w).show(..)`）：塞不下就收行、行内左上角对齐、**只换行不压缩**
- `foldable(id, label)`：**可收缩区块**（标题行 + 可折叠正文；默认折叠，点标题翻转）
- `namespace(id, |ui| ...)`：**只加 ID 前缀**（不新增容器、不占额外光标）

### 可收缩区块（`foldable`）

```rust
ui.foldable("perf", "性能统计").show(|ui| {
    ui.label(&format!("FPS {fps}"));      // 折叠时这段**完全不录制**
});
ui.foldable("advanced", "高级").open(true).show(|ui| { /* 首次就展开 */ });

// 标题 = 标准容器：里面可放任意控件（它们自己认领按下，不会连带折叠标题）；
// 只固定宽、高度自然 ⇒ 放 Row / 多行内容时标题块自己长高、正文随之让位。
ui.foldable_custom("filters", |t| {
    t.label("过滤");
    t.row(|r| { r.checkbox_mut("all", "全选", &mut all); });
}).show(|ui| { ui.text_input("kw", &mut kw); });
```

- 标题行 = 一整行（高 `Theme::row_h`、宽铺满容器内容宽）：▶ / ▼ + 文本；**点它即翻转**（当帧
  几何不变、**下一帧**生效，与窗口 ⌃ 同口径）；`Tab` 可聚焦、`Enter` / `Space` 翻转；
- **默认折叠**（首次只见标题行）；`.open(true)` 只影响**从未被点过**的区块 —— 首帧把默认态
  落盘到 `UiState::folded`（**绝对 ID → bool 的表**，存"明确态"）⇒ 默认折叠的区块点开后
  **不会**被默认值折回去；
- **折叠 = 正文完全不录制**（不占高 / 不进命中表 / 不产生顶点）；正文内部的跨帧状态
  （滚动偏移 / 输入内容 / 焦点）**不清**，展开回来还是原样；
- **折叠状态引擎托管**（`UiState::folded`）：应用不必多一个 `bool` 字段 —— 要读 / 改用
  `UiState::{is_folded, set_folded, toggle_folded}`（`set_folded(id, false)` = 记住展开）；
  `reset()` 一并清空（各区块回到自己的 `open(..)` 默认态）；
- 正文录在**本区块的 ID 命名空间**里、并整体按 `FoldableStyle::body_indent` 缩进
  （缩进同时作用于绘制与命中 ⇒ "点得到的就是看得见的"，容器尺寸不变）⇒ 两个区块里的同名
  控件互不干扰、正文的归属一眼可见；
- 样式取 `Theme::foldable`（`FoldableStyle`：常态透明底 + 悬停 / 按下高亮 + ▶ / ▼ 颜色 / 字号；
  正文左缘竖引导线 `guide*`；可选"上下端阴影渐隐" `.with_body_fade(h, color)`；
  标题"类按钮"块状外观 `FoldableStyle::button_like(&palette)`）。
  `RJ_FOLD_TRACE=1` 打印每个区块的折叠态与几何（宽 / 高 / 缩进 / 渐隐）。

### 命名空间（`namespace`）

```rust
ui.namespace("left",  |ui| { ui.text_input("kw", &mut left);  });   // 键 = "left/kw"
ui.namespace("right", |ui| { ui.text_input("kw", &mut right); });   // 键 = "right/kw"
```

**不做任何布局**（没有背景 / 内边距 / 裁剪，也不另开 frame）——正文与"不用它"时逐像素
相同，只是内部控件的**绝对 ID** 多了 `id/` 前缀；返回正文结算尺寸（`Vec2`）。窗口 / 面板 /
滚动容器 / grid / 区块各自已是命名空间边界，本入口给"只想要 ID 隔离、不要容器"的场合。

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

## 绘制器（`Painter`，独立于 `Ui` 的组件）

上面那些 `ui.xxx_at(..)` 只是糖：录制机制住在 [`Painter`] / [`DrawQueue`]（`rjw_ui::painter`）。
一个**绘制块**取一次 painter，块内所有原语不再逐参数传 `elem` / 环境裁剪：

```rust
let mut p = ui.painter();
p.panel(rect, theme.button.bg, theme.button.border, 1.0, theme.button.radius); // elem 自动 = elem_hint
p.text(rect, label, 14.0, Color::WHITE, None, TextAlign::Center, TextVAlign::Center, None, None);
p.panel_elem(shadow_rect, bg, border, 1.0, 0.0, 0);   // elem = 0 = 容器装饰层（画在所有元素之下）
```

- **独立**：`Painter` 只拥有录制状态（队列 + 播放头）与 DPI 换算，**不引用 `Ui`、不需要
  字体图集 / GPU** ⇒ `Painter::new(1.0)` + `commands()` 就能在单测里断言"画出了哪几条命令"；
- **一段绘制一个 painter**：`ui.painter()` 借 `&mut self`，所以顺序是「先量 → 画 → 再量」
  （不是一个 painter 画到底，也不能边画边跑子控件）；
- **装饰压住自家内容** = 重新取一次 `ui.painter()`（不是冻结 elem）；容器装饰传 `elem = 0`；
- **局部裁剪**：`ui.painter_clipped(Some(rect), |p| { … })`（块外自动恢复，不动 `Ui` 的强制裁剪层）。

## 依赖

`rjw_2d_render`（绘制）/ `rjw_text`（测量与渲染）/ `rjw_transform`（屏幕固定变换）/ `rjw_color` / `rjw_mouse`（鼠标）/ `rjw_keyboard`（字符输入）/ `glam`

## 许可

MIT
