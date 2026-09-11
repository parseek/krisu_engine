//! eg260731RPG —— 小型 2D 顶视角 RPG 范例
//!
//! 玩法：
//! - WASD / 方向键：移动（角色自动面向移动/鼠标方向）
//! - 鼠标左键 / 空格：挥砍攻击（面向方向扇形，击杀史莱姆得金币）
//! - 史莱姆会追踪并撞击玩家造成伤害
//! - HP 归零 → 游戏结束，按 R 重新开始，Esc 退出
//!
//! 展示的引擎能力：DynamicAtlas 图集 / Guillotine 空闲矩形打包 / clamp_margin / 合批优化 / RStates 责任链。

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use rjw_krusie::gpu::{MeshData, TEXTURES, wgpu};
use rjw_krusie::prelude::*;
use rjw_krusie::render2d::VertexP3U2C4;

// ── 常量 ─────────────────────────────────────────────────────────
const TILE: f32 = 32.0;
const MAP_W: usize = 64;
const MAP_H: usize = 64;
const PLAYER_RADIUS: f32 = 13.0;
const ENEMY_RADIUS: f32 = 15.0;
const PLAYER_SPEED: f32 = 210.0;
const ENEMY_SPEED: f32 = 55.0;
const WAVE_BREAK: f32 = 1.6;
const WAVE_BASE_COUNT: usize = 4;
const WAVE_PER: usize = 2;
const WAVE_MAX: usize = 20;
const WAVE_HEAL: i32 = 1;
const WAVE_BONUS_COINS: i32 = 2;
const SLASH_RANGE: f32 = 74.0;
const SLASH_HALF_ANGLE: f32 = 38.0_f32.to_radians();
const SLASH_DURATION: f32 = 0.2;
const MAX_HP: i32 = 5;

const LAYER_GROUND: f32 = 0.0;
const LAYER_TERRAIN: f32 = 1.0;
const LAYER_Y_SORT_BASE: f32 = 10.0;
/// 世界层「效果」层：远高于 y-sort（`LAYER_Y_SORT_BASE + foot_y`），把攻击弧等效果
/// 压在实体之上（仍是**世界层**内容，随相机移动）。
const LAYER_EFFECT: f32 = 1000000.0;

// 屏幕固定 UI 走 **UI 层**（`f.draw_ui()` / `f.text_ui()`）：UI 层按**录制顺序**提交
// （`SortMode::None`），所以那里的小 layer 只用于同层内微调，与世界的 `LAYER_*` 无关。

#[inline]
fn y_layer(foot_y: f32) -> f32 {
    LAYER_Y_SORT_BASE + foot_y
}

// ── 简单随机数 ────────────────────────────────────────────────────
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next() % (hi - lo) as u64) as usize
    }
}

// ── 地图 ──────────────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq)]
enum Tile {
    Grass,
    Field,
    Sand,
    Water,
    Tree,
    Stone,
    Flower,
}
impl Tile {
    fn is_blocked(self) -> bool {
        matches!(self, Tile::Water | Tile::Tree | Tile::Stone)
    }
}

struct Map {
    tiles: Vec<Tile>,
}
impl Map {
    fn tile_at(&self, tx: i32, ty: i32) -> Option<Tile> {
        if tx < 0 || ty < 0 || tx as usize >= MAP_W || ty as usize >= MAP_H {
            None
        } else {
            Some(self.tiles[ty as usize * MAP_W + tx as usize])
        }
    }
    fn set_tile(&mut self, tx: i32, ty: i32, t: Tile) {
        if tx >= 0 && ty >= 0 && (tx as usize) < MAP_W && (ty as usize) < MAP_H {
            self.tiles[ty as usize * MAP_W + tx as usize] = t;
        }
    }
    fn collides(&self, min: Vec2, max: Vec2) -> bool {
        let tx0 = (min.x / TILE).floor() as i32;
        let ty0 = (min.y / TILE).floor() as i32;
        let tx1 = (max.x / TILE).floor() as i32;
        let ty1 = (max.y / TILE).floor() as i32;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if let Some(t) = self.tile_at(tx, ty)
                    && t.is_blocked() {
                        return true;
                    }
            }
        }
        false
    }
}

fn set_radius(map: &mut Map, rng: &mut Rng, cx: i32, cy: i32, r: i32, t: Tile) {
    for dy in -r..=r {
        for dx in -r..=r {
            let dist = (dx * dx + dy * dy) as f32;
            let wob = 1.0 + (rng.f32() - 0.5) * 0.7;
            if dist <= (r as f32 * wob) * (r as f32 * wob) {
                map.set_tile(cx + dx, cy + dy, t);
            }
        }
    }
}

fn generate_map() -> Map {
    let mut rng = Rng::new(0x9E37_79B9_7F4A_7C15);
    let mut map = Map {
        tiles: vec![Tile::Grass; MAP_W * MAP_H],
    };
    for _ in 0..7 {
        let cx = 3 + rng.range(0, MAP_W - 6) as i32;
        let cy = 3 + rng.range(0, MAP_H - 6) as i32;
        let r = 3 + rng.range(0, 5) as i32;
        set_radius(&mut map, &mut rng, cx, cy, r + 1, Tile::Sand);
        set_radius(&mut map, &mut rng, cx, cy, r, Tile::Water);
    }
    for _ in 0..6 {
        let cx = 3 + rng.range(0, MAP_W - 6) as i32;
        let cy = 3 + rng.range(0, MAP_H - 6) as i32;
        let r = 2 + rng.range(0, 4) as i32;
        set_radius(&mut map, &mut rng, cx, cy, r, Tile::Stone);
    }
    for _ in 0..9 {
        let cx = 2 + rng.range(0, MAP_W - 4) as i32;
        let cy = 2 + rng.range(0, MAP_H - 4) as i32;
        let r = 2 + rng.range(0, 4) as i32;
        set_radius(&mut map, &mut rng, cx, cy, r, Tile::Tree);
    }
    for _ in 0..5 {
        let x0 = 2 + rng.range(0, MAP_W - 2) as i32;
        let y0 = 2 + rng.range(0, MAP_H - 2) as i32;
        let w = 4 + rng.range(0, 5) as i32;
        let h = 4 + rng.range(0, 5) as i32;
        for dy in 0..h {
            for dx in 0..w {
                map.set_tile(x0 + dx, y0 + dy, Tile::Field);
            }
        }
    }
    for _ in 0..40 {
        map.set_tile(2 + rng.range(0, MAP_W - 4) as i32, 2 + rng.range(0, MAP_H - 4) as i32, Tile::Flower);
    }
    for x in 0..MAP_W as i32 {
        map.set_tile(x, 0, Tile::Tree);
        map.set_tile(x, MAP_H as i32 - 1, Tile::Tree);
    }
    for y in 0..MAP_H as i32 {
        map.set_tile(0, y, Tile::Tree);
        map.set_tile(MAP_W as i32 - 1, y, Tile::Tree);
    }
    let cx = (MAP_W / 2) as i32;
    let cy = (MAP_H / 2) as i32;
    for dy in -5..=5 {
        for dx in -5..=5 {
            map.set_tile(cx + dx, cy + dy, Tile::Grass);
        }
    }
    map
}

fn free_tiles(map: &Map) -> Vec<Vec2> {
    let mut out = Vec::new();
    for y in 0..MAP_H {
        for x in 0..MAP_W {
            if !map.tiles[y * MAP_W + x].is_blocked() {
                out.push(Vec2::new((x as f32 + 0.5) * TILE, (y as f32 + 0.5) * TILE));
            }
        }
    }
    out
}
fn center_of_map() -> Vec2 {
    Vec2::new(MAP_W as f32 * 0.5 * TILE, MAP_H as f32 * 0.5 * TILE)
}

// ── 实体 ──────────────────────────────────────────────────────────
struct Player {
    pos: Vec2,
    hp: i32,
    max_hp: i32,
    facing_angle: f32,
    attack_timer: f32,
    attack_cooldown: f32,
    flash_timer: f32,
}
impl Player {
    fn new(pos: Vec2) -> Self {
        Self {
            pos,
            hp: MAX_HP,
            max_hp: MAX_HP,
            facing_angle: 0.0,
            attack_timer: 0.0,
            attack_cooldown: 0.0,
            flash_timer: 0.0,
        }
    }
}

struct Enemy {
    pos: Vec2,
    hp: i32,
    speed: f32,
    attack_timer: f32,
    anim: f32,
    alive: bool,
}
impl Enemy {
    fn new(pos: Vec2, speed: f32) -> Self {
        Self {
            pos,
            hp: 3,
            speed,
            attack_timer: 0.0,
            anim: 0.0,
            alive: true,
        }
    }
}

struct Particle {
    pos: Vec2,
    vel: Vec2,
    life: f32,
    max_life: f32,
    size: f32,
    color: Color,
}

#[derive(Clone, Copy, PartialEq)]
enum GameState {
    Playing,
    GameOver,
}

/// 地图重开版本号（R 重开 → `Game::new()` → +1，StaticTerrain 据此重建）。
static MAP_REV: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct Game {
    map: Map,
    map_rev: u64,
    player: Player,
    enemies: Vec<Enemy>,
    particles: Vec<Particle>,
    state: GameState,
    coins: i32,
    kills: i32,
    wave: usize,
    wave_break_timer: f32,
    elapsed: f32,
}
impl Game {
    fn new() -> Self {
        let map = generate_map();
        let free = free_tiles(&map);
        let mut rng = Rng::new(0x0123_4567);
        let mut game = Self {
            map,
            map_rev: MAP_REV.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            player: Player::new(center_of_map()),
            enemies: Vec::new(),
            particles: Vec::new(),
            state: GameState::Playing,
            coins: 0,
            kills: 0,
            wave: 0,
            wave_break_timer: 0.0,
            elapsed: 0.0,
        };
        game.spawn_next_wave(&free, &mut rng);
        game
    }
    fn spawn_next_wave(&mut self, free: &[Vec2], rng: &mut Rng) {
        self.wave += 1;
        let count = (WAVE_BASE_COUNT + (self.wave - 1) * WAVE_PER).min(WAVE_MAX);
        let hp_bonus = (self.wave as f32 * 0.5) as i32;
        let speed_bonus = (self.wave as f32 - 1.0) * 3.0;
        for _ in 0..count {
            let p = free[rng.range(0, free.len())];
            let mut e = Enemy::new(p, (ENEMY_SPEED + speed_bonus + rng.f32() * 10.0).min(140.0));
            e.hp = (3 + hp_bonus).clamp(3, 9);
            self.enemies.push(e);
        }
        spawn_burst(&mut self.particles, self.player.pos, Color::rgba(0.8, 0.5, 1.0, 1.0), 10);
    }
}

// ── 逻辑更新 ──────────────────────────────────────────────────────
fn move_entity(pos: &mut Vec2, vel: Vec2, map: &Map, radius: f32, dt: f32) {
    let nx = pos.x + vel.x * dt;
    if !map.collides(Vec2::new(nx - radius, pos.y - radius), Vec2::new(nx + radius, pos.y + radius)) {
        pos.x = nx;
    }
    let ny = pos.y + vel.y * dt;
    if !map.collides(Vec2::new(pos.x - radius, ny - radius), Vec2::new(pos.x + radius, ny + radius)) {
        pos.y = ny;
    }
}

fn spawn_burst(particles: &mut Vec<Particle>, center: Vec2, color: Color, count: usize) {
    for i in 0..count {
        let a = i as f32 / count as f32 * TAU + (center.x * 0.11).sin() * 0.5;
        let speed = 70.0 + (i % 5) as f32 * 24.0;
        particles.push(Particle {
            pos: center,
            vel: Vec2::new(a.cos(), a.sin()) * speed,
            life: 0.5,
            max_life: 0.5,
            size: 3.5 + (i % 3) as f32 * 2.0,
            color,
        });
    }
}

fn update(game: &mut Game, cam: &Camera2D, ctx: &Ctx, dt: f32) {
    game.elapsed += dt;
    for p in &mut game.particles {
        p.vel *= (1.0 - 6.0 * dt).max(0.0);
        p.pos += p.vel * dt;
        p.life -= dt;
    }
    game.particles.retain(|p| p.life > 0.0);
    if game.state == GameState::GameOver {
        if ctx.keys().key(KeyCode::KeyR).down_edge() {
            *game = Game::new();
        }
        return;
    }
    let p = &mut game.player;
    p.attack_timer = (p.attack_timer - dt).max(0.0);
    p.attack_cooldown = (p.attack_cooldown - dt).max(0.0);
    p.flash_timer = (p.flash_timer - dt).max(0.0);
    let k = ctx.keys();
    let mut dir = Vec2::ZERO;
    if k.key(KeyCode::KeyA).pressed() || k.key(KeyCode::ArrowLeft).pressed() {
        dir.x -= 1.0;
    }
    if k.key(KeyCode::KeyD).pressed() || k.key(KeyCode::ArrowRight).pressed() {
        dir.x += 1.0;
    }
    if k.key(KeyCode::KeyW).pressed() || k.key(KeyCode::ArrowUp).pressed() {
        dir.y -= 1.0;
    }
    if k.key(KeyCode::KeyS).pressed() || k.key(KeyCode::ArrowDown).pressed() {
        dir.y += 1.0;
    }
    if dir != Vec2::ZERO {
        dir = dir.normalize();
        p.facing_angle = dir.y.atan2(dir.x);
        move_entity(&mut p.pos, dir * PLAYER_SPEED, &game.map, PLAYER_RADIUS, dt);
    }
    if ctx.mouse().in_window() {
        let mouse = ctx.mouse().pos_px();
        let mouse_world = cam.screen_to_world(mouse);
        let aim = mouse_world - p.pos;
        if aim.length_squared() > 1.0 {
            p.facing_angle = aim.y.atan2(aim.x);
        }
    }
    let want_attack = k.key(KeyCode::Space).down_edge() || ctx.mouse().button(MouseButton::Left).down_edge();
    if want_attack && p.attack_cooldown <= 0.0 {
        p.attack_timer = SLASH_DURATION;
        p.attack_cooldown = 0.45;
        let ppos = p.pos;
        let pang = p.facing_angle;
        for e in &mut game.enemies {
            if !e.alive {
                continue;
            }
            let to = e.pos - ppos;
            let dist = to.length();
            if dist < SLASH_RANGE + ENEMY_RADIUS {
                let ang = to.y.atan2(to.x);
                let diff = (ang - pang).rem_euclid(TAU);
                let diff = if diff > PI { TAU - diff } else { diff };
                if diff <= SLASH_HALF_ANGLE {
                    e.hp -= 1;
                    e.pos += to.normalize_or_zero() * 16.0;
                    spawn_burst(&mut game.particles, e.pos, Color::rgba(1.0, 1.0, 0.65, 1.0), 6);
                    if e.hp <= 0 {
                        e.alive = false;
                        game.coins += 3;
                        game.kills += 1;
                        spawn_burst(&mut game.particles, e.pos, Color::rgba(0.4, 1.0, 0.4, 1.0), 14);
                    }
                }
            }
        }
    }
    let ppos = game.player.pos;
    for e in &mut game.enemies {
        if !e.alive {
            continue;
        }
        e.anim += dt;
        e.attack_timer = (e.attack_timer - dt).max(0.0);
        let to = ppos - e.pos;
        let dist = to.length();
        if dist > 0.5 {
            let d = to / dist;
            let speed = e.speed * (0.8 + 0.2 * (e.anim * 2.0).sin().abs());
            move_entity(&mut e.pos, d * speed, &game.map, ENEMY_RADIUS, dt);
        }
        if dist < PLAYER_RADIUS + ENEMY_RADIUS && e.attack_timer <= 0.0 {
            e.attack_timer = 0.9;
            game.player.hp = (game.player.hp - 1).max(0);
            game.player.flash_timer = 0.3;
            spawn_burst(&mut game.particles, game.player.pos, Color::rgba(1.0, 0.3, 0.3, 1.0), 8);
            if game.player.hp <= 0 {
                game.state = GameState::GameOver;
                spawn_burst(&mut game.particles, game.player.pos, Color::rgba(1.0, 0.2, 0.2, 1.0), 24);
            }
        }
    }
    let all_dead = game.enemies.iter().all(|e| !e.alive);
    if all_dead {
        if game.wave_break_timer <= 0.0 {
            game.enemies.clear();
            game.player.hp = (game.player.hp + WAVE_HEAL).min(game.player.max_hp);
            game.coins += WAVE_BONUS_COINS;
            let free = free_tiles(&game.map);
            let mut rng = Rng::new(0xBEEF_CAFE + game.wave as u64 * 0x9E37_79B9);
            game.spawn_next_wave(&free, &mut rng);
        } else {
            game.wave_break_timer -= dt;
        }
    } else {
        game.wave_break_timer = WAVE_BREAK;
    }
}

// ── 渲染辅助 ──────────────────────────────────────────────────────
fn draw_circle(r2d: &mut Render2D, center: Vec2, radius: f32, color: Color, layer: f32) {
    const SEGS: usize = 22;
    let mut verts = Vec::with_capacity(SEGS + 2);
    verts.push(center);
    for i in 0..=SEGS {
        let a = i as f32 / SEGS as f32 * TAU;
        verts.push(center + Vec2::new(a.cos(), a.sin()) * radius);
    }
    r2d.polygon(&verts).tint(color).layer(layer);
}
fn draw_attack_slash(r2d: &mut Render2D, center: Vec2, angle: f32, opacity: f32) {
    let mut verts = Vec::with_capacity(14);
    verts.push(center);
    let segs = 12;
    for i in 0..=segs {
        let a = angle - SLASH_HALF_ANGLE + 2.0 * SLASH_HALF_ANGLE * i as f32 / segs as f32;
        verts.push(center + Vec2::new(a.cos(), a.sin()) * SLASH_RANGE);
    }
    r2d.polygon(&verts)
        .tint(Color::rgba_one(1.0 * opacity))
        .layer(LAYER_EFFECT)
        .blend(BlendMode::Inverse);
}

// ── 静态地形（石头 / 花：固定遮挡层、不插入实体排序，经 StaticMesh 合批） ──
//
// 设计说明（后续改动请保留注释）：
// - `unit_circle_mesh` 是半径为 1 的单位圆扇面网格，注册到 MESHES 后**所有**圆图形用
//   `static_mesh` 共享它，实例变换 = Translate(pos) * Scale(radius)。同 mesh_id + 同
//   纹理（white） + 同 RStates → 整张地图的石头/花全部合批为极少数 DrawCall。
// - 树**不能**放入静态地形：树的遮挡层是 `y_layer(foot_y)`，会插入玩家/史莱姆的实体
//   Y 排序，必须保持动态绘制；石头/花使用固定 `LAYER_TERRAIN`，不参与实体排序，安全静态化。
// - 静态地形在 `map_rev` 变化（R 重开 → `Game::new()`）时自动重建；重建发生在**帧内**
//   （`init` 的 `Gfx` 借用期已过），故应用在 `init` 里存一份 `wgpu::Device` 句柄（克隆共享）。

/// 单个静态圆实例：圆心 + 半径（单位圆网格 instance scale）+ 颜色 + 层级。
struct StaticInst {
    pos: Vec2,
    r: f32,
    color: Color,
    layer: f32,
}

/// 静态地形缓存：单位圆网格 + 石头/花实例列表。
struct StaticTerrain {
    /// 构建时的 map_rev；不匹配则重建。
    rev: u64,
    circle_mesh_id: MeshId,
    stone_insts: Vec<StaticInst>,
    flower_insts: Vec<StaticInst>,
}

/// 构建单位圆扇面网格（中心 (0,0)、半径 1，世界坐标直通），供所有圆实例共享。
///
/// 注：`Gfx` 只在 `App::init` 里存在（借用期不覆盖每帧重建），而静态地形会在 `map_rev`
/// 变化时**帧内**重建，所以这里走低层注册路径（`MeshData::from_pod` + 全局 `MESHES`），
/// 与 `Gfx::mesh` 内部完全等价。
fn unit_circle_mesh(device: &wgpu::Device) -> MeshId {
    const SEGS: usize = 22;
    let mut verts = Vec::with_capacity(SEGS + 2);
    verts.push(VertexP3U2C4 {
        pos: [0.0, 0.0, 0.0],
        uv: [0.0, 0.0],
        color: [1.0; 4],
    });
    for i in 0..=SEGS {
        let a = i as f32 / SEGS as f32 * TAU;
        verts.push(VertexP3U2C4 {
            pos: [a.cos(), a.sin(), 0.0],
            uv: [0.0, 0.0],
            color: [1.0; 4],
        });
    }
    let mut idx = Vec::with_capacity(SEGS * 3);
    for i in 0..SEGS {
        idx.extend_from_slice(&[0, (i + 1) as u16, (i + 2) as u16]);
    }
    let mesh = MeshData::from_pod(device, &verts, &idx, "RPG static circle");
    MeshId::new(rjw_krusie::gpu::MESHES.register(Arc::new(mesh)))
}

impl StaticTerrain {
    fn build(device: &wgpu::Device, map: &Map, rev: u64) -> Self {
        let circle_mesh = unit_circle_mesh(device);
        let circle_mesh_id = circle_mesh;
        let mut stone_insts = Vec::new();
        let mut flower_insts = Vec::new();
        for y in 0..MAP_H {
            for x in 0..MAP_W {
                let o = Vec2::new(x as f32 * TILE, y as f32 * TILE);
                match map.tiles[y * MAP_W + x] {
                    Tile::Stone => {
                        let c = o + Vec2::splat(TILE * 0.5);
                        stone_insts.push(StaticInst {
                            pos: c,
                            r: 13.0,
                            color: Color::rgba(0.52, 0.52, 0.58, 1.0),
                            layer: LAYER_TERRAIN,
                        });
                        stone_insts.push(StaticInst {
                            pos: c + Vec2::new(-3.5, -3.5),
                            r: 8.0,
                            color: Color::rgba(0.7, 0.7, 0.75, 1.0),
                            layer: LAYER_TERRAIN + 0.1,
                        });
                    }
                    Tile::Flower => {
                        let c = o + Vec2::splat(TILE * 0.5);
                        flower_insts.push(StaticInst {
                            pos: c + Vec2::new(-6.0, -5.0),
                            r: 3.0,
                            color: Color::rgba(1.0, 0.5, 0.7, 1.0),
                            layer: LAYER_TERRAIN,
                        });
                        flower_insts.push(StaticInst {
                            pos: c + Vec2::new(5.0, -6.0),
                            r: 3.0,
                            color: Color::rgba(1.0, 0.9, 0.4, 1.0),
                            layer: LAYER_TERRAIN,
                        });
                        flower_insts.push(StaticInst {
                            pos: c + Vec2::new(-1.0, 5.0),
                            r: 3.0,
                            color: Color::rgba(0.9, 0.6, 1.0, 1.0),
                            layer: LAYER_TERRAIN,
                        });
                    }
                    _ => {}
                }
            }
        }
        Self {
            rev,
            circle_mesh_id,
            stone_insts,
            flower_insts,
        }
    }

    /// 提交所有静态圆实例（使用白纹理 → 纯色，全部可合批）。
    fn draw(&self, r2d: &mut Render2D) {
        let white = r2d.white_texture().clone();
        let submit = |r2d: &mut Render2D, insts: &[StaticInst]| {
            for inst in insts {
                let tf = Transform2D::default().with_pos(inst.pos).with_scale(Vec2::splat(inst.r));
                r2d.static_mesh(self.circle_mesh_id, &white)
                    .tint(inst.color)
                    .transform(tf)
                    .layer(inst.layer);
            }
        };
        submit(r2d, &self.stone_insts);
        submit(r2d, &self.flower_insts);
    }
}

fn draw_tiles(r2d: &mut Render2D, cam: &Camera2D, tex: &Tex, game: &Game) {
    let size = cam.region.size();
    let zoom = cam.zoom();
    let hw = size.x * 0.5 / zoom.x;
    let hh = size.y * 0.5 / zoom.y;
    let min = cam.transform.pos - Vec2::new(hw, hh);
    let max = cam.transform.pos + Vec2::new(hw, hh);
    let tx0 = (min.x / TILE).floor().max(0.0) as usize;
    let ty0 = (min.y / TILE).floor().max(0.0) as usize;
    let tx1 = ((max.x / TILE).floor() as usize).min(MAP_W - 1);
    let ty1 = ((max.y / TILE).floor() as usize).min(MAP_H - 1);
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            let tile = game.map.tiles[ty * MAP_W + tx];
            let o = Vec2::new(tx as f32 * TILE, ty as f32 * TILE);
            match tile {
                Tile::Grass => {
                    tex.draw(r2d, &tex.grass, o, Vec2::splat(TILE), Color::WHITE, Transform2D::default(), LAYER_GROUND);
                }
                Tile::Water => {
                    tex.draw(r2d, &tex.water, o, Vec2::splat(TILE), Color::WHITE, Transform2D::default(), LAYER_GROUND);
                }
                Tile::Field => {
                    tex.draw(r2d, &tex.field, o, Vec2::splat(TILE), Color::WHITE, Transform2D::default(), LAYER_GROUND);
                }
                Tile::Sand => {
                    tex.draw(r2d, &tex.sand, o, Vec2::splat(TILE), Color::WHITE, Transform2D::default(), LAYER_GROUND);
                }
                Tile::Flower => {
                    // 花朵圆面片已由 StaticTerrain 静态实例化提交（固定 LAYER_TERRAIN 层），
                    // 这里只画 grass 底。
                    tex.draw(r2d, &tex.grass, o, Vec2::splat(TILE), Color::WHITE, Transform2D::default(), LAYER_GROUND);
                }
                Tile::Stone => {
                    // 石头圆面片已由 StaticTerrain 静态实例化提交（固定 LAYER_TERRAIN 层），
                    // 这里只画 grass 底。
                    tex.draw(r2d, &tex.grass, o, Vec2::splat(TILE), Color::WHITE, Transform2D::default(), LAYER_GROUND);
                }
                Tile::Tree => {
                    tex.draw(r2d, &tex.grass, o, Vec2::splat(TILE), Color::WHITE, Transform2D::default(), LAYER_GROUND);
                    let c = o + Vec2::splat(TILE * 0.5);
                    let trunk_foot = c.y + 15.0;
                    tex.draw(
                        r2d,
                        &tex.white,
                        c + Vec2::new(-5.0, 7.0),
                        Vec2::new(10.0, 14.0),
                        Color::rgba(0.45, 0.3, 0.16, 1.0),
                        Transform2D::default(),
                        y_layer(trunk_foot),
                    );
                    let crown_foot = c.y + 8.0;
                    let sway = (game.elapsed + (tx * 13 + ty * 7) as f32 * 0.35).sin() * 1.5;
                    let tf = Transform2D::default().with_pos(c + Vec2::new(sway, 0.0));
                    tex.draw(r2d, &tex.tree, Vec2::splat(-24.0), Vec2::splat(48.0), Color::WHITE, tf, y_layer(crown_foot));
                }
            }
        }
    }
}

fn draw_entities(r2d: &mut Render2D, tex: &Tex, game: &Game) {
    let p = &game.player;
    let color = if p.flash_timer > 0.0 { Color::rgba(1.0, 0.55, 0.55, 1.0) } else { Color::WHITE };
    let p_foot = p.pos.y + 14.0;
    let tf = Transform2D::default().with_pos(p.pos).with_rot(p.facing_angle);
    tex.draw(r2d, &tex.player, Vec2::splat(-15.0), Vec2::splat(30.0), color, tf, y_layer(p_foot));
    let ppos = game.player.pos;
    for e in &game.enemies {
        if !e.alive {
            continue;
        }
        let e_foot = e.pos.y + 16.0;
        let toward = (ppos - e.pos).y.atan2((ppos - e.pos).x);
        let squash = (e.anim * 3.0).sin();
        let tf = Transform2D::default().with_pos(e.pos).with_rot(toward).with_scale(Vec2::new(1.0 + 0.1 * squash, 1.0 - 0.08 * squash));
        tex.draw(r2d, &tex.slime, Vec2::splat(-16.0), Vec2::splat(32.0), Color::WHITE, tf, y_layer(e_foot));
        let bar_w = 32.0;
        let bar_tl = e.pos + Vec2::new(-bar_w * 0.5, -27.0);
        let frac = e.hp as f32 / 3.0;
        tex.draw(
            r2d,
            &tex.white,
            bar_tl,
            Vec2::new(bar_w, 4.0),
            Color::rgba(0.0, 0.0, 0.0, 0.7),
            Transform2D::default(),
            y_layer(e_foot) + 0.1,
        );
        tex.draw(
            r2d,
            &tex.white,
            bar_tl + Vec2::new(0.5, 0.5),
            Vec2::new((bar_w - 1.0) * frac, 3.0),
            Color::rgba(0.95, 0.25, 0.2, 1.0),
            Transform2D::default(),
            y_layer(e_foot) + 0.2,
        );
    }
    if p.attack_timer > 0.0 {
        draw_attack_slash(r2d, p.pos, p.facing_angle, p.attack_timer / SLASH_DURATION);
    }
    for pt in &game.particles {
        let t = (pt.life / pt.max_life).clamp(0.0, 1.0);
        draw_circle(r2d, pt.pos, pt.size * t, pt.color, y_layer(pt.pos.y) + 0.5);
    }
}

/// HUD 几何（纯色矩形 / 圆）：**屏幕固定 UI ⇒ 走 UI 层**（`Frame::draw_ui()`）。
///
/// 坐标系 = **物理像素、左上原点**（UI 层用 identity 相机提交，无需相机反算）；
/// 常量按 DPI（`scale`）缩放，保证在 1.5×/2× 屏幕上尺寸与位置和逻辑像素一致。
fn draw_ui(r2d: &mut Render2D, tex: &Tex, game: &Game, scale: f32) {
    // 逻辑 → 物理的 HUD 基准（左上角留白 12 逻辑像素）。
    let px = |v: f32| v * scale;
    let origin = Vec2::new(px(12.0), px(12.0));
    let panel_wh = Vec2::new(px(242.0), px(62.0));
    tex.draw(
        r2d,
        &tex.white,
        origin,
        panel_wh,
        Color::rgba(0.08, 0.08, 0.14, 0.72),
        Transform2D::default(),
        0.0,
    );
    let bar_pos = origin + Vec2::new(px(14.0), px(14.0));
    let bar_wh = Vec2::new(px(204.0), px(16.0));
    tex.draw(r2d, &tex.white, bar_pos, bar_wh, Color::rgba(0.15, 0.0, 0.0, 1.0), Transform2D::default(), 0.1);
    let frac = game.player.hp as f32 / game.player.max_hp as f32;
    if frac > 0.0 {
        let hp_color = if frac > 0.5 { Color::rgba(0.25, 0.9, 0.35, 1.0) } else { Color::rgba(0.95, 0.32, 0.25, 1.0) };
        tex.draw(
            r2d,
            &tex.white,
            bar_pos + Vec2::new(px(2.0), px(2.0)),
            Vec2::new((bar_wh.x - px(4.0)) * frac, bar_wh.y - px(4.0)),
            hp_color,
            Transform2D::default(),
            0.2,
        );
    }
    let coin = origin + Vec2::new(px(22.0), px(46.0));
    draw_circle(r2d, coin + Vec2::new(px(5.0), 0.0), px(6.0), Color::rgba(1.0, 0.82, 0.2, 1.0), 0.2);
    for i in 0..10 {
        let lit = game.coins > i * 3;
        tex.draw(
            r2d,
            &tex.white,
            coin + Vec2::new(px(18.0 + i as f32 * 9.0), px(-4.0)),
            Vec2::new(px(6.0), px(8.0)),
            if lit { Color::rgba(1.0, 0.82, 0.2, 1.0) } else { Color::rgba(1.0, 1.0, 1.0, 0.18) },
            Transform2D::default(),
            0.2,
        );
    }
    let kill = origin + Vec2::new(px(22.0), px(66.0));
    draw_circle(r2d, kill + Vec2::new(px(5.0), 0.0), px(6.0), Color::rgba(0.95, 0.3, 0.3, 1.0), 0.2);
    for i in 0..10 {
        let lit = game.kills > i;
        tex.draw(
            r2d,
            &tex.white,
            kill + Vec2::new(px(18.0 + i as f32 * 9.0), px(-4.0)),
            Vec2::new(px(6.0), px(8.0)),
            if lit { Color::rgba(0.95, 0.3, 0.3, 1.0) } else { Color::rgba(1.0, 1.0, 1.0, 0.18) },
            Transform2D::default(),
            0.2,
        );
    }
}

/// Game Over 全屏压暗（**UI 层**，在 HUD 之后录制 ⇒ 盖住 HUD）。
fn draw_gameover(r2d: &mut Render2D, tex: &Tex, size: Vec2) {
    tex.draw(
        r2d,
        &tex.white,
        Vec2::ZERO,
        size,
        Color::rgba(0.45, 0.0, 0.0, 0.38),
        Transform2D::default(),
        1.0,
    );
}

/// HUD 文本（HP / 波次 / 击杀）：**UI 层**文本（`Frame::text_ui`），物理像素定位。
///
/// 与几何分成两个函数，是为了走 `Frame::text_ui(|t| ..)`（文本子系统与 UI 层渲染器需同时
/// 可变借用）；二者都录制进**同一个 UI 层队列**，UI 层按录制顺序提交（`SortMode::None`）。
///
/// 文本渲染（唯一链）：`TextStyle` 公共字号/行距一次定义，逐标签只写差异
/// （`Label::style(..)` 套用；定位走 `at` / `center`）。
fn draw_ui_text(t: &mut TextCtx<'_>, game: &Game, scale: f32) {
    let px = |v: f32| v * scale;
    let origin = Vec2::new(px(12.0), px(12.0));
    let bar_pos = origin + Vec2::new(px(14.0), px(14.0));
    let coin = origin + Vec2::new(px(22.0), px(46.0));
    let kill = origin + Vec2::new(px(22.0), px(66.0));

    // ── 文本渲染（TextStyle：公共字体/字号/行距一次定义，逐处只写差异） ──
    // 字号同样按 DPI 缩放（UI 层坐标是物理像素）。
    let style = TextStyle::new()
        .font_family("SimHei")
        .size(px(14.0))
        .line_space(LineSpace::Multiple(1.5))
        .align(Align::Left);
    t.label(format!("❤HP: {} / {}", game.player.hp, game.player.max_hp))
        .style(style.clone())
        .color(Color::WHITE)
        .at(bar_pos + vec2(0.0, px(26.0)))
        .draw(0.3);
    t.label(format!("第 {} 波", game.wave))
        .style(style.clone())
        .color(Color::rgba(1.0, 0.82, 0.2, 1.0))
        .at(coin + vec2(px(120.0), px(-2.0)))
        .draw(0.3);
    t.label(format!("击杀 {}", game.kills))
        .style(style)
        .color(Color::rgba(0.95, 0.3, 0.3, 1.0))
        .at(kill + vec2(px(120.0), px(-2.0)))
        .draw(0.3);
}

/// Game Over 提示文本（**UI 层**，在压暗层之后录制 ⇒ 恒在最上）。
fn draw_gameover_text(t: &mut TextCtx<'_>, size: Vec2, scale: f32) {
    let px = |v: f32| v * scale;
    t.label("❤GAME OVER — 按 R 重开❤")
        .font_family("SimHei")
        .size(px(22.0))
        .line_height(px(28.0))
        .align(Align::Center)
        .center(size * 0.5)
        .color(Color::rgba(1.0, 0.3, 0.3, 1.0))
        .draw(1.1);
}

// ── 程序化纹理 ────────────────────────────────────────────────────
fn set_px(buf: &mut [u8], w: usize, x: usize, y: usize, c: [u8; 4]) {
    let i = (y * w + x) * 4;
    buf[i..i + 4].copy_from_slice(&c);
}
fn blend_px(buf: &mut [u8], w: usize, x: usize, y: usize, c: [u8; 4]) {
    let i = (y * w + x) * 4;
    let a = c[3] as f32 / 255.0;
    for k in 0..3 {
        buf[i + k] = (buf[i + k] as f32 * (1.0 - a) + c[k] as f32 * a) as u8;
    }
    buf[i + 3] = 255;
}
fn fill_circle(buf: &mut [u8], w: usize, h: usize, center: Vec2, r: f32, color: [u8; 4]) {
    let x0 = (center.x - r - 1.0).floor().max(0.0) as usize;
    let y0 = (center.y - r - 1.0).floor().max(0.0) as usize;
    let x1 = (center.x + r + 1.0).ceil().min(w as f32 - 1.0) as usize;
    let y1 = (center.y + r + 1.0).ceil().min(h as f32 - 1.0) as usize;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - center.x;
            let dy = y as f32 + 0.5 - center.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= r {
                let alpha = ((r - dist).clamp(0.0, 1.0) * color[3] as f32 / 255.0).clamp(0.0, 1.0);
                let mut c = color;
                c[3] = (alpha * 255.0) as u8;
                blend_px(buf, w, x, y, c);
            }
        }
    }
}
fn fill_ellipse(buf: &mut [u8], w: usize, h: usize, center: Vec2, rx: f32, ry: f32, color: [u8; 4]) {
    let x0 = (center.x - rx - 1.0).floor().max(0.0) as usize;
    let y0 = (center.y - ry - 1.0).floor().max(0.0) as usize;
    let x1 = (center.x + rx + 1.0).ceil().min(w as f32 - 1.0) as usize;
    let y1 = (center.y + ry + 1.0).ceil().min(h as f32 - 1.0) as usize;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let nx = (x as f32 + 0.5 - center.x) / rx;
            let ny = (y as f32 + 0.5 - center.y) / ry;
            let d = (nx * nx + ny * ny).sqrt();
            if d <= 1.0 {
                let alpha = ((1.0 - d).clamp(0.0, 0.5) * 2.0 * color[3] as f32 / 255.0).clamp(0.0, 1.0);
                let mut c = color;
                c[3] = (alpha * 255.0) as u8;
                blend_px(buf, w, x, y, c);
            }
        }
    }
}
fn make_grass() -> Vec<u8> {
    let (w, h) = (32, 32);
    let mut buf = vec![0u8; w * h * 4];
    let mut rng = Rng::new(0xA8_BCDE);
    for y in 0..h {
        for x in 0..w {
            let v = 0.9 + rng.f32() * 0.12;
            set_px(&mut buf, w, x, y, [(0.20 * v * 255.0) as u8, (0.52 * v * 255.0) as u8, (0.22 * v * 255.0) as u8, 255]);
        }
    }
    buf
}
fn make_field() -> Vec<u8> {
    let (w, h) = (32, 32);
    let mut buf = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let v = 0.9 + 0.1 * ((x as f32 * 0.6 + y as f32 * 0.2).sin() * 0.5 + 0.5);
            set_px(&mut buf, w, x, y, [(0.55 * v * 255.0) as u8, (0.62 * v * 255.0) as u8, (0.18 * v * 255.0) as u8, 255]);
        }
    }
    for y in 0..h {
        for x in 0..w {
            if (((x as i32 + y as i32) / 6) % 2) != 0 {
                let i = (y * w + x) * 4;
                buf[i + 1] = buf[i + 1].saturating_sub(26);
            }
        }
    }
    buf
}
fn make_sand() -> Vec<u8> {
    let (w, h) = (32, 32);
    let mut buf = vec![0u8; w * h * 4];
    let mut rng = Rng::new(0x00F1_5ABC);
    for y in 0..h {
        for x in 0..w {
            let v = 0.92 + rng.f32() * 0.1;
            set_px(&mut buf, w, x, y, [(0.85 * v * 255.0) as u8, (0.78 * v * 255.0) as u8, (0.52 * v * 255.0) as u8, 255]);
        }
    }
    buf
}
fn make_water() -> Vec<u8> {
    let (w, h) = (32, 32);
    let mut buf = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let shade = 0.82 + 0.18 * ((x as f32 * 0.4 + y as f32 * 0.15).sin() * 0.5 + 0.5);
            set_px(&mut buf, w, x, y, [(30.0 * shade) as u8, (82.0 * shade) as u8, (178.0 * shade) as u8, 255]);
        }
    }
    for y in (4..h).step_by(9) {
        for x in 2..w - 2 {
            if ((x as f32 * 0.5 + y as f32) % 9.0) < 3.0 {
                let i = (y * w + x) * 4;
                buf[i..i + 3].copy_from_slice(&[210, 228, 255]);
            }
        }
    }
    buf
}
fn make_tree() -> Vec<u8> {
    let (w, h) = (64, 64);
    let mut buf = vec![0u8; w * h * 4];
    let c = Vec2::new(32.0, 30.0);
    fill_circle(&mut buf, w, h, c, 26.0, [24, 70, 30, 255]);
    fill_circle(&mut buf, w, h, c, 22.0, [44, 116, 48, 255]);
    fill_circle(&mut buf, w, h, c + Vec2::new(-6.0, -7.0), 13.0, [76, 150, 66, 255]);
    fill_circle(&mut buf, w, h, c + Vec2::new(8.0, 6.0), 9.0, [60, 138, 58, 255]);
    let mut rng = Rng::new(0x77_01);
    for _ in 0..40 {
        let a = rng.f32() * TAU;
        let r = rng.f32() * 18.0;
        let p = c + Vec2::new(a.cos(), a.sin()) * r;
        fill_circle(&mut buf, w, h, p, 1.5, [130, 200, 100, 200]);
    }
    buf
}
fn make_player() -> Vec<u8> {
    let (w, h) = (32, 32);
    let mut buf = vec![0u8; w * h * 4];
    let c = Vec2::new(16.0, 16.0);
    fill_circle(&mut buf, w, h, c, 14.0, [250, 208, 56, 255]);
    fill_circle(&mut buf, w, h, c + Vec2::new(-2.5, -3.0), 8.0, [255, 236, 132, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(17.0, 11.0), 4.2, [255, 255, 255, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(19.5, 11.0), 2.2, [24, 24, 24, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(17.0, 21.0), 4.2, [255, 255, 255, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(19.5, 21.0), 2.2, [24, 24, 24, 255]);
    buf
}
fn make_slime() -> Vec<u8> {
    let (w, h) = (32, 32);
    let mut buf = vec![0u8; w * h * 4];
    let c = Vec2::new(16.0, 18.0);
    fill_ellipse(&mut buf, w, h, c, 14.0, 11.5, [82, 196, 88, 255]);
    fill_ellipse(&mut buf, w, h, c + Vec2::new(-2.0, -2.5), 10.0, 7.5, [132, 232, 122, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(17.0, 13.0), 4.0, [255, 255, 255, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(19.4, 13.0), 2.1, [22, 22, 22, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(17.0, 23.0), 4.0, [255, 255, 255, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(19.4, 23.0), 2.1, [22, 22, 22, 255]);
    buf
}

/// 图集纹理 + 一行绘制便利方法。
struct Tex {
    #[allow(dead_code)]
    atlas: DynamicAtlas,
    grass: AtlasRegion,
    field: AtlasRegion,
    sand: AtlasRegion,
    water: AtlasRegion,
    tree: AtlasRegion,
    player: AtlasRegion,
    slime: AtlasRegion,
    white: AtlasRegion,
}
impl Tex {
    fn create(gfx: &Gfx) -> Self {
        let mut atlas = DynamicAtlas::new(
            gfx,
            AtlasConfig {
                max_pages: 2,
                page_size: 512, // 够了
                ..Default::default()
            },
        );
        let white = atlas.white();
        let grass = atlas.insert("grass".to_string(), Rgba8::new(&make_grass(), (32, 32))).unwrap();
        let field = atlas.insert("field".to_string(), Rgba8::new(&make_field(), (32, 32))).unwrap();
        let sand = atlas.insert("sand".to_string(), Rgba8::new(&make_sand(), (32, 32))).unwrap();
        let water = atlas.insert("water".to_string(), Rgba8::new(&make_water(), (32, 32))).unwrap();
        let tree = atlas.insert("tree".to_string(), Rgba8::new(&make_tree(), (64, 64))).unwrap();
        let player = atlas.insert("player".to_string(), Rgba8::new(&make_player(), (32, 32))).unwrap();
        let slime = atlas.insert("slime".to_string(), Rgba8::new(&make_slime(), (32, 32))).unwrap();
        Self {
            atlas,
            grass,
            field,
            sand,
            water,
            tree,
            player,
            slime,
            white,
        }
    }

    #[allow(clippy::too_many_arguments, reason = "示例内的绘制辅助函数：渲染器/区域/世界坐标/图元全部必要")]
    fn draw<'a>(
        &self,
        r2d: &'a mut Render2D,
        region: &AtlasRegion,
        world_tl: Vec2,
        world_wh: Vec2,
        color: Color,
        transform: Transform2D,
        layer: impl Into<Layer> + 'a,
    ) -> rjw_krusie::render2d::SpriteBuilder<'a> {
        let tex_ref = TEXTURES.get(region.page_uid).unwrap();
        let spr = SpriteRect::with_uv_tex(
            world_tl,
            world_wh,
            Vec2::new(region.tl_px.0 as f32, region.tl_px.1 as f32),
            Vec2::new(region.wh_px.0 as f32, region.wh_px.1 as f32),
            &tex_ref,
        );
        r2d.sprite(spr, &tex_ref)
            .tint(color)
            .transform(transform)
            .layer(layer)
    }
}

// ── App ───────────────────────────────────────────────────────────
struct RpgApp {
    /// 底层设备（`init` 里取一次；静态地形重建需要在**取帧之前**也能建网格）。
    device: Option<Arc<wgpu::Device>>,
    cam: Camera2D,
    tex: Option<Tex>,
    game: Game,
    /// 石头/花静态地形缓存（按 `map_rev` 重建，R 重开后自动更新）。
    static_terrain: Option<StaticTerrain>,
}
impl Default for RpgApp {
    fn default() -> Self {
        Self {
            device: None,
            cam: Camera2D::default(),
            tex: None,
            game: Game::new(),
            static_terrain: None,
        }
    }
}
impl App for RpgApp {
    fn config(&self) -> AppConfig {
        AppConfig::new("eg260731RPG").size(1280.0, 720.0)
    }

    fn init(&mut self, gfx: &Gfx) {
        // `wgpu::Device` 是共享句柄（`Clone` 共享同一底层设备），存一份供帧内重建静态网格用。
        self.device = Some(Arc::new(gfx.device().clone()));
        self.tex = Some(Tex::create(gfx));
        // 文本子系统由运行时 `Ctx` 持有（`Frame::text` 借出），应用不再自建。
        // 相机视口在 `update` 里用当前画面矩形写回（`f.region()`），与窗口尺寸自动一致。
        self.cam.transform.pos = self.game.player.pos;
    }

    fn update(&mut self, ctx: &mut Ctx) {
        if ctx.key(KeyCode::Escape).down_edge() {
            ctx.exit();
        }
        let dt = ctx.dt().min(0.05);
        // ── 逻辑半程：无帧也执行 ──
        update(&mut self.game, &self.cam, ctx, dt);
        self.cam.transform.pos += (self.game.player.pos - self.cam.transform.pos) * (1.0 - (-20.0 * dt).exp());

        // ── 渲染半程：守卫在应用里（无帧不执行渲染代码）──
        let Some(mut f) = ctx.frame() else { return };
        // 相机：把画面矩形写回相机（相机自己存视口），随后可用它做屏幕 ↔ 世界换算。
        self.cam.set_region(f.region());
        let tex = self.tex.as_ref().expect("tex 已在 init 建立");

        if let Some(w) = f.window_handle() {
            w.set_title(&format!(
                "eg260731RPG  第 {} 波 FPS {:.0} | HP {}/{} | 金币 {} | 击杀 {} | WASD 移动 · 空格/左键 攻击 · R 重开 · Esc 退出",
                self.game.wave,
                f.fps(),
                self.game.player.hp,
                self.game.player.max_hp,
                self.game.coins,
                self.game.kills
            ));
        }

        // 石头 / 花静态地形：地图版本变化时重建一次（单位圆网格 + 实例列表常驻），
        // 每帧只提交实例数据，全部合批。树保持动态（Y 排序插入实体，绝不入此地）。
        if self.static_terrain.as_ref().map(|t| t.rev) != Some(self.game.map_rev) {
            let device = self.device.as_ref().expect("device 已在 init 取得");
            self.static_terrain = Some(StaticTerrain::build(device, &self.game.map, self.game.map_rev));
        }
        self.static_terrain.as_ref().unwrap().draw(f.draw());
        draw_tiles(f.draw(), &self.cam, tex, &self.game);
        draw_entities(f.draw(), tex, &self.game);

        // ── 屏幕固定 UI ⇒ **UI 层**（`f.draw_ui()` / `f.text_ui()`）──
        // 层级：世界层（`f.draw()` / `f.text()`）先提交，UI 层随后提交 ⇒ UI 恒在世界之上；
        // 位置：UI 层是**物理像素、左上原点**（identity 相机），不再用相机反算屏幕左上角，
        //       相机旋转 / 缩放 / 跟随都不会影响 HUD；常量按 DPI（`f.scale()`）缩放。
        let s = f.scale();
        let region = f.region();
        draw_ui(f.draw_ui(), tex, &self.game, s);
        f.text_ui(|t| draw_ui_text(t, &self.game, s));
        if self.game.state == GameState::GameOver {
            // 顺序：压暗层（盖住 HUD）→ GAME OVER 文本（恒在最上）；UI 层按录制顺序提交。
            draw_gameover(f.draw_ui(), tex, region.size());
            f.text_ui(|t| draw_gameover_text(t, region.size(), s));
        }

        f.submit(&mut self.cam, Clear::color(Color::rgb(0.13, 0.24, 0.12)));
    }

    fn resized(&mut self, _ctx: &mut Ctx) {
        // 画面矩形已自动跟随（提交时写回 `cam.region`），本应用无依赖尺寸的资源需重建。
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    run(RpgApp::default())
}
