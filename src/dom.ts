// Acesso ao HTML estático de `index.html`.

/// O elemento de `id`, conferido contra o tipo esperado: se o HTML e o código divergirem, a
/// falha é na montagem, com o id no texto, e não num `null` lido mais adiante.
export function byId<T extends HTMLElement>(id: string, type: new () => T): T {
  const element = document.getElementById(id);
  if (!(element instanceof type)) {
    throw new Error(`#${id} is missing or is not a ${type.name}`);
  }
  return element;
}
