//! 「上层窗口没挡住背后窗口的控件」脚本化复现（`--sim-cover`）。
//!
//! 两个窗口 + 一个拖拽探针，**故意**把"录制顺序"与"z 序"解耦：
//!
//! - **探针窗**先录制、z 预置为 20（较高）；
//! - **移动窗**后录制、z 预置为 10（较低）。
//!
//! 这样"几何上盖住探针"与"画在探针之上"是两件事，两条真实路径都能被隔离出来：
//!
//! 1. **同帧移动**（帧 30）：移动窗"本帧才移到探针上"，而按下也发生在同一帧。
//!    命中发生在移动窗写入遮挡表**之前** ⇒ 判定用的仍是上一帧（让开）的位置。
//! 2. **应用改 z**（帧 60）：应用在帧首把移动窗 z 抬到 500（"置顶到最前"——与点击置顶
//!    在帧末改 z 是同一类状态变化）。本帧它**画在探针之上**，但遮挡表里它的矩形还键在
//!    **旧 z**（10 < 探针的 20）⇒ 探针被判"没被遮挡"，按下照样落在它身上。
//!
//! 断言（`carried == 0`）：探针**从未**出现"带着上一帧的按下进来"的帧。被错误认领的按下
//! 会让 `WidgetState::pressed` 一直带到释放帧（`update_interact` 只在释放时清），于是
//! 后续每帧都以"还按着"的状态进入控件——这正是"被遮挡的控件仍被触发"的引擎可观测形式。
//! 两条路径分别需要两处修复，见 `docs/DEBUGGING.md`。

use rjw_krusie::prelude::*;
use rjw_krusie::ui::draw::{Size, TextVAlign};
use rjw_krusie::ui::hit::update_drag;
use rjw_krusie::ui::{IdAbsolute, Position, Response, TextAlign, Widget, WindowClamp};

/// 拖拽探针尺寸（**物理像素**，固定值 ⇒ 脚本算得出命中点）。
pub const PROBE_W: f32 = 240.0;
/// 拖拽探针高（同上）。
pub const PROBE_H: f32 = 48.0;
/// 移动窗宽（物理像素）。
pub const MOVER_W: f32 = 420.0;
/// 移动窗 id。
pub const MOVER_ID: &str = "cover_mover";
/// 探针窗 id。
pub const PROBE_WIN_ID: &str = "cover_probe_win";
/// 移动窗预置 z（**较高**：默认画在探针窗之上 ⇒ 它盖住探针时，探针不该被命中）。
pub const MOVER_Z: u32 = 20;
/// 探针窗预置 z（**较低**）。
pub const PROBE_Z: u32 = 10;
/// 段 B 用：把移动窗压到探针窗**之下**（准备"应用把它抬上来"）。
pub const MOVER_Z_LOW: u32 = 5;
/// 段 B 用：脚本把移动窗"置顶到最前"。
pub const LIFT_Z: u32 = 500;

/// 移动窗**盖住探针**时的位置（物理像素、屏幕坐标）。
pub const MOVER_OVER: Vec2 = Vec2::new(40.0, 640.0);
/// 移动窗**让开**时的位置（物理像素）。
pub const MOVER_AWAY: Vec2 = Vec2::new(1120.0, 900.0);
/// 探针窗位置（物理像素；落在 `MOVER_OVER` 的纵向范围内 ⇒ 被盖住）。
pub const PROBE_POS: Vec2 = Vec2::new(40.0, 760.0);
/// 探针窗宽（物理像素）。
pub const PROBE_WIN_W: f32 = 300.0;

/// 灰色 = 未被按下；蓝色 = 正在被拖（"被触发了"的样子）。
const GREY: Color = Color::rgba_u8(122, 122, 122, 255);
/// 见 [`GREY`]。
const BLUE: Color = Color::rgba_u8(0, 162, 232, 255);

/// 「被遮挡控件仍被触发」复现器（状态 = 探针的几次计数）。
#[derive(Default)]
pub struct CoverDemo {
    /// 探针**认领按下**的次数（`down_edge` 且命中）——被盖住时不该发生（信息性计数）。
    pub starts: u32,
    /// **拖拽活着的帧数**（`dragging == true`）——正对照：没有任何窗口盖住探针时，
    /// 窗口内控件**必须能被拖动**（曾经被帧末复核误撤而拖不动）。
    pub drag_frames: u32,
    /// **被盖住却还在拖**的帧数（`dragging && !hit`）——错误认领的按下会一直拖到释放，
    /// 这正是"被遮挡的控件仍被触发"的用户可见后果。必须是 **0**。
    pub covered_drags: u32,
    /// 本帧解算出的探针命中点（屏幕物理像素；脚本按下用）。
    pub probe_point: Vec2,
    /// 移动窗是否**确实盖住**探针命中点（脚本前置条件；不成立说明演示布局失效）。
    pub covers: bool,
    /// z 是否已预置（只在首帧做一次——之后点击置顶会改 z，不能被"复位"覆盖）。
    seeded: bool,
}

impl CoverDemo {
    /// 帧首：把移动窗 z 抬高（"应用把它置顶到最前"）。点击置顶在帧末改 z 是同一类
    /// 状态变化，但这里**不占用鼠标按键** ⇒ 本帧还能跟一次全新按下（暴露缺陷的关键）。
    pub fn lift_mover(ui: &mut Ui) {
        Self::set_mover_z(ui, LIFT_Z);
    }

    /// 帧首：把移动窗压到探针窗**之下**（段 B 的前置条件）。
    pub fn lower_mover_z(ui: &mut Ui) {
        Self::set_mover_z(ui, MOVER_Z_LOW);
    }

    /// 帧首：把两个窗口的 z 复位成预置值（脚本在两段之间调用——上一段的按下会把被点窗口
    /// 置顶，不复位的话下一段的前置条件"移动窗画在探针窗之上"就不成立了）。
    pub fn reseed_z(ui: &mut Ui) {
        Self::set_mover_z(ui, MOVER_Z);
        ui.state_mut()
            .window_z
            .insert(IdAbsolute::owned(PROBE_WIN_ID.to_owned()), PROBE_Z);
    }

    fn set_mover_z(ui: &mut Ui, z: u32) {
        ui.state_mut()
            .window_z
            .insert(IdAbsolute::owned(MOVER_ID.to_owned()), z);
    }

    /// 录制两个窗口（**探针窗先录、移动窗后录**；`over` = 移动窗本帧是否移到探针上）。
    pub fn ui(&mut self, ui: &mut Ui, over: bool) {
        if !self.seeded {
            self.seeded = true;
            // 只在首帧预置：之后脚本 / 点击置顶会改 z，不能每帧覆盖。
            if !ui.state().window_z.contains_key(MOVER_ID) {
                Self::set_mover_z(ui, MOVER_Z);
            }
            if !ui.state().window_z.contains_key(PROBE_WIN_ID) {
                ui.state_mut()
                    .window_z
                    .insert(IdAbsolute::owned(PROBE_WIN_ID.to_owned()), PROBE_Z);
            }
        }
        // ① 探针窗**先录**：它的命中判定发生在移动窗写入遮挡表之前。
        // `Locked`：位置只由脚本给（用户拖不动）——复现里的按下落点同时落在两个窗口内，
        // 若允许拖拽，窗口会跟着鼠标跑，几何就不再是"脚本说的那个"。
        let probe_size = ui
            .window(PROBE_WIN_ID)
            .pos(Position::Physical(PROBE_POS))
            .width(Size::Physical(PROBE_WIN_W))
            .clamp(WindowClamp::Locked)
            .show(|w| {
                w.add(DragProbe {
                    id: "cover_probe",
                    starts: &mut self.starts,
                    drag_frames: &mut self.drag_frames,
                    covered: &mut self.covered_drags,
                });
            });
        // ② 移动窗**后录**：位置由脚本切换（"本帧才移过去"）。
        let mover_pos = if over { MOVER_OVER } else { MOVER_AWAY };
        let mover_size = ui
            .window(MOVER_ID)
            .pos(Position::Physical(mover_pos))
            .width(Size::Physical(MOVER_W))
            .clamp(WindowClamp::Locked)
            .show(|w| {
                w.label("移动窗（后录制；默认 z 低于探针窗）");
                w.divider();
                w.label("这一块空白被探针窗压着——点击会落到探针上");
                w.label("本窗移到探针上 / z 被抬高后，探针就不该再收到按下");
            });
        // 脚本用坐标（与绘制同源解算；由本帧结算尺寸算出，无写死像素）。
        let probe_c = Vec2::new(
            PROBE_POS.x + probe_size.x * 0.5,
            PROBE_POS.y + probe_size.y * 0.5,
        );
        self.probe_point = probe_c;
        // **确实盖住** = 几何覆盖 **且** 画在探针窗之上（后者才是"该挡住"的判据）。
        let mover_z = ui.state().window_z.get(MOVER_ID).copied().unwrap_or(0);
        let probe_z = ui.state().window_z.get(PROBE_WIN_ID).copied().unwrap_or(0);
        self.covers = Rect::new(mover_pos.x, mover_pos.y, mover_size.x, mover_size.y)
            .contains_point(probe_c)
            && mover_z > probe_z;
    }
}

/// 固定尺寸的**拖拽探针**（`WidgetState` 跨帧 + `update_drag` + 自绘）。
struct DragProbe<'a> {
    id: &'a str,
    starts: &'a mut u32,
    drag_frames: &'a mut u32,
    covered: &'a mut u32,
}

impl Widget for DragProbe<'_> {
    fn size(&self, _ui: &mut Ui) -> Vec2 {
        Vec2::new(PROBE_W, PROBE_H)
    }

    fn ui(self, ui: &mut Ui, rect: Rect) -> Response {
        let abs = ui.id_for(self.id);
        let hit = ui.hit_abs(&abs, &rect);
        let btn = ui.mouse_left();
        // ① **鼠标就在本控件上、却拿不到命中** = 被更高 z 的窗口盖住；此时**还在拖** ⇒
        //    上一帧错误认领的按下一直拖到释放（用户看到的"被遮挡的控件仍被触发"）。
        //    ⚠ 必须带上"鼠标仍在 rect 内"：拖拽中鼠标移出矩形（或已松开）时 `hit` 也是 false，
        //    那是**正常拖拽行为**，不算被遮挡。
        let mouse_on_me = rect.contains_point(ui.mouse_local());
        if ui.state_mut().widget(&abs).dragging && !hit && mouse_on_me {
            *self.covered += 1;
        }
        // ② 认领按下（`down_edge` + 命中）——被盖住时不该发生（信息性计数）。
        if btn.down_edge() && hit {
            *self.starts += 1;
        }
        // ③ 拖拽状态机。
        let dragging = {
            let ws = ui.state_mut().widget(&abs);
            update_drag(ws, hit, btn)
        };
        // ④ **正对照**：拖拽活着的帧数（没被盖住时按下并按住 ⇒ 应当一直活着）。
        if dragging {
            *self.drag_frames += 1;
        }
        if std::env::var_os("RJ_COVER_TRACE").is_some() {
            eprintln!(
                "probe f={} hit={hit} down_edge={} dragging={dragging} under_mouse={:?} starts={} drag={} covered={}",
                ui.state().frame,
                btn.down_edge(),
                ui.window_under_mouse(),
                *self.starts,
                *self.drag_frames,
                *self.covered
            );
        }
        let bg = if dragging { BLUE } else { GREY };
        ui.push_panel_like(
            rect,
            bg,
            Color::rgba_u8(20, 24, 32, 255),
            1.0,
            CornerRadius::all(6.0),
            ui.elem_hint(),
        );
        ui.push_text_rect(
            rect,
            "拖拽探针（被盖住时不该收到按下）",
            12.0,
            Color::rgba_u8(255, 255, 255, 255),
            None,
            TextAlign::Center,
            TextVAlign::Center,
            None,
            None,
        );
        Response { hovered: hit, pressed: dragging, ..Default::default() }
    }
}
