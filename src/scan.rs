//! Parallel directory scanner.
//!
//! Walks a folder tree with rayon, then flattens the result into an arena
//! (`Tree`) where every node knows its parent so sizes can be patched after a
//! delete. Reparse-point directories (junctions, symlinks, mounted volumes) are
//! listed but never followed, so nothing is counted twice and loops can't happen.

use rayon::prelude::*;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const ATTR_SPARSE: u32 = 0x200;
const ATTR_REPARSE_POINT: u32 = 0x400;
const ATTR_COMPRESSED: u32 = 0x800;
const ATTR_OFFLINE: u32 = 0x1000;
const ATTR_RECALL_ON_OPEN: u32 = 0x40000;
const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x400000;
const CLUSTER: u64 = 4096;

pub const NONE: u32 = u32::MAX;

#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub cancel: AtomicBool,
    pub current: Mutex<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    /// Junction / symlink / mount point: shown, never followed.
    Link,
}

pub struct Node {
    pub name: Box<str>,
    pub parent: u32,
    pub children: Vec<u32>,
    /// Logical size in bytes.
    pub size: u64,
    /// Estimated space taken on this disk (cloud-only files count as 0).
    pub disk: u64,
    pub files: u64,
    pub dirs: u64,
    /// Last write time, Windows FILETIME (100 ns since 1601). Newest inside for folders.
    pub modified: u64,
    pub kind: Kind,
    /// Some entries inside could not be read (usually access denied).
    pub denied: bool,
    pub removed: bool,
}

pub struct Tree {
    pub root_path: PathBuf,
    pub nodes: Vec<Node>,
}

impl Tree {
    pub fn path_of(&self, mut id: u32) -> PathBuf {
        let mut parts = Vec::new();
        while self.nodes[id as usize].parent != NONE {
            parts.push(id);
            id = self.nodes[id as usize].parent;
        }
        let mut p = self.root_path.clone();
        for &i in parts.iter().rev() {
            p.push(&*self.nodes[i as usize].name);
        }
        p
    }

    /// Detach a node after it was deleted and subtract its totals from every ancestor.
    pub fn remove(&mut self, id: u32) {
        let (size, disk, files, dirs, kind) = {
            let n = &self.nodes[id as usize];
            (n.size, n.disk, n.files, n.dirs, n.kind)
        };
        let self_dirs = if kind == Kind::Dir { 1 } else { 0 };
        let self_files = if kind == Kind::File { 1 } else { 0 };
        self.nodes[id as usize].removed = true;
        let parent = self.nodes[id as usize].parent;
        if parent != NONE {
            self.nodes[parent as usize].children.retain(|&c| c != id);
        }
        let mut p = parent;
        while p != NONE {
            let n = &mut self.nodes[p as usize];
            n.size -= size;
            n.disk -= disk;
            n.files -= files + self_files;
            n.dirs -= dirs + self_dirs;
            p = n.parent;
        }
    }

    /// The `n` largest files anywhere in the tree, biggest first.
    pub fn largest_files(&self, n: usize) -> Vec<u32> {
        let mut v: Vec<u32> = (0..self.nodes.len() as u32)
            .filter(|&i| {
                let x = &self.nodes[i as usize];
                x.kind == Kind::File && !x.removed
            })
            .collect();
        if v.len() > n {
            v.select_nth_unstable_by(n, |a, b| {
                self.nodes[*b as usize].size.cmp(&self.nodes[*a as usize].size)
            });
            v.truncate(n);
        }
        v.sort_by(|a, b| self.nodes[*b as usize].size.cmp(&self.nodes[*a as usize].size));
        v
    }
}

/// Temporary owned tree built by the parallel walk.
struct Built {
    name: Box<str>,
    size: u64,
    disk: u64,
    files: u64,
    dirs: u64,
    modified: u64,
    kind: Kind,
    denied: bool,
    children: Vec<Built>,
}

pub fn scan(root: &Path, progress: Arc<Progress>) -> Tree {
    let root_name: Box<str> = root.to_string_lossy().into();
    let built = walk(root, root_name, &progress);
    let mut nodes = Vec::with_capacity((built.files + built.dirs + 1) as usize);
    flatten(built, NONE, &mut nodes);
    Tree { root_path: root.to_path_buf(), nodes }
}

fn walk(dir: &Path, name: Box<str>, progress: &Progress) -> Built {
    let mut me = Built {
        name,
        size: 0,
        disk: 0,
        files: 0,
        dirs: 0,
        modified: 0,
        kind: Kind::Dir,
        denied: false,
        children: Vec::new(),
    };
    if progress.cancel.load(Ordering::Relaxed) {
        return me;
    }
    progress.dirs.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut cur) = progress.current.try_lock() {
        cur.clear();
        cur.push_str(&dir.to_string_lossy());
    }

    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => {
            me.denied = true;
            progress.errors.fetch_add(1, Ordering::Relaxed);
            return me;
        }
    };

    let mut subdirs: Vec<(PathBuf, Box<str>)> = Vec::new();
    let mut local_files = 0u64;
    let mut local_bytes = 0u64;
    for entry in rd {
        let Ok(entry) = entry else {
            me.denied = true;
            continue;
        };
        // On Windows this metadata comes straight from the directory listing: no extra syscall.
        let Ok(md) = entry.metadata() else {
            me.denied = true;
            continue;
        };
        let attrs = md.file_attributes();
        let fname: Box<str> = entry.file_name().to_string_lossy().into();
        let modified = md.last_write_time();
        if md.is_dir() {
            if attrs & ATTR_REPARSE_POINT != 0 {
                me.children.push(Built {
                    name: fname,
                    size: 0,
                    disk: 0,
                    files: 0,
                    dirs: 0,
                    modified,
                    kind: Kind::Link,
                    denied: false,
                    children: Vec::new(),
                });
            } else {
                subdirs.push((entry.path(), fname));
            }
        } else {
            let size = md.file_size();
            let cloud_only =
                attrs & (ATTR_OFFLINE | ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS) != 0;
            let disk = if cloud_only {
                0
            } else if attrs & (ATTR_COMPRESSED | ATTR_SPARSE) != 0 {
                // NTFS-compressed (e.g. CompactOS) and sparse files take less than their size.
                allocated_size(&entry.path()).unwrap_or(size)
            } else {
                size.div_ceil(CLUSTER) * CLUSTER
            };
            local_files += 1;
            local_bytes += size;
            me.size += size;
            me.disk += disk;
            me.files += 1;
            me.modified = me.modified.max(modified);
            me.children.push(Built {
                name: fname,
                size,
                disk,
                files: 0,
                dirs: 0,
                modified,
                kind: Kind::File,
                denied: false,
                children: Vec::new(),
            });
        }
    }
    progress.files.fetch_add(local_files, Ordering::Relaxed);
    progress.bytes.fetch_add(local_bytes, Ordering::Relaxed);

    let sub: Vec<Built> = subdirs
        .into_par_iter()
        .map(|(p, n)| walk(&p, n, progress))
        .collect();
    for s in sub {
        me.size += s.size;
        me.disk += s.disk;
        me.files += s.files;
        me.dirs += s.dirs + 1;
        me.modified = me.modified.max(s.modified);
        me.children.push(s);
    }
    me
}

fn allocated_size(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetCompressedFileSizeW, INVALID_FILE_SIZE};
    let w: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut high = 0u32;
    let low = unsafe { GetCompressedFileSizeW(w.as_ptr(), &mut high) };
    if low == INVALID_FILE_SIZE && std::io::Error::last_os_error().raw_os_error() != Some(0) {
        return None;
    }
    Some(((high as u64) << 32) | low as u64)
}

fn flatten(b: Built, parent: u32, nodes: &mut Vec<Node>) -> u32 {
    let id = nodes.len() as u32;
    nodes.push(Node {
        name: b.name,
        parent,
        children: Vec::new(),
        size: b.size,
        disk: b.disk,
        files: b.files,
        dirs: b.dirs,
        modified: b.modified,
        kind: b.kind,
        denied: b.denied,
        removed: false,
    });
    let mut kids = b.children;
    kids.sort_unstable_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    let mut ids = Vec::with_capacity(kids.len());
    for k in kids {
        ids.push(flatten(k, id, nodes));
    }
    nodes[id as usize].children = ids;
    id
}
