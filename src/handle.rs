use crate::inner::{Inner, SuspendError};
use corosensei::Yielder;
use std::future::Future;

/// 能力 trait：句柄只实现这个。不导出到 crate 外（模块私有），
/// 但对 blanket impl 可见。
pub(crate) trait SuspendCapability {
	fn inner(&self) -> &Inner;
	fn with_yielder<R>(&self, f: impl FnOnce(&Yielder<(), ()>) -> R) -> Result<R, SuspendError>;

	fn suspend_with<R>(&self, f: impl FnOnce(&Yielder<(), ()>) -> R) -> Result<R, SuspendError> {
		if std::thread::panicking() {
			return Err(SuspendError::Panicking);
		}
		self.with_yielder(f)
	}
}

/// 公开 trait：用户看到的只有这两个方法。
pub trait SuspendHandle {
	fn block_on<F: Future>(&self, f: F) -> F::Output;

	fn on_parent_stack<R, F>(&self, f: F) -> R
	where
		F: FnOnce() -> R + Send,
		R: Send;
}

/// 任何 `SuspendCapability` 自动获得 `SuspendHandle`。
impl<H: SuspendCapability> SuspendHandle for H {
	fn block_on<F: Future>(&self, f: F) -> F::Output {
		self.inner()
			.run_until_ready(f, || self.suspend_with(|y| y.suspend(())))
			.unwrap_or_else(|e| panic!("block_on: {e}"))
	}

	fn on_parent_stack<R, F>(&self, f: F) -> R
	where
		F: FnOnce() -> R + Send,
		R: Send,
	{
		self.suspend_with(|y| y.on_parent_stack(f))
			.unwrap_or_else(|e| panic!("on_parent_stack: {e}"))
	}
}
