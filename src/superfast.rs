use crate::cpu::Exception;
use crate::cpu::Reg;
use crate::riscv::MemoryAccessType::Read;
use crate::riscv::MemoryAccessType::Write;

// For instructions that need their source address.
// Currently we use the full address but in future it will be an offset from
// a per-translation base address
type CodeAddr = i64;
// XXX Tuples quickly become unclear.  Switch to regular structs
#[derive(Default, Clone, Copy, Debug)]
pub enum Op {
    #[default]
    Unimplemented,
    Const(Reg, i64),         // XXX May keep the lui/auipc/...
    Jal(Reg, i64, CodeAddr), // XXX Could be ~ Reg, i16, i16
    Jalr(Reg, i16, CodeAddr),
    Beq(CodeAddr),
    Bne(CodeAddr),
    Blt(CodeAddr),
    Bge(CodeAddr),
    Bltu(CodeAddr),
    Bgeu(CodeAddr),
    Lb(Reg, i16, CodeAddr), // XXX We'll definitely compress the instruction address in future.
    Lh(Reg, i16, CodeAddr), // XXX We'll definitely compress the instruction address in future.
    Lw(Reg, i16, CodeAddr), // XXX We'll definitely compress the instruction address in future.
    Lbu(Reg, i16, CodeAddr), // XXX We'll definitely compress the instruction address in future.
    Lhu(Reg, i16, CodeAddr), // XXX We'll definitely compress the instruction address in future.
    Sb(i16, CodeAddr),      // XXX We'll definitely compress the instruction address in future.
    Sh(i16, CodeAddr),      // XXX We'll definitely compress the instruction address in future.
    Sw(i16, CodeAddr),      // XXX We'll definitely compress the instruction address in future.
    Sd(i16, CodeAddr),      // XXX We'll definitely compress the instruction address in future.
    Addi(Reg, i16),
    Slti(Reg, i16),
    Sltiu(Reg, i16),
    Xori(Reg, i16),
    Ori(Reg, i16),
    Andi(Reg, i16),
    Xor(Reg),
    Or(Reg),
    And(Reg),
    Add(Reg),
    Sub(Reg),
    Sll(Reg),
    Srl(Reg),
    Sra(Reg),
    Slt(Reg),
    Sltu(Reg),
}

// Instruction in our virtual machine have two source registers as this is so
// common that it's worth optimizing for.  There is a small benefit to loading
// registers up-front so the fetch can happen in the shadow of the dispatch,
// even though there a small downside for instruction that need fewer or even no
// source registers.  Well, at least that the theory.
//
// XXX Should we factor out the destination register and always assign?  I
// suspect it might lead to less code in the execution loop at the cost of
// assigning to drain for instructions that doesn't return a result.  Very few
// common instruction would be affected: branches and stores are the main ones.
//
// Instruction sizes: currently Jal is the worst case, needing both the original
// address and a destination address. Representing the source address depends on
// the assumptions;
// - 64b when we can't assume anything
// - 16b when we can assume all source instruction come from a 64k window around
//   a baseaddress (which needs to be stored somewhere).
// - 1b when we can assume that all instructions come from a linear sequence
//   starting with the base address (+ follow jumps with some complication).
//   However reconstructing the address this way is expensive which is
//   unacceptable for jal (but ok for excepting instructions).
//
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
#[allow(clippy::cast_possible_wrap)]
#[allow(clippy::too_many_lines)] // Literally unavoidable
pub fn execute(code: &Code, cpu: &mut super::cpu::Cpu) -> Result<(), Exception> {
    for Insn(op, rs1, rs2) in code {
        println!("{op:x?} {rs1},{rs2}");
        let s1 = cpu.read_register(*rs1);
        let s2 = cpu.read_register(*rs2);
        match *op {
            Op::Const(rd, k) => cpu.write_x(rd, k),
            Op::Jal(rd, retaddr, target) => {
                cpu.pc = target;
                cpu.write_x(rd, retaddr);
            }
            Op::Jalr(rd, offset, retaddr) => {
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
            Op::Sb(offset, insn_addr) => {
                cpu.insn_addr = insn_addr;
                let _ = cpu.memop(Write, s1, offset as i64, s2, 1)?;
            }
            Op::Sh(offset, insn_addr) => {
                cpu.insn_addr = insn_addr;
                let _ = cpu.memop(Write, s1, offset as i64, s2, 2)?;
            }
            Op::Sw(offset, insn_addr) => {
                cpu.insn_addr = insn_addr;
                let _ = cpu.memop(Write, s1, offset as i64, s2, 4)?;
            }
            Op::Sd(offset, insn_addr) => {
                cpu.insn_addr = insn_addr;
                let _ = cpu.memop(Write, s1, offset as i64, s2, 8)?;
            }
            Op::Addi(rd, imm) => cpu.write_x(rd, s1.wrapping_add(i64::from(imm))),
            Op::Slti(rd, imm) => cpu.write_x(rd, i64::from(s1 < imm as i64)),
            Op::Sltiu(rd, imm) => cpu.write_x(rd, i64::from((s1 as u64) < imm as i64 as u64)),
            Op::Xori(rd, imm) => cpu.write_x(rd, s1 ^ i64::from(imm)),
            Op::Ori(rd, imm) => cpu.write_x(rd, s1 | i64::from(imm)),
            Op::Andi(rd, imm) => cpu.write_x(rd, s1 & i64::from(imm)),
            Op::Xor(rd) => cpu.write_x(rd, s1 ^ s2),
            Op::Or(rd) => cpu.write_x(rd, s1 | s2),
            Op::And(rd) => cpu.write_x(rd, s1 & s2),
            Op::Add(rd) => cpu.write_x(rd, s1.wrapping_add(s2)),
            Op::Sub(rd) => cpu.write_x(rd, s1.wrapping_sub(s2)),
            Op::Sll(rd) => cpu.write_x(rd, s1.wrapping_shl(s2 as u32)),
            Op::Srl(rd) => cpu.write_x(rd, (s1 as u64).wrapping_shr(s2 as u32) as i64),
            Op::Sra(rd) => cpu.write_x(rd, s1.wrapping_shr(s2 as u32)),
            Op::Slt(rd) => cpu.write_x(rd, i64::from(s1 < s2)),
            Op::Sltu(rd) => cpu.write_x(rd, i64::from((s1 as u64) < (s2 as u64))),

            Op::Unimplemented => todo!(),
        }
    }

    Ok(())
}

impl Default for TranslationCache {
    fn default() -> Self { Self::new() }
}
