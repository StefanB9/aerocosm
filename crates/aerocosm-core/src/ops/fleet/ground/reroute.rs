//! Rerouting out of a deadlock (plan M4 D93): when aircraft wait for each
//! other, the controller sends the one to give way another way from the
//! next node ahead of it, round the taxiways the other stands on. Only when
//! there is none does it taxi on regardless (D75).

use super::{Start, Target};
use crate::{
    Id, LatLon, SimTime,
    ground::{
        AirportGround, Edge, Node,
        junctions::{Passage, Run},
    },
    ops::{Phase, fleet::Operations, motion::Motion},
    scenario::Scenario,
};

/// Most times one taxi is sent another way before it taxies on regardless.
const MOST: u8 = 3;

impl Operations {
    /// Sends tail `index`, in `motion` and the one to give way in a
    /// deadlock, another way from the next node ahead, round the taxiways
    /// the aircraft it waits for stands on and not on along the way it was
    /// going; whether there was one.
    pub(super) fn reroute(
        &mut self,
        scenario: &Scenario,
        index: usize,
        motion: &Motion,
        now: SimTime,
    ) -> bool {
        let out = match motion.phase() {
            Phase::TaxiOut => true,
            Phase::TaxiIn => false,
            _ => return false,
        };
        let along = motion.along();
        let Some(old) = self.taxis[index].as_ref() else {
            return false;
        };
        if old.escaped || old.reroutes >= MOST || along < old.push {
            return false;
        }
        let airport = old.airport;
        let (Some(layout), Some(airside), Some(to)) = (
            scenario.ground().airport(airport),
            self.ground.airside(airport),
            old.end_node,
        ) else {
            return false;
        };
        let Some((k, from, at)) = node_ahead(layout, &old.edges, old.offset, along) else {
            return false;
        };
        if from == to {
            return false;
        }
        let mut avoid: Vec<u32> = Vec::new();
        // The junction it is held up at, and those the other is in or
        // cleared into: a way through them would leave it waiting still.
        let mut closed: Vec<u32> = old
            .passages
            .iter()
            .find(|p| p.entry > along)
            .map(|p| p.junction)
            .into_iter()
            .collect();
        if let Some(blocker) = old.waiting_for {
            for (chain, list) in airside.on_chain.iter().enumerate() {
                if list.iter().any(|o| o.tail == blocker) {
                    avoid.push(u32::try_from(chain).unwrap_or(u32::MAX));
                }
            }
            for (j, (inside, claim)) in airside.occupied.iter().zip(&airside.claims).enumerate() {
                if inside.contains(&blocker) || *claim == Some(blocker) {
                    closed.push(u32::try_from(j).unwrap_or(u32::MAX));
                }
            }
        }
        let shut = |node: Id<Node>| {
            airside
                .junctions
                .junction_of(node)
                .is_some_and(|j| closed.contains(&j))
        };
        if shut(from) {
            return false;
        }
        let ahead = old.edges.get(k).map(|&(edge, _)| edge);
        avoid.extend(
            ahead
                .and_then(|edge| airside.junctions.chain(edge))
                .map(|(c, _)| c),
        );
        // It cannot turn round where it stands.
        let behind = k.checked_sub(1).and_then(|i| old.edges.get(i)).copied();
        let owner = Self::owner(index);
        let route = self.route_avoiding(layout, index, (from, to), |edge, forward| {
            Some(edge) == ahead
                || behind == Some((edge, !forward))
                || layout
                    .edge(edge)
                    .is_some_and(|e| shut(e.from()) || shut(e.to()))
                || airside.junctions.chain(edge).is_some_and(|(chain, way)| {
                    // Never into a taxiway someone is coming down.
                    airside.opposed(owner, chain, way == forward).is_some()
                })
                || airside
                    .junctions
                    .chain(edge)
                    .is_some_and(|(chain, _)| avoid.contains(&chain))
        });
        let Some(route) = route else {
            return false;
        };
        let old = old.clone();
        let extension = match (out, old.target, self.paths[index].as_ref()) {
            (true, Target::Hold, Some(path)) => path.queue_points().to_vec(),
            _ => Vec::new(),
        };
        let start = Start {
            route,
            from,
            target: old.target,
            base: cut(&old.points, at),
            offset: at,
            extension,
        };
        let Some(polyline) = self.start_taxi(scenario, index, airport, start, now) else {
            self.taxis[index] = Some(old);
            return false;
        };
        let Some(taxi) = self.taxis[index].as_mut() else {
            return false;
        };
        taxi.passages = splice_passages(&old.passages, &taxi.passages, at);
        taxi.runs = splice_runs(&old.runs, &taxi.runs, at);
        let mut edges = old.edges[..k].to_vec();
        edges.extend_from_slice(&taxi.edges);
        taxi.edges = edges;
        taxi.offset = old.offset;
        taxi.next = taxi
            .passages
            .iter()
            .take_while(|p| p.entry < at - 1e-6)
            .count();
        taxi.locked = taxi.runs.iter().take_while(|r| r.from < at - 1e-6).count();
        taxi.limit = along;
        taxi.goal = old.goal;
        taxi.stand = old.stand;
        taxi.push = old.push;
        taxi.started = old.started;
        taxi.unimpeded = old.unimpeded;
        taxi.reroutes = old.reroutes + 1;
        // It keeps what it stands on and gives up what lay ahead.
        let cleared: Vec<u32> = taxi.passages[..taxi.next]
            .iter()
            .map(|p| p.junction)
            .collect();
        let current: Vec<(u32, bool)> = taxi.runs[..taxi.locked]
            .iter()
            .filter(|r| r.to > along)
            .map(|r| (r.chain, r.forward))
            .collect();
        if let Some(airside) = self.ground.airside_mut(airport) {
            for (j, claim) in airside.claims.iter_mut().enumerate() {
                let j = u32::try_from(j).unwrap_or(u32::MAX);
                if *claim == Some(owner) && !cleared.contains(&j) {
                    *claim = None;
                }
            }
            for locks in &mut airside.locks {
                locks.retain(|&(other, _)| other != owner);
            }
            for (chain, forward) in current {
                airside.lock(owner, chain, forward);
            }
        }
        if let Some(path) = self.paths[index].as_mut() {
            let length = path.retaxi(out, polyline);
            if out {
                self.profiles[index].set_taxi_out_length(length);
            } else {
                self.profiles[index].set_taxi_in_length(length);
            }
        }
        self.stats.rerouted(airport);
        true
    }
}

/// The first node at or ahead of `along` metres on the route `edges`, its
/// first node `offset` metres along: how many edges lead to it, the node
/// and where it is.
fn node_ahead(
    layout: &AirportGround,
    edges: &[(Id<Edge>, bool)],
    offset: f64,
    along: f64,
) -> Option<(usize, Id<Node>, f64)> {
    let mut at = offset;
    for (k, &(id, forward)) in edges.iter().enumerate() {
        let edge = layout.edge(id)?;
        at += edge.length().value();
        if at >= along - 1e-6 {
            return Some((k + 1, if forward { edge.to() } else { edge.from() }, at));
        }
    }
    None
}

/// The polyline `points` up to `distance` metres along it.
fn cut(points: &[LatLon], distance: f64) -> Vec<LatLon> {
    let mut kept = Vec::with_capacity(points.len());
    let mut walked = 0.0;
    for pair in points.windows(2) {
        if kept.is_empty() {
            kept.push(pair[0]);
        }
        let length = pair[0].distance_to(pair[1]).value();
        if walked + length >= distance - 1e-6 {
            if distance - walked > 1e-6 {
                let bearing = pair[0].initial_bearing_to(pair[1]);
                kept.push(
                    pair[0].destination(bearing, crate::units::Meters::from_raw(distance - walked)),
                );
            }
            return kept;
        }
        walked += length;
        kept.push(pair[1]);
    }
    if kept.is_empty() {
        kept.extend(points.first());
    }
    kept
}

/// The passages of a route taxied as `old` up to `at` metres and as `new`
/// from there: a junction the two share at `at` is one passage.
fn splice_passages(old: &[Passage], new: &[Passage], at: f64) -> Vec<Passage> {
    let mut passages: Vec<Passage> = Vec::with_capacity(old.len() + new.len());
    passages.extend(old.iter().filter(|p| p.entry < at - 1e-6));
    for passage in new {
        match passages.last_mut() {
            Some(last) if last.junction == passage.junction => last.exit = passage.exit,
            _ => passages.push(*passage),
        }
    }
    passages
}

/// The runs of a route taxied as `old` up to `at` metres and as `new` from
/// there.
fn splice_runs(old: &[Run], new: &[Run], at: f64) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::with_capacity(old.len() + new.len());
    for run in old.iter().filter(|r| r.from < at - 1e-6) {
        runs.push(Run {
            to: run.to.min(at),
            ..*run
        });
    }
    for run in new {
        match runs.last_mut() {
            Some(last)
                if last.chain == run.chain
                    && last.forward == run.forward
                    && (last.to - run.from).abs() < 1e-6 =>
            {
                last.to = run.to;
            }
            _ => runs.push(*run),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    fn passage(junction: u32, entry: f64, exit: f64) -> Passage {
        Passage {
            junction,
            entry,
            exit,
            hold: None,
        }
    }

    fn run(chain: u32, from: f64, to: f64) -> Run {
        Run {
            chain,
            from,
            to,
            at: 0.0,
            forward: true,
        }
    }

    #[test]
    fn test_the_taxied_part_is_kept_to_the_node() -> Result<()> {
        let north = |m: f64| LatLon::new(50.0 + m / 111_195.0, 8.5);
        let points = [north(0.0)?, north(100.0)?, north(300.0)?];
        let kept = cut(&points, 200.0);
        assert_eq!(kept.len(), 3);
        assert!(kept[2].distance_to(north(200.0)?).value() < 0.5);
        assert_eq!(cut(&points, 100.0).len(), 2);
        assert_eq!(cut(&points, 500.0).len(), 3);
        Ok(())
    }

    #[test]
    fn test_a_new_way_joins_the_old_at_the_node() {
        // Junction 1 entered at 200 on the old way; the node at 230 is in
        // it, and the new way leaves it at 240 for junction 5.
        let old = [
            passage(0, 50.0, 60.0),
            passage(1, 200.0, 250.0),
            passage(2, 400.0, 410.0),
        ];
        let new = [passage(1, 230.0, 240.0), passage(5, 300.0, 310.0)];
        let spliced = splice_passages(&old, &new, 230.0);
        assert_eq!(
            spliced,
            [
                passage(0, 50.0, 60.0),
                passage(1, 200.0, 240.0),
                passage(5, 300.0, 310.0)
            ]
        );
        // Chain 3 runs on to the node at 230 and on along the new way;
        // chain 4 was the old way on.
        let runs = splice_runs(
            &[run(3, 100.0, 260.0), run(4, 260.0, 400.0)],
            &[run(3, 230.0, 280.0), run(6, 280.0, 300.0)],
            230.0,
        );
        assert_eq!(runs, [run(3, 100.0, 280.0), run(6, 280.0, 300.0)]);
    }
}
