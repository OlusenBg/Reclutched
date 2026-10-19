//! Parses every saved payload. `offline_bots` and `casual_2v2` are real captures from the game
//! (names and ids replaced with fake ones); `docs_examples` come from the official docs.

use reclutched_core::rocketleague::{parse_message, StatsEvent};

fn load(name: &str) -> Vec<StatsEvent> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| parse_message(line).unwrap_or_else(|e| panic!("{name}: {e}\n{line}")))
        .collect()
}

#[test]
fn all_fixture_events_are_known_types() {
    for file in [
        "offline_bots.jsonl",
        "casual_2v2.jsonl",
        "docs_examples.jsonl",
    ] {
        for ev in load(file) {
            assert!(!matches!(ev, StatsEvent::Unknown { .. }), "{file}: {ev:?}");
        }
    }
}

#[test]
fn online_match_has_guid_winner_and_playlist() {
    let events = load("casual_2v2.jsonl");
    let ended = events.iter().find_map(|e| match e {
        StatsEvent::MatchEnded(m) => Some(m),
        _ => None,
    });
    let ended = ended.expect("fixture has a MatchEnded");
    assert_eq!(ended.winner_team_num, 1);
    assert!(ended.match_guid.is_some());

    let state = events.iter().rev().find_map(|e| match e {
        StatsEvent::UpdateState(u) => Some(u),
        _ => None,
    });
    let state = state.expect("fixture has an UpdateState");
    assert_eq!(state.game.playlist_id, 3);
    // More than 4 because players who left or joined mid-match still show up.
    assert!(state.players.len() >= 4);
    assert_eq!(state.game.teams.len(), 2);
}

#[test]
fn goal_with_assist_is_decoded() {
    let events = load("casual_2v2.jsonl");
    let goal = events.iter().find_map(|e| match e {
        StatsEvent::GoalScored(g) if g.assister.is_some() => Some(g),
        _ => None,
    });
    let goal = goal.expect("fixture has a goal with an assist");
    assert_eq!(
        goal.scorer.team_num,
        goal.assister.as_ref().unwrap().team_num
    );
}

#[test]
fn offline_match_has_no_guid_and_spectator_fields_only_for_target() {
    let events = load("offline_bots.jsonl");
    let state = events.iter().find_map(|e| match e {
        StatsEvent::UpdateState(u) => Some(u),
        _ => None,
    });
    let state = state.unwrap();
    assert_eq!(state.match_guid, None);
    assert_eq!(state.game.playlist_id, 8);
    // Boost/Speed are spectator-only: just the tracked player has them.
    assert_eq!(
        state.players.iter().filter(|p| p.boost.is_some()).count(),
        1
    );
}

#[test]
fn docs_only_events_parse() {
    let events = load("docs_examples.jsonl");
    assert!(events
        .iter()
        .any(|e| matches!(e, StatsEvent::CrossbarHit(_))));
    assert!(events
        .iter()
        .any(|e| matches!(e, StatsEvent::PodiumStart(_))));
    assert!(events
        .iter()
        .any(|e| matches!(e, StatsEvent::ReplayCreated(_))));
    assert!(events
        .iter()
        .any(|e| matches!(e, StatsEvent::GoalReplayWillEnd(_))));
}
