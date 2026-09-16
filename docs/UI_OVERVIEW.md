# rjw_ui 使用方式与原理（概览）

> 面向"用之前先看懂"——先讲**原理**（它为什么这么设计），再讲**使用方式**（怎么用），
> 最后给自定义控件与调优入口。详细 API 见 `API_REFERENCE.md`、`WIDGET_GUIDE.md`、
> `GUI_GUIDE.md`。

---

## 一、它是什么

`rjw_ui` 是 krusie 引擎的 UI 模块，采用 **hybrid 模式**：

- **外观立即录制**：每帧 `Ui::begin` → 摆控件 → `Ui::finish`，布局/绘制命令随帧生成；
- **状态按 ID 持久**：交互状态（hover / 按下 / 焦点 / 输入内容 / 拖拽 / 单选 / 滚动 /
  grid 单元格 / 窗口位置）经 `UiState` 跨帧保留。

这带来两个好处：**不需要保留 UI 树**（像 egui 一样，代码即界面），又**不会因为重新绘制
丢掉状态**（像传统 immediate GUI 那样每帧重置）。

**"ui anywhere"**：一帧 = **开场 + N 段 + 收尾**——运行时入口 `f.ui(theme)` 开**一段**，
一帧可开任意多段、位置随意（世界绘制之前 / 之间 / 之后均可），`&mut Ui` 可直接透传给
任意函数 / 模块；帧级账（帧号 / 命中区翻页 / 输入快照 / 焦点导航 + 描边 / 光标 / 统计）
**每帧只做一次**（开场在第一段懒执行，收尾由运行时在提交前补齐）。

布局是 **DOM 风格自动尺寸**（叶子控件由内容撑开、容器闭包结束时按子控件结算），几何
管理是 **Tkinter 风格**（`pack` 堆叠 / `grid` 网格 / `*_at` 绝对定位）。

---

## 二、原理

### 1. 一帧的生命周期（开场 → 录制（N 段）→ 提交 → 收尾）

```
【开场】本帧第一段 Ui::begin(..).build() 懒执行（运行时路径由 Frame::ui 触发）：
        帧号 +1 / 命中区表翻页 / 冻结输入快照 / 帧级暂存清零 + 责任链种入
【段】  f.ui(theme) → ui.label_at / pack_at / ui.window(id).show(..) / add(...)
        → Ui::finish(&mut backend)：分桶 → 顶点（含窗口顶点缓存）→ 提交到 UI 层 Render2D
        （一帧可开多段，段序 = 绘制序：后一段整体压在前一段之上）
【收尾】Ui::end_frame(&mut backend)（每帧一次，运行时在提交前调）：
        输入结算（空白清焦点 / 清一次性边沿 / 窗口按下裁决）/ 焦点导航 + 描边 /
        光标定夺 / 统计写回 / 帧级暂存关场
```

- **录制阶段**：每次控件调用把一条/多条 `UiDraw` 命令压入**本段**队列（坐标是**相对当前
  容器的局部逻辑像素**，容器弹出时统一平移成绝对）。这一阶段不碰 GPU，也不碰输入设备
  （快照）。**帧级事实**（按下归属 / 窗口原点 / 焦点链 / 光标意图 / 责任链 / 位置尺寸责任链）
  由各段共享，段收尾时回存、下段开头装载——所以第二段录的窗口也参与遮挡、Tab 顺序与拖动。
- **提交阶段**（`finish`）：排序 → 按窗口分组 → 收集成四边形 → 提交到 UI 层自己的
  `Render2D`（`set_sort_mode(SortMode::None)`，绘制顺序由 UI 自己管理）。
- **段间可交错**：世界层绘制、世界文本、逻辑代码都可以放在两段之间（段存活期间 `f` 被借用，
  编译期拦住 `draw`/`submit`）。

### 2. 坐标与 DPI

- **对外 API 全是逻辑像素**（`scale_factor` 传入物理/逻辑比）。坐标约定：屏幕左上角原点、
  Y+ 向下。
- **内部计算一律物理像素**：渲染取整（`snap_rect`）、命中、滚动偏移都在物理侧，DPI 只在
  API 边界换算一次。对外若提供"物理/逻辑可选"参数，用 `draw::Metric<T>`（`Physical` /
  `Logical`，`to_physical(scale)`）。
- 世界坐标：UI 是**屏幕固定**的（不随世界相机旋转/缩放）。`finish` 只接收
  [`rjw_transform::Viewport`]（大小 + 位置），不需要 `Camera2D`（它留给世界渲染）。

### 3. 布局引擎（`layout.rs` 的 `Frame` 栈）

`Ui` 维护一个 `frames: Vec<Frame>` 栈，每个容器（panel / pack / grid / window / flex /
scroll / row）压一帧。控件经 `child_rect(w, h)` 在栈顶帧内占一个矩形并推进光标：

- **pack**：按 `PackSide::Top`（垂直）/ `Left`（水平）堆叠，尺寸 = 最大子项；
- **grid**：`cols` 列均匀网格，单元格尺寸跨帧缓存（内容变化可扩可缩）；
- **固定宽容器**（`ui.window(id).width(w)`）：子项宽度 clamp 到固定值、高度自然（egui 风格）；
- **flex**：固定总高按权重等分；
- **min/max 约束**：`p.min_size(w,h)` / `p.max_size(w,h)` 作用于下一子项；
- **row（等高）**：水平排列 + `Theme.row_h` 强制所有子项等高 → 文字中心线对齐。

**Widget 尺寸契约**（`widget.rs`）：
```rust
fn constraints(&self) -> SizeConstraints { SizeConstraints::default() }  // min_w/max_w/min_h/max_h 全 Option<f32>
fn expansion(&self) -> Expansion { Expansion::UnlimitedExpansion }       // DisableAutoExpansion / LimitedInParent / UnlimitedExpansion
fn resizable(&self) -> Option<(Vec2, Vec2)> { None }                      // 可选拖拽缩放范围
```
`Ui::add` 统一 clamp + 膨胀调整。`LimitedInParent` 取 min(内容, 父级可用宽
`Ui::avail_w()`)，超出由控件自洽（Label 换行 / 省略、Button 省略、TextArea 滚动）。

**容器尺寸必须包住子控件**：`grid_at` / `pack_at` / `add_at` 这类**绝对放置**不占光标，
若容器只按流内子项结算尺寸，内容就会"长到容器外"——画得出来却不在窗口矩形里（拖不动、
被别的窗口穿透）。`Frame::content_bounds` 记录每处子项矩形（含 `Child::Expand` 的子项、
绝对容器整体、`add_at` 控件），`settle_size` = 自然尺寸 ∪ 内容包围盒；**固定轴**
（`fixed_w` / `fixed_h`）例外，那一轴由调用方定死、超出交给 `Clip` 语义。
即便真的溢出，窗口遮挡矩形 = 盒子 ∪ 子控件命中区（`Ui::win_hit_bounds`），
"看得见就能点、被压住就不响应"仍然成立。

### 4. 命中与交互状态机（`hit.rs` + `WidgetState`）

控件交互三件套：`hit_abs(&id, &rect)`（矩形命中 + 窗口遮挡 + **控件级遮挡** + **强制层
命中过滤**）、`mouse_left()`（左键含边沿）、`hit::update_interact` / `update_drag`（跨帧状态机）。
窗口 / 面板 / 浮层的**整块本体**用 `hit_body_abs(&rect)`（同样的过滤，但不参与控件级遮挡）。

关键约定：
- **窗口遮挡**：重叠区域只让鼠标下最上层窗口响应（点击穿透修复）；
- **控件级遮挡**：同一窗口 / 面板内的重叠控件，只有**后录制（画在上面）**的那个响应
  （`hit_abs` 的第一个参数就是控件身份；诊断 `UiState::widget_occluded_hits`）；
- **press_claimed**：自身有拖拽语义的控件（滑块 / 滚动条 / 文本框 / 缩放柄）按下时置位，
  阻止外层窗口/面板把本次按下当拖拽基准（窗口内拖滑块不连窗口动）；
- **拖动需位移** ≥ 3 物理像素才激活（纯点击不拖拽 → 子控件正常响应）。

### 5. 裁剪分层（`view.rs` + `draw.rs`）

裁剪分**两层**：

- **强制层（硬裁剪）**：ScrollView 可视区 / Clip 沙箱（`ui.window(id).placement(Placement::Clip)`、文本框）。
  `UiDraw.clip` 恒为该层，**所有绘制（含 noclip 变体）都服从**——内容超出可视区是物理约束；
- **软层（内容裁剪）**：控件自身内容边界，由调用方显式传（`push_text_rect` 的局部 clip、
  文本框内容区）。内容**自洽**的控件（自动换行 / "…"省略 / 滚动）用 `push_*_noclip`
  **跳过软层**——但 ScrollView 强制裁切躲不掉（无 Scroll 的普通容器本无强制层）。

**View 沙箱**（`view.rs`）：闭包作用域 `ui.view_at(pos, size, mode, |v| …)`，统一
"裁剪层 / 坐标原点 / 可用宽度 / 命中过滤"，`scroll_at`、文本编辑框、严格窗口共用底座。

### 6. 滚动（ScrollView）

`scroll_at`（容器）/ 文本框（控件内 ScrollView）共用滚动机制：滚轮 + 滚动条（拖 thumb /
点轨道翻页）+ 光标跟随。滚动偏移**物理像素**（`ScrollState.offset` / `text_scroll` /
`scroll_y`）。语义细节：

- 滚轮**自由滚动**（可把光标滚出视图，不被拉回）；
- **指针离开**输入框/可视区后滚轮失效（`hit` gating）；
- 光标跟随仅在**光标移动**（打字 / 方向键 / 点击 / 拖选）时执行；
- 拖选 edge-scroll（指针移出仍延伸选择）。

### 7. 窗口系统

`ui.window(id)` 可重叠 + 点击置顶（z-order）+ 可拖拽（位置持久于 `UiState.panel_pos`，可经
`pos_handler` 责任链由脚本/动画驱动）。`.width(w)` 固定宽 + 右下角缩放柄（`resize_handle`
通用原语，持久于 `UiState.window_widths` / `UiState.sizes`）。`.placement(Placement::Clip)`
内容严格裁剪（Clip 沙箱）；默认 `.placement(Expand)`（内容自动换行 / 撑高）。

### 8. 文本编辑（`edit.rs` 纯逻辑，可单测）

- 编辑状态机 `apply_frame_edits`：剪贴板（Ctrl+C/V/X/A）→ 选择替换 → IME 上屏 →
  普通字符 → 退格/删除（单行/多行共用）；
- 光标移动 `caret_horiz`（←/→，Shift 扩展）；
- 视觉行定位（多行 ↑/↓/Home/End 按自动换行后的视觉行）；
- 双击按词选择 `word_range` / `extend_word_caret`（CJK 单字成词）；
- 省略号 `ellipsize`（ASCII `"..."`，字符级二分）；
- IME 组合候选浮动提示框 + 候选框定位。

### 9. 绘制与排序（`draw.rs` + `finish`）

命令排序键 `(win, depth, elem, 图形/文字组, seq)`：窗口 z 升序 → 元素录制序 → 元素内
"背景/图形先于文字"。实心填充 / 光标用**字形图集页白纹理**（与字形同页同纹理 → 合批）。
**圆角矩形与矩形渐变都不需要纹理**（v0.4）：渐变走四角顶点色，圆角由 CPU 镶嵌成
三角形（硬体 + 边缘羽化带，`rjw_ui::tess`；羽化宽 = `Theme::feather`）——两者都属于图形组、都用白纹理，
因此圆角图形与文字/白填充**同页同纹理**，窗口内仍合并成一次 draw。
（旧的 `rjw_ui::proc` 32×32 圆角 9-patch 纹理已删除。）

**几何**：`UiBatch` 携带 `vertices` + `indices`（`Tri = [u16; 3]`），UI 全程直出三角形。
索引与顶点同段存放、`Geom::append` 拼接时自动平移，二者永不脱节。为空索引时后端按
「每 4 顶点一组、`TL,TR,BL,BR`」的旧约定回退，保证外部后端兼容。

**窗口级合批（尽力而为）**：`finish` 提交按窗口聚合——同一窗口内**连续的同纹理同状态
同变换**内容顶点合并成整段，一次 `mesh_indexed(..)` → Render2D 一次 `draw_indexed`。
窗口内出现不同纹理或超顶点上限（`MAX_UI_SEG_VERTS`）时自动**切段**（层级保序）。
窗口 FX tint 非白时该段自带实例色（`MeshStyled`）会自成一整段、不参与跨段合批
（这是 `Render2D` 既有语义，与旧 `quads(..).tint(..)` 路径一致）。

**窗口级 FX**（[`Ui::window_fx`] / `WindowFx`）：每个窗口可设 `tint`（整窗混合色，
shader 里 `顶点色 × 实例色`）、`transform` override（叠加在窗口原点上）与 **`anchor`
归一化变换锚点**（`(0.5,0.5)` = 窗口中心，变换绕该锚点旋转/缩放/位移）——**顶点缓存
不变、仅提交时应用到窗口段实例**，支撑整窗口动画（淡入淡出 / 整体位移缩放旋转 / 整窗
染色），移动窗口/改 FX 只更新实例矩阵/颜色而不重建顶点。**无论锚点何值，`transform =
IDENTITY` 时窗口位置恒为原位置**。

**窗口位置约束**（[`WindowBuilder::clamp`](crate::ui::WindowBuilder::clamp) /
[`WindowClamp`](crate::ui::WindowClamp)）：默认 `Screen` 限位（窗口整体不跑出屏幕；
窗口比画面大时仍可拖动——左上角允许到 `屏幕-尺寸`，窗口覆盖画面不钉死）、
`Free` 自由拖出、`Locked` 锁定位置不可拖（脚本仍可定位）。
限位与命中共享同一基准：clamp 用的窗口尺寸**按 id 跨帧持久**（`UiState.window_sizes`，
点击置顶 z 变化不丢尺寸）+ **首帧尺寸未知不 clamp**（窗口出现在应用指定位置，次帧起
收敛一致）——**按下即跳 / 主窗口缩小点不到 / 窗口比画面大拖不动**等位置错位全部消除。
**拖拽中 clamp 边界固定为按下帧窗口尺寸**（`WidgetState.press_size`）——拖拽期间内容
尺寸变化（换行 / 滚动条 / 动态文本）不会把贴边窗口推回，位置**纯跟手、无单帧跳变**；
松开后才按最新尺寸复位。

**位置与交互先于内容录制求解**（`resolve_drag`，`window_impl` / `panel_impl` 共用）：
命中矩形 / 拖拽基准 / clamp 全用**上一帧结算尺寸**（= 屏幕上那个矩形，鼠标事件正是
针对它产生的），因此 `display_pos` 在录制前已知，`Ui::abs_base` 与随后
`translate(display_pos)` 的几何**当帧一致**。旧实现 `abs_base` 取自上一帧位置、几何用
本帧位置，于是**拖拽期间**一切走 `abs_base` 的绝对空间量（文本 `box_clip`、IME 光标、
滑块拖拽基准、下拉浮层位置）都落后一帧的位移量——位移越大越明显（快速拖动时"文字被
裁 / 点击瞬间偏移"）。按下帧**先无条件**建立基准，窗口内子控件（输入框选择 / 滑块 /
滚动条）随后声明本次按下（`press_claimed`）时在内容录制后清除基准——"从输入框上拖拽
= 选择文本、窗口从空白处拖动"的语义不变。

### 10. 焦点与键盘导航（`focus.rs`）

Tab / Shift+Tab / 方向键遍历焦点链；Enter / Space 激活；滑块方向键调值；下拉框展开时
上下切换；Esc 收起/失焦。焦点描边（`Theme::focus`）。

---

## 三、使用方式

```rust
use rjw_ui::{Button, Label, NumberInput, PackSide, Theme, Ui, UiAdd, Viewport};

// 运行时路径（推荐）：一帧可开任意多段 / 任意位置；段收尾自动提交。
fn update(&mut self, ctx: &mut Ctx) {
    let Some(mut f) = ctx.frame() else { return };
    let mut ui = f.ui(Theme::dark().with_radius(8.0));
    ui.label_at(Vec2::new(16.0, 12.0), "FPS: 60");

    // pack 垂直堆叠
    ui.pack_at(Vec2::new(16.0, 56.0), PackSide::Top, |p| {
        if p.button("start", "开始游戏").clicked() { /* ... */ }
        p.checkbox_mut(None, "全屏", &mut self.fullscreen);
        p.divider();                                // 分割线
        p.row(|r| {                                 // 水平等高行
            r.label("HP:");
            r.add(NumberInput::new("hp", &mut self.hp).range(0.0, 100.0));
            if r.button("hp_btn", "应用").clicked() { /* ... */ }
        });
    });

    // 窗口 + Label 溢出处理（容器责任链 builder：`ui.window(id).pos(..).width(..)`
    // 固定宽 + 右下角缩放；`.placement(Placement::Clip)` = 强制裁剪；`.style(..)` = 逐窗口样式覆盖）
    ui.window("win")
        .pos(Vec2::new(560.0, 240.0))
        .width(220.0)
        .show(|w| {
            w.label("标题（缩窄窗口自动换行）");
            w.add(Label::new("省略标签……").ellipsis());
        });
    ui.finish();                                    // 段收尾（可省略：作用域结束即收尾）
    f.submit(&mut self.cam, Clear::color(Color::rgb(0.05, 0.05, 0.08)));
}
```

**低层路径**（自己持有 `UiState`；此时帧级账也归调用方）：

```rust
let mut ui = Ui::begin(&window, &mut text, &mut state)
    .capture(&mouse, &keyboard)
    .theme(Theme::dark().with_radius(8.0))      // with_font_family / with_font_size / with_radius / with_border_w / with_feather / ...
    .scale_factor(ctx.scale_factor().unwrap_or(1.0))
    .build();
// ... 录制（同上）...
ui.end_frame(r2d_ui);   // 帧收尾 + 提交（UI 无需相机/视口参数：屏幕固定变换由运行时 UI 层相机决定）
```

### 常用控件 / 方法速查

| 分类 | 方法 / 控件 |
|---|---|
| 容器 | `ui.window(id)`/`ui.panel()`/`ui.modal(id)` builder（选项链 + `.show(..)`，已统一旧的 `window_at*` / `modal_at*`）/ `panel_at` / `pack_at` / `grid_at` / `flex_at` / `scroll_at` / `list_at` / `row` / `view_at` |
| 占光标便捷 | `p.label` / `p.button` / `p.checkbox(_mut)` / `p.radio` / `p.slider` / `p.text_input` / `p.text_area(_nw)` / `p.combo` / `p.divider` / `p.row` |
| Widget builder | `Label`（`wrap` / `ellipsis`）/ `Button` / `Checkbox` / `Divider`（`p.add(...)` 放置） |
| 组合控件 | `NumberInput`（拖动调值 + 输入）/ `ColorPicker`（内联色块 → 弹出取色面板：u8/HEX/F 呈现 + HSV 平面/色相条 + 通道行 + 可选 A）/ `FontModal`（字体切换） |
| 绝对定位 | `*_at(pos, …)`、`add_at`、`divider_at`、`anchor_pos(Anchor::…)`（视口锚定） |

### 输入屏蔽

`UiState::text_focus()` 为 `Some` 表示有**文本控件**持有焦点（按钮/滑块的 Tab 焦点不算）——
应用处理快捷键（`R` 重置、`Esc` 退出等）前应检查并跳过。

---

## 四、写一个自定义控件

实现 `Widget` trait（`size` 测量 + `ui` 渲染/交互），属性用 `Option` 字段 + builder setter：

```rust
use rjw_ui::{Response, Ui, Widget};

pub struct MyButton<'a> { id: &'a str, label: &'a str }
impl<'a> MyButton<'a> {
    pub fn new(id: &'a str, label: &'a str) -> Self { Self { id, label } }
}
impl Widget for MyButton<'_> {
    fn size(&self, ui: &mut Ui) -> Vec2 {
        let style = ui.theme.button.clone();
        let t = ui.text_size(self.label, style.font_size, style.font_family.as_deref());
        Vec2::new(t.x + style.padding.x * 2.0, t.y + style.padding.y * 2.0)
    }
    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        // 复用现有控件 / 原语；交互用 ui.hit_abs(&abs, &rect) / ui.mouse_left() /
        // ui.state_mut().widget(&id_for)
        // （状态键 / 焦点 / **控件级遮挡身份**都用**绝对 ID**：let abs = ui.id_for(self.id);）
        let st = ui.button_at(self.id, rect, self.label);
        st.into()   // ButtonState → Response
    }
}
```

公开原语：`text_size(_wrap)`（测量）、`hit_abs(&绝对ID, &rect)`（命中）/ `hit_body_abs(&rect)`
（窗口 / 面板本体）/ `mouse_left` / `mouse_logical`、
`claim_press` / `register_focus` / `key_click`（焦点；**收绝对 ID** `&ui.id_for(id_relative)`）、
`state_mut().widget(&id_for)`（持久状态 +
`hit::update_drag/update_interact`）、`push_solid_rect` / `push_border_rect` /
`push_text_rect(_noclip)` / `push_panel_like` / `resize_handle`（绘制）。

**注意**：`rect` 是相对当前容器 origin 的**局部坐标**；坐标换算、窗口遮挡、控件级遮挡、
裁剪过滤都由父级（容器/沙箱）负责，控件只需"相对自己"绘制与命中。

---

## 五、调试与调优

- `debug_layout(true)`：给每个控件/容器画青色描边（布局/命中区域可视化）；
- 屏幕空间调试图元：`ui.debug_line` / `debug_rect_outline` / `debug_circle_outline` /
  `debug_cross` / `debug_grid`（覆盖在 UI 之上；世界坐标调试图元见 `rjw_2d_render::debug_draw`）；
- 窗口诊断：`ui.window_order()` / `ui.window_under_mouse()` /
  `UiState::last_press_window()` / `UiState::occluded_hits()`；
- 性能：`UiState::stats`（`finish` 各阶段 µs + 窗口缓存命中/未命中）；示例 `eg260818UI`
  每 120 帧打印 `[perf]` 均值，`--auto-drag` 走"拖动中内容逐帧变化"最坏路径。
- 文本缓冲缓存（`UiState::text_buffers`，帧级近似 LRU）：静态标签每帧命中零排版；
  动态文本（FPS/日志）不会冲掉静态缓存。
- **窗口顶点缓存键 = 命令全量签名 ⊕ 字形图集区域失效世代号**（`rjw_text::Text::atlas_revision`
  = `rjw_atlas::DynamicAtlas::revision`）。顶点里烘着**最终 UV**（字形与 WHITE 基础纹理都取自
  字形图集），图集一旦重排或复用已逐出条目的槽位，旧 UV 就指向别的像素（"陈旧文字"/"背景
  消失"），而命令内容没变 ⇒ 必须靠世代号强制重建。重建期由 `rjw_text` 重新校验字形
  （位置表 **且** 图集仍在才算可用，见 `chain::location_usable`）并重新光栅化缺失字形。

---

## 六、文档导航

- `API_REFERENCE.md`：完整 API 表；
- `GUI_GUIDE.md`：示例驱动的使用引导；
- `WIDGET_GUIDE.md`：Widget trait / 尺寸契约 / 自定义控件 / 裁剪分层；
- `UI_NEEDS.md`：需求/TODO 便条（含实现说明）；
- `UI_DRAG_FLICKER_FIX.md`：窗口顶点缓存与拖动闪烁修复记录。
