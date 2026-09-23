use crate::model::Frame;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrError {
    Credential,
    Quota,
    Temporary,
    InvalidResponse,
    InvalidImage,
}
impl OcrError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::Credential => {
                "Verifique a chave, a ativação da Cloud Vision API e a restrição vision.googleapis.com."
            }
            Self::Quota => {
                "Cota do Cloud Vision atingida. Verifique o projeto e retome manualmente."
            }
            Self::Temporary => "Cloud Vision indisponível. Aguarde e tente novamente pelo atalho.",
            Self::InvalidResponse => {
                "Resposta inválida do Cloud Vision. Tente novamente pelo atalho."
            }
            Self::InvalidImage => {
                "O Cloud Vision não aceitou a imagem. Selecione uma área menor e tente novamente."
            }
        }
    }
    pub fn suspends(&self) -> bool {
        matches!(self, Self::Credential | Self::Quota)
    }
}

#[derive(Debug)]
pub struct OcrOutput {
    pub text: String,
    pub elapsed_ms: u64,
    pub api_ms: u64,
}

#[derive(Clone)]
pub struct Ocr {
    client: reqwest::Client,
    endpoint: String,
}

#[derive(Deserialize)]
struct BatchResponse {
    responses: Vec<Annotation>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Annotation {
    full_text_annotation: Option<FullText>,
    #[serde(default)]
    text_annotations: Vec<Text>,
    error: Option<Status>,
}
#[derive(Deserialize)]
struct FullText {
    text: String,
}
#[derive(Deserialize)]
struct Text {
    description: String,
}
#[derive(Deserialize)]
struct Status {
    #[serde(default)]
    code: u32,
}

impl Ocr {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(15))
                .pool_max_idle_per_host(1)
                .build()?,
            endpoint: "https://vision.googleapis.com/v1/images:annotate".into(),
        })
    }
    #[cfg(test)]
    pub(crate) fn with_test_endpoint(endpoint: String) -> Self {
        let mut ocr = Self::new().unwrap();
        ocr.endpoint = endpoint;
        ocr
    }

    pub async fn recognize(&self, key: &str, frame: Frame) -> Result<OcrOutput, OcrError> {
        let started = Instant::now();
        // CPU encoding stays off the actor, so pause/stop/expiry remain responsive.
        let content = tokio::task::spawn_blocking(move || encode_crop(&frame))
            .await
            .map_err(|_| OcrError::InvalidImage)??;
        let api_started = Instant::now();
        let response = self.client.post(&self.endpoint).header("X-Goog-Api-Key", key)
            .json(&serde_json::json!({"requests": [{"image": {"content": content},
                "features": [{"type": "TEXT_DETECTION"}], "imageContext": {"languageHints": ["en"]}}]}))
            .send().await.map_err(|_| OcrError::Temporary)?;
        let status = response.status().as_u16();
        // Never include provider bodies, URLs containing credentials, or image data in errors/logs.
        let body = response.json::<serde_json::Value>().await;
        if !(200..300).contains(&status) {
            let reason = body
                .as_ref()
                .map(|body| body.to_string().to_ascii_lowercase())
                .unwrap_or_default();
            return Err(
                if status == 429
                    || reason.contains("resource_exhausted")
                    || reason.contains("quota")
                    || reason.contains("limitexceeded")
                {
                    OcrError::Quota
                } else if matches!(status, 401 | 403) || reason.contains("api_key_invalid") {
                    OcrError::Credential
                } else if status == 400 {
                    OcrError::InvalidImage
                } else {
                    OcrError::Temporary
                },
            );
        }
        let text = parse_response(body.map_err(|_| OcrError::InvalidResponse)?)?;
        Ok(OcrOutput {
            text,
            elapsed_ms: started.elapsed().as_millis() as u64,
            api_ms: api_started.elapsed().as_millis() as u64,
        })
    }
}

fn encode_crop(frame: &Frame) -> Result<String, OcrError> {
    if frame.width == 0
        || frame.height == 0
        || u64::from(frame.width) * u64::from(frame.height) > 4_000_000
        || frame.gray.len() as u64 != u64::from(frame.width) * u64::from(frame.height)
    {
        return Err(OcrError::InvalidImage);
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, frame.width, frame.height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder
            .write_header()
            .map_err(|_| OcrError::InvalidImage)?
            .write_image_data(&frame.gray)
            .map_err(|_| OcrError::InvalidImage)?;
    }
    Ok(STANDARD.encode(bytes))
}

fn parse_response(body: serde_json::Value) -> Result<String, OcrError> {
    let mut response: BatchResponse =
        serde_json::from_value(body).map_err(|_| OcrError::InvalidResponse)?;
    if response.responses.len() != 1 {
        return Err(OcrError::InvalidResponse);
    }
    let result = response.responses.pop().unwrap();
    if let Some(error) = result.error
        && error.code != 0
    {
        return Err(match error.code {
            7 | 16 => OcrError::Credential,
            8 => OcrError::Quota,
            3 => OcrError::InvalidImage,
            _ => OcrError::Temporary,
        });
    }
    Ok(result
        .full_text_annotation
        .map(|a| a.text)
        .or_else(|| {
            result
                .text_annotations
                .into_iter()
                .next()
                .map(|a| a.description)
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn annotations_empty_results_and_embedded_errors() {
        assert_eq!(parse_response(serde_json::json!({"responses":[{"fullTextAnnotation":{"text":"One\nTwo"},"textAnnotations":[{"description":"duplicate"}]}]})).unwrap(), "One\nTwo");
        assert_eq!(parse_response(serde_json::json!({"responses":[{"textAnnotations":[{"description":"Fallback"},{"description":"word"}]}]})).unwrap(), "Fallback");
        assert_eq!(
            parse_response(serde_json::json!({"responses":[{}]})).unwrap(),
            ""
        );
        assert_eq!(
            parse_response(serde_json::json!({"responses":[]})),
            Err(OcrError::InvalidResponse)
        );
        for (code, expected) in [
            (7, OcrError::Credential),
            (16, OcrError::Credential),
            (8, OcrError::Quota),
            (3, OcrError::InvalidImage),
            (14, OcrError::Temporary),
        ] {
            assert_eq!(
                parse_response(
                    serde_json::json!({"responses":[{"error":{"code":code,"message":"do not echo"},"fullTextAnnotation":{"text":"partial"}}]})
                ),
                Err(expected)
            );
        }
    }

    #[tokio::test]
    async fn http_errors_do_not_require_json_bodies() {
        for (status, body, expected) in [
            (401, "invalid", OcrError::Credential),
            (403, "invalid", OcrError::Credential),
            (429, "invalid", OcrError::Quota),
            (503, "invalid", OcrError::Temporary),
            (
                400,
                r#"{"error":{"details":[{"reason":"API_KEY_INVALID"}]}}"#,
                OcrError::Credential,
            ),
            (200, "invalid", OcrError::InvalidResponse),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let ocr = Ocr::with_test_endpoint(format!("http://{}", listener.local_addr().unwrap()));
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 8192];
                let _ = socket.read(&mut request).await.unwrap();
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            let result = ocr
                .recognize(
                    "test-only",
                    Frame {
                        width: 16,
                        height: 16,
                        gray: vec![255; 256],
                        captured: Instant::now(),
                    },
                )
                .await;
            assert_eq!(result.unwrap_err(), expected);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn request_contains_only_crop_png_and_key_in_header() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ocr = Ocr::with_test_endpoint(format!(
            "http://{}/v1/images:annotate",
            listener.local_addr().unwrap()
        ));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut bytes = [0; 4096];
                let n = socket.read(&mut bytes).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&bytes[..n]);
                if let Some((headers, body)) = std::str::from_utf8(&request)
                    .unwrap()
                    .split_once("\r\n\r\n")
                {
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    if body.len() >= length {
                        break;
                    }
                }
            }
            let request = String::from_utf8(request).unwrap();
            let (headers, body) = request.split_once("\r\n\r\n").unwrap();
            assert!(headers.starts_with("POST /v1/images:annotate HTTP"));
            assert!(headers.contains("x-goog-api-key: test-only"));
            let body: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(body["requests"].as_array().unwrap().len(), 1);
            let item = &body["requests"][0];
            assert_eq!(
                item["features"],
                serde_json::json!([{"type":"TEXT_DETECTION"}])
            );
            assert_eq!(
                item["imageContext"]["languageHints"],
                serde_json::json!(["en"])
            );
            assert!(item["image"].get("source").is_none());
            let png = STANDARD
                .decode(item["image"]["content"].as_str().unwrap())
                .unwrap();
            let mut reader = png::Decoder::new(std::io::Cursor::new(png))
                .read_info()
                .unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut pixels).unwrap();
            assert_eq!((info.width, info.height), (16, 20));
            assert_eq!(pixels, vec![97; 320]);
            let body = r#"{"responses":[{"fullTextAnnotation":{"text":"Hello!"}}]}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let result = ocr
            .recognize(
                "test-only",
                Frame {
                    width: 16,
                    height: 20,
                    gray: vec![97; 320],
                    captured: Instant::now(),
                },
            )
            .await
            .unwrap();
        assert_eq!(result.text, "Hello!");
        server.await.unwrap();
    }
}
