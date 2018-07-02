extern crate num;

use self::num::Float;
use super::cpu::Cpu;
use super::Mipsop;
use std::marker::PhantomData;

#[derive(Default)]
pub(crate) struct Cop1 {
    pub(crate) regs: [u64; 32],
    fir: u64,
    fccr: u64,
    fexr: u64,
    fenr: u64,
    fcsr: u64,
}

trait FloatRawConvert {
    fn from_u64bits(v: u64) -> Self;
    fn to_u64bits(self) -> u64;
    fn bankers_round(self) -> Self;
}

impl FloatRawConvert for f32 {
    fn from_u64bits(v: u64) -> Self {
        f32::from_bits(v as u32)
    }
    fn to_u64bits(self) -> u64 {
        self.to_bits() as u64
    }
    fn bankers_round(self) -> Self {
        let y = self.round();
        if (self - y).abs() == 0.5 {
            (self * 0.5).round() * 2.0
        } else {
            y
        }
    }
}

impl FloatRawConvert for f64 {
    fn from_u64bits(v: u64) -> Self {
        f64::from_bits(v)
    }
    fn to_u64bits(self) -> u64 {
        self.to_bits()
    }
    fn bankers_round(self) -> Self {
        let y = self.round();
        if (self - y).abs() == 0.5 {
            (self * 0.5).round() * 2.0
        } else {
            y
        }
    }
}

struct Fop<'a, F: Float + FloatRawConvert> {
    opcode: u32,
    cpu: &'a mut Cpu,
    phantom: PhantomData<F>,
}

impl<'a, F: Float + FloatRawConvert> Fop<'a, F> {
    fn func(&self) -> u32 {
        self.opcode & 0x3f
    }
    fn rs(&self) -> usize {
        ((self.opcode >> 11) & 0x1f) as usize
    }
    fn rt(&self) -> usize {
        ((self.opcode >> 16) & 0x1f) as usize
    }
    fn rd(&self) -> usize {
        ((self.opcode >> 6) & 0x1f) as usize
    }
    fn fs(&self) -> F {
        F::from_u64bits(self.cpu.cop1.regs[self.rs()])
    }
    fn ft(&self) -> F {
        F::from_u64bits(self.cpu.cop1.regs[self.rt()])
    }
    fn set_fd(&mut self, v: F) {
        self.cpu.cop1.regs[self.rd()] = v.to_u64bits();
    }
    fn mfd64(&'a mut self) -> &'a mut u64 {
        &mut self.cpu.cop1.regs[self.rd()]
    }
}

macro_rules! approx {
    ($op:ident, $round:ident, $size:ident) => {{
        match $op.fs().$round().$size() {
            Some(v) => *$op.mfd64() = v as u64,
            None => panic!("approx out of range"),
        }
    }};
}

impl Cop1 {
    fn fop<M: Float + FloatRawConvert>(cpu: &mut Cpu, opcode: u32) {
        let mut op = Fop::<M> {
            opcode,
            cpu,
            phantom: PhantomData,
        };
        match op.func() {
            0x00 => {
                // ADD.fmt
                let v = op.fs() + op.ft();
                op.set_fd(v)
            }
            0x01 => {
                // SUB.fmt
                let v = op.fs() - op.ft();
                op.set_fd(v)
            }
            0x02 => {
                // MUL.fmt
                let v = op.fs() * op.ft();
                op.set_fd(v)
            }
            0x03 => {
                // DIV.fmt
                let v = op.fs() / op.ft();
                op.set_fd(v)
            }
            0x04 => {
                // SQRT.fmt
                let v = op.fs().sqrt();
                op.set_fd(v)
            }
            0x05 => {
                // ABS.fmt
                let v = op.fs().abs();
                op.set_fd(v)
            }
            0x07 => {
                // NEG.fmt
                let v = op.fs().neg();
                op.set_fd(v)
            }
            0x08 => approx!(op, bankers_round, to_i64), // ROUND.L.fmt
            0x09 => approx!(op, trunc, to_i64),         // TRUNC.L.fmt
            0x0A => approx!(op, ceil, to_i64),          // CEIL.L.fmt
            0x0B => approx!(op, floor, to_i64),         // FLOOR.L.fmt
            0x0C => approx!(op, bankers_round, to_i32), // ROUND.W.fmt
            0x0D => approx!(op, trunc, to_i32),         // TRUNC.W.fmt
            0x0E => approx!(op, ceil, to_i32),          // CEIL.W.fmt
            0x0F => approx!(op, floor, to_i32),         // FLOOR.W.fmt
            _ => panic!("unimplemented COP1 opcode: func={:x?}", op.func()),
        }
    }

    #[inline(always)]
    pub(crate) fn op(cpu: &mut Cpu, opcode: u32) {
        let fmt = (opcode >> 21) & 0x1F;
        match fmt {
            16 => Cop1::fop::<f32>(cpu, opcode),
            17 => Cop1::fop::<f64>(cpu, opcode),
            _ => panic!("unimplemented COP1 fmt: fmt={:x?}", fmt),
        }
    }
}
