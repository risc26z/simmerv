use crate::cpu::Exception;
use crate::cpu::Reg;
use crate::riscv::MemoryAccessType::Read;

// XXX Tuples quickly become unclear.  Switch to regular structs
#[derive(Default, Clone, Copy)]
pub enum Op {
    #[default]
    Unimplemented,
    Const(Reg, i64),
    Jal(Reg, i64, i64),
    Jalr(Reg, i64, i16),
    Beq(i64),
    Bne(i64),
    Blt(i64),
    Bge(i64),
    Bltu(i64),
    Bgeu(i64),
    Lb(Reg, i16, i64), // XXX We'll definitely compress the instruction address in future.
    Lh(Reg, i16, i64), // XXX We'll definitely compress the instruction address in future.
    Lw(Reg, i16, i64), // XXX We'll definitely compress the instruction address in future.
    Lbu(Reg, i16, i64), // XXX We'll definitely compress the instruction address in future.
    Lhu(Reg, i16, i64), // XXX We'll definitely compress the instruction address in future.
}

// Instruction in our virtual machine have two source registers as this is so
// common that it's worth optimizing for.  There is a small benefit to loading
// registers up-front so the fetch can happen in the shadow of the dispatch,
// even though there a small downside for instruction that need fewer or even no
// source registers.  Well, at least that the theory.
pub struct Insn(pub Op, pub Reg, pub Reg);
pub type Code = Vec<Insn>;

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
#[allow(clippy::cast_sign_loss)]
#[allow(clippy::cast_lossless)]
#[allow(clippy::cast_possible_truncation)]
pub fn execute(code: &Code, cpu: &mut super::cpu::Cpu) -> Result<(), Exception> {
    for Insn(op, rs1, rs2) in code {
        let s1 = cpu.read_register(*rs1);
        let s2 = cpu.read_register(*rs2);
        match *op {
            Op::Const(rd, k) => cpu.write_x(rd, k),
            Op::Jal(rd, retaddr, target) => {
                cpu.write_x(rd, retaddr);
                cpu.pc = target;
            }
            Op::Jalr(rd, retaddr, offset) => {
                cpu.pc = s1.wrapping_add(i64::from(offset)) & !1;
                cpu.write_x(rd, retaddr);
            }
            Op::Beq(target) => {
                if s1 == s2 {
                    cpu.pc = target;
                }
            }
            Op::Bne(target) => {
                if s1 != s2 {
                    cpu.pc = target;
                }
            }
            Op::Blt(target) => {
                if s1 < s2 {
                    cpu.pc = target;
                }
            }
            Op::Bge(target) => {
                if s1 >= s2 {
                    cpu.pc = target;
                }
            }
            Op::Bltu(target) => {
                if (s1 as u64) < (s2 as u64) {
                    cpu.pc = target;
                }
            }
            Op::Bgeu(target) => {
                if (s1 as u64) >= (s2 as u64) {
                    cpu.pc = target;
                }
            }
            Op::Lb(rd, offset, insn_addr) => {
                // XXX This raises an issue!  If this throws an exception then
                // how do we find the address of the faulting instruction?  Oops.
                // Immediate options that come to mind:
                //
                // 1.* all state updates are rolled back/not committed and we retry in
                // single-step mode
                // 2. we keep enough information "somewhere" that we can recreate addresses
                // 3. we keep count and find the address by tracing the instructions from the BB
                //    entry (this in particular only works if we have a 1-1 mapping).
                // 4.* variation on 2., for potentially-faulting insn, keep the address (as on
                // offset) (* are the most promising ones)
                //
                // If we do any optimization on the generated code, then option 1. seems the
                // only option, but that is not cheap, so a compromise for now,
                // might be that state needs to be coherent at the
                // time of a potentially-faulting insn (it acts as a serializing barrier).
                cpu.insn_addr = insn_addr;
                let v = cpu.memop(Read, s1, offset as i64, 0, 1)? as i8 as i64;
                cpu.write_x(rd, v);
            }

            Op::Lh(rd, offset, insn_addr) => {
                cpu.insn_addr = insn_addr;
                let v = cpu.memop(Read, s1, offset as i64, 0, 2)? as i16 as i64;
                cpu.write_x(rd, v);
            }

            Op::Lw(rd, offset, insn_addr) => {
                cpu.insn_addr = insn_addr;
                let v = cpu.memop(Read, s1, offset as i64, 0, 4)? as i32 as i64;
                cpu.write_x(rd, v);
            }

            Op::Lbu(rd, offset, insn_addr) => {
                cpu.insn_addr = insn_addr;
                let v = cpu.memop(Read, s1, offset as i64, 0, 1)?;
                cpu.write_x(rd, v);
            }

            Op::Lhu(rd, offset, insn_addr) => {
                cpu.insn_addr = insn_addr;
                let v = cpu.memop(Read, s1, offset as i64, 0, 2)?;
                cpu.write_x(rd, v);
            }

            Op::Unimplemented => todo!(),
        }
    }

    Ok(())
}

impl Default for TranslationCache {
    fn default() -> Self { Self::new() }
}
