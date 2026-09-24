# Validação

Este relatório mantém o histórico de versões. A versão atual usa Cloud Vision, acionamento manual e legenda por 15 segundos; as medições de Tesseract abaixo são históricas e não representam o OCR online.

# Histórico — 22/09/2026

## Ambiente

- Ubuntu, GNOME Shell 46.0 / Mutter 46.2, Wayland, x86-64.
- Ryzen 5 5600GT, 6 núcleos / 12 threads, aproximadamente 13 GiB de RAM disponível ao sistema.
- Rust 1.96.0; build release com LTO thin. GStreamer 1.24.2; Tesseract 5.3.4.
- Modelo inglês do pacote Ubuntu `tesseract-ocr-eng` 4.1.0-2; o copyright do pacote identifica o upstream como `tessdata_fast`.

## Verificações executadas

| Verificação | Resultado |
| --- | --- |
| Build release | Compilou; binário de aproximadamente 7,9 MiB, sem incluir bibliotecas compartilhadas e modelo. |
| Rust | 10 testes de unidade/contrato passaram. Cobrem recorte e stride real de buffers GStreamer, mudanças de resolução, estabilidade, resultados obsoletos e contrato HTTP com servidor local. |
| OCR nativo | Executado explicitamente nas seis imagens sintéticas abaixo. |
| Geometria da legenda | 4 testes Node passaram, incluindo origem negativa, escala fracionária e prevenção de sobreposição com o recorte. |
| D-Bus do serviço | Inicialização, rejeição de parâmetros inválidos, ausência de chamadas à API sem seleção, Stop idempotente e ausência da chave no snapshot passaram em barramento isolado. |
| GTK4 | Janela de configuração e prévia de seleção renderizaram sob Xvfb; prévia conferida visualmente. |
| Extensão GNOME 46 | Carregou em compositor isolado, exportou monitores e exibiu legenda sobre uma aplicação GTK em tela cheia. A janela de teste manteve foco; a legenda ficou `reactive=false` e `canFocus=false`, fora do retângulo do OCR. Um clique virtual na posição da legenda chegou à aplicação GTK; pausar ocultou a legenda e desativar descarregou a extensão sem erro. |
| Instalador | Instalou em prefixo isolado; launcher instalado passou em `--check`; arquivo desktop validado. |
| Google Cloud real | NMT inglês → PT-BR retornou a tradução correta de uma frase sintética em 456 ms em uma chamada isolada. Credencial restrita salva e conferida no chaveiro; nenhuma credencial integra este repositório. |

Os testes sintéticos da extensão usam respostas simuladas por D-Bus: **não equivalem à captura pelo portal nem a uma tradução real do Google**. A instrumentação fica apenas na cópia de teste em `.deps/`.

![Seleção de área em uma cena sintética](evidence/selection-synthetic.png)

![Legenda sobre aplicação sintética em tela cheia](evidence/overlay-synthetic.png)

## OCR real sobre imagens sintéticas

Tempos de uma execução de desenvolvimento, um thread OpenMP; incluem pré-processamento e reconhecimento, excluem carregamento do modelo, captura e rede.

| Caso | Tempo observado | Confiança Tesseract | Resultado |
| --- | ---: | ---: | --- |
| Diálogo comum | 21 ms | 95 | Texto exato. |
| Fundo escuro | 16 ms | 95 | Texto exato. |
| Fonte pequena | 14 ms | 95 | Texto exato. |
| Fonte pixelada | 20 ms | 80 | Reconheceu as letras e pontuação, mas juntou `dragon is` em `dragonis`. |
| Duas linhas | 24 ms | 95 | Texto exato após normalizar espaços. |
| Quadro vazio | 6 ms | 0 | Resultado vazio, como esperado. |

O teste de fonte pixelada tolera diferenças de espaços explicitamente; os demais exigem correspondência exata. Esses exemplos não demonstram precisão universal em fontes de jogos.

## Consumo em repouso

Serviço release observado por **60 segundos**, sem captura, sem OCR carregado e sem chamadas à API:

- RSS máximo: **12,215 MiB**.
- CPU média: **0,0% da capacidade total**, arredondada pelo coletor baseado em `/proc`.
- 60 amostras. Registros brutos de execução permanecem locais, fora do repositório; o coletor abaixo permite reproduzir a medição.

O resultado exclui a interface GTK, o incremento de memória do GNOME Shell, o modelo carregado e os custos de captura/tradução. Não deve ser usado como estimativa do consumo ativo.

## Validação que ainda depende da sessão de uso

O Google Cloud e a credencial do aplicativo foram configurados e uma tradução real foi validada separadamente. Ainda não houve uma sessão com jogo/emulador para validar o fluxo completo. Permanecem pendentes:

- Autorizar o portal e conferir a captura PipeWire do monitor real integrada ao OCR e à tradução.
- Medir a distribuição de latência da API durante uso contínuo e validar cota/retomada com um projeto Google configurado para testes. A chamada sintética isolada já confirmou autenticação e tradução.
- Conferir seleção e legenda em vários monitores físicos, escala fracionária e troca de resolução.
- Medir **15 minutos de uso ativo**, com comparação do mesmo jogo/cena e configurações, ligado/desligado; confirmar CPU, memória adicional total e impacto no FPS.
- Validar controles reais de teclado, mouse e controle com o aplicativo usado pelo usuário.

As metas de até 250 MB adicionais, menos de 5% da CPU total, menos de 3% de impacto no FPS e tradução em até dois segundos **ainda não foram comprovadas para uso ativo**.

## Reproduzir a medição de 15 minutos

1. Execute um trecho reproduzível do aplicativo alvo, mantendo resolução, limite de FPS e opções gráficas iguais. Registre FPS e tempos de quadro pelo contador/log do próprio aplicativo durante 15 minutos, com o tradutor desligado.
2. Registre também CPU/RSS do GNOME Shell no cenário de referência. Inicie depois a tradução, feche a janela GTK e repita o mesmo trecho por 15 minutos.
3. Encontre o PID do serviço com `pgrep -x area-translator` e o do Shell com `pgrep -x gnome-shell`. Se houver mais de um resultado, use somente o processo da sessão real.
4. Execute `python3 scripts/benchmark.py --pid PID_SERVICO PID_GNOME --seconds 900 --output docs/evidence/active-15min.csv`.
5. Compare a memória do serviço mais o incremento do Shell contra a referência. CPU total é a soma das diferenças, em percentual da máquina; 100% de um núcleo equivale a aproximadamente 8,33% neste processador de 12 threads. Compare FPS e percentis de tempo de quadro registrados pelo aplicativo; RSS sozinho não mede impacto gráfico.

O coletor lê somente `/proc`; ele não grava telas. Para tempos de OCR/API, execute o serviço em primeiro plano com `RUST_LOG=area_translator=info` e armazene os logs de métricas. Eles não contêm o texto reconhecido.

## Ajustes após o primeiro teste com emulador

- Adicionado diagnóstico local de quadros, leitura/confiança do OCR, chamadas à API e tradução, atualizado somente enquanto sua página está aberta.
- Seleções com mais de 400 pixels de altura passam a usar segmentação automática do Tesseract; recortes menores mantêm o modo de bloco único. A escolha reduz a suposição de que uma seleção com cenário constitui um único parágrafo, mas não substitui um recorte adequado.
- Sete imagens sintéticas passaram pelo OCR nativo, incluindo diálogo sobre cenário; o caso de cenário obteve confiança 95 e texto exato. O teste também alterna novamente para o recorte pequeno no mesmo trabalhador.
- Cinco testes de geometria passaram. No compositor GNOME isolado, a legenda cresceu com uma tradução longa e encolheu com uma curta, mantendo-se fora da área de captura. Passagem de cliques e foco continuaram funcionando.
- A evidência enviada durante a depuração foi analisada localmente; não integra o repositório. As metas de consumo durante 15 minutos continuam pendentes.

## Captura por janela — 23/09/2026

- 11 testes Rust e 6 de geometria passaram, incluindo recorte de janela independente da proporção/posição do monitor de saída e pedido de nova seleção após mudança de dimensões.
- O teste com portal simulado executou o serviço Rust real: confirmou `types=2` para janela, `types=1` para monitor, cursor oculto, uma fonte, limpeza após cancelamento e erro sem abrir sessão quando o sistema não oferece captura de janelas. Nenhuma chamada à API foi feita.
- No GNOME 46 isolado, uma legenda com origem de janela permaneceu no rodapé do monitor e cresceu para cima com texto longo, mesmo com recorte de 640 × 480 e monitor de 1280 × 720. A extensão também passou pelo fluxo anterior de monitor, preservação de foco e passagem de cliques.
- Ambas as prévias GTK foram renderizadas e inspecionadas. A seleção por janela identifica o monitor como destino da legenda e não exige a mesma proporção entre janela e monitor.
- A instalação local é atualizada; carregar o novo código da extensão requer renovar a sessão do GNOME. A UI detecta a versão antiga e informa essa necessidade antes de iniciar captura por janela.
- A autorização pelo portal real, o deslocamento/minimização da janela do emulador e a captura de tela cheia precisam ser conferidos em sessão de uso. Os testes de portal simulado e de geometria não comprovam esses comportamentos do compositor real.

## Permanência da legenda — 23/09/2026

- Removida a limpeza imediata da legenda a cada candidato diferente do OCR. Uma leitura vazia passa a exigir 1,5 segundo de estabilidade; a legenda anterior permanece durante falhas pontuais e durante a estabilização de uma nova frase.
- 12 testes unitários passaram. Um teste adicional do motor foi executado em D-Bus isolado, usando instantes simulados: mesma frase após uma hora sem expirar, diferenças de espaços, vazio/ruído transitórios, retorno ao texto anterior, nova frase estável, desaparecimento confirmado e pausa manual.
- O teste do motor usou traduções sintéticas em cache e confirmou zero chamadas à API. Compilação release, formatação e Clippy passaram. Nenhuma alteração da extensão do GNOME foi necessária.

## Oscilações do OCR — 23/09/2026

- Na sessão real, a captura estava identificada como janela e continuava recebendo quadros. Leituras de uma mesma mensagem apresentavam pequenas diferenças que reiniciavam a estabilidade e podiam cancelar uma tradução pendente. A movimentação da janela não foi reproduzida de forma controlada.
- A estabilidade agora tolera ruído limitado em frases longas, preservando números e negações. Uma variante próxima persistente é confirmada como possível mudança real, inclusive quando o compositor deixa de enviar quadros.
- Uma leitura vazia isolada não apaga a legenda: são necessárias duas leituras e pelo menos 1,5 segundo. Um recorte retido em memória permite uma confirmação sem novos quadros. Não há gravação de imagens ou diálogos.
- Passaram 15 testes unitários e três testes do motor em D-Bus isolado. A cobertura inclui ruído durante uma tradução em andamento, manutenção da legenda, desaparecimento confirmado e confirmação única sem novos quadros. Este último usa Tesseract real com imagens sintéticas de diálogo e quadro vazio; confirma recuperação/manutenção do texto, ausência confirmada e zero chamadas à API.
- Formatação, Clippy e compilação release passaram. Não houve alteração da extensão. A permanência da legenda no emulador real com esta versão ainda precisa ser confirmada pelo usuário; os testes não substituem essa validação nem a medição de desempenho de 15 minutos.

## Frase curta sobre cenário — 23/09/2026

- O teste com a versão anterior continuou falhando: o usuário confirmou que a mensagem permanecia na tela quando a legenda sumia. A tolerância a ruído, isoladamente, não resolveu a falha.
- Reproduzido localmente a partir de uma imagem fornecida pelo usuário: Tesseract PSM 3 retornou vazio no recorte amplo, apesar da frase curta legível. Com PSM 11 e filtragem de linhas, o novo código reconheceu a frase completa, sem texto adicional, com confiança 94/100 em dois recortes. O teste usou pixels em memória; a imagem e os recortes não foram incorporados ao repositório.
- Acrescentadas três imagens inteiramente sintéticas: uma frase curta sobre dois cenários distintos e o cenário sem texto. O teste nativo reconhece a mesma frase nos dois fundos e retorna vazio no terceiro, além de preservar os sete casos anteriores.
- Acrescentada prévia sob demanda do recorte no diagnóstico. Os testes verificam que ela rejeita captura ausente/pausada, recebe somente o recorte solicitado, codifica PNG cinza e não faz chamadas à API. A cópia é feita uma única vez por solicitação.
- Passaram 17 testes unitários, três testes do motor em D-Bus isolado, o teste nativo com dez imagens, Clippy, formatação e compilação release. O ciclo D-Bus foi verificado e a página de diagnóstico GTK foi renderizada no Xvfb. A captura real após a atualização e as metas de desempenho ainda precisam de validação.

## Confirmação antes de traduzir — 23/09/2026

- A versão anterior registrou 17 OCRs e 12 traduções concluídas, em cerca de 30 segundos entre a primeira e a última resposta. Uma única leitura podia ser aceita depois de 500 ms sem mudança visual; a tolerância a diferenças não se aplicava a frases curtas.
- Substituída essa regra: três leituras consecutivas iguais ao longo de pelo menos um segundo confirmam uma frase. Vazio usa pelo menos 1,5 segundo. A revisão agora muda apenas na confirmação; candidatos não cancelam a tradução em andamento nem trocam a legenda. Esta regra substitui a comparação aproximada e a confirmação única descritas nas validações históricas acima.
- Regressão com servidor HTTP local: cinco candidatos transitórios iniciais geraram zero chamadas. Após uma frase confirmada, vinte grupos de variantes (duas leituras cada), incluindo texto vazio, mantiveram a legenda e não acrescentaram chamadas. Uma frase nova confirmada acrescentou exatamente uma chamada; retornar à primeira usou o cache. Teste também verificou desaparecimento confirmado e pausa. Nenhuma credencial real ou chamada ao Google foi usada nesse teste.
- Passaram 17 testes unitários e três testes de integração em D-Bus isolado. O teste nativo do motor confirmou diálogo estático e desaparecimento mesmo sem novos quadros: três OCRs em cada caso e nenhum OCR adicional após confirmação. Outro teste confirmou que ruído não aborta a tradução em andamento.
- Formatação, Clippy, compilação release, verificação das dependências e ciclo D-Bus passaram. Serviço e interface foram atualizados na instalação local; a extensão do GNOME não mudou.
- A confirmação exige consistência do OCR: uma leitura incorreta repetida três vezes ainda pode ser aceita, e texto persistentemente instável permanece pendente. A validação com o emulador real após esta mudança e a medição de desempenho de 15 minutos continuam pendentes.


## Atalho de atualização manual — 2026-09-23

- Adicionado `Refresh` no D-Bus, botão GTK, item no menu e atalho global padrão **Super + Shift + R**. Limpa a legenda e reconfirma o recorte, preservando região, captura e cache. Não retoma pausa ou bloqueio.
- Passaram 17 testes unitários e cinco testes do motor em D-Bus isolado. Os novos casos verificam rejeição sem região/em pausa, cancelamento de tradução antiga, descarte de OCR antigo, preservação do recorte mais recente, agrupamento de repetições da tecla, recuperação dos pixels quando o trabalhador está ocupado e confirmação de vazio. A reutilização do cache não disparou chamadas à API.
- O cliente do atalho foi executado contra um serviço simulado no GNOME 46 isolado: exatamente uma chamada a `Refresh`, foco mantido na aplicação sintética em tela cheia e passagem de cliques preservada. O teste D-Bus confirmou também que executar o cliente sem serviço não o inicia.
- O teste de registro/atualização/remoção do atalho usou GSettings em memória e confirmou a preservação de outros atalhos e da combinação personalizada. Instalador validado com prefixo contendo espaço; launcher, dependências e arquivo desktop passaram nas verificações.
- Formatação, Clippy e compilação release passaram. Serviço, interface, extensão e cliente foram instalados localmente; o registro real do atalho e os arquivos instalados foram conferidos. A nova opção do menu será carregada pelo GNOME no próximo login.
- Não houve teste de pressão física do atalho durante captura real do emulador nem medição adicional de desempenho. Estes testes não demonstram correção de toda leitura instável do OCR.


## Configuração do atalho na interface — 2026-09-23

- A tela principal mostra a combinação salva e permite gravar outra, restaurar o padrão ou desativar o atalho. Cancelar descarta a alteração. O registro global continua usando a mesma entrada do Ubuntu.
- Os testes GTK passaram no Xvfb e no GNOME 46/Wayland isolado: validação de combinações, conflito com outro atalho personalizado e com Alt+F4, gravação por evento de teclado, salvamento, cancelamento, desativação e restauração. O teste do instalador também confirmou que atualizar não reativa uma combinação desativada.
- A tela principal e o gravador foram renderizados e inspecionados. O instalador foi executado em prefixo isolado com espaço, incluindo o novo módulo. A interface foi atualizada na instalação local, sem alterar a combinação existente nem reiniciar a captura.
- Não houve teste de pressão física de teclas durante o jogo; os eventos do gravador foram simulados. Esta mudança configura o atalho de refresh existente; não altera o motor para funcionar exclusivamente sob demanda.


## Tradução exclusivamente manual — 2026-09-23

Esta versão substitui o fluxo automático descrito nas seções históricas acima.

- Selecionar/retomar a área e receber novos quadros não executam OCR ou tradução. O atalho configurado ou o botão **Traduzir agora** solicita um único recorte, um OCR e no máximo uma chamada à API; o cache evita chamadas repetidas. Acionamentos durante uma operação são agrupados.
- O serviço mantém uma referência ao buffer mais recente do PipeWire e só converte o recorte sob demanda. Isso permite reler uma cena estática. A referência é liberada antes de pausar/fechar o pipeline. O diagnóstico também pode solicitar um recorte, sem OCR/rede.
- A legenda anterior permanece durante a leitura, resposta da API, leitura vazia e falha de rede. Não há repetição automática após erros; a espera progressiva limita novos acionamentos. Pausa, encerramento e seleção de outra área limpam a legenda e invalidam trabalhos antigos.
- A combinação do usuário estava registrada, e o serviço anterior havia recebido pedidos de refresh; isso não demonstra que todos os acionamentos físicos chegaram. O fluxo anterior ainda aguardava três leituras iguais. O cliente também ocultava erros de captura inativa; agora envia uma notificação local sem abrir GTK.
- Passaram nove testes unitários e quatro testes de integração do motor em D-Bus isolado, incluindo Tesseract real e API HTTP simulada. Mudanças de imagem sem acionamento produziram zero OCRs adicionais; três pedidos (diálogo, vazio, diálogo repetido) produziram três OCRs e apenas uma chamada à API. Também foram verificados agrupamento de pressões, cache, erros 503/403/429 sem repetição automática, bloqueio/espera progressiva, pausa, cancelamento e descarte de resultados antigos.
- Cliente do atalho testado com serviço de notificações simulado: serviço ausente e pedido rejeitado geram aviso; pedido aceito chama Refresh uma vez e não gera aviso. Ciclo D-Bus, Clippy, formatação, compilação release, interface GTK e extensão no GNOME 46/Wayland isolado passaram.
- Serviço e interface instalados localmente; atalho Ctrl+A preservado. O snapshot instalado informa modo manual, inicialmente com zero pedidos, OCRs e chamadas à API. O teste físico do atalho no emulador e a medição de desempenho de 15 minutos ainda dependem de validação real.

- No teste conduzido pelo usuário após a instalação, a captura foi selecionada e retomada. O serviço registrou três pedidos manuais, três OCRs e três respostas válidas do Google, sem OCRs adicionais além dos pedidos recebidos. O snapshot manteve tradução presente após as respostas. Essa observação valida o caminho real de captura/OCR/API para esses acionamentos; não substitui confirmação visual da legenda nem a medição de desempenho.

## Cloud Vision e legenda por 15 segundos — 23/09/2026

Esta versão substitui o Tesseract pelo Google Cloud Vision `TEXT_DETECTION`. O acionamento continua exclusivamente manual. A imagem PNG em escala de cinza do recorte é enviada ao Google; a aplicação não salva capturas em disco. O cache economiza chamadas de tradução, mas não elimina o OCR online de cada acionamento.

- Uma chamada real com a imagem pública `tests/fixtures/dialog.png` reconheceu os 33 caracteres esperados (após normalização de espaços), em 484 ms totais / 482 ms de API. O texto foi traduzido para PT-BR na API real em 375 ms, com 40 caracteres de saída. Credencial acessada do chaveiro em memória, sem exposição no comando ou saída. Esses tempos são de uma amostra sintética, não um benchmark de jogo.
- Passaram dez testes unitários e sete testes do motor em D-Bus isolado. Os servidores HTTP locais verificam PNG do recorte, chave no cabeçalho, uma operação por acionamento, cache, OCR vazio, respostas malformadas, erros por imagem sob HTTP 200, autenticação/cota e ausência de repetição automática. Pausa/parada invalidam operações pendentes.
- O temporizador foi executado por 15 segundos reais no ator, sem captura ou comandos: a legenda estava presente aos 14 segundos e foi removida ao vencer o prazo, sem OCR ou tradução adicional. Outros casos verificaram novo prazo para resposta do cache, descarte do prazo antigo e que novo pedido, vazio e erro não prolongam a legenda anterior.
- Formatação, Clippy, build release e contrato D-Bus passaram. A janela GTK foi renderizada e inspecionada no Xvfb, incluindo os avisos de envio da imagem e duração da legenda. Instalador verificado em prefixo com espaço e instalado na sessão real, removendo somente os arquivos de Tesseract anteriormente incluídos pela aplicação.
- O serviço instalado foi reiniciado e conferido: modo manual, provedor `google_cloud_vision`, duração 15 segundos e nenhuma requisição antes de acionar. Ctrl+A e a credencial do chaveiro foram preservados. A extensão não mudou; os sinais existentes removem a legenda sem exigir novo login.
- A qualidade desta versão nos diálogos reais do emulador, a observação visual dos 15 segundos sobre o jogo e a sessão de desempenho de 15 minutos continuam pendentes. Os testes acima não garantem acurácia universal nem as metas de CPU, RAM ou FPS.

## Liberação do atalho ao parar — 23/09/2026

- Corrigido o registro permanente do Ctrl+A: a combinação permanece salva, mas a entrada só integra os atalhos ativos do GNOME durante uma captura em estado `running`. Pausa, encerramento, bloqueio e erro liberam a tecla. Acrescentado o botão **Encerrar captura e liberar atalho** na interface.
- Teste GJS em D-Bus isolado, com GSettings em memória, verificou ativação/retomada, pausa/parada, bloqueio/erro, região ausente, perda inesperada da conexão do serviço, preservação de outra entrada e da combinação escolhida. Um snapshot inicial atrasado não reativou o atalho depois de uma pausa.
- Os testes do instalador e do gravador GTK passaram, incluindo alterar a combinação quando o atalho está temporariamente fora da lista ativa. Formatação, Clippy, build release e ciclo D-Bus do binário instalado em prefixo com espaço passaram. A tela com o novo botão foi renderizada e inspecionada no Xvfb.
- Instalação real atualizada sem recarregar a extensão: serviço em `idle`, Ctrl+A salvo, entrada ausente da lista global ativa e exatamente um acompanhante de estado sem GTK. Zero requisições de OCR/tradução foram feitas nessa verificação.
- Não houve simulação de pressão física de Ctrl+A na sessão real nem medição adicional de desempenho. A verificação da sessão real consultou o registro efetivo do Ubuntu; o teste isolado exercitou as transições de estado sem capturar a tela ou acessar o Google.
