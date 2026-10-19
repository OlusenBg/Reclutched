//! Turns one raw Stats API message into a [`StatsEvent`].

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;

use super::events::StatsEvent;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("bad data for event {event}: {source}")]
    BadData {
        event: String,
        source: serde_json::Error,
    },
}

/// The outer wrapper: `{"Event": "...", "Data": ...}`.
#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "Event")]
    event: String,
    #[serde(rename = "Data", default)]
    data: Value,
}

/// Parses one websocket text message.
///
/// The docs show `Data` as a JSON object, but the real game sends it as a *string* that itself
/// contains JSON (so it is decoded twice). We accept both.
pub fn parse_message(raw: &str) -> Result<StatsEvent, ParseError> {
    let Envelope { event, mut data } = serde_json::from_str(raw)?;

    if let Value::String(inner) = &data {
        // `inner` borrows from `data`, so we build the new value first, then assign it.
        let decoded: Value = serde_json::from_str(inner)?;
        data = decoded;
    }

    // Decodes `data` into whichever struct the event needs; labels errors with the event name.
    fn decode<T: DeserializeOwned>(event: &str, data: Value) -> Result<T, ParseError> {
        serde_json::from_value(data).map_err(|source| ParseError::BadData {
            event: event.to_string(),
            source,
        })
    }

    use StatsEvent as E;
    Ok(match event.as_str() {
        "UpdateState" => E::UpdateState(decode(&event, data)?),
        "BallHit" => E::BallHit(decode(&event, data)?),
        "BoostPickup" => E::BoostPickup(decode(&event, data)?),
        "ClockUpdatedSeconds" => E::ClockUpdatedSeconds(decode(&event, data)?),
        "CrossbarHit" => E::CrossbarHit(decode(&event, data)?),
        "GoalScored" => E::GoalScored(decode(&event, data)?),
        "MatchEnded" => E::MatchEnded(decode(&event, data)?),
        "PlayerJoined" => E::PlayerJoined(decode(&event, data)?),
        "PlayerLeft" => E::PlayerLeft(decode(&event, data)?),
        "ReplayCreated" => E::ReplayCreated(decode(&event, data)?),
        "StatfeedEvent" => E::StatfeedEvent(decode(&event, data)?),
        "CountdownBegin" => E::CountdownBegin(decode(&event, data)?),
        "GoalReplayEnd" => E::GoalReplayEnd(decode(&event, data)?),
        "GoalReplayStart" => E::GoalReplayStart(decode(&event, data)?),
        "GoalReplayWillEnd" => E::GoalReplayWillEnd(decode(&event, data)?),
        "MatchCreated" => E::MatchCreated(decode(&event, data)?),
        "MatchDestroyed" => E::MatchDestroyed(decode(&event, data)?),
        "MatchInitialized" => E::MatchInitialized(decode(&event, data)?),
        "MatchPaused" => E::MatchPaused(decode(&event, data)?),
        "MatchUnpaused" => E::MatchUnpaused(decode(&event, data)?),
        "PodiumStart" => E::PodiumStart(decode(&event, data)?),
        "RoundStarted" => E::RoundStarted(decode(&event, data)?),
        _ => E::Unknown { name: event, data },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_as_string_and_as_object_parse_the_same() {
        let as_string =
            r#"{"Event":"MatchEnded","Data":"{\"MatchGuid\":\"AB\",\"WinnerTeamNum\":1}"}"#;
        let as_object = r#"{"Event":"MatchEnded","Data":{"MatchGuid":"AB","WinnerTeamNum":1}}"#;
        assert_eq!(
            parse_message(as_string).unwrap(),
            parse_message(as_object).unwrap()
        );
    }

    #[test]
    fn empty_match_guid_becomes_none() {
        let raw = r#"{"Event":"MatchPaused","Data":"{\"MatchGuid\":\"\"}"}"#;
        match parse_message(raw).unwrap() {
            StatsEvent::MatchPaused(m) => assert_eq!(m.match_guid, None),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn unknown_event_is_kept_not_dropped() {
        let raw = r#"{"Event":"BrandNewThing","Data":{"X":1}}"#;
        assert!(matches!(
            parse_message(raw).unwrap(),
            StatsEvent::Unknown { name, .. } if name == "BrandNewThing"
        ));
    }

    #[test]
    fn bad_json_and_bad_data_are_errors() {
        assert!(matches!(parse_message("nope"), Err(ParseError::Json(_))));
        // MatchEnded without the required WinnerTeamNum.
        let raw = r#"{"Event":"MatchEnded","Data":{"MatchGuid":"AB"}}"#;
        assert!(matches!(
            parse_message(raw),
            Err(ParseError::BadData { .. })
        ));
    }
}
