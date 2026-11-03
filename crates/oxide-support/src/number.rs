//! Number formatting helpers, like Laravel's `Number`.

/// `Number` helpers as associated functions.
#[derive(Debug, Clone, Copy)]
pub struct Number;

const FILE_UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB"];
const ABBREVIATIONS: &[&str] = &["", "K", "M", "B", "T", "Q"];

impl Number {
    /// `1234567.891` → `1,234,567.89` with 2 decimals.
    pub fn format(value: f64, decimals: usize) -> String {
        if !value.is_finite() {
            return value.to_string();
        }
        let fixed = format!("{:.decimals$}", value.abs());
        let (whole, fraction) = fixed.split_once('.').unwrap_or((&fixed, ""));
        let mut grouped = String::new();
        for (position, digit) in whole.chars().enumerate() {
            if position > 0 && (whole.len() - position) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(digit);
        }
        let negative =
            value.is_sign_negative() && fixed.chars().any(|c| c.is_ascii_digit() && c != '0');
        let sign = if negative { "-" } else { "" };
        if fraction.is_empty() {
            format!("{sign}{grouped}")
        } else {
            format!("{sign}{grouped}.{fraction}")
        }
    }

    /// `0.5` → `0.5%`; the value is already a percentage (`12.345` → `12.35%`).
    pub fn percentage(value: f64, precision: usize) -> String {
        format!("{}%", Self::format(value, precision).replace(',', ""))
    }

    /// Bytes in binary units: `1536` → `1.5 KB`. Always shows `precision` decimals.
    #[allow(clippy::cast_precision_loss)] // display only: a rounded size is fine
    pub fn file_size(bytes: u64, precision: usize) -> String {
        let mut value = bytes as f64;
        let mut unit = 0;
        while value >= 1024.0 && unit < FILE_UNITS.len() - 1 {
            value /= 1024.0;
            unit += 1;
        }
        let decimals = if unit == 0 { 0 } else { precision };
        format!("{} {}", Self::format(value, decimals), FILE_UNITS[unit])
    }

    /// A short form: `1200` → `1.2K`, `3_000_000` → `3M`. Trailing zeros are dropped.
    pub fn abbreviate(value: f64, precision: usize) -> String {
        let mut scaled = value;
        let mut unit = 0;
        while scaled.abs() >= 1000.0 && unit < ABBREVIATIONS.len() - 1 {
            scaled /= 1000.0;
            unit += 1;
        }
        let mut text = format!("{scaled:.precision$}");
        if text.contains('.') {
            text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
        }
        format!("{text}{}", ABBREVIATIONS[unit])
    }

    /// An amount with its currency: `$1,234.50`, `€10.00`, `¥1,200`. Unknown codes print as
    /// `1,234.50 CHF`.
    pub fn currency(amount: f64, code: &str) -> String {
        let code = code.to_ascii_uppercase();
        let (symbol, decimals) = match code.as_str() {
            "USD" => ("$", 2),
            "EUR" => ("€", 2),
            "GBP" => ("£", 2),
            "BRL" => ("R$", 2),
            "JPY" => ("¥", 0),
            _ => ("", 2),
        };
        let number = Self::format(amount.abs(), decimals);
        let sign = if amount < 0.0 && number.chars().any(|c| c.is_ascii_digit() && c != '0') {
            "-"
        } else {
            ""
        };
        if symbol.is_empty() {
            format!("{sign}{number} {code}")
        } else {
            format!("{sign}{symbol}{number}")
        }
    }

    /// `1` → `1st`, `22` → `22nd`, `113` → `113th`.
    pub fn ordinal(value: u64) -> String {
        let suffix = match (value % 100, value % 10) {
            (11..=13, _) => "th",
            (_, 1) => "st",
            (_, 2) => "nd",
            (_, 3) => "rd",
            _ => "th",
        };
        format!("{value}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_with_grouping() {
        assert_eq!(Number::format(1_234_567.891, 2), "1,234,567.89");
        assert_eq!(Number::format(999.0, 0), "999");
        assert_eq!(Number::format(1000.0, 0), "1,000");
        assert_eq!(Number::format(-1234.5, 1), "-1,234.5");
        assert_eq!(Number::format(-0.001, 2), "0.00");
        assert_eq!(Number::format(f64::NAN, 2), "NaN");
        assert_eq!(Number::percentage(12.345, 1), "12.3%");
    }

    #[test]
    fn sizes_and_abbreviations() {
        assert_eq!(Number::file_size(512, 2), "512 B");
        assert_eq!(Number::file_size(1_536, 1), "1.5 KB");
        assert_eq!(Number::file_size(1_024, 0), "1 KB");
        assert_eq!(Number::file_size(5 * 1024 * 1024 * 1024, 2), "5.00 GB");
        assert_eq!(
            Number::file_size(u64::MAX, 0).split(' ').next_back(),
            Some("PB")
        );
        assert_eq!(Number::abbreviate(1_200.0, 1), "1.2K");
        assert_eq!(Number::abbreviate(1_000.0, 1), "1K");
        assert_eq!(Number::abbreviate(3_000_000.0, 0), "3M");
        assert_eq!(Number::abbreviate(950.0, 0), "950");
        assert_eq!(Number::abbreviate(-2_500.0, 1), "-2.5K");
    }

    #[test]
    fn currency_and_ordinals() {
        assert_eq!(Number::currency(1234.5, "usd"), "$1,234.50");
        assert_eq!(Number::currency(10.0, "EUR"), "€10.00");
        assert_eq!(Number::currency(1200.0, "JPY"), "¥1,200");
        assert_eq!(Number::currency(-5.0, "GBP"), "-£5.00");
        assert_eq!(Number::currency(1234.5, "CHF"), "1,234.50 CHF");
        for (n, text) in [
            (1, "1st"),
            (2, "2nd"),
            (3, "3rd"),
            (4, "4th"),
            (11, "11th"),
            (12, "12th"),
            (13, "13th"),
            (21, "21st"),
            (102, "102nd"),
            (113, "113th"),
        ] {
            assert_eq!(Number::ordinal(n), text);
        }
    }
}
