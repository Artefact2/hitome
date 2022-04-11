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
use page_size;
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

impl<'a> MemoryStats<'a> {
    pub fn new(s: &'a Settings) -> MemoryStats {
        let z = Threshold {
            val: Bytes(0),
            med: Bytes(1),
            high: Bytes(1),
            crit: Bytes(1),
            smart: s.smart,
        };
        MemoryStats {
            settings: s,
            pagesize: page_size::get() as u64,
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

    pub fn update(&mut self) {
        let s = &mut self.state;
        s.swap.0 = 0;
        s.zram.0 = 0;

        if let Ok(_) = read_to_string("/proc/swaps", &mut self.buf) {
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
            if let Ok(_) = read_to_string(mm, &mut self.buf) {
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

        match read_to_string("/proc/vmstat", &mut self.buf) {
            Ok(_) => (),
            _ => return,
        };

        s.active.0 = 0;
        s.inactive.0 = 0;
        s.cached.0 = 0;

        for line in self.buf.lines() {
            let mut iter = line.split_ascii_whitespace();
            let k = iter.next().unwrap();
            /* XXX: inefficient, we don't always need the parsed
             * value, but it makes for more readable code */
            let v = iter.next().unwrap().parse::<u64>().unwrap() * self.pagesize;
            match k {
                "nr_active_anon" => s.active.0 += v,
                "nr_active_file" => {
                    s.active.0 += v;
                    s.cached.0 += v
                }
                "nr_inactive_anon" => s.inactive.0 += v,
                "nr_inactive_file" => {
                    s.inactive.0 += v;
                    s.cached.0 += v
                }
                "nr_slab_unreclaimable" => s.cached.0 += v,
                "nr_slab_reclaimable" => s.cached.0 += v,
                "nr_kernel_misc_reclaimable" => s.cached.0 += v,
                "nr_swapcached" => {
                    s.cached.0 += v;
                    /* Swap is already filled, should be ok to substract without wrapping around */
                    s.swap.0 -= v;
                }
                "nr_free_pages" => s.free.0 = v,
                "nr_dirty" => s.dirty.val.0 = v,
                "nr_dirty_threshold" => s.dirty.crit.0 = v,
                "nr_dirty_background_threshold" => {
                    s.dirty.med.0 = v;
                    s.dirty.high.0 = v
                }
                "nr_writeback" => s.writeback.val.0 = v,
                _ => continue,
            };
        }
    }
}

impl<'a> fmt::Display for MemoryStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let newline = newline(self.settings.smart);
        let (hdrbegin, hdrend) = headings(self.settings.smart);
        let w = self.settings.colwidth;
        let s = &self.state;
        write!(f,
               "{}{:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$}{}{}{:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$}{}{}",
               hdrbegin, "ACTIVE", "INACTIVE", "CACHED", "FREE", "DIRTY", "W_BACK", "SWAP" ,"ZRAM", hdrend, newline,
               s.active, s.inactive, s.cached, s.free, s.dirty, s.writeback, s.swap, s.zram, newline, newline
        )
    }
}
