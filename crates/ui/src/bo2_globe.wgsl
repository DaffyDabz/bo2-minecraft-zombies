// bo2zm M4: Black Ops II's Zombies globe (technique
// sw4_2d_globe_game_controlled, ui_zm.ff), its pixel shader as BO2 runs it:
// the picture's square is a sphere seen face on, turned by the element's
// shader vector 2 (x, y, z in radians), mapped with the Earth's day map and
// the mesh (the glowing grid), each revealed from the top by shader vector
// 0 (x: the mesh, y: the day map).
#import bevy_ui::ui_vertex_output::UiVertexOutput

struct Globe {
    // Shader vectors 0 and 2 (setShaderVector) and the element's colour.
    v0: vec4<f32>,
    v2: vec4<f32>,
    color: vec4<f32>,
}

@group(1) @binding(0) var<uniform> globe: Globe;
@group(1) @binding(1) var mesh_tex: texture_2d<f32>;
@group(1) @binding(2) var mesh_smp: sampler;
@group(1) @binding(3) var day_tex: texture_2d<f32>;
@group(1) @binding(4) var day_smp: sampler;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    // BO2 reads the texture coordinates crossed (x from v, y from u).
    let x = in.uv.y * 2.0 - 1.0;
    let y = in.uv.x * 2.0 - 1.0;
    let zz = 1.0 - y * y - x * x;
    if zz < 0.0 {
        return vec4<f32>(0.0);
    }
    let w = sqrt(zz);
    // The turn about the view axis (z), then x, then y.
    let sz = sin(globe.v2.z);
    let cz = cos(globe.v2.z);
    let y1 = y * cz - x * sz;
    let x1 = x * cz + y * sz;
    let sx = sin(globe.v2.x);
    let cx = cos(globe.v2.x);
    let up = x1 * cx - w * sx;
    let x2 = x1 * sx + w * cx;
    let sy = sin(globe.v2.y);
    let cy = cos(globe.v2.y);
    let zf = y1 * cy - x2 * sy;
    let xf = y1 * sy + x2 * cy;
    // Longitude and latitude on the maps.
    let uv = vec2<f32>(atan2(zf, xf) * 0.159155 + 0.5, acos(clamp(-up, -1.0, 1.0)) * 0.318310);
    var mesh = textureSample(mesh_tex, mesh_smp, uv);
    let day = textureSample(day_tex, day_smp, uv);
    // The rim fades; each map shows below its reveal line.
    let rim = min(w * w * 20.0, 1.0);
    let h = 1.0 - up;
    let mesh_on = (1.0 - clamp((h * 0.5 - globe.v0.x) * 20.0, 0.0, 1.0)) * rim;
    let day_on = clamp((h * 0.5 - (1.0 - globe.v0.y)) * 20.0, 0.0, 1.0) * rim;
    mesh.a = mesh.a * mesh_on;
    let day_a = day_on * day.a;
    let mesh_pm = mesh * mesh.a;
    var out = (1.0 - day_a) * mesh_pm + day_a * vec4<f32>(day.rgb, day_a);
    if out.a > 0.0 {
        out = vec4<f32>(out.rgb / out.a, out.a);
    }
    return out * globe.color;
}
