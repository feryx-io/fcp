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
(`POST /me/devices/{id}/captures`, the body the file). FCP stores it like a
recording: a `device_captures` row (tenant, device, user, call id, size, SHA-256,
expiry) and the file in the recordings store, under the tenant's retention and legal
holds, deleted after 30 days by default. The admin sees them on the device's page
(`GET /devices/{id}/captures`, download under `device_manage`, each download
audited). The user is told captures are uploaded before they turn it on.

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
