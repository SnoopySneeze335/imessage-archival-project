//! What each attachment file is, and the external tools that make
//! derivatives from it (decision 0005).
//!
//! Tools run inside the container only: ImageMagick 6's `convert` (see
//! decision 0001) for images and HEIC, and ffmpeg for video frames. Each call
//! is limited to one thread and a timeout, so a damaged file cannot hang an
//! import; how many run at once (`--jobs`) sets the CPU use. Every file
//! reaches a tool through [`input::Input`], which works around files the
//! tools misread.

pub mod convert;
pub mod input;

use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Longest side of a thumbnail, in pixels.
pub const THUMB_MAX: u32 = 400;
/// Version of the thumbnail settings below. Bump it when they change, so
/// existing thumbnails can be told apart from ones the new settings make.
pub const THUMB_RECIPE: i64 = 1;
/// Recipes of the derivatives the background queue makes (Milestone 3b,
/// decision 0007). Their settings are in [`convert`].
pub const PREVIEW_RECIPE: i64 = 1;
pub const AUDIO_RECIPE: i64 = 1;
pub const VIDEO_RECIPE: i64 = 1;

/// Time one tool call may take before it is killed and recorded as failed.
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

/// How a file is treated, decided from its attachment's MIME type, UTI and
/// file name (in that order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// JPEG, PNG, GIF or WebP: browsers show the original.
    WebImage,
    /// HEIC, TIFF and other images browsers cannot show: needs a preview.
    Image,
    Video,
    /// Voice messages and other Core Audio / AMR files: needs conversion.
    Voice,
    /// Audio browsers can play (MP3, AAC).
    Audio,
    Other,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WebImage => "web_image",
            Self::Image => "image",
            Self::Video => "video",
            Self::Voice => "voice",
            Self::Audio => "audio",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "web_image" => Self::WebImage,
            "image" => Self::Image,
            "video" => Self::Video,
            "voice" => Self::Voice,
            "audio" => Self::Audio,
            _ => Self::Other,
        }
    }

    /// Derivative kinds to make for a file of this kind, with their recipe.
    /// The thumbnail is made at import; the rest wait in the queue.
    pub fn derivatives(self) -> &'static [(&'static str, i64)] {
        match self {
            Self::WebImage => &[("thumb", THUMB_RECIPE)],
            Self::Image => &[("thumb", THUMB_RECIPE), ("preview", PREVIEW_RECIPE)],
            Self::Video => &[("thumb", THUMB_RECIPE), ("video", VIDEO_RECIPE)],
            Self::Voice => &[("audio", AUDIO_RECIPE)],
            Self::Audio | Self::Other => &[],
        }
    }

    /// Classifies a file from its attachment record.
    pub fn classify(mime: Option<&str>, uti: Option<&str>, name: Option<&str>) -> Self {
        if let Some(kind) = mime.and_then(|m| from_mime(&m.to_ascii_lowercase())) {
            return kind;
        }
        if let Some(kind) = uti.and_then(|u| from_uti(&u.to_ascii_lowercase())) {
            return kind;
        }
        let ext = name
            .and_then(|n| n.rsplit_once('.'))
            .map(|(_, e)| e.to_ascii_lowercase());
        ext.as_deref().map_or(Self::Other, from_extension)
    }
}

fn from_mime(m: &str) -> Option<MediaKind> {
    Some(match m {
        "image/jpeg" | "image/jpg" | "image/png" | "image/gif" | "image/webp" => {
            MediaKind::WebImage
        }
        // SVG is a document, not a picture to rasterize.
        "image/svg+xml" => MediaKind::Other,
        m if m.starts_with("image/") => MediaKind::Image,
        m if m.starts_with("video/") => MediaKind::Video,
        "audio/x-caf" | "audio/caf" | "audio/amr" | "audio/3gpp" => MediaKind::Voice,
        m if m.starts_with("audio/") => MediaKind::Audio,
        _ => return None,
    })
}

fn from_uti(u: &str) -> Option<MediaKind> {
    Some(match u {
        "public.jpeg" | "public.png" | "com.compuserve.gif" | "org.webmproject.webp" => {
            MediaKind::WebImage
        }
        "public.heic"
        | "public.heif"
        | "public.tiff"
        | "com.microsoft.bmp"
        | "com.adobe.raw-image"
        | "public.camera-raw-image" => MediaKind::Image,
        "com.apple.quicktime-movie" | "public.mpeg-4" | "public.movie" | "public.avi" => {
            MediaKind::Video
        }
        "com.apple.coreaudio-format" | "org.3gpp.adaptive-multi-rate-audio" => MediaKind::Voice,
        "public.mp3" | "public.mpeg-4-audio" | "com.apple.m4a-audio" => MediaKind::Audio,
        _ => return None,
    })
}

fn from_extension(e: &str) -> MediaKind {
    match e {
        "jpg" | "jpeg" | "png" | "gif" | "webp" => MediaKind::WebImage,
        "heic" | "heif" | "tif" | "tiff" | "bmp" | "dng" => MediaKind::Image,
        "mov" | "mp4" | "m4v" | "3gp" | "avi" => MediaKind::Video,
        "caf" | "amr" => MediaKind::Voice,
        "mp3" | "m4a" | "aac" | "wav" => MediaKind::Audio,
        _ => MediaKind::Other,
    }
}

/// Fails unless the tools for thumbnails are installed.
pub fn check_thumbnail_tools() -> Result<(), String> {
    check_tools(&["convert", "ffmpeg"])
}

/// Fails unless the tools for previews and conversions are installed.
pub fn check_conversion_tools() -> Result<(), String> {
    check_tools(&["convert", "ffmpeg", "ffprobe"])
}

fn check_tools(tools: &[&str]) -> Result<(), String> {
    for tool in tools {
        let ok = Command::new(tool)
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            return Err(format!("{tool} is not installed"));
        }
    }
    Ok(())
}

/// Writes a WebP thumbnail of `src` to `out`, at most [`THUMB_MAX`] pixels
/// on its longest side. Images use their first frame, videos their first
/// video frame. `work_dir` holds a patched copy of `src` while one is needed
/// (see [`input`]). Returns the size of `out`.
pub fn thumbnail(src: &Path, out: &Path, kind: MediaKind, work_dir: &Path) -> Result<u64, String> {
    let input = input::Input::prepare(src, work_dir)?;
    match kind {
        MediaKind::WebImage | MediaKind::Image => {
            match run_tool(image_still(&input, out, Still::THUMB), TOOL_TIMEOUT) {
                Ok(()) => {}
                // An MP4-family file ImageMagick cannot read, such as a HEIF
                // sequence with no still image or a video labelled as a
                // picture: use its first video frame instead.
                Err(first) if input.container() != input::Container::Other => {
                    run_tool(video_still(&input, out, Still::THUMB), TOOL_TIMEOUT)
                        .map_err(|second| shorten(&format!("{first}; then {second}")))?;
                }
                Err(e) => return Err(e),
            }
        }
        MediaKind::Video => run_tool(video_still(&input, out, Still::THUMB), TOOL_TIMEOUT)?,
        MediaKind::Voice | MediaKind::Audio | MediaKind::Other => {
            return Err(format!("no thumbnail for {} files", kind.as_str()));
        }
    }
    match std::fs::metadata(out) {
        Ok(m) if m.len() > 0 => Ok(m.len()),
        _ => Err("tool succeeded but wrote no output".to_owned()),
    }
}

/// Settings for a WebP still image: a thumbnail or a preview.
#[derive(Debug, Clone, Copy)]
pub struct Still {
    /// Longest side, in pixels; smaller images keep their size.
    pub max: u32,
    pub quality: u32,
    /// Keep the colour profile (previews), or strip everything (thumbnails).
    pub keep_icc: bool,
}

impl Still {
    pub const THUMB: Self = Self {
        max: THUMB_MAX,
        quality: 80,
        keep_icc: false,
    };
}

/// ImageMagick: the first frame of `input` as WebP.
pub(crate) fn image_still(input: &input::Input, out: &Path, s: Still) -> Command {
    let size = format!("{0}x{0}>", s.max);
    let mut cmd = Command::new("convert");
    cmd.args(["-limit", "thread", "1"])
        .args(["-limit", "memory", "256MiB"])
        .args(["-limit", "map", "512MiB"])
        // Lets libjpeg decode large JPEGs at a reduced scale.
        .args(["-define", &format!("jpeg:size={0}x{0}", s.max * 2)])
        .arg(input.magick_first_frame())
        .arg("-auto-orient");
    if s.keep_icc {
        // Every profile but the colour one (Display P3 on iPhones), so
        // browsers show the colours as shot.
        cmd.args(["-resize", &size, "+profile", "!icc,*"]);
    } else {
        cmd.args(["-thumbnail", &size, "-strip"]);
    }
    cmd.args(["-quality", &s.quality.to_string()])
        .arg(format!("webp:{}", out.display()));
    cmd
}

/// ffmpeg: the first video frame of `input` as WebP.
pub(crate) fn video_still(input: &input::Input, out: &Path, s: Still) -> Command {
    let scale = format!(
        "scale='min({0},iw)':'min({0},ih)':force_original_aspect_ratio=decrease",
        s.max
    );
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-v", "error", "-threads", "1", "-i"])
        .arg(input.path())
        .args(["-map", "0:v:0", "-frames:v", "1", "-vf", &scale])
        .args(["-c:v", "libwebp", "-quality", &s.quality.to_string()])
        .args(["-f", "webp", "-y"])
        .arg(out);
    cmd
}

/// Runs `cmd` with no input, killing it after `timeout`. On failure, returns
/// the start of its error output.
pub(crate) fn run_tool(cmd: Command, timeout: Duration) -> Result<(), String> {
    run_tool_output(cmd, timeout).map(drop)
}

/// Like [`run_tool`], and returns the tool's standard output (at most 1 MB).
pub(crate) fn run_tool_output(mut cmd: Command, timeout: Duration) -> Result<Vec<u8>, String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut stdout).take(1 << 20).read_to_end(&mut buf);
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
        buf
    });
    // Read error output on its own thread so a chatty tool cannot block on a
    // full pipe; keep only the start of it.
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut stderr).take(4096).read_to_end(&mut buf);
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
        buf
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("{program}: timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => break Err(format!("{program}: {e}")),
        }
    };
    let output = reader.join().unwrap_or_default();
    let stdout = out_reader.join().unwrap_or_default();
    let status = status?;
    if status.success() {
        return Ok(stdout);
    }
    let text = String::from_utf8_lossy(&output);
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    Err(shorten(&format!("{program}: {status}: {first}")))
}

/// Error text short enough to store.
pub(crate) fn shorten(s: &str) -> String {
    const MAX: usize = 300;
    if s.len() <= MAX {
        return s.to_owned();
    }
    let mut end = MAX;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_prefers_mime_then_uti_then_extension() {
        use MediaKind::*;
        assert_eq!(MediaKind::classify(Some("image/heic"), None, None), Image);
        assert_eq!(
            MediaKind::classify(Some("IMAGE/JPEG"), None, None),
            WebImage
        );
        assert_eq!(
            MediaKind::classify(None, Some("com.apple.quicktime-movie"), None),
            Video
        );
        assert_eq!(
            MediaKind::classify(Some("audio/x-caf"), Some("public.png"), None),
            Voice
        );
        assert_eq!(
            MediaKind::classify(None, Some("dyn.unknown"), Some("Audio Message.caf")),
            Voice
        );
        assert_eq!(
            MediaKind::classify(Some("image/svg+xml"), None, None),
            Other
        );
        assert_eq!(MediaKind::classify(None, None, Some("notes.pdf")), Other);
        assert_eq!(MediaKind::classify(None, None, None), Other);
        for k in [WebImage, Image, Video, Voice, Audio, Other] {
            assert_eq!(MediaKind::parse(k.as_str()), k);
        }
    }

    #[test]
    fn errors_are_shortened_on_a_char_boundary() {
        let long = "é".repeat(400);
        let s = shorten(&long);
        assert!(s.len() <= 304 && s.ends_with('…'));
    }
}
