pub type Code = Vec<Op>;
use crate::cpu::Exception;
use crate::cpu::Reg;

#[derive(Default, Clone, Copy)]
pub enum Op {
    #[default]
    Unimplemented,
    Const(Reg, i64),
    Jal(Reg, i64, i64),
    Jalr(Reg, i64, Reg, i16),
    Beq(Reg, Reg, i64),
    Bne(Reg, Reg, i64),
}

pub enum CacheEntry {
    Count(u8),
    Code(Code),
}

pub struct TranslationCache(pub std::collections::HashMap<i64, CacheEntry>);

impl TranslationCache {
    #[must_use]
    pub fn new() -> Self { Self(std::collections::HashMap::new()) }
}

/// # Errors
/// Exceptions are returned as errors
pub fn execute(code: &Code, state: &mut super::cpu::Cpu) -> Result<(), Exception> {
    for op in code {
        match *op {
            Op::Const(rd, k) => state.write_x(rd, k),
            Op::Jal(rd, retaddr, target) => {
                state.write_x(rd, retaddr);
                state.pc = target;
            }
            Op::Jalr(rd, retaddr, rs1, offset) => {
                state.pc = state.read_x(rs1).wrapping_add(i64::from(offset)) & !1;
                state.write_x(rd, retaddr);
            }
            Op::Beq(rs1, rs2, target) => {
                if state.read_x(rs1) == state.read_x(rs2) {
                    state.pc = target;
                }
            }
            Op::Bne(rs1, rs2, target) => {
                if state.read_x(rs1) != state.read_x(rs2) {
                    state.pc = target;
                }
            }
            Op::Unimplemented => todo!(),
        }
    }

    Ok(())
}

impl Default for TranslationCache {
    fn default() -> Self { Self::new() }
}
