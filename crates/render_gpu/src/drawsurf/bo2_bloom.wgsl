// bo2zm: Black Ops II's bloom (see bo2_bloom.rs): the frame's bright parts
// at quarter size, blurred, put back as BO2's hdr_bloom_apply does:
// sqrt(scene^2 + strength * bloom^2).

struct BloomParams {
    // xy: one source texel in uv; zw: the blur direction (in texels).
    texel_dir: vec4<f32>,
    // x, y: the luminance where the bright pass starts and is full;
    // z: the strength bloom goes back in with.
    threshold: vec4<f32>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> params: BloomParams;
@group(0) @binding(3) var bloom: texture_2d<f32>;

struct FullscreenOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenOut {
    let uv = vec2(f32((index << 1u) & 2u), f32(index & 2u));
    var out: FullscreenOut;
    out.position = vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

// The bright pass, a 4x4 block of the full frame per quarter texel (four
// bilinear taps), weighted by a smooth threshold on its luminance.
@fragment
fn extract(in: FullscreenOut) -> @location(0) vec4<f32> {
    let t = params.texel_dir.xy;
    var c = vec3(0.0);
    for (var i = 0; i < 4; i++) {
        let o = vec2(f32(i & 1) * 2.0 - 1.0, f32(i >> 1u) * 2.0 - 1.0) * t;
        c += textureSample(source, source_sampler, in.uv + o).rgb;
    }
    c *= 0.25;
    let luma = dot(c, vec3(0.212585, 0.715195, 0.07222));
    let w = smoothstep(params.threshold.x, params.threshold.y, luma);
    return vec4(c * w, 1.0);
}

// A 9-tap gaussian along one direction.
@fragment
fn blur(in: FullscreenOut) -> @location(0) vec4<f32> {
    let step = params.texel_dir.xy * params.texel_dir.zw;
    let w = array<f32, 5>(0.2270270, 0.1945946, 0.1216216, 0.0540541, 0.0162162);
    var c = textureSample(source, source_sampler, in.uv).rgb * w[0];
    for (var i = 1; i < 5; i++) {
        let o = step * f32(i);
        c += textureSample(source, source_sampler, in.uv + o).rgb * w[i];
        c += textureSample(source, source_sampler, in.uv - o).rgb * w[i];
    }
    return vec4(c, 1.0);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3(0.299, 0.587, 0.114));
}

// bo2zm: his FXAA row: the classic fast approximate anti-aliasing (four
// diagonal taps find the edge, two to four taps along it blend it).
fn fxaa(uv: vec2<f32>, t: vec2<f32>) -> vec3<f32> {
    let m = textureSample(source, source_sampler, uv).rgb;
    let nw = textureSample(source, source_sampler, uv + vec2(-1.0, -1.0) * t).rgb;
    let ne = textureSample(source, source_sampler, uv + vec2(1.0, -1.0) * t).rgb;
    let sw = textureSample(source, source_sampler, uv + vec2(-1.0, 1.0) * t).rgb;
    let se = textureSample(source, source_sampler, uv + vec2(1.0, 1.0) * t).rgb;
    let l_m = luma(m);
    let l_nw = luma(nw);
    let l_ne = luma(ne);
    let l_sw = luma(sw);
    let l_se = luma(se);
    let l_min = min(l_m, min(min(l_nw, l_ne), min(l_sw, l_se)));
    let l_max = max(l_m, max(max(l_nw, l_ne), max(l_sw, l_se)));
    var dir = vec2(-((l_nw + l_ne) - (l_sw + l_se)), (l_nw + l_sw) - (l_ne + l_se));
    let reduce = max((l_nw + l_ne + l_sw + l_se) * (0.25 / 8.0), 1.0 / 128.0);
    let rcp_min = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * rcp_min, vec2(-8.0), vec2(8.0)) * t;
    let a = 0.5 * (textureSample(source, source_sampler, uv + dir * (1.0 / 3.0 - 0.5)).rgb
        + textureSample(source, source_sampler, uv + dir * (2.0 / 3.0 - 0.5)).rgb);
    let b = a * 0.5 + 0.25 * (textureSample(source, source_sampler, uv - dir * 0.5).rgb
        + textureSample(source, source_sampler, uv + dir * 0.5).rgb);
    let l_b = luma(b);
    if (l_b < l_min || l_b > l_max) {
        return a;
    }
    return b;
}

// Back into the frame as BO2 does, then BO2's brightness: the picture
// raised to 1 / r_gamma (threshold.w; 1 = as made). texel_dir.z: FXAA.
@fragment
fn composite(in: FullscreenOut) -> @location(0) vec4<f32> {
    var s = textureSample(source, source_sampler, in.uv).rgb;
    if (params.texel_dir.z > 0.5) {
        s = fxaa(in.uv, params.texel_dir.xy);
    }
    let b = textureSample(bloom, source_sampler, in.uv).rgb;
    let c = sqrt(s * s + params.threshold.z * b * b);
    return vec4(pow(c, vec3(1.0 / max(params.threshold.w, 0.1))), 1.0);
}

// bo2zm M4: the menus' world blur put back in the frame's place
// (threshold.w: how much).
@fragment
fn blurred(in: FullscreenOut) -> @location(0) vec4<f32> {
    let s = textureSample(source, source_sampler, in.uv).rgb;
    let b = textureSample(bloom, source_sampler, in.uv).rgb;
    return vec4(mix(s, b, params.threshold.w), 1.0);
}
