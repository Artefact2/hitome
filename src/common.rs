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

use argh::FromArgs;
use std::cmp::Ordering;
use std::fmt::{Display, Formatter, Result};
use std::fs::File;
use std::io::Read;

pub const MIN_COL_WIDTH: usize = 8;

#[derive(FromArgs)]
/// A very simple, non-interactive system monitor
pub struct CLI {
    #[argh(option, short = 'c')]
    /// true/false: use colour and other fancy escape sequences (defaults to guessing based on $TERM)
    pub colour: Option<bool>,

    #[argh(option, short = 'w', default = "8")]
    /// the width of columns, in characters
    pub column_width: usize,

    #[argh(option, short = 'i', default = "2000")]
    /// refresh interval in milliseconds
    pub refresh_interval: u64,
}

pub struct Settings {
    pub smart: bool,
    pub colwidth: usize,
    pub refresh: u64,
}

pub fn newline(smart: bool) -> &'static str {
    if smart {
        "\x1B[0K\n"
    } else {
        "\n"
    }
}

pub fn headings(smart: bool) -> (&'static str, &'static str) {
    if smart {
        ("\x1B[1m", "\x1B[0m")
    } else {
        ("", "")
    }
}

pub trait StatBlock<'a> {
    fn new(s: &'a Settings) -> Self;
    fn update(&mut self);
}

#[derive(PartialEq, PartialOrd, Clone, Copy)]
pub struct Bytes(pub u64);

impl Display for Bytes {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let w = f.width().unwrap_or(8) - 1;
        if self.0 >= 10000 * 1024 * 1024 * 1024 {
            write!(
                f,
                "{:>w$.2}T",
                self.0 as f32 / (1024. * 1024. * 1024. * 1024.)
            )
        } else if self.0 >= 10000 * 1024 * 1024 {
            write!(f, "{:>w$.2}G", self.0 as f32 / (1024. * 1024. * 1024.))
        } else if self.0 >= 10000 * 1024 {
            write!(f, "{:>w$.2}M", self.0 as f32 / (1024. * 1024.))
        } else {
            write!(f, "{:>w$.2}K", self.0 as f32 / 1024.)
        }
    }
}

#[derive(PartialEq, PartialOrd, Clone, Copy)]
pub struct Percentage(pub f32);

impl Display for Percentage {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let w = f.width().unwrap_or(8) - 1;
        write!(f, "{:>w$.2}%", self.0)
    }
}

/// Generates coloured values when printing, if the value is above defined thresholds
#[derive(Clone, Copy)]
pub struct Threshold<T> {
    pub val: T,
    pub med: T,
    pub high: T,
    pub crit: T,
    /// If false, don't do any colouring
    pub smart: bool,
}

impl<T> Display for Threshold<T>
where
    T: Display + PartialOrd,
{
    fn fmt(&self, f: &mut Formatter) -> Result {
        let w = f.width().unwrap_or(8);

        if !self.smart || self.val.partial_cmp(&self.med) == Some(Ordering::Less) {
            /* < med or dumb terminal */
            return write!(f, "{:w$}", self.val);
        }

        if self.val.partial_cmp(&self.high) == Some(Ordering::Less) {
            /* < high: we're med */
            write!(f, "\x1B[1;93m{:w$}\x1B[0m", self.val)
        } else if self.val.partial_cmp(&self.crit) == Some(Ordering::Less) {
            /* < crit: we're high */
            write!(f, "\x1B[1;91m{:w$}\x1B[0m", self.val)
        } else {
            /* crit */
            write!(f, "\x1B[1;95m{:w$}\x1B[0m", self.val)
        }
    }
}

#[derive(PartialEq, Eq)]
pub struct Stale(pub bool);

/// Helper function similar to std::fs::read_to_string() that allows reusing the buffer
/* XXX: don't panic on invalid utf8 */
pub fn read_to_string<P: AsRef<std::path::Path>>(p: P, s: &mut String) -> std::io::Result<usize> {
    s.clear();
    let mut f = File::open(p)?;
    f.read_to_string(s)
}

/// Merge two StatBlocks side by side, if the combined result fits in 80 columns or fewer. For each
/// block, assumes that all lines print the same number of visible characters.
pub struct MergedStatBlock<'a, T, U>
where
    T: StatBlock<'a> + Display,
    U: StatBlock<'a> + Display,
{
    t: T,
    u: U,
    tbuf: String,
    ubuf: String,
    /* We need to know colwidth when joining */
    settings: &'a Settings,
}

/* XXX: is there a way to not repeat where clauses in every impl? */
impl<'a, T, U> StatBlock<'a> for MergedStatBlock<'a, T, U>
where
    T: StatBlock<'a> + Display,
    U: StatBlock<'a> + Display,
{
    fn new(s: &'a Settings) -> MergedStatBlock<T, U> {
        MergedStatBlock {
            t: T::new(s),
            u: U::new(s),
            tbuf: String::new(),
            ubuf: String::new(),
            settings: s,
        }
    }

    fn update(&mut self) {
        use std::fmt::Write;

        self.t.update();
        self.tbuf.clear();
        write!(self.tbuf, "{}", self.t).unwrap();

        self.u.update();
        self.ubuf.clear();
        write!(self.ubuf, "{}", self.u).unwrap()
    }
}

impl<'a, T, U> Display for MergedStatBlock<'a, T, U>
where
    T: StatBlock<'a> + Display,
    U: StatBlock<'a> + Display,
{
    fn fmt(&self, f: &mut Formatter) -> Result {
        if self.tbuf.is_empty() {
            return write!(f, "{}", self.ubuf);
        }
        if self.ubuf.is_empty() {
            return write!(f, "{}", self.tbuf);
        }

        let widths = [&self.tbuf, &self.ubuf]
            .map(|s| ascii_term_printable_chars_len(s.lines().nth(0).unwrap()));
        /* Round first width to line up columns */
        let wfirst =
            widths[0] + self.settings.colwidth - (widths[0] % (self.settings.colwidth + 1));

        /* XXX: make the 80 col limit user configurable and/or guess based on terminal */
        if wfirst + 1 + widths[1] > 80 {
            /* Too wide, fall back to printing vertically */
            return write!(f, "{}{}", self.tbuf, self.ubuf);
        }

        let newline = newline(self.settings.smart);
        let mut iters = (self.tbuf.lines(), self.ubuf.lines());
        loop {
            match (iters.0.next(), iters.1.next()) {
                (Some(a), Some(b)) => write!(
                    f,
                    "{:len$} {}{}",
                    a,
                    b,
                    newline,
                    len = wfirst + a.len() - ascii_term_printable_chars_len(a),
                )?,
                (None, Some(b)) => write!(f, "{:wfirst$} {}{}", "", b, newline)?,
                (Some(a), None) => write!(f, "{}{}", a, newline)?,
                _ => break,
            }
        }
        Ok(())
    }
}

/// Length of a string, minus unprintable characters (eg terminal escape sequences)
/// Will panic if fed non-ASCII stuff
/// XXX: probably can be rewritten much more simply
fn ascii_term_printable_chars_len(s: &str) -> usize {
    let mut i = 0;
    let mut iter = s.chars();
    while let Some(c) = iter.next() {
        if !c.is_ascii() {
            unimplemented!();
        }

        /* https://en.wikipedia.org/wiki/ANSI_escape_code#Description */
        if c == '\x1B' {
            let c = iter.next().unwrap();
            if c == '[' {
                /* Gobble up ESC [ (...) 0x40..=0x7E */
                while let Some(c) = iter.next() {
                    if c >= '\x40' && c <= '\x7E' {
                        break;
                    }
                }
            } else {
                unimplemented!();
            }
        } else if !c.is_ascii_control() {
            i += 1
        }
    }
    i
}
