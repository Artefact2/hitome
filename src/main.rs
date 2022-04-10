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

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Bytes(u64);

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.0 >= 10000 * 1024 * 1024 * 1024 {
            write!(
                f,
                "{:7.2}T",
                self.0 as f32 / (1024. * 1024. * 1024. * 1024.)
            )
        } else if self.0 >= 10000 * 1024 * 1024 {
            write!(f, "{:7.2}G", self.0 as f32 / (1024. * 1024. * 1024.))
        } else if self.0 >= 10000 * 1024 {
            write!(f, "{:7.2}M", self.0 as f32 / (1024. * 1024.))
        } else {
            write!(f, "{:7.2}K", self.0 as f32 / 1024.)
        }
    }
}

struct Threshold<T> {
    val: T,
    med: T,
    high: T,
    crit: T,
    smart: bool,
}

impl<T> fmt::Display for Threshold<T>
where
    T: fmt::Display + Ord,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if !self.smart || self.val.cmp(&self.med) == Ordering::Less {
            /* < med or dumb terminal */
            return write!(f, "{}", self.val);
        }

        if self.val.cmp(&self.high) == Ordering::Less {
            /* < high: we're med */
            write!(f, "\x1B[1;93m{}\x1B[0m", self.val)
        } else if self.val.cmp(&self.crit) == Ordering::Less {
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

    loop {
        let t = Instant::now();

        if settings.smart {
            /* Move cursor to top-left */
            write!(w, "\x1B[1;1H\x1B[0J").unwrap();
        } else {
            writeln!(w, "----------").unwrap();
        }

        write!(w, "{}", mem).unwrap();

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
