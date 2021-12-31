//! Converts WebAssembly to Cranelift signatures.

use cranelift_codegen::ir::{self, AbiParam};
use walrus::ValType;

use crate::conversions::ty::wasm_of_cranelift;

/// Transforms a Cranelift [cranelift_codegen::ir::Signature] into the corresponding
/// [walrus::ValType]'s, returning them in the form `(Vec<parameters>, Vec<return_values>)`.
pub(crate) fn wasm_of_sig(sig: ir::Signature) -> (Vec<ValType>, Vec<ValType>) {
    // todo: handle some of the other information
    fn map_abi_param(param: AbiParam) -> ValType {
        wasm_of_cranelift(param.value_type)
    }

    let params = sig.params.into_iter().map(map_abi_param).collect();
    let returns = sig.returns.into_iter().map(map_abi_param).collect();

    (params, returns)
}
