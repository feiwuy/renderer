use std::{sync::mpsc::{Receiver, channel}, io::Write};
use termion::raw::IntoRawMode;

mod gl {
    #![allow(warnings)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

mod utils;
mod context;
mod vrm;
mod shader;
mod fig;
mod tracking;
mod framebuffer;
mod avatar;
mod mesh;
mod texture;
mod term;
mod serve;
mod display;



/// 读 f32 环境变量；值非法时由 utils::env_parse 打 warn（不再静默回落）
fn env_f32(name: &str, default: f32) -> f32 {
    utils::env_parse(name).unwrap_or(default)
}

/// 读 "x,y,z" 环境变量；格式不对时打 warn 并回落默认值
fn env_vec3(name: &str, default: glam::Vec3) -> glam::Vec3 {
    match utils::env_csv3(name) {
        Some((x, y, z)) => glam::Vec3::new(x, y, z),
        None => default,
    }
}

/// 判断模型是否带 VRM 0.x 的旧扩展（`"VRM":{`）。
/// 这类模型（含用 tools/vrm0_to_vrm1_min.py 转出来的）朝向是 -Z，需要 yaw=0；
/// 原生 VRM 1.0 朝 +Z，需要 yaw=180°。
fn model_is_legacy_vrm0(path: &str) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut buf = vec![0u8; 2 * 1024 * 1024]; // JSON 块在文件开头，读前 2MB 足够
    let n = f.read(&mut buf).unwrap_or(0);
    buf.truncate(n);
    buf.windows(7).any(|w| w == b"\"VRM\":{")
}

/// 资源目录：默认 <crate>/assets，可用环境变量 COLONQ_ASSETS 覆盖
fn asset(name: &str) -> String {
    match std::env::var("COLONQ_ASSETS") {
        Ok(dir) => format!("{}/{}", dir.trim_end_matches('/'), name),
        Err(_) => format!("{}/assets/{}", env!("CARGO_MANIFEST_DIR"), name),
    }
}

fn render_loop(
    tracking_state: tracking::SharedTrackingState,
    control: fig::Control,
    command_receiver: Receiver<fig::ControlCommand>
) {
    let ctx = context::Context::new();
    // 内部采样始终使用请求的网格。输出尺寸只影响终端显示，不能拖低 OBS 推流精度。
    let requested = match std::env::var("COLONQ_GRID") {
        Ok(raw) => display::Size::parse(&raw).unwrap_or_else(|| {
            log::warn!("COLONQ_GRID 的值 {:?} 不是 WxH 形式，已忽略（使用 128x64）", raw);
            display::Size::new(128, 64)
        }),
        Err(_) => display::Size::new(128, 64),
    };
    let (grid_w, grid_h) = (requested.w.clamp(16, 512) as i32, requested.h.clamp(16, 256) as i32);
    let requested = display::Size::new(grid_w as usize, grid_h as usize);
    // 开关统一走 utils::env_flag：COLONQ_NO_TERM=0 表示"要终端输出"（未设置时默认要）
    let term_mode = !utils::env_flag("COLONQ_NO_TERM", false);
    let fit_enabled = utils::env_flag("COLONQ_FIT", true);
    let size_hint = std::env::var("COLONQ_SIZE").ok()
        .and_then(|v| display::Size::parse(&v));
    let pixels_mode = size_hint.is_some() || utils::env_flag("COLONQ_PIXELS", false);
    let cell_px = (env_f32("COLONQ_CELL_W", 7.0), env_f32("COLONQ_CELL_H", 14.0));
    let fit = || {
        let terminal = termion::terminal_size().ok()
            .filter(|(w, h)| *w > 0 && *h > 0)
            .map(|(w, h)| display::Size::new(w as usize, h as usize));
        let pixels = if pixels_mode {
            size_hint.or_else(|| termion::terminal_size_pixels().ok()
                .filter(|(w, h)| *w > 0 && *h > 0)
                .map(|(w, h)| display::Size::new(w as usize, h as usize)))
        } else { None };
        display::terminal_bounds(requested, terminal, pixels, cell_px, fit_enabled)
    };
    let mut last_output_size = fit();
    log::info!("渲染网格: {}x{}；终端显示区域: {}x{}（像素提示 {:?}，内部精度保持不变）",
               grid_w, grid_h, last_output_size.w, last_output_size.h, size_hint);
    let mut term0 = term::Term::new(grid_w as usize, grid_h as usize);
    let mut term1 = term::Term::new(grid_w as usize, grid_h as usize);
    let mut term = &mut term0;
    let mut lastterm = &mut term1;
    let mut term_counter = 0;
    let mut keyframe_counter = 0;
    let mut players = avatar::PalettePlayers::new(&ctx);

    // 按部位分表：COLONQ_PARTS=1 启用，COLONQ_PART_WORDS="1=.,2=~,3=#,4=colonq"
    // 部位编号：1=脸/皮肤 2=头发 3=眼睛 4=衣服 9=其他；COLONQ_WORD 是默认词
    if utils::env_flag("COLONQ_PARTS", false) {
        let mut c = control.0.lock().unwrap();
        if let Ok(w) = std::env::var("COLONQ_WORD") {
            c.palette.default_word = w;
        }
        let spec = std::env::var("COLONQ_PART_WORDS")
            .unwrap_or_else(|_| "2=.,4=#".to_string());
        for item in spec.split(',') {
            if let Some((k, v)) = item.split_once('=') {
                if let Ok(id) = k.trim().parse::<u8>() {
                    c.palette.part_words.insert(id, v.to_string());
                }
            }
        }
        log::info!("部位分表: {:?} 默认词={}", c.palette.part_words, c.palette.default_word);
    }
    // 模型文件可用 COLONQ_MODEL 指定（放在 assets/ 下）
    let model_file = std::env::var("COLONQ_MODEL")
        .unwrap_or_else(|_| "my_avatar.vrm".to_string());
    log::info!("使用模型: {}", model_file);

    // 先读模型有多少根骨骼，决定 shader 里骨骼矩阵数组多大（NVIDIA 顶点常量寄存器有限）
    let model_path = asset(&model_file);
    let joints = vrm::detect_max_joints(&model_path);

    // 裁剪面：COLONQ_CLIP=chest 只渲染胸口以上；也可以直接给世界 Y 数值；不设 = 不裁剪
    let clip_default = if std::env::var("COLONQ_VIEW").map(|v| v == "bust").unwrap_or(false) { "chest" } else { "none" };
    let clip_y: f32 = match std::env::var("COLONQ_CLIP").or_else(|_| Ok::<String, std::env::VarError>(clip_default.to_string())) {
        Ok(v) if v.eq_ignore_ascii_case("chest") => {
            let y = vrm::detect_chest_y(&model_path);
            log::info!("裁剪面: 胸部 y={:.3}（只渲染头部和胸口以上）", y);
            y
        }
        Ok(v) if v.eq_ignore_ascii_case("none") => f32::NEG_INFINITY,
        Ok(v) => v.trim().parse().unwrap_or(f32::NEG_INFINITY),
        Err(_) => f32::NEG_INFINITY,
    };
    let shader = shader::Shader::new(&ctx, &asset("shader.vert"), &asset("shader.frag"), joints as i32, clip_y);
    // 浏览器源/后台模式不要求 stdout 是终端。stdout 不是 TTY（比如被重定向、
    // 直接前台跑二进制）时自动降级为"只推流"，避免 panic。
    let _raw_stdout = if term_mode {
        match termion::terminal_size() {
            Ok(_) => Some(std::io::stdout().into_raw_mode()
                .expect("终端进入 raw mode 失败")),
            Err(e) => {
                log::warn!("stdout 不是 TTY（{}），自动关闭终端输出；ASCII 推流不受影响", e);
                None
            }
        }
    } else { None };
    let term_mode = term_mode && _raw_stdout.is_some();

    // 终端写入放到独立线程：只有一个槽位，主循环永不阻塞 ——
    // 窗口被遮挡 / OBS 不采集时终端读取变慢，也不会把渲染循环拖住（来不及就丢帧）
    // 本地推流服务（COLONQ_SERVE=端口）：把字符画面以 HTML 推给 OBS 浏览器源，
    // 完全不依赖终端窗口是否可见
    let frame_store = match std::env::var("COLONQ_SERVE").ok().and_then(|v| v.trim().parse::<u16>().ok()) {
        Some(port) => {
            let store = std::sync::Arc::new(serve::FrameStore::new((ctx.bgcolor.0 as u8, ctx.bgcolor.1 as u8, ctx.bgcolor.2 as u8)));
            store.set_frame(term.snapshot());
            let s2 = store.clone();
            std::thread::spawn(move || serve::run(port, s2));
            Some(store)
        }
        None => None,
    };

    let (term_tx, term_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(1);
    let dropped_frames = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let dropped_clone = dropped_frames.clone();
    std::thread::spawn(move || {
        let mut out = std::io::stdout();
        for buf in term_rx {
            use std::io::Write;
            if out.write_all(&buf).is_err() { break }
            let _ = out.flush();
        }
        let _ = dropped_clone;
    });

    // 本地部署：原版的 rebroadcast(TCP 31340) 与 websocket 推流已移除
    // 取景预设：head = 头部特写（默认，字符密度高）；full = 全身
    let view_mode = std::env::var("COLONQ_VIEW").unwrap_or_else(|_| "head".to_string());
    // head  = 头部 + 肩胸，头顶基本贴住画面上沿（不留空）
    // face  = 面部特写，眼睛占画面主体
    // full  = 全身
    let (default_cam, default_look) = match view_mode.as_str() {
        "full" => (glam::Vec3::new(0.0, 0.82, -2.55), glam::Vec3::new(0.0, 0.05, 1.0)),
        "face" => (glam::Vec3::new(0.0, 1.40, -0.27), glam::Vec3::new(0.0, 0.0, 1.0)),
        // 半身：头部 + 胸肩（配合 COLONQ_CLIP=chest，bust 预设会自动打开裁剪）
        "bust" => (glam::Vec3::new(0.0, 1.29, -0.70), glam::Vec3::new(0.0, 0.0, 1.0)),
        _ => (glam::Vec3::new(0.0, 1.377, -0.48), glam::Vec3::new(0.0, 0.0, 1.0)),
    };
    log::info!("取景: {} (cam={:?})", view_mode, default_cam);

    // 模型变换（用于适配不同 VRM 摆位/朝向，默认值是按通用 VRM 1.0 调的）
    let model_scale = env_f32("COLONQ_SCALE", 1.0);
    let legacy_yaw = model_is_legacy_vrm0(&asset(&model_file));
    let default_yaw = if legacy_yaw { 0.0 } else { std::f32::consts::PI };
    log::info!("模型{} VRM 0.x 转换件，默认 yaw={}",
               if legacy_yaw { "是" } else { "不是" }, default_yaw);
    let model_yaw = env_f32("COLONQ_YAW", default_yaw);
    let model_offset = env_vec3("COLONQ_OFFSET", glam::Vec3::new(0.0, 0.0, 0.0));

    let mut avatar_old = avatar::Avatar::new(
        &ctx, &asset(&model_file),
        move |_| glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::splat(model_scale),
            glam::Quat::from_rotation_y(model_yaw),
            model_offset,
        ),
    ); 
    let mut avatar_new = avatar::Avatar::new(
        &ctx, &asset(&model_file),
        move |sts: &fig::Control| glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::splat(model_scale),
            glam::Quat::from_rotation_y(
                model_yaw + if sts.is("forsen") { -0.30 } else { 0.0 }
            ),
            model_offset,
        ),
    );
    // avatar_new.add_hat(avatar::hat::Hat::cone(&ctx));

    let mut fb = framebuffer::Framebuffer::new(&ctx, true, (grid_w, grid_h), (0, 0));
    let pixels_len = (ctx.dims.w * ctx.dims.h * 4.0) as usize;
    let pixels = vec![0; pixels_len];
    let screen = framebuffer::Framebuffer {
        fbo: 0,
        tex: 0,
        dims: ctx.dims.clone(),
        offsets: (0, 0),
        pixels_len,
        pixels,
    };

    // 终端字符宽高约 1:2，所以正方形的世界视野需要横向多一倍像素
    let fb_projection = glam::Mat4::perspective_lh(
        std::f32::consts::PI / 4.0,
        (fb.dims.w as f32 * 0.5) / fb.dims.h as f32,
        0.1,
        10.0,
    );

    // COLONQ_DUMP_PNG=1 时把画面存成 PNG，用于无终端/无显示环境下验证渲染是否正确
    let dump_png = utils::env_flag("COLONQ_DUMP_PNG", false);
    let dump_at = utils::env_parse::<i32>("COLONQ_DUMP_AT").unwrap_or(120).max(1);
    let arm_pose = env_f32("COLONQ_ARM_POSE", -0.80);
    let no_mpv = utils::env_flag("COLONQ_NO_MPV", false);
    // 待机动画：让模型自己动起来（转头、眨眼、呼吸），默认开启
    let idle = utils::env_flag("COLONQ_IDLE", true);
    let idle_speed = env_f32("COLONQ_IDLE_SPEED", 1.0);
    let idle_phase = env_f32("COLONQ_T0", 0.0);
    let anim_start = std::time::Instant::now();
    // 序列存图：COLONQ_DUMP_SEQ=1 时存成 frame-000005.png 这样，方便合成动图
    let dump_seq = utils::env_flag("COLONQ_DUMP_SEQ", false);
    // 终端重画间隔（帧）：默认 6 → 60fps 下约 10 次/秒。设 1 则每帧都重画（更流畅但更费 CPU）
    let term_every = utils::env_parse::<i32>("COLONQ_TERM_EVERY").unwrap_or(1).max(1);
    // 分层计时（COLONQ_PROFILE=1 时每 60 帧输出一次各阶段平均耗时）
    let profile = utils::env_flag("COLONQ_PROFILE", false);
    let (mut t_avatar, mut t_blit, mut t_swap, mut t_term, mut t_pop, mut t_cnt) =
        (0f64, 0f64, 0f64, 0f64, 0f64, 0u32);
    let mut term_updates = 0u32;
    let mut rendered_updates = 0u32;
    let mut ascii_updates = 0u32;
    let mut last_profile = std::time::Instant::now();
    // 面捕动作幅度：旋转增益（越大头部转动越夸张）
    let track_gain = env_f32("COLONQ_TRACK_GAIN", 1.0);
    // 面捕表情增益：blink / oh 权重放大
    let expr_gain  = env_f32("COLONQ_EXPR_GAIN", 1.0);

    // 颜色调参（都可以用环境变量覆盖）
    let c_exposure   = env_f32("COLONQ_EXPOSURE", 0.85);        // 整体亮度倍率
    let c_saturation = env_f32("COLONQ_SATURATION", 1.30);
    let c_contrast   = env_f32("COLONQ_CONTRAST", 1.0);        // 对比度（围绕中灰拉伸）     // 饱和度
    let c_lift       = env_f32("COLONQ_SHADE_LIFT", 0.0);      // 暗部提亮（0 = 不提）
    let c_toony      = env_f32("COLONQ_TOONY", 0.95);          // 明暗分界硬度
    let c_shift      = env_f32("COLONQ_SHIFT", -0.10);          // 整体明暗偏移
    let c_shade_tint = env_vec3("COLONQ_SHADE_TINT", glam::Vec3::new(0.82, 0.80, 0.88));
    let c_rim        = env_f32("COLONQ_RIM", 0.03);            // 边缘光强度
    let c_outline_color = env_vec3("COLONQ_OUTLINE_COLOR", glam::Vec3::new(0.55, 0.85, 1.0)); // 描边色（冰蓝）
    let c_outline_glow = env_f32("COLONQ_OUTLINE_GLOW", 0.6);  // 描边发光强度（0=普通描边）
    // 皮肤/脸部单独染色（部位编号 1 = FACE/SKIN 材质），默认不染色
    let c_skin_tint = env_vec3("COLONQ_SKIN_TINT", glam::Vec3::new(1.22, 1.18, 1.20));
    let c_skin_sat  = env_f32("COLONQ_SKIN_SAT", 1.00);
    log::info!("颜色参数: exposure={} sat={} contrast={} toony={} shift={} rim={} shade_tint={:?}",
               c_exposure, c_saturation, c_contrast, c_toony, c_shift, c_rim, c_shade_tint);
    log::info!("皮肤染色: tint={:?} sat={}", c_skin_tint, c_skin_sat);
    log::info!("面捕增益: 旋转={} 表情={}", track_gain, expr_gain);

    let mut event_pump = ctx.sdl2.event_pump().unwrap();
    let mut framecount = 0;
    let mut float_time: f32 = 0.0;

    log::info!("Starting model renderer");

    // 目标帧率：默认锁 24fps（COLONQ_FPS=0 表示不限速）
    let target_fps = env_f32("COLONQ_FPS", 24.0);
    let frame_dt: f32 = if target_fps > 0.0 { 1.0 / target_fps } else { 0.0 };
    log::info!("目标帧率: {}", if frame_dt > 0.0 { format!("{:.1} fps", target_fps) } else { "不限速".into() });
    let mut last = std::time::Instant::now();
    let mut acc: f32 = 0.0;
    'mainloop: loop {
        unsafe {
            let err = gl::GetError();
            if err != 0 {
                log::error!("OpenGL error: {:?}", err);
                break;
            }
        }

        let avatar = if control.is("old") {
            &mut avatar_old
        } else {
            &mut avatar_new
        };

        acc += last.elapsed().as_secs_f32();
        last = std::time::Instant::now();
        // 本轮是否有有效面捕数据（决定待机动画是否让位给真人）
        let tracking_live = tracking_state.lock().unwrap().tracking_active();

        // 这一轮要渲染几帧：限速时按累计时间算，不限速时每轮都跑
        let steps: i32 = if frame_dt > 0.0 {
            let mut n = 0;
            while acc >= frame_dt {
                acc -= frame_dt;
                n += 1;
            }
            n
        } else {
            1
        };

        for _ in 0..steps {
            for event in event_pump.poll_iter() {
                match event {
                    sdl2::event::Event::Quit { .. } => {
                        break 'mainloop;
                    },
                    _ => (),
                }
            }

            // 头部：有有效面捕数据时完全交给追踪（不加待机摆动）；
            // 面捕离线或超时（tracking.rs 的 TRACKING_TIMEOUT）时回退到待机摆动
            let idle_head = if idle && !tracking_live {
                let t = (anim_start.elapsed().as_secs_f32() + idle_phase) * idle_speed;
                glam::Quat::from_rotation_y((t * 0.62).sin() * 0.22)
                    * glam::Quat::from_rotation_x((t * 0.41).sin() * 0.09)
            } else {
                glam::Quat::IDENTITY
            };
            // 追踪旋转 × 增益：把四元数转成轴角，放大角度后重组
            let tracked_q = tracking_state.lock().unwrap().orientation.inverse();
            let (axis, angle) = tracked_q.to_axis_angle();
            let amplified = glam::Quat::from_axis_angle(axis, (angle * track_gain).clamp(-2.6, 2.6));
            avatar.transform_bone("head", &glam::Mat4::from_rotation_translation(
                amplified * idle_head,
                glam::Vec3::ZERO,
            ));
            // 待机呼吸：脊柱轻微起伏
            if idle && !tracking_live {
                let t = (anim_start.elapsed().as_secs_f32() + idle_phase) * idle_speed;
                avatar.transform_bone("spine", &glam::Mat4::from_rotation_translation(
                    glam::Quat::from_rotation_x((t * 1.05).sin() * 0.012),
                    glam::Vec3::ZERO,
                ));
            }

            // 手臂姿态：原版写死 ±45°，那是按作者的模型调的。不同模型骨骼朝向不同，
            // 这里默认不打姿态（0），需要时用 COLONQ_ARM_POSE 调整
            if arm_pose != 0.0 {
                avatar.transform_bone("leftUpperArm", &glam::Mat4::from_rotation_translation(
                    glam::Quat::from_rotation_z(-arm_pose),
                    glam::Vec3::ZERO,
                ));
                avatar.transform_bone("rightUpperArm", &glam::Mat4::from_rotation_translation(
                    glam::Quat::from_rotation_z(arm_pose),
                    glam::Vec3::ZERO,
                ));
            }

            // render framebuffer to terminal
            framecount = framecount + 1;
            float_time = anim_start.elapsed().as_secs_f32();

            if framecount == 1000 {
                let mut f = std::fs::File::create("oneframe.txt").unwrap();
                // term.render_stream_nocolor_small(&mut f);
                // term.serialize(&mut f);
                f.write_all(&term.serialize_and_compress()).unwrap();
            }

        }

        // 还没到下一帧就睡一下，避免空转把 CPU 吃满（这也是"前后台都稳 24fps"的关键）
        if frame_dt > 0.0 && steps == 0 {
            let remain = frame_dt - acc;
            if remain > 0.0 {
                std::thread::sleep(std::time::Duration::from_secs_f32(remain));
            }
            continue;
        }

        // compute camera position and view matrix
        let camera_pos_base = if control.is("forsen") {
            env_vec3("COLONQ_CAM", glam::Vec3::new(0.0, 1.35, -0.4))
        } else {
            env_vec3("COLONQ_CAM", default_cam)
        };
        let camera_pos = if control.is("zoom_wave") {
            let offset = float_time.sin() * 0.1;
            camera_pos_base + glam::Vec3::new(0.0, 0.0, offset)
        } else {
            camera_pos_base
        };
        let up = if control.is("spin") {
            let angle = (std::f32::consts::PI / 2.0)
                + if control.is("spin_direction") {
                    -(float_time / 4.0)
                } else {
                    float_time / 4.0
                };
            glam::Vec3::new(angle.cos(), angle.sin(), 0.0)
        } else {
            glam::Vec3::new(0.0, 1.0, 0.0)
        };
        let view = glam::Mat4::look_at_lh(
            camera_pos,
            camera_pos + if control.is("forsen") {
                glam::Vec3::new(0.0, 0.2, 1.0)
            } else {
                env_vec3("COLONQ_LOOK", default_look)
            },
            up,
        );

        // update video players
        while let Ok(comm) = command_receiver.try_recv() {
            match comm {
                fig::ControlCommand::PlayVideo { pty, url } => {
                    if let Some(player) = players.players.get(&pty) {
                        player.mpv.playlist_load_files(&[(&url, libmpv::FileState::Replace, None)]).unwrap();
                    }
                },
                fig::ControlCommand::ReloadPumpkin => {
                    // avatar.pumpkin.reload(&ctx);
                },
            }
        }
        if !no_mpv {
            players.update(&ctx, &control.0.lock().unwrap().palette);
        }

        // render avatar framebuffer
        fb.bind(&ctx);
        unsafe {
            gl::ClearColor(ctx.bgcolor.0 as f32 / 255.0, ctx.bgcolor.1 as f32 / 255.0, ctx.bgcolor.2 as f32 / 255.0, 1.0);
        }
        unsafe { gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT); }
        shader.bind(&ctx);
        unsafe {
            gl::Uniform1f(shader.uniform_exposure, c_exposure);
            gl::Uniform1f(shader.uniform_saturation, c_saturation);
            gl::Uniform1f(shader.uniform_contrast, c_contrast);
            gl::Uniform3fv(shader.uniform_skin_tint, 1, c_skin_tint.to_array().as_ptr());
            gl::Uniform1f(shader.uniform_skin_sat, c_skin_sat);
            gl::Uniform1f(shader.uniform_lift, c_lift);
            gl::Uniform1f(shader.uniform_toony, c_toony);
            gl::Uniform1f(shader.uniform_shift, c_shift);
            gl::Uniform3fv(shader.uniform_shade_tint, 1, c_shade_tint.to_array().as_ptr());
            gl::Uniform1f(shader.uniform_rim, c_rim);
            gl::Uniform1f(shader.uniform_outline_width, shader.outline_width);
            gl::Uniform1f(shader.uniform_clip_y, shader.clip_y);
            gl::Uniform3fv(shader.uniform_outline_color, 1, c_outline_color.to_array().as_ptr());
            gl::Uniform1f(shader.uniform_outline_glow, c_outline_glow);
            gl::UniformMatrix4fv(shader.uniform_view, 1, false as u8, view.to_cols_array().as_ptr());
            gl::UniformMatrix4fv(shader.uniform_projection, 1, false as u8, fb_projection.to_cols_array().as_ptr());
            gl::Uniform3fv(shader.uniform_camera_pos, 1, camera_pos.to_array().as_ptr());
            // 待机眨眼：只在面捕离线时生效（面捕在线时 blink 权重来自接收线程）
            let idle_blink = if idle && !tracking_live {
                let t = (anim_start.elapsed().as_secs_f32() + idle_phase) * idle_speed;
                let p = t % 3.7;
                if p < 0.10 { 1.0 } else if p < 0.18 { (0.18 - p) / 0.08 } else { 0.0 }
            } else {
                0.0
            };
            for (enm, uniform) in &shader.uniform_expressions {
                let tracked = *tracking_state.lock().unwrap().expression_weights.get(enm).unwrap_or(&0.0) * expr_gain;
                let w = if enm == "blink" { tracked.max(idle_blink) } else { tracked };
                gl::Uniform1f(*uniform, w.clamp(0.0, 1.0));
            }
        }
        let ta = std::time::Instant::now();
        avatar.render(&ctx, &shader, &view, &fb_projection, &control);
        t_avatar += ta.elapsed().as_secs_f64() * 1000.0;
        rendered_updates += 1;
        let tb = std::time::Instant::now();
        if ctx.preview_enabled {
            screen.bind(&ctx);
            unsafe {
                gl::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT);
                gl::BindFramebuffer(gl::READ_FRAMEBUFFER, fb.fbo);
                gl::BindFramebuffer(gl::DRAW_FRAMEBUFFER, screen.fbo);
                gl::BlitFramebuffer(0, 0, fb.dims.w as _, fb.dims.h as _, 0, 0, 512 as _, 512 as _, gl::COLOR_BUFFER_BIT, gl::NEAREST);
            }
        }
        t_blit += tb.elapsed().as_secs_f64() * 1000.0;

        // 当前帧画完才回读并发布；OBS 推流与 stdout/终端可见性完全解耦。
        let bgcolor = if control.is("forsen") { (0x2c, 0x52, 0x39) }
            else { (ctx.bgcolor.0 as u8, ctx.bgcolor.1 as u8, ctx.bgcolor.2 as u8) };
        if framecount % term_every == 0 && (term_mode || frame_store.is_some()) {
            let t0 = std::time::Instant::now();
            fb.populate_pixels();
            t_pop += t0.elapsed().as_secs_f64() * 1000.0;
            fb.render_term(&ctx, framecount, &mut players, &control.0.lock().unwrap().palette, term, term_counter);
            term_counter += 1;
            ascii_updates += 1;
            if let Some(store) = frame_store.as_ref() {
                store.set_frame(term.snapshot());
            }
            if term_mode {
                let output_size = fit();
                if output_size != last_output_size {
                    log::info!("终端输出尺寸变化: {:?} → {:?}；渲染网格仍 {}x{}",
                               last_output_size, output_size, grid_w, grid_h);
                    last_output_size = output_size;
                }
                let buf = term.encode_stream_sized(output_size, bgcolor);
                match term_tx.try_send(buf) {
                    Ok(()) => term_updates += 1,
                    Err(std::sync::mpsc::TrySendError::Full(_)) => {
                        dropped_frames.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {}
                }
            }
            t_term += t0.elapsed().as_secs_f64() * 1000.0;
        }
        if framecount % 12 == 0 && utils::env_flag("COLONQ_DUMP_FRAMES", false) {
            let frame = if keyframe_counter == 0 {
                term.serialize_and_compress()
            } else {
                term::Diff::new(lastterm, term).serialize_and_compress()
            };
            let _ = std::fs::write(format!("frame-{:06}.bin", framecount), &frame);
            core::mem::swap(&mut term, &mut lastterm);
            keyframe_counter = (keyframe_counter + 1) % 10;
        }

        // 调试存图：64x64 是渲染原始尺寸，512x512 是 blit 放大后的尺寸
        if dump_png && framecount >= dump_at && framecount % dump_at == 0 {
            unsafe {
                gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
                while gl::GetError() != 0 {} // 清掉可能残留的错误，避免主循环把它当致命错误退出
            }
            fb.populate_pixels();
            let name = if dump_seq { format!("frame-{:06}.png", framecount) } else { "frame64.png".to_string() };
            let _ = image::save_buffer(
                &name, &fb.pixels, fb.dims.w as u32, fb.dims.h as u32, image::ColorType::Rgba8,
            );

            // 128x64 这种宽屏网格是"横向 2 倍采样"的，直接看会被拉宽，
            // 这里额外存一张横向压回一半的，方便肉眼对照
            if let Ok(img) = image::open(&name) {
                // 一个像素对应一个字符格（宽:高 = 1:2），所以纵向拉一倍就是屏幕上的真实比例
                let view = image::imageops::resize(&img.to_rgba8(), img.width(), img.height() * 2, image::imageops::FilterType::Nearest);
                let _ = view.save("frame-view.png");
            }
            unsafe { while gl::GetError() != 0 {} } // 调试回读自身可能产生 GL 错误，清掉
            log::info!("dumped {} / frame-view.png at frame {}", name, framecount);
        }

        // refresh display
        let ts = std::time::Instant::now();
        if ctx.preview_enabled { ctx.window.gl_swap_window(); }
        t_swap += ts.elapsed().as_secs_f64() * 1000.0;

        t_cnt += 1;
        if profile && t_cnt >= 60 {
            let wall = last_profile.elapsed().as_secs_f64();
            log::info!(
                "profile 实际 fps={:.1} ASCII生成={:.1}/s 终端刷新={:.1}/s 丢帧={} | framecount={} 每帧均值(ms): 总计 {:.1} | avatar {:.2} | blit {:.2} | swap {:.1} | 终端(含回读 {:.1}) {:.1}",
                rendered_updates as f64 / wall,
                ascii_updates as f64 / wall,
                term_updates as f64 / wall,
                dropped_frames.swap(0, std::sync::atomic::Ordering::Relaxed),
                framecount,
                t_avatar / 60.0 + t_blit / 60.0 + t_swap / 60.0 + t_term / 60.0,
                t_avatar / 60.0, t_blit / 60.0, t_swap / 60.0, t_pop / 60.0, t_term / 60.0
            );
            t_avatar = 0.0; t_blit = 0.0; t_swap = 0.0; t_term = 0.0; t_pop = 0.0; t_cnt = 0;
            term_updates = 0; rendered_updates = 0; ascii_updates = 0;
            last_profile = std::time::Instant::now();
        }
    }
}

fn main() {
    simple_logging::log_to_file("colonq.log", log::LevelFilter::Debug).unwrap();
    log_panics::init();
    // 先把所有生效的 COLONQ_* 打进日志：改完变量能一眼确认有没有传到（值非法会有 warn）
    utils::log_env_snapshot();

    let tracking_state = tracking::TrackingState::new();
    let shared_tracking_state = std::sync::Arc::new(std::sync::Mutex::new(tracking_state));
    let shared_tracking_state_clone = shared_tracking_state.clone();

    let control = fig::Control::new();
    let (command_sender, command_receiver) = channel();

    let render_handle = std::thread::spawn({
        let control = control.clone();
        move || {
            render_loop(shared_tracking_state_clone, control, command_receiver);
        }
    });

    // 绑定面捕 UDP：失败只 warn、渲染照跑。成功时日志会打印**实际绑定地址**——
    // 跨机面捕就靠这一行确认到底有没有听在 0.0.0.0 上（见 UBUNTU_FACECAP.md）。
    tracking::run(shared_tracking_state.clone());
    control.run(command_sender);

    render_handle.join().unwrap();
}
