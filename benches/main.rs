#![feature(test)]
extern crate test;
use hitome::common::{Settings, StatBlock};
use hitome::tasks::TaskStats;
use test::Bencher;

#[bench]
fn bench_tasks(b: &mut Bencher) {
    let s: Settings = Default::default();
    let mut t = TaskStats::new(&s);
    b.iter(|| t.update());
}
