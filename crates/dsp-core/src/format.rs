//! Number and text formatting that matches the Dart implementation, so log
//! texts and device bytes are identical.

/// Dart's `double.toStringAsFixed(digits)`: like `format!("{:.digits$}")`,
/// but an exact tie rounds away from zero (423.625 → "423.63"), where Rust
/// rounds to even ("423.62").
pub fn to_fixed(x: f64, digits: usize) -> String {
    if !x.is_finite() {
        return if x.is_nan() { "NaN".into() } else if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let rounded = format!("{x:.digits$}");
    // A tie has a finite expansion of digits + 1 places ending in 5. Doubles
    // of a magnitude where a tie is possible (|x| ≥ 0.5·10^-digits) have at
    // most ~70 fractional decimal digits, so this expansion is exact.
    let exact = format!("{:.*}", digits + 80, x.abs());
    let point = exact.find('.').unwrap_or(exact.len());
    let tail = &exact[point + 1 + digits..];
    let is_tie = tail.starts_with('5') && tail[1..].bytes().all(|b| b == b'0');
    if !is_tie {
        return rounded;
    }
    // Truncate to [digits] places and add one unit in the last place.
    let mut kept: Vec<u8> = exact[..point + 1 + digits].bytes().collect();
    if digits == 0 {
        kept.pop(); // the '.'
    }
    let mut i = kept.len();
    loop {
        if i == 0 {
            kept.insert(0, b'1');
            break;
        }
        i -= 1;
        match kept[i] {
            b'.' => continue,
            b'9' => kept[i] = b'0',
            d => {
                kept[i] = d + 1;
                break;
            }
        }
    }
    let magnitude = String::from_utf8(kept).unwrap_or_default();
    if x < 0.0 { format!("-{magnitude}") } else { magnitude }
}

/// Dart's `double.toString()` for the values shown in texts: integral values
/// keep ".0" ("-60.0"), others use the shortest representation.
pub fn double_to_string(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e21 { format!("{x:.1}") } else { format!("{x}") }
}

/// Dart's `String.toUpperCase()` for the ASCII-oriented uses here: every
/// character maps to one character (simple case mapping), so 'ß' stays 'ß'
/// instead of becoming "SS".
pub fn to_upper_simple(text: &str) -> String {
    text.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) => u,
                _ => c,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ties_round_away_from_zero() {
        assert_eq!(to_fixed(423.625, 2), "423.63");
        assert_eq!(to_fixed(-423.625, 2), "-423.63");
        assert_eq!(to_fixed(0.5, 0), "1");
        assert_eq!(to_fixed(9.95, 1), "9.9"); // 9.95 is below the tie in binary
        assert_eq!(to_fixed(9.875, 2), "9.88");
        assert_eq!(to_fixed(99.5, 0), "100");
        assert_eq!(to_fixed(-6.0, 1), "-6.0");
        assert_eq!(to_fixed(198.425, 0), "198");
    }

    #[test]
    fn simple_upper_case() {
        assert_eq!(to_upper_simple("ab12ßx"), "AB12ßX");
    }
}
