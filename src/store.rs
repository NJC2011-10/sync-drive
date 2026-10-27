//! 逃逸路径的句柄。唯一不安全点在这里。

use crate::inner::{Inner, Lifecycle, SuspendError};
use corosensei::Yielder;
use std::cell::Cell;
use std::future::Future;
use std::ptr::NonNull;
use std::rc::Rc;

/// 可跨作用域存活的挂起句柄。`Clone` 廉价，所有克隆指向同一条栈。
#[derive(Clone)]
pub struct StoreCtx {
	pub(crate) yielder: Cell<Option<NonNull<Yielder<(), ()>>>>,
	pub(crate) inner: Rc<Inner>,
}

impl StoreCtx {
	/// 在协程内同步驱动 Future。`Pending` 时三项检查后 `suspend`，`Ready` 时返回。
	///
	/// # Panics
	/// - 协程未启动、挂起中、已结束或已 panic（`lifecycle != Running`）；
	/// - 处于 panic 展开中；
	/// - `yielder` 指针已清空。
	pub fn block_on<F: Future>(&self, f: F) -> F::Output {
		self.inner
			.run_until_ready(f, || {
				match self.inner.lifecycle.get() {
					Lifecycle::Running => {}
					other => return Err(SuspendError::NotInResume(other)),
				}
				if std::thread::panicking() {
					return Err(SuspendError::Panicking);
				}
				let ptr = self.yielder.get().ok_or(SuspendError::YielderCleared)?;
				// SAFETY: lifecycle == Running 保证协程正在 resume 内，栈帧有效；
				// 非 panicking 保证不在 unwind 中切栈；指针非空。&Yielder 是
				// 闭包局部，不逃逸。
				unsafe { ptr.as_ref() }.suspend(());
				Ok(())
			})
			.unwrap_or_else(|e| panic!("StoreCtx::block_on: {e}"))
	}
}
