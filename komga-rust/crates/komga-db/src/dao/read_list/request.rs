//! Kotlin `ReadListRequestDao`.

use komga_types::{BookId, ReadListBookMatch, ReadListRequestBook, ReadListSeriesMatch, SeriesId};
use serde_json::json;

use crate::support::local_date_time::SqlDate;
use crate::{DbError, DbTransaction};

/// A series title matches under SQLite's ASCII-only `NOCASE`, a number once both lose their
/// leading zeros. Soft-deleted books match too, as in Kotlin. The requests travel as one JSON
/// array of `[index, series title, number]` rows, so their count is not bound by SQLite's
/// variable limit; rows keep request order, and series follow the order of the titles.
const MATCH_SQL: &str = r#"
SELECT request.value ->> 0, sd.SERIES_ID, sd.TITLE, bd.BOOK_ID, bd.NUMBER, bd.TITLE,
    bma.RELEASE_DATE
FROM json_each(?) AS request
JOIN SERIES_METADATA sd ON request.value ->> 1 = sd.TITLE COLLATE NOCASE
LEFT JOIN BOOK_METADATA_AGGREGATION bma ON bma.SERIES_ID = sd.SERIES_ID
JOIN BOOK b ON b.SERIES_ID = sd.SERIES_ID
JOIN BOOK_METADATA bd ON bd.BOOK_ID = b.ID
    AND ltrim(bd.NUMBER, '0') = ltrim(request.value ->> 2, '0') COLLATE NOCASE
ORDER BY request.key, sd.SERIES_ID, bd.BOOK_ID
"#;

type MatchRow = (
    i64,
    SeriesId,
    String,
    BookId,
    String,
    String,
    Option<SqlDate>,
);

pub struct ReadListRequestDao;

impl ReadListRequestDao {
    /// `matchBookRequests`: for each request, in order, the series holding a matching book with
    /// those books.
    pub async fn match_book_requests(
        transaction: &mut DbTransaction,
        requests: &[ReadListRequestBook],
    ) -> Result<Vec<Vec<ReadListSeriesMatch>>, DbError> {
        let rows: Vec<_> = requests
            .iter()
            .enumerate()
            .flat_map(|(index, request)| {
                request
                    .series
                    .iter()
                    .map(move |series| json!([index, series, request.number]))
            })
            .collect();
        let found: Vec<MatchRow> = sqlx::query_as(MATCH_SQL)
            .bind(serde_json::Value::Array(rows).to_string())
            .fetch_all(transaction.connection())
            .await?;
        let mut matches: Vec<Vec<ReadListSeriesMatch>> = vec![Vec::new(); requests.len()];
        for (index, series_id, series_title, book_id, number, title, release_date) in found {
            let index = usize::try_from(index).expect("Request indexes are not negative");
            let series_matches = &mut matches[index];
            let book = ReadListBookMatch {
                book_id,
                number,
                title,
            };
            match series_matches.last_mut() {
                Some(last) if last.series_id == series_id => last.books.push(book),
                _ => {
                    let release_date = release_date.map(|date| date.0);
                    series_matches.push(ReadListSeriesMatch {
                        series_id,
                        series_title,
                        release_date,
                        books: vec![book],
                    });
                }
            }
        }
        Ok(matches)
    }
}
