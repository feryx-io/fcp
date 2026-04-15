
● Updated plan
  ⎿  /plan to preview

● User approved Claude's plan
  ⎿  Plan saved to: ~/.claude/plans/elegant-toasting-hickey.md · /plan to edit
     Plan: Dedicated Shared TTS Service (fcp-tts)

     Context

     Currently 4 services (call-manager, conference, admin, IVR) each spawn their own Kokoro
      TTS subprocess — ~86s model load and ~500MB RAM each (~2GB total, ~6 min cumulative
     startup). A dedicated TTS service loads the model once and serves all clients via HTTP.

     Architecture

     ┌─────────────┐  ┌──────────────┐  ┌───────────┐  ┌─────────┐
     │call-manager  │  │  conference  │  │    IVR    │  │  admin  │
     │RemoteTts     │  │RemoteTts     │  │RemoteTts  │  │RemoteTts│
     └──────┬───────┘  └──────┬───────┘  └─────┬─────┘  └────┬────┘
            │                 │                │              │
            └────────────┬────┴────────────────┴──────────────┘
                         │  HTTP POST /v1/synthesize
                   ┌─────▼──────┐
                   │  fcp-tts   │  ← single Kokoro subprocess
                   │  :8093     │  ← shared LRU cache
                   │  (GPU node)│  ← multi-provider routing
                   └────────────┘

     Phase 1: Core Service (fcp-tts-server binary)

     New files to create

     ┌─────────────────────────────────────┬────────────────────────────────────────────────
     ┐
     │                File                 │                    Purpose
     │
     ├─────────────────────────────────────┼────────────────────────────────────────────────
     ┤
     │ crates/fcp-tts-server/Cargo.toml    │ Crate manifest ([[bin]] name = "fcp-tts")
     │
     ├─────────────────────────────────────┼────────────────────────────────────────────────
     ┤
     │ crates/fcp-tts-server/src/main.rs   │ Entry point — follow
     │
     │                                     │ fcp-events-server/src/main.rs pattern
     │
     ├─────────────────────────────────────┼────────────────────────────────────────────────
     ┤
     │ crates/fcp-tts-server/src/cli.rs    │ Clap CLI (--config, --log-level, --bind)
     │
     ├─────────────────────────────────────┼────────────────────────────────────────────────
     ┤
     │                                     │ TOML config: ServiceSection, HttpSection,
     │
     │ crates/fcp-tts-server/src/config.rs │ AuthSection, TtsSection, ClusterSection,
     │
     │                                     │ LoggingSection
     │
     ├─────────────────────────────────────┼────────────────────────────────────────────────
     ┤
     │ crates/fcp-tts-server/src/state.rs  │ AppState: providers map, TtsService, usage
     │
     │                                     │ counters
     │
     ├─────────────────────────────────────┼────────────────────────────────────────────────
     ┤
     │ crates/fcp-tts-server/src/routes.rs │ Axum handlers for
     │
     │                                     │ synthesis/voices/cache/health
     │
     ├─────────────────────────────────────┼────────────────────────────────────────────────
     ┤
     │ fcp-tts-local.toml                  │ Local dev config (port 8093, Kokoro provider)
     │
     └─────────────────────────────────────┴────────────────────────────────────────────────
     ┘

     Modify

     ┌──────────────────────┬──────────────────────────────────────────────────┐
     │         File         │                      Change                      │
     ├──────────────────────┼──────────────────────────────────────────────────┤
     │ Cargo.toml (root)    │ Add "crates/fcp-tts-server" to workspace members │
     ├──────────────────────┼──────────────────────────────────────────────────┤
     │ scripts/start-all.sh │ Add fcp-tts to service list                      │
     └──────────────────────┴──────────────────────────────────────────────────┘

     HTTP API

     - POST /v1/synthesize — {text, language, voice, provider?, tenant_id?} → WAV bytes
     (audio/wav)
     - GET /v1/voices?provider=&language= → voice list JSON
     - GET /v1/providers → provider names + status
     - GET /v1/cache/stats → entry count, size, max
     - DELETE /v1/cache → clear cache
     - GET /health → provider health (unprotected)

     Config structure (fcp-tts-local.toml)

     [service]
     service_type = "tts"
     node_id = "tts-local"

     [http]
     bind_address = "0.0.0.0:8093"

     [auth]
     tokens = ["tts-local-secret"]

     [tts]
     default_provider = "kokoro"
     cache_dir = "/tmp/fcp/tts_cache"
     cache_max_size_mb = 1000
     default_language = "en-US"

     [[tts.providers]]
     name = "kokoro"
     type = "local"
     engine = "kokoro"
     python_path = ".venv/bin/python3"
     # GPU acceleration for local models (Kokoro, etc.)
     # device = "auto"  # "auto" | "cuda" | "cuda:0" | "cuda:1" | "cpu"
     #                   # auto = cuda if available, else cpu

     [logging]
     level = "info"
     format = "pretty"

     [cluster]
     enabled = true
     redis_url = "redis://localhost:6379"

     GPU support for local providers

     The kokoro_tts_worker.py currently loads the model on whatever PyTorch device is
     available. The service adds explicit GPU control:

     - Config: Each local provider entry gets an optional device field ("auto", "cuda",
     "cuda:0", "cuda:1", "cpu")
     - Worker protocol: Pass device in the JSONL ready handshake so the Python worker calls
     model.to(device) at load time
     - Health endpoint: Report GPU info (device name, VRAM usage) when running on CUDA
     - Multi-GPU: Run multiple fcp-tts instances, each pinned to a different GPU (cuda:0,
     cuda:1), behind cluster load balancing
     - Deployment: A GPU node runs fcp-tts with device = "cuda"; CPU nodes run all other
     services with provider = "remote" pointing at the GPU node

     The kokoro_tts_worker.py changes are minimal — add a device field to the startup config
      and pass it to KPipeline(lang_code, device=device). PyTorch handles the rest.

     Key reuse

     - fcp-media/src/tts.rs — TtsService + cache logic (used directly, not duplicated)
     - fcp-media/src/tts/providers/ — All provider implementations (local.rs, openai.rs,
     google_cloud.rs)
     - fcp-cluster — ClusterManager for Redis heartbeat registration as node_type = "tts"
     - scripts/kokoro_tts_worker.py — Reused as-is by the Local provider

     Phase 2: Client Integration (RemoteTtsProvider)

     New files

     ┌──────────────────────────────────────────────┬───────────────────────────────────────
     ┐
     │                     File                     │                Purpose
     │
     ├──────────────────────────────────────────────┼───────────────────────────────────────
     ┤
     │ crates/fcp-media/src/tts/providers/remote.rs │ RemoteTtsProvider implementing
     │
     │                                              │ TtsProvider trait
     │
     └──────────────────────────────────────────────┴───────────────────────────────────────
     ┘

     Modify

     ┌──────────────────────────────────────────┬───────────────────────────────────────────
     ┐
     │                   File                   │                  Change
     │
     ├──────────────────────────────────────────┼───────────────────────────────────────────
     ┤
     │ crates/fcp-media/src/tts.rs              │ Add Remote { url, auth_token, ... }
     │
     │                                          │ variant to TtsProviderConfig
     │
     ├──────────────────────────────────────────┼───────────────────────────────────────────
     ┤
     │ crates/fcp-media/src/tts/providers.rs    │ Add mod remote; + Remote arm in
     │
     │                                          │ create_provider()
     │
     ├──────────────────────────────────────────┼───────────────────────────────────────────
     ┤
     │ crates/fcp-media/src/config.rs           │ Allow Remote in validate()
     │
     ├──────────────────────────────────────────┼───────────────────────────────────────────
     ┤
     │ crates/fcp-call-manager/src/main.rs      │ Check for tts_remote_url config, use
     │
     │                                          │ Remote provider if set
     │
     ├──────────────────────────────────────────┼───────────────────────────────────────────
     ┤
     │ crates/fcp-ivr/src/main.rs               │ Same — prefer Remote, fall back to Local
     │
     ├──────────────────────────────────────────┼───────────────────────────────────────────
     ┤
     │ crates/fcp-conference-server/src/main.rs │ Same
     │
     ├──────────────────────────────────────────┼───────────────────────────────────────────
     ┤
     │ crates/fcp-admin/src/main.rs             │ Same
     │
     └──────────────────────────────────────────┴───────────────────────────────────────────
     ┘

     RemoteTtsProvider design

     pub struct RemoteTtsProvider {
         http: reqwest::Client,
         url: String,              // "http://localhost:8093"
         auth_token: String,
         fallback: Option<Arc<dyn TtsProvider>>,  // graceful degradation
     }

     Implements TtsProvider::synthesize() by POSTing to /v1/synthesize. If fcp-tts is down
     and fallback is set, falls back to local synthesis. This is transparent to callers —
     TtsService::synthesize() works the same way.

     Service config additions

     Each consuming service gets optional TTS remote fields:
     [tts]
     provider = "remote"  # or "local" for backward compat
     remote_url = "http://localhost:8093"
     remote_token = "tts-local-secret"

     Phase 3: Multi-Provider + Tenant Tracking

     - Multi-provider: Load multiple providers at startup from [[tts.providers]] array. POST
      /v1/synthesize accepts provider field to select backend.
     - Tenant usage: Track requests/characters/cache-hits per tenant_id. New endpoint GET
     /v1/usage?tenant_id=. Persist counters to Redis for billing.
     - Complete stubs: Finish ElevenLabs, AWS Polly, Azure provider implementations in
     fcp-media/src/tts/providers/.

     Verification

     1. Build: cargo build -p fcp-tts-server
     2. Start: ./target/debug/fcp-tts --config fcp-tts-local.toml
     3. Health: curl http://localhost:8093/health → provider status
     4. Synthesize: curl -X POST http://localhost:8093/v1/synthesize -H "Authorization:
     Bearer tts-local-secret" -H "Content-Type: application/json" -d
     '{"text":"Hello","language":"en-US"}' -o test.wav
     5. Play: aplay test.wav (should hear "Hello")
     6. Cache: curl http://localhost:8093/v1/cache/stats → entry_count=1
     7. Integration: Configure IVR with provider = "remote", call 1000, hear menu prompt
     8. Memory: Verify single Kokoro subprocess (pgrep -c kokoro_tts_worker = 1 vs current
     4)
