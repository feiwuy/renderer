use std::time::{Duration, Instant};

use byteorder::ByteOrder;

use crate::utils;

pub type SharedTrackingState = std::sync::Arc<std::sync::Mutex<TrackingState>>;

/// 面捕数据在多久内算“有效”；超时后回退到待机动画
pub const TRACKING_TIMEOUT: Duration = Duration::from_secs(2);

/// 平滑系数的标定基准：`COLONQ_SMOOTH` 按"每 1/30 秒收到一个包"来标定。
const SMOOTH_REF_DT: f32 = 1.0 / 30.0;

/// 把 `COLONQ_SMOOTH`（0~0.95，越大越平滑）换算成混合系数。
///
/// 旧做法是每个包固定混 `1 - smooth`：稳态手心感固定，但面捕端帧率一变手感就变。
/// 这里改成按时间做指数衰减 `alpha = 1 - smooth^(dt / 标称间隔)`：
///
/// - `nominal_dt` 用**实测的包间隔**（见 TrackingState::nominal_dt），因此稳态下
///   `dt ≈ nominal_dt`，alpha 恰好等于旧的 `1 - smooth` —— **手感与原版完全一致**，
///   不会因为面捕端是 30fps 还是 60fps 而变得迟钝。
/// - 只有丢包、掉帧（`dt` 明显大于标称值）时才额外补偿，长时间断流后几乎直接吸附。
/// - `nominal_dt` 还没测出来（首包）时退回 `SMOOTH_REF_DT`。
pub fn smooth_alpha(smooth: f32, dt: f32, nominal_dt: f32) -> f32 {
    let s = smooth.clamp(0.0, 0.95);
    if s <= 0.001 {
        return 1.0; // 关闭平滑
    }
    if !dt.is_finite() || dt <= 0.0 {
        return 1.0 - s; // dt 不可用（首包/时钟异常）时退回旧公式
    }
    let reference = if nominal_dt > 0.001 {
        nominal_dt
    } else {
        SMOOTH_REF_DT
    };
    (1.0 - s.powf(dt / reference)).clamp(0.0, 1.0)
}

/// facetracker.py 单脸数据包长度（见 tools/openseeface-server.patch 的字段表）
pub const PACKET_LEN: usize = 1785;

/// 上游轴补偿之外的翻转模式（COLONQ_FLIP 选择）：
/// 0=无  1=Z轴180°（左右翻转）  2=Y轴180°（掉头）  3=X轴180°（前滚翻）  4=Z+Y
/// 真人面捕实测：档3（Z轴180°）修正了静态测试，但真人数据需要换档时用此开关。
fn flip_quat(mode: u8) -> glam::Quat {
    match mode {
        1 => glam::Quat::from_xyzw(0.0, 0.0, 1.0, 0.0), // Z 180°
        2 => glam::Quat::from_xyzw(0.0, 1.0, 0.0, 0.0), // Y 180°
        3 => glam::Quat::from_xyzw(1.0, 0.0, 0.0, 0.0), // X 180°
        4 => glam::Quat::from_xyzw(0.0, 0.0, 1.0, 0.0) * glam::Quat::from_xyzw(0.0, 1.0, 0.0, 0.0), // Z×Y
        5 => glam::Quat::from_xyzw(1.0, 0.0, 0.0, 0.0) * glam::Quat::from_xyzw(0.0, 1.0, 0.0, 0.0), // X×Y
        6 => glam::Quat::from_xyzw(1.0, 0.0, 0.0, 0.0) * glam::Quat::from_xyzw(0.0, 0.0, 1.0, 0.0), // X×Z
        _ => glam::Quat::IDENTITY,
    }
}

// ---- 包内字段偏移（facetracker.py 的 struct.pack 顺序，小端）----
// time(8) | face_id(4) | width(4) | height(4) | eye_blink[2](8) | success(1)
// | pnp_error(4) | quaternion(16) | euler(12) | translation(12)
// | confidence 68*4 | points 68*8 | pts_3d 70*12 | features 14*4
const BLINK_OFFSET: usize = 20; // 两眼张开程度，0..1，越大越睁开
const QUAT_OFFSET: usize = 33; // 头部姿态四元数 x,y,z,w

/// 14 个特征 f32 的起始；顺序与 facetracker.py 的 `features` 列表完全一致，
/// 1729 + 14*4 = 1785 = 整包长度，也就是说**每帧都在这 56 字节里送过来**。
const FEATURES_OFFSET: usize = 1729;

// 每个特征都是 OpenSeeFace 的 `Feature` 做自适应归一化之后的输出（见
// deps/OpenSeeFace/tracker.py 的 Feature 类）：以**使用者自己的中位数**为 0 点，
// 超出 min/max 时饱和到 ∓1，所以天然落在约 -1..1 且**正负代表方向**。
const F_EYE_L: usize = FEATURES_OFFSET;
const F_EYE_R: usize = FEATURES_OFFSET + 4;
const F_BROW_STEEP_L: usize = FEATURES_OFFSET + 8;
const F_BROW_UPDOWN_L: usize = FEATURES_OFFSET + 12;
const F_BROW_QUIRK_L: usize = FEATURES_OFFSET + 16;
const F_BROW_STEEP_R: usize = FEATURES_OFFSET + 20;
const F_BROW_UPDOWN_R: usize = FEATURES_OFFSET + 24;
const F_BROW_QUIRK_R: usize = FEATURES_OFFSET + 28;
const F_MOUTH_CORNER_L: usize = FEATURES_OFFSET + 32;
const F_MOUTH_INOUT_L: usize = FEATURES_OFFSET + 36;
const F_MOUTH_CORNER_R: usize = FEATURES_OFFSET + 40;
const F_MOUTH_INOUT_R: usize = FEATURES_OFFSET + 44;
const F_MOUTH_OPEN: usize = FEATURES_OFFSET + 48;
const F_MOUTH_WIDE: usize = FEATURES_OFFSET + 52;

/// 表情槽位顺序 —— 必须与 `context.rs` 的 `attrib_expressions`（名字 → 顶点属性槽
/// 5..15）以及 `assets/shader.vert` 里的 `expression_<名字>_weight` 一一对应。
pub const EXPR_NAMES: [&str; 11] = [
    "happy",
    "angry",
    "sad",
    "relaxed",
    "surprised",
    "aa",
    "ih",
    "ou",
    "ee",
    "oh",
    "blink",
];

/// 名字 → 槽位下标（不认识的名字返回 None，调用方跳过即可）
pub fn expr_index(name: &str) -> Option<usize> {
    EXPR_NAMES.iter().position(|n| *n == name)
}

/// 11 个表情槽的权重，按 `EXPR_NAMES` 顺序。
///
/// 以前是 `HashMap<String, f32>`：每收到一个包要做两次 `String` 分配、渲染循环里每帧
/// 还要做 11 次字符串哈希。改成固定数组后这些都省了，而且编译期就能保证槽位对得上。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExprWeights(pub [f32; 11]);

impl ExprWeights {
    /// 按名字取值。主循环为了省哈希是直接下标访问的，这里是给测试和调试用的。
    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> f32 {
        expr_index(name).map(|i| self.0[i]).unwrap_or(0.0)
    }

    pub fn set(&mut self, name: &str, v: f32) {
        if let Some(i) = expr_index(name) {
            self.0[i] = v;
        }
    }
}

/// 一帧面捕里的 14 个几何特征（含义与符号见 `compute_weights`）
#[derive(Clone, Copy, Debug, Default)]
pub struct RawFeatures {
    pub eye_l: f32,
    pub eye_r: f32,
    pub brow_steep_l: f32,
    pub brow_updown_l: f32,
    pub brow_quirk_l: f32,
    pub brow_steep_r: f32,
    pub brow_updown_r: f32,
    pub brow_quirk_r: f32,
    pub mouth_corner_l: f32,
    pub mouth_inout_l: f32,
    pub mouth_corner_r: f32,
    pub mouth_inout_r: f32,
    pub mouth_open: f32,
    pub mouth_wide: f32,
}

impl RawFeatures {
    /// 从包里读出 14 个特征；任一不是有限数（3D 估计失败时会发 NaN）就返回 None
    fn read(buf: &[u8]) -> Option<Self> {
        let rd = |off: usize| byteorder::LittleEndian::read_f32(&buf[off..]);
        let f = Self {
            eye_l: rd(F_EYE_L),
            eye_r: rd(F_EYE_R),
            brow_steep_l: rd(F_BROW_STEEP_L),
            brow_updown_l: rd(F_BROW_UPDOWN_L),
            brow_quirk_l: rd(F_BROW_QUIRK_L),
            brow_steep_r: rd(F_BROW_STEEP_R),
            brow_updown_r: rd(F_BROW_UPDOWN_R),
            brow_quirk_r: rd(F_BROW_QUIRK_R),
            mouth_corner_l: rd(F_MOUTH_CORNER_L),
            mouth_inout_l: rd(F_MOUTH_INOUT_L),
            mouth_corner_r: rd(F_MOUTH_CORNER_R),
            mouth_inout_r: rd(F_MOUTH_INOUT_R),
            mouth_open: rd(F_MOUTH_OPEN),
            mouth_wide: rd(F_MOUTH_WIDE),
        };
        let all_finite = [
            f.eye_l,
            f.eye_r,
            f.brow_steep_l,
            f.brow_updown_l,
            f.brow_quirk_l,
            f.brow_steep_r,
            f.brow_updown_r,
            f.brow_quirk_r,
            f.mouth_corner_l,
            f.mouth_inout_l,
            f.mouth_corner_r,
            f.mouth_inout_r,
            f.mouth_open,
            f.mouth_wide,
        ]
        .iter()
        .all(|v| v.is_finite());
        if all_finite {
            Some(f)
        } else {
            None
        }
    }

    /// 由眼特征算出的闭眼程度。注意包里 offset 20 的 `eye_blink[2]` 就是
    /// facetracker 用 `1 - clamp(-eye_l/r)` 预算好的同一个量（同一个 Feature 实例），
    /// 所以这里和那边读到的是同源同值——两路都取，等于给"到底有没有闭眼"加了个交叉校验。
    fn blink_from_eyes(&self) -> f32 {
        positive(-(self.eye_l + self.eye_r) * 0.5)
    }
}

/// 夹到 0..1
fn clamp01(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// 只取正向部分：`pos(x) = clamp(x, 0, 1)`
fn positive(v: f32) -> f32 {
    clamp01(v)
}

fn avg(a: f32, b: f32) -> f32 {
    (a + b) * 0.5
}

/// 梯形窗：`x <= a` 为 0，`b..=c` 为 1，`x >= d` 为 0，两侧线性过渡。
/// 用来把"张嘴程度"切成几档，各档对应不同的元音口型。
fn trapezoid(x: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    if x <= a || x >= d {
        0.0
    } else if x < b {
        (x - a) / (b - a)
    } else if x <= c {
        1.0
    } else {
        (d - x) / (d - c)
    }
}

/// 表情映射的可调参数（启动时从环境变量读一次，之后每条包都按它算）
#[derive(Clone, Copy, Debug)]
pub struct ExprOptions {
    /// 笑的时候元音组让出多少嘴部形变（0~1）。
    /// VRM 的 happy morph 本身就把嘴拉开了，再叠 oh/aa 会变成"血盆大口"——
    /// 这就是"大笑和张嘴绑在一起"的来源。默认 0.7 表示笑到满时元音口型只剩 30%。
    pub mouth_override: f32,
    /// 眼睛睁大对"惊讶"的贡献（抬眉看不出来时，睁大眼睛是第二个信号）
    pub eye_surprise: f32,
    /// 张嘴时把"嘴角上扬"信号扣掉多少（乘以张口程度）。张嘴会让上唇抬高，容易被误判成笑。
    pub open_vs_smile: f32,
    /// 笑到眯眼（`>＜`）。开启后 `relaxed` 槽改绑"整只眼闭上"的 morph、
    /// `ih` 槽改绑"眼珠变白"的 morph（改绑在 vrm.rs），权重按 happy 强度斜坡给出。
    /// 依据 VRM 1.0 的程序化表情组规范：Blink/Mouth 组之外的情绪表情可以带出眼部形变。
    pub eye_laugh: bool,
    /// 斜坡起点：happy 超过这个值才开始眯眼
    pub laugh_lo: f32,
    /// 斜坡终点：happy 到这个值时眯眼拉满
    pub laugh_hi: f32,
    /// 眯眼（`relaxed` 槽）的最大权重；设 0 就等于不要眯眼
    pub laugh_close: f32,
    /// 眼珠变白（`ih` 槽）的最大权重；觉得"眼睛整个没了"就把这个调小或设 0
    pub laugh_iris: f32,
    /// 嘴部灵敏度：`mouth_open` 特征先乘这个再夹到 0~1。
    /// 大于 1 = 小张口就有明显反应（此前面捕要"张很大"才有嘴型）。
    pub mouth_gain: f32,
    /// 嘴部响应曲线（gamma）。小于 1 会抬起中低段，大于 1 更"线性到后半段"。
    pub mouth_curve: f32,
}

impl Default for ExprOptions {
    fn default() -> Self {
        Self {
            mouth_override: 0.7,
            eye_surprise: 0.55,
            open_vs_smile: 0.45,
            eye_laugh: false,
            laugh_lo: 0.55,
            laugh_hi: 0.95,
            laugh_close: 1.0,
            laugh_iris: 0.9,
            mouth_gain: 1.8,
            mouth_curve: 1.0,
        }
    }
}

impl ExprOptions {
    pub fn from_env() -> Self {
        let d = Self::default();
        let read = |name: &str, fallback: f32| {
            utils::env_parse::<f32>(name)
                .map(|v| v.clamp(0.0, 2.0))
                .unwrap_or(fallback)
        };
        Self {
            mouth_override: read("COLONQ_MOUTH_OVERRIDE", d.mouth_override),
            eye_surprise: read("COLONQ_EYE_SURPRISE", d.eye_surprise),
            open_vs_smile: read("COLONQ_OPEN_VS_SMILE", d.open_vs_smile),
            eye_laugh: utils::env_flag("COLONQ_EYE_LAUGH", d.eye_laugh),
            laugh_lo: read("COLONQ_EYE_LAUGH_LO", d.laugh_lo),
            laugh_hi: read("COLONQ_EYE_LAUGH_HI", d.laugh_hi),
            laugh_close: read("COLONQ_EYE_LAUGH_CLOSE", d.laugh_close),
            laugh_iris: read("COLONQ_EYE_LAUGH_IRIS", d.laugh_iris),
            mouth_gain: read("COLONQ_MOUTH_GAIN", d.mouth_gain).max(0.01),
            mouth_curve: read("COLONQ_MOUTH_CURVE", d.mouth_curve).max(0.01),
        }
    }
}

/// 14 个面捕特征 → 11 个表情槽权重。
///
/// 输入全部是相对使用者自身中位数的归一化量（约 -1..1，正负代表方向），
/// 所以这里只做"分档 + 组合"，不需要标定绝对量。输出已夹在 0..1。
///
/// 映射表（每一项后面是它用到的特征）：
///
/// | 槽位        | 来源                                                      |
/// |-------------|-----------------------------------------------------------|
/// | `blink`     | `eye_l`/`eye_r`（+ 包里预算的 eye_blink，二者同源）        |
/// | `surprised` | `eyebrow_updown_l/r` 抬眉（+ `eyebrow_quirk` 歪眉辅助）    |
/// | `angry`     | `eyebrow_steepness_l/r` 内侧下压 + `eyebrow_updown` 压眉   |
/// | `happy`     | `mouth_corner_updown_l/r` 大幅上扬                        |
/// | `relaxed`   | 同一信号的小幅区间（小微笑 = 放松，大微笑才升级成 happy）  |
/// | `sad`       | `mouth_corner_updown_l/r` 下垂 + `steepness` 内侧上挑(困り眉) |
/// | `aa`        | `mouth_open` 大 + `mouth_wide` 宽（あ）                    |
/// | `oh`        | `mouth_open` 大 + 嘴不宽（お）——**保留"张嘴就有 oh"的旧手感** |
/// | `ou`        | 嘴比平时窄（嘟嘴）+ 少量张嘴（う）                        |
/// | `ee`        | `mouth_open` 小 + `mouth_wide`/`mouth_corner_inout` 宽（え）|
/// | `ih`        | `mouth_open` 中 + 宽（い）                                |
///
/// 系数都是第一版凭经验给的，不合手感可以用 `COLONQ_EXPR_GAINS` 单独调，
/// 或者直接改这里（每个槽一行，注释写清了各自吃哪些特征）。
pub fn compute_weights(f: &RawFeatures, opts: &ExprOptions) -> ExprWeights {
    // --- 眉毛：抬/压/内侧倾角/歪斜 ---
    let brow_up = positive(avg(f.brow_updown_l, f.brow_updown_r));
    let brow_down = positive(-avg(f.brow_updown_l, f.brow_updown_r));
    let steep = avg(f.brow_steep_l, f.brow_steep_r); // 负 = 内侧下压（凶），正 = 内侧上挑（困）
    let brow_tilt_in = positive(-steep);
    let brow_tilt_up = positive(steep);
    let quirk = positive(avg(f.brow_quirk_l, f.brow_quirk_r));

    // --- 嘴型：张口程度 + 横向宽度 ---
    //
    // `mouth_open` 是 OpenSeeFace 相对自己中位数归一化后的量：小张口时只有零点几，
    // 张到"比自己平时的最大值还大"才到 1。所以直接当权重用会显得"要张很大才有嘴型"。
    // 这里先乘灵敏度（默认 1.8）再夹取，中低段用 gamma 曲线抬起来。
    let open = positive(f.mouth_open * opts.mouth_gain).powf(opts.mouth_curve);

    // --- 嘴角：上扬 / 下垂 ---
    // 张嘴时上唇会整体抬高，`mouth_corner_updown` 于是也会变正 —— 如果直接当"笑"，
    // 就会变成"一张嘴就在笑"。这里用张口程度把它压回去（这就是反向的那个耦合）。
    let corner_up_raw = positive(avg(f.mouth_corner_l, f.mouth_corner_r));
    let corner_up = (corner_up_raw - opts.open_vs_smile * open).max(0.0);
    let corner_down = positive(-avg(f.mouth_corner_l, f.mouth_corner_r));

    // --- 眼睛睁大（比平时睁得更大 = 惊讶的第二个信号）---
    let eye_wide = positive(avg(f.eye_l, f.eye_r));
    let wide_raw = positive(f.mouth_wide);
    let narrow = positive(-f.mouth_wide);
    // 嘴角外拉也是"嘴宽"的另一个视角，权重给小一点
    let inout = positive(avg(f.mouth_inout_l, f.mouth_inout_r));
    let wide = clamp01(wide_raw * 0.75 + inout * 0.25);

    // 嘴巴三档开合，用来区分元音口型
    let open_low = trapezoid(open, 0.02, 0.10, 0.22, 0.40); // 微张
    let open_mid = trapezoid(open, 0.18, 0.35, 0.55, 0.75); // 半张

    let mut w = ExprWeights::default();
    // 情绪组
    w.set(
        "surprised",
        (brow_up * 1.0 + eye_wide * opts.eye_surprise + quirk * 0.15 + brow_tilt_up * 0.10)
            .min(1.0),
    );
    w.set(
        "angry",
        (brow_tilt_in * 0.85 + brow_down * 0.35 + quirk * 0.20).min(1.0),
    );
    // 小微笑算"放松"，大幅上扬才算"开心"（两段梯形自然交叉，不会同时打满）
    w.set("happy", trapezoid(corner_up, 0.30, 0.75, 2.0, 2.0));
    w.set(
        "relaxed",
        trapezoid(corner_up, 0.05, 0.35, 0.50, 0.90) * (1.0 - wide * 0.3),
    );
    w.set("sad", (corner_down * 1.0 + brow_tilt_up * 0.30).min(1.0));

    // 元音组（口型 = 张口程度 × 横向宽窄 的二维分布）
    // 笑的时候让位：happy 的 morph 自己就会把嘴拉开，再叠 oh/aa 就是一张血盆大口。
    let vowel_yield = (1.0 - opts.mouth_override * w.get("happy")).clamp(0.0, 1.0);
    w.set("aa", (open * wide * 0.95 * vowel_yield).min(1.0));
    w.set("oh", (open * (1.0 - 0.85 * wide) * vowel_yield).min(1.0));
    w.set(
        "ou",
        (narrow * (0.35 + 0.65 * open) * 0.90 * vowel_yield).min(1.0),
    );
    w.set("ih", (open_mid * wide * 0.70 * vowel_yield).min(1.0));
    w.set("ee", (open_low * wide * 0.60 * vowel_yield).min(1.0));

    // 笑到眯眼（`>＜` 笑眼）：借用 relaxed / ih 两个槽位（vrm.rs 已把它们改绑到
    // "整只眼闭上" 和 "眼珠变白" 的 morph），权重按 happy 的斜坡来 ——
    // 小小一笑只动嘴角，笑到一定程度眼角才眯起来，这才是动画里的观感。
    if opts.eye_laugh {
        let t = ramp(w.get("happy"), opts.laugh_lo, opts.laugh_hi);
        w.set("relaxed", t * opts.laugh_close);
        w.set("ih", t * opts.laugh_iris);
    }
    w
}

/// 线性斜坡：x<=lo 为 0，x>=hi 为 1
fn ramp(x: f32, lo: f32, hi: f32) -> f32 {
    if !x.is_finite() || hi <= lo {
        return if x.is_finite() && x >= hi { 1.0 } else { 0.0 };
    }
    clamp01((x - lo) / (hi - lo))
}

pub struct TrackingState {
    /// 11 个表情槽的当前权重（按 `EXPR_NAMES` 顺序）
    pub expression_weights: ExprWeights,
    pub orientation: glam::Quat,
    /// 最近一次收到有效面捕包的时间；None = 从未收到
    pub last_packet: Option<Instant>,
    /// 实测的包间隔（秒）：平滑系数的"标称周期"，让手感不随面捕端帧率变化
    pub nominal_dt: f32,
}

impl TrackingState {
    pub fn new() -> Self {
        Self {
            expression_weights: ExprWeights::default(),
            orientation: glam::Quat::IDENTITY,
            last_packet: None,
            nominal_dt: 0.0,
        }
    }

    /// 最近 TRACKING_TIMEOUT 内收到过有效面捕数据？
    pub fn tracking_active(&self) -> bool {
        self.last_packet
            .map(|t| t.elapsed() < TRACKING_TIMEOUT)
            .unwrap_or(false)
    }
}

/// 解析单脸数据包并写入共享状态；返回 false 表示包无效（长度/数值异常）
// 参数多是有意的：这是接收线程里每条包都要走的热路径，拆成结构体会引入额外拷贝
#[allow(clippy::too_many_arguments)]
fn parse_packet(
    buf: &[u8],
    sts: &SharedTrackingState,
    debug_level: u8,
    counter: u64,
    flip_mode: u8,
    offset_rad: (f32, f32, f32),
    pitch_sign: f32,
    qy_sign: f32,
    qz_sign: f32,
    pitch_flip: bool,
    smooth: f32,
    opts: &ExprOptions,
) -> bool {
    if buf.len() < PACKET_LEN || !buf.len().is_multiple_of(PACKET_LEN) {
        return false; // 多脸包按 1785 的整数倍拼接；只取第一张脸
    }
    let left_blinking = byteorder::LittleEndian::read_f32(&buf[BLINK_OFFSET..]);
    let right_blinking = byteorder::LittleEndian::read_f32(&buf[BLINK_OFFSET + 4..]);
    // 包里 14 个特征（offset 1729 起）；不是有限数就当无效包
    let Some(feats) = RawFeatures::read(buf) else {
        return false;
    };
    let mouth_open = feats.mouth_open;
    let quat = (
        byteorder::LittleEndian::read_f32(&buf[QUAT_OFFSET..]),
        byteorder::LittleEndian::read_f32(&buf[QUAT_OFFSET + 4..]),
        byteorder::LittleEndian::read_f32(&buf[QUAT_OFFSET + 8..]),
        byteorder::LittleEndian::read_f32(&buf[QUAT_OFFSET + 12..]),
    );
    // 3D 估计失败时 facetracker 会发全零/无效数值；丢弃这类包
    if !(quat.0.is_finite()
        && quat.1.is_finite()
        && quat.2.is_finite()
        && quat.3.is_finite()
        && left_blinking.is_finite()
        && right_blinking.is_finite()
        && mouth_open.is_finite())
    {
        return false;
    }
    // 闭眼程度：先看包里 offset 20 那两个预算好的值，再和眼特征算出来的取较大者
    // （二者同源同值，见 RawFeatures::blink_from_eyes），只为交叉校验。
    let blink = (1.0 - (left_blinking + right_blinking) / 2.0)
        .clamp(0.0, 1.0)
        .max(feats.blink_from_eyes());
    // 14 个特征 → 11 个表情槽（纯函数，不依赖历史值）
    let mut target = compute_weights(&feats, opts);
    target.set("blink", blink);

    // COLONQ_TRACK_DEBUG=1：打印关键原始值；=2：连 14 个特征和 11 个权重一起打。
    // 只在前 3 个包和之后每 100 个包打，避免把日志刷爆。
    if debug_level > 0 && (counter <= 3 || counter.is_multiple_of(100)) {
        log::info!(
            "面捕包 #{}: blink L/R={:.3}/{:.3} mouth_open={:+.3} quat=({:.3},{:.3},{:.3},{:.3})",
            counter,
            left_blinking,
            right_blinking,
            mouth_open,
            quat.0,
            quat.1,
            quat.2,
            quat.3
        );
        if debug_level >= 2 {
            log::info!(
                "  特征 眼={:+.2}/{:+.2} 眉(倾/抬/歪) 左={:+.2}/{:+.2}/{:+.2} 右={:+.2}/{:+.2}/{:+.2} 嘴角 上/拉 左={:+.2}/{:+.2} 右={:+.2}/{:+.2} 张嘴={:+.2} 嘴宽={:+.2}",
                feats.eye_l,
                feats.eye_r,
                feats.brow_steep_l,
                feats.brow_updown_l,
                feats.brow_quirk_l,
                feats.brow_steep_r,
                feats.brow_updown_r,
                feats.brow_quirk_r,
                feats.mouth_corner_l,
                feats.mouth_inout_l,
                feats.mouth_corner_r,
                feats.mouth_inout_r,
                feats.mouth_open,
                feats.mouth_wide
            );
            let pairs: Vec<String> = EXPR_NAMES
                .iter()
                .zip(target.0.iter())
                .filter(|(_, w)| **w > 0.01)
                .map(|(n, w)| format!("{}={:.2}", n, w))
                .collect();
            log::info!(
                "  权重 {}",
                if pairs.is_empty() {
                    "（全部为 0 = 中性）".to_string()
                } else {
                    pairs.join(" ")
                }
            );
        }
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
        std::f32::consts::PI + offset_rad.0,
        offset_rad.1,
        std::f32::consts::PI / 2.0 + offset_rad.2,
    );
    let mut new = glam::Quat::from_xyzw(qx, qy, qz, qw)
        .mul_quat(glam::Quat::from_euler(
            glam::EulerRot::XYZ,
            comp_x,
            comp_y,
            comp_z,
        ))
        .mul_quat(flip_quat(flip_mode));
    // COLONQ_PITCH_FLIP=1：真人点头方向与模型相反时，把最终姿态欧拉分解后的
    // pitch（俯仰）分量取反再重组——只翻转俯仰，不破坏偏航和滚转
    if pitch_flip {
        let (r, p, y) = new.to_euler(glam::EulerRot::YXZ);
        new = glam::Quat::from_euler(glam::EulerRot::YXZ, r, -p, y);
    }
    let now = Instant::now();
    let mut state = sts.lock().unwrap();
    // 指数平滑：用上一次姿态与新姿态做球面插值（slerp），smooth 越大越平滑。
    // 首包（last_packet 为 None）不平滑——没有可依据的上一次姿态。
    // 混合系数按"距上一包的间隔"换算（见 smooth_alpha），因此包率变化不影响手感。
    let dt = state
        .last_packet
        .map(|t| now.saturating_duration_since(t).as_secs_f32())
        .unwrap_or(0.0);
    // 用实测包间隔做标称周期（慢速 EMA，抗抖动）：稳态下 alpha 就等于 1 - smooth，
    // 即和改动前的手感一致；只有丢包/掉帧时才会额外补偿。
    if dt > 0.002 && dt < 0.5 {
        state.nominal_dt = if state.nominal_dt <= 0.001 {
            dt
        } else {
            state.nominal_dt + (dt - state.nominal_dt) * 0.05
        };
    }
    let alpha = smooth_alpha(smooth, dt, state.nominal_dt);
    // 每 300 个包报告一次实测包率 + 平滑折算出的延迟，方便判断延迟来源
    if counter.is_multiple_of(300) && state.nominal_dt > 0.001 {
        let steady_alpha = 1.0 - smooth.clamp(0.0, 0.95);
        // 一阶指数平滑的时间常数 τ = -dt / ln(1 - alpha)，可粗略看作引入的延迟
        let tau_ms = if steady_alpha >= 0.999 {
            0.0
        } else {
            -state.nominal_dt / (1.0 - steady_alpha).ln() * 1000.0
        };
        log::info!("面捕包率 ≈ {:.1} Hz（间隔 {:.1} ms），smooth={} → 每包混合 {:.2}，平滑约引入 {:.0} ms 延迟",
                   1.0 / state.nominal_dt, state.nominal_dt * 1000.0, smooth, steady_alpha, tau_ms);
    }
    let smoothing = alpha < 1.0 && state.last_packet.is_some();
    // 首包不平滑（没有可依据的历史）；之后姿态走 slerp，权重走同一套 alpha 的线性插值
    state.orientation = if smoothing {
        state.orientation.slerp(new, alpha)
    } else {
        new
    };
    if smoothing {
        for (cur, tgt) in state.expression_weights.0.iter_mut().zip(target.0.iter()) {
            *cur += (*tgt - *cur) * alpha;
        }
    } else {
        state.expression_weights = target;
    }
    state.last_packet = Some(now);
    true
}

/// `COLONQ_TRACK_DEBUG` 的调试等级：
/// `0/未设置` = 安静；`1` = 打印关键原始值；`2`+ = 连 14 个特征和 11 个表情权重一起打。
/// `true/on/yes` 这类开关写法按 1 处理（保持和 `env_flag` 一致的直觉）。
fn track_debug_level() -> u8 {
    match std::env::var("COLONQ_TRACK_DEBUG") {
        Err(_) => 0,
        Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "" | "0" | "false" | "off" | "no" => 0,
            t => t.parse::<u8>().unwrap_or(1),
        },
    }
}

/// 启动 UDP 接收线程，返回绑定的本地地址（供测试/日志）
pub fn start_receiver(
    bind: &str,
    sts: SharedTrackingState,
) -> std::io::Result<std::net::SocketAddr> {
    let socket = std::net::UdpSocket::bind(bind)?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    let addr = socket.local_addr()?;
    let mut counter = 0u64;
    // 面捕平滑系数（COLONQ_SMOOTH=0~0.95，越大越平滑但延迟越高；默认 0.5）
    let smooth = utils::env_parse::<f32>("COLONQ_SMOOTH")
        .map(|v| v.clamp(0.0, 0.95))
        .unwrap_or(0.5);
    if smooth > 0.001 {
        log::info!("面捕平滑: 系数 {}（越大越平滑，延迟越高）", smooth);
    }
    std::thread::spawn(move || {
        let debug_level = track_debug_level();
        if debug_level >= 2 {
            log::info!(
                "面捕调试等级 2：每个采样点打印 14 个特征 + 11 个表情权重（日志会明显变多）"
            );
        }
        let expr_opts = ExprOptions::from_env();
        log::info!(
            "表情映射参数: 笑的嘴部压制={} 睁眼→惊讶={} 张嘴→非笑={}",
            expr_opts.mouth_override,
            expr_opts.eye_surprise,
            expr_opts.open_vs_smile
        );
        let flip_mode: u8 = utils::env_parse("COLONQ_FLIP").unwrap_or(1);
        let offset_rad = utils::env_csv3("COLONQ_ROT_OFFSET")
            .map(|(x, y, z)| (x.to_radians(), y.to_radians(), z.to_radians()))
            .unwrap_or((0.0, 0.0, 0.0));
        // COLONQ_PITCH_SIGN=-1：低头/抬头方向反转（俯仰轴镜像）
        // COLONQ_QSIGN="x,y,z"：分别翻转原始四元数的三个分量（默认 1,1,1）
        // COLONQ_SWAP_PY=1：交换补偿欧拉角的 pitch 和 yaw（实测：点头↔摇头轴错位时用）
        let pitch_flip = utils::env_flag("COLONQ_PITCH_FLIP", false);
        let (qx_sign, qy_sign, qz_sign) =
            utils::env_csv3("COLONQ_QSIGN").unwrap_or((1.0, 1.0, 1.0));
        let pitch_sign: f32 = utils::env_parse("COLONQ_PITCH_SIGN").unwrap_or(qx_sign);
        let mut buf = [0u8; 65535];
        // 5 秒没收到任何包就提醒一次：绑 127.0.0.1 而后端在另一台机器上，是最容易踩的坑
        let mut warned_no_packet = false;
        loop {
            match socket.recv_from(&mut buf) {
                Ok((len, src)) => {
                    counter += 1;
                    warned_no_packet = false;
                    if !parse_packet(
                        &buf[..len],
                        &sts,
                        debug_level,
                        counter,
                        flip_mode,
                        offset_rad,
                        pitch_sign,
                        qy_sign,
                        qz_sign,
                        pitch_flip,
                        smooth,
                        &expr_opts,
                    ) {
                        log::debug!("忽略无效面捕包（{} bytes，来自 {}）", len, src);
                    }
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    // 面捕端暂停时保持接收，等待恢复；但一直一个包都没有就别让人干等
                    if counter == 0 && !warned_no_packet {
                        warned_no_packet = true;
                        log::warn!(
                            "面捕 {} 上 {} 秒没收到任何 UDP 包。\
面捕跑在局域网另一台机器时**必须**绑全网卡：COLONQ_TRACK_BIND=0.0.0.0:11573（./run-obs.sh 默认如此）；\
只有面捕和渲染同机时才用 127.0.0.1:11573。另外确认面捕端在跑、防火墙放行 UDP 11573。",
                            addr,
                            5
                        );
                    }
                    continue;
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
        put(&mut packet, F_MOUTH_OPEN, mouth_open);
        packet
    }

    fn wait_tracking(sts: &SharedTrackingState) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if sts.lock().unwrap().last_packet.is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "未收到面捕包");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// 按 facetracker.py 的 features 顺序写 14 个特征（偏移见 FEATURES_OFFSET），
    /// 并把四元数设为单位、eye_blink 设为"睁眼"，让包本身是有效的。
    fn make_feature_packet(f: &[f32; 14]) -> Vec<u8> {
        let mut packet = vec![0u8; PACKET_LEN];
        for (i, v) in f.iter().enumerate() {
            byteorder::LittleEndian::write_f32(&mut packet[FEATURES_OFFSET + i * 4..], *v);
        }
        byteorder::LittleEndian::write_f32(&mut packet[QUAT_OFFSET + 12..], 1.0);
        byteorder::LittleEndian::write_f32(&mut packet[BLINK_OFFSET..], 1.0);
        byteorder::LittleEndian::write_f32(&mut packet[BLINK_OFFSET + 4..], 1.0);
        packet
    }

    /// 特征数组 → 表情权重（跳过网络，直接测映射函数）
    fn weights_from(f: [f32; 14]) -> ExprWeights {
        weights_with(f, &ExprOptions::default())
    }

    fn weights_with(f: [f32; 14], opts: &ExprOptions) -> ExprWeights {
        let packet = make_feature_packet(&f);
        let raw = RawFeatures::read(&packet).expect("14 个特征应可解析");
        compute_weights(&raw, opts)
    }

    #[test]
    fn valid_packet_updates_state_with_correct_offsets() {
        let sts = std::sync::Arc::new(std::sync::Mutex::new(TrackingState::new()));
        let addr = start_receiver("127.0.0.1:0", sts.clone()).unwrap();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .send_to(&make_packet(0.25, 0.75, [0.0, 0.0, 0.0, 1.0], 0.42), addr)
            .unwrap();
        wait_tracking(&sts);
        let s = sts.lock().unwrap();
        assert_eq!(s.expression_weights.get("blink"), 0.5);
        // 默认嘴部灵敏度 1.8：0.42 × 1.8 = 0.756（宽=0，所以全给 oh）
        assert!((s.expression_weights.get("oh") - 0.756).abs() < 1e-5);
        assert!(s.tracking_active());
        assert_ne!(s.expression_weights.get("oh"), 10.25);
    }

    #[test]
    fn identity_quaternion_plus_zflip_matches_calibrated_neutral() {
        // 校准结论（tools/calibrate_rotation.py 档3）：包内单位四元数 + 接收器的
        // 轴补偿 + Z轴180° = "正脸"。发送单位四元数时，接收到的姿态应等于
        // 上游补偿 × Z_FLIP —— 也就是 calibrate_rotation 档3 发送值的共轭逆。
        let sts = std::sync::Arc::new(std::sync::Mutex::new(TrackingState::new()));
        let addr = start_receiver("127.0.0.1:0", sts.clone()).unwrap();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .send_to(&make_packet(0.0, 0.0, [0.0, 0.0, 0.0, 1.0], 0.0), addr)
            .unwrap();
        wait_tracking(&sts);
        let orientation = sts.lock().unwrap().orientation;
        // 档3（calibrate_rotation.py）实测让头部回正：接收器对单位四元数的输出即中性姿态
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let expected = glam::Quat::from_xyzw(-h, -h, -3.0908623e-8, 3.0908623e-8);
        assert!(
            (orientation.dot(expected) - 1.0).abs() < 1e-4,
            "orientation={:?} expected={:?}",
            orientation,
            expected
        );
    }

    #[test]
    fn steady_state_alpha_equals_old_formula_at_any_packet_rate() {
        // 关键回归：无论面捕端是 30 还是 60 包/秒，稳态下 alpha 都必须等于旧的
        // 1 - smooth。否则升级会让"响应变慢"，体感就是"动捕延迟变高"。
        for &hz in &[15.0f32, 24.0, 30.0, 60.0, 120.0] {
            let nominal = 1.0 / hz;
            let alpha = smooth_alpha(0.5, nominal, nominal);
            assert!((alpha - 0.5).abs() < 1e-5, "hz={} alpha={}", hz, alpha);
        }
        // 关闭平滑
        assert_eq!(smooth_alpha(0.0, 1.0 / 30.0, 1.0 / 30.0), 1.0);
        // 还没测出标称间隔时退回 30fps 基准
        assert!((smooth_alpha(0.5, 1.0 / 30.0, 0.0) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn smooth_alpha_compensates_jitter_and_dropouts() {
        let nominal = 1.0 / 60.0; // 面捕端 60 包/秒
                                  // 正常一帧：等于旧公式
        assert!((smooth_alpha(0.5, nominal, nominal) - 0.5).abs() < 1e-5);
        // 掉了一个包（间隔翻倍）：系数变大，追得更紧
        assert!((smooth_alpha(0.5, 2.0 * nominal, nominal) - 0.75).abs() < 1e-5);
        // 断流一秒后重连：几乎直接吸附
        assert!(smooth_alpha(0.5, 1.0, nominal) > 0.999);
        // 指数衰减自洽：两个短间隔 == 一个长间隔
        let two_steps = 1.0 - (1.0 - smooth_alpha(0.5, nominal, nominal)).powi(2);
        assert!((two_steps - smooth_alpha(0.5, 2.0 * nominal, nominal)).abs() < 1e-5);
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
        socket
            .send_to(&make_packet(0.0, 0.0, [0.0, 0.0, 0.0, 1.0], 0.9), addr)
            .unwrap();
        wait_tracking(&sts);
        // 默认嘴部灵敏度 1.8：0.9 × 1.8 会被夹到 1.0
        assert_eq!(sts.lock().unwrap().expression_weights.get("oh"), 1.0);
    }

    #[test]
    fn mouth_open_is_clamped() {
        let sts = std::sync::Arc::new(std::sync::Mutex::new(TrackingState::new()));
        let addr = start_receiver("127.0.0.1:0", sts.clone()).unwrap();
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .send_to(&make_packet(0.0, 0.0, [0.0, 0.0, 0.0, 1.0], 5.0), addr)
            .unwrap();
        wait_tracking(&sts);
        assert_eq!(sts.lock().unwrap().expression_weights.get("oh"), 1.0);
    }

    // ---- 14 个特征 → 11 个表情槽的映射 ----
    // 特征下标顺序即 facetracker.py 的 features 列表：
    // 0 eye_l, 1 eye_r, 2 brow_steep_l, 3 brow_updown_l, 4 brow_quirk_l,
    // 5 brow_steep_r, 6 brow_updown_r, 7 brow_quirk_r,
    // 8 corner_l, 9 inout_l, 10 corner_r, 11 inout_r, 12 open, 13 wide

    #[test]
    fn feature_offsets_follow_facetracker_order() {
        // 用 1..14 这种可区分的值，任何一个偏移写错都会立刻对不上
        let vals = [
            1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0,
        ];
        let f = RawFeatures::read(&make_feature_packet(&vals)).unwrap();
        assert_eq!((f.eye_l, f.eye_r), (1.0, 2.0));
        assert_eq!(
            (f.brow_steep_l, f.brow_updown_l, f.brow_quirk_l),
            (3.0, 4.0, 5.0)
        );
        assert_eq!(
            (f.brow_steep_r, f.brow_updown_r, f.brow_quirk_r),
            (6.0, 7.0, 8.0)
        );
        assert_eq!((f.mouth_corner_l, f.mouth_inout_l), (9.0, 10.0));
        assert_eq!((f.mouth_corner_r, f.mouth_inout_r), (11.0, 12.0));
        assert_eq!((f.mouth_open, f.mouth_wide), (13.0, 14.0));
        // 14 个特征正好占满包的尾部：1729 + 14*4 == 1785
        assert_eq!(FEATURES_OFFSET + 14 * 4, PACKET_LEN);
    }

    #[test]
    fn nan_feature_invalidates_packet() {
        let mut vals = [0.0f32; 14];
        vals[13] = f32::NAN;
        assert!(
            RawFeatures::read(&make_feature_packet(&vals)).is_none(),
            "特征里有 NaN 应判为无效包"
        );
    }

    #[test]
    fn neutral_face_produces_no_expression() {
        let w = weights_from([0.0; 14]);
        for (i, v) in w.0.iter().enumerate() {
            assert_eq!(*v, 0.0, "中性脸时 {} 应为 0，实际 {}", EXPR_NAMES[i], v);
        }
    }

    #[test]
    fn raised_brows_are_surprised_and_tilted_in_brows_are_angry() {
        let mut f = [0.0f32; 14];
        f[3] = 0.8; // 左侧眉抬
        f[6] = 0.8; // 右侧眉抬
        let w = weights_from(f);
        assert!(w.get("surprised") > 0.6, "{:?}", w);
        assert_eq!(w.get("angry"), 0.0);

        let mut f = [0.0f32; 14];
        f[2] = -0.8; // 左侧眉内侧下压
        f[5] = -0.8; // 右侧眉内侧下压
        let w = weights_from(f);
        assert!(w.get("angry") > 0.5, "{:?}", w);
        assert_eq!(w.get("surprised"), 0.0);
    }

    #[test]
    fn small_smile_is_relaxed_and_big_smile_is_happy() {
        let mut small = [0.0f32; 14];
        small[8] = 0.30;
        small[10] = 0.30;
        let small = weights_from(small);
        assert!(
            small.get("relaxed") > 0.5,
            "小微笑应偏 relaxed: {:?}",
            small
        );
        assert!(
            small.get("happy") < 0.2,
            "小微笑不该把 happy 打满: {:?}",
            small
        );

        let mut big = [0.0f32; 14];
        big[8] = 1.0;
        big[10] = 1.0;
        let big = weights_from(big);
        assert!(big.get("happy") > 0.9, "大笑容应给 happy: {:?}", big);
        assert!(
            big.get("relaxed") < 0.3,
            "大笑容不该还留 relaxed: {:?}",
            big
        );
    }

    #[test]
    fn mouth_corner_down_is_sad() {
        let mut f = [0.0f32; 14];
        f[8] = -0.7;
        f[10] = -0.7;
        assert!(weights_from(f).get("sad") > 0.6);
    }

    #[test]
    fn vowel_shape_follows_open_and_wide() {
        // 张大 + 嘴宽 → あ
        let mut f = [0.0f32; 14];
        f[12] = 0.9;
        f[13] = 0.8;
        let aa = weights_from(f);
        assert!(aa.get("aa") > 0.5, "{:?}", aa);
        assert!(
            aa.get("aa") > aa.get("oh"),
            "张开且宽应是 aa 而不是 oh: {:?}",
            aa
        );

        // 张大 + 嘴不宽 → お（保留"只要张嘴就有 oh"的旧手感）
        let mut f = [0.0f32; 14];
        f[12] = 0.9;
        let oh = weights_from(f);
        assert!(oh.get("oh") > 0.8, "{:?}", oh);
        assert_eq!(oh.get("aa"), 0.0);

        // 比平时窄（嘟嘴）→ う
        let mut f = [0.0f32; 14];
        f[13] = -0.8;
        assert!(weights_from(f).get("ou") > 0.2);

        // 微张 + 嘴宽 → え / い
        let mut f = [0.0f32; 14];
        f[12] = 0.15;
        f[13] = 0.9;
        let ee = weights_from(f);
        assert!(ee.get("ee") > 0.2, "{:?}", ee);
    }

    #[test]
    fn weights_always_stay_within_zero_and_one() {
        // 线性同余伪随机，覆盖 [-1, 1] 全区间（包括超出特征正常范围的极端值）
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
        };
        for _ in 0..2000 {
            let mut f = [0.0f32; 14];
            for v in f.iter_mut() {
                *v = next() * 1.5; // 故意越界，验证夹取
            }
            let w = weights_from(f);
            for (i, v) in w.0.iter().enumerate() {
                assert!(
                    v.is_finite() && (0.0..=1.0).contains(v),
                    "{} 越界: {}",
                    EXPR_NAMES[i],
                    v
                );
            }
        }
    }

    #[test]
    fn opening_the_mouth_is_not_read_as_a_smile() {
        // 光张嘴（上唇被抬高 → corner_up 变正）不该被判成笑
        let mut f = [0.0f32; 14];
        f[12] = 0.9; // 张嘴
        f[8] = 0.8; // 嘴角信号被上唇带起来
        f[10] = 0.8;
        let w = weights_from(f);
        assert!(w.get("happy") < 0.35, "张嘴不该触发大笑: {:?}", w);
        assert!(w.get("oh") > 0.3, "该是张嘴的口型: {:?}", w);
    }

    #[test]
    fn smiling_suppresses_vowel_mouth_shapes() {
        // 笑 + 同时张嘴：元音口型要让位，否则 happy 和 oh/aa 叠出"血盆大口"
        let mut smile_only = [0.0f32; 14];
        smile_only[8] = 1.0;
        smile_only[10] = 1.0;
        let happy = weights_from(smile_only);
        assert!(happy.get("happy") > 0.9);
        assert_eq!(happy.get("aa"), 0.0);
        assert_eq!(happy.get("oh"), 0.0);

        let mut smile_and_open = [0.0f32; 14];
        smile_and_open[8] = 1.0;
        smile_and_open[10] = 1.0;
        smile_and_open[12] = 0.8;
        let both = weights_with(smile_and_open, &ExprOptions::default());
        let no_override = weights_with(
            smile_and_open,
            &ExprOptions {
                mouth_override: 0.0,
                ..ExprOptions::default()
            },
        );
        assert!(both.get("oh") > 0.0, "说话时还是要能张嘴: {:?}", both);
        assert!(
            both.get("oh") < no_override.get("oh") * 0.7,
            "笑的时候元音口型应被明显压低: {} vs {}",
            both.get("oh"),
            no_override.get("oh")
        );
    }

    #[test]
    fn wide_eyes_alone_can_trigger_surprise() {
        let mut f = [0.0f32; 14];
        f[0] = 0.8; // 眼睛比平时睁得大
        f[1] = 0.8;
        let w = weights_from(f);
        assert!(w.get("surprised") > 0.3, "睁大眼睛应给惊讶: {:?}", w);
        // 关掉这条通路后又回到 0
        let w0 = weights_with(
            f,
            &ExprOptions {
                eye_surprise: 0.0,
                ..ExprOptions::default()
            },
        );
        assert_eq!(w0.get("surprised"), 0.0);
    }

    #[test]
    fn eye_laugh_only_kicks_in_for_big_smiles() {
        let opts = ExprOptions {
            eye_laugh: true,
            ..ExprOptions::default()
        };
        // 小微笑（happy≈0.1）：眼角还不该眯
        let mut f = [0.0f32; 14];
        f[8] = 0.35;
        f[10] = 0.35;
        let small = weights_with(f, &opts);
        assert!(small.get("happy") > 0.0);
        assert_eq!(small.get("relaxed"), 0.0, "小微笑不该眯眼: {:?}", small);
        assert_eq!(small.get("ih"), 0.0);

        // 大笑（happy=1）：眯眼拉满、眼珠变白稍弱一点
        let mut f = [0.0f32; 14];
        f[8] = 1.0;
        f[10] = 1.0;
        let big = weights_with(f, &opts);
        assert!(big.get("happy") > 0.99);
        assert!((big.get("relaxed") - 1.0).abs() < 1e-5, "{:?}", big);
        assert!((big.get("ih") - 0.9).abs() < 1e-5, "{:?}", big);

        // 两个强度都能单独关掉（觉得"眼睛整个没了"就把眼珠那一路调小）
        let no_iris = weights_with(
            f,
            &ExprOptions {
                eye_laugh: true,
                laugh_iris: 0.0,
                ..ExprOptions::default()
            },
        );
        assert!((no_iris.get("relaxed") - 1.0).abs() < 1e-5);
        assert_eq!(no_iris.get("ih"), 0.0);

        // 关掉开关时这两个槽还是原来的语义（小微笑）
        let mut f = [0.0f32; 14];
        f[8] = 1.0;
        f[10] = 1.0;
        let off = weights_with(f, &ExprOptions::default());
        assert_eq!(off.get("relaxed"), 0.0);
        assert_eq!(off.get("ih"), 0.0);
    }

    #[test]
    fn every_slot_name_is_unique_and_looked_up() {
        for (i, name) in EXPR_NAMES.iter().enumerate() {
            assert_eq!(expr_index(name), Some(i));
            assert_eq!(
                EXPR_NAMES.iter().filter(|n| *n == name).count(),
                1,
                "槽位名重复: {}",
                name
            );
        }
        assert_eq!(expr_index("不存在的表情"), None);
    }
}
