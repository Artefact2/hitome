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
use std::collections::HashMap;
use std::fmt;
use std::time::Instant;

#[derive(Clone, Copy)]
struct IfaceStats {
    t: Instant,
    rx: Bytes,
    tx: Bytes,
}

pub struct NetworkStats<'a> {
    settings: &'a Settings,
    /// kname (eg. enp6s0) -> ...
    ifaces: HashMap<String, (IfaceStats, IfaceStats, Stale)>,
    buf: String,
}

impl<'a> StatBlock<'a> for NetworkStats<'a> {
    fn new(s: &'a Settings) -> NetworkStats {
        let mut ns = NetworkStats {
            settings: s,
            ifaces: HashMap::new(),
            buf: String::new(),
        };
        ns.update();
        ns
    }

    fn update(&mut self) {
        match read_to_string("/proc/net/dev", &mut self.buf) {
            Ok(_) => (),
            _ => return,
        }

        let t = Instant::now();

        for iface in self.ifaces.values_mut() {
            iface.2 = Stale(true);
        }

        for dev in self.buf.lines().skip(2) {
            let mut dev = dev.split_ascii_whitespace();
            let kname = dev.next().unwrap().strip_suffix(':').unwrap();

            /* XXX: make this user-configurable */
            if kname.starts_with("br") {
                continue;
            }

            let mut ent = match self.ifaces.get_mut(kname) {
                Some(v) => v,
                _ => {
                    let z = IfaceStats {
                        t: t,
                        rx: Bytes(0),
                        tx: Bytes(0),
                    };
                    self.ifaces
                        .insert(String::from(kname), (z, z, Stale(false)));
                    self.ifaces.get_mut(kname).unwrap()
                }
            };

            ent.0 = ent.1;
            ent.1 = IfaceStats {
                t: t,
                rx: Bytes(dev.next().unwrap().parse().unwrap()),
                tx: Bytes(dev.nth(7).unwrap().parse().unwrap()),
            };
            ent.2 = Stale(false);
        }

        self.ifaces.retain(|_, v| v.2 == Stale(false));
    }
}

impl<'a> fmt::Display for NetworkStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.ifaces.is_empty() {
            return Ok(());
        }

        let newline = MaybeSmart(Newline(), self.settings);
        let w = self.settings.colwidth;
        write!(
            f,
            "{} {} {}{}",
            MaybeSmart(Heading("IFACE"), self.settings),
            MaybeSmart(Heading("RX/s"), self.settings),
            MaybeSmart(Heading("TX/s"), self.settings),
            newline
        )?;

        for (kname, s) in self.ifaces.iter() {
            /* From https://github.com/torvalds/linux/blob/master/include/uapi/linux/if_link.h, the
             * stats reported will wrap at either u32::MAX or (more likely) u64::MAX. */
            let t = (s.1.t - s.0.t).as_millis() as u64;
            let rx = Bytes(1000 * (s.1.rx.0.wrapping_sub(s.0.rx.0)) / t);
            let tx = Bytes(1000 * (s.1.tx.0.wrapping_sub(s.0.tx.0)) / t);
            write!(f, "{:>w$.w$} {:>w$} {:>w$}{}", kname, rx, tx, newline)?
        }

        write!(f, "{}", newline)
    }
}
