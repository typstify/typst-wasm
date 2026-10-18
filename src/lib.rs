//! Typst engine compiled to WASM for use in web environment.

mod compile;
mod diagnostics;
mod errors;
mod ide;
mod package;
mod vfs;
mod world;

use std::str::FromStr;

use tsify::{Ts, Tsify, serde_wasm_bindgen};
use typst_syntax::package::PackageSpec;
use typstyle_core::Config;

use crate::ide::click_jump::{self, ClickJump, CursorJump};
pub use crate::ide::completion::{Completion, CompletionKind, CompletionResult, auto_complete};

pub use crate::compile::{
    CompileResult, PageInfo, compile, render_page, render_pages, render_pdf, render_png,
};
pub use crate::diagnostics::{Diagnostic, Location, Severity};
use crate::ide::hover::{HoverTooltip, on_hover};
use crate::ide::{format, highlight};
pub use crate::world::Workspace;

use js_sys::Uint8Array;
use serde::Deserialize;
use wasm_bindgen::prelude::*;

/// How many compile generations of memoized results `comemo` retains. Trimmed
/// after each compile to bound memory across an editing session.
const COMEMO_MAX_AGE: usize = 20;

#[derive(Deserialize, Tsify, Clone, PartialEq, Eq, Debug)]
struct PackageMeta {
    pub spec: String,
    pub desc: Option<String>,
}

/// typst_version returns the version of the typst compiler.
#[wasm_bindgen]
pub fn typst_version() -> String {
    typst_utils::version().raw().to_string()
}

#[wasm_bindgen]
pub struct Engine {
    workspace: Workspace,
}

#[wasm_bindgen]
impl Engine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            workspace: Workspace::new(),
        }
    }

    /// Add a typst package to the workspace as gzip compressed tarball and
    /// extract the package file tree into the package cache.
    pub fn add_package(
        &mut self,
        spec: &str,
        tgz_files: Vec<u8>,
        max_total: u64,
    ) -> Result<JsValue, JsError> {
        let parsed_spec = PackageSpec::from_str(spec).map_err(|e| JsError::new(&e))?;

        self.workspace
            .add_package_tar(parsed_spec, &tgz_files, max_total)
            .map(|val| serde_wasm_bindgen::to_value(&val).unwrap())
            .map_err(|e| JsError::from(e.as_ref()))
    }

    /// remove a package from the workspace by package spec.
    pub fn remove_package(&mut self, spec: &str) -> Result<(), JsError> {
        let parsed_spec = PackageSpec::from_str(spec).map_err(|e| JsError::new(&e))?;

        self.workspace
            .remove_package(parsed_spec)
            .map_err(|e| JsError::from(e.as_ref()))
    }

    /// add user accessible package list to the workspace, so the editor can use this to generate
    /// completion candidates when importing packages.
    pub fn set_accessible_packages(&mut self, packages: JsValue) -> Result<(), JsError> {
        let packages: Vec<PackageMeta> =
            serde_wasm_bindgen::from_value(packages).map_err(|e| JsError::new(&e.to_string()))?;

        let pkgs: Vec<(PackageSpec, Option<String>)> = packages
            .into_iter()
            .map(|PackageMeta { spec, desc }| {
                PackageSpec::from_str(&spec)
                    .map(|spec| (spec, desc))
                    .map_err(|e| JsError::new(e.as_str()))
            })
            .collect::<Result<_, _>>()?;

        self.workspace.set_accessible_packages(pkgs);
        Ok(())
    }

    /// add font to the workspace.
    pub fn add_font(&mut self, bytes: Vec<u8>) -> Vec<String> {
        self.workspace.add_font(bytes)
    }

    /// clear all custom fonts in the workspace. We don't have a remove_font
    /// method so caller must call clear_fonts first then call add_font
    /// to add other fonts back.
    pub fn clear_fonts(&mut self) {
        self.workspace.clear_fonts();
    }

    /// Insert or replace normal files into the user workspace.
    /// The `file_path` is a path relative to the workspace `source_dir`.
    pub fn set_file(&mut self, file_path: &str, bytes: Vec<u8>) -> Result<(), JsError> {
        self.workspace
            .set_file(file_path, bytes)
            .map_err(|e| JsError::from(e.as_ref()))
    }

    /// Remove normal files from the user workspace.
    /// The `file_path` is a path relative to the workspace `source_dir`.
    pub fn remove_file(&mut self, file_path: &str) -> Result<(), JsError> {
        self.workspace
            .remove_file(file_path)
            .map_err(|e| JsError::from(e.as_ref()))
    }

    pub fn set_today(&mut self, year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) {
        self.workspace
            .set_today(year, month, day, hour, minute, second);
    }

    /// Compile the typst sources into a [`PagedDocument`]. A main file must be specified so the Typst
    /// compiler knows where to start.
    /// To get rendered results, please call render_* methods below.
    pub fn compile(&mut self, main_file: &str) -> Result<JsValue, JsError> {
        let result = compile(&mut self.workspace, main_file)
            .map(|val| serde_wasm_bindgen::to_value(&val).unwrap())
            .map_err(|e| JsError::from(e.as_ref()));

        // Trim memoized entries recent compiles no longer touch.
        comemo::evict(COMEMO_MAX_AGE);

        result
    }

    /// Render a single page of the last compiled document to SVG, or `None` if there is no
    /// cached document or index is out of range.
    pub fn render_page(&self, index: usize) -> Option<String> {
        render_page(&self.workspace, index)
    }

    /// Render a range of pages of the last compiled document to SVGs. If there is no
    //// cached document or the range start >= end, a empty list of SVGs is returned.
    ///
    /// Page range are inclusive at both boundaries.
    pub fn render_pages(&self, start: Option<usize>, end: Option<usize>) -> Vec<String> {
        let pages = match (start, end) {
            (Some(s), Some(e)) => Some(s..e),
            _ => None,
        };
        render_pages(&self.workspace, pages)
    }

    /// Render the last compiled document to PDF bytes. If there is no cached document, it
    /// returns an empty list.
    ///
    /// Page range is specified as start_page(inclusive) and end_page(inclusive).
    ///
    /// Diagnotics are returned when there is any.
    pub fn render_pdf(
        &self,
        standards: Option<Vec<String>>,
        use_tag: bool,
        start_page: Option<usize>,
        end_page: Option<usize>,
    ) -> Result<Vec<u8>, Vec<Ts<Diagnostic>>> {
        let pages = match (start_page, end_page) {
            (Some(s), Some(e)) => Some(s..e),
            _ => None, // Falls back to rendering everything if bounds aren't fully provided
        };

        render_pdf(&self.workspace, standards, use_tag, pages)
            .map(|val| val.to_vec())
            .map_err(|diags| diags.into_iter().map(|d| d.into_ts().unwrap()).collect())
    }

    /// Render the fist page of the last compiled document to PNG. If there is no cached document, it
    /// returns an empty list.
    ///
    /// Page range is specified as start_page(inclusive) and end_page(inclusive).
    pub fn render_png(
        &self,
        ppi: i32,
        start_page: Option<usize>,
        end_page: Option<usize>,
    ) -> Result<Vec<Uint8Array>, JsError> {
        let pages = match (start_page, end_page) {
            (Some(s), Some(e)) => Some(s..e),
            _ => None, // Falls back to rendering everything if bounds aren't fully provided
        };

        render_png(&self.workspace, ppi, pages)
            .map(|pages| {
                pages
                    .into_iter()
                    .map(|page| Uint8Array::from(page.as_ref()))
                    .collect()
            })
            .map_err(|err| JsError::from(err.as_ref()))
    }

    pub fn auto_complete(
        &self,
        source_path: &str,
        explicit: bool,
        cursor: usize,
    ) -> Result<Ts<CompletionResult>, JsError> {
        let id = self
            .workspace
            .source_id(source_path)
            .map_err(|err| JsError::from(err.as_ref()))?;

        auto_complete(&self.workspace, id, explicit, cursor)
            .and_then(|c| {
                let ts = c.into_ts()?;
                Ok(ts)
            })
            .map_err(|err| JsError::from(err.as_ref()))
    }

    pub fn hover(&self, source_path: &str, cursor: usize) -> Option<Ts<HoverTooltip>> {
        let Ok(id) = self.workspace.source_id(source_path) else {
            return None;
        };

        on_hover(&self.workspace, id, cursor).and_then(|v| {
            let Ok(ts) = v.into_ts() else {
                return None;
            };

            Some(ts)
        })
    }

    /// Highlight typst source to list of [highlight::HighlightSpan]. Offsets of `from` and `to` are
    /// bytes offset. Spans may nest and the result ensures parent spans always emit first.
    ///
    /// This method does not depends on a compiled document.
    pub fn highlight(&self, source: &str, from: usize, to: usize) -> Option<JsValue> {
        if source.is_empty() {
            return None;
        }

        let spans = highlight::highlight_source(source, from, to);
        Some(serde_wasm_bindgen::to_value(&spans).unwrap())
    }

    /// Highlight typst source to html fragment wrapped in \<code\> element.
    ///
    /// This method does not depends on a compiled document.
    pub fn highlight_to_html(&self, source: &str) -> Option<String> {
        if source.is_empty() {
            return None;
        }

        Some(highlight::highlight_to_html(source))
    }

    /// Format typst source using specified format config, returning the formatted source.
    ///
    /// `format_cfg` is deserialized to [`Config`] internally, to see all possible config keys,
    /// refer to the `Config` document.
    pub fn format_source(&self, source_path: &str, format_cfg: Option<JsValue>) -> Option<String> {
        let Ok(id) = self.workspace.source_id(source_path) else {
            return None;
        };

        let format_cfg =
            format_cfg.and_then(|val| serde_wasm_bindgen::from_value::<Config>(val).ok());

        format::format_source(&self.workspace, id, format_cfg)
    }

    /// Resolve a click on the page of the cached document into a source file location,
    /// or an internal link in the page, or an external URL. Returns `None` if there
    /// is no cached document, or the page is out of range, or the click hit nothing actionable.
    ///
    /// page_index is zero-based page index. x, y is the pixel position in the page.
    pub fn on_page_click_jump(&self, page_index: usize, x: f64, y: f64) -> Option<Ts<ClickJump>> {
        click_jump::on_page_click_jump(&self.workspace, page_index, x, y).and_then(|c| {
            let Ok(ts) = c.into_ts() else {
                return None;
            };

            Some(ts)
        })
    }

    /// Detect the position in the laied-out document to jump to when a source cursor moves.
    /// When the source_path is not inside the world, or no target position is found, None
    /// is returned.
    pub fn on_source_cursor_jump(
        &self,
        source_path: &str,
        cursor: usize,
    ) -> Option<Ts<CursorJump>> {
        click_jump::on_source_cursor_jump(&self.workspace, source_path, cursor).and_then(|c| {
            let Ok(ts) = c.into_ts() else {
                return None;
            };

            Some(ts)
        })
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
