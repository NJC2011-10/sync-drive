//! 普通路径的借用句柄。

use crate::handle::SuspendCapability;
use crate::inner::{Inner, SuspendError};
use crate::store::StoreCtx;
use corosensei::Yielder;
use std::cell::Cell;
use std::ptr::NonNull;
use std::rc::Rc;

/// 挂起能力的轻量句柄。只在 `body` 调用期间有效，不可克隆。
pub struct SuspendCtx<'a> {
	pub(crate) yielder: &'a Yielder<(), ()>,
	pub(crate) inner: Rc<Inner>,
}

impl SuspendCapability for SuspendCtx<'_> {
	#[inline]
	fn inner(&self) -> &Inner {
		&self.inner
	}

	#[inline]
	fn with_yielder<R>(&self, f: impl FnOnce(&Yielder<(), ()>) -> R) -> Result<R, SuspendError> {
		Ok(f(self.yielder))
	}
}

impl SuspendCtx<'_> {
	/// 擦除生命周期，产生可逃逸的 [`StoreCtx`]。
	#[inline]
	#[must_use]
	pub fn to_store(&self) -> StoreCtx {
		StoreCtx {
			yielder: Cell::new(Some(NonNull::from(self.yielder))),
			inner: self.inner.clone(),
		}
	}
}
