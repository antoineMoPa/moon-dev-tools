//! The files a process and everything it started hold open - which is how a Codex terminal is
//! tied to its rollout.
//!
//! Codex keeps the rollout of the thread it is running open for writing for as long as the
//! thread is the one in front, and nothing else does. So the rollout a terminal's Codex is
//! writing is the `rollout-*.jsonl` open in the process on its pty - or in one it started,
//! for a `codex` that is a launcher for the real binary.

use std::path::PathBuf;

/// How far down a process tree the open files are looked for. A launcher and the binary it
/// runs is two levels; the rest are tools and servers the agent started.
const DESCENDANT_DEPTH: usize = 3;

/// Every file open in this process and its descendants, each once.
pub(crate) fn open_in_process_tree(root: u32) -> Vec<PathBuf> {
    let mut pids = vec![root];
    let mut generation = vec![root];
    for _ in 0..DESCENDANT_DEPTH {
        generation = generation.into_iter().flat_map(child_pids).collect();
        pids.extend(&generation);
    }
    let mut paths: Vec<PathBuf> = pids.into_iter().flat_map(open_files).collect();
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(target_os = "macos")]
fn child_pids(pid: u32) -> Vec<u32> {
    const MAX_CHILDREN: usize = 256;
    let mut buffer = vec![0 as libc::pid_t; MAX_CHILDREN];
    // SAFETY: the buffer is as large as the size handed in, and the call writes at most that.
    let count = unsafe {
        libc::proc_listchildpids(
            pid as libc::pid_t,
            buffer.as_mut_ptr().cast(),
            (MAX_CHILDREN * std::mem::size_of::<libc::pid_t>()) as libc::c_int,
        )
    };
    // A process that has ended since it was listed has no children to give.
    buffer.truncate(usize::try_from(count).unwrap_or(0));
    buffer.into_iter().map(|pid| pid as u32).collect()
}

/// `struct vnode_fdinfowithpath` of `sys/proc_info.h`, which libc does not define: a
/// `proc_fileinfo` of 24 bytes, a `vnode_info` of 152, then the path, `MAXPATHLEN` long.
#[cfg(target_os = "macos")]
const VNODE_FDINFO_WITH_PATH_SIZE: usize = 24 + 152 + 1024;
#[cfg(target_os = "macos")]
const VNODE_PATH_OFFSET: usize = 24 + 152;
#[cfg(target_os = "macos")]
const PROC_PIDFDVNODEPATHINFO: libc::c_int = 2;

#[cfg(target_os = "macos")]
fn open_files(pid: u32) -> Vec<PathBuf> {
    use std::{ffi::CStr, mem::size_of};

    const MAX_FDS: usize = 4096;
    let mut fds = vec![
        libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0,
        };
        MAX_FDS
    ];
    // SAFETY: the buffer is as large as the size handed in; the call answers in bytes written.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDLISTFDS,
            0,
            fds.as_mut_ptr().cast(),
            (MAX_FDS * size_of::<libc::proc_fdinfo>()) as libc::c_int,
        )
    };
    // Nothing for a process that has ended, or one this user may not look into.
    fds.truncate(usize::try_from(written).unwrap_or(0) / size_of::<libc::proc_fdinfo>());

    fds.into_iter()
        .filter(|fd| fd.proc_fdtype == libc::PROX_FDTYPE_VNODE as u32)
        .filter_map(|fd| {
            let mut info = [0u8; VNODE_FDINFO_WITH_PATH_SIZE];
            // SAFETY: the buffer is exactly the struct this flavor fills.
            let written = unsafe {
                libc::proc_pidfdinfo(
                    pid as libc::c_int,
                    fd.proc_fd,
                    PROC_PIDFDVNODEPATHINFO,
                    info.as_mut_ptr().cast(),
                    VNODE_FDINFO_WITH_PATH_SIZE as libc::c_int,
                )
            };
            // A file closed between the listing and this reads as nothing written.
            if written <= 0 {
                return None;
            }
            assert_eq!(
                written as usize, VNODE_FDINFO_WITH_PATH_SIZE,
                "the kernel's vnode_fdinfowithpath is not the size this was written against"
            );
            let path = CStr::from_bytes_until_nul(&info[VNODE_PATH_OFFSET..])
                .expect("a vnode path is nul-terminated within MAXPATHLEN");
            Some(PathBuf::from(path.to_string_lossy().into_owned()))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn child_pids(pid: u32) -> Vec<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .unwrap_or_default()
        .split_whitespace()
        .map(|child| child.parse().expect("/proc lists children as numbers"))
        .collect()
}

#[cfg(target_os = "linux")]
fn open_files(pid: u32) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn child_pids(_pid: u32) -> Vec<u32> {
    Vec::new()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn open_files(_pid: u32) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    fn a_file_this_process_holds_open_is_listed() {
        let path = std::env::temp_dir().join(format!(
            "moon-open-file-{}.jsonl",
            crate::moontasks::store::new_uuid()
        ));
        let _held = std::fs::File::create(&path).expect("create the file");

        let open = open_in_process_tree(std::process::id());

        assert!(
            open.contains(&std::fs::canonicalize(&path).unwrap()),
            "{open:?}"
        );
    }

    #[test]
    fn a_file_a_child_holds_open_is_listed() {
        let path = std::env::temp_dir().join(format!(
            "moon-child-file-{}.jsonl",
            crate::moontasks::store::new_uuid()
        ));
        std::fs::write(&path, "").expect("create the file");
        // A shell that holds the file open on fd 3 while it waits.
        let mut launcher = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("exec 3<'{}'; sleep 5", path.display()))
            .spawn()
            .expect("start a shell");
        std::thread::sleep(std::time::Duration::from_millis(300));

        let open = open_in_process_tree(std::process::id());
        launcher.kill().unwrap();
        launcher.wait().unwrap();

        assert!(
            open.contains(&std::fs::canonicalize(&path).unwrap()),
            "{open:?}"
        );
    }
}
