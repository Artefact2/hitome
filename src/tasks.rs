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
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::fmt;
use std::fmt::Write;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct PID(u32);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
/// 1 jiffy = 1/user_hz seconds
struct Jiffies(u64, Instant);

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Sleeping,
    Running,
    Uninterruptible,
    Zombie,
    Traced,
    Idle,
    Unknown,
}

/* XXX: impl colour Display for Smart(State) */
impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{:1.1}",
            match self {
                State::Sleeping => 'S',
                State::Running => 'R',
                State::Uninterruptible => 'D',
                State::Zombie => 'Z',
                State::Traced => 'T',
                State::Idle => 'I',
                State::Unknown => '?',
            }
        )
    }
}

#[derive(PartialEq, Eq)]
struct TaskSort(State, u64);

impl PartialOrd for TaskSort {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TaskSort {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.0, other.0) {
            (State::Uninterruptible, State::Uninterruptible) => self.1.cmp(&other.1),
            (State::Uninterruptible, _) => Ordering::Greater,
            (_, State::Uninterruptible) => Ordering::Less,
            _ => self.1.cmp(&other.1),
        }
    }
}

pub struct TaskStats<'a> {
    settings: &'a Settings,
    /// How many jiffies in a second, as exposed to userspace
    user_hz: f32,
    buf: String,
    buf2: String,
    tasks: HashMap<PID, (Jiffies, Jiffies, State, Stale)>,
    /// Used to sort tasks by their State/CPU%. Pushing is O(1) and popping is O(log n). Pushing all
    /// the tasks and popping the 10 highest is only O(n + 10 log n) instead of sorting which is O(n
    /// log n).
    sorted: BinaryHeap<(TaskSort, PID)>,
    /// XXX: make the number of tasks user-configurable and/or guess based on terminal lines
    relevant: [String; 10],
}

impl<'a> StatBlock<'a> for TaskStats<'a> {
    fn new(s: &'a Settings) -> Self {
        let mut ts = TaskStats {
            settings: s,
            user_hz: unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f32,
            buf: String::new(),
            buf2: String::new(),
            tasks: HashMap::new(),
            sorted: BinaryHeap::new(),
            relevant: Default::default(),
        };
        ts.update();
        ts
    }

    /* XXX: split off into smaller, more digestible fns */
    fn update(&mut self) {
        for t in self.tasks.values_mut() {
            t.3 = Stale(true);
        }

        let t = Instant::now();

        for pid_path in std::fs::read_dir("/proc").unwrap() {
            let pid_path = match pid_path {
                Ok(p) => p,
                _ => continue,
            };

            if !pid_path.file_type().unwrap().is_dir() {
                continue;
            }

            let pid = match pid_path.file_name().to_str().unwrap().parse::<u32>() {
                Ok(i) => PID(i),
                _ => continue,
            };

            let mut ent = match self.tasks.get_mut(&pid) {
                Some(e) => e,
                _ => {
                    let z = (Jiffies(0, t), Jiffies(0, t), State::Sleeping, Stale(false));
                    self.tasks.insert(pid, z);
                    self.tasks.get_mut(&pid).unwrap()
                }
            };

            /* XXX: don't stop there, branch into ./tasks/.../stat */
            let mut pid_path = pid_path.path();
            pid_path.push("stat");
            match read_to_string(pid_path, &mut self.buf) {
                Ok(_) => (),
                _ => continue,
            }

            let mut stat = self
                .buf
                .rsplit_once(')')
                .unwrap()
                .1
                .split_ascii_whitespace();

            ent.2 = match stat.nth(0).unwrap() {
                "S" => State::Sleeping,
                "R" => State::Running,
                "D" => State::Uninterruptible,
                "Z" => State::Zombie,
                "T" => State::Traced,
                "I" => State::Idle,
                _ => State::Unknown,
            };
            ent.0 = ent.1;
            ent.1 = Jiffies(
                stat.nth(10).unwrap().parse::<u64>().unwrap()
                    + stat.nth(0).unwrap().parse::<u64>().unwrap(),
                t,
            );
            ent.3 = Stale(false);
        }

        self.tasks.retain(|_, t| t.3 == Stale(false));

        self.sorted.clear();
        for (pid, task) in self.tasks.iter() {
            self.sorted
                .push((TaskSort(task.2, task.1 .0 - task.0 .0), *pid));
        }

        let newline = newline(self.settings.smart);
        let w = self.settings.colwidth;
        for s in self.relevant.iter_mut() {
            s.clear();
            let pid = match self.sorted.pop() {
                Some((_, pid)) => pid,
                _ => continue, /* out of tasks, clear remaining Strings */
            };
            let ent = self.tasks.get(&pid).unwrap();
            let cpupc = ((100000 * (ent.1 .0 - ent.0 .0)) as f32)
                / self.user_hz
                / ((ent.1 .1 - ent.0 .1).as_millis() as f32);
            if cpupc < 1.0 {
                /* Don't show tasks that barely use the CPU */
                continue;
            }

            /* XXX: find better way to do this */
            self.buf2.clear();
            write!(self.buf2, "/proc/{}/cmdline", pid.0).unwrap();
            /* XXX: this is very rough, format me better! */
            let cmdline = match read_to_string(&self.buf2, &mut self.buf) {
                Ok(_) => &self.buf,
                _ => "?",
            };

            write!(
                s,
                /* XXX: fix hardcoded length */
                "{:>w$} {} {:>3.0}% {:<55.55}{}",
                pid.0, ent.2, cpupc, cmdline, newline
            )
            .unwrap();
        }
    }
}

impl<'a> fmt::Display for TaskStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let newline = newline(self.settings.smart);
        let (hdrbegin, hdrend) = headings(self.settings.smart);
        let w = self.settings.colwidth;
        write!(
            f,
            "{}{:>w$} S CPU% COMMAND{}{}",
            hdrbegin, "PID", hdrend, newline
        )?;

        for s in self.relevant.iter() {
            f.write_str(s)?;
        }
        Ok(())
    }
}
