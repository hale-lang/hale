//! A `Decimal` literal's value: an i128 mantissa at the fixed scale 9
//! (`mantissa × 10⁻⁹`). The checker refuses a literal this cannot
//! represent, with the span, and codegen lowers the same parse. The
//! runtime's `parse_decimal` (`parse_decimal_to_i128` in
//! lotus_arena.c) implements the same rule and must stay in step:
//! an optional sign, digits with `_` separators, one optional `.`, an
//! optional trailing `d`.
//!
//! A digit past the ninth fractional place is a value no `Decimal`
//! holds unless it is zero. It used to be dropped in silence
//! (`0.1234567891d` was `0.123456789`, downstream handoff); now a
//! nonzero one is refused. Trailing zeros past the ninth place change
//! nothing and are accepted.

/// The fixed scale every `Decimal` carries.
pub const SCALE: u32 = 9;

/// Why a spelling names no `Decimal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecimalLiteralError {
    /// Not a decimal number.
    Malformed,
    /// A nonzero digit past the ninth fractional place.
    TooPrecise,
    /// The mantissa does not fit an i128.
    Overflow,
}

/// The scale-9 mantissa of `s`, or why there is none.
pub fn parse_scale9(s: &str) -> Result<i128, DecimalLiteralError> {
    use DecimalLiteralError::*;
    let s = s.strip_suffix('d').unwrap_or(s);
    let (sign, rest) = match s.as_bytes().first() {
        Some(b'-') => (-1i128, &s[1..]),
        Some(b'+') => (1i128, &s[1..]),
        _ => (1i128, s),
    };
    if rest.is_empty() {
        return Err(Malformed);
    }
    let mut mantissa: i128 = 0;
    let mut frac_digits: u32 = 0;
    let mut seen_dot = false;
    let mut seen_digit = false;
    let mut too_precise = false;
    for c in rest.chars() {
        match c {
            '0'..='9' => {
                seen_digit = true;
                if seen_dot && frac_digits >= SCALE {
                    too_precise |= c != '0';
                    continue;
                }
                mantissa = mantissa
                    .checked_mul(10)
                    .and_then(|m| m.checked_add((c as u8 - b'0') as i128))
                    .ok_or(Overflow)?;
                if seen_dot {
                    frac_digits += 1;
                }
            }
            '.' if !seen_dot => seen_dot = true,
            '_' => {}
            _ => return Err(Malformed),
        }
    }
    if !seen_digit {
        return Err(Malformed);
    }
    if too_precise {
        return Err(TooPrecise);
    }
    while frac_digits < SCALE {
        mantissa = mantissa.checked_mul(10).ok_or(Overflow)?;
        frac_digits += 1;
    }
    Ok(sign * mantissa)
}

#[cfg(test)]
mod tests {
    use super::*;
    use DecimalLiteralError::*;

    #[test]
    fn scale_nine_is_exact_and_past_it_is_refused() {
        assert_eq!(parse_scale9("1.50d"), Ok(1_500_000_000));
        assert_eq!(parse_scale9("-7.25"), Ok(-7_250_000_000));
        assert_eq!(parse_scale9("0.123456789d"), Ok(123_456_789));
        assert_eq!(parse_scale9("1_000.5"), Ok(1_000_500_000_000));
        assert_eq!(parse_scale9("0.1234567890000d"), Ok(123_456_789), "trailing zeros are the same value");
        assert_eq!(parse_scale9("0.1234567891d"), Err(TooPrecise));
        assert_eq!(parse_scale9("0.123456789012"), Err(TooPrecise));
        assert_eq!(parse_scale9("0.0000000001"), Err(TooPrecise));
        assert_eq!(parse_scale9("abc"), Err(Malformed));
        assert_eq!(parse_scale9("."), Err(Malformed));
        assert_eq!(parse_scale9(""), Err(Malformed));
        assert_eq!(parse_scale9(&"9".repeat(40)), Err(Overflow));
    }
}
