//! Os formatos aceitos e o arquivo de saída: que extensão entra, com que perfil de codificação
//! sai, e com que nome — sem nunca sobrescrever um arquivo que já exista na pasta.
//!
//! # Vídeo sai sempre em MP4
//!
//! Todo vídeo, qualquer que seja o contêiner de entrada, sai em MP4 com H.264 e AAC: é o par
//! que qualquer player e editor abre, e manter o contêiner original exigiria um perfil por
//! contêiner (o `WebM` só aceita VP8/VP9/AV1, o AVI mal aceita H.264). O áudio mantém o formato de
//! entrada, porque ali a escolha do formato costuma ser deliberada (WAV para edição, MP3 para
//! publicar); a exceção é o WMA, que sai em M4A.
//!
//! # O nome do resultado é reservado antes do `ffmpeg` começar
//!
//! [`Reservation`] cria, com `create_new`, o arquivo final vazio e um irmão `.parcial`, onde o
//! `ffmpeg` escreve. Só no fim a renomeação troca o vazio pelo pronto. Assim o nome escolhido
//! não pode ser tomado por outro arquivo no meio do caminho, nenhum arquivo alheio é
//! sobrescrito, e uma queda deixa um `.parcial` que se reconhece como tal — nunca um arquivo
//! truncado com o nome do resultado. Desistir (erro ou cancelamento) apaga os dois.

use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::error::AppError;

/// Com que perfil de codificação o resultado é gravado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputProfile {
    /// Vídeo em MP4: H.264 + AAC.
    Mp4,
    /// Áudio MP3 (LAME, VBR de alta qualidade).
    Mp3,
    /// Áudio WAV (PCM 16 bits).
    Wav,
    /// Áudio M4A (AAC).
    M4a,
    /// Áudio FLAC (sem perdas).
    Flac,
    /// Áudio Ogg Vorbis.
    Ogg,
    /// Áudio Opus.
    Opus,
}

/// A tabela de formatos aceitos: extensão de entrada (minúscula) e o perfil de saída.
///
/// Fonte única: a validação do arquivo solto e o filtro do seletor de arquivos leem daqui.
pub const FORMATS: &[(&str, OutputProfile)] = &[
    ("mp4", OutputProfile::Mp4),
    ("m4v", OutputProfile::Mp4),
    ("mov", OutputProfile::Mp4),
    ("mkv", OutputProfile::Mp4),
    ("webm", OutputProfile::Mp4),
    ("avi", OutputProfile::Mp4),
    ("wmv", OutputProfile::Mp4),
    ("flv", OutputProfile::Mp4),
    ("mpg", OutputProfile::Mp4),
    ("mpeg", OutputProfile::Mp4),
    ("ts", OutputProfile::Mp4),
    ("mts", OutputProfile::Mp4),
    ("m2ts", OutputProfile::Mp4),
    ("3gp", OutputProfile::Mp4),
    ("mp3", OutputProfile::Mp3),
    ("wav", OutputProfile::Wav),
    ("m4a", OutputProfile::M4a),
    ("aac", OutputProfile::M4a),
    ("wma", OutputProfile::M4a),
    ("flac", OutputProfile::Flac),
    ("ogg", OutputProfile::Ogg),
    ("oga", OutputProfile::Ogg),
    ("opus", OutputProfile::Opus),
];

impl OutputProfile {
    /// O perfil do arquivo em `path`, pela extensão (sem distinguir maiúsculas), ou `None` se
    /// o formato não é aceito.
    #[must_use]
    pub fn for_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        FORMATS
            .iter()
            .find(|(accepted, _)| *accepted == extension)
            .map(|&(_, profile)| profile)
    }

    /// A extensão do arquivo de saída.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::M4a => "m4a",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
            Self::Opus => "opus",
        }
    }

    /// Se o resultado carrega vídeo (quando a entrada tem uma trilha de vídeo de verdade).
    #[must_use]
    pub const fn keeps_video(self) -> bool {
        matches!(self, Self::Mp4)
    }

    /// Os argumentos de codificação do `ffmpeg` para este perfil.
    #[must_use]
    pub const fn codec_args(self) -> &'static [&'static str] {
        match self {
            // CRF 18 é visualmente sem perdas para o H.264; `yuv420p` porque gravações de tela
            // em 4:4:4 ou 10 bits não abrem em boa parte dos players; `faststart` põe o índice
            // no começo, para o arquivo tocar antes de terminar de carregar.
            Self::Mp4 => &[
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-movflags",
                "+faststart",
            ],
            Self::Mp3 => &["-c:a", "libmp3lame", "-q:a", "2"],
            Self::Wav => &["-c:a", "pcm_s16le"],
            Self::M4a => &["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"],
            Self::Flac => &["-c:a", "flac"],
            Self::Ogg => &["-c:a", "libvorbis", "-q:a", "6"],
            Self::Opus => &["-c:a", "libopus", "-b:a", "128k"],
        }
    }
}

/// As extensões aceitas, na ordem da tabela — o filtro do seletor de arquivos.
#[must_use]
pub fn accepted_extensions() -> Vec<&'static str> {
    FORMATS.iter().map(|&(extension, _)| extension).collect()
}

/// O que se acrescenta ao nome do arquivo de entrada para nomear o resultado.
const OUTPUT_TAG: &str = "TIMEBOATED";

/// A marca do arquivo onde o `ffmpeg` escreve enquanto trabalha.
const PARTIAL_TAG: &str = "parcial";

/// Quantos nomes numerados se tentam antes de desistir da pasta.
const MAX_NAME_ATTEMPTS: u32 = 999;

/// Os nomes do resultado e do seu parcial para a tentativa `attempt` (a partir de 1):
/// `aula (TIMEBOATED).mp4` e `aula (TIMEBOATED).parcial.mp4`; da segunda em diante,
/// `aula (TIMEBOATED 2).mp4`.
fn output_names(stem: &OsStr, attempt: u32, extension: &str) -> (OsString, OsString) {
    let mut base = stem.to_os_string();
    if attempt <= 1 {
        base.push(format!(" ({OUTPUT_TAG})"));
    } else {
        base.push(format!(" ({OUTPUT_TAG} {attempt})"));
    }
    let mut final_name = base.clone();
    final_name.push(format!(".{extension}"));
    let mut partial_name = base;
    partial_name.push(format!(".{PARTIAL_TAG}.{extension}"));
    (final_name, partial_name)
}

/// O nome do resultado e o parcial, ambos já criados vazios na pasta da entrada.
///
/// Soltá-la sem [`Reservation::commit`] apaga os dois.
#[derive(Debug)]
pub struct Reservation {
    final_path: PathBuf,
    partial_path: PathBuf,
    armed: bool,
}

impl Reservation {
    /// Reserva, na pasta de `input`, o primeiro nome livre para o resultado.
    ///
    /// # Errors
    /// [`AppError::UnsupportedFile`] se `input` não tiver pasta ou nome;
    /// [`AppError::OutputFolder`] se a pasta recusar a criação ou não houver nome livre.
    pub fn reserve(input: &Path, profile: OutputProfile) -> Result<Self, AppError> {
        let folder = input.parent().ok_or_else(|| unsupported(input))?;
        let stem = input.file_stem().ok_or_else(|| unsupported(input))?;
        let refuse = |source: io::Error| AppError::OutputFolder {
            folder: folder.display().to_string(),
            source,
        };
        for attempt in 1..=MAX_NAME_ATTEMPTS {
            let (final_name, partial_name) = output_names(stem, attempt, profile.extension());
            let final_path = folder.join(final_name);
            if !create_new(&final_path).map_err(refuse)? {
                continue;
            }
            let partial_path = folder.join(partial_name);
            match create_new(&partial_path) {
                Ok(true) => {
                    return Ok(Self {
                        final_path,
                        partial_path,
                        armed: true,
                    });
                }
                Ok(false) => remove_quietly(&final_path),
                Err(source) => {
                    remove_quietly(&final_path);
                    return Err(refuse(source));
                }
            }
        }
        Err(refuse(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "every numbered output name is taken",
        )))
    }

    /// Onde o `ffmpeg` deve escrever.
    #[must_use]
    pub fn partial_path(&self) -> &Path {
        &self.partial_path
    }

    /// Troca o reservado vazio pelo parcial pronto, e devolve o caminho do resultado.
    ///
    /// # Errors
    /// [`AppError::OutputFolder`] se a renomeação falhar — e então os dois arquivos são
    /// apagados, como em qualquer desistência.
    pub fn commit(mut self) -> Result<PathBuf, AppError> {
        rename_patiently(&self.partial_path, &self.final_path).map_err(|source| AppError::OutputFolder {
            folder: self
                .final_path
                .parent()
                .map_or_else(String::new, |folder| folder.display().to_string()),
            source,
        })?;
        self.armed = false;
        Ok(self.final_path.clone())
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.armed {
            remove_quietly(&self.partial_path);
            remove_quietly(&self.final_path);
        }
    }
}

fn unsupported(input: &Path) -> AppError {
    AppError::UnsupportedFile(format!("{} has no folder or file name", input.display()))
}

/// Cria `path` vazio se ele não existir: `Ok(false)` quando já existe.
fn create_new(path: &Path) -> io::Result<bool> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

/// Quantas vezes a remoção de um arquivo é tentada, e o intervalo entre as tentativas.
const REMOVE_ATTEMPTS: u32 = 10;
const REMOVE_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Quantas vezes a renomeação final é tentada, no mesmo intervalo: mais que a remoção, porque
/// desistir dela descarta uma gravação inteira que deu certo.
const RENAME_ATTEMPTS: u32 = 50;

/// Renomeia `from` para `to`, repetindo pelo mesmo motivo de [`remove_quietly`]: o parcial
/// que o `ffmpeg` acabou de fechar pode seguir aberto por um instante pelo antivírus ou pelo
/// indexador, e a renomeação falha com acesso negado.
fn rename_patiently(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempt = 1;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound || attempt >= RENAME_ATTEMPTS => {
                return Err(error);
            }
            Err(_) => {
                attempt += 1;
                thread::sleep(REMOVE_RETRY_DELAY);
            }
        }
    }
}

/// Apaga `path` se existir.
///
/// No Windows, o parcial que o `ffmpeg` acabou de fechar pode seguir aberto por um instante
/// por outro processo (o antivírus que o examina, o indexador), e a remoção falha com acesso
/// negado. Ela é repetida por até um segundo antes de desistir: a interrupção promete que nada
/// fica na pasta. A falha final só vai para o log, porque quem chama já está desistindo.
fn remove_quietly(path: &Path) {
    for attempt in 1..=REMOVE_ATTEMPTS {
        match fs::remove_file(path) {
            Ok(()) => return,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return,
            Err(error) if attempt == REMOVE_ATTEMPTS => {
                log::warn!("could not remove {}: {error}", path.display());
            }
            Err(_) => thread::sleep(REMOVE_RETRY_DELAY),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    #[cfg(windows)]
    use std::thread;
    #[cfg(windows)]
    use std::time::Duration;

    use super::{FORMATS, OutputProfile, Reservation, accepted_extensions, output_names};
    use crate::test_support::TestDir;

    type TestResult = Result<(), Box<dyn Error>>;

    #[test]
    fn the_profile_comes_from_the_extension_in_any_case() {
        let cases = [
            ("aula.MP4", Some(OutputProfile::Mp4)),
            ("gravação.mkv", Some(OutputProfile::Mp4)),
            ("podcast.Mp3", Some(OutputProfile::Mp3)),
            ("voz.wma", Some(OutputProfile::M4a)),
            ("notas.txt", None),
            ("sem-extensao", None),
        ];
        for (name, expected) in cases {
            assert_eq!(OutputProfile::for_path(Path::new(name)), expected, "{name}");
        }
    }

    /// Só o MP4 carrega vídeo, e toda extensão da tabela é minúscula — a busca compara com a
    /// extensão já rebaixada, então uma maiúscula aqui nunca casaria.
    #[test]
    fn the_table_is_consistent() {
        for &(extension, profile) in FORMATS {
            assert_eq!(
                extension,
                extension.to_ascii_lowercase(),
                "{extension} deveria ser minúscula"
            );
            assert_eq!(profile.keeps_video(), profile == OutputProfile::Mp4);
        }
        assert_eq!(accepted_extensions().len(), FORMATS.len());
    }

    #[test]
    fn output_names_carry_the_tag_and_the_attempt() {
        let (final_name, partial_name) = output_names(OsStr::new("aula 1"), 1, "mp4");
        assert_eq!(final_name, "aula 1 (TIMEBOATED).mp4");
        assert_eq!(partial_name, "aula 1 (TIMEBOATED).parcial.mp4");
        let (second, _) = output_names(OsStr::new("aula 1"), 2, "wav");
        assert_eq!(second, "aula 1 (TIMEBOATED 2).wav");
    }

    /// Um resultado anterior na pasta não é sobrescrito: a reserva pula para o nome seguinte.
    #[test]
    fn an_existing_output_is_never_overwritten() -> TestResult {
        let dir = TestDir::new("media-reserve")?;
        let input = dir.path().join("aula.mov");
        let taken = dir.path().join("aula (TIMEBOATED).mp4");
        fs::write(&taken, "anterior")?;

        let reservation = Reservation::reserve(&input, OutputProfile::Mp4)?;
        assert_eq!(
            reservation.partial_path(),
            dir.path().join("aula (TIMEBOATED 2).parcial.mp4")
        );
        fs::write(reservation.partial_path(), "novo")?;
        let output = reservation.commit()?;

        assert_eq!(output, dir.path().join("aula (TIMEBOATED 2).mp4"));
        assert_eq!(fs::read_to_string(&output)?, "novo");
        assert_eq!(
            fs::read_to_string(&taken)?,
            "anterior",
            "o arquivo anterior tem de ficar intacto"
        );
        assert!(!dir.path().join("aula (TIMEBOATED 2).parcial.mp4").exists());
        Ok(())
    }

    /// O parcial travado por um instante — o antivírus examinando o arquivo recém-fechado —
    /// não faz a gravação inteira se perder: a renomeação espera a trava sair.
    #[cfg(windows)]
    #[test]
    fn a_briefly_locked_partial_is_still_committed() -> TestResult {
        use std::os::windows::fs::OpenOptionsExt as _;

        let dir = TestDir::new("media-locked")?;
        let reservation = Reservation::reserve(&dir.path().join("aula.wav"), OutputProfile::Wav)?;
        fs::write(reservation.partial_path(), "pronto")?;
        // Sem compartilhamento nenhum, como o antivírus abre: a renomeação falha enquanto dura.
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(reservation.partial_path())?;
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            drop(lock);
        });
        let output = reservation.commit()?;
        release.join().map_err(|_| "a thread da trava entrou em pânico")?;
        assert_eq!(fs::read_to_string(output)?, "pronto");
        Ok(())
    }

    /// Desistir apaga o reservado e o parcial: nada sobra na pasta da pessoa.
    #[test]
    fn dropping_a_reservation_cleans_up() -> TestResult {
        let dir = TestDir::new("media-drop")?;
        let input = dir.path().join("voz.wav");
        let reservation = Reservation::reserve(&input, OutputProfile::Wav)?;
        fs::write(reservation.partial_path(), "meio caminho")?;
        drop(reservation);
        assert_eq!(fs::read_dir(dir.path())?.count(), 0, "a pasta deveria ter ficado vazia");
        Ok(())
    }
}
