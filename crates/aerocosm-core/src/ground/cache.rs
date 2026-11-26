//! Taxi routes kept once found (plan M9 PR 4): a route depends only on the
//! airport's layout, which never changes while the simulation runs, its two
//! ends and which taxiways the wingspan may use. Most searches repeat an
//! earlier one (in a midday hour 88 %), so each airport keeps its routes
//! and gives them again, exactly as searched.

use std::{collections::HashMap, sync::Mutex};

use super::route::GroundRoute;
use crate::units::Meters;

/// Routes kept per airport before the store is emptied and starts again:
/// bounds the memory, never the results.
const MAX_ROUTES: usize = 8192;

/// One airport's routes, by their ends and wingspan class.
pub(super) struct RouteCache {
    /// The airport's distinct taxiway wingspan limits, ascending: a
    /// wingspan's class is how many of them it exceeds, so all wingspans of
    /// a class may use the same taxiways.
    limits: Vec<f64>,
    routes: Mutex<HashMap<(u32, u32, u32), Option<GroundRoute>>>,
}

impl RouteCache {
    /// An empty store for an airport whose taxiways admit `limits`.
    pub(super) fn new(limits: impl IntoIterator<Item = Meters>) -> Self {
        let mut limits: Vec<f64> = limits.into_iter().map(Meters::value).collect();
        limits.sort_by(f64::total_cmp);
        limits.dedup();
        Self {
            limits,
            routes: Mutex::new(HashMap::new()),
        }
    }

    /// How many of the limits `wingspan` exceeds.
    fn class(&self, wingspan: Meters) -> u32 {
        let exceeded = self
            .limits
            .partition_point(|&limit| limit < wingspan.value());
        u32::try_from(exceeded).unwrap_or(u32::MAX)
    }

    /// The route from `from` to `to` for `wingspan`: kept if found before,
    /// else searched with `search` and kept.
    pub(super) fn get_or_search(
        &self,
        (from, to): (u32, u32),
        wingspan: Meters,
        search: impl FnOnce() -> Option<GroundRoute>,
    ) -> Option<GroundRoute> {
        let key = (from, to, self.class(wingspan));
        // A poisoned lock only means another thread panicked mid-insert;
        // searching again gives the same route.
        let Ok(mut routes) = self.routes.lock() else {
            return search();
        };
        if let Some(route) = routes.get(&key) {
            return route.clone();
        }
        let route = search();
        if routes.len() >= MAX_ROUTES {
            routes.clear();
        }
        routes.insert(key, route.clone());
        route
    }

    /// How many routes are kept.
    pub(super) fn len(&self) -> usize {
        self.routes.lock().map_or(0, |routes| routes.len())
    }
}

/// A copy keeps the limits and starts without routes.
impl Clone for RouteCache {
    fn clone(&self) -> Self {
        Self {
            limits: self.limits.clone(),
            routes: Mutex::new(HashMap::new()),
        }
    }
}

/// Kept routes are invisible: two layouts are equal whatever they hold.
impl PartialEq for RouteCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for RouteCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouteCache")
            .field("limits", &self.limits)
            .field("routes", &self.len())
            .finish()
    }
}
