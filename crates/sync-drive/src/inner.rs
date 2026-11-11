//! 协程执行环境与两个方向的驱动逻辑。

use crate::error::SuspendError;
use crate::state::InternalLifecycle;
use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult};
use std::cell::{Cell, OnceCell, RefCell};
use std::future::Future;
use std::task::{Context, Poll, Waker};

/// 一条协程的执行环境。
pub(crate) struct Inner {
    pub(crate) waker: RefCell<Option<Waker>>,
    pub(crate) lifecycle: Cell<InternalLifecycle>,
    pub(crate) trap_check: OnceCell<Box<dyn Fn(usize) -> bool>>,
}

impl Inner {
    pub(crate) fn new() -> Self {
        Self {
            waker: RefCell::new(None),
            lifecycle: Cell::new(InternalLifecycle::NotStarted),
            trap_check: OnceCell::new(),
        }
    }

    #[inline]
    pub(crate) fn set_waker(&self, w: &Waker) {
        *self.waker.borrow_mut() = Some(w.clone());
    }

    /// # Panics
    /// 若 `waker` 未设置（`body` 执行前未 `set_waker`，属内部不变量违反）。
    #[inline]
    fn get_waker(&self) -> Waker {
        self.waker
            .borrow()
            .clone()
            .expect("waker is always set before body runs")
    }

    /// 异步转同步：poll future 直到 `Ready`，`Pending` 时调用 `suspend`。
    ///
    /// `panicking` 检查由调用方的 `suspend` 闭包负责，不在此处。
    pub(crate) fn run_until_ready<F, S>(
        &self,
        f: F,
        mut suspend: S,
    ) -> Result<F::Output, SuspendError>
    where
        F: Future,
        S: FnMut() -> Result<(), SuspendError>,
    {
        let mut f = std::pin::pin!(f);
        loop {
            let waker = self.get_waker();
            let mut cx = Context::from_waker(&waker);
            match f.as_mut().poll(&mut cx) {
                Poll::Ready(v) => return Ok(v),
                Poll::Pending => suspend()?,
            }
        }
    }

    /// 同步转异步：resume 协程一次，显式写 `lifecycle`。
    ///
    /// # Panics
    ///
    /// 协程内 panic 会经 unwind 传播，`ResumeGuard` 在展开中把 `lifecycle`
    /// 写为 `Done`。若协程已结束（绕过 `poll` 的 `done()` 检查直接调用），
    /// `Coroutine::resume` 会 panic。
    pub(crate) fn resume_once<T>(&self, co: &mut Coroutine<(), (), T, DefaultStack>) -> Poll<T> {
        self.lifecycle.set(InternalLifecycle::Suspendable);
        let guard = ResumeGuard(&self.lifecycle);

        match co.resume(()) {
            CoroutineResult::Yield(()) => {
                guard.commit(InternalLifecycle::Waiting);
                Poll::Pending
            }
            CoroutineResult::Return(t) => Poll::Ready(t),
        }
    }

    /// 指定 SP 是否在本协程栈范围内（含 guard page）。
    ///
    /// # Panics
    /// 若 `trap_check` 未初始化（构造逻辑错误，不应发生）。
    #[inline]
    pub(crate) fn stack_ptr_in_bounds(&self, sp: usize) -> bool {
        self.trap_check
            .get()
            .expect("trap_check set exactly once at construction")(sp)
    }

    /// 当前执行栈是否在本协程栈范围内。
    #[inline]
    pub(crate) fn current_stack_in_bounds(&self) -> bool {
        self.stack_ptr_in_bounds(psm::stack_pointer() as usize)
    }
}

/// resume 期间的状态守卫。正常路径 `commit` 跳过，panic 时 `Drop` 兜底写 `Done`。
struct ResumeGuard<'a>(&'a Cell<InternalLifecycle>);

impl ResumeGuard<'_> {
    /// 提交到 `Waiting`，跳过 `Drop` 兜底。
    fn commit(self, state: InternalLifecycle) {
        self.0.set(state);
        std::mem::forget(self);
    }
}

impl Drop for ResumeGuard<'_> {
    fn drop(&mut self) {
        self.0.set(InternalLifecycle::Done);
    }
}
