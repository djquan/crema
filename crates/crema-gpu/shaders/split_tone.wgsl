@group(0) @binding(0) var input: texture_storage_2d<rgba32float, read>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;

struct Params {
    shadow_r: f32, shadow_g: f32, shadow_b: f32, shadow_sat: f32,
    highlight_r: f32, highlight_g: f32, highlight_b: f32, highlight_sat: f32,
    balance: f32, _pad1: f32, _pad2: f32, _pad3: f32,
}

@group(0) @binding(2) var<uniform> params: Params;

fn smoothstep_manual(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input);
    if gid.x >= dims.x || gid.y >= dims.y { return; }

    let color = textureLoad(input, vec2<i32>(vec2(gid.xy)));
    let r = color.r; let g = color.g; let b = color.b;

    let y = clamp(0.2126 * r + 0.7152 * g + 0.0722 * b, 0.0, 1.0);

    let crossover = 0.5 - params.balance / 200.0;

    let shadow_w = smoothstep_manual(crossover, 0.0, y) * params.shadow_sat;
    let highlight_w = smoothstep_manual(crossover, 1.0, y) * params.highlight_sat;

    let out_r = max(r + shadow_w * (params.shadow_r - 0.5) + highlight_w * (params.highlight_r - 0.5), 0.0);
    let out_g = max(g + shadow_w * (params.shadow_g - 0.5) + highlight_w * (params.highlight_g - 0.5), 0.0);
    let out_b = max(b + shadow_w * (params.shadow_b - 0.5) + highlight_w * (params.highlight_b - 0.5), 0.0);

    textureStore(output, vec2<i32>(vec2(gid.xy)), vec4(out_r, out_g, out_b, color.a));
}
