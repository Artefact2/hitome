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
use std::fmt;

pub struct PressureStats<'a> {
    settings: &'a Settings,
}

impl<'a> PressureStats<'a> {
    pub fn new(s: &'a Settings) -> PressureStats {
        PressureStats { settings: s }
    }
}

impl<'a> fmt::Display for PressureStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut cells = [Threshold {
            val: Percentage(0.0),
            med: Percentage(1.0),
            high: Percentage(5.0),
            crit: Percentage(10.0),
            smart: self.settings.smart,
        }; 18];

        [
            /* XXX: pass through mutable slices? */
            ("cpu", 0, 3),
            ("memory", 6, 9),
            ("io", 12, 15),
        ]
        .map(|s| {
            let mut p = std::path::PathBuf::from("/proc/pressure");
            p.push(s.0);
            let pressure = match std::fs::read_to_string(p) {
                Ok(a) => a,
                _ => return,
            };
            for line in pressure.lines() {
                let mut elems = line.split_ascii_whitespace();
                let idx = match elems.nth(0) {
                    Some("some") => s.1,
                    Some("full") => s.2,
                    _ => continue,
                };

                for el in elems {
                    let (idx, p) = match el.rsplit_once('=') {
                        Some(("avg10", p)) => (idx, p),
                        Some(("avg60", p)) => (idx + 1, p),
                        Some(("avg300", p)) => (idx + 2, p),
                        _ => continue,
                    };
                    cells[idx].val.0 = p.parse::<f32>().unwrap();
                }
            }
        });

        let w = self.settings.colwidth;
        let newline = newline(self.settings.smart);
        let (hdrbegin, hdrend) = headings(self.settings.smart);
        write!(
            f,
            "{}{:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$}{}{}",
            hdrbegin,
            "PSI",
            "SOME_CPU",
            "FULL_CPU",
            "SOME_MEM",
            "FULL_MEM",
            "SOME_IO",
            "FULL_IO",
            hdrend,
            newline
        )?;

        for el in [("avg10", 0), ("avg60", 1), ("avg300", 2)] {
            write!(
                f,
                "{:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$}{}",
                el.0,
                cells[el.1],
                cells[el.1 + 3],
                cells[el.1 + 6],
                cells[el.1 + 9],
                cells[el.1 + 12],
                cells[el.1 + 15],
                newline
            )?;
        }

        write!(f, "{}", newline)
    }
}
