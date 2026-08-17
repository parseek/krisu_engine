//! egTilemap —— `rjw_tilemap` v2 演示：任意图集区域贴片 + 物件化 + Chunk+AABB 剔除。
//!
//! - 运行时向 `DynamicAtlas` 插入程序生成纹理 → `RegionRef` 句柄（重排后仍可用、保活）→ 任意尺寸贴片；
//! - tile 仅位移+缩放：**负 size 翻转**（负宽 = 水平镜像）；
//! - **M 键**旋转整个地图（TileMap 整体变换，物件化）；
//! - **C 键**切换相机剔除（`TileMap::draw` 直接收 `&Camera2D`，内部用 `view_aabb` 世界保守 AABB
//!   → 局部空间 chunk 粗剔 + tile 精剔；HUD 显示 visible / total）；
//! - **Q/E** 旋转相机、**R/F** 缩放相机（演示旋转/缩放下剔除仍保守正确）；
//! - WASD 移动玩家（`rjw_collision::move_and_collide` 对 solid 贴片滑动碰撞）；方向键移动相机。

use glam::Vec2;
use rjw_2d_render::{ClearConfig, Render2D};
use rjw_atlas::{AtlasConfig, DynamicAtlas, RegionRef};
use rjw_collision::move_and_collide;
use rjw_color::Color;
use rjw_main::*;
use rjw_render::{RenderConfig, RenderContext, wgpu};
use rjw_text::{Align, Text};
use rjw_tilemap::{Tile, TileMap};
use rjw_transform::{Camera2D, Transform2D};

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
                .expect("tile texture insert");
            // RegionRef 句柄：保活 + 重排后 resolve 最新 UV
            atlas.acquire(&name.to_owned()).expect("acquire after insert")
        };
        let grass = insert(atlas, "grass", [96, 168, 84], [46, 96, 46]);
        let stone = insert(atlas, "stone", [150, 150, 158], [90, 90, 100]);
        let water = insert(atlas, "water", [70, 120, 200], [30, 70, 140]);
        let accent = insert(atlas, "accent", [232, 150, 60], [160, 90, 20]);

        let mut map = TileMap::new(512.0);
        let push = |map: &mut TileMap, region: &RegionRef, gx: i32, gy: i32, solid: bool, flip_x: bool| {
            let mut t = Tile::new(
                region.clone(),
                Vec2::new(gx as f32 * TILE, gy as f32 * TILE),
                Vec2::new(if flip_x { -TILE } else { TILE }, TILE), // 负宽 = 水平镜像
            );
            t.solid = solid;
            map.push(t);
        };
        for gy in 0..GRID_H {
            for gx in 0..GRID_W {
                let border = gx == 0 || gy == 0 || gx == GRID_W - 1 || gy == GRID_H - 1;
                let (region, solid) = if border {
                    (&stone, true)
                } else if (gx + gy) % 7 == 0 {
                    (&accent, false)
                } else if (gx + gy) % 5 == 0 {
                    (&stone, true) // 内部石柱（碰撞）
                } else if (gx + gy) % 11 == 0 {
                    (&water, false)
                } else {
                    (&grass, false)
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
            .with_title("egTilemap - rjw_tilemap 任意图集贴片 v2")
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

        // 动态图集：运行时插入程序生成的瓦片纹理。
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
        // 世界 (0,0) = 视口中心；全窗口相机，viewport_pos 保持 ZERO。
        self.cam.set_vp(Vec2::new(width as f32, height as f32), Vec2::ZERO);
    }

    fn about_to_wait(&mut self, ctx: &mut MainContext) {
        if ctx.keyboard.get(KeyCode::Escape).down_edge() {
            ctx.request_exit();
        }
        let dt = ctx.timer.dt().get_f32();
        let kb = &ctx.keyboard;

        // C：切换剔除（TileMap::draw 直接收 &Camera2D）
        if kb.get(KeyCode::KeyC).down_edge() {
            self.culling = !self.culling;
            eprintln!(
                "culling: {}  visible {}/{}  chunks {}",
                self.culling,
                self.map.visible_count(self.culling.then_some(&self.cam)),
                self.map.tile_count(),
                self.map.chunk_count(),
            );
        }

        // Q/E：旋转相机；R/F：缩放相机
        if kb.get(KeyCode::KeyQ).pressed() { self.cam.rotation -= 1.2 * dt; }
        if kb.get(KeyCode::KeyE).pressed() { self.cam.rotation += 1.2 * dt; }
        if kb.get(KeyCode::KeyR).pressed() { self.cam.zoom = (self.cam.zoom * 1.05).min(Vec2::splat(4.0)); }
        if kb.get(KeyCode::KeyF).pressed() { self.cam.zoom = (self.cam.zoom * (1.0 / 1.05)).max(Vec2::splat(0.25)); }

        // M：旋转整个地图（物件化整体变换）
        if kb.get(KeyCode::KeyM).pressed() {
            let rot = self.map.transform().map(|t| t.rotation).unwrap_or(0.0);
            let center = Vec2::new(GRID_W as f32 * TILE * 0.5, GRID_H as f32 * TILE * 0.5);
            self.map.set_transform(Transform2D::IDENTITY
                .with_pos(center)
                .with_rot(rot + 0.8 * dt)
                .with_move_by(-center));
        }

        // 方向键：移动相机
        let cam_speed = 700.0;
        let mut cam_move = Vec2::ZERO;
        if kb.get(KeyCode::ArrowLeft).pressed() { cam_move.x -= cam_speed * dt; }
        if kb.get(KeyCode::ArrowRight).pressed() { cam_move.x += cam_speed * dt; }
        if kb.get(KeyCode::ArrowUp).pressed() { cam_move.y -= cam_speed * dt; }
        if kb.get(KeyCode::ArrowDown).pressed() { cam_move.y += cam_speed * dt; }
        self.cam.move_by(cam_move);

        // WASD：玩家移动（对 solid 贴片做滑动碰撞；solid_rects 为脏标记缓存，静态地图零开销）
        let speed = 340.0;
        let mut vel = Vec2::ZERO;
        if kb.get(KeyCode::KeyW).pressed() { vel.y -= speed; }
        if kb.get(KeyCode::KeyS).pressed() { vel.y += speed; }
        if kb.get(KeyCode::KeyA).pressed() { vel.x -= speed; }
        if kb.get(KeyCode::KeyD).pressed() { vel.x += speed; }
        let solids = self.map.solid_rects();
        self.player_pos = move_and_collide(self.player_pos, self.player_size, vel, dt, solids);

        // ── 渲染 ──
        let Some(r2d) = &mut self.render2d else { return };
        let Some(font) = &mut self.font else { return };
        let atlas = self._atlas.as_ref().expect("atlas initialized");
        r2d.set_mvp(self.cam.vp_matrix());

        let cam_ref = if self.culling { Some(&self.cam) } else { None };
        self.map.draw(r2d, atlas, 0.0, cam_ref);

        // 玩家方块（地图旋转时玩家仍在世界坐标移动）
        r2d.add_sprite2d_solid(
            rjw_2d_render::SpriteRect::from_texture(self.player_pos, self.player_size),
            Color::WHITE,
            Transform2D::default(),
            50.0,
        );

        // HUD（锚定相机：世界坐标 = cam.position + 屏幕偏移，固定屏幕左上角）
        let half = self.cam.view_half_size();
        let hud = format!(
            "C: cull {} · Q/E cam rot {:.0}° · R/F zoom {:.2} · M: map rot {:.0}° | visible {}/{} chunks {} | WASD move · Arrows cam",
            if self.culling { "ON" } else { "OFF" },
            self.cam.rotation.to_degrees(),
            self.cam.zoom.x,
            self.map.transform().map(|t| t.rotation.to_degrees()).unwrap_or(0.0),
            self.map.visible_count(cam_ref),
            self.map.tile_count(),
            self.map.chunk_count(),
        );
        font.draw_label(
            r2d, &hud, Color::YELLOW,
            16.0, 24.0,
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

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    rjw_main::run_app(TilemapDemo {
        render: None,
        render2d: None,
        font: None,
        cam: Camera2D::new(Vec2::new(1280.0, 720.0)),
        map: TileMap::new(512.0),
        player_pos: Vec2::new(TILE * 4.0, TILE * 4.0),
        player_size: Vec2::new(48.0, 48.0),
        culling: false,
        _atlas: None,
    })
}
