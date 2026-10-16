//! Helper to unpack a Typst package into the VFS.

use flate2::read::GzDecoder;
use serde::Serialize;
use std::error::Error;
use std::io;
use std::path::Path;
use tar::Archive;
use tsify::Tsify;
use typst::ecow::EcoString;
use typst_syntax::VirtualRoot;
use typst_syntax::package::PackageSpec;

use crate::errors::EngineErr;
use crate::vfs::Vfs;

/// A package name and description registry used by [IdeWorld] to generate
/// imports completion items.
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

    pub fn add_many(&mut self, packages: &[(PackageSpec, Option<String>)]) {
        for (spec, desc) in packages {
            self.add(spec, desc.as_ref().cloned());
        }
    }

    pub fn add(&mut self, spec: &PackageSpec, desc: Option<String>) {
        let desc = desc.map(EcoString::from);
        match self.packages.iter_mut().find(|(s, _)| s == spec) {
            Some(entry) => entry.1 = desc,
            None => self.packages.push((spec.clone(), desc)),
        }
    }

    pub fn remove(&mut self, spec: &PackageSpec) {
        self.packages.retain(|(s, _)| s != spec);
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
    packages_dir: &Path,
    spec: &PackageSpec,
    tgz_bytes: &[u8],
    max_total: u64,
) -> Result<UnpackSummary, Box<dyn Error>> {
    let gz = GzDecoder::new(tgz_bytes);
    let mut ar = Archive::new(gz);
    let mut total: u64 = 0;
    let mut count = 0;

    let pkg_root = packages_dir
        .join(spec.namespace.to_string())
        .join(spec.name.to_string())
        .join(spec.version.to_string());

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

        let file_path = pkg_root.join(path).into_os_string().into_string();
        if let Ok(path) = file_path {
            let id = Vfs::file_id(VirtualRoot::Package(spec.clone()), &path)?;
            vfs.set(id, bytes);
        }

        count += 1;
    }

    Ok(UnpackSummary {
        files: count,
        bytes: total,
    })
}
