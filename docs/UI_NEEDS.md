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
* ✅ 对于 Resizable 窗口，缩小宽度后，Label 等**所有控件**需要处理超过框后字符（Label 默认在父级可用宽内**自动换行**、`.ellipsis()` 显式“…”省略；Button/勾选/下拉文本自动省略；`window_at`/`window_at_w` 默认 Expand 语义，新增 `window_at_strict` 严格裁剪）
* ✅ 文本框双击“扩散式”选择（双击选中“词”——CJK 单字成词/空白分隔/字母数字连续段；按住拖拽按词边界扩散）
* ✅ 分割线（`p.divider()` 占光标 / `ui.divider_at` / `Divider` widget；`Theme.divider` 样式）
* 
* ✅ 部分代码可以合并简化、抽象化、责任拆分（**View 沙箱** `crates/rjw_ui/src/view.rs`：裁剪分层（强制层/软层）+ 可用宽度 + 命中过滤，ScrollView/文本框/严格窗口共用；`edit.rs` 收纳纯文本逻辑：`apply_frame_edits` 编辑状态机 / `caret_horiz` / `word_range` / `ellipsize` / 剪贴板，单行/多行去重；`Metric<T>` 物理/逻辑单位包装，内部计算一律物理像素；`resize_handle` 通用拖拽缩放原语）

Widget：
* ✅ 设置可选的 min、max 大小，可以设置 DisableAutoExpansion, LimitedInParent, UnlimitedExpansion（`SizeConstraints{min_w,max_w,min_h,max_h}` 四字段全 `Option<f32>` + `Expansion` 三模式；`Ui::add` 统一 clamp/调整）
* ✅ 可选的允许用户拖拽在可选的范围内缩放（`Ui::resize_handle` 通用原语 + `Widget::resizable()` 声明 + `UiState::sizes` 持久尺寸；`window_at_w` 宽度缩放已基于它）
* ✅ Widget 输入数据由父级换算、过滤（父级负责局部坐标换算 `abs_base`、窗口遮挡、`press_claimed` 拖拽占用；Clip 沙箱外命中失效并入 `hit_abs`）
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