/// 协程的内部生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InternalLifecycle {
    /// 已创建，尚未首次 resume。
    NotStarted,
    /// 在 resume 内执行，可以 suspend。
    Suspendable,
    /// 已挂起，等待 waker 唤醒。
    Waiting,
    /// 正常完成。
    Done,
    /// panic 后。
    Poisoned,
}

impl InternalLifecycle {
    /// 投影到公开面。`NotStarted` 在 `body` 外不可达，不会走到这个分支。
    pub(crate) fn to_public(self) -> Lifecycle {
        match self {
            Self::NotStarted => not_started_unreachable(),
            Self::Suspendable => Lifecycle::Suspendable,
            Self::Waiting => Lifecycle::Waiting,
            Self::Done => Lifecycle::Done,
            Self::Poisoned => Lifecycle::Poisoned,
        }
    }
}

/// `NotStarted` 投影到公开面时的终止。结构上不可达，触发即库 bug。
#[cold]
#[inline(never)]
fn not_started_unreachable() -> ! {
    unreachable!("InternalLifecycle::NotStarted is not observable from outside")
}

/// 协程的生命周期状态。外部可观察的四种状态。
///
/// 调用 `lifecycle()` 时协程一定已启动，因此不存在“未启动”变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    /// 在 resume 内执行，可以调用 `block_on` / `on_parent_stack`。
    Suspendable,
    /// 已挂起，等待 waker 唤醒。
    Waiting,
    /// 正常完成。
    Done,
    /// panic 后。
    Poisoned,
}

/// `StackRuntime` 的 poll 视角状态。和 `Lifecycle` 是两个维度：
/// 这里管“能否 poll”，`Lifecycle` 管“能否 suspend”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    /// 可 poll，`coroutine` 为 `Some`。
    Running,
    /// 不可 poll，`coroutine` 仍为 `Some`（栈保留，等 `into_stack` 取回或 drop 时释放）。
    Done,
    /// 不可 poll，`coroutine` 为 `None`（panic 后已 drop）。
    Poisoned,
}
