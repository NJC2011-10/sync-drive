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
	assert!(msg.contains("not inside resume"));
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

// ============ 三任务 ============

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

	// 全部挂起，各自独立的 waker 都已存储
	for t in &mut tasks {
		assert!(t.as_mut().poll(&mut cx).is_pending());
	}
	for w in &wakers {
		assert!(w.borrow().is_some());
	}

	// 任意顺序放行：先 1，再 2，再 0
	for i in [1usize, 2, 0] {
		ready[i].set(true);
		// 放行 i 之前，其他未放行的仍 Pending
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

	// B panic
	let r = catch_unwind(AssertUnwindSafe(|| {
		let _ = b.as_mut().poll(&mut cx);
	}));
	assert!(extract_panic_msg(r.expect_err("b should panic").as_ref()).contains("b panics"));

	// A 和 C 不受影响
	assert!(matches!(a.as_mut().poll(&mut cx), Poll::Ready(1)));
	assert!(matches!(c.as_mut().poll(&mut cx), Poll::Ready(3)));

	// B 二次 poll 得到 Poisoned panic
	let r2 = catch_unwind(AssertUnwindSafe(|| {
		let _ = b.as_mut().poll(&mut cx);
	}));
	assert!(
		extract_panic_msg(r2.expect_err("b second poll should panic").as_ref())
			.contains("polled after panic")
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

	// drop 挂起中的 B
	drop(b);

	// A 和 C 已完成，结果不受影响（已 Ready 的 task 字段仍可读）
	// 注意：已 Ready 的 task 不能再 poll，但 drop 不 panic
	drop(a);
	drop(c);
}

// ============ 时间收益（并发 vs 串行） ============

use std::time::{Duration, Instant};

#[test]
fn concurrent_tasks_overlap_in_time() {
	let rt = tokio::runtime::Builder::new_current_thread()
		.enable_time()
		.build()
		.unwrap();

	let make = || {
		StackRuntime::new(|ctx| {
			ctx.block_on(tokio::time::sleep(Duration::from_millis(100)));
		})
		.unwrap()
	};

	let a = make();
	let b = make();

	let start = Instant::now();
	rt.block_on(async { futures::future::join(a, b).await });
	let elapsed = start.elapsed();

	assert!(
		elapsed < Duration::from_millis(150),
		"两个 100ms sleep 应并发执行，实际耗时 {elapsed:?}"
	);
	assert!(
		elapsed >= Duration::from_millis(100),
		"至少要等一个 100ms sleep，实际 {elapsed:?}"
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
			ctx.block_on(tokio::time::sleep(Duration::from_millis(100)));
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
		elapsed >= Duration::from_millis(180),
		"两个 100ms sleep 串行应 ~200ms，实际 {elapsed:?}"
	);
}

#[test]
fn block_on_pending_while_suspended_panics() {
	let slot: Rc<RefCell<Option<SuspendCtx>>> = Rc::new(RefCell::new(None));
	let s = slot.clone();
	let rt = StackRuntime::new(move |ctx| {
		*s.borrow_mut() = Some(ctx.clone());
		ctx.block_on(std::future::pending::<()>());
	})
	.unwrap();

	let mut rt = Box::pin(rt);
	let waker = noop_waker();
	let mut cx = Context::from_waker(&waker);
	assert!(rt.as_mut().poll(&mut cx).is_pending());

	let ctx = slot.borrow().clone().expect("ctx not stored");
	let r = catch_unwind(AssertUnwindSafe(|| {
		ctx.block_on(std::future::pending::<()>());
	}));
	let msg = extract_panic_msg(r.expect_err("suspended block_on should panic").as_ref());
	assert!(msg.contains("not inside resume"));
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
