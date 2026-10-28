//! Every behavioural knob of the grid AI: [`AiTuning`] (what the bot does), the difficulty
//! presets, and [`Adaptive`] (a survival tuning and an aggressive one blended by how threatened
//! the board is). Each knob was either kept because a measurement supported it or removed; the
//! journal (`docs/ia-grille-journal-des-tests.html`) holds the evidence.

use crate::ai_difficulty::AIDifficulty;

/// Every behavioural knob of the controller. The named difficulty tiers are presets over this
/// struct ([`AiTuning::for_difficulty`]). Only the strategic tiers read the "strategic" knobs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AiTuning {
    /// Ticks the bot pauses after each swap (reaction pacing: speed, not behaviour).
    pub cooldown: i32,
    /// Dig blocks into gaps when nothing better exists.
    pub flatten: bool,
    /// Look-ahead planning ([`super::controller::AiController`]'s strategic rules) instead of
    /// the reactive "clear what is one swap away".
    pub strategic: bool,
    /// Survival (clear anything) at this margin below the safe height.
    pub danger_margin: i32,
    /// Score weight per garbage cell a candidate's cascade shatters.
    pub shatter_weight: i32,
    /// Multi-swap garbage-shatter set-ups up to this many lateral swaps; 0 disables them.
    pub shatter_setup_max_cost: i32,
    /// When garbage rests on a tower no set-up can reach, dig its support out.
    pub undermine: bool,
    /// When safe with nothing to fire, prefer a bank move that enables a chain or combo one
    /// trigger swap later.
    pub chain_setup: bool,
    /// When no single chain enabler exists, search one level deeper.
    pub chain_lookahead: bool,
    /// Crossed-column x8/x10 combo construction while safe, up to this many lateral swaps; 0 off.
    pub big_combo_setup_max_cost: i32,
    /// A cascade at least this deep is worth firing (2 = any chain).
    pub fire_min_chain: i32,
    /// A single run at least this long is worth firing (width garbage).
    pub fire_min_run: i32,
    /// Keep playing while blocks wake up, garbage moves or a chain is still falling: take the
    /// clearing swaps away from the live chain instead of standing still.
    pub opportunism: bool,
    /// The same during a chain-linked fall.
    pub opportunism_linked: bool,
    /// Garbage cells on the board from which the bot counts as under pressure.
    pub pressure_garbage_cells: i32,
    /// Raise the board by hand, before banking, when fewer than this many blocks are on it
    /// (little material left); 0 off. Raising is also what the bot does when nothing is playable.
    pub advance_below_blocks: i32,
    /// Under pressure, allow shatter set-ups up to this cost (0 off).
    pub pressure_setups: i32,
    /// While blocks of a shattered slab are waking up (about 14 s), prepare what they will
    /// match with when they wake: swaps that, once the awaking blocks have woken (and fallen,
    /// if there is a gap below them), form a match, a chain when they belong to a combo.
    pub wake_prep: bool,
    /// When the board is in motion and no play is worth making, raise it by hand (as on a
    /// settled board) instead of doing nothing.
    pub raise_in_motion: bool,
    /// In danger with nothing to dig and the board frozen at the top (it cannot be raised), go on
    /// to the set-ups and clears below the dig instead of standing still until the end.
    pub act_when_frozen: bool,
    /// Refuse a swap that turns a row back into what it was before one of the last few swaps
    /// (the bot would be undoing its own work, back and forth).
    pub row_undo_guard: bool,
    /// When nothing else is worth playing, the longest sequence of swaps (2 or 3) searched for a
    /// plain clear, instead of raising or standing still; 0 off.
    pub clear_setup: i32,
    /// Also search that plain clear while the board is in motion and nothing else is worth
    /// playing (instead of waiting for the fall or the awakening to end).
    pub clear_in_motion: bool,
    /// A slight preference between close options: per grid row of the swap, an eighth of a
    /// cell of score. Positive favours the higher swap, negative the lower; 0 off.
    pub height_bias: i32,
    /// Hand tempo, speed and not behaviour: ticks of rest after each cursor step (a step is a
    /// press and a release, 2 ticks, so 0 walks 25 cells a second, which no hand does).
    pub move_gap: i32,
    /// Up to this many more ticks of rest after a step, drawn from the bot's seed so that
    /// the walk is uneven like a hand's (none for an unseeded bot).
    pub move_jitter: i32,
}

/// Lists the knobs once and derives from the list what must stay in step with it: the blend of
/// two tunings and the `name=value` experiment overrides. A knob missing from the list is a
/// compile error, so the three never drift apart.
macro_rules! knobs {
    (blended_numbers: [$($num:ident),*], blended_flags: [$($flag:ident),*], fixed: [$($fixed:ident),*]) => {
        impl AiTuning {
            /// The tuning at aggression `m` (0 = `survival`, 1 = `aggressive`): numbers are
            /// interpolated, switches follow the nearest side, the `fixed` knobs (pacing and the
            /// choice of algorithm) come from the survival side.
            fn blend(survival: &AiTuning, aggressive: &AiTuning, m: f64) -> AiTuning {
                let num = |x: i32, y: i32| (x as f64 + (y - x) as f64 * m).round() as i32;
                let flag = |x: bool, y: bool| if m >= 0.5 { y } else { x };
                AiTuning {
                    $($num: num(survival.$num, aggressive.$num),)*
                    $($flag: flag(survival.$flag, aggressive.$flag),)*
                    $($fixed: survival.$fixed,)*
                }
            }

            /// Overrides from `name=value` pairs separated by commas, for the experiments of
            /// `arena` (switches are 0 or 1). Unknown names are an error.
            #[cfg(test)]
            pub(crate) fn with_overrides(mut self, spec: &str) -> Result<Self, String> {
                for pair in spec.split(',').map(str::trim).filter(|p| !p.is_empty() && *p != "base") {
                    let (name, value) = pair.split_once('=').ok_or_else(|| format!("'{pair}': expected name=value"))?;
                    let v: i32 = value.trim().parse().map_err(|_| format!("'{pair}': not a number"))?;
                    match name.trim() {
                        // Pacing, not behaviour: not a blended knob, but sweepable for speed tests.
                        "cooldown" => self.cooldown = v,
                        "strategic" => self.strategic = v == 1,
                        $(stringify!($num) => self.$num = v,)*
                        $(stringify!($flag) => {
                            if v != 0 && v != 1 {
                                return Err(format!("'{pair}': a switch is 0 or 1"));
                            }
                            self.$flag = v == 1;
                        })*
                        other => return Err(format!("unknown parameter '{other}'")),
                    }
                }
                Ok(self)
            }
        }
    };
}

knobs! {
    blended_numbers: [
        danger_margin, shatter_weight, shatter_setup_max_cost, big_combo_setup_max_cost,
        fire_min_chain, fire_min_run, pressure_garbage_cells, advance_below_blocks, pressure_setups, height_bias,
        move_gap, move_jitter, clear_setup
    ],
    blended_flags: [
        flatten, undermine, chain_setup, chain_lookahead, opportunism, opportunism_linked, wake_prep, raise_in_motion, act_when_frozen, row_undo_guard, clear_in_motion
    ],
    fixed: [cooldown, strategic]
}

impl AiTuning {
    /// Difficulty is behavioural, not just paced: the tiers differ in strategy, not reflexes
    /// (the reference bot measured that reaction speed barely affects survival). The cooldown is
    /// kept for feel only: easy visibly calmer, hard snappier.
    ///   easy   - the strategic rules without any set-up (no chains, combos or shatters) except
    ///            under garbage pressure; slow hands.
    ///   medium - strategic-lite: fires the chains, combos and shatters it sees, clears to
    ///            survive in danger, undermines garbage towers; no shatter set-ups outside
    ///            pressure, one-level chain set-ups only.
    ///   hard   - full strategic: also shatter set-ups, chains and x8/x10 building.
    pub(crate) fn for_difficulty(difficulty: AIDifficulty) -> Self {
        let base = AiTuning {
            cooldown: 8,
            flatten: true,
            strategic: true,
            danger_margin: 3,
            shatter_weight: 3,
            shatter_setup_max_cost: 10,
            undermine: true,
            chain_setup: true,
            chain_lookahead: true,
            big_combo_setup_max_cost: 18,
            fire_min_chain: 2,
            fire_min_run: 4,
            opportunism: true,
            opportunism_linked: true,
            pressure_garbage_cells: 12,
            advance_below_blocks: 0,
            pressure_setups: 0,
            clear_setup: 0,
            clear_in_motion: false,
            wake_prep: true,
            raise_in_motion: false,
            act_when_frozen: true,
            row_undo_guard: true,
            height_bias: 0,
            // A step every 5 to 7 ticks: 7 to 10 cells a second, a quick player's taps.
            move_gap: 3,
            move_jitter: 2,
        };
        match difficulty {
            AIDifficulty::Easy => AiTuning {
                cooldown: 20,
                move_gap: 5,
                move_jitter: 3,
                wake_prep: false,
                opportunism: false,
                opportunism_linked: false,
                chain_setup: false,
                chain_lookahead: false,
                big_combo_setup_max_cost: 0,
                shatter_setup_max_cost: 0,
                // Only what it does under garbage pressure keeps it alive about three minutes
                // alone (journal S44).
                pressure_setups: 2,
                pressure_garbage_cells: 20,
                ..base
            },
            AIDifficulty::Medium => AiTuning {
                cooldown: 12,
                shatter_setup_max_cost: 0,
                // One-level chain set-ups and plain clears in a few swaps, no look-ahead
                // (journal S44: about 300 points alone).
                chain_setup: true,
                chain_lookahead: false,
                clear_setup: 3,
                big_combo_setup_max_cost: 0,
                wake_prep: false,
                // Without garbage on the board it does not build; under pressure it digs and sets
                // up shatters (cost 2) like the top tier. Cost 4 doubled its survival but brought it within
                // reach of the top tier (hard beat it 35 times in 60); 2 keeps the ladder (54 in 60).
                pressure_setups: 2,
                ..base
            },
            // Quick hands, tuned to about 600 points alone (journal S43, S44); the behaviour is
            // the measured one.
            AIDifficulty::Hard => AiTuning { clear_setup: 3, cooldown: 4, move_gap: 1, move_jitter: 1, height_bias: 2, ..base },
        }
    }
}

/// Survival/aggression adaptation. Two tunings and a mode `m` in [0, 1] (0 = survival,
/// 1 = aggressive) computed every decision from how threatened the board is:
/// `threat = stack row + garbage cells / garbage_div`, and `m` falls linearly from 1 at
/// `threat <= lo` to 0 at `threat >= hi`. The live tuning is the blend of the two.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Adaptive {
    pub survival: AiTuning,
    pub aggressive: AiTuning,
    pub lo: i32,
    pub hi: i32,
    pub garbage_div: i32,
}

impl Adaptive {
    /// No adaptation: both sides are `tuning`.
    pub(crate) fn fixed(tuning: AiTuning) -> Self {
        Self {
            survival: tuning,
            aggressive: tuning,
            lo: 6,
            hi: 11,
            garbage_div: 6,
        }
    }

    /// The shipped adaptation. Hard raises the board by hand when little material is left, but
    /// only while it is not threatened: below 45 blocks, between 8 and 12 rows of threat
    /// (journal S32: +18 points in duel; any raise on the survival side costs). The other
    /// tiers do not adapt.
    pub(crate) fn for_difficulty(difficulty: AIDifficulty) -> Self {
        let base = AiTuning::for_difficulty(difficulty);
        match difficulty {
            AIDifficulty::Hard => Self {
                aggressive: AiTuning { advance_below_blocks: 45, ..base },
                lo: 8,
                hi: 12,
                ..Self::fixed(base)
            },
            _ => Self::fixed(base),
        }
    }

    /// Whether the two sides differ at all (otherwise there is nothing to adapt).
    pub(super) fn adapts(&self) -> bool {
        self.survival != self.aggressive
    }

    /// Aggression for a threat level given in sixths of a row.
    pub(crate) fn mode(&self, threat_x6: i32) -> f64 {
        let threat = threat_x6 as f64 / 6.0;
        if self.hi <= self.lo {
            return if threat >= self.hi as f64 { 0.0 } else { 1.0 };
        }
        ((self.hi as f64 - threat) / (self.hi - self.lo) as f64).clamp(0.0, 1.0)
    }

    pub(crate) fn blend(&self, m: f64) -> AiTuning {
        AiTuning::blend(&self.survival, &self.aggressive, m)
    }

    /// Experiment variants: `name=value` overrides for both sides, `s.name=value` for the
    /// survival side only, `a.name=value` for the aggressive side only, and the function's own
    /// `mode_lo`, `mode_hi`, `mode_gdiv`.
    #[cfg(test)]
    pub(crate) fn from_spec(base: AiTuning, spec: &str) -> Result<Self, String> {
        let mut out = Self::fixed(base);
        let (mut both, mut surv, mut aggr) = (String::new(), String::new(), String::new());
        for pair in spec.split(',').map(str::trim).filter(|p| !p.is_empty() && *p != "base") {
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("'{pair}': expected name=value"))?;
            let v = || {
                value
                    .trim()
                    .parse::<i32>()
                    .map_err(|_| format!("'{pair}': not a number"))
            };
            let target = match name.trim() {
                "mode_lo" => {
                    out.lo = v()?;
                    continue;
                }
                "mode_hi" => {
                    out.hi = v()?;
                    continue;
                }
                "mode_gdiv" => {
                    out.garbage_div = v()?;
                    continue;
                }
                n if n.starts_with("s.") => (&mut surv, &n[2..]),
                n if n.starts_with("a.") => (&mut aggr, &n[2..]),
                n => (&mut both, n),
            };
            target.0.push_str(&format!("{}={},", target.1, value.trim()));
        }
        out.survival = base.with_overrides(&both)?.with_overrides(&surv)?;
        out.aggressive = base.with_overrides(&both)?.with_overrides(&aggr)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_reject_bad_switches_and_unknown_names() {
        let base = AiTuning::for_difficulty(AIDifficulty::Hard);
        assert!(!base.with_overrides("flatten=0").unwrap().flatten);
        assert!(base.with_overrides("flatten=2").is_err());
        assert!(base.with_overrides("nonsense=1").is_err());
        assert_eq!(base.with_overrides("danger_margin=5").unwrap().danger_margin, 5);
    }

    #[test]
    fn shipped_hard_is_the_measured_variant() {
        let base = AiTuning::for_difficulty(AIDifficulty::Hard);
        let shipped = Adaptive::for_difficulty(AIDifficulty::Hard);
        let measured = Adaptive::from_spec(base, "a.advance_below_blocks=45,mode_lo=8,mode_hi=12").unwrap();
        assert_eq!((shipped.survival, shipped.aggressive), (measured.survival, measured.aggressive));
        assert_eq!((shipped.lo, shipped.hi, shipped.garbage_div), (measured.lo, measured.hi, measured.garbage_div));
        assert!(!Adaptive::for_difficulty(AIDifficulty::Medium).adapts());
    }
}
