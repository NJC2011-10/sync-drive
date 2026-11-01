/// 协程的生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// 初始值。`StoreCtx`/`SuspendCtx` 只能在 `body` 内拿到，此时 lifecycle
    /// 已是 `Running`，所以这个状态永远不会出现在 `SuspendError` 中。
    NotStarted,
    Running,
    Suspended,
    Done,
    Poisoned,
}

/// `StackRuntime` 的 poll 视角状态。和 `Inner::lifecycle` 是两个维度：
/// 这里管“能否 poll”，`lifecycle` 管“能否 suspend”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// 可 poll，`coroutine` 为 `Some`。
    Running,
    /// 不可 poll，`coroutine` 仍为 `Some`（栈保留，等 `into_stack` 取回或 drop 时释放）。
    Done,
    /// 不可 poll，`coroutine` 为 `None`（panic 后已 drop）。
    Poisoned,
}
