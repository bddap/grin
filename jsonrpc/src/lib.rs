// Declare JSONRPCServer and JSONRPCClient interfaces.

use jsonrpc_core::{Error, IoHandler, Params, Value};

pub trait JSONRPCServer {
	fn into_iohandler(self) -> IoHandler;
}

// The JSONRPCClient generator design is still WIP, but ideally clients will satisfy this
// property:
//   if T implements                  fn f(&self, args..) -> R
//   then JSONRPCClient<T> implements fn f(&self, args..) -> Future<Result<R, E>>

pub fn add_rpc_method<F>(
	iohandler: &mut IoHandler,
	name: &'static str,
	arg_names: &'static [&'static str],
	cb: F,
) where
	F: Fn(Vec<Value>) -> Result<Value, InvalidArgs>
		+ std::marker::Sync
		+ std::marker::Send
		+ 'static,
{
	iohandler.add_method(name, move |params: Params| {
		get_rpc_args(arg_names, params)
			.and_then(|args| cb(args))
			.map_err(std::convert::Into::into)
	})
}

// Verify and convert jsonrpc Params into owned argument list.
// Verifies:
//    - Number of args in positional parameter list is correct
//    - No missing args in named parameter object
//    - No extra args in named parameter object
// Absent parameter objects are interpreted as empty positional parameter lists
pub fn get_rpc_args(names: &[&'static str], params: Params) -> Result<Vec<Value>, InvalidArgs> {
	let ar: Vec<Value> = match params {
		Params::Array(ar) => ar,
		Params::Map(mut ma) => {
			let mut ar: Vec<Value> = Vec::with_capacity(names.len());
			for name in names.iter() {
				ar.push(
					ma.remove(*name)
						.ok_or(InvalidArgs::MissingNamedParameter { name })?,
				);
			}
			debug_assert_eq!(ar.len(), names.len());
			match ma.keys().next() {
				Some(key) => return Err(InvalidArgs::ExtraNamedParameter { name: key.clone() }),
				None => ar,
			}
		}
		Params::None => vec![],
	};
	if ar.len() != names.len() {
		Err(InvalidArgs::WrongNumberOfArgs {
			expected: ar.len(),
			actual: names.len(),
		})
	} else {
		Ok(ar)
	}
}

pub enum InvalidArgs {
	WrongNumberOfArgs { expected: usize, actual: usize },
	ExtraNamedParameter { name: String },
	MissingNamedParameter { name: &'static str },
	InvalidArgStructure { name: &'static str, index: usize },
}

impl Into<Error> for InvalidArgs {
	fn into(self) -> Error {
		match self {
			InvalidArgs::WrongNumberOfArgs { expected, actual } => Error::invalid_params(format!(
				"WrongNumberOfArgs. Expected {}. Actual {}",
				expected, actual
			)),
			InvalidArgs::ExtraNamedParameter { name } => {
				Error::invalid_params(format!("ExtraNamedParameter {}", name))
			}
			InvalidArgs::MissingNamedParameter { name } => {
				Error::invalid_params(format!("MissingNamedParameter {}", name))
			}
			InvalidArgs::InvalidArgStructure { name, index } => Error::invalid_params(format!(
				"InvalidArgStructure {} at position {}.",
				name, index
			)),
		}
	}
}

#[cfg(test)]
mod test {
	use crate::{add_rpc_method, InvalidArgs, JSONRPCServer};
	use assert_matches::assert_matches;
	use jsonrpc_core::types::response::{Failure, Output, Response};
	use jsonrpc_core::types::{Error, ErrorCode};
	use jsonrpc_core::{IoHandler, Value};
	use jsonrpc_proc_macro::jsonrpc_server;
	use serde_json;

	#[jsonrpc_server]
	pub trait Adder {
		fn checked_add(&self, a: isize, b: isize) -> Option<isize>;
		fn wrapping_add(&self, a: isize, b: isize) -> isize;
		fn greet(&self) -> String;
		fn swallow(&self);
		fn repeat_list(&self, lst: Vec<usize>) -> Vec<usize>;
		fn fail(&self) -> Result<isize, String>;
		fn succeed(&self) -> Result<isize, String>;
	}

	#[derive(Clone)]
	struct AdderImpl;
	impl Adder for AdderImpl {
		fn checked_add(&self, a: isize, b: isize) -> Option<isize> {
			a.checked_add(b)
		}

		fn wrapping_add(&self, a: isize, b: isize) -> isize {
			a.wrapping_add(b)
		}

		fn greet(&self) -> String {
			"hello".into()
		}

		fn swallow(&self) {}

		fn repeat_list(&self, lst: Vec<usize>) -> Vec<usize> {
			let mut ret = lst.clone();
			ret.extend(lst);
			ret
		}

		fn fail(&self) -> Result<isize, String> {
			Err("tada!".into())
		}

		fn succeed(&self) -> Result<isize, String> {
			Ok(1)
		}
	}

	fn adder_call(request: &str) -> String {
		let api = AdderImpl {};
		let io = api.into_iohandler();
		io.handle_request_sync(request).unwrap()
	}

	fn adder_call_ty(request: &str) -> Output {
		match serde_json::from_str(&adder_call(request)).unwrap() {
			Response::Single(out) => out,
			Response::Batch(_) => panic!(),
		}
	}

	fn assert_adder_response(request: &str, response: &str) {
		assert_eq!(adder_call(request), response.to_owned());
	}

	#[test]
	fn positional_args() {
		assert_adder_response(
			r#"{"jsonrpc": "2.0", "method": "wrapping_add", "params": [1, 1], "id": 1}"#,
			r#"{"jsonrpc":"2.0","result":2,"id":1}"#,
		);
	}

	#[test]
	fn named_args() {
		assert_adder_response(
			r#"{"jsonrpc": "2.0", "method": "wrapping_add", "params": {"a": 1, "b":1}, "id": 1}"#,
			r#"{"jsonrpc":"2.0","result":2,"id":1}"#,
		);
	}

	#[test]
	fn null_args() {
		let response = r#"{"jsonrpc":"2.0","result":"hello","id":1}"#;
		assert_adder_response(
			r#"{"jsonrpc": "2.0", "method": "greet", "params": {}, "id": 1}"#,
			response,
		);
		assert_adder_response(
			r#"{"jsonrpc": "2.0", "method": "greet", "params": [], "id": 1}"#,
			response,
		);
		assert_adder_response(
			r#"{"jsonrpc": "2.0", "method": "greet", "params": null, "id": 1}"#,
			response,
		);
		assert_adder_response(
			r#"{"jsonrpc": "2.0", "method": "greet", "id": 1}"#,
			response,
		);
	}

	#[test]
	fn null_return() {
		assert_adder_response(
			r#"{"jsonrpc": "2.0", "method": "swallow", "params": [], "id": 1}"#,
			r#"{"jsonrpc":"2.0","result":null,"id":1}"#,
		);
	}

	#[test]
	fn incorrect_method_name() {
		assert_matches!(
			adder_call_ty(r#"{"jsonrpc": "2.0", "method": "nonexist", "params": [], "id": 1}"#),
			Output::Failure(Failure {
				error: Error {
					code: ErrorCode::MethodNotFound,
					..
				},
				..
			})
		);
	}

	#[test]
	fn incorrect_args() {
		assert_matches!(
			adder_call_ty(r#"{"jsonrpc": "2.0", "method": "wrapping_add", "params": [], "id": 1}"#),
			Output::Failure(Failure {
				error: Error {
					code: ErrorCode::InvalidParams,
					..
				},
				..
			})
		);
		assert_matches!(
			adder_call_ty(
				r#"{"jsonrpc": "2.0", "method": "wrapping_add", "params": {
                    "notanarg": 1, "notarg": 1}, "id": 1}"#
			),
			Output::Failure(Failure {
				error: Error {
					code: ErrorCode::InvalidParams,
					..
				},
				..
			})
		);
		assert_matches!(
			adder_call_ty(
				r#"{"jsonrpc": "2.0", "method": "wrapping_add", "params": [[], []], "id": 1}"#
			),
			Output::Failure(Failure {
				error: Error {
					code: ErrorCode::InvalidParams,
					..
				},
				..
			})
		);
	}

	#[test]
	fn complex_type() {
		assert_adder_response(
			r#"{"jsonrpc": "2.0", "method": "repeat_list", "params": [[1, 2, 3]], "id": 1}"#,
			r#"{"jsonrpc":"2.0","result":[1,2,3,1,2,3],"id":1}"#,
		);
		assert_matches!(
			adder_call_ty(
				r#"{"jsonrpc": "2.0", "method": "repeat_list", "params": [[1], [12]], "id": 1}"#,
			),
			Output::Failure(Failure {
				error: Error {
					code: ErrorCode::InvalidParams,
					..
				},
				..
			})
		);
		assert_adder_response(
			r#"{"jsonrpc": "2.0", "method": "fail", "params": [], "id": 1}"#,
			r#"{"jsonrpc":"2.0","result":{"Err":"tada!"},"id":1}"#,
		);
		assert_adder_response(
			r#"{"jsonrpc": "2.0", "method": "succeed", "params": [], "id": 1}"#,
			r#"{"jsonrpc":"2.0","result":{"Ok":1},"id":1}"#,
		);
	}

}
