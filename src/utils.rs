use crate::gl;

pub const ATTRIB_VERTEX: gl::types::GLuint = 0;
pub const ATTRIB_NORMAL: gl::types::GLuint = 1;
pub const ATTRIB_TEXCOORD: gl::types::GLuint = 2;

pub const ATTRIB_JOINT: gl::types::GLuint = 3;
pub const ATTRIB_WEIGHT: gl::types::GLuint = 4;

#[allow(dead_code)]
#[derive(Clone, PartialEq, Debug)]
pub struct Dimensions {
    pub w: f32,
    pub h: f32,
}

/// 开关类环境变量的统一解析。
///
/// 之前各处手写 `env::var(..).is_ok()` / `== "0"` / `== "1"`，语义互不一致：
/// `is_ok()` 会把空字符串和 `COLONQ_PARTS=0` 都当成"打开"，导致"明明关了却生效"。
///
/// 统一规则（`flag_value`）：
/// - 未设置、空字符串、纯空白 → 用调用方给的 `default`
/// - `0 / false / off / no`（大小写不敏感）→ false
/// - 其他任何非空值（含 `1 / true / on / yes`）→ true
///
/// 即：**只有显式写了关闭值才算关，只有非空才算开**。
pub fn env_flag(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(raw) => flag_value(&raw).unwrap_or(default),
        Err(_) => default,
    }
}

/// 解析单个开关取值；`None` 表示"空值，交回默认值"。
pub fn flag_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => None,
        "0" | "false" | "off" | "no" => Some(false),
        _ => Some(true),
    }
}

/// 读取并解析环境变量。
///
/// 变量不存在 → `None`（静默，属正常情况）；
/// 变量存在但为空值 / 无法解析 → 打一条 warn 再返回 `None`。
/// 这样可以避免"我明明改了变量却没反应"却毫无提示。
pub fn env_parse<T: std::str::FromStr>(name: &str) -> Option<T> {
    let raw = std::env::var(name).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        log::warn!("环境变量 {} 为空值，已忽略（使用默认值）", name);
        return None;
    }
    match trimmed.parse::<T>() {
        Ok(v) => Some(v),
        Err(_) => {
            log::warn!("环境变量 {} 的值 {:?} 无法解析为 {}，已忽略（使用默认值）",
                       name, raw, std::any::type_name::<T>());
            None
        }
    }
}

/// 解析 `"a,b,c"` 形式的三个浮点数（相机位置、旋转偏移、四元数符号等）。
/// 数量不对或含非法数字时打 warn 并返回 `None`。
pub fn env_csv3(name: &str) -> Option<(f32, f32, f32)> {
    let raw = std::env::var(name).ok()?;
    match parse_csv3(&raw) {
        Some(v) => Some(v),
        None => {
            log::warn!("环境变量 {} 的值 {:?} 不是 3 个逗号分隔的数字，已忽略（使用默认值）",
                       name, raw);
            None
        }
    }
}

fn parse_csv3(raw: &str) -> Option<(f32, f32, f32)> {
    let parts: Vec<&str> = raw.split(',').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut out = [0f32; 3];
    for (i, part) in parts.iter().enumerate() {
        out[i] = part.trim().parse::<f32>().ok()?;
    }
    Some((out[0], out[1], out[2]))
}

/// 把当前生效的 `COLONQ_*` 环境变量全部打进日志，方便确认"改的到底有没有生效"。
pub fn log_env_snapshot() {
    let mut vars: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| k.starts_with("COLONQ_"))
        .collect();
    vars.sort();
    if vars.is_empty() {
        log::info!("配置: 未设置任何 COLONQ_* 变量（全部走默认值）");
        return;
    }
    log::info!("配置: 共 {} 个 COLONQ_* 变量生效", vars.len());
    for (k, v) in vars {
        log::info!("    {}={}", k, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_value_accepts_common_spellings() {
        assert_eq!(flag_value(""), None);
        assert_eq!(flag_value("   "), None);
        assert_eq!(flag_value("0"), Some(false));
        assert_eq!(flag_value("false"), Some(false));
        assert_eq!(flag_value(" OFF "), Some(false));
        assert_eq!(flag_value("no"), Some(false));
        assert_eq!(flag_value("1"), Some(true));
        assert_eq!(flag_value("true"), Some(true));
        assert_eq!(flag_value("YES"), Some(true));
        assert_eq!(flag_value("lcolonq"), Some(true));
    }

    #[test]
    fn env_flag_treats_empty_and_zero_as_off() {
        // 每个用例用独立变量名，避免并行测试互相踩
        std::env::set_var("COLONQ_TEST_FLAG_EMPTY", "");
        std::env::set_var("COLONQ_TEST_FLAG_ZERO", "0");
        std::env::set_var("COLONQ_TEST_FLAG_ONE", "1");
        std::env::set_var("COLONQ_TEST_FLAG_OFF", "off");
        std::env::set_var("COLONQ_TEST_FLAG_ON", "on");

        // 空字符串 → 回落到默认值（这是 COLONQ_PARTS 那次事故的根因）
        assert!(env_flag("COLONQ_TEST_FLAG_EMPTY", true));
        assert!(!env_flag("COLONQ_TEST_FLAG_EMPTY", false));
        // 显式 0 / off 一定为关，哪怕默认是开
        assert!(!env_flag("COLONQ_TEST_FLAG_ZERO", true));
        assert!(!env_flag("COLONQ_TEST_FLAG_OFF", true));
        // 显式 1 / on 一定为开，哪怕默认是关
        assert!(env_flag("COLONQ_TEST_FLAG_ONE", false));
        assert!(env_flag("COLONQ_TEST_FLAG_ON", false));
        // 从未设置 → 默认值
        assert!(env_flag("COLONQ_TEST_FLAG_NEVER_SET_A", true));
        assert!(!env_flag("COLONQ_TEST_FLAG_NEVER_SET_A", false));
    }

    #[test]
    fn env_parse_returns_none_for_bad_or_empty_values() {
        std::env::set_var("COLONQ_TEST_PARSE_OK", " 2.5 ");
        std::env::set_var("COLONQ_TEST_PARSE_BAD", "abc");
        std::env::set_var("COLONQ_TEST_PARSE_EMPTY", "   ");
        assert_eq!(env_parse::<f32>("COLONQ_TEST_PARSE_OK"), Some(2.5));
        assert_eq!(env_parse::<f32>("COLONQ_TEST_PARSE_BAD"), None);
        assert_eq!(env_parse::<f32>("COLONQ_TEST_PARSE_EMPTY"), None);
        assert_eq!(env_parse::<f32>("COLONQ_TEST_PARSE_MISSING"), None);
    }

    #[test]
    fn env_csv3_requires_exactly_three_numbers() {
        std::env::set_var("COLONQ_TEST_CSV_OK", "0,-20,360");
        std::env::set_var("COLONQ_TEST_CSV_SHORT", "1,2");
        std::env::set_var("COLONQ_TEST_CSV_BAD", "1,x,3");
        assert_eq!(env_csv3("COLONQ_TEST_CSV_OK"), Some((0.0, -20.0, 360.0)));
        assert_eq!(env_csv3("COLONQ_TEST_CSV_SHORT"), None);
        assert_eq!(env_csv3("COLONQ_TEST_CSV_BAD"), None);
    }
}
