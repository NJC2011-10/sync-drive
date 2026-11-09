mod common;

use common::{ManualFuture, extract_panic_msg};
use futures::executor::block_on;
use futures::task::noop_waker;
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};
use sync_drive::{StackRuntime, SuspendHandle};

#[test]
fn three_tasks_join() {
    let a = StackRuntime::new(|_| 1u32).unwrap();
    let b = StackRuntime::new(|_| 2u32).unwrap();
    let c = StackRuntime::new(|_| 3u32).unwrap();
    let (ra, rb, rc) = block_on(futures::future::join3(a, b, c));
    assert_eq!((ra, rb, rc), (1, 2, 3));
}

#[test]
fn three_tasks_arbitrary_wake_order() {
    let ready = [
        Rc::new(Cell::new(false)),
        Rc::new(Cell::new(false)),
        Rc::new(Cell::new(false)),
    ];
    let wakers: [Rc<RefCell<Option<Waker>>>; 3] = [
        Rc::new(RefCell::new(None)),
        Rc::new(RefCell::new(None)),
        Rc::new(RefCell::new(None)),
    ];

    let mk = |i: usize| {
        let r = ready[i].clone();
        let w = wakers[i].clone();
        StackRuntime::new(move |ctx| {
            ctx.block_on(ManualFuture { ready: r, waker: w });
            i
        })
        .unwrap()
    };

    let mut tasks = [Box::pin(mk(0)), Box::pin(mk(1)), Box::pin(mk(2))];
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    for t in &mut tasks {
        assert!(t.as_mut().poll(&mut cx).is_pending());
    }
    for w in &wakers {
        assert!(w.borrow().is_some());
    }

    for i in [1usize, 2, 0] {
        ready[i].set(true);
        for j in 0..3 {
            if j == i || ready[j].get() {
                continue;
            }
            assert!(tasks[j].as_mut().poll(&mut cx).is_pending());
        }
        match tasks[i].as_mut().poll(&mut cx) {
            Poll::Ready(v) => assert_eq!(v, i),
            Poll::Pending => panic!("task {i} should be ready"),
        }
    }
}

#[test]
fn three_tasks_one_panics_others_unaffected() {
    let a = StackRuntime::new(|_| 1u32).unwrap();
    let b = StackRuntime::new(|_| -> u32 { panic!("b panics") }).unwrap();
    let c = StackRuntime::new(|_| 3u32).unwrap();

    let mut a = Box::pin(a);
    let mut b = Box::pin(b);
    let mut c = Box::pin(c);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    let r = catch_unwind(AssertUnwindSafe(|| {
        let _ = b.as_mut().poll(&mut cx);
    }));
    assert!(extract_panic_msg(r.expect_err("b should panic").as_ref()).contains("b panics"));

    assert!(matches!(a.as_mut().poll(&mut cx), Poll::Ready(1)));
    assert!(matches!(c.as_mut().poll(&mut cx), Poll::Ready(3)));

    let r2 = catch_unwind(AssertUnwindSafe(|| {
        let _ = b.as_mut().poll(&mut cx);
    }));
    assert!(
        extract_panic_msg(r2.expect_err("b second poll should panic").as_ref())
            .contains("polled after completion")
    );
}

#[test]
fn three_tasks_one_dropped_others_unaffected() {
    let ready = Rc::new(Cell::new(false));
    let waker_slot: Rc<RefCell<Option<Waker>>> = Rc::new(RefCell::new(None));

    let a = StackRuntime::new(|_| "A").unwrap();
    let b = StackRuntime::new({
        let r = ready.clone();
        let w = waker_slot.clone();
        move |ctx| {
            ctx.block_on(ManualFuture { ready: r, waker: w });
            "B"
        }
    })
    .unwrap();
    let c = StackRuntime::new(|_| "C").unwrap();

    let mut a = Box::pin(a);
    let mut b = Box::pin(b);
    let mut c = Box::pin(c);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    assert!(a.as_mut().poll(&mut cx).is_ready());
    assert!(b.as_mut().poll(&mut cx).is_pending());
    assert!(c.as_mut().poll(&mut cx).is_ready());

    drop(b);
    drop(a);
    drop(c);
}

#[test]
fn concurrent_tasks_overlap_in_time() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();

    let make = || {
        StackRuntime::new(|ctx| {
            ctx.block_on(tokio::time::sleep(Duration::from_millis(200)));
        })
        .unwrap()
    };

    let a = make();
    let b = make();

    let start = Instant::now();
    rt.block_on(async { futures::future::join(a, b).await });
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(300),
        "两个 200ms sleep 应并发执行，实际耗时 {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(200),
        "至少要等一个 200ms sleep，实际 {elapsed:?}"
    );
}

#[test]
fn serial_tasks_take_full_time() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();

    let make = || {
        StackRuntime::new(|ctx| {
            ctx.block_on(tokio::time::sleep(Duration::from_millis(200)));
        })
        .unwrap()
    };

    let a = make();
    let b = make();

    let start = Instant::now();
    rt.block_on(async {
        a.await;
        b.await;
    });
    let elapsed = start.elapsed();

    assert!(
        elapsed >= Duration::from_millis(380),
        "两个 200ms sleep 串行应 ~400ms，实际 {elapsed:?}"
    );
}
