# Arquitetura e contrato local

## Fluxo

`Portal ScreenCast → PipeWire/GStreamer → acionamento manual → recorte → PNG em memória → Google Cloud Vision → cache/Google Cloud Translation → D-Bus → GNOME Shell`

Rust mantém o processamento e a rede fora do Shell. GTK4/GJS é usado somente para chave e seleção. A extensão usa `St.Label` como chrome acima das janelas em tela cheia, sem região de entrada nem foco durante a tradução.

## Captura e coordenadas

O portal recebe exatamente o tipo escolhido: `Monitor` ou `Window`, um stream e cursor oculto. O serviço verifica `AvailableSourceTypes` e não substitui silenciosamente uma janela por monitor. A seleção permanece uma sessão explícita e não é restaurada silenciosamente. O descritor do remote PipeWire fica vivo junto do pipeline. O portal `Closed`, EOS, erros GStreamer e alterações de resolução encerram a sessão.

O appsink aceita BGRx/RGBx/BGRA/RGBA em memória de CPU. Não existe `videoconvert` do monitor inteiro. O callback retém apenas uma referência ao buffer mais recente, sem mapear/converter pixels após a seleção. O acionamento manual recorta esse buffer sob demanda; uma cena estática pode reutilizar o mesmo buffer sem esperar outro quadro. O stride informado pelo GStreamer é respeitado; o recorte é copiado em cinza. O limite de 4 megapixels por área limita memória e trabalho acidental.

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
| `GetCropPreview` | — | `ay`: PNG de um recorte recente sob demanda, durante a captura ativa. |
| `SetRegion` | `ss` | JSON de `Rect` e `Monitor`; valida e prepara a área, sem iniciar OCR/rede. |
| `Pause` | — | Invalida trabalhos e pausa captura/rede. |
| `Resume` | — | Prepara a área existente e libera bloqueio de API após ação manual; aguarda outro acionamento. |
| `Refresh` | — | Requer região ativa; faz um recorte e uma leitura OCR, mantendo captura e cache; a legenda anterior conserva seu prazo restante. Não retoma pausa/bloqueio. Agrupa acionamentos enquanto ocupado. |
| `Stop` | — | Idempotente; fecha captura e limpa legenda. |

`Rect = {x: u32, y: u32, width: u32, height: u32}`.
`Monitor = {x: i32, y: i32, width: u32, height: u32}`.

Sinais:

- `StatusChanged(s)`: snapshot JSON com estado, mensagem, geometria, geração, revisão, tradução e métricas.
- `TranslationChanged(tts)`: geração da sessão, revisão do texto, tradução (vazia para limpar).

O snapshot inclui `source_type`: `monitor`, `window` ou nulo sem captura. Na parada, também são limpos posição e tamanho retornados pelo portal.

Estados: `idle`, `opening`, `selecting`, `running`, `paused`, `blocked`, `error`.

Snapshot inclui `ocr_provider` (`google_cloud_vision`), `ocr_count` (tentativas), `ocr_successes`, `ocr_pending`, `ocr_api_ms`, `api_count`, `cache_hits`, `characters_sent`, `ocr_ms`, `api_ms`, `latency_ms` e `subtitle_duration_seconds` (15). Contagens acumulam durante a vida do processo. Os tempos são da última operação, não percentis; o log estruturado permite coletar a distribuição. `api_count` inclui tentativas que falharam. `latency_ms` mede desde o quadro que originou o texto, incluindo OCR e espera de rede, sem espera por estabilidade.

O diagnóstico sob demanda consulta `GetStatus` uma vez por segundo, somente enquanto a página estiver aberta. `mode` é sempre `manual`; `manual_requests` acumula os acionamentos aceitos. `captured_frames` conta o recorte do pedido atual; `last_frame_age_ms` mede o tempo desde esse recorte (nulo antes do primeiro). `ocr_text` mostra a última leitura aceita; `ocr_confidence` permanece nulo por compatibilidade, sem inventar uma confiança agregada para o Cloud Vision. Esses campos são limpos na pausa, parada ou troca de área. `api_pending` indica uma chamada em andamento e `api_successes` conta respostas válidas durante a vida do serviço. Texto reconhecido e traduzido permanecem em memória e não são incluídos nos logs. A página de diagnóstico deve ficar fora da região selecionada.

A extensão exporta `io.github.areatranslator.Overlay.GetMonitors() → s` no nome `org.gnome.Shell`, objeto `/io/github/areatranslator/Overlay`, com os monitores atuais em JSON. `GetVersion() → u` retorna `2`; a interface consulta essa versão antes de iniciar uma captura de janela para evitar usar a geometria de uma extensão antiga ainda carregada no Shell.

## Concorrência e limites

Um ator Tokio serializa comandos e estado. Dois threads de runtime servem I/O; a codificação PNG limitada a 4 megapixels usa `spawn_blocking`. Cada pedido manual faz uma chamada assíncrona ao Cloud Vision e, para texto não vazio fora do cache, no máximo uma chamada ao Translation. Seleção, retomada e chegada de quadros não criam pedidos. A detecção automática de mudanças e o mecanismo de três confirmações foram removidos.

Existe no máximo um pedido manual em andamento. Pressões adicionais enquanto ocupado são agrupadas; não há fila crescente. Pausa, parada e troca de região invalidam a geração e cancelam a rede. Resultados antigos de OCR/rede são descartados. A geração identifica cada pedido; a revisão do sinal acompanha esse identificador.

Cada tradução pronta, inclusive do cache, inicia um prazo monotônico de 15 segundos. O ator aguarda esse prazo independentemente da captura e da rede, emite `TranslationChanged` vazio e limpa o snapshot ao expirar. Um novo pedido, OCR vazio ou erro não prolonga o prazo anterior. Uma resposta válida substitui a legenda e inicia novo prazo; pausa, parada e troca da área a limpam e cancelam o prazo. O temporizador atual é sempre consultado, evitando que um prazo antigo apague uma legenda nova. Falhas temporárias impõem espera progressiva para o próximo acionamento, sem agendar nova chamada; falhas de autenticação/cota bloqueiam novos pedidos até retomada explícita.

`manual_translation_requested` registra geração; `manual_ocr` registra tempo total, tempo de API de OCR e quantidade de caracteres. `ocr_confirmations` permanece por compatibilidade, com zero antes da leitura e um após ela. Imagens, textos e credenciais continuam fora dos logs.

A normalização para envio e cache preserva letras, caixa e pontuação, uniformizando apenas espaços. O cache continua usando chaves exatas, sem correspondência aproximada. O cache guarda 2.000 pares para a combinação fixa inglês → PT-BR/NMT. OCR sem texto não dispara tradução. Cloud Vision tem timeout total de 15 segundos; Translation, de 10 segundos. Ambos têm conexão limitada a 5 segundos; a tradução aceita no máximo 4.000 caracteres por texto. A pausa não garante cancelamento da cobrança de uma requisição já recebida pelo Google.

Nenhum endpoint HTTP é exposto. Não há telemetria. A interface D-Bus pertence à sessão do usuário; outros processos da mesma sessão têm a mesma fronteira de confiança do desktop.

## OCR online e diagnóstico

O serviço envia um PNG em escala de cinza do recorte via `POST https://vision.googleapis.com/v1/images:annotate`, com `TEXT_DETECTION`, dica de idioma `en` e credencial no cabeçalho `X-Goog-Api-Key`. A imagem é codificada em base64 dentro do JSON; o limite de 4 megapixels mantém esse envio abaixo do limite de 10 MB do JSON. Não são enviados o monitor inteiro, a prévia ou um endereço público da imagem.

A leitura usa `fullTextAnnotation.text` ou, na ausência, a descrição completa da primeira `textAnnotations`, sem concatenar novamente as palavras individuais. Resposta sem texto é válida; resposta malformada ou erro por imagem, mesmo sob HTTP 200, é falha. Mensagens de erro do provedor não são exibidas nem registradas literalmente. Não há fallback Tesseract nem chamadas automáticas de repetição.

`GetCropPreview() → ay` recorta o buffer mais recente e codifica PNG em memória sob demanda. Não precisa esperar um novo quadro em uma cena estática. Uma captura pausada ou sem quadro disponível é rejeitada. Não dispara OCR nem rede. O buffer retido é liberado antes de pausar ou fechar o pipeline.

Referências: [OCR do Cloud Vision](https://docs.cloud.google.com/vision/docs/ocr) e [formato da requisição](https://docs.cloud.google.com/vision/docs/request).

## Atualização manual

O instalador salva `Ctrl+A` como padrão, preservando a combinação escolhida e os demais atalhos. A preferência fica na entrada própria de GSettings, mas essa entrada só integra a lista `custom-keybindings` quando o snapshot tem estado `running` e região válida. Pausa, parada, seleção, bloqueio e erro retiram a entrada da lista, sem apagar a combinação. O cliente GJS efêmero chama `Refresh` com `NO_AUTO_START`: não abre GTK, toma foco nem inicia o serviço quando ele estiver parado. A desinstalação remove somente a entrada própria.

O atalho usa `Refresh` para solicitar uma tradução manual. Quando executado explicitamente pelo terminal sem captura ativa, o cliente mostra uma notificação local e encerra, sem abrir GTK nem tomar foco. As teclas não executam esse cliente enquanto a captura estiver parada. Acionamentos válidos conservam somente o prazo restante da legenda anterior durante a operação.

A configuração GTK grava a combinação na mesma entrada de GSettings do atalho global. O gravador suspende os atalhos do sistema somente enquanto a janela modal está aberta e os restaura ao fechá-la. Cancelar não grava alterações; desativar grava uma combinação vazia, preservada pelo instalador. São aceitas combinações com Ctrl/Alt/Super ou teclas de função, com validação contra atalhos personalizados e esquemas comuns do GNOME, incluindo pausa/retomada da extensão quando instalada. Atalhos internos de outros aplicativos não são enumerados.


Na instalação local, o processo Rust inicia um pequeno acompanhante GJS (`shortcut-guard.js`), sem GTK e sem consultas periódicas, para acompanhar `StatusChanged` e a conexão D-Bus única do serviço. A perda dessa conexão também libera o atalho em caso de queda inesperada e encerra o acompanhante. A consulta inicial não pode sobrescrever um evento de estado mais recente. O guardião existe durante a vida do serviço; não executa OCR, captura ou rede. Binários de desenvolvimento fora da estrutura instalada não iniciam esse acompanhante.

A janela GTK oferece **Encerrar captura e liberar atalho**. Fechar a configuração continua independente da captura, pois a seleção também fecha a janela ao preparar o jogo. O menu existente da extensão chama o mesmo `Stop`; nenhuma alteração ou recarga da extensão é necessária para liberar o atalho.
