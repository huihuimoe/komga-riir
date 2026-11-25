use std::fs;

use komga_test_support::TestDir;
use komga_types::SearchEntity;

use super::{RebuildReason, SearchError, SearchIndex};
use crate::{IndexUpdate, NgramOptions, SearchDocument};

const CORPUS: &[(&str, &[(&str, &str)])] = &[
    (
        "s1",
        &[
            ("title", "Batman: The Dark Knight"),
            ("writer", "Frank Miller"),
            ("author", "Frank Miller"),
            ("tag", "action"),
            ("release_date", "1986"),
        ],
    ),
    (
        "s2",
        &[
            ("title", "Superman"),
            ("author", "Joe Shuster"),
            ("writer", "Jerry Siegel"),
            ("release_date", "1938"),
        ],
    ),
    (
        "s3",
        &[
            ("title", "進撃の巨人"),
            ("author", "諫山創"),
            ("tag", "action"),
        ],
    ),
    (
        "s4",
        &[
            ("title", "Batman Begins"),
            ("tag", "movie"),
            ("release_date", "2005"),
        ],
    ),
    (
        "s5",
        &[("title", "Café au lait"), ("publisher", "Kodansha")],
    ),
    ("s6", &[("title", "3月のライオン"), ("genre", "Drama")]),
    (
        "s7",
        &[
            ("title", "X-Men"),
            ("penciller", "Jack Kirby"),
            ("author", "Jack Kirby"),
            ("release_date", "1963"),
        ],
    ),
    ("s8", &[("title", "Men in Black")]),
    (
        "s9",
        &[
            ("title", "Spider-Man"),
            ("writer", "Stan Lee"),
            ("author", "Stan Lee"),
        ],
    ),
    ("s10", &[("title", "巨人の星")]),
];

/// The hits of Lucene 9.9.1 over [`CORPUS`] with Komga's analyzers (index-side CJK unigrams
/// on), most relevant first, after dropping the appended `*:*` where another clause can match.
const LUCENE_HITS: &[(&str, &[&str])] = &[
    ("batman", &["s4", "s1"]),
    ("dark knight", &["s1"]),
    ("\"dark knight\"", &["s1"]),
    ("\"batman knight\"~3", &["s1"]),
    ("bat*", &["s1", "s4"]),
    ("b?tman", &["s1", "s4"]),
    ("ba*man", &["s1", "s4"]),
    ("batman~1", &["s4", "s1"]),
    ("batmn~1", &["s4", "s1"]),
    ("/bat.*/", &["s1", "s4"]),
    ("[a TO c]", &["s1", "s4", "s5", "s8"]),
    ("release_date:[1950 TO 2000]", &["s1", "s4", "s7"]),
    ("release_date:{1938 TO 1986]", &["s1", "s7"]),
    ("release_date:[* TO 1950]", &["s2", "s4"]),
    ("writer:miller", &["s1"]),
    ("writer:mil*", &["s1"]),
    ("writer:millr~1", &["s1"]),
    ("writer:[m TO n]", &["s1"]),
    ("writer:/s.*/", &["s2", "s9"]),
    ("penciller:kirby", &["s7"]),
    ("author:\"frank miller\"", &["s1"]),
    ("author:\"stan lee\"", &["s9"]),
    ("tag:action", &["s1", "s3"]),
    (
        "-batman",
        &["s2", "s3", "s5", "s6", "s7", "s8", "s9", "s10"],
    ),
    ("batman -begins", &["s1"]),
    ("batman OR superman", &["s2", "s4", "s1"]),
    ("(batman OR superman) -dark", &["s2", "s4"]),
    ("進撃", &["s3"]),
    ("巨人", &["s10", "s3"]),
    ("進撃の巨人", &["s3", "s10"]),
    ("進撃巨人", &["s3", "s10"]),
    ("3月", &["s6"]),
    ("月", &["s6"]),
    ("x-men", &["s7", "s8"]),
    ("title:x-men", &["s7"]),
    ("cafe", &["s5"]),
    ("CAFÉ", &["s5"]),
    ("caf*", &["s5"]),
    ("men", &["s7", "s8"]),
    ("man", &["s2", "s4", "s9", "s1"]),
    (
        "*:*",
        &["s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10"],
    ),
    (
        "-tag:action",
        &["s2", "s4", "s5", "s6", "s7", "s8", "s9", "s10"],
    ),
    ("nosuch:batman", &[]),
    ("batman superman", &[]),
    ("kodansha", &[]),
    ("publisher:kodansha", &["s5"]),
    ("genre:dra*", &["s6"]),
];

/// Same hits, other order: fuzzy matches score as constants, and a term's n-grams count
/// towards its field's length, where Lucene counts positions.
const ORDERED_DIFFERENTLY: &[&str] = &["batman~1", "batmn~1", "man"];

fn series(id: &str, fields: &[(&str, &str)]) -> SearchDocument {
    let mut document = SearchDocument::new(SearchEntity::Series, id.to_owned());
    for (field, value) in fields {
        document.add(field, value);
    }
    document
}

/// The index in `directory`, with a current generation.
fn open(directory: &TestDir) -> SearchIndex {
    let index =
        SearchIndex::open(directory.path(), NgramOptions::default()).expect("the index opens");
    if index.rebuild_reason().is_some() {
        let generation = index.new_generation().expect("a generation is created");
        index
            .switch(generation)
            .expect("the generation becomes current");
    }
    index
}

fn apply(index: &SearchIndex, updates: &[IndexUpdate]) {
    index
        .current()
        .expect("a current generation")
        .apply(updates)
        .expect("the updates are applied");
}

fn corpus_index(directory: &TestDir) -> SearchIndex {
    let index = open(directory);
    let updates: Vec<IndexUpdate> = CORPUS
        .iter()
        .map(|(id, fields)| IndexUpdate::Upsert(series(id, fields)))
        .collect();
    apply(&index, &updates);
    index
}

fn search(index: &SearchIndex, entity: SearchEntity, query: &str) -> Vec<String> {
    let mut hits = index.hits(entity, query).expect("the index is readable");
    hits.next_batch(hits.len()).expect("the IDs are readable")
}

#[test]
fn searches_find_what_lucene_finds() {
    let directory = TestDir::new().expect("a temporary directory");
    let index = corpus_index(&directory);

    for (query, lucene) in LUCENE_HITS {
        let mut hits = search(&index, SearchEntity::Series, query);
        let mut lucene = lucene.to_vec();
        if ORDERED_DIFFERENTLY.contains(query) {
            hits.sort_unstable();
            lucene.sort_unstable();
        }
        assert_eq!(hits, lucene, "query {query:?}");
    }
}

#[test]
fn a_negative_group_keeps_everything_else() {
    let directory = TestDir::new().expect("a temporary directory");
    let index = corpus_index(&directory);

    let hits = search(&index, SearchEntity::Series, "title:(-batman) tag:action");

    assert_eq!(hits, ["s3"]);
}

#[test]
fn invalid_syntax_finds_nothing() {
    let directory = TestDir::new().expect("a temporary directory");
    let index = corpus_index(&directory);

    for query in ["!!!", "AC/DC", "*man", "(batman", "batman~1.5", "/[/"] {
        assert!(
            search(&index, SearchEntity::Series, query).is_empty(),
            "query {query:?}"
        );
    }
}

#[test]
fn updates_replace_and_delete_by_entity_and_id() {
    let directory = TestDir::new().expect("a temporary directory");
    let index = open(&directory);
    let mut book = SearchDocument::new(SearchEntity::Book, "b1".to_owned());
    book.add("title", "Batman");
    apply(
        &index,
        &[
            IndexUpdate::Upsert(book),
            IndexUpdate::Upsert(series("b1", &[("title", "Batman")])),
        ],
    );

    apply(
        &index,
        &[IndexUpdate::Upsert(series("b1", &[("title", "Superman")]))],
    );

    assert_eq!(
        search(&index, SearchEntity::Series, "batman"),
        Vec::<String>::new()
    );
    assert_eq!(search(&index, SearchEntity::Series, "superman"), ["b1"]);
    assert_eq!(search(&index, SearchEntity::Book, "batman"), ["b1"]);

    let delete = IndexUpdate::Delete {
        entity: SearchEntity::Book,
        id: "b1".to_owned(),
    };
    apply(&index, &[delete]);

    assert_eq!(
        search(&index, SearchEntity::Book, "batman"),
        Vec::<String>::new()
    );
    assert_eq!(search(&index, SearchEntity::Series, "superman"), ["b1"]);
}

#[test]
fn committed_updates_survive_reopening() {
    let directory = TestDir::new().expect("a temporary directory");
    drop(corpus_index(&directory));

    let index = open(&directory);

    assert_eq!(
        search(&index, SearchEntity::Series, "writer:miller"),
        ["s1"]
    );
}

#[test]
fn book_queries_search_title_and_isbn_by_default() {
    let directory = TestDir::new().expect("a temporary directory");
    let index = open(&directory);
    let mut book = SearchDocument::new(SearchEntity::Book, "b1".to_owned());
    book.add("title", "Watchmen");
    book.add("isbn", "9780930289232");
    let mut collection = SearchDocument::new(SearchEntity::Collection, "c1".to_owned());
    collection.add("name", "Watchmen");
    apply(
        &index,
        &[IndexUpdate::Upsert(book), IndexUpdate::Upsert(collection)],
    );

    assert_eq!(search(&index, SearchEntity::Book, "9780930289232"), ["b1"]);
    assert_eq!(search(&index, SearchEntity::Book, "watchmen"), ["b1"]);
    assert_eq!(search(&index, SearchEntity::Collection, "watchmen"), ["c1"]);
}

#[test]
fn hiragana_and_katakana_find_each_other() {
    let directory = TestDir::new().expect("a temporary directory");
    let index = open(&directory);
    apply(
        &index,
        &[
            IndexUpdate::Upsert(series("s1", &[("title", "ワンピース")])),
            IndexUpdate::Upsert(series("s2", &[("title", "らーめん大好き小泉さん")])),
        ],
    );

    for (query, expected) in [
        ("わんぴーす", "s1"),
        ("ﾜﾝﾋﾟｰｽ", "s1"),
        ("ラーメン", "s2"),
        ("title:らー*", "s2"),
        ("\"小泉サン\"", "s2"),
    ] {
        assert_eq!(
            search(&index, SearchEntity::Series, query),
            [expected],
            "query {query:?}"
        );
    }
}

#[test]
fn a_new_index_is_unavailable_until_a_generation_is_switched_in() {
    let directory = TestDir::new().expect("a temporary directory");
    let index =
        SearchIndex::open(directory.path(), NgramOptions::default()).expect("the index opens");
    assert_eq!(index.rebuild_reason(), Some(RebuildReason::Missing));
    assert!(matches!(
        index.hits(SearchEntity::Series, "batman"),
        Err(SearchError::Unavailable(RebuildReason::Missing))
    ));

    let generation = index.new_generation().expect("a generation is created");
    generation
        .apply(&[IndexUpdate::Upsert(series("s1", &[("title", "Batman")]))])
        .expect("the generation is written");
    index
        .switch(generation)
        .expect("the generation becomes current");
    assert_eq!(index.rebuild_reason(), None);
    drop(index);

    let index = open(&directory);
    assert_eq!(search(&index, SearchEntity::Series, "batman"), ["s1"]);
}

#[test]
fn changed_ngram_settings_keep_the_old_generation_until_the_switch() {
    let directory = TestDir::new().expect("a temporary directory");
    drop(corpus_index(&directory));
    let four_grams = NgramOptions::new(4, 4, false).expect("valid n-gram settings");
    let reopen = || SearchIndex::open(directory.path(), four_grams).expect("the index opens");
    let matches = |index: &SearchIndex, query| {
        let mut hits = search(index, SearchEntity::Series, query);
        hits.sort_unstable();
        hits
    };

    // A rebuild that never switches leaves the old generation current, and is removed.
    let index = reopen();
    assert_eq!(index.rebuild_reason(), Some(RebuildReason::NgramChanged));
    let abandoned = index.new_generation().expect("a generation is created");
    abandoned.apply(&[]).expect("the generation is written");
    drop((abandoned, index));
    let index = reopen();
    assert_eq!(index.rebuild_reason(), Some(RebuildReason::NgramChanged));
    assert_eq!(matches(&index, "atm"), ["s1", "s4"]);
    assert!(!directory.path().join("tantivy/2").exists());

    let generation = index.new_generation().expect("a generation is created");
    let updates: Vec<IndexUpdate> = CORPUS
        .iter()
        .map(|(id, fields)| IndexUpdate::Upsert(series(id, fields)))
        .collect();
    generation
        .apply(&updates)
        .expect("the generation is written");
    index
        .switch(generation)
        .expect("the generation becomes current");

    assert_eq!(index.rebuild_reason(), None);
    assert!(search(&index, SearchEntity::Series, "atm").is_empty());
    assert_eq!(matches(&index, "atma"), ["s1", "s4"]);
    assert!(!directory.path().join("tantivy/1").exists());
}

#[test]
fn an_index_of_another_format_or_layout_is_rebuilt() {
    let directory = TestDir::new().expect("a temporary directory");
    drop(corpus_index(&directory));
    let root = directory.path().join("tantivy");
    let manifest = fs::read_to_string(root.join("generation.json")).expect("a manifest");
    fs::write(
        root.join("generation.json"),
        manifest.replace(r#""format":1"#, r#""format":0"#),
    )
    .expect("the manifest is replaced");
    fs::write(root.join("meta.json"), "{}").expect("a file of the old layout");

    let index =
        SearchIndex::open(directory.path(), NgramOptions::default()).expect("the index opens");

    assert_eq!(index.rebuild_reason(), Some(RebuildReason::OtherFormat));
    let mut left: Vec<_> = fs::read_dir(&root)
        .expect("the index directory")
        .map(|entry| entry.expect("an entry").file_name())
        .collect();
    left.sort();
    assert_eq!(left, ["generation.json"]);
}

#[test]
fn hits_are_read_in_batches_from_the_reader_they_started_with() {
    let directory = TestDir::new().expect("a temporary directory");
    let index = open(&directory);
    let updates: Vec<IndexUpdate> = (0..5)
        .map(|n| IndexUpdate::Upsert(series(&format!("s{n}"), &[("title", "Batman")])))
        .collect();
    apply(&index, &updates);

    let mut hits = index
        .hits(SearchEntity::Series, "batman")
        .expect("the index is readable");
    apply(
        &index,
        &[IndexUpdate::Delete {
            entity: SearchEntity::Series,
            id: "s0".to_owned(),
        }],
    );

    assert_eq!(hits.len(), 5);
    let mut read = Vec::new();
    loop {
        let batch = hits.next_batch(2).expect("the IDs are readable");
        if batch.is_empty() {
            break;
        }
        assert!(batch.len() <= 2, "{batch:?}");
        read.extend(batch);
    }
    read.sort_unstable();
    assert_eq!(read, ["s0", "s1", "s2", "s3", "s4"]);
    assert!(hits.is_empty());
    assert_eq!(search(&index, SearchEntity::Series, "batman").len(), 4);
}
