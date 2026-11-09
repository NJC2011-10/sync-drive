//! 各测试文件共享的 helpers。

#![allow(dead_code)]

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use sync_drive::StoreCtx;

/// 手动可控的 Future：首次 poll 返回 Pending 并保存 waker，
/// 之后由外部设置 ready + 唤醒。
pub struct ManualFuture {
    pub ready: Rc<Cell<bool>>,
    pub waker: Rc<RefCell<Option<Waker>>>,
}

impl Future for ManualFuture {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.ready.get() {
            Poll::Ready(())
        } else {
            *self.waker.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

pub fn extract_panic_msg(err: &(dyn Any + Send)) -> String {
    if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else {
        String::new()
    }
}

/// 仅用于测试：`StoreCtx` 是 `!Send`，用 unsafe 包装骗过
/// `on_parent_stack` 的 `F: Send` 约束，以验证 SP 检查是最后一道防线。
pub struct SendStore(pub StoreCtx);
// SAFETY: 仅用于测试，构造 on_parent_stack 内的跨栈 suspend 场景。
unsafe impl Send for SendStore {}
