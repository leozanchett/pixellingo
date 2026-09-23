# Configurar OCR e tradução online

O aplicativo utiliza Google Cloud Vision **TEXT_DETECTION** para reconhecer o recorte e Google Cloud Translation **Basic (v2)**, modelo NMT, de inglês para português brasileiro. Cada instalação usa sua própria credencial; o repositório não fornece chave compartilhada.

## Projeto e API

1. No [Console do Google Cloud](https://console.cloud.google.com/), crie ou selecione um projeto para o aplicativo.
2. Vincule uma conta de faturamento. O Google exige faturamento habilitado para Cloud Translation.
3. Em **APIs e serviços → Biblioteca**, habilite **Cloud Vision API** (`vision.googleapis.com`) e **Cloud Translation API** (`translate.googleapis.com`).
4. Em **APIs e serviços → Credenciais**, crie uma chave de API. Nas restrições de API, limite seu uso exclusivamente a **Cloud Vision API** e **Cloud Translation API**. Se já usava a versão com OCR local, habilite Vision e acrescente essa API às restrições da chave existente.

Referências: [configuração do Cloud Vision](https://docs.cloud.google.com/vision/docs/setup) e [Cloud Translation](https://docs.cloud.google.com/translate/docs/setup).

## Salvar no aplicativo

Abra **Tradutor de área**, cole a chave no campo de credencial e clique em **Salvar chave**. Ela é armazenada pelo Secret Service no chaveiro da sessão. Não é necessário criar `.env`, baixar uma chave JSON de conta de serviço ou inserir a credencial no código.

O serviço recebe a chave em memória por D-Bus da sessão e a envia somente no cabeçalho HTTPS de autenticação. Outros processos da mesma sessão compartilham a fronteira de confiança do desktop. O chaveiro protege a persistência, mas não isola a credencial de software malicioso executado como o mesmo usuário.

Para substituir a credencial, salve a nova chave na interface. Para removê-la, apague a entrada **Area Translator — Google Cloud** no aplicativo **Senhas e chaves**. Desinstalar o tradutor preserva essa entrada.

## Importação opcional pela CLI

Para quem já usa `gcloud`, o script abaixo importa uma chave existente e restrita diretamente para o chaveiro. Ele requer `gcloud` autenticado, Python 3 com PyGObject e o binding Secret Service (`python3-gi` e `gir1.2-secret-1` no Ubuntu).

Substitua somente os identificadores de exemplo; **não passe o valor da chave na linha de comando**:

```sh
python3 scripts/configure-gcp-key.py \
  --project YOUR_PROJECT_ID \
  --key-id YOUR_KEY_RESOURCE_ID
```

O script valida a restrição às duas APIs, salva a credencial e faz **uma requisição real** de tradução com frase sintética, sujeita à cobrança da API. A saída mostra identificador do projeto, resultado e latência, sem o valor da chave. Revise essa saída antes de compartilhá-la. O script não cria projetos, não habilita APIs/faturamento e não cria chaves. A verificação incluída nele testa apenas Translation; o teste opt-in abaixo verifica também Vision.

## Custos e falhas

Consulte os preços de [Vision](https://cloud.google.com/vision/pricing) e [Translation](https://cloud.google.com/products/translate/pricing) e configure limites nas cotas de [Vision](https://docs.cloud.google.com/vision/quotas) e [Translation](https://docs.cloud.google.com/translate/quotas). Cada acionamento solicita um OCR online; o cache economiza somente a tradução de textos já conhecidos. O aplicativo mantém cache de traduções, mas não estabelece um teto financeiro. Requisições já recebidas pelo provedor podem ser contabilizadas mesmo após cancelamento local.

Erros temporários usam espera progressiva. Credencial inválida ou cota esgotada suspendem novas chamadas até correção e retomada manual. Não publique a chave em issues, capturas de tela ou logs. Se houver exposição, revogue a credencial no Google Cloud e cadastre uma nova no chaveiro.

## Primeiro teste

Selecione uma caixa com uma frase curta em inglês, por exemplo `The door is locked. Find the key.`. Espere o texto ficar completo e pressione o atalho configurado. O recorte da área selecionada é enviado ao Cloud Vision e o texto reconhecido segue para Translation. A legenda em português permanece por 15 segundos após ficar pronta. A aplicação não salva capturas em disco. Veja o [roteiro com emulador](../README.md#primeiro-teste-com-emulador).


Teste real opcional, com credencial salva no chaveiro:

```sh
AREA_TRANSLATOR_CLOUD_TEST=1 ./scripts/dev.sh cargo test --test cloud_ocr -- --ignored --nocapture
```

Envia uma imagem sintética do repositório para Vision e seu texto para Translation. Pode gerar cobrança; não imprime a chave nem salva imagens recebidas da captura. Os testes padrão não acessam as APIs reais.
