# Arquitetura e contrato local

## Fluxo

`Portal ScreenCast → PipeWire/GStreamer → acionamento manual → recorte → uma leitura Tesseract → cache/Google Cloud → D-Bus → GNOME Shell`

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
| `Refresh` | — | Requer região ativa; faz um recorte e uma leitura OCR, mantendo captura, modelo, legenda anterior e cache. Não retoma pausa/bloqueio. Agrupa acionamentos enquanto ocupado. |
| `Stop` | — | Idempotente; fecha captura e limpa legenda. |

`Rect = {x: u32, y: u32, width: u32, height: u32}`.
`Monitor = {x: i32, y: i32, width: u32, height: u32}`.

Sinais:

- `StatusChanged(s)`: snapshot JSON com estado, mensagem, geometria, geração, revisão, tradução e métricas.
- `TranslationChanged(tts)`: geração da sessão, revisão do texto, tradução (vazia para limpar).

O snapshot inclui `source_type`: `monitor`, `window` ou nulo sem captura. Na parada, também são limpos posição e tamanho retornados pelo portal.

Estados: `idle`, `opening`, `selecting`, `running`, `paused`, `blocked`, `error`.

Snapshot inclui `ocr_count`, `api_count`, `cache_hits`, `characters_sent`, `ocr_ms`, `api_ms`, `latency_ms`. Contagens acumulam durante a vida do processo. Os tempos são da última operação, não percentis; o log estruturado permite coletar a distribuição. `api_count` inclui tentativas que falharam. `latency_ms` mede desde o quadro que originou o texto, incluindo OCR e espera de rede, sem espera por estabilidade.

O diagnóstico sob demanda consulta `GetStatus` uma vez por segundo, somente enquanto a página estiver aberta. `mode` é sempre `manual`; `manual_requests` acumula os acionamentos aceitos. `captured_frames` conta o recorte do pedido atual; `last_frame_age_ms` mede o tempo desde esse recorte (nulo antes do primeiro). `ocr_text` e `ocr_confidence` mostram a última leitura aceita e a confiança; texto de baixa confiança fica vazio. Esses campos são limpos na pausa, parada ou troca de área. `api_pending` indica uma chamada em andamento e `api_successes` conta respostas válidas durante a vida do serviço. Texto reconhecido e traduzido permanecem em memória e não são incluídos nos logs. A página de diagnóstico deve ficar fora da região selecionada.

A extensão exporta `io.github.areatranslator.Overlay.GetMonitors() → s` no nome `org.gnome.Shell`, objeto `/io/github/areatranslator/Overlay`, com os monitores atuais em JSON. `GetVersion() → u` retorna `2`; a interface consulta essa versão antes de iniciar uma captura de janela para evitar usar a geometria de uma extensão antiga ainda carregada no Shell.

## Concorrência e limites

Um ator Tokio serializa comandos e estado. Dois threads de runtime servem I/O; um trabalhador nativo mantém o handle Tesseract. Cada pedido manual faz exatamente um OCR e, para texto não vazio fora do cache, no máximo uma chamada à API. Seleção, retomada e chegada de quadros não criam pedidos. A detecção automática de mudanças e o mecanismo de três confirmações foram removidos.

Existe no máximo um pedido manual em andamento. Pressões adicionais enquanto ocupado são agrupadas; não há fila crescente. Pausa, parada e troca de região invalidam a geração e cancelam a rede. Resultados antigos de OCR/rede são descartados. A geração identifica cada pedido; a revisão do sinal acompanha esse identificador.

A legenda anterior permanece enquanto o novo pedido é processado e em leituras vazias ou falhas. Uma resposta válida a substitui; pausa, parada e troca da área a limpam. Falhas temporárias impõem espera progressiva para o próximo acionamento, sem agendar nova chamada; falhas de autenticação/cota bloqueiam novos pedidos até retomada explícita.

`manual_translation_requested` registra geração; `manual_ocr` registra tempo, confiança e quantidade de caracteres. `ocr_confirmations` permanece por compatibilidade, com zero antes da leitura e um após ela. Imagens, textos e credenciais continuam fora dos logs.

A normalização para envio e cache preserva letras, caixa e pontuação, uniformizando apenas espaços. O cache continua usando chaves exatas, sem correspondência aproximada. O cache guarda 2.000 pares para a combinação fixa inglês → PT-BR/NMT. OCR de baixa confiança é considerado vazio, evitando enviar ruído. Requisições têm timeout total de 10 segundos, conexão de 5 segundos e no máximo 4.000 caracteres por texto. A pausa não garante cancelamento da cobrança de uma requisição já recebida pelo Google.

Nenhum endpoint HTTP é exposto. Não há telemetria. A interface D-Bus pertence à sessão do usuário; outros processos da mesma sessão têm a mesma fronteira de confiança do desktop.

## Texto curto em cenas grandes

Recortes com mais de 400 pixels de altura usam Tesseract PSM 11 (texto esparso), com saída TSV para agrupar palavras por linha. O modo automático PSM 3 podia retornar vazio mesmo com uma frase curta legível. Linhas são aceitas por confiança média ponderada pelos caracteres alfanuméricos (mínimo 65). A filtragem preserva a linha completa, inclusive palavras incertas como negações e números. Fragmentos isolados com menos de três caracteres são descartados, salvo algumas opções curtas comuns e números de dois ou mais dígitos com confiança alta. Isso reduz falsos textos do cenário, mas pode omitir rótulos curtos; seleções justas com até 400 pixels continuam no modo de bloco PSM 6, sem esse filtro.

`GetCropPreview() → ay` recorta o buffer mais recente e codifica PNG em memória sob demanda. Não precisa esperar um novo quadro em uma cena estática. Uma captura pausada ou sem quadro disponível é rejeitada. Não dispara OCR nem rede. O buffer retido é liberado antes de pausar ou fechar o pipeline.

Referência do formato TSV: [documentação do Tesseract](https://tesseract-ocr.github.io/tessdoc/Command-Line-Usage.html#tsv-output).

## Atualização manual

O instalador registra `Super + Shift + R` como atalho personalizado do GNOME, preservando os demais atalhos e uma combinação alterada pelo usuário em reinstalações. O cliente GJS efêmero chama `Refresh` com `NO_AUTO_START`: não abre GTK, toma foco nem inicia o serviço quando ele estiver parado. A desinstalação remove somente a entrada própria.

O atalho usa `Refresh` para solicitar uma tradução manual. Quando não existe captura ativa, o cliente mostra uma notificação local e encerra, sem abrir GTK nem tomar foco. Acionamentos válidos mantêm a legenda anterior durante a operação.

A configuração GTK grava a combinação na mesma entrada de GSettings do atalho global. O gravador suspende os atalhos do sistema somente enquanto a janela modal está aberta e os restaura ao fechá-la. Cancelar não grava alterações; desativar grava uma combinação vazia, preservada pelo instalador. São aceitas combinações com Ctrl/Alt/Super ou teclas de função, com validação contra atalhos personalizados e esquemas comuns do GNOME, incluindo pausa/retomada da extensão quando instalada. Atalhos internos de outros aplicativos não são enumerados.
