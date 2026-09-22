//! `ui` 模块的单元测试（从 `ui.rs` 拆出；无 GPU 依赖的纯逻辑回归）。
//!
//! 这些测试原本内联在 `ui.rs` 末尾（约 700 行），拆出后 `ui.rs` 只留生产代码。
//! 覆盖：文本编辑原语 / 对齐转换 / 内容签名 / 组排序 / 命令分桶等价性 /
//! 窗口限位与拖拽 / 命中与焦点 / 字符边界安全等——全部不需要 GPU。

use super::*;
use crate::draw::{PanelCmdCtx, push_panel_img_cmds};
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
        cmd_sig_hash(&mut h, d, Vec2::ZERO);
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
fn panel_cmds_draw_image_and_border_at_any_radius() {
    // 回归防线（真实 bug）：背景图与边框曾经被写在"圆角分支"里 ⇒ `radius == 0`（直角）
    // 的面板**静默丢掉背景图**——演示里「背景图：Tile（1:1 平铺，直角）」那个窗口因此
    // 一片空白（像素采样证实窗口体只有平渐变、找不到棋盘两色）。
    let rect = Rect::new(0.0, 0.0, 120.0, 60.0);
    let bg = crate::style::Brush::Solid(Color::rgba_u8(30, 40, 50, 255));
    let img = ImageBg::new(7, Vec2::new(32.0, 32.0));
    let run = |radius: CornerRadius, img: Option<ImageBg>, border_w: f32| -> Vec<DrawKind> {
        let mut out = Vec::new();
        push_panel_img_cmds(
            &mut out,
            PanelCmdCtx { depth: 0, win: 0, elem: 1, rect, clip: None, seq: 1 },
            &bg,
            img,
            Color::WHITE,
            border_w,
            radius,
        );
        out.into_iter().map(|d| d.kind).collect()
    };
    let kinds = |v: &[DrawKind]| -> Vec<&'static str> {
        v.iter()
            .map(|k| match k {
                DrawKind::Solid(_) => "solid",
                DrawKind::Rect(_) => "grad",
                DrawKind::RoundedRect { .. } => "rounded",
                DrawKind::Image(_) => "image",
                DrawKind::Border { .. } => "border",
                _ => "other",
            })
            .collect()
    };
    // 直角 + 纯色：实心 + 图 + 边框 —— **图必须在**（就是这条曾经缺失）。
    let flat = run(CornerRadius::default(), Some(img), 1.0);
    assert_eq!(kinds(&flat), ["solid", "image", "border"], "直角面板必须画背景图");
    // 圆角 + 纯色：整块圆角 + 图 + 边框。
    let rounded = run(CornerRadius::all(8.0), Some(img), 1.0);
    assert_eq!(kinds(&rounded), ["rounded", "image", "border"]);
    // 无图 / 无边框时不多推命令（层数随参数收缩，命令序仍连续）。
    assert_eq!(kinds(&run(CornerRadius::default(), None, 0.0)), ["solid"]);
    assert_eq!(kinds(&run(CornerRadius::default(), None, 2.0)), ["solid", "border"]);
    assert_eq!(kinds(&run(CornerRadius::default(), Some(img), 0.0)), ["solid", "image"]);
    // 图带的 radius 被面板 radius 覆盖（免得"图与面板圆角不一致"）。
    let with_r = run(CornerRadius::all(6.0), Some(img), 0.0);
    match with_r.iter().find(|k| matches!(k, DrawKind::Image(_))) {
        Some(DrawKind::Image(b)) => assert_eq!(b.radius, CornerRadius::all(6.0)),
        _ => panic!("应有一条 Image 命令"),
    }
}

#[test]
fn cmd_sig_covers_image_fields() {
    // 背景图的每个渲染输入都必须进签名：否则改铺排 / 染色 / 圆角 / 换纹理时
    // 窗口顶点缓存会误判"内容未变"而继续用旧顶点（图片不刷新）。
    use std::hash::Hasher;
    fn sig(d: &UiDraw) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        cmd_sig_hash(&mut h, d, Vec2::ZERO);
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
fn auto_window_pos_cascades_then_remembers() {
    use std::collections::HashMap;
    let mut map: HashMap<IdAbsolute<'static>, Vec2> = HashMap::new();
    let mut next = 0u32;
    let vp = Vec2::new(1920.0, 1080.0);
    // ① 首个窗口 = 左上留边；第二、三个逐级右下偏移（Win32 层叠）。
    let a = auto_pos_take(&mut map, &mut next, &IdAbsolute::from("win_a"), vp, 1.0);
    let b = auto_pos_take(&mut map, &mut next, &IdAbsolute::from("win_b"), vp, 1.0);
    assert_eq!(a, Vec2::new(16.0, 16.0));
    assert_eq!(b, a + Vec2::splat(AUTO_POS_STEP));
    // ② **跨帧稳定**：再问同一个 id 拿到同一个位置（不会顺着级联继续往下漂）。
    let a2 = auto_pos_take(&mut map, &mut next, &IdAbsolute::from("win_a"), vp, 1.0);
    assert_eq!(a2, a);
    assert_eq!(next, 2, "重复询问不消耗槽位");
    // ③ 回绕：槽位越界后取模回到可用范围内（不会跑出屏幕）。
    let far = auto_pos_slot(10_000, vp, 1.0);
    assert!(far.x >= 16.0 && far.x <= vp.x - 16.0, "x 在视口内: {far:?}");
    assert!(far.y >= 16.0 && far.y <= vp.y - 16.0, "y 在视口内: {far:?}");
    // ④ 小视口：跨度至少一格，不 panic / 不产生 NaN。
    let tiny = auto_pos_slot(3, Vec2::new(10.0, 10.0), 1.0);
    assert!(tiny.x.is_finite() && tiny.y.is_finite() && tiny.x >= 16.0);
    // ⑤ DPI：步长与留边都按 scale 放大（内部全物理像素）。
    let hd = auto_pos_slot(1, Vec2::new(3840.0, 2160.0), 2.0);
    assert_eq!(hd, Vec2::new(16.0 * 2.0, 16.0 * 2.0) + Vec2::splat(AUTO_POS_STEP * 2.0));
}

#[test]
fn container_builder_options_defaults_and_overrides() {
    // 窗口责任链选项默认值 = 旧 `window_at` 语义（自动宽 / 置顶 / Expand 不裁剪 /
    // 全局主题）。
    let o = WindowOptions::default();
    assert_eq!(o.pos, None, "默认位置 = 引擎自动分配（CW_USEDEFAULT 语义）");
    assert_eq!(o.width, None);
    assert_eq!(o.level, Level::Topmost, "默认点击置顶");
    assert_eq!(o.placement, Placement::Expand, "默认 Expand 语义（不裁剪）");
    assert!(o.style.is_none(), "默认跟随全局 Theme::panel");
    // 覆盖组合 = 固定宽 + Level::Normal + Placement::Clip + 逐窗口样式。
    let o2 = WindowOptions {
        pos: Some(Position::Logical(Vec2::new(10.0, 20.0))),
        width: Some(Size::Logical(300.0)),
        level: Level::Normal,
        placement: Placement::Clip,
        style: Some(PanelStyle::default().with_radius(8.0)),
        ..WindowOptions::default()
    };
    assert_eq!(o2.pos, Some(Position::Logical(Vec2::new(10.0, 20.0))));
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

#[test]
fn window_chrome_bar_and_collapse_flags() {
    // **默认零影响**：不调 `.title` / `.close_button` / `.collapsible` ⇒ 不画标题栏 ——
    // 这是"新特性不移动任何既有几何"的机器化保证（截图 / 仿真基线因此不需要更新）。
    let st = crate::UiState::default();
    let none = WindowChrome::none();
    assert!(!none.bar_on(), "空外框不画标题栏");
    assert!(!none.collapsed(&st, "w"), "空外框不收起");
    assert_eq!(none.title, None);

    // 标题 ⇒ 有栏；单独 `title` 不产生按钮。
    let c = WindowChrome { title: Some("T"), ..WindowChrome::none() };
    assert!(c.bar_on());
    assert!(c.close.is_none() && c.collapsible.is_none());

    // `close_button` ⇒ 有栏（只有 × 也是标题栏）。
    let mut open = true;
    let c = WindowChrome { close: Some(&mut open), ..WindowChrome::none() };
    assert!(c.bar_on(), "只有关闭按钮时也要有栏（否则 × 没有落脚处）");

    // `collapsible(false, Some(&mut c))`：**不画栏**，但状态照旧生效 —— 菜单/代码收起窗口用。
    let mut collapsed = true;
    let c = WindowChrome {
        collapsible: Some((false, Some(&mut collapsed))),
        ..WindowChrome::none()
    };
    assert!(!c.bar_on(), "show=false ⇒ 不画标题栏（按钮不画）");
    assert!(c.collapsed(&st, "w"), "show=false 时 *collapsed 仍管布局");
    assert!(!c.show_collapse(), "show=false ⇒ 不画 ⌃");
    assert_eq!(c.collapsible.as_ref().map(|(s, _)| *s), Some(false));

    // `collapsible(true, ..)` ⇒ 有栏 + 收起状态透传。
    let mut collapsed2 = true;
    let c = WindowChrome {
        collapsible: Some((true, Some(&mut collapsed2))),
        ..WindowChrome::none()
    };
    assert!(c.bar_on() && c.show_collapse());
    assert!(c.collapsed(&st, "w"));
    // 标题 + 收起按钮 = 典型标题栏（demo 的 win_a 就是这种）。
    let mut c3 = false;
    let c = WindowChrome {
        title: Some("图形"),
        collapsible: Some((true, Some(&mut c3))),
        ..WindowChrome::none()
    };
    assert!(c.bar_on() && !c.collapsed(&st, "w"));

    // **`None` = 引擎托管**：状态读 `UiState::collapsed`（按窗口**绝对 ID**）。
    let mut st = crate::UiState::default();
    let c = WindowChrome { collapsible: Some((true, None)), ..WindowChrome::none() };
    assert!(c.bar_on());
    assert!(!c.collapsed(&st, "win_b"), "引擎里没有记录 ⇒ 不收起");
    st.set_collapsed("win_b", true);
    assert!(c.collapsed(&st, "win_b"), "引擎记录为收起 ⇒ 本帧只留标题栏");
    assert!(!c.collapsed(&st, "win_a"), "只按自己的绝对 ID 取，串不到别的窗口");
    // `show = false` + `None`：按钮不画，但引擎托管的状态照旧生效（与 `Some` 一致）。
    let c2 = WindowChrome { collapsible: Some((false, None)), ..WindowChrome::none() };
    assert!(!c2.bar_on() && c2.collapsed(&st, "win_b"));
}

#[test]
fn window_resize_switch_resolves_old_default_and_explicit_false() {
    // **没设 `.resize(..)` = 旧行为**：有 `.width(..)` 就能横向拖，没有就不出来柄。
    // （这是"新开关不改变既有窗口行为"的机器化保证。）
    assert_eq!(resolve_window_resize(None, false), (false, Resize::Horizontal));
    assert_eq!(resolve_window_resize(None, true), (true, Resize::Horizontal));
    // **显式关闭**：`resize(false, Resize::None)` ⇒ 不画柄、不响应拖拽（但 `.width(..)`
    // 仍是布局固定宽）——菜单 / 下拉浮层用它换"固定宽但尺寸不可拖"。
    assert_eq!(resolve_window_resize(Some((false, Resize::None)), true), (false, Resize::None));
    // **显式开启**：宽高同调（`Both`）；`allow=false` 时轴怎么写都不生效（不画不拖）。
    assert_eq!(resolve_window_resize(Some((true, Resize::Both)), false), (true, Resize::Both));
    assert_eq!(resolve_window_resize(Some((false, Resize::Both)), true), (false, Resize::Both));
    // 只否允许、轴仍留 Horizontal：等价"不能拖但语义上还是横轴"。
    assert_eq!(
        resolve_window_resize(Some((false, Resize::Horizontal)), true),
        (false, Resize::Horizontal)
    );
}

#[test]
fn resize_axes_pick_the_axes_that_the_user_takes_over() {
    // **轴 → "谁接管这一轴"** 的机器化表格（`Resize::Vertical` 是本轮新增的变体）：
    // 拖过的轴由用户接管（宽度进 `window_widths`、高度进 `window_heights`，且固定轴
    // 不参与内容撑开；高度被拖过还会让窗口成为固定尺寸视口 ⇒ `window_content_clipped`）。
    assert!(resize_fixes_width(Resize::Horizontal) && !resize_fixes_height(Resize::Horizontal));
    assert!(!resize_fixes_width(Resize::Vertical) && resize_fixes_height(Resize::Vertical));
    assert!(resize_fixes_width(Resize::Both) && resize_fixes_height(Resize::Both));
    assert!(!resize_fixes_width(Resize::None) && !resize_fixes_height(Resize::None));
    // `Vertical` 也能出现在"显式允许"的位置上（不再只能 Horizontal / Both）。
    assert_eq!(
        resolve_window_resize(Some((true, Resize::Vertical)), false),
        (true, Resize::Vertical)
    );
    // 没给 `.width()` + `Vertical` ⇒ 宽度轴仍由内容决定（`width.is_some()` 才走旧默认）。
    assert_eq!(resolve_window_resize(None, false).1, Resize::Horizontal);
}

#[test]
fn scroll_mode_resolution_prefers_explicit_then_legacy_then_dragged() {
    use ScrollMode::*;
    // ① **显式设置胜**（`.vscroll` / `.hscroll` 覆盖 `Placement` 与"拖过尺寸"的老语义）。
    assert_eq!(resolve_scroll_mode(Some(NoClip), true, true), NoClip);
    assert_eq!(resolve_scroll_mode(Some(ClipOnly), false, false), ClipOnly);
    assert_eq!(resolve_scroll_mode(Some(Scroll), false, true), Scroll);
    // ② 没显式给：`Placement::Clip` ⇒ 两条轴都裁（老行为）。
    assert_eq!(resolve_scroll_mode(None, true, false), ClipOnly);
    // ③ 没显式给、也没 Clip：**被用户拖过尺寸的那条轴**自动成为视口
    //    （"拖过高度之后内容会被裁掉"既有语义的机器化表达）。
    assert_eq!(resolve_scroll_mode(None, false, true), ClipOnly);
    // ④ 都没有 ⇒ 不裁（与不加本 API 之前逐像素一致）。
    assert_eq!(resolve_scroll_mode(None, false, false), NoClip);
    // **与老判据的等价性**：两条轴都按老判据解算时，"是否裁"必须与
    // `window_content_clipped(strict, fixed_h)` 完全一致（否则就是无声的行为漂移）。
    for strict in [false, true] {
        for fixed_h in [None, Some(120.0)] {
            let v = resolve_scroll_mode(None, strict, fixed_h.is_some());
            let h = resolve_scroll_mode(None, strict, fixed_h.is_some());
            let clipped = v != NoClip || h != NoClip;
            assert_eq!(
                clipped,
                window_content_clipped(strict, fixed_h),
                "strict={strict} fixed_h={fixed_h:?}"
            );
        }
    }
}

#[test]
fn clip_for_axes_only_clips_the_axes_that_ask() {
    use ScrollMode::*;
    let win = Rect::new(100.0, 200.0, 300.0, 150.0);
    // "不裁"那条轴的兜底：引擎里传的是**屏幕**矩形（不裁 ≠ 无限，只是别用窗口边界裁）。
    let screen = Rect::new(0.0, 0.0, 1920.0, 1080.0);
    // 两条轴都 NoClip 且没有外层裁剪 ⇒ **不动**（返回 None，与旧行为逐像素一致）。
    assert_eq!(clip_for_axes(None, win, screen, NoClip, NoClip), None);
    // 只裁纵向：y 收到窗口范围；x **不裁**（用屏幕兜底）⇒ 横向溢出仍可见。
    let c = clip_for_axes(None, win, screen, ClipOnly, NoClip).expect("有裁剪");
    assert_eq!((c.y, c.h), (win.y, win.h), "纵向 = 窗口范围");
    assert!(c.x < win.x && c.w > win.w, "横向不裁：x 范围远大于窗口");
    // 只裁横向：镜像。
    let c = clip_for_axes(None, win, screen, NoClip, ClipOnly).expect("有裁剪");
    assert_eq!((c.x, c.w), (win.x, win.w), "横向 = 窗口范围");
    assert!(c.y < win.y && c.h > win.h, "纵向不裁");
    // 两条轴都裁 ⇒ 与老的 `clip_for_view(.., Clip)` **等价**。
    let both = clip_for_axes(None, win, screen, ClipOnly, ClipOnly).expect("有裁剪");
    assert_eq!(both, clip_for_view(None, win, ViewMode::Clip).expect("老实现也给裁剪"));
    let outer = Rect::new(150.0, 250.0, 400.0, 400.0);
    assert_eq!(
        clip_for_axes(Some(outer), win, screen, ClipOnly, ClipOnly),
        clip_for_view(Some(outer), win, ViewMode::Clip),
        "有外层裁剪时也要与老实现等价"
    );
    // 外层裁剪 ∩ 单轴裁剪：外层仍然生效（不会因为"另一条轴不裁"就被丢掉）。
    let c = clip_for_axes(Some(outer), win, screen, ClipOnly, NoClip).expect("有裁剪");
    assert_eq!(c.y, outer.y.max(win.y), "纵向取两者交集");
    assert_eq!(c.x, outer.x, "横向沿用外层");
    assert_eq!(c.w, outer.w);
}

#[test]
fn window_content_clips_when_height_is_user_fixed() {
    // 老判据本身（`window_impl` 已改用按轴解算，本测试与上一条的"等价性"一起守住它）。
    // **内容裁剪的触发条件**：
    // ① 应用显式 `.placement(Placement::Clip)`；
    // ② **高度被用户拖过**（`Resize::Both` 的柄 ⇒ 窗口成了固定尺寸视口）——不裁剪的话
    //    内容会画到窗口外面（用户实测的 TTT 窗口 bug："内容不会被裁剪"）。
    // ⚠ 固定**宽**不触发：固定宽窗口的高度仍由内容决定，垂直方向没有溢出可言。
    assert!(window_content_clipped(true, None), "显式 Clip");
    assert!(window_content_clipped(false, Some(120.0)), "高度被拖过 ⇒ 视口裁剪");
    assert!(window_content_clipped(true, Some(120.0)));
    assert!(!window_content_clipped(false, None), "默认 Expand 不裁剪");
}

#[test]
fn overlay_z_bands_by_nesting_depth() {
    // 浮层 z = **基址 + 嵌套层数**：子浮层 z 更大 ⇒ 整段（含**阴影**）画在父浮层之后。
    // 同一个 z 会让两层的命令落进同一个 `(win, elem)` 分组排序，而窗口阴影/背景是
    // `elem = 0`、控件是 `elem ≥ 1` ⇒ 子层阴影被父层控件盖住
    // （用户实测："下级 popup 阴影被绘制在了上级控件后面"）。
    assert_eq!(overlay_z(0), WIN_TOPMOST);
    assert_eq!(overlay_z(1), WIN_TOPMOST + 1);
    assert!(overlay_z(0) < overlay_z(1), "子浮层 z 必须更大");
    // 整个浮层区间都在真实窗口 z 之上（真实 z 从 1 起按 `max+1` 递增，够不到基址）。
    assert!(is_overlay_z(overlay_z(0)));
    assert!(is_overlay_z(overlay_z(7)));
    assert!(!is_overlay_z(WIN_TOPMOST - 1), "基址之下不是浮层");
    assert!(!is_overlay_z(1), "普通窗口不是浮层");
    // 层数超出上限时 clamp（不回绕、也不会溢出成普通窗口区间）。
    assert!(is_overlay_z(overlay_z(u32::MAX)));
    assert_eq!(overlay_z(u32::MAX), overlay_z(OVERLAY_Z_SPAN));
}

#[test]
fn title_bar_hugs_the_top_and_does_not_clip_content() {
    // 标题行**贴窗口顶边**录（`window_title_bar` 把内容光标抬到 y=0）⇒ 条高 = **一行**，
    // 不再含上内边距（用户实测："可以往上抬"）。
    assert_eq!(title_bar_h(45.0), 45.0);
    // ⚠ 条只是**背景装饰**：标题 / ▲ / ✕ 的边长是 `row_h - 2`，允许**比条高**（用户明确
    // 要求"内容可以比条高再高一点"）⇒ 条高**与按钮边长无关**（这里只吃 `row_h` 一个参数，
    // 谁想改成"条比内容更矮"就改这一个函数）。
    let btn = 45.0 - 2.0;
    assert!(btn < title_bar_h(45.0) + 1.0, "按钮可以接近/超过条高");
}

#[test]
fn title_bar_buttons_hug_the_window_outer_right_edge() {
    // **Windows 风格**：固定尺寸窗口的 caption 簇（`[收缩][关闭]`）右缘 = **外框右缘** − inset。
    // 旧实现把按钮当行内子项，靠 `spacer = 内容宽 − 标题宽` 推到**内容**右缘 ⇒ 永远差
    // `pad + 4`（实测 win_a @scale1.5：✕ 右缘 340 / 外框 358 ⇒ 偏左 18px）。这条断言是那个
    // 偏差的守卫：算错了就一定失败（不是"点得到就行"）。
    let (pad, row_h, gap, inset) = (14.0, 39.0, 9.0, TITLE_BUTTON_INSET);
    let btn = row_h - 2.0;
    let l = title_bar_layout(Some(358.0), pad, row_h, gap, true, true, inset, 62.0);
    let close = l.close.expect("有关闭按钮");
    assert_eq!(close.x + close.w, 358.0 - inset, "✕ 贴外框右缘");
    // 收缩在它左边一个按钮 + 一个 gap（顺序：关闭恒在最右）。
    let shrink = l.collapse.expect("有收缩按钮");
    assert_eq!(shrink.x + shrink.w + gap, close.x);
    assert_eq!((close.w, close.h), (btn, row_h));
    assert_eq!(close.y, 0.0, "按钮贴窗口顶边（Windows 的 caption 观感）");
    // 标题可用宽 = 簇左缘 − gap − 左内边距（不会压到按钮上）。
    assert_eq!(l.title_max, shrink.x - gap - pad);
    assert!(l.title_max > 0.0);
    assert_eq!(l.bar_w, 358.0, "固定宽窗口的条宽 = 外框宽");
}

#[test]
fn title_bar_layout_inset_and_single_button() {
    let (pad, row_h, gap) = (14.0, 39.0, 9.0);
    // inset 生效：留给"想在圆角外留一条缝"的开关（`TITLE_BUTTON_INSET` 是唯一入口）。
    let l = title_bar_layout(Some(358.0), pad, row_h, gap, false, true, 12.0, 0.0);
    let close = l.close.expect("只有关闭按钮");
    assert_eq!(close.x + close.w, 358.0 - 12.0);
    assert!(l.collapse.is_none(), "show_collapse=false ⇒ 不画收起");
    // 只有收缩按钮时它自己接最右（只有一个按钮就占 0 号位）。
    let l = title_bar_layout(Some(358.0), pad, row_h, gap, true, false, 0.0, 0.0);
    assert!(l.close.is_none());
    assert_eq!(l.collapse.expect("只有收缩按钮").x, 358.0 - (row_h - 2.0));
    // 两个都不画：不产生矩形，标题可用宽仍是"到外框右缘"。
    let l = title_bar_layout(Some(358.0), pad, row_h, gap, false, false, 0.0, 0.0);
    assert!(l.collapse.is_none() && l.close.is_none());
    assert_eq!(l.title_max, 358.0 - gap - pad);
}

#[test]
fn title_bar_layout_clamps_in_tiny_windows_and_follows_title_when_auto() {
    let (pad, row_h, gap) = (14.0, 39.0, 9.0);
    // 窗口比按钮簇还窄：簇左缘夹到 `pad`（按钮不左越内容左缘），标题宽不产生负值
    // （负宽会让 `max_size` 把标题压成 0，甚至让省略号路径拿到负可用宽）。
    let l = title_bar_layout(Some(60.0), pad, row_h, gap, true, true, 0.0, 500.0);
    assert_eq!(l.collapse.expect("有收缩按钮").x, pad);
    assert_eq!(l.title_max, 0.0);
    // 自动宽窗口（无 `.width()`）：簇**跟随标题**（与旧版自动宽窗口一致），
    // 外框宽 = 内容右上角 + 右内边距（与 `Frame::natural_size` / `settle_size` 同口径）。
    let l = title_bar_layout(None, pad, row_h, gap, true, true, 0.0, 62.0);
    assert_eq!(l.collapse.expect("有收缩按钮").x, pad + 62.0 + gap, "自动宽：按钮跟在标题后");
    let close = l.close.expect("有关闭按钮");
    assert_eq!(l.bar_w, close.x + close.w + pad);
    // **不变量与 DPI 无关**（输入已是物理像素）：两套尺寸各自都贴右缘。
    for (bar_w, pad, row_h, gap) in [(358.0, 14.0, 39.0, 9.0), (716.0, 28.0, 78.0, 18.0)] {
        let l = title_bar_layout(Some(bar_w), pad, row_h, gap, true, true, 0.0, 124.0);
        let close = l.close.expect("有关闭按钮");
        assert_eq!(close.x + close.w, bar_w, "bar_w={bar_w} 下也必须贴右缘");
        assert_eq!(close.h, row_h);
    }
}

// ─── 顶层放置序（`z0_place_for_seq`）与提交排序键 ─────────────────────
//
// 这一组把用户报告的两个**闪烁**变成断言。症状：可拖动玩家名面板的底色被更早录制的
// win=0 内容（FPS / 点击次数标签）穿透；`scroll_at` 的滚动条被列表项盖住（只在条目
// 间隙里忽隐忽现）。根因：绘制序只有 `(win, elem, ...)`，而**所有非窗口内容共享
// `win = 0`**，于是"容器装饰用 `elem = 0`（画在本容器元素之下）"退化成
// "画在整个 win=0 空间的最底"。修法：在 `win` 之后插入**顶层放置序** `place`。

#[test]
fn place_is_zero_before_the_first_placement() {
    // 开头的散装顶层命令（`label_at` 等）不属于任何放置 ⇒ 序 0。
    let starts = [5u32, 20, 40];
    assert_eq!(z0_place_for_seq(&starts, 0), 0);
    assert_eq!(z0_place_for_seq(&starts, 4), 0);
    // 放置 1 从 seq=5 开始 ⇒ seq=5 已属于它。
    assert_eq!(z0_place_for_seq(&starts, 5), 1);
}

#[test]
fn place_is_constant_inside_a_placement_and_grows_after_it() {
    // 放置 1 = seq 5..19，放置 2 = seq 20..39，放置 3 = seq 40..。
    let starts = [5u32, 20, 40];
    for seq in 5..20 {
        assert_eq!(z0_place_for_seq(&starts, seq), 1, "放置内恒定（seq={seq}）");
    }
    for seq in 20..40 {
        assert_eq!(z0_place_for_seq(&starts, seq), 2);
    }
    for seq in 40..60 {
        assert_eq!(z0_place_for_seq(&starts, seq), 3);
    }
    // 单调不减（提前 break 的正确性前提就是 starts 递增）。
    let mut prev = 0;
    for seq in 0..80 {
        let p = z0_place_for_seq(&starts, seq);
        assert!(p >= prev, "place 必须单调不减（seq={seq}）");
        prev = p;
    }
}

#[test]
fn place_splits_a_run_of_loose_commands_around_placements() {
    // ⚠ 这一条是本次修复的**核心**：散装命令与放置交错时，
    // "放置 A → 散装 → 放置 B" 必须得到 1 / 1 / 2（散装跟随"上一个放置"），
    // 于是后续放置的 `place` 严格更大 ⇒ 它的装饰（elem=0）不会被前面的内容穿透。
    let starts = [10u32, 30];
    assert_eq!(z0_place_for_seq(&starts, 0), 0, "放置 A 之前的散装");
    assert_eq!(z0_place_for_seq(&starts, 10), 1, "放置 A");
    assert_eq!(z0_place_for_seq(&starts, 25), 1, "A 与 B 之间的散装跟随 A");
    assert_eq!(z0_place_for_seq(&starts, 30), 2, "放置 B 严格更大");
}

#[test]
fn place_boundary_is_the_first_command_not_the_container_entry() {
    // ⚠ 回归：`begin_top_placement` 必须记"**本放置第一条命令**的 seq"
    // （`queue.seq + 1`），而不是容器入口时的 `queue.seq`。
    //
    // 现场（实测）：`scroll_at` 的滚动条由 `next_seq()` 取号（seq = N），紧接着
    // 后面的 `flex_at` 容器在同一 `seq = N` 上开新放置。若记 `N`，滚动条那条命令
    // （seq = N）就被 `≤` 命中到**新**放置里 ⇒ 滚动条与它自己的列表项分属两个
    // 排序空间（`place` 差 1），序随录制细节漂移。记 `N + 1` 后滚动条留在原放置。
    let starts = [1u32, 83]; // 第 2 个放置的第一条命令是 seq=83
    assert_eq!(z0_place_for_seq(&starts, 82), 1, "seq=82 属于上一个放置（滚动条）");
    assert_eq!(z0_place_for_seq(&starts, 83), 2, "seq=83 才是新放置的第一条");
    // 反例：若把起点记成"容器入口 seq"（82），seq=82 会被算进新放置。
    let wrong = [1u32, 82];
    assert_eq!(z0_place_for_seq(&wrong, 82), 2, "这就是踩过的 off-by-one");
}

#[test]
fn empty_placements_still_consume_a_place() {
    // 空闭包的 `pack_at` 会"开了放置但一条命令都没录"。它不产生几何，但计数照加——
    // 后面的放置因此拿到更大的 place（序仍然正确）。
    let starts = [7u32, 7, 9];
    assert_eq!(z0_place_for_seq(&starts, 7), 2, "同一起点的两个放置都算");
    assert_eq!(z0_place_for_seq(&starts, 8), 2);
    assert_eq!(z0_place_for_seq(&starts, 9), 3);
}

/// 构造一条合成几何段（`place` / `elem` 之外都取最小可用值）。
fn cq(win: u32, place: u32, elem: u32) -> crate::gpu_batch::CachedQuad {
    (
        win,
        place,
        elem,
        0,
        1,
        None,
        crate::gpu_batch::Geom::default(),
        Vec::new(),
    )
}

#[test]
fn submit_order_puts_a_placement_above_earlier_content_and_below_its_own_children() {
    // 现场 = `--sim-zorder` 的那个：顶部两个散装标签（place 0，elem 1/2）→
    // 按钮行（place 1）→ **可拖动面板（place 2）**，面板的投影 / 底色是 `elem = 0`、
    // 面板自己的 label / 输入框是 elem 15/16。
    let mut v = [
        cq(0, 2, 15), // 面板自己的内容
        cq(0, 0, 2),  // 散装标签 2
        cq(0, 1, 3),  // 按钮行（place 1）
        cq(0, 2, 0),  // **面板底色 / 投影（elem = 0）**
        cq(0, 0, 1),  // 散装标签 1
    ];
    v.sort_unstable_by_key(crate::gpu_batch::submit_sort_key);
    let order: Vec<(u32, u32)> = v.iter().map(|q| (q.1, q.2)).collect();
    assert_eq!(
        order,
        vec![(0, 1), (0, 2), (1, 3), (2, 0), (2, 15)],
        "散装标签(0,1/0,2) → 按钮行(1,3) → **面板底色(2,0)** → 面板内容(2,15)"
    );
    // 关键不变量：面板的 elem=0 装饰**必须排在按钮行之后**（否则被按钮穿透），
    // 且**必须排在自己的内容之前**（否则底色盖住自家文字）。
    let idx = |pl: u32, el: u32| order.iter().position(|&x| x == (pl, el)).unwrap();
    assert!(idx(2, 0) > idx(1, 3), "面板底色不得被更早录制的放置穿透");
    assert!(idx(2, 0) < idx(2, 15), "面板底色仍必须在自家内容之下");
}

#[test]
fn submit_order_puts_a_scrollbar_above_its_own_items() {
    // 现场 = `scroll_at`：列表项（place 1，elem ≥ 1，各自裁剪）与滚动条轨道 / 滑块。
    // 修法后滚动条用 `elem_hint()`（在内容**之后**录制）⇒ 排在自家内容**之上**。
    // 修法前滚动条用 `elem = 0` ⇒ 排在 (place 1, elem 0) ⇒ 被列表项盖住（闪烁）。
    let mut fixed = [
        cq(0, 1, 0),  // 滚动条轨道（elem_hint 之后 → 实际不会是 0，见下）
        cq(0, 1, 1),  // 列表项 0
        cq(0, 1, 2),  // 列表项 1
    ];
    // 修复后的键：滚动条 elem 取"内容之后的下一个元素序"（这里以 3 代表）。
    fixed[0].2 = 3;
    fixed.sort_unstable_by_key(crate::gpu_batch::submit_sort_key);
    let order: Vec<u32> = fixed.iter().map(|q| q.2).collect();
    assert_eq!(order, vec![1, 2, 3], "列表项 → 滚动条（滑块最后画 = 在最上）");

    // 反例（修复前的形态）：滚动条 elem = 0 ⇒ 排到列表项之前（被盖住）。
    let mut broken = [cq(0, 1, 0), cq(0, 1, 1), cq(0, 1, 2)];
    broken.sort_unstable_by_key(crate::gpu_batch::submit_sort_key);
    let broken_order: Vec<u32> = broken.iter().map(|q| q.2).collect();
    assert_eq!(
        broken_order,
        vec![0, 1, 2],
        "elem=0 的滚动条排在列表项之前 = 被列表项盖住（正是要修掉的现象）"
    );
}

#[test]
fn submit_order_still_keeps_windows_above_non_window_content() {
    // `win` 仍是第一位：非窗口内容（0）恒在窗口（≥1）之下——本修复不得动摇这条
    // （窗口 z 序 / 遮挡语义都建立在它上面）。
    let mut v = [cq(3, 0, 0), cq(0, 9, 0), cq(1, 0, 5), cq(0, 0, 1)];
    v.sort_unstable_by_key(crate::gpu_batch::submit_sort_key);
    let order: Vec<(u32, u32, u32)> = v.iter().map(|q| (q.0, q.1, q.2)).collect();
    assert_eq!(
        order,
        vec![(0, 0, 1), (0, 9, 0), (1, 0, 5), (3, 0, 0)],
        "win 升序优先，place 只在同 win 内比较"
    );
}

/// **边界守卫**（正典：`docs/UI_ARCHITECTURE.md` §2.5）：`widgets/` 是**控件层**，只许用公开面。
///
/// 用源码文本做"明显越界"的机器检查——它守的是粗线条（原始绘制队列 / 播放头 / crate 私有
/// 字段 / 跨层依赖 / `ui_mut`），**不替代 review**。失败信息直接指向 §2.5 与判据编号。
///
/// 白名单（`ALLOW`）**必须逐条写理由**，否则守卫会退化成噪音。
#[test]
fn widget_boundary_guard() {
    // 全部控件源码（`include_str!` 相对本文件 `src/ui/tests.rs` ⇒ `../widgets/…`）。
    const FILES: &[(&str, &str)] = &[
        ("button.rs", include_str!("../widgets/button.rs")),
        ("checkbox.rs", include_str!("../widgets/checkbox.rs")),
        ("colorpicker.rs", include_str!("../widgets/colorpicker.rs")),
        ("colorpicker/format.rs", include_str!("../widgets/colorpicker/format.rs")),
        ("colorpicker/hsv.rs", include_str!("../widgets/colorpicker/hsv.rs")),
        ("colorpicker/panel.rs", include_str!("../widgets/colorpicker/panel.rs")),
        ("colorpicker/state.rs", include_str!("../widgets/colorpicker/state.rs")),
        ("divider.rs", include_str!("../widgets/divider.rs")),
        ("dropdown.rs", include_str!("../widgets/dropdown.rs")),
        ("fontmodal.rs", include_str!("../widgets/fontmodal.rs")),
        ("label.rs", include_str!("../widgets/label.rs")),
        ("menu.rs", include_str!("../widgets/menu.rs")),
        ("menubar.rs", include_str!("../widgets/menubar.rs")),
        ("numberinput.rs", include_str!("../widgets/numberinput.rs")),
        ("segmented.rs", include_str!("../widgets/segmented.rs")),
        ("slider.rs", include_str!("../widgets/slider.rs")),
        ("texteditor.rs", include_str!("../widgets/texteditor.rs")),
        ("title_button.rs", include_str!("../widgets/title_button.rs")),
    ];
    // 禁用子串 + 它违反的判据（§2.5.2）。
    const BANNED: &[(&str, &str)] = &[
        ("painter.q", "判据 5：绘制队列属于 Painter；引擎/控件都不得直接 push 原始队列"),
        ("next_seq", "判据 5：播放头属于绘制层（缺原语就在 Painter 补，见判据 3）"),
        ("crate::tess", "§2.5.1：widgets 不得依赖 tess"),
        ("crate::gpu_batch", "§2.5.1：widgets 不得依赖 gpu_batch"),
        ("window_rects", "判据 2/3：几何事实走 `state().windows()` 模块视图，不读字段"),
        ("ui.theme.", "判据 3：读 `ui.theme()` 访问器；写引擎主题是判据 4 的违规"),
    ];
    // 白名单：`(文件, 允许的子串, 理由)`。**加白名单就是加欠账**，理由必须能追溯。
    const ALLOW: &[(&str, &str, &str)] = &[
        (
            "texteditor.rs",
            "ui.theme.",
            "§2.5.3 #2 **未修**：文本核心仍靠临时改帧内主题拿样式（D3 第一步改 `&InputStyle` 参数后删除本白名单）",
        ),
        (
            "menu.rs",
            ".ui_mut()",
            "判据 6 例外：`MenuCtx` 是**容器**（`Deref` 到 `Window`），容器实现可用 `ui_mut`",
        ),
        (
            "menubar.rs",
            ".ui_mut()",
            "判据 6 例外：`MenuBar` 是**容器**（`Deref` 到 `Pack`），同上",
        ),
        (
            "colorpicker/panel.rs",
            ".ui_mut()",
            "判据 6 例外：取色弹层里 `w` 是**容器**（popup `Window`），同上",
        ),
        (
            "fontmodal.rs",
            ".ui_mut()",
            "§2.5.3 #3 记缺口：组合控件需要 `child_rect`/`cursor_pos`/`wrap_buffer`/`push_*` 的公开 compose 面（D4）——待补",
        ),
    ];
    let allowed = |file: &str, pat: &str| ALLOW.iter().any(|(f, p, _)| *f == file && *p == pat);

    for (name, src) in FILES {
        for (pat, why) in BANNED {
            assert!(
                !src.contains(pat) || allowed(name, pat),
                "widgets/{name} 含禁用子串 {pat:?} —— {why}（docs/UI_ARCHITECTURE.md §2.5.2）"
            );
        }
    }
    // 守卫自身：白名单里的文件必须真的在 FILES 里（防拼错路径 ⇒ 白名单形同虚设）。
    for (file, _, _) in ALLOW {
        assert!(
            FILES.iter().any(|(f, _)| f == file),
            "白名单引用了不存在的控件文件 {file:?}"
        );
    }
    // 守卫自身：**它必须真的会失败**——合成一段含禁用子串的源码要被抓住
    // （否则 `BANNED` 写错字也会"永远绿"）。
    let fake = "fn ui(ui: &mut Ui) { ui.painter.q.queue.push(UiDraw { .. }); }";
    for (pat, _) in BANNED {
        if *pat == "painter.q" {
            assert!(fake.contains(pat), "守卫失效：合成源码没被 {pat:?} 抓住");
            assert!(!allowed("button.rs", pat), "button.rs 不该被白名单放行 {pat:?}");
        }
    }
}
