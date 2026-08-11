//! Tests for `gc_sim::offball_runs`.

use gc_core::vec2::Vec2;
use gc_data::formations::FormationRole;
use gc_data::players::StatBlock;
use gc_sim::brain;
use gc_sim::offball_runs as runs;
use gc_sim::stats;

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-6,
        "expected ~{expected}, got {actual}"
    );
}

/// Mirrors the Lua fixture's `x(home_x)` helper: away contexts flip across
/// the pitch's horizontal midline.
fn mirror_x(team: runs::Team, home_x: f64) -> f64 {
    if team == runs::Team::Home {
        home_x
    } else {
        960.0 - home_x
    }
}

/// Mirrors the Lua fixture's `context(team)` helper (its `overrides` table
/// is expressed in Rust with struct-update syntax at each call site).
fn context(team: runs::Team) -> runs::OffballRunContext {
    let (mid_index, fwd_index) = if team == runs::Team::Home {
        (4, 5)
    } else {
        (9, 10)
    };
    runs::OffballRunContext {
        team,
        field: runs::Field { w: 960.0, h: 540.0 },
        carrier_pos: Vec2::new(mirror_x(team, 300.0), 270.0),
        carrier_settled: true,
        carrier_pressure: 70.0,
        pressure_distance: 120.0,
        counterattack: false,
        players: vec![
            runs::OffballRunPlayer {
                player_index: mid_index,
                role: FormationRole::Mid,
                run_drive: 0.55,
                pos: Vec2::new(mirror_x(team, 500.0), 190.0),
                anchor_y: 0.5,
            },
            runs::OffballRunPlayer {
                player_index: fwd_index,
                role: FormationRole::Fwd,
                run_drive: 0.8,
                pos: Vec2::new(mirror_x(team, 570.0), 270.0),
                anchor_y: 0.5,
            },
        ],
        teammates: vec![
            runs::OffballRunTeammate {
                player_index: mid_index,
                pos: Vec2::new(mirror_x(team, 500.0), 190.0),
            },
            runs::OffballRunTeammate {
                player_index: fwd_index,
                pos: Vec2::new(mirror_x(team, 570.0), 270.0),
            },
        ],
        opponents: vec![
            runs::OffballRunOpponent {
                pos: Vec2::new(mirror_x(team, 650.0), 80.0),
                is_keeper: false,
            },
            runs::OffballRunOpponent {
                pos: Vec2::new(mirror_x(team, 620.0), 170.0),
                is_keeper: false,
            },
            runs::OffballRunOpponent {
                pos: Vec2::new(mirror_x(team, 610.0), 370.0),
                is_keeper: false,
            },
            runs::OffballRunOpponent {
                pos: Vec2::new(mirror_x(team, 640.0), 460.0),
                is_keeper: false,
            },
            runs::OffballRunOpponent {
                pos: Vec2::new(mirror_x(team, 920.0), 270.0),
                is_keeper: true,
            },
        ],
    }
}

fn slot_for(slots: &[brain::RunSlot], player_index: u32) -> Option<brain::RunSlot> {
    slots
        .iter()
        .find(|slot| slot.player_index == player_index)
        .copied()
}

fn authored_drive(pace: i64, mental: i64) -> f64 {
    stats::run_drive(StatBlock {
        pace,
        strength: 5,
        technique: 5,
        stamina: 5,
        mental,
    })
}

#[test]
fn role_gated_off_ball_runs_derives_the_authored_role_from_formation_and_stable_outfield_ordinal() {
    let cases: [(&str, [FormationRole; 4]); 3] = [
        (
            "2-1-1",
            [
                FormationRole::Def,
                FormationRole::Def,
                FormationRole::Mid,
                FormationRole::Fwd,
            ],
        ),
        (
            "1-2-1",
            [
                FormationRole::Def,
                FormationRole::Wide,
                FormationRole::Wide,
                FormationRole::Fwd,
            ],
        ),
        (
            "1-1-2",
            [
                FormationRole::Def,
                FormationRole::Mid,
                FormationRole::Fwd,
                FormationRole::Fwd,
            ],
        ),
    ];
    for (formation_id, roles) in cases {
        for (outfield_index, role) in roles.iter().enumerate() {
            assert_eq!(runs::formation_role(formation_id, outfield_index), *role);
        }
    }
}

#[test]
fn role_gated_off_ball_runs_grants_a_driven_forward_an_in_behind_target_beyond_the_last_defender() {
    let slots = runs::grant(&context(runs::Team::Home), &[], 10.0);
    let slot = slot_for(&slots, 5).expect("forward 5 should be granted an in-behind run");
    assert_eq!(slot.run_type, brain::RunType::InBehind);
    assert!(slot.target_x > 650.0);
    assert!(slot.target_x > 480.0);
    assert_eq!(slot.expires_at, 11.8);
}

#[test]
fn role_gated_off_ball_runs_mirrors_in_behind_geometry_and_arbitration_for_the_away_team() {
    let home = slot_for(&runs::grant(&context(runs::Team::Home), &[], 10.0), 5)
        .expect("home forward should be granted an in-behind run");
    let away = slot_for(&runs::grant(&context(runs::Team::Away), &[], 10.0), 10)
        .expect("away forward should be granted an in-behind run");
    assert_eq!(home.run_type, away.run_type);
    near(home.target_x + away.target_x, 960.0);
    near(home.target_y, away.target_y);
    near(home.score, away.score);
}

#[test]
fn role_gated_off_ball_runs_rejects_backward_or_immaterial_in_behind_targets_for_both_teams() {
    let home = runs::OffballRunContext {
        players: vec![
            runs::OffballRunPlayer {
                player_index: 4,
                role: FormationRole::Fwd,
                run_drive: 0.8,
                pos: Vec2::new(730.0, 240.0),
                anchor_y: 0.5,
            },
            runs::OffballRunPlayer {
                player_index: 5,
                role: FormationRole::Fwd,
                run_drive: 0.8,
                pos: Vec2::new(695.0, 300.0),
                anchor_y: 0.5,
            },
        ],
        teammates: vec![
            runs::OffballRunTeammate {
                player_index: 4,
                pos: Vec2::new(730.0, 240.0),
            },
            runs::OffballRunTeammate {
                player_index: 5,
                pos: Vec2::new(695.0, 300.0),
            },
        ],
        ..context(runs::Team::Home)
    };
    let away = runs::OffballRunContext {
        players: vec![
            runs::OffballRunPlayer {
                player_index: 9,
                role: FormationRole::Fwd,
                run_drive: 0.8,
                pos: Vec2::new(230.0, 240.0),
                anchor_y: 0.5,
            },
            runs::OffballRunPlayer {
                player_index: 10,
                role: FormationRole::Fwd,
                run_drive: 0.8,
                pos: Vec2::new(265.0, 300.0),
                anchor_y: 0.5,
            },
        ],
        teammates: vec![
            runs::OffballRunTeammate {
                player_index: 9,
                pos: Vec2::new(230.0, 240.0),
            },
            runs::OffballRunTeammate {
                player_index: 10,
                pos: Vec2::new(265.0, 300.0),
            },
        ],
        ..context(runs::Team::Away)
    };

    assert_eq!(runs::grant(&home, &[], 0.0).len(), 0);
    assert_eq!(runs::grant(&away, &[], 0.0).len(), 0);
}

#[test]
fn role_gated_off_ball_runs_requires_the_conservative_drive_threshold_and_a_clear_settled_lane() {
    assert_eq!(runs::RUN_DRIVE_THRESHOLD, 0.55);
    assert!(authored_drive(8, 2) >= runs::RUN_DRIVE_THRESHOLD);
    for (pace, mental) in [(7, 3), (6, 3), (5, 5)] {
        assert!(authored_drive(pace, mental) < runs::RUN_DRIVE_THRESHOLD);
    }

    let mut low_drive = context(runs::Team::Home);
    low_drive.players[1].run_drive = runs::RUN_DRIVE_THRESHOLD - 0.01;
    assert!(slot_for(&runs::grant(&low_drive, &[], 0.0), 5).is_none());

    let unsettled = runs::OffballRunContext {
        carrier_settled: false,
        ..context(runs::Team::Home)
    };
    assert_eq!(runs::grant(&unsettled, &[], 0.0).len(), 0);

    let mut blocked = context(runs::Team::Home);
    blocked.opponents[0].pos = Vec2::new(500.0, 270.0);
    assert!(slot_for(&runs::grant(&blocked, &[], 0.0), 5).is_none());
}

#[test]
fn role_gated_off_ball_runs_checks_a_midfield_runner_short_under_pressure_and_moves_goal_side_of_its_marker()
 {
    let mut fixture = context(runs::Team::Home);
    fixture.players = vec![runs::OffballRunPlayer {
        player_index: 4,
        role: FormationRole::Mid,
        run_drive: 0.5,
        pos: Vec2::new(500.0, 190.0),
        anchor_y: 0.5,
    }];
    fixture.teammates = vec![runs::OffballRunTeammate {
        player_index: 4,
        pos: Vec2::new(500.0, 190.0),
    }];
    fixture.opponents[0].pos = Vec2::new(400.0, 205.0);
    let slots = runs::grant(&fixture, &[], 0.0);
    let slot = slot_for(&slots, 4).expect("mid should be granted a come-short run");
    assert_eq!(slot.run_type, brain::RunType::ComeShort);
    assert!(slot.target_x > fixture.opponents[0].pos.x);
    let distance = fixture
        .carrier_pos
        .dist(Vec2::new(slot.target_x, slot.target_y));
    assert!(distance >= runs::MIN_SUPPORT_DISTANCE);
    assert!(distance <= 139.0);

    fixture.carrier_pressure = fixture.pressure_distance + 1.0;
    assert_eq!(runs::grant(&fixture, &[], 0.0).len(), 0);
}

#[test]
fn role_gated_off_ball_runs_mirrors_come_short_geometry_and_scoring_for_the_away_team() {
    let mut home = context(runs::Team::Home);
    home.players = vec![runs::OffballRunPlayer {
        player_index: 4,
        role: FormationRole::Mid,
        run_drive: 0.5,
        pos: Vec2::new(500.0, 190.0),
        anchor_y: 0.5,
    }];
    home.teammates = vec![runs::OffballRunTeammate {
        player_index: 4,
        pos: Vec2::new(500.0, 190.0),
    }];
    home.opponents[0].pos = Vec2::new(400.0, 205.0);

    let mut away = context(runs::Team::Away);
    away.players = vec![runs::OffballRunPlayer {
        player_index: 9,
        role: FormationRole::Mid,
        run_drive: 0.5,
        pos: Vec2::new(460.0, 190.0),
        anchor_y: 0.5,
    }];
    away.teammates = vec![runs::OffballRunTeammate {
        player_index: 9,
        pos: Vec2::new(460.0, 190.0),
    }];
    away.opponents[0].pos = Vec2::new(560.0, 205.0);

    let home_slot =
        slot_for(&runs::grant(&home, &[], 0.0), 4).expect("home mid should be granted a run");
    let away_slot =
        slot_for(&runs::grant(&away, &[], 0.0), 9).expect("away mid should be granted a run");
    assert_eq!(home_slot.run_type, brain::RunType::ComeShort);
    assert_eq!(away_slot.run_type, brain::RunType::ComeShort);
    near(home_slot.target_x + away_slot.target_x, 960.0);
    near(home_slot.target_y, away_slot.target_y);
    near(home_slot.score, away_slot.score);
}

#[test]
fn role_gated_off_ball_runs_rejects_come_short_targets_without_meaningful_carrier_progress() {
    let projected_home = runs::OffballRunContext {
        players: vec![runs::OffballRunPlayer {
            player_index: 4,
            role: FormationRole::Mid,
            run_drive: 0.5,
            pos: Vec2::new(390.0, 270.0),
            anchor_y: 0.5,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 4,
            pos: Vec2::new(390.0, 270.0),
        }],
        ..context(runs::Team::Home)
    };
    let projected_away = runs::OffballRunContext {
        players: vec![runs::OffballRunPlayer {
            player_index: 9,
            role: FormationRole::Mid,
            run_drive: 0.5,
            pos: Vec2::new(570.0, 270.0),
            anchor_y: 0.5,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 9,
            pos: Vec2::new(570.0, 270.0),
        }],
        ..context(runs::Team::Away)
    };
    assert_eq!(runs::grant(&projected_home, &[], 0.0).len(), 0);
    assert_eq!(runs::grant(&projected_away, &[], 0.0).len(), 0);

    let mut marked_home = runs::OffballRunContext {
        players: vec![runs::OffballRunPlayer {
            player_index: 4,
            role: FormationRole::Mid,
            run_drive: 0.5,
            pos: Vec2::new(445.0, 270.0),
            anchor_y: 0.5,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 4,
            pos: Vec2::new(445.0, 270.0),
        }],
        ..context(runs::Team::Home)
    };
    marked_home.opponents[0].pos = Vec2::new(415.0, 270.0);
    let mut marked_away = runs::OffballRunContext {
        players: vec![runs::OffballRunPlayer {
            player_index: 9,
            role: FormationRole::Mid,
            run_drive: 0.5,
            pos: Vec2::new(515.0, 270.0),
            anchor_y: 0.5,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 9,
            pos: Vec2::new(515.0, 270.0),
        }],
        ..context(runs::Team::Away)
    };
    marked_away.opponents[0].pos = Vec2::new(545.0, 270.0);
    assert_eq!(runs::grant(&marked_home, &[], 0.0).len(), 0);
    assert_eq!(runs::grant(&marked_away, &[], 0.0).len(), 0);
}

#[test]
fn role_gated_off_ball_runs_mirrors_multi_candidate_grant_order_while_enforcing_the_two_run_cap() {
    let home = runs::OffballRunContext {
        players: vec![
            runs::OffballRunPlayer {
                player_index: 4,
                role: FormationRole::Fwd,
                run_drive: 0.9,
                pos: Vec2::new(500.0, 180.0),
                anchor_y: 0.3,
            },
            runs::OffballRunPlayer {
                player_index: 5,
                role: FormationRole::Fwd,
                run_drive: 0.9,
                pos: Vec2::new(500.0, 360.0),
                anchor_y: 0.7,
            },
            runs::OffballRunPlayer {
                player_index: 3,
                role: FormationRole::Mid,
                run_drive: 0.6,
                pos: Vec2::new(450.0, 230.0),
                anchor_y: 0.5,
            },
        ],
        teammates: vec![
            runs::OffballRunTeammate {
                player_index: 3,
                pos: Vec2::new(450.0, 230.0),
            },
            runs::OffballRunTeammate {
                player_index: 4,
                pos: Vec2::new(500.0, 180.0),
            },
            runs::OffballRunTeammate {
                player_index: 5,
                pos: Vec2::new(500.0, 360.0),
            },
        ],
        ..context(runs::Team::Home)
    };
    let away = runs::OffballRunContext {
        players: vec![
            runs::OffballRunPlayer {
                player_index: 9,
                role: FormationRole::Fwd,
                run_drive: 0.9,
                pos: Vec2::new(460.0, 180.0),
                anchor_y: 0.3,
            },
            runs::OffballRunPlayer {
                player_index: 10,
                role: FormationRole::Fwd,
                run_drive: 0.9,
                pos: Vec2::new(460.0, 360.0),
                anchor_y: 0.7,
            },
            runs::OffballRunPlayer {
                player_index: 8,
                role: FormationRole::Mid,
                run_drive: 0.6,
                pos: Vec2::new(510.0, 230.0),
                anchor_y: 0.5,
            },
        ],
        teammates: vec![
            runs::OffballRunTeammate {
                player_index: 8,
                pos: Vec2::new(510.0, 230.0),
            },
            runs::OffballRunTeammate {
                player_index: 9,
                pos: Vec2::new(460.0, 180.0),
            },
            runs::OffballRunTeammate {
                player_index: 10,
                pos: Vec2::new(460.0, 360.0),
            },
        ],
        ..context(runs::Team::Away)
    };
    let home_slots = runs::grant(&home, &[], 0.0);
    let away_slots = runs::grant(&away, &[], 0.0);
    assert_eq!(home_slots.len(), 2);
    assert_eq!(away_slots.len(), 2);
    for index in 0..2 {
        assert_eq!(
            away_slots[index].player_index,
            home_slots[index].player_index + 5
        );
        assert_eq!(away_slots[index].run_type, home_slots[index].run_type);
        near(
            away_slots[index].target_x + home_slots[index].target_x,
            960.0,
        );
        near(away_slots[index].target_y, home_slots[index].target_y);
        near(away_slots[index].score, home_slots[index].score);
    }
}

#[test]
fn role_gated_off_ball_runs_sends_a_wide_role_to_an_unoccupied_outer_lane_at_a_legal_support_distance()
 {
    let mut fixture = runs::OffballRunContext {
        carrier_pressure: 200.0,
        players: vec![runs::OffballRunPlayer {
            player_index: 3,
            role: FormationRole::Wide,
            run_drive: 0.5,
            pos: Vec2::new(500.0, 150.0),
            anchor_y: 0.3,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 3,
            pos: Vec2::new(500.0, 150.0),
        }],
        ..context(runs::Team::Home)
    };
    let slot = slot_for(&runs::grant(&fixture, &[], 0.0), 3).expect("wide should be granted a run");
    assert_eq!(slot.run_type, brain::RunType::HoldWidth);
    assert!(slot.target_y < fixture.field.h / 3.0);
    let distance = fixture
        .carrier_pos
        .dist(Vec2::new(slot.target_x, slot.target_y));
    assert!(distance >= runs::MIN_SUPPORT_DISTANCE);
    assert!(distance <= runs::MAX_SUPPORT_DISTANCE);

    fixture.teammates.push(runs::OffballRunTeammate {
        player_index: 4,
        pos: Vec2::new(fixture.carrier_pos.x + 70.0, fixture.field.h / 6.0),
    });
    assert_eq!(runs::grant(&fixture, &[], 0.0).len(), 0);
}

#[test]
fn role_gated_off_ball_runs_treats_a_mirrored_wide_carrier_as_occupying_the_same_width_lane() {
    let home = runs::OffballRunContext {
        carrier_pos: Vec2::new(300.0, 90.0),
        carrier_pressure: 200.0,
        players: vec![runs::OffballRunPlayer {
            player_index: 3,
            role: FormationRole::Wide,
            run_drive: 0.5,
            pos: Vec2::new(500.0, 150.0),
            anchor_y: 0.3,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 3,
            pos: Vec2::new(500.0, 150.0),
        }],
        ..context(runs::Team::Home)
    };
    let away = runs::OffballRunContext {
        carrier_pos: Vec2::new(660.0, 450.0),
        carrier_pressure: 200.0,
        players: vec![runs::OffballRunPlayer {
            player_index: 8,
            role: FormationRole::Wide,
            run_drive: 0.5,
            pos: Vec2::new(460.0, 390.0),
            anchor_y: 0.7,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 8,
            pos: Vec2::new(460.0, 390.0),
        }],
        ..context(runs::Team::Away)
    };

    assert_eq!(runs::grant(&home, &[], 0.0).len(), 0);
    assert_eq!(runs::grant(&away, &[], 0.0).len(), 0);
}

#[test]
fn role_gated_off_ball_runs_mirrors_the_opposite_hold_width_flank_and_rejects_an_empty_distance_intersection()
 {
    let home = runs::OffballRunContext {
        carrier_pressure: 200.0,
        players: vec![runs::OffballRunPlayer {
            player_index: 3,
            role: FormationRole::Wide,
            run_drive: 0.5,
            pos: Vec2::new(500.0, 400.0),
            anchor_y: 0.3,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 3,
            pos: Vec2::new(500.0, 150.0),
        }],
        ..context(runs::Team::Home)
    };
    let away = runs::OffballRunContext {
        carrier_pressure: 200.0,
        players: vec![runs::OffballRunPlayer {
            player_index: 8,
            role: FormationRole::Wide,
            run_drive: 0.5,
            pos: Vec2::new(460.0, 140.0),
            anchor_y: 0.7,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 8,
            pos: Vec2::new(460.0, 390.0),
        }],
        ..context(runs::Team::Away)
    };
    let home_slot =
        slot_for(&runs::grant(&home, &[], 0.0), 3).expect("home wide should be granted a run");
    let away_slot =
        slot_for(&runs::grant(&away, &[], 0.0), 8).expect("away wide should be granted a run");
    assert_eq!(home_slot.run_type, brain::RunType::HoldWidth);
    assert_eq!(away_slot.run_type, brain::RunType::HoldWidth);
    near(home_slot.target_x + away_slot.target_x, 960.0);
    near(home_slot.target_y + away_slot.target_y, 540.0);
    near(home_slot.score, away_slot.score);

    let impossible = runs::OffballRunContext {
        carrier_pos: Vec2::new(480.0, 450.0),
        carrier_pressure: 200.0,
        players: vec![runs::OffballRunPlayer {
            player_index: 3,
            role: FormationRole::Wide,
            run_drive: 0.5,
            pos: Vec2::new(500.0, 60.0),
            anchor_y: 0.3,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 3,
            pos: Vec2::new(500.0, 60.0),
        }],
        ..context(runs::Team::Home)
    };
    assert_eq!(runs::grant(&impossible, &[], 0.0).len(), 0);
}

#[test]
fn role_gated_off_ball_runs_rejects_come_short_and_hold_width_targets_when_field_clamping_breaks_spacing()
 {
    let come_home = runs::OffballRunContext {
        carrier_pos: Vec2::new(20.0, 270.0),
        players: vec![runs::OffballRunPlayer {
            player_index: 4,
            role: FormationRole::Mid,
            run_drive: 0.5,
            pos: Vec2::new(12.0, 270.0),
            anchor_y: 0.5,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 4,
            pos: Vec2::new(12.0, 270.0),
        }],
        ..context(runs::Team::Home)
    };
    let come_away = runs::OffballRunContext {
        carrier_pos: Vec2::new(940.0, 270.0),
        players: vec![runs::OffballRunPlayer {
            player_index: 9,
            role: FormationRole::Mid,
            run_drive: 0.5,
            pos: Vec2::new(948.0, 270.0),
            anchor_y: 0.5,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 9,
            pos: Vec2::new(948.0, 270.0),
        }],
        ..context(runs::Team::Away)
    };
    assert_eq!(runs::grant(&come_home, &[], 0.0).len(), 0);
    assert_eq!(runs::grant(&come_away, &[], 0.0).len(), 0);

    let width_home = runs::OffballRunContext {
        carrier_pos: Vec2::new(936.0, 90.0),
        carrier_pressure: 200.0,
        players: vec![runs::OffballRunPlayer {
            player_index: 3,
            role: FormationRole::Wide,
            run_drive: 0.5,
            pos: Vec2::new(900.0, 80.0),
            anchor_y: 0.3,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 3,
            pos: Vec2::new(900.0, 80.0),
        }],
        ..context(runs::Team::Home)
    };
    let width_away = runs::OffballRunContext {
        carrier_pos: Vec2::new(24.0, 450.0),
        carrier_pressure: 200.0,
        players: vec![runs::OffballRunPlayer {
            player_index: 8,
            role: FormationRole::Wide,
            run_drive: 0.5,
            pos: Vec2::new(60.0, 460.0),
            anchor_y: 0.7,
        }],
        teammates: vec![runs::OffballRunTeammate {
            player_index: 8,
            pos: Vec2::new(60.0, 460.0),
        }],
        ..context(runs::Team::Away)
    };
    assert_eq!(runs::grant(&width_home, &[], 0.0).len(), 0);
    assert_eq!(runs::grant(&width_away, &[], 0.0).len(), 0);
}

#[test]
fn role_gated_off_ball_runs_caps_a_team_at_two_and_retains_marginal_active_assignments_until_expiry()
 {
    let fixture = runs::OffballRunContext {
        players: vec![
            runs::OffballRunPlayer {
                player_index: 2,
                role: FormationRole::Wide,
                run_drive: 0.4,
                pos: Vec2::new(460.0, 110.0),
                anchor_y: 0.3,
            },
            runs::OffballRunPlayer {
                player_index: 3,
                role: FormationRole::Wide,
                run_drive: 0.9,
                pos: Vec2::new(500.0, 430.0),
                anchor_y: 0.7,
            },
            runs::OffballRunPlayer {
                player_index: 4,
                role: FormationRole::Mid,
                run_drive: 0.7,
                pos: Vec2::new(500.0, 190.0),
                anchor_y: 0.5,
            },
            runs::OffballRunPlayer {
                player_index: 5,
                role: FormationRole::Fwd,
                run_drive: 0.9,
                pos: Vec2::new(570.0, 270.0),
                anchor_y: 0.5,
            },
        ],
        teammates: vec![
            runs::OffballRunTeammate {
                player_index: 2,
                pos: Vec2::new(460.0, 110.0),
            },
            runs::OffballRunTeammate {
                player_index: 3,
                pos: Vec2::new(500.0, 430.0),
            },
            runs::OffballRunTeammate {
                player_index: 4,
                pos: Vec2::new(500.0, 190.0),
            },
            runs::OffballRunTeammate {
                player_index: 5,
                pos: Vec2::new(570.0, 270.0),
            },
        ],
        ..context(runs::Team::Home)
    };
    let active = [brain::RunSlot {
        player_index: 2,
        run_type: brain::RunType::HoldWidth,
        score: 1.0,
        target_x: 410.0,
        target_y: 90.0,
        granted_at: 8.0,
        expires_at: 11.0,
    }];
    let slots = runs::grant(&fixture, &active, 10.0);
    assert_eq!(slots.len() as u32, runs::MAX_ACTIVE_PER_TEAM);
    let retained = slot_for(&slots, 2).expect("player 2's active run should be retained");
    assert_eq!(retained.target_x, 410.0);
    assert_eq!(retained.expires_at, 11.0);
}

#[test]
fn role_gated_off_ball_runs_derives_the_fixed_telegraph_window_from_expiry_without_persistent_state()
 {
    let expires_at = 11.8;
    assert!(!runs::telegraphing(9.99, expires_at));
    assert!(runs::telegraphing(10.0, expires_at));
    assert!(runs::telegraphing(10.199_999, expires_at));
    assert!(!runs::telegraphing(10.2, expires_at));
    assert!(!runs::telegraphing(expires_at, expires_at));

    let negative_now = -0.2;
    let negative_expiry = negative_now + runs::RUN_LIFETIME_SECONDS;
    assert!(runs::telegraphing(negative_now, negative_expiry));
    assert!(!runs::telegraphing(
        negative_now + runs::TELEGRAPH_SECONDS,
        negative_expiry
    ));
}
