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
use std::collections::BTreeMap;
use std::fmt;
use std::time::Instant;

#[derive(Clone, Copy)]
struct DevStats {
    t: Instant,
    read: Bytes,
    written: Bytes,
    /// Weighed request time
    wrt: u64,
}

const SECTOR_SIZE: u64 = 512;

pub struct BlockDeviceStats<'a> {
    settings: &'a Settings,
    /* Use a BTreeMap to traverse in sorted order */
    devices: BTreeMap<String, (DevStats, DevStats, Stale)>,
    buf: String,
}

impl<'a> StatBlock<'a> for BlockDeviceStats<'a> {
    fn new(s: &'a Settings) -> BlockDeviceStats {
        let mut bdev = BlockDeviceStats {
            settings: s,
            devices: BTreeMap::new(),
            buf: String::new(),
        };
        bdev.update();
        bdev
    }

    fn update(&mut self) {
        match read_to_string("/proc/diskstats", &mut self.buf) {
            Ok(_) => (),
            _ => return,
        }

        let t = Instant::now();

        for bdev in self.devices.values_mut() {
            bdev.2 = Stale(true);
        }

        /* https://www.kernel.org/doc/Documentation/iostats.txt */
        for bdev in self.buf.lines() {
            let mut bdev = bdev.split_ascii_whitespace();
            let kname = bdev.nth(2).unwrap();

            /* XXX: make this user-configurable */
            if kname.starts_with("dm-") || kname.starts_with("loop") {
                continue;
            }

            /* Filter out sda1, sda2 etc if we have sda */
            /* XXX: assumes /proc/diskstats has some inherent sort order (partitions after) */
            /* XXX: will not work for bdevs with over 10 partitions */
            if ('0'..'9').contains(&kname.chars().rev().next().unwrap())
                && self.devices.contains_key(&kname[0..(kname.len() - 1)])
            {
                continue;
            }

            let mut ent = match self.devices.get_mut(kname) {
                Some(v) => v,
                _ => {
                    let z = DevStats {
                        t,
                        read: Bytes(0),
                        written: Bytes(0),
                        wrt: 0,
                    };
                    self.devices
                        .insert(String::from(kname), (z, z, Stale(false)));
                    self.devices.get_mut(kname).unwrap()
                }
            };

            ent.0 = ent.1;
            ent.1 = DevStats {
                t,
                read: Bytes(SECTOR_SIZE * bdev.nth(2).unwrap().parse::<u64>().unwrap()),
                written: Bytes(SECTOR_SIZE * bdev.nth(3).unwrap().parse::<u64>().unwrap()),
                wrt: bdev.nth(3).unwrap().parse::<u64>().unwrap(),
            };
            ent.2 = Stale(false);
        }

        self.devices.retain(|_, v| v.2 == Stale(false));
    }

    fn columns(&self) -> u16 {
        if self.devices.is_empty() {
            0
        } else {
            4 * self.settings.colwidth.get() + 3
        }
    }

    fn rows(&self) -> u16 {
        if self.devices.is_empty() {
            0
        } else {
            2 + self.devices.len() as u16
        }
    }
}

impl<'a> fmt::Display for BlockDeviceStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.devices.is_empty() {
            return Ok(());
        }

        let newline = MaybeSmart(Newline(), self.settings);
        let w = self.settings.colwidth.get().into();
        write!(
            f,
            "{} {} {} {}{}",
            MaybeSmart(Heading("DEVICE"), self.settings),
            MaybeSmart(Heading("READ/s"), self.settings),
            MaybeSmart(Heading("WRITE/s"), self.settings),
            MaybeSmart(Heading("PRESSURE"), self.settings),
            newline
        )?;

        for (kname, s) in self.devices.iter() {
            let t = (s.1.t - s.0.t).as_millis() as u64;
            if t == 0 {
                /* Device was just added */
                continue;
            }
            let rd = Bytes(1000 * (s.1.read.0 - s.0.read.0) / t);
            let wt = Bytes(1000 * (s.1.written.0 - s.0.written.0) / t);
            let p = Threshold {
                val: Percentage(100.0 * ((s.1.wrt - s.0.wrt) as f32) / (t as f32)),
                med: Percentage(50.0),
                high: Percentage(80.0),
                crit: Percentage(200.0),
            };
            write!(
                f,
                "{:>w$.w$} {:>w$} {:>w$} {:>w$}{}",
                kname,
                rd,
                wt,
                MaybeSmart(p, self.settings),
                newline
            )?
        }

        write!(f, "{}", newline)
    }
}
