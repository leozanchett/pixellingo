#!/usr/bin/env python3
"""Import an existing restricted GCP key directly into the desktop keyring.

Never prints key material or writes it to a file. The verification request uses
one synthetic sentence and the same API contract as the Rust application.
"""
import argparse
import json
import subprocess
import time
import urllib.error
import urllib.request

import gi

gi.require_version('Secret', '1')
from gi.repository import Secret  # noqa: E402


def gcloud(*arguments):
    # Some gcloud long-running operations print full results (including keys)
    # to stderr even with --format. Capture BOTH streams and never echo them.
    result = subprocess.run(['gcloud', *arguments, '--quiet'], capture_output=True, text=True)
    if result.returncode:
        raise SystemExit('Operação gcloud falhou; saída omitida para proteger credenciais.')
    return result.stdout.strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', required=True)
    parser.add_argument('--key-id', required=True)
    args = parser.parse_args()
    project_flag = f'--project={args.project}'
    metadata = json.loads(gcloud('services', 'api-keys', 'describe', args.key_id,
                                project_flag, '--format=json(name,restrictions)'))
    targets = metadata.get('restrictions', {}).get('apiTargets', [])
    if len(targets) != 2 or {target.get('service') for target in targets} != {'translate.googleapis.com', 'vision.googleapis.com'}:
        raise SystemExit('Restrinja a chave a translate.googleapis.com e vision.googleapis.com.')
    key = gcloud('services', 'api-keys', 'get-key-string', args.key_id,
                 project_flag, '--format=value(keyString)')
    if not key.startswith('AIza') or len(key) > 256:
        raise SystemExit('A API não retornou uma chave válida.')

    schema = Secret.Schema.new('io.github.areatranslator.Credential', Secret.SchemaFlags.NONE,
                               {'application': Secret.SchemaAttributeType.STRING})
    attributes = {'application': 'area-translator'}
    if not Secret.password_store_sync(schema, attributes, Secret.COLLECTION_DEFAULT,
                                      'Area Translator — Google Cloud', key, None):
        raise SystemExit('Não foi possível salvar no chaveiro.')
    if Secret.password_lookup_sync(schema, attributes, None) != key:
        raise SystemExit('A verificação do chaveiro falhou.')
    print('Chave restrita salva e verificada no chaveiro. Nenhuma credencial foi gravada em arquivo.')

    body = json.dumps({'q': 'The door is locked. Find the key.', 'source': 'en',
                       'target': 'pt-BR', 'format': 'text', 'model': 'nmt'}).encode()
    request = urllib.request.Request('https://translation.googleapis.com/language/translate/v2',
                                     data=body, headers={'Content-Type': 'application/json',
                                                        'X-Goog-Api-Key': key}, method='POST')
    started = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            result = json.load(response)
    except urllib.error.HTTPError as error:
        result = json.loads(error.read())
        message = str(result.get('error', {}).get('message', 'Erro da API')).replace(key, '[redigido]')
        raise SystemExit(f'Teste de tradução falhou (HTTP {error.code}): {message}') from None
    except urllib.error.URLError:
        raise SystemExit('Teste de tradução falhou por erro de rede; a chave permanece salva.') from None
    print(json.dumps({'project': args.project, 'api': 'Cloud Translation Basic / NMT',
                      'source': 'en', 'target': 'pt-BR',
                      'translation': result['data']['translations'][0]['translatedText'],
                      'elapsed_ms': round((time.monotonic() - started) * 1000)}, ensure_ascii=False))


if __name__ == '__main__':
    main()
