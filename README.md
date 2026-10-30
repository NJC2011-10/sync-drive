**栈式协程桥接内核：让同步 C API 引擎用同步风格调用 Rust async，线程不被阻塞。**

`sync-drive` 用 [corosensei](https://crates.io/crates/corosensei) 的栈式协程，把异步 Future 翻译成同步调用——挂起的是**整条 C 调用栈**（不是编译器生成的状态机），因此可以桥接 QuickJS、Lua 等同步 C API 引擎。

## 为什么需要它

同步 C API 引擎（QuickJS 的 `eval`、Lua 的 `lua_pcall`）的调用链是一条真实的 C 栈，native 函数必须当场返回。Rust 的 `async/await` 只能挂起编译器生成的状态机，**挂不住手写的 C 调用链**。

`sync-drive` 挂起整条栈：`suspend()` 冻结 `JS_CallInternal` 的 C 帧、Rust 帧、JS 帧，`resume()` 从原位精确恢复。已验证 QuickJS 0.14 上 Rust → JS 回调 → Rust 的两次嵌套 suspend 正确工作。

## Workspace 结构

```

sync-drive/
├── Cargo.toml              # 虚拟 workspace root（无 [package]）
└── crates/
    ├── sync-drive/         # 核心库
    └── sync-drive-macro/   # #[blocking] 属性宏

```

外部探针 [rquickjs-ctx-probe](https://github.com/NJC2011-10/sync-drive/tree/main/rquickjs-ctx-probe) 不在 workspace 内，它依赖 `crates/sync-drive` 验证 QuickJS 集成，不跟随产品发布。

## 快速开始

```toml
[dependencies]
sync-drive = "0.1"
```

```rust
use sync_drive::{StackRuntime, SuspendHandle};

async fn async_add(a: i32, b: i32) -> i32 { a + b }

let rt = StackRuntime::new(|ctx| {
    // 同步写法，内部是异步
    let a = ctx.block_on(async_add(1, 2));
    let b = ctx.block_on(async_add(a, 10));
    b
}).expect("failed to allocate stack");

// rt 实现 Future，交给 LocalSet / current_thread runtime
let result = futures::executor::block_on(rt);
assert_eq!(result, 13);
```

架构

crate 职责
sync-drive 核心库：StackRuntime / SuspendCtx / StoreCtx / SuspendHandle
sync-drive-macro #[blocking] 属性宏：async fn → fn(ctx, ...)

边界（不支持）

· ❌ 跨线程：Rc + 单线程 !Send，协程栈不能换线程恢复。
· ❌ aarch64-pc-windows-msvc：corosensei 不支持。
· ⚠️ 深递归：受 stack_size 限制（默认 2MB），无运行时深度检测。配合 QuickJS 时，QuickJS 的 max_stack_size（默认 256KB）先触发 RangeError。
· ⚠️ 挂起时 Drop 内再次 suspend：corosensei 明确 abort。

构建与测试

```bash
cargo build
cargo test
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
```

许可证

MIT OR Apache-2.0