//! SQL form of search operators (Kotlin `SearchOperatorUtils`).

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use komga_types::{
    BooleanOperator, DateOperator, Equality, EqualityNullable, Numeric, NumericNullable,
    StringOperator,
};
use sea_query::{Expr, ExprTrait, LikeExpr, SelectStatement};

use crate::sqlite::collation::{self, UNICODE_1};
use crate::sqlite::strip_accents;

/// Plain `=`/`<>` against the operand's text.
pub(crate) fn equality<T>(operator: &Equality<T>, column: Expr, text: fn(&T) -> &str) -> Expr {
    match operator {
        Equality::Is { value } => column.eq(text(value)),
        Equality::IsNot { value } => column.ne(text(value)),
    }
}

/// `column IN`/`NOT IN` the rows `matching` selects for the operand.
pub(crate) fn membership<T>(
    operator: &Equality<T>,
    column: Expr,
    matching: impl Fn(&T) -> SelectStatement,
) -> Expr {
    match operator {
        Equality::Is { value } => column.in_subquery(matching(value)),
        Equality::IsNot { value } => column.not_in_subquery(matching(value)),
    }
}

/// `column IN`/`NOT IN` the rows `matching` selects for the operand, or for any value
/// (`matching(None)`) when checking for null.
pub(crate) fn nullable_membership(
    operator: &EqualityNullable<String>,
    column: Expr,
    matching: impl Fn(Option<&str>) -> SelectStatement,
) -> Expr {
    match operator {
        EqualityNullable::Is { value } => column.in_subquery(matching(Some(value))),
        EqualityNullable::IsNot { value } => column.not_in_subquery(matching(Some(value))),
        EqualityNullable::IsNull => column.not_in_subquery(matching(None)),
        EqualityNullable::IsNotNull => column.in_subquery(matching(None)),
    }
}

/// Accent-insensitive pattern matches, ASCII case-insensitive like SQLite `LIKE`; `is`/`isNot`
/// compare by base letters only.
pub(crate) fn string(operator: &StringOperator, column: Expr) -> Expr {
    let like = |prefix: &str, value: &str, suffix: &str| {
        let value = strip_accents::strip_accents(value)
            .replace('!', "!!")
            .replace('%', "!%")
            .replace('_', "!_");
        LikeExpr::new(format!("{prefix}{value}{suffix}")).escape('!')
    };
    let stripped = || strip_accents::call_sql(column.clone());
    match operator {
        StringOperator::BeginsWith { value } => stripped().like(like("", value, "%")),
        StringOperator::DoesNotBeginWith { value } => stripped().not_like(like("", value, "%")),
        StringOperator::EndsWith { value } => stripped().like(like("%", value, "")),
        StringOperator::DoesNotEndWith { value } => stripped().not_like(like("%", value, "")),
        StringOperator::Contains { value } => stripped().like(like("%", value, "%")),
        StringOperator::DoesNotContain { value } => stripped().not_like(like("%", value, "%")),
        StringOperator::Is { value } => unicode_1(column.clone()).eq(value.as_str()),
        StringOperator::IsNot { value } => unicode_1(column.clone()).ne(value.as_str()),
    }
}

/// `column COLLATE COLLATION_UNICODE_1`.
pub(crate) fn unicode_1(column: Expr) -> Expr {
    collation::collate(column, UNICODE_1)
}

/// Compares the UTC dates of the operands; `isInTheLast` counts whole days back from today.
pub(crate) fn date(operator: &DateOperator, column: Expr) -> Expr {
    let utc_date = |instant: Timestamp| instant.to_zoned_datetime(TimeZone::UTC).date().to_string();
    let days_ago = |duration: &SignedDuration| {
        let days = duration.as_secs() / 86_400;
        let today = Timestamp::now().to_zoned_datetime(TimeZone::UTC).date();
        // Durations out of the date range reach past every stored date.
        today
            .checked_sub(SignedDuration::from_hours(days * 24))
            .unwrap_or(if days > 0 { Date::MIN } else { Date::MAX })
            .to_string()
    };
    match operator {
        DateOperator::After { date_time } => column.gt(utc_date(*date_time)),
        DateOperator::Before { date_time } => column.lt(utc_date(*date_time)),
        DateOperator::IsInTheLast { duration } => column.gt(days_ago(duration)),
        DateOperator::IsNotInTheLast { duration } => column.lt(days_ago(duration)),
        DateOperator::IsNull => column.is_null(),
        DateOperator::IsNotNull => column.is_not_null(),
    }
}

/// `greaterThan` and `lessThan` include the bound.
pub(crate) fn numeric(operator: &Numeric<f32>, column: Expr) -> Expr {
    match *operator {
        Numeric::GreaterThan { value } => column.gte(value),
        Numeric::LessThan { value } => column.lte(value),
        Numeric::Is { value } => column.eq(value),
        Numeric::IsNot { value } => column.ne(value),
    }
}

/// Like [`numeric`]; `isNot` also matches null.
pub(crate) fn numeric_nullable(operator: &NumericNullable<i32>, column: Expr) -> Expr {
    match *operator {
        NumericNullable::GreaterThan { value } => column.gte(value),
        NumericNullable::LessThan { value } => column.lte(value),
        NumericNullable::Is { value } => column.eq(value),
        NumericNullable::IsNot { value } => column.clone().ne(value).or(column.is_null()),
        NumericNullable::IsNull => column.is_null(),
        NumericNullable::IsNotNull => column.is_not_null(),
    }
}

pub(crate) fn boolean(operator: BooleanOperator, column: Expr) -> Expr {
    column.eq(operator == BooleanOperator::IsTrue)
}
