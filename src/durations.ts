// Durações em relógio: minutos:segundos ("2:05"), e horas:minutos:segundos a partir de uma
// hora ("1:02:05"). É a forma em que se lê a duração de um vídeo em qualquer player.

/// Dois dígitos, para os minutos e segundos que vêm depois de uma unidade maior.
function twoDigits(value: number): string {
  return String(value).padStart(2, "0");
}

/// `secs` arredondado ao segundo inteiro — a precisão do relógio. Quem subtrai durações
/// para exibir arredonda cada parcela antes, para a conta fechar na tela.
export function wholeSeconds(secs: number): number {
  return Math.round(secs);
}

/// `secs` no relógio. Valor inválido vira um travessão.
export function formatClock(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) {
    return "—";
  }
  const total = wholeSeconds(secs);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  if (hours > 0) {
    return `${hours}:${twoDigits(minutes)}:${twoDigits(seconds)}`;
  }
  return `${minutes}:${twoDigits(seconds)}`;
}
