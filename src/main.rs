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

use hitome::blockdev::BlockDeviceStats;
use hitome::common::*;
use hitome::cpu::CpuStats;
use hitome::fs::FilesystemStats;
use hitome::hwmon::HwmonStats;
use hitome::mem::MemoryStats;
use hitome::network::NetworkStats;
use hitome::pressure::PressureStats;
use hitome::tasks::TaskStats;
use std::cell::Cell;
use std::io::{self, BufWriter, Write};
use std::thread;
use std::time::{Duration, Instant};

// XXX: find a better place for these
const MIN_COL_WIDTH: u16 = 8;
const MIN_COLUMNS: u16 = 8 * MIN_COL_WIDTH + 7;
const MIN_ROWS: u16 = 24;

/// A function-like macro that .update()s all of its arguments
macro_rules! update {
    ($( $x:expr ),*) => {
        $($x.update();)*
    }
}

struct TermDimensions {
    rows: u16,
    cols: u16,
}

fn get_term_dimensions() -> Option<TermDimensions> {
    unsafe {
        let mut w = std::mem::MaybeUninit::<libc::winsize>::uninit();
        /* This isn't very portable, but neither is Hitome */
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, w.as_mut_ptr()) == 0 {
            let w = w.assume_init();
            return Some(TermDimensions {
                rows: w.ws_row,
                cols: w.ws_col,
            });
        }
    }

    /* As a fallback, get dimensions from the environment */
    if let Some(lines) = std::env::var_os("LINES") {
        if let Ok(lines) = lines.to_string_lossy().parse::<u16>() {
            if let Some(columns) = std::env::var_os("COLUMNS") {
                if let Ok(columns) = columns.to_string_lossy().parse::<u16>() {
                    return Some(TermDimensions {
                        rows: lines,
                        cols: columns,
                    });
                }
            }
        }
    }

    None
}

fn update_term_dimensions(s: &Settings) {
    if !s.auto_maxcols && !s.auto_maxrows {
        return;
    }

    let termsize = get_term_dimensions().unwrap_or(TermDimensions { rows: 0, cols: 0 });
    if s.auto_maxcols {
        s.maxcols.set(termsize.cols.max(MIN_COLUMNS));
        if s.auto_colwidth {
            s.colwidth
                .set(((s.maxcols.get() - 7) / 8).clamp(MIN_COL_WIDTH, 10));
        }
    }
    if s.auto_maxrows {
        s.maxrows.set(termsize.rows.max(MIN_ROWS));
    }

    assert!(s.maxcols.get() >= MIN_COLUMNS);
    assert!(s.maxrows.get() >= MIN_ROWS);
    assert!(s.colwidth.get() >= MIN_COL_WIDTH);
}

fn main() {
    if !cfg!(target_os = "linux") {
        eprintln!("Hitome only works by reading Linux-specific /proc interfaces, sorry.");
        return;
    }

    let settings;
    {
        let cli: Cli = argh::from_env();
        if cli.columns == None || cli.rows == None {}
        settings = Settings {
            smart: cli
                .colour
                .unwrap_or_else(|| match std::env::var_os("TERM") {
                    Some(val) => val != "dumb",
                    None => false,
                }),
            auto_maxcols: cli.columns == None,
            auto_maxrows: cli.rows == None,
            auto_colwidth: cli.column_width == None,
            maxcols: Cell::new(cli.columns.unwrap_or(0)),
            maxrows: Cell::new(cli.rows.unwrap_or(0)),
            colwidth: Cell::new(cli.column_width.unwrap_or(0)),
            refresh: cli.refresh_interval,
        };
        update_term_dimensions(&settings);
        /* Let cli drop out of scope, it has lived its usefulness */
    }

    /* Use ManuallyDrop to prevent flushing screen-clearing escape sequences, in case the program
     * crashes. This allows us to see Rust errors. */
    let mut w = std::mem::ManuallyDrop::new(BufWriter::new(io::stdout()));

    let mut mem = MemoryStats::new(&settings);
    let mut psi = PressureStats::new(&settings);
    let mut cpu_net = MergedStatBlock::<CpuStats, NetworkStats>::new(&settings);
    let mut bdev_fs = MergedStatBlock::<BlockDeviceStats, FilesystemStats>::new(&settings);
    let mut hwmon = HwmonStats::new(&settings);
    let mut tasks = TaskStats::new(&settings);

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

        update_term_dimensions(&settings);
        update!(mem, psi, cpu_net, bdev_fs, hwmon);
        let remaining_rows = settings.maxrows.get() as i16
            - mem.rows() as i16
            - psi.rows() as i16
            - cpu_net.rows() as i16
            - bdev_fs.rows() as i16
            - hwmon.rows() as i16
            - 2;
        tasks.set_max_tasks(remaining_rows.max(5) as u16);
        update!(tasks);
        write!(w, "{}{}{}{}{}{}", mem, psi, cpu_net, bdev_fs, hwmon, tasks).unwrap();

        if settings.smart {
            /* Erase from cursor to end */
            write!(w, "\x1B[0J").unwrap();
        }

        w.flush().unwrap();
        thread::sleep(Duration::from_millis(
            settings
                .refresh
                .saturating_sub(t.elapsed().as_millis() as u64),
        ));
    }
}
