pub mod lsp;
pub mod fmt;
pub mod ast;
pub mod cache;
pub mod check;
// SWARM RABBIT HOLE — Wave 15 (Impl-T, Cohort Delta): Salt ε's DBM
// zone layer — the contract engine's arithmetic backbone (compile
// time only; the runtime sees just the degraded assertions).
pub mod dbm;
pub mod diag;
pub mod driver;
pub mod ir;
pub mod lexer;
pub mod llvm;
pub mod parser;
pub mod pkg;
pub mod span;
pub mod token;
pub mod translate;
pub mod types;
pub mod test;

