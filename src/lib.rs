//! 栈式协程桥接内核：让调用者用同步写法驱动异步操作，线程不被阻塞。
//!
//! # 快速开始
//! ```
//! use sync_drive::StackRuntime;
//!
//! let rt = StackRuntime::new(|ctx| {
//!    let v = ctx.block_on(std::future::ready(1));
//!    v + 1
//! }).unwrap();
//! assert_eq!(futures::executor::block_on(rt), 2);
//! ```
//!
//! # 边界
//! - 单线程，`!Send`：协程栈不能换线程恢复。
//! - 不支持跨 `await` 持有 `!Send` 资源以外的特殊语义。
//! - 深递归受 `stack_size` 限制，用 `with_stack_size` 调整。

extern crate self as sync_drive;

mod core;

pub use core::{DEFAULT_STACK_SIZE, StackRuntime, SuspendCtx};
pub use sync_drive_macro::blocking;
