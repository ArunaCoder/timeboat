//! Erros do app: o [`AppError`] de uso interno e o [`IpcError`] serializável da fronteira IPC.
//!
//! Todo comando que pode falhar devolve `Result<T, IpcError>`, e o que cruza a fronteira é o
//! objeto `{ code, message }`. O `code` é enum fechado, serializado em `snake_case`, para que a
//! tela case por código em vez de inspecionar texto; a `message` é diagnóstico em inglês e
//! **não** é o que a pessoa lê — a frase em português mora no mapa de `src/messages.ts`.
//!
//! # O lado TS é gerado daqui
//!
//! O union `IpcErrorCode` de `src/ipc-error-codes.generated.ts` não é espelhado à mão: quem o
//! escreve é o teste `tests::generated_ts_matches_the_enum`, que renderiza um literal por
//! variante com o próprio serde e falha enquanto o arquivo commitado divergir. Código novo aqui
//! pede regenerar (`UPDATE_IPC_TS=1 cargo test`) e a frase no mapa de `messages.ts`, que o
//! `Record` do TypeScript cobra na compilação.

use std::io;

use serde::Serialize;
use thiserror::Error;

/// Código de erro estável exposto à tela. Um código por **desfecho**, porque a ação que a
/// frase sugere difere entre eles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
// Só em build de teste: o `iter()` sai das variantes que o compilador vê, então variante
// nova entra sozinha no arquivo gerado. Fora do teste a crate nem é linkada.
#[cfg_attr(test, derive(strum::EnumIter))]
#[serde(rename_all = "snake_case")]
pub enum IpcErrorCode {
    /// O `ffmpeg` ou o `ffprobe` não está no `PATH`.
    FfmpegMissing,
    /// O `ffmpeg` não chegou ao fim: codec sem decoder, disco cheio, arquivo truncado.
    FfmpegFailed,
    /// O item solto ou escolhido não é um arquivo de vídeo ou áudio suportado.
    UnsupportedFile,
    /// Mais de um item solto de uma vez.
    MultipleFiles,
    /// Já há um arquivo em processamento.
    Busy,
    /// O arquivo não tem trilha de áudio: não há silêncio a detectar.
    NoAudio,
    /// O `ffprobe` não conseguiu ler o arquivo (duração, trilhas).
    UnreadableMedia,
    /// A pasta do arquivo não aceitou a gravação do resultado.
    OutputFolder,
    /// O script de filtros do `ffmpeg` não pôde ser criado na pasta temporária.
    TempFile,
    /// Configuração fora da faixa aceita.
    InvalidSettings,
    /// O arquivo de configurações não pôde ser gravado.
    SettingsStore,
    /// Pedido para mostrar o resultado antes de haver um.
    NoOutput,
    /// O Explorer não abriu a pasta do resultado.
    Reveal,
    /// Falha que nenhum outro código prevê: um defeito do app, e não do arquivo.
    Internal,
}

/// Erro de uso interno, com a causa concreta. Vira [`IpcError`] na fronteira.
#[derive(Debug, Error)]
pub enum AppError {
    /// O programa externo não foi encontrado para ser iniciado.
    #[error("{program} was not found on PATH")]
    ToolMissing {
        /// Nome do executável procurado.
        program: &'static str,
    },
    /// O programa externo existe, mas o sistema recusou iniciá-lo.
    #[error("could not start {program}: {source}")]
    ToolSpawn {
        /// Nome do executável.
        program: &'static str,
        /// A recusa do sistema.
        #[source]
        source: io::Error,
    },
    /// O programa externo terminou com falha.
    #[error("{program} failed ({status}): {detail}")]
    ToolFailed {
        /// Nome do executável.
        program: &'static str,
        /// O código de saída, como o sistema o descreve.
        status: String,
        /// O fim da saída de erro do programa.
        detail: String,
    },
    /// O item não é arquivo, ou a extensão não está na tabela de formatos.
    #[error("unsupported file: {0}")]
    UnsupportedFile(String),
    /// Mais de um item solto de uma vez; carrega quantos.
    #[error("{0} items were dropped at once")]
    MultipleFiles(usize),
    /// Um processamento já está em andamento.
    #[error("a file is already being processed")]
    Busy,
    /// O arquivo não tem trilha de áudio.
    #[error("the media has no audio stream")]
    NoAudio,
    /// O `ffprobe` falhou ou devolveu um relatório sem duração.
    #[error("could not read the media: {0}")]
    UnreadableMedia(String),
    /// Falha de disco na pasta de saída (reserva do nome, renomeação final).
    #[error("could not write the output in {folder}: {source}")]
    OutputFolder {
        /// A pasta de saída.
        folder: String,
        /// A falha do sistema de arquivos.
        #[source]
        source: io::Error,
    },
    /// Falha ao criar o script de filtros na pasta temporária.
    #[error("could not create the filter script: {0}")]
    TempFile(#[source] io::Error),
    /// Configuração fora da faixa aceita.
    #[error("invalid settings: {0}")]
    InvalidSettings(String),
    /// Falha ao gravar o arquivo de configurações.
    #[error("could not store the settings: {0}")]
    SettingsStore(String),
    /// Nenhum arquivo foi gerado nesta sessão.
    #[error("there is no output to reveal yet")]
    NoOutput,
    /// O Explorer não abriu a pasta do resultado.
    #[error("could not reveal the output: {0}")]
    Reveal(String),
    /// Um defeito do app interrompeu o processamento (um pânico na thread do trabalho).
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    /// O código estável que a tela recebe para este erro.
    ///
    /// `match` exaustivo de propósito: variante nova não compila até ganhar código aqui.
    #[must_use]
    pub const fn code(&self) -> IpcErrorCode {
        match self {
            Self::ToolMissing { .. } => IpcErrorCode::FfmpegMissing,
            Self::ToolSpawn { .. } | Self::ToolFailed { .. } => IpcErrorCode::FfmpegFailed,
            Self::UnsupportedFile(_) => IpcErrorCode::UnsupportedFile,
            Self::MultipleFiles(_) => IpcErrorCode::MultipleFiles,
            Self::Busy => IpcErrorCode::Busy,
            Self::NoAudio => IpcErrorCode::NoAudio,
            Self::UnreadableMedia(_) => IpcErrorCode::UnreadableMedia,
            Self::OutputFolder { .. } => IpcErrorCode::OutputFolder,
            Self::TempFile(_) => IpcErrorCode::TempFile,
            Self::InvalidSettings(_) => IpcErrorCode::InvalidSettings,
            Self::SettingsStore(_) => IpcErrorCode::SettingsStore,
            Self::NoOutput => IpcErrorCode::NoOutput,
            Self::Reveal(_) => IpcErrorCode::Reveal,
            Self::Internal(_) => IpcErrorCode::Internal,
        }
    }
}

/// Erro serializável devolvido à tela pelos comandos e pelos eventos de processamento.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Error)]
#[error("{code:?}: {message}")]
pub struct IpcError {
    /// Código estável para a tela casar sem inspecionar texto.
    pub code: IpcErrorCode,
    /// Diagnóstico legível, sempre em inglês — a tela o mostra só como detalhe técnico.
    pub message: String,
}

impl From<AppError> for IpcError {
    fn from(err: AppError) -> Self {
        Self {
            code: err.code(),
            message: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fmt::Write as _;
    use std::path::Path;
    use std::{env, fs, io};

    use strum::IntoEnumIterator as _;

    use super::{AppError, IpcError, IpcErrorCode};

    type TestResult = Result<(), Box<dyn Error>>;

    /// O arquivo que este enum produz, relativo à raiz do crate.
    const GENERATED_TS: &str = "../src/ipc-error-codes.generated.ts";

    /// Variável que troca **conferir** por **reescrever** o arquivo gerado. Sem ela o teste é
    /// somente-leitura, e rodar a suíte nunca suja o working tree.
    const UPDATE_ENV: &str = "UPDATE_IPC_TS";

    /// Renderiza o union do webview a partir das variantes do enum.
    ///
    /// O literal de cada código sai do **serde de verdade** (`to_value`), não de uma
    /// reimplementação da regra do `rename_all`: a conversão fica correta por construção.
    fn render_ts() -> Result<String, Box<dyn Error>> {
        let mut out = format!(
            "// Arquivo GERADO — não editar à mão.
//
// Fonte: o enum `IpcErrorCode` de `src-tauri/src/error.rs`. Quem o produz e confere é o
// teste `error::tests::generated_ts_matches_the_enum`, que falha enquanto o arquivo
// commitado divergir do que o Rust produz hoje. Para reescrevê-lo depois de mexer na
// fonte, rode a suíte com `{UPDATE_ENV}=1`.

/// Código de erro estável da fronteira IPC — um literal por variante do enum do Rust, na
/// forma que o serde serializa.
export type IpcErrorCode =
"
        );
        let mut codes = IpcErrorCode::iter().peekable();
        while let Some(code) = codes.next() {
            let value = serde_json::to_value(code)?;
            let literal = value.as_str().ok_or("o código IPC deve serializar como string")?;
            let end = if codes.peek().is_some() { "" } else { ";" };
            writeln!(out, "  | \"{literal}\"{end}")?;
        }
        Ok(out)
    }

    /// Onde o renderizado e o commitado divergem, ou `None` quando são o mesmo.
    ///
    /// Função pura: é o que permite provar o gate vermelho sem tocar disco.
    fn divergence(rendered: &str, committed: &str) -> Option<String> {
        if rendered == committed {
            return None;
        }
        let mut expected = rendered.lines();
        let mut found = committed.lines();
        let mut line = 1_usize;
        loop {
            match (expected.next(), found.next()) {
                (None, None) => return Some("o conteúdo difere só nas quebras de linha do fim".to_owned()),
                (left, right) if left == right => line += 1,
                (left, right) => return Some(format!("linha {line}: esperado {left:?}, encontrado {right:?}")),
            }
        }
    }

    /// O arquivo commitado é o que o Rust produz hoje.
    ///
    /// Código novo no enum sem regenerar deixa a suíte vermelha, em vez de produzir um union
    /// menor que o enum — cujo sintoma seria a tela exibir a frase genérica para um erro que
    /// tem frase própria.
    #[test]
    fn generated_ts_matches_the_enum() -> TestResult {
        let rendered = render_ts()?;
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(GENERATED_TS);
        if env::var_os(UPDATE_ENV).is_some() {
            fs::write(&path, &rendered)?;
            return Ok(());
        }
        let committed = fs::read_to_string(&path)?;
        if let Some(report) = divergence(&rendered, &committed) {
            return Err(format!(
                "{} está desatualizado ({report}). Rode a suíte com {UPDATE_ENV}=1 para reescrevê-lo.",
                path.display()
            )
            .into());
        }
        Ok(())
    }

    /// O gate visto **vermelho** — um gate que ninguém viu reprovar é prosa. O caso simétrico
    /// entra junto porque um comparador que reprovasse tudo passaria na primeira metade.
    #[test]
    fn the_comparator_rejects_a_union_missing_a_code() -> TestResult {
        let rendered = render_ts()?;
        let tampered = rendered.replace("  | \"no_audio\"\n", "");
        assert_ne!(tampered, rendered, "o código adulterado tem de existir no render");
        assert!(
            divergence(&rendered, &tampered).is_some(),
            "um union sem `no_audio` tem de reprovar"
        );
        assert!(
            divergence(&rendered, &rendered).is_none(),
            "o conteúdo intacto tem de passar"
        );
        Ok(())
    }

    /// Cada erro interno chega à tela com o código que escolhe a frase certa.
    #[test]
    fn each_app_error_carries_its_code() {
        let cases: [(AppError, IpcErrorCode); 15] = [
            (AppError::ToolMissing { program: "ffmpeg" }, IpcErrorCode::FfmpegMissing),
            (
                AppError::ToolSpawn {
                    program: "ffmpeg",
                    source: io::Error::from(io::ErrorKind::PermissionDenied),
                },
                IpcErrorCode::FfmpegFailed,
            ),
            (
                AppError::ToolFailed {
                    program: "ffmpeg",
                    status: "exit code: 1".to_owned(),
                    detail: "Invalid data".to_owned(),
                },
                IpcErrorCode::FfmpegFailed,
            ),
            (
                AppError::UnsupportedFile("x.txt".to_owned()),
                IpcErrorCode::UnsupportedFile,
            ),
            (AppError::MultipleFiles(2), IpcErrorCode::MultipleFiles),
            (AppError::Busy, IpcErrorCode::Busy),
            (AppError::NoAudio, IpcErrorCode::NoAudio),
            (
                AppError::UnreadableMedia("no duration".to_owned()),
                IpcErrorCode::UnreadableMedia,
            ),
            (
                AppError::OutputFolder {
                    folder: "C:\\x".to_owned(),
                    source: io::Error::from(io::ErrorKind::PermissionDenied),
                },
                IpcErrorCode::OutputFolder,
            ),
            (
                AppError::TempFile(io::Error::from(io::ErrorKind::StorageFull)),
                IpcErrorCode::TempFile,
            ),
            (AppError::InvalidSettings("x".to_owned()), IpcErrorCode::InvalidSettings),
            (AppError::SettingsStore("x".to_owned()), IpcErrorCode::SettingsStore),
            (AppError::NoOutput, IpcErrorCode::NoOutput),
            (AppError::Reveal("x".to_owned()), IpcErrorCode::Reveal),
            (AppError::Internal("x".to_owned()), IpcErrorCode::Internal),
        ];
        for (error, expected) in cases {
            let ipc = IpcError::from(error);
            assert_eq!(ipc.code, expected, "{ipc:?}");
        }
    }

    #[test]
    fn serializes_code_in_snake_case_with_the_english_diagnostic() -> TestResult {
        let err = IpcError::from(AppError::ToolMissing { program: "ffprobe" });
        let json = serde_json::to_value(&err)?;
        assert_eq!(json["code"], "ffmpeg_missing");
        assert_eq!(json["message"], "ffprobe was not found on PATH");
        Ok(())
    }
}
