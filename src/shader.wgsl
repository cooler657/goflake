// Частицы и небо — инстансированные квадраты; форма (крупинка, штрих, лист, свечение, луна) считается
// и сглаживается во фрагментном шейдере. Кучи — треугольники с цветом в вершинах.

struct Globals {
    size: vec2f,    // размер экрана в логических точках
    scale: f32,     // физических пикселей на точку
    premul: f32,    // 1 — поверхность ждёт цвет, умноженный на альфу
}

@group(0) @binding(0) var<uniform> g: Globals;

fn to_clip(p: vec2f) -> vec4f {
    return vec4f(p.x / g.size.x * 2.0 - 1.0, 1.0 - p.y / g.size.y * 2.0, 0.0, 1.0);
}

fn out_color(rgb: vec3f, a: f32) -> vec4f {
    if g.premul > 0.5 {
        return vec4f(rgb * a, a);
    }
    return vec4f(rgb, a);
}

// ---- частицы и небо ----

struct FlakeOut {
    @builtin(position) pos: vec4f,
    @location(0) local: vec2f,    // точка в осях фигуры, физические пиксели
    @location(1) half: vec2f,     // полуразмеры в физических пикселях
    @location(2) shape: f32,      // 0 — крупинка или штрих, 1 — лист, 2 — свечение, 3 — луна
    @location(3) color: vec4f,
    @location(4) param: f32,      // фаза луны
}

@vertex
fn vs_flake(
    @builtin(vertex_index) vi: u32,
    @location(0) center: vec2f,
    @location(1) half: vec2f,
    @location(2) rot_shape: vec3f,   // поворот, вид фигуры, параметр
    @location(3) color: vec4f,
) -> FlakeOut {
    var corners = array<vec2f, 6>(
        vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(-1.0, 1.0),
        vec2f(-1.0, 1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
    );
    let d = corners[vi] * (half + vec2f(1.0 / g.scale));   // запас в пиксель под сглаживание
    let c = cos(rot_shape.x);
    let s = sin(rot_shape.x);
    var out: FlakeOut;
    out.pos = to_clip(center + vec2f(d.x * c - d.y * s, d.x * s + d.y * c));
    out.local = d * g.scale;
    out.half = half * g.scale;
    out.shape = rot_shape.y;
    out.param = rot_shape.z;
    out.color = color;
    return out;
}

@fragment
fn fs_flake(in: FlakeOut) -> @location(0) vec4f {
    let a = in.half.x;
    let b = in.half.y;
    var cover: f32;
    var rgb = in.color.rgb;
    if in.shape < 0.5 {
        // капсула: при a == b это круг, при a > b — штрих со скруглёнными концами
        let q = vec2f(max(abs(in.local.x) - (a - b), 0.0), in.local.y);
        cover = clamp(b - length(q) + 0.5, 0.0, 1.0);
    } else if in.shape < 1.5 {
        // лист: линза с острыми концами и тёмной прожилкой посередине
        let t = clamp(in.local.x / a, -1.0, 1.0);
        let f = b * (1.0 - t * t) - abs(in.local.y);
        let k = 2.0 * b * t / a;
        let grad = sqrt(1.0 + k * k);
        cover = clamp(f / grad + 0.5, 0.0, 1.0);
        rgb *= mix(0.72, 1.0, smoothstep(0.3, 1.3, abs(in.local.y)));
    } else if in.shape < 2.5 {
        // свечение: яркость падает от центра к краю по квадрату
        let gl = clamp(1.0 - length(in.local) / a, 0.0, 1.0);
        cover = gl * gl;
    } else {
        // луна: диск, освещённый до терминатора (половины эллипса)
        let disk = clamp(a - length(in.local) + 0.5, 0.0, 1.0);
        let v = clamp(in.local.y / a, -1.0, 1.0);
        let w = sqrt(max(1.0 - v * v, 0.0));
        let c = cos(6.2831853 * in.param);
        var edge: f32;
        if in.param < 0.5 {
            edge = in.local.x - c * w * a;
        } else {
            edge = -c * w * a - in.local.x;
        }
        cover = disk * clamp(edge + 0.5, 0.0, 1.0);
    }
    let alpha = in.color.a * cover;
    if alpha <= 0.002 {
        discard;
    }
    return out_color(rgb, alpha);
}

// ---- кучи ----

struct PileOut {
    @builtin(position) pos: vec4f,
    @location(0) color: vec4f,
}

@vertex
fn vs_pile(@location(0) pos: vec2f, @location(1) color: vec4f) -> PileOut {
    var out: PileOut;
    out.pos = to_clip(pos);
    out.color = color;
    return out;
}

@fragment
fn fs_pile(in: PileOut) -> @location(0) vec4f {
    return out_color(in.color.rgb, in.color.a);
}
