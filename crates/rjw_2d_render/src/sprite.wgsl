struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    // ── Instance 属性 ──
    @location(3) mesh_tl: vec2<f32>,
    @location(4) mesh_wh: vec2<f32>,
    @location(5) uv_tl: vec2<f32>,
    @location(6) uv_wh: vec2<f32>,
    @location(7) color_i: vec4<f32>,
    // WGSL 不允许 mat4x4 作为入口点输入，拆分为 4 个列向量（列主序）。
    @location(8) model_c0: vec4<f32>,
    @location(9) model_c1: vec4<f32>,
    @location(10) model_c2: vec4<f32>,
    @location(11) model_c3: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> vp: mat4x4<f32>;

@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    // 局部网格坐标 → 实例的 mesh 范围
    let mesh_pos = in.mesh_tl + in.pos.xy * in.mesh_wh;

    // 4 个列向量组合为列主序矩阵
    let model = mat4x4<f32>(in.model_c0, in.model_c1, in.model_c2, in.model_c3);

    var out: VertexOut;
    out.clip_pos = vp * model * vec4<f32>(mesh_pos, 0.0, 1.0);
    // 像素 UV → [0, 1) 区间
    out.uv = in.uv_tl + in.uv * in.uv_wh;
    out.color = in.color * in.color_i;
    return out;
}

/// Mesh/Polygon 非实例化顶点输入（仅 slot0 三属性，避免要求 location 3..11 绑定）。
struct MeshVertex {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

/// Mesh/Polygon 非实例化顶点入口：
/// 顶点 `pos` 即**世界坐标**，直接经 VP 输出；颜色来自顶点内置（per-vertex color）。
@vertex
fn vs_mesh(in: MeshVertex) -> VertexOut {
    var out: VertexOut;
    out.clip_pos = vp * vec4<f32>(in.pos, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let sampled = textureSample(tex, samp, in.uv);
    return sampled * in.color;
}