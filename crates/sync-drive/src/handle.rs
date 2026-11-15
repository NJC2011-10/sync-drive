use crate::error::SuspendError;
use crate::inner::Inner;
use corosensei::Yielder;
use std::future::Future;

/// 能力 trait：句柄只实现这个。`pub(crate)` 可见但不导出到 crate 外，
/// 对 `impl<H: SuspendCapability> SuspendHandle for H` 的 blanket impl 可见。
pub(crate) trait SuspendCapability {
    #[must_use]
    fn inner(&self) -> &Inner;
    fn with_yielder<R>(&self, f: impl FnOnce(&Yielder<(), ()>) -> R) -> Result<R, SuspendError>;

    fn suspend_with<R>(&self, f: impl FnOnce(&Yielder<(), ()>) -> R) -> Result<R, SuspendError> {
        if std::thread::panicking() {
            return Err(SuspendError::Panicking);
        }
        self.with_yielder(f)
    }
}

/// 挂起能力：在协程内同步驱动 Future、在父栈执行闭包、查询生命周期状态。
pub trait SuspendHandle {
    /// 在协程内同步驱动 Future。`Pending` 时挂起整条栈，`Ready` 时返回。
    ///
    /// # Panics
    ///
    /// 以下情况是**调用者违反契约，不可恢复**：
    ///
    /// - 协程未启动、已结束或已 panic；
    /// - 当前栈不属于该协程（`CrossCoroutine`）。
    ///
    /// # Aborts
    ///
    /// 在 panic 展开中调用（如从 `Drop` 实现里调 `block_on`）时，unwind 中
    /// 不能切栈，**可能导致进程 abort**。不要在任何 `Drop` 实现里调用这些方法。
    ///
    /// # 注意
    ///
    /// 若 future 立即 `Ready`，不要求协程上下文，可在协程外调用。
    /// 只有 `Pending` 时才检查上述条件。
    fn block_on<F: Future>(&self, f: F) -> F::Output;

    /// 同 [`block_on`](Self::block_on)，但对调用方可控的失败返回 `Err`
    /// 而非 panic。
    ///
    /// # Errors
    ///
    /// 以下均为**调用方错误**，返回 `Err` 而非 panic：
    ///
    /// - [`SuspendError::NotInResume`]：协程未运行 / 已挂起 / 已结束 / 已 panic；
    /// - [`SuspendError::CrossCoroutine`]：当前执行栈不属于该协程；
    /// - [`SuspendError::Panicking`]：处于 panic 展开中。
    ///
    /// future 立即 `Ready` 时返回 `Ok`，不要求协程上下文。
    ///
    /// # Panics
    ///
    /// **内部不变量破坏**（库的 bug，非调用方错误）可能 panic。这类 panic
    /// 意味着库的构造逻辑被绕过，应报 issue。`Err` 不覆盖此类失败。
    fn try_block_on<F: Future>(&self, f: F) -> Result<F::Output, SuspendError>;

    /// 在父栈上执行闭包。
    ///
    /// 闭包执行期间协程仍处于可挂起状态，但执行栈位于父栈。若经
    /// `unsafe` 绕过 `!Send` 在闭包内调用 `try_block_on`，会返回
    /// [`SuspendError::CrossCoroutine`](crate::SuspendError::CrossCoroutine)。
    ///
    /// # Panics / Aborts
    ///
    /// 同 [`block_on`](Self::block_on)。
    fn on_parent_stack<R, F>(&self, f: F) -> R
    where
        F: FnOnce() -> R + Send,
        R: Send;

    /// 给出信息查询接口。
    #[must_use]
    fn inner(&self) -> &Inner;
}

/// 为所有可挂起句柄实现 `SuspendHandle`。
impl<H: SuspendCapability> SuspendHandle for H {
    #[inline]
    fn try_block_on<F: Future>(&self, f: F) -> Result<F::Output, SuspendError> {
        self.inner()
            .run_until_ready(f, || self.suspend_with(|y| y.suspend(())))
    }

    #[inline]
    fn block_on<F: Future>(&self, f: F) -> F::Output {
        self.try_block_on(f)
            .unwrap_or_else(|e| panic!("block_on: {e}"))
    }

    #[inline]
    fn on_parent_stack<R, F>(&self, f: F) -> R
    where
        F: FnOnce() -> R + Send,
        R: Send,
    {
        self.suspend_with(|y| y.on_parent_stack(f))
            .unwrap_or_else(|e| panic!("on_parent_stack: {e}"))
    }

    #[inline]
    fn inner(&self) -> &Inner {
        self.inner()
    }
}
