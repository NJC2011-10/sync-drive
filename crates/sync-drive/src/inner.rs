//! 协程执行环境与两个方向的驱动逻辑。

use crate::error::SuspendError;
use crate::state::Lifecycle;
use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult};
use std::any::Any;
use std::cell::{Cell, OnceCell, RefCell};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::task::{Context, Poll, Waker};

/// 一条协程的执行环境。
pub(crate) struct Inner {
	pub(crate) waker: RefCell<Option<Waker>>,
	pub(crate) lifecycle: Cell<Lifecycle>,
	pub(crate) trap_check: OnceCell<Box<dyn Fn(usize) -> bool>>,
}

impl Inner {
	#[inline]
	pub(crate) fn new() -> Self {
		Self {
			waker: RefCell::new(None),
			lifecycle: Cell::new(Lifecycle::NotStarted),
			trap_check: OnceCell::new(),
		}
	}

	#[inline]
	pub(crate) fn set_waker(&self, w: &Waker) {
		*self.waker.borrow_mut() = Some(w.clone());
	}

	/// # Panics
	/// 若 `waker` 未设置（`body` 执行前未 `set_waker`，属内部不变量违反）。
	#[inline]
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
	///
	/// # Panics
	/// 若协程已处于 `Done` / `Poisoned` 状态（绕过 `poll` 直接调用），
	/// `Coroutine::resume` 会 panic。正常调用路径（`poll`）已在顶部检查状态。
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
