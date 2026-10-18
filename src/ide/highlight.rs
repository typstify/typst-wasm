//! Source highlighting for Typst using typst-syntax crate.

use serde::Serialize;
use tsify::Tsify;
use typst_syntax::{LinkedNode, highlight, highlight_html, parse};
use wasm_bindgen::prelude::*;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Tsify)]
pub struct HighlightSpan {
    /// Start offset in bytes.
    pub from: usize,
    /// End offset in bytes (exclusive).
    pub to: usize,
    /// tag as typst's recommended CSS class.
    pub css_class: String,
    /// tag as text-mate scope
    pub tm_scope: String,
}

/// Highlight typst source to list of [HighlightSpan]. Offsets of `from` and `to` are
/// bytes offset. Spans may nest and the result ensures parent spans always emit first.
pub fn highlight_source(text: &str, from: usize, to: usize) -> Vec<HighlightSpan> {
    let root = parse(text);
    let mut spans = Vec::new();
    traverse_node(&LinkedNode::new(&root), from, to, &mut spans);
    spans
}

/// Do a pre-order DFS traversal against the `node` to collect all highlight spans. Spans can
/// be nested, so the parent node is always emitted first and child nodes after parent node
/// can have the same start offset.
fn traverse_node(node: &LinkedNode, from: usize, to: usize, output: &mut Vec<HighlightSpan>) {
    let range = node.range();
    if range.start >= to || range.end <= from {
        return;
    }

    if let Some(tag) = highlight(node) {
        output.push(HighlightSpan {
            from: range.start,
            to: range.end,
            css_class: tag.css_class().to_string(),
            tm_scope: tag.tm_scope().to_string(),
        });
    }

    for child in node.children() {
        traverse_node(&child, from, to, output);
    }
}

/// Highlight typst source to a <code> html fragment. Best for static typst source rendering.
pub fn highlight_to_html(text: &str) -> String {
    highlight_html(&parse(text))
}
