//! Previews and audio/video conversions made by the background queue
//! (Milestone 3b, decision 0007).
//!
//! Only what browsers cannot use is converted, and when only the container
//! is the problem the streams are copied into a new one instead of being
//! re-encoded:
//!
//! | Kind | Browsers can use it | Container only | Otherwise |
//! | --- | --- | --- | --- |
//! | `preview` | JPEG, PNG, GIF, WebP, BMP, AVIF bytes | | WebP, ≤ 2048 px |
//! | `video` | MP4/QuickTime with H.264 (8-bit 4:2:0) and AAC, MP3 or no audio | other audio, or a patched `chnl` box: copy video, audio to AAC | H.264 ≤ 1920 px, CRF 23, HDR tone-mapped |
//! | `audio` | MP4 or MP3 with AAC or MP3 | AAC in CAF: copy into M4A | AAC 64 kbit/s M4A |
//!
//! "Browsers can use it" means current Safari, Chrome, Edge and Firefox on
//! any system, without relying on hardware decoding. HEVC is therefore
//! always converted. Every file reaches a tool through [`Input`], and images
//! fall back to ffmpeg's first frame exactly as thumbnails do.

use std::{fs::File, io::Read, path::Path, process::Command, time::Duration};

use serde::Deserialize;

use super::{
    Still, TOOL_TIMEOUT, image_still,
    input::{Container, Input},
    run_tool, run_tool_output, shorten, video_still,
};

/// Previews: longest side and WebP quality.
pub const PREVIEW: Still = Still {
    max: 2048,
    quality: 82,
    keep_icc: true,
};
/// Longest side of a converted video, in pixels.
pub const VIDEO_MAX: u32 = 1920;
pub const VIDEO_CRF: u32 = 23;
pub const VIDEO_PRESET: &str = "veryfast";
pub const VIDEO_AUDIO_BITRATE: &str = "128k";
pub const VOICE_BITRATE: &str = "64k";

/// A video conversion may take this many times the video's length, on top of
/// [`TOOL_TIMEOUT`], before it is killed and recorded as failed.
const VIDEO_TIMEOUT_PER_SECOND: u64 = 8;

/// What was done to make a derivative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    /// Browsers can use the original as it is; no file is made.
    Original,
    /// The streams were copied into a new container (audio re-encoded only
    /// when browsers cannot play it).
    Remux,
    /// Re-encoded.
    Transcode,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Remux => "remux",
            Self::Transcode => "transcode",
        }
    }
}

/// Which derivative the queue makes; the names stored in `derivative.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A web image for a picture browsers cannot show (HEIC, TIFF, ...).
    Preview,
    Video,
    /// Voice messages.
    Audio,
}

impl Kind {
    pub const ALL: [Self; 3] = [Self::Audio, Self::Preview, Self::Video];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// File extension of a made file.
    pub fn ext(self) -> &'static str {
        match self {
            Self::Preview => "webp",
            Self::Video => "mp4",
            Self::Audio => "m4a",
        }
    }

    pub fn recipe(self) -> i64 {
        match self {
            Self::Preview => super::PREVIEW_RECIPE,
            Self::Video => super::VIDEO_RECIPE,
            Self::Audio => super::AUDIO_RECIPE,
        }
    }
}

/// What converting a file will do, found without converting it.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub action: Action,
    /// Length in seconds, for video and audio.
    pub duration: f64,
    /// Source resolution, for video.
    pub pixels: u64,
}

/// A finished derivative.
#[derive(Debug, Clone, PartialEq)]
pub struct Made {
    pub action: Action,
    /// Size of the file written to `out`; `None` when the original is used.
    pub bytes: Option<u64>,
}

/// Decides what to do with `src` without making anything.
pub fn plan(kind: Kind, src: &Path, work_dir: &Path) -> Result<Plan, String> {
    match kind {
        Kind::Preview => Ok(Plan {
            action: if is_web_image(src)? {
                Action::Original
            } else {
                Action::Transcode
            },
            duration: 0.0,
            pixels: 0,
        }),
        Kind::Video => {
            let input = Input::prepare(src, work_dir)?;
            let probe = probe(&input)?;
            let v = video_plan(&probe, input.is_patched())?;
            Ok(Plan {
                action: v.action,
                duration: probe.duration,
                pixels: probe.video.as_ref().map_or(0, |s| s.pixels()),
            })
        }
        Kind::Audio => {
            let input = Input::prepare(src, work_dir)?;
            let probe = probe(&input)?;
            Ok(Plan {
                action: audio_plan(&probe)?.action,
                duration: probe.duration,
                pixels: 0,
            })
        }
    }
}

/// Makes the derivative of `kind` from `src` into `out`, unless browsers can
/// use `src` as it is. `work_dir` holds a patched copy of `src` while one is
/// needed.
pub fn make(kind: Kind, src: &Path, out: &Path, work_dir: &Path) -> Result<Made, String> {
    let action = match kind {
        Kind::Preview => preview(src, out, work_dir)?,
        Kind::Video => video(src, out, work_dir)?,
        Kind::Audio => audio(src, out, work_dir)?,
    };
    if action == Action::Original {
        return Ok(Made {
            action,
            bytes: None,
        });
    }
    match std::fs::metadata(out) {
        Ok(m) if m.len() > 0 => Ok(Made {
            action,
            bytes: Some(m.len()),
        }),
        _ => Err("tool succeeded but wrote no output".to_owned()),
    }
}

/// Whether a tool error means the disk is full, which is not the file's
/// fault: the file stays pending and the run stops.
pub fn is_disk_full(error: &str) -> bool {
    error.contains("No space left on device") || error.contains("Disk quota exceeded")
}

fn preview(src: &Path, out: &Path, work_dir: &Path) -> Result<Action, String> {
    if is_web_image(src)? {
        return Ok(Action::Original);
    }
    let input = Input::prepare(src, work_dir)?;
    match run_tool(image_still(&input, out, PREVIEW), TOOL_TIMEOUT) {
        Ok(()) => {}
        // The same fallback as thumbnails: an MP4-family file ImageMagick
        // cannot read gets its first video frame.
        Err(first) if input.container() != Container::Other && !is_disk_full(&first) => {
            run_tool(video_still(&input, out, PREVIEW), TOOL_TIMEOUT)
                .map_err(|second| shorten(&format!("{first}; then {second}")))?;
        }
        Err(e) => return Err(e),
    }
    Ok(Action::Transcode)
}

/// Picture formats every current browser shows, recognised by their bytes
/// (a file labelled HEIC can hold a JPEG).
fn is_web_image(src: &Path) -> Result<bool, String> {
    let mut head = [0u8; 16];
    let mut file = File::open(src).map_err(|e| format!("opening the file: {e}"))?;
    let n = read_up_to(&mut file, &mut head).map_err(|e| format!("reading the file: {e}"))?;
    let h = &head[..n];
    Ok(h.starts_with(&[0xFF, 0xD8, 0xFF])
        || h.starts_with(b"\x89PNG\r\n\x1a\n")
        || h.starts_with(b"GIF87a")
        || h.starts_with(b"GIF89a")
        || (h.len() >= 12 && &h[..4] == b"RIFF" && &h[8..12] == b"WEBP")
        || h.starts_with(b"BM")
        || (h.len() >= 12 && &h[4..8] == b"ftyp" && matches!(&h[8..12], b"avif" | b"avis")))
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match file.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// What ffprobe reports about a file: its container and first streams.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Probe {
    /// ffprobe's format names, such as `mov,mp4,m4a,3gp,3g2,mj2`.
    pub format: String,
    pub duration: f64,
    pub video: Option<Stream>,
    pub audio: Option<Stream>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Stream {
    #[serde(default)]
    pub codec_type: String,
    #[serde(default)]
    pub codec_name: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub pix_fmt: Option<String>,
    #[serde(default)]
    pub width: Option<u64>,
    #[serde(default)]
    pub height: Option<u64>,
    #[serde(default)]
    pub color_transfer: Option<String>,
    #[serde(default)]
    pub disposition: Option<Disposition>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Disposition {
    #[serde(default)]
    pub attached_pic: u8,
}

impl Stream {
    fn pixels(&self) -> u64 {
        self.width.unwrap_or(0) * self.height.unwrap_or(0)
    }
}

#[derive(Deserialize)]
struct ProbeJson {
    #[serde(default)]
    streams: Vec<Stream>,
    format: Option<FormatJson>,
}

#[derive(Deserialize)]
struct FormatJson {
    #[serde(default)]
    format_name: String,
    #[serde(default)]
    duration: Option<String>,
}

/// Runs ffprobe on `input`.
pub fn probe(input: &Input) -> Result<Probe, String> {
    let mut cmd = Command::new("ffprobe");
    cmd.args(["-v", "error", "-of", "json"])
        .args([
            "-show_entries",
            "format=format_name,duration:stream=codec_type,codec_name,profile,pix_fmt,width,height,color_transfer:stream_disposition=attached_pic",
        ])
        .arg(input.path());
    let out = run_tool_output(cmd, TOOL_TIMEOUT)?;
    parse_probe(&out)
}

fn parse_probe(json: &[u8]) -> Result<Probe, String> {
    let parsed: ProbeJson =
        serde_json::from_slice(json).map_err(|e| format!("ffprobe: unreadable output: {e}"))?;
    let format = parsed
        .format
        .ok_or_else(|| "ffprobe: no format found".to_owned())?;
    let first = |kind: &str| {
        parsed
            .streams
            .iter()
            .find(|s| {
                s.codec_type == kind && s.disposition.as_ref().is_none_or(|d| d.attached_pic == 0)
            })
            .cloned()
    };
    Ok(Probe {
        duration: format
            .duration
            .and_then(|d| d.parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d >= 0.0)
            .unwrap_or(0.0),
        format: format.format_name,
        video: first("video"),
        audio: first("audio"),
    })
}

impl Probe {
    /// MP4, QuickTime, M4A or 3GP.
    fn is_iso_media(&self) -> bool {
        self.format.split(',').any(|f| f == "mov" || f == "mp4")
    }

    fn audio_codec(&self) -> Option<&str> {
        self.audio.as_ref().map(|a| a.codec_name.as_str())
    }
}

/// What to do with a video, and how.
#[derive(Debug, Clone, PartialEq)]
struct VideoPlan {
    action: Action,
    /// Copy the audio as it is (or there is none); otherwise encode AAC.
    copy_audio: bool,
    /// The HDR transfer function to tone-map from, when converting.
    hdr: Option<String>,
}

/// Video codecs browsers play inside MP4 without hardware help.
fn browser_video(v: &Stream) -> bool {
    let profile_ok = v.profile.as_deref().is_none_or(|p| {
        matches!(
            p,
            "Baseline" | "Constrained Baseline" | "Main" | "High" | "Extended"
        )
    });
    v.codec_name == "h264"
        && profile_ok
        && matches!(v.pix_fmt.as_deref(), Some("yuv420p" | "yuvj420p"))
}

fn browser_audio_in_mp4(codec: Option<&str>) -> bool {
    matches!(codec, None | Some("aac" | "mp3"))
}

fn video_plan(p: &Probe, patched: bool) -> Result<VideoPlan, String> {
    let v = p
        .video
        .as_ref()
        .ok_or_else(|| "no video stream".to_owned())?;
    let audio_ok = browser_audio_in_mp4(p.audio_codec());
    let hdr = v
        .color_transfer
        .as_deref()
        .filter(|t| matches!(*t, "arib-std-b67" | "smpte2084"))
        .map(str::to_owned);
    let action = if !browser_video(v) {
        Action::Transcode
    } else if p.is_iso_media() && audio_ok && !patched {
        Action::Original
    } else {
        Action::Remux
    };
    Ok(VideoPlan {
        action,
        copy_audio: audio_ok,
        hdr: if action == Action::Transcode {
            hdr
        } else {
            None
        },
    })
}

fn video(src: &Path, out: &Path, work_dir: &Path) -> Result<Action, String> {
    let input = Input::prepare(src, work_dir)?;
    let probe = probe(&input)?;
    let plan = video_plan(&probe, input.is_patched())?;
    if plan.action == Action::Original {
        return Ok(Action::Original);
    }
    let timeout =
        TOOL_TIMEOUT + Duration::from_secs(VIDEO_TIMEOUT_PER_SECOND * probe.duration.ceil() as u64);
    run_tool(video_command(&input, out, &plan), timeout)?;
    Ok(plan.action)
}

fn video_command(input: &Input, out: &Path, plan: &VideoPlan) -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-v", "error", "-threads", "1", "-i"])
        .arg(input.path())
        .args(["-map", "0:v:0", "-map", "0:a:0?"]);
    if plan.action == Action::Remux {
        cmd.args(["-c:v", "copy"]);
    } else {
        cmd.args(["-vf", &video_filter(plan.hdr.as_deref())])
            .args(["-filter_threads", "1", "-c:v", "libx264"])
            .args(["-preset", VIDEO_PRESET, "-crf", &VIDEO_CRF.to_string()])
            .args(["-profile:v", "high", "-threads", "1"])
            .args(["-color_primaries", "bt709", "-color_trc", "bt709"])
            .args(["-colorspace", "bt709"]);
    }
    if plan.copy_audio {
        cmd.args(["-c:a", "copy"]);
    } else {
        cmd.args(["-c:a", "aac", "-b:a", VIDEO_AUDIO_BITRATE]);
    }
    cmd.args(["-movflags", "+faststart", "-f", "mp4", "-y"])
        .arg(out);
    cmd
}

/// Scales to at most [`VIDEO_MAX`] on the long side (after rotation, which
/// ffmpeg applies), tone-maps HDR to SDR, and ends in 8-bit 4:2:0.
fn video_filter(hdr: Option<&str>) -> String {
    let scale = format!(
        "scale=w='min({VIDEO_MAX},iw)':h='min({VIDEO_MAX},ih)':force_original_aspect_ratio=decrease:force_divisible_by=2"
    );
    match hdr {
        Some(transfer) => format!(
            "{scale},zscale=tin={transfer}:pin=bt2020:min=bt2020nc:t=linear:npl=100,format=gbrpf32le,\
             zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p"
        ),
        None => format!("{scale},format=yuv420p"),
    }
}

/// What to do with a voice message.
#[derive(Debug, Clone, PartialEq)]
struct AudioPlan {
    action: Action,
}

fn audio_plan(p: &Probe) -> Result<AudioPlan, String> {
    let codec = p
        .audio_codec()
        .ok_or_else(|| "no audio stream".to_owned())?;
    let playable_container = p.is_iso_media() || p.format == "mp3";
    let action = match codec {
        "aac" | "mp3" if playable_container => Action::Original,
        "aac" => Action::Remux,
        // Opus, AMR, PCM, iLBC and the rest. Opus could stay Opus, but
        // ffmpeg cannot copy it from CAF into MP4 and older Safari cannot
        // play it in WebM.
        _ => Action::Transcode,
    };
    Ok(AudioPlan { action })
}

fn audio(src: &Path, out: &Path, work_dir: &Path) -> Result<Action, String> {
    let input = Input::prepare(src, work_dir)?;
    let probe = probe(&input)?;
    let plan = audio_plan(&probe)?;
    if plan.action == Action::Original {
        return Ok(Action::Original);
    }
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-v", "error", "-threads", "1", "-i"])
        .arg(input.path())
        .args(["-map", "0:a:0", "-vn"]);
    if plan.action == Action::Remux {
        cmd.args(["-c:a", "copy"]);
    } else {
        cmd.args(["-c:a", "aac", "-b:a", VOICE_BITRATE]);
    }
    cmd.args(["-movflags", "+faststart", "-f", "mp4", "-y"])
        .arg(out);
    run_tool(cmd, TOOL_TIMEOUT)?;
    Ok(plan.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(kind: &str, codec: &str) -> Stream {
        Stream {
            codec_type: kind.to_owned(),
            codec_name: codec.to_owned(),
            ..Stream::default()
        }
    }

    fn h264() -> Stream {
        Stream {
            profile: Some("High".to_owned()),
            pix_fmt: Some("yuv420p".to_owned()),
            ..stream("video", "h264")
        }
    }

    fn mov(video: Stream, audio: Option<&str>) -> Probe {
        Probe {
            format: "mov,mp4,m4a,3gp,3g2,mj2".to_owned(),
            duration: 3.0,
            video: Some(video),
            audio: audio.map(|c| stream("audio", c)),
        }
    }

    #[test]
    fn videos_are_left_remuxed_or_converted() {
        let plan = |p: &Probe, patched| video_plan(p, patched).unwrap();
        assert_eq!(
            plan(&mov(h264(), Some("aac")), false).action,
            Action::Original
        );
        assert_eq!(plan(&mov(h264(), None), false).action, Action::Original);
        // Only the audio is the problem: copy the video, encode AAC.
        let p = plan(&mov(h264(), Some("pcm_s16le")), false);
        assert_eq!((p.action, p.copy_audio), (Action::Remux, false));
        // A file read through a patched copy gets a clean container.
        let p = plan(&mov(h264(), Some("aac")), true);
        assert_eq!((p.action, p.copy_audio), (Action::Remux, true));
        // HEVC and 10-bit H.264 are converted; HDR is tone-mapped.
        let hevc = Stream {
            color_transfer: Some("arib-std-b67".to_owned()),
            pix_fmt: Some("yuv420p10le".to_owned()),
            ..stream("video", "hevc")
        };
        let p = plan(&mov(hevc, Some("aac")), false);
        assert_eq!(p.action, Action::Transcode);
        assert_eq!(p.hdr.as_deref(), Some("arib-std-b67"));
        let ten_bit = Stream {
            pix_fmt: Some("yuv420p10le".to_owned()),
            profile: Some("High 10".to_owned()),
            ..h264()
        };
        let p = plan(&mov(ten_bit, None), false);
        assert_eq!((p.action, p.hdr), (Action::Transcode, None));
        assert!(video_plan(&Probe::default(), false).is_err());
        assert!(video_filter(Some("smpte2084")).contains("tonemap"));
        assert!(!video_filter(None).contains("tonemap"));
    }

    #[test]
    fn voice_messages_keep_aac_and_convert_the_rest() {
        let probe = |format: &str, codec: &str| Probe {
            format: format.to_owned(),
            audio: Some(stream("audio", codec)),
            ..Probe::default()
        };
        let plan = |p| audio_plan(&p).unwrap().action;
        assert_eq!(plan(probe("caf", "aac")), Action::Remux);
        assert_eq!(plan(probe("caf", "opus")), Action::Transcode);
        assert_eq!(plan(probe("amr", "amr_nb")), Action::Transcode);
        assert_eq!(plan(probe("caf", "pcm_s16le")), Action::Transcode);
        assert_eq!(
            plan(probe("mov,mp4,m4a,3gp,3g2,mj2", "aac")),
            Action::Original
        );
        assert!(audio_plan(&Probe::default()).is_err());
    }

    #[test]
    fn probe_output_is_parsed() {
        let json = br#"{"streams":[
            {"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}},
            {"codec_type":"video","codec_name":"hevc","pix_fmt":"yuv420p","width":1920,"height":1080},
            {"codec_type":"audio","codec_name":"aac"}],
            "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"12.5"}}"#;
        let p = parse_probe(json).unwrap();
        assert_eq!(p.duration, 12.5);
        assert_eq!(p.video.as_ref().unwrap().codec_name, "hevc");
        assert_eq!(p.video.as_ref().unwrap().pixels(), 1920 * 1080);
        assert_eq!(p.audio_codec(), Some("aac"));
        assert!(p.is_iso_media());
        assert!(parse_probe(b"{}").is_err());
    }

    #[test]
    fn disk_full_is_told_apart() {
        assert!(is_disk_full(
            "ffmpeg: exit status: 1: Error writing trailer: No space left on device"
        ));
        assert!(!is_disk_full("ffmpeg: exit status: 1: Invalid data"));
    }

    #[test]
    fn web_images_are_recognised_by_their_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let check = |bytes: &[u8]| {
            let p = dir.path().join("f");
            std::fs::write(&p, bytes).unwrap();
            is_web_image(&p).unwrap()
        };
        assert!(check(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]));
        assert!(check(b"\x89PNG\r\n\x1a\n...."));
        assert!(check(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(check(b"\0\0\0\x1cftypavif\0\0\0\0"));
        assert!(!check(b"\0\0\0\x18ftypheic\0\0\0\0"));
        assert!(!check(b""));
    }
}
