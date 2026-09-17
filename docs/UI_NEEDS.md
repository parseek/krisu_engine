便条   
UI模块的需求：  

* ✅ 多行文本：自动换行（Ui::text\_area\_at）与不自动换行 + 水平滚动（Ui::text\_area\_at\_nw）模式
* ✅ 不同字体：默认、SimHei、Sarasa Mono SC（demo 字体 Modal，Theme::with\_font\_family 级联）
* ✅ （非数字输入被屏蔽；拖拽基准用独立状态 ID `{id}::grip`）
* ✅ 窗体：悬停、拖动时保持普通 Arrow（光标由 set\_cursor / 内置规则管理）而不是 <-> 也不是 <-↑↓->
* ✅ 视口滑条：普通 Arrow
* ✅ 字体切换：Modal 窗口（builtin::FontModal，Ui::modal\_at 基础原语）、Input + PreviewInput + 确定键取消键水平排列右对齐（遮罩默认全屏，大小/颜色由 Theme.modal 设置）
* ✅ 移动窗口时强制 Arrow（修复拖动中 <-> 的 BUG）
* 控件绘制/事件/状态处理：见 `crate::widget` 模块文档 / WIDGET\_GUIDE.md（绘制 = Ui 公开原语推 UiDraw；事件 = 输入快照 + hit\_abs/update\_drag 状态机；状态 = UiState.widgets 按 ID 持久）
* ✅ 支持水平滚轮（触控板）：单行输入框 / 不换行多行文本的水平滚动
* ✅ 未悬停任何 UI 内容时不主动设置光标图案（保留应用自定义光标；仅离开 UI 时清一次回 Default）
* ok 滚轮滚动多行文本还是会被光标卡住（需要滚轮自由滚动、光标可滚出视图；光标跟随仅打字/方向键/拖选时）
* ok 数字输入框（builtin::NumberInput）：拖动框整体调值（EwResize 光标，**向右拖 = 增加**，鼠标光标在窗口内 **warp**，松开结束）；点击文本框：进入输入模式 -> 禁用拖动 -> 全选（**仅获取焦点后第一次**，不然永远无法靠鼠标选择部分文本） -> I 型光标输入 -> 直到失去焦点 -> 恢复拖动模式
* ok 未达到 max 大小的控件按内容自动改大小（grid 单元格缓存允许缩小；测量式控件本就逐帧重测；还有一些 Button 的动态展示，就比如 字体...）
* ok 窗口可**通过鼠标**缩放（如果要UI模块自动改大小请用max）
* ok Modal 与背景还是不会主动置顶（点击对话框/背景不触发 z 提升，被其他上去的窗口挡住）
* ok（自动换行还行，） 预览文本不会被裁剪
* 
* ✅ 多行文本光标没有被裁剪（文本框 = 控件内 **Clip 子沙箱**：光标/高亮/文本命令自动受输入框强制裁剪，滚出视图不画出框；外层 ScrollView 裁切一并生效）
* ✅ 多行文本的 ScrollBar（内容超出可视区显示垂直滚动条：拖 thumb / 点轨道翻页 / 滚轮；滚动条条带按下不建立文本选择）
* ✅ 单行文本会滚动被文字光标卡住（滚动跟随仅"光标移动"时执行——打字/方向键/点击/拖选；滚轮自由滚动后不被光标拉回）
* ✅ 即使鼠标指针已经离开文本框控件，滚动仍然有效（滚轮 gating 加 `hit`：指针离开输入框/ScrollView 可视区后不再滚动；拖选 edge-scroll 不受影响）
* ✅ 对于 Resizable 窗口，缩小宽度后，Label 等**所有控件**需要处理超过框后字符（Label 默认在父级可用宽内**自动换行**、`.ellipsis()` 显式“…”省略；Button/勾选/下拉文本自动省略；`ui.window(id)` 默认 `Placement::Expand` 语义，`Placement::Clip` 严格裁剪）
* ✅ 文本框双击“扩散式”选择（双击选中“词”——CJK 单字成词/空白分隔/字母数字连续段；按住拖拽按词边界扩散）
* ✅ 分割线（`p.divider()` 占光标 / `ui.divider_at` / `Divider` widget；`Theme.divider` 样式）
* 
* ✅ 部分代码可以合并简化、抽象化、责任拆分（**View 沙箱** `crates/rjw_ui/src/view.rs`：裁剪分层（强制层/软层）+ 可用宽度 + 命中过滤，ScrollView/文本框/严格窗口共用；`edit.rs` 收纳纯文本逻辑：`apply_frame_edits` 编辑状态机 / `caret_horiz` / `word_range` / `ellipsize` / 剪贴板，单行/多行去重；`Metric<T>` 物理/逻辑单位包装，内部计算一律物理像素；`resize_handle` 通用拖拽缩放原语）

Widget：
* ✅ 设置可选的 min、max 大小，可以设置 DisableAutoExpansion, LimitedInParent, UnlimitedExpansion（`SizeConstraints{min_w,max_w,min_h,max_h}` 四字段全 `Option<f32>` + `Expansion` 三模式；`Ui::add` 统一 clamp/调整）
* ✅ 可选的允许用户拖拽在可选的范围内缩放（`Ui::resize_handle` 通用原语 + `Resize` 枚举（`None`/`Horizontal`/`Both`，v0.3 起取代 `Widget::resizable()` 裸布尔）+ `UiState::sizes` 持久尺寸；`ui.window(id).width(w)` 宽度缩放、`resizable_text_input_at` / `resizable_text_area_at` 均基于它）
* ✅ Widget 输入数据由父级换算、过滤（父级负责局部坐标换算 `abs_base`、窗口遮挡、**控件级遮挡**（同窗口内重叠控件只让最上层响应，`hit_abs(绝对ID, rect)`）、`press_claimed` 拖拽占用；Clip 沙箱外命中失效并入 `hit_abs`）
* ✅ 绘制方面提供服从内容裁剪的绘制方法和不服从裁剪的方法（`push_text_rect_noclip` 等：不附加软层、内容自洽；**仍服从 ScrollView 强制层**——父级强制裁切躲不掉，无 Scroll 的普通容器本无强制层）
* ✅ min_width、max_width、min_height、max_height 皆是 `Option<f32>`（`SizeConstraints` 四字段）

Issue: 
* ✅ 自动换行仅“逻辑”实现，渲染并未自动换行（Label 默认 LimitedInParent 自动换行时渲染也传同一宽度的换行缓冲——`wrap_buffer(rect.w)`，逻辑与渲染一致）
* ✅ 省略标志使用ASCII的“...”而不是“…”（`edit::ellipsize` 省略号改为 ASCII 三点 `"..."`）
* ✅ 可以加入水平的类似 `{Label} {NumberInput} {NumberInput} {Button}` 的排列（新增占光标的 `p.row(...)` 水平行容器：PackSide::Left 堆叠 + 结算后补记父容器光标，宽 = 子项结算、撑大父级）

Issue 20260820 20:15:
* ✅ eg 的下方提示文本完全被挡住了（😂），那这样可不可以将其排版到窗口 LeftBottom？（新增视口锚定：`Ui::anchor_pos(Anchor::BottomLeft, …)` + `Ui::viewport_size()`；示例底部提示锚定左下角）
* ✅ 因为实际上 Camera2D 在独立的 Render2D 里没有任何含义，撤掉，仅用接收 Viewport 参数（大小位置）即可（新增 `rjw_transform::Viewport{pos,size}`；`Ui::finish(&Viewport, …)`；Render2D `set_mvp(viewport.vp_matrix())`；Camera2D 保留给世界渲染）
* ✅ 窗口 A 的“标题”（实际上是 Label）没有任何对于越界的处理。解决方案：默认指定自动换行（`UiAdd::label` 委托 `Label` widget——默认 LimitedInParent 自动换行）
* ✅ 对于水平排列，出现了 Label 与其他控件“基线不一致”的情况，HP Label 整体偏上了。方案：**等高**（`Theme.row_h` 单行高度 + `row` 内全部子项强制等高 + 各自内容垂直居中 → 文字中心线对齐）：
  ```
  ----------
  ↑ ascent（基线往上高度）
  -- 基线 --
  ↓ descent（基线往下高度）
  ----------
  ```
* ✅ Checkbox 的绘制：中心表示 `true` 的蓝色矩形 = 外框 **shrink**（减法内缩 `floor(border_w·scale) + floor(CHECKBOX_INNER·scale)`，非写死偏移）：
  ```rust
  const CHECKBOX_INNER: f32 = 1.0;
  let outer_box = rect(CHECKBOX_SIZE).into_physical(scale_factor).floor();
  draw_inner_border(outer_box, CHECKBOX_BORDER_W.into_physical(scale_factor).floor())
  let inner_box = outer_box.shrink(CHECKBOX_BORDER_W.into_physical(scale_factor).floor() + CHECKBOX_INNER.into_physical(scale_factor).floor())
  if checkbox_true {
      draw_soild(inner_box);
  }
  ```

ISSUE:
* 然后是合批问题：能不能以窗口为单位合并 DrawCall？对于现在的 Render2D 依赖，每个窗口可传入包含位移信息的 Transform 矩阵（可以实现在不改变顶点的情况下拖动）、混合颜色信息，可以方便整窗口动画&特效

需求 20260821（逐项落地，每项都带引擎测试 + 示例演示 + `docs` 更新）：
* ✅ 标题栏 + 窗口关闭收缩按钮（`WindowBuilder::{title, close_button, shrink}`；`shrink(显示按钮: bool, &mut 收起状态)`；关闭 = 整窗短路不占遮挡矩形，重开由应用负责；脚本验证 `--sim-chrome`）
* ✅ 拖拽按钮（右下角缩放柄）样式可配（`GripStyle{GripShape::Squares/Bars/Hidden, color, size, step, count}` + `PanelStyle::with_grip*`；`Hidden` 仍可拖）
* ✅ 直角（radius 0）窗口的背景图不再被丢掉（`push_panel_img_cmds` 把"背景刷形状"与"图/边框"解耦；回归测试 A/B 可复现）
* ✅ 字重选项（`Theme::font_weight` 全局令牌 + `FontModal` 字重下拉七档 300…900；字重进排版缓冲缓存键 + 窗口几何签名前缀，`--sim-weight` 实测宽度 112→116）
* ✅ 文件导入（系统文件选择器 `rfd`：图片 → 背景纹理（`Render2D::gpu().texture` → `ImageBg`），字体 → 运行时文本子系统；`Text::load_font_data` 现返回**新增族名**并自动切过去；`--sim-import` 脚本化验证，实测默认族 340.0 vs 导入族 348.0）
* ✅ 阴影颜色（主题调节窗口「投影」滑杆后跟 `ColorPicker`（可拖 alpha），`ShadowStyle { color, .. }`；顶点 RGB 原样带出，`--sim-shadow` 实测主题值 0,0,0/0.47 → 0.9,0.15,0.1/0.55）
* ✅ 主题调节窗口：每根滑杆后跟 `NumberInput`（Slider 后 NumberInput；`--sim-tuner` 实测拖数字条手柄 radius 8→18、拖滑杆 →0，主题圆角同步）
* ✅ 数字条手柄宽度公开为 `rjw_ui::GRIP_W`（脚本算坐标不再写死 20；`RJ_NUM_TRACE=1` 打印矩形切分 + 拖拽状态机）
* ✅ 菜单栏（`Ui::menu_bar` 横向触发器 + **闭包下拉面板**：`MenuCtx` 提供 `item` / `item_checked` / `caption` / `separator` 并 `Deref` 到 `Window` ⇒ 文本输入 / 分割线 / 按钮 / 横向排版都能放；展开状态 `UiState::menu_open`，点项 / 点栏外 / Esc 收起；下拉面板 `WindowClamp::Locked` + `WIN_TOPMOST`（**拖不动、恒在最上**）、内容与菜单项文字列对齐；示例三菜单「文件/视图/帮助」，**左侧竖排主菜单保留**；`--sim-menu` 两阶段实测；**面板实现与 `Dropdown` 共用**——见下条）
* ✅ 分段按钮组 `Segmented`（互斥选项**拼在一起**：相邻段共享边、只有外侧角圆、选中段高亮；段间分隔线与 `border_w` 解耦，边框归零也分得开；`--sim-tuner` 阶段 3 点段实测 `preset=2` + 角点单测）
* ✅ `border_w = 0` 的可见性兜底（未勾选 `Checkbox` 改画实心底——否则勾选框整个消失、看起来"控件严重错位"）
* ✅ 修：`FontModal` 一帧被录两次（面板 + 文本画两遍 = "文本输入重复"；现在有 `modal_recorded` 帧内断言守着）
* ✅ 修：`GripShape::Bars` 三条横杠改用实心矩形（图标羽化在小尺寸下糊成一坨 = "三横是斜的"）
* ✅ 菜单栏视觉：内边距 = `ComboStyle::item_pad_x`（菜单项 / 标题 / 分割线天然同列；勾选**方框**画在项内容里、不占内边距）、面板宽取上一帧结算宽（高亮与分割线**铺满面板**）、分割线自绘（`Divider` 在自动宽容器里退回 120 ⇒ "Menu 分割线错位"）、`caption` 小字号 + `text_muted` 做分组标题；`RJ_MENU_TRACE=1` 打印行矩形（实测 `item/separator/caption` 全 `x=8 w=224`，DPI 1.5；关键是三者相同）
* ✅ 修：combo 下拉**勾选列恒留位**（未选中项文字不再比选中项凸出一个图标宽 —— "一列像素突兀"）
* ✅ 字体弹窗预览不再重复显示输入框里的字体名（"文本输入重复"）
* ✅ 主题调节窗口去掉尾部"当前值一览"两行标签（每行都有滑杆 + 数字条，就近可读）
* ✅ 缩放柄「斜线」档：三条 45° 斜线（**首端点在水平线等距 / 末端点在竖直线等距** ⇒ 互相平行、垂直间距相等），单测钉住几何；柄方框 1.5× 免得羽化糊成一片；`GripShape::{Squares, Bars, Diagonal, Hidden}`，demo 默认斜线
* ✅ 窗口拖拽缩放：`.resize(allow: bool, axes: Resize)`（显式 bool + 轴向）+ **Y 轴缩放**（高度持久于 `UiState::window_heights`）；`allow = false` ⇒ 不画柄也不响应拖拽（菜单下拉用它，去掉 resizable）；判定抽成 `resolve_window_resize` 单测；`--sim-resize` 实测 `328×145 → 388×185`（正好等于注入位移 +60/+40）
* ✅ 修：**高度被用户固定后内容不裁剪**（TTT 窗口缩小后标签画到面板外）⇒ `window_content_clipped(strict, fixed_h)`（固定高 ⇒ 视口裁剪；固定宽不触发）+ 单测
* ✅ 菜单下拉：勾选标记改成**方框 CheckBox**（画在菜单项内容里、方框列恒留位）、行高收紧（`item_h` 里固定 6px → 2px）、左内边距只留 `item_pad_x`（用户改的），并 `.resize(false, Resize::None)`（**不再 resizable / 不画柄**）
* ✅ **按钮下拉菜单 `Dropdown`（图一 + 图二简并）**：`Dropdown` 是普通 `Widget` ⇒ `UiAdd::add` 加进任何容器；① `Dropdown::options(id, label, &mut u32, &[&str])` = 选项列表模式（选中行打勾 + 整行高亮 + 点击写回 + ↑/↓ 切换），② `Dropdown::new(id, label).menu(|m| ..)` = 富内容模式（`m: MenuCtx`，`Deref` 到 `Window`）⇒ **菜单内又可以 `UiAdd::add`**（文本输入 / 分割线 / 菜单项 / 横向排版 / 再嵌 `Dropdown::side(Right)` 当子菜单）
* ✅ **下拉面板只有一套实现**（`widgets::menu::popup_show`）：菜单栏与 `Dropdown` 共用（哨兵 z + 锁定位置 + 无缩放柄 + 面板样式 + 宽度收敛 + 点外/Esc/点项收起）；`Ui::combo_at` 退化成糖（签名 / 行为不变，`FontModal` 照旧用）；`UiState::combo_open()` 与 `menu_open()` 对称可读
* ✅ 新增"点在任意 `WIN_TOPMOST` 浮层上不收起"（**子菜单**不被外层菜单误关）；公开几何助手 `item_h` / `popup_padding` / `popup_origin`（脚本算坐标与引擎同源）
* ✅ `--sim-dropdown` 五段实测全 `[OK]`：① 点触发器开下拉且面板原点 = `popup_origin(触发器, Below)`（实测 `(990,59)`）；② 点选项 ⇒ `dd_opt_idx=0` + 自动收起 + 面板消失；③ 富内容下拉同样开（`(1290,59)`）；④ `text_focus=dd_file::popup/dd_filter` ⇒ **菜单里的文本输入真可聚焦**；⑤ 点菜单项 ⇒ 计数 =1 + 自动收起