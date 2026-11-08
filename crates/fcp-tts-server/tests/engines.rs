//! The cloud engines that were placeholders (#339) — ElevenLabs, Azure and
//! Amazon Polly — through the running server's `/v1/synthesize` and
//! `/v1/voices`, each against the testkit's stand-in for its vendor, which
//! checks what the vendor would: the key, the output format, Polly's
//! signature. Polly once with keys and once with none, reading the
//! machine's role from a stand-in instance metadata service.

use fcp_testkit::service::Service;
use fcp_testkit::smoke::{self, free_port};
use fcp_testkit::tts::{StandInImds, StandInTts, TONE_HZ};
use std::path::Path;

const BIN: &str = env!("CARGO_BIN_EXE_fcp-tts");
const POLLY_KEY: &str = "AKIDSTANDIN";
const POLLY_SECRET: &str = "polly-secret-7f3a";

fn config(http: u16, cache: &Path, eleven: &str, azure: &str, polly: &str, role: &str) -> String {
    format!(
        r#"[http]
bind_address = "127.0.0.1:{http}"

[auth]
tokens = ["tts-token"]

[tts]
default_provider = "eleven"
cache_dir = "{cache}"

[[tts.providers]]
name = "eleven"
type = "elevenlabs"
api_key = "eleven-key"
endpoint = "{eleven}"

[[tts.providers]]
name = "azure"
type = "azure"
api_key = "azure-key"
region = "westeurope"
endpoint = "{azure}"

[[tts.providers]]
name = "polly"
type = "aws_polly"
region = "eu-west-1"
access_key_id = "{POLLY_KEY}"
secret_access_key = "{POLLY_SECRET}"
endpoint = "{polly}"

[[tts.providers]]
name = "polly-wrong"
type = "aws_polly"
region = "eu-west-1"
access_key_id = "{POLLY_KEY}"
secret_access_key = "not-the-secret"
endpoint = "{polly}"

[[tts.providers]]
name = "polly-role"
type = "aws_polly"
region = "eu-west-1"
engine = "standard"
endpoint = "{role}"

[logging]
level = "info"
format = "compact"
"#,
        cache = smoke::toml_path(cache),
    )
}

/// What `/v1/synthesize` answers for `body`: 200 and a WAV at 8 kHz with
/// the stand-ins' tone in it.
async fn speaks(base: &str, body: serde_json::Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/synthesize"))
        .bearer_auth("tts-token")
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(status, 200, "{body}: {}", String::from_utf8_lossy(&bytes));
    let reader = hound::WavReader::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    assert_eq!(reader.spec().sample_rate, 8000, "{body}");
    assert_eq!(reader.spec().channels, 1, "{body}");
    let samples: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
    assert!(
        fcp_testkit::wav::contains_tone(&samples, TONE_HZ as u32),
        "{body}: the vendor's tone, at 8 kHz"
    );
}

async fn voices(base: &str, provider: &str, language: &str) -> Vec<String> {
    let list: serde_json::Value = reqwest::Client::new()
        .get(format!(
            "{base}/v1/voices?provider={provider}&language={language}"
        ))
        .bearer_auth("tts-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let list = list.get("voices").unwrap_or(&list);
    list.as_array()
        .unwrap_or_else(|| panic!("a list of voices: {list}"))
        .iter()
        .map(|v| v["name"].as_str().unwrap_or("").to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn elevenlabs_azure_and_polly_speak_at_8k() {
    let eleven = StandInTts::elevenlabs("eleven-key").await;
    let azure = StandInTts::azure("azure-key").await;
    let polly = StandInTts::polly("eu-west-1", POLLY_KEY, POLLY_SECRET).await;
    let polly_role = StandInTts::polly("eu-west-1", "ASIAROLE", "role-secret").await;
    let imds = StandInImds::start("ASIAROLE", "role-secret", "role-session-token").await;

    let http = free_port();
    let cache = smoke::temp_dir();
    let toml = config(
        http,
        cache.path(),
        eleven.url(),
        azure.url(),
        polly.url(),
        polly_role.url(),
    );
    let server = Service::start_with(
        "fcp-tts",
        BIN,
        &toml,
        &smoke::loopback(http, "/health"),
        &[("AWS_EC2_METADATA_SERVICE_ENDPOINT", imds.url())],
    )
    .await;
    let base = format!("http://127.0.0.1:{http}");

    // ElevenLabs: a voice by name is its id, the v2.5 model told the
    // language, 16 kHz PCM back at 8 kHz.
    speaks(
        &base,
        serde_json::json!({"text": "Press one for sales.", "language": "en-GB", "voice": "Domi"}),
    )
    .await;
    let spoken = eleven.requests_to("/v1/text-to-speech");
    assert_eq!(spoken.len(), 1);
    assert_eq!(spoken[0].path, "/v1/text-to-speech/AZnzlk1XvdvUeBnXmlld");
    assert_eq!(spoken[0].query["output_format"], "pcm_16000");
    assert_eq!(spoken[0].json()["model_id"], "eleven_flash_v2_5");
    assert_eq!(spoken[0].json()["language_code"], "en");
    assert_eq!(voices(&base, "eleven", "en-US").await.len(), 2);

    // Azure: no voice named, so the first neural voice of the language;
    // the text escaped into SSML; an 8 kHz WAV asked for.
    speaks(
        &base,
        serde_json::json!({"text": "Tom & Jerry's line", "language": "en-GB", "provider": "azure"}),
    )
    .await;
    let spoken = azure.requests_to("/cognitiveservices/v1");
    assert_eq!(spoken.len(), 1);
    let ssml = spoken[0].text();
    assert!(
        ssml.contains("xml:lang=\"en-GB\"")
            && ssml
                .contains("<voice name=\"en-GB-SoniaNeural\">Tom &amp; Jerry&apos;s line</voice>"),
        "{ssml}"
    );
    assert_eq!(
        spoken[0].headers["x-microsoft-outputformat"],
        "riff-8khz-16bit-mono-pcm"
    );
    assert_eq!(voices(&base, "azure", "en-GB").await, ["en-GB-SoniaNeural"]);

    // Polly with keys: signed (the stand-in refuses a signature it cannot
    // recompute), the language's voice, PCM at 8 kHz.
    speaks(
        &base,
        serde_json::json!({"text": "Your call is important.", "language": "en-GB", "provider": "polly"}),
    )
    .await;
    let spoken = polly.requests_to("/v1/speech");
    assert_eq!(spoken.len(), 1);
    let speech = spoken[0].json();
    assert_eq!(speech["VoiceId"], "Amy");
    assert_eq!(speech["Engine"], "neural");
    assert_eq!(speech["SampleRate"], "8000");
    assert!(
        !spoken[0].headers.contains_key("x-amz-security-token"),
        "static keys carry no session token"
    );
    assert_eq!(voices(&base, "polly", "en-US").await, ["Joanna"]);

    // A wrong secret signs a request Polly refuses, and the call fails.
    let refused = reqwest::Client::new()
        .post(format!("{base}/v1/synthesize"))
        .bearer_auth("tts-token")
        .json(&serde_json::json!({"text": "x", "voice": "Amy", "provider": "polly-wrong"}))
        .send()
        .await
        .unwrap();
    assert!(refused.status().is_server_error(), "{}", refused.status());
    assert_eq!(polly.requests_to("/v1/speech").len(), 2);

    // Polly with no keys: the instance's role, its session token signed in.
    speaks(
        &base,
        serde_json::json!({"text": "Goodbye.", "language": "en-US", "provider": "polly-role", "voice": "Joanna"}),
    )
    .await;
    let spoken = polly_role.requests_to("/v1/speech");
    assert_eq!(spoken.len(), 1);
    assert_eq!(
        spoken[0].headers["x-amz-security-token"],
        "role-session-token"
    );
    assert!(spoken[0].headers["authorization"].contains("Credential=ASIAROLE/"));
    assert_eq!(spoken[0].json()["Engine"], "standard");

    // No key reached the log.
    let log = server.stop().await;
    for secret in [
        "eleven-key",
        "azure-key",
        POLLY_SECRET,
        "role-secret",
        "role-session-token",
    ] {
        assert!(!log.contains(secret), "{secret} in the log:\n{log}");
    }
}

/// `--check-config` prints the configuration with every key masked.
#[test]
fn the_engines_keys_are_masked_in_the_printed_configuration() {
    let cache = smoke::temp_dir();
    let toml = config(
        free_port(),
        cache.path(),
        "http://127.0.0.1:9",
        "http://127.0.0.1:9",
        "http://127.0.0.1:9",
        "http://127.0.0.1:9",
    );
    let path = cache.path().join("tts.toml");
    std::fs::write(&path, toml).unwrap();
    let out = std::process::Command::new(BIN)
        .arg("--check-config")
        .arg("--config")
        .arg(&path)
        .output()
        .unwrap();
    let printed = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{printed}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for secret in ["eleven-key", "azure-key", POLLY_SECRET] {
        assert!(!printed.contains(secret), "{secret} printed:\n{printed}");
    }
    assert!(
        printed.contains(POLLY_KEY),
        "the access key id is not a secret"
    );
}
