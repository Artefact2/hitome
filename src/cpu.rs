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
    /* Use a BTreeMap to keep CPUs in a deterministic order */
    state: BTreeMap<usize, (CpuTicks, CpuTicks, Stale)>,
    buf: String,
}

impl<'a> StatBlock<'a> for CpuStats<'a> {
    fn new(s: &'a Settings) -> CpuStats {
        let mut cpu = CpuStats {
            settings: s,
            state: Default::default(),
            buf: String::new(),
        };
        cpu.update();
        cpu
    }

    fn update(&mut self) {
        /* /proc/stats never contains arbitrary user data */
        match unsafe { read_to_string_unchecked("/proc/stat", &mut self.buf) } {
            Ok(_) => (),
            _ => {
                self.state.clear();
                return;
            }
        }

        for (_, s) in self.state.iter_mut() {
            s.2 = Stale(true);
        }

        for cpu in self.buf.lines().skip(1) {
            let mut fields = cpu.split_ascii_whitespace();

            let cpuid = fields.next().unwrap();
            if !cpuid.starts_with("cpu") {
                break;
            }
            let cpuid = cpuid.strip_prefix("cpu").unwrap().parse::<usize>().unwrap();

            let mut ent = match self.state.get_mut(&cpuid) {
                Some(ent) => ent,
                _ => {
                    let z = CpuTicks {
                        user: 0,
                        nice: 0,
                        system: 0,
                        iowait: 0,
                        idle: 0,
                        total: 0,
                    };
                    self.state.insert(cpuid, (z, z, Stale(false)));
                    self.state.get_mut(&cpuid).unwrap()
                }
            };

            ent.0 = ent.1;
            ent.1.total = 0;
            ent.2 = Stale(false);

            for j in 0..=4 {
                let t = fields.next().unwrap().parse::<u64>().unwrap();

                /* https://docs.kernel.org/filesystems/proc.html#miscellaneous-kernel-statistics-in-proc-stat */
                match j {
                    0 => ent.1.user = t,
                    1 => ent.1.nice = t,
                    2 => ent.1.system = t,
                    3 => ent.1.idle = t,
                    4 => ent.1.iowait = t,
                    _ => unreachable!(),
                }

                ent.1.total += t;
            }
        }

        self.state.retain(|_, s| s.2 == Stale(false));
    }

    fn columns(&self) -> u16 {
        if self.state.is_empty() {
            0
        } else {
            self.settings.colwidth + 1 + self.state.len() as u16
        }
    }

    fn rows(&self) -> u16 {
        if self.state.is_empty() {
            0
        } else {
            5
        }
    }
}

impl<'a> fmt::Display for CpuStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.state.is_empty() {
            return Ok(());
        }

        let newline = MaybeSmart(Newline(), self.settings);

        for cat in ["IOWAIT", "SYSTEM", "USER", "NICE"].iter() {
            write!(f, "{} ", MaybeSmart(Heading(cat), self.settings))?;

            /* XXX: this doesn't feel like the best way */
            let get = |c: CpuTicks| match *cat {
                "IOWAIT" => c.iowait,
                "SYSTEM" => c.system,
                "USER" => c.user,
                "NICE" => c.nice,
                _ => unreachable!(),
            };

            for (_, cpu) in self.state.iter() {
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

            write!(f, "{}", newline)?
        }

        write!(f, "{}", newline)
    }
}
