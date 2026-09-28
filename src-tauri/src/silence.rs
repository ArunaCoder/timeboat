//! A análise do áudio: o `ffmpeg` com o filtro `silencedetect`, e a leitura do que ele relata.
//!
//! O filtro escreve no stderr uma linha `silence_start: <s>` quando o nível cai abaixo do
//! limiar por pelo menos a janela pedida, e uma `silence_end: <s> | silence_duration: <s>`
//! quando volta. Um silêncio que vai até o fim do arquivo pode ficar sem `silence_end`
//! (depende da versão do `ffmpeg`), e é fechado aqui na duração do arquivo.
//!
//! O que o filtro relata é o silêncio **bruto**; decidir o que se corta é do
//! [`crate::timeline`].

use std::ffi::OsString;
use std::path::Path;

use crate::settings::Settings;
use crate::timeline::Interval;
use crate::tool;

/// Os argumentos do `ffmpeg` que analisam o primeiro áudio de `input`, sem gravar nada, com
/// o progresso no stdout.
#[must_use]
pub fn analysis_args(input: &Path, settings: &Settings) -> Vec<OsString> {
    let filter = format!(
        "silencedetect=noise={:.3}dB:d={:.3}",
        settings.threshold_db,
        settings.detection_window()
    );
    let mut args: Vec<OsString> = ["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info"]
        .into_iter()
        .map(OsString::from)
        .collect();
    args.extend(["-progress", "pipe:1", "-i"].map(OsString::from));
    args.push(tool::file_arg(input));
    args.extend(["-map", "0:a:0", "-af", &filter, "-f", "null", "-"].map(OsString::from));
    args
}

/// Os silêncios que o `silencedetect` relatou em `stderr`, na ordem, com o silêncio final
/// sem `silence_end` fechado em `duration_secs`.
#[must_use]
pub fn parse_silences(stderr: &str, duration_secs: f64) -> Vec<Interval> {
    let mut silences = Vec::new();
    let mut open: Option<f64> = None;
    for line in stderr.lines().filter(|line| line.contains("[silencedetect @")) {
        if let Some(start) = value_after(line, "silence_start:") {
            open = Some(start);
        } else if let Some(end) = value_after(line, "silence_end:") {
            silences.push(Interval::new(open.take().unwrap_or(0.0), end));
        }
    }
    if let Some(start) = open {
        silences.push(Interval::new(start, duration_secs));
    }
    silences
}

/// O número logo depois de `key` em `line`.
fn value_after(line: &str, key: &str) -> Option<f64> {
    let (_, rest) = line.split_once(key)?;
    rest.split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{analysis_args, parse_silences};
    use crate::settings::DEFAULTS;
    use crate::timeline::Interval;

    /// A saída real do `ffmpeg` 8, com as linhas vizinhas que não são do filtro.
    const STDERR: &str = "\
Input #0, wav, from 'file:C:\\x\\t.wav':
  Duration: 00:00:12.00, bitrate: 705 kb/s
[silencedetect @ 000001dfdc027600] silence_start: 0.999977
[silencedetect @ 000001dfdc027600] silence_end: 5 | silence_duration: 4.000023
[silencedetect @ 000001dfdc027600] silence_start: -0.0213
[silencedetect @ 000001dfdc027600] silence_end: 0.5 | silence_duration: 0.5213
[silencedetect @ 000001dfdc027600] silence_start: 9.25
[out#0/null @ 00000166eeeca700] video:0KiB audio:172KiB
";

    #[test]
    fn silences_are_paired_and_the_last_one_closes_at_the_end() {
        let silences = parse_silences(STDERR, 12.0);
        assert_eq!(
            silences,
            [
                Interval::new(0.999_977, 5.0),
                Interval::new(-0.0213, 0.5),
                Interval::new(9.25, 12.0),
            ]
        );
    }

    #[test]
    fn no_report_means_no_silence() {
        assert!(parse_silences("Input #0, wav\n", 3.0).is_empty());
    }

    /// Os parâmetros do filtro saem das réguas: o limiar em dB e a janela de detecção.
    #[test]
    fn the_filter_carries_the_settings() {
        let args = analysis_args(Path::new(r"C:\a.wav"), &DEFAULTS);
        let args: Vec<String> = args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert!(
            args.contains(&"silencedetect=noise=-40.000dB:d=0.750".to_owned()),
            "{args:?}"
        );
        assert!(args.contains(&r"file:C:\a.wav".to_owned()), "{args:?}");
        assert!(args.windows(2).any(|pair| pair == ["-progress", "pipe:1"]), "{args:?}");
    }
}
