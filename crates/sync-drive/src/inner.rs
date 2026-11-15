//! 协程执行环境与两个方向的驱动逻辑。

use crate::error::SuspendError;
use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult};
use std::cell::{Cell, OnceCell, RefCell};
use std::future::Future;
use std::task::{Context, Poll, Waker};

type TrapCheckFn = Box<dyn Fn(usize) -> bool>;

/// 一条协程的执行环境。
pub struct Inner {
    waker: RefCell<Option<Waker>>,
    in_resume: Cell<bool>,
    trap_check: OnceCell<TrapCheckFn>,
}

impl Inner {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            waker: RefCell::new(None),
            in_resume: Cell::new(false),
            trap_check: OnceCell::new(),
        }
    }

    pub(crate) fn set_trap_check(&self, trap_check: TrapCheckFn) {
        self.trap_check
            .set(trap_check)
            .ok()
            .expect("trap_check set exactly once at construction");
    }

    pub(crate) fn set_waker(&self, w: &Waker) {
        *self.waker.borrow_mut() = Some(w.clone());
    }

    /// # Panics
    /// 若 `waker` 未设置（`body` 执行前未 `set_waker`，属内部不变量违反）。
    #[must_use]
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
    /// 协程内 panic 会经传播，`ResumeGuard` 在展开时把 `in_resume` 设为 `false`
    pub(crate) fn resume_once<T>(&self, co: &mut Coroutine<(), (), T, DefaultStack>) -> Poll<T> {
        let _guard = ResumeGuard::new(&self.in_resume);

        match co.resume(()) {
            CoroutineResult::Yield(()) => Poll::Pending,
            CoroutineResult::Return(t) => Poll::Ready(t),
        }
    }

    /// 指定 SP 是否在本协程栈范围内（含 guard page）。
    ///
    /// 具体含义请看
    ///
    /// # Panics
    /// 若 `trap_check` 未初始化（构造逻辑错误，不应发生）。
    #[inline]
    #[must_use]
    pub fn stack_ptr_in_bounds(&self, sp: usize) -> bool {
        self.trap_check
            .get()
            .expect("trap_check set exactly once at construction")(sp)
    }

    /// 当前执行栈是否在本协程栈范围内。
    #[inline]
    #[must_use]
    pub fn current_stack_in_bounds(&self) -> bool {
        self.stack_ptr_in_bounds(psm::stack_pointer() as usize)
    }

    /// 是否处在一次 `resume` 中。
    #[inline]
    #[must_use]
    pub fn is_in_resume(&self) -> bool {
        self.in_resume.get()
    }
}

/// resume 期间的状态守卫。存活期间 `in_resume` 设为 `true`。
#[must_use]
struct ResumeGuard<'a>(&'a Cell<bool>);

impl<'a> ResumeGuard<'a> {
    fn new(in_resume: &'a Cell<bool>) -> Self {
        in_resume.set(true);
        Self(in_resume)
    }
}

impl Drop for ResumeGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
