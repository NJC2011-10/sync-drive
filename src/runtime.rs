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
	/// 用默认栈大小创建。
	///
	/// `body` 接收协程内部的 [`SuspendCtx`]。
	///
	/// # Errors
	/// 栈分配失败时返回 [`std::io::Error`]。
	pub fn new<F>(body: F) -> Result<Self, std::io::Error>
	where
		F: FnOnce(SuspendCtx<'_>) -> T + 'static,
	{
		Self::with_stack_size(body, DEFAULT_STACK_SIZE)
	}

	/// 指定栈大小创建。
	///
	/// # Errors
	/// 栈分配失败时返回 [`std::io::Error`]。
	pub fn with_stack_size<F>(body: F, stack_size: usize) -> Result<Self, std::io::Error>
	where
		F: FnOnce(SuspendCtx<'_>) -> T + 'static,
	{
		let inner = Rc::new(Inner::new());
		let inner_for_co = inner.clone();
		let stack = DefaultStack::new(stack_size)?;
		let co = Coroutine::with_stack(stack, move |yielder, ()| {
			body(SuspendCtx {
				yielder,
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
		let this = self.get_mut();

		match &this.state {
			TaskState::Done => panic!("StackRuntime polled after completion"),
			TaskState::Poisoned => panic!("StackRuntime polled after panic"),
			TaskState::Running(_) => {}
		}

		this.inner.set_waker(cx.waker());

		let TaskState::Running(co) = &mut this.state else {
			unreachable!()
		};

		match this.inner.resume_once(co) {
			Ok(Poll::Pending) => Poll::Pending,
			Ok(Poll::Ready(t)) => {
				let old = std::mem::replace(&mut this.state, TaskState::Done);
				drop(old);
				Poll::Ready(t)
			}
			Err(e) => {
				let old = std::mem::replace(&mut this.state, TaskState::Poisoned);
				drop(old);
				resume_unwind(e)
			}
		}
	}
}

impl<T> Drop for StackRuntime<T> {
	fn drop(&mut self) {
		match self.inner.lifecycle.get() {
			Lifecycle::NotStarted | Lifecycle::Running | Lifecycle::Suspended => {
				self.inner.lifecycle.set(Lifecycle::Done);
			}
			Lifecycle::Done | Lifecycle::Poisoned => {}
		}
	}
}
