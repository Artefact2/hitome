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

            match bdev.file_name().to_str() {
                Some(s) => {
                    if !s.starts_with("zram") {
                        continue;
                    }
                }
                _ => continue,
            };

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
                _ => (),
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
                let idx: usize;

                match elems.nth(0) {
                    Some("some") => idx = s.1,
                    Some("full") => idx = s.2,
                    _ => continue,
                }

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

    loop {
        let t = Instant::now();

        if settings.smart {
            /* Move cursor to top-left */
            write!(w, "\x1B[1;1H\x1B[0J").unwrap();
        } else {
            writeln!(w, "----------").unwrap();
        }

        write!(w, "{}{}", mem, psi).unwrap();

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
