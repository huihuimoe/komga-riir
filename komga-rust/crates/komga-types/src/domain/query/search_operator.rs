//! Kotlin `SearchOperator`: how a search condition compares its field.
//!
//! The JSON form names the operator in an `operator` property next to its operands.

use jiff::{SignedDuration, Span, SpanRelativeTo, Timestamp};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

/// `SearchOperator.Equality`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "operator", rename_all = "camelCase")]
pub enum Equality<T> {
    Is { value: T },
    IsNot { value: T },
}

/// `SearchOperator.EqualityNullable`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "operator", rename_all = "camelCase")]
pub enum EqualityNullable<T> {
    Is { value: T },
    IsNot { value: T },
    IsNull,
    IsNotNull,
}

/// `SearchOperator.StringOp`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "operator", rename_all = "camelCase")]
pub enum StringOperator {
    BeginsWith { value: String },
    DoesNotBeginWith { value: String },
    Contains { value: String },
    DoesNotContain { value: String },
    EndsWith { value: String },
    DoesNotEndWith { value: String },
    Is { value: String },
    IsNot { value: String },
}

/// `SearchOperator.Numeric`; the bounds of `greaterThan` and `lessThan` are inclusive.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "operator", rename_all = "camelCase")]
pub enum Numeric<T> {
    GreaterThan { value: T },
    LessThan { value: T },
    Is { value: T },
    IsNot { value: T },
}

/// `SearchOperator.Date`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "operator", rename_all = "camelCase")]
pub enum NumericNullable<T> {
    GreaterThan { value: T },
    LessThan { value: T },
    Is { value: T },
    IsNot { value: T },
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "operator", rename_all = "camelCase")]
pub enum DateOperator {
    Before {
        #[serde(rename = "dateTime", deserialize_with = "date_time")]
        date_time: Timestamp,
    },
    After {
        #[serde(rename = "dateTime", deserialize_with = "date_time")]
        date_time: Timestamp,
    },
    IsInTheLast {
        #[serde(deserialize_with = "duration")]
        duration: SignedDuration,
    },
    IsNotInTheLast {
        #[serde(deserialize_with = "duration")]
        duration: SignedDuration,
    },
    IsNull,
    IsNotNull,
}

/// `SearchOperator.Boolean`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "operator", rename_all = "camelCase")]
pub enum BooleanOperator {
    IsTrue,
    IsFalse,
}

impl From<bool> for BooleanOperator {
    /// The operator matching `value`, as the deprecated query parameters build it.
    fn from(value: bool) -> Self {
        if value { Self::IsTrue } else { Self::IsFalse }
    }
}

/// A `ZonedDateTime`: an ISO 8601 date-time with an offset, and optionally a zone.
fn date_time<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
    let value = String::deserialize(deserializer)?;
    value.parse().map_err(D::Error::custom)
}

/// A Java `Duration`: ISO 8601 `PnDTnHnMn.nS`, where a day is 24 hours.
fn duration<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SignedDuration, D::Error> {
    let value = String::deserialize(deserializer)?;
    let span: Span = value.parse().map_err(D::Error::custom)?;
    if span.get_years() != 0 || span.get_months() != 0 || span.get_weeks() != 0 {
        return Err(D::Error::custom(format!(
            "Duration {value:?} has years, months or weeks"
        )));
    }
    span.to_duration(SpanRelativeTo::days_are_24_hours())
        .map_err(D::Error::custom)
}

#[cfg(test)]
mod tests {
    use jiff::SignedDuration;
    use serde_json::json;

    use super::{DateOperator, Equality, StringOperator};

    #[test]
    fn operators_read_their_tag_and_operands() {
        let operator: Equality<String> =
            serde_json::from_value(json!({"operator": "isNot", "value": "a"})).expect("Valid");
        assert_eq!(operator, Equality::IsNot { value: "a".into() });
        let operator: StringOperator =
            serde_json::from_value(json!({"operator": "doesNotBeginWith", "value": "a"}))
                .expect("Valid");
        assert_eq!(
            operator,
            StringOperator::DoesNotBeginWith { value: "a".into() }
        );
        assert!(serde_json::from_value::<Equality<String>>(json!({"value": "a"})).is_err());
    }

    #[test]
    fn dates_take_an_offset_and_durations_count_days_as_24_hours() {
        let operator: DateOperator = serde_json::from_value(
            json!({"operator": "after", "dateTime": "2020-01-01T23:00:00-02:00"}),
        )
        .expect("Valid");
        let DateOperator::After { date_time } = operator else {
            panic!("Parsed {operator:?}");
        };
        assert_eq!(date_time.to_string(), "2020-01-02T01:00:00Z");

        let operator: DateOperator =
            serde_json::from_value(json!({"operator": "isInTheLast", "duration": "P2DT1H"}))
                .expect("Valid");
        assert_eq!(
            operator,
            DateOperator::IsInTheLast {
                duration: SignedDuration::from_hours(49)
            }
        );
        let weeks = json!({"operator": "isInTheLast", "duration": "P1W"});
        assert!(serde_json::from_value::<DateOperator>(weeks).is_err());
    }
}
