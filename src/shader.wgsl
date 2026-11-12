// Снежинки — инстансированные квадраты с кругом, сглаженным во фрагментном шейдере.
// Сугробы — обычные треугольники с цветом в вершинах.

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

// ---- снежинки ----

struct FlakeOut {
    @builtin(position) pos: vec4f,
    @location(0) offset: vec2f,   // смещение от центра, физические пиксели
    @location(1) ra: vec2f,       // радиус в физических пикселях, прозрачность
}

@vertex
fn vs_flake(
    @builtin(vertex_index) vi: u32,
    @location(0) center: vec2f,
    @location(1) ra: vec2f,
) -> FlakeOut {
    var corners = array<vec2f, 6>(
        vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(-1.0, 1.0),
        vec2f(-1.0, 1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
    );
    let ext = ra.x + 1.0 / g.scale;   // запас в один пиксель под сглаживание
    let d = corners[vi] * ext;
    var out: FlakeOut;
    out.pos = to_clip(center + d);
    out.offset = d * g.scale;
    out.ra = vec2f(ra.x * g.scale, ra.y);
    return out;
}

@fragment
fn fs_flake(in: FlakeOut) -> @location(0) vec4f {
    let a = in.ra.y * clamp(in.ra.x - length(in.offset) + 0.5, 0.0, 1.0);
    if a <= 0.002 {
        discard;
    }
    return out_color(vec3f(1.0), a);
}

// ---- сугробы ----

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
