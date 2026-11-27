//! Freely flying aircraft advanced in parallel (plan M9 D178, PR 5). Most
//! aircraft in a tick neither book a landing slot nor ask runway control
//! anything: their step only advances their own motion. A pass before the
//! fleet's step advances all of them on every core; the step then takes each
//! result in tail order, where it would have computed it, and handles its
//! events as before. A result is used only if the tail is still exactly as
//! the pass found it, so a tick is the same whatever the number of threads.

use super::Operations;
use crate::{
    SimTime,
    ops::{
        Phase,
        motion::{Event, Motion},
    },
};

/// One freely flying tail's tick: its motion before and after, and the event
/// it met.
#[derive(Debug, Clone, Copy)]
pub(super) struct Flown {
    pub(super) start: Motion,
    pub(super) motion: Motion,
    pub(super) event: Option<Event>,
}

/// The pass's results, reused from tick to tick.
#[derive(Debug, Clone)]
pub(super) struct FreeFlight {
    /// Whether the pass runs; off, every tail steps as before (tests).
    pub(super) enabled: bool,
    /// The tails the pass advanced this tick, ascending, with their motion
    /// before.
    pub(super) tails: Vec<(u32, Motion)>,
    /// Their ticks, in the same order.
    pub(super) flown: Vec<Flown>,
}

impl Default for FreeFlight {
    fn default() -> Self {
        Self {
            enabled: true,
            tails: Vec::new(),
            flown: Vec::new(),
        }
    }
}

/// Scratch, not state: two fleets are equal whatever their passes hold.
impl PartialEq for FreeFlight {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// `f` of every item, in order: on every core where threads exist, else one
/// after another (the web build).
fn map_in_order<I: Sync, T: Send>(items: &[I], f: impl Fn(&I) -> T + Sync + Send) -> Vec<T> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        items.par_iter().map(f).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        items.iter().map(f).collect()
    }
}

impl Operations {
    /// Advances every tail that flies freely this tick, in parallel; the
    /// fleet's step takes the results (plan M9 PR 5).
    pub(super) fn fly_free_ahead(&mut self, now: SimTime, dt: f64) {
        let mut free = std::mem::take(&mut self.free);
        free.tails.clear();
        free.flown.clear();
        if free.enabled {
            for index in 0..self.tails.len() {
                if self.look_at[index] > now {
                    continue;
                }
                let Some(motion) = self.tails[index].motion else {
                    continue;
                };
                let phase = motion.phase();
                if phase != Phase::Boarding
                    && !self.controlled(index, phase)
                    && self.flies_free(index, &motion)
                    && let Ok(tail) = u32::try_from(index)
                {
                    free.tails.push((tail, motion));
                }
            }
            let this = &*self;
            free.flown = map_in_order(&free.tails, |&(tail, start)| {
                let index = tail as usize;
                let mut motion = start;
                let event = motion.advance_free(&this.profiles[index], dt);
                Flown {
                    start,
                    motion,
                    event,
                }
            });
        }
        self.free = free;
    }

    /// The pass's tick for tail `index`, if it advanced it and the tail is
    /// still exactly as it found it; `cursor` walks the results in step.
    pub(super) fn flown(&self, index: usize, cursor: &mut usize) -> Option<Flown> {
        let tail = u32::try_from(index).ok()?;
        while self.free.tails.get(*cursor).is_some_and(|&(t, _)| t < tail) {
            *cursor += 1;
        }
        if self.free.tails.get(*cursor).map(|&(t, _)| t) != Some(tail) {
            return None;
        }
        let flown = *self.free.flown.get(*cursor)?;
        let unchanged = self.tails[index].motion == Some(flown.start)
            && flown.start.phase() != Phase::Boarding
            && !self.controlled(index, flown.start.phase())
            && self.flies_free(index, &flown.start);
        unchanged.then_some(flown)
    }
}
