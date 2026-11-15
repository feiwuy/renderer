use std::collections::HashMap;
use std::time::{Duration, Instant};

use byteorder::ByteOrder;

use crate::utils;

pub type SharedTrackingState = std::sync::Arc<std::sync::Mutex<TrackingState>>;

/// 面捕数据在多久内算“有效”；超时后回退到待机动画
pub const TRACKING_TIMEOUT: Duration = Duration::from_secs(2);

/// facetracker.py 单脸数据包长度（见 tools/openseeface-server.patch 的字段表）
pub const PACKET_LEN: usize = 1785;

/// 上游轴补偿之外的翻转模式（COLONQ_FLIP 选择）：
/// 0=无  1=Z轴180°（左右翻转）  2=Y轴180°（掉头）  3=X轴180°（前滚翻）  4=Z+Y
/// 真人面捕实测：档3（Z轴180°）修正了静态测试，但真人数据需要换档时用此开关。
fn flip_quat(mode: u8) -> glam::Quat {
    match mode {
        1 => glam::Quat::from_xyzw(0.0, 0.0, 1.0, 0.0),            // Z 180°
        2 => glam::Quat::from_xyzw(0.0, 1.0, 0.0, 0.0),            // Y 180°
        3 => glam::Quat::from_xyzw(1.0, 0.0, 0.0, 0.0),            // X 180°
        4 => glam::Quat::from_xyzw(0.0, 0.0, 1.0, 0.0)
                * glam::Quat::from_xyzw(0.0, 1.0, 0.0, 0.0),       // Z×Y
        5 => glam::Quat::from_xyzw(1.0, 0.0, 0.0, 0.0)
                * glam::Quat::from_xyzw(0.0, 1.0, 0.0, 0.0),       // X×Y
        6 => glam::Quat::from_xyzw(1.0, 0.0, 0.0, 0.0)
                * glam::Quat::from_xyzw(0.0, 0.0, 1.0, 0.0),       // X×Z
        _ => glam::Quat::IDENTITY,
    }
}

// ---- 包内字段偏移（facetracker.py 的 struct.pack 顺序，小端）----
// time(8) | face_id(4) | width(4) | height(4) | eye_blink[2](8) | success(1)
// | pnp_error(4) | quaternion(16) | euler(12) | translation(12)
// | confidence 68*4 | points 68*8 | pts_3d 70*12 | features 14*4
const BLINK_OFFSET: usize = 20; // 两眼张开程度，0..1，越大越睁开
const QUAT_OFFSET: usize = 33; // 头部姿态四元数 x,y,z,w
const FEATURES_OFFSET: usize = 1729; // 14 个特征 f32 的起始
// features 顺序见 facetracker.py：eye_l, eye_r, eyebrow_*6, mouth_corner_*4, mouth_open, mouth_wide
const FEATURE_MOUTH_OPEN: usize = FEATURES_OFFSET + 12 * 4; // = 1777

pub struct TrackingState {
    pub expression_weights: HashMap<String, f32>,
    pub orientation: glam::Quat,
    /// 最近一次收到有效面捕包的时间；None = 从未收到
    pub last_packet: Option<Instant>,
}

impl TrackingState {
    pub fn new() -> Self {
        Self {
            expression_weights: HashMap::from([
                ("happy".to_owned(), 0.0),
                ("angry".to_owned(), 0.0),
                ("sad".to_owned(), 0.0),
                ("relaxed".to_owned(), 0.0),
                ("surprised".to_owned(), 0.0),
                ("aa".to_owned(), 0.0),
                ("ih".to_owned(), 0.0),
                ("ou".to_owned(), 0.0),
                ("ee".to_owned(), 0.0),
                ("oh".to_owned(), 0.0),
                ("blink".to_owned(), 0.0),
            ]),
            orientation: glam::Quat::IDENTITY,
            last_packet: None,
        }
    }

    /// 最近 TRACKING_TIMEOUT 内收到过有效面捕数据？
    pub fn tracking_active(&self) -> bool {
        self.last_packet.map(|t| t.elapsed() < TRACKING_TIMEOUT).unwrap_or(false)
    }
}

/// 解析单脸数据包并写入共享状态；返回 false 表示包无效（长度/数值异常）
fn parse_packet(buf: &[u8], sts: &SharedTrackingState, diag: bool, counter: u64, flip_mode: u8, offset_rad: (f32, f32, f32), pitch_sign: f32, qy_sign: f32, qz_sign: f32, pitch_flip: bool, smooth: f32) -> bool {
    if buf.len() < PACKET_LEN || buf.len() % PACKET_LEN != 0 {
        return false; // 多脸包按 1785 的整数倍拼接；只取第一张脸
    }
    let left_blinking = byteorder::LittleEndian::read_f32(&buf[BLINK_OFFSET..]);
    let right_blinking = byteorder::LittleEndian::read_f32(&buf[BLINK_OFFSET + 4..]);
    let mouth_open = byteorder::LittleEndian::read_f32(&buf[FEATURE_MOUTH_OPEN..]);
    let quat = (
        byteorder::LittleEndian::read_f32(&buf[QUAT_OFFSET..]),
        byteorder::LittleEndian::read_f32(&buf[QUAT_OFFSET + 4..]),
        byteorder::LittleEndian::read_f32(&buf[QUAT_OFFSET + 8..]),
        byteorder::LittleEndian::read_f32(&buf[QUAT_OFFSET + 12..]),
    );
    // 3D 估计失败时 facetracker 会发全零/无效数值；丢弃这类包
    if !(quat.0.is_finite() && quat.1.is_finite() && quat.2.is_finite() && quat.3.is_finite()
        && left_blinking.is_finite() && right_blinking.is_finite() && mouth_open.is_finite())
    {
        return false;
    }
    // COLONQ_TRACK_DEBUG=1：打印收到的原始数值（前 3 个包 + 之后每 100 个），
    // 用来确认"包到底有没有到""数值合不合理"
    if diag && (counter <= 3 || counter % 100 == 0) {
        log::info!("面捕包 #{}: blink L/R={:.3}/{:.3} mouth={:.3} quat=({:.3},{:.3},{:.3},{:.3})",
                   counter, left_blinking, right_blinking, mouth_open, quat.0, quat.1, quat.2, quat.3);
    }
    // 原始四元数的俯仰分量翻转（COLONQ_PITCH_SIGN=-1 修正低头/抬头反向）：
    // OpenSeeFace 的四元数 x 分量主要承载俯仰（pitch），取反即镜像俯仰方向
    let qx = quat.0 * pitch_sign;
    let qy = -quat.1 * qy_sign;
    let qz = -quat.2 * qz_sign;
    let qw = quat.3;
    // 上游的轴补偿：OpenSeeFace 坐标系 → 本渲染器的模型坐标系，
    // 外加 COLONQ_FLIP 选择的翻转 + COLONQ_ROT_OFFSET 自由角度偏移（度）
    let (comp_x, comp_y, comp_z) = (
        std::f32::consts::PI + offset_rad.0, offset_rad.1, std::f32::consts::PI / 2.0 + offset_rad.2
    );
    let mut new = glam::Quat::from_xyzw(qx, qy, qz, qw)
        .mul_quat(glam::Quat::from_euler(glam::EulerRot::XYZ, comp_x, comp_y, comp_z))
        .mul_quat(flip_quat(flip_mode));
    // COLONQ_PITCH_FLIP=1：真人点头方向与模型相反时，把最终姿态欧拉分解后的
    // pitch（俯仰）分量取反再重组——只翻转俯仰，不破坏偏航和滚转
    if pitch_flip {
        let (r, p, y) = new.to_euler(glam::EulerRot::YXZ);
        new = glam::Quat::from_euler(glam::EulerRot::YXZ, r, -p, y);
    }
    let blink = (1.0 - (left_blinking + right_blinking) / 2.0).clamp(0.0, 1.0);
    let mut state = sts.lock().unwrap();
    // 指数平滑：用上一次姿态与新姿态做球面插值（slerp），smooth 越大越平滑。
    // 首包（last_packet 为 None）不平滑——没有可依据的上一次姿态。
    if smooth > 0.001 && state.last_packet.is_some() {
        let prev = state.orientation;
        new = prev.slerp(new, 1.0 - smooth);
    }
    state.orientation = new;
    // 权重平滑同样跳过首包
    if smooth > 0.001 && state.last_packet.is_some() {
        let smooth_w = |prev: Option<&f32>, new_val: f32| -> f32 {
            match prev {
                Some(p) => p + (new_val - p) * (1.0 - smooth),
                None => new_val,
            }
        };
        let bw = smooth_w(state.expression_weights.get("blink"), blink);
        let ow = smooth_w(state.expression_weights.get("oh"), mouth_open.clamp(0.0, 1.0));
        state.expression_weights.insert("blink".to_owned(), bw);
        state.expression_weights.insert("oh".to_owned(), ow);
    } else {
        state.expression_weights.insert("blink".to_owned(), blink);
        state.expression_weights.insert("oh".to_owned(), mouth_open.clamp(0.0, 1.0));
    }
    state.last_packet = Some(Instant::now());
    true
}

/// 启动 UDP 接收线程，返回绑定的本地地址（供测试/日志）
pub fn start_receiver(bind: &str, sts: SharedTrackingState) -> std::io::Result<std::net::SocketAddr> {
    let socket = std::net::UdpSocket::bind(bind)?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    let addr = socket.local_addr()?;
    let mut counter = 0u64;
    // 面捕平滑系数（COLONQ_SMOOTH=0~0.95，越大越平滑但延迟越高；默认 0.5）
    let smooth = utils::env_parse::<f32>("COLONQ_SMOOTH")
        .map(|v| v.clamp(0.0, 0.95)).unwrap_or(0.5);
    if smooth > 0.001 {
        log::info!("面捕平滑: 系数 {}（越大越平滑，延迟越高）", smooth);
    }
    std::thread::spawn(move || {
        let diag = utils::env_flag("COLONQ_TRACK_DEBUG", false);
        let flip_mode: u8 = utils::env_parse("COLONQ_FLIP").unwrap_or(1);
        let offset_rad = utils::env_csv3("COLONQ_ROT_OFFSET")
            .map(|(x, y, z)| (x.to_radians(), y.to_radians(), z.to_radians()))
            .unwrap_or((0.0, 0.0, 0.0));
        // COLONQ_PITCH_SIGN=-1：低头/抬头方向反转（俯仰轴镜像）
        // COLONQ_QSIGN="x,y,z"：分别翻转原始四元数的三个分量（默认 1,1,1）
        // COLONQ_SWAP_PY=1：交换补偿欧拉角的 pitch 和 yaw（实测：点头↔摇头轴错位时用）
        let pitch_flip = utils::env_flag("COLONQ_PITCH_FLIP", false);
        let (qx_sign, qy_sign, qz_sign) = utils::env_csv3("COLONQ_QSIGN").unwrap_or((1.0, 1.0, 1.0));
        let pitch_sign: f32 = utils::env_parse("COLONQ_PITCH_SIGN").unwrap_or(qx_sign);
        let mut buf = [0u8; 65535];
        loop {
            match socket.recv_from(&mut buf) {
                Ok((len, src)) => {
                    counter += 1;
                    if !parse_packet(&buf[..len], &sts, diag, counter, flip_mode, offset_rad, pitch_sign, qy_sign, qz_sign, pitch_flip, smooth) {
                        log::debug!("忽略无效面捕包（{} bytes，来自 {}）", len, src);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                    continue; // 面捕端暂停时保持接收，等待恢复
                }
                Err(e) => {
                    log::warn!("面捕 UDP 接收失败（{}），停止接收线程", e);
                    return;
                }
            }
        }
    });
    Ok(addr)
}

pub fn run(sts: SharedTrackingState) {
    // 默认只收本机；跨机面捕时设为例如 COLONQ_TRACK_BIND=0.0.0.0:11573
    // （注意防火墙：UDP 11573 只应对面捕服务器的 IP 开放）
    let bind = std::env::var("COLONQ_TRACK_BIND").unwrap_or_else(|_| "127.0.0.1:11573".into());
    match start_receiver(&bind, sts) {
        Ok(addr) => log::info!("面捕 UDP 接收: {}（仅接收标准 1785 字节单脸包）", addr),
        Err(e) => log::warn!("面捕端口 {} 绑定失败（{}），无追踪数据", bind, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按 facetracker.py 的字段顺序构造单脸数据包
    fn make_packet(blink_l: f32, blink_r: f32, quat: [f32; 4], mouth_open: f32) -> Vec<u8> {
        let mut packet = vec![0u8; PACKET_LEN];
        let put = |packet: &mut Vec<u8>, offset: usize, value: f32| {
            byteorder::LittleEndian::write_f32(&mut packet[offset..], value);
        };
        put(&mut packet, BLINK_OFFSET, blink_l);
        put(&mut packet, BLINK_OFFSET + 4, blink_r);
        put(&mut packet, QUAT_OFFSET, quat[0]);
        put(&mut packet, QUAT_OFFSET + 4, quat[1]);
        put(&mut packet, QUAT_OFFSET + 8, quat[2]);
        put(&mut packet, QUAT_OFFSET + 12, quat[3]);
        put(&mut packet, FEATURE_MOUTH_OPEN, mouth_open);
        packet
    }

    fn wait_tracking(sts: &SharedTrackingState) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if sts.lock().unwrap().last_packet.is_some() { break }
            assert!(Instant::now() < deadline, "未收到面捕包");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn valid_packet_updates_state_with_correct_offsets() {
        let sts = std::sync::Arc::new(std::sync::Mutex::new(TrackingState::new()));
        let addr = start_receiver("127.0.0.1:0", sts.clone()).unwrap();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.send_to(&make_packet(0.25, 0.75, [0.0, 0.0, 0.0, 1.0], 0.42), addr).unwrap();
        wait_tracking(&sts);
        let s = sts.lock().unwrap();
        assert_eq!(s.expression_weights["blink"], 0.5);
        assert_eq!(s.expression_weights["oh"], 0.42);
        assert!(s.tracking_active());
        assert_ne!(s.expression_weights["oh"], 10.25);
    }

    #[test]
    fn identity_quaternion_plus_zflip_matches_calibrated_neutral() {
        // 校准结论（tools/calibrate_rotation.py 档3）：包内单位四元数 + 接收器的
        // 轴补偿 + Z轴180° = "正脸"。发送单位四元数时，接收到的姿态应等于
        // 上游补偿 × Z_FLIP —— 也就是 calibrate_rotation 档3 发送值的共轭逆。
        let sts = std::sync::Arc::new(std::sync::Mutex::new(TrackingState::new()));
        let addr = start_receiver("127.0.0.1:0", sts.clone()).unwrap();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.send_to(&make_packet(0.0, 0.0, [0.0, 0.0, 0.0, 1.0], 0.0), addr).unwrap();
        wait_tracking(&sts);
        let orientation = sts.lock().unwrap().orientation;
        // 档3（calibrate_rotation.py）实测让头部回正：接收器对单位四元数的输出即中性姿态
        let expected = glam::Quat::from_xyzw(-0.7071068, -0.7071068, -3.0908623e-8, 3.0908623e-8);
        assert!((orientation.dot(expected) - 1.0).abs() < 1e-4,
                "orientation={:?} expected={:?}", orientation, expected);
    }

    #[test]
    fn malformed_packets_are_ignored() {
        let sts = std::sync::Arc::new(std::sync::Mutex::new(TrackingState::new()));
        let addr = start_receiver("127.0.0.1:0", sts.clone()).unwrap();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.send_to(&[0u8; 100], addr).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!sts.lock().unwrap().tracking_active());
        let bad = make_packet(0.5, 0.5, [f32::NAN, 0.0, 0.0, 1.0], 0.5);
        socket.send_to(&bad, addr).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!sts.lock().unwrap().tracking_active());
        socket.send_to(&make_packet(0.0, 0.0, [0.0, 0.0, 0.0, 1.0], 0.9), addr).unwrap();
        wait_tracking(&sts);
        assert_eq!(sts.lock().unwrap().expression_weights["oh"], 0.9);
    }

    #[test]
    fn mouth_open_is_clamped() {
        let sts = std::sync::Arc::new(std::sync::Mutex::new(TrackingState::new()));
        let addr = start_receiver("127.0.0.1:0", sts.clone()).unwrap();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.send_to(&make_packet(0.0, 0.0, [0.0, 0.0, 0.0, 1.0], 5.0), addr).unwrap();
        wait_tracking(&sts);
        assert_eq!(sts.lock().unwrap().expression_weights["oh"], 1.0);
    }
}
