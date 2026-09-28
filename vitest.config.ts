import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

// Raiz ancorada no próprio arquivo, com a letra do drive normalizada para maiúscula.
// No Windows o Vite trata `c:/…` e `C:/…` como caminhos distintos: herdando o cwd, um
// shell que nasce com a letra minúscula duplica o grafo de módulos e deixa o runner sem
// estado — toda suíte morre na linha do `describe`. Ancorar aqui torna a suíte
// indiferente ao case de quem a invoca.
const here = fileURLToPath(new URL(".", import.meta.url));
const root = here.replace(/^[a-z](?=:)/, (drive) => drive.toUpperCase());

// Não basta declarar o `root`: os workers herdam o cwd do processo e é por ele que
// resolvem `node_modules`. Com os dois em case diferente a divergência só troca de lado.
process.chdir(root);

// Os testes do webview exercitam funções puras (números, durações, frases): nenhum
// precisa de DOM, então o ambiente é o `node`, sem o custo de montar um DOM falso.
export default defineConfig({
  root,
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
