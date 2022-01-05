use cranelift_codegen::ir::condcodes::IntCC;
use walrus::ir::BinaryOp;

pub(crate) fn wasm_of_cond(cond: IntCC, bits_32: bool) -> BinaryOp {
    match cond {
        IntCC::Equal if bits_32 => BinaryOp::I32Eq,
        IntCC::Equal => BinaryOp::I64Eq,
        IntCC::NotEqual => todo!(),
        IntCC::SignedLessThan => todo!(),
        IntCC::SignedGreaterThanOrEqual => todo!(),
        IntCC::SignedGreaterThan => todo!(),
        IntCC::SignedLessThanOrEqual => todo!(),
        IntCC::UnsignedLessThan => todo!(),
        IntCC::UnsignedGreaterThanOrEqual => todo!(),
        IntCC::UnsignedGreaterThan => todo!(),
        IntCC::UnsignedLessThanOrEqual if bits_32 => BinaryOp::I32LeU,
        IntCC::UnsignedLessThanOrEqual => BinaryOp::I64LeU,
        IntCC::Overflow => todo!(),
        IntCC::NotOverflow => todo!(),
    }
}
