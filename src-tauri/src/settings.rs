//! As quatro réguas da detecção de silêncio que a pessoa edita na janela, e o arquivo onde
//! elas persistem entre uma abertura e outra.
//!
//! # O que cada régua significa
//!
//! - **Nível de som** (`threshold_db`): abaixo dele, o áudio conta como silêncio.
//! - **Silêncio mínimo** (`min_silence_secs`): só o silêncio **mais longo** que isto é cortado.
//! - **Detecção mínima** (`min_sound_secs`): o som mais curto que isto — um clique, uma tosse
//!   — é ignorado, e o silêncio dos dois lados dele conta como um só.
//! - **Margem** (`padding_secs`): o tanto de silêncio preservado de cada lado do corte, para a
//!   fala não começar nem terminar seca.
//!
//! Os limites de cada régua ([`LIMITS`]) têm fonte única aqui: a tela os recebe pelo comando
//! `get_settings` e valida com eles, em vez de manter uma cópia que envelheceria.
//!
//! # A persistência nunca bloqueia o app
//!
//! Arquivo ausente, ilegível ou com valor fora da faixa cai nos padrões, com um aviso no log:
//! as configurações são conveniência, e perder a edição anterior é melhor que o app não abrir.
//! A gravação é atômica (arquivo temporário e renomeação), para que uma queda no meio não
//! deixe um JSON truncado.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Nome do arquivo de configurações, dentro do diretório de configuração do app.
pub const FILE_NAME: &str = "settings.json";

/// As réguas da detecção. Os campos são os que atravessam a fronteira IPC, em `snake_case`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
// Campo ausente no arquivo assume o padrão: um arquivo de uma versão anterior, com menos
// réguas, continua valendo no que tem.
#[serde(default)]
pub struct Settings {
    /// Nível, em dBFS, abaixo do qual o áudio conta como silêncio.
    pub threshold_db: f64,
    /// Só o silêncio mais longo que isto, em segundos, é removido.
    pub min_silence_secs: f64,
    /// O som mais curto que isto, em segundos, é ignorado (conta como silêncio).
    pub min_sound_secs: f64,
    /// Silêncio preservado, em segundos, de cada lado de cada corte.
    pub padding_secs: f64,
}

/// Os padrões de fábrica — os que o botão "Restaurar padrões" devolve.
pub const DEFAULTS: Settings = Settings {
    threshold_db: -40.0,
    min_silence_secs: 3.0,
    min_sound_secs: 0.75,
    padding_secs: 0.25,
};

impl Default for Settings {
    fn default() -> Self {
        DEFAULTS
    }
}

/// Faixa fechada aceita por uma régua.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Bounds {
    /// Menor valor aceito.
    pub min: f64,
    /// Maior valor aceito.
    pub max: f64,
}

impl Bounds {
    /// Se `value` está na faixa. `NaN` nunca está.
    #[must_use]
    pub fn contains(self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }
}

/// A faixa aceita de cada régua, com os mesmos nomes de campo de [`Settings`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Limits {
    /// Faixa de [`Settings::threshold_db`].
    pub threshold_db: Bounds,
    /// Faixa de [`Settings::min_silence_secs`].
    pub min_silence_secs: Bounds,
    /// Faixa de [`Settings::min_sound_secs`].
    pub min_sound_secs: Bounds,
    /// Faixa de [`Settings::padding_secs`].
    pub padding_secs: Bounds,
}

/// As faixas aceitas. O piso do silêncio mínimo não é zero porque o `silencedetect` do
/// `ffmpeg` com duração zero marcaria cada amostra silenciosa como um silêncio próprio.
pub const LIMITS: Limits = Limits {
    threshold_db: Bounds { min: -100.0, max: -1.0 },
    min_silence_secs: Bounds { min: 0.1, max: 3600.0 },
    min_sound_secs: Bounds { min: 0.0, max: 60.0 },
    padding_secs: Bounds { min: 0.0, max: 10.0 },
};

impl Settings {
    /// Devolve as réguas se todas estiverem na faixa de [`LIMITS`].
    ///
    /// # Errors
    /// [`AppError::InvalidSettings`] nomeando a primeira régua fora da faixa (ou `NaN`).
    pub fn validate(self) -> Result<Self, AppError> {
        check("threshold_db", self.threshold_db, LIMITS.threshold_db)?;
        check("min_silence_secs", self.min_silence_secs, LIMITS.min_silence_secs)?;
        check("min_sound_secs", self.min_sound_secs, LIMITS.min_sound_secs)?;
        check("padding_secs", self.padding_secs, LIMITS.padding_secs)?;
        Ok(self)
    }

    /// A duração mínima do silêncio que a análise do `ffmpeg` reporta.
    ///
    /// É a menor das duas réguas de duração, e não o silêncio mínimo, porque o som curto só é
    /// ignorado quando está **entre** silêncios: um clique no meio de 4 s de silêncio parte o
    /// trecho em dois de 2 s, e a análise precisa reportar os dois para que a junção os una.
    /// E não é zero, porque então a pausa entre duas sílabas também partiria a fala em trechos
    /// curtos, que seriam ignorados como se fossem cliques. Com a detecção mínima desligada
    /// (zero), nada se junta e basta o silêncio mínimo.
    #[must_use]
    pub const fn detection_window(&self) -> f64 {
        if self.min_sound_secs > 0.0 {
            self.min_sound_secs.min(self.min_silence_secs)
        } else {
            self.min_silence_secs
        }
    }
}

fn check(name: &str, value: f64, bounds: Bounds) -> Result<(), AppError> {
    if bounds.contains(value) {
        Ok(())
    } else {
        Err(AppError::InvalidSettings(format!(
            "{name} = {value} is outside {}..={}",
            bounds.min, bounds.max
        )))
    }
}

/// As configurações correntes, em memória, e o arquivo que as persiste.
#[derive(Debug)]
pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    /// Carrega do arquivo em `path`, caindo nos padrões se ele faltar ou não servir.
    #[must_use]
    pub fn load(path: PathBuf) -> Self {
        let current = match read(&path) {
            Ok(Some(settings)) => settings,
            Ok(None) => DEFAULTS,
            Err(reason) => {
                log::warn!("ignoring the settings in {}: {reason}", path.display());
                DEFAULTS
            }
        };
        Self {
            path,
            current: Mutex::new(current),
        }
    }

    /// As configurações correntes.
    #[must_use]
    pub fn current(&self) -> Settings {
        *self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Valida, grava no arquivo e só então adota `settings` como correntes.
    ///
    /// # Errors
    /// [`AppError::InvalidSettings`] se alguma régua estiver fora da faixa — nada é gravado;
    /// [`AppError::SettingsStore`] se o arquivo não puder ser escrito — as correntes não mudam.
    pub fn save(&self, settings: Settings) -> Result<Settings, AppError> {
        let valid = settings.validate()?;
        write(&self.path, &valid).map_err(|error| AppError::SettingsStore(error.to_string()))?;
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = valid;
        Ok(valid)
    }
}

/// Lê o arquivo: `Ok(None)` se ele não existe, `Err` com a causa se não serve.
fn read(path: &Path) -> Result<Option<Settings>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let settings: Settings = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    settings.validate().map(Some).map_err(|error| error.to_string())
}

/// Grava de forma atômica: o conteúdo vai para um arquivo irmão, que então substitui o
/// original numa renomeação só.
fn write(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let mut text = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
    text.push('\n');
    let staging = path.with_extension("json.tmp");
    fs::write(&staging, text)?;
    fs::rename(&staging, path)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;

    use super::{DEFAULTS, FILE_NAME, LIMITS, Settings, SettingsStore};
    use crate::error::AppError;
    use crate::test_support::TestDir;

    type TestResult = Result<(), Box<dyn Error>>;

    /// Os padrões são os que a especificação do app pede.
    #[test]
    fn defaults_are_the_specified_ones() {
        assert!((DEFAULTS.threshold_db - -40.0).abs() < f64::EPSILON);
        assert!((DEFAULTS.min_silence_secs - 3.0).abs() < f64::EPSILON);
        assert!((DEFAULTS.min_sound_secs - 0.75).abs() < f64::EPSILON);
        assert!((DEFAULTS.padding_secs - 0.25).abs() < f64::EPSILON);
        assert!(
            DEFAULTS.validate().is_ok(),
            "os padrões têm de caber nas próprias faixas"
        );
    }

    #[test]
    fn each_setting_outside_its_bounds_is_rejected() {
        let cases = [
            Settings {
                threshold_db: 0.0,
                ..DEFAULTS
            },
            Settings {
                threshold_db: -101.0,
                ..DEFAULTS
            },
            Settings {
                min_silence_secs: 0.05,
                ..DEFAULTS
            },
            Settings {
                min_sound_secs: -0.1,
                ..DEFAULTS
            },
            Settings {
                padding_secs: 11.0,
                ..DEFAULTS
            },
            Settings {
                padding_secs: f64::NAN,
                ..DEFAULTS
            },
            Settings {
                min_silence_secs: f64::INFINITY,
                ..DEFAULTS
            },
        ];
        for settings in cases {
            assert!(
                matches!(settings.validate(), Err(AppError::InvalidSettings(_))),
                "{settings:?} deveria ser recusado"
            );
        }
    }

    #[test]
    fn the_bounds_themselves_are_accepted() {
        let low = Settings {
            threshold_db: LIMITS.threshold_db.min,
            min_silence_secs: LIMITS.min_silence_secs.min,
            min_sound_secs: LIMITS.min_sound_secs.min,
            padding_secs: LIMITS.padding_secs.min,
        };
        let high = Settings {
            threshold_db: LIMITS.threshold_db.max,
            min_silence_secs: LIMITS.min_silence_secs.max,
            min_sound_secs: LIMITS.min_sound_secs.max,
            padding_secs: LIMITS.padding_secs.max,
        };
        assert!(low.validate().is_ok());
        assert!(high.validate().is_ok());
    }

    /// A janela da análise é a menor das duas durações, e só o silêncio mínimo quando a
    /// detecção mínima está desligada.
    #[test]
    fn the_detection_window_is_the_smaller_duration() {
        assert!((DEFAULTS.detection_window() - 0.75).abs() < f64::EPSILON);
        let aggressive = Settings {
            min_silence_secs: 0.5,
            min_sound_secs: 1.0,
            ..DEFAULTS
        };
        assert!((aggressive.detection_window() - 0.5).abs() < f64::EPSILON);
        let no_absorption = Settings {
            min_sound_secs: 0.0,
            ..DEFAULTS
        };
        assert!((no_absorption.detection_window() - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_missing_file_loads_the_defaults() -> TestResult {
        let dir = TestDir::new("settings-missing")?;
        let store = SettingsStore::load(dir.path().join(FILE_NAME));
        assert_eq!(store.current(), DEFAULTS);
        Ok(())
    }

    #[test]
    fn a_broken_or_out_of_range_file_loads_the_defaults() -> TestResult {
        let dir = TestDir::new("settings-broken")?;
        for content in ["{ not json", r#"{ "threshold_db": 12 }"#] {
            let path = dir.path().join(FILE_NAME);
            fs::write(&path, content)?;
            let store = SettingsStore::load(path);
            assert_eq!(store.current(), DEFAULTS, "{content} deveria cair nos padrões");
        }
        Ok(())
    }

    /// O que se grava é o que a próxima abertura carrega, e campo ausente assume o padrão.
    #[test]
    fn saved_settings_survive_a_reload() -> TestResult {
        let dir = TestDir::new("settings-roundtrip")?;
        let path = dir.path().join("nested").join(FILE_NAME);
        let edited = Settings {
            threshold_db: -35.5,
            padding_secs: 0.5,
            ..DEFAULTS
        };
        SettingsStore::load(path.clone()).save(edited)?;
        assert_eq!(SettingsStore::load(path.clone()).current(), edited);

        fs::write(&path, r#"{ "threshold_db": -50 }"#)?;
        let partial = SettingsStore::load(path).current();
        assert_eq!(
            partial,
            Settings {
                threshold_db: -50.0,
                ..DEFAULTS
            }
        );
        Ok(())
    }

    /// Configuração inválida não chega ao disco nem à memória.
    #[test]
    fn an_invalid_save_changes_nothing() -> TestResult {
        let dir = TestDir::new("settings-invalid")?;
        let path = dir.path().join(FILE_NAME);
        let store = SettingsStore::load(path.clone());
        let result = store.save(Settings {
            min_silence_secs: -1.0,
            ..DEFAULTS
        });
        assert!(matches!(result, Err(AppError::InvalidSettings(_))));
        assert_eq!(store.current(), DEFAULTS);
        assert!(!path.exists(), "nada deveria ter sido gravado");
        Ok(())
    }
}
