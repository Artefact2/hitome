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

pub struct MemoryStats<'a> {
    settings: &'a Settings,
    pagesize: u64,
}

impl<'a> MemoryStats<'a> {
    pub fn new(s: &'a Settings) -> MemoryStats {
        MemoryStats {
            settings: s,
            pagesize: page_size::get() as u64,
        }
    }
}

impl<'a> fmt::Display for MemoryStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut active = Bytes(0);
        let mut inactive = Bytes(0);
        let mut cached = Bytes(0);
        let mut free = Bytes(0);
        let mut dirty = Threshold {
            val: Bytes(0),
            med: Bytes(0),
            high: Bytes(0),
            crit: Bytes(0),
            smart: self.settings.smart,
        };
        let mut writeback = Threshold {
            val: Bytes(0),
            med: Bytes(1),
            high: Bytes(1),
            crit: Bytes(1),
            smart: self.settings.smart,
        };

        /* XXX: parse /proc/swaps and /sys/block/zramX/mm_stat */
        let mut swap = Bytes(0);
        let mut zram = Bytes(0);

        let vmstat = match std::fs::read_to_string("/proc/vmstat") {
            Ok(s) => s,
            _ => return write!(f, ""),
        };

        if let Ok(swaps) = std::fs::read_to_string("/proc/swaps") {
            for line in swaps.lines().skip(1) {
                swap.0 += line
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
            if let Ok(mm) = std::fs::read_to_string(mm) {
                /* https://docs.kernel.org/admin-guide/blockdev/zram.html */
                zram.0 += mm
                    .split_ascii_whitespace()
                    .nth(2)
                    .unwrap()
                    .parse::<u64>()
                    .unwrap();
            }
        }

        for line in vmstat.lines() {
            let mut iter = line.split_ascii_whitespace();
            let k = iter.next().unwrap();
            /* XXX: inefficient, we don't always need the parsed
             * value, but it makes for more readable code */
            let v = iter.next().unwrap().parse::<u64>().unwrap() * self.pagesize;
            match k {
                "nr_active_anon" => active.0 += v,
                "nr_active_file" => {
                    active.0 += v;
                    cached.0 += v
                }
                "nr_inactive_anon" => inactive.0 += v,
                "nr_inactive_file" => {
                    inactive.0 += v;
                    cached.0 += v
                }
                "nr_slab_unreclaimable" => cached.0 += v,
                "nr_slab_reclaimable" => cached.0 += v,
                "nr_kernel_misc_reclaimable" => cached.0 += v,
                "nr_swapcached" => {
                    cached.0 += v;
                    /* Swap is already filled, should be ok to substract without wrapping around */
                    swap.0 -= v;
                }
                "nr_free_pages" => free.0 = v,
                "nr_dirty" => dirty.val.0 = v,
                "nr_dirty_threshold" => dirty.crit.0 = v,
                "nr_dirty_background_threshold" => {
                    dirty.med.0 = v;
                    dirty.high.0 = v
                }
                "nr_writeback" => writeback.val.0 = v,
                _ => continue,
            };
        }

        let newline = newline(self.settings.smart);
        let (hdrbegin, hdrend) = headings(self.settings.smart);
        let w = self.settings.colwidth;
        write!(f,
               "{}{:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$}{}{}{:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$}{}{}",
               hdrbegin, "ACTIVE", "INACTIVE", "CACHED", "FREE", "DIRTY", "W_BACK", "SWAP" ,"ZRAM", hdrend, newline,
               active, inactive, cached, free, dirty, writeback, swap, zram, newline, newline
        )
    }
}
