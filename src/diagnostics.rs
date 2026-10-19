//! Diagnostic DTOs and the conversion from Typst's `SourceDiagnostic`.
//!
//! Use `tsify` to generate `.d.ts` file so these objects can be used cross wasm boundary.

use serde::Serialize;
use tsify::Tsify;
use typst::World;
use typst::WorldExt;
use typst::diag::{Severity as TypstSeverity, SourceDiagnostic};
use typst::syntax::DiagSpan;

/// Whether a diagnostic is a fatal error or a non-fatal warning.
#[derive(Serialize, Tsify, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// Where a diagnostic points, when its span resolves to a real location.
///
/// Offsets are UTF-8 byte indices, and `line`/`column` are 1-based.
#[derive(Serialize, Tsify, Clone, PartialEq, Eq, Debug)]
pub struct Location {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

/// A single compiler diagnostic.
#[derive(Serialize, Tsify, Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub hints: Vec<String>,
    /// `None` when the span is detached or cannot be resolved.
    pub location: Option<Location>,
}

impl Diagnostic {
    /// Convert one Typst `SourceDiagnostic`, resolving its span against `world`.
    pub fn from_source(world: &dyn World, diag: &SourceDiagnostic) -> Self {
        Diagnostic {
            severity: match diag.severity {
                TypstSeverity::Error => Severity::Error,
                TypstSeverity::Warning => Severity::Warning,
            },
            message: diag.message.to_string(),
            hints: diag.hints.iter().map(|hint| hint.v.to_string()).collect(),
            location: Location::resolve(world, diag.span),
        }
    }
}

impl Location {
    /// Resolve a span to a file location, or `None` if it has no location.
    fn resolve(world: &dyn World, span: DiagSpan) -> Option<Location> {
        let id = span.id()?;
        let range = world.range(span)?;
        let source = world.source(id).ok()?;
        let (line, column) = source.lines().byte_to_line_column(range.start)?;
        Some(Location {
            file: id.vpath().get_without_slash().to_string(),
            start: range.start,
            end: range.end,
            line: line + 1, // convert to 1-based offset
            column: column + 1,
        })
    }
}
