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

use std::cmp::Ordering;
use std::fmt;

pub const MIN_COL_WIDTH: usize = 8;

pub struct Settings {
    /// Terminal understands ansi colour/escape sequences?
    pub smart: bool,
    pub colwidth: usize,
    /// Refresh interval in ms
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

#[derive(PartialEq, PartialOrd, Clone, Copy)]
pub struct Bytes(pub u64);

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
pub struct Percentage(pub f32);

impl fmt::Display for Percentage {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:>7.2}%", self.0)
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
