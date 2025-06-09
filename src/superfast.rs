pub type Code = Vec<Operation>;
use crate::cpu::Exception;
use crate::cpu::Reg;

#[derive(Default, Clone, Copy)]
pub enum Operation {
    #[default]
    Unimplemented,
    Const(Reg, i64),
    Jal(Reg, i64, i64),
    Jalr(Reg, i64, Reg, i16),
}

pub enum CacheEntry {
    Count(u8),
    Code(Code),
}

pub struct TranslationCache(pub std::collections::HashMap<i64, CacheEntry>);

impl TranslationCache {
    #[must_use]
    pub fn new() -> Self {
        Self(std::collections::HashMap::new())
    }
}

pub fn execute(code: &Code, state: &mut super::cpu::Cpu) -> Result<(), Exception> {
    use Operation::*;
    for op in code {
        match op {
            Const(rd, k) => state.write_x(*rd, *k),
            Jal(rd, retaddr, target) => {
                state.write_x(*rd, *retaddr);
                state.pc = *target;
            }
            Jalr(rd, retaddr, rs1, delta) => {
                state.write_x(*rd, *retaddr);
                state.pc = state.read_x(*rs1).wrapping_add(*delta as i64);
            }
            _ => todo!(),
        }
    }

    Ok(())
}

impl Default for TranslationCache {
    fn default() -> Self {
        Self::new()
    }
}
