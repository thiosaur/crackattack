//! Auto QA — headless multi-seed runner with invariant checks.
//!
//! Runs Crack Attack in autoplay over many seeds with no GPU,
//! checking a set of game-state invariants on every tick. When
//! an invariant fails, dumps the last N frames of the offending
//! game into a per-incident directory (named from the run timestamp,
//! seed and tick) in the output folder, along with an
//! `incidents.json` describing the failure.
//!
//! Meant as the regression harness for game-logic bugs that only
//! appear in rare configurations (e.g. three same-flavor blocks
//! in a row that the core fails to match).
//!
//! Usage examples:
//!   cargo run -p auto_qa --release -- --seeds 100
//!   cargo run -p auto_qa --release -- --seeds 1,2,3 --max-ticks 20000
//!   cargo run -p auto_qa --release -- --seed-start 1 --seed-count 500

use crackattack_core::{
    AIController, GameState, Input, ScoringMode, BlockState, FrameData,
    constants::PLAY_WIDTH,
};

const PLAYFIELD_HEIGHT: i32 = 12;
const RING_SIZE: usize = 5;
/// An invariant violation must persist this many consecutive ticks
/// to be considered a real bug rather than a transient gameplay state
/// (e.g. a block in mid-flight whose `f_y` will be 0 next tick, or a
/// match that the core resolves on the very next tick).
const PERSIST_TICKS_THRESHOLD: u32 = 5;
/// After an incident is reported, continue capturing this many frames
/// to see whether the bug self-resolves or keeps festering.
const POST_INCIDENT_CAPTURE: u32 = 10;

/// Top-level CLI arguments.
struct CliArgs {
    /// Explicit list of seeds to run (overrides `seed_start`/`seed_count` if set).
    seeds: Option<Vec<u32>>,
    /// Starting seed for the auto-generated range.
    seed_start: u32,
    /// Number of seeds in the auto-generated range.
    seed_count: u32,
    /// Hard ticks cap per game (avoids runaway).
    max_ticks: i32,
    /// If true, stop after the first incident (otherwise keep running).
    stop_on_first: bool,
    /// Output directory for reports.
    output_dir: String,
    /// Verbosity: 0=silent, 1=summary, 2=per-game, 3=per-tick.
    verbose: u8,
}

impl Default for CliArgs {
    fn default() -> Self {
        Self {
            seeds: None,
            seed_start: 1,
            seed_count: 100,
            max_ticks: 30_000,
            stop_on_first: false,
            output_dir: "qa_reports".to_string(),
            verbose: 1,
        }
    }
}

/// Per-tick incident record. Written to `incidents.json` of the
/// reporting directory.
#[derive(Debug, Clone)]
struct Incident {
    category: &'static str,
}

/// One detected violation. The `signature` is a stable key that lets
/// the runner group repeats of the same issue across ticks (so we can
/// require the issue to *persist* before promoting it to an incident,
/// filtering out transient one-tick states that the core resolves at
/// the next step).
#[derive(Debug, Clone)]
struct Violation {
    category: &'static str,
    signature: String,
    message: String,
}

/// Catalogue of invariant checks. Each function takes a fully-stepped
/// `&GameState` (i.e. the state right after `step()` returned) and
/// returns a list of violations if the invariant is broken.
mod invariants {
    use super::*;

    /// Invariant 1 — no undetected match.
    /// Scans the playfield for 3+ adjacent same-flavor STATIC blocks
    /// that are NOT in DYING / FALLING / SWAPPING / AWAKING state.
    /// Such an alignment should have been resolved by the core within
    /// this tick; if it's still here, the match-detection has missed it.
    pub fn no_undetected_match(game: &GameState) -> Vec<Violation> {
        let mut incidents = Vec::new();
        // Eligible blocks: in the grid, STATIC bit set, no other
        // animation bits (DYING/FALLING/SWAPPING/AWAKING). The block's
        // own state is looked up by id rather than the cell phase, so
        // transient grid bookkeeping cannot mislead the check.
        let is_eligible = |x: i32, y: i32| -> Option<i32> {
            if !game.grid.in_bounds(x, y) {
                return None;
            }
            let block_id = game.grid.block_id_at(x, y)?;
            let block = game.blocks.get(block_id)?;
            if !block.physics_flags.contains(BlockState::STATIC) {
                return None;
            }
            // A block in one of these animations may legitimately sit
            // in a 3-alignment that is not matched yet.
            let blocking = BlockState::FALLING
                | BlockState::SWAPPING
                | BlockState::DYING
                | BlockState::AWAKING;
            if block.physics_flags.intersects(blocking) {
                return None;
            }
            // The core marks a block STATIC|FALLING during the hang
            // phase before it really falls. Exclude it too.
            if block.fall_delay_countdown > 0 {
                return None;
            }
            // Sub-grid offset: block is mid-step.
            if block.sub_grid_y != 0 {
                return None;
            }
            Some(block.flavor)
        };

        // The scan starts at y=1. Row y=0 is the creep row (used by the
        // stack-rise mechanic), which the core's match detection also
        // excludes.
        // Horizontal scan.
        for y in 1..PLAYFIELD_HEIGHT {
            for x in 0..(PLAY_WIDTH as i32 - 2) {
                if let (Some(f0), Some(f1), Some(f2)) =
                    (is_eligible(x, y), is_eligible(x + 1, y), is_eligible(x + 2, y))
                {
                    if f0 == f1 && f1 == f2 {
                        incidents.push(Violation {
                            category: "undetected_match",
                            signature: format!("h_y{}_x{}_f{}", y, x, f0),
                            message: format!(
                                "horizontal triple flavor {} at row y={}, cols x={}..={}",
                                f0, y, x, x + 2
                            ),
                        });
                    }
                }
            }
        }
        // Vertical scan. It starts at y=1 too, so a triple at rows
        // (1, 2, 3) is checked but (0, 1, 2), which includes the
        // creep row, is not.
        for x in 0..PLAY_WIDTH as i32 {
            for y in 1..(PLAYFIELD_HEIGHT - 2) {
                if let (Some(f0), Some(f1), Some(f2)) =
                    (is_eligible(x, y), is_eligible(x, y + 1), is_eligible(x, y + 2))
                {
                    if f0 == f1 && f1 == f2 {
                        incidents.push(Violation {
                            category: "undetected_match",
                            signature: format!("v_x{}_y{}_f{}", x, y, f0),
                            message: format!(
                                "vertical triple flavor {} at col x={}, rows y={}..={}",
                                f0, x, y, y + 2
                            ),
                        });
                    }
                }
            }
        }
        incidents
    }

    /// Invariant 2 — no orphan floating block.
    /// A block in STATIC state should have either another solid cell
    /// (block or garbage) directly below it, or be at y==0 (the floor
    /// row). Otherwise it's "floating in mid-air" with no support and
    /// should have been marked FALLING.
    pub fn no_floating_static(game: &GameState) -> Vec<Violation> {
        let mut incidents = Vec::new();
        for y in 1..PLAYFIELD_HEIGHT {
            for x in 0..PLAY_WIDTH as i32 {
                let Some(block_id) = game.grid.block_id_at(x, y) else { continue };
                let Some(block) = game.blocks.get(block_id) else { continue };
                if !block.physics_flags.contains(BlockState::STATIC) {
                    continue;
                }
                if block.physics_flags.intersects(
                    BlockState::FALLING | BlockState::SWAPPING
                        | BlockState::DYING | BlockState::AWAKING,
                ) {
                    continue;
                }
                if block.fall_delay_countdown > 0 {
                    continue;
                }
                let below = game.grid.get(x, y - 1);
                if !below.is_empty() {
                    continue;
                }
                // Transient filter. A block that is falling, swapping,
                // dying or awaking next to the empty cell below can leave
                // it empty for one tick before gravity propagates up.
                // If any block within one column of x at y-1 is in one of
                // those states, treat the case as transient.
                let mut is_transient = false;
                for dx in -1..=1 {
                    let nx = x + dx;
                    if !game.grid.in_bounds(nx, y - 1) { continue; }
                    if let Some(nbid) = game.grid.block_id_at(nx, y - 1) {
                        if let Some(nb) = game.blocks.get(nbid) {
                            if nb.physics_flags.intersects(
                                BlockState::FALLING | BlockState::SWAPPING
                                    | BlockState::DYING | BlockState::AWAKING,
                            ) {
                                is_transient = true;
                                break;
                            }
                        }
                    }
                }
                if is_transient {
                    continue;
                }
                incidents.push(Violation {
                    category: "floating_static",
                    signature: format!("float_x{}_y{}", x, y),
                    message: format!(
                        "floating block flavor {} at ({}, {}) — cell below is empty",
                        block.flavor, x, y
                    ),
                });
            }
        }
        incidents
    }

    /// Invariant 3 — no score regression.
    /// Score is monotonically non-decreasing during a single game.
    /// Invariant 4 — no block stuck in DYING with stale alarm.
    /// The DYING animation lasts `DYING_DELAY` ticks. If a block is
    /// still DYING after its `animation_countdown` has gone well
    /// negative, the core's cleanup missed it and the block leaks
    /// (a frozen shrinking block on screen).
    pub fn no_dying_stuck(game: &GameState) -> Vec<Violation> {
        let mut incidents = Vec::new();
        for block in game.blocks.iter_active() {
            if !block.physics_flags.contains(BlockState::DYING) {
                continue;
            }
            // More than 15 ticks past zero means the block is stuck.
            if block.animation_countdown < -15 {
                incidents.push(Violation {
                    category: "dying_stuck",
                    signature: format!("dying_stuck_{}", block.id.0),
                    message: format!(
                        "block id {} at ({}, {}) flavor {} stuck DYING with animation_countdown={}",
                        block.id.0, block.grid_x, block.grid_y, block.flavor, block.animation_countdown
                    ),
                });
            }
        }
        incidents
    }

    /// Invariant 5 — grid and block arena in sync.
    /// Every active block in the arena should be at the same
    /// position the grid references for its id. A desync means
    /// one of the two structures has a stale entry — the kind
    /// of bug that makes a block appear in two cells at once
    /// or disappear altogether.
    pub fn grid_arena_sync(game: &GameState) -> Vec<Violation> {
        let mut incidents = Vec::new();
        for block in game.blocks.iter_active() {
            // The block claims to be at (x, y). The grid cell at that
            // position must reference this block's id.
            if !game.grid.in_bounds(block.grid_x, block.grid_y) {
                // A block outside the grid is always a bug.
                incidents.push(Violation {
                    category: "grid_arena_desync",
                    signature: format!("oob_block_{}", block.id.0),
                    message: format!(
                        "block id {} at ({}, {}) flavor {} is out of grid bounds",
                        block.id.0, block.grid_x, block.grid_y, block.flavor
                    ),
                });
                continue;
            }
            match game.grid.block_id_at(block.grid_x, block.grid_y) {
                Some(gid) if gid == block.id => { /* OK */ }
                _ => {
                    // The block thinks it is at (x, y) but the grid
                    // disagrees. Skip blocks in transitional animation
                    // states: the grid can lag by a tick during these.
                    if block.physics_flags.intersects(
                        BlockState::FALLING | BlockState::SWAPPING
                            | BlockState::DYING | BlockState::AWAKING,
                    ) {
                        continue;
                    }
                    incidents.push(Violation {
                        category: "grid_arena_desync",
                        signature: format!("desync_block_{}", block.id.0),
                        message: format!(
                            "block id {} claims ({}, {}) flavor {} but grid disagrees",
                            block.id.0, block.grid_x, block.grid_y, block.flavor
                        ),
                    });
                }
            }
        }
        incidents
    }

    pub fn no_score_regression(
        game: &GameState,
        prev_score: i32,
    ) -> Vec<Violation> {
        let mut incidents = Vec::new();
        let current = game.score.value();
        if current < prev_score {
            incidents.push(Violation {
                category: "score_regression",
                signature: format!("score_regress_{}_{}", prev_score, current),
                message: format!(
                    "score regression: {} → {} (delta {})",
                    prev_score, current, current - prev_score
                ),
            });
        }
        incidents
    }
}

/// Bounded ring buffer keeping the last `RING_SIZE` items.
struct RingBuffer<T> {
    data: std::collections::VecDeque<T>,
}

impl<T> RingBuffer<T> {
    fn new() -> Self {
        Self { data: std::collections::VecDeque::with_capacity(RING_SIZE) }
    }
    fn push(&mut self, item: T) {
        if self.data.len() == RING_SIZE {
            self.data.pop_front();
        }
        self.data.push_back(item);
    }
    fn iter(&self) -> impl Iterator<Item = &T> {
        self.data.iter()
    }
}

/// Serialize a FrameData snapshot to a compact JSON line.
fn frame_to_json(seed: u32, tick: i32, frame: &FrameData) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "{{\"seed\":{},\"tick\":{},\"play_offset_y\":{:.4},\"level\":{},\"phase\":\"{:?}\",",
        seed, tick, frame.play_offset_y,
        frame.ui.level, frame.ui.phase,
    ));
    s.push_str("\"blocks\":[");
    for (i, b) in frame.blocks.iter().enumerate() {
        if i > 0 { s.push(','); }
        s.push_str(&format!(
            "{{\"id\":{},\"x\":{},\"y\":{},\"f_y\":{},\"flavor\":{},\"state\":{},\"animation_countdown\":{},\"pop_countdown\":{}}}",
            b.id, b.grid_x, b.grid_y, b.sub_grid_y, b.flavor, b.state, b.animation_countdown, b.pop_countdown
        ));
    }
    s.push_str("],\"garbage\":[");
    for (i, g) in frame.garbage.iter().enumerate() {
        if i > 0 { s.push(','); }
        s.push_str(&format!(
            "{{\"id\":{},\"x\":{},\"y\":{},\"width\":{},\"height\":{},\"flavor\":{},\"state\":{},\"shatter_progress\":{:.3}}}",
            g.id, g.grid_x, g.grid_y, g.width, g.height, g.flavor, g.state, g.shatter_progress
        ));
    }
    s.push_str("]}");
    s
}

fn parse_args(argv: &[String]) -> CliArgs {
    let mut cli = CliArgs::default();
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--seeds" => {
                i += 1;
                if let Some(arg) = argv.get(i) {
                    if let Ok(n) = arg.parse::<u32>() {
                        cli.seed_count = n;
                    } else if arg.contains(',') {
                        cli.seeds = Some(
                            arg.split(',')
                                .filter_map(|s| s.parse::<u32>().ok())
                                .collect(),
                        );
                    } else {
                        eprintln!("--seeds expects a count or a comma list");
                        std::process::exit(2);
                    }
                }
            }
            "--seed-start" => {
                i += 1;
                cli.seed_start = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(1);
            }
            "--seed-count" => {
                i += 1;
                cli.seed_count = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(100);
            }
            "--max-ticks" => {
                i += 1;
                cli.max_ticks = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(30_000);
            }
            "--stop-on-first" => cli.stop_on_first = true,
            "--output" | "-o" => {
                i += 1;
                cli.output_dir = argv.get(i).cloned().unwrap_or("qa_reports".to_string());
            }
            "-v" => cli.verbose = 2,
            "-vv" => cli.verbose = 3,
            "-q" => cli.verbose = 0,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {}", other);
                print_help();
                std::process::exit(2);
            }
        }
        i += 1;
    }
    cli
}

fn print_help() {
    println!("auto_qa — Crack Attack invariant runner");
    println!();
    println!("USAGE:");
    println!("  auto_qa [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("  --seeds <N|list>       Either a count (auto-range) or comma list (e.g. 1,2,3)");
    println!("  --seed-start <N>       Starting seed for the auto-generated range (default 1)");
    println!("  --seed-count <N>       Count for the auto-generated range (default 100)");
    println!("  --max-ticks <N>        Hard tick cap per game (default 30000)");
    println!("  --stop-on-first        Stop after the first incident");
    println!("  -o, --output <dir>     Output directory for reports (default qa_reports)");
    println!("  -v / -vv / -q          Verbosity");
    println!("  -h, --help             Print this help");
}

/// Run a single seed and return the list of incidents found.
fn run_seed(seed: u32, max_ticks: i32, verbose: u8, output_dir: &str, run_id: &str) -> Vec<Incident> {
    let mut game = GameState::with_scoring(seed, ScoringMode::Classic);
    game.ranking_entry_score = Some(10_000);
    let mut ai = AIController::new();
    let mut input = Input::new();
    let mut incidents: Vec<Incident> = Vec::new();
    let mut ring: RingBuffer<String> = RingBuffer::new();
    let mut prev_score = 0_i32;
    // Consecutive ticks each violation signature has been seen. A
    // violation becomes an incident once the count hits
    // PERSIST_TICKS_THRESHOLD; the entry is dropped when the
    // signature stops appearing.
    let mut persistence: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();
    // Signatures already reported in this game, so a long-running
    // issue is reported only once.
    let mut reported: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    // Post-incident capture queue. After an incident, frames keep
    // being written to the same directory for POST_INCIDENT_CAPTURE
    // ticks, to show whether the bug resolves by itself. Each entry
    // is (incident_dir, remaining_frames, frame_index_offset).
    let mut post_captures: Vec<(String, u32, usize)> = Vec::new();

    for _tick in 0..max_ticks {
        let ai_input = ai.update(&game);
        for flag in [
            crackattack_core::INPUT_LEFT,
            crackattack_core::INPUT_RIGHT,
            crackattack_core::INPUT_UP,
            crackattack_core::INPUT_DOWN,
            crackattack_core::INPUT_SWAP,
            crackattack_core::INPUT_ADVANCE,
        ] {
            input.set_key(flag, ai_input & flag != 0);
        }
        let snap = input.snapshot();
        let frame = game.step(&snap);
        input.end_frame();

        ring.push(frame_to_json(seed, game.time_step, &frame));

        let mut all_violations: Vec<Violation> = Vec::new();
        all_violations.extend(invariants::no_undetected_match(&game));
        all_violations.extend(invariants::no_floating_static(&game));
        all_violations.extend(invariants::no_dying_stuck(&game));
        all_violations.extend(invariants::grid_arena_sync(&game));
        all_violations.extend(invariants::no_score_regression(&game, prev_score));
        prev_score = game.score.value();

        // Count the signatures seen now; drop the ones that vanished.
        let seen_now: std::collections::HashSet<String> = all_violations
            .iter()
            .map(|v| v.signature.clone())
            .collect();
        persistence.retain(|sig, _| seen_now.contains(sig));
        for sig in &seen_now {
            *persistence.entry(sig.clone()).or_insert(0) += 1;
        }

        // Promote only violations that persisted long enough and were
        // not reported yet in this game.
        let mut promoted: Vec<&Violation> = Vec::new();
        for v in &all_violations {
            if let Some(&count) = persistence.get(&v.signature) {
                if count == PERSIST_TICKS_THRESHOLD && !reported.contains(&v.signature) {
                    reported.insert(v.signature.clone());
                    promoted.push(v);
                }
            }
        }

        if !promoted.is_empty() {
            let incident_dir = format!(
                "{}/{}/seed_{}_tick_{}",
                output_dir, run_id, seed, game.time_step
            );
            if let Err(e) = std::fs::create_dir_all(&incident_dir) {
                eprintln!("[seed {}] failed to create dir {}: {}", seed, incident_dir, e);
                continue;
            }
            for (i, snapshot) in ring.iter().enumerate() {
                let path = format!("{}/frame_{}.json", incident_dir, i);
                let _ = std::fs::write(&path, snapshot);
            }
            let mut s = String::new();
            s.push_str("[\n");
            for (i, v) in promoted.iter().enumerate() {
                let sep = if i + 1 == promoted.len() { "" } else { "," };
                s.push_str(&format!(
                    "  {{\"category\":\"{}\",\"signature\":{:?},\"message\":{:?},\"persisted_ticks\":{}}}{}\n",
                    v.category, v.signature, v.message, PERSIST_TICKS_THRESHOLD, sep
                ));
            }
            s.push_str("]\n");
            let _ = std::fs::write(format!("{}/incidents.json", incident_dir), s);

            for v in &promoted {
                if verbose >= 1 {
                    eprintln!("[seed {} tick {}] {}: {} (persisted {} ticks)",
                        seed, game.time_step, v.category, v.message, PERSIST_TICKS_THRESHOLD);
                }
                incidents.push(Incident { category: v.category });
            }
            // Schedule post-incident frame captures. Ring buffer
            // produced frames 0..RING_SIZE-1; post-capture writes
            // frames RING_SIZE.. onward so the file names reflect
            // chronology. The remaining counter decrements each
            // tick until the capture window is exhausted.
            post_captures.push((
                incident_dir.clone(),
                POST_INCIDENT_CAPTURE,
                RING_SIZE,
            ));
        }

        // Write post-incident frames for any active capture windows.
        let current_snapshot = frame_to_json(seed, game.time_step, &frame);
        post_captures.retain_mut(|(dir, remaining, offset)| {
            if *remaining == 0 {
                return false;
            }
            let path = format!("{}/frame_{}.json", dir, *offset);
            let _ = std::fs::write(&path, &current_snapshot);
            *offset += 1;
            *remaining -= 1;
            *remaining > 0
        });

        // Stop once the game is finished.
        use crackattack_core::GamePhase;
        if frame.ui.phase == GamePhase::Finished {
            if verbose >= 2 {
                eprintln!("[seed {}] game finished at tick {}", seed, game.time_step);
            }
            break;
        }
    }
    incidents
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let cli = parse_args(&argv);

    let run_id = chrono_like_timestamp();
    if cli.verbose >= 1 {
        eprintln!("auto_qa run id: {}", run_id);
        eprintln!("output: {}/{}", cli.output_dir, run_id);
    }

    let seed_list: Vec<u32> = match &cli.seeds {
        Some(list) => list.clone(),
        None => (cli.seed_start..cli.seed_start + cli.seed_count).collect(),
    };

    let mut all_incidents: Vec<Incident> = Vec::new();
    let mut games_ok = 0;
    let mut games_with_incidents = 0;
    let start = std::time::Instant::now();

    for &seed in &seed_list {
        if cli.verbose >= 2 {
            eprintln!("--- seed {} ---", seed);
        }
        let incidents = run_seed(seed, cli.max_ticks, cli.verbose, &cli.output_dir, &run_id);
        if incidents.is_empty() {
            games_ok += 1;
        } else {
            games_with_incidents += 1;
            all_incidents.extend(incidents);
            if cli.stop_on_first {
                break;
            }
        }
    }

    let elapsed = start.elapsed();
    let summary_dir = format!("{}/{}", cli.output_dir, run_id);
    let _ = std::fs::create_dir_all(&summary_dir);
    let mut summary = String::new();
    summary.push_str(&format!(
        "{{\n  \"run_id\":\"{}\",\n  \"seed_count\":{},\n  \"games_ok\":{},\n  \"games_with_incidents\":{},\n  \"total_incidents\":{},\n  \"elapsed_seconds\":{:.2},\n",
        run_id, seed_list.len(), games_ok, games_with_incidents, all_incidents.len(),
        elapsed.as_secs_f32(),
    ));
    // Group by category for a quick at-a-glance summary.
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for inc in &all_incidents {
        *counts.entry(inc.category).or_insert(0) += 1;
    }
    summary.push_str("  \"by_category\":{\n");
    let entries: Vec<_> = counts.iter().collect();
    for (i, (cat, n)) in entries.iter().enumerate() {
        let sep = if i + 1 == entries.len() { "" } else { "," };
        summary.push_str(&format!("    \"{}\":{}{}\n", cat, n, sep));
    }
    summary.push_str("  }\n}\n");
    let _ = std::fs::write(format!("{}/summary.json", summary_dir), &summary);

    eprintln!();
    eprintln!("=== auto_qa summary ===");
    eprintln!("seeds tested:       {}", seed_list.len());
    eprintln!("games OK:           {}", games_ok);
    eprintln!("games w/ incidents: {}", games_with_incidents);
    eprintln!("total incidents:    {}", all_incidents.len());
    eprintln!("elapsed:            {:.2} s", elapsed.as_secs_f32());
    eprintln!("report dir:         {}", summary_dir);

    if !all_incidents.is_empty() {
        std::process::exit(1);
    }
}

/// Cheap timestamp without pulling in chrono.
fn chrono_like_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("run_{}", secs)
}
