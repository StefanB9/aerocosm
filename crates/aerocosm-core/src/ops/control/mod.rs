//! Runway control and arrival management (plan M3 PR 3).

// Temporary while the parts are added one commit at a time: they become reachable once
// operations use runway control (removed in that commit).
#![cfg_attr(not(test), allow(dead_code))]

mod arrival;
mod separation;
mod sequence;
