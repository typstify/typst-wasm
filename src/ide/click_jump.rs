//! Click juming from the previewing SVG to source, or from source to the previewing SVG.

use std::num::NonZeroUsize;

use crate::Workspace;

use serde::Serialize;
use tsify::Tsify;
use typst::{
    World,
    introspection::PagedPosition,
    layout::{Abs, Point},
};
use typst_ide::{self, Jump};

/// A click jump kind in the preview which should take the user to different target:
///
/// - `Source`, the click should jump to the editor to that 1-based source location.
/// - `Position`, the click should jump to an internal link.
/// - `Url`, the click should open an external link.
#[derive(Clone, PartialEq, Debug, Serialize, Tsify)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ClickJump {
    Source {
        file: String,
        line: usize,
        column: usize,
    },
    Position {
        page: usize,
        x: f64,
        y: f64,
    },
    Url {
        url: String,
    },
}

/// Resolve a click on the page of the cached document into a source file location,
/// or an internal link in the page, or an external URL. Returns `None` if there
/// is no cached document, or the page is out of range, or the click hit nothing actionable.
pub fn on_page_click_jump(
    world: &Workspace,
    page_index: usize,
    x: f64,
    y: f64,
) -> Option<ClickJump> {
    let click_pos = PagedPosition {
        page: NonZeroUsize::new(page_index + 1)?,
        point: Point::new(Abs::pt(x), Abs::pt(y)),
    };

    world.map_document(|document| {
        let jump = typst_ide::jump_from_click(world, document, &click_pos)?;

        let rv = match jump {
            Jump::File(id, offset) => {
                let source = world.source(id).ok()?;
                let (line, column) = source.lines().byte_to_line_column(offset)?;
                let file_path = world
                    .id_to_path(id)
                    .ok()?
                    .into_os_string()
                    .into_string()
                    .ok()?;

                ClickJump::Source {
                    file: file_path,
                    line: line + 1,
                    column: column + 1,
                }
            }
            Jump::Position(pos) => ClickJump::Position {
                page: pos.page.get().saturating_sub(1),
                x: pos.point.x.to_pt(),
                y: pos.point.y.to_pt(),
            },
            Jump::Url(url) => ClickJump::Url {
                url: url.to_string(),
            },
        };

        Some(rv)
    })
}

/// The position in the laied-out document to jump to when a source cursor moves.
/// `page` is 0-based and (x, y) are points in pixel. This is the reverse of `on_page_click_jump`.
#[derive(Serialize, Tsify, Clone, PartialEq, Debug)]
pub struct CursorJump {
    pub page: usize,
    pub x: f64,
    pub y: f64,
}

/// Detect the position in the laied-out document to jump to when a source cursor moves.
/// When the source_path is not inside the world, or no target position is found, None
/// is returned.
pub fn on_source_cursor_jump(
    world: &Workspace,
    source_path: &str,
    cursor: usize,
) -> Option<CursorJump> {
    let Ok(id) = world.source_id(source_path) else {
        return None;
    };

    let source = world.source(id).ok()?;

    world.map_document(|document| {
        let pos = typst_ide::jump_from_cursor(document, &source, cursor)
            .into_iter()
            .next()?;
        Some(CursorJump {
            page: pos.page.get().saturating_sub(1),
            x: pos.point.x.to_pt(),
            y: pos.point.y.to_pt(),
        })
    })
}
