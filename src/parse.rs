//! 人类可读输入解析,对齐其它语言 SDK(Go parse.go)。
//!
//! - [`parse_size`]:"4GiB" / "512MiB" / "1024" → 字节数
//! - [`parse_duration`]:"30m" / "6h" / "90s" → 秒数

use crate::errors::Error;

/// 解析大小字符串成字节数。支持后缀(大小写不敏感):
/// B / KB / MB / GB / TB(1000 进制)、KiB / MiB / GiB / TiB(1024 进制)。
/// 纯数字按字节处理。
pub fn parse_size(s: &str) -> Result<i64, Error> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    let lower = s.to_ascii_lowercase();
    // 后缀按长度从长到短匹配,避免 "kib" 被 "b" 抢先。
    let units: &[(&str, i64)] = &[
        ("tib", 1i64 << 40),
        ("gib", 1i64 << 30),
        ("mib", 1i64 << 20),
        ("kib", 1i64 << 10),
        ("tb", 1_000_000_000_000),
        ("gb", 1_000_000_000),
        ("mb", 1_000_000),
        ("kb", 1_000),
        ("b", 1),
    ];
    for (suffix, mult) in units {
        if let Some(num) = lower.strip_suffix(suffix) {
            let num = num.trim();
            let val: f64 = num
                .parse()
                .map_err(|_| Error::Parse(format!("invalid size: {s:?}")))?;
            return Ok((val * *mult as f64) as i64);
        }
    }
    // 无后缀:纯字节数。
    lower
        .parse::<i64>()
        .map_err(|_| Error::Parse(format!("invalid size: {s:?}")))
}

/// 解析时长字符串成秒数。支持后缀:s / m / h / d。纯数字按秒处理。
pub fn parse_duration(s: &str) -> Result<i64, Error> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    let lower = s.to_ascii_lowercase();
    let units: &[(&str, i64)] = &[("d", 86_400), ("h", 3_600), ("m", 60), ("s", 1)];
    for (suffix, mult) in units {
        if let Some(num) = lower.strip_suffix(suffix) {
            let num = num.trim();
            let val: f64 = num
                .parse()
                .map_err(|_| Error::Parse(format!("invalid duration: {s:?}")))?;
            return Ok((val * *mult as f64) as i64);
        }
    }
    lower
        .parse::<i64>()
        .map_err(|_| Error::Parse(format!("invalid duration: {s:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("4GiB").unwrap(), 4i64 << 30);
        assert_eq!(parse_size("512MiB").unwrap(), 512i64 << 20);
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert_eq!(parse_size("1GB").unwrap(), 1_000_000_000);
        assert_eq!(parse_size("").unwrap(), 0);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("30m").unwrap(), 1800);
        assert_eq!(parse_duration("6h").unwrap(), 21600);
        assert_eq!(parse_duration("90s").unwrap(), 90);
        assert_eq!(parse_duration("1d").unwrap(), 86400);
    }

    #[test]
    fn invalid() {
        assert!(parse_size("abc").is_err());
        assert!(parse_duration("xyz").is_err());
    }
}
