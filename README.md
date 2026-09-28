# Timeboat

App de Windows que remove os silêncios de um vídeo ou áudio. Arraste o arquivo para a janela (ou clique para escolher no Explorer): o resultado é gravado na mesma pasta, com `(TIMEBOATED)` no fim do nome (`aula.mp4` vira `aula (TIMEBOATED).mp4`). O original nunca é alterado, e nenhum arquivo existente é sobrescrito.

Durante o processamento a janela mostra uma barra de progresso, o tempo decorrido e o botão **Interromper**, que pede confirmação e não deixa arquivo incompleto na pasta. Ao terminar, mostra quanto o vídeo perdeu e quanto o processamento levou, em minutos:segundos.

## Requisitos

- Windows 10 ou 11, com o WebView2 (já vem no Windows 11).
- **ffmpeg** no `PATH`, com o `ffprobe`: `winget install Gyan.FFmpeg`. A janela avisa se não o encontrar; depois de instalar, feche e abra o app de novo.

## Configurações

| Na tela | Padrão | O que faz |
| --- | --- | --- |
| Filtrar abaixo do nível de som | -40 dB | O áudio abaixo deste nível conta como silêncio. |
| Remover silêncios maiores que | 3 s | Só o silêncio mais longo que isto é cortado. |
| Ignorar detecções menores que | 0,75 s | O som mais curto que isto entre silêncios (clique, tosse) conta como silêncio. |
| Margem à esquerda e à direita | 0,25 s | Silêncio mantido de cada lado de cada corte. |

As configurações ficam gravadas em `%APPDATA%\com.timeboat.desktop\settings.json`. A regra exata de cada uma está no `//!` de [`src-tauri/src/timeline.rs`](src-tauri/src/timeline.rs).

## Formato do resultado

Vídeo sai sempre em MP4 (H.264 + AAC), qualquer que seja o contêiner de entrada. Áudio mantém o formato (MP3, WAV, M4A, FLAC, OGG, OPUS); WMA sai em M4A. A tabela completa é a `FORMATS` de [`src-tauri/src/media.rs`](src-tauri/src/media.rs).

## Desenvolvimento

```bash
npm install
npm run app          # tauri dev
npm run app:build    # instalador NSIS em target/release/bundle/nsis/
```

A régua de tipagem e lint é a do repositório tudolindo: clippy `all` + `pedantic` + `nursery` negados, `unsafe` proibido, `unwrap`/`expect`/`panic` vetados (inclusive nos testes); TypeScript `strict` com `noUncheckedIndexedAccess` e `exactOptionalPropertyTypes`; Biome no lint e na formatação. Nenhuma supressão de lint no código.

| Comando | O que roda |
| --- | --- |
| `npm run ts:check` | Biome e `tsc` (webview e configs) |
| `npm run rs:check` | clippy com `-D warnings`, `rustfmt --check` e `cargo doc` |
| `npm test` / `npm run rs:test` | vitest / `cargo test` — este usa o ffmpeg de verdade |
| `npm run gate` | tudo acima, mais o build do webview, `cargo deny` e `npm audit` |

O union `IpcErrorCode` de `src/ipc-error-codes.generated.ts` é gerado do enum do Rust: depois de mexer em `src-tauri/src/error.rs`, rode `UPDATE_IPC_TS=1 cargo test`.
