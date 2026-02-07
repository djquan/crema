@group(0) @binding(0) var input: texture_storage_2d<rgba32float, read>;
@group(0) @binding(1) var output: texture_storage_2d<rgba32float, write>;

// 3x3 Bradford CAT matrix passed as three vec4 rows (w components unused).
// Matches the CPU pipeline's wb_matrix() output.
struct Params {
    row0: vec4<f32>,
    row1: vec4<f32>,
    row2: vec4<f32>,
}

@group(0) @binding(2) var<uniform> params: Params;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input);
    if gid.x >= dims.x || gid.y >= dims.y {
        return;
    }

    let color = textureLoad(input, vec2<i32>(gid.xy));
    let rgb = color.rgb;
    let adjusted = vec4<f32>(
        max(dot(params.row0.xyz, rgb), 0.0),
        max(dot(params.row1.xyz, rgb), 0.0),
        max(dot(params.row2.xyz, rgb), 0.0),
        color.a,
    );
    textureStore(output, vec2<i32>(gid.xy), adjusted);
}
