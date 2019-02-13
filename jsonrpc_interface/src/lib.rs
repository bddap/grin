// Declare JSONRPCServer and JSONRPCClient interfaces.

use jsonrpc_core::types::*;
pub use jsonrpc_core::{Error, Params, Request, Value};
use serde::ser::Serialize;

pub trait JSONRPCServer {
	fn handle(&self, method: &str, params: Params) -> Result<Value, Error>;

	fn handle_call(&self, call: Call) -> Option<Output> {
		match call {
			Call::Notification(Notification { method, params, .. }) => {
				let _ = self.handle(&method, params);
				None
			}
			Call::MethodCall(MethodCall {
				method,
				params,
				id,
				jsonrpc,
			}) => {
				let output = match self.handle(&method, params) {
					Ok(ok) => Output::Success(Success {
						jsonrpc,
						result: ok,
						id,
					}),
					Err(err) => Output::Failure(Failure {
						jsonrpc,
						error: err,
						id,
					}),
				};
				Some(output)
			}
			Call::Invalid { id } => Some(Output::Failure(Failure {
				jsonrpc: Some(Version::V2),
				error: Error::invalid_request(),
				id,
			})),
		}
	}

	fn handle_parsed(&self, request: Request) -> Option<Response> {
		match request {
			Request::Single(call) => self.handle_call(call).map(Response::Single),
			Request::Batch(mut calls) => {
				let outputs = calls
					.drain(..)
					.filter_map(|call| self.handle_call(call))
					.collect::<Vec<_>>();
				if outputs.is_empty() {
					None
				} else {
					Some(Response::Batch(outputs))
				}
			}
		}
	}

	/// Accept request as a jsonrpc string. Call handler. Return result as a jsonrpc string.
	fn handle_raw(&self, request: &str) -> Option<String> {
		let request: Request = serde_json::from_str(request)
			.unwrap_or(Request::Single(Call::Invalid { id: Id::Null }));
		self.handle_parsed(request).map(|response| {
			serde_json::to_string(&response).expect("to_string does not perform io")
		})
	}
}

// The JSONRPCClient generator design is still WIP, but ideally clients will satisfy this
// property:
//   if T implements                  fn f(&self, args..) -> R
//   then JSONRPCClient<T> implements fn f(&self, args..) -> Future<Result<R, E>>

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

/// Result types in the return position are handled differently from others.
/// Errors are reported according to the
/// [jsonrpc spec](https://www.jsonrpc.org/specification#error_object).
/// Calling to_response on any type T that is not an Result will yield an Ok(T).
/// Calling to_response on a Result<T, E> will result in either Ok(T) or Err(jsonrpc_core::Error).
/// E must be convertable to an jsonrpc_core::Error.
/// jsonrpc spec requires a unique integer id for every error type. That sounds like a hassle in
/// this use-case. For now we'll semantically merge all error types into a single uni-type,
/// granting a single id to be shared by all ;)
///
/// Instead using an integer id to identify errors, we'll use serde to report structured errors.
trait ToRPCResult {
	fn to_result(&self) -> Result<Value, Error>;
}

trait IsResult {
	fn to_rpc_result(&self) -> Result<Value, Error>;
}

impl<A: Serialize, B: Serialize> ToRPCResult for Result<A, B> {
	fn to_result(&self) -> Result<Value, Error> {
		match self {
			Ok(k) => Ok(to_value(k)),
			Err(e) => Err(Error {
				code: ErrorCode::ServerError(8),
				message: "Server error.".into(),
				data: Some(to_value(&e)),
			}),
		}
	}
}

trait NotResult: !IsResult {}

impl<T: NotResult + Serialize> ToRPCResult for T {
	fn to_result(&self) -> Result<Value, Error> {
		Ok(to_value(self))
	}
}

fn to_value<T: Serialize>(t: &T) -> Value {
	serde_json::to_value(t).expect(
		"serde_json::to_value unexpectedly returned an error, this shouldn't have \
		 happened because serde_json::to_value does not perform io.",
	)
}

#[cfg(test)]
mod test {
	use crate as rpc_interface;
	use crate::{InvalidArgs, JSONRPCServer};
	use assert_matches::assert_matches;
	use jsonrpc_core::types::*;
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

	fn assert_adder_response(request: &str, response: &str) {
		assert_eq!(&AdderImpl {}.handle_raw(request).unwrap(), response);
	}

	fn handle_single(request: &str) -> Output {
		let a: Option<Response> =
			AdderImpl {}.handle_parsed(serde_json::from_str(&request).unwrap());
		match a {
			Some(Response::Single(a)) => a,
			_ => panic!(),
		}
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
			handle_single(r#"{"jsonrpc": "2.0", "method": "nonexist", "params": [], "id": 1}"#),
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
			handle_single(r#"{"jsonrpc": "2.0", "method": "wrapping_add", "params": [], "id": 1}"#),
			Output::Failure(Failure {
				error: Error {
					code: ErrorCode::InvalidParams,
					..
				},
				..
			})
		);
		assert_matches!(
			handle_single(
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
			handle_single(
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
			handle_single(
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

	#[test]
	fn notification() {
		let request =
			serde_json::from_str(r#"{"jsonrpc": "2.0", "method": "succeed", "params": []}"#)
				.unwrap();
		assert_eq!(AdderImpl {}.handle_parsed(request), None);
	}
}
