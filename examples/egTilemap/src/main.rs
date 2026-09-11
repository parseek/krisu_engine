//! egTilemap —— `rjw_tilemap` v4 演示：源裁剪贴片 + Chunk 预生成顶点 + 单一剔除语言。
//!
//! - 运行时向 `DynamicAtlas` 插入程序生成纹理 → `RegionRef` 句柄（重排后仍可用、保活）→
//!   `Tile { src, src_tl/src_wh（源内裁剪）, mesh_tl/mesh_wh（目标，负 = 翻转）}`；
//! - **M 键**旋转整个地图（TileMap 整体变换，物件化；变换在绘制期应用，不重建顶点）；
//! - **C 键**切换剔除：只设置一次 `r2d.cull(Cull::from(&cam))`——它同时驱动
//!   `Render2D` 的命令级剔除（Sprite）与 `TileMap::draw` 的 chunk 级粗剔（不再传闭包）；
//! - **Q/E** 旋转相机、**R/F** 缩放相机（旋转/缩放下剔除仍保守正确）；
//! - WASD 移动玩家（`rjw_krusie::collision::Aabb::slide` 对 solid 贴片滑动碰撞）；方向键移动相机。
//!
//! 新驱动（`docs/API_DESIGN.md`）：`App::{config, init, update}` + `Ctx::frame()` 守卫 +
//! `Frame::draw()` / `Frame::text(..)` / `Frame::submit(&mut cam, Clear::..)`；
//! 渲染上下文 / 帧 / present 全部由运行时接管。

use rjw_krusie::collision::Aabb;
use rjw_krusie::prelude::*;

const TILE: f32 = 64.0;
const GRID_W: i32 = 22;
const GRID_H: i32 = 14;

struct TilemapDemo {
    cam: Camera2D,
    map: TileMap,
    player_pos: Vec2,
    player_size: Vec2,
    culling: bool,
    /// 窗口 DPI scale factor（物理/逻辑像素）。
    scale_factor: f32,
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
            // `no_clamp`：瓦片 UV 精确贴合，不要 1px 外扩
            atlas
                .insert_with(
                    name.to_owned(),
                    Rgba8::new(&px, (TILE as u32, TILE as u32)),
                    InsertOpts::new().no_clamp(),
                )
                .expect("tile texture insert");
            // RegionRef 句柄：保活 + 重排后 resolve 最新 UV
            atlas.handle(&name.to_owned()).expect("handle after insert")
        };
        let grass = insert(atlas, "grass", [96, 168, 84], [46, 96, 46]);
        let stone = insert(atlas, "stone", [150, 150, 158], [90, 90, 100]);
        let water = insert(atlas, "water", [70, 120, 200], [30, 70, 140]);
        let accent = insert(atlas, "accent", [232, 150, 60], [160, 90, 20]);

        let mut map = TileMap::new(1024.0);
        let push = |map: &mut TileMap, src: &RegionRef, gx: i32, gy: i32, solid: bool, flip_x: bool| {
            // `Tile::new(&handle, ..)` = 整张精灵（`src` 句柄被克隆保活）；链式只写差异。
            let t = Tile::new(
                src,
                Vec2::new(gx as f32 * TILE, gy as f32 * TILE),
                Vec2::new(if flip_x { -TILE } else { TILE }, TILE), // 负宽 = 水平镜像
            )
            .solid(solid);
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
    fn config(&self) -> AppConfig {
        AppConfig::new("egTilemap - rjw_tilemap 任意图集贴片 v2").size(1280.0, 720.0)
    }

    fn init(&mut self, gfx: &Gfx) {
        eprintln!("MARK: on_init");

        // 动态图集：运行时插入程序生成的瓦片纹理。
        let mut atlas = DynamicAtlas::new(
            gfx,
            AtlasConfig { max_pages: 4, padding: 1, page_size: 1024, ..Default::default() },
        );
        self.map = Self::build_map(&mut atlas);
        self._atlas = Some(atlas);
    }

    fn update(&mut self, ctx: &mut Ctx) {
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }
        let dt = ctx.dt();
        // DPI scale factor：`init` 只有 `Gfx`（拿不到窗口），每帧从宿主事实刷新。
        self.scale_factor = ctx.scale();

        // C：切换剔除（一个开关驱动两处：命令级剔除 + chunk 级粗剔）
        if ctx.key(KeyCode::KeyC).down_edge() {
            self.culling = !self.culling;
            eprintln!(
                "culling: {}  tiles {}  chunks {}",
                self.culling,
                self.map.tile_count(),
                self.map.chunk_count(),
            );
        }

        // Q/E：旋转相机；R/F：缩放相机
        if ctx.key(KeyCode::KeyQ).pressed() { self.cam.transform.rotation -= 1.2 * dt; }
        if ctx.key(KeyCode::KeyE).pressed() { self.cam.transform.rotation += 1.2 * dt; }
        if ctx.key(KeyCode::KeyR).pressed() { self.cam.set_zoom((self.cam.zoom() * 1.05).min(Vec2::splat(4.0))); }
        if ctx.key(KeyCode::KeyF).pressed() { self.cam.set_zoom((self.cam.zoom() * (1.0 / 1.05)).max(Vec2::splat(0.25))); }

        // M：旋转整个地图（物件化整体变换）
        if ctx.key(KeyCode::KeyM).pressed() {
            let rot = self.map.transform().rotation;
            let center = Vec2::new(GRID_W as f32 * TILE * 0.5, GRID_H as f32 * TILE * 0.5);
            // 旧 `with_move_by(-center)` 已在重设计中删除，用同义的 `move_by` 就地表达。
            let mut t = Transform2D::IDENTITY.with_pos(center).with_rot(rot + 0.8 * dt);
            t.move_by(-center);
            self.map.set_transform(t);
        }

        // 方向键：沿**相机朝向**移动（`move_local` 把位移旋转到相机系；相机旋转后方向键仍符合直觉）
        let cam_speed = 700.0;
        let mut cam_walk = Vec2::ZERO;
        if ctx.key(KeyCode::ArrowLeft).pressed() { cam_walk.x -= cam_speed * dt; }  // 相机左
        if ctx.key(KeyCode::ArrowRight).pressed() { cam_walk.x += cam_speed * dt; } // 相机右
        if ctx.key(KeyCode::ArrowUp).pressed() { cam_walk.y -= cam_speed * dt; }    // 相机前
        if ctx.key(KeyCode::ArrowDown).pressed() { cam_walk.y += cam_speed * dt; }  // 相机后
        self.cam.move_local(cam_walk);

        // WASD：玩家移动（对 solid 贴片做滑动碰撞；`solid_rects` 是脏标记缓存，静态地图零开销）
        let speed = 340.0;
        let mut vel = Vec2::ZERO;
        if ctx.key(KeyCode::KeyW).pressed() { vel.y -= speed; }
        if ctx.key(KeyCode::KeyS).pressed() { vel.y += speed; }
        if ctx.key(KeyCode::KeyA).pressed() { vel.x -= speed; }
        if ctx.key(KeyCode::KeyD).pressed() { vel.x += speed; }
        // 玩家碰撞体：`rect` = 世界矩形（左上角 = 当前 player_pos），`transform` = 单位变换。
        // `slide` 施加速度位移（先 X 后 Y 的分离轴回退 + 沿障碍滑动），返回新的世界左上角。
        // `solid_rects()` 只收 `&self`（内部缓存）；其返回值是借用，用完即放（作用域内）。
        self.player_pos = {
            let solids = self.map.solid_rects();
            let mut body = Aabb::at(
                Rect::new(self.player_pos.x, self.player_pos.y, self.player_size.x, self.player_size.y),
                Transform2D::IDENTITY,
            );
            body.slide(vel * dt, &solids)
        };

        // ── 渲染：取不到表面则一行都不执行 ──
        let Some(mut f) = ctx.frame() else { return };
        // 相机自己存画面矩形（HUD 的 screen_to_world / 剔除的 view_aabb 都依赖它）。
        self.cam.set_region(f.region());

        let atlas = self._atlas.as_ref().expect("atlas initialized");

        {
            let r2d = f.draw();
            // 一处设置、两处生效：命令级剔除（Sprite）+ `TileMap::draw` 的 chunk 级粗剔。
            r2d.cull(if self.culling { Cull::from(&self.cam) } else { Cull::Off });

            self.map.draw(r2d, atlas, 0.0);

            // 玩家方块（地图旋转时玩家仍在世界坐标移动）
            r2d.solid(SpriteRect::new(self.player_pos, self.player_size))
                .tint(Color::WHITE)
                .layer(50.0);
        }

        // HUD（UI 文本，屏幕固定——内联实现，不依赖 rjw_text 扩展）：
        // - 位置：anchor = cam.screen_to_world(屏幕像素)，随相机旋转/缩放仍是屏幕左上角；
        // - 缩放：transform scale = 1/zoom → 屏幕字形大小 = size × zoom × (1/zoom) = size；
        // - 旋转：屏幕→世界逆变换带 R(+rotation)（世界→屏幕是 zoom·R(-rotation)·(w-pos)），
        //   transform rotation = +cam.rotation → 文字方向抵消相机旋转（屏幕对齐）。
        //   注意：若希望 HUD 跟随世界旋转（倾斜），去掉 with_rot(…) 即可。
        // - 字号/锚点按 scale_factor 换算为物理像素。
        let sf = self.scale_factor;
        let hud = format!(
            "C: cull {} · Q/E cam rot {:.0}° · R/F zoom {:.2} · M: map rot {:.0}° | {} tiles / {} chunks | WASD move · Arrows cam",
            if self.culling { "ON" } else { "OFF" },
            self.cam.transform.rotation.to_degrees(),
            self.cam.zoom().x,
            self.map.transform().rotation.to_degrees(),
            self.map.tile_count(),
            self.map.chunk_count(),
        );
        let anchor = self.cam.screen_to_world(Vec2::new(14.0 * sf, 14.0 * sf));
        // 唯一文本链：`TextCtx::label(..)` 已绑定世界层 Render2D ⇒ `draw(layer)` 1 参。
        // 旧 `.into_render().transform(..).color(..)` 机械适配：transform / color 同名同义；
        // 旧默认 origin/offset 均为零 ⇒ 新链默认定位（`at`/`anchor`/`offset` 全 0）。
        f.text(|t| {
            t.label(hud.as_str())
                .size(16.0 * sf)
                .align(Align::Left)
                .transform(
                    Transform2D::IDENTITY
                        .with_pos(anchor)
                        .with_rot(self.cam.transform.rotation)
                        .with_scale(Vec2::new(1.0 / self.cam.zoom().x, 1.0 / self.cam.zoom().y)),
                )
                .color(Color::YELLOW)
                .draw(100.0);
        });

        // 一个画面 = 一次 submit(相机, clear)：开 pass / 写画面矩形 / present 由运行时接管。
        f.submit(&mut self.cam, Clear::color(Color::rgb(0.08, 0.09, 0.12)));
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    // 可选启动参数：--cam-rot <弧度> 设置初始相机旋转（RenderDoc 验证屏幕固定文本用）。
    let mut cam = Camera2D::full(Vec2::new(1280.0, 720.0));
    let mut args = std::env::args();
    while let Some(a) = args.next() {
        if a == "--cam-rot"
            && let Some(v) = args.next() {
                cam.transform.rotation = v.parse().unwrap_or(0.0);
            }
    }
    if cam.transform.rotation != 0.0 {
        eprintln!("initial cam rotation = {:.3} rad", cam.transform.rotation);
    }
    run(TilemapDemo {
        cam,
        map: TileMap::new(1024.0),
        player_pos: Vec2::new(TILE * 4.0, TILE * 4.0),
        player_size: Vec2::new(48.0, 48.0),
        culling: false,
        scale_factor: 1.0,
        _atlas: None,
    })
}
