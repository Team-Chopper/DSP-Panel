//! Parsing typed values and formatting them like the Flutter client.

use dsp_core::format::to_fixed;

/// Number from user input: accepts a comma as decimal separator and ignores a
/// unit after the number ("−3,5 dB", "12ms").
pub fn parse_number(input: &str) -> Option<f64> {
    let s = input.trim_start();
    let mut chars = s.char_indices().peekable();
    let mut text = String::new();
    if let Some(&(_, c)) = chars.peek()
        && (c == '+' || c == '-' || c == '−') {
            text.push(if c == '+' { '+' } else { '-' });
            chars.next();
        }
    let mut digits_before = 0;
    let mut separator = false;
    let mut digits_after = 0;
    for (_, c) in chars {
        if c.is_ascii_digit() {
            text.push(c);
            if separator { digits_after += 1 } else { digits_before += 1 }
        } else if (c == '.' || c == ',') && !separator {
            separator = true;
            text.push('.');
        } else {
            break;
        }
    }
    // Like the regex [+-]?\d*[.,]?\d+ : at least one digit after a separator.
    if separator && digits_after == 0 {
        text.pop();
        if digits_before == 0 {
            return None;
        }
    }
    if digits_before == 0 && digits_after == 0 {
        return None;
    }
    text.parse().ok()
}

/// Frequency in Hz from "1200", "1.2k", "1,2 kHz".
pub fn parse_frequency(input: &str) -> Option<f64> {
    let value = parse_number(input)?;
    let lower = input.to_lowercase();
    let kilo = lower
        .char_indices()
        .any(|(i, c)| c == 'k' && lower[..i].trim_end().chars().last().is_some_and(|p| p.is_ascii_digit()));
    Some(if kilo { value * 1000.0 } else { value })
}

pub fn format_hz(hz: f64) -> String {
    if hz >= 1000.0 {
        let k = hz / 1000.0;
        return format!("{}k", to_fixed(k, if k >= 10.0 { 1 } else { 2 }));
    }
    to_fixed(hz, if hz >= 100.0 { 0 } else { 1 })
}

pub fn format_db_value(db: f64) -> String {
    format!("{}{}", if db > 0.0 { "+" } else { "" }, to_fixed(db, 1))
}

/// "+12" / "-6" labels of a dB scale.
pub fn db_mark_label(db: f64) -> String {
    if db > 0.0 { format!("+{}", db.round() as i64) } else { format!("{}", db.round() as i64) }
}

pub fn fixed(x: f64, digits: usize) -> String {
    to_fixed(x, digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(parse_number("-3,5 dB"), Some(-3.5));
        assert_eq!(parse_number("−3.5"), Some(-3.5));
        assert_eq!(parse_number("12ms"), Some(12.0));
        assert_eq!(parse_number(".5"), Some(0.5));
        assert_eq!(parse_number("abc"), None);
        assert_eq!(parse_frequency("1,2 kHz"), Some(1200.0));
        assert_eq!(parse_frequency("1.2k"), Some(1200.0));
        assert_eq!(parse_frequency("80"), Some(80.0));
        assert_eq!(format_hz(1250.0), "1.25k");
        assert_eq!(format_hz(12500.0), "12.5k");
        assert_eq!(format_hz(80.0), "80.0");
        assert_eq!(format_db_value(3.0), "+3.0");
    }
}
