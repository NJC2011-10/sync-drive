//! 逃逸路径的句柄。唯一不安全点在这里。

use crate::error::SuspendError;
use crate::handle::SuspendCapability;
use crate::inner::Inner;
use crate::state::InternalLifecycle;
use corosensei::Yielder;
use std::ptr::NonNull;
use std::rc::Rc;

/// 可跨作用域存活的挂起句柄。`Clone` 廉价，所有克隆指向同一条栈。
#[derive(Clone)]
pub struct StoreCtx {
    pub(crate) yielder: NonNull<Yielder<(), ()>>,
    pub(crate) inner: Rc<Inner>,
}

impl SuspendCapability for StoreCtx {
    #[inline]
    fn inner(&self) -> &Inner {
        &self.inner
    }

    fn with_yielder<R>(&self, f: impl FnOnce(&Yielder<(), ()>) -> R) -> Result<R, SuspendError> {
        match self.inner.lifecycle.get() {
            InternalLifecycle::Suspendable => {}
            other => return Err(SuspendError::NotInResume(other.to_public())),
        }
        if !self.inner.current_stack_in_bounds() {
            return Err(SuspendError::CrossCoroutine);
        }
        let ptr = self.yielder;
        // SAFETY: lifecycle == Suspendable 且 SP 在本协程栈内 ⟹ 栈帧有效；
        // 指针非空由 NonNull 类型保证。panicking 已由 suspend_with 检查。
        // HRTB 闭包约束保证 &Yielder 不逃逸。
        Ok(f(unsafe { ptr.as_ref() }))
    }
}
