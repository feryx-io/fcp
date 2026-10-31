# Call diagnostics in the app

D8 findings 43 and 44 (2026-10-30), both raised on Anvil:

- **43.** "Call stats: the option to see packets sent, packets received, and call
  quality metrics during a live call."
- **44.** "Pcap call logs. Not for every call, but the ability to enable a capture,
  make a call, have it save the pcap locally. Off by default and only an admin can
  enable it?"

Agreed the same day for 44: **off by default; an admin allows it; the user then
captures; the file is saved locally and also uploaded to FCP**, where the admin can
download it from the device's page.

## 1. What exists

- **Anvil counts its media** (`anvil-core/src/media.rs`, `MediaMetrics`): packets and
  bytes each way, receive drops by reason, and the RFC 3550 jitter estimate. Nothing
  shows them, and nothing is sent: Anvil sends and reads **no RTCP**, so it knows
  nothing of what the far end receives, and nothing of the round trip.
- **FCP's records carry media totals** (`cdrs.rtp_packets_*`, `rtp_packets_lost`,
  `rtp_jitter_ms`) from the node's own legs, not the phone's.
- **FCP already captures SIP on the server**, per tenant (`sip_trace`, kept under the
  retention policy, `docs/COMPLIANCE.md`), and the D8 server keeps a rolling packet
  capture by hand. A phone's own view, behind its NAT, decrypted from TLS and SRTP, is
  what neither has.

## 2. Call stats (43)

**What Anvil measures**, per call, every second:

- packets and bytes sent and received, and their rate;
- packets lost (sequence gaps) and late or discarded (the drops it already counts),
  as counts and a percentage over the last 5 s and the whole call;
- jitter (the RFC 3550 estimate, in ms) and the jitter buffer's depth;
- from **RTCP** (new: sender and receiver reports every 5 s, RFC 3550, and RTCP over
  SRTP): what the far end reports losing and its jitter, and the round-trip time;
- the codec, its clock rate and packet time, SRTP on or off, the transport and the
  addresses media flows between;
- an **estimated MOS** (the ITU-T G.107 E-model from loss, jitter, delay and the
  codec), shown as a number and good / fair / poor.

**Where it is shown:** a "Call stats" panel opened from the call screen (and a small
quality indicator always on it), updating live; on mobile a sheet.

**After the call:** the call's final figures are sent to FCP
(`POST /me/calls/{call_id}/quality`), kept beside the record (`call_quality`: call,
device, direction, the figures, the MOS) and shown on the call's detail in the admin
UI and the user's recents. A poor call (MOS under 3.1) publishes
`call.quality_poor`.

## 3. Captures (44)

**Who may.** The tenant's `diagnostics.capture_allowed` (off by default); then, per
user or per device, an admin's allow with an end date (at most 30 days). `/me/softphone`
answers `capture: {allowed, until, max_mb}`. While allowed, Anvil's Settings show
**Capture calls**; the user turns it on for the next call or for every call until they
turn it off. A capture in progress shows on the call screen.

**What a capture holds** (`.pcapng`):

- every SIP message the app sent and received, **as it was before TLS encryption and
  after decryption**, written as UDP/TCP frames between the real addresses, so
  Wireshark reads them as SIP;
- the call's RTP and RTCP as sent and received on the wire (SRTP encrypted), with the
  SRTP keys of the call written into the file's decryption secrets block, so
  Wireshark plays the audio;
- the app's version, the device, the account and the call's id in the file's
  comments.

**Kept locally** in the app's data directory, at most `max_mb` (default 50 MB per
capture, 200 MB in all), deleted after 7 days; the user can open the folder or delete
them.

**Uploaded to FCP** when the call ends, over the app's session
(`POST /me/devices/{id}/captures`, the body the file). FCP keeps a
`device_captures` row (tenant, device, user, call id, size, SHA-256, expiry) and the
file in the captures' store (`[api.captures]`: a directory or a bucket), under legal
holds, deleted after 30 days by default. The admin sees them on the device's
Captures (`GET /captures?device_id=…`; download `GET /captures/{id}/file`, each
download audited). The user is told captures are uploaded before they turn it on.

## 4. Phases

1. **RTCP in forge-media / anvil-core.** Sender and receiver reports sent and read,
   over SRTP too; round-trip time; the stats snapshot API in anvil-core (`call_stats`).
2. **Anvil: the panel.** The live panel, the quality indicator, the MOS estimate.
3. **FCP: a call's quality kept.** `POST /me/calls/{id}/quality`, `call_quality`
   (migration), on the call's detail and recents, `call.quality_poor`; Anvil sends it.
4. **FCP: captures allowed and kept.** The tenant setting and the per-user/device
   allow with an end date, `/me/softphone`'s `capture`, the upload route,
   `device_captures`, retention and holds, the device page's captures, audited
   downloads.
5. **Anvil: captures.** The pcapng writer (SIP before TLS, RTP/RTCP as on the wire,
   SRTP keys), the setting, local limits and expiry, the upload.

## 4a. Done

- **Phase 1** (Anvil v0.2.27, anvil #61): RTCP in anvil-core. No forge-media change was
  needed: forge-rtp already had the sender and receiver statistics, the round-trip
  tracker and SRTCP, and FCP's media relay already passes each phone's RTCP to the
  other leg on the port above RTP, so the far end's reports and the round trip are
  measured end to end. A call binds RTP on an even port and RTCP on the next; reports
  go a second after media starts and every five seconds after; `Anvil::call_stats`.
- **Phase 2** (Anvil v0.2.28, anvil #62): the panel. `anvil-core`'s `quality.rs` is
  the G.107 E-model, narrowband, with G.113's figures for codecs played **without
  concealment** — Anvil has no jitter buffer and no loss concealment, so jitter is
  counted as delay a buffer would have added and loss is not softened. Stats every
  second, loss over the last five seconds, a score for what the user hears and, once
  the far end reports, for what it hears. The call screen shows the lower score; the
  Call stats panel shows everything §2 lists except the jitter buffer's depth (there is
  no buffer to measure).
- **Phase 3** (v0.25.490): a call's quality kept.
  - `call_quality` (migration 131): one row per phone on a call — tenant, call, user,
    device — a second report from the same phone replacing its first.
  - `POST /api/v1/me/calls/{call_id}/quality` from the app's session only (the
    device is the session's). It refuses figures that make no sense (a score outside
    1 to 4.5, more lost than expected, a negative jitter). When the call's record is
    written and names other people, it answers 404. A call whose record is not yet
    written is taken on the app's word: the app reports at hangup, usually before the
    record lands.
  - FCP works out the loss itself from the packets lost and expected.
  - `call.quality_poor` when either score is under 3.1 (`fcp_core::call_quality::POOR_MOS`),
    naming the call, the user, the device, both scores, loss, jitter and the round trip.
  - `GET /api/v1/cdrs/{call_id}/quality` under `cdr_read` (every phone's report, one
    page); the admin UI's call record shows each report. `/me/calls` carries each
    call's `quality` (`mos`, `far_mos`, `poor`; the worst of the user's phones).
  - Retention: a report goes in the CDR sweep once it is older than the cut **and**
    no record of the call is left, so a held record keeps its report.
  - A user purged takes their reports with them (the row's user is a foreign key).
  - **Not yet:** a site's reports stay at the site; they are not shipped up its link
    like its CDRs. Subject export and erasure do not list the reports as their own
    class: the reports hold no addresses but the phone's own media address.
  - Anvil sends the report when a call ends: anvil #63.
- **Phase 4** (v0.25.491): captures allowed and kept.
  - The tenant's `diagnostics` (migration 132's `diagnostics_json`):
    `capture_allowed` (off), `capture_max_mb` (50, at most 500) and
    `capture_keep_days` (30, at most 365), on the Tenants page.
  - Allows (`capture_allows`): a user — every app they sign in to — or one device,
    until a date at most 30 days away, with a reason.
    - `GET`/`POST /api/v1/captures/allows`, `DELETE /captures/allows/{id}` to end one.
    - An allow is refused while the tenant has captures off (409).
    - A purged user's or removed device's allows go with them.
  - `/me/softphone` answers `capture: {allowed, until, max_mb}`. It is allowed only
    when the tenant has captures on, an allow covers the user or the app, and this
    admin has `[api.captures]` (a directory, or `[api.captures.s3]` with keys under
    `captures/`).
  - `POST /api/v1/me/devices/{id}/captures?call_id=…` takes the pcapng as the body.
    - It is accepted from the app's own session, for its own device, while allowed;
      otherwise 400 from a web session, 404 for another device and 403 when not
      allowed.
    - The body must start with pcapng's section header (else 400) and be at most
      the tenant's limit (else 413, a new error code).
    - A named call must be the user's (404 when its record names others; one not yet
      written is taken, as for quality).
    - The file is staged, hashed, stored, then its row written.
    - The response says `held` when a legal hold covers it.
  - Listing and download:
    - `GET /api/v1/captures` (by `device_id` or `username`) and `GET /me/captures`.
    - `GET /captures/{id}/file`; the audit middleware records every download.
    - `DELETE /captures/{id}` answers 409 while held.
    - The Devices page's **Captures** (on a softphone) allows the app or its owner
      for 1, 7 or 30 days, ends an allow, and downloads or removes a capture.
  - Permissions: listing under `device_read`, allows and downloads under
    `device_update`, removal under `device_delete`. There is no `device_manage`; Q5's
    meaning is `device_update`.
  - Legal holds: `device_capture` is a hold class.
    - A capture is held by the user's subject keys taken at upload, or by its call.
    - Migration 132 adds it to `fcp_guard_held_deletion`, so a delete of a held row
      is refused whatever path it takes.
    - The Compliance page offers it.
  - Retention: the admin's hourly sweep (`fcp:captures` lock) removes captures past
    their expiry that no hold covers.
  - Files: every removed capture's file (by an administrator, by the sweep, or with
    a purged user) is queued by a trigger in `device_capture_file_deletions` and
    removed from the store. A failed removal is retried an hour later.
  - **Not yet:**
    - A site's admin keeps its own captures and does not ship them up.
    - Subject export and erasure do not list captures. Erasure reaches them only
      through the purge.
    - Captures are not in the trigger-written deletion log.
    - A capture file restored without its row is neither served nor removed.

## 5. Questions

1. **RTCP in Anvil** (needed for the far end's loss and round-trip time), phase 1.
   Recommended: yes; without it the panel can only report what Anvil receives.
2. **Quality sent to FCP after every call**, not only when stats were opened.
   Recommended: yes; it is a few hundred bytes and it is what an admin troubleshoots
   from.
3. **Captures uploaded at the call's end**, retried while the app runs, and the local
   copy kept too. Recommended: yes.
4. **An allow expires** (at most 30 days) so a capture is not left on for ever.
   Recommended: yes.
5. **Who may download:** `device_manage` holders in the tenant, and platform admins.
   Recommended: yes; the user sees their own on the device's page in the portal.
