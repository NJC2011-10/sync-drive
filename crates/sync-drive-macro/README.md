**把 `async fn` 包装成协程内可调用的同步函数。**

```rust,ignore
#[blocking]
async fn fetch(url: String) -> String { ... }
```

展开为：

```rust,ignore
fn fetch(ctx: &::sync_drive::SuspendCtx<'_>, url: String) -> String {
    ::sync_drive::SuspendHandle::block_on(ctx, async move { ... })
}
```

调用者写 fetch(&ctx, url)，得到同步返回值——内部走 `block_on`，`Pending` 时挂起整条栈。

安装

通常不需要直接依赖 sync-drive-macro——sync-drive 已经 re-export 了 blocking 宏：

```toml
[dependencies]
sync-drive = "0.1"
```

```rust,ignore
use sync_drive::blocking;   // ← 从这里拿
```

用法

```rust,ignore
use sync_drive::{blocking, StackRuntime, SuspendHandle};

#[blocking]
async fn fetch(url: String) -> String {
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    format!("<response from {url}>")
}

#[blocking]
async fn safe_parse(s: String) -> std::io::Result<i32> {
    s.parse().map_err(|e| std::io::Error::other(e))
}

let rt = StackRuntime::new(|ctx| -> std::io::Result<()> {
    let html = fetch(&ctx, "https://example.com".into());
    let n = safe_parse(&ctx, "42".into())?;   // ? 正常用
    assert!(html.contains("example.com"));
    assert_eq!(n, 42);
    Ok(())
}).unwrap();
```

展开规则

· 第一个参数自动插入 `ctx: &SuspendCtx<'_>`；
· 返回类型不变；
· 函数体包进 `SuspendHandle::block_on(ctx, async move { ... })`；
· 原始属性（`#[allow]` 等）保留；
· self 方法：ctx 插在 self 之后。

不支持

· 泛型 / 生命周期参数 / where 子句
· impl Trait 参数 / 返回值
· mut 参数模式
· 宏依赖名：`#[blocking]` 展开引用 `::sync_drive`，调用方必须以 `sync-drive`
  名称依赖，重命名依赖会导致宏展开编译失败（E0433）。暂不支持自动解析。

需要泛型时手写闭包：

```rust,ignore
use sync_drive::{SuspendCtx, SuspendHandle};

fn double<T>(ctx: &SuspendCtx<'_>, x: T) -> T
where
    T: std::ops::Add<Output = T> + Copy,
{
    ctx.block_on(async move { x + x })
}
```

许可证

MIT OR Apache-2.0
