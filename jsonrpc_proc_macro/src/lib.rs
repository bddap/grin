// Allow JSONRPCServer to be derived for a trait using the '#[jsonrpc_server]' macro.

#![recursion_limit = "256"]

extern crate proc_macro;
use proc_macro2;
use proc_macro2::Span;
use quote::quote;
use syn::spanned::Spanned;
use syn::{
	parse_macro_input, ArgSelfRef, FnArg, FnDecl, Ident, ItemTrait, MethodSig, Pat, TraitItem, Type,
};

#[proc_macro_attribute]
pub fn jsonrpc_server(
	_attr: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	let trait_def = parse_macro_input!(item as ItemTrait);
	let server_impl = match impl_server(&trait_def) {
		Ok(s) => s,
		Err(reject) => {
			reject.raise();
			return proc_macro::TokenStream::new();
		}
	};
	proc_macro::TokenStream::from(quote! {
		#trait_def
		#server_impl
	})
}

// Generate a blanket JSONRPCServer implementation for types implementing trait.
fn impl_server(tr: &ItemTrait) -> Result<proc_macro2::TokenStream, Rejection> {
	let trait_name = &tr.ident;
	let methods: Vec<&MethodSig> = trait_methods(&tr)?;

	for method in methods.iter() {
		if method.ident.to_string().starts_with("rpc.") {
			return Err(Rejection::create(
				method.ident.span(),
				RejectReason::ReservedMethodPrefix,
			));
		}
	}

	let handlers = methods
		.iter()
		.map(|method| add_handler(trait_name, method))
		.collect::<Result<Vec<_>, Rejection>>()?;

	Ok(quote! {
		impl<T: #trait_name + 'static> JSONRPCServer for T where T: Clone + Send + Sync {
			fn into_iohandler(self) -> IoHandler {
				let mut io = IoHandler::new(); // Value to be returned.
				#(#handlers)*
				io
			}
		}
	})
}

// return all methods in the trait, or reject if trait contains an item that is not a method
fn trait_methods<'a>(tr: &'a ItemTrait) -> Result<Vec<&'a MethodSig>, Rejection> {
	tr.items
		.iter()
		.map(|item| match item {
			TraitItem::Method(method) => Ok(&method.sig),
			other => Err(Rejection::create(
				other.span(),
				RejectReason::TraitNotStrictlyMethods,
			)),
		})
		.collect()
}

fn add_handler(
	trait_name: &Ident,
	method: &MethodSig,
) -> Result<proc_macro2::TokenStream, Rejection> {
	let method_name = &method.ident;
	let method_name_literal = format!("\"{}\"", method.ident);
	let args = get_args(&method.decl)?;
	let arg_names_literals = args.iter().map(|(ident, _)| format!("\"{}\"", ident));
	let drain_args = {
		args.iter().enumerate().map(|(index, (ident, typ))| {
			let argn = Ident::new(&format!("arg{}", index), Span::call_site());
			let argname_literal = format!("\"{}\"", ident);
			quote! {
				let next_arg = ordered_args.next().expect(
					"RPC method Got too few args. This is a bug." // checked in get_rpc_args
				);
				let #argn: #typ = serde_json::from_value(next_arg).map_err(|_| {
					InvalidArgs::InvalidArgStructure {
						name: #argname_literal,
						index: #index,
					}
				})?;
			}
		})
	};
	let arg_list = args
		.iter()
		.enumerate()
		.map(|(index, _)| Ident::new(&format!("arg{}", index), Span::call_site()));

	Ok(quote! {
		// each closure gets its own copy of the API object
		let api = self.clone();
		add_rpc_method(
			&mut io,
			#method_name_literal,
			&[ #(#arg_names_literals),* ],
			move |mut args: Vec<Value>| {
				let mut ordered_args = args.drain(..);

				#(#drain_args)*

				// call the target procedure
				let res = <#trait_name>::#method_name(&self, #(#arg_list),*);

				// serialize result into a json value
				let ret = serde_json::to_value(res).expect(
					"serde_json::to_value unexpectedly returned an error, this shouldn't have \
					 happened because serde_json::to_value does not perform io.",
				);

				Ok(ret)
			},
		);
	})
}

// Get the name and type of each argument from method. Skip the first argument, which must be &self.
// If the first argument is not &self, an error will be returned.
fn get_args<'a>(method: &'a FnDecl) -> Result<Vec<(&'a Ident, &'a Type)>, Rejection> {
	let mut inputs = method.inputs.iter();
	match inputs.next() {
		Some(FnArg::SelfRef(ArgSelfRef {
			mutability: None, ..
		})) => Ok(()),
		Some(a) => Err(Rejection::create(
			a.span(),
			RejectReason::FirstArgumentNotSelfRef,
		)),
		None => Err(Rejection::create(
			method.inputs.span(),
			RejectReason::FirstArgumentNotSelfRef,
		)),
	}?;
	let args: Vec<(&Ident, &Type)> = inputs
		.map(as_jsonrpc_arg)
		.collect::<Result<_, Rejection>>()?;
	Ok(args)
}

fn as_jsonrpc_arg<'a>(arg: &'a FnArg) -> Result<(&'a Ident, &'a Type), Rejection> {
	let arg = match arg {
		FnArg::Captured(captured) => Ok(captured),
		a => Err(Rejection::create(
			a.span(),
			RejectReason::ConcreteTypesRequired,
		)),
	}?;
	let ty = &arg.ty;
	let ident = match &arg.pat {
		Pat::Ident(id) => Ok(&id.ident),
		a => Err(Rejection::create(
			a.span(),
			RejectReason::PatternMatchedArgsNotSupported,
		)),
	}?;
	Ok((&ident, &ty))
}

struct Rejection {
	span: Span,
	reason: RejectReason,
}

enum RejectReason {
	FirstArgumentNotSelfRef,
	PatternMatchedArgsNotSupported,
	ConcreteTypesRequired,
	TraitNotStrictlyMethods,
	ReservedMethodPrefix,
}

impl Rejection {
	// Unfortunately syn's neat error reporting capabilities don't work on stable.
	// If 'proc_macro_diagnostic' support does land on stable, we can add nicer error reporting,
	// like:
	//
	// match item {
	//     TraitItem::Method(method) => methods.push(*method),
	// 	   other => {
	//         other
	//             .span()
	//             .unstable()
	//             .error("Macro 'jsonrpc_server' expects a trait containing methods only.")
	//             .emit();
	//         return TokenStream::new();
	//     }
	// }
	//
	// more info: https://github.com/rust-lang/rust/issues/54140
	fn create(span: Span, reason: RejectReason) -> Self {
		Rejection { span, reason }
	}

	// For now, we'll panic with an error message. When #![feature(proc_macro_diagnostic)]
	// becomes available, we'll be able to output pretty errors just like rustc!
	fn raise(self) {
		let description = match self.reason {
			RejectReason::FirstArgumentNotSelfRef => {
				"First argument to jsonrpc method must be &self."
			}
			RejectReason::PatternMatchedArgsNotSupported => {
				"Pattern matched arguments are not supported in jsonrpc methods."
			}
			RejectReason::ConcreteTypesRequired => {
				"Arguments and return values must have concrete types."
			}
			RejectReason::TraitNotStrictlyMethods => {
				"Macro 'jsonrpc_server' expects trait definition containing methods only."
			}
			RejectReason::ReservedMethodPrefix => {
				"The prefix 'rpc.' is reserved https://www.jsonrpc.org/specification#request_object"
			}
		};
		panic!("{:?} {}", self.span, description);
	}
}
