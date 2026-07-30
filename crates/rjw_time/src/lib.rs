use std::time::{self, Duration};

pub const DT_MAX: f64 = 0.1;

#[derive(Debug, Clone, Copy, Default)]
pub struct DeltaTime {
    dt_dur: Duration,
    dt_f32: f32,
    dt_f64: f64,
}

impl DeltaTime {
    pub fn set(&mut self, dt_dur: Duration) {
        self.dt_dur = dt_dur;
        self.dt_f64 = dt_dur.as_secs_f64();
        self.dt_f32 = dt_dur.as_secs_f32();
    }

    pub fn get_f32(&self) -> f32 {
        self.dt_f32
    }

    pub fn get_f64(&self) -> f64 {
        self.dt_f64
    }
}

trait Get<T> {
    fn get(&self) -> T;
}

impl Get<f32> for DeltaTime {
    fn get(&self) -> f32 {
        self.dt_f32
    }
}

impl Get<f64> for DeltaTime {
    fn get(&self) -> f64 {
        self.dt_f64
    }
}

#[derive(Debug)]
pub struct DeltaTimer {
    frame_stamp: time::Instant,
    fps: f64,
    dt: DeltaTime,
}

impl Default for DeltaTimer {
    fn default() -> Self {
        Self {
            frame_stamp: time::Instant::now(),
            fps: 0.0,
            dt: DeltaTime::default(),
        }
    }
}

impl DeltaTimer {
    /// Current smoothed FPS value (updated every frame via EMA).
    /// 当前平滑后的 FPS 值（每帧通过 EMA 更新）。
    pub fn get_fps(&self) -> f64 {
        self.fps
    }

    /// Compute delta time since last call and advance the frame stamp.
    /// 计算距上次调用的帧间隔，并更新帧时间戳。
    pub fn per_frame(&mut self) {
        let now = time::Instant::now();
        let dt_dur = now - self.frame_stamp;
        self.dt.set(dt_dur);

        // FPS

        // Update the smoothed FPS using exponential moving average (EMA).
        // 使用指数移动平均（EMA）更新平滑 FPS。
        //
        // Takes the same `dt` that was returned by `pre_frame_and_get_delta_time`
        // (possibly clamped) so no extra `Instant::now()` call is needed.
        // 使用与 `pre_frame_and_get_delta_time` 相同的 `dt`（可能已被 clamp），
        // 避免重复取时间戳。
        //
        // The EMA constant α = 0.1 provides a good balance between
        // responsiveness and smoothness.
        // EMA 常数 α = 0.1，在响应速度和平滑度之间取得良好平衡。
        let dt: f64 = self.dt.get();
        let alpha = 0.1;
        let instant_fps = 1.0 / dt.max(1e-10);
        self.fps = self.fps * (1.0 - alpha) + instant_fps * alpha;

        self.frame_stamp = now;
    }

    pub fn dt(&self) -> &DeltaTime {
        &self.dt
    }
}