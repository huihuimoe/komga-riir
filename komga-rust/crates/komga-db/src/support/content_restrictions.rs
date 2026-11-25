//! SQL form of per-user content restrictions (Kotlin `ContentRestrictionsSearchHelper`).

use std::collections::BTreeSet;

use crate::iden::{SeriesMetadataSharingTable, SeriesMetadataTable};
use komga_types::{AllowExclude, ContentRestrictions};
use sea_query::{Cond, Condition, Expr, ExprTrait, IntoCondition, Query, SelectStatement};

/// Condition hiding restricted series, or `None` when nothing is restricted.
///
/// The condition references `SERIES_METADATA`, which the caller's query must include.
/// Allow rules (age or label) admit a series if either matches; deny rules must all pass.
pub(crate) fn condition(restrictions: &ContentRestrictions) -> Option<Condition> {
    let age_rating = || Expr::col((SeriesMetadataTable::Table, SeriesMetadataTable::AgeRating));
    let series_id = || Expr::col((SeriesMetadataTable::Table, SeriesMetadataTable::SeriesId));

    let mut allow = Vec::new();
    let mut deny = Vec::new();
    if let Some(age_restriction) = restrictions.age_restriction() {
        let age = age_restriction.age;
        match age_restriction.restriction {
            AllowExclude::AllowOnly => allow.push(
                Cond::all()
                    .add(age_rating().is_not_null())
                    .add(age_rating().lte(age)),
            ),
            AllowExclude::Exclude => deny.push(
                Cond::any()
                    .add(age_rating().is_null())
                    .add(age_rating().lt(age)),
            ),
        }
    }
    if !restrictions.labels_allow().is_empty() {
        allow.push(
            series_id()
                .in_subquery(labelled_series(restrictions.labels_allow()))
                .into_condition(),
        );
    }
    if !restrictions.labels_exclude().is_empty() {
        deny.push(
            series_id()
                .not_in_subquery(labelled_series(restrictions.labels_exclude()))
                .into_condition(),
        );
    }

    if allow.is_empty() && deny.is_empty() {
        return None;
    }
    let mut condition = Cond::all();
    if !allow.is_empty() {
        condition = condition.add(allow.into_iter().fold(Cond::any(), Cond::add));
    }
    Some(deny.into_iter().fold(condition, Cond::add))
}

fn labelled_series(labels: &BTreeSet<String>) -> SelectStatement {
    Query::select()
        .column(SeriesMetadataSharingTable::SeriesId)
        .from(SeriesMetadataSharingTable::Table)
        .and_where(
            Expr::col(SeriesMetadataSharingTable::Label).is_in(labels.iter().map(String::as_str)),
        )
        .to_owned()
}
