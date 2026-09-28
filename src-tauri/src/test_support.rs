//! Andaime compartilhado pelos testes: uma pasta temporária própria de cada teste.

use std::path::{Path, PathBuf};
use std::{env, fs, io, process};

/// Pasta temporária exclusiva de um teste, apagada quando ele termina.
///
/// O nome leva o id do processo e o do teste, então testes paralelos e corridas simultâneas
/// da suíte nunca dividem pasta. Uma sobra de corrida interrompida é apagada na criação.
#[derive(Debug)]
pub struct TestDir(PathBuf);

impl TestDir {
    /// Cria a pasta vazia de `name`.
    ///
    /// # Errors
    /// A falha do sistema de arquivos ao limpar a sobra ou criar a pasta.
    pub fn new(name: &str) -> io::Result<Self> {
        let path = env::temp_dir().join(format!("timeboat-test-{}-{name}", process::id()));
        match fs::remove_dir_all(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    /// O caminho da pasta.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("não foi possível apagar {}: {error}", self.0.display());
        }
    }
}
