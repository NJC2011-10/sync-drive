//! executor 适配层。

use crate::DEFAULT_STACK_SIZE;
use crate::ctx::SuspendCtx;
use crate::inner::Inner;
use crate::state::InternalLifecycle;
use corosensei::Coroutine;
use corosensei::stack::DefaultStack;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

/// 栈所有者。实现 `Future`，由 executor 驱动。
pub struct StackRuntime<T> {
    coroutine: Option<Coroutine<(), (), T, DefaultStack>>,
    inner: Rc<Inner>,
}

impl<T: 'static> StackRuntime<T> {
    /// 用默认栈大小创建。
    ///
    /// # Errors
    #[inline]
    pub fn new<F>(body: F) -> Result<Self, std::io::Error>
    where
        F: FnOnce(SuspendCtx<'_>) -> T + 'static,
    {
        Self::with_stack_size(body, DEFAULT_STACK_SIZE)
    }

    /// 指定栈大小创建。
    ///
    /// # Errors
    #[inline]
    pub fn with_stack_size<F>(body: F, stack_size: usize) -> Result<Self, std::io::Error>
    where
        F: FnOnce(SuspendCtx<'_>) -> T + 'static,
    {
        let stack = DefaultStack::new(stack_size)?;
        Ok(Self::with_stack(body, stack))
    }

    /// 用已有的栈创建。`stack` 通常来自 [`into_stack`](Self::into_stack) 的返回值。
    ///
    /// # Panics
    /// 若内部构造逻辑被破坏（不应发生）。
    pub fn with_stack<F>(body: F, stack: DefaultStack) -> Self
    where
        F: FnOnce(SuspendCtx<'_>) -> T + 'static,
    {
        let inner = Rc::new(Inner::new());
        let inner_for_co = inner.clone();
        let co = Coroutine::with_stack(stack, move |yielder, ()| {
            body(SuspendCtx {
                yielder,
                inner: inner_for_co,
            })
        });

        // trap_handler 带泛型 T，Inner 不带 T——用 Box<dyn Fn> 擦除后存入，
        // 供 StoreCtx 的 SP 检查使用。
        let handler = co.trap_handler();
        inner
            .trap_check
            .set(Box::new(move |sp| handler.stack_ptr_in_bounds(sp)))
            .ok()
            .expect("trap_check set exactly once at construction");

        Self {
            coroutine: Some(co),
            inner,
        }
    }

    /// 消费 `self`，取回协程使用的栈。
    ///
    /// 取回的 [`DefaultStack`] 可以传给 [`with_stack`](Self::with_stack)
    /// 复用，避免重新分配。
    ///
    /// # Panics
    ///
    /// 若协程尚未完成（运行中或挂起中），`corosensei` 不允许提取栈，会 panic：
    ///
    /// ```text
    /// cannot extract stack from an incomplete coroutine
    /// ```
    ///
    /// 调用前需先 `poll` 到 `Ready` 或 panic，或 `drop` 本对象让栈随协程展开释放。
    ///
    /// # 消费 `self`
    ///
    /// 调用后 `StackRuntime` 不可再用。逃逸的 [`StoreCtx`](crate::StoreCtx)
    /// 此后再调 `block_on` 会得到 `NotInResume(Done)`。
    #[must_use]
    pub fn into_stack(mut self) -> DefaultStack {
        self.coroutine
            .take()
            .expect("coroutine is always Some until into_stack")
            .into_stack()
    }
}

impl<T> Future for StackRuntime<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let this = self.get_mut();

        let co = this.coroutine.as_mut().expect("coroutine is always Some");
        assert!(!co.done(), "StackRuntime polled after completion");

        this.inner.set_waker(cx.waker());

        this.inner.resume_once(co)
    }
}

impl<T> Drop for StackRuntime<T> {
    fn drop(&mut self) {
        // coroutine 由 Option 自动 drop，这里只更新 lifecycle：
        // 标记 Done 让逃逸的 StoreCtx 事后调用 block_on 得到 NotInResume(Done) 而非 UB。
        match self.inner.lifecycle.get() {
            InternalLifecycle::NotStarted
            | InternalLifecycle::Suspendable
            | InternalLifecycle::Waiting => {
                self.inner.lifecycle.set(InternalLifecycle::Done);
            }
            InternalLifecycle::Done | InternalLifecycle::Poisoned => {}
        }
    }
}

#[cfg(test)]
mod pin_tests {
    use super::StackRuntime;

    fn assert_unpin<T: Unpin>() {}

    #[test]
    fn stack_runtime_is_unpin() {
        assert_unpin::<StackRuntime<()>>();
    }
}
