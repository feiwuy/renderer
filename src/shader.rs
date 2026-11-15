use crate::{context, gl, utils};

use std::collections::HashMap;

#[allow(dead_code)]
pub struct Shader {
    pub prog: gl::types::GLuint,
    pub uniform_view: gl::types::GLint,
    pub uniform_position: gl::types::GLint,
    pub uniform_projection: gl::types::GLint,
    pub uniform_normal: gl::types::GLint,
    pub uniform_camera_pos: gl::types::GLint,
    pub uniform_base_color: gl::types::GLint,
    pub uniform_exposure: gl::types::GLint,
    pub uniform_saturation: gl::types::GLint,
    pub uniform_contrast: gl::types::GLint,
    pub uniform_skin_tint: gl::types::GLint,
    pub uniform_skin_sat: gl::types::GLint,
    pub uniform_lift: gl::types::GLint,
    pub uniform_toony: gl::types::GLint,
    pub uniform_shift: gl::types::GLint,
    pub uniform_shade_tint: gl::types::GLint,
    pub uniform_rim: gl::types::GLint,
    pub uniform_outline_width: gl::types::GLint,
    pub uniform_outline_color: gl::types::GLint,
    pub uniform_outline_pass: gl::types::GLint,
    pub uniform_outline_glow: gl::types::GLint,
    pub uniform_part_id: gl::types::GLint,
    pub uniform_part_pass: gl::types::GLint,
    /// 是否启用"按部位分表"（COLONQ_PARTS=1）
    pub parts_enabled: bool,
    /// 描边外扩量（0 = 关闭），render_node 用它决定是否多画一遍
    pub outline_width: f32,

    pub uniform_joint_matrices: gl::types::GLint,
    /// joint_matrices 数组大小：NVIDIA 顶点常量寄存器只有 1024 个 vec4，
    /// 一个 mat4 吃 4 个，所以 256 会正好吃满并导致链接失败（error C6020）
    pub max_joints: i32,
    /// 世界 Y 方向的裁剪面：低于它的片元直接丢弃（用来只渲染上半身）
    pub clip_y: f32,
    pub uniform_clip_y: gl::types::GLint,

    pub uniform_expressions: HashMap<String, gl::types::GLint>,
}

impl Shader {
    fn check_compile_error(path: &str, shader: gl::types::GLuint) {
        unsafe {
            let mut success: gl::types::GLint = 0;
            gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success as *mut _);
            if success == 0 {
                let mut buf: [i8; 512] = [0; 512];
                gl::GetShaderInfoLog(shader, 512, 0 as _, &mut buf as *mut i8);
                let msg: &std::ffi::CStr = std::ffi::CStr::from_ptr(&buf as *const i8);
                let strmsg = String::from_utf8_lossy(msg.to_bytes()).to_string();
                panic!("shader compile error for {}: {}", path, strmsg);
            }
        }
    }

    fn check_link_error(prog: gl::types::GLuint) {
        unsafe {
            let mut success: gl::types::GLint = 0;
            gl::GetProgramiv(prog, gl::LINK_STATUS, &mut success as *mut _);
            if success == 0 {
                let mut buf: [i8; 512] = [0; 512];
                gl::GetProgramInfoLog(prog, 512, 0 as _, &mut buf as *mut i8);
                let msg: &std::ffi::CStr = std::ffi::CStr::from_ptr(&buf as *const i8);
                let strmsg = String::from_utf8_lossy(msg.to_bytes()).to_string();
                panic!("shader link error: {}", strmsg);
            }
        }
    }

    pub fn new(
        ctx: &context::Context,
        vtxpath: &str,
        fragpath: &str,
        joints_hint: i32,
        clip_y: f32,
    ) -> Self {
        unsafe {
            let prog = gl::CreateProgram();

            // 优先用环境变量；否则用模型实际的骨骼数（向 4 对齐），夹在 32..=256
            let max_joints: i32 = utils::env_parse::<i32>("COLONQ_MAX_JOINTS")
                .unwrap_or_else(|| ((joints_hint + 3) / 4 * 4).max(32))
                .clamp(16, 256);
            log::info!(
                "joint_matrices 数组大小: {}（{} 个常量寄存器 / NVIDIA 上限 1024）",
                max_joints,
                max_joints * 4
            );
            // 把骨骼数组长度以宏的形式注入，避免吃光 NVIDIA 的常量寄存器
            let define = format!("#version 330 core\n#define MAX_JOINTS {}\n", max_joints);
            let vtxsrc = std::fs::read_to_string(vtxpath).unwrap();
            // 源文件开头是注释、后面才是 #version —— 把所有 #version 行去掉，由我们统一放最前面
            let vtxsrc = format!(
                "{}{}\n",
                define,
                vtxsrc
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("#version"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let vtx = gl::CreateShader(gl::VERTEX_SHADER);
            gl::ShaderSource(
                vtx,
                1,
                [vtxsrc.as_bytes().as_ptr() as *const gl::types::GLchar].as_ptr(),
                [vtxsrc.len() as gl::types::GLint].as_ptr(),
            );
            gl::CompileShader(vtx);
            Self::check_compile_error(vtxpath, vtx);
            gl::AttachShader(prog, vtx);
            gl::DeleteShader(vtx);

            let fragsrc = std::fs::read_to_string(fragpath).unwrap();
            let frag = gl::CreateShader(gl::FRAGMENT_SHADER);
            gl::ShaderSource(
                frag,
                1,
                [fragsrc.as_bytes().as_ptr() as *const gl::types::GLchar].as_ptr(),
                [fragsrc.len() as gl::types::GLint].as_ptr(),
            );
            gl::CompileShader(frag);
            Self::check_compile_error(fragpath, frag);
            gl::AttachShader(prog, frag);
            gl::DeleteShader(frag);

            gl::BindAttribLocation(prog, utils::ATTRIB_VERTEX, c"vertex".as_ptr());
            gl::BindAttribLocation(prog, utils::ATTRIB_NORMAL, c"normal".as_ptr());
            gl::BindAttribLocation(prog, utils::ATTRIB_TEXCOORD, c"texcoord".as_ptr());
            gl::BindAttribLocation(prog, utils::ATTRIB_JOINT, c"joint".as_ptr());
            gl::BindAttribLocation(prog, utils::ATTRIB_WEIGHT, c"weight".as_ptr());

            gl::LinkProgram(prog);
            Self::check_link_error(prog);

            for (enm, attrib) in &ctx.attrib_expressions {
                gl::BindAttribLocation(
                    prog,
                    *attrib,
                    format!("expression_{}_vertex\0", enm).as_ptr() as *const i8,
                );
            }

            let uniform_expressions: HashMap<String, gl::types::GLint> = ctx
                .attrib_expressions
                .keys()
                .map(|enm| {
                    (
                        enm.clone(),
                        gl::GetUniformLocation(
                            prog,
                            format!("expression_{}_weight\0", enm).as_ptr() as *const i8,
                        ),
                    )
                })
                .collect();

            // 名字里带 expression_* 的 uniform 一旦查不到（-1），上传就是空操作，
            // 画面看起来"表情完全没反应"。debug 级别打出来，便于排查这类静默失效。
            let mut missing: Vec<&str> = Vec::new();
            for (enm, loc) in &uniform_expressions {
                if *loc < 0 {
                    missing.push(enm);
                }
            }
            log::debug!("表情 uniform 位置: {:?}", uniform_expressions);
            if !missing.is_empty() {
                log::warn!(
                    "表情 {:?} 的 weight uniform 没找到（location=-1），这些表情不会生效",
                    missing
                );
            }

            Self {
                prog,
                uniform_view: gl::GetUniformLocation(prog, c"view".as_ptr()),
                uniform_position: gl::GetUniformLocation(prog, c"position".as_ptr()),
                uniform_projection: gl::GetUniformLocation(prog, c"projection".as_ptr()),
                uniform_normal: gl::GetUniformLocation(prog, c"normal_matrix".as_ptr()),
                uniform_camera_pos: gl::GetUniformLocation(prog, c"camera_pos".as_ptr()),
                uniform_base_color: gl::GetUniformLocation(prog, c"base_color".as_ptr()),
                uniform_exposure: gl::GetUniformLocation(prog, c"exposure".as_ptr()),
                uniform_saturation: gl::GetUniformLocation(prog, c"saturation".as_ptr()),
                uniform_contrast: gl::GetUniformLocation(prog, c"contrast".as_ptr()),
                uniform_skin_tint: gl::GetUniformLocation(prog, c"skin_tint".as_ptr()),
                uniform_skin_sat: gl::GetUniformLocation(prog, c"skin_sat".as_ptr()),
                uniform_lift: gl::GetUniformLocation(prog, c"shade_lift".as_ptr()),
                uniform_toony: gl::GetUniformLocation(prog, c"shading_toony".as_ptr()),
                uniform_shift: gl::GetUniformLocation(prog, c"shading_shift".as_ptr()),
                uniform_shade_tint: gl::GetUniformLocation(prog, c"shade_tint".as_ptr()),
                uniform_rim: gl::GetUniformLocation(prog, c"rim_strength".as_ptr()),
                uniform_outline_width: gl::GetUniformLocation(prog, c"outline_width".as_ptr()),
                uniform_outline_color: gl::GetUniformLocation(prog, c"outline_color".as_ptr()),
                uniform_outline_pass: gl::GetUniformLocation(prog, c"outline_pass".as_ptr()),
                uniform_outline_glow: gl::GetUniformLocation(prog, c"outline_glow".as_ptr()),
                uniform_part_id: gl::GetUniformLocation(prog, c"part_id".as_ptr()),
                uniform_part_pass: gl::GetUniformLocation(prog, c"part_pass".as_ptr()),
                parts_enabled: utils::env_flag("COLONQ_PARTS", false),
                outline_width: utils::env_parse::<f32>("COLONQ_OUTLINE").unwrap_or(0.004),
                uniform_joint_matrices: gl::GetUniformLocation(prog, c"joint_matrices".as_ptr()),
                max_joints,
                clip_y,
                uniform_clip_y: gl::GetUniformLocation(prog, c"clip_y".as_ptr()),
                uniform_expressions,
            }
        }
    }

    pub fn bind(&self, _ctx: &context::Context) {
        unsafe {
            gl::UseProgram(self.prog);
        }
    }
}
