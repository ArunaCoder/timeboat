//! A gravação do resultado: o grafo de filtros que junta os trechos mantidos, e os
//! argumentos do `ffmpeg` que o aplicam.
//!
//! # Por que `trim` + `concat`, e não cópia de fluxo
//!
//! Copiar o fluxo sem recodificar só corta em quadro-chave, que em vídeo comprimido pode
//! estar segundos longe do ponto pedido — o corte sairia no lugar errado. E o atalho do
//! `select`, com `setpts=N/FRAME_RATE/TB`, supõe taxa de quadros constante, que gravação de
//! tela e de celular raramente têm: o áudio dessincronizaria. Cada trecho é recortado com
//! `trim`/`atrim`, tem o relógio zerado e entra no `concat`, que costura os trechos com o
//! áudio e o vídeo alinhados trecho a trecho.
//!
//! # O grafo vai por arquivo
//!
//! Um arquivo longo com centenas de cortes gera um grafo maior que o limite de 32 767
//! caracteres da linha de comando do Windows. O grafo vai num arquivo temporário
//! ([`ScriptFile`]), lido pelo `ffmpeg` com a sintaxe `-/filter_complex <arquivo>`.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{env, process};

use crate::error::AppError;
use crate::media::OutputProfile;
use crate::timeline::CutPlan;
use crate::tool;

/// O rótulo da saída de vídeo do grafo.
const VIDEO_OUT: &str = "[outv]";
/// O rótulo da saída de áudio do grafo.
const AUDIO_OUT: &str = "[outa]";

/// O grafo de filtros que recorta os trechos de `plan` e os concatena, com ou sem vídeo.
///
/// Usa o primeiro vídeo e o primeiro áudio da entrada, e expõe as saídas `[outv]` e `[outa]`.
#[must_use]
pub fn filter_script(plan: &CutPlan, with_video: bool) -> String {
    let segments = plan.keep();
    let count = segments.len();
    let mut statements = Vec::with_capacity(2 * count + 3);
    let mut concat_inputs = Vec::with_capacity(2 * count);
    if with_video {
        statements.push(split_statement("[0:v:0]split", 'v', count));
    }
    statements.push(split_statement("[0:a:0]asplit", 'a', count));
    for (index, segment) in segments.iter().enumerate() {
        let window = format!("start={:.6}:end={:.6}", segment.start, segment.end);
        if with_video {
            statements.push(format!("[v{index}]trim={window},setpts=PTS-STARTPTS[vs{index}];"));
            concat_inputs.push(format!("[vs{index}]"));
        }
        statements.push(format!("[a{index}]atrim={window},asetpts=PTS-STARTPTS[as{index}];"));
        concat_inputs.push(format!("[as{index}]"));
    }
    let (video_streams, outputs) = if with_video {
        (1, format!("{VIDEO_OUT}{AUDIO_OUT}"))
    } else {
        (0, AUDIO_OUT.to_owned())
    };
    statements.push(format!(
        "{}concat=n={count}:v={video_streams}:a=1{outputs}",
        concat_inputs.concat()
    ));
    // A linha vazia do fim dá ao script o newline final.
    statements.push(String::new());
    statements.join("\n")
}

/// `[0:a:0]asplit=3[a0][a1][a2];` e o equivalente de vídeo.
fn split_statement(head: &str, label: char, count: usize) -> String {
    let outputs: Vec<String> = (0..count).map(|index| format!("[{label}{index}]")).collect();
    format!("{head}={count}{};", outputs.concat())
}

/// Os argumentos do `ffmpeg` que gravam em `output` o resultado do grafo em `script`.
///
/// Metadados do arquivo seguem para o resultado; capítulos não, porque os tempos deles
/// deixariam de valer depois dos cortes.
#[must_use]
pub fn render_args(
    input: &Path,
    script: &Path,
    output: &Path,
    profile: OutputProfile,
    with_video: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-nostdin", "-nostats", "-loglevel", "error"]
        .into_iter()
        .map(OsString::from)
        .collect();
    // `-y` porque a saída é o parcial que a reserva já criou vazio (`media::Reservation`).
    args.extend(["-progress", "pipe:1", "-y", "-i"].map(OsString::from));
    args.push(tool::file_arg(input));
    args.push(OsString::from("-/filter_complex"));
    args.push(script.as_os_str().to_owned());
    if with_video {
        args.extend(["-map", VIDEO_OUT].map(OsString::from));
    }
    args.extend(["-map", AUDIO_OUT, "-map_metadata", "0", "-map_chapters", "-1"].map(OsString::from));
    args.extend(profile.codec_args().iter().map(OsString::from));
    args.push(tool::file_arg(output));
    args
}

/// Numeração dos scripts deste processo, para dois nunca disputarem o mesmo nome.
static NEXT_SCRIPT: AtomicU64 = AtomicU64::new(0);

/// O grafo de filtros gravado na pasta temporária, apagado quando sai de escopo.
#[derive(Debug)]
pub struct ScriptFile(PathBuf);

impl ScriptFile {
    /// Grava `contents` num arquivo novo da pasta temporária do sistema.
    ///
    /// # Errors
    /// [`AppError::TempFile`] se o arquivo não puder ser criado ou escrito.
    pub fn create(contents: &str) -> Result<Self, AppError> {
        let serial = NEXT_SCRIPT.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("timeboat-{}-{serial}.filtergraph", process::id()));
        let written = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut file| file.write_all(contents.as_bytes()));
        match written {
            Ok(()) => Ok(Self(path)),
            Err(error) => {
                remove_script(&path);
                Err(AppError::TempFile(error))
            }
        }
    }

    /// O caminho do arquivo.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScriptFile {
    fn drop(&mut self) {
        remove_script(&self.0);
    }
}

fn remove_script(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("could not remove the filter script {}: {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::Path;

    use super::{ScriptFile, filter_script, render_args};
    use crate::media::OutputProfile;
    use crate::settings::DEFAULTS;
    use crate::timeline::{CutPlan, Interval, Verdict, plan};

    type TestResult = Result<(), Box<dyn Error>>;

    /// Um plano com dois trechos mantidos: [0, 10.25] e [19.75, 30].
    fn two_segments() -> Result<CutPlan, Box<dyn Error>> {
        match plan(&[Interval::new(10.0, 20.0)], 30.0, &DEFAULTS) {
            Verdict::Cut(plan) => Ok(plan),
            other => Err(format!("esperava um corte, veio {other:?}").into()),
        }
    }

    #[test]
    fn the_video_graph_trims_both_streams_and_concatenates_in_pairs() -> TestResult {
        let script = filter_script(&two_segments()?, true);
        let expected = "\
[0:v:0]split=2[v0][v1];
[0:a:0]asplit=2[a0][a1];
[v0]trim=start=0.000000:end=10.250000,setpts=PTS-STARTPTS[vs0];
[a0]atrim=start=0.000000:end=10.250000,asetpts=PTS-STARTPTS[as0];
[v1]trim=start=19.750000:end=30.000000,setpts=PTS-STARTPTS[vs1];
[a1]atrim=start=19.750000:end=30.000000,asetpts=PTS-STARTPTS[as1];
[vs0][as0][vs1][as1]concat=n=2:v=1:a=1[outv][outa]
";
        assert_eq!(script, expected);
        Ok(())
    }

    #[test]
    fn the_audio_graph_has_no_video_branch() -> TestResult {
        let script = filter_script(&two_segments()?, false);
        assert!(!script.contains("[0:v:0]"), "{script}");
        assert!(script.ends_with("[as0][as1]concat=n=2:v=0:a=1[outa]\n"), "{script}");
        Ok(())
    }

    #[test]
    fn audio_output_maps_only_the_audio_label() {
        let args = render_args(
            Path::new(r"C:\in.mp3"),
            Path::new(r"C:\t\g.filtergraph"),
            Path::new(r"C:\out.mp3"),
            OutputProfile::Mp3,
            false,
        );
        let args: Vec<String> = args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert!(!args.contains(&"[outv]".to_owned()), "{args:?}");
        assert!(args.windows(2).any(|pair| pair == ["-map", "[outa]"]), "{args:?}");
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-/filter_complex", r"C:\t\g.filtergraph"])
        );
        assert_eq!(args.last().map(String::as_str), Some(r"file:C:\out.mp3"));
    }

    #[test]
    fn the_script_file_lives_while_in_scope() -> TestResult {
        let script = ScriptFile::create("anullsrc")?;
        let path = script.path().to_owned();
        assert_eq!(fs::read_to_string(&path)?, "anullsrc");
        drop(script);
        assert!(!path.exists(), "o script temporário deveria ter sido apagado");
        Ok(())
    }
}
