use chrono::{Datelike, FixedOffset, Local, Timelike, Utc};
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use typst::ecow::EcoString;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt, World};
use typst_ide::IdeWorld;
use typst_layout::PagedDocument;
use typst_syntax::package::PackageSpec;
use typst_syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};

use crate::errors::EngineErr;
use crate::package::{PackageRegistry, UnpackSummary, unpacked_package};
use crate::vfs::Vfs;

/// An in-memory Typst project world.
pub struct Workspace {
    vfs: Vfs,
    /// directory inside the vfs as root dir of package cache.
    packages_dir: PathBuf,
    /// Project root directory (sources, images, data files).
    root: PathBuf,

    /// External packages metadata to faciliate auto-completion of package
    /// importsduring editing.
    package_registry: PackageRegistry,

    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    main_file: FileId,

    // Parsed sources
    sources: Mutex<HashMap<FileId, Source>>,

    // The last successfully compiled document.
    document: Mutex<Option<PagedDocument>>,

    today: Option<Datetime>,
}

impl Default for Workspace {
    fn default() -> Self {
        let (book, fonts) = embedded_fonts();

        Self {
            vfs: Vfs::new(),
            packages_dir: PathBuf::from("packages"),
            root: PathBuf::from("src"),
            package_registry: PackageRegistry::new(),

            library: LazyHash::new(Library::default()),
            book: LazyHash::new(book),
            fonts,
            main_file: FileId::new(RootedPath::new(
                VirtualRoot::Project,
                VirtualPath::new("/main.typ").unwrap(),
            )),
            sources: Mutex::new(HashMap::new()),
            document: Mutex::new(None),
            today: None,
        }
    }
}

impl Workspace {
    pub fn new() -> Self {
        Default::default()
    }

    /// Add a typst package to the workspace as gzip compressed tarball.
    pub fn add_package_tar(
        &mut self,
        spec: PackageSpec,
        tgz_bytes: &[u8],
        max_total: u64,
    ) -> Result<UnpackSummary, Box<dyn Error>> {
        unpacked_package(&mut self.vfs, &spec, tgz_bytes, max_total).inspect(|_| {
            self.package_registry.add(spec, None);
        })
    }

    /// Add a typst package to the workspace. The content of the package is normalized to
    /// a map of relative file path in the package to file content.
    pub fn add_package(
        &mut self,
        spec: PackageSpec,
        files: HashMap<String, Vec<u8>>,
    ) -> Result<(), Box<dyn Error>> {
        for (file, bytes) in files.into_iter() {
            let file_path = Path::new(&file).as_os_str().to_str();
            if let Some(path) = file_path {
                let id = Vfs::file_id(VirtualRoot::Package(spec.clone()), path)?;
                self.vfs.set(id, bytes);
            } else {
                return Err(Box::new(EngineErr::PathError(file.clone())));
            }
        }

        self.package_registry.add(spec, None);

        Ok(())
    }

    /// Remove a package from the workspace. Parsed sources of the removed package are dropped
    ///  as well, otherwise the compiler would keep serving them from the cache.
    pub fn remove_package(&mut self, spec: PackageSpec) -> Result<(), Box<dyn Error>> {
        let removed = self.vfs.remove_where(|id| {
            if let VirtualRoot::Package(pkg_spec) = id.root()
                && *pkg_spec == spec
            {
                return true;
            }

            false
        });

        let mut sources = self.sources.lock().unwrap();
        for id in removed {
            sources.remove(&id);
        }

        Ok(())
    }

    /// add user accessible package list to the workspace, so the editor can use this to generate
    /// completion items when importing packages.
    pub fn set_accessible_packages(&mut self, packages: Vec<(PackageSpec, Option<String>)>) {
        self.package_registry.add_many(packages);
    }

    // Clear all accessible package list.
    pub fn clear_accessible_packages(&mut self) {
        self.package_registry.clear();
    }

    /// add a font to the workspace by providing a font file name and file content.
    pub fn add_font(&mut self, bytes: Vec<u8>) -> Vec<String> {
        let buffer = Bytes::new(bytes);
        let mut families = Vec::new();
        for font in Font::iter(buffer) {
            families.push(font.info().family.to_string());
            self.book.push(font.info().clone());
            self.fonts.push(font);
        }

        families
    }

    /// Clear all custom fonts set by `add_font`.
    ///
    /// We have no way to remove fonts from [`FontBook`], so this method
    /// can't remove fonts by file name. Instead we clear all fonts and then
    /// add them back.
    ///
    /// pub fn remove_font(&mut self, font_file: String) {
    ///     unimplemented!("")
    /// }
    pub fn clear_fonts(&mut self) {
        let (book, fonts) = embedded_fonts();

        self.book = LazyHash::new(book);
        self.fonts = fonts;
    }

    pub fn source_id(&self, file_path: &str) -> Result<FileId, Box<dyn Error>> {
        Ok(Vfs::file_id(VirtualRoot::Project, file_path)?)
    }

    /// Insert or replace normal files into the user workspace.
    /// The `file_path` is a path relative to the workspace `source_dir`.
    pub fn set_file(&mut self, file_path: &str, bytes: Vec<u8>) -> Result<(), Box<dyn Error>> {
        let id = Vfs::file_id(VirtualRoot::Project, file_path)?;

        // only process text files.
        let mut src = self.sources.lock().unwrap();
        if src.contains_key(&id) {
            match std::str::from_utf8(bytes.as_ref()) {
                Ok(text) => {
                    if let Some(source) = src.get_mut(&id) {
                        source.replace(text);
                    }
                }
                // Non-UTF-8 bytes cannot back a `Source`; drop the stale cached one.
                Err(_) => {
                    src.remove(&id);
                }
            }
        }

        self.vfs.set(id, bytes);

        Ok(())
    }

    /// Remove normal files from the user workspace.
    /// The `file_path` is a path relative to the workspace `source_dir`.
    pub fn remove_file(&mut self, file_path: &str) -> Result<(), Box<dyn Error>> {
        let id = Vfs::file_id(VirtualRoot::Project, file_path)?;

        // only process text files.
        let mut src = self.sources.lock().unwrap();
        src.remove(&id);
        self.vfs.remove(id);

        Ok(())
    }

    pub fn set_entry(&mut self, entry: &str) -> Result<(), Box<dyn Error>> {
        self.main_file = Vfs::file_id(VirtualRoot::Project, entry).unwrap();

        Ok(())
    }

    /// When running in browser there is no clock, we have to push the date from the
    /// javascript side before each compile.`set_today` before each compile.
    pub fn set_today(&mut self, year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) {
        if let Some(dt) = Datetime::from_ymd_hms(year, month, day, hour, minute, second) {
            self.today = Some(dt);
        }
    }

    /// Cache a freshly compiled document so its pages can be rendered on demand.
    pub fn cache_document(&self, document: PagedDocument) {
        let mut doc = self.document.lock().unwrap();
        *doc = Some(document);
    }

    /// Maps the PagedDocument to R by applying func to the reference of [`PagedDocument`].
    pub fn map_document<F, R>(&self, func: F) -> Option<R>
    where
        F: Fn(&PagedDocument) -> Option<R>,
    {
        // Clone the document out of the lock before invoking the callback.
        // Rendering SVG can be lengthy, and callbacks such as typst-svg's
        // image/font resolvers must never observe the document mutex held
        // across their execution.
        let doc = self.document.lock().unwrap().as_ref().cloned()?;
        func(&doc)
    }

    // Get a mutex guared document reference for use outside the world.
    pub fn document(&self) -> Result<MutexGuard<'_, Option<PagedDocument>>, &'static str> {
        self.document.lock().map_err(|_| "document mutex poisoned")
    }

    /// Resolves a virtual file ID to a virtual path within its root.
    pub fn id_to_vpath(&self, id: FileId) -> String {
        id.vpath().get_without_slash().to_string()
    }

    /// Resolves a virtual file ID to a real path on the VFS.
    pub fn id_to_path(&self, id: FileId) -> FileResult<PathBuf> {
        let vpath = id.vpath();
        match id.root() {
            VirtualRoot::Project => vpath.realize(&self.root),
            VirtualRoot::Package(spec) => {
                let root = self
                    .packages_dir
                    .join(spec.namespace.as_str())
                    .join(spec.name.as_str())
                    .join(spec.version.to_string());
                vpath.realize(&root)
            }
        }
        .map_err(FileError::Realize)
    }
}

// asset macro from the typst-assets project
macro_rules! asset {
    ($path:literal) => {
        include_bytes!(concat!("../fonts/", $path)).as_slice()
    };
}

fn embedded_fonts() -> (FontBook, Vec<Font>) {
    let mut book = FontBook::new();
    let mut fonts = Vec::new();

    let embedded = [
        asset!("LibertinusSerif-Regular.otf"),
        asset!("LibertinusSerif-Bold.otf"),
        asset!("LibertinusSerif-Italic.otf"),
        asset!("NewCMMath-Regular.otf"),
        asset!("NewCM10-Regular.otf"),
        asset!("NewCM10-Bold.otf"),
        asset!("DejaVuSansMono-Bold.ttf"),
        asset!("DejaVuSansMono.ttf"),
    ];

    for data in embedded.into_iter() {
        let buf = Bytes::new(data);
        for font in Font::iter(buf) {
            book.push(font.info().clone());
            fonts.push(font)
        }
    }

    (book, fonts)
}

impl World for Workspace {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main_file
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        let mut cache = self.sources.lock().unwrap();
        if let Some(source) = cache.get(&id) {
            return Ok(source.clone());
        }

        let bytes = self.vfs.lookup(id)?;
        let text = String::from_utf8(bytes.into())?;
        let source = Source::new(id, text);

        cache.insert(id, source.clone());
        Ok(source)
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.vfs.lookup(id).map(Bytes::new)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, offset: Option<Duration>) -> Option<Datetime> {
        if self.today.is_some() {
            return self.today;
        }

        match offset {
            Some(off) => {
                let tz_offset = FixedOffset::east_opt(off.seconds() as i32).unwrap();
                let now = Utc::now().with_timezone(&tz_offset);
                let time = now.time();

                Datetime::from_ymd_hms(
                    now.year(),
                    now.month() as u8,
                    now.day() as u8,
                    time.hour() as u8,
                    now.minute() as u8,
                    now.second() as u8,
                )
            }

            None => {
                let now = Local::now();
                let time = now.time();

                Datetime::from_ymd_hms(
                    now.year(),
                    now.month() as u8,
                    now.day() as u8,
                    time.hour() as u8,
                    now.minute() as u8,
                    now.second() as u8,
                )
            }
        }
    }
}

impl IdeWorld for Workspace {
    fn upcast(&self) -> &dyn World {
        self
    }

    /// A list of all available packages and optionally descriptions for them.
    ///
    /// This function is **optional** to implement. It enhances the user
    /// experience by enabling autocompletion for packages.
    fn packages(&self) -> &[(PackageSpec, Option<EcoString>)] {
        self.package_registry.all()
    }

    /// Returns a list of all known files.
    fn files(&self) -> Vec<FileId> {
        self.vfs.all().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    use std::str::FromStr;

    fn package_spec() -> PackageSpec {
        PackageSpec::from_str("@preview/demo:1.2.3").unwrap()
    }

    fn package_file_id(spec: &PackageSpec, path: &str) -> FileId {
        Vfs::file_id(VirtualRoot::Package(spec.clone()), path).unwrap()
    }

    const PACKAGE_MANIFEST: &[u8] =
        b"[package]\nname = \"demo\"\nversion = \"1.2.3\"\nentrypoint = \"src/lib.typ\"\n";

    const PACKAGE_ENTRIES: &[(&str, &[u8])] = &[
        ("typst.toml", PACKAGE_MANIFEST),
        (
            "src/lib.typ",
            b"#import \"util.typ\": punctuation\n#let greeting = [Hello from package] + punctuation",
        ),
        ("src/util.typ", b"#let punctuation = [.]"),
    ];

    fn assert_package_import_compiles(ws: &mut Workspace) {
        ws.set_file(
            "main.typ",
            b"#import \"@preview/demo:1.2.3\": greeting\n#greeting".to_vec(),
        )
        .expect("set file error");

        let result = crate::compile::compile(ws, "main.typ").expect("compile failed");

        assert_eq!(result.diagnostics.len(), 0);
        assert_eq!(result.pages.len(), 1);
    }

    /// Build an in-memory `.tar.gz` archive from `(path, content)` entries.
    fn make_tgz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, path, *data).unwrap();
        }

        let tar = builder.into_inner().unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn test_new_workspace_defaults() {
        let ws = Workspace::new();

        assert_eq!(ws.root, PathBuf::from("src"));
        assert_eq!(ws.packages_dir, PathBuf::from("packages"));
        assert!(ws.package_registry.all().is_empty());
        assert!(!ws.fonts.is_empty());
        assert!(ws.map_document(|_| Some(())).is_none());
    }

    #[test]
    fn test_set_file_and_read_back() {
        let mut ws = Workspace::new();
        ws.set_file("chapters/intro.typ", b"= Intro".to_vec())
            .expect("set file error");

        let id = ws.source_id("chapters/intro.typ").unwrap();
        assert_eq!(ws.source(id).unwrap().text(), "= Intro");
        assert_eq!(ws.file(id).unwrap().to_vec(), b"= Intro");
        assert_eq!(
            ws.id_to_path(id).unwrap(),
            PathBuf::from("src/chapters/intro.typ")
        );
    }

    #[test]
    fn test_missing_file_returns_error() {
        let ws = Workspace::new();
        let id = ws.source_id("missing.typ").unwrap();

        assert!(ws.source(id).is_err());
        assert!(ws.file(id).is_err());
    }

    #[test]
    fn test_set_file_refreshes_cached_source() {
        let mut ws = Workspace::new();
        ws.set_file("main.typ", b"old".to_vec()).unwrap();
        let id = ws.source_id("main.typ").unwrap();

        // Populate the source cache before replacing the file.
        assert_eq!(ws.source(id).unwrap().text(), "old");

        ws.set_file("main.typ", b"new".to_vec()).unwrap();
        assert_eq!(ws.source(id).unwrap().text(), "new");
    }

    #[test]
    fn test_set_file_non_utf8_drops_cached_source() {
        let mut ws = Workspace::new();
        ws.set_file("main.typ", b"old".to_vec()).unwrap();
        let id = ws.source_id("main.typ").unwrap();
        assert!(ws.source(id).is_ok());

        ws.set_file("main.typ", vec![0xff, 0xfe]).unwrap();

        // The stale text must not be served after the bytes became non-UTF-8.
        assert!(ws.source(id).is_err());
        assert_eq!(ws.file(id).unwrap().to_vec(), vec![0xff, 0xfe]);
    }

    #[test]
    fn test_remove_file_clears_source_and_vfs() {
        let mut ws = Workspace::new();
        ws.set_file("main.typ", b"hello".to_vec()).unwrap();
        let id = ws.source_id("main.typ").unwrap();
        assert!(ws.source(id).is_ok());

        ws.remove_file("main.typ").unwrap();

        assert!(ws.source(id).is_err());
        assert!(ws.file(id).is_err());
        assert_eq!(ws.vfs.size(), 0);
    }

    #[test]
    fn test_set_entry_changes_main_file() {
        let mut ws = Workspace::new();
        ws.set_file("chapter.typ", b"= Chapter".to_vec()).unwrap();

        ws.set_entry("chapter.typ").unwrap();

        assert_eq!(ws.main(), ws.source_id("chapter.typ").unwrap());
    }

    #[test]
    fn test_add_and_remove_package() {
        let mut ws = Workspace::new();
        ws.set_file("main.typ", b"project".to_vec()).unwrap();

        let spec = package_spec();
        let files = HashMap::from([
            ("typst.toml".to_string(), b"[package]".to_vec()),
            ("src/lib.typ".to_string(), b"#let x = 1".to_vec()),
        ]);
        ws.add_package(spec.clone(), files).unwrap();

        assert_eq!(ws.vfs.size(), 3);
        assert_eq!(ws.package_registry.all().len(), 1);

        let lib_id = package_file_id(&spec, "src/lib.typ");
        assert_eq!(ws.source(lib_id).unwrap().text(), "#let x = 1");
        assert_eq!(
            ws.id_to_path(lib_id).unwrap(),
            PathBuf::from("packages/preview/demo/1.2.3/src/lib.typ")
        );

        ws.remove_package(spec).unwrap();

        // Only the package files are removed; project files stay put.
        assert_eq!(ws.vfs.size(), 1);
        assert!(ws.source(lib_id).is_err());
        let main_id = ws.source_id("main.typ").unwrap();
        assert_eq!(ws.source(main_id).unwrap().text(), "project");
    }

    #[test]
    fn test_add_package_tar() {
        let mut ws = Workspace::new();
        let spec = package_spec();
        let entries: &[(&str, &[u8])] =
            &[("typst.toml", b"[package]"), ("src/lib.typ", b"#let x = 1")];
        let tgz = make_tgz(entries);

        let summary = ws
            .add_package_tar(spec.clone(), &tgz, 1024 * 1024)
            .expect("add package tar failed");

        let expected_bytes: u64 = entries.iter().map(|(_, data)| data.len() as u64).sum();
        assert_eq!(summary.files, 2);
        assert_eq!(summary.bytes, expected_bytes);
        assert_eq!(ws.vfs.size(), 2);
        assert_eq!(ws.package_registry.all().len(), 1);

        let id = package_file_id(&spec, "src/lib.typ");
        assert_eq!(ws.file(id).unwrap().to_vec(), b"#let x = 1");
    }

    #[test]
    fn test_compile_with_package_import() {
        let mut ws = Workspace::new();
        let files = PACKAGE_ENTRIES
            .iter()
            .map(|(path, data)| (path.to_string(), data.to_vec()))
            .collect();

        ws.add_package(package_spec(), files).unwrap();

        assert_package_import_compiles(&mut ws);
    }

    #[test]
    fn test_compile_with_package_import_from_tar() {
        let mut ws = Workspace::new();
        let tgz = make_tgz(PACKAGE_ENTRIES);

        ws.add_package_tar(package_spec(), &tgz, 1024 * 1024)
            .expect("add package tar failed");

        assert_package_import_compiles(&mut ws);
    }

    #[test]
    fn test_add_package_tar_respects_size_limit() {
        let mut ws = Workspace::new();
        let tgz = make_tgz(&[("src/lib.typ", b"12345")]);

        let err = ws
            .add_package_tar(package_spec(), &tgz, 4)
            .expect_err("package should exceed the byte limit");

        assert_eq!(err.to_string(), "unpack package error");
        assert!(ws.package_registry.all().is_empty());
        assert_eq!(ws.vfs.size(), 0);
    }

    #[test]
    fn test_set_accessible_packages() {
        let mut ws = Workspace::new();
        let first = package_spec();
        let second = PackageSpec::from_str("@preview/other:0.1.0").unwrap();

        ws.set_accessible_packages(vec![
            (first.clone(), Some("demo package".into())),
            (second.clone(), None),
        ]);

        assert_eq!(ws.package_registry.all().len(), 2);
        assert_eq!(
            ws.package_registry.all()[0],
            (first, Some(EcoString::from("demo package")))
        );
        assert_eq!(ws.package_registry.all()[1].0, second);
    }

    #[test]
    fn test_set_today() {
        let mut ws = Workspace::new();
        let expected = Datetime::from_ymd_hms(2024, 2, 29, 12, 34, 56).unwrap();

        ws.set_today(2024, 2, 29, 12, 34, 56);
        assert_eq!(ws.today(None), Some(expected));

        // Invalid dates are ignored and keep the last valid value.
        ws.set_today(2023, 2, 29, 0, 0, 0);
        assert_eq!(ws.today(None), Some(expected));
    }

    // #[test]
    // fn test_add_font_and_clear_fonts() {
    //     let mut ws = Workspace::new();
    //     let embedded = ws.fonts.len();
    //     let font = typst_assets::fonts()
    //         .next()
    //         .expect("embedded fonts should not be empty")
    //         .to_vec();

    //     let families = ws.add_font(font);

    //     assert!(!families.is_empty());
    //     assert!(ws.fonts.len() > embedded);

    //     ws.clear_fonts();
    //     assert_eq!(ws.fonts.len(), embedded);
    // }
}
