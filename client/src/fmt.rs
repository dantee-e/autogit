//! Display helpers (dates, sizes, version labels).

use chrono::{DateTime, Local, TimeZone, Utc};

pub fn date(iso: &str) -> String {
    DateTime::parse_from_rfc3339(iso)
        .map(|d| d.with_timezone(&Local).format("%d/%m/%Y %H:%M").to_string())
        .or_else(|_| {
            // Server may omit the offset; treat as UTC.
            chrono::NaiveDateTime::parse_from_str(iso.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S")
                .map(|n| Utc.from_utc_datetime(&n).with_timezone(&Local).format("%d/%m/%Y %H:%M").to_string())
        })
        .unwrap_or_else(|_| iso.to_owned())
}

pub fn file_timestamp(unix_secs: u64) -> String {
    Local
        .timestamp_opt(unix_secs as i64, 0)
        .single()
        .map(|d| d.format("%Y%m%d-%H%M%S").to_string())
        .unwrap_or_else(|| unix_secs.to_string())
}

pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", b / 1024.0),
        1_048_576..1_073_741_824 => format!("{:.1} MB", b / 1_048_576.0),
        _ => format!("{:.2} GB", b / 1_073_741_824.0),
    }
}

pub fn version_label(kind: &str, label: &str) -> String {
    if kind == "cad" { format!("Rev {label}") } else { format!("v{label}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size(10), "10 B");
        assert_eq!(size(2048), "2.0 KB");
        assert_eq!(size(5 * 1_048_576), "5.0 MB");
    }

    #[test]
    fn labels() {
        assert_eq!(version_label("cad", "B"), "Rev B");
        assert_eq!(version_label("software", "1.2.0"), "v1.2.0");
    }
}
