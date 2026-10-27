//! 普通路径的借用句柄。

use crate::inner::Inner;
use crate::store::StoreCtx;
use corosensei::Yielder;
use std::cell::Cell;
use std::future::Future;
use std::ptr::NonNull;
use std::rc::Rc;

/// 挂起能力的轻量句柄。只在 `body` 调用期间有效，不可克隆。
pub struct SuspendCtx<'a> {
	pub(crate) yielder: &'a Yielder<(), ()>,
	pub(crate) inner: Rc<Inner>,
}

impl SuspendCtx<'_> {
	/// 在协程内同步驱动 Future。`Pending` 时 `suspend`，`Ready` 时返回。
	///
	/// # Panics
	/// 若 `waker` 未设置（`SuspendCtx` 路径理论上不会发生）。
	pub fn block_on<F: Future>(&self, f: F) -> F::Output {
		self.inner
			.run_until_ready(f, || {
				self.yielder.suspend(());
				Ok(())
			})
			.unwrap_or_else(|e| panic!("SuspendCtx::block_on: {e}"))
	}

	/// 擦除生命周期，产生可逃逸的 [`StoreCtx`]。
	#[must_use]
	pub fn to_store(&self) -> StoreCtx {
		StoreCtx {
			yielder: Cell::new(Some(NonNull::from(self.yielder))),
			inner: self.inner.clone(),
		}
	}
}
