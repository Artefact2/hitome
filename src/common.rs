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
use std::cell::Cell;
use std::cmp::Ordering;
use std::fmt::{Alignment, Display, Formatter, Result};
use std::fs::File;
use std::io::Read;

const SMART_NEWLINE: &str = "\x1B[0K";

#[derive(FromArgs)]
/// A very simple, non-interactive system monitor
pub struct Cli {
    #[argh(option, short = 'c')]
    /// true/false: use colour and other fancy escape sequences (defaults to guessing based on $TERM)
    pub colour: Option<bool>,

    #[argh(option)]
    /// width of the terminal window, in characters (if omitted, guess)
    pub columns: Option<u16>,

    #[argh(option)]
    /// height of the terminal window, in lines (if omitted, guess)
    pub rows: Option<u16>,

    #[argh(option, short = 'w')]
    /// the width of columns, in characters
    pub column_width: Option<u16>,

    #[argh(option, short = 'i', default = "2000")]
    /// refresh interval in milliseconds
    pub refresh_interval: u64,
}

pub struct Settings {
    pub smart: bool,
    pub refresh: u64,
    pub auto_colwidth: bool,
    pub auto_maxcols: bool,
    pub auto_maxrows: bool,
    /* These settings can change at runtime, usage of a Cell required to provide interior
     * mutability */
    pub maxcols: Cell<u16>,
    pub maxrows: Cell<u16>,
    pub colwidth: Cell<u16>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            smart: false,
            refresh: 2000,
            auto_colwidth: false,
            auto_maxcols: false,
            auto_maxrows: false,
            maxcols: Cell::new(120),
            maxrows: Cell::new(50),
            colwidth: Cell::new(10),
        }
    }
}

pub trait StatBlock<'a> {
    fn new(s: &'a Settings) -> Self;
    fn update(&mut self);

    /// The width of any non-empty line that would be printed if this block were Displayed
    fn columns(&self) -> u16;
    /// The number of lines that would be printed if this block were Displayed
    fn rows(&self) -> u16;
}

#[derive(PartialEq, PartialOrd, Clone, Copy)]
pub struct Bytes(pub u64);

impl Display for Bytes {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let w = f.width().unwrap_or(8) - 1;
        let p = f.precision().unwrap_or(2);
        let threshold = if p == 0 {
            if w >= 4 {
                10000
            } else {
                1000
            }
        } else {
            if w - p - 1 >= 4 {
                10000
            } else {
                1000
            }
        };
        if self.0 >= threshold * 1024 * 1024 * 1024 {
            write!(
                f,
                "{:>w$.p$}T",
                self.0 as f32 / (1024. * 1024. * 1024. * 1024.)
            )
        } else if self.0 >= threshold * 1024 * 1024 {
            write!(f, "{:>w$.p$}G", self.0 as f32 / (1024. * 1024. * 1024.))
        } else if self.0 >= threshold * 1024 {
            write!(f, "{:>w$.p$}M", self.0 as f32 / (1024. * 1024.))
        } else if self.0 > 0 {
            write!(f, "{:>w$.p$}K", self.0 as f32 / 1024.)
        } else {
            let w = w + 1;
            write!(f, "{:>w$}", ".")
        }
    }
}

#[derive(PartialEq, PartialOrd, Clone, Copy)]
pub struct Watts(pub u64);

impl Display for Watts {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let w = f.width().unwrap_or(8) - 1;
        let p = f.precision().unwrap_or(0);
        write!(f, "{:>w$.p$}W", self.0)
    }
}

#[derive(PartialEq, PartialOrd, Clone, Copy)]
pub struct Percentage(pub f32);

impl Display for Percentage {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let w = f.width().unwrap_or(8) - 1;
        let p = f.precision().unwrap_or(2);
        write!(f, "{:>w$.p$}%", self.0)
    }
}

#[derive(Clone, Copy)]
pub struct Threshold<T> {
    pub val: T,
    pub med: T,
    pub high: T,
    pub crit: T,
}

pub struct Heading<'a>(pub &'a str);
pub struct Newline();

/// A wrapper type to access settings in fmt::Display
pub struct MaybeSmart<'a, T>(pub T, pub &'a Settings);

impl<'a, 'b> Display for MaybeSmart<'a, Heading<'b>> {
    fn fmt(&self, f: &mut Formatter) -> Result {
        let w = f.width().unwrap_or_else(|| self.1.colwidth.get().into());
        let p = f.precision().unwrap_or(w);

        /* XXX: is there a way to not repeat ourselves? */
        match (self.1.smart, f.align()) {
            (false, Some(Alignment::Center)) => write!(f, "{:^w$.p$}", self.0 .0),
            (false, Some(Alignment::Left)) => write!(f, "{:<w$.p$}", self.0 .0),
            (false, _) => write!(f, "{:>w$.p$}", self.0 .0),
            (true, Some(Alignment::Center)) => write!(f, "\x1B[1m{:^w$.p$}\x1B[0m", self.0 .0),
            (true, Some(Alignment::Left)) => write!(f, "\x1B[1m{:<w$.p$}\x1B[0m", self.0 .0),
            (true, _) => write!(f, "\x1B[1m{:>w$.p$}\x1B[0m", self.0 .0),
        }
    }
}

impl<'a> Display for MaybeSmart<'a, Newline> {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self.1.smart {
            false => writeln!(f),
            true => writeln!(f, "{}", SMART_NEWLINE),
        }
    }
}

impl<'a, T> Display for MaybeSmart<'a, Threshold<T>>
where
    T: Display + PartialOrd,
{
    fn fmt(&self, f: &mut Formatter) -> Result {
        let w = f.width().unwrap_or_else(|| self.1.colwidth.get().into());
        let p = f.precision().unwrap_or(2);
        let t = &self.0;

        if !self.1.smart {
            return write!(f, "{:>w$.p$}", t.val);
        }

        if t.val.partial_cmp(&t.med) == Some(Ordering::Less) {
            /* < med */
            write!(f, "{:>w$.p$}", t.val)
        } else if t.val.partial_cmp(&t.high) == Some(Ordering::Less) {
            /* < high: we're med */
            write!(f, "\x1B[1;93m{:>w$.p$}\x1B[0m", t.val)
        } else if t.val.partial_cmp(&t.crit) == Some(Ordering::Less) {
            /* < crit: we're high */
            write!(f, "\x1B[1;91m{:>w$.p$}\x1B[0m", t.val)
        } else {
            /* crit */
            write!(f, "\x1B[1;95m{:>w$.p$}\x1B[0m", t.val)
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct Stale(pub bool);

/// Read contents of a file to a given String buffer
/// # Safety
/// Make sure the file you are reading can never contain bad UTF-8
pub unsafe fn read_to_string_unchecked<P: AsRef<std::path::Path>>(
    p: P,
    s: &mut String,
) -> std::io::Result<usize> {
    s.clear();
    let mut f = File::open(p)?;
    f.read_to_end(s.as_mut_vec())
}

/// Read contents of a file and mangle it into valid UTF-8
pub fn read_to_string<P: AsRef<std::path::Path>>(p: P, s: &mut String) -> std::io::Result<usize> {
    const REPLACEMENT_CHAR: u8 = b'?';

    fn check_byte(b: Option<&mut u8>) -> Option<&mut u8> {
        match b {
            Some(b) if *b <= 0b10111111 => Some(b),
            Some(b) => {
                *b = REPLACEMENT_CHAR;
                None
            }
            _ => None,
        }
    }

    unsafe {
        let length = read_to_string_unchecked(p, s)?;
        /* Now s may contain invalid UTF-8, iterate over the bytes and correct that to make a safe
         * String */
        /* XXX: would be nice to leverage String::from_utf8_lossy() or OsString::to_string_lossy(),
         * but they don't work in-place so are not suited here */
        let mut iter = s.as_mut_vec().iter_mut();
        #[allow(clippy::while_let_on_iterator)]
        while let Some(cp) = iter.next() {
            /* This is very naive, probably buggy and slow */
            /* https://doc.rust-lang.org/std/primitive.char.html#validity */

            if *cp <= 0b01111111 {
                /* Was an ASCII code point */
                continue;
            }

            if *cp >= 0b11111000 {
                /* Invalid leader */
                *cp = REPLACEMENT_CHAR;
                continue;
            }

            let a = match check_byte(iter.next()) {
                Some(a) => a,
                None => {
                    *cp = REPLACEMENT_CHAR;
                    continue;
                }
            };

            if *cp < 0b11100000 {
                /* Was a 2-byte sequence */
                continue;
            }

            let b = match check_byte(iter.next()) {
                Some(b) => b,
                None => {
                    *cp = REPLACEMENT_CHAR;
                    *a = REPLACEMENT_CHAR;
                    continue;
                }
            };

            if *cp < 0b11110000 {
                /* Was a 3-byte sequence */

                /* Check for 0xD800..0xE000 codepoint */
                let first_byte = ((*cp & 0b00001111) << 4) | ((*a & 0b00111100) >> 2);
                if (0xD8..0xE0).contains(&first_byte) {
                    *cp = REPLACEMENT_CHAR;
                    *a = REPLACEMENT_CHAR;
                    *b = REPLACEMENT_CHAR;
                }

                continue;
            }

            /* Is a 4-byte sequence */

            let c = match check_byte(iter.next()) {
                Some(c) => c,
                None => {
                    *cp = REPLACEMENT_CHAR;
                    *a = REPLACEMENT_CHAR;
                    *b = REPLACEMENT_CHAR;
                    continue;
                }
            };

            /* Check for 0x110000.. codepoint */
            let first_byte = ((*cp & 0b00000111) << 2) | ((*a & 0b00110000) >> 4);
            if first_byte >= 0x11 {
                *cp = REPLACEMENT_CHAR;
                *a = REPLACEMENT_CHAR;
                *b = REPLACEMENT_CHAR;
                *c = REPLACEMENT_CHAR;
            }
        }
        Ok(length)
    }
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
impl<'a, T, U> MergedStatBlock<'a, T, U>
where
    T: StatBlock<'a> + Display,
    U: StatBlock<'a> + Display,
{
    fn pad_length_to_columns(&self, len: u16) -> u16 {
        if len > 0 {
            len + self.settings.colwidth.get() - len % (self.settings.colwidth.get() + 1)
        } else {
            0
        }
    }

    fn can_merge(&self) -> bool {
        self.pad_length_to_columns(self.t.columns()) + self.u.columns()
            < self.settings.maxcols.get()
    }
}

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

    fn columns(&self) -> u16 {
        let tc = self.t.columns();
        let tu = self.u.columns();

        /* Space separator only needed if t and u print something */
        if self.can_merge() && tc > 0 && tu > 0 {
            self.pad_length_to_columns(tc) + tu + 1
        } else {
            tc.max(tu)
        }
    }

    fn rows(&self) -> u16 {
        if self.can_merge() {
            self.t.rows().max(self.u.rows())
        } else {
            /* If t or u prints nothing, we can merge; control flow reaches here only if t and u
             * print something, so the newline separator is always there */
            self.t.rows() + self.u.rows() + 1
        }
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

        if !self.can_merge() {
            return write!(f, "{}{}", self.tbuf, self.ubuf);
        }

        let tc = self.t.columns();
        let uc = self.u.columns() as usize;
        let tc_padded = self.pad_length_to_columns(tc) as usize;
        let padding = tc_padded - tc as usize;
        let newline = MaybeSmart(Newline(), self.settings);
        let mut iters = (self.tbuf.lines(), self.ubuf.lines());
        loop {
            let a = iters.0.next();
            let b = iters.1.next();

            if a == None && b == None {
                break;
            }

            let a = a.unwrap_or("");
            let b = b.unwrap_or("");

            /* Make sure we always print lines .columns() characters long */
            if a.is_empty() || a == SMART_NEWLINE {
                write!(f, "{:tc_padded$} ", "")?
            } else {
                write!(f, "{}{:padding$} ", a, "")?
            }

            if b.is_empty() || b == SMART_NEWLINE {
                write!(f, "{:uc$}{}", "", newline)?
            } else {
                write!(f, "{}{}", b, newline)?
            }
        }
        Ok(())
    }
}

pub fn libc_panic(msg: &'static str) -> ! {
    let msg = std::ffi::CString::new(msg).unwrap();
    unsafe { libc::perror(msg.as_ptr()) };
    panic!();
}
