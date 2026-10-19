//! Helper to unpack a Typst package into the VFS.

use flate2::read::GzDecoder;
use serde::Serialize;
use std::error::Error;
use std::io;
use tar::Archive;
use tsify::Tsify;
use typst::ecow::EcoString;
use typst_syntax::VirtualRoot;
use typst_syntax::package::PackageSpec;

use crate::errors::EngineErr;
use crate::vfs::Vfs;

/// A package name and description registry used by [IdeWorld] to generate
/// imports completion hints.
pub(crate) struct PackageRegistry {
    // Owned storage so `IdeWorld::packages` can return a borrow into it.
    // Package lists are small, so linear scans are fine.
    packages: Vec<(PackageSpec, Option<EcoString>)>,
}

impl PackageRegistry {
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
        }
    }

    pub fn add_many(&mut self, packages: Vec<(PackageSpec, Option<String>)>) {
        for (spec, desc) in packages {
            self.add(spec, desc.as_ref().cloned());
        }
    }

    pub fn add(&mut self, spec: PackageSpec, desc: Option<String>) {
        let desc = desc.map(EcoString::from);
        match self.packages.iter_mut().find(|(s, _)| s == &spec) {
            Some(entry) => entry.1 = desc,
            None => self.packages.push((spec, desc)),
        }
    }

    // pub fn remove(&mut self, spec: &PackageSpec) {
    //     self.packages.retain(|(s, _)| s != spec);
    // }

    pub fn clear(&mut self) {
        self.packages.clear();
    }

    pub fn all(&self) -> &[(PackageSpec, Option<EcoString>)] {
        &self.packages
    }
}

#[derive(Debug, Serialize, Tsify, Clone, PartialEq, Eq)]
pub struct UnpackSummary {
    pub files: i32,
    pub bytes: u64,
}

pub(crate) fn unpacked_package(
    vfs: &mut Vfs,
    spec: &PackageSpec,
    tgz_bytes: &[u8],
    max_total: u64,
) -> Result<UnpackSummary, Box<dyn Error>> {
    let gz = GzDecoder::new(tgz_bytes);
    let mut ar = Archive::new(gz);
    let mut total: u64 = 0;
    let mut count = 0;

    let entries = ar.entries()?;
    for entry in entries {
        let mut e = entry?;

        let path = e.path()?.into_owned();
        // validate relative path
        // TODO

        if !e.header().entry_type().is_file() {
            continue;
        }

        total += e.header().size().unwrap_or(0);
        if total > max_total {
            return Err(Box::new(EngineErr::UnpackError));
        }

        let mut bytes = Vec::with_capacity(e.size() as usize);
        io::copy(&mut e, &mut bytes)?;

        let file_path = path.as_os_str().to_str().ok_or("invalid path in tar")?;
        let id = Vfs::file_id(VirtualRoot::Package(spec.clone()), file_path)?;
        vfs.set(id, bytes);
        count += 1;
    }

    Ok(UnpackSummary {
        files: count,
        bytes: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    use std::str::FromStr;
    use typst_syntax::VirtualRoot;

    fn package_spec() -> PackageSpec {
        PackageSpec::from_str("@preview/demo:1.2.3").unwrap()
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
    fn test_package_registry_add_updates_existing_entry() {
        let mut registry = PackageRegistry::new();
        let spec = package_spec();

        assert!(registry.all().is_empty());

        registry.add(spec.clone(), Some("first".into()));
        assert_eq!(registry.all().len(), 1);
        assert_eq!(registry.all()[0].1.as_deref(), Some("first"));

        // Adding the same spec again replaces the description instead of
        // creating a duplicate entry.
        registry.add(spec.clone(), Some("second".into()));
        assert_eq!(registry.all().len(), 1);
        assert_eq!(registry.all()[0].1.as_deref(), Some("second"));

        registry.add(spec, None);
        assert_eq!(registry.all().len(), 1);
        assert_eq!(registry.all()[0].1, None);
    }

    #[test]
    fn test_package_registry_add_many_and_remove() {
        let mut registry = PackageRegistry::new();
        let first = package_spec();
        let second = PackageSpec::from_str("@preview/other:0.1.0").unwrap();

        registry.add_many(vec![
            (first.clone(), Some("first package".into())),
            (second.clone(), None),
        ]);

        assert_eq!(registry.all().len(), 2);
        assert_eq!(registry.all()[0].1.as_deref(), Some("first package"));
        assert_eq!(registry.all()[1].0, second);
    }

    #[test]
    fn test_unpack_package_populates_vfs() {
        let entries: &[(&str, &[u8])] = &[
            ("typst.toml", b"[package]\nname = \"demo\""),
            ("src/lib.typ", b"#let answer = 42"),
        ];
        let tgz = make_tgz(entries);
        let spec = package_spec();
        let mut vfs = Vfs::new();

        let summary = unpacked_package(&mut vfs, &spec, &tgz, 1024 * 1024).expect("unpack failed");

        let expected_bytes = entries.iter().map(|(_, data)| data.len() as u64).sum();
        assert_eq!(
            summary,
            UnpackSummary {
                files: entries.len() as i32,
                bytes: expected_bytes,
            }
        );
        assert_eq!(vfs.size(), entries.len());

        let id = Vfs::file_id(VirtualRoot::Package(spec), "src/lib.typ").unwrap();
        assert_eq!(
            vfs.lookup(id).unwrap().as_ref(),
            b"#let answer = 42".as_slice()
        );
    }

    #[test]
    fn test_unpack_package_rejects_oversized_total() {
        let tgz = make_tgz(&[("a.typ", b"12345"), ("b.typ", b"67890")]);
        let mut vfs = Vfs::new();

        let err = unpacked_package(&mut vfs, &package_spec(), &tgz, 9)
            .expect_err("unpack should exceed the byte limit");

        assert_eq!(err.to_string(), "unpack package error");
    }
}
