//! Budget rules and their measurement on one file snapshot.

#[cfg(test)]
mod tests;

use std::fmt;

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Block, ImplItemFn, ItemFn, ItemImpl, ItemMod, ItemTrait, Signature, TraitItemFn, Type,
    Visibility,
};

use crate::exceptions::{Exception, Exceptions};

#[derive(Clone, Copy)]
struct Limits {
    soft: usize,
    hard: usize,
}

const PRODUCTION_FILE: Limits = Limits {
    soft: 350,
    hard: 600,
};
const USE_CASE_FILE: Limits = Limits {
    soft: 250,
    hard: 400,
};
const TEST_FILE: Limits = Limits {
    soft: 400,
    hard: 700,
};
const FUNCTION: Limits = Limits {
    soft: 60,
    hard: 100,
};

/// Every service module is a business use case or a task entry.
const USE_CASE_DIR: &str = "komga-rust/crates/komga-service/src/";
const SERVICE_SUPPORT_DIR: &str = "komga-rust/crates/komga-service/src/support/";

/// Lines searched for the generated-file marker.
const GENERATED_MARKER_LINES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Production,
    UseCase,
    Test,
}

impl FileKind {
    /// Classifies a repository-relative path.
    pub fn of(path: &str) -> Self {
        let in_tests_dir = path
            .split('/')
            .rev()
            .skip(1)
            .any(|segment| segment == "tests");
        if in_tests_dir || path.ends_with("/tests.rs") {
            Self::Test
        } else if path.starts_with(USE_CASE_DIR)
            && path != format!("{USE_CASE_DIR}lib.rs")
            && !path.starts_with(SERVICE_SUPPORT_DIR)
        {
            Self::UseCase
        } else {
            Self::Production
        }
    }

    fn limits(self) -> Limits {
        match self {
            Self::Production => PRODUCTION_FILE,
            Self::UseCase => USE_CASE_FILE,
            Self::Test => TEST_FILE,
        }
    }
}

impl fmt::Display for FileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Production => "production file",
            Self::UseCase => "use case file",
            Self::Test => "test file",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The responsibilities must be reviewed.
    Warning,
    /// The commit is blocked.
    Error,
}

/// The limit a finding is measured against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    Soft,
    Hard,
    /// The `max_lines` of an approved exception, replacing the hard limit.
    Approved,
}

pub enum Subject {
    File(FileKind),
    Function { line: usize, name: String },
}

pub struct Finding {
    pub severity: Severity,
    pub bound: Bound,
    pub path: String,
    pub subject: Subject,
    pub lines: usize,
    pub limit: usize,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        let bound = match self.bound {
            Bound::Soft => "soft threshold",
            Bound::Hard => "hard limit",
            Bound::Approved => "approved limit",
        };
        match &self.subject {
            Subject::File(kind) => write!(f, "{severity}: {} ({kind})", self.path)?,
            Subject::Function { line, name } => {
                write!(f, "{severity}: {}:{line} fn {name}", self.path)?
            }
        }
        write!(f, ": {} lines, {bound} {}", self.lines, self.limit)
    }
}

/// Returns the declared source of a generated file, which is exempt from all budgets.
///
/// The marker is a `// @generated <source>` comment within the first lines; a marker that does
/// not name its source is ignored, as the exemption must be traceable.
pub fn generated_source(content: &str) -> Option<&str> {
    content
        .lines()
        .take(GENERATED_MARKER_LINES)
        .filter(|line| line.trim_start().starts_with("//"))
        .find_map(|line| {
            line.split_once("@generated")
                .map(|(_, source)| source.trim())
        })
        .filter(|source| !source.is_empty())
}

/// Measures the file and every function in it.
///
/// Files are counted in physical lines. A function spans from its signature (visibility
/// included, attributes and doc comments excluded) to its closing brace, so nested items and
/// comments in the body count. Functions generated inside macro invocations are not seen.
pub fn check(path: &str, content: &str, exceptions: &Exceptions) -> syn::Result<Vec<Finding>> {
    let kind = FileKind::of(path);
    let mut findings: Vec<Finding> = Vec::new();
    let file_lines = content.lines().count();
    if let Some(verdict) = judge(file_lines, kind.limits(), exceptions.find(path, None)) {
        findings.push(Finding {
            severity: verdict.severity,
            bound: verdict.bound,
            path: path.to_owned(),
            subject: Subject::File(kind),
            lines: file_lines,
            limit: verdict.limit,
        });
    }

    let mut collector = FunctionCollector::default();
    collector.visit_file(&syn::parse_file(content)?);
    for function in collector.functions {
        let exception = exceptions.find(path, Some(&function.name));
        if let Some(verdict) = judge(function.lines, FUNCTION, exception) {
            findings.push(Finding {
                severity: verdict.severity,
                bound: verdict.bound,
                path: path.to_owned(),
                subject: Subject::Function {
                    line: function.line,
                    name: function.name,
                },
                lines: function.lines,
                limit: verdict.limit,
            });
        }
    }
    Ok(findings)
}

struct Verdict {
    severity: Severity,
    bound: Bound,
    limit: usize,
}

fn judge(lines: usize, limits: Limits, exception: Option<&Exception>) -> Option<Verdict> {
    let (severity, bound, limit) = match exception {
        Some(exception) if lines > exception.max_lines => {
            (Severity::Error, Bound::Approved, exception.max_lines)
        }
        Some(exception) if lines > limits.hard => {
            (Severity::Warning, Bound::Approved, exception.max_lines)
        }
        _ if lines > limits.hard => (Severity::Error, Bound::Hard, limits.hard),
        _ if lines >= limits.soft => (Severity::Warning, Bound::Soft, limits.soft),
        _ => return None,
    };
    Some(Verdict {
        severity,
        bound,
        limit,
    })
}

struct Function {
    line: usize,
    name: String,
    lines: usize,
}

/// Collects functions with bodies, named by their enclosing modules, impls, traits and functions.
#[derive(Default)]
struct FunctionCollector {
    scope: Vec<String>,
    functions: Vec<Function>,
}

impl FunctionCollector {
    fn record(&mut self, vis: Option<&Visibility>, sig: &Signature, body: &Block) {
        let start: Span = match vis {
            Some(vis @ (Visibility::Public(_) | Visibility::Restricted(_))) => vis.span(),
            _ => sig.span(),
        };
        let line = start.start().line;
        let end = body.brace_token.span.close().end().line;
        let mut name = self.scope.join("::");
        if !name.is_empty() {
            name.push_str("::");
        }
        name.push_str(&sig.ident.to_string());
        self.functions.push(Function {
            line,
            name,
            lines: end - line + 1,
        });
    }

    fn scoped(&mut self, name: String, visit: impl FnOnce(&mut Self)) {
        self.scope.push(name);
        visit(self);
        self.scope.pop();
    }
}

impl<'ast> Visit<'ast> for FunctionCollector {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        self.record(Some(&node.vis), &node.sig, &node.block);
        self.scoped(node.sig.ident.to_string(), |this| {
            visit::visit_item_fn(this, node)
        });
    }

    fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
        self.record(Some(&node.vis), &node.sig, &node.block);
        self.scoped(node.sig.ident.to_string(), |this| {
            visit::visit_impl_item_fn(this, node)
        });
    }

    fn visit_trait_item_fn(&mut self, node: &'ast TraitItemFn) {
        if let Some(body) = &node.default {
            self.record(None, &node.sig, body);
        }
        self.scoped(node.sig.ident.to_string(), |this| {
            visit::visit_trait_item_fn(this, node)
        });
    }

    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        let name = match &*node.self_ty {
            Type::Path(path) => path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string()),
            _ => None,
        };
        self.scoped(name.unwrap_or_else(|| "impl".to_owned()), |this| {
            visit::visit_item_impl(this, node);
        });
    }

    fn visit_item_trait(&mut self, node: &'ast ItemTrait) {
        self.scoped(node.ident.to_string(), |this| {
            visit::visit_item_trait(this, node)
        });
    }

    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        self.scoped(node.ident.to_string(), |this| {
            visit::visit_item_mod(this, node)
        });
    }
}
