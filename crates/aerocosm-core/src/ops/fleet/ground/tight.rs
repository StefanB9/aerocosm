//! Junctions too close together to wait between (plan M4 D93): an aircraft
//! standing short of a junction has its tail clear of the one before only if
//! there is room for it. The controller clears an aircraft into a junction
//! only if it can stop clear beyond it, so those it cannot stop short of
//! have to be free too.

use std::ops::Range;

use super::{AIRCRAFT_LENGTH, HOLD_SHORT, ZONE};
use crate::ground::junctions::Passage;

/// Room an aircraft needs to wait between two junctions, m: its tail out of
/// the first one's zone, its nose short of the second.
const ROOM: f64 = ZONE + AIRCRAFT_LENGTH + HOLD_SHORT;

/// The passages after passage `i` an aircraft cleared into it cannot stop
/// short of.
pub(super) fn committed(passages: &[Passage], i: usize) -> Range<usize> {
    let mut end = i + 1;
    while end < passages.len() && passages[end].entry - passages[end - 1].exit < ROOM {
        end += 1;
    }
    (i + 1).min(passages.len())..end.min(passages.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passage(entry: f64) -> Passage {
        Passage {
            junction: 0,
            entry,
            exit: entry + 20.0,
            hold: None,
        }
    }

    #[test]
    fn test_a_clearance_reaches_where_it_can_stop_clear() {
        // Junctions at 300, 400 (80 m on) and 500 (80 m on), then 700.
        let passages = [
            passage(300.0),
            passage(400.0),
            passage(500.0),
            passage(700.0),
        ];
        assert_eq!(committed(&passages, 0), 1..3);
        assert_eq!(committed(&passages, 2), 3..3);
        assert_eq!(committed(&passages, 3), 4..4);
    }
}
