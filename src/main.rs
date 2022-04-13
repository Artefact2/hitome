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

use crate::blockdev::BlockDeviceStats;
use crate::common::*;
use crate::cpu::CpuStats;
use crate::fs::FilesystemStats;
use crate::mem::MemoryStats;
use crate::network::NetworkStats;
use crate::pressure::PressureStats;
use crate::tasks::TaskStats;
use std::io::{self, BufWriter, Write};
use std::thread;
use std::time::{Duration, Instant};

mod blockdev;
mod common;
mod cpu;
mod fs;
mod mem;
mod network;
mod pressure;
mod tasks;

/// A function-like macro that .update()s all of its arguments
macro_rules! update {
    ($( $x:expr ),*) => {
        $($x.update();)*
    }
}

fn main() {
    if !cfg!(target_os = "linux") {
        eprintln!("Hitome only works by reading Linux-specific /proc interfaces, sorry.");
        return;
    }

    let settings;
    {
        let cli: Cli = argh::from_env();
        settings = Settings {
            smart: cli
                .colour
                .unwrap_or_else(|| match std::env::var_os("TERM") {
                    Some(val) => val != "dumb",
                    None => false,
                }),
            colwidth: cli.column_width,
            refresh: cli.refresh_interval,
        };
        assert!(settings.colwidth >= MIN_COL_WIDTH);
        /* Let cli drop out of scope, it has lived its usefulness */
    }

    /* Use ManuallyDrop to prevent flushing screen-clearing escape sequences, in case the program
     * crashes. This allows us to see Rust errors. */
    let mut w = std::mem::ManuallyDrop::new(BufWriter::new(io::stdout()));

    let mut mem = MemoryStats::new(&settings);
    let mut psi = PressureStats::new(&settings);
    let mut cpu_net = MergedStatBlock::<CpuStats, NetworkStats>::new(&settings);
    let mut bdev_fs = MergedStatBlock::<BlockDeviceStats, FilesystemStats>::new(&settings);
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

        update!(mem, psi, cpu_net, bdev_fs, tasks);
        write!(w, "{}{}{}{}{}", mem, psi, cpu_net, bdev_fs, tasks).unwrap();

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
