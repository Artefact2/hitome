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
struct RxBytes(Bytes);

#[derive(Clone, Copy)]
struct TxBytes(Bytes);

#[derive(Clone, Copy)]
struct IfaceStats(Instant, RxBytes, TxBytes);

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

        for iface in self.ifaces.values_mut() {
            iface.2 = Stale(true);
        }

        for dev in self.buf.lines().skip(2) {
            let mut dev = dev.split_ascii_whitespace();
            let kname = dev.nth(0).unwrap().strip_suffix(':').unwrap();

            /* XXX: make this user-configurable */
            if kname.starts_with("br") {
                continue;
            }

            let mut ent = match self.ifaces.get_mut(kname) {
                Some(v) => v,
                _ => {
                    let z = IfaceStats(Instant::now(), RxBytes(Bytes(0)), TxBytes(Bytes(0)));
                    self.ifaces
                        .insert(String::from(kname), (z, z, Stale(false)));
                    self.ifaces.get_mut(kname).unwrap()
                }
            };

            ent.0 = ent.1;
            ent.1 = IfaceStats(
                Instant::now(),
                RxBytes(Bytes(dev.nth(0).unwrap().parse::<u64>().unwrap())),
                TxBytes(Bytes(dev.nth(7).unwrap().parse::<u64>().unwrap())),
            );
            ent.2 = Stale(false);
        }

        self.ifaces.retain(|_, v| v.2 == Stale(false));
    }
}

impl<'a> fmt::Display for NetworkStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.ifaces.is_empty() {
            return write!(f, "");
        }

        let newline = newline(self.settings.smart);
        let (hdrb, hdre) = headings(self.settings.smart);
        let w = self.settings.colwidth;
        write!(
            f,
            "{}{:>w$} {:>w$} {:>w$}{}{}",
            hdrb, "IFACE", "RX/s", "TX/s", hdre, newline
        )?;

        for (kname, s) in self.ifaces.iter() {
            let t = (s.1 .0 - s.0 .0).as_millis() as u64;
            /* XXX: surely there's a way to avoid the dot hell */
            /* XXX: how do the counters wrap in /proc/net/dev? */
            let rx = Bytes(1000 * (s.1 .1 .0 .0 - s.0 .1 .0 .0) / t);
            let tx = Bytes(1000 * (s.1 .2 .0 .0 - s.0 .2 .0 .0) / t);
            write!(f, "{:>w$} {:>w$} {:>w$}{}", kname, rx, tx, newline)?
        }

        write!(f, "{}", newline)
    }
}
