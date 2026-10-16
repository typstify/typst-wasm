use chrono::{Datelike, FixedOffset, Local, Timelike, Utc};
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Mutex;
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
        unpacked_package(
            &mut self.vfs,
            &self.packages_dir,
            &spec,
            tgz_bytes,
            max_total,
        )
        .and_then(|v| {
            self.package_registry.add(&spec, None);
            Ok(v)
        })
    }

    /// Add a typst package to the workspace. The content of the package is normalized to
    /// a map of relative file path in the package to file content.
    pub fn add_package(
        &mut self,
        spec: PackageSpec,
        files: HashMap<String, Vec<u8>>,
    ) -> Result<(), Box<dyn Error>> {
        let pkg_root = self
            .packages_dir
            .join(spec.namespace.to_string())
            .join(spec.name.to_string())
            .join(spec.version.to_string());

        for (file, bytes) in files.into_iter() {
            let file_path = pkg_root.join(file).into_os_string().into_string();
            if let Ok(path) = file_path {
                let id = Vfs::file_id(VirtualRoot::Package(spec.clone()), &path)?;
                self.vfs.set(id, bytes);
            }
        }

        self.package_registry.add(&spec, None);

        Ok(())
    }

    pub fn remove_package(&mut self, spec: PackageSpec) -> Result<(), Box<dyn Error>> {
        let pkg_root = self
            .packages_dir
            .join(spec.namespace.to_string())
            .join(spec.name.to_string())
            .join(spec.version.to_string())
            .into_os_string()
            .into_string()
            .map_err(|os_str| EngineErr::PathError(os_str.to_string_lossy().to_string()))?;

        self.vfs.remove_by_prefix(&pkg_root);
        self.package_registry.remove(&spec);

        Ok(())
    }

    /// add user accessible package list to the workspace, so the editor can use this to generate
    /// completion items when importing packages.
    pub fn set_accessible_packages(&mut self, packages: &[(PackageSpec, Option<String>)]) {
        self.package_registry.add_many(packages);
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

    /// Insert or replace normal files into the user workspace.
    /// The `file_path` is a path relative to the workspace `source_dir`.
    pub fn set_file(&mut self, file_path: &str, bytes: Vec<u8>) -> Result<(), Box<dyn Error>> {
        let path = self
            .root
            .join(file_path)
            .into_os_string()
            .into_string()
            .map_err(|os_str| EngineErr::PathError(os_str.to_string_lossy().to_string()))?;

        let id = Vfs::file_id(VirtualRoot::Project, &path)?;

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
        let path = self
            .root
            .join(file_path)
            .into_os_string()
            .into_string()
            .map_err(|os_str| EngineErr::PathError(os_str.to_string_lossy().to_string()))?;

        let id = Vfs::file_id(VirtualRoot::Project, &path)?;

        // only process text files.
        let mut src = self.sources.lock().unwrap();
        src.remove(&id);
        self.vfs.remove(id);

        Ok(())
    }

    pub fn set_entry(&mut self, entry: &str) -> Result<(), Box<dyn Error>> {
        let path = self
            .root
            .join(entry)
            .into_os_string()
            .into_string()
            .map_err(|os_str| EngineErr::PathError(os_str.to_string_lossy().to_string()))?;

        self.main_file = Vfs::file_id(VirtualRoot::Project, &path).unwrap();

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
        F: Fn(&PagedDocument) -> R,
    {
        let doc = self.document.lock().unwrap();
        doc.as_ref().map(func)
    }

    /// Resolves a virtual file ID to a real path on the VFS.
    #[allow(unused)]
    fn id_to_path(&self, id: FileId) -> FileResult<PathBuf> {
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

fn embedded_fonts() -> (FontBook, Vec<Font>) {
    let mut book = FontBook::new();
    let mut fonts = Vec::new();

    for data in typst_assets::fonts() {
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
