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

#[derive(Clone, Copy)]
struct CpuTicks {
    user: u64,
    nice: u64,
    system: u64,
    iowait: u64,
    idle: u64,
    total: u64,
}

#[derive(PartialEq, PartialOrd)]
struct CpuUsage(f32);

impl fmt::Display for CpuUsage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self.0 {
                x if x >= 0.6 => 'X',
                x if x >= 0.2 => 'O',
                x if x >= 0.1 => 'o',
                x if x >= 0.01 => '.',
                _ => ' ',
            }
        )
    }
}

pub struct CpuStats<'a> {
    settings: &'a Settings,
    state: Vec<(CpuTicks, CpuTicks)>,
    buf: String,
}

impl<'a> StatBlock<'a> for CpuStats<'a> {
    fn new(s: &'a Settings) -> CpuStats {
        let mut cpu = CpuStats {
            settings: s,
            state: Vec::new(),
            buf: String::new(),
        };
        cpu.update();
        cpu
    }

    fn update(&mut self) {
        match read_to_string("/proc/stat", &mut self.buf) {
            Ok(_) => (),
            _ => return,
        }

        for (i, cpu) in self.buf.lines().skip(1).enumerate() {
            let mut fields = cpu.split_ascii_whitespace();

            if !fields.nth(0).unwrap().starts_with("cpu") {
                return;
            }

            if self.state.len() <= i {
                let z = CpuTicks {
                    user: 0,
                    nice: 0,
                    system: 0,
                    iowait: 0,
                    idle: 0,
                    total: 0,
                };
                /* XXX: handle cases where CPUs go offline */
                self.state.push((z, z));
            } else {
                self.state[i].0 = self.state[i].1;
                self.state[i].1.total = 0;
            }

            for (j, t) in fields.enumerate() {
                let t = t.parse::<u64>().unwrap();

                /* https://docs.kernel.org/filesystems/proc.html#miscellaneous-kernel-statistics-in-proc-stat */
                match j {
                    0 => self.state[i].1.user = t,
                    1 => self.state[i].1.nice = t,
                    2 => self.state[i].1.system = t,
                    3 => self.state[i].1.idle = t,
                    4 => self.state[i].1.iowait = t,
                    _ => (),
                }

                self.state[i].1.total += t;
            }
        }
    }
}

impl<'a> fmt::Display for CpuStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.state.len() == 0 {
            return Ok(());
        }

        let newline = newline(self.settings.smart);
        let (hdrstart, hdrend) = headings(self.settings.smart);
        let w = self.settings.colwidth;

        ["IOWAIT", "SYSTEM", "USER", "NICE"].map(|cat| {
            /* XXX: find way to pass through io::Errors */
            write!(f, "{}{:>w$}{} ", hdrstart, cat, hdrend).unwrap();

            /* XXX: this doesn't feel like the best way */
            let get = |c: CpuTicks| match cat {
                "IOWAIT" => c.iowait,
                "SYSTEM" => c.system,
                "USER" => c.user,
                "NICE" => c.nice,
                _ => panic!(),
            };

            for cpu in &self.state {
                /* Set thresholds for colouring based on idle% */
                let trs = match ((cpu.1.idle - cpu.0.idle) as f32)
                    / ((cpu.1.total - cpu.0.total) as f32)
                {
                    x if x <= 0.2 => (0.0, 0.0, 0.0),
                    x if x <= 0.4 => (0.0, 0.0, 1.0),
                    x if x <= 0.6 => (0.0, 1.0, 1.0),
                    _ => (1.0, 1.0, 1.0),
                };

                /* XXX: figure out if the counters ever wrap */
                write!(
                    f,
                    "{}",
                    MaybeSmart(Threshold {
                        /* Use a saturating sub, the iowait counters occasionally decrease(!). */
                        val: CpuUsage(
                            (get(cpu.1).saturating_sub(get(cpu.0)) as f32)
                                / ((cpu.1.total - cpu.0.total) as f32)
                        ),
                        med: CpuUsage(trs.0),
                        high: CpuUsage(trs.1),
                        crit: CpuUsage(trs.2),
                    }, self.settings)
                )
                .unwrap();
            }

            write!(f, "{}", newline).unwrap();
        });

        write!(f, "{}", newline)
    }
}
