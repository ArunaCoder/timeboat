//! A execução do `ffmpeg` e do `ffprobe`: iniciar sem janela de console, acompanhar o
//! progresso e interromper a pedido.
//!
//! # Os dois programas vêm do `PATH`
//!
//! O app não embarca o `ffmpeg` (são ~100 MB, e o `winget install Gyan.FFmpeg` já o põe no
//! `PATH` da pessoa, mantido e atualizado por fora). Não achá-lo é um desfecho de primeira
//! classe, com código próprio ([`AppError::ToolMissing`]) e checagem na abertura da janela
//! ([`check_tools`]), para a pessoa descobrir antes de soltar o primeiro arquivo.
//!
//! # Progresso e cancelamento sem compartilhar o processo
//!
//! O `ffmpeg` roda com `-progress pipe:1`, que escreve blocos `chave=valor` no stdout. Uma
//! thread lê esse stdout e manda cada posição por um canal; quem chamou recebe do canal com
//! tempo limite e, entre um recebimento e outro, confere o [`CancelToken`]. Assim o processo
//! filho tem um dono só — a função que o iniciou — e o cancelamento não precisa de trava em
//! volta dele: basta um booleano atômico.

use std::ffi::OsString;
use std::io::{self, BufRead as _, BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::error::AppError;

/// Os programas externos que o app usa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// O conversor, que analisa o áudio e grava o resultado.
    Ffmpeg,
    /// O inspetor, que lê duração e trilhas.
    Ffprobe,
}

impl Tool {
    /// O nome do executável, procurado no `PATH`.
    #[must_use]
    pub const fn program(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
        }
    }
}

/// `CREATE_NO_WINDOW`: sem ele, cada `ffmpeg` abriria uma janela de console por cima do app,
/// que em release não tem console próprio (`windows_subsystem = "windows"`).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// De quanto em quanto tempo o laço de progresso confere o pedido de cancelamento.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// Quantas linhas do fim do stderr entram no diagnóstico de uma falha.
const STDERR_TAIL_LINES: usize = 12;

/// O caminho como argumento de entrada ou saída do `ffmpeg`, com o protocolo `file:`
/// explícito: sem ele, um nome de arquivo com `:` ou começado por `-` seria lido como
/// protocolo ou como opção.
#[must_use]
pub fn file_arg(path: &Path) -> OsString {
    let mut arg = OsString::from("file:");
    arg.push(path);
    arg
}

fn command(tool: Tool) -> Command {
    let mut command = Command::new(tool.program());
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn spawn_error(tool: Tool, source: io::Error) -> AppError {
    if source.kind() == io::ErrorKind::NotFound {
        AppError::ToolMissing {
            program: tool.program(),
        }
    } else {
        AppError::ToolSpawn {
            program: tool.program(),
            source,
        }
    }
}

/// A saída de um programa que roda até o fim sem acompanhamento.
#[derive(Debug)]
pub struct Captured {
    /// Se o programa saiu com sucesso.
    pub success: bool,
    /// O código de saída, como o sistema o descreve.
    pub status: String,
    /// O stdout, decodificado com substituição do que não for UTF-8.
    pub stdout: String,
    /// O stderr, decodificado do mesmo jeito.
    pub stderr: String,
}

/// Roda `tool` com `args` até o fim e captura a saída; o veredito fica com quem chama.
///
/// # Errors
/// [`AppError::ToolMissing`] se o programa não estiver no `PATH`; [`AppError::ToolSpawn`]
/// se o sistema recusar iniciá-lo.
pub fn run_captured(tool: Tool, args: &[OsString]) -> Result<Captured, AppError> {
    let output = command(tool)
        .args(args)
        .output()
        .map_err(|error| spawn_error(tool, error))?;
    Ok(Captured {
        success: output.status.success(),
        status: output.status.to_string(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Confere que o `ffmpeg` e o `ffprobe` estão no `PATH` e respondem.
///
/// # Errors
/// [`AppError::ToolMissing`] para o primeiro que faltar; [`AppError::ToolSpawn`] ou
/// [`AppError::ToolFailed`] se ele existir mas não responder ao `-version`.
pub fn check_tools() -> Result<(), AppError> {
    for tool in [Tool::Ffmpeg, Tool::Ffprobe] {
        let captured = run_captured(tool, &[OsString::from("-version")])?;
        if !captured.success {
            return Err(failure(tool, &captured.status, &captured.stderr));
        }
    }
    Ok(())
}

/// O erro de um programa que terminou com falha, com o fim do stderr como diagnóstico.
#[must_use]
pub fn failure(tool: Tool, status: &str, stderr: &str) -> AppError {
    AppError::ToolFailed {
        program: tool.program(),
        status: status.to_owned(),
        detail: tail(stderr, STDERR_TAIL_LINES),
    }
}

/// As últimas `count` linhas não vazias de `text`, unidas por ` | `.
fn tail(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    lines[lines.len().saturating_sub(count)..].join(" | ")
}

/// O pedido de interrupção de um processamento, visto pela thread do trabalho e pelo comando
/// que a pessoa aciona.
#[derive(Debug, Default)]
pub struct CancelToken(AtomicBool);

impl CancelToken {
    /// Pede a interrupção; o laço de progresso encerra o `ffmpeg` em até um décimo de segundo.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Se a interrupção foi pedida.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Como terminou um `ffmpeg` acompanhado.
#[derive(Debug)]
pub enum RunOutcome {
    /// Terminou com sucesso; carrega o stderr, onde vivem os relatórios dos filtros.
    Completed {
        /// O stderr completo do processo.
        stderr: String,
    },
    /// Foi interrompido a pedido.
    Cancelled,
}

/// Roda o `ffmpeg` com `args` — que devem incluir `-progress pipe:1` —, chamando
/// `on_progress` com a posição, em segundos, de cada bloco de progresso.
///
/// Um pedido em `token` encerra o processo e devolve [`RunOutcome::Cancelled`].
///
/// # Errors
/// [`AppError::ToolMissing`] ou [`AppError::ToolSpawn`] se o `ffmpeg` não iniciar;
/// [`AppError::ToolFailed`] se ele sair com falha sem ter sido interrompido.
pub fn run_with_progress(
    args: &[OsString],
    token: &CancelToken,
    on_progress: &mut dyn FnMut(f64),
) -> Result<RunOutcome, AppError> {
    let tool = Tool::Ffmpeg;
    let mut child = command(tool)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| spawn_error(tool, error))?;
    let stderr = child.stderr.take().map(spawn_text_reader);
    if let Some(stdout) = child.stdout.take() {
        pump_progress(&spawn_progress_reader(stdout), &mut child, token, on_progress);
    }
    let status = child.wait().map_err(|source| AppError::ToolSpawn {
        program: tool.program(),
        source,
    })?;
    let stderr = stderr.map(join_text).unwrap_or_default();
    conclude(tool, status, stderr, token)
}

fn conclude(tool: Tool, status: ExitStatus, stderr: String, token: &CancelToken) -> Result<RunOutcome, AppError> {
    if token.is_cancelled() {
        Ok(RunOutcome::Cancelled)
    } else if status.success() {
        Ok(RunOutcome::Completed { stderr })
    } else {
        Err(failure(tool, &status.to_string(), &stderr))
    }
}

/// Repassa o progresso até o stdout fechar, encerrando o processo se a interrupção for pedida.
fn pump_progress(progress: &Receiver<f64>, child: &mut Child, token: &CancelToken, on_progress: &mut dyn FnMut(f64)) {
    let mut killed = false;
    loop {
        match progress.recv_timeout(CANCEL_POLL) {
            Ok(position) => on_progress(position),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if !killed && token.is_cancelled() {
            if let Err(error) = child.kill() {
                log::warn!("could not stop ffmpeg: {error}");
            }
            killed = true;
        }
    }
}

/// Lê o stdout de progresso numa thread e manda cada posição pelo canal devolvido; o canal
/// fecha quando o stdout fecha.
fn spawn_progress_reader(stdout: ChildStdout) -> Receiver<f64> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            if let Some(position) = parse_progress_line(&line)
                && sender.send(position).is_err()
            {
                return;
            }
        }
    });
    receiver
}

/// Lê um fluxo inteiro numa thread — o stderr, que precisa ser drenado enquanto o stdout é
/// acompanhado, senão o `ffmpeg` trava com o buffer do pipe cheio.
fn spawn_text_reader(mut stream: impl Read + Send + 'static) -> JoinHandle<String> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Err(error) = stream.read_to_end(&mut bytes) {
            log::warn!("could not read the ffmpeg output: {error}");
        }
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

fn join_text(handle: JoinHandle<String>) -> String {
    handle.join().unwrap_or_default()
}

/// A posição, em segundos, de uma linha `out_time_us=` do `-progress`; `None` para as demais
/// linhas e para o `N/A` que o `ffmpeg` escreve antes do primeiro quadro.
#[must_use]
pub fn parse_progress_line(line: &str) -> Option<f64> {
    let micros: f64 = line.strip_prefix("out_time_us=")?.trim().parse().ok()?;
    (micros.is_finite() && micros >= 0.0).then_some(micros / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::Path;

    use super::{CancelToken, Tool, failure, file_arg, parse_progress_line, tail};
    use crate::error::AppError;

    #[test]
    fn progress_lines_become_seconds() {
        assert_eq!(parse_progress_line("out_time_us=1500000"), Some(1.5));
        assert_eq!(parse_progress_line("out_time_us=0"), Some(0.0));
        assert_eq!(parse_progress_line("out_time_us=N/A"), None);
        assert_eq!(parse_progress_line("out_time_us=-5"), None);
        assert_eq!(parse_progress_line("out_time_ms=1500000"), None);
        assert_eq!(parse_progress_line("progress=continue"), None);
    }

    #[test]
    fn paths_go_to_ffmpeg_with_the_file_protocol() {
        assert_eq!(
            file_arg(Path::new(r"C:\vídeos\-aula:1.mp4")),
            r"file:C:\vídeos\-aula:1.mp4"
        );
    }

    /// O diagnóstico de falha é o fim do stderr, sem as linhas vazias.
    #[test]
    fn a_failure_keeps_the_tail_of_stderr() -> Result<(), Box<dyn Error>> {
        let stderr = (1..=20).map(|n| format!("line {n}\n")).collect::<Vec<_>>().join("\n");
        let AppError::ToolFailed { program, detail, .. } = failure(Tool::Ffmpeg, "exit code: 1", &stderr) else {
            return Err("a falha deveria ser ToolFailed".into());
        };
        assert_eq!(program, "ffmpeg");
        assert!(detail.starts_with("line 9 | line 10"), "{detail}");
        assert!(detail.ends_with("line 20"), "{detail}");
        assert_eq!(tail("", 3), "");
        Ok(())
    }

    #[test]
    fn the_cancel_token_starts_clear_and_stays_set() {
        let token = CancelToken::default();
        assert!(!token.is_cancelled());
        token.cancel();
        token.cancel();
        assert!(token.is_cancelled());
    }
}
