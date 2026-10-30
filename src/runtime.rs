//! executor 适配层。

use crate::DEFAULT_STACK_SIZE;
use crate::ctx::SuspendCtx;
use crate::inner::{Inner, Lifecycle};
use corosensei::Coroutine;
use corosensei::stack::DefaultStack;
use std::future::Future;
use std::panic::resume_unwind;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

/// `StackRuntime` 的 poll 视角状态。和 `Inner::lifecycle` 是两个维度：
/// 这里管“能否 poll”，`lifecycle` 管“能否 suspend”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunState {
	/// 可 poll，`coroutine` 为 `Some`。
	Running,
	/// 不可 poll，`coroutine` 仍为 `Some`（栈保留，等 `into_stack` 取回或 drop 时释放）。
	Done,
	/// 不可 poll，`coroutine` 为 `None`（panic 后已 drop）。
	Poisoned,
}

/// 栈所有者。实现 `Future`，由 executor 驱动。
pub struct StackRuntime<T> {
	coroutine: Option<Coroutine<(), (), T, DefaultStack>>,
	/// poll 视角的状态。和 `coroutine` 的 `Some` / `None` 保持一致：
	/// `Running` / `Done` → `Some`，`Poisoned` → `None`。
	state: RunState,
	inner: Rc<Inner>,
}

impl<T: 'static> StackRuntime<T> {
	/// 用默认栈大小创建。
	///
	/// `body` 接收协程内部的 [`SuspendCtx`]。
	///
	/// # Errors
	/// 栈分配失败时返回 [`std::io::Error`]。
	#[inline]
	pub fn new<F>(body: F) -> Result<Self, std::io::Error>
	where
		F: FnOnce(SuspendCtx<'_>) -> T + 'static,
	{
		Self::with_stack_size(body, DEFAULT_STACK_SIZE)
	}

	/// 指定栈大小创建。内部调 [`DefaultStack::new`] 分配栈，然后委托给
	/// [`with_stack`](Self::with_stack)。
	///
	/// # Errors
	/// 栈分配失败时返回 [`std::io::Error`]。
	#[inline]
	pub fn with_stack_size<F>(body: F, stack_size: usize) -> Result<Self, std::io::Error>
	where
		F: FnOnce(SuspendCtx<'_>) -> T + 'static,
	{
		let stack = DefaultStack::new(stack_size)?;
		Ok(Self::with_stack(body, stack))
	}

	/// 用已有的栈创建。`stack` 通常来自 [`into_stack`](Self::into_stack) 的返回值。
	///
	/// 不返回 `Result`——栈已存在，不存在分配失败。栈的分配错误由调用方
	/// 在 [`DefaultStack::new`] 时处理。
	///
	/// # Panics
	/// 若 `trap_check` 设置失败（构造逻辑错误，不应发生）。
	pub fn with_stack<F>(body: F, stack: DefaultStack) -> Self
	where
		F: FnOnce(SuspendCtx<'_>) -> T + 'static,
	{
		let inner = Rc::new(Inner::new());
		let inner_for_co = inner.clone();
		let co = Coroutine::with_stack(stack, move |yielder, ()| {
			body(SuspendCtx {
				yielder,
				inner: inner_for_co,
			})
		});

		// trap_handler 带泛型 T，Inner 不带 T——用 Box<dyn Fn> 擦除后存入，
		// 供 StoreCtx 的 SP 检查使用。
		let handler = co.trap_handler();
		inner
			.trap_check
			.set(Box::new(move |sp| handler.stack_ptr_in_bounds(sp)))
			.ok()
			.expect("trap_check set exactly once at construction");

		Self {
			coroutine: Some(co),
			state: RunState::Running,
			inner,
		}
	}

	/// 消费 `self`，取回协程使用的栈。
	///
	/// 取回的 [`DefaultStack`] 可以传给 [`with_stack`](Self::with_stack)
	/// 复用，避免重新 `mmap` 分配。
	///
	/// # 返回
	///
	/// - `Some(stack)`：`RunState` 为 `Running`（挂起中）或 `Done`（已完成），
	///   `coroutine` 字段为 `Some`，`Coroutine::into_stack` 消费它取回栈。
	/// - `None`：`RunState` 为 `Poisoned`（panic 后），`coroutine` 字段已在
	///   `poll` 的 `Err` 分支被 `take()`，栈随 `Coroutine::drop` 展开释放，
	///   无法回收。
	///
	/// # 挂起中取栈
	///
	/// 若协程尚未完成，`Coroutine::into_stack` 会强制结束协程（展开栈），
	/// 栈上的局部变量按正常路径 drop。这意味着**未完成的协程逻辑被放弃**，
	/// 调用前需确认这是预期行为。
	///
	/// # 消费 `self`
	///
	/// 调用后 `StackRuntime` 不可再用，`Drop` 会在 `into_stack` 返回前执行：
	/// `lifecycle` 若仍为 `Running` / `Suspended`，被写为 `Done`；
	/// 逃逸的 [`StoreCtx`](crate::StoreCtx) 此后再调 `block_on` 会得到
	/// `NotInResume(Done)`。
	#[inline]
	#[must_use]
	pub fn into_stack(mut self) -> Option<DefaultStack> {
		self.coroutine.take().map(Coroutine::into_stack)
	}
}

impl<T> Future for StackRuntime<T> {
	type Output = T;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
		// coroutine 在 Done 时保留（供 into_stack 取栈），在 Poisoned 时 take
		// （panic 后不可 resume）。
		let this = self.get_mut();

		match this.state {
			RunState::Done => panic!("StackRuntime polled after completion"),
			RunState::Poisoned => panic!("StackRuntime polled after panic"),
			RunState::Running => {}
		}

		this.inner.set_waker(cx.waker());

		let co = this.coroutine.as_mut().expect("Running implies Some");
		match this.inner.resume_once(co) {
			Ok(Poll::Pending) => Poll::Pending,
			Ok(Poll::Ready(t)) => {
				this.state = RunState::Done;
				Poll::Ready(t)
			}
			Err(e) => {
				this.state = RunState::Poisoned;
				this.coroutine.take();
				resume_unwind(e)
			}
		}
	}
}

impl<T> Drop for StackRuntime<T> {
	#[inline]
	fn drop(&mut self) {
		// coroutine 由 Option 自动 drop，这里只更新 lifecycle：
		// 标记 Done 让逃逸的 StoreCtx 事后调用 block_on 得到 NotInResume(Done) 而非 UB。
		match self.inner.lifecycle.get() {
			Lifecycle::NotStarted | Lifecycle::Running | Lifecycle::Suspended => {
				self.inner.lifecycle.set(Lifecycle::Done);
			}
			Lifecycle::Done | Lifecycle::Poisoned => {}
		}
	}
}

#[cfg(test)]
mod pin_tests {
	use super::StackRuntime;

	fn assert_unpin<T: Unpin>() {}

	#[test]
	fn stack_runtime_is_unpin() {
		assert_unpin::<StackRuntime<()>>();
	}
}
