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
use crate::cpu::CpuStats;
use crate::mem::MemoryStats;
use crate::network::NetworkStats;
use crate::pressure::PressureStats;
use std::io::{self, BufWriter, Write};
use std::thread;
use std::time::{Duration, Instant};

mod common;
mod cpu;
mod mem;
mod network;
mod pressure;

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
    /* XXX: encapsulate in common.rs */
    assert!(settings.colwidth >= MIN_COL_WIDTH);

    /* Use ManuallyDrop to prevent flushing screen-clearing escape sequences, in case the program
     * crashes. This allows us to see Rust errors. */
    let mut w = std::mem::ManuallyDrop::new(BufWriter::new(io::stdout()));

    let mem = MemoryStats::new(&settings);
    let psi = PressureStats::new(&settings);
    let mut cpu = CpuStats::new(&settings);
    let mut net = NetworkStats::new(&settings);

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
        net.update();

        /* XXX: merge cpu/net if term has enough cols */
        write!(w, "{}{}{}{}", mem, psi, cpu, net).unwrap();

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
