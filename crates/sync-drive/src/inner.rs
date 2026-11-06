//! 协程执行环境与两个方向的驱动逻辑。

use crate::error::SuspendError;
use crate::state::InternalLifecycle;
use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult};
use std::any::Any;
use std::cell::{Cell, OnceCell, RefCell};
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::task::{Context, Poll, Waker};

/// 一条协程的执行环境。
pub(crate) struct Inner {
    pub(crate) waker: RefCell<Option<Waker>>,
    pub(crate) lifecycle: Cell<InternalLifecycle>,
    pub(crate) trap_check: OnceCell<Box<dyn Fn(usize) -> bool>>,
}

impl Inner {
    #[inline]
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
    pub(crate) fn get_waker(&self) -> Waker {
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

    /// 同步转异步：resume 协程一次，显式写 `lifecycle`，panic 原样返回。
    ///
    /// # Panics
    /// 若协程已处于 `Done` / `Poisoned` 状态（绕过 `poll` 直接调用），
    /// `Coroutine::resume` 会 panic。正常调用路径（`poll`）已在顶部检查状态。
    pub(crate) fn resume_once<T>(
        &self,
        co: &mut Coroutine<(), (), T, DefaultStack>,
    ) -> Result<Poll<T>, Box<dyn Any + Send>> {
        self.lifecycle.set(InternalLifecycle::Suspendable);
        match catch_unwind(AssertUnwindSafe(|| co.resume(()))) {
            Ok(CoroutineResult::Yield(())) => {
                self.lifecycle.set(InternalLifecycle::Waiting);
                Ok(Poll::Pending)
            }
            Ok(CoroutineResult::Return(t)) => {
                self.lifecycle.set(InternalLifecycle::Done);
                Ok(Poll::Ready(t))
            }
            Err(e) => {
                self.lifecycle.set(InternalLifecycle::Poisoned);
                Err(e)
            }
        }
    }

    /// 指定 SP 是否在本协程栈范围内（含 guard page）。
    ///
    /// # Panics
    /// 若 `trap_check` 未初始化（构造逻辑错误，不应发生）。
    pub(crate) fn stack_ptr_in_bounds(&self, sp: usize) -> bool {
        self.trap_check
            .get()
            .expect("trap_check set exactly once at construction")(sp)
    }

    /// 当前执行栈是否在本协程栈范围内。
    pub(crate) fn current_stack_in_bounds(&self) -> bool {
        self.stack_ptr_in_bounds(psm::stack_pointer() as usize)
    }
}
