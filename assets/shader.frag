// 本地部署版片元着色器
// 动漫风卡通明暗（参考 pixiv/three-vrm 的 MToon：getShading / getDiffuse 的二值化思路）
//
// 颜色管线（"颜色不对"的关键）：
//   贴图以 SRGB8_ALPHA8 上传 -> texture() 采样得到**线性**颜色
//   光照在**线性空间**算 -> 最后手动 linear -> sRGB 编码输出
//   跳过这两步（当线性用、直接输出）会把中间调抬高，画面发白、发灰
//
// 所有调色参数都是从 Rust 端传进来的 uniform，可用环境变量改：
//   COLONQ_EXPOSURE  亮度倍率           默认 1.0
//   COLONQ_SATURATION 饱和度            默认 1.15
//   COLONQ_TOONY     明暗分界硬度       默认 0.90
//   COLONQ_SHIFT     整体明暗偏移       默认 0.02
//   COLONQ_SHADE_TINT 暗部色调          默认 0.82,0.80,0.88
//   COLONQ_SHADE_LIFT 暗部提亮          默认 0.0
//   COLONQ_RIM       边缘光强度         默认 0.06
#version 330 core

uniform sampler2D base_color_tex;   // 绑在 TEXTURE0
uniform vec4 base_color;            // glTF baseColorFactor（每个材质不同）
uniform vec3 camera_pos;

uniform float exposure;
uniform float saturation;
uniform float contrast;   // 对比度：围绕中灰拉伸明暗（1.0 = 不变）
uniform vec3 skin_tint;   // 皮肤/脸部染色（部位 1）
uniform float skin_sat;   // 皮肤饱和度倍率
uniform float shade_lift;
uniform float shading_toony;
uniform float shading_shift;
uniform vec3 shade_tint;
uniform float rim_strength;
uniform vec3 outline_color;    // 描边颜色
uniform float outline_glow;    // 描边发光强度（0 = 普通描边，>0 = 加色发光）
uniform int outline_pass;      // 1 = 正在画描边壳
uniform int part_pass;         // 1 = 正在画"部位编号"（写进 alpha 通道）
uniform float clip_y;          // 世界 Y 裁剪面：低于它的片元丢弃（只渲染上半身用）
uniform float part_id;         // 当前材质的部位编号 0..1

in vec2 v_uv;
in vec3 v_normal;
in vec3 v_world_pos;

out vec4 frag_color;

const vec3 LIGHT_DIR = vec3(0.30, 0.72, 0.62);  // 主光方向（世界空间）

float linearstep(float a, float b, float t) {
    return clamp((t - a) / (b - a), 0.0, 1.0);
}

vec3 to_srgb(vec3 c) {
    return pow(clamp(c, 0.0, 1.0), vec3(1.0 / 2.2));
}

void main() {
    // 只渲染上半身：把裁剪面以下的片元全部丢掉（描边/部位 pass 也一起裁掉）
    if (v_world_pos.y < clip_y) {
        discard;
    }

    // 描边壳：颜色按 glow 倍数大幅提亮（过曝产生发光感）
    // glow=2.5 时颜色乘 16 倍，输出远超 1.0，OpenGL 会截断到纯白——发光感强烈
    if (outline_pass == 1) {
        vec3 oc = outline_color * (1.0 + outline_glow * 6.0);
        frag_color = vec4(oc, 1.0);
        return;
    }

    // 部位编号 pass：只有 alpha 会被写入（颜色通道被 ColorMask 关掉了）
    if (part_pass == 1) {
        frag_color = vec4(0.0, 0.0, 0.0, part_id);
        return;
    }

    vec4 tex = texture(base_color_tex, v_uv) * base_color;

    if (tex.a < 0.08) discard;                    // 头发/睫毛/眉毛的 alpha 抠图

    vec3 n = normalize(v_normal);
    if (!gl_FrontFacing) n = -n;                  // 双面材质

    // --- MToon 式卡通明暗：N·L 经 toony/shift 变成硬边明暗因子 ---
    float dotNL   = dot(n, normalize(LIGHT_DIR));
    float shading = linearstep(-1.0 + shading_toony, 1.0 - shading_toony, dotNL + shading_shift);

    // 皮肤/脸部（部位编号 1）单独染色：先把颜色调暖，再按需提饱和度
    vec3 base = tex.rgb;
    float pid = floor(part_id * 255.0 + 0.5);
    if (pid == 1.0) {
        base *= skin_tint;
        float sl = dot(base, vec3(0.2126, 0.7152, 0.0722));
        base = mix(vec3(sl), base, skin_sat);
    }

    vec3 lit   = base;
    vec3 shade = base * shade_tint + vec3(shade_lift);
    vec3 col   = mix(shade, lit, shading);

    // --- 边缘光（克制）---
    vec3 view_dir = normalize(camera_pos - v_world_pos);
    float rim = pow(1.0 - clamp(dot(n, view_dir), 0.0, 1.0), 4.0);
    col += vec3(rim * rim_strength);

    // --- 曝光 / 对比度 / 饱和度（线性空间做）---
    col *= exposure;
    col = (col - 0.18) * contrast + 0.18;   // 0.18 是线性空间的中灰
    float lum = dot(col, vec3(0.2126, 0.7152, 0.0722));
    col = mix(vec3(lum), col, saturation);

    frag_color = vec4(to_srgb(col), tex.a);
}
