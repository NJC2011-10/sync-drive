use crate::state::Lifecycle;

/// 协程挂起或驱动失败的原因。
#[derive(Debug, Clone, Copy)]
pub enum SuspendError {
    /// 协程当前不可 suspend。携带实际状态便于诊断。
    NotInResume(Lifecycle),
    /// 处于 panic 展开中。`block_on` 路径下：有 `catch_unwind` 包裹时被捕获，
    /// 无包裹时 double panic → abort。不是“正常错误”，是“契约违反后的终止”。
    Panicking,
    /// 当前执行栈不属于本 `StoreCtx` 所属的协程。
    CrossCoroutine,
}

impl std::fmt::Display for SuspendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInResume(l) => write!(f, "not inside resume (lifecycle: {l:?})"),
            Self::Panicking => write!(f, "cannot suspend during panic unwinding"),
            Self::CrossCoroutine => write!(f, "cross-coroutine suspend attempt"),
        }
    }
}

impl std::error::Error for SuspendError {}
