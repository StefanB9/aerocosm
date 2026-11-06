//! Flight operations: tails flying their weekly rotations, phase by phase
//! (plan M2, PR 3).

// Temporary while the parts are added one commit at a time: they become reachable once the
// simulation runs operations (removed in that commit).
#![cfg_attr(not(test), allow(dead_code))]

mod runway;
