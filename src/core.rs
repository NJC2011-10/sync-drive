use corosensei::stack::DefaultStack;
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
	in_resume: Cell<bool>,
}

struct ResumeGuard<'a>(&'a Cell<bool>);
impl Drop for ResumeGuard<'_> {
	fn drop(&mut self) {
		self.0.set(false);
	}
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
	///
	/// # Panics
	/// 在以下情况 panic：
	/// - 协程外调用（`yielder` 未设置）；
	/// - `StackRuntime` 尚未 poll（`waker` 未设置）；
	/// - 协程已结束或已 panic（`yielder` 已清空）。
	pub fn block_on<F: Future>(&self, f: F) -> F::Output {
		let mut f = Box::pin(f);
		loop {
			let waker = self.inner.waker.borrow().clone().expect("waker not set");
			let mut cx = Context::from_waker(&waker);
			match f.as_mut().poll(&mut cx) {
				Poll::Ready(v) => return v,
				Poll::Pending => {
					assert!(
						self.inner.in_resume.get(),
						"block_on: cannot suspend: not inside resume"
					);
					let y = self
						.inner
						.yielder
						.get()
						.expect("block_on: coroutine not running or already finished");
					// SAFETY: in_resume=true 保证协程正在 resume 中，栈帧有效；
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

enum TaskState<T> {
	Running(Coroutine<(), (), T, DefaultStack>),
	Done,
	Poisoned,
}

/// 栈所有者。实现 `Future`，由 executor 驱动。
pub struct StackRuntime<T> {
	state: TaskState<T>,
	inner: Rc<Inner>,
}

impl<T: 'static> StackRuntime<T> {
	/// 用默认栈大小创建。栈在此处分配，失败立即返回错误。
	///
	/// `body` 接收协程内部的 [`SuspendCtx`]。
	///
	/// # Errors
	/// 栈分配失败（如内存不足）时返回 [`std::io::Error`]。
	pub fn new<F>(body: F) -> Result<Self, std::io::Error>
	where
		F: FnOnce(SuspendCtx) -> T + 'static,
	{
		Self::with_stack_size(body, DEFAULT_STACK_SIZE)
	}

	/// 指定栈大小创建。栈在此处分配，失败立即返回错误。
	///
	/// # Errors
	/// 栈分配失败（如内存不足、`stack_size` 无效）时返回 [`std::io::Error`]。
	pub fn with_stack_size<F>(body: F, stack_size: usize) -> Result<Self, std::io::Error>
	where
		F: FnOnce(SuspendCtx) -> T + 'static,
	{
		let inner = Rc::new(Inner {
			yielder: Cell::new(None),
			waker: RefCell::new(None),
			in_resume: Cell::new(false),
		});
		let inner_for_co = inner.clone();
		let stack = DefaultStack::new(stack_size)?;
		let co = Coroutine::with_stack(stack, move |yielder, ()| {
			inner_for_co.yielder.set(Some(NonNull::from(yielder)));
			body(SuspendCtx {
				inner: inner_for_co,
			})
		});
		Ok(Self {
			state: TaskState::Running(co),
			inner,
		})
	}
}

impl<T> Future for StackRuntime<T> {
	type Output = T;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
		// 状态机：Running -> {Done | Poisoned}。
		// Running 时 resume 返回 Yield 则保持 Running；Return 转 Done；
		// panic 经 catch_unwind 转 Poisoned 并 re-panic。
		// SAFETY: 内部不做 pin projection；Coroutine 可自由移动，协程栈在堆上。
		let this = unsafe { self.get_unchecked_mut() };

		match &this.state {
			TaskState::Done => panic!("StackRuntime polled after completion"),
			TaskState::Poisoned => panic!("StackRuntime polled after panic"),
			TaskState::Running(_) => {}
		}

		*this.inner.waker.borrow_mut() = Some(cx.waker().clone());

		let result = {
			let TaskState::Running(co) = &mut this.state else {
				unreachable!()
			};
			this.inner.in_resume.set(true);
			let _guard = ResumeGuard(&this.inner.in_resume);
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

impl<T> Drop for StackRuntime<T> {
	fn drop(&mut self) {
		// Coroutine 的 Drop 会展开协程栈，之后 yielder 失效；
		// 清空以让 SuspendCtx 事后调用 block_on 时得到清晰 panic 而非 UB。
		self.inner.yielder.set(None);
		self.inner.in_resume.set(false);
	}
}
