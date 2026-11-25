//! The image types the pages of listed books are read as.

use std::collections::HashMap;

use komga_db::MediaDao;
use komga_types::{BookDetails, BookId, MediaProfile};

use super::{ReadingError, ReadingService};
use crate::books::analysis::pdf_extractor;

impl ReadingService {
    /// Kotlin `OpdsController`'s page streaming types: the distinct types of the pages of each
    /// DiViNa book and DiViNa-compatible EPUB, the rendered type for PDF books, none for others.
    pub async fn page_types(
        &self,
        books: &[BookDetails],
    ) -> Result<HashMap<BookId, Vec<String>>, ReadingError> {
        let mut types: HashMap<BookId, Vec<String>> = HashMap::new();
        let mut stored = Vec::new();
        for book in books {
            match MediaProfile::from_media_type(book.media.media_type.as_deref()) {
                Some(MediaProfile::Pdf) => {
                    let rendered = pdf_extractor::IMAGE_TYPE.media_type().to_owned();
                    types.insert(book.book.id.clone(), vec![rendered]);
                }
                Some(MediaProfile::Divina) => stored.push(book.book.id.clone()),
                Some(MediaProfile::Epub) if book.media.epub_divina_compatible => {
                    stored.push(book.book.id.clone());
                }
                Some(MediaProfile::Epub) | None => {}
            }
        }
        if !stored.is_empty() {
            let mut tx = self.db.begin().await?;
            let pages = MediaDao::find_page_media_types(&mut tx, &stored).await?;
            tx.rollback().await?;
            for (id, media_type) in pages {
                types.entry(id).or_default().push(media_type);
            }
        }
        Ok(types)
    }
}
