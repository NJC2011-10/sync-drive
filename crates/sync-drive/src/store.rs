//! 逃逸路径的句柄。唯一不安全点在这里。

use crate::error::SuspendError;
use crate::handle::SuspendCapability;
use crate::inner::Inner;
use corosensei::Yielder;
use std::ptr::NonNull;
use std::rc::Rc;

/// 可跨作用域存活的挂起句柄。`Clone` 廉价，所有克隆指向同一条栈。
#[derive(Clone)]
pub struct StoreCtx {
    yielder: NonNull<Yielder<(), ()>>,
    inner: Rc<Inner>,
}

impl StoreCtx {
    #[must_use]
    pub(crate) fn from_parts(yielder: &Yielder<(), ()>, inner: Rc<Inner>) -> Self {
        Self {
            yielder: NonNull::from(yielder),
            inner,
        }
    }
}

impl SuspendCapability for StoreCtx {
    fn inner(&self) -> &Inner {
        &self.inner
    }

    fn with_yielder<R>(&self, f: impl FnOnce(&Yielder<(), ()>) -> R) -> Result<R, SuspendError> {
        if !self.inner.is_in_resume() {
            return Err(SuspendError::NotInResume);
        }
        if !self.inner.current_stack_in_bounds() {
            return Err(SuspendError::CrossCoroutine);
        }
        let ptr = self.yielder;
        // SAFETY: is_in_resume() == true 且 SP 在本协程栈内 ⟹ 栈帧有效；
        // 指针非空由 NonNull 类型保证。panicking 已由 suspend_with 检查。
        // HRTB 闭包约束保证 &Yielder 不逃逸。
        Ok(f(unsafe { ptr.as_ref() }))
    }
}
