use corosensei::{Coroutine, CoroutineResult, Yielder};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::pin::Pin;
use std::ptr::NonNull;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

/// 默认协程栈大小。
pub const DEFAULT_STACK_SIZE: usize = 2 * 1024 * 1024;

struct Inner {
	yielder: Cell<Option<NonNull<Yielder<(), ()>>>>,
	waker: RefCell<Option<Waker>>,
}

/// 挂起能力的轻量句柄。`Clone` 廉价，所有克隆指向同一条栈。
#[derive(Clone)]
pub struct SuspendCtx {
	inner: Rc<Inner>,
}

impl SuspendCtx {
	/// 在协程内同步驱动 Future。`Pending` 时 suspend，`Ready` 时返回。
	///
	/// # 调用约束
	/// 必须在协程运行期间调用，否则 panic。
	/// 支持重入：挂起的是整条栈，恢复精确回到最内层 suspend 点。
	pub fn block_on<F: Future>(&self, f: F) -> F::Output {
		let mut f = Box::pin(f);
		loop {
			let waker = self.inner.waker.borrow().clone().expect("waker not set");
			let mut cx = Context::from_waker(&waker);
			match f.as_mut().poll(&mut cx) {
				Poll::Ready(v) => return v,
				Poll::Pending => {
					let y = self
						.inner
						.yielder
						.get()
						.expect("block_on: coroutine not running or already finished");
					// SAFETY: yielder 由 StackRuntime 持有，suspend 期间存活；
					// 单线程无并发访问。
					unsafe { y.as_ref() }.suspend(());
				}
			}
		}
	}
}

impl std::fmt::Debug for SuspendCtx {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SuspendCtx")
			.field("yielder_set", &self.inner.yielder.get().is_some())
			.finish()
	}
}

enum TaskState<F, T> {
	Fresh(F),
	Running(Coroutine<(), (), T>),
	Done,
	Poisoned,
}

/// 栈所有者。实现 `Future`，由 executor 驱动。
pub struct StackRuntime<F, T> {
	state: TaskState<F, T>,
	inner: Rc<Inner>,
	stack_size: usize,
}

impl<F, T> StackRuntime<F, T>
where
	F: FnOnce(SuspendCtx) -> T + 'static,
{
	/// 用默认栈大小创建。返回 `(runtime, ctx)`，两者指向同一条栈。
	pub fn new(body: F) -> (Self, SuspendCtx) {
		Self::with_stack_size(body, DEFAULT_STACK_SIZE)
	}

	/// 指定栈大小创建。
	pub fn with_stack_size(body: F, stack_size: usize) -> (Self, SuspendCtx) {
		let inner = Rc::new(Inner {
			yielder: Cell::new(None),
			waker: RefCell::new(None),
		});
		let ctx = SuspendCtx {
			inner: inner.clone(),
		};
		let runtime = Self {
			state: TaskState::Fresh(body),
			inner,
			stack_size,
		};
		(runtime, ctx)
	}
}

impl<F, T> Future for StackRuntime<F, T>
where
	F: FnOnce(SuspendCtx) -> T + 'static,
{
	type Output = T;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
		// 状态机：Fresh -> Running -> {Done | Poisoned}。
		// Running 时 resume 返回 Yield 则保持 Running；Return 转 Done；
		// panic 经 catch_unwind 转 Poisoned 并 re-panic。
		// SAFETY: 内部不做 pin projection；TaskState 各字段可自由移动。
		let this = unsafe { self.get_unchecked_mut() };

		match &this.state {
			TaskState::Done => panic!("StackRuntime polled after completion"),
			TaskState::Poisoned => panic!("StackRuntime polled after panic"),
			_ => {}
		}

		*this.inner.waker.borrow_mut() = Some(cx.waker().clone());

		if matches!(this.state, TaskState::Fresh(_)) {
			let TaskState::Fresh(body) = std::mem::replace(&mut this.state, TaskState::Done) else {
				unreachable!()
			};
			let inner = this.inner.clone();
			let stack_size = this.stack_size;
			this.state = TaskState::Running(Coroutine::new(stack_size, move |yielder, _| {
				inner.yielder.set(Some(NonNull::from(yielder)));
				body(SuspendCtx { inner })
			}));
		}

		let result = {
			let TaskState::Running(co) = &mut this.state else {
				unreachable!()
			};
			// catch_unwind 只为毒化状态后 re-panic，不恢复执行。
			catch_unwind(AssertUnwindSafe(|| co.resume(())))
		};

		match result {
			Ok(CoroutineResult::Yield(())) => Poll::Pending,
			Ok(CoroutineResult::Return(t)) => {
				this.inner.yielder.set(None);
				this.state = TaskState::Done;
				Poll::Ready(t)
			}
			Err(e) => {
				this.inner.yielder.set(None);
				this.state = TaskState::Poisoned;
				resume_unwind(e)
			}
		}
	}
}

impl<F, T> Drop for StackRuntime<F, T> {
	fn drop(&mut self) {
		// Coroutine 的 Drop 会展开协程栈，之后 yielder 失效；
		// 清空以让 SuspendCtx 事后调用 block_on 时得到清晰 panic 而非 UB。
		self.inner.yielder.set(None);
	}
}
