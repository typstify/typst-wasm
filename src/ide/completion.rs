//! Implement auto-completion feature.

use std::error::Error;

use serde::Serialize;
use tsify::Tsify;

use typst::World;
use typst_ide::{
    Completion as TypstCompletion, CompletionKind as TypstCompletionKind, autocomplete,
};
use typst_syntax::FileId;

use crate::Workspace;

/// Completion kind defined by Typst.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Tsify)]
pub enum CompletionKind {
    /// A syntactical structure.
    Syntax,
    /// A function.
    Func,
    /// A type.
    Type,
    /// A function parameter.
    Param,
    /// A constant.
    Constant,
    /// A file path.
    Path,
    /// A package.
    Package,
    /// A label.
    Label,
    /// A font family.
    Font,
    /// A symbol.
    Symbol,
}

impl From<&TypstCompletionKind> for CompletionKind {
    fn from(kind: &TypstCompletionKind) -> Self {
        match kind {
            TypstCompletionKind::Syntax => CompletionKind::Syntax,
            TypstCompletionKind::Func => CompletionKind::Func,
            TypstCompletionKind::Type => CompletionKind::Type,
            TypstCompletionKind::Param => CompletionKind::Param,
            TypstCompletionKind::Constant => CompletionKind::Constant,
            TypstCompletionKind::Path => CompletionKind::Path,
            TypstCompletionKind::Package => CompletionKind::Package,
            TypstCompletionKind::Label => CompletionKind::Label,
            TypstCompletionKind::Font => CompletionKind::Font,
            // The symbol value itself is carried in `label`/`apply`, so drop the payload.
            TypstCompletionKind::Symbol(_) => CompletionKind::Symbol,
        }
    }
}

/// A completion candidate.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Tsify)]
pub struct Completion {
    pub kind: CompletionKind,
    pub label: String,
    /// Insertion text, possibly with snippet placeholders like `${body}`.
    /// Falls back to `label` when `None`.
    pub apply: Option<String>,
    pub detail: Option<String>,
}

impl From<&TypstCompletion> for Completion {
    fn from(comp: &TypstCompletion) -> Self {
        Completion {
            kind: (&comp.kind).into(),
            label: comp.label.to_string(),
            apply: comp.apply.as_ref().map(|s| s.to_string()),
            detail: comp.detail.as_ref().map(|d| d.to_string()),
        }
    }
}

/// Completion response, inclusing from, the byte offset indicating the start
/// of the completion, and completions which contains all completion candidates.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Tsify)]
pub struct CompletionResult {
    pub from: usize,
    pub completions: Vec<Completion>,
}

/// handling auto-completion request from editor and returns completion candidates
/// if there is any.
pub fn auto_complete(
    world: &Workspace,
    source_id: FileId,
    explicit: bool,
    cursor: usize,
) -> Result<CompletionResult, Box<dyn Error>> {
    let source = world.source(source_id)?;

    let doc_guard = world.document();

    let doc = doc_guard?;

    if let Some((from, completions)) = autocomplete(world, doc.as_ref(), &source, cursor, explicit)
    {
        return Ok(CompletionResult {
            from: from,
            completions: completions.iter().map(|comp| comp.into()).collect(),
        });
    }

    Ok(CompletionResult {
        from: cursor,
        completions: Vec::new(),
    })
}
