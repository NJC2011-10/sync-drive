mod common;

use common::ManualFuture;
use futures::executor::block_on;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use sync_drive::{StackRuntime, SuspendHandle};

struct CountingWake(Arc<AtomicUsize>);
impl Wake for CountingWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn basic_output() {
    let rt = StackRuntime::new(|_| 42u32).unwrap();
    assert_eq!(block_on(rt), 42);
}

#[test]
fn result_output() {
    let rt = StackRuntime::new(|_| -> Result<i32, &'static str> { Ok(7) }).unwrap();
    assert_eq!(block_on(rt), Ok(7));
}

#[test]
fn block_on_drives_async() {
    let rt = StackRuntime::new(|ctx| {
        ctx.block_on(std::future::ready(1)) + ctx.block_on(std::future::ready(2))
    })
    .unwrap();
    assert_eq!(block_on(rt), 3);
}

#[test]
fn custom_stack_size() {
    let rt = StackRuntime::with_stack_size(|_| "ok", 512 * 1024).unwrap();
    assert_eq!(block_on(rt), "ok");
}

#[test]
fn suspend_then_wake() {
    let ready = Rc::new(Cell::new(false));
    let waker_slot = Rc::new(RefCell::new(None::<Waker>));

    let r = ready.clone();
    let w = waker_slot.clone();
    let rt = StackRuntime::new(move |ctx| {
        ctx.block_on(ManualFuture { ready: r, waker: w });
        100u32
    })
    .unwrap();

    let wake_count = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(CountingWake(wake_count.clone())));
    let mut cx = Context::from_waker(&waker);

    let mut rt = Box::pin(rt);
    assert!(matches!(rt.as_mut().poll(&mut cx), Poll::Pending));

    let stored = waker_slot.borrow_mut().take().expect("waker not stored");
    ready.set(true);
    stored.wake_by_ref();
    assert_eq!(wake_count.load(Ordering::SeqCst), 1);

    assert!(matches!(rt.as_mut().poll(&mut cx), Poll::Ready(100)));
}

#[test]
fn nested_block_on_works() {
    let rt = StackRuntime::new(|ctx| {
        ctx.block_on(async {
            ctx.block_on(std::future::ready(1)) + ctx.block_on(std::future::ready(2))
        })
    })
    .unwrap();
    assert_eq!(block_on(rt), 3);
}
