//! A webinar streamed out as HLS (`docs/WEBINARS.md` §2.8, W6.2).
//!
//! The recorder's frames go through `forge_mp4::cmaf::CmafSegmenter` into a
//! directory of their own:
//! - `init.mp4`, once the first keyframe has made it;
//! - `seg-<n>.m4s`, a segment of about the target duration, each starting
//!   on a keyframe the writer asked the encoder for;
//! - `index.m3u8`, a live `EVENT` playlist rewritten after each segment,
//!   which ends with `EXT-X-ENDLIST` when the stream does.
//!
//! The playlist is written to a temporary file and renamed into place, so
//! a player never reads half of one.

#[cfg(feature = "recording-mp4")]
use anyhow::{Context, Result};
#[cfg(feature = "recording-mp4")]
use forge_core::VideoCodec;
#[cfg(feature = "recording-mp4")]
use forge_mp4::cmaf::{CmafSegment, CmafSegmenter};
#[cfg(feature = "recording-mp4")]
use forge_mp4::{AudioTrack, Mp4Config, Mp4VideoCodec, VideoTrack};
#[cfg(feature = "recording-mp4")]
use forge_video::flavor::Flavor;
use std::path::{Path, PathBuf};

/// The playlist's name in a stream's directory.
pub const PLAYLIST: &str = "index.m3u8";
/// The init segment's name.
pub const INIT: &str = "init.mp4";

/// Whether `name` is a file a stream's directory holds: the playlist, the
/// init segment, or `seg-<n>.m4s`. Nothing else is served.
pub fn is_stream_file(name: &str) -> bool {
    name == PLAYLIST
        || name == INIT
        || name
            .strip_prefix("seg-")
            .and_then(|n| n.strip_suffix(".m4s"))
            .is_some_and(|n| {
                !n.is_empty() && n.len() <= 10 && n.bytes().all(|b| b.is_ascii_digit())
            })
}

/// Whether `name` could be a stream's directory: a sitting's id, never a
/// recording's file name (which has an extension).
pub fn is_sitting_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A stream directory's files in the order they play: the init segment,
/// then each segment by its number. Read back in this order they are one
/// fragmented MP4. Nothing when `dir` holds no init segment.
pub fn stream_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let init = dir.join(INIT);
    if !init.is_file() {
        return Ok(Vec::new());
    }
    let mut segments = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(n) = name
            .strip_prefix("seg-")
            .and_then(|n| n.strip_suffix(".m4s"))
            .and_then(|n| n.parse::<u32>().ok())
        {
            segments.push((n, dir.join(name)));
        }
    }
    segments.sort_by_key(|(n, _)| *n);
    Ok(std::iter::once(init)
        .chain(segments.into_iter().map(|(_, p)| p))
        .collect())
}

/// The content type a stream's file is served as.
pub fn content_type(name: &str) -> &'static str {
    if name.ends_with(".m3u8") {
        "application/vnd.apple.mpegurl"
    } else {
        "video/mp4"
    }
}

/// A segment's name.
pub fn segment_name(sequence: u32) -> String {
    format!("seg-{sequence}.m4s")
}

/// A media playlist of `segments` (sequence, duration in ms), live until
/// `ended`.
pub fn playlist(segments: &[(u32, u64)], target_ms: u64, ended: bool) -> String {
    // A segment runs to the keyframe after its target, so the playlist's
    // target duration is the longest there has been, never less than the
    // target itself.
    let longest = segments.iter().map(|(_, d)| *d).max().unwrap_or(0);
    let target = longest.max(target_ms).div_ceil(1000).max(1);
    let first = segments.first().map_or(1, |(n, _)| *n);
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{target}\n\
         #EXT-X-MEDIA-SEQUENCE:{first}\n#EXT-X-PLAYLIST-TYPE:EVENT\n\
         #EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-MAP:URI=\"{INIT}\"\n"
    );
    for (n, d) in segments {
        out.push_str(&format!(
            "#EXTINF:{}.{:03},\n{}\n",
            d / 1000,
            d % 1000,
            segment_name(*n)
        ));
    }
    if ended {
        out.push_str("#EXT-X-ENDLIST\n");
    }
    out
}

/// Writes one stream's directory.
#[cfg(feature = "recording-mp4")]
pub struct HlsWriter {
    dir: PathBuf,
    segmenter: CmafSegmenter,
    target_ms: u64,
    /// One video frame's length, for asking a little ahead.
    frame_ms: u64,
    has_video: bool,
    init_written: bool,
    segments: Vec<(u32, u64)>,
    bytes: u64,
    /// The last keyframe written, and whether the next has been asked for.
    last_keyframe_ms: Option<u64>,
    asked: bool,
    /// The bucket's copy, sent each file as it is written.
    mirror: Option<super::hls_bucket::HlsMirror>,
}

#[cfg(feature = "recording-mp4")]
impl HlsWriter {
    /// A stream into `dir` (made if missing), of the composite `video`
    /// (H.264, HEVC or AV1) when there is one and the mix as Opus, cut every
    /// `target_ms`; each file also sent to `mirror` when there is one.
    pub fn open(
        dir: &Path,
        video: Option<&Flavor>,
        target_ms: u64,
        mirror: Option<super::hls_bucket::HlsMirror>,
    ) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let mut config = Mp4Config::new().audio(AudioTrack::opus(48_000, 1));
        let mut frame_ms = 66;
        if let Some(flavor) = video {
            let codec = match flavor.codec {
                VideoCodec::H264 => Mp4VideoCodec::H264,
                VideoCodec::H265 => Mp4VideoCodec::Hevc,
                VideoCodec::AV1 => Mp4VideoCodec::Av1,
                other => anyhow::bail!("an HLS stream does not carry {}", other.sdp_name()),
            };
            config = config.video(
                VideoTrack::new(codec, flavor.resolution.width, flavor.resolution.height)
                    .fps(flavor.fps),
            );
            frame_ms = 1000 / u64::from(flavor.fps.max(1));
        }
        let segmenter = CmafSegmenter::new(config, target_ms).context("opening the stream")?;
        let mut w = Self {
            dir: dir.to_path_buf(),
            segmenter,
            target_ms,
            frame_ms,
            has_video: video.is_some(),
            init_written: false,
            segments: Vec::new(),
            bytes: 0,
            last_keyframe_ms: None,
            asked: false,
            mirror,
        };
        w.drain()?;
        Ok(w)
    }

    pub fn write_video(&mut self, ms: u64, keyframe: bool, data: &[u8]) -> Result<()> {
        if keyframe {
            self.last_keyframe_ms = Some(ms);
            self.asked = false;
        }
        self.segmenter.write_video(ms, keyframe, data)?;
        self.drain()
    }

    pub fn write_audio(&mut self, ms: u64, data: &[u8]) -> Result<()> {
        self.segmenter.write_audio(ms, data)?;
        self.drain()
    }

    /// Whether to ask for a keyframe now: once a segment, a frame before
    /// its target, so the segment closes about on time.
    pub fn wants_keyframe(&mut self, ms: u64) -> bool {
        if !self.has_video || self.asked {
            return false;
        }
        let Some(last) = self.last_keyframe_ms else {
            return false;
        };
        if ms + self.frame_ms >= last + self.target_ms {
            self.asked = true;
            return true;
        }
        false
    }

    /// End the stream: the last segment and `EXT-X-ENDLIST`. Its duration
    /// and size on disk.
    pub fn finish(mut self) -> Result<(u64, u64)> {
        if let Some(last) = self.segmenter.finish()? {
            self.write_segment(last)?;
        }
        if self.init_written {
            self.write_playlist(true)?;
        }
        let duration = self.segments.iter().map(|(_, d)| *d).sum();
        Ok((duration, self.bytes))
    }

    /// Write out what the segmenter has made.
    fn drain(&mut self) -> Result<()> {
        if !self.init_written {
            if let Some(init) = self.segmenter.init_segment() {
                let init = init.to_vec();
                self.write_file(INIT, &init)?;
                self.init_written = true;
            }
        }
        while let Some(segment) = self.segmenter.take_segment() {
            self.write_segment(segment)?;
            self.write_playlist(false)?;
        }
        Ok(())
    }

    fn write_segment(&mut self, segment: CmafSegment) -> Result<()> {
        self.write_file(&segment_name(segment.sequence), &segment.bytes)?;
        self.segments.push((segment.sequence, segment.duration_ms));
        Ok(())
    }

    fn write_playlist(&mut self, ended: bool) -> Result<()> {
        let text = playlist(&self.segments, self.target_ms, ended);
        self.write_file(PLAYLIST, text.as_bytes())
    }

    /// Write `name` whole: to a temporary name, then renamed into place.
    fn write_file(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        let path = self.dir.join(name);
        let tmp = self.dir.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("renaming to {}", path.display()))?;
        self.bytes += bytes.len() as u64;
        if let Some(mirror) = &self.mirror {
            mirror.send(name, bytes);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_stream_s_own_files_are_named() {
        for ok in ["index.m3u8", "init.mp4", "seg-1.m4s", "seg-1234.m4s"] {
            assert!(is_stream_file(ok), "{ok}");
        }
        for bad in [
            "",
            "seg-.m4s",
            "seg-1.mp4",
            "seg-a.m4s",
            "../index.m3u8",
            "seg-1/../x.m4s",
            ".index.m3u8.tmp",
            "seg-12345678901.m4s",
        ] {
            assert!(!is_stream_file(bad), "{bad}");
        }
        assert_eq!(content_type("index.m3u8"), "application/vnd.apple.mpegurl");
        assert_eq!(content_type("seg-3.m4s"), "video/mp4");
    }

    #[test]
    fn a_stream_s_directory_reads_back_init_first_then_segments_by_number() {
        for ok in ["5f1c0e2a-0000-4000-8000-000000000001", "abc"] {
            assert!(is_sitting_name(ok), "{ok}");
        }
        for bad in ["", "a.mp4", "../x", "a/b", &"x".repeat(65)] {
            assert!(!is_sitting_name(bad), "{bad}");
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(
            stream_files(dir.path()).unwrap().is_empty(),
            "no init, no stream"
        );
        for name in [
            "seg-10.m4s",
            "seg-2.m4s",
            INIT,
            PLAYLIST,
            "seg-1.m4s",
            ".seg-3.m4s.tmp",
        ] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        let names: Vec<String> = stream_files(dir.path())
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [INIT, "seg-1.m4s", "seg-2.m4s", "seg-10.m4s"]);
    }

    #[test]
    fn the_playlist_is_live_until_it_ends() {
        let live = playlist(&[(1, 2000), (2, 2066)], 2000, false);
        assert!(live.starts_with("#EXTM3U\n#EXT-X-VERSION:7\n"), "{live}");
        assert!(live.contains("#EXT-X-TARGETDURATION:3\n"), "{live}");
        assert!(live.contains("#EXT-X-MEDIA-SEQUENCE:1\n"), "{live}");
        assert!(live.contains("#EXT-X-PLAYLIST-TYPE:EVENT\n"), "{live}");
        assert!(live.contains("#EXT-X-MAP:URI=\"init.mp4\"\n"), "{live}");
        assert!(
            live.contains("#EXTINF:2.000,\nseg-1.m4s\n#EXTINF:2.066,\nseg-2.m4s\n"),
            "{live}"
        );
        assert!(!live.contains("ENDLIST"));
        let ended = playlist(&[(1, 2000)], 2000, true);
        assert!(ended.contains("#EXT-X-TARGETDURATION:2\n"), "{ended}");
        assert!(ended.ends_with("#EXT-X-ENDLIST\n"), "{ended}");
    }

    #[cfg(feature = "recording-mp4")]
    #[test]
    fn audio_alone_streams_into_its_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = HlsWriter::open(dir.path(), None, 2000, None).unwrap();
        assert!(
            dir.path().join(INIT).exists(),
            "audio alone has its init at once"
        );
        for i in 0..250u64 {
            w.write_audio(i * 20, &[0xFC, 1, 2]).unwrap();
        }
        let live = std::fs::read_to_string(dir.path().join(PLAYLIST)).unwrap();
        assert!(
            live.contains("seg-2.m4s") && !live.contains("ENDLIST"),
            "{live}"
        );
        let (duration, bytes) = w.finish().unwrap();
        assert_eq!(duration, 5000);
        assert!(bytes > 0);
        let ended = std::fs::read_to_string(dir.path().join(PLAYLIST)).unwrap();
        assert!(
            ended.contains("seg-3.m4s") && ended.ends_with("#EXT-X-ENDLIST\n"),
            "{ended}"
        );
        for n in 1..=3 {
            assert!(dir.path().join(segment_name(n)).exists());
        }
        // Nothing half-written is left behind.
        let stray: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(stray.is_empty());
    }
}
