use bytes::Bytes;
use std::collections::HashMap;
use typst::diag::FileError;
use typst_library::diag::FileResult;
use typst_syntax::{FileId, RootedPath, VirtualPath, VirtualRoot};

use crate::errors::EngineErr;

/// Implement an VFS backing the compiler.
#[derive(Default)]
pub struct Vfs {
    files: HashMap<FileId, Bytes>,
}

#[allow(unused)]
impl Vfs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, id: FileId, bytes: Vec<u8>) {
        self.files.insert(id, Bytes::from(bytes));
    }

    pub fn remove(&mut self, id: FileId) {
        self.files.remove(&id);
    }

    pub fn remove_by_prefix(&mut self, prefix: &str) {
        self.files
            .retain(|key, _| key.vpath().get_with_slash() != prefix);
    }

    pub fn lookup(&self, id: FileId) -> FileResult<Bytes> {
        self.files
            .get(&id)
            .cloned()
            .ok_or_else(|| FileError::NotFound(id.vpath().get_with_slash().into()))
    }

    pub fn all(&self) -> impl Iterator<Item = FileId> {
        self.files.keys().copied()
    }

    pub fn size(&self) -> usize {
        self.files.len()
    }

    pub fn file_id(root: VirtualRoot, path: &str) -> Result<FileId, EngineErr> {
        let rooted_path = RootedPath::new(
            root,
            VirtualPath::new(path).map_err(|err| EngineErr::PathError(err.to_string()))?,
        );

        Ok(FileId::new(rooted_path))
    }
}

#[cfg(test)]
mod tests {
    use crate::vfs::Vfs;
    use typst_syntax::VirtualRoot::Project;

    #[test]
    fn test_vfs_set() {
        let mut vfs = Vfs::new();

        vfs.set(Vfs::file_id(Project, "main.typ").unwrap(), Vec::new());

        assert_eq!(vfs.size(), 1, "Add file to vfs failed")
    }

    #[test]
    fn test_vfs_remove() {
        let mut vfs = Vfs::new();

        let id = Vfs::file_id(Project, "main.typ").unwrap();
        vfs.set(id, Vec::new());

        vfs.remove(id);

        assert_eq!(vfs.size(), 0, "Remove file from vfs failed")
    }

    #[test]
    fn test_vfs_lookup() {
        let mut vfs = Vfs::new();

        let id = Vfs::file_id(Project, "main.typ").unwrap();
        let content = String::from("hello, world!");

        vfs.set(id, content.into_bytes());

        match vfs.lookup(id) {
            Ok(rv) => {
                assert_eq!(String::from_utf8(rv.to_vec()).unwrap(), "hello, world!");
            }

            Err(err) => {
                panic!("lookup vfs failed: {}", err);
            }
        }
    }
}
