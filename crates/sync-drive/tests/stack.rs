mod common;

use futures::executor::block_on;
use futures::task::noop_waker;
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use sync_drive::{StackRuntime, SuspendError, SuspendHandle};

#[test]
fn drop_runtime_runs_stack_destructors() {
    struct DropGuard(Rc<Cell<bool>>);
    impl Drop for DropGuard {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    let dropped = Rc::new(Cell::new(false));
    let d = dropped.clone();

    let rt = StackRuntime::new(move |ctx| {
        let _g = DropGuard(d);
        ctx.block_on(std::future::pending::<()>());
    })
    .unwrap();

    let mut rt = Box::pin(rt);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(matches!(rt.as_mut().poll(&mut cx), Poll::Pending));

    drop(rt);
    assert!(
        dropped.get(),
        "DropGuard should be dropped when coroutine is dropped"
    );
}

#[test]
fn into_stack_after_completion_returns_stack() {
    let mut rt = StackRuntime::new(|_| 7u32).unwrap();
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(matches!(Pin::new(&mut rt).poll(&mut cx), Poll::Ready(7)));

    let stack = rt.into_stack();
    let rt2 = StackRuntime::with_stack(|_| 8u32, stack);
    assert_eq!(block_on(rt2), 8);
}

#[test]
#[should_panic(expected = "cannot extract stack from an incomplete coroutine")]
fn into_stack_while_suspended_panics() {
    let rt = StackRuntime::new(|ctx| {
        ctx.block_on(std::future::pending::<()>());
    })
    .unwrap();

    let mut rt = Box::pin(rt);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(rt.as_mut().poll(&mut cx).is_pending());

    let rt = *Pin::into_inner(rt);
    let _ = rt.into_stack();
}

#[test]
fn cross_coroutine_via_nested_runtime() {
    let rt = StackRuntime::new(|outer_ctx| {
        let store = outer_ctx.to_store();
        let inner = StackRuntime::new(move |_inner_ctx| {
            match store.try_block_on(std::future::pending::<()>()) {
                Err(SuspendError::CrossCoroutine) => {}
                other => panic!("expected CrossCoroutine, got {other:?}"),
            }
        })
        .unwrap();
        outer_ctx.block_on(inner);
    })
    .unwrap();
    block_on(rt);
}

#[test]
fn into_stack_after_panic_returns_stack() {
    let rt = StackRuntime::new(|_| -> u32 { panic!("boom") }).unwrap();
    let mut rt = Box::pin(rt);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    let r = catch_unwind(AssertUnwindSafe(|| {
        let _ = rt.as_mut().poll(&mut cx);
    }));
    assert!(r.is_err(), "首次 poll 应 panic");

    let rt = *Pin::into_inner(rt);
    let stack = rt.into_stack();
    let rt2 = StackRuntime::with_stack(|_| 8u32, stack);
    assert_eq!(block_on(rt2), 8);
}
