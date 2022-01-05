use cranelift_codegen::ir::condcodes::IntCC;
use walrus::ir::BinaryOp;

pub(crate) fn wasm_of_cond(cond: IntCC, bits_32: bool) -> BinaryOp {
    match cond {
        IntCC::Equal if bits_32 => BinaryOp::I32Eq,
        IntCC::Equal => BinaryOp::I64Eq,
        IntCC::UnsignedLessThanOrEqual if bits_32 => BinaryOp::I32LeU,
        IntCC::UnsignedLessThanOrEqual => BinaryOp::I64LeU,
        IntCC::NotEqual
        | IntCC::SignedLessThan
        | IntCC::SignedGreaterThanOrEqual
        | IntCC::SignedGreaterThan
        | IntCC::SignedLessThanOrEqual
        | IntCC::UnsignedLessThan
        | IntCC::UnsignedGreaterThanOrEqual
        | IntCC::UnsignedGreaterThan
        | IntCC::Overflow
        | IntCC::NotOverflow => todo!(),
    }
}
