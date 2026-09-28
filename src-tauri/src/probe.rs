//! O que o `ffprobe` diz de um arquivo: a duração e se há trilha de áudio e de vídeo.
//!
//! A capa embutida de um MP3 ou M4A aparece no `ffprobe` como trilha de vídeo, com a
//! disposição `attached_pic`. Ela não conta como vídeo: sem essa distinção, um podcast com
//! capa seria tratado como vídeo de um quadro só.

use std::ffi::OsString;
use std::path::Path;

use serde::Deserialize;

use crate::error::AppError;
use crate::tool::{self, Tool};

/// A duração e as trilhas de um arquivo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaInfo {
    /// A duração, em segundos; sempre finita e positiva.
    pub duration_secs: f64,
    /// Se há trilha de vídeo que não seja capa embutida.
    pub has_video: bool,
    /// Se há trilha de áudio.
    pub has_audio: bool,
}

/// Lê a duração e as trilhas de `path`.
///
/// # Errors
/// [`AppError::ToolMissing`] ou [`AppError::ToolSpawn`] se o `ffprobe` não iniciar;
/// [`AppError::UnreadableMedia`] se ele não reconhecer o arquivo ou não achar a duração.
pub fn probe(path: &Path) -> Result<MediaInfo, AppError> {
    let args: Vec<OsString> = ["-v", "error", "-print_format", "json", "-show_format", "-show_streams"]
        .into_iter()
        .map(OsString::from)
        .chain([tool::file_arg(path)])
        .collect();
    let captured = tool::run_captured(Tool::Ffprobe, &args)?;
    if !captured.success {
        let detail = captured.stderr.trim();
        return Err(AppError::UnreadableMedia(format!(
            "ffprobe {}: {detail}",
            captured.status
        )));
    }
    parse_probe(&captured.stdout)
}

#[derive(Debug, Deserialize)]
struct ProbeReport {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    duration: Option<String>,
    #[serde(default)]
    disposition: Disposition,
}

#[derive(Debug, Default, Deserialize)]
struct Disposition {
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

/// Interpreta o JSON do `ffprobe -show_format -show_streams`.
///
/// A duração é a do contêiner; na falta dela — fluxos crus, alguns MPEG-TS —, a maior das
/// trilhas.
///
/// # Errors
/// [`AppError::UnreadableMedia`] se o JSON não parsear ou não houver duração positiva.
pub fn parse_probe(json: &str) -> Result<MediaInfo, AppError> {
    let report: ProbeReport =
        serde_json::from_str(json).map_err(|error| AppError::UnreadableMedia(format!("ffprobe report: {error}")))?;
    let container = report
        .format
        .and_then(|format| positive_secs(format.duration.as_deref()));
    let duration_secs = container
        .or_else(|| {
            report
                .streams
                .iter()
                .filter_map(|stream| positive_secs(stream.duration.as_deref()))
                .reduce(f64::max)
        })
        .ok_or_else(|| AppError::UnreadableMedia("the media reports no duration".to_owned()))?;
    let is = |kind: &str, stream: &ProbeStream| stream.codec_type.as_deref() == Some(kind);
    Ok(MediaInfo {
        duration_secs,
        has_video: report
            .streams
            .iter()
            .any(|stream| is("video", stream) && stream.disposition.attached_pic == 0),
        has_audio: report.streams.iter().any(|stream| is("audio", stream)),
    })
}

fn positive_secs(text: Option<&str>) -> Option<f64> {
    let secs: f64 = text?.trim().parse().ok()?;
    (secs.is_finite() && secs > 0.0).then_some(secs)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{MediaInfo, parse_probe};
    use crate::error::AppError;

    type TestResult = Result<(), Box<dyn Error>>;

    #[test]
    fn a_video_with_audio() -> TestResult {
        let json = r#"{
            "streams": [
                { "codec_type": "video", "duration": "12.000000", "disposition": { "attached_pic": 0 } },
                { "codec_type": "audio", "duration": "12.010000", "disposition": { "attached_pic": 0 } }
            ],
            "format": { "duration": "12.010000" }
        }"#;
        let info = parse_probe(json)?;
        assert_eq!(
            info,
            MediaInfo {
                duration_secs: 12.01,
                has_video: true,
                has_audio: true
            }
        );
        Ok(())
    }

    /// A capa de um MP3 não faz dele um vídeo.
    #[test]
    fn a_cover_picture_is_not_video() -> TestResult {
        let json = r#"{
            "streams": [
                { "codec_type": "audio" },
                { "codec_type": "video", "disposition": { "attached_pic": 1 } }
            ],
            "format": { "duration": "61.5" }
        }"#;
        let info = parse_probe(json)?;
        assert!(info.has_audio);
        assert!(!info.has_video, "a capa embutida não é trilha de vídeo");
        Ok(())
    }

    #[test]
    fn without_a_container_duration_the_longest_stream_counts() -> TestResult {
        let json = r#"{
            "streams": [
                { "codec_type": "audio", "duration": "9.5" },
                { "codec_type": "video", "duration": "10.25" }
            ],
            "format": {}
        }"#;
        assert!((parse_probe(json)?.duration_secs - 10.25).abs() < f64::EPSILON);
        Ok(())
    }

    #[test]
    fn a_report_without_duration_or_json_is_unreadable() {
        for json in [
            r#"{ "streams": [{ "codec_type": "audio" }], "format": { "duration": "N/A" } }"#,
            r#"{ "streams": [], "format": { "duration": "0" } }"#,
            "not json",
        ] {
            assert!(
                matches!(parse_probe(json), Err(AppError::UnreadableMedia(_))),
                "{json} deveria ser ilegível"
            );
        }
    }
}
