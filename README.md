hitome
======

`hitome` is a quick and dirty system monitor that aims to be light on
system resources. It aims to be a lighter version of
[`glances`](https://github.com/nicolargo/glances).

Released under the Apache License, version 2.0.

Features
========

Monitors memory usage, swap/zram usage, system pressure information,
usage of CPU cores, traffic to and from block devices and network
interfaces and processes' status and CPU usage.

This is not meant to be a full-blown `top/htop` replacement, use these
tools instead if you want more features.

Dependencies
============

* PHP (CLI only)
* Linux kernel

Installation
============

* Clone the repository or download the `hitome` file.
* Add this directory to your `$PATH` or copy/symlink `hitome` to `/usr/local/bin`.
