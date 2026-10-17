//! Compiling engine for Typst.

use std::error::Error;
use std::num::NonZeroUsize;
use std::ops::Range;
use std::vec;

use crate::diagnostics::Diagnostic;
use crate::world::Workspace;
use bytes::Bytes;
use serde::Serialize;
use tsify::Tsify;
use typst::layout::{PageRange, PageRanges};
use typst::{self, World};
use typst_pdf::{PdfOptions, PdfStandard, PdfStandards, Timestamp};
use typst_render;
use typst_svg::svg;

/// The width and height of a rendered page, in points.
#[derive(Debug, Clone, PartialEq, Serialize, Tsify)]
pub struct PageInfo {
    pub width: f64,
    pub height: f64,
}

/// The outcome of a compile which is composed of one `PageInfo` per renderable
/// page plus all diagnostics. On a failed compile the `pages` describe the
/// last successful document, so the presence of an error diagnostic means `pages` is stale.
#[derive(Clone, Debug, PartialEq, Serialize, Tsify)]
pub struct CompileResult {
    pub pages: Vec<PageInfo>,
    pub diagnostics: Vec<Diagnostic>,
}

/// compile accepts a typst world and an entry file, then compile the typst sources
/// into a [`PagedDocument`].
///
/// To get a rendered result, call the render_* functions below.
pub fn compile(
    workspace: &mut Workspace,
    main_file: &str,
) -> Result<CompileResult, Box<dyn Error>> {
    workspace.set_entry(main_file)?;

    let warned = typst::compile(workspace);

    let mut diagnostics: Vec<Diagnostic> = warned
        .warnings
        .iter()
        .map(|d| Diagnostic::from_source(workspace, d))
        .collect();

    match warned.output {
        Ok(doc) => workspace.cache_document(doc),
        Err(dg) => diagnostics.extend(dg.iter().map(|d| Diagnostic::from_source(workspace, d))),
    }

    Ok(CompileResult {
        pages: collect_page_info(workspace),
        diagnostics,
    })
}

/// Render a single page of the last compiled document to SVG, or `None` if there is no
/// cached document or index is out of range.
pub fn render_page(workspace: &Workspace, index: usize) -> Option<String> {
    workspace
        .map_document(|doc| {
            doc.pages()
                .get(index)
                .map(|page| svg(page, &Default::default()))
        })
        .flatten()
}

/// Render a range of pages of the last compiled document to SVGs. If there is no
//// cached document or the range start >= end, a empty list of SVGs is returned.
///
/// Page range are inclusive at both boundaries.
pub fn render_pages(workspace: &Workspace, pages: Option<Range<usize>>) -> Vec<String> {
    workspace
        .map_document(|doc| {
            let all_pages = doc.pages();
            let Range { start, end } = pages.clone().unwrap_or(0..all_pages.len());
            let end = end.min(all_pages.len());
            if start >= end {
                return Vec::new();
            }

            all_pages[start..end]
                .iter()
                .map(|page| svg(page, &Default::default()))
                .collect()
        })
        .unwrap_or_default()
}

/// Render the last compiled document to PDF bytes. If there is no cached document, it
/// returns an empty `Vec`. Diagnotics are returned as [`Err`] when there is any.
///
/// Page range are inclusive at both boundaries.
pub fn render_pdf(
    workspace: &Workspace,
    standards: Option<Vec<String>>,
    use_tag: bool,
    pages: Option<Range<usize>>,
) -> Result<Bytes, Vec<Diagnostic>> {
    let standards_vec = standards.unwrap_or_default();

    let result = workspace.map_document(|doc| {
        let page_range = pages.clone().map(
            |rng| PageRanges::new(vec![convert_to_page_range(rng)]), // only handle one range for now.
        );

        let typed_standards = with_pdf_standards(&standards_vec);
        let pdf_standards = PdfStandards::new(&typed_standards).unwrap_or_default();

        let pdf_options = PdfOptions {
            standards: pdf_standards,
            tagged: use_tag,
            // PDF/A requires a creation date; use the UTC clock.
            timestamp: Some(Timestamp::new_utc(workspace.today(None).unwrap())),
            page_ranges: page_range,
            ..Default::default()
        };

        typst_pdf::pdf(doc, &pdf_options)
    });

    let Some(pdf_output) = result else {
        // When no cached document is found, we only return empty result.
        return Ok(Bytes::new());
    };

    pdf_output.map(Bytes::from).map_err(|errors| {
        errors
            .iter()
            .map(|d| Diagnostic::from_source(workspace, d))
            .collect()
    })
}

/// Render the fist page of the last compiled document to PNG. If there is no cached document, it
/// returns an empty `Vec`.
///
/// Page range are inclusive at both boundaries.
pub fn render_png(
    workspace: &Workspace,
    ppi: i32,
    pages: Option<Range<usize>>,
) -> Result<Vec<Bytes>, Box<dyn Error>> {
    let result = workspace.map_document(|doc| {
        let all_pages = doc.pages();
        if all_pages.len() == 0 {
            return vec![];
        }
        let Range { start, end } = pages.clone().unwrap_or(0..all_pages.len());
        let end = end.min(all_pages.len() - 1);
        if start >= end {
            return Vec::new();
        }

        let mut pixmaps: Vec<Bytes> = Vec::with_capacity(end - start + 1);
        for page in &all_pages[start..=end] {
            let pixmap = typst_render::render(
                page,
                &typst_render::RenderOptions {
                    pixel_per_pt: typst::utils::Scalar::new(ppi as f64 / 72.0),
                    render_bleed: false,
                },
            );

            match pixmap.encode_png() {
                Ok(pixels) => pixmaps.push(Bytes::from(pixels)),
                Err(_) => break,
            };
        }

        pixmaps
    });

    let Some(png_output) = result else {
        // When no cached document is found, we only return empty result.
        return Ok(vec![]);
    };
    Ok(png_output)
}

fn with_pdf_standards(standard_names: &Vec<String>) -> Vec<PdfStandard> {
    let mut standards = Vec::new();

    for name in standard_names {
        if let Some(standard) = parse_standard(name) {
            standards.push(standard)
        }
    }

    standards
}

fn collect_page_info(workspace: &Workspace) -> Vec<PageInfo> {
    workspace
        .map_document(|doc| {
            doc.pages()
                .iter()
                .map(|page| PageInfo {
                    width: page.frame.width().to_pt(),
                    height: page.frame.height().to_pt(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Convert an half-open range to a 1-based inclusive range.
fn convert_to_page_range(src: Range<usize>) -> PageRange {
    let start = src.start + 1;
    let end = src.end; // src end is exclusive.

    PageRange::new(NonZeroUsize::new(start), NonZeroUsize::new(end))
}

/// Parses a PDF standard name (as used by the Typst CLI, e.g. `a-2b`, `ua-1`,
/// `1.7`) into a [`PdfStandard`].
pub(crate) fn parse_standard(name: &str) -> Option<PdfStandard> {
    Some(match name {
        "1.4" => PdfStandard::V_1_4,
        "1.5" => PdfStandard::V_1_5,
        "1.6" => PdfStandard::V_1_6,
        "1.7" => PdfStandard::V_1_7,
        "2.0" => PdfStandard::V_2_0,
        "a-1b" => PdfStandard::A_1b,
        "a-1a" => PdfStandard::A_1a,
        "a-2b" => PdfStandard::A_2b,
        "a-2u" => PdfStandard::A_2u,
        "a-2a" => PdfStandard::A_2a,
        "a-3b" => PdfStandard::A_3b,
        "a-3u" => PdfStandard::A_3u,
        "a-3a" => PdfStandard::A_3a,
        "a-4" => PdfStandard::A_4,
        "a-4f" => PdfStandard::A_4f,
        "a-4e" => PdfStandard::A_4e,
        "ua-1" => PdfStandard::Ua_1,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use crate::compile;
    use crate::world::Workspace;

    #[test]
    fn test_engine_compile() {
        let mut ws = Workspace::new();
        ws.set_file("main.typ", b"hello, world".to_vec())
            .expect("set file error");

        let result = compile::compile(&mut ws, "main.typ").expect("compile failed");
        assert_eq!(result.diagnostics.len(), 0);
        assert_eq!(result.pages.len(), 1);
    }

    #[test]
    fn test_render_page() {
        let mut ws = Workspace::new();
        ws.set_file("main.typ", b"= Header \nhello, world".to_vec())
            .expect("set file error");

        let result = compile::compile(&mut ws, "main.typ").expect("compile failed");
        assert_eq!(result.diagnostics.len(), 0);
        assert_eq!(result.pages.len(), 1);

        let output = compile::render_page(&ws, 0).expect("render failed");
        assert!(output.len() > 0);
    }

    #[test]
    fn test_render_pages() {
        let mut ws = Workspace::new();
        ws.set_file(
            "main.typ",
            b"= Header \nhello, world #lorem(300)\n #pagebreak()\none more line".to_vec(),
        )
        .expect("set file error");

        let result = compile::compile(&mut ws, "main.typ").expect("compile failed");
        assert_eq!(result.diagnostics.len(), 0);
        assert_eq!(result.pages.len(), 2);

        let output = compile::render_pages(&ws, (0..2).into());
        assert!(output.len() == 2);
    }

    #[test]
    fn test_render_pdf() {
        let mut ws = Workspace::new();
        ws.set_file(
            "main.typ",
            b"= Header \nhello, world\none more line\n#lorem(600)".to_vec(),
        )
        .expect("set file error");

        let result = compile::compile(&mut ws, "main.typ").expect("compile failed");
        assert_eq!(result.diagnostics.len(), 0);
        assert_eq!(result.pages.len(), 1);

        let output = compile::render_pdf(&ws, None, true, None).expect("export png failed");
        assert!(output.len() > 0);
    }

    #[test]
    fn test_render_png() {
        let mut ws = Workspace::new();
        ws.set_file(
            "main.typ",
            b"= Header \nhello, world\none more line\n#pagebreak()\n#lorem(600)".to_vec(),
        )
        .expect("set file error");

        let result = compile::compile(&mut ws, "main.typ").expect("compile failed");
        assert_eq!(result.diagnostics.len(), 0);
        assert!(result.pages.len() > 1);

        let output = compile::render_png(&ws, 144, (0..2).into()).expect("export png failed");
        assert!(output.len() > 1);
    }
}
