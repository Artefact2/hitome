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
use fnv::FnvHashMap;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fmt;
use std::fmt::Write;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Linux PIDs should not go above 2^22, says proc(5)
struct Pid(u32);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
/// 1 jiffie = 1/user_hz seconds; (jiffies_used, system_uptime)
struct Jiffies(u64, u64);

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

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
struct CPUPercentage(u8);

impl fmt::Display for CPUPercentage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let w = f.width().unwrap_or(4) - 1;
        write!(f, "{:>w$.0}%", self.0)
    }
}

#[derive(PartialEq, Eq)]
struct TaskSort(TaskState, CPUPercentage);

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

struct TaskEntry {
    /// If >0, opened file descriptor of /proc/pid/task/pid/stat
    filedes: libc::c_int,
    jiffies: (Jiffies, Jiffies),
    state: TaskState,
    stale: Stale,
}

pub struct TaskStats<'a> {
    settings: &'a Settings,
    /// How many jiffies in a second, as exposed to userspace
    user_hz: u16,
    /// System uptime in jiffies
    uptime: u64,
    /// Hopefully near-ish time elapsed since uptime was updated
    since_uptime: Instant,
    buf: String,
    buf2: String,
    buf3: String,
    bufp: PathBuf,
    bufp2: PathBuf,
    tasks: FnvHashMap<Pid, TaskEntry>,
    /// Used to sort tasks by their State/CPU%. Pushing is O(1) and popping is O(log n). Pushing all
    /// the tasks and popping the 10 highest is only O(n + 10 log n) instead of sorting which is O(n
    /// log n).
    sorted: BinaryHeap<(TaskSort, Pid)>,
    /// Formatted and ordered lines, ready to be printed
    relevant: Vec<String>,
    /// How many tasks we can print
    maxtasks: u16,
}

/// Walk /proc and call the closure for each task, eg /proc/X/task/Y. Skips invalid files instead of
/// panicking, as tasks are created/deleted all the time and scanning them in /proc is inherently
/// racy. XXX: this would work better as an Iterator, but i don't know how to do that
fn map_tasks<F>(buf: &mut String, p: &mut PathBuf, pi: &mut PathBuf, mut doit: F)
where
    F: FnMut(&str),
{
    /* XXX: find if io_uring is worth using here */
    /* XXX: same, but with inotify watches */
    p.clear();
    p.push("/proc");
    pi.clear();
    pi.push("/proc");

    for process in std::fs::read_dir("/proc").unwrap() {
        let process = match process {
            Ok(p) => p,
            _ => continue,
        };

        match process.file_type() {
            Ok(f) if f.is_dir() => (),
            _ => continue,
        }

        p.push(process.file_name());
        p.push("task");
        pi.push(process.file_name());
        pi.push("task");

        /* XXX: this is glorified goto to avoid repeating the popping in case of an early break. Is
         * there a better solution? */
        #[allow(clippy::never_loop)]
        loop {
            for task in match std::fs::read_dir(&p) {
                Ok(a) => a,
                _ => break,
            } {
                let task = match task {
                    Ok(p) => p,
                    _ => continue,
                };

                pi.push(task.file_name());
                pi.push("stat");
                /* XXX: cache the file descriptor */
                if read_to_string(&pi, buf).is_ok() {
                    doit(buf);
                }
                pi.pop();
                pi.pop();
            }
            break;
        }

        p.pop();
        p.pop();
        pi.pop();
        pi.pop();
    }
}

impl<'a> TaskStats<'a> {
    pub fn set_max_tasks(&mut self, tasks: u16) {
        self.maxtasks = tasks;
    }

    // XXX: this would be much simpler as a method that mutates self, but the borrow checker won't
    // let us do that since we already take a &TaskEntry argument
    /// Format a task's line to out String
    fn format_task(
        settings: &Settings,
        buf: &mut String,
        buf2: &mut String,
        buf3: &mut String,
        out: &mut String,
        taskid: Pid,
        cpupc: CPUPercentage,
        ent: &TaskEntry,
    ) {
        /* XXX: find better way to do this */
        buf2.clear();
        write!(buf2, "/proc/{}/task/{}/cmdline", taskid.0, taskid.0).unwrap();
        let cmdline = match read_to_string(&buf2, buf) {
            Ok(_) => &buf,
            _ => "",
        };

        buf2.clear();
        write!(buf2, "/proc/{}/task/{}/comm", taskid.0, taskid.0).unwrap();
        let comm = match read_to_string(&buf2, buf3) {
            Ok(_) => buf3.strip_suffix('\n').unwrap(),
            _ => "",
        };

        /* Format the cmdline: skip path of argv[0], split args by spaces */
        let max_length = (settings.maxcols.get() - settings.colwidth.get() - 8).into();
        let mut cmdline = cmdline.split('\0');
        let progname = cmdline.next().unwrap_or("");
        let progname = match progname.rsplit_once('/') {
            Some((_, p)) => p,
            _ => progname,
        };

        buf2.clear();
        for arg in cmdline {
            if buf2.len() >= max_length {
                break;
            }

            /* Some half-assed shell-like escaping, should cover most cases, doesn't need to be
             * perfect since it will be truncated anyway */
            match arg.contains(' ') {
                false => write!(buf2, "{} ", arg).unwrap(),
                true => match arg.contains('\'') {
                    false => write!(buf2, "'{}' ", arg).unwrap(),
                    /* XXX: creating a new String here may not be a good idea, hopefully this case is rare */
                    true => write!(buf2, "'{}' ", arg.replace('\\', "\\'")).unwrap(),
                },
            }
        }

        let newline = MaybeSmart(Newline(), settings);
        let w = settings.colwidth.get().into();

        write!(
            out,
            "{:>w$} {:1} {:>4} {:<max_length$}{}",
            taskid.0,
            MaybeSmart(ent.state, settings),
            MaybeSmart(
                Threshold {
                    val: cpupc,
                    med: CPUPercentage(40),
                    high: CPUPercentage(60),
                    crit: CPUPercentage(80),
                },
                settings
            ),
            MaybeSmart(CommandLine(comm, progname, &buf2), settings),
            newline
        )
        .unwrap();
    }
}

impl<'a> StatBlock<'a> for TaskStats<'a> {
    fn new(s: &'a Settings) -> Self {
        let mut ts = TaskStats {
            settings: s,
            user_hz: unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as u16,
            buf: String::new(),
            buf2: String::new(),
            buf3: String::new(),
            bufp: Default::default(),
            bufp2: Default::default(),
            tasks: FnvHashMap::default(),
            sorted: BinaryHeap::new(),
            relevant: Default::default(),
            maxtasks: 10,
            uptime: 0,
            since_uptime: Instant::now(),
        };
        ts.update();
        ts
    }

    fn update(&mut self) {
        /* Measure and store jiffies of each task in self.tasks */
        for t in self.tasks.values_mut() {
            t.stale = Stale(true);
        }

        self.since_uptime = Instant::now();
        /* /proc/uptime is never exposed to user data */
        unsafe { read_to_string_unchecked("/proc/uptime", &mut self.buf) }.unwrap();
        self.uptime = (self
            .buf
            .split_ascii_whitespace()
            .next()
            .unwrap()
            .parse::<f32>()
            .unwrap()
            * 100.0) as u64
            * self.user_hz as u64
            / 100;

        map_tasks(&mut self.buf, &mut self.bufp, &mut self.bufp2, |stat| {
            let uptime = self.uptime
                + self.since_uptime.elapsed().as_millis() as u64 * self.user_hz as u64 / 1000;

            /* See https://www.kernel.org/doc/html/latest/filesystems/proc.html table 1-4 */
            /* And proc(5) */
            let taskid = Pid(stat.split_once(' ').unwrap().0.parse::<u32>().unwrap());
            let mut stat = stat.rsplit_once(')').unwrap().1.split_ascii_whitespace();
            let state = match stat.next().unwrap() {
                "S" => TaskState::Sleeping,
                "R" => TaskState::Running,
                "D" => TaskState::Uninterruptible,
                "Z" => TaskState::Zombie,
                "T" => TaskState::Traced,
                "I" => TaskState::Idle,
                _ => TaskState::Unknown,
            };
            let used_jiffies = stat.nth(10).unwrap().parse::<u64>().unwrap()
                + stat.next().unwrap().parse::<u64>().unwrap();

            let mut ent = match self.tasks.get_mut(&taskid) {
                Some(e) => e,
                _ => {
                    let start_time = stat.nth(6).unwrap().parse::<u64>().unwrap();
                    let z = TaskEntry {
                        filedes: 0,
                        jiffies: (Jiffies(0, 0), Jiffies(0, start_time)),
                        state: TaskState::Sleeping,
                        stale: Stale(false),
                    };
                    self.tasks.insert(taskid, z);
                    self.tasks.get_mut(&taskid).unwrap()
                }
            };

            ent.jiffies.0 = ent.jiffies.1;
            ent.jiffies.1 = Jiffies(used_jiffies, uptime);
            ent.state = state;
            ent.stale = Stale(false);
        });
        self.tasks.retain(|_, t| t.stale == Stale(false));

        /* Sort tasks by state/cpu% */
        self.sorted.clear();
        for (pid, task) in self.tasks.iter() {
            if task.jiffies.0 .1 >= task.jiffies.1 .1 {
                continue;
            }
            self.sorted.push((
                TaskSort(
                    task.state,
                    CPUPercentage(
                        (100 * (task.jiffies.1 .0 - task.jiffies.0 .0)
                            / (task.jiffies.1 .1 - task.jiffies.0 .1))
                            as u8,
                    ),
                ),
                *pid,
            ));
        }

        for s in self.relevant.iter_mut() {
            s.clear();
        }
        if (self.relevant.len() as u16) < self.maxtasks {
            let n = self.maxtasks as usize - self.relevant.len();
            self.relevant.reserve(n);
            for _ in 0..n {
                self.relevant
                    .push(String::with_capacity(self.settings.maxcols.get() as usize));
            }
        }

        /* Format the most important tasks */
        for i in 0..(self.maxtasks as usize) {
            let (tasksort, taskid) = match self.sorted.pop() {
                Some(x) => x,
                _ => break,
            };
            if tasksort.0 == TaskState::Sleeping && tasksort.1 .0 == 0 {
                /* Ran out of interesting tasks */
                break;
            }
            let ent = self.tasks.get(&taskid).unwrap();
            Self::format_task(
                self.settings,
                &mut self.buf,
                &mut self.buf2,
                &mut self.buf3,
                &mut self.relevant[i],
                taskid,
                tasksort.1,
                ent,
            );
        }
    }

    fn columns(&self) -> u16 {
        self.settings.maxcols.get()
    }

    fn rows(&self) -> u16 {
        1 + self.maxtasks
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
