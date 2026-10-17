//! Query hover tips when cursor is pointed to some source text.

use serde::Serialize;
use tsify::Tsify;
use typst::World;
use typst_ide::{Tooltip, tooltip};
use typst_syntax::{FileId, Side};

use crate::Workspace;

/// Kind of a hover tooltip.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Tsify)]
pub enum TooltipKind {
    Text,
    Code,
}

/// A hover tooltip.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Tsify)]
pub struct HoverTooltip {
    pub kind: TooltipKind,
    pub value: String,
}

/// Query hover tooltip at `cursor` in the source file. `cursor` should be byte offset.
pub fn on_hover(world: &Workspace, source_id: FileId, cursor: usize) -> Option<HoverTooltip> {
    let Ok(source) = world.source(source_id) else {
        return None;
    };

    let doc_guard = world.document();
    let Ok(doc) = doc_guard else { return None };

    let tip = tooltip(world, doc.as_ref(), &source, cursor, Side::Before);

    tip.map(|t| match t {
        Tooltip::Text(text) => HoverTooltip {
            kind: TooltipKind::Text,
            value: text.to_string(),
        },
        Tooltip::Code(code) => HoverTooltip {
            kind: TooltipKind::Code,
            value: code.to_string(),
        },
    })
}
