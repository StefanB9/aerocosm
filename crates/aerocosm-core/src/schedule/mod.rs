//! Schedule generation: a deterministic weekly template of flights grouped into
//! tails (plan M2, PR 2).

// Temporary while the stages are added one commit at a time: they are crate-internal and only
// become reachable once `generate` composes them (removed in that commit).
#![cfg_attr(not(test), allow(dead_code))]

mod demand;
mod overflight;
mod rotation;
mod timetable;
