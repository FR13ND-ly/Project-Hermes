//! Parsing for Kubernetes resource quantities (memory & CPU) into canonical
//! units. Previously each call site hand-rolled its own fragile parser
//! (`builder.rs::parse_memory_quantity`, the live-metrics SSE in
//! `app_controller.rs`, the scaling in `prometheus.rs`). This centralizes the
//! logic so suffixes are handled consistently.

/// Split a quantity string into its leading numeric part (parsed as f64) and
/// the trailing unit (e.g. "128Mi" -> (128.0, "Mi"), "250m" -> (250.0, "m")).
fn leading_number(s: &str) -> (f64, &str) {
    let s = s.trim();
    let idx = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E'))
        .unwrap_or(s.len());
    let (num_part, unit) = s.split_at(idx);
    (num_part.parse::<f64>().unwrap_or(0.0), unit.trim())
}

/// Parse a Kubernetes memory quantity into bytes. Handles binary suffixes
/// (Ki/Mi/Gi/Ti/Pi), decimal SI suffixes (k/K/M/G/T) and plain bytes.
/// Returns 0 on parse failure.
pub fn parse_memory_bytes(qty: &str) -> u64 {
    let (num, unit) = leading_number(qty);
    let mult = match unit {
        "Ki" => 1024.0,
        "Mi" => 1024.0 * 1024.0,
        "Gi" => 1024.0 * 1024.0 * 1024.0,
        "Ti" => 1024.0_f64.powi(4),
        "Pi" => 1024.0_f64.powi(5),
        "k" | "K" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        _ => 1.0, // plain bytes (or unknown unit -> treat as bytes)
    };
    (num * mult).max(0.0) as u64
}

/// Parse a Kubernetes memory quantity into whole MiB (rounded down), for
/// resource-limit headroom math.
pub fn parse_memory_mib(qty: &str) -> i64 {
    (parse_memory_bytes(qty) / (1024 * 1024)) as i64
}

/// Parse a Kubernetes CPU quantity into nanocores. "100m" -> 100_000_000,
/// "250000000n" -> 250_000_000, "2" -> 2_000_000_000. Returns 0 on failure.
pub fn parse_cpu_nanocores(qty: &str) -> u64 {
    let (num, unit) = leading_number(qty);
    let mult = match unit {
        "n" => 1.0,
        "u" => 1_000.0,
        "m" => 1_000_000.0,
        _ => 1_000_000_000.0, // whole cores (no suffix)
    };
    (num * mult).max(0.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_binary_suffixes() {
        assert_eq!(parse_memory_bytes("1Ki"), 1024);
        assert_eq!(parse_memory_bytes("1Mi"), 1024 * 1024);
        assert_eq!(parse_memory_bytes("2Gi"), 2 * 1024 * 1024 * 1024);
        assert_eq!(parse_memory_bytes("1048576Ki"), 1024 * 1024 * 1024);
    }

    #[test]
    fn memory_plain_and_decimal() {
        assert_eq!(parse_memory_bytes("1000000"), 1_000_000);
        assert_eq!(parse_memory_bytes("1M"), 1_000_000);
        assert_eq!(parse_memory_bytes(""), 0);
        assert_eq!(parse_memory_bytes("garbage"), 0);
    }

    #[test]
    fn memory_mib() {
        assert_eq!(parse_memory_mib("512Mi"), 512);
        assert_eq!(parse_memory_mib("1Gi"), 1024);
        assert_eq!(parse_memory_mib("2048Ki"), 2);
    }

    #[test]
    fn cpu_suffixes() {
        assert_eq!(parse_cpu_nanocores("100m"), 100_000_000);
        assert_eq!(parse_cpu_nanocores("250000000n"), 250_000_000);
        assert_eq!(parse_cpu_nanocores("2"), 2_000_000_000);
        assert_eq!(parse_cpu_nanocores("500u"), 500_000);
        assert_eq!(parse_cpu_nanocores(""), 0);
    }
}
