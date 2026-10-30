//! 逃逸路径的句柄。唯一不安全点在这里。

use crate::handle::SuspendCapability;
use crate::inner::{Inner, Lifecycle, SuspendError};
use corosensei::Yielder;
use std::cell::Cell;
use std::ptr::NonNull;
use std::rc::Rc;

/// 可跨作用域存活的挂起句柄。`Clone` 廉价，所有克隆指向同一条栈。
#[derive(Clone)]
pub struct StoreCtx {
	pub(crate) yielder: Cell<Option<NonNull<Yielder<(), ()>>>>,
	pub(crate) inner: Rc<Inner>,
}

impl SuspendCapability for StoreCtx {
	#[inline]
	fn inner(&self) -> &Inner {
		&self.inner
	}

	fn with_yielder<R>(&self, f: impl FnOnce(&Yielder<(), ()>) -> R) -> Result<R, SuspendError> {
		match self.inner.lifecycle.get() {
			Lifecycle::Running => {}
			other => return Err(SuspendError::NotInResume(other)),
		}
		let sp = psm::stack_pointer() as usize;
		match self.inner.trap_check.get() {
			Some(check) if check(sp) => {}
			_ => return Err(SuspendError::WrongStack),
		}
		let ptr = self
			.yielder
			.get()
			.expect("StoreCtx yielder is never cleared");
		// SAFETY: lifecycle == Running 且 SP 在本协程栈内 ⟹ 栈帧有效；
		// 指针非空。panicking 已由 suspend_with 检查。
		// HRTB 闭包约束保证 &Yielder 不逃逸。
		Ok(f(unsafe { ptr.as_ref() }))
	}
}
