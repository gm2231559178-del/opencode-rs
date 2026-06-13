/// Truncate text with ellipsis in the middle
pub fn truncate_middle(s: &str, max_len: usize) -> String {
    if s.len() <= max_len || max_len < 5 {
        return s.to_string();
    }
    let half = (max_len - 3) / 2;
    let left: String = s.chars().take(half).collect();
    let right: String = s.chars().rev().take(max_len - 3 - half).collect::<String>().chars().rev().collect();
    format!("{}...{}", left, right)
}

/// Truncate text with ellipsis on the left
pub fn truncate_left(s: &str, max_len: usize) -> String {
    if s.len() <= max_len || max_len < 4 {
        return s.to_string();
    }
    let right: String = s.chars().rev().take(max_len - 3).collect::<String>().chars().rev().collect();
    format!("...{}", right)
}

/// Format a number with compact notation (1.2K, 3.5M, etc.)
pub fn format_number(n: usize) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1_000_000_000.0)
    } else if n >= 100_000_000 {
        format!("{}M", n / 1_000_000)
    } else if n >= 10_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 100_000 {
        format!("{}K", n / 1_000)
    } else if n >= 10_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else if n >= 1_000 {
        format!("{:.2}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Format a duration in milliseconds to a human-readable string
pub fn format_duration(ms: u64) -> String {
    if ms < 1_000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else if ms < 3_600_000 {
        let m = ms / 60_000;
        let s = (ms % 60_000) / 1_000;
        format!("{}m {}s", m, s)
    } else {
        let h = ms / 3_600_000;
        let m = (ms % 3_600_000) / 60_000;
        format!("{}h {}m", h, m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_middle() {
        assert_eq!(truncate_middle("hello", 10), "hello");
        assert_eq!(truncate_middle("hello world", 5), "h...d");
        assert_eq!(truncate_middle("hello world", 11), "hello world");
    }

    #[test]
    fn test_truncate_left() {
        assert_eq!(truncate_left("hello", 10), "hello");
        assert_eq!(truncate_left("hello world", 6), "...rld");
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(42), "42");
        assert_eq!(format_number(1234), "1.23K");
        assert_eq!(format_number(12345), "12.3K");
        assert_eq!(format_number(123456), "123K");
        assert_eq!(format_number(1234567), "1.23M");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(500), "500ms");
        assert_eq!(format_duration(1500), "1.5s");
        assert_eq!(format_duration(90000), "1m 30s");
        assert_eq!(format_duration(3660000), "1h 1m");
    }
}
