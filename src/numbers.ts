// Números no formato brasileiro: vírgula decimal na tela, ponto aceito na digitação.
//
// Os campos são `type="text"` com `inputmode="decimal"`, e não `type="number"`: o separador
// que o campo numérico do WebView2 aceita depende do idioma do Windows, e a pessoa que
// digita "0,75" num sistema em inglês veria o valor recusado sem explicação.

/// Um sinal opcional (o menos comum ou o tipográfico), e dígitos com no máximo um separador
/// decimal, vírgula ou ponto. Sem separador de milhar: "1.000" seria ambíguo.
const DECIMAL = /^[+\-−]?(?:\d+(?:[.,]\d*)?|[.,]\d+)$/;

/// O número em `text`, ou `null` se não for um número decimal.
export function parseDecimal(text: string): number | null {
  const trimmed = text.trim();
  if (!DECIMAL.test(trimmed)) {
    return null;
  }
  const value = Number(trimmed.replace("−", "-").replace(",", "."));
  return Number.isFinite(value) ? value : null;
}

const DECIMAL_FORMAT = new Intl.NumberFormat("pt-BR", {
  maximumFractionDigits: 3,
  useGrouping: false,
});

/// `value` com vírgula decimal e até três casas, sem zeros sobrando: -40, 0,75.
export function formatDecimal(value: number): string {
  return DECIMAL_FORMAT.format(value);
}
