// 本地部署版顶点着色器
// 接口固定（由 src/shader.rs / src/vrm.rs / src/utils.rs 决定，不能改名）：
//   属性 0=vertex 1=normal 2=texcoord 3=joint 4=weight
//   属性 5..15 = expression_{happy..blink}_vertex（morph target 的顶点偏移量）
//   uniform: view / projection / position / normal_matrix / joint_matrices[MAX_JOINTS] /
//            expression_{happy..blink}_weight
#version 330 core

layout(location = 0) in vec3 vertex;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 texcoord;
layout(location = 3) in vec4 joint;
layout(location = 4) in vec4 weight;

layout(location = 5)  in vec3 expression_happy_vertex;
layout(location = 6)  in vec3 expression_angry_vertex;
layout(location = 7)  in vec3 expression_sad_vertex;
layout(location = 8)  in vec3 expression_relaxed_vertex;
layout(location = 9)  in vec3 expression_surprised_vertex;
layout(location = 10) in vec3 expression_aa_vertex;
layout(location = 11) in vec3 expression_ih_vertex;
layout(location = 12) in vec3 expression_ou_vertex;
layout(location = 13) in vec3 expression_ee_vertex;
layout(location = 14) in vec3 expression_oh_vertex;
layout(location = 15) in vec3 expression_blink_vertex;

uniform mat4 view;
uniform mat4 projection;
uniform mat4 position;
uniform mat4 normal_matrix;
#ifndef MAX_JOINTS
#define MAX_JOINTS 128
#endif
uniform mat4 joint_matrices[MAX_JOINTS];
uniform vec3 camera_pos;
uniform float outline_width;   // 描边外扩量（世界单位），0 = 不描边
uniform int outline_pass;      // 1 = 正在画描边壳

uniform float expression_happy_weight;
uniform float expression_angry_weight;
uniform float expression_sad_weight;
uniform float expression_relaxed_weight;
uniform float expression_surprised_weight;
uniform float expression_aa_weight;
uniform float expression_ih_weight;
uniform float expression_ou_weight;
uniform float expression_ee_weight;
uniform float expression_oh_weight;
uniform float expression_blink_weight;

out vec2 v_uv;
out vec3 v_normal;
out vec3 v_world_pos;

void main() {
    // 1) blend shape：morph target 存的是偏移量，按权重叠加
    vec3 p = vertex;
    p += expression_happy_vertex     * expression_happy_weight;
    p += expression_angry_vertex     * expression_angry_weight;
    p += expression_sad_vertex       * expression_sad_weight;
    p += expression_relaxed_vertex   * expression_relaxed_weight;
    p += expression_surprised_vertex * expression_surprised_weight;
    p += expression_aa_vertex        * expression_aa_weight;
    p += expression_ih_vertex        * expression_ih_weight;
    p += expression_ou_vertex        * expression_ou_weight;
    p += expression_ee_vertex        * expression_ee_weight;
    p += expression_oh_vertex        * expression_oh_weight;
    p += expression_blink_vertex     * expression_blink_weight;

    // 2) 决定用哪个矩阵把顶点送到世界空间
    //
    //    注意：joint_matrices[i] = global(joint_i) * inverseBindMatrix_i，
    //    其中 global(joint_i) 已经包含了根节点（也就是本项目的「模型矩阵」）。
    //    所以蒙皮顶点只能再乘 skin，绝不能再乘一次 position —— 否则模型变换会平方。
    //    非蒙皮顶点（没有权重）才用 position。
    float wsum = weight.x + weight.y + weight.z + weight.w;

    mat4 world_mat;
    if (wsum < 0.0001) {
        world_mat = position;
    } else {
        // 索引夹紧到数组范围内，避免模型骨骼数超过 MAX_JOINTS 时读到越界数据
        int j0 = clamp(int(joint.x + 0.5), 0, MAX_JOINTS - 1);
        int j1 = clamp(int(joint.y + 0.5), 0, MAX_JOINTS - 1);
        int j2 = clamp(int(joint.z + 0.5), 0, MAX_JOINTS - 1);
        int j3 = clamp(int(joint.w + 0.5), 0, MAX_JOINTS - 1);
        world_mat = (weight.x * joint_matrices[j0] +
                     weight.y * joint_matrices[j1] +
                     weight.z * joint_matrices[j2] +
                     weight.w * joint_matrices[j3]) / wsum;
    }

    // 描边：沿法线外扩顶点（MToon 的 inverted hull 做法）
    if (outline_pass == 1) {
        p += normal * outline_width;
    }

    vec4 world = world_mat * vec4(p, 1.0);

    v_uv = texcoord;
    // 本项目缩放是均匀的，直接用 3x3 部分变换法线即可
    v_normal = normalize(mat3(world_mat) * normal);
    v_world_pos = world.xyz;

    gl_Position = projection * view * world;
}
