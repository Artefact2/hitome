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
struct ReadBytes(Bytes);

#[derive(Clone, Copy)]
struct WrittenBytes(Bytes);

#[derive(Clone, Copy)]
struct WeighedRequestTime(u64);

#[derive(Clone, Copy)]
struct DevStats(Instant, ReadBytes, WrittenBytes, WeighedRequestTime);

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
            if kname.starts_with("dm-") {
                continue;
            }

            let mut ent = match self.devices.get_mut(kname) {
                Some(v) => v,
                _ => {
                    let z = DevStats(
                        t,
                        ReadBytes(Bytes(0)),
                        WrittenBytes(Bytes(0)),
                        WeighedRequestTime(0),
                    );
                    self.devices
                        .insert(String::from(kname), (z, z, Stale(false)));
                    self.devices.get_mut(kname).unwrap()
                }
            };

            ent.0 = ent.1;
            ent.1 = DevStats(
                t,
                ReadBytes(Bytes(
                    SECTOR_SIZE * bdev.nth(2).unwrap().parse::<u64>().unwrap(),
                )),
                WrittenBytes(Bytes(
                    SECTOR_SIZE * bdev.nth(3).unwrap().parse::<u64>().unwrap(),
                )),
                WeighedRequestTime(bdev.nth(3).unwrap().parse::<u64>().unwrap()),
            );
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
            let t = (s.1 .0 - s.0 .0).as_millis() as u64;
            if t == 0 {
                /* Device was just added */
                continue;
            }
            /* XXX ugly syntax */
            let rd = Bytes(1000 * (s.1 .1 .0 .0 - s.0 .1 .0 .0) / t);
            let wt = Bytes(1000 * (s.1 .2 .0 .0 - s.0 .2 .0 .0) / t);
            let p = Threshold {
                val: Percentage(100.0 * ((s.1 .3 .0 - s.0 .3 .0) as f32) / (t as f32)),
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
