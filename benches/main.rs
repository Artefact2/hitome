#![feature(test)]
extern crate test;
use hitome::common::{Settings, StatBlock};
use hitome::fs::FilesystemStats;
use hitome::tasks::TaskStats;
use test::Bencher;

#[bench]
fn bench_tasks(b: &mut Bencher) {
    let s: Settings = Default::default();
    let mut t = TaskStats::new(&s);
    b.iter(|| t.update());
}

#[bench]
fn bench_filesystems(b: &mut Bencher) {
    let s: Settings = Default::default();
    let mut fs = FilesystemStats::new(&s);
    b.iter(|| fs.update());
}
