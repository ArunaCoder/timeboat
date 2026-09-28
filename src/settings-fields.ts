// A regra de cada campo do formulário, sem DOM: a ordem dos campos e a conferência do texto
// digitado contra a faixa que o Rust informa.

import { formatDecimal, parseDecimal } from "./numbers.ts";
import type { Bounds, SettingKey } from "./types.ts";

/// Os campos, na ordem em que aparecem. Cada id de `index.html` é o nome da régua; o teste
/// de `settings-fields.test.ts` confere que a lista cobre todas, uma vez cada.
export const FIELD_ORDER: readonly SettingKey[] = [
  "threshold_db",
  "min_silence_secs",
  "min_sound_secs",
  "padding_secs",
];

/// O veredito sobre o texto de um campo.
export type FieldCheck =
  | { readonly ok: true; readonly value: number }
  | { readonly ok: false; readonly reason: string };

/// `value` com as três casas que o campo exibe (`formatDecimal`), sem o zero negativo.
function toDisplayPrecision(value: number): number {
  const rounded = Math.round(value * 1000) / 1000;
  return rounded === 0 ? 0 : rounded;
}

/// Confere `text` contra `bounds`: o número, ou a frase que diz o que corrigir.
///
/// O número sai arredondado às três casas que o campo exibe: gravar mais precisão do que a
/// tela mostra faria o valor em vigor diferir do que a pessoa lê.
export function checkField(text: string, bounds: Bounds): FieldCheck {
  const parsed = parseDecimal(text);
  if (parsed === null) {
    return { ok: false, reason: "Digite um número, como 0,75." };
  }
  const value = toDisplayPrecision(parsed);
  if (value < bounds.min || value > bounds.max) {
    return {
      ok: false,
      reason: `Use um valor entre ${formatDecimal(bounds.min)} e ${formatDecimal(bounds.max)}.`,
    };
  }
  return { ok: true, value };
}
