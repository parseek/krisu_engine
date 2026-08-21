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

布局是 **DOM 风格自动尺寸**（叶子控件由内容撑开、容器闭包结束时按子控件结算），几何
管理是 **Tkinter 风格**（`pack` 堆叠 / `grid` 网格 / `*_at` 绝对定位）。

---

## 二、原理

### 1. 一帧的生命周期（录制 → 提交）

```
Ui::begin(window, &mut text, &mut state)
   .capture(&mouse, &keyboard)   // 拷贝输入快照（与设备解耦）
   .theme(theme).scale_factor(dpi).build()
   → 录制：ui.label_at / pack_at / window_at / add(...)  …
   → Ui::finish(&viewport, &mut render2d)
```

- **录制阶段**：每次控件调用把一条/多条 `UiDraw` 命令压入队列（坐标是**相对当前容器的
  局部逻辑像素**，容器弹出时统一平移成绝对）。这一阶段不碰 GPU，也不碰输入设备（快照）。
- **提交阶段**（`finish`）：排序 → 按窗口分组 → 收集成四边形 → 提交到独立的 `Render2D`
  （UI 必须 `set_sorting(false)`，绘制顺序由 UI 自己管理）。

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
- **固定宽容器**（`window_at_w`）：子项宽度 clamp 到固定值、高度自然（egui 风格）；
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

### 4. 命中与交互状态机（`hit.rs` + `WidgetState`）

控件交互三件套：`hit_abs(&rect)`（矩形命中 + 窗口遮挡 + **强制层命中过滤**）、
`mouse_left()`（左键含边沿）、`hit::update_interact` / `update_drag`（跨帧状态机）。

关键约定：
- **窗口遮挡**：重叠区域只让鼠标下最上层窗口响应（点击穿透修复）；
- **press_claimed**：自身有拖拽语义的控件（滑块 / 滚动条 / 文本框 / 缩放柄）按下时置位，
  阻止外层窗口/面板把本次按下当拖拽基准（窗口内拖滑块不连窗口动）；
- **拖动需位移** ≥ 3 物理像素才激活（纯点击不拖拽 → 子控件正常响应）。

### 5. 裁剪分层（`view.rs` + `draw.rs`）

裁剪分**两层**：

- **强制层（硬裁剪）**：ScrollView 可视区 / Clip 沙箱（`window_at_strict`、文本框）。
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

`window_at` 可重叠 + 点击置顶（z-order）+ 可拖拽（位置持久于 `UiState.panel_pos`，可经
`pos_handler` 责任链由脚本/动画驱动）。`window_at_w` 固定宽 + 右下角缩放柄（`resize_handle`
通用原语，持久于 `UiState.window_widths` / `UiState.sizes`）。`window_at_strict` 内容严格
裁剪（Clip 沙箱）；默认窗口是 Expand 语义（内容自动换行 / 撑高）。

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
圆角矩形（9-patch）纹理也**塞进字形图集**（[`rjw_text::Text::insert_user_texture`]，
白色 + alpha、顶点色 tint）——圆角图形与文字/白填充同页同纹理 → 窗口内合并一次 draw；
线性渐变仍在独立程序化 Atlas 页。

**窗口级合批（尽力而为）**：`finish` 提交按窗口聚合——同一窗口内**连续的同纹理同状态
内容**顶点合并成整段，一次 `add_quads_styled` → Render2D 一次 `draw_indexed`（命中其
QuadVertices 合批）。窗口内出现不同纹理（白纹理 / 圆角渐变程序化页）或不同混合状态、
或超顶点上限（`MAX_UI_SEG_VERTS`）时自动**切段**（层级保序）。

**窗口级 FX**（[`Ui::window_fx`] / `WindowFx`）：每个窗口可设 `tint`（整窗混合色，
shader 里 `顶点色 × 实例色`）、`transform` override（叠加在窗口原点上）与 **`anchor`
归一化变换锚点**（`(0.5,0.5)` = 窗口中心，变换绕该锚点旋转/缩放/位移）——**顶点缓存
不变、仅提交时应用到窗口段实例**，支撑整窗口动画（淡入淡出 / 整体位移缩放旋转 / 整窗
染色），移动窗口/改 FX 只更新实例矩阵/颜色而不重建顶点。**无论锚点何值，`transform =
IDENTITY` 时窗口位置恒为原位置**。

**窗口位置约束**（[`WindowBuilder::clamp`](crate::ui::WindowBuilder::clamp) /
[`WindowClamp`](crate::ui::WindowClamp)）：默认 `Screen` 限位（窗口整体不跑出屏幕）、
`Free` 自由拖出、`Locked` 锁定位置不可拖（脚本仍可定位）。

### 10. 焦点与键盘导航（`focus.rs`）

Tab / Shift+Tab / 方向键遍历焦点链；Enter / Space 激活；滑块方向键调值；下拉框展开时
上下切换；Esc 收起/失焦。焦点描边（`Theme::focus`）。

---

## 三、使用方式

```rust
use rjw_ui::{Button, Label, NumberInput, PackSide, Theme, Ui, UiAdd, Viewport};

// 每帧：
let mut ui = Ui::begin(&window, &mut text, &mut state)
    .capture(&mouse, &keyboard)
    .theme(Theme::dark().with_radius(8.0))      // with_font_family / with_font_size / with_radius / ...
    .scale_factor(ctx.scale_factor().unwrap_or(1.0))
    .build();

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
// 等价旧 `window_at_w`；`.strict()` = 强制裁剪；`.style(..)` = 逐窗口样式覆盖）
ui.window("win")
    .pos(Vec2::new(560.0, 240.0))
    .width(220.0)
    .show(|w| {
        w.label("标题（缩窄窗口自动换行）");
        w.add(Label::new("省略标签……").ellipsis());
    });

// 提交
let viewport = Viewport::new(render2d.size(), Vec2::ZERO);
r2d_ui.set_mvp(viewport.vp_matrix());           // UI 的 Render2D 须 set_sorting(false)
ui.finish(&viewport, r2d_ui);
```

### 常用控件 / 方法速查

| 分类 | 方法 / 控件 |
|---|---|
| 容器 | `ui.window(id)`/`ui.panel()`/`ui.modal(id)` builder（选项链 + `.show(..)`，统一 `window_at*` / `panel_at` / `modal_at*`）/ `pack_at` / `grid_at` / `flex_at` / `scroll_at` / `list_at` / `row` / `view_at` |
| 占光标便捷 | `p.label` / `p.button` / `p.checkbox(_mut)` / `p.radio` / `p.slider` / `p.text_input` / `p.text_area(_nw)` / `p.combo` / `p.divider` / `p.row` |
| Widget builder | `Label`（`wrap` / `ellipsis`）/ `Button` / `Checkbox` / `Divider`（`p.add(...)` 放置） |
| 组合控件 | `NumberInput`（拖动调值 + 输入）/ `FontModal`（字体切换） |
| 绝对定位 | `*_at(pos, …)`、`add_at`、`divider_at`、`anchor_pos(Anchor::…)`（视口锚定） |

### 输入屏蔽

`UiState::capturing_text()` 为真表示有输入框持有焦点——应用处理快捷键（`R` 重置、
`Esc` 退出等）前应检查并跳过。

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
        // 复用现有控件 / 原语；交互用 ui.hit_abs / ui.mouse_left / ui.state_mut().widget(&id_for)
        // （状态键 / 焦点用**绝对 ID**：let id_for = ui.id_for(self.id);）
        let st = ui.button_at(self.id, rect, self.label);
        st.into()   // ButtonState → Response
    }
}
```

公开原语：`text_size(_wrap)`（测量）、`hit_abs` / `mouse_left` / `mouse_logical`（命中）、
`claim_press` / `register_focus` / `key_click`（焦点；**收绝对 ID** `&ui.id_for(id_relative)`）、
`state_mut().widget(&id_for)`（持久状态 +
`hit::update_drag/update_interact`）、`push_solid_rect` / `push_border_rect` /
`push_text_rect(_noclip)` / `push_panel_like` / `resize_handle`（绘制）。

**注意**：`rect` 是相对当前容器 origin 的**局部坐标**；坐标换算、窗口遮挡、裁剪过滤都
由父级（容器/沙箱）负责，控件只需"相对自己"绘制与命中。

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

---

## 六、文档导航

- `API_REFERENCE.md`：完整 API 表；
- `GUI_GUIDE.md`：示例驱动的使用引导；
- `WIDGET_GUIDE.md`：Widget trait / 尺寸契约 / 自定义控件 / 裁剪分层；
- `UI_NEEDS.md`：需求/TODO 便条（含实现说明）；
- `UI_DRAG_FLICKER_FIX.md`：窗口顶点缓存与拖动闪烁修复记录。
