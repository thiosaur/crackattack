use super::*;
use crate::input::{InputSnapshot, INPUT_UP, INPUT_SWAP};
use crate::fingerprint::Fingerprint;

/// Determinism: fingerprint is stable after 600 ticks with seed=42
#[test]
fn test_determinism_600_ticks() {
    let mut game = GameState::new(42);
    let empty_input = InputSnapshot::new();
    for _ in 0..600 {
        game.step(&empty_input);
    }
    assert_eq!(game.status, GameStatus::Playing);
    let fp = Fingerprint::from_state(&game);

    // Replay with identical inputs
    let mut game2 = GameState::new(42);
    for _ in 0..600 {
        game2.step(&empty_input);
    }
    let fp2 = Fingerprint::from_state(&game2);

    assert_eq!(fp.combined, fp2.combined, "Fingerprint should be deterministic");
    assert_eq!(fp.hash_grid, fp2.hash_grid);
    assert_eq!(fp.hash_blocks, fp2.hash_blocks);
    assert_eq!(fp.hash_swapper, fp2.hash_swapper);
    assert_eq!(fp.hash_score, fp2.hash_score);
    assert_eq!(fp.rng_state, fp2.rng_state);
}

/// Swapper bounds: cursor never exceeds y=12 (SAFE_HEIGHT - 1)
#[test]
fn test_swapper_y_bound() {
    let mut game = GameState::new(1);
    let empty = InputSnapshot::new();
    // Initialize game state
    for _ in 0..10 {
        game.step(&empty);
    }
    // Press UP repeatedly, releasing between presses: a held button only moves once, so
    // the bound would never be reached (the original version of this test proved nothing).
    for i in 0..40 {
        let mut input = InputSnapshot::new();
        if i % 2 == 0 {
            input.state = INPUT_UP;
            input.pressed_this_frame = INPUT_UP;
        }
        game.step(&input);
        assert!(game.swapper.y <= 12, "swapper.y={} exceeds 12", game.swapper.y);
    }
    // The bound must actually have been reached.
    assert_eq!(game.swapper.y, 12);
}

/// Swap position: saved coordinates used during swap
#[test]
fn test_swap_uses_saved_position() {
    let mut game = GameState::new(42);
    let empty = InputSnapshot::new();
    // Let grid reach stable state
    for _ in 0..50 {
        game.step(&empty);
    }
    // Record swap coordinates
    let swap_x = game.swapper.x;
    let swap_y = game.swapper.y;

    // Initiate a swap
    let mut swap_input = InputSnapshot::new();
    swap_input.state = INPUT_SWAP;
    swap_input.pressed_this_frame = INPUT_SWAP;
    game.step(&swap_input);

    // Verify swap coordinates are recorded
    assert_eq!(
        game.swapper.state,
        crate::swapper::SwapperState::Swapping,
        "the swap must start, otherwise the coordinates below are not checked"
    );
    assert_eq!(game.swapper.swap_x, swap_x);
    assert_eq!(game.swapper.swap_y, swap_y);
}

/// Garbage retention: queued garbage is not lost
#[test]
fn test_garbage_retry_on_failure() {
    let mut game = GameState::new(42);
    let empty = InputSnapshot::new();
    // Add garbage to queue
    game.garbage_generator.add_to_queue(2, 6, 0, game.time_step, None);
    let initial_count = game.garbage_generator.waiting_count();
    assert!(initial_count > 0);
    // Advance enough for garbage to attempt drop
    for _ in 0..400 {
        game.step(&empty);
    }
    // Garbage must either have been created or still be in queue
    let garbage_count = game.garbage.iter_active().count();
    let queue_count = game.garbage_generator.waiting_count();
    // Garbage must either have been created or still be in queue
    assert!(
        garbage_count > 0 || queue_count > 0 || game.status != GameStatus::Playing,
        "Garbage lost: arena={}, queue={}",
        garbage_count,
        queue_count
    );
}

/// Game over: creep eventually causes loss
#[test]
fn test_game_over_eventually() {
    let mut game = GameState::new(42);
    let empty = InputSnapshot::new();
    // Play without input — creep must cause game over
    for _ in 0..50000 {
        game.step(&empty);
        if game.status == GameStatus::Lost {
            return; // OK
        }
    }
    panic!("Game should have ended in loss after 50000 ticks");
}

/// Different seeds produce different states
#[test]
fn test_different_seeds_different_states() {
    let mut game1 = GameState::new(42);
    let mut game2 = GameState::new(99);
    let empty = InputSnapshot::new();
    for _ in 0..100 {
        game1.step(&empty);
        game2.step(&empty);
    }
    // Fingerprints must differ
    let fp1 = Fingerprint::from_state(&game1);
    let fp2 = Fingerprint::from_state(&game2);
    assert_ne!(
        fp1.combined, fp2.combined,
        "Two different seeds produced identical fingerprints"
    );
}

// ---- Helpers for constructed-grid tests ----

/// Clear entire grid and deallocate all blocks.
fn clear_playfield(game: &mut GameState) {
    // Deallocate all active blocks
    let ids: Vec<_> = game.blocks.iter_active().map(|b| b.id).collect();
    for id in ids {
        game.blocks.deallocate(id);
    }
    // Clear all grid cells
    for x in 0..crate::constants::PLAY_WIDTH as i32 {
        for y in 0..crate::constants::PLAY_HEIGHT as i32 {
            game.grid.clear(x, y);
        }
    }
}

/// Place a static block at (x, y) with the given flavor, registering
/// it in both the block arena and the grid.
fn place_block(game: &mut GameState, x: i32, y: i32, flavor: i32) {
    let id = game.blocks.allocate(x, y, flavor)
        .expect("block pool exhausted in test");
    game.grid.set(x, y, crate::grid::CellRef::block(id, crate::grid::BlockPhase::Static));
}

// ---- Constructed-grid gameplay tests ----

/// Prepare a clean game state for constructed-grid tests.
/// Clears all blocks, fills support rows, resets creep.
fn setup_clean_grid(game: &mut GameState) {
    clear_playfield(game);
    // Row 0 must be immutable (creep preparation row) to prevent
    // gravity from pulling blocks below y=1.
    for x in 0..crate::constants::PLAY_WIDTH as i32 {
        game.grid.set_empty_immutable(x, 0);
    }
    // Fill support row at y=1 with alternating flavors (no 3-in-a-row).
    for x in 0..crate::constants::PLAY_WIDTH as i32 {
        let flavor = 3 + (x % 3); // 3,4,5,3,4,5 — never matches test flavors 0,1,2
        place_block(game, x, 1, flavor);
    }
    // Reset creep and clear pending elimination requests from init.
    game.creep = crate::creep::Creep::new();
    game.elimination_requests.clear();
}

/// Three horizontal blocks of the same flavor trigger elimination.
#[test]
fn test_horizontal_match_triggers_dying() {
    let mut game = GameState::new(0);
    let input = InputSnapshot::new();

    setup_clean_grid(&mut game);

    let flavor = 0; // purple
    place_block(&mut game, 0, 2, flavor);
    place_block(&mut game, 1, 2, flavor);
    place_block(&mut game, 2, 2, flavor);

    game.elimination_requests.push((0, 2, None));

    let frame = game.step(&input);

    let dying_count = frame.blocks.iter()
        .filter(|b| b.state & crate::block::BlockState::DYING.bits() != 0)
        .count();
    assert_eq!(dying_count, 3, "Three matching blocks should all be DYING");
}

/// Three vertical blocks of the same flavor trigger elimination.
#[test]
fn test_vertical_match_triggers_dying() {
    let mut game = GameState::new(0);
    let input = InputSnapshot::new();

    setup_clean_grid(&mut game);

    let flavor = 1; // blue
    place_block(&mut game, 2, 2, flavor);
    place_block(&mut game, 2, 3, flavor);
    place_block(&mut game, 2, 4, flavor);

    game.elimination_requests.push((2, 2, None));

    let frame = game.step(&input);

    let dying_count = frame.blocks.iter()
        .filter(|b| b.state & crate::block::BlockState::DYING.bits() != 0)
        .count();
    assert_eq!(dying_count, 3, "Three vertical blocks should all be DYING");
}

/// Two non-matching blocks do NOT trigger elimination.
#[test]
fn test_two_blocks_no_elimination() {
    let mut game = GameState::new(0);
    let input = InputSnapshot::new();

    setup_clean_grid(&mut game);

    place_block(&mut game, 0, 2, 0); // purple
    place_block(&mut game, 1, 2, 0); // purple (only 2, not 3)

    game.elimination_requests.push((0, 2, None));
    game.elimination_requests.push((1, 2, None));

    let frame = game.step(&input);

    let dying_count = frame.blocks.iter()
        .filter(|b| b.state & crate::block::BlockState::DYING.bits() != 0)
        .count();
    assert_eq!(dying_count, 0, "Two matching blocks should not trigger elimination");
}

/// Elimination produces a score increase.
#[test]
fn test_elimination_produces_score() {
    let mut game = GameState::new(0);
    let input = InputSnapshot::new();

    setup_clean_grid(&mut game);

    let flavor = 2; // green
    place_block(&mut game, 0, 2, flavor);
    place_block(&mut game, 1, 2, flavor);
    place_block(&mut game, 2, 2, flavor);

    game.elimination_requests.push((0, 2, None));

    let score_before = game.score.value();

    for _ in 0..120 {
        game.step(&input);
    }

    assert!(game.score.value() > score_before,
        "Score should increase after a 3-match elimination");
}

/// Chain: eliminating blocks causes blocks above to fall and form a new match.
#[test]
fn test_chain_from_falling_blocks() {
    let mut game = GameState::new(0);
    let input = InputSnapshot::new();

    setup_clean_grid(&mut game);

    // Setup: row y=2 has 3 red blocks (will match and die).
    // Row y=3 has 2 blue blocks at x=0,1 and a third blue at x=2
    // is blocked by a red block at (2,2). When the red row dies,
    // the blue block at (2,3) falls to y=2, forming a 3-blue match.
    //
    //  y=3:  [B] [B] [B]  .   .   .     <- 3 blue, but (2,3) is above (2,2)
    //  y=2:  [R] [R] [R]  .   .   .     <- 3 red = first match
    //  y=1:  [3] [4] [5] [3] [4] [5]    <- support (alternating, no match)
    //  y=0:  [immutable row]

    let red = 0;
    let blue = 1;

    // First match row (red)
    place_block(&mut game, 0, 2, red);
    place_block(&mut game, 1, 2, red);
    place_block(&mut game, 2, 2, red);

    // Chain row (blue) — will fall after red dies
    place_block(&mut game, 0, 3, blue);
    place_block(&mut game, 1, 3, blue);
    place_block(&mut game, 2, 3, blue);

    game.elimination_requests.push((0, 2, None));

    // Run enough ticks for:
    // 1. Red match → DYING (1 tick)
    // 2. DYING_DELAY (90 ticks)
    // 3. Blue blocks fall to y=2
    // 4. Landing → elimination check → second match
    // 5. Score from chain
    let score_before = game.score.value();
    for _ in 0..200 {
        game.step(&input);
    }

    // A chain should produce significantly more score than a single match.
    let score_after = game.score.value();
    assert!(score_after > score_before, "Chain should produce score");

    // Verify the blue blocks also died (they should be gone by now).
    let remaining_test_blocks = game.blocks.iter_active()
        .filter(|b| b.flavor == red || b.flavor == blue)
        .count();
    assert_eq!(remaining_test_blocks, 0,
        "Both match groups should have been eliminated by the chain");
}

/// Post-game UI animation timers must keep advancing after gameplay freezes.
#[test]
fn test_finished_game_continues_post_game_timing() {
    let mut game = GameState::new(42);
    let empty = InputSnapshot::new();

    game.status = GameStatus::Won;
    game.finished_tick = Some(game.time_step);

    let frame0 = game.frame_data();
    assert_eq!(frame0.ui.ticks_since_finished, 0);

    let frame1 = game.step(&empty);
    assert_eq!(frame1.ui.ticks_since_finished, 1);

    let frame2 = game.step(&empty);
    assert_eq!(frame2.ui.ticks_since_finished, 2);
}

/// A garbage dropped from the top reports its first landing as `initial`.
#[test]
fn dropped_garbage_first_landing_is_initial() {
    use crate::gameplay_event::GameplayEvent;
    let mut game = GameState::new(42);
    let empty = InputSnapshot::new();
    for _ in 0..10 {
        game.step(&empty);
    }
    game.send_garbage(1, 6, 0);
    let mut landings = Vec::new();
    for _ in 0..2000 {
        let frame = game.step(&empty);
        landings.extend(frame.gameplay_events.iter().filter_map(|e| match e {
            GameplayEvent::GarbageLanded(g) => Some(g.initial),
            _ => None,
        }));
        if !landings.is_empty() {
            break;
        }
    }
    assert_eq!(landings, vec![true]);
}
