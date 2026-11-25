use super::*;

const API_FILE: &str = "komga-rust/crates/komga-api/src/a.rs";

/// A function whose definition spans exactly `lines` lines.
fn function(name: &str, lines: usize) -> String {
    let body = "    let _ = 0;\n".repeat(lines - 2);
    format!("fn {name}() {{\n{body}}}\n")
}

fn file_of(lines: usize) -> String {
    "// line\n".repeat(lines)
}

fn verdicts(findings: &[Finding]) -> Vec<(Severity, Bound, usize)> {
    findings
        .iter()
        .map(|finding| (finding.severity, finding.bound, finding.limit))
        .collect()
}

fn approved(entries: &str) -> Exceptions {
    Exceptions::parse(entries).expect("valid exceptions")
}

#[test]
fn classifies_paths() {
    let cases = [
        (
            "komga-rust/crates/komga-db/src/dao/komga_user.rs",
            FileKind::Production,
        ),
        (
            "komga-rust/crates/komga-service/src/identity/users.rs",
            FileKind::UseCase,
        ),
        (
            "komga-rust/crates/komga-service/src/lib.rs",
            FileKind::Production,
        ),
        (
            "komga-rust/crates/komga-service/src/support/blocking.rs",
            FileKind::Production,
        ),
        (
            "komga-rust/crates/komga-rust/tests/it/main.rs",
            FileKind::Test,
        ),
        (
            "komga-rust/crates/komga-service/src/identity/tests.rs",
            FileKind::Test,
        ),
        (
            "komga-rust/crates/komga-db/src/tests_support.rs",
            FileKind::Production,
        ),
    ];
    for (path, kind) in cases {
        assert_eq!(FileKind::of(path), kind, "{path}");
    }
}

#[test]
fn file_thresholds_per_kind() {
    let service = "komga-rust/crates/komga-service/src/x/a.rs";
    let test = "komga-rust/crates/komga-rust/tests/it/a.rs";
    let cases = [
        (API_FILE, 349, None),
        (API_FILE, 350, Some((Severity::Warning, Bound::Soft, 350))),
        (API_FILE, 600, Some((Severity::Warning, Bound::Soft, 350))),
        (API_FILE, 601, Some((Severity::Error, Bound::Hard, 600))),
        (service, 249, None),
        (service, 250, Some((Severity::Warning, Bound::Soft, 250))),
        (service, 401, Some((Severity::Error, Bound::Hard, 400))),
        (test, 601, Some((Severity::Warning, Bound::Soft, 400))),
        (test, 700, Some((Severity::Warning, Bound::Soft, 400))),
        (test, 701, Some((Severity::Error, Bound::Hard, 700))),
    ];
    for (path, lines, expected) in cases {
        let findings = check(path, &file_of(lines), &Exceptions::default()).expect("parses");
        assert_eq!(
            verdicts(&findings).first().copied(),
            expected,
            "{path} with {lines} lines"
        );
        assert!(findings.iter().all(|finding| finding.lines == lines));
    }
}

#[test]
fn function_thresholds_and_names() {
    let source = format!(
        "{}{}\nstruct Foo;\nimpl Foo {{\n    pub(crate)\n{}}}\nmod tests {{\n{}}}\n",
        function("short", 59),
        function("long", 60),
        function("method", 101),
        function("helper", 100),
    );
    let findings = check(API_FILE, &source, &Exceptions::default()).expect("parses");
    let summary: Vec<(Severity, &str, usize, usize)> = findings
        .iter()
        .map(|finding| match &finding.subject {
            Subject::Function { line, name } => {
                (finding.severity, name.as_str(), *line, finding.lines)
            }
            Subject::File(_) => panic!("unexpected file finding {finding}"),
        })
        .collect();
    assert_eq!(
        summary,
        [
            (Severity::Warning, "long", 60, 60),
            // `pub(crate)` on its own line starts the definition.
            (Severity::Error, "Foo::method", 123, 102),
            (Severity::Warning, "tests::helper", 227, 100),
        ]
    );
}

#[test]
fn nested_and_trait_default_functions() {
    let nested = function("inner", 61).replace('\n', "\n    ");
    let source =
        format!("trait Run {{\n    fn run() {{\n        {nested}\n    }}\n    fn decl();\n}}\n");
    let findings = check(API_FILE, &source, &Exceptions::default()).expect("parses");
    let names: Vec<String> = findings
        .iter()
        .map(|finding| match &finding.subject {
            Subject::Function { name, .. } => format!("{name}:{}", finding.lines),
            Subject::File(_) => panic!("unexpected file finding {finding}"),
        })
        .collect();
    assert_eq!(names, ["Run::run:64", "Run::run::inner:61"]);
}

#[test]
fn rejects_unparsable_source() {
    assert!(check(API_FILE, "fn broken( {", &Exceptions::default()).is_err());
}

#[test]
fn approved_exception_replaces_only_its_hard_limit() {
    let exceptions = approved(&format!(
        r#"
        [[exception]]
        path = "{API_FILE}"
        max_lines = 650
        reason = "r"

        [[exception]]
        path = "{API_FILE}"
        function = "Foo::run"
        max_lines = 120
        reason = "r"
        "#
    ));
    let cases = [
        (500, Some((Severity::Warning, Bound::Soft, 350))),
        (650, Some((Severity::Warning, Bound::Approved, 650))),
        (651, Some((Severity::Error, Bound::Approved, 650))),
    ];
    for (lines, expected) in cases {
        let findings = check(API_FILE, &file_of(lines), &exceptions).expect("parses");
        assert_eq!(
            verdicts(&findings).first().copied(),
            expected,
            "{lines} lines"
        );
    }
    // Another file keeps the default limit.
    let other = "komga-rust/crates/komga-api/src/b.rs";
    let findings = check(other, &file_of(601), &exceptions).expect("parses");
    assert_eq!(verdicts(&findings), [(Severity::Error, Bound::Hard, 600)]);

    let methods = |run: usize, other: usize| {
        let source = format!(
            "struct Foo;\nimpl Foo {{\n{}{}}}\n",
            function("run", run),
            function("other", other)
        );
        verdicts(&check(API_FILE, &source, &exceptions).expect("parses"))
    };
    assert_eq!(
        methods(120, 101),
        [
            (Severity::Warning, Bound::Approved, 120),
            (Severity::Error, Bound::Hard, 100)
        ]
    );
    assert_eq!(methods(121, 10), [(Severity::Error, Bound::Approved, 120)]);
}

#[test]
fn generated_marker_must_name_its_source() {
    let marked = |header: &str| format!("{header}\n{}", file_of(700));
    assert_eq!(
        generated_source(&marked("// @generated by build.rs from migrations/main")),
        Some("by build.rs from migrations/main")
    );
    assert_eq!(
        generated_source(&marked("//! @generated from schema.json")),
        Some("from schema.json")
    );
    assert_eq!(generated_source(&marked("// @generated")), None);
    assert_eq!(
        generated_source("let marker = \"@generated by me\";\n"),
        None
    );
    let late = format!("{}// @generated by tool\n", file_of(GENERATED_MARKER_LINES));
    assert_eq!(generated_source(&late), None);
}
