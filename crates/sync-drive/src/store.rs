//! 逃逸路径的句柄。唯一不安全点在这里。

use crate::error::SuspendError;
use crate::handle::SuspendCapability;
use crate::inner::Inner;
use crate::state::Lifecycle;
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
            Lifecycle::Running => {}
            other => return Err(SuspendError::NotInResume(other)),
        }
        let sp = psm::stack_pointer() as usize;
        match self.inner.trap_check.get() {
            Some(check) if check(sp) => {}
            // 调用方错误：unsafe 绕过 !Send 把 StoreCtx 带进 on_parent_stack
            // 闭包，SP 在父栈。保留 Err 以守住 try_block_on 的契约。
            Some(_) => return Err(SuspendError::CrossCoroutine),
            // 内部不变量破坏：with_stack 用 OnceCell::set() 保证构造时设置。
            None => unreachable!("trap_check not initialized (construction logic bug)"),
        }
        let ptr = self.yielder;
        // SAFETY: lifecycle == Running 且 SP 在本协程栈内 ⟹ 栈帧有效；
        // 指针非空由 NonNull 类型保证。panicking 已由 suspend_with 检查。
        // HRTB 闭包约束保证 &Yielder 不逃逸。
        Ok(f(unsafe { ptr.as_ref() }))
    }
}
