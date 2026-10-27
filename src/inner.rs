//! 协程执行环境与两个方向的驱动逻辑。

use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::task::{Context, Poll, Waker};

/// 协程的生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lifecycle {
	NotStarted,
	Running,
	Suspended,
	Done,
	Poisoned,
}

/// `run_until_ready` 的失败原因。
#[derive(Debug)]
pub(crate) enum SuspendError {
	/// 当前不在协程 `resume` 期间。携带实际状态便于诊断。
	NotInResume(Lifecycle),
	/// 处于 panic 展开中，不能在 unwind 中切栈。
	Panicking,
	/// 逃逸句柄的 `Yielder` 指针已被清空。
	YielderCleared,
}

/// 一条协程的执行环境。
pub(crate) struct Inner {
	pub(crate) waker: RefCell<Option<Waker>>,
	pub(crate) lifecycle: Cell<Lifecycle>,
}

impl Inner {
	pub(crate) fn new() -> Self {
		Self {
			waker: RefCell::new(None),
			lifecycle: Cell::new(Lifecycle::NotStarted),
		}
	}

	pub(crate) fn set_waker(&self, w: &Waker) {
		*self.waker.borrow_mut() = Some(w.clone());
	}

	pub(crate) fn get_waker(&self) -> Option<Waker> {
		self.waker.borrow().clone()
	}

	/// 异步转同步：poll future 直到 `Ready`，`Pending` 时调用 `suspend`。
	///
	/// 只做环境检查（`waker` 是否已设置）；`Yielder` 相关的安全检查由调用方
	/// 提供的 `suspend` 闭包负责——`SuspendCtx` 走借用，`StoreCtx` 走运行时检查。
	pub(crate) fn run_until_ready<F, S>(
		&self,
		f: F,
		mut suspend: S,
	) -> Result<F::Output, SuspendError>
	where
		F: Future,
		S: FnMut() -> Result<(), SuspendError>,
	{
		let mut f = std::pin::pin!(f);
		loop {
			let waker = self
				.get_waker()
				.ok_or(SuspendError::NotInResume(Lifecycle::NotStarted))?;
			let mut cx = Context::from_waker(&waker);
			match f.as_mut().poll(&mut cx) {
				Poll::Ready(v) => return Ok(v),
				Poll::Pending => suspend()?,
			}
		}
	}

	/// 同步转异步：resume 协程一次，显式写 `lifecycle`，panic 原样返回。
	pub(crate) fn resume_once<T>(
		&self,
		co: &mut Coroutine<(), (), T, DefaultStack>,
	) -> Result<Poll<T>, Box<dyn Any + Send>> {
		self.lifecycle.set(Lifecycle::Running);
		match catch_unwind(AssertUnwindSafe(|| co.resume(()))) {
			Ok(CoroutineResult::Yield(())) => {
				self.lifecycle.set(Lifecycle::Suspended);
				Ok(Poll::Pending)
			}
			Ok(CoroutineResult::Return(t)) => {
				self.lifecycle.set(Lifecycle::Done);
				Ok(Poll::Ready(t))
			}
			Err(e) => {
				self.lifecycle.set(Lifecycle::Poisoned);
				Err(e)
			}
		}
	}
}

impl std::fmt::Display for SuspendError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NotInResume(l) => write!(f, "not inside resume (lifecycle: {l:?})"),
			Self::Panicking => write!(f, "cannot suspend during panic unwinding"),
			Self::YielderCleared => write!(f, "yielder already cleared"),
		}
	}
}
