//! Computer opponent — wraps VirtualPlayer with delayed garbage queues.
//!
//! The AI simulates a network opponent: same 32-frame garbage delay,
//! same interface. Zero special cases in calling code.

use std::collections::VecDeque;
use crate::virtual_player::{AIDifficulty, VirtualPlayer};
use super::opponent::Opponent;

/// Simulated network delay in game ticks.
const NETWORK_DELAY: i32 = 32;

#[derive(Debug, Clone)]
struct GarbagePacket {
    height: i32,
    flavor: i32,
    delivery_time: i32,
}

/// AI opponent with simulated network delay.
pub struct ComputerOpponent {
    ai: VirtualPlayer,
    /// Player → AI garbage (delayed by NETWORK_DELAY).
    incoming_queue: VecDeque<GarbagePacket>,
    /// AI → Player garbage (ready to deliver).
    outgoing_queue: VecDeque<(i32, i32, i32)>,
    current_time: i32,
    ai_loss_timestamp: Option<i32>,
    difficulty: AIDifficulty,
    /// Round ready signal — AI is always immediately ready.
    round_ready: u32,
}

impl ComputerOpponent {
    pub fn new(difficulty: AIDifficulty) -> Self {
        Self {
            ai: VirtualPlayer::new(difficulty),
            incoming_queue: VecDeque::new(),
            outgoing_queue: VecDeque::new(),
            current_time: 0,
            ai_loss_timestamp: None,
            difficulty,
            round_ready: 0,
        }
    }

    pub fn difficulty(&self) -> AIDifficulty {
        self.difficulty
    }
}

impl Opponent for ComputerOpponent {
    fn game_start(&mut self) {
        self.ai.reset();
        self.ai.reset_alarm(0);
        self.incoming_queue.clear();
        self.outgoing_queue.clear();
        self.current_time = 0;
        self.ai_loss_timestamp = None;
    }

    fn time_step(&mut self) {
        self.current_time += 1;

        // Deliver delayed incoming garbage from player
        while let Some(front) = self.incoming_queue.front() {
            if front.delivery_time <= self.current_time {
                let pkt = self.incoming_queue.pop_front().unwrap();
                self.ai.garbage_queue_mut().add(pkt.height, pkt.flavor);
            } else {
                // Packets are queued in delivery-time order, so the first future packet lets us stop scanning.
                break;
            }
        }

        // Check if AI's alarm has fired (time to produce garbage)
        if self.current_time >= self.ai.alarm() {
            let garbage = self.ai.garbage_amount();
            for (h, w, f) in garbage {
                self.outgoing_queue.push_back((h, w, f));
            }
            self.ai.reset_alarm(self.current_time);
        }

        // Check if AI has lost
        if self.ai_loss_timestamp.is_none() && self.ai.determine_loss() {
            self.ai_loss_timestamp = Some(self.current_time);
        }
    }

    fn send_garbage(&mut self, height: i32, width: i32, flavor: i32) {
        // The wire format uses (0, 0, flavor) to mean a full-width special row, one row high.
        // The AI queue only models heights, so the width is not kept.
        let h = if height == 0 && width == 0 { 1 } else { height };
        self.incoming_queue.push_back(GarbagePacket {
            height: h,
            flavor,
            delivery_time: self.current_time + NETWORK_DELAY,
        });
    }

    fn recv_garbage(&mut self) -> Vec<(i32, i32, i32)> {
        self.outgoing_queue.drain(..).collect()
    }

    fn grid_level(&mut self) -> i32 {
        self.ai.get_grid_level()
    }

    fn has_opponent_lost(&self) -> bool {
        self.ai_loss_timestamp.is_some()
    }

    fn opponent_loss_timestamp(&self) -> Option<i32> {
        self.ai_loss_timestamp
    }

    fn opponent_name(&self) -> &str {
        self.ai.difficulty().name()
    }

    fn set_local_round_ready(&mut self, round: u32) {
        // AI is always immediately ready for the requested round
        self.round_ready = round;
    }

    fn opponent_round_ready(&self) -> u32 {
        // AI is always ready — return whatever was set
        self.round_ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtual_player::GARBAGE_FLAVOR_NORMAL;

    #[test]
    fn garbage_delay() {
        let mut opp = ComputerOpponent::new(AIDifficulty::Medium);
        opp.send_garbage(1, 6, GARBAGE_FLAVOR_NORMAL);
        // Not delivered yet (need 32 ticks)
        opp.time_step();
        assert_eq!(opp.ai.garbage_queue_mut().height(), 0);

        // Fast-forward to delivery
        for _ in 0..NETWORK_DELAY {
            opp.time_step();
        }
        assert!(opp.ai.garbage_queue_mut().height() > 0);
    }

}
