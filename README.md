hitome
======

`hitome` is a quick and dirty system monitor that aims to be light on system
resources. Think of it as a lighter, less featureful version of
[`glances`](https://github.com/nicolargo/glances) or
[`htop`](https://htop.dev/).

`hitome` only targets Linux as it parses non-portable data from `/proc`.

Released under the Apache License, version 2.0.

[![asciicast](https://asciinema.org/a/1QD7pn9Y3UtzmjfujKkEuseYr.svg)](https://asciinema.org/a/1QD7pn9Y3UtzmjfujKkEuseYr)

Features
========

- Monitors memory usage,
- Swap/Zram usage,
- System pressure information (CPU/Mem/IO),
- Usage of each CPU core,
- Traffic to/from block devices and network interfaces,
- Hardware temperatures (as reported by the hwmon interfaces),
- Filesystem usage,
- Tasks (processes) status and CPU utilisation.

This is not meant to be a full-blown `top/htop` replacement, use these
tools instead if you want more features.

Want to improve hitome? Have a look at fixing one of the many
[`XXX`s](https://github.com/Artefact2/hitome/search?q=XXX) present in the
source.

Usage
=====

~~~
% hitome --help
Usage: hitome [-c <colour>] [--columns <columns>] [--rows <rows>] [-w <column-width>] [-i <refresh-interval>]

A very simple, non-interactive system monitor

Options:
  -c, --colour      true/false: use colour and other fancy escape sequences
                    (defaults to guessing based on $TERM)
  --columns         width of the terminal window, in characters (if omitted,
                    guess)
  --rows            height of the terminal window, in lines (if omitted, guess)
  -w, --column-width
                    the width of columns, in characters
  -i, --refresh-interval
                    refresh interval in milliseconds
  --help            display usage information
~~~

Dependencies
============

* Linux kernel
* A rust toolchain (only for building)

Installation
============

1. Clone this repository: `git clone https://github.com/Artefact2/hitome` then `cd hitome`

2. `cargo build -r`

3. Run hitome with `./target/release/hitome` or copy/symlink this file in your
   `$PATH` (eg `/usr/local/bin` or `~/.local/bin`)
