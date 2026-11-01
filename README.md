**栈式协程桥接内核：让同步 C API 引擎用同步风格调用 Rust async，线程不被阻塞。**

[![Crates.io](https://img.shields.io/crates/v/sync-drive.svg)](https://crates.io/crates/sync-drive)
[![Docs.rs](https://docs.rs/sync-drive/badge.svg)](https://docs.rs/sync-drive)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#许可证)

`sync-drive` 用 [corosensei](https://crates.io/crates/corosensei) 的栈式协程，把异步 Future 翻译成同步调用——挂起的是**整条 C 调用栈**（不是编译器生成的状态机），因此可以桥接 QuickJS、Lua 等同步 C API 引擎。

## 为什么需要它

同步 C API 引擎（QuickJS 的 `eval`、Lua 的 `lua_pcall`）的调用链是一条真实的 C 栈，native 函数必须当场返回。Rust 的 `async/await` 只能挂起编译器生成的状态机，**挂不住手写的 C 调用链**。

`sync-drive` 挂起整条栈：`suspend()` 冻结 `JS_CallInternal` 的 C 帧、Rust 帧、JS 帧，`resume()` 从原位精确恢复。已在 QuickJS 0.14 上验证 Rust → JS 回调 → Rust 的两次嵌套 suspend 正确工作。

## Workspace 结构

```

sync-drive/
├── Cargo.toml              # 虚拟 workspace root
└── crates/
├── sync-drive/         # 核心库
└── sync-drive-macro/   # #[blocking] 属性宏

```

| crate | 职责 | 文档 |
|---|---|---|
| [`sync-drive`](crates/sync-drive) | 核心库：`StackRuntime` / `SuspendCtx` / `StoreCtx` / `SuspendHandle` | [docs.rs](https://docs.rs/sync-drive/latest/sync_drive/) |
| [`sync-drive-macro`](crates/sync-drive-macro) | `#[blocking]` 属性宏：`async fn` → `fn(ctx, ...)` | [docs.rs](https://docs.rs/sync-drive-macro/latest/sync_drive_macro/) |

外部探针 [rquickjs-ctx-probe](https://github.com/NJC2011-10/rquickjs-ctx-probe) 不在 workspace 内，它依赖 `crates/sync-drive` 验证 QuickJS 集成，不跟随产品发布。

## 构建与测试

```bash
cargo build
cargo test
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic
```

边界

· 单线程 !Send：不能 tokio::spawn，必须 spawn_local / current_thread + LocalSet。
· aarch64-pc-windows-msvc：corosensei 不支持。
· 深递归：受 stack_size 限制（默认 2MB），无运行时深度检测。
· 挂起时 Drop 内再次 suspend：corosensei 明确 abort。

许可证

MIT OR Apache-2.0
