use proc_macro::TokenStream;
use quote::quote;
use syn::{ItemFn, parse_macro_input};

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
	let ctx_arg: syn::FnArg = syn::parse_quote! { ctx: &::sync_drive::SuspendCtx };
	let insert_at = usize::from(matches!(
		new_sig.inputs.first(),
		Some(syn::FnArg::Receiver(_))
	));
	new_sig.inputs.insert(insert_at, ctx_arg);

	let expanded = quote! {
		#(#attrs)*
		#vis #new_sig {
			ctx.block_on(async move #body)
		}
	};

	expanded.into()
}
