//! ISO-8601 UTC instants for `Time` (GH #607): the literal's
//! compile-time parse and the formatter diagnostics and tests share.
//! A `Time` is i64 nanoseconds since the Unix epoch. The runtime's
//! parser and formatter (`lotus_time_parse_iso8601_raw`,
//! `lotus_str_from_time` in lotus_arena.c) implement the same shape
//! and must stay in step: `YYYY-MM-DDTHH:MM:SS`, an optional fraction
//! of one to nine digits, an optional `Z`. A timezone offset is
//! rejected rather than ignored: a local time is never read as UTC in
//! silence. Calendar arithmetic is Howard Hinnant's, proleptic
//! Gregorian, so a date before 1970 is a negative instant and not an
//! error.

pub const NS_PER_SEC: i64 = 1_000_000_000;

/// Days since 1970-01-01 of a civil date.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The civil date of a day count since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Nanoseconds since the epoch, or None when `s` is not an ISO-8601
/// UTC instant of the shape above.
pub fn parse_iso8601_utc_ns(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let shape = b"____-__-__T__:__:__";
    for i in 0..19 {
        if shape[i] == b'_' {
            if !b[i].is_ascii_digit() {
                return None;
            }
        } else if b[i] != shape[i] {
            return None;
        }
    }
    let num = |a: usize, n: usize| -> i64 {
        b[a..a + n].iter().fold(0i64, |acc, c| acc * 10 + (c - b'0') as i64)
    };
    let (y, mo, d, h, mi, se) =
        (num(0, 4), num(5, 2), num(8, 2), num(11, 2), num(14, 2), num(17, 2));
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut i = 19;
    let mut frac: i64 = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let mut digits = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            if digits < 9 {
                frac = frac * 10 + (b[i] - b'0') as i64;
            }
            digits += 1;
            i += 1;
        }
        if digits == 0 || digits > 9 {
            return None;
        }
        for _ in digits..9 {
            frac *= 10;
        }
    }
    if i < b.len() {
        if b[i] == b'Z' {
            i += 1;
        } else {
            return None;
        }
    }
    if i != b.len() {
        return None;
    }
    let secs = days_from_civil(y, mo as u32, d as u32) * 86400 + h * 3600 + mi * 60 + se;
    Some(secs * NS_PER_SEC + frac)
}

/// `YYYY-MM-DDTHH:MM:SS[.fraction]Z`; the fraction only when it is not
/// zero, trailing zeros dropped.
pub fn format_iso8601_utc_ns(ns: i64) -> String {
    let secs = ns.div_euclid(NS_PER_SEC);
    let frac = ns.rem_euclid(NS_PER_SEC);
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let mut out = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        y,
        m,
        d,
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    );
    if frac != 0 {
        let mut f = format!("{:09}", frac);
        while f.ends_with('0') {
            f.pop();
        }
        out.push('.');
        out.push_str(&f);
    }
    out.push('Z');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trips() {
        for s in ["1970-01-01T00:00:00Z", "2026-05-08T12:00:00Z", "1969-12-31T23:59:59Z", "2026-09-14T08:30:15.25Z", "2000-02-29T00:00:00.000000001Z"] {
            let ns = parse_iso8601_utc_ns(s).unwrap();
            assert_eq!(format_iso8601_utc_ns(ns), s, "{s}");
        }
        assert_eq!(parse_iso8601_utc_ns("1970-01-01T00:00:00"), Some(0));
        assert_eq!(parse_iso8601_utc_ns("1970-01-01T00:00:01Z"), Some(NS_PER_SEC));
        assert_eq!(parse_iso8601_utc_ns("1969-12-31T23:59:59Z"), Some(-NS_PER_SEC));
        for bad in ["2026-05-08 12:00:00Z", "2026-13-01T00:00:00Z", "2026-05-08T12:00:00+01:00", "2026-05-08T12:00:00.Z", "2026-05-08T12:00:00.1234567891Z", "now"] {
            assert!(parse_iso8601_utc_ns(bad).is_none(), "{bad}");
        }
    }
}
