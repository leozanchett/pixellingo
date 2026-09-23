# PixelLingo — Tradução além dos pixels

Ao acionar um atalho configurável, recorta uma região de uma janela ou monitor, reconhece o texto em inglês localmente e mostra a tradução para português brasileiro em uma legenda transparente. Independente de emulador, jogo ou aplicativo.

**Plataforma inicial:** Ubuntu 24.04, GNOME Shell 46 e Wayland, x86-64. O serviço é Rust, a interface sob demanda é GTK4/GJS e a legenda é uma extensão do GNOME. A tradução usa Google Cloud Translation Basic/NMT.

**Estado:** versão inicial 0.1.0 para testes. OCR, interface, extensão e uma chamada real de tradução foram verificados separadamente. A validação completa com emulador e as medições de desempenho durante 15 minutos ainda estão pendentes.

![Legenda transparente sobre uma cena sintética em tela cheia](docs/evidence/overlay-synthetic.png)

*A imagem mostra um teste sintético da extensão com tradução simulada; não representa uma sessão de jogo real.*

## Navegação

- [Recursos e compatibilidade](#recursos-e-compatibilidade)
- [Instalação](#instalar)
- [Primeiro uso](#usar)
- [Teste com emulador](#primeiro-teste-com-emulador)
- [Consumo de recursos](#como-mantém-o-consumo-baixo)
- [Privacidade e custos](#privacidade-custos-e-falhas)
- [Desenvolvimento](#desenvolvimento-e-testes)
- [Solução de problemas](#solução-de-problemas)

## Recursos e compatibilidade

| Item | Suporte inicial |
| --- | --- |
| Desktop | Ubuntu 24.04, GNOME Shell 46, sessão Wayland, x86-64 |
| Texto | Inglês → português brasileiro; um bloco de texto por vez |
| Aplicações | Captura de uma janela ou monitor, com recorte interno, independente do emulador |
| Legenda | Transparente, com contorno e fundo translúcido opcional |
| Entrada | Preserva o foco e permite passagem de cliques durante o uso |
| OCR | Tesseract local com modelo inglês `tessdata_fast` |
| Tradução | Google Cloud Translation Basic/NMT; requer internet e credencial própria |
| Controles | Tradução exclusivamente manual por atalho configurável ou botão; pausa/retomada |

Outras versões do GNOME, KDE, X11, Windows e macOS não foram validadas. Não há tradução offline nem seleção de várias regiões simultâneas. No modo janela, o recorte acompanha seu deslocamento; a legenda permanece no monitor escolhido.

## Arquitetura

```mermaid
flowchart LR
    A[Janela ou monitor autorizado pelo portal] --> B[PipeWire / GStreamer]
    B --> C[Recorte solicitado pelo atalho]
    C --> D[OCR Tesseract local]
    D --> E[Cache em memória]
    E -->|Somente texto por HTTPS| F[Google Cloud Translation]
    E -->|Tradução em cache| G[D-Bus local]
    F --> G
    G --> H[Legenda no GNOME Shell]
```

O serviço Rust executa captura, OCR e rede fora do GNOME Shell. A interface GTK4 abre somente para configuração e seleção. O contrato entre componentes está em [Arquitetura e D-Bus](docs/ARCHITECTURE.md).

## Instalar

Clone o repositório na sua máquina:

```sh
git clone https://github.com/leozanchett/pixellingo.git
cd pixellingo
```

Confira a sessão com `gnome-shell --version` e `echo "$XDG_SESSION_TYPE"`: esta versão foi desenvolvida para GNOME 46 e `wayland`.


Rust/Cargo 1.88 ou superior deve estar disponível; consulte o [instalador oficial do Rust](https://www.rust-lang.org/tools/install). Python 3 também é usado pelo instalador. No Ubuntu, os componentes de interface e captura normalmente já estão instalados. Dependências de sistema:

```sh
sudo apt update
sudo apt install git python3 build-essential pkg-config libglib2.0-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-pipewire gstreamer1.0-plugins-base \
  libtesseract5 tesseract-ocr-eng gjs gir1.2-gtk-4.0 gir1.2-secret-1
./scripts/install.sh
```

Alternativa **sem sudo**, em Ubuntu 24.04 x86-64 com GTK4/GJS, GLib de desenvolvimento e GStreamer já disponíveis:

```sh
./scripts/install.sh --local-deps
```

Essa opção baixa pacotes dos repositórios APT configurados, extrai os arquivos de compilação em `.deps/` e instala apenas o runtime de OCR e o modelo inglês junto da aplicação. Não modifica os pacotes do sistema. O instalador compila com `Cargo.lock`, valida o runtime e grava os arquivos em `~/.local/`.

Ative a extensão:

```sh
gnome-extensions enable area-translator@local
```

Na primeira instalação, o GNOME pode precisar que você **saia da sessão e entre novamente** para descobrir a extensão. Isso é diferente de bloquear e desbloquear a tela. Ao atualizar uma extensão já carregada, também é necessário renovar a sessão para carregar o novo código. Depois, abra **Tradutor de área** no menu de aplicativos ou execute `~/.local/bin/area-translator-ui`.

## Usar

1. Configure um projeto com faturamento habilitado, ative a **Cloud Translation API** e crie uma chave restrita a essa API. Siga o [guia de configuração do Google Cloud](docs/CLOUD_SETUP.md). Se a credencial já estiver no chaveiro desta sessão, não é necessário cadastrá-la novamente.
2. Cole a chave na janela e clique em **Salvar chave**. Ela fica no chaveiro do sistema, não em arquivo de configuração. Não coloque a chave no repositório.
3. Em **O que capturar?**, escolha **Janela do aplicativo** (padrão) ou **Monitor inteiro**. Clique em **Selecionar área** e autorize a janela do emulador ou o monitor no diálogo do Ubuntu.
4. Marque a caixa de texto na prévia. No modo janela, escolha **Onde exibir a legenda**; no modo monitor, confirme o monitor compartilhado e deixe pelo menos 110 pixels lógicos livres acima ou abaixo do recorte.
5. Clique em **Concluir seleção**, volte ao jogo e pressione o atalho quando o texto estiver completo. Selecionar a área não inicia OCR nem tradução.

**Super + Shift + R** é o atalho padrão; vale a combinação que você configurar. Cada acionamento faz **uma leitura de OCR** e no máximo uma chamada à API; textos conhecidos usam o cache. Também é possível clicar em **Traduzir agora**. Não há tradução automática nem confirmação por três leituras. Enquanto um pedido estiver em andamento, novas pressões são agrupadas, sem criar fila. Se a captura estiver encerrada ou pausada, o atalho mostra uma notificação com a orientação necessária.

O instalador registra o refresh nos atalhos personalizados do Ubuntu; ele funciona na sessão atual. Em uma atualização, o novo item do menu aparece quando o GNOME carregar novamente a extensão, normalmente no próximo login. Para mudar a combinação, abra o tradutor e clique em **Alterar atalho…**, pressione as teclas desejadas e clique em **Salvar**. Você também pode restaurar o padrão ou desativar o atalho; a escolha é preservada nas atualizações. A interface verifica conflitos com atalhos personalizados e atalhos comuns do GNOME. Alternativamente, use Configurações do Ubuntu → Teclado → Atalhos personalizados → **PixelLingo — Atualizar tradução**. Pelo terminal: `~/.local/bin/area-translator-refresh`.

**Super + Shift + T** pausa/retoma. O menu também permite selecionar outra área, encerrar a captura, habilitar fundo translúcido e reposicionar a legenda. Durante o reposicionamento, a captura pausa: arraste a legenda e solte. O modo termina automaticamente após 15 segundos. Fora desse modo, a legenda deixa os cliques passarem e não recebe foco.

A legenda permanece até outra tradução ficar pronta ou até você pausar/encerrar a captura. Ela não é apagada ao acionar o atalho, nem por leitura vazia ou erro de rede. A troca da região limpa a legenda. No diagnóstico, acompanhe os pedidos manuais e as contagens de OCR/API.

No modo **Janela do aplicativo**, a captura contém somente a janela autorizada; mover a janela mantém o recorte relativo ao seu conteúdo. A legenda fica inicialmente no rodapé do monitor escolhido e pode ser reposicionada pelo menu. Ela não acompanha a posição da janela e não faz parte do stream capturado.

No modo **Monitor inteiro**, o retângulo permanece fixo na tela: selecione novamente se mover o jogo. A legenda fica fora desse retângulo.

Redimensionar a janela, mudar a resolução ou entrar/sair de tela cheia pode alterar as dimensões do stream; quando isso acontece, a captura encerra e solicita uma nova seleção. Alterar a configuração dos monitores também encerra a captura. Ao fechar a janela, o encerramento informado pelo portal é tratado pelo serviço. Uma janela minimizada pode deixar de fornecer quadros; use pausar/retomar ao alternar o uso. Não há cadastro de jogos.

Bloquear a sessão oculta a legenda e pausa a tradução. Retome pelo menu ou atalho após desbloquear.

No diagnóstico, **Conferir imagem enviada ao OCR** mostra um único recorte recente em escala de cinza. Use-o para verificar se a frase está inteira dentro da seleção. A imagem é solicitada somente ao clicar, fica em memória e não é salva. Recortes com mais de 400 pixels de altura usam detecção de texto esparso para evitar que uma frase curta seja descartada como parte do cenário. Ainda é preferível selecionar a caixa de diálogo justa.

## Primeiro teste com emulador

1. Abra seu emulador e um jogo com diálogos em inglês. Escolha a resolução e a posição final da janela; se for jogar em tela cheia, entre nesse modo antes de selecionar a área.
2. Pare em um diálogo estático e legível. Abra **Tradutor de área** e selecione somente a caixa de texto, evitando elementos animados sempre que possível.
3. Escolha **Janela do aplicativo**, autorize a janela do jogo no diálogo do Ubuntu, marque o retângulo na prévia e escolha o monitor da legenda. Conclua a seleção e volte ao emulador.
4. Espere a frase terminar de aparecer, pressione seu atalho e confira a legenda em português. Avance alguns diálogos: nenhuma tradução deve acontecer até o próximo acionamento. Repita um texto para testar o cache e use **Super + Shift + T** para pausar/retomar.
5. Confira se o teclado e o controle continuam no emulador e se cliques atravessam a legenda. No modo janela, mova o emulador para conferir que a leitura acompanha seu conteúdo. Após redimensionar, use **Selecionar outra área**.
6. Ao terminar, encerre a captura pelo menu do tradutor.

Se a leitura estiver imprecisa, aumente o tamanho do texto ou a escala de renderização no emulador e selecione novamente. Para avaliar custo e desempenho, use uma cena reproduzível e siga o [roteiro de validação](docs/VALIDATION.md).

## Como mantém o consumo baixo

- PipeWire fornece a janela ou o monitor autorizado, mas somente o recorte é convertido para escala de cinza e processado. Não há conversão contínua do monitor inteiro.
- O compartilhamento permanece aberto e mantém somente uma referência ao buffer mais recente do PipeWire, sem copiar ou converter seus pixels continuamente. Isso permite traduzir também cenas estáticas, sem exigir um novo quadro após a tecla.
- Recorte e OCR ocorrem apenas por acionamento. O diagnóstico também pode solicitar um recorte, sem OCR nem rede. Modelo Tesseract inglês mantido em um trabalhador com OpenMP limitado a um thread.
- Cache LRU de 2.000 traduções em memória, compartilhado entre seleções durante a vida do serviço.
- Apenas uma tradução em andamento. Resultados de seleções, pausas e diálogos antigos são descartados.
- Ao pausar, o pipeline entra em `Paused`; ao encerrar, a sessão do portal fecha e o modelo é liberado pelo trabalhador.
- Sem Electron, servidor web local ou inferência de tradução na GPU.

Medições preliminares: OCR em cenas sintéticas levou entre 14 e 24 ms para quadros com texto; o serviço sem captura consumiu até 12,215 MiB de RSS. **O valor em repouso não representa o consumo durante um jogo**, nem inclui GTK, incremento do Shell ou modelo carregado.

As metas de 250 MB, 5% da CPU total e menos de 3% de impacto no FPS **não são garantias**. Veja as medições executadas e as lacunas em [docs/VALIDATION.md](docs/VALIDATION.md).

## Privacidade, custos e falhas

Capturas ficam em memória. A aplicação não salva imagens nem histórico de texto em disco. Somente o texto reconhecido segue para a API oficial por HTTPS. Logs contêm tempos e contagens, sem chave, imagens ou conteúdo dos diálogos. Imagens em `tests/fixtures` e nos registros de validação são cenas sintéticas.

Requisições repetidas usam o cache; erros de rede impõem espera de 2 até 32 segundos antes de aceitar outro acionamento. Não há repetição automática da chamada. Autenticação inválida ou cota esgotada suspendem captura e novas chamadas até correção e retomada manual. Cancelar uma chamada local não garante que o provedor deixe de contabilizá-la. O aplicativo não impõe um teto de gastos: configure cotas no Google Cloud.

OCR retorna vazio em leituras com baixa confiança. Fontes muito estilizadas, texto minúsculo, efeitos e cenários movimentados podem reduzir a qualidade. A primeira versão usa um bloco de texto; não reconstrói a disposição de menus complexos. A altura da legenda acompanha o texto e diminui quando a tradução encurta. No modo monitor, ela pode mudar para cima da região quando não há espaço abaixo. No modo janela, cresce para cima a partir do rodapé do monitor. Textos que excedem o espaço disponível são limitados visualmente. Prefira selecionar a caixa de diálogo justa.

## Desenvolvimento e testes

```sh
./scripts/dev.sh cargo test --all-targets
./scripts/dev.sh cargo test --test ocr_native -- --ignored --nocapture
./scripts/dev.sh cargo clippy --all-targets -- -D warnings
cargo fmt --check
node --test tests/geometry.mjs
dbus-run-session -- env AREA_TRANSLATOR_ISOLATED_TEST=1 GIO_USE_VFS=local gjs -m tests/refresh-client.js
glib-compile-schemas --strict --dry-run extension/schemas
./scripts/dev.sh target/release/area-translator --check
```

O teste nativo de OCR é opt-in porque precisa da biblioteca e do modelo. Ele usa dez imagens sintéticas, incluindo fonte pequena/pixelada, fundo escuro, múltiplas linhas, uma frase curta sobre cenários diferentes e quadros vazios. Para regenerá-las, instale Pillow e execute `python3 scripts/make-fixtures.py`.

Testes de integração adicionais, sem tocar na sessão gráfica atual:

```sh
./scripts/dev.sh cargo build
./scripts/dev.sh dbus-run-session -- env AREA_TRANSLATOR_ISOLATED_TEST=1 bash tests/dbus-smoke.sh
./scripts/dev.sh dbus-run-session -- env AREA_TRANSLATOR_ISOLATED_TEST=1 python3 tests/portal-smoke.py
./scripts/dev.sh dbus-run-session -- env AREA_TRANSLATOR_ISOLATED_TEST=1 cargo test --lib -- --ignored
./scripts/test-gnome.sh
xvfb-run -a -s '-screen 0 1200x800x24' bash tests/ui-smoke.sh
xvfb-run -a -s '-screen 0 1200x800x24' bash tests/ui-smoke.sh window
GSETTINGS_BACKEND=memory GDK_BACKEND=x11 GTK_A11Y=none xvfb-run -a gjs -m tests/shortcut.js
```

O teste GNOME cria um compositor isolado, carrega uma cópia instrumentada da extensão em `.deps/`, usa tradução simulada e uma aplicação de teste em tela cheia. A instrumentação não é instalada. Os testes gráficos requerem GNOME 46, Xvfb, Node e Pillow.

O teste de portal usa um serviço D-Bus simulado para conferir os pedidos de janela/monitor, o cancelamento e a ausência de suporte a janelas, sem capturar a tela nem chamar o Google. Requer Python com PyGObject (`python3-gi`). Ele não substitui o teste manual de autorização e captura real com o emulador.

Arquitetura e contrato D-Bus: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Desinstalar

```sh
./scripts/uninstall.sh
```

A chave é preservada no chaveiro. Para removê-la também, apague a entrada **Area Translator — Google Cloud** no aplicativo **Senhas e chaves**.

## Solução de problemas

Se a legenda não aparecer, abra **Tradutor de área → Diagnóstico: captura, OCR e tradução**. A página mostra os quadros recebidos, a confiança e o texto lido pelo OCR, as tentativas/conclusões da API e a última tradução. As contagens de OCR/API acumulam durante a vida do serviço; a contagem de quadros reinicia na seleção/retomada. Mantenha essa janela fora da área selecionada para não reconhecer a própria interface.

Se o texto lido estiver vazio ou incorreto, ajuste a região para conter somente a caixa de diálogo e confira a legibilidade. Se estiver correto, mas sem tradução, consulte o estado da API nessa mesma página. Se a tradução estiver presente no diagnóstico, mas não na tela, confira a extensão e o espaço reservado à legenda. Os dados do diagnóstico permanecem em memória.

| Sintoma | O que verificar |
| --- | --- |
| Extensão não encontrada após instalar | Saia da sessão e entre novamente; depois execute `gnome-extensions enable area-translator@local`. |
| Legenda não aparece | Após selecionar a área, pressione o atalho. Confira a extensão, o estado da captura e o diagnóstico se não aparecer. |
| Diálogo de captura cancelado ou compartilhamento encerrado | Abra novamente a seleção e autorize o monitor pelo portal. |
| Texto deslocado após mover o jogo | No modo monitor, selecione novamente; no modo janela, o recorte acompanha o conteúdo. Mudanças de tamanho exigem nova seleção. |
| Captura por janela pede extensão atualizada | A versão antiga ainda está na memória do GNOME; saia da sessão e entre novamente após instalar a atualização. |
| Erro de OCR ou modelo ausente | Execute `~/.local/bin/area-translator --check`; instale o modelo inglês ou use a opção `--local-deps`. |
| Credencial inválida ou cota esgotada | Corrija a configuração no Google Cloud, atualize a chave se necessário e retome manualmente. |
| Falhas temporárias de rede | Aguarde alguns segundos e pressione o atalho novamente; confira a conexão se persistirem. |
| Fonte estilizada ou leitura incompleta | Selecione uma caixa menor e aumente o texto no jogo; consulte os limites de OCR acima. |

## Estrutura do projeto

```text
src/                 Serviço Rust, captura, OCR, tradução e D-Bus
ui/                  Interface GTK4/GJS de configuração e seleção
extension/           Legenda, controles e atalhos do GNOME Shell
scripts/             Instalação local, desenvolvimento e medições
tests/               Testes automatizados e imagens sintéticas
docs/                Arquitetura, configuração genérica e validação
```

## Contribuir

Para relatar um problema, informe versão do Ubuntu/GNOME, tipo de sessão, escala dos monitores e passos para reproduzir. Use textos ou imagens sintéticas quando possível. Remova credenciais e informações da conta de qualquer material anexado. Alterações de código devem incluir validação proporcional ao comportamento afetado; testes de nuvem devem ser executados explicitamente com credenciais próprias.

## Licença

Código distribuído sob a [licença MIT](LICENSE). Bibliotecas e modelos mantêm suas próprias licenças; o instalador preserva os avisos dos pacotes de OCR quando os inclui na instalação local.
