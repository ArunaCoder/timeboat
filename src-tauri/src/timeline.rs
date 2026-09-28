//! O plano de cortes: dos silêncios brutos que a análise relatou aos trechos que ficam.
//!
//! Função pura sobre números, sem `ffmpeg` nem disco — é aqui que as quatro réguas de
//! [`Settings`] ganham significado, e é por isso que o módulo concentra os testes de regra.
//!
//! # As etapas, em ordem
//!
//! 1. **Normalizar**: prender cada silêncio ao arquivo, ordenar e fundir os sobrepostos.
//! 2. **Ignorar o som curto**: o som mais curto que a detecção mínima, entre dois silêncios
//!    (ou entre um silêncio e a borda do arquivo), vira silêncio — os dois lados se fundem.
//! 3. **Escolher**: só o silêncio mais longo que o silêncio mínimo é cortado.
//! 4. **Acolchoar**: cada corte encolhe a margem de cada lado que encosta em som. O lado que
//!    encosta na borda do arquivo não tem fala a proteger, e é cortado inteiro.
//! 5. **Complementar**: o que fica são os intervalos entre os cortes.

use serde::Serialize;

use crate::settings::Settings;

/// Um trecho do arquivo, em segundos.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Interval {
    /// Início, em segundos.
    pub start: f64,
    /// Fim, em segundos.
    pub end: f64,
}

impl Interval {
    /// O trecho de `start` a `end`.
    #[must_use]
    pub const fn new(start: f64, end: f64) -> Self {
        Self { start, end }
    }

    /// A duração, em segundos (negativa se o trecho estiver invertido).
    #[must_use]
    pub const fn length(self) -> f64 {
        self.end - self.start
    }
}

/// Tolerância para dizer que um silêncio encosta na borda do arquivo: o `silencedetect`
/// relata o início em frações de amostra, e o fim pode passar da duração do contêiner.
const EDGE_TOLERANCE_SECS: f64 = 0.001;

/// O menor corte que vale a pena: abaixo disto o corte é ruído de arredondamento.
const MIN_CUT_SECS: f64 = 0.01;

/// O menor trecho mantido. Um trecho mais curto que um quadro e meio sairia como vídeo sem
/// quadro nenhum, e o `concat` do `ffmpeg` não lida bem com segmento vazio.
const MIN_KEEP_SECS: f64 = 0.05;

/// O veredito sobre um arquivo.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Nenhum silêncio passa das réguas: o arquivo ficaria igual.
    NothingToRemove,
    /// O arquivo inteiro seria cortado: não sobra o que gravar.
    AllSilent,
    /// Há o que cortar e o que manter.
    Cut(CutPlan),
}

/// Os trechos mantidos e os cortados, ambos **não vazios** por construção — só
/// [`plan`] cria um plano.
#[derive(Debug, Clone, PartialEq)]
pub struct CutPlan {
    keep: Vec<Interval>,
    cuts: Vec<Interval>,
}

impl CutPlan {
    /// Os trechos mantidos, em ordem.
    #[must_use]
    pub fn keep(&self) -> &[Interval] {
        &self.keep
    }

    /// Quantos cortes o plano faz.
    #[must_use]
    pub const fn cut_count(&self) -> usize {
        self.cuts.len()
    }

    /// A duração do resultado, em segundos.
    #[must_use]
    pub fn kept_secs(&self) -> f64 {
        self.keep.iter().map(|segment| segment.length()).sum()
    }
}

/// O veredito sobre um arquivo de `duration_secs` com os silêncios brutos `raw`.
#[must_use]
pub fn plan(raw: &[Interval], duration_secs: f64, settings: &Settings) -> Verdict {
    let silences = normalize(raw, duration_secs);
    let silences = absorb_short_sounds(silences, duration_secs, settings.min_sound_secs);
    let cuts: Vec<Interval> = silences
        .into_iter()
        .filter(|silence| silence.length() > settings.min_silence_secs)
        .filter_map(|silence| pad(silence, duration_secs, settings.padding_secs))
        .collect();
    if cuts.is_empty() {
        return Verdict::NothingToRemove;
    }
    let keep = complement(&cuts, duration_secs);
    if keep.is_empty() {
        return Verdict::AllSilent;
    }
    Verdict::Cut(CutPlan { keep, cuts })
}

/// Prende ao arquivo, descarta o vazio, ordena e funde o que se sobrepõe ou encosta.
fn normalize(raw: &[Interval], duration_secs: f64) -> Vec<Interval> {
    let mut clamped: Vec<Interval> = raw
        .iter()
        .map(|silence| Interval::new(silence.start.max(0.0), silence.end.min(duration_secs)))
        .filter(|silence| silence.length() > 0.0)
        .collect();
    clamped.sort_by(|left, right| left.start.total_cmp(&right.start));
    let mut merged: Vec<Interval> = Vec::with_capacity(clamped.len());
    for silence in clamped {
        match merged.last_mut() {
            Some(last) if silence.start <= last.end => last.end = last.end.max(silence.end),
            _ => merged.push(silence),
        }
    }
    merged
}

/// Funde os silêncios separados por som mais curto que `min_sound_secs`, e estende até a
/// borda o silêncio separado dela por som assim.
fn absorb_short_sounds(silences: Vec<Interval>, duration_secs: f64, min_sound_secs: f64) -> Vec<Interval> {
    let mut merged: Vec<Interval> = Vec::with_capacity(silences.len());
    for silence in silences {
        match merged.last_mut() {
            Some(last) if silence.start - last.end < min_sound_secs => last.end = silence.end,
            _ => merged.push(silence),
        }
    }
    if let Some(first) = merged.first_mut()
        && first.start < min_sound_secs
    {
        first.start = 0.0;
    }
    if let Some(last) = merged.last_mut()
        && duration_secs - last.end < min_sound_secs
    {
        last.end = duration_secs;
    }
    merged
}

/// O corte de um silêncio, com a margem só nos lados que encostam em som; `None` se a
/// margem consumir o silêncio.
fn pad(silence: Interval, duration_secs: f64, padding_secs: f64) -> Option<Interval> {
    let start = if silence.start > EDGE_TOLERANCE_SECS {
        silence.start + padding_secs
    } else {
        0.0
    };
    let end = if silence.end < duration_secs - EDGE_TOLERANCE_SECS {
        silence.end - padding_secs
    } else {
        duration_secs
    };
    let cut = Interval::new(start, end);
    (cut.length() > MIN_CUT_SECS).then_some(cut)
}

/// Os trechos de `[0, duration_secs]` fora dos `cuts` (ordenados e disjuntos).
fn complement(cuts: &[Interval], duration_secs: f64) -> Vec<Interval> {
    let mut keep = Vec::with_capacity(cuts.len() + 1);
    let mut cursor = 0.0;
    for cut in cuts {
        keep.push(Interval::new(cursor, cut.start));
        cursor = cut.end;
    }
    keep.push(Interval::new(cursor, duration_secs));
    keep.retain(|segment| segment.length() >= MIN_KEEP_SECS);
    keep
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{CutPlan, Interval, Verdict, plan};
    use crate::settings::{DEFAULTS, Settings};

    type TestResult = Result<(), Box<dyn Error>>;

    /// Os padrões: -40 dB, silêncio maior que 3 s, som menor que 0,75 s ignorado, 0,25 s de
    /// margem.
    const S: Settings = DEFAULTS;

    fn iv(start: f64, end: f64) -> Interval {
        Interval::new(start, end)
    }

    fn cut(raw: &[Interval], duration: f64, settings: &Settings) -> Result<CutPlan, Box<dyn Error>> {
        match plan(raw, duration, settings) {
            Verdict::Cut(plan) => Ok(plan),
            other => Err(format!("esperava um corte, veio {other:?}").into()),
        }
    }

    /// Compara trechos com tolerância, porque as margens somam e subtraem frações.
    fn assert_segments(actual: &[Interval], expected: &[(f64, f64)]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?} × {expected:?}");
        for (segment, &(start, end)) in actual.iter().zip(expected) {
            assert!(
                (segment.start - start).abs() < 1e-9 && (segment.end - end).abs() < 1e-9,
                "{actual:?} × {expected:?}"
            );
        }
    }

    #[test]
    fn without_silence_there_is_nothing_to_remove() {
        assert_eq!(plan(&[], 60.0, &S), Verdict::NothingToRemove);
    }

    /// "Maior que 3 s" é estrito: o silêncio de 3 s exatos fica.
    #[test]
    fn a_silence_not_longer_than_the_minimum_stays() {
        assert_eq!(plan(&[iv(10.0, 13.0)], 60.0, &S), Verdict::NothingToRemove);
        assert_eq!(plan(&[iv(10.0, 12.0)], 60.0, &S), Verdict::NothingToRemove);
    }

    /// O caso típico: a margem fica dos dois lados do corte.
    #[test]
    fn a_long_silence_in_the_middle_is_cut_with_padding() -> TestResult {
        let plan = cut(&[iv(10.0, 20.0)], 60.0, &S)?;
        assert_segments(plan.keep(), &[(0.0, 10.25), (19.75, 60.0)]);
        assert_eq!(plan.cut_count(), 1);
        assert!((plan.kept_secs() - 50.5).abs() < 1e-9);
        Ok(())
    }

    /// O silêncio do começo e o do fim não têm fala do lado da borda: saem inteiros.
    #[test]
    fn silences_at_the_edges_are_cut_whole_on_the_edge_side() -> TestResult {
        let plan = cut(&[iv(0.0, 5.0), iv(50.0, 60.0)], 60.0, &S)?;
        assert_segments(plan.keep(), &[(4.75, 50.25)]);
        assert_eq!(plan.cut_count(), 2);
        Ok(())
    }

    #[test]
    fn a_file_that_is_all_silence_has_nothing_to_keep() {
        assert_eq!(plan(&[iv(0.0, 60.0)], 60.0, &S), Verdict::AllSilent);
        // O silêncio que só não encosta nas bordas por um clique curto também é tudo.
        assert_eq!(plan(&[iv(0.3, 59.8)], 60.0, &S), Verdict::AllSilent);
    }

    /// Um clique de 0,2 s no meio de dois silêncios de 2 s: nenhum dos dois passa de 3 s
    /// sozinho, mas o clique é ignorado e os 4,2 s viram um corte só.
    #[test]
    fn a_short_sound_between_silences_is_ignored() -> TestResult {
        let plan = cut(&[iv(10.0, 12.0), iv(12.2, 14.2)], 60.0, &S)?;
        assert_segments(plan.keep(), &[(0.0, 10.25), (13.95, 60.0)]);
        Ok(())
    }

    /// O som de 0,75 s ou mais é detecção de verdade: separa os silêncios, e cada lado curto
    /// fica.
    #[test]
    fn a_sound_as_long_as_the_minimum_is_kept() {
        assert_eq!(
            plan(&[iv(10.0, 12.0), iv(12.75, 14.75)], 60.0, &S),
            Verdict::NothingToRemove
        );
    }

    /// Com a detecção mínima zerada, nenhum som é ignorado.
    #[test]
    fn a_zero_minimum_sound_ignores_nothing() {
        let settings = Settings {
            min_sound_secs: 0.0,
            ..S
        };
        assert_eq!(
            plan(&[iv(10.0, 12.0), iv(12.01, 14.0)], 60.0, &settings),
            Verdict::NothingToRemove
        );
    }

    /// A margem que consome o silêncio inteiro anula o corte.
    #[test]
    fn padding_that_swallows_the_silence_cancels_the_cut() {
        let settings = Settings { padding_secs: 2.0, ..S };
        assert_eq!(plan(&[iv(10.0, 13.5)], 60.0, &settings), Verdict::NothingToRemove);
    }

    /// O relato bruto pode vir fora de ordem, sobreposto, com início negativo e fim além da
    /// duração — tudo isso é normalizado antes das regras.
    #[test]
    fn raw_reports_are_normalized() -> TestResult {
        let raw = [iv(40.0, 45.0), iv(-0.02, 1.0), iv(42.0, 70.0), iv(20.0, 20.0)];
        let plan = cut(&raw, 60.0, &S)?;
        assert_segments(plan.keep(), &[(0.0, 40.25)]);
        assert_eq!(plan.cut_count(), 1);
        Ok(())
    }

    /// Um trecho mantido curto demais para virar quadro de vídeo é descartado.
    #[test]
    fn slivers_are_not_kept() -> TestResult {
        let settings = Settings {
            min_sound_secs: 0.0,
            padding_secs: 0.0,
            ..S
        };
        let plan = cut(&[iv(10.0, 20.0), iv(20.02, 30.0)], 60.0, &settings)?;
        assert_segments(plan.keep(), &[(0.0, 10.0), (30.0, 60.0)]);
        Ok(())
    }
}
