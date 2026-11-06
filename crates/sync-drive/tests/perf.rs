//! 性能测量，手动跑：
//!     cargo test --release --test perf -- --ignored --nocapture

use std::future::Future;
use std::hint::black_box;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use sync_drive::{StackRuntime, SuspendHandle};

/// 第一次 poll 返回 Pending 并立即 wake，第二次 Ready。
struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// 返回 (对照组总耗时, 实验组总耗时)，单位 ns。
fn measure_pair(iterations: usize) -> (u128, u128) {
    // 对照组：零 Pending
    let start = Instant::now();
    for _ in 0..iterations {
        let rt = StackRuntime::new(|ctx| ctx.block_on(black_box(std::future::ready(())))).unwrap();
        futures::executor::block_on(rt);
    }
    let baseline = start.elapsed().as_nanos();

    // 实验组：一次 Pending
    let start = Instant::now();
    for _ in 0..iterations {
        let rt = StackRuntime::new(|ctx| ctx.block_on(black_box(YieldOnce(false)))).unwrap();
        futures::executor::block_on(rt);
    }
    let with_suspend = start.elapsed().as_nanos();

    (baseline, with_suspend)
}

/// 多次重复取中位数，压噪声。
fn measure_suspend_cost(iterations: usize, repeats: usize) -> Duration {
    let _ = measure_pair(100);

    let mut samples = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let (a, b) = measure_pair(iterations);
        let diff = b.saturating_sub(a);
        samples.push(diff / iterations as u128);
    }
    samples.sort_unstable();
    Duration::from_nanos(u64::try_from(samples[repeats / 2]).unwrap_or(u64::MAX))
}

#[test]
#[ignore = "性能测量，手动跑：cargo test --release --test perf -- --ignored --nocapture"]
fn suspend_resume_cost() {
    let cost = measure_suspend_cost(10_000, 5);
    println!("单次 suspend/resume 成本: {cost:?}");
}
