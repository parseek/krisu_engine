//! eg260731RPG —— 小型 2D 顶视角 RPG 范例
//!
//! 玩法：
//! - WASD / 方向键：移动（角色自动面向移动/鼠标方向）
//! - 鼠标左键 / 空格：挥砍攻击（面向方向扇形，击杀史莱姆得金币）
//! - 史莱姆会追踪并撞击玩家造成伤害
//! - HP 归零 → 游戏结束，按 R 重新开始，Esc 退出
//!
//! 展示的引擎能力：`Render2D` 程序化纹理 / sprite / mesh(fan) / 层级 / 相机跟随。

use std::f32::consts::{PI, TAU};

use glam::Vec2;
use rjw_2d_render::{ArcTextureWrapped, ClearConfig, Render2D, SpriteRect};
use rjw_color::Color;
use rjw_main::*;
use rjw_render::{RenderConfig, RenderContext, wgpu};
use rjw_transform::{Camera2D, Transform2D};

// ── 常量 ─────────────────────────────────────────────────────────
const TILE: f32 = 32.0;
// 地图扩大：64×64 格 = 2048×2048 世界像素，提供宽阔活动范围。
const MAP_W: usize = 64;
const MAP_H: usize = 64;

const PLAYER_RADIUS: f32 = 13.0;
const ENEMY_RADIUS: f32 = 15.0;
const PLAYER_SPEED: f32 = 210.0;
const ENEMY_SPEED: f32 = 55.0;

// ── 波次系统 ──────────────────────────────────────────────────────
/// 当前波敌人全灭后，下一波生成的间歇（秒）。
const WAVE_BREAK: f32 = 1.6;
/// 第 1 波敌人数量。
const WAVE_BASE_COUNT: usize = 4;
/// 每过一波增加的敌人数。
const WAVE_PER: usize = 2;
/// 单波敌人数量上限。
const WAVE_MAX: usize = 20;
/// 每清完一波：回血额度。
const WAVE_HEAL: i32 = 1;
/// 每清完一波：额外金币奖励。
const WAVE_BONUS_COINS: i32 = 2;

const SLASH_RANGE: f32 = 74.0;
const SLASH_HALF_ANGLE: f32 = 38.0_f32.to_radians();
const SLASH_DURATION: f32 = 0.2;

const MAX_HP: i32 = 5;

// 绘制层级（数值小先画）
const LAYER_GROUND: f32 = 0.0;
const LAYER_TERRAIN: f32 = 1.0; // 水 / 石头装饰

/// 自动 **y-sort** 基准层级：树干的底部、树冠、玩家、史莱姆等「立绘」对象
/// 统一从这里起步，用「脚底世界 Y」计算动态 layer —— Y 越大（越靠屏幕下）
/// layer 越大 → 绘制时排在后面 → 盖住上方对象，形成 RPG 纵深感。
/// 引擎对 (layer, states) 排序后自动生效，无需手动穿插绘制调用。
const LAYER_Y_SORT_BASE: f32 = 10.0;
const LAYER_UI: f32 = 10000000.0;
const LAYER_GAMEOVER: f32 = 20000000.0;

/// y-sort 辅助：按脚底 y（世界坐标）生成对象 layer。
/// 数越小先画（更靠上/更远），越大后画（更靠下/更近）。
#[inline]
fn y_layer(foot_y: f32) -> f32 {
    LAYER_Y_SORT_BASE + foot_y
}

// ── 简单随机数（无 rand 依赖） ────────────────────────────────────
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        debug_assert!(lo < hi);
        lo + (self.next() % (hi - lo) as u64) as usize
    }
}

// ── 地图 ──────────────────────────────────────────────────────────
#[derive(Clone, Copy, PartialEq)]
enum Tile {
    /// 草地（基础地表）
    Grass,
    /// 麦田 / 田野（可通行，视觉多样）
    Field,
    /// 沙地 / 湖岸（可通行）
    Sand,
    /// 水（不可通行）
    Water,
    /// 树（不可通行，树冠 y-sort 盖人）
    Tree,
    /// 石头（不可通行）
    Stone,
    /// 花丛（可通行，装饰）
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

    /// 世界坐标 AABB 是否与不可通行瓦片相交。
    fn collides(&self, min: Vec2, max: Vec2) -> bool {
        let tx0 = (min.x / TILE).floor() as i32;
        let ty0 = (min.y / TILE).floor() as i32;
        let tx1 = (max.x / TILE).floor() as i32;
        let ty1 = (max.y / TILE).floor() as i32;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if let Some(t) = self.tile_at(tx, ty) {
                    if t.is_blocked() {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// 画一个带椒盐噪声边缘的圆区域。
fn set_radius(map: &mut Map, rng: &mut Rng, cx: i32, cy: i32, r: i32, t: Tile) {
    for dy in -r..=r {
        for dx in -r..=r {
            let dist = (dx * dx + dy * dy) as f32;
            // 半径内带一点椒盐噪声边缘，让区域更自然
            let wob = 1.0 + (rng.f32() - 0.5) * 0.7;
            if dist <= (r as f32 * wob) * (r as f32 * wob) {
                map.set_tile(cx + dx, cy + dy, t);
            }
        }
    }
}

/// 生成多样化大地图：
/// - 边框一圈树
/// - 多个随机湖泊（Water）+ 湖岸沙地（Sand）
/// - 多个石山区（Stone）
/// - 多个树林区（Tree）
/// - 几块麦田（Field）
/// - 零星花丛（Flower）
/// - 出生点清空为安全草原
fn generate_map() -> Map {
    let mut rng = Rng::new(0x9E37_79B9_7F4A_7C15);
    let mut map = Map {
        tiles: vec![Tile::Grass; MAP_W * MAP_H],
    };

    // 1) 湖泊：多个随机圆心 + 沙岸
    for _ in 0..7 {
        let cx = 3 + rng.range(0, MAP_W - 6) as i32;
        let cy = 3 + rng.range(0, MAP_H - 6) as i32;
        let r = 3 + rng.range(0, 5) as i32;
        set_radius(&mut map, &mut rng, cx, cy, r + 1, Tile::Sand);
        set_radius(&mut map, &mut rng, cx, cy, r, Tile::Water);
    }
    // 2) 石山区
    for _ in 0..6 {
        let cx = 3 + rng.range(0, MAP_W - 6) as i32;
        let cy = 3 + rng.range(0, MAP_H - 6) as i32;
        let r = 2 + rng.range(0, 4) as i32;
        set_radius(&mut map, &mut rng, cx, cy, r, Tile::Stone);
    }
    // 3) 树林区
    for _ in 0..9 {
        let cx = 2 + rng.range(0, MAP_W - 4) as i32;
        let cy = 2 + rng.range(0, MAP_H - 4) as i32;
        let r = 2 + rng.range(0, 4) as i32;
        set_radius(&mut map, &mut rng, cx, cy, r, Tile::Tree);
    }
    // 4) 麦田：随机位置几块矩形
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
    // 5) 花丛：零星散布
    for _ in 0..40 {
        let x = 2 + rng.range(0, MAP_W - 4) as i32;
        let y = 2 + rng.range(0, MAP_H - 4) as i32;
        map.set_tile(x, y, Tile::Flower);
    }

    // 边框一圈树
    for x in 0..MAP_W as i32 {
        map.set_tile(x, 0, Tile::Tree);
        map.set_tile(x, MAP_H as i32 - 1, Tile::Tree);
    }
    for y in 0..MAP_H as i32 {
        map.set_tile(0, y, Tile::Tree);
        map.set_tile(MAP_W as i32 - 1, y, Tile::Tree);
    }

    // 出生区域（地图正中心）清空为安全草原 11×11
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

struct Game {
    map: Map,
    player: Player,
    enemies: Vec<Enemy>,
    particles: Vec<Particle>,
    state: GameState,
    coins: i32,
    kills: i32,
    /// 当前波号（第 1 波 = 1）。
    wave: usize,
    /// 波与波之间的倒计时（秒）：≤0 时生成下一波。
    wave_break_timer: f32,
    elapsed: f32,
}

impl Game {
    fn new() -> Self {
        let map = generate_map();
        let free = free_tiles(&map);
        let mut rng = Rng::new(0x1234_567);
        let mut game = Self {
            map,
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

    /// 生成第 `wave+1` 波：敌人数量递增、速度/血量随波提升，并播报粒子。
    fn spawn_next_wave(&mut self, free: &[Vec2], rng: &mut Rng) {
        self.wave += 1;
        let count = (WAVE_BASE_COUNT + (self.wave - 1) * WAVE_PER).min(WAVE_MAX);
        let hp_bonus = (self.wave as f32 * 0.5) as i32; // 每两波 +1 血量
        let speed_bonus = (self.wave as f32 - 1.0) * 3.0;
        for _ in 0..count {
            let p = free[rng.range(0, free.len())];
            let mut e = Enemy::new(p, (ENEMY_SPEED + speed_bonus + rng.f32() * 10.0).min(140.0));
            e.hp = (3 + hp_bonus).clamp(3, 9);
            self.enemies.push(e);
        }
        // 波次播报：中心一圈紫色粒子
        spawn_burst(&mut self.particles, self.player.pos, Color::rgba(0.8, 0.5, 1.0, 1.0), 10);
    }
}

// ── 逻辑更新 ──────────────────────────────────────────────────────
fn move_entity(pos: &mut Vec2, vel: Vec2, map: &Map, radius: f32, dt: f32) {
    // 分离轴移动，沿墙滑动
    let nx = pos.x + vel.x * dt;
    if !map.collides(
        Vec2::new(nx - radius, pos.y - radius),
        Vec2::new(nx + radius, pos.y + radius),
    ) {
        pos.x = nx;
    }
    let ny = pos.y + vel.y * dt;
    if !map.collides(
        Vec2::new(pos.x - radius, ny - radius),
        Vec2::new(pos.x + radius, ny + radius),
    ) {
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

fn update(game: &mut Game, cam: &Camera2D, ctx: &MainContext, dt: f32) {
    game.elapsed += dt;

    // 粒子
    for p in &mut game.particles {
        p.vel *= (1.0 - 6.0 * dt).max(0.0);
        p.pos += p.vel * dt;
        p.life -= dt;
    }
    game.particles.retain(|p| p.life > 0.0);

    if game.state == GameState::GameOver {
        if ctx.keyboard.get(KeyCode::KeyR).down_edge() {
            *game = Game::new();
        }
        return;
    }

    let p = &mut game.player;
    p.attack_timer = (p.attack_timer - dt).max(0.0);
    p.attack_cooldown = (p.attack_cooldown - dt).max(0.0);
    p.flash_timer = (p.flash_timer - dt).max(0.0);

    // 移动输入
    let k = &ctx.keyboard;
    let mut dir = Vec2::ZERO;
    if k.get(KeyCode::KeyA).pressed() || k.get(KeyCode::ArrowLeft).pressed() {
        dir.x -= 1.0;
    }
    if k.get(KeyCode::KeyD).pressed() || k.get(KeyCode::ArrowRight).pressed() {
        dir.x += 1.0;
    }
    if k.get(KeyCode::KeyW).pressed() || k.get(KeyCode::ArrowUp).pressed() {
        dir.y -= 1.0;
    }
    if k.get(KeyCode::KeyS).pressed() || k.get(KeyCode::ArrowDown).pressed() {
        dir.y += 1.0;
    }
    if dir != Vec2::ZERO {
        dir = dir.normalize();
        p.facing_angle = dir.y.atan2(dir.x);
        move_entity(&mut p.pos, dir * PLAYER_SPEED, &game.map, PLAYER_RADIUS, dt);
    }

    // 鼠标瞄准
    if ctx.mouse.in_window() {
        let mouse = ctx.mouse.get_mouse_position();
        let mouse_world = cam.screen_to_world(Vec2::new(mouse.0 as f32, mouse.1 as f32));
        let aim = mouse_world - p.pos;
        if aim.length_squared() > 1.0 {
            p.facing_angle = aim.y.atan2(aim.x);
        }
    }

    // 攻击
    let want_attack = k.get(KeyCode::Space).down_edge()
        || ctx
            .mouse
            .get_mouse_button_state(rjw_main::winit::event::MouseButton::Left)
            .down_edge();
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
                        spawn_burst(
                            &mut game.particles,
                            e.pos,
                            Color::rgba(0.4, 1.0, 0.4, 1.0),
                            14,
                        );
                    }
                }
            }
        }
    }

    // 敌人 AI
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
                spawn_burst(
                    &mut game.particles,
                    game.player.pos,
                    Color::rgba(1.0, 0.2, 0.2, 1.0),
                    24,
                );
            }
        }
    }

    // ── 波次推进：当前波全部敌人死亡 → 间歇倒计时 → 生成更强下一波 ──
    let all_dead = game.enemies.iter().all(|e| !e.alive);
    if all_dead {
        if game.wave_break_timer <= 0.0 {
            // 清掉尸体、回血、发放清波奖励，然后生成下一波
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
    r2d.add_polygon_fan(&verts, color, layer);
}

fn draw_attack_slash(r2d: &mut Render2D, center: Vec2, angle: f32, opacity: f32) {
    let mut verts = Vec::with_capacity(14);
    verts.push(center);
    let segs = 12;
    for i in 0..=segs {
        let a = angle - SLASH_HALF_ANGLE + 2.0 * SLASH_HALF_ANGLE * i as f32 / segs as f32;
        verts.push(center + Vec2::new(a.cos(), a.sin()) * SLASH_RANGE);
    }
    // y-sort 内跟随玩家脚底（+0.3）：攻击弧盖住同 Y 的实体本体与
    // 血条（实体本体/血条为 y_layer+0.0 / +0.1 / +0.2），又仍受 y-sort 约束。
    r2d.add_polygon_fan(
        &verts,
        Color::rgba(1.0, 1.0, 0.9, 0.65 * opacity),
        y_layer(center.y + 14.0) + 0.3,
    );
}

fn draw_tiles(r2d: &mut Render2D, cam: &Camera2D, tex: &Textures, game: &Game) {
    let half_w = cam.viewport_size.x * 0.5 / cam.zoom.x;
    let half_h = cam.viewport_size.y * 0.5 / cam.zoom.y;
    let min = cam.position - Vec2::new(half_w, half_h);
    let max = cam.position + Vec2::new(half_w, half_h);
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
                    r2d.add_sprite2d_default(
                        SpriteRect::from_texture(o, Vec2::splat(TILE)),
                        Color::WHITE,
                        Transform2D::default(),
                        LAYER_GROUND,
                        &tex.grass,
                    );
                }
                Tile::Water => {
                    r2d.add_sprite2d_default(
                        SpriteRect::from_texture(o, Vec2::splat(TILE)),
                        Color::WHITE,
                        Transform2D::default(),
                        LAYER_GROUND,
                        &tex.water,
                    );
                }
                Tile::Field => {
                    // 麦田：基础草地 + 金色条纹
                    r2d.add_sprite2d_default(
                        SpriteRect::from_texture(o, Vec2::splat(TILE)),
                        Color::WHITE,
                        Transform2D::default(),
                        LAYER_GROUND,
                        &tex.field,
                    );
                }
                Tile::Sand => {
                    r2d.add_sprite2d_default(
                        SpriteRect::from_texture(o, Vec2::splat(TILE)),
                        Color::WHITE,
                        Transform2D::default(),
                        LAYER_GROUND,
                        &tex.sand,
                    );
                }
                Tile::Flower => {
                    // 花丛：草地 + 随机小花瓣
                    r2d.add_sprite2d_default(
                        SpriteRect::from_texture(o, Vec2::splat(TILE)),
                        Color::WHITE,
                        Transform2D::default(),
                        LAYER_GROUND,
                        &tex.grass,
                    );
                    let c = o + Vec2::splat(TILE * 0.5);
                    draw_circle(
                        r2d,
                        c + Vec2::new(-6.0, -5.0),
                        3.0,
                        Color::rgba(1.0, 0.5, 0.7, 1.0),
                        LAYER_TERRAIN,
                    );
                    draw_circle(
                        r2d,
                        c + Vec2::new(5.0, -6.0),
                        3.0,
                        Color::rgba(1.0, 0.9, 0.4, 1.0),
                        LAYER_TERRAIN,
                    );
                    draw_circle(
                        r2d,
                        c + Vec2::new(-1.0, 5.0),
                        3.0,
                        Color::rgba(0.9, 0.6, 1.0, 1.0),
                        LAYER_TERRAIN,
                    );
                }
                Tile::Stone => {
                    r2d.add_sprite2d_default(
                        SpriteRect::from_texture(o, Vec2::splat(TILE)),
                        Color::WHITE,
                        Transform2D::default(),
                        LAYER_GROUND,
                        &tex.grass,
                    );
                    let c = o + Vec2::splat(TILE * 0.5);
                    draw_circle(r2d, c, 13.0, Color::rgba(0.52, 0.52, 0.58, 1.0), LAYER_TERRAIN);
                    draw_circle(
                        r2d,
                        c + Vec2::new(-3.5, -3.5),
                        8.0,
                        Color::rgba(0.7, 0.7, 0.75, 1.0),
                        LAYER_TERRAIN + 0.1,
                    );
                }
                Tile::Tree => {
                    r2d.add_sprite2d_default(
                        SpriteRect::from_texture(o, Vec2::splat(TILE)),
                        Color::WHITE,
                        Transform2D::default(),
                        LAYER_GROUND,
                        &tex.grass,
                    );
                    let c = o + Vec2::splat(TILE * 0.5);
                    // 树干（脚底 = 树心 y + 15，y-sort 基准）
                    let trunk_foot = c.y + 15.0;
                    r2d.add_sprite2d_default_solid(
                        SpriteRect::from_texture(c + Vec2::new(-5.0, 7.0), Vec2::new(10.0, 14.0)),
                        Color::rgba(0.45, 0.3, 0.16, 1.0),
                        Transform2D::default(),
                        y_layer(trunk_foot),
                    );
                    // 树冠（脚底略微下移让角色从树冠下面露出时更自然；轻微摇摆）
                    let crown_foot = c.y + 8.0;
                    let sway =
                        (game.elapsed + (tx * 13 + ty * 7) as f32 * 0.35).sin() * 1.5;
                    let tf = Transform2D::default().with_pos(c + Vec2::new(sway, 0.0));
                    r2d.add_sprite2d_default(
                        SpriteRect::from_texture(Vec2::splat(-24.0), Vec2::splat(48.0)),
                        Color::WHITE,
                        tf,
                        y_layer(crown_foot),
                        &tex.tree,
                    );
                }
            }
        }
    }
}

fn draw_entities(r2d: &mut Render2D, tex: &Textures, game: &Game) {
    let p = &game.player;

    // 玩家（受伤闪红）
    let color = if p.flash_timer > 0.0 {
        Color::rgba(1.0, 0.55, 0.55, 1.0)
    } else {
        Color::WHITE
    };
    let p_foot = p.pos.y + 14.0;
    let tf = Transform2D::default().with_pos(p.pos).with_rot(p.facing_angle);
    r2d.add_sprite2d_default(
        SpriteRect::from_texture(Vec2::splat(-15.0), Vec2::splat(30.0)),
        color,
        tf,
        y_layer(p_foot),
        &tex.player,
    );

    // 史莱姆
    let ppos = game.player.pos;
    for e in &game.enemies {
        if !e.alive {
            continue;
        }
        let e_foot = e.pos.y + 16.0;
        let toward = (ppos - e.pos).y.atan2((ppos - e.pos).x);
        let squash = (e.anim * 3.0).sin();
        let tf = Transform2D::default()
            .with_pos(e.pos)
            .with_rot(toward)
            .with_scale(Vec2::new(1.0 + 0.1 * squash, 1.0 - 0.08 * squash));
        r2d.add_sprite2d_default(
            SpriteRect::from_texture(Vec2::splat(-16.0), Vec2::splat(32.0)),
            Color::WHITE,
            tf,
            y_layer(e_foot),
            &tex.slime,
        );
        // 血条（跟随实体 y-sort 层级，始终盖在本体之上一点）
        let bar_w = 32.0;
        let bar_tl = e.pos + Vec2::new(-bar_w * 0.5, -27.0);
        let frac = e.hp as f32 / 3.0;
        r2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(bar_tl, Vec2::new(bar_w, 4.0)),
            Color::rgba(0.0, 0.0, 0.0, 0.7),
            Transform2D::default(),
            y_layer(e_foot) + 0.1,
        );
        r2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(bar_tl + Vec2::new(0.5, 0.5), Vec2::new((bar_w - 1.0) * frac, 3.0)),
            Color::rgba(0.95, 0.25, 0.2, 1.0),
            Transform2D::default(),
            y_layer(e_foot) + 0.2,
        );
    }

    // 攻击弧（跟随玩家 y-sort 层级，恒高于同一 Y 的实体，形成正确遮挡）
    if p.attack_timer > 0.0 {
        draw_attack_slash(r2d, p.pos, p.facing_angle, p.attack_timer / SLASH_DURATION);
    }

    // 粒子（跟随生成点周边 y-sort，盖于实体之上）
    for pt in &game.particles {
        let t = (pt.life / pt.max_life).clamp(0.0, 1.0);
        draw_circle(r2d, pt.pos, pt.size * t, pt.color, y_layer(pt.pos.y) + 0.5);
    }
}

fn draw_ui(r2d: &mut Render2D, cam: &Camera2D, game: &Game) {
    let half_w = cam.viewport_size.x * 0.5 / cam.zoom.x;
    let half_h = cam.viewport_size.y * 0.5 / cam.zoom.y;
    let tl = cam.position - Vec2::new(half_w, half_h);

    // 面板
    r2d.add_sprite2d_default_solid(
        SpriteRect::from_texture(tl + Vec2::new(12.0, 12.0), Vec2::new(242.0, 62.0)),
        Color::rgba(0.08, 0.08, 0.14, 0.72),
        Transform2D::default(),
        LAYER_UI,
    );

    // 生命条
    let bar_pos = tl + Vec2::new(26.0, 26.0);
    let bar_wh = Vec2::new(204.0, 16.0);
    r2d.add_sprite2d_default_solid(
        SpriteRect::from_texture(bar_pos, bar_wh),
        Color::rgba(0.15, 0.0, 0.0, 1.0),
        Transform2D::default(),
        LAYER_UI + 0.5,
    );
    let frac = game.player.hp as f32 / game.player.max_hp as f32;
    if frac > 0.0 {
        let hp_color = if frac > 0.5 {
            Color::rgba(0.25, 0.9, 0.35, 1.0)
        } else {
            Color::rgba(0.95, 0.32, 0.25, 1.0)
        };
        r2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(
                bar_pos + Vec2::new(2.0, 2.0),
                Vec2::new((bar_wh.x - 4.0) * frac, bar_wh.y - 4.0),
            ),
            hp_color,
            Transform2D::default(),
            LAYER_UI + 0.6,
        );
    }

    // 金币图标 + 击杀图标（小圆 + 计数条，数字显示在标题栏）
    let coin = tl + Vec2::new(34.0, 58.0);
    draw_circle(r2d, coin + Vec2::new(5.0, 0.0), 6.0, Color::rgba(1.0, 0.82, 0.2, 1.0), LAYER_UI + 0.6);
    // 金币数量条（10 格）
    for i in 0..10 {
        let lit = game.coins > i * 3;
        r2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(coin + Vec2::new(18.0 + i as f32 * 9.0, -4.0), Vec2::new(6.0, 8.0)),
            if lit {
                Color::rgba(1.0, 0.82, 0.2, 1.0)
            } else {
                Color::rgba(1.0, 1.0, 1.0, 0.18)
            },
            Transform2D::default(),
            LAYER_UI + 0.6,
        );
    }

    // 击杀图标（红点六边形 → 圆）
    let kill = tl + Vec2::new(34.0, 78.0);
    draw_circle(r2d, kill + Vec2::new(5.0, 0.0), 6.0, Color::rgba(0.95, 0.3, 0.3, 1.0), LAYER_UI + 0.6);
    for i in 0..10 {
        let lit = game.kills > i;
        r2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(kill + Vec2::new(18.0 + i as f32 * 9.0, -4.0), Vec2::new(6.0, 8.0)),
            if lit {
                Color::rgba(0.95, 0.3, 0.3, 1.0)
            } else {
                Color::rgba(1.0, 1.0, 1.0, 0.18)
            },
            Transform2D::default(),
            LAYER_UI + 0.6,
        );
    }

    // 游戏结束覆盖
    if game.state == GameState::GameOver {
        r2d.add_sprite2d_default_solid(
            SpriteRect::from_texture(
                tl,
                Vec2::new(cam.viewport_size.x / cam.zoom.x, cam.viewport_size.y / cam.zoom.y),
            ),
            Color::rgba(0.45, 0.0, 0.0, 0.38),
            Transform2D::default(),
            LAYER_GAMEOVER,
        );
    }
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

/// 带 1px 边缘抗锯齿的实心圆。
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

/// 带抗锯齿的实心椭圆。
#[allow(clippy::too_many_arguments)]
fn fill_ellipse(
    buf: &mut [u8],
    w: usize,
    h: usize,
    center: Vec2,
    rx: f32,
    ry: f32,
    color: [u8; 4],
) {
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
    let (w, h) = (32usize, 32usize);
    let mut buf = vec![0u8; w * h * 4];
    let mut rng = Rng::new(0xA8_BCDE);
    for y in 0..h {
        for x in 0..w {
            let v = 0.9 + rng.f32() * 0.12;
            set_px(
                &mut buf,
                w,
                x,
                y,
                [
                    (0.20 * v * 255.0) as u8,
                    (0.52 * v * 255.0) as u8,
                    (0.22 * v * 255.0) as u8,
                    255,
                ],
            );
        }
    }
    buf
}

fn make_field() -> Vec<u8> {
    let (w, h) = (32usize, 32usize);
    let mut buf = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let v = 0.9 + 0.1 * ((x as f32 * 0.6 + y as f32 * 0.2).sin() * 0.5 + 0.5);
            set_px(
                &mut buf,
                w,
                x,
                y,
                [
                    (0.55 * v * 255.0) as u8,
                    (0.62 * v * 255.0) as u8,
                    (0.18 * v * 255.0) as u8,
                    255,
                ],
            );
        }
    }
    // 麦浪斜向条纹
    for y in 0..h {
        for x in 0..w {
            let stripe = (((x as i32 + y as i32) / 6) % 2) == 0;
            if !stripe {
                let i = (y * w + x) * 4;
                buf[i + 1] = buf[i + 1].saturating_sub(26);
            }
        }
    }
    buf
}

fn make_sand() -> Vec<u8> {
    let (w, h) = (32usize, 32usize);
    let mut buf = vec![0u8; w * h * 4];
    let mut rng = Rng::new(0xF15A_BC);
    for y in 0..h {
        for x in 0..w {
            let v = 0.92 + rng.f32() * 0.1;
            set_px(
                &mut buf,
                w,
                x,
                y,
                [
                    (0.85 * v * 255.0) as u8,
                    (0.78 * v * 255.0) as u8,
                    (0.52 * v * 255.0) as u8,
                    255,
                ],
            );
        }
    }
    buf
}

fn make_water() -> Vec<u8> {
    let (w, h) = (32usize, 32usize);
    let mut buf = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let shade = 0.82 + 0.18 * ((x as f32 * 0.4 + y as f32 * 0.15).sin() * 0.5 + 0.5);
            set_px(
                &mut buf,
                w,
                x,
                y,
                [
                    (30.0 * shade) as u8,
                    (82.0 * shade) as u8,
                    (178.0 * shade) as u8,
                    255,
                ],
            );
        }
    }
    // 波浪高光
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
    let (w, h) = (64usize, 64usize);
    let mut buf = vec![0u8; w * h * 4]; // 透明
    let c = Vec2::new(32.0, 30.0);
    fill_circle(&mut buf, w, h, c, 26.0, [24, 70, 30, 255]);
    fill_circle(&mut buf, w, h, c, 22.0, [44, 116, 48, 255]);
    fill_circle(&mut buf, w, h, c + Vec2::new(-6.0, -7.0), 13.0, [76, 150, 66, 255]);
    fill_circle(&mut buf, w, h, c + Vec2::new(8.0, 6.0), 9.0, [60, 138, 58, 255]);
    // 零星亮斑
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
    let (w, h) = (32usize, 32usize);
    let mut buf = vec![0u8; w * h * 4];
    let c = Vec2::new(16.0, 16.0);
    fill_circle(&mut buf, w, h, c, 14.0, [250, 208, 56, 255]);
    fill_circle(&mut buf, w, h, c + Vec2::new(-2.5, -3.0), 8.0, [255, 236, 132, 255]);
    // 眼睛朝右
    fill_circle(&mut buf, w, h, Vec2::new(17.0, 11.0), 4.2, [255, 255, 255, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(19.5, 11.0), 2.2, [24, 24, 24, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(17.0, 21.0), 4.2, [255, 255, 255, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(19.5, 21.0), 2.2, [24, 24, 24, 255]);
    buf
}

fn make_slime() -> Vec<u8> {
    let (w, h) = (32usize, 32usize);
    let mut buf = vec![0u8; w * h * 4];
    let c = Vec2::new(16.0, 18.0);
    fill_ellipse(&mut buf, w, h, c, 14.0, 11.5, [82, 196, 88, 255]);
    fill_ellipse(&mut buf, w, h, c + Vec2::new(-2.0, -2.5), 10.0, 7.5, [132, 232, 122, 255]);
    // 眼睛朝右
    fill_circle(&mut buf, w, h, Vec2::new(17.0, 13.0), 4.0, [255, 255, 255, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(19.4, 13.0), 2.1, [22, 22, 22, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(17.0, 23.0), 4.0, [255, 255, 255, 255]);
    fill_circle(&mut buf, w, h, Vec2::new(19.4, 23.0), 2.1, [22, 22, 22, 255]);
    buf
}

struct Textures {
    grass: ArcTextureWrapped,
    field: ArcTextureWrapped,
    sand: ArcTextureWrapped,
    water: ArcTextureWrapped,
    tree: ArcTextureWrapped,
    player: ArcTextureWrapped,
    slime: ArcTextureWrapped,
}

impl Textures {
    fn create(r2d: &mut Render2D) -> Self {
        Self {
            grass: r2d.create_texture("grass", &make_grass(), 32, 32),
            field: r2d.create_texture("field", &make_field(), 32, 32),
            sand: r2d.create_texture("sand", &make_sand(), 32, 32),
            water: r2d.create_texture("water", &make_water(), 32, 32),
            tree: r2d.create_texture("tree", &make_tree(), 64, 64),
            player: r2d.create_texture("player", &make_player(), 32, 32),
            slime: r2d.create_texture("slime", &make_slime(), 32, 32),
        }
    }
}

// ── App ───────────────────────────────────────────────────────────
struct RpgApp {
    render: Option<RenderContext>,
    render2d: Option<Render2D>,
    cam: Camera2D,
    tex: Option<Textures>,
    game: Game,
}

impl RpgApp {
    fn new() -> Self {
        let game = Game::new();
        let mut cam = Camera2D::new(Vec2::new(1280.0, 720.0));
        // 相机初始直接对准玩家，避免启动过渡期看到空白地图角落。
        cam.position = game.player.pos;
        Self {
            render: None,
            render2d: None,
            cam,
            tex: None,
            game,
        }
    }
}

impl App for RpgApp {
    fn primary_window_attrib(&self) -> WindowAttributes {
        WindowAttributes::default()
            .with_title("eg260731RPG")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
    }

    fn on_init(&mut self, ctx: &mut MainContext) {
        let window = ctx
            .primary_window()
            .expect("primary window must exist during on_init");
        self.render = Some(RenderContext::new(window, &RenderConfig::default()));
        let render = self.render.as_ref().expect("render initialized");

        let mut render2d = Render2D::new(render);
        let tex = Textures::create(&mut render2d);

        let (w, h) = render.size();
        let mut cam = Camera2D::new(Vec2::new(w as f32, h as f32));
        cam.set_vp(Vec2::new(w as f32, h as f32), Vec2::ZERO);
        // 相机初始对准玩家，保证开局即可看到角色。
        cam.position = self.game.player.pos;

        self.render2d = Some(render2d);
        self.cam = cam;
        self.tex = Some(tex);
    }

    fn on_resized(&mut self, _ctx: &mut MainContext, width: u32, height: u32) {
        if let Some(render) = &mut self.render {
            render.resize(width, height);
        }
        self.cam.set_vp(Vec2::new(width as f32, height as f32), Vec2::ZERO);
    }

    fn about_to_wait(&mut self, ctx: &mut MainContext) {
        if ctx.keyboard.get(KeyCode::Escape).down_edge() {
            ctx.request_exit();
        }

        let dt = ctx.timer.dt().get_f32().min(0.05);

        // 游戏逻辑更新（相机跟随在绘制前基于旧位置，误差可忽略）
        update(&mut self.game, &self.cam, ctx, dt);

        // 相机**平滑跟踪玩家**：每帧向玩家位置指数插值（快速收敛、无死区），
        // 玩家/游戏画面始终保持在窗口中心附近。出生点在地图几何中心，
        // 42×42 地图足够大，正常游玩时相机不会露出地图边界，无需 clamp。
        self.cam.position +=
            (self.game.player.pos - self.cam.position) * (1.0 - (-20.0 * dt).exp());

        let Some(render2d) = &mut self.render2d else {
            return;
        };
        let tex = self.tex.as_ref().expect("textures initialized");

        // 标题栏显示 HUD 数字
        if let Some(w) = ctx.primary_window() {
            w.set_title(&format!(
                "eg260731RPG  第 {} 波  FPS {:.0}  |  HP {}/{}  |  金币 {}  |  击杀 {}  |  WASD 移动 · 空格/左键 攻击 · R 重开 · Esc 退出",
                self.game.wave,
                ctx.timer.get_fps(),
                self.game.player.hp,
                self.game.player.max_hp,
                self.game.coins,
                self.game.kills,
            ));
        }

        render2d.set_mvp(self.cam.vp_matrix());
        draw_tiles(render2d, &self.cam, tex, &self.game);
        draw_entities(render2d, tex, &self.game);
        draw_ui(render2d, &self.cam, &self.game);

        render2d.render(&ClearConfig {
            color: Some(wgpu::Color {
                r: 0.13,
                g: 0.24,
                b: 0.12,
                a: 1.0,
            }),
            depth: None,
            stencil: None,
        });
    }
}

fn main() -> Result<(), EventLoopError> {
    env_logger::init();
    rjw_main::run_app(RpgApp::new())
}