# Voicemail Implementation Plan - Phase 1 Addition

## Overview

Add voicemail functionality to Phase 1 MVP to provide a complete PBX experience. Without voicemail, callers can't leave messages when extensions don't answer - this is a fundamental PBX feature.

**Timeline:** 1-2 weeks
**Dependencies:** Recording implementation (media backend)
**Priority:** HIGH - Essential for Phase 1 completion

---

## What Already Exists ✅

### Types & Data Models (fcp-core/src/voicemail.rs)
- ✅ `VoicemailMessage` - Message metadata and storage
- ✅ `VoicemailMailbox` - Mailbox configuration
- ✅ `VoicemailGreeting` - Greeting management (unavailable/busy/temporary)
- ✅ `MessageStatus` - New/Heard/Saved/Deleted
- ✅ `MessagePriority` - Normal/Urgent
- ✅ `GreetingType` - Unavailable/Busy/Name/Temporary

### Storage Trait (fcp-core/src/storage.rs)
- ✅ `VoicemailStore` trait defined with methods:
  - `get_mailbox()`, `get_mailbox_by_extension()`, `get_mailbox_by_user()`
  - `list_mailboxes()`, `put_mailbox()`, `delete_mailbox()`
  - `get_message()`, `list_messages()`, `list_messages_by_status()`
  - `put_message()`, `delete_message()`, `count_messages()`

### Media Recording
- ✅ `MediaManager::start_recording()` exists
- ✅ `MediaManager::stop_recording()` exists
- ✅ `MediaBackend::start_recording()` trait method
- ⚠️  Implementation may be stubbed - needs verification

### Flow Nodes
- ✅ `StartRecording` node exists
- ✅ `StopRecording` node exists
- ❌ No `Voicemail` node (needs creation)

---

## What Needs Implementation ❌

### 1. Storage Layer (HIGH Priority)

**Files:**
- `crates/fcp-storage-sqlite/src/lib.rs`
- `crates/fcp-storage-postgres/src/lib.rs`
- `crates/fcp-storage-mysql/src/lib.rs`
- `crates/fcp-storage-mongodb/src/lib.rs`

**Tasks:**
- [ ] Create `voicemail_mailboxes` table schema
  ```sql
  CREATE TABLE voicemail_mailboxes (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    line_binding_id TEXT NOT NULL,
    extension TEXT NOT NULL,
    pin TEXT NOT NULL,  -- hashed
    enabled BOOLEAN DEFAULT true,
    max_message_duration INTEGER DEFAULT 300,
    max_messages INTEGER DEFAULT 100,
    email_notification BOOLEAN DEFAULT false,
    email_address TEXT,
    attach_audio BOOLEAN DEFAULT false,
    include_transcription BOOLEAN DEFAULT false,
    transcription_enabled BOOLEAN DEFAULT false,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(tenant_id, extension)
  );
  ```

- [ ] Create `voicemail_messages` table schema
  ```sql
  CREATE TABLE voicemail_messages (
    id TEXT PRIMARY KEY,
    mailbox_id TEXT NOT NULL REFERENCES voicemail_mailboxes(id),
    caller TEXT NOT NULL,
    caller_name TEXT,
    status TEXT NOT NULL CHECK(status IN ('new', 'heard', 'saved', 'deleted')),
    priority TEXT NOT NULL CHECK(priority IN ('normal', 'urgent')),
    duration INTEGER NOT NULL,
    recording_path TEXT NOT NULL,
    transcription TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    accessed_at TIMESTAMP
  );
  ```

- [ ] Create `voicemail_greetings` table schema
  ```sql
  CREATE TABLE voicemail_greetings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mailbox_id TEXT NOT NULL REFERENCES voicemail_mailboxes(id),
    greeting_type TEXT NOT NULL CHECK(greeting_type IN ('unavailable', 'busy', 'name', 'temporary')),
    audio_path TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP
  );
  ```

- [ ] Implement all `VoicemailStore` methods in all backends
- [ ] Add database migrations
- [ ] Add indexes for performance

**Estimated Time:** 2-3 days

---

### 2. Voicemail Service (HIGH Priority)

**New File:** `crates/fcp-voicemail/src/lib.rs`

**Tasks:**
- [ ] Create `fcp-voicemail` crate
- [ ] Implement `VoicemailService` struct:
  ```rust
  pub struct VoicemailService {
      storage: Arc<dyn VoicemailStore>,
      media_manager: Arc<MediaManager>,
      recordings_dir: PathBuf,
      greetings_dir: PathBuf,
  }

  impl VoicemailService {
      /// Play greeting and record message
      pub async fn record_message(
          &self,
          call_id: &CallId,
          mailbox_id: &str,
          caller: String,
          is_busy: bool,
      ) -> Result<VoicemailMessage>;

      /// Retrieve messages for playback
      pub async fn get_messages(
          &self,
          mailbox_id: &str,
          status: MessageStatus,
      ) -> Result<Vec<VoicemailMessage>>;

      /// Play a message
      pub async fn play_message(
          &self,
          call_id: &CallId,
          message_id: &str,
      ) -> Result<()>;

      /// Delete a message
      pub async fn delete_message(&self, message_id: &str) -> Result<()>;

      /// Send MWI notification
      pub async fn send_mwi(&self, mailbox_id: &str) -> Result<()>;
  }
  ```

- [ ] Implement greeting playback
- [ ] Implement message recording with max duration
- [ ] Implement beep before recording
- [ ] Implement message storage to disk
- [ ] Implement MWI (Message Waiting Indicator) via SIP NOTIFY
- [ ] Add configurable system greetings (default audio files)

**Estimated Time:** 3-4 days

---

### 3. Flow Engine Integration (MEDIUM Priority)

**File:** `crates/fcp-flow/src/schema.rs`

**Tasks:**
- [ ] Add `Voicemail` node type:
  ```rust
  Voicemail {
      /// Extension or mailbox to leave message for
      mailbox: String,

      /// Play unavailable or busy greeting
      #[serde(default)]
      is_busy: bool,

      /// Maximum recording duration (seconds)
      #[serde(default = "default_max_duration")]
      max_duration: u32,

      /// Play beep before recording
      #[serde(default = "default_true")]
      play_beep: bool,

      /// Node to go to after leaving message
      next_on_success: String,

      /// Node to go to on error (e.g., mailbox full)
      next_on_error: String,
  }
  ```

**File:** `crates/fcp-flow/src/engine.rs`

**Tasks:**
- [ ] Implement `execute_voicemail()` function
- [ ] Integrate with VoicemailService
- [ ] Set flow variables:
  - `voicemail_status`: "success" | "error" | "mailbox_full"
  - `voicemail_message_id`: message ID
  - `voicemail_duration`: recording duration

**Estimated Time:** 1 day

---

### 4. SIP Integration - MWI (MEDIUM Priority)

**File:** `crates/fcp-sip/src/handler.rs`

**Tasks:**
- [ ] Implement SIP NOTIFY for MWI (Message-Account header)
- [ ] Send MWI when new message arrives
- [ ] Clear MWI when all messages are heard
- [ ] Track subscriptions to MWI notifications
- [ ] Handle SUBSCRIBE requests for "message-summary" event

**MWI NOTIFY Example:**
```
NOTIFY sip:alice@192.168.1.100:5060 SIP/2.0
Event: message-summary
Subscription-State: active
Content-Type: application/simple-message-summary

Messages-Waiting: yes
Message-Account: sip:alice@pbx.example.com
Voice-Message: 2/5 (2/3)
```

**Estimated Time:** 2 days

---

### 5. REST API Endpoints (MEDIUM Priority)

**New File:** `crates/fcp-server/src/api/v1/voicemail.rs`

**Tasks:**
- [ ] Implement REST endpoints:
  ```
  POST   /api/v1/voicemail/mailboxes              - Create mailbox
  GET    /api/v1/voicemail/mailboxes              - List mailboxes
  GET    /api/v1/voicemail/mailboxes/:id          - Get mailbox
  PUT    /api/v1/voicemail/mailboxes/:id          - Update mailbox
  DELETE /api/v1/voicemail/mailboxes/:id          - Delete mailbox

  GET    /api/v1/voicemail/mailboxes/:id/messages - List messages
  GET    /api/v1/voicemail/messages/:id           - Get message
  PUT    /api/v1/voicemail/messages/:id           - Update message status
  DELETE /api/v1/voicemail/messages/:id           - Delete message
  GET    /api/v1/voicemail/messages/:id/audio     - Download audio

  POST   /api/v1/voicemail/mailboxes/:id/greetings - Upload greeting
  GET    /api/v1/voicemail/mailboxes/:id/greetings - List greetings
  DELETE /api/v1/voicemail/greetings/:id           - Delete greeting
  ```

- [ ] Add authentication/authorization
- [ ] Add tenant isolation
- [ ] Add pagination for message lists

**Estimated Time:** 2 days

---

### 6. IVR Voicemail Access (MEDIUM Priority)

**File:** `crates/fcp-flow/src/schema.rs`

**Tasks:**
- [ ] Add `VoicemailMenu` node for checking messages:
  ```rust
  VoicemailMenu {
      /// Mailbox to access
      mailbox: String,

      /// Require PIN authentication
      #[serde(default = "default_true")]
      require_pin: bool,

      /// Allow DTMF navigation
      /// 1 = Listen to new messages
      /// 2 = Listen to saved messages
      /// 3 = Advanced options
      /// * = Main menu
      /// # = Exit
      menu_options: HashMap<char, String>,  // digit -> node_id
  }
  ```

- [ ] Implement DTMF menu navigation
- [ ] Implement PIN authentication
- [ ] Implement message playback controls:
  - `1` = Replay message
  - `2` = Save message
  - `3` = Delete message
  - `4` = Reply to message (call back)
  - `*` = Return to menu
  - `#` = Next message

**Estimated Time:** 2-3 days

---

### 7. Call Routing Integration (HIGH Priority)

**File:** `crates/fcp-sip/src/handler.rs`

**Tasks:**
- [ ] Integrate voicemail forwarding on no-answer
- [ ] Check `LineBinding.forward_on_no_answer` → voicemail
- [ ] Check `LineBinding.forward_on_busy` → voicemail
- [ ] Auto-forward to voicemail after ring timeout
- [ ] Set `is_busy` flag correctly based on call state

**File:** `crates/fcp-core/src/line_binding.rs`

**Tasks:**
- [ ] Link `LineBinding` to `VoicemailMailbox`
- [ ] Add `voicemail_enabled` flag
- [ ] Add `voicemail_box_id` field (already exists)

**Estimated Time:** 1 day

---

### 8. CLI Integration (LOW Priority)

**File:** `crates/fcp-cli/src/tui/tabs/voicemail.rs`

**Tasks:**
- [ ] Add "Voicemail" tab to TUI
- [ ] Display mailboxes with message counts
- [ ] Display messages (new/heard/saved)
- [ ] Allow message playback from CLI (if audio device available)
- [ ] Allow message deletion

**Estimated Time:** 1 day

---

### 9. Testing (HIGH Priority)

**Tasks:**
- [ ] Unit tests for VoicemailService
- [ ] Integration tests for storage layer
- [ ] Flow engine tests with Voicemail node
- [ ] End-to-end test: Call extension, leave voicemail, retrieve message
- [ ] MWI notification test
- [ ] Voicemail menu navigation test

**Estimated Time:** 1-2 days

---

## Implementation Order

### Week 1: Core Functionality
1. **Day 1-2**: Storage layer (database schema + VoicemailStore implementation)
2. **Day 3-5**: VoicemailService (greeting playback, recording, storage)
3. **Day 6-7**: Flow integration (Voicemail node + execute logic)

### Week 2: Integration & Features
4. **Day 8-9**: SIP MWI integration (NOTIFY support)
5. **Day 10**: Call routing integration (forward to voicemail)
6. **Day 11-12**: REST API endpoints
7. **Day 13**: IVR voicemail menu (basic)
8. **Day 14**: Testing + bug fixes

**Total: ~2 weeks** (with 1 developer)

---

## Minimal Viable Voicemail (1 Week Option)

If time is constrained, implement this minimal subset:

### Must Have (1 Week)
1. ✅ Storage layer (mailboxes + messages tables)
2. ✅ VoicemailService (record message only, no playback)
3. ✅ Flow integration (Voicemail node)
4. ✅ Call routing (forward on no-answer)
5. ✅ REST API (create mailbox, list messages)

### Defer to Phase 2
- ❌ IVR voicemail menu (access via phone)
- ❌ MWI notifications
- ❌ Greeting management (use default system greeting)
- ❌ Message playback via IVR
- ❌ Email notifications
- ❌ Transcription

**Minimal Viable: 1 week** (record and view via API only)

---

## Dependencies & Prerequisites

### Before Starting
- [ ] Verify recording implementation works (MediaBackend)
- [ ] Test audio file storage and retrieval
- [ ] Decide on recordings directory structure: `/var/fcp/voicemail/{tenant_id}/{mailbox_id}/messages/`
- [ ] Decide on greetings directory: `/var/fcp/voicemail/{tenant_id}/{mailbox_id}/greetings/`
- [ ] Choose audio format (WAV for quality, MP3 for size)

### External Dependencies
- None - all functionality is internal

---

## Configuration

**Add to fcp.toml:**
```toml
[voicemail]
enabled = true
recordings_dir = "/var/fcp/voicemail"
max_message_duration = 300  # 5 minutes
max_messages_per_mailbox = 100
default_greeting = "/var/fcp/audio/default-greeting.wav"
play_beep = true
email_notifications = false
transcription_enabled = false
```

---

## Success Criteria

### Phase 1 MVP Voicemail Complete When:
- [ ] User calls extension, no answer, hears greeting, leaves message
- [ ] Message stored to disk with metadata in database
- [ ] REST API can list and retrieve messages
- [ ] Flow engine Voicemail node works
- [ ] Multi-tenant isolation enforced
- [ ] Basic tests pass
- [ ] Documentation updated

### Stretch Goals (Phase 2):
- [ ] IVR menu for checking messages
- [ ] MWI notifications to phones
- [ ] Email notifications with audio attachment
- [ ] Transcription via Deepgram/Whisper
- [ ] Web UI for voicemail management

---

## Risk Assessment

### Low Risk
- Storage layer (well-defined trait, schema is straightforward)
- REST API (follows existing patterns)
- Flow integration (similar to existing nodes)

### Medium Risk
- Recording implementation (may need forge-media fixes)
- MWI notifications (SIP NOTIFY can be tricky)
- Greeting playback (audio streaming to call)

### High Risk
- IVR voicemail menu (complex DTMF navigation state machine)
- Transcription (external API integration, async processing)

---

## Recommendation

**Option 1: Full Voicemail (2 weeks)** ✅ Recommended
- Complete voicemail implementation
- IVR access + MWI notifications
- Production-ready for Phase 1
- Extends Phase 1 timeline by 2 weeks

**Option 2: Minimal Voicemail (1 week)** ⚠️ Acceptable
- Record and view via API only
- No IVR access or MWI
- Ship Phase 1 faster
- Add IVR/MWI in Phase 2

**Decision Point:** Choose based on Phase 1 completion urgency. Full implementation recommended for complete PBX experience.
