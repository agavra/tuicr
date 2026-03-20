use serde::{Deserialize, Serialize};

/// Discriminator field used by all messages in the NDJSON protocol.
/// Both inbound and outbound messages carry `"type"` as the first-level key.
/// Response messages follow the convention: `<request_type>_response`.

// ---------------------------------------------------------------------------
// Inbound messages (channel.ts → tuicr)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum InboundMessage {
    #[serde(rename = "subscribe")]
    Subscribe(SubscribeRequest),
    #[serde(rename = "get_review_status")]
    GetReviewStatus,
    #[serde(rename = "poll_feedback")]
    PollFeedback(PollFeedbackRequest),
}

#[derive(Debug, Deserialize)]
pub struct SubscribeRequest {
    #[allow(dead_code)]
    #[serde(default)]
    pub events: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PollFeedbackRequest {
    #[serde(default)]
    pub wait: bool,
}

// ---------------------------------------------------------------------------
// Outbound messages (tuicr → channel.ts)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum OutboundMessage {
    #[serde(rename = "subscribe_response")]
    SubscribeResponse { ok: bool },

    #[serde(rename = "get_review_status_response")]
    GetReviewStatusResponse {
        summary: String,
        comment_count: usize,
        files_reviewed: usize,
        files_total: usize,
    },

    #[serde(rename = "poll_feedback_response")]
    PollFeedbackResponse {
        has_feedback: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        feedback: Option<String>,
    },

    #[serde(rename = "event_notification")]
    EventNotification {
        event: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        payload: Option<EventPayload>,
    },
}

#[derive(Debug, Serialize)]
pub struct EventPayload {
    pub message: String,
}

// ---------------------------------------------------------------------------
// Encode / Decode helpers
// ---------------------------------------------------------------------------

/// Encode an outbound message as a single NDJSON line (JSON + newline).
pub fn encode(msg: &OutboundMessage) -> serde_json::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(msg)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Decode an inbound message from a JSON string (one NDJSON line, no trailing newline).
pub fn decode(line: &str) -> serde_json::Result<InboundMessage> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_decode_subscribe() {
        let json = r#"{"type":"subscribe","events":["feedback_submitted"]}"#;
        let msg = decode(json).unwrap();
        match msg {
            InboundMessage::Subscribe(req) => {
                assert_eq!(req.events, vec!["feedback_submitted"]);
            }
            _ => panic!("expected Subscribe"),
        }
    }

    #[test]
    fn should_decode_subscribe_without_events() {
        let json = r#"{"type":"subscribe"}"#;
        let msg = decode(json).unwrap();
        match msg {
            InboundMessage::Subscribe(req) => {
                assert!(req.events.is_empty());
            }
            _ => panic!("expected Subscribe"),
        }
    }

    #[test]
    fn should_decode_get_review_status() {
        let json = r#"{"type":"get_review_status"}"#;
        let msg = decode(json).unwrap();
        assert!(matches!(msg, InboundMessage::GetReviewStatus));
    }

    #[test]
    fn should_decode_poll_feedback() {
        let json = r#"{"type":"poll_feedback","wait":true}"#;
        let msg = decode(json).unwrap();
        match msg {
            InboundMessage::PollFeedback(req) => {
                assert!(req.wait);
            }
            _ => panic!("expected PollFeedback"),
        }
    }

    #[test]
    fn should_decode_poll_feedback_default_wait() {
        let json = r#"{"type":"poll_feedback"}"#;
        let msg = decode(json).unwrap();
        match msg {
            InboundMessage::PollFeedback(req) => {
                assert!(!req.wait);
            }
            _ => panic!("expected PollFeedback"),
        }
    }

    #[test]
    fn should_encode_subscribe_response() {
        let msg = OutboundMessage::SubscribeResponse { ok: true };
        let bytes = encode(&msg).unwrap();
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.ends_with('\n'));
        assert!(s.contains(r#""type":"subscribe_response""#));
        assert!(s.contains(r#""ok":true"#));
    }

    #[test]
    fn should_encode_review_status_response() {
        let msg = OutboundMessage::GetReviewStatusResponse {
            summary: "2 comments on 3 files".to_string(),
            comment_count: 2,
            files_reviewed: 1,
            files_total: 3,
        };
        let bytes = encode(&msg).unwrap();
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.contains(r#""comment_count":2"#));
        assert!(s.contains(r#""files_reviewed":1"#));
    }

    #[test]
    fn should_encode_poll_feedback_response_with_feedback() {
        let msg = OutboundMessage::PollFeedbackResponse {
            has_feedback: true,
            feedback: Some("Review markdown here".to_string()),
        };
        let bytes = encode(&msg).unwrap();
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.contains(r#""has_feedback":true"#));
        assert!(s.contains(r#""feedback":"Review markdown here""#));
    }

    #[test]
    fn should_encode_poll_feedback_response_without_feedback() {
        let msg = OutboundMessage::PollFeedbackResponse {
            has_feedback: false,
            feedback: None,
        };
        let bytes = encode(&msg).unwrap();
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.contains(r#""has_feedback":false"#));
        // The "feedback" key should not appear (skipped when None)
        assert!(!s.contains(r#""feedback":"#));
    }

    #[test]
    fn should_encode_event_notification() {
        let msg = OutboundMessage::EventNotification {
            event: "feedback_submitted".to_string(),
            payload: Some(EventPayload {
                message: "Review ready".to_string(),
            }),
        };
        let bytes = encode(&msg).unwrap();
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.contains(r#""event":"feedback_submitted""#));
        assert!(s.contains(r#""message":"Review ready""#));
    }

    #[test]
    fn should_roundtrip_unknown_type_fails() {
        let json = r#"{"type":"unknown_message"}"#;
        assert!(decode(json).is_err());
    }
}
