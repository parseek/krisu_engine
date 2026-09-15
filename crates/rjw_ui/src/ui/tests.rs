//! `ui` 模块的单元测试（从 `ui.rs` 拆出；无 GPU 依赖的纯逻辑回归）。
//!
//! 这些测试原本内联在 `ui.rs` 末尾（约 700 行），拆出后 `ui.rs` 只留生产代码。
//! 覆盖：文本编辑原语 / 对齐转换 / 内容签名 / 组排序 / 命令分桶等价性 /
//! 窗口限位与拖拽 / 命中与焦点 / 字符边界安全等——全部不需要 GPU。

use super::*;
use crate::gpu_batch::{GROUP_GRAPHIC, GROUP_TEXT};
use crate::edit::{remove_at, remove_before};

#[test]
fn insert_char_at_handles_caret() {
    let mut s = String::from("abc");
    insert_char_at(&mut s, 1, 'X');
    assert_eq!(s, "aXbc");
    insert_char_at(&mut s, 0, '!');
    assert_eq!(s, "!aXbc");
    insert_char_at(&mut s, 99, 'z');
    assert_eq!(s, "!aXbcz", "caret 越界 clamp 到末尾");
    // 多字节字符按 char 索引
    let mut s = String::from("中文ab");
    insert_char_at(&mut s, 2, '字');
    assert_eq!(s, "中文字ab");
}

#[test]
fn remove_before_and_at() {
    let mut s = String::from("abc");
    assert_eq!(remove_before(&mut s, 2), 1);
    assert_eq!(s, "ac");
    remove_at(&mut s, 0);
    assert_eq!(s, "c");
    // 多字节
    let mut s = String::from("中文");
    assert_eq!(remove_before(&mut s, 2), 1);
    assert_eq!(s, "中", "caret=2 删除前一个字符（'文'）");
    remove_at(&mut s, 0);
    assert_eq!(s, "");
}

#[test]
fn text_align_conversion_roundtrip() {
    assert_eq!(Align::from(TextAlign::Left), Align::Left);
    assert_eq!(Align::from(TextAlign::Center), Align::Center);
    assert_eq!(Align::from(TextAlign::Right), Align::Right);
    assert_eq!(TextAlign::from(Align::Left), TextAlign::Left);
    assert_eq!(TextAlign::from(Align::Center), TextAlign::Center);
    assert_eq!(TextAlign::from(Align::Right), TextAlign::Right);
    // 其他 Align（Justified/End）归入 Center
    assert_eq!(TextAlign::from(Align::Justified), TextAlign::Center);
    assert_eq!(TextAlign::from(Align::End), TextAlign::Center);
}

#[test]
fn cmd_sig_invalidates_on_content_change() {
    // 回归防线：**内容签名必须区分一切渲染相关字段**（尤其颜色 / 文本）——
    // 否则 hover/click 变色、传入值改变时窗口 / win=0 放置缓存误判"未变"而
    // 复用陈旧顶点（历史 bug：轻量摘要漏颜色位 → 交互不刷新）。窗口与 win=0
    // 子槽缓存共用本签名，故在此直接验证。
    use std::hash::{Hasher};
    let base = UiDraw {
        depth: 0,
        seq: 1,
        win: 0,
        elem: 0,
        rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        clip: None,
        kind: DrawKind::Solid(Color::rgba_u8(10, 20, 30, 255)),
    };
    fn sig(d: &UiDraw) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        cmd_sig_hash(&mut h, d);
        h.finish()
    }
    // 同一命令哈希确定（命中复用前提）
    assert_eq!(sig(&base), sig(&base), "同命令签名应确定");
    // 颜色位变化 → 签名变化（hover/click 变色必须使缓存失效）
    let hovered = UiDraw { kind: DrawKind::Solid(Color::rgba_u8(11, 20, 30, 255)), ..base.clone() };
    assert_ne!(sig(&base), sig(&hovered), "颜色变化必须改变签名");
    // 文本内容变化 → 签名变化（传入值改变必须使缓存失效）
    let text_kind = |s: &str| DrawKind::Text {
        text: s.into(),
        size: 14.0,
        color: Color::WHITE,
        align: TextAlign::Left,
        valign: TextVAlign::Center,
        family: None,
        clip: None,
        buf: None,
    };
    let t1 = UiDraw { kind: text_kind("on"), ..base.clone() };
    let t2 = UiDraw { kind: text_kind("off"), ..base.clone() };
    assert_ne!(sig(&t1), sig(&t2), "文本内容变化必须改变签名");
}

#[test]
fn cmd_sig_covers_image_fields() {
    // 背景图的每个渲染输入都必须进签名：否则改铺排 / 染色 / 圆角 / 换纹理时
    // 窗口顶点缓存会误判"内容未变"而继续用旧顶点（图片不刷新）。
    use std::hash::Hasher;
    fn sig(d: &UiDraw) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        cmd_sig_hash(&mut h, d);
        h.finish()
    }
    let mk = |bg: ImageBg| UiDraw {
        depth: 0,
        seq: 1,
        win: 0,
        elem: 0,
        rect: Rect::new(0.0, 0.0, 100.0, 50.0),
        clip: None,
        kind: DrawKind::Image(bg),
    };
    let base = ImageBg::new(7, Vec2::new(64.0, 64.0));
    assert_eq!(sig(&mk(base)), sig(&mk(base)), "同参数签名确定");
    assert_ne!(sig(&mk(base)), sig(&mk(base.tint(Color::RED))), "染色必须进签名");
    assert_ne!(
        sig(&mk(base)),
        sig(&mk(base.fit(crate::draw::ImageFit::Fill))),
        "铺排方式必须进签名"
    );
    assert_ne!(sig(&mk(base)), sig(&mk(base.radius(6.0))), "圆角遮罩必须进签名");
    assert_ne!(sig(&mk(base)), sig(&mk(ImageBg { tex: 8, ..base })), "纹理必须进签名");
    assert_ne!(
        sig(&mk(base)),
        sig(&mk(ImageBg { texel: Vec2::new(32.0, 64.0), ..base })),
        "纹素尺寸必须进签名（Center/Tile 的 1:1 基准）"
    );
}

#[test]
fn draw_kind_group_graphic_before_text() {
    // 同一 layer 内：图形（Solid/Border/Caret）分组 0，文字（Text）分组 1
    assert_eq!(DrawKind::Solid(Color::WHITE).group(), 0);
    assert_eq!(
        DrawKind::Border {
            color: Color::WHITE,
            width: 1.0,
            radius: Default::default(),
        }
        .group(),
        0
    );
    assert_eq!(DrawKind::Caret { color: Color::WHITE, width: 1.0 }.group(), 0);
    assert_eq!(
        DrawKind::Text {
            text: "x".into(),
            size: 14.0,
            color: Color::WHITE,
            align: TextAlign::Left,
            family: None,
            valign: TextVAlign::Center,
            clip: None,
            buf: None,
        }
        .group(),
        1
    );
    // finish 排序键顺序：win → depth → elem → group → seq
    let mut cmds = [UiDraw {
            depth: 0,
            seq: 2,
            win: 0,
            elem: 1,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            clip: None,                kind: DrawKind::Text {                    text: "t".into(),
                size: 14.0,
                color: Color::WHITE,
                align: TextAlign::Left,
                family: None,
                valign: TextVAlign::Center,
                clip: None,
                buf: None,
            },
        },
        UiDraw {
            depth: 0,
            seq: 1,
            win: 0,
            elem: 1,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            kind: DrawKind::Solid(Color::WHITE),
            clip: None,
        },
        UiDraw {
            depth: 0,
            seq: 3,
            win: 1,
            elem: 2,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            kind: DrawKind::Solid(Color::WHITE),
            clip: None,
        },
        UiDraw {
            depth: 0,
            seq: 4,
            win: 1,
            elem: 2,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            clip: None,                kind: DrawKind::Text {                    text: "w".into(),
                size: 14.0,
                color: Color::WHITE,
                align: TextAlign::Left,
                family: None,
                valign: TextVAlign::Center,
                clip: None,
                buf: None,
            },
        }];
    cmds.sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
    // 期望：win0 图形(elem1) → win0 文字(elem1) → win1 图形(elem2) → win1 文字(elem2)
    let order: Vec<u32> = cmds.iter().map(|d| d.seq).collect();
    assert_eq!(order, vec![1, 2, 3, 4], "窗口 z 升序；元素内图形先、文字后");
}

#[test]
fn bucket_cmds_equals_full_sort() {
    // P2 回归：depth 分桶提交序必须与全量稳定排序 (win, depth, elem, group, seq) 等价。
    // 模拟"录制序"：seq 单调递增，但 depth（容器嵌套进出）与 win（跨帧 z 缓存）
    // 乱序——正是免排序分桶要处理的场景。
    let mut rng = 42u64;
    let mut rand = move |m: u64| -> u64 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (rng >> 33) % m
    };
    let n = 300;
    let mut seq = 0u32;
    let cmds: Vec<UiDraw> = (0..n)
        .map(|_| {
            let depth = (rand(5)) as u32; // depth ∈ [0,5)
            let win = (rand(3)) as u32; // win ∈ {0,1,2}
            let graphic = rand(2) == 0;
            seq += 1;
            let kind = if graphic {
                DrawKind::Solid(Color::WHITE)
            } else {
                DrawKind::Text {
                    text: Arc::from("x"),
                    size: 14.0,
                    color: Color::WHITE,
                    align: TextAlign::Left,
                    valign: TextVAlign::Center,
                    family: None,
                    clip: None,
                    buf: None,
                }
            };
            UiDraw {
                depth,
                seq,
                win,
                elem: seq,
                rect: Rect::ZERO,
                clip: None,
                kind,
            }
        })
        .collect();
    let mut sorted = cmds.clone();
    sorted.sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
    let groups = bucket_cmds(cmds);
    let mut wins: Vec<u32> = groups.keys().copied().collect();
    wins.sort_unstable();
    let key = |d: &UiDraw| (d.win, d.depth, d.elem, d.kind.group(), d.seq);
    let mut got: Vec<(u32, u32, u32, u8, u32)> = Vec::with_capacity(n);
    for win in wins {
        let buckets = &groups[&win];
        for bucket in buckets {
            for d in bucket {
                got.push(key(d));
            }
        }
    }
    let want: Vec<(u32, u32, u32, u8, u32)> = sorted.iter().map(key).collect();
    assert_eq!(got, want, "depth 分桶提交序必须与全量排序完全等价");
}

#[test]
fn clamp_window_pos_screen_mode() {
    // 窗口 ≤ 屏幕：整体在屏幕内，贴边 clamp。
    let p = clamp_window_pos(Vec2::new(-50.0, 200.0), Vec2::new(100.0, 60.0), 800.0, 600.0);
    assert_eq!(p, Vec2::new(0.0, 200.0), "左/上超界贴 0");
    let p = clamp_window_pos(Vec2::new(900.0, 580.0), Vec2::new(100.0, 60.0), 800.0, 600.0);
    assert_eq!(p, Vec2::new(700.0, 540.0), "右/下超界贴 sw-size");
    // 窗口 ≤ 屏幕：区间内位置原样保留。
    let p = clamp_window_pos(Vec2::new(100.0, 50.0), Vec2::new(100.0, 60.0), 800.0, 600.0);
    assert_eq!(p, Vec2::new(100.0, 50.0));
    // 窗口 > 屏幕：左上角允许 ∈ [sw-size, 0]（仍覆盖屏幕、**可拖动**，不钉死 0）。
    let p = clamp_window_pos(Vec2::new(300.0, 200.0), Vec2::new(900.0, 700.0), 800.0, 600.0);
    assert_eq!(p, Vec2::new(0.0, 0.0), "超大窗口贴左上角（覆盖全屏）");
    let p = clamp_window_pos(Vec2::new(-300.0, -200.0), Vec2::new(900.0, 700.0), 800.0, 600.0);
    assert_eq!(p, Vec2::new(-100.0, -100.0), "超大窗口可拖到负区间（sw-size）");
    let p = clamp_window_pos(Vec2::new(-50.0, -50.0), Vec2::new(900.0, 700.0), 800.0, 600.0);
    assert_eq!(p, Vec2::new(-50.0, -50.0), "超大窗口在负区间内原样保留");
}

#[test]
fn window_drag_clamp_size_fixed_during_drag() {
    // 拖拽中窗口内容尺寸变化（变宽）时，clamp 边界**固定为按下帧尺寸**：
    // 贴右缘的窗口不被推回（纯跟手、无单帧跳变）；非拖拽帧才按最新尺寸复位。
    let sw = 800.0;
    let sh = 600.0;
    let press_size = Vec2::new(200.0, 100.0); // 按下帧窗口尺寸
    let now_size = Vec2::new(260.0, 100.0); // 拖拽中内容变宽后的尺寸
    // 同一目标位置（拖到屏幕右缘外）：
    let target = Vec2::new(720.0, 300.0);
    let with_press = clamp_window_pos(target, press_size, sw, sh);
    let with_now = clamp_window_pos(target, now_size, sw, sh);
    assert_eq!(with_press.x, 600.0, "按下帧尺寸 clamp：窗口贴右缘 600（拖拽中稳定）");
    assert_eq!(with_now.x, 540.0, "当前尺寸 clamp：窗口被推左 60px（拖拽中跳变源）");
    // 拖拽中窗口位置 = clamp(press_panel + d, press_size)：随鼠标位移连续。
    let press_panel = Vec2::new(600.0, 300.0);
    // let press_mouse = Vec2::new(700.0, 350.0);
    let mut last = press_panel.x;
    for i in 1..=6 {
        // 鼠标位移 -30, -20, -10, 0, +10, +20（先回拖离开右缘、再拖回贴边）。
        let d = Vec2::new(10.0 * (i as f32 - 4.0), 0.0);
        let disp = clamp_window_pos(press_panel + d, press_size, sw, sh);
        assert!(
            (disp.x - last).abs() <= 30.0 + 0.001,
            "帧 {i}: 拖拽中位置应随鼠标平滑（每帧 ≤ 鼠标步进 30px），实际 {last} → {}",
            disp.x
        );
        last = disp.x;
    }
}

#[test]
fn window_first_frame_position_converges() {
    // 首帧（`prev_size` 未知）：命中矩形 / 显示位置都用 `origin`（**不 clamp**）——
    // 屏幕上还没有这个窗口，"上一帧的矩形"无从谈起。次帧：`prev_size = size`
    // → 位置 = `clamp(origin, size)`，此后每帧一致（命中基准与显示同一个值）。
    // 之所以把 clamp 起点放在次帧，是为了让 `abs_base` 与几何**当帧一致**（见
    // `resolve_drag` 文档）；旧实现首帧"几何 clamp、`abs_base` 用 origin"，本身
    // 就是不一致的（首帧内容绝对空间错位）。
    let origin = Vec2::new(700.0, 300.0); // 窗口内容大，origin 超出真实 clamp 边界
    let size = Vec2::new(300.0, 100.0);
    let sw = 800.0;
    let sh = 600.0;
    // 次帧起：clamp(origin, size) 生效，且**再 clamp 不变**（收敛，无逐帧跳变）。
    let settled = clamp_window_pos(origin, size, sw, sh);
    assert_eq!(settled.x, 500.0, "次帧 clamp 到 [0, sw-size]=500");
    assert_eq!(
        clamp_window_pos(settled, size, sw, sh),
        settled,
        "已 clamp 位置再 clamp 不变（收敛、无跳变）"
    );
    // 首帧命中基准 = origin（未 clamp）：首帧按下不会因"基准/显示不一致"而跳变。
    assert_eq!(origin.x, 700.0);
}

#[test]
fn window_size_persists_across_z_change() {
    // 点击置顶 z+1 只影响 `window_rects[z]` 索引；尺寸按 **id** 持久
    // （`window_sizes`）→ 下帧 prev_size 不回落 ZERO，clamp 边界稳定。
    let id = "win_a";
    let mut sizes = std::collections::HashMap::<String, Vec2>::new();
    sizes.insert(id.into(), Vec2::new(200.0, 100.0));
    // 置顶后（window_rects 无新 z 记录）：
    let prev_size = sizes.get(id).copied(); // window_sizes 优先
    assert_eq!(prev_size, Some(Vec2::new(200.0, 100.0)), "置顶后尺寸仍可取（不回落 ZERO）");
    // 用持久尺寸 clamp：贴右缘窗口位置稳定（若回落 ZERO 会贴到 sw=800）。
    let p = clamp_window_pos(Vec2::new(650.0, 300.0), prev_size.unwrap(), 800.0, 600.0);
    assert_eq!(p.x, 600.0, "clamp 边界 = sw-size = 600（稳定）");
    let p_zero = clamp_window_pos(Vec2::new(650.0, 300.0), Vec2::ZERO, 800.0, 600.0);
    assert_eq!(p_zero.x, 650.0, "ZERO 尺寸 clamp 区间 [0,sw]（不生效，跳变源对比）");
}

#[test]
fn submit_order_graphics_before_text_per_window() {
    // 回归：窗口图形/文字绘制顺序抖动——同一窗口内图形组（白纹理 / 圆角 / 渐变）
    // 必须先于字形文字组，且跨帧稳定（不随 HashMap 迭代顺序 / 纹理 uid 分配变化）。
    let g = GROUP_GRAPHIC;
    let t = GROUP_TEXT;
    // 模拟 finish() 的提交列表：win=0 非窗口内容 + 窗口 1/2（各含图形组与文字组）。
    // 故意用乱序 + 反序 uid 输入（如 HashMap 迭代顺序）。
    let mut groups = vec![
        (2u32, t, 3u64),
        (1u32, t, 3u64),
        (0u32, t, 2u64),
        (2u32, g, 2u64),
        (0u32, g, 1u64),
        (1u32, g, 1u64),
        (1u32, g, 2u64),
    ];
    groups.sort_by_key(|&(w, gr, tex)| (w, gr, tex));
    // 期望：win 升序；同一窗口内图形组先于文字组；组内按纹理 uid 稳定。
    assert_eq!(
        groups,
        vec![
            (0, g, 1),
            (0, t, 2),
            (1, g, 1),
            (1, g, 2),
            (1, t, 3),
            (2, g, 2),
            (2, t, 3),
        ],
        "win 升序 + 窗口内图形先于文字（含程序化纹理），跨帧确定"
    );
}

#[test]
fn decoration_elem_must_follow_its_own_widget_background() {
    // 回归："图标不见了"——**组合控件里"后画的装饰"必须用比自己背景更大的元素序**。
    // 排序键是 `(win, depth, elem, group, seq)`：`elem` 是"录制到该元素时的序号"，
    // 所以元素序小的**先画**。`NumberInput` 的手柄底色 / 分隔线 / `≡` 图标曾写死
    // `elem = 0/1`，而文本框用 `elem = seq + 1`（大得多）⇒ 整个手柄被文本框盖住，
    // 看上去就是一个普通输入框；`ColorPicker` 的展开箭头（`push_draw` 默认 `elem = 0`）
    // 同理被色块盖住。`Ui::elem_hint()` 就是为这条约定提供的（= 当前 `seq + 1`）。
    //
    // 用"文本框 + 手柄 + 图标"三个命令的排序模拟一次提交顺序。
    let elem_field = 40u32; // 文本框：录制时的 seq + 1
    let mut cmds = [
        (0u32, 40u32), // 手柄底色：elem 0（错误写法）
        (1, 41),       // 分隔线：elem 1（错误写法）
        (0, 42),       // 图标：push_draw 默认 elem 0
        (elem_field, 10), // 文本框背景
    ];
    cmds.sort_by_key(|&(elem, seq)| (elem, seq));
    assert_eq!(
        cmds.last().copied(),
        Some((elem_field, 10)),
        "写死小元素序的装饰会被文本框盖住（历史 bug 的成因）"
    );
    // 正确写法：装饰取 `elem_hint()`（录制到装饰时 seq 已增大）⇒ 排在文本框之后。
    let mut fixed = [
        (elem_field, 10), // 文本框背景
        (41, 41),         // 手柄底色：elem_hint()
        (41, 42),         // 分隔线：同元素内靠 seq 决定
        (42, 43),         // 图标：elem_hint()
    ];
    fixed.sort_by_key(|&(elem, seq)| (elem, seq));
    let last_two = &fixed[fixed.len() - 2..];
    assert_eq!(
        last_two.iter().map(|&(e, _)| e).collect::<Vec<_>>(),
        vec![41, 42],
        "手柄（41）与图标（42）都必须排在文本框（40）之后"
    );
}

#[test]
fn drag_needs_movement_so_clicks_work() {
    // 回归：窗口/可拖拽面板内 CheckBox / 输入框失效——按下即激活拖拽导致
    // `drag_panel` 在释放帧抑制子控件，点击被吞。修复：位移 ≥ DRAG_ACTIVATE_PX
    // 才视为拖拽；纯点击（无位移 / 微小抖动）不激活。
    let press = Vec2::new(100.0, 200.0); // 按下基准（物理像素，已取整）
    // 静止（纯点击）：未激活
    assert!(!drag_moved(press, Some(press)), "按下无位移不激活拖拽");
    // 微小抖动（< 阈值）：仍视为点击
    assert!(!drag_moved(press + Vec2::new(1.0, 1.0), Some(press)), "1px 抖动不激活");
    assert!(!drag_moved(press + Vec2::new(2.0, 2.0), Some(press)), "~2.8px 位移不激活");
    // 恰好达到阈值
    assert!(
        drag_moved(press + Vec2::new(3.0, 0.0), Some(press)),
        "3px 水平位移激活拖拽"
    );
    // 明显位移：激活
    assert!(
        drag_moved(press + Vec2::new(0.0, -8.0), Some(press)),
        "8px 竖直位移激活拖拽"
    );
    // 无按下基准（如窗口未命中时）：不激活
    assert!(!drag_moved(press, None), "无按下基准不激活");
}

/// 主键状态（`pressed` / 本帧边沿）——构造 `resolve_drag` 的输入。
fn mouse_btn(pressed: bool, edge: bool) -> rjw_keystate::KeyState {
    use rjw_keystate::{
        KEY_STATE_DOWN_EDGE, KEY_STATE_PRESSING, KEY_STATE_RELEASED, KEY_STATE_UP_EDGE,
    };
    match (pressed, edge) {
        (true, true) => KEY_STATE_DOWN_EDGE,
        (true, false) => KEY_STATE_PRESSING,
        (false, true) => KEY_STATE_UP_EDGE,
        (false, false) => KEY_STATE_RELEASED,
    }
}

#[test]
fn window_or_panel_drag_position_has_no_frame_lag() {
    // 回归：**拖动窗口 / 可拖拽面板时，`abs_base` 落后几何一帧**——`abs_base` 取上一帧
    // 位置、几何按本帧 `display_pos` 平移，于是拖拽期间一切走 `abs_base` 的绝对空间量
    // （文本 `box_clip` / IME 光标 / 滑块基准）都错一帧的位移量：位移越大越明显
    // （"高速拖动过程瞬间偏移"）。
    //
    // 修复的**结构前提**（本测试锁定它）：位置只由**按下帧基准 + 当前鼠标位移**决定，
    // 与上一帧位置无关 —— 因此 `display_pos` 能在内容录制前求出，`abs_base` 与几何
    // 用同一个值。
    let origin = Vec2::new(100.0, 40.0);
    let mut ws = WidgetState::default();
    // ① 按下帧（down_edge + 命中）：建立基准，纯点击不激活。
    let (active, pos) = resolve_drag(&mut ws, true, mouse_btn(true, true), Vec2::new(150.0, 90.0), origin);
    assert!(!active, "按下帧不激活");
    assert_eq!(pos, origin, "按下帧位置 = origin");
    assert_eq!(ws.press_mouse, Some(Vec2::new(150.0, 90.0)), "基准 = 按下帧鼠标");
    // ② 拖拽中（每帧位移 40px：模拟"高速拖动"）：位置 = 基准 + 当前位移，
    //    与上一帧位置无关 —— 若实现改成"上一帧位置 + 帧增量"，这里就会累积误差。
    for step in 1..=5 {
        let m = Vec2::new(150.0 + 40.0 * step as f32, 90.0);
        let (active, pos) = resolve_drag(&mut ws, true, mouse_btn(true, false), m, origin);
        assert!(active, "帧 {step}: 位移 ≥ 阈值应激活");
        assert_eq!(
            pos,
            origin + Vec2::new(40.0 * step as f32, 0.0),
            "帧 {step}: 位置 = 按下基准 + 鼠标总位移（无逐帧累积误差）"
        );
    }
    // ③ 释放：拖拽结束（基准保留但不再激活）。
    let (active, pos) = resolve_drag(&mut ws, true, mouse_btn(false, true), Vec2::new(350.0, 90.0), origin);
    assert!(!active, "释放后不激活");
    assert_eq!(pos, origin);
}

#[test]
fn child_press_claim_clears_drag_base() {
    // 回归：窗口 / 面板内输入框按下（选择文本）或滑块按下（调值）时，外层**不得**
    // 跟随拖动。子控件在录制期调用 `claim_press`，外层在本帧按下帧清除基准。
    let origin = Vec2::new(100.0, 40.0);
    let mut ws = WidgetState::default();
    // 按下（命中窗口空白处）：基准先无条件建立……
    let _ = resolve_drag(&mut ws, true, mouse_btn(true, true), Vec2::new(150.0, 90.0), origin);
    assert!(ws.press_mouse.is_some(), "按下帧先建立基准（判定在内容录制后）");
    // ……随后子控件声明本次按下 → 清除基准。
    clear_drag_base(&mut ws);
    assert!(ws.press_mouse.is_none() && ws.press_panel.is_none() && ws.press_size.is_none());
    // 之后鼠标大幅移动（按钮仍按住）也不得拖动窗口（"从输入框上拖拽 = 选择文本"）。
    for step in 1..=5 {
        let m = Vec2::new(150.0 + 50.0 * step as f32, 90.0);
        let (active, pos) = resolve_drag(&mut ws, true, mouse_btn(true, false), m, origin);
        assert!(!active, "帧 {step}: 无基准不得激活拖拽");
        assert_eq!(pos, origin, "帧 {step}: 窗口不动");
    }
}

#[test]
fn drag_outside_press_does_not_establish_base() {
    // 体外按下（`hit = false`，如点在窗口外的桌面区域 / 被更高 z 窗口遮挡）：
    // 不建立基准 → 拖入窗口内也不会把窗口拖走。
    let origin = Vec2::new(100.0, 40.0);
    let mut ws = WidgetState::default();
    let (active, _) = resolve_drag(&mut ws, false, mouse_btn(true, true), Vec2::new(10.0, 10.0), origin);
    assert!(!active);
    assert!(ws.press_mouse.is_none(), "体外按下不建立基准");
    let (active, pos) = resolve_drag(&mut ws, true, mouse_btn(true, false), Vec2::new(120.0, 60.0), origin);
    assert!(!active, "无基准不激活");
    assert_eq!(pos, origin);
}

#[test]
fn text_block_placement_is_integer_with_line_box_top() {
    // 回归：UI 文本亚像素模糊——垂直行盒对齐偏移经**整数运算**后，
    // `block_tl = anchor + off` 的两侧操作数均为整数（浮点整数不变量）：
    // 小数（行盒顶 / 奇数宽的一半）只在 `round` 边界被一次性消化，
    // 不流入加法链 → 无误差累加、字形四边形角点恒为整数屏幕像素。
    // 数据来自 14px 排版实测：行高 16.8→content_h 17，"a" 行盒顶 = -7.2
    // （rjw_text 收集期取整为 -7.0）。
    let anchor = Vec2::new(100.0, 200.0);
    let content = Vec2::new(14.0, 17.0);
    let first_line_top = -7.0;
    // 左对齐标签：block = anchor + off，两项均为整数
    let block = anchor + text_block_offset(TextAlign::Left, TextVAlign::Center, content, first_line_top);
    assert_eq!(block.x.fract(), 0.0, "标签块 x 必须为整数像素，实际 {}", block.x);
    assert_eq!(block.y.fract(), 0.0, "标签块 y 必须为整数像素，实际 {}", block.y);
    // 行盒中心对准锚点（±0.5px 量化，奇数 content_h 的固有半像素）：
    // block.y + first_line_top + content_h/2 ≈ anchor.y
    let center = block.y + first_line_top + content.y * 0.5;
    assert!(
        (center - anchor.y).abs() <= 0.5,
        "行盒中心应贴近锚点，偏差 {}",
        center - anchor.y
    );
    // 居中按钮 + 奇数物理宽（21px，如 DPI 1.5 下）：水平对齐亦为整数
    let btn = anchor + text_block_offset(TextAlign::Center, TextVAlign::Center, Vec2::new(21.0, 17.0), first_line_top);
    assert_eq!(btn.x.fract(), 0.0, "按钮块 x 必须为整数像素，实际 {}", btn.x);
    assert_eq!(btn.y.fract(), 0.0, "按钮块 y 必须为整数像素，实际 {}", btn.y);
}

#[test]
fn overlapping_elements_follow_record_order() {
    // 元素重叠层级正确性：后录元素（elem 大）覆盖先录元素（elem 小），
    // **即使后录元素是图形（group 0）、先录元素是文字（group 1）**——
    // 元素序优先于图形/文字分组。
    let mut cmds = [
        // 元素 A（先录）：文字
        UiDraw {
            depth: 1,
            seq: 1,
            win: 1,
            elem: 1,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            kind: DrawKind::Text {
                text: "a".into(),
                size: 14.0,
                color: Color::WHITE,
                align: TextAlign::Left,
                family: None,
                valign: TextVAlign::Center,
                clip: None,
                buf: None,
            },
            clip: None,
        },
        // 元素 B（后录）：图形——应覆盖 A 的文字
        UiDraw {
            depth: 1,
            seq: 2,
            win: 1,
            elem: 2,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            kind: DrawKind::Solid(Color::WHITE),
            clip: None,
        },
    ];
    cmds.sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
    let order: Vec<u32> = cmds.iter().map(|d| d.seq).collect();
    assert_eq!(order, vec![1, 2], "后录元素（B 图形）应覆盖先录元素（A 文字）");
    // 元素内：同一 elem 的图形先于文字
    let mut inner = [
        UiDraw {
            depth: 1,
            seq: 3,
            win: 1,
            elem: 3,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            kind: DrawKind::Text {
                text: "x".into(),
                size: 14.0,
                color: Color::WHITE,
                align: TextAlign::Left,
                family: None,
                valign: TextVAlign::Center,
                clip: None,
                buf: None,
            },
            clip: None,
        },
        UiDraw {
            depth: 1,
            seq: 2,
            win: 1,
            elem: 3,
            rect: Rect::new(0.0, 0.0, 1.0, 1.0),
            kind: DrawKind::Solid(Color::WHITE),
            clip: None,
        },
    ];
    inner.sort_by_key(|d| (d.win, d.depth, d.elem, d.kind.group(), d.seq));
    let order: Vec<u32> = inner.iter().map(|d| d.seq).collect();
    assert_eq!(order, vec![2, 3], "元素内：图形(seq2)先画，文字(seq3)后画（文字覆盖图形）");
}

#[test]
fn window_transform_tracks_origin() {
    // 窗口四边形的**局部顶点**经 `screen_fixed_tf(origin)` 提交 ⇒ 窗口移动 =
    // 变换平移量同步变化（拖动"视觉上要跟着走"的数学保证）。
    let a = screen_fixed_tf(Vec2::new(100.0, 50.0));
    let b = screen_fixed_tf(Vec2::new(140.0, 70.0));
    let local = Vec2::new(7.0, 9.0);
    assert_eq!(
        b.transform_point(local) - a.transform_point(local),
        Vec2::new(40.0, 20.0),
        "origin 变化 ⇒ 窗口局部点整体平移同样的量"
    );
    // UI 层坐标空间 = 物理像素、左上原点（identity 相机 + `center` 平移）：
    // 局部点 (0,0) 经变换即落在屏幕上 origin 处。
    assert_eq!(a.transform_point(Vec2::ZERO), Vec2::new(100.0, 50.0));
}

#[test]
fn text_focus_only_for_text_widgets() {
    let mut st = UiState::new();
    assert!(st.text_focus().is_none(), "无焦点");
    // 按钮持焦点：**不是**文本焦点（不应吞掉应用快捷键）。
    st.focused = Some(IdAbsolute::from("btn"));
    st.focused_kind = Some(FocusKind::Button);
    assert!(st.text_focus().is_none(), "按钮焦点不算文本焦点");
    // 文本框持焦点：是文本焦点，且带出控件 id。
    st.focused = Some(IdAbsolute::from("name"));
    st.focused_kind = Some(FocusKind::TextInput);
    assert_eq!(st.text_focus().map(|f| f.id.as_str().to_owned()), Some("name".to_owned()));
    st.focused = None;
    st.focused_kind = None;
    assert!(st.text_focus().is_none());
}

#[test]
fn anchor_pos_covers_all_corners_and_clamps() {
    let vp = Vec2::new(1280.0, 720.0);
    let size = Vec2::new(200.0, 20.0);
    let m = Vec2::new(16.0, 16.0);
    // 四角 + 边距
    assert_eq!(Ui::anchor_pos_in(vp, Anchor::TopLeft, size, m), Vec2::new(16.0, 16.0));
    assert_eq!(
        Ui::anchor_pos_in(vp, Anchor::TopRight, size, m),
        Vec2::new(1280.0 - 16.0 - 200.0, 16.0)
    );
    assert_eq!(
        Ui::anchor_pos_in(vp, Anchor::BottomLeft, size, m),
        Vec2::new(16.0, 720.0 - 16.0 - 20.0)
    );
    assert_eq!(
        Ui::anchor_pos_in(vp, Anchor::BottomRight, size, m),
        Vec2::new(1280.0 - 16.0 - 200.0, 720.0 - 16.0 - 20.0)
    );
    // 居中（上下 / 左右边距对称）
    assert_eq!(
        Ui::anchor_pos_in(vp, Anchor::Center, size, m),
        Vec2::new(16.0 + (1280.0 - 32.0 - 200.0) * 0.5, 16.0 + (720.0 - 32.0 - 20.0) * 0.5)
    );
    // 底中对齐
    assert_eq!(
        Ui::anchor_pos_in(vp, Anchor::BottomCenter, size, m),
        Vec2::new(16.0 + (1280.0 - 32.0 - 200.0) * 0.5, 720.0 - 16.0 - 20.0)
    );
    // 内容超视口（+边距）→ clamp 贴边（左上/左下角，不产生负坐标；
    // 内容本身比视口宽时无法完全放入，贴边即可）
    let big = Vec2::new(2000.0, 1000.0);
    let p = Ui::anchor_pos_in(vp, Anchor::BottomRight, big, m);
    assert!(p.x >= 0.0 && p.y >= 0.0, "不产生负坐标");
    assert_eq!(Ui::anchor_pos_in(vp, Anchor::TopLeft, big, m), Vec2::new(16.0, 16.0), "左上角不 clamp");
}

#[test]
fn container_builder_options_defaults_and_overrides() {
    // 窗口责任链选项默认值 = 旧 `window_at` 语义（自动宽 / 置顶 / Expand 不裁剪 /
    // 全局主题）。
    let o = WindowOptions::default();
    assert_eq!(o.pos, Position::Logical(Vec2::ZERO));
    assert_eq!(o.width, None);
    assert_eq!(o.level, Level::Topmost, "默认点击置顶");
    assert_eq!(o.placement, Placement::Expand, "默认 Expand 语义（不裁剪）");
    assert!(o.style.is_none(), "默认跟随全局 Theme::panel");
    // 覆盖组合 = 固定宽 + Level::Normal + Placement::Clip + 逐窗口样式。
    let o2 = WindowOptions {
        pos: Position::Logical(Vec2::new(10.0, 20.0)),
        width: Some(Size::Logical(300.0)),
        level: Level::Normal,
        placement: Placement::Clip,
        style: Some(PanelStyle::default().with_radius(8.0)),
        ..WindowOptions::default()
    };
    assert_eq!(o2.pos, Position::Logical(Vec2::new(10.0, 20.0)));
    assert_eq!(o2.width, Some(Size::Logical(300.0)));
    assert_eq!(o2.level, Level::Normal);
    assert_eq!(o2.placement, Placement::Clip);
    assert_eq!(o2.style.unwrap().radius, 8.0);

    // 面板责任链选项默认值 = `panel_at` 语义。
    let p = PanelOptions::default();
    assert_eq!(p.pos, Position::Logical(Vec2::ZERO));
    assert_eq!(p.drag, None);
    assert!(p.style.is_none());
    // 拖拽面板 = `drag_panel_at` 语义。
    let p2 = PanelOptions {
        pos: Position::Logical(Vec2::new(4.0, 5.0)),
        drag: Some("inspector".to_owned()),
        style: None,
    };
    assert_eq!(p2.drag.as_deref(), Some("inspector"));
}

#[test]
fn topmost_win_never_occluded() {
    use crate::hit::window_occluded;
    // 任意普通窗口叠放时，置顶哨兵 z（浮层）恒不被遮挡 → 浮层始终可交互。
    let rects = [
        (1u32, Rect::new(0.0, 0.0, 100.0, 100.0)),
        (5u32, Rect::new(0.0, 0.0, 100.0, 100.0)),
    ];
    assert!(
        !window_occluded(WIN_TOPMOST, Vec2::new(10.0, 10.0), rects.iter().copied()),
        "置顶哨兵 z 恒不被遮挡（IME 候选框 / 下拉浮层可交互）"
    );
    // 对照：普通窗口仍被更高 z 遮挡
    assert!(window_occluded(1, Vec2::new(10.0, 10.0), rects.iter().copied()));
}

#[test]
fn pos_chain_resolves_by_priority() {
    use std::collections::HashMap;
    // 责任链解析顺序：脚本（优先级降序）→ 内置拖拽状态（优先级 0）→ 调用者 pos 兜底
    let mut chain: Vec<(i32, PosLink)> = vec![(0, PosLink::Drag)];
    chain.push((10, PosLink::Script(Box::new(|_| Some(Vec2::new(1.0, 1.0))))));
    chain.push((-10, PosLink::Script(Box::new(|_| Some(Vec2::new(2.0, 2.0))))));
    chain.sort_by_key(|e| std::cmp::Reverse(e.0)); // 高优先级在前（pos_handler 内部同款排序）
    let mut panel_pos = HashMap::new();
    panel_pos.insert(IdAbsolute::from("w"), Vec2::new(3.0, 3.0));
    // 高优先级脚本胜出
    assert_eq!(
        resolve_pos_link(&chain, &panel_pos, &IdAbsolute::from("w"), Vec2::ZERO),
        Vec2::new(1.0, 1.0)
    );
    // 去掉高优先级脚本 → 内置拖拽状态（优先级 0 > -10）先被询问 → 用户拖拽优先
    chain.retain(|(p, _)| *p != 10);
    chain.sort_by_key(|e| std::cmp::Reverse(e.0));
    assert_eq!(
        resolve_pos_link(&chain, &panel_pos, &IdAbsolute::from("w"), Vec2::ZERO),
        Vec2::new(3.0, 3.0),
        "拖拽状态优先级 0 高于负优先级脚本 → 用户拖过就赢过动画"
    );
    // 负优先级脚本在用户**未拖过**时兜底提供位置（动画"填空"语义）
    assert_eq!(
        resolve_pos_link(&chain, &panel_pos, &IdAbsolute::from("not_dragged"), Vec2::ZERO),
        Vec2::new(2.0, 2.0)
    );
    // 全部脚本落空 → 内置用户拖拽状态胜出
    chain.retain(|(p, _)| *p != -10);
    chain.sort_by_key(|e| std::cmp::Reverse(e.0));
    assert_eq!(
        resolve_pos_link(&chain, &panel_pos, &IdAbsolute::from("w"), Vec2::ZERO),
        Vec2::new(3.0, 3.0)
    );
    // 用户未拖过 → 调用者传入 pos 兜底
    assert_eq!(
        resolve_pos_link(&chain, &panel_pos, &IdAbsolute::from("other"), Vec2::new(9.0, 9.0)),
        Vec2::new(9.0, 9.0)
    );
    // 脚本按 id 选择性响应：返回 None 即交还下一环
    let mut chain2: Vec<(i32, PosLink)> = vec![(0, PosLink::Drag)];
    chain2.push((5, PosLink::Script(Box::new(|id| {
        if id == "scripted" {
            Some(Vec2::new(7.0, 7.0))
        } else {
            None
        }
    }))));
    chain2.sort_by_key(|e| std::cmp::Reverse(e.0));
    assert_eq!(
        resolve_pos_link(&chain2, &panel_pos, &IdAbsolute::from("scripted"), Vec2::ZERO),
        Vec2::new(7.0, 7.0)
    );
    assert_eq!(
        resolve_pos_link(&chain2, &panel_pos, &IdAbsolute::from("w"), Vec2::ZERO),
        Vec2::new(3.0, 3.0),
        "脚本对 id 返回 None → 落到用户拖拽状态"
    );
}

#[test]
fn safe_line_slice_never_panics_on_stale_byte_ranges() {
    // 回归：TextArea 粘贴中文后，旧视觉行字节区间落在新文本多字节字符中间
    // （"start byte index 64 is not a char boundary; inside '尾'" panic）。
    // safe_line_slice 必须对齐到字符边界且不 panic。
    // "窗口 A 选项\n尾部文字"：'项' = bytes 12..15（3 字节/字符）
    let value = "窗口 A 选项\n尾部文字";
    // 模拟过期 vlines：区间起点/终点落在 '项'（12..15）中间
    let stale = VisualLine { byte_start: 13, byte_end: 14, top: 0.0, width: 10.0 };
    let s = safe_line_slice(value, &stale);
    // 13 → floor 到 '项' 起点 12；14 → floor 到 12 → s == e → 空串（不 panic）
    assert_eq!(s, "");
    // 区间越过末尾：byte_end 超 len → clamp 到 len 并取整
    let over = VisualLine { byte_start: 0, byte_end: 999, top: 0.0, width: 10.0 };
    assert_eq!(safe_line_slice(value, &over), value);
    // 正常区间原样返回（0..6 = "窗口"）
    let ok = VisualLine { byte_start: 0, byte_end: 6, top: 0.0, width: 10.0 };
    assert_eq!(safe_line_slice(value, &ok), "窗口");
}

#[test]
fn image_bg_layout_covers_all_four_fits() {
    // 铺排是**纯数学**（不碰 GPU）——四种方式的"画在哪、取哪块 UV"全部可断言。
    let rect = Rect::new(10.0, 20.0, 200.0, 100.0);
    let texel = Vec2::new(64.0, 64.0);
    // ① 拉伸：恒等于目标矩形，UV 取满。
    let l = ImageBg::new(1, texel).layout(rect).expect("拉伸");
    assert_eq!(l.rect, rect);
    assert_eq!((l.uv0, l.uv1), (Vec2::ZERO, Vec2::ONE));
    assert!(l.tile.is_none());
    // ② 等比覆盖：`scale = max(200/64, 100/64) = 3.125` ⇒ 图 200×200、可见中段。
    //    覆盖的那一轴（宽）正好铺满 ⇒ u 取满；另一轴取中间一半 ⇒ v 取 [0.25, 0.75]。
    //    ⚠ 等比是关键：Fill 不会把 64×64 压成 200×100（那是 Stretch 的行为）。
    let l = ImageBg::new(1, texel).fit(crate::draw::ImageFit::Fill).layout(rect).expect("覆盖");
    assert_eq!(l.rect, rect, "覆盖恒铺满目标矩形（裁剪靠 UV）");
    assert_eq!(l.uv0, Vec2::new(0.0, 0.25));
    assert_eq!(l.uv1, Vec2::new(1.0, 0.75));
    // ③ 原始尺寸居中：64×64 居中放在 200×100 里，UV 取满。
    let l = ImageBg::new(1, texel).fit(crate::draw::ImageFit::Center).layout(rect).expect("居中");
    assert_eq!(l.rect, Rect::new(78.0, 38.0, 64.0, 64.0));
    assert_eq!((l.uv0, l.uv1), (Vec2::ZERO, Vec2::ONE));
    // ③b 放不下时居中裁剪：可用宽 30 < 64 ⇒ 只取中央 30/64 的 UV。
    let narrow = Rect::new(0.0, 0.0, 30.0, 100.0);
    let l = ImageBg::new(1, texel).fit(crate::draw::ImageFit::Center).layout(narrow).expect("裁剪");
    assert_eq!(l.rect, Rect::new(0.0, 18.0, 30.0, 64.0), "可见区居中且不超过可用区");
    let half = (1.0 - 30.0 / 64.0) * 0.5;
    assert!((l.uv0.x - half).abs() < 1e-6 && (l.uv1.x - (1.0 - half)).abs() < 1e-6);
    assert_eq!((l.uv0.y, l.uv1.y), (0.0, 1.0), "够用的轴不裁");
    // ④ 平铺：`tile` 有值（= 纹素尺寸，1:1），几何由 `tile_grid` 展开。
    let l = ImageBg::new(1, texel).fit(crate::draw::ImageFit::Tile).layout(rect).expect("平铺");
    assert_eq!(l.tile, Some(texel));
    // 退化输入：空尺寸 / 零纹素 ⇒ 无可绘制。
    assert!(ImageBg::new(1, texel).layout(Rect::new(0.0, 0.0, 0.0, 10.0)).is_none());
    assert!(ImageBg::new(1, Vec2::ZERO).layout(rect).is_none());
}

#[test]
fn tile_grid_covers_area_with_truncated_partial_blocks() {
    // 平铺网格：块数 = ceil(w/tile) × ceil(h/tile)；边缘**部分块**按比例截断 UV
    // （否则整块图会被压缩进残块，图案变形）。
    let rect = Rect::new(5.0, 7.0, 100.0, 70.0);
    let tile = Vec2::new(32.0, 32.0);
    let cells: Vec<_> = crate::draw::tile_grid(rect, tile).expect("网格").collect();
    // 4 列 × 3 行 = 12 块
    assert_eq!(cells.len(), 12);
    // 首块：完整 32×32，UV 取满。
    assert_eq!(cells[0], (Rect::new(5.0, 7.0, 32.0, 32.0), Vec2::ONE));
    // 末列（第 4 列）：宽 = 100 - 3×32 = 4 ⇒ u 只取 4/32。
    let last_col_first = cells[3];
    assert_eq!(last_col_first.0, Rect::new(5.0 + 96.0, 7.0, 4.0, 32.0));
    assert!((last_col_first.1.x - 4.0 / 32.0).abs() < 1e-6);
    assert_eq!(last_col_first.1.y, 1.0);
    // 末行 / 末列交叉块：宽 4、高 70 - 2×32 = 6 ⇒ 两个方向都截断。
    let corner = cells[11];
    assert_eq!(corner.0, Rect::new(5.0 + 96.0, 7.0 + 64.0, 4.0, 6.0));
    assert!((corner.1.x - 4.0 / 32.0).abs() < 1e-6 && (corner.1.y - 6.0 / 32.0).abs() < 1e-6);
    // 恰好整除时不留部分块（3 列 × 2 行 = 6 块，全部取满）。
    let exact: Vec<_> =
        crate::draw::tile_grid(Rect::new(0.0, 0.0, 96.0, 64.0), tile).expect("整除").collect();
    assert_eq!(exact.len(), 6);
    assert!(exact.iter().all(|(_, uv)| *uv == Vec2::ONE), "整除时全是完整块");
    // 块数上限：1px 图块铺满大面板 ⇒ 退化（`None`），由调用方回退到拉伸。
    assert!(
        crate::draw::tile_grid(Rect::new(0.0, 0.0, 4000.0, 4000.0), Vec2::ONE).is_none(),
        "超上限必须返回 None（避免上万顶点）"
    );
    // 退化输入
    assert!(crate::draw::tile_grid(Rect::new(0.0, 0.0, 10.0, 10.0), Vec2::ZERO).is_none());
    assert!(crate::draw::tile_grid(Rect::new(0.0, 0.0, 0.0, 10.0), Vec2::ONE).is_none());
}

#[test]
fn geometry_cache_signature_tracks_atlas_revision() {
    // 回归：**字形图集重排 / 复用已逐出槽位后，窗口顶点缓存不失效** ⇒ 缓存的 UV
    // 采样到别的字形像素（"陈旧文字 200 帧后出现" / "背景消失"）。命令内容没变，
    // 只靠命令哈希永远命中原顶点，故缓存键必须并入图集区域失效世代号。
    let cmds = 0xABCD_1234_u64;
    // 同一帧内容 + 同一世代价：签名必须一致（否则缓存永不命中，每帧全量重建）。
    assert_eq!(
        geom_cache_sig(cmds, 7),
        geom_cache_sig(cmds, 7),
        "同内容同世代 ⇒ 同签名（缓存可命中）"
    );
    // 内容不变、只图集世代推进：签名必须变（否则缓存永不失效 → 陈旧 UV）。
    assert_ne!(
        geom_cache_sig(cmds, 7),
        geom_cache_sig(cmds, 8),
        "图集世代推进 ⇒ 签名必须变化（强制重建、重新解析字形区域）"
    );
    // 不同内容仍必须区分（世代号不能吞掉命令哈希的区分度）。
    assert_ne!(
        geom_cache_sig(cmds, 7),
        geom_cache_sig(cmds ^ 1, 7),
        "世代号不得掩盖内容差异"
    );
    // 世代号在低位/高位都参与：单调推进的整数不能因哈希碰撞而互相抵消。
    let revs: std::collections::HashSet<u64> =
        (0..64).map(|r| geom_cache_sig(cmds, r)).collect();
    assert_eq!(revs.len(), 64, "64 个连续世代应给出 64 个不同签名");
}

// ─── 滚动条几何（`scroll_thumb` / `scroll_offset_for_thumb`） ─────

#[test]
fn scrollbar_is_centred_capsule_with_side_air() {
    let view = Rect::new(100.0, 50.0, 200.0, 300.0);
    let (strip, track) = scrollbar_rects(&view, 300.0);
    // 条带 = 右缘全高（占位 / 命中 / 翻页热区）。
    assert_eq!(
        strip,
        Rect::new(100.0 + 200.0 - SCROLLBAR_W, 50.0, SCROLLBAR_W, 300.0)
    );
    // 可见滑块**居中**于条带 ⇒ 两侧各留白（用户："两侧留白"）——不得贴边。
    assert_eq!(track.w, SCROLLBAR_BAR_W);
    assert_eq!(track.x, strip.x + (SCROLLBAR_W - SCROLLBAR_BAR_W) * 0.5);
    assert!(track.x > strip.x && track.x + track.w < strip.x + strip.w);
    assert_eq!(track.x - strip.x, SCROLLBAR_W - (track.x + track.w - strip.x));
    // 上下留白（胶囊两端不贴可视区边缘）。
    assert_eq!(track.y, strip.y + SCROLLBAR_MARGIN);
    assert_eq!(track.y + track.h, strip.y + strip.h - SCROLLBAR_MARGIN);
    // 比旧版（8）**更粗**。
    assert!(track.w > 8.0);
    // 极矮可视区不 panic、不溢出（轨道高至少 1px）。
    let (_, tiny) = scrollbar_rects(&Rect::new(0.0, 0.0, 10.0, 4.0), 4.0);
    assert_eq!(tiny.h, 1.0);
}

#[test]
fn scroll_thumb_fills_track_when_content_fits() {
    let (thumb_h, travel, y) = scroll_thumb(200.0, 200.0, 200.0, 0.0, 0.0);
    assert_eq!((thumb_h, travel, y), (200.0, 0.0, 0.0), "正好装下 → 满轨");
    // 内容比可视区矮（ratio clamp 到 1）⇒ 仍满轨、无行程。
    let (thumb_h, travel, y) = scroll_thumb(200.0, 200.0, 40.0, 0.0, 0.0);
    assert_eq!((thumb_h, travel, y), (200.0, 0.0, 0.0));
    // 空内容（content_h = 0）不得除零、不得出现 NaN。
    let (thumb_h, travel, y) = scroll_thumb(120.0, 100.0, 0.0, 0.0, 0.0);
    assert_eq!((thumb_h, travel, y), (120.0, 0.0, 0.0));
}

#[test]
fn scroll_thumb_is_proportional_and_bottom_aligned() {
    // 可视 100 / 内容 400 ⇒ 滑块 = 轨道 1/4。
    let (thumb_h, travel, top) = scroll_thumb(400.0, 100.0, 400.0, 0.0, 300.0);
    assert_eq!((thumb_h, travel, top), (100.0, 300.0, 0.0));
    // 中途：正比。
    let (_, travel, top) = scroll_thumb(400.0, 100.0, 400.0, 150.0, 300.0);
    assert_eq!((travel, top), (300.0, 150.0));
    // **不变量**：滚到底 ⇒ 滑块底端与轨道底端对齐（视觉上真的贴底）。
    let (thumb_h, travel, top) = scroll_thumb(400.0, 100.0, 400.0, 300.0, 300.0);
    assert_eq!(top, travel);
    assert_eq!(top + thumb_h, 400.0);
    // 越界偏移被夹住（不得把滑块推出轨道）。
    let (_, travel, top) = scroll_thumb(400.0, 100.0, 400.0, 9999.0, 300.0);
    assert_eq!(top, travel);
}

#[test]
fn scroll_thumb_respects_minimum_and_roundtrips() {
    // 内容极高 ⇒ 滑块被最小值兜住（不会细成一条线）。
    let (thumb_h, travel, _) = scroll_thumb(300.0, 100.0, 100_000.0, 0.0, 99_900.0);
    assert_eq!(thumb_h, SCROLLBAR_MIN_THUMB);
    assert_eq!(travel, 300.0 - SCROLLBAR_MIN_THUMB);
    // 轨道比最小值还矮 ⇒ 滑块 = 轨道，不溢出。
    let (thumb_h, travel, _) = scroll_thumb(10.0, 100.0, 100_000.0, 0.0, 1.0);
    assert_eq!((thumb_h, travel), (10.0, 0.0));
    // 拖拽逆映射与正映射互逆（整像素取整 ⇒ 往返误差 ≤ 1px）。
    for off in [0.0, 1.0, 123.0, 499.0, 899.0, 900.0] {
        let (_, travel, top) = scroll_thumb(400.0, 100.0, 400.0, off, 900.0);
        let back = scroll_offset_for_thumb(top, travel, 900.0);
        assert!(
            (back - off).abs() <= 1.0,
            "off={off} → top={top} → back={back}"
        );
    }
    // 无行程（内容装得下）时拖拽不产生偏移。
    assert_eq!(scroll_offset_for_thumb(50.0, 0.0, 900.0), 0.0);
}
