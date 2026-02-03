// Helpers used by multiple test files — some may be unused in each individual binary.
#![allow(dead_code, unused_imports)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use sip_core::{Headers, Method, Request, RequestLine, SipUri};
use sip_transaction::{TransactionManager, TransportContext, TransportDispatcher, TransportKind};
use smol_str::SmolStr;
use tempfile::TempDir;

use fcp_b2bua_core::{B2buaConfig, CallManager};
use fcp_core::{FileBackedStorage, RegistrationStore};
use fcp_sip::handler::SipMessageHandler;
use fcp_sip::listener::InboundMessageHandler;
use fcp_sip::registrar::FcpRegistrar;
use sip_dialog::DialogManager;

// ---------------------------------------------------------------------------
// TestDispatcher — captures outbound messages instead of sending via network
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct TestDispatcher {
    pub sent: tokio::sync::Mutex<Vec<(TransportKind, Bytes)>>,
}

#[async_trait]
impl TransportDispatcher for TestDispatcher {
    async fn dispatch(&self, ctx: &TransportContext, payload: Bytes) -> anyhow::Result<()> {
        self.sent.lock().await.push((ctx.transport(), payload));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TestHarness — builder-pattern wrapper around SipMessageHandler
// ---------------------------------------------------------------------------

pub struct TestHarness {
    pub handler: SipMessageHandler,
    pub dispatcher: Arc<TestDispatcher>,
    pub storage: Option<Arc<FileBackedStorage>>,
    pub call_manager: Arc<CallManager>,
    _temp_dir: Option<TempDir>,
}

pub struct TestHarnessBuilder {
    with_registrar: bool,
    with_storage: bool,
    pbx_config: Option<fcp_pbx::PbxConfig>,
}

impl TestHarness {
    pub fn builder() -> TestHarnessBuilder {
        TestHarnessBuilder {
            with_registrar: false,
            with_storage: false,
            pbx_config: None,
        }
    }

    /// Feed a serialised SIP request into the full handler pipeline.
    pub async fn send_request(&self, request: &Request) {
        let payload = sip_parse::serialize_request(request);
        let peer: SocketAddr = "127.0.0.1:15060".parse().unwrap();
        self.handler
            .handle_inbound(TransportKind::Udp, peer, payload, None)
            .await
            .ok();
    }

    /// Parse the last outbound message as a `Response`. Returns `None` when
    /// the buffer is empty or the bytes are not a valid response.
    pub async fn last_response(&self) -> Option<sip_core::Response> {
        let sent = self.dispatcher.sent.lock().await;
        sent.last()
            .and_then(|(_, bytes)| sip_parse::parse_response(bytes))
    }

    /// Number of outbound messages captured so far.
    pub async fn sent_count(&self) -> usize {
        self.dispatcher.sent.lock().await.len()
    }

    /// Parse and return all captured responses, clearing the buffer.
    pub async fn drain_responses(&self) -> Vec<sip_core::Response> {
        let mut sent = self.dispatcher.sent.lock().await;
        let responses: Vec<_> = sent
            .drain(..)
            .filter_map(|(_, bytes)| sip_parse::parse_response(&bytes))
            .collect();
        responses
    }

    /// Poll until `count` messages captured or timeout, then return all responses.
    pub async fn wait_for_responses(
        &self,
        count: usize,
        timeout: Duration,
    ) -> Vec<sip_core::Response> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.sent_count().await >= count {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.drain_responses().await
    }

    /// Shortcut: build a REGISTER, call `handle_register()` directly, return
    /// the `Response` object without going through the transport pipeline.
    pub async fn register_device(&self, aor: &str, contact: &str) -> sip_core::Response {
        let request = build_register_request(aor, contact);
        // handle_register is private, so we go through the full pipeline and
        // capture the response from the dispatcher.
        self.send_request(&request).await;
        // REGISTER is synchronous (not spawned), so the response is immediate.
        self.last_response().await.expect("expected a response")
    }
}

impl TestHarnessBuilder {
    pub fn with_registrar(mut self) -> Self {
        self.with_registrar = true;
        self.with_storage = true;
        self
    }

    pub fn with_storage(mut self) -> Self {
        self.with_storage = true;
        self
    }

    pub fn with_pbx(mut self, config: fcp_pbx::PbxConfig) -> Self {
        self.pbx_config = Some(config);
        self
    }

    pub async fn build(self) -> TestHarness {
        let dispatcher = Arc::new(TestDispatcher::default());
        let transaction_manager =
            Arc::new(TransactionManager::new(dispatcher.clone() as Arc<dyn TransportDispatcher>));
        let dialog_manager = Arc::new(DialogManager::new());
        let local_uri = SipUri::parse("sip:test@127.0.0.1").unwrap();
        let call_manager = Arc::new(CallManager::new());

        let mut temp_dir: Option<TempDir> = None;
        let mut storage: Option<Arc<FileBackedStorage>> = None;
        let mut registrar: Option<Arc<FcpRegistrar>> = None;

        if self.with_storage || self.with_registrar {
            let td = TempDir::new().expect("Failed to create temp dir");
            let fs = Arc::new(
                FileBackedStorage::new(td.path().to_path_buf())
                    .await
                    .expect("Failed to create FileBackedStorage"),
            );
            if self.with_registrar {
                registrar = Some(Arc::new(FcpRegistrar::new(fs.clone(), None, None)));
            }
            storage = Some(fs);
            temp_dir = Some(td);
        }

        let pbx_handler = self.pbx_config.map(|cfg| Arc::new(fcp_pbx::PbxHandler::new(cfg)));

        let handler = SipMessageHandler::new(
            transaction_manager,
            dialog_manager,
            local_uri,
            None,                      // media_profile
            "127.0.0.1".to_string(),   // local_addr
            10000,                     // rtp_port
            registrar,                 // registrar
            None,                      // routing_service
            call_manager.clone(),      // call_manager
            None,                      // uac
            None,                      // storage (DynStorage, not FileBackedStorage)
            None,                      // media_manager
            None,                      // media_config
            None,                      // flow_handler
            B2buaConfig::default(),    // b2bua_config
            None,                      // outbound_route_provider
            pbx_handler,               // pbx_handler
        );

        TestHarness {
            handler,
            dispatcher,
            storage,
            call_manager,
            _temp_dir: temp_dir,
        }
    }
}

// ---------------------------------------------------------------------------
// Request builder helpers
// ---------------------------------------------------------------------------

/// Build a REGISTER request with the given AOR and contact.
pub fn build_register_request(aor: &str, contact: &str) -> Request {
    let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
    let call_id = format!("reg-{}@test", uuid::Uuid::new_v4().simple());
    let tag = uuid::Uuid::new_v4().simple().to_string();

    let mut headers = Headers::new();
    let _ = headers.push(
        SmolStr::new("Via"),
        SmolStr::new(format!("SIP/2.0/UDP 127.0.0.1:15060;branch={}", branch)),
    );
    let _ = headers.push(
        SmolStr::new("From"),
        SmolStr::new(format!("<sip:{}>;tag={}", aor, tag)),
    );
    let _ = headers.push(
        SmolStr::new("To"),
        SmolStr::new(format!("<sip:{}>", aor)),
    );
    let _ = headers.push(SmolStr::new("Call-ID"), SmolStr::new(&call_id));
    let _ = headers.push(SmolStr::new("CSeq"), SmolStr::new("1 REGISTER"));
    let _ = headers.push(SmolStr::new("Max-Forwards"), SmolStr::new("70"));
    let _ = headers.push(
        SmolStr::new("Contact"),
        SmolStr::new(format!("<sip:{}>;expires=3600", contact)),
    );
    let _ = headers.push(SmolStr::new("Expires"), SmolStr::new("3600"));
    let _ = headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

    // The Request-URI for REGISTER is the registrar URI (domain)
    let domain = aor.split('@').last().unwrap_or("example.com");
    Request::new(
        RequestLine::new(
            Method::Register,
            SipUri::parse(&format!("sip:{}", domain)).unwrap(),
        ),
        headers,
        Bytes::new(),
    )
    .expect("Failed to create REGISTER request")
}

/// Build an INVITE request from `from_user` to `to_user` with optional SDP body.
pub fn build_invite_request(from: &str, to: &str) -> Request {
    build_invite_with_sdp(from, to, None)
}

/// Build an INVITE request with a specific SDP body (or `None` for empty body).
pub fn build_invite_with_sdp(from: &str, to: &str, sdp: Option<&str>) -> Request {
    let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
    let call_id = format!("call-{}@test", uuid::Uuid::new_v4().simple());
    let tag = uuid::Uuid::new_v4().simple().to_string();

    let mut headers = Headers::new();
    let _ = headers.push(
        SmolStr::new("Via"),
        SmolStr::new(format!("SIP/2.0/UDP 127.0.0.1:15060;branch={}", branch)),
    );
    let _ = headers.push(
        SmolStr::new("From"),
        SmolStr::new(format!("<sip:{}>;tag={}", from, tag)),
    );
    let _ = headers.push(
        SmolStr::new("To"),
        SmolStr::new(format!("<sip:{}>", to)),
    );
    let _ = headers.push(SmolStr::new("Call-ID"), SmolStr::new(&call_id));
    let _ = headers.push(SmolStr::new("CSeq"), SmolStr::new("1 INVITE"));
    let _ = headers.push(SmolStr::new("Max-Forwards"), SmolStr::new("70"));
    let _ = headers.push(
        SmolStr::new("Contact"),
        SmolStr::new(format!("<sip:{}>", from)),
    );

    let body = if let Some(sdp_str) = sdp {
        let _ = headers.push(
            SmolStr::new("Content-Type"),
            SmolStr::new("application/sdp"),
        );
        let _ = headers.push(
            SmolStr::new("Content-Length"),
            SmolStr::new(sdp_str.len().to_string()),
        );
        Bytes::from(sdp_str.to_string())
    } else {
        let _ = headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));
        Bytes::new()
    };

    // Extract user part for Request-URI
    let to_user = to.split('@').next().unwrap_or(to);
    let to_domain = to.split('@').last().unwrap_or("example.com");

    Request::new(
        RequestLine::new(
            Method::Invite,
            SipUri::parse(&format!("sip:{}@{}", to_user, to_domain)).unwrap(),
        ),
        headers,
        body,
    )
    .expect("Failed to create INVITE request")
}

/// Build an OPTIONS request to the given target URI.
pub fn build_options_request(target: &str) -> Request {
    let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
    let call_id = format!("opt-{}@test", uuid::Uuid::new_v4().simple());
    let tag = uuid::Uuid::new_v4().simple().to_string();

    let mut headers = Headers::new();
    let _ = headers.push(
        SmolStr::new("Via"),
        SmolStr::new(format!("SIP/2.0/UDP 127.0.0.1:15060;branch={}", branch)),
    );
    let _ = headers.push(
        SmolStr::new("From"),
        SmolStr::new(format!("<sip:probe@test.local>;tag={}", tag)),
    );
    let _ = headers.push(
        SmolStr::new("To"),
        SmolStr::new(format!("<sip:{}>", target)),
    );
    let _ = headers.push(SmolStr::new("Call-ID"), SmolStr::new(&call_id));
    let _ = headers.push(SmolStr::new("CSeq"), SmolStr::new("1 OPTIONS"));
    let _ = headers.push(SmolStr::new("Max-Forwards"), SmolStr::new("70"));
    let _ = headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

    Request::new(
        RequestLine::new(
            Method::Options,
            SipUri::parse(&format!("sip:{}", target)).unwrap(),
        ),
        headers,
        Bytes::new(),
    )
    .expect("Failed to create OPTIONS request")
}

/// Build a BYE request for an established dialog.
pub fn build_bye_request(call_id: &str, from_tag: &str, to_tag: &str) -> Request {
    let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());

    let mut headers = Headers::new();
    let _ = headers.push(
        SmolStr::new("Via"),
        SmolStr::new(format!("SIP/2.0/UDP 127.0.0.1:15060;branch={}", branch)),
    );
    let _ = headers.push(
        SmolStr::new("From"),
        SmolStr::new(format!(
            "<sip:alice@example.com>;tag={}",
            from_tag
        )),
    );
    let _ = headers.push(
        SmolStr::new("To"),
        SmolStr::new(format!(
            "<sip:bob@example.com>;tag={}",
            to_tag
        )),
    );
    let _ = headers.push(SmolStr::new("Call-ID"), SmolStr::new(call_id));
    let _ = headers.push(SmolStr::new("CSeq"), SmolStr::new("2 BYE"));
    let _ = headers.push(SmolStr::new("Max-Forwards"), SmolStr::new("70"));
    let _ = headers.push(SmolStr::new("Content-Length"), SmolStr::new("0"));

    Request::new(
        RequestLine::new(
            Method::Bye,
            SipUri::parse("sip:bob@example.com").unwrap(),
        ),
        headers,
        Bytes::new(),
    )
    .expect("Failed to create BYE request")
}

/// Minimal valid SDP with an audio m-line.
pub fn minimal_sdp() -> &'static str {
    "v=0\r\n\
     o=- 0 0 IN IP4 127.0.0.1\r\n\
     s=-\r\n\
     c=IN IP4 127.0.0.1\r\n\
     t=0 0\r\n\
     m=audio 4000 RTP/AVP 0\r\n"
}

/// SDP that is missing the audio m-line.
pub fn sdp_no_audio() -> &'static str {
    "v=0\r\n\
     o=- 0 0 IN IP4 127.0.0.1\r\n\
     s=-\r\n\
     c=IN IP4 127.0.0.1\r\n\
     t=0 0\r\n"
}
