#![doc = include_str!("../README.md")]

use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

/// 属性宏：把 `async fn` 展开为同步函数 `fn(ctx: &SuspendCtx<'_>, ...)`。
///
/// # 依赖名约束
///
/// 宏展开引用 `::sync_drive::SuspendHandle`，因此**调用方 crate 必须以
/// `sync-drive` 名称依赖本库**。
///
/// 重命名依赖（如
/// `my-drive = { package = "sync-drive", version = "0.1" }`）会导致宏展开
/// 找不到 `::sync_drive`，编译报 `E0433: use of undeclared crate or
/// module 'sync_drive'`。错误会指向 `#[blocking]` 标注处，但根因在
/// `Cargo.toml`。
///
/// 当前版本不支持重命名自动解析。若需要重命名，请改用
/// `sync_drive::SuspendHandle::block_on` 手写，或暂不重命名依赖。
#[proc_macro_attribute]
pub fn blocking(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    // 如果不是 async fn，直接编译报错
    if input.sig.asyncness.is_none() {
        return syn::Error::new_spanned(&input.sig, "#[blocking] 只能用于 async fn")
            .to_compile_error()
            .into();
    }

    let attrs = &input.attrs;
    let vis = &input.vis;
    let sig = &input.sig;
    let body = &input.block;

    // 1. 去掉 async
    let mut new_sig = sig.clone();
    new_sig.asyncness = None;

    // 2. 插入 ctx: &SuspendCtx；方法（有 self）插到 self 之后
    let ctx_arg: syn::FnArg = syn::parse_quote! { ctx: &::sync_drive::SuspendCtx<'_> };
    let insert_at = usize::from(matches!(
        new_sig.inputs.first(),
        Some(syn::FnArg::Receiver(_))
    ));
    new_sig.inputs.insert(insert_at, ctx_arg);

    let expanded = quote! {
        #(#attrs)*
        #vis #new_sig {
            ::sync_drive::SuspendHandle::block_on(ctx, async move #body)
        }
    };

    expanded.into()
}
