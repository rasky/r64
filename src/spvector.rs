extern crate emu;

use super::sp::Sp;
use byteorder::{BigEndian, ByteOrder, LittleEndian};
use emu::bus::be::{Bus, DevPtr};
use emu::int::Numerics;
use mips64::{Cop, CpuContext};
use slog;
use std::arch::x86_64::*;
use std::cell::RefCell;
use std::rc::Rc;

// Vector registers as array of u8.
// We define a separate structure for this array to be able
// to specify alignment, since these will be used with SSE intrinsics.
#[repr(align(16))]
struct VectorRegs([[u8; 16]; 32]);

#[derive(Copy, Clone)]
#[repr(align(16))]
struct VectorReg([u8; 16]);

pub struct SpVector {
    vregs: VectorRegs,
    accum: [VectorReg; 3],
    vco_carry: VectorReg,
    vco_ne: VectorReg,
    sp: DevPtr<Sp>,
    logger: slog::Logger,
}

impl SpVector {
    pub const REG_VCC: usize = 33;
    pub const REG_ACCUM_LO: usize = 34;
    pub const REG_ACCUM_MD: usize = 35;
    pub const REG_ACCUM_HI: usize = 36;

    pub fn new(sp: &DevPtr<Sp>, logger: slog::Logger) -> Box<SpVector> {
        Box::new(SpVector {
            vregs: VectorRegs([[0u8; 16]; 32]),
            accum: [VectorReg([0u8; 16]); 3],
            vco_carry: VectorReg([0u8; 16]),
            vco_ne: VectorReg([0u8; 16]),
            sp: sp.clone(),
            logger,
        })
    }

    fn oploadstore(op: u32, ctx: &CpuContext) -> (u32, usize, u32, u32, u32) {
        let base = ctx.regs[((op >> 21) & 0x1F) as usize] as u32;
        let vt = ((op >> 16) & 0x1F) as usize;
        let opcode = (op >> 11) & 0x1F;
        let element = (op >> 7) & 0xF;
        let offset = op & 0x7F;
        (base, vt, opcode, element, offset)
    }

    fn vcc(&self) -> u16 {
        let mut res = 0u16;
        for i in 0..8 {
            res |= LittleEndian::read_u16(&self.vco_carry.0[i * 2..]) << i;
            res |= LittleEndian::read_u16(&self.vco_ne.0[i * 2..]) << (i + 8);
        }
        res
    }

    fn set_vcc(&mut self, vcc: u16) {
        for i in 0..8 {
            LittleEndian::write_u16(&mut self.vco_carry.0[i * 2..], (vcc >> i) & 1);
            LittleEndian::write_u16(&mut self.vco_ne.0[i * 2..], (vcc >> (i + 8)) & 1);
        }
    }
}

struct Vectorop<'a> {
    op: u32,
    spv: &'a mut SpVector,
}

impl<'a> Vectorop<'a> {
    fn func(&self) -> u32 {
        self.op & 0x3F
    }
    fn e(&self) -> usize {
        ((self.op >> 21) & 0xF) as usize
    }
    fn rs(&self) -> usize {
        ((self.op >> 11) & 0x1F) as usize
    }
    fn rt(&self) -> usize {
        ((self.op >> 16) & 0x1F) as usize
    }
    fn rd(&self) -> usize {
        ((self.op >> 6) & 0x1F) as usize
    }
    fn vs(&self) -> __m128i {
        unsafe { _mm_loadu_si128(self.spv.vregs.0[self.rs()].as_ptr() as *const _) }
    }
    fn vt(&self) -> __m128i {
        unsafe { _mm_loadu_si128(self.spv.vregs.0[self.rt()].as_ptr() as *const _) }
    }
    fn setvd(&mut self, val: __m128i) {
        unsafe {
            let rd = self.rd();
            _mm_store_si128(self.spv.vregs.0[rd].as_ptr() as *mut _, val);
        }
    }
    fn accum(&self, idx: usize) -> __m128i {
        unsafe { _mm_loadu_si128(self.spv.accum[idx].0.as_ptr() as *const _) }
    }
    fn setaccum(&mut self, idx: usize, val: __m128i) {
        unsafe { _mm_store_si128(self.spv.accum[idx].0.as_ptr() as *mut _, val) }
    }
    fn carry(&self) -> __m128i {
        unsafe { _mm_loadu_si128(self.spv.vco_carry.0.as_ptr() as *const _) }
    }
    fn setcarry(&self, val: __m128i) {
        unsafe { _mm_store_si128(self.spv.vco_carry.0.as_ptr() as *mut _, val) }
    }
}

impl Cop for SpVector {
    fn reg(&self, idx: usize) -> u128 {
        match idx {
            SpVector::REG_VCC => self.vcc() as u128,
            SpVector::REG_ACCUM_LO => LittleEndian::read_u128(&self.accum[0].0),
            SpVector::REG_ACCUM_MD => LittleEndian::read_u128(&self.accum[1].0),
            SpVector::REG_ACCUM_HI => LittleEndian::read_u128(&self.accum[2].0),
            _ => LittleEndian::read_u128(&self.vregs.0[idx]),
        }
    }
    fn set_reg(&mut self, idx: usize, val: u128) {
        match idx {
            SpVector::REG_VCC => self.set_vcc(val as u16),
            SpVector::REG_ACCUM_LO => LittleEndian::write_u128(&mut self.accum[0].0, val),
            SpVector::REG_ACCUM_MD => LittleEndian::write_u128(&mut self.accum[1].0, val),
            SpVector::REG_ACCUM_HI => LittleEndian::write_u128(&mut self.accum[2].0, val),
            _ => LittleEndian::write_u128(&mut self.vregs.0[idx], val),
        }
    }

    fn op(&mut self, _cpu: &mut CpuContext, op: u32) {
        let mut op = Vectorop { op, spv: self };
        unsafe {
            match op.func() {
                0x10 => {
                    // VADD
                    if op.e() != 0 {
                        unimplemented!();
                    }
                    let vs = op.vs();
                    let vt = op.vt();
                    let carry = op.carry();
                    let res = _mm_adds_epi16(_mm_adds_epi16(vs, vt), carry);
                    op.setvd(res);
                    op.setcarry(_mm_setzero_si128());
                }
                0x1D => {
                    // VSAR
                    let e = op.e();
                    match e {
                        8..=10 => {
                            let sar = op.accum(e - 8);
                            op.setvd(sar);
                            let new = op.vs();
                            op.setaccum(e - 8, new);
                        }
                        _ => unimplemented!(),
                    }
                }
                _ => panic!("unimplemented VU opcode={}", op.func().hex()),
            }
        }
    }

    fn lwc(&mut self, op: u32, ctx: &CpuContext, _bus: &Rc<RefCell<Box<Bus>>>) {
        let sp = self.sp.borrow();
        let dmem = sp.dmem.buf();
        let (base, vt, op, element, offset) = SpVector::oploadstore(op, ctx);
        match op {
            0x04 => {
                // LQV
                let ea = ((base + (offset << 4)) & 0xFFF) as usize;
                let ea_end = (ea & !0xF) + 0x10;
                for (m, r) in dmem[ea..ea_end]
                    .iter()
                    .zip(self.vregs.0[vt].iter_mut().rev())
                {
                    *r = *m;
                }
            }
            _ => panic!("unimplemented VU load opcode={}", op.hex()),
        }
    }
    fn swc(&mut self, op: u32, ctx: &CpuContext, _bus: &Rc<RefCell<Box<Bus>>>) {
        let sp = self.sp.borrow();
        let mut dmem = sp.dmem.buf();
        let (base, vt, op, element, offset) = SpVector::oploadstore(op, ctx);
        match op {
            0x04 => {
                // SQV
                let ea = ((base + (offset << 4)) & 0xFFF) as usize;
                let ea_end = (ea & !0xF) + 0x10;
                for (m, r) in dmem[ea..ea_end]
                    .iter_mut()
                    .zip(self.vregs.0[vt].iter().rev())
                {
                    *m = *r;
                }
            }
            _ => panic!("unimplemented VU load opcode={}", op.hex()),
        }
    }

    fn ldc(&mut self, _op: u32, _ctx: &CpuContext, _bus: &Rc<RefCell<Box<Bus>>>) {
        unimplemented!()
    }
    fn sdc(&mut self, _op: u32, _ctx: &CpuContext, _bus: &Rc<RefCell<Box<Bus>>>) {
        unimplemented!()
    }
}