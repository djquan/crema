@group(0) @binding(0) var input: texture_storage_2d<rgba32float, read>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;

struct Params {
    lut_size: u32,
    lut_top: f32,
    lut_slope: f32,
    _pad: f32,
}

@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var<storage, read> lut: array<f32>;

fn lut_lerp(y: f32) -> f32 {
    let idx_f = y * f32(params.lut_size - 1u);
    let i0 = min(u32(idx_f), params.lut_size - 2u);
    let frac = idx_f - f32(i0);
    return lut[i0] * (1.0 - frac) + lut[i0 + 1u] * frac;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input);
    if gid.x >= dims.x || gid.y >= dims.y { return; }

    let color = textureLoad(input, vec2<i32>(vec2(gid.xy)));
    let r = color.r; let g = color.g; let b = color.b;

    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if y < 1e-6 {
        textureStore(output, vec2<i32>(vec2(gid.xy)), color);
        return;
    }

    var scale: f32;
    if y <= 1.0 {
        scale = lut_lerp(y) / y;
    } else {
        let new_y = params.lut_top + params.lut_slope * (y - 1.0);
        scale = max(new_y / y, 0.0);
    }

    textureStore(output, vec2<i32>(vec2(gid.xy)), vec4(
        max(r * scale, 0.0),
        max(g * scale, 0.0),
        max(b * scale, 0.0),
        color.a
    ));
}
