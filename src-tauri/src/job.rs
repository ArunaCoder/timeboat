//! A ponte entre o [`crate::pipeline`] e a janela: qual arquivo processar, um processamento
//! por vez, numa thread à parte, contando o andamento por eventos.
//!
//! # O caminho do arquivo nunca atravessa a fronteira IPC
//!
//! O arquivo chega por dois gestos, e os dois são vistos **só pelo Rust**: o soltar na
//! janela, pelo evento nativo de drag-and-drop (`lib.rs`), e o seletor de arquivos, aberto
//! pelo comando `pick_media`. O webview não passa caminho nenhum a comando nenhum — ele só
//! ouve os eventos [`EVENT`] e pede cancelamento. Um script que tomasse o webview não teria
//! como mandar o app ler ou gravar um arquivo que a pessoa não escolheu.
//!
//! # Um por vez
//!
//! O arquivo que chega com outro em andamento é recusado com [`AppError::Busy`], num evento
//! [`JobEvent::Rejected`] que não mexe no que a tela mostra do andamento. Fechar o app no
//! meio interrompe o `ffmpeg` e espera a limpeza ([`Jobs::shutdown`]): sem isso o processo
//! filho seguiria vivo, gravando um `.parcial` que ninguém apagaria.
//!
//! # O relógio é o do Rust
//!
//! O tempo de processamento que o resumo final mostra é medido aqui, do aceite do arquivo ao
//! desfecho, e o [`JobSnapshot`] carrega o tempo decorrido de um processamento em andamento:
//! a janela que recarrega no meio retoma o cronômetro do ponto certo, e não do zero.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter as _, Manager as _};

use crate::AppState;
use crate::error::{AppError, IpcError};
use crate::media::OutputProfile;
use crate::pipeline::{self, Outcome, Phase, Rendered, Request};
use crate::settings::Settings;
use crate::tool::CancelToken;

/// O nome do evento que conta o andamento à janela.
pub const EVENT: &str = "job";

/// O resumo do resultado gravado, como a tela o exibe.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// O nome do arquivo gravado (sem a pasta, que é a do original).
    pub output_name: String,
    /// A duração do original, em segundos.
    pub original_secs: f64,
    /// A duração do resultado, em segundos.
    pub final_secs: f64,
    /// Quantos trechos de silêncio foram removidos.
    pub cut_count: usize,
    /// Se o resultado é um vídeo (ou só áudio) — para a tela dizer "o vídeo perdeu".
    pub video: bool,
    /// Quanto o processamento levou, em segundos, do aceite do arquivo ao fim da gravação.
    pub elapsed_secs: f64,
}

impl Report {
    fn new(rendered: &Rendered, elapsed: Duration) -> Self {
        Self {
            output_name: display_name(&rendered.output),
            original_secs: rendered.original_secs,
            final_secs: rendered.final_secs,
            cut_count: rendered.cut_count,
            video: rendered.video,
            elapsed_secs: elapsed.as_secs_f64(),
        }
    }
}

/// O processamento em andamento, para a janela que (re)abre no meio dele.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JobSnapshot {
    /// O nome do arquivo (sem a pasta).
    pub file_name: String,
    /// Há quanto tempo o processamento começou, em segundos.
    pub elapsed_secs: f64,
}

/// O que a janela fica sabendo, em ordem: `started`, vários `progress`, e um desfecho.
///
/// Espelhado à mão em `src/types.ts`; o teste `events_keep_their_wire_shape` prende a forma.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobEvent {
    /// Um arquivo foi aceito e o processamento começou.
    Started {
        /// O nome do arquivo (sem a pasta).
        file_name: String,
    },
    /// Andamento do processamento.
    Progress {
        /// A etapa em andamento.
        phase: Phase,
        /// A fração cumprida do processamento **inteiro**, de 0 a 1 — a barra é uma só.
        fraction: f64,
    },
    /// O resultado foi gravado.
    Finished {
        /// O resumo do resultado.
        report: Report,
    },
    /// Nenhum silêncio passou das réguas; nada foi gravado.
    NothingToRemove,
    /// O arquivo inteiro seria cortado; nada foi gravado.
    AllSilent,
    /// Interrompido a pedido; nada ficou na pasta.
    Cancelled,
    /// O processamento falhou; nada ficou na pasta.
    Failed {
        /// A causa.
        error: IpcError,
    },
    /// O arquivo não foi aceito; o que estava em andamento, se havia, segue.
    Rejected {
        /// A causa.
        error: IpcError,
    },
}

/// O processamento em andamento e o último resultado gravado. Clonar compartilha o estado.
#[derive(Debug, Clone, Default)]
pub struct Jobs(Arc<JobsInner>);

#[derive(Debug, Default)]
struct JobsInner {
    running: Mutex<Option<Running>>,
    last_output: Mutex<Option<PathBuf>>,
}

#[derive(Debug)]
struct Running {
    file_name: String,
    started: Instant,
    token: Arc<CancelToken>,
}

impl Jobs {
    /// O processamento em andamento, se há um.
    #[must_use]
    pub fn current(&self) -> Option<JobSnapshot> {
        lock(&self.0.running).as_ref().map(|running| JobSnapshot {
            file_name: running.file_name.clone(),
            elapsed_secs: running.started.elapsed().as_secs_f64(),
        })
    }

    /// Pede a interrupção do processamento em andamento, se há um.
    pub fn cancel(&self) {
        if let Some(running) = lock(&self.0.running).as_ref() {
            running.token.cancel();
        }
    }

    /// O último resultado gravado nesta sessão.
    #[must_use]
    pub fn last_output(&self) -> Option<PathBuf> {
        lock(&self.0.last_output).clone()
    }

    /// Interrompe o processamento em andamento e espera, até `grace`, a limpeza terminar.
    pub fn shutdown(&self, grace: Duration) {
        self.cancel();
        let deadline = Instant::now() + grace;
        while self.is_running() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn is_running(&self) -> bool {
        lock(&self.0.running).is_some()
    }

    /// Ocupa o posto de processamento com `file_name`, começado em `started`.
    fn claim(&self, file_name: String, started: Instant) -> Result<Arc<CancelToken>, AppError> {
        let mut running = lock(&self.0.running);
        if running.is_some() {
            return Err(AppError::Busy);
        }
        let token = Arc::new(CancelToken::default());
        *running = Some(Running {
            file_name,
            started,
            token: Arc::clone(&token),
        });
        drop(running);
        Ok(token)
    }

    fn release(&self) {
        *lock(&self.0.running) = None;
    }

    fn record_output(&self, output: PathBuf) {
        *lock(&self.0.last_output) = Some(output);
    }
}

/// A trava, mesmo envenenada: o estado guardado é um `Option` que toda escrita deixa
/// coerente, então o pânico de outra thread não o corrompe.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// O nome do arquivo sem a pasta, para exibir.
fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// O arquivo a processar entre os itens soltos ou escolhidos, com o perfil de saída.
fn select(paths: &[PathBuf]) -> Result<(PathBuf, OutputProfile), AppError> {
    let path = match paths {
        [path] => path,
        [] => return Err(AppError::UnsupportedFile("nothing was dropped".to_owned())),
        many => return Err(AppError::MultipleFiles(many.len())),
    };
    if !path.is_file() {
        return Err(AppError::UnsupportedFile(format!("{} is not a file", path.display())));
    }
    let profile = OutputProfile::for_path(path)
        .ok_or_else(|| AppError::UnsupportedFile(format!("{} has an unsupported extension", path.display())))?;
    Ok((path.clone(), profile))
}

/// Um processamento aceito, pronto para a thread.
struct Job {
    path: PathBuf,
    profile: OutputProfile,
    token: Arc<CancelToken>,
    settings: Settings,
    started: Instant,
}

/// Começa a processar o arquivo em `paths`, ou conta à janela por que não.
pub fn start(app: &AppHandle, paths: &[PathBuf]) {
    let Some(state) = app.try_state::<AppState>() else {
        log::error!("ignoring {} item(s): the app state is not ready", paths.len());
        return;
    };
    let accepted = select(paths).and_then(|(path, profile)| {
        let started = Instant::now();
        let token = state.jobs.claim(display_name(&path), started)?;
        Ok(Job {
            path,
            profile,
            token,
            settings: state.settings.current(),
            started,
        })
    });
    match accepted {
        Ok(job) => launch(app, &state.jobs, job),
        Err(error) => {
            log::info!("rejected a file: {error}");
            emit(app, &JobEvent::Rejected { error: error.into() });
        }
    }
}

fn launch(app: &AppHandle, jobs: &Jobs, job: Job) {
    emit(
        app,
        &JobEvent::Started {
            file_name: display_name(&job.path),
        },
    );
    let app = app.clone();
    let jobs = jobs.clone();
    // No pool de threads bloqueantes do runtime do Tauri, e não num `thread::spawn`: o pool
    // não devolve falha de criação de thread para tratar, e o processamento é bloqueante do
    // começo ao fim (processos filhos e disco).
    drop(tauri::async_runtime::spawn_blocking(move || {
        let event = run(&app, &jobs, &job);
        // O posto é liberado antes do desfecho sair: quem soltar outro arquivo ao ver o
        // desfecho já encontra o posto livre.
        jobs.release();
        emit(&app, &event);
    }));
}

/// Roda o pipeline e traduz o desfecho no evento final.
fn run(app: &AppHandle, jobs: &Jobs, job: &Job) -> JobEvent {
    let request = Request {
        input: &job.path,
        profile: job.profile,
        settings: &job.settings,
        token: &job.token,
    };
    let mut on_progress = |phase, fraction| emit(app, &JobEvent::Progress { phase, fraction });
    match pipeline::process(&request, &mut on_progress) {
        Ok(Outcome::Rendered(rendered)) => {
            let report = Report::new(&rendered, job.started.elapsed());
            log::info!("wrote {} in {:.1} s", rendered.output.display(), report.elapsed_secs);
            jobs.record_output(rendered.output);
            JobEvent::Finished { report }
        }
        Ok(Outcome::NothingToRemove) => JobEvent::NothingToRemove,
        Ok(Outcome::AllSilent) => JobEvent::AllSilent,
        Ok(Outcome::Cancelled) => JobEvent::Cancelled,
        Err(error) => {
            log::error!("processing {} failed: {error}", job.path.display());
            JobEvent::Failed { error: error.into() }
        }
    }
}

fn emit(app: &AppHandle, event: &JobEvent) {
    if let Err(error) = app.emit(EVENT, event) {
        log::error!("could not emit the {EVENT} event: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::{JobEvent, JobSnapshot, Jobs, Report, select};
    use crate::error::{AppError, IpcError};
    use crate::media::OutputProfile;
    use crate::pipeline::{Phase, Rendered};
    use crate::test_support::TestDir;

    type TestResult = Result<(), Box<dyn Error>>;

    #[test]
    fn only_a_single_supported_file_is_selected() -> TestResult {
        let dir = TestDir::new("job-select")?;
        let audio = dir.path().join("voz.mp3");
        let text = dir.path().join("notas.txt");
        fs::write(&audio, "")?;
        fs::write(&text, "")?;

        assert_eq!(
            select(std::slice::from_ref(&audio))?,
            (audio.clone(), OutputProfile::Mp3)
        );
        assert!(matches!(
            select(&[audio, text.clone()]),
            Err(AppError::MultipleFiles(2))
        ));
        assert!(matches!(select(&[text]), Err(AppError::UnsupportedFile(_))));
        assert!(matches!(
            select(&[dir.path().to_owned()]),
            Err(AppError::UnsupportedFile(_))
        ));
        assert!(matches!(
            select(&[dir.path().join("sumiu.mp3")]),
            Err(AppError::UnsupportedFile(_))
        ));
        assert!(matches!(select(&[]), Err(AppError::UnsupportedFile(_))));
        Ok(())
    }

    /// Um posto só: o segundo pedido é recusado até o primeiro liberar, e o cancelamento
    /// chega ao token de quem está no posto.
    #[test]
    fn one_job_at_a_time() -> TestResult {
        let jobs = Jobs::default();
        let token = jobs.claim("a.mp4".to_owned(), Instant::now())?;
        assert_eq!(jobs.current().map(|job| job.file_name).as_deref(), Some("a.mp4"));
        assert!(matches!(
            jobs.claim("b.mp4".to_owned(), Instant::now()),
            Err(AppError::Busy)
        ));

        jobs.cancel();
        assert!(token.is_cancelled());

        jobs.release();
        assert_eq!(jobs.current(), None);
        assert!(jobs.claim("b.mp4".to_owned(), Instant::now()).is_ok());
        Ok(())
    }

    /// O instantâneo conta o tempo desde o começo do processamento, e não desde a pergunta.
    #[test]
    fn the_snapshot_carries_the_elapsed_time() -> TestResult {
        let jobs = Jobs::default();
        let started = Instant::now()
            .checked_sub(Duration::from_secs(5))
            .ok_or("o relógio do sistema não recua 5 s")?;
        jobs.claim("a.mp4".to_owned(), started)?;
        let JobSnapshot {
            file_name,
            elapsed_secs,
        } = jobs.current().ok_or("deveria haver processamento")?;
        assert_eq!(file_name, "a.mp4");
        assert!((5.0..6.0).contains(&elapsed_secs), "{elapsed_secs}");
        Ok(())
    }

    #[test]
    fn the_last_output_is_remembered() {
        let jobs = Jobs::default();
        assert_eq!(jobs.last_output(), None);
        jobs.record_output(PathBuf::from(r"C:\x\a (sem silêncio).mp4"));
        assert_eq!(jobs.last_output(), Some(PathBuf::from(r"C:\x\a (sem silêncio).mp4")));
    }

    /// O resumo leva o nome do arquivo sem a pasta e o tempo de processamento medido.
    #[test]
    fn the_report_names_the_output_and_the_elapsed_time() {
        let rendered = Rendered {
            output: PathBuf::from(r"C:\aulas\aula (sem silêncio).mp4"),
            original_secs: 725.0,
            final_secs: 600.0,
            cut_count: 3,
            video: true,
        };
        let report = Report::new(&rendered, Duration::from_millis(42_500));
        assert_eq!(report.output_name, "aula (sem silêncio).mp4");
        assert!((report.elapsed_secs - 42.5).abs() < f64::EPSILON);
        assert!(report.video);
    }

    /// A forma dos eventos no fio é a que `src/types.ts` espelha: a etiqueta em `kind`, e os
    /// campos em `snake_case`.
    #[test]
    fn events_keep_their_wire_shape() -> TestResult {
        let cases = [
            (
                JobEvent::Started {
                    file_name: "a.mp4".to_owned(),
                },
                json!({ "kind": "started", "file_name": "a.mp4" }),
            ),
            (
                JobEvent::Progress {
                    phase: Phase::Rendering,
                    fraction: 0.5,
                },
                json!({ "kind": "progress", "phase": "rendering", "fraction": 0.5 }),
            ),
            (
                JobEvent::Finished {
                    report: Report {
                        output_name: "a (sem silêncio).mp4".to_owned(),
                        original_secs: 10.0,
                        final_secs: 4.5,
                        cut_count: 2,
                        video: true,
                        elapsed_secs: 3.25,
                    },
                },
                json!({
                    "kind": "finished",
                    "report": {
                        "output_name": "a (sem silêncio).mp4",
                        "original_secs": 10.0,
                        "final_secs": 4.5,
                        "cut_count": 2,
                        "video": true,
                        "elapsed_secs": 3.25
                    }
                }),
            ),
            (JobEvent::NothingToRemove, json!({ "kind": "nothing_to_remove" })),
            (JobEvent::AllSilent, json!({ "kind": "all_silent" })),
            (JobEvent::Cancelled, json!({ "kind": "cancelled" })),
            (
                JobEvent::Rejected {
                    error: IpcError::from(AppError::Busy),
                },
                json!({
                    "kind": "rejected",
                    "error": { "code": "busy", "message": "a file is already being processed" }
                }),
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(serde_json::to_value(&event)?, expected, "{event:?}");
        }
        let snapshot = JobSnapshot {
            file_name: "a.mp4".to_owned(),
            elapsed_secs: 1.5,
        };
        assert_eq!(
            serde_json::to_value(&snapshot)?,
            json!({ "file_name": "a.mp4", "elapsed_secs": 1.5 })
        );
        Ok(())
    }
}
