//! 普通路径的借用句柄。

use crate::error::SuspendError;
use crate::handle::SuspendCapability;
use crate::inner::Inner;
use crate::store::StoreCtx;
use corosensei::Yielder;
use std::rc::Rc;

/// 挂起能力的轻量句柄。只在 `body` 调用期间有效，不可克隆。
pub struct SuspendCtx<'a> {
    yielder: &'a Yielder<(), ()>,
    inner: Rc<Inner>,
}

impl<'a> SuspendCtx<'a> {
    #[must_use]
    pub(crate) fn from_parts(yielder: &'a Yielder<(), ()>, inner: Rc<Inner>) -> Self {
        Self { yielder, inner }
    }
}

impl SuspendCapability for SuspendCtx<'_> {
    fn inner(&self) -> &Inner {
        &self.inner
    }

    fn with_yielder<R>(&self, f: impl FnOnce(&Yielder<(), ()>) -> R) -> Result<R, SuspendError> {
        Ok(f(self.yielder))
    }
}

impl SuspendCtx<'_> {
    /// 擦除生命周期，产生可逃逸的 [`StoreCtx`]。
    #[inline]
    #[must_use]
    pub fn to_store(&self) -> StoreCtx {
        StoreCtx::from_parts(self.yielder, self.inner.clone())
    }
}
