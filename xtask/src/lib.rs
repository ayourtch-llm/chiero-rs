//! Build/CI automation. See `docs/specs/001-architecture.md` §4 and
//! `docs/specs/070-testing-and-tdd-protocol.md` §4.

pub mod cc;
pub mod contracts;
pub mod deps;
pub mod mutation_gate;
pub mod proof_surface;
pub mod sweep;
pub mod vpp_leak;
