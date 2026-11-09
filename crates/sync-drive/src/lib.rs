#![doc = include_str!("../README.md")]

extern crate self as sync_drive;

mod ctx;
mod error;
mod handle;
mod inner;
mod runtime;
mod state;
mod store;

pub use corosensei::stack::DefaultStack;
pub use ctx::SuspendCtx;
pub use error::SuspendError;
pub use handle::SuspendHandle;
pub use runtime::StackRuntime;
pub use state::Lifecycle;
pub use store::StoreCtx;

#[cfg(feature = "macros")]
pub use sync_drive_macro::blocking;

/// 默认协程栈大小：2 MiB。
pub const DEFAULT_STACK_SIZE: usize = 2 * 1024 * 1024;
