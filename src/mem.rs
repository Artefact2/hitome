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

use super::common::*;
use std::fmt;

struct Memory {
    active: Bytes,
    inactive: Bytes,
    cached: Bytes,
    free: Bytes,
    dirty: Threshold<Bytes>,
    writeback: Threshold<Bytes>,
    swap: Bytes,
    zram: Bytes,
}

pub struct MemoryStats<'a> {
    settings: &'a Settings,
    pagesize: u64,
    state: Memory,
    buf: String,
}

impl<'a> StatBlock<'a> for MemoryStats<'a> {
    fn new(s: &'a Settings) -> MemoryStats {
        let z = Threshold {
            val: Bytes(0),
            med: Bytes(1),
            high: Bytes(1),
            crit: Bytes(1),
        };
        MemoryStats {
            settings: s,
            pagesize: unsafe { libc::sysconf(libc::_SC_PAGE_SIZE) } as u64,
            state: Memory {
                active: Bytes(0),
                inactive: Bytes(0),
                cached: Bytes(0),
                free: Bytes(0),
                dirty: z,
                writeback: z,
                swap: Bytes(0),
                zram: Bytes(0),
            },
            buf: String::new(),
        }
    }

    fn update(&mut self) {
        let s = &mut self.state;
        s.swap.0 = 0;
        s.zram.0 = 0;

        /* /proc/swaps doesn't contain arbitrary user data */
        if unsafe { read_to_string_unchecked("/proc/swaps", &mut self.buf) }.is_ok() {
            for line in self.buf.lines().skip(1) {
                s.swap.0 += line
                    .split_ascii_whitespace()
                    .nth(3)
                    .unwrap()
                    .parse::<u64>()
                    .unwrap()
                    * 1024;
            }
        }

        for bdev in std::fs::read_dir("/sys/block").unwrap() {
            let bdev = match bdev {
                Ok(s) => s,
                _ => continue,
            };

            if bdev
                .file_name()
                .to_str()
                .map_or(true, |s| !s.starts_with("zram"))
            {
                continue;
            }

            let mut mm = bdev.path();
            mm.push("mm_stat");
            /* /sys/block/zramN/mm_stat only contains space separated numeric fields */
            if unsafe { read_to_string_unchecked(mm, &mut self.buf) }.is_ok() {
                /* https://docs.kernel.org/admin-guide/blockdev/zram.html */
                s.zram.0 += self
                    .buf
                    .split_ascii_whitespace()
                    .nth(2)
                    .unwrap()
                    .parse::<u64>()
                    .unwrap();
            }
        }

        /* No arbitrary strings in /proc/vmstat */
        match unsafe { read_to_string_unchecked("/proc/vmstat", &mut self.buf) } {
            Ok(_) => (),
            _ => return,
        };

        s.active.0 = 0;
        s.inactive.0 = 0;
        s.cached.0 = 0;

        for line in self.buf.lines() {
            let mut iter = line.split_ascii_whitespace();
            let k = iter.next().unwrap();
            let mut val = || iter.next().unwrap().parse::<u64>().unwrap() * self.pagesize;
            match k {
                "nr_active_anon" => s.active.0 += val(),
                "nr_active_file" => {
                    let v = val();
                    s.active.0 += v;
                    s.cached.0 += v
                }
                "nr_inactive_anon" => s.inactive.0 += val(),
                "nr_inactive_file" => {
                    let v = val();
                    s.inactive.0 += v;
                    s.cached.0 += v
                }
                "nr_slab_unreclaimable" => s.cached.0 += val(),
                "nr_slab_reclaimable" => s.cached.0 += val(),
                "nr_kernel_misc_reclaimable" => s.cached.0 += val(),
                "nr_swapcached" => {
                    let v = val();
                    s.cached.0 += v;
                    /* Swap is already filled, should be ok to substract without wrapping around */
                    s.swap.0 -= v;
                }
                "nr_free_pages" => s.free.0 = val(),
                "nr_dirty" => s.dirty.val.0 = val(),
                "nr_dirty_threshold" => s.dirty.crit.0 = val(),
                "nr_dirty_background_threshold" => {
                    let v = val();
                    s.dirty.med.0 = v;
                    s.dirty.high.0 = v
                }
                "nr_writeback" => s.writeback.val.0 = val(),
                _ => continue,
            };
        }
    }
}

impl<'a> fmt::Display for MemoryStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let w = self.settings.colwidth.into();
        let s = &self.state;
        let se = &self.settings;
        let newline = MaybeSmart(Newline(), se);
        write!(
            f,
            "{} {} {} {} {} {} {} {}{}{:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$}{}{}",
            MaybeSmart(Heading("ACTIVE"), se),
            MaybeSmart(Heading("INACTIVE"), se),
            MaybeSmart(Heading("CACHED"), se),
            MaybeSmart(Heading("FREE"), se),
            MaybeSmart(Heading("DIRTY"), se),
            MaybeSmart(Heading("W_BACK"), se),
            MaybeSmart(Heading("SWAP"), se),
            MaybeSmart(Heading("ZRAM"), se),
            newline,
            s.active,
            s.inactive,
            s.cached,
            s.free,
            MaybeSmart(s.dirty, self.settings),
            MaybeSmart(s.writeback, self.settings),
            s.swap,
            s.zram,
            newline,
            newline
        )
    }
}
