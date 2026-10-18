//! Tool used to format typst source.

use typst::World;
use typst_syntax::FileId;
use typstyle_core::{Config, Typstyle};

use crate::world::Workspace;

/// Format the file at `path`. Returns `None` if it is absent or has syntax
/// errors (invalid source cannot be formatted).
///
/// Uses the default style:
///     tab_spaces 2,
///     max_width 80
/// For full default value, see [`Config`].
pub fn format_source(
    world: &Workspace,
    source_id: FileId,
    format_cfg: Option<Config>,
) -> Option<String> {
    let Ok(source) = world.source(source_id) else {
        return None;
    };

    let format_cfg = format_cfg.unwrap_or_default();

    Typstyle::new(format_cfg)
        .format_source(source)
        .render()
        .ok()
}
