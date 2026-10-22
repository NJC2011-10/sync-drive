use futures::executor::block_on;
use futures::task::noop_waker;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use sync_drive::{StackRuntime, SuspendCtx};

// ============ helpers ============

/// 记录被唤醒次数的 Waker。
struct CountingWake(Arc<AtomicUsize>);
impl Wake for CountingWake {
	fn wake(self: Arc<Self>) {
		self.0.fetch_add(1, Ordering::SeqCst);
	}
	fn wake_by_ref(self: &Arc<Self>) {
		self.0.fetch_add(1, Ordering::SeqCst);
	}
}

/// 手动可控的 Future：首次 poll 返回 Pending 并保存 waker，
/// 之后由外部设置 ready + 唤醒。
struct ManualFuture {
	ready: Rc<Cell<bool>>,
	waker: Rc<RefCell<Option<Waker>>>,
}

impl Future for ManualFuture {
	type Output = ();
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
		if self.ready.get() {
			Poll::Ready(())
		} else {
			*self.waker.borrow_mut() = Some(cx.waker().clone());
			Poll::Pending
		}
	}
}

fn extract_panic_msg(err: &(dyn std::any::Any + Send)) -> String {
	if let Some(s) = err.downcast_ref::<&str>() {
		s.to_string()
	} else if let Some(s) = err.downcast_ref::<String>() {
		s.clone()
	} else {
		String::new()
	}
}

// ============ tests ============

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

	// 协程挂起，ManualFuture 的 waker 应已存储
	let stored = waker_slot.borrow_mut().take().expect("waker not stored");

	// 设置就绪并唤醒
	ready.set(true);
	stored.wake_by_ref();
	assert_eq!(wake_count.load(Ordering::SeqCst), 1);

	// 再次 poll，应 Ready(100)
	assert!(matches!(rt.as_mut().poll(&mut cx), Poll::Ready(100)));
}

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
	assert!(msg2.contains("polled after panic"));
}

#[test]
fn block_on_after_completion_panics() {
	let ctx_slot: Rc<RefCell<Option<SuspendCtx>>> = Rc::new(RefCell::new(None));
	let slot = ctx_slot.clone();

	let rt = StackRuntime::new(move |ctx| {
		*slot.borrow_mut() = Some(ctx.clone());
		42u32
	})
	.unwrap();

	assert_eq!(block_on(rt), 42);

	let ctx = ctx_slot.borrow_mut().take().expect("ctx not stored");
	let r = catch_unwind(AssertUnwindSafe(|| {
		// 必须用会 Pending 的 future，逼 block_on 去查 yielder。
		// ready(()) 第一次 poll 就 Ready，永远走不到 suspend，不触发 panic。
		ctx.block_on(std::future::pending::<()>());
	}));
	let msg = extract_panic_msg(
		r.expect_err("block_on after completion should panic")
			.as_ref(),
	);
	assert!(msg.contains("coroutine not running"));
}

#[test]
fn block_on_ready_after_completion_is_ok() {
	let ctx_slot: Rc<RefCell<Option<SuspendCtx>>> = Rc::new(RefCell::new(None));
	let slot = ctx_slot.clone();

	let rt = StackRuntime::new(move |ctx| {
		*slot.borrow_mut() = Some(ctx.clone());
		42u32
	})
	.unwrap();

	assert_eq!(block_on(rt), 42);

	let ctx = ctx_slot.borrow_mut().take().unwrap();
	// future 不需要挂起，所以不需要 yielder，能正常返回
	assert_eq!(ctx.block_on(std::future::ready(7)), 7);
}

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
