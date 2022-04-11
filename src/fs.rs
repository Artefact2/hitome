/* Copyright 2022 Romain "Artefact2" Dal Maso <romain.dalmaso@artefact2.com>
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *	   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use crate::common::*;
use std::collections::{BTreeMap, HashSet};
use std::ffi::CString;
use std::fmt;

struct FSUsage {
    size: Bytes,
    avail: Bytes,
}

pub struct FilesystemStats<'a> {
    settings: &'a Settings,
    filesystems: BTreeMap<String, (FSUsage, CString, Stale)>,
    buf: String,
}

impl<'a> StatBlock<'a> for FilesystemStats<'a> {
    fn new(s: &'a Settings) -> FilesystemStats {
        FilesystemStats {
            settings: s,
            filesystems: BTreeMap::new(),
            buf: String::new(),
        }
    }

    fn update(&mut self) {
        match read_to_string("/proc/self/mountstats", &mut self.buf) {
            Ok(_) => (),
            _ => return,
        }

        /* XXX: keep instance in self and blank it when we're done? don't know how to work around
         * lifetime stuff */
        let mut seen: HashSet<&str> = HashSet::new();

        for v in self.filesystems.values_mut() {
            v.2 = Stale(true);
        }

        let mut vfs: std::mem::MaybeUninit<libc::statvfs64> = std::mem::MaybeUninit::uninit();

        for mount in self.buf.lines() {
            let (bdev, mountpoint) = mount
                .strip_prefix("device ")
                .unwrap()
                .split_once(" mounted on ")
                .unwrap();

            if !bdev.starts_with("/") {
                /* Not interested in these kind of mounts */
                continue;
            }

            if seen.contains(bdev) {
                /* Another fs on the same block device, could be eg bind mount or btrfs subvolume...
                 * skip them */
                /* XXX: are there any edge cases? */
                continue;
            }
            seen.insert(bdev);

            let (mountpoint, _) = mountpoint.rsplit_once(" with fstype ").unwrap();

            let mut ent = match self.filesystems.get_mut(mountpoint) {
                Some(v) => v,
                _ => {
                    self.filesystems.insert(
                        String::from(mountpoint),
                        (
                            FSUsage {
                                size: Bytes(0),
                                avail: Bytes(0),
                            },
                            CString::new(mountpoint).unwrap(),
                            Stale(false),
                        ),
                    );
                    self.filesystems.get_mut(mountpoint).unwrap()
                }
            };

            unsafe {
                if libc::statvfs64(ent.1.as_ptr() as *const libc::c_char, vfs.as_mut_ptr()) != 0 {
                    panic!("statvfs64({}) returned non-zero", mountpoint);
                }

                let vfs = vfs.assume_init();
                ent.0.size.0 = vfs.f_blocks * vfs.f_frsize;
                ent.0.avail.0 = vfs.f_bavail * vfs.f_bsize;
            }

            ent.2 = Stale(false);
        }

        self.filesystems.retain(|_, v| v.2 == Stale(false))
    }
}

impl<'a> fmt::Display for FilesystemStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.filesystems.is_empty() {
            return write!(f, "");
        }

        let newline = newline(self.settings.smart);
        let (hdrb, hdre) = headings(self.settings.smart);
        let w = self.settings.colwidth;
        write!(
            f,
            "{}{:>w$} {:>w$} {:>w$} {:>w$}{}{}",
            hdrb, "FS", "USED%", "USED", "AVAIL", hdre, newline
        )?;

        for (k, v) in self.filesystems.iter() {
            write!(
                f,
                "{:>w$.w$} {:>w$} {:>w$} {:>w$}{}",
                if k == "/" {
                    k
                } else {
                    k.rsplit_once('/').unwrap().1
                },
                Percentage(100.0 * ((v.0.size.0 - v.0.avail.0) as f32) / (v.0.size.0 as f32)),
                Bytes(v.0.size.0 - v.0.avail.0),
                v.0.avail,
                newline
            )?;
        }

        write!(f, "{}", newline)
    }
}
