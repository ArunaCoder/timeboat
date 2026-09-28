//! Um processamento de ponta a ponta — ler, analisar, planejar, gravar —, sem Tauri.
//!
//! O que conversa com a janela (eventos, estado, thread) é do [`crate::job`]; aqui só há
//! arquivos, o `ffmpeg` e um callback de progresso. É essa separação que deixa a suíte
//! exercitar o caminho inteiro contra o `ffmpeg` de verdade, com arquivos sintetizados na
//! hora, sem subir app nenhum.
//!
//! # Uma barra só para as duas etapas
//!
//! O progresso que sai daqui é o do processamento **inteiro**, e não o de cada etapa: uma
//! barra que enchesse na análise e voltasse a zero na gravação diria duas vezes que está
//! acabando. A análise ocupa o começo da barra na proporção do que ela custa — só decodifica
//! o áudio, então é uma fatia pequena quando a gravação recodifica vídeo, e perto da metade
//! quando só há áudio dos dois lados (`analysis_share`).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::AppError;
use crate::media::{OutputProfile, Reservation};
use crate::probe::{self, MediaInfo};
use crate::render::{self, ScriptFile};
use crate::settings::Settings;
use crate::silence;
use crate::timeline::{self, CutPlan, Verdict};
use crate::tool::{self, CancelToken, RunOutcome};

/// A etapa em andamento, para a tela dizer o que está acontecendo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// O `silencedetect` percorre o áudio.
    Analyzing,
    /// O `ffmpeg` grava o resultado.
    Rendering,
}

/// O resultado gravado.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    /// Onde o resultado foi gravado.
    pub output: PathBuf,
    /// A duração do arquivo original, em segundos.
    pub original_secs: f64,
    /// A duração do resultado, em segundos, pelo plano de cortes.
    pub final_secs: f64,
    /// Quantos trechos de silêncio foram removidos.
    pub cut_count: usize,
    /// Se o resultado tem vídeo (ou é só áudio).
    pub video: bool,
}

/// Como terminou um processamento que não falhou.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// O resultado foi gravado.
    Rendered(Rendered),
    /// Nenhum silêncio passou das réguas; nada foi gravado.
    NothingToRemove,
    /// O arquivo inteiro seria cortado; nada foi gravado.
    AllSilent,
    /// Interrompido a pedido; nada ficou na pasta.
    Cancelled,
}

/// O pedido de um processamento: o arquivo, como gravá-lo, com que réguas, e por onde ele
/// pode ser interrompido.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// O arquivo de entrada.
    pub input: &'a Path,
    /// O perfil de codificação do resultado.
    pub profile: OutputProfile,
    /// As réguas da detecção.
    pub settings: &'a Settings,
    /// O pedido de interrupção.
    pub token: &'a CancelToken,
}

/// Remove os silêncios do arquivo pedido, gravando o resultado na mesma pasta.
///
/// `on_progress` recebe a etapa em andamento e a fração do processamento **inteiro** já
/// cumprida, de 0 a 1, que nunca volta atrás.
///
/// # Errors
/// [`AppError::NoAudio`] se o arquivo não tiver áudio; os erros de [`probe::probe`],
/// [`tool::run_with_progress`] e [`Reservation`] para as falhas de leitura, do `ffmpeg` e
/// da pasta.
pub fn process(request: &Request<'_>, on_progress: &mut dyn FnMut(Phase, f64)) -> Result<Outcome, AppError> {
    let info = probe::probe(request.input)?;
    if !info.has_audio {
        return Err(AppError::NoAudio);
    }
    let share = analysis_share(renders_video(request.profile, &info));
    let mut on_overall = |phase, fraction| on_progress(phase, overall(phase, fraction, share));
    let Some(analysis) = analyze(request, &info, &mut on_overall)? else {
        return Ok(Outcome::Cancelled);
    };
    // A duração do `ffprobe` pode ser estimativa (MP3 VBR sem cabeçalho Xing, contêiner de
    // gravação interrompida) e ficar aquém da real; o que passasse dela seria cortado como se
    // não existisse. A análise decodifica o áudio inteiro, e vale a maior das duas.
    let info = MediaInfo {
        duration_secs: info.duration_secs.max(analysis.decoded_secs),
        ..info
    };
    let silences = silence::parse_silences(&analysis.stderr, info.duration_secs);
    match timeline::plan(&silences, info.duration_secs, request.settings) {
        Verdict::NothingToRemove => Ok(Outcome::NothingToRemove),
        Verdict::AllSilent => Ok(Outcome::AllSilent),
        Verdict::Cut(plan) => write(request, &info, &plan, &mut on_overall),
    }
}

/// Se o resultado carrega vídeo: o perfil o mantém e a entrada tem uma trilha de verdade.
const fn renders_video(profile: OutputProfile, info: &MediaInfo) -> bool {
    profile.keeps_video() && info.has_video
}

/// A fatia da barra que a análise ocupa. Estimativa, e não medição: a análise decodifica só
/// o áudio, dezenas de vezes mais rápido que o tempo real; a gravação de vídeo decodifica e
/// recodifica cada quadro, e é ela que domina o tempo.
const fn analysis_share(renders_video: bool) -> f64 {
    if renders_video { 0.05 } else { 0.4 }
}

/// A posição na barra única, dada a etapa e a fração cumprida dela.
fn overall(phase: Phase, fraction: f64, analysis_share: f64) -> f64 {
    match phase {
        Phase::Analyzing => fraction * analysis_share,
        Phase::Rendering => fraction.mul_add(1.0 - analysis_share, analysis_share),
    }
}

/// O que a análise relatou.
struct Analysis {
    /// O stderr, com os relatos do `silencedetect`.
    stderr: String,
    /// Até onde o áudio foi decodificado, em segundos: a duração real do áudio.
    decoded_secs: f64,
}

/// A análise do áudio, ou `None` se ela foi interrompida.
fn analyze(
    request: &Request<'_>,
    info: &MediaInfo,
    on_progress: &mut dyn FnMut(Phase, f64),
) -> Result<Option<Analysis>, AppError> {
    let args = silence::analysis_args(request.input, request.settings);
    let mut decoded_secs = 0.0_f64;
    let mut report = |position: f64| {
        decoded_secs = decoded_secs.max(position);
        on_progress(Phase::Analyzing, fraction(position, info.duration_secs));
    };
    match tool::run_with_progress(&args, request.token, &mut report)? {
        RunOutcome::Completed { stderr } => Ok(Some(Analysis { stderr, decoded_secs })),
        RunOutcome::Cancelled => Ok(None),
    }
}

/// Grava os trechos mantidos de `plan` no nome reservado ao lado da entrada.
fn write(
    request: &Request<'_>,
    info: &MediaInfo,
    plan: &CutPlan,
    on_progress: &mut dyn FnMut(Phase, f64),
) -> Result<Outcome, AppError> {
    if request.token.is_cancelled() {
        return Ok(Outcome::Cancelled);
    }
    let Request { input, profile, .. } = *request;
    let with_video = renders_video(profile, info);
    let script = ScriptFile::create(&render::filter_script(plan, with_video))?;
    let reservation = Reservation::reserve(input, profile)?;
    let args = render::render_args(input, script.path(), reservation.partial_path(), profile, with_video);
    let final_secs = plan.kept_secs();
    let mut report = |position: f64| on_progress(Phase::Rendering, fraction(position, final_secs));
    match tool::run_with_progress(&args, request.token, &mut report)? {
        // A reserva sai de escopo sem `commit` e apaga o parcial e o nome reservado.
        RunOutcome::Cancelled => Ok(Outcome::Cancelled),
        RunOutcome::Completed { .. } => Ok(Outcome::Rendered(Rendered {
            output: reservation.commit()?,
            original_secs: info.duration_secs,
            final_secs,
            cut_count: plan.cut_count(),
            video: with_video,
        })),
    }
}

/// A fração de `total` que `position` representa, presa entre 0 e 1.
fn fraction(position: f64, total: f64) -> f64 {
    if total > 0.0 {
        (position / total).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    //! Contra o `ffmpeg` de verdade, que precisa estar no `PATH` — a mesma exigência do app.
    //! Os arquivos são sintetizados pelo próprio `ffmpeg` (`lavfi`): um tom de 440 Hz com um
    //! trecho de silêncio digital no meio.

    use std::error::Error;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{Outcome, Phase, Request, analysis_share, fraction, overall, process};
    use crate::error::AppError;
    use crate::media::OutputProfile;
    use crate::probe;
    use crate::settings::DEFAULTS;
    use crate::test_support::TestDir;
    use crate::tool::{self, CancelToken, Tool};

    type TestResult = Result<(), Box<dyn Error>>;

    /// Tom de 0 a `from`, silêncio de `from` a `to`, tom de `to` a `total`.
    fn tone_with_gap(from: f64, to: f64, total: f64) -> String {
        format!("aevalsrc='if(between(t,{from},{to}),0,0.5*sin(2*PI*440*t))':d={total}:s=48000")
    }

    /// Sintetiza `name` em `dir` com o áudio de [`tone_with_gap`] e, se `video`, um vídeo de
    /// teste da mesma duração.
    fn synthesize(dir: &Path, name: &str, gap: (f64, f64), total: f64, video: bool) -> Result<PathBuf, Box<dyn Error>> {
        let path = dir.join(name);
        let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y"].map(str::to_owned).into();
        if video {
            args.extend(["-f", "lavfi", "-i"].map(str::to_owned));
            args.push(format!("testsrc2=size=320x240:rate=25:duration={total}"));
        }
        args.extend(["-f", "lavfi", "-i"].map(str::to_owned));
        args.push(tone_with_gap(gap.0, gap.1, total));
        if video {
            args.extend(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest"].map(str::to_owned));
        }
        args.push(path.to_string_lossy().into_owned());
        let args: Vec<OsString> = args.into_iter().map(OsString::from).collect();
        let captured = tool::run_captured(Tool::Ffmpeg, &args)?;
        if !captured.success {
            return Err(format!("o ffmpeg não sintetizou {name}: {}", captured.stderr).into());
        }
        Ok(path)
    }

    /// Processa `input` com os padrões, repassando o progresso a `on_progress`.
    fn run(
        input: &Path,
        profile: OutputProfile,
        token: &CancelToken,
        on_progress: &mut dyn FnMut(Phase, f64),
    ) -> Result<Outcome, AppError> {
        let request = Request {
            input,
            profile,
            settings: &DEFAULTS,
            token,
        };
        process(&request, on_progress)
    }

    fn file_names(dir: &Path) -> Result<Vec<String>, Box<dyn Error>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(dir)? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    /// 7 s com 5 s de silêncio no meio: com os padrões, o corte vai de 1,25 s a 5,75 s e
    /// sobram 2,5 s — e o arquivo gravado tem mesmo essa duração. A barra passa pelas duas
    /// etapas sem nunca voltar atrás.
    #[test]
    fn an_audio_file_loses_its_long_silence() -> TestResult {
        let dir = TestDir::new("pipeline-audio")?;
        let input = synthesize(dir.path(), "fala.wav", (1.0, 6.0), 7.0, false)?;
        let mut ticks = Vec::new();
        let outcome = run(
            &input,
            OutputProfile::Wav,
            &CancelToken::default(),
            &mut |phase, fraction| {
                ticks.push((phase, fraction));
            },
        )?;

        let Outcome::Rendered(rendered) = outcome else {
            return Err(format!("esperava o arquivo gravado, veio {outcome:?}").into());
        };
        assert_eq!(rendered.output, dir.path().join("fala (TIMEBOATED).wav"));
        assert_eq!(rendered.cut_count, 1);
        assert!(!rendered.video);
        assert!((rendered.final_secs - 2.5).abs() < 0.05, "{rendered:?}");
        let written = probe::probe(&rendered.output)?;
        assert!((written.duration_secs - 2.5).abs() < 0.05, "{written:?}");
        assert_eq!(file_names(dir.path())?, ["fala (TIMEBOATED).wav", "fala.wav"]);

        assert!(ticks.iter().any(|&(phase, _)| phase == Phase::Analyzing), "{ticks:?}");
        assert!(ticks.iter().any(|&(phase, _)| phase == Phase::Rendering), "{ticks:?}");
        assert!(
            ticks.windows(2).all(|pair| pair[0].1 <= pair[1].1),
            "a barra voltou atrás: {ticks:?}"
        );
        let last = ticks.last().map_or(0.0, |&(_, fraction)| fraction);
        assert!((last - 1.0).abs() < 0.01, "a barra deveria terminar cheia: {ticks:?}");
        Ok(())
    }

    /// A interrupção no meio da gravação encerra o `ffmpeg` e apaga o parcial que ele já
    /// escrevia: a pasta volta a ter só o original.
    #[test]
    fn cancelling_during_the_render_leaves_only_the_original() -> TestResult {
        let dir = TestDir::new("pipeline-cancel-render")?;
        let input = synthesize(dir.path(), "longa.mkv", (5.0, 15.0), 240.0, true)?;
        let token = CancelToken::default();
        let mut cancelled_at = None;
        let mut partial_existed = false;
        let outcome = run(&input, OutputProfile::Mp4, &token, &mut |phase, fraction| {
            if phase == Phase::Rendering && cancelled_at.is_none() {
                partial_existed = fs::read_dir(dir.path()).is_ok_and(|entries| {
                    entries
                        .flatten()
                        .any(|entry| entry.file_name().to_string_lossy().contains(".parcial."))
                });
                cancelled_at = Some(fraction);
                token.cancel();
            }
        })?;

        assert_eq!(outcome, Outcome::Cancelled);
        let fraction = cancelled_at.ok_or("a gravação deveria ter relatado progresso")?;
        assert!(
            fraction < 1.0,
            "a interrupção tem de cair no meio da gravação ({fraction})"
        );
        assert!(
            partial_existed,
            "o parcial deveria existir quando a interrupção foi pedida"
        );
        assert_eq!(file_names(dir.path())?, ["longa.mkv"]);
        Ok(())
    }

    /// A análise enche a primeira fatia, a gravação o resto; as duas se encontram na
    /// fronteira, e a fatia da análise é menor quando há vídeo a recodificar.
    #[test]
    fn the_single_bar_spans_both_phases() {
        for renders_video in [false, true] {
            let share = analysis_share(renders_video);
            assert!(overall(Phase::Analyzing, 0.0, share).abs() < f64::EPSILON);
            let boundary = overall(Phase::Analyzing, 1.0, share);
            assert!((boundary - overall(Phase::Rendering, 0.0, share)).abs() < f64::EPSILON);
            assert!((overall(Phase::Rendering, 1.0, share) - 1.0).abs() < f64::EPSILON);
        }
        assert!(analysis_share(true) < analysis_share(false));
    }

    /// O vídeo sai em MP4 com as duas trilhas, e o vídeo encolhe junto com o áudio.
    #[test]
    fn a_video_file_keeps_video_and_audio() -> TestResult {
        let dir = TestDir::new("pipeline-video")?;
        let input = synthesize(dir.path(), "aula.mkv", (1.0, 6.0), 7.0, true)?;
        let outcome = run(&input, OutputProfile::Mp4, &CancelToken::default(), &mut |_, _| {})?;

        let Outcome::Rendered(rendered) = outcome else {
            return Err(format!("esperava o arquivo gravado, veio {outcome:?}").into());
        };
        assert_eq!(rendered.output, dir.path().join("aula (TIMEBOATED).mp4"));
        let written = probe::probe(&rendered.output)?;
        assert!(written.has_video && written.has_audio, "{written:?}");
        assert!((written.duration_secs - 2.5).abs() < 0.1, "{written:?}");
        Ok(())
    }

    /// Um MP3 VBR sem cabeçalho Xing tem a duração **estimada** pela taxa do começo: com
    /// ruído alto no começo, o `ffprobe` diz perto de metade da duração real. O plano tem de
    /// valer para o áudio inteiro — ruído de 0 a 10 s, silêncio até 30 s, tom até 40 s —, e
    /// não sumir com o tom do fim por ele passar da duração estimada.
    #[test]
    fn an_underestimated_duration_does_not_drop_the_end() -> TestResult {
        let dir = TestDir::new("pipeline-vbr")?;
        let input = dir.path().join("vbr.mp3");
        let mut args: Vec<OsString> = [
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "aevalsrc='if(lt(t,10),random(0)-0.5,if(lt(t,30),0,0.3*sin(2*PI*440*t)))':d=40:s=48000",
            "-c:a",
            "libmp3lame",
            "-q:a",
            "2",
            "-write_xing",
            "0",
        ]
        .map(OsString::from)
        .into();
        args.push(input.clone().into_os_string());
        let captured = tool::run_captured(Tool::Ffmpeg, &args)?;
        if !captured.success {
            return Err(format!("o ffmpeg não sintetizou o MP3: {}", captured.stderr).into());
        }
        let probed = probe::probe(&input)?;
        assert!(probed.duration_secs < 30.0, "o ffprobe deveria subestimar: {probed:?}");

        let outcome = run(&input, OutputProfile::Mp3, &CancelToken::default(), &mut |_, _| {})?;
        let Outcome::Rendered(rendered) = outcome else {
            return Err(format!("esperava o arquivo gravado, veio {outcome:?}").into());
        };
        assert!((rendered.original_secs - 40.0).abs() < 0.15, "{rendered:?}");
        // Mantidos: 0 a 10,25 s e 29,75 a 40 s.
        assert!((rendered.final_secs - 20.5).abs() < 0.15, "{rendered:?}");
        let written = probe::probe(&rendered.output)?;
        assert!((written.duration_secs - 20.5).abs() < 0.15, "{written:?}");
        Ok(())
    }

    /// Um silêncio curto demais não gera arquivo nenhum.
    #[test]
    fn a_short_silence_leaves_the_folder_untouched() -> TestResult {
        let dir = TestDir::new("pipeline-nothing")?;
        let input = synthesize(dir.path(), "curto.wav", (1.0, 3.0), 4.0, false)?;
        let outcome = run(&input, OutputProfile::Wav, &CancelToken::default(), &mut |_, _| {})?;
        assert_eq!(outcome, Outcome::NothingToRemove);
        assert_eq!(file_names(dir.path())?, ["curto.wav"]);
        Ok(())
    }

    /// Interrompido, o processamento não deixa nada na pasta.
    #[test]
    fn a_cancelled_run_leaves_the_folder_untouched() -> TestResult {
        let dir = TestDir::new("pipeline-cancel")?;
        let input = synthesize(dir.path(), "longo.wav", (1.0, 6.0), 7.0, false)?;
        let token = CancelToken::default();
        token.cancel();
        let outcome = run(&input, OutputProfile::Wav, &token, &mut |_, _| {})?;
        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(file_names(dir.path())?, ["longo.wav"]);
        Ok(())
    }

    #[test]
    fn fractions_are_clamped() {
        assert!((fraction(5.0, 10.0) - 0.5).abs() < f64::EPSILON);
        assert!((fraction(12.0, 10.0) - 1.0).abs() < f64::EPSILON);
        assert!(fraction(-1.0, 10.0).abs() < f64::EPSILON);
        assert!(fraction(1.0, 0.0).abs() < f64::EPSILON);
    }
}
