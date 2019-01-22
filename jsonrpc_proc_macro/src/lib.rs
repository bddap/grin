#![recursion_limit = "256"]

// extern crate proc_macro;
// use proc_macro::TokenStream;
// use quote::quote;
extern crate proc_macro;
// use proc_macro;
use proc_macro2;
use proc_macro2::Span;
use quote::quote;
use syn::spanned::Spanned;
use syn::{
	parse_macro_input, ArgCaptured, ArgSelfRef, FnArg, FnDecl, Ident, ItemTrait, MethodSig, Pat,
	PatIdent, TraitItem, TraitItemMethod, Type,
};
// use syn::spanned::Spanned;
// use syn::{
// 	parse_macro_input, parse_quote, Data, DeriveInput, Fields, GenericParam, Generics, Index,
// };

#[proc_macro_attribute]
pub fn jsonrpc_server(
	_attr: proc_macro::TokenStream,
	item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
	let tr = parse_macro_input!(item as ItemTrait);
	let server_impl = match impl_server(&tr) {
		Ok(s) => s,
		Err(reject) => {
			reject.raise();
			return proc_macro::TokenStream::new();
		}
	};
	proc_macro::TokenStream::from(quote! {
		#tr
		#server_impl
	})
}

fn impl_server(tr: &ItemTrait) -> Result<proc_macro2::TokenStream, Rejection> {
	let trait_name = &tr.ident;
	let methods: Vec<&MethodSig> = trait_methods(&tr)?;

	let handlers: proc_macro2::TokenStream = methods
		.iter()
		.map(|method| add_handler(trait_name, method))
		.collect::<Result<Vec<_>, Rejection>>()?
		.iter()
		.map(|a| a.clone())
		.collect();

	Ok(quote! {
		impl<T: #trait_name + 'static> JSONRPCServer for T {
			fn into_iohandler(self) -> IoHandler {
				let mut io = IoHandler::new(); // Value to be returned.
				#handlers
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
	// proc_name: String, arg_names: &[String]

	let args = get_args(&method.decl)?;
	let drain_args = read_args(&args)?;
	let rpc_call = call_rpc(trait_name, &method.ident, &args)?;

	Ok(quote! {
		// each closure gets its own copy of the API object
		let api = self.clone();
		add_rpc_method(
			&mut io,
			"checked_add",
			&["a", "b"],
			move |mut args: Vec<Value>| {
				let expected = 1;
				let actual = args.len();
				debug_assert_eq!(expected, actual); // ensured by get_rpc_args
				let unreachable_argnum_err = || InvalidArgs::WrongNumberOfArgs { expected, actual };
				let mut ordered_args = args.drain(..);

				#drain_args

				// call the target procedure
				let res = #rpc_call;

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

fn get_args<'a>(method: &'a FnDecl) -> Result<Vec<(&'a Ident, &'a Type)>, Rejection> {
	let mut inputs = method.inputs.iter();
	match inputs.next() {
		Some(FnArg::SelfRef(ArgSelfRef {
			mutability: None, ..
		})) => Ok(()),
		Some(FnArg::SelfValue(_)) => Ok(()),
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

fn read_args(args: &[(&Ident, &Type)]) -> Result<proc_macro2::TokenStream, Rejection> {
	let ret = args
		.iter()
		.enumerate()
		.map(|(index, arg)| {
			let argn = format!("arg{}", index);
			// let argname = arg.
			quote! {
				let next_arg = ordered_args.next().ok_or_else(unreachable_argnum_err)?;
				let #argn = serde_json::from_value(next_arg).map_err(|_| {
					InvalidArgs::InvalidArgStructure {
						name: "a",
						index: #index,
					}
				})?;
			}
		})
		.collect();
	Ok(ret)
}

fn call_rpc(
	trait_name: &Ident,
	method_name: &Ident,
	method_args: &[(&Ident, &Type)],
) -> Result<proc_macro2::TokenStream, Rejection> {
	let arg_list = method_args
		.iter()
		.skip(1)
		.enumerate()
		.map(|(index, _)| format!("arg{}, ", index))
		.collect::<proc_macro2::TokenStream>();
	Ok(quote! {
		#trait_name :: #method_name ()
	})
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

	fn create(span: Span, reason: RejectReason) -> Self {
		Rejection { span, reason }
	}

	// feature #![feature(proc_macro_diagnostic)] https://github.com/rust-lang/rust/issues/54140
	// is not stable yet. For now we'll just panic with an error message. When proc_macro_diagnostic
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
		};
		panic!("{:?} {}", self.span, description);
	}
}
