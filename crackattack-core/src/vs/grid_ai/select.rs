//! Choosing among equally good candidates the same way everywhere: the highest value wins; on a
//! tie the higher seeded rank (so two bots of different seeds differ, a given bot never does);
//! on a further tie the swap nearest the cursor. An unseeded bot has rank 0 for every swap, which
//! leaves nearest-cursor in scan order.

/// The best candidate offered so far.
pub(super) struct Best<V, T> {
    entry: Option<(V, u32, i32, T)>,
}

impl<V: Ord + Copy, T: Copy> Best<V, T> {
    pub(super) fn new() -> Self {
        Self { entry: None }
    }

    /// Offer a candidate: its `value` (higher is better), seeded `rank`, cursor distance `dist`.
    pub(super) fn offer(&mut self, value: V, rank: u32, dist: i32, item: T) {
        let better = match &self.entry {
            None => true,
            Some((v, r, d, _)) => value > *v || (value == *v && (rank > *r || (rank == *r && dist < *d))),
        };
        if better {
            self.entry = Some((value, rank, dist, item));
        }
    }

    pub(super) fn item(&self) -> Option<T> {
        self.entry.map(|(_, _, _, item)| item)
    }
}

#[cfg(test)]
mod tests {
    use super::Best;

    #[test]
    fn value_then_rank_then_distance() {
        let mut best = Best::new();
        best.offer(1, 9, 0, "low value");
        best.offer(2, 1, 5, "value wins");
        assert_eq!(best.item(), Some("value wins"));
        best.offer(2, 3, 9, "rank wins");
        assert_eq!(best.item(), Some("rank wins"));
        best.offer(2, 3, 4, "nearer wins");
        assert_eq!(best.item(), Some("nearer wins"));
        best.offer(2, 3, 4, "first of equals stays");
        assert_eq!(best.item(), Some("nearer wins"));
    }

    #[test]
    fn nothing_offered_is_none() {
        assert_eq!(Best::<i32, i32>::new().item(), None);
    }
}
