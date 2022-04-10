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

use page_size;
use std::cmp::Ordering;
use std::fmt;
use std::io::{self, BufWriter, Write};
use std::thread;
use std::time::{Duration, Instant};

const MIN_COL_WIDTH: usize = 8;

struct Settings {
    /// Terminal understands ansi colour/escape sequences?
    smart: bool,
    colwidth: usize,
    /// Refresh interval in ms
    refresh: u64,
}

#[derive(PartialEq, PartialOrd, Clone, Copy)]
struct Bytes(u64);

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.0 >= 10000 * 1024 * 1024 * 1024 {
            write!(
                f,
                "{:>7.2}T",
                self.0 as f32 / (1024. * 1024. * 1024. * 1024.)
            )
        } else if self.0 >= 10000 * 1024 * 1024 {
            write!(f, "{:>7.2}G", self.0 as f32 / (1024. * 1024. * 1024.))
        } else if self.0 >= 10000 * 1024 {
            write!(f, "{:>7.2}M", self.0 as f32 / (1024. * 1024.))
        } else {
            write!(f, "{:>7.2}K", self.0 as f32 / 1024.)
        }
    }
}

#[derive(PartialEq, PartialOrd, Clone, Copy)]
struct Percentage(f32);

impl fmt::Display for Percentage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:>7.2}%", self.0)
    }
}

#[derive(Clone, Copy)]
struct Threshold<T> {
    val: T,
    med: T,
    high: T,
    crit: T,
    smart: bool,
}

impl<T> fmt::Display for Threshold<T>
where
    T: fmt::Display + PartialOrd,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if !self.smart || self.val.partial_cmp(&self.med) == Some(Ordering::Less) {
            /* < med or dumb terminal */
            return write!(f, "{}", self.val);
        }

        if self.val.partial_cmp(&self.high) == Some(Ordering::Less) {
            /* < high: we're med */
            write!(f, "\x1B[1;93m{}\x1B[0m", self.val)
        } else if self.val.partial_cmp(&self.crit) == Some(Ordering::Less) {
            /* < crit: we're high */
            write!(f, "\x1B[1;91m{}\x1B[0m", self.val)
        } else {
            /* crit */
            write!(f, "\x1B[1;95m{}\x1B[0m", self.val)
        }
    }
}

struct MemoryStats<'a> {
    settings: &'a Settings,
    pagesize: u64,
}

impl<'a> MemoryStats<'a> {
    fn new(s: &'a Settings) -> MemoryStats {
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

struct PressureStats<'a> {
    settings: &'a Settings,
}

impl<'a> PressureStats<'a> {
    fn new(s: &'a Settings) -> PressureStats {
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

struct CpuStats<'a> {
    settings: &'a Settings,
    state: Vec<(CpuTicks, CpuTicks)>,
}

impl<'a> CpuStats<'a> {
    fn new(s: &'a Settings) -> CpuStats {
        let mut cpu = CpuStats {
            settings: s,
            state: Vec::new(),
        };
        cpu.update();
        cpu
    }

    fn update(&mut self) {
        let cpus = match std::fs::read_to_string("/proc/stat") {
            Ok(s) => s,
            _ => return,
        };

        for (i, cpu) in cpus.lines().skip(1).enumerate() {
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
            return write!(f, "");
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

                write!(
                    f,
                    "{}",
                    Threshold {
                        val: CpuUsage(
                            ((get(cpu.1) - get(cpu.0)) as f32)
                                / ((cpu.1.total - cpu.0.total) as f32)
                        ),
                        med: CpuUsage(trs.0),
                        high: CpuUsage(trs.1),
                        crit: CpuUsage(trs.2),
                        smart: self.settings.smart,
                    }
                )
                .unwrap();
            }

            write!(f, "{}", newline).unwrap();
        });

        write!(f, "{}", newline)
    }
}

fn newline(smart: bool) -> &'static str {
    if smart {
        "\x1B[0K\n"
    } else {
        "\n"
    }
}

fn headings(smart: bool) -> (&'static str, &'static str) {
    if smart {
        ("\x1B[1m", "\x1B[0m")
    } else {
        ("", "")
    }
}

fn main() {
    if !cfg!(target_os = "linux") {
        writeln!(
            io::stderr(),
            "Hitome only works by reading Linux-specific /proc interfaces, sorry."
        )
        .unwrap();
        return;
    }

    let settings = Settings {
        smart: match std::env::var_os("TERM") {
            Some(val) => val != "dumb",
            None => false,
        },
        /* TODO: adjust based on user setting and/or tput cols */
        colwidth: 8,
        /* TODO: make user configurable */
        refresh: 2000,
    };
    assert!(settings.colwidth >= MIN_COL_WIDTH);

    let mut w = BufWriter::new(io::stdout());
    let mem = MemoryStats::new(&settings);
    let psi = PressureStats::new(&settings);
    let mut cpu = CpuStats::new(&settings);

    println!("Hitome will now wait a while to collect statistics...");
    thread::sleep(Duration::from_millis(settings.refresh));

    loop {
        let t = Instant::now();

        if settings.smart {
            /* Move cursor to top-left */
            write!(w, "\x1B[1;1H\x1B[0J").unwrap();
        } else {
            writeln!(w, "----------").unwrap();
        }

        cpu.update();
        write!(w, "{}{}{}", mem, psi, cpu).unwrap();

        if settings.smart {
            /* Erase from cursor to end */
            write!(w, "\x1B[0J").unwrap();
        }

        w.flush().unwrap();
        thread::sleep(Duration::from_millis(
            settings.refresh - t.elapsed().as_millis() as u64,
        ));
    }
}
