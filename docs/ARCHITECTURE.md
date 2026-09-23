# Arquitetura e contrato local

## Fluxo

`Portal ScreenCast → PipeWire/GStreamer → recorte → detecção de mudanças → trabalhador Tesseract → estabilidade → cache/Google Cloud → D-Bus → GNOME Shell`

Rust mantém o processamento e a rede fora do Shell. GTK4/GJS é usado somente para chave e seleção. A extensão usa `St.Label` como chrome acima das janelas em tela cheia, sem região de entrada nem foco durante a tradução.

## Captura e coordenadas

O portal recebe exatamente o tipo escolhido: `Monitor` ou `Window`, um stream e cursor oculto. O serviço verifica `AvailableSourceTypes` e não substitui silenciosamente uma janela por monitor. A seleção permanece uma sessão explícita e não é restaurada silenciosamente. O descritor do remote PipeWire fica vivo junto do pipeline. O portal `Closed`, EOS, erros GStreamer e alterações de resolução encerram a sessão.

O appsink aceita BGRx/RGBx/BGRA/RGBA em memória de CPU. Não existe `videoconvert` do monitor inteiro. Quadros são descartados antes de mapear pixels se chegaram dentro da janela de 200 ms. O stride informado pelo GStreamer é respeitado; o recorte é copiado em cinza. O limite de 4 megapixels por área limita memória e trabalho acidental.

Durante a seleção existe somente uma prévia RGB congelada. `GetPreview` a codifica em PNG em memória. Ao confirmar, a prévia é descartada. Posição e tamanho da região são pixels da captura; monitor é um retângulo de coordenadas lógicas do GNOME. No modo monitor, a extensão projeta a região usando a razão entre dimensões da captura e do monitor. O usuário confirma o monitor quando a identificação do portal é insuficiente; a legenda fica completamente fora do recorte. No modo janela, a região é relativa ao stream da janela e não é projetada no monitor: o portal não informa a posição dessa janela na tela. O monitor escolhido define somente a saída da legenda, inicialmente no rodapé e reposicionável. O chrome do Shell não integra o stream da janela, portanto não é necessário reservar uma faixa fora do recorte. Mudanças nas dimensões do stream invalidam a seleção em ambos os modos; não há redimensionamento automático de coordenadas.

## Serviço D-Bus

- Nome/interface: `io.github.areatranslator.Service`
- Objeto: `/io/github/areatranslator/Service`
- Ativação: arquivo de serviço D-Bus instalado no diretório do usuário.

| Método | Entrada | Saída / efeito |
| --- | --- | --- |
| `SetApiKey` | `s` | Guarda a chave apenas na memória do serviço. A UI usa Secret Service para persistência. |
| `BeginSelection` | — | Cancela a sessão anterior e abre o portal para monitor, preservando o contrato anterior. Requer chave configurada. |
| `BeginWindowSelection` | — | Cancela a sessão anterior e abre o portal para uma janela. Requer chave configurada. |
| `GetStatus` | — | `s`: snapshot JSON sem credencial. |
| `GetPreview` | — | `ay`: PNG da prévia em memória, somente durante a seleção. |
| `SetRegion` | `ss` | JSON de `Rect` e `Monitor`; valida e inicia o processamento. |
| `Pause` | — | Invalida trabalhos e pausa captura/rede. |
| `Resume` | — | Reinicia leitura da área existente e libera bloqueio de API após ação manual. |
| `Stop` | — | Idempotente; fecha captura e limpa legenda. |

`Rect = {x: u32, y: u32, width: u32, height: u32}`.
`Monitor = {x: i32, y: i32, width: u32, height: u32}`.

Sinais:

- `StatusChanged(s)`: snapshot JSON com estado, mensagem, geometria, geração, revisão, tradução e métricas.
- `TranslationChanged(tts)`: geração da sessão, revisão do texto, tradução (vazia para limpar).

O snapshot inclui `source_type`: `monitor`, `window` ou nulo sem captura. Na parada, também são limpos posição e tamanho retornados pelo portal.

Estados: `idle`, `opening`, `selecting`, `running`, `paused`, `retrying`, `blocked`, `error`.

Snapshot inclui `ocr_count`, `api_count`, `cache_hits`, `characters_sent`, `ocr_ms`, `api_ms`, `latency_ms`. Contagens acumulam durante a vida do processo. Os tempos são da última operação, não percentis; o log estruturado permite coletar a distribuição. `api_count` inclui tentativas que falharam. `latency_ms` mede desde o quadro que originou o texto, incluindo estabilidade e espera de rede.

O diagnóstico sob demanda consulta `GetStatus` uma vez por segundo, somente enquanto a página estiver aberta. `captured_frames` conta recortes recebidos desde a seleção/retomada; `last_frame_age_ms` mede a idade do último quadro recebido (nulo antes do primeiro). `ocr_text` e `ocr_confidence` mostram a última leitura aceita e a confiança; texto de baixa confiança fica vazio. Esses campos são limpos na pausa, parada ou troca de área. `api_pending` indica uma chamada em andamento e `api_successes` conta respostas válidas durante a vida do serviço. Texto reconhecido e traduzido permanecem em memória e não são incluídos nos logs. A página de diagnóstico deve ficar fora da região selecionada.

A extensão exporta `io.github.areatranslator.Overlay.GetMonitors() → s` no nome `org.gnome.Shell`, objeto `/io/github/areatranslator/Overlay`, com os monitores atuais em JSON. `GetVersion() → u` retorna `2`; a interface consulta essa versão antes de iniciar uma captura de janela para evitar usar a geometria de uma extensão antiga ainda carregada no Shell.

## Concorrência e limites

Um ator Tokio serializa comandos e estado. Dois threads de runtime servem I/O; um trabalhador nativo mantém o handle Tesseract. O canal de OCR comporta um trabalho, e um slot mantém somente o último recorte relevante enquanto o trabalhador está ocupado. Uma tarefa de rede é cancelada quando o texto muda, na pausa ou na seleção de nova região. Resultados são aceitos somente se geração e revisão ainda correspondem.

A revisão acompanha o candidato atual do OCR, mas a legenda exibida é preservada durante a confirmação de uma mudança. Texto não vazio usa estabilidade de 500 ms; ausência de texto usa 1.500 ms. Texto não vazio exige imagem estável ou leituras repetidas. Ausência de texto exige ao menos duas leituras; se houver legenda visível e a imagem ficar parada após uma única leitura vazia, o motor confirma uma vez usando o último recorte. O trabalhador devolve esse recorte por movimentação de memória, sem cópia adicional. Há no máximo um recorte retido para confirmação (até 4 MB), liberado na pausa, parada ou troca de região. Uma nova frase estável substitui diretamente a legenda se estiver em cache; caso contrário, limpa a anterior enquanto aguarda a API. Leituras idênticas mantêm a legenda por tempo ilimitado. Pausa, parada, troca de região e falhas que interrompem a sessão continuam limpando a legenda imediatamente.

A estabilidade tolera pequenas diferenças em frases com pelo menos oito palavras e 60 caracteres: distância de edição de até 1/16 do comprimento, limitada a oito caracteres, ignorando caixa e pontuação nas bordas das palavras. A comparação usa sempre o candidato de referência, impedindo deriva gradual. Números e negações devem continuar iguais; uma variante próxima que se repete por 500 ms é adotada como possível mudança real. Se a imagem parar, essa variante também recebe uma confirmação pelo recorte retido, evitando perder pequenas mudanças reais em diálogos estáticos. Essa heurística reduz ruído, mas pode atrasar a identificação de mudanças pequenas em frases longas.

A normalização para envio e cache preserva letras, caixa e pontuação, uniformizando apenas espaços. O cache continua usando chaves exatas, sem correspondência aproximada. O cache guarda 2.000 pares para a combinação fixa inglês → PT-BR/NMT. OCR de baixa confiança é considerado vazio, evitando enviar ruído. Requisições têm timeout total de 10 segundos, conexão de 5 segundos e no máximo 4.000 caracteres por texto. A pausa não garante cancelamento da cobrança de uma requisição já recebida pelo Google.

Nenhum endpoint HTTP é exposto. Não há telemetria. A interface D-Bus pertence à sessão do usuário; outros processos da mesma sessão têm a mesma fronteira de confiança do desktop.
