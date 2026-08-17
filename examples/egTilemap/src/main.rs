//! egTilemap —— `rjw_tilemap` 演示：**任意图集区域贴片**（GameMaker 风格网格是特例）。
//!
//! - 运行时向 `DynamicAtlas` 插入程序生成纹理 → 得到 `AtlasRegion` → 任意尺寸/位置贴片；
//! - 贴片带翻转变换（`Transform2D`）、solid 碰撞标记；
//! - **C 键**切换视口剔除（开启后只提交可见 tile，界面显示 visible / total）；
//! - WASD 移动玩家方块（`rjw_collision::move_and_collide` 对 solid 贴片做滑动碰撞）；
//! - 方向键移动相机（视口随之移动，演示剔除）。

use glam::Vec2;
use rjw_2d_render::{ClearConfig, Render2D};
use rjw_atlas::{AtlasConfig, AtlasRegion, DynamicAtlas};
use rjw_collision::move_and_collide;
use rjw_color::Color;
use rjw_main::*;
use rjw_render::{RenderConfig, RenderContext, wgpu};
use rjw_text::{Align, Text};
use rjw_tilemap::{Tile, TileMap};
use rjw_transform::{Camera2D, Rect, Transform2D};

const TILE: f32 = 64.0;
const GRID_W: i32 = 22;
const GRID_H: i32 = 14;

struct TilemapDemo {
    render: Option<RenderContext>,
    render2d: Option<Render2D>,
    font: Option<Text>,
    cam: Camera2D,
    map: TileMap,
    player_pos: Vec2,
    player_size: Vec2,
    culling: bool,
    /// 动态图集必须存活（持有 GPU 资源）。
    _atlas: Option<DynamicAtlas>,
}

/// 生成一个带边框/纹理的 RGBA 瓦片（w×h）。
fn make_tile_px(rgb: [u8; 3], accent: [u8; 3], w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let border = x < 2 || y < 2 || x >= w - 2 || y >= h - 2;
            // 简单棋盘点缀
            let checker = ((x / 8 + y / 8) % 2) == 0;
            let c = if border {
                accent
            } else if checker {
                [rgb[0] / 2, rgb[1] / 2, rgb[2] / 2]
            } else {
                rgb
            };
            px.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    px
}

impl TilemapDemo {
    fn build_map(atlas: &mut DynamicAtlas) -> TileMap {
        let insert = |atlas: &mut DynamicAtlas, name: &str, rgb: [u8; 3], accent: [u8; 3]| {
            let px = make_tile_px(rgb, accent, TILE as u32, TILE as u32);
            atlas.insert(name.to_owned(), &px, TILE as u32, TILE as u32, (0, 0), false)
                .expect("tile texture insert")
        };
        let grass = insert(atlas, "grass", [96, 168, 84], [46, 96, 46]);
        let stone = insert(atlas, "stone", [150, 150, 158], [90, 90, 100]);
        let water = insert(atlas, "water", [70, 120, 200], [30, 70, 140]);
        let accent = insert(atlas, "accent", [232, 150, 60], [160, 90, 20]);

        let mut map = TileMap::new();
        let push = |map: &mut TileMap, region: AtlasRegion, gx: i32, gy: i32, solid: bool, flip_x: bool| {
            let mut t = Tile::new(region, Vec2::new(gx as f32 * TILE, gy as f32 * TILE));
            t.solid = solid;
            if flip_x {
                // 绕贴片原位水平翻转：scale.x=-1 时平移需补偿 +size.x，使 quad 仍落在 [pos, pos+size]。
                t.transform = Some(
                    Transform2D::IDENTITY
                        .with_pos(t.pos + Vec2::new(TILE, 0.0))
                        .with_scale(Vec2::new(-1.0, 1.0)),
                );
            }
            map.push(t);
        };
        for gy in 0..GRID_H {
            for gx in 0..GRID_W {
                let border = gx == 0 || gy == 0 || gx == GRID_W - 1 || gy == GRID_H - 1;
                let (region, solid) = if border {
                    (stone, true)
                } else if (gx + gy) % 7 == 0 {
                    (accent, false)
                } else if (gx + gy) % 5 == 0 {
                    (stone, true) // 内部石柱（碰撞）
                } else if (gx + gy) % 11 == 0 {
                    (water, false)
                } else {
                    (grass, false)
                };
                push(&mut map, region, gx, gy, solid, (gx + gy) % 9 == 0);
            }
        }
        map
    }
}

impl App for TilemapDemo {
    fn primary_window_attrib(&self) -> WindowAttributes {
        WindowAttributes::default()
            .with_title("egTilemap - rjw_tilemap 任意图集贴片")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
    }

    fn on_init(&mut self, ctx: &mut MainContext) {
        eprintln!("MARK: on_init");
        let window = ctx.primary_window().expect("primary window must exist during on_init");
        self.render = Some(RenderContext::new(window, &RenderConfig::default()));
        let render = self.render.as_ref().expect("render must be initialized");

        let render2d = Render2D::new(render);
        let (w, h) = render.size();
        self.cam = Camera2D::new(Vec2::new(w as f32, h as f32));

        // 动态图集：运行时插入程序生成的瓦片纹理（任意尺寸/位置裁切贴片）。
        let mut atlas = DynamicAtlas::new(
            render2d.device(),
            render2d.queue(),
            render2d.tex_bind_group_layout(),
            AtlasConfig { max_pages: 4, padding: 1, ..Default::default() },
            1024,
        );
        self.map = Self::build_map(&mut atlas);
        self._atlas = Some(atlas);

        self.font = Some(Text::new(
            render2d.device(),
            render2d.queue(),
            render2d.tex_bind_group_layout(),
        ));
        self.render2d = Some(render2d);
    }

    fn on_resized(&mut self, _ctx: &mut MainContext, width: u32, height: u32) {
        if let Some(render) = &mut self.render {
            render.resize(width, height);
        }
        // 更新正交投影尺寸（世界 (0,0) = 视口中心；全窗口相机，viewport_pos 保持 ZERO）。
        self.cam.set_vp(Vec2::new(width as f32, height as f32), Vec2::ZERO);
    }

    fn about_to_wait(&mut self, ctx: &mut MainContext) {
        if ctx.keyboard.get(KeyCode::Escape).down_edge() {
            ctx.request_exit();
        }
        let dt = ctx.timer.dt().get_f32();
        let kb = &ctx.keyboard;

        // C：切换视口剔除
        if kb.get(KeyCode::KeyC).down_edge() {
            self.culling = !self.culling;
            let vp = self.viewport_rect();
            eprintln!(
                "culling: {}  visible {}/{}",
                self.culling,
                self.map.visible_count(Some(vp)),
                self.map.tile_count()
            );
        }

        // 方向键：移动相机（Camera2D::move_by 直接改 position，vp_matrix 才会反映）
        let cam_speed = 700.0;
        let mut cam_move = Vec2::ZERO;
        if kb.get(KeyCode::ArrowLeft).pressed() { cam_move.x -= cam_speed * dt; }
        if kb.get(KeyCode::ArrowRight).pressed() { cam_move.x += cam_speed * dt; }
        if kb.get(KeyCode::ArrowUp).pressed() { cam_move.y -= cam_speed * dt; }
        if kb.get(KeyCode::ArrowDown).pressed() { cam_move.y += cam_speed * dt; }
        self.cam.move_by(cam_move);

        // WASD：玩家移动（对 solid 贴片做滑动碰撞）
        let speed = 340.0;
        let mut vel = Vec2::ZERO;
        if kb.get(KeyCode::KeyW).pressed() { vel.y -= speed; }
        if kb.get(KeyCode::KeyS).pressed() { vel.y += speed; }
        if kb.get(KeyCode::KeyA).pressed() { vel.x -= speed; }
        if kb.get(KeyCode::KeyD).pressed() { vel.x += speed; }
        let solids = self.map.solid_rects();
        self.player_pos = move_and_collide(self.player_pos, self.player_size, vel, dt, &solids);

        // ── 渲染 ──
        // vp_matrix = P × V（P 以视口中心为原点，V 平移 -position）；set_vp 仅在 resize 时更新。
        let vp = if self.culling { Some(self.viewport_rect()) } else { None };
        let Some(r2d) = &mut self.render2d else { return };
        let Some(font) = &mut self.font else { return };
        r2d.set_mvp(self.cam.vp_matrix());

        self.map.draw(r2d, 0.0, vp);

        // 玩家方块
        r2d.add_sprite2d_solid(
            rjw_2d_render::SpriteRect::from_texture(self.player_pos, self.player_size),
            Color::WHITE,
            Transform2D::default(),
            50.0,
        );

        // HUD（锚定相机：世界坐标 = cam.position + 屏幕偏移，保证固定屏幕左上角）
        let half = self.cam.viewport_size * 0.5;
        let hud = format!(
            "C: toggle culling ({})  visible {}/{} | WASD move · Arrows camera · Esc quit",
            if self.culling { "ON" } else { "OFF" },
            self.map.visible_count(vp),
            self.map.tile_count(),
        );
        font.draw_label(
            r2d, &hud, Color::YELLOW,
            18.0, 26.0,
            self.cam.position + Vec2::new(-half.x + 14.0, -half.y + 14.0),
            "SimHei", Align::Left, 100.0,
        );

        let clear = ClearConfig {
            color: Some(wgpu::Color { r: 0.08, g: 0.09, b: 0.12, a: 1.0 }),
            depth: None,
            stencil: None,
        };
        r2d.render(&clear);
    }
}

impl TilemapDemo {
    /// 视口世界矩形：世界 (0,0) = 视口中心，故 = position ± viewport/2。
    fn viewport_rect(&self) -> Rect {
        let half = self.cam.viewport_size * 0.5;
        Rect::new(
            self.cam.position.x - half.x,
            self.cam.position.y - half.y,
            self.cam.viewport_size.x,
            self.cam.viewport_size.y,
        )
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    rjw_main::run_app(TilemapDemo {
        render: None,
        render2d: None,
        font: None,
        cam: Camera2D::new(Vec2::new(1280.0, 720.0)),
        map: TileMap::new(),
        player_pos: Vec2::new(TILE * 4.0, TILE * 4.0),
        player_size: Vec2::new(48.0, 48.0),
        culling: false,
        _atlas: None,
    })
}
