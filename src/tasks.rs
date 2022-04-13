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
/// Linux PIDs should not go above 2^22, says proc(5)
struct PID(u32);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
/// 1 jiffie = 1/user_hz seconds
struct Jiffies(u64, Instant);

#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskState {
    Sleeping,
    Running,
    Uninterruptible,
    Zombie,
    Traced,
    Idle,
    Unknown,
}

impl<'a> fmt::Display for MaybeSmart<'a, TaskState> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let letter = match self.0 {
            TaskState::Sleeping => 'S',
            TaskState::Running => 'R',
            TaskState::Uninterruptible => 'D',
            TaskState::Zombie => 'Z',
            TaskState::Traced => 'T',
            TaskState::Idle => 'I',
            TaskState::Unknown => '?',
        };

        let w = f.width().unwrap_or(1);

        if !self.1.smart {
            return write!(f, "{:>w$}", letter);
        }

        match self.0 {
            TaskState::Running => write!(f, "\x1B[1;93m{:>w$}\x1B[0m", letter),
            TaskState::Uninterruptible => write!(f, "\x1B[1;95m{:>w$}\x1B[0m", letter),
            _ => write!(f, "{:>w$}", letter),
        }
    }
}

#[derive(Clone, Copy, PartialEq, PartialOrd)]
struct CPUPercentage(f32);

impl fmt::Display for CPUPercentage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let w = f.width().unwrap_or(4) - 1;
        write!(f, "{:>w$.0}%", self.0)
    }
}

#[derive(PartialEq, Eq)]
struct TaskSort(TaskState, u64);

impl PartialOrd for TaskSort {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TaskSort {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.0, other.0) {
            (TaskState::Uninterruptible, TaskState::Uninterruptible) => self.1.cmp(&other.1),
            (TaskState::Uninterruptible, _) => Ordering::Greater,
            (_, TaskState::Uninterruptible) => Ordering::Less,
            _ => self.1.cmp(&other.1),
        }
    }
}

/// (tcomm, stripped arg0, args)
struct CommandLine<'a>(&'a str, &'a str, &'a str);

impl<'a, 'b> fmt::Display for MaybeSmart<'a, CommandLine<'b>> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let len = f.width().unwrap_or(60);
        match self.0 {
            CommandLine(x, y, z) if y.starts_with(x) => {
                let len = len - y.len() - 1;
                match self.1.smart {
                    false => write!(f, "{} {:<len$}", y, z),
                    true => write!(f, "\x1B[1m{}\x1B[0m {:<len$.len$}", y, z),
                }
            }
            CommandLine(x, y, z) => {
                let len = len - x.len() - y.len() - 4;
                match self.1.smart {
                    false => write!(f, "({}) {} {:<len$}", x, y, z),
                    true => write!(f, "({}) \x1B[1m{}\x1B[0m {:<len$.len$}", x, y, z),
                }
            }
        }
    }
}

pub struct TaskStats<'a> {
    settings: &'a Settings,
    /// How many jiffies in a second, as exposed to userspace
    user_hz: f32,
    buf: String,
    buf2: String,
    buf3: String,
    tasks: HashMap<PID, (Jiffies, Jiffies, TaskState, Stale)>,
    /// Used to sort tasks by their State/CPU%. Pushing is O(1) and popping is O(log n). Pushing all
    /// the tasks and popping the 10 highest is only O(n + 10 log n) instead of sorting which is O(n
    /// log n).
    sorted: BinaryHeap<(TaskSort, PID)>,
    /// XXX: make the number of tasks user-configurable and/or guess based on terminal lines
    relevant: [String; 10],
}

/// Walk /proc and call the closure for each task, eg /proc/X/task/Y. Skips invalid files instead of
/// panicking, as tasks are created/deleted all the time and scanning them in /proc is inherently
/// racy. XXX: this would work better as an Iterator, but i don't know how to do that
fn map_tasks<F>(buf: &mut String, mut doit: F)
where
    F: FnMut(PID, &str),
{
    for process in std::fs::read_dir("/proc").unwrap() {
        let process = match process {
            Ok(p) => p,
            _ => continue,
        };

        match process.file_type() {
            Ok(f) if f.is_dir() => (),
            _ => continue,
        }

        /* XXX: is this allocating in every loop? */
        let mut tasks_path = process.path();
        tasks_path.push("task");
        for task in match std::fs::read_dir(tasks_path) {
            Ok(a) => a,
            _ => continue,
        } {
            let task = match task {
                Ok(p) => p,
                _ => continue,
            };
            let taskid = match task.file_name().to_str().unwrap_or("").parse::<u32>() {
                Ok(p) => PID(p),
                _ => continue,
            };

            let mut path = task.path();
            path.push("stat");
            match read_to_string(path, buf) {
                Ok(_) => doit(taskid, buf),
                _ => continue,
            }
        }
    }
}

impl<'a> TaskStats<'a> {
    /* XXX: we don't really need to mutate self, we just need an output String, but the borrow
     * checker won't let us */
    /// Format a task's line to self.relevant[i]
    fn format_task(&mut self, taskid: PID, i: usize) {
        let newline = MaybeSmart(Newline(), self.settings);
        let w = self.settings.colwidth;

        let ent = self.tasks.get(&taskid).unwrap();
        let cpupc = ((100000 * (ent.1 .0 - ent.0 .0)) as f32)
            / self.user_hz
            / ((ent.1 .1 - ent.0 .1).as_millis() as f32);
        if cpupc < 1.0 {
            /* Don't show tasks that barely use the CPU */
            return;
        }

        /* XXX: find better way to do this */
        self.buf2.clear();
        write!(self.buf2, "/proc/{}/task/{}/cmdline", taskid.0, taskid.0).unwrap();
        let cmdline = match read_to_string(&self.buf2, &mut self.buf) {
            Ok(_) => &self.buf,
            _ => "",
        };

        self.buf2.clear();
        write!(self.buf2, "/proc/{}/task/{}/comm", taskid.0, taskid.0).unwrap();
        let comm = match read_to_string(&self.buf2, &mut self.buf3) {
            Ok(_) => &self.buf3.strip_suffix("\n").unwrap(),
            _ => "",
        };

        /* Format the cmdline: skip path of argv[0], split args by spaces */
        let max_length = 55; /* XXX: adjust this based on term/user pref */
        let mut cmdline = cmdline.split('\0');
        let progname = cmdline.next().unwrap_or("");
        let progname = match progname.rsplit_once("/") {
            Some((_, p)) => p,
            _ => progname,
        };

        self.buf2.clear();
        for arg in cmdline {
            if self.buf2.len() >= max_length {
                break;
            }

            /* Some half-assed shell-like escaping, should cover most cases, doesn't need to be
             * perfect since it will be truncated anyway */
            match arg.contains(' ') {
                false => write!(self.buf2, "{} ", arg).unwrap(),
                true => match arg.contains('\'') {
                    false => write!(self.buf2, "'{}' ", arg).unwrap(),
                    /* XXX: creating a new String here may not be a good idea, hopefully this case is rare */
                    true => write!(self.buf2, "'{}' ", arg.replace('\\', "\\'")).unwrap(),
                },
            }
        }

        write!(
            self.relevant[i],
            "{:>w$} {:1} {:>4} {:<max_length$}{}",
            taskid.0,
            MaybeSmart(ent.2, self.settings),
            MaybeSmart(
                Threshold {
                    val: CPUPercentage(cpupc),
                    med: CPUPercentage(40.0),
                    high: CPUPercentage(60.0),
                    crit: CPUPercentage(80.0),
                },
                self.settings
            ),
            MaybeSmart(CommandLine(comm, progname, &self.buf2), self.settings),
            newline
        )
        .unwrap();
    }
}

impl<'a> StatBlock<'a> for TaskStats<'a> {
    fn new(s: &'a Settings) -> Self {
        let mut ts = TaskStats {
            settings: s,
            user_hz: unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f32,
            buf: String::new(),
            buf2: String::new(),
            buf3: String::new(),
            tasks: HashMap::new(),
            sorted: BinaryHeap::new(),
            relevant: Default::default(),
        };
        ts.update();
        ts
    }

    fn update(&mut self) {
        /* Measure and store jiffies of each task in self.tasks */
        for t in self.tasks.values_mut() {
            t.3 = Stale(true);
        }
        let t = Instant::now();
        map_tasks(&mut self.buf, |taskid, stat| {
            let mut ent = match self.tasks.get_mut(&taskid) {
                Some(e) => e,
                _ => {
                    let z = (
                        Jiffies(0, t),
                        Jiffies(0, t),
                        TaskState::Sleeping,
                        Stale(false),
                    );
                    self.tasks.insert(taskid, z);
                    self.tasks.get_mut(&taskid).unwrap()
                }
            };

            let mut stat = stat.rsplit_once(')').unwrap().1.split_ascii_whitespace();

            ent.2 = match stat.nth(0).unwrap() {
                "S" => TaskState::Sleeping,
                "R" => TaskState::Running,
                "D" => TaskState::Uninterruptible,
                "Z" => TaskState::Zombie,
                "T" => TaskState::Traced,
                "I" => TaskState::Idle,
                _ => TaskState::Unknown,
            };
            ent.0 = ent.1;
            ent.1 = Jiffies(
                stat.nth(10).unwrap().parse::<u64>().unwrap()
                    + stat.nth(0).unwrap().parse::<u64>().unwrap(),
                t,
            );
            ent.3 = Stale(false);
        });
        self.tasks.retain(|_, t| t.3 == Stale(false));

        /* Sort tasks by state/jiffies */
        self.sorted.clear();
        for (pid, task) in self.tasks.iter() {
            if task.0 .1 == task.1 .1 {
                /* Task was seen for the first time, data is not accurate yet */
                continue;
            }
            self.sorted
                .push((TaskSort(task.2, task.1 .0 - task.0 .0), *pid));
        }

        /* Format the most important tasks */
        for i in 0..self.relevant.len() {
            self.relevant[i].clear();
            let taskid = match self.sorted.pop() {
                Some((_, t)) => t,
                _ => continue, /* Out of tasks, keep clearing strings anyway */
            };
            self.format_task(taskid, i);
        }
    }
}

impl<'a> fmt::Display for TaskStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {:1} {:4} {:<}{}",
            MaybeSmart(Heading("PID"), self.settings),
            MaybeSmart(Heading("S"), self.settings),
            MaybeSmart(Heading("CPU%"), self.settings),
            MaybeSmart(Heading("COMMAND"), self.settings),
            MaybeSmart(Newline(), self.settings)
        )?;

        for s in self.relevant.iter() {
            f.write_str(s)?;
        }
        Ok(())
    }
}
