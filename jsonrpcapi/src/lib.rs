extern crate grin_core;
extern crate grin_keychain as keychain;
extern crate grin_util as util;
extern crate grin_wallet as wallet;
extern crate jsonrpc_core;
extern crate jsonrpc_minihttp_server;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

#[cfg(test)]
mod test {
	use super::*;
	use jsonrpc_core::{self, IoHandler, Params, Value};
	use jsonrpc_minihttp_server::ServerBuilder;

	enum InvalidArgs {
		WrongNumberOfArgs,
		ExtraNamedParameter,
		MissingNamedParameter { name: &'static str },
		InvalidArgStructure { name: &'static str, index: usize },
	}

	impl Into<jsonrpc_core::Error> for InvalidArgs {
		fn into(self) -> jsonrpc_core::Error {
			use jsonrpc_core::Error;
			match self {
				InvalidArgs::WrongNumberOfArgs => Error::invalid_params("WrongNumberOfArgs"),
				InvalidArgs::ExtraNamedParameter => Error::invalid_params("ExtraNamedParameter"),
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

	#[derive(Serialize, Deserialize)]
	struct InternalError;

	#[test]
	fn api_foriegn() {
		let (api_owner, api_foriegn) = {
			use keychain::ExtKeychain;
			use std::sync::Arc;
			use wallet::libwallet::api::{APIForeign, APIOwner};

			// These contain sample implementations of each part needed for a wallet
			use wallet::{HTTPNodeClient, LMDBBackend, WalletBackend, WalletConfig};

			let wallet_config = WalletConfig::default();

			// A NodeClient must first be created to handle communication between
			// the wallet and the node.

			let node_client = HTTPNodeClient::new(&wallet_config.check_node_api_http_addr, None);
			let wallet: Arc<util::Mutex<WalletBackend<HTTPNodeClient, ExtKeychain>>> = Arc::new(
				util::Mutex::new(LMDBBackend::new(wallet_config.clone(), "", node_client).unwrap()),
			);

			(
				APIOwner::new(wallet.clone()),
				APIForeign::new(wallet.clone()),
			)
		};

		// We need to condider how this api should report errors. There are several
		// options.
		//
		// 1. All procedures return a Result which is serialzed using serde. This could
		//    be a security concern, as it may leak sensitive data to api clients.
		// 2. All procedures return a Result which is not serialized; instead it is
		//    reported as an opaque "Internal Error".
		// 3. The jsonrpc 2.0 spec provides a mechanism for reporting internal errors
		//    https://www.jsonrpc.org/specification#error_object
		//    Use of this mechanism has pros and cons:
		//    pros:
		//        - Conforms to jsonrpc user expectations
		//        - More easily consumable by non-rust clients
		//    cons:
		//        - Each error type must have an associated number id, assigning ids
		//          will likely be a manual process.
		//    Jsonrpc errors MAY include a structured "data" field. Internal errors
		//    would be serialized into the field using serde. As with option 1, detailed
		//    error messages could leak sensitive information.
		// 4. Use jsonrpc error reporting, but report iternal errors as an opaque
		//    "Internal Error".
		//
		// The options, in order of ease of implementation are: 2, 4, 1, 3.
		//
		// Options 1 and 3, the options involving structured error reporting, are slightly more
		// difficult to implement because grin errors contain failure::Context objects.
		// AFIK failure::Context does not implement serde Serialize and Deserialze traits.
		//
		// Baring further disscussion. Option 2 will be used, as it is simplest and safest.

		let foriegn_handler = {
			let mut io = IoHandler::new();

			// each endpoint gets it's own copy of wallet
			let api_copy = api_foriegn.clone();
			add_rpc_method(
				&mut io,
				"build_coinbase",
				&["block_fees"],
				move |mut args: Vec<Value>| {
					let mut ordered_args = args.drain(..);

					// parse each arguments in order
					let next_arg = ordered_args.next().ok_or(InvalidArgs::WrongNumberOfArgs)?;
					let arg0: wallet::libwallet::types::BlockFees =
						serde_json::from_value(next_arg).map_err(|_| {
							InvalidArgs::InvalidArgStructure {
								name: "block_fees",
								index: 0,
							}
						})?;

					// Api object will be mutated, we make a copy so rustc will let us call mutable
					// methods.
					let mut api = api_copy.clone();

					// call the target procedure
					let res = api.build_coinbase(&arg0).map_err(|_| InternalError);

					// serialize result into a json value
					let ret = serde_json::to_value(res).expect(
						"serde_json::to_value unexpectedly returned an error, this shouldn't have \
						 happened because serde_json::to_value does not perform io.",
					);

					Ok(ret)
				},
			);

			let api_copy = api_foriegn.clone();
			add_rpc_method(
				&mut io,
				"verify_slate_messages",
				&["slate"],
				move |mut args: Vec<Value>| {
					let mut ordered_args = args.drain(..);

					// parse each arguments in order
					let next_arg = ordered_args.next().ok_or(InvalidArgs::WrongNumberOfArgs)?;
					let arg0: grin_core::libtx::slate::Slate = serde_json::from_value(next_arg)
						.map_err(|_| InvalidArgs::InvalidArgStructure {
							name: "slate",
							index: 0,
						})?;

					// Api object will be mutated, we make a copy so rustc will let us call mutable
					// methods.
					let mut api = api_copy.clone();

					// call the target procedure
					let res = api.verify_slate_messages(&arg0).map_err(|_| InternalError);

					// serialize result into a json value
					let ret = serde_json::to_value(res).expect(
						"serde_json::to_value unexpectedly returned an error, this shouldn't have \
						 happened because serde_json::to_value does not perform io.",
					);

					Ok(ret)
				},
			);

			let api_copy = api_foriegn.clone();
			add_rpc_method(
				&mut io,
				"receive_tx",
				&["slate", "dest_acct_name", "message"],
				move |mut args: Vec<Value>| {
					let mut ordered_args = args.drain(..);

					// parse each arguments in order
					let next_arg = ordered_args.next().ok_or(InvalidArgs::WrongNumberOfArgs)?;
					let arg0: grin_core::libtx::slate::Slate = serde_json::from_value(next_arg)
						.map_err(|_| InvalidArgs::InvalidArgStructure {
							name: "slate",
							index: 0,
						})?;
					let next_arg = ordered_args.next().ok_or(InvalidArgs::WrongNumberOfArgs)?;
					let arg1: Option<String> = serde_json::from_value(next_arg).map_err(|_| {
						InvalidArgs::InvalidArgStructure {
							name: "dest_acc_name",
							index: 1,
						}
					})?;
					let next_arg = ordered_args.next().ok_or(InvalidArgs::WrongNumberOfArgs)?;
					let arg2: Option<String> = serde_json::from_value(next_arg).map_err(|_| {
						InvalidArgs::InvalidArgStructure {
							name: "message",
							index: 2,
						}
					})?;

					// Api object will be mutated, we make a copy so rustc will let us call mutable
					// methods.
					let mut api = api_copy.clone();

					// These conversions are necessary because receive_tx takes a mix of borrowed
					// and owned parameters. Later on, in order to automatially generate json rpc
					// apis arguments ownership will likely need to be homogeonus for all
					// procedures.
					let mut arg0_converted = arg0;
					let arg1_converted = arg1.as_ref().map(|x| &**x);

					// call the target procedure
					let res = api
						.receive_tx(&mut arg0_converted, arg1_converted, arg2)
						.map_err(|_| InternalError);

					// serialize result into a json value
					let ret = serde_json::to_value(res).expect(
						"serde_json::to_value unexpectedly returned an error, this shouldn't have \
						 happened because serde_json::to_value does not perform io.",
					);

					Ok(ret)
				},
			);
			io
		};
	}

	fn add_rpc_method<F>(
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
			let args = get_rpc_args(arg_names, params).map_err(std::convert::Into::into)?;
			cb(args).map_err(std::convert::Into::into)
		})
	}

	// Verify and convert jsonrpc Params into owned argument list.
	// Verifies:
	//    - Number of args in positional parameter list is correct
	//    - No missing args in named parameter object
	//    - No extra args in named parameter object
	// Absent parameter objects are interpreted as empty positional parameter lists
	fn get_rpc_args(names: &[&'static str], params: Params) -> Result<Vec<Value>, InvalidArgs> {
		let ar: Vec<Value> = match params {
			Params::Array(ar) => ar,
			Params::Map(ma) => {
				if ma.len() > names.len() {
					return Err(InvalidArgs::ExtraNamedParameter);
				}
				let mut ar: Vec<Value> = Vec::with_capacity(names.len());
				for name in names.iter() {
					ar.push(
						ma.get(*name)
							.map(|a| a.clone())
							.ok_or(InvalidArgs::MissingNamedParameter { name })?,
					);
				}
				ar
			}
			Params::None => vec![],
		};
		if ar.len() != names.len() {
			Err(InvalidArgs::WrongNumberOfArgs)
		} else {
			Ok(ar)
		}
	}

	#[ignore]
	#[test]
	fn histeresis() {
		let mut io = IoHandler::new();
		io.add_method("say_hello", |_params: Params| {
			Ok(Value::String("hello".to_string()))
		});

		let server = ServerBuilder::new(io)
			.threads(3)
			.start_http(&"127.0.0.1:3030".parse().unwrap())
			.unwrap();

		server.wait().unwrap();
	}
}
