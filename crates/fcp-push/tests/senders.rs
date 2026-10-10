//! The senders against the push services' stand-ins (`docs/PUSH.md` §3):
//! what each sends, how it authenticates, and what each answer means.

use fcp_core::push::{PushConfig, PushPayload};
use fcp_push::{PushKind, PushMessage, PushOutcome, PushSender};
use fcp_testkit::push::{StandInApns, StandInFcm, APNS_TOPIC};

fn as_is(value: &str) -> anyhow::Result<String> {
    Ok(value.to_string())
}

async fn sender(payload: PushPayload) -> (PushSender, StandInApns, StandInFcm) {
    let apns = StandInApns::start().await;
    let fcm = StandInFcm::start().await;
    let config = PushConfig {
        max_expires: None,
        payload,
        apns: vec![apns.config()],
        fcm: vec![fcm.config()],
        relay: None,
    };
    (PushSender::new(&config, &as_is).unwrap(), apns, fcm)
}

fn ringing() -> PushMessage {
    PushMessage::new(PushKind::Call, "call-0f3a", "tenant-1")
        .caller(Some("1002".into()), Some("Ann Lee".into()))
}

#[tokio::test]
async fn a_call_goes_to_apns_as_a_voip_push_now_or_never() {
    let (sender, apns, _) = sender(PushPayload::Full).await;
    let outcome = sender
        .send(&StandInApns::address("a1b2c3d4e5f6a7b8c9d0"), &ringing())
        .await;
    assert_eq!(outcome, PushOutcome::Sent, "{:?}", apns.refused());
    let pushed = apns.pushes().pop().expect("a push");
    assert_eq!(pushed.token, "a1b2c3d4e5f6a7b8c9d0");
    assert_eq!(pushed.topic.as_deref(), Some(APNS_TOPIC));
    assert_eq!(pushed.headers["apns-push-type"], "voip");
    assert_eq!(pushed.headers["apns-priority"], "10");
    assert_eq!(pushed.headers["apns-expiration"], "0");
    assert_eq!(pushed.headers["apns-collapse-id"], "call-0f3a");
    assert_eq!(pushed.kind(), "call");
    assert_eq!(pushed.payload["call"], "call-0f3a");
    assert_eq!(pushed.payload["from"], "1002");
    assert_eq!(pushed.payload["name"], "Ann Lee");
    assert_eq!(pushed.payload["tenant"], "tenant-1");

    // The provider token is kept: a second push signs nothing new, and is
    // taken as well.
    let again = sender
        .send(
            &StandInApns::address("a1b2c3d4e5f6a7b8c9d0"),
            &PushMessage::new(PushKind::Cancel, "call-0f3a", "tenant-1"),
        )
        .await;
    assert_eq!(again, PushOutcome::Sent);
    assert_eq!(apns.pushes().len(), 2);
    assert!(apns.refused().is_empty());
}

#[tokio::test]
async fn a_token_apns_no_longer_knows_is_gone() {
    let (sender, apns, _) = sender(PushPayload::Full).await;
    apns.forget("deadbeefdeadbeefdead");
    let outcome = sender
        .send(&StandInApns::address("deadbeefdeadbeefdead"), &ringing())
        .await;
    assert!(matches!(outcome, PushOutcome::Gone(_)), "{outcome:?}");
}

#[tokio::test]
async fn a_call_goes_to_fcm_as_a_high_priority_data_message() {
    let (sender, _, fcm) = sender(PushPayload::Full).await;
    let outcome = sender
        .send(&StandInFcm::address("dA1b2C3d4E5f6G7h8I9j"), &ringing())
        .await;
    assert_eq!(outcome, PushOutcome::Sent, "{:?}", fcm.refused());
    let pushed = fcm.pushes().pop().expect("a push");
    assert_eq!(pushed.token, "dA1b2C3d4E5f6G7h8I9j");
    assert_eq!(pushed.android.as_ref().unwrap()["priority"], "high");
    // A data message's values are strings.
    assert_eq!(pushed.payload["type"], "call");
    assert_eq!(pushed.payload["from"], "1002");
    assert!(pushed.payload["sent"].is_string());

    // The access token is kept for the next push.
    sender
        .send(&StandInFcm::address("dA1b2C3d4E5f6G7h8I9j"), &ringing())
        .await;
    assert_eq!(fcm.pushes().len(), 2);
    assert!(fcm.refused().is_empty());
}

#[tokio::test]
async fn a_token_fcm_no_longer_knows_is_gone() {
    let (sender, _, fcm) = sender(PushPayload::Full).await;
    fcm.forget("gone-token-0123456789");
    let outcome = sender
        .send(&StandInFcm::address("gone-token-0123456789"), &ringing())
        .await;
    assert!(matches!(outcome, PushOutcome::Gone(_)), "{outcome:?}");
}

#[tokio::test]
async fn a_minimal_payload_keeps_the_caller_off_the_providers() {
    let (sender, apns, fcm) = sender(PushPayload::Minimal).await;
    sender
        .send(&StandInApns::address("a1b2c3d4e5f6a7b8c9d0"), &ringing())
        .await;
    sender
        .send(&StandInFcm::address("dA1b2C3d4E5f6G7h8I9j"), &ringing())
        .await;
    for pushed in [apns.pushes().pop().unwrap(), fcm.pushes().pop().unwrap()] {
        assert!(pushed.payload.get("from").is_none(), "{pushed:?}");
        assert!(pushed.payload.get("name").is_none(), "{pushed:?}");
        assert_eq!(pushed.payload["call"], "call-0f3a");
    }
}

#[tokio::test]
async fn an_app_this_node_has_no_key_for_is_not_sent() {
    let (sender, apns, _) = sender(PushPayload::Full).await;
    let mut other = StandInApns::address("a1b2c3d4e5f6a7b8c9d0");
    other.param = "OTHERTEAM1.com.example.anvil.voip".into();
    let outcome = sender.send(&other, &ringing()).await;
    assert!(matches!(outcome, PushOutcome::Failed(_)), "{outcome:?}");
    assert!(apns.pushes().is_empty());
    let text = fcp_push::PushCounters::global().prometheus("");
    assert!(text.contains("outcome=\"failed\""), "{text}");
}
