mod common;

use common::{SendStore, extract_panic_msg};
use futures::executor::block_on;
use futures::task::noop_waker;
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::task::Context;
use sync_drive::{Lifecycle, StackRuntime, StoreCtx, SuspendError, SuspendHandle};

#[test]
fn block_on_after_completion_panics() {
    let slot: Rc<RefCell<Option<StoreCtx>>> = Rc::new(RefCell::new(None));
    let s = slot.clone();

    let rt = StackRuntime::new(move |ctx| {
        *s.borrow_mut() = Some(ctx.to_store());
        42u32
    })
    .unwrap();

    assert_eq!(block_on(rt), 42);

    let store = slot.borrow().clone().expect("store not stored");
    let r = catch_unwind(AssertUnwindSafe(|| {
        store.block_on(std::future::pending::<()>());
    }));
    let msg = extract_panic_msg(
        r.expect_err("block_on after completion should panic")
            .as_ref(),
    );
    assert!(msg.contains("not inside resume"));
}

#[test]
fn block_on_ready_after_completion_is_ok() {
    let slot: Rc<RefCell<Option<StoreCtx>>> = Rc::new(RefCell::new(None));
    let s = slot.clone();

    let rt = StackRuntime::new(move |ctx| {
        *s.borrow_mut() = Some(ctx.to_store());
        42u32
    })
    .unwrap();

    assert_eq!(block_on(rt), 42);

    let store = slot.borrow().clone().expect("store not stored");
    assert_eq!(store.block_on(std::future::ready(7u32)), 7);
}

#[test]
fn block_on_pending_while_suspended_panics() {
    let slot: Rc<RefCell<Option<StoreCtx>>> = Rc::new(RefCell::new(None));
    let s = slot.clone();

    let rt = StackRuntime::new(move |ctx| {
        *s.borrow_mut() = Some(ctx.to_store());
        ctx.block_on(std::future::pending::<()>());
    })
    .unwrap();

    let mut rt = Box::pin(rt);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(rt.as_mut().poll(&mut cx).is_pending());

    let store = slot.borrow().clone().expect("store not stored");
    let r = catch_unwind(AssertUnwindSafe(|| {
        store.block_on(std::future::pending::<()>());
    }));
    let msg = extract_panic_msg(r.expect_err("suspended block_on should panic").as_ref());
    assert!(msg.contains("not inside resume"));
}

#[test]
fn try_block_on_returns_err_instead_of_panic() {
    let slot: Rc<RefCell<Option<StoreCtx>>> = Rc::new(RefCell::new(None));
    let s = slot.clone();
    let rt = StackRuntime::new(move |ctx| {
        *s.borrow_mut() = Some(ctx.to_store());
        42u32
    })
    .unwrap();

    assert_eq!(block_on(rt), 42);

    let store = slot.borrow().clone().unwrap();
    match store.try_block_on(std::future::pending::<()>()) {
        Ok(()) => panic!("should return Err"),
        Err(SuspendError::NotInResume(Lifecycle::Done)) => {}
        Err(e) => panic!("unexpected: {e}"),
    }
}

#[test]
fn lifecycle_reflects_state() {
    let captured: Rc<Cell<Option<Lifecycle>>> = Rc::new(Cell::new(None));
    let c = captured.clone();
    let rt = StackRuntime::new(move |ctx| {
        c.set(Some(ctx.lifecycle()));
    })
    .unwrap();
    block_on(rt);
    assert_eq!(captured.get(), Some(Lifecycle::Suspendable));
}

#[test]
fn on_parent_stack_executes() {
    let rt = StackRuntime::new(|ctx| {
        let v = ctx.on_parent_stack(|| 42u32);
        v + 1
    })
    .unwrap();
    assert_eq!(block_on(rt), 43);
}

#[test]
fn cross_coroutine_via_on_parent_stack() {
    let rt = StackRuntime::new(|ctx| {
        let store = SendStore(ctx.to_store());
        ctx.on_parent_stack(move || {
            let store = store;
            match store.0.try_block_on(std::future::pending::<()>()) {
                Err(SuspendError::CrossCoroutine) => {}
                other => panic!("expected CrossCoroutine, got {other:?}"),
            }
        });
    })
    .unwrap();
    block_on(rt);
}

#[test]
fn is_on_coroutine_stack_reflects_sp() {
    let rt = StackRuntime::new(|ctx| {
        assert!(ctx.is_on_coroutine_stack(), "协程栈上应为 true");
        let store = SendStore(ctx.to_store());
        ctx.on_parent_stack(move || {
            let store = store;
            assert!(!store.0.is_on_coroutine_stack(), "父栈上应为 false");
        });
    })
    .unwrap();
    block_on(rt);
}
