use super::*;

/// Build a PlanBoard from ASCII rows given top-to-bottom. `.` = empty, `#` = garbage,
/// a digit = that flavour (0..=4 here; the Rust sim has five colours and gray is 5).
fn board(rows: &[&str]) -> PlanBoard {
    let height = rows.len();
    let width = rows[0].len();
    let mut b = PlanBoard::new(width, height);
    for (r, row) in rows.iter().enumerate() {
        let y = height - 1 - r;
        for (x, ch) in row.bytes().enumerate() {
            let v = match ch {
                b'.' => PLAN_EMPTY,
                b'#' => PLAN_GARBAGE,
                d => i16::from(d - b'0'),
            };
            b.set(x, y, v);
        }
    }
    b
}

/// Apply a lateral swap in place (test helper).
fn apply_swap(b: &mut PlanBoard, x: usize, y: usize) {
    let tmp = b.get(x, y);
    b.set(x, y, b.get(x + 1, y));
    b.set(x + 1, y, tmp);
}

fn cascade(d: i32, t: i32, m: i32, g: i32) -> Cascade {
    Cascade {
        chain_depth: d,
        total_cleared: t,
        max_round: m,
        garbage_shattered: g,
    }
}

const OPTS: ChainSetupOptions = ChainSetupOptions {
    min_chain: 2,
    min_run: 4,
    shatter_weight: 3,
    lookahead: false,
    tie_seed: None,
};

#[test]
fn swap_completing_a_run_clears_exactly_those_blocks() {
    let c = evaluate_swap(&board(&["....", "1121"]), 2, 0);
    assert_eq!(c, cascade(1, 3, 3, 0));
}

#[test]
fn gravity_forms_a_four_wide_combo() {
    let c = evaluate_gravity(&board(&["...1", "111."]));
    assert_eq!((c.chain_depth, c.total_cleared, c.max_round), (1, 4, 4));
}

#[test]
fn detects_a_two_chain() {
    let b = board(&[".3.", ".21", ".11", "313"]);
    let c = evaluate_swap(&b, 1, 2);
    assert_eq!((c.chain_depth, c.total_cleared), (2, 6));
}

#[test]
fn shatters_garbage_adjacent_to_a_match() {
    let c = evaluate_swap(&board(&["####", "1121"]), 2, 0);
    assert_eq!(c.chain_depth, 1);
    assert_eq!(c.garbage_shattered, 3);
}

#[test]
fn a_swap_completing_nothing_is_all_zero() {
    assert_eq!(evaluate_swap(&board(&["....", "1213"]), 0, 0), cascade(0, 0, 0, 0));
}

#[test]
fn attack_value_ranks_a_chain_above_a_flat_combo() {
    let chain = attack_value(&cascade(3, 9, 3, 0));
    let flat = attack_value(&cascade(1, 9, 9, 0));
    assert!(chain > flat);
}

#[test]
fn gray_never_matches() {
    assert!(!flavor_match(5, 5));
    assert!(flavor_match(2, 2));
    assert!(!flavor_match(2, 3));
}

#[test]
fn shatter_cheapest_lateral_plan_under_a_slab() {
    let b = board(&["######", "121213"]);
    assert_eq!(plan_shatter_setup(&b, 10), Some(SetupPlan { x: 0, y: 0, cost: 2 }));
}

#[test]
fn shatter_respects_cost_cap_and_needs_garbage() {
    assert_eq!(plan_shatter_setup(&board(&["######", "121213"]), 1), None);
    assert_eq!(plan_shatter_setup(&board(&["......", "121213"]), 10), None);
}

#[test]
fn shatter_sees_windows_beside_a_slab() {
    let b = board(&["......", "#11211"]);
    assert_eq!(plan_shatter_setup(&b, 10), Some(SetupPlan { x: 3, y: 0, cost: 1 }));
    assert_eq!(evaluate_swap(&b, 3, 0).garbage_shattered, 1);
}

#[test]
fn shatter_sources_cannot_cross_garbage_and_standing_matches_skipped() {
    assert_eq!(plan_shatter_setup(&board(&["......", "11#212", "333333"]), 10), None);
}

#[test]
fn shatter_plan_converges() {
    let mut b = board(&["######", "121213"]);
    let mut plan = plan_shatter_setup(&b, 10);
    let mut last_cost = i32::MAX;
    let mut guard = 0;
    while let Some(p) = plan.filter(|p| p.cost > 1) {
        assert!(p.cost < last_cost);
        last_cost = p.cost;
        apply_swap(&mut b, p.x, p.y);
        plan = plan_shatter_setup(&b, 10);
        guard += 1;
        assert!(guard < 10);
    }
    let p = plan.expect("a final plan");
    assert!(evaluate_swap(&b, p.x, p.y).garbage_shattered > 0);
}

#[test]
fn shatter_never_swaps_two_matching_blocks() {
    let mut b = board(&["######", "112131"]);
    let plan = plan_shatter_setup(&b, 10).expect("plan");
    let before = b.cell.clone();
    apply_swap(&mut b, plan.x, plan.y);
    assert_ne!(b.cell, before);
}

#[test]
fn shatter_vertical_column_beside_a_slab() {
    // TS rows '#27.', '#72.', '#27.' use 7 as a filler; 3 stands in for it here.
    let b = board(&["#23.", "#32.", "#23.", "4545"]);
    let plan = plan_shatter_setup(&b, 10);
    assert_eq!(plan, Some(SetupPlan { x: 1, y: 2, cost: 1 }));
    assert!(evaluate_swap(&b, 1, 2).garbage_shattered > 0);
}

#[test]
fn shatter_vertical_plans_converge() {
    let mut b = board(&["#32.", "#32.", "#24.", "3434"]);
    let mut plan = plan_shatter_setup(&b, 10);
    assert_eq!(plan, Some(SetupPlan { x: 1, y: 2, cost: 2 }));
    let p = plan.unwrap();
    apply_swap(&mut b, p.x, p.y);
    plan = plan_shatter_setup(&b, 10);
    assert_eq!(plan.map(|p| p.cost), Some(1));
    let p = plan.unwrap();
    assert!(evaluate_swap(&b, p.x, p.y).garbage_shattered > 0);
}

#[test]
fn shatter_one_wide_tower_has_no_vertical_plan() {
    let b = board(&[".#..", ".1..", ".2..", ".3..", "4234"]);
    assert_eq!(plan_shatter_setup(&b, 10), None);
}

#[test]
fn chain_setup_re_enables_a_broken_two_chain() {
    let mut b = board(&[".3.", ".21", ".11", "133"]);
    let plan = plan_chain_setup(&b, &OPTS).expect("plan");
    assert_eq!((plan.x, plan.y), (0, 0));
    assert_eq!(plan.score, attack_value(&cascade(2, 6, 3, 0)));
    apply_swap(&mut b, 0, 0);
    assert_eq!(evaluate_swap(&b, 1, 2).chain_depth, 2);
}

#[test]
fn chain_setup_none_when_no_run_can_form() {
    assert_eq!(plan_chain_setup(&board(&["....", "1213"]), &OPTS), None);
}

#[test]
fn chain_setup_lookahead_ladder_converges_to_a_fire() {
    // TS rows ['.45.', '.21.', '.16.', '5125'] use colours 1,2,4,5,6; remapped
    // order-preservingly to 0,1,2,3,4 (colour 3 is the scarce one, 4 the blocker).
    let rows = [".23.", ".10.", ".04.", "3013"];
    assert_eq!(plan_chain_setup(&board(&rows), &OPTS), None);
    let mut b = board(&rows);
    let opts = ChainSetupOptions {
        lookahead: true,
        ..OPTS
    };
    let mut plan = plan_chain_setup(&b, &opts);
    assert!(plan.is_some());
    for _ in 0..2 {
        let Some(p) = plan else { break };
        apply_swap(&mut b, p.x, p.y);
        plan = plan_chain_setup(&b, &opts);
    }
    let mut fires = false;
    for y in 0..b.height {
        for x in 0..b.width - 1 {
            if evaluate_swap(&b, x, y).chain_depth >= 2 {
                fires = true;
            }
        }
    }
    assert!(fires);
}

#[test]
fn chain_setup_never_proposes_a_clearing_swap() {
    assert_eq!(plan_chain_setup(&board(&["....", "1121"]), &OPTS), None);
}

#[test]
fn big_combo_finishes_a_crossed_five_row_construction() {
    // TS colours 1..6 remapped to 0..4 (5 -> 4, 6 -> 2 as the right-hand filler).
    let mut b = board(&["0123", "0142", "1042", "0123", "1023"]);
    let plan = plan_big_combo_setup(&b, 18, None);
    assert_eq!(
        plan,
        Some(BigComboSetupPlan {
            x: 0,
            y: 0,
            cost: 1,
            size: 10
        })
    );
    apply_swap(&mut b, 0, 0);
    let trigger = evaluate_swap(&b, 0, 2);
    assert_eq!(trigger.total_cleared, 10);
    assert_eq!(trigger.max_round, 5);
}

#[test]
fn big_combo_is_reproducible_for_a_seed() {
    let b = board(&["012344", "012344", "102344", "012344", "102344"]);
    assert_eq!(
        plan_big_combo_setup(&b, 18, Some(7)),
        plan_big_combo_setup(&b, 18, Some(7))
    );
}

#[test]
fn undermine_digs_the_load_bearing_block() {
    let b = board(&[".#..", ".1..", ".2..", "3123"]);
    assert_eq!(plan_undermine(&b, 0, 2, None), Some((0, 2)));
    assert_eq!(plan_undermine(&b, 3, 2, None), Some((1, 2)));
}

#[test]
fn undermine_ignores_digs_without_garbage_or_fall_through() {
    assert_eq!(
        plan_undermine(&board(&["....", ".1..", ".2..", "3123"]), 0, 0, None),
        None
    );
    assert_eq!(
        plan_undermine(&board(&[".#..", "31.4", "3245", "3123"]), 0, 0, None),
        None
    );
}

#[test]
fn undermine_clears_a_lower_pocket_first() {
    // TS floor row '345634' uses a sixth colour; '340134' stands in (no 3-run).
    let mut b = board(&["######", "1.....", "12....", "340134"]);
    let prep = plan_undermine(&b, 0, 0, None);
    assert_eq!(prep, Some((1, 1)));
    let (px, py) = prep.unwrap();
    apply_swap(&mut b, px, py);
    assert_eq!(plan_undermine(&b, 0, 2, None), Some((0, 2)));
}

// --- Differential test against the reference implementation. ---
//
// `planner_cases.txt` holds recorded cases: each `board` line is followed by
// `op args => result` lines whose results were recorded from the reference planner; the
// Rust output must be identical.

const CASES: &str = include_str!("../planner_cases.txt");

fn fmt_cascade(c: &Cascade) -> String {
    format!(
        "{}.{}.{}.{}",
        c.chain_depth, c.total_cleared, c.max_round, c.garbage_shattered
    )
}

fn parse_seed(s: &str) -> Option<u32> {
    if s == "n" {
        None
    } else {
        Some(s.parse().expect("seed"))
    }
}

fn run_op(b: &PlanBoard, op: &[&str]) -> String {
    match op[0] {
        "canswap" => {
            let mut s = String::new();
            for y in 0..b.height {
                for x in 0..b.width - 1 {
                    s.push(if can_swap(b, x, y) { '1' } else { '0' });
                }
            }
            s
        }
        "swaps" => {
            let mut v = Vec::new();
            for y in 0..b.height {
                for x in 0..b.width - 1 {
                    v.push(fmt_cascade(&evaluate_swap(b, x, y)));
                }
            }
            v.join(" ")
        }
        "grav" => {
            let c = evaluate_gravity(b);
            format!("{}.{}", fmt_cascade(&c), attack_value(&c))
        }
        "hash" => hash_plan_board(b).to_string(),
        "shatter" => match plan_shatter_setup(b, op[1].parse().unwrap()) {
            Some(p) => format!("{}.{}.{}", p.x, p.y, p.cost),
            None => "null".into(),
        },
        "big" => match plan_big_combo_setup(b, op[1].parse().unwrap(), parse_seed(op[2])) {
            Some(p) => format!("{}.{}.{}.{}", p.x, p.y, p.cost, p.size),
            None => "null".into(),
        },
        "chain" => {
            let opts = ChainSetupOptions {
                min_chain: op[1].parse().unwrap(),
                min_run: op[2].parse().unwrap(),
                shatter_weight: op[3].parse().unwrap(),
                lookahead: op[4] == "1",
                tie_seed: parse_seed(op[5]),
            };
            match plan_chain_setup(b, &opts) {
                Some(p) => format!("{}.{}.{}", p.x, p.y, p.score),
                None => "null".into(),
            }
        }
        "undermine" => {
            let r = plan_undermine(b, op[1].parse().unwrap(), op[2].parse().unwrap(), parse_seed(op[3]));
            match r {
                Some((x, y)) => format!("{x}.{y}"),
                None => "null".into(),
            }
        }
        other => panic!("unknown op {other}"),
    }
}

#[test]
fn matches_typescript_on_generated_boards() {
    let mut board_opt: Option<PlanBoard> = None;
    let (mut boards, mut checks) = (0, 0);
    for (n, line) in CASES.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("board ") {
            let mut it = rest.split(' ');
            let height: usize = it.next().unwrap().parse().unwrap();
            let cells: Vec<i16> = it.next().unwrap().split(',').map(|v| v.parse().unwrap()).collect();
            assert_eq!(cells.len(), 6 * height);
            board_opt = Some(PlanBoard {
                cell: cells,
                width: 6,
                height,
            });
            boards += 1;
            continue;
        }
        let (lhs, rhs) = line.split_once(" => ").expect("op line");
        let op: Vec<&str> = lhs.split(' ').collect();
        let b = board_opt.as_ref().expect("board before op");
        let got = run_op(b, &op);
        assert_eq!(got, rhs, "line {} ({lhs}) board #{boards}", n + 1);
        checks += 1;
    }
    assert!(boards >= 250 && checks > 2000, "{boards} boards, {checks} checks");
}
