# Configurar a tradução online

O aplicativo utiliza Google Cloud Translation **Basic (v2)**, modelo NMT, de inglês para português brasileiro. Cada instalação usa sua própria credencial; o repositório não fornece chave compartilhada.

## Projeto e API

1. No [Console do Google Cloud](https://console.cloud.google.com/), crie ou selecione um projeto para o aplicativo.
2. Vincule uma conta de faturamento. O Google exige faturamento habilitado para Cloud Translation.
3. Em **APIs e serviços → Biblioteca**, habilite **Cloud Translation API** (`translate.googleapis.com`).
4. Em **APIs e serviços → Credenciais**, crie uma chave de API. Nas restrições de API, limite seu uso exclusivamente à **Cloud Translation API**.

Referência: [configuração oficial do Cloud Translation](https://docs.cloud.google.com/translate/docs/setup).

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

O script valida a restrição, salva a credencial e faz **uma requisição real** de tradução com frase sintética, sujeita à cobrança da API. A saída mostra identificador do projeto, resultado e latência, sem o valor da chave. Revise essa saída antes de compartilhá-la. O script não cria projetos, não habilita faturamento e não cria chaves.

## Custos e falhas

Consulte os [preços vigentes](https://cloud.google.com/products/translate/pricing) e configure limites em [cotas de uso](https://docs.cloud.google.com/translate/quotas). O aplicativo mantém cache de traduções, mas não estabelece um teto financeiro. Requisições já recebidas pelo provedor podem ser contabilizadas mesmo após cancelamento local.

Erros temporários usam espera progressiva. Credencial inválida ou cota esgotada suspendem novas chamadas até correção e retomada manual. Não publique a chave em issues, capturas de tela ou logs. Se houver exposição, revogue a credencial no Google Cloud e cadastre uma nova no chaveiro.

## Primeiro teste

Selecione uma caixa com uma frase curta em inglês, por exemplo `The door is locked. Find the key.`. Aguarde a estabilização e confira a legenda em português. Apenas o texto reconhecido é enviado à API; imagens da captura permanecem em memória local. Veja o [roteiro com emulador](../README.md#primeiro-teste-com-emulador).
