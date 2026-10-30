//! 栈式协程桥接内核：让调用者用同步写法驱动异步操作，线程不被阻塞。

extern crate self as sync_drive;

mod ctx;
mod handle;
mod inner;
mod runtime;
mod store;

pub use ctx::SuspendCtx;
pub use handle::SuspendHandle;
pub use runtime::StackRuntime;
pub use store::StoreCtx;
pub use sync_drive_macro::blocking;

/// 默认协程栈大小。
pub const DEFAULT_STACK_SIZE: usize = 2 * 1024 * 1024;
