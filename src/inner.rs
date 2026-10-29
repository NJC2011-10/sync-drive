//! 协程执行环境与两个方向的驱动逻辑。

use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult};
use std::any::Any;
use std::cell::{Cell, OnceCell, RefCell};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::task::{Context, Poll, Waker};

/// 协程的生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lifecycle {
	/// 初始值。`StoreCtx`/`SuspendCtx` 只能在 `body` 内拿到，此时 lifecycle
	/// 已是 `Running`，所以这个状态永远不会出现在 `SuspendError` 中。
	NotStarted,
	Running,
	Suspended,
	Done,
	Poisoned,
}

/// `run_until_ready` / `suspend_with` 的失败原因。
#[derive(Debug, Clone, Copy)]
pub(crate) enum SuspendError {
	/// lifecycle 不是 Running。携带实际状态便于诊断。
	NotInResume(Lifecycle),
	/// 处于 panic 展开中。`block_on` 路径下：有 `catch_unwind` 包裹时被捕获，
	/// 无包裹时 double panic → abort。不是“正常错误”，是“契约违反后的终止”。
	Panicking,
	/// 当前 SP 不在本协程的栈范围内。
	WrongStack,
}

impl std::fmt::Display for SuspendError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::NotInResume(l) => write!(f, "not inside resume (lifecycle: {l:?})"),
			Self::Panicking => write!(f, "cannot suspend during panic unwinding"),
			Self::WrongStack => write!(f, "cross-coroutine suspend attempt"),
		}
	}
}

/// 一条协程的执行环境。
pub(crate) struct Inner {
	pub(crate) waker: RefCell<Option<Waker>>,
	pub(crate) lifecycle: Cell<Lifecycle>,
	pub(crate) trap_check: OnceCell<Box<dyn Fn(usize) -> bool>>,
}

impl Inner {
	pub(crate) fn new() -> Self {
		Self {
			waker: RefCell::new(None),
			lifecycle: Cell::new(Lifecycle::NotStarted),
			trap_check: OnceCell::new(),
		}
	}

	pub(crate) fn set_waker(&self, w: &Waker) {
		*self.waker.borrow_mut() = Some(w.clone());
	}

	pub(crate) fn get_waker(&self) -> Waker {
		self.waker
			.borrow()
			.clone()
			.expect("waker is always set before body runs")
	}

	/// 异步转同步：poll future 直到 `Ready`，`Pending` 时调用 `suspend`。
	///
	/// `panicking` 检查由 [`SuspendHandle::suspend_with`] 统一负责，不在此处。
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
			let waker = self.get_waker();
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
