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

/// 10s, 60s, 300s
#[derive(Copy, Clone)]
struct Pressure {
    some: [Threshold<Percentage>; 3],
    full: [Threshold<Percentage>; 3],
}

pub struct PressureStats<'a> {
    settings: &'a Settings,
    cpu: Pressure,
    memory: Pressure,
    io: Pressure,
    buf: String,
}

impl<'a> PressureStats<'a> {
    fn update_cat(pa: &str, buf: &mut String, pr: &mut Pressure) {
        match read_to_string(pa, buf) {
            Ok(_) => (),
            _ => return,
        }

        match read_to_string(pa, buf) {
            Ok(_) => (),
            _ => return,
        }

        for line in buf.lines() {
            let mut elems = line.split_ascii_whitespace();
            let pr = match elems.nth(0) {
                Some("some") => &mut pr.some,
                Some("full") => &mut pr.full,
                _ => continue,
            };

            for el in elems {
                let (idx, p) = match el.rsplit_once('=') {
                    Some(("avg10", p)) => (0, p),
                    Some(("avg60", p)) => (1, p),
                    Some(("avg300", p)) => (2, p),
                    _ => continue,
                };
                pr[idx].val.0 = p.parse::<f32>().unwrap();
            }
        }
    }
}

impl<'a> StatBlock<'a> for PressureStats<'a> {
    fn new(s: &'a Settings) -> PressureStats {
        let z = Threshold {
            val: Percentage(0.0),
            med: Percentage(1.0),
            high: Percentage(5.0),
            crit: Percentage(10.0),
        };
        let z = Pressure {
            some: [z; 3],
            full: [z; 3],
        };
        PressureStats {
            settings: s,
            cpu: z,
            memory: z,
            io: z,
            buf: String::new(),
        }
    }

    fn update(&mut self) {
        PressureStats::update_cat("/proc/pressure/cpu", &mut self.buf, &mut self.cpu);
        PressureStats::update_cat("/proc/pressure/memory", &mut self.buf, &mut self.memory);
        PressureStats::update_cat("/proc/pressure/io", &mut self.buf, &mut self.io);
    }
}

impl<'a> fmt::Display for PressureStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        /* XXX: this isn't very reliable */
        if self.buf.len() == 0 {
            return Ok(());
        }

        let w = self.settings.colwidth;
        let s = self.settings;
        let newline = MaybeSmart(Newline(), s);
        write!(
            f,
            "{} {} {} {} {} {} {}{}",
            MaybeSmart(Heading("PSI"), s),
            MaybeSmart(Heading("SOME_CPU"), s),
            MaybeSmart(Heading("FULL_CPU"), s),
            MaybeSmart(Heading("SOME_MEM"), s),
            MaybeSmart(Heading("FULL_MEM"), s),
            MaybeSmart(Heading("SOME_IO"), s),
            MaybeSmart(Heading("FULL_IO"), s),
            newline
        )?;

        for (label, i) in [("avg10", 0), ("avg60", 1), ("avg300", 2)] {
            write!(
                f,
                "{:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$} {:>w$}{}",
                label,
                MaybeSmart(self.cpu.some[i], self.settings),
                MaybeSmart(self.cpu.full[i], self.settings),
                MaybeSmart(self.memory.some[i], self.settings),
                MaybeSmart(self.memory.full[i], self.settings),
                MaybeSmart(self.io.some[i], self.settings),
                MaybeSmart(self.io.full[i], self.settings),
                newline
            )?;
        }

        write!(f, "{}", newline)
    }
}
