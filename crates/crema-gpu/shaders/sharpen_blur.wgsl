@group(0) @binding(0) var input: texture_storage_2d<rgba32float, read>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;

struct Params {
    direction: u32,      // 0 = horizontal, 1 = vertical
    kernel_radius: u32,  // taps on each side of center
    _pad1: u32,
    _pad2: u32,
    // Kernel weights follow: 20 floats max (center at [kernel_radius])
    k0: f32, k1: f32, k2: f32, k3: f32,
    k4: f32, k5: f32, k6: f32, k7: f32,
    k8: f32, k9: f32, k10: f32, k11: f32,
    k12: f32, k13: f32, k14: f32, k15: f32,
    k16: f32, k17: f32, k18: f32, k19: f32,
}
@group(0) @binding(2) var<uniform> params: Params;

fn kernel_weight(i: u32) -> f32 {
    switch i {
        case 0u: { return params.k0; }
        case 1u: { return params.k1; }
        case 2u: { return params.k2; }
        case 3u: { return params.k3; }
        case 4u: { return params.k4; }
        case 5u: { return params.k5; }
        case 6u: { return params.k6; }
        case 7u: { return params.k7; }
        case 8u: { return params.k8; }
        case 9u: { return params.k9; }
        case 10u: { return params.k10; }
        case 11u: { return params.k11; }
        case 12u: { return params.k12; }
        case 13u: { return params.k13; }
        case 14u: { return params.k14; }
        case 15u: { return params.k15; }
        case 16u: { return params.k16; }
        case 17u: { return params.k17; }
        case 18u: { return params.k18; }
        case 19u: { return params.k19; }
        default: { return 0.0; }
    }
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }

    let kr = i32(params.kernel_radius);
    let kernel_size = kr * 2 + 1;

    var acc = vec3<f32>(0.0);
    for (var i = 0; i < kernel_size; i++) {
        let offset = i - kr;
        var sample_pos: vec2<i32>;
        if params.direction == 0u {
            // Horizontal
            let sx = clamp(i32(gid.x) + offset, 0, i32(dims.x) - 1);
            sample_pos = vec2<i32>(sx, i32(gid.y));
        } else {
            // Vertical
            let sy = clamp(i32(gid.y) + offset, 0, i32(dims.y) - 1);
            sample_pos = vec2<i32>(i32(gid.x), sy);
        }
        let pixel = textureLoad(input, sample_pos);
        let w = kernel_weight(u32(i));
        acc += pixel.rgb * w;
    }

    textureStore(output, gid.xy, vec4<f32>(acc, 1.0));
}
