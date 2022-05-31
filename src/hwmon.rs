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
use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write;
use std::path::PathBuf;

#[derive(Copy, Clone, PartialEq, PartialOrd)]
pub struct Celsius(f32);

impl fmt::Display for Celsius {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let w = f.width().unwrap_or(8) - 1;
        let p = f.precision().unwrap_or(1);
        write!(f, "{:>w$.p$}C", self.0)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Hwmon(usize),
    Nvml(usize),
}

pub struct HwmonStats<'a> {
    settings: &'a Settings,
    /// hwmonX -> label, (label, value)...
    state: BTreeMap<Kind, (String, BTreeMap<String, (Celsius, Stale)>, Stale)>,
    nvml: Option<nvml_wrapper::Nvml>,
    // internal buffers re-used in update()
    p: PathBuf,
    sb: String,
    sb2: String,
}

impl<'a> StatBlock<'a> for HwmonStats<'a> {
    fn new(s: &'a Settings) -> Self {
        Self {
            settings: s,
            state: Default::default(),
            nvml: nvml_wrapper::Nvml::init().ok(),
            p: PathBuf::from("/sys/class/hwmon"),
            sb: Default::default(),
            sb2: Default::default(),
        }
    }

    fn update(&mut self) {
        for (_, s) in self.state.iter_mut() {
            s.2 = Stale(true);
        }

        if let Ok(monitors) = std::fs::read_dir("/sys/class/hwmon") {
            for m in monitors {
                let m = match m {
                    Ok(m) => m,
                    _ => continue,
                };

                /* XXX: feels clunky */
                let x = match m.file_name().to_str().unwrap()[5..].parse::<usize>() {
                    Ok(k) => Kind::Hwmon(k),
                    _ => continue,
                };

                let ent = match self.state.get_mut(&x) {
                    Some(ent) => ent,
                    None => {
                        let z = (String::new(), Default::default(), Stale(false));
                        self.state.insert(x, z);
                        self.state.get_mut(&x).unwrap()
                    }
                };
                ent.2 = Stale(false);

                for (_, v) in ent.1.iter_mut() {
                    v.1 = Stale(true);
                }

                self.p.push(m.file_name());

                // Update name
                // XXX: not very descriptive, esp. for nvme
                self.p.push("name");
                ent.0.clear();
                unsafe { read_to_string_unchecked(&self.p, &mut ent.0) }.unwrap();
                self.p.pop();
                ent.0.pop(); // Remove terminating \n

                // Read /sys/class/hwmonX/tempY_{label,input} while they exist
                let mut y = 1;
                loop {
                    self.sb2.clear();
                    self.p.push(format!("temp{}_input", y));
                    let input = unsafe { read_to_string_unchecked(&self.p, &mut self.sb2) };
                    self.p.pop();
                    let input = match input {
                        Ok(_) => {
                            // Remove terminating \n
                            self.sb2.pop();
                            self.sb2.parse::<f32>().unwrap()
                        }
                        _ => break,
                    };

                    self.sb.clear();
                    self.p.push(format!("temp{}_label", y));
                    let label = unsafe { read_to_string_unchecked(&self.p, &mut self.sb) };
                    self.p.pop();
                    if !label.is_ok() {
                        // No label, this is OK
                        self.sb.clear();
                        write!(self.sb, "Temp{}", y).unwrap();
                    } else {
                        // Remove terminating \n
                        self.sb.pop();
                    }

                    let ent = match ent.1.get_mut(&self.sb) {
                        Some(ent) => ent,
                        None => {
                            ent.1.insert(self.sb.clone(), (Celsius(0f32), Stale(false)));
                            ent.1.get_mut(&self.sb).unwrap()
                        }
                    };
                    ent.0 = Celsius(input / 1000f32);
                    ent.1 = Stale(false);

                    y += 1;
                }

                if ent.1.len() != y {
                    ent.1.retain(|_, v| v.1 == Stale(false));
                }

                self.p.pop();
            }
        }

        if let Some(nvml) = &self.nvml {
            if let Ok(n) = nvml.device_count() {
                for i in 0..n {
                    let device = match nvml.device_by_index(i) {
                        Ok(device) => device,
                        _ => continue,
                    };
                    let k = Kind::Nvml(i as usize);

                    let ent = match self.state.get_mut(&k) {
                        Some(nv) => nv,
                        _ => {
                            let mut z = (String::new(), Default::default(), Stale(false));
                            write!(z.0, "nvidia{}", i).unwrap(); /* XXX: find better name */
                            self.state.insert(k, z);
                            /* XXX: yes, this is stupid. Can't insert above ^ because type inference sucks */
                            let ent = self.state.get_mut(&k).unwrap();
                            ent.1
                                .insert(String::from("Tgpu"), (Celsius(0f32), Stale(false)));
                            ent
                        }
                    };
                    ent.2 = Stale(false);

                    let v = ent.1.get_mut("Tgpu").unwrap();
                    v.0 .0 = match device
                        .temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)
                    {
                        Ok(t) => t as f32,
                        _ => 0f32,
                    }
                }
            }
        }

        self.state.retain(|_, s| s.2 == Stale(false));
    }

    fn columns(&self) -> u16 {
        if self.state.is_empty() {
            0
        } else {
            8
        }
    }

    fn rows(&self) -> u16 {
        if self.state.is_empty() {
            return 0;
        }

        let two_cols = self.state.values().all(|v| v.1.len() <= 3);
        if two_cols {
            1 + (self.state.len() as u16 + 1) / 2
        } else {
            let mut cols = 1;
            for v in self.state.values() {
                cols += (v.1.len() as u16 + 6) / 7
            }
            cols
        }
    }
}

impl<'a> fmt::Display for HwmonStats<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.state.is_empty() {
            return Ok(());
        }

        let newline = MaybeSmart(Newline(), self.settings);
        let w = self.settings.colwidth.get().into();
        let two_cols = self.state.values().all(|v| v.1.len() <= 3);
        let mut used_cols = 0;

        for v in self.state.values() {
            if v.1.is_empty() {
                continue;
            }

            used_cols += 1;
            write!(f, "{:>w$.w$}", v.0)?;

            let mut i = 0;
            for (k, vv) in v.1.iter() {
                if i > 0 && i % 7 == 0 {
                    write!(f, "{}{:>w$.w$}", newline, v.0)?;
                    used_cols = 1;
                }

                let label = MaybeSmart(Heading(k), self.settings);
                let value = MaybeSmart(
                    Threshold {
                        val: vv.0,
                        med: Celsius(50.0),
                        high: Celsius(70.0),
                        crit: Celsius(90.0),
                    },
                    self.settings,
                );

                if w > 10 {
                    let w = w - 6;
                    write!(f, " {:>w$.w$}{:>6.1}", label, value)?;
                } else {
                    let w = w - 4;
                    write!(f, " {:>w$.w$}{:>4.0}", label, value)?;
                }

                i += 1;
                used_cols += 1;
            }

            if two_cols {
                if (1..=4).contains(&used_cols) {
                    for _ in used_cols..4 {
                        write!(f, " {:>w$.w$}", "")?;
                    }
                    write!(f, " ")?;
                    used_cols = 4;
                } else if used_cols > 4 {
                    for _ in used_cols..8 {
                        write!(f, " {:>w$.w$}", "")?;
                    }
                    write!(f, "{}", newline)?;
                    used_cols = 0;
                }
            } else if used_cols > 0 {
                for _ in used_cols..8 {
                    write!(f, " {:>w$.w$}", "")?;
                }
                write!(f, "{}", newline)?;
                used_cols = 0;
            }
        }

        for _ in used_cols..8 {
            write!(f, " {:>w$.w$}", "")?;
        }
        write!(f, "{}{}", newline, newline)
    }
}
