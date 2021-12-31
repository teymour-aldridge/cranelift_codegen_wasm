//! Translates atomic types (i.e. not functions) from Cranelift to WebAssembly.

use cranelift_codegen::ir::types::Type as CraneliftType;
use walrus::ValType;

/// Convert a Cranelift type into its corresponding WebAssembly form.
///
/// note: this function panics if it does not yet know how to represent the type
/// todo: report a proper error
pub(crate) fn wasm_of_cranelift(ty: CraneliftType) -> ValType {
    // integers
    if ty.is_int() && ty.bits() == 32 {
        return ValType::I32;
    } else if ty.is_int() && ty.bits() == 64 {
        return ValType::I64;
    }

    // floats
    if ty.is_float() && ty.bits() == 32 {
        return ValType::F32;
    } else if ty.is_float() && ty.bits() == 64 {
        return ValType::F64;
    }

    todo!()
}
