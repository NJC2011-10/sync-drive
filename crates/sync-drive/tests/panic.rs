mod common;

use common::extract_panic_msg;
use futures::executor::block_on;
use futures::task::noop_waker;
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::task::Context;
use sync_drive::{StackRuntime, StoreCtx, SuspendError, SuspendHandle};

#[test]
fn poll_after_panic() {
    let rt = StackRuntime::new(|_| -> () { panic!("intentional") }).unwrap();
    let mut rt = Box::pin(rt);
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    let r1 = catch_unwind(AssertUnwindSafe(|| {
        let _ = rt.as_mut().poll(&mut cx);
    }));
    let msg1 = extract_panic_msg(r1.expect_err("first poll should panic").as_ref());
    assert!(msg1.contains("intentional"));

    let r2 = catch_unwind(AssertUnwindSafe(|| {
        let _ = rt.as_mut().poll(&mut cx);
    }));
    let msg2 = extract_panic_msg(r2.expect_err("second poll should panic").as_ref());
    assert!(msg2.contains("polled after completion"));
}

#[test]
fn suspend_during_unwind_returns_panicking() {
    struct D {
        store: StoreCtx,
        hit: Rc<Cell<bool>>,
    }
    impl Drop for D {
        fn drop(&mut self) {
            let r = catch_unwind(AssertUnwindSafe(|| {
                self.store.block_on(std::future::pending::<()>());
            }));
            let msg = extract_panic_msg(r.expect_err("should panic").as_ref());
            if msg.contains("panic unwinding") {
                self.hit.set(true);
            }
        }
    }

    let hit = Rc::new(Cell::new(false));
    let h = hit.clone();

    let rt = StackRuntime::new(move |ctx| {
        let _d = D {
            store: ctx.to_store(),
            hit: h,
        };
        panic!("boom");
    })
    .unwrap();

    let _ = catch_unwind(AssertUnwindSafe(|| {
        let _ = block_on(rt);
    }));
    assert!(hit.get(), "panicking check should fire");
}

#[test]
fn try_block_on_during_unwind_returns_panicking() {
    struct D(StoreCtx);
    impl Drop for D {
        fn drop(&mut self) {
            assert!(matches!(
                self.0.try_block_on(std::future::pending::<()>()),
                Err(SuspendError::Panicking)
            ));
        }
    }
    let slot: Rc<RefCell<Option<StoreCtx>>> = Rc::new(RefCell::new(None));
    let s = slot.clone();
    let rt = StackRuntime::new(move |ctx| {
        *s.borrow_mut() = Some(ctx.to_store());
        let _d = D(slot.borrow().clone().unwrap());
        panic!("boom");
    })
    .unwrap();
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let _ = block_on(rt);
    }));
}
