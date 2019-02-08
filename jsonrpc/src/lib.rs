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
	use jsonrpc_core::{IoHandler, Value};
	use jsonrpc_proc_macro::jsonrpc_server;

	#[jsonrpc_server]
	pub trait Adder {
		fn checked_add(&self, a: isize, b: isize) -> Option<isize>;
		fn wrapping_add(&self, a: isize, b: isize) -> isize;
	}

	#[test]
	fn into_iohandler() {
		#[derive(Clone)]
		struct AdderImpl;
		impl Adder for AdderImpl {
			fn checked_add(&self, a: isize, b: isize) -> Option<isize> {
				a.checked_add(b)
			}

			fn wrapping_add(&self, a: isize, b: isize) -> isize {
				a.wrapping_add(b)
			}
		}
	}

	#[test]
	fn test_are_written() {
		panic!("No, they are not.")
	}
}
