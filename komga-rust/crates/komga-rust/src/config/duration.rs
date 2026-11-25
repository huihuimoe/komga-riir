//! Spring Boot's `DurationStyle`: `java.time.Duration` properties written as `500ms`, `2s`, or
//! ISO-8601 `PT2S`. Unlike Spring, negative durations are rejected.

use std::time::Duration;

use super::{ConfigError, Setting, invalid};

const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// Nanoseconds in the unit a bare number means (`@DurationUnit`).
pub(super) const MILLISECONDS: i128 = 1_000_000;
pub(super) const SECONDS: i128 = NANOS_PER_SECOND;

/// A number with an optional `ns`/`us`/`ms`/`s`/`m`/`h`/`d` suffix, `default_unit` without one,
/// or a `java.time.Duration.parse` text; trimmed, units case-insensitive.
pub(super) fn parse_duration(
    key: &'static str,
    setting: Setting<String>,
    default_unit: i128,
) -> Result<Setting<Duration>, ConfigError> {
    let text = setting.value.trim();
    let nanos = if text.trim_start_matches(['+', '-']).starts_with(['P', 'p']) {
        iso_nanos(text)
    } else {
        simple_nanos(text, default_unit)
    };
    let Some(nanos) = nanos else {
        return Err(invalid(
            key,
            setting,
            "expected a duration such as 2s or PT2S",
        ));
    };
    let Ok(nanos) = u64::try_from(nanos) else {
        return Err(invalid(key, setting, "must not be negative"));
    };
    Ok(Setting {
        value: Duration::from_nanos(nanos),
        source: setting.source,
    })
}

/// `DurationStyle.SIMPLE`: `^([+-]?\d+)([a-zA-Z]{0,2})$`.
fn simple_nanos(text: &str, default_unit: i128) -> Option<i128> {
    let (number, unit) = text.split_at(text.trim_end_matches(char::is_alphabetic).len());
    let per_unit: i128 = match unit.to_ascii_lowercase().as_str() {
        "ns" => 1,
        "us" => 1_000,
        "" => default_unit,
        "ms" => 1_000_000,
        "s" => NANOS_PER_SECOND,
        "m" => 60 * NANOS_PER_SECOND,
        "h" => 3_600 * NANOS_PER_SECOND,
        "d" => 86_400 * NANOS_PER_SECOND,
        _ => return None,
    };
    let number: i64 = number.parse().ok()?;
    i128::from(number).checked_mul(per_unit)
}

/// `java.time.Duration.parse`: `[-+]PnDTnHnMn.nS`, each number signed, at least one part, and
/// `T` followed by one.
fn iso_nanos(text: &str) -> Option<i128> {
    let upper = text.to_ascii_uppercase();
    let (negated, rest) = match upper.as_bytes().first()? {
        b'-' => (true, &upper[1..]),
        b'+' => (false, &upper[1..]),
        _ => (false, upper.as_str()),
    };
    let rest = rest.strip_prefix('P')?;
    let (date, time) = match rest.split_once('T') {
        Some((date, time)) if !time.is_empty() => (date, Some(time)),
        Some(_) => return None,
        None => (rest, None),
    };
    if date.is_empty() && time.is_none() {
        return None;
    }
    let mut total: i128 = 0;
    if !date.is_empty() {
        total = parts(date, &[('D', 86_400 * NANOS_PER_SECOND)])?;
    }
    if let Some(time) = time {
        let units = [
            ('H', 3_600 * NANOS_PER_SECOND),
            ('M', 60 * NANOS_PER_SECOND),
            ('S', NANOS_PER_SECOND),
        ];
        total = total.checked_add(parts(time, &units)?)?;
    }
    Some(if negated { -total } else { total })
}

/// The sum of `<number><unit>` parts in the order of `units`, each at most once; only seconds
/// take a fraction of up to nine digits.
fn parts(mut text: &str, units: &[(char, i128)]) -> Option<i128> {
    let mut total: i128 = 0;
    for &(unit, per_unit) in units {
        let Some((number, rest)) = text.split_once(unit) else {
            continue;
        };
        let (whole, fraction) = match number.split_once(['.', ',']) {
            Some((whole, fraction)) if unit == 'S' => (whole, Some(fraction)),
            Some(_) => return None,
            None => (number, None),
        };
        let whole: i64 = whole.parse().ok()?;
        let mut nanos = i128::from(whole).checked_mul(per_unit)?;
        if let Some(fraction) = fraction {
            if fraction.len() > 9 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let digits: i128 = format!("{fraction:0<9}").parse().ok()?;
            nanos += if number.starts_with('-') {
                -digits
            } else {
                digits
            };
        }
        total = total.checked_add(nanos)?;
        text = rest;
    }
    text.is_empty().then_some(total)
}

#[cfg(test)]
mod tests {
    use super::{MILLISECONDS, SECONDS, iso_nanos, simple_nanos};

    const SECOND: i128 = 1_000_000_000;

    #[test]
    fn simple_durations_use_the_default_unit_without_a_suffix() {
        assert_eq!(simple_nanos("2000", MILLISECONDS), Some(2 * SECOND));
        assert_eq!(simple_nanos("2000", SECONDS), Some(2000 * SECOND));
        assert_eq!(simple_nanos("2s", MILLISECONDS), Some(2 * SECOND));
        assert_eq!(simple_nanos("+5MS", SECONDS), Some(5_000_000));
        assert_eq!(simple_nanos("1d", MILLISECONDS), Some(86_400 * SECOND));
        assert_eq!(simple_nanos("-1m", MILLISECONDS), Some(-60 * SECOND));
        for invalid in ["", "s", "2 s", "2sec", "1.5s", "2w"] {
            assert_eq!(simple_nanos(invalid, MILLISECONDS), None, "{invalid:?}");
        }
    }

    #[test]
    fn iso_durations_follow_java_duration_parse() {
        assert_eq!(iso_nanos("PT2S"), Some(2 * SECOND));
        assert_eq!(iso_nanos("pt0.5s"), Some(SECOND / 2));
        assert_eq!(iso_nanos("PT1,25S"), Some(SECOND + SECOND / 4));
        assert_eq!(iso_nanos("P1DT1H1M"), Some((86_400 + 3_600 + 60) * SECOND));
        assert_eq!(iso_nanos("PT-0.5S"), Some(-SECOND / 2));
        assert_eq!(iso_nanos("-PT2S"), Some(-2 * SECOND));
        assert_eq!(iso_nanos("PT1H-30M"), Some(1_800 * SECOND));
        for invalid in [
            "P",
            "PT",
            "P1H",
            "PT1D",
            "PT1S1M",
            "PT1.5M",
            "PT0.1234567891S",
            "PT1",
        ] {
            assert_eq!(iso_nanos(invalid), None, "{invalid:?}");
        }
    }
}
