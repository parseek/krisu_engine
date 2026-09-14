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
fn draw_kind_group_graphic_before_text() {
    // 同一 layer 内：图形（Solid/Border/Caret）分组 0，文字（Text）分组 1
    assert_eq!(DrawKind::Solid(Color::WHITE).group(), 0);
    assert_eq!(
        DrawKind::Border {
            color: Color::WHITE,
            width: 1.0,
            radius: 0.0,
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
    // 首帧（prev_size 未知）：命中基准 = origin（不 clamp）；显示 = clamp(origin, size)。
    // 次帧：prev_size = size → base_pos = clamp(origin, size) = **首帧显示位置**
    // → 命中/显示收敛、位置无跳（消除"窗口刚出现就按下 → 按下瞬间跳变"）。
    let origin = Vec2::new(700.0, 300.0); // 窗口内容大，origin 超出真实 clamp 边界
    let size = Vec2::new(300.0, 100.0);
    let sw = 800.0;
    let sh = 600.0;
    let first_display = clamp_window_pos(origin, size, sw, sh);
    assert_eq!(first_display.x, 500.0, "首帧显示 clamp 到 [0, sw-size]=500");
    // 次帧命中基准 = clamp(origin, size)（首帧显示位置）→ 一致。
    let second_base = clamp_window_pos(origin, size, sw, sh);
    assert_eq!(second_base, first_display, "次帧命中基准 = 首帧显示位置（无跳变）");
    // 次帧后 origin 已持久化为 clamp 后位置 → 再 clamp 不变。
    let origin2 = first_display;
    assert_eq!(
        clamp_window_pos(origin2, size, sw, sh),
        origin2,
        "已 clamp 位置再 clamp 不变"
    );
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
