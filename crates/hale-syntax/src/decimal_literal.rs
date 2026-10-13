//! A `Decimal<S>` literal's value: an i128 mantissa at the scale `S`
//! (`mantissa × 10⁻ˢ`), `S` from 0 to [`MAX_SCALE`]; plain `Decimal` is
//! `Decimal<9>` ([`DEFAULT_SCALE`]). The checker refuses a literal its
//! type's scale cannot represent, with the span, and codegen lowers the
//! same parse. The runtime's `parse_decimal` (`parse_decimal_to_i128_s`
//! in lotus_arena.c) implements the same rule and must stay in step: an
//! optional sign, digits with `_` separators, one optional `.`, an
//! optional trailing `d`.
//!
//! A digit past the `S`th fractional place is a value no `Decimal<S>`
//! holds unless it is zero. It used to be dropped in silence
//! (`0.1234567891d` was `0.123456789`, downstream handoff); now a
//! nonzero one is refused. Trailing zeros past the `S`th place change
//! nothing and are accepted.

/// The scale plain `Decimal` carries: `Decimal` is `Decimal<9>`.
pub const DEFAULT_SCALE: u8 = 9;

/// The largest scale a `Decimal<S>` may have. The mantissa is an i128
/// at every scale, so at 18 places it still holds twenty integer digits
/// (about ±1.7 × 10²⁰), and `10^S` is an `Int` (`10¹⁸ < 2⁶³`), which
/// the conversions between scales multiply and divide by.
pub const MAX_SCALE: u8 = 18;

/// The scale every plain `Decimal` carries, as before `Decimal<S>`.
pub const SCALE: u32 = DEFAULT_SCALE as u32;

/// Why a spelling names no `Decimal<S>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecimalLiteralError {
    /// Not a decimal number.
    Malformed,
    /// A nonzero digit past the `S`th fractional place.
    TooPrecise,
    /// The mantissa at the scale does not fit an i128.
    Overflow,
}

/// The scale-9 mantissa of `s`, or why there is none.
pub fn parse_scale9(s: &str) -> Result<i128, DecimalLiteralError> {
    parse_at_scale(s, DEFAULT_SCALE)
}

/// The mantissa of `s` at `scale` (`mantissa × 10⁻ˢᶜᵃˡᵉ`), or why there
/// is none.
pub fn parse_at_scale(s: &str, scale: u8) -> Result<i128, DecimalLiteralError> {
    use DecimalLiteralError::*;
    let scale = u32::from(scale);
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
                if seen_dot && frac_digits >= scale {
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
    while frac_digits < scale {
        mantissa = mantissa.checked_mul(10).ok_or(Overflow)?;
        frac_digits += 1;
    }
    Ok(sign * mantissa)
}

/// `10^n` as an i128, `n` at most 38.
pub fn pow10(n: u32) -> i128 {
    10i128.pow(n)
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

    #[test]
    fn every_scale_is_its_own_count_of_places() {
        assert_eq!(parse_at_scale("1.5d", 2), Ok(150));
        assert_eq!(parse_at_scale("1.505d", 2), Err(TooPrecise));
        assert_eq!(parse_at_scale("1.500d", 2), Ok(150));
        assert_eq!(parse_at_scale("7d", 0), Ok(7));
        assert_eq!(parse_at_scale("7.1d", 0), Err(TooPrecise));
        assert_eq!(parse_at_scale("0.000000000000000001d", 18), Ok(1));
        assert_eq!(parse_at_scale("-1.5d", 18), Ok(-1_500_000_000_000_000_000));
        // Twenty integer digits at scale 18 fit; twenty-one do not.
        assert!(parse_at_scale(&format!("{}d", "9".repeat(20)), 18).is_ok());
        assert_eq!(parse_at_scale(&format!("{}d", "9".repeat(21)), 18), Err(Overflow));
    }
}
