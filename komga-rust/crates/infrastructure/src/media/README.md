# Media infrastructure

This subtree owns persisted media lookup plus filesystem and archive access used to serve,
analyze, or maintain book media content.

## Files in this subtree

- `content/`: persisted records, page rendering, EPUB resources, and content resolution.
- `formats/`: Pdfium loading plus RAR and ZIP format primitives.
- `maintenance/`: media-derived maintenance operations such as page hashing.
- `progress/`: read-progress persistence and aggregation.
- `reader.rs`: application media read-port adapter.

## Keep outside this subtree

- HTTP request and response mapping.
- Application-level media import or metadata orchestration.
- Unrelated filesystem concerns such as fonts.
