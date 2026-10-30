**把 `async fn` 包装成协程内可调用的同步函数。**

```rust
#[blocking]
async fn fetch(url: String) -> String { ... }
```

展开为：

```rust
fn fetch(ctx: &::sync_drive::SuspendCtx<'_>, url: String) -> String {
    ::sync_drive::SuspendHandle::block_on(ctx, async move { ... })
}
```

调用者写 fetch(&ctx, url)，得到同步返回值——内部走 block_on，Pending 时挂起整条栈。

安装

```toml
[dependencies]
sync-drive = "0.1"
sync-drive-macro = "0.1"
```

sync-drive 已经 re-export 了 blocking 宏，通常不需要直接依赖 sync-drive-macro：

```rust
use sync_drive::blocking;   // ← 从这里拿
```

用法

```rust
use sync_drive::{blocking, StackRuntime, SuspendHandle};

#[blocking]
async fn fetch(url: String) -> String {
    reqwest::get(url).await.unwrap().text().await.unwrap()
}

#[blocking]
async fn safe_read(path: String) -> std::io::Result<Vec<u8>> {
    tokio::fs::read(path).await
}

let rt = StackRuntime::new(|ctx| -> std::io::Result<()> {
    let html = fetch(&ctx, "https://example.com".into());
    let bytes = safe_read(&ctx, "a.txt".into())?;   // ? 正常用
    Ok(())
}).unwrap();
```

展开规则

· 第一个参数自动插入 ctx: &SuspendCtx<'_>；
· 返回类型不变；
· 函数体包进 SuspendHandle::block_on(ctx, async move { ... })；
· 原始属性（#[allow] 等）保留；
· self 方法：ctx 插在 self 之后。

不支持

· 泛型 / 生命周期参数 / where 子句
· impl Trait 参数 / 返回值
· mut 参数模式

需要泛型时手写闭包：

```rust
fn double<T: std::ops::Add<Output = T> + Copy>(ctx: &SuspendCtx<'_>, x: T) -> T {
    ctx.block_on(async move { x + x })
}
```

许可证

MIT OR Apache-2.0