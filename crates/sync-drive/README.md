**栈式协程桥接内核：让调用者用同步写法驱动异步操作，线程不被阻塞。**

`StackRuntime` 实现 `Future`，交给 executor 驱动；`body` 闭包用 `ctx.block_on(...)` 同步驱动异步 Future；`Pending` 时冻结整条栈，`resume` 后从原位恢复。

## 安装

```toml
[dependencies]
sync-drive = "0.1"
```

快速开始

```rust
use sync_drive::{StackRuntime, SuspendHandle};
use tokio::task::LocalSet;

async fn fetch(url: String) -> String {
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    format!("<response from {url}>")
}

let rt = StackRuntime::new(|ctx| {
    let a = ctx.block_on(fetch("a".into()));
    let b = ctx.block_on(fetch("b".into()));
    format!("{a} | {b}")
}).expect("failed to allocate 2MB coroutine stack");

let local = LocalSet::new();
local.block_on(&tokio::runtime::Builder::new_current_thread().build()?, async {
    let result = rt.await;
    assert!(result.contains("response from a"));
});
# Ok::<(), Box<dyn std::error::Error>>(())
```

API

StackRuntime<T>

栈所有者，实现 Future<Output = T>。

方法 说明
new(body) 用默认栈大小（2MB）创建。body: FnOnce(SuspendCtx<'_>) -> T + 'static
with_stack_size(body, size) 指定栈大小
with_stack(body, stack) 用已有 DefaultStack 创建（配合 into_stack 复用）
into_stack(self) -> Option<DefaultStack> 消费 self 取回栈；Poisoned 时返回 None

SuspendHandle（trait）

所有句柄共享的挂起能力。

方法 说明
block_on(f) 在协程内同步驱动 Future 到完成。Pending 时挂起整条栈
on_parent_stack(f) 在父栈上执行闭包（不消耗协程栈）

SuspendCtx<'a> / StoreCtx

类型 生命周期 Clone 用途
SuspendCtx<'a> 绑定 body 调用期 ❌ 普通路径，block_on 零 unsafe
StoreCtx 'static，可逃逸 ✅ 逃逸路径（如存进 QuickJS 的 userdata），运行时检查 lifecycle + SP

SuspendCtx::to_store() 把 &'a Yielder 擦除成 NonNull，产生可跨作用域的 StoreCtx。

与 #[blocking] 宏配合

```rust
use sync_drive::{blocking, StackRuntime, SuspendHandle};

#[blocking]
async fn double(x: i32) -> i32 { x * 2 }

let rt = StackRuntime::new(|ctx| double(&ctx, 21)).unwrap();
```

详见 sync-drive-macro。

Panics / Aborts

SuspendHandle::block_on 和 on_parent_stack：

· # Panics：协程未启动 / 已结束 / 已 panic（lifecycle != Running）、当前栈不属于该协程（WrongStack）。不可恢复。
· # Aborts：从 Drop 实现里调用（panic 展开中），unwind 中不能切栈，可能导致进程 abort。不要在任何 Drop 实现里调用这些方法。

future 立即 Ready 时不要求协程上下文，可在协程外调用。

边界

· 单线程 !Send：不能 tokio::spawn，必须 spawn_local / current_thread + LocalSet。
· aarch64-pc-windows-msvc：corosensei 不支持。
· 深递归：受 stack_size 限制，无运行时深度检测。
· 挂起时 Drop 内再次 suspend：corosensei 明确 abort。

许可证

MIT OR Apache-2.0
