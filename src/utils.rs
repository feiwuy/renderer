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
}
