#![allow(clippy::unreadable_literal, clippy::cast_possible_wrap)]

use crate::cpu;
use crate::csr;
use crate::device::clint::Clint;
use crate::device::plic::Plic;
use crate::device::uart::Uart;
use crate::device::virtio_block_disk::VirtioBlockDisk;
pub use crate::memory::*;
use crate::riscv;
use crate::terminal::Terminal;
use cpu::{
    CONFIG_SW_MANAGED_A_AND_D, Exception, MSTATUS_MPP_SHIFT, MSTATUS_MPRV, MSTATUS_MXR,
    MSTATUS_SUM, PG_SHIFT,
};
use csr::SATP_MODE_MASK;
use csr::SATP_MODE_SHIFT;
use csr::SATP_PPN_MASK;
use csr::SATP_PPN_SHIFT;
use csr::SatpMode;
use fnv::FnvHashMap;
use log::trace;
use log::warn;
use riscv::MemoryAccessType;
use riscv::PrivMode;
use riscv::Trap;
use riscv::priv_mode_from;

const DTB_SIZE: usize = 0xfe0;

/// Emulates Memory Management Unit. It holds the Main memory and peripheral
/// devices, maps address to them, and accesses them depending on address.
///
/// It also manages virtual-physical address translation and memory protection.
/// It could also be called Bus.
pub struct Mmu {
    // CPU state that lives here
    pub prv: PrivMode,
    pub mstatus: u64,
    pub mip: u64,
    pub satp: u64,

    pub memory: Memory,
    dtb: Vec<u8>,
    disk: VirtioBlockDisk,
    plic: Plic,
    clint: Clint,
    uart: Uart,

    /// Address translation page cache.
    /// The cache is cleared when translation mapping can be changed;
    /// SATP, or PRV is updated or FENCE.VMA is executed.
    /// Technically page table entries
    /// can be updated anytime with store instructions, but RISC-V
    /// allows for this and requires that FENCE.VMA is executed before
    /// any of these are observed.
    /// If you want to enable, use `enable_page_cache()`.
    page_cache_enabled: bool,
    fetch_page_cache: FnvHashMap<u64, u64>,
    load_page_cache: FnvHashMap<u64, u64>,
    store_page_cache: FnvHashMap<u64, u64>,
}

pub const PTE_V_MASK: u64 = 1 << 0;
pub const PTE_U_MASK: u64 = 1 << 4;
pub const PTE_A_MASK: u64 = 1 << 6;
pub const PTE_D_MASK: u64 = 1 << 7;

impl Mmu {
    /// Creates a new `Mmu`.
    ///
    /// # Arguments
    /// * `xlen`
    /// * `terminal`
    #[must_use]
    pub fn new(terminal: Box<dyn Terminal>) -> Self {
        let mut dtb = vec![0; DTB_SIZE];

        // Load default device tree binary content
        let content = include_bytes!("./device/dtb.dtb");
        dtb[..content.len()].copy_from_slice(&content[..]);

        Self {
            prv: PrivMode::M,
            mstatus: 0,
            mip: 0,
            satp: 0,
            memory: Memory::new(),
            dtb,
            disk: VirtioBlockDisk::new(),
            plic: Plic::new(),
            clint: Clint::new(),
            uart: Uart::new(terminal),
            page_cache_enabled: false,
            fetch_page_cache: FnvHashMap::default(),
            load_page_cache: FnvHashMap::default(),
            store_page_cache: FnvHashMap::default(),
        }
    }

    /// Initializes Main memory. This method is expected to be called only once.
    ///
    /// # Arguments
    /// * `capacity`
    pub fn init_memory(&mut self, capacity: usize) {
        self.memory.init(capacity);
    }

    /// Initializes Virtio block disk. This method is expected to be called only once.
    ///
    /// # Arguments
    /// * `data` Filesystem binary content
    pub fn init_disk(&mut self, data: Vec<u8>) {
        self.disk.init(data);
    }

    /// Overrides default Device tree configuration.
    ///
    /// # Arguments
    /// * `data` DTB binary content
    pub fn init_dtb(&mut self, data: &[u8]) {
        self.dtb[..data.len()].copy_from_slice(data);
        for i in data.len()..self.dtb.len() {
            self.dtb[i] = 0;
        }
    }

    /// Enables or disables page cache optimization.
    ///
    /// # Arguments
    /// * `enabled`
    pub fn enable_page_cache(&mut self, enabled: bool) {
        self.page_cache_enabled = enabled;
        self.clear_page_cache();
    }

    /// Clears page cache entries
    pub fn clear_page_cache(&mut self) {
        self.fetch_page_cache.clear();
        self.load_page_cache.clear();
        self.store_page_cache.clear();
    }

    /// Runs one cycle of MMU and peripheral devices.
    pub fn service(&mut self, cycle: u64) {
        self.clint.service(cycle, &mut self.mip);
        self.disk.service(&mut self.memory, cycle);
        self.uart.service();
        self.plic.service(
            self.disk.is_interrupting(),
            self.uart.is_interrupting(),
            &mut self.mip,
        );
    }

    /// Updates privilege mode
    ///
    /// # Arguments
    /// * `mode`
    pub fn update_priv_mode(&mut self, mode: PrivMode) {
        self.prv = mode;
        self.clear_page_cache();
    }

    /// # Errors
    /// Cannot really panic
    #[allow(clippy::result_unit_err, clippy::cast_possible_truncation)]
    pub fn load_mmio_u8(&mut self, pa: u64) -> Result<u8, ()> {
        match pa {
            0x00001020..=0x00001fff => Ok(self.dtb[pa as usize - 0x1020]),
            0x02000000..=0x0200ffff => Ok(self.clint.load(pa)),
            0x0C000000..=0x0fffffff => Ok(self.plic.load(pa)),
            0x10000000..=0x100000ff => Ok(self.uart.load(pa)),
            0x10001000..=0x10001FFF => Ok(self.disk.load(pa)),
            _ => Err(()),
        }
    }

    /// Stores a byte to main memory or peripheral devices depending on
    /// physical address.
    ///
    /// # Arguments
    /// * `pa` Physical address
    /// * `value` data written
    /// # Errors
    /// Will return error for access outside supported memory range
    #[allow(clippy::result_unit_err, clippy::cast_sign_loss)]
    pub fn store_mmio_u8(&mut self, pa: i64, value: u8) -> Result<(), ()> {
        let pa = pa as u64;
        match pa {
            0x02000000..=0x0200ffff => self.clint.store(pa, value, &mut self.mip),
            0x0c000000..=0x0fffffff => self.plic.store(pa, value, &mut self.mip),
            0x10000000..=0x100000ff => self.uart.store(pa, value),
            0x10001000..=0x10001FFF => self.disk.store(pa, value),
            _ => return Err(()),
        }
        Ok(())
    }

    /// # Panics
    /// It can panic which is bad and which is why it's going away soon
    /// # Errors
    /// If any part of the access is outside of memory, a unit error is returned
    #[allow(
        clippy::unwrap_used,
        clippy::result_unit_err,
        clippy::cast_possible_wrap
    )]
    pub fn store_phys_u8(&mut self, pa: u64, value: u8) -> Result<(), ()> {
        panic!("dead-code?");
        // @TODO: Mapping should be configurable with dtb
	/*

        if DRAM_BASE <= pa {
            self.memory.write_u8(pa, value)
        } else {
            panic!("8-bit store to non-memory @ {pa:016x}");
            self.store_mmio_u8(pa as i64, value)
        }
	 */

    }

    /// Stores two bytes to main memory or peripheral devices depending on
    /// physical address.
    ///
    /// # Arguments
    /// * `pa` Physical address
    /// * `value` data written
    /// # Panics
    /// It can panic which is bad and which is why it's going away soon
    /// # Errors
    /// If any part of the access is outside of memory, a unit error is returned
    #[allow(clippy::result_unit_err)]
    pub fn store_phys_u16(&mut self, pa: u64, value: u16) -> Result<(), ()> {
        panic!("dead-code?");
        /*
	if DRAM_BASE <= pa {
            self.memory.write_u16(pa, value)
        } else {
            panic!("16-bit store to non-memory @ {pa:016x}");
            for i in 0..2 {
                self.store_phys_u8(pa.wrapping_add(i), ((value >> (i * 8)) & 0xff) as u8)?;
            }
            Ok(())
        }
	*/
    }

    /// Stores four bytes to main memory or peripheral devices depending on
    /// physical address.
    ///
    /// # Arguments
    /// * `pa` Physical address
    /// * `value` data written
    /// # Errors
    /// If any part of the access is outside of memory, a unit error is returned
    #[allow(clippy::result_unit_err)]
    pub fn store_phys_u32(&mut self, pa: u64, value: u32) -> Result<(), ()> {
        panic!("dead-code?");
        /*
	if DRAM_BASE <= pa {
            self.memory.write_u32(pa, value)
        } else {
            panic!("32-bit store to non-memory @ {pa:016x}");
            for i in 0..4 {
                self.store_phys_u8(pa.wrapping_add(i), ((value >> (i * 8)) & 0xff) as u8)?;
            }
            Ok(())
        }
        */
    }

    /// Stores eight bytes to main memory or peripheral devices depending on
    /// physical address.
    ///
    /// # Arguments
    /// * `pa` Physical address
    /// * `value` data written
    /// # Errors
    /// If any part of the access is outside of memory, a unit error is returned
    #[allow(clippy::result_unit_err)]
    pub fn store_phys_u64(&mut self, pa: u64, value: u64) -> Result<(), ()> {
        panic!("dead-code?");
        /*
	if DRAM_BASE <= pa {
            self.memory.write_u64(pa, value)
        } else {
            panic!("64-bit store to non-memory @ {pa:016x}");
            for i in 0..8 {
                self.store_phys_u8(pa.wrapping_add(i), ((value >> (i * 8)) & 0xff) as u8)?;
            }
            Ok(())
        }
        */
    }

    /// # Errors
    /// If this fails then the error will have the exception that should be raised
    pub fn translate_address(
        &mut self,
        address: u64,
        access_type: MemoryAccessType,
        side_effect_free: bool,
    ) -> Result<u64, Exception> {
        let v_page = address & !0xfff;

        let cache = if self.page_cache_enabled {
            match access_type {
                MemoryAccessType::Execute => self.fetch_page_cache.get(&v_page),
                MemoryAccessType::Read => self.load_page_cache.get(&v_page),
                MemoryAccessType::Write => self.store_page_cache.get(&v_page),
            }
        } else {
            None
        };

        if let Some(p_page) = cache {
            return Ok(p_page | (address & 0xfff));
        }

        let pa = self.translate_address_slow(address, access_type, side_effect_free)?;

        if self.page_cache_enabled && !side_effect_free {
            let p_page = pa & !0xfff;
            let _ = match access_type {
                MemoryAccessType::Execute => self.fetch_page_cache.insert(v_page, p_page),
                MemoryAccessType::Read => self.load_page_cache.insert(v_page, p_page),
                MemoryAccessType::Write => self.store_page_cache.insert(v_page, p_page),
            };
        }

        Ok(pa)
    }

    #[allow(
        clippy::cast_possible_wrap,
        clippy::too_many_lines,
        clippy::expect_used,
        clippy::cognitive_complexity
    )]
    fn translate_address_slow(
        &mut self,
        va: u64,
        access: MemoryAccessType,
        side_effect_free: bool,
    ) -> Result<u64, Exception> {
        let prv = self.prv;
        let effective_prv =
            if self.mstatus & MSTATUS_MPRV != 0 && access != MemoryAccessType::Execute {
                // Use previous privilege
                priv_mode_from((self.mstatus >> MSTATUS_MPP_SHIFT) & 3)
            } else {
                prv
            };

        let satp_mode = ((self.satp >> SATP_MODE_SHIFT) & SATP_MODE_MASK) as usize;
        if effective_prv == PrivMode::M || satp_mode == SatpMode::Bare as usize {
            return Ok(va);
        }

        // Sv39, Sv48, Sv57
        let levels = 3 + satp_mode - SatpMode::Sv39 as usize;
        let access_shift = match access {
            MemoryAccessType::Read => 0,
            MemoryAccessType::Write => 1,
            MemoryAccessType::Execute => 2,
        };

        let pte_size_log2 = 3;
        let vaddr_shift = 64 - (PG_SHIFT + levels * 9);
        // Check for canonical addresses
        if ((va as i64) << vaddr_shift) >> vaddr_shift != va as i64 {
            // XXX Some debugging logging here might be useful
            return page_fault(va as i64, access);
        }
        let pte_addr_bits = 44;
        let page_table_root = (self.satp >> SATP_PPN_SHIFT) & SATP_PPN_MASK;
        let mut pte_addr = (page_table_root & ((1 << pte_addr_bits) - 1)) << PG_SHIFT;
        let pte_bits = 12 - pte_size_log2;
        let pte_mask = (1 << pte_bits) - 1;

        for i in 0..levels {
            let vaddr_shift = PG_SHIFT + pte_bits * (levels - 1 - i);
            let pte_idx = (va >> vaddr_shift) & pte_mask;
            pte_addr += pte_idx << pte_size_log2;
            // XXX Not only do we need to raise an exception if this
            // fails, but failing here doesn't cause a page fault but
            // just a fault (eg CAUSE_FAULT_LOAD/STORE instead of all
            // the others which are
            // CAUSE_LOAD/STORE/FETCH_PAGE_FAULT).
            let pte = self.memory.read_u64(pte_addr);
            // return access_fault(address, access_type);

            if pte & PTE_V_MASK == 0 {
                trace!("** {prv:?} mode access to {va:08x} denied: invalid PTE");
                break;
            }

            // XXX too many hardcoded values
            let paddr = (pte >> 10) << PG_SHIFT;
            let mut xwr = (pte >> 1) & 7;
            if xwr == 0 {
                pte_addr = paddr;
                continue;
            }

            // *** Found a leaf node ***

            if xwr == 2 || xwr == 6 {
                trace!("** {prv:?} mode access to {va:08x} denied: invalid xwr {xwr}");
                break;
            }

            // priviledge check
            if effective_prv == PrivMode::S {
                if pte & PTE_U_MASK != 0 && self.mstatus & MSTATUS_SUM == 0 {
                    // XXX Debug log would be useful
                    warn!("** {prv:?} mode access to {va:08x} denied: U & !SUM");
                    break;
                }
            } else if pte & PTE_U_MASK == 0 {
                // XXX Debug log would be useful
                warn!("** {prv:?} mode access to {va:08x} denied: !U");
                return page_fault(va as i64, access);
            }

            /* protection check */
            /* MXR allows read access to execute-only pages */
            if self.mstatus & MSTATUS_MXR != 0 {
                xwr |= xwr >> 2;
            }

            if (xwr >> access_shift) & 1 == 0 {
                let want = 1 << access_shift;
                trace!("** {prv:?} mode access to {va:08x} denied: want {want}, got {xwr}");
                break;
            }

            /* 6. Check for misaligned superpages */
            let ppn = pte >> 10;
            let j = levels - 1 - i;
            if ((1 << j) - 1) & ppn != 0 {
                warn!("** access to {va:08x} denied: misaligned superpage {i} / {ppn}");
                break;
            }

            /*
              RISC-V Priv. Spec 1.11 (draft) Section 4.3.1 offers two
              ways to handle the A and D TLB flags.  Spike uses the
              software managed approach whereas Dromajo used to manage
              them (causing far fewer exceptions).
            */
            if CONFIG_SW_MANAGED_A_AND_D {
                if pte & PTE_A_MASK == 0 {
                    trace!("** {prv:?} mode access to {va:08x} denied: missing A");
                    break; // Must have A on access
                }
                if access == MemoryAccessType::Write && pte & PTE_D_MASK == 0 {
                    trace!("** {prv:?} mode access to {va:08x} denied: missing D");
                    break; // Must have D on write
                }
            } else {
                let mut new_pte = pte | PTE_A_MASK;
                if access == MemoryAccessType::Write {
                    new_pte |= PTE_D_MASK;
                }
                if pte != new_pte
                    && !side_effect_free
                    && self.store_phys_u64(pte_addr, new_pte).is_err()
                {
                    return access_fault(va as i64, access);
                }
            }

            let vaddr_mask = (1 << vaddr_shift) - 1;
            return Ok(paddr & !vaddr_mask | va & vaddr_mask);
        }

        page_fault(va as i64, access)
    }

    /// Returns immutable reference to `Clint`.
    #[must_use]
    pub const fn get_clint(&self) -> &Clint {
        &self.clint
    }

    /// Returns mutable reference to `Clint`.
    pub const fn get_mut_clint(&mut self) -> &mut Clint {
        &mut self.clint
    }

    /// Returns mutable reference to `Uart`.
    pub const fn get_mut_uart(&mut self) -> &mut Uart {
        &mut self.uart
    }
}

#[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)] // XXX Try to remove this later when the u64 -> i64 conversion is done
const fn page_fault<T>(address: i64, access_type: MemoryAccessType) -> Result<T, Exception> {
    Err::<T, Exception>(Exception {
        trap: match access_type {
            MemoryAccessType::Read => Trap::LoadPageFault,
            MemoryAccessType::Write => Trap::StorePageFault,
            MemoryAccessType::Execute => Trap::InstructionPageFault,
        },
        tval: address,
    })
}

#[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)] // XXX Try to remove this later when the u64 -> i64 conversion is done
const fn access_fault<T>(address: i64, access_type: MemoryAccessType) -> Result<T, Exception> {
    Err::<T, Exception>(Exception {
        trap: match access_type {
            MemoryAccessType::Read => Trap::LoadAccessFault,
            MemoryAccessType::Write => Trap::StoreAccessFault,
            MemoryAccessType::Execute => Trap::InstructionAccessFault,
        },
        tval: address,
    })
}
