//! The solver. See `docs/specs/022-solver.md`.
//!
//! **Knows nothing about C and nothing about CIR** (001 §2). Its vocabulary is sorts and
//! terms, which keeps its test suite pure constraint solving and stops C semantics
//! leaking into a layer that must be trustworthy.

use indexmap::IndexMap;

pub mod eval;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sort {
    Bool,
    BitVec(u32),
}

/// A bitvector constant, stored in the low `width` bits.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BvConst {
    width: u32,
    bits: u128,
}

impl BvConst {
    pub fn new(_width: u32, _bits: u128) -> Self {
        todo!("green")
    }
    pub fn width(self) -> u32 {
        self.width
    }
    pub fn bits(self) -> u128 {
        self.bits
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Term(pub u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarId(pub u32);

/// A complete assignment. Every declared variable has a value (022 §2), which is what
/// makes evaluation total and a validated model a real witness.
#[derive(Clone, Debug, Default)]
pub struct Model {
    values: IndexMap<VarId, BvConst>,
}

impl Model {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&mut self, _v: VarId, _c: BvConst) {
        todo!("green")
    }
    pub fn get(&self, _v: VarId) -> Option<BvConst> {
        todo!("green")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalError(pub String);

#[derive(Debug, Default)]
pub struct TermArena {}

impl TermArena {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn bv(&mut self, _w: u32, _v: u128) -> Term {
        todo!("green")
    }
    pub fn var(&mut self, _s: Sort, _name: &str) -> Term {
        todo!("green")
    }
    pub fn var_id(&self, _t: Term) -> Option<VarId> {
        todo!("green")
    }
    pub fn as_const(&self, _t: Term) -> Option<BvConst> {
        todo!("green")
    }
    pub fn add(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn mul(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn and(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn udiv(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn sdiv(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn urem(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn srem(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn shl(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn lshr(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn ashr(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn ult(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn slt(&mut self, _a: Term, _b: Term) -> Term {
        todo!("green")
    }
    pub fn sext(&mut self, _a: Term, _w: u32) -> Term {
        todo!("green")
    }
    pub fn zext(&mut self, _a: Term, _w: u32) -> Term {
        todo!("green")
    }
    pub fn extract(&mut self, _a: Term, _hi: u32, _lo: u32) -> Term {
        todo!("green")
    }
    pub fn eval(&self, _m: &Model, _t: Term) -> Result<BvConst, EvalError> {
        todo!("green")
    }
    pub fn eval_ground(&self, _t: Term) -> Result<BvConst, EvalError> {
        todo!("green")
    }
    pub fn eval_ground_bool(&self, _t: Term) -> Result<bool, EvalError> {
        todo!("green")
    }
}
