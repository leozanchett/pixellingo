use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslationError {
    Credential,
    Quota,
    Temporary,
    InvalidResponse,
}

impl TranslationError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Credential => {
                "Verifique a chave e a ativação da Cloud Translation API nas configurações."
            }
            Self::Quota => "Limite da API atingido. Verifique sua cota e retome manualmente.",
            Self::Temporary => "Tradução indisponível. Nova tentativa em instantes.",
            Self::InvalidResponse => "A API retornou uma resposta inválida.",
        }
    }
    pub fn suspends(&self) -> bool {
        matches!(self, Self::Credential | Self::Quota)
    }
}

#[derive(Clone)]
pub struct Translator {
    client: reqwest::Client,
    endpoint: String,
}

#[derive(Deserialize)]
struct Response {
    data: ResponseData,
}
#[derive(Deserialize)]
struct ResponseData {
    translations: Vec<Translation>,
}
#[derive(Deserialize)]
struct Translation {
    #[serde(rename = "translatedText")]
    text: String,
}

impl Translator {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(10))
                .pool_max_idle_per_host(1)
                .build()?,
            endpoint: "https://translation.googleapis.com/language/translate/v2".into(),
        })
    }

    pub async fn translate(&self, key: &str, text: &str) -> Result<String, TranslationError> {
        // Do not put the credential in the URL (URLs can appear in diagnostics).
        // Never log the response body: provider errors can echo submitted text.
        let response = self.client.post(&self.endpoint).header("X-Goog-Api-Key", key)
            .json(&serde_json::json!({"q": text, "source": "en", "target": "pt-BR", "format": "text", "model": "nmt"}))
            .send().await.map_err(|_| TranslationError::Temporary)?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            // Google v2 sometimes uses 403 for rate/daily limits and for auth.
            let reason = response.json::<serde_json::Value>().await.ok();
            let quota = reason.as_ref().is_some_and(|body| {
                body.to_string()
                    .to_ascii_lowercase()
                    .contains("limitexceeded")
                    || body.to_string().to_ascii_lowercase().contains("quota")
                    || body.to_string().contains("RESOURCE_EXHAUSTED")
            });
            return Err(classify_error(status, quota));
        }
        let result: Response = response
            .json()
            .await
            .map_err(|_| TranslationError::InvalidResponse)?;
        result
            .data
            .translations
            .into_iter()
            .next()
            .map(|t| t.text)
            .filter(|t| !t.trim().is_empty())
            .ok_or(TranslationError::InvalidResponse)
    }
}

fn classify_error(status: u16, quota: bool) -> TranslationError {
    if status == 429 || quota {
        TranslationError::Quota
    } else if matches!(status, 400 | 401 | 403) {
        TranslationError::Credential
    } else {
        TranslationError::Temporary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn quota_and_auth_suspend_but_server_errors_retry() {
        assert_eq!(classify_error(403, true), TranslationError::Quota);
        assert_eq!(classify_error(403, false), TranslationError::Credential);
        assert!(!classify_error(503, false).suspends());
    }

    #[tokio::test]
    async fn request_contract_and_plain_text_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![];
            loop {
                let mut bytes = [0; 2048];
                let n = socket.read(&mut bytes).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&bytes[..n]);
                let input = String::from_utf8_lossy(&request);
                if let Some((headers, body)) = input.split_once("\r\n\r\n") {
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if body.len() >= length {
                        break;
                    }
                }
            }
            let input = String::from_utf8(request).unwrap();
            assert!(input.contains("x-goog-api-key: test-only"));
            assert!(!input.lines().next().unwrap().contains("test-only"));
            let body: serde_json::Value =
                serde_json::from_str(input.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body["target"], "pt-BR");
            assert_eq!(body["format"], "text");
            assert_eq!(body["q"], "Hello!");
            let body = r#"{"data":{"translations":[{"translatedText":"Olá!"}]}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let mut translator = Translator::new().unwrap();
        translator.endpoint = endpoint;
        assert_eq!(
            translator.translate("test-only", "Hello!").await.unwrap(),
            "Olá!"
        );
        server.await.unwrap();
    }
}
